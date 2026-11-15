use seihai_core::protocol::graal2002::{Graal2002Build, Graal210ReceiveBuild};
use seihai_core::protocol::{ClientVersion, Message, SessionState};

fn public_session() -> SessionState {
    let mut s = SessionState::for_version(ClientVersion::V210);
    s.set_graal210_receive_build(Graal210ReceiveBuild::Public20011113);
    s
}

fn apply(s: &mut SessionState, id: u8, payload: &[u8]) {
    s.apply(&Message {
        id,
        payload: payload.to_vec(),
    });
}

#[test]
fn public_short_flags_preserve_state_and_below_bias_is_false() {
    let mut s = public_session();
    for id in [170, 174] {
        apply(&mut s, id, &[33]);
        apply(&mut s, id, b"");
        assert!(if id == 170 {
            s.ghost_mode
        } else {
            s.ghost_icon
        });
        for (byte, want) in [
            (32, false),
            (33, true),
            (34, true),
            (255, true),
            (31, false),
            (0, false),
        ] {
            apply(&mut s, id, &[byte, 255]);
            assert_eq!(
                if id == 170 {
                    s.ghost_mode
                } else {
                    s.ghost_icon
                },
                want
            );
        }
    }
}

#[test]
fn public_map_coordinates_are_optional_and_independent() {
    let mut s = public_session();
    for id in [171, 172] {
        for (body, xy) in [
            ("map.txt,map.png", (0.0, 0.0)),
            ("map.txt,map.png,9", (0.0, 0.0)),
            ("map.txt,map.png,bad,4.5", (0.0, 4.5)),
            ("map.txt,map.png,2.5,bad", (2.5, 0.0)),
        ] {
            apply(&mut s, id, body.as_bytes());
            let map = if id == 171 { &s.bigmap } else { &s.minimap };
            let map = map
                .as_ref()
                .expect("two fields are sufficient in public 2.1");
            assert_eq!((&*map.txt, &*map.png), ("map.txt", "map.png"));
            assert_eq!((map.x, map.y), xy);
        }
    }
}

#[test]
fn complete_bound_sets_match_the_pinned_matrix() {
    let builds = Graal2002Build::ALL
        .into_iter()
        .map(Graal210ReceiveBuild::World)
        .chain([Graal210ReceiveBuild::Public20011113]);
    let rows = include_str!("fixtures/graal210-receive.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 7);
    for (build, row) in builds.zip(rows) {
        let fields = row.split('\t').collect::<Vec<_>>();
        let mut expected = Vec::new();
        for span in fields[3].split(',') {
            let (lo, hi) = span.split_once('-').unwrap_or((span, span));
            expected.extend(lo.parse::<u8>().unwrap()..=hi.parse::<u8>().unwrap());
        }
        assert_eq!(expected.len(), fields[2].parse::<usize>().unwrap());
        assert_eq!(
            build.bound_ids().iter().collect::<Vec<_>>(),
            expected,
            "{}",
            build.label()
        );
    }
}

#[test]
fn explicit_metadata_changes_receive_effects_despite_shared_token() {
    use Graal2002Build::*;
    assert_eq!(Ver10.descriptor().token, V22.descriptor().token);
    for (build, chest, mode) in [
        (Graal210ReceiveBuild::World(Ver10), false, false),
        (Graal210ReceiveBuild::Public20011113, true, false),
        (Graal210ReceiveBuild::World(V22), true, true),
    ] {
        let mut s = SessionState::for_version(ClientVersion::V210);
        s.set_graal210_receive_build(build);
        apply(&mut s, 4, b"abc");
        apply(&mut s, 176, b"");
        assert_eq!(!s.chests.is_empty(), chest, "{}", build.label());
        assert_eq!(s.server_mode.is_some(), mode, "{}", build.label());
        assert_eq!(s.handles(177), mode);
        assert!(!s.handles(178));
        // A raw file block is still accepted behind RAWDATA (100), not
        // mistaken for a directly bound ID in the native dispatcher.
        assert!(s.handles(101) && s.handles(102));
    }
}

#[test]
fn public_raw_text_and_existing_generic_readers_remain_separate() {
    let mut public = public_session();
    apply(&mut public, 173, b"ghost");
    apply(&mut public, 173, b"");
    assert_eq!(public.ghost_text.as_deref(), Some(""));
    let mut generic = SessionState::for_version(ClientVersion::V210);
    let before = generic.known_plo;
    apply(&mut generic, 171, b"map.txt,map.png");
    assert!(generic.bigmap.is_none());
    assert_eq!(generic.known_plo, before);
    let mut later = SessionState::for_version(ClientVersion::V222);
    apply(&mut later, 171, b"map.txt,map.png");
    assert!(later.bigmap.is_none());
    apply(&mut later, 171, b"map.txt,map.png,1,2");
    assert_eq!(later.bigmap.unwrap().x, 1.0);
    let mut middle = SessionState::for_version(ClientVersion::V210);
    middle.set_graal210_receive_build(Graal210ReceiveBuild::World(Graal2002Build::Ver10));
    apply(&mut middle, 171, b"map.txt,map.png");
    assert!(
        middle.bigmap.is_none(),
        "binding alone does not select the public layout"
    );
}
