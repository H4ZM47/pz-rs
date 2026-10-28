// Bullet Continuous Collision Detection and Physics Library
// Copyright (c) 2003-2006 Erwin Coumans  http://continuousphysics.com/Bullet/
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
// Altered for pz-rs: Rust translation of btCollisionDispatcher,
// btDefaultCollisionConfiguration's dispatch table, btCollisionObjectWrapper,
// btManifoldResult and the collision algorithms the game reaches
// (convex-convex with its capsule-capsule special case, compound,
// compound-compound, box-box, sphere-sphere, empty) from Bullet 2.82,
// double precision.

//! Collision dispatch: the narrowphase algorithms run on broadphase pairs.
//!
//! Algorithms live in the dispatcher's arena and are addressed by
//! [`AlgId`]; a broadphase pair stores its algorithm id. Manifolds live in
//! the dispatcher's manifold arena; `manifolds_ptr` keeps Bullet's
//! `m_manifoldsPtr` order (the island manager and solver iterate it).

#![allow(clippy::manual_clamp, clippy::needless_range_loop)]

use super::broadphase::dbvt::{intersect, Dbvt, DbvtAabbMm, NodeId, NIL};
use super::broadphase::pair_cache::PairDispatcher;
use super::broadphase::simple_pair_cache::HashedSimplePairCache;
use super::collision_object::*;
use super::dynamics::rigid_body::check_collide_with;
use super::dynamics::typed_constraint::ConstraintArena;
use super::linear_math::*;
use super::manifold::*;
use super::narrowphase::box_box::box_box_get_closest_points;
use super::narrowphase::gjk_pair_detector::{
    ClosestPointInput, DiscreteResult, GjkPairDetector, PenetrationDepthSolver,
};
use super::narrowphase::simplex::VoronoiSimplexSolver;
use super::shapes::*;

/// Index of a collision algorithm in the dispatcher's arena.
pub type AlgId = usize;

/// The object arena type shared by the collision world and the solver.
pub type ObjArena = Vec<Option<CollisionObject>>;

#[inline(always)]
pub fn obj(objects: &ObjArena, id: ObjId) -> &CollisionObject {
    objects[id].as_ref().expect("dangling object id")
}

pub const CD_STATIC_STATIC_REPORTED: i32 = 1;
pub const CD_USE_RELATIVE_CONTACT_BREAKING_THRESHOLD: i32 = 2;
pub const CD_DISABLE_CONTACTPOOL_DYNAMIC_ALLOCATION: i32 = 4;

pub const DISPATCH_DISCRETE: i32 = 1;
pub const DISPATCH_CONTINUOUS: i32 = 2;

/// `btDispatcherInfo`.
#[derive(Clone, Debug)]
pub struct DispatcherInfo {
    pub time_step: f64,
    pub step_count: i32,
    pub dispatch_func: i32,
    pub time_of_impact: f64,
    pub use_continuous: bool,
    pub enable_sat_convex: bool,
    pub enable_spu: bool,
    pub use_epa: bool,
    pub allowed_ccd_penetration: f64,
    pub use_convex_conservative_distance_util: bool,
    pub convex_conservative_distance_threshold: f64,
}

impl Default for DispatcherInfo {
    fn default() -> Self {
        DispatcherInfo {
            time_step: 0.0,
            step_count: 0,
            dispatch_func: DISPATCH_DISCRETE,
            time_of_impact: 1.0,
            use_continuous: true,
            enable_sat_convex: false,
            enable_spu: true,
            use_epa: true,
            allowed_ccd_penetration: 0.04,
            use_convex_conservative_distance_util: false,
            convex_conservative_distance_threshold: 0.0,
        }
    }
}

/// `gContactAddedCallback`: called by `btManifoldResult::addContactPoint`
/// when either object has `CF_CUSTOM_MATERIAL_CALLBACK`.
pub trait ContactAddedHook {
    /// Whether a callback is installed (`gContactAddedCallback != 0`).
    fn contact_added_enabled(&self) -> bool {
        false
    }
    /// `(cp, colObj0Wrap, partId0, index0, colObj1Wrap, partId1, index1)`;
    /// the wrappers are reduced to their collision objects.
    #[allow(clippy::too_many_arguments)]
    fn contact_added(
        &mut self,
        _cp: &mut ManifoldPoint,
        _obj0: ObjId,
        _co0: &CollisionObject,
        _part_id0: i32,
        _index0: i32,
        _obj1: ObjId,
        _co1: &CollisionObject,
        _part_id1: i32,
        _index1: i32,
    ) -> bool {
        false
    }
}

/// No contact-added callback.
pub struct NoContactHook;
impl ContactAddedHook for NoContactHook {}

/// `btCollisionObjectWrapper` (the parent pointer is never read in the
/// reached code).
#[derive(Clone, Copy, Debug)]
pub struct ObjWrapper {
    pub shape: ShapeId,
    pub obj: ObjId,
    pub world_transform: Transform,
    pub part_id: i32,
    pub index: i32,
}

/// Read-only state a collision algorithm sees, plus the contact hook.
pub struct Ctx<'a> {
    pub shapes: &'a Shapes,
    pub objects: &'a ObjArena,
    /// Constraint arena for `checkCollideWith` (`None`: no constraints).
    pub constraints: Option<&'a ConstraintArena>,
    pub info: &'a DispatcherInfo,
    pub hook: &'a mut dyn ContactAddedHook,
}

/// `btManifoldResult`.
#[derive(Clone, Copy, Debug)]
pub struct ManifoldResult {
    pub manifold: Option<ManifoldId>,
    pub body0: ObjWrapper,
    pub body1: ObjWrapper,
    /// Uninitialised in C++ until an algorithm sets them; 0 here.
    pub part_id0: i32,
    pub part_id1: i32,
    pub index0: i32,
    pub index1: i32,
}

/// `btManifoldResult::calculateCombinedFriction`.
pub fn calculate_combined_friction(b0: &CollisionObject, b1: &CollisionObject) -> f64 {
    let mut friction = b0.friction * b1.friction;
    const MAX_FRICTION: f64 = 10.0;
    if friction < -MAX_FRICTION {
        friction = -MAX_FRICTION;
    }
    if friction > MAX_FRICTION {
        friction = MAX_FRICTION;
    }
    friction
}

fn calculate_combined_rolling_friction(b0: &CollisionObject, b1: &CollisionObject) -> f64 {
    let mut friction = b0.rolling_friction * b1.rolling_friction;
    const MAX_FRICTION: f64 = 10.0;
    if friction < -MAX_FRICTION {
        friction = -MAX_FRICTION;
    }
    if friction > MAX_FRICTION {
        friction = MAX_FRICTION;
    }
    friction
}

/// `btManifoldResult::calculateCombinedRestitution`.
pub fn calculate_combined_restitution(b0: &CollisionObject, b1: &CollisionObject) -> f64 {
    b0.restitution * b1.restitution
}

impl ManifoldResult {
    pub fn new(body0: ObjWrapper, body1: ObjWrapper) -> ManifoldResult {
        ManifoldResult {
            manifold: None,
            body0,
            body1,
            part_id0: 0,
            part_id1: 0,
            index0: 0,
            index1: 0,
        }
    }

    /// `addContactPoint`.
    pub fn add_contact_point(
        &mut self,
        manifolds: &mut [Option<PersistentManifold>],
        ctx: &mut Ctx,
        normal_on_b_in_world: &Vec3,
        point_in_world: &Vec3,
        depth: f64,
    ) {
        let mid = self.manifold.expect("manifold result without manifold");
        let mf = manifolds[mid].as_mut().expect("dangling manifold");
        if depth > mf.get_contact_breaking_threshold() {
            return;
        }
        let is_swapped = mf.body0 != self.body0.obj;
        let point_a = *point_in_world + *normal_on_b_in_world * depth;
        let o0 = obj(ctx.objects, self.body0.obj);
        let o1 = obj(ctx.objects, self.body1.obj);
        let (local_a, local_b) = if is_swapped {
            (
                o1.world_transform.inv_xform(&point_a),
                o0.world_transform.inv_xform(point_in_world),
            )
        } else {
            (
                o0.world_transform.inv_xform(&point_a),
                o1.world_transform.inv_xform(point_in_world),
            )
        };
        let mut new_pt = ManifoldPoint::new(local_a, local_b, *normal_on_b_in_world, depth);
        new_pt.position_world_on_a = point_a;
        new_pt.position_world_on_b = *point_in_world;
        let mut insert_index = mf.get_cache_entry(&new_pt);
        new_pt.combined_friction = calculate_combined_friction(o0, o1);
        new_pt.combined_restitution = calculate_combined_restitution(o0, o1);
        new_pt.combined_rolling_friction = calculate_combined_rolling_friction(o0, o1);
        let n = new_pt.normal_world_on_b;
        plane_space1(
            &n,
            &mut new_pt.lateral_friction_dir1,
            &mut new_pt.lateral_friction_dir2,
        );
        if is_swapped {
            new_pt.part_id0 = self.part_id1;
            new_pt.part_id1 = self.part_id0;
            new_pt.index0 = self.index1;
            new_pt.index1 = self.index0;
        } else {
            new_pt.part_id0 = self.part_id0;
            new_pt.part_id1 = self.part_id1;
            new_pt.index0 = self.index0;
            new_pt.index1 = self.index1;
        }
        if insert_index >= 0 {
            mf.replace_contact_point(&new_pt, insert_index as usize);
        } else {
            insert_index = mf.add_manifold_point(&new_pt);
        }
        if ctx.hook.contact_added_enabled()
            && ((o0.collision_flags & CF_CUSTOM_MATERIAL_CALLBACK) != 0
                || (o1.collision_flags & CF_CUSTOM_MATERIAL_CALLBACK) != 0)
        {
            let (w0, w1) = if is_swapped {
                (self.body1, self.body0)
            } else {
                (self.body0, self.body1)
            };
            let c0 = obj(ctx.objects, w0.obj);
            let c1 = obj(ctx.objects, w1.obj);
            let cp = mf.get_contact_point_mut(insert_index as usize);
            ctx.hook.contact_added(
                cp,
                w0.obj,
                c0,
                new_pt.part_id0,
                new_pt.index0,
                w1.obj,
                c1,
                new_pt.part_id1,
                new_pt.index1,
            );
        }
    }

    /// `refreshContactPoints`.
    pub fn refresh_contact_points(
        &self,
        manifolds: &mut [Option<PersistentManifold>],
        objects: &ObjArena,
    ) {
        let mid = self.manifold.expect("manifold result without manifold");
        let mf = manifolds[mid].as_mut().expect("dangling manifold");
        if mf.get_num_contacts() == 0 {
            return;
        }
        let is_swapped = mf.body0 != self.body0.obj;
        let t0 = obj(objects, self.body0.obj).world_transform;
        let t1 = obj(objects, self.body1.obj).world_transform;
        if is_swapped {
            mf.refresh_contact_points(&t1, &t0);
        } else {
            mf.refresh_contact_points(&t0, &t1);
        }
    }
}

/// Adapts a [`ManifoldResult`] to the narrowphase `Result` interface.
struct ResultSink<'r, 'm, 'c, 'a> {
    res: &'r mut ManifoldResult,
    manifolds: &'m mut [Option<PersistentManifold>],
    ctx: &'c mut Ctx<'a>,
}

impl DiscreteResult for ResultSink<'_, '_, '_, '_> {
    fn set_shape_identifiers_a(&mut self, part_id0: i32, index0: i32) {
        self.res.part_id0 = part_id0;
        self.res.index0 = index0;
    }
    fn set_shape_identifiers_b(&mut self, part_id1: i32, index1: i32) {
        self.res.part_id1 = part_id1;
        self.res.index1 = index1;
    }
    fn add_contact_point(&mut self, normal: &Vec3, point: &Vec3, depth: f64) {
        self.res
            .add_contact_point(self.manifolds, self.ctx, normal, point, depth);
    }
}

/// `btPerturbedContactResult`.
struct PerturbedSink<'r, 'm, 'c, 'a> {
    inner: ResultSink<'r, 'm, 'c, 'a>,
    transform_a: Transform,
    transform_b: Transform,
    un_perturbed_transform: Transform,
    perturb_a: bool,
}

impl DiscreteResult for PerturbedSink<'_, '_, '_, '_> {
    fn set_shape_identifiers_a(&mut self, part_id0: i32, index0: i32) {
        self.inner.set_shape_identifiers_a(part_id0, index0);
    }
    fn set_shape_identifiers_b(&mut self, part_id1: i32, index1: i32) {
        self.inner.set_shape_identifiers_b(part_id1, index1);
    }
    fn add_contact_point(&mut self, normal: &Vec3, point: &Vec3, org_depth: f64) {
        let (start_pt, new_depth);
        if self.perturb_a {
            let end_pt_org = *point + *normal * org_depth;
            let end_pt =
                (self.un_perturbed_transform * self.transform_a.inverse()).apply(&end_pt_org);
            new_depth = (end_pt - *point).dot(normal);
            start_pt = end_pt + *normal * new_depth;
        } else {
            let end_pt = *point + *normal * org_depth;
            start_pt = (self.un_perturbed_transform * self.transform_b.inverse()).apply(point);
            new_depth = (end_pt - start_pt).dot(normal);
        }
        self.inner.add_contact_point(normal, &start_pt, new_depth);
    }
}

/// `segmentsClosestPoints` (btConvexConvexAlgorithm.cpp): returns
/// `(ptsVector, offsetA, offsetB)`.
fn segments_closest_points(
    translation: &Vec3,
    dir_a: &Vec3,
    hlen_a: f64,
    dir_b: &Vec3,
    hlen_b: f64,
) -> (Vec3, Vec3, Vec3) {
    let dir_a_dot_dir_b = dir_a.dot(dir_b);
    let dir_a_dot_trans = dir_a.dot(translation);
    let dir_b_dot_trans = dir_b.dot(translation);
    let denom = fma(-dir_a_dot_dir_b, dir_a_dot_dir_b, 1.0);
    let mut t_a;
    if denom == 0.0 {
        t_a = 0.0;
    } else {
        t_a = fma(-dir_b_dot_trans, dir_a_dot_dir_b, dir_a_dot_trans) / denom;
        if t_a < -hlen_a {
            t_a = -hlen_a;
        } else if t_a > hlen_a {
            t_a = hlen_a;
        }
    }
    let mut t_b = fma(t_a, dir_a_dot_dir_b, -dir_b_dot_trans);
    if t_b < -hlen_b {
        t_b = -hlen_b;
        t_a = fma(t_b, dir_a_dot_dir_b, dir_a_dot_trans);
        if t_a < -hlen_a {
            t_a = -hlen_a;
        } else if t_a > hlen_a {
            t_a = hlen_a;
        }
    } else if t_b > hlen_b {
        t_b = hlen_b;
        t_a = fma(t_b, dir_a_dot_dir_b, dir_a_dot_trans);
        if t_a < -hlen_a {
            t_a = -hlen_a;
        } else if t_a > hlen_a {
            t_a = hlen_a;
        }
    }
    let offset_a = *dir_a * t_a;
    let offset_b = *dir_b * t_b;
    let pts_vector = *translation - offset_a + offset_b;
    (pts_vector, offset_a, offset_b)
}

/// `capsuleCapsuleDistance`: returns `(distance, normalOnB, pointOnB)`.
/// When the distance exceeds the threshold the normal and point are not
/// computed (zero here; Bullet leaves them unset).
#[allow(clippy::too_many_arguments)]
pub fn capsule_capsule_distance(
    capsule_length_a: f64,
    capsule_radius_a: f64,
    capsule_length_b: f64,
    capsule_radius_b: f64,
    capsule_axis_a: usize,
    capsule_axis_b: usize,
    transform_a: &Transform,
    transform_b: &Transform,
    distance_threshold: f64,
) -> (f64, Vec3, Vec3) {
    let direction_a = transform_a.basis.get_column(capsule_axis_a);
    let translation_a = transform_a.origin;
    let direction_b = transform_b.basis.get_column(capsule_axis_b);
    let translation_b = transform_b.origin;
    let translation = translation_b - translation_a;
    let (pts_vector, _offset_a, offset_b) = segments_closest_points(
        &translation,
        &direction_a,
        capsule_length_a,
        &direction_b,
        capsule_length_b,
    );
    let distance = pts_vector.length() - capsule_radius_a - capsule_radius_b;
    let mut normal_on_b = Vec3::ZERO;
    if distance > distance_threshold {
        return (distance, normal_on_b, Vec3::ZERO);
    }
    let len_sqr = pts_vector.length2();
    if len_sqr <= SIMD_EPSILON * SIMD_EPSILON {
        let mut q = Vec3::ZERO;
        plane_space1(&direction_a, &mut normal_on_b, &mut q);
    } else {
        normal_on_b = pts_vector * -bt_recip_sqrt(len_sqr);
    }
    let point_on_b = transform_b.origin + offset_b + normal_on_b * capsule_radius_b;
    (distance, normal_on_b, point_on_b)
}

/// The collision algorithms (`btCollisionAlgorithm` subclasses).
#[derive(Clone, Debug)]
pub enum Algorithm {
    /// `btConvexConvexAlgorithm`.
    ConvexConvex {
        manifold: Option<ManifoldId>,
        own_manifold: bool,
        num_perturbation_iterations: i32,
        minimum_points_perturbation_threshold: i32,
    },
    /// `btCompoundCollisionAlgorithm`.
    Compound {
        child_algorithms: Vec<Option<AlgId>>,
        is_swapped: bool,
        shared_manifold: Option<ManifoldId>,
        compound_shape_revision: i32,
    },
    /// `btCompoundCompoundCollisionAlgorithm`.
    CompoundCompound {
        cache: HashedSimplePairCache,
        shared_manifold: Option<ManifoldId>,
        compound_shape_revision0: i32,
        compound_shape_revision1: i32,
    },
    /// `btBoxBoxCollisionAlgorithm`.
    BoxBox {
        manifold: Option<ManifoldId>,
        own_manifold: bool,
    },
    /// `btSphereSphereCollisionAlgorithm`.
    SphereSphere {
        manifold: Option<ManifoldId>,
        own_manifold: bool,
    },
    /// `btEmptyAlgorithm`.
    Empty,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CreateFunc {
    SphereSphere,
    BoxBox,
    ConvexConvex,
    CompoundCompound,
    Compound,
    SwappedCompound,
    Empty,
}

/// `btDefaultCollisionConfiguration::getCollisionAlgorithmCreateFunc` for
/// the shape types this port has (no triangles, planes or concave shapes).
fn create_func(t0: i32, t1: i32) -> CreateFunc {
    if t0 == SPHERE_SHAPE_PROXYTYPE && t1 == SPHERE_SHAPE_PROXYTYPE {
        return CreateFunc::SphereSphere;
    }
    if t0 == BOX_SHAPE_PROXYTYPE && t1 == BOX_SHAPE_PROXYTYPE {
        return CreateFunc::BoxBox;
    }
    if is_convex(t0) && is_convex(t1) {
        return CreateFunc::ConvexConvex;
    }
    if is_compound(t0) && is_compound(t1) {
        return CreateFunc::CompoundCompound;
    }
    if is_compound(t0) {
        return CreateFunc::Compound;
    }
    if is_compound(t1) {
        return CreateFunc::SwappedCompound;
    }
    CreateFunc::Empty
}

/// `btCollisionDispatcher` (with the `btDefaultCollisionConfiguration`
/// it was built from).
#[derive(Clone, Debug)]
pub struct CollisionDispatcher {
    pub dispatcher_flags: i32,
    /// Manifold arena.
    pub manifolds: Vec<Option<PersistentManifold>>,
    free_manifolds: Vec<ManifoldId>,
    /// `m_manifoldsPtr`.
    pub manifolds_ptr: Vec<ManifoldId>,
    /// Algorithm arena.
    pub algorithms: Vec<Option<Algorithm>>,
    free_algorithms: Vec<AlgId>,
    /// The configuration's shared `m_simplexSolver`.
    pub simplex_solver: VoronoiSimplexSolver,
    /// `btConvexConvexAlgorithm::CreateFunc::m_numPerturbationIterations`.
    pub convex_convex_num_perturbation_iterations: i32,
    /// `...::m_minimumPointsPerturbationThreshold`.
    pub convex_convex_minimum_points_perturbation_threshold: i32,
}

impl Default for CollisionDispatcher {
    fn default() -> Self {
        CollisionDispatcher::new()
    }
}

impl CollisionDispatcher {
    /// `btCollisionDispatcher(btDefaultCollisionConfiguration)` with EPA.
    pub fn new() -> CollisionDispatcher {
        CollisionDispatcher {
            dispatcher_flags: CD_USE_RELATIVE_CONTACT_BREAKING_THRESHOLD,
            manifolds: Vec::new(),
            free_manifolds: Vec::new(),
            manifolds_ptr: Vec::new(),
            algorithms: Vec::new(),
            free_algorithms: Vec::new(),
            simplex_solver: VoronoiSimplexSolver::new(),
            convex_convex_num_perturbation_iterations: 0,
            convex_convex_minimum_points_perturbation_threshold: 3,
        }
    }

    #[inline]
    pub fn get_num_manifolds(&self) -> usize {
        self.manifolds_ptr.len()
    }
    #[inline]
    pub fn manifold(&self, id: ManifoldId) -> &PersistentManifold {
        self.manifolds[id].as_ref().expect("dangling manifold")
    }
    #[inline]
    pub fn manifold_mut(&mut self, id: ManifoldId) -> &mut PersistentManifold {
        self.manifolds[id].as_mut().expect("dangling manifold")
    }

    /// `getNewManifold(body0, body1)`.
    pub fn get_new_manifold(
        &mut self,
        shapes: &Shapes,
        objects: &ObjArena,
        body0: ObjId,
        body1: ObjId,
    ) -> ManifoldId {
        let b0 = obj(objects, body0);
        let b1 = obj(objects, body1);
        let contact_breaking_threshold =
            if (self.dispatcher_flags & CD_USE_RELATIVE_CONTACT_BREAKING_THRESHOLD) != 0 {
                bt_min(
                    shapes
                        .get(b0.shape())
                        .get_contact_breaking_threshold(G_CONTACT_BREAKING_THRESHOLD),
                    shapes
                        .get(b1.shape())
                        .get_contact_breaking_threshold(G_CONTACT_BREAKING_THRESHOLD),
                )
            } else {
                G_CONTACT_BREAKING_THRESHOLD
            };
        let contact_processing_threshold = bt_min(
            b0.contact_processing_threshold,
            b1.contact_processing_threshold,
        );
        let mut m = PersistentManifold::new(
            body0,
            body1,
            contact_breaking_threshold,
            contact_processing_threshold,
        );
        m.index1a = self.manifolds_ptr.len() as i32;
        let id = if let Some(id) = self.free_manifolds.pop() {
            self.manifolds[id] = Some(m);
            id
        } else {
            self.manifolds.push(Some(m));
            self.manifolds.len() - 1
        };
        self.manifolds_ptr.push(id);
        id
    }

    /// `releaseManifold`: swap-removes from `m_manifoldsPtr`.
    pub fn release_manifold(&mut self, id: ManifoldId) {
        self.manifold_mut(id).clear_manifold();
        let find_index = self.manifold(id).index1a as usize;
        let last = self.manifolds_ptr.len() - 1;
        self.manifolds_ptr.swap(find_index, last);
        let moved = self.manifolds_ptr[find_index];
        self.manifold_mut(moved).index1a = find_index as i32;
        self.manifolds_ptr.pop();
        self.manifolds[id] = None;
        self.free_manifolds.push(id);
    }

    /// `needsResponse(body0, body1)`.
    pub fn needs_response(b0: &CollisionObject, b1: &CollisionObject) -> bool {
        let mut has_response = b0.has_contact_response() && b1.has_contact_response();
        has_response = has_response
            && (!b0.is_static_or_kinematic_object() || !b1.is_static_or_kinematic_object());
        has_response
    }

    /// `needsCollision(body0, body1)`.
    pub fn needs_collision(
        objects: &ObjArena,
        constraints: Option<&ConstraintArena>,
        body0: ObjId,
        body1: ObjId,
    ) -> bool {
        let b0 = obj(objects, body0);
        let b1 = obj(objects, body1);
        if !b0.is_active() && !b1.is_active() {
            return false;
        }
        let empty = ConstraintArena::default();
        let arena = constraints.unwrap_or(&empty);
        if !check_collide_with(|id| obj(objects, id), arena, body0, body1) {
            return false;
        }
        true
    }

    fn alloc_algorithm(&mut self, a: Algorithm) -> AlgId {
        if let Some(id) = self.free_algorithms.pop() {
            self.algorithms[id] = Some(a);
            id
        } else {
            self.algorithms.push(Some(a));
            self.algorithms.len() - 1
        }
    }

    /// `findAlgorithm(body0Wrap, body1Wrap, sharedManifold)`.
    pub fn find_algorithm(
        &mut self,
        ctx: &mut Ctx,
        w0: &ObjWrapper,
        w1: &ObjWrapper,
        shared_manifold: Option<ManifoldId>,
    ) -> AlgId {
        let t0 = ctx.shapes.get(w0.shape).shape_type();
        let t1 = ctx.shapes.get(w1.shape).shape_type();
        let alg = match create_func(t0, t1) {
            CreateFunc::SphereSphere => {
                let (manifold, own_manifold) = match shared_manifold {
                    Some(m) => (Some(m), false),
                    None => (
                        Some(self.get_new_manifold(ctx.shapes, ctx.objects, w0.obj, w1.obj)),
                        true,
                    ),
                };
                Algorithm::SphereSphere {
                    manifold,
                    own_manifold,
                }
            }
            CreateFunc::BoxBox => {
                let mut manifold = shared_manifold;
                let mut own_manifold = false;
                if manifold.is_none()
                    && Self::needs_collision(ctx.objects, ctx.constraints, w0.obj, w1.obj)
                {
                    manifold = Some(self.get_new_manifold(ctx.shapes, ctx.objects, w0.obj, w1.obj));
                    own_manifold = true;
                }
                Algorithm::BoxBox {
                    manifold,
                    own_manifold,
                }
            }
            CreateFunc::ConvexConvex => Algorithm::ConvexConvex {
                manifold: shared_manifold,
                own_manifold: false,
                num_perturbation_iterations: self.convex_convex_num_perturbation_iterations,
                minimum_points_perturbation_threshold: self
                    .convex_convex_minimum_points_perturbation_threshold,
            },
            CreateFunc::CompoundCompound => {
                let r0 = ctx.shapes.get(w0.shape).compound().update_revision;
                let r1 = ctx.shapes.get(w1.shape).compound().update_revision;
                Algorithm::CompoundCompound {
                    cache: HashedSimplePairCache::new(),
                    shared_manifold,
                    compound_shape_revision0: r0,
                    compound_shape_revision1: r1,
                }
            }
            CreateFunc::Compound | CreateFunc::SwappedCompound => {
                let is_swapped = create_func(t0, t1) == CreateFunc::SwappedCompound;
                let col = if is_swapped { w1 } else { w0 };
                let rev = ctx.shapes.get(col.shape).compound().update_revision;
                let children =
                    self.preallocate_child_algorithms(ctx, w0, w1, is_swapped, shared_manifold);
                Algorithm::Compound {
                    child_algorithms: children,
                    is_swapped,
                    shared_manifold,
                    compound_shape_revision: rev,
                }
            }
            CreateFunc::Empty => Algorithm::Empty,
        };
        self.alloc_algorithm(alg)
    }

    /// `btCompoundCollisionAlgorithm::preallocateChildAlgorithms`.
    fn preallocate_child_algorithms(
        &mut self,
        ctx: &mut Ctx,
        w0: &ObjWrapper,
        w1: &ObjWrapper,
        is_swapped: bool,
        shared_manifold: Option<ManifoldId>,
    ) -> Vec<Option<AlgId>> {
        let (col, other) = if is_swapped { (w1, w0) } else { (w0, w1) };
        let compound = ctx.shapes.get(col.shape).compound();
        let n = compound.get_num_child_shapes();
        let has_tree = compound.dynamic_aabb_tree.is_some();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            if has_tree {
                out.push(None);
            } else {
                let child_shape = ctx.shapes.get(col.shape).compound().get_child_shape(i);
                let child_wrap = ObjWrapper {
                    shape: child_shape,
                    obj: col.obj,
                    world_transform: col.world_transform,
                    part_id: -1,
                    index: i as i32,
                };
                out.push(Some(self.find_algorithm(
                    ctx,
                    &child_wrap,
                    other,
                    shared_manifold,
                )));
            }
        }
        out
    }

    /// `~btCollisionAlgorithm()` followed by `freeCollisionAlgorithm`.
    pub fn destroy_algorithm(&mut self, id: AlgId) {
        let alg = self.algorithms[id].take().expect("dangling algorithm");
        match alg {
            Algorithm::ConvexConvex {
                manifold,
                own_manifold,
                ..
            }
            | Algorithm::BoxBox {
                manifold,
                own_manifold,
            }
            | Algorithm::SphereSphere {
                manifold,
                own_manifold,
            } => {
                if own_manifold {
                    if let Some(m) = manifold {
                        self.release_manifold(m);
                    }
                }
            }
            Algorithm::Compound {
                child_algorithms, ..
            } => {
                for c in child_algorithms.into_iter().flatten() {
                    self.destroy_algorithm(c);
                }
            }
            Algorithm::CompoundCompound { cache, .. } => {
                for p in cache.overlapping_pair_array.iter() {
                    if let Some(a) = p.user_pointer {
                        self.destroy_algorithm(a);
                    }
                }
            }
            Algorithm::Empty => {}
        }
        self.free_algorithms.push(id);
    }

    /// `getAllContactManifolds`.
    pub fn get_all_contact_manifolds(&self, id: AlgId, out: &mut Vec<ManifoldId>) {
        match self.algorithms[id].as_ref().expect("dangling algorithm") {
            Algorithm::ConvexConvex {
                manifold,
                own_manifold,
                ..
            }
            | Algorithm::BoxBox {
                manifold,
                own_manifold,
            }
            | Algorithm::SphereSphere {
                manifold,
                own_manifold,
            } => {
                if let (Some(m), true) = (manifold, *own_manifold) {
                    out.push(*m);
                }
            }
            Algorithm::Compound {
                child_algorithms, ..
            } => {
                for c in child_algorithms.iter().flatten() {
                    self.get_all_contact_manifolds(*c, out);
                }
            }
            Algorithm::CompoundCompound { cache, .. } => {
                for p in cache.overlapping_pair_array.iter() {
                    if let Some(a) = p.user_pointer {
                        self.get_all_contact_manifolds(a, out);
                    }
                }
            }
            Algorithm::Empty => {}
        }
    }

    /// `processCollision(body0Wrap, body1Wrap, dispatchInfo, resultOut)`.
    pub fn process_collision(
        &mut self,
        id: AlgId,
        ctx: &mut Ctx,
        w0: &ObjWrapper,
        w1: &ObjWrapper,
        result: &mut ManifoldResult,
    ) {
        let mut alg = self.algorithms[id].take().expect("dangling algorithm");
        match &mut alg {
            Algorithm::ConvexConvex {
                manifold,
                own_manifold,
                num_perturbation_iterations,
                minimum_points_perturbation_threshold,
            } => {
                if manifold.is_none() {
                    *manifold =
                        Some(self.get_new_manifold(ctx.shapes, ctx.objects, w0.obj, w1.obj));
                    *own_manifold = true;
                }
                result.manifold = *manifold;
                self.convex_convex_process(
                    ctx,
                    w0,
                    w1,
                    result,
                    *own_manifold,
                    *num_perturbation_iterations,
                    *minimum_points_perturbation_threshold,
                );
            }
            Algorithm::BoxBox {
                manifold,
                own_manifold,
            } => {
                if let Some(m) = *manifold {
                    result.manifold = Some(m);
                    let input = ClosestPointInput {
                        transform_a: w0.world_transform,
                        transform_b: w1.world_transform,
                        maximum_distance_squared: BT_LARGE_FLOAT,
                    };
                    let shapes = ctx.shapes;
                    {
                        let mut sink = ResultSink {
                            res: result,
                            manifolds: &mut self.manifolds,
                            ctx,
                        };
                        box_box_get_closest_points(
                            shapes.get(w0.shape),
                            shapes.get(w1.shape),
                            &input,
                            &mut sink,
                        );
                    }
                    if *own_manifold {
                        result.refresh_contact_points(&mut self.manifolds, ctx.objects);
                    }
                }
            }
            Algorithm::SphereSphere { manifold, .. } => {
                if let Some(m) = *manifold {
                    result.manifold = Some(m);
                    let diff = w0.world_transform.origin - w1.world_transform.origin;
                    let len = diff.length();
                    let radius0 = ctx.shapes.get(w0.shape).sphere_radius();
                    let radius1 = ctx.shapes.get(w1.shape).sphere_radius();
                    if len > radius0 + radius1 {
                        result.refresh_contact_points(&mut self.manifolds, ctx.objects);
                    } else {
                        let dist = len - (radius0 + radius1);
                        let mut normal_on_surface_b = Vec3::new(1.0, 0.0, 0.0);
                        if len > SIMD_EPSILON {
                            normal_on_surface_b = diff / len;
                        }
                        let pos1 = w1.world_transform.origin + radius1 * normal_on_surface_b;
                        result.add_contact_point(
                            &mut self.manifolds,
                            ctx,
                            &normal_on_surface_b,
                            &pos1,
                            dist,
                        );
                        result.refresh_contact_points(&mut self.manifolds, ctx.objects);
                    }
                }
            }
            Algorithm::Compound {
                child_algorithms,
                is_swapped,
                shared_manifold,
                compound_shape_revision,
            } => {
                self.compound_process(
                    ctx,
                    w0,
                    w1,
                    result,
                    child_algorithms,
                    *is_swapped,
                    *shared_manifold,
                    *compound_shape_revision,
                );
            }
            Algorithm::CompoundCompound {
                cache,
                shared_manifold,
                compound_shape_revision0,
                compound_shape_revision1,
            } => {
                self.compound_compound_process(
                    ctx,
                    w0,
                    w1,
                    result,
                    cache,
                    *shared_manifold,
                    *compound_shape_revision0,
                    *compound_shape_revision1,
                );
            }
            Algorithm::Empty => {}
        }
        self.algorithms[id] = Some(alg);
    }

    /// `btConvexConvexAlgorithm::processCollision` after the manifold is set.
    #[allow(clippy::too_many_arguments)]
    fn convex_convex_process(
        &mut self,
        ctx: &mut Ctx,
        w0: &ObjWrapper,
        w1: &ObjWrapper,
        result: &mut ManifoldResult,
        own_manifold: bool,
        num_perturbation_iterations: i32,
        minimum_points_perturbation_threshold: i32,
    ) {
        let shapes = ctx.shapes;
        let min0 = shapes.get(w0.shape);
        let min1 = shapes.get(w1.shape);
        let mid = result.manifold.expect("no manifold");
        if let (ShapeKind::Capsule(capsule_a), ShapeKind::Capsule(capsule_b)) =
            (&min0.kind, &min1.kind)
        {
            // The specialised capsule-capsule collider (always refreshes).
            let threshold = self.manifold(mid).get_contact_breaking_threshold();
            let (dist, normal_on_b, point_on_b_world) = capsule_capsule_distance(
                capsule_a.get_half_height(),
                capsule_a.get_radius(),
                capsule_b.get_half_height(),
                capsule_b.get_radius(),
                capsule_a.up_axis,
                capsule_b.up_axis,
                &w0.world_transform,
                &w1.world_transform,
                threshold,
            );
            if dist < threshold {
                result.add_contact_point(
                    &mut self.manifolds,
                    ctx,
                    &normal_on_b,
                    &point_on_b_world,
                    dist,
                );
            }
            result.refresh_contact_points(&mut self.manifolds, ctx.objects);
            return;
        }
        let mut input = ClosestPointInput::default();
        {
            let x = min0.get_margin()
                + min1.get_margin()
                + self.manifold(mid).get_contact_breaking_threshold();
            input.maximum_distance_squared = x * x;
        }
        input.transform_a = w0.world_transform;
        input.transform_b = w1.world_transform;
        // Polyhedral shapes have no btConvexPolyhedron (initializePolyhedralFeatures
        // is never called) and there are no triangle shapes, so the polyhedral
        // branch falls through to plain GJK.
        let (cached_axis, num_contacts);
        {
            let mut gjk = GjkPairDetector::new(
                min0,
                min1,
                &mut self.simplex_solver,
                Some(PenetrationDepthSolver::GjkEpa),
            );
            {
                let mut sink = ResultSink {
                    res: result,
                    manifolds: &mut self.manifolds,
                    ctx,
                };
                gjk.get_closest_points(&input, &mut sink);
            }
            cached_axis = gjk.cached_separating_axis;
            num_contacts = self.manifolds[mid].as_ref().unwrap().get_num_contacts();
            if num_perturbation_iterations != 0
                && num_contacts < minimum_points_perturbation_threshold
            {
                let mut v0 = Vec3::ZERO;
                let mut v1 = Vec3::ZERO;
                let l2 = cached_axis.length2();
                if l2 > SIMD_EPSILON {
                    let sep_normal_world_space = cached_axis * (flt(1.0) / l2);
                    plane_space1(&sep_normal_world_space, &mut v0, &mut v1);
                    let angle_limit = flt(0.125) * SIMD_PI;
                    let radius_a = min0.get_angular_motion_disc();
                    let radius_b = min1.get_angular_motion_disc();
                    let mut perturbe_angle;
                    let perturbe_a;
                    if radius_a < radius_b {
                        perturbe_angle = G_CONTACT_BREAKING_THRESHOLD / radius_a;
                        perturbe_a = true;
                    } else {
                        perturbe_angle = G_CONTACT_BREAKING_THRESHOLD / radius_b;
                        perturbe_a = false;
                    }
                    if perturbe_angle > angle_limit {
                        perturbe_angle = angle_limit;
                    }
                    let un_perturbed_transform = if perturbe_a {
                        input.transform_a
                    } else {
                        input.transform_b
                    };
                    for i in 0..num_perturbation_iterations {
                        if v0.length2() > SIMD_EPSILON {
                            let perturbe_rot = Quat::from_axis_angle(&v0, perturbe_angle);
                            let iteration_angle =
                                i as f64 * (SIMD_2_PI / num_perturbation_iterations as f64);
                            let rotq =
                                Quat::from_axis_angle(&sep_normal_world_space, iteration_angle);
                            if perturbe_a {
                                input.transform_a.basis =
                                    Mat3::from_quat(&(rotq.inverse() * perturbe_rot * rotq))
                                        * w0.world_transform.basis;
                                input.transform_b = w1.world_transform;
                            } else {
                                input.transform_a = w0.world_transform;
                                input.transform_b.basis =
                                    Mat3::from_quat(&(rotq.inverse() * perturbe_rot * rotq))
                                        * w1.world_transform.basis;
                            }
                            let mut sink = PerturbedSink {
                                inner: ResultSink {
                                    res: result,
                                    manifolds: &mut self.manifolds,
                                    ctx,
                                },
                                transform_a: input.transform_a,
                                transform_b: input.transform_b,
                                un_perturbed_transform,
                                perturb_a: perturbe_a,
                            };
                            gjk.get_closest_points(&input, &mut sink);
                        }
                    }
                }
            }
        }
        if own_manifold {
            result.refresh_contact_points(&mut self.manifolds, ctx.objects);
        }
    }

    /// `btCompoundCollisionAlgorithm::processCollision`.
    #[allow(clippy::too_many_arguments)]
    fn compound_process(
        &mut self,
        ctx: &mut Ctx,
        w0: &ObjWrapper,
        w1: &ObjWrapper,
        result: &mut ManifoldResult,
        child_algorithms: &mut Vec<Option<AlgId>>,
        is_swapped: bool,
        shared_manifold: Option<ManifoldId>,
        compound_shape_revision: i32,
    ) {
        let shapes = ctx.shapes;
        let (col, other) = if is_swapped { (*w1, *w0) } else { (*w0, *w1) };
        let compound = shapes.get(col.shape).compound();
        if compound.update_revision != compound_shape_revision {
            for c in child_algorithms.iter().flatten() {
                self.destroy_algorithm(*c);
            }
            *child_algorithms =
                self.preallocate_child_algorithms(ctx, w0, w1, is_swapped, shared_manifold);
        }
        // refresh the child manifolds
        {
            let mut manifold_array = Vec::new();
            for c in child_algorithms.iter().flatten() {
                self.get_all_contact_manifolds(*c, &mut manifold_array);
                for m in manifold_array.iter() {
                    if self.manifold(*m).get_num_contacts() != 0 {
                        result.manifold = Some(*m);
                        result.refresh_contact_points(&mut self.manifolds, ctx.objects);
                        result.manifold = None;
                    }
                }
                manifold_array.clear();
            }
        }
        if let Some(tree) = &compound.dynamic_aabb_tree {
            let other_in_compound_space = col.world_transform.inverse() * other.world_transform;
            let (local_aabb_min, local_aabb_max) =
                shapes.get(other.shape).get_aabb(&other_in_compound_space);
            let bounds = DbvtAabbMm::from_mm(&local_aabb_min, &local_aabb_max);
            let mut leaves = Vec::new();
            tree.collide_tv(tree.root, &bounds, &mut |n| leaves.push(tree.node(n).data));
            for index in leaves {
                let child_shape = compound.get_child_shape(index);
                self.compound_process_child(
                    ctx,
                    &col,
                    &other,
                    result,
                    child_algorithms,
                    shared_manifold,
                    child_shape,
                    index,
                );
            }
        } else {
            for i in 0..child_algorithms.len() {
                let child_shape = compound.get_child_shape(i);
                self.compound_process_child(
                    ctx,
                    &col,
                    &other,
                    result,
                    child_algorithms,
                    shared_manifold,
                    child_shape,
                    i,
                );
            }
        }
        // remove child algorithms whose AABBs no longer overlap
        for i in 0..child_algorithms.len() {
            if let Some(c) = child_algorithms[i] {
                let child_shape = compound.get_child_shape(i);
                let org_trans = col.world_transform;
                let child_trans = compound.get_child_transform(i);
                let new_child_world_trans = org_trans * *child_trans;
                let (aabb_min0, aabb_max0) =
                    shapes.get(child_shape).get_aabb(&new_child_world_trans);
                let (aabb_min1, aabb_max1) =
                    shapes.get(other.shape).get_aabb(&other.world_transform);
                if !test_aabb_against_aabb2(&aabb_min0, &aabb_max0, &aabb_min1, &aabb_max1) {
                    self.destroy_algorithm(c);
                    child_algorithms[i] = None;
                }
            }
        }
    }

    /// `btCompoundLeafCallback::ProcessChildShape`.
    #[allow(clippy::too_many_arguments)]
    fn compound_process_child(
        &mut self,
        ctx: &mut Ctx,
        col: &ObjWrapper,
        other: &ObjWrapper,
        result: &mut ManifoldResult,
        child_algorithms: &mut [Option<AlgId>],
        shared_manifold: Option<ManifoldId>,
        child_shape: ShapeId,
        index: usize,
    ) {
        let shapes = ctx.shapes;
        let compound = shapes.get(col.shape).compound();
        let org_trans = col.world_transform;
        let child_trans = compound.get_child_transform(index);
        let new_child_world_trans = org_trans * *child_trans;
        let (aabb_min0, aabb_max0) = shapes.get(child_shape).get_aabb(&new_child_world_trans);
        let (aabb_min1, aabb_max1) = shapes.get(other.shape).get_aabb(&other.world_transform);
        if test_aabb_against_aabb2(&aabb_min0, &aabb_max0, &aabb_min1, &aabb_max1) {
            let compound_wrap = ObjWrapper {
                shape: child_shape,
                obj: col.obj,
                world_transform: new_child_world_trans,
                part_id: -1,
                index: index as i32,
            };
            if child_algorithms[index].is_none() {
                child_algorithms[index] =
                    Some(self.find_algorithm(ctx, &compound_wrap, other, shared_manifold));
            }
            let tmp_wrap;
            let on_body0 = result.body0.obj == col.obj;
            if on_body0 {
                tmp_wrap = result.body0;
                result.body0 = compound_wrap;
                result.part_id0 = -1;
                result.index0 = index as i32;
            } else {
                tmp_wrap = result.body1;
                result.body1 = compound_wrap;
                result.part_id1 = -1;
                result.index1 = index as i32;
            }
            let c = child_algorithms[index].unwrap();
            self.process_collision(c, ctx, &compound_wrap, other, result);
            if result.body0.obj == col.obj {
                result.body0 = tmp_wrap;
            } else {
                result.body1 = tmp_wrap;
            }
        }
    }

    /// `btCompoundCompoundCollisionAlgorithm::processCollision`.
    #[allow(clippy::too_many_arguments)]
    fn compound_compound_process(
        &mut self,
        ctx: &mut Ctx,
        w0: &ObjWrapper,
        w1: &ObjWrapper,
        result: &mut ManifoldResult,
        cache: &mut HashedSimplePairCache,
        shared_manifold: Option<ManifoldId>,
        rev0: i32,
        rev1: i32,
    ) {
        let shapes = ctx.shapes;
        let compound0 = shapes.get(w0.shape).compound();
        let compound1 = shapes.get(w1.shape).compound();
        if compound0.update_revision != rev0 || compound1.update_revision != rev1 {
            for p in cache.overlapping_pair_array.iter() {
                if let Some(a) = p.user_pointer {
                    self.destroy_algorithm(a);
                }
            }
            cache.remove_all_pairs();
        }
        {
            let mut manifold_array = Vec::new();
            for i in 0..cache.overlapping_pair_array.len() {
                if let Some(a) = cache.overlapping_pair_array[i].user_pointer {
                    self.get_all_contact_manifolds(a, &mut manifold_array);
                    for m in manifold_array.iter() {
                        if self.manifold(*m).get_num_contacts() != 0 {
                            result.manifold = Some(*m);
                            result.refresh_contact_points(&mut self.manifolds, ctx.objects);
                            result.manifold = None;
                        }
                    }
                    manifold_array.clear();
                }
            }
        }
        let tree0 = compound0
            .dynamic_aabb_tree
            .as_ref()
            .expect("compound without tree");
        let tree1 = compound1
            .dynamic_aabb_tree
            .as_ref()
            .expect("compound without tree");
        let xform = w0.world_transform.inverse() * w1.world_transform;
        let pairs = my_collide_tt(tree0, tree1, &xform);
        for (l0, l1) in pairs {
            let child_index0 = tree0.node(l0).data;
            let child_index1 = tree1.node(l1).data;
            let child_shape0 = compound0.get_child_shape(child_index0);
            let child_shape1 = compound1.get_child_shape(child_index1);
            let new_child_world_trans0 =
                w0.world_transform * *compound0.get_child_transform(child_index0);
            let new_child_world_trans1 =
                w1.world_transform * *compound1.get_child_transform(child_index1);
            let (aabb_min0, aabb_max0) = shapes.get(child_shape0).get_aabb(&new_child_world_trans0);
            let (aabb_min1, aabb_max1) = shapes.get(child_shape1).get_aabb(&new_child_world_trans1);
            if test_aabb_against_aabb2(&aabb_min0, &aabb_max0, &aabb_min1, &aabb_max1) {
                let compound_wrap0 = ObjWrapper {
                    shape: child_shape0,
                    obj: w0.obj,
                    world_transform: new_child_world_trans0,
                    part_id: -1,
                    index: child_index0 as i32,
                };
                let compound_wrap1 = ObjWrapper {
                    shape: child_shape1,
                    obj: w1.obj,
                    world_transform: new_child_world_trans1,
                    part_id: -1,
                    index: child_index1 as i32,
                };
                let col_algo = match cache.find_pair(child_index0 as i32, child_index1 as i32) {
                    Some(pi) => cache.overlapping_pair_array[pi]
                        .user_pointer
                        .expect("pair without algorithm"),
                    None => {
                        let a = self.find_algorithm(
                            ctx,
                            &compound_wrap0,
                            &compound_wrap1,
                            shared_manifold,
                        );
                        let pi =
                            cache.add_overlapping_pair(child_index0 as i32, child_index1 as i32);
                        cache.overlapping_pair_array[pi].user_pointer = Some(a);
                        a
                    }
                };
                let tmp_wrap0 = result.body0;
                let tmp_wrap1 = result.body1;
                result.body0 = compound_wrap0;
                result.body1 = compound_wrap1;
                result.part_id0 = -1;
                result.index0 = child_index0 as i32;
                result.part_id1 = -1;
                result.index1 = child_index1 as i32;
                self.process_collision(col_algo, ctx, &compound_wrap0, &compound_wrap1, result);
                result.body0 = tmp_wrap0;
                result.body1 = tmp_wrap1;
            }
        }
        let mut remove_pairs: Vec<(i32, i32)> = Vec::new();
        for i in 0..cache.overlapping_pair_array.len() {
            let p = cache.overlapping_pair_array[i];
            if let Some(a) = p.user_pointer {
                let t0 = w0.world_transform * *compound0.get_child_transform(p.index_a as usize);
                let (aabb_min0, aabb_max0) = shapes
                    .get(compound0.get_child_shape(p.index_a as usize))
                    .get_aabb(&t0);
                let t1 = w1.world_transform * *compound1.get_child_transform(p.index_b as usize);
                let (aabb_min1, aabb_max1) = shapes
                    .get(compound1.get_child_shape(p.index_b as usize))
                    .get_aabb(&t1);
                if !test_aabb_against_aabb2(&aabb_min0, &aabb_max0, &aabb_min1, &aabb_max1) {
                    self.destroy_algorithm(a);
                    remove_pairs.push((p.index_a, p.index_b));
                }
            }
        }
        for (a, b) in remove_pairs {
            cache.remove_overlapping_pair(a, b);
        }
    }

    /// `dispatchAllCollisionPairs` with `defaultNearCallback`, discrete only.
    pub fn dispatch_all_collision_pairs(
        &mut self,
        pairs: &mut [super::broadphase::pair_cache::BroadphasePair],
        proxy_client: &dyn Fn(super::collision_object::ProxyId) -> ObjId,
        ctx: &mut Ctx,
    ) {
        for pair in pairs.iter_mut() {
            let o0 = proxy_client(pair.proxy0);
            let o1 = proxy_client(pair.proxy1);
            if Self::needs_collision(ctx.objects, ctx.constraints, o0, o1) {
                let c0 = obj(ctx.objects, o0);
                let c1 = obj(ctx.objects, o1);
                let w0 = ObjWrapper {
                    shape: c0.shape(),
                    obj: o0,
                    world_transform: c0.world_transform,
                    part_id: -1,
                    index: -1,
                };
                let w1 = ObjWrapper {
                    shape: c1.shape(),
                    obj: o1,
                    world_transform: c1.world_transform,
                    part_id: -1,
                    index: -1,
                };
                if pair.algorithm.is_none() {
                    pair.algorithm = Some(self.find_algorithm(ctx, &w0, &w1, None));
                }
                if let Some(a) = pair.algorithm {
                    let mut result = ManifoldResult::new(w0, w1);
                    if ctx.info.dispatch_func == DISPATCH_DISCRETE {
                        self.process_collision(a, ctx, &w0, &w1, &mut result);
                    }
                }
            }
        }
    }
}

impl PairDispatcher for CollisionDispatcher {
    fn free_collision_algorithm(&mut self, algorithm: usize) {
        self.destroy_algorithm(algorithm);
    }
}

/// `MyIntersect`: `a` against `b` moved by `xform`.
fn my_intersect(a: &DbvtAabbMm, b: &DbvtAabbMm, xform: &Transform) -> bool {
    let (newmin, newmax) = transform_aabb(&b.mi, &b.mx, 0.0, xform);
    let newb = DbvtAabbMm::from_mm(&newmin, &newmax);
    intersect(a, &newb)
}

/// `MycollideTT`: leaf pairs of two compound trees, in callback order.
fn my_collide_tt(t0: &Dbvt, t1: &Dbvt, xform: &Transform) -> Vec<(NodeId, NodeId)> {
    let mut out = Vec::new();
    let (root0, root1) = (t0.root, t1.root);
    if root0 == NIL || root1 == NIL {
        return out;
    }
    let mut stack: Vec<(NodeId, NodeId)> = vec![(root0, root1)];
    while let Some((a, b)) = stack.pop() {
        let na = t0.node(a);
        let nb = t1.node(b);
        if my_intersect(&na.volume, &nb.volume, xform) {
            if na.is_internal() {
                if nb.is_internal() {
                    stack.push((na.childs[0], nb.childs[0]));
                    stack.push((na.childs[1], nb.childs[0]));
                    stack.push((na.childs[0], nb.childs[1]));
                    stack.push((na.childs[1], nb.childs[1]));
                } else {
                    stack.push((na.childs[0], b));
                    stack.push((na.childs[1], b));
                }
            } else if nb.is_internal() {
                stack.push((a, nb.childs[0]));
                stack.push((a, nb.childs[1]));
            } else {
                out.push((a, b));
            }
        }
    }
    out
}
