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

Seed corpora under `fuzz/corpus/` come from the crate's test fixtures
(no real hosts or account captures).
