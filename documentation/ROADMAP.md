# seihai-core roadmap

Working mirror of [issue #7](https://github.com/squirelworks/seihai-core/issues/7).
The suite-wide plan is squirelworks/seihai#353. This page groups that work
into three phases so "finished" has a clear meaning for this crate.
Last checked 2026-11-19 against `main` at `eb1a732` (after #107), plus
the 0.6.1 work. Tags are cut with the owner's go-ahead.

## Where it stands

- **In the crate:** protocol, assets, the GS1 engine per client build, the
  GS2 compiler/VM by profile, version profiles, the client session, combat
  rules, sound, `seihai-draw`, 1.1.9 relay networking (#86 / seihai#542),
  and a weekly `cargo-fuzz` scaffold under `fuzz/` (#90; excluded from the
  workspace).
- **Pin round #79 is closed:** one core in the suite lock files and
  gserver's temporary `[patch]` removed (seihai-rc#61, seihai-gserver#151,
  seihai-servertools#95).
- **Public-address scrub #81 is closed:** list examples use documentation
  ranges; CI fails on public IPv4 literals outside the allow-list.

## Phase 1: to functional

seihai-core is functional when every consumer builds on one tag and the
protocol, assets, GS1 and GS2 layers cover the builds those consumers play.

- [x] One seihai-core in every lock file, and gserver's temporary Cargo
      patch removed (#79).
- [x] No public addresses in source, tests or docs, with a CI check (#81).
- [ ] GS1 to GS2 lowering, the remaining slices (#6): unnamed-event
      wrapper coverage, residual unsupported forms, and
      original-client/control acceptance. Named-event structured-lift
      ordering is already fixed (#71–#73). Synthetic regressions are not
      native all-version parity. Gap triage by corpus fail hits is #93
      (`documentation/gs1-to-gs2-gaps.md`); `setbody`, plain `addstring`,
      `savelog2`, and engine-removed omits (`setbackpal`, `putobject`) now
      lower; `putbomb` and `shoot` call through by name, as real GS2
      weapons do (0.5.1) — from 0.6.2 `putexplosion(2)` and `puthorse`
      call through too (lab #104: real 4.20r8 and 5.12 run them);
      `putnpc2` stays `Missing` (those clients refuse it) and `putnpc` is
      untested.
- [ ] GS2 VM coverage for the Mac 4.04r2 and 5.12 VMs and the 7.x client
      (`documentation/gs2-vm-versions.md`). The 7.x opcode set is now
      read (Graal_analysis `protocol/gs2-opcode-table.md`, PR #630):
      7.12.43 dispatches the same 165 bytes as 5.30r3 to 6.1.1.3, and so
      do Android Classic 1.8 and 2.2. Its per-opcode semantics are still
      unread. Two points for the compiler: `waitfor` (11) has no case
      before 4.20r8, so a script meant for 4.03r3 to 4.11 must not use
      it; and the Linux x64 6.x VMs also run 0 and a Linux-only 164,
      which the compiler never needs to emit. Of the Delteria features
      below, only `waitfor` needs more than the 4.03r3 opcode set.
- [ ] GS2 seen in the 2019 Delteria backup (Graal_analysis PR #625,
      counts in its note) that compiles but does not run yet. The
      compiler takes every feature the note counts; mixed GS1/GS2 NPCs
      and layers 1/2 are covered by `tests/mixed_npcs.rs` (0.5.0). Still
      open:
      - ~~`new Type(…) { … }` blocks and `new TStaticVar()`~~ run from
        0.6.1: objects live under the global object by name, `with`
        reaches them, `Machine::objects` lists them. Drawing the GUI
        controls (`addcontrol`) is the host's (seihai).
      - ~~`makevar(…) = v`~~ runs from 0.6.1.
      - ~~`triggerserver`, `triggeraction`, `.objecttype()`~~ from
        0.6.1 (`triggerserver` as GServer-v2 reads it, **[inf]**).
      - Host calls that need the host's object model: `findplayer`,
        `findplayerbycommunityname`, `findweapon`, `trigger(…)`. Server-only calls
        (`triggerclient`, `findlevel`, `findnpc`, `putnpc2`,
        `loadlines`/`savelines`, `loadvars`/`getstringkeys`,
        `requesttext`, `savelog`) wait on server-side GS2, which is out
        of the compiler's scope.
      - ~~GS1 message codes on plain strings (`#s(name)`)~~ read
        `player.<name>` from 0.6.2, as plain flag reads do; `local.` and
        `server.` strings and run-time plain names stay refused.
      - ~~`delivery` compiles client scripts as strict GS1~~: from
        0.6.1 a client part strict GS1 refuses is compiled as
        `Language::Detect` reads it.
- [x] Server trigger actions (#106): `PLO_TRIGGERACTION` (48) is
      decoded from 0.6.1; the field order is lab-confirmed on real 2.22,
      4.20r8 and 5.12 (0.6.2). Which script a real client runs for a
      received 48 is still unobserved.
- [ ] `.code` files for 1.x (#63): core codec shipped in 0.2.10 (#66);
      Seihai client-folder integration and observable guild/registration
      behaviour remain open.
- [ ] Profile lab checks still open in #7 (5.12 to 5.31r4 ids,
      `PLI_BADDYHURT` byte order, laying items; walk-update / gmap login
      bundling leftovers; non-local link first-load warps).

## Phase 2: polish and release

- [ ] Fuzz targets for every decoder that sees network or world-file
      input, with every crasher fixed (#80). **Scaffold landed** in #90
      (11 targets, seeds, weekly workflow). 0.6.2 adds `session_apply`
      and `gs2_exec`, and a 4-minute run of all 13 found one GS1 parser
      timeout, fixed. Longer runs remain — leave #80 open.
- [ ] macOS build and tests before a tag is published (#78). macOS stays
      off the per-push CI.
- [ ] Release notes per tag in `release-notes/`; tags only with the
      owner's go-ahead. After each tag, move consumers in one round so
      each lock file has one seihai-core.
- [ ] Public API pass: mark internal modules `pub(crate)`, document each
      public module, and write down the semver rule consumers rely on
      (what counts as breaking, as v0.3.0's GANI change did).
- [x] `cargo doc` builds without warnings, checked in CI with
      `RUSTDOCFLAGS=-D warnings` (0.6.1). Publishing the docs with each
      release is still open.

## Phase 3: after it works

- **A stable 1.0 API** that third parties can build tools, bots and
  servers on.
- **crates.io publication** of the clean-room crates once the licence and
  naming are settled with the owner.
- **Bindings**: a C ABI or WebAssembly build of the asset and protocol
  layers, so the level/gani/GS1 tooling can run in a browser.
- **Conformance corpus**: the authored fixtures from Graal_analysis
  (TRM 0.0.7) shipped as a test suite any implementation can run.
- **Performance**: benchmarks for packet encode/decode and the GS2 VM,
  tracked per release, ahead of large stress runs.
- **Preservation**: freeze each released profile table as data (TSV) so
  it outlives the code.

## Rules

Clean room as in seihai#353: no GPL gs2-parser/gs2lib code, behaviour from
notes and lab captures. No real hostnames or addresses in tests or docs;
loopback, documentation ranges, or `<host>`.
