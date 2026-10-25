//! Narrowphase collision detection: GJK, EPA, convex casts and the box-box
//! detector (Bullet 2.82 `NarrowPhaseCollision` plus `btBoxBoxDetector`).
//!
//! Loops and comparisons follow the C++ statement by statement (index loops,
//! negated comparisons that must stay false for NaN), hence the lint allows.

#[allow(clippy::needless_range_loop, clippy::type_complexity)]
pub mod box_box;
#[allow(clippy::too_many_arguments, clippy::neg_cmp_op_on_partial_ord)]
pub mod convex_cast;
#[allow(clippy::needless_range_loop)]
pub mod gjk_epa;
#[allow(clippy::collapsible_if, clippy::collapsible_else_if)]
pub mod gjk_pair_detector;
#[allow(clippy::needless_range_loop)]
pub mod simplex;
