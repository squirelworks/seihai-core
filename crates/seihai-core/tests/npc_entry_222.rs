//! What 2.22 sends for a server NPC on the first entry of a level: the props
//! its `created` set that the server's copy lacks, under the 2.05 rule
//! (`documentation/gmap-order.md`, "NPC props on entry for 2.22").
//!
//! Bytes: the Linux lab, 2026-10-31, `lo` capture of Seihai 2.22 (main
//! `0613858`) entering `testworld.nw` by a link, on the lab GServer-v2. The
//! server's record is its `PLO_NPCPROPS` for the warp NPC; the reply is the
//! 63-byte `PLI_NPCPROPS` Seihai sent 46 ms later. The real 2.22 sent a
//! 63-byte message for the same NPC on the same entry (Mirae, `lan-222g`).

use seihai_core::offline::{Linked, OfflineSession, ServerNpcSync};
use seihai_core::protocol::npcprops::parse_npc_props;
use seihai_core::protocol::ClientVersion;
use seihai_core::scripting::engine::host::SessionScripts;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// The server's `PLO_NPCPROPS` payload for testworld's warp NPC: the script,
/// the position and VISFLAGS, none of the looks or the message.
const SERVER_WARP_NPC: &str = "2022512020212147696620286372656174656429207b2073686f776368617261637465723b207365746368617270726f702023332c68656164322e706e673b2073657463686172616e692069646c652c3b20646972203d20323b206d65737361676520546f20746865206d696e6920706c61796572776f726c64213b207da769662028706c61796572746f756368736d6529207365746c6576656c32206d696e6970772e676d61702c39322c39363b226823602d212f203120225132224bff6b29206c28206d2c81";

/// The `PLI_NPCPROPS` payload after the message id: `#c#`, GANI `idle`,
/// VISFLAGS 17, the message, HEAD `head2.png` and BODY `body.png`.
const SENT_WARP_NPC: &str = "20225120232363232c2469646c652d312f38546f20746865206d696e6920706c61796572776f726c6421368d68656164322e706e674328626f64792e706e67";

#[test]
fn on_2_22_the_first_entry_reports_what_created_set() {
    let (id, p) = parse_npc_props(&hex(SERVER_WARP_NPC), false).expect("the server's record parses");
    let mut s = OfflineSession::new();
    s.current_level = Some(seihai_core::assets::Level::from_board(&vec![0u16; 4096]));
    let mut sc = SessionScripts::new(ClientVersion::V222);
    sc.load_level(&mut s, "testworld.nw");
    let Linked::Added(i) = s.take_server_npc(id, &p) else { panic!("{id} not added") };
    sc.add_npc(&mut s, i);
    let mut sync = ServerNpcSync::default();
    sync.heard(id, &p, &s.current_level.as_ref().unwrap().npcs[i]);
    sc.tick(&mut s, 16);
    let sent = sync.changes(s.current_level.as_ref().unwrap());
    let want = [&[3 + 0x20][..], &hex(SENT_WARP_NPC), b"\n"].concat();
    assert_eq!(sent, vec![want]);
    assert!(sync.changes(s.current_level.as_ref().unwrap()).is_empty(), "nothing twice");
}
