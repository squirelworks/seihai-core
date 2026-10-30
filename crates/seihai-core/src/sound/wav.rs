//! RIFF/WAVE decoder for sound effects.
//!
//! Reads what the shipped sound folders hold: PCM at 8 or 16 bits, mono,
//! at rates from 5 kHz to 27 kHz, and the four IMA ADPCM files (format
//! tag 17) in the 2.22 folder. It also reads 24 and 32-bit PCM and
//! WAVE_FORMAT_EXTENSIBLE wrappers of those. Anything else is an error,
//! and the caller logs it and plays nothing.

/// Decoded audio: interleaved signed 16-bit samples.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub channels: u16,
    pub rate: u32,
    pub samples: Vec<i16>,
}

impl Pcm {
    /// Sample frames (one sample per channel each).
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    /// Length in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        if self.rate == 0 {
            return 0;
        }
        self.frames() as u64 * 1000 / self.rate as u64
    }
}

const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107,
    118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963,
    1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894,
    6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];
const INDEX_TABLE: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

struct Fmt {
    tag: u16,
    channels: u16,
    rate: u32,
    block_align: usize,
    bits: u16,
}

/// Decode a WAV file.
pub fn decode_wav(bytes: &[u8]) -> Result<Pcm, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut pos = 12;
    let mut fmt: Option<Fmt> = None;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32le(bytes, pos + 4).unwrap_or(0) as usize;
        let body_start = pos + 8;
        // A data chunk may claim more than the file holds; use what is there.
        let body_end = body_start.saturating_add(len).min(bytes.len());
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err("short fmt chunk".into());
                }
                let mut tag = u16le(body, 0).unwrap_or(0);
                let bits = u16le(body, 14).unwrap_or(0);
                // WAVE_FORMAT_EXTENSIBLE: the real tag is the first two
                // bytes of the sub-format GUID.
                if tag == 0xFFFE && body.len() >= 26 {
                    tag = u16le(body, 24).unwrap_or(tag);
                }
                fmt = Some(Fmt {
                    tag,
                    channels: u16le(body, 2).unwrap_or(0),
                    rate: u32le(body, 4).unwrap_or(0),
                    block_align: u16le(body, 12).unwrap_or(0) as usize,
                    bits,
                });
            }
            b"data" => {
                if data.is_none() {
                    data = Some(body);
                }
            }
            _ => {}
        }
        // Chunks are padded to an even length.
        pos = body_start.saturating_add(len).saturating_add(len & 1);
    }
    let fmt = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    if fmt.channels == 0 || fmt.channels > 2 {
        return Err(format!("unsupported channel count {}", fmt.channels));
    }
    if fmt.rate == 0 {
        return Err("sample rate 0".into());
    }
    let samples = match (fmt.tag, fmt.bits) {
        (1, 8) => data.iter().map(|b| ((*b as i16) - 128) << 8).collect(),
        (1, 16) => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect(),
        (1, 24) => data.chunks_exact(3).map(|c| i16::from_le_bytes([c[1], c[2]])).collect(),
        (1, 32) => data.chunks_exact(4).map(|c| i16::from_le_bytes([c[2], c[3]])).collect(),
        (17, 4) => decode_ima(data, &fmt)?,
        (tag, bits) => return Err(format!("unsupported WAV format tag {tag} at {bits} bits")),
    };
    Ok(Pcm { channels: fmt.channels, rate: fmt.rate, samples })
}

/// Microsoft IMA ADPCM: blocks with a 4-byte header per channel (the
/// first sample and the step index), then 4-byte groups of 8 nibbles per
/// channel, low nibble first.
fn decode_ima(data: &[u8], fmt: &Fmt) -> Result<Vec<i16>, String> {
    let ch = fmt.channels as usize;
    let align = fmt.block_align;
    if align < 4 * ch || (align - 4 * ch) % (4 * ch) != 0 {
        return Err(format!("bad IMA ADPCM block size {align}"));
    }
    let mut out = Vec::new();
    for block in data.chunks(align) {
        if block.len() < 4 * ch {
            break;
        }
        let mut pred = [0i32; 2];
        let mut idx = [0i32; 2];
        let mut chan: [Vec<i16>; 2] = [Vec::new(), Vec::new()];
        for c in 0..ch {
            pred[c] = i16::from_le_bytes([block[4 * c], block[4 * c + 1]]) as i32;
            idx[c] = (block[4 * c + 2] as i32).clamp(0, 88);
            chan[c].push(pred[c] as i16);
        }
        let groups = (block.len() - 4 * ch) / (4 * ch);
        for g in 0..groups {
            for c in 0..ch {
                let at = 4 * ch + (g * ch + c) * 4;
                for b in &block[at..at + 4] {
                    for nib in [b & 0x0F, b >> 4] {
                        let step = STEP_TABLE[idx[c] as usize];
                        let mut diff = step >> 3;
                        if nib & 1 != 0 {
                            diff += step >> 2;
                        }
                        if nib & 2 != 0 {
                            diff += step >> 1;
                        }
                        if nib & 4 != 0 {
                            diff += step;
                        }
                        if nib & 8 != 0 {
                            diff = -diff;
                        }
                        pred[c] = (pred[c] + diff).clamp(-32768, 32767);
                        idx[c] = (idx[c] + INDEX_TABLE[(nib & 7) as usize]).clamp(0, 88);
                        chan[c].push(pred[c] as i16);
                    }
                }
            }
        }
        let n = chan[0].len();
        for i in 0..n {
            for c in 0..ch {
                out.push(chan[c][i]);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(tag: u16, channels: u16, rate: u32, bits: u16, block_align: u16, extra: &[u8], data: &[u8]) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend(tag.to_le_bytes());
        fmt.extend(channels.to_le_bytes());
        fmt.extend(rate.to_le_bytes());
        fmt.extend((rate * block_align as u32).to_le_bytes());
        fmt.extend(block_align.to_le_bytes());
        fmt.extend(bits.to_le_bytes());
        fmt.extend(extra);
        let mut out = b"RIFF".to_vec();
        out.extend(((4 + 8 + fmt.len() + 8 + data.len()) as u32).to_le_bytes());
        out.extend(b"WAVE");
        out.extend(b"fmt ");
        out.extend((fmt.len() as u32).to_le_bytes());
        out.extend(&fmt);
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn pcm8_is_unsigned_and_centred_on_128() {
        let p = decode_wav(&wav(1, 1, 22050, 8, 1, &[], &[128, 255, 0])).unwrap();
        assert_eq!(p.samples, vec![0, 127 << 8, -128 << 8]);
        assert_eq!((p.channels, p.rate), (1, 22050));
    }

    #[test]
    fn pcm16_stereo_and_duration() {
        let data: Vec<u8> = [100i16, -100, 200, -200].iter().flat_map(|s| s.to_le_bytes()).collect();
        let p = decode_wav(&wav(1, 2, 8000, 16, 4, &[], &data)).unwrap();
        assert_eq!(p.samples, vec![100, -100, 200, -200]);
        assert_eq!(p.frames(), 2);
        let long = decode_wav(&wav(1, 1, 8000, 16, 2, &[], &vec![0u8; 16000])).unwrap();
        assert_eq!(long.duration_ms(), 1000);
    }

    #[test]
    fn data_chunk_longer_than_the_file_is_cut() {
        let mut b = wav(1, 1, 8000, 16, 2, &[], &[1, 0, 2, 0]);
        // Claim 100 data bytes.
        let at = b.len() - 4 - 4;
        b[at..at + 4].copy_from_slice(&100u32.to_le_bytes());
        assert_eq!(decode_wav(&b).unwrap().samples, vec![1, 2]);
    }

    #[test]
    fn unknown_chunks_and_padding_are_skipped() {
        let mut b = wav(1, 1, 8000, 8, 1, &[], &[128]);
        let tail = b.split_off(12);
        b.extend(b"LIST");
        b.extend(3u32.to_le_bytes());
        b.extend([1, 2, 3, 0]); // 3 bytes + 1 pad
        b.extend(tail);
        assert_eq!(decode_wav(&b).unwrap().samples, vec![0]);
    }

    #[test]
    fn rejects_junk_and_unsupported_formats() {
        assert!(decode_wav(b"").is_err());
        assert!(decode_wav(b"RIFF0000WAVEjunk").is_err());
        assert!(decode_wav(b"not a wav file at all").is_err());
        // mu-law (tag 7) is not supported.
        assert!(decode_wav(&wav(7, 1, 8000, 8, 1, &[], &[0, 1])).unwrap_err().contains("tag 7"));
        // A fmt with no data chunk.
        let mut b = wav(1, 1, 8000, 8, 1, &[], &[]);
        b.truncate(b.len() - 8);
        assert!(decode_wav(&b).is_err());
        assert!(decode_wav(&wav(1, 3, 8000, 8, 1, &[], &[0])).is_err());
    }

    #[test]
    fn ima_adpcm_block_decodes_header_and_nibbles() {
        // One mono block: header (predictor 1000, index 0), then 4 bytes.
        // Nibble 0 at index 0 (step 7): diff = 7>>3 = 0, so it holds 1000.
        // Nibble 7 (step 7): diff = 0+1+3+7 = 11, index +8.
        let mut block = vec![];
        block.extend(1000i16.to_le_bytes());
        block.extend([0u8, 0]);
        block.extend([0x70, 0x00, 0x00, 0x00]); // low nibble 0, high nibble 7
        let extra = [2u8, 0, 4, 0]; // cbSize 2, samplesPerBlock 4... (only read as u16 at fmt+18)
        let w = wav(17, 1, 11025, 4, 8, &extra, &block);
        let p = decode_wav(&w).unwrap();
        assert_eq!(p.samples.len(), 9); // header sample + 8 nibbles
        assert_eq!(p.samples[0], 1000);
        assert_eq!(p.samples[1], 1000);
        assert_eq!(p.samples[2], 1011);
    }

    #[test]
    fn ima_adpcm_rejects_a_bad_block_size() {
        let w = wav(17, 1, 11025, 4, 6, &[2, 0, 4, 0], &[0; 6]);
        assert!(decode_wav(&w).is_err());
    }

    /// Decode every file in a real client's `sounds` folder. Run with
    /// `SEIHAI_SOUNDS_DIR=<dir> cargo test real_sounds -- --ignored`.
    /// With `SEIHAI_DUMP_DIR` set, also writes each result as raw s16le.
    #[test]
    #[ignore]
    fn real_sounds_decode() {
        let Ok(dir) = std::env::var("SEIHAI_SOUNDS_DIR") else {
            return;
        };
        let dump = std::env::var("SEIHAI_DUMP_DIR").ok();
        let mut n = 0;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let bytes = std::fs::read(e.path()).unwrap();
            let name = e.file_name().to_string_lossy().to_string();
            let pcm = decode_wav(&bytes).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert!(!pcm.samples.is_empty(), "{name}");
            if let Some(d) = &dump {
                let raw: Vec<u8> = pcm.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                std::fs::write(format!("{d}/{name}.raw"), raw).unwrap();
            }
            n += 1;
        }
        eprintln!("decoded {n} files");
    }
}


/// A tiny mono 8-bit PCM WAV for tests: `len` samples at `rate`.
#[cfg(test)]
pub(crate) fn test_wav(rate: u32, len: usize) -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend(((36 + len) as u32).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(8u16.to_le_bytes());
    out.extend(b"data");
    out.extend((len as u32).to_le_bytes());
    out.extend(std::iter::repeat(128u8).take(len));
    out
}
