//! Buddy, ignore and mute lists, chat bubbles that expire, and the
//! filter that hides ignored and muted players.
//!
//! What a real client does (lab, 2026-10-24, a real 2.22 client; see
//! `documentation/chat-lists.md`):
//!
//! - **Buddy** and **Ignore** write a local file, one account name per
//!   line (`buddyaccounts.txt`, `ignoredaccounts.txt`), and send nothing.
//!   The client deletes the buddy file when it becomes empty.
//! - **Mute** sends `PLI_MUTEPLAYER` (43): `GSHORT player id`, `GCHAR 1`
//!   (mute) or `GCHAR 0` (unmute), one message per click.
//!
//! Hiding an ignored player's chat and PMs is the documented behaviour of
//! Ignore. Hiding a muted player's chat and PMs is Seihai's reading of Mute
//! (the notes only say "client-local mute set, same filter as ignore"); the
//! mute list file is Seihai's own, because no real client file for mute has
//! been seen.
//!
//! The files hold account names and nothing else, never a password.

use crate::protocol::{IdSet, SessionState};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// How long a chat bubble stays over a speaker, in milliseconds.
///
/// The 2.05 client sets a 60 counter when a chat line arrives (prop 12) and
/// counts it down once per drawn frame, then removes the text; there is no
/// fade (see `documentation/chat-log.md`). The 2.22 vendor text says chat
/// text "is always displayed at least 3 seconds". 60 frames of the 50 ms
/// game frame is 3 s, so the two agree. The frame length is the one
/// assumption (it was not read from the decompile for the draw loop).
pub const BUBBLE_MS: u32 = 3000;

/// `PLI_MUTEPLAYER`.
pub const PLI_MUTEPLAYER: u8 = 43;

/// The `PLI_MUTEPLAYER` message: `GSHORT player id`, `GCHAR 1` to mute or
/// `GCHAR 0` to unmute (lab capture of a real 2.22 client, player id 2).
pub fn mute_message(player_id: u16, mute: bool) -> Vec<u8> {
    let mut m = vec![PLI_MUTEPLAYER + 0x20];
    m.extend_from_slice(&crate::net::gshort(player_id));
    m.push(u8::from(mute) + 0x20);
    m.push(b'\n');
    m
}

/// Which list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Buddy,
    Ignore,
    Mute,
}

impl ListKind {
    pub const ALL: [ListKind; 3] = [ListKind::Buddy, ListKind::Ignore, ListKind::Mute];

    /// File name in the settings directory. The first two are the real
    /// client's names; the mute file is Seihai's own.
    pub fn file_name(self) -> &'static str {
        match self {
            ListKind::Buddy => "buddyaccounts.txt",
            ListKind::Ignore => "ignoredaccounts.txt",
            ListKind::Mute => "mutedaccounts.txt",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ListKind::Buddy => "buddy",
            ListKind::Ignore => "ignore",
            ListKind::Mute => "mute",
        }
    }
}

/// A set of account names, compared without regard to case. Keeps the
/// order names were added in and the case they were written in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameList {
    names: Vec<String>,
}

/// A name that can go in a list file: not empty, one line, no control
/// characters, no leading `#` (that is a comment in a file).
pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && !n.starts_with('#') && !n.chars().any(|c| c.is_control())
}

impl NameList {
    /// Read a list file's text. Tolerant: any line ending, a leading byte
    /// order mark, blank lines, `#` comments, spaces around a name and
    /// repeated names (in any case) are all fine.
    pub fn parse(text: &str) -> Self {
        let mut l = NameList::default();
        for line in text.trim_start_matches('\u{feff}').lines() {
            let name = line.trim();
            if valid_name(name) {
                l.add(name);
            }
        }
        l
    }

    /// The file text: one name per line.
    pub fn to_text(&self) -> String {
        self.names.iter().map(|n| format!("{n}\n")).collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        let n = name.trim();
        self.names.iter().any(|x| x.eq_ignore_ascii_case(n))
    }

    /// Add a name; false if it was there already or is not a valid name.
    pub fn add(&mut self, name: &str) -> bool {
        let n = name.trim();
        if !valid_name(n) || self.contains(n) {
            return false;
        }
        self.names.push(n.to_string());
        true
    }

    /// Remove a name; false if it was not there.
    pub fn remove(&mut self, name: &str) -> bool {
        let n = name.trim();
        let before = self.names.len();
        self.names.retain(|x| !x.eq_ignore_ascii_case(n));
        self.names.len() != before
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    /// Read a list file. A missing or unreadable file is an empty list.
    pub fn load(path: &Path) -> Self {
        match fs::read(path) {
            // Latin-1 safe: bytes that are not UTF-8 are read as is.
            Ok(b) => Self::parse(&String::from_utf8(b.clone()).unwrap_or_else(|_| b.iter().map(|&c| c as char).collect())),
            Err(_) => NameList::default(),
        }
    }

    /// Write the file atomically: a temporary file in the same directory,
    /// then a rename over the old one. An empty list deletes the file, as
    /// the real client does with `buddyaccounts.txt`.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if self.is_empty() {
            return match fs::remove_file(path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(
            ".{}.tmp{}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("list"),
            std::process::id()
        ));
        let write = || -> io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(self.to_text().as_bytes())?;
            f.sync_all()?;
            fs::rename(&tmp, path)
        };
        let r = write();
        if r.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        r
    }
}

/// The three lists and the directory they live in (the settings
/// directory, next to `seihai.conf`).
#[derive(Debug, Clone, Default)]
pub struct ChatLists {
    dir: Option<PathBuf>,
    pub buddy: NameList,
    pub ignore: NameList,
    pub mute: NameList,
    /// `PLI_MUTEPLAYER` messages sent since the lists were loaded.
    pub mute_sent: u32,
}

/// Most mute messages Seihai sends in one session. The stock GServer-v2
/// registers no handler for id 43, counts each as an invalid packet and
/// disconnects the client after more than 5 (lab notes, 2026-10-24).
/// Past this the list still changes but nothing more is sent.
pub const MUTE_SENDS_MAX: u32 = 5;

impl ChatLists {
    /// Load all three from `dir`. Missing files are empty lists.
    pub fn load(dir: &Path) -> Self {
        let mut l = ChatLists { dir: Some(dir.to_path_buf()), ..Default::default() };
        for k in ListKind::ALL {
            *l.list_mut(k) = NameList::load(&dir.join(k.file_name()));
        }
        l
    }

    /// Load from the settings directory.
    pub fn load_default() -> Self {
        Self::load(&crate::config::config_dir())
    }

    /// Lists that are not tied to a directory and are never saved.
    pub fn in_memory() -> Self {
        Self::default()
    }

    pub fn list(&self, k: ListKind) -> &NameList {
        match k {
            ListKind::Buddy => &self.buddy,
            ListKind::Ignore => &self.ignore,
            ListKind::Mute => &self.mute,
        }
    }

    pub fn list_mut(&mut self, k: ListKind) -> &mut NameList {
        match k {
            ListKind::Buddy => &mut self.buddy,
            ListKind::Ignore => &mut self.ignore,
            ListKind::Mute => &mut self.mute,
        }
    }

    /// Write one list to its file (nothing for an in-memory set).
    pub fn save(&self, k: ListKind) -> io::Result<()> {
        match &self.dir {
            Some(d) => self.list(k).save(&d.join(k.file_name())),
            None => Ok(()),
        }
    }

    /// Add `name` to a list and save it. Returns whether it changed.
    pub fn add(&mut self, k: ListKind, name: &str) -> io::Result<bool> {
        let changed = self.list_mut(k).add(name);
        if changed {
            self.save(k)?;
        }
        Ok(changed)
    }

    /// Remove `name` from a list and save it. Returns whether it changed.
    pub fn remove(&mut self, k: ListKind, name: &str) -> io::Result<bool> {
        let changed = self.list_mut(k).remove(name);
        if changed {
            self.save(k)?;
        }
        Ok(changed)
    }

    /// The filter for a client version: Ignore always applies; Mute only
    /// where the build sends `PLI_MUTEPLAYER` (43), so a mute file does
    /// nothing on a build that has no mute.
    pub fn filter(&self, pli: &IdSet) -> ChatFilter {
        ChatFilter {
            ignore: self.ignore.clone(),
            mute: if pli.contains(PLI_MUTEPLAYER) { self.mute.clone() } else { NameList::default() },
        }
    }
}

/// Who a chat line came from, as far as the client knows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    /// Account name (prop 34), once the server has sent it.
    pub account: Option<String>,
    pub nick: Option<String>,
}

impl Identity {
    /// Whether this is the player named `name` (an account or a nickname).
    pub fn is_named(&self, name: &str) -> bool {
        let n = name.trim();
        [&self.account, &self.nick].into_iter().flatten().any(|x| x.eq_ignore_ascii_case(n))
    }

    /// The name to store in a list for this player: the account if
    /// known, else the nickname.
    pub fn list_name(&self) -> Option<&str> {
        self.account.as_deref().or(self.nick.as_deref()).filter(|n| valid_name(n))
    }
}

/// The ignore and mute sets applied to incoming chat, bubbles and PMs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatFilter {
    pub ignore: NameList,
    pub mute: NameList,
}

impl ChatFilter {
    /// True if nothing can be hidden.
    pub fn is_empty(&self) -> bool {
        self.ignore.is_empty() && self.mute.is_empty()
    }

    /// Whether to hide a player's chat, bubbles and PMs. The account name
    /// decides when it is known; before that the nickname does (a player
    /// whose account has not arrived yet, or a build without account
    /// names, is matched by nickname).
    pub fn hides(&self, who: &Identity) -> bool {
        let key = who.account.as_deref().or(who.nick.as_deref());
        key.is_some_and(|k| self.ignore.contains(k) || self.mute.contains(k))
    }

    pub fn ignored(&self, who: &Identity) -> bool {
        who.account.as_deref().or(who.nick.as_deref()).is_some_and(|k| self.ignore.contains(k))
    }

    pub fn muted(&self, who: &Identity) -> bool {
        who.account.as_deref().or(who.nick.as_deref()).is_some_and(|k| self.mute.contains(k))
    }
}

/// A chat-bar command for the lists. Anything else is sent as chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListCommand {
    Add(ListKind, String),
    Remove(ListKind, String),
    Show(Option<ListKind>),
}

/// Read `/buddy name`, `/unbuddy name`, `/ignore name`, `/unignore name`,
/// `/mute name`, `/unmute name`, `/lists` and `/lists buddy|ignore|mute`.
pub fn parse_command(line: &str) -> Option<ListCommand> {
    let t = line.trim();
    let rest = t.strip_prefix('/')?;
    let (cmd, arg) = match rest.split_once(char::is_whitespace) {
        Some((c, a)) => (c, a.trim()),
        None => (rest, ""),
    };
    let cmd = cmd.to_ascii_lowercase();
    let named = |k: ListKind, add: bool| {
        if arg.is_empty() {
            return None;
        }
        Some(if add { ListCommand::Add(k, arg.to_string()) } else { ListCommand::Remove(k, arg.to_string()) })
    };
    match cmd.as_str() {
        "buddy" => named(ListKind::Buddy, true),
        "unbuddy" => named(ListKind::Buddy, false),
        "ignore" => named(ListKind::Ignore, true),
        "unignore" => named(ListKind::Ignore, false),
        "mute" => named(ListKind::Mute, true),
        "unmute" => named(ListKind::Mute, false),
        "lists" => match arg.to_ascii_lowercase().as_str() {
            "" => Some(ListCommand::Show(None)),
            "buddy" | "buddies" => Some(ListCommand::Show(Some(ListKind::Buddy))),
            "ignore" => Some(ListCommand::Show(Some(ListKind::Ignore))),
            "mute" => Some(ListCommand::Show(Some(ListKind::Mute))),
            _ => None,
        },
        _ => None,
    }
}

/// What a list command did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandOutcome {
    /// Lines to show the user.
    pub notes: Vec<String>,
    /// `PLI_MUTEPLAYER` messages to send: (player id, mute or unmute).
    pub mute_sends: Vec<(u16, bool)>,
}

/// Run a list command: update and save the list, refresh the filter in
/// `state`, and say what to send. Buddy and Ignore send nothing; Mute sends
/// id 43 for a player who is in the level, on a build whose `pli` has 43.
pub fn apply_command(cmd: &ListCommand, lists: &mut ChatLists, state: &mut SessionState, pli: &IdSet) -> CommandOutcome {
    let mut out = CommandOutcome::default();
    match cmd {
        ListCommand::Show(kind) => {
            let kinds: Vec<ListKind> = match kind {
                Some(k) => vec![*k],
                None => ListKind::ALL.to_vec(),
            };
            for k in kinds {
                if k == ListKind::Mute && !pli.contains(PLI_MUTEPLAYER) {
                    out.notes.push("mute: this version has no mute".into());
                    continue;
                }
                let l = lists.list(k);
                let names: Vec<&str> = l.iter().collect();
                out.notes.push(if names.is_empty() {
                    format!("{}: empty", k.label())
                } else {
                    format!("{}: {}", k.label(), names.join(", "))
                });
            }
        }
        ListCommand::Add(k, name) | ListCommand::Remove(k, name) => {
            let add = matches!(cmd, ListCommand::Add(..));
            if *k == ListKind::Mute && !pli.contains(PLI_MUTEPLAYER) {
                out.notes.push("this version has no mute".into());
                return out;
            }
            if !valid_name(name) {
                out.notes.push(format!("not a usable name: {name}"));
                return out;
            }
            // A player we have seen: store the account name if we know
            // it, otherwise what was typed.
            let seen = state.find_identity(name).map(|(_, w)| w.list_name().map(str::to_string));
            let stored = seen.flatten().unwrap_or_else(|| name.trim().to_string());
            let r = if add { lists.add(*k, &stored) } else { lists.remove(*k, &stored) };
            match r {
                Ok(true) => out.notes.push(format!(
                    "{} {} {} list",
                    stored,
                    if add { "added to" } else { "removed from" },
                    k.label()
                )),
                Ok(false) => out.notes.push(format!(
                    "{} is {} the {} list",
                    stored,
                    if add { "already in" } else { "not in" },
                    k.label()
                )),
                Err(e) => out.notes.push(format!("could not save {}: {e}", k.file_name())),
            }
            if *k == ListKind::Mute {
                match state.present_id(&stored).or_else(|| state.present_id(name)) {
                    Some(id) if lists.mute_sent < MUTE_SENDS_MAX => {
                        lists.mute_sent += 1;
                        out.mute_sends.push((id, add));
                    }
                    Some(_) => out.notes.push(format!(
                        "mute message not sent: {MUTE_SENDS_MAX} sent already this session (GServer-v2 disconnects after more than 5 unknown packets)"
                    )),
                    None => out.notes.push("not in the level: only the local list changed, nothing was sent".into()),
                }
            }
            state.set_filter(lists.filter(pli));
        }
    }
    out
}

/// The items of a player's popup menu, in the order of the 2.05 F7 list
/// (Message first and the default, History, Buddy, Ignore, Profile). A
/// build that sends `PLI_MUTEPLAYER` (43) has Mute after Ignore, as the
/// real 2.22 menu does (Message, History, Buddy, Ignore, Mute, Profile).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Message,
    History,
    Buddy,
    Ignore,
    Mute,
    Profile,
}

pub fn menu_items(pli: &IdSet) -> Vec<MenuItem> {
    let mut v = vec![MenuItem::Message, MenuItem::History, MenuItem::Buddy, MenuItem::Ignore];
    if pli.contains(PLI_MUTEPLAYER) {
        v.push(MenuItem::Mute);
    }
    v.push(MenuItem::Profile);
    v
}

/// A chat bubble's remaining time. Counts down from [`BUBBLE_MS`].
///
/// The player's own typed line starts *held*: the 2.05 client gives it a
/// counter one above the countdown range (61), which does not count down
/// until a direction key is pressed, and then runs the usual 60. So a line
/// typed while standing still stays over the head until the player moves.
///
/// Another player's line is a *floor* timer: it counts down from arrival
/// while it waits for the speaker's empty CURCHAT, and only ends once both
/// have happened. So it shows for at least 60 frames and until the empty
/// line comes (real 2.05 rev 2 under Wine, 2026-10-30: the text went 3.0 s
/// after it arrived when the empty line came 0.5 s in, and at once when it
/// came after 20 s).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BubbleTimer {
    left_ms: u32,
    held: bool,
    /// Counts down while held ([`Self::start_floor`]).
    floor: bool,
}

impl BubbleTimer {
    pub fn start() -> Self {
        BubbleTimer { left_ms: BUBBLE_MS, held: false, floor: false }
    }

    /// The timer of a line the player typed: waits for [`Self::release`].
    pub fn start_held() -> Self {
        BubbleTimer { left_ms: BUBBLE_MS, held: true, floor: false }
    }

    /// The timer of another player's line: runs at once, but the line also
    /// waits for [`Self::release`] (their empty CURCHAT).
    pub fn start_floor() -> Self {
        BubbleTimer { left_ms: BUBBLE_MS, held: true, floor: true }
    }

    /// Time left before the line may end.
    pub fn left_ms(&self) -> u32 {
        self.left_ms
    }

    /// A direction key was pressed: a held bubble starts its countdown.
    pub fn release(&mut self) {
        self.held = false;
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Advance by `dt_ms`. True when this step ended the bubble.
    pub fn tick(&mut self, dt_ms: u32) -> bool {
        if self.left_ms == 0 || (self.held && !self.floor) {
            return false;
        }
        self.left_ms = self.left_ms.saturating_sub(dt_ms);
        self.left_ms == 0 && !self.held
    }

    pub fn active(&self) -> bool {
        self.left_ms > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("seihai-chatlists-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parsing_is_tolerant() {
        let text = "\u{feff}Alice\r\n  bob  \n\n# a comment\nALICE\n\tCarol\t\nbad\u{7}name\n";
        let l = NameList::parse(text);
        let names: Vec<_> = l.iter().collect();
        assert_eq!(names, vec!["Alice", "bob", "Carol"]);
        assert!(l.contains("alice") && l.contains("BOB") && l.contains(" carol "));
        assert!(!l.contains("dave"));
        // No trailing newline, one name.
        assert_eq!(NameList::parse("x").len(), 1);
        assert!(NameList::parse("").is_empty());
    }

    #[test]
    fn add_remove_are_case_insensitive() {
        let mut l = NameList::default();
        assert!(l.add("Zed"));
        assert!(!l.add("zed"), "same name in another case");
        assert!(!l.add("   "));
        assert!(!l.add("#x"));
        assert!(!l.add("two\nlines"));
        assert!(l.remove("ZED"));
        assert!(!l.remove("zed"));
        assert!(l.is_empty());
    }

    #[test]
    fn round_trip_through_files_in_a_directory() {
        let d = tmpdir("rt");
        let mut l = ChatLists::load(&d);
        assert!(l.buddy.is_empty() && l.ignore.is_empty() && l.mute.is_empty());
        assert!(l.add(ListKind::Buddy, "Pal").unwrap());
        assert!(l.add(ListKind::Ignore, "Jerk").unwrap());
        assert!(l.add(ListKind::Ignore, "Troll2").unwrap());
        assert!(l.add(ListKind::Mute, "Loud").unwrap());
        assert!(!l.add(ListKind::Mute, "LOUD").unwrap(), "no change, no write");
        assert_eq!(fs::read_to_string(d.join("ignoredaccounts.txt")).unwrap(), "Jerk\nTroll2\n");
        assert_eq!(fs::read_to_string(d.join("buddyaccounts.txt")).unwrap(), "Pal\n");
        let back = ChatLists::load(&d);
        assert_eq!(back.buddy, l.buddy);
        assert_eq!(back.ignore, l.ignore);
        assert_eq!(back.mute, l.mute);
        // Removing the last name deletes the file.
        assert!(l.remove(ListKind::Buddy, "pal").unwrap());
        assert!(!d.join("buddyaccounts.txt").exists());
        // No temporary file is left behind.
        let left: Vec<_> = fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(left.iter().all(|n| !n.contains(".tmp")), "{left:?}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn saving_replaces_the_old_file_whole() {
        let d = tmpdir("atomic");
        let p = d.join("ignoredaccounts.txt");
        fs::write(&p, "a-very-long-old-name\nanother\n").unwrap();
        let mut l = NameList::load(&p);
        l.remove("a-very-long-old-name");
        l.remove("another");
        l.add("x");
        l.save(&p).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "x\n");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_or_unreadable_files_are_empty_lists() {
        let d = tmpdir("missing");
        assert!(NameList::load(&d.join("nope.txt")).is_empty());
        // A Latin-1 file (not UTF-8) still loads.
        fs::write(d.join("l.txt"), b"Ren\xe9\n").unwrap();
        assert_eq!(NameList::load(&d.join("l.txt")).len(), 1);
        // Deleting a file that is not there is fine.
        NameList::default().save(&d.join("nope.txt")).unwrap();
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn files_never_hold_anything_but_names() {
        let d = tmpdir("nopw");
        let mut l = ChatLists::load(&d);
        l.add(ListKind::Ignore, "someone").unwrap();
        let text = fs::read_to_string(d.join("ignoredaccounts.txt")).unwrap();
        assert_eq!(text, "someone\n");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn mute_packet_bytes() {
        // Lab capture: player id 2, mute then unmute.
        assert_eq!(mute_message(2, true), vec![43 + 0x20, 0x20, 0x22, 0x21, b'\n']);
        assert_eq!(mute_message(2, false), vec![43 + 0x20, 0x20, 0x22, 0x20, b'\n']);
        // A GSHORT of 200 is (1, 72).
        assert_eq!(mute_message(200, true), vec![0x4b, 0x21, 0x20 + 72, 0x21, b'\n']);
    }

    #[test]
    fn mute_is_gated_by_the_profile() {
        use crate::protocol::{profile, ClientVersion};
        let mut l = ChatLists::in_memory();
        l.mute.add("loud");
        l.ignore.add("jerk");
        let who = |n: &str| Identity { account: None, nick: Some(n.into()) };
        for v in ClientVersion::all_versions() {
            let p = profile(*v);
            let f = l.filter(&p.protocol.pli);
            let has43 = p.protocol.pli.contains(43);
            assert_eq!(f.hides(&who("loud")), has43, "{}", p.label);
            assert!(f.hides(&who("jerk")), "{}: ignore applies everywhere", p.label);
            assert_eq!(menu_items(&p.protocol.pli).contains(&MenuItem::Mute), has43, "{}", p.label);
        }
        // The set differs across versions: some build sends 43, some does not.
        let any = ClientVersion::all_versions().iter().any(|v| profile(*v).protocol.pli.contains(43));
        let none = ClientVersion::all_versions().iter().any(|v| !profile(*v).protocol.pli.contains(43));
        assert!(any && none);
    }

    #[test]
    fn menu_order() {
        use crate::protocol::{profile, ClientVersion};
        let p205 = profile(ClientVersion::V205);
        assert!(!p205.protocol.pli.contains(43));
        assert_eq!(
            menu_items(&p205.protocol.pli),
            vec![MenuItem::Message, MenuItem::History, MenuItem::Buddy, MenuItem::Ignore, MenuItem::Profile]
        );
        let p222 = profile(ClientVersion::V222);
        assert_eq!(
            menu_items(&p222.protocol.pli),
            vec![MenuItem::Message, MenuItem::History, MenuItem::Buddy, MenuItem::Ignore, MenuItem::Mute, MenuItem::Profile]
        );
    }

    #[test]
    fn filter_matches_account_first_then_nick() {
        let mut f = ChatFilter::default();
        f.ignore.add("jerk");
        let acc = |a: &str, n: &str| Identity { account: Some(a.into()), nick: Some(n.into()) };
        assert!(f.hides(&acc("Jerk", "Nice Guy")), "account decides, any case");
        assert!(!f.hides(&acc("other", "jerk")), "a known account is not overridden by a nickname");
        assert!(f.hides(&Identity { account: None, nick: Some("JERK".into()) }), "no account yet: nickname");
        assert!(!f.hides(&Identity::default()));
    }

    #[test]
    fn commands_parse() {
        assert_eq!(parse_command("/ignore  Bob "), Some(ListCommand::Add(ListKind::Ignore, "Bob".into())));
        assert_eq!(parse_command("/UNIGNORE bob"), Some(ListCommand::Remove(ListKind::Ignore, "bob".into())));
        assert_eq!(parse_command("/buddy a b"), Some(ListCommand::Add(ListKind::Buddy, "a b".into())));
        assert_eq!(parse_command("/unmute x"), Some(ListCommand::Remove(ListKind::Mute, "x".into())));
        assert_eq!(parse_command("/lists"), Some(ListCommand::Show(None)));
        assert_eq!(parse_command("/lists mute"), Some(ListCommand::Show(Some(ListKind::Mute))));
        // No name, unknown commands and plain chat are not commands.
        assert_eq!(parse_command("/ignore"), None);
        assert_eq!(parse_command("/dance"), None);
        assert_eq!(parse_command("hello /ignore bob"), None);
        assert_eq!(parse_command("ignore bob"), None);
    }

    fn who(state: &mut SessionState, id: u16, nick: &str, account: &str) {
        let mut v = vec![0x20 + 8, 0x20 + (id >> 7) as u8, 0x20 + (id & 0x7f) as u8, 0x20, 0x20 + nick.len() as u8];
        v.extend_from_slice(nick.as_bytes());
        v.extend_from_slice(&[34 + 0x20, 0x20 + account.len() as u8]);
        v.extend_from_slice(account.as_bytes());
        v.push(b'\n');
        state.apply_body(&v);
    }

    #[test]
    fn commands_update_lists_filter_and_mute_sends() {
        use crate::protocol::{profile, ClientVersion};
        let d = tmpdir("cmd");
        let pli222 = profile(ClientVersion::V222).protocol.pli;
        let mut lists = ChatLists::load(&d);
        let mut st = SessionState::default();
        who(&mut st, 2, "Loudmouth", "loud_acct");

        // Ignore by nickname: the account is what is stored, nothing is sent.
        let out = apply_command(&parse_command("/ignore loudmouth").unwrap(), &mut lists, &mut st, &pli222);
        assert!(out.mute_sends.is_empty());
        assert!(lists.ignore.contains("loud_acct"));
        assert_eq!(fs::read_to_string(d.join("ignoredaccounts.txt")).unwrap(), "loud_acct\n");
        assert!(st.hides(2));
        let out = apply_command(&parse_command("/unignore LOUD_ACCT").unwrap(), &mut lists, &mut st, &pli222);
        assert!(out.mute_sends.is_empty());
        assert!(!st.hides(2));
        assert!(!d.join("ignoredaccounts.txt").exists());

        // Buddy: local file only.
        let out = apply_command(&parse_command("/buddy Loudmouth").unwrap(), &mut lists, &mut st, &pli222);
        assert!(out.mute_sends.is_empty());
        assert_eq!(fs::read_to_string(d.join("buddyaccounts.txt")).unwrap(), "loud_acct\n");
        assert!(!st.hides(2), "a buddy is not hidden");

        // Mute: id 43 for the player in the level, then unmute.
        let out = apply_command(&parse_command("/mute loud_acct").unwrap(), &mut lists, &mut st, &pli222);
        assert_eq!(out.mute_sends, vec![(2, true)]);
        assert!(st.hides(2));
        let out = apply_command(&parse_command("/unmute loud_acct").unwrap(), &mut lists, &mut st, &pli222);
        assert_eq!(out.mute_sends, vec![(2, false)]);
        assert!(!st.hides(2));

        // Not in the level: the list changes, nothing is sent.
        let out = apply_command(&parse_command("/mute Faraway").unwrap(), &mut lists, &mut st, &pli222);
        assert!(out.mute_sends.is_empty());
        assert!(lists.mute.contains("Faraway"));
        assert!(out.notes.iter().any(|n| n.contains("nothing was sent")), "{:?}", out.notes);

        // A repeated add says so and changes nothing.
        let out = apply_command(&parse_command("/buddy LOUD_ACCT").unwrap(), &mut lists, &mut st, &pli222);
        assert!(out.notes[0].contains("already in"), "{:?}", out.notes);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn mute_command_is_refused_where_the_build_has_no_mute() {
        use crate::protocol::{profile, ClientVersion};
        let pli205 = profile(ClientVersion::V205).protocol.pli;
        assert!(!pli205.contains(43));
        let mut lists = ChatLists::in_memory();
        let mut st = SessionState::default();
        who(&mut st, 2, "Loud", "loud");
        let out = apply_command(&parse_command("/mute loud").unwrap(), &mut lists, &mut st, &pli205);
        assert!(out.mute_sends.is_empty());
        assert!(lists.mute.is_empty());
        assert!(out.notes[0].contains("no mute"));
        // Ignore still works there.
        apply_command(&parse_command("/ignore loud").unwrap(), &mut lists, &mut st, &pli205);
        assert!(st.hides(2));
    }

    #[test]
    fn mute_sends_are_capped_for_the_session() {
        use crate::protocol::{profile, ClientVersion};
        let pli = profile(ClientVersion::V222).protocol.pli;
        let mut lists = ChatLists::in_memory();
        let mut st = SessionState::default();
        who(&mut st, 2, "Loud", "loud");
        let mut sent = 0;
        for _ in 0..4 {
            sent += apply_command(&parse_command("/mute loud").unwrap(), &mut lists, &mut st, &pli).mute_sends.len();
            sent += apply_command(&parse_command("/unmute loud").unwrap(), &mut lists, &mut st, &pli).mute_sends.len();
        }
        assert_eq!(sent as u32, MUTE_SENDS_MAX);
        // The list still follows the command after the cap.
        apply_command(&parse_command("/mute loud").unwrap(), &mut lists, &mut st, &pli);
        assert!(lists.mute.contains("loud"));
    }

    #[test]
    fn lists_command_shows_names() {
        use crate::protocol::{profile, ClientVersion};
        let pli = profile(ClientVersion::V222).protocol.pli;
        let mut lists = ChatLists::in_memory();
        lists.buddy.add("a");
        lists.buddy.add("b");
        let mut st = SessionState::default();
        let out = apply_command(&ListCommand::Show(None), &mut lists, &mut st, &pli);
        assert_eq!(out.notes, vec!["buddy: a, b", "ignore: empty", "mute: empty"]);
    }

    #[test]
    fn bubble_expires_after_the_duration() {
        let mut b = BubbleTimer::start();
        assert!(b.active());
        assert!(!b.tick(BUBBLE_MS - 1), "one ms short: still up");
        assert!(b.active());
        assert!(b.tick(1), "this step ends it");
        assert!(!b.active());
        assert!(!b.tick(500), "ending is reported once");
        // A big step ends it too.
        let mut b = BubbleTimer::start();
        assert!(b.tick(60_000));
    }

    #[test]
    fn bubble_is_sixty_frames_of_fifty_ms() {
        assert_eq!(BUBBLE_MS, 60 * 50);
    }

    #[test]
    fn held_bubble_waits_for_release_then_runs_the_full_time() {
        let mut b = BubbleTimer::start_held();
        assert!(b.is_held() && b.active());
        // Standing still: it never counts down, however long it takes.
        assert!(!b.tick(600_000));
        assert!(b.active());
        // A direction key releases it and the usual 60 frames follow.
        b.release();
        assert!(!b.is_held());
        assert!(!b.tick(BUBBLE_MS - 1));
        assert!(b.tick(1));
        assert!(!b.active());
        // An ordinary bubble is not held.
        assert!(!BubbleTimer::start().is_held());
    }

    #[test]
    fn floor_bubble_runs_while_held_and_ends_on_release_or_time() {
        // Released early: it still runs its 60 frames.
        let mut b = BubbleTimer::start_floor();
        assert!(!b.tick(500));
        b.release();
        assert!(!b.tick(BUBBLE_MS - 501));
        assert!(b.tick(1));
        // Never released: the time runs out but the line stays.
        let mut b = BubbleTimer::start_floor();
        assert!(!b.tick(60_000));
        assert_eq!(b.left_ms(), 0);
        assert!(b.is_held());
    }
}
