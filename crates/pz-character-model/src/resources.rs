//! [`ModelResources`] over the game's media files through pz-model's
//! loaders: `ModelManager`'s mesh assets (`FileTask_AbstractLoadModel`
//! extension probing, then `FileTask_LoadMesh`) and `Model`'s texture
//! paths. Textures themselves come from the caller (they need the
//! renderer's texture manager).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_anim::skeleton::Skeleton;
use pz_model::assets::{
    resolve_model_file, split_mesh_path, texture_path, AssetKind, MediaIndex, ModelFileType,
};
use pz_model::loader::{load_mesh, MeshParams};
use pz_render::Texture;

use crate::model_tree::TextureSource;
use crate::render::{LoadedMesh, ModelResources};

/// Loads the textures an instance shows: a media file, or a smart
/// texture job's result.
pub trait TextureLoader {
    fn file(&self, path: &Path) -> Option<Texture>;
    fn smart(&self, job: usize) -> Option<Texture>;
}

/// Meshes from a [`MediaIndex`], loaded once per (name, static) and cached
/// (a failed load stays `None`, like an asset that failed).
pub struct MediaModelResources<'a> {
    pub media: &'a MediaIndex,
    pub textures: &'a dyn TextureLoader,
    meshes: RefCell<HashMap<(String, bool), Option<LoadedMesh>>>,
    next_id: Cell<u64>,
}

impl<'a> MediaModelResources<'a> {
    pub fn new(media: &'a MediaIndex, textures: &'a dyn TextureLoader) -> Self {
        MediaModelResources {
            media,
            textures,
            meshes: RefCell::new(HashMap::new()),
            next_id: Cell::new(1),
        }
    }

    /// `Model`'s texture file for a texture name, if the game has it.
    pub fn texture_file(&self, name: &str) -> Option<PathBuf> {
        self.media.get(&texture_path(name)).cloned()
    }

    /// The file a mesh name loads from and how it is read.
    pub fn mesh_file(&self, mesh_name: &str) -> Option<(PathBuf, ModelFileType)> {
        let (raw, _) = split_mesh_path(mesh_name);
        resolve_model_file(self.media, raw, AssetKind::Mesh)
    }

    fn load(&self, mesh_name: &str, is_static: bool) -> Option<LoadedMesh> {
        let (raw, sub) = split_mesh_path(mesh_name);
        let (path, ty) = resolve_model_file(self.media, raw, AssetKind::Mesh)?;
        // TODO(model-load): legacy `.txt` meshes (pz_model::txt) are not
        // turned into render meshes yet.
        let format = ty.import_format()?;
        let data = std::fs::read(path).ok()?;
        let params = MeshParams {
            is_static,
            mesh_name: sub.map(str::to_string),
            ..Default::default()
        };
        let scene = load_mesh(&data, format, &params).ok()?;
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let mm = scene.into_model_mesh(id)?;
        let skeleton = mm
            .skinning
            .as_ref()
            .map(|s| Rc::new(Skeleton::from_skinning_data(s)));
        Some(LoadedMesh {
            mesh: mm.mesh,
            transform: mm.transform,
            skeleton,
        })
    }
}

impl ModelResources for MediaModelResources<'_> {
    fn mesh(&self, mesh_name: &str, is_static: bool) -> Option<LoadedMesh> {
        let key = (mesh_name.to_lowercase(), is_static);
        if let Some(m) = self.meshes.borrow().get(&key) {
            return m.clone();
        }
        let m = self.load(mesh_name, is_static);
        self.meshes.borrow_mut().insert(key, m.clone());
        m
    }

    fn texture(&self, source: &TextureSource) -> Option<Texture> {
        match source {
            TextureSource::None => None,
            TextureSource::File(name) => self.textures.file(&self.texture_file(name)?),
            TextureSource::Smart(job) => self.textures.smart(*job),
        }
    }
}
