//! `FileTask_LoadMesh` / `FileTask_LoadAnimation`: the import flags and
//! `ProcessedAiScene` parameters each file type gets.

use std::collections::HashMap;

use crate::import::{self, Format, Steps};
use crate::processed::{fbx_bone_rotate_modifier, LoadMode, ProcessParams, ProcessedAiScene};
use crate::{AnimationClip, SkinningData};

#[derive(Debug)]
pub enum LoadError {
    Import(import::ImportError),
    Process(crate::processed::ProcessError),
    Io(std::io::Error),
    NotFound(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Import(e) => write!(f, "{e}"),
            LoadError::Process(e) => write!(f, "{e}"),
            LoadError::Io(e) => write!(f, "{e}"),
            LoadError::NotFound(s) => write!(f, "not found: {s}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<import::ImportError> for LoadError {
    fn from(e: import::ImportError) -> Self {
        LoadError::Import(e)
    }
}

impl From<crate::processed::ProcessError> for LoadError {
    fn from(e: crate::processed::ProcessError) -> Self {
        LoadError::Process(e)
    }
}

impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}

/// `ModelMesh.MeshAssetParams` plus the `|meshName` suffix of the path.
#[derive(Clone, Debug, Default)]
pub struct MeshParams<'a> {
    pub is_static: bool,
    /// The part after `|` in the mesh path, if any.
    pub mesh_name: Option<String>,
    /// `animationsMesh.skinningData`.
    pub skinned_to: Option<&'a SkinningData>,
    /// `postProcess` (`+FLAG;-FLAG`), honoured for FBX and glTF only.
    pub post_process: Option<String>,
}

/// `FileTask_LoadMesh.loadX/loadFBX/loadGLTF`.
pub fn load_mesh(data: &[u8], format: Format, p: &MeshParams) -> Result<ProcessedAiScene, LoadError> {
    let mut steps = Steps::mesh_default();
    if format != Format::X {
        if let Some(pp) = &p.post_process {
            steps.apply_overrides(pp);
        }
    }
    let scene = import::import(data, format, steps)?;
    let mut params = ProcessParams {
        mode: if p.is_static { LoadMode::StaticMesh } else { LoadMode::Normal },
        skinned_to: p.skinned_to,
        mesh_name: p.mesh_name.clone(),
        ..Default::default()
    };
    if format == Format::Fbx {
        params.anim_bones_scale_modifier = 0.01;
        params.anim_bones_rotate_modifier = Some(fbx_bone_rotate_modifier());
    }
    Ok(ProcessedAiScene::process(&scene, &params)?)
}

/// `FileTask_LoadAnimation.loadX/loadFBX/loadGLTF`: the clips of an
/// animation file, bound to the animations mesh's skeleton.
pub fn load_animation(
    data: &[u8],
    format: Format,
    skinned_to: Option<&SkinningData>,
) -> Result<HashMap<String, AnimationClip>, LoadError> {
    let scene = import::import(data, format, Steps::animation_default())?;
    let mut params = ProcessParams { mode: LoadMode::AnimationOnly, skinned_to, ..Default::default() };
    if format != Format::X {
        params.anim_bones_scale_modifier = 0.01;
        params.anim_bones_rotate_modifier = Some(fbx_bone_rotate_modifier());
    }
    let p = ProcessedAiScene::process(&scene, &params)?;
    match p.skeleton {
        Some(s) => Ok(s.animation_clips),
        // ProcessedAiScene.applyToAnimation would throw here.
        None => Err(LoadError::Process(crate::processed::ProcessError("animation file has no skinned mesh".into()))),
    }
}
