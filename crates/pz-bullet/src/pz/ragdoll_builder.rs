//! `RagdollBuilder`: the global ragdoll template (skeleton hierarchy, bind
//! pose, default skeleton, body-part shapes and the uploaded script),
//! reconstructed from the shipped library (RagdollBuilder.cpp).

use std::sync::Arc as Rc;

use super::ragdoll_script::RagdollScript;
use super::ragdoll_skeleton::Skeleton;
use crate::bt::linear_math::*;
use crate::bt::shapes::Shape;

/// The bone pair of each body part (`createBonePairs`): the body is centred
/// between the two bones and points from the second to the first.
pub const BONE_PAIRS: [[i32; 2]; 11] = [
    [4, 5],
    [5, 6],
    [5, 6],
    [20, 21],
    [21, 22],
    [24, 25],
    [25, 26],
    [8, 9],
    [9, 10],
    [14, 15],
    [15, 16],
];

/// `BoneInfo` (0x110 bytes): a bone of the template skeleton.
#[derive(Clone, Copy, Debug)]
pub struct BoneInfo {
    pub index: i32,
    /// `+0x08`: absolute transform.
    pub absolute: Transform,
    /// `+0x88`: transform relative to the parent.
    pub local: Transform,
    /// `+0x108`: the parent bone (`None` for -1).
    pub parent: Option<usize>,
}

/// One body-part collision shape: a compound with a single child (the
/// capsule, box or sphere) offset by the body part's `offset`.
#[derive(Clone, Debug)]
pub struct BodyPartShape {
    pub child: Shape,
    pub child_transform: Transform,
}

/// `RagdollBuilder::instance`.
#[derive(Clone, Debug)]
pub struct RagdollBuilder {
    /// `+0x00`.
    pub script: RagdollScript,
    /// `+0x08`: the ragdoll's local rotation (`initializeRagdollPose`).
    pub local_transform: Transform,
    /// `+0x88`: parent of each bone (`initializeRagdollSkeleton`).
    pub hierarchy: Vec<i32>,
    /// `+0xa0`: set by `initialize()` (called from `initWorld`).
    pub bone_pairs: Vec<[i32; 2]>,
    /// `+0xb8`: created once by `setupDefaulSkeleton`.
    pub bone_infos: Vec<BoneInfo>,
    /// `+0xd0`: replaced by every `initializeRagdollPose`; ragdolls keep the
    /// skeleton they were created with.
    pub default_skeleton: Option<Rc<Skeleton>>,
    /// `+0xd8`: the 11 body-part shapes, rebuilt by `updateBodyPartShapes`.
    pub body_part_shapes: Vec<Option<BodyPartShape>>,
    /// Bumped whenever the body-part shapes are rebuilt (the original
    /// deletes the old shapes; worlds keep a copy per generation).
    pub shape_generation: u64,
    /// `+0xf0`: total ragdoll mass (`setRagdollMass`).
    pub mass: f64,
    /// `+0xf8`, `+0x100`: unused by the ragdoll code.
    pub friction: f64,
    pub rolling_friction: f64,
    /// The `__once` flags of `simulateRagdoll`,
    /// `simulateRagdollWithRigidBodyOutput` and
    /// `updateSkeletonFromNetworkPhysics` (process-wide statics): until a
    /// native has run once, its error paths log, and the log call clears the
    /// Java exception just thrown, so Java sees -1 instead.
    pub jni_once: [bool; 3],
}

impl Default for RagdollBuilder {
    fn default() -> Self {
        RagdollBuilder::new()
    }
}

impl RagdollBuilder {
    /// `RagdollBuilder()`.
    pub fn new() -> RagdollBuilder {
        RagdollBuilder {
            script: RagdollScript::new(),
            local_transform: Transform::IDENTITY,
            hierarchy: Vec::new(),
            bone_pairs: Vec::new(),
            bone_infos: Vec::new(),
            default_skeleton: None,
            body_part_shapes: Vec::new(),
            shape_generation: 0,
            mass: 70.0,
            friction: 1.5,
            rolling_friction: 0.5,
            jni_once: [false; 3],
        }
    }

    /// `initialize()` / `createBonePairs()` (only the first call does
    /// anything).
    pub fn initialize(&mut self) {
        if self.bone_pairs.is_empty() {
            self.bone_pairs = BONE_PAIRS.to_vec();
        }
    }

    /// `initializeSkeletonHiearachy(n, hierarchy)`.
    pub fn initialize_skeleton_hierarchy(&mut self, n: i32, hierarchy: &[i32]) {
        let n = n.max(0) as usize;
        self.hierarchy.resize(n, 0);
        self.hierarchy[..n].copy_from_slice(&hierarchy[..n]);
    }

    /// `setupDefaulSkeleton(n)`: the bone infos, once.
    fn setup_default_skeleton(&mut self, n: usize) {
        if !self.bone_infos.is_empty() {
            return;
        }
        for i in 0..n {
            let p = self.hierarchy[i];
            self.bone_infos.push(BoneInfo {
                index: i as i32,
                absolute: Transform::IDENTITY,
                local: Transform::IDENTITY,
                parent: if p != -1 { Some(p as usize) } else { None },
            });
        }
    }

    /// `initializeSkeleton(n, boneTransforms, qx, qy, qz, qw)`
    /// (`initializeRagdollPose`): the bind pose from 7 floats per bone
    /// (position, rotation quaternion), bone 0 replaced by the local
    /// rotation; then a new default skeleton and new body-part shapes.
    pub fn initialize_skeleton(&mut self, n: i32, p: &[f32], qx: f32, qy: f32, qz: f32, qw: f32) {
        let q = Quat::new(qx as f64, qy as f64, qz as f64, qw as f64);
        let mut basis = Mat3::IDENTITY;
        basis.set_rotation(&q);
        self.local_transform = Transform::new(basis, Vec3::ZERO);
        let nu = n.max(0) as usize;
        self.setup_default_skeleton(nu);
        for i in 0..nu {
            let r = &p[i * 7..i * 7 + 7];
            let b = &mut self.bone_infos[i];
            b.local.origin = Vec3::new(r[0] as f64, r[1] as f64, r[2] as f64);
            b.local.basis.set_rotation(&Quat::new(
                r[3] as f64,
                r[4] as f64,
                r[5] as f64,
                r[6] as f64,
            ));
        }
        let lt = self.local_transform;
        let b0 = &mut self.bone_infos[0];
        b0.absolute = Transform::IDENTITY;
        b0.absolute.basis = lt.basis;
        b0.local = b0.absolute;
        for i in 1..nu {
            let parent = self.bone_infos[i].parent.expect("bone without parent");
            let abs = self.bone_infos[parent].absolute * self.bone_infos[i].local;
            self.bone_infos[i].absolute = abs;
        }
        self.create_default_skeleton(nu);
        self.update_body_part_shapes();
    }

    /// `createDefaultSkeleton(n)`.
    fn create_default_skeleton(&mut self, n: usize) {
        let bind: Vec<Transform> = self.bone_infos[..n].iter().map(|b| b.local).collect();
        self.default_skeleton = Some(Rc::new(Skeleton::new(&self.hierarchy, &bind)));
    }

    /// `getBodyPartLength(part)`: the distance between the part's bones in
    /// the template.
    pub fn get_body_part_length(&self, part: usize) -> f64 {
        let pair = self.bone_pairs[part];
        let a = self.bone_infos[pair[0] as usize].absolute.origin;
        let b = self.bone_infos[pair[1] as usize].absolute.origin;
        (a - b).length()
    }

    /// `createBodyPartShape(part)`.
    fn create_body_part_shape(&self, part: i32) -> BodyPartShape {
        let info = *self
            .script
            .get_body_part_info(part)
            .expect("body part info not defined");
        let height = if info.calculate_length {
            self.get_body_part_length(info.part as usize) - info.gap as f64
        } else {
            info.height as f64
        };
        let child = match info.shape {
            0 => Shape::new_capsule(info.radius as f64, height, 1),
            2 => Shape::new_sphere(info.radius as f64),
            1 => {
                let r = info.radius as f64;
                Shape::new_box(&Vec3::new(r, r, r))
            }
            // The original adds a null child (and crashes when it is used).
            s => panic!("unknown body part shape {s}"),
        };
        BodyPartShape {
            child,
            child_transform: Transform::new(Mat3::IDENTITY, info.offset),
        }
    }

    /// `updateBodyPartShapes()`.
    pub fn update_body_part_shapes(&mut self) {
        let shapes: Vec<Option<BodyPartShape>> = (0..11)
            .map(|i| Some(self.create_body_part_shape(i)))
            .collect();
        self.body_part_shapes = shapes;
        self.shape_generation += 1;
    }

    /// `getBodyPartMass(part)`: the ragdoll mass times the part's share.
    pub fn get_body_part_mass(&self, part: i32) -> f64 {
        let info = self
            .script
            .get_body_part_info(part)
            .expect("body part info not defined");
        self.mass * info.mass
    }
}
