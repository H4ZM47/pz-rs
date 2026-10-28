//! Corpse-count and flies algorithms over retained actual squares.
//!
//! The engine supplies current chunk/count/building queries through
//! [`flies_sound::FliesWorld`], releasing singleton and square guards before
//! sound/render hooks. Live engine activation and tracing remain unwired.
//!
//! Corpse rot, reanimation, and the corpse object registry are deliberately
//! separate follow-up work: current public `static_moving` owns serializable
//! body values and has no genuine retained native corpse owner for callbacks.
//! These routines never manufacture corpse identity from a square/index or ID.

pub mod corpse_count;
pub mod flies_sound;

#[cfg(test)]
mod count_flies_tests;
