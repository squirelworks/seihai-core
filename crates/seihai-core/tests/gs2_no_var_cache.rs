//! `Options::no_var_cache`: a script for a client before 6.x is compiled
//! without the per-function variable cache. Only the 6.x loaders fold the
//! cache's `makeprop` + `setreg` into one opcode that keeps a cached name a
//! name; on 4.x and 5.x a call through a cached command name finds nothing
//! (Graal_analysis `client-internals/gs2-vm-quirks-by-version.md`).

use seihai_core::scripting::gs2::compile::{self, Language, Mode, Options};
use seihai_core::scripting::gs2::Op;

const SRC: &str = r#"
function onCreated() {
  for (temp.i = 0; temp.i < 3; temp.i++)
    echo("tick " @ temp.i);
  this.n = 1; this.n++; this.n++; this.n++; this.n++;
}
"#;

fn ops(language: Language, no_var_cache: bool) -> Vec<u8> {
    let c = compile::compile_with(
        SRC,
        language,
        Options {
            mode: Mode::Legacy,
            no_var_cache,
            ..Options::default()
        },
    )
    .unwrap();
    c.script.code.iter().map(|i| i.op.value()).collect()
}

fn uses_cache(ops: &[u8]) -> bool {
    ops.iter().any(|&o| (45..=47).contains(&o))
}

#[test]
fn the_default_keeps_the_official_cache() {
    assert!(uses_cache(&ops(Language::Gs2, false)));
}

#[test]
fn no_var_cache_leaves_out_opcodes_45_to_47() {
    let plain = ops(Language::Gs2, true);
    assert!(!uses_cache(&plain), "{plain:?}");
    // The script still calls echo by name.
    let c = compile::compile_with(
        SRC,
        Language::Gs2,
        Options {
            no_var_cache: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(c.script.strings.iter().any(|s| s == "echo"));
    assert!(c.script.code.iter().any(|i| i.op == Op::Call));
}
