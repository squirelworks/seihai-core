//! Gen-5 compression helpers.
//!
//! The XOR cipher lives in `gen5.rs`. This module supplies the
//! compression choice and the bz2 path used by bodies ≥ 0x1000 bytes.

use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;
use bzip2::Compression;
use std::io::{self, Write};

/// Compress `data` with bz2.
pub fn bz2_compress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut enc = BzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data)?;
    enc.finish()
}

/// Decompress a bz2 body (at most [`super::frame::MAX_BODY`] bytes of
/// output; bz2 packs a long run of one byte far tighter than zlib).
pub fn bz2_decompress(data: &[u8]) -> io::Result<Vec<u8>> {
    super::frame::read_bounded(BzDecoder::new(data), "bz2")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bz2_bomb_is_refused() {
        let body = vec![b'A'; super::super::frame::MAX_BODY + 1];
        let c = bz2_compress(&body).unwrap();
        assert!(c.len() < 64 * 1024, "the bomb fits one short frame");
        let e = bz2_decompress(&c).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        let ok = vec![b'A'; 1 << 20];
        assert_eq!(bz2_decompress(&bz2_compress(&ok).unwrap()).unwrap(), ok);
    }

    #[test]
    fn zlib_bomb_is_refused() {
        let body = vec![0u8; super::super::frame::MAX_BODY + 1];
        let c = super::super::zlib_compress(&body).unwrap();
        assert!(super::super::zlib_decompress(&c).is_err());
    }

    #[test]
    fn bz2_roundtrip() {
        let data = b"hello gen5 bz2 body";
        let c = bz2_compress(data).unwrap();
        let d = bz2_decompress(&c).unwrap();
        assert_eq!(d, data);
    }
}
