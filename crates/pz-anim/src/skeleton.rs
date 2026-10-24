//! `SkinningBone` / `SkinningBoneHierarchy`: the bone tree built from a
//! [`pz_model::SkinningData`], plus the skeleton matrices converted to the
//! original's [`Matrix4f`] layout.

use std::collections::HashMap;

use pz_model::SkinningData;

use crate::math::Matrix4f;

/// `SkinningBone`.
#[derive(Clone, Debug, Default)]
pub struct SkinningBone {
    /// `None` for bones missing from `boneIndices` (the original leaves the
    /// name null).
    pub name: Option<String>,
    pub index: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

/// The skeleton side of `SkinningData` the animation code reads.
#[derive(Clone, Debug, Default)]
pub struct Skeleton {
    pub bones: Vec<SkinningBone>,
    pub root_bones: Vec<usize>,
    /// `boneIndices` (exact-case names, like the original's `HashMap`).
    pub bone_indices: HashMap<String, i32>,
    pub bind_pose: Vec<Matrix4f>,
    pub inverse_bind_pose: Vec<Matrix4f>,
    pub bone_offset: Vec<Matrix4f>,
}

impl Skeleton {
    /// `SkinningBoneHierarchy.buildBoneHierarchy`.
    pub fn from_skinning_data(data: &SkinningData) -> Skeleton {
        let n = data.num_bones();
        let mut bones: Vec<SkinningBone> = (0..n).map(|i| SkinningBone { index: i, ..Default::default() }).collect();
        for (name, &idx) in &data.bone_indices {
            if idx >= 0 && (idx as usize) < n {
                bones[idx as usize].name = Some(name.clone());
            }
        }
        let mut root_bones = Vec::new();
        for i in 0..n {
            let parent = data.skeleton_hierarchy[i];
            if parent > -1 && (parent as usize) < n {
                bones[i].parent = Some(parent as usize);
                bones[parent as usize].children.push(i);
            } else {
                root_bones.push(i);
            }
        }
        let conv = |v: &Vec<pz_model::Mat4>| v.iter().map(Matrix4f::from_mat4).collect::<Vec<_>>();
        Skeleton {
            bones,
            root_bones,
            bone_indices: data.bone_indices.clone(),
            bind_pose: conv(&data.bind_pose),
            inverse_bind_pose: conv(&data.inverse_bind_pose),
            bone_offset: conv(&data.bone_offset),
        }
    }

    pub fn num_bones(&self) -> usize {
        self.bones.len()
    }

    /// `SkinningData.getBone(String)`: exact-case lookup through `boneIndices`.
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bone_indices.get(name).map(|&i| i as usize).filter(|&i| i < self.bones.len())
    }

    /// `SkinningBoneHierarchy.getBone(String)`: case-insensitive search.
    pub fn find_bone_ignore_case(&self, name: &str) -> Option<usize> {
        self.bones
            .iter()
            .find(|b| b.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)))
            .map(|b| b.index)
    }

    pub fn parent(&self, bone: usize) -> Option<usize> {
        self.bones.get(bone).and_then(|b| b.parent)
    }

    /// `SkinningBone.forEachDescendant`: all children first, then each
    /// child's descendants.
    pub fn for_each_descendant(&self, bone: usize, f: &mut impl FnMut(usize)) {
        let children = &self.bones[bone].children;
        for &c in children {
            f(c);
        }
        for &c in children {
            self.for_each_descendant(c, f);
        }
    }
}
