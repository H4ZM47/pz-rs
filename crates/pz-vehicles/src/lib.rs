//! Project Zomboid's vehicle runtime: `BaseVehicle` with its parts,
//! engine, alarm and lights (`zombie.vehicles`), `CarController` and
//! `WorldSimulation` (`zombie.core.physics`), the vehicle id map and the
//! single-player update loop, and `VehicleType`.
//!
//! Physics goes through [`physics::VehiclePhysics`] (the `Bullet`
//! natives); the vehicle Lua (`Vehicles.*`) is called by name through
//! [`lua`]; game state outside vehicles is read from [`env`].

// The ported arithmetic keeps the original's comparisons so NaN behaves the
// same (`!(a < b)`, hand-written clamps), and a few callback types stay
// spelled out.
#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    clippy::manual_clamp,
    clippy::type_complexity
)]

pub mod cell_link;
pub mod chunk_physics;
pub mod controller;
pub mod engine;
pub mod env;
pub mod geometry;
pub mod items;
pub mod jmath;
pub mod lua;
pub mod manager;
pub mod part;
pub mod pathfind_view;
pub mod physics;
pub mod poly;
pub mod rand;
pub mod save;
pub mod spawn;
pub mod vehicle;
pub mod vehicle_type;
pub mod world_sim;

pub use part::{PartRef, VehiclePart};
pub use physics::VehiclePhysics;
pub use vehicle::{BaseVehicle, VehicleRef};
