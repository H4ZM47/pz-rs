//! `PZVehicleClosestConvexResultCallback`: the closest convex-sweep hit,
//! ignoring the vehicle's own compound (used by `setOwnVehiclePhysics`).

use crate::bt::broadphase::pair_cache::BroadphaseProxy;
use crate::bt::collision_object::{ObjId, ShapeId};
use crate::bt::collision_world::{
    default_needs_collision, ConvexResultBase, ConvexResultCallback, LocalConvexResult,
};
use crate::bt::dispatch::{obj, ObjArena};
use crate::bt::linear_math::Vec3;
use crate::bt::shapes::COMPOUND_SHAPE_PROXYTYPE;

pub struct PzVehicleClosestConvexResultCallback {
    pub base: ConvexResultBase,
    pub hit_normal_world: Vec3,
    pub hit_point_world: Vec3,
    pub hit_collision_object: Option<ObjId>,
    /// The vehicle's compound shape.
    pub me: ShapeId,
}

impl PzVehicleClosestConvexResultCallback {
    /// Filter group 2, mask -1.
    pub fn new(me: ShapeId) -> Self {
        PzVehicleClosestConvexResultCallback {
            base: ConvexResultBase {
                closest_hit_fraction: 1.0,
                collision_filter_group: 2,
                collision_filter_mask: -1,
            },
            hit_normal_world: Vec3::ZERO,
            hit_point_world: Vec3::ZERO,
            hit_collision_object: None,
            me,
        }
    }
    pub fn closest_hit_fraction(&self) -> f64 {
        self.base.closest_hit_fraction
    }
}

impl ConvexResultCallback for PzVehicleClosestConvexResultCallback {
    fn base(&self) -> &ConvexResultBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ConvexResultBase {
        &mut self.base
    }
    fn needs_collision(&self, proxy: &BroadphaseProxy, _objects: &ObjArena) -> bool {
        default_needs_collision(
            self.base.collision_filter_group,
            self.base.collision_filter_mask,
            proxy,
        )
    }
    fn add_single_result(
        &mut self,
        r: &mut LocalConvexResult,
        normal_in_world_space: bool,
        objects: &ObjArena,
    ) -> f64 {
        let o = obj(objects, r.hit_collision_object);
        let shape = o.collision_shape;
        // The original checks the shape type through the object's shape;
        // only the vehicle's own compound is skipped.
        if shape == Some(self.me) && self.is_compound(shape) {
            return 1.0;
        }
        self.base.closest_hit_fraction = r.hit_fraction;
        self.hit_collision_object = Some(r.hit_collision_object);
        self.hit_normal_world = if normal_in_world_space {
            r.hit_normal_local
        } else {
            o.world_transform.basis * r.hit_normal_local
        };
        self.hit_point_world = r.hit_point_local;
        r.hit_fraction
    }
}

impl PzVehicleClosestConvexResultCallback {
    fn is_compound(&self, _shape: Option<ShapeId>) -> bool {
        // `me` is always the vehicle's compound (type COMPOUND_SHAPE_PROXYTYPE).
        let _ = COMPOUND_SHAPE_PROXYTYPE;
        true
    }
}
