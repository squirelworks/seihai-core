# Changelog

One line per release. The full notes for each version from 0.0.14 on are
in [`release-notes/`](release-notes/) and on the GitHub release; the
versioning rule is in the README.

## 0.2.x

- **0.2.2**: `KNOWN_TOKENS` gains the iPhone login tokens: Belote 1.2/1.50 `G3D04048` and Classic+ 1.13 `G3D0208A`, read from the IPAs, and three gs2lib-only iPhone tokens (83 rows).
- **0.2.1**: GS1 lowering of `players[i].x` / `.y` / `.dir` and `npcs[i]` reads, `tiles[x, y]`, lists inside literals and a list into an element, from four new compiler fixtures; GS2 no longer converts an array element before a member read.
- **0.2.0**: GS2 weapon and class scripts from `PLO_NPCWEAPONSCRIPT` (140), NPC scripts sent after their level or for a gmap neighbour handed to the game, `player.*` reads and writes in GS2 (`player.chat` included), and `.mid` music from a small built-in synthesizer. Breaking: new fields in `OwnWeapons` and `EffectHost`, and `.mid` now plays.

## 0.1.x

- **0.1.9**: The built-in player is a squirrel (head, body and bushy tail), with an acorn for the money icon and on the shield. Graal files in the user's asset folder still replace it file by file.
- **0.1.8**: 5.x builds send the nickname once the first level is in, so 4.x players no longer see them as "default".
- **0.1.7**: The client session hands the game the level's baddies, the leader flag and other players' carried objects; GS2 `play` and its kin make sound; `.mp3` music plays.
- **0.1.6**: `KNOWN_TOKENS`, every login token in the Graal_analysis catalogue (78, with or without a profile), and its generated `documentation/version-tokens.tsv`.
- **0.1.5**: A file the server names with no extension (the Graal3D Sky's textures) is taken instead of dropped, so Ato's 3D login reaches step 6.
- **0.1.4**: The shared client session hands the game the server's floor items (`PLO_ITEMADD` / `PLO_ITEMDEL`), for other players' drops in Ato and Issho.
- **0.1.3**: The shared client session hands the game the hits on the player (other players' swings and ids 40 and 46), for being hit in Ato and Issho.
- **0.1.2**: The shared client session hands the view the player's weapons (built-in and the server's), for the item list and weapon key of Ato and Issho, and other players' bombs, arrows and throws.
- **0.1.1**: GS1 writes `selectedweapon`, reads a bare-number loop condition as false, and rounds `timeout` down to whole 50 ms steps on 1.41 to 2.31, as real clients did in the lab.
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
