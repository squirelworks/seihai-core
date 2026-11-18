# Mixed GS1/GS2 NPC and multi-layer level fixtures

Self-written, synthetic scripts and levels for `tests/mixed_npcs.rs`. Each
copies the *shape* of a pattern counted in the 2019 Delteria server backup
(read statically on 2026-11-18; the note is in Graal_analysis), never its
text. The backup's own levels and scripts are private and are not in this
repository.

In that backup, 300 level NPCs in 100 levels hold a GS1 event test and a
GS2 `function onX()` in the same script; 292 have both `if (created)` and
`function onCreated()`. None is marked `//#GS2`. Three levels have `BOARD`
rows on tile layers 1 and 2.

- `stub.txt`: the editor stub (248 NPCs): `if (created) dontblock;` with no
  braces, then `function onCreated()` that moves the NPC by `.5` and calls
  `dontblock()`.
- `stub-between.txt`: the same with a top-level statement between the two
  parts.
- `lamp.txt`: a GS1 server part, then a client part that holds both a GS1
  `if (playerenters)` and a GS2 `function onCreated()` (19 NPCs).
- `shape.txt`: a GS1 server part and a GS2 client part that passes an inline
  array with a trailing comma to `setshape2` (11 NPCs).
- `timer.txt`: a client part with GS1 `if (timeout)` / `if (playertouchsme)`
  beside GS2 `function onActionX()` handlers (the rarer mixes).
- `layers.nw`: a level with full layer-0 rows, sparse layer-1 and layer-2
  patches (widths 1, 2 and 5) and one of each NPC above.
