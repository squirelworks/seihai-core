#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::scripting::engine::{parse, Era};

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    // Cap so libFuzzer can still explore deep ASTs without burning the
    // weekly budget on one huge input.
    if src.len() > 64 * 1024 {
        return;
    }
    let _ = parse(src, Era::V205);
    let _ = parse(src, Era::V140);
});
