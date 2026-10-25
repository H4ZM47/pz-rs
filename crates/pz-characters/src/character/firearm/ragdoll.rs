//! Ragdolls: `RagdollBuilder` (the shared skeleton and T-pose sent to the
//! physics once), `RagdollController` (one per ragdolling character: adds
//! the ragdoll to the physics world, feeds it the animated pose, simulates
//! it, writes the simulated pose back to the ragdoll animation clip, tracks
//! whether it still moves, and applies the shot impulse) and
//! `RagdollStateData`.
//!
//! The animation side (the bone transforms, the ragdoll clip's keyframes,
//! the bone world positions) is reached through [`RagdollAnim`]; the
//! animation player decides when a ragdoll track is active and calls
//! [`GameCharacter::ragdoll_update`] / [`GameCharacter::ragdoll_post_update`]
//! (`AnimationPlayer.updateRagdoll` / `postUpdateRagdoll`). Rendering and
//! the debug drawing are not here.

use pz_scripts::ScriptManager;

use super::ballistics::pack_bone_transforms;
use super::physics::*;
use crate::character::state::StateWorld;
use crate::character::{GameCharacter, SimContext};
use crate::Characters;

/// The bones `RagdollBuilder` resolves on the T-pose skeleton.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RagdollBone {
    /// `Bip01_Pelvis`.
    Pelvis,
    /// `Bip01_Head`.
    Head,
    /// `Bip01_L_UpperArm`.
    LeftShoulder,
    /// `Bip01_R_UpperArm`.
    RightShoulder,
}

/// The animation player as the ragdoll controller sees it.
pub trait RagdollAnim {
    /// `AnimationPlayer.isReady()`.
    fn is_ready(&self) -> bool;
    /// The animator's local transform of `SkeletonBone.all()[bone]`
    /// (position, rotation `(x, y, z, w)`); `None` for a bone the skeleton
    /// lacks.
    fn bone_transform(&self, bone: usize) -> Option<([f32; 3], [f32; 4])>;
    /// The bone's previous transform (`getPreviousTransform`).
    fn previous_bone_transform(&self, bone: usize) -> Option<([f32; 3], [f32; 4])>;
    /// `getBoneTransformsTimeDelta()`.
    fn bone_transforms_time_delta(&self) -> f32;
    /// `getRagdollSimulationAnimationClip() != null` and a skeleton.
    fn has_ragdoll_clip(&self) -> bool;
    /// `ragdollClip.setRagdollSimulationActive(true)`.
    fn set_ragdoll_simulation_active(&mut self);
    /// `setBoneKeyframePRS(clip, bone, pos, rot, scale 1)` for
    /// `SkeletonBone.all()[bone]` (the previous keyframe takes the current
    /// one). Not called for a bone the skeleton lacks.
    fn set_ragdoll_bone(&mut self, bone: usize, pos: [f32; 3], rot: [f32; 4]);
    /// `getRenderedAngle()`.
    fn rendered_angle(&self) -> f32;
    /// `Model.boneToWorldCoords(chr, bone)`.
    fn bone_world_position(&self, bone: RagdollBone) -> [f32; 3];
    /// `Model.boneToWorldCoords(chr, 0)`.
    fn root_world_position(&self) -> [f32; 3];
    /// `Model.boneZDirectionToWorldCoords(chr, pelvis, out, length)`.
    fn bone_z_direction_world(&self, bone: RagdollBone, length: f32) -> [f32; 3];
    /// `isFullyRagdolling()`.
    fn is_fully_ragdolling(&self) -> bool;
}

/// The skeleton `RagdollBuilder.Initialize` reads off the T-pose asset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RagdollSkeletonData {
    /// The parent `SkeletonBone` index of each `SkeletonBone` (-1 for a
    /// missing bone or the root).
    pub hierarchy: Vec<i32>,
    /// The T-pose keyframe (frame 1) of each `SkeletonBone`, `None` for a
    /// missing bone.
    pub tpose: Vec<Option<([f32; 3], [f32; 4])>>,
    /// The first `RagdollBodyDynamics` of the ragdoll scripts' defaults
    /// (linear damping, angular damping, deactivation time, linear and
    /// angular sleeping thresholds, rolling friction).
    pub vehicle_dynamics: Option<[f32; 6]>,
}

/// `RagdollBuilder.instance`'s state.
#[derive(Clone, Debug, PartialEq)]
pub struct RagdollBuilder {
    pub initialized: bool,
    pub mass: f32,
    /// `RagdollController.vehicleRagdollBodyDynamicsParams`.
    pub vehicle_body_dynamics_params: [f32; 8],
}

impl Default for RagdollBuilder {
    fn default() -> Self {
        RagdollBuilder { initialized: false, mass: 70.0, vehicle_body_dynamics_params: [0.0; 8] }
    }
}

/// `RagdollController.vehicleCollisionFriction`.
pub const VEHICLE_COLLISION_FRICTION: f32 = 0.4;

/// `RagdollController.getRagdollLocalRotation()`: `(0, 1, 0, -pi)` times
/// `(0, 0, 1, -pi/2)` (LWJGL `Quaternion.mul`), as bits.
pub fn ragdoll_local_rotation() -> [f32; 4] {
    [f32::from_bits(0x3f35_04f3), f32::from_bits(0xbf35_04f3), f32::from_bits(0x3304_c063), f32::from_bits(0xb304_c063)]
}

/// `PZMath.setFromAxisAngle(0, 1, 0, angle)`.
fn yaw_quaternion(angle: f32) -> [f32; 4] {
    let s = ((0.5 * angle as f64).sin() / 1.0) as f32;
    [0.0 * s, s, 0.0 * s, (0.5 * angle as f64).cos() as f32]
}

impl RagdollBuilder {
    /// `Initialize()` with the T-pose skeleton: sends the skeleton, the
    /// target shape scale and the T-pose to the physics.
    pub fn initialize(&mut self, skeleton: &RagdollSkeletonData, server: bool, physics: Option<&mut dyn PhysicsWorld>) -> bool {
        self.initialized = true;
        let n = SKELETON_BONE_COUNT as i32;
        let mut hierarchy = vec![0i32; SKELETON_BONE_COUNT * 7];
        for (i, p) in skeleton.hierarchy.iter().enumerate().take(SKELETON_BONE_COUNT) {
            hierarchy[i] = *p;
        }
        if let (false, Some(p)) = (server, physics) {
            p.initialize_ragdoll_skeleton(n, &hierarchy);
            p.set_ballistics_target_adjusting_shape_scale(0.35, 0.4, 0.35);
            let mut data = vec![0.0f32; SKELETON_BONE_COUNT * 7];
            for (i, b) in skeleton.tpose.iter().enumerate().take(SKELETON_BONE_COUNT) {
                let o = i * 7;
                match b {
                    None => data[o..o + 7].copy_from_slice(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
                    Some((pos, rot)) => {
                        data[o] = -pos[0] * 1.5;
                        data[o + 1] = -pos[1] * 1.5;
                        data[o + 2] = -pos[2] * 1.5;
                        data[o + 3..o + 7].copy_from_slice(rot);
                    }
                }
            }
            p.initialize_ragdoll_pose(n, &data, ragdoll_local_rotation());
        }
        if let Some(d) = skeleton.vehicle_dynamics {
            self.vehicle_body_dynamics_params =
                [RagdollBodyPart::Count.ordinal() as f32, d[0], d[1], d[2], d[3], d[4], VEHICLE_COLLISION_FRICTION, d[5]];
        }
        true
    }

    /// `setMass(mass)`.
    pub fn set_mass(&mut self, mass: f32, physics: Option<&mut dyn PhysicsWorld>) {
        if self.mass != mass {
            if let Some(p) = physics {
                p.set_ragdoll_mass(mass);
            }
        }
        self.mass = mass;
    }
}

/// `RagdollController.SimulationState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimulationState {
    Unknown,
    ActiveTag,
    IslandSleeping,
    WantsDeactivation,
    DisableDeactivation,
    DisableSimulation,
}

/// `RagdollStateData`.
#[derive(Clone, Debug, PartialEq)]
pub struct RagdollStateData {
    pub simulation_timeout: f32,
    pub is_simulating: bool,
    pub is_simulation_movement: bool,
    pub is_calculated: bool,
    pub is_contacting_vehicle: bool,
    pub simulation_rendered_angle: f32,
    pub simulation_character_forward_angle: f32,
    pub simulation_direction: (f32, f32),
    pub pelvis_direction: [f32; 3],
}

impl Default for RagdollStateData {
    fn default() -> Self {
        RagdollStateData {
            simulation_timeout: 1.5,
            is_simulating: false,
            is_simulation_movement: false,
            is_calculated: false,
            is_contacting_vehicle: false,
            simulation_rendered_angle: 0.0,
            simulation_character_forward_angle: 0.0,
            simulation_direction: (0.0, 0.0),
            pelvis_direction: [0.0; 3],
        }
    }
}

/// `RagdollController`.
#[derive(Clone, Debug, PartialEq)]
pub struct RagdollController {
    pub is_initialized: bool,
    pub added_to_world_frame_no: i32,
    pub is_upright: bool,
    pub is_on_back: bool,
    pub head_position: [f32; 3],
    pub pelvis_position: [f32; 3],
    pub left_shoulder_position: [f32; 3],
    pub right_shoulder_position: [f32; 3],
    pub desired_character_position: [f32; 3],
    pub previous_head_position: [f32; 3],
    pub previous_pelvis_position: [f32; 3],
    pub added_to_world: bool,
    pub simulation_state: i32,
    pub was_contacting_vehicle: bool,
    pub state: RagdollStateData,
    /// `ragdollWorldPosition` (iso), its physics form and the rotation.
    pub world_position: [f32; 3],
    pub world_position_bullet: [f32; 3],
    pub world_rotation_bullet: [f32; 4],
}

impl Default for RagdollController {
    fn default() -> Self {
        RagdollController {
            is_initialized: false,
            added_to_world_frame_no: -1,
            is_upright: true,
            is_on_back: false,
            head_position: [0.0; 3],
            pelvis_position: [0.0; 3],
            left_shoulder_position: [0.0; 3],
            right_shoulder_position: [0.0; 3],
            desired_character_position: [0.0; 3],
            previous_head_position: [0.0; 3],
            previous_pelvis_position: [0.0; 3],
            added_to_world: false,
            simulation_state: -1,
            was_contacting_vehicle: false,
            state: RagdollStateData::default(),
            world_position: [0.0; 3],
            world_position_bullet: [0.0; 3],
            world_rotation_bullet: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// What the controller reads of its character each call.
#[derive(Clone, Copy, Debug)]
pub struct RagdollOwner {
    pub id: i32,
    pub position: [f32; 3],
    /// `getAnimAngleRadians()`.
    pub anim_angle: f32,
    pub fall_on_front: bool,
}

/// The shot impulse `simulateHitReaction` applies (from the target's
/// ballistics combat data).
#[derive(Clone, Debug, PartialEq)]
pub struct HitImpulse {
    pub body_part: RagdollBodyPart,
    pub target_position: [f32; 3],
    pub attacker_position: [f32; 3],
    pub ammo_type: Option<std::rc::Rc<pz_scripts::registry::RegistryKey>>,
    pub explosive: bool,
    pub physics_object: Option<String>,
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    super::ballistics::v3_distance(a, b)
}

/// `PZMath.roundFloat(value, scale)`.
fn round_float(value: f32, scale: i32) -> f32 {
    let mut pow = 10i32;
    for _ in 1..scale {
        pow *= 10;
    }
    let tmp = value * pow as f32;
    let sub = tmp - (tmp as i32) as f32;
    let r = if value >= 0.0 {
        if sub >= 0.5 {
            tmp + 1.0
        } else {
            tmp
        }
    } else if sub >= -0.5 {
        tmp
    } else {
        tmp - 1.0
    };
    (r as i32) as f32 / pow as f32
}

impl RagdollController {
    /// `isFirstFrame()` with `WorldSimulation.getBulletFrameNo()`.
    pub fn is_first_frame(&self, frame_no: i32) -> bool {
        self.added_to_world_frame_no == -1 || self.added_to_world_frame_no >= frame_no - 1
    }

    /// `isSimulationSleeping()`.
    pub fn is_simulation_sleeping(&self) -> bool {
        self.simulation_state == SimulationState::IslandSleeping as i32
    }

    /// `isSimulationActive()`.
    pub fn is_simulation_active(&self) -> bool {
        self.state.is_simulating
    }

    fn calculate_world_transform(&mut self, o: &RagdollOwner) {
        self.world_position = o.position;
        self.world_position_bullet = pz_space_to_bullet_space(o.position);
        self.world_rotation_bullet = yaw_quaternion(-o.anim_angle);
    }

    fn update_world_transform(&mut self, o: &RagdollOwner, p: &mut dyn PhysicsWorld) {
        self.calculate_world_transform(o);
        p.update_ragdoll(o.id, self.world_position_bullet, self.world_rotation_bullet);
    }

    fn set_local_rotation(&self, id: i32, p: &mut dyn PhysicsWorld) {
        p.set_ragdoll_local_transform_rotation(id, ragdoll_local_rotation());
    }

    /// `setActive(active)`.
    fn set_active(&mut self, o: &RagdollOwner, active: bool, p: &mut dyn PhysicsWorld) {
        self.update_world_transform(o, p);
        p.set_ragdoll_active(o.id, active);
    }

    /// `addToWorld()`; returns whether it was added.
    fn add_to_world(&mut self, o: &RagdollOwner, active: &mut i32, p: &mut dyn PhysicsWorld) {
        if self.added_to_world {
            return;
        }
        *active += 1;
        self.calculate_world_transform(o);
        p.add_ragdoll(o.id, self.world_position_bullet, self.world_rotation_bullet);
        self.added_to_world = true;
        self.added_to_world_frame_no = p.frame_no();
    }

    /// `removeFromWorld()`.
    fn remove_from_world(&mut self, id: i32, active: &mut i32, p: Option<&mut dyn PhysicsWorld>) {
        if !self.added_to_world {
            return;
        }
        *active -= 1;
        if let Some(p) = p {
            p.remove_ragdoll(id);
        }
        self.added_to_world = false;
    }

    fn bone_buffer(anim: &dyn RagdollAnim) -> Vec<f32> {
        let bones: Vec<_> = (0..SKELETON_BONE_COUNT).map(|i| anim.bone_transform(i)).collect();
        let mut buf = vec![0.0f32; SKELETON_BONE_COUNT * 7];
        pack_bone_transforms(&bones, &mut buf);
        buf
    }

    /// `getBoneTransformVelocitiesFromAnimation(floats, deltaT)`.
    fn velocity_buffer(anim: &dyn RagdollAnim, dt: f32) -> Vec<f32> {
        let mut buf = vec![0.0f32; SKELETON_BONE_COUNT * 7];
        for i in 0..SKELETON_BONE_COUNT {
            let (vp, vr) = match (anim.bone_transform(i), anim.previous_bone_transform(i)) {
                (Some((cp, cr)), Some((pp, pr))) if dt > 0.0 => {
                    let m = 1.0 / dt;
                    (
                        [(cp[0] - pp[0]) * m, (cp[1] - pp[1]) * m, (cp[2] - pp[2]) * m],
                        [(cr[0] - pr[0]) * m, (cr[1] - pr[1]) * m, (cr[2] - pr[2]) * m, (cr[3] - pr[3]) * m],
                    )
                }
                _ => ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
            };
            let o = i * 7;
            buf[o] = -vp[0] * 1.5;
            buf[o + 1] = -vp[1] * 1.5;
            buf[o + 2] = -vp[2] * 1.5;
            buf[o + 3..o + 7].copy_from_slice(&vr);
        }
        buf
    }

    /// `updateRagdollSkeleton()`.
    pub fn update_skeleton(&mut self, o: &RagdollOwner, ctx: &dyn SimContext, anim: &dyn RagdollAnim, p: &mut dyn PhysicsWorld) {
        self.set_local_rotation(o.id, p);
        self.update_world_transform(o, p);
        let buf = Self::bone_buffer(anim);
        p.update_ragdoll_skeleton_transforms(o.id, SKELETON_BONE_COUNT as i32, &buf);
        if ctx.debug_ragdoll_initial_velocities() {
            let dt = anim.bone_transforms_time_delta().max(0.0);
            let vel = Self::velocity_buffer(anim, dt);
            p.update_ragdoll_skeleton_previous_transforms(o.id, SKELETON_BONE_COUNT as i32, dt, &vel);
        }
    }

    /// `initialize()`.
    fn initialize(&mut self, o: &RagdollOwner, ctx: &dyn SimContext, active: &mut i32, anim: Option<&dyn RagdollAnim>, p: &mut dyn PhysicsWorld) -> bool {
        if self.is_initialized {
            return true;
        }
        let Some(anim) = anim.filter(|a| a.is_ready()) else { return false };
        self.state = RagdollStateData { is_simulating: true, ..RagdollStateData::default() };
        self.add_to_world(o, active, p);
        self.update_skeleton(o, ctx, anim, p);
        self.set_active(o, true, p);
        true
    }

    /// `simulateRagdoll(...)`: one physics read-back into the ragdoll clip.
    fn simulate(&mut self, o: &RagdollOwner, anim: &mut dyn RagdollAnim, p: &mut dyn PhysicsWorld) {
        self.update_world_transform(o, p);
        self.set_local_rotation(o.id, p);
        let mut skel = vec![0.0f32; SKELETON_BONE_COUNT * 7];
        let mut rb = vec![0.0f32; 77];
        let n = p.simulate_ragdoll_with_rigid_body_output(o.id, &mut skel, &mut rb);
        if anim.has_ragdoll_clip() && !self.is_first_frame(p.frame_no()) {
            anim.set_ragdoll_simulation_active();
            for i in 0..(n.max(0) as usize).min(SKELETON_BONE_COUNT) {
                let f = &skel[i * 7..i * 7 + 7];
                let pos = [f[0] / 1.5, f[1] / 1.5, f[2] / 1.5];
                let rot = [f[3], f[4], f[5], f[6]];
                if anim.bone_transform(i).is_none() {
                    continue;
                }
                anim.set_ragdoll_bone(i, [pos[0] * -1.0, pos[1] * -1.0, pos[2] * -1.0], rot);
            }
        }
        self.state.simulation_rendered_angle = anim.rendered_angle();
        self.state.simulation_character_forward_angle = o.anim_angle;
    }

    /// `simulateHitReaction()`: the target's unprocessed shot becomes an
    /// impulse on the hit body part. `hit` is `None` when there is no
    /// unprocessed combat data.
    fn simulate_hit_reaction(&mut self, sm: &ScriptManager, ctx: &dyn SimContext, id: i32, hit: Option<&HitImpulse>, p: &mut dyn PhysicsWorld) -> bool {
        let Some(hit) = hit else { return false };
        if hit.body_part == RagdollBodyPart::Count {
            return false;
        }
        if !ctx.debug_physics_hit_reaction() {
            return true;
        }
        let part = hit.body_part.ordinal() as usize;
        let (impulse, upward) = if hit.ammo_type.is_some() {
            (
                pz_scripts::objects::items::impulse_for_ammo(sm, part, hit.ammo_type.as_ref()),
                pz_scripts::objects::items::upward_impulse_for_ammo(sm, part, hit.ammo_type.as_ref()),
            )
        } else if hit.explosive {
            (
                pz_scripts::objects::items::impulse_for_object(sm, part, hit.physics_object.as_deref()),
                pz_scripts::objects::items::upward_impulse_for_object(sm, part, hit.physics_object.as_deref()),
            )
        } else {
            return true;
        };
        let d = super::ballistics::v3_normalize([
            hit.target_position[0] - hit.attacker_position[0],
            hit.target_position[1] - hit.attacker_position[1],
            hit.target_position[2] - hit.attacker_position[2],
        ]);
        let buf = [d[0] * impulse, upward, d[1] * impulse, 0.0, 0.0, 0.0];
        p.apply_impulse(id, hit.body_part.ordinal(), &buf);
        true
    }

    /// `update(deltaT, out position, out rotation)`; returns whether the
    /// combat data was consumed (the caller marks it processed and clears
    /// the hit reaction).
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        sm: &ScriptManager,
        ctx: &dyn SimContext,
        o: &RagdollOwner,
        active: &mut i32,
        anim: Option<&mut dyn RagdollAnim>,
        hit: Option<&HitImpulse>,
        p: &mut dyn PhysicsWorld,
    ) -> bool {
        if !self.is_initialized {
            self.added_to_world_frame_no = -1;
            self.is_initialized = self.initialize(o, ctx, active, anim.as_deref(), p);
            return false;
        }
        let Some(anim) = anim else { return false };
        self.simulate(o, anim, p);
        self.simulation_state = p.get_ragdoll_simulation_state(o.id);
        self.simulate_hit_reaction(sm, ctx, o.id, hit, p)
    }

    /// `calculateSimulationData(stateData, deltaT)`: the head and pelvis
    /// motion, uprightness, which side the body lies on and the direction
    /// it lies in. Returns the new `fallOnFront`, or `None` on the first
    /// frame.
    fn calculate_simulation_data(&mut self, o: &RagdollOwner, dt: f32, frame_no: i32, anim: &dyn RagdollAnim) -> Option<(bool, bool)> {
        if self.is_first_frame(frame_no) {
            return None;
        }
        self.previous_head_position = self.head_position;
        self.previous_pelvis_position = self.pelvis_position;
        self.head_position = anim.bone_world_position(RagdollBone::Head);
        self.pelvis_position = anim.bone_world_position(RagdollBone::Pelvis);
        if !self.state.is_calculated {
            self.previous_head_position = self.head_position;
            self.previous_pelvis_position = self.pelvis_position;
            self.desired_character_position = o.position;
        }
        let d = self.desired_character_position;
        self.desired_character_position = [
            d[0] + (self.pelvis_position[0] - self.previous_pelvis_position[0]),
            d[1] + (self.pelvis_position[1] - self.previous_pelvis_position[1]),
            round_float(d[2] + (self.pelvis_position[2] - self.previous_pelvis_position[2]), 1),
        ];
        self.state.is_simulation_movement = false;
        let head_distance = dist3(self.previous_head_position, self.head_position);
        let pelvis_distance = dist3(self.previous_pelvis_position, self.pelvis_position);
        if self.state.is_contacting_vehicle {
            self.state.simulation_timeout = 1.5;
            self.state.is_contacting_vehicle = false;
        } else if head_distance > 0.01 || pelvis_distance > 0.01 {
            if self.state.simulation_timeout < 1.5 {
                self.state.simulation_timeout = 1.5;
            }
            self.state.is_simulation_movement = true;
        } else if self.is_simulation_sleeping() {
            self.state.simulation_timeout -= dt * 10.0;
        }
        let ground = anim.root_world_position();
        self.is_upright = self.head_position[2] > ground[2] + 0.6 - 0.2;
        let on_floor = !self.is_upright;
        self.state.pelvis_direction = anim.bone_z_direction_world(RagdollBone::Pelvis, 0.5);
        self.left_shoulder_position = anim.bone_world_position(RagdollBone::LeftShoulder);
        self.right_shoulder_position = anim.bone_world_position(RagdollBone::RightShoulder);
        let (l, r) = (self.left_shoulder_position, self.right_shoulder_position);
        let center = [l[0] + (r[0] - l[0]) * 0.5, l[1] + (r[1] - l[1]) * 0.5, l[2] + (r[2] - l[2]) * 0.5];
        self.is_on_back = !o.fall_on_front;
        if !self.is_upright && anim.is_fully_ragdolling() {
            let right = [r[0] - center[0], r[1] - center[1], 0.0];
            let up = [center[0] - self.pelvis_position[0], center[1] - self.pelvis_position[1], 0.0];
            let fwd = [
                right[1] * up[2] - right[2] * up[1],
                -(right[0] * up[2] - right[2] * up[0]),
                right[0] * up[1] - right[1] * up[0],
            ];
            let fwd = super::ballistics::v3_normalize(fwd);
            if fwd[2].abs() > 0.2 {
                self.is_on_back = fwd[2] > 0.0;
            }
        }
        let p = self.pelvis_position;
        self.state.simulation_direction = if self.is_on_back { (p[0] - center[0], p[1] - center[1]) } else { (center[0] - p[0], center[1] - p[1]) };
        self.state.is_calculated = true;
        let (dx, dy) = self.state.simulation_direction;
        self.state.simulation_direction = crate::character::zombie::math::normalize(dx, dy);
        Some((on_floor, !self.is_on_back))
    }

    /// `updateSimulationTimeout(stateData)`.
    fn update_simulation_timeout(&mut self, o: &RagdollOwner, time_delta: f32, p: &mut dyn PhysicsWorld) {
        if self.state.simulation_timeout > 0.0 {
            self.state.simulation_timeout -= time_delta;
        }
        let end = !self.state.is_simulation_movement && self.state.simulation_timeout <= 0.0;
        if self.is_simulation_sleeping() && end {
            self.state.is_simulating = false;
            self.set_active(o, false, p);
        }
    }

    /// `vehicleCollision(zombie, vehicle)`: `contacting` is the vehicle's
    /// `isCollided(chr)` and `touching` its `testTouchingVehicle(zombie,
    /// this)` (none: no vehicle hit yet).
    pub fn vehicle_collision(&mut self, id: i32, contacting: Option<(bool, bool)>, vehicle_params: &[f32; 8], p: &mut dyn PhysicsWorld) {
        let mut touching = false;
        if let Some((c, t)) = contacting {
            self.state.is_contacting_vehicle = c;
            touching = t;
        }
        if self.was_contacting_vehicle && !touching {
            p.reset_ragdoll_body_dynamics(id);
        } else if !self.was_contacting_vehicle && touching {
            p.set_ragdoll_body_dynamics(id, vehicle_params);
        }
        self.was_contacting_vehicle = touching;
    }
}

impl<I: 'static> GameCharacter<I> {
    fn ragdoll_owner(&self, anim_angle: f32) -> RagdollOwner {
        RagdollOwner {
            id: self.id,
            position: [self.vars.x, self.vars.y, self.vars.z],
            anim_angle,
            fall_on_front: self.environment.fall_on_front,
        }
    }

    /// `isRagdollSimulationActive()`'s controller part.
    pub fn is_ragdoll_simulation_active(&self) -> bool {
        self.ragdoll.as_ref().is_some_and(|r| r.is_simulation_active())
    }

    /// `isUpright()`.
    pub fn is_ragdoll_upright(&self, chars: &Characters, ctx: &dyn SimContext) -> bool {
        self.can_ragdoll(chars, ctx) && self.ragdoll.as_ref().is_some_and(|r| r.is_upright)
    }

    /// `isOnBack()`.
    pub fn is_ragdoll_on_back(&self) -> bool {
        self.ragdoll.as_ref().is_some_and(|r| r.is_on_back)
    }

    /// `AnimationPlayer.updateRagdoll(deltaT)`'s controller part, for an
    /// animation player whose active ragdoll track exists and whose bone
    /// transforms are past their first frame: creates the controller
    /// (`getOrCreateRagdollController`) and updates it. `anim_angle` is
    /// `getAnimAngleRadians()`.
    pub fn ragdoll_update(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, world: &mut dyn StateWorld<I>, anim_angle: f32) {
        let mut ctrl = self.ragdoll.take().unwrap_or_default();
        let owner = self.ragdoll_owner(anim_angle);
        // The shot to react to: the ballistics target's unprocessed data.
        let hit = self.ballistics_target.as_ref().filter(|t| !t.combat_damage_data_processed).map(|t| {
            let d = &t.combat_damage_data;
            let attacker_position = world.character_mut(d.attacker).map_or(d.attacker_position, |a| [a.vars.x, a.vars.y, a.vars.z]);
            HitImpulse {
                body_part: d.body_part,
                target_position: owner.position,
                attacker_position,
                ammo_type: d.weapon_ammo_type.clone(),
                explosive: d.weapon_explosive,
                physics_object: d.weapon_physics_object.clone(),
            }
        });
        let mut active = chars.combat.borrow().active_ragdolls;
        let consumed = {
            let (physics, anim) = world.ragdoll_io(self.id);
            match physics {
                Some(p) => ctrl.update(sm, ctx, &owner, &mut active, anim, hit.as_ref(), p),
                None => false,
            }
        };
        chars.combat.borrow_mut().active_ragdolls = active;
        if consumed {
            if let Some(t) = self.ballistics_target.as_mut() {
                t.combat_damage_data_processed = true;
            }
            self.vars.hit_reaction = String::new();
        }
        self.vars.ragdoll_simulation_active = ctrl.is_simulation_active();
        self.ragdoll = Some(ctrl);
    }

    /// `AnimationPlayer.postUpdateRagdoll(deltaT)`'s controller part
    /// (`RagdollController.postUpdate`): sets `onFloor`, `fallOnFront` and
    /// `ragdollFall` from the simulated body.
    pub fn ragdoll_post_update(&mut self, ctx: &dyn SimContext, world: &mut dyn StateWorld<I>, dt: f32) {
        let Some(mut ctrl) = self.ragdoll.take() else { return };
        let owner = self.ragdoll_owner(self.vars.anim_angle);
        let (physics, anim) = world.ragdoll_io(self.id);
        if let (Some(p), Some(anim)) = (physics, anim) {
            let frame = p.frame_no();
            if let Some((on_floor, fall_on_front)) = ctrl.calculate_simulation_data(&owner, dt, frame, anim) {
                self.vars.on_floor = on_floor;
                self.environment.fall_on_front = fall_on_front;
                self.vars.ragdoll_fall = true;
            }
            ctrl.update_simulation_timeout(&owner, ctx.time_delta(), p);
        }
        self.vars.ragdoll_simulation_active = ctrl.is_simulation_active();
        self.ragdoll = Some(ctrl);
    }

    /// `releaseRagdollController()` (`RagdollController.reset` on release):
    /// out of the physics world, `ragdollFall` and `usePhysicHitReaction`
    /// cleared.
    pub fn release_ragdoll_controller(&mut self, chars: &Characters, world: &mut dyn StateWorld<I>) {
        let Some(mut ctrl) = self.ragdoll.take() else { return };
        let mut active = chars.combat.borrow().active_ragdolls;
        ctrl.remove_from_world(self.id, &mut active, world.physics());
        chars.combat.borrow_mut().active_ragdolls = active;
        self.vars.ragdoll_fall = false;
        self.vars.use_physic_hit_reaction = false;
        self.vars.ragdoll_simulation_active = false;
    }
}
