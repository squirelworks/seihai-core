#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::assets::Level;

fuzz_target!(|data: &[u8]| {
    let _ = Level::from_bytes(data);
});
