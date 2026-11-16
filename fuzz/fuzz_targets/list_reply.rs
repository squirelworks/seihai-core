#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::protocol::{parse_list_reply, parse_svrlist};

fuzz_target!(|data: &[u8]| {
    let _ = parse_list_reply(data);
    let _ = parse_svrlist(data);
});
