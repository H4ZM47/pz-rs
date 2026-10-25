//! `SkeletonBone` (the bone enumeration the ragdoll natives share with
//! Java) and `SkinningBoneHierarchy.getBone(SkeletonBone)`.

use pz_anim::skeleton::Skeleton;

/// `SkeletonBone.all()`: the 35 bones, by ordinal.
pub const SKELETON_BONES: [&str; 35] = [
    "Dummy01",
    "Bip01",
    "Bip01_Pelvis",
    "Bip01_Spine",
    "Bip01_Spine1",
    "Bip01_Neck",
    "Bip01_Head",
    "Bip01_L_Clavicle",
    "Bip01_L_UpperArm",
    "Bip01_L_Forearm",
    "Bip01_L_Hand",
    "Bip01_L_Finger0",
    "Bip01_L_Finger1",
    "Bip01_R_Clavicle",
    "Bip01_R_UpperArm",
    "Bip01_R_Forearm",
    "Bip01_R_Hand",
    "Bip01_R_Finger0",
    "Bip01_R_Finger1",
    "Bip01_BackPack",
    "Bip01_L_Thigh",
    "Bip01_L_Calf",
    "Bip01_L_Foot",
    "Bip01_L_Toe0",
    "Bip01_R_Thigh",
    "Bip01_R_Calf",
    "Bip01_R_Foot",
    "Bip01_R_Toe0",
    "Bip01_DressFront",
    "Bip01_DressFront02",
    "Bip01_DressBack",
    "Bip01_DressBack02",
    "Bip01_Prop1",
    "Bip01_Prop2",
    "Translation_Data",
];

/// `SkeletonBone.count()`.
pub const COUNT: usize = SKELETON_BONES.len();

pub const DUMMY01: usize = 0;
pub const BIP01: usize = 1;
pub const BIP01_PELVIS: usize = 2;
pub const BIP01_NECK: usize = 5;
pub const BIP01_HEAD: usize = 6;
pub const BIP01_L_UPPER_ARM: usize = 8;
pub const BIP01_L_FOREARM: usize = 9;
pub const BIP01_R_UPPER_ARM: usize = 14;
pub const BIP01_R_FOREARM: usize = 15;
pub const BIP01_L_THIGH: usize = 20;
pub const BIP01_L_CALF: usize = 21;
pub const BIP01_R_THIGH: usize = 24;
pub const BIP01_R_CALF: usize = 25;

/// The `SkeletonBone` a skinning bone's name resolves to
/// (`StringUtils.tryParseEnum`, exact case); `None` for names that are not
/// one of the 35 bones (`SkeletonBone.None`, whose `index()` is -1).
pub fn from_name(name: &str) -> Option<usize> {
    SKELETON_BONES.iter().position(|&b| b == name)
}

/// `SkinningBoneHierarchy.getBone(skeletonBone)`: the first skinning bone
/// (by index) whose `skeletonBone` is `sb`.
pub fn find_bone(skeleton: &Skeleton, sb: usize) -> Option<usize> {
    skeleton
        .bones
        .iter()
        .position(|b| b.name.as_deref().and_then(from_name) == Some(sb))
}

/// `getBone` for every `SkeletonBone`, by ordinal.
pub fn bone_map(skeleton: &Skeleton) -> [Option<usize>; COUNT] {
    std::array::from_fn(|sb| find_bone(skeleton, sb))
}

/// `SkinningBone.getParentSkeletonBone().index()`: the ordinal of the
/// parent's `SkeletonBone`, or -1 (no parent, or a parent that is not one
/// of the 35).
pub fn parent_skeleton_bone_index(skeleton: &Skeleton, bone: usize) -> i32 {
    skeleton
        .parent(bone)
        .and_then(|p| skeleton.bones[p].name.as_deref().and_then(from_name))
        .map(|i| i as i32)
        .unwrap_or(-1)
}
