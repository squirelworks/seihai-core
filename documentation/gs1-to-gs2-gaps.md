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
| `Missing` | Compile error. No 3.153 shared-command form is known yet. |

`gs1_commands_without_gs2()` lists every name that is not `Same` (so
`Renamed` / `Statements` / `Missing` all appear there).
`gs1_gap_priorities()` is the #93 hit table for triage.

A `Missing` name must not be emitted as a silent `Same` call: the 3.153
client tables do not register it, so a real GS2 client would report
"Function not found" at run time. Prefer a lab-backed `Statements` or
`Renamed` form, or keep the compile error.

## Top failing-unit hits (#93)

| GS1 name | failing units | Current class | Notes |
|---|---:|---|---|
| `putexplosion` | 424 | `Missing` | Not in 3.153 tables. `explodebomb` is index-based, not a put. |
| `puthorse` | 249 | `Missing` | Not in 3.153 tables. `takehorse` removes by index. |
| `setbackpal` | 239 | `Missing` | Removed from GS1 tables at 3.04; absent from 3.153 GS2. |
| `setstring` | 188 | `Statements` | Partial: plain / `this.` / `client.` forms lower; residual flag forms still refuse (see README Known gaps). |
| `putnpc` | 158 | `Missing` | GS1 NPC spawn; no 3.153 client command. |
| `setbody` | 135 | `Missing` | Removed at 3.04; GS2 keeps `sethead` only. |
| `putbomb` | 134 | `Missing` | Not in 3.153 tables. |
| `shoot` | 126 | `Missing` | Not in 3.153 tables; `shootarrow` / `setshootparams` are different shapes. |
| `putnpc2` | 104 | `Missing` | Server-oriented in 2.17–3.04 notes; not a 3.153 client command. |
| `putexplosion2` | 86 | `Missing` | Same family as `putexplosion`. |
| `addstring` | 80 | `Statements` | Partial: only `this.` / `client.` lists; plain GS1 string lists refuse. |

Smaller #93 counts (examples): `putcomp` / `drawovertrees` / `replacestring` /
`timershow` (21); `savelog2` (18); `removecompus` (15); `sendpm` /
`sendtonc` / `putobject` (~9–10); others ≤6. Several inventory names had
**0** failing-unit hits in that run (low priority unless product need says
otherwise).

## Inert NoExec → omit

These GS1 names are `NoExec` on 2.05 (the classic client accepts them and
does nothing) and are **not** in the 3.153 shared-command list. The front
end now **omits** the call (`Statements`: "omitted…"):

`seturllevel`, `setbacktile`, `setbacktile2`, `canwarp`, `canwarp2`,
`drawovertrees`, `freezeplayer2`, `unfreezeplayer`, `setz`.

`setfocus`, `resetfocus`, and `updateterrain` are also `NoExec` in GS1 but
**are** registered in 3.153, so they stay `Same`. `join` stays the special
`join("class")` call.

## What is not in this pass

No `Same` / `Renamed` inventing for the high-hit Effect commands above.
That needs a documented GS2 form (shared-command table, opcode notes, or
lab capture), not a Seihai-only host polyfill. Track progress by moving a
name out of `Missing` in `gs1_command_lowering` and updating the tests in
`tests/gs1_gap_priorities.rs`.

## Inventory API

```text
gs1_command_lowering(name) -> Option<Gs1Lowering>
gs1_commands_without_gs2() -> Vec<&str>
gs1_gap_priorities()       -> &[Gs1GapPriority]  // #93 aggregates
```
