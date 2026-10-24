//! Port of Project Zomboid build 42's native pathfinder
//! (`libPZPathFind`, the C++ counterpart of `zombie.pathfind`).
//!
//! The entry points mirror the JNI functions of
//! `zombie.pathfind.nativeCode.PathfindNative` and take the same byte
//! buffers, so recorded native inputs can be replayed:
//!
//! | JNI | here |
//! |---|---|
//! | `initWorld` | [`World::new`] |
//! | `updateChunk` | [`World::update_chunk`] |
//! | `removeChunk` | [`World::remove_chunk`] |
//! | `updateSquare` | [`World::update_square`] |
//! | `addVehicle` / `teleportVehicle` / `removeVehicle` | [`World::add_vehicle`] etc. |
//! | `update` | [`World::update`] |
//! | `findPath` | [`World::find_path_bytes`], [`World::find_path`] |
//!
//! Levels carry a +32 offset everywhere inside the pathfinder (ground
//! floor = 32), as in the native code. Results are [`PathNode`]s in that
//! representation; [`PathNode::level`] converts back.

pub mod astar;
pub mod bytes;
mod clip;
pub mod high_level;
pub mod line_clear_collide;
pub mod path;
pub mod polygonal_map2;
pub mod pool;
pub mod request;
pub mod square;
pub mod util;
pub mod vgastar;
pub mod visibility;
pub mod world;

pub use bytes::PathfindError;
pub use path::{Path, PathNode};
pub use request::{MoverKind, PathFindRequest, REQUEST_SIZE};
pub use world::World;
