//! The ragdoll natives of `zombie.core.physics.Bullet`, one method each.
//!
//! Java's `RagdollController` drives a ragdoll per character: `addRagdoll`,
//! then every frame `updateRagdoll` (world position, in Bullet space: x,
//! z * 2.44949, y), `setRagdollLocalTransformRotation`,
//! `simulateRagdollWithRigidBodyOutput` and `getRagdollSimulationState`;
//! `updateRagdollSkeletonTransforms` uploads the animated pose. Bone arrays
//! hold 7 floats per `SkeletonBone` (-position * 1.5, then the rotation
//! quaternion x, y, z, w); rigid-body arrays 7 floats per `RagdollBodyPart`
//! (position, rotation).
//!
//! Where the original dereferences a missing world or ragdoll and crashes,
//! these methods do nothing.

use crate::api::{JniError, PzBullet};
use crate::bt::linear_math::*;
use crate::pz::ragdoll::{RagdollCtx, BODY_PART_COUNT};
use crate::pz::ragdoll_script::{RagdollBodyDynamics, RagdollScript};
use crate::pz::ragdoll_skeleton::{validate_against_java, SkeletonBoneCallbacks};
use crate::pz::world::WorldSimulation;

const NULL_WORLD: &str = "WorldSimulation::instance is null";

impl PzBullet {
    fn ragdoll_world(&mut self) -> Option<&mut WorldSimulation> {
        self.world.as_deref_mut()
    }

    /// `initPZBullet()` followed by its `SkeletonBone::ValidateAgainstJava()`
    /// (which only logs); returns whether every bone matched.
    pub fn init_pz_bullet_with_bones(&mut self, cb: &mut dyn SkeletonBoneCallbacks) -> bool {
        self.init_pz_bullet();
        validate_against_java(cb)
    }

    /// `initializeRagdollSkeleton(numberOfBones, boneHierarchy)`: the parent
    /// of each bone (`RagdollBuilder.getSkinningDataSkeletonHierarchy`).
    pub fn initialize_ragdoll_skeleton(&mut self, n: i32, hierarchy: &[i32]) {
        self.ragdoll_builder
            .initialize_skeleton_hierarchy(n, hierarchy);
    }

    /// `initializeRagdollPose(numberOfBones, boneTransforms, qx, qy, qz,
    /// qw)`: the T-pose bones (7 floats each) and the local rotation
    /// (`RagdollController.getRagdollLocalRotation`). Builds the default
    /// skeleton and the body-part shapes; needs `initializeRagdollSkeleton`,
    /// `initWorld` and the body-part definitions first.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize_ragdoll_pose(
        &mut self,
        n: i32,
        bones: &[f32],
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
    ) {
        self.ragdoll_builder
            .initialize_skeleton(n, bones, qx, qy, qz, qw);
    }

    /// `addRagdoll(id, x, y, z, qx, qy, qz, qw)`. The position and rotation
    /// are not used: the bodies are placed by `updateRagdoll` and
    /// `updateRagdollSkeletonTransforms`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_ragdoll(
        &mut self,
        id: i32,
        _x: f32,
        _y: f32,
        _z: f32,
        _qx: f32,
        _qy: f32,
        _qz: f32,
        _qw: f32,
    ) -> Result<(), JniError> {
        let Some(ws) = self.world.as_deref_mut() else {
            return Err(JniError::NullPointer(NULL_WORLD));
        };
        ws.ragdoll.alloc(&self.ragdoll_builder, &mut ws.dw, id);
        Ok(())
    }

    /// `removeRagdoll(id)`: the ragdoll leaves the world and goes back to the
    /// pool (keeping its bodies, mass and dynamics for its next use).
    pub fn remove_ragdoll(&mut self, id: i32) -> Result<(), JniError> {
        let Some(ws) = self.world.as_deref_mut() else {
            return Err(JniError::NullPointer(NULL_WORLD));
        };
        ws.ragdoll.remove(&mut ws.dw, id);
        Ok(())
    }

    /// The checks the simulate natives share. `Err(Ok(-1))`: the first call
    /// of a native logs its error, which clears the exception.
    fn check_sim(&mut self, which: usize, id: i32) -> Result<(), Result<i32, JniError>> {
        let first = !self.ragdoll_builder.jni_once[which];
        self.ragdoll_builder.jni_once[which] = true;
        let e = if self.world.is_none() {
            JniError::Runtime(NULL_WORLD)
        } else if id < 0 {
            JniError::IllegalArgument("id is invalid")
        } else {
            return Ok(());
        };
        Err(if first { Ok(-1) } else { Err(e) })
    }

    /// `simulateRagdoll(id, skeletonOut)`: number of bones written, 0 for
    /// an unknown id.
    pub fn simulate_ragdoll(&mut self, id: i32, skel: &mut [f32]) -> Result<i32, JniError> {
        if let Err(r) = self.check_sim(0, id) {
            return r;
        }
        if skel.is_empty() {
            return Err(JniError::IllegalArgument("output array is too small"));
        }
        let ws = self.world.as_deref_mut().unwrap();
        Ok(match ws.ragdoll.ragdolls.get_mut(&id) {
            Some(r) => r.simulate(&ws.dw, skel),
            None => 0,
        })
    }

    /// `simulateRagdollWithRigidBodyOutput(id, skeletonOut, rigidBodyOut)`:
    /// also writes the bodies (7 floats per part) and clamps their speed.
    pub fn simulate_ragdoll_with_rigid_body_output(
        &mut self,
        id: i32,
        skel: &mut [f32],
        rb: &mut [f32],
    ) -> Result<i32, JniError> {
        if let Err(r) = self.check_sim(1, id) {
            return r;
        }
        if skel.is_empty() {
            return Err(JniError::IllegalArgument(
                "output skeletonBuffer is too small",
            ));
        }
        if rb.is_empty() {
            return Err(JniError::IllegalArgument(
                "output rigidBodyBuffer is too small",
            ));
        }
        let ws = self.world.as_deref_mut().unwrap();
        Ok(match ws.ragdoll.ragdolls.get_mut(&id) {
            Some(r) => r.simulate_with_rigid_bodies(&mut ws.dw, skel, rb),
            None => 0,
        })
    }

    /// `updateSkeletonFromNetworkPhysics(id, rigidBodyIn, skeletonOut)`: the
    /// rigid-body array is not read; writes the skeleton like
    /// `simulateRagdoll` and returns 11 (0 for an unknown id).
    pub fn update_skeleton_from_network_physics(
        &mut self,
        id: i32,
        rb: &mut [f32],
        skel: &mut [f32],
    ) -> Result<i32, JniError> {
        if let Err(r) = self.check_sim(2, id) {
            return r;
        }
        if skel.is_empty() {
            return Err(JniError::IllegalArgument(
                "output skeletonBuffer is too small",
            ));
        }
        if rb.is_empty() {
            return Err(JniError::IllegalArgument(
                "output rigidBodyBuffer is too small",
            ));
        }
        let ws = self.world.as_deref_mut().unwrap();
        Ok(match ws.ragdoll.ragdolls.get_mut(&id) {
            Some(r) => {
                r.simulate(&ws.dw, skel);
                BODY_PART_COUNT as i32
            }
            None => 0,
        })
    }

    /// `getCorrectedWorldSpace(id, out)`: the ragdoll's position from the
    /// last `updateRagdoll` (offset-relative), 3 floats.
    pub fn get_corrected_world_space(&mut self, id: i32, out: &mut [f32]) -> Result<(), JniError> {
        if id < 0 {
            return Err(JniError::IllegalArgument("id is invalid"));
        }
        if out.is_empty() {
            return Err(JniError::IllegalArgument("output array is too small"));
        }
        let Some(ws) = self.ragdoll_world() else {
            return Ok(());
        };
        if let Some(r) = ws.ragdoll.ragdolls.get(&id) {
            let o = r.ragdoll.as_ref().unwrap().transform.origin;
            out[0] = o.x as f32;
            out[1] = o.y as f32;
            out[2] = o.z as f32;
        }
        Ok(())
    }

    /// `setRagdollLocalTransformRotation(id, qx, qy, qz, qw)`.
    pub fn set_ragdoll_local_transform_rotation(
        &mut self,
        id: i32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
    ) {
        if let Some(r) = self
            .ragdoll_world()
            .and_then(|ws| ws.ragdoll.ragdolls.get_mut(&id))
        {
            r.set_local_transform_rotation(qx, qy, qz, qw);
        }
    }

    /// `updateRagdoll(id, x, y, z, qx, qy, qz, qw)`: the ragdoll's world
    /// transform; x and z are made relative to the world offset in float.
    #[allow(clippy::too_many_arguments)]
    pub fn update_ragdoll(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
    ) {
        let Some(ws) = self.ragdoll_world() else {
            return;
        };
        let (ox, oy) = (ws.offset_x as f32, ws.offset_y as f32);
        if let Some(r) = ws.ragdoll.ragdolls.get_mut(&id) {
            r.update(x - ox, y, z - oy, qx, qy, qz, qw);
        }
    }

    /// `updateRagdollSkeletonTransforms(id, numberOfBones, bones)`: poses the
    /// skeleton and puts the bodies there, at rest.
    pub fn update_ragdoll_skeleton_transforms(&mut self, id: i32, n: i32, bones: &[f32]) {
        let Some(ws) = self.ragdoll_world() else {
            return;
        };
        if let Some(r) = ws.ragdoll.ragdolls.get_mut(&id) {
            r.update_skeleton_bone_transforms(&mut ws.dw, n, bones);
        }
    }

    /// `updateRagdollSkeletonPreviousTransforms(id, numberOfBones, dt,
    /// bones)`: Java passes bone velocities (position and quaternion
    /// differences over `dt`); they are read as transforms and the bodies
    /// get the velocities between those and the current pose.
    pub fn update_ragdoll_skeleton_previous_transforms(
        &mut self,
        id: i32,
        n: i32,
        dt: f32,
        bones: &[f32],
    ) {
        let Some(ws) = self.ragdoll_world() else {
            return;
        };
        if let Some(r) = ws.ragdoll.ragdolls.get_mut(&id) {
            r.update_previous_bone_transforms(&mut ws.dw, n, dt, bones);
        }
    }

    /// `getRagdollSimulationState(id)`: the activation state of all bodies
    /// when they agree, else 0; -1 for an unknown id.
    pub fn get_ragdoll_simulation_state(&mut self, id: i32) -> i32 {
        let Some(ws) = self.ragdoll_world() else {
            return -1;
        };
        match ws.ragdoll.ragdolls.get(&id) {
            Some(r) => r.get_simulation_state(&ws.dw),
            None => -1,
        }
    }

    /// `resetSkeletonPose(id)`: the bind pose, bodies at rest.
    pub fn reset_skeleton_pose(&mut self, id: i32) {
        let Some(ws) = self.ragdoll_world() else {
            return;
        };
        if let Some(r) = ws.ragdoll.ragdolls.get_mut(&id) {
            r.reset_skeleton_pose(&mut ws.dw);
        }
    }

    /// `setRagdollActive(id, active)`: adds the bodies and joints to the
    /// world (velocities zeroed, joints re-enabled) or removes them.
    pub fn set_ragdoll_active(&mut self, id: i32, active: bool) {
        let Some(ws) = self.ragdoll_world() else {
            return;
        };
        if let Some(r) = ws.ragdoll.ragdolls.get_mut(&id) {
            r.set_active(&mut ws.dw, active);
        }
    }

    /// `drawDebugSingleBone(id, enable)`: a debug-renderer flag only.
    pub fn draw_debug_single_bone(&mut self, id: i32, enable: bool) {
        if let Some(r) = self
            .ragdoll_world()
            .and_then(|ws| ws.ragdoll.ragdolls.get_mut(&id))
        {
            r.draw_single_bone = enable;
        }
    }

    /// `drawDebugRagdollSkeleton(id, enable, singlePart)`: debug flags only.
    pub fn draw_debug_ragdoll_skeleton(&mut self, id: i32, enable: bool, single: bool) {
        if let Some(r) = self
            .ragdoll_world()
            .and_then(|ws| ws.ragdoll.ragdolls.get_mut(&id))
        {
            r.draw_skeleton = enable;
            r.draw_skeleton_single = single;
        }
    }

    /// `drawDebugRagdollBodyParts(id, enable, singlePart)`: debug flags only.
    pub fn draw_debug_ragdoll_body_parts(&mut self, id: i32, enable: bool, single: bool) {
        if let Some(r) = self
            .ragdoll_world()
            .and_then(|ws| ws.ragdoll.ragdolls.get_mut(&id))
        {
            r.draw_body_parts = enable;
            r.draw_body_parts_single = single;
        }
    }

    /// `highlightRagdollBodyPart(id, part)`: debug state only.
    pub fn highlight_ragdoll_body_part(&mut self, id: i32, part: i32) {
        if let Some(r) = self
            .ragdoll_world()
            .and_then(|ws| ws.ragdoll.ragdolls.get_mut(&id))
        {
            r.highlighted_body_part = part;
        }
    }

    fn force_args(p: &[f32]) -> (Vec3, Vec3) {
        let v = |i: usize| p[i] as f64;
        (Vec3::new(v(0), v(1), v(2)), Vec3::new(v(3), v(4), v(5)))
    }

    /// `applyForce(id, part, [fx, fy, fz, relX, relY, relZ])`.
    pub fn apply_force(&mut self, id: i32, part: i32, p: &[f32]) {
        let Some(ws) = self.ragdoll_world() else {
            return;
        };
        if let Some(r) = ws.ragdoll.ragdolls.get(&id) {
            let (f, rel) = Self::force_args(p);
            r.ragdoll
                .as_ref()
                .unwrap()
                .apply_force(&mut ws.dw, part as usize, &f, &rel);
        }
    }

    /// `applyImpulse(id, part, [ix, iy, iz, relX, relY, relZ])`.
    pub fn apply_impulse(&mut self, id: i32, part: i32, p: &[f32]) {
        let Some(ws) = self.ragdoll_world() else {
            return;
        };
        if let Some(r) = ws.ragdoll.ragdolls.get(&id) {
            let (i, rel) = Self::force_args(p);
            r.ragdoll
                .as_ref()
                .unwrap()
                .apply_impulse(&mut ws.dw, part as usize, &i, &rel);
        }
    }

    /// `detachConstraint(id, joint)`: disables a joint.
    pub fn detach_constraint(&mut self, id: i32, joint: i32) {
        let Some(ws) = self.ragdoll_world() else {
            return;
        };
        if let Some(r) = ws.ragdoll.ragdolls.get(&id) {
            r.ragdoll
                .as_ref()
                .unwrap()
                .detach_constraint(&mut ws.dw, joint as usize);
        }
    }

    /// `setRagdollMass(mass)`: the template mass (for new body parts) and
    /// the mass of every body of every ragdoll, pooled ones too.
    pub fn set_ragdoll_mass(&mut self, mass: f32) {
        self.ragdoll_builder.mass = mass as f64;
        let Some(ws) = self.world.as_deref_mut() else {
            return;
        };
        let dw = &mut ws.dw;
        ws.ragdoll.for_each(|r, _| r.set_mass(dw, mass));
    }

    /// `checkWheelCollision(vehicleId, ragdollId, bodyPart)`: whether a
    /// wheel of the vehicle is within 0.5 of the body part's centre.
    pub fn check_wheel_collision_ragdoll(
        &mut self,
        vehicle: i32,
        ragdoll: i32,
        part: i32,
    ) -> Result<bool, JniError> {
        let Some(ws) = self.world.as_deref() else {
            return Err(JniError::NullPointer(NULL_WORLD));
        };
        let Some(v) = ws.vehicles.get(&vehicle) else {
            return Ok(false);
        };
        let Some(r) = ws.ragdoll.ragdolls.get(&ragdoll) else {
            return Ok(false);
        };
        let body = r.ragdoll.as_ref().unwrap().body(part as usize);
        for w in v.rv(ws).wheel_info.iter() {
            let wp = w.world_transform.origin;
            let bp = ws.dw.cw.obj(body).world_transform.origin;
            let d = (bp - wp).length();
            if d <= 0.5 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The live-update loops of the `defineRagdoll*` natives.
    fn update_all_ragdolls(&mut self, what: u8) {
        let builder = &self.ragdoll_builder;
        let Some(ws) = self.world.as_deref_mut() else {
            return;
        };
        let dw = &mut ws.dw;
        ws.ragdoll.for_each(|r, shapes| {
            let mut ctx = RagdollCtx {
                builder,
                dw,
                shapes,
            };
            match what {
                0 => r.update_constraints(&mut ctx),
                1 => r.update_anchors(&mut ctx),
                _ => r.update_body_parts(&mut ctx),
            }
        });
    }

    /// `updateRagdollBodyParts(liveUpdate)`: new shapes and bodies for every
    /// ragdoll. (The original also updates the ballistics targets.)
    fn update_ragdoll_body_parts(&mut self, live: bool) {
        if !live {
            return;
        }
        self.ragdoll_builder.update_body_part_shapes();
        self.update_all_ragdolls(2);
    }

    /// `defineRagdollConstraints(params, liveUpdate)`: 22 floats per joint
    /// (`RagdollScript.uploadConstraints`); with `liveUpdate` every ragdoll
    /// rebuilds its joints (which then are not in the world).
    pub fn define_ragdoll_constraints(
        &mut self,
        p: Option<&[f32]>,
        live: bool,
    ) -> Result<bool, JniError> {
        let p = p.ok_or(JniError::NullPointer("argument is null"))?;
        self.ragdoll_builder.script.constraints_from_java(p);
        if live {
            self.update_all_ragdolls(0);
        }
        Ok(true)
    }

    /// `defineRagdollAnchors(params, liveUpdate)`: 5 floats per bone.
    pub fn define_ragdoll_anchors(
        &mut self,
        p: Option<&[f32]>,
        live: bool,
    ) -> Result<bool, JniError> {
        let p = p.ok_or(JniError::NullPointer("argument is null"))?;
        self.ragdoll_builder.script.anchors_from_java(p);
        if live {
            self.update_all_ragdolls(1);
        }
        Ok(true)
    }

    /// `defineRagdollBodyPartInfo(params, liveUpdate)`: 10 floats per body
    /// part.
    pub fn define_ragdoll_body_part_info(
        &mut self,
        p: Option<&[f32]>,
        live: bool,
    ) -> Result<bool, JniError> {
        let p = p.ok_or(JniError::NullPointer("argument is null"))?;
        self.ragdoll_builder.script.body_part_info_from_java(p);
        self.update_ragdoll_body_parts(live);
        Ok(true)
    }

    /// `defineRagdollBodyDynamics(params, liveUpdate)`: 8 floats per body
    /// part.
    pub fn define_ragdoll_body_dynamics(
        &mut self,
        p: Option<&[f32]>,
        live: bool,
    ) -> Result<bool, JniError> {
        let p = p.ok_or(JniError::NullPointer("argument is null"))?;
        self.ragdoll_builder.script.body_dynamics_from_java(p);
        self.update_ragdoll_body_parts(live);
        Ok(true)
    }

    /// `setRagdollBodyDynamics(id, params)`: one record of 8 floats (part,
    /// linear and angular damping, deactivation time, linear and angular
    /// sleeping thresholds, friction, rolling friction) for every part of
    /// the first ragdoll with this id (active ones, then the pool).
    pub fn set_ragdoll_body_dynamics(
        &mut self,
        id: i32,
        p: Option<&[f32]>,
    ) -> Result<bool, JniError> {
        let p = p.ok_or(JniError::NullPointer("argument is null"))?;
        let mut d = RagdollBodyDynamics::default();
        RagdollScript::set_ragdoll_body_dynamics(&mut d, p);
        let builder = &self.ragdoll_builder;
        if let Some(ws) = self.world.as_deref_mut() {
            if let Some(r) = ws.ragdoll.find_any(id) {
                r.ragdoll
                    .as_ref()
                    .unwrap()
                    .update_body_dynamics(builder, &mut ws.dw, &d);
            }
        }
        Ok(true)
    }

    /// `resetRagdollBodyDynamics(id)`: back to the script's dynamics;
    /// whether a ragdoll with this id was found.
    pub fn reset_ragdoll_body_dynamics(&mut self, id: i32) -> bool {
        let builder = &self.ragdoll_builder;
        let Some(ws) = self.world.as_deref_mut() else {
            return false;
        };
        match ws.ragdoll.find_any(id) {
            Some(r) => {
                r.ragdoll
                    .as_ref()
                    .unwrap()
                    .reset_body_dynamics(builder, &mut ws.dw);
                true
            }
            None => false,
        }
    }
}
