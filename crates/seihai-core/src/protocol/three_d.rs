//! What a client build's Torque 3D side can do: the `three_d` part of a
//! [`VersionProfile`](super::VersionProfile).
//!
//! Only the 3.x to 5.12 clients have one (Graal3D 2000, the DirectX
//! program, is separate and not covered; 1.x, 2.x and 5.24r4 on have none).
//! The values come from Graal_analysis `version-profiles/three-d.toml`, the
//! source of truth, and are checked against the mirrored fixture
//! `three_d_toml.tsv` (regenerate it with seihai's
//! `tools/gen_three_d_fixture.py`).
//!
//! Every field is either confirmed for that build (a static read, a lab run
//! or a capture) or **assumed**: carried from the nearest read build or from
//! stock TGE 1.x. [`ThreeD::assumed`] lists the assumed fields, so a caller
//! can treat an assumed value as a default rather than a fact.
//!
//! The client (Ato) and the server (seihai-gserver-v2) read this to decide,
//! per client build, what to offer: whether to stream terrain, whether a
//! `.cs` may be sent, which class may be the control object, how to name a
//! texture, how to frame the connection.

/// The class table Graal adds to the connect accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassTable {
    /// Stock Torque: no table, class ids sized from the class count.
    None,
    /// Three types: object, datablock, event (protocol 19).
    WithDatablocks,
    /// Two types: object, event (protocols 21 and 24).
    NoDatablocks,
}

impl ClassTable {
    /// The number of class types the table lists (0 for none).
    pub const fn types(self) -> u8 {
        match self {
            ClassTable::None => 0,
            ClassTable::WithDatablocks => 3,
            ClassTable::NoDatablocks => 2,
        }
    }
}

/// Which game layer the 3D side runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameLayer {
    /// Torque's own: datablocks, `Player`, `ShapeBase`, moves.
    Tge,
    /// Graal's `GameObject` model: no datablocks, the client reports its position.
    GameObject,
}

/// A class the client accepts as its control object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlObject {
    Player,
    Camera,
    GameObject,
}

impl ControlObject {
    pub const fn class_name(self) -> &'static str {
        match self {
            ControlObject::Player => "Player",
            ControlObject::Camera => "Camera",
            ControlObject::GameObject => "GameObject",
        }
    }
}

/// Where the client caches streamed terrain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkCache {
    /// No streamed terrain.
    None,
    /// `levels3d/heights/<server>/heights<x>-<y>.dat` plus `heights.info`
    /// (one texture name per line); the server name escapes a space as `%032`.
    Heights,
}

/// The terrain a build can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terrain {
    /// Loads a local `.ter` file from `levels3d/terrain/`.
    pub ter_file: bool,
    /// Highest `.ter` version read; 0 when there is no `.ter` reader.
    pub ter_max: u8,
    /// Takes streamed terrain (`TerrainChunkEvent`, `TerrainRequestUpdatesEvent`).
    pub chunks: bool,
    pub cache: ChunkCache,
    /// Terrain textures must be this (width, height, bits); `None` when not read.
    pub texture: Option<(u16, u16, u8)>,
    /// Largest zlib payload of one `TerrainChunkEvent` (an int10 length); 0 without chunks.
    pub chunk_max: u16,
}

/// Whether the build compiles and runs TorqueScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TorqueScript {
    /// The compiler, `exec` and `compile` are present.
    Compiles,
    /// No compiler: GS2 replaced TorqueScript.
    None,
}

/// How much of Torque's `Terraformer` the build keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terraformer {
    /// The stock procedural generator.
    Full,
    /// Only `loadGreyscale`, `saveGreyscale` and `setTerrain`.
    Greyscale,
    None,
}

/// The 3D content formats a build reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Formats {
    /// Highest DTS shape version.
    pub dts_max: u8,
    /// DIF interior resource version; 0 for none.
    pub dif_version: u8,
    /// Reads `.3ds` shapes.
    pub three_ds: bool,
    /// Quake 3 `.md3` version; 0 for none.
    pub md3_version: u8,
}

/// How the client names a material's texture when it asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureNames {
    /// Strips `.png` and requests and stores the bare name (5.12, capture).
    StripPng,
    /// Requests the name as given.
    AsNamed,
}

/// One field of [`ThreeD`], for [`ThreeD::assumed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Field {
    Protocol,
    Game,
    ClassTable,
    ClassIds12Bit,
    LengthPrefixes,
    GameLayer,
    MissionPhases,
    PhasesOptional,
    ClientMoves,
    ControlObjects,
    TerFile,
    TerMax,
    TerrainChunks,
    ChunkCache,
    TerrainTexture,
    Water,
    TorqueScript,
    Terraformer,
    TerrainEditor,
    DtsMax,
    DifVersion,
    ThreeDs,
    Md3Version,
    TextureNames,
    DownloadRefused,
    Bigdata,
    BigdataChunkMax,
    TerrainChunkMax,
    UdpPortOffset,
}

impl Field {
    /// Every field, in the order of `three-d.toml`.
    pub const ALL: [Field; 29] = [
        Field::Protocol,
        Field::Game,
        Field::ClassTable,
        Field::ClassIds12Bit,
        Field::LengthPrefixes,
        Field::GameLayer,
        Field::MissionPhases,
        Field::PhasesOptional,
        Field::ClientMoves,
        Field::ControlObjects,
        Field::TerFile,
        Field::TerMax,
        Field::TerrainChunks,
        Field::ChunkCache,
        Field::TerrainTexture,
        Field::Water,
        Field::TorqueScript,
        Field::Terraformer,
        Field::TerrainEditor,
        Field::DtsMax,
        Field::DifVersion,
        Field::ThreeDs,
        Field::Md3Version,
        Field::TextureNames,
        Field::DownloadRefused,
        Field::Bigdata,
        Field::BigdataChunkMax,
        Field::TerrainChunkMax,
        Field::UdpPortOffset,
    ];

    /// The key in `three-d.toml`.
    pub const fn key(self) -> &'static str {
        match self {
            Field::Protocol => "protocol",
            Field::Game => "game",
            Field::ClassTable => "class_table",
            Field::ClassIds12Bit => "class_ids_12bit",
            Field::LengthPrefixes => "length_prefixes",
            Field::GameLayer => "game_layer",
            Field::MissionPhases => "mission_phases",
            Field::PhasesOptional => "phases_optional",
            Field::ClientMoves => "client_moves",
            Field::ControlObjects => "control_objects",
            Field::TerFile => "ter_file",
            Field::TerMax => "ter_max",
            Field::TerrainChunks => "terrain_chunks",
            Field::ChunkCache => "chunk_cache",
            Field::TerrainTexture => "terrain_texture",
            Field::Water => "water",
            Field::TorqueScript => "torque_script",
            Field::Terraformer => "terraformer",
            Field::TerrainEditor => "terrain_editor",
            Field::DtsMax => "dts_max",
            Field::DifVersion => "dif_version",
            Field::ThreeDs => "three_ds",
            Field::Md3Version => "md3_version",
            Field::TextureNames => "texture_names",
            Field::DownloadRefused => "download_refused",
            Field::Bigdata => "bigdata",
            Field::BigdataChunkMax => "bigdata_chunk_max",
            Field::TerrainChunkMax => "terrain_chunk_max",
            Field::UdpPortOffset => "udp_port_offset",
        }
    }

    pub fn from_key(key: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == key)
    }
}

/// A set of [`Field`]s: the fields of a row that are assumed.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Fields(u32);

impl Fields {
    pub const NONE: Fields = Fields(0);
    /// Every field: a row whose 3D side was not read at all.
    pub const ALL: Fields = Fields((1 << Field::ALL.len()) - 1);

    pub const fn of(fields: &[Field]) -> Fields {
        let mut m = 0u32;
        let mut i = 0;
        while i < fields.len() {
            m |= 1 << fields[i] as u8;
            i += 1;
        }
        Fields(m)
    }

    /// Every field except these.
    pub const fn all_but(fields: &[Field]) -> Fields {
        Fields(Fields::ALL.0 & !Fields::of(fields).0)
    }

    pub const fn contains(self, f: Field) -> bool {
        self.0 & (1 << f as u8) != 0
    }

    pub fn iter(self) -> impl Iterator<Item = Field> {
        Field::ALL.into_iter().filter(move |f| self.contains(*f))
    }
}

impl std::fmt::Debug for Fields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter().map(Field::key)).finish()
    }
}

/// A build's Torque 3D side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreeD {
    /// Torque protocol number; the accept must carry exactly this.
    pub protocol: u16,
    /// Game string in the connect request.
    pub game: &'static str,
    pub class_table: ClassTable,
    /// Class ids are a fixed 12 bits in the server's numbering.
    pub class_ids_12bit: bool,
    /// Every event and ghost update carries an int14 bit length.
    pub length_prefixes: bool,
    pub game_layer: GameLayer,
    /// Mission phases the client walks.
    pub mission_phases: u8,
    /// The client still builds the world when the server sends no phases.
    pub phases_optional: bool,
    /// The client sends Torque `Move`s; otherwise it reports its own position.
    pub client_moves: bool,
    /// Classes the client accepts as its control object.
    pub control_objects: &'static [ControlObject],
    pub terrain: Terrain,
    /// `WaterBlock` is a networked class.
    pub water: bool,
    pub torque_script: TorqueScript,
    pub terraformer: Terraformer,
    /// Torque's `TerrainEditor` is present.
    pub terrain_editor: bool,
    pub formats: Formats,
    pub texture_names: TextureNames,
    /// Extensions the resource-miss path never asks the gserver for.
    pub download_refused: &'static [&'static str],
    /// `BigDataStartEvent` / `BigDataChunkEvent` text channel.
    pub bigdata: bool,
    /// Largest `BigDataChunkEvent` payload (an int8 length).
    pub bigdata_chunk_max: u8,
    /// The Torque UDP port is the listed gserver port plus this.
    pub udp_port_offset: u8,
    /// The fields that are assumed for this build, not read.
    pub assumed: Fields,
}

impl ThreeD {
    /// Whether `field` was read (statically, in a lab or in a capture) for this build.
    pub const fn confirmed(&self, field: Field) -> bool {
        !self.assumed.contains(field)
    }

    /// The same row with every field assumed: a build whose 3D side was not read.
    pub const fn all_assumed(self) -> ThreeD {
        ThreeD {
            assumed: Fields::ALL,
            ..self
        }
    }

    /// A server may stream terrain chunks to this build.
    pub const fn offers_terrain_chunks(&self) -> bool {
        self.terrain.chunks
    }

    /// A server may send TorqueScript (`.cs`, `.dso`) for this build to run.
    pub const fn runs_torque_script(&self) -> bool {
        matches!(self.torque_script, TorqueScript::Compiles)
    }

    /// The build takes datablocks (protocol 19 and the stock TGE layer).
    pub const fn has_datablocks(&self) -> bool {
        matches!(self.game_layer, GameLayer::Tge)
    }

    /// `class` may be the control object of this build.
    pub fn accepts_control(&self, class: ControlObject) -> bool {
        self.control_objects.contains(&class)
    }

    /// The client would ask the gserver for a missing resource named `name`.
    /// Case-insensitive on the extension, as the client lowercases names.
    pub fn requests_download(&self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        !self.download_refused.iter().any(|ext| lower.ends_with(ext))
    }

    /// The name the client asks for (and stores under `levels3d/images`)
    /// when a material names the texture `texture`.
    pub fn texture_request_name<'a>(&self, texture: &'a str) -> &'a str {
        match self.texture_names {
            TextureNames::StripPng => {
                let n = texture.len();
                if n >= 4 && texture[n - 4..].eq_ignore_ascii_case(".png") {
                    &texture[..n - 4]
                } else {
                    texture
                }
            }
            TextureNames::AsNamed => texture,
        }
    }

    /// A terrain texture of this size and depth is drawn (true when the
    /// build has no rule).
    pub fn terrain_texture_ok(&self, width: u16, height: u16, bits: u8) -> bool {
        match self.terrain.texture {
            Some(t) => t == (width, height, bits),
            None => true,
        }
    }

    /// The build reads a DTS shape of this version.
    pub const fn reads_dts(&self, version: u8) -> bool {
        version <= self.formats.dts_max
    }

    /// The Torque UDP port for a server listed on `listed_port`.
    pub const fn udp_port(&self, listed_port: u16) -> u16 {
        listed_port.wrapping_add(self.udp_port_offset as u16)
    }
}

const PLAYER_CAMERA: &[ControlObject] = &[ControlObject::Player, ControlObject::Camera];
const GAMEOBJECT_CAMERA: &[ControlObject] = &[ControlObject::GameObject, ControlObject::Camera];
const REFUSE_SCRIPTS_TER: &[&str] = &[".dso", ".cs", ".ter"];

/// 3.04 Linux: stock TGE 1.x networking, no Graal changes, runs TorqueScript.
pub const THREE_D_304: ThreeD = ThreeD {
    protocol: 9,
    game: "Torque Game Engine Demo",
    class_table: ClassTable::None,
    class_ids_12bit: false,
    length_prefixes: false,
    game_layer: GameLayer::Tge,
    mission_phases: 3,
    phases_optional: false,
    client_moves: true,
    control_objects: PLAYER_CAMERA,
    terrain: Terrain {
        ter_file: true,
        ter_max: 3,
        chunks: false,
        cache: ChunkCache::None,
        texture: None,
        chunk_max: 0,
    },
    water: true,
    torque_script: TorqueScript::Compiles,
    terraformer: Terraformer::Full,
    terrain_editor: true,
    formats: Formats {
        dts_max: 24,
        dif_version: 44,
        three_ds: false,
        md3_version: 0,
    },
    texture_names: TextureNames::AsNamed,
    download_refused: &[],
    bigdata: false,
    bigdata_chunk_max: 0,
    udp_port_offset: 1,
    assumed: Fields::of(&[
        Field::MissionPhases,
        Field::PhasesOptional,
        Field::ClientMoves,
        Field::ControlObjects,
        Field::TerFile,
        Field::TerMax,
        Field::TerrainTexture,
        Field::Water,
        Field::DtsMax,
        Field::DifVersion,
        Field::ThreeDs,
        Field::Md3Version,
        Field::TextureNames,
        Field::DownloadRefused,
        Field::UdpPortOffset,
    ]),
};

/// 3.153 Mac: Graal's net changes, the TGE game layer, TorqueScript, both terrain paths.
pub const THREE_D_3153: ThreeD = ThreeD {
    protocol: 19,
    game: "Graal3D",
    class_table: ClassTable::WithDatablocks,
    class_ids_12bit: true,
    length_prefixes: true,
    game_layer: GameLayer::Tge,
    mission_phases: 3,
    phases_optional: false,
    client_moves: true,
    control_objects: PLAYER_CAMERA,
    terrain: Terrain {
        ter_file: true,
        ter_max: 3,
        chunks: true,
        cache: ChunkCache::Heights,
        texture: None,
        chunk_max: 1023,
    },
    water: true,
    torque_script: TorqueScript::Compiles,
    terraformer: Terraformer::Full,
    terrain_editor: true,
    formats: Formats {
        dts_max: 24,
        dif_version: 44,
        three_ds: true,
        md3_version: 15,
    },
    texture_names: TextureNames::AsNamed,
    download_refused: &[],
    bigdata: true,
    bigdata_chunk_max: 191,
    udp_port_offset: 1,
    assumed: Fields::of(&[
        Field::MissionPhases,
        Field::PhasesOptional,
        Field::ControlObjects,
        Field::TerMax,
        Field::ChunkCache,
        Field::TerrainTexture,
        Field::Water,
        Field::DtsMax,
        Field::DifVersion,
        Field::Md3Version,
        Field::TextureNames,
        Field::DownloadRefused,
        Field::BigdataChunkMax,
        Field::TerrainChunkMax,
    ]),
};

/// 4.04r2 Windows `Graal3DEngine.dll`: protocol 19 with datablocks, no
/// TorqueScript compiler, greyscale-only Terraformer, `.ter` and chunks.
pub const THREE_D_404: ThreeD = ThreeD {
    protocol: 19,
    game: "Graal3D",
    class_table: ClassTable::WithDatablocks,
    class_ids_12bit: true,
    length_prefixes: true,
    game_layer: GameLayer::Tge,
    mission_phases: 3,
    phases_optional: false,
    client_moves: true,
    control_objects: PLAYER_CAMERA,
    terrain: Terrain {
        ter_file: true,
        ter_max: 3,
        chunks: true,
        cache: ChunkCache::Heights,
        texture: None,
        chunk_max: 1023,
    },
    water: true,
    torque_script: TorqueScript::None,
    terraformer: Terraformer::Greyscale,
    terrain_editor: true,
    formats: Formats {
        dts_max: 24,
        dif_version: 44,
        three_ds: true,
        md3_version: 15,
    },
    texture_names: TextureNames::AsNamed,
    download_refused: REFUSE_SCRIPTS_TER,
    bigdata: true,
    bigdata_chunk_max: 191,
    udp_port_offset: 1,
    assumed: Fields::of(&[
        Field::PhasesOptional,
        Field::ControlObjects,
        Field::ChunkCache,
        Field::TerrainTexture,
        Field::TextureNames,
        Field::TerrainChunkMax,
    ]),
};

/// 4.20r6 Mac: protocol 21, the `GameObject` layer, streamed terrain only.
pub const THREE_D_420: ThreeD = ThreeD {
    protocol: 21,
    game: "Graal3D",
    class_table: ClassTable::NoDatablocks,
    class_ids_12bit: true,
    length_prefixes: true,
    game_layer: GameLayer::GameObject,
    mission_phases: 2,
    phases_optional: false,
    client_moves: false,
    control_objects: GAMEOBJECT_CAMERA,
    terrain: Terrain {
        ter_file: false,
        ter_max: 0,
        chunks: true,
        cache: ChunkCache::Heights,
        texture: Some((256, 256, 24)),
        chunk_max: 1023,
    },
    water: true,
    torque_script: TorqueScript::None,
    terraformer: Terraformer::Greyscale,
    terrain_editor: true,
    formats: Formats {
        dts_max: 24,
        dif_version: 44,
        three_ds: true,
        md3_version: 0,
    },
    texture_names: TextureNames::StripPng,
    download_refused: REFUSE_SCRIPTS_TER,
    bigdata: true,
    bigdata_chunk_max: 191,
    udp_port_offset: 1,
    assumed: Fields::all_but(&[
        Field::Protocol,
        Field::Game,
        Field::ClassTable,
        Field::ClassIds12Bit,
        Field::LengthPrefixes,
        Field::GameLayer,
        Field::ClientMoves,
        Field::TerrainChunks,
        Field::Water,
        Field::Bigdata,
    ]),
};

/// 4.20r8 Windows: 4.20r6's values. Its 3D DLL shows the `GameObject`
/// layer, two phases, no TorqueScript and terrain chunks; the protocol
/// number was not read.
pub const THREE_D_4208: ThreeD = ThreeD {
    assumed: Fields::all_but(&[
        Field::GameLayer,
        Field::MissionPhases,
        Field::TorqueScript,
        Field::TerrainChunks,
    ]),
    ..THREE_D_420
};

/// 5.00r7 Windows: protocol 24, the `GameObject` layer, streamed terrain.
pub const THREE_D_500: ThreeD = ThreeD {
    protocol: 24,
    game: "Graal3D",
    class_table: ClassTable::NoDatablocks,
    class_ids_12bit: true,
    length_prefixes: true,
    game_layer: GameLayer::GameObject,
    mission_phases: 2,
    phases_optional: true,
    client_moves: false,
    control_objects: GAMEOBJECT_CAMERA,
    terrain: Terrain {
        ter_file: false,
        ter_max: 0,
        chunks: true,
        cache: ChunkCache::Heights,
        texture: Some((256, 256, 24)),
        chunk_max: 1023,
    },
    water: true,
    torque_script: TorqueScript::None,
    terraformer: Terraformer::Greyscale,
    terrain_editor: true,
    formats: Formats {
        dts_max: 24,
        dif_version: 44,
        three_ds: true,
        md3_version: 0,
    },
    texture_names: TextureNames::StripPng,
    download_refused: REFUSE_SCRIPTS_TER,
    bigdata: true,
    bigdata_chunk_max: 191,
    udp_port_offset: 1,
    assumed: Fields::all_but(&[
        Field::Protocol,
        Field::Game,
        Field::ClassTable,
        Field::GameLayer,
        Field::MissionPhases,
        Field::TerrainChunks,
        Field::ChunkCache,
    ]),
};

/// 5.12 Windows: the most-read build. Streamed terrain and its cache run
/// live; a `Camera` control object and bare-name textures seen in a capture.
pub const THREE_D_512: ThreeD = ThreeD {
    assumed: Fields::of(&[Field::DtsMax, Field::DifVersion, Field::Md3Version]),
    ..THREE_D_500
};

/// Every 3D row, with the label of the build it belongs to, in chain order.
/// The Mac 4.04r2 and 5.12 rows are their Windows twin, all assumed.
pub const ROWS: [(&str, ThreeD); 9] = [
    ("3.04", THREE_D_304),
    ("3.153", THREE_D_3153),
    ("4.04r2", THREE_D_404),
    ("4.04r2 Mac", THREE_D_404.all_assumed()),
    ("4.20r6", THREE_D_420),
    ("4.20r8", THREE_D_4208),
    ("5.00r7", THREE_D_500),
    ("5.12", THREE_D_512),
    ("5.12 Mac", THREE_D_512.all_assumed()),
];

/// The fixture row for one build: the same columns, order and spelling as
/// `three_d_toml.tsv`. The unit test compares every profile's row with it.
pub fn fixture_row(label: &str, t: &ThreeD) -> String {
    let b = |v: bool| if v { "1" } else { "0" };
    let control: Vec<&str> = t.control_objects.iter().map(|c| c.class_name()).collect();
    let refused = if t.download_refused.is_empty() {
        "-".to_string()
    } else {
        t.download_refused.join(",")
    };
    let texture = match t.terrain.texture {
        Some((w, h, d)) => format!("{w}x{h}x{d}"),
        None => "-".into(),
    };
    let assumed: Vec<&str> = t.assumed.iter().map(Field::key).collect();
    let assumed = if t.assumed == Fields::ALL {
        "all".to_string()
    } else if assumed.is_empty() {
        "-".to_string()
    } else {
        assumed.join(",")
    };
    [
        label.to_string(),
        t.protocol.to_string(),
        t.game.to_string(),
        t.class_table.types().to_string(),
        b(t.class_ids_12bit).into(),
        b(t.length_prefixes).into(),
        match t.game_layer {
            GameLayer::Tge => "tge",
            GameLayer::GameObject => "gameobject",
        }
        .into(),
        t.mission_phases.to_string(),
        b(t.phases_optional).into(),
        b(t.client_moves).into(),
        control.join(","),
        b(t.terrain.ter_file).into(),
        t.terrain.ter_max.to_string(),
        b(t.terrain.chunks).into(),
        match t.terrain.cache {
            ChunkCache::None => "none",
            ChunkCache::Heights => "heights",
        }
        .into(),
        texture,
        b(t.water).into(),
        match t.torque_script {
            TorqueScript::Compiles => "compiles",
            TorqueScript::None => "none",
        }
        .into(),
        match t.terraformer {
            Terraformer::Full => "full",
            Terraformer::Greyscale => "greyscale",
            Terraformer::None => "none",
        }
        .into(),
        b(t.terrain_editor).into(),
        t.formats.dts_max.to_string(),
        t.formats.dif_version.to_string(),
        b(t.formats.three_ds).into(),
        t.formats.md3_version.to_string(),
        match t.texture_names {
            TextureNames::StripPng => "strip_png",
            TextureNames::AsNamed => "as_named",
        }
        .into(),
        refused,
        b(t.bigdata).into(),
        t.bigdata_chunk_max.to_string(),
        t.terrain.chunk_max.to_string(),
        t.udp_port_offset.to_string(),
        assumed,
    ]
    .join("\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{profile, ClientVersion, PROFILES};

    const FIXTURE: &str = include_str!("three_d_toml.tsv");

    /// Every profile's `three_d` matches the row mirrored from Graal_analysis
    /// `version-profiles/three-d.toml`, field for field and assumption for
    /// assumption, and every build without a row has no 3D side.
    #[test]
    fn profiles_match_the_graal_analysis_fixture() {
        let rows: Vec<&str> = FIXTURE
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .collect();
        assert_eq!(rows.len(), ROWS.len(), "one fixture row per 3D build");
        for want in &rows {
            let label = want.split('\t').next().unwrap();
            let p = profile::by_label(label).unwrap_or_else(|| panic!("no profile {label}"));
            let t = p
                .three_d
                .unwrap_or_else(|| panic!("{label} has no three_d"));
            assert_eq!(&fixture_row(label, &t), want, "{label}");
        }
        for p in PROFILES {
            if !rows.iter().any(|r| r.split('\t').next() == Some(p.label)) {
                assert!(
                    p.three_d.is_none(),
                    "{} has a three_d but no fixture row",
                    p.label
                );
            }
        }
    }

    #[test]
    fn rows_are_the_profiles() {
        for (label, t) in ROWS {
            assert_eq!(
                profile::by_label(label).unwrap().three_d,
                Some(t),
                "{label}"
            );
        }
    }

    fn of(v: ClientVersion) -> ThreeD {
        profile(v)
            .three_d
            .unwrap_or_else(|| panic!("{v:?} has no three_d"))
    }

    #[test]
    fn only_3x_to_512_have_a_3d_side() {
        for v in [
            ClientVersion::V121,
            ClientVersion::V205,
            ClientVersion::V222,
            ClientVersion::V231,
            ClientVersion::V524R4,
            ClientVersion::V530R3,
            ClientVersion::V531R4,
            ClientVersion::V600R2,
            ClientVersion::V603R7,
            ClientVersion::V603R7Linux,
        ] {
            assert!(profile(v).three_d.is_none(), "{v:?}");
        }
    }

    #[test]
    fn protocols_and_game_layers() {
        let want = [
            (ClientVersion::V304, 9, GameLayer::Tge, ClassTable::None),
            (
                ClientVersion::V3153,
                19,
                GameLayer::Tge,
                ClassTable::WithDatablocks,
            ),
            (
                ClientVersion::V404R2,
                19,
                GameLayer::Tge,
                ClassTable::WithDatablocks,
            ),
            (
                ClientVersion::V404R2Mac,
                19,
                GameLayer::Tge,
                ClassTable::WithDatablocks,
            ),
            (
                ClientVersion::V420R6,
                21,
                GameLayer::GameObject,
                ClassTable::NoDatablocks,
            ),
            (
                ClientVersion::V420R8,
                21,
                GameLayer::GameObject,
                ClassTable::NoDatablocks,
            ),
            (
                ClientVersion::V500R7,
                24,
                GameLayer::GameObject,
                ClassTable::NoDatablocks,
            ),
            (
                ClientVersion::V512,
                24,
                GameLayer::GameObject,
                ClassTable::NoDatablocks,
            ),
            (
                ClientVersion::V512Mac,
                24,
                GameLayer::GameObject,
                ClassTable::NoDatablocks,
            ),
        ];
        for (v, protocol, layer, table) in want {
            let t = of(v);
            assert_eq!(t.protocol, protocol, "{v:?}");
            assert_eq!(t.game_layer, layer, "{v:?}");
            assert_eq!(t.class_table, table, "{v:?}");
            assert_eq!(t.has_datablocks(), layer == GameLayer::Tge, "{v:?}");
            assert_eq!(t.client_moves, layer == GameLayer::Tge, "{v:?}");
            assert_eq!(t.udp_port(14800), 14801);
        }
        // Stock TGE in 3.04, Graal3D from 3.153 on.
        assert_eq!(of(ClientVersion::V304).game, "Torque Game Engine Demo");
        assert!(!of(ClientVersion::V304).length_prefixes);
        assert_eq!(of(ClientVersion::V3153).game, "Graal3D");
        // 4.20r8's protocol was never read.
        assert!(!of(ClientVersion::V420R8).confirmed(Field::Protocol));
        assert!(of(ClientVersion::V420R6).confirmed(Field::Protocol));
    }

    /// Only 3.04 and 3.153 run TorqueScript; 4.04r2 and 5.12 refuse to
    /// download `.cs`, `.dso` and `.ter`.
    #[test]
    fn torque_script_and_download_filter() {
        assert!(of(ClientVersion::V304).runs_torque_script());
        assert!(of(ClientVersion::V3153).runs_torque_script());
        for v in [
            ClientVersion::V404R2,
            ClientVersion::V420R6,
            ClientVersion::V420R8,
            ClientVersion::V500R7,
            ClientVersion::V512,
        ] {
            let t = of(v);
            assert!(!t.runs_torque_script(), "{v:?}");
            assert!(!t.requests_download("main.cs"), "{v:?}");
            assert!(!t.requests_download("MAIN.DSO"), "{v:?}");
            assert!(!t.requests_download("world.ter"), "{v:?}");
            assert!(t.requests_download("tree.dts"), "{v:?}");
            assert!(t.requests_download("grass"), "{v:?}");
        }
        assert!(of(ClientVersion::V512).confirmed(Field::TorqueScript));
        assert!(of(ClientVersion::V404R2).confirmed(Field::DownloadRefused));
        assert!(!of(ClientVersion::V500R7).confirmed(Field::TorqueScript));
        // 3.153 compiles scripts, so it is not assumed to refuse them.
        assert!(of(ClientVersion::V3153).requests_download("main.cs"));
    }

    #[test]
    fn terrain_per_build() {
        // (version, .ter, chunks, chunk payload/cache confirmed)
        let want = [
            (ClientVersion::V304, true, false),
            (ClientVersion::V3153, true, true),
            (ClientVersion::V404R2, true, true),
            (ClientVersion::V420R6, false, true),
            (ClientVersion::V420R8, false, true),
            (ClientVersion::V500R7, false, true),
            (ClientVersion::V512, false, true),
        ];
        for (v, ter, chunks) in want {
            let t = of(v);
            assert_eq!(t.terrain.ter_file, ter, "{v:?}");
            assert_eq!(t.offers_terrain_chunks(), chunks, "{v:?}");
            assert_eq!(t.terrain.cache == ChunkCache::Heights, chunks, "{v:?}");
            assert_eq!(t.terrain.chunk_max > 0, chunks, "{v:?}");
        }
        let t512 = of(ClientVersion::V512);
        assert!(t512.confirmed(Field::TerrainChunks));
        assert!(t512.confirmed(Field::ChunkCache));
        assert!(t512.confirmed(Field::TerrainChunkMax));
        assert_eq!(t512.terrain.chunk_max, 1023);
        assert!(t512.terrain_texture_ok(256, 256, 24));
        assert!(!t512.terrain_texture_ok(512, 512, 24));
        assert!(!t512.terrain_texture_ok(256, 256, 32));
        // Only 5.12's chunk layout was read; the others are assumed to match.
        assert!(!of(ClientVersion::V420R6).confirmed(Field::ChunkCache));
        assert!(of(ClientVersion::V500R7).confirmed(Field::ChunkCache));
        assert!(!of(ClientVersion::V500R7).confirmed(Field::TerrainChunkMax));
    }

    #[test]
    fn terrain_tools() {
        assert_eq!(of(ClientVersion::V3153).terraformer, Terraformer::Full);
        assert_eq!(
            of(ClientVersion::V404R2).terraformer,
            Terraformer::Greyscale
        );
        assert_eq!(of(ClientVersion::V512).terraformer, Terraformer::Greyscale);
        for v in [
            ClientVersion::V3153,
            ClientVersion::V404R2,
            ClientVersion::V512,
        ] {
            assert!(of(v).terrain_editor, "{v:?}");
            assert!(of(v).confirmed(Field::TerrainEditor), "{v:?}");
        }
        assert!(!of(ClientVersion::V420R6).confirmed(Field::Terraformer));
    }

    #[test]
    fn control_objects_and_phases() {
        let t512 = of(ClientVersion::V512);
        assert!(t512.accepts_control(ControlObject::GameObject));
        assert!(t512.accepts_control(ControlObject::Camera));
        assert!(!t512.accepts_control(ControlObject::Player));
        assert!(t512.phases_optional);
        assert_eq!(t512.mission_phases, 2);
        let t404 = of(ClientVersion::V404R2);
        assert!(t404.accepts_control(ControlObject::Player));
        assert!(!t404.accepts_control(ControlObject::GameObject));
        assert_eq!(t404.mission_phases, 3);
        assert!(!t404.phases_optional);
        // Not played on 4.04r2 yet.
        assert!(!t404.confirmed(Field::ControlObjects));
    }

    #[test]
    fn texture_names_and_formats() {
        let t512 = of(ClientVersion::V512);
        assert_eq!(t512.texture_request_name("grass.png"), "grass");
        assert_eq!(t512.texture_request_name("Grass.PNG"), "Grass");
        assert_eq!(t512.texture_request_name("rock.jpg"), "rock.jpg");
        assert_eq!(t512.texture_request_name("png"), "png");
        assert!(t512.confirmed(Field::TextureNames));
        let t404 = of(ClientVersion::V404R2);
        assert_eq!(t404.texture_request_name("grass.png"), "grass.png");
        assert!(!t404.confirmed(Field::TextureNames));
        assert!(t404.reads_dts(24));
        assert!(!t404.reads_dts(25));
        assert!(t404.confirmed(Field::DtsMax));
        assert!(!t512.confirmed(Field::DtsMax));
        assert_eq!(t404.formats.dif_version, 44);
        assert!(!of(ClientVersion::V304).formats.three_ds);
        assert!(t512.formats.three_ds && t512.confirmed(Field::ThreeDs));
    }

    #[test]
    fn mac_twins_are_all_assumed() {
        for (mac, win) in [
            (ClientVersion::V404R2Mac, ClientVersion::V404R2),
            (ClientVersion::V512Mac, ClientVersion::V512),
        ] {
            let m = of(mac);
            assert_eq!(m.assumed, Fields::ALL);
            assert_eq!(
                ThreeD {
                    assumed: Fields::NONE,
                    ..m
                },
                ThreeD {
                    assumed: Fields::NONE,
                    ..of(win)
                }
            );
            assert!(Field::ALL.iter().all(|f| !m.confirmed(*f)));
        }
    }

    #[test]
    fn field_keys_round_trip() {
        for f in Field::ALL {
            assert_eq!(Field::from_key(f.key()), Some(f));
        }
        assert_eq!(Fields::ALL.iter().count(), Field::ALL.len());
    }
}
