//! Each client version scripts with its own build's GS1 command table
//! (Graal_analysis `npc-library/gs1-commands-by-version.tsv`). Early builds
//! no longer borrow the 2.00 set, and discovery borrowing (the 2.05
//! listserver) does not change the session's script era.

use seihai_core::protocol::{ClientVersion, EncryptionGen};
use seihai_core::scripting::engine::host::SessionScripts;
use seihai_core::scripting::engine::{parse, vocab, Era};

#[test]
fn early_sessions_use_their_own_table_without_modern_syntax() {
    use ClientVersion::*;
    for (version, era) in [
        (V121, Era::V121),
        (V128B2, Era::V128B2),
        (V13T, Era::V13T),
        (V131, Era::V13T),
        (V132, Era::V13T),
    ] {
        let scripts = SessionScripts::new(version);
        assert_eq!(scripts.world.era(), era, "{version:?}");
        assert!(vocab::builtin_fn("strlen", era).is_none(), "{version:?}");
        assert!(vocab::command_in("tokenize2", era).is_none(), "{version:?}");
        assert!(vocab::command_in("say2", era).is_none(), "{version:?}");
        assert!(
            !parse("if (created) { this.a = {1, 2}; }", era)
                .diagnostics
                .is_empty(),
            "{version:?} must not inherit the 2.1 array grammar"
        );
    }
    // 1.21 has `putnpc` and `for` but not `sleep`, `while` or `function`.
    let era = Era::for_version(V121);
    assert!(vocab::command_in("putnpc", era).is_some());
    assert!(vocab::command_in("sleep", era).is_none());
    assert!(parse("for (i=0;i<2;i++) hide;", era).diagnostics.is_empty());
    assert!(!parse("while (1) hide;", era).diagnostics.is_empty());
    assert!(!parse("function f() { hide; }", Era::for_version(V139))
        .diagnostics
        .is_empty());
    assert!(parse("function f() { hide; }", Era::for_version(V141))
        .diagnostics
        .is_empty());
}

#[test]
fn discovery_borrowing_does_not_change_early_game_connection_profiles() {
    use ClientVersion::*;
    for version in [V121, V128B2, V13T, V131, V132] {
        assert_eq!(version.list_version(), V205, "{version:?}");
        assert!(
            version.profile().protocol.listserver.is_none(),
            "{version:?}"
        );
        assert_eq!(version.encryption(), EncryptionGen::Gen2, "{version:?}");
        assert_eq!(version.profile().version, version);
        assert_eq!(
            SessionScripts::new(version).world.era(),
            Era::for_version(version)
        );
        assert!(Era::for_version(version) < Era::V200);
    }
}

#[test]
fn later_script_era_boundaries_stay_distinct() {
    use ClientVersion::*;
    for (version, expected) in [
        (V133, Era::V133),
        (V139, Era::V139),
        (V141, Era::V141),
        (V142, Era::V142),
        (V200, Era::V200),
        (V201, Era::V201),
        (V202, Era::V202),
        (V202R2, Era::V202),
        (V203, Era::V203),
        (V204, Era::V204),
        (V205, Era::V205),
        (V210, Era::V21),
        (V214, Era::V212),
        (V215, Era::V215),
        (V216, Era::V216),
        (V217, Era::V217),
        (V222, Era::V217),
        (V231, Era::V217),
        (V304, Era::V304),
        (V3153, Era::V3153),
        (V420R8, Era::V3153),
        (V500R7, Era::V500),
        (V603R7Linux, Era::V500),
    ] {
        assert_eq!(
            SessionScripts::new(version).world.era(),
            expected,
            "{version:?}"
        );
    }
    assert!(
        parse("if (created) { this.a = {1, 2}; }", Era::for_version(V210))
            .diagnostics
            .is_empty()
    );
}

/// The 2.17.1 to 2.31 commands 2.2x worlds use parse in 2.22 and are
/// skipped in 2.05.
#[test]
fn the_2_17_commands_reach_2_2x_clients_only() {
    use ClientVersion::*;
    let src = "if (created) { showimg2 1,a.png,1,2,3; putnpc2 1,2,{hide;}; showtext2 1,1,2,3,Arial,b,hi; wraptext 10,\" \",#s(a); with (getnpc(x)) { hide; } }";
    let new = parse(src, Era::for_version(V222));
    assert!(new.diagnostics.is_empty(), "{:?}", new.diagnostics);
    let old = Era::for_version(V205);
    for c in ["showimg2", "putnpc2", "showtext2", "wraptext"] {
        assert!(vocab::command_in(c, old).is_none(), "{c}");
        assert!(
            vocab::command_in(c, Era::for_version(V222)).is_some(),
            "{c}"
        );
    }
    assert!(!parse(src, old).diagnostics.is_empty());
    // 3.153 and later engines dropped `with`.
    assert!(!parse(src, Era::for_version(V3153)).diagnostics.is_empty());
}
