//! `zombie.core.Core` (global options and display state) and the
//! `zombie.config` option types. Kept below the Lua API layer so both the
//! bindings and the game loop can use it.

pub mod config;
pub mod core;
pub mod random;
