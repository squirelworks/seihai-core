# Real 2.22: a follower's baddy hit, laying items, and a link into a level the client lacks

Lab run, 2026-11-20 (#121). Two unchanged Windows 2.22 clients (`GNW03014`) under
Wine with a virtual desktop, in a Debian VM, against a loopback GServer-v2 lab world
behind a logging proxy. Nothing left the VM's private network namespace. The level
is a plain 64 x 64 grass level with one gray soldier (`BADDY x y 0`) and a west-edge
link to a second level that only the server holds. Both accounts had 20 hearts, so
the baddy could not kill the leader (an earlier attempt lost the leader role to a
death). Captures and screenshots stay in the private lab folder; this note keeps the
derived bytes only.

Values on the wire are written as hex bytes, without the `+32` removed, as in the
capture. Positions are in tiles.

## 1. A follower's sword hit on a baddy (seihai#274)

Setup: the first client in the level is the leader (it sends `PLI_BADDYPROPS`, 15).
To freeze the baddy at a known place, the leader's process was paused (SIGSTOP)
for the hit and resumed afterwards. The follower was put 3 tiles west of the baddy,
facing east, and swung once.

| Step | Who | Bytes | Read as |
|---|---|---|---|
| Hit | follower, `PLI_BADDYHURT` (16) | `21 38 60 21` | id 1, dx `0x38` = 24/16 - 4 = **-2.5**, dy `0x60` = **0**, power 1 |
| Same frame | follower, `PLI_BADDYPROPS` (15) | `21 24 21 20 25 23` | id 1, power 1 with an empty image, mode 3 (hurt) |
| Relay | server to leader, `PLO_BADDYHURT` (27) | `21 38 60 21` | byte for byte the follower's message |

The follower's centre (x + 1.5, y + 2) was (28.5, 17) and the baddy's centre
(x + 1, y + 1) was (31, 17): dx -2.5, dy 0, which is exactly what the bytes say.
This is the same layout as the earlier `lan-222hurt` capture (`21 28 60 21`, dx -3.5),
so the byte order is: id, dx, dy, power, with dx and dy as `(d + 4) * 16`.

**Knockback, on the leader** after it was resumed (its next thinks, 100 ms apart):

| Think | Props sent (`PLI_BADDYPROPS`) | x |
|---|---|---|
| 1 | x, power 1 with the empty image, mode 3, counter 7 | 30 to **32** |
| 2 to 6 | x, counter 6 down to 2 | 34, 36, 38, 40, **42** |
| 7 | counter 1 only | 42 (no move) |
| 8 | mode 2 (hunt), counter 12, facing | 42 |

So a hurt baddy moves **2.0 tiles a think, away from the hitter, for 6 thinks**
(counter 7 down to 2), 12 tiles in all on open ground, then hunts. This matches
`documentation/baddies.md` in seihai and the leader test in
`offline/baddies.rs`.

A second run hit the baddy on the leader's own client (the leader was hitting
from above, next to the west edge): its first think moved the baddy 2 tiles down
and set mode 3, counter 7. That run did not show the rest of the push clearly, so
it is not counted here.

## 2. Laying items (seihai#278)

The follower was given 10 bombs and 20 arrows (no gralats), stood at (31, 30) or
(30.5, 30), and used S held + A, then A. The keys had to be held about 0.2 s; a
zero-length key tap was not seen by the client.

| Keys | Facing | `PLI_ITEMADD` (12) | Read as | Then |
|---|---|---|---|---|
| S+A, A | east, x 31 | `63 5e 23` | x 33.5, y 31, item 3 (bombs) | `PLI_PLAYERPROPS` bombs 5 |
| S+A, A, A (next) | east | `63 5e 24` | x 33.5, y 31, item 4 (darts) | arrows 15 |
| S+A, A | west, x 30.5 | `5a 5e 23` (and `59 5e 23` in a second run) | x 29 (28.5), y 31, item 3 | bombs 0 |

- The spot is `(x + 0.5 + dx, y + 1 + dy)` with east `(2, 0)` and west `(-2, 0)`,
  as in seihai's `documentation/item-laying.md`.
- The first item offered is bombs (both bombs and darts were at 5 or more; bombs
  come first in the lay order), and the second A with S held moves on to darts.
- The S+A pick sends only the `sword` and `idle` gani props and `item2.wav`; the
  lay sends `item.wav`.
- About **8.2 s** after each lay the client sends `PLI_ITEMDEL` (13) for the spot
  (`63 5e`, `5a 5e`): 8.19, 8.16 and 8.19 s.

This is the 2.05 rev 2 capture's behaviour, now also seen on 2.22, and the first
west-facing lay.

## 3. A link into a level the client does not have (seihai#224)

The follower walked west into a link to `m121miss.nw`, a level only the server had.

**Not cached** (the client's `weblevels\<host>\` folder had no copy; two runs):

1. One client frame: `PLI_LEVELWARP` (0) `5c 5c` + name (arrival 30, 30), then
   `PLI_TAMPERCHECK` (39).
2. The client stays where it is and keeps reporting its old position (x 0.5).
3. The server answers 50 to 140 ms later: `PLO_LEVELNAME` (6), `PLO_RAWDATA` (100),
   `PLO_BOARDPACKET` (101), `PLO_LEVELMODTIME` (39), `PLO_LEVELNAME` again,
   `PLO_ISLEADER` (10), 174, `PLO_SETACTIVELEVEL` (156). No `PLO_PLAYERWARP`.
4. Only then does the client send its new position (30, 30).

There is **no file request** (`PLI_WANTFILE` or `PLI_UPDATEFILE`) at any point:
the level comes as board data, unasked.

**Cached** (the first run had left `weblevels\127.0.0.1\m121miss.nw-14808.code`):

1. One client frame: `PLI_LEVELWARPMOD` (30) with the cached mod time and the
   same arrival and name, `PLI_TAMPERCHECK` (39), **and the new position** (30, 30).
2. The server, whose copy was newer, still sent the full board in the same order.

So the warp is the first thing on the wire either way; what changes is whether the
client switches at once (cached) or waits for the server's board (not cached). 2.22
sends no `PLI_ENTERLEVEL` (46) in either case. Seihai's `link_request_frame` and
`LinkArrival` already follow this (they were taken from a real 2.31); 2.22 agrees.
