//! Asset loading and export (levels, GANIs, images).

pub mod builtin;
pub(crate) mod coverage;
pub mod crc32;
mod gani;
pub mod gif;
mod level;
mod level_write;
pub mod mng;
mod objects;
mod png;
pub(crate) mod resolve;
mod root;
mod tiledefs;
pub mod tutorial;
mod zelda;

pub use coverage::{check_folder, expected_files, format_report, Finding, Need, NeedKind, Status};
pub use objects::{ObjectSet, Stamp};
pub use resolve::{AssetResolver, Layer, TileTypesFile, IMAGE_CATEGORIES};
pub use root::AssetRoot;
pub use tiledefs::{load_tiledefs, parse_tiledefs, TileDef};

pub use crc32::{crc32, crc32_update};
pub use gani::{
    Gani, GaniFrame, GaniPlacement, GaniPlayer, GaniSound, GaniSprite, LISTENER_DX, LISTENER_DY,
};
pub use level::{
    decode_sign_record, decode_sign_text, graal_code_bits, image_vis_height, new_world_tile_type,
    tile_code, tile_id, Baddy, BoardRun, Chest, DrawLayer, GraalLayout, ImageEffect, Level, Link,
    Npc, NpcImage, Object2x2, ObjectTables, Sign, Tile, TileKind, TileTypes, CHEST_ITEMS,
    SCREEN_HEIGHT, TYPE_BED_BOTTOM, TYPE_CHAIR, TYPE_THROW_THROUGH, TYPE_WATER,
};
pub use level_write::{encode_binary, encode_board, encode_sign_text, BinaryLevel, Written};
pub use mng::{
    is_mng, read_mng, write_indexed_png, write_mng, ImageAnimations, IndexedFrame, MngAnimation,
    MngRepeat,
};
pub use png::{
    read_png_rgb, read_png_rgba, read_png_rgba_recolored, write_png_indexed, write_png_rgb,
};
pub use zelda::{is_zelda, Computer, ZeldaLayout, ZeldaLevel, ZeldaLink, ZeldaMagic, ZeldaSign};
