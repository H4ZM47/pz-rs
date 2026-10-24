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
//! | `addVehicle` / `teleportVehicle` / `removeVehicle` | [`World::add_vehicle`], [`World::teleport_vehicle`], [`World::remove_vehicle`] (typed: `*_typed` with [`Vehicle`]) |
//! | `update` | [`World::update`] |
//! | `findPath` | [`World::find_path_bytes`], [`World::find_path_into`], [`World::find_path`] |
//!
//! Levels carry a +32 offset everywhere inside the pathfinder (ground
//! floor = 32), as in the native code. Results are [`PathNode`]s in that
//! representation; [`PathNode::level`] converts back.
//!
//! # Where the native library differs from the Java `zombie.pathfind`
//!
//! The port follows the arm64 build of the native library where the two
//! disagree. The differences that change results:
//!
//! * The A* open list is a binary heap keyed on `f = g + h` rounded to
//!   1/1000 (`floor((g + h) * 1000 + 0.5) / 1000`, fused), closed nodes
//!   are reopened when a cheaper key turns up, and there is no step
//!   limit; `shortestPath` gives up beyond 5000 nodes.
//! * Distances and interpolations use fused multiply-adds; `lerp(a, b, t)`
//!   is `fma(a - b, t, b)`.
//! * `canNotMoveBetween` and the search-node costs also handle thumpable
//!   squares, target-only paths, bendable and tall-hoppable fences,
//!   farming plants (avoided by players that ask for it) and a +10 cost
//!   for squares under vehicles. For zombies following a target-only path
//!   the native code tests the north flag of the first square for both
//!   vertical directions; that quirk is kept.
//! * `findPath` checks start and goal squares differently, always
//!   smooths player paths and never runs `fixPathZ`; a goal square in an
//!   unresolved vehicle graph is moved out of obstacles first.
//! * `LineClearCollide` sweeps vehicles at `RADIUS` (0.3).
//! * Visibility-graph obstacles come from the clipper union of the
//!   squares' blocking boxes (no bevels); polygon points are taken in
//!   reverse order, and can-path connections also carry the target-path
//!   and bendable flags.
//! * Chunk updates take effect at once and `update()` does nothing; a
//!   chunk keeps the load id of its first update until it is removed.
//!
//! # Parity status
//!
//! `examples/oracle_replay.rs` replays recorded runs of the original
//! library (directory passed on the command line; no recorded data is
//! kept in this repository). On 6 map regions (town, dense downtown with
//! basements, high-rises up to level 13, open forest, and a synthetic
//! sloped-ramp region) all 6300 requests, 4136 of them with a path, match
//! the native output bit for bit: return code, node count, raw float bits
//! of every node and node flags.
//!
//! Not covered by that data: vehicles and the visibility graphs built
//! around them, farming plants, bendable fences and open doors or
//! windows. Those paths are ported from the disassembly and checked by the
//! behavioural tests in `tests/synthetic.rs` only.
//!
//! # The Java layer around it
//!
//! * [`mover`]: what `PathFindBehavior2`, `PathFindRequest.init`,
//!   `PathRequestTask` and `PathfindNative.findPath` read and call on the
//!   moving character ([`mover::PathMover`]), implemented by the character
//!   port.
//! * [`service`]: `PathfindNative` and `PathfindNativeThread` (task and
//!   request queues, priorities, cancel, delivery), single-threaded.
//! * [`main_thread`]: `LineClearCollideMain` (`PolygonalMap2.canStandAt`,
//!   `lineClearCollide`, ...) and `LosUtil.lineClear` on live `pz-world`
//!   squares.
//! * [`game_vehicle`]: what that code reads from `BaseVehicle`.

// Loops index squares and nodes by position to keep the native iteration
// order visible.
#![allow(clippy::needless_range_loop)]

pub mod astar;
pub mod bytes;
mod clip;
pub mod game_vehicle;
pub mod high_level;
pub mod line_clear_collide;
pub mod main_thread;
pub mod mover;
pub mod path;
pub mod polygonal_map2;
pub mod pool;
pub mod request;
pub mod service;
pub mod square;
pub mod square_bits;
pub mod util;
pub mod vgastar;
pub mod visibility;
pub mod world;

pub use bytes::PathfindError;
pub use path::{Path, PathNode};
pub use request::{MoverKind, PathFindRequest, REQUEST_SIZE};
pub use visibility::{Vehicle, VehiclePoly};
pub use world::World;
