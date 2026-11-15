# seihai-core roadmap

The live checklist is issue #7; the suite-wide plan is squirelworks/seihai#353.
This page groups that work into three phases so "finished" has a clear meaning
for this crate. Last checked 2026-11-15 against `main` (`v0.3.0`).

## Phase 1: to functional

seihai-core is functional when every consumer (seihai, seihai-gserver,
seihai-listserver, seihai-npcserver, seihai-rc, seihai-servertools) builds on
one tag and the protocol, assets, GS1 and GS2 layers cover the builds those
consumers play.

- [ ] One seihai-core in every lock file, and gserver's temporary Cargo patch removed (#79).
- [ ] GS1 to GS2 lowering, the remaining slices (#6).
- [ ] GS2 VM coverage for the Mac 4.04r2 and 5.12 VMs and the 7.x client (#7, item 4).
- [ ] `.code` files for 1.x (#63).
- [ ] Profile lab checks still open in #7 (5.12 to 5.31r4 ids, `PLI_BADDYHURT` byte order, laying items).
- [ ] No public addresses in source, tests or docs, with a CI check (#81).

## Phase 2: polish and release

- [ ] Fuzz targets for every decoder that sees network or world-file input, with every crasher fixed (#80).
- [ ] macOS build and tests before a tag is published (#78). macOS stays off the per-push CI.
- [ ] Release notes per tag in `release-notes/`; tags only with the owner's go-ahead.
- [ ] Public API pass: mark internal modules `pub(crate)`, document each public module, and write down the semver rule consumers rely on (what counts as breaking, as v0.3.0's GANI change did).
- [ ] `cargo doc` builds without warnings; docs published with each release.

## Phase 3: after it works

- **A stable 1.0 API** that third parties can build tools, bots and servers on.
- **crates.io publication** of the clean-room crates once the licence and naming are settled with the owner.
- **Bindings**: a C ABI or WebAssembly build of the asset and protocol layers, so the level/gani/GS1 tooling can run in a browser (a web level viewer or script playground).
- **Conformance corpus**: the authored fixtures from Graal_analysis (TRM 0.0.7) shipped as a test suite any implementation can run.
- **Performance**: benchmarks for packet encode/decode and the GS2 VM, tracked per release, ahead of large stress runs.
- **Preservation**: freeze each released profile table as data (TSV) so it outlives the code.
