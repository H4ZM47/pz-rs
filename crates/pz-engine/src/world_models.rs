//! Live character models shared by simulation and every camera.
//!
//! Animation advances once in the simulation, with the actual character as
//! its variable source. Rendering only reads that pose: a second local
//! camera must not advance tracks or fire combat events a second time.
//! Models rebuild when the visual or equipment changes, preserving tracks;
//! changing sex or animation set replaces the rig. Animals and ragdoll
//! poses still need their own model producers.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_anim::{AdvancedAnimator, AnimEventRecord, AnimationPlayer};
use pz_character_model::combine::run_jobs;
use pz_character_model::data::{HumanModel, HumanVisual, ResolvedItemVisual};
use pz_character_model::items::{ActionHandModels, AttachedModel, EquippedModels, HeldItem};
use pz_character_model::lights::{LightContext, PlayerLightData};
use pz_character_model::model_tree::{CharacterModelInput, ModelSlot, TextureSource};
use pz_character_model::scene::{
    character_shadow, render_character_object, ShadowInput, ShadowSmoothing,
};
use pz_character_model::texture_creator::{create_textures, SmartTextureJob, TextureCreatorInput};
use pz_characters::character::CharacterKind;
use pz_fs::GameFs;
use pz_inventory::visual::human::HumanVisualOwner;
use pz_iso_render::models::ModelCamera;
use pz_iso_render::objects::ObjectRender;
use pz_lua_api::bind_character_sim::LuaGameCharacter;
use pz_render::combiner::Combiner;
use pz_render::{flags, Texture, TextureManager};

use crate::ui_models::Assets;

/// Camera-dependent character state. Lighting comes from the actual
/// square's lighting result, rather than a fixed white model light.
pub struct CharacterView<'a> {
    pub camera: &'a ModelCamera,
    pub camera_index: usize,
    pub lighting: &'a LightContext,
    pub alpha: f32,
    pub hidden: bool,
    pub height_above_floor: f32,
    pub tick: i64,
    pub thirty_fps_multiplier: f32,
}

#[derive(PartialEq)]
struct Appearance {
    human: HumanVisual,
    worn: Vec<ResolvedItemVisual>,
    // The equipment model helpers expose all the values used to select
    // models and initialise their textures. Keep the full values, so a
    // visual change does not disappear through a lossy hash.
    equipment: String,
    simple_clothing: bool,
    simple_weapon: bool,
}

struct CharacterModel {
    female: bool,
    zombie: bool,
    skeleton: bool,
    set_name: String,
    player: AnimationPlayer,
    animator: AdvancedAnimator,
    appearance: Option<Appearance>,
    slot: Option<ModelSlot>,
    jobs: Option<Vec<SmartTextureJob>>,
    lights: HashMap<usize, PlayerLightData>,
    shadow: HashMap<usize, ShadowSmoothing>,
    initial_turn_around_target: f32,
}

/// `ModelManager`'s live character slots, indexed by the world's character
/// ids. The world must retire ids when characters leave, before reusing an
/// id. Keep this manager across menu/world transitions to reuse assets;
/// [`Self::clear`] releases the world's slots and animation state.
pub struct WorldModels {
    game_dir: PathBuf,
    fs: Rc<RefCell<GameFs>>,
    tm: Rc<RefCell<TextureManager>>,
    assets: Option<Assets>,
    characters: HashMap<i32, CharacterModel>,
    retired_textures: Vec<Texture>,
}

impl WorldModels {
    pub fn new(game_dir: &Path, fs: Rc<RefCell<GameFs>>, tm: Rc<RefCell<TextureManager>>) -> Self {
        Self {
            game_dir: game_dir.to_path_buf(),
            fs,
            tm,
            assets: None,
            characters: HashMap::new(),
            retired_textures: Vec::new(),
        }
    }

    /// Load or rebuild the slot and advance the actual character's
    /// animation by `dt` seconds. Call once per simulation frame, never
    /// once per rendered camera. Route every returned event through
    /// `GameCharacter::on_anim_event` with the real `StateCx`; the animator
    /// itself has already applied variable-setting events.
    ///
    /// The first model build reads the shared body-location registry.
    /// Call without holding a `shared_characters()` borrow; event dispatch
    /// can acquire its `StateCx` afterwards. Missing assets remain an
    /// explicit error and are retried, not an invented idle pose.
    pub fn update_character(
        &mut self,
        chr: &mut LuaGameCharacter,
        dt: f32,
        world_age_days: f32,
    ) -> Result<Vec<AnimEventRecord>, String> {
        self.update_character_before_action_modifiers(chr, dt, world_age_days)?;
        self.finish_character_update(chr, dt, None)
    }

    /// Stop before the Lua timed-action movement modifier callback.
    pub fn update_character_before_action_modifiers(
        &mut self,
        chr: &mut LuaGameCharacter,
        dt: f32,
        world_age_days: f32,
    ) -> Result<(), String> {
        if chr.kind == CharacterKind::Animal {
            return Err("animal model producer is not connected".into());
        }
        if self.assets.is_none() {
            self.assets = Some(
                Assets::load(&self.game_dir, &self.fs, &self.tm)
                    .ok_or("character media index could not be loaded")?,
            );
        }
        let assets = self.assets.as_mut().expect("assets installed above");
        let zombie = chr.is_zombie();
        let female = if zombie {
            chr.zombie.female
        } else {
            chr.is_female()
        };
        let skeleton = chr.is_skeleton();
        let set_name = if zombie {
            chr.zombie_anim_set_name()
        } else {
            "player"
        };
        let replace = self.characters.get(&chr.id).is_none_or(|m| {
            m.female != female
                || m.zombie != zombie
                || m.skeleton != skeleton
                || m.set_name != set_name
        });
        if replace {
            let set = assets.anim_set(set_name);
            let sd = assets.skeleton(female, &set).ok_or_else(|| {
                format!("character body skeleton is unavailable ({set_name}, female={female})")
            })?;
            let mut player = AnimationPlayer::new(sd);
            player.set_target_and_current_direction(chr.vars.forward_x, chr.vars.forward_y);
            let mut animator = AdvancedAnimator::new();
            animator.set_anim_set(Some(set));
            let model = CharacterModel {
                female,
                zombie,
                skeleton,
                set_name: set_name.into(),
                player,
                animator,
                appearance: None,
                slot: None,
                jobs: None,
                lights: HashMap::new(),
                shadow: HashMap::new(),
                initial_turn_around_target: 0.0,
            };
            if let Some(old) = self.characters.insert(chr.id, model) {
                retire(old, &mut self.retired_textures);
            }
        }
        let model = self
            .characters
            .get_mut(&chr.id)
            .expect("model inserted above");
        rebuild_if_changed(
            assets,
            model,
            chr,
            world_age_days,
            &self.fs,
            &mut self.retired_textures,
        )?;
        let state = chr.state.action_context.current_state_name();
        let children: Vec<String> = chr
            .state
            .action_context
            .child_states()
            .iter()
            .map(|s| s.name().to_string())
            .collect();
        let children: Vec<&str> = children.iter().map(String::as_str).collect();
        model
            .animator
            .set_state(&mut model.player, chr, &state, &children);
        update_direction_and_turning(model, chr);
        publish_animation_variables(model, chr);
        model.animator.update(&mut model.player, chr, dt);
        chr.wait_to_start_timed_action = model.animator.should_wait_for_sit_action();
        Ok(())
    }

    pub fn action_movement_modifiers(&self, chr: &LuaGameCharacter) -> Option<pz_characters::character::state::MoveDeltaModifiers> {
        self.characters.get(&chr.id).map(|model| base_deltas(model, chr))
    }

    /// Resume once the host releases its borrows and calls the Lua modifier.
    pub fn finish_character_update(
        &mut self,
        chr: &mut LuaGameCharacter,
        dt: f32,
        modifiers: Option<pz_characters::character::state::MoveDeltaModifiers>,
    ) -> Result<Vec<AnimEventRecord>, String> {
        let model = self.characters.get_mut(&chr.id).ok_or("character model is not initialized")?;
        apply_deltas(model, chr, modifiers);
        model.player.update(dt, Some(&*chr));
        model.animator.dispatch_track_events(&mut model.player, chr);
        // Callback animation variables read the real player's result on
        // the next simulation/action-context update.
        publish_animation_variables(model, chr);
        Ok(std::mem::take(&mut model.animator.events))
    }

    /// The state world's thump/root-motion/turn-alerted hooks must use
    /// these same objects, rather than a separate animation player.
    pub fn animation_mut(
        &mut self,
        id: i32,
    ) -> Option<(&mut AnimationPlayer, &mut AdvancedAnimator)> {
        self.characters
            .get_mut(&id)
            .map(|m| (&mut m.player, &mut m.animator))
    }

    pub fn animation(&self, id: i32) -> Option<(&AnimationPlayer, &AdvancedAnimator)> {
        self.characters.get(&id).map(|m| (&m.player, &m.animator))
    }

    pub fn primary_hand_model_ready(&self, id: i32) -> bool {
        use pz_character_model::render::ModelResources;
        let Some(assets) = self.assets.as_ref() else {
            return false;
        };
        let Some(slot) = self.characters.get(&id).and_then(|m| m.slot.as_ref()) else {
            return false;
        };
        let Some(hand) = slot.primary_hand else {
            return false;
        };
        let instance = slot.instance(hand);
        assets
            .res
            .mesh(&instance.mesh_name, instance.is_static)
            .is_some()
    }

    /// Retain all world residents, including offscreen and paused
    /// characters. Call after population removals, not after culling.
    pub fn retain_characters(&mut self, ids: impl IntoIterator<Item = i32>) {
        let live: HashSet<i32> = ids.into_iter().collect();
        let retired: Vec<i32> = self
            .characters
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for id in retired {
            if let Some(model) = self.characters.remove(&id) {
                retire(model, &mut self.retired_textures);
            }
        }
    }

    /// Release removed/rebuilt slots' composited textures, then execute
    /// the current slots' texture jobs before drawing the world's models.
    pub fn prepare(&mut self, combiner: &mut dyn Combiner) {
        release_textures(&mut self.retired_textures, combiner);
        let fs = self.fs.borrow();
        let mut tm = self.tm.borrow_mut();
        let model_flags = flags::COMPRESS | flags::LIMIT_TEXTURE_SIZE_1;
        for model in self.characters.values_mut() {
            let Some(slot) = model.slot.as_mut() else {
                continue;
            };
            let mut file =
                |tm: &mut TextureManager, p: &str| tm.shared_texture_flags(&fs, p, model_flags);
            if let Some(jobs) = model.jobs.take() {
                let done = run_jobs(&jobs, &mut file, combiner, &mut tm);
                for inst in &mut slot.instances {
                    if let TextureSource::Smart(j) = inst.tex {
                        inst.tex = done
                            .get(j)
                            .cloned()
                            .flatten()
                            .map_or(TextureSource::None, TextureSource::Combined);
                    }
                }
            }
            pz_character_model::item_texture::render_equipped_textures_with(
                slot,
                &|p| fs.resolve(p).is_some(),
                &mut file,
                combiner,
                &mut tm,
            );
        }
    }

    /// Append the existing pose to the iso object's pass, with that
    /// camera's lights and eased shadow. No animation or simulation runs.
    pub fn render_character(
        &mut self,
        chr: &LuaGameCharacter,
        view: &CharacterView<'_>,
    ) -> Option<ObjectRender> {
        let assets = self.assets.as_ref()?;
        let model = self.characters.get_mut(&chr.id)?;
        if model.jobs.is_some() {
            return None;
        }
        let slot = model.slot.as_ref()?;
        if view.hidden {
            return None;
        }
        let lights = model.lights.entry(view.camera_index).or_default();
        lights.update(view.lighting);
        let frame = pz_character_model::render::CharacterFrame {
            player: &model.player,
            x: chr.vars.x,
            y: chr.vars.y,
            z: chr.vars.z,
            alpha: view.alpha,
            effect_lights: &lights.effect_lights_main,
            ambient: lights.current_ambient,
            final_scale: 1.0,
        };
        let shadow_input = ShadowInput {
            player: Some(&model.player),
            anim_angle: model.player.angle(),
            ragdoll_direction: None,
            height_above_floor: view.height_above_floor,
            alpha: view.alpha,
            skeleton: model.skeleton,
            animal_size: None,
            light: view.lighting.target_ambient,
            slope: None,
            hidden: view.hidden,
        };
        let shadow = character_shadow(
            &shadow_input,
            [chr.vars.x, chr.vars.y, chr.vars.z],
            Some((
                model.shadow.entry(view.camera_index).or_default(),
                view.tick,
                view.thirty_fps_multiplier,
            )),
        );
        Some(render_character_object(
            slot,
            &frame,
            shadow,
            view.camera,
            &assets.res,
        ))
    }

    /// World unload. `prepare` must run once more with the owning GPU to
    /// release the retired composite textures before disposing this manager.
    pub fn clear(&mut self) {
        for (_, model) in self.characters.drain() {
            retire(model, &mut self.retired_textures);
        }
    }

    /// Release the world's composites while its GPU still exists.
    pub fn dispose(&mut self, combiner: &mut dyn Combiner) {
        self.clear();
        release_textures(&mut self.retired_textures, combiner);
    }

    #[cfg(test)]
    pub(crate) fn insert_test_animation(
        &mut self,
        id: i32,
        player: AnimationPlayer,
        animator: AdvancedAnimator,
    ) {
        self.characters.insert(
            id,
            CharacterModel {
                female: false,
                zombie: false,
                skeleton: false,
                set_name: "synthetic".into(),
                player,
                animator,
                appearance: None,
                slot: None,
                jobs: None,
                lights: HashMap::new(),
                shadow: HashMap::new(),
                initial_turn_around_target: 0.0,
            },
        );
    }
}

/// The supported single-player branch of `postUpdateAnimating`'s facing
/// and `applyDeltas`. Timed-action delta overrides and the pathfinder's
/// turning-to-obstacle override still need those owners to expose them.
fn update_direction_and_turning(model: &mut CharacterModel, chr: &mut LuaGameCharacter) {
    model
        .player
        .set_target_direction(chr.vars.forward_x, chr.vars.forward_y);
    let target_twist = 57.295776 * model.player.target_twist_angle();
    let twisting = target_twist.abs() > 1.0;
    let attacking = chr.is_player() && !chr.vars.attack_type.is_empty();
    let allows_twist = chr.is_player()
        && !attacking
        && !chr.vars.knocked_down
        && !chr.is_getting_up()
        && !chr.vars.performing_an_action
        && (chr.is_aiming() || !chr.is_sneaking() && twisting);
    let shoulder_weight = if chr.is_aiming() {
        1.0
    } else if chr.is_sneaking() {
        0.6
    } else {
        0.75
    };
    model.player.update_forward_direction(
        chr.vars.forward_x,
        chr.vars.forward_y,
        allows_twist,
        shoulder_weight,
    );
    let turning = if chr.is_player() && model.player.tracks().is_empty()
        || chr.is_zombie()
            && chr.is_current_state_id(pz_characters::character::states::ids::ZOMBIE_FALL_DOWN)
        || chr.vars.block_turning
    {
        false
    } else if chr.current_state().is_some_and(|s| s.is_moving(chr)) || chr.player_moving {
        twisting
    } else if attacking {
        !chr.vars.aim_at_floor
    } else {
        57.295776 * model.player.excess_twist_angle().abs() > 1.0 || chr.vars.turning && twisting
    };
    chr.vars.turning = turning;
    chr.vars.turning90 = turning && (chr.vars.turning90 || target_twist.abs() > 65.0);
    let was_around = chr.vars.turning_around;
    let around = turning && target_twist.abs() > if was_around { 45.0 } else { 110.0 };
    chr.vars.turning_around = around;
    if around && !was_around {
        model.animator.invoke_global_anim_event("Turn180Started");
        model.initial_turn_around_target = chr.direction_angle();
    } else if around && was_around {
        let difference = pz_anim::math::pzmath::get_closest_angle(
            model.initial_turn_around_target * (std::f32::consts::PI / 180.0),
            chr.direction_angle() * (std::f32::consts::PI / 180.0),
        );
        if (57.295776 * difference).abs() > 90.0 {
            model
                .animator
                .invoke_global_anim_event("Turn180TargetChanged");
            chr.vars.turning_around = false;
        }
    }
}

fn base_deltas(model: &CharacterModel, chr: &LuaGameCharacter) -> pz_characters::character::state::MoveDeltaModifiers {
    let mut deltas = pz_characters::character::state::MoveDeltaModifiers {
        move_delta: chr.vars.move_delta,
        turn_delta: if model.zombie {
            chr.zombie_turn_delta()
        } else {
            chr.turn_delta()
        },
        twist_delta: -1.0,
    };
    let has_path = chr.path.path2.is_some();
    if chr.is_player()
        && has_path
        && chr.is_running(&pz_lua_api::bind_registry::shared_script_manager())
    {
        deltas.turn_delta = deltas.turn_delta.max(2.0);
    }
    if let Some(state) = chr.current_state() {
        state.get_delta_modifiers(chr, &mut deltas);
    }
    deltas
}

fn apply_deltas(model: &mut CharacterModel, chr: &LuaGameCharacter, modifiers: Option<pz_characters::character::state::MoveDeltaModifiers>) {
    let mut deltas = modifiers.unwrap_or_else(|| base_deltas(model, chr));
    if deltas.twist_delta == -1.0 {
        deltas.twist_delta = deltas.turn_delta * 1.8;
    }
    if !chr.vars.turning {
        deltas.turn_delta = 0.0;
    }
    let movement_turn = (1.0 - deltas.move_delta / 2.0).max(0.0);
    model.player.angle_step_delta = movement_turn * deltas.turn_delta;
    model.player.angle_twist_delta = movement_turn * deltas.twist_delta;
    model
        .player
        .set_max_twist_angle(std::f32::consts::PI / 180.0 * chr.vars.max_twist);
}

fn publish_animation_variables(model: &CharacterModel, chr: &mut LuaGameCharacter) {
    chr.vars.anim_angle = 57.295776 * model.player.angle();
    chr.vars.twist = 57.295776 * model.player.twist_angle();
    chr.vars.target_twist = 57.295776 * model.player.target_twist_angle();
    chr.vars.shoulder_twist = 57.295776 * model.player.shoulder_twist_angle();
    chr.vars.excess_twist = 57.295776 * model.player.excess_twist_angle();
    chr.vars.num_twist_bones = model.player.num_twist_bones() as i32;
    chr.vars.angle_step_delta = model.player.angle_step_delta;
    chr.vars.angle_twist_delta = model.player.angle_twist_delta;
}

fn retire(model: CharacterModel, textures: &mut Vec<Texture>) {
    if let Some(slot) = model.slot {
        retire_slot(slot, textures);
    }
}

fn retire_slot(slot: ModelSlot, textures: &mut Vec<Texture>) {
    for inst in slot.instances {
        if let TextureSource::Combined(t) = inst.tex {
            textures.push(t);
        }
        if let Some(t) = inst.equipped_texture.texture {
            textures.push(t);
        }
    }
}

fn release_textures(textures: &mut Vec<Texture>, combiner: &mut dyn Combiner) {
    let mut released = HashSet::new();
    for t in textures.drain(..) {
        // Several slot instances can refer to the same composite job.
        if released.insert((t.id, t.name.clone())) {
            combiner.release(t);
        }
    }
}

fn equipment_key(
    e: &EquippedModels<'_>,
    scripts: &dyn pz_character_model::model_tree::ModelScripts,
) -> String {
    fn key(h: &HeldItem<'_>, scripts: &dyn pz_character_model::model_tree::ModelScripts) -> String {
        format!(
            "{:?}",
            (
                h.item.id,
                h.static_model(scripts),
                h.static_model_exception(scripts),
                h.texture_init(),
                h.visual.as_ref(),
                h.replacement_primary_hand(),
                h.replacement_second_hand(),
                h.weapon_part_models()
            )
        )
    }
    format!(
        "{:?}",
        (
            e.primary.as_ref().map(|h| key(h, scripts)),
            e.secondary.as_ref().map(|h| key(h, scripts)),
            e.hide_weapon_model,
            e.action.as_ref().map(|a| (a.primary_item.as_ref().map(|h| key(h, scripts)), a.secondary_item.as_ref().map(|h| key(h, scripts)), &a.primary_model, &a.secondary_model)),
            e.attached
                .iter()
                .map(|a| (&a.attachment_name, key(&a.item, scripts)))
                .collect::<Vec<_>>()
        )
    )
}

fn rebuild_if_changed(
    a: &mut Assets,
    model: &mut CharacterModel,
    chr: &mut LuaGameCharacter,
    world_age_days: f32,
    fs: &Rc<RefCell<GameFs>>,
    retired: &mut Vec<Texture>,
) -> Result<(), String> {
    let mut hv = chr
        .human_visual
        .clone()
        .ok_or("character has no HumanVisual")?;
    let env = pz_lua_api::bind_items::item_env();
    let item_assets = env.context().assets();
    let visuals = if model.zombie {
        chr.zombie
            .item_visuals
            .iter()
            .map(|v| ResolvedItemVisual {
                visual: v.clone(),
                clothing_item: a.clothing_item(&v.clothing_item_name),
                body_location: None,
            })
            .collect()
    } else {
        let mut visuals = Vec::new();
        for id in chr.items.worn_ids() {
            let Some(item) = chr.items.item(id) else {
                continue;
            };
            let Some(assets) = item_assets else {
                return Err("worn-item visual assets are unavailable".into());
            };
            let loc = chr.items.worn_location(id);
            if let Some(v) = ResolvedItemVisual::from_item(
                &mut item.borrow_mut(),
                assets,
                chr,
                env.visual_rng(),
                loc,
            ) {
                visuals.push(v);
            };
        }
        visuals
    };
    let primary = chr.items.primary_hand_item();
    let secondary = chr.items.secondary_hand_item();
    let action = chr.character_actions.borrow().first().and_then(|a| {
        let a = a.borrow();
        a.override_hand_models.then(|| (a.primary_item.clone(), a.secondary_item.clone(), a.primary_model.clone(), a.secondary_model.clone()))
    });
    let action_primary = action.as_ref().and_then(|a| a.0.as_ref()).map(|i| i.borrow());
    let action_secondary = action.as_ref().and_then(|a| a.1.as_ref()).map(|i| i.borrow());
    let pb = primary.as_ref().map(|i| i.borrow());
    let sb = secondary.as_ref().map(|i| i.borrow());
    let held = |i| {
        let mut h = HeldItem::new(i);
        if let Some(assets) = item_assets {
            h = h.with_replacement_assets(assets);
        }
        if let Some(entity) = env.entity() {
            h = h.with_fluid(entity);
        }
        h
    };
    let chars = pz_lua_api::bind_characters::shared_characters();
    let (group, attached_group, hair_colors) = {
        let mut c = chars
            .try_borrow_mut()
            .map_err(|_| "body-location registry is already borrowed")?;
        let group = c
            .body_locations
            .get_group("Human")
            .map_err(|e| e.to_string())?;
        let attached = c
            .attached_locations
            .get_group(Some("Human"))
            .map_err(|e| e.to_string())?;
        let colors: Vec<[f32; 3]> = c
            .survivor_desc
            .hair_common_colors
            .iter()
            .map(|c| c.rgb())
            .collect();
        (group, attached, colors)
    };
    let mut equipped = EquippedModels {
        primary: pb.as_deref().map(held),
        secondary: sb.as_deref().map(held),
        hide_weapon_model: chr.vars.hide_weapon_model,
        action: action.as_ref().map(|a| ActionHandModels {
            primary_item: action_primary.as_deref().map(held),
            secondary_item: action_secondary.as_deref().map(held),
            primary_model: a.2.clone(), secondary_model: a.3.clone(),
        }),
        ..Default::default()
    };
    for (location, item) in &chr.zombie.attached_items {
        equipped.attached.push(AttachedModel {
            attachment_name: attached_group
                .borrow()
                .location(Some(location))
                .and_then(|l| l.borrow().attachment_name().map(str::to_string)),
            item: held(item),
        });
    }
    let sm = pz_lua_api::bind_registry::shared_script_manager();
    let (simple_clothing, simple_weapon) = {
        let core = pz_lua_api::bind_core::shared_core();
        let c = core.borrow();
        (
            c.is_option_simple_clothing_textures(model.zombie),
            c.get_option_simple_weapon_textures(),
        )
    };
    equipped.simple_weapon_textures = simple_weapon;
    let mut appearance = Appearance {
        human: hv.clone(),
        worn: visuals,
        equipment: equipment_key(&equipped, &*sm),
        simple_clothing,
        simple_weapon,
    };
    if model.appearance.as_ref() == Some(&appearance) {
        return Ok(());
    }
    let body_visuals: Vec<ResolvedItemVisual> = hv
        .body_visuals
        .iter()
        .map(|v| ResolvedItemVisual {
            visual: v.clone(),
            clothing_item: a.clothing_item(&v.clothing_item_name),
            body_location: None,
        })
        .collect();
    let human = HumanModel::resolve(
        &mut hv,
        model.female,
        model.zombie,
        model.skeleton,
        &a.pop.styles(&hair_colors, world_age_days),
        &body_visuals,
    );
    let bone_index = |n: &str| model.player.skinning_bone_index(n, -1);
    let mut slot = ModelSlot::build(&CharacterModelInput {
        human: &human,
        item_visuals: &appearance.worn,
        equipped: &equipped,
        body_locations: &group.borrow(),
        hair_styles: a.pop.hair_styles.as_ref(),
        beard_styles: a.pop.beard_styles.as_ref(),
        scripts: &*sm,
        body_model_script: None,
        bone_index: &bone_index,
    });
    let fs = fs.borrow();
    let decals = |name: &str| {
        a.pop
            .clothing_decals
            .as_ref()
            .and_then(|d| d.get_decal(&fs, name))
    };
    let jobs = create_textures(
        &mut slot,
        &TextureCreatorInput {
            human: &human,
            item_visuals: &appearance.worn,
            body_locations: &group.borrow(),
            decals: &decals,
            simple_clothing_textures: simple_clothing,
        },
    );
    appearance.human = hv.clone();
    // Lazy skin/hair selections belong to the live visual, so subsequent
    // frames and saving see them, and no new OutfitRNG draw repeats.
    chr.human_visual = Some(hv);
    if let Some(old) = model.slot.replace(slot) {
        retire_slot(old, retired);
    }
    model.appearance = Some(appearance);
    model.jobs = Some(jobs.jobs);
    Ok(())
}
