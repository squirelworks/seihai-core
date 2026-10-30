//! new4.nw: shallow water, water, deep water, lava and lava floor, with
//! the lists those tiles have in the 2.22 `arrays.dat`.

use seihai_core::assets::{TileKind, TileTypes};
use seihai_core::offline::{OfflineSession, FOOT_DX, FOOT_DY};
use std::path::Path;

const SHALLOW: u16 = 398;
const WATER: u16 = 461;
const LAVA: u16 = 2882;
const LAVA_FLOOR: u16 = 2958;
const DEEP: u16 = 3644;

/// Arrays 0 to 3 (passable, liquid, throw-through, lava) of an
/// `arrays.dat`, holding only the new4 tiles as the 2.22 file lists them.
fn arrays() -> TileTypes {
    let lists: [&[u32]; 4] = [&[0, 398, 461, 472, 2958], &[461, 472, 2882], &[], &[2882, 2958]];
    let mut data = Vec::new();
    for l in lists {
        data.extend_from_slice(&(l.len() as u32).to_be_bytes());
        for id in l {
            data.extend_from_slice(&id.to_be_bytes());
        }
    }
    TileTypes::parse_arrays(&data).unwrap()
}

fn session() -> OfflineSession {
    let mut s = OfflineSession::new();
    s.load_level(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/levels/new4.nw")).unwrap();
    s.tiletypes = Some(arrays());
    s
}

/// Stand with the feet at `(fx, fy)`.
fn feet_at(s: &mut OfflineSession, fx: f32, fy: f32) {
    s.player_x = fx - FOOT_DX;
    s.player_y = fy - FOOT_DY;
    s.refresh_water();
}

fn foot_tile(s: &OfflineSession) -> u16 {
    let (fx, fy) = s.foot();
    s.current_level.as_ref().unwrap().tile_id_at(fx as i32, fy as i32).unwrap()
}

#[test]
fn the_fixture_has_each_kind() {
    let s = session();
    let l = s.current_level.as_ref().unwrap();
    let t = s.tiletypes.as_ref().unwrap();
    for (x, y, id, kind) in [(33, 7, SHALLOW, TileKind::Open), (38, 18, WATER, TileKind::Water), (30, 12, DEEP, TileKind::Blocking), (28, 24, LAVA, TileKind::Lava), (28, 21, LAVA_FLOOR, TileKind::Open)] {
        assert_eq!(l.tile_id_at(x, y), Some(id), "({x}, {y})");
        assert_eq!(t.kind(id), kind, "{id}");
    }
}

#[test]
fn deep_water_blocks_the_walk() {
    let mut s = session();
    // Feet on the grass at (30, 7), deep water from y 9 down.
    feet_at(&mut s, 30.5, 7.5);
    let start = s.player_y;
    for _ in 0..20 {
        s.try_move(0.0, 0.5);
        assert_ne!(foot_tile(&s), DEEP);
    }
    assert!(s.player_y > start, "the walk went down");
    assert!(s.player_y + FOOT_DY < 9.0, "stopped before the deep water");
    assert!(!s.in_water);
}

#[test]
fn water_and_lava_are_swum_and_shallow_water_and_lava_floor_walked() {
    let mut s = session();
    feet_at(&mut s, 38.5, 19.5);
    assert_eq!(foot_tile(&s), WATER);
    assert!(s.in_water, "water");
    feet_at(&mut s, 28.5, 25.5);
    assert_eq!(foot_tile(&s), LAVA);
    assert!(s.in_water, "lava is in the liquid list");
    feet_at(&mut s, 28.5, 21.25);
    assert_eq!(foot_tile(&s), LAVA_FLOOR);
    assert!(!s.in_water, "lava floor is walked on");
    feet_at(&mut s, 33.5, 7.5);
    assert_eq!(foot_tile(&s), SHALLOW);
    assert!(!s.in_water, "shallow water is walked on");
}

#[test]
fn lava_burns_half_a_heart_after_101_ticks() {
    for (fx, fy) in [(28.5, 25.5), (28.5, 21.25)] {
        let mut s = session();
        s.set_hearts_half(6);
        feet_at(&mut s, fx, fy);
        assert!(s.on_lava(), "({fx}, {fy})");
        assert_eq!(s.tick_lava(100 * 50), 0, "100 ticks do nothing");
        assert_eq!(s.tick_lava(50), 1, "the 101st burns");
        assert_eq!(s.hearts_half(), 5);
        // The count starts again.
        assert_eq!(s.tick_lava(100 * 50), 0);
        assert_eq!(s.tick_life(50).len(), 0);
        assert_eq!(s.hearts_half(), 4, "tick_life runs the burn");
    }
}

#[test]
fn leaving_lava_resets_the_count() {
    let mut s = session();
    s.set_hearts_half(6);
    feet_at(&mut s, 28.5, 25.5);
    assert_eq!(s.tick_lava(90 * 50), 0);
    feet_at(&mut s, 33.5, 7.5);
    assert!(!s.on_lava());
    s.tick_lava(50);
    feet_at(&mut s, 28.5, 25.5);
    assert_eq!(s.tick_lava(100 * 50), 0, "counted from 0 again");
    assert_eq!(s.hearts_half(), 6);
}
