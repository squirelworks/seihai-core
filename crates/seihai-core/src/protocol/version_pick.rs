//! Picking the client version for a server by itself (consumer mode).
//!
//! A game server never says which versions it takes until it turns one away:
//! GServer-v2 answers a login whose token it does not allow with S>C 16
//! (`PLO_DISCMESSAGE`) `Your client version is not allowed on this server.`,
//! a `\r` or `\n`, and `Allowed: ` followed by its list (`PlayerClient.cpp`,
//! `PlayerLogin.cpp` for a secure login, where the refusal comes before the
//! password). The list is built by `Server::loadAllowedVersions` from
//! `config/allowedversions.txt`: each entry is a token, printed by its gs2lib
//! name, or a range `A:B`, printed `A - B`, joined with `, `. A range admits
//! every token whose position in gs2lib's `clientVersions[]` lies between the
//! two ends. The listserver does not check versions, and its server list
//! carries no allowed list (Graal_analysis
//! `server-notes/public-allowed-versions.md`).
//!
//! So a client that picks its own version logs in once, reads the list from
//! the refusal if there is one ([`AllowedList::from_disconnect`]), and asks
//! [`pick`] which version to log in as next. The design, with the launcher's
//! side of it, is `documentation/auto-version.md`.
//!
//! The server's own gs2lib table is not visible to the client. This module
//! reads names and ranges against [`GS2LIB`], the order in seihai-gs2lib,
//! which keeps stock gs2lib's entries in stock order and adds Seihai's own
//! between them. A range on a stock server holds only stock entries, so a
//! token Seihai added ([`Gs2libEntry::stock`] false) is only
//! [`Admit::Unsure`] inside a range.

use super::ClientVersion;

/// One game entry of gs2lib's `clientVersions[]`, in table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gs2libEntry {
    pub token: &'static str,
    /// The names a server prints for this token: seihai-gs2lib's first, then
    /// older or stock spellings.
    pub names: &'static [&'static str],
    /// In stock gs2lib's table, not only in seihai-gs2lib's.
    pub stock: bool,
}

const fn e(token: &'static str, names: &'static [&'static str], stock: bool) -> Gs2libEntry {
    Gs2libEntry {
        token,
        names,
        stock,
    }
}

/// The game entries of seihai-gs2lib's `clientVersions[]` (`src/IUtil.cpp`),
/// in table order, with the names of `clientVersionString[]`. Empty slots are
/// left out: only the order matters here. iPhone and Worlds entries are kept
/// because ranges end on them (`1.41r1 - iPhone Belote 1.5`).
pub const GS2LIB: &[Gs2libEntry] = &[
    e("G2N15041", &["NW 2.02"], true),
    e("GSERV007", &["1.21"], true),
    e("GSERV009", &["1.25"], true),
    e("GSERV011", &["1.27"], true),
    e("GSERV013", &["1.28"], true),
    e("GSERV014", &["1.30"], true),
    e("GSERV016", &["1.31"], true),
    e("GSERV018", &["1.32"], true),
    e("GNW20020", &["1.33"], true),
    e("GNW22030", &["1.34"], true),
    e("GNW13040", &["1.35"], true),
    e("GNW10050", &["1.36"], true),
    e("GNW21050", &["1.37"], true),
    e("GNW22060", &["1.38"], true),
    e("GNW02070", &["1.38r1"], true),
    e("GNW07080", &["1.39"], true),
    e("GNW12080", &["1.39r1"], true),
    e("GNW23090", &["1.40"], false),
    e("GNW13110", &["1.41r1"], true),
    e("GNW08011", &["1.42 (NewMain)"], false),
    e("GNW24011", &["2.0"], true),
    e("GNW23021", &["2.01"], true),
    e("GNW15041", &["2.02"], true),
    e("GNW20041", &["2.02r2"], true),
    e("GNW06051", &["2.03"], true),
    e("GNW24061", &["2.04"], true),
    e("GNW03081", &["2.05r2", "2.05"], true),
    e("GNW31101", &["2.1"], true),
    e("GNW01012", &["2.12"], true),
    e("GNW23012", &["2.13"], true),
    e("GNW30042", &["2.14"], true),
    e("GNW19052", &["2.15"], true),
    e("GNW20052", &["2.151"], true),
    e("GNW12102", &["2.16"], true),
    e("GNW22122", &["2.17"], true),
    e("GNW21033", &["2.18"], true),
    e("GNW15053", &["2.19"], true),
    e("GNW28063", &["2.2"], true),
    e("GNW01113", &["2.21"], true),
    e("GNW03014", &["2.22"], true),
    e("GNW14015", &["2.3"], true),
    e("GNW28015", &["2.31"], true),
    e("G3D16053", &["3.0"], true),
    e("G3D27063", &["3.01"], true),
    e("G3D03014", &["3.041"], true),
    e("G3D27015", &["3.151 (Linux)"], false),
    e("G3D28015", &["3.153"], false),
    e("G3D28095", &["4.0211"], true),
    e("G3D09125", &["4.034"], true),
    e("G3D11125", &["4.031 (Linux)"], false),
    e("G3D17026", &["4.042"], true),
    e("G3D18026", &["4.042 (Mac)"], false),
    e("G3D19026", &["4.042 (Linux)"], false),
    e("G3D26076", &["4.110"], true),
    e("G3D27076", &["4.102 (Mac)"], false),
    e("G3D28076", &["4.110 (Linux)"], false),
    e("G3D21126", &["4.206 (Mac)"], false),
    e("G3D22126", &["4.208 (Linux)"], false),
    e("G3D20126", &["4.208"], true),
    e("G3D22067", &["5.007", "5.07"], true),
    e("G3D14097", &["5.12"], true),
    e("G3D15097", &["5.12 (Mac)"], false),
    e("G3D03048", &["6.00 beta1"], false),
    e("G3D15010", &["5.24"], false),
    e("G3D26090", &["5.31x"], true),
    e("G3D27090", &["6.004 (Mac)"], false),
    e("G3D28090", &["6.00 beta (Linux)"], false),
    e("G3D3007A", &["6.015"], true),
    e("G3D3107A", &["6.015 (Mac)"], false),
    e("G3D0108A", &["6.01 (Linux)"], false),
    e("G3D2505C", &["6.034/6.035"], true),
    e("G3D0311C", &["6.037"], true),
    e("G3D0511C", &["6.037 (Linux)"], true),
    e("GNP1905C", &["6.1.1.3"], false),
    e("G3D04048", &["iPhone 1.1"], true),
    e("G3D18010", &["iPhone 1.5"], true),
    e("G3D29090", &["iPhone 1.11"], true),
    e("G3D0208A", &["iPhone Classic+ 1.13"], false),
    e("G3D2504D", &["iPhone Belote 1.5"], true),
    e("G3D2204D", &["Graal Worlds"], true),
];

/// The table position of a token, or of a printed name (any spelling, any
/// case) or a bare token, as an `Allowed:` list or a config line writes it.
pub fn gs2lib_index(name_or_token: &str) -> Option<usize> {
    let s = name_or_token.trim();
    GS2LIB
        .iter()
        .position(|e| e.token == s || e.names.iter().any(|n| n.eq_ignore_ascii_case(s)))
}

/// One entry of an allowed list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowedItem {
    /// One token, by its [`GS2LIB`] position.
    One(usize),
    /// A range, both ends included, by [`GS2LIB`] position (low end first).
    Range(usize, usize),
    /// A name this table does not know, as the server printed it. Such an
    /// entry admits nothing Seihai can present as.
    Unknown(String),
}

/// How sure it is that a server takes a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Admit {
    /// Not in the list.
    No,
    /// Inside a range, but the token is one Seihai added to gs2lib: a server
    /// on stock gs2lib does not know it and refuses it.
    Unsure,
    /// Inside a range, and a stock entry.
    InRange,
    /// Named on its own.
    Named,
}

/// A server's allowed list, as printed after `Allowed:`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AllowedList {
    pub items: Vec<AllowedItem>,
}

impl AllowedList {
    /// The list from the text after `Allowed:`.
    pub fn parse(list: &str) -> AllowedList {
        let mut items = Vec::new();
        for part in list.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            // GServer-v2 prints a range as `A - B`. No gs2lib name holds
            // " - ", and none of them starts or ends with a space.
            let item = match part.split_once(" - ") {
                Some((a, b)) => match (gs2lib_index(a), gs2lib_index(b)) {
                    (Some(a), Some(b)) => AllowedItem::Range(a.min(b), a.max(b)),
                    _ => AllowedItem::Unknown(part.to_string()),
                },
                None => match gs2lib_index(part) {
                    Some(i) => AllowedItem::One(i),
                    None => AllowedItem::Unknown(part.to_string()),
                },
            };
            items.push(item);
        }
        AllowedList { items }
    }

    /// The list in a `PLO_DISCMESSAGE` text, if the text is a version
    /// refusal that carries one. Line breaks may be `\r` or `\n`.
    pub fn from_disconnect(text: &str) -> Option<AllowedList> {
        let lower = text.to_ascii_lowercase();
        if !lower.contains("version is not allowed") && !lower.contains("version not allowed") {
            return None;
        }
        let at = lower.find("allowed:")?;
        let rest = &text[at + "allowed:".len()..];
        let line = rest.split(['\r', '\n']).next().unwrap_or("");
        Some(AllowedList::parse(line))
    }

    /// Whether the list takes this token.
    pub fn admits(&self, token: &str) -> Admit {
        let Some(i) = GS2LIB.iter().position(|e| e.token == token) else {
            // gs2lib gives an unknown token CLVER_UNKNOWN, which no entry
            // or range admits.
            return Admit::No;
        };
        let mut best = Admit::No;
        for item in &self.items {
            let a = match *item {
                AllowedItem::One(j) if j == i => Admit::Named,
                AllowedItem::Range(lo, hi) if (lo..=hi).contains(&i) => {
                    if GS2LIB[i].stock {
                        Admit::InRange
                    } else {
                        Admit::Unsure
                    }
                }
                _ => Admit::No,
            };
            best = best.max(a);
        }
        best
    }

    /// The names it holds that this table does not know.
    pub fn unknown(&self) -> impl Iterator<Item = &str> {
        self.items.iter().filter_map(|i| match i {
            AllowedItem::Unknown(s) => Some(s.as_str()),
            _ => None,
        })
    }
}

/// The three kinds of world a version can play, after GServer-v2's server
/// generations (`classic`, `newmain`, `modern` in `allowedversions.txt`).
/// Versions in different eras play different worlds, so a server that takes
/// two eras with nothing between them is a choice for the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Era {
    /// Before 2.00: 1.x levels and GS1 as the 1.x clients ran it.
    Classic,
    /// 2.00 to 5.00r7: the NewMain line, GS1 run by the client.
    NewMain,
    /// 5.12 and later: GS2.
    Modern,
}

impl Era {
    pub fn of(v: ClientVersion) -> Era {
        let all = ClientVersion::all_versions();
        let at = |x| all.iter().position(|&a| a == x).expect("in all_versions");
        let i = at(v);
        if i < at(ClientVersion::V200) {
            Era::Classic
        } else if i < at(ClientVersion::V512) {
            Era::NewMain
        } else {
            Era::Modern
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Era::Classic => "Classic (1.x)",
            Era::NewMain => "NewMain (2.x to 5.00)",
            Era::Modern => "Modern (5.12 and later)",
        }
    }
}

/// What the server and the player need from the pick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Need {
    /// The server is listed as a 3D server (`3 ` prefix): prefer a build
    /// with a 3D side.
    pub three_d: bool,
    /// Versions that were already refused or failed on this server.
    pub refused: Vec<ClientVersion>,
}

/// What to log in as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Log in as this version without asking.
    Auto(ClientVersion),
    /// The server takes versions from more than one era with a gap between
    /// them: ask. One option per era, newest era first; `recommended` is the
    /// first option.
    Ask {
        options: Vec<ClientVersion>,
        recommended: ClientVersion,
    },
    /// Nothing Seihai can present as is in the list.
    Nothing,
}

/// A candidate's rank: higher is better. Field order is priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    /// Has a 3D side, when the server is a 3D one (always true otherwise).
    three_d: bool,
    /// Sure to be admitted (named, or a stock entry in a range).
    sure: bool,
    /// Read from a binary, not inferred.
    binary: bool,
    /// One of the default-listed builds (one per release, stock gs2lib token).
    listed: bool,
    /// Newer is better: position in [`ClientVersion::all_versions`].
    newest: usize,
}

fn rank(v: ClientVersion, admit: Admit, need: &Need) -> Rank {
    let p = v.profile();
    let all = ClientVersion::all_versions();
    Rank {
        three_d: !need.three_d || p.three_d.is_some(),
        sure: admit >= Admit::InRange,
        binary: p.basis == super::Basis::Binary,
        listed: v.listed_by_default(),
        newest: all.iter().position(|&a| a == v).unwrap_or(0),
    }
}

/// The versions the list takes, best first, with how sure each is.
pub fn ranked(list: &AllowedList, need: &Need) -> Vec<(ClientVersion, Admit)> {
    let mut out: Vec<(ClientVersion, Admit, Rank)> = ClientVersion::all_versions()
        .iter()
        .filter(|v| !need.refused.contains(v))
        .filter_map(|&v| {
            let a = list.admits(v.token());
            (a != Admit::No).then(|| (v, a, rank(v, a, need)))
        })
        .collect();
    out.sort_by_key(|x| std::cmp::Reverse(x.2));
    out.into_iter().map(|(v, a, _)| (v, a)).collect()
}

/// The version to log in as next, from a server's allowed list.
///
/// - The best candidate by [`ranked`] wins: a 3D build for a 3D server, then
///   a sure admit, then a binary-backed row, then a default-listed one, then
///   the newest.
/// - The player is asked only when the default-listed versions the list
///   takes fall in two or more runs (a listed version between them is not
///   taken) and those runs are in different [`Era`]s. One option per era:
///   that era's best. A contiguous range across eras (`1.41r1 - 6.037`)
///   is not a choice: the newest wins.
pub fn pick(list: &AllowedList, need: &Need) -> Pick {
    let ranked = ranked(list, need);
    let Some(&(best, _)) = ranked.first() else {
        return Pick::Nothing;
    };

    // Runs over the default-listed builds (one per release), so a gap means
    // a whole release is not taken, not a twin or an inferred row.
    let mut runs: Vec<Vec<ClientVersion>> = Vec::new();
    let mut open = false;
    for &v in ClientVersion::reborn_versions() {
        let taken = !need.refused.contains(&v) && list.admits(v.token()) != Admit::No;
        if taken {
            if open {
                runs.last_mut().expect("open run").push(v);
            } else {
                runs.push(vec![v]);
                open = true;
            }
        } else {
            open = false;
        }
    }
    let mut eras: Vec<Era> = Vec::new();
    for run in &runs {
        for &v in run {
            let era = Era::of(v);
            if !eras.contains(&era) {
                eras.push(era);
            }
        }
    }
    let split = runs.len() > 1 && {
        // Different eras in different runs: the first run's eras and some
        // later run's eras do not overlap.
        let first: Vec<Era> = runs[0].iter().map(|&v| Era::of(v)).collect();
        runs[1..]
            .iter()
            .any(|r| r.iter().all(|&v| !first.contains(&Era::of(v))))
    };
    if !split {
        return Pick::Auto(best);
    }

    // One option per era, the era's best by rank, newest era first.
    eras.sort_by(|a, b| b.cmp(a));
    let options: Vec<ClientVersion> = eras
        .iter()
        .filter_map(|&era| ranked.iter().map(|&(v, _)| v).find(|&v| Era::of(v) == era))
        .collect();
    if options.len() < 2 {
        return Pick::Auto(best);
    }
    // The overall best leads, so the recommendation follows the rank (a 3D
    // build for a 3D server even when a newer era is taken).
    let mut options = options;
    options.retain(|&v| Era::of(v) != Era::of(best));
    options.insert(0, best);
    Pick::Ask {
        recommended: options[0],
        options,
    }
}

/// The version to try first on a server with no remembered list: the newest
/// default-listed build (a 3D one for a 3D server). If it is refused, the
/// refusal carries the list for [`pick`].
pub fn first_try(need: &Need) -> ClientVersion {
    ClientVersion::reborn_versions()
        .iter()
        .rev()
        .copied()
        .filter(|v| !need.refused.contains(v))
        .find(|v| !need.three_d || v.profile().three_d.is_some())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ClientVersion::*;

    #[test]
    fn table_has_each_token_once_and_profiled_tokens_are_in_it() {
        let mut seen = std::collections::HashSet::new();
        for e in GS2LIB {
            assert_eq!(e.token.len(), 8, "{}", e.token);
            assert!(seen.insert(e.token), "{} twice", e.token);
        }
        // Every default-listed build's token is a stock entry.
        for v in ClientVersion::reborn_versions() {
            let i = gs2lib_index(v.token()).unwrap_or_else(|| panic!("{}", v.label()));
            assert!(GS2LIB[i].stock, "{}", v.label());
        }
    }

    /// Graal the Adventure, 2026-10-29: `Allowed: 2.22, 5.07`.
    #[test]
    fn adventure_list_is_two_newmain_builds_so_no_question() {
        let l = AllowedList::from_disconnect(
            "Your client version is not allowed on this server.\rAllowed: 2.22, 5.07",
        )
        .unwrap();
        assert_eq!(l.unknown().count(), 0);
        assert_eq!(l.admits("GNW03014"), Admit::Named);
        assert_eq!(l.admits("G3D22067"), Admit::Named);
        assert_eq!(l.admits("G3D14097"), Admit::No);
        // 2.22 and 5.00r7 are both NewMain: the newest wins.
        assert_eq!(pick(&l, &Need::default()), Pick::Auto(V500R7));
        // Once 5.00r7 fails there, 2.22.
        let need = Need {
            refused: vec![V500R7],
            ..Need::default()
        };
        assert_eq!(pick(&l, &need), Pick::Auto(V222));
    }

    /// "S2", 2026-10-30: `Allowed: 2.22, 1.41r1 - iPhone Belote 1.5`.
    #[test]
    fn s2_range_runs_across_eras_so_the_newest_wins() {
        let l = AllowedList::from_disconnect(
            "Your client version is not allowed on this server.\nAllowed: 2.22, 1.41r1 - iPhone Belote 1.5",
        )
        .unwrap();
        assert_eq!(l.admits("G3D0311C"), Admit::InRange);
        // 3.153 is Seihai's own gs2lib entry: unsure on a stock server.
        assert_eq!(l.admits("G3D28015"), Admit::Unsure);
        // 1.39 is below the range.
        assert_eq!(l.admits("GNW07080"), Admit::No);
        assert_eq!(pick(&l, &Need::default()), Pick::Auto(V603R7));
    }

    /// The case that needs the player: 1.41 and 5.x/6.x, nothing between.
    #[test]
    fn classic_and_modern_with_a_gap_asks() {
        let l = AllowedList::parse("1.41r1, 5.12 - 6.037");
        match pick(&l, &Need::default()) {
            Pick::Ask {
                options,
                recommended,
            } => {
                assert_eq!(options, [V603R7, V141]);
                assert_eq!(recommended, V603R7);
            }
            p => panic!("{p:?}"),
        }
    }

    /// A gap inside one era is not a question.
    #[test]
    fn a_gap_inside_one_era_is_not_asked() {
        let l = AllowedList::parse("2.05r2, 2.22");
        assert_eq!(pick(&l, &Need::default()), Pick::Auto(V222));
    }

    #[test]
    fn a_3d_server_prefers_a_3d_build() {
        let l = AllowedList::parse("3.041 - 6.037");
        let need = Need {
            three_d: true,
            ..Need::default()
        };
        let best = match pick(&l, &need) {
            Pick::Auto(v) => v,
            p => panic!("{p:?}"),
        };
        assert!(best.profile().three_d.is_some(), "{}", best.label());
        assert!(first_try(&need).profile().three_d.is_some());
    }

    /// GServer-v2's default `[generation-range]` lines, as it prints them.
    #[test]
    fn generation_range_lines() {
        // classic = GNW13110
        assert_eq!(
            pick(&AllowedList::parse("1.41r1"), &Need::default()),
            Pick::Auto(V141)
        );
        // newmain = GNW24011:GNW28015, G3D16053:G3D28015
        let l = AllowedList::parse("2.0 - 2.31, 3.0 - 3.153");
        assert_eq!(pick(&l, &Need::default()), Pick::Auto(V304));
        // modern = G3D14097:GNP1905C
        let l = AllowedList::parse("5.12 - 6.1.1.3");
        assert_eq!(pick(&l, &Need::default()), Pick::Auto(V603R7));
    }

    #[test]
    fn unknown_names_and_other_messages() {
        let l = AllowedList::parse("9.99, 2.22");
        assert_eq!(l.unknown().collect::<Vec<_>>(), ["9.99"]);
        assert_eq!(pick(&l, &Need::default()), Pick::Auto(V222));
        assert_eq!(
            pick(&AllowedList::parse("9.99"), &Need::default()),
            Pick::Nothing
        );
        assert!(AllowedList::from_disconnect("Account is already in use.").is_none());
        // A bare token, as an old server with no name for it prints.
        assert_eq!(
            AllowedList::parse("GNW03014").admits("GNW03014"),
            Admit::Named
        );
    }

    #[test]
    fn first_try_is_the_newest_listed_build() {
        assert_eq!(first_try(&Need::default()), V603R7);
        let need = Need {
            refused: vec![V603R7],
            ..Need::default()
        };
        assert_eq!(first_try(&need), V531R4);
    }
}
