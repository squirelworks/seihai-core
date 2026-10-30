//! Health and death, the way the 2.05 client runs them.
//!
//! Source: `decomp-findings-2026-10-24/01-death-respawn.md` and
//! `constants-205.tsv` (derived behaviour of the 2.05 rev 2 client), and the
//! server side in Graal_analysis `04-death-respawn.md`. The numbers come from
//! the version profile's [`DeathRules`]; nothing here names a version. A
//! build without a row has no death sequence, so hearts at 0 do nothing.
//!
//! The model is pure: [`Life`] knows time and state, the session (see
//! [`crate::offline::OfflineSession`]) feeds it the hearts, and the game loop
//! turns the [`LifeEvent`]s it returns into messages with [`messages_for`].
//! `documentation/health-death.md` lists what is not known.

use crate::protocol::{ClientVersion, DeathRules};

/// `PLI_CLAIMPKER` (14), client to server: `GSHORT attacker player id`. The
/// victim's client sends it when its hearts reach 0 after a hit by another
/// player.
pub const PLI_CLAIMPKER: u8 = 14;
/// `PLPROP_STATUS` (18), a `GCHAR` of bit flags.
pub const PLPROP_STATUS: u8 = 18;
/// The dead bit of the STATUS prop.
pub const STATUS_DEAD: u8 = 0x08;
/// One client tick, in milliseconds.
pub const TICK_MS: u64 = 50;

/// The `PLI_CLAIMPKER` message: the id and the attacker's player id as a
/// GSHORT (the layout in Graal_analysis `packet-layouts.md`: `s player id`).
pub fn claim_message(attacker: u16) -> Vec<u8> {
    let mut m = vec![PLI_CLAIMPKER + 0x20];
    m.extend_from_slice(&crate::net::gshort(attacker));
    m.push(b'\n');
    m
}

/// `PLI_PLAYERPROPS` (2) carrying only STATUS (18).
pub fn status_message(status: u8) -> Vec<u8> {
    vec![2 + 0x20, PLPROP_STATUS + 0x20, (status & 0x7F) + 0x20, b'\n']
}

/// `base` with the dead bit set or cleared.
pub fn with_dead_bit(base: u8, dead: bool) -> u8 {
    if dead {
        base | STATUS_DEAD
    } else {
        base & !STATUS_DEAD
    }
}

/// Hearts after a respawn, in half hearts, from alignment points and the
/// maximum in half hearts: below `low_ap_below` AP the low value (3 hearts),
/// below `mid_ap_below` min(5 hearts, max), otherwise the maximum.
pub fn respawn_half_hearts(rules: &DeathRules, ap: u8, max_half: i32) -> i32 {
    let max_half = max_half.max(0);
    if ap < rules.low_ap_below {
        i32::from(rules.low_hearts) * 2
    } else if ap < rules.mid_ap_below {
        (i32::from(rules.mid_hearts_cap) * 2).min(max_half)
    } else {
        max_half
    }
}

/// Something the death sequence did that the game loop must act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeEvent {
    /// Hearts reached 0: the sequence started. `attacker` is the player
    /// whose hit did it, when one is known. The `playerdies` script event
    /// fires and the victim claims the kill now (not at the status bit).
    Died { attacker: Option<u16> },
    /// The timer reached `status_bit_from`: the dead bit goes out in STATUS.
    StatusDead,
    /// The player acted after the lock and respawned. `clear_status` is true
    /// when the dead bit had been sent and must now be cleared.
    Respawned { clear_status: bool },
}

/// The death sequence of one session.
#[derive(Debug, Clone, Default)]
pub struct Life {
    rules: Option<DeathRules>,
    dead: bool,
    /// Death timer in ticks, counts up and stops at `timer_cap`.
    timer: u16,
    /// Milliseconds not yet worth a tick.
    acc_ms: u64,
    /// The attacker of the latest hit, valid for the next [`Life::tick`].
    attacker: Option<u16>,
    status_sent: bool,
    events: Vec<LifeEvent>,
}

impl Life {
    pub fn new(rules: Option<DeathRules>) -> Self {
        Life { rules, ..Life::default() }
    }

    pub fn rules(&self) -> Option<&DeathRules> {
        self.rules.as_ref()
    }

    /// Take the rules of another build. Ends any death in progress.
    pub fn set_rules(&mut self, rules: Option<DeathRules>) {
        *self = Life::new(rules);
    }

    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// The death timer, in ticks.
    pub fn timer(&self) -> u16 {
        self.timer
    }

    /// Controls are ignored while dead until the timer is past
    /// `input_lock_until`.
    pub fn input_locked(&self) -> bool {
        match (&self.rules, self.dead) {
            (Some(r), true) => self.timer <= r.input_lock_until,
            _ => false,
        }
    }

    /// The dead bit belongs in STATUS: dead and the timer at or past
    /// `status_bit_from`.
    pub fn status_dead(&self) -> bool {
        match (&self.rules, self.dead) {
            (Some(r), true) => self.timer >= r.status_bit_from,
            _ => false,
        }
    }

    /// Dead, the lock is over and a key press respawns.
    pub fn can_respawn(&self) -> bool {
        self.dead && !self.input_locked()
    }

    /// Note who hit the player. Valid until the next [`Life::tick`], so it
    /// belongs to the hit that is about to take the hearts to 0 (or not),
    /// and never to a later change of hearts.
    pub fn note_attacker(&mut self, attacker: Option<u16>) {
        self.attacker = attacker;
    }

    /// Advance by `ms` and check the hearts. `hearts_zero` is true when the
    /// hearts are 0 or below: that starts the death once, and holds it (the
    /// per-frame safety net) until the player respawns.
    pub fn tick(&mut self, ms: u64, hearts_zero: bool) {
        let Some(rules) = self.rules else {
            self.attacker = None;
            return;
        };
        let attacker = self.attacker.take();
        if !self.dead {
            if hearts_zero {
                self.dead = true;
                self.timer = 0;
                self.acc_ms = 0;
                self.status_sent = false;
                self.events.push(LifeEvent::Died { attacker });
            }
            return;
        }
        self.acc_ms += ms;
        while self.acc_ms >= TICK_MS {
            self.acc_ms -= TICK_MS;
            if self.timer < rules.timer_cap {
                self.timer += 1;
            }
        }
        if !self.status_sent && self.timer >= rules.status_bit_from {
            self.status_sent = true;
            self.events.push(LifeEvent::StatusDead);
        }
    }

    /// The player pressed a movement or action key. Respawns when allowed.
    pub fn respawn(&mut self) -> bool {
        if !self.can_respawn() {
            return false;
        }
        self.dead = false;
        self.timer = 0;
        self.acc_ms = 0;
        let clear_status = self.status_sent;
        self.status_sent = false;
        self.events.push(LifeEvent::Respawned { clear_status });
        true
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<LifeEvent> {
        std::mem::take(&mut self.events)
    }
}

/// The messages one event puts on the wire for `version`, in order.
/// `status_base` is the STATUS value the server last gave us (or 0), with
/// only the dead bit changed.
///
/// - `Died` with an attacker: `PLI_CLAIMPKER` (14), for a build whose
///   profile sends it ([`ClientVersion::sends_claim`]). With no attacker, or
///   on a build without it, nothing: no other message is known for a death.
/// - `StatusDead`: STATUS with the dead bit, 1.4 s after the death. So the
///   claim always goes out before the status.
/// - `Respawned`: STATUS with the bit cleared, when it had been sent. The
///   level re-send of an online respawn is the caller's (and goes first).
pub fn messages_for(event: LifeEvent, version: ClientVersion, status_base: u8) -> Vec<Vec<u8>> {
    if version.death_rules().is_none() {
        return Vec::new();
    }
    match event {
        LifeEvent::Died { attacker: Some(a) } if version.sends_claim() => vec![claim_message(a)],
        LifeEvent::Died { .. } => Vec::new(),
        LifeEvent::StatusDead => vec![status_message(with_dead_bit(status_base, true))],
        LifeEvent::Respawned { clear_status: true } => vec![status_message(with_dead_bit(status_base, false))],
        LifeEvent::Respawned { clear_status: false } => Vec::new(),
    }
}

/// The player's own pose as the hearts report names it in GANI (10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReportPose {
    #[default]
    Idle,
    Hurt,
    Dead,
}

impl ReportPose {
    pub fn gani(self) -> &'static str {
        match self {
            ReportPose::Idle => "idle",
            ReportPose::Hurt => "hurt",
            ReportPose::Dead => "dead",
        }
    }
}

/// One hearts report for the server: what changed since the last one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerReport {
    /// New CURPOWER in half hearts, when the hearts changed.
    pub cur_half: Option<u8>,
    /// New pose, when it changed.
    pub pose: Option<ReportPose>,
    /// SPRITE, sent with the start of hurt mode (the hurt sprite).
    pub sprite: Option<u8>,
}

impl PowerReport {
    /// The `PLI_PLAYERPROPS` message. `ganis` is false for a build without
    /// GANIs: the pose is then left out.
    pub fn message(&self, ganis: bool) -> Option<Vec<u8>> {
        let gani = self.pose.filter(|_| ganis).map(ReportPose::gani);
        if self.cur_half.is_none() && gani.is_none() && self.sprite.is_none() {
            return None;
        }
        Some(crate::net::power_props_message(self.cur_half, gani, self.sprite))
    }
}

/// What the server knows of the player's hearts and pose, so the client
/// can tell it what changed.
///
/// The real client reports its own hearts: GServer never decides them for
/// a hit, and does not echo the client's CURPOWER. Seen on the wire:
///
/// - **Hit** (2.05 lab run 1 and the 2.22 lab of 2026-10-25): one message
///   with CURPOWER, GANI `hurt` and SPRITE, in the same tick as the hit.
///   After it the client sends GANI `idle` when it stands again.
/// - **Death** (2.22): CURPOWER 0 and GANI `dead`. When the server set the
///   hearts to 0 itself, GANI `dead` without CURPOWER.
/// - **Respawn** (2.22, and 2.05 live): CURPOWER by the AP table and GANI
///   `idle`.
///
/// Lava burns and heart pickups are reported the same way, as a plain
/// CURPOWER; that is inferred, not captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerReporter {
    reported_half: i32,
    pose: ReportPose,
}

impl Default for PowerReporter {
    /// A new session: three hearts (the default account) and idle.
    fn default() -> Self {
        PowerReporter { reported_half: 6, pose: ReportPose::Idle }
    }
}

impl PowerReporter {
    /// The server gave the hearts (its PLAYERPROPS): it knows them, so they
    /// are not sent back.
    pub fn server_set(&mut self, half: i32) {
        self.reported_half = half;
    }

    /// The hearts the server last heard of.
    pub fn reported_half(&self) -> i32 {
        self.reported_half
    }

    /// Compare the player's state with what was reported and return what
    /// changed. `dead` counts a player at 0 hearts whose death has not yet
    /// started (the death starts on the next life tick), so a killing hit
    /// reports `dead`, not `hurt`. `sprite` is the SPRITE value to send with
    /// the start of hurt mode.
    pub fn report(&mut self, half: i32, dead: bool, hurting: bool, sprite: u8) -> Option<PowerReport> {
        let pose = if dead {
            ReportPose::Dead
        } else if hurting {
            ReportPose::Hurt
        } else {
            ReportPose::Idle
        };
        let half = half.clamp(0, 223);
        let cur_half = (half != self.reported_half).then_some(half as u8);
        // The respawn report is CURPOWER and GANI `idle` (2.05, live
        // 2026-10-29), even while hurt mode from the killing hit still runs.
        let pose = if self.pose == ReportPose::Dead && pose == ReportPose::Hurt { ReportPose::Idle } else { pose };
        let new_pose = (pose != self.pose).then_some(pose);
        self.pose = pose;
        if cur_half.is_none() && new_pose.is_none() {
            return None;
        }
        self.reported_half = half;
        let sprite = (new_pose == Some(ReportPose::Hurt)).then_some(sprite);
        Some(PowerReport { cur_half, pose: new_pose, sprite })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ClientVersion, DeathRules, DEATH_205, PROFILES};

    fn life() -> Life {
        Life::new(Some(DEATH_205))
    }

    fn run(l: &mut Life, ticks: u64) {
        for _ in 0..ticks {
            l.tick(TICK_MS, true);
        }
    }

    #[test]
    fn timer_counts_from_zero_and_stops_at_30() {
        let mut l = life();
        l.tick(0, true); // hearts reach 0: the sequence starts at timer 0
        assert!(l.is_dead());
        assert_eq!(l.timer(), 0);
        run(&mut l, 10);
        assert_eq!(l.timer(), 10);
        run(&mut l, 100);
        assert_eq!(l.timer(), 30, "1.5 s cap");
    }

    #[test]
    fn partial_ticks_add_up() {
        let mut l = life();
        l.tick(0, true);
        for _ in 0..10 {
            l.tick(25, true);
        }
        assert_eq!(l.timer(), 5);
    }

    #[test]
    fn input_is_locked_through_tick_27() {
        let mut l = life();
        assert!(!l.input_locked(), "alive: not locked");
        l.tick(0, true);
        assert!(l.input_locked());
        run(&mut l, 27);
        assert_eq!(l.timer(), 27);
        assert!(l.input_locked(), "timer 27: still ignored");
        assert!(!l.respawn());
        run(&mut l, 1);
        assert_eq!(l.timer(), 28);
        assert!(!l.input_locked(), "past 27: controls work");
        assert!(l.can_respawn());
    }

    #[test]
    fn dead_bit_from_tick_28() {
        let mut l = life();
        l.tick(0, true);
        run(&mut l, 27);
        assert!(!l.status_dead());
        assert_eq!(l.take_events(), vec![LifeEvent::Died { attacker: None }]);
        run(&mut l, 1);
        assert!(l.status_dead());
        assert_eq!(l.take_events(), vec![LifeEvent::StatusDead]);
        run(&mut l, 5);
        assert!(l.take_events().is_empty(), "the status goes out once");
    }

    #[test]
    fn death_starts_once() {
        let mut l = life();
        l.tick(0, true);
        run(&mut l, 12);
        l.tick(0, true);
        assert_eq!(l.timer(), 12);
        assert_eq!(l.take_events().iter().filter(|e| matches!(e, LifeEvent::Died { .. })).count(), 1);
    }

    #[test]
    fn hearts_back_above_zero_do_not_revive() {
        let mut l = life();
        l.tick(0, true);
        run(&mut l, 40);
        l.tick(50, false);
        assert!(l.is_dead(), "only a key press respawns");
    }

    #[test]
    fn respawn_clears_state_and_the_status_bit() {
        let mut l = life();
        l.tick(0, true);
        run(&mut l, 30);
        l.take_events();
        assert!(l.respawn());
        assert!(!l.is_dead() && !l.status_dead() && !l.input_locked());
        assert_eq!(l.take_events(), vec![LifeEvent::Respawned { clear_status: true }]);
        assert!(!l.respawn(), "nothing to respawn from");
        // Dying again starts a fresh timer.
        l.tick(0, true);
        assert_eq!((l.is_dead(), l.timer()), (true, 0));
    }

    #[test]
    fn attacker_belongs_to_the_next_tick_only() {
        let mut l = life();
        l.note_attacker(Some(12));
        l.tick(0, false); // survived the hit
        l.tick(0, true); // later death by something else
        assert_eq!(l.take_events(), vec![LifeEvent::Died { attacker: None }]);
        let mut l = life();
        l.note_attacker(Some(12));
        l.tick(0, true);
        assert_eq!(l.take_events(), vec![LifeEvent::Died { attacker: Some(12) }]);
    }

    #[test]
    fn no_rules_no_death() {
        let mut l = Life::new(None);
        l.note_attacker(Some(3));
        l.tick(1000, true);
        assert!(!l.is_dead() && !l.input_locked() && l.take_events().is_empty());
    }

    #[test]
    fn respawn_hearts_table_by_ap() {
        let r = DEATH_205;
        // (ap, max hearts in half hearts) -> half hearts
        for (ap, max, want) in [
            (0, 20, 6),
            (19, 20, 6),
            (19, 4, 6),
            (20, 20, 10),
            (39, 20, 10),
            (39, 8, 8),
            (40, 20, 20),
            (100, 6, 6),
            (50, 40, 40),
        ] {
            assert_eq!(respawn_half_hearts(&r, ap, max), want, "ap {ap} max {max}");
        }
    }

    #[test]
    fn claim_payload() {
        // 14 + 0x20, GSHORT 7, newline.
        assert_eq!(claim_message(7), vec![0x2E, 0x20, 0x27, b'\n']);
        // 300 = 2*128 + 44.
        assert_eq!(claim_message(300), vec![0x2E, 0x22, 0x4C, b'\n']);
        assert_eq!(claim_message(0x3FFF), vec![0x2E, 0x9F, 0x9F, b'\n']);
    }

    #[test]
    fn status_payload() {
        assert_eq!(status_message(0x08), vec![0x22, 0x32, 0x28, b'\n']);
        assert_eq!(with_dead_bit(0x14, true), 0x1C);
        assert_eq!(with_dead_bit(0x1C, false), 0x14);
    }

    /// Run a death to the respawn and collect every message for `v`.
    fn session_messages(v: ClientVersion, attacker: Option<u16>) -> Vec<Vec<u8>> {
        let mut l = Life::new(v.death_rules());
        let mut out = Vec::new();
        let mut feed = |l: &mut Life, out: &mut Vec<Vec<u8>>| {
            for e in l.take_events() {
                out.extend(messages_for(e, v, 0x10));
            }
        };
        l.note_attacker(attacker);
        l.tick(0, true);
        feed(&mut l, &mut out);
        run(&mut l, 40);
        feed(&mut l, &mut out);
        l.respawn();
        feed(&mut l, &mut out);
        out
    }

    #[test]
    fn claim_then_status_then_clear_on_every_profile_with_rules() {
        let mut with_rules = 0;
        for p in PROFILES {
            let msgs = session_messages(p.version, Some(5));
            if p.life.death.is_none() {
                assert!(msgs.is_empty(), "{}: no death sequence, no messages", p.label);
                continue;
            }
            with_rules += 1;
            assert!(p.protocol.pli.contains(PLI_CLAIMPKER), "{}", p.label);
            assert_eq!(
                msgs,
                vec![claim_message(5), status_message(0x18), status_message(0x10)],
                "{}: claim at death, dead bit at 1.4 s, cleared at respawn",
                p.label
            );
        }
        assert!(with_rules >= 4, "2.05, 2.17, 2.22 and 2.31 at least");
    }

    #[test]
    fn no_attacker_no_claim() {
        let msgs = session_messages(ClientVersion::V222, None);
        assert_eq!(msgs, vec![status_message(0x18), status_message(0x10)]);
    }

    #[test]
    fn profile_gates() {
        assert_eq!(ClientVersion::V205.death_rules(), Some(DEATH_205));
        for v in [ClientVersion::V200, ClientVersion::V201, ClientVersion::V204] {
            // 2.00 to 2.04 run the 2.05 sequence (read in 2.00, constants checked in all).
            assert_eq!(v.death_rules(), Some(DEATH_205), "{}", v.label());
        }
        for v in [ClientVersion::V205, ClientVersion::V217, ClientVersion::V222, ClientVersion::V231] {
            assert!(v.sends_claim(), "{}", v.label());
            // Only the save-entry name-group test differs between them.
            let d = v.death_rules().unwrap();
            assert_eq!(DeathRules { entry_name_group: true, ..d }, DEATH_205, "{}", v.label());
        }
        for v in [ClientVersion::V133, ClientVersion::V139] {
            assert!(!v.sends_claim(), "{}", v.label());
            assert!(v.death_rules().is_none(), "{}", v.label());
        }
    }

    #[test]
    fn constants_match_the_decompile_table() {
        let r = DEATH_205;
        assert_eq!((r.timer_cap, r.input_lock_until, r.status_bit_from), (30, 27, 28));
        assert_eq!(u64::from(r.timer_cap) * TICK_MS, 1500);
        assert_eq!((f32::from(r.default_x2) / 2.0, f32::from(r.default_y2) / 2.0), (30.5, 30.0));
    }

    #[test]
    fn a_hit_reports_curpower_hurt_and_sprite_as_the_205_capture() {
        // 2.05 lab run 1, t = 130.962 s: 6 -> 5 half hearts, facing up.
        let mut p = PowerReporter::default();
        let r = p.report(5, false, true, 128).expect("a report");
        assert_eq!(r, PowerReport { cur_half: Some(5), pose: Some(ReportPose::Hurt), sprite: Some(128) });
        let mut want = vec![0x22];
        want.extend_from_slice(b"\x22\x25\x2a\x24hurt\x31\xa0\n");
        assert_eq!(r.message(true).unwrap(), want);
        assert_eq!(p.report(5, false, true, 128), None, "reported once");
    }

    #[test]
    fn the_end_of_hurt_mode_sends_idle_alone() {
        let mut p = PowerReporter::default();
        p.report(5, false, true, 128);
        let r = p.report(5, false, false, 0).unwrap();
        // As the client's `*$idle` after the hit.
        assert_eq!(r.message(true).unwrap(), b"\x22\x2a\x24idle\n".to_vec());
    }

    #[test]
    fn a_killing_hit_reports_zero_and_dead() {
        let mut p = PowerReporter::default();
        let r = p.report(0, true, true, 130).unwrap();
        assert_eq!(r.message(true).unwrap(), b"\x22\x22\x20\x2a\x24dead\n".to_vec());
        // Respawn: the hearts by AP and idle (2.05 live).
        let r = p.report(6, false, false, 2).unwrap();
        assert_eq!(r.message(true).unwrap(), b"\x22\x22\x26\x2a\x24idle\n".to_vec());
        // Respawned while the killing hit's hurt mode still runs: still idle,
        // and nothing more when it ends.
        let mut p = PowerReporter::default();
        p.report(0, true, true, 130);
        assert_eq!(p.report(6, false, true, 130).unwrap().message(true).unwrap(), b"\x22\x22\x26\x2a\x24idle\n".to_vec());
        assert_eq!(p.report(6, false, false, 2), None);
    }

    #[test]
    fn server_hearts_are_not_sent_back() {
        let mut p = PowerReporter::default();
        p.server_set(10);
        assert_eq!(p.report(10, false, false, 2), None);
        // The server set 0: only the dead gani goes out.
        p.server_set(0);
        let r = p.report(0, true, false, 2).unwrap();
        assert_eq!(r.message(true).unwrap(), b"\x22\x2a\x24dead\n".to_vec());
    }

    #[test]
    fn a_burn_or_a_heart_is_a_plain_curpower() {
        let mut p = PowerReporter::default();
        assert_eq!(p.report(5, false, false, 2).unwrap().message(true).unwrap(), b"\x22\x22\x25\n".to_vec());
        assert_eq!(p.report(6, false, false, 2).unwrap().message(true).unwrap(), b"\x22\x22\x26\n".to_vec());
    }

    #[test]
    fn a_build_without_ganis_leaves_the_pose_out() {
        let mut p = PowerReporter::default();
        let r = p.report(5, false, true, 128).unwrap();
        assert_eq!(r.message(false).unwrap(), b"\x22\x22\x25\x31\xa0\n".to_vec());
        // Only a pose changed: nothing to send.
        assert_eq!(p.report(5, false, false, 0).unwrap().message(false), None);
    }

}
