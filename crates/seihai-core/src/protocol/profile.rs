//! Per-version client profiles.
//!
//! One table row per client version Seihai can present as. A row says how
//! that version logs in and frames its traffic, which server packets and
//! props it understands, which file and level features it has, and which
//! parts of the client it shows. The rest of Seihai reads these fields
//! instead of matching on versions, so picking a version in the GUI makes
//! the whole client behave as that build.
//!
//! The rows are not written here: [`PROFILES`] is the resolved version chain
//! in [`super::chain`] (each build is a small delta against the one before).
//!
//! Sources, all cleaned (no decompile material):
//! - Graal_analysis `client-internals/cross-version-timeline.md`
//!   (packet and prop switch sets read from each build, 1.33 to 2.1 tests)
//! - Graal_analysis `protocol/client-versions.md` and `player-props.md`
//!   (2.05, 2.17, 2.22, 2.31 read from the binaries)
//! - Graal_analysis `client-internals/feature-timeline.md` and the shipped
//!   release notes (features per version)
//! - GServer-v2 and gs2lib (public source): version tokens and the
//!   server's per-version branches
//!
//! Rows marked [`Basis::Inferred`] have no client binary behind them; their
//! values come from release notes and server source and should be checked
//! when a build turns up.

use super::lobby::Lobby;
use super::login::LoginInfo;
use super::showimg::ShowImgInfo;
use super::tampercheck::{TamperInfo, TamperResend, TamperWords};
use super::version::{ClientVersion, EncryptionGen};
use std::time::Duration;

/// A set of message or prop ids (0 to 255).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct IdSet([u64; 4]);

impl IdSet {
    pub const EMPTY: IdSet = IdSet([0; 4]);

    /// Every id from `lo` to `hi`, inclusive.
    pub const fn range(lo: u8, hi: u8) -> Self {
        Self::EMPTY.with_range(lo, hi)
    }

    pub const fn with(self, id: u8) -> Self {
        let mut w = self.0;
        w[(id / 64) as usize] |= 1 << (id % 64);
        IdSet(w)
    }

    pub const fn without(self, id: u8) -> Self {
        let mut w = self.0;
        w[(id / 64) as usize] &= !(1 << (id % 64));
        IdSet(w)
    }

    pub const fn with_range(self, lo: u8, hi: u8) -> Self {
        let mut s = self;
        let mut i = lo as u16;
        while i <= hi as u16 {
            s = s.with(i as u8);
            i += 1;
        }
        s
    }

    pub const fn without_range(self, lo: u8, hi: u8) -> Self {
        let mut s = self;
        let mut i = lo as u16;
        while i <= hi as u16 {
            s = s.without(i as u8);
            i += 1;
        }
        s
    }

    pub const fn with_all(self, ids: &[u8]) -> Self {
        let mut s = self;
        let mut i = 0;
        while i < ids.len() {
            s = s.with(ids[i]);
            i += 1;
        }
        s
    }

    pub const fn without_all(self, ids: &[u8]) -> Self {
        let mut s = self;
        let mut i = 0;
        while i < ids.len() {
            s = s.without(ids[i]);
            i += 1;
        }
        s
    }

    /// Ids in both sets.
    pub const fn and(self, other: IdSet) -> Self {
        IdSet([
            self.0[0] & other.0[0],
            self.0[1] & other.0[1],
            self.0[2] & other.0[2],
            self.0[3] & other.0[3],
        ])
    }

    pub const fn contains(&self, id: u8) -> bool {
        self.0[(id / 64) as usize] & (1 << (id % 64)) != 0
    }

    pub fn len(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = u8> + '_ {
        (0..=255u8).filter(|&i| self.contains(i))
    }
}

impl std::fmt::Debug for IdSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

/// How well a row is backed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// Read from that version's client binary.
    Binary,
    /// No binary: release notes, server source, and the neighbouring builds.
    Inferred,
}

/// Where the client finds servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerList {
    /// No listserver: the client connects straight to one game server.
    Direct,
    /// A listserver with one list.
    Single,
    /// A listserver with this many tabs (2.14: company and playerworlds;
    /// 2.22 adds under-construction worlds).
    Tabs(u8),
}

/// The one host every profile dials for its server list.
pub const LISTSERVER_HOST: &str = "listserver.graal.in";
/// Default port of a profile that sends the classic listserver login.
pub const LEGACY_LIST_PORT: u16 = 14923;
/// Default port of a profile that sends the gen-5 listserver login.
pub const SUPPORTED_LIST_PORT: u16 = 14922;
/// Port the public service resets. No profile defaults to it.
pub const DEAD_LIST_PORT: u16 = 14921;

/// Which public listserver port a profile's list login uses. The public
/// listserver picks its reply by the login type, not by the version number
/// or token (matrix run, 2026-10-25): a classic login gets a list on 14923
/// and no reply on 14922; a gen-5 login gets a list on 14922 and also on
/// 14923. So the default follows the login the profile sends
/// (`Connection::login_listserver`: gen-5 only where the profile's
/// encryption is gen 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    /// Classic login (one plain zlib frame, id 0, then id 1 or not): port 14923.
    /// The profiles 1.41 to 2.17.
    Legacy,
    /// Gen-5 login (id 7 with `GNW30123`, then a gen-5 frame): port 14922.
    /// The profiles 2.22, 2.3 and 2.31.
    Supported,
}

impl ListKind {
    pub const fn port(self) -> u16 {
        match self {
            ListKind::Legacy => LEGACY_LIST_PORT,
            ListKind::Supported => SUPPORTED_LIST_PORT,
        }
    }
}

/// Whether a public check of this profile's own login got a list on its port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortStatus {
    /// A public check with Seihai's login for this profile got a list on this
    /// port. It says nothing about a real client of that version, which was
    /// not captured.
    Confirmed,
    /// The rule applied to a profile whose own login was not run on the public service.
    Assumed,
}

/// What a client sends to the listserver at login. The shape follows the
/// client generation; inside a generation every observed build sends the same
/// bytes (`documentation/listserver-login.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginShape {
    /// Shape A: one plain zlib frame, id 0 with the list name `newmain`.
    /// No account, password, token or key. Real 2.00 and 2.05.
    NewmainOnly,
    /// Shape B: one plain zlib frame with id 0 `newmain` then id 1 (account,
    /// password). No token, key or cipher. Real 2.17.1.
    NewmainAccount,
    /// Shape C: a plain zlib frame with id 7 (key byte, token `GNW30123`,
    /// `newmain`), then a gen-5 frame with id 1. Real 2.22 and 2.31.
    Gen5,
    /// Shape D: shape C's id 7 and id 1 in **one** plain zlib frame at the
    /// default level (`78 9C`); replies are gen 5. Real 3.04.
    Gen5OneFrame,
    /// Shape S: the client sends nothing and waits for the listserver to
    /// send the list. Real 1.41 rev 1 (lab, 2026-10-30; Graal_analysis
    /// `client-internals/version-profiles/README.md`). Seihai adds one thing:
    /// when no reply comes within [`SILENT_LIST_WAIT`] it sends shape B, so
    /// a listserver that waits for a request still answers.
    Silent,
}

/// How long a shape S login waits for the listserver to speak before
/// Seihai sends shape B. The real 1.41 waits with no limit.
pub const SILENT_LIST_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

impl LoginShape {
    /// `A`, `B` or `C`, as in the notes.
    pub const fn letter(self) -> &'static str {
        match self {
            LoginShape::NewmainOnly => "A",
            LoginShape::NewmainAccount => "B",
            LoginShape::Gen5 => "C",
            LoginShape::Gen5OneFrame => "D",
            LoginShape::Silent => "S",
        }
    }
}

/// Where a profile's [`LoginShape`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeBasis {
    /// A real client of this generation was captured in the lab, and the
    /// shape is what it sent.
    Captured,
    /// No capture of this build. The shape is a best guess from a captured
    /// neighbour (2.01 to 2.04 and 1.42 take 2.00's shape A).
    Inferred,
    /// No capture and no evidence for the shape. It is what Seihai sent
    /// before, kept as it was.
    Unknown,
}

impl ShapeBasis {
    /// The words used in the version-chain table and the launcher's hover text.
    pub const fn text(self) -> &'static str {
        match self {
            ShapeBasis::Captured => "login frame captured from a real client",
            ShapeBasis::Inferred => "login frame inferred, not captured",
            ShapeBasis::Unknown => "login frame inferred, not captured; no evidence for this shape",
        }
    }
}

/// Listserver login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listserver {
    /// Always [`LISTSERVER_HOST`].
    pub host: &'static str,
    /// The port dialled by default: [`ListKind::port`] of `kind`.
    pub port: u16,
    pub kind: ListKind,
    pub status: PortStatus,
    /// Token sent at listserver login. 2.22 and 2.31 send `GNW30123`
    /// inside id 7; older clients send their game token.
    pub token: Option<&'static str>,
    /// The frames `Connection::login_listserver` sends for this profile.
    pub shape: LoginShape,
    /// Whether that shape is from a captured real client.
    pub shape_basis: ShapeBasis,
}

impl Listserver {
    /// A listserver entry with the default shape of its kind: classic is
    /// shape B with an unknown basis (what Seihai sent before the lab
    /// captures), gen-5 is shape C, captured. Use [`Self::shape`] to set
    /// another.
    pub const fn new(kind: ListKind, status: PortStatus, token: Option<&'static str>) -> Self {
        let (shape, shape_basis) = match kind {
            ListKind::Legacy => (LoginShape::NewmainAccount, ShapeBasis::Unknown),
            ListKind::Supported => (LoginShape::Gen5, ShapeBasis::Captured),
        };
        Self {
            host: LISTSERVER_HOST,
            port: kind.port(),
            kind,
            status,
            token,
            shape,
            shape_basis,
        }
    }

    /// The same entry with another login shape and basis.
    pub const fn shape(mut self, shape: LoginShape, basis: ShapeBasis) -> Self {
        self.shape = shape;
        self.shape_basis = basis;
        self
    }

    /// The same entry dialled on another port, with what a public run
    /// found there. For a login whose reply differs from its kind's rule
    /// (3.x: gen-5, but only 14923 sends the list without an account).
    pub const fn on_port(mut self, port: u16, status: PortStatus) -> Self {
        self.port = port;
        self.status = status;
        self
    }

    /// `host:port`, as typed into the launcher or given to `--online`.
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Login, framing and the packets a version speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protocol {
    /// First GCHAR of the login line.
    pub client_type: u8,
    pub encryption: EncryptionGen,
    pub listserver: Option<Listserver>,
    /// Server→client ids the client has a handler for. The client drops
    /// any other id without error.
    pub plo: IdSet,
    /// Client→server ids the build can send. Seihai sends nothing else.
    pub pli: IdSet,
    /// `PLI_LANGUAGE` (37) right after login.
    pub sends_language: bool,
    /// `PLI_TAMPERCHECK` (39) after the language.
    pub tampercheck: bool,
    /// The build's tamper-check constant and re-send rule. `Some` for every
    /// build with `tampercheck`; `words` is `None` where the constant was
    /// never captured (2.3). Seihai sends it only when asked
    /// (`--send-tampercheck`); see [`super::tampercheck`].
    pub tamper: Option<TamperInfo>,
    /// When the build sends `PLI_SHOWIMGPLAYER` (24) with its constant 9;
    /// `Some` only for the builds observed doing it (2.22 and 2.31).
    /// Seihai sends it only when asked (`--send-showimg`); see
    /// [`super::showimg`].
    pub showimg: Option<ShowImgInfo>,
    /// What the build sends once the server's login burst is in (image
    /// checks, process list, login props); `Some` only for the builds
    /// observed doing it (2.17, 2.22 and 2.31). See [`super::login`].
    pub login: Option<LoginInfo>,
    /// The 4.x to 6.x Login lobby: the first server is named `Login`, the
    /// login line ends with the machine-field comma list, and nothing but
    /// the level is drawn until a `PLO_SERVERWARP` (178). See
    /// [`super::lobby`]. `None` for every build before 5.00.
    pub lobby: Option<Lobby>,
    /// 5.12's gen-5 step count: on zlib and bz2 frames the iterator steps
    /// once per whole 4-byte group (rounded down). Every other build rounds
    /// up, except the Mac PowerPC 4.04r2 and 5.12, read in their binaries
    /// (Tiger lab 2026-11-13). Read from 5.12's frame decoder and needed by
    /// the lab proxy to talk to it (Graal_analysis `protocol/v5v6-lab-play.md`,
    /// "5.12's cipher step count").
    pub gen5_round_down: bool,
    /// Every frame the client sends is compressed, even a short one: the
    /// Windows 5.12. The Mac 4.04r2 and 5.12 round down but send bodies under
    /// 48 bytes raw, as every other build.
    pub gen5_compress_all: bool,
    /// 6.x frame lengths: a frame of 0xFFFF bytes or more has `FF FF` and
    /// then a `u32be` length ([`super::FrameLen::Long`]). Read from the
    /// Linux 6.01 and 6.03 builds (Graal_analysis `protocol/v6x-clients.md`).
    pub gen5_long_frames: bool,
    /// Peer-to-peer UDP channel present.
    pub udp: bool,
    /// UDP off by default (`noudp=true` in the shipped config).
    pub udp_off_by_default: bool,
}

/// Player prop parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Props {
    /// Ids the `PLO_PLAYERPROPS` (own player) parser accepts. The first
    /// id outside the set ends the stream.
    pub own: IdSet,
    /// Ids the `PLO_OTHERPLPROPS` parser accepts, same rule.
    pub other: IdSet,
    /// Highest NPC prop id parsed.
    pub npc_max: u8,
    /// Prop 10 is the old bow slot (`c(10 + len) + image`) instead of a
    /// gani name. GServer sends that form below 2.1.
    pub bow_prop10: bool,
    /// X2/Y2/Z2 (78–80) and `PLO_MOVE2` are used for positions.
    pub precise_pos: bool,
}

/// File transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Files {
    /// `PLO_FILE` carries a 5-byte modtime (2.1+).
    pub modtime: bool,
    /// `PLO_LARGEFILESTART`/`END` (68/69): files over 64 KB (2.14+).
    pub large_files: bool,
    /// `PLO_LARGEFILESIZE` (84) (2.3+).
    pub large_file_size: bool,
    /// Default image extension. GServer adds `.gif` to bare image names for
    /// clients below 2.1.
    pub image_ext: &'static str,
    /// A downloaded `.gmap`, and a map-named `.txt`/`.gif`/`.png`/`.mng`,
    /// is saved in `maps/`. Read in 2.17.1, 2.22 and 2.31; 2.05 has no such
    /// rule (every download goes under `levels/`). 2.1 to 2.16 are inferred.
    /// See `filecache::download_dir`.
    pub download_maps: bool,
}

/// Level formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levels {
    /// `.graal` (GR-V1.0x) levels.
    pub graal: bool,
    /// Z3-V1 (`.zelda`) levels: `Z3-V1.03` and `Z3-V1.04`, read by magic.
    /// Every client binary from 1st Graal to 2.31 compares its level magic
    /// with these two (read from the binaries' strings and decompiles); the
    /// file's extension decides nothing. Seihai also opens a bare level name
    /// as `.zelda` after `.graal`, which is its own choice.
    pub zelda: bool,
    /// `.nw` (GLEVNW01) levels.
    pub nw: bool,
    /// `.gmap` overworlds.
    pub gmap: bool,
    /// Level heights in the level file (`PLO_BOARDHEIGHTS`, 2.17+).
    pub heights: bool,
    /// The build asks for a level it already holds with `PLI_LEVELWARPMOD`
    /// (30, with the level's mod time) instead of `PLI_LEVELWARP` (0).
    /// Seen on real 2.22 and 2.31 clients (lab, 2026-10-24); written up for
    /// 2.05 in Graal_analysis `packet-layouts.md`; taken as the same for
    /// the builds between and after (inferred). Not claimed for 1.33 to
    /// 2.04: those binaries send id 30, but no document or capture says
    /// they use it for cached levels.
    pub cached_links: bool,
}

/// The HUD sheet (`state.png`) a build ships. A version's own asset folder
/// supplies the image; this says which layout to expect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudArt {
    /// 1.33–1.34: 224×81, A/S/D letters on the item buttons, digits in
    /// `sprites.png`.
    Letters,
    /// 1.35–1.37: as `Letters` with a CHAT label.
    LettersChat,
    /// 1.38+: blank item buttons (keys are configurable) and the digits
    /// moved into `state.png`. 1.38–1.39 ship it at 224×97; 1.41 to 2.31
    /// ship a 224×145 sheet with the same layout plus the AP and MP bar
    /// frames, which the HUD draws only when the sheet has them.
    BlankButtons,
}

/// The typeface a build asks Windows for when it draws chat text over a
/// player (`Canvas.Font.Name`). The default can be changed with `font=` in
/// the build's config; Seihai uses the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatFace {
    /// `Dialog` (1st Graal to 1.38). Windows has no font of that name, so
    /// its font mapper picks a sans serif; Seihai uses Arial or a look-alike.
    Dialog,
    /// `Helvetica` (1.39, 1.41). Windows maps it to Arial.
    Helvetica,
    /// `Tempus Sans ITC` (2.00 to 2.31, 6.00r2 and 6.03r7). The client
    /// installs `tempsitc.ttf` from its own folder.
    TempusSans,
    /// `Arial` (5.24: `font=Arial` in its config; 5.00r7 and 5.12 drew a
    /// bold sans serif in the lab).
    Arial,
}

/// How a build draws chat text over a player: its font (bold, as every
/// build asks for) at a cell height of `height` pixels (`Font.Height`; the
/// `fontsize=` default). The colour, shadow, anchor and wrapping are the
/// same in every build and live in `render::chattext`.
/// See Graal_analysis `client-internals/chat-text.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatText {
    pub face: ChatFace,
    pub height: u8,
}

/// Outer size of the main form, in pixels, title bar and borders included.
///
/// The playfield Seihai draws stays 640 by 480. These numbers are the form
/// sizes of the executables from 1.21 through 2.31. A build with no form of
/// its own keeps the size of the build before it: 2.12 to 2.16 keep 2.1's,
/// 2.3 keeps 2.22's, and 3.x onward keep 2.31's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPx {
    pub width: u16,
    pub height: u16,
}

/// Which parts of the client this version shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ui {
    pub server_list: ServerList,
    /// Up/Down recalls earlier typed chat lines (1.39+, last 100).
    pub chat_recall: bool,
    /// Which HUD art the build ships in `state.png`.
    pub hud: HudArt,
    /// Minimap set by the game server (2.1+).
    pub minimap: bool,
    /// The level editor (F4) and level loading (F5) are inside the client.
    /// 2.3 moved them to a separate GraalEditor.
    pub editor: bool,
    /// Ganis (`TGraalAni`, NewMain 1.42+).
    pub ganis: bool,
    /// Emotion icons on Ctrl+letter (1.38+).
    pub emoticons: bool,
    /// Configurable keys (1.38+).
    pub key_config: bool,
    /// Alt+0 log window (2.1+).
    pub log_window: bool,
    /// Ghost mode for free players (2.1+).
    pub ghost_mode: bool,
    /// Server name in the window title (2.05+).
    pub server_in_title: bool,
    /// The font of the chat text over players' heads.
    pub chat_text: ChatText,
    /// Outer size of the main form. See [`WindowPx`].
    pub window: WindowPx,
    /// Number of GS1 commands in the script table, for reference.
    pub gs1_commands: u16,
}

/// How a build's own client handles the player's death.
///
/// Read from the 2.05 rev 2 notes (`decomp-findings-2026-10-24/
/// 01-death-respawn.md`, `constants-205.tsv`); the tick is 50 ms. 1.39
/// has its own row ([`DEATH_139`]). 1.41 takes the 2.05 row, 1.42 clears
/// it, and 2.00 to 2.04 use the 2.05 numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeathRules {
    /// The death timer counts up one per tick and stops here (30 = 1.5 s).
    pub timer_cap: u16,
    /// Controls are ignored while the timer is at or below this (27); the
    /// first tick past it (28) lets the player act and respawn.
    pub input_lock_until: u16,
    /// The dead bit (0x08) goes into the STATUS prop (18) once the timer
    /// reaches this (28, 1.4 s). Before it, others see no death.
    pub status_bit_from: u16,
    /// Respawn hearts: below `low_ap_below` AP the player gets
    /// `low_hearts`; below `mid_ap_below` AP min(`mid_hearts_cap`, max
    /// hearts); otherwise max hearts.
    pub low_ap_below: u8,
    pub mid_ap_below: u8,
    pub low_hearts: u8,
    pub mid_hearts_cap: u8,
    /// Position used when no level entry point was saved and the client is
    /// online: x 30.5, y 30.0 in the current level, facing down. Kept in
    /// half tiles (61 and 60), the wire scale.
    pub default_x2: u8,
    pub default_y2: u8,
    /// The level-entry save (the point a respawn returns to) skips an entry
    /// whose level is in the same *name group* as the level left: the name
    /// without its folder, lower-cased, up to its last `§` or `$`
    /// (delimiter included; names with neither all share the empty group).
    /// Only the 2.00 to 2.05 builds have this test; 2.1 test 2 and everything
    /// after it keep just the gmap rule. See `documentation/health-death.md`.
    pub entry_name_group: bool,
}

/// The 2.05 rev 2 values. 2.00 to 2.04 use the same: the 2.00 r0 handler
/// was read, and the timer cap, input gate, dead bit and respawn AP steps
/// are the same instructions in all eleven 2.00 to 2.04 builds. 2.00 alone
/// fires no `playerdies` (the script vocabulary already starts it at 2.01).
pub const DEATH_205: DeathRules = DeathRules {
    timer_cap: 30,
    input_lock_until: 27,
    status_bit_from: 28,
    low_ap_below: 20,
    mid_ap_below: 40,
    low_hearts: 3,
    mid_hearts_cap: 5,
    default_x2: 61,
    default_y2: 60,
    entry_name_group: true,
};

/// 2.1 test 2 and later: the same death sequence, but the level-entry save
/// no longer has the name-group test (the delimiter set is not in those
/// binaries; the 2.17.1, 2.22 and 2.31 entry code was read, the 2.1 tests'
/// was inferred from the missing constant).
pub const DEATH_21: DeathRules = DeathRules {
    entry_name_group: false,
    ..DEATH_205
};

/// 1.39. The timer, the input lock, the dead bit and the online default
/// spot match 2.05. A respawn always gives 3 hearts: the revive routine
/// sets 3.0 before anything else and reads no AP, so the low threshold is
/// `u8::MAX` and every alignment takes the first branch of
/// [`crate::life::respawn_half_hearts`]. 1.21 to 1.38 revive the same way;
/// the AP bands start in 1.41 rev 1 (U7 static reads, 2026-11-13; this
/// replaces the "hearts to the maximum" of Graal_analysis
/// `version-profiles/matrix/rules-139.md`). There is no name-group test on
/// the saved entry (that test is 2.00 to 2.05).
pub const DEATH_139: DeathRules = DeathRules {
    low_ap_below: u8::MAX,
    mid_ap_below: u8::MAX,
    entry_name_group: false,
    ..DEATH_205
};

/// How a build's client judges and sends hits from other players (the
/// victim's side of combat). Sources: `decomp-findings-2026-10-24/
/// 04-sword-hitbox.md`, `01-death-respawn.md`, `constants-205.tsv` (2.05
/// rev 2, derived), and Graal_analysis `protocol/packet-layouts.md` "Hits".
/// Later builds take the same row as an inference. 2.00 to 2.04 were read
/// in their own binaries (Graal_analysis `client-internals/combat-200-204.md`):
/// 2.03 and 2.04 judge as 2.05, 2.00 to 2.02 differ in the fields below.
/// Distances are in half tiles, the wire scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatRules {
    /// The attacker's sprite number (SPRITE prop 17 is `dir + 4 * S`) is a
    /// sword swing from `swing_sprite_first` to `swing_sprite_last`
    /// (animation steps 9 to 13). Outside it a sword cannot hit.
    pub swing_sprite_first: u8,
    pub swing_sprite_last: u8,
    /// Seihai's own swing, one animation step each, in ticks of 50 ms.
    /// **Placeholder, not a client value**: the real frame times come from
    /// the sword gani file, which Seihai does not play yet.
    pub swing_step_ticks: u16,
    /// The victim must be `reach_min_half` to `reach_max_half` ahead of the
    /// attacker in the direction the attacker faces (0 to 4 tiles,
    /// inclusive) ...
    pub reach_min_half: u8,
    pub reach_max_half: u8,
    /// ... and at most `lateral_max_half` to the side (2 tiles, the edge
    /// included: every build from 1.21 to 2.31 compares `|side| <= 2.0`).
    pub lateral_max_half: u8,
    /// A victim with exactly this alignment (AP) cannot be hit by a sword.
    /// `u8::MAX` (never an AP) where the build has no such test (before
    /// 1.34).
    pub immune_ap: u8,
    /// Hurt mode (mode 3) lasts this many ticks; a hurt player cannot be
    /// hurt again until it ends (40 ticks, 2 s).
    pub hurt_ticks: u16,
    /// Sprite number others see while the player is hurt (S = 32).
    pub hurt_sprite: u8,
    /// `PLO_HURTPLAYER` (40) applies only when its attacker is within this
    /// many half tiles of the victim (10 tiles). `u8::MAX` means no range
    /// test: 1.31 to 1.39 apply the hit from any distance.
    pub hurtplayer_range_half: u8,
    /// A hit by id 40 or 46 takes at most this many half hearts (1 heart).
    /// `u8::MAX` means no cap: 1.31 to 1.34 take the power as sent.
    pub packet_hit_cap_half: u8,
    /// A sword hit takes the attacker's sword power in half hearts, with the
    /// power limited to plus or minus this (20). The wire value is power + 30
    /// (10 to 50), and 0 to 4 for the default swords 1 to 4.
    pub sword_power_limit: u8,
    /// A negative sword power heals the victim, but only at this AP or more
    /// (40). Below it the hit hurts (hurt mode) without changing hearts.
    pub heal_min_ap: u8,
    /// An attacker whose STATUS (18) has any of these bits (0x22: hidden and
    /// one untraced) cannot land a sword hit.
    pub attacker_blocked_status: u8,
    /// The hit judge handles one swing or hit per attacker per value of the
    /// client's second-of-day clock; later ones in the same second are
    /// dropped.
    pub judge_throttle: bool,
    /// How the victim keeps another player's sword power (see
    /// [`PowerStore`]).
    pub sword_power_store: PowerStore,
    /// The hurt packets (40, 46) are refused, as a sword hit is, when the
    /// victim's AP is 100 or the level ran `noplayerkilling` (2.01 on). The
    /// 2.00 handler tests neither; it has only the hurtable, guild and range
    /// gates.
    pub packet_victim_gates: bool,
    /// The client takes hits by packet (`PLO_HURTPLAYER` 40, and 46 from
    /// 2.01) under the gates above. 1.31 is the first build with an id 40
    /// handler; 1.21, 1.28 beta 2 and 1.3 test send id 40 to the default
    /// case (U7 static reads, 2026-11-13).
    pub packet_hits: bool,
    /// 2.17.1, 2.22 and 2.31: the sword judge does nothing while the client
    /// is in ghost mode (`PLO_GHOSTMODE` 170), and neither the sword judge
    /// nor the hurt packets hurt the player while the client holds the
    /// flag that `PLO` 176 or 177 sets. 2.05 and 2.1 test 3 have neither
    /// test (U7 static reads, 2026-11-13).
    pub ghost_and_stop_gates: bool,
}

/// How the victim's client stores another player's sword power. Every build
/// keeps it in a 6-bit slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerStore {
    /// 2.03 on: limited to plus or minus `sword_power_limit` and stored with
    /// that offset, so a negative power survives (and heals).
    Limited,
    /// 2.00 to 2.02: the received power as is, in 6 bits, so it reads back
    /// as 0 to 63. A negative power wraps (63 for -1) and never heals; the
    /// wrap is inferred from the store's bit mask, not seen.
    Raw6Bit,
}

/// The 2.05 rev 2 values.
pub const COMBAT_205: CombatRules = CombatRules {
    swing_sprite_first: 6,
    swing_sprite_last: 10,
    swing_step_ticks: 2,
    reach_min_half: 0,
    reach_max_half: 8,
    lateral_max_half: 4,
    immune_ap: 100,
    hurt_ticks: 40,
    hurt_sprite: 32,
    hurtplayer_range_half: 20,
    packet_hit_cap_half: 2,
    sword_power_limit: 20,
    heal_min_ap: 40,
    attacker_blocked_status: 0x22,
    judge_throttle: true,
    sword_power_store: PowerStore::Limited,
    packet_victim_gates: true,
    packet_hits: true,
    ghost_and_stop_gates: false,
};

/// 2.17.1, 2.22 and 2.31: the 2.05 rules, plus the ghost-mode test in the
/// sword judge and the 176/177 flag test in the sword judge and the hurt
/// packets (read in all three binaries).
pub const COMBAT_217: CombatRules = CombatRules {
    ghost_and_stop_gates: true,
    ..COMBAT_205
};

/// 1.35 to 1.39. One swing step per 50 ms tick: the lab capture of a real
/// 1.39 (Graal_analysis live trace `item-drops/b4_139.txt`) sends S 6, 8, 10
/// at 100 ms intervals and then the idle sprite, and the video read gives a
/// 5-tick swing that cuts the bush; the 40-tick hurt lock-out is the
/// video's too (`player-mannerisms-live.md`, "Cross-version table").
/// The rest was read in the binaries of 1.34 to 1.41 (gap-2 research,
/// 2026-11-13): the sword box is the 2.05 box, the sword judge tests AP 100
/// and the attacker's hidden flag, has no once-a-second stamp, and keeps
/// the sword power raw (no plus or minus 20 limit, as 2.00 to 2.02). Its
/// id 40 handler caps the hit at one heart but tests no range, no AP and no
/// no-kill level (U7 static reads, 2026-11-13).
pub const COMBAT_139: CombatRules = CombatRules {
    swing_step_ticks: 1,
    hurtplayer_range_half: u8::MAX,
    ..COMBAT_200
};

/// 1.34: the 1.39 rules, but the id 40 handler takes the power as sent,
/// with no one-heart cap.
pub const COMBAT_134: CombatRules = CombatRules {
    packet_hit_cap_half: u8::MAX,
    ..COMBAT_139
};

/// 1.41 rev 1: the 1.39 sword judge, and the 2.00 id 40 handler (the
/// 10-tile range and the one-heart cap, no AP test).
pub const COMBAT_141: CombatRules = CombatRules {
    swing_step_ticks: 1,
    ..COMBAT_200
};

/// NewMain 1.42: the 2.00 rules (read in NewMain 24 and 26). Its id 40
/// handler is the 2.00 one, so packet hits are on.
pub const COMBAT_142: CombatRules = COMBAT_200;

/// 1.31 to 1.33: as 1.34, but the sword judge has no AP immunity (alignment
/// arrives in 1.3 and the judge first tests it in 1.34) and gates on the
/// attacker's sprite S 6 to 10 itself, polled every tick.
pub const COMBAT_131: CombatRules = CombatRules {
    immune_ap: u8::MAX,
    ..COMBAT_134
};

/// 1.21, 1.28 beta 2 and 1.3 test: as 1.31, with no id 40 handler.
pub const COMBAT_121: CombatRules = CombatRules {
    packet_hits: false,
    ..COMBAT_131
};

/// 2.01 and 2.02: as 2.05, but the sword power is stored without the offset
/// and limit. (Their kick test and the no-kill flag on the player, not the
/// level, change nothing Seihai checks.)
pub const COMBAT_201: CombatRules = CombatRules {
    sword_power_store: PowerStore::Raw6Bit,
    ..COMBAT_205
};

/// 2.00: as 2.01, and the hurt packet has only the hurtable, guild and range
/// gates: no AP 100 immunity, no no-kill test and no once-a-second limit.
/// The sword judge has no once-a-second stamp either (no build read has one
/// there), so `judge_throttle` is off.
pub const COMBAT_200: CombatRules = CombatRules {
    judge_throttle: false,
    packet_victim_gates: false,
    ..COMBAT_201
};

/// Health, death and combat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Health {
    /// The client-side death sequence, where it was traced.
    pub death: Option<DeathRules>,
    /// How hits from other players are judged, where it was traced.
    pub combat: Option<CombatRules>,
}

/// When the hurt blink starts (the body drawn on alternate ticks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blink {
    /// From the hit, for the whole of hurt mode (1.41 on).
    FromHit,
    /// Only once the hurt pose is over, to the end of hurt mode (1.39).
    AfterHurtPose,
}

/// What a player hiding under a carried object (D) looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HidePose {
    /// `hiddenstill` / `hidden`: only the object is drawn (2.05, 2.22).
    Hidden,
    /// The carry poses go on, legs cycling (1.39, NewMain 1.41).
    Carry,
}

/// What the pull pose does when the arrow is let go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullRelease {
    /// It stays, looping, until the player moves (2.05, 2.22).
    StaysPull,
    /// Back to `grab`, which stays after A too (NewMain 1.41).
    BackToGrab,
}

/// What the swim stroke does while the player floats still.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwimStill {
    /// It runs on (2.05).
    Animates,
    /// Back to its first step at the stop, then runs on (2.22).
    Restarts,
    /// Back to its first step and held (NewMain 1.41).
    Freezes,
}

/// How the player moves and looks, where builds differ: read live tick by
/// tick in 1.39, NewMain 1.41, 2.05 rev 2 and 2.22 (Graal_analysis
/// `client-internals/player-mannerisms-live.md`, "Cross-version table").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mannerisms {
    pub blink: Blink,
    pub hide: HidePose,
    pub pull_release: PullRelease,
    pub swim_still: SwimStill,
    /// Knockback steps of one tile at the start of hurt mode. 1.39 uses
    /// five, the same window as 2.05 (counter readings 39 down to 35). An
    /// earlier video counted four; the binary's five steps are the row.
    pub knockback_ticks: u16,
}

/// 1.39 (video and memory read).
pub const MANNER_139: Mannerisms = Mannerisms {
    blink: Blink::AfterHurtPose,
    hide: HidePose::Carry,
    pull_release: PullRelease::StaysPull,
    swim_still: SwimStill::Animates,
    knockback_ticks: 5,
};
/// NewMain 1.41.1, the first gani client.
pub const MANNER_141: Mannerisms = Mannerisms {
    blink: Blink::FromHit,
    hide: HidePose::Carry,
    pull_release: PullRelease::BackToGrab,
    swim_still: SwimStill::Freezes,
    knockback_ticks: 5,
};
/// 2.05 rev 2.
pub const MANNER_205: Mannerisms = Mannerisms {
    blink: Blink::FromHit,
    hide: HidePose::Hidden,
    pull_release: PullRelease::StaysPull,
    swim_still: SwimStill::Animates,
    knockback_ticks: 5,
};
/// 2.22.
pub const MANNER_222: Mannerisms = Mannerisms {
    swim_still: SwimStill::Restarts,
    ..MANNER_205
};

/// Who puts down the items a bush or a pot leaves, and what the client
/// tells the server about floor items (Graal_analysis
/// `client-internals/item-drops.md`, lab runs with 1.39, 2.05 and 2.22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemDrops {
    /// The client rolls the drop itself (1.33 to 1.41): one `Random(100)`
    /// per cut or lifted bush, and a heart for every lifted pot. From 1.42
    /// the server drops them.
    pub client_roll: bool,
    /// A rolled item is told to the server with `PLI_ITEMADD` (1.38 to 1.41).
    pub add_to_server: bool,
    /// Online, a pickup is sent as `PLI_ITEMTAKE` and the server gives the
    /// item with props (1.38 on); before, the client applies it and sends
    /// `PLI_ITEMDEL`.
    pub server_take: bool,
    /// Online, a timed-out item is sent as `PLI_ITEMDEL` (1.38 on); before,
    /// it goes silently.
    pub del_on_expiry: bool,
}

/// 1.33 to 1.37.
pub const DROPS_133: ItemDrops = ItemDrops {
    client_roll: true,
    add_to_server: false,
    server_take: false,
    del_on_expiry: false,
};
/// 1.38 to 1.41.
pub const DROPS_138: ItemDrops = ItemDrops {
    client_roll: true,
    add_to_server: true,
    server_take: true,
    del_on_expiry: true,
};
/// 1.42 on: the server drops.
pub const DROPS_142: ItemDrops = ItemDrops {
    client_roll: false,
    add_to_server: false,
    server_take: true,
    del_on_expiry: true,
};

/// Baddy and tile rules where builds differ. Read in every client binary
/// from 1.21 to 2.31 (gap-2 research, 2026-11-13): the baddy tables (step,
/// look, aim, rock, arrow, power and hare height) are the same bytes in all
/// of them, and the baddy AI the same code apart from the fields here. A
/// dying baddy's item roll follows [`ItemDrops::client_roll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldRules {
    /// Standing on lava burns: half a heart once every 101 ticks of 50 ms,
    /// tested at the player's `(x + 1.5, y + 2)`. Every build from 1.28
    /// beta 2 has it; 1.21 does not.
    pub lava_burns: bool,
    /// When the local player's sword reaches a swamp soldier rising from
    /// the water.
    pub swamp_sword: SwampSword,
}

/// When a sword hits a swamp soldier that is up (mode 6, no hurt blink).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwampSword {
    /// From counter 17, when it surfaces, to the end of its round (1.21 to
    /// 2.00, every revision). Its touch and object tests keep 22 to 36.
    From17,
    /// Counter 22 to 36, the window its touch and object tests use (2.01 on).
    From22To36,
}

/// 1.21: no lava burn.
pub const WORLD_121: WorldRules = WorldRules {
    lava_burns: false,
    swamp_sword: SwampSword::From17,
};
/// 1.28 beta 2 to 2.00.
pub const WORLD_128: WorldRules = WorldRules {
    lava_burns: true,
    ..WORLD_121
};
/// 2.01 on.
pub const WORLD_201: WorldRules = WorldRules {
    swamp_sword: SwampSword::From22To36,
    ..WORLD_128
};

/// Everything that makes one client version itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionProfile {
    pub version: ClientVersion,
    /// Display name, e.g. "2.05".
    pub label: &'static str,
    /// Eight-character token sent at login.
    pub token: &'static str,
    /// Link date of the build, where known.
    pub date: &'static str,
    pub basis: Basis,
    pub protocol: Protocol,
    pub props: Props,
    pub files: Files,
    pub levels: Levels,
    pub life: Health,
    pub ui: Ui,
    pub mannerisms: Mannerisms,
    pub drops: ItemDrops,
    pub world: WorldRules,
    /// Which script engines the build runs, as far as the evidence goes.
    pub scripts: Scripts,
}

/// Whether a build runs a script engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Runs {
    Yes,
    No,
    /// Nothing read from this build settles it.
    Unknown,
}

impl Runs {
    pub fn as_str(self) -> &'static str {
        match self {
            Runs::Yes => "yes",
            Runs::No => "no",
            Runs::Unknown => "unknown",
        }
    }
}

/// The script engines of a build. Set per step in the version chain from
/// evidence only (`documentation/version-chain.md`, "Script engines"):
///
/// - `gs1`: yes where the GS1 command table was read from the binary
///   (1.21 to 2.05, the `gs1_commands` count) or the per-build vocabulary
///   notes cover it (2.17, 2.22, 2.31). Unknown elsewhere.
/// - `gs2`: yes where the binary handles `PLO_NPCBYTECODE` (131), the packet
///   that carries a compiled GS2 script (3.153 and every binary build after
///   it); no where the binary's server-id set has no 131 (every classic
///   binary and 3.04). Unknown for builds with no binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scripts {
    pub gs1: Runs,
    pub gs2: Runs,
    /// The GS2 builtin `resolveHost(host)`, a DNS lookup of a host name.
    /// Only 6.00r4 registers it: 6.00r2 and the 6.037 stub have no such
    /// name (Graal_analysis `client-internals/engine-600r4.md`).
    pub resolve_host: bool,
}

impl Scripts {
    /// GS1 only, both read.
    pub const GS1: Scripts = Scripts {
        gs1: Runs::Yes,
        gs2: Runs::No,
        resolve_host: false,
    };
    /// No binary: neither is known.
    pub const UNKNOWN: Scripts = Scripts {
        gs1: Runs::Unknown,
        gs2: Runs::Unknown,
        resolve_host: false,
    };
}

impl VersionProfile {
    /// Window title the way this version shows it.
    pub fn window_title(&self, server: Option<&str>) -> String {
        match server {
            Some(s) if self.ui.server_in_title && !s.is_empty() => {
                format!("Graal {} - {s}", self.label)
            }
            _ => format!("Graal {}", self.label),
        }
    }

    /// Level file extensions this version can open.
    pub fn level_extensions(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.levels.nw {
            v.push(".nw");
        }
        if self.levels.graal {
            v.push(".graal");
        }
        if self.levels.zelda {
            v.push(".zelda");
        }
        if self.levels.gmap {
            v.push(".gmap");
        }
        v
    }

    /// One-line summary for the GUI.
    pub fn summary(&self) -> String {
        let gen = match self.protocol.encryption {
            EncryptionGen::Gen1 => "gen 1",
            EncryptionGen::Gen2 => "gen 2 (zlib)",
            EncryptionGen::Gen3 => "gen 3 (zlib + LCG byte)",
            EncryptionGen::Gen5 => "gen 5 (XOR + zlib/bz2)",
        };
        let list = match self.ui.server_list {
            ServerList::Direct => "no listserver".to_string(),
            ServerList::Single => "server list".to_string(),
            ServerList::Tabs(n) => format!("server list, {n} tabs"),
        };
        let basis = match self.basis {
            Basis::Binary => "",
            Basis::Inferred => ", inferred",
        };
        format!(
            "{} {}: {gen}, {list}, {} server packets{basis}",
            self.label,
            self.token,
            self.protocol.plo.len()
        )
    }
}

// Tamper-check constants (CRC words A and B), from the 2026-10-24 decompile
// reading; the encoded bytes match the lab captures of the real clients.
pub(super) const TAMPER_217: TamperInfo = TamperInfo {
    words: Some(TamperWords {
        a: 0xab63_7cbe,
        b: 0x3e48_dc0c,
    }),
    resend: TamperResend::Never,
};
pub(super) const TAMPER_222: TamperInfo = TamperInfo {
    words: Some(TamperWords {
        a: 0x5dcb_77f6,
        b: 0xbc73_6311,
    }),
    resend: TamperResend::OnLevelChange,
};
pub(super) const TAMPER_231: TamperInfo = TamperInfo {
    words: Some(TamperWords {
        a: 0x9ecd_4bc0,
        b: 0xecea_65e2,
    }),
    resend: TamperResend::Timer,
};
/// 2.3: no binary, so no constant; it is taken to re-send like 2.31.
pub(super) const TAMPER_230: TamperInfo = TamperInfo {
    words: None,
    resend: TamperResend::Timer,
};

// Id 24 (lab, real clients, 2026-10-24): 2.22 about every 5.1 s while another
// player is in the level, 2.31 every 5.13 s always.
pub(super) const SHOWIMG_222: ShowImgInfo = ShowImgInfo {
    period: Duration::from_millis(5100),
    needs_others: true,
};
pub(super) const SHOWIMG_231: ShowImgInfo = ShowImgInfo {
    period: Duration::from_millis(5130),
    needs_others: false,
};
/// The real 4.04r2 (lab, 2026-10-31): alone in the level, 5.142 to 5.151 s
/// apart, the first at the end of the after-burst frames.
pub(super) const SHOWIMG_404: ShowImgInfo = ShowImgInfo {
    period: Duration::from_millis(5145),
    needs_others: false,
};
// 2.17.1 (lab, 2026-10-30, `documentation/difftest-217.md` item 3): alone in the
// level on both lab servers, the first one in the post-login frame, then 5.07
// to 5.11 s apart while idle (median 5.10 s over the five runs).
pub(super) const SHOWIMG_217: ShowImgInfo = ShowImgInfo {
    period: Duration::from_millis(5100),
    needs_others: false,
};

/// Every version Seihai can present as, oldest first. Resolved from
/// [`super::chain::CHAIN`]: each row is the row before it plus that build's
/// delta, so a new version is a step in the chain, not a new row here.
pub static PROFILES: &[VersionProfile] = &super::chain::RESOLVED;

/// The profile row for a version.
pub fn profile(v: ClientVersion) -> &'static VersionProfile {
    PROFILES
        .iter()
        .find(|p| p.version == v)
        .expect("every ClientVersion has a profile row")
}

/// The profile with this GUI label, e.g. "2.05".
pub fn by_label(label: &str) -> Option<&'static VersionProfile> {
    PROFILES.iter().find(|p| p.label == label)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The script-engine field says only what the evidence says: GS2 is
    /// "yes" exactly where a binary handles PLO_NPCBYTECODE (131) and "no"
    /// exactly where a binary does not; a build with no binary is unknown.
    #[test]
    fn script_engines_follow_the_evidence() {
        for p in PROFILES.iter() {
            let gs2 = match (p.basis, p.protocol.plo.contains(131)) {
                (Basis::Inferred, _) => Runs::Unknown,
                (Basis::Binary, true) => Runs::Yes,
                (Basis::Binary, false) => Runs::No,
            };
            assert_eq!(p.scripts.gs2, gs2, "{} gs2", p.label);
            if p.basis == Basis::Inferred {
                assert_eq!(p.scripts.gs1, Runs::Unknown, "{} gs1", p.label);
            }
        }
        use ClientVersion::*;
        for v in [V121, V139, V205, V217, V222, V231] {
            assert_eq!(v.profile().scripts, Scripts::GS1, "{v:?}");
        }
        assert_eq!(V3153.profile().scripts.gs2, Runs::Yes);
        assert_eq!(V304.profile().scripts.gs2, Runs::No);
        assert_eq!(V603R7Linux.profile().scripts.gs2, Runs::Yes);
    }

    #[test]
    fn every_version_has_one_row() {
        for v in ClientVersion::all_versions() {
            assert_eq!(
                PROFILES.iter().filter(|p| p.version == *v).count(),
                1,
                "{v:?}"
            );
        }
        assert_eq!(PROFILES.len(), ClientVersion::all_versions().len());
    }

    #[test]
    fn tokens_and_labels() {
        for p in PROFILES {
            assert_eq!(p.token.len(), 8, "{}", p.label);
            // GSERV before 1.33, GNW for the later 2D builds, G3D for the 3.x ones.
            assert!(
                p.token.starts_with("GNW")
                    || p.token.starts_with("G3D")
                    || p.token.starts_with("GSERV"),
                "{}",
                p.label
            );
        }
        let mut labels: Vec<_> = PROFILES.iter().map(|p| p.label).collect();
        labels.dedup();
        assert_eq!(labels.len(), PROFILES.len());
    }

    /// Counts read from the binaries.
    #[test]
    fn server_id_counts() {
        let n = |v| profile(v).protocol.plo.len();
        assert_eq!(n(ClientVersion::V133), 43);
        assert_eq!(n(ClientVersion::V139), 45);
        assert_eq!(n(ClientVersion::V142), 51);
        assert_eq!(n(ClientVersion::V205), 59);
        assert_eq!(n(ClientVersion::V217), 79);
        assert_eq!(n(ClientVersion::V222), 80);
        assert_eq!(n(ClientVersion::V231), 81);
        assert!(!profile(ClientVersion::V205).protocol.plo.contains(26));
        // Mac PowerPC switches, read statically 2026-11-13.
        let ids = |v| profile(v).protocol.plo.iter().collect::<Vec<u8>>();
        let mac404: Vec<u8> = (0..=25)
            .chain(27..=40)
            .chain([42, 43])
            .chain(45..=49)
            .chain([67, 68, 69, 74, 75, 82, 84, 100, 101, 102, 104, 105, 106])
            .chain([131, 132, 133, 134, 150, 151, 153, 154, 155, 156, 165, 166])
            .chain(168..=176)
            .chain(178..=187)
            .chain([189, 190, 191, 193, 194, 195])
            .collect();
        assert_eq!(mac404.len(), 97);
        assert_eq!(ids(ClientVersion::V404R2Mac), mac404);
        let mut mac420 = mac404.clone();
        mac420.extend([107, 108]);
        mac420.sort();
        assert_eq!(ids(ClientVersion::V420R6), mac420);
        let mut mac512 = mac420.clone();
        mac512.extend([140, 197]);
        mac512.sort();
        assert_eq!(ids(ClientVersion::V512Mac), mac512);
        assert_eq!(n(ClientVersion::V512Mac), 101);
        assert_eq!(n(ClientVersion::V404R2), 98);
        assert_eq!(n(ClientVersion::V512), 100);
        for v in [
            ClientVersion::V404R2,
            ClientVersion::V404R2Mac,
            ClientVersion::V420R6,
            ClientVersion::V420R8,
            ClientVersion::V512,
            ClientVersion::V512Mac,
        ] {
            assert!(profile(v).protocol.gen5_round_down, "{v:?}");
        }
        for v in [ClientVersion::V500R7, ClientVersion::V524R4] {
            assert!(!profile(v).protocol.gen5_round_down, "{v:?}");
        }
        assert!(!profile(ClientVersion::V231).protocol.plo.contains(44));
        assert!(!profile(ClientVersion::V139).protocol.plo.contains(42));
    }

    /// seihai-core#7 / seihai-gserver#95: `PLO_PING`-like 108 is handled
    /// by the 4.20r6 (Mac), 4.20r8 and Mac 5.12 reads only. The PLO
    /// switches of 4.04r2 (both), 5.00r7, Windows 5.12, 5.24r4 and later (Linux 6.01r1 and
    /// 6.03r7 read, Graal_analysis `v5v6/6x-findings.md`) have no case for
    /// it, though 6.x still sends 162, the 4.x answer to 108. A server must
    /// not count on a 6.x client answering 108 until a lab shows one does.
    #[test]
    fn id_108_follows_the_binary_reads() {
        use ClientVersion::*;
        for v in [V420R6, V420R8, V512Mac] {
            assert!(profile(v).protocol.plo.contains(108), "{v:?}");
        }
        for v in [
            V404R2,
            V404R2Mac,
            V500R7,
            V512,
            V524R4,
            V531R4,
            V600R2,
            V603R7,
            V603R7Linux,
        ] {
            assert!(!profile(v).protocol.plo.contains(108), "{v:?}");
        }
        for v in [V600R2, V603R7] {
            assert!(profile(v).protocol.pli.contains(162), "{v:?}");
        }
    }

    #[test]
    fn other_prop_sets() {
        let o = profile(ClientVersion::V205).props.other;
        assert_eq!(o.len(), 31);
        for id in [1, 3, 14, 22, 25, 43] {
            assert!(!o.contains(id));
        }
        assert!(profile(ClientVersion::V205).props.own.contains(42));
        assert!(!profile(ClientVersion::V205).props.own.contains(43));
    }

    #[test]
    fn generations() {
        use EncryptionGen::*;
        let g = |v| profile(v).protocol.encryption;
        assert_eq!(g(ClientVersion::V139), Gen2);
        assert_eq!(g(ClientVersion::V141), Gen3);
        assert_eq!(g(ClientVersion::V217), Gen3);
        assert_eq!(g(ClientVersion::V222), Gen5);
        assert_eq!(profile(ClientVersion::V222).protocol.client_type, 5);
        assert_eq!(profile(ClientVersion::V217).protocol.client_type, 0);
    }

    /// GServer's 2.1 line: modtime, gani prop 10 and `.png` defaults.
    #[test]
    fn the_2_1_line() {
        for p in PROFILES {
            let new = p.files.modtime;
            assert_eq!(new, !p.props.bow_prop10, "{}", p.label);
            assert_eq!(new, p.files.image_ext == ".png", "{}", p.label);
        }
        assert!(!profile(ClientVersion::V205).files.modtime);
        assert!(profile(ClientVersion::V210).files.modtime);
    }

    #[test]
    fn classic_sets() {
        let p = |v| profile(v);
        assert_eq!(p(ClientVersion::V133).protocol.pli, IdSet::range(0, 31));
        assert!(!p(ClientVersion::V138).protocol.pli.contains(25));
        assert!(p(ClientVersion::V139).protocol.pli.contains(80));
        assert!(p(ClientVersion::V133).props.other.contains(51));
        assert!(!p(ClientVersion::V133).props.other.contains(34));
        assert!(p(ClientVersion::V134).props.other.contains(34));
        assert!(!p(ClientVersion::V138).props.other.contains(33));
        assert!(p(ClientVersion::V139).props.other.contains(33));
        assert_eq!(p(ClientVersion::V135).ui.hud, HudArt::LettersChat);
        assert_eq!(p(ClientVersion::V138).ui.hud, HudArt::BlankButtons);
    }

    /// A version only sends language if it can send id 37.
    #[test]
    fn language_needs_pli_37() {
        for p in PROFILES {
            assert_eq!(
                p.protocol.sends_language,
                p.protocol.pli.contains(37),
                "{}",
                p.label
            );
            if p.protocol.tampercheck {
                assert!(p.protocol.pli.contains(39), "{}", p.label);
            }
            // The tamper-check record exists exactly where the build sends it,
            // and every build that was read from a binary carries its words.
            assert_eq!(
                p.protocol.tamper.is_some(),
                p.protocol.tampercheck,
                "{}",
                p.label
            );
            if p.protocol.tampercheck && p.basis == Basis::Binary {
                assert!(p.protocol.tamper.unwrap().words.is_some(), "{}", p.label);
            }
        }
    }

    /// The per-version data from Graal_analysis' version-profile TOMLs
    /// (`profile_toml.tsv`, made by `tools/gen_profile_fixture.py`).
    struct TomlRow {
        label: String,
        token: String,
        inferred: bool,
        plo: IdSet,
        pli: IdSet,
        pli_lower_bound: bool,
        /// `None` (`-`) where the TOML has no parser reading (the 4.x to 6.x rows).
        own_max: Option<u8>,
        npc_max: Option<u8>,
        other: Option<IdSet>,
        tampercheck: bool,
        list_port: Option<u16>,
        list_host: Option<String>,
        list_kind: Option<String>,
        list_status: Option<String>,
    }

    fn parse_ids(s: &str) -> Option<IdSet> {
        if s == "-" {
            return None;
        }
        let mut set = IdSet::EMPTY;
        for part in s.split(',') {
            let (lo, hi) = part.split_once('-').unwrap_or((part, part));
            set = set.with_range(lo.parse().unwrap(), hi.parse().unwrap());
        }
        Some(set)
    }

    fn opt(s: &str) -> Option<String> {
        (s != "-").then(|| s.to_string())
    }

    fn toml_rows() -> Vec<TomlRow> {
        include_str!("profile_toml.tsv")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .map(|l| {
                let c: Vec<&str> = l.split('\t').collect();
                assert_eq!(c.len(), 13, "{l}");
                TomlRow {
                    label: c[0].to_string(),
                    token: c[1].to_string(),
                    inferred: c[2] == "inf",
                    plo: parse_ids(c[3]).unwrap(),
                    pli: parse_ids(c[4].trim_start_matches('>')).unwrap(),
                    pli_lower_bound: c[4].starts_with('>'),
                    own_max: (c[5] != "-").then(|| c[5].parse().unwrap()),
                    npc_max: (c[6] != "-").then(|| c[6].parse().unwrap()),
                    other: parse_ids(c[7]),
                    tampercheck: c[8] == "1",
                    list_port: c[9].parse().ok(),
                    list_host: opt(c[10]),
                    list_kind: opt(c[11]),
                    list_status: opt(c[12]),
                }
            })
            .collect()
    }

    /// Every row agrees with the version-profile TOMLs on token, basis,
    /// server and client id sets, prop limits, tampercheck and listserver
    /// port. To fix a failure, correct the row, or (if the TOML changed)
    /// regenerate `profile_toml.tsv`.
    #[test]
    fn rows_match_version_profile_toml() {
        let rows = toml_rows();
        assert_eq!(rows.len(), PROFILES.len(), "one TOML row per profile row");
        let mut bad = Vec::new();
        for t in &rows {
            let Some(p) = by_label(&t.label) else {
                bad.push(format!("{}: no profile row", t.label));
                continue;
            };
            let mut check = |name: &str, ok: bool| {
                if !ok {
                    bad.push(format!("{}: {name}", t.label));
                }
            };
            check("token", p.token == t.token);
            check("basis", (p.basis == Basis::Inferred) == t.inferred);
            check("plo", p.protocol.plo == t.plo);
            if t.pli_lower_bound {
                // Only the constant send ids are listed: they must all be in the set.
                check(
                    "pli covers the listed ids",
                    (0..=255u8).all(|id| !t.pli.contains(id) || p.protocol.pli.contains(id)),
                );
            } else {
                check("pli", p.protocol.pli == t.pli);
            }
            check("npc_max", t.npc_max.is_none_or(|n| p.props.npc_max == n));
            check("tampercheck", p.protocol.tampercheck == t.tampercheck);
            check(
                "sends_language == pli has 37",
                p.protocol.sends_language == t.pli.contains(37),
            );
            let ls = p.protocol.listserver;
            check("listserver port", ls.map(|l| l.port) == t.list_port);
            check(
                "listserver host",
                ls.map(|l| l.host.to_string()) == t.list_host,
            );
            check(
                "listserver kind",
                ls.map(|l| {
                    if l.kind == ListKind::Legacy {
                        "legacy"
                    } else {
                        "supported"
                    }
                    .to_string()
                }) == t.list_kind,
            );
            // The TOML records the matrix run of 2026-10-25, which sent Seihai's older
            // classic login (shape B). A profile that now sends shape A has not been run
            // with it, so it is assumed here whatever the TOML says.
            let want_status = match ls {
                Some(l) if l.shape == LoginShape::NewmainOnly => Some("assumed".to_string()),
                _ => t.list_status.clone(),
            };
            check(
                "listserver port status",
                ls.map(|l| {
                    if l.status == PortStatus::Confirmed {
                        "confirmed"
                    } else {
                        "assumed"
                    }
                    .to_string()
                }) == want_status,
            );
            // 2.16: its TOML entry keeps the 2.15 prop limit but notes that
            // 54-74 arrive in 2.16; the row follows the note.
            if t.label != "2.16" {
                check(
                    "own_max",
                    t.own_max
                        .is_none_or(|n| p.props.own.iter().max() == Some(n)),
                );
                if let Some(o) = t.other {
                    check("other props", p.props.other == o);
                }
            }
        }
        assert!(bad.is_empty(), "rows differ from the TOML: {bad:#?}");
    }

    /// The corrections this table was regenerated for.
    #[test]
    fn client_send_sets_after_toml_sync() {
        let pli = |v| profile(v).protocol.pli;
        // 2.00 predates ADJACENTLEVEL and HITOBJECTS (2.01).
        assert!(!pli(ClientVersion::V200).contains(35));
        assert!(!pli(ClientVersion::V200).contains(36));
        assert!(pli(ClientVersion::V201).contains(35));
        // 2.17 sends the 2.17 additions.
        for id in [34, 35, 36, 40, 41, 43, 44, 45] {
            assert!(pli(ClientVersion::V217).contains(id), "2.17 id {id}");
        }
        // 2.22 follows the binary list: no 36/45/81, adds 40/41/43/44.
        for id in [36, 45, 81] {
            assert!(!pli(ClientVersion::V222).contains(id), "2.22 id {id}");
        }
        for id in [40, 41, 43, 44] {
            assert!(pli(ClientVersion::V222).contains(id), "2.22 id {id}");
        }
        // 2.31: PUTNPC (21) gone, ENTERLEVEL (46) new.
        assert!(pli(ClientVersion::V222).contains(21));
        assert!(!pli(ClientVersion::V231).contains(21));
        assert!(pli(ClientVersion::V231).contains(46));
        // A real 2.31 client answers the process-list request with 44.
        assert!(pli(ClientVersion::V231).contains(44));
        assert_eq!(pli(ClientVersion::V230), pli(ClientVersion::V231));
        // The default follows the login type: 2.17 sends the classic login (14923),
        // 2.22 the gen-5 one (14922); 2.05 sends the classic login too.
        assert_eq!(ClientVersion::V217.listserver_port(), 14923);
        assert_eq!(ClientVersion::V205.listserver_port(), 14923);
        assert_eq!(ClientVersion::V222.listserver_port(), 14922);
        assert_eq!(ClientVersion::V200.listserver_port(), 14923);
        assert_eq!(ClientVersion::V201.listserver_port(), 14923);
        assert_eq!(profile(ClientVersion::V217).props.npc_max, 73);
    }

    /// The port rule (documentation/port-rule.md): 14921 is dead, so no
    /// profile defaults to it, and the binary's own port never decides it.
    #[test]
    fn no_profile_defaults_to_14921() {
        for p in PROFILES {
            if let Some(l) = p.protocol.listserver {
                assert_ne!(l.port, DEAD_LIST_PORT, "{}", p.label);
            }
            assert_ne!(p.version.listserver_port(), DEAD_LIST_PORT, "{}", p.label);
        }
    }

    /// Every profile with a listserver, and every version through
    /// `listserver_host`, dials `listserver.graal.in`.
    #[test]
    fn every_profile_host_is_listserver_graal_in() {
        for p in PROFILES {
            if let Some(l) = p.protocol.listserver {
                assert_eq!(l.host, "listserver.graal.in", "{}", p.label);
            }
            assert_eq!(
                p.version.listserver_host(),
                "listserver.graal.in",
                "{}",
                p.label
            );
            assert_eq!(
                p.version.listserver_addr(),
                format!("listserver.graal.in:{}", p.version.listserver_port()),
                "{}",
                p.label
            );
        }
    }

    /// The matrix of 2026-10-25 (19 profiles, ports 14922 and 14923, one
    /// list-only login each): every profile that sends the classic login got
    /// a list on 14923 and no reply on 14922; every profile that sends the
    /// gen-5 login got a list on both. The default is 14923 for the classic
    /// login and 14922 for the gen-5 login, and never 14921. 1.33 to 1.39
    /// have no listserver and borrow 2.05's (a classic login on 14923).
    #[test]
    fn login_type_decides_the_default_port() {
        let classic = [
            "1.41",
            "1.42",
            "2.00",
            "2.01",
            "2.02",
            "2.02 rev2",
            "2.03",
            "2.04",
            "2.05",
            "2.1",
            "2.12",
            "2.13",
            "2.14",
            "2.15",
            "2.16",
            "2.17",
        ];
        let gen5 = ["2.22", "2.3", "2.31", "3.04", "3.153"];
        let mut seen = 0;
        let mut lobby = 0;
        for p in PROFILES {
            match p.protocol.listserver {
                // The 4.x, 5.x and 6.x builds have no listserver protocol: their list is
                // scripted GUI on the `Login` game server.
                None if p.protocol.lobby.is_some() => lobby += 1,
                None => {
                    assert!(
                        p.label.starts_with("1.2") || p.label.starts_with("1.3"),
                        "{} has no listserver",
                        p.label
                    );
                    assert_eq!(p.version.listserver_port(), 14923, "{}", p.label);
                }
                Some(l) => {
                    seen += 1;
                    let want = if classic.contains(&p.label) {
                        ListKind::Legacy
                    } else {
                        assert!(gen5.contains(&p.label), "{} is in neither list", p.label);
                        ListKind::Supported
                    };
                    assert_eq!(l.kind, want, "{}", p.label);
                    // 3.x sends a gen-5 login but is dialled on 14923:
                    // 14922 answered it with the account error only, 14923
                    // with the list (public run, 2026-11-11).
                    let g3d = p.label.starts_with('3');
                    assert_eq!(
                        l.port,
                        if want == ListKind::Legacy || g3d {
                            14923
                        } else {
                            14922
                        },
                        "{}",
                        p.label
                    );
                    assert_eq!(l.port == l.kind.port(), !g3d, "{}", p.label);
                    assert_ne!(l.port, DEAD_LIST_PORT, "{}", p.label);
                    // The kind is the login the code sends: gen-5 only for a gen-5 profile.
                    assert_eq!(
                        l.kind == ListKind::Supported,
                        p.protocol.encryption == EncryptionGen::Gen5,
                        "{}: kind and the login type that login_listserver sends must agree",
                        p.label
                    );
                    assert_eq!(p.version.listserver_port(), l.port, "{}", p.label);
                }
            }
        }
        assert_eq!(seen, classic.len() + gen5.len());
        assert_eq!(lobby, 16);
        assert_eq!(seen + 12 + lobby, PROFILES.len());
    }

    /// The login shape per profile and where it comes from: real 2.00, 2.05,
    /// 2.17.1, 2.22 and 2.31 were captured; 1.42 and 2.01 to 2.04 take shape
    /// A as an inference; 1.41 and 2.1 to 2.16 keep shape B with no
    /// evidence; 2.3 is 2.31's shape C by inference.
    #[test]
    fn login_shape_and_basis_per_profile() {
        use LoginShape::*;
        use ShapeBasis::*;
        let want = [
            ("1.41", Silent, Captured),
            ("1.42", NewmainOnly, Captured),
            ("2.00", NewmainOnly, Captured),
            ("2.01", NewmainOnly, Inferred),
            ("2.02", NewmainOnly, Inferred),
            ("2.02 rev2", NewmainOnly, Inferred),
            ("2.03", NewmainOnly, Inferred),
            ("2.04", NewmainOnly, Inferred),
            ("2.05", NewmainOnly, Captured),
            ("2.1", NewmainAccount, Unknown),
            ("2.12", NewmainAccount, Unknown),
            ("2.13", NewmainAccount, Unknown),
            ("2.14", NewmainAccount, Unknown),
            ("2.15", NewmainAccount, Unknown),
            ("2.16", NewmainAccount, Unknown),
            ("2.17", NewmainAccount, Captured),
            ("2.22", Gen5, Captured),
            ("2.3", Gen5, Inferred),
            ("2.31", Gen5, Captured),
            ("3.04", Gen5OneFrame, Captured),
            ("3.153", Gen5OneFrame, Captured),
        ];
        for (label, shape, basis) in want {
            let l = by_label(label).unwrap().protocol.listserver.unwrap();
            assert_eq!((l.shape, l.shape_basis), (shape, basis), "{label}");
        }
        // 1.33 to 1.39 fetch their list as 2.05.
        assert_eq!(ClientVersion::V139.listserver().shape, NewmainOnly);
    }

    /// What the matrix of 2026-10-25 confirms: Seihai's own list login for
    /// 15 classic profiles on 14923 and for 2.22, 2.3 and 2.31 (gen-5) on
    /// 14922. 2.05 is the exception: the matrix binary sent a gen-5 login for
    /// 2.05, but this profile sends the classic one, which was not run, so
    /// its port is assumed. Profiles with no listserver have no status.
    #[test]
    fn port_status_confirmed_versus_assumed() {
        let confirmed: Vec<&str> = PROFILES
            .iter()
            .filter(|p| {
                p.protocol
                    .listserver
                    .is_some_and(|l| l.status == PortStatus::Confirmed)
            })
            .map(|p| p.label)
            .collect();
        assert_eq!(
            confirmed,
            [
                "1.41", "2.1", "2.12", "2.13", "2.14", "2.15", "2.16", "2.17", "2.22", "2.3",
                "2.31", "3.04", "3.153"
            ]
        );
        // The profiles that now send shape A (real 2.00 and 2.05, and the
        // inferred 1.42 and 2.01 to 2.04) were not run with it.
        for l in [
            "1.42",
            "2.00",
            "2.01",
            "2.02",
            "2.02 rev2",
            "2.03",
            "2.04",
            "2.05",
        ] {
            assert_eq!(
                by_label(l).unwrap().protocol.listserver.unwrap().status,
                PortStatus::Assumed,
                "{l}"
            );
        }
        assert_eq!(ClientVersion::V205.listserver().status, PortStatus::Assumed);
    }

    /// `--online host:port` is used exactly as given; a bare listserver host
    /// gets the version's default port.
    #[test]
    fn command_line_address_wins() {
        let v = ClientVersion::V200;
        assert_eq!(
            v.complete_list_addr("listserver.graal.in:14922"),
            "listserver.graal.in:14922"
        );
        assert_eq!(
            v.complete_list_addr("example.org:14921"),
            "example.org:14921"
        );
        assert_eq!(
            v.complete_list_addr("listserver.graal.in"),
            "listserver.graal.in:14923"
        );
        assert_eq!(
            ClientVersion::V222.complete_list_addr("listserver.graal.in"),
            "listserver.graal.in:14922"
        );
        assert_eq!(v.complete_list_addr("10.0.0.5"), "10.0.0.5");
    }

    /// Cached level links (`PLI_LEVELWARPMOD`): rows with evidence or an
    /// inferred successor of it, never the builds before 2.05, and always
    /// with id 30 in the send set.
    #[test]
    fn cached_links_by_profile() {
        for p in PROFILES {
            if p.levels.cached_links {
                assert!(
                    p.protocol.pli.contains(crate::net::PLI_LEVELWARPMOD),
                    "{}",
                    p.label
                );
                assert!(p.levels.nw, "{}: cached levels need .nw support", p.label);
            }
        }
        for v in [
            ClientVersion::V205,
            ClientVersion::V217,
            ClientVersion::V222,
            ClientVersion::V231,
        ] {
            assert!(v.sends_cached_links(), "{}", v.label());
        }
        for v in [
            ClientVersion::V133,
            ClientVersion::V139,
            ClientVersion::V200,
            ClientVersion::V204,
        ] {
            assert!(!v.sends_cached_links(), "{}", v.label());
        }
    }

    /// 2.02 rev 2, 2.03 and 2.04 each differ from the build before.
    #[test]
    fn the_2_0x_builds() {
        let p = |v| profile(v);
        let (a, b, c, d) = (
            p(ClientVersion::V202),
            p(ClientVersion::V202R2),
            p(ClientVersion::V203),
            p(ClientVersion::V204),
        );
        assert!(!a.protocol.sends_language && b.protocol.sends_language);
        assert!(!b.protocol.pli.contains(38) && c.protocol.pli.contains(38));
        assert!(!b.protocol.plo.contains(48) && c.protocol.plo.contains(48));
        assert!(c.protocol.plo.contains(26) && !d.protocol.plo.contains(26));
        assert!(d.protocol.plo.contains(166) && !c.protocol.plo.contains(166));
        assert!(d.props.own.contains(42) && !c.props.own.contains(42));
        assert_eq!(
            (a.token, b.token, c.token, d.token),
            ("GNW15041", "GNW20041", "GNW06051", "GNW24061")
        );
        // .nw levels: 1.41 and 1.42 have the loader, 2.00 to 2.04 drop it,
        // 2.05 brings it back. 2.05 is also the first with the server name in
        // the title.
        assert!(p(ClientVersion::V141).levels.nw && p(ClientVersion::V142).levels.nw);
        assert!(!p(ClientVersion::V200).levels.nw && !a.levels.nw && !d.levels.nw);
        assert!(p(ClientVersion::V205).levels.nw);
        assert_eq!(b.window_title(Some("X")), "Graal 2.02 rev2");
        assert_eq!(
            by_label("2.02 rev2").unwrap().version,
            ClientVersion::V202R2
        );
    }

    #[test]
    fn ui_switches() {
        assert!(matches!(
            profile(ClientVersion::V139).ui.server_list,
            ServerList::Direct
        ));
        assert!(profile(ClientVersion::V139).ui.chat_recall);
        assert!(!profile(ClientVersion::V138).ui.chat_recall);
        assert!(!profile(ClientVersion::V205).ui.minimap);
        assert!(profile(ClientVersion::V217).ui.minimap);
        assert!(!profile(ClientVersion::V231).ui.editor);
        assert_eq!(
            profile(ClientVersion::V139).level_extensions(),
            vec![".graal", ".zelda"]
        );
        // Every build reads Z3-V1 by magic (strings of every client binary).
        assert!(PROFILES.iter().all(|p| p.levels.zelda));
        assert_eq!(
            profile(ClientVersion::V205).window_title(Some("Classic")),
            "Graal 2.05 - Classic"
        );
        assert_eq!(
            profile(ClientVersion::V139).window_title(Some("Classic")),
            "Graal 1.39"
        );
    }

    /// Outer size of the main form. The playfield stays 640 by 480.
    /// 2.12 to 2.16 keep 2.1's size, 2.3 keeps 2.22's, and 3.x onward keep
    /// 2.31's: those forms were not measured on their own.
    #[test]
    fn main_form_outer_size() {
        let wh = |v: ClientVersion| profile(v).ui.window;
        let size = |w, h| WindowPx {
            width: w,
            height: h,
        };
        assert_eq!(wh(ClientVersion::V121), size(648, 509));
        for v in [
            ClientVersion::V128B2,
            ClientVersion::V13T,
            ClientVersion::V131,
            ClientVersion::V132,
            ClientVersion::V133,
            ClientVersion::V134,
            ClientVersion::V135,
            ClientVersion::V136,
            ClientVersion::V137,
            ClientVersion::V138,
            ClientVersion::V139,
        ] {
            assert_eq!(wh(v), size(648, 508), "{}", v.label());
        }
        assert_eq!(wh(ClientVersion::V141), size(648, 514));
        assert_eq!(wh(ClientVersion::V142), size(648, 507));
        assert_eq!(wh(ClientVersion::V200), size(648, 507));
        for v in [
            ClientVersion::V201,
            ClientVersion::V202,
            ClientVersion::V202R2,
            ClientVersion::V203,
            ClientVersion::V204,
            ClientVersion::V205,
            ClientVersion::V210,
            ClientVersion::V212,
            ClientVersion::V213,
            ClientVersion::V214,
            ClientVersion::V215,
            ClientVersion::V216,
        ] {
            assert_eq!(wh(v), size(648, 514), "{}", v.label());
        }
        let mut from_217 = false;
        for v in ClientVersion::all_versions() {
            if *v == ClientVersion::V217 {
                from_217 = true;
            }
            if from_217 {
                assert_eq!(wh(*v), size(644, 507), "{}", v.label());
            }
        }
        assert!(from_217);
    }

    #[test]
    fn rules_139_knockback_and_respawn() {
        let p = profile(ClientVersion::V139);
        assert_eq!(p.mannerisms.knockback_ticks, 5);
        assert_eq!(p.life.death, Some(DEATH_139));
        assert_eq!(p.life.combat, Some(COMBAT_139));
        assert_eq!(p.life.combat.unwrap().hurt_ticks, 40);
        assert_eq!(
            (
                p.life.combat.unwrap().reach_min_half,
                p.life.combat.unwrap().reach_max_half,
                p.life.combat.unwrap().lateral_max_half,
            ),
            (0, 8, 4)
        );
        // 1.41 keeps the AP bands. 1.42 clears the death row again.
        assert_eq!(profile(ClientVersion::V141).life.death, Some(DEATH_205));
        assert_eq!(profile(ClientVersion::V142).life.death, None);
        assert_eq!(profile(ClientVersion::V133).mannerisms.knockback_ticks, 5);
    }
}
