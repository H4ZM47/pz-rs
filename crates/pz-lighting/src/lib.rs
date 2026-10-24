//! `pz-lighting`: a pure-Rust replacement for the game's native lighting
//! library (`libLighting`, the backend of `zombie.iso.LightingJNI`).

mod model;
mod results;
mod world;

pub use model::{flags, vis, Character, ResultLight, ResultLights, TestResult, Transmission, GROUND_LEVEL, NUM_LEVELS};
