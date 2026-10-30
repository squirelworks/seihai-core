//! How many NPC prop messages Seihai sends while the scripts run: none for
//! NPCs whose scripts keep setting the same values, and one every 100 ms
//! for an NPC a script keeps moving or a `setcharani` loop, as the 2.05
//! client did in the lab (`documentation/ledges-push-pull.md`).

use seihai_core::offline::{Linked, OfflineSession, ServerNpcSync, NPC_SEND_INTERVAL};
use seihai_core::protocol::npcprops::NpcProps;
use seihai_core::protocol::ClientVersion;
use seihai_core::scripting::engine::host::SessionScripts;
use std::collections::BTreeMap;
use std::time::Duration;

/// Run `scripts` (id, script) as server NPCs for `secs` seconds of 16 ms
/// frames and count the messages each frame's sync sent, per second.
fn messages_per_second(scripts: &[(u32, &str)], secs: u64) -> Vec<usize> {
    let mut s = OfflineSession::new();
    s.current_level = Some(seihai_core::assets::Level::from_board(&vec![0u16; 4096]));
    s.player_x = 30.0;
    s.player_y = 50.0;
    let mut sc = SessionScripts::new(ClientVersion::V205);
    sc.load_level(&mut s, "x.nw");
    let mut sync = ServerNpcSync::default();
    let mut heard = BTreeMap::new();
    for (k, &(id, body)) in scripts.iter().enumerate() {
        let p = NpcProps { image: Some("a.png".into()), script: Some(body.into()), x: Some(20.0 + 4.0 * k as f32), y: Some(30.0), ..Default::default() };
        let Linked::Added(i) = s.take_server_npc(id, &p) else { panic!("{id} not added") };
        sc.add_npc(&mut s, i);
        heard.insert(id, (i, p));
    }
    for (id, (i, p)) in &heard {
        sync.heard(*id, p, &s.current_level.as_ref().unwrap().npcs[*i]);
    }
    let frame = Duration::from_millis(16);
    let mut per_second = vec![0; secs as usize];
    let mut t = Duration::ZERO;
    while t < Duration::from_secs(secs) {
        sc.tick(&mut s, frame.as_millis() as u64);
        per_second[t.as_secs() as usize] += sync.tick(s.current_level.as_ref().unwrap(), frame).len();
        t += frame;
    }
    per_second
}

#[test]
fn npcs_whose_scripts_set_the_same_values_send_nothing_after_the_first_round() {
    let n = messages_per_second(
        &[
            (1, "//"),
            (2, "if (created||timeout) { message hi; dir=2; timeout=0.05; }\n"),
            (3, "if (created) { showcharacter; setcharprop #3,head0.png; message yo; } if (created||timeout) { dir=2; timeout=0.05; }\n"),
            (4, "if (created||timeout) { canbepushed; hide; show; timeout=0.1; }\n"),
        ],
        5,
    );
    // The first round says what the scripts set up (one message per NPC at
    // most); after that, five seconds of nothing.
    assert!(n[0] <= 4, "{n:?}");
    assert_eq!(n[1..].iter().sum::<usize>(), 0, "{n:?}");
}

/// What 2.05 sent in the lab: X every 0.1 s for an NPC a 0.05 s timeout
/// moves, and GANI every 0.1 s for a `setcharani` loop with the same
/// animation. Ten a second, never more.
#[test]
fn a_moving_npc_and_a_setcharani_loop_go_out_every_100_ms() {
    for script in [
        "if (created||timeout) { x+=0.5; if (x>40) x=20; timeout=0.05; }\n",
        "if (created||timeout) { showcharacter; setcharani idle,; timeout=0.05; }\n",
    ] {
        let n = messages_per_second(&[(1, script)], 5);
        let most = (Duration::from_secs(1).as_millis() / NPC_SEND_INTERVAL.as_millis()) as usize;
        for (sec, &count) in n.iter().enumerate() {
            assert!(count <= most && count >= most - 1, "{script}second {sec}: {count} messages ({n:?})");
        }
    }
}
