// Bullet Continuous Collision Detection and Physics Library
// Copyright (c) 2003-2009 Erwin Coumans  http://bulletphysics.org
//
// This software is provided 'as-is', without any express or implied warranty.
// In no event will the authors be held liable for any damages arising from the use of this software.
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it freely,
// subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Altered for pz-rs: Rust translation of btCollisionShape, btConvexShape,
// btConvexInternalShape, btPolyhedralConvexShape (and the AABB caching
// variants), btBoxShape, btSphereShape, btCapsuleShape (with
// btCapsuleShapeX/Z), btConvexHullShape and btCompoundShape from Bullet
// 2.82, double precision.

//! Collision shapes.
//!
//! The game only creates boxes, spheres, capsules, convex hulls and compounds
//! of those, so [`ShapeKind`] has exactly these variants; Bullet's virtual dispatch
//! becomes a `match`. Shapes live in a [`Shapes`] arena and refer to each
//! other (compound children) by [`ShapeId`]. Methods are named after the
//! Bullet member they translate; `*_non_virtual` are the `btConvexShape`
//! switch-based variants, the others follow the virtual overrides.

use super::broadphase::dbvt::{Dbvt, DbvtAabbMm, DbvtNodeId};
use super::collision_object::ShapeId;
use super::linear_math::*;

// btBroadphaseProxy.h BroadphaseNativeTypes (the ones that matter here).
pub const BOX_SHAPE_PROXYTYPE: i32 = 0;
pub const TRIANGLE_SHAPE_PROXYTYPE: i32 = 1;
pub const CONVEX_HULL_SHAPE_PROXYTYPE: i32 = 4;
pub const CONVEX_POINT_CLOUD_SHAPE_PROXYTYPE: i32 = 5;
pub const IMPLICIT_CONVEX_SHAPES_START_HERE: i32 = 7;
pub const SPHERE_SHAPE_PROXYTYPE: i32 = 8;
pub const CAPSULE_SHAPE_PROXYTYPE: i32 = 10;
pub const CONE_SHAPE_PROXYTYPE: i32 = 11;
pub const CYLINDER_SHAPE_PROXYTYPE: i32 = 13;
pub const BOX_2D_SHAPE_PROXYTYPE: i32 = 17;
pub const CONVEX_2D_SHAPE_PROXYTYPE: i32 = 18;
pub const CONCAVE_SHAPES_START_HERE: i32 = 20;
pub const TRIANGLE_MESH_SHAPE_PROXYTYPE: i32 = 21;
pub const STATIC_PLANE_PROXYTYPE: i32 = 28;
pub const CONCAVE_SHAPES_END_HERE: i32 = 30;
pub const COMPOUND_SHAPE_PROXYTYPE: i32 = 31;
pub const INVALID_SHAPE_PROXYTYPE: i32 = 35;
pub const MAX_BROADPHASE_COLLISION_TYPES: i32 = 36;

/// `CONVEX_DISTANCE_MARGIN`.
pub const CONVEX_DISTANCE_MARGIN: f64 = 0.04;

/// `btBroadphaseProxy::isPolyhedral`.
#[inline(always)]
pub fn is_polyhedral(t: i32) -> bool {
    t < IMPLICIT_CONVEX_SHAPES_START_HERE
}
/// `btBroadphaseProxy::isConvex`.
#[inline(always)]
pub fn is_convex(t: i32) -> bool {
    t < CONCAVE_SHAPES_START_HERE
}
/// `btBroadphaseProxy::isNonMoving`.
#[inline(always)]
pub fn is_non_moving(t: i32) -> bool {
    is_concave(t) && t != 25
}
/// `btBroadphaseProxy::isConcave`.
#[inline(always)]
pub fn is_concave(t: i32) -> bool {
    t > CONCAVE_SHAPES_START_HERE && t < CONCAVE_SHAPES_END_HERE
}
/// `btBroadphaseProxy::isCompound`.
#[inline(always)]
pub fn is_compound(t: i32) -> bool {
    t == COMPOUND_SHAPE_PROXYTYPE
}
/// `btBroadphaseProxy::isConvex2d`.
#[inline(always)]
pub fn is_convex2d(t: i32) -> bool {
    t == BOX_2D_SHAPE_PROXYTYPE || t == CONVEX_2D_SHAPE_PROXYTYPE
}
/// `btBroadphaseProxy::isInfinite`.
#[inline(always)]
pub fn is_infinite(t: i32) -> bool {
    t == STATIC_PLANE_PROXYTYPE
}

/// `btFsel(a, b, c)`: `a >= 0 ? b : c`.
#[inline(always)]
fn fsel(a: f64, b: f64, c: f64) -> f64 {
    if a >= 0.0 {
        b
    } else {
        c
    }
}

/// The `btConvexInternalShape` members.
#[derive(Clone, Copy, Debug)]
pub struct ConvexInternal {
    pub local_scaling: Vec3,
    pub implicit_shape_dimensions: Vec3,
    pub collision_margin: f64,
    pub padding: f64,
}

impl ConvexInternal {
    /// `btConvexInternalShape()`. `m_implicitShapeDimensions` is left
    /// uninitialised by Bullet; every concrete shape sets it.
    fn new() -> ConvexInternal {
        ConvexInternal {
            local_scaling: Vec3::new(1.0, 1.0, 1.0),
            implicit_shape_dimensions: Vec3::ZERO,
            collision_margin: CONVEX_DISTANCE_MARGIN,
            padding: 0.0,
        }
    }
}

/// `btBoxShape`.
#[derive(Clone, Debug)]
pub struct BoxShape {
    pub ci: ConvexInternal,
}

/// `btSphereShape`.
#[derive(Clone, Debug)]
pub struct SphereShape {
    pub ci: ConvexInternal,
}

/// `btCapsuleShape` (`up_axis` 1), `btCapsuleShapeX` (0) or
/// `btCapsuleShapeZ` (2). The game's `PZCapsuleShape` derives from
/// `btCapsuleShape` without overriding anything, so it is a plain Y capsule.
#[derive(Clone, Debug)]
pub struct CapsuleShape {
    pub ci: ConvexInternal,
    pub up_axis: usize,
}

impl CapsuleShape {
    /// `getRadius()`: `m_implicitShapeDimensions[(m_upAxis + 2) % 3]`.
    #[inline(always)]
    pub fn get_radius(&self) -> f64 {
        self.ci.implicit_shape_dimensions[(self.up_axis + 2) % 3]
    }
    /// `getHalfHeight()`.
    #[inline(always)]
    pub fn get_half_height(&self) -> f64 {
        self.ci.implicit_shape_dimensions[self.up_axis]
    }
    /// `localGetSupportingVertexWithoutMargin`; the non-virtual
    /// `btConvexShape` capsule case is the same code.
    fn support_without_margin(&self, vec0: &Vec3) -> Vec3 {
        let mut sup_vec = Vec3::ZERO;
        let mut max_dot = -BT_LARGE_FLOAT;
        let mut vec = *vec0;
        let len_sqr = vec.length2();
        if len_sqr < 0.0001 {
            vec.set_value(1.0, 0.0, 0.0);
        } else {
            let rlen = 1.0 / len_sqr.sqrt();
            vec *= rlen;
        }
        let radius = self.get_radius();
        let margin = self.ci.collision_margin;
        for top in [true, false] {
            let mut pos = Vec3::ZERO;
            let hh = self.get_half_height();
            pos[self.up_axis] = if top { hh } else { -hh };
            let vtx = pos + vec * radius - vec * margin;
            let new_dot = vec.dot(&vtx);
            if new_dot > max_dot {
                max_dot = new_dot;
                sup_vec = vtx;
            }
        }
        Vec3::new(sup_vec.x, sup_vec.y, sup_vec.z)
    }
    /// `batchedUnitVectorGetSupportingVertexWithoutMargin`.
    fn batched(&self, vectors: &[Vec3], out: &mut [Vec3]) {
        let radius = self.get_radius();
        let margin = self.ci.collision_margin;
        for (o, vec) in out.iter_mut().zip(vectors) {
            let mut max_dot = -BT_LARGE_FLOAT;
            for top in [true, false] {
                let mut pos = Vec3::ZERO;
                let hh = self.get_half_height();
                pos[self.up_axis] = if top { hh } else { -hh };
                let vtx = pos + *vec * radius - *vec * margin;
                let new_dot = vec.dot(&vtx);
                if new_dot > max_dot {
                    max_dot = new_dot;
                    *o = vtx;
                }
            }
        }
    }
    /// `getAabb` (and the `getAabbNonVirtual` capsule case).
    fn get_aabb(&self, t: &Transform) -> (Vec3, Vec3) {
        let r = self.get_radius();
        let mut half_extents = Vec3::new(r, r, r);
        half_extents[self.up_axis] = r + self.get_half_height();
        let m = self.ci.collision_margin;
        half_extents += Vec3::new(m, m, m);
        let abs_b = t.basis.absolute();
        let center = t.origin;
        let extent = half_extents.dot3(&abs_b.el[0], &abs_b.el[1], &abs_b.el[2]);
        (center - extent, center + extent)
    }
}

/// `btConvexHullShape` (a `btPolyhedralConvexAabbCachingShape`).
#[derive(Clone, Debug)]
pub struct ConvexHullShape {
    pub ci: ConvexInternal,
    pub local_aabb_min: Vec3,
    pub local_aabb_max: Vec3,
    pub is_local_aabb_valid: bool,
    pub unscaled_points: Vec<Vec3>,
}

/// `btCompoundShapeChild`.
#[derive(Clone, Debug)]
pub struct CompoundShapeChild {
    pub transform: Transform,
    pub child_shape: ShapeId,
    pub child_shape_type: i32,
    pub child_margin: f64,
    pub node: Option<DbvtNodeId>,
}

/// `btCompoundShape`.
#[derive(Clone, Debug)]
pub struct CompoundShape {
    pub children: Vec<CompoundShapeChild>,
    pub local_aabb_min: Vec3,
    pub local_aabb_max: Vec3,
    pub dynamic_aabb_tree: Option<Dbvt>,
    pub update_revision: i32,
    pub collision_margin: f64,
    pub local_scaling: Vec3,
}

impl CompoundShape {
    /// `btCompoundShape(enableDynamicAabbTree)`.
    pub fn new(enable_dynamic_aabb_tree: bool) -> CompoundShape {
        CompoundShape {
            children: Vec::new(),
            local_aabb_min: Vec3::new(BT_LARGE_FLOAT, BT_LARGE_FLOAT, BT_LARGE_FLOAT),
            local_aabb_max: Vec3::new(-BT_LARGE_FLOAT, -BT_LARGE_FLOAT, -BT_LARGE_FLOAT),
            dynamic_aabb_tree: if enable_dynamic_aabb_tree {
                Some(Dbvt::new())
            } else {
                None
            },
            update_revision: 1,
            collision_margin: 0.0,
            local_scaling: Vec3::new(1.0, 1.0, 1.0),
        }
    }
    #[inline(always)]
    pub fn get_num_child_shapes(&self) -> usize {
        self.children.len()
    }
    #[inline(always)]
    pub fn get_child_shape(&self, i: usize) -> ShapeId {
        self.children[i].child_shape
    }
    #[inline(always)]
    pub fn get_child_transform(&self, i: usize) -> &Transform {
        &self.children[i].transform
    }
}

/// The concrete shape.
#[derive(Clone, Debug)]
pub enum ShapeKind {
    Box(BoxShape),
    Sphere(SphereShape),
    Capsule(CapsuleShape),
    ConvexHull(ConvexHullShape),
    Compound(CompoundShape),
}

/// A `btCollisionShape`.
#[derive(Clone, Debug)]
pub struct Shape {
    pub kind: ShapeKind,
}

const DIRECTIONS6: [Vec3; 6] = [
    Vec3::new(1.0, 0.0, 0.0),
    Vec3::new(0.0, 1.0, 0.0),
    Vec3::new(0.0, 0.0, 1.0),
    Vec3::new(-1.0, 0.0, 0.0),
    Vec3::new(0.0, -1.0, 0.0),
    Vec3::new(0.0, 0.0, -1.0),
];

impl Shape {
    /// `btBoxShape(boxHalfExtents)`.
    pub fn new_box(box_half_extents: &Vec3) -> Shape {
        let mut ci = ConvexInternal::new();
        // setSafeMargin(boxHalfExtents): 0.1f * min dimension.
        let min_dimension = box_half_extents[box_half_extents.min_axis()];
        let safe_margin = flt(0.1) * min_dimension;
        if safe_margin < ci.collision_margin {
            ci.collision_margin = safe_margin;
        }
        let m = ci.collision_margin;
        let margin = Vec3::new(m, m, m);
        ci.implicit_shape_dimensions = (*box_half_extents * ci.local_scaling) - margin;
        Shape {
            kind: ShapeKind::Box(BoxShape { ci }),
        }
    }
    /// `btSphereShape(radius)`.
    pub fn new_sphere(radius: f64) -> Shape {
        let mut ci = ConvexInternal::new();
        ci.implicit_shape_dimensions.x = radius;
        ci.collision_margin = radius;
        Shape {
            kind: ShapeKind::Sphere(SphereShape { ci }),
        }
    }
    /// `btCapsuleShape(radius, height)` around `up_axis` (1 for
    /// `btCapsuleShape` and the game's `PZCapsuleShape`, 0 for
    /// `btCapsuleShapeX`, 2 for `btCapsuleShapeZ`). `height` is the distance
    /// between the two cap centers.
    pub fn new_capsule(radius: f64, height: f64, up_axis: usize) -> Shape {
        assert!(up_axis < 3, "capsule up axis {up_axis}");
        let mut ci = ConvexInternal::new();
        // 0.5f*height: the float literal promotes to double exactly.
        let half_height = 0.5 * height;
        ci.implicit_shape_dimensions = match up_axis {
            0 => Vec3::new(half_height, radius, radius),
            1 => Vec3::new(radius, half_height, radius),
            _ => Vec3::new(radius, radius, half_height),
        };
        Shape {
            kind: ShapeKind::Capsule(CapsuleShape { ci, up_axis }),
        }
    }
    /// `btConvexHullShape(points, numPoints, stride)` with packed xyz
    /// doubles (`stride == 24`) or `btVector3`s (`stride == 32`).
    pub fn new_convex_hull(points: &[f64], num_points: usize, stride_doubles: usize) -> Shape {
        let mut pts = Vec::with_capacity(num_points);
        for i in 0..num_points {
            let p = &points[i * stride_doubles..];
            pts.push(Vec3::new(p[0], p[1], p[2]));
        }
        let mut h = ConvexHullShape {
            ci: ConvexInternal::new(),
            local_aabb_min: Vec3::new(1.0, 1.0, 1.0),
            local_aabb_max: Vec3::new(-1.0, -1.0, -1.0),
            is_local_aabb_valid: false,
            unscaled_points: pts,
        };
        h.recalc_local_aabb();
        Shape {
            kind: ShapeKind::ConvexHull(h),
        }
    }
    /// `btCompoundShape(enableDynamicAabbTree)`.
    pub fn new_compound(enable_dynamic_aabb_tree: bool) -> Shape {
        Shape {
            kind: ShapeKind::Compound(CompoundShape::new(enable_dynamic_aabb_tree)),
        }
    }

    /// `getShapeType()`.
    #[inline(always)]
    pub fn shape_type(&self) -> i32 {
        match &self.kind {
            ShapeKind::Box(_) => BOX_SHAPE_PROXYTYPE,
            ShapeKind::Sphere(_) => SPHERE_SHAPE_PROXYTYPE,
            ShapeKind::Capsule(_) => CAPSULE_SHAPE_PROXYTYPE,
            ShapeKind::ConvexHull(_) => CONVEX_HULL_SHAPE_PROXYTYPE,
            ShapeKind::Compound(_) => COMPOUND_SHAPE_PROXYTYPE,
        }
    }
    #[inline(always)]
    pub fn is_convex(&self) -> bool {
        is_convex(self.shape_type())
    }
    #[inline(always)]
    pub fn is_compound(&self) -> bool {
        is_compound(self.shape_type())
    }
    #[inline(always)]
    pub fn is_polyhedral(&self) -> bool {
        is_polyhedral(self.shape_type())
    }
    #[inline(always)]
    pub fn is_concave(&self) -> bool {
        is_concave(self.shape_type())
    }
    #[inline(always)]
    pub fn is_convex2d(&self) -> bool {
        is_convex2d(self.shape_type())
    }
    #[inline(always)]
    pub fn is_infinite(&self) -> bool {
        is_infinite(self.shape_type())
    }
    #[inline(always)]
    pub fn is_non_moving(&self) -> bool {
        is_non_moving(self.shape_type())
    }

    /// The convex-internal members (panics for a compound).
    #[inline(always)]
    pub fn ci(&self) -> &ConvexInternal {
        match &self.kind {
            ShapeKind::Box(s) => &s.ci,
            ShapeKind::Sphere(s) => &s.ci,
            ShapeKind::Capsule(s) => &s.ci,
            ShapeKind::ConvexHull(s) => &s.ci,
            ShapeKind::Compound(_) => panic!("compound shape is not convex"),
        }
    }
    /// The capsule (panics for other shapes).
    #[inline(always)]
    pub fn capsule(&self) -> &CapsuleShape {
        match &self.kind {
            ShapeKind::Capsule(c) => c,
            _ => panic!("not a capsule shape"),
        }
    }
    #[inline(always)]
    pub fn compound(&self) -> &CompoundShape {
        match &self.kind {
            ShapeKind::Compound(c) => c,
            _ => panic!("not a compound shape"),
        }
    }
    #[inline(always)]
    pub fn compound_mut(&mut self) -> &mut CompoundShape {
        match &mut self.kind {
            ShapeKind::Compound(c) => c,
            _ => panic!("not a compound shape"),
        }
    }
    #[inline(always)]
    pub fn convex_hull(&self) -> &ConvexHullShape {
        match &self.kind {
            ShapeKind::ConvexHull(c) => c,
            _ => panic!("not a convex hull"),
        }
    }

    /// `btSphereShape::getRadius`.
    #[inline(always)]
    pub fn sphere_radius(&self) -> f64 {
        let ci = self.ci();
        ci.implicit_shape_dimensions.x * ci.local_scaling.x
    }
    /// `btBoxShape::getHalfExtentsWithoutMargin`.
    #[inline(always)]
    pub fn box_half_extents_without_margin(&self) -> Vec3 {
        self.ci().implicit_shape_dimensions
    }
    /// `btBoxShape::getHalfExtentsWithMargin`.
    #[inline(always)]
    pub fn box_half_extents_with_margin(&self) -> Vec3 {
        let mut half_extents = self.box_half_extents_without_margin();
        let m = self.get_margin();
        half_extents += Vec3::new(m, m, m);
        half_extents
    }

    /// `getMargin()` (virtual).
    pub fn get_margin(&self) -> f64 {
        match &self.kind {
            ShapeKind::Box(s) => s.ci.collision_margin,
            ShapeKind::Sphere(_) => self.sphere_radius(),
            ShapeKind::Capsule(s) => s.ci.collision_margin,
            ShapeKind::ConvexHull(s) => s.ci.collision_margin,
            ShapeKind::Compound(c) => c.collision_margin,
        }
    }
    /// `setMargin(margin)` (virtual).
    pub fn set_margin(&mut self, collision_margin: f64) {
        match &mut self.kind {
            ShapeKind::Box(BoxShape { ci: c }) | ShapeKind::Capsule(CapsuleShape { ci: c, .. }) => {
                let om = c.collision_margin;
                let old_margin = Vec3::new(om, om, om);
                let with_margin = c.implicit_shape_dimensions + old_margin;
                c.collision_margin = collision_margin;
                let nm = c.collision_margin;
                let new_margin = Vec3::new(nm, nm, nm);
                c.implicit_shape_dimensions = with_margin - new_margin;
            }
            ShapeKind::Sphere(s) => s.ci.collision_margin = collision_margin,
            ShapeKind::ConvexHull(s) => s.ci.collision_margin = collision_margin,
            ShapeKind::Compound(c) => c.collision_margin = collision_margin,
        }
    }
    /// `getLocalScaling()`.
    pub fn get_local_scaling(&self) -> Vec3 {
        match &self.kind {
            ShapeKind::Compound(c) => c.local_scaling,
            _ => self.ci().local_scaling,
        }
    }
    /// `setLocalScaling` for the convex shapes (compounds go through
    /// [`Shapes::compound_set_local_scaling`]).
    pub fn set_local_scaling_convex(&mut self, scaling: &Vec3) {
        match &mut self.kind {
            ShapeKind::Box(BoxShape { ci: c }) | ShapeKind::Capsule(CapsuleShape { ci: c, .. }) => {
                let om = c.collision_margin;
                let old_margin = Vec3::new(om, om, om);
                let with_margin = c.implicit_shape_dimensions + old_margin;
                let unscaled = with_margin.div_vec(&c.local_scaling);
                c.local_scaling = scaling.absolute();
                c.implicit_shape_dimensions = (unscaled * c.local_scaling) - old_margin;
            }
            ShapeKind::Sphere(s) => s.ci.local_scaling = scaling.absolute(),
            ShapeKind::ConvexHull(s) => {
                s.ci.local_scaling = *scaling;
                s.recalc_local_aabb();
            }
            ShapeKind::Compound(_) => panic!("use Shapes::compound_set_local_scaling"),
        }
    }

    /// `localGetSupportingVertexWithoutMargin` (virtual).
    pub fn local_get_supporting_vertex_without_margin(&self, vec: &Vec3) -> Vec3 {
        match &self.kind {
            ShapeKind::Box(s) => {
                let h = &s.ci.implicit_shape_dimensions;
                Vec3::new(
                    fsel(vec.x, h.x, -h.x),
                    fsel(vec.y, h.y, -h.y),
                    fsel(vec.z, h.z, -h.z),
                )
            }
            ShapeKind::Sphere(_) => Vec3::ZERO,
            ShapeKind::Capsule(c) => c.support_without_margin(vec),
            ShapeKind::ConvexHull(h) => h.local_get_supporting_vertex_without_margin(vec),
            ShapeKind::Compound(_) => panic!("compound shape is not convex"),
        }
    }

    /// `localGetSupportingVertex` (virtual).
    pub fn local_get_supporting_vertex(&self, vec: &Vec3) -> Vec3 {
        match &self.kind {
            ShapeKind::Box(_) => {
                let h = self.box_half_extents_with_margin();
                Vec3::new(
                    fsel(vec.x, h.x, -h.x),
                    fsel(vec.y, h.y, -h.y),
                    fsel(vec.z, h.z, -h.z),
                )
            }
            ShapeKind::Sphere(_) => {
                let mut sup_vertex = Vec3::ZERO;
                let mut vecnorm = *vec;
                if vecnorm.length2() < SIMD_EPSILON * SIMD_EPSILON {
                    vecnorm.set_value(-1.0, -1.0, -1.0);
                }
                vecnorm.normalize();
                sup_vertex += self.get_margin() * vecnorm;
                sup_vertex
            }
            ShapeKind::Capsule(_) | ShapeKind::ConvexHull(_) => {
                // btConvexInternalShape::localGetSupportingVertex
                let mut sup_vertex = self.local_get_supporting_vertex_without_margin(vec);
                let m = self.ci().collision_margin;
                if m != 0.0 {
                    let mut vecnorm = *vec;
                    if vecnorm.length2() < SIMD_EPSILON * SIMD_EPSILON {
                        vecnorm.set_value(-1.0, -1.0, -1.0);
                    }
                    vecnorm.normalize();
                    sup_vertex += m * vecnorm;
                }
                sup_vertex
            }
            ShapeKind::Compound(_) => panic!("compound shape is not convex"),
        }
    }

    /// `batchedUnitVectorGetSupportingVertexWithoutMargin`.
    pub fn batched_unit_vector_get_supporting_vertex_without_margin(
        &self,
        vectors: &[Vec3],
        out: &mut [Vec3],
    ) {
        match &self.kind {
            ShapeKind::Box(s) => {
                let h = &s.ci.implicit_shape_dimensions;
                for (o, vec) in out.iter_mut().zip(vectors) {
                    o.set_value(
                        fsel(vec.x, h.x, -h.x),
                        fsel(vec.y, h.y, -h.y),
                        fsel(vec.z, h.z, -h.z),
                    );
                }
            }
            ShapeKind::Sphere(_) => {
                for o in out.iter_mut().take(vectors.len()) {
                    o.set_value(0.0, 0.0, 0.0);
                }
            }
            ShapeKind::Capsule(c) => c.batched(vectors, out),
            ShapeKind::ConvexHull(h) => h.batched(vectors, out),
            ShapeKind::Compound(_) => panic!("compound shape is not convex"),
        }
    }

    /// `getAabb(t, aabbMin, aabbMax)` (virtual).
    pub fn get_aabb(&self, t: &Transform) -> (Vec3, Vec3) {
        match &self.kind {
            ShapeKind::Box(s) => {
                transform_aabb_half(&s.ci.implicit_shape_dimensions, s.ci.collision_margin, t)
            }
            ShapeKind::Sphere(_) => {
                let center = t.origin;
                let m = self.get_margin();
                let extent = Vec3::new(m, m, m);
                (center - extent, center + extent)
            }
            ShapeKind::Capsule(c) => c.get_aabb(t),
            ShapeKind::ConvexHull(h) => transform_aabb(
                &h.local_aabb_min,
                &h.local_aabb_max,
                h.ci.collision_margin,
                t,
            ),
            ShapeKind::Compound(c) => {
                let mut local_half_extents = 0.5 * (c.local_aabb_max - c.local_aabb_min);
                let mut local_center = 0.5 * (c.local_aabb_max + c.local_aabb_min);
                if c.children.is_empty() {
                    local_half_extents.set_value(0.0, 0.0, 0.0);
                    local_center.set_value(0.0, 0.0, 0.0);
                }
                let m = c.collision_margin;
                local_half_extents += Vec3::new(m, m, m);
                let abs_b = t.basis.absolute();
                let center = t.apply(&local_center);
                let extent = local_half_extents.dot3(&abs_b.el[0], &abs_b.el[1], &abs_b.el[2]);
                (center - extent, center + extent)
            }
        }
    }

    /// `btConvexInternalShape::getAabbSlow`.
    pub fn get_aabb_slow(&self, trans: &Transform) -> (Vec3, Vec3) {
        let margin = self.get_margin();
        let mut min_aabb = Vec3::ZERO;
        let mut max_aabb = Vec3::ZERO;
        for i in 0..3 {
            let mut vec = Vec3::ZERO;
            vec[i] = 1.0;
            let sv = self.local_get_supporting_vertex(&(vec * trans.basis));
            let tmp = trans.apply(&sv);
            max_aabb[i] = tmp[i] + margin;
            vec[i] = -1.0;
            let tmp = trans.apply(&self.local_get_supporting_vertex(&(vec * trans.basis)));
            min_aabb[i] = tmp[i] - margin;
        }
        (min_aabb, max_aabb)
    }

    /// `calculateLocalInertia(mass, inertia)`.
    pub fn calculate_local_inertia(&self, mass: f64) -> Vec3 {
        match &self.kind {
            ShapeKind::Box(_) => {
                let h = self.box_half_extents_with_margin();
                let lx = 2.0 * h.x;
                let ly = 2.0 * h.y;
                let lz = 2.0 * h.z;
                Vec3::new(
                    mass / 12.0 * fma(ly, ly, lz * lz),
                    mass / 12.0 * fma(lx, lx, lz * lz),
                    mass / 12.0 * fma(lx, lx, ly * ly),
                )
            }
            ShapeKind::Sphere(_) => {
                let elem = 0.4 * mass * self.get_margin() * self.get_margin();
                Vec3::new(elem, elem, elem)
            }
            ShapeKind::Capsule(c) => {
                let radius = c.get_radius();
                let mut half_extents = Vec3::new(radius, radius, radius);
                half_extents[c.up_axis] += c.get_half_height();
                let margin = CONVEX_DISTANCE_MARGIN;
                let lx = 2.0 * (half_extents.x + margin);
                let ly = 2.0 * (half_extents.y + margin);
                let lz = 2.0 * (half_extents.z + margin);
                let x2 = lx * lx;
                let y2 = ly * ly;
                let z2 = lz * lz;
                let scaledmass = mass * 0.08333333;
                Vec3::new(
                    scaledmass * (y2 + z2),
                    scaledmass * (x2 + z2),
                    scaledmass * (x2 + y2),
                )
            }
            ShapeKind::ConvexHull(_) => {
                // btPolyhedralConvexShape::calculateLocalInertia
                let margin = self.get_margin();
                let (aabb_min, aabb_max) = self.get_aabb(&Transform::IDENTITY);
                let half_extents = (aabb_max - aabb_min) * 0.5;
                let lx = 2.0 * (half_extents.x + margin);
                let ly = 2.0 * (half_extents.y + margin);
                let lz = 2.0 * (half_extents.z + margin);
                let x2 = lx * lx;
                let y2 = ly * ly;
                let z2 = lz * lz;
                let scaledmass = mass * 0.08333333;
                scaledmass * Vec3::new(y2 + z2, x2 + z2, x2 + y2)
            }
            ShapeKind::Compound(_) => {
                let (aabb_min, aabb_max) = self.get_aabb(&Transform::IDENTITY);
                let half_extents = (aabb_max - aabb_min) * 0.5;
                let lx = 2.0 * half_extents.x;
                let ly = 2.0 * half_extents.y;
                let lz = 2.0 * half_extents.z;
                Vec3::new(
                    mass / 12.0 * fma(ly, ly, lz * lz),
                    mass / 12.0 * fma(lx, lx, lz * lz),
                    mass / 12.0 * fma(lx, lx, ly * ly),
                )
            }
        }
    }

    /// `getBoundingSphere(center, radius)`.
    pub fn get_bounding_sphere(&self) -> (Vec3, f64) {
        let (aabb_min, aabb_max) = self.get_aabb(&Transform::IDENTITY);
        let radius = (aabb_max - aabb_min).length() * 0.5;
        let center = (aabb_min + aabb_max) * 0.5;
        (center, radius)
    }
    /// `getAngularMotionDisc()`.
    pub fn get_angular_motion_disc(&self) -> f64 {
        let (center, mut disc) = self.get_bounding_sphere();
        disc += center.length();
        disc
    }
    /// `getContactBreakingThreshold(defaultContactThresholdFactor)`.
    pub fn get_contact_breaking_threshold(&self, default_contact_threshold: f64) -> f64 {
        self.get_angular_motion_disc() * default_contact_threshold
    }
    /// `calculateTemporalAabb`.
    pub fn calculate_temporal_aabb(
        &self,
        cur_trans: &Transform,
        linvel: &Vec3,
        angvel: &Vec3,
        time_step: f64,
    ) -> (Vec3, Vec3) {
        let (tmin, tmax) = self.get_aabb(cur_trans);
        let mut maxx = tmax.x;
        let mut maxy = tmax.y;
        let mut maxz = tmax.z;
        let mut minx = tmin.x;
        let mut miny = tmin.y;
        let mut minz = tmin.z;
        let lin_motion = *linvel * time_step;
        if lin_motion.x > 0.0 {
            maxx += lin_motion.x;
        } else {
            minx += lin_motion.x;
        }
        if lin_motion.y > 0.0 {
            maxy += lin_motion.y;
        } else {
            miny += lin_motion.y;
        }
        if lin_motion.z > 0.0 {
            maxz += lin_motion.z;
        } else {
            minz += lin_motion.z;
        }
        let angular_motion = angvel.length() * self.get_angular_motion_disc() * time_step;
        let am = Vec3::new(angular_motion, angular_motion, angular_motion);
        let mut out_min = Vec3::new(minx, miny, minz);
        let mut out_max = Vec3::new(maxx, maxy, maxz);
        out_min -= am;
        out_max += am;
        (out_min, out_max)
    }

    // ---- btConvexShape non-virtual switch versions ----

    /// `localGetSupportVertexWithoutMarginNonVirtual`.
    pub fn local_get_support_vertex_without_margin_non_virtual(&self, local_dir: &Vec3) -> Vec3 {
        match &self.kind {
            ShapeKind::Sphere(_) => Vec3::ZERO,
            ShapeKind::Box(s) => {
                let h = &s.ci.implicit_shape_dimensions;
                Vec3::new(
                    fsel(local_dir.x, h.x, -h.x),
                    fsel(local_dir.y, h.y, -h.y),
                    fsel(local_dir.z, h.z, -h.z),
                )
            }
            ShapeKind::Capsule(c) => c.support_without_margin(local_dir),
            ShapeKind::ConvexHull(h) => {
                // convexHullSupport(localDir, points, numPoints, localScaling)
                let vec = *local_dir * h.ci.local_scaling;
                let mut max_dot = 0.0;
                let pt_index = vec.max_dot(&h.unscaled_points, &mut max_dot);
                h.unscaled_points[pt_index as usize] * h.ci.local_scaling
            }
            ShapeKind::Compound(_) => panic!("compound shape is not convex"),
        }
    }
    /// `localGetSupportVertexNonVirtual`.
    pub fn local_get_support_vertex_non_virtual(&self, local_dir: &Vec3) -> Vec3 {
        let mut local_dir_norm = *local_dir;
        if local_dir_norm.length2() < SIMD_EPSILON * SIMD_EPSILON {
            local_dir_norm.set_value(-1.0, -1.0, -1.0);
        }
        local_dir_norm.normalize();
        self.local_get_support_vertex_without_margin_non_virtual(&local_dir_norm)
            + self.get_margin_non_virtual() * local_dir_norm
    }
    /// `getMarginNonVirtual`.
    pub fn get_margin_non_virtual(&self) -> f64 {
        match &self.kind {
            ShapeKind::Sphere(_) => self.sphere_radius(),
            ShapeKind::Box(s) => s.ci.collision_margin,
            ShapeKind::Capsule(s) => s.ci.collision_margin,
            ShapeKind::ConvexHull(s) => s.ci.collision_margin,
            ShapeKind::Compound(c) => c.collision_margin,
        }
    }
    /// `getAabbNonVirtual`.
    pub fn get_aabb_non_virtual(&self, t: &Transform) -> (Vec3, Vec3) {
        match &self.kind {
            ShapeKind::Sphere(s) => {
                let radius = s.ci.implicit_shape_dimensions.x;
                let margin = radius + self.get_margin_non_virtual();
                let center = t.origin;
                let extent = Vec3::new(margin, margin, margin);
                (center - extent, center + extent)
            }
            ShapeKind::Box(s) => {
                let margin = s.ci.collision_margin;
                let mut half_extents = s.ci.implicit_shape_dimensions;
                half_extents += Vec3::new(margin, margin, margin);
                let abs_b = t.basis.absolute();
                let center = t.origin;
                let extent = half_extents.dot3(&abs_b.el[0], &abs_b.el[1], &abs_b.el[2]);
                (center - extent, center + extent)
            }
            ShapeKind::Capsule(c) => c.get_aabb(t),
            ShapeKind::ConvexHull(h) => transform_aabb(
                &h.local_aabb_min,
                &h.local_aabb_max,
                h.ci.collision_margin,
                t,
            ),
            ShapeKind::Compound(_) => self.get_aabb(t),
        }
    }
}

impl ConvexHullShape {
    /// `localGetSupportingVertexWithoutMargin`.
    pub fn local_get_supporting_vertex_without_margin(&self, vec: &Vec3) -> Vec3 {
        if !self.unscaled_points.is_empty() {
            let scaled = *vec * self.ci.local_scaling;
            let mut max_dot = -BT_LARGE_FLOAT;
            let index = scaled.max_dot(&self.unscaled_points, &mut max_dot);
            return self.unscaled_points[index as usize] * self.ci.local_scaling;
        }
        Vec3::ZERO
    }
    /// `batchedUnitVectorGetSupportingVertexWithoutMargin`.
    pub fn batched(&self, vectors: &[Vec3], out: &mut [Vec3]) {
        for o in out.iter_mut().take(vectors.len()) {
            o.w = -BT_LARGE_FLOAT;
        }
        for (j, v) in vectors.iter().enumerate() {
            let vec = *v * self.ci.local_scaling;
            if !self.unscaled_points.is_empty() {
                let mut new_dot = 0.0;
                let i = vec.max_dot(&self.unscaled_points, &mut new_dot);
                out[j] = self.unscaled_points[i as usize] * self.ci.local_scaling;
                out[j].w = new_dot;
            } else {
                out[j].w = -BT_LARGE_FLOAT;
            }
        }
    }
    /// `btPolyhedralConvexAabbCachingShape::recalcLocalAabb`.
    pub fn recalc_local_aabb(&mut self) {
        self.is_local_aabb_valid = true;
        let mut supporting = [Vec3::ZERO; 6];
        self.batched(&DIRECTIONS6, &mut supporting);
        for i in 0..3 {
            self.local_aabb_max[i] = supporting[i][i] + self.ci.collision_margin;
            self.local_aabb_min[i] = supporting[i + 3][i] - self.ci.collision_margin;
        }
    }
    /// `addPoint(point, recalculateLocalAabb)`.
    pub fn add_point(&mut self, point: &Vec3, recalculate_local_aabb: bool) {
        self.unscaled_points.push(*point);
        if recalculate_local_aabb {
            self.recalc_local_aabb();
        }
    }
    #[inline(always)]
    pub fn get_scaled_point(&self, i: usize) -> Vec3 {
        self.unscaled_points[i] * self.ci.local_scaling
    }
    #[inline(always)]
    pub fn get_num_points(&self) -> usize {
        self.unscaled_points.len()
    }
}

/// Arena of collision shapes.
#[derive(Clone, Debug, Default)]
pub struct Shapes {
    pub slots: Vec<Option<Shape>>,
}

impl Shapes {
    pub fn new() -> Shapes {
        Shapes { slots: Vec::new() }
    }
    /// Stores a shape and returns its id (free slots are reused).
    pub fn add(&mut self, shape: Shape) -> ShapeId {
        if let Some(i) = self.slots.iter().position(|s| s.is_none()) {
            self.slots[i] = Some(shape);
            return i;
        }
        self.slots.push(Some(shape));
        self.slots.len() - 1
    }
    /// Deletes a shape (`delete shape`).
    pub fn remove(&mut self, id: ShapeId) -> Option<Shape> {
        self.slots.get_mut(id).and_then(|s| s.take())
    }
    #[inline(always)]
    pub fn get(&self, id: ShapeId) -> &Shape {
        self.slots[id].as_ref().expect("dangling shape id")
    }
    #[inline(always)]
    pub fn get_mut(&mut self, id: ShapeId) -> &mut Shape {
        self.slots[id].as_mut().expect("dangling shape id")
    }

    /// `btCompoundShape::addChildShape(localTransform, shape)`.
    pub fn compound_add_child_shape(
        &mut self,
        compound: ShapeId,
        local_transform: &Transform,
        shape: ShapeId,
    ) {
        let (child_type, child_margin, local_aabb_min, local_aabb_max) = {
            let s = self.get(shape);
            let (mn, mx) = s.get_aabb(local_transform);
            (s.shape_type(), s.get_margin(), mn, mx)
        };
        let c = self.get_mut(compound).compound_mut();
        c.update_revision += 1;
        for i in 0..3 {
            if c.local_aabb_min[i] > local_aabb_min[i] {
                c.local_aabb_min[i] = local_aabb_min[i];
            }
            if c.local_aabb_max[i] < local_aabb_max[i] {
                c.local_aabb_max[i] = local_aabb_max[i];
            }
        }
        let index = c.children.len();
        let node = c.dynamic_aabb_tree.as_mut().map(|tree| {
            let bounds = DbvtAabbMm::from_mm(&local_aabb_min, &local_aabb_max);
            tree.insert(&bounds, index)
        });
        c.children.push(CompoundShapeChild {
            transform: *local_transform,
            child_shape: shape,
            child_shape_type: child_type,
            child_margin,
            node,
        });
    }

    /// `btCompoundShape::updateChildTransform`.
    pub fn compound_update_child_transform(
        &mut self,
        compound: ShapeId,
        child_index: usize,
        new_child_transform: &Transform,
        should_recalculate_local_aabb: bool,
    ) {
        let child_shape = self.get(compound).compound().children[child_index].child_shape;
        let (mn, mx) = self.get(child_shape).get_aabb(new_child_transform);
        let c = self.get_mut(compound).compound_mut();
        c.children[child_index].transform = *new_child_transform;
        if let Some(tree) = c.dynamic_aabb_tree.as_mut() {
            let bounds = DbvtAabbMm::from_mm(&mn, &mx);
            let node = c.children[child_index]
                .node
                .expect("compound child without tree node");
            tree.update(node, &bounds);
        }
        if should_recalculate_local_aabb {
            self.compound_recalculate_local_aabb(compound);
        }
    }

    /// `btCompoundShape::recalculateLocalAabb`.
    pub fn compound_recalculate_local_aabb(&mut self, compound: ShapeId) {
        let mut aabb_min = Vec3::new(BT_LARGE_FLOAT, BT_LARGE_FLOAT, BT_LARGE_FLOAT);
        let mut aabb_max = Vec3::new(-BT_LARGE_FLOAT, -BT_LARGE_FLOAT, -BT_LARGE_FLOAT);
        {
            let c = self.get(compound).compound();
            for ch in &c.children {
                let (mn, mx) = self.get(ch.child_shape).get_aabb(&ch.transform);
                for i in 0..3 {
                    if aabb_min[i] > mn[i] {
                        aabb_min[i] = mn[i];
                    }
                    if aabb_max[i] < mx[i] {
                        aabb_max[i] = mx[i];
                    }
                }
            }
        }
        let c = self.get_mut(compound).compound_mut();
        c.local_aabb_min = aabb_min;
        c.local_aabb_max = aabb_max;
    }

    /// `btCompoundShape::removeChildShapeByIndex`.
    pub fn compound_remove_child_shape_by_index(
        &mut self,
        compound: ShapeId,
        child_shape_index: usize,
    ) {
        let c = self.get_mut(compound).compound_mut();
        c.update_revision += 1;
        if let Some(tree) = c.dynamic_aabb_tree.as_mut() {
            tree.remove(
                c.children[child_shape_index]
                    .node
                    .expect("child without node"),
            );
        }
        let last = c.children.len() - 1;
        c.children.swap(child_shape_index, last);
        if let Some(tree) = c.dynamic_aabb_tree.as_mut() {
            if let Some(node) = c.children[child_shape_index].node {
                tree.set_leaf_data(node, child_shape_index);
            }
        }
        c.children.pop();
    }

    /// `btCompoundShape::setLocalScaling`.
    pub fn compound_set_local_scaling(&mut self, compound: ShapeId, scaling: &Vec3) {
        let n = self.get(compound).compound().children.len();
        for i in 0..n {
            let (mut child_trans, child_shape, comp_scaling) = {
                let c = self.get(compound).compound();
                (
                    c.children[i].transform,
                    c.children[i].child_shape,
                    c.local_scaling,
                )
            };
            let mut child_scale = self.get(child_shape).get_local_scaling();
            child_scale = (child_scale * *scaling).div_vec(&comp_scaling);
            self.get_mut(child_shape)
                .set_local_scaling_convex(&child_scale);
            child_trans.origin = (child_trans.origin * *scaling).div_vec(&comp_scaling);
            self.compound_update_child_transform(compound, i, &child_trans, false);
        }
        self.get_mut(compound).compound_mut().local_scaling = *scaling;
        self.compound_recalculate_local_aabb(compound);
    }
}
