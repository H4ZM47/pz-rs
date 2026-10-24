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
// Altered for pz-rs: Rust translation of btCollisionObject and the data
// members of btRigidBody (Bullet 2.82, double precision).

//! `btCollisionObject` and the `btRigidBody` data.
//!
//! Objects live in an arena owned by the collision world and are addressed
//! by [`ObjId`]; a rigid body is a collision object whose `body` is `Some`
//! (`m_internalType` has `CO_RIGID_BODY`). Shapes are addressed by
//! [`ShapeId`] into the shape arena.

use super::linear_math::*;

/// Index of a collision object in the world's object arena.
pub type ObjId = usize;
/// Index of a collision shape in the shape arena.
pub type ShapeId = usize;
/// Index of a broadphase proxy.
pub type ProxyId = usize;
/// Index of a typed constraint in the constraint arena.
pub type ConstraintId = usize;

pub const ACTIVE_TAG: i32 = 1;
pub const ISLAND_SLEEPING: i32 = 2;
pub const WANTS_DEACTIVATION: i32 = 3;
pub const DISABLE_DEACTIVATION: i32 = 4;
pub const DISABLE_SIMULATION: i32 = 5;

pub const CF_STATIC_OBJECT: i32 = 1;
pub const CF_KINEMATIC_OBJECT: i32 = 2;
pub const CF_NO_CONTACT_RESPONSE: i32 = 4;
pub const CF_CUSTOM_MATERIAL_CALLBACK: i32 = 8;
pub const CF_CHARACTER_OBJECT: i32 = 16;
pub const CF_DISABLE_VISUALIZE_OBJECT: i32 = 32;

pub const CO_COLLISION_OBJECT: i32 = 1;
pub const CO_RIGID_BODY: i32 = 2;

pub const CF_ANISOTROPIC_FRICTION_DISABLED: i32 = 0;
pub const CF_ANISOTROPIC_FRICTION: i32 = 1;
pub const CF_ANISOTROPIC_ROLLING_FRICTION: i32 = 2;

/// `btRigidBodyFlags`.
pub const BT_DISABLE_WORLD_GRAVITY: i32 = 1;
pub const BT_ENABLE_GYROPSCOPIC_FORCE: i32 = 2;

/// `btDefaultMotionState`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultMotionState {
    pub graphics_world_trans: Transform,
    pub center_of_mass_offset: Transform,
    pub start_world_trans: Transform,
}

impl DefaultMotionState {
    /// `btDefaultMotionState(startTrans, centerOfMassOffset = identity)`.
    pub fn new(start_trans: Transform) -> DefaultMotionState {
        DefaultMotionState {
            graphics_world_trans: start_trans,
            center_of_mass_offset: Transform::IDENTITY,
            start_world_trans: start_trans,
        }
    }
    /// `getWorldTransform`: `m_centerOfMassOffset.inverse() * m_graphicsWorldTrans`.
    pub fn get_world_transform(&self) -> Transform {
        self.center_of_mass_offset.inverse() * self.graphics_world_trans
    }
    /// `setWorldTransform`: `centerOfMassWorldTrans * m_centerOfMassOffset`.
    pub fn set_world_transform(&mut self, center_of_mass_world_trans: &Transform) {
        self.graphics_world_trans = *center_of_mass_world_trans * self.center_of_mass_offset;
    }
}

/// The `btRigidBody` members (beyond `btCollisionObject`). Methods live in
/// `dynamics::rigid_body`.
#[derive(Clone, Debug, Default)]
pub struct RigidBody {
    pub inv_inertia_tensor_world: Mat3,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub inverse_mass: f64,
    pub linear_factor: Vec3,
    pub gravity: Vec3,
    pub gravity_acceleration: Vec3,
    pub inv_inertia_local: Vec3,
    pub total_force: Vec3,
    pub total_torque: Vec3,
    pub linear_damping: f64,
    pub angular_damping: f64,
    pub additional_damping: bool,
    pub additional_damping_factor: f64,
    pub additional_linear_damping_threshold_sqr: f64,
    pub additional_angular_damping_threshold_sqr: f64,
    pub additional_angular_damping_factor: f64,
    pub linear_sleeping_threshold: f64,
    pub angular_sleeping_threshold: f64,
    /// `m_optionalMotionState` (the game only uses `btDefaultMotionState`).
    pub optional_motion_state: Option<DefaultMotionState>,
    /// `m_constraintRefs`.
    pub constraint_refs: Vec<ConstraintId>,
    pub rigidbody_flags: i32,
    pub debug_body_id: i32,
    pub delta_linear_velocity: Vec3,
    pub delta_angular_velocity: Vec3,
    pub angular_factor: Vec3,
    pub inv_mass: Vec3,
    pub push_velocity: Vec3,
    pub turn_velocity: Vec3,
}

/// `btCollisionObject` (plus `btRigidBody` data in `body`).
#[derive(Clone, Debug)]
pub struct CollisionObject {
    pub world_transform: Transform,
    pub interpolation_world_transform: Transform,
    pub interpolation_linear_velocity: Vec3,
    pub interpolation_angular_velocity: Vec3,
    pub anisotropic_friction: Vec3,
    pub has_anisotropic_friction: i32,
    pub contact_processing_threshold: f64,
    /// `m_broadphaseHandle`.
    pub broadphase_handle: Option<ProxyId>,
    /// `m_collisionShape`.
    pub collision_shape: Option<ShapeId>,
    pub root_collision_shape: Option<ShapeId>,
    pub collision_flags: i32,
    pub island_tag1: i32,
    pub companion_id: i32,
    pub activation_state1: i32,
    pub deactivation_time: f64,
    pub friction: f64,
    pub restitution: f64,
    pub rolling_friction: f64,
    pub internal_type: i32,
    /// `m_userObjectPointer` / `m_userIndex`: opaque handle for the owner.
    pub user_index: i64,
    pub hit_fraction: f64,
    pub ccd_swept_sphere_radius: f64,
    pub ccd_motion_threshold: f64,
    /// `m_checkCollideWith`.
    pub check_collide_with: bool,
    pub update_revision: i32,
    /// `m_worldArrayIndex` is not a member in 2.82; the world keeps an
    /// ordered list instead. Rigid-body data when this is a `btRigidBody`.
    pub body: Option<Box<RigidBody>>,
}

impl Default for CollisionObject {
    fn default() -> Self {
        CollisionObject::new()
    }
}

impl CollisionObject {
    /// `btCollisionObject()`.
    pub fn new() -> CollisionObject {
        CollisionObject {
            world_transform: Transform::IDENTITY,
            interpolation_world_transform: Transform::default(),
            interpolation_linear_velocity: Vec3::ZERO,
            interpolation_angular_velocity: Vec3::ZERO,
            anisotropic_friction: Vec3::new(1.0, 1.0, 1.0),
            has_anisotropic_friction: 0,
            contact_processing_threshold: BT_LARGE_FLOAT,
            broadphase_handle: None,
            collision_shape: None,
            root_collision_shape: None,
            collision_flags: CF_STATIC_OBJECT,
            island_tag1: -1,
            companion_id: -1,
            activation_state1: 1,
            deactivation_time: 0.0,
            friction: 0.5,
            restitution: 0.0,
            rolling_friction: 0.0,
            internal_type: CO_COLLISION_OBJECT,
            user_index: 0,
            hit_fraction: 1.0,
            ccd_swept_sphere_radius: 0.0,
            ccd_motion_threshold: 0.0,
            check_collide_with: false,
            update_revision: 0,
            body: None,
        }
    }
    #[inline(always)]
    pub fn merges_simulation_islands(&self) -> bool {
        (self.collision_flags & (CF_STATIC_OBJECT | CF_KINEMATIC_OBJECT | CF_NO_CONTACT_RESPONSE))
            == 0
    }
    pub fn set_anisotropic_friction(&mut self, f: Vec3, mode: i32) {
        self.anisotropic_friction = f;
        let is_unity = f.x != 1.0 || f.y != 1.0 || f.z != 1.0;
        self.has_anisotropic_friction = if is_unity { mode } else { 0 };
    }
    #[inline(always)]
    pub fn has_anisotropic_friction(&self, mode: i32) -> bool {
        (self.has_anisotropic_friction & mode) != 0
    }
    #[inline(always)]
    pub fn is_static_object(&self) -> bool {
        (self.collision_flags & CF_STATIC_OBJECT) != 0
    }
    #[inline(always)]
    pub fn is_kinematic_object(&self) -> bool {
        (self.collision_flags & CF_KINEMATIC_OBJECT) != 0
    }
    #[inline(always)]
    pub fn is_static_or_kinematic_object(&self) -> bool {
        (self.collision_flags & (CF_KINEMATIC_OBJECT | CF_STATIC_OBJECT)) != 0
    }
    #[inline(always)]
    pub fn has_contact_response(&self) -> bool {
        (self.collision_flags & CF_NO_CONTACT_RESPONSE) == 0
    }
    /// `setCollisionShape`.
    pub fn set_collision_shape(&mut self, shape: ShapeId) {
        self.update_revision += 1;
        self.collision_shape = Some(shape);
        self.root_collision_shape = Some(shape);
    }
    /// The shape (every object in the world has one).
    #[inline(always)]
    pub fn shape(&self) -> ShapeId {
        self.collision_shape
            .expect("collision object without shape")
    }
    #[inline(always)]
    pub fn get_activation_state(&self) -> i32 {
        self.activation_state1
    }
    /// `setActivationState`: ignored while deactivation or simulation is disabled.
    pub fn set_activation_state(&mut self, new_state: i32) {
        if self.activation_state1 != DISABLE_DEACTIVATION
            && self.activation_state1 != DISABLE_SIMULATION
        {
            self.activation_state1 = new_state;
        }
    }
    #[inline(always)]
    pub fn force_activation_state(&mut self, new_state: i32) {
        self.activation_state1 = new_state;
    }
    /// `activate(forceActivation)`.
    pub fn activate(&mut self, force_activation: bool) {
        if force_activation
            || (self.collision_flags & (CF_STATIC_OBJECT | CF_KINEMATIC_OBJECT)) == 0
        {
            self.set_activation_state(ACTIVE_TAG);
            self.deactivation_time = 0.0;
        }
    }
    #[inline(always)]
    pub fn is_active(&self) -> bool {
        self.activation_state1 != ISLAND_SLEEPING && self.activation_state1 != DISABLE_SIMULATION
    }
    pub fn set_restitution(&mut self, r: f64) {
        self.update_revision += 1;
        self.restitution = r;
    }
    pub fn set_friction(&mut self, f: f64) {
        self.update_revision += 1;
        self.friction = f;
    }
    pub fn set_rolling_friction(&mut self, f: f64) {
        self.update_revision += 1;
        self.rolling_friction = f;
    }
    pub fn set_world_transform(&mut self, t: Transform) {
        self.update_revision += 1;
        self.world_transform = t;
    }
    pub fn set_interpolation_world_transform(&mut self, t: Transform) {
        self.update_revision += 1;
        self.interpolation_world_transform = t;
    }
    pub fn set_interpolation_linear_velocity(&mut self, v: Vec3) {
        self.update_revision += 1;
        self.interpolation_linear_velocity = v;
    }
    pub fn set_interpolation_angular_velocity(&mut self, v: Vec3) {
        self.update_revision += 1;
        self.interpolation_angular_velocity = v;
    }
    #[inline(always)]
    pub fn get_ccd_square_motion_threshold(&self) -> f64 {
        self.ccd_motion_threshold * self.ccd_motion_threshold
    }
    /// `btRigidBody::upcast`.
    #[inline(always)]
    pub fn is_rigid_body(&self) -> bool {
        (self.internal_type & CO_RIGID_BODY) != 0
    }
    #[inline(always)]
    pub fn rb(&self) -> &RigidBody {
        self.body.as_deref().expect("not a rigid body")
    }
    #[inline(always)]
    pub fn rb_mut(&mut self) -> &mut RigidBody {
        self.body.as_deref_mut().expect("not a rigid body")
    }
}
