# seihai-core

The shared library of the Seihai suite: protocol, assets, scripting (the
GS1 engine, the GS2 VM and the GS1/GS2 compiler), the offline simulation,
the version profiles and the connection code. Nothing
in `seihai-core` draws or opens a window.

The repo also holds `seihai-draw`, a small UI-less crate that paints a
level's tiles and one gani frame into an RGBA buffer, for tools (the
server tools' LAT previews). It is tagged with core, so a tool pins one tag
for both.

This crate used to live inside [squirelworks/seihai](https://github.com/squirelworks/seihai)
as `crates/seihai-core`. Its history was carried over with `git filter-repo`,
so `git log` and `git blame` go back to the first commit that touched it.

## Using it

Between releases, a consumer's `main` follows core's `main`. `Cargo.lock`
keeps the exact commit, and `cargo update -p seihai-core -p seihai-draw` moves
it in the PR that needs it:

```toml
seihai-core = { git = "https://github.com/squirelworks/seihai-core", branch = "main", default-features = false }
```

and, for drawing (the same source, so a `Level` or `Gani` from the parser is
the type `seihai-draw` paints):

```toml
seihai-draw = { git = "https://github.com/squirelworks/seihai-core", branch = "main" }
```

A consumer's release pins a published tag instead (`tag = "vX.Y.Z"`). Test
builds against a local checkout use an override that isn't committed. See
[VERSIONING.md](VERSIONING.md), "Pinning seihai-core".

Features: `dev` (developer mode) and `audio` (sound output through rodio,
needs the ALSA development files on Linux). Both are on by default.

Crates that share a lock file have to name core the same way, so that the
lock file holds only one seihai-core (#79).

## Building and testing

First time: install Rust from [rustup.rs](https://rustup.rs/), then clone the repo.
On Linux, the default `audio` feature needs the ALSA development files
(`sudo apt-get install libasound2-dev` on Debian and Ubuntu); leave the feature out
if you do not want them.

```sh
git clone https://github.com/squirelworks/seihai-core.git
cd seihai-core
cargo build --workspace
cargo test --workspace
# without sound, as Linux CI builds it
cargo test --workspace --no-default-features --features dev
```

`seihai-core` is a library, so there is no program to run. The clients and
servers that use it are in their own repos (see the
[seihai README](https://github.com/squirelworks/seihai)).

## Command-line tools

A few developer tools ship as Cargo examples. Run them from the repo root with
`cargo run -p seihai-core --example NAME -- ARGS`.

| Example | What it does | Arguments |
| --- | --- | --- |
| `gs1c` | Compiles GS1 client scripts with Seihai's compiler, prints the disassembly and what each client build would be sent, and writes `FILE.gs2bc` next to each input | `FILE...` (one or more script files); `--era LABEL` picks the GS1 command set by client build, such as `1.39`, `2.05` or `2.17` (default `2.17`); `--out DIR` writes the output there instead; `--packet NPCID` also writes `FILE.plo131.bin`, the packet pair GServer-v2 would send for that NPC |
| `profile_audit` | Prints the version-profile audit | no argument for the TSV (`documentation/profile-audit.tsv`); `summary` for the summary table; `connections` for `documentation/connection-capabilities.md` |
| `gs1_fuzzcases` | Writes GS1 script fuzz cases for a live client lab, with the result Seihai predicts, as TSV on standard output | `--seed N` (default 1), `--count N` (default 200), `--first-id N` (default 40000), `--only FAMILY` (`expr`, `code`, `cond`, `limit` or `text`) |
| `gs2_fuzzcases` | The same for GS2 bytecode | `--seed N`, `--count N`, `--first-id N`, `--only FAMILY` (see the comment at the top of the file for the families) |

Some tests read real Graal files that are not in the repo. They are marked
`#[ignore]` and take their input from an environment variable:

| Variable | Used by |
| --- | --- |
| `SEIHAI_DATA_222` | the 2.22 data folder, for the asset coverage test |
| `SEIHAI_GANI_DIR` | a `levels/ganis` folder, for `real_ganis` |
| `SEIHAI_SOUNDS_DIR` | a sounds folder, for `real_sounds`; `SEIHAI_DUMP_DIR` also writes the decoded sound there |
| `SEIHAI_SCRIPT_CORPUS` | a file of real scripts, for the `corpus` test; `SEIHAI_CORPUS_SHOW` prints the named scripts' results |
| `SEIHAI_VIEWER_SAMPLE_OUT` | where `write_sample_pcap` writes its sample capture |

For example: `SEIHAI_GANI_DIR=path/to/levels/ganis cargo test real_ganis -- --ignored`.

## Versioning

[VERSIONING.md](VERSIONING.md) says when a release happens, what the
numbers mean before 1.0, and what 1.0.0 needs. In short: a merge doesn't make
a release. A PR that changes the public API adds a line under `## Unreleased`
in `CHANGELOG.md` and leaves the version alone. A release PR (with
`Release: vX.Y.Z` in its body) bumps `[workspace.package] version` and adds
`release-notes/vX.Y.Z.md`. Once CI passes on `main`, the release workflow
builds and tests on macOS and publishes the release with its tag.
Before 1.0, a minor bump (0.6 to 0.7) is for a breaking change or a finished
milestone, and a patch is for everything else. Next milestone: #108. 1.0.0
criteria: #109.

CI builds Linux and Windows on every push and PR. macOS is built and
tested only at release: the release workflow's `macos` job runs on the
commit being released, and the release waits for it to pass (#78).

The `dev`, `difftest` and `viewer` modules are tools for the suite's own
binaries. They are hidden from the docs and can change in a patch release.

Before 0.1.0, every release was a 0.0.x bump, and any of them could break.

## Known gaps

- **GS1 flag writes.** GS1 lowered to GS2 reads player flags as
  `player.<flag>` and writes `set`, `unset` and `setstring` on a plain name
  as an assignment to `client.gs1_<flag>` (`= true`, `= "value"`, `= ""`
  to clear): real 5.24r4 and 4.20r8 clients have no GS2 `set()` and send
  only `client.` flags. seihai-gserver keeps it as `<flag>` and sends it
  straight back, so `player.<flag>` reads it in the same session and after
  a relogin (`GS1_FLAG_PREFIX`, #30). Other servers keep
  `client.gs1_<flag>`. A bare `set x;` is stored as `x=1`. Assigning to a
  flag's plain name (`gotkey = 2`), reading a flag inside `with`, and
  `set` on `local.`, `clientr.`, `server.` or `serverr.` names are refused.
- **GS2 VM coverage.** The 7.x client and the Mac 4.04r2 and 5.12 VMs have
  not been read (`documentation/gs2-vm-versions.md`).
- **Profile audit and lab checks** listed in the roadmap (#7).

## Layout

- `crates/seihai-core/` is the crate.
- `crates/seihai-draw/` is the drawing crate. It came from the seihai repo
  with its history (`git subtree`); it depends on `crates/seihai-core` by
  path.
- `fuzz/` is a nightly `cargo-fuzz` project (#80). It is excluded from the
  workspace so stable CI stays on the normal toolchain; see `fuzz/README.md`.
- `assets/default/` is the built-in default asset set, embedded with
  `include_bytes!`. It is generated by `tools/gen_default_assets.py` in the
  seihai repo; copy changes here when it is regenerated.
- `documentation/` holds the tables the crate embeds or tests against
  (`packet-layouts.tsv`, `version-chain.*`, `version-tokens.tsv`,
  `gs2-opcodes.md`, `gs2-vm-versions.md`) and the update cadence notes. The
  rest of the documentation stays in the seihai repo.
- **Version tokens and the version chain live here only.**
  `documentation/version-tokens.tsv` (generated from
  `protocol/tokens.rs`) lists every login token in Graal_analysis's
  `client-internals/version-token-catalogue.md`, with its evidence grade
  and the Seihai profiles that send it; `documentation/version-chain.tsv`
  is the per-profile chain. Other repos link to these files instead of
  keeping copies; a unit test fails when either is stale.
- `release-notes/` holds the notes for each tag; `CHANGELOG.md` is the
  one-line index.

## License

MIT OR Apache-2.0.

