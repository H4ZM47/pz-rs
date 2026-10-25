//! The physics the firearm code calls: the `zombie.core.physics.Bullet`
//! ballistics natives that `BallisticsController` and `BallisticsTarget`
//! use, with the argument order and float-array layouts of the natives.
//! Ragdolls are pz-ragdoll's (over `PzBullet` directly).
//!
//! The world (engine) owns the physics world and hands it out through
//! [`StateWorld::physics`](crate::character::state::StateWorld::physics).
//!
//! Positions cross in the physics frame: an iso position `(x, y, z level)`
//! is sent as `(x, z * 2.44949, y)` ([`BULLET_Z_SCALE`]).
//!
//! The natives that throw in the original (no world, an empty output array)
//! report nothing here: the methods return what the native returns after
//! the exception is cleared (0 for the counts). Native quirks are kept by
//! the implementation: shots ignore static geometry, body-part hit flags
//! never clear, `get_ballistics_targets` returns at most one hit and spread
//! rays use the platform's unseeded `rand()`.

/// The iso-to-physics height scale (`2.44949f`).
pub const BULLET_Z_SCALE: f32 = 2.44949;

/// `SkeletonBone.count()`: the bones the physics skeleton has.
pub const SKELETON_BONE_COUNT: usize = 35;

/// The float arrays' sizes (`BallisticsController`).
pub const MAX_BALLISTICS_TARGETS: i32 = 20;
pub const MAX_BALLISTICS_TARGETS_ARRAY_SIZE: usize = 80;
pub const MAX_BALLISTICS_CAMERA_TARGETS: i32 = 10;
pub const MAX_BALLISTICS_CAMERA_TARGETS_ARRAY_SIZE: usize = 50;
pub const MAX_BALLISTICS_SPREAD_LOCATIONS: i32 = 9;
pub const MAX_BALLISTICS_SPREAD_LOCATIONS_ARRAY_SIZE: usize = 36;

/// The Bullet natives the character code calls. Every method mirrors one
/// native; the defaults are what a missing world gives.
pub trait PhysicsWorld {
    /// `WorldSimulation.instance.getBulletFrameNo()`: the physics steps
    /// taken (counted by the engine around `stepSimulation`).
    fn frame_no(&self) -> i32 {
        0
    }

    // ---- ballistics: the shooter (`BallisticsController`) ----

    /// `updateBallistics(id, x, y, z)`: the muzzle position.
    fn update_ballistics(&mut self, id: i32, x: f32, y: f32, z: f32);
    /// `updateBallisticsAimReticlePosition(id, x, y, z)`.
    fn update_ballistics_aim_reticle_position(&mut self, id: i32, x: f32, y: f32, z: f32);
    /// `updateBallisticsMuzzleAimDirection(id, x, y, z)`.
    fn update_ballistics_muzzle_aim_direction(&mut self, id: i32, x: f32, y: f32, z: f32);
    /// `updateBallisticsAimReticleQuaternion(id, x, y, z, w)`.
    fn update_ballistics_aim_reticle_quaternion(&mut self, id: i32, x: f32, y: f32, z: f32, w: f32);
    /// `setBallisticsSize(id, size)` (debug drawing).
    fn set_ballistics_size(&mut self, _id: i32, _size: f32) {}
    /// `setBallisticsColor(id, r, g, b)` (debug drawing).
    fn set_ballistics_color(&mut self, _id: i32, _r: f32, _g: f32, _b: f32) {}
    /// `setBallisticsRange(id, range)`.
    fn set_ballistics_range(&mut self, id: i32, range: f32);
    /// `getBallisticsTargets(id, range, count, out)`: 4 floats per hit.
    fn get_ballistics_targets(&mut self, id: i32, range: f32, count: i32, out: &mut [f32]) -> i32;
    /// `getBallisticsTargetsSpreadData(id, range, spread, weightCenter,
    /// count, max, out)`: 4 floats per ray.
    #[allow(clippy::too_many_arguments)]
    fn get_ballistics_targets_spread_data(
        &mut self,
        id: i32,
        range: f32,
        spread: f32,
        weight_center: f32,
        count: i32,
        max: i32,
        out: &mut [f32],
    ) -> i32;
    /// `getBallisticsCameraTargets(id, range, count, parts, out)`: 5 floats
    /// per target (id, x, y, z, body part).
    fn get_ballistics_camera_targets(&mut self, id: i32, range: f32, count: i32, parts: bool, out: &mut [f32]) -> i32;
    /// `removeBallistics(id)`.
    fn remove_ballistics(&mut self, id: i32);

    // ---- ballistics: the targets (`BallisticsTarget`) ----

    /// `addBallisticsTarget(id)`: 0 added, 1 already in the world.
    fn add_ballistics_target(&mut self, id: i32) -> i32;
    /// `removeBallisticsTarget(id)`.
    fn remove_ballistics_target(&mut self, id: i32) -> i32;
    /// `setBallisticsTargetAxis(id, x, y, z)`.
    fn set_ballistics_target_axis(&mut self, id: i32, x: f32, y: f32, z: f32);
    /// `updateBallisticsTarget(id, x, y, z, qx, qy, qz, qw, b)`.
    #[allow(clippy::too_many_arguments)]
    fn update_ballistics_target(&mut self, id: i32, x: f32, y: f32, z: f32, qx: f32, qy: f32, qz: f32, qw: f32, b: bool);
    /// `updateBallisticsTargetSkeleton(id, numberOfBones, data)`: 7 floats
    /// per `SkeletonBone`.
    fn update_ballistics_target_skeleton(&mut self, id: i32, count: i32, data: &[f32]);
    /// `setBallisticsTargetAdjustingShapeScale(x, y, z)`.
    fn set_ballistics_target_adjusting_shape_scale(&mut self, x: f32, y: f32, z: f32);
}

/// `RagdollBodyPart` (ordinal order; `BODYPART_COUNT` is "none").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RagdollBodyPart {
    Pelvis,
    Spine,
    Head,
    LeftUpperLeg,
    LeftLowerLeg,
    RightUpperLeg,
    RightLowerLeg,
    LeftUpperArm,
    LeftLowerArm,
    RightUpperArm,
    RightLowerArm,
    Count,
}

impl RagdollBodyPart {
    pub const ALL: [RagdollBodyPart; 12] = [
        RagdollBodyPart::Pelvis,
        RagdollBodyPart::Spine,
        RagdollBodyPart::Head,
        RagdollBodyPart::LeftUpperLeg,
        RagdollBodyPart::LeftLowerLeg,
        RagdollBodyPart::RightUpperLeg,
        RagdollBodyPart::RightLowerLeg,
        RagdollBodyPart::LeftUpperArm,
        RagdollBodyPart::LeftLowerArm,
        RagdollBodyPart::RightUpperArm,
        RagdollBodyPart::RightLowerArm,
        RagdollBodyPart::Count,
    ];

    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        self as i32
    }

    /// `values()[i]` (`Count` past the end).
    pub fn from_ordinal(i: i32) -> RagdollBodyPart {
        usize::try_from(i).ok().and_then(|i| Self::ALL.get(i).copied()).unwrap_or(RagdollBodyPart::Count)
    }

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            RagdollBodyPart::Pelvis => "BODYPART_PELVIS",
            RagdollBodyPart::Spine => "BODYPART_SPINE",
            RagdollBodyPart::Head => "BODYPART_HEAD",
            RagdollBodyPart::LeftUpperLeg => "BODYPART_LEFT_UPPER_LEG",
            RagdollBodyPart::LeftLowerLeg => "BODYPART_LEFT_LOWER_LEG",
            RagdollBodyPart::RightUpperLeg => "BODYPART_RIGHT_UPPER_LEG",
            RagdollBodyPart::RightLowerLeg => "BODYPART_RIGHT_LOWER_LEG",
            RagdollBodyPart::LeftUpperArm => "BODYPART_LEFT_UPPER_ARM",
            RagdollBodyPart::LeftLowerArm => "BODYPART_LEFT_LOWER_ARM",
            RagdollBodyPart::RightUpperArm => "BODYPART_RIGHT_UPPER_ARM",
            RagdollBodyPart::RightLowerArm => "BODYPART_RIGHT_LOWER_ARM",
            RagdollBodyPart::Count => "BODYPART_COUNT",
        }
    }

    /// `isHead(value)`.
    pub fn is_head(value: i32) -> bool {
        value == RagdollBodyPart::Head as i32
    }

    /// `isLeg(value)`.
    pub fn is_leg(value: i32) -> bool {
        (RagdollBodyPart::LeftUpperLeg as i32..=RagdollBodyPart::RightLowerLeg as i32).contains(&value)
    }

    /// `isArm(value)`.
    pub fn is_arm(value: i32) -> bool {
        (RagdollBodyPart::LeftUpperArm as i32..=RagdollBodyPart::RightLowerArm as i32).contains(&value)
    }

    /// `getBodyPartType(value)`: the `BodyPartType` field the ragdoll part
    /// maps to; `secondary` is the `Rand.NextBool(2)` the original draws
    /// first (always, even for the parts that ignore it).
    pub fn body_part_type_field(value: i32, secondary: bool) -> &'static str {
        let pick = |a: &'static str, b: &'static str| if secondary { a } else { b };
        match Self::from_ordinal(value) {
            RagdollBodyPart::Head => pick("Head", "Neck"),
            RagdollBodyPart::LeftUpperLeg => pick("Groin", "UpperLeg_L"),
            RagdollBodyPart::LeftLowerLeg => pick("LowerLeg_L", "Foot_L"),
            RagdollBodyPart::RightUpperLeg => pick("Groin", "UpperLeg_R"),
            RagdollBodyPart::RightLowerLeg => pick("LowerLeg_R", "Foot_R"),
            RagdollBodyPart::Pelvis => pick("Torso_Lower", "Groin"),
            RagdollBodyPart::Spine => pick("Torso_Upper", "Torso_Lower"),
            RagdollBodyPart::LeftUpperArm => "UpperArm_L",
            RagdollBodyPart::LeftLowerArm => pick("ForeArm_L", "Hand_L"),
            RagdollBodyPart::RightUpperArm => "UpperArm_R",
            RagdollBodyPart::RightLowerArm => pick("ForeArm_R", "Hand_R"),
            RagdollBodyPart::Count => "MAX",
        }
    }
}

