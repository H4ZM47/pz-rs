//! Models, skeletons and animation clips as the model importers produce
//! them (`ModelMesh`, `SkinningData`, `AnimationClip`, `Keyframe`). The
//! loaders (`.X`, `.fbx`, `.txt`) fill these; the animation code samples
//! them into bone transforms; the renderer draws the meshes
//! ([`pz_render::model::MeshData`]).
//!
//! Matrices are column-major `[[f32; 4]; 4]` (column vectors, `M * v`).
//! The original keeps LWJGL/XNA-style row-vector matrices; loaders and the
//! animation code convert at their boundaries and say so.

use std::collections::HashMap;

pub mod import;
pub mod loader;
pub mod processed;

pub use pz_render::model::{MeshData, MeshKind};

/// A column-major 4x4 matrix.
pub type Mat4 = [[f32; 4]; 4];

/// `org.lwjgl.util.vector.Quaternion` (x, y, z, w).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// `Keyframe`: one bone's pose at one time.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub bone: i32,
    pub bone_name: String,
    pub time: f32,
    pub position: [f32; 3],
    pub rotation: Quat,
    pub scale: [f32; 3],
}

/// `AnimationClip`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimationClip {
    pub name: String,
    pub duration: f32,
    pub keep_last_frame: bool,
    pub is_ragdoll: bool,
    /// All keyframes in the order the importer produced them.
    pub keyframes: Vec<Keyframe>,
}

/// `SkinningData`: the skeleton of a skinned model and the clips that came
/// with it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkinningData {
    /// `bindPose`: each bone's local bind transform.
    pub bind_pose: Vec<Mat4>,
    /// `inverseBindPose`.
    pub inverse_bind_pose: Vec<Mat4>,
    /// `boneOffset` (skin offset per bone).
    pub bone_offset: Vec<Mat4>,
    /// `skeletonHierarchy`: each bone's parent index, -1 for roots.
    pub skeleton_hierarchy: Vec<i32>,
    /// `boneIndices`: bone name to index.
    pub bone_indices: HashMap<String, i32>,
    pub animation_clips: HashMap<String, AnimationClip>,
}

impl SkinningData {
    pub fn num_bones(&self) -> usize {
        self.skeleton_hierarchy.len()
    }
}

/// `ModelMesh`: a loaded mesh with its optional skeleton.
#[derive(Debug)]
pub struct ModelMesh {
    pub mesh: std::rc::Rc<MeshData>,
    pub min_xyz: [f32; 3],
    pub max_xyz: [f32; 3],
    /// The mesh's root transform (`ModelMesh.transform`).
    pub transform: Option<Mat4>,
    pub skinning: Option<SkinningData>,
}
