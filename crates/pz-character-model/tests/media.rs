//! [`MediaModelResources`] on the installed game's files (read at runtime:
//! point `PZ_GAME_DIR` at the folder holding `media/`; without it this test
//! does nothing).

use std::path::{Path, PathBuf};

use std::collections::HashMap;
use std::rc::Rc;

use pz_character_model::data::{
    BodyLocationGroup, ClothingItem, HumanModel, HumanVisual, ModelScriptRef, Population,
    ResolvedItemVisual,
};
use pz_character_model::items::EquippedModels;
use pz_character_model::model_tree::{
    process_model_file_name, CharacterModelInput, InstanceKind, ModelInstance, ModelSlot,
};
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

fn game_dir() -> Option<PathBuf> {
    let game = std::env::var_os("PZ_GAME_DIR")
        .map(PathBuf::from)
        .filter(|g| g.join("media").is_dir());
    if game.is_none() {
        eprintln!("PZ_GAME_DIR not set or has no media/; skipping");
    }
    game
}

#[test]
fn loads_game_meshes() {
    let Some(game) = game_dir() else { return };
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

/// The hair, beard (and hat alternate) instances `populateCharacterModelSlot`
/// adds for `hair`/`beard` under an optional hat of `hat_category`.
fn head_models(
    pop: &Population,
    hair: &str,
    beard: &str,
    hat_category: Option<&str>,
) -> Vec<ModelInstance> {
    let mut hv = HumanVisual::new(pz_character_model::mask::BLOOD_PARTS);
    hv.skin_texture_name = Some("MaleBody01a".into());
    hv.hair_model = Some(hair.into());
    hv.beard_model = Some(beard.into());
    let human = HumanModel::resolve(&mut hv, false, false, false, &pop.styles(&[], 0.0), &[]);
    let visuals: Vec<ResolvedItemVisual> = hat_category
        .map(|cat| {
            let mut hat = ClothingItem::parse(
                "Hat",
                "",
                "<clothingItem><m_MaleModel>skinned/hat</m_MaleModel></clothingItem>",
            )
            .unwrap();
            hat.hat_category = Some(cat.into());
            ResolvedItemVisual::new("Base.Hat", Some(Rc::new(hat)))
        })
        .into_iter()
        .collect();
    let groups = BodyLocationGroup::new("Human").unwrap();
    let scripts: HashMap<String, ModelScriptRef> = HashMap::new();
    let slot = ModelSlot::build(&CharacterModelInput {
        human: &human,
        item_visuals: &visuals,
        equipped: &EquippedModels::default(),
        body_locations: &groups,
        hair_styles: pop.hair_styles.as_ref(),
        beard_styles: pop.beard_styles.as_ref(),
        scripts: &scripts,
        body_model_script: None,
        bone_index: &|_| -1,
    });
    slot.instances
        .into_iter()
        .filter(|i| matches!(i.kind, InstanceKind::Hair | InstanceKind::Beard))
        .collect()
}

#[test]
fn assembles_game_hair_and_decals() {
    let Some(game) = game_dir() else { return };
    let mut fs = pz_fs::GameFs::new(&game);
    fs.set_cache_dir(std::env::temp_dir().join("pz-character-model-media-test"));
    let mut pop = Population::default();
    pop.init(&mut fs);
    let hair = pop.hair_styles.as_ref().expect("hairStyles.xml");
    let media = MediaIndex::scan(&game).unwrap();
    let res = MediaModelResources::new(&media, &NoTextures);

    // Every valid hair and beard style's model and texture load.
    let beards = pop.beard_styles.as_ref().expect("beardStyles.xml");
    let styles = hair
        .male
        .iter()
        .chain(&hair.female)
        .map(|s| (&s.model, &s.texture));
    let beard_styles = beards.styles.iter().map(|s| (&s.model, &s.texture));
    let mut n = 0;
    for (model, texture) in styles.chain(beard_styles) {
        let Some(model) = model.as_deref().filter(|m| !m.trim().is_empty()) else {
            continue;
        };
        let mesh = process_model_file_name(model);
        let m = res.mesh(&mesh, false).unwrap_or_else(|| panic!("{mesh}"));
        assert_eq!(m.mesh.kind, MeshKind::Skinned, "{mesh}");
        assert!(res.texture_file(texture).is_some(), "{texture}");
        n += 1;
    }
    assert!(n > 40, "{n} hair/beard models");

    // CrewCut + Chops: one hair and one beard instance with the style's model.
    let crew = hair.find_style(false, Some("CrewCut")).unwrap();
    let got = head_models(&pop, "CrewCut", "Chops", None);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].kind, InstanceKind::Hair);
    assert_eq!(
        got[0].mesh_name,
        process_model_file_name(crew.model.as_deref().unwrap())
    );
    assert_eq!(got[0].model_texture.as_deref(), Some(crew.texture.as_str()));
    assert_eq!(got[1].kind, InstanceKind::Beard);
    assert_eq!(got[1].mesh_name, "skinned/beards/bob_beard_chops");
    assert!(res.mesh(&got[0].mesh_name, false).is_some());

    // Under a hat the hair swaps to its alternate for the hat's category
    // (`getAlternateForHat`); `nohairnobeard` drops both.
    let picard = hair.find_style(false, Some("Picard")).unwrap();
    let alt = picard.get_alternate("default").unwrap();
    let alt_style = hair.find_style(false, Some(alt)).unwrap();
    assert_ne!(alt_style.model, picard.model);
    let got = head_models(&pop, "Picard", "", Some("default"));
    let hair_inst = got.iter().find(|i| i.kind == InstanceKind::Hair).unwrap();
    assert_eq!(
        hair_inst.mesh_name,
        process_model_file_name(alt_style.model.as_deref().unwrap())
    );
    assert!(head_models(&pop, "Picard", "Chops", Some("nohairnobeard")).is_empty());

    // Decals: a group's random pick loads with a texture that exists.
    let decals = pop.clothing_decals.as_ref().expect("clothingDecals.xml");
    let name = decals.get_random_decal("TShirtSpiffo").expect("decal");
    let d = decals.get_decal(&fs, &name).expect("decal file");
    assert!(d.is_valid());
    assert!(res.texture_file(d.texture.as_deref().unwrap()).is_some());
}
