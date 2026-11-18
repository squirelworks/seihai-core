//! Level NPCs that mix GS1 event tests with GS2 event functions, and a
//! level with tile layers 1 and 2. The fixtures in `tests/fixtures/mixed`
//! are synthetic; each copies the shape of a pattern counted in the 2019
//! Delteria backup (see the README there).
//!
//! Seihai's reading of a mixed NPC (`compile::mixed` in the source): the
//! GS1 block and the GS2 function for one event are both event code, and
//! both run, the GS1 block first. No original client run has confirmed it.

use seihai_core::assets::{encode_binary, tile_id, BinaryLevel, Level, EMPTY_UPPER_TILE};
use seihai_core::scripting::engine::{Effect, Era, Val};
use seihai_core::scripting::gs2::compile::{self, Compiled, Front, Language, Mode, Quirk};
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};
use seihai_core::scripting::gs2::Script;
use std::path::Path;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mixed")
            .join(name),
    )
    .unwrap()
}

/// The server and client parts of an NPC script, split at `//#CLIENTSIDE`.
fn parts(src: &str) -> (String, String) {
    match src.find("//#CLIENTSIDE") {
        Some(at) => {
            let client = &src[at + "//#CLIENTSIDE".len()..];
            (
                src[..at].to_string(),
                client.trim_start_matches(['\r', '\n']).to_string(),
            )
        }
        None => (src.to_string(), String::new()),
    }
}

/// An unmarked level NPC script, as a server holding both languages reads it.
fn detect(src: &str) -> Compiled {
    compile::compile(src, Language::Detect(Era::V217), Mode::Legacy)
        .unwrap_or_else(|e| panic!("{e}\n{src}"))
}

fn names(script: &Script) -> Vec<&str> {
    script.functions.iter().map(|f| f.name.as_str()).collect()
}

/// Load the script, then fire `events` in order, on a host at (10, 12).
fn run(script: Script, events: &[&str]) -> (Machine, EffectHost) {
    let mut machine = Machine::new(script);
    let mut host = EffectHost::new(Era::V217, 7);
    host.x = 10.0;
    host.y = 12.0;
    assert_eq!(machine.run_top_level(&mut host), Outcome::Done);
    for ev in events {
        assert_eq!(
            machine.call_event(ev, &[], &mut host),
            Outcome::Done,
            "{ev}: {}",
            machine.script().disassemble()
        );
    }
    (machine, host)
}

/// The commands the host was asked to run, in order.
fn commands(host: &EffectHost) -> Vec<String> {
    host.effects
        .iter()
        .filter_map(|e| match e {
            Effect::Command { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// A `this.` variable as a number.
fn num(machine: &Machine, name: &str) -> f64 {
    let name = name.strip_prefix("this.").unwrap_or(name);
    match machine.var(Root::This, name) {
        Some(Val::Num(n)) => *n,
        other => panic!("this.{name} is {other:?}"),
    }
}

#[test]
fn the_editor_stub_runs_both_parts_on_created() {
    let src = fixture("stub.txt");
    let got = detect(&src);
    assert_eq!(got.front, Front::Gs2);
    // One handler: the GS1 block folded into the GS2 function, not two
    // functions for one event.
    assert_eq!(names(&got.script), ["onCreated"]);
    assert!(got
        .warnings
        .iter()
        .any(|w| w.quirk == Quirk::Gs1EventTest("created".into())));
    let (_, host) = run(got.script, &["onCreated"]);
    // GS1 `dontblock;` first, then the function's `dontblock()`.
    assert_eq!(commands(&host), ["dontblock", "dontblock"]);
    assert_eq!(host.y, 12.5, "the leading-dot float moves the NPC");
}

#[test]
fn the_editor_stub_also_compiles_as_strict_gs1() {
    // What `delivery` does for a GS1 client script: the GS1 front end. Its
    // lifted `onCreated` and the script's own `function onCreated` (GS1
    // lower-cases the name) are one handler.
    let src = fixture("stub.txt");
    let script = compile::compile_gs1(&src, Era::V217).unwrap();
    assert_eq!(script.functions.len(), 1, "{}", script.disassemble());
    assert!(script.functions[0].name.eq_ignore_ascii_case("onCreated"));
    let (_, host) = run(script, &["onCreated"]);
    assert_eq!(commands(&host), ["dontblock", "dontblock"]);
    assert_eq!(host.y, 12.5);
}

#[test]
fn a_statement_between_the_parts_runs_once_at_load() {
    let got = detect(&fixture("stub-between.txt"));
    assert_eq!(names(&got.script), ["onCreated"]);
    let mut machine = Machine::new(got.script);
    let mut host = EffectHost::new(Era::V217, 7);
    host.y = 12.0;
    assert_eq!(machine.run_top_level(&mut host), Outcome::Done);
    assert_eq!(host.y, 12.5);
    assert!(commands(&host).is_empty());
    assert_eq!(
        machine.call_event("onCreated", &[], &mut host),
        Outcome::Done
    );
    assert_eq!(commands(&host), ["dontblock", "dontblock", "dontblock"]);
    assert_eq!(host.y, 12.5);
}

#[test]
fn a_gs1_server_part_and_a_mixed_client_part() {
    let (server, client) = parts(&fixture("lamp.txt"));

    let server = detect(&server);
    assert_eq!(server.front, Front::Gs1);
    assert_eq!(names(&server.script), ["onCreated"]);
    let (_, host) = run(server.script, &["onCreated"]);
    assert_eq!(
        host.effects,
        [
            Effect::Command {
                obj: 7,
                name: "setimg".into(),
                args: vec![Val::from("lamp_glow.png")],
            },
            Effect::Command {
                obj: 7,
                name: "dontblock".into(),
                args: vec![],
            },
        ]
    );

    let client = detect(&client);
    assert_eq!(client.front, Front::Gs2);
    let mut fns = names(&client.script);
    fns.sort_unstable();
    assert_eq!(fns, ["onCreated", "onPlayerEnters"]);
    let (_, host) = run(client.script, &["onCreated", "onPlayerEnters"]);
    assert_eq!(host.x, 11.0);
    let cmds = commands(&host);
    for want in ["setcoloreffect", "setzoomeffect", "drawaslight"] {
        assert!(cmds.iter().any(|c| c == want), "{want} not run: {cmds:?}");
    }
}

#[test]
fn an_inline_array_with_a_trailing_comma_reaches_the_call() {
    let (server, client) = parts(&fixture("shape.txt"));

    let server = detect(&server);
    assert_eq!(server.front, Front::Gs1);
    let (_, host) = run(server.script, &["onCreated"]);
    assert_eq!(host.x, 10.5);

    let client = detect(&client);
    assert_eq!(client.front, Front::Gs2);
    assert!(client
        .warnings
        .iter()
        .all(|w| !matches!(w.quirk, Quirk::Gs1EventTest(_))));
    let (_, host) = run(client.script, &["onCreated"]);
    let shape = host
        .effects
        .iter()
        .find_map(|e| match e {
            Effect::Command { name, args, .. } if name == "setshape2" => Some(args.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no setshape2: {:?} {:?}", host.effects, host.log));
    // Width, height, then the four tile types (the trailing comma adds no
    // fifth).
    let shown = format!("{shape:?}");
    assert_eq!(shown.matches("22").count(), 4, "{shown}");
}

#[test]
fn gs1_timeout_and_touch_tests_beside_gs2_action_handlers() {
    let (_, client) = parts(&fixture("timer.txt"));
    let got = detect(&client);
    assert_eq!(got.front, Front::Gs2);
    let mut fns = names(&got.script);
    fns.sort_unstable();
    assert_eq!(
        fns,
        ["onActionHit", "onCreated", "onPlayerTouchsMe", "onTimeout"]
    );
    let (machine, _) = run(
        got.script,
        &["onCreated", "onTimeout", "onPlayerTouchsMe", "onActionHit"],
    );
    // `if (timeout)` adds 1, then the GS2 `onTimeout` adds 10.
    assert_eq!(num(&machine, "this.ticks"), 11.0);
    assert_eq!(num(&machine, "this.touched"), 1.0);
    assert_eq!(num(&machine, "this.hits"), 1.0);
}

#[test]
fn a_gs1_block_that_returns_does_not_skip_the_gs2_handler() {
    let src = "\
if (created) {
  this.a = 1;
  if (this.stop) return;
  this.b = 1;
}
public function onCreated() {
  this.c = 1;
}
";
    let got = detect(src);
    let (machine, _) = run(got.script.clone(), &["onCreated"]);
    for v in ["this.a", "this.b", "this.c"] {
        assert_eq!(num(&machine, v), 1.0, "{v}: {}", got.script.disassemble());
    }
    // The GS1 block keeps its own function, so its `return` ends only it.
    assert_eq!(names(&got.script).len(), 2, "{:?}", names(&got.script));
}

#[test]
fn the_handler_name_matches_without_regard_to_case() {
    let got = detect("if (created) this.a = 1;\nfunction oncreated() { this.b = 1; }\n");
    assert_eq!(names(&got.script), ["oncreated"]);
    let (machine, _) = run(got.script, &["onCreated"]);
    assert_eq!(num(&machine, "this.a"), 1.0);
    assert_eq!(num(&machine, "this.b"), 1.0);
}

#[test]
fn a_script_marked_gs2_and_strict_mode_are_unchanged() {
    let src = fixture("stub.txt");
    // Marked GS2: the `if (created)` stays a variable test (issue 32), so
    // the script holds only the GS2 handler.
    let gs2 = compile::compile(&src, Language::Gs2, Mode::Legacy).unwrap();
    assert!(gs2
        .warnings
        .iter()
        .all(|w| !matches!(w.quirk, Quirk::Gs1EventTest(_))));
    assert!(gs2.script.strings.iter().any(|s| s == "created"));
    // Strict mode still needs the language.
    assert!(compile::compile(&src, Language::Detect(Era::V217), Mode::Strict).is_err());
}

// ---- tile layers ----

#[test]
fn a_level_with_sparse_layer_1_and_2_patches_reads_and_round_trips() {
    let level = Level::parse(&fixture("layers.nw")).unwrap();
    assert_eq!(level.tile_layers(), [0, 1, 2]);
    // Layer 0 is full; the patches sit where their rows say.
    assert_eq!(level.tile_id_on_layer(0, 63, 63), Some((63 + 63) % 4));
    assert_eq!(level.tile_id_on_layer(1, 25, 18), Some(1056));
    assert_eq!(level.tile_id_on_layer(1, 26, 19), Some(1073));
    assert_eq!(level.tile_id_on_layer(1, 22, 35), Some(2154));
    assert_eq!(level.tile_id_on_layer(2, 43, 34), Some(2176));
    // A cell no row covers is empty.
    assert_eq!(level.tile_id_on_layer(1, 0, 0), None);
    assert_eq!(level.tile_id_on_layer(2, 42, 34), None);
    // Layer 0 lookups ignore the upper layers.
    assert_eq!(level.tile_id_at(25, 18), Some((25 + 18) % 4));

    // GR-V1.05 fills every uncovered upper cell with 4095, and reads back
    // to the same tiles.
    let w = encode_binary(&level, BinaryLevel::Graal);
    assert!(w.warnings.is_empty(), "{:?}", w.warnings);
    assert_eq!(&w.bytes[..8], b"GR-V1.05");
    assert_eq!(w.bytes[8], 3 + 32);
    let back = Level::from_bytes(&w.bytes).unwrap();
    assert_eq!(back.layer0_board(), level.layer0_board());
    for layer in [1, 2] {
        for y in 0..64 {
            for x in 0..64 {
                assert_eq!(
                    back.tile_id_on_layer(layer, x, y),
                    level.tile_id_on_layer(layer, x, y),
                    "layer {layer} ({x}, {y})"
                );
            }
        }
    }
    let raw = back
        .boards
        .iter()
        .find(|r| r.layer == 1 && r.y == 0)
        .unwrap();
    assert!(raw.tiles.iter().all(|t| tile_id(*t) == EMPTY_UPPER_TILE));

    // The mixed NPCs in the level compile.
    assert_eq!(level.npcs.len(), 2);
    for npc in &level.npcs {
        let (server, client) = parts(&npc.script);
        detect(&server);
        if !client.trim().is_empty() {
            detect(&client);
        }
    }
}

#[test]
fn blank_upper_layers_written_in_full_read_as_empty() {
    // How an editor saves empty layers 1 and 2: every row, all `//`.
    let mut text = String::from("GLEVNW01\n");
    for y in 0..64 {
        text += &format!("BOARD 0 {y} 64 0 {}\n", "AB".repeat(64));
    }
    for layer in [1, 2] {
        for y in 0..64 {
            text += &format!("BOARD 0 {y} 64 {layer} {}\n", "//".repeat(64));
        }
    }
    let level = Level::parse(&text).unwrap();
    assert_eq!(level.tile_layers(), [0, 1, 2]);
    for layer in [1, 2] {
        for (x, y) in [(0, 0), (31, 40), (63, 63)] {
            assert_eq!(level.tile_id_on_layer(layer, x, y), None);
        }
    }
    assert_eq!(level.tile_id_on_layer(0, 5, 5), Some(1));

    // The 4095s go through GR-V1.05 unchanged.
    let w = encode_binary(&level, BinaryLevel::Graal);
    assert_eq!(&w.bytes[..8], b"GR-V1.05");
    let back = Level::from_bytes(&w.bytes).unwrap();
    for layer in [1, 2] {
        assert!(back
            .boards
            .iter()
            .filter(|r| r.layer == layer)
            .flat_map(|r| &r.tiles)
            .all(|t| tile_id(*t) == EMPTY_UPPER_TILE));
    }
    assert_eq!(back.tile_id_on_layer(2, 10, 10), None);
}
