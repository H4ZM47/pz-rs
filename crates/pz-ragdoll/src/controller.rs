//! `RagdollController` (+ `RagdollStateData`): one character's ragdoll.
//! It adds the ragdoll to the physics world posed like the character's
//! animation, then every frame reads the simulated skeleton back
//! (positions divided by 1.5 and negated) into the `AnimationPlayer`'s
//! ragdoll clip, and tracks where the body lies (head, pelvis, on back or
//! front, the direction it points, whether it still moves).

use pz_anim::math::{helpers, pzmath, Quaternion, Vector2, Vector3f};
use pz_anim::{AnimationPlayer, RagdollHost};
use pz_bullet::PzBullet;

use crate::builder::RagdollBuilder;
use crate::skeleton_bone::{self as sb, COUNT};

/// `RagdollController.MovementThreshold`.
pub const MOVEMENT_THRESHOLD: f32 = 0.01;
/// `RagdollController.MovementThresholdTime`.
pub const MOVEMENT_THRESHOLD_TIME: f32 = 1.5;
const SIMULATION_TIMEOUT_DECAY_FACTOR: f32 = 10.0;

/// `RagdollController.SimulationState` (the native activation state).
pub mod simulation_state {
    pub const UNKNOWN: i32 = 0;
    pub const ACTIVE_TAG: i32 = 1;
    pub const ISLAND_SLEEPING: i32 = 2;
    pub const WANTS_DEACTIVATION: i32 = 3;
    pub const DISABLE_DEACTIVATION: i32 = 4;
    pub const DISABLE_SIMULATION: i32 = 5;
}

/// `zombie.iso.Vector3`, with the float operations the controller uses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    /// `distanceTo` (`Math.pow(d, 2.0)` of each float difference).
    pub fn distance_to(&self, o: &Vec3) -> f32 {
        let (dx, dy, dz) = (
            (o.x - self.x) as f64,
            (o.y - self.y) as f64,
            (o.z - self.z) as f64,
        );
        (dx * dx + dy * dy + dz * dz).sqrt() as f32
    }

    /// `normalize()`.
    pub fn normalize(&mut self) {
        let length = ((self.x * self.x + self.y * self.y + self.z * self.z) as f64).sqrt() as f32;
        if length == 0.0 {
            *self = Vec3::default();
        } else {
            self.x /= length;
            self.y /= length;
            self.z /= length;
        }
    }

    /// `Vector3.sub(a, b, out)`.
    pub fn sub(a: &Vec3, b: &Vec3) -> Vec3 {
        Vec3::new(a.x - b.x, a.y - b.y, a.z - b.z)
    }

    /// `PZMath.lerp(out, a, b, t)`.
    pub fn lerp(a: &Vec3, b: &Vec3, t: f32) -> Vec3 {
        Vec3::new(
            a.x + (b.x - a.x) * t,
            a.y + (b.y - a.y) * t,
            a.z + (b.z - a.z) * t,
        )
    }

    /// `PZMath.cross(a, b, out)`.
    pub fn cross(a: &Vec3, b: &Vec3) -> Vec3 {
        Vec3::new(
            a.y * b.z - a.z * b.y,
            -(a.x * b.z - a.z * b.x),
            a.x * b.y - a.y * b.x,
        )
    }
}

/// `Model.vectorToWorldCoords(x, y, z, angle, vec)`.
pub fn vector_to_world(pos: Vec3, angle: f32, v: Vec3) -> Vec3 {
    let x = -v.x;
    let (c, s) = ((angle as f64).cos(), (angle as f64).sin());
    let rx = (x as f64 * c - v.z as f64 * s) as f32;
    let rz = (x as f64 * s + v.z as f64 * c) as f32;
    Vec3::new(
        rx * 1.5 + pos.x,
        rz * 1.5 + pos.y,
        v.y * 0.612_372_34 + pos.z,
    )
}

/// `Model.boneToWorldCoords(character, boneIndex, vec)`.
pub fn bone_to_world(
    player: &AnimationPlayer,
    pos: Vec3,
    animal_size: Option<f32>,
    bone: usize,
) -> Vec3 {
    let m = player.model_transform(bone);
    let mut v = Vec3::new(m.m03, m.m13, m.m23);
    if let Some(s) = animal_size {
        v = Vec3::new(v.x * s, v.y * s, v.z * s);
    }
    vector_to_world(pos, player.rendered_angle(), v)
}

/// `Model.boneZDirectionToWorldCoords(character, boneIndex, vec, length)`.
pub fn bone_z_direction_to_world(
    player: &AnimationPlayer,
    pos: Vec3,
    bone: usize,
    length: f32,
) -> Vec3 {
    let m = player.model_transform(bone);
    let v = Vec3::new(
        m.m02 * length + m.m03,
        m.m12 * length + m.m13,
        m.m22 * length + m.m23,
    );
    vector_to_world(pos, player.rendered_angle(), v)
}

/// `RagdollController.getRagdollLocalRotation`: half a turn about y, then a
/// quarter turn about z.
pub fn ragdoll_local_rotation() -> Quaternion {
    let r = helpers::set_from_axis_angle(0.0, 1.0, 0.0, -std::f64::consts::PI as f32);
    let yaw = helpers::set_from_axis_angle(0.0, 0.0, 1.0, -1.570_796_4);
    quat_mul(r, yaw)
}

/// LWJGL `Quaternion.mul(left, right, dest)`.
pub fn quat_mul(l: Quaternion, r: Quaternion) -> Quaternion {
    Quaternion::new(
        l.x * r.w + l.w * r.x + l.y * r.z - l.z * r.y,
        l.y * r.w + l.w * r.y + l.z * r.x - l.x * r.z,
        l.z * r.w + l.w * r.z + l.x * r.y - l.y * r.x,
        l.w * r.w - l.x * r.x - l.y * r.y - l.z * r.z,
    )
}

/// `RagdollController.pzSpaceToBulletSpace`: (x, z * 2.44949, y).
pub fn pz_space_to_bullet_space(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[2] * 2.449_49, v[1]]
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
    pub simulation_direction: Vector2,
    pub pelvis_direction: Vec3,
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
            simulation_direction: Vector2::default(),
            pelvis_direction: Vec3::default(),
        }
    }
}

/// A combat hit the ragdoll reacts to (`simulateHitReaction`): what the
/// character's `BallisticsTarget.CombatDamageData` and
/// `PhysicsHitReactionScript` give.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RagdollHit {
    /// `RagdollBodyPart` ordinal.
    pub body_part: i32,
    pub impulse: f32,
    pub upward_impulse: f32,
    /// The target's and attacker's positions (for explosives, the attack's
    /// target square in place of the attacker).
    pub target: Vec3,
    pub attacker: Vec3,
}

/// What `RagdollController` reads and writes on its `IsoGameCharacter`.
pub trait RagdollCharacter {
    /// `getID()` (the ragdoll's id in the physics world).
    fn id(&self) -> i32;
    /// `getX()`, `getY()`, `getZ()`.
    fn position(&self) -> Vec3;
    /// `forwardDirection.getDirection()` (what `getAnimAngleRadians` uses
    /// before the animation player has a pose).
    fn forward_direction_angle(&self) -> f32;
    /// `canRagdoll()`, apart from the cap on active simulations (which the
    /// [`RagdollFrame`] checks).
    fn can_ragdoll(&self) -> bool;
    /// `onRagdollSimulationStarted()` (slides away from walls).
    fn on_ragdoll_simulation_started(&mut self) {}
    fn set_ragdoll_fall(&mut self, v: bool);
    fn set_use_physic_hit_reaction(&mut self, v: bool);
    fn is_fall_on_front(&self) -> bool;
    fn set_fall_on_front(&mut self, v: bool);
    fn set_on_floor(&mut self, v: bool);
    /// `IsoAnimal.getAnimalSize()` for animals.
    fn animal_size(&self) -> Option<f32> {
        None
    }
    /// `doDeferredMovementFromRagdoll(dMovement)`.
    fn do_deferred_movement_from_ragdoll(&mut self, player: &AnimationPlayer, m: Vec3);
    /// The ballistics target's unprocessed combat hit, marked processed
    /// (`getCombatDamageDataProcessed` / `setCombatDamageDataProcessed`),
    /// with the hit reaction cleared. `None` when there is none, or the
    /// weapon is neither a firearm nor an explosive.
    fn take_ragdoll_hit(&mut self) -> Option<RagdollHit> {
        None
    }
}

/// The original's statics: the number of ragdolls in the world and the
/// cap `canRagdoll` applies (`Core.getMaxActiveRagdolls`).
#[derive(Clone, Debug)]
pub struct RagdollGlobals {
    pub number_of_active_simulations: i32,
    pub max_active_ragdolls: i32,
}

impl Default for RagdollGlobals {
    fn default() -> Self {
        RagdollGlobals {
            number_of_active_simulations: 0,
            max_active_ragdolls: 20,
        }
    }
}

/// The debug options the controller reads (debug-only options: their
/// defaults hold in release builds).
#[derive(Clone, Copy, Debug)]
pub struct RagdollDebugOptions {
    pub enable_initial_velocities: bool,
    pub physics_hit_reaction: bool,
}

impl Default for RagdollDebugOptions {
    fn default() -> Self {
        RagdollDebugOptions {
            enable_initial_velocities: true,
            physics_hit_reaction: true,
        }
    }
}

/// `RagdollController`.
#[derive(Clone, Debug, Default)]
pub struct RagdollController {
    is_initialized: bool,
    added_to_world_frame_no: i32,
    is_upright: bool,
    is_on_back: bool,
    head_position: Vec3,
    pelvis_position: Vec3,
    left_shoulder_position: Vec3,
    right_shoulder_position: Vec3,
    desired_character_position: Vec3,
    previous_head_position: Vec3,
    previous_pelvis_position: Vec3,
    added_to_world: bool,
    simulation_state: i32,
    was_contacting_vehicle: bool,
    state: RagdollStateData,
    ragdoll_world_position: Vec3,
    ragdoll_world_position_pz_bullet: [f32; 3],
    ragdoll_world_rotation_pz_bullet: Quaternion,
}

/// Everything a controller call needs besides the player: the world, the
/// character, the clock.
pub struct RagdollEnv<'a> {
    pub bullet: &'a mut PzBullet,
    pub globals: &'a mut RagdollGlobals,
    pub builder: &'a RagdollBuilder,
    pub character: &'a mut dyn RagdollCharacter,
    /// `WorldSimulation.instance.getBulletFrameNo()`.
    pub bullet_frame_no: i32,
    /// `GameTime.getTimeDelta()`.
    pub time_delta: f32,
    pub debug: RagdollDebugOptions,
}

impl RagdollController {
    /// `RagdollController.alloc()` (a fresh pool object).
    pub fn new() -> RagdollController {
        RagdollController {
            added_to_world_frame_no: -1,
            is_upright: true,
            simulation_state: -1,
            ..Default::default()
        }
    }

    pub fn state(&self) -> &RagdollStateData {
        &self.state
    }

    pub fn is_initialized(&self) -> bool {
        self.is_initialized
    }

    /// `isFirstFrame()`.
    pub fn is_first_frame(&self, bullet_frame_no: i32) -> bool {
        self.added_to_world_frame_no == -1 || self.added_to_world_frame_no >= bullet_frame_no - 1
    }

    pub fn is_upright(&self) -> bool {
        self.is_upright
    }

    pub fn is_on_back(&self) -> bool {
        self.is_on_back
    }

    pub fn head_position(&self) -> Vec3 {
        self.head_position
    }

    pub fn pelvis_position(&self) -> Vec3 {
        self.pelvis_position
    }

    pub fn desired_character_position(&self) -> Vec3 {
        self.desired_character_position
    }

    /// `isSimulationSleeping()`.
    pub fn is_simulation_sleeping(&self) -> bool {
        self.simulation_state == simulation_state::ISLAND_SLEEPING
    }

    /// `isSimulationActive()`.
    pub fn is_simulation_active(&self) -> bool {
        self.state.is_simulating
    }

    pub fn simulation_state(&self) -> i32 {
        self.simulation_state
    }

    /// `getCalculatedSimulationDirectionAngle()`.
    pub fn calculated_simulation_direction_angle(&self) -> f32 {
        Vector2::direction(
            self.state.simulation_direction.x,
            self.state.simulation_direction.y,
        )
    }

    /// `getAnimAngleRadians()` of the character.
    fn anim_angle_radians(player: &AnimationPlayer, env: &RagdollEnv) -> f32 {
        if player.bone_transforms_need_first_frame() {
            return env.character.forward_direction_angle();
        }
        player.angle()
    }

    /// `initialize()`.
    fn initialize(&mut self, player: &mut AnimationPlayer, env: &mut RagdollEnv) -> bool {
        if self.is_initialized {
            return true;
        }
        self.state = RagdollStateData::default();
        self.state.is_simulating = true;
        self.add_to_world(player, env);
        self.update_ragdoll_skeleton(player, env);
        self.set_active(player, env, true);
        true
    }

    /// `reset()` (`onReleased`).
    pub fn reset(&mut self, env: &mut RagdollEnv) {
        self.remove_from_world(env);
        self.state = RagdollStateData::default();
        self.is_upright = true;
        self.is_on_back = false;
        self.simulation_state = -1;
        env.character.set_ragdoll_fall(false);
        self.is_initialized = false;
        self.added_to_world_frame_no = -1;
        env.character.set_use_physic_hit_reaction(false);
    }

    /// `reinitialize()`.
    pub fn reinitialize(&mut self, player: &mut AnimationPlayer, env: &mut RagdollEnv) {
        self.reset(env);
        self.added_to_world_frame_no = -1;
        self.is_initialized = self.initialize(player, env);
    }

    /// `setActive(active)`.
    pub fn set_active(&mut self, player: &AnimationPlayer, env: &mut RagdollEnv, active: bool) {
        self.update_ragdoll_world_transform(player, env);
        env.bullet.set_ragdoll_active(env.character.id(), active);
    }

    /// `addToWorld()`.
    fn add_to_world(&mut self, player: &AnimationPlayer, env: &mut RagdollEnv) {
        if self.added_to_world {
            return;
        }
        env.globals.number_of_active_simulations += 1;
        self.calculate_ragdoll_world_transform(player, env);
        let p = self.ragdoll_world_position_pz_bullet;
        let q = self.ragdoll_world_rotation_pz_bullet;
        // Fails only without a world, which the original would not survive.
        let _ = env
            .bullet
            .add_ragdoll(env.character.id(), p[0], p[1], p[2], q.x, q.y, q.z, q.w);
        self.added_to_world = true;
        self.added_to_world_frame_no = env.bullet_frame_no;
    }

    /// `removeFromWorld()`.
    fn remove_from_world(&mut self, env: &mut RagdollEnv) {
        if !self.added_to_world {
            return;
        }
        env.globals.number_of_active_simulations -= 1;
        let _ = env.bullet.remove_ragdoll(env.character.id());
        self.added_to_world = false;
    }

    /// `updateRagdollSkeleton()`: uploads the animated pose (and its
    /// velocities).
    pub fn update_ragdoll_skeleton(&mut self, player: &AnimationPlayer, env: &mut RagdollEnv) {
        let id = env.character.id();
        self.set_ragdoll_local_rotation(env);
        self.update_ragdoll_world_transform(player, env);
        let mut buf = [0f32; COUNT * 7];
        bone_transforms_from_animation(player, &mut buf);
        env.bullet
            .update_ragdoll_skeleton_transforms(id, COUNT as i32, &buf);
        if env.debug.enable_initial_velocities {
            let dt = pzmath::max(player.bone_transforms_time_delta(), 0.0);
            bone_transform_velocities_from_animation(player, &mut buf, dt);
            env.bullet
                .update_ragdoll_skeleton_previous_transforms(id, COUNT as i32, dt, &buf);
        }
    }

    /// `update(deltaT, ragdollWorldPosition, ragdollWorldRotation)`.
    pub fn update(&mut self, player: &mut AnimationPlayer, env: &mut RagdollEnv, _dt: f32) {
        if !self.is_initialized {
            self.added_to_world_frame_no = -1;
            self.is_initialized = self.initialize(player, env);
            return;
        }
        self.simulate_ragdoll(player, env);
        self.simulation_state = env.bullet.get_ragdoll_simulation_state(env.character.id());
        self.simulate_hit_reaction(env);
    }

    /// `postUpdate(deltaT)`.
    pub fn post_update(&mut self, player: &AnimationPlayer, env: &mut RagdollEnv, dt: f32) {
        self.calculate_simulation_data(player, env, dt);
        self.update_simulation_timeout(player, env);
    }

    fn update_simulation_timeout(&mut self, player: &AnimationPlayer, env: &mut RagdollEnv) {
        let mut end = false;
        if self.state.simulation_timeout > 0.0 {
            self.state.simulation_timeout -= env.time_delta;
        }
        if !self.state.is_simulation_movement && self.state.simulation_timeout <= 0.0 {
            end = true;
        }
        if self.is_simulation_sleeping() && end {
            self.state.is_simulating = false;
            self.set_active(player, env, false);
        }
    }

    /// `simulateHitReaction()`.
    fn simulate_hit_reaction(&mut self, env: &mut RagdollEnv) {
        let Some(hit) = env.character.take_ragdoll_hit() else {
            return;
        };
        if hit.body_part == crate::RagdollBodyPart::Count as i32 || !env.debug.physics_hit_reaction
        {
            return;
        }
        let mut dir = Vec3::sub(&hit.target, &hit.attacker);
        dir.normalize();
        let buf = [
            dir.x * hit.impulse,
            hit.upward_impulse,
            dir.y * hit.impulse,
            0.0,
            0.0,
            0.0,
        ];
        env.bullet
            .apply_impulse(env.character.id(), hit.body_part, &buf);
    }

    /// `simulateRagdoll(...)`.
    fn simulate_ragdoll(&mut self, player: &mut AnimationPlayer, env: &mut RagdollEnv) {
        self.update_ragdoll_world_transform(player, env);
        self.set_ragdoll_local_rotation(env);
        let mut skel = [0f32; COUNT * 7];
        let mut rb = [0f32; 77];
        let n = env
            .bullet
            .simulate_ragdoll_with_rigid_body_output(env.character.id(), &mut skel, &mut rb)
            .unwrap_or(0);
        self.set_bone_transforms_to_animation(player, env, &skel, n);
        self.state.simulation_rendered_angle = player.rendered_angle();
        self.state.simulation_character_forward_angle = Self::anim_angle_radians(player, env);
    }

    /// `setRagdollLocalRotation()`.
    fn set_ragdoll_local_rotation(&self, env: &mut RagdollEnv) {
        let q = ragdoll_local_rotation();
        env.bullet
            .set_ragdoll_local_transform_rotation(env.character.id(), q.x, q.y, q.z, q.w);
    }

    /// `updateRagdollWorldTransform(...)`.
    fn update_ragdoll_world_transform(&mut self, player: &AnimationPlayer, env: &mut RagdollEnv) {
        self.calculate_ragdoll_world_transform(player, env);
        let p = self.ragdoll_world_position_pz_bullet;
        let q = self.ragdoll_world_rotation_pz_bullet;
        env.bullet
            .update_ragdoll(env.character.id(), p[0], p[1], p[2], q.x, q.y, q.z, q.w);
    }

    /// `calculateRagdollWorldTransform(...)`: the character's position in
    /// Bullet space and its yaw.
    fn calculate_ragdoll_world_transform(&mut self, player: &AnimationPlayer, env: &RagdollEnv) {
        let p = env.character.position();
        self.ragdoll_world_position = p;
        self.ragdoll_world_position_pz_bullet = pz_space_to_bullet_space([p.x, p.y, p.z]);
        let forward = Self::anim_angle_radians(player, env);
        self.ragdoll_world_rotation_pz_bullet =
            helpers::set_from_axis_angle(0.0, 1.0, 0.0, -forward);
    }

    /// `setBoneTransformsToAnimation(floats, numberOfBones)`: the simulated
    /// skeleton into the ragdoll clip's keyframes (the previous pose into
    /// the first keyframe).
    fn set_bone_transforms_to_animation(
        &self,
        player: &mut AnimationPlayer,
        env: &RagdollEnv,
        f: &[f32],
        n: i32,
    ) {
        if player.ragdoll_simulation_animation_clip().is_none() {
            return;
        }
        if self.is_first_frame(env.bullet_frame_no) {
            return;
        }
        player
            .ragdoll_simulation_animation_clip()
            .unwrap()
            .set_ragdoll_simulation_active(true);
        let bones = sb::bone_map(player.skeleton());
        let mut writes = Vec::new();
        for (i, bone) in bones.iter().enumerate().take(n.max(0) as usize) {
            let k = i * 7;
            let mut pos = Vector3f::new(f[k] / 1.5, f[k + 1] / 1.5, f[k + 2] / 1.5);
            let rot = Quaternion::new(f[k + 3], f[k + 4], f[k + 5], f[k + 6]);
            let Some(bone) = *bone else { continue };
            pos.x *= -1.0;
            pos.y *= -1.0;
            pos.z *= -1.0;
            writes.push((bone, pos, rot));
        }
        player.edit_ragdoll_clip(|clip| {
            for (bone, pos, rot) in writes {
                clip.edit_bone_frames(bone, |frames| {
                    // setBoneKeyframePRS: the two keyframes of the bone.
                    let cur = *frames[1];
                    frames[0].position = cur.position;
                    frames[0].rotation = cur.rotation;
                    frames[0].scale = cur.scale;
                    frames[1].position = pos;
                    frames[1].rotation = rot;
                    frames[1].scale = Vector3f::ONE;
                });
            }
        });
    }

    /// `calculateSimulationData(ragdollStateData, deltaT)`.
    fn calculate_simulation_data(
        &mut self,
        player: &AnimationPlayer,
        env: &mut RagdollEnv,
        dt: f32,
    ) {
        if self.is_first_frame(env.bullet_frame_no) {
            return;
        }
        let pos = env.character.position();
        let size = env.character.animal_size();
        let b = env.builder;
        let at = |bone: Option<usize>| bone_to_world(player, pos, size, bone.unwrap_or(0));
        self.previous_head_position = self.head_position;
        self.previous_pelvis_position = self.pelvis_position;
        self.head_position = at(b.head_bone);
        self.pelvis_position = at(b.pelvis_bone);
        if !self.state.is_calculated {
            self.previous_head_position = self.head_position;
            self.previous_pelvis_position = self.pelvis_position;
            self.desired_character_position = pos;
        }
        let d = &mut self.desired_character_position;
        *d = Vec3::new(
            d.x + (self.pelvis_position.x - self.previous_pelvis_position.x),
            d.y + (self.pelvis_position.y - self.previous_pelvis_position.y),
            pzmath_round_float(
                d.z + (self.pelvis_position.z - self.previous_pelvis_position.z),
                1,
            ),
        );
        self.state.is_simulation_movement = false;
        let head_distance = self.previous_head_position.distance_to(&self.head_position);
        let pelvis_distance = self
            .previous_pelvis_position
            .distance_to(&self.pelvis_position);
        if self.state.is_contacting_vehicle {
            self.state.simulation_timeout = 1.5;
            self.state.is_contacting_vehicle = false;
        } else if head_distance > MOVEMENT_THRESHOLD || pelvis_distance > MOVEMENT_THRESHOLD {
            if self.state.simulation_timeout < 1.5 {
                self.state.simulation_timeout = 1.5;
            }
            self.state.is_simulation_movement = true;
        } else if self.is_simulation_sleeping() {
            self.state.simulation_timeout -= dt * SIMULATION_TIMEOUT_DECAY_FACTOR;
        }
        let ground = bone_to_world(player, pos, size, 0);
        self.is_upright = self.head_position.z > ground.z + 0.6 - 0.2;
        env.character.set_on_floor(!self.is_upright);
        self.state.pelvis_direction = match b.pelvis_bone {
            Some(p) => bone_z_direction_to_world(player, pos, p, 0.5),
            None => bone_z_direction_to_world(player, pos, 0, 0.5),
        };
        self.left_shoulder_position = at(b.left_shoulder);
        self.right_shoulder_position = at(b.right_shoulder);
        let center = Vec3::lerp(
            &self.left_shoulder_position,
            &self.right_shoulder_position,
            0.5,
        );
        self.is_on_back = !env.character.is_fall_on_front();
        if !self.is_upright && player.is_fully_ragdolling() {
            let mut right = Vec3::sub(&self.right_shoulder_position, &center);
            right.z = 0.0;
            let mut up = Vec3::sub(&center, &self.pelvis_position);
            up.z = 0.0;
            let mut forward = Vec3::cross(&right, &up);
            forward.normalize();
            if pzmath::abs(forward.z) > 0.2 {
                self.is_on_back = forward.z > 0.0;
            }
        }
        let dir = &mut self.state.simulation_direction;
        if self.is_on_back {
            dir.x = self.pelvis_position.x - center.x;
            dir.y = self.pelvis_position.y - center.y;
        } else {
            dir.x = center.x - self.pelvis_position.x;
            dir.y = center.y - self.pelvis_position.y;
        }
        self.state.is_calculated = true;
        dir.normalize();
        env.character.set_fall_on_front(!self.is_on_back);
        env.character.set_ragdoll_fall(true);
    }

    /// `vehicleCollision(zombie, collidedVehicle)`, with the vehicle's
    /// answers: `is_collided` (`lastCollidedVehicle.isCollided`) and
    /// `touching` (`testTouchingVehicle`), both `None` without a vehicle.
    pub fn vehicle_collision(&mut self, env: &mut RagdollEnv, vehicle: Option<(bool, bool)>) {
        let mut touching = false;
        if let Some((collided, t)) = vehicle {
            self.state.is_contacting_vehicle = collided;
            touching = t;
        }
        if self.was_contacting_vehicle && !touching {
            env.bullet.reset_ragdoll_body_dynamics(env.character.id());
        } else if !self.was_contacting_vehicle && touching {
            let _ = env.bullet.set_ragdoll_body_dynamics(
                env.character.id(),
                Some(&env.builder.vehicle_body_dynamics_params),
            );
        }
        self.was_contacting_vehicle = touching;
    }
}

/// `PZMath.roundFloat(value, scale)`.
pub fn pzmath_round_float(value: f32, scale: i32) -> f32 {
    let mut pow = 10i32;
    for _ in 1..scale {
        pow *= 10;
    }
    let tmp = value * pow as f32;
    let tmp_sub = tmp - (tmp as i32) as f32;
    let r = if value >= 0.0 {
        if tmp_sub >= 0.5 {
            tmp + 1.0
        } else {
            tmp
        }
    } else if tmp_sub >= -0.5 {
        tmp
    } else {
        tmp - 1.0
    };
    (r as i32) as f32 / pow as f32
}

/// `getBoneTransformsFromAnimation(floats)`: the animated local transforms
/// of the 35 bones, positions negated and scaled by 1.5.
pub fn bone_transforms_from_animation(player: &AnimationPlayer, f: &mut [f32]) {
    let bones = sb::bone_map(player.skeleton());
    for (i, bone) in bones.iter().enumerate() {
        let (p, r) = match bone {
            Some(b) => {
                let mut t = player.bone_transform_at(*b);
                (t.position(), t.rotation())
            }
            None => (Vector3f::ZERO, Quaternion::IDENTITY),
        };
        let k = i * 7;
        f[k..k + 7].copy_from_slice(&[-p.x * 1.5, -p.y * 1.5, -p.z * 1.5, r.x, r.y, r.z, r.w]);
    }
}

/// `getBoneTransformVelocitiesFromAnimation(floats, deltaT)`: each bone's
/// position and quaternion change since the previous frame over `dt`.
pub fn bone_transform_velocities_from_animation(player: &AnimationPlayer, f: &mut [f32], dt: f32) {
    let bones = sb::bone_map(player.skeleton());
    for (i, bone) in bones.iter().enumerate() {
        let (vp, vr) = match bone {
            Some(b) if dt > 0.0 => {
                let bt = &player.bone_transforms[*b];
                let mut cur = bt.transform;
                let mut prev = bt.previous;
                let (cp, cr) = (cur.position(), cur.rotation());
                let (pp, pr) = (prev.position(), prev.rotation());
                let m = 1.0 / dt;
                (
                    Vector3f::new((cp.x - pp.x) * m, (cp.y - pp.y) * m, (cp.z - pp.z) * m),
                    Quaternion::new(
                        (cr.x - pr.x) * m,
                        (cr.y - pr.y) * m,
                        (cr.z - pr.z) * m,
                        (cr.w - pr.w) * m,
                    ),
                )
            }
            _ => (Vector3f::ZERO, Quaternion::IDENTITY),
        };
        let k = i * 7;
        f[k..k + 7].copy_from_slice(&[
            -vp.x * 1.5,
            -vp.y * 1.5,
            -vp.z * 1.5,
            vr.x,
            vr.y,
            vr.z,
            vr.w,
        ]);
    }
}

/// The character's side of the ragdoll for one `AnimationPlayer.Update`:
/// the [`RagdollHost`] the player calls into, holding the character's
/// controller slot (`AnimationPlayer.ragdollController`).
pub struct RagdollFrame<'a> {
    pub env: RagdollEnv<'a>,
    pub controller: &'a mut Option<RagdollController>,
    /// `GameTime.getInstance().getMultiplier()`.
    pub multiplier: f32,
}

impl RagdollHost for RagdollFrame<'_> {
    fn has_character(&self) -> bool {
        true
    }

    fn can_ragdoll(&self) -> bool {
        if self.controller.is_none()
            && self.env.globals.number_of_active_simulations >= self.env.globals.max_active_ragdolls
        {
            return false;
        }
        self.env.character.can_ragdoll()
    }

    fn has_controller(&self) -> bool {
        self.controller.is_some()
    }

    fn alloc_controller(&mut self) {
        *self.controller = Some(RagdollController::new());
        self.env.character.on_ragdoll_simulation_started();
    }

    fn release_controller(&mut self) {
        if let Some(mut c) = self.controller.take() {
            c.reset(&mut self.env);
        }
    }

    fn is_initialized(&self) -> bool {
        self.controller.as_ref().is_some_and(|c| c.is_initialized())
    }

    fn is_first_frame(&self) -> bool {
        self.controller
            .as_ref()
            .is_some_and(|c| c.is_first_frame(self.env.bullet_frame_no))
    }

    fn is_simulation_active(&self) -> bool {
        self.controller
            .as_ref()
            .is_some_and(|c| c.is_simulation_active())
    }

    fn is_simulation_direction_calculated(&self) -> bool {
        self.controller
            .as_ref()
            .is_some_and(|c| c.state.is_calculated)
    }

    fn desired_character_position(&self) -> Vector3f {
        let d = self
            .controller
            .as_ref()
            .map(|c| c.desired_character_position)
            .unwrap_or_default();
        Vector3f::new(d.x, d.y, d.z)
    }

    fn calculated_simulation_direction_angle(&self) -> f32 {
        self.controller
            .as_ref()
            .map(|c| c.calculated_simulation_direction_angle())
            .unwrap_or(0.0)
    }

    fn update(&mut self, player: &mut AnimationPlayer, dt: f32) {
        if let Some(c) = self.controller.as_mut() {
            c.update(player, &mut self.env, dt);
        }
    }

    fn post_update(&mut self, player: &mut AnimationPlayer, dt: f32) {
        if let Some(c) = self.controller.as_mut() {
            c.post_update(player, &mut self.env, dt);
        }
    }

    fn character_position(&self) -> Vector3f {
        let p = self.env.character.position();
        Vector3f::new(p.x, p.y, p.z)
    }

    fn do_deferred_movement_from_ragdoll(&mut self, player: &AnimationPlayer, m: Vector3f) {
        self.env
            .character
            .do_deferred_movement_from_ragdoll(player, Vec3::new(m.x, m.y, m.z));
    }

    fn game_time_multiplier(&self) -> f32 {
        self.multiplier
    }
}
