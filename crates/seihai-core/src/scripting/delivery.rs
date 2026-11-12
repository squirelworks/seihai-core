//! One GS1 script, sent to each client in the form that client runs
//! (seihai-core#6, seihai-gserver#45).
//!
//! A server owner writes a client script once, in GS1. [`Prepared`] holds
//! that text and its GS2 bytecode, compiled once. [`Prepared::for_client`]
//! picks, for one client build, what to send:
//!
//! 1. **GS1 text** when the build's profile says it runs GS1
//!    ([`Scripts::gs1`] is `yes`, 1.21 to 2.31) and the script parses in
//!    that build's GS1 vocabulary ([`Era::for_version`]).
//! 2. **GS2 bytecode** when the profile says it runs GS2 (`gs2` is `yes`:
//!    the binary handles `PLO_NPCBYTECODE`, 3.153 on) and the script
//!    compiled.
//! 3. **GS1 text, unconfirmed** when the profile does not know whether the
//!    build runs GS1 and does not say it runs GS2 (2.1 to 2.16, 3.04, and
//!    builds with no binary), and the script parses in its vocabulary.
//! 4. Otherwise a **stub**: the client gets the NPC without a script, and
//!    [`Delivery::Stub`] says why.
//!
//! The order follows the profile's evidence, never a version number: the
//! profile is the one place that says which builds run which engine
//! (`documentation/version-chain.md`, "Script engines"). Only rule 1 and
//! rule 2 are backed by a binary read. No original client has run bytecode
//! from this compiler yet, so [`Choice::confirmed`] means "the build has
//! the engine", not "this output was seen running there".

use super::engine::{self, Era};
use super::gs2::compile::{compile, CompileErrors, Language, Mode};
use crate::protocol::{ClientVersion, Runs};

/// A GS1 client script, ready to be sent to any client.
#[derive(Debug, Clone)]
pub struct Prepared {
    source: String,
    bytecode: Result<Vec<u8>, CompileErrors>,
}

/// What one client is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery<'a> {
    /// The GS1 source, as the NPC's script prop.
    Gs1Text(&'a str),
    /// The compiled script, as `PLO_NPCBYTECODE`.
    Gs2Bytecode(&'a [u8]),
    /// Nothing the client can run; the NPC goes out without a script.
    Stub(Why),
}

/// Why a client gets no script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The script uses GS1 newer than the build's vocabulary, and the
    /// build does not run GS2 (or the script did not compile).
    NewerGs1(Era),
    /// The build runs only GS2, and the script did not compile to it
    /// ([`Prepared::compile_errors`] has the reasons).
    NoBytecode,
    /// The profile says the build runs neither engine.
    NoEngine,
}

/// A [`Delivery`] and whether the profile's evidence backs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice<'a> {
    pub delivery: Delivery<'a>,
    /// The build's profile says `yes` for the engine chosen. False for
    /// rule 3 above, and for a stub.
    pub confirmed: bool,
}

impl Prepared {
    /// Compile `source`, written as GS1 of `era`, once. Strict mode: a
    /// server owner's script is compiled as written. A script that does
    /// not compile still goes out as GS1 text to the builds that read it.
    pub fn gs1(source: &str, era: Era) -> Prepared {
        Prepared {
            source: source.to_string(),
            bytecode: compile(source, Language::Gs1(era), Mode::Strict).map(|c| c.script.write()),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// The GS2 bytecode, when the script compiled.
    pub fn bytecode(&self) -> Option<&[u8]> {
        self.bytecode.as_deref().ok()
    }

    /// Why the script has no bytecode.
    pub fn compile_errors(&self) -> Option<&CompileErrors> {
        self.bytecode.as_ref().err()
    }

    /// What to send to a client of build `version`.
    pub fn for_client(&self, version: ClientVersion) -> Choice<'_> {
        let scripts = version.profile().scripts;
        let era = Era::for_version(version);
        let reads_gs1 = engine::parse(&self.source, era).diagnostics.is_empty();
        let text = Delivery::Gs1Text(&self.source);
        let choice = |delivery, confirmed| Choice {
            delivery,
            confirmed,
        };
        if scripts.gs1 == Runs::Yes && reads_gs1 {
            return choice(text, true);
        }
        if scripts.gs2 == Runs::Yes {
            if let Some(b) = self.bytecode() {
                return choice(Delivery::Gs2Bytecode(b), true);
            }
        }
        if scripts.gs1 == Runs::Unknown && scripts.gs2 != Runs::Yes && reads_gs1 {
            return choice(text, false);
        }
        let why = if scripts.gs1 != Runs::No && !reads_gs1 {
            Why::NewerGs1(era)
        } else if scripts.gs2 == Runs::Yes {
            Why::NoBytecode
        } else {
            Why::NoEngine
        };
        choice(Delivery::Stub(why), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ClientVersion::*;

    const DOOR: &str = "\
if (playerenters) {
  setimg door.png;
  this.open = 0;
}
if (playertouchsme && this.open == 0) {
  this.open = 1;
  setimg dooropen.png;
  play door.wav;
  timeout = 1;
}
if (timeout) {
  this.open = 0;
  setimg door.png;
}
";

    #[test]
    fn one_script_reaches_every_profiled_build() {
        let p = Prepared::gs1(DOOR, Era::V21);
        assert!(p.bytecode().is_some(), "{:?}", p.compile_errors());
        for &v in ClientVersion::all_versions() {
            let s = v.profile().scripts;
            let c = p.for_client(v);
            match c.delivery {
                Delivery::Gs1Text(t) => {
                    assert_eq!(t, DOOR);
                    assert_ne!(s.gs1, Runs::No, "{v:?}");
                    assert_eq!(c.confirmed, s.gs1 == Runs::Yes, "{v:?}");
                }
                Delivery::Gs2Bytecode(b) => {
                    assert_eq!(s.gs2, Runs::Yes, "{v:?}");
                    assert_ne!(s.gs1, Runs::Yes, "{v:?}: a GS1 build gets text");
                    assert!(c.confirmed);
                    assert_eq!(b, p.bytecode().unwrap());
                }
                Delivery::Stub(why) => panic!("{v:?} got a stub: {why:?}"),
            }
        }
    }

    #[test]
    fn the_engine_follows_the_profile() {
        let p = Prepared::gs1(DOOR, Era::V21);
        for v in [V139, V205, V222, V231] {
            assert!(
                matches!(p.for_client(v).delivery, Delivery::Gs1Text(_)),
                "{v:?}"
            );
        }
        for v in [V3153, V524R4, V603R7] {
            assert!(
                matches!(p.for_client(v).delivery, Delivery::Gs2Bytecode(_)),
                "{v:?}"
            );
        }
    }

    #[test]
    fn newer_gs1_is_a_stub_on_older_builds_and_bytecode_on_gs2_builds() {
        // `with` is 2.1 GS1: 2.05 cannot read it, 3.153 on runs the
        // bytecode.
        let src = "if (created) { with (getplayer(a)) { hide; } }\n";
        let p = Prepared::gs1(src, Era::V21);
        assert!(p.bytecode().is_some(), "{:?}", p.compile_errors());
        assert_eq!(
            p.for_client(V205).delivery,
            Delivery::Stub(Why::NewerGs1(Era::V205))
        );
        assert!(matches!(p.for_client(V231).delivery, Delivery::Gs1Text(_)));
        assert!(matches!(
            p.for_client(V603R7).delivery,
            Delivery::Gs2Bytecode(_)
        ));
    }

    #[test]
    fn a_script_without_bytecode_still_goes_to_gs1_builds() {
        let p = Prepared::gs1("if (created) { putnpc a.png,x.txt,1,2; }\n", Era::V205);
        assert!(p.compile_errors().is_some());
        assert!(matches!(p.for_client(V205).delivery, Delivery::Gs1Text(_)));
        assert_eq!(
            p.for_client(V603R7).delivery,
            Delivery::Stub(Why::NoBytecode)
        );
    }
}
