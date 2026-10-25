//! `zombie.worldMap`: the world map (the in-game map, the mini-map, the
//! spawn-selection map and printed maps), engine side.
//!
//! The map UI element (`UIWorldMap`) and its Lua API objects live in
//! pz-lua-api; this crate holds the engine-independent parts.

pub mod binary;
pub mod data;
pub mod draw;
pub mod projection;
pub mod rasterize;
pub mod renderer;
pub mod style;
pub mod triangulate;
pub mod vbo;
pub mod world_map;
pub mod xml;

pub use data::WorldMapData;
pub use renderer::WorldMapRenderer;
pub use world_map::WorldMap;
#[cfg(test)]
mod data_tests;
#[cfg(test)]
mod draw_tests;
