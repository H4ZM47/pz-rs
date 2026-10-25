//! Parity against the original classes (`transformToParent`, clothing skin
//! palettes, `Shader.setLightInst`, `PlayerData` light fading, and a hand
//! weapon with attached parts: `Item.Load`'s `ModelWeaponPart` entries
//! picked for the attached parts and their transforms on the weapon).
//!
//! The oracle harness (outside the repo) writes `skeleton.txt`,
//! `clothing_skeleton.txt` and `model_frames.txt`; point
//! `PZ_MODEL_ORACLE_DIR` at them and `PZ_SCRIPTS_DIR` at
//! `media/scripts/generated`. Without them this test does nothing.

#![allow(clippy::needless_range_loop)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use pz_anim::math::Matrix4f;
use pz_anim::skeleton::Skeleton;
use pz_anim::AnimationPlayer;
use pz_character_model::data::{BodyLocationGroup, HumanModel, HumanVisual};
use pz_character_model::items::{EquippedModels, HeldItem};
use pz_character_model::lights::{EffectLight, PlayerLightData, ResultLight};
use pz_character_model::model_tree::{CharacterModelInput, InstanceKind, ModelInstance, ModelSlot};
use pz_character_model::render::transform_to_parent;
use pz_character_model::{ModelScript, ModelScriptRef, TextureSource};
use pz_inventory::{InventoryItem, ItemClass, ItemKind, ItemTemplate};
use pz_model::{AnimationClip, Keyframe, Quat, SkinningData};
use pz_scripts::objects::items::Item;
use pz_scripts::{ScriptLoadMode, ScriptManager, ScriptObject};

fn f(s: &str) -> f32 {
    s.parse().unwrap()
}

fn load_skinning(path: &std::path::Path) -> SkinningData {
    let text = std::fs::read_to_string(path).unwrap();
    let mut sd = SkinningData::default();
    let mut clip: Option<AnimationClip> = None;
    let m16 = |v: &[&str]| {
        let mut a = [0f32; 16];
        for i in 0..16 {
            a[i] = f(v[i]);
        }
        Matrix4f::load(&a).to_mat4()
    };
    for line in text.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        match p[0] {
            "bones" => {
                let n: usize = p[1].parse().unwrap();
                sd.bind_pose = vec![Default::default(); n];
                sd.inverse_bind_pose = vec![Default::default(); n];
                sd.bone_offset = vec![Default::default(); n];
                sd.skeleton_hierarchy = vec![-1; n];
            }
            "name" => {
                sd.bone_indices
                    .insert(p[2].to_string(), p[1].parse().unwrap());
            }
            "parent" => {
                sd.skeleton_hierarchy[p[1].parse::<usize>().unwrap()] = p[2].parse().unwrap()
            }
            "bind" => sd.bind_pose[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "invbind" => sd.inverse_bind_pose[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "offset" => sd.bone_offset[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "clip" => {
                if let Some(c) = clip.take() {
                    sd.animation_clips.insert(c.name.clone(), c);
                }
                clip = Some(AnimationClip {
                    name: p[1].into(),
                    duration: f(p[2]),
                    keep_last_frame: p[3] == "true",
                    ..Default::default()
                });
            }
            "k" => clip.as_mut().unwrap().keyframes.push(Keyframe {
                bone: p[1].parse().unwrap(),
                time: f(p[2]),
                position: [f(p[3]), f(p[4]), f(p[5])],
                rotation: Quat {
                    x: f(p[6]),
                    y: f(p[7]),
                    z: f(p[8]),
                    w: f(p[9]),
                },
                scale: [f(p[10]), f(p[11]), f(p[12])],
                bone_name: p.get(13).unwrap_or(&"").to_string(),
            }),
            _ => {}
        }
    }
    if let Some(c) = clip.take() {
        sd.animation_clips.insert(c.name.clone(), c);
    }
    sd
}

/// One `model <name> { ... }` block of a generated script file, loaded by
/// pz-scripts' `ModelScript.Load`.
fn script(sm: &ScriptManager, dir: &std::path::Path, file: &str, name: &str) -> ModelScriptRef {
    let t = std::fs::read_to_string(dir.join(file)).unwrap();
    let start = t
        .find(&format!("    model {name}\n"))
        .or_else(|| t.find(&format!("    model {name}\r\n")))
        .unwrap();
    let open = start + t[start..].find('{').unwrap();
    let mut depth = 0;
    let mut end = open;
    for (i, c) in t[open..].char_indices() {
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                end = open + i;
                break;
            }
        }
    }
    let mut m = ModelScript::new();
    m.init_load_pp(sm, name);
    m.load(sm, name, &t[start..=end]).unwrap();
    Rc::new(RefCell::new(m))
}

fn inst() -> ModelInstance {
    ModelInstance {
        kind: InstanceKind::Attached,
        mesh_name: String::new(),
        model_texture: None,
        is_static: true,
        shader: None,
        tex: TextureSource::None,
        tint: [1.0; 3],
        hue: 0.0,
        depth_bias: 0.0,
        parent: None,
        parent_bone: 0,
        parent_bone_name: None,
        attachment_name_self: None,
        attachment_name_parent: None,
        scale: 1.0,
        model_script: None,
        item_visual: None,
        mask_variable_value: None,
        texture_init: None,
        sub: Vec::new(),
    }
}

/// The item block of `name` in `items/weapon.txt`, cut down to its model
/// lines (`WeaponSprite`, `ModelWeaponPart`) as the harness does.
fn weapon_block(dir: &std::path::Path, name: &str) -> String {
    let t = std::fs::read_to_string(dir.join("items/weapon.txt")).unwrap();
    let i = t.find(&format!("    item {name}\n")).unwrap();
    let open = i + t[i..].find('{').unwrap();
    let close = open + t[open..].find('}').unwrap();
    let mut b = format!("item {name}\n{{\n");
    for l in t[open + 1..close].split('\n') {
        let tl = l.trim();
        if tl.starts_with("WeaponSprite") || tl.starts_with("ModelWeaponPart") {
            b += l;
            b += "\n";
        }
    }
    b + "}"
}

#[test]
fn matches_model_oracle() {
    let (Ok(dir), Ok(scripts)) = (
        std::env::var("PZ_MODEL_ORACLE_DIR"),
        std::env::var("PZ_SCRIPTS_DIR"),
    ) else {
        eprintln!("PZ_MODEL_ORACLE_DIR / PZ_SCRIPTS_DIR not set; skipping");
        return;
    };
    let (dir, scripts) = (PathBuf::from(dir), PathBuf::from(scripts));
    let body = Rc::new(load_skinning(&dir.join("skeleton.txt")));
    let cloth = Skeleton::from_skinning_data(&load_skinning(&dir.join("clothing_skeleton.txt")));
    let frames = std::fs::read_to_string(dir.join("model_frames.txt")).unwrap();

    let mut player = AnimationPlayer::new(body.clone());
    player.set_target_and_current_direction(0.6, 0.8);
    let t = player.play("Bob_Walk", true, false, -1.0).unwrap();
    player.track_mut(t).unwrap().set_blend_weight(1.0);

    let sm = ScriptManager::new();
    let body_script = script(&sm, &scripts, "models_characters.txt", "MaleBody");
    let bowl = script(&sm, &scripts, "models_items.txt", "Bowl");
    let rifle = script(&sm, &scripts, "models_weapons.txt", "HuntingRifle");
    let mut m_body = inst();
    m_body.is_static = false;
    m_body.model_script = Some(body_script);
    let mut prim = inst();
    prim.parent_bone_name = Some("Bip01_Prop1".into());
    prim.scale = bowl.borrow().scale;
    prim.model_script = Some(bowl.clone());
    let mut sec = prim.clone();
    sec.parent_bone_name = Some("Bip01_Prop2".into());
    let mut back = inst();
    back.attachment_name_self = Some("rifle_back".into());
    back.attachment_name_parent = Some("rifle_back".into());
    back.scale = rifle.borrow().scale;
    back.model_script = Some(rifle.clone());
    let mut scope = inst();
    scope.attachment_name_self = Some("scope".into());
    scope.attachment_name_parent = Some("scope".into());
    scope.model_script = Some(rifle.clone());
    let mut band = inst();
    band.parent_bone_name = Some("Bip01_Head".into());

    // A HuntingRifle in the right hand with an x4Scope and a RecoilPad
    // attached, through `DoCharacterModelEquipped`.
    sm.parse_script(ScriptLoadMode::Init, "module Base { }")
        .unwrap();
    let base = sm.get_module("Base").unwrap();
    let mut rifle_item = Item::new();
    rifle_item.set_module(&base);
    rifle_item
        .load(&sm, "HuntingRifle", &weapon_block(&scripts, "HuntingRifle"))
        .unwrap();
    let mut tpl = ItemTemplate::new(ItemClass::Weapon, "Base", "HuntingRifle");
    tpl.weapon_sprite = rifle_item.weapon_sprite.clone();
    tpl.model_weapon_part = rifle_item
        .model_weapon_part
        .as_ref()
        .map(|v| Rc::new(v.iter().map(|m| m.borrow().clone()).collect()));
    let mut held_rifle = InventoryItem::create(Rc::new(tpl), 0);
    if let ItemKind::Weapon(w) = &mut held_rifle.kind {
        for part in ["x4Scope", "RecoilPad"] {
            let t = ItemTemplate::new(ItemClass::WeaponPart, "Base", part);
            w.attachments.push(InventoryItem::create(Rc::new(t), 0));
        }
    }
    let mut model_scripts: std::collections::HashMap<String, ModelScriptRef> =
        [("HuntingRifle".to_string(), rifle.clone())].into();
    for part in ["x4Scope", "RecoilPad", "x2Scope", "x8Scope"] {
        model_scripts.insert(
            format!("Base.{part}"),
            script(&sm, &scripts, "models_items.txt", part),
        );
    }
    let eq = EquippedModels {
        primary: Some(HeldItem::new(&held_rifle)),
        ..Default::default()
    };
    let mut hv = HumanVisual::new(pz_character_model::mask::BLOOD_PARTS);
    let human = HumanModel::resolve(
        &mut hv,
        false,
        false,
        false,
        &pz_inventory::visual::human::NoStyles,
        &[],
    );
    let groups = BodyLocationGroup::new("Human").unwrap();
    let bone_of = |n: &str| body.bone_indices.get(n).map_or(-1, |&i| i);
    let wslot = ModelSlot::build(&CharacterModelInput {
        human: &human,
        item_visuals: &[],
        equipped: &eq,
        body_locations: &groups,
        hair_styles: None,
        beard_styles: None,
        scripts: &model_scripts,
        body_model_script: Some(m_body.model_script.clone().unwrap()),
        bone_index: &bone_of,
    });
    let wprim = wslot.instance(wslot.primary_hand.unwrap()).clone();
    let wparts: Vec<ModelInstance> = wprim
        .sub
        .iter()
        .map(|&i| wslot.instance(i).clone())
        .collect();
    let mut cur_wprim = [[0f32; 4]; 4];
    let mut max_w = 0f32;
    let mut nw = 0;

    let mut max_x = 0f32;
    let mut max_p = 0f32;
    let mut max_l = 0f32;
    let mut fade_mismatch = 0;
    let mut nx = 0;
    let mut np = 0;
    let mut nl = 0;
    let mut cur_back = [[0f32; 4]; 4];
    let ident = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let mut lights = PlayerLightData::default();
    let mut fade_frame = 0;
    let seq: [&[i32]; 12] = [
        &[1, 2],
        &[1, 2],
        &[1, 2, 3],
        &[2, 3],
        &[2, 3],
        &[3],
        &[3],
        &[],
        &[],
        &[],
        &[1],
        &[1],
    ];
    for line in frames.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        match p[0] {
            "frame" => {
                player.update(0.05, None);
                assert!((player.rendered_angle() - f(p[2])).abs() < 1e-6);
            }
            "x" => {
                let ours = match p[1] {
                    "body" => transform_to_parent(&m_body, None, None, &player),
                    "prim" => transform_to_parent(&prim, None, Some((&m_body, &ident)), &player),
                    "sec" => transform_to_parent(&sec, None, Some((&m_body, &ident)), &player),
                    "back" => {
                        cur_back =
                            transform_to_parent(&back, None, Some((&m_body, &ident)), &player);
                        cur_back
                    }
                    "band" => transform_to_parent(&band, None, Some((&m_body, &ident)), &player),
                    "scope" => transform_to_parent(&scope, None, Some((&back, &cur_back)), &player),
                    "wprim" => {
                        cur_wprim =
                            transform_to_parent(&wprim, None, Some((&m_body, &ident)), &player);
                        cur_wprim
                    }
                    _ => unreachable!(),
                };
                // The original keeps xfrm transposed; JOML's get() is column-major.
                for i in 0..4 {
                    for j in 0..4 {
                        max_x = max_x.max((f(p[2 + i * 4 + j]) - ours[j][i]).abs());
                    }
                }
                nx += 1;
            }
            "wp" => {
                // `wp <i> <modelName> <attachSelf> <attachParent> <xfrm>`.
                let i: usize = p[1].parse().unwrap();
                let part = &wparts[i];
                let script_name = part
                    .model_script
                    .as_ref()
                    .and_then(|s| s.borrow().name.clone())
                    .unwrap();
                assert_eq!(format!("Base.{script_name}"), p[2], "part {i} model");
                assert_eq!(part.attachment_name_self.as_deref(), Some(p[3]));
                assert_eq!(part.attachment_name_parent.as_deref(), Some(p[4]));
                let ours = transform_to_parent(part, None, Some((&wprim, &cur_wprim)), &player);
                for r in 0..4 {
                    for c in 0..4 {
                        max_w = max_w.max((f(p[5 + r * 4 + c]) - ours[c][r]).abs());
                    }
                }
                nw += 1;
            }
            "p" => {
                let b: usize = p[1].parse().unwrap();
                let ours = Matrix4f::from_mat4(&player.palette_for(&cloth)[b]).store();
                for i in 0..16 {
                    max_p = max_p.max((f(p[2 + i]) - ours[i]).abs());
                }
                np += 1;
            }
            "l" => {
                let ang = f(p[1]);
                let i: usize = p[2].parse().unwrap();
                let ls: [[f32; 7]; 5] = [
                    [12.5, 10.5, 0.5, 1.0, 0.8, 0.6, 8.0],
                    [9.0, 9.0, 1.0, 0.3, 0.3, 0.3, 5000.0],
                    [10.2, 10.3, 0.0, 1.0, 1.0, 1.0, 2.0],
                    [0.0; 7],
                    [14.0, 3.0, 2.0, 0.9, 0.2, 0.1, 4.0],
                ];
                let l = ls[i];
                let e = EffectLight {
                    x: l[0],
                    y: l[1],
                    z: l[2],
                    r: l[3],
                    g: l[4],
                    b: l[5],
                    radius: l[6] as i32,
                };
                let m = e.to_model_light([10.3, 10.6, 0.2], (!ang.is_nan()).then_some(ang));
                let got = [
                    m.direction[0],
                    m.direction[1],
                    m.direction[2],
                    m.colour[0],
                    m.colour[1],
                    m.colour[2],
                ];
                for k in 0..6 {
                    max_l = max_l.max((f(p[3 + k]) - got[k]).abs());
                }
                nl += 1;
            }
            "fl" => {
                let chosen: Vec<ResultLight> = seq[fade_frame]
                    .iter()
                    .map(|&id| ResultLight {
                        id,
                        x: 10 + id,
                        y: 10 - id,
                        z: 0,
                        radius: 5 + id,
                        r: 0.2 * id as f32,
                        g: 0.9 - 0.1 * id as f32,
                        b: 0.5,
                        flags: 0,
                    })
                    .collect();
                lights.fade_frame_lights(&chosen, 0.8);
                fade_frame += 1;
                let ours: Vec<String> = lights
                    .frame_lights
                    .iter()
                    .map(|fl| match fl {
                        None => "-".to_string(),
                        Some(l) => format!(
                            "{}:{}:{:?}:{},{},{}",
                            l.id,
                            l.active,
                            l.stage,
                            pz_anim::vars::java_float_to_string(l.current_color[0]),
                            pz_anim::vars::java_float_to_string(l.current_color[1]),
                            pz_anim::vars::java_float_to_string(l.current_color[2])
                        ),
                    })
                    .collect();
                if ours.join(" ") != p[2..].join(" ") {
                    fade_mismatch += 1;
                    eprintln!(
                        "fade frame {}: ours {:?} java {:?}",
                        fade_frame - 1,
                        ours,
                        &p[2..]
                    );
                }
            }
            _ => {}
        }
    }
    eprintln!(
        "model oracle: {nx} attachment transforms max err {max_x:e}; {nw} weapon-part transforms max err {max_w:e}; {np} clothing palette matrices max err {max_p:e}; {nl} light encodings max err {max_l:e}; {fade_frame} fade frames, {fade_mismatch} mismatches"
    );
    assert_eq!(nw, wparts.len() * 12, "weapon parts per frame");
    assert!(max_x < 1e-4 && max_w < 1e-4 && max_p < 1e-4 && max_l < 1e-5 && fade_mismatch == 0);
}
