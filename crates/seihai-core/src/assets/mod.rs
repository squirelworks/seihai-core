//! Asset loading and export (levels, GANIs, images).

pub mod builtin;
pub mod crc32;
mod gani;
pub mod gif;
mod level;
mod objects;
mod png;
pub(crate) mod resolve;
mod root;
mod tiledefs;
mod zelda;
pub(crate) mod coverage;

pub use objects::{ObjectSet, Stamp};
pub use coverage::{check_folder, expected_files, format_report, Finding, Need, NeedKind, Status};
pub use resolve::{AssetResolver, Layer, TileTypesFile, IMAGE_CATEGORIES};
pub use root::AssetRoot;
pub use tiledefs::{load_tiledefs, parse_tiledefs, TileDef};

pub use crc32::{crc32, crc32_update};
pub use gani::{Gani, GaniFrame, GaniPlacement, GaniPlayer, GaniSound, GaniSprite, LISTENER_DX, LISTENER_DY};
pub use level::{
    CHEST_ITEMS, decode_sign_record, decode_sign_text, graal_code_bits, GraalLayout, tile_code, tile_id, Baddy, BoardRun, Chest, DrawLayer, ImageEffect, Level,
    Link, Npc, NpcImage, Object2x2, ObjectTables, Sign, Tile, TileKind, TileTypes, new_world_tile_type, image_vis_height, SCREEN_HEIGHT, TYPE_BED_BOTTOM, TYPE_CHAIR, TYPE_THROW_THROUGH, TYPE_WATER,
};
pub use zelda::{is_zelda, Computer, ZeldaLayout, ZeldaLevel, ZeldaLink, ZeldaMagic, ZeldaSign};
pub use png::{read_png_rgb, read_png_rgba, read_png_rgba_recolored, write_png_indexed, write_png_rgb};
