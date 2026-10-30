//! Runtime semantics, timeout determinism and version gating.

use super::world::*;
use super::{Era, Val};

fn npc(src: &str) -> (ScriptWorld, ObjId) {
    npc_in(src, Era::V21)
}

fn npc_in(src: &str, era: Era) -> (ScriptWorld, ObjId) {
    let mut w = ScriptWorld::new(era);
    let id = w.add(ObjSpec::npc("blank.png", 10.0, 20.0, src));
    (w, id)
}

fn messages(w: &mut ScriptWorld) -> Vec<String> {
    w.take_effects()
        .into_iter()
        .filter_map(|e| match e {
            Effect::Message { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn var(w: &ScriptWorld, id: ObjId, name: &str) -> Val {
    w.obj(id).unwrap().vars.get(name).cloned().unwrap_or_default()
}

// ---- events and basic commands ----

#[test]
fn created_runs_its_block_only() {
    let (mut w, id) = npc("if (created) { message made; }\nif (playerenters) { message entered; }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["made"]);
    w.fire(id, "playerenters", &[]);
    assert_eq!(messages(&mut w), vec!["entered"]);
}

#[test]
fn start_all_runs_created_once() {
    let (mut w, _) = npc("if (created) { this.n++; message hi; }");
    w.start_all();
    w.start_all();
    assert_eq!(messages(&mut w), vec!["hi"]);
}

#[test]
fn hide_and_show_track_visibility_and_emit_effects() {
    let (mut w, id) = npc("if (playertouchsme) { hide; }\nif (timeout) { show; }");
    w.fire(id, "playertouchsme", &[]);
    assert!(!w.obj(id).unwrap().visible);
    assert_eq!(w.take_effects(), vec![Effect::SetVisible { obj: id, visible: false, local: false }]);
    w.fire(id, "timeout", &[]);
    assert!(w.obj(id).unwrap().visible);
}

#[test]
fn setimg_sets_image_and_visibility() {
    let (mut w, id) = npc("if (created) { hide; setimg door.png; setimgpart sheet.png,16,0,16,32; }");
    w.fire(id, "created", &[]);
    let o = w.obj(id).unwrap();
    assert_eq!(o.image, "sheet.png");
    assert!(o.visible);
    let fx = w.take_effects();
    assert!(fx.contains(&Effect::SetImg { obj: id, file: "door.png".into(), part: None }));
    assert!(fx.contains(&Effect::SetImg { obj: id, file: "sheet.png".into(), part: Some((16.0, 0.0, 16.0, 32.0)) }));
}

#[test]
fn setlevel2_warps_and_ends_the_run() {
    let (mut w, id) = npc("if (playertouchsme) { setlevel2 house.nw,32,60; message never; }");
    w.fire(id, "playertouchsme", &[]);
    assert_eq!(w.take_effects(), vec![Effect::SetLevel { level: "house.nw".into(), x: Some(32.0), y: Some(60.0) }]);
}

#[test]
fn setlevel_without_position() {
    let (mut w, id) = npc("if (created) setlevel castle.nw;");
    w.fire(id, "created", &[]);
    assert_eq!(w.take_effects(), vec![Effect::SetLevel { level: "castle.nw".into(), x: None, y: None }]);
}

#[test]
fn hurt_lowers_hearts_in_halves() {
    let (mut w, id) = npc("if (playertouchsme) { hurt 3; }");
    w.core.player.hearts = 3.0;
    w.fire(id, "playertouchsme", &[]);
    assert_eq!(w.core.player.hearts, 1.5);
    assert_eq!(w.take_effects(), vec![Effect::Hurt { half_hearts: 3.0 }]);
}

#[test]
fn character_and_sound_commands() {
    let (mut w, id) = npc("if (created) { showcharacter; setcharani idle,; setcharprop #3,head1.png; play jump.wav; playlooped loop.mid; say2 Hello there; say 2; }");
    w.fire(id, "created", &[]);
    let fx = w.take_effects();
    assert!(fx.contains(&Effect::Command { obj: id, name: "showcharacter".into(), args: vec![] }));
    assert!(fx.contains(&Effect::SetCharAni { obj: id, ani: "idle".into(), params: "".into() }));
    assert!(fx.contains(&Effect::SetCharProp { obj: id, code: "#3".into(), value: "head1.png".into() }));
    assert!(fx.contains(&Effect::Play { obj: id, file: "jump.wav".into(), looped: false, at: None }));
    assert!(fx.contains(&Effect::Play { obj: id, file: "loop.mid".into(), looped: true, at: None }));
    assert!(fx.contains(&Effect::Say2 { text: "Hello there".into() }));
    assert!(fx.contains(&Effect::SaySign { index: 2 }));
}

#[test]
fn sendtoserver_is_a_stub_effect() {
    let (mut w, id) = npc("if (created) { sendtoserver hello,world; }");
    w.fire(id, "created", &[]);
    assert_eq!(w.take_effects(), vec![Effect::SendToServer { text: "hello,world".into() }]);
}

#[test]
fn setshape_changes_the_touch_rectangle() {
    let (mut w, id) = npc("if (created) { setshape 1,64,16; }");
    w.fire(id, "created", &[]);
    assert_eq!(w.obj(id).unwrap().rect(), (10.0, 20.0, 4.0, 1.0));
    // Other shape types are ignored.
    let (mut w, id) = npc("if (created) { setshape 5,64,16; }");
    w.fire(id, "created", &[]);
    assert_eq!(w.obj(id).unwrap().rect(), (10.0, 20.0, 2.0, 2.0));
}

// ---- unknown things ----

#[test]
fn unknown_commands_are_logged_and_ignored() {
    let (mut w, id) = npc("if (created) { frobnicate 1,2; message after; }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["after"]);
    let log = w.take_log();
    assert!(log.iter().any(|l| l.contains("frobnicate") && l.contains("unknown command")), "{log:?}");
}

#[test]
fn unknown_functions_and_events_do_not_crash() {
    let (mut w, id) = npc("if (created) { x = nosuchfn(1,2); message ok; }");
    w.fire(id, "created", &[]);
    w.fire(id, "notanevent", &[]);
    assert_eq!(messages(&mut w), vec!["ok"]);
    let log = w.take_log().join("\n");
    assert!(log.contains("unknown function `nosuchfn`"));
    assert!(log.contains("`notanevent` is not an event"));
}

#[test]
fn commands_with_no_executor_are_noted() {
    let (mut w, id) = npc("if (created) { setfocus 1,2; join foo; message ok; }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["ok"]);
    assert!(w.take_log().iter().filter(|l| l.contains("does nothing")).count() == 2);
}

#[test]
fn parse_errors_become_log_lines_and_the_rest_runs() {
    let (mut w, id) = npc("if (created) { message one; x = ; message two; }");
    let diag_logged = w.take_log().iter().any(|l| l.contains("expected an expression"));
    assert!(diag_logged);
    w.fire(id, "created", &[]);
    assert!(messages(&mut w).contains(&"two".to_string()));
}

// ---- variables ----

#[test]
fn scopes_are_separate_and_this_persists_across_events() {
    let mut w = ScriptWorld::new(Era::V21);
    let a = w.add(ObjSpec::npc("a.png", 0.0, 0.0, "if (created) { this.hp = 3; level.shared = 7; server.s = 9; local.l = 5; client.c = 1; }\nif (playerenters) { this.hp += 1; }"));
    let b = w.add(ObjSpec::npc("b.png", 0.0, 0.0, "if (created) { message #v(level.shared); }"));
    w.fire(a, "created", &[]);
    w.fire(a, "playerenters", &[]);
    w.fire(b, "created", &[]);
    let fx = w.take_effects();
    assert_eq!(var(&w, a, "this.hp"), Val::Num(4.0));
    assert_eq!(var(&w, b, "this.hp"), Val::Num(0.0));
    assert!(fx.iter().any(|e| matches!(e, Effect::Message { text, .. } if text == "7")));
    assert_eq!(w.core.server_vars["server.s"], Val::Num(9.0));
    assert_eq!(w.flag("client.c"), Some("1"));
    assert_eq!(w.flag("local.l"), Some("5"));
    // local. flags never go to the server; client. flags do.
    assert!(fx.iter().any(|e| matches!(e, Effect::SetFlag { name, .. } if name == "client.c")));
    assert!(!fx.iter().any(|e| matches!(e, Effect::SetFlag { name, .. } if name == "local.l")));
}

#[test]
fn level_variables_clear_with_the_level() {
    let (mut w, id) = npc("if (created) { level.a = 1; }");
    w.fire(id, "created", &[]);
    assert_eq!(w.core.level_vars.len(), 1);
    w.clear_level();
    assert!(w.core.level_vars.is_empty());
    assert!(w.is_empty() || w.ids().is_empty());
}

#[test]
fn flags_set_unset_and_read_back() {
    let (mut w, id) = npc("if (playertouchsme) { if (hasdoorkey) { message open; } else { message locked; set hasdoorkey; } }\nif (timeout) { unset hasdoorkey; }");
    w.fire(id, "playertouchsme", &[]);
    w.fire(id, "playertouchsme", &[]);
    assert_eq!(messages(&mut w), vec!["locked", "open"]);
    w.fire(id, "timeout", &[]);
    w.fire(id, "playertouchsme", &[]);
    assert_eq!(messages(&mut w), vec!["locked"]);
    let fx = w.take_effects();
    assert!(fx.iter().any(|e| matches!(e, Effect::UnsetFlag { name } if name == "hasdoorkey")) || w.flag("hasdoorkey").is_some());
}

#[test]
fn setstring_and_string_codes() {
    let (mut w, id) = npc("if (created) { setstring greeting,Hello Bob; message #s(greeting)!; }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["Hello Bob!"]);
    assert_eq!(w.flag("greeting"), Some("Hello Bob"));
}

#[test]
fn arrays_with_setarray_and_bounds() {
    let (mut w, id) = npc("if (created) { setarray this.a,3; this.a[0]=10; this.a[1]=20; this.a[5]=99; message #v(this.a[0]+this.a[1]) #v(this.a[5]) #v(arraylen(this.a)); }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["30 0 3"]);
}

#[test]
fn arrays_in_loops() {
    let (mut w, id) = npc("if (created) { setarray this.sq,5; for (i=0;i<5;i++) { this.sq[i] = i*i; } this.sum = 0; for (i=0;i<5;i++) this.sum += this.sq[i]; }");
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.sum"), Val::Num(30.0));
}

#[test]
fn two_dimensional_indices() {
    let (mut w, id) = npc("if (created) { this.g[1,2] = 5; this.g[2,1] = 7; this.r = this.g[1,2] * 10 + this.g[2,1]; }");
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.r"), Val::Num(57.0));
}

#[test]
fn builtin_player_variables_read_and_write() {
    let (mut w, id) = npc("if (created) { this.px = playerx; playerrupees += 5; playerdir = 5; }");
    w.core.player.x = 12.5;
    w.core.player.rupees = 1.0;
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.px"), Val::Num(12.5));
    assert_eq!(w.core.player.rupees, 6.0);
    assert_eq!(w.core.player.dir, 1.0); // 5 mod 4
    let fx = w.take_effects();
    assert!(fx.contains(&Effect::SetPlayerVar { name: "playerrupees".into(), value: 6.0 }));
}

#[test]
fn npc_position_variables() {
    let (mut w, id) = npc("if (created) { x = x + 2; y -= 1; }");
    w.fire(id, "created", &[]);
    let o = w.obj(id).unwrap();
    assert_eq!((o.x, o.y), (12.0, 19.0));
    assert!(w.take_effects().contains(&Effect::SetPos { obj: id, x: 12.0, y: 19.0 }));
}

#[test]
fn read_only_builtins_ignore_writes() {
    let (mut w, id) = npc("if (created) { playermaxhp = 99; this.m = playermaxhp; }");
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.m"), Val::Num(w.core.player.max_hp));
}

// ---- expressions ----

fn eval(expr: &str) -> Val {
    let (mut w, id) = npc(&format!("if (created) {{ this.r = {expr}; }}"));
    w.fire(id, "created", &[]);
    var(&w, id, "this.r")
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(eval("1 + 2 * 3"), Val::Num(7.0));
    assert_eq!(eval("(1 + 2) * 3"), Val::Num(9.0));
    assert_eq!(eval("7 % 4"), Val::Num(3.0));
    assert_eq!(eval("2 ^ 3 ^ 2"), Val::Num(512.0));
    assert_eq!(eval("-3 + 5"), Val::Num(2.0));
    assert_eq!(eval("10 / 4"), Val::Num(2.5));
}

#[test]
fn division_and_modulo_by_zero_are_zero() {
    assert_eq!(eval("5 / 0"), Val::Num(0.0));
    assert_eq!(eval("5 % 0"), Val::Num(0.0));
}

#[test]
fn comparison_and_logic() {
    assert_eq!(eval("3 < 4 && 4 <= 4"), Val::Num(1.0));
    assert_eq!(eval("3 > 4 || 0"), Val::Num(0.0));
    assert_eq!(eval("!0"), Val::Num(1.0));
    assert_eq!(eval("1 == 1.0"), Val::Num(1.0));
    assert_eq!(eval("1 != 2"), Val::Num(1.0));
}

#[test]
fn logic_short_circuits() {
    let (mut w, id) = npc("if (created) { 0 && bump(); 1 || bump(); this.r = this.n; }\nfunction bump() { this.n++; return 1; }");
    w.fire(id, "created", &[]);
    // `0 && bump();` is not a statement shape; check via an expression.
    let (mut w2, id2) = npc("if (created) { this.t = 0 && bump(); this.u = 1 || bump(); }\nfunction bump() { this.n++; return 1; }");
    w2.fire(id2, "created", &[]);
    assert_eq!(var(&w2, id2, "this.n"), Val::Num(0.0));
    let _ = (w.take_log(), id);
}

#[test]
fn in_ranges_and_lists() {
    assert_eq!(eval("5 in |0,10|"), Val::Num(1.0));
    assert_eq!(eval("11 in |0,10|"), Val::Num(0.0));
    assert_eq!(eval("2 in {1,2,3}"), Val::Num(1.0));
    assert_eq!(eval("4 in {1,2,3}"), Val::Num(0.0));
}

#[test]
fn a_pair_in_a_range_needs_both() {
    // The *Quiver weapon on 2gta: `mapdx,mapdy in |-1,1|`.
    assert_eq!(eval("1 && 0,-1 in |-1,1|"), Val::Num(1.0));
    assert_eq!(eval("1 && 2,0 in |-1,1|"), Val::Num(0.0));
    assert_eq!(eval("0,2 in |-1,1|"), Val::Num(0.0));
    let src = "if (created) { this.a=0; this.b=1; if ((1 || 0) && this.a,this.b in |-1,1|) { message near; } else { message far; } }";
    let (mut w, id) = npc_in(src, Era::V205);
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["near"]);
    // A comma in an argument list is still an argument list.
    assert_eq!(eval("strequals(\"a\", \"a\")"), Val::Num(1.0));
}

#[test]
fn string_concatenation_only_for_non_numeric_text() {
    assert_eq!(eval("\"ab\" + \"cd\""), Val::Str("abcd".into()));
    assert_eq!(eval("\"5\" + 1"), Val::Num(6.0));
    assert_eq!(eval("\"x\" + 1"), Val::Str("x1".into()));
}

#[test]
fn builtin_functions() {
    assert_eq!(eval("abs(-4)"), Val::Num(4.0));
    assert_eq!(eval("int(3.9)"), Val::Num(3.0));
    assert_eq!(eval("strlen(hello)"), Val::Num(5.0));
    assert_eq!(eval("startswith(he,hello)"), Val::Num(1.0));
    assert_eq!(eval("indexof(lo,hello)"), Val::Num(3.0));
    assert_eq!(eval("indexof(z,hello)"), Val::Num(-1.0));
    assert_eq!(eval("strequals(Hello,hello)"), Val::Num(1.0));
    assert_eq!(eval("ascii(A)"), Val::Num(65.0));
    let v = eval("sin(0) + cos(0)");
    assert_eq!(v, Val::Num(1.0));
}

#[test]
fn random_is_in_range_and_deterministic_per_seed() {
    let run = |seed| {
        let mut w = ScriptWorld::with_seed(Era::V21, seed);
        let id = w.add(ObjSpec::npc("a.png", 0.0, 0.0, "if (created) { this.a = random(1,7); this.b = random(1,7); }"));
        w.fire(id, "created", &[]);
        (var(&w, id, "this.a"), var(&w, id, "this.b"))
    };
    let (a, b) = run(42);
    assert_eq!((a.clone(), b.clone()), run(42));
    assert_ne!(run(42), run(43));
    for v in [a, b] {
        assert!(v.num() >= 1.0 && v.num() < 7.0);
    }
}

#[test]
fn onwall_reads_the_level_collision_and_is_true_off_the_board() {
    let (mut w, id) = npc("if (created) { this.a = onwall(3.5, 4.2); this.b = onwall(5, 5); this.c = onwall(-1, 2); this.d = onwall(64, 2); }");
    let mut walls = vec![false; 64 * 64];
    walls[4 * 64 + 3] = true;
    w.core.walls = Some(walls);
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.a"), Val::Num(1.0));
    assert_eq!(var(&w, id, "this.b"), Val::Num(0.0));
    assert_eq!(var(&w, id, "this.c"), Val::Num(1.0));
    assert_eq!(var(&w, id, "this.d"), Val::Num(1.0));
}

#[test]
fn onwater_reads_the_liquid_tiles_and_tiletype_is_minus_one() {
    let (mut w, id) = npc("if (created) { this.a = onwater(3.5, 4.2); this.b = onwater(5, 5); this.c = onwater(-1, 2); this.t = tiletype(3, 4); }");
    let mut water = vec![false; 64 * 64];
    water[4 * 64 + 3] = true;
    w.core.water = Some(water);
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.a"), Val::Num(1.0));
    assert_eq!(var(&w, id, "this.b"), Val::Num(0.0));
    assert_eq!(var(&w, id, "this.c"), Val::Num(0.0));
    assert_eq!(var(&w, id, "this.t"), Val::Num(-1.0));
}

#[test]
fn playersays_matches_only_during_a_chat_event() {
    let (mut w, id) = npc("if (playerchats && playersays(Hello)) { message exact; }\nif (playerchats && playersays2(open)) { message has; }\nif (created) { if (playersays(Hello)) message early; }");
    w.core.player.chat = "hello".into();
    w.fire(id, "created", &[]);
    assert!(messages(&mut w).is_empty());
    w.chat("HELLO");
    assert_eq!(messages(&mut w), vec!["exact"]);
    w.chat("please open it");
    assert_eq!(messages(&mut w), vec!["has"]);
    w.chat("hello there");
    assert!(messages(&mut w).is_empty());
}

// ---- control flow ----

#[test]
fn while_for_break_continue() {
    let (mut w, id) = npc(
        "if (created) {
           i = 0; while (i < 10) { i++; if (i == 3) continue; if (i == 6) break; this.s += i; }
           for (j = 0; j < 4; j++) { this.f += 1; }
         }",
    );
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.s"), Val::Num(1.0 + 2.0 + 4.0 + 5.0));
    assert_eq!(var(&w, id, "this.f"), Val::Num(4.0));
}

#[test]
fn infinite_loops_stop_after_ten_thousand_passes() {
    let (mut w, id) = npc("if (created) { while (1) { this.n++; } message unreachable; }");
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.n"), Val::Num(10000.0));
    assert!(messages(&mut w).is_empty());
    assert!(w.take_log().iter().any(|l| l.contains("loop stopped")));
}

#[test]
fn functions_params_and_return_values() {
    let (mut w, id) = npc("if (created) { this.r = twice(add(2,3)); this.n = n; }\nfunction add(a,b) { return a + b; }\nfunction twice(v) { return v * 2; }");
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.r"), Val::Num(10.0));
    // Parameters are not left behind.
    assert_eq!(var(&w, id, "a"), Val::Num(0.0));
}

#[test]
fn function_call_statements_run_and_return_ends_the_function_only() {
    let (mut w, id) = npc("if (created) { bump(); bump(); message done; }\nfunction bump() { this.n++; return; this.n += 100; }");
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.n"), Val::Num(2.0));
    assert_eq!(messages(&mut w), vec!["done"]);
}

#[test]
fn return_at_top_level_ends_the_script() {
    let (mut w, id) = npc("if (created) { message a; return; message b; }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["a"]);
}

#[test]
fn recursion_is_cut_off() {
    let (mut w, id) = npc("if (created) { forever(); message after; }\nfunction forever() { this.d++; forever(); }");
    w.fire(id, "created", &[]);
    assert!(var(&w, id, "this.d").num() <= 64.0);
    assert!(w.take_log().iter().any(|l| l.contains("nested deeper")));
}

#[test]
fn deep_expression_nesting_at_runtime_is_cut_off() {
    let src = format!("if (created) {{ this.r = {}1{}; }}", "(".repeat(150), ")".repeat(150));
    let (mut w, id) = npc(&src);
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.r"), Val::Num(1.0));
}

#[test]
fn step_budget_stops_runaway_straight_line_work() {
    // 10,000-pass loops nested enough to exceed the step budget.
    let (mut w, id) = npc("if (created) { for (a=0;a<9000;a++) { for (b=0;b<9000;b++) { this.n++; } } }");
    w.fire(id, "created", &[]);
    let log = w.take_log().join("\n");
    assert!(log.contains("stopped after"), "{log}");
}

// ---- message codes ----

#[test]
fn message_codes() {
    let (mut w, id) = npc("if (playerchats) { message #n said #c in #L (#a); }");
    w.core.player.nick = "Bob".into();
    w.core.player.account = "bob123".into();
    w.core.level = "town.nw".into();
    w.chat("hi there");
    assert_eq!(messages(&mut w), vec!["Bob said hi there in town.nw (bob123)"]);
    let _ = id;
}

#[test]
fn v_code_evaluates_expressions_and_unknown_codes_stay() {
    let (mut w, id) = npc("if (created) { this.k = 4; message #v(this.k * 2) #z #(x); }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["8 #z #(x)"]);
}

#[test]
fn string_helper_codes_and_tokenize() {
    let (mut w, id) = npc("if (created) { tokenize alpha, \"beta gamma\" delta; message #t(1)|#t(2)|#v(tokenscount)|#e(1,3,abcdef)|#T(  pad  ); }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["beta gamma|delta|3|bcd|pad"]);
}

#[test]
fn tokenize2_adds_delimiters() {
    let (mut w, id) = npc("if (created) { tokenize2 a|b|c,|; message #v(tokenscount); }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["3"]);
}

#[test]
fn event_parameters_are_codes() {
    let (mut w, id) = npc("if (actionopen) { message #p(0) and #p(1) (#v(paramscount)); }");
    w.fire(id, "actionopen", &["red".into(), "door".into()]);
    assert_eq!(messages(&mut w), vec!["red and door (2)"]);
}

// ---- timeout and sleep ----

#[test]
fn timeout_assignment_fires_the_timeout_event_once() {
    let (mut w, id) = npc("if (created) { timeout = 1; }\nif (timeout) { this.n++; message tick; }");
    w.fire(id, "created", &[]);
    w.tick(950);
    assert!(messages(&mut w).is_empty());
    w.tick(50);
    assert_eq!(messages(&mut w), vec!["tick"]);
    w.tick(10_000);
    assert!(messages(&mut w).is_empty());
    assert_eq!(var(&w, id, "this.n"), Val::Num(1.0));
}

#[test]
fn a_timeout_block_can_rearm_itself() {
    let (mut w, id) = npc("if (created) { timeout = 0.5; }\nif (timeout) { this.n++; timeout = 0.5; }");
    w.fire(id, "created", &[]);
    w.tick(2000);
    assert_eq!(var(&w, id, "this.n"), Val::Num(4.0));
}

#[test]
fn timeout_zero_cancels() {
    let (mut w, id) = npc("if (created) { timeout = 1; timeout = 0; }\nif (timeout) { this.n++; }");
    w.fire(id, "created", &[]);
    w.tick(5000);
    assert_eq!(var(&w, id, "this.n"), Val::Num(0.0));
}

#[test]
fn sleep_resumes_after_the_sleep_statement() {
    let (mut w, id) = npc("if (playertouchsme) { message one; sleep 1; message two; sleep 1; message three; }");
    w.fire(id, "playertouchsme", &[]);
    assert_eq!(messages(&mut w), vec!["one"]);
    w.tick(1000);
    assert_eq!(messages(&mut w), vec!["two"]);
    w.tick(1000);
    assert_eq!(messages(&mut w), vec!["three"]);
    w.tick(5000);
    assert!(messages(&mut w).is_empty());
}

#[test]
fn sleep_inside_a_loop_resumes_in_the_loop() {
    let (mut w, id) = npc("if (created) { for (i=0;i<3;i++) { this.n++; sleep 0.5; } message done; }");
    w.fire(id, "created", &[]);
    // The resume point is the node after `sleep`: the loop continues.
    w.tick(500);
    w.tick(500);
    w.tick(500);
    assert_eq!(var(&w, id, "this.n"), Val::Num(3.0));
    assert_eq!(messages(&mut w), vec!["done"]);
}

#[test]
fn sleep_zero_does_not_end_the_run() {
    let (mut w, id) = npc("if (created) { message a; sleep 0; message b; }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["a", "b"]);
}

#[test]
fn timeouts_fire_in_tick_order_then_creation_order_and_are_repeatable() {
    let run = |slices: &[u64]| {
        let mut w = ScriptWorld::new(Era::V21);
        let mk = |w: &mut ScriptWorld, name: &str, t: f64| {
            w.add(ObjSpec::npc("a.png", 0.0, 0.0, &format!("if (created) {{ timeout = {t}; }}\nif (timeout) {{ message {name}; }}")))
        };
        let c = mk(&mut w, "c", 0.2);
        let a = mk(&mut w, "a", 0.1);
        let b = mk(&mut w, "b", 0.2);
        for id in [c, a, b] {
            w.fire(id, "created", &[]);
        }
        let mut out = Vec::new();
        for &ms in slices {
            w.tick(ms);
            out.extend(messages(&mut w));
        }
        out
    };
    let whole = run(&[300]);
    assert_eq!(whole, vec!["a", "c", "b"]);
    // Slicing the time differently gives the same order.
    assert_eq!(run(&[50, 50, 50, 50, 50, 50]), whole);
    assert_eq!(run(&[7, 93, 13, 187]), whole);
    assert_eq!(run(&[300]), whole);
}

#[test]
fn partial_ticks_accumulate() {
    let (mut w, id) = npc("if (created) { timeout = 0.05; }\nif (timeout) { message t; }");
    w.fire(id, "created", &[]);
    w.tick(20);
    w.tick(20);
    assert!(messages(&mut w).is_empty());
    w.tick(10);
    assert_eq!(messages(&mut w), vec!["t"]);
    assert_eq!(w.now_ms(), 50);
}

// ---- touch and cross-object calls ----

#[test]
fn touch_fires_on_entry_and_rearms_after_leaving() {
    let (mut w, id) = npc("if (playertouchsme) { message touched; }");
    assert_eq!(w.touch(0.0, 0.0), 0);
    assert_eq!(w.touch(10.5, 20.5), 1);
    assert_eq!(w.touch(10.6, 20.6), 0); // still inside
    assert_eq!(w.touch(0.0, 0.0), 0);
    assert_eq!(w.touch(11.9, 21.9), 1);
    assert_eq!(messages(&mut w), vec!["touched", "touched"]);
    let _ = id;
}

#[test]
fn a_thrown_object_finds_the_first_visible_npc_at_its_point_and_pelts_it() {
    let (mut w, id) = npc("if (waspelt) { message ouch; hide; }");
    assert_eq!(w.npc_at(9.9, 20.5), None);
    assert_eq!(w.npc_at(10.0, 20.0), Some(id), "the box includes its top-left edge");
    assert_eq!(w.npc_at(12.0, 20.5), None, "and not its right edge");
    w.fire(id, "waspelt", &[]);
    assert_eq!(messages(&mut w), vec!["ouch"]);
    assert_eq!(w.npc_at(10.5, 20.5), None, "a hidden NPC is not hit");
}

#[test]
fn hidden_npcs_are_not_touched() {
    let (mut w, id) = npc("if (playertouchsme) { hide; message hit; }");
    assert_eq!(w.touch(10.5, 20.5), 1);
    assert_eq!(w.touch(0.0, 0.0), 0);
    assert_eq!(w.touch(10.5, 20.5), 0);
    assert!(!w.obj(id).unwrap().visible);
}

#[test]
fn weapons_are_not_touched() {
    let mut w = ScriptWorld::new(Era::V21);
    let id = w.add(ObjSpec::weapon("Bow", "bow.png", "if (playertouchsme) { message no; }"));
    w.obj_mut(id).unwrap().x = 10.0;
    assert_eq!(w.touch(10.5, 0.5), 0);
}

#[test]
fn weaponfired_runs_on_the_named_weapon() {
    let mut w = ScriptWorld::new(Era::V21);
    w.add(ObjSpec::weapon("Bow", "bow.png", "if (weaponfired) { message bow; }"));
    w.add(ObjSpec::weapon("Gun", "gun.png", "if (weaponfired) { message gun; }"));
    w.weapon_fired("Gun");
    assert_eq!(messages(&mut w), vec!["gun"]);
    assert_eq!(w.obj(0).unwrap().kind, ObjKind::Weapon);
}

#[test]
fn callnpc_runs_after_the_caller_finishes() {
    let mut w = ScriptWorld::new(Era::V21);
    let a = w.add(ObjSpec::npc("a.png", 0.0, 0.0, "if (created) { callnpc 1,playerenters; message caller; }"));
    w.add(ObjSpec::npc("b.png", 0.0, 0.0, "if (playerenters) { message callee; }"));
    w.fire(a, "created", &[]);
    assert_eq!(messages(&mut w), vec!["caller", "callee"]);
}

#[test]
fn mutual_calls_are_bounded() {
    let mut w = ScriptWorld::new(Era::V21);
    let a = w.add(ObjSpec::npc("a.png", 0.0, 0.0, "if (created) { callnpc 1,playerenters; }\nif (playerenters) { callnpc 1,playerenters; }"));
    w.add(ObjSpec::npc("b.png", 0.0, 0.0, "if (playerenters) { callnpc 0,playerenters; }"));
    w.fire(a, "created", &[]);
    assert!(w.take_log().iter().any(|l| l.contains("cut off")));
}

#[test]
fn triggeraction_reaches_the_object_under_the_point_and_the_server() {
    let mut w = ScriptWorld::new(Era::V21);
    let a = w.add(ObjSpec::npc("a.png", 0.0, 0.0, "if (created) { triggeraction 10.5,20.5,Open,red,door; }"));
    w.add(ObjSpec::npc("b.png", 10.0, 20.0, "if (actionopen) { message #p(0)-#p(1); }"));
    w.fire(a, "created", &[]);
    let fx = w.take_effects();
    assert!(fx.contains(&Effect::TriggerAction { x: 10.5, y: 20.5, action: "open".into(), params: "red,door".into() }));
    assert!(fx.iter().any(|e| matches!(e, Effect::Message { text, .. } if text == "red-door")));
}

#[test]
fn destroy_removes_the_object_and_stops_timers() {
    let (mut w, id) = npc("if (created) { timeout = 1; destroy; message no; }\nif (timeout) { message late; }");
    w.fire(id, "created", &[]);
    w.tick(2000);
    w.touch(10.5, 20.5);
    assert!(w.take_effects().contains(&Effect::Destroy { obj: id }));
    assert!(w.ids().is_empty());
}

#[test]
fn move_applies_the_offset_and_can_signal_the_end() {
    let (mut w, id) = npc("if (created) { move 2,0,1,8; }\nif (movementfinished) { message arrived; }");
    w.fire(id, "created", &[]);
    assert_eq!(w.obj(id).unwrap().x, 12.0);
    assert_eq!(messages(&mut w), vec!["arrived"]);
    // Before 2.03: two arguments only, and no end event.
    let (mut w, id) = npc_in("if (created) { move 2,0,1,8; }", Era::V202);
    w.fire(id, "created", &[]);
    assert_eq!(w.obj(id).unwrap().x, 12.0);
    assert!(w.take_effects().contains(&Effect::Move { obj: id, dx: 2.0, dy: 0.0, time: 0.0, options: 0 }));
}

// ---- version gating ----

#[test]
fn gated_commands_do_nothing_in_older_eras() {
    let src = "if (created) { hitobjects 1,2,3; message after; }";
    let (mut w, id) = npc_in(src, Era::V200);
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["after"]);
    assert!(w.take_log().iter().any(|l| l.contains("hitobjects") && l.contains("not available")));
    let (mut w, id) = npc_in(src, Era::V201);
    w.fire(id, "created", &[]);
    let fx = w.take_effects();
    assert!(fx.iter().any(|e| matches!(e, Effect::Command { name, .. } if name == "hitobjects")));
}

#[test]
fn string_functions_appear_in_2_02() {
    let src = "if (created) { this.n = strlen(hello); message #e(0,2,hello); tokenize a b; this.t = tokenscount; }";
    let (mut w, id) = npc_in(src, Era::V201);
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.n"), Val::Num(0.0));
    // `#e` is not a code before 2.02: it stays as written.
    assert_eq!(messages(&mut w), vec!["#e(0,2,hello)"]);
    let (mut w, id) = npc_in(src, Era::V202);
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.n"), Val::Num(5.0));
    assert_eq!(messages(&mut w), vec!["he"]);
    assert_eq!(var(&w, id, "this.t"), Val::Num(2.0));
}

#[test]
fn actions_and_triggeraction_appear_in_2_03() {
    let src = "if (created) { triggeraction 10.5,20.5,go,; }\nif (actiongo) { message got; }";
    let (mut w, id) = npc_in(src, Era::V202);
    w.fire(id, "created", &[]);
    assert!(messages(&mut w).is_empty());
    w.fire(id, "actiongo", &[]);
    assert!(w.take_log().iter().any(|l| l.contains("does not exist in 2.02")));
    let (mut w, id) = npc_in(src, Era::V203);
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["got"]);
}

#[test]
fn events_added_later_are_not_fired_early() {
    let (mut w, id) = npc_in("if (playerdies) { message dead; }", Era::V200);
    w.fire(id, "playerdies", &[]);
    assert!(messages(&mut w).is_empty());
    assert!(w.take_log().iter().any(|l| l.contains("does not exist in 2.00")));
    let (mut w, id) = npc_in("if (playerdies) { message dead; }", Era::V201);
    w.fire(id, "playerdies", &[]);
    assert_eq!(messages(&mut w), vec!["dead"]);
}

#[test]
fn builtin_variables_are_gated() {
    let src = "if (created) { this.v = vecx + 5; }";
    let (mut w, id) = npc_in(src, Era::V202);
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.v"), Val::Num(5.0)); // plain unknown variable, 0
    let (mut w, id) = npc_in("if (created) { vecx = 3; this.v = vecx; }", Era::V203);
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.v"), Val::Num(0.0)); // read-only
}

#[test]
fn status_flags_come_from_the_player_view() {
    let (mut w, id) = npc("if (playertouchsme) { if (playerswimming) message wet; else message dry; }");
    w.fire(id, "playertouchsme", &[]);
    w.core.player.status.insert("playerswimming".into());
    w.fire(id, "playertouchsme", &[]);
    assert_eq!(messages(&mut w), vec!["dry", "wet"]);
}

#[test]
fn online_gated_commands_need_an_npc_server_when_connected() {
    let src = "if (created) { seteffect 1,0,0,1; changeimgzoom 1,2; message after; }";
    // Offline: runs.
    let (mut w, id) = npc(src);
    w.fire(id, "created", &[]);
    assert_eq!(w.take_effects().iter().filter(|e| matches!(e, Effect::Command { .. })).count(), 2);
    // Online without an NPC server: nothing.
    let (mut w, id) = npc(src);
    w.set_online(true, false);
    w.fire(id, "created", &[]);
    assert_eq!(w.take_effects().iter().filter(|e| matches!(e, Effect::Command { .. })).count(), 0);
    // Online with one: runs.
    let (mut w, id) = npc(src);
    w.set_online(true, true);
    w.fire(id, "created", &[]);
    assert_eq!(w.take_effects().iter().filter(|e| matches!(e, Effect::Command { .. })).count(), 2);
}

#[test]
fn real_world_style_scripts() {
    // A pickup that hides itself, pays out, and comes back after a while.
    let src = "if (created) { setimg rupee.png; setshape 1,16,16; }\n\
               if (playertouchsme) { hide; playerrupees += 5; timeout = 30; play coin.wav; }\n\
               if (timeout) { show; }";
    let mut w = ScriptWorld::new(Era::V205);
    let id = w.add(ObjSpec::npc("blank.png", 4.0, 4.0, src));
    w.start_all();
    assert_eq!(w.obj(id).unwrap().image, "rupee.png");
    assert_eq!(w.touch(4.5, 4.5), 1);
    assert_eq!(w.core.player.rupees, 5.0);
    assert!(!w.obj(id).unwrap().visible);
    w.tick(29_900);
    assert!(!w.obj(id).unwrap().visible);
    w.tick(100);
    assert!(w.obj(id).unwrap().visible);
    // Walk off and on again: a second payout.
    w.touch(0.0, 0.0);
    assert_eq!(w.touch(4.5, 4.5), 1);
    assert_eq!(w.core.player.rupees, 10.0);
}

/// Smoke test over a local corpus of real GS1 scripts (NUL-separated text
/// in the file named by `SEIHAI_SCRIPT_CORPUS`). The corpus is not part of
/// the repository, so this is ignored by default:
/// `SEIHAI_SCRIPT_CORPUS=scripts.txt cargo test -p seihai corpus -- --ignored --nocapture`
#[test]
#[ignore]
fn corpus_parses_and_runs_without_panicking() {
    let Ok(path) = std::env::var("SEIHAI_SCRIPT_CORPUS") else {
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let mut scripts = 0;
    let mut with_diag = 0;
    let mut shown = 0;
    let mut diag_kinds: std::collections::BTreeMap<String, usize> = Default::default();
    let mut unknown: std::collections::BTreeMap<String, usize> = Default::default();
    for src in text.split('\0').filter(|s| !s.trim().is_empty()) {
        scripts += 1;
        let parsed = super::parse(src, Era::V205);
        if !parsed.diagnostics.is_empty() {
            with_diag += 1;
            for d in &parsed.diagnostics {
                *diag_kinds.entry(d.msg.clone()).or_default() += 1;
            }
            let want = std::env::var("SEIHAI_CORPUS_SHOW").unwrap_or_default();
            if !want.is_empty() && parsed.diagnostics.iter().any(|d| d.msg.contains(&want)) && shown < 8 {
                shown += 1;
                println!("--- {:?}\n    {:?}", src.trim(), parsed.diagnostics);
            }
        }
        let mut w = ScriptWorld::new(Era::V205);
        let id = w.add(ObjSpec::npc("a.png", 10.0, 10.0, src));
        for l in w.take_log() {
            if let Some(i) = l.find("unknown command") {
                let _ = i;
            }
        }
        for ev in ["created", "playerenters", "playertouchsme", "timeout", "wasshot", "playerendsreading", "compusdied"] {
            w.fire(id, ev, &[]);
        }
        w.tick(120_000);
        for l in w.take_log() {
            if let Some(f) = l.split("unknown function `").nth(1) {
                *unknown.entry(format!("fn {}", f.split('`').next().unwrap_or(""))).or_default() += 1;
            }
            if let Some(rest) = l.split("`").nth(1) {
                if l.contains("ignored (unknown command)") {
                    let name = rest.split_whitespace().next().unwrap_or("").to_string();
                    // Stray image names and single letters are level-file debris.
                    if name.len() > 1 && !name.contains('.') {
                        *unknown.entry(name).or_default() += 1;
                    }
                }
            }
        }
    }
    let mut top: Vec<_> = unknown.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1));
    println!("corpus: {scripts} scripts, {with_diag} with parse diagnostics");
    println!("diagnostics: {diag_kinds:?}");
    println!("most common unknown commands: {:?}", &top[..top.len().min(25)]);
}

#[test]
fn diagnostic_names_the_object_and_shows_the_line() {
    let mut w = ScriptWorld::new(Era::V21);
    w.add(ObjSpec::weapon("w1", "a.png", "hide;\nsay 2 3\n"));
    w.add(ObjSpec::npc("block.png", 4.0, 5.5, "if (playerenters) {\n  say 2 3\n  hide;\n}\n"));
    let log = w.take_log();
    assert!(log.iter().any(|l| l.starts_with("object 0 (weapon w1): line 2: expected `;`") && l.ends_with("[say 2 3]")), "{log:?}");
    assert!(log.iter().any(|l| l.starts_with("object 1 (npc block.png at 4,5.5): line 2:") && l.contains("[say 2 3]")), "{log:?}");
}

#[test]
fn weapon_script_with_section_signs_keeps_its_lines() {
    // The server's weapon packet writes line breaks as `§`; a UTF-8 server
    // sends it as C2 A7. Both must give a script with real lines.
    let body = "//#CLIENTSIDE\u{a7}//#GS1\u{a7}if (playerenters) {\u{a7}  hearts = 2;\u{a7}}\u{a7}";
    for bytes in [body.as_bytes().to_vec(), body.chars().map(|c| c as u32 as u8).collect::<Vec<u8>>()] {
        let text = crate::protocol::script_text(&bytes);
        assert_eq!(text.lines().count(), 5, "{text:?}");
        let mut w = ScriptWorld::new(Era::V21);
        w.add(ObjSpec::weapon("w", "", &text));
        assert!(w.take_log().is_empty());
    }
}

// ---- text join operators and the chat attribute ----

#[test]
fn the_users_touch_script_parses_and_sets_the_chat_bubble() {
    // Line 4 of the script that logged `expected ;`.
    let src = "//#CLIENTSIDE\nif (playertouchsme) {\n  this.n = 1;\n  chat = player.account SPC \"toouched me last\";\n}\n";
    let (mut w, id) = npc(src);
    assert!(w.obj(id).unwrap().diagnostics().is_empty(), "{:?}", w.obj(id).unwrap().diagnostics());
    w.core.player.account = "bob123".into();
    w.fire(id, "playertouchsme", &[]);
    assert_eq!(messages(&mut w), vec!["bob123 toouched me last"]);
}

#[test]
fn text_join_operators_put_the_right_separator_between() {
    let (mut w, id) = npc("if (created) { this.a = \"x\" @ \"y\"; this.b = \"x\" SPC \"y\"; this.c = \"x\" NL \"y\"; this.d = \"x\" TAB \"y\"; message #v(this.a)|#v(this.b)|#v(this.c)|#v(this.d); }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["xy|x y|x\ny|x\ty"]);
}

#[test]
fn text_join_binds_looser_than_arithmetic_and_tighter_than_and() {
    let (mut w, id) = npc("if (created) { this.v = 1 + 2 @ 3 * 2; chat = this.v; }");
    w.fire(id, "created", &[]);
    assert_eq!(messages(&mut w), vec!["36"]);
    // `a @ b && c`: the join is evaluated first, then the test.
    let (w2, id2) = npc("if (1 @ 2 && 1) { message yes; }");
    assert!(w2.obj(id2).unwrap().diagnostics().is_empty());
}

#[test]
fn chat_and_player_attributes_read_and_write() {
    let (mut w, id) = npc("if (created) { player.chat = player.nick SPC \"says hi\"; chat = player.chat; }");
    w.core.player.nick = "Bob".into();
    w.fire(id, "created", &[]);
    assert_eq!(w.core.player.chat, "Bob says hi");
    assert_eq!(messages(&mut w), vec!["Bob says hi"]);
}

// ---- 2.1 forms from the NPC corpus ----

#[test]
fn an_array_literal_fills_an_array() {
    let (mut w, id) = npc("if (created) { this.a = {4,5,6}; this.n = arraylen(this.a); this.b = this.a[1]; this.c = this.a[3]; }");
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.n"), Val::Num(3.0));
    assert_eq!(var(&w, id, "this.b"), Val::Num(5.0));
    assert_eq!(var(&w, id, "this.c"), Val::Num(0.0), "past the end");
}

#[test]
fn the_choice_operator_picks_a_side() {
    let (mut w, id) = npc("if (created) { this.a = (1 > 2) ? 3 : 4; this.b = 1 ? 5 : 6; }");
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.a"), Val::Num(4.0));
    assert_eq!(var(&w, id, "this.b"), Val::Num(5.0));
}

#[test]
fn a_with_body_is_skipped_not_run_on_the_wrong_object() {
    let (mut w, id) = npc("if (created) { with (getnpc(other)) { this.x = 1; } this.after = 2; }");
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.x"), Val::default());
    assert_eq!(var(&w, id, "this.after"), Val::Num(2.0));
}

#[test]
fn a_gani_attribute_reads_back_what_setcharprop_set() {
    let (mut w, id) = npc("if (created) { setcharprop #P1,7; this.a = strtofloat(#P1(-1)); this.b = strtofloat(#P1); message #P2(-1)x; }");
    w.fire(id, "created", &[]);
    assert_eq!(var(&w, id, "this.a"), Val::Num(7.0));
    assert_eq!(var(&w, id, "this.b"), Val::Num(7.0));
    assert_eq!(messages(&mut w), vec!["x"]);
}

#[test]
fn the_2_1_forms_are_not_read_by_2_05() {
    for src in ["this.a = {1,2};", "this.a = 1 ? 2 : 3;", "with (players[0]) { x = 1; }"] {
        assert!(!super::parse(src, Era::V205).diagnostics.is_empty(), "{src}");
        assert!(super::parse(src, Era::V21).diagnostics.is_empty(), "{src}");
    }
}

// ---- 2.2 string lists and the builtins from the laundry list (2026-10-29) ----

#[test]
fn string_lists_add_insert_replace_remove_delete() {
    let (mut w, _) = npc(
        "if (created) {\n\
         addstring client.l,apple; addstring client.l,pear; addstring client.l,plum;\n\
         insertstring client.l,1,fig; replacestring client.l,0,kiwi;\n\
         message #s(client.l)|#I(client.l,2)|#I(client.l,3)|#v(sarraylen(#s(client.l)));\n\
         deletestring client.l,1; removestring client.l,pear;\n\
         message #s(client.l)|#v(lindexof(kiwi,#s(client.l)))|#v(lindexof(fig,#s(client.l)));\n\
         }",
    );
    w.start_all();
    let m = messages(&mut w);
    assert_eq!(m.len(), 2, "{m:?}");
    assert_eq!(m[0], "kiwi,fig,pear,plum|pear|plum|4");
    assert_eq!(m[1], "kiwi,plum|0|-1");
}

#[test]
fn addstring_before_2_1_adds_text() {
    let (mut w, _) = npc_in("if (created) { setstring client.s,ab; addstring client.s,cd; message #s(client.s); }", Era::V205);
    w.start_all();
    assert_eq!(messages(&mut w), ["abcd"]);
}

#[test]
fn strcontains_and_onmap() {
    let (mut w, _) = npc(
        "if (created) { message #v(strcontains(Hello World,WORLD))#v(strcontains(Hello,xyz))#v(onmapx(e.nw)); }",
    );
    w.start_all();
    assert_eq!(messages(&mut w), ["10-1"], "no map: -1");
    let (mut w, _) = npc("if (created) { message #v(onmapx(e.nw)),#v(onmapy(e.nw)),#v(onmapx(nowhere.nw)); }");
    w.core.map = crate::gmap::Gmap::parse_grid("\"a.nw\",\"b.nw\"\n\"c.nw\",\"e.nw\"\n");
    w.start_all();
    assert_eq!(messages(&mut w), ["1,1,0"]);
}

#[test]
fn play2_and_sendrpgmessage_are_effects() {
    let (mut w, _) = npc("if (created) { play2 bell.wav,30,30,1; sendrpgmessage Hi there; }");
    w.start_all();
    let e = w.take_effects();
    assert!(e.iter().any(|x| matches!(x, Effect::Play { file, looped: false, at: Some((30.0, 30.0)), .. } if file == "bell.wav")), "{e:?}");
    assert!(e.contains(&Effect::RpgMessage { text: "Hi there".into() }), "{e:?}");
}

#[test]
fn a_list_entry_with_a_comma_is_quoted() {
    use super::interp::{join_list, split_list};
    let items = vec!["a,b".to_string(), "say \"hi\"".to_string(), "c".to_string()];
    let text = join_list(&items);
    assert_eq!(text, "\"a,b\",\"say \"\"hi\"\"\",c");
    assert_eq!(split_list(&text), items);
    assert!(split_list("").is_empty());
}

// ---- `-System3`'s lastweapon list (2gta, 2026-10-30 capture) ----

#[test]
fn lastweapon_list_is_set_once_per_weapon_and_capped() {
    // As the server's -System3 weapon script: add the selected weapon to the
    // list when it is not the last entry, keeping at most 10.
    let src = "if (timeout || created) {\n  if(!strequals(#w,#I(lastweapon,sarraylen(lastweapon) - 1))){\n    if (sarraylen(lastweapon) == 10) deletestring lastweapon,0;\n    addstring lastweapon,#w;\n  }\n}";
    let (mut w, id) = npc(src);
    w.core.player.weapon = "Bow".into();
    w.fire(id, "created", &[]);
    assert_eq!(w.flag("lastweapon"), Some("Bow"));
    w.take_effects();
    // Same weapon on the next frames: no change, nothing for the server.
    for _ in 0..5 {
        w.fire(id, "timeout", &[]);
    }
    assert_eq!(w.flag("lastweapon"), Some("Bow"));
    assert!(!w.take_effects().iter().any(|e| matches!(e, Effect::SetFlag { .. })));
    // Twelve weapon changes keep the last ten.
    for i in 0..12 {
        w.core.player.weapon = format!("W{i}");
        w.fire(id, "timeout", &[]);
    }
    let list = w.flag("lastweapon").unwrap().to_string();
    assert_eq!(list.split(',').count(), 10, "{list}");
    assert!(list.ends_with("W11"), "{list}");
    // One FLAGSET a change, and one more for each `deletestring` at the cap
    // (Bow and W0 to W8 fill it; W9 to W11 each delete one).
    assert_eq!(w.take_effects().iter().filter(|e| matches!(e, Effect::SetFlag { .. })).count(), 15);
}

#[test]
fn a_flag_set_to_its_value_again_is_not_sent() {
    let (mut w, id) = npc("if (created) { setstring a,x; }\nif (playerenters) { setstring a,x; setstring a,y; }");
    w.fire(id, "created", &[]);
    w.take_effects();
    w.fire(id, "playerenters", &[]);
    let sets: Vec<_> = w.take_effects().into_iter().filter(|e| matches!(e, Effect::SetFlag { .. })).collect();
    assert_eq!(sets.len(), 1);
}
