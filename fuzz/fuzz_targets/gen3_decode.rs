#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::protocol::Gen3;

fuzz_target!(|data: &[u8]| {
    let Some((&key, rest)) = data.split_first() else {
        return;
    };
    let mut codec = Gen3::new(key & 0x7F);
    let _ = codec.decode(rest);
});
