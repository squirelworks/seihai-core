#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::scripting::gs2::Script;
use seihai_core::scripting::Bytecode;

fuzz_target!(|data: &[u8]| {
    let _ = Script::parse(data);
    let _ = Bytecode::parse(data);
});
