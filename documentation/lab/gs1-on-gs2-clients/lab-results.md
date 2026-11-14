# Lab results: GS1 scripts as GS2 bytecode on original clients (#6)

Run on 2026-11-14 in a Debian lab VM. Each run was in its own private network
namespace with loopback only, so nothing reached the internet.

## Setup

- **Compiler.** `gs1c` from this branch. The first pass used `main` (0.2.4 era).
  The final pass used `e08d4d3`, which adds `setplayerprop #c` → `player.chat`
  (bf6a2d8) and `playerscount` → `players.size()` (e08d4d3).
- **Server.** Stock GServer-v2 (gs2emu, packet-logging build `fbd30767`), in
  two pieces:
  - a login server that `serverwarp`s to the lab world;
  - the lab world, `generation = modern`, with no NPC-server.
- **Lab NPC.** The world's start level holds one `block.png` NPC at 30,27,
  carrying a trivial placeholder client script. The player starts at 30,32.
- **Delivery.** A private lab gen-5 proxy sits in front of the world.
  It decrypts each server→client bundle and finds the
  `PLO_RAWDATA` + `PLO_NPCBYTECODE` (131) pair. It replaces the bytes after the
  GINT3 npc id with the `.gs2bc` from `gs1c`, fixes the RAWDATA length, then
  recompresses and re-encrypts. Each swap is logged with the sha256 of the
  bytes the client received.
- **Clients**, all original and unmodified:

  | Client | Build and runtime | Notes |
  |---|---|---|
  | 6.03r7 | Linux x64 build | Mesa software GL, own Xvfb |
  | 5.24r4 | Windows, under Wine | real clock |
  | 4.20r8 | Windows, under Wine | libfaketime at 2007-07-01 |
  | 2.22 | Windows, under Wine | control: GS1 text |

- **Art.** The world serves the 2.22-era art set plus `arial.ttf` and
  `arialbd.ttf`.
- **Touch scripts.** The driver holds Up to walk the player into the NPC, then
  backs off and pushes again (`c-touch` / `e-touch2` frames).

## Payloads (final pass, compiler e08d4d3)

| Script | bytes | SHA-256 prefix (8 hex) of the bytecode the client received |
|---|---|---|
| 01-chat-and-timer | 190 | `52364e51` |
| 02-touch-and-image | 202 | `a8faee9e` |
| 03-showimg-and-vars | 278 | `6b24bd2a` |
| 04-players-array | 191 | `09982911` |
| 05-tiles | 207 | `1da4eb6f` |
| 05e-tile632 (05 with tile 632) | 207 | `624ce59e` |
| 05d-readback (diagnostic) | 229 | `5f02f5fa` |
| 06-dispatch | 444 | `d95c57c1` |

Notes on these hashes:
- Public references use eight-character SHA-256 prefixes. Full digests
  are retained with the private lab receipts for exact artifact matching.
- Pass-1 hashes, before the fixes:

  | Script | bytes | SHA-256 prefix (8 hex, pass 1) |
  |---|---|---|
  | 01 | 206 | `b3a71bec` |
  | 02 | 217 | `3b4b5aa5` |
  | 04 | 201 | `b5919cdc` |
  | 06 | 459 | `f8b70ffc` |

- 03 and 05 did not change between passes.
- The final 02 bytes are byte-identical to what GServer-v2's own GS1→GS2
  translator (#73) produces for the same text.

The variant scripts (04b/04c/04d, 05b–05e) are in `variants/` beside this file.

## Results (final compiler)

✓ means the README effect was seen on screen. Screenshots and raw proxy
tools are retained privately; only behavior summaries and payload hashes
are published here.

| Script | 6.03r7 | 5.24r4 | 4.20r8 | 2.22 GS1 text (control) |
|---|---|---|---|---|
| 01 chat + timer | ✓ counter `n=…` every second; "gs1 ok" not seen (see notes) | ✓ counter | ✓ counter drawn, box glyphs (*) | ✓ counter `n=126` |
| 02 touch + image | ✓ image swaps to bush, "touched 1", "touched 2" | ✓ same | ✓ image swaps; chat drawn, box glyphs (*) | not run (wrap-up) |
| 03 showimg + client. var | ✓ "seen 1 x=30" in yellow above the player | ✓ | ✓ | not run |
| 04 players[0] follow | ✓ NPC sits 2 tiles right of the player and follows | ✓ | ✓ | not run |
| 05 tiles (tile 2047) | runs, nothing visible: 2047 is grass in this pics1 (see 05) | same | same | not run |
| 05 with tile 632 (black) | ✓ 2x2 black block at x+3 on touch | ✓ | ✓ | not run |
| 06 dispatch | ✓ "other runs: N" ticks; touch shows "touch 1 on gs1lab.nw", "touch 2 …" | ✓ | ✓ two text lines, box glyphs (*) | not run |

No client crashed, and no script error appeared in any final-pass console log.

(*) Under Wine, 4.20r8 draws `message` and chat text with an empty-box glyph for
every character. Box counts and spacing match the expected strings, and the
showimg text in 03 (`@Arial`) renders normally. This is a client font problem
under Wine, not a compiler one.

## What failed in pass 1, and why

| Script(s) | Seen on real 6.03r7 | Cause | Fix |
|---|---|---|---|
| 01, 02 (touch branch) and 06 (touch branch) | `Script: Function setplayerprop not found in function onCreated` / `onPlayerTouchsMe` / `__gs1run`. Execution continued: the 01 timer and the 02 `setimg` still ran. | `setplayerprop #c,…` had been lowered to a call to a function that GS2 doesn't have. | bf6a2d8: `player.chat = …`. Rerun: errors gone, chat text shows. |
| 04 | NPC never moved; no error. | Diagnostic 04d printed `pc= p0x=30 x=30`: bare `playerscount` reads as empty (0) in GS2, so the `if` body never ran. `players[0].x` and the NPC's `x` were already correct. 04b (no count guard) followed the player. | e08d4d3: `players.size()`. Rerun: follows on all three clients. |
| 05 | Nothing visible on touch; no error. | Not a compiler fault. 05d read the write back: `x=30 y=27 t=2047 u=48`, so `tiles[x+3,y]` was written and reads back 2047, and `updateboard` raised no error. Tile 2047 (0x7FF) is plain grass in this lab's 2048x512 pics1. With tile 632 (the only all-black tile there), the 2x2 block appears on every client. | The kit's "black" comment depends on the tileset. Either keep 2047 and drop "black", or use a tile that is dark in the shipped pics1. |

## Notes

- **The "gs1 ok" chat from 01's `if (created)` was not visible at 25–30 s on
  any client.** The same happened with GServer-v2's own translation of 01. The
  chat set on touch (02, 06) does show. The likely reason is that a chat set
  during NPC creation is gone, or set before the player is in the level, by the
  time of the first shot. No early frame was taken.
- **Comparison with GServer-v2's #73 translator.** I put the kit text in the
  level with no swap. It compiled 01–03 and drew the same effects. It refused
  04, 05 and 06:
  - 04: "a field after an index is not lowered";
  - 05: "a two-part index is not lowered";
  - 06: "a top-level statement after an event test" / "event test with else".

  So `gs1c` covers cases the server translator does not.
- **Lab issues (not client findings):**
  - **6.03r7 tiles.** 6.03r7 asks for pics1 only on the login server and drew
    black tiles until pics1 was in the client cache. The fix was to put pics1
    in the cache.
  - **Fonts.** Without fonts served, `message` text didn't draw on 6.03r7. The
    first 03/06 runs predate the fonts.
  - **Shared VM.** Another lab shared the VM, so this lab ended up in its own
    namespace, client copies and Wine prefix.

## Not done (handed over)

- **2.22 GS1-text control:** only 01 was run (✓). 02–06 are still to do.
- **01's creation-time chat:** check it with an early frame, about 12–16 s
  after start.
