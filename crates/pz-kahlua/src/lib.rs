//! Pure-Rust port of Kahlua2, the Lua 5.1 implementation Project Zomboid
//! embeds. Semantics follow the game's fork exactly (insertion-ordered
//! tables, metatable-aware `rawget`, truncating `%`, Java number formatting,
//! UTF-16 string lengths) so existing game scripts and mods behave the same.

pub mod compiler;
pub mod exec;
pub mod java_io;
pub mod number;
pub mod stdlib;
pub mod table;
pub mod table_save;
pub mod value;
pub mod vm;

pub use table::TableRef;
pub use value::{LStr, LuaObject, Value};
pub use vm::{LuaError, LuaResult, NativeCall, NativeFunction, Thread};
