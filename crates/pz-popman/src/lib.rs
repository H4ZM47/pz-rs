//! The zombie population: `zombie.popman` (`ZombiePopulationManager`,
//! `LoadedAreas`, `PlayerSpawns`, `RealZombieMap`, `ZombieStateFlags`, ...),
//! `zombie.MapCollisionData`, the parts of `zombie.VirtualZombieManager`
//! that do not need a live `IsoZombie`, and a pure-Rust port of the
//! `PZPopMan` native library they drive ([`native`]).
//!
//! The population's save files (`zpop/zpop_X_Y.bin`, `zpop_virtual.bin`,
//! `zpop_ids.bin`) and the collision cache (`chunkdata/chunkdata_X_Y.bin`)
//! are read and written by [`native`], as the original library does.

pub mod map_collision;
pub mod native;
