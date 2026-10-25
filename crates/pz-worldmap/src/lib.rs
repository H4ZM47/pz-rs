//! `zombie.worldMap`: the world map (the in-game map, the mini-map, the
//! spawn-selection map and printed maps), engine side.
//!
//! The map UI element (`UIWorldMap`) and its Lua API objects live in
//! pz-lua-api; this crate holds the engine-independent parts.

pub mod data;
pub mod projection;
pub mod renderer;
pub mod world_map;

pub use data::WorldMapData;
pub use renderer::WorldMapRenderer;
pub use world_map::WorldMap;
