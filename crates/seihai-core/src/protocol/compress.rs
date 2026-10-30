//! Gen-5 compression helpers.
//!
//! The XOR cipher lives in `gen5.rs`. This module supplies the
//! compression choice and the bz2 path used by bodies ≥ 0x1000 bytes.

use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;
use bzip2::Compression;
use std::io::{self, Read, Write};

/// Compress `data` with bz2.
pub fn bz2_compress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut enc = BzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data)?;
    enc.finish()
}

/// Decompress a bz2 body.
pub fn bz2_decompress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut dec = BzDecoder::new(data);
    let mut out = Vec::new();
    dec.read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bz2_roundtrip() {
        let data = b"hello gen5 bz2 body";
        let c = bz2_compress(data).unwrap();
        let d = bz2_decompress(&c).unwrap();
        assert_eq!(d, data);
    }
}
