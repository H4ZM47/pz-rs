//! Main-thread collision queries on the live world: `zombie.pathfind
//! .LineClearCollideMain` behind `PolygonalMap2.canStandAt`,
//! `lineClearCollide` and `getCollidepoint`, plus
//! `PolygonalMap2.getApparentZ` and `intersectLineWithVehicle`.
//!
//! The `IsoGridSquare` queries and `LosUtil.lineClear` they rest on are
//! `pz-world`'s ([`pz_world::los`], [`pz_world::square`]). What is here on
//! top: `LineClearCollideMain` itself, the [`square::WorldObjectHooks`] that
//! report `pz-world`'s thumpables, barricades, fires and door curtains to
//! those queries, and the edge-object getters as [`ObjectRef`]s.
//!
//! The arithmetic is the Java code's float arithmetic, not the native
//! library's: no fused multiply-adds, Java's `Vector2.normalize`, and the
//! square-based `canStandAtOld` / `isNotClearOld` the shipped game uses
//! (`isNotClear` and `canStandAt` forward to them). The debug-render flag
//! (8) only draws in Java; the results are the same with or without it, so
//! it is ignored.
//!
//! # Parity
//!
//! A local oracle (not in this repository) loads real B42 map chunks in a
//! headless game JVM and calls the original `PolygonalMap2.canStandAt`,
//! `lineClearCollide`, `getCollidepoint`, `getApparentZ` and
//! `LosUtil.lineClear` with random inputs; `examples/line_clear_replay.rs`
//! replays them on the same chunks loaded with `pz-world`. See that example
//! for the numbers.
//!
//! [`ObjectRef`]: crate::mover::ObjectRef

pub mod line_clear_collide;
pub mod square;

use pz_world::chunk::IsoGridSquare;
use pz_world::square::{SquareHooks, VisionEnv};
use pz_world::{GetSquare, IsoSpriteManager};

use crate::game_vehicle::VehicleSource;
use crate::square_bits::SquareBits;

pub use line_clear_collide::LineClearCollideMain;
pub use pz_world::los::{line_clear, line_clear_range, TestResults};
pub use square::WorldObjectHooks;

/// The world the main-thread queries read.
pub struct MainThreadMap<'w, 'a, G: GetSquare> {
    /// Sprites, meta grid, squares and object hooks.
    pub bits: &'w SquareBits<'a, G>,
    /// `chunk.vehicles` / `IsoCell.getVehicles()`.
    pub vehicles: &'w dyn VehicleSource,
    /// `GameServer.server` (`testVisionAdjacent`'s diagonal re-check is
    /// skipped on a server).
    pub is_server: bool,
    /// The objects `pz-world`'s square queries ask about.
    pub hooks: WorldObjectHooks<'a>,
}

impl<'w, 'a: 'w, G: GetSquare> MainThreadMap<'w, 'a, G> {
    pub fn new(bits: &'w SquareBits<'a, G>, vehicles: &'w dyn VehicleSource) -> Self {
        MainThreadMap {
            bits,
            vehicles,
            is_server: false,
            hooks: WorldObjectHooks {
                sprites: bits.sprites,
                bits_hooks: bits.hooks,
            },
        }
    }

    /// `IsoCell.getGridSquare(x, y, z)`.
    #[inline]
    pub fn square(&self, x: i32, y: i32, z: i32) -> Option<&'a IsoGridSquare> {
        self.bits.squares.get_grid_square(x, y, z)
    }

    /// The squares (`IsoWorld.instance.currentCell`).
    #[inline]
    pub fn squares(&self) -> &'a G {
        self.bits.squares
    }

    /// The sprite manager.
    #[inline]
    pub fn sprites(&self) -> &'a IsoSpriteManager {
        self.bits.sprites
    }

    /// The object hooks for `pz-world`'s square queries.
    #[inline]
    pub fn hooks(&self) -> &dyn SquareHooks {
        &self.hooks
    }

    /// What `LosUtil.lineClear` reads.
    pub fn vision_env(&self) -> VisionEnv<'_, G> {
        VisionEnv {
            squares: self.bits.squares,
            sprites: self.bits.sprites,
            hooks: &self.hooks,
            is_server: self.is_server,
        }
    }
}

/// `PZMath.fastfloor(float)`.
#[inline]
pub(crate) fn fastfloor(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 {
        i - 1
    } else {
        i
    }
}

/// `(int)Math.ceil(v)`.
#[inline]
pub(crate) fn ceil_i(v: f32) -> i32 {
    v.ceil() as i32
}

/// `zombie.iso.Vector2.normalize()` applied to `(x, y)`: unchanged when the
/// squared length is within `1e-5` of 1, zero for a zero vector.
#[inline]
pub(crate) fn normalize(x: f32, y: f32) -> (f32, f32) {
    let len_sq = x * x + y * y;
    if (1.0f32 - len_sq).abs() < 1.0e-5f32 {
        return (x, y);
    }
    if len_sq == 0.0 {
        return (0.0, 0.0);
    }
    let len = (len_sq as f64).sqrt() as f32;
    (x / len, y / len)
}
