//! Pose animations of the player: the sword swing, the hurt pose, sitting
//! (item 22), lying in bed, and the idle, walking and swimming figure.
//!
//! The poses here are *driven by state that already exists* rather than by a
//! clock of their own, so what is drawn can never disagree with what the
//! other players are sent:
//!
//! Every step of a GANI is held for `1 + WAIT` ticks of 50 ms, as the real
//! 2.05 client holds it (Graal_analysis `player-mannerisms-live.md`, read
//! live: push 4 ticks a frame, pull 2, hurt 31, lift 6, swim 2). A file
//! without `WAIT` lines steps once a tick.
//!
//! * **Walk** shows the frame `offline` has stepped to: one GANI frame per
//!   step taken (every 50 ms), the same stepping that plays the footstep
//!   cues. **Swim** runs on the tick clock instead ([`OfflineSession::tick_poses`]):
//!   the real client keeps the stroke going while the player floats still.
//!   **Idle** shows `idle.gani`'s first step when the player is not walking.
//!   The sprite value the other players are sent is not touched.
//! * **Sword** follows the swing of [`crate::combat::CombatState`]: swing
//!   step `i` (the sprite value S = 6 + `i` that goes on the wire) shows step
//!   `i` of `sword.gani`. A GANI `PLAYSOUND` cue after a step is queued as a
//!   sound event when that step is entered, so `sword.wav` plays at the start
//!   of the swing. When the swing ends the pose is gone and the player is
//!   drawn as idle again, which is what the file's `SETBACKTO idle` says.
//! * **Spin** plays `spin.gani` once by its waits (the stock file: 13
//!   ticks) when S is pressed again during a swing with the has-spin
//!   status (STATUS 0x40), then idle.
//! * **Hurt** follows hurt mode. The 2.05 client sets the `hurt` gani when
//!   hurt mode starts (Graal_analysis 01-death-respawn). A `LOOP` file
//!   repeats for the whole hurt mode; any other plays once by its waits (the
//!   stock file: 31 ticks) and gives way to idle. The sprite others see
//!   (S = 32) lasts the whole of hurt mode and is not changed here.
//! * **Dead** shows `dead.gani` while the player is dead, one step per tick
//!   of the death timer by its waits, held on its last step. The 2.05 client plays the
//!   local `dead` gani on death. Seihai's default set has its own: the hurt
//!   pose, kneeling, then lying on the back.
//! * **Push**, **grab** and **pull** follow [`crate::offline::PushPull`]:
//!   the push pose on the 18th blocked step in a row (`push.gani` from its
//!   first step there, restarting each time the count wraps from 38 to 18),
//!   the grab pose while A holds a wall, and the pull pose (`pull.gani`,
//!   looping) from the arrow away from the wall on. Once started the pull
//!   pose stays, letting go of the arrow and of A, until the player moves:
//!   the 2.05 client sets nothing else then. The 2.05 client plays its
//!   `push`, `grab` and `pull` ganis then. Before the push pose (and before a ledge jump) the walk
//!   pose keeps stepping while the arrow is held: 2.05 sets the walk gani
//!   on every walk tick whether the step moved or not, so the player runs in
//!   place against the wall. A ledge jump keeps the walk pose: 2.05 plays no
//!   other gani in the air.
//! * **Lift** shows `lift.gani` once when an object is picked up (the stock
//!   file: 6 ticks), then the carry poses.
//! * **Hidden** under a carried object (D): `hiddenstill` / `hidden` on
//!   builds that play them (2.05 on), else the carry poses go on (see
//!   `offline/hide.rs`).
//! * **Sit** and **sleep** are shown while [`OfflineSession::sitting`] or
//!   [`OfflineSession::sleeping`] is set, walking or not: the 2.05 client
//!   sets the `sit` or `sleep` gani after the walk has set `walk`, in the
//!   same tick. The session sets them with the 2.05 client's probes: the
//!   body's middle on a chair (tile type 3), or the head on a bed top
//!   (type 4); see [`OfflineSession::refresh_rest`] and
//!   `documentation/sitting.md`.
//!
//! A pose is shown only when its GANI is loaded (`OfflineSession::poses`),
//! so a build without GANIs, or a data folder without `sword.gani`, draws the
//! plain idle figure as before.

use crate::assets::{Gani, GaniSound};
use crate::offline::OfflineSession;
use crate::sound::SoundEvent;

/// One GANI tick: a step is shown for `1 + WAIT` of these.
pub const GANI_STEP_MS: u64 = 50;

/// Ticks after a hit before a held arrow walks again (and the walk
/// replaces the hurt pose): 12 in 2.05 rev 2 and 2.22, read live
/// (Graal_analysis `player-mannerisms-live.md`, "Walking again with an
/// arrow held"). The pose lasts its full 31 ticks only with no arrow held.
pub const HURT_WALK_AFTER_TICKS: u64 = 12;

/// The 1.39 hurt pose, built into the client: about 10 ticks (video, ±1).
/// Used for the blink of a build that blinks after the hurt pose when no
/// `hurt.gani` is loaded.
pub const HURT_POSE_139_TICKS: usize = 10;

/// The hurt pose when no `hurt.gani` is loaded: 31 ticks, the stock file's
/// one step of `WAIT 30`. The real 2.00 sends GANI `idle` 1.35 to 1.55 s
/// after the hit, while its hurt sprite stays on to the end of hurt mode
/// (lab, 2026-10-30 run 2 and lan-200i).
pub const HURT_POSE_TICKS: usize = 31;

/// The loaded pose animations.
#[derive(Debug, Clone, Default)]
pub struct PoseGanis {
    pub sword: Option<Gani>,
    pub hurt: Option<Gani>,
    pub sit: Option<Gani>,
    /// Lying in bed.
    pub sleep: Option<Gani>,
    pub idle: Option<Gani>,
    /// Standing and walking with an object held over the head.
    pub carrystill: Option<Gani>,
    pub carry: Option<Gani>,
    /// Shown while the player is dead.
    pub dead: Option<Gani>,
    /// Running against a wall, grabbing one, and pulling.
    pub push: Option<Gani>,
    pub grab: Option<Gani>,
    pub pull: Option<Gani>,
    /// Picking an object up, before `carrystill`.
    pub lift: Option<Gani>,
    /// The spin attack.
    pub spin: Option<Gani>,
    /// Hiding under the carried object, standing and walking.
    pub hiddenstill: Option<Gani>,
    pub hidden: Option<Gani>,
}

impl PoseGanis {
    /// Every loaded pose GANI.
    pub fn all(&self) -> impl Iterator<Item = &Gani> {
        [&self.sword, &self.hurt, &self.sit, &self.sleep, &self.idle, &self.carrystill, &self.carry, &self.dead, &self.push, &self.grab, &self.pull, &self.lift, &self.hiddenstill, &self.hidden, &self.spin]
            .into_iter()
            .flatten()
    }
}

/// Which pose is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoseKind {
    Sword,
    Hurt,
    Sit,
    Sleep,
    Idle,
    Walk,
    Swim,
    CarryStill,
    Carry,
    Dead,
    Push,
    Grab,
    Pull,
    Lift,
    Spin,
    HiddenStill,
    Hidden,
}

/// A pose step ready to draw.
#[derive(Debug, Clone, Copy)]
pub struct PoseFrame<'a> {
    pub kind: PoseKind,
    pub gani: &'a Gani,
    /// Step index inside `gani` (already limited to its length).
    pub frame: usize,
}

impl PoseKind {
    /// The GANI file of the pose.
    pub fn file(self) -> &'static str {
        match self {
            PoseKind::Sword => "sword.gani",
            PoseKind::Hurt => "hurt.gani",
            PoseKind::Sit => "sit.gani",
            PoseKind::Sleep => "sleep.gani",
            PoseKind::Idle => "idle.gani",
            PoseKind::Walk => "walk.gani",
            PoseKind::Swim => "swim.gani",
            PoseKind::CarryStill => "carrystill.gani",
            PoseKind::Carry => "carry.gani",
            PoseKind::Dead => "dead.gani",
            PoseKind::Push => "push.gani",
            PoseKind::Grab => "grab.gani",
            PoseKind::Pull => "pull.gani",
            PoseKind::Lift => "lift.gani",
            PoseKind::Spin => "spin.gani",
            PoseKind::HiddenStill => "hiddenstill.gani",
            PoseKind::Hidden => "hidden.gani",
        }
    }

    /// The pose a file name belongs to (case-insensitive). `walk.gani` and
    /// `swim.gani` are not here: they load as the stepped motion ganis.
    pub fn from_file(name: &str) -> Option<PoseKind> {
        [
            PoseKind::Sword,
            PoseKind::Hurt,
            PoseKind::Sit,
            PoseKind::Sleep,
            PoseKind::Idle,
            PoseKind::CarryStill,
            PoseKind::Carry,
            PoseKind::Dead,
            PoseKind::Push,
            PoseKind::Grab,
            PoseKind::Pull,
            PoseKind::Lift,
            PoseKind::Spin,
            PoseKind::HiddenStill,
            PoseKind::Hidden,
        ]
            .into_iter()
            .find(|k| k.file().eq_ignore_ascii_case(name))
    }
}

fn pick(kind: PoseKind, gani: &Option<Gani>, i: usize) -> Option<PoseFrame<'_>> {
    gani.as_ref().filter(|g| g.frame_count() > 0).map(|g| PoseFrame { kind, gani: g, frame: i.min(g.frame_count() - 1) })
}

/// The step `tick` ticks into a pose, each step held for its `WAIT`:
/// wrapping when `wrap`, else held on the last step.
fn at_tick(kind: PoseKind, gani: &Option<Gani>, tick: usize, wrap: bool) -> Option<PoseFrame<'_>> {
    gani.as_ref().filter(|g| g.frame_count() > 0).map(|g| PoseFrame { kind, gani: g, frame: g.frame_at_tick(tick, wrap) })
}

/// A one-pass pose `tick` ticks in: `None` once its waits have run out.
fn once(kind: PoseKind, gani: &Option<Gani>, tick: usize) -> Option<PoseFrame<'_>> {
    gani.as_ref().filter(|g| tick < g.total_ticks()).and_then(|_| at_tick(kind, gani, tick, false))
}

/// The step of `hurt.gani` `ms` into hurt mode. A `LOOP` GANI repeats for
/// the whole hurt mode; any other is shown once, by its waits, and then
/// gives way, as its `SETBACKTO` says.
fn hurt_step(hurt: &Option<Gani>, ms: u64) -> Option<PoseFrame<'_>> {
    let tick = (ms / GANI_STEP_MS) as usize;
    match hurt {
        Some(g) if g.looped => at_tick(PoseKind::Hurt, hurt, tick, true),
        _ => once(PoseKind::Hurt, hurt, tick),
    }
}

fn pick_player<'a>(kind: PoseKind, p: &'a crate::assets::GaniPlayer) -> Option<PoseFrame<'a>> {
    (p.gani.frame_count() > 0).then(|| PoseFrame { kind, gani: &p.gani, frame: p.frame.min(p.gani.frame_count() - 1) })
}

impl OfflineSession {
    /// Attach a pose animation.
    pub fn set_pose_gani(&mut self, kind: PoseKind, gani: Gani) {
        match kind {
            PoseKind::Sword => self.poses.sword = Some(gani),
            PoseKind::Hurt => self.poses.hurt = Some(gani),
            PoseKind::Sit => self.poses.sit = Some(gani),
            PoseKind::Sleep => self.poses.sleep = Some(gani),
            PoseKind::Idle => self.poses.idle = Some(gani),
            PoseKind::CarryStill => self.poses.carrystill = Some(gani),
            PoseKind::Carry => self.poses.carry = Some(gani),
            PoseKind::Dead => self.poses.dead = Some(gani),
            PoseKind::Push => self.poses.push = Some(gani),
            PoseKind::Grab => self.poses.grab = Some(gani),
            PoseKind::Pull => self.poses.pull = Some(gani),
            PoseKind::Lift => self.poses.lift = Some(gani),
            PoseKind::Spin => self.poses.spin = Some(gani),
            PoseKind::HiddenStill => self.poses.hiddenstill = Some(gani),
            PoseKind::Hidden => self.poses.hidden = Some(gani),
            // Loaded with `set_walk_gani` / `set_swim_gani`.
            PoseKind::Walk | PoseKind::Swim => {}
        }
    }

    /// Take the mannerisms of a build.
    pub fn set_mannerisms(&mut self, m: crate::protocol::Mannerisms) {
        self.mannerisms = m;
        self.combat.set_knockback_ticks(m.knockback_ticks);
    }

    /// The hurt pose is still on: hurt mode, no step taken since the hit,
    /// and `hurt.gani` not yet run out (a `LOOP` file lasts the whole of
    /// hurt mode). This is when the hearts report says GANI `hurt`; it
    /// gives way to `idle` before hurt mode itself ends.
    pub fn hurt_pose_on(&self) -> bool {
        let Some(ms) = self.combat.hurt_elapsed_ms() else { return false };
        let ticks = match &self.poses.hurt {
            Some(g) if g.looped => return !self.hurt_walked,
            Some(g) => g.total_ticks(),
            None => HURT_POSE_TICKS,
        };
        !self.hurt_walked && ((ms / GANI_STEP_MS) as usize) < ticks
    }

    /// The hurt blink leaves the figure out this tick: only its shadow is
    /// drawn. Every other tick of hurt mode, from the hit (1.41 on) or once
    /// the hurt pose is over (1.39).
    pub fn blink_off(&self) -> bool {
        let Some(ms) = self.combat.hurt_elapsed_ms() else { return false };
        let tick = (ms / GANI_STEP_MS) as usize;
        let from = match self.mannerisms.blink {
            crate::protocol::Blink::FromHit => 0,
            crate::protocol::Blink::AfterHurtPose => self.poses.hurt.as_ref().map_or(HURT_POSE_139_TICKS, |g| g.total_ticks()),
        };
        tick >= from && (tick - from) % 2 == 1
    }

    /// Advance the poses that run on the 50 ms tick, not on steps taken:
    /// the swim stroke (moving or not) and the lift. Each swim step entered
    /// queues its `PLAYSOUND` cues. True if the drawn pose may have changed.
    pub fn tick_poses(&mut self, ms: u64) -> bool {
        if !self.combat.is_hurt() {
            self.hurt_walked = false;
        }
        // Empty hands: no hiding place.
        if self.carried.is_none() {
            self.hidden = false;
        }
        let swimming = self.in_water && self.swim_gani.is_some();
        if !swimming && self.lift_ticks.is_none() {
            self.pose_clock_ms = 0;
            return false;
        }
        self.pose_clock_ms += ms;
        let mut changed = false;
        while self.pose_clock_ms >= GANI_STEP_MS {
            self.pose_clock_ms -= GANI_STEP_MS;
            changed = true;
            if let Some(t) = self.lift_ticks.as_mut() {
                *t += 1;
                // Kept until the object counts as carried, with or without
                // a `lift.gani`.
                let done = self.poses.lift.as_ref().map_or(true, |g| *t as usize >= g.total_ticks())
                    && *t >= crate::offline::LIFT_CARRIED_TICK;
                if done || self.carried.is_none() {
                    self.lift_ticks = None;
                }
            }
            if swimming {
                use crate::protocol::SwimStill;
                let moving = self.walking;
                let stopped = std::mem::replace(&mut self.swim_was_moving, moving) && !moving;
                let Some(p) = self.swim_gani.as_mut() else { continue };
                match self.mannerisms.swim_still {
                    // NewMain 1.41: back to the first step and held.
                    SwimStill::Freezes if !moving => {
                        self.swim_ticks = 0;
                        p.frame = 0;
                        continue;
                    }
                    // 2.22: back to the first step at the stop, then on.
                    SwimStill::Restarts if stopped => {
                        self.swim_ticks = 0;
                        p.frame = 0;
                        continue;
                    }
                    _ => {}
                }
                self.swim_ticks += 1;
                let total = p.gani.total_ticks().max(1);
                let t = self.swim_ticks % total;
                p.frame = p.gani.frame_at_tick(t, true);
                // A step is entered when `t` is where it starts.
                let start: usize = (0..p.frame).map(|i| p.gani.frame_ticks(i)).sum();
                if t == start {
                    let cues = p.current().map(|f| f.sounds.clone()).unwrap_or_default();
                    for c in cues {
                        self.sound_events.push(SoundEvent { file: c.file, x: self.player_x + c.x, y: self.player_y + c.y });
                    }
                }
            }
        }
        changed
    }

    /// The player stopped walking (no arrow key is held): back to idle.
    /// True if the pose changed.
    pub fn stop_walking(&mut self) -> bool {
        std::mem::take(&mut self.walking)
    }

    /// The pose to draw now, if any: only for a pose whose GANI is loaded.
    /// Dead: none. A swing wins over hurt, hurt over sitting or lying in
    /// bed, those over the motion poses: swimming (the `swim.gani` frame),
    /// walking (the `walk.gani` frame) or idle (`idle.gani`, first step).
    /// Swimming has no sword, hurt, sit or sleep pose.
    pub fn current_pose(&self) -> Option<PoseFrame<'_>> {
        // Dead: `dead.gani`, one step per 50 ms tick of the death timer,
        // held on its last step.
        if self.life.is_dead() {
            return at_tick(PoseKind::Dead, &self.poses.dead, usize::from(self.life.timer()), false);
        }
        if self.in_water {
            return self.swim_gani.as_ref().and_then(|p| pick_player(PoseKind::Swim, p));
        }
        // The spin attack: `spin.gani` by its waits, once, then idle.
        if let Some(p) = self.combat.spin_elapsed_ms().and_then(|ms| once(PoseKind::Spin, &self.poses.spin, (ms / GANI_STEP_MS) as usize)) {
            return Some(p);
        }
        if let Some(i) = self.combat.swing_frame() {
            if let Some(p) = pick(PoseKind::Sword, &self.poses.sword, i) {
                return Some(p);
            }
        }
        if let Some(p) = self.combat.hurt_elapsed_ms().filter(|_| !self.hurt_walked).and_then(|ms| hurt_step(&self.poses.hurt, ms)) {
            return Some(p);
        }
        // Holding a wall with A: the grab pose, or the pull once the arrow
        // away from it was pressed. The pull goes on after the keys are let
        // go, until the player moves. Without those ganis the player stands.
        if self.pushpull.pull_pose() {
            return at_tick(PoseKind::Pull, &self.poses.pull, self.pushpull.pull_ticks as usize, true)
                .or_else(|| pick(PoseKind::Idle, &self.poses.idle, 0));
        }
        if self.pushpull.grab.is_some() || self.pushpull.grab_shown {
            return pick(PoseKind::Grab, &self.poses.grab, 0).or_else(|| pick(PoseKind::Idle, &self.poses.idle, 0));
        }
        // Holding an object: the carry poses (the 2.05 client plays its
        // carrystill and carry ganis then). The carry walk shows the step the
        // walk has reached.
        if self.carried.is_some() {
            // Hidden (2.05 on): `hiddenstill` / `hidden`, which draw only a
            // shadow; the object is drawn where the player stands. Without
            // those files nothing of the figure is drawn. Builds that keep
            // the carry poses (1.39, NewMain 1.41) fall through.
            if self.figure_hidden() {
                let (kind, g) = if self.walking { (PoseKind::Hidden, &self.poses.hidden) } else { (PoseKind::HiddenStill, &self.poses.hiddenstill) };
                return pick(kind, g, 0);
            }
            if let Some(p) = self.lift_ticks.and_then(|t| once(PoseKind::Lift, &self.poses.lift, t as usize)) {
                return Some(p);
            }
            if self.walking {
                let step = self.walk_gani.as_ref().map_or(0, |p| p.frame);
                if let Some(p) = pick(PoseKind::Carry, &self.poses.carry, step) {
                    return Some(p);
                }
            }
            if let Some(p) = pick(PoseKind::CarryStill, &self.poses.carrystill, 0) {
                return Some(p);
            }
        }
        // Over the walk too: 2.05 sets these after the walk pose.
        if self.sitting {
            if let Some(p) = pick(PoseKind::Sit, &self.poses.sit, 0) {
                return Some(p);
            }
        }
        if self.sleeping {
            if let Some(p) = pick(PoseKind::Sleep, &self.poses.sleep, 0) {
                return Some(p);
            }
        }
        if let Some(p) = self.push_tick().and_then(|t| at_tick(PoseKind::Push, &self.poses.push, t, true)) {
            return Some(p);
        }
        if self.walking {
            if let Some(p) = self.walk_gani.as_ref().and_then(|p| pick_player(PoseKind::Walk, p)) {
                return Some(p);
            }
        }
        pick(PoseKind::Idle, &self.poses.idle, 0)
    }

    /// Queue the sound cues of a step of `sword.gani`, once, at its offset
    /// from the player.
    pub(crate) fn queue_sword_cues(&mut self, step: usize) {
        let cues: Vec<GaniSound> = match self.poses.sword.as_ref().and_then(|g| g.frames.get(step)) {
            Some(f) => f.sounds.clone(),
            None => return,
        };
        for c in cues {
            self.sound_events.push(SoundEvent { file: c.file, x: self.player_x + c.x, y: self.player_y + c.y });
        }
    }
}
