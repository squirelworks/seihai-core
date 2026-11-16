#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::assets::Gani;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = Gani::parse(text);
});
