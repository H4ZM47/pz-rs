//! The zombie population: `zombie.popman` ([`ZombiePopulationManager`],
//! `LoadedAreas`, `PlayerSpawns`, `RealZombieMap`, `ZombieStateFlags`,
//! `ObjectPool`), the Java side of `zombie.MapCollisionData`, and a
//! pure-Rust port of the `PZPopMan` native library they drive ([`native`]).
//!
//! The population's save files (`zpop/zpop_X_Y.bin`, `zpop_virtual.bin`,
//! `zpop_ids.bin`) and the collision cache (`chunkdata/chunkdata_X_Y.bin`)
//! are read and written by [`native`], as the original library does. Real
//! zombies are created and inspected through [`PopulationHost`].

pub mod manager;
pub mod map_collision;
pub mod native;
pub mod object_pool;
pub mod state_flags;

pub use manager::{PopulationHost, ZombiePopulationManager};
