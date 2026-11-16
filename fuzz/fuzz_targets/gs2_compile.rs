#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::scripting::gs2::compile::{compile_gs1, compile_gs2, compile_gs2_legacy};
use seihai_core::scripting::engine::Era;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    if src.len() > 32 * 1024 {
        return;
    }
    let _ = compile_gs1(src, Era::V205);
    let _ = compile_gs2(src);
    let _ = compile_gs2_legacy(src);
});
