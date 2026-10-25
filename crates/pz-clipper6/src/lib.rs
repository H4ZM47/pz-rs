//! `pz-clipper6`: a pure-Rust port of the polygon code statically linked
//! into the game's native pathfinding library (`libPZPathFind`), so the
//! Rust pathfinder reproduces its obstacle polygons exactly.
//!
//! The library is built from:
//!
//! * Angus Johnson's **Clipper 6.4.2** (`ClipperLib`: `Clipper`,
//!   `ClipperOffset`, `PolyTree`), configured with 64-bit coordinates and
//!   `use_lines` — [`clipper`]; Boost Software License 1.0, see
//!   `LICENSE-BOOST`.
//! * mapbox **earcut.hpp** (a57e4ce, earcut 2.2.1 parity) instantiated over
//!   `ClipperLib::IntPoint` — [`earcut`]; ISC License, see `LICENSE-ISC`.
//! * The game's own `PZClipper` wrapper — [`pz`], original code.
//!
//! `std::sort` from libc++ is reproduced in `cxxsort` because Clipper and
//! earcut sort with comparators that leave ties, and tie order changes the
//! output.
//!
//! Floating point reproduces the arm64 build bit for bit, including clang's
//! FMA contraction (see [`clipper`]).

pub mod clipper;
mod cxxsort;
pub mod earcut;
pub mod pz;

pub use crate::clipper::{
    ClipType, Clipper, ClipperException, ClipperOffset, EndType, IntPoint, JoinType, Path, Paths,
    PolyFillType, PolyNode, PolyNodeId, PolyTree, PolyType,
};
pub use crate::pz::{PZClipper, PZPoly, Triangle2f};
