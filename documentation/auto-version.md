# Picking the version by itself (consumer mode)

Date: 2026-11-25. Status: the picker is in core (`protocol::version_pick`);
the launcher side is the follow-up plan at the end.

## Why

Seihai's start window asks the player to choose a client version before it
fetches the server list. That suits development, where the version is the
thing under test. A player only wants to join a server, and should not need to
know that one server takes 2.22 and 5.00r7 while another takes 1.41 and 6.x.

Consumer mode picks the version and its profile for each server. It asks the
player only when the server takes two kinds of client that play different
worlds, with nothing in between (for example 1.41 and 5.12 to 6.03r7).

## Part of Seihai, not a new client

Consumer mode is a mode of the `seihai` program, and developer mode keeps
today's version box. A separate client would need its own launcher, hand-off
and settings, and those would drift from Seihai's. All the inputs are already
in core: the profiles, the token table, the login, and the refusal text
(`SessionState::disconnect_message`). The new parts are a small picker in core
and a change to the launcher.

## Where the allowed list comes from

- **Only from a refusal.** GServer-v2 checks the login token against
  `config/allowedversions.txt` and refuses with S>C 16 (`PLO_DISCMESSAGE`):
  `Your client version is not allowed on this server.` + `\r` +
  `Allowed: <list>` (`PlayerClient.cpp`). A secure login (`SEI00001`) is
  refused the same way before the password is sent (`PlayerLogin.cpp`).
- **The list format.** Each entry is a token, printed by its gs2lib name
  (`2.22`, `5.07`), or a range `A:B`, printed `A - B`. A range takes every
  token whose position in gs2lib's `clientVersions[]` lies between the two
  ends. A token that is not in the server's table is `CLVER_UNKNOWN` and is
  never taken. The `[generation-range]` form (`classic`, `newmain`, `modern`)
  prints the same way.
- **Not from the listserver.** The listserver does not check versions, and
  its server list has no allowed list. Its `version` field is the server
  program's own version, such as `Beta Version: 4.0.0` (Graal_analysis
  `server-notes/public-allowed-versions.md`).
- **Not seen in advance.** The client cannot see the server's gs2lib table.
  The picker reads names and ranges against seihai-gs2lib's order
  (`version_pick::GS2LIB`), which keeps stock entries in stock order. Inside a
  range, a token that only seihai-gs2lib has (3.153, the Mac and Linux twins,
  5.24) is only `Admit::Unsure`, because a stock server refuses it.

## Ranking (`version_pick::ranked`)

The candidates are the profiles the list takes, minus the ones already
refused on this server. They are ranked by these rules, in order:

1. **3D.** For a server listed with the `3 ` prefix, a build with a
   `three_d` row (3.04 to 5.12).
2. **Sure.** Named in the list, or a stock entry inside a range, ahead of
   `Unsure`.
3. **Profile quality.** Read from a binary (`Basis::Binary`) ahead of
   inferred.
4. **Default listed.** One of `reborn_versions` (one row per release, the
   stock token) ahead of twins and minor builds.
5. **Newest.**

## When to ask (`version_pick::pick`)

- Walk the default-listed versions in order and group the ones the server
  takes into runs. A run breaks at a listed version the server does not take.
- Each version has an `Era`, taken from GServer-v2's generations: Classic
  (before 2.00), NewMain (2.00 to 5.00r7, where the client runs GS1) and Modern
  (5.12 and later, GS2).
- **Ask** only when there are two or more runs and a later run shares no era
  with the first. The card offers each era's best version, with the overall
  best first and recommended.
- Otherwise pick at once. A gap inside one era is not a question: for
  `2.22, 5.07` (Graal the Adventure), 5.00r7 is picked. A single range that
  crosses eras, such as `1.41r1 - iPhone Belote 1.5` (S2), is not a question
  either: 6.03r7 is picked.
- The answer is saved for that server, so the question comes once.

## The join flow and fallbacks

1. **First try.** Use the version remembered for this server if there is
   one. Otherwise use `version_pick::first_try`: the newest default-listed
   build, or the newest one with a 3D side for a 3D server. If the server
   takes it, nothing else happens.
2. **Refused with a list.** Parse it (`AllowedList::from_disconnect`), add
   the refused version to `Need::refused`, and `pick`. Log in again with the
   pick, or show the card.
3. **Refused again.** For example, S2 refused 2.05 inside a range that covers
   it in stock order, so its table is older. Mark that version as refused
   and pick again. After three tries, stop and show the list with the
   developer version box.
4. **Refused with no list, or closed (GServer-RS).** Try the next
   era's best once, in the order Modern, NewMain, Classic. If that also
   fails, stop with the server's message.
5. **`Pick::Nothing`.** The list names only builds Seihai has no profile
   for, such as iPhone builds or Worlds. Show the list and say that Seihai
   cannot join this server.
6. **Remember** the parsed list and the version that worked, per server
   (`host:port`). When a remembered version is refused later, the server's
   list has changed: forget it and start again at step 2.

A classic login sends the password in its first frame, so a refused try has
already sent it to that server. This is the same server the player chose,
and that login would have sent the password anyway. A secure login is refused
before the password is sent.

## Developer mode

- Developer mode is a checkbox in the start window, saved as `mode=dev` in
  `launcher.conf`, and the `--version` argument. It keeps today's version
  box, the "All versions" list and per-version asset folders, and it never
  re-picks after a refusal.
- In consumer mode the version box is hidden. The picked version shows next
  to the Join button ("Joins as 6.03r7"), and a "Change" link opens the
  developer box for that one join.
- `--version` on the command line always wins (developer use and the lab).
  `--auto-version` asks a client for consumer mode when it is started without
  the launcher.

## Follow-up plan (seihai repo, after this core change is released)

1. Launcher (`seihai-play/src/start.rs`): a `Mode` field, consumer mode as
   the default for new settings and developer mode for existing
   `launcher.conf` files, so current users see no change. Per-server
   `allowed.<host:port>` and `picked.<host:port>` keys in `launcher.conf`.
2. Hand-off: when a client's login is refused with a version message, the
   client returns to the launcher with that message, as it does now for a
   lost connection. The launcher parses the message, picks again and hands
   off again. The clients do not need any version logic.
3. Card: two to four buttons, each era's name and version label, with
   "Remember for this server" checked.
4. Server list fetch: consumer mode fetches the list in a fixed list shape
   instead of the picked version's (the listserver answers by login type,
   `protocol/listserver-login-type-matrix-2026-10-25.md`). Which shape to use
   is decided in that PR.
5. Tests: launcher unit tests for the refusal, re-pick and hand-off loop,
   and a lab run against seihai-gserver-v2 with its three
   `[generation-range]` configs and the two recorded public lists.

## Open

- Whether 5.00r7 or 2.22 should win on a NewMain server that takes both.
  The rule picks the newest, but Ato still draws 3.x to 5.x flat. If that
  matters, a per-line maturity weight can go between rules 4 and 5.
- Stock gs2lib's spelling of some names (for example `2.05` and `2.05r2`).
  The table accepts both, and a name it does not know is kept as
  `AllowedItem::Unknown` and takes nothing.
