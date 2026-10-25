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
// Altered for pz-rs: Rust translation of btCollisionWorld (object list,
// AABB updates, discrete collision detection, ray tests and convex sweeps)
// from Bullet 2.82, double precision.

//! `btCollisionWorld`.
//!
//! The world owns the object arena, the shape arena, the broadphase and the
//! dispatcher. An object can stay in the arena while it is not in the world
//! (the game pools removed bodies); [`CollisionWorld::collision_objects`] is
//! Bullet's ordered `m_collisionObjects`.

use super::broadphase::dbvt_broadphase::{BroadphaseRayCallback, DbvtBroadphase};
use super::broadphase::pair_cache::BroadphaseProxy;
use super::collision_object::*;
use super::dispatch::*;
use super::dynamics::typed_constraint::ConstraintArena;
use super::linear_math::transform_util::calculate_velocity;
use super::linear_math::*;
use super::manifold::G_CONTACT_BREAKING_THRESHOLD;
use super::narrowphase::convex_cast::{
    continuous_convex_collision, gjk_convex_cast, subsimplex_convex_cast, CastResult,
};
use super::narrowphase::gjk_pair_detector::PenetrationDepthSolver;
use super::narrowphase::simplex::VoronoiSimplexSolver;
use super::shapes::*;

/// `btBroadphaseProxy::CollisionFilterGroups`.
pub const DEFAULT_FILTER: i16 = 1;
pub const STATIC_FILTER: i16 = 2;
pub const KINEMATIC_FILTER: i16 = 4;
pub const DEBRIS_FILTER: i16 = 8;
pub const SENSOR_TRIGGER: i16 = 16;
pub const CHARACTER_FILTER: i16 = 32;
pub const ALL_FILTER: i16 = -1;

/// `btTriangleRaycastCallback::kF_UseSubSimplexConvexCastRaytest`.
pub const KF_USE_SUB_SIMPLEX_CONVEX_CAST_RAYTEST: u32 = 4;

/// `btCollisionWorld::LocalShapeInfo`.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalShapeInfo {
    pub shape_part: i32,
    pub triangle_index: i32,
}

/// `btCollisionWorld::LocalRayResult`.
#[derive(Clone, Copy, Debug)]
pub struct LocalRayResult {
    pub collision_object: ObjId,
    pub local_shape_info: Option<LocalShapeInfo>,
    pub hit_normal_local: Vec3,
    pub hit_fraction: f64,
}

/// `btCollisionWorld::LocalConvexResult`.
#[derive(Clone, Copy, Debug)]
pub struct LocalConvexResult {
    pub hit_collision_object: ObjId,
    pub local_shape_info: Option<LocalShapeInfo>,
    pub hit_normal_local: Vec3,
    pub hit_point_local: Vec3,
    pub hit_fraction: f64,
}

/// `RayResultCallback`'s data members.
#[derive(Clone, Copy, Debug)]
pub struct RayResultBase {
    pub closest_hit_fraction: f64,
    pub collision_object: Option<ObjId>,
    pub collision_filter_group: i16,
    pub collision_filter_mask: i16,
    pub flags: u32,
}

impl Default for RayResultBase {
    fn default() -> Self {
        RayResultBase {
            closest_hit_fraction: 1.0,
            collision_object: None,
            collision_filter_group: DEFAULT_FILTER,
            collision_filter_mask: ALL_FILTER,
            flags: 0,
        }
    }
}

/// The default `needsCollision` filter test of the result callbacks.
#[inline]
pub fn default_needs_collision(group: i16, mask: i16, proxy: &BroadphaseProxy) -> bool {
    let mut collides = (proxy.collision_filter_group & mask) != 0;
    collides = collides && (group & proxy.collision_filter_mask) != 0;
    collides
}

/// `btCollisionWorld::RayResultCallback`.
pub trait RayResultCallback {
    fn base(&self) -> &RayResultBase;
    fn base_mut(&mut self) -> &mut RayResultBase;
    fn needs_collision(&self, proxy: &BroadphaseProxy) -> bool {
        let b = self.base();
        default_needs_collision(b.collision_filter_group, b.collision_filter_mask, proxy)
    }
    fn add_single_result(
        &mut self,
        r: &mut LocalRayResult,
        normal_in_world_space: bool,
        objects: &ObjArena,
    ) -> f64;
    fn has_hit(&self) -> bool {
        self.base().collision_object.is_some()
    }
}

/// `btCollisionWorld::ClosestRayResultCallback`.
#[derive(Clone, Copy, Debug)]
pub struct ClosestRayResultCallback {
    pub base: RayResultBase,
    pub ray_from_world: Vec3,
    pub ray_to_world: Vec3,
    pub hit_normal_world: Vec3,
    pub hit_point_world: Vec3,
}

impl ClosestRayResultCallback {
    pub fn new(ray_from_world: Vec3, ray_to_world: Vec3) -> ClosestRayResultCallback {
        ClosestRayResultCallback {
            base: RayResultBase::default(),
            ray_from_world,
            ray_to_world,
            hit_normal_world: Vec3::ZERO,
            hit_point_world: Vec3::ZERO,
        }
    }
}

impl RayResultCallback for ClosestRayResultCallback {
    fn base(&self) -> &RayResultBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut RayResultBase {
        &mut self.base
    }
    fn add_single_result(
        &mut self,
        r: &mut LocalRayResult,
        normal_in_world_space: bool,
        objects: &ObjArena,
    ) -> f64 {
        self.base.closest_hit_fraction = r.hit_fraction;
        self.base.collision_object = Some(r.collision_object);
        if normal_in_world_space {
            self.hit_normal_world = r.hit_normal_local;
        } else {
            self.hit_normal_world =
                obj(objects, r.collision_object).world_transform.basis * r.hit_normal_local;
        }
        self.hit_point_world.set_interpolate3(
            &self.ray_from_world,
            &self.ray_to_world,
            r.hit_fraction,
        );
        r.hit_fraction
    }
}

/// `ConvexResultCallback`'s data members.
#[derive(Clone, Copy, Debug)]
pub struct ConvexResultBase {
    pub closest_hit_fraction: f64,
    pub collision_filter_group: i16,
    pub collision_filter_mask: i16,
}

impl Default for ConvexResultBase {
    fn default() -> Self {
        ConvexResultBase {
            closest_hit_fraction: 1.0,
            collision_filter_group: DEFAULT_FILTER,
            collision_filter_mask: ALL_FILTER,
        }
    }
}

/// `btCollisionWorld::ConvexResultCallback`.
pub trait ConvexResultCallback {
    fn base(&self) -> &ConvexResultBase;
    fn base_mut(&mut self) -> &mut ConvexResultBase;
    fn needs_collision(&self, proxy: &BroadphaseProxy, _objects: &ObjArena) -> bool {
        let b = self.base();
        default_needs_collision(b.collision_filter_group, b.collision_filter_mask, proxy)
    }
    fn add_single_result(
        &mut self,
        r: &mut LocalConvexResult,
        normal_in_world_space: bool,
        objects: &ObjArena,
    ) -> f64;
    fn has_hit(&self) -> bool {
        self.base().closest_hit_fraction < 1.0
    }
}

/// `btCollisionWorld::ClosestConvexResultCallback`.
#[derive(Clone, Copy, Debug)]
pub struct ClosestConvexResultCallback {
    pub base: ConvexResultBase,
    pub convex_from_world: Vec3,
    pub convex_to_world: Vec3,
    pub hit_normal_world: Vec3,
    pub hit_point_world: Vec3,
    pub hit_collision_object: Option<ObjId>,
}

impl ClosestConvexResultCallback {
    pub fn new(convex_from_world: Vec3, convex_to_world: Vec3) -> ClosestConvexResultCallback {
        ClosestConvexResultCallback {
            base: ConvexResultBase::default(),
            convex_from_world,
            convex_to_world,
            hit_normal_world: Vec3::ZERO,
            hit_point_world: Vec3::ZERO,
            hit_collision_object: None,
        }
    }
    /// `ClosestConvexResultCallback::addSingleResult`.
    pub fn closest_add_single_result(
        &mut self,
        r: &LocalConvexResult,
        normal_in_world_space: bool,
        objects: &ObjArena,
    ) -> f64 {
        self.base.closest_hit_fraction = r.hit_fraction;
        self.hit_collision_object = Some(r.hit_collision_object);
        if normal_in_world_space {
            self.hit_normal_world = r.hit_normal_local;
        } else {
            self.hit_normal_world =
                obj(objects, r.hit_collision_object).world_transform.basis * r.hit_normal_local;
        }
        self.hit_point_world = r.hit_point_local;
        r.hit_fraction
    }
}

impl ConvexResultCallback for ClosestConvexResultCallback {
    fn base(&self) -> &ConvexResultBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ConvexResultBase {
        &mut self.base
    }
    fn add_single_result(
        &mut self,
        r: &mut LocalConvexResult,
        normal_in_world_space: bool,
        objects: &ObjArena,
    ) -> f64 {
        self.closest_add_single_result(r, normal_in_world_space, objects)
    }
}

/// `LocalInfoAdder2` (compound children of a ray test).
struct LocalInfoAdder2<'u> {
    base: RayResultBase,
    user: &'u mut dyn RayResultCallback,
    i: i32,
}

impl RayResultCallback for LocalInfoAdder2<'_> {
    fn base(&self) -> &RayResultBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut RayResultBase {
        &mut self.base
    }
    fn needs_collision(&self, proxy: &BroadphaseProxy) -> bool {
        self.user.needs_collision(proxy)
    }
    fn add_single_result(&mut self, r: &mut LocalRayResult, b: bool, objects: &ObjArena) -> f64 {
        if r.local_shape_info.is_none() {
            r.local_shape_info = Some(LocalShapeInfo {
                shape_part: -1,
                triangle_index: self.i,
            });
        }
        let result = self.user.add_single_result(r, b, objects);
        self.base.closest_hit_fraction = self.user.base().closest_hit_fraction;
        result
    }
}

/// `LocalInfoAdder` (compound children of a convex sweep).
struct LocalInfoAdder<'u> {
    base: ConvexResultBase,
    user: &'u mut dyn ConvexResultCallback,
    i: i32,
}

impl ConvexResultCallback for LocalInfoAdder<'_> {
    fn base(&self) -> &ConvexResultBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ConvexResultBase {
        &mut self.base
    }
    fn needs_collision(&self, proxy: &BroadphaseProxy, objects: &ObjArena) -> bool {
        self.user.needs_collision(proxy, objects)
    }
    fn add_single_result(&mut self, r: &mut LocalConvexResult, b: bool, objects: &ObjArena) -> f64 {
        if r.local_shape_info.is_none() {
            r.local_shape_info = Some(LocalShapeInfo {
                shape_part: -1,
                triangle_index: self.i,
            });
        }
        let result = self.user.add_single_result(r, b, objects);
        self.base.closest_hit_fraction = self.user.base().closest_hit_fraction;
        result
    }
}

/// `btCollisionWorld` (with the collision configuration and dispatcher the
/// game builds).
pub struct CollisionWorld {
    /// Object arena (objects outside the world may stay here).
    pub objects: ObjArena,
    free_objects: Vec<ObjId>,
    /// `m_collisionObjects`.
    pub collision_objects: Vec<ObjId>,
    pub shapes: Shapes,
    pub dispatcher: CollisionDispatcher,
    pub broadphase: DbvtBroadphase,
    pub dispatch_info: DispatcherInfo,
    pub force_update_all_aabbs: bool,
}

impl Default for CollisionWorld {
    fn default() -> Self {
        CollisionWorld::new()
    }
}

impl CollisionWorld {
    pub fn new() -> CollisionWorld {
        CollisionWorld {
            objects: Vec::new(),
            free_objects: Vec::new(),
            collision_objects: Vec::new(),
            shapes: Shapes::new(),
            dispatcher: CollisionDispatcher::new(),
            broadphase: DbvtBroadphase::new(),
            dispatch_info: DispatcherInfo::default(),
            force_update_all_aabbs: true,
        }
    }

    /// Stores an object in the arena (not yet in the world).
    pub fn insert_object(&mut self, o: CollisionObject) -> ObjId {
        if let Some(id) = self.free_objects.pop() {
            self.objects[id] = Some(o);
            id
        } else {
            self.objects.push(Some(o));
            self.objects.len() - 1
        }
    }
    /// Deletes an object from the arena; it must not be in the world.
    pub fn delete_object(&mut self, id: ObjId) -> Option<CollisionObject> {
        let o = self.objects[id].take();
        if o.is_some() {
            self.free_objects.push(id);
        }
        o
    }
    #[inline]
    pub fn obj(&self, id: ObjId) -> &CollisionObject {
        obj(&self.objects, id)
    }
    #[inline]
    pub fn obj_mut(&mut self, id: ObjId) -> &mut CollisionObject {
        self.objects[id].as_mut().expect("dangling object id")
    }
    pub fn get_num_collision_objects(&self) -> usize {
        self.collision_objects.len()
    }

    /// `addCollisionObject(obj, group, mask)`.
    pub fn add_collision_object(&mut self, id: ObjId, group: i16, mask: i16) {
        self.collision_objects.push(id);
        let o = obj(&self.objects, id);
        let trans = o.world_transform;
        let (min_aabb, max_aabb) = self.shapes.get(o.shape()).get_aabb(&trans);
        let proxy = self
            .broadphase
            .create_proxy(&min_aabb, &max_aabb, id, group, mask);
        self.obj_mut(id).broadphase_handle = Some(proxy);
    }

    /// `removeCollisionObject(obj)` (swap-removes from the object list).
    pub fn remove_collision_object(&mut self, id: ObjId) {
        if let Some(bp) = self.obj(id).broadphase_handle {
            self.broadphase
                .paircache
                .clean_proxy_from_pairs(bp, Some(&mut self.dispatcher));
            self.broadphase
                .destroy_proxy(bp, Some(&mut self.dispatcher));
            self.obj_mut(id).broadphase_handle = None;
        }
        if let Some(i) = self.collision_objects.iter().position(|&o| o == id) {
            let last = self.collision_objects.len() - 1;
            self.collision_objects.swap(i, last);
            self.collision_objects.pop();
        }
    }

    /// `updateSingleAabb(colObj)`.
    pub fn update_single_aabb(&mut self, id: ObjId) {
        let o = obj(&self.objects, id);
        let shape = self.shapes.get(o.shape());
        let (mut min_aabb, mut max_aabb) = shape.get_aabb(&o.world_transform);
        let t = G_CONTACT_BREAKING_THRESHOLD;
        let contact_threshold = Vec3::new(t, t, t);
        min_aabb -= contact_threshold;
        max_aabb += contact_threshold;
        if self.dispatch_info.use_continuous
            && o.internal_type == CO_RIGID_BODY
            && !o.is_static_or_kinematic_object()
        {
            let (mut min_aabb2, mut max_aabb2) = shape.get_aabb(&o.interpolation_world_transform);
            min_aabb2 -= contact_threshold;
            max_aabb2 += contact_threshold;
            min_aabb.set_min(&min_aabb2);
            max_aabb.set_max(&max_aabb2);
        }
        if o.is_static_object() || (max_aabb - min_aabb).length2() < 1e12 {
            let bp = o.broadphase_handle.expect("object without proxy");
            self.broadphase.set_aabb(bp, &min_aabb, &max_aabb);
        } else {
            self.obj_mut(id).set_activation_state(DISABLE_SIMULATION);
        }
    }

    /// `updateAabbs()`.
    pub fn update_aabbs(&mut self) {
        for i in 0..self.collision_objects.len() {
            let id = self.collision_objects[i];
            if self.force_update_all_aabbs || self.obj(id).is_active() {
                self.update_single_aabb(id);
            }
        }
    }

    /// `performDiscreteCollisionDetection()`.
    pub fn perform_discrete_collision_detection(
        &mut self,
        constraints: Option<&ConstraintArena>,
        hook: &mut dyn ContactAddedHook,
    ) {
        self.update_aabbs();
        self.broadphase
            .calculate_overlapping_pairs(Some(&mut self.dispatcher));
        let CollisionWorld {
            objects,
            shapes,
            dispatcher,
            broadphase,
            dispatch_info,
            ..
        } = self;
        let mut ctx = Ctx {
            shapes,
            objects,
            constraints,
            info: dispatch_info,
            hook,
        };
        let proxies = &broadphase.proxies;
        let client = |p: ProxyId| {
            proxies[p]
                .as_ref()
                .expect("dangling proxy")
                .base
                .client_object
        };
        dispatcher.dispatch_all_collision_pairs(
            &mut broadphase.paircache.overlapping_pair_array,
            &client,
            &mut ctx,
        );
    }

    /// `rayTest(rayFromWorld, rayToWorld, resultCallback)`.
    pub fn ray_test(
        &self,
        ray_from_world: &Vec3,
        ray_to_world: &Vec3,
        cb: &mut dyn RayResultCallback,
    ) {
        let mut ray_cb = SingleRayCallback::new(ray_from_world, ray_to_world, self, cb);
        let zero = Vec3::ZERO;
        self.broadphase
            .ray_test(ray_from_world, ray_to_world, &mut ray_cb, &zero, &zero);
    }

    /// `rayTestSingle`.
    pub fn ray_test_single(
        &self,
        ray_from_trans: &Transform,
        ray_to_trans: &Transform,
        collision_object: ObjId,
        collision_shape: ShapeId,
        col_obj_world_transform: &Transform,
        cb: &mut dyn RayResultCallback,
    ) {
        let w = ObjWrapper {
            shape: collision_shape,
            obj: collision_object,
            world_transform: *col_obj_world_transform,
            part_id: -1,
            index: -1,
        };
        self.ray_test_single_internal(ray_from_trans, ray_to_trans, &w, cb);
    }

    /// `rayTestSingleInternal`.
    pub fn ray_test_single_internal(
        &self,
        ray_from_trans: &Transform,
        ray_to_trans: &Transform,
        w: &ObjWrapper,
        cb: &mut dyn RayResultCallback,
    ) {
        let mut point_shape = Shape::new_sphere(0.0);
        point_shape.set_margin(0.0);
        let collision_shape = self.shapes.get(w.shape);
        let col_obj_world_transform = w.world_transform;
        if collision_shape.is_convex() {
            let mut cast_result = CastResult {
                fraction: cb.base().closest_hit_fraction,
                ..Default::default()
            };
            let mut simplex_solver = VoronoiSimplexSolver::new();
            let hit = if cb.base().flags & KF_USE_SUB_SIMPLEX_CONVEX_CAST_RAYTEST != 0 {
                subsimplex_convex_cast(
                    &point_shape,
                    collision_shape,
                    &mut simplex_solver,
                    ray_from_trans,
                    ray_to_trans,
                    &col_obj_world_transform,
                    &col_obj_world_transform,
                    &mut cast_result,
                )
            } else {
                gjk_convex_cast(
                    &point_shape,
                    collision_shape,
                    &mut simplex_solver,
                    ray_from_trans,
                    ray_to_trans,
                    &col_obj_world_transform,
                    &col_obj_world_transform,
                    &mut cast_result,
                )
            };
            if hit
                && cast_result.normal.length2() > 0.0001
                && cast_result.fraction < cb.base().closest_hit_fraction
            {
                cast_result.normal.normalize();
                let mut r = LocalRayResult {
                    collision_object: w.obj,
                    local_shape_info: None,
                    hit_normal_local: cast_result.normal,
                    hit_fraction: cast_result.fraction,
                };
                cb.add_single_result(&mut r, true, &self.objects);
            }
        } else if collision_shape.is_concave() {
            // No concave shapes exist in this port.
        } else if collision_shape.is_compound() {
            let compound = collision_shape.compound();
            let process_leaf = |i: usize, cb: &mut dyn RayResultCallback| {
                let child_collision_shape = compound.get_child_shape(i);
                let child_trans = compound.get_child_transform(i);
                let child_world_trans = col_obj_world_transform * *child_trans;
                let tmp_ob = ObjWrapper {
                    shape: child_collision_shape,
                    obj: w.obj,
                    world_transform: child_world_trans,
                    part_id: -1,
                    index: i as i32,
                };
                let base = RayResultBase {
                    closest_hit_fraction: cb.base().closest_hit_fraction,
                    flags: cb.base().flags,
                    ..Default::default()
                };
                let mut my_cb = LocalInfoAdder2 {
                    base,
                    user: cb,
                    i: i as i32,
                };
                self.ray_test_single_internal(ray_from_trans, ray_to_trans, &tmp_ob, &mut my_cb);
            };
            if let Some(dbvt) = &compound.dynamic_aabb_tree {
                let local_ray_from = col_obj_world_transform.inverse_times(ray_from_trans).origin;
                let local_ray_to = col_obj_world_transform.inverse_times(ray_to_trans).origin;
                let mut leaves = Vec::new();
                dbvt.ray_test(dbvt.root, &local_ray_from, &local_ray_to, &mut |n| {
                    leaves.push(dbvt.node(n).data)
                });
                for i in leaves {
                    process_leaf(i, cb);
                }
            } else {
                for i in 0..compound.get_num_child_shapes() {
                    process_leaf(i, cb);
                }
            }
        }
    }

    /// `objectQuerySingle`.
    #[allow(clippy::too_many_arguments)]
    pub fn object_query_single(
        &self,
        cast_shape: &Shape,
        convex_from_trans: &Transform,
        convex_to_trans: &Transform,
        collision_object: ObjId,
        collision_shape: ShapeId,
        col_obj_world_transform: &Transform,
        cb: &mut dyn ConvexResultCallback,
        allowed_penetration: f64,
    ) {
        let w = ObjWrapper {
            shape: collision_shape,
            obj: collision_object,
            world_transform: *col_obj_world_transform,
            part_id: -1,
            index: -1,
        };
        self.object_query_single_internal(
            cast_shape,
            convex_from_trans,
            convex_to_trans,
            &w,
            cb,
            allowed_penetration,
        );
    }

    /// `objectQuerySingleInternal`.
    pub fn object_query_single_internal(
        &self,
        cast_shape: &Shape,
        convex_from_trans: &Transform,
        convex_to_trans: &Transform,
        w: &ObjWrapper,
        cb: &mut dyn ConvexResultCallback,
        allowed_penetration: f64,
    ) {
        let collision_shape = self.shapes.get(w.shape);
        let col_obj_world_transform = w.world_transform;
        if collision_shape.is_convex() {
            let mut cast_result = CastResult {
                allowed_penetration,
                fraction: cb.base().closest_hit_fraction,
                ..Default::default()
            };
            let mut simplex_solver = VoronoiSimplexSolver::new();
            if continuous_convex_collision(
                cast_shape,
                collision_shape,
                &mut simplex_solver,
                Some(PenetrationDepthSolver::GjkEpa),
                convex_from_trans,
                convex_to_trans,
                &col_obj_world_transform,
                &col_obj_world_transform,
                &mut cast_result,
            ) && cast_result.normal.length2() > 0.0001
                && cast_result.fraction < cb.base().closest_hit_fraction
            {
                cast_result.normal.normalize();
                let mut r = LocalConvexResult {
                    hit_collision_object: w.obj,
                    local_shape_info: None,
                    hit_normal_local: cast_result.normal,
                    hit_point_local: cast_result.hit_point,
                    hit_fraction: cast_result.fraction,
                };
                cb.add_single_result(&mut r, true, &self.objects);
            }
        } else if collision_shape.is_concave() {
            // No concave shapes exist in this port.
        } else if collision_shape.is_compound() {
            let compound = collision_shape.compound();
            for i in 0..compound.get_num_child_shapes() {
                let child_trans = *compound.get_child_transform(i);
                let child_collision_shape = compound.get_child_shape(i);
                let child_world_trans = col_obj_world_transform * child_trans;
                let base = ConvexResultBase {
                    closest_hit_fraction: cb.base().closest_hit_fraction,
                    ..Default::default()
                };
                let mut my_cb = LocalInfoAdder {
                    base,
                    user: cb,
                    i: i as i32,
                };
                let tmp_obj = ObjWrapper {
                    shape: child_collision_shape,
                    obj: w.obj,
                    world_transform: child_world_trans,
                    part_id: -1,
                    index: i as i32,
                };
                self.object_query_single_internal(
                    cast_shape,
                    convex_from_trans,
                    convex_to_trans,
                    &tmp_obj,
                    &mut my_cb,
                    allowed_penetration,
                );
            }
        }
    }

    /// `convexSweepTest(castShape, from, to, resultCallback, allowedCcdPenetration)`.
    pub fn convex_sweep_test(
        &self,
        cast_shape: &Shape,
        convex_from_world: &Transform,
        convex_to_world: &Transform,
        cb: &mut dyn ConvexResultCallback,
        allowed_ccd_penetration: f64,
    ) {
        let convex_from_trans = *convex_from_world;
        let convex_to_trans = *convex_to_world;
        let (cast_shape_aabb_min, cast_shape_aabb_max);
        {
            let (_lin_vel, ang_vel) = calculate_velocity(&convex_from_trans, &convex_to_trans, 1.0);
            let zero_lin_vel = Vec3::new(0.0, 0.0, 0.0);
            let mut r = Transform::IDENTITY;
            r.set_rotation(&convex_from_trans.get_rotation());
            let (mi, ma) = cast_shape.calculate_temporal_aabb(&r, &zero_lin_vel, &ang_vel, 1.0);
            cast_shape_aabb_min = mi;
            cast_shape_aabb_max = ma;
        }
        let mut convex_cb = SingleSweepCallback::new(
            cast_shape,
            convex_from_world,
            convex_to_world,
            self,
            cb,
            allowed_ccd_penetration,
        );
        self.broadphase.ray_test(
            &convex_from_trans.origin,
            &convex_to_trans.origin,
            &mut convex_cb,
            &cast_shape_aabb_min,
            &cast_shape_aabb_max,
        );
    }
}

/// `btSingleRayCallback`.
struct SingleRayCallback<'w, 'c> {
    ray_direction_inverse: Vec3,
    signs: [u32; 3],
    lambda_max: f64,
    ray_from_trans: Transform,
    ray_to_trans: Transform,
    world: &'w CollisionWorld,
    result_callback: &'c mut dyn RayResultCallback,
}

fn inv_dir(d: f64) -> f64 {
    if d == 0.0 {
        BT_LARGE_FLOAT
    } else {
        1.0 / d
    }
}

impl<'w, 'c> SingleRayCallback<'w, 'c> {
    fn new(
        ray_from_world: &Vec3,
        ray_to_world: &Vec3,
        world: &'w CollisionWorld,
        result_callback: &'c mut dyn RayResultCallback,
    ) -> Self {
        let mut ray_from_trans = Transform::IDENTITY;
        ray_from_trans.origin = *ray_from_world;
        let mut ray_to_trans = Transform::IDENTITY;
        ray_to_trans.origin = *ray_to_world;
        let mut ray_dir = *ray_to_world - *ray_from_world;
        ray_dir.normalize();
        let inv = Vec3::new(inv_dir(ray_dir.x), inv_dir(ray_dir.y), inv_dir(ray_dir.z));
        let signs = [
            (inv.x < 0.0) as u32,
            (inv.y < 0.0) as u32,
            (inv.z < 0.0) as u32,
        ];
        let lambda_max = ray_dir.dot(&(*ray_to_world - *ray_from_world));
        SingleRayCallback {
            ray_direction_inverse: inv,
            signs,
            lambda_max,
            ray_from_trans,
            ray_to_trans,
            world,
            result_callback,
        }
    }
}

impl BroadphaseRayCallback for SingleRayCallback<'_, '_> {
    fn ray_direction_inverse(&self) -> Vec3 {
        self.ray_direction_inverse
    }
    fn signs(&self) -> [u32; 3] {
        self.signs
    }
    fn lambda_max(&self) -> f64 {
        self.lambda_max
    }
    fn process(&mut self, _proxy_id: ProxyId, proxy: &BroadphaseProxy) -> bool {
        if self.result_callback.base().closest_hit_fraction == 0.0 {
            return false;
        }
        let id = proxy.client_object;
        let o = self.world.obj(id);
        let handle = o.broadphase_handle.expect("object without proxy");
        let own = &self.world.broadphase.proxy(handle).base;
        if self.result_callback.needs_collision(own) {
            let t = o.world_transform;
            self.world.ray_test_single(
                &self.ray_from_trans,
                &self.ray_to_trans,
                id,
                o.shape(),
                &t,
                self.result_callback,
            );
        }
        true
    }
}

/// `btSingleSweepCallback`.
struct SingleSweepCallback<'w, 'c, 's> {
    ray_direction_inverse: Vec3,
    signs: [u32; 3],
    lambda_max: f64,
    convex_from_trans: Transform,
    convex_to_trans: Transform,
    world: &'w CollisionWorld,
    result_callback: &'c mut dyn ConvexResultCallback,
    allowed_ccd_penetration: f64,
    cast_shape: &'s Shape,
}

impl<'w, 'c, 's> SingleSweepCallback<'w, 'c, 's> {
    fn new(
        cast_shape: &'s Shape,
        convex_from_trans: &Transform,
        convex_to_trans: &Transform,
        world: &'w CollisionWorld,
        result_callback: &'c mut dyn ConvexResultCallback,
        allowed_penetration: f64,
    ) -> Self {
        let unnormalized_ray_dir = convex_to_trans.origin - convex_from_trans.origin;
        let ray_dir = unnormalized_ray_dir.normalized();
        let inv = Vec3::new(inv_dir(ray_dir.x), inv_dir(ray_dir.y), inv_dir(ray_dir.z));
        let signs = [
            (inv.x < 0.0) as u32,
            (inv.y < 0.0) as u32,
            (inv.z < 0.0) as u32,
        ];
        let lambda_max = ray_dir.dot(&unnormalized_ray_dir);
        SingleSweepCallback {
            ray_direction_inverse: inv,
            signs,
            lambda_max,
            convex_from_trans: *convex_from_trans,
            convex_to_trans: *convex_to_trans,
            world,
            result_callback,
            allowed_ccd_penetration: allowed_penetration,
            cast_shape,
        }
    }
}

impl BroadphaseRayCallback for SingleSweepCallback<'_, '_, '_> {
    fn ray_direction_inverse(&self) -> Vec3 {
        self.ray_direction_inverse
    }
    fn signs(&self) -> [u32; 3] {
        self.signs
    }
    fn lambda_max(&self) -> f64 {
        self.lambda_max
    }
    fn process(&mut self, _proxy_id: ProxyId, proxy: &BroadphaseProxy) -> bool {
        if self.result_callback.base().closest_hit_fraction == 0.0 {
            return false;
        }
        let id = proxy.client_object;
        let o = self.world.obj(id);
        let handle = o.broadphase_handle.expect("object without proxy");
        let own = &self.world.broadphase.proxy(handle).base;
        if self
            .result_callback
            .needs_collision(own, &self.world.objects)
        {
            let t = o.world_transform;
            self.world.object_query_single(
                self.cast_shape,
                &self.convex_from_trans,
                &self.convex_to_trans,
                id,
                o.shape(),
                &t,
                self.result_callback,
                self.allowed_ccd_penetration,
            );
        }
        true
    }
}
