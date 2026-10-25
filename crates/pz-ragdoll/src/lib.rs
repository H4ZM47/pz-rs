//! The Java side of Project Zomboid's ragdolls, driving [`pz_bullet`] (the
//! physics) and [`pz_anim`] (the pose):
//!
//! * [`script`]: `RagdollScript`, the `ragdoll` script blocks and the
//!   arrays `toBullet` uploads;
//! * [`builder`]: `RagdollBuilder`, the skeleton hierarchy and T-pose the
//!   bodies are built from;
//! * [`controller`]: `RagdollController` and `RagdollStateData`, the
//!   per-character ragdoll: added posed like the animation, simulated, read
//!   back into the `AnimationPlayer`'s ragdoll clip, and watched until it
//!   settles; [`RagdollFrame`] is the [`pz_anim::RagdollHost`] the player
//!   calls into;
//! * [`corpse`]: `IsoDeadBody`'s frozen ragdoll pose
//!   (`diedBoneTransforms`) and the `DeathPose` track `DeadBodyAtlas`
//!   poses the body with.

pub mod builder;
pub mod controller;
pub mod corpse;
pub mod script;
pub mod skeleton_bone;

pub use builder::RagdollBuilder;
pub use controller::{
    RagdollCharacter, RagdollController, RagdollDebugOptions, RagdollEnv, RagdollFrame,
    RagdollGlobals, RagdollHit, RagdollStateData, Vec3,
};
pub use script::RagdollScripts;

/// `RagdollBodyPart`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// `RagdollJoint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RagdollJoint {
    PelvisSpine,
    SpineHead,
    LeftHip,
    LeftKnee,
    RightHip,
    RightKnee,
    LeftShoulder,
    LeftElbow,
    RightShoulder,
    RightElbow,
    Count,
}

impl RagdollJoint {
    /// `RagdollController.getJointAssociatedBone`: the `SkeletonBone`
    /// ordinal (`Dummy01` for `Count`).
    pub fn associated_bone(self) -> usize {
        use skeleton_bone as sb;
        match self {
            RagdollJoint::PelvisSpine => sb::BIP01_PELVIS,
            RagdollJoint::SpineHead => sb::BIP01_NECK,
            RagdollJoint::LeftHip => sb::BIP01_L_THIGH,
            RagdollJoint::LeftKnee => sb::BIP01_L_CALF,
            RagdollJoint::RightHip => sb::BIP01_R_THIGH,
            RagdollJoint::RightKnee => sb::BIP01_R_CALF,
            RagdollJoint::LeftShoulder => sb::BIP01_L_UPPER_ARM,
            RagdollJoint::LeftElbow => sb::BIP01_L_FOREARM,
            RagdollJoint::RightShoulder => sb::BIP01_R_UPPER_ARM,
            RagdollJoint::RightElbow => sb::BIP01_R_FOREARM,
            RagdollJoint::Count => sb::DUMMY01,
        }
    }
}
