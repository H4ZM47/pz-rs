//! Corpse-count and flies algorithms over retained actual squares.
//!
//! The engine supplies current chunk/count/building queries through
//! [`flies_sound::FliesWorld`], releasing singleton and square guards before
//! sound/render hooks. Live engine activation and tracing remain unwired.
//!
//! Corpse rot, reanimation, and the corpse object registry are deliberately
//! separate follow-up work. Each corpse in `static_moving` is one shared
//! object ([`crate::save::visual::DeadBodyRef`]), the owner that Lua and the
//! engine hold. These routines never manufacture corpse identity from a
//! square/index or ID.

pub mod corpse_count;
pub mod inventory;
pub mod flies_sound;
pub mod update;

#[cfg(test)]
mod count_flies_tests;
