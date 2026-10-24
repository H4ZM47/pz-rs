//! Model tree, texture jobs and draw order on synthetic outfits.

use std::collections::HashMap;
use std::rc::Rc;

use pz_anim::AnimationPlayer;
use std::cell::RefCell;

use pz_character_model::data::*;
use pz_character_model::items::*;
use pz_character_model::lights::{LightContext, PlayerLightData, ResultLight};
use pz_character_model::mask::{CharacterMask, Part};
use pz_character_model::model_tree::*;
use pz_character_model::render::*;
use pz_character_model::texture_creator::*;
use pz_inventory::lua_table::{LuaKey, LuaTable, LuaValue};
use pz_inventory::{InventoryItem, ItemClass, ItemKind, ItemTemplate};
use pz_iso_render::models::ModelCamera;
use pz_iso_render::Camera;
use pz_model::SkinningData;
use pz_render::model::{MeshData, MeshKind};
use pz_render::Texture;
use pz_scripts::objects::items::{Item, ItemReplacement, ModelWeaponPart};

fn clothing(name: &str, model: Option<&str>, tex: &str, masks: &[i32]) -> Rc<ClothingItem> {
    let mut xml = format!("<clothingItem><textureChoices>{tex}</textureChoices><m_BaseTextures>{tex}</m_BaseTextures>");
    if let Some(m) = model {
        xml += &format!("<m_MaleModel>{m}</m_MaleModel><m_FemaleModel>{m}</m_FemaleModel>");
    }
    for m in masks {
        xml += &format!("<m_Masks>{m}</m_Masks>");
    }
    xml += "</clothingItem>";
    Rc::new(ClothingItem::parse(name, "", &xml).unwrap())
}

fn item(class: ItemClass, name: &str, edit: impl FnOnce(&mut ItemTemplate)) -> InventoryItem {
    let mut t = ItemTemplate::new(class, "Base", name);
    edit(&mut t);
    InventoryItem::create(Rc::new(t), 0)
}

fn weapon(name: &str, sprite: Option<&str>) -> InventoryItem {
    item(ItemClass::Weapon, name, |t| {
        t.weapon_sprite = sprite.map(Into::into)
    })
}

/// A plain item and its script with `staticModel`.
fn plain(name: &str, static_model: Option<&str>) -> (InventoryItem, ItemScriptRef) {
    let mut s = Item::new();
    s.static_model = static_model.map(Into::into);
    (
        item(ItemClass::Normal, name, |_| {}),
        Rc::new(RefCell::new(s)),
    )
}

fn model_script(name: &str, mesh: &str) -> ModelScriptRef {
    let mut m = ModelScript::new();
    m.name = Some(name.into());
    m.mesh_name = Some(mesh.into());
    m.texture_name = Some(format!("{mesh}_tex"));
    Rc::new(RefCell::new(m))
}

fn mwp(part: &str, model: &str, att: &str) -> Rc<RefCell<ModelWeaponPart>> {
    Rc::new(RefCell::new(ModelWeaponPart {
        part_type: part.into(),
        model_name: model.into(),
        attachment_name_self: Some(att.into()),
        attachment_parent: Some(att.into()),
    }))
}

fn human_visual() -> HumanVisual {
    let mut v = HumanVisual::new(pz_character_model::mask::BLOOD_PARTS);
    v.skin_texture_name = Some("MaleBody01a".into());
    v.hair_model = Some("CrewCut".into());
    v.hair_color = Some([0.5, 0.3, 0.1]);
    v.beard_model = Some("Chops".into());
    v.beard_color = Some([0.4, 0.2, 0.1]);
    v
}

fn human<'a>(v: &'a mut HumanVisual, hs: &HairStyles) -> HumanModel<'a> {
    let skins = PopTemplateSkins::default();
    let styles = CharacterStyles {
        hair: hs,
        skins: &skins,
    };
    HumanModel::resolve(v, false, false, false, &styles, &[])
}

fn iv(full_type: &str, c: Rc<ClothingItem>) -> ResolvedItemVisual {
    ResolvedItemVisual::new(full_type, Some(c))
}

fn styles() -> HairStyles {
    let st = |name: &str, model: &str| HairStyle {
        name: name.into(),
        model: Some(model.into()),
        texture: Some("F_Hair_White".into()),
        alternate: vec![("Group01".into(), "Bald".into())],
        level: 1,
    };
    HairStyles {
        male: vec![
            HairStyle {
                name: "Bald".into(),
                model: Some(String::new()),
                texture: Some("F_Hair_White".into()),
                alternate: vec![],
                level: 0,
            },
            st("CrewCut", "Skinned\\Hair\\M_Hair_CrewCut"),
        ],
        female: vec![],
        beards: vec![st("Chops", "skinned/beards/bob_beard_chops")],
    }
}

fn build(visuals: &[ResolvedItemVisual], eq: &EquippedModels) -> ModelSlot {
    build_with(visuals, eq, &HashMap::new())
}

fn build_with(
    visuals: &[ResolvedItemVisual],
    eq: &EquippedModels,
    scripts: &HashMap<String, ModelScriptRef>,
) -> ModelSlot {
    let hs = styles();
    let mut hv = human_visual();
    let h = human(&mut hv, &hs);
    let groups = BodyLocationGroup::new("Human").unwrap();
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
        scripts,
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
    let shirt = clothing(
        "Shirt",
        Some("Skinned\\Clothes\\Bob_Shirt"),
        "Clothes\\Shirt",
        &[1],
    );
    let pants = clothing(
        "Pants",
        Some("skinned/clothes/bob_pants"),
        "Clothes/Pants",
        &[2],
    );
    let visuals = vec![iv("Base.Shirt", shirt), iv("Base.Pants", pants)];
    let hammer = weapon("Hammer", Some("Hammer"));
    let eq = EquippedModels {
        primary: Some(HeldItem::new(&hammer, None)),
        secondary: Some(HeldItem::new(&hammer, None)),
        ..Default::default()
    };
    let slot = build(&visuals, &eq);
    let names: Vec<&str> = slot
        .sub
        .iter()
        .map(|&i| slot.instance(i).mesh_name.as_str())
        .collect();
    // Clothing in the visuals' order, then beard, then hair, then the hand
    // item (the same item in both hands is drawn once).
    assert_eq!(
        names,
        [
            "skinned/clothes/bob_shirt",
            "skinned/clothes/bob_pants",
            "skinned/beards/bob_beard_chops",
            "skinned/hair/m_hair_crewcut",
            "Hammer"
        ]
    );
    let hair = slot.instance(slot.sub[3]);
    assert_eq!(hair.tint, [0.5, 0.3, 0.1]);
    assert_eq!(hair.parent, Some(slot.model));
    let hammer = slot.instance(slot.sub[4]);
    assert_eq!(
        (hammer.parent_bone, hammer.parent_bone_name.as_deref()),
        (32, Some("Bip01_Prop1"))
    );
    assert_eq!(
        slot.instance(slot.sub[0]).tex,
        TextureSource::File("Clothes/Shirt".into())
    );
}

#[test]
fn hats_swap_hair_and_hide_beards() {
    let mut hat = (*clothing("Hat", Some("skinned/hat"), "Hat", &[0])).clone();
    hat.hat_category = Some("Group01".into());
    let mut nobeard = (*clothing("Mask", Some("skinned/mask"), "Mask", &[])).clone();
    nobeard.hat_category = Some("nobeard".into());
    let visuals = vec![iv("Base.Mask", Rc::new(nobeard))];
    let slot = build(&visuals, &EquippedModels::default());
    assert!(!slot.instances.iter().any(|i| i.kind == InstanceKind::Beard));
    assert!(slot.instances.iter().any(|i| i.kind == InstanceKind::Hair));
    // A hat hiding the head mask part removes hair and beard altogether.
    let slot = build(&[iv("Base.Hat", Rc::new(hat))], &EquippedModels::default());
    assert!(!slot
        .instances
        .iter()
        .any(|i| matches!(i.kind, InstanceKind::Hair | InstanceKind::Beard)));
}

#[test]
fn texture_only_clothing_composites_onto_the_body() {
    let tshirt = clothing("TShirt", None, "clothes/tshirt", &[]);
    let jacket = clothing("Jacket", Some("skinned/jacket"), "clothes/jacket", &[1]);
    let mut bloody = iv("Base.Jacket", jacket);
    bloody.set_blood(6, 1.0);
    let visuals = vec![bloody, iv("Base.TShirt", tshirt)];
    let mut slot = build(&visuals, &EquippedModels::default());
    let hs = styles();
    let mut hv = human_visual();
    let h = human(&mut hv, &hs);
    let groups = BodyLocationGroup::new("Human").unwrap();
    let out = create_textures(
        &mut slot,
        &TextureCreatorInput {
            human: &h,
            item_visuals: &visuals,
            body_locations: &groups,
            decals: &|_| None,
            simple_clothing_textures: false,
        },
    );
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
    assert!(job.commands[..12]
        .iter()
        .all(|c| c.tex == TexRef::Job(0) && c.shader == SmartShader::BodyMask));
    assert_eq!(
        job.commands[12].tex,
        TexRef::File("media/textures/clothes/tshirt.png".into())
    );
    assert_eq!(job.commands[12].shader, SmartShader::Blit);
    let jacket = slot
        .sub
        .iter()
        .map(|&i| slot.instance(i))
        .find(|i| i.mesh_name == "skinned/jacket")
        .unwrap();
    let TextureSource::Smart(j) = jacket.tex else {
        panic!()
    };
    let full = match &out.jobs[j].commands[0].tex {
        TexRef::Job(k) => *k,
        t => panic!("{t:?}"),
    };
    assert!(out.jobs[full]
        .commands
        .iter()
        .any(|c| c.shader == SmartShader::OverlayMask));
}

struct Res;

impl ModelResources for Res {
    fn mesh(&self, name: &str, is_static: bool) -> Option<LoadedMesh> {
        if name == "missing" {
            return None;
        }
        Some(LoadedMesh {
            mesh: Rc::new(MeshData {
                id: 1,
                kind: if is_static {
                    MeshKind::Static
                } else {
                    MeshKind::Skinned
                },
                vertices: vec![],
                indices: vec![],
            }),
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
    let visuals = vec![iv("Base.Shirt", shirt)];
    let mut slot = build(&visuals, &EquippedModels::default());
    slot.instances[slot.model].tex = TextureSource::Smart(0);
    let mut sd = SkinningData {
        skeleton_hierarchy: vec![-1],
        bind_pose: vec![[
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]],
        ..Default::default()
    };
    sd.inverse_bind_pose = sd.bind_pose.clone();
    sd.bone_offset = sd.bind_pose.clone();
    sd.bone_indices.insert("Root".into(), 0);
    let player = AnimationPlayer::new(Rc::new(sd));
    let mut lights = PlayerLightData::default();
    lights.update(&LightContext {
        x: 10.5,
        y: 10.5,
        z: 0.0,
        result_lights: vec![ResultLight {
            id: 7,
            x: 12,
            y: 10,
            z: 0,
            radius: 6,
            r: 1.0,
            g: 0.5,
            b: 0.2,
            flags: 0,
        }],
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
    let frame = CharacterFrame {
        player: &player,
        x: 10.5,
        y: 10.5,
        z: 0.0,
        alpha: 1.0,
        effect_lights: &lights.effect_lights_main,
        ambient: lights.current_ambient,
        final_scale: 1.0,
    };
    let draws = render_character(&slot, &frame, &mc, &Res);
    // body, shirt, beard, hair
    assert_eq!(draws.len(), 4);
    assert!(draws[0].lights[0].colour[0] > 0.0);
    assert!((draws[0].ambient[0] - 0.27).abs() < 1e-6);
    assert_eq!(draws[0].target_depth, draws[3].target_depth);
}

#[test]
fn body_locations_hide_and_alt_models() {
    use pz_scripts::registry::Registry;
    let mut reg = Registry::new("item_body_location");
    let [jacket_loc, shirt_loc, belt_loc] =
        ["t:jacket", "t:shirt", "t:belt"].map(|id| reg.register_id(id, false).unwrap());
    let mut group = BodyLocationGroup::new("Human").unwrap();
    for k in [&jacket_loc, &shirt_loc, &belt_loc] {
        group.get_or_create_location(Some(k.clone())).unwrap();
    }
    group
        .set_hide_model(Some(&jacket_loc), Some(shirt_loc.clone()))
        .unwrap();
    group
        .set_alt_model(Some(&jacket_loc), Some(belt_loc.clone()))
        .unwrap();
    let mut belt_item = (*clothing("Belt", Some("skinned/belt"), "Belt", &[])).clone();
    belt_item.alt_male_model = Some("skinned/belt_alt".into());
    let mut jacket = iv(
        "Base.Jacket",
        clothing("Jacket", Some("skinned/jacket"), "Jacket", &[]),
    );
    jacket.body_location = Some(jacket_loc.clone());
    let mut shirt = iv(
        "Base.Shirt",
        clothing("Shirt", Some("skinned/shirt"), "Shirt", &[]),
    );
    shirt.body_location = Some(shirt_loc.clone());
    let mut belt = iv("Base.Belt", Rc::new(belt_item));
    belt.body_location = Some(belt_loc.clone());
    let visuals = vec![shirt, belt, jacket];
    assert!(is_item_model_hidden(&group, &visuals, 0));
    assert!(!is_item_model_hidden(&group, &visuals, 2));
    assert!(is_item_model_alt(&group, &visuals, 1));
    assert!(should_hide_model(&group, &visuals, &shirt_loc));

    let hs = styles();
    let mut hv = human_visual();
    let h = human(&mut hv, &hs);
    let scripts: HashMap<String, ModelScriptRef> = HashMap::new();
    let shirt_item = plain("Shirt", Some("Base.ShirtModel"));
    let belt_item = plain("Belt", Some("Base.BeltModel"));
    let eq = EquippedModels {
        attached: vec![
            AttachedModel {
                attachment_name: Some("t:shirt".into()),
                item: HeldItem::new(&shirt_item.0, Some(shirt_item.1.clone())),
            },
            AttachedModel {
                attachment_name: Some("t:belt".into()),
                item: HeldItem::new(&belt_item.0, Some(belt_item.1.clone())),
            },
        ],
        ..Default::default()
    };
    let slot = ModelSlot::build(&CharacterModelInput {
        human: &h,
        item_visuals: &visuals,
        equipped: &eq,
        body_locations: &group,
        hair_styles: &hs,
        scripts: &scripts,
        body_model_script: None,
        bone_index: &|_| -1,
    });
    let names: Vec<&str> = slot
        .sub
        .iter()
        .map(|&i| slot.instance(i).mesh_name.as_str())
        .collect();
    assert!(!names.contains(&"skinned/shirt"));
    // An attached item on a hidden body location is skipped
    // (`shouldHideModel`); the others hang off their attachment.
    assert!(!names.contains(&"ShirtModel"));
    let belt = slot.instance(slot.attached_models[0]);
    assert_eq!(slot.attached_models.len(), 1);
    assert_eq!(
        (
            belt.mesh_name.as_str(),
            belt.attachment_name_self.as_deref()
        ),
        ("BeltModel", Some("t:belt"))
    );
    assert!(names.contains(&"skinned/belt_alt") && names.contains(&"skinned/jacket"));
}

/// A rifle with a scope attached: in the right hand and on the back, each
/// with its `ModelWeaponPart` models under it.
fn rifle_with_scope() -> (InventoryItem, ItemScriptRef) {
    let mut rifle = weapon("HuntingRifle", Some("HuntingRifle"));
    let scope = item(ItemClass::WeaponPart, "x4Scope", |_| {});
    if let ItemKind::Weapon(w) = &mut rifle.kind {
        w.attachments.push(scope);
        w.blood_level = 0.25;
    }
    let mut s = Item::new();
    s.model_weapon_part = Some(vec![
        mwp("Base.RecoilPad", "Base.RecoilPad", "recoilpad"),
        mwp("Base.x4Scope", "Base.x4Scope", "scope"),
        mwp("Base.x8Scope", "Base.x8Scope", "scope"),
    ]);
    (rifle, Rc::new(RefCell::new(s)))
}

#[test]
fn weapons_carry_their_part_models() {
    let (rifle, script) = rifle_with_scope();
    let scripts: HashMap<String, ModelScriptRef> = [
        ("HuntingRifle", "weapons/firearm/MSR788_Rifle"),
        ("Base.x4Scope", "weapons/parts/Rifle_4XScope"),
    ]
    .into_iter()
    .map(|(n, m)| (n.to_string(), model_script(get_item_name(n), m)))
    .collect();
    let eq = EquippedModels {
        attached: vec![AttachedModel {
            attachment_name: Some("rifle_back".into()),
            item: HeldItem::new(&rifle, Some(script.clone())),
        }],
        primary: Some(HeldItem::new(&rifle, Some(script.clone()))),
        secondary: Some(HeldItem::new(&rifle, Some(script))),
        ..Default::default()
    };
    let slot = build_with(&[], &eq, &scripts);
    let hand = slot.instance(slot.primary_hand.unwrap());
    assert_eq!(hand.mesh_name, "weapons/firearm/MSR788_Rifle");
    assert_eq!(hand.parent_bone_name.as_deref(), Some("Bip01_Prop1"));
    assert!(slot.secondary_hand.is_none());
    let init = hand.texture_init.unwrap();
    assert_eq!((init.blood_level, init.tint), (0.25, [1.0; 3]));
    // Only the attached part's model, on the weapon (not in the slot).
    assert_eq!(hand.sub.len(), 1);
    let scope = slot.instance(hand.sub[0]);
    assert_eq!(scope.kind, InstanceKind::WeaponPart);
    assert_eq!(scope.mesh_name, "weapons/parts/Rifle_4XScope");
    assert_eq!(scope.attachment_name_self.as_deref(), Some("scope"));
    assert_eq!(scope.parent, slot.primary_hand);
    assert!(!slot.sub.contains(&hand.sub[0]));
    // On the back: the body's attachment, with its own scope.
    let back = slot.instance(slot.attached_models[0]);
    assert_eq!(
        (back.kind, back.attachment_name_parent.as_deref()),
        (InstanceKind::Attached, Some("rifle_back"))
    );
    assert_eq!(
        slot.instance(back.sub[0]).mesh_name,
        "weapons/parts/Rifle_4XScope"
    );
    // The model script's texture names the hand model's texture.
    assert_eq!(
        hand.tex,
        TextureSource::File("weapons/firearm/MSR788_Rifle_tex".into())
    );
    // Hidden weapon models: nothing in the hands.
    let eq = EquippedModels {
        hide_weapon_model: true,
        ..eq
    };
    let slot = build_with(&[], &eq, &scripts);
    assert!(slot.primary_hand.is_none() && slot.secondary_hand.is_none());
}

#[test]
fn static_model_getters() {
    let scripts: HashMap<String, ModelScriptRef> = [(
        "Base.AppleBurnt".to_string(),
        model_script("AppleBurnt", "apple_burnt"),
    )]
    .into();
    // Weapon: `weaponSpritesByIndex` by model index, else the sprite.
    let mut w = weapon("Bat", Some("Bat"));
    let mut ws = Item::new();
    ws.weapon_sprites_by_index = Some(vec!["BatA".into(), "BatB".into()]);
    let ws = Rc::new(RefCell::new(ws));
    assert_eq!(
        HeldItem::new(&w, Some(ws.clone()))
            .static_model(&scripts)
            .as_deref(),
        Some("Bat")
    );
    w.model_index = 1;
    assert_eq!(
        HeldItem::new(&w, Some(ws))
            .static_model(&scripts)
            .as_deref(),
        Some("BatB")
    );
    // Plain item: mod data, then the script; the world model by tag.
    let (mut p, ps) = plain("Pan", Some("Base.Pan"));
    ps.borrow_mut().world_static_model = Some("Base.PanGround".into());
    assert_eq!(
        HeldItem::new(&p, Some(ps.clone()))
            .static_model_exception(&scripts)
            .as_deref(),
        Some("Base.Pan")
    );
    let mut t = LuaTable::default();
    t.rawset(
        LuaKey::Str("staticModel".into()),
        Some(LuaValue::Str("Base.Pot".into())),
    );
    p.table = Some(t);
    let h = HeldItem::new(&p, Some(ps.clone()));
    assert_eq!(h.static_model(&scripts).as_deref(), Some("Base.Pot"));
    assert_eq!(h.world_static_model().as_deref(), Some("Base.PanGround"));
    let mut tpl = (*p.template).clone();
    tpl.tags.push(TAG_USE_WORLD_STATIC_MODEL.into());
    p.template = Rc::new(tpl);
    assert_eq!(
        HeldItem::new(&p, Some(ps))
            .static_model_exception(&scripts)
            .as_deref(),
        Some("Base.PanGround")
    );
    // Food: a burnt variant when its model script exists.
    let mut apple = item(ItemClass::Food, "Apple", |_| {});
    let (_, s) = plain("Apple", Some("Base.Apple"));
    if let ItemKind::Food(f) = &mut apple.kind {
        f.burnt = true;
    }
    let h = HeldItem::new(&apple, Some(s));
    assert_eq!(h.static_model(&scripts).as_deref(), Some("AppleBurnt"));
    assert!(!h.requires_smart_texture());
}

#[test]
fn hand_replacements_and_action_overrides() {
    // A non-weapon held item only shows through a hand replacement or an
    // action that overrides the hand models.
    let (torch, s) = plain("Torch", Some("Base.Torch"));
    let held = |s: &ItemScriptRef| HeldItem::new(&torch, Some(s.clone()));
    let eq = EquippedModels {
        primary: Some(held(&s)),
        ..Default::default()
    };
    assert!(build(&[], &eq).primary_hand.is_none());
    let eq = EquippedModels {
        action: Some(ActionHandModels {
            primary_item: Some(held(&s)),
            secondary_model: Some("Base.Lighter".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let slot = build(&[], &eq);
    assert_eq!(slot.instance(slot.primary_hand.unwrap()).mesh_name, "Torch");
    let sec = slot.instance(slot.secondary_hand.unwrap());
    assert_eq!(
        (sec.mesh_name.as_str(), sec.parent_bone_name.as_deref()),
        ("Lighter", Some("Bip01_Prop2"))
    );
    // `primaryAnimMask`: the static model as a masking model, with the
    // mask variable set.
    s.borrow_mut().replace_primary_hand = Some(ItemReplacement {
        clothing_item_name: None,
        mask_variable_name: Some("RightHandMask".into()),
        mask_variable_value: Some("HoldTorch".into()),
        attachment: None,
    });
    let eq = EquippedModels {
        primary: Some(held(&s)),
        ..Default::default()
    };
    let slot = build(&[], &eq);
    let hand = slot.instance(slot.primary_hand.unwrap());
    assert_eq!(hand.mask_variable_value.as_deref(), Some("HoldTorch"));
    assert_eq!(
        slot.mask_variables,
        [("RightHandMask".to_string(), "HoldTorch".to_string())]
    );
    // On an attachment it hangs off the body only (not the slot's `sub`).
    s.borrow_mut()
        .replace_primary_hand
        .as_mut()
        .unwrap()
        .attachment = Some("torch".into());
    let slot = build(&[], &eq);
    let i = slot.primary_hand.unwrap();
    assert_eq!(
        slot.instance(i).attachment_name_self.as_deref(),
        Some("torch")
    );
    assert!(!slot.sub.contains(&i) && slot.instance(slot.model).sub.contains(&i));
}

#[test]
fn equipped_texture_jobs() {
    let (mut rifle, _) = rifle_with_scope();
    rifle.color_red = 0.5;
    let init = HeldItem::new(&rifle, None).texture_init();
    assert_eq!(init.tint, [0.5, 1.0, 1.0]);
    let tex = "media/textures/weapons/firearm/MSR788.png";
    assert_eq!(
        equipped_texture_name(tex, "TINT"),
        "media/textures/weapons/firearm/MSR788TINT.png"
    );
    assert_eq!(
        equipped_texture_name("weapons/Bat", "TINT"),
        "media/textures/weapons/BatTINT.png"
    );
    // With a tint mask: base, tinted overlay (overlay + tint), blood.
    let job = equipped_texture_job(tex, &init, &|n| n.ends_with("TINT.png")).unwrap();
    let shaders: Vec<_> = job.commands.iter().map(|c| c.shader).collect();
    assert_eq!(
        shaders,
        [
            SmartShader::Blit,
            SmartShader::OverlayMask,
            SmartShader::HueChange,
            SmartShader::OverlayMask
        ]
    );
    // Without: a plain tint.
    let job = equipped_texture_job(tex, &init, &|_| false).unwrap();
    assert_eq!(job.commands[1].shader, SmartShader::HueChange);
    // White and clean: no smart texture.
    let clean = ItemTextureInit {
        blood_level: 0.0,
        tint: [1.0; 3],
        fluid_level: 1.0,
        fluid_tint: None,
    };
    assert!(equipped_texture_job(tex, &clean, &|_| true).is_none());
}
