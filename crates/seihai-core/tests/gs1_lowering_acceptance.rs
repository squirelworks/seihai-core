//! seihai-core#6: acceptance tests for the GS1 lowering of flags, `this.`
//! and `client.` variables, the timer, and the player-property commands.
//!
//! Byte-comparison controls compile GS1 next to the GS2 a scripter would
//! write and require the same bytes. Where the GS2 spelling is
//! fixture-checked (`tests/fixtures/gs2`), the test says which fixture;
//! the rest are Seihai's reading of GS2 and say so. Then the bytecode is
//! run on Seihai's GS2 machine to see it do what the GS1 did. No original
//! client is involved. Value/effect-only controls use GS1 compilation directly.

use seihai_core::scripting::engine::{Effect, Era, Val};
use seihai_core::scripting::gs2::compile::{
    self, gs1_command_lowering, gs1_commands_without_gs2, Gs1Lowering,
};
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};
use seihai_core::scripting::gs2::Script;

fn gs1(src: &str) -> Script {
    compile::compile_gs1(src, Era::V217).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

fn gs2(src: &str) -> Script {
    compile::compile_gs2(src).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

fn same(gs1_src: &str, gs2_src: &str) -> Script {
    let a = gs1(gs1_src);
    let b = gs2(gs2_src);
    assert_eq!(
        a.disassemble(),
        b.disassemble(),
        "GS1:\n{gs1_src}\nGS2:\n{gs2_src}"
    );
    assert_eq!(a.write(), b.write());
    a
}

/// Like `same`, for forms whose GS2 spelling now compiles to the official
/// shape while the GS1 lowering keeps its older one: the GS1 side may
/// carry an extra `OP_CMD_CALL` (it counts the lowered command as a call)
/// and `OP_CONV_TO_STRING` on a text receiver. Everything else must match.
fn same_but_gs1_extras(gs1_src: &str, gs2_src: &str) -> Script {
    let a = gs1(gs1_src);
    let b = gs2(gs2_src);
    let ops = |s: &Script| -> Vec<String> {
        s.code
            .iter()
            .map(|i| format!("{} {:?}", i.op, i.operand))
            .collect()
    };
    let drop = |v: Vec<String>| -> Vec<String> {
        v.into_iter()
            .filter(|o| !o.starts_with("OP_CMD_CALL ") && !o.starts_with("OP_CONV_TO_STRING "))
            .filter(|o| !o.starts_with("OP_SET_INDEX "))
            .collect()
    };
    assert_eq!(
        drop(ops(&a)),
        drop(ops(&b)),
        "GS1:\n{gs1_src}\nGS2:\n{gs2_src}"
    );
    a
}

fn created(script: Script) -> (Machine, EffectHost) {
    let mut m = Machine::new(script);
    let mut host = EffectHost::new(Era::V217, 3);
    assert_eq!(m.run_top_level(&mut host), Outcome::Done);
    assert_eq!(m.call_event("onCreated", &[], &mut host), Outcome::Done);
    (m, host)
}

#[test]
fn flag_writes_are_client_variable_assignments() {
    // A real 5.24r4 and 4.20r8 have no GS2 `set()`/`unset()` and send
    // PLI 18/19 for an assignment to a `client.` variable: `= true` as
    // `name=1`, `= ""` as a clear (lab, 2026-11-14, #30). A plain name is
    // sent as `client.gs1_<flag>`, which seihai-gserver keeps as `<flag>`.
    let s = same(
        "if (created) {\n  set gotkey;\n  setstring door,open;\n  unset gotkey;\n  setstring door,;\n  set client.seen;\n  set quest.started;\n  set mood=calm;\n}\n",
        "function onCreated() {\n  client.gs1_gotkey = true;\n  client.gs1_door = \"open\";\n  client.gs1_gotkey = \"\";\n  client.gs1_door = \"\";\n  client.seen = true;\n  client.gs1_quest.started = true;\n  client.gs1_mood = \"calm\";\n}\n",
    );
    let (_, host) = created(s);
    let flags: Vec<_> = host
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::SetFlag { name, value } => Some(format!("set {name}={value:?}")),
            Effect::UnsetFlag { name } => Some(format!("unset {name}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        flags,
        [
            "set client.gs1_gotkey=Some(\"1\")",
            "set client.gs1_door=Some(\"open\")",
            "unset client.gs1_gotkey",
            "unset client.gs1_door",
            "set client.seen=Some(\"1\")",
            "set client.gs1_quest.started=Some(\"1\")",
            "set client.gs1_mood=Some(\"calm\")"
        ]
    );
    // Flags a client script cannot send, and names made at run time.
    for src in ["set local.x;", "unset server.x;", "set #s(a);"] {
        let err = compile::compile_gs1(&format!("if (created) {{ {src} }}\n"), Era::V217)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not lowered") || err.contains("plain flag"),
            "{src}: {err}"
        );
    }
}

#[test]
fn reading_a_flag_by_its_plain_name_reads_the_player_flag() {
    // GS1 reads `gotkey` as the flag; a real 5.24r4 client reads a server
    // flag as `player.gotkey` (lab probe, 2026-11-13).
    same(
        "if (playerenters) { set gotkey; }\nif (playertouchsme && gotkey) { hide; }\n",
        "function onPlayerEnters() { client.gs1_gotkey = true; }\nfunction onPlayerTouchsMe() { if (player.gotkey) { hide(); } }\n",
    );
    // Writing it by its plain name is still refused.
    let err = compile::compile_gs1("if (created) { set gotkey; gotkey = 2; }\n", Era::V217)
        .unwrap_err()
        .to_string();
    assert!(err.contains("player flag"), "{err}");
    // A plain variable the script never sets as a flag still lowers.
    gs1("if (created) { count = 1; }\n");
}

#[test]
fn a_plain_string_code_reads_the_player_flag() {
    // `#s(door)` reads the string flag `door`, which `setstring` sends as
    // `client.gs1_door` and the server keeps as `door`.
    let dis = compile::compile_gs1(
        "if (created) { setstring door,open; message #s(door); }\n",
        Era::V217,
    )
    .unwrap()
    .disassemble();
    assert!(
        dis.contains("OP_PLAYER\n   13  OP_TYPE_VAR #3 \"door\"\n   14  OP_MEMBER_ACCESS"),
        "{dis}"
    );
    // `local.` and `server.` strings, and a name built at run time, stay refused.
    for src in [
        "message #s(local.a);",
        "message #s(server.a);",
        "message #s(w#v(1));",
    ] {
        assert!(
            compile::compile_gs1(&format!("if (created) {{ {src} }}\n"), Era::V217).is_err(),
            "{src}"
        );
    }
}

#[test]
fn this_and_client_variables_keep_their_names() {
    let s = same(
        "if (created) {\n  this.hp = 3;\n  client.score = 2;\n  client.score += 5;\n  setstring this.name,bob;\n  setstring client.k,v;\n}\n",
        "function onCreated() {\n  this.hp = 3;\n  client.score = 2;\n  client.score += 5;\n  this.name = \"bob\";\n  client.k = \"v\";\n}\n",
    );
    let (m, _) = created(s);
    assert_eq!(m.var(Root::This, "hp"), Some(&Val::Num(3.0)));
    assert_eq!(m.var(Root::This, "name"), Some(&Val::Str("bob".into())));
}

#[test]
fn the_timer_is_this_timeout() {
    // `this.timeout` read and written, and `onTimeout`: `timers.gs2`.
    let s = same(
        "if (created) { timeout = 0.1; }\nif (timeout) {\n  this.ticks++;\n  if (this.ticks < 3) timeout = 0.05;\n}\n",
        "function onCreated() { this.timeout = 0.1; }\nfunction onTimeout() {\n  this.ticks++;\n  if (this.ticks < 3) this.timeout = 0.05;\n}\n",
    );
    let (mut m, mut host) = created(s);
    assert!(m.timeout() > 0.0);
    for _ in 0..40 {
        m.tick(&mut host);
    }
    assert_eq!(m.var(Root::This, "ticks"), Some(&Val::Num(3.0)));
    assert_eq!(m.timeout(), 0.0);
}

#[test]
fn player_properties_lower_to_the_player_object() {
    // `setplayerprop #c` is `player.chat = ...`: the real 6.03r7 has no
    // `setplayerprop` function (isolated lab, 2026-11-14). The `player.` names
    // are Seihai's reading, not a fixture.
    let s = same(
        "if (playerenters) {\n  setplayerprop #c,hello;\n  playerdir = 2;\n  playerrupees += 5;\n  this.h = playerhearts;\n  this.s = playerswordpower + playershieldpower;\n  this.b = playerbombs + playerdarts;\n  this.p = playerx + playery;\n}\n",
        "function onPlayerEnters() {\n  player.chat = \"hello\";\n  player.dir = 2;\n  player.rupees += 5;\n  this.h = player.hearts;\n  this.s = player.swordpower + player.shieldpower;\n  this.b = player.bombs + player.darts;\n  this.p = player.x + player.y;\n}\n",
    );
    let mut m = Machine::new(s);
    let mut host = EffectHost::new(Era::V217, 3);
    m.run_top_level(&mut host);
    assert_eq!(
        m.call_event("onPlayerEnters", &[], &mut host),
        Outcome::Done
    );
    assert!(host.effects.iter().any(|e| matches!(
        e,
        Effect::SetPlayerProp { code, value } if code == "#c" && value == "hello"
    )));
}

#[test]
fn setbody_lowers_to_player_bodyimg_effect() {
    // `setbody` is gone from the C++ GS1 table; GS2 keeps `sethead` only.
    // Lowering matches `setplayerprop #8` → `player.bodyimg`.
    let s = same(
        "if (created) { setbody body13.png; setplayerprop #3,head5.png; }\n",
        "function onCreated() {\n  player.bodyimg = \"body13.png\";\n  player.headimg = \"head5.png\";\n}\n",
    );
    let (mut m, mut host) = created(s);
    assert!(host.effects.iter().any(|e| matches!(
        e,
        Effect::SetPlayerProp { code, value } if code == "#8" && value == "body13.png"
    )));
    assert!(host.effects.iter().any(|e| matches!(
        e,
        Effect::SetPlayerProp { code, value } if code == "#3" && value == "head5.png"
    )));
    let _ = (&mut m, &mut host);
}

#[test]
fn counts_and_old_command_names_lower() {
    same_but_gs1_extras(
        "if (created) {\n  tokenize a b c;\n  this.n = tokenscount;\n  setgif door.png;\n  setgifpart door.png,0,0,32,32;\n}\n",
        "function onCreated() {\n  tokens = \"a b c\".tokenize();\n  this.n = tokens.size();\n  setimg(\"door.png\");\n  setimgpart(\"door.png\", 0, 0, 32, 32);\n}\n",
    );
    same_but_gs1_extras(
        "if (created) { this.a = playerhp; this.b = weaponscount; hide; }\n",
        "function onCreated() { this.a = player.hearts; this.b = player.weapons.size(); hide(); }\n",
    );
    for (src, name) in [
        ("if (created) { playerhp = 1; }\n", "playerhp"),
        ("if (created) { tokenscount = 1; }\n", "tokenscount"),
    ] {
        let err = compile::compile_gs1(src, Era::V217)
            .unwrap_err()
            .to_string();
        assert!(err.contains(name), "{src}: {err}");
    }
}

#[test]
fn every_gs1_command_says_how_it_lowers() {
    let mut missing = Vec::new();
    for c in seihai_core::scripting::engine::vocab::COMMANDS {
        let how = gs1_command_lowering(c.name).unwrap();
        let without = gs1_commands_without_gs2().contains(&c.name);
        match how {
            Gs1Lowering::Same => assert!(!without, "{}", c.name),
            Gs1Lowering::Renamed(_) | Gs1Lowering::Statements(_) => {
                assert!(without, "{}", c.name)
            }
            Gs1Lowering::Missing { .. } => {
                assert!(without, "{}", c.name);
                missing.push(c.name);
                // A missing command is a compile error naming it.
                let src = format!("if (created) {{ {}; }}\n", c.name);
                if let Err(e) = compile::compile_gs1(&src, Era::V217) {
                    assert!(e.to_string().contains(c.name), "{}: {e}", c.name);
                }
            }
        }
    }
    assert_eq!(
        gs1_command_lowering("setgif"),
        Some(Gs1Lowering::Renamed("setimg"))
    );
    assert!(matches!(
        gs1_command_lowering("setstring"),
        Some(Gs1Lowering::Statements(_))
    ));
    assert!(matches!(
        gs1_command_lowering("setz"),
        Some(Gs1Lowering::Statements(_))
    ));
    assert_eq!(
        gs1_command_lowering("putnpc"),
        Some(Gs1Lowering::Missing { inert: false })
    );
    assert!(matches!(
        gs1_command_lowering("setbody"),
        Some(Gs1Lowering::Statements(_))
    ));
    assert!(missing.len() >= 20, "{missing:?}");
    assert_eq!(gs1_command_lowering("notacommand"), None);
}

#[test]
fn a_count_alone_marks_the_function_as_calling() {
    same_but_gs1_extras(
        "if (created) { this.n = tokenscount; }\n",
        "function onCreated() { this.n = tokens.size(); }\n",
    );
    same(
        "if (created) { setcharprop #n,bob; }\n",
        "function onCreated() { this.nick = \"bob\"; }\n",
    );
}

#[test]
fn with_player_preserves_npc_variables_and_restores_effect_context() {
    // This uses the GS1 front end and production EffectHost, not legacy
    // GS2 compilation or an invented target/host. It covers one accepted
    // selective-with shape; existing refusal boundaries remain separate.
    // Seed the NPC explicitly: a newly written bare name outside with
    // otherwise follows the own VM resolver's global fallback.
    let (machine, host) = created(gs1(r#"
        if (created) {
            this.n = 3;
            x = 2;
            with (player) {
                x = 12;
                n += x;
            }
            this.after = n;
            x = 7;
        }
        "#));
    assert!(host.log.is_empty(), "{:?}", host.log);
    assert_eq!(machine.var(Root::This, "n"), Some(&Val::Num(15.0)));
    assert_eq!(machine.var(Root::This, "after"), Some(&Val::Num(15.0)));
    assert_eq!(machine.var(Root::Player, "n"), None);
    assert_eq!(machine.var(Root::Global, "n"), None);
    assert_eq!(host.player.x, 12.0);
    assert_eq!(host.x, 7.0);
    assert_eq!(
        host.effects,
        [
            Effect::SetPos {
                obj: 3,
                x: 2.0,
                y: 0.0
            },
            Effect::SetPlayerVar {
                name: "playerx".into(),
                value: 12.0
            },
            Effect::SetPos {
                obj: 3,
                x: 7.0,
                y: 0.0
            },
        ]
    );
}

#[test]
fn indexed_array_loops_execute_through_the_gs1_front_end() {
    // This checks the existing GS1-to-Machine seam, not colon-foreach,
    // native behavior, or a byte-for-byte GS2 comparison.
    let (machine, host) = created(gs1(r#"
        if (created) {
            setarray this.squares,5;
            this.visits = 0;
            for (i=0;i<5;i++) {
                this.squares[i] = i*i;
                this.visits++;
            }
            this.sum = 0;
            for (i=0;i<5;i++) this.sum += this.squares[i];
            this.after = i;
        }
        "#));
    let expected: Vec<_> = [0.0, 1.0, 4.0, 9.0, 16.0]
        .into_iter()
        .map(|n| seihai_core::scripting::gs2::lists::Value::from(Val::Num(n)))
        .collect();
    assert_eq!(
        machine.list(Root::This, "squares"),
        Some(expected.as_slice())
    );
    assert_eq!(machine.var(Root::This, "visits"), Some(&Val::Num(5.0)));
    assert_eq!(machine.var(Root::This, "sum"), Some(&Val::Num(30.0)));
    assert_eq!(machine.var(Root::This, "after"), Some(&Val::Num(5.0)));
    assert!(host.log.is_empty(), "{:?}", host.log);
    assert!(host.effects.is_empty(), "{:?}", host.effects);
}
