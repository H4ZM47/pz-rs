//! `IsoMannequin` rendering in the build-42 renderer:
//! - poses an `AnimationPlayer` on the mannequin's body skeleton with
//!   `pz_character_model::mannequin::pose`,
//! - builds its `ModelSlot` (script skin texture `Body/<texture>` plus its
//!   worn clothing visuals) and queues `SmartTextureJob`s for
//!   [`WorldModels::prepare`],
//! - emits `pz_character_model::mannequin::chunk_models` into
//!   `FrameObjects.chunk_models` on frames its `ChunkTexture` is recorded,
//! - and emits `Mannequin::shadow(light)` every frame for
//!   `FrameObjects.mannequins`.

use std::collections::HashMap;

use pz_anim::{AnimConditionType, AnimationPlayer, AnimationSet};
use pz_character_model::data::{HumanModel, ResolvedItemVisual};
use pz_character_model::items::EquippedModels;
use pz_character_model::mannequin::{chunk_models, pose, Mannequin};
use pz_character_model::model_tree::{CharacterModelInput, ModelSlot};
use pz_character_model::texture_creator::{create_textures, SmartTextureJob, TextureCreatorInput};
use pz_inventory::SavedItem;
use pz_iso_render::chunk::ChunkTexture;
use pz_iso_render::objects::{ChunkModels, ShadowRequest};
use pz_scripts::objects::items::ItemBodyLocation;
use pz_scripts::registry::ResourceLocation;
use pz_world::objects::IsoMannequin;
use pz_world::save::visual::HumanVisual;

use super::{retire_slot, WorldModels};

/// World-space identity of a cached mannequin model.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MannequinId {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub object_id: u32,
}

/// What the square gives a mannequin's render.
pub struct MannequinSquare {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub object_id: u32,
    pub surfaces_below: Vec<f32>,
    pub surfaces_all: Vec<f32>,
    pub outside: bool,
    pub room: bool,
    /// Interpolated square light `(r, g, b, a)`.
    pub light: [f32; 4],
    /// Highlight tint when highlighted, else `[1.0; 3]`.
    pub tint: [f32; 3],
    pub alpha: f32,
    pub world_age_days: f32,
}

/// Output of [`WorldModels::render_mannequin`] for one mannequin in one frame.
pub struct MannequinDraw {
    /// Model draw into the mannequin's dirty chunk texture, when ready.
    pub chunk_models: Option<ChunkModels>,
    /// Drop shadow emitted every frame (`renderMannequinShadows`).
    pub shadow: ShadowRequest,
    /// True while the mannequin's slot textures are still waiting on
    /// [`WorldModels::prepare`]; the caller should invalidate the chunk once
    /// more so the next frame records the mannequin into the chunk texture.
    pub pending_textures: bool,
}

#[derive(PartialEq)]
struct MannequinSignature {
    dir: u8,
    female: bool,
    zombie: bool,
    skeleton: bool,
    model_script: Option<String>,
    anim_set: String,
    anim_state: String,
    pose: String,
    human: HumanVisual,
    worn: Vec<ResolvedItemVisual>,
    simple_clothing: bool,
}

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

struct CachedMannequin {
    signature: MannequinSignature,
    slot: ModelSlot,
    player: AnimationPlayer,
    textures: Option<TextureInput>,
    jobs: Option<Vec<SmartTextureJob>>,
}

/// Cached posed mannequins owned by [`WorldModels`].
#[derive(Default)]
pub(super) struct Mannequins {
    entries: HashMap<MannequinId, CachedMannequin>,
}

impl Mannequins {
    /// Build `TextureCreator` jobs for newly built/rebuilt mannequins before
    /// [`WorldModels::prepare`] runs them on the combiner.
    pub(super) fn create_textures(
        &mut self,
        a: &crate::ui_models::Assets,
        fs: &pz_fs::GameFs,
    ) -> Result<(), String> {
        if self.entries.values().all(|m| m.textures.is_none()) {
            return Ok(());
        }
        let chars = pz_lua_api::bind_characters::shared_characters();
        let (group, hair_colors) = {
            let mut c = chars
                .try_borrow_mut()
                .map_err(|_| "body-location registry is already borrowed")?;
            let group = c
                .body_locations
                .get_group("Human")
                .map_err(|e| e.to_string())?;
            let colors: Vec<[f32; 3]> = c
                .survivor_desc
                .hair_common_colors
                .iter()
                .map(|c| c.rgb())
                .collect();
            (group, colors)
        };
        let decals = |name: &str| {
            a.pop
                .clothing_decals
                .as_ref()
                .and_then(|d| d.get_decal(fs, name))
        };
        for entry in self.entries.values_mut() {
            let Some(mut t) = entry.textures.take() else {
                continue;
            };
            let styles = a.pop.styles(&hair_colors, t.world_age_days);
            let human = HumanModel::resolve(
                &mut t.visual,
                t.female,
                t.zombie,
                t.skeleton,
                &styles,
                &t.body_visuals,
            );
            let jobs = create_textures(
                &mut entry.slot,
                &TextureCreatorInput {
                    human: &human,
                    item_visuals: &t.item_visuals,
                    body_locations: &group.borrow(),
                    decals: &decals,
                    simple_clothing_textures: t.simple_clothing,
                },
            );
            entry.jobs = Some(jobs.jobs);
        }
        Ok(())
    }

    pub(super) fn slots_mut(
        &mut self,
    ) -> impl Iterator<Item = (&mut ModelSlot, &mut Option<Vec<SmartTextureJob>>)> {
        self.entries
            .values_mut()
            .map(|m| (&mut m.slot, &mut m.jobs))
    }

    pub(super) fn retire(&mut self, textures: &mut Vec<pz_render::Texture>) {
        for (_, m) in self.entries.drain() {
            retire_slot(m.slot, textures);
        }
    }
}

/// `IsoMannequin.validatePose`: ensures `pose` matches a `Pose` condition in
/// `set`'s `state_name`, falling back to the first matching pose node or
/// `"pose01"`.
pub(crate) fn validate_pose(set: &AnimationSet, state_name: &str, requested: Option<&str>) -> String {
    let req = requested.map(str::trim).filter(|s| !s.is_empty());
    let Some(state) = set.states.get(&state_name.to_lowercase()) else {
        return req.unwrap_or("pose01").to_string();
    };
    let mut first_pose: Option<&str> = None;
    for node in &state.nodes {
        for cond in &node.conditions {
            if cond.name.eq_ignore_ascii_case("Pose")
                && matches!(cond.ty, AnimConditionType::String)
            {
                let val = if !cond.string_value.is_empty() {
                    cond.string_value.as_str()
                } else {
                    cond.value.as_str()
                };
                if val.is_empty() {
                    continue;
                }
                if first_pose.is_none() {
                    first_pose = Some(val);
                }
                if let Some(r) = req {
                    if val.eq_ignore_ascii_case(r) {
                        return val.to_string();
                    }
                }
            }
        }
    }
    first_pose
        .or(req)
        .unwrap_or("pose01")
        .to_string()
}

/// Build the [`Mannequin`] descriptor from an [`IsoMannequin`] and its square.
pub(crate) fn build_mannequin(
    man: &IsoMannequin,
    sq: &MannequinSquare,
    pose_name: String,
) -> Mannequin {
    let anim_set = man
        .anim_set
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("mannequin")
        .to_string();
    let anim_state = man
        .anim_state
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(if man.female { "female" } else { "male" })
        .to_string();
    Mannequin {
        x: sq.x,
        y: sq.y,
        z: sq.z,
        dir: man.dir.max(0) as u8,
        female: man.female,
        anim_set,
        anim_state,
        pose: pose_name,
        surfaces_below: sq.surfaces_below.clone(),
        surfaces_all: sq.surfaces_all.clone(),
        outside: sq.outside,
        room: sq.room,
        ambient: [sq.light[0], sq.light[1], sq.light[2]],
        tint: sq.tint,
        alpha: sq.alpha,
    }
}

fn resolved_skin_name(man: &IsoMannequin) -> Option<String> {
    let raw = man
        .texture_name
        .as_deref()
        .or(man.human_visual.skin_texture_name.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let stripped = raw
        .strip_prefix("Body/")
        .or_else(|| raw.strip_prefix("body/"))
        .unwrap_or(raw);
    let stripped = stripped.strip_suffix(".png").unwrap_or(stripped);
    Some(stripped.to_string())
}

impl WorldModels {
    /// Render one `IsoMannequin`: always produces its drop shadow for
    /// `FrameObjects.mannequins`, and when `dirty_texture` is `Some(tex)` and
    /// the mannequin's textures are ready, emits `mannequin::chunk_models`
    /// into `FrameObjects.chunk_models`.
    pub fn render_mannequin(
        &mut self,
        man: &mut IsoMannequin,
        sq: &MannequinSquare,
        dirty_texture: Option<ChunkTexture>,
        cam_xy: [f32; 2],
    ) -> Result<MannequinDraw, String> {
        if self.assets.is_none() {
            self.assets = Some(
                crate::ui_models::Assets::load(&self.game_dir, &self.fs, &self.tm)
                    .ok_or("character media index could not be loaded")?,
            );
        }
        let a = self.assets.as_mut().expect("assets installed above");
        let anim_set_name = man
            .anim_set
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("mannequin")
            .to_string();
        let anim_state_name = man
            .anim_state
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(if man.female { "female" } else { "male" })
            .to_string();
        let set = a.anim_set(&anim_set_name);
        let validated_pose = validate_pose(&set, &anim_state_name, man.pose.as_deref());
        let desc = build_mannequin(man, sq, validated_pose.clone());
        let shadow = desc.shadow(sq.light);

        // Collect worn clothing visuals (runtime worn items -> archive worn items -> item_visuals).
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let env = pz_lua_api::bind_items::item_env();
        let item_assets = env.context().assets();
        let mut visuals = Vec::new();
        let mut add_item_visual =
            |location: Option<&str>, item: &mut pz_inventory::InventoryItem| -> Result<(), String> {
                let Some(assets) = item_assets else {
                    return Ok(());
                };
                let key = location
                    .and_then(|loc| ResourceLocation::of(loc).ok())
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
        if let Some(worn) = &man.runtime_worn_items {
            let worn = worn
                .try_borrow()
                .map_err(|_| "mannequin worn items are borrowed")?;
            for entry in worn.items() {
                let loc = entry.location.as_ref().map(|l| l.location.id());
                let Ok(mut item) = entry.item.try_borrow_mut() else {
                    return Err("a mannequin worn item is borrowed".into());
                };
                add_item_visual(loc.as_deref(), &mut item)?;
            }
        } else if let Some(container) = man.container.as_mut() {
            for (location, index) in &man.worn_items {
                let Some(SavedItem::Item(item)) = container.items.get_mut(*index as usize) else {
                    continue;
                };
                add_item_visual(Some(location.as_str()), item)?;
            }
        } else {
            for v in &man.item_visuals {
                visuals.push(ResolvedItemVisual {
                    visual: v.clone(),
                    clothing_item: a.clothing_item(&v.clothing_item_name),
                    body_location: None,
                });
            }
        }

        let mut hv = man.human_visual.clone();
        if let Some(skin) = resolved_skin_name(man) {
            hv.skin_texture_name = Some(skin);
        }
        let simple_clothing = pz_lua_api::bind_core::shared_core()
            .try_borrow()
            .is_ok_and(|c| c.is_option_simple_clothing_textures(man.zombie));
        let model_script = man
            .model_script_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let signature = MannequinSignature {
            dir: desc.dir,
            female: man.female,
            zombie: man.zombie,
            skeleton: man.skeleton,
            model_script: model_script.clone(),
            anim_set: anim_set_name,
            anim_state: anim_state_name,
            pose: validated_pose,
            human: hv.clone(),
            worn: visuals.clone(),
            simple_clothing,
        };
        let id = MannequinId {
            x: sq.x,
            y: sq.y,
            z: sq.z,
            object_id: sq.object_id,
        };
        let needs_rebuild = self
            .mannequins
            .entries
            .get(&id)
            .is_none_or(|e| e.signature != signature);
        if needs_rebuild {
            if let Some(old) = self.mannequins.entries.remove(&id) {
                retire_slot(old.slot, &mut self.retired_textures);
            }
            let sd = a.skeleton(man.female, &set).ok_or_else(|| {
                format!(
                    "mannequin body skeleton is unavailable ({}, female={})",
                    desc.anim_set, man.female
                )
            })?;
            let mut player = AnimationPlayer::new(sd);
            pose(&mut player, set, &desc);
            let chars = pz_lua_api::bind_characters::shared_characters();
            let (group, hair_colors) = {
                let mut c = chars
                    .try_borrow_mut()
                    .map_err(|_| "body-location registry is already borrowed")?;
                let group = c
                    .body_locations
                    .get_group("Human")
                    .map_err(|e| e.to_string())?;
                let colors: Vec<[f32; 3]> = c
                    .survivor_desc
                    .hair_common_colors
                    .iter()
                    .map(|c| c.rgb())
                    .collect();
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
            let human = HumanModel::resolve(
                &mut hv,
                man.female,
                man.zombie,
                man.skeleton,
                &styles,
                &body_visuals,
            );
            let bone_index = |n: &str| player.skinning_bone_index(n, -1);
            let body_model_script = model_script.as_deref().and_then(|name| {
                sm.get::<pz_scripts::objects::models::ModelScript>(
                    pz_scripts::ScriptType::Model,
                    name,
                )
            });
            let slot = ModelSlot::try_build(&CharacterModelInput {
                human: &human,
                item_visuals: &visuals,
                equipped: &EquippedModels::default(),
                body_locations: &group.borrow(),
                hair_styles: a.pop.hair_styles.as_ref(),
                beard_styles: a.pop.beard_styles.as_ref(),
                scripts: &*sm,
                body_model_script,
                bone_index: &bone_index,
            })
            .map_err(|e| e.to_string())?;
            drop(human);
            let textures = TextureInput {
                visual: hv,
                body_visuals,
                item_visuals: visuals,
                female: man.female,
                zombie: man.zombie,
                skeleton: man.skeleton,
                simple_clothing,
                world_age_days: sq.world_age_days,
            };
            self.mannequins.entries.insert(
                id,
                CachedMannequin {
                    signature,
                    slot,
                    player,
                    textures: Some(textures),
                    jobs: None,
                },
            );
            return Ok(MannequinDraw {
                chunk_models: None,
                shadow,
                pending_textures: true,
            });
        }

        let entry = self
            .mannequins
            .entries
            .get(&id)
            .expect("mannequin entry checked above");
        if entry.textures.is_some() || entry.jobs.is_some() {
            return Ok(MannequinDraw {
                chunk_models: None,
                shadow,
                pending_textures: true,
            });
        }
        let cm = dirty_texture.map(|tex| {
            chunk_models(&desc, &entry.slot, &entry.player, &a.res, tex, cam_xy)
        });
        Ok(MannequinDraw {
            chunk_models: cm,
            shadow,
            pending_textures: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_anim::{AnimCondition, AnimNode, AnimState};
    use std::rc::Rc;

    #[test]
    fn validate_pose_matches_state_node_or_falls_back_to_first() {
        let mut set = AnimationSet {
            name: "mannequin".into(),
            states: HashMap::new(),
        };
        let mut state = AnimState::new("female");
        let empty = pz_anim::xml::parse("<animNode/>").unwrap();
        for p in ["pose01", "pose02", "pose03"] {
            let mut node = AnimNode::from_element(&empty);
            let mut cond = AnimCondition::default();
            cond.name = "Pose".into();
            cond.ty = AnimConditionType::String;
            cond.value = p.into();
            cond.string_value = p.into();
            node.conditions.push(cond);
            state.nodes.push(Rc::new(node));
        }
        set.states.insert("female".into(), Rc::new(state));

        assert_eq!(validate_pose(&set, "female", Some("pose02")), "pose02");
        assert_eq!(validate_pose(&set, "female", Some("POSE03")), "pose03");
        assert_eq!(validate_pose(&set, "female", Some("invalid")), "pose01");
        assert_eq!(validate_pose(&set, "female", None), "pose01");
    }

    #[test]
    fn resolved_skin_strips_body_prefix_and_png() {
        let mut m = IsoMannequin::new(18);
        m.texture_name = Some("Body/F_Mannequin_White.png".into());
        assert_eq!(resolved_skin_name(&m).as_deref(), Some("F_Mannequin_White"));
        m.texture_name = Some("M_Mannequin_Black".into());
        assert_eq!(resolved_skin_name(&m).as_deref(), Some("M_Mannequin_Black"));
    }

    #[test]
    fn build_mannequin_and_shadow_preserve_surfaces_and_light() {
        let mut m = IsoMannequin::new(18);
        m.female = true;
        m.dir = 4; // S
        let sq = MannequinSquare {
            x: 10,
            y: 20,
            z: 0,
            object_id: 9,
            surfaces_below: vec![30.0],
            surfaces_all: vec![30.0, 12.0],
            outside: false,
            room: true,
            light: [0.8, 0.7, 0.6, 0.9],
            tint: [1.0; 3],
            alpha: 1.0,
            world_age_days: 0.0,
        };
        let desc = build_mannequin(&m, &sq, "pose02".into());
        assert_eq!(desc.anim_set, "mannequin");
        assert_eq!(desc.anim_state, "female");
        assert_eq!(desc.pose, "pose02");
        assert_eq!(desc.surfaces_below, [30.0]);
        assert!(matches!(desc.shadow(sq.light), ShadowRequest::Character { .. }));
    }
}
