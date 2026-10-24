//! Bullet Physics 2.82 (double precision), translated to Rust.
//!
//! Only the parts the game's world, vehicle and constraint code reaches are
//! ported. Module layout follows Bullet's classes; names are Bullet's in
//! snake_case.

pub mod broadphase;
pub mod collision_object;
pub mod dynamics;
pub mod linear_math;
pub mod manifold;
pub mod narrowphase;
pub mod shapes;
pub mod union_find;
