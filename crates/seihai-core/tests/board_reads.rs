//! `board[]` and `tiles[x,y]` read the level's tiles
//! (`documentation/board-reads.md`). The level is the `new3.nw` fixture;
//! the script is a real weapon from the NPC library.

use seihai_core::assets::Level;
use seihai_core::offline::OfflineSession;
use seihai_core::protocol::{BoardModify, ClientVersion};
use seihai_core::scripting::engine::host::{SessionScripts, WeaponScript};
use seihai_core::scripting::engine::{Effect, Val};
use std::path::Path;

/// `new3.nw` row 10, x 20 to 24: grass, then the edge tiles 158, 158, 159, 158.
const EDGE: u16 = 158;

fn new3() -> Level {
    Level::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/levels/new3.nw")).unwrap()
}

/// `new3.nw` with one NPC whose `playerenters` runs `body`, for `version`.
/// Returns the message it left.
fn read(version: ClientVersion, body: &str) -> String {
    let mut s = OfflineSession::new();
    let mut level = new3();
    let npc = Level::parse(&format!("GLEVNW01\nNPC - 1 1\nif (playerenters) {{ {body} }}\nNPCEND\n")).unwrap().npcs.remove(0);
    level.npcs.push(npc);
    s.current_level = Some(level);
    let mut sc = SessionScripts::new(version);
    sc.load_level(&mut s, "new3.nw");
    s.last_message.unwrap_or_default()
}

#[test]
fn board_reads_the_tile_at_x_plus_64_y() {
    assert_eq!(read(ClientVersion::V205, "message #v(board[21+64*10]),#v(board[23+64*10]),#v(board[20+64*10]);"), format!("{EDGE},159,0"));
    assert_eq!(read(ClientVersion::V222, "message #v(board[21+64*10]);"), EDGE.to_string());
}

#[test]
fn board_indexes_are_rounded_down_with_the_clients_small_margin() {
    // floor(i + 0.0001): 660.99995 is 661, 660.9 is 660.
    assert_eq!(read(ClientVersion::V205, "message #v(board[660.99995]),#v(board[660.9]);"), format!("{EDGE},0"));
}

#[test]
fn board_outside_0_to_4095_reads_0() {
    assert_eq!(read(ClientVersion::V205, "message #v(board[-1]),#v(board[4096]);"), "0,0");
}

#[test]
fn tiles_reads_the_tile_at_x_y_from_2_1() {
    assert_eq!(read(ClientVersion::V222, "message #v(tiles[21,10]),#v(tiles[23,10]);"), format!("{EDGE},159"));
}

#[test]
fn tiles_off_the_level_reads_tile_632() {
    assert_eq!(read(ClientVersion::V222, "message #v(tiles[64,10]),#v(tiles[21,-1]);"), "632,632");
}

#[test]
fn before_2_1_tiles_is_a_plain_variable() {
    assert_eq!(read(ClientVersion::V205, "message #v(tiles[21,10]);"), "0");
    assert_eq!(read(ClientVersion::V205, "tiles[21,10] = 5; message #v(tiles[21,10]);"), "5");
}

#[test]
fn with_no_level_both_read_0() {
    let mut s = OfflineSession::new();
    let mut sc = SessionScripts::new(ClientVersion::V222);
    sc.sync_weapons(&mut s, &[WeaponScript { name: "W", image: "", script: "if (weaponfired) { message #v(board[0]),#v(tiles[0,0]); }" }]);
    sc.sync_player(&s);
    sc.world.weapon_fired("W");
    sc.apply(&mut s);
    assert_eq!(s.last_message.as_deref(), Some("0,0"));
}

/// The Shovel weapon from the NPC library (`npc-1823`, `shovel.txt`),
/// unchanged. It reads the 2x2 tiles in front of the player with
/// `board[]` and puts a hole there only when all four are tile 15 of row
/// 31, 63 or 127 (`b % 16`, `int(b / 16)`).
const SHOVEL: &str = "if (playertouchsme) { set gotshovel; toweapons Shovel; destroy; } if (weaponfired) { putx = playerx + 1.5; puty = playery + 2; if (playerdir==0) puty -= 1; if (playerdir==1) putx -= 1; if (playerdir==2) puty += 1; if (playerdir==3) putx += 1; putx = int(putx/2)*2; puty = int(puty/2)*2; emtpy = 1; for (i=0; i<2; i++) for (j=0; j<2; j++) { b = board[putx+i+64*(puty+j)]; bx = b % 16; by = int(b / 16); if (!(bx==15 && (by==31 || by==63 || by==127))) emtpy = 0; } if (emtpy==1) { /* board[putx+64*puty] = 10+114*16; board[putx+1+64*puty] = 11+114*16; board[putx+64*(puty+1)] = 10+115*16; board[putx+1+64*(puty+1)] = 11+115*16; updateboard putx,puty,2,2;*/ putobject Hole,putx,puty; } }";

/// Fire the shovel facing down from (30.5, 27): it digs at (32, 30).
fn dig(s: &mut OfflineSession, sc: &mut SessionScripts) -> Vec<Vec<Val>> {
    s.player_x = 30.5;
    s.player_y = 27.0;
    s.player_dir = 2;
    sc.unhandled.clear();
    sc.sync_player(s);
    sc.world.weapon_fired("Shovel");
    sc.apply(s);
    sc.unhandled.iter().filter_map(|e| match e {
        Effect::Command { name, args, .. } if name == "putobject" => Some(args.clone()),
        _ => None,
    }).collect()
}

#[test]
fn the_real_shovel_digs_only_where_board_shows_the_dig_tiles() {
    let mut s = OfflineSession::new();
    s.current_level = Some(new3());
    let mut sc = SessionScripts::new(ClientVersion::V205);
    sc.sync_weapons(&mut s, &[WeaponScript { name: "Shovel", image: "", script: SHOVEL }]);
    sc.load_level(&mut s, "new3.nw");
    assert!(dig(&mut s, &mut sc).is_empty(), "grass (tile 0) at (32..33, 30..31): no hole");
    // The server changes those four tiles to 15 + 31*16 = 511 ("H/").
    s.apply_board_update(&BoardModify { x: 32, y: 30, width: 2, height: 2, tiles: b"H/H/H/H/".to_vec() });
    let holes = dig(&mut s, &mut sc);
    assert_eq!(holes.len(), 1, "the board read sees the new tiles");
    let a: Vec<String> = holes[0].iter().map(|v| v.text()).collect();
    assert_eq!(a, ["Hole", "32", "30"]);
    // One tile back to grass: no hole again.
    s.apply_board_update(&BoardModify { x: 33, y: 31, width: 1, height: 1, tiles: b"AA".to_vec() });
    assert!(dig(&mut s, &mut sc).is_empty());
}
