//! Seihai core — the shared part of the Graal Reborn-compatible clients.
//!
//! Protocol codecs, asset parsers, the GS1 engine, the offline simulation,
//! the version profiles and the connection code. Nothing here draws or
//! opens a window; the client binaries (`seihai-mae`, `seihai-ato`) do.
//!
//! # Stability
//!
//! From 0.1.0 a breaking change to the public API bumps the minor version
//! (0.1 to 0.2) and anything else bumps the patch (0.1.0 to 0.1.1); see
//! the README. Three modules are tools for the suite's own binaries and are
//! hidden from the docs and left out of that promise: `dev` (developer
//! mode), `difftest` (capture diffing) and `viewer` (the capture viewer).

pub mod arrows;
pub mod assets;
pub mod baddies;
pub mod bombs;
pub mod chatlists;
pub mod client;
pub mod combat;
pub mod config;
/// Developer mode. Outside the API stability promise; see the crate docs.
#[doc(hidden)]
pub mod dev;
/// Capture diffing for the `difftest` tool. Outside the API stability promise.
#[doc(hidden)]
pub mod difftest;
pub mod filecache;
pub mod gmap;
pub mod graaldes;
pub mod items;
pub mod life;
pub mod looks;
pub mod net;
pub mod npcart;
pub mod offline;
pub mod pose;
pub mod protocol;
pub mod scripting;
pub mod servercache;
pub mod sound;
pub mod throwing;
/// The capture viewer (`seihai capture`). Outside the API stability promise.
#[doc(hidden)]
pub mod viewer;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
