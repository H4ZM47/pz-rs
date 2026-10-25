#![allow(clippy::needless_range_loop)] // bone and body-part loops follow the C++
//! `Ragdoll` (the rigid bodies, anchors and joints of one ragdoll),
//! `PZRagdoll` (the game's wrapper with its skeleton pose) and
//! `PZRagdollPool`, reconstructed from the shipped library (Ragdoll.cpp,
//! PZRagdoll.cpp, PZRagdollPool.h and the ragdoll parts of
//! WorldSimulation.cpp).
//!
//! A ragdoll has 11 body parts (`RagdollBodyPart`: pelvis, spine, head,
//! left/right upper and lower legs, left/right upper and lower arms), each a
//! compound shape with one capsule, box or sphere, and 10 joints
//! (`RagdollJoint`), each a cone-twist or hinge constraint. Bones of the
//! 35-bone skeleton are "anchored" to body parts: `offsets[bone]` maps the
//! bone's absolute transform to its body's transform.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use super::ragdoll_builder::RagdollBuilder;
use super::ragdoll_script::{
    RagdollBodyDynamics, CONETWIST_CONSTRAINT_TYPE, HINGE_CONSTRAINT_TYPE,
};
use super::ragdoll_skeleton::{rotation_normalized, skeleton_bone, Skeleton, SkeletonPose};
use crate::bt::collision_object::*;
use crate::bt::dynamics::cone_twist::ConeTwistConstraint;
use crate::bt::dynamics::hinge::HingeConstraint;
use crate::bt::dynamics::rigid_body::RigidBodyConstructionInfo;
use crate::bt::dynamics::typed_constraint::ConstraintKind;
use crate::bt::dynamics::world::DiscreteDynamicsWorld;
use crate::bt::linear_math::*;
use crate::bt::shapes::Shape;

/// Number of body parts (`RagdollBodyPart::BODYPART_COUNT`).
pub const BODY_PART_COUNT: usize = 11;
/// Number of joints (`RagdollJoint::JOINT_COUNT`).
pub const JOINT_COUNT: usize = 10;
/// `Ragdoll::rigidBodyToBone`: the bone that drives each body part.
pub const RIGID_BODY_TO_BONE: [usize; BODY_PART_COUNT] = [2, 4, 6, 20, 21, 24, 25, 8, 9, 14, 15];
/// Broadphase group and mask of ragdoll bodies (`addRigidBody(body, 8, 7)`).
pub const RAGDOLL_GROUP: i16 = 8;
pub const RAGDOLL_MASK: i16 = 7;
/// `PZbtVector3::btVector3Up`.
pub const UP: Vec3 = Vec3::new(0.0, 1.0, 0.0);

/// The world's copies of the builder's body-part shapes (one compound per
/// part and shape generation). Bodies keep the shape they were given; the
/// original deletes replaced shapes while bodies may still use them.
#[derive(Clone, Debug, Default)]
pub struct RagdollShapeCache {
    pub generation: u64,
    pub ids: Vec<ShapeId>,
}

impl RagdollShapeCache {
    /// The compound shape of body part `part` for the builder's current
    /// generation.
    pub fn part_shape(
        &mut self,
        builder: &RagdollBuilder,
        dw: &mut DiscreteDynamicsWorld,
        part: usize,
    ) -> ShapeId {
        if self.ids.is_empty() || self.generation != builder.shape_generation {
            self.ids.clear();
            for s in &builder.body_part_shapes {
                let s = s.as_ref().expect("body part shape");
                let child = dw.cw.shapes.add(s.child.clone());
                let compound = dw.cw.shapes.add(Shape::new_compound(true));
                dw.cw
                    .shapes
                    .compound_add_child_shape(compound, &s.child_transform, child);
                self.ids.push(compound);
            }
            self.generation = builder.shape_generation;
        }
        self.ids[part]
    }
}

/// What the ragdoll code borrows from the world and the global builder.
pub struct RagdollCtx<'a> {
    pub builder: &'a RagdollBuilder,
    pub dw: &'a mut DiscreteDynamicsWorld,
    pub shapes: &'a mut RagdollShapeCache,
}

/// `Ragdoll` (0x108 bytes).
#[derive(Clone, Debug)]
pub struct Ragdoll {
    /// `+0x00`: add the joints with the bodies (always set).
    pub add_constraints: bool,
    /// `+0x08`: the ragdoll's world transform (`updateRagdoll`).
    pub transform: Transform,
    /// `+0x88`: `m_dbgDrawSize` of the joints (0.2f).
    pub dbg_draw_size: f64,
    /// `+0x90`: speed limit of `clampVelocity`.
    pub max_velocity: f64,
    /// `+0x98`: in the dynamics world.
    pub in_world: bool,
    /// `+0xa0`: body part anchored to each bone, or -1.
    pub anchors: Vec<i32>,
    /// `+0xb8`.
    pub bodies: Vec<Option<ObjId>>,
    /// `+0xd0`: per bone, its body's transform relative to the bone.
    pub offsets: Vec<Transform>,
    /// `+0xe8`.
    pub constraints: Vec<Option<ConstraintId>>,
}

impl Default for Ragdoll {
    fn default() -> Self {
        Ragdoll::new()
    }
}

impl Ragdoll {
    /// `Ragdoll()`.
    pub fn new() -> Ragdoll {
        Ragdoll {
            add_constraints: true,
            transform: Transform::IDENTITY,
            dbg_draw_size: flt(0.2),
            max_velocity: 10.0,
            in_world: false,
            anchors: Vec::new(),
            bodies: Vec::new(),
            offsets: Vec::new(),
            constraints: Vec::new(),
        }
    }

    /// `getRigidBody(part)`.
    pub fn body(&self, part: usize) -> ObjId {
        self.bodies[part].expect("ragdoll body part")
    }

    /// `create(pose)`.
    pub fn create(&mut self, ctx: &mut RagdollCtx, pose: &mut SkeletonPose) {
        self.anchors = vec![-1; pose.num_bones()];
        self.bodies.resize(BODY_PART_COUNT, None);
        self.update_body_parts(ctx, pose);
        self.load_anchors(ctx, pose);
        self.load_constraints(ctx);
    }

    /// `updateBodyParts(pose)`.
    pub fn update_body_parts(&mut self, ctx: &mut RagdollCtx, pose: &mut SkeletonPose) {
        for part in 0..BODY_PART_COUNT as i32 {
            self.update_body_part(ctx, pose, part);
        }
    }

    /// `createRigidBody(mass, transform, shape)`: default construction info
    /// with a `btDefaultMotionState(transform)`.
    fn create_rigid_body(
        dw: &mut DiscreteDynamicsWorld,
        mass: f64,
        t: &Transform,
        shape: ShapeId,
    ) -> ObjId {
        let inertia = if mass != 0.0 {
            dw.cw.shapes.get(shape).calculate_local_inertia(mass)
        } else {
            Vec3::ZERO
        };
        let info = RigidBodyConstructionInfo::new(
            mass,
            Some(DefaultMotionState::new(*t)),
            Some(shape),
            inertia,
        );
        dw.create_rigid_body(&info)
    }

    /// `updateBodyPart(pose, part)`: (re)creates the body of a part between
    /// its two bones, with the builder's current shape and mass.
    pub fn update_body_part(&mut self, ctx: &mut RagdollCtx, pose: &mut SkeletonPose, part: i32) {
        let info = *ctx
            .builder
            .script
            .get_body_part_info(part)
            .expect("body part info not defined");
        let p = info.part as usize;
        let body = self.bodies[p];
        if let Some(b) = body {
            if self.in_world {
                ctx.dw.remove_rigid_body(b);
            }
        }
        let pair = ctx.builder.bone_pairs[p];
        let (b0, b1) = (pair[0] as usize, pair[1] as usize);
        let a0 = pose.get_bone_pose_absolute(b0);
        let a1 = pose.get_bone_pose_absolute(b1);
        let center = (a0.origin + a1.origin) * 0.5;
        // (the original also takes the rotation of bone 0 and drops it)
        let a0 = pose.get_bone_pose_absolute(b0);
        let a1 = pose.get_bone_pose_absolute(b1);
        let mut dir = a0.origin - a1.origin;
        let q = if dir.x == 0.0 && dir.y == 0.0 && dir.z == 0.0 {
            shortest_arc_quat(&UP, &UP)
        } else {
            if pose.skeleton.parent(b0) == pair[1] {
                dir = -dir;
            }
            dir.normalize();
            shortest_arc_quat(&UP, &dir)
        };
        let mut basis = Mat3::IDENTITY;
        basis.set_rotation(&q);
        let t = Transform::new(basis, center);
        let shape = ctx.shapes.part_shape(ctx.builder, ctx.dw, p);
        let mass = ctx.builder.get_body_part_mass(info.part);
        let inertia = if mass != 0.0 {
            ctx.dw.cw.shapes.get(shape).calculate_local_inertia(mass)
        } else {
            Vec3::ZERO
        };
        let b = match body {
            None => {
                let b = Self::create_rigid_body(ctx.dw, mass, &t, shape);
                self.bodies[p] = Some(b);
                b
            }
            Some(b) => {
                let o = ctx.dw.cw.obj_mut(b);
                o.set_collision_shape(shape);
                o.set_mass_props(mass, &inertia);
                o.update_revision += 1;
                o.world_transform = t;
                b
            }
        };
        configure_body_dynamics(ctx.builder, info.part, ctx.dw.cw.obj_mut(b));
        if self.in_world {
            // (cleanProxyFromPairs on the removed body's null proxy: no-op)
            ctx.dw
                .add_rigid_body_filtered(b, RAGDOLL_GROUP, RAGDOLL_MASK);
        }
    }

    /// `loadAnchors(pose)` / `updateAnchors(pose)`.
    pub fn load_anchors(&mut self, ctx: &mut RagdollCtx, pose: &mut SkeletonPose) {
        let n = pose.num_bones();
        self.anchors.resize(n, 0);
        self.offsets.resize(n, Transform::IDENTITY);
        for i in 0..n {
            self.anchors[i] = -1;
            self.offsets[i] = Transform::IDENTITY;
        }
        let anchors = ctx.builder.script.anchors.clone();
        for a in anchors.iter().filter(|a| a.enabled) {
            self.anchor_bone_to_body_part(
                ctx,
                pose,
                a.bone as usize,
                a.body_part,
                a.reverse,
                a.original,
                Vec3::ZERO,
            );
        }
    }

    /// `anchorBoneToBodyPart(pose, bone, part, reverse, original, offset)`.
    #[allow(clippy::too_many_arguments)]
    pub fn anchor_bone_to_body_part(
        &mut self,
        ctx: &mut RagdollCtx,
        pose: &mut SkeletonPose,
        bone: usize,
        part: i32,
        reverse: bool,
        original: bool,
        offset: Vec3,
    ) {
        self.anchors[bone] = part;
        let b = ctx.dw.cw.obj(self.body(part as usize)).world_transform;
        let info = *ctx
            .builder
            .script
            .get_body_part_info(part)
            .expect("body part info not defined");
        let a = pose.get_bone_pose_absolute(bone);
        let t = if original {
            Transform::new(b.basis, b.origin + info.offset)
        } else {
            let a2 = pose.get_bone_pose_absolute(bone);
            let mid = (b.origin + a2.origin) * 0.5;
            let a3 = pose.get_bone_pose_absolute(bone);
            let mut dir = b.origin - a3.origin;
            // The original leaves the rotation uninitialised for a zero
            // direction (it keeps whatever the registers held).
            let q = if dir.x == 0.0 && dir.y == 0.0 && dir.z == 0.0 {
                Quat::IDENTITY
            } else {
                if reverse {
                    dir = -dir;
                }
                dir.normalize();
                shortest_arc_quat(&UP, &dir)
            };
            let mut basis = Mat3::IDENTITY;
            basis.set_rotation(&q);
            Transform::new(basis, offset + mid)
        };
        self.offsets[bone] = a.inverse() * t;
    }

    /// `loadConstraints()`: joints 0..9, stopping at the first that fails.
    pub fn load_constraints(&mut self, ctx: &mut RagdollCtx) {
        self.constraints.resize(JOINT_COUNT, None);
        for joint in 0..JOINT_COUNT as i32 {
            if !self.load_constraint(ctx, joint) {
                break;
            }
        }
    }

    /// `loadConstraint(joint)`.
    pub fn load_constraint(&mut self, ctx: &mut RagdollCtx, joint: i32) -> bool {
        let c = *ctx
            .builder
            .script
            .get_ragdoll_constraint(joint)
            .expect("ragdoll constraint not defined");
        let mut ba = Mat3::IDENTITY;
        ba.set_euler_zyx(c.axis_a.x, c.axis_a.y, c.axis_a.z);
        let fa = Transform::new(ba, c.position_offset_a);
        let mut bb = Mat3::IDENTITY;
        bb.set_euler_zyx(c.axis_b.x, c.axis_b.y, c.axis_b.z);
        let fb = Transform::new(bb, c.position_offset_b);
        let ra = self.body(c.part_a as usize);
        let rb = self.body(c.part_b as usize);
        let mut tc = match c.constraint_type {
            CONETWIST_CONSTRAINT_TYPE => {
                let mut tc = ConeTwistConstraint::new_typed(ra, rb, fa, fb);
                if let ConstraintKind::ConeTwist(ct) = &mut tc.kind {
                    ct.set_limit(c.limit.x, c.limit.y, c.limit.z, 1.0, flt(0.3), 1.0);
                }
                tc
            }
            HINGE_CONSTRAINT_TYPE => {
                let mut tc = HingeConstraint::new_typed_frames(ra, rb, fa, fb, false);
                if let ConstraintKind::Hinge(h) = &mut tc.kind {
                    h.limit.set(c.limit.x, c.limit.y, flt(0.9), flt(0.3), 1.0);
                }
                tc
            }
            _ => return false,
        };
        tc.dbg_draw_size = self.dbg_draw_size;
        let id = ctx.dw.constraints.insert(tc);
        if let Some(slot) = self.constraints.get_mut(c.joint as usize) {
            *slot = Some(id);
        }
        true
    }

    /// `deleteConstraints()`: removes the joints from the world and deletes
    /// them.
    pub fn delete_constraints(&mut self, dw: &mut DiscreteDynamicsWorld) {
        for c in self.constraints.iter_mut() {
            if let Some(id) = c.take() {
                if self.in_world {
                    dw.remove_constraint(id);
                }
                dw.constraints.remove(id);
            }
        }
    }

    /// `updateConstraints()`: deletes and reloads the joints. New joints are
    /// not added to the world, even when the ragdoll is in it.
    pub fn update_constraints(&mut self, ctx: &mut RagdollCtx) {
        self.delete_constraints(ctx.dw);
        self.load_constraints(ctx);
    }

    /// `addToSimulation(world)`.
    pub fn add_to_simulation(&mut self, dw: &mut DiscreteDynamicsWorld) {
        self.in_world = true;
        for b in self.bodies.iter().flatten() {
            let o = dw.cw.obj_mut(*b);
            o.set_linear_velocity(&Vec3::ZERO);
            o.set_angular_velocity(&Vec3::ZERO);
            o.activate(true);
            dw.add_rigid_body_filtered(*b, RAGDOLL_GROUP, RAGDOLL_MASK);
        }
        if self.add_constraints {
            for c in self.constraints.iter().flatten() {
                dw.constraints.get_mut(*c).is_enabled = true;
                add_existing_constraint(dw, *c, true);
            }
        }
    }

    /// `removeFromSimulation()`.
    pub fn remove_from_simulation(&mut self, dw: &mut DiscreteDynamicsWorld) {
        for b in self.bodies.iter().flatten() {
            dw.remove_rigid_body(*b);
            let o = dw.cw.obj_mut(*b);
            o.activate(false);
            o.set_linear_velocity(&Vec3::ZERO);
            o.set_angular_velocity(&Vec3::ZERO);
        }
        for c in self.constraints.iter().flatten() {
            dw.remove_constraint(*c);
            dw.constraints.get_mut(*c).is_enabled = false;
        }
        self.in_world = false;
    }

    /// `updateSkeletonFromBodies(pose)`: anchored bones follow their bodies.
    pub fn update_skeleton_from_bodies(&self, dw: &DiscreteDynamicsWorld, pose: &mut SkeletonPose) {
        let inv = self.transform.inverse();
        for i in 0..pose.num_bones() {
            let p = self.anchors[i];
            if p < 0 {
                continue;
            }
            let Some(b) = self.bodies[p as usize] else {
                continue;
            };
            let bt = dw.cw.obj(b).world_transform;
            let x = inv * bt * self.offsets[i].inverse();
            pose.set_bone_pose_absolute(i, &x);
        }
    }

    /// `updateBodiesFromSkeleton(pose)`: bodies jump to their bones, at rest.
    pub fn update_bodies_from_skeleton(
        &self,
        dw: &mut DiscreteDynamicsWorld,
        pose: &mut SkeletonPose,
    ) {
        for i in 0..BODY_PART_COUNT {
            let Some(b) = self.bodies[i] else {
                continue;
            };
            let bone = RIGID_BODY_TO_BONE[i];
            let a = pose.get_bone_pose_absolute(bone);
            if self.in_world {
                dw.remove_rigid_body(b);
            }
            let t = self.transform * a * self.offsets[bone];
            let o = dw.cw.obj_mut(b);
            o.world_transform = t;
            let rb = o.rb_mut();
            rb.linear_velocity = Vec3::ZERO;
            rb.angular_velocity = Vec3::ZERO;
            o.update_revision += 3;
            if self.in_world {
                dw.add_rigid_body_filtered(b, RAGDOLL_GROUP, RAGDOLL_MASK);
            }
        }
    }

    /// `applySkeletonVelocities(pose, dt, previous)`: gives each body the
    /// velocity of its bone between `previous` and the current pose.
    pub fn apply_skeleton_velocities(
        &self,
        dw: &mut DiscreteDynamicsWorld,
        pose: &mut SkeletonPose,
        dt: f32,
        previous: &[Transform],
    ) {
        let inv_dt = 1.0 / dt as f64;
        for i in 0..BODY_PART_COUNT {
            let Some(b) = self.bodies[i] else {
                continue;
            };
            let bone = RIGID_BODY_TO_BONE[i];
            let a = pose.get_bone_pose_absolute(bone);
            let cur = self.transform * a * self.offsets[bone];
            let prev = self.transform * previous[bone] * self.offsets[bone];
            let d = prev.inverse() * cur;
            let (yaw, pitch, roll) = euler_ypr(&d.basis);

            let o = dw.cw.obj_mut(b);
            o.apply_central_impulse(&(d.origin * inv_dt));
            o.apply_torque_impulse(&(Vec3::new(yaw, pitch, roll) * inv_dt));
        }
    }

    /// `applyForce(part, force, relPos)`.
    pub fn apply_force(&self, dw: &mut DiscreteDynamicsWorld, part: usize, f: &Vec3, rel: &Vec3) {
        dw.cw.obj_mut(self.body(part)).apply_force(f, rel);
    }

    /// `applyImpulse(part, impulse, relPos)`.
    pub fn apply_impulse(&self, dw: &mut DiscreteDynamicsWorld, part: usize, i: &Vec3, rel: &Vec3) {
        dw.cw.obj_mut(self.body(part)).apply_impulse(i, rel);
    }

    /// `detachConstraint(joint)`: disables the joint (it stays in the
    /// world).
    pub fn detach_constraint(&self, dw: &mut DiscreteDynamicsWorld, joint: usize) {
        let c = self.constraints[joint].expect("ragdoll joint");
        dw.constraints.get_mut(c).is_enabled = false;
    }

    /// `updateBodyDynamics(dynamics)`: the same dynamics for every part.
    pub fn update_body_dynamics(
        &self,
        builder: &RagdollBuilder,
        dw: &mut DiscreteDynamicsWorld,
        d: &RagdollBodyDynamics,
    ) {
        for part in 0..BODY_PART_COUNT as i32 {
            let info = builder
                .script
                .get_body_part_info(part)
                .expect("body part info not defined");
            let b = self.body(info.part as usize);
            apply_body_dynamics(d, dw.cw.obj_mut(b));
        }
    }

    /// `resetBodyDynamics()`: back to the script's dynamics.
    pub fn reset_body_dynamics(&self, builder: &RagdollBuilder, dw: &mut DiscreteDynamicsWorld) {
        for part in 0..BODY_PART_COUNT as i32 {
            let info = builder
                .script
                .get_body_part_info(part)
                .expect("body part info not defined");
            let b = self.body(info.part as usize);
            configure_body_dynamics(builder, info.part, dw.cw.obj_mut(b));
        }
    }

    /// `getRigidBodyData(out)`: position and rotation of each body, 7
    /// floats per part.
    pub fn get_rigid_body_data(&self, dw: &DiscreteDynamicsWorld, out: &mut [f32]) {
        for i in 0..BODY_PART_COUNT {
            let t = dw.cw.obj(self.body(i)).world_transform;
            let q = t.basis.get_rotation();
            let o = &mut out[i * 7..i * 7 + 7];
            o[0] = t.origin.x as f32;
            o[1] = t.origin.y as f32;
            o[2] = t.origin.z as f32;
            o[3] = q.x as f32;
            o[4] = q.y as f32;
            o[5] = q.z as f32;
            o[6] = q.w as f32;
        }
    }

    /// `clampVelocity()`: limits each body's linear speed.
    pub fn clamp_velocity(&self, dw: &mut DiscreteDynamicsWorld) {
        for i in 0..BODY_PART_COUNT {
            let o = dw.cw.obj_mut(self.body(i));
            let max = self.max_velocity;
            let v = o.rb().linear_velocity;
            let len = v.length();
            if max < len {
                let k = 1.0 / len;
                o.update_revision += 1;
                o.rb_mut().linear_velocity = Vec3::new(v.x * k * max, v.y * k * max, v.z * k * max);
            }
        }
    }
}

/// `btMatrix3x3::getEulerYPR(yaw, pitch, roll)`.
pub fn euler_ypr(m: &Mat3) -> (f64, f64, f64) {
    let m = &m.el;
    let mut yaw = bt_atan2(m[1].x, m[0].x);
    let pitch = bt_asin(-m[2].x);
    let mut roll = bt_atan2(m[2].y, m[2].z);
    if pitch.abs() == SIMD_HALF_PI {
        if yaw > 0.0 {
            yaw -= SIMD_PI;
        } else {
            yaw += SIMD_PI;
        }
        if roll > 0.0 {
            roll -= SIMD_PI;
        } else {
            roll += SIMD_PI;
        }
    }
    (yaw, pitch, roll)
}

/// Writes a `RagdollBodyDynamics` into a body (`configureBodyDynamics` and
/// `Ragdoll::updateBodyDynamics`).
fn apply_body_dynamics(d: &RagdollBodyDynamics, o: &mut CollisionObject) {
    o.set_damping(d.linear_damping as f64, d.angular_damping as f64);
    let rb = o.rb_mut();
    rb.linear_sleeping_threshold = d.linear_sleeping_threshold as f64;
    rb.angular_sleeping_threshold = d.angular_sleeping_threshold as f64;
    o.deactivation_time = d.deactivation_time as f64;
    o.friction = d.friction as f64;
    o.update_revision += 2;
    o.rolling_friction = d.rolling_friction as f64;
}

/// `RagdollBuilder::configureBodyDynamics(part, body)`.
pub fn configure_body_dynamics(builder: &RagdollBuilder, part: i32, o: &mut CollisionObject) {
    let d = *builder
        .script
        .get_ragdoll_body_dynamics(part)
        .expect("body dynamics not defined");
    apply_body_dynamics(&d, o);
}

/// `addConstraint(constraint, disableCollisions)` for a constraint already
/// in the arena.
fn add_existing_constraint(dw: &mut DiscreteDynamicsWorld, id: ConstraintId, disable: bool) {
    dw.constraint_list.push(id);
    if disable {
        let (a, b) = {
            let c = dw.constraints.get(id);
            (c.rb_a, c.rb_b)
        };
        if let Some(a) = a {
            dw.cw.obj_mut(a).add_constraint_ref(id);
        }
        if let Some(b) = b {
            dw.cw.obj_mut(b).add_constraint_ref(id);
        }
    }
}

/// `PZRagdoll` (0xe8 bytes).
#[derive(Clone, Debug)]
pub struct PzRagdoll {
    /// `+0x08`, `+0x09`, `+0x0a`, `+0x0b`, `+0x0c`, `+0x0d`, `+0x10`: debug
    /// drawing flags (only read by the debug renderer).
    pub draw_single_bone: bool,
    pub draw_body_parts_single: bool,
    pub draw_skeleton: bool,
    pub draw_skeleton_single: bool,
    pub draw_body_parts: bool,
    pub draw_highlighted: bool,
    pub draw_constraints: bool,
    /// `+0x1c`: highlighted body part (debug drawing only).
    pub highlighted_body_part: i32,
    /// `+0x20`, `+0x28`: missing when there was no default skeleton.
    pub ragdoll: Option<Ragdoll>,
    pub pose: Option<SkeletonPose>,
    /// `+0x30`: local transform (`setRagdollLocalTransformRotation`).
    pub local: Transform,
    /// `+0xb0`.
    pub id: i32,
    /// `+0xb8`: mass of every body after `setRagdollMass` (70 at first).
    pub mass: f64,
    /// `+0xc0`: in the simulation (`setRagdollActive`).
    pub active: bool,
    /// `+0xc8`: last position given to `updateRagdoll`.
    pub position: Vec3,
}

impl PzRagdoll {
    /// `PZRagdoll(id)`: a pose of the builder's default skeleton and a new
    /// `Ragdoll` built on it.
    pub fn new(ctx: &mut RagdollCtx, id: i32) -> PzRagdoll {
        let mut r = PzRagdoll {
            draw_single_bone: false,
            draw_body_parts_single: false,
            draw_skeleton: false,
            draw_skeleton_single: false,
            draw_body_parts: false,
            draw_highlighted: false,
            draw_constraints: false,
            highlighted_body_part: 1,
            ragdoll: None,
            pose: None,
            local: Transform::IDENTITY,
            id,
            mass: 70.0,
            active: false,
            position: Vec3::ZERO,
        };
        r.pose = ctx
            .builder
            .default_skeleton
            .as_ref()
            .map(|s: &Arc<Skeleton>| SkeletonPose::new(s.clone()));
        if let Some(pose) = r.pose.as_mut() {
            let mut rd = Ragdoll::new();
            rd.create(ctx, pose);
            r.ragdoll = Some(rd);
        }
        r
    }

    fn parts(&mut self) -> (&mut Ragdoll, &mut SkeletonPose) {
        (
            self.ragdoll.as_mut().expect("ragdoll"),
            self.pose.as_mut().expect("skeleton pose"),
        )
    }

    /// `reset(id)` (when the pool hands the ragdoll out again). The bodies,
    /// pose, mass and debug flags are kept.
    pub fn reset(&mut self, id: i32) {
        self.id = id;
        if let Some(r) = self.ragdoll.as_mut() {
            r.transform = Transform::IDENTITY;
        }
        self.local = Transform::IDENTITY;
        self.active = false;
        self.position = Vec3::ZERO;
    }

    /// `removeFromWorld()`.
    pub fn remove_from_world(&mut self, dw: &mut DiscreteDynamicsWorld) {
        if self.active {
            self.ragdoll.as_mut().unwrap().remove_from_simulation(dw);
            self.active = false;
        }
    }

    /// `setActive(active)`.
    pub fn set_active(&mut self, dw: &mut DiscreteDynamicsWorld, active: bool) {
        if self.active != active {
            let r = self.ragdoll.as_mut().unwrap();
            if active {
                r.add_to_simulation(dw);
            } else {
                r.remove_from_simulation(dw);
            }
            self.active = active;
        }
    }

    /// `setLocalTransformRotation(qx, qy, qz, qw)`.
    pub fn set_local_transform_rotation(&mut self, qx: f32, qy: f32, qz: f32, qw: f32) {
        let mut basis = Mat3::IDENTITY;
        basis.set_rotation(&Quat::new(qx as f64, qy as f64, qz as f64, qw as f64));
        self.local = Transform::new(basis, Vec3::ZERO);
    }

    /// `update(x, y, z, qx, qy, qz, qw)`: the ragdoll's world transform.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, x: f32, y: f32, z: f32, qx: f32, qy: f32, qz: f32, qw: f32) {
        self.position = Vec3::new(x as f64, y as f64, z as f64);
        let r = self.ragdoll.as_mut().expect("ragdoll");
        r.transform.origin = self.position;
        r.transform
            .basis
            .set_rotation(&Quat::new(qx as f64, qy as f64, qz as f64, qw as f64));
    }

    /// `parseBoneTransforms(n, floats, relative, absolute)`: 7 floats per
    /// bone (position, rotation); bone 0 is combined with the local
    /// transform, then the absolute transforms follow the hierarchy.
    pub fn parse_bone_transforms(&self, n: i32, p: &[f32]) -> (Vec<Transform>, Vec<Transform>) {
        let n = n.max(0) as usize;
        let mut rel = vec![Transform::IDENTITY; n];
        for (i, t) in rel.iter_mut().enumerate() {
            let r = &p[i * 7..i * 7 + 7];
            t.origin = Vec3::new(r[0] as f64, r[1] as f64, r[2] as f64);
            t.basis.set_rotation(&Quat::new(
                r[3] as f64,
                r[4] as f64,
                r[5] as f64,
                r[6] as f64,
            ));
        }
        let mut abs = vec![Transform::IDENTITY; n];
        if n > 0 {
            rel[0] = rel[0] * self.local;
            abs[0] = rel[0];
        }
        let skel = &self.pose.as_ref().expect("skeleton pose").skeleton;
        for i in 1..n {
            let parent = skel.parent(i) as usize;
            abs[i] = abs[parent] * rel[i];
        }
        (rel, abs)
    }

    /// `updateSkeletonBoneTransforms(n, floats)`: poses the skeleton and
    /// moves the bodies to it.
    pub fn update_skeleton_bone_transforms(
        &mut self,
        dw: &mut DiscreteDynamicsWorld,
        n: i32,
        p: &[f32],
    ) {
        let (_, abs) = self.parse_bone_transforms(n, p);
        let (r, pose) = self.parts();
        pose.set_skeleton_pose(&abs);
        r.update_bodies_from_skeleton(dw, pose);
    }

    /// `updateRagdollSkeletonPreviousBoneTransforms(n, dt, floats)`.
    pub fn update_previous_bone_transforms(
        &mut self,
        dw: &mut DiscreteDynamicsWorld,
        n: i32,
        dt: f32,
        p: &[f32],
    ) {
        let (_, abs) = self.parse_bone_transforms(n, p);
        let (r, pose) = self.parts();
        r.apply_skeleton_velocities(dw, pose, dt, &abs);
    }

    /// `resetSkeletonPose()`.
    pub fn reset_skeleton_pose(&mut self, dw: &mut DiscreteDynamicsWorld) {
        let (r, pose) = self.parts();
        pose.reset_bone_transforms();
        pose.update();
        r.update_bodies_from_skeleton(dw, pose);
    }

    /// `simulate(skeletonOut)`: reads the pose back from the bodies and
    /// writes 7 floats per bone: bone 0 relative to the local transform, the
    /// others relative to their parent. Returns the number of bones.
    pub fn simulate(&mut self, dw: &DiscreteDynamicsWorld, out: &mut [f32]) -> i32 {
        let local_inv = self.local.inverse();
        let (r, pose) = self.parts();
        let n = pose.num_bones();
        r.update_skeleton_from_bodies(dw, pose);
        let saved = pose.pose[0];
        pose.pose[0] = Transform::new(rotation_normalized(&saved).basis, Vec3::ZERO);
        let b1 = skeleton_bone::get_bone_index(skeleton_bone::BIP01) as usize;
        let a1 = pose.get_bone_pose_absolute(b1);
        pose.set_bone_pose_absolute(b1, &Transform::new(a1.basis, Vec3::ZERO));
        let a2 = pose.get_bone_pose_absolute(skeleton_bone::BIP01_PELVIS as usize);
        let bt = skeleton_bone::get_bone_index(skeleton_bone::TRANSLATION_DATA) as usize;
        pose.set_bone_pose_absolute(bt, &a2);
        let rel0 = pose.skeleton.bind[0] * pose.pose[0];
        let x = rel0 * local_inv;
        write_bone(&mut out[0..7], &x);
        for i in 1..n {
            let t = pose.get_bone_pose_relative(i);
            write_bone(&mut out[i * 7..i * 7 + 7], &t);
        }
        n as i32
    }

    /// `simulate(skeletonOut, rigidBodyOut)`.
    pub fn simulate_with_rigid_bodies(
        &mut self,
        dw: &mut DiscreteDynamicsWorld,
        skel: &mut [f32],
        rb: &mut [f32],
    ) -> i32 {
        let n = self.simulate(dw, skel);
        let r = self.ragdoll.as_ref().unwrap();
        r.get_rigid_body_data(dw, rb);
        r.clamp_velocity(dw);
        n
    }

    /// `getSimulationState()`: the activation state shared by all bodies,
    /// or 0.
    pub fn get_simulation_state(&self, dw: &DiscreteDynamicsWorld) -> i32 {
        let r = self.ragdoll.as_ref().unwrap();
        let s0 = dw.cw.obj(r.body(0)).activation_state1;
        for i in 1..BODY_PART_COUNT {
            if dw.cw.obj(r.body(i)).activation_state1 != s0 {
                return 0;
            }
        }
        s0
    }

    /// `setMass(mass)`: gives every body the whole mass.
    pub fn set_mass(&mut self, dw: &mut DiscreteDynamicsWorld, mass: f32) {
        let m = mass as f64;
        if self.mass != m {
            let r = self.ragdoll.as_ref().unwrap();
            for i in 0..BODY_PART_COUNT {
                let b = r.body(i);
                let shape = dw.cw.obj(b).shape();
                let inertia = dw.cw.shapes.get(shape).calculate_local_inertia(m);
                dw.cw.obj_mut(b).set_mass_props(m, &inertia);
            }
            self.mass = m;
        }
    }

    /// `updateBodyParts()`.
    pub fn update_body_parts(&mut self, ctx: &mut RagdollCtx) {
        let (r, pose) = self.parts();
        r.update_body_parts(ctx, pose);
    }

    /// `updateAnchors()`.
    pub fn update_anchors(&mut self, ctx: &mut RagdollCtx) {
        if self.pose.is_some() {
            let (r, pose) = self.parts();
            r.load_anchors(ctx, pose);
        }
    }

    /// `updateConstraints()`.
    pub fn update_constraints(&mut self, ctx: &mut RagdollCtx) {
        self.ragdoll.as_mut().unwrap().update_constraints(ctx);
    }
}

fn write_bone(o: &mut [f32], t: &Transform) {
    let q = t.basis.get_rotation();
    o[0] = t.origin.x as f32;
    o[1] = t.origin.y as f32;
    o[2] = t.origin.z as f32;
    o[3] = q.x as f32;
    o[4] = q.y as f32;
    o[5] = q.z as f32;
    o[6] = q.w as f32;
}

/// The ragdolls of a world: `WorldSimulation::m_ragdolls` with its
/// `PZRagdollPool` (released ragdolls, reused front first) and the world's
/// copies of the body-part shapes.
#[derive(Clone, Debug, Default)]
pub struct RagdollWorld {
    pub ragdolls: BTreeMap<i32, PzRagdoll>,
    pub pool: VecDeque<PzRagdoll>,
    pub shapes: RagdollShapeCache,
}

impl RagdollWorld {
    /// `PZRagdollPool::alloc(id)` (`WorldSimulation::addRagdoll`; the
    /// transform it is given is not used). A ragdoll whose id is taken is
    /// lost.
    pub fn alloc(&mut self, builder: &RagdollBuilder, dw: &mut DiscreteDynamicsWorld, id: i32) {
        let r = match self.pool.pop_front() {
            None => {
                let mut ctx = RagdollCtx {
                    builder,
                    dw,
                    shapes: &mut self.shapes,
                };
                PzRagdoll::new(&mut ctx, id)
            }
            Some(mut r) => {
                r.reset(id);
                r
            }
        };
        self.ragdolls.entry(id).or_insert(r);
    }

    /// `WorldSimulation::removeRagdoll(id)`: out of the world and into the
    /// pool.
    pub fn remove(&mut self, dw: &mut DiscreteDynamicsWorld, id: i32) {
        if let Some(mut r) = self.ragdolls.remove(&id) {
            r.remove_from_world(dw);
            self.pool.push_front(r);
        }
    }

    /// Every ragdoll, active ones (by id) first, then the pool front to back
    /// (the order of the original's "for all ragdolls" loops).
    pub fn for_each(&mut self, mut f: impl FnMut(&mut PzRagdoll, &mut RagdollShapeCache)) {
        for r in self.ragdolls.values_mut() {
            f(r, &mut self.shapes);
        }
        for r in self.pool.iter_mut() {
            f(r, &mut self.shapes);
        }
    }

    /// The first ragdoll with this id, active ones first, then the pool
    /// (`updateRagdollBodyDynamics`, `resetRagdollBodyDynamics`).
    pub fn find_any(&mut self, id: i32) -> Option<&mut PzRagdoll> {
        if let Some(r) = self.ragdolls.values_mut().find(|r| r.id == id) {
            return Some(r);
        }
        self.pool.iter_mut().find(|r| r.id == id)
    }

    /// `WorldSimulation::removeConstraintsOnRagdoll`: never called by the
    /// original, and it compares the vehicle pointers of the vehicle
    /// constraint map with the ragdoll, so it never matches.
    pub fn remove_constraints_on_ragdoll(&mut self, _id: i32) {}
}
