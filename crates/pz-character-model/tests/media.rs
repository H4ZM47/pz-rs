//! [`MediaModelResources`] on the installed game's files (read at runtime:
//! point `PZ_GAME_DIR` at the folder holding `media/`; without it this test
//! does nothing).

use std::path::{Path, PathBuf};

use pz_character_model::render::ModelResources;
use pz_character_model::resources::{MediaModelResources, TextureLoader};
use pz_model::assets::MediaIndex;
use pz_render::model::MeshKind;
use pz_render::Texture;

struct NoTextures;

impl TextureLoader for NoTextures {
    fn file(&self, _: &Path) -> Option<Texture> {
        None
    }
    fn smart(&self, _: usize) -> Option<Texture> {
        None
    }
}

#[test]
fn loads_game_meshes() {
    let Some(game) = std::env::var_os("PZ_GAME_DIR")
        .map(PathBuf::from)
        .filter(|g| g.join("media").is_dir())
    else {
        eprintln!("PZ_GAME_DIR not set or has no media/; skipping");
        return;
    };
    let media = MediaIndex::scan(&game).unwrap();
    let res = MediaModelResources::new(&media, &NoTextures);
    let body = res.mesh("skinned/malebody", false).expect("malebody");
    assert_eq!(body.mesh.kind, MeshKind::Skinned);
    assert!(body.skeleton.is_some() && !body.mesh.indices.is_empty());
    let again = res.mesh("Skinned/MaleBody", false).unwrap();
    assert!(std::rc::Rc::ptr_eq(&body.mesh, &again.mesh));
    assert!(res.mesh("no/such/mesh", true).is_none());
    assert!(res.texture_file("Body/MaleBody01a").is_some());
}
