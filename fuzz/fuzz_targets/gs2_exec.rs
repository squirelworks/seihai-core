#![no_main]
//! The GS2 executor on whatever bytecode the reader accepts: top level,
//! then `onCreated`, with the 3.153 and 6.x VM rules. The machine's own
//! step limit bounds each run.
use libfuzzer_sys::fuzz_target;
use seihai_core::scripting::engine::Era;
use seihai_core::scripting::gs2::exec::{EffectHost, Machine};
use seihai_core::scripting::gs2::vm;
use seihai_core::scripting::gs2::Script;

fuzz_target!(|data: &[u8]| {
    let Ok(script) = Script::parse(data) else {
        return;
    };
    for profile in [&vm::V3153, &vm::V500R7] {
        let mut m = Machine::with_profile(script.clone(), profile);
        let mut h = EffectHost::new(Era::V217, 1);
        let _ = m.run_top_level(&mut h);
        let _ = m.call_event("onCreated", &[], &mut h);
        let _ = m.tick(&mut h);
    }
});
