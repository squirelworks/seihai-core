//! Seihai core — the shared part of the Graal Reborn-compatible clients.
//!
//! Protocol codecs, asset parsers, the GS1 engine, the offline simulation,
//! the version profiles and the connection code. Nothing here draws or
//! opens a window; the client binaries (`seihai-mae`, `seihai-ato`) do.

pub mod arrows;
pub mod assets;
pub mod baddies;
pub mod bombs;
pub mod chatlists;
pub mod combat;
pub mod config;
pub mod dev;
pub mod filecache;
pub mod difftest;
pub mod gmap;
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
pub mod viewer;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
