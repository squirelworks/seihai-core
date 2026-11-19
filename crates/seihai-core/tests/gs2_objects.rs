//! GS2 objects made with `new`, `with` on them, `makevar`, and the
//! trigger commands of the effect host (seihai-core roadmap #7, "Delteria
//! GS2 that compiles but does not run").

use seihai_core::scripting::engine::{Effect, Era, Val};
use seihai_core::scripting::gs2::compile::compile_gs2;
use seihai_core::scripting::gs2::exec::{EffectHost, Machine, Outcome, Root};
use seihai_core::scripting::gs2::lists::Value;

fn run(src: &str) -> (Machine, EffectHost) {
    let mut m = Machine::new(compile_gs2(src).unwrap());
    let mut h = EffectHost::new(Era::V217, 7);
    assert_eq!(m.run_top_level(&mut h), Outcome::Done);
    assert_eq!(m.call_event("onCreated", &[], &mut h), Outcome::Done);
    (m, h)
}

fn not_skipped(h: &EffectHost) {
    assert!(
        !h.log
            .iter()
            .any(|l| l.contains("not run yet") || l.contains("skipped")),
        "{:?}",
        h.log
    );
}

#[test]
fn an_anonymous_static_var_keeps_its_members_and_type() {
    let (m, h) = run("function onCreated() {\n\
                        temp.o = new TStaticVar();\n\
                        temp.o.a = 3;\n\
                        this.r = temp.o.a + 1;\n\
                        this.t = temp.o.objecttype();\n\
                        this.copy = temp.o;\n\
                        this.copy.b = 5;\n\
                        this.s = temp.o.b;\n\
                      }");
    not_skipped(&h);
    assert_eq!(m.var(Root::This, "r"), Some(&Val::Num(4.0)));
    assert_eq!(m.var(Root::This, "t"), Some(&Val::from("TStaticVar")));
    // Two variables that hold the object reach the same members.
    assert_eq!(m.var(Root::This, "s"), Some(&Val::Num(5.0)));
    assert_eq!(m.objects().count(), 1);
}

#[test]
fn two_anonymous_objects_are_two_objects() {
    let (m, _) = run("function onCreated() {\n\
                        temp.a = new TStaticVar();\n\
                        temp.b = new TStaticVar();\n\
                        temp.a.v = 1; temp.b.v = 2;\n\
                        this.va = temp.a.v; this.vb = temp.b.v;\n\
                      }");
    assert_eq!(m.var(Root::This, "va"), Some(&Val::Num(1.0)));
    assert_eq!(m.var(Root::This, "vb"), Some(&Val::Num(2.0)));
    assert_eq!(m.objects().count(), 2);
}

#[test]
fn a_named_object_is_a_global_name() {
    let (m, h) = run("function onCreated() {\n\
                        temp.o = new TStaticVar(\"Store\");\n\
                        Store.gold = 12;\n\
                        this.g = temp.o.gold;\n\
                        this.t = Store.objecttype();\n\
                      }");
    not_skipped(&h);
    assert_eq!(m.var(Root::This, "g"), Some(&Val::Num(12.0)));
    assert_eq!(m.var(Root::This, "t"), Some(&Val::from("TStaticVar")));
}

#[test]
fn a_new_block_sets_the_control_inside_and_hands_it_to_addcontrol() {
    let (m, h) = run("function onCreated() {\n\
                        new GuiWindowCtrl(\"MyWin\") {\n\
                          profile = \"GuiBlueWindowProfile\";\n\
                          this.width = 5;\n\
                          text = \"Hi\";\n\
                          new GuiButtonCtrl(\"MyWin_Ok\") { text = \"OK\"; }\n\
                        }\n\
                        MyWin.extent = 4;\n\
                        with (MyWin) { height = 2; this.w2 = width * 2; }\n\
                        this.text = MyWin.text;\n\
                        this.own = width;\n\
                      }");
    assert!(
        !h.log.iter().any(|l| l.contains("not run yet")),
        "{:?}",
        h.log
    );
    let mut objects: Vec<_> = m.objects().collect();
    objects.sort();
    assert_eq!(
        objects,
        [("MyWin", "GuiWindowCtrl"), ("MyWin_Ok", "GuiButtonCtrl")]
    );
    let get = |o: &str, n: &str| m.object_var(o, n).cloned();
    assert_eq!(
        get("MyWin", "profile"),
        Some(Value::Val(Val::from("GuiBlueWindowProfile")))
    );
    assert_eq!(get("MyWin", "width"), Some(Value::Val(Val::Num(5.0))));
    assert_eq!(get("MyWin", "text"), Some(Value::Val(Val::from("Hi"))));
    assert_eq!(get("MyWin", "extent"), Some(Value::Val(Val::Num(4.0))));
    assert_eq!(get("MyWin", "height"), Some(Value::Val(Val::Num(2.0))));
    assert_eq!(get("MyWin", "w2"), Some(Value::Val(Val::Num(10.0))));
    assert_eq!(get("MyWin_Ok", "text"), Some(Value::Val(Val::from("OK"))));
    // Outside the blocks, plain names are the script's own again.
    assert_eq!(m.var(Root::This, "text"), Some(&Val::from("Hi")));
    assert_eq!(m.var(Root::This, "own").map(Val::num), Some(0.0));
    // Each block's object goes to `addcontrol`, which this host lacks.
    assert_eq!(
        h.log
            .iter()
            .filter(|l| l.contains("function addcontrol not found"))
            .count(),
        2,
        "{:?}",
        h.log
    );
}

#[test]
fn destroy_removes_the_object_and_its_variables() {
    let (m, _) = run("function onCreated() {\n\
                        temp.o = new TStaticVar(\"Gone\");\n\
                        Gone.a = 1;\n\
                        Gone.destroy();\n\
                        this.a = Gone.a;\n\
                      }");
    assert_eq!(m.objects().count(), 0);
    assert_eq!(m.object_var("Gone", "a"), None);
    assert_eq!(m.var(Root::This, "a").map(Val::num), Some(0.0));
}

#[test]
fn with_on_a_variable_path_reaches_its_variables() {
    let (m, h) = run("function onCreated() {\n\
                        this.part.hp = 3;\n\
                        with (this.part) { hp += 1; mp = 2; }\n\
                        this.hp = this.part.hp;\n\
                        this.mp = this.part.mp;\n\
                      }");
    not_skipped(&h);
    assert_eq!(m.var(Root::This, "hp"), Some(&Val::Num(4.0)));
    assert_eq!(m.var(Root::This, "mp"), Some(&Val::Num(2.0)));
}

#[test]
fn makevar_names_a_variable_at_run_time() {
    let (m, h) = run("function onCreated() {\n\
                        for (temp.i = 0; temp.i < 3; temp.i++)\n\
                          makevar(\"this.slot\" @ temp.i) = temp.i * 10;\n\
                        this.got = makevar(\"this.slot2\");\n\
                        makevar(\"plain\") = 7;\n\
                        this.p = plain;\n\
                      }");
    not_skipped(&h);
    for i in 0..3 {
        assert_eq!(
            m.var(Root::This, &format!("slot{i}")),
            Some(&Val::Num(i as f64 * 10.0))
        );
    }
    assert_eq!(m.var(Root::This, "got"), Some(&Val::Num(20.0)));
    assert_eq!(m.var(Root::This, "p"), Some(&Val::Num(7.0)));
}

#[test]
fn triggeraction_and_triggerserver_become_trigger_actions() {
    let (_, h) = run("function onCreated() {\n\
                        triggeraction(10.5, 20, \"Open\", \"red\", 3);\n\
                        triggerserver(\"gui\", \"-Shop\", \"buy\", 2);\n\
                        triggerserver(\"gui\", \"-Ping\");\n\
                      }");
    assert_eq!(
        h.effects,
        [
            Effect::TriggerAction {
                x: 10.5,
                y: 20.0,
                action: "open".into(),
                params: "red,3".into()
            },
            Effect::TriggerAction {
                x: 0.0,
                y: 0.0,
                action: "serverside".into(),
                params: "-Shop,buy,2".into()
            },
            Effect::TriggerAction {
                x: 0.0,
                y: 0.0,
                action: "serverside".into(),
                params: "-Ping".into()
            },
        ]
    );
}
