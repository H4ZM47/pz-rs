//! Maps and world: lot files (`zombie.iso.LotHeader`, `IsoLot`, `MapFiles`),
//! the meta grid (`IsoMetaGrid`, `RoomDef`, `BuildingDef`), tile definitions
//! and chunk loading (`IsoCell`, `IsoChunk`, `IsoGridSquare`).
//!
//! Map data is read from the installed game's `media/maps` at runtime; this
//! crate contains no game data.

pub mod animal_zone_save;
pub mod bra;
pub mod cell_loader;
pub mod cell_drag;
pub mod chunk;
pub mod chunk_data;
pub mod chunk_vehicles;
pub mod defs;
pub mod erosion_data;
pub mod fire;
pub mod fix2x;
pub mod floor_item;
pub mod iso_lot;
pub mod isoregion;
pub mod java;
pub mod los;
pub mod map_files;
pub mod map_groups;
pub mod meta_grid;
pub mod meta_save;
pub mod meta_zones;
pub mod nav_streets;
pub mod objects;
pub mod process_items;
pub mod save;
pub mod sprite;
pub mod square;
pub mod square_mutation;
pub mod streamer;
pub mod tile_overlays;
pub mod tiledef;
pub mod world_state;
pub mod zone_save;
pub mod zones;

pub use chunk::{
    ChunkGenerationStatus, ChunkGenerationStatusSet, ChunkLoadEnv, IsoCell, IsoChunk, IsoGridSquare,
};
pub use defs::{BuildingDef, BuildingIdx, MetaObject, RoomDef, RoomIdx, RoomRect};
pub use iso_lot::IsoLot;
pub use los::TestResults;
pub use java::{FactoryRand, GameRand};
pub use map_files::{LotHeader, MapFiles, MapInfo};
pub use meta_grid::{
    CreateParams, IsoMetaCell, IsoMetaChunk, IsoMetaGrid, WorldGenBounds, ZombieVoronoi,
};
pub use objects::{
    IsoObject, NoHooks, ObjectKind, ObjectLoadHooks, ObjectLoadSettings, ObjectTables,
};
pub use sprite::{IsoSpriteManager, SpriteTables};
pub use zones::{
    ClipperOps, PropValue, RoomTone, VehicleZone, Zone, ZoneError, ZoneGeometryType, ZoneId, ZoneKind,
    ZoneProps, ZoneTables,
};
pub use square::{
    ChunkGetter, EdgeFacing, GetSquare, GridSquareEdge, IsoDirections, NoSquareHooks,
    SpecialObject, SquareHooks, ThumpableInfo, VisionEnv, VisionResult,
};
pub use world_state::{ObjectHooks, PowerSettings, WorldState};
