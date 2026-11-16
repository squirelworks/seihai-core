#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::gmap::Gmap;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = Gmap::parse(text);
});
