# GS1→GS2 lowering gaps (issue #93)

Prioritization for GS1 commands that still have no complete GS2 form.
Related umbrella: [#6](https://github.com/squirelworks/seihai-core/issues/6).
Corpus ranking: [#93](https://github.com/squirelworks/seihai-core/issues/93).

**Privacy:** command names and aggregate failing-unit counts only. No
script text, paths, hosts, or world names.

## How to read a gap

Each GS1 command has a [`Gs1Lowering`](../crates/seihai-core/src/scripting/gs2/compile/tables.rs)
class:

| Class | Meaning |
|---|---|
| `Same` | GS2 has the same command or opcode; call by name. |
| `Renamed` | GS2 uses another name (`setgif` → `setimg`). |
| `Statements` | Becomes GS2 statements (assignments, list methods, or an intentional omit). |
| `Missing` | Compile error. No 3.153 shared-command form, and no real GS2 script calls it. |

`gs1_commands_without_gs2()` lists every name that is not `Same` (so
`Renamed` / `Statements` / `Missing` all appear there).
`gs1_gap_priorities()` is the #93 hit table for triage.

A `Missing` name must not be emitted as a silent `Same` call: the 3.153
client tables do not register it, so a real GS2 client would report
"Function not found" at run time. Prefer a lab-backed `Statements` or
`Renamed` form, or keep the compile error.

The one exception is a name that real compiled GS2 scripts call on 4.x to
6.x clients. The weapon corpus (Graal_analysis
`npc-library/gs2-weapon-corpus-2026-11-14.md`) has `shoot` in 25 weapons of
the 2007 set and 67 of the 2018 set, and `putbomb` in 5 of the 2018 set.
Both are `Same` (`GS2_CORPUS_COMMANDS` in `tables.rs`), with their GS1
argument order. That is an inference from shipped scripts **[inf]**, not a
lab run.

## Top failing-unit hits (#93)

| GS1 name | failing units | Current class | Notes |
|---|---:|---|---|
| `putexplosion` | 424 | `Missing` | Not in 3.153 tables. `explodebomb` is index-based, not a put. **TODO(lab):** needs a documented GS2 or server path. |
| `puthorse` | 249 | `Missing` | Not in 3.153 tables. `takehorse` removes by index. **TODO(lab).** |
| `setbackpal` | 239 | `Statements` | Omitted: removed from every C++ engine at 3.04; GS2-era clients never had it. |
| `setstring` | 188 | `Statements` | Plain / `this.` / `client.` forms lower; residual flag forms still refuse (see README Known gaps). |
| `putnpc` | 158 | `Missing` | GS1 NPC spawn; no 3.153 client command. **TODO(lab):** server `PUTNPC` path. |
| `setbody` | 135 | `Statements` | `player.bodyimg = file` (same as `setplayerprop #8`). |
| `putbomb` | 134 | `Same` | Not in 3.153 tables, but real 2018 GS2 weapons call `putbomb(power, x, y)`. |
| `shoot` | 126 | `Same` | Not in 3.153 tables, but real 2007 and 2018 GS2 weapons call `shoot(...)` next to `setshootparams`. |
| `putnpc2` | 104 | `Missing` | Server-oriented in 2.17–3.04 notes; not a 3.153 client command. **TODO(lab).** |
| `putexplosion2` | 86 | `Missing` | Same family as `putexplosion`. **TODO(lab).** |
| `addstring` | 80 | `Statements` | `this.` / `client.` lists, and plain names via `client.gs1_<flag>` like `setstring`. |

Smaller #93 counts (examples): `putcomp` / `drawovertrees` / `replacestring` /
`timershow` (21); `savelog2` (18, now `savelog(a @ b)`); `removecompus` (15);
`sendpm` / `sendtonc` / `putobject` (~9–10; `putobject` omitted like
`setbackpal`). Several inventory names had **0** failing-unit hits in that
run (low priority unless product need says otherwise).

## Inert / engine-removed → omit

These names have **no** 3.153 GS2 command. The front end **omits** the call
(`Statements`: "omitted…"):

**2.05 `NoExec`:** `seturllevel`, `setbacktile`, `setbacktile2`, `canwarp`,
`canwarp2`, `drawovertrees`, `freezeplayer2`, `unfreezeplayer`, `setz`.

**Removed from the C++ engine at 3.04** (never on a GS2-era client):
`setbackpal`, `putobject`.

`setfocus`, `resetfocus`, and `updateterrain` are also `NoExec` in GS1 but
**are** registered in 3.153, so they stay `Same`. `join` stays the special
`join("class")` call.

## What still needs a lab form

Do **not** invent a Seihai-only host polyfill for the spawn/combat family
below. A silent `Same` call would fail on real GS2 clients. Track progress
by moving a name out of `Missing` in `gs1_command_lowering` once a
shared-command, opcode, or server-assisted form is documented:

- `putexplosion`, `putexplosion2`, `puthorse`
- `putnpc`, `putnpc2`
- `timershow` (not the same as `timereverywhere`)
- `sendpm`, `setpm`, `sendtonc` (server-side in 2.17–3.04 notes)

## Inventory API

```text
gs1_command_lowering(name) -> Option<Gs1Lowering>
gs1_commands_without_gs2() -> Vec<&str>
gs1_gap_priorities()       -> &[Gs1GapPriority]  // #93 aggregates
```
