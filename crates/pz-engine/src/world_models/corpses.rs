//! `IsoDeadBody.render` and `renderShadow` for the corpses on the squares:
//! `DeadBodyAtlas.getBodyTexture(body)` (`BodyParams.init(IsoDeadBody)`,
//! `getBodyKey`, the render job), then the atlas entry's draw.
//!
//! A body's model is built once per body key. The draw waits for the
//! model's texture jobs ([`WorldModels::prepare`]), then the render job
//! draws it into the atlas and the model is released; the atlas entry
//! stays for the key.

use std::collections::HashMap;

use pz_anim::bone_transform::BoneTransform;
use pz_anim::math::{Quaternion, Vector3f};
use pz_anim::AnimationPlayer;
use pz_character_model::corpse::{
    animal_body_key, body_key, corpse_shadow, corpse_world_draw, pose, BodyVisualKey, ClothingKey,
    DeadBody, DeadBodyAtlas, PointLight,
};
use pz_character_model::data::{ClothingItemExt, HumanModel, ResolvedItemVisual, WHITE};
use pz_character_model::items::EquippedModels;
use pz_character_model::model_tree::{CharacterModelInput, ModelSlot};
use pz_character_model::texture_creator::{create_textures, SmartTextureJob, TextureCreatorInput};
use pz_inventory::SavedItem;
use pz_iso_render::objects::ShadowRequest;
use pz_iso_render::Camera;
use pz_render::DrawCmd;
use pz_scripts::objects::items::ItemBodyLocation;
use pz_scripts::objects::models::{AnimationsMesh, ModelScript};
use pz_scripts::registry::ResourceLocation;
use pz_scripts::{ScriptManager, ScriptType};
use pz_world::save::visual::{CorpseVisual, HumanVisual, IsoDeadBody};

use super::{retire_slot, WorldModels};

/// The first `DeadBodyAtlas` texture id: above the chunk textures' ids
/// (`wx << 32 | wy << 16 | level`) for any world chunk.
const ATLAS_FIRST_ID: u64 = 0x7FFF_FFF0_0000_0000;

/// A body's model waiting for its textures.
pub(super) struct PendingBody {
    slot: ModelSlot,
    player: AnimationPlayer,
    /// What the texture creator reads, until [`Corpses::create_textures`]
    /// (the game files are only reachable there, not while drawing).
    textures: Option<TextureInput>,
    jobs: Option<Vec<SmartTextureJob>>,
}

/// The resolved visual the texture creator needs.
struct TextureInput {
    visual: HumanVisual,
    body_visuals: Vec<ResolvedItemVisual>,
    item_visuals: Vec<ResolvedItemVisual>,
    female: bool,
    zombie: bool,
    skeleton: bool,
    simple_clothing: bool,
    world_age_days: f32,
}

/// `DeadBodyAtlas.instance` and the models of the bodies not drawn into
/// it yet.
pub(super) struct Corpses {
    atlas: DeadBodyAtlas,
    pending: HashMap<String, PendingBody>,
}

impl Default for Corpses {
    fn default() -> Self {
        Corpses {
            atlas: DeadBodyAtlas::new(ATLAS_FIRST_ID),
            pending: HashMap::new(),
        }
    }
}

impl Corpses {
    /// `TextureCreator` for the bodies built since the last call: their
    /// texture jobs, which [`WorldModels::prepare`] then runs.
    pub(super) fn create_textures(&mut self, a: &crate::ui_models::Assets, fs: &pz_fs::GameFs) -> Result<(), String> {
        if self.pending.values().all(|p| p.textures.is_none()) {
            return Ok(());
        }
        let chars = pz_lua_api::bind_characters::shared_characters();
        let (group, hair_colors) = {
            let mut c = chars
                .try_borrow_mut()
                .map_err(|_| "body-location registry is already borrowed")?;
            let group = c.body_locations.get_group("Human").map_err(|e| e.to_string())?;
            let colors: Vec<[f32; 3]> = c.survivor_desc.hair_common_colors.iter().map(|c| c.rgb()).collect();
            (group, colors)
        };
        let decals = |name: &str| a.pop.clothing_decals.as_ref().and_then(|d| d.get_decal(fs, name));
        for p in self.pending.values_mut() {
            let Some(mut t) = p.textures.take() else { continue };
            let styles = a.pop.styles(&hair_colors, t.world_age_days);
            let human = HumanModel::resolve(&mut t.visual, t.female, t.zombie, t.skeleton, &styles, &t.body_visuals);
            let jobs = create_textures(
                &mut p.slot,
                &TextureCreatorInput {
                    human: &human,
                    item_visuals: &t.item_visuals,
                    body_locations: &group.borrow(),
                    decals: &decals,
                    simple_clothing_textures: t.simple_clothing,
                },
            );
            p.jobs = Some(jobs.jobs);
        }
        Ok(())
    }

    /// The pending models' slots and texture jobs.
    pub(super) fn slots_mut(
        &mut self,
    ) -> impl Iterator<Item = (&mut ModelSlot, &mut Option<Vec<SmartTextureJob>>)> {
        self.pending.values_mut().map(|p| (&mut p.slot, &mut p.jobs))
    }

    pub(super) fn retire(&mut self, textures: &mut Vec<pz_render::Texture>) {
        for (_, p) in self.pending.drain() {
            retire_slot(p.slot, textures);
        }
    }
}

/// What the square gives a body's render.
pub struct CorpseSquare {
    /// `square.isOutside()`.
    pub outside: bool,
    /// `square.getRoom() != null`.
    pub room: bool,
    /// `square.lighting[0]`'s result lights.
    pub lights: Vec<PointLight>,
    /// `square.interpolateLight(inf, x - sx, y - sy)` with its alpha.
    pub light: [f32; 4],
    /// `GameTime.getWorldAgeDays()` (for a visual's unset values).
    pub world_age_days: f32,
}

/// A body's draws: the atlas entry's quad and its shadow.
pub struct CorpseDraw {
    pub draws: Vec<DrawCmd>,
    pub shadow: ShadowRequest,
}

/// `diedBoneTransforms` from the saved position, rotation and scale.
fn died_bone_transforms(body: &IsoDeadBody) -> Vec<BoneTransform> {
    body.ragdoll
        .iter()
        .flatten()
        .map(|b| {
            let v = b.values;
            let mut t = BoneTransform::default();
            t.set_prs(
                Vector3f { x: v[0], y: v[1], z: v[2] },
                Quaternion { x: v[3], y: v[4], z: v[5], w: v[6] },
                Vector3f { x: v[7], y: v[8], z: v[9] },
            );
            t
        })
        .collect()
}

/// Resolved animal corpse rendering inputs derived from `IsoDeadBody`,
/// `AnimalDefinitions`, `ModelScript` and `AnimationsMesh`.
#[derive(Clone, Debug)]
struct AnimalCorpseSpec {
    dead: DeadBody,
    model_name: String,
    mesh_name: String,
    skeleton_mesh: String,
    skin_texture: String,
    skin_path: String,
    shader_name: String,
    anim_set: String,
    anim_dirs: Vec<String>,
}

fn resolve_animal_corpse(
    body: &IsoDeadBody,
    skin_texture_name: &str,
    rot_stage: i8,
    sq: &CorpseSquare,
    defs: &HashMap<String, pz_animals::AnimalDefinitions>,
    sm: &ScriptManager,
) -> Result<AnimalCorpseSpec, String> {
    let animal = body
        .animal
        .as_ref()
        .ok_or_else(|| "animal corpse is missing AnimalCorpse metadata".to_string())?;
    let animal_type = animal.animal_type.trim();
    let def = defs
        .get(&animal_type.to_lowercase())
        .ok_or_else(|| format!("animal definition for '{animal_type}' is unavailable"))?;
    let skeleton = body.is_skeleton();
    let model_name = if skeleton {
        def.body_model_skel_str
            .as_deref()
            .or(def.body_model_str.as_deref())
    } else {
        def.body_model_str.as_deref()
    }
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .ok_or_else(|| format!("animal definition '{animal_type}' has no body model script"))?;

    let ms = sm
        .get::<ModelScript>(ScriptType::Model, model_name)
        .ok_or_else(|| format!("animal model script '{model_name}' is unavailable"))?;
    let ms = ms.borrow();
    let mesh_name = ms
        .mesh_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("animal model script '{model_name}' has no mesh"))?
        .to_string();
    let shader_name = ms
        .shader_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("animalEffect")
        .to_string();

    let (skeleton_mesh, anim_dirs) = ms
        .animations_mesh
        .as_deref()
        .and_then(|name| sm.get::<AnimationsMesh>(ScriptType::AnimationMesh, name))
        .map(|am| {
            let am = am.borrow();
            let skel = am
                .mesh_file
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(mesh_name.as_str())
                .to_string();
            (skel, am.animation_directories.clone())
        })
        .unwrap_or_else(|| (mesh_name.clone(), Vec::new()));

    let raw_skin = if skeleton {
        def.texture_skeleton
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(skin_texture_name.trim())
    } else if rot_stage > 0 && !body.rotten_texture.trim().is_empty() {
        body.rotten_texture.trim()
    } else if rot_stage > 0
        && def
            .texture_rotten
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
    {
        def.texture_rotten.as_deref().unwrap().trim()
    } else if !skin_texture_name.trim().is_empty() {
        skin_texture_name.trim()
    } else {
        ms.texture_name.as_deref().map(str::trim).unwrap_or("")
    };
    let skin_path = if raw_skin.is_empty() || raw_skin.contains('/') {
        raw_skin.to_string()
    } else {
        format!("Body/{raw_skin}")
    };

    let anim_set = def
        .animset
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(animal_type)
        .to_string();

    let dead = DeadBody {
        x: body.base.x,
        y: body.base.y,
        z: body.base.z,
        angle: body.angle,
        dir: body.base.dir as u8,
        female: body.female,
        zombie: false,
        skeleton,
        animal: true,
        fall_on_front: body.fall_on_front,
        killed_by_fall: body.killed_by_fall,
        being_grappled: false,
        primary_hand: None,
        secondary_hand: None,
        has_hand_item: false,
        outside: sq.outside,
        room: sq.room,
        lights: sq.lights.clone(),
        alpha: 1.0,
        animal_size: if animal.size > 0.0 { animal.size } else { 1.0 },
        animal_anim_set: Some(anim_set.clone()),
        ragdoll_fall: body.ragdoll.is_some(),
        died_bone_transforms: died_bone_transforms(body),
    };

    Ok(AnimalCorpseSpec {
        dead,
        model_name: model_name.to_string(),
        mesh_name,
        skeleton_mesh,
        skin_texture: raw_skin.to_string(),
        skin_path,
        shader_name,
        anim_set,
        anim_dirs,
    })
}

impl WorldModels {
    /// `IsoDeadBody.render` and `renderShadow` for `body`: `None` while
    /// its model's textures are being made. The visual's unset values
    /// (hair, skin) and the worn items' visuals are filled on the body, as
    /// the original's getters do.
    ///
    /// TODO(port): attached models (`AttachedModelNames`), the hand items'
    /// static models, highlights, `getAlpha(playerIndex)` and the atlas
    /// entries' release (`DeadBodyAtlas.checkLights`/`updateCounter`).
    pub fn render_corpse(
        &mut self,
        body: &mut IsoDeadBody,
        sq: &CorpseSquare,
        cam: &Camera,
        pre: &mut Vec<DrawCmd>,
    ) -> Result<Option<CorpseDraw>, String> {
        if self.assets.is_none() {
            self.assets = Some(
                crate::ui_models::Assets::load(&self.game_dir, &self.fs, &self.tm)
                    .ok_or("character media index could not be loaded")?,
            );
        }
        let a = self.assets.as_mut().expect("assets installed above");
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        if let CorpseVisual::Animal {
            skin_texture_name,
            rot_stage,
        } = &body.visual
        {
            let spec = resolve_animal_corpse(
                body,
                skin_texture_name,
                *rot_stage,
                sq,
                &self.animal_defs,
                &sm,
            )?;
            let key = animal_body_key(
                &spec.dead,
                Some(&spec.model_name),
                (!spec.skin_texture.is_empty()).then_some(spec.skin_texture.as_str()),
                &[],
            );
            if !self.corpses.atlas.bodies.contains_key(&key) {
                let (set_name, _, _) = spec.dead.anim_state();
                let set = a.anim_set(set_name);
                let sd = a
                    .skeleton_for_mesh(&spec.skeleton_mesh, &spec.anim_dirs, &set)
                    .ok_or_else(|| {
                        format!(
                            "animal corpse skeleton is unavailable (mesh={}, anim_set={set_name})",
                            spec.skeleton_mesh
                        )
                    })?;
                let mut player = AnimationPlayer::new(sd);
                pose(&mut player, set, &spec.dead);
                let slot = ModelSlot::single_body(
                    &spec.mesh_name,
                    &spec.skin_path,
                    Some(&spec.shader_name),
                );
                self.corpses
                    .atlas
                    .body_texture(&key, &spec.dead, &slot, &player, &a.res, pre);
                retire_slot(slot, &mut self.retired_textures);
            }
            let t = &self.corpses.atlas.bodies[&key];
            let draws = corpse_world_draw(t, cam, &spec.dead, sq.light);
            let shadow = corpse_shadow(&spec.dead, Some(t), [sq.light[0], sq.light[1], sq.light[2]]);
            return Ok(Some(CorpseDraw { draws, shadow }));
        }
        let female = body.female;
        let zombie = body.was_zombie;
        let skeleton = body.is_skeleton();
        let dead = DeadBody {
            x: body.base.x,
            y: body.base.y,
            z: body.base.z,
            angle: body.angle,
            dir: body.base.dir as u8,
            female,
            zombie,
            skeleton,
            animal: false,
            fall_on_front: body.fall_on_front,
            killed_by_fall: body.killed_by_fall,
            being_grappled: false,
            primary_hand: None,
            secondary_hand: None,
            has_hand_item: false,
            outside: sq.outside,
            room: sq.room,
            lights: sq.lights.clone(),
            alpha: 1.0,
            animal_size: 1.0,
            animal_anim_set: None,
            ragdoll_fall: body.ragdoll.is_some(),
            died_bone_transforms: died_bone_transforms(body),
        };
        // `getItemVisuals`: the worn items' visuals, in worn order.
        let env = pz_lua_api::bind_items::item_env();
        let item_assets = env.context().assets();
        let mut visuals = Vec::new();
        let mut add_visual = |location: &str, item: &mut pz_inventory::InventoryItem| -> Result<(), String> {
            let assets = item_assets.ok_or("worn-item visual assets are unavailable")?;
            let key = ResourceLocation::of(location)
                .ok()
                .and_then(|l| ItemBodyLocation::get(&sm, &l));
            if let Some(v) = ResolvedItemVisual::try_from_item(
                item,
                assets,
                &pz_inventory::context::NoOwner,
                env.visual_rng(),
                key,
            )
            .map_err(|e| e.to_string())?
            {
                visuals.push(v);
            }
            Ok(())
        };
        if let Some(r) = &body.runtime {
            for (location, item) in &r.worn {
                let Some(item) = item else { continue };
                let Ok(mut item) = item.try_borrow_mut() else {
                    return Err("a worn item is borrowed".into());
                };
                add_visual(location, &mut item)?;
            }
        } else if let Some(container) = body.container.as_mut() {
            for (location, index) in &body.worn_items {
                let Some(SavedItem::Item(item)) = container.items.get_mut(*index as usize) else {
                    continue;
                };
                add_visual(location, item)?;
            }
        }
        let CorpseVisual::Human(hv) = &mut body.visual else {
            return Err("unsupported: IsoDeadBody.render for an animal".into());
        };
        let chars = pz_lua_api::bind_characters::shared_characters();
        let (group, hair_colors) = {
            let mut c = chars
                .try_borrow_mut()
                .map_err(|_| "body-location registry is already borrowed")?;
            let group = c.body_locations.get_group("Human").map_err(|e| e.to_string())?;
            let colors: Vec<[f32; 3]> = c.survivor_desc.hair_common_colors.iter().map(|c| c.rgb()).collect();
            (group, colors)
        };
        let body_visuals: Vec<ResolvedItemVisual> = hv
            .body_visuals
            .iter()
            .map(|v| ResolvedItemVisual {
                visual: v.clone(),
                clothing_item: a.clothing_item(&v.clothing_item_name),
                body_location: None,
            })
            .collect();
        let styles = a.pop.styles(&hair_colors, sq.world_age_days);
        let skin_color = hv.get_skin_color(&styles);
        let total_blood = hv.get_total_blood();
        let clothing: Vec<ClothingKey> = visuals
            .iter()
            .map(|v| match v.clothing_item.as_deref() {
                Some(c) => (
                    v.base_texture(c).map(str::to_string),
                    v.texture_choice(c).map(str::to_string),
                    v.tint(c),
                    c.model(female).map(str::to_string),
                    v.visual.get_total_blood(),
                ),
                None => (None, None, WHITE, None, v.visual.get_total_blood()),
            })
            .collect();
        let human = HumanModel::resolve(hv, female, zombie, skeleton, &styles, &body_visuals);
        let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let vkey = BodyVisualKey {
            hair_model: non_empty(human.hair_model()),
            beard_model: non_empty(human.beard_model()),
            skin_color,
            skin_texture: non_empty(human.skin_texture()),
            total_blood,
            attached: Vec::new(),
            clothing,
        };
        let key = body_key(&dead, &vkey);
        if !self.corpses.atlas.bodies.contains_key(&key) {
            match self.corpses.pending.get(&key) {
                None => {
                    let (set_name, _, _) = dead.anim_state();
                    let set = a.anim_set(set_name);
                    let sd = a.skeleton(female, &set).ok_or_else(|| {
                        format!("corpse body skeleton is unavailable ({set_name}, female={female})")
                    })?;
                    let mut player = AnimationPlayer::new(sd);
                    pose(&mut player, set, &dead);
                    let bone_index = |n: &str| player.skinning_bone_index(n, -1);
                    let simple_clothing = pz_lua_api::bind_core::shared_core()
                        .borrow()
                        .is_option_simple_clothing_textures(zombie);
                    let slot = ModelSlot::try_build(&CharacterModelInput {
                        human: &human,
                        item_visuals: &visuals,
                        equipped: &EquippedModels::default(),
                        body_locations: &group.borrow(),
                        hair_styles: a.pop.hair_styles.as_ref(),
                        beard_styles: a.pop.beard_styles.as_ref(),
                        scripts: &*sm,
                        body_model_script: None,
                        bone_index: &bone_index,
                    })
                    .map_err(|e| e.to_string())?;
                    drop(human);
                    let textures = TextureInput {
                        visual: (**hv).clone(),
                        body_visuals,
                        item_visuals: visuals,
                        female,
                        zombie,
                        skeleton,
                        simple_clothing,
                        world_age_days: sq.world_age_days,
                    };
                    self.corpses.pending.insert(
                        key,
                        PendingBody {
                            slot,
                            player,
                            textures: Some(textures),
                            jobs: None,
                        },
                    );
                    return Ok(None);
                }
                Some(p) if p.textures.is_some() || p.jobs.is_some() => return Ok(None),
                Some(_) => {
                    // `DeadBodyAtlas.render()`: the render job, then the
                    // model is no longer needed.
                    let p = self.corpses.pending.remove(&key).expect("pending body above");
                    self.corpses
                        .atlas
                        .body_texture(&key, &dead, &p.slot, &p.player, &a.res, pre);
                    retire_slot(p.slot, &mut self.retired_textures);
                }
            }
        }
        let t = &self.corpses.atlas.bodies[&key];
        let draws = corpse_world_draw(t, cam, &dead, sq.light);
        let shadow = corpse_shadow(&dead, Some(t), [sq.light[0], sq.light[1], sq.light[2]]);
        Ok(Some(CorpseDraw { draws, shadow }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::save::visual::BoneTransform as SavedBone;

    fn body(ragdoll: Option<Vec<SavedBone>>) -> IsoDeadBody {
        IsoDeadBody {
            ragdoll,
            ..IsoDeadBody::for_tests(1.5, 2.5, 0.0)
        }
    }

    #[test]
    fn no_ragdoll_has_no_died_bones() {
        assert!(died_bone_transforms(&body(None)).is_empty());
    }

    #[test]
    fn ragdoll_bones_are_position_rotation_scale() {
        let values = [1.0, 2.0, 3.0, 0.0, 0.6, 0.0, 0.8, 1.0, 2.0, 0.5];
        let bones = vec![
            SavedBone { bone: 0, values: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0] },
            SavedBone { bone: 1, values },
        ];
        let mut t = died_bone_transforms(&body(Some(bones)));
        assert_eq!(t.len(), 2);
        let (pos, rot, scale) = t[1].prs();
        assert_eq!(pos, Vector3f { x: 1.0, y: 2.0, z: 3.0 });
        assert_eq!(rot, Quaternion { x: 0.0, y: 0.6, z: 0.0, w: 0.8 });
        assert_eq!(scale, Vector3f { x: 1.0, y: 2.0, z: 0.5 });
        assert_eq!(t[0].prs().0, Vector3f::ZERO);
    }

    #[test]
    fn resolves_animal_corpse_model_skin_rotten_and_skeleton() {
        use pz_scripts::ScriptObject;
        use pz_world::save::lua_table::{LuaKey, LuaTable, LuaValue};
        use pz_world::save::visual::AnimalCorpse;

        let sm = ScriptManager::new();
        sm.parse_script(pz_scripts::ScriptLoadMode::Init, "module Base {} ")
            .unwrap();
        let module = sm.get_module("Base").unwrap();

        let mut anim = AnimationsMesh::new();
        anim.base.set_module(&module);
        anim.init_load_pp(&sm, "TestAnimalAnim");
        anim.mesh_file = Some("Skinned/TestAnimalBody".into());
        anim.animation_directories = vec!["TestAnimalDir".into()];
        sm.add_runtime_script(pz_scripts::ScriptRef::new(anim)).unwrap();

        let mut body_ms = ModelScript::new();
        body_ms.base.set_module(&module);
        body_ms.init_load_pp(&sm, "TestAnimalBody");
        body_ms.name = Some("TestAnimalBody".into());
        body_ms.mesh_name = Some("Skinned/TestAnimalBody".into());
        body_ms.shader_name = Some("animalEffect".into());
        body_ms.is_static = false;
        body_ms.animations_mesh = Some("TestAnimalAnim".into());
        sm.add_runtime_script(pz_scripts::ScriptRef::new(body_ms))
            .unwrap();

        let mut skel_ms = ModelScript::new();
        skel_ms.base.set_module(&module);
        skel_ms.init_load_pp(&sm, "TestAnimalSkel");
        skel_ms.name = Some("TestAnimalSkel".into());
        skel_ms.mesh_name = Some("Skinned/TestAnimalSkel".into());
        skel_ms.is_static = false;
        skel_ms.animations_mesh = Some("TestAnimalAnim".into());
        sm.add_runtime_script(pz_scripts::ScriptRef::new(skel_ms))
            .unwrap();

        let mut def = pz_animals::AnimalDefinitions::default();
        def.animal_type_str = Some("testanimal".into());
        def.body_model_str = Some("TestAnimalBody".into());
        def.body_model_skel_str = Some("TestAnimalSkel".into());
        def.texture_skeleton = Some("TestAnimalSkeletonTex".into());
        def.texture_rotten = Some("TestAnimalRottenDef".into());
        def.animset = Some("testanimset".into());
        let defs: HashMap<String, pz_animals::AnimalDefinitions> =
            [("testanimal".to_string(), def)].into();

        let mut b = IsoDeadBody::for_tests(4.0, 5.0, 0.0);
        b.animal = Some(AnimalCorpse {
            animal_type: "testanimal".into(),
            size: 0.75,
            genome: Vec::new(),
            disorders: Vec::new(),
            custom_name: String::new(),
            corpse_item: String::new(),
            weight: 10.0,
            inv_icon: String::new(),
            shadow: [1.0, 1.0, 1.0],
        });
        b.visual = CorpseVisual::Animal {
            skin_texture_name: "TestAnimalCoat".into(),
            rot_stage: 0,
        };
        let sq = CorpseSquare {
            outside: true,
            room: false,
            lights: Vec::new(),
            light: [1.0; 4],
            world_age_days: 0.0,
        };

        let fresh = resolve_animal_corpse(&b, "TestAnimalCoat", 0, &sq, &defs, &sm).unwrap();
        assert!(fresh.dead.animal);
        assert_eq!(fresh.dead.animal_size, 0.75);
        assert_eq!(fresh.dead.anim_state(), ("testanimset", "deadbody", 0.0));
        assert_eq!(fresh.mesh_name, "Skinned/TestAnimalBody");
        assert_eq!(fresh.skeleton_mesh, "Skinned/TestAnimalBody");
        assert_eq!(fresh.anim_dirs, vec!["TestAnimalDir"]);
        assert_eq!(fresh.skin_path, "Body/TestAnimalCoat");
        assert_eq!(fresh.shader_name, "animalEffect");

        b.rotten_texture = "TestAnimalRottenSaved".into();
        let rotten = resolve_animal_corpse(&b, "TestAnimalCoat", 1, &sq, &defs, &sm).unwrap();
        assert_eq!(rotten.skin_path, "Body/TestAnimalRottenSaved");

        let mut md = LuaTable::default();
        md.entries.push((LuaKey::Str("skeleton".into()), LuaValue::Str("true".into())));
        b.base.mod_data = Some(md);
        let skel = resolve_animal_corpse(&b, "TestAnimalCoat", 2, &sq, &defs, &sm).unwrap();
        assert!(skel.dead.skeleton);
        assert_eq!(skel.model_name, "TestAnimalSkel");
        assert_eq!(skel.mesh_name, "Skinned/TestAnimalSkel");
        assert_eq!(skel.skeleton_mesh, "Skinned/TestAnimalBody");
        assert_eq!(skel.skin_path, "Body/TestAnimalSkeletonTex");
        assert_eq!(skel.shader_name, "animalEffect");
        assert_ne!(
            animal_body_key(&fresh.dead, Some(&fresh.model_name), Some(&fresh.skin_texture), &[]),
            animal_body_key(&skel.dead, Some(&skel.model_name), Some(&skel.skin_texture), &[]),
        );
    }
}
