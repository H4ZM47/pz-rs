//! `RagdollBuilder`: hands the physics library the skeleton hierarchy and
//! the T-pose the ragdoll's bodies are fitted to.

use pz_anim::clip::AnimClip;
use pz_anim::skeleton::Skeleton;
use pz_bullet::PzBullet;

use crate::controller::ragdoll_local_rotation;
use crate::script::RagdollBodyDynamics;
use crate::skeleton_bone::{self as sb, COUNT};

/// `RagdollController.vehicleCollisionFriction`.
pub const VEHICLE_COLLISION_FRICTION: f32 = 0.4;

/// `RagdollBuilder.instance`, after `Initialize()`.
#[derive(Clone, Debug)]
pub struct RagdollBuilder {
    /// `boneHierarchy` (sized `numberOfBones * 7`, the first 35 used).
    pub bone_hierarchy: Vec<i32>,
    /// `boneTransformData`: the T-pose, 7 floats per bone.
    pub bone_transform_data: Vec<f32>,
    /// `pelvisBone`, `headBone`, `leftShoulder`, `rightShoulder` (skinning
    /// bone indices in the T-pose's skeleton).
    pub pelvis_bone: Option<usize>,
    pub head_bone: Option<usize>,
    pub left_shoulder: Option<usize>,
    pub right_shoulder: Option<usize>,
    /// `mass`.
    pub mass: f32,
    /// `RagdollController.vehicleRagdollBodyDynamicsParams`.
    pub vehicle_body_dynamics_params: [f32; 8],
}

impl RagdollBuilder {
    /// `RagdollBuilder.Initialize()` on a client: `skeleton` is the
    /// `bob/bob_tpose` asset's skinning data, `t_pose` the `Bob_TPose` clip,
    /// `body_dynamics` `RagdollScript.getRagdollBodyDynamicsList()`.
    pub fn initialize(
        bullet: &mut PzBullet,
        skeleton: &Skeleton,
        t_pose: &AnimClip,
        body_dynamics: &[RagdollBodyDynamics],
    ) -> RagdollBuilder {
        let n = COUNT;
        let mut b = RagdollBuilder {
            bone_hierarchy: vec![0; n * 7],
            bone_transform_data: vec![0.0; n * 7],
            pelvis_bone: None,
            head_bone: None,
            left_shoulder: None,
            right_shoulder: None,
            mass: 70.0,
            vehicle_body_dynamics_params: [0.0; 8],
        };
        b.skeleton_hierarchy(skeleton);
        bullet.initialize_ragdoll_skeleton(n as i32, &b.bone_hierarchy);
        bullet.set_ballistics_target_adjusting_shape_scale(0.35, 0.4, 0.35);
        b.initialize_ragdoll_pose(bullet, skeleton, t_pose);
        b.pelvis_bone = sb::find_bone(skeleton, sb::BIP01_PELVIS);
        b.head_bone = sb::find_bone(skeleton, sb::BIP01_HEAD);
        b.left_shoulder = sb::find_bone(skeleton, sb::BIP01_L_UPPER_ARM);
        b.right_shoulder = sb::find_bone(skeleton, sb::BIP01_R_UPPER_ARM);
        if let Some(d) = body_dynamics.first() {
            b.vehicle_body_dynamics_params = vehicle_body_dynamics_params(d);
        }
        b
    }

    /// `getSkinningDataSkeletonHierarchy`: each bone's parent, as
    /// `SkeletonBone` ordinals.
    fn skeleton_hierarchy(&mut self, skeleton: &Skeleton) {
        for i in 0..COUNT {
            self.bone_hierarchy[i] = match sb::find_bone(skeleton, i) {
                None => -1,
                Some(bone) => sb::parent_skeleton_bone_index(skeleton, bone),
            };
        }
    }

    /// `initializeRagdollPose()`.
    pub fn initialize_ragdoll_pose(
        &mut self,
        bullet: &mut PzBullet,
        skeleton: &Skeleton,
        t_pose: &AnimClip,
    ) {
        self.bone_transforms(skeleton, t_pose);
        let q = ragdoll_local_rotation();
        bullet.initialize_ragdoll_pose(COUNT as i32, &self.bone_transform_data, q.x, q.y, q.z, q.w);
    }

    /// `getBoneTransforms()`: the T-pose's second keyframe of every bone,
    /// position negated and scaled by 1.5.
    fn bone_transforms(&mut self, skeleton: &Skeleton, t_pose: &AnimClip) {
        let d = &mut self.bone_transform_data;
        for i in 0..COUNT {
            let k = i * 7;
            match sb::find_bone(skeleton, i) {
                None => d[k..k + 7].copy_from_slice(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
                Some(bone) => {
                    let kf = &t_pose.bone_frames(bone)[1];
                    let (p, r) = (kf.position, kf.rotation);
                    d[k..k + 7].copy_from_slice(&[
                        -p.x * 1.5,
                        -p.y * 1.5,
                        -p.z * 1.5,
                        r.x,
                        r.y,
                        r.z,
                        r.w,
                    ]);
                }
            }
        }
    }

    /// `setMass(mass)`.
    pub fn set_mass(&mut self, bullet: &mut PzBullet, mass: f32) {
        if self.mass != mass {
            bullet.set_ragdoll_mass(mass);
        }
        self.mass = mass;
    }
}

/// `RagdollController.setVehicleRagdollBodyDynamics(dynamics)`: the script
/// defaults of the first body part, with the vehicle friction, for every
/// part (`BODYPART_COUNT`).
pub fn vehicle_body_dynamics_params(d: &RagdollBodyDynamics) -> [f32; 8] {
    [
        crate::RagdollBodyPart::Count as i32 as f32,
        d.linear_damping,
        d.angular_damping,
        d.deactivation_time,
        d.linear_sleeping_threshold,
        d.angular_sleeping_threshold,
        VEHICLE_COLLISION_FRICTION,
        d.rolling_friction,
    ]
}
