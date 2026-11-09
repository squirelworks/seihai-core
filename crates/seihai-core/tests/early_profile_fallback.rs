//! The documented Seihai fallback is not a native early-client vocabulary claim.

use seihai_core::protocol::{ClientVersion, EncryptionGen};
use seihai_core::scripting::engine::host::SessionScripts;
use seihai_core::scripting::engine::{parse, vocab, Era};

#[test]
fn early_sessions_use_the_documented_script_fallback_without_modern_syntax() {
    use ClientVersion::*;
    for version in [V121, V128B2, V13T, V131, V132] {
        let scripts = SessionScripts::new(version);
        assert_eq!(scripts.world.era(), Era::V200, "{version:?}");
        let era = Era::for_version(version);
        assert!(vocab::builtin_fn("strlen", era).is_none(), "{version:?}");
        assert!(vocab::command_in("tokenize2", era).is_none(), "{version:?}");
        assert!(
            !parse("if (created) { this.a = {1, 2}; }", era)
                .diagnostics
                .is_empty(),
            "{version:?} must not inherit the 2.1 array grammar"
        );
    }
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
        assert_eq!(SessionScripts::new(version).world.era(), Era::V200);
    }
}

#[test]
fn later_script_era_boundaries_stay_distinct() {
    use ClientVersion::*;
    for (version, expected) in [
        (V133, Era::V200),
        (V200, Era::V200),
        (V201, Era::V201),
        (V202, Era::V202),
        (V202R2, Era::V202),
        (V203, Era::V203),
        (V204, Era::V204),
        (V205, Era::V205),
        (V210, Era::V21),
        (V222, Era::V21),
        (V304, Era::V21),
        (V3153, Era::V21),
        (V603R7Linux, Era::V21),
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
