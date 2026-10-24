//! Maps and world: lot files (`zombie.iso.LotHeader`, `IsoLot`, `MapFiles`),
//! the meta grid (`IsoMetaGrid`, `RoomDef`, `BuildingDef`), tile definitions
//! and chunk loading (`IsoCell`, `IsoChunk`, `IsoGridSquare`).
//!
//! Map data is read from the installed game's `media/maps` at runtime; this
//! crate contains no game data.

pub mod bra;
pub mod chunk;
pub mod chunk_data;
pub mod defs;
pub mod fix2x;
pub mod iso_lot;
pub mod java;
pub mod map_files;
pub mod meta_grid;
pub mod meta_zones;
pub mod tiledef;
pub mod zones;

pub use chunk::{
    ChunkLoadEnv, IsoCell, IsoChunk, IsoGridSquare, IsoObject, SpriteLookup, TileDefSprites,
};
pub use defs::{BuildingDef, BuildingIdx, MetaObject, RoomDef, RoomIdx, RoomRect};
pub use iso_lot::IsoLot;
pub use java::GameRand;
pub use map_files::{LotHeader, MapFiles, MapInfo};
pub use meta_grid::{
    CreateParams, IsoMetaCell, IsoMetaChunk, IsoMetaGrid, WorldGenBounds, ZombieVoronoi,
};
pub use zones::{
    ClipperOps, PropValue, RoomTone, Zone, ZoneError, ZoneGeometryType, ZoneId, ZoneKind,
    ZoneProps, ZoneTables,
};
