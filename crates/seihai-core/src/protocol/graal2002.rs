//! Static command registrations and explicit Windows 2.1 receive metadata.
//!
//! Source: Graal_analysis `client-internals/graal2002-and-140-builds.md`
//! (2026-11-13, CLIENT-HUNT-20261113). These are command registrations read
//! from binaries, not execution or wire-compatibility results. Select a
//! build explicitly: neither its token nor FileVersion identifies it.
//!
//! This module does not create an online [`super::ClientVersion`] profile,
//! change the inferred GS1 eras, or select parser/interpreter semantics.

use super::IdSet;
use crate::scripting::engine::vocab::{command_in, keyword_in, Era, Exec};

/// Exact identity for the statically read Windows 2.1 receive dispatcher.
///
/// Select from explicit build metadata, never a shared login token or
/// FileVersion. This is a receive binding override, not a complete online
/// profile, a sender contract, or a native execution result.
/// Source: Graal_analysis by squirelworks (CC BY 4.0), pinned study:
/// https://github.com/squirelworks/Graal_analysis/blob/1880eed52d3cc30dd31ac1892a6d306425e69d68/client-internals/graal2002-and-140-builds.md
/// https://creativecommons.org/licenses/by/4.0/
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Graal210ReceiveBuild {
    Public20011113,
    World(Graal2002Build),
}

impl Graal210ReceiveBuild {
    /// Stable explicit-selection label; not inferred from a token.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Public20011113 => "public-2001-11-13",
            Self::World(Graal2002Build::Ver7) => "ver7",
            Self::World(Graal2002Build::Ver8) => "ver8",
            Self::World(Graal2002Build::Ver9) => "ver9",
            Self::World(Graal2002Build::Ver10) => "ver10",
            Self::World(Graal2002Build::Ver13) => "ver13",
            Self::World(Graal2002Build::V22) => "v22",
        }
    }

    /// IDs with non-default receive destinations, not proof of payload
    /// compatibility, native delivery, or a client send of the same ID.
    /// Complete sets are checked against the pinned published matrix.
    pub const fn bound_ids(self) -> IdSet {
        use Graal2002Build::*;
        let base = IdSet::range(0, 25)
            .with_range(27, 49)
            .with(75)
            .with(100)
            .with_range(150, 151)
            .with_range(153, 156)
            .with_range(165, 166)
            .with_range(168, 169);
        match self {
            Self::Public20011113 => base.with_range(170, 174),
            Self::World(V22) => base.with_range(170, 177),
            Self::World(build) => {
                let lean = base.without_all(&[2, 4, 17, 18, 154, 155]);
                match build {
                    Ver7 => lean,
                    Ver8 | Ver9 => lean.with(170),
                    Ver10 | Ver13 => lean.with_range(170, 174),
                    V22 => unreachable!(),
                }
            }
        }
    }
}

/// An explicitly identified Windows Graal 2002 world build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Graal2002Build {
    Ver7,
    Ver8,
    Ver9,
    Ver10,
    Ver13,
    V22,
}

/// Binary-derived identity metadata, without an inferred protocol profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Graal2002Descriptor {
    pub label: &'static str,
    pub file_version: &'static str,
    /// Character-immediate token read statically; not a captured login.
    pub token: &'static str,
    /// Full scanner table count, including script keywords and aliases.
    /// This is not the number of commands implemented by Seihai.
    pub gs1_table_entries: u16,
}

impl Graal2002Build {
    /// All six builds, in the order of the published static study.
    pub const ALL: [Self; 6] = [
        Self::Ver7,
        Self::Ver8,
        Self::Ver9,
        Self::Ver10,
        Self::Ver13,
        Self::V22,
    ];

    pub fn descriptor(self) -> Graal2002Descriptor {
        let (label, file_version, token, gs1_table_entries) = match self {
            Self::Ver7 => ("Graal 2002 ver7", "2.1.0.0", "GNW03081", 167),
            Self::Ver8 => ("Graal 2002 ver8", "2.1.0.0", "GNW03081", 167),
            Self::Ver9 => ("Graal 2002 ver9", "2.1.0.0", "GNW03081", 168),
            Self::Ver10 => ("Graal 2002 ver10", "2.1.0.2", "GNW31101", 169),
            Self::Ver13 => ("Graal 2002 ver13", "2.1.0.4", "GNW31101", 169),
            Self::V22 => ("Graal 2002 v22", "2.1.0.4", "GNW31101", 174),
        };
        Graal2002Descriptor {
            label,
            file_version,
            token,
            gs1_table_entries,
        }
    }

    /// Whether a lower-case word is in this build's GS1 registration table.
    ///
    /// Includes keywords and aliases, excludes Seihai extensions. The
    /// published scanner was rerun on all six builds and the 2.05 baseline:
    /// after normalizing trailing-space markers, there are no removals and
    /// only the additions below (2026-11-15 static corroboration).
    /// Membership does not establish argument semantics, an executor, or
    /// successful effects on an original client. This is not a parser.
    pub fn gs1_word_registered(self, name: &str) -> bool {
        use Graal2002Build::*;
        match name {
            "play2" | "shoot" | "addstring" | "deletestring" | "insertstring" | "removestring" => {
                true
            }
            "replacestring" => matches!(self, Ver9 | Ver10 | Ver13 | V22),
            "savelog" => matches!(self, Ver10 | Ver13 | V22),
            "setcursor" | "setcursor2" | "setshootparams" | "wraptext" | "wraptext2" => self == V22,
            _ => {
                command_in(name, Era::V205).is_some_and(|c| c.exec != Exec::Ext)
                    || keyword_in(name, Era::V205)
            }
        }
    }
}
