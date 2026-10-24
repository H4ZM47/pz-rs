//! `zombie.core.Core` (global options and display state) and the
//! `zombie.config` option types, plus small engine data tables the Lua API
//! fills at boot (`map_symbols`). Kept below the Lua API layer so both the
//! bindings and the game loop can use it.

pub mod config;
pub mod core;
pub mod debug;
pub mod map_symbols;
