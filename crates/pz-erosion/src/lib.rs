//! The erosion system (`zombie.erosion`): seasons, noise, per-square
//! erosion categories (trees, bushes, plants, grass, street and wall
//! cracks, wall vines, flowerbeds) and their spawn / growth logic.
//!
//! Game-code tables are read from the installed game at runtime
//! ([`tables::ErosionTables`]); this crate contains no game data.

pub mod bits;
pub mod category;
pub mod config;
pub mod data;
pub mod erosion_main;
pub mod host;
pub mod ice_queen;
pub mod java;
pub mod noise;
pub mod obj;
pub mod rand;
pub mod regions;
pub mod season;
pub mod tables;
pub mod world_host;

pub use erosion_main::{ErosionChunkHook, ErosionMain, ErosionSettings};
pub use tables::ErosionTables;

#[cfg(test)]
mod tests;
