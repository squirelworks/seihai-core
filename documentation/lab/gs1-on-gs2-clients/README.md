# Lab kit: GS1 scripts as GS2 bytecode on original clients (#6)

Six GS1 client scripts, each with an effect that shows on screen. They are
compiled by Seihai's clean-room compiler and sent to original clients as
`PLO_NPCBYTECODE` (131), and as GS1 text to a GS1 client as the control.

```sh
cargo run -p seihai-core --example gs1c -- --packet <npc id> --out <dir> documentation/lab/gs1-on-gs2-clients/*.txt
```

`gs1c` writes `NAME.gs2bc` (the bytecode) and, with `--packet`,
`NAME.plo131.bin` (the `PLO_RAWDATA` + 131 pair as GServer-v2 sends it), and
prints the disassembly and what each client build is sent.

| Script | What a working client shows |
|---|---|
| `01-chat-and-timer.txt` | the player says "gs1 ok"; a counter message once a second |
| `02-touch-and-image.txt` | touching the NPC swaps its image and the player says "touched N" |
| `03-showimg-and-vars.txt` | a text image above the player with a count and x |
| `04-players-array.txt` | the NPC follows the first player (`players[0].x`) |
| `05-tiles.txt` | touching the NPC paints a 2x2 black block beside it |
| `06-dispatch.txt` | whole-script GS1: touching says "touch N on <level>"; between touches a message counts the other runs |

Results go in `lab-results.md` in this folder: the client, the VM, what was
seen, and a screenshot name for each script.
