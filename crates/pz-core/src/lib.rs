//! `zombie.core.Core` (global options and display state) and the
//! `zombie.config` option types, plus small engine data tables the Lua API
//! fills at boot (`map_symbols`). Kept below the Lua API layer so both the
//! bindings and the game loop can use it.

pub mod calendar;
pub mod config;
pub mod core;
pub mod debug;
pub mod random;
pub mod weather;
pub mod map_symbols;
pub mod game_time;
pub mod game_time_save;
pub mod save_trackers;
pub mod thumbnail;
