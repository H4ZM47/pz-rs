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
// Altered for pz-rs: Rust translation of btDiscreteDynamicsWorld (and the
// parts of btDynamicsWorld it uses) from Bullet 2.82, double precision.

//! `btDiscreteDynamicsWorld`.
//!
//! The game's glue hooks in through [`WorldHooks`]: the internal (pre-)tick
//! callbacks, the `saveKinematicState` override of `PZDiscreteDynamicsWorld`
//! and `gContactAddedCallback`. Actions (the raycast vehicles) implement
//! [`ActionInterface`].

use std::any::Any;

use super::rigid_body::RigidBodyConstructionInfo;
use super::sequential_impulse::SequentialImpulseConstraintSolver;
use super::solver_body::ContactSolverInfo;
use super::typed_constraint::{body_ref, ConstraintArena, TypedConstraint};
use crate::bt::broadphase::pair_cache::BroadphaseProxy;
use crate::bt::collision_object::*;
use crate::bt::collision_world::*;
use crate::bt::dispatch::{
    calculate_combined_friction, calculate_combined_restitution, obj, CollisionDispatcher,
    ContactAddedHook, ObjArena,
};
use crate::bt::island::SimulationIslandManager;
use crate::bt::linear_math::*;
use crate::bt::manifold::{ManifoldId, ManifoldPoint};
use crate::bt::shapes::Shape;

/// `btIDebugDraw::DBG_NoDeactivation`.
pub const DBG_NO_DEACTIVATION: i32 = 16;

/// Callbacks the game installs on the world.
pub trait WorldHooks: ContactAddedHook {
    /// `m_internalPreTickCallback` (called first in each substep).
    fn internal_pre_tick(&mut self, _world: &mut DiscreteDynamicsWorld, _time_step: f64) {}
    /// `m_internalTickCallback` (called last in each substep).
    fn internal_tick(&mut self, _world: &mut DiscreteDynamicsWorld, _time_step: f64) {}
    /// Override of `saveKinematicState`; return `true` when handled.
    fn save_kinematic_state(
        &mut self,
        _world: &mut DiscreteDynamicsWorld,
        _time_step: f64,
    ) -> bool {
        false
    }
}

/// No hooks installed.
pub struct NoHooks;
impl ContactAddedHook for NoHooks {}
impl WorldHooks for NoHooks {}

/// `btActionInterface`.
pub trait ActionInterface: Any {
    /// `updateAction(collisionWorld, deltaTimeStep)`.
    fn update_action(&mut self, world: &mut DiscreteDynamicsWorld, delta_time_step: f64);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Handle of an action added to the world.
pub type ActionId = usize;

/// `btDiscreteDynamicsWorld`.
pub struct DiscreteDynamicsWorld {
    pub cw: CollisionWorld,
    pub constraints: ConstraintArena,
    /// `m_constraints`.
    pub constraint_list: Vec<usize>,
    /// `m_sortedConstraints`.
    pub sorted_constraints: Vec<usize>,
    pub solver: SequentialImpulseConstraintSolver,
    pub island_manager: SimulationIslandManager,
    /// `m_nonStaticRigidBodies`.
    pub non_static_rigid_bodies: Vec<ObjId>,
    pub gravity: Vec3,
    pub local_time: f64,
    pub fixed_time_step: f64,
    pub synchronize_all_motion_states: bool,
    pub apply_speculative_contact_restitution: bool,
    pub latency_motion_state_interpolation: bool,
    /// `m_predictiveManifolds`.
    pub predictive_manifolds: Vec<ManifoldId>,
    pub solver_info: ContactSolverInfo,
    /// `m_actions` (with their handles).
    pub actions: Vec<(ActionId, Box<dyn ActionInterface>)>,
    next_action_id: ActionId,
    /// Mode of the installed debug drawer (`None`: no drawer). Each step
    /// sets `gDisableDeactivation` from `DBG_NoDeactivation`.
    pub debug_mode: Option<i32>,
    /// `gDisableDeactivation` (as last set by `stepSimulation`).
    pub disable_deactivation: bool,
    // InplaceSolverIslandCallback batch state
    cb_bodies: Vec<ObjId>,
    cb_manifolds: Vec<ManifoldId>,
    cb_constraints: Vec<usize>,
}

impl Default for DiscreteDynamicsWorld {
    fn default() -> Self {
        DiscreteDynamicsWorld::new()
    }
}

/// `btGetConstraintIslandId`.
fn constraint_island_id(objects: &ObjArena, c: &TypedConstraint) -> i32 {
    let a = body_ref(objects, c.rb_a).island_tag1;
    if a >= 0 {
        a
    } else {
        body_ref(objects, c.rb_b).island_tag1
    }
}

impl DiscreteDynamicsWorld {
    /// `btDiscreteDynamicsWorld(dispatcher, broadphase, solver, configuration)`.
    pub fn new() -> DiscreteDynamicsWorld {
        DiscreteDynamicsWorld {
            cw: CollisionWorld::new(),
            constraints: ConstraintArena::default(),
            constraint_list: Vec::new(),
            sorted_constraints: Vec::new(),
            solver: SequentialImpulseConstraintSolver::new(),
            island_manager: SimulationIslandManager::new(),
            non_static_rigid_bodies: Vec::new(),
            gravity: Vec3::new(0.0, -10.0, 0.0),
            local_time: 0.0,
            fixed_time_step: 0.0,
            synchronize_all_motion_states: false,
            apply_speculative_contact_restitution: false,
            latency_motion_state_interpolation: true,
            predictive_manifolds: Vec::new(),
            solver_info: ContactSolverInfo::default(),
            actions: Vec::new(),
            next_action_id: 0,
            debug_mode: None,
            disable_deactivation: false,
            cb_bodies: Vec::new(),
            cb_manifolds: Vec::new(),
            cb_constraints: Vec::new(),
        }
    }

    #[inline]
    pub fn obj(&self, id: ObjId) -> &CollisionObject {
        self.cw.obj(id)
    }
    #[inline]
    pub fn obj_mut(&mut self, id: ObjId) -> &mut CollisionObject {
        self.cw.obj_mut(id)
    }

    /// Creates a `btRigidBody` in the object arena (not yet in the world).
    pub fn create_rigid_body(&mut self, info: &RigidBodyConstructionInfo) -> ObjId {
        self.cw.insert_object(CollisionObject::new_rigid_body(info))
    }

    /// `saveKinematicState` (the base implementation).
    pub fn save_kinematic_state_default(&mut self, time_step: f64) {
        for i in 0..self.cw.collision_objects.len() {
            let id = self.cw.collision_objects[i];
            let o = self.cw.obj_mut(id);
            if o.is_rigid_body()
                && o.get_activation_state() != ISLAND_SLEEPING
                && o.is_kinematic_object()
            {
                o.save_kinematic_state(time_step);
            }
        }
    }

    fn save_kinematic_state(&mut self, time_step: f64, hooks: &mut dyn WorldHooks) {
        if !hooks.save_kinematic_state(self, time_step) {
            self.save_kinematic_state_default(time_step);
        }
    }

    /// `clearForces`.
    pub fn clear_forces(&mut self) {
        for i in 0..self.non_static_rigid_bodies.len() {
            let id = self.non_static_rigid_bodies[i];
            self.cw.obj_mut(id).clear_forces();
        }
    }

    /// `applyGravity`.
    pub fn apply_gravity(&mut self) {
        for i in 0..self.non_static_rigid_bodies.len() {
            let id = self.non_static_rigid_bodies[i];
            let o = self.cw.obj_mut(id);
            if o.is_active() {
                o.apply_gravity();
            }
        }
    }

    /// `synchronizeSingleMotionState`.
    pub fn synchronize_single_motion_state(&mut self, id: ObjId) {
        let t = if self.latency_motion_state_interpolation && self.fixed_time_step != 0.0 {
            self.local_time - self.fixed_time_step
        } else {
            self.local_time * self.cw.obj(id).hit_fraction
        };
        let o = self.cw.obj_mut(id);
        if o.rb().optional_motion_state.is_some() && !o.is_static_or_kinematic_object() {
            let interpolated = transform_util::integrate_transform(
                &o.interpolation_world_transform,
                &o.interpolation_linear_velocity,
                &o.interpolation_angular_velocity,
                t,
            );
            o.rb_mut()
                .optional_motion_state
                .as_mut()
                .unwrap()
                .set_world_transform(&interpolated);
        }
    }

    /// `synchronizeMotionStates`.
    pub fn synchronize_motion_states(&mut self) {
        if self.synchronize_all_motion_states {
            for i in 0..self.cw.collision_objects.len() {
                let id = self.cw.collision_objects[i];
                if self.cw.obj(id).is_rigid_body() {
                    self.synchronize_single_motion_state(id);
                }
            }
        } else {
            for i in 0..self.non_static_rigid_bodies.len() {
                let id = self.non_static_rigid_bodies[i];
                if self.cw.obj(id).is_active() {
                    self.synchronize_single_motion_state(id);
                }
            }
        }
    }

    /// `stepSimulation(timeStep, maxSubSteps, fixedTimeStep)`.
    pub fn step_simulation(
        &mut self,
        time_step: f64,
        mut max_sub_steps: i32,
        mut fixed_time_step: f64,
        hooks: &mut dyn WorldHooks,
    ) -> i32 {
        let mut num_simulation_sub_steps = 0;
        if max_sub_steps != 0 {
            self.fixed_time_step = fixed_time_step;
            self.local_time += time_step;
            if self.local_time >= fixed_time_step {
                num_simulation_sub_steps = (self.local_time / fixed_time_step) as i32;
                // `m_localTime -= n * fixedTimeStep` (fused)
                self.local_time = fma(
                    -(num_simulation_sub_steps as f64),
                    fixed_time_step,
                    self.local_time,
                );
            }
        } else {
            fixed_time_step = time_step;
            self.local_time = if self.latency_motion_state_interpolation {
                0.0
            } else {
                time_step
            };
            self.fixed_time_step = 0.0;
            if bt_fuzzy_zero(time_step) {
                num_simulation_sub_steps = 0;
                max_sub_steps = 0;
            } else {
                num_simulation_sub_steps = 1;
                max_sub_steps = 1;
            }
        }
        if let Some(mode) = self.debug_mode {
            self.disable_deactivation = (mode & DBG_NO_DEACTIVATION) != 0;
        }
        if num_simulation_sub_steps != 0 {
            let clamped = if num_simulation_sub_steps > max_sub_steps {
                max_sub_steps
            } else {
                num_simulation_sub_steps
            };
            self.save_kinematic_state(fixed_time_step * clamped as f64, hooks);
            self.apply_gravity();
            for _ in 0..clamped {
                self.internal_single_step_simulation(fixed_time_step, hooks);
                self.synchronize_motion_states();
            }
        } else {
            self.synchronize_motion_states();
        }
        self.clear_forces();
        num_simulation_sub_steps
    }

    /// `internalSingleStepSimulation(timeStep)`.
    pub fn internal_single_step_simulation(&mut self, time_step: f64, hooks: &mut dyn WorldHooks) {
        hooks.internal_pre_tick(self, time_step);
        self.predict_unconstraint_motion(time_step);
        self.cw.dispatch_info.time_step = time_step;
        self.cw.dispatch_info.step_count = 0;
        self.create_predictive_contacts(time_step);
        self.cw
            .perform_discrete_collision_detection(Some(&self.constraints), hooks);
        self.calculate_simulation_islands();
        self.solver_info.time_step = time_step;
        self.solve_constraints();
        self.integrate_transforms(time_step);
        self.update_actions(time_step);
        self.update_activation_state(time_step);
        hooks.internal_tick(self, time_step);
    }

    /// `setGravity`.
    pub fn set_gravity(&mut self, gravity: &Vec3) {
        self.gravity = *gravity;
        for i in 0..self.non_static_rigid_bodies.len() {
            let id = self.non_static_rigid_bodies[i];
            let o = self.cw.obj_mut(id);
            if o.is_active() && (o.rb().rigidbody_flags & BT_DISABLE_WORLD_GRAVITY) == 0 {
                o.set_gravity(gravity);
            }
        }
    }

    /// `addCollisionObject(obj, group, mask)` for a plain collision object.
    pub fn add_collision_object(&mut self, id: ObjId, group: i16, mask: i16) {
        self.cw.add_collision_object(id, group, mask);
    }

    /// `removeCollisionObject` (rigid bodies go through `removeRigidBody`).
    pub fn remove_collision_object(&mut self, id: ObjId) {
        if self.cw.obj(id).is_rigid_body() {
            self.remove_rigid_body(id);
        } else {
            self.cw.remove_collision_object(id);
        }
    }

    /// `removeRigidBody`.
    pub fn remove_rigid_body(&mut self, id: ObjId) {
        if let Some(i) = self.non_static_rigid_bodies.iter().position(|&o| o == id) {
            let last = self.non_static_rigid_bodies.len() - 1;
            self.non_static_rigid_bodies.swap(i, last);
            self.non_static_rigid_bodies.pop();
        }
        self.cw.remove_collision_object(id);
    }

    /// `addRigidBody(body)`.
    pub fn add_rigid_body(&mut self, id: ObjId) {
        let gravity = self.gravity;
        let o = self.cw.obj_mut(id);
        if !o.is_static_or_kinematic_object()
            && (o.rb().rigidbody_flags & BT_DISABLE_WORLD_GRAVITY) == 0
        {
            o.set_gravity(&gravity);
        }
        if o.collision_shape.is_some() {
            if !o.is_static_object() {
                self.non_static_rigid_bodies.push(id);
            } else {
                o.set_activation_state(ISLAND_SLEEPING);
            }
            let o = self.cw.obj(id);
            let is_dynamic = !(o.is_static_object() || o.is_kinematic_object());
            let group = if is_dynamic {
                DEFAULT_FILTER
            } else {
                STATIC_FILTER
            };
            let mask = if is_dynamic {
                ALL_FILTER
            } else {
                ALL_FILTER ^ STATIC_FILTER
            };
            self.add_collision_object(id, group, mask);
        }
    }

    /// `addRigidBody(body, group, mask)`.
    pub fn add_rigid_body_filtered(&mut self, id: ObjId, group: i16, mask: i16) {
        let gravity = self.gravity;
        let o = self.cw.obj_mut(id);
        if !o.is_static_or_kinematic_object()
            && (o.rb().rigidbody_flags & BT_DISABLE_WORLD_GRAVITY) == 0
        {
            o.set_gravity(&gravity);
        }
        if o.collision_shape.is_some() {
            if !o.is_static_object() {
                self.non_static_rigid_bodies.push(id);
            } else {
                o.set_activation_state(ISLAND_SLEEPING);
            }
            self.add_collision_object(id, group, mask);
        }
    }

    /// `updateActions`.
    pub fn update_actions(&mut self, time_step: f64) {
        let mut actions = std::mem::take(&mut self.actions);
        for (_, a) in actions.iter_mut() {
            a.update_action(self, time_step);
        }
        // Actions added during the update (none in practice) go after.
        actions.append(&mut self.actions);
        self.actions = actions;
    }

    /// `updateActivationState`.
    pub fn update_activation_state(&mut self, time_step: f64) {
        let disable = self.disable_deactivation;
        for i in 0..self.non_static_rigid_bodies.len() {
            let id = self.non_static_rigid_bodies[i];
            let o = self.cw.obj_mut(id);
            o.update_deactivation(time_step);
            // wantsSleeping() is false while gDisableDeactivation is set.
            let wants = !disable && o.wants_sleeping();
            if wants {
                if o.is_static_or_kinematic_object() {
                    o.set_activation_state(ISLAND_SLEEPING);
                } else {
                    if o.get_activation_state() == ACTIVE_TAG {
                        o.set_activation_state(WANTS_DEACTIVATION);
                    }
                    if o.get_activation_state() == ISLAND_SLEEPING {
                        o.set_angular_velocity(&Vec3::new(0.0, 0.0, 0.0));
                        o.set_linear_velocity(&Vec3::new(0.0, 0.0, 0.0));
                    }
                }
            } else if o.get_activation_state() != DISABLE_DEACTIVATION {
                o.set_activation_state(ACTIVE_TAG);
            }
        }
    }

    /// `addConstraint(constraint, disableCollisionsBetweenLinkedBodies)`.
    pub fn add_constraint(
        &mut self,
        c: TypedConstraint,
        disable_collisions_between_linked_bodies: bool,
    ) -> usize {
        let (a, b) = (c.rb_a, c.rb_b);
        let id = self.constraints.insert(c);
        self.constraint_list.push(id);
        if disable_collisions_between_linked_bodies {
            if let Some(a) = a {
                self.cw.obj_mut(a).add_constraint_ref(id);
            }
            if let Some(b) = b {
                self.cw.obj_mut(b).add_constraint_ref(id);
            }
        }
        id
    }

    /// `removeConstraint(constraint)`: the constraint stays in the arena
    /// (call `constraints.remove` to delete it).
    pub fn remove_constraint(&mut self, id: usize) {
        if let Some(i) = self.constraint_list.iter().position(|&c| c == id) {
            let last = self.constraint_list.len() - 1;
            self.constraint_list.swap(i, last);
            self.constraint_list.pop();
        }
        let (a, b) = {
            let c = self.constraints.get(id);
            (c.rb_a, c.rb_b)
        };
        if let Some(a) = a {
            self.cw.obj_mut(a).remove_constraint_ref(id);
        }
        if let Some(b) = b {
            self.cw.obj_mut(b).remove_constraint_ref(id);
        }
    }

    /// `addAction(action)`.
    pub fn add_action(&mut self, action: Box<dyn ActionInterface>) -> ActionId {
        let id = self.next_action_id;
        self.next_action_id += 1;
        self.actions.push((id, action));
        id
    }

    /// `removeAction(action)` (swap-remove); returns the action.
    pub fn remove_action(&mut self, id: ActionId) -> Option<Box<dyn ActionInterface>> {
        let i = self.actions.iter().position(|(a, _)| *a == id)?;
        let last = self.actions.len() - 1;
        self.actions.swap(i, last);
        self.actions.pop().map(|(_, a)| a)
    }

    pub fn action(&self, id: ActionId) -> Option<&dyn ActionInterface> {
        self.actions
            .iter()
            .find(|(a, _)| *a == id)
            .map(|(_, a)| a.as_ref())
    }
    pub fn action_mut(&mut self, id: ActionId) -> Option<&mut Box<dyn ActionInterface>> {
        self.actions
            .iter_mut()
            .find(|(a, _)| *a == id)
            .map(|(_, a)| a)
    }

    /// `solveConstraints(solverInfo)`.
    pub fn solve_constraints(&mut self) {
        self.sorted_constraints.clear();
        self.sorted_constraints
            .extend_from_slice(&self.constraint_list);
        {
            let objects = &self.cw.objects;
            let arena = &self.constraints;
            quick_sort(&mut self.sorted_constraints, |l, r| {
                let r_id = constraint_island_id(objects, arena.get(*r));
                let l_id = constraint_island_id(objects, arena.get(*l));
                l_id < r_id
            });
        }
        self.cb_bodies.clear();
        self.cb_manifolds.clear();
        self.cb_constraints.clear();
        let DiscreteDynamicsWorld {
            cw,
            constraints,
            sorted_constraints,
            solver,
            island_manager,
            solver_info,
            cb_bodies,
            cb_manifolds,
            cb_constraints,
            ..
        } = self;
        let num_constraints = sorted_constraints.len();
        island_manager.build_and_process_islands(cw, &mut |cw, bodies, manifolds, island_id| {
            // islandId >= 0 always (split islands)
            let mut start = None;
            let mut num_cur = 0;
            let mut i = 0;
            while i < num_constraints {
                if constraint_island_id(&cw.objects, constraints.get(sorted_constraints[i]))
                    == island_id
                {
                    start = Some(i);
                    break;
                }
                i += 1;
            }
            while i < num_constraints {
                if constraint_island_id(&cw.objects, constraints.get(sorted_constraints[i]))
                    == island_id
                {
                    num_cur += 1;
                }
                i += 1;
            }
            let cur: &[usize] = match start {
                Some(s) => &sorted_constraints[s..s + num_cur],
                None => &[],
            };
            if solver_info.minimum_solver_batch_size <= 1 {
                solver.solve_group(
                    &mut cw.objects,
                    bodies,
                    &mut cw.dispatcher.manifolds,
                    manifolds,
                    constraints,
                    cur,
                    solver_info,
                );
            } else {
                cb_bodies.extend_from_slice(bodies);
                cb_manifolds.extend_from_slice(manifolds);
                cb_constraints.extend_from_slice(cur);
                if (cb_constraints.len() + cb_manifolds.len()) as i32
                    > solver_info.minimum_solver_batch_size
                {
                    solver.solve_group(
                        &mut cw.objects,
                        cb_bodies,
                        &mut cw.dispatcher.manifolds,
                        cb_manifolds,
                        constraints,
                        cb_constraints,
                        solver_info,
                    );
                    cb_bodies.clear();
                    cb_manifolds.clear();
                    cb_constraints.clear();
                }
            }
        });
        // processConstraints()
        solver.solve_group(
            &mut cw.objects,
            cb_bodies,
            &mut cw.dispatcher.manifolds,
            cb_manifolds,
            constraints,
            cb_constraints,
            solver_info,
        );
        cb_bodies.clear();
        cb_manifolds.clear();
        cb_constraints.clear();
    }

    /// `calculateSimulationIslands`.
    pub fn calculate_simulation_islands(&mut self) {
        self.island_manager.update_activation_state(&mut self.cw);
        for i in 0..self.predictive_manifolds.len() {
            let m = self.predictive_manifolds[i];
            let (b0, b1) = {
                let mf = self.cw.dispatcher.manifold(m);
                (mf.body0, mf.body1)
            };
            let c0 = self.cw.obj(b0);
            let c1 = self.cw.obj(b1);
            if !c0.is_static_or_kinematic_object() && !c1.is_static_or_kinematic_object() {
                let (t0, t1) = (c0.island_tag1, c1.island_tag1);
                self.island_manager.union_find.unite(t0, t1);
            }
        }
        for i in 0..self.constraint_list.len() {
            let c = self.constraints.get(self.constraint_list[i]);
            if c.is_enabled {
                let c0 = body_ref(&self.cw.objects, c.rb_a);
                let c1 = body_ref(&self.cw.objects, c.rb_b);
                if !c0.is_static_or_kinematic_object() && !c1.is_static_or_kinematic_object() {
                    let (t0, t1) = (c0.island_tag1, c1.island_tag1);
                    self.island_manager.union_find.unite(t0, t1);
                }
            }
        }
        self.island_manager
            .store_island_activation_state(&mut self.cw);
    }

    /// CCD sweep shared by `createPredictiveContacts` and `integrateTransforms`.
    fn ccd_sweep(
        &self,
        id: ObjId,
        predicted_trans: &Transform,
    ) -> ClosestNotMeConvexResultCallback {
        let o = self.cw.obj(id);
        let mut sweep_results = ClosestNotMeConvexResultCallback::new(
            id,
            o.world_transform.origin,
            predicted_trans.origin,
        );
        let tmp_sphere = Shape::new_sphere(o.ccd_swept_sphere_radius);
        sweep_results.allowed_penetration = self.cw.dispatch_info.allowed_ccd_penetration;
        let proxy = self
            .cw
            .broadphase
            .proxy(o.broadphase_handle.expect("no proxy"))
            .base;
        sweep_results.inner.base.collision_filter_group = proxy.collision_filter_group;
        sweep_results.inner.base.collision_filter_mask = proxy.collision_filter_mask;
        let mut modified_predicted_trans = *predicted_trans;
        modified_predicted_trans.basis = o.world_transform.basis;
        let wt = o.world_transform;
        self.cw.convex_sweep_test(
            &tmp_sphere,
            &wt,
            &modified_predicted_trans,
            &mut sweep_results,
            0.0,
        );
        sweep_results
    }

    /// `createPredictiveContacts(timeStep)`.
    pub fn create_predictive_contacts(&mut self, time_step: f64) {
        for i in 0..self.predictive_manifolds.len() {
            let m = self.predictive_manifolds[i];
            self.cw.dispatcher.release_manifold(m);
        }
        self.predictive_manifolds.clear();
        for i in 0..self.non_static_rigid_bodies.len() {
            let id = self.non_static_rigid_bodies[i];
            self.cw.obj_mut(id).hit_fraction = 1.0;
            let o = self.cw.obj(id);
            if o.is_active() && !o.is_static_or_kinematic_object() {
                let predicted_trans = o.predict_integrated_transform(time_step);
                let square_motion = (predicted_trans.origin - o.world_transform.origin).length2();
                if self.cw.dispatch_info.use_continuous
                    && o.get_ccd_square_motion_threshold() != 0.0
                    && o.get_ccd_square_motion_threshold() < square_motion
                    && self.cw.shapes.get(o.shape()).is_convex()
                {
                    let sweep_results = self.ccd_sweep(id, &predicted_trans);
                    if sweep_results.has_hit()
                        && sweep_results.inner.base.closest_hit_fraction < 1.0
                    {
                        let o = self.cw.obj(id);
                        let hit = sweep_results.inner.hit_collision_object.unwrap();
                        let dist_vec = (predicted_trans.origin - o.world_transform.origin)
                            * sweep_results.inner.base.closest_hit_fraction;
                        let distance = dist_vec.dot(&-sweep_results.inner.hit_normal_world);
                        let m = self.cw.dispatcher.get_new_manifold(
                            &self.cw.shapes,
                            &self.cw.objects,
                            id,
                            hit,
                        );
                        self.predictive_manifolds.push(m);
                        let o = self.cw.obj(id);
                        let h = self.cw.obj(hit);
                        let world_point_b = o.world_transform.origin + dist_vec;
                        let local_point_b = h.world_transform.inverse().apply(&world_point_b);
                        let new_point = ManifoldPoint::new(
                            Vec3::new(0.0, 0.0, 0.0),
                            local_point_b,
                            sweep_results.inner.hit_normal_world,
                            distance,
                        );
                        let friction = calculate_combined_friction(o, h);
                        let origin = o.world_transform.origin;
                        let mf = self.cw.dispatcher.manifold_mut(m);
                        let index = mf.add_manifold_point(&new_point) as usize;
                        let pt = mf.get_contact_point_mut(index);
                        pt.combined_restitution = 0.0;
                        pt.combined_friction = friction;
                        pt.position_world_on_a = origin;
                        pt.position_world_on_b = world_point_b;
                    }
                }
            }
        }
    }

    /// `integrateTransforms(timeStep)`.
    pub fn integrate_transforms(&mut self, time_step: f64) {
        for i in 0..self.non_static_rigid_bodies.len() {
            let id = self.non_static_rigid_bodies[i];
            self.cw.obj_mut(id).hit_fraction = 1.0;
            let o = self.cw.obj(id);
            if o.is_active() && !o.is_static_or_kinematic_object() {
                let mut predicted_trans = o.predict_integrated_transform(time_step);
                let square_motion = (predicted_trans.origin - o.world_transform.origin).length2();
                if self.cw.dispatch_info.use_continuous
                    && o.get_ccd_square_motion_threshold() != 0.0
                    && o.get_ccd_square_motion_threshold() < square_motion
                    && self.cw.shapes.get(o.shape()).is_convex()
                {
                    let sweep_results = self.ccd_sweep(id, &predicted_trans);
                    if sweep_results.has_hit()
                        && sweep_results.inner.base.closest_hit_fraction < 1.0
                    {
                        let o = self.cw.obj_mut(id);
                        o.hit_fraction = sweep_results.inner.base.closest_hit_fraction;
                        predicted_trans =
                            o.predict_integrated_transform(time_step * o.hit_fraction);
                        o.hit_fraction = 0.0;
                        o.proceed_to_transform(&predicted_trans);
                        continue;
                    }
                }
                self.cw.obj_mut(id).proceed_to_transform(&predicted_trans);
            }
        }
        if self.apply_speculative_contact_restitution {
            for i in 0..self.predictive_manifolds.len() {
                let m = self.predictive_manifolds[i];
                let (b0, b1, n) = {
                    let mf = self.cw.dispatcher.manifold(m);
                    (mf.body0, mf.body1, mf.get_num_contacts())
                };
                for p in 0..n as usize {
                    let pt = *self.cw.dispatcher.manifold(m).get_contact_point(p);
                    let combined_restitution =
                        calculate_combined_restitution(self.cw.obj(b0), self.cw.obj(b1));
                    if combined_restitution > 0.0 && pt.applied_impulse != 0.0 {
                        let imp = -pt.normal_world_on_b * pt.applied_impulse * combined_restitution;
                        let rel_pos0 =
                            pt.position_world_on_a - self.cw.obj(b0).world_transform.origin;
                        let rel_pos1 =
                            pt.position_world_on_b - self.cw.obj(b1).world_transform.origin;
                        if self.cw.obj(b0).is_rigid_body() {
                            self.cw.obj_mut(b0).apply_impulse(&imp, &rel_pos0);
                        }
                        if self.cw.obj(b1).is_rigid_body() {
                            self.cw.obj_mut(b1).apply_impulse(&-imp, &rel_pos1);
                        }
                    }
                }
            }
        }
    }

    /// `predictUnconstraintMotion(timeStep)`.
    pub fn predict_unconstraint_motion(&mut self, time_step: f64) {
        for i in 0..self.non_static_rigid_bodies.len() {
            let id = self.non_static_rigid_bodies[i];
            let o = self.cw.obj_mut(id);
            if !o.is_static_or_kinematic_object() {
                o.apply_damping(time_step);
                o.interpolation_world_transform = o.predict_integrated_transform(time_step);
            }
        }
    }
}

/// `btClosestNotMeConvexResultCallback`.
pub struct ClosestNotMeConvexResultCallback {
    pub inner: ClosestConvexResultCallback,
    pub me: ObjId,
    pub allowed_penetration: f64,
}

impl ClosestNotMeConvexResultCallback {
    pub fn new(me: ObjId, from: Vec3, to: Vec3) -> Self {
        ClosestNotMeConvexResultCallback {
            inner: ClosestConvexResultCallback::new(from, to),
            me,
            allowed_penetration: 0.0,
        }
    }
}

impl ConvexResultCallback for ClosestNotMeConvexResultCallback {
    fn base(&self) -> &ConvexResultBase {
        &self.inner.base
    }
    fn base_mut(&mut self) -> &mut ConvexResultBase {
        &mut self.inner.base
    }
    fn needs_collision(&self, proxy: &BroadphaseProxy, objects: &ObjArena) -> bool {
        if proxy.client_object == self.me {
            return false;
        }
        let b = &self.inner.base;
        if !default_needs_collision(b.collision_filter_group, b.collision_filter_mask, proxy) {
            return false;
        }
        CollisionDispatcher::needs_response(
            obj(objects, self.me),
            obj(objects, proxy.client_object),
        )
    }
    fn add_single_result(
        &mut self,
        r: &mut LocalConvexResult,
        normal_in_world_space: bool,
        objects: &ObjArena,
    ) -> f64 {
        if r.hit_collision_object == self.me {
            return 1.0;
        }
        if !obj(objects, r.hit_collision_object).has_contact_response() {
            return 1.0;
        }
        let lin_vel_a = self.inner.convex_to_world - self.inner.convex_from_world;
        let lin_vel_b = Vec3::new(0.0, 0.0, 0.0);
        let relative_velocity = lin_vel_a - lin_vel_b;
        if r.hit_normal_local.dot(&relative_velocity) >= -self.allowed_penetration {
            return 1.0;
        }
        self.inner
            .closest_add_single_result(r, normal_in_world_space, objects)
    }
}

/// The collision world as seen by a raycast vehicle: its bodies plus the
/// game's `PZVehicleRaycaster::castRay` (closest hit with filter group 2 and
/// mask 13; only rigid bodies with contact response count; the hit normal is
/// normalised; the fraction is the callback's closest hit fraction).
pub struct PzVehicleRays<'a> {
    pub cw: &'a mut CollisionWorld,
}

impl super::typed_constraint::BodyAccess for PzVehicleRays<'_> {
    fn obj(&self, id: ObjId) -> &CollisionObject {
        self.cw.obj(id)
    }
}
impl super::typed_constraint::BodyArena for PzVehicleRays<'_> {
    fn obj_mut(&mut self, id: ObjId) -> &mut CollisionObject {
        self.cw.obj_mut(id)
    }
}
impl super::raycast_vehicle::VehicleWorld for PzVehicleRays<'_> {
    fn cast_ray(
        &self,
        from: &Vec3,
        to: &Vec3,
    ) -> Option<super::raycast_vehicle::VehicleRaycasterResult> {
        pz_vehicle_cast_ray(self.cw, from, to)
    }
}

/// `PZVehicleRaycaster::castRay`.
pub fn pz_vehicle_cast_ray(
    cw: &CollisionWorld,
    from: &Vec3,
    to: &Vec3,
) -> Option<super::raycast_vehicle::VehicleRaycasterResult> {
    let mut cb = ClosestRayResultCallback::new(*from, *to);
    cb.base.collision_filter_group = 2;
    cb.base.collision_filter_mask = 13;
    cw.ray_test(from, to, &mut cb);
    let hit = cb.base.collision_object?;
    let o = cw.obj(hit);
    if !o.is_rigid_body() || !o.has_contact_response() {
        return None;
    }
    let mut n = cb.hit_normal_world;
    n.normalize();
    Some(super::raycast_vehicle::VehicleRaycasterResult {
        hit_point_in_world: cb.hit_point_world,
        hit_normal_in_world: n,
        dist_fraction: cb.base.closest_hit_fraction,
        hit_object: hit,
    })
}

/// A `btRaycastVehicle` in the world's action list, casting rays with
/// [`pz_vehicle_cast_ray`].
impl ActionInterface for super::raycast_vehicle::RaycastVehicle {
    fn update_action(&mut self, world: &mut DiscreteDynamicsWorld, delta_time_step: f64) {
        let mut rays = PzVehicleRays { cw: &mut world.cw };
        self.update_vehicle_in(&mut rays, delta_time_step);
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
