#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::protocol::parse_player_props;

fuzz_target!(|data: &[u8]| {
    let _ = parse_player_props(data);
});
