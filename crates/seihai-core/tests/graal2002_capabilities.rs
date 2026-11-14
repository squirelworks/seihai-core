use seihai_core::protocol::graal2002::Graal2002Build;
use seihai_core::protocol::ClientVersion;
use seihai_core::scripting::engine::vocab::{command_in, Era};

#[test]
fn complete_registered_name_counts_match_the_static_tables() {
    use seihai_core::scripting::engine::vocab::{COMMANDS, KEYWORDS};
    // Count names independently of the descriptor's reported table size.
    for (build, expected) in Graal2002Build::ALL
        .into_iter()
        .zip([167, 167, 168, 169, 169, 174])
    {
        let count = COMMANDS
            .iter()
            .map(|c| c.name)
            .chain(KEYWORDS.iter().map(|(name, _, _)| *name))
            .filter(|name| build.gs1_word_registered(name))
            .count();
        assert_eq!(count, expected, "{build:?}");
        assert_eq!(usize::from(build.descriptor().gs1_table_entries), expected);
        assert!(build.gs1_word_registered("if"));
        assert!(!build.gs1_word_registered("unknowncommand"));
        assert!(!build.gs1_word_registered("showimg2"));
    }
}

#[test]
fn shared_tokens_and_file_versions_do_not_erase_build_capabilities() {
    use Graal2002Build::*;
    let early = Ver7.descriptor();
    let replace = Ver9.descriptor();
    assert_eq!(early.token, replace.token);
    assert_eq!(early.file_version, replace.file_version);
    assert!(!Ver7.gs1_word_registered("replacestring"));
    assert!(!Ver8.gs1_word_registered("replacestring"));
    assert!(Ver9.gs1_word_registered("replacestring"));

    let before = Ver13.descriptor();
    let after = V22.descriptor();
    assert_eq!(before.token, after.token);
    assert_eq!(before.file_version, after.file_version);
    for name in [
        "setshootparams",
        "setcursor",
        "setcursor2",
        "wraptext",
        "wraptext2",
    ] {
        assert!(!Ver13.gs1_word_registered(name), "{name}");
        assert!(V22.gs1_word_registered(name), "{name}");
    }
    assert!(!Ver9.gs1_word_registered("savelog"));
    assert!(Ver10.gs1_word_registered("savelog"));
}

#[test]
fn registrations_do_not_promote_generic_eras() {
    for build in Graal2002Build::ALL {
        for name in [
            "play2",
            "shoot",
            "addstring",
            "deletestring",
            "insertstring",
            "removestring",
        ] {
            assert!(build.gs1_word_registered(name), "{build:?} {name}");
        }
        // Baseline names remain registered; Seihai extensions do not.
        assert!(build.gs1_word_registered("message"));
        assert!(!build.gs1_word_registered("sendtoserver"));
    }
    assert_eq!(Era::for_version(ClientVersion::V210), Era::V21);
    assert!(Era::V21.inferred());
    assert!(command_in("replacestring", Era::V21).is_none());
    assert!(command_in("savelog", Era::V21).is_none());
    assert!(command_in("setshootparams", Era::V21).is_none());
    assert!(command_in("replacestring", Era::V217).is_some());
}
