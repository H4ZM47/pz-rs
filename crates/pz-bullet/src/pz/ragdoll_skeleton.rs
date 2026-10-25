#![allow(clippy::needless_range_loop)] // bone and body-part loops follow the C++
//! The ragdoll skeleton: `SkeletonBone` (the bone enumeration shared with
//! Java), `Skeleton` (bind pose and hierarchy) and `SkeletonPose` (a posed
//! skeleton with relative and absolute bone transforms), reconstructed from
//! the shipped library (Skeleton.cpp, SkeletonBone.cpp, SkeletonPose.cpp).

use std::sync::Arc as Rc;

use crate::bt::linear_math::*;

/// `SkeletonBone`: the native mirror of Java's
/// `zombie.core.skinnedmodel.model.SkeletonBone`.
pub mod skeleton_bone {
    /// `SkeletonBone::Count()`.
    pub const COUNT: i32 = 35;
    /// The bones the native code refers to by ordinal.
    pub const BIP01: i32 = 1;
    pub const BIP01_PELVIS: i32 = 2;
    pub const TRANSLATION_DATA: i32 = 34;

    /// `SkeletonBone::GetBoneIndex(type)`: the ordinal, or -1 past the last
    /// bone.
    pub fn get_bone_index(t: i32) -> i32 {
        if t > COUNT - 1 {
            -1
        } else {
            t
        }
    }

    /// `SkeletonBone::next(type)`.
    pub fn next(t: u32) -> u32 {
        if t < COUNT as u32 {
            t + 1
        } else {
            COUNT as u32
        }
    }
}

/// The Java methods `SkeletonBone::ValidateAgainstJava` calls back:
/// `SkeletonBone.getBoneName(int)` and `SkeletonBone.getBoneOrdinal(String)`.
pub trait SkeletonBoneCallbacks {
    /// `getBoneName(ordinal)`; `None` when the call threw.
    fn get_bone_name(&mut self, ordinal: i32) -> Option<String>;
    /// `getBoneOrdinal(name)`.
    fn get_bone_ordinal(&mut self, name: &str) -> i32;
}

/// `SkeletonBone::ValidateAgainstJava()`: checks for every native bone that
/// Java maps its name back to the same ordinal. The original only logs the
/// result ("Validate result: Success." / "Failed."), which is returned here.
pub fn validate_against_java(cb: &mut dyn SkeletonBoneCallbacks) -> bool {
    let mut ok = true;
    for t in 0..skeleton_bone::COUNT {
        // GetName returns an empty string when the Java call threw.
        let name = cb.get_bone_name(t).unwrap_or_default();
        let java = cb.get_bone_ordinal(&name);
        ok &= t == java;
    }
    ok
}

/// `Skeleton::Bone`.
#[derive(Clone, Debug, Default)]
pub struct Bone {
    pub parent: i32,
    pub children: Vec<i32>,
}

/// `Skeleton`.
#[derive(Clone, Debug)]
pub struct Skeleton {
    /// `+0x00`: bind pose of each bone, relative to its parent.
    pub bind: Vec<Transform>,
    /// `+0x18`: inverse absolute bind pose of each bone.
    pub inverse_absolute: Vec<Transform>,
    /// `+0x30`.
    pub num_bones: i32,
    /// `+0x38`.
    pub bones: Vec<Bone>,
}

/// `btTransform` with the rotation of `t` re-normalised through a quaternion
/// (`setIdentity(); setRotation(t.getRotation()); setOrigin(t.getOrigin())`).
pub fn rotation_normalized(t: &Transform) -> Transform {
    let q = t.basis.get_rotation();
    let mut basis = Mat3::IDENTITY;
    basis.set_rotation(&q);
    Transform::new(basis, t.origin)
}

impl Skeleton {
    /// `Skeleton(hierarchy, names, bindPose)` / `Skeleton::initialize`.
    pub fn new(hierarchy: &[i32], bind_pose: &[Transform]) -> Skeleton {
        let n = bind_pose.len();
        let mut bones = vec![Bone::default(); n];
        let mut bind = vec![Transform::IDENTITY; n];
        for i in 0..n {
            bones[i].parent = hierarchy[i];
            for j in i + 1..n {
                if hierarchy[j] == i as i32 {
                    bones[i].children.push(j as i32);
                }
            }
            bind[i] = bind_pose[i];
        }
        let mut abs = vec![Transform::IDENTITY; n];
        if n > 0 {
            abs[0] = rotation_normalized(&bind[0]);
        }
        for i in 1..n {
            let p = bones[i].parent as usize;
            abs[i] = abs[p] * bind[i];
        }
        for t in abs.iter_mut() {
            *t = t.inverse();
        }
        Skeleton {
            bind,
            inverse_absolute: abs,
            num_bones: n as i32,
            bones,
        }
    }

    pub fn parent(&self, i: usize) -> i32 {
        self.bones[i].parent
    }
}

/// `SkeletonPose`.
#[derive(Clone, Debug)]
pub struct SkeletonPose {
    pub skeleton: Rc<Skeleton>,
    /// `+0x20`: the pose of each bone, relative to its bind pose.
    pub pose: Vec<Transform>,
    /// `+0x38`: absolute bone transforms (a cache, refreshed on read).
    pub absolute: Vec<Transform>,
    /// `+0x50`: bone transforms relative to the parent bone (a cache).
    pub relative: Vec<Transform>,
}

impl SkeletonPose {
    /// `SkeletonPose(skeleton)`: every transform starts as the identity.
    pub fn new(skeleton: Rc<Skeleton>) -> SkeletonPose {
        let n = skeleton.num_bones.max(0) as usize;
        SkeletonPose {
            skeleton,
            pose: vec![Transform::IDENTITY; n],
            absolute: vec![Transform::IDENTITY; n],
            relative: vec![Transform::IDENTITY; n],
        }
    }

    pub fn num_bones(&self) -> usize {
        self.skeleton.num_bones.max(0) as usize
    }

    /// `updateBonePoseRelative(i)`.
    pub fn update_bone_pose_relative(&mut self, i: usize) {
        self.relative[i] = self.skeleton.bind[i] * self.pose[i];
    }

    /// `updateBonePoseAbsolute(i)` (recursive over the parents).
    pub fn update_bone_pose_absolute(&mut self, i: usize) {
        self.update_bone_pose_relative(i);
        let p = self.skeleton.parent(i);
        if p >= 0 {
            let p = p as usize;
            self.update_bone_pose_absolute(p);
            self.absolute[i] = self.absolute[p] * self.relative[i];
        } else {
            self.absolute[i] = rotation_normalized(&self.relative[i]);
        }
    }

    /// `getBonePoseAbsolute(i)`.
    pub fn get_bone_pose_absolute(&mut self, i: usize) -> Transform {
        self.update_bone_pose_absolute(i);
        self.absolute[i]
    }

    /// `getBonePoseRelative(i)`.
    pub fn get_bone_pose_relative(&mut self, i: usize) -> Transform {
        self.update_bone_pose_relative(i);
        self.relative[i]
    }

    /// `update()`: all relative transforms, then the absolute ones in bone
    /// order (bone 0 is the root).
    pub fn update(&mut self) {
        let n = self.num_bones();
        for i in 0..n {
            self.relative[i] = self.skeleton.bind[i] * self.pose[i];
        }
        if n > 0 {
            self.absolute[0] = rotation_normalized(&self.relative[0]);
        }
        for i in 1..n {
            let p = self.skeleton.parent(i) as usize;
            self.absolute[i] = self.absolute[p] * self.relative[i];
        }
    }

    /// `setBonePoseAbsolute(i, t)`: sets the pose of bone `i` so that its
    /// absolute transform becomes `t` (with the rotation re-normalised).
    pub fn set_bone_pose_absolute(&mut self, i: usize, t: &Transform) {
        let bind = self.skeleton.bind[i];
        let p = self.skeleton.parent(i);
        let parent = if p < 0 {
            Transform::IDENTITY
        } else {
            self.update_bone_pose_absolute(p as usize);
            self.absolute[p as usize]
        };
        let x = bind.inverse() * parent.inverse() * *t;
        let q = x.basis.get_rotation().normalized();
        let mut b = Mat3::IDENTITY;
        b.set_rotation(&q);
        let q2 = b.get_rotation();
        let mut basis = Mat3::IDENTITY;
        basis.set_rotation(&q2);
        self.pose[i] = Transform::new(basis, x.origin);
    }

    /// `setSkeletonPose(transforms)`: `setBonePoseAbsolute` for every bone.
    pub fn set_skeleton_pose(&mut self, transforms: &[Transform]) {
        for i in 0..self.num_bones() {
            self.set_bone_pose_absolute(i, &transforms[i]);
        }
    }

    /// `resetBoneTransforms()`.
    pub fn reset_bone_transforms(&mut self) {
        for t in self.pose.iter_mut() {
            *t = Transform::IDENTITY;
        }
    }
}
