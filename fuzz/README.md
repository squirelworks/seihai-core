# Fuzz targets for seihai-core (#80)

Nightly-only `cargo-fuzz` project. It is **not** a workspace member, so
stable CI (`cargo test --workspace`) is unaffected.

## Setup

```
rustup toolchain install nightly
cargo install cargo-fuzz
```

## Run

From the repo root (set `CXX=g++` when the default `c++` is clang
without libstdc++ headers):

```
CXX=g++ cargo +nightly fuzz run level
CXX=g++ cargo +nightly fuzz run zlib_body -- -max_total_time=60
```

Crashers land under `fuzz/artifacts/<target>/`. Each one should become a
normal regression test plus a fix PR.

## Targets

| Target | Entry point |
| --- | --- |
| `level` | `assets::Level::from_bytes` (`.nw` / `.graal` / Z3) |
| `gani` | `assets::Gani::parse` |
| `gmap` | `gmap::Gmap::parse` |
| `zlib_body` | gen-2/3 zlib frame / body decompress |
| `list_reply` | listserver `parse_list_reply` / `parse_svrlist` |
| `player_props` | `parse_player_props` |
| `gs1_parse` | GS1 `engine::parse` |
| `gs2_compile` | GS1/GS2 compiler front end |
| `gs2_bytecode` | GS2 segment loader and OpenGS1-style bytecode |
| `gen3_decode` | gen-3 insertion decode (key = first byte) |
| `gen5_frame` | gen-5 length headers + body decode (key = first byte) |
| `session_apply` | every server→client decoder behind `SessionState::apply`, as 2.05, 2.22 and 5.12 |
| `gs2_exec` | the GS2 executor (top level, `onCreated`, one tick) on any bytecode the reader takes, 3.153 and 5.00r7 rules |

Seed corpora under `fuzz/corpus/` come from the crate's test fixtures
(no real hosts or account captures). Commit only those seeds; leave
libFuzzer-grown inputs local (they rewrite the same directories).

## Live differential cases (GS1 and GS2 scripts)

libFuzzer above only checks that Seihai's own readers never panic. Two
examples write cases for a **real client** in a lab instead, with what
Seihai predicts that client will do:

```
cargo run -p seihai-core --no-default-features --example gs2_fuzzcases -- --seed 1 --count 200 > gs2cases.tsv
cargo run -p seihai-core --no-default-features --example gs1_fuzzcases -- --seed 1 --count 150 > gs1cases.tsv
```

Each case is one NPC script that sets `player.chat` to `FZ<id>a`, waits,
computes one value and sets `player.chat` to `FZ<id>:<value>`, so the
client reports what its VM did through an ordinary player-prop packet.

- `gs2_fuzzcases`: compiled bytecode with fuzzed parts spliced in.
  Families `control`, `op` (every byte no compiler writes, the slot and
  literal opcodes 45-49, 54, 66, 67, 104, 121, 164, the fold bytes 200-242
  written raw, `waitfor` timeouts), `limit` (stack, loop, `maxlooplimit`
  and call depth around each profile's limit), `container` (damaged
  segments, references and operand markers) and `expr` (random
  expressions over edge values). One prediction column per VM profile
  (4.20r8, 5.24r4, 6.03r7), from `gs2::exec::Machine`.
- `gs1_fuzzcases`: GS1 script text (the clients never get GS1 as
  bytecode). Families `code` (message codes), `cond`, `limit`, `text`
  (damaged scripts) and `expr`. Predictions for the 2.05 and 2.17
  vocabularies, from the GS1 engine.

The lab side (a probe plan per case file, and scoring a run's proxy log
against the prediction columns) is in Graal_analysis
`tools/probe/scriptfuzz.py`. A case where the client and the prediction
differ is a lead for a fix here, once the lab note says which side is
right.
