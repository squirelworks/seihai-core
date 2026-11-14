//! The classic clients' DES variant and the files and strings it hides.
//!
//! Graal's DES is a d3des port that takes the key bits LSB first, which is
//! standard DES-ECB with each key byte bit-reversed (Graal_analysis
//! `tools/npc-control.md`, "Password obfuscation"). Three uses are known:
//!
//! - the 24-byte block: a length byte, up to 23 bytes of text, padding,
//!   encrypted as three ECB blocks;
//! - the 48-character string form of that block (each byte as two nibbles
//!   plus `0x20`), for the RemoteControl password and the list hosts in RC
//!   1.41 and the 2.05 client;
//! - the 1999 Guildmaker `.code` file (`guildname.code`, `reg.code`): the
//!   block encrypted under [`GUILD_KEYS`]`[0]` then, separately, under
//!   [`GUILD_KEYS`]`[1]`, 48 bytes; both halves must decrypt to the same
//!   block (Graal_analysis `tools/server-tools-2001.md`).

use des::cipher::generic_array::GenericArray;
use des::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};

/// The keys of the two halves of a `.code` file.
pub const GUILD_KEYS: [&[u8; 8]; 2] = [b"AXBORG15", b"VERGRAAL"];

/// The RemoteControl 1.33 to 1.41 password key.
pub const RC_PASSWORD_KEY: &[u8; 8] = b"\\!x<|.ak";

/// The longest text a block holds.
pub const MAX_TEXT: usize = 23;

/// DES with a Graal key.
pub struct GraalDes(des::Des);

impl GraalDes {
    /// A cipher for `key` as the clients pass it (bits reversed here).
    pub fn new(key: &[u8; 8]) -> Self {
        let k: Vec<u8> = key.iter().map(|b| b.reverse_bits()).collect();
        Self(des::Des::new(GenericArray::from_slice(&k)))
    }

    /// Encrypt whole 8-byte blocks in place; a short tail is left as is.
    pub fn encrypt(&self, data: &mut [u8]) {
        for block in data.chunks_exact_mut(8) {
            self.0.encrypt_block(GenericArray::from_mut_slice(block));
        }
    }

    /// Decrypt whole 8-byte blocks in place; a short tail is left as is.
    pub fn decrypt(&self, data: &mut [u8]) {
        for block in data.chunks_exact_mut(8) {
            self.0.decrypt_block(GenericArray::from_mut_slice(block));
        }
    }
}

/// The 24-byte plain block for `text`: its length, the text, then `pad`
/// bytes. Text over 23 bytes is cut, as the 1999 Guildmaker cut it.
pub fn block(text: &[u8], mut pad: impl FnMut() -> u8) -> [u8; 24] {
    let n = text.len().min(MAX_TEXT);
    let mut b = [0u8; 24];
    b[0] = n as u8;
    b[1..=n].copy_from_slice(&text[..n]);
    for x in &mut b[n + 1..] {
        *x = pad();
    }
    b
}

/// The text of a plain block, or `None` when its length byte is 24 or more.
pub fn block_text(b: &[u8; 24]) -> Option<&[u8]> {
    let n = usize::from(b[0]);
    (n <= MAX_TEXT).then(|| &b[1..=n])
}

/// The 48-character string form: [`block`], encrypted, each byte as two
/// characters (high nibble first, each nibble plus `0x20`).
pub fn string_encrypt(text: &str, key: &[u8; 8], pad: impl FnMut() -> u8) -> String {
    let mut b = block(text.as_bytes(), pad);
    GraalDes::new(key).encrypt(&mut b);
    b.iter()
        .flat_map(|x| [char::from((x >> 4) + 0x20), char::from((x & 15) + 0x20)])
        .collect()
}

/// Read the 48-character string form back. `None` when it is not 48
/// characters of nibbles or the length byte is out of range.
pub fn string_decrypt(text: &str, key: &[u8; 8]) -> Option<String> {
    let raw = text.as_bytes();
    if raw.len() != 48 {
        return None;
    }
    let mut b = [0u8; 24];
    for (i, pair) in raw.chunks(2).enumerate() {
        let hi = pair[0].checked_sub(0x20).filter(|n| *n < 16)?;
        let lo = pair[1].checked_sub(0x20).filter(|n| *n < 16)?;
        b[i] = (hi << 4) | lo;
    }
    GraalDes::new(key).decrypt(&mut b);
    block_text(&b).map(|t| String::from_utf8_lossy(t).into_owned())
}

/// A `.code` file (48 bytes) holding `text`, at most 23 bytes.
pub fn code_write(text: &[u8], pad: impl FnMut() -> u8) -> [u8; 48] {
    let plain = block(text, pad);
    let mut out = [0u8; 48];
    for (half, key) in out.chunks_exact_mut(24).zip(GUILD_KEYS) {
        half.copy_from_slice(&plain);
        GraalDes::new(key).encrypt(half);
    }
    out
}

/// The text of a `.code` file. Refused, as the client refuses it, when the
/// file is not 48 bytes, the two halves do not decrypt to the same block,
/// or the length byte is out of range.
pub fn code_read(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() != 48 {
        return Err(format!("a .code file is 48 bytes, not {}", bytes.len()));
    }
    let mut halves = [[0u8; 24]; 2];
    for ((h, src), key) in halves
        .iter_mut()
        .zip(bytes.chunks_exact(24))
        .zip(GUILD_KEYS)
    {
        h.copy_from_slice(src);
        GraalDes::new(key).decrypt(h);
    }
    if halves[0] != halves[1] {
        return Err("the two halves of the .code file do not match".into());
    }
    block_text(&halves[0])
        .map(<[u8]>::to_vec)
        .ok_or_else(|| "the .code file's length byte is out of range".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_des_with_reversed_key_bytes() {
        // FIPS 81 / the classic DES test vector, with the key bytes
        // reversed first so the cipher sees 133457799BBCDFF1.
        let key: Vec<u8> = [0x13, 0x34, 0x57, 0x79, 0x9B, 0xBC, 0xDF, 0xF1]
            .iter()
            .map(|b: &u8| b.reverse_bits())
            .collect();
        let mut data = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        GraalDes::new(key.as_slice().try_into().unwrap()).encrypt(&mut data);
        assert_eq!(data, [0x85, 0xE8, 0x13, 0x54, 0x0F, 0x0A, 0xB4, 0x05]);
    }

    /// Vectors from an independent implementation (Graal_analysis
    /// `tools/file-formats/graaldes.py`), padding 0, 1, 2 … 17.
    #[test]
    fn matches_the_reference_vectors() {
        let mut i = 0u8;
        let plain = block(b"Baddy", || {
            i += 1;
            i - 1
        });
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let mut a = plain;
        GraalDes::new(GUILD_KEYS[0]).encrypt(&mut a);
        assert_eq!(hex(&a), "f4115524bc77aa8981ad6567243c37409afec83fed3ed3f4");
        let mut v = plain;
        GraalDes::new(GUILD_KEYS[1]).encrypt(&mut v);
        assert_eq!(hex(&v), "b5e5583ceb78c29cf914d839dc5b5597fce1aa92bdd1251a");
        // The saved-password key of the 2.x clients, zero padding.
        assert_eq!(
            string_encrypt("secret", b"~2%$zsd^", || 0),
            "*&%&+%,%##$)($/+%\"/\"\". \"-.-%*./(%\"/\"\". \"-.-%*./("
        );
    }

    #[test]
    fn code_files_round_trip_and_reject_damage() {
        let mut n = 7u8;
        let file = code_write(b"Baddy", || {
            n = n.wrapping_mul(31).wrapping_add(1);
            n
        });
        assert_eq!(code_read(&file).unwrap(), b"Baddy");
        let mut bad = file;
        bad[30] ^= 1;
        assert!(code_read(&bad).unwrap_err().contains("halves"));
        assert!(code_read(&file[..47]).is_err());
        let long = code_write(b"if (playersays(killall)) {", || 0);
        assert_eq!(code_read(&long).unwrap(), b"if (playersays(killall)");
    }

    #[test]
    fn strings_round_trip() {
        let s = string_encrypt("hunter2", RC_PASSWORD_KEY, || 0x55);
        assert_eq!(s.len(), 48);
        assert!(s.bytes().all(|c| (0x20..0x30).contains(&c)));
        assert_eq!(
            string_decrypt(&s, RC_PASSWORD_KEY).as_deref(),
            Some("hunter2")
        );
        assert_eq!(string_decrypt("short", RC_PASSWORD_KEY), None);
    }
}
