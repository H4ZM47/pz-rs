//! How names in scripts become files: `FileTask_AbstractLoadModel`'s
//! extension probing, `ModelScript` defaults, `Model`'s texture path,
//! `ModelManager`'s model keys and animation directories
//! (`AnimationsMesh`, `loadAnimsFromDir`, `getAnimName`).
//!
//! The scripts crate is not on this branch, so [`ModelScriptInfo`] and
//! [`AnimationsMeshInfo`] carry just the fields model loading reads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::import::Format;

/// A media directory indexed case-insensitively, the way
/// `ZomboidFileSystem.getString` resolves `media/...` paths.
#[derive(Clone, Debug, Default)]
pub struct MediaIndex {
    /// lowercase path relative to the game root (`media/models_x/...`) to
    /// the real file.
    files: HashMap<String, PathBuf>,
}

impl MediaIndex {
    /// Indexes `<game_root>/media` recursively.
    pub fn scan(game_root: &Path) -> std::io::Result<MediaIndex> {
        let mut idx = MediaIndex::default();
        let media = game_root.join("media");
        let mut stack = vec![media];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d)? {
                let e = e?;
                let p = e.path();
                if e.file_type()?.is_dir() {
                    stack.push(p);
                } else if let Ok(rel) = p.strip_prefix(game_root) {
                    let key = rel.to_string_lossy().replace('\\', "/").to_lowercase();
                    idx.files.insert(key, p);
                }
            }
        }
        Ok(idx)
    }

    pub fn from_files(files: impl IntoIterator<Item = (String, PathBuf)>) -> MediaIndex {
        MediaIndex {
            files: files
                .into_iter()
                .map(|(k, v)| (k.to_lowercase(), v))
                .collect(),
        }
    }

    /// `getString` + `File.exists`.
    pub fn get(&self, rel: &str) -> Option<&PathBuf> {
        self.files.get(&rel.replace('\\', "/").to_lowercase())
    }

    /// Lowercase relative paths under a directory (`media/anims_x/bob/`).
    pub fn list_dir(&self, dir_rel: &str) -> Vec<(&str, &PathBuf)> {
        let mut prefix = dir_rel.to_lowercase();
        if !prefix.ends_with('/') {
            prefix.push('/');
        }
        let mut v: Vec<(&str, &PathBuf)> = self
            .files
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(k, v)| (k.as_str(), v))
            .collect();
        v.sort();
        v
    }
}

/// `ModelFileExtensionType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelFileType {
    X,
    Fbx,
    Gltf,
    Txt,
}

impl ModelFileType {
    pub fn import_format(self) -> Option<Format> {
        match self {
            ModelFileType::X => Some(Format::X),
            ModelFileType::Fbx => Some(Format::Fbx),
            ModelFileType::Gltf => Some(Format::Glb),
            ModelFileType::Txt => None,
        }
    }
}

/// Which file task is probing: meshes look in `media/models_x` and
/// `media/models`, animations in `media/anims_x` and `media/anims`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Mesh,
    Animation,
}

impl AssetKind {
    fn dirs(self) -> (&'static str, &'static str) {
        match self {
            AssetKind::Mesh => ("media/models", "media/models_x"),
            AssetKind::Animation => ("media/anims", "media/anims_x"),
        }
    }
}

/// A mesh asset path split at `|` (`FileTask_LoadMesh.getRawFileName` /
/// `getMeshName`).
pub fn split_mesh_path(path: &str) -> (&str, Option<&str>) {
    match path.find('|') {
        Some(p) => (&path[..p], Some(&path[p + 1..])),
        None => (path, None),
    }
}

/// `FileTask_AbstractLoadModel.checkExtensionType`: the file a raw asset
/// name loads from, and how it is read.
pub fn resolve_model_file(
    media: &MediaIndex,
    raw: &str,
    kind: AssetKind,
) -> Option<(PathBuf, ModelFileType)> {
    let (media_path, media_x_path) = kind.dirs();
    let mut processed = raw.to_lowercase();
    if processed.ends_with(".txt") {
        return media.get(raw).map(|p| (p.clone(), ModelFileType::Txt));
    }
    let force_x = raw.starts_with("x:");
    if force_x {
        processed = raw[2..].to_string();
    }
    if raw.contains("media/") || raw.contains('.') {
        let name = if raw.contains("media/") {
            raw.to_string()
        } else {
            format!("{media_x_path}/{raw}")
        };
        if let Some(p) = media.get(&name) {
            let s = p.to_string_lossy();
            let ty = if s.ends_with(".fbx") {
                ModelFileType::Fbx
            } else if s.ends_with(".glb") {
                ModelFileType::Gltf
            } else {
                ModelFileType::X
            };
            return Some((p.clone(), ty));
        }
    }
    for (ext, ty) in [
        ("fbx", ModelFileType::Fbx),
        ("glb", ModelFileType::Gltf),
        ("x", ModelFileType::X),
    ] {
        if let Some(p) = media.get(&format!("{media_x_path}/{processed}.{ext}")) {
            return Some((p.clone(), ty));
        }
    }
    if force_x {
        return None;
    }
    if !processed.ends_with(".x") {
        let name = if raw.contains("media/") {
            raw.to_string()
        } else {
            format!("{media_path}/{processed}.txt")
        };
        if let Some(p) = media.get(&name) {
            return Some((p.clone(), ModelFileType::Txt));
        }
    }
    None
}

/// The part of `ModelScript` model loading reads.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelScriptInfo {
    pub name: String,
    pub mesh: Option<String>,
    pub texture: Option<String>,
    pub shader: Option<String>,
    /// `static`, default true.
    pub is_static: bool,
    pub animations_mesh: Option<String>,
    pub post_process: Option<String>,
    pub scale: f32,
    pub invert_x: bool,
    /// GL cull face (`1029` back, `1028` front, `0` none, `-1` default).
    pub cull_face: i32,
}

impl Default for ModelScriptInfo {
    fn default() -> Self {
        ModelScriptInfo {
            name: String::new(),
            mesh: None,
            texture: None,
            shader: None,
            is_static: true,
            animations_mesh: None,
            post_process: None,
            scale: 1.0,
            invert_x: false,
            cull_face: -1,
        }
    }
}

fn blank(s: &Option<String>) -> bool {
    s.as_deref().map_or(true, |s| s.trim().is_empty())
}

impl ModelScriptInfo {
    /// `getTextureName`: the mesh name when no texture is set.
    pub fn texture_name(&self) -> Option<&str> {
        if blank(&self.texture) {
            self.mesh.as_deref()
        } else {
            self.texture.as_deref()
        }
    }

    /// `getShaderName`.
    pub fn shader_name(&self) -> &str {
        if blank(&self.shader) {
            "basicEffect"
        } else {
            self.shader.as_deref().unwrap()
        }
    }

    /// `ModelManager.createModelKey` for this script.
    pub fn model_key(&self) -> Option<String> {
        model_key(
            self.mesh.as_deref()?,
            self.texture_name(),
            self.is_static,
            Some(self.shader_name()),
        )
    }
}

/// `ModelManager.createModelKey`: `mesh&shader=...;tex=...;isStatic=true`.
pub fn model_key(
    mesh: &str,
    tex: Option<&str>,
    is_static: bool,
    shader: Option<&str>,
) -> Option<String> {
    let mut k = mesh.to_lowercase().trim().to_string();
    k.push('&');
    let shader = match shader {
        Some(s) if !s.trim().is_empty() => s,
        _ => "basicEffect",
    };
    k.push_str("shader=");
    k.push_str(shader.to_lowercase().trim());
    if let Some(t) = tex.filter(|t| !t.trim().is_empty()) {
        k.push_str(";tex=");
        k.push_str(t.to_lowercase().trim());
    }
    if is_static {
        k.push_str(";isStatic=true");
    }
    Some(k)
}

/// `Model`'s constructor: the texture file a texture name loads.
pub fn texture_path(texture_name: &str) -> String {
    if texture_name.contains("media/") {
        texture_name.to_string()
    } else {
        format!("media/textures/{texture_name}.png")
    }
}

/// `AnimationsMesh`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimationsMeshInfo {
    pub name: String,
    pub mesh_file: String,
    pub keep_mesh_animations: bool,
    pub post_process: Option<String>,
    pub animation_directories: Vec<String>,
    pub animation_prefixes: Vec<String>,
}

/// `ModelManager.checkAnimationMeshPrefixes`.
pub fn check_animation_prefixes(file_name: &str, prefixes: &[String]) -> bool {
    if prefixes.is_empty() {
        return true;
    }
    let lower = file_name.to_lowercase();
    let mut ok = false;
    for p in prefixes {
        if let Some(neg) = p.strip_prefix('!') {
            if lower.starts_with(&neg.to_lowercase()) {
                return false;
            }
        }
        if lower.starts_with(&p.to_lowercase()) {
            ok = true;
        }
    }
    ok
}

/// `ZomboidFileSystem.getAnimName`: lowercase path under `anims_x/`
/// without extension.
pub fn anim_name(rel_to_media: &str) -> String {
    let mut s = rel_to_media.replace('\\', "/").to_lowercase();
    if let Some(d) = s.rfind('.') {
        s.truncate(d);
    }
    for p in ["anims/", "anims_x/"] {
        if let Some(r) = s.strip_prefix(p) {
            return r.to_string();
        }
    }
    s
}

/// `ModelManager.initAnimationMeshes` + `loadAnimsFromDir` for the game's
/// files: the animation asset names (and files) an animations mesh gets,
/// directory by directory, filtered by its prefixes.
pub fn animations_for_mesh(media: &MediaIndex, am: &AnimationsMeshInfo) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for dir in &am.animation_directories {
        for (rel, path) in media.list_dir(&format!("media/anims_x/{dir}")) {
            let real_name = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !check_animation_prefixes(&real_name, &am.animation_prefixes) {
                continue;
            }
            out.push((
                anim_name(rel.strip_prefix("media/").unwrap_or(rel)),
                path.clone(),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> MediaIndex {
        MediaIndex::from_files(
            [
                "media/models_x/skinned/malebody.x",
                "media/models_x/worlditems/apple.fbx",
                "media/models_x/isoobject/door.glb",
                "media/models/vehicles_wheel.txt",
                "media/anims_x/bob/bob_idle.x",
                "media/anims_x/chicken/chk_walk.x",
                "media/anims_x/chicken/chk_chick_walk.x",
            ]
            .iter()
            .map(|s| (s.to_string(), PathBuf::from(format!("/g/{s}")))),
        )
    }

    #[test]
    fn extension_probing() {
        let m = index();
        assert_eq!(
            resolve_model_file(&m, "Skinned/MaleBody", AssetKind::Mesh)
                .unwrap()
                .1,
            ModelFileType::X
        );
        assert_eq!(
            resolve_model_file(&m, "WorldItems/Apple", AssetKind::Mesh)
                .unwrap()
                .1,
            ModelFileType::Fbx
        );
        assert_eq!(
            resolve_model_file(&m, "IsoObject/Door", AssetKind::Mesh)
                .unwrap()
                .1,
            ModelFileType::Gltf
        );
        assert_eq!(
            resolve_model_file(&m, "Vehicles_Wheel", AssetKind::Mesh)
                .unwrap()
                .1,
            ModelFileType::Txt
        );
        assert_eq!(
            resolve_model_file(&m, "bob/bob_idle", AssetKind::Animation)
                .unwrap()
                .1,
            ModelFileType::X
        );
        assert!(resolve_model_file(&m, "missing", AssetKind::Mesh).is_none());
        assert_eq!(
            split_mesh_path("physics/ramp10|Segment1"),
            ("physics/ramp10", Some("Segment1"))
        );
    }

    #[test]
    fn script_defaults_and_keys() {
        let s = ModelScriptInfo {
            name: "Apple".into(),
            mesh: Some("WorldItems/Apple".into()),
            ..Default::default()
        };
        assert_eq!(s.texture_name(), Some("WorldItems/Apple"));
        assert_eq!(s.shader_name(), "basicEffect");
        assert_eq!(
            s.model_key().unwrap(),
            "worlditems/apple&shader=basiceffect;tex=worlditems/apple;isStatic=true"
        );
        assert_eq!(
            texture_path("WorldItems/Apple"),
            "media/textures/WorldItems/Apple.png"
        );
        assert_eq!(texture_path("media/textures/x.png"), "media/textures/x.png");
    }

    #[test]
    fn animation_dirs_and_prefixes() {
        let m = index();
        let am = AnimationsMeshInfo {
            name: "Chicken_Hen".into(),
            mesh_file: "Skinned/Chicken_Hen".into(),
            animation_directories: vec!["Chicken".into()],
            animation_prefixes: vec!["Chk_".into(), "!Chk_Chick_".into()],
            ..Default::default()
        };
        let names: Vec<String> = animations_for_mesh(&m, &am)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, vec!["chicken/chk_walk".to_string()]);
    }
}
