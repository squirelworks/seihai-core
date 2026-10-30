//! How the real 2.22 client sends its own walk and sword props.
//!
//! Captures (lab `gmap_cross` and `door_warp`, and the live Graal the
//! Adventure run in Graal_analysis `live-runs/2gta-222-2026-10-30/`):
//!
//! * at most one `PLI_PLAYERPROPS` per 100 ms, carrying only the props that
//!   changed since the last one, in ascending id order;
//! * the first step: `GANI` (`walk`), the axis that moved, and `SPRITE` when
//!   the direction changed; the next steps: the moving axis alone, so one
//!   message per tile at the 0.5 tile per 50 ms walk;
//! * a stop on a half tile: that position in the next 100 ms slot, then
//!   `GANI` (`idle`) in the one after;
//! * a sword swing: `GANI` (`sword`) with no `SPRITE`, in the same frame as
//!   the `PLI_BOARDMODIFY` of the bush it cuts.
//!
//! The real 3.04 (lab, 2026-10-30, `documentation/login-burst.md`)
//! sends the same walk: `walk` + X + `SPRITE`, then X every 100 ms. It sent
//! `idle` one tick after the last step, with that step's position, where
//! 2.22 waits two.
//!
//! The real 2.17.1 (lab, 2026-10-30, `documentation/difftest-217.md` item 4)
//! walks the same way: `walk` + X + `SPRITE`, then the moving axis alone
//! every 100 ms. Its stop is 3.04's: a stop on a half tile goes out with
//! `idle` in the next slot (`X=36`, then `GANI=idle, X=35.5` 100 ms later),
//! and a stop on a whole tile sends `idle` alone in the next slot.
//!
//! The 2.31 client sends `X2`/`Y2` every 50 ms instead, and is not handled
//! here.

use super::props::encode_pos_classic;
use super::ClientVersion;

/// The 2.22 client's shortest gap between two of its own props messages,
/// in 50 ms ticks.
pub const INTERVAL_TICKS: u32 = 2;

/// Ticks the GANI stays `walk` after the last step before `idle` is sent:
/// in both 2.22 captures `idle` went out two ticks after the last step.
pub const IDLE_AFTER_TICKS: u32 = 2;
/// The same for the real 3.04: one tick (one stop, lab 2026-10-30).
pub const IDLE_AFTER_TICKS_304: u32 = 1;
/// The same for the real 2.17.1: one tick, as 3.04 (five stops away from a wall in the two
/// 2026-10-30 `gmap_cross` captures).
pub const IDLE_AFTER_TICKS_217: u32 = 1;

/// What the player shows, as the props would carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkState {
    /// `X` and `Y` in half tiles.
    pub x: u8,
    pub y: u8,
    pub sprite: u8,
    /// The GANI name, when the pose is one this sender names (`walk`,
    /// `idle`, `sword`). `None` leaves the GANI prop alone.
    pub gani: Option<&'static str>,
}

impl WalkState {
    pub fn new(x: f32, y: f32, sprite: u8, gani: Option<&'static str>) -> Self {
        Self { x: encode_pos_classic(x), y: encode_pos_classic(y), sprite, gani }
    }
}

/// The 2.22 props sender. [`tick`](Self::tick) is called once per 50 ms
/// tick; it holds what was last sent and the tick counts it needs.
#[derive(Debug, Clone)]
pub struct WalkCadence {
    sent: Option<WalkState>,
    tick: u32,
    sent_tick: Option<u32>,
    pos: (u8, u8),
    moved_tick: u32,
    idle_after: u32,
}

impl WalkCadence {
    /// The builds this sender is for: 2.17, 2.22 and 3.04.
    pub fn for_version(v: ClientVersion) -> Option<Self> {
        let idle_after = match v {
            ClientVersion::V217 => IDLE_AFTER_TICKS_217,
            ClientVersion::V222 => IDLE_AFTER_TICKS,
            ClientVersion::V304 => IDLE_AFTER_TICKS_304,
            _ => return None,
        };
        Some(Self { sent: None, tick: 0, sent_tick: None, pos: (0, 0), moved_tick: 0, idle_after })
    }

    /// The last props sent had `walk` as the GANI: the server knows the
    /// player is walking.
    pub fn walk_sent(&self) -> bool {
        self.sent.as_ref().is_some_and(|s| s.gani == Some("walk"))
    }

    /// One 50 ms tick: the props message to send in this tick's frame, if
    /// anything changed since the last one and two ticks have passed since
    /// it. The first call only records the state: the client sends nothing
    /// until the player does something.
    pub fn tick(&mut self, cur: WalkState) -> Option<Vec<u8>> {
        self.tick = self.tick.wrapping_add(1);
        let n = self.tick;
        if (cur.x, cur.y) != self.pos {
            self.pos = (cur.x, cur.y);
            self.moved_tick = n;
        }
        let Some(last) = self.sent else {
            self.sent = Some(cur);
            return None;
        };
        // A stop keeps `walk` for a tick or two after the last step.
        let settling = last.gani == Some("walk") && n.wrapping_sub(self.moved_tick) < self.idle_after;
        let gani = cur.gani.filter(|g| last.gani != Some(*g) && !(settling && *g == "idle"));
        if gani.is_none() && cur.x == last.x && cur.y == last.y && cur.sprite == last.sprite {
            return None;
        }
        if self.sent_tick.is_some_and(|t| n.wrapping_sub(t) < INTERVAL_TICKS) {
            return None;
        }
        let mut m = vec![2 + 0x20];
        if let Some(g) = gani {
            m.push(10 + 0x20);
            m.push(g.len() as u8 + 0x20);
            m.extend(g.as_bytes());
        }
        if cur.x != last.x {
            m.extend([15 + 0x20, cur.x.wrapping_add(0x20)]);
        }
        if cur.y != last.y {
            m.extend([16 + 0x20, cur.y.wrapping_add(0x20)]);
        }
        if cur.sprite != last.sprite {
            m.extend([17 + 0x20, cur.sprite.wrapping_add(0x20)]);
        }
        m.push(b'\n');
        self.sent = Some(WalkState { gani: gani.or(last.gani), ..cur });
        self.sent_tick = Some(n);
        Some(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(m: &[u8]) -> String {
        String::from_utf8_lossy(&m[1..m.len() - 1]).into_owned()
    }

    fn hx(v: f32) -> char {
        ((v * 2.0) as u8 + 0x20) as char
    }

    /// Run `ticks` (position, gani) through a sender that starts idle at
    /// `start`, facing down; the result is (tick, props) of each message.
    fn run(start: f32, ticks: &[(f32, &'static str)], sprite: u8) -> Vec<(usize, String)> {
        run_as(ClientVersion::V222, start, ticks, sprite)
    }

    fn run_as(v: ClientVersion, start: f32, ticks: &[(f32, &'static str)], sprite: u8) -> Vec<(usize, String)> {
        let mut c = WalkCadence::for_version(v).unwrap();
        assert_eq!(c.tick(WalkState::new(start, 30.0, 2, Some("idle"))), None);
        // Long idle first, as in both captures.
        for _ in 0..10 {
            assert_eq!(c.tick(WalkState::new(start, 30.0, 2, Some("idle"))), None);
        }
        ticks.iter().enumerate().filter_map(|(i, &(x, g))| c.tick(WalkState::new(x, 30.0, sprite, Some(g))).map(|m| (i, props(&m)))).collect()
    }

    /// Lab 2026-10-30, real 2.22, right from 30 for eleven steps: walk +
    /// X + SPRITE at once, then X every second tick, idle two ticks after
    /// the last step.
    #[test]
    fn the_lab_walk_right() {
        let mut t: Vec<(f32, &str)> = (1..=11).map(|i| (30.0 + 0.5 * i as f32, "walk")).collect();
        t.extend([(35.5, "idle"), (35.5, "idle"), (35.5, "idle")]);
        assert_eq!(
            run(30.0, &t, 3),
            vec![
                (0, format!("*$walk/{}1#", hx(30.5))),
                (2, format!("/{}", hx(31.5))),
                (4, format!("/{}", hx(32.5))),
                (6, format!("/{}", hx(33.5))),
                (8, format!("/{}", hx(34.5))),
                (10, format!("/{}", hx(35.5))),
                (12, "*$idle".to_string()),
            ]
        );
    }

    /// `walk_sent` turns on with the frame that tells the server `walk`,
    /// and off with the one that tells it `idle`.
    #[test]
    fn walk_sent_follows_the_sent_gani() {
        let mut c = WalkCadence::for_version(ClientVersion::V222).unwrap();
        c.tick(WalkState::new(30.0, 30.0, 3, Some("idle")));
        assert!(!c.walk_sent());
        assert!(c.tick(WalkState::new(30.5, 30.0, 3, Some("walk"))).is_some());
        assert!(c.walk_sent());
        for _ in 0..4 {
            c.tick(WalkState::new(30.5, 30.0, 3, Some("idle")));
        }
        assert!(!c.walk_sent());
    }

    /// Graal the Adventure: the last step on a tick between two sends; the
    /// position goes in the next slot, idle in the one after.
    #[test]
    fn the_gta_walk_stops_between_two_sends() {
        let mut t: Vec<(f32, &str)> = (0..8).map(|i| (42.0 + 0.5 * i as f32, "walk")).collect();
        t.extend([(45.5, "idle"), (45.5, "idle"), (45.5, "idle"), (45.5, "idle")]);
        assert_eq!(
            run(41.5, &t, 3),
            vec![
                (0, format!("*$walk/{}1#", hx(42.0))),
                (2, format!("/{}", hx(43.0))),
                (4, format!("/{}", hx(44.0))),
                (6, format!("/{}", hx(45.0))),
                (8, format!("/{}", hx(45.5))),
                (10, "*$idle".to_string()),
            ]
        );
    }

    /// Lab 2026-10-30, the real 3.04, right from 30 to 41: walk + X +
    /// SPRITE, X every second tick on the half tiles, and the last step
    /// (41, a tick after 40.5) goes with `idle`.
    #[test]
    fn the_304_walk_stops_with_idle_in_the_same_message() {
        let mut t: Vec<(f32, &str)> = (1..=22).map(|i| (30.0 + 0.5 * i as f32, "walk")).collect();
        t.extend([(41.0, "idle"), (41.0, "idle"), (41.0, "idle")]);
        let mut want: Vec<(usize, String)> = vec![(0, format!("*$walk/{}1#", hx(30.5)))];
        want.extend((1..=10).map(|i| (2 * i, format!("/{}", hx(30.5 + i as f32)))));
        want.push((22, format!("*$idle/{}", hx(41.0))));
        assert_eq!(run_as(ClientVersion::V304, 30.0, &t, 3), want);
    }

    /// Lab 2026-10-30, real 2.17.1 (`2.17-gmap_cross-real.trace.tsv`): left
    /// from 47.5 to 35.5; X every 100 ms on the whole tiles, and the last
    /// step, a tick after 36, goes with `idle` (`X=36` at 21019 ms, then
    /// `GANI=idle, X=35.5` at 21119 ms).
    #[test]
    fn the_217_walk_stops_with_idle_in_the_same_message() {
        let mut t: Vec<(f32, &str)> = (1..=24).map(|i| (47.5 - 0.5 * i as f32, "walk")).collect();
        t.extend([(35.5, "idle"), (35.5, "idle"), (35.5, "idle")]);
        let mut want: Vec<(usize, String)> = vec![(0, format!("*$walk/{}1!", hx(47.0)))];
        want.extend((1..=11).map(|i| (2 * i, format!("/{}", hx(47.0 - i as f32)))));
        want.push((24, format!("*$idle/{}", hx(35.5))));
        assert_eq!(run_as(ClientVersion::V217, 47.5, &t, 1), want);
    }

    /// Real 2.17.1, a stop on a whole tile (`Y=52` at 31603 ms in
    /// `2.17-gmap_cross-real-old-cached.trace.tsv`): `idle` alone 100 ms later.
    #[test]
    fn the_217_walk_stopping_on_a_whole_tile_sends_idle_alone() {
        let mut t: Vec<(f32, &str)> = (1..=5).map(|i| (30.5 + 0.5 * i as f32, "walk")).collect();
        t.extend([(33.0, "idle"), (33.0, "idle"), (33.0, "idle")]);
        assert_eq!(
            run_as(ClientVersion::V217, 30.5, &t, 3),
            vec![(0, format!("*$walk/{}1#", hx(31.0))), (2, format!("/{}", hx(32.0))), (4, format!("/{}", hx(33.0))), (6, "*$idle".to_string())]
        );
    }

    #[test]
    fn a_sword_swing_sends_its_gani_without_a_sprite() {
        assert_eq!(run(30.0, &[(30.0, "sword"), (30.0, "sword")], 2), vec![(0, "*%sword".to_string())]);
        assert!(WalkCadence::for_version(ClientVersion::V231).is_none());
    }
}
