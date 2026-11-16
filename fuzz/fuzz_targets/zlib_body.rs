#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::protocol::{read_zlib_frame, zlib_decompress};
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let _ = zlib_decompress(data);
    let mut cur = Cursor::new(data);
    let _ = read_zlib_frame(&mut cur);
});
