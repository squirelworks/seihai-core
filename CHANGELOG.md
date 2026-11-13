# Changelog

One line per release. The full notes for each version from 0.0.14 on are
in [`release-notes/`](release-notes/) and on the GitHub release; the
versioning rule is in the README.

## 0.1.x

- **0.1.0**: First release under the written versioning rule. No API
  changes beyond hiding `dev`, `difftest` and `viewer` from the docs.
  Known gap: GS1 flag and built-in writes are still refused (#30, #6).

## 0.0.x (any release could break)

- **0.0.31** (#43): GS2 reads the VM of every Windows client from 4.04r2 to 6.03r7 and runs opcode 54.
- **0.0.30** (#45): The combat and respawn rules of the 1.x and 2.17+ clients follow a new read of their binaries (Graal_analysis #444).
- **0.0.29** (#40): The shared client session (`client`, used by Ato and Issho) hands the view the player's own counters and the chat, says what the player types, and sends the sword swing and what the view builds for the server.
- **0.0.28** (#42): The client session can tell a client line about each game server before it logs in, and fetch files the client line names. Ato uses both for its Graal3D side.
- **0.0.27**: The Mac PowerPC 4.04r2 and 5.12 get their own PLO sets, and every 4.x build (Windows 4.04r2 and 4.20r8, Mac 4.04r2 and 4.20r6) steps the frame cipher rounding down.
- **0.0.26**: GS1 player flags and four player built-ins are read by the GS2 names a real 5.24r4 client checked.
- **0.0.25**: Strict GS2 accepts the loose spellings the engine's own compiler accepts, and Detect reads GS1-looking scripts as GS1 again.
- **0.0.24**: Update cadence of 3.x, 4.x and 6.x measured: the 50 ms step holds.
- **0.0.23**: The GS2 executor follows the VM of the client it presents as, from 3.153 to 6.03r7.
- **0.0.22**: The Mac PowerPC 4.04r2 and 5.12 as client versions of their own.
- **0.0.21**: GS1 per client build: each version scripts with its own command table, and the 1.x–2.1 engine's condition and message-code rules.
- **0.0.20**: The spark of a sword clash, and no binary addresses in the source.
- **0.0.19**: Baddy, combat and tile rules that follow each client version, read from every binary from 1.21 to 2.31.
- **0.0.18**: The 6.00r4 client profile and its `resolveHost` script builtin.
- **0.0.17**: Sound effects decoded by their contents: A-law, mu-law, MPEG audio and Ogg Vorbis.
- **0.0.16**: Logging out when the game window closes.
- **0.0.15**: Drawing other players from what the server sent, for the flat view Ato and Issho share.
- **0.0.14**: The GS1/GS2 compiler rewrite, plus the compiler work that led up to it.
- **0.0.13** (#12): Play the sounds the server sends; send the 6.x nickname.
- **0.0.12**: Remove a level NPC's script object when the NPC goes.
- **0.0.11**: seihai-draw, a UI-less tile and gani buffer, builds in this workspace.
- **0.0.10** (#3): Take `PLO_NPCDEL2` deletes and hand them to the view.
- **0.0.9** (#1): Hand the server's NPC updates for the player's level to the view.
- **0.0.8**: First tag after the split from squirelworks/seihai.
