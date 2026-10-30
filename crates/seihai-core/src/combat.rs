//! Combat, the way the 2.05 client runs it: the victim's client judges.
//!
//! Sources: `decomp-findings-2026-10-24/04-sword-hitbox.md` (who decides a
//! sword hit and the reach rule), `01-death-respawn.md` (hurt state, claim),
//! `constants-205.tsv`, and Graal_analysis `protocol/packet-layouts.md`
//! "Hits" and `02-weapons-combat.md` (the ids and layouts of 26/40 and
//! 36/46). The numbers come from the version profile's [`CombatRules`];
//! nothing here names a version. A build without a row judges no hits and
//! sends none. `documentation/combat.md` lists what is not known.
//!
//! How a sword hit happens: the attacker does not send "I hit you". Its
//! client broadcasts its position, direction and sprite as it swings (the
//! sprite is `dir + 4 * S` with S 6 to 10 while the sword is out), and every
//! other client tests whether *it* is in the box in front of that swing.
//! [`judge_sword`] is that test, run on the receiving side. A 2.1+ client
//! shows the swing in its GANI prop (`sword`) instead of in the sprite, and
//! [`is_swing_gani`] reads it from there. Packet hits
//! (`PLO_HURTPLAYER` 40, `PLO_HITOBJECTS` 46) carry their own power and are
//! judged by [`judge_hurtplayer`].

use crate::offline::OfflineSession;
use crate::protocol::{ClientVersion, CombatRules, OtherPlayer, PowerStore};

/// `PLI_HURTPLAYER` (26), client to server:
/// `GSHORT victim, GCHAR (dx+4)*16, GCHAR (dy+4)*16, GCHAR power` (half
/// hearts), with an optional `GINT3` NPC id.
pub const PLI_HURTPLAYER: u8 = 26;
/// `PLO_HURTPLAYER` (40): the same with the attacker's id in front.
pub const PLO_HURTPLAYER: u8 = 40;
/// `PLO_HITOBJECTS` (46): `GSHORT attacker, GCHAR power*2, GCHAR x*2,
/// GCHAR y*2` and an optional `GINT3` NPC id.
pub const PLO_HITOBJECTS: u8 = 46;
/// The dead bit of STATUS (prop 18): other players can see it.
pub const STATUS_DEAD: u8 = 0x08;

/// Knockback (read in 2.05 rev 2, 2.17.1, 2.22 and 2.31; the same in all
/// four): a hit stores a direction (the packet's `dx, dy`; from the
/// attacker to the victim for a sword; a zero vector means east) scaled to
/// length 1. In each of the first `KNOCKBACK_TICKS` ticks of hurt mode the
/// victim moves one tile along it, and then every probe point of the body
/// box must be free, or that step is taken back.
pub const KNOCKBACK_TICKS: u16 = 5;
/// Tiles moved per knockback tick.
pub const KNOCKBACK_STEP_TILES: f32 = 1.0;
/// The twelve points tested after a knockback step, relative to the X/Y
/// props: the top, left, bottom and right edges of a box 0.6 to 2.4 tiles
/// across and 1.1 to 2.9 tiles down (three points per edge, the corners
/// twice).
pub const KNOCKBACK_PROBES: [(f32, f32); 12] = [
    (0.6, 1.1),
    (1.5, 1.1),
    (2.4, 1.1),
    (0.6, 1.1),
    (0.6, 2.0),
    (0.6, 2.9),
    (0.6, 2.9),
    (1.5, 2.9),
    (2.4, 2.9),
    (2.4, 1.1),
    (2.4, 2.0),
    (2.4, 2.9),
];
/// Where a player's centre lies relative to its X/Y props (tiles): the
/// point `PLO_HITOBJECTS` and the sword box are measured from.
pub const HIT_CENTRE: (f32, f32) = (1.5, 2.0);
/// `PLO_HITOBJECTS` hurts a player whose centre is strictly less than this
/// many tiles from the hit point on each axis.
pub const HITOBJECTS_BOX_TILES: f32 = 2.0;

/// The knockback direction for a hit vector: length 1, and east when the
/// vector is exactly zero.
pub fn knockback_unit(dx: f32, dy: f32) -> (f32, f32) {
    let (dx, dy) = if dx == 0.0 && dy == 0.0 { (1.0, 0.0) } else { (dx, dy) };
    let len = (dx * dx + dy * dy).sqrt();
    if len > 0.0 {
        (dx / len, dy / len)
    } else {
        (dx, dy)
    }
}

/// What a packet hit needs besides the packet: the same attacker-side gates
/// as a sword swing (`documentation/damage-respawn.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PacketGates {
    /// The attacker's "(guild)" tag is in the victim's nickname.
    pub friendly: bool,
    /// The attacker was already judged in this second.
    pub repeat: bool,
}

/// Where a player stands and which way it faces. Tiles; `dir` is 0 up,
/// 1 left, 2 down, 3 right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub dir: u8,
}

/// Why a hit does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// The build has no combat rules, or the packet is not in its id set.
    NotInBuild,
    /// The attacker's sprite is not a sword-swing frame.
    NotSwinging,
    /// The victim is not 0 to 4 tiles ahead of the attacker.
    OutOfReach,
    /// The victim is 2 or more tiles to the side.
    OffLine,
    /// The victim's AP is exactly the immune value (100).
    Immune,
    /// The victim is dead (or its hearts are 0).
    Dead,
    /// The victim is in hurt mode (mode 3, 40 ticks).
    AlreadyHurt,
    /// The victim is swimming (mode 10).
    Swimming,
    /// The victim hides under a carried object (mode 14).
    Hidden,
    /// The victim is paused (STATUS 0x01): the P key, or its window lost
    /// the focus. Real clients let a paused player be hurt only in a
    /// `sparringzone` level, which Seihai does not have.
    Paused,
    /// Packet hit from farther than the profile's range.
    TooFar,
    /// The hit came from an NPC. Its position is not known to Seihai.
    NpcSource,
    /// The attacker's sword power was never sent.
    UnknownPower,
    /// The hit takes nothing (power 0).
    NoDamage,
    /// The level said `noplayerkilling`: no player hits here.
    NoPkZone,
    /// The attacker's "(guild)" tag is in the victim's nickname.
    Friendly,
    /// The attacker's STATUS shows it hidden (or the untraced 0x20 bit).
    AttackerBlocked,
    /// This attacker was already judged in this second.
    Throttled,
    /// `PLO_HITOBJECTS`: the player's centre is 2 or more tiles from the
    /// hit point on an axis.
    OutOfBlast,
}

/// Whether the player can be hurt right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vulnerability {
    Hurtable,
    Dead,
    Hurt,
    Swimming,
    /// Hiding under a carried object (2.05 mode 14).
    Hidden,
    /// Paused (STATUS 0x01).
    Paused,
}

impl Vulnerability {
    fn check(self) -> Result<(), Reject> {
        match self {
            Vulnerability::Hurtable => Ok(()),
            Vulnerability::Dead => Err(Reject::Dead),
            Vulnerability::Hurt => Err(Reject::AlreadyHurt),
            Vulnerability::Swimming => Err(Reject::Swimming),
            Vulnerability::Hidden => Err(Reject::Hidden),
            Vulnerability::Paused => Err(Reject::Paused),
        }
    }
}

/// The player on the receiving end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Victim {
    pub x: f32,
    pub y: f32,
    /// Alignment points.
    pub ap: u8,
    pub state: Vulnerability,
    /// The level is a no-player-killing level (script `noplayerkilling`).
    pub no_pk: bool,
}

/// A player swinging a sword, as the victim sees it from the props.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swinger {
    pub pose: Pose,
    /// SPRITE (17) as received: `dir + 4 * S`.
    pub sprite: u8,
    /// The attacker's GANI (10, 2.1+) is a swing ([`is_swing_gani`]). A
    /// GANI client sends the direction alone in SPRITE, so this is how its
    /// swing is seen.
    pub swing_gani: bool,
    /// SWORDPOWER (8) as last received, if ever: -20 to 20, negative heals.
    pub sword_power: Option<i8>,
    /// STATUS (18) as last received, if ever.
    pub status: Option<u8>,
    /// The attacker's "(guild)" tag is in the victim's nickname
    /// ([`same_guild`]).
    pub friendly: bool,
    /// The attacker was already judged in this second ([`CombatRules::
    /// judge_throttle`]).
    pub repeat: bool,
}

/// The "(guild)" tag of a nickname: from the first `(` to the first `)`
/// after it, brackets included, when at least one character lies between
/// them. Nicknames are compared as the client keeps them, case-sensitive.
pub fn guild_tag(nick: &str) -> Option<&str> {
    let rest = &nick[nick.find('(')?..];
    let close = rest.find(')')?;
    (close >= 2).then(|| &rest[..=close])
}

/// The victim's client does not let a hit through when the attacker's
/// "(guild)" tag appears anywhere in the victim's own nickname.
pub fn same_guild(attacker_nick: &str, victim_nick: &str) -> bool {
    guild_tag(attacker_nick).is_some_and(|t| victim_nick.contains(t))
}

fn half_tiles(h: u8) -> f32 {
    f32::from(h) / 2.0
}

/// `(forward, sideways)` distance of the point `(tx, ty)` from `a`: forward
/// is along the way `a` faces (negative behind it), sideways is the
/// absolute offset across it. Tiles.
pub fn forward_and_side(a: Pose, tx: f32, ty: f32) -> (f32, f32) {
    let (dx, dy) = (tx - a.x, ty - a.y);
    match a.dir % 4 {
        0 => (-dy, dx.abs()),
        1 => (-dx, dy.abs()),
        2 => (dy, dx.abs()),
        _ => (dx, dy.abs()),
    }
}

/// The sword box: 0 to 4 tiles ahead (both ends count), under 2 tiles to the
/// side.
pub fn in_sword_box(rules: &CombatRules, a: Pose, tx: f32, ty: f32) -> Result<(), Reject> {
    let (fwd, side) = forward_and_side(a, tx, ty);
    if fwd < half_tiles(rules.reach_min_half) || fwd > half_tiles(rules.reach_max_half) {
        return Err(Reject::OutOfReach);
    }
    if side >= half_tiles(rules.lateral_below_half) {
        return Err(Reject::OffLine);
    }
    Ok(())
}

/// The sprite number S of the carrystill pose in 2.05 (animation step 24).
/// Seihai sends it the whole time an object is held, walking included, as
/// it sends S = 0 for a plain walk.
pub const CARRY_SPRITE_S: u8 = 18;

/// The sprite number S (animation) in a SPRITE value, and the direction.
pub fn sprite_parts(sprite: u8) -> (u8, u8) {
    (sprite / 4, sprite % 4)
}

/// True when `sprite` is one of the sword-swing frames.
pub fn is_swing_sprite(rules: &CombatRules, sprite: u8) -> bool {
    let s = sprite_parts(sprite).0;
    s >= rules.swing_sprite_first && s <= rules.swing_sprite_last
}

/// The GANI names that are a swing: the ones the 2.05 client turns into
/// the swing steps 9 to 13 (S 6 to 10), by its gani-to-step rule
/// (Graal_analysis `feature-specs/11-emotes-ganis.md`). That a 2.1+ client
/// judges a swing by these names is inferred from that rule and from its
/// kick test, which reads the attacker's animation (`kick` is index 3 of the
/// built-in names) instead of a sprite; the 2.1+ swing test was not read.
pub const SWING_GANIS: [&str; 3] = ["sword", "ridesword", "spin"];

/// True when another player's GANI prop (10, 2.1+) names a swing.
pub fn is_swing_gani(gani: &str) -> bool {
    // GServer-v2 adds `.gif` to the name a 2.05 client sends in the bow slot.
    let name = gani.strip_suffix(".gani").or_else(|| gani.strip_suffix(".gif")).unwrap_or(gani);
    SWING_GANIS.contains(&name)
}

/// Whether the build sends its own swing as an animation name in prop 10
/// (the pre-2.1 bow slot, [`crate::net::bow_slot_message`]) and keeps SPRITE
/// on the direction. The lab decodes of the real 2.00 and 2.05 (2026-10-30)
/// show exactly that: `sword` as the swing starts (2.00 then `idle` about
/// 250 ms later), and SPRITE only ever the direction, or 128 + direction
/// while hurt. 2.01 to 2.04 lie between the two and are taken to do the
/// same (not captured). A real 2.00 took no damage from Seihai's sprite
/// swing (S 6 to 10) in the same lab.
pub fn swing_in_bow_slot(v: ClientVersion) -> bool {
    matches!(
        v,
        ClientVersion::V200 | ClientVersion::V201 | ClientVersion::V202 | ClientVersion::V202R2 | ClientVersion::V203 | ClientVersion::V204 | ClientVersion::V205
    )
}

/// The attacker is on a swing: a swing sprite (2.05), or a swing GANI (2.1+).
pub fn is_swinging(rules: &CombatRules, s: &Swinger) -> bool {
    s.swing_gani || is_swing_sprite(rules, s.sprite)
}

/// Half hearts a sword hit takes: the attacker's sword power as the victim
/// stores it ([`PowerStore`]). From 2.03 that is the power limited to the
/// profile's plus or minus 20, and a negative one heals (see
/// [`CombatRules::heal_min_ap`]); before 2.03 it is the power in 6 bits, 0 to
/// 63. 0 only starts hurt mode.
pub fn sword_half_hearts(rules: &CombatRules, power: i8) -> i32 {
    match rules.sword_power_store {
        PowerStore::Limited => {
            let lim = i32::from(rules.sword_power_limit);
            i32::from(power).clamp(-lim, lim)
        }
        PowerStore::Raw6Bit => i32::from(power).rem_euclid(64),
    }
}

/// The victim's judgement of a sword swing. `Ok` is the signed half hearts
/// the hit asks for (negative: heal). The attacker must be on a swing
/// frame, the victim hurtable, its AP not immune, the level not a no-kill
/// level, the attacker not hidden and not on the victim's guild, and the
/// victim in the box in front of the attacker.
pub fn judge_sword(rules: &CombatRules, s: &Swinger, v: &Victim) -> Result<i32, Reject> {
    if !is_swinging(rules, s) {
        return Err(Reject::NotSwinging);
    }
    v.state.check()?;
    if v.ap == rules.immune_ap {
        return Err(Reject::Immune);
    }
    if v.no_pk {
        return Err(Reject::NoPkZone);
    }
    if rules.judge_throttle && s.repeat {
        return Err(Reject::Throttled);
    }
    if s.friendly {
        return Err(Reject::Friendly);
    }
    if s.status.is_some_and(|st| st & rules.attacker_blocked_status != 0) {
        return Err(Reject::AttackerBlocked);
    }
    let pose = Pose { dir: sprite_parts(s.sprite).1, ..s.pose };
    in_sword_box(rules, pose, v.x, v.y)?;
    let power = s.sword_power.ok_or(Reject::UnknownPower)?;
    Ok(sword_half_hearts(rules, power))
}

/// A decoded `PLO_HURTPLAYER` (40).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HurtPlayer {
    pub attacker: u16,
    /// Knockback, tiles (-4 to 4).
    pub dx: f32,
    pub dy: f32,
    /// Half hearts, as sent (not yet capped).
    pub half: i32,
    /// The hit came from this NPC, not from the player.
    pub npc: Option<u32>,
}

/// A decoded `PLO_HITOBJECTS` (46).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitObjects {
    pub attacker: u16,
    pub half: i32,
    /// The hit's centre, tiles.
    pub x: f32,
    pub y: f32,
    pub npc: Option<u32>,
}

/// A packet hit waiting for the game loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IncomingHit {
    Hurt(HurtPlayer),
    Objects(HitObjects),
}

fn gc(b: u8) -> i32 {
    i32::from(b) - 0x20
}

/// `GSHORT` at the start of `b`.
fn id_of(b: &[u8]) -> Option<(u16, &[u8])> {
    let (hi, lo) = (*b.first()?, *b.get(1)?);
    Some((((gc(hi).clamp(0, 127) as u16) << 7) | gc(lo).clamp(0, 127) as u16, &b[2..]))
}

fn npc_of(rest: &[u8]) -> Option<u32> {
    let t = rest.get(3..6)?;
    let v = (gc(t[0]).clamp(0, 127) << 14) | (gc(t[1]).clamp(0, 127) << 7) | gc(t[2]).clamp(0, 127);
    Some(v as u32)
}

/// Decode `PLO_HURTPLAYER` (40): `GSHORT attacker, c dx, c dy, c power`
/// (+ `GINT3 npc` when 6 or more bytes follow the id). Knockback is
/// `(byte - 0x60) / 16`, power `(byte - 32)` half hearts.
pub fn parse_hurtplayer(payload: &[u8]) -> Option<HurtPlayer> {
    let (attacker, rest) = id_of(payload)?;
    if rest.len() < 3 {
        return None;
    }
    Some(HurtPlayer {
        attacker,
        dx: (i32::from(rest[0]) - 0x60) as f32 / 16.0,
        dy: (i32::from(rest[1]) - 0x60) as f32 / 16.0,
        half: gc(rest[2]).max(0),
        npc: if rest.len() >= 6 { npc_of(rest) } else { None },
    })
}

/// Decode `PLO_HITOBJECTS` (46): `GSHORT attacker, c power*2, c x*2, c y*2`
/// (+ `GINT3 npc`).
pub fn parse_hitobjects(payload: &[u8]) -> Option<HitObjects> {
    let (attacker, rest) = id_of(payload)?;
    if rest.len() < 3 {
        return None;
    }
    Some(HitObjects {
        attacker,
        half: gc(rest[0]).max(0),
        x: gc(rest[1]).max(0) as f32 / 2.0,
        y: gc(rest[2]).max(0) as f32 / 2.0,
        npc: if rest.len() >= 6 { npc_of(rest) } else { None },
    })
}

fn within(a: (f32, f32), b: (f32, f32), half_tiles_range: u8) -> bool {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    let r = half_tiles(half_tiles_range);
    dx * dx + dy * dy <= r * r
}

/// The gates `PLO_HURTPLAYER` and `PLO_HITOBJECTS` share, in the order the
/// client runs them (read in 2.05 rev 2 and 2.22): the victim can be hurt,
/// the attacker is a known player (an NPC source is refused here), the
/// victim's AP is not 100, the level is not `noplayerkilling`, the attacker
/// was not already judged this second, its `(tag)` is not in the victim's
/// nickname, and the attacker is within the profile's range (10 tiles).
fn packet_gates(
    rules: &CombatRules,
    npc: Option<u32>,
    attacker_at: Option<(f32, f32)>,
    v: &Victim,
    g: PacketGates,
) -> Result<(), Reject> {
    if npc.is_some() {
        return Err(Reject::NpcSource);
    }
    v.state.check()?;
    let at = attacker_at.ok_or(Reject::TooFar)?;
    if rules.packet_victim_gates && v.ap == rules.immune_ap {
        return Err(Reject::Immune);
    }
    if rules.packet_victim_gates && v.no_pk {
        return Err(Reject::NoPkZone);
    }
    if rules.judge_throttle && g.repeat {
        return Err(Reject::Throttled);
    }
    if g.friendly {
        return Err(Reject::Friendly);
    }
    if !within(at, (v.x, v.y), rules.hurtplayer_range_half) {
        return Err(Reject::TooFar);
    }
    Ok(())
}

/// The victim's judgement of a `PLO_HURTPLAYER` with the default gates (no
/// guild match, not a repeat): see [`judge_hurtplayer_gated`].
pub fn judge_hurtplayer(rules: &CombatRules, h: &HurtPlayer, attacker_at: Option<(f32, f32)>, v: &Victim) -> Result<i32, Reject> {
    judge_hurtplayer_gated(rules, h, attacker_at, v, PacketGates::default())
}

/// The victim's judgement of a `PLO_HURTPLAYER`: the shared packet gates
/// ([`PacketGates`]), then the power capped at one heart. A power of 0 is
/// accepted (hurt mode and knockback start, no hearts change). `attacker_at`
/// is where the attacker is, in the victim's coordinates, or `None` when it
/// is not known. A hit from an NPC has no known position and is refused.
pub fn judge_hurtplayer_gated(
    rules: &CombatRules,
    h: &HurtPlayer,
    attacker_at: Option<(f32, f32)>,
    v: &Victim,
    g: PacketGates,
) -> Result<i32, Reject> {
    packet_gates(rules, h.npc, attacker_at, v, g)?;
    Ok(h.half.clamp(0, i32::from(rules.packet_hit_cap_half)))
}

/// The victim's judgement of a `PLO_HITOBJECTS`: the shared packet gates,
/// then the player's centre (`X + 1.5, Y + 2.0`) must be strictly under 2
/// tiles from the hit point on each axis. Power is capped at one heart as
/// for id 40. Returns the half hearts and the knockback vector (from the hit
/// point to the player's centre, not yet scaled).
pub fn judge_hitobjects(
    rules: &CombatRules,
    h: &HitObjects,
    attacker_at: Option<(f32, f32)>,
    v: &Victim,
    g: PacketGates,
) -> Result<(i32, (f32, f32)), Reject> {
    packet_gates(rules, h.npc, attacker_at, v, g)?;
    let (dx, dy) = (v.x + HIT_CENTRE.0 - h.x, v.y + HIT_CENTRE.1 - h.y);
    if dx.abs() >= HITOBJECTS_BOX_TILES || dy.abs() >= HITOBJECTS_BOX_TILES {
        return Err(Reject::OutOfBlast);
    }
    Ok((h.half.clamp(0, i32::from(rules.packet_hit_cap_half)), (dx, dy)))
}

/// What the attacker knows about a player it might hit, from that player's
/// props.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub id: u16,
    pub x: f32,
    pub y: f32,
    /// STATUS (18) as last received.
    pub status: Option<u8>,
    /// SPRITE (17) as last received.
    pub sprite: Option<u8>,
}

/// The attacker's side of the same rules, for the hits it sends: nothing
/// goes out that the victim would refuse, where the attacker can tell. It
/// can see the dead bit (STATUS 0x08), hurt mode (sprite S = 32) and the
/// distance. It cannot see swimming, so that is left to the victim.
pub fn attacker_may_send(rules: &CombatRules, from: (f32, f32), t: &Target) -> Result<(), Reject> {
    if t.status.is_some_and(|s| s & STATUS_DEAD != 0) {
        return Err(Reject::Dead);
    }
    if t.sprite.is_some_and(|s| sprite_parts(s).0 == rules.hurt_sprite) {
        return Err(Reject::AlreadyHurt);
    }
    if !within(from, (t.x, t.y), rules.hurtplayer_range_half) {
        return Err(Reject::TooFar);
    }
    Ok(())
}

/// The `PLI_HURTPLAYER` (26) message for `victim`, or why it is not sent.
/// `knock` is the knockback in tiles (-4 to 4). The power is capped at the
/// profile's limit, and the id must be in the build's send set.
pub fn hurtplayer_message(
    version: ClientVersion,
    from: (f32, f32),
    t: &Target,
    knock: (f32, f32),
    half: i32,
) -> Result<Vec<u8>, Reject> {
    let p = version.profile();
    let Some(rules) = p.life.combat else {
        return Err(Reject::NotInBuild);
    };
    if !p.protocol.pli.contains(PLI_HURTPLAYER) {
        return Err(Reject::NotInBuild);
    }
    attacker_may_send(&rules, from, t)?;
    let half = half.min(i32::from(rules.packet_hit_cap_half));
    if half <= 0 {
        return Err(Reject::NoDamage);
    }
    let kb = |k: f32| (((k.clamp(-4.0, 3.9375) + 4.0) * 16.0).round() as u8).wrapping_add(0x20);
    let mut m = vec![PLI_HURTPLAYER + 0x20];
    m.extend_from_slice(&crate::net::gshort(t.id));
    m.push(kb(knock.0));
    m.push(kb(knock.1));
    m.push(half as u8 + 0x20);
    m.push(b'\n');
    Ok(m)
}

/// The swing and the hurt state of one session.
#[derive(Debug, Clone, Default)]
pub struct CombatState {
    rules: Option<CombatRules>,
    /// Index into the swing frames (0 is the first, step 9) and the
    /// milliseconds spent on it.
    swing: Option<(u8, u64)>,
    hurt_left: u16,
    hurt_acc_ms: u64,
    /// Knockback direction (unit vector) of the hit that started hurt mode.
    knock: Option<(f32, f32)>,
    /// Knockback steps that came due in the ticks just run; the session
    /// takes them with [`CombatState::take_knock_steps`].
    knock_steps: Vec<(f32, f32)>,
    /// The spin attack (mode 13) in progress: milliseconds in, and its
    /// length in ticks.
    spin: Option<(u64, u16)>,
    /// The has-spin status (STATUS bit 0x40) is set.
    has_spin: bool,
    /// Knockback steps per hit when the build differs from
    /// [`KNOCKBACK_TICKS`] (1.39: 4).
    knockback_ticks: Option<u16>,
    swing_ticks: Option<u16>,
}

/// The STATUS (18) bit that allows the spin attack.
pub const HAS_SPIN_STATUS: u8 = 0x40;
/// Ticks the spin lasts when no `spin.gani` gives its length (the stock
/// file: 13 steps of one tick).
pub const SPIN_TICKS: u16 = 13;

/// One client tick, in milliseconds.
const TICK_MS: u64 = crate::life::TICK_MS;

impl CombatState {
    pub fn new(rules: Option<CombatRules>) -> Self {
        CombatState { rules, ..Self::default() }
    }

    pub fn rules(&self) -> Option<&CombatRules> {
        self.rules.as_ref()
    }

    /// Knockback steps per hit, the build's own (`Mannerisms`).
    pub fn set_knockback_ticks(&mut self, n: u16) {
        self.knockback_ticks = Some(n);
    }

    /// The swing lasts `ticks` of 50 ms in all, spread over its animation
    /// steps: the length of the sword GANI, where the build plays one. The
    /// real 2.22 sent `GANI idle` 250 ms after `GANI sword` with a five-step
    /// `sword.gani` (lab, 2026-10-30). Without it, `swing_step_ticks`.
    pub fn set_swing_ticks(&mut self, ticks: Option<u16>) {
        self.swing_ticks = ticks;
    }

    /// Take the rules of another build. Ends a swing and a hurt in progress.
    pub fn set_rules(&mut self, rules: Option<CombatRules>) {
        let has_spin = self.has_spin;
        let k = self.knockback_ticks;
        *self = CombatState::new(rules);
        self.has_spin = has_spin;
        self.knockback_ticks = k;
    }

    pub fn is_hurt(&self) -> bool {
        self.hurt_left > 0
    }

    pub fn is_swinging(&self) -> bool {
        self.swing.is_some()
    }

    /// The swing step the sword is on (0 is the first, sprite S = 6).
    pub fn swing_frame(&self) -> Option<usize> {
        self.swing.map(|(i, _)| usize::from(i))
    }

    /// Milliseconds since hurt mode began, or `None` when not hurt.
    pub fn hurt_elapsed_ms(&self) -> Option<u64> {
        let r = self.rules.as_ref()?;
        (self.hurt_left > 0).then(|| u64::from(r.hurt_ticks - self.hurt_left.min(r.hurt_ticks)) * TICK_MS + self.hurt_acc_ms)
    }

    /// Start the swing. False if the build has no combat rules, a swing is
    /// already out or a spin is on.
    pub fn start_swing(&mut self) -> bool {
        if self.rules.is_none() || self.swing.is_some() || self.spin.is_some() {
            return false;
        }
        self.swing = Some((0, 0));
        true
    }

    /// The server's STATUS has the has-spin bit (0x40), or not.
    pub fn set_has_spin(&mut self, on: bool) {
        self.has_spin = on;
    }

    /// A second S press during a swing, with the has-spin status: the swing
    /// is cut off and the spin (mode 13) runs for `ticks`. Read live in
    /// 2.05 rev 2 online (Graal_analysis `player-mannerisms-live.md`,
    /// "Spin attack"): a press during the spin, or with no swing out, does
    /// not start one.
    pub fn start_spin(&mut self, ticks: u16) -> bool {
        if !self.has_spin || self.swing.is_none() || self.spin.is_some() {
            return false;
        }
        self.swing = None;
        self.spin = Some((0, ticks.max(1)));
        true
    }

    /// Milliseconds into the spin, or `None` when there is none.
    pub fn spin_elapsed_ms(&self) -> Option<u64> {
        self.spin.map(|(ms, _)| ms)
    }

    pub fn cancel_swing(&mut self) -> bool {
        self.swing.take().is_some()
    }

    /// Hurt mode starts (40 ticks), with no knockback.
    pub fn start_hurt(&mut self) {
        self.start_hurt_knock(None);
    }

    /// Hurt mode starts (40 ticks). `knock` is the hit vector: the player is
    /// pushed one tile a tick along it for the first five ticks (the vector
    /// is scaled to length 1; a zero vector pushes east).
    pub fn start_hurt_knock(&mut self, knock: Option<(f32, f32)>) {
        if let Some(r) = &self.rules {
            self.hurt_left = r.hurt_ticks;
            self.hurt_acc_ms = 0;
            self.knock = knock.map(|(x, y)| knockback_unit(x, y));
            self.knock_steps.clear();
        }
    }

    /// The knockback steps (unit vectors) that came due since the last call.
    pub fn take_knock_steps(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.knock_steps)
    }

    /// The SPRITE (17) value to send for a player facing `dir`: the swing
    /// frame while the sword is out, the hurt sprite while hurt, otherwise
    /// plain idle (S = 0, as every walk message sends).
    pub fn sprite(&self, dir: u8) -> u8 {
        let Some(r) = &self.rules else { return dir % 4 };
        let s = match (self.swing, self.hurt_left) {
            (Some((i, _)), _) => r.swing_sprite_first + i,
            (None, h) if h > 0 => r.hurt_sprite,
            _ => 0,
        };
        (dir % 4).wrapping_add(s.wrapping_mul(4))
    }

    /// Advance by `ms`. True when the sprite others see changed (a swing
    /// frame, the end of the swing, the end of hurt mode).
    pub fn tick(&mut self, ms: u64) -> bool {
        let Some(r) = self.rules else { return false };
        let before = self.sprite(0);
        if let Some((ms0, len)) = self.spin {
            let ms1 = ms0 + ms;
            self.spin = (ms1 < u64::from(len) * TICK_MS).then_some((ms1, len));
        }
        if let Some((i, acc)) = self.swing {
            let acc = acc + ms;
            let frames = r.swing_sprite_last - r.swing_sprite_first + 1;
            let per = match self.swing_ticks {
                Some(t) => (u64::from(t) * TICK_MS / u64::from(frames)).max(1),
                None => u64::from(r.swing_step_ticks.max(1)) * TICK_MS,
            };
            let step = (acc / per) as u8;
            self.swing = if i + step >= frames { None } else { Some((i + step, acc % per)) };
        }
        if self.hurt_left > 0 {
            self.hurt_acc_ms += ms;
            let ticks = (self.hurt_acc_ms / TICK_MS) as u16;
            self.hurt_acc_ms %= TICK_MS;
            // The client counts down first and pushes while the counter is
            // still within the first five ticks of hurt mode.
            for _ in 0..ticks {
                if self.hurt_left == 0 {
                    break;
                }
                self.hurt_left -= 1;
                if let Some(k) = self.knock {
                    if self.hurt_left + self.knockback_ticks.unwrap_or(KNOCKBACK_TICKS) >= r.hurt_ticks {
                        self.knock_steps.push(k);
                    } else {
                        self.knock = None;
                    }
                }
            }
        }
        self.sprite(0) != before
    }
}

/// Where a hit came from, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    Sword,
    HurtPlayer,
    HitObjects,
}

/// What became of one hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitReport {
    pub kind: HitKind,
    pub attacker: Option<u16>,
    /// Half hearts taken, or why not.
    pub outcome: Result<i32, Reject>,
}

impl OfflineSession {
    /// The player as the judge of a hit sees them.
    pub fn victim(&self) -> Victim {
        let state = if self.life.is_dead() || self.hearts_half() <= 0 {
            Vulnerability::Dead
        } else if self.combat.is_hurt() {
            Vulnerability::Hurt
        } else if self.in_water {
            Vulnerability::Swimming
        } else if self.hidden {
            Vulnerability::Hidden
        } else if self.paused {
            Vulnerability::Paused
        } else {
            Vulnerability::Hurtable
        };
        Victim { x: self.player_x, y: self.player_y, ap: self.alignment, state, no_pk: self.no_pk }
    }

    /// A hit that was judged good: take the hearts, name the attacker for a
    /// claim, start hurt mode. `half` is signed: a negative one heals, but
    /// only a victim at the profile's `heal_min_ap` or more; below it the
    /// hit changes no hearts and still starts hurt mode, as does a power of
    /// 0. Returns the half hearts taken (negative: healed, 0: none).
    fn take_hit(&mut self, half: i32, attacker: Option<u16>, knock: (f32, f32)) -> Result<i32, Reject> {
        if self.life.is_dead() {
            return Err(Reject::Dead);
        }
        let heal_min_ap = self.combat.rules().map_or(u8::MAX, |r| r.heal_min_ap);
        let applied = if half < 0 && self.alignment < heal_min_ap { 0 } else { half };
        if applied > 0 {
            self.damage_half(applied, attacker);
        } else if applied < 0 {
            let healed = (self.hearts_half() - applied).min(self.max_hearts);
            self.set_hearts_half(healed);
        }
        self.combat.start_hurt_knock(Some(knock));
        Ok(applied)
    }

    /// A player's props showed a sword swing: judge it from this side.
    /// `own_nick` is the victim's nickname (for the guild test) and `second`
    /// the client's clock as seconds of the day (for the once-a-second
    /// limit per attacker).
    pub fn receive_sword(&mut self, id: Option<u16>, s: &Swinger, second: u32) -> HitReport {
        let outcome = match self.combat.rules().copied() {
            None => Err(Reject::NotInBuild),
            Some(r) => {
                let v = self.victim();
                let mut s = *s;
                // The client stamps the attacker once the swing, the
                // victim's state, AP and level have passed, before the
                // guild and box tests.
                let reaches = is_swinging(&r, &s) && v.state == Vulnerability::Hurtable && v.ap != r.immune_ap && !v.no_pk;
                if let (true, true, Some(id)) = (r.judge_throttle, reaches, id) {
                    s.repeat = self.judged_at.insert(id, second) == Some(second);
                }
                // Knockback runs from the attacker to the victim.
                let knock = (v.x - s.pose.x, v.y - s.pose.y);
                judge_sword(&r, &s, &v).and_then(|h| self.take_hit(h, id, knock))
            }
        };
        HitReport { kind: HitKind::Sword, attacker: id, outcome }
    }

    /// The stamp of the hit judge: true when this attacker was already
    /// judged in `second`. The client stamps an attacker once the victim's
    /// state, AP and level have passed, before the guild and range tests.
    fn stamp_attacker(&mut self, rules: &CombatRules, id: u16, second: Option<u32>, npc: bool) -> bool {
        let v = self.victim();
        let reaches = v.state == Vulnerability::Hurtable && v.ap != rules.immune_ap && !v.no_pk;
        match (rules.judge_throttle && reaches && !npc, second) {
            (true, Some(sec)) => self.judged_at.insert(id, sec) == Some(sec),
            _ => false,
        }
    }

    /// A `PLO_HURTPLAYER` with the default gates (see
    /// [`OfflineSession::receive_hurtplayer_gated`]).
    pub fn receive_hurtplayer(&mut self, h: &HurtPlayer, attacker_at: Option<(f32, f32)>) -> HitReport {
        self.receive_hurtplayer_gated(h, attacker_at, false, None)
    }

    /// A `PLO_HURTPLAYER`. `attacker_at` is where the attacker is, in this
    /// player's coordinates. The packet goes through the sword's gates: AP
    /// 100, a no-player-killing level, the once-a-second limit per attacker
    /// (`second` is the client's second of the day, `None` skips the limit),
    /// the guild tag (`friendly`) and the 10-tile range. The victim is pushed
    /// along the packet's `dx, dy`.
    pub fn receive_hurtplayer_gated(&mut self, h: &HurtPlayer, attacker_at: Option<(f32, f32)>, friendly: bool, second: Option<u32>) -> HitReport {
        let outcome = match self.combat.rules().copied() {
            None => Err(Reject::NotInBuild),
            Some(r) => {
                let repeat = attacker_at.is_some() && self.stamp_attacker(&r, h.attacker, second, h.npc.is_some());
                judge_hurtplayer_gated(&r, h, attacker_at, &self.victim(), PacketGates { friendly, repeat })
                    .and_then(|half| self.take_hit(half, Some(h.attacker), (h.dx, h.dy)))
            }
        };
        HitReport { kind: HitKind::HurtPlayer, attacker: Some(h.attacker), outcome }
    }

    /// A `PLO_HITOBJECTS` with the default gates.
    pub fn receive_hitobjects(&mut self, h: &HitObjects, attacker_at: Option<(f32, f32)>) -> HitReport {
        self.receive_hitobjects_gated(h, attacker_at, false, None)
    }

    /// A `PLO_HITOBJECTS`: the same gates as [`Self::receive_hurtplayer_gated`],
    /// then the player's centre (`X + 1.5, Y + 2.0`) must be under 2 tiles
    /// from `(x, y)` on each axis. The victim is pushed away from that point.
    pub fn receive_hitobjects_gated(&mut self, h: &HitObjects, attacker_at: Option<(f32, f32)>, friendly: bool, second: Option<u32>) -> HitReport {
        let outcome = match self.combat.rules().copied() {
            None => Err(Reject::NotInBuild),
            Some(r) => {
                let repeat = attacker_at.is_some() && self.stamp_attacker(&r, h.attacker, second, h.npc.is_some());
                judge_hitobjects(&r, h, attacker_at, &self.victim(), PacketGates { friendly, repeat })
                    .and_then(|(half, knock)| self.take_hit(half, Some(h.attacker), knock))
            }
        };
        HitReport { kind: HitKind::HitObjects, attacker: Some(h.attacker), outcome }
    }

    /// Judge everything that arrived: `swings` (ids of players whose props
    /// showed a swing) and packet `hits`, against `others` (the players in
    /// the level, from their props) and `own_cell` (our gmap cell, so a
    /// player in another cell is placed relative to us). Reports every hit,
    /// taken or refused, in order: swings first, then packets.
    pub fn receive_all(
        &mut self,
        others: &[OtherPlayer],
        own_cell: Option<(i32, i32)>,
        own_nick: Option<&str>,
        second: u32,
        swings: &[u16],
        hits: &[IncomingHit],
    ) -> Vec<HitReport> {
        let mut out = Vec::new();
        for id in swings {
            let Some(o) = others.iter().find(|o| o.id == *id) else { continue };
            let swing_gani = o.looks.swing_gani();
            // A GANI client may never have sent SPRITE: its direction is all
            // the sprite would hold.
            let Some(sprite) = o.sprite.or(swing_gani.then_some(o.dir % 4)) else { continue };
            let (x, y) = o.position_from(own_cell);
            let friendly = matches!((o.nick.as_deref(), own_nick), (Some(a), Some(v)) if same_guild(a, v));
            let s = Swinger {
                pose: Pose { x, y, dir: o.dir },
                sprite,
                swing_gani,
                sword_power: o.sword_power,
                status: o.status,
                friendly,
                repeat: false,
            };
            out.push(self.receive_sword(Some(o.id), &s, second));
        }
        for h in hits {
            let attacker = match h {
                IncomingHit::Hurt(h) => h.attacker,
                IncomingHit::Objects(h) => h.attacker,
            };
            let other = others.iter().find(|o| o.id == attacker);
            let at = other.map(|o| o.position_from(own_cell));
            let friendly = matches!((other.and_then(|o| o.nick.as_deref()), own_nick), (Some(a), Some(v)) if same_guild(a, v));
            match h {
                IncomingHit::Hurt(h) => out.push(self.receive_hurtplayer_gated(h, at, friendly, Some(second))),
                IncomingHit::Objects(h) => out.push(self.receive_hitobjects_gated(h, at, friendly, Some(second))),
            }
        }
        out
    }

    /// Start a sword swing on the player's own key press. Not while dead,
    /// swimming, or already swinging, and not with no sword. Not while
    /// carrying either: the 2.05 client's sword key throws what is carried
    /// instead (the caller passes the key on to the interact).
    pub fn start_swing(&mut self) -> bool {
        // An open sign takes S (2.05): it turns the page instead.
        if self.life.is_dead() || self.in_water || self.sword_power == 0 || self.warp_pending.is_some() || self.carried.is_some() || self.reading.is_some() {
            return false;
        }
        // S again during a swing: the spin attack, if the status allows it.
        let spin_len = self.poses.spin.as_ref().map_or(SPIN_TICKS, |g| g.total_ticks().max(1) as u16);
        if self.combat.start_spin(spin_len) {
            return true;
        }
        let sword_ticks = self.poses.sword.as_ref().map(|g| g.total_ticks().max(1) as u16);
        self.combat.set_swing_ticks(sword_ticks.filter(|_| self.gani_swing));
        let started = self.combat.start_swing();
        if started {
            self.queue_sword_cues(0);
            // The client cuts what is in front of the sword as the swing
            // starts.
            self.try_cut();
        }
        started
    }

    /// Advance the swing and hurt mode by `ms`. True when the sprite others
    /// see changed. A death ends the swing.
    pub fn tick_combat(&mut self, ms: u64) -> bool {
        let cancelled = self.life.is_dead() && self.combat.cancel_swing();
        let before = self.combat.swing_frame();
        let changed = self.combat.tick(ms);
        // Each swing step entered plays the cues that follow it in sword.gani
        // (a swing that ends has entered the steps up to its last).
        if let Some(b) = before {
            let swing_steps = self.combat.rules().map_or(1, |r| usize::from(r.swing_sprite_last - r.swing_sprite_first) + 1);
            let last = self.poses.sword.as_ref().map_or(0, |g| g.frame_count().min(swing_steps).saturating_sub(1));
            let to = self.combat.swing_frame().unwrap_or(last);
            for step in b + 1..=to {
                self.queue_sword_cues(step);
            }
        }
        for (ux, uy) in self.combat.take_knock_steps() {
            let moved = self.knock_step(ux, uy);
            self.knock_moved |= moved;
        }
        changed || cancelled
    }

    /// The SPRITE (17) value for the player's own walk and swing messages.
    pub fn sprite_value(&self) -> u8 {
        self.sprite_value_for(false)
    }

    /// The animation a bow-slot build ([`swing_in_bow_slot`]) names in prop
    /// 10 now: `sword` during a swing, `spin` during the spin, the lift,
    /// carry and hidden poses with an object, else `idle`.
    ///
    /// Read in the 2.05 rev 2 decompile: the sender writes its current
    /// gani's name in prop 10. A watcher draws another player's carried
    /// object only while that player's gani step is 23 to 32 (`lift`,
    /// `carrystill` / `hiddenstill`, `carry` / `hidden`, or `def` driven by
    /// SPRITE), and the STATUS hidden bit (0x02) leaves out the figure and
    /// the name. Under a bush with `idle` in prop 10 (GServer's default), a
    /// real 2.05 drew nothing at all.
    pub fn bow_slot_anim(&self) -> &'static str {
        if self.combat.spin_elapsed_ms().is_some() {
            "spin"
        } else if self.combat.is_swinging() {
            "sword"
        } else if self.hidden {
            if self.walking { "hidden" } else { "hiddenstill" }
        } else if self.lift_ticks.is_some_and(|t| t < crate::offline::LIFT_CARRIED_TICK) {
            "lift"
        } else if self.carried.is_some() {
            if self.walking { "carry" } else { "carrystill" }
        } else {
            "idle"
        }
    }

    /// The SPRITE (17) a bow-slot build sends: the swing is in prop 10, so
    /// a swing frame goes out as the plain direction.
    pub fn bow_slot_sprite(&self, gani_client: bool) -> u8 {
        if self.combat.is_swinging() {
            self.player_dir % 4
        } else {
            self.sprite_value_for(gani_client)
        }
    }

    /// [`Self::sprite_value`] as the client sends it. A GANI client
    /// (`gani_client`, 2.1+) plays the push and pull poses from its own
    /// GANIs and sends SPRITE only when it changes
    /// (`documentation/difftest-leftovers.md`), so the pose keeps its first
    /// number: stepping the 2.05 numbers sent a walk update about six times
    /// a second while the player pushed against a wall.
    pub fn sprite_value_for(&self, gani_client: bool) -> u8 {
        let s = self.combat.sprite(self.player_dir);
        // Hidden: `hiddenstill` (18) or the `hidden` walk.
        if let Some(h) = self.hidden_sprite_s().filter(|_| s == self.player_dir % 4) {
            return (self.player_dir % 4) + 4 * h;
        }
        // Holding an object (no swing can be out then): the carrystill
        // sprite, S = 18 (2.05 step 24). Hurt mode keeps its own sprite.
        if self.carried.is_some() && s == self.player_dir % 4 {
            return (self.player_dir % 4) + 4 * CARRY_SPRITE_S;
        }
        // Pushing, grabbing, pulling: their own sprite numbers.
        if s == self.player_dir % 4 {
            if let Some(ps) = self.pushpull_sprite_s_stepped(!gani_client) {
                return (self.player_dir % 4) + 4 * ps;
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::life::LifeEvent;
    use crate::protocol::{ClientVersion, SessionState, COMBAT_200, COMBAT_201, COMBAT_205, DEATH_205, PROFILES};

    const R: CombatRules = COMBAT_205;

    /// SPRITE value of swing frame `s` facing `dir`.
    fn sw(s: u8, dir: u8) -> u8 {
        dir + 4 * s
    }

    fn at(x: f32, y: f32, dir: u8) -> Pose {
        Pose { x, y, dir }
    }

    fn victim(x: f32, y: f32) -> Victim {
        Victim { x, y, ap: 50, state: Vulnerability::Hurtable, no_pk: false }
    }

    fn swinger(dir: u8, sprite: u8) -> Swinger {
        Swinger { pose: at(20.0, 20.0, dir), sprite, swing_gani: false, sword_power: Some(1), status: None, friendly: false, repeat: false }
    }

    // ---- the box: reach 0..=4 ahead, under 2 to the side ----

    #[test]
    fn reach_runs_from_0_to_4_tiles_inclusive_in_every_direction() {
        // (dir, step along the facing axis as (dx, dy) per tile)
        for (dir, ux, uy) in [(0u8, 0.0, -1.0), (1, -1.0, 0.0), (2, 0.0, 1.0), (3, 1.0, 0.0)] {
            let a = at(20.0, 20.0, dir);
            let ok = |t: f32| in_sword_box(&R, a, 20.0 + ux * t, 20.0 + uy * t);
            assert_eq!(ok(0.0), Ok(()), "dir {dir}: 0 tiles counts");
            assert_eq!(ok(2.0), Ok(()));
            assert_eq!(ok(4.0), Ok(()), "dir {dir}: 4 tiles counts");
            assert_eq!(ok(4.5), Err(Reject::OutOfReach), "dir {dir}");
            assert_eq!(ok(-0.5), Err(Reject::OutOfReach), "dir {dir}: behind the attacker");
        }
    }

    #[test]
    fn the_side_limit_is_strictly_under_2_tiles() {
        // Facing east: forward is x, sideways is y.
        let a = at(20.0, 20.0, 3);
        assert_eq!(in_sword_box(&R, a, 22.0, 21.5), Ok(()));
        assert_eq!(in_sword_box(&R, a, 22.0, 18.5), Ok(()), "either side");
        assert_eq!(in_sword_box(&R, a, 22.0, 22.0), Err(Reject::OffLine), "exactly 2 is out");
        assert_eq!(in_sword_box(&R, a, 22.0, 18.0), Err(Reject::OffLine));
        assert_eq!(in_sword_box(&R, a, 22.0, 21.99), Ok(()));
        // Facing north: sideways is x.
        let n = at(20.0, 20.0, 0);
        assert_eq!(in_sword_box(&R, n, 21.5, 17.0), Ok(()));
        assert_eq!(in_sword_box(&R, n, 22.0, 17.0), Err(Reject::OffLine));
    }

    #[test]
    fn reach_is_tested_before_the_side() {
        let a = at(20.0, 20.0, 3);
        assert_eq!(in_sword_box(&R, a, 30.0, 30.0), Err(Reject::OutOfReach));
    }

    // ---- the whole judgement ----

    #[test]
    fn only_sprites_6_to_10_are_a_swing() {
        let v = victim(22.0, 20.0);
        for s in 0..40u8 {
            let r = judge_sword(&R, &swinger(3, sw(s, 3)), &v);
            if (6..=10).contains(&s) {
                assert_eq!(r, Ok(1), "S {s}");
            } else {
                assert_eq!(r, Err(Reject::NotSwinging), "S {s}");
            }
        }
    }

    #[test]
    fn the_direction_is_the_one_in_the_sprite() {
        // The pose says north, the sprite says east: the sprite is what was sent.
        let s = Swinger { sprite: sw(8, 3), ..swinger(0, sw(8, 3)) };
        assert_eq!(judge_sword(&R, &s, &victim(22.0, 20.0)), Ok(1));
        assert_eq!(judge_sword(&R, &s, &victim(18.0, 20.0)), Err(Reject::OutOfReach), "behind an east-facing swing");
        assert_eq!(judge_sword(&R, &s, &victim(20.0, 18.0)), Err(Reject::OffLine), "where a north-facing swing would have hit");
    }

    #[test]
    fn ap_100_cannot_be_hit_by_a_sword_and_99_can() {
        let mut v = victim(22.0, 20.0);
        v.ap = 100;
        assert_eq!(judge_sword(&R, &swinger(3, sw(8, 3)), &v), Err(Reject::Immune));
        v.ap = 99;
        assert_eq!(judge_sword(&R, &swinger(3, sw(8, 3)), &v), Ok(1));
        v.ap = 0;
        assert_eq!(judge_sword(&R, &swinger(3, sw(8, 3)), &v), Ok(1));
    }

    #[test]
    fn a_player_who_cannot_be_hurt_is_refused_first() {
        for (state, why) in [
            (Vulnerability::Dead, Reject::Dead),
            (Vulnerability::Hurt, Reject::AlreadyHurt),
            (Vulnerability::Swimming, Reject::Swimming),
        ] {
            let mut v = victim(22.0, 20.0);
            v.state = state;
            assert_eq!(judge_sword(&R, &swinger(3, sw(8, 3)), &v), Err(why));
        }
    }

    #[test]
    fn the_damage_is_the_attackers_sword_power_in_half_hearts() {
        let v = victim(22.0, 20.0);
        for (power, half) in [(0i8, 0), (1, 1), (2, 2), (4, 4), (20, 20), (-3, -3), (-20, -20)] {
            let s = Swinger { sword_power: Some(power), ..swinger(3, sw(8, 3)) };
            assert_eq!(judge_sword(&R, &s, &v), Ok(half), "power {power}");
        }
        // The profile limit is 20 either way, whatever a server sent.
        assert_eq!(sword_half_hearts(&R, 100), 20);
        assert_eq!(sword_half_hearts(&R, -100), -20);
        let unknown = Swinger { sword_power: None, ..swinger(3, sw(8, 3)) };
        assert_eq!(judge_sword(&R, &unknown, &v), Err(Reject::UnknownPower));
    }

    #[test]
    fn before_203_the_sword_power_is_kept_in_6_bits_and_never_heals() {
        let v = victim(22.0, 20.0);
        for (power, half) in [(0i8, 0), (3, 3), (20, 20), (63, 63), (64, 0), (-1, 63), (-20, 44)] {
            let s = Swinger { sword_power: Some(power), ..swinger(3, sw(8, 3)) };
            assert_eq!(judge_sword(&COMBAT_201, &s, &v), Ok(half), "2.01, power {power}");
            assert_eq!(judge_sword(&COMBAT_200, &s, &v), Ok(half), "2.00, power {power}");
        }
        // From 2.03 the same powers are limited and a negative one heals.
        assert_eq!(sword_half_hearts(&COMBAT_205, -1), -1);
        assert_eq!(sword_half_hearts(&COMBAT_205, 63), 20);
    }

    #[test]
    fn the_200_hurt_packet_skips_the_ap_100_and_no_kill_gates() {
        let mut v = victim(20.0, 20.0);
        v.ap = 100;
        v.no_pk = true;
        assert_eq!(judge_hurtplayer(&COMBAT_200, &hurt(1, 2), Some((20.0, 20.0)), &v), Ok(2));
        assert_eq!(judge_hurtplayer(&COMBAT_201, &hurt(1, 2), Some((20.0, 20.0)), &v), Err(Reject::Immune));
        v.ap = 50;
        assert_eq!(judge_hurtplayer(&COMBAT_201, &hurt(1, 2), Some((20.0, 20.0)), &v), Err(Reject::NoPkZone));
        // 2.00 keeps the hurtable, guild and range gates, and the sword keeps
        // its own AP and no-kill tests.
        let friendly = PacketGates { friendly: true, ..PacketGates::default() };
        assert_eq!(judge_hurtplayer_gated(&COMBAT_200, &hurt(1, 2), Some((20.0, 20.0)), &v, friendly), Err(Reject::Friendly));
        assert_eq!(judge_hurtplayer(&COMBAT_200, &hurt(1, 2), Some((30.5, 20.0)), &v), Err(Reject::TooFar));
        let s = swinger(3, sw(8, 3));
        assert_eq!(judge_sword(&COMBAT_200, &s, &Victim { no_pk: true, ..victim(22.0, 20.0) }), Err(Reject::NoPkZone));
        assert_eq!(judge_sword(&COMBAT_200, &s, &Victim { ap: 100, ..victim(22.0, 20.0) }), Err(Reject::Immune));
    }

    #[test]
    fn the_sword_power_byte_decodes_like_the_client_sends_it() {
        use crate::protocol::sword_power_from_wire as w;
        // 0 to 4: the default swords send their number; 10 to 50: power + 30.
        assert_eq!([w(0), w(1), w(4)], [0, 1, 4]);
        assert_eq!([w(10), w(30), w(31), w(50)], [-20, 0, 1, 20]);
        assert_eq!(w(200), 20, "a byte past 50 is limited like the sender limits it");
    }

    #[test]
    fn a_no_kill_level_a_guild_mate_or_a_hidden_attacker_cannot_hit() {
        let mut v = victim(22.0, 20.0);
        v.no_pk = true;
        assert_eq!(judge_sword(&R, &swinger(3, sw(8, 3)), &v), Err(Reject::NoPkZone));
        let v = victim(22.0, 20.0);
        let friend = Swinger { friendly: true, ..swinger(3, sw(8, 3)) };
        assert_eq!(judge_sword(&R, &friend, &v), Err(Reject::Friendly));
        for bit in [0x02u8, 0x20, 0x22] {
            let s = Swinger { status: Some(bit | 0x10), ..swinger(3, sw(8, 3)) };
            assert_eq!(judge_sword(&R, &s, &v), Err(Reject::AttackerBlocked), "status {bit:#x}");
        }
        // Weapons allowed (0x10), male (0x04), spin (0x40): not blockers.
        let s = Swinger { status: Some(0x54), ..swinger(3, sw(8, 3)) };
        assert_eq!(judge_sword(&R, &s, &v), Ok(1));
        let again = Swinger { repeat: true, ..swinger(3, sw(8, 3)) };
        assert_eq!(judge_sword(&R, &again, &v), Err(Reject::Throttled));
    }

    #[test]
    fn the_guild_tag_is_the_first_bracket_pair_and_must_not_be_empty() {
        assert_eq!(guild_tag("Bob (Red Cross) x"), Some("(Red Cross)"));
        assert_eq!(guild_tag("Bob (a)"), Some("(a)"));
        assert_eq!(guild_tag("Bob ()"), None, "nothing between the brackets");
        assert_eq!(guild_tag("Bob (open"), None);
        assert_eq!(guild_tag("Bob) ("), None);
        assert_eq!(guild_tag("Bob"), None);
        assert!(same_guild("Bob (Red)", "Ann (Red)"));
        assert!(same_guild("Bob (Red)", "(Red) Ann"), "anywhere in the victim's nickname");
        assert!(!same_guild("Bob (Red)", "Ann (red)"), "case matters");
        assert!(!same_guild("Bob", "Ann (Red)"), "no tag on the attacker");
        assert!(!same_guild("Bob (Red)", "Ann"), "the victim's tag does not count");
    }

    // ---- packets ----

    /// A wire GSHORT.
    fn gs(v: u16) -> [u8; 2] {
        [0x20 + (v >> 7) as u8, 0x20 + (v & 127) as u8]
    }

    fn p40(attacker: u16, dx: i32, dy: i32, half: u8, npc: Option<u32>) -> Vec<u8> {
        let mut p = gs(attacker).to_vec();
        p.push((0x20 + (dx + 4) * 16) as u8);
        p.push((0x20 + (dy + 4) * 16) as u8);
        p.push(0x20 + half);
        if let Some(n) = npc {
            p.extend_from_slice(&[0x20 + (n >> 14) as u8, 0x20 + ((n >> 7) & 127) as u8, 0x20 + (n & 127) as u8]);
        }
        p
    }

    #[test]
    fn hurtplayer_decodes_the_layout() {
        let h = parse_hurtplayer(&p40(300, -2, 1, 2, None)).unwrap();
        assert_eq!(h.attacker, 300);
        assert_eq!((h.dx, h.dy, h.half, h.npc), (-2.0, 1.0, 2, None));
        let h = parse_hurtplayer(&p40(7, 3, -4, 1, Some(1234))).unwrap();
        assert_eq!((h.dx, h.dy, h.half, h.npc), (3.0, -4.0, 1, Some(1234)));
        assert_eq!(parse_hurtplayer(&[0x20, 0x21, 0x60, 0x60]), None, "no power byte");
        assert_eq!(parse_hurtplayer(&[]), None);
    }

    #[test]
    fn hitobjects_decodes_the_layout() {
        let mut p = gs(5).to_vec();
        p.extend_from_slice(&[0x20 + 2, 0x20 + 41, 0x20 + 25]);
        let h = parse_hitobjects(&p).unwrap();
        assert_eq!((h.attacker, h.half, h.x, h.y, h.npc), (5, 2, 20.5, 12.5, None));
        p.extend_from_slice(&[0x20, 0x20, 0x20 + 9]);
        assert_eq!(parse_hitobjects(&p).unwrap().npc, Some(9));
    }

    fn hurt(attacker: u16, half: i32) -> HurtPlayer {
        HurtPlayer { attacker, dx: 0.0, dy: 0.0, half, npc: None }
    }

    #[test]
    fn hurtplayer_needs_the_attacker_within_10_tiles() {
        let v = victim(20.0, 20.0);
        let go = |x: f32, y: f32| judge_hurtplayer(&R, &hurt(1, 2), Some((x, y)), &v);
        assert_eq!(go(30.0, 20.0), Ok(2), "10 tiles away counts");
        assert_eq!(go(30.5, 20.0), Err(Reject::TooFar));
        assert_eq!(go(20.0, 9.5), Err(Reject::TooFar));
        assert_eq!(judge_hurtplayer(&R, &hurt(1, 2), None, &v), Err(Reject::TooFar), "attacker not in the level");
    }

    #[test]
    fn hurtplayer_power_is_capped_at_one_heart() {
        let v = victim(20.0, 20.0);
        assert_eq!(judge_hurtplayer(&R, &hurt(1, 9), Some((20.0, 20.0)), &v), Ok(2));
        assert_eq!(judge_hurtplayer(&R, &hurt(1, 1), Some((20.0, 20.0)), &v), Ok(1));
        assert_eq!(judge_hurtplayer(&R, &hurt(1, 0), Some((20.0, 20.0)), &v), Ok(0), "power 0 still hurts: hurt mode and knockback, no hearts");
    }

    #[test]
    fn hurtplayer_has_the_swords_gates_and_refuses_the_unhurtable_and_npcs() {
        let mut v = victim(20.0, 20.0);
        v.ap = 100;
        assert_eq!(judge_hurtplayer(&R, &hurt(1, 2), Some((20.0, 20.0)), &v), Err(Reject::Immune), "AP 100 is immune to packet hits too (2.05 and 2.22 read)");
        v.ap = 99;
        assert_eq!(judge_hurtplayer(&R, &hurt(1, 2), Some((20.0, 20.0)), &v), Ok(2));
        v.state = Vulnerability::Dead;
        assert_eq!(judge_hurtplayer(&R, &hurt(1, 2), Some((20.0, 20.0)), &v), Err(Reject::Dead));
        let from_npc = HurtPlayer { npc: Some(4), ..hurt(1, 2) };
        assert_eq!(judge_hurtplayer(&R, &from_npc, Some((20.0, 20.0)), &victim(20.0, 20.0)), Err(Reject::NpcSource));
    }

    #[test]
    fn the_lab_servers_relayed_hit_carries_ff_ff_ff_and_is_read_as_an_npc_hit() {
        // GServer-v2 relays PLI_HURTPLAYER as id, dx, dy, power and then
        // `ff ff ff` (lab, 2026-10-28). The 2.05 handler reads 6 or more
        // bytes as an NPC hit and drops it when no NPC has that id; Seihai
        // keeps that rule rather than treating the bytes as "no NPC".
        let h = parse_hurtplayer(&[0x20, 0x23, 0x70, 0x60, 0x21, 0xff, 0xff, 0xff]).unwrap();
        assert_eq!((h.attacker, h.dx, h.dy, h.half), (3, 1.0, 0.0, 1));
        assert!(h.npc.is_some());
        assert_eq!(judge_hurtplayer(&R, &h, Some((27.0, 30.0)), &victim(30.0, 30.0)), Err(Reject::NpcSource));
        let clean = parse_hurtplayer(&[0x20, 0x23, 0x70, 0x60, 0x21]).unwrap();
        assert_eq!(judge_hurtplayer(&R, &clean, Some((27.0, 30.0)), &victim(30.0, 30.0)), Ok(1));
    }

    // ---- the attacker's side ----

    fn target(x: f32, y: f32) -> Target {
        Target { id: 300, x, y, status: None, sprite: None }
    }

    #[test]
    fn the_attacker_does_not_send_what_the_victim_would_refuse() {
        let from = (20.0, 20.0);
        assert_eq!(attacker_may_send(&R, from, &target(25.0, 20.0)), Ok(()));
        assert_eq!(attacker_may_send(&R, from, &target(31.0, 20.0)), Err(Reject::TooFar));
        let dead = Target { status: Some(0x04 | STATUS_DEAD), ..target(21.0, 20.0) };
        assert_eq!(attacker_may_send(&R, from, &dead), Err(Reject::Dead));
        let alive = Target { status: Some(0x14), ..target(21.0, 20.0) };
        assert_eq!(attacker_may_send(&R, from, &alive), Ok(()));
        let hurt = Target { sprite: Some(sw(32, 2)), ..target(21.0, 20.0) };
        assert_eq!(attacker_may_send(&R, from, &hurt), Err(Reject::AlreadyHurt));
    }

    #[test]
    fn hurtplayer_message_bytes_and_gating() {
        let v = ClientVersion::V205;
        let m = hurtplayer_message(v, (20.0, 20.0), &target(21.0, 20.0), (0.0, -1.0), 2).unwrap();
        // 26 + 0x20, GSHORT 300, (0+4)*16 = 64, (-1+4)*16 = 48, power 2.
        let mut want = vec![26 + 0x20];
        want.extend_from_slice(&gs(300));
        want.extend_from_slice(&[0x20 + 64, 0x20 + 48, 0x20 + 2, b'\n']);
        assert_eq!(m, want);
        // And it decodes the way the victim reads it.
        let back = parse_hurtplayer(&m[1..m.len() - 1]).unwrap();
        assert_eq!((back.attacker, back.dx, back.dy, back.half), (300, 0.0, -1.0, 2));
        // The cap, the refusals, and a build without the rules.
        let big = hurtplayer_message(v, (20.0, 20.0), &target(21.0, 20.0), (0.0, 0.0), 9).unwrap();
        assert_eq!(big[big.len() - 2], 0x20 + 2);
        assert_eq!(hurtplayer_message(v, (20.0, 20.0), &target(40.0, 20.0), (0.0, 0.0), 2), Err(Reject::TooFar));
        assert_eq!(hurtplayer_message(v, (20.0, 20.0), &target(21.0, 20.0), (0.0, 0.0), 0), Err(Reject::NoDamage));
        for old in [ClientVersion::V133, ClientVersion::V139, ClientVersion::V142] {
            assert_eq!(hurtplayer_message(old, (20.0, 20.0), &target(21.0, 20.0), (0.0, 0.0), 2), Err(Reject::NotInBuild), "{old:?}");
        }
    }

    // ---- profiles ----

    #[test]
    fn combat_follows_the_profile() {
        for p in PROFILES {
            // 2.00 to 2.04 have a traced combat row but no death row yet.
            assert!(p.life.death.is_none() || p.life.combat.is_some(), "{}: a death row comes with a combat row", p.label);
            if let Some(c) = p.life.combat {
                assert!(c.swing_sprite_first <= c.swing_sprite_last, "{}", p.label);
                assert!(c.reach_min_half <= c.reach_max_half, "{}", p.label);
            }
        }
        for v in [ClientVersion::V133, ClientVersion::V139, ClientVersion::V142] {
            assert!(v.combat_rules().is_none(), "{}", v.label());
        }
        // Read in each binary (Graal_analysis `client-internals/combat-200-204.md`).
        assert_eq!(ClientVersion::V200.combat_rules(), Some(COMBAT_200));
        for v in [ClientVersion::V201, ClientVersion::V202, ClientVersion::V202R2] {
            assert_eq!(v.combat_rules(), Some(COMBAT_201), "{}", v.label());
        }
        for v in [ClientVersion::V203, ClientVersion::V204, ClientVersion::V205, ClientVersion::V217, ClientVersion::V222, ClientVersion::V231] {
            assert_eq!(v.combat_rules(), Some(COMBAT_205), "{}", v.label());
        }
        assert_eq!(COMBAT_205.immune_ap, 100);
        assert_eq!((COMBAT_205.reach_min_half, COMBAT_205.reach_max_half, COMBAT_205.lateral_below_half), (0, 8, 4));
    }

    // ---- the session: damage, hurt mode, death ----

    fn session() -> OfflineSession {
        let mut s = OfflineSession::new();
        s.life.set_rules(Some(DEATH_205));
        s.combat.set_rules(Some(COMBAT_205));
        s.player_x = 22.0;
        s.player_y = 20.0;
        s.set_max_hearts_half(12);
        s.set_hearts_half(12);
        s
    }

    #[test]
    fn a_sword_hit_takes_hearts_and_starts_hurt_mode() {
        let mut s = session();
        let r = s.receive_sword(Some(9), &swinger(3, sw(8, 3)), 10);
        assert_eq!(r.outcome, Ok(1));
        assert_eq!(s.hearts_half(), 11);
        assert!(s.combat.is_hurt());
        // Hurt for 40 ticks: the same swing again does nothing.
        let r = s.receive_sword(Some(9), &swinger(3, sw(9, 3)), 20);
        assert_eq!(r.outcome, Err(Reject::AlreadyHurt));
        assert_eq!(s.hearts_half(), 11);
        s.tick_combat(39 * 50);
        assert!(s.combat.is_hurt());
        s.tick_combat(50);
        assert!(!s.combat.is_hurt(), "40 ticks, 2 s");
        (s.player_x, s.player_y) = (22.0, 20.0); // the first hit pushed the player away
        assert_eq!(s.receive_sword(Some(9), &swinger(3, sw(10, 3)), 30).outcome, Ok(1));
        assert_eq!(s.hearts_half(), 10);
    }

    #[test]
    fn a_rejected_hit_changes_nothing() {
        let mut s = session();
        s.alignment = 100;
        assert_eq!(s.receive_sword(Some(9), &swinger(3, sw(8, 3)), 40).outcome, Err(Reject::Immune));
        s.alignment = 50;
        s.player_x = 30.0; // 10 tiles ahead: out of reach
        assert_eq!(s.receive_sword(Some(9), &swinger(3, sw(8, 3)), 50).outcome, Err(Reject::OutOfReach));
        s.player_x = 22.0;
        s.player_y = 23.0; // 3 tiles to the side
        assert_eq!(s.receive_sword(Some(9), &swinger(3, sw(8, 3)), 60).outcome, Err(Reject::OffLine));
        assert_eq!(s.hearts_half(), 12);
        assert!(!s.combat.is_hurt());
    }

    #[test]
    fn swimming_players_cannot_be_hit() {
        let mut s = session();
        s.in_water = true;
        assert_eq!(s.receive_sword(Some(9), &swinger(3, sw(8, 3)), 70).outcome, Err(Reject::Swimming));
        assert_eq!(s.hearts_half(), 12);
    }

    #[test]
    fn the_killing_hit_names_the_attacker_for_the_claim() {
        let mut s = session();
        s.set_hearts_half(1);
        assert_eq!(s.receive_sword(Some(321), &swinger(3, sw(8, 3)), 0).outcome, Ok(1));
        assert_eq!(s.hearts_half(), 0);
        assert_eq!(s.tick_life(50), vec![LifeEvent::Died { attacker: Some(321) }]);
        let msgs = crate::life::messages_for(LifeEvent::Died { attacker: Some(321) }, ClientVersion::V205, 0);
        assert_eq!(msgs, vec![crate::life::claim_message(321)]);
    }

    #[test]
    fn a_dead_player_cannot_be_hit_again() {
        let mut s = session();
        s.set_hearts_half(1);
        s.receive_sword(Some(1), &swinger(3, sw(8, 3)), 0);
        s.tick_life(50);
        assert!(s.life.is_dead());
        s.tick_combat(2_000); // hurt mode over: only death is left to refuse it
        assert!(!s.combat.is_hurt());
        assert_eq!(s.receive_sword(Some(2), &swinger(3, sw(8, 3)), 0).outcome, Err(Reject::Dead));
        assert_eq!(s.receive_hurtplayer(&hurt(2, 2), Some((22.0, 20.0))).outcome, Err(Reject::Dead));
    }

    #[test]
    fn hearts_at_zero_before_the_death_tick_count_as_dead() {
        let mut s = session();
        s.set_hearts_half(0);
        assert_eq!(s.victim().state, Vulnerability::Dead);
    }

    #[test]
    fn a_hurtplayer_packet_hurts_through_the_session() {
        let mut s = session();
        let r = s.receive_hurtplayer(&hurt(4, 5), Some((25.0, 20.0)));
        assert_eq!((r.kind, r.outcome), (HitKind::HurtPlayer, Ok(2)));
        assert_eq!(s.hearts_half(), 10);
        assert!(s.combat.is_hurt());
    }

    #[test]
    fn no_rules_no_hits() {
        let mut s = OfflineSession::new();
        s.life.set_rules(Some(DEATH_205));
        assert_eq!(s.receive_sword(Some(1), &swinger(3, sw(8, 3)), 0).outcome, Err(Reject::NotInBuild));
        assert_eq!(s.receive_hurtplayer(&hurt(1, 2), Some((0.0, 0.0))).outcome, Err(Reject::NotInBuild));
        assert!(!s.start_swing());
        assert_eq!(s.sprite_value(), s.player_dir);
    }

    // ---- the hit's effect on hearts ----

    #[test]
    fn a_sword_hit_takes_the_power_in_half_hearts() {
        for (power, left) in [(1i8, 11), (3, 9), (20, 0)] {
            let mut s = session();
            let r = s.receive_sword(Some(9), &Swinger { sword_power: Some(power), ..swinger(3, sw(8, 3)) }, 0);
            assert_eq!(r.outcome, Ok(i32::from(power)));
            assert_eq!(s.hearts_half(), left, "power {power}");
        }
    }

    #[test]
    fn a_power_0_sword_hurts_without_taking_hearts() {
        let mut s = session();
        let r = s.receive_sword(Some(9), &Swinger { sword_power: Some(0), ..swinger(3, sw(8, 3)) }, 0);
        assert_eq!(r.outcome, Ok(0));
        assert_eq!(s.hearts_half(), 12);
        assert!(s.combat.is_hurt(), "hurt mode starts anyway");
    }

    #[test]
    fn a_negative_sword_heals_only_from_ap_40() {
        let heal = Swinger { sword_power: Some(-3), ..swinger(3, sw(8, 3)) };
        let mut s = session();
        s.set_hearts_half(4);
        s.alignment = 40;
        assert_eq!(s.receive_sword(Some(9), &heal, 0).outcome, Ok(-3));
        assert_eq!(s.hearts_half(), 7);
        assert!(s.combat.is_hurt());
        // Healing stops at the maximum.
        let mut s = session();
        s.set_hearts_half(11);
        s.alignment = 90;
        assert_eq!(s.receive_sword(Some(9), &heal, 0).outcome, Ok(-3));
        assert_eq!(s.hearts_half(), 12);
        // Below 40 nothing changes but hurt mode still starts.
        let mut s = session();
        s.set_hearts_half(4);
        s.alignment = 39;
        assert_eq!(s.receive_sword(Some(9), &heal, 0).outcome, Ok(0));
        assert_eq!(s.hearts_half(), 4);
        assert!(s.combat.is_hurt());
    }

    #[test]
    fn the_same_attacker_is_judged_once_a_second() {
        let mut s = session();
        let a = swinger(3, sw(8, 3));
        assert_eq!(s.receive_sword(Some(9), &a, 100).outcome, Ok(1));
        // Same second: dropped by the throttle (and the victim is hurt anyway,
        // which is checked first, so end hurt mode to see the throttle).
        s.combat.set_rules(Some(COMBAT_205));
        assert_eq!(s.receive_sword(Some(9), &a, 100).outcome, Err(Reject::Throttled));
        // Another attacker, or the next second, goes through.
        assert_eq!(s.receive_sword(Some(8), &a, 100).outcome, Ok(1));
        s.combat.set_rules(Some(COMBAT_205));
        assert_eq!(s.receive_sword(Some(9), &a, 101).outcome, Ok(1));
    }

    #[test]
    fn a_refused_swing_does_not_stamp_the_attacker() {
        let mut s = session();
        // Out of reach is decided after the stamp, so the stamp is set ...
        s.player_x = 40.0;
        let a = swinger(3, sw(8, 3));
        assert_eq!(s.receive_sword(Some(9), &a, 5).outcome, Err(Reject::OutOfReach));
        s.player_x = 22.0;
        assert_eq!(s.receive_sword(Some(9), &a, 5).outcome, Err(Reject::Throttled));
        // ... but a swing the client never judges (not a swing frame) does not stamp.
        let mut s = session();
        assert_eq!(s.receive_sword(Some(7), &swinger(3, sw(2, 3)), 5).outcome, Err(Reject::NotSwinging));
        assert_eq!(s.receive_sword(Some(7), &a, 5).outcome, Ok(1));
    }

    #[test]
    fn no_player_killing_stops_sword_and_packet_hits() {
        let mut s = session();
        s.no_pk = true;
        assert_eq!(s.receive_sword(Some(9), &swinger(3, sw(8, 3)), 0).outcome, Err(Reject::NoPkZone));
        assert_eq!(s.receive_hurtplayer(&hurt(9, 1), Some((21.0, 20.0))).outcome, Err(Reject::NoPkZone));
        assert_eq!(s.hearts_half(), 12);
        s.no_pk = false;
        assert_eq!(s.receive_hurtplayer(&hurt(9, 1), Some((21.0, 20.0))).outcome, Ok(1));
    }

    #[test]
    fn receive_all_applies_the_guild_and_status_rules() {
        let mut s = session();
        let mut a = other(10, 20.0, 20.0, 3, sw(7, 3), Some(2));
        a.nick = Some("Bob (Red)".into());
        assert_eq!(s.receive_all(&[a.clone()], None, Some("Ann (Red)"), 0, &[10], &[])[0].outcome, Err(Reject::Friendly));
        assert_eq!(s.receive_all(&[a.clone()], None, Some("Ann (Blue)"), 1, &[10], &[])[0].outcome, Ok(2));
        let mut s = session();
        a.status = Some(0x12);
        assert_eq!(s.receive_all(&[a], None, None, 0, &[10], &[])[0].outcome, Err(Reject::AttackerBlocked));
    }

    // ---- receive_all over real players ----

    fn other(id: u16, x: f32, y: f32, dir: u8, sprite: u8, power: Option<i8>) -> OtherPlayer {
        OtherPlayer {
            id,
            x,
            y,
            dir,
            nick: None,
            chat: None,
            chat_timer: Default::default(),
            account: None,
            cell: None,
            sprite: Some(sprite),
            status: None,
            ap: None,
            sword_power: power,
            carry: None,
            looks: Default::default(),
        }
    }

    #[test]
    fn receive_all_judges_swings_and_packets_in_order() {
        let mut s = session();
        let others = [other(10, 20.0, 20.0, 3, sw(7, 3), Some(2)), other(11, 22.0, 24.0, 0, sw(7, 0), Some(1))];
        let hits = [IncomingHit::Hurt(hurt(11, 2)), IncomingHit::Objects(HitObjects { attacker: 11, half: 2, x: 22.0, y: 20.0, npc: None })];
        let out = s.receive_all(&others, None, None, 0, &[10, 11, 99], &hits);
        let kinds: Vec<_> = out.iter().map(|r| (r.kind, r.attacker, r.outcome)).collect();
        assert_eq!(
            kinds,
            vec![
                (HitKind::Sword, Some(10), Ok(2)),
                (HitKind::Sword, Some(11), Err(Reject::AlreadyHurt)),
                (HitKind::HurtPlayer, Some(11), Err(Reject::AlreadyHurt)),
                (HitKind::HitObjects, Some(11), Err(Reject::AlreadyHurt)),
            ],
            "an unknown id (99) is skipped"
        );
    }

    #[test]
    fn a_player_in_another_gmap_cell_is_placed_relative_to_us() {
        // We stand at x 2 of cell (1, 0). The attacker is at x 62 of cell
        // (0, 0), facing east: that is x -2 in our cell, 4 tiles behind us
        // along its facing, so the hit lands (the edge of the reach).
        let mut s = session();
        s.player_x = 2.0;
        let mut o = other(10, 62.0, 20.0, 3, sw(7, 3), Some(1));
        o.cell = Some((0, 0));
        assert_eq!(s.receive_all(&[o.clone()], Some((1, 0)), None, 0, &[10], &[])[0].outcome, Ok(1));
        // Without the cell offset the same numbers are 60 tiles apart.
        let mut s = session();
        s.player_x = 2.0;
        assert_eq!(s.receive_all(&[o], None, None, 0, &[10], &[])[0].outcome, Err(Reject::OutOfReach));
    }

    // ---- our own swing ----

    #[test]
    fn our_swing_sends_frames_6_to_10_then_goes_idle() {
        let mut s = session();
        s.player_dir = 3;
        assert_eq!(s.sprite_value(), 3);
        assert!(s.start_swing());
        assert_eq!(s.sprite_value(), sw(6, 3));
        assert!(!s.start_swing(), "one swing at a time");
        let mut seen = vec![s.sprite_value()];
        // swing_step_ticks (2) of 50 ms per frame.
        for _ in 0..20 {
            if s.tick_combat(50) {
                seen.push(s.sprite_value());
            }
        }
        assert_eq!(seen, vec![sw(6, 3), sw(7, 3), sw(8, 3), sw(9, 3), sw(10, 3), 3]);
        assert!(!s.combat.is_swinging());
        assert!(s.start_swing(), "and again once it is over");
    }

    #[test]
    fn a_20x_build_swings_with_sword_in_the_bow_slot_and_a_plain_sprite() {
        // The real 2.00 and 2.05 (lab decodes, 2026-10-30): prop 10
        // `c(10 + 5)` + `sword`, then `idle`; SPRITE stays the direction.
        for v in [ClientVersion::V200, ClientVersion::V201, ClientVersion::V204, ClientVersion::V205] {
            assert!(swing_in_bow_slot(v), "{}", v.label());
        }
        for v in [ClientVersion::V142, ClientVersion::V139, ClientVersion::V210, ClientVersion::V217, ClientVersion::V222] {
            assert!(!swing_in_bow_slot(v), "{}", v.label());
        }
        assert_eq!(crate::net::bow_slot_message("sword"), [&[2 + 0x20, 10 + 0x20, 0x20 + 15][..], b"sword", b"\n"].concat());
        assert_eq!(crate::net::bow_slot_message("idle"), [&[2 + 0x20, 10 + 0x20, 0x20 + 14][..], b"idle", b"\n"].concat());
        let mut s = session();
        s.player_dir = 1;
        assert_eq!((s.bow_slot_anim(), s.bow_slot_sprite(true)), ("idle", 1));
        assert!(s.start_swing());
        assert_eq!((s.bow_slot_anim(), s.bow_slot_sprite(true)), ("sword", 1));
        for _ in 0..20 {
            s.tick_combat(50);
        }
        assert_eq!(s.bow_slot_anim(), "idle");
        // Hurt mode still goes out in SPRITE: 128 + direction, as the real ones send it.
        s.combat.start_hurt();
        assert_eq!(s.bow_slot_sprite(true), 128 + 1);
    }

    #[test]
    fn a_20x_build_names_the_lift_carry_and_hidden_poses_in_the_bow_slot() {
        // A real 2.05 watcher draws the carried object only for these gani
        // names (or `def`), and nothing at all of a hidden player on `idle`.
        let mut s = session();
        s.current_level = Some(crate::assets::Level::from_board(&[0u16; 64 * 64]));
        s.tiletypes = Some(crate::assets::TileTypes::from_types([0; 4096]));
        s.carried = Some(crate::offline::Carried { kind: 0, tiles: [[0, 0], [0, 0]] });
        s.lift_ticks = Some(0);
        assert_eq!(s.bow_slot_anim(), "lift");
        s.lift_ticks = None;
        assert_eq!(s.bow_slot_anim(), "carrystill");
        s.walking = true;
        assert_eq!(s.bow_slot_anim(), "carry");
        s.walking = false;
        assert!(s.toggle_hide());
        assert_eq!(s.bow_slot_anim(), "hiddenstill");
        s.walking = true;
        assert_eq!(s.bow_slot_anim(), "hidden");
        s.walking = false;
        assert!(s.toggle_hide());
        assert_eq!(s.bow_slot_anim(), "carrystill", "out of hiding, back to carrying");
        assert!(s.throw_carried().is_some());
        assert_eq!(s.bow_slot_anim(), "idle");
    }

    #[test]
    fn the_swing_needs_a_sword_and_a_live_dry_player() {
        let mut s = session();
        s.sword_power = 0;
        assert!(!s.start_swing());
        s.sword_power = 1;
        s.in_water = true;
        assert!(!s.start_swing());
        s.in_water = false;
        s.set_hearts_half(1);
        s.damage_half(1, None);
        s.tick_life(0);
        assert!(!s.start_swing(), "dead");
    }

    #[test]
    fn no_swing_while_a_sign_is_open() {
        // 2.05: S turns the sign's page and does not reach the sword.
        let mut s = session();
        s.reading = Some("Hello".into());
        assert!(!s.start_swing());
        assert!(!s.combat.is_swinging());
        s.reading = None;
        assert!(s.start_swing());
    }

    #[test]
    fn dying_ends_the_swing_and_hurt_mode_shows_the_hurt_sprite() {
        let mut s = session();
        s.player_dir = 2;
        assert!(s.start_swing());
        s.damage_half(12, Some(1));
        s.tick_life(0);
        assert!(s.tick_combat(0), "the sprite changed");
        assert!(!s.combat.is_swinging());
        let mut h = session();
        h.player_dir = 1;
        h.receive_sword(Some(2), &swinger(3, sw(8, 3)), 0);
        assert_eq!(h.sprite_value(), 1 + 4 * 32);
        assert!(h.tick_combat(2_000), "hurt mode ends, the sprite goes back");
        assert_eq!(h.sprite_value(), 1);
    }

    #[test]
    fn walk_message_carries_the_sprite() {
        assert_eq!(crate::net::position_message(10.5, 3.0, 2), crate::net::position_message_sprite(10.5, 3.0, 2));
        let m = crate::net::position_message_sprite(10.5, 3.0, sw(8, 3));
        assert_eq!(m[5], 17 + 0x20);
        assert_eq!(m[6], 0x20 + 35);
    }

    // ---- the session state reading the wire ----

    fn plo8(id: u16, props: &[u8]) -> Vec<u8> {
        let mut m = vec![8 + 0x20];
        m.extend_from_slice(&gs(id));
        m.extend_from_slice(props);
        m.push(b'\n');
        m
    }

    #[test]
    fn other_props_feed_the_victims_side() {
        let mut st = SessionState::for_version(ClientVersion::V205);
        // SWORDPOWER (8) = 2 with image "s.png", X 40 (20 tiles), Y 40,
        // SPRITE swing frame 8 facing east, STATUS 0x14, ALIGNMENT 77.
        let mut props = vec![8 + 0x20, 0x20 + 32, 0x20 + 5];
        props.extend_from_slice(b"s.png");
        props.extend_from_slice(&[15 + 0x20, 0x20 + 40, 16 + 0x20, 0x20 + 40, 17 + 0x20, 0x20 + sw(8, 3)]);
        props.extend_from_slice(&[18 + 0x20, 0x20 + 0x14, 32 + 0x20, 0x20 + 77]);
        st.apply_body(&plo8(300, &props));
        let o = &st.others[0];
        assert_eq!((o.sword_power, o.sprite, o.status, o.ap), (Some(2), Some(sw(8, 3)), Some(0x14), Some(77)));
        assert_eq!((o.x, o.y, o.dir), (20.0, 20.0, 3));
        assert_eq!(st.swings, vec![300]);
        // An idle update later keeps what it knew and adds no swing.
        st.swings.clear();
        st.apply_body(&plo8(300, &[17 + 0x20, 0x20 + 3]));
        assert!(st.swings.is_empty());
        assert_eq!(st.others[0].sword_power, Some(2));
        assert_eq!(st.others[0].sprite, Some(3));
    }

    // ---- 2.1+ swings: the GANI prop ----

    #[test]
    fn the_swing_ganis_are_sword_ridesword_and_spin() {
        for g in ["sword", "ridesword", "spin", "sword.gani", "sword.gif"] {
            assert!(is_swing_gani(g), "{g}");
        }
        for g in ["idle", "walk", "kick", "hurt", "swordx", "Sword", ""] {
            assert!(!is_swing_gani(g), "{g}");
        }
    }

    #[test]
    fn a_swing_gani_is_a_swing_whatever_the_sprite() {
        // A GANI client's SPRITE is the direction alone (3, east).
        let s = Swinger { swing_gani: true, ..swinger(3, 3) };
        assert_eq!(judge_sword(&R, &s, &victim(22.0, 20.0)), Ok(1));
        assert_eq!(judge_sword(&R, &swinger(3, 3), &victim(22.0, 20.0)), Err(Reject::NotSwinging));
        // The direction still comes from the sprite: facing west, the victim is behind.
        let west = Swinger { swing_gani: true, ..swinger(1, 1) };
        assert_eq!(judge_sword(&R, &west, &victim(22.0, 20.0)), Err(Reject::OutOfReach));
    }

    /// GANI (10) as a 2.1+ string prop.
    fn gani_prop(name: &str) -> Vec<u8> {
        let mut p = vec![10 + 0x20, 0x20 + name.len() as u8];
        p.extend_from_slice(name.as_bytes());
        p
    }

    #[test]
    fn a_gani_clients_sword_gani_is_a_swing() {
        for v in [ClientVersion::V217, ClientVersion::V222, ClientVersion::V231] {
            let mut st = SessionState::for_version(v);
            // SWORDPOWER 1, X 40, Y 40 (20 tiles), SPRITE 3 (east), GANI sword.
            let mut props = vec![8 + 0x20, 0x20 + 1, 15 + 0x20, 0x20 + 40, 16 + 0x20, 0x20 + 40, 17 + 0x20, 0x20 + 3];
            props.extend(gani_prop("sword"));
            st.apply_body(&plo8(300, &props));
            assert_eq!(st.others[0].looks.gani.as_deref(), Some("sword"), "{}", v.label());
            assert_eq!(st.swings, vec![300], "{}", v.label());
            // A step while the GANI is still sword is judged again.
            st.swings.clear();
            st.apply_body(&plo8(300, &[15 + 0x20, 0x20 + 41]));
            assert_eq!(st.swings, vec![300], "{}: a step during the swing", v.label());
            // Back to idle: no swing.
            st.swings.clear();
            st.apply_body(&plo8(300, &gani_prop("idle")));
            st.apply_body(&plo8(300, &[15 + 0x20, 0x20 + 42]));
            assert!(st.swings.is_empty(), "{}", v.label());
            // A GANI that is not a swing is none either.
            st.apply_body(&plo8(300, &gani_prop("walk")));
            assert!(st.swings.is_empty(), "{}", v.label());
        }
    }

    #[test]
    fn a_sword_gani_from_the_wire_hurts_through_receive_all() {
        let mut st = SessionState::for_version(ClientVersion::V222);
        let mut props = vec![8 + 0x20, 0x20 + 2, 15 + 0x20, 0x20 + 40, 16 + 0x20, 0x20 + 40, 17 + 0x20, 0x20 + 3];
        props.extend(gani_prop("sword"));
        st.apply_body(&plo8(300, &props));
        let swings = std::mem::take(&mut st.swings);
        let mut s = session();
        let out = s.receive_all(&st.others, None, None, 0, &swings, &[]);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].kind, out[0].attacker, out[0].outcome), (HitKind::Sword, Some(300), Ok(2)));
    }

    #[test]
    fn a_sword_gani_with_no_sprite_ever_sent_faces_the_players_direction() {
        let mut s = session();
        let mut o = other(10, 20.0, 20.0, 3, 0, Some(1));
        o.sprite = None;
        assert!(s.receive_all(&[o.clone()], None, None, 0, &[10], &[]).is_empty(), "no sprite, no swing GANI: nothing to judge");
        o.looks.gani = Some("sword".into());
        assert_eq!(s.receive_all(&[o], None, None, 0, &[10], &[])[0].outcome, Ok(1));
    }

    #[test]
    fn a_sprite_swing_relayed_by_gserver_is_seen_by_every_profile() {
        for v in [ClientVersion::V205, ClientVersion::V217, ClientVersion::V222, ClientVersion::V231] {
            let mut st = SessionState::for_version(v);
            st.apply_body(&plo8(300, &[8 + 0x20, 0x20 + 31, 0x20 + 10, b's', b'w', b'o', b'r', b'd', b'1', b'.', b'g', b'i', b'f']));
            // A swing sprite (as Seihai's own 2.05 swing sends it): GServer-v2
            // relays X, Y, SPRITE, then adds X2, Y2 for 2.30+.
            let x2 = gs((20 * 16) << 1);
            let props = [15 + 0x20, 0x20 + 40, 16 + 0x20, 0x20 + 40, 17 + 0x20, 0x20 + sw(6, 3), 78 + 0x20, x2[0], x2[1], 79 + 0x20, x2[0], x2[1]];
            st.apply_body(&plo8(300, &props));
            // GServer-v2 re-encodes the sword as power + 30 and the image.
            assert_eq!(st.swings, vec![300], "{}", v.label());
            let o = &st.others[0];
            assert_eq!((o.x, o.y, o.sprite, o.sword_power), (20.0, 20.0, Some(sw(6, 3)), Some(1)), "{}", v.label());
        }
    }

    /// Prop 10 in the pre-2.1 bow form: `c(10 + len)` + name.
    fn bow_slot(name: &str) -> Vec<u8> {
        let mut p = vec![10 + 0x20, 0x20 + 10 + name.len() as u8];
        p.extend_from_slice(name.as_bytes());
        p
    }

    #[test]
    fn a_205_swing_is_the_sword_name_in_the_bow_slot() {
        // The real 2.05 client (2026-10-30 lab decode) sends its swing as
        // `sword` in prop 10 and keeps SPRITE on the direction alone.
        // GServer-v2 relays it as a bow image with `.gif` added.
        let mut st = SessionState::for_version(ClientVersion::V205);
        let mut props = vec![8 + 0x20, 0x20 + 32, 0x20 + 0, 15 + 0x20, 0x20 + 40, 16 + 0x20, 0x20 + 40, 17 + 0x20, 0x20 + 3];
        props.extend(bow_slot("walk.gif"));
        st.apply_body(&plo8(300, &props));
        assert!(st.swings.is_empty(), "walk is no swing");
        // The swing comes alone, with no position or sprite.
        st.apply_body(&plo8(300, &bow_slot("sword.gif")));
        assert_eq!(st.swings, vec![300]);
        assert_eq!(st.others[0].looks.gani, None, "a 2.05 player is still drawn from its sprite");
        let swings = std::mem::take(&mut st.swings);
        let mut s = session();
        let out = s.receive_all(&st.others, None, None, 0, &swings, &[]);
        assert_eq!((out[0].kind, out[0].attacker, out[0].outcome), (HitKind::Sword, Some(300), Ok(2)));
        // A bow power under 10 names nothing, and later updates are no swing.
        st.apply_body(&plo8(300, &[10 + 0x20, 0x20 + 3]));
        assert_eq!(st.others[0].looks.bow_slot_gani.as_deref(), Some("sword.gif"), "a power keeps the last name");
        st.apply_body(&plo8(300, &bow_slot("idle.gif")));
        st.apply_body(&plo8(300, &[15 + 0x20, 0x20 + 41]));
        assert!(st.swings.is_empty());
    }

    #[test]
    fn a_2x_server_relays_a_20x_swing_as_a_gani_string() {
        // GServer-v2 beta4 (generation newmain) reads the real 2.00's
        // `c(15)` + `sword` as a GANI and sends `c(5)` + `sword` to everyone,
        // a 2.0x login included. The 2.00 swing carries no position.
        for v in [ClientVersion::V200, ClientVersion::V205] {
            let mut st = SessionState::for_version(v);
            st.combat = Some(COMBAT_205);
            let mut props = vec![8 + 0x20, 0x20 + 32, 0x20 + 0, 15 + 0x20, 0x20 + 40, 16 + 0x20, 0x20 + 40, 17 + 0x20, 0x20 + 3];
            props.extend(gani_prop("idle"));
            st.apply_body(&plo8(300, &props));
            assert!(st.swings.is_empty(), "{}", v.label());
            assert_eq!((st.others[0].x, st.others[0].sprite), (20.0, Some(3)), "{}: the props after the GANI are read", v.label());
            st.apply_body(&plo8(300, &gani_prop("sword")));
            assert_eq!(st.swings, vec![300], "{}", v.label());
            let swings = std::mem::take(&mut st.swings);
            let mut s = session();
            assert_eq!(s.receive_all(&st.others, None, None, 0, &swings, &[])[0].outcome, Ok(2), "{}", v.label());
        }
    }

    #[test]
    fn packet_hits_are_decoded_by_profile() {
        let m40 = |id: u16| {
            let mut m = vec![40 + 0x20];
            m.extend_from_slice(&p40(id, 1, 0, 2, None));
            m.push(b'\n');
            m
        };
        let mut st = SessionState::for_version(ClientVersion::V205);
        st.apply_body(&m40(12));
        let mut m46 = vec![46 + 0x20];
        m46.extend_from_slice(&gs(12));
        m46.extend_from_slice(&[0x20 + 2, 0x20 + 40, 0x20 + 40, b'\n']);
        st.apply_body(&m46);
        assert!(matches!(st.hits[0], IncomingHit::Hurt(h) if h.attacker == 12 && h.half == 2));
        assert!(matches!(st.hits[1], IncomingHit::Objects(h) if h.attacker == 12 && h.x == 20.0));
        // A build without the rules decodes nothing and counts the ids as unhandled.
        for v in [ClientVersion::V133, ClientVersion::V139, ClientVersion::V142] {
            let mut st = SessionState::for_version(v);
            st.apply_body(&m40(12));
            st.apply_body(&m46);
            assert!(st.hits.is_empty(), "{}", v.label());
            let mut sw_props = vec![17 + 0x20, 0x20 + sw(8, 3)];
            sw_props.extend_from_slice(&[15 + 0x20, 0x20 + 40]);
            st.apply_body(&plo8(5, &sw_props));
            assert!(st.swings.is_empty(), "{}: no swing is noticed", v.label());
        }
        // 2.00 has the rules but no id 46 (added in 2.01): 40 is decoded, 46
        // is not.
        let mut st = SessionState::for_version(ClientVersion::V200);
        st.apply_body(&m40(12));
        st.apply_body(&m46);
        assert_eq!(st.hits.len(), 1);
        assert!(matches!(st.hits[0], IncomingHit::Hurt(h) if h.attacker == 12));
        let mut st = SessionState::for_version(ClientVersion::V201);
        st.apply_body(&m40(12));
        st.apply_body(&m46);
        assert_eq!(st.hits.len(), 2);
    }

    // ---- packet gates, id 46 and knockback (damage-respawn leftovers) ----

    fn hits(x: f32, y: f32, half: i32) -> HitObjects {
        HitObjects { attacker: 1, half, x, y, npc: None }
    }

    #[test]
    fn packet_hits_share_the_swords_once_a_second_and_guild_gates() {
        let mut s = open_session(22.0, 20.0);
        let at = Some((24.0, 20.0));
        // Hurt mode over, and the player back where the attacker aims.
        let reset = |s: &mut OfflineSession| {
            s.tick_combat(2_000);
            (s.player_x, s.player_y) = (22.0, 20.0);
        };
        assert_eq!(s.receive_hurtplayer_gated(&hurt(7, 1), at, false, Some(40)).outcome, Ok(1));
        reset(&mut s);
        // Same attacker, same second of the day: dropped (the stamp is per attacker).
        assert_eq!(s.receive_hurtplayer_gated(&hurt(7, 1), at, false, Some(40)).outcome, Err(Reject::Throttled));
        assert_eq!(s.receive_hurtplayer_gated(&hurt(7, 1), at, false, Some(41)).outcome, Ok(1), "the next second counts");
        reset(&mut s);
        // The stamp is shared with the sword: a swing judged in second 42 stamps the attacker too.
        let swing = Swinger { pose: at_pose(24.0, 20.0, 1), ..swinger(1, sw(8, 1)) };
        assert_eq!(s.receive_sword(Some(7), &swing, 42).outcome, Ok(1));
        reset(&mut s);
        assert_eq!(s.receive_hurtplayer_gated(&hurt(7, 1), at, false, Some(42)).outcome, Err(Reject::Throttled));
        // The guild tag refuses it, after the stamp.
        assert_eq!(s.receive_hurtplayer_gated(&hurt(8, 1), at, true, Some(50)).outcome, Err(Reject::Friendly));
        assert_eq!(s.receive_hurtplayer_gated(&hurt(8, 1), at, false, Some(50)).outcome, Err(Reject::Throttled), "stamped even though the guild test refused it");
    }

    fn at_pose(x: f32, y: f32, dir: u8) -> Pose {
        Pose { x, y, dir }
    }

    #[test]
    fn a_level_flagged_no_player_killing_does_not_stamp_the_attacker() {
        let mut s = session();
        s.no_pk = true;
        assert_eq!(s.receive_hurtplayer_gated(&hurt(3, 1), Some((24.0, 20.0)), false, Some(5)).outcome, Err(Reject::NoPkZone));
        s.no_pk = false;
        assert_eq!(s.receive_hurtplayer_gated(&hurt(3, 1), Some((24.0, 20.0)), false, Some(5)).outcome, Ok(1));
    }

    #[test]
    fn hitobjects_hurts_a_player_whose_centre_is_under_2_tiles_from_the_point() {
        let rules = R;
        let v = victim(20.0, 20.0); // centre (21.5, 22.0)
        let g = PacketGates::default();
        let at = Some((20.0, 20.0));
        let ok = |x: f32, y: f32| judge_hitobjects(&rules, &hits(x, y, 2), at, &v, g).map(|(h, _)| h);
        assert_eq!(ok(21.5, 22.0), Ok(2), "on the centre");
        assert_eq!(ok(23.0, 23.5), Ok(2), "1.5 tiles off on each axis");
        assert_eq!(ok(23.5, 22.0), Err(Reject::OutOfBlast), "exactly 2 tiles in x is out");
        assert_eq!(ok(21.5, 20.0), Err(Reject::OutOfBlast), "exactly 2 tiles in y is out");
        assert_eq!(ok(21.5, 20.5), Ok(2));
        // The X/Y props themselves are not the centre: a point on them is 1.5 and 2.0 tiles away.
        assert_eq!(ok(20.0, 20.0), Err(Reject::OutOfBlast));
        // Power is capped at one heart, and 0 is accepted.
        assert_eq!(judge_hitobjects(&rules, &hits(21.5, 22.0, 9), at, &v, g).map(|(h, _)| h), Ok(2));
        assert_eq!(judge_hitobjects(&rules, &hits(21.5, 22.0, 0), at, &v, g).map(|(h, _)| h), Ok(0));
        // The knockback runs from the point to the centre.
        assert_eq!(judge_hitobjects(&rules, &hits(21.0, 21.0, 1), at, &v, g).map(|(_, k)| k), Ok((0.5, 1.0)));
        // Same gates as id 40: range, AP, NPC source.
        assert_eq!(judge_hitobjects(&rules, &hits(21.5, 22.0, 1), Some((31.0, 20.0)), &v, g), Err(Reject::TooFar));
        assert_eq!(judge_hitobjects(&rules, &HitObjects { npc: Some(2), ..hits(21.5, 22.0, 1) }, at, &v, g), Err(Reject::NpcSource));
        let mut immune = v;
        immune.ap = 100;
        assert_eq!(judge_hitobjects(&rules, &hits(21.5, 22.0, 1), at, &immune, g), Err(Reject::Immune));
    }

    #[test]
    fn hitobjects_through_the_session_hurts_and_pushes_away_from_the_point() {
        let mut s = session(); // player at (22, 20), centre (23.5, 22.0)
        let r = s.receive_hitobjects(&hits(22.5, 22.0, 1), Some((22.0, 20.0)));
        assert_eq!((r.kind, r.outcome), (HitKind::HitObjects, Ok(1)));
        assert_eq!(s.hearts_half(), 11);
        s.tick_combat(50);
        assert_eq!((s.player_x, s.player_y), (23.0, 20.0), "pushed one tile east, away from a point to the west");
    }

    #[test]
    fn knockback_scales_to_one_tile_and_a_zero_vector_goes_east() {
        assert_eq!(knockback_unit(0.0, 0.0), (1.0, 0.0));
        assert_eq!(knockback_unit(3.0, 0.0), (1.0, 0.0));
        assert_eq!(knockback_unit(0.0, -2.0), (0.0, -1.0));
        let (x, y) = knockback_unit(3.0, 4.0);
        assert!((x - 0.6).abs() < 1e-6 && (y - 0.8).abs() < 1e-6);
    }

    fn open_session(x: f32, y: f32) -> OfflineSession {
        use crate::assets::{Level, TileTypes};
        let mut s = session();
        let tiles: Vec<u16> = vec![0; 64 * 64];
        s.current_level = Some(Level::from_board(&tiles));
        s.tiletypes = Some(TileTypes::from_types([0; 4096]));
        s.player_x = x;
        s.player_y = y;
        s
    }

    #[test]
    fn a_hit_pushes_one_tile_a_tick_for_five_ticks_and_then_stops() {
        let mut s = open_session(30.0, 30.0);
        let h = HurtPlayer { dx: 2.0, dy: 0.0, ..hurt(1, 2) };
        assert_eq!(s.receive_hurtplayer(&h, Some((28.0, 30.0))).outcome, Ok(2));
        assert_eq!(s.take_knock_moved(), false, "nothing has moved before the first tick");
        let mut seen = vec![];
        for _ in 0..8 {
            s.tick_combat(50);
            seen.push(s.player_x);
        }
        assert_eq!(seen, vec![31.0, 32.0, 33.0, 34.0, 35.0, 35.0, 35.0, 35.0], "five tiles in 250 ms");
        assert!(s.take_knock_moved(), "the caller is told to send the position");
        assert!(!s.take_knock_moved());
        assert!(s.combat.is_hurt(), "hurt mode runs on for 40 ticks");
        assert_eq!(s.player_y, 30.0);
    }

    #[test]
    fn knockback_steps_are_counted_in_whole_ticks_not_in_calls() {
        let mut s = open_session(30.0, 30.0);
        s.receive_hurtplayer(&HurtPlayer { dx: 0.0, dy: 3.0, ..hurt(1, 2) }, Some((30.0, 28.0)));
        s.tick_combat(20);
        s.tick_combat(20);
        assert_eq!(s.player_y, 30.0, "40 ms is not a tick yet");
        s.tick_combat(20);
        assert_eq!(s.player_y, 31.0);
        s.tick_combat(1_000);
        assert_eq!(s.player_y, 35.0, "one big step of time still pushes only five tiles");
    }

    #[test]
    fn a_wall_in_the_way_takes_the_step_back_and_the_push_goes_on_next_tick() {
        use crate::assets::{Level, TileTypes};
        let mut s = open_session(30.0, 30.0);
        // A wall column at tile x = 34 (types[1] = 22 is a wall).
        let tiles: Vec<u16> = (0..64 * 64).map(|i| if i % 64 == 34 { 1 } else { 0 }).collect();
        s.current_level = Some(Level::from_board(&tiles));
        let mut t = TileTypes::from_types([0; 4096]);
        t.types[1] = 22;
        s.tiletypes = Some(t);
        s.receive_hurtplayer(&HurtPlayer { dx: 1.0, dy: 0.0, ..hurt(1, 2) }, Some((28.0, 30.0)));
        let mut xs = vec![];
        for _ in 0..5 {
            s.tick_combat(50);
            xs.push(s.player_x);
        }
        // The body box reaches 2.4 tiles right of X: at X = 32 its right edge is at 34.4, in the wall.
        assert_eq!(xs, vec![31.0, 31.0, 31.0, 31.0, 31.0]);
        assert!(s.take_knock_moved(), "the first step did move");
    }

    #[test]
    fn a_sword_hit_pushes_away_from_the_attacker() {
        let mut s = open_session(30.0, 30.0);
        // Attacker west of the victim, facing east.
        let sw = Swinger { pose: Pose { x: 28.0, y: 30.0, dir: 3 }, ..swinger(3, sw(8, 3)) };
        assert_eq!(s.receive_sword(Some(5), &sw, 1).outcome, Ok(1));
        s.tick_combat(50);
        assert_eq!((s.player_x, s.player_y), (31.0, 30.0));
    }

    #[test]
    fn knockback_does_nothing_without_a_hit_or_after_death() {
        let mut s = open_session(30.0, 30.0);
        s.tick_combat(500);
        assert_eq!((s.player_x, s.player_y), (30.0, 30.0));
        s.set_hearts_half(1);
        s.receive_hurtplayer(&hurt(1, 2), Some((30.0, 30.0)));
        s.tick_life(50);
        assert!(s.life.is_dead());
        let at = (s.player_x, s.player_y);
        s.tick_combat(500);
        assert_eq!((s.player_x, s.player_y), at, "a dead player is not pushed");
    }

    #[test]
    fn knockback_probes_form_the_body_box() {
        assert_eq!(KNOCKBACK_PROBES.len(), 12);
        let (minx, maxx) = KNOCKBACK_PROBES.iter().fold((9.0f32, 0.0f32), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        let (miny, maxy) = KNOCKBACK_PROBES.iter().fold((9.0f32, 0.0f32), |a, p| (a.0.min(p.1), a.1.max(p.1)));
        assert_eq!((minx, maxx, miny, maxy), (0.6, 2.4, 1.1, 2.9));
        assert_eq!(((minx + maxx) / 2.0, (miny + maxy) / 2.0), HIT_CENTRE, "the box is centred on the hit centre");
    }
}
