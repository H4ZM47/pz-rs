//! Port of Project Zomboid's native physics library (`libPZBullet`):
//! Bullet Physics 2.82 in double precision plus the game's glue, without
//! any native code at runtime.
//!
//! [`PzBullet`] mirrors the JNI surface of `zombie.core.physics.Bullet`.

pub mod api;
pub mod bt;
pub mod pz;

pub use api::{JavaCallbacks, JniError, NoCallbacks, PzBullet};
