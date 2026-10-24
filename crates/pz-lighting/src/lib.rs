//! `pz-lighting`: a pure-Rust replacement for the game's native lighting
//! library (`libLighting`, the backend of `zombie.iso.LightingJNI`).
//!
//! The library keeps one lighting world per local player. The Java side
//! streams squares in chunk by chunk (vision flags, room ids, transmissions
//! through curtains, doors and windows), registers lights, room lights and
//! torches, and once per frame sends the player and the frame parameters.
//! An update then
//!
//! 1. computes visibility with a cached line-of-sight walk from the
//!    player's square (`seen`, `canSee`, `couldSee` and the target
//!    darkness),
//! 2. floods sunlight from the open-air columns (-16 per square, -32 across
//!    levels, scaled through closed edges by their transmission) and the
//!    point and room lights into per-square light lists of up to five
//!    entries,
//! 3. adds torch light on the player's level,
//! 4. turns all of that into a per-square colour, a darkness multiplier and
//!    eight vertex colours (four floor corners, four wall tops).
//!
//! [`LightingSession`] mirrors the native method table one-to-one; see its
//! docs for the hand-off model. [`SquareLighting`] decodes the 44-int block
//! `getSquareLighting` produces.
//!
//! # Example: a noon frame without a player
//!
//! ```
//! use pz_lighting::{Character, LightingSession, SquareLighting, Transmission};
//!
//! let sky = 0x00_e6_e6_e6; // GameTime.getSkyLightLevel(): b | g << 8 | r << 16
//! let frame = |s: &mut LightingSession, first: bool| {
//!     s.state_begin_update(0, 10, 10, 1, 1).unwrap(); // chunk (10, 10), 1x1 chunks
//!     s.player_set(Character { see_all: true, ..Character::default() }).unwrap();
//!     s.state_end_frame(1.0, 1.0, 1.0, 1.0, 0.0, 30.0, 40.0, first, 100.0, sky).unwrap();
//!     if first {
//!         s.chunk_begin_update(10, 10, 32, 32).unwrap();
//!         s.chunk_level_begin_update(32).unwrap();
//!         for y in 0..8 {
//!             for x in 0..8 {
//!                 s.square_begin_update(x, y, 32).unwrap();
//!                 // All eight directions open, sky above, outdoors.
//!                 s.square_set(0xff, true, false, false, 0, -1, -1, sky, true).unwrap();
//!                 s.square_set_light_transmission([Transmission::default(); 4]).unwrap();
//!                 s.square_end_update().unwrap();
//!             }
//!         }
//!         s.chunk_level_end_update().unwrap();
//!         s.chunk_end_update().unwrap();
//!     }
//!     s.state_end_update().unwrap();
//! };
//! let mut s = LightingSession::new();
//! frame(&mut s, true); // hands the world over
//! s.do_lighting_update(0, true); // computes
//! frame(&mut s, false); // swaps the results in
//! let l = SquareLighting(s.get_square_lighting(0, 83, 84, 32).unwrap().unwrap());
//! assert_eq!(l.dark_multi_raw(), 100000);
//! assert_eq!(l.light_level(), sky as u32);
//! ```
//!
//! # Exactness
//!
//! The port follows the arm64 build of the library: `f32` throughout, fused
//! multiply-adds exactly where the binary has them (`DistanceSquared`, the
//! distance and direction maths of `CalcVisibility` and
//! `CalculateTorchRGB`), truncating conversions, and the one `f64`
//! multiply that turns the darkness values into the integers of
//! `getSquareLighting`. Iteration orders (the visibility pass, which fills
//! the line-of-sight cache, the flood queues, and the in-place vertex
//! blending) follow the native loops.
//!
//! One order cannot be reproduced: the native code collects the sky seeds
//! of the sunlight flood in a `std::unordered_set` keyed by heap address
//! and floods them in hash order. The flood replaces all three channels of
//! a square when any channel improves, so with coloured transmissions (or
//! sky colours whose channels fall off differently through a mix of open
//! and closed edges) the result can depend on that order. The port floods
//! the seeds in chunk/column order; wherever the native result does not
//! depend on the order, the two agree.
//!
//! # Known differences
//!
//! * In "see everything" mode ([`Character::see_all`]) the native code never
//!   writes the target darkness and reads uninitialised memory instead. The
//!   port writes 1 (fully visible), so a frame rendered without a player has
//!   `darkMulti = 1` and colours straight from sun, sky and lights.
//! * Arrays the native code leaves uninitialised (colours, vertex colours,
//!   targets of squares the update never reaches, e.g. when the player
//!   stands outside every square) read as zero here.
//! * Sky seeds are flooded in chunk/column order (see above).

// Many comparisons are written negated (`!(a <= b)`) on purpose: they mirror
// the binary's branch conditions, which differ from the plain form for NaN.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

mod color;
mod lights;
mod model;
mod results;
mod session;
mod state;
mod sun;
mod torch;
mod vision;
mod world;

pub use model::{flags, vis, Character, ResultLight, ResultLights, TestResult, Transmission, GROUND_LEVEL, NUM_LEVELS};
pub use session::{LightingError, LightingSession, SQUARE_LIGHTING_LEN};

/// A decoded `getSquareLighting` block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SquareLighting(pub [i32; SQUARE_LIGHTING_LEN]);

impl SquareLighting {
    /// Visibility bits, see [`vis`].
    pub fn vis(&self) -> u8 {
        self.0[0] as u8
    }

    /// The square's light colour bytes (r, g, b).
    pub fn light_info(&self) -> [u8; 3] {
        let v = self.0[1] as u32;
        [v as u8, (v >> 8) as u8, (v >> 16) as u8]
    }

    /// `darkMulti * 100000`, truncated.
    pub fn dark_multi_raw(&self) -> i32 {
        self.0[2]
    }

    /// `targetDarkMulti * 100000`, truncated.
    pub fn target_dark_multi_raw(&self) -> i32 {
        self.0[3]
    }

    /// The packed propagated light level (sun RGB in bytes 0..2).
    pub fn light_level(&self) -> u32 {
        self.0[4] as u32
    }

    /// Vertex colour `0xFFbbggrr`: 0-3 floor corners, 4-7 the same corners
    /// one level up.
    pub fn vertex(&self, i: usize) -> u32 {
        self.0[5 + i] as u32
    }

    /// Number of lights (at most five).
    pub fn light_count(&self) -> usize {
        self.0[13].clamp(0, 5) as usize
    }

    /// Light `i`: id, x, y, z, radius, `r | g << 8 | b << 16 | flags << 24`.
    pub fn light(&self, i: usize) -> [i32; 6] {
        let b = 14 + 6 * i;
        [self.0[b], self.0[b + 1], self.0[b + 2], self.0[b + 3], self.0[b + 4], self.0[b + 5]]
    }
}
