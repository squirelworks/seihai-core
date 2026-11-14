# Connection capabilities

Roadmap #7, "Next 3". How each original build reaches a server, kept apart
from how Seihai finds servers for it. Code:
`crates/seihai-core/src/protocol/connect.rs` (`ClientVersion::native_connection`,
`ClientVersion::discovery`, `OfflineBuild::native_connection`).

| Native connection | Builds | What it means |
|---|---|---|
| no network | Zelda Online 1 | The build has no networking. |
| relay lobby | Zelda Online 2 to 1.1, Java Graal | Knorr's GServ relay rooms; Seihai has no relay client ([offline-only-builds.md](offline-only-builds.md)). |
| direct game server | 1.21 to 1.39 | Host and port typed in; no listserver protocol in the binary. |
| listserver (classic / gen 5) | 1.41 to 2.17 / 2.22 to 3.153 | A list protocol on its own port (14923 or 14922). |
| Login server | 4.04r2 to 6.03r7 | A game connection to a server named `Login`, whose scripts show the list, then a server warp (178). |

**Borrowed discovery is not a native capability.** Seihai fetches the server
list as 2.05 for every row without its own listserver (1.21 to 1.39 and the
4.x to 6.x Login builds). That lets an early profile pick a server from a
modern list, and it does not mean the original 1.33 or 5.24r4 ever spoke a
listserver protocol. A unit test checks that a row is "native" for
discovery exactly when its own profile has a listserver.

The NewWorld betas log in with a token no profile row has and that no
server was seen to accept, so they are "direct game server (unconfirmed
token)" and stay offline-only.

## Every build

| Build | Native connection | Seihai discovery |
|---|---|---|
| Zelda Online 1 | no network | none (offline-only) |
| Zelda Online 2 | relay lobby | none (offline-only) |
| 1st Graal | relay lobby | none (offline-only) |
| 2nd Graal | relay lobby | none (offline-only) |
| 3rd Graal | relay lobby | none (offline-only) |
| 4th Graal | relay lobby | none (offline-only) |
| 4th Graal alpha | relay lobby | none (offline-only) |
| 5th Graal | relay lobby | none (offline-only) |
| 5th Graal alpha | relay lobby | none (offline-only) |
| 6th Graal (beta 5) | relay lobby | none (offline-only) |
| 7th Graal (beta 7) | relay lobby | none (offline-only) |
| 8th Graal (beta 8) | relay lobby | none (offline-only) |
| 9th Graal alpha | relay lobby | none (offline-only) |
| 9th Graal (beta 9) | relay lobby | none (offline-only) |
| 10th Graal (1.0) | relay lobby | none (offline-only) |
| 11th Graal (1.0) | relay lobby | none (offline-only) |
| 12th Graal (1.0) | relay lobby | none (offline-only) |
| 13th Graal (1.1 beta 1) | relay lobby | none (offline-only) |
| 14th Graal (1.1) | relay lobby | none (offline-only) |
| 15th Graal (1.1) | relay lobby | none (offline-only) |
| 1.1.9 Gold Edition | relay lobby | none (offline-only) |
| 1.1 | relay lobby | none (offline-only) |
| Java Graal | relay lobby | none (offline-only) |
| NewWorld beta 1 | direct game server (unconfirmed token) | none (offline-only) |
| NewWorld beta 2 | direct game server (unconfirmed token) | none (offline-only) |
| 1.21 | direct game server | borrowed: lists as 2.05 |
| 1.28 beta 2 | direct game server | borrowed: lists as 2.05 |
| 1.3 test | direct game server | borrowed: lists as 2.05 |
| 1.31 | direct game server | borrowed: lists as 2.05 |
| 1.32 | direct game server | borrowed: lists as 2.05 |
| 1.33 | direct game server | borrowed: lists as 2.05 |
| 1.34 | direct game server | borrowed: lists as 2.05 |
| 1.35 | direct game server | borrowed: lists as 2.05 |
| 1.36 | direct game server | borrowed: lists as 2.05 |
| 1.37 | direct game server | borrowed: lists as 2.05 |
| 1.38 | direct game server | borrowed: lists as 2.05 |
| 1.39 | direct game server | borrowed: lists as 2.05 |
| 1.41 | listserver (classic) | own listserver login |
| 1.42 | listserver (classic) | own listserver login |
| 2.00 | listserver (classic) | own listserver login |
| 2.01 | listserver (classic) | own listserver login |
| 2.02 | listserver (classic) | own listserver login |
| 2.02 rev2 | listserver (classic) | own listserver login |
| 2.03 | listserver (classic) | own listserver login |
| 2.04 | listserver (classic) | own listserver login |
| 2.05 | listserver (classic) | own listserver login |
| 2.1 | listserver (classic) | own listserver login |
| 2.12 | listserver (classic) | own listserver login |
| 2.13 | listserver (classic) | own listserver login |
| 2.14 | listserver (classic) | own listserver login |
| 2.15 | listserver (classic) | own listserver login |
| 2.16 | listserver (classic) | own listserver login |
| 2.17 | listserver (classic) | own listserver login |
| 2.22 | listserver (gen 5) | own listserver login |
| 2.3 | listserver (gen 5) | own listserver login |
| 2.31 | listserver (gen 5) | own listserver login |
| 3.04 | listserver (gen 5) | own listserver login |
| 3.153 | listserver (gen 5) | own listserver login |
| 4.04r2 | Login server | borrowed: lists as 2.05 |
| 4.04r2 Mac | Login server | borrowed: lists as 2.05 |
| 4.20r6 | Login server | borrowed: lists as 2.05 |
| 4.20r8 | Login server | borrowed: lists as 2.05 |
| 5.00r7 | Login server | borrowed: lists as 2.05 |
| 5.12 | Login server | borrowed: lists as 2.05 |
| 5.12 Mac | Login server | borrowed: lists as 2.05 |
| 5.24r4 | Login server | borrowed: lists as 2.05 |
| 5.30r3 | Login server | borrowed: lists as 2.05 |
| 5.31r4 | Login server | borrowed: lists as 2.05 |
| 6.00r2 | Login server | borrowed: lists as 2.05 |
| 6.00r4 | Login server | borrowed: lists as 2.05 |
| 6.015 | Login server | borrowed: lists as 2.05 |
| 6.034 | Login server | borrowed: lists as 2.05 |
| 6.03r7 | Login server | borrowed: lists as 2.05 |
| 6.03r7 Linux | Login server | borrowed: lists as 2.05 |
