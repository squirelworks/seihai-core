#![no_main]
//! Every server→client decoder behind `SessionState::apply`: the input is
//! split into messages as a frame body is, and each message goes through
//! a 2.05, a 2.22 and a 5.12 session (different receive sets and layouts).
use libfuzzer_sys::fuzz_target;
use seihai_core::protocol::{split_messages, ClientVersion, SessionState};

fuzz_target!(|data: &[u8]| {
    let msgs = split_messages(data);
    for v in [ClientVersion::V205, ClientVersion::V222, ClientVersion::V512] {
        let mut st = SessionState::for_version(v);
        for m in &msgs {
            st.apply(m);
        }
    }
});
