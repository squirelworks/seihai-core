# Changelog

One line per release. The full notes for each version from 0.0.14 on are
in [`release-notes/`](release-notes/) and on the GitHub release; the
versioning rule is in [VERSIONING.md](VERSIONING.md). PRs add a line under
Unreleased; the release PR moves them under the new version.

## Unreleased

- GS2 compiler: output now follows the official server-side compiler. It adds the per-function variable cache (opcodes 45, 46 and 47), `params` as `OP_PARAMS`, `NULL` as `OP_TYPE_NULL`, the smallest operand for every jump, jump threading, no closing `OP_RET`, and the function table in the engine's hash-list order. It also folds number constants, joins text literals, follows the official `&&`/`||` jump chains, writes numbers as the official number text, sets the segment 1 event mask, and calls `pow(a, b)` by name (the VM computes it). Legacy GS2 also reads the old GS1 names (`playerx` as `player.x`, `imgwidth(a.png)` as `getimgwidth("a.png")`, `sarraylen`), folds `Enum.MEMBER` and the `VK_` key codes, compiles `s.lower()` as `lowercase(s)`, `float(x)`, `_()` and `obj.link()`, leaves out unused built-in values and bare names, keeps every copy of an `obj.f` function, and takes labeled loops (`outer: for …`, `break outer;`), `xor`, `~`, `^=`, `<<=`, `>>=` and `new T() { … }` without a name. Functions now end with `return 0` unless the last statement is a `return`. New `compile::Options` and `compile_with` take an optional `lambda_id`: the official server names lambdas `function_<id>_<n>`, where `id` comes from its own running count of compiles (two captures of the same source got 1038 and 1044) and `n` counts up within the script, so a server passes its own counter; with none, Seihai uses the line as before. A `for` test with `&&`/`||` is now a value then `OP_IF`, a post-increment value is converted again, and a member name in brackets that starts with text (`obj.(@n - 1)`) is text as a whole. Legacy GS2 reads `=` in tests the official way: in `if`/`while` it compares (`(a=1)==2`; `a=1 && b=2` is `a == ((1 && b) = 2)`), in `for` it stores and tests the stored value, a conditional's first branch may store, and `c ? b=2 : d=3` stores 3 into the whole conditional. Of 151 official captures, legacy mode now matches all 151 byte for byte when given each capture's lambda number (146 without it; the other five differ only in that number) and strict mode 144, rejecting the other seven as legacy quirks (before: 0). The rules are in Graal_analysis `npc-library/gs2-official-emit-spec-2026-11-26.md`. `Script::parse` now accepts `OP_IN_RANGE` with its integer operand, and `Script::write` uses the smallest marker for jump targets. The VM resolves a cached `player`, `level`, `temp` or `this` to its root object. Breaking for callers that compared bytes with gs2-parser output.
- `three_d`: new field `ground_cover` (`Field::GroundCover`, 41 fields): whether the build grows ground cover from a `DynamicShapeReplicator`, `DynamicGrass` or `DynamicGrassReplicator` ghost. 4.20r8 takes the ghost but grows nothing, under GDI and Mesa (lab, Graal_analysis PR #951); 5.12 grows it (lab).
- `three_d`: new field `terrain_textures` (`Field::TerrainTextures`, 40 fields): whether the build fetches terrain textures and `heights.info` for streamed terrain. 4.20r8 takes streamed TerrainBlock geometry again (`terrain_chunks` true, lab T3D-117 under Mesa) but draws it untextured and never fetches `heights.info`, so its `terrain_textures` is false (lab). The Linux 4.20 row offers streamed terrain again (lab).
- `three_d`: new field `drops_unknown_ghost` (`Field::DropsUnknownGhost`, 39 fields): 4.04r2 crashes on an update to a ghost that was never created, 5.12 and 4.20r8 log and drop it (fuzz tier 1, lab). 3.153's `drawn` is now `SkyOnly`: it reaches its sky view once its `.gif` sky requests are answered (lab).
- New `protocol::version_pick`: reads a game server's `Allowed:` list from its version refusal (`AllowedList::from_disconnect`) against seihai-gs2lib's table order (`GS2LIB`), ranks the profiles it takes (a 3D build for a 3D server and a 2D one otherwise, then sure admits, binary-backed, default-listed, newest) and says whether to log in at once or ask the player (`pick`: `Auto`, `Ask` when the server takes two eras with a gap, `Nothing`). `first_try` gives the version to try on an unknown server. The design for Seihai's consumer mode is `documentation/auto-version.md`.
- Every version profile now has `three_d`, the build's Torque 3D side (`protocol::three_d`): Torque protocol, game string, class table and framing, game layer, mission phases and control objects, `.ter` vs streamed terrain with the `levels3d/heights` cache and the 1023-byte chunk limit, terrain texture format, TorqueScript, Terraformer and TerrainEditor, DTS version range (`dts_min` to `dts_max`), DIF/3DS/MD3 versions, texture naming (5.12 strips `.png`), the `.dso`/`.cs`/`.ter` download filter and the BigData chunk limit. 3.04 to 5.12 (Windows and Mac) have one; every other build has `None`. Each row lists the fields that are assumed rather than read, and helpers (`offers_terrain_chunks`, `runs_torque_script`, `requests_download`, `reads_dts`, `runs_server_scripts`, `texture_request_name`, `accepts_control`) let Ato and the gserver gate behaviour per build. Mirrored from Graal_analysis `version-profiles/three-d.toml` into `three_d_toml.tsv` by seihai `tools/gen_three_d_fixture.py`; a unit test checks every row against it. The "3d not modelled" unknowns on those steps now name what is still assumed. Breaking: a new field in `VersionProfile` and `chain::Delta`.
- `three_d::THREE_D_420_LINUX` and `three_d::by_token`: Linux 4.20r6 and 4.20r8 (`G3D22126`) link as protocol 24 and have no version profile, so their 3D row is reached by token (lab 2026-11-25). 4.04r2 and the TGE 3.x rows now have no DTS floor (`dts_min` 0): versions below 19 go to an old-format reader, not a reject.
- `three_d`: Windows 4.20r8 links as protocol 24 (lab) and crashes once a TerrainBlock is added, so its row no longer offers streamed terrain; the Linux 4.20 row follows it (assumed).
- `three_d`: new field `greyscale_rgba32_only` (`Field::GreyscaleRgba32Only`): the Terraformer's `loadGreyscale` is only safe with a 32-bit RGBA PNG; 24-bit RGB and 8-bit grey crash 5.12 (lab). 5.12's terrain texture and Terraformer rows are now lab-checked too. `Fields` holds 32 fields.
- `three_d`: six new fields from the 2026-11-25 labs (`requires_sky`, `sky_gif`, `drawn` with `Drawn`, `mirrors_2d_position`, `sees_2d_players`, `takes_2d_chat`) and helpers `sky_texture_request_name` and `needs_2d_position_relay`. 3.153 needs a ghosted Sky, asks for sky textures as `.gif`, never asks for `.cs`/`.dso`/`.ter` (its `download_refused` is now the script and `.ter` filter, lab) and has drawn nothing yet; 4.04r2 draws only the Sky and its 3D walk does not move its 2D player; 5.12 does not see 2D players and gets their chat. 5.00r7's TorqueScript, terrain texture and `.3ds` fields are now read from its engine DLL. Breaking: `Fields` is now 64 bits wide (38 fields) and `ThreeD` has new fields.

- GANI frame drawing widens origin and placement translations before clipping, avoiding coordinate overflow while retaining visible sprite placement, transparency and replacement behavior. Public APIs and dependencies are unchanged.

- Own-player position updates now apply precise X2 and Y2 coordinates when the receiver profile accepts them, retaining the other axis when an update supplies only one.

- An authored GS1 indexed-array loop control now executes through the real GS1 front end and Machine with production EffectHost, checking every square element, visit count, sum and final counter with no host effects. Existing GS1 engine and array-literal coverage remain separate; this adds finite index-loop own-VM acceptance, not colon-foreach or native behavior.

- A focused authored GS1 `with(player)` execution control now uses the real GS1 front end and production EffectHost to assert an explicitly seeded NPC variable is preserved, player coordinates are updated, and NPC context and exact host-effect order are restored after the block. Existing selective-with refusal boundaries are unchanged; this is own-VM acceptance, not native behavior or general-with completeness.

- Clipped rectangle outlines widen coordinates before forming their edges, avoiding overflow for large positive extents while preserving the original border and fill clipping. Pure public-API regressions and an independent visible-pixel geometry matrix cover extreme origins/extents, thin/empty rectangles and empty buffers. Public APIs and dependencies are unchanged; this assumes a valid allocated RGBA buffer and does not claim stock-client behavior or arbitrary allocation safety.

- Own-VM value acceptance now covers strict rejection and legacy warning/value behavior for text-valued enums, including numeric continuation and bare/qualified names, plus computed calls to the script's own methods with named-call, argument-binding and evaluation-order controls. These authored programs do not establish external/native method semantics or official evaluation order; compiler and VM implementations are unchanged.

- Text rendering clips widened shadow, glyph/bold and decoration coordinates before integer narrowing or rule iteration, avoiding overflow at extreme origins while preserving ordinary raster rounding and visible glyph bearings. Synthetic-font regressions reproduce the former X/Y shadow panics, retain visible controls, and cover selected style/alignment extremes, clipped-image translation and a bounded large-glyph path. This does not change public APIs or claim arbitrary font/rasterizer resource safety.

- Own-VM acceptance tests execute authored numeric enums (implicit values, explicit continuation and bare/qualified references) and computed dynamic-member reads/writes in strict and legacy modes. They assert resulting values and unchanged neighboring state, without corpus fixtures, official bytecode equivalence or native-client acceptance; compiler and VM implementations are unchanged.

- The generic 2.1 row's `download_maps` (the `maps` download folder) is no longer an unknown inferred from 2.17.1: Graal_analysis read it statically in every Graal 2002 build from ver9 (2.1.0.0) on, the builds that send `GNW31101`; ver7 and ver8, which send 2.05's token, lack it (Graal_analysis #719). The row's value is unchanged, and the row as a whole stays inferred because it is not pinned to one Graal 2002 build.
- GS2 `mousex` and `mousey` read the last click, in level tiles, from the new `EffectHost::mouse`. The real 6.03r7 gives `onMouseDown` only the button as `params[0]` (`"left"`, `"right"`) and keeps the position in those globals (squirelworks/seihai#497 lab, 2026-11-20; #136). Breaking: a new field in `EffectHost`.
- The 5.12, 5.24r4 and 5.31r4 login line is now read from captures. On 64-bit Windows 10, each build sent `win,"",<md5 of the MAC>,<md5 of the C: serial>,"6.2 9200 "` to the lobby and again after the warp (lab, 2026-11-20; #121). The product-id field is empty on any stock 64-bit Windows, so the `""` Seihai sends matches it. Documented in `protocol::lobby` and in the three steps' sources; Seihai's behaviour is unchanged.
- GS1 scripts lowered to GS2 get a wrapper only for the events they test. A real 2.22 runs a GS1 body only for the events the script names: with only `created` and `timeout` tested, touching or pushing the NPC never ran the code outside the tests, while the lowered bytecode on 6.03r7, 5.24r4 and 4.20r8 ran it for every event in the era's list (seihai-core#130 lab, 2026-11-20). This reverses the every-event wrappers from 0.2.17. Results in `documentation/lab/gs1-on-gs2-clients/lab-results.md`.
- NewWorld beta 1 and beta 2 stay one token served as beta 1: a 2026-11-20 lab run found their login streams byte-identical from the same client folder, so nothing tells them apart (#128). Documented in `documentation/connection-capabilities.md`, with a test.
- The 5.x and 6.x script cache (squirelworks/seihai#497): a `PLO_NPCWEAPONSCRIPT` (140) with the cache flag keeps `GRESCR01` and the body DES-encrypted under the header's key, and `PLO_LOADSCRIPT` (197) loads that copy when the CRC-32 of its stored body matches, else owes a `PLI_UPDATESCRIPT` (158) in `SessionState::script_requests`, which the client sends. New module `scriptcache` (`ScriptCache`, `Descriptor`, `seal`, `open`, `csv_fields`); the client keeps the files in the server's cache folder when it has one. A 140 header is read as CSV, so a quoted name with a comma is kept whole. Breaking: new `SessionState` fields.
- GS1 `playeringuild(NAME)` (and the bare `playeringuild`, for the empty name) is true when the guild name from `guildname.code` equals NAME exactly, case included. No file, or one whose halves differ, matches no name (lab, 1.1 and 1.1.9 offline, 2026-11-20; #63). The host fills the new `PlayerView::guild` with the new `graaldes::guild_name`. Breaking: a new field in `PlayerView`.

## 0.7.x

- **0.7.2**: `files.modtime` is yes for 2.05 and both NewWorld betas: real 2.05 rev 2 and both betas drop a served file sent without the GINT5 mod time (Wine labs against seihai-gserver, 2026-11-05 and 2026-11-20). The `PLO_FILE` layout row starts at 2.05, and the NewWorld COLORS note names bytes 6 to 8 (pullover, pants, border).
- **0.7.1**: The NewWorld betas send COLORS as 8 bytes, not 7 (`Props::colors_sent` is 8; lab against seihai-gserver, 2026-11-19). With 7, every login prop after COLORS was misread.
- **0.7.0**: The NewWorld betas (`G2N15041`) have profiles: `ClientVersion::NewWorldB1` and `NewWorldB2`, side builds of 2.05 and 2.1 (new `Delta::side`), with COLORS sent as 7 bytes and read as 8 (`Props::colors_sent`, `colors_read`, `parse_player_props_sized`). Every catalogued version token has a profile, a stand-in or a reason (`STAND_INS`, `NOT_SERVED`, `serve_as`). GS2 texts that are not numbers read as -1, as on the real clients (#120). A secure list fetch for 4.x to 6.x borrows 2.22's login. Breaking: new `ClientVersion` variants, new `Props` and `Delta` fields, new `TokenKind` and `TokenGrade` variants, and the NewWorld rows leave `offline_builds`. Released early for seihai-gserver; the rest of #108 moves to 0.8.0 (#123).

## 0.6.x

- **0.6.3**: GS1 `keypressed` event from 2.17.1 on (`ScriptWorld::key_pressed`, `SessionScripts::key_pressed`), lowered to GS2 as `onKeyPressed`. Lab fuzz-case examples for GS1 and GS2. Releases come only from a release PR, checked by `version-guard`, and publish automatically (VERSIONING.md).
- **0.6.2**: GS1 `putexplosion`, `putexplosion2` and `puthorse` lower to GS2 calls, as real 4.20r8 and 5.12 run them (lab, #104); the PLO 48 layout is lab-confirmed (#106). GS1 `#s(name)` on a plain string reads `player.<name>`. Two new fuzz targets (`session_apply`, `gs2_exec`) and a fix for an exponential GS1 parse the fuzzer found (#80).
- **0.6.1**: The GS2 VM runs `new` objects (`new TStaticVar()`, `new GuiWindowCtrl("name") { … }` blocks), `with` on them and on variable paths, `makevar`, and `objecttype()` / `destroy()`; `Machine::objects` lists them. `EffectHost` turns GS2 `triggeraction` and `triggerserver` into `Effect::TriggerAction`. `PLO_TRIGGERACTION` (48) is decoded into `SessionState::triggers` and `Live::triggers` (#106). `delivery` compiles a mixed GS1/GS2 client part that strict GS1 refuses. `cargo doc` builds without warnings, checked in CI.
- **0.6.0**: `client::Login` gains `secure`: with it, every game server of a client session (the 5.x/6.x Login lobby and its warp included) uses the Seihai secure login instead of the build's clear login (squirelworks/seihai#564). Breaking: a new field in a struct built by literal. The secure list login's error names the shapes it takes.

## 0.5.x

- **0.5.1**: GS1 `putbomb` and `shoot` lower to GS2 calls of the same name, as real 4.x–6.x GS2 weapon scripts make them (#93). The 1.1.9 relay client shows a `reg.code` registration key as type `8` and flags a peer with the same key (#63). GS2 event handlers read their arguments as `params`. The release workflow builds and tests on macOS before it publishes (#78).
- **0.5.0**: Mixed GS1/GS2 level NPCs run both parts: unmarked scripts lift top-level `if (created)`-style tests, and both front ends fold them into the script's own handler (new `Quirk::Gs1EventTest`). `Level::tile_id_on_layer` treats 4095 as empty on upper layers, and `seihai-draw` no longer paints it.

## 0.4.x

- **0.4.4**: `Script::write` omits the trailing `0x0A` after the four GS2 segments, matching official bodies; the reader still accepts an older leftover newline.
- **0.4.3**: Explicit-endpoint `relay::Client` for the 1.1.9 relay dialect (separate game/level rooms, pairing, chat and peer updates).
- **0.4.2**: Explicit Windows 2.1 build metadata selects verified receive bindings and the public 2001 flag/map reader, without changing generic token profiles.
- **0.4.1**: Correct the native client 2.05 rev 2 GS1 command lookup to all 161 ordered IDs; public signatures and interpreter behavior are unchanged.
- **0.4.0**: Breaking `Era::V140` addition enables explicit selection of the observed 1.40 GS1 vocabulary; existing online profiles and 1.41 timing gates remain unchanged.

## 0.3.x

- **0.3.1**: Version-chain and audit provenance distinguish binary-read Graal 2002 identities from the still-inferred generic 2.1 profile; profile values and token grades are unchanged.
- **0.3.0**: Breaking GANI placement-reference API preserves observed PARAM1 distinctly from numeric sprite zero; unresolved symbolic drawing is skipped.

## 0.2.x

- **0.2.18**: Explicit Graal 2002 build descriptors and static GS1 word membership distinguish six builds sharing login tokens; generic eras and online profiles stay unchanged.
- **0.2.17**: A dispatched GS1 script runs its outside code for every event in that era's closed list, not only the events the script names.
- **0.2.16**: GS1 suffix commands use the whole-script dispatcher to retain per-event source order.
- **0.2.15**: GS1 unconditional prefixes run once per named event through the existing dispatcher, preserving eventless load and direct GS2 calls.
- **0.2.14**: GS1 function calls after event tests retain source order through the existing whole-script dispatcher; direct GS2 calls stay unchanged.
- **0.2.13**: GS1 `setplayerprop` compiles to `player.*` and `playerscount` to `players.size()`, from the first 6.03r7 run of Seihai's bytecode.
- **0.2.12**: `assets::chr`: classic character files (`.chr`, `.chi`, `CHR:1.00name=head,sex,five colours`) read and written, as Graal Character Designer and GChar wrote them; `Character::looks` for drawing.
- **0.2.11**: seihai-draw `draw_text` and `measure_text`: text in a caller's TrueType/OpenType font with size, colour, bold, italic, underline, strikeout, shadow, alignment and wrapping (`ab_glyph`), for GraalShop TEXT actors (servertools #78).
- **0.2.10**: `graaldes`: the classic clients' DES variant (bit-reversed key bytes), the 48-character string form of RC passwords and list hosts, and the 1999 Guildmaker `.code` files (`guildname.code`, `reg.code`) read and written (#63).
- **0.2.9**: `assets::gani_movie` reads and writes GraalShop's `GANI0002` movie mode (`FRAMES`, `MOVIE`, `ACTOR CHAR|SPRITE|SOUND|TEXT`, `FRAME n key=value,…`) and answers an actor's state at a frame, with `dx` / `dy` moving between keyframes (servertools #78).
- **0.2.8**: the iPhone `KNOWN_TOKENS` labels list five more builds read from IPAs (Cocoto Kart Online 1.0 and 1.3, Cocoto Magic Circus 1.0, Ace Tennis 1.3); no new token.
- **0.2.7**: `G3D18010` becomes a binary-read token (the 2010 iPhone engine: Cocoto Kart HD, Ace Tennis HD, Bomber Online 1.40, Dino Smash 1.4), and `G3D04048` is relabelled as the 2008 iPhone engine shared by 14 builds.
- **0.2.6**: GS1 `set`, `unset` and `setstring` on a plain flag lower to an assignment to `client.gs1_<flag>`, which real 5.24r4 and 4.20r8 clients send and seihai-gserver keeps as `<flag>` (#30).
- **0.2.5**: `read_gif_frames` reads every frame of an animated GIF as palette indices on the logical screen (disposal, delays, the `NETSCAPE2.0` loop count), for gif2mng in seihai-servertools.
- **0.2.4**: `KNOWN_TOKENS` gains the iPhone login tokens: Belote 1.2/1.50 `G3D04048` and Classic+ 1.13 `G3D0208A`, read from the IPAs, and three gs2lib-only iPhone tokens (83 rows).
- **0.2.3**: Binary level writer: `encode_binary` writes `GR-V1.03` (`.graal`) and `Z3-V1.04` (`.zelda`) and reports what each format cannot hold; `encode_board` and `encode_sign_text`; `write_mng` and `write_indexed_png` for gif2mng-style animations.
- **0.2.2**: Whole-script GS1 runs from one dispatcher (1,204 of the 1,765 level scripts we hold now compile, from 957); GS1 `board[i]` lowers to `tiles[x, y]`; `#I` and `#L`; the `gs1c` example and a lab kit for original clients; the profile audit, connection capabilities and an id-108 check from the roadmap (#7).
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
