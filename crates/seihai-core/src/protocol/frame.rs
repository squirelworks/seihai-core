//! Frame helpers: zlib compress/decompress and gen-3/gen-5 wrapping.
//!
//! A frame is `u16be length` + body. The body is zlib for gen-2/3,
//! or a compression-type byte + body for gen-5.

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{self, Read, Write};

/// Compress `data` with zlib (default level).
pub fn zlib_compress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data)?;
    enc.finish()
}

/// Compress `data` with zlib into a stream whose length is a multiple of 4.
///
/// 5.12 steps the gen-5 cipher once per whole 4-byte group of a compressed
/// body (`len / 4`), and GServer-v2 once per started group
/// (`len.div_ceil(4)`). The two agree only when the length is a multiple of
/// 4 or the body is 16 bytes or longer (both stop at 4 steps). 5.12 sends
/// even a one-message body compressed, so a short body put the two sides
/// out of step, and GServer-v2 could read nothing Seihai sent after it.
///
/// The stream is the normal deflate output, sync-flushed so it ends on a
/// byte, then 0–3 empty stored blocks (5 bytes each) and an empty final
/// stored block, so any zlib reader inflates it to the same bytes.
pub fn zlib_compress_len4(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut c = flate2::Compress::new(Compression::default(), true);
    let mut out = Vec::with_capacity(data.len() + 64);
    let mut input = data;
    loop {
        out.reserve(64);
        let before = c.total_in();
        c.compress_vec(input, &mut out, flate2::FlushCompress::Sync).map_err(io::Error::other)?;
        input = &input[(c.total_in() - before) as usize..];
        if input.is_empty() && out.len() < out.capacity() {
            break;
        }
    }
    // Stream so far + k empty blocks of 5 + the final block of 5 + Adler-32.
    let k = (4 - (out.len() + 9) % 4) % 4;
    for _ in 0..k {
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0xFF, 0xFF]);
    }
    out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    out.extend_from_slice(&adler32(data).to_be_bytes());
    Ok(out)
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// Compress `data` with zlib at the best level: the stream starts `78 DA`,
/// as in the real clients' listserver frames (checked on real 2.05, 2.17.1,
/// 2.22 and 2.31 frames). Same deflate body and checksum as the default
/// level for the short login frames; only the two-byte header differs.
pub fn zlib_compress_best(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
    enc.write_all(data)?;
    enc.finish()
}

/// The largest body Seihai inflates from one frame. No real frame comes
/// near it (a 6.x long frame carrying a whole file is the biggest seen),
/// but without a bound a server could send a small compression bomb: in
/// the 2026-10-31 stress test a 60 KB bz2 frame grew one client to 617 MB,
/// and a 6.x long frame can hold enough zlib for tens of gigabytes.
pub const MAX_BODY: usize = 32 << 20;

/// Read all of `r` into a vector, failing once it passes [`MAX_BODY`].
pub(crate) fn read_bounded<R: Read>(r: R, what: &str) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    r.take(MAX_BODY as u64 + 1).read_to_end(&mut out)?;
    if out.len() > MAX_BODY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{what} body is over {} MB once decompressed", MAX_BODY >> 20),
        ));
    }
    Ok(out)
}

/// Decompress a zlib body (at most [`MAX_BODY`] bytes of output).
pub fn zlib_decompress(data: &[u8]) -> io::Result<Vec<u8>> {
    read_bounded(ZlibDecoder::new(data), "zlib")
}

/// Write a gen-2/3 frame: `u16be len` + zlib(body).
pub fn write_zlib_frame(body: &[u8]) -> io::Result<Vec<u8>> {
    let compressed = zlib_compress(body)?;
    let mut out = Vec::with_capacity(2 + compressed.len());
    out.extend_from_slice(&(compressed.len() as u16).to_be_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// As [`write_zlib_frame`] with the best level (`78 DA`). Used for the
/// listserver login only, which is where the header was compared with real
/// clients; game-server frames keep the default level.
pub fn write_zlib_frame_best(body: &[u8]) -> io::Result<Vec<u8>> {
    let compressed = zlib_compress_best(body)?;
    let mut out = Vec::with_capacity(2 + compressed.len());
    out.extend_from_slice(&(compressed.len() as u16).to_be_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// Read one gen-2/3 frame from a reader: `u16be len` then that many bytes,
/// decompressed with zlib.
pub fn read_zlib_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 2];
    r.read_exact(&mut len_buf)?;
    let len = u16::from_be_bytes(len_buf) as usize;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    zlib_decompress(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zlib_roundtrip() {
        let data = b"hello\nworld\n";
        let c = zlib_compress(data).unwrap();
        let d = zlib_decompress(&c).unwrap();
        assert_eq!(d, data);
    }

    /// The best level starts `78 DA` (the real clients), the default `78 9C`,
    /// and both decompress to the same data.
    #[test]
    fn best_level_header_is_78_da() {
        let data = b" newmain\n";
        let best = zlib_compress_best(data).unwrap();
        assert_eq!(&best[..2], &[0x78, 0xDA]);
        assert_eq!(&zlib_compress(data).unwrap()[..2], &[0x78, 0x9C]);
        assert_eq!(zlib_decompress(&best).unwrap(), data);
        let frame = write_zlib_frame_best(data).unwrap();
        assert_eq!(frame.len(), 2 + best.len());
        assert_eq!(&frame[2..4], &[0x78, 0xDA]);
    }

    #[test]
    fn frame_roundtrip() {
        let body = b"msg\n";
        let frame = write_zlib_frame(body).unwrap();
        let mut cur = std::io::Cursor::new(frame);
        let got = read_zlib_frame(&mut cur).unwrap();
        assert_eq!(got, body);
    }

    #[test]
    fn len4_zlib_is_a_multiple_of_4_and_inflates_the_same() {
        for n in 0..200usize {
            let data: Vec<u8> = (0..n).map(|i| (i * 7 % 13) as u8 + b'a').collect();
            let z = zlib_compress_len4(&data).unwrap();
            assert_eq!(z.len() % 4, 0, "{n} bytes gave {}", z.len());
            assert_eq!(zlib_decompress(&z).unwrap(), data, "{n} bytes");
            let mut d = flate2::Decompress::new(true);
            let mut back = Vec::with_capacity(n + 16);
            let st = d.decompress_vec(&z, &mut back, flate2::FlushDecompress::Finish).unwrap();
            assert_eq!(st, flate2::Status::StreamEnd);
            assert_eq!(d.total_in() as usize, z.len(), "no bytes left over");
        }
    }
}
