# Update cadence of the 3.x, 4.x and 6.x clients

Lab, 2026-11-13 (Graal_analysis E#301). The Linux 3.04, 4.04r2,
4.20r6 and 6.03r7 clients ran natively under Xvfb, and the Windows 4.04r2
under Wine 10. Timing calls were logged with an `LD_PRELOAD` shim, walking
was read at a decrypting proxy, and a GS1 weapon counted its own `timeout`
fires.

**Answer: every build steps the game in fixed 50 ms steps, as 1.x to 2.31
do.** One step is one render (about 20 fps in game), a walk step is 8 px
(half a tile), so walking is 10 tiles a second. Between steps every client
polls with a 1 ms `nanosleep` (the Windows build `Sleep(1)`); none sleeps
until the next step.

## The loop

| Build | Loop | Step rule | Behind schedule |
|---|---|---|---|
| 3.04 | Torque `TimeManager` | step when `now - last >= 50`, then `last = now` | no catch-up: a slow machine slows the game |
| 4.04r2, 4.20r6 | Torque `TimeManager`, interval set to 50 at start-up | as 3.04 | as 3.04 |
| 6.03r7 | its own fixed-step loop | step when `now >= last + 50`, then `last += 50` | one catch-up step at most; more than two steps behind, it drops them (`last = now`) |

3.04's start-up interval is 5 ms, which is why its login screen renders at
about 197 fps; the game control sets 50 ms when it wakes, so in game it is
20 fps like the others. 6.03r7's update also counts steps and runs a
once-a-second dispatch every 20th step.

## Measurements

Frame interval in ms (mean / median / p5 / p95):

| Build | Login | In game, walking |
|---|---|---|
| 3.04 | 5.07 / 4.79 / 4.57 / 5.79 (197 fps) | 50.06 / 49.89 / 49.19 / 50.90 (offline tutorial 50.08 / 50.03) |
| 4.04r2 (Linux) | 50.07 / 50.01 / 49.11 / 51.15 | 50.07 / 50.10 / 49.10 / 51.24 |
| 4.20r6 | 50.10 / 49.94 / 49.08 / 51.09 | 50.06 / 50.04 / 49.06 / 51.08 |
| 6.03r7 | 50.00 / 49.83 / 49.42 / 50.81 | 50.00 / 49.98 / 49.15 / 50.98 |
| 4.04r2 (Wine) | about 15 fps | 19.80 to 19.87 fps (Wine `+fps`) |

Walking, client to server `PLAYERPROPS`:

| Build | Props | Interval | Per message | Speed |
|---|---|---|---|---|
| 3.04 | X | 100.0 ms (99 to 101) | 1 tile (two steps) | 9.92 tiles/s |
| 4.04r2 (Linux) | X2 | 50.0 ms (49 to 51) | 8 px | 9.98 tiles/s |
| 4.20r6 | X2 | 50.0 ms (50 to 51) | 8 px | 9.98 tiles/s |
| 6.03r7 | X2 | 50.0 ms (49 to 51) | 8 px | 10.12 tiles/s |
| 4.04r2 (Wine) | X2 | 50.0 ms (49 to 51) | 8 px | 9.93 tiles/s |

Throttled (the process stopped and continued so it runs 4 ms of every
100 ms): the step stays 8 px and speed follows the step rate. 3.04 online
10.3 fps, a message every 200 ms, 4.82 tiles/s; 4.04r2 9.7 fps, 4.64
tiles/s; 4.20r6 10.3 fps, 4.99 tiles/s; 4.04r2 under Wine about 7 fps,
3.78 tiles/s. 6.03r7 ran 17.1 fps, two steps about 1.6 ms apart in each
100 ms window (its one catch-up step), 8.24 tiles/s; with 200 ms windows one
step per window, 2.68 tiles/s (the rest dropped).

GS1 `timeout = 0.05` (weapon, counter in chat), fires per second:
3.04 19.97, 4.04r2 19.97, 4.20r6 19.97, 4.04r2 under Wine 19.84; throttled
9.72, 9.98, 9.98 and 6.71.

A GS1 `timeout` counts whole 50 ms steps, **rounded down**, and a timeout of
no whole step never fires. Swept on 4.04r2 (Linux), with each value checked
in the script the client received: `0.01` and `0.049` never fire; `0.05`
19.97/s and `0.07` 19.95/s (every 50 ms); `0.1` 9.99/s and `0.12` 9.98/s
(every 100 ms); `0.15` 6.66/s (every 150 ms). 3.04 never fired `0.01`, and
4.20r6 never fired `0.01` or `0.025`.

`walk.gani` has 8 frames and no `WAIT`: one frame per step, a 0.4 s cycle
(static read only).

## What Seihai does with it

- The 50 ms tick (`scripting::engine::world::TICK_MS`, `gs2::exec::TICK`,
  `assets::gani`, `pose`, `protocol::walkcadence`) is right for 3.x, 4.x
  and 6.x as well; the walk cadences in `walkcadence` (3.04 X every 100 ms,
  4.x to 6.x X2/Y2 every 50 ms, 8 px a step) match.
- Changed: the C++ engine (`Era::V304` and `Era::V3153`) counts a GS1
  `timeout` in whole steps, rounded down, and drops one below 0.05 s. The
  other engines keep the round-up to at least one tick, since they were not
  measured.
- Not modelled: the real clients' behaviour when the machine falls behind
  (3.x/4.x slow down, 6.x catches up one step and drops the rest).
  `ScriptWorld::tick` still runs every missed tick.

## Not measured

- 6.03r7's script timer: the lab server sends no GS1 to clients after 5.007, has
  no GS2 compiler, and its NPC server crashes when a 6.x
  client gets a weapon.
- The timeout sweep on 5.x and 6.x, and on the 1.x/2.x engines.
- Walk GANI frame timing at run time (read from the file only).
- Per-frame timestamps under Wine (averages from `+fps` only).
