//! `pz-clipper`: a pure-Rust replacement for the game's `PZClipper` native
//! library, the backend of `zombie.vehicles.Clipper` and
//! `zombie.vehicles.ClipperOffset`.
//!
//! The native library is built from three parts, all ported here so that
//! results are bit-identical to the shipped arm64 binary:
//!
//! * Angus Johnson's **Clipper2 1.5.4** (`Clipper64` boolean operations,
//!   `PolyTree64`, `ClipperOffset`) — [`engine`], [`offset`], [`core`];
//!   Boost Software License 1.0, see `LICENSE-BOOST`.
//! * mapbox **earcut.hpp** at commit a57e4ce (earcut 2.2.1 parity; polygon
//!   triangulation) — [`earcut`];
//!   ISC License, see `LICENSE-ISC`.
//! * The game's own JNI glue (float/fixed-point conversion, polygon
//!   bookkeeping, buffer layouts) — [`pz`], original code.
//!
//! `std::sort` from libc++ is reproduced in `cxxsort` because Clipper2 and
//! earcut sort with comparators that leave ties, and tie order changes the
//! output.

pub mod core;
mod cxxsort;
pub mod earcut;
pub mod engine;
pub mod offset;
pub mod pz;

pub use crate::core::{FillRule, Path64, Paths64, Point64, PointD, Rect64};
pub use crate::engine::{ClipType, Clipper64, PathType, PolyPath64};
pub use crate::offset::{EndType, JoinType};
pub use crate::pz::{Clipper, ClipperError, ClipperOffset, Polygon, ZoneClipper};
