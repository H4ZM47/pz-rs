//! `BallisticsController` (a shooter's muzzle and aim in the physics world,
//! with the last query results) and `BallisticsTarget` (a character's
//! hit-box skeleton in the physics world, with the hit that the ragdoll
//! hit reaction reads).
//!
//! The model and animation inputs (the muzzle attachment, the reticle, the
//! bone transforms) come from the [`StateWorld`] hooks; the physics calls go
//! to [`StateWorld::physics`].

use std::rc::Rc;

use pz_scripts::registry::RegistryKey;

use super::physics::*;
use crate::character::state::StateWorld;
use crate::character::GameCharacter;

/// `ModelInstance.getAttachmentWorldPosition(MUZZLE)`: the muzzle
/// attachment's world position and (unnormalized) direction, iso space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MuzzleAttachment {
    pub position: [f32; 3],
    pub direction: [f32; 3],
}

/// What a ballistics target reads of its character's animation player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TargetSkeleton {
    /// `boneTransforms.length`.
    pub bone_count: i32,
    /// `getAngle()`.
    pub angle: f32,
    /// `getBoneTransforms()`: per `SkeletonBone` (in `SkeletonBone.all()`
    /// order) the animator's local bone transform as `-pos * 1.5` and the
    /// rotation `(x, y, z, w)` (identity for a bone the skeleton lacks), 7
    /// floats each (see [`pack_bone_transforms`]).
    pub packed: Vec<f32>,
}

/// Reborrows an optional physics world for one call.
pub(crate) fn reborrow<'a>(p: &'a mut Option<&mut dyn PhysicsWorld>) -> Option<&'a mut dyn PhysicsWorld> {
    match p {
        Some(p) => Some(&mut **p),
        None => None,
    }
}

/// `zombie.iso.Vector3` arithmetic in float.
pub(crate) fn v3_len(v: [f32; 3]) -> f32 {
    ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) as f64).sqrt() as f32
}

/// `Vector3.normalize()`.
pub(crate) fn v3_normalize(v: [f32; 3]) -> [f32; 3] {
    let l = v3_len(v);
    if l == 0.0 {
        [0.0; 3]
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

/// `Vector3.distanceTo(x, y, z)`: through `Math.pow` in double.
pub(crate) fn v3_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = |p: f32, q: f32| ((p - q) as f64).powf(2.0);
    (d(b[0], a[0]) + d(b[1], a[1]) + d(b[2], a[2])).sqrt() as f32
}

/// `pzCameraRotation.rotationYXZ(0.7853982f, -0.5235988f, 0f)`: JOML's
/// result (its `cosFromSin` path), as bits.
pub fn camera_rotation() -> [f32; 4] {
    [f32::from_bits(0xbe74_db3f), f32::from_bits(0x3ebd_41f5), f32::from_bits(0x3dca_d882), f32::from_bits(0x3f64_7444)]
}

/// `BallisticsController`.
#[derive(Clone, Debug, PartialEq)]
pub struct BallisticsController {
    pub muzzle_position: [f32; 3],
    pub muzzle_direction: [f32; 3],
    pub converted_muzzle_direction: [f32; 3],
    pub targets: [f32; MAX_BALLISTICS_TARGETS_ARRAY_SIZE],
    pub camera_targets: [f32; MAX_BALLISTICS_CAMERA_TARGETS_ARRAY_SIZE],
    pub spread_data: [f32; MAX_BALLISTICS_SPREAD_LOCATIONS_ARRAY_SIZE],
    pub cached_targets: [f32; MAX_BALLISTICS_TARGETS_ARRAY_SIZE],
    pub cached_camera_targets: [f32; MAX_BALLISTICS_CAMERA_TARGETS_ARRAY_SIZE],
    pub cached_spread_data: [f32; MAX_BALLISTICS_SPREAD_LOCATIONS_ARRAY_SIZE],
    pub number_of_targets: i32,
    pub number_of_spread_data: i32,
    pub number_of_camera_targets: i32,
    pub cached_number_of_targets: i32,
    pub cached_number_of_spread_data: i32,
    pub cached_number_of_camera_targets: i32,
    pub is_initialized: bool,
    pub iso_aiming_position: [f32; 3],
    pub target_position: [f32; 3],
}

impl Default for BallisticsController {
    fn default() -> Self {
        BallisticsController {
            muzzle_position: [0.0; 3],
            muzzle_direction: [0.0; 3],
            converted_muzzle_direction: [0.0; 3],
            targets: [0.0; MAX_BALLISTICS_TARGETS_ARRAY_SIZE],
            camera_targets: [0.0; MAX_BALLISTICS_CAMERA_TARGETS_ARRAY_SIZE],
            spread_data: [0.0; MAX_BALLISTICS_SPREAD_LOCATIONS_ARRAY_SIZE],
            cached_targets: [0.0; MAX_BALLISTICS_TARGETS_ARRAY_SIZE],
            cached_camera_targets: [0.0; MAX_BALLISTICS_CAMERA_TARGETS_ARRAY_SIZE],
            cached_spread_data: [0.0; MAX_BALLISTICS_SPREAD_LOCATIONS_ARRAY_SIZE],
            number_of_targets: 0,
            number_of_spread_data: 0,
            number_of_camera_targets: 0,
            cached_number_of_targets: 0,
            cached_number_of_spread_data: 0,
            cached_number_of_camera_targets: 0,
            is_initialized: false,
            iso_aiming_position: [0.0; 3],
            target_position: [0.0; 3],
        }
    }
}

fn count_in(arr: &[f32], n: i32, stride: usize, id: i32) -> i32 {
    (0..n.max(0) as usize).filter(|i| arr[i * stride] == id as f32).count() as i32
}

impl BallisticsController {
    /// `getMuzzlePositionAtCharacterPlane`: moves the muzzle onto the plane
    /// through the character's position along the muzzle direction; returns
    /// the distance moved.
    pub fn muzzle_position_at_character_plane(char_pos: [f32; 3], m: &MuzzleAttachment) -> ([f32; 3], f32) {
        let len = v3_len(m.direction);
        if len <= 0.0 {
            return (m.position, 0.0);
        }
        let inv = 1.0 / len;
        let px = m.direction[0] * inv;
        let py = m.direction[1] * inv;
        let pz = m.direction[2] * inv;
        let pw = -(char_pos[0] * px + char_pos[1] * py + char_pos[2] * pz);
        let dist = px * m.position[0] + py * m.position[1] + pz * m.position[2] + pw;
        ([m.position[0] - px * dist, m.position[1] - py * dist, m.position[2] - pz * dist], dist)
    }

    /// `calculateMuzzlePosition(pos, dirUnnormalized)`: the muzzle at the
    /// character plane, its unnormalized direction and the offset distance
    /// (0 without a muzzle).
    pub fn calculate_muzzle_position(char_pos: [f32; 3], muzzle: Option<&MuzzleAttachment>) -> Option<([f32; 3], [f32; 3], f32)> {
        let m = muzzle?;
        let (p, d) = Self::muzzle_position_at_character_plane(char_pos, m);
        Some((p, m.direction, d))
    }

    /// `initialize()` (it never sets `isInitialized`).
    fn initialize(&self, id: i32, physics: Option<&mut dyn PhysicsWorld>, server: bool) {
        if self.is_initialized {
            return;
        }
        if !server {
            if let Some(p) = physics {
                let q = camera_rotation();
                p.update_ballistics_aim_reticle_quaternion(id, q[0], q[1], q[2], q[3]);
            }
        }
    }

    /// `update()`: `weapon_range` is the attacking weapon's
    /// `getMaxRange() * getRangeMod(chr)`; `reticle` the aiming reticle in
    /// iso `(x, y)`.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        id: i32,
        char_pos: [f32; 3],
        muzzle: Option<&MuzzleAttachment>,
        reticle: (f32, f32),
        weapon_range: f32,
        server: bool,
        debug: bool,
        mut physics: Option<&mut dyn PhysicsWorld>,
    ) {
        if !self.is_initialized {
            self.initialize(id, reborrow(&mut physics), server);
        }
        let Some((p, d, _)) = Self::calculate_muzzle_position(char_pos, muzzle) else { return };
        self.muzzle_position = p;
        self.muzzle_direction = d;
        self.converted_muzzle_direction = v3_normalize([d[0], d[2] * BULLET_Z_SCALE, d[1]]);
        self.muzzle_direction = v3_normalize(self.muzzle_direction);
        let (x, y) = reticle;
        self.iso_aiming_position = [x, y, 0.0];
        if !server {
            if let Some(ph) = reborrow(&mut physics) {
                let mp = self.muzzle_position;
                ph.update_ballistics(id, mp[0], mp[2] * BULLET_Z_SCALE, mp[1]);
                ph.update_ballistics_aim_reticle_position(id, x, 0.0, y);
                let c = self.converted_muzzle_direction;
                ph.update_ballistics_muzzle_aim_direction(id, c[0], c[1], c[2]);
            }
        }
        if self.has_ballistics_target() {
            self.target_position = self.camera_target(0);
        } else {
            let (mp, md) = (self.muzzle_position, self.muzzle_direction);
            self.target_position = [mp[0] + md[0] * weapon_range, mp[1] + md[1] * weapon_range, mp[2] + md[2] * weapon_range];
        }
        if !self.is_initialized {
            if !server {
                if let Some(ph) = reborrow(&mut physics) {
                    if debug {
                        ph.set_ballistics_size(id, 0.025);
                        ph.set_ballistics_color(id, 0.0, 0.0, 1.0);
                    }
                    let q = camera_rotation();
                    ph.update_ballistics_aim_reticle_quaternion(id, q[0], q[1], q[2], q[3]);
                }
            }
            self.is_initialized = true;
        }
    }

    /// `setRange(range)`.
    pub fn set_range(&self, id: i32, range: f32, physics: Option<&mut dyn PhysicsWorld>) {
        if let Some(p) = physics {
            p.set_ballistics_range(id, range);
        }
    }

    /// `getTargets(range)`.
    pub fn get_targets(&mut self, id: i32, range: f32, physics: Option<&mut dyn PhysicsWorld>) {
        self.number_of_targets = match physics {
            Some(p) => {
                let n = p.get_ballistics_targets(id, range, MAX_BALLISTICS_TARGETS, &mut self.targets);
                self.cached_targets = self.targets;
                n
            }
            None => 0,
        };
        self.cached_number_of_targets = self.number_of_targets;
    }

    /// `getSpreadData(range, spread, weightCenter, numberOfBullets)`.
    pub fn get_spread_data(&mut self, id: i32, range: f32, spread: f32, weight_center: f32, bullets: i32, physics: Option<&mut dyn PhysicsWorld>) {
        self.number_of_spread_data = match physics {
            Some(p) => {
                let n = p.get_ballistics_targets_spread_data(id, range, spread, weight_center, bullets, MAX_BALLISTICS_SPREAD_LOCATIONS, &mut self.spread_data);
                self.cached_spread_data = self.spread_data;
                n
            }
            None => 0,
        };
        self.cached_number_of_spread_data = self.number_of_spread_data;
    }

    /// `getCameraTargets(range, parts)`.
    pub fn get_camera_targets(&mut self, id: i32, range: f32, parts: bool, physics: Option<&mut dyn PhysicsWorld>) {
        self.number_of_camera_targets = match physics {
            Some(p) => {
                let n = p.get_ballistics_camera_targets(id, range, MAX_BALLISTICS_CAMERA_TARGETS, parts, &mut self.camera_targets);
                self.cached_camera_targets = self.camera_targets;
                n
            }
            None => 0,
        };
        self.cached_number_of_camera_targets = self.number_of_camera_targets;
    }

    pub fn is_valid_target(&self, id: i32) -> bool {
        self.is_target(id) || self.is_camera_target(id) || self.is_spread_target(id)
    }

    pub fn is_valid_cached_target(&self, id: i32) -> bool {
        self.is_cached_target(id) || self.is_cached_camera_target(id) || self.is_cached_spread_target(id)
    }

    pub fn is_target(&self, id: i32) -> bool {
        count_in(&self.targets, self.number_of_targets, 4, id) > 0
    }

    pub fn is_cached_target(&self, id: i32) -> bool {
        count_in(&self.cached_targets, self.cached_number_of_targets, 4, id) > 0
    }

    pub fn is_camera_target(&self, id: i32) -> bool {
        count_in(&self.camera_targets, self.number_of_camera_targets, 5, id) > 0
    }

    pub fn is_cached_camera_target(&self, id: i32) -> bool {
        count_in(&self.cached_camera_targets, self.cached_number_of_camera_targets, 5, id) > 0
    }

    pub fn is_spread_target(&self, id: i32) -> bool {
        count_in(&self.spread_data, self.number_of_spread_data, 4, id) > 0
    }

    pub fn is_cached_spread_target(&self, id: i32) -> bool {
        count_in(&self.cached_spread_data, self.cached_number_of_spread_data, 4, id) > 0
    }

    /// `spreadCount(id)`.
    pub fn spread_count(&self, id: i32) -> i32 {
        count_in(&self.spread_data, self.number_of_spread_data, 4, id)
    }

    /// `getTargetedBodyPart(id)`.
    pub fn targeted_body_part(&self, id: i32) -> i32 {
        Self::part_in(&self.camera_targets, self.number_of_camera_targets, id)
    }

    /// `getCachedTargetedBodyPart(id)`.
    pub fn cached_targeted_body_part(&self, id: i32) -> i32 {
        Self::part_in(&self.cached_camera_targets, self.cached_number_of_camera_targets, id)
    }

    fn part_in(arr: &[f32], n: i32, id: i32) -> i32 {
        for i in 0..n.max(0) as usize {
            if arr[i * 5] == id as f32 {
                return arr[i * 5 + 4] as i32;
            }
        }
        RagdollBodyPart::Count.ordinal()
    }

    /// `hasSpreadData()`.
    pub fn has_spread_data(&self) -> bool {
        self.number_of_spread_data != 0
    }

    /// `hasBallisticsTarget()`.
    pub fn has_ballistics_target(&self) -> bool {
        self.number_of_camera_targets > 0
    }

    /// `getCameraTarget(index, out)`: `(x, z, y / 2.44949)` of a record.
    fn camera_target(&self, index: usize) -> [f32; 3] {
        let x = self.camera_targets[index * 5 + 1];
        let y = self.camera_targets[index * 5 + 2] / BULLET_Z_SCALE;
        let z = self.camera_targets[index * 5 + 3];
        [x, z, y]
    }

    /// `setBallisticsTargetHitLocation(id, hitInfo)`: the hit point of the
    /// target's record, if any.
    pub fn target_hit_location(&self, id: i32) -> Option<[f32; 3]> {
        for i in 0..self.number_of_targets.max(0) as usize {
            let a = i * 4;
            if self.targets[a] as i32 == id {
                return Some([self.targets[a + 1], self.targets[a + 3], self.targets[a + 2] / BULLET_Z_SCALE]);
            }
        }
        None
    }

    /// `setBallisticsCameraTargetHitLocation(id, hitInfo)`.
    pub fn camera_target_hit_location(&self, id: i32) -> Option<[f32; 3]> {
        for i in 0..self.number_of_camera_targets.max(0) as usize {
            let a = i * 5;
            if self.camera_targets[a] as i32 == id {
                return Some([self.camera_targets[a + 1], self.camera_targets[a + 3], self.camera_targets[a + 2] / BULLET_Z_SCALE]);
            }
        }
        None
    }

    /// `postUpdate()`.
    pub fn post_update(&mut self) {
        self.number_of_targets = 0;
        self.number_of_spread_data = 0;
        self.number_of_camera_targets = 0;
    }

    /// `clearCacheTargets()`.
    pub fn clear_cache_targets(&mut self) {
        self.cached_number_of_targets = 0;
        self.cached_number_of_camera_targets = 0;
        self.cached_number_of_spread_data = 0;
    }

    /// `releaseController()`: out of the physics world, reset.
    pub fn release(&mut self, id: i32, physics: Option<&mut dyn PhysicsWorld>) {
        if let Some(p) = physics {
            p.remove_ballistics(id);
        }
        self.number_of_targets = 0;
        self.number_of_spread_data = 0;
        self.number_of_camera_targets = 0;
        self.is_initialized = false;
    }
}

/// `BallisticsTarget.CombatDamageData`: the hit the ragdoll hit reaction
/// applies.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatDamageData {
    pub event: String,
    pub target: i32,
    pub attacker: i32,
    pub body_part: RagdollBodyPart,
    /// `handWeapon.getAmmoType()`, `isExplosive()`, `getPhysicsObject()`.
    pub weapon_ammo_type: Option<Rc<RegistryKey>>,
    pub weapon_explosive: bool,
    pub weapon_physics_object: Option<String>,
    /// The attacker's position when the hit was made.
    pub attacker_position: [f32; 3],
}

impl Default for CombatDamageData {
    fn default() -> Self {
        CombatDamageData {
            event: String::new(),
            target: -1,
            attacker: -1,
            body_part: RagdollBodyPart::Count,
            weapon_ammo_type: None,
            weapon_explosive: false,
            weapon_physics_object: None,
            attacker_position: [0.0; 3],
        }
    }
}

/// `BallisticsTarget`.
#[derive(Clone, Debug, PartialEq)]
pub struct BallisticsTarget {
    pub is_initialized: bool,
    pub added_to_world: bool,
    pub number_of_bones: i32,
    pub combat_damage_data: CombatDamageData,
    pub combat_damage_data_processed: bool,
    pub release_frame: i32,
}

impl Default for BallisticsTarget {
    fn default() -> Self {
        BallisticsTarget {
            is_initialized: false,
            added_to_world: false,
            number_of_bones: -1,
            combat_damage_data: CombatDamageData::default(),
            combat_damage_data_processed: true,
            release_frame: 0,
        }
    }
}

thread_local! {
    /// `BallisticsTarget.boneTransformData` (static, grows, never shrinks).
    static BONE_TRANSFORM_DATA: std::cell::RefCell<Vec<f32>> = std::cell::RefCell::new(vec![0.0; SKELETON_BONE_COUNT * 7]);
}

/// Packs the animator's bone transforms as `getBoneTransforms` does:
/// `-pos * 1.5` and the rotation, 7 floats per `SkeletonBone`.
pub fn pack_bone_transforms(bones: &[Option<([f32; 3], [f32; 4])>], out: &mut [f32]) {
    let mut i = 0;
    for b in bones {
        let (p, r) = b.unwrap_or(([0.0; 3], [0.0, 0.0, 0.0, 1.0]));
        out[i] = -p[0] * 1.5;
        out[i + 1] = -p[1] * 1.5;
        out[i + 2] = -p[2] * 1.5;
        out[i + 3] = r[0];
        out[i + 4] = r[1];
        out[i + 5] = r[2];
        out[i + 6] = r[3];
        i += 7;
    }
}

impl BallisticsTarget {
    /// `initialize()` (it never sets `isInitialized` itself).
    fn initialize(&mut self, skeleton: Option<&TargetSkeleton>) -> bool {
        if self.is_initialized {
            return true;
        }
        let Some(s) = skeleton else { return false };
        self.number_of_bones = s.bone_count;
        BONE_TRANSFORM_DATA.with(|d| {
            let mut d = d.borrow_mut();
            let need = (self.number_of_bones.max(0) as usize) * 7;
            if d.len() < need {
                *d = vec![0.0; need];
            }
        });
        true
    }

    fn update_skeleton(&self, id: i32, skeleton: Option<&TargetSkeleton>, physics: &mut dyn PhysicsWorld) {
        let Some(s) = skeleton else { return };
        BONE_TRANSFORM_DATA.with(|d| {
            let mut d = d.borrow_mut();
            let n = s.packed.len().min(d.len());
            d[..n].copy_from_slice(&s.packed[..n]);
            physics.update_ballistics_target_skeleton(id, self.number_of_bones, &d);
        });
    }

    fn add_to_world(&mut self, id: i32, physics: &mut dyn PhysicsWorld) -> bool {
        if !self.added_to_world {
            physics.add_ballistics_target(id);
            self.added_to_world = true;
            return true;
        }
        false
    }

    /// `removeFromWorld()`.
    fn remove_from_world(&mut self, id: i32, physics: Option<&mut dyn PhysicsWorld>) {
        if self.added_to_world {
            if let Some(p) = physics {
                p.remove_ballistics_target(id);
            }
            self.added_to_world = false;
        }
    }

    /// `update()`: true when the target should be released.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        id: i32,
        pos: [f32; 3],
        direction_angle_degrees: f32,
        skeleton: Option<&TargetSkeleton>,
        server: bool,
        physics: Option<&mut dyn PhysicsWorld>,
    ) -> bool {
        if !self.is_initialized {
            self.is_initialized = self.initialize(skeleton);
            return false;
        }
        if self.combat_damage_data_processed {
            if self.release_frame > 0 {
                self.release_frame -= 1;
            }
            if self.release_frame == 0 {
                return true;
            }
        }
        if server {
            return false;
        }
        let Some(physics) = physics else { return false };
        self.add_to_world(id, physics);
        let forward = direction_angle_degrees;
        let angle = skeleton.map_or(0.0, |s| s.angle);
        physics.set_ballistics_target_axis(id, -1.570_796_4, -1.570_796_4 + angle, std::f32::consts::PI);
        let half = (forward * 0.5) as f64;
        physics.update_ballistics_target(id, pos[0], pos[2] * BULLET_Z_SCALE, pos[1], 0.0, half.sin() as f32, 0.0, half.cos() as f32, false);
        self.update_skeleton(id, skeleton, physics);
        false
    }

    /// `add()`: `pelvis` is the ragdoll controller's pelvis position, if
    /// the character has one.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        id: i32,
        pos: [f32; 3],
        pelvis: Option<[f32; 3]>,
        direction_angle_degrees: f32,
        skeleton: Option<&TargetSkeleton>,
        physics: &mut dyn PhysicsWorld,
    ) {
        self.release_frame = 2;
        if !self.add_to_world(id, physics) {
            return;
        }
        self.initialize(skeleton);
        let forward = direction_angle_degrees;
        let angle = skeleton.map_or(0.0, |s| s.angle);
        physics.set_ballistics_target_axis(id, -1.570_796_4, -1.570_796_4 + angle, std::f32::consts::PI);
        let p = pelvis.unwrap_or(pos);
        let half = (forward * 0.5) as f64;
        physics.update_ballistics_target(id, p[0], p[2] * BULLET_Z_SCALE, p[1], 0.0, half.sin() as f32, 0.0, half.cos() as f32, false);
        self.update_skeleton(id, skeleton, physics);
    }

    /// `releaseTarget()`.
    pub fn release(&mut self, id: i32, physics: Option<&mut dyn PhysicsWorld>) {
        self.remove_from_world(id, physics);
        *self = BallisticsTarget { combat_damage_data: std::mem::take(&mut self.combat_damage_data), ..BallisticsTarget::default() };
    }
}

impl<I: 'static> GameCharacter<I> {
    /// `updateBallistics()`: creates the controller and updates it.
    pub fn update_ballistics(&mut self, sm: &pz_scripts::ScriptManager, chars: &crate::Characters, ctx: &dyn crate::SimContext, world: &mut dyn StateWorld<I>) {
        if ctx.is_server() {
            return;
        }
        let mut c = match self.ballistics_controller.take() {
            Some(c) => c,
            None => Box::new(chars.combat.borrow_mut().controller_pool.pop().unwrap_or_default()),
        };
        let range = self.attacking_weapon_range(sm, chars);
        let muzzle = world.muzzle_attachment(self.id);
        let reticle = world.aim_reticle_iso(self.id);
        c.update(self.id, [self.vars.x, self.vars.y, self.vars.z], muzzle.as_ref(), reticle, range, false, ctx.is_debug(), world.physics());
        self.ballistics_controller = Some(c);
    }

    /// `releaseBallisticsController()`: back to the pool.
    pub fn release_ballistics_controller(&mut self, chars: &crate::Characters, world: &mut dyn StateWorld<I>) {
        if let Some(mut c) = self.ballistics_controller.take() {
            c.release(self.id, world.physics());
            chars.combat.borrow_mut().controller_pool.push(*c);
        }
    }

    /// `ensureExistsBallisticsTarget(this)`.
    pub fn ensure_ballistics_target(&mut self) -> &mut BallisticsTarget {
        self.ballistics_target.get_or_insert_with(Default::default)
    }

    /// `updateBallisticsTarget()`: called from `updateInternal` for
    /// non-animals.
    pub fn update_ballistics_target(&mut self, ctx: &dyn crate::SimContext, world: &mut dyn StateWorld<I>) {
        let Some(mut t) = self.ballistics_target.take() else { return };
        let skeleton = world.ballistics_skeleton(self.id);
        let release = t.update(self.id, [self.vars.x, self.vars.y, self.vars.z], self.direction_angle(), skeleton.as_ref(), ctx.is_server(), world.physics());
        if release {
            t.release(self.id, world.physics());
        } else {
            self.ballistics_target = Some(t);
        }
    }

    /// `releaseBallisticsTarget()`.
    pub fn release_ballistics_target(&mut self, world: &mut dyn StateWorld<I>) {
        if let Some(mut t) = self.ballistics_target.take() {
            t.release(self.id, world.physics());
        }
    }

    /// `getAttackingWeapon()`'s `getMaxRange() * getRangeMod(this)` (what
    /// `BallisticsController.update` aims along).
    pub fn attacking_weapon_range(&self, sm: &pz_scripts::ScriptManager, chars: &crate::Characters) -> f32 {
        let (w, _) = self.attacking_weapon(sm);
        w.max_range * self.weapon_range_mod(sm, chars, &w)
    }
}
