#![no_main]
use libfuzzer_sys::fuzz_target;
use seihai_core::protocol::{FrameLen, Gen5, Gen5Comp};

fuzz_target!(|data: &[u8]| {
    let Some((&key, rest)) = data.split_first() else {
        return;
    };
    let _ = FrameLen::Short.header(rest);
    let _ = FrameLen::Long.header(rest);
    let mut cipher = Gen5::new(key & 0x7F);
    // Treat the rest as a compressed body after the compression byte when
    // present; decode must not panic on junk.
    if let Some((&comp, body)) = rest.split_first() {
        let kind = match comp {
            0x02 => Some(Gen5Comp::None),
            0x04 => Some(Gen5Comp::Zlib),
            0x06 => Some(Gen5Comp::Bz2),
            _ => None,
        };
        if let Some(kind) = kind {
            let _ = cipher.decode(body.to_vec(), kind);
        }
    }
});
