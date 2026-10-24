//! Kahlua standard library (BaseLib, StringLib, TableLib, MathLib, OsLib,
//! CoroutineLib, RandomLib) and the game's `stdlib.lua` equivalents.

use crate::vm::Thread;

/// Install the Kahlua standard library into `thread`'s globals.
pub fn install(_thread: &mut Thread) {}
