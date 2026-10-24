//! Model tree, texture jobs and draw order on synthetic outfits.

use std::collections::HashMap;
use std::rc::Rc;

use pz_anim::AnimationPlayer;
use pz_character_model::data::*;
use pz_character_model::lights::{LightContext, PlayerLightData, ResultLight};
use pz_character_model::mask::{CharacterMask, Part, BLOOD_PARTS};
use pz_character_model::model_tree::*;
use pz_character_model::render::*;
use pz_character_model::texture_creator::*;
use pz_iso_render::models::ModelCamera;
use pz_iso_render::Camera;
use pz_model::SkinningData;
use pz_render::model::{MeshData, MeshKind};
use pz_render::Texture;

fn clothing(name: &str, model: Option<&str>, tex: &str, masks: &[i32]) -> Rc<ClothingItem> {
    let mut xml = format!("<clothingItem><textureChoices>{tex}</textureChoices><m_BaseTextures>{tex}</m_BaseTextures>");
    if let Some(m) = model {
        xml += &format!("<m_MaleModel>{m}</m_MaleModel><m_FemaleModel>{m}</m_FemaleModel>");
    }
    for m in masks {
        xml += &format!("<m_Masks>{m}</m_Masks>");
    }
    xml += "</clothingItem>";
    Rc::new(ClothingItem::from_element(name, &pz_anim::xml::parse(&xml).unwrap()))
}

fn human() -> HumanVisual {
    HumanVisual {
        female: false,
        zombie: false,
        skeleton: false,
        skin_texture: "MaleBody01a".into(),
        hair_model: "CrewCut".into(),
        hair_color: ImmutableColor::new(0.5, 0.3, 0.1),
        beard_model: "Chops".into(),
        beard_color: ImmutableColor::new(0.4, 0.2, 0.1),
        body_visuals: vec![],
        blood: [0.0; BLOOD_PARTS],
        dirt: [0.0; BLOOD_PARTS],
    }
}

fn styles() -> HairStyles {
    let st = |name: &str, model: &str| HairStyle { name: name.into(), model: Some(model.into()), texture: Some("F_Hair_White".into()), alternate: vec![("Group01".into(), "Bald".into())], level: 1 };
    HairStyles {
        male: vec![HairStyle { name: "Bald".into(), model: Some(String::new()), texture: Some("F_Hair_White".into()), alternate: vec![], level: 0 }, st("CrewCut", "Skinned\\Hair\\M_Hair_CrewCut")],
        female: vec![],
        beards: vec![st("Chops", "skinned/beards/bob_beard_chops")],
    }
}

fn build(visuals: &[ItemVisual], eq: &EquippedItems) -> ModelSlot {
    let h = human();
    let hs = styles();
    let scripts: HashMap<String, Rc<ModelScript>> = HashMap::new();
    let groups = BodyLocationGroup::default();
    let bi = |n: &str| match n {
        "Bip01_Prop1" => 32,
        "Bip01_Prop2" => 33,
        _ => -1,
    };
    ModelSlot::build(&CharacterModelInput {
        human: &h,
        item_visuals: visuals,
        equipped: eq,
        body_locations: &groups,
        hair_styles: &hs,
        scripts: &scripts,
        body_model_script: None,
        bone_index: &bi,
    })
}

#[test]
fn mask_parts_and_subdivisions() {
    let mut m = CharacterMask::default();
    m.set_part_visible(Part::Torso, false);
    assert!(!m.is_part_visible(Part::Chest) && !m.is_part_visible(Part::Waist));
    assert!(!m.is_all_visible() && !m.is_nothing_visible());
    assert!(!m.visible_leaves().contains(&Part::Chest));
}

#[test]
fn tree_order_matches_populate() {
    let shirt = clothing("Shirt", Some("Skinned\\Clothes\\Bob_Shirt"), "Clothes\\Shirt", &[1]);
    let pants = clothing("Pants", Some("skinned/clothes/bob_pants"), "Clothes/Pants", &[2]);
    let visuals = vec![ItemVisual::new("Base.Shirt", Some(shirt)), ItemVisual::new("Base.Pants", Some(pants))];
    let eq = EquippedItems { attached: vec![], primary: Some("Base.Hammer".into()), secondary: Some("Base.Hammer".into()) };
    let slot = build(&visuals, &eq);
    let names: Vec<&str> = slot.sub.iter().map(|&i| slot.instance(i).mesh_name.as_str()).collect();
    // Clothing in the visuals' order, then beard, then hair, then the hand
    // item (the same item in both hands is drawn once).
    assert_eq!(names, ["skinned/clothes/bob_shirt", "skinned/clothes/bob_pants", "skinned/beards/bob_beard_chops", "skinned/hair/m_hair_crewcut", "Hammer"]);
    let hair = slot.instance(slot.sub[3]);
    assert_eq!(hair.tint, [0.5, 0.3, 0.1]);
    assert_eq!(hair.parent, Some(slot.model));
    let hammer = slot.instance(slot.sub[4]);
    assert_eq!((hammer.parent_bone, hammer.parent_bone_name.as_deref()), (32, Some("Bip01_Prop1")));
    assert_eq!(slot.instance(slot.sub[0]).tex, TextureSource::File("Clothes/Shirt".into()));
}

#[test]
fn hats_swap_hair_and_hide_beards() {
    let mut hat = (*clothing("Hat", Some("skinned/hat"), "Hat", &[0])).clone();
    hat.hat_category = Some("Group01".into());
    let mut nobeard = (*clothing("Mask", Some("skinned/mask"), "Mask", &[])).clone();
    nobeard.hat_category = Some("nobeard".into());
    let visuals = vec![ItemVisual::new("Base.Mask", Some(Rc::new(nobeard)))];
    let slot = build(&visuals, &EquippedItems::default());
    assert!(!slot.instances.iter().any(|i| i.kind == InstanceKind::Beard));
    assert!(slot.instances.iter().any(|i| i.kind == InstanceKind::Hair));
    // A hat hiding the head mask part removes hair and beard altogether.
    let slot = build(&[ItemVisual::new("Base.Hat", Some(Rc::new(hat)))], &EquippedItems::default());
    assert!(!slot.instances.iter().any(|i| matches!(i.kind, InstanceKind::Hair | InstanceKind::Beard)));
}

#[test]
fn texture_only_clothing_composites_onto_the_body() {
    let tshirt = clothing("TShirt", None, "clothes/tshirt", &[]);
    let jacket = clothing("Jacket", Some("skinned/jacket"), "clothes/jacket", &[1]);
    let mut bloody = ItemVisual::new("Base.Jacket", Some(jacket));
    bloody.blood[6] = 255;
    let visuals = vec![bloody, ItemVisual::new("Base.TShirt", Some(tshirt))];
    let mut slot = build(&visuals, &EquippedItems::default());
    let h = human();
    let groups = BodyLocationGroup::default();
    let out = create_textures(&mut slot, &TextureCreatorInput { human: &h, item_visuals: &visuals, body_locations: &groups, decals: &|_| None, simple_clothing_textures: false });
    // Body skin job, jacket full texture + jacket smart texture, body.
    let body = match &slot.instance(slot.model).tex {
        TextureSource::Smart(i) => *i,
        t => panic!("{t:?}"),
    };
    assert_eq!(body, out.jobs.len() - 1);
    let job = &out.jobs[body];
    // The skin masked to what the jacket leaves visible (every leaf but
    // the torso's two), then the t-shirt underneath it, whole.
    assert_eq!(job.commands.len(), 12 + 1);
    assert!(job.commands[..12].iter().all(|c| c.tex == TexRef::Job(0) && c.shader == SmartShader::BodyMask));
    assert_eq!(job.commands[12].tex, TexRef::File("media/textures/clothes/tshirt.png".into()));
    assert_eq!(job.commands[12].shader, SmartShader::Blit);
    let jacket = slot.sub.iter().map(|&i| slot.instance(i)).find(|i| i.mesh_name == "skinned/jacket").unwrap();
    let TextureSource::Smart(j) = jacket.tex else { panic!() };
    let full = match &out.jobs[j].commands[0].tex {
        TexRef::Job(k) => *k,
        t => panic!("{t:?}"),
    };
    assert!(out.jobs[full].commands.iter().any(|c| c.shader == SmartShader::OverlayMask));
}

struct Res;

impl ModelResources for Res {
    fn mesh(&self, name: &str, is_static: bool) -> Option<LoadedMesh> {
        if name == "missing" {
            return None;
        }
        Some(LoadedMesh {
            mesh: Rc::new(MeshData { id: 1, kind: if is_static { MeshKind::Static } else { MeshKind::Skinned }, vertices: vec![], indices: vec![] }),
            transform: None,
            skeleton: None,
        })
    }

    fn texture(&self, s: &TextureSource) -> Option<Texture> {
        match s {
            TextureSource::None => None,
            _ => None,
        }
    }
}

#[test]
fn draws_follow_tree_order_with_lights() {
    let shirt = clothing("Shirt", Some("skinned/shirt"), "Shirt", &[]);
    let visuals = vec![ItemVisual::new("Base.Shirt", Some(shirt))];
    let mut slot = build(&visuals, &EquippedItems::default());
    slot.instances[slot.model].tex = TextureSource::Smart(0);
    let mut sd = SkinningData { skeleton_hierarchy: vec![-1], bind_pose: vec![[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]], ..Default::default() };
    sd.inverse_bind_pose = sd.bind_pose.clone();
    sd.bone_offset = sd.bind_pose.clone();
    sd.bone_indices.insert("Root".into(), 0);
    let player = AnimationPlayer::new(Rc::new(sd));
    let mut lights = PlayerLightData::default();
    lights.update(&LightContext {
        x: 10.5,
        y: 10.5,
        z: 0.0,
        result_lights: vec![ResultLight { id: 7, x: 12, y: 10, z: 0, radius: 6, r: 1.0, g: 0.5, b: 0.2, flags: 0 }],
        target_ambient: [0.6, 0.6, 0.6],
        outside: true,
        in_room: false,
        room_light_ids: vec![],
        player_index: None,
        torch_strength: 0.0,
        forward: [1.0, 0.0],
        multiplier: 1.0,
    });
    assert_eq!(lights.effect_lights_main[0].radius, 6);
    assert_eq!(lights.effect_lights_main[3].radius, 5000);
    let cam = Camera::centred(10.0, 10.0, 0.0, 1920, 1080, 1.0);
    let mc = ModelCamera::new(&cam, 1920.0, 1080.0);
    let frame = CharacterFrame { player: &player, x: 10.5, y: 10.5, z: 0.0, alpha: 1.0, effect_lights: &lights.effect_lights_main, ambient: lights.current_ambient, final_scale: 1.0 };
    let draws = render_character(&slot, &frame, &mc, &Res);
    // body, shirt, beard, hair
    assert_eq!(draws.len(), 4);
    assert!(draws[0].lights[0].colour[0] > 0.0);
    assert!((draws[0].ambient[0] - 0.27).abs() < 1e-6);
    assert_eq!(draws[0].target_depth, draws[3].target_depth);
}
