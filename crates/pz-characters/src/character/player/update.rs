//! `IsoPlayer.update()` for the local player (`updateInternal1` /
//! `updateInternal2`), its `IsoGameCharacter.updateInternal` part, and
//! `IsoPlayer.postupdate()`, in the original order.

use pz_anim::AnimationVariableSource;
use pz_scripts::{ScriptManager, ScriptResult};

use super::{PlayerEvent, PlayerInput, PlayerUpdate};
use crate::character::body_part::{adjust_for_framerate, next_bool};
use crate::character::state::{ActionGroups, StateCx, StateRegistry};
use crate::character::states::ids;
use crate::character::zombie::math::{fastfloor, normalize, set_length};
use crate::character::{GameCharacter, SimContext};
use crate::Characters;

/// `MoveVars`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveVars {
    pub move_x: f32,
    pub move_y: f32,
    pub strafe_x: f32,
    pub strafe_y: f32,
    pub new_facing: Option<pz_world::square::IsoDirections>,
}

/// `InputState` (what `UpdateInputState` fills).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputState {
    pub melee: bool,
    pub grapple: bool,
    pub is_attacking: bool,
    pub is_aiming: bool,
    pub is_charging: bool,
    pub is_charging_lt: bool,
    pub running: bool,
    pub sprinting: bool,
}

impl<I: 'static> GameCharacter<I> {
    fn moodle_level(&self, sm: &ScriptManager, f: &str) -> i32 {
        self.moodles.moodle_level_of(sm, f).unwrap_or(0)
    }

    fn perk_lv(&self, chars: &Characters, f: &str) -> i32 {
        self.perk_level(chars.perks.constant(f).as_ref())
    }

    fn trait_on(&self, sm: &ScriptManager, f: &str) -> bool {
        self.has_trait(sm, f).unwrap_or(false)
    }

    /// `IsoMovingObject.preupdate()` for the player (`IsoGameCharacter`'s
    /// animation time delta is the animation's).
    pub fn preupdate_player(&mut self) {
        self.moving_preupdate();
    }

    /// `isRunning()`/`isSprinting()` with the endurance and sprint checks.
    fn run_or_sprint(&self, sm: &ScriptManager, ctx: &dyn SimContext) -> bool {
        self.is_running(sm) || self.is_sprinting(sm, ctx)
    }

    /// `IsoPlayer.update()` for a local player in single player.
    pub fn update_player(
        &mut self,
        input: &dyn PlayerInput,
        registry: &mut StateRegistry<I>,
        groups: &mut ActionGroups,
        cx: &mut StateCx<'_, I>,
    ) -> ScriptResult<PlayerUpdate> {
        let mut out = PlayerUpdate::default();
        self.player.now_ms = input.current_time_ms();
        let owner = self.input_owner();
        self.vars.melee_button_down = self.player.input.is_melee_button_down(input, owner);
        self.vars.manual_floor_attack_button_down = self.player.input.is_manual_floor_atk_button_down(input, owner);
        if self.is_animal() {
            return Ok(out);
        }
        if self.update_internal2(input, registry, groups, &mut out, cx)? {
            // updateLOS: the spotting of other characters is the world's
            // (TODO(los): IsoPlayer.updateLOS).
            self.update_character_internal(&mut out, cx)?;
        }
        Ok(out)
    }

    /// `setBeenMovingSprinting()`.
    fn set_been_moving_sprinting(&mut self, sm: &ScriptManager, ctx: &dyn SimContext) {
        let m = ctx.multiplier();
        if self.player.just_moved {
            self.vars.been_moving_for += 1.25 * m;
        } else {
            self.vars.been_moving_for -= 0.625 * m;
        }
        self.vars.been_moving_for = self.vars.been_moving_for.clamp(0.0, 70.0);
        if self.player.just_moved && self.is_sprinting(sm, ctx) {
            self.vars.been_sprinting_for += 1.25 * m;
        } else {
            self.vars.been_sprinting_for = 0.0;
        }
        self.vars.been_sprinting_for = self.vars.been_sprinting_for.clamp(0.0, 100.0);
    }

    /// `IsoPlayer.updateInternal2()` (single player, local, not an NPC).
    /// `false`: the base update is skipped.
    fn update_internal2(
        &mut self,
        input: &dyn PlayerInput,
        registry: &mut StateRegistry<I>,
        groups: &mut ActionGroups,
        out: &mut PlayerUpdate,
        cx: &mut StateCx<'_, I>,
    ) -> ScriptResult<bool> {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        if (self.vars.falling || self.vars.fall_time > 0.0 || self.vars.on_fire) && cx.world.game_speed() > 1 {
            cx.ctx.event(crate::character::SimEvent::SetGameSpeed(1));
        }
        // TODO(grappling): let go of the grappled while falling.
        if !self.vars.attack_started {
            self.vars.initiate_attack = false;
            self.vars.attack_type = String::new();
        }
        let dm = cx.world.deferred_movement_length_sq(self.id);
        self.player.running_time =
            if self.run_or_sprint(sm, ctx) && dm > 0.0 { self.player.running_time + ctx.thirty_fps_multiplier() } else { 0.0 };
        if self.vars.last_collide_time > 0.0 {
            self.vars.last_collide_time -= ctx.thirty_fps_multiplier();
        }
        self.update_death_drag_down(cx);
        self.do_deferred_movement(cx);
        // Vehicle collision (no vehicle to test: false).
        self.vars.vehicle_collision = false;
        // updateEquippedItemSounds, updateEmitter: sound.
        // updateMechanicsItems (vehicle mechanics UI), updateHeavyBreathing (empty).
        self.update_temperature_check(sm);
        if self.is_weapon_ready(cx) {
            self.update_aiming_stance(cx);
        }
        let reticle = self.player.input.is_precision_aim_key_down(input, self.input_owner()) && self.is_aiming_firearm_equipped();
        out.events.push(PlayerEvent::ReticleMode { reticle });
        self.vars.precision_aim_key_down = self.player.input.is_precision_aim_key_down(input, self.input_owner());
        self.update_nutrition_and_fitness(sm, chars, ctx)?;
        // updateVisionEffectTargets.
        let p = &mut self.player;
        let r = if p.blur_factor < p.blur_factor_target { 0.1 } else { 0.01 };
        p.blur_factor = crate::character::zombie::math::lerp(p.blur_factor, p.blur_factor_target, r);
        // updateSoundListener, SafetySystemManager.update: sound / MP.
        if self.player.death_finished {
            return Ok(false);
        }
        // getCurrentBuildingDef().setHasBeenVisited(true) is the world's.
        let m = ctx.multiplier();
        self.player.time_since_open_door += m;
        self.path.time_since_close_door += m;
        self.player.last_targeted += m;
        self.player.targeted_by_zombie = false;
        self.check_action_group(registry, groups);
        let endurance = Self::stat_ref_pub(chars, "ENDURANCE")?;
        if self.player.just_moved && !self.player.npc && self.path.path2.is_none() && !self.is_current_action_pathfinding() {
            if cx.world.game_speed() > 1 {
                cx.ctx.event(crate::character::SimEvent::SetGameSpeed(1));
            }
        } else if self.stats.get(&endurance) < 0.25 && ctx.rand_next((300.0 * ctx.inv_multiplier()) as i32) == 0 {
            self.add_xp(sm, chars, chars.perks.constant("Fitness").as_ref(), 1.0, ctx)?;
        }
        let mut delta = 1.0f32;
        let mut d = 0.0f32;
        if self.player.just_moved && !self.player.npc {
            d = if self.run_or_sprint(sm, ctx) { 1.5 } else { 1.0 };
            out.events.push(PlayerEvent::LuaEvent("OnPlayerMove"));
        }
        // updateRemotePlayer(): false for a local player.
        // The camera and IsoPlayer.instance are the engine's.
        if self.player.closest_zombie > 1.2 {
            self.player.slow_timer = -1.0;
            self.player.slow_factor = 0.0;
        }
        self.player.context_panic -= 1.5 * ctx.time_delta();
        if self.player.context_panic < 0.0 {
            self.player.context_panic = 0.0;
        }
        self.player.last_seen_zombie_time += (ctx.game_world_seconds_since_last_update() / 60.0 / 60.0) as f64;
        out.events.push(PlayerEvent::LuaEvent("OnPlayerUpdate"));
        if self.pressed_movement(input, false) {
            self.player.context_panic = 0.0;
            self.player.ticks_since_pressed_movement = 0.0;
        } else {
            self.player.ticks_since_pressed_movement += ctx.thirty_fps_multiplier();
        }
        let pm = self.pressed_movement(input, true);
        self.set_var_bool("pressedMovement", pm);
        if self.update_while_dead() {
            return Ok(true);
        }
        // updateAttackLoopSound .. updateEquippedBaggageContainer: sound.
        self.update_sneak_key(input);
        self.check_is_near_wall(cx);
        // checkIsNearVehicle: TODO(vehicles).
        let idle_nodes = cx.world.contains_any_idle_nodes(self.id);
        self.update_ext(sm, ctx, idle_nodes);
        self.update_interact_key_panic();
        // updateMusicIntensityEvents, updateMusicThreatStatuses: music.
        if self.asleep {
            self.player_moving = false;
            self.vars.is_player_moving = false;
        }
        if self.vars.ignore_movement || self.asleep {
            self.clear_use_key_variables();
            return Ok(true);
        }
        if self.player.action_blocks_movement {
            self.clear_use_key_variables();
            return Ok(true);
        }
        // enterExitVehicle: TODO(vehicles).
        self.check_action_group(registry, groups);
        self.check_reloading(input, out, cx);
        if self.check_walk_to(input) {
            out.events.push(PlayerEvent::PressWalkTo);
        }
        if self.player.action_blocks_movement {
            self.clear_use_key_variables();
            return Ok(true);
        }
        if self.environment.in_vehicle {
            // TODO(vehicles): updateWhileInVehicle.
            return Ok(true);
        }
        // checkVehicleContainers: TODO(vehicles).
        self.player.collidable = true;
        self.path.collidable = true;
        if self.player.player_index == 0 && self.is_player_aiming() && !self.is_dead() {
            out.events.push(PlayerEvent::HideCursor);
        }
        self.player.seen_this_frame = false;
        self.player.could_be_seen_this_frame = false;
        if self.update_use_key(input, out, cx) {
            return Ok(true);
        }
        let mut is_attacking = false;
        let mut b_melee = false;
        let mut b_grapple = false;
        self.running = false;
        self.environment.sprinting = false;
        self.player.use_charge_time = self.vars.charge_time;
        if !self.vars.block_movement && !self.player.npc {
            self.vars.charge_time =
                if self.player.is_charging || self.player.is_charging_lt { self.vars.charge_time + 1.0 * m } else { 0.0 };
            let st = self.update_input_state(input, ctx);
            b_melee = st.melee;
            b_grapple = st.grapple;
            is_attacking = st.is_attacking;
            self.running = self.player.input.allow_run && st.running;
            self.environment.sprinting = st.sprinting;
            if self.is_sprinting(sm, ctx) && !self.player.just_moved {
                self.environment.sprinting = false;
            }
            if self.is_sprinting(sm, ctx) {
                self.running = false;
            }
            if st.sprinting && !self.is_sprinting(sm, ctx) {
                self.running = true;
            }
            // TODO(grappling): picking up / putting down a body stops aiming.
            self.vars.is_aiming = st.is_aiming;
            self.player.is_charging = st.is_charging;
            self.player.is_charging_lt = st.is_charging_lt;
            self.update_movement_rates(cx);
            if self.is_player_aiming() {
                out.events.push(PlayerEvent::StopActionQueue("Aiming"));
            }
            if is_attacking {
                self.vars.is_aiming = true;
            }
            self.player.waiting = false;
            if self.is_player_aiming() {
                self.vars.moving = false;
                self.running = false;
                self.environment.sprinting = false;
            }
            // isFarming: TODO(farming).
            self.player.ticks_since_seen_zombie += 1;
        }
        if self.player.player_move_dir.0 == 0.0 && self.player.player_move_dir.1 == 0.0 {
            self.player.input.force_run = false;
            self.player.input.force_sprint = false;
        }
        self.player.movement_last_frame = self.player.player_move_dir;
        if self.is_current_state_id(ids::STAGGER_BACK) || self.is_current_state_id(ids::FAKE_DEAD_ZOMBIE) {
            return Ok(true);
        }
        self.player.just_moved = false;
        let mut mv = MoveVars::default();
        self.update_movement_from_input(input, &mut mv, cx);
        if !self.player.just_moved && self.path.path2.is_some() && cx.world.path_should_be_moving(self.id) {
            self.player.just_moved = true;
        }
        let (strafe_x, strafe_y) = (mv.strafe_x, mv.strafe_y);
        self.update_aiming_delay(sm, chars, ctx);
        self.set_been_moving_sprinting(sm, ctx);
        delta *= d;
        if delta > 1.0 {
            delta *= sprint_mod(self.perk_lv(chars, "Sprinting"));
        }
        if delta > 1.0 && self.trait_on(sm, "ATHLETIC") {
            delta *= 1.2;
        }
        if delta > 1.0 {
            if self.trait_on(sm, "OVERWEIGHT") {
                delta *= 0.99;
            }
            if self.trait_on(sm, "OBESE") {
                delta *= 0.85;
            }
            if self.nutrition.as_ref().is_some_and(|n| n.weight() > 120.0) {
                delta *= 0.97;
            }
            if self.trait_on(sm, "OUT_OF_SHAPE") {
                delta *= 0.99;
            }
            if self.trait_on(sm, "UNFIT") {
                delta *= 0.8;
            }
        }
        self.update_endurance(sm, chars, ctx)?;
        if self.is_player_aiming() && self.player.just_moved {
            delta *= 0.7;
        }
        if self.is_player_aiming() {
            delta *= nimble_mod(self.perk_lv(chars, "Nimble"));
        }
        self.player.is_walking = delta > 0.0 && !self.player.npc;
        self.vars.melee_pressed = b_melee;
        self.vars.grapple_pressed = b_grapple;
        if !self.vars.grapple_pressed {
            self.vars.do_grapple = false;
            self.vars.can_let_go_of_grappled = true;
        }
        if !self.player.press_context && self.player.let_go_after_context_is_released {
            self.vars.do_grapple = false;
            // TODO(grappling): setDoGrappleLetGo.
        }
        if b_melee {
            if !self.player.last_attack_was_hand_to_hand {
                self.vars.melee_delay = self.vars.melee_delay.min(2.0);
            }
            if !self.vars.banned_attacking
                && self.player.authorize_hand_to_hand
                && !self.vars.performing_attack_animation
                && self.vars.melee_delay <= 0.0
            {
                self.vars.do_shove = true;
                self.vars.do_grapple = false;
                if !self.player.is_charging && !self.player.is_charging_lt {
                    self.vars.is_aiming = false;
                }
                let c = self.player.use_charge_time;
                self.attempt_attack(c, out, cx)?;
                self.player.use_charge_time = 0.0;
                self.vars.charge_time = 0.0;
            }
        } else if b_grapple {
            if !self.player.last_attack_was_hand_to_hand {
                self.vars.melee_delay = self.vars.melee_delay.min(2.0);
            }
            if !self.vars.banned_attacking && self.player.authorize_hand_to_hand && self.can_attack(cx) && self.vars.melee_delay <= 0.0 {
                // TODO(grappling): grab / let go.
                self.player.use_charge_time = 0.0;
                self.vars.charge_time = 0.0;
            }
        } else if self.is_player_aiming() && self.can_attack(cx) {
            // TODO(grappling): drop the dragged character.
            if is_attacking
                && !self.vars.banned_attacking
                && !self.is_current_state_id(ids::PLAYER_HIT_REACTION)
                && !self.is_current_state_id(ids::PLAYER_HIT_REACTION_PVP)
            {
                if self.vars.recoil_delay <= 0.0 && self.vars.melee_delay <= 0.0 {
                    let c = self.player.use_charge_time;
                    self.attempt_attack(c, out, cx)?;
                }
                self.player.use_charge_time = 0.0;
                self.vars.charge_time = 0.0;
            }
        } else if self.vars.head_look_around {
            // TODO(anim bones): the head look-around twist reads the head bone.
        }
        if self.is_player_aiming() && !self.player.npc && !self.is_current_state_id(ids::FISHING) {
            let (vx, vy) = self.aim_vector(input);
            if vx * vx + vy * vy > 0.0 {
                self.set_forward_direction(vx, vy);
            }
            mv.new_facing = Some(self.vars.dir);
        }
        if self.player.last_angle.0 != self.vars.forward_x || self.player.last_angle.1 != self.vars.forward_y {
            self.player.last_angle = (self.vars.forward_x, self.vars.forward_y);
            self.player.dirty_recalc_grid_stack_time = 2.0;
        }
        let anim_ready = cx.world.animation_player_ready(self.id);
        if !(anim_ready || self.vars.falling || self.is_player_aiming() || is_attacking) {
            if let Some(f) = mv.new_facing {
                self.set_forward_iso_direction(f);
            }
        }
        if self.is_player_aiming() && self.player.input.input_mode(input, self.input_owner()) != super::CharacterInputMode::Gamepad {
            self.player.player_move_dir = (mv.move_x, mv.move_y);
        }
        if !self.is_player_aiming() && self.player.just_moved {
            self.player.player_move_dir = (self.vars.forward_x, self.vars.forward_y);
        }
        let speed = if self.player.just_moved {
            if self.is_sprinting(sm, ctx) {
                1.5
            } else if self.is_running(sm) {
                1.0
            } else {
                0.5
            }
        } else {
            0.0
        };
        self.environment.current_speed = speed;
        let override_anim = self.is_current_state_id(ids::SWIPE_STATE_PLAYER) || self.player.action_overrides_animation;
        if !override_anim && !self.player.force_override_anim {
            if self.path.path2.is_none() {
                if self.environment.current_speed > 0.0 && (!self.vars.climbing || self.vars.last_fall_speed > 0.0) {
                    out.events.push(PlayerEvent::StopActionQueue(if self.run_or_sprint(sm, ctx) { "Running" } else { "Walking" }));
                }
            } else {
                out.events.push(PlayerEvent::StopActionQueue("Walking"));
            }
        }
        if self.player.slow_timer > 0.0 {
            self.player.slow_timer -= ctx.realworld_seconds_since_last_update();
            self.environment.current_speed *= 1.0 - self.player.slow_factor;
            self.player.slow_factor -= m / 100.0;
            if self.player.slow_factor < 0.0 {
                self.player.slow_factor = 0.0;
            }
        } else {
            self.player.slow_factor = 0.0;
        }
        let (px, py) = self.player.player_move_dir;
        self.player.player_move_dir = set_length(px, py, self.environment.current_speed);
        if self.player.player_move_dir != (0.0, 0.0) {
            self.player.dirty_recalc_grid_stack_time = 10.0;
        }
        if self.path.path2.is_some() && self.path.current_square != self.vars.last_square {
            self.player.dirty_recalc_grid_stack_time = 10.0;
        }
        self.player.closest_zombie = 1_000_000.0;
        self.player.weight = 0.3;
        cx.world.separate(self);
        self.update_sleeping_pills_taken(chars, ctx)?;
        // updateTorchStrength: the flicker is off (`flickTorch = false` first).
        self.player_moving = self.player.just_moved || self.path.path2.is_some() && cx.world.path_should_be_moving(self.id);
        self.vars.is_player_moving = self.player_moving;
        let in_trees = self.current_square_xyz().is_some_and(|sq| cx.world.square_trees(sq).0);
        if in_trees {
            let v = self.calculate_in_trees_speed();
            self.set_var_float("WalkSpeedTrees", v);
        }
        let collapsed_fence = self.current_square_xyz().is_some_and(|sq| cx.world.collapsed_fence(sq));
        if (in_trees || collapsed_fence || self.vars.walk_speed < 0.4 || self.vars.walk_injury > 0.5)
            && self.is_sprinting(sm, ctx)
            && !self.is_ghost_mode(ctx)
        {
            if self.player.run_speed_modifier < 1.0 {
                self.moodle_cant_sprint = true;
            }
            self.environment.sprinting = false;
            self.running = true;
            if self.vars.in_trees_no_bush || collapsed_fence {
                self.player.input.force_sprint = false;
                self.set_bump_type("left");
                self.set_var_bool("BumpDone", false);
                self.set_var_bool("BumpFall", true);
                self.set_var_str("TripObstacleType", "tree");
                self.report_event("wasBumped");
            }
        }
        self.vars.delta_x = strafe_x;
        self.vars.delta_y = strafe_y;
        self.vars.windspeed = self.player.climate_wind_speed_movement;
        self.vars.wind_force = self.player.climate_wind_force_movement;
        Ok(true)
    }

    /// `CharacterStats` lookup for the player update.
    pub(crate) fn stat_ref_pub(chars: &Characters, f: &str) -> ScriptResult<crate::character::stats::CharacterStatRef> {
        chars.sim.character_stats.field(f)
    }

    /// `isCurrentActionPathfinding()` (the Lua timed action's; false).
    fn is_current_action_pathfinding(&self) -> bool {
        false
    }

    /// `calculateInTreesSpeed()`.
    fn calculate_in_trees_speed(&self) -> f32 {
        // TODO(professions): PARK_RANGER 1.3, LUMBERJACK 1.15.
        let mut s = 1.0f32;
        if self.running {
            s *= 1.1;
        }
        s
    }

    /// `updateDeathDragDown()`.
    fn update_death_drag_down(&mut self, cx: &mut StateCx<'_, I>) {
        if self.is_dead() || !self.death_drag_down {
            return;
        }
        if self.is_god_mod(cx.ctx) {
            self.death_drag_down = false;
            return;
        }
        if self.vars.hit_reaction == "EndDeath" {
            return;
        }
        let (sx, sy, sz) = (fastfloor(self.vars.x), fastfloor(self.vars.y), fastfloor(self.vars.z));
        for dy in -1..=1 {
            for dx in -1..=1 {
                for id in cx.world.characters_on_square((sx + dx, sy + dy, sz)) {
                    if let Some(z) = cx.world.character_mut(id) {
                        if z.is_zombie() && !z.is_dead() && !z.vars.on_floor {
                            self.vars.attacked_by = Some(id);
                            self.vars.hit_reaction = "EndDeath".into();
                            self.vars.block_movement = true;
                            return;
                        }
                    }
                }
            }
        }
        self.death_drag_down = false;
    }

    /// `updateTemperatureCheck()` (`forceAwake` while asleep).
    fn update_temperature_check(&mut self, sm: &ScriptManager) {
        let hypo = self.moodle_level(sm, "HYPOTHERMIA");
        if self.player.hypothermia_cache == -1 || self.player.hypothermia_cache != hypo {
            if hypo >= 3 && hypo > self.player.hypothermia_cache && self.asleep && !self.force_wake_up {
                self.force_awake();
            }
            self.player.hypothermia_cache = hypo;
        }
        let hyper = self.moodle_level(sm, "HYPERTHERMIA");
        if self.player.hyperthermia_cache == -1 || self.player.hyperthermia_cache != hyper {
            if hyper >= 3 && hyper > self.player.hyperthermia_cache && self.asleep && !self.force_wake_up {
                self.force_awake();
            }
            self.player.hyperthermia_cache = hyper;
        }
    }

    /// `updateAimingStance()` (the targets come from the melee hit list
    /// code; `LeftHandMask`/`aimAtFloor`).
    fn update_aiming_stance(&mut self, cx: &mut StateCx<'_, I>) {
        if self.get_variable_string("LeftHandMask") == "RaiseHand" {
            self.clear_variable("LeftHandMask");
        }
        if !self.is_player_aiming() || self.is_current_state_id(ids::SWIPE_STATE_PLAYER) {
            return;
        }
        // TODO(combat): calcValidTargets / isProneTargetBetter for the
        // aiming stance (the target lists are the attack code's).
        let attacking = self.is_performing_hostile_animation();
        if !attacking {
            self.vars.aim_at_floor = false;
        }
        let _ = cx;
    }

    /// `updateWhileDead()`.
    fn update_while_dead(&mut self) -> bool {
        if !self.is_dead() {
            return false;
        }
        self.set_var_bool("bPathfind", false);
        self.vars.moving = false;
        self.player_moving = false;
        self.vars.is_player_moving = false;
        true
    }

    /// `updateSneakKey()`.
    fn update_sneak_key(&mut self, input: &dyn PlayerInput) {
        if self.player.input.is_crouch_button_pressed(input, self.input_owner()) {
            self.environment.sneaking = !self.environment.sneaking;
        }
    }

    /// `checkIsNearWall()`.
    pub fn check_is_near_wall(&mut self, cx: &StateCx<'_, I>) -> f32 {
        let Some(sq) = self.current_square_xyz().filter(|_| self.environment.sneaking) else {
            self.vars.near_wall_crouching = false;
            return 0.0;
        };
        let w = &*cx.world;
        let adj = |dx: i32, dy: i32| {
            let s = (sq.0 + dx, sq.1 + dy, sq.2);
            w.square(s.0, s.1, s.2).map(|_| s)
        };
        if let Some(n) = adj(0, -1) {
            let r = w.grid_sneak_modifier(n, true);
            if r > 1.0 {
                self.vars.near_wall_crouching = true;
                return r;
            }
        }
        for (dx, dy) in [(0, 1), (1, 0), (-1, 0)] {
            if let Some(s) = adj(dx, dy) {
                let r = w.grid_sneak_modifier(s, false);
                let r2 = w.grid_sneak_modifier(s, true);
                if r > 1.0 || r2 > 1.0 {
                    self.vars.near_wall_crouching = true;
                    return if r > 1.0 { r } else { r2 };
                }
            }
        }
        let r = w.grid_sneak_modifier(sq, false);
        if r > 1.0 {
            self.vars.near_wall_crouching = true;
            return r;
        }
        // TODO(vehicles): isNearVehicle gives 6.
        self.vars.near_wall_crouching = false;
        0.0
    }

    /// `updateExt()` (the fidget choice is TODO(anim): `onIdlePerformFidgets`).
    fn update_ext(&mut self, sm: &ScriptManager, ctx: &dyn SimContext, cx_idle_nodes: bool) {
        if self.environment.sneaking {
            return;
        }
        // TODO(grappling): isGrappling / isBeingGrappled.
        self.player.ext_update_count += ctx.multiplier() / 0.8;
        let idle = cx_idle_nodes;
        if !idle && !self.sit_on_ground {
            self.player.ext_update_count = 0.0;
        }
        if self.player.ext_update_count <= 5000.0 {
            return;
        }
        self.player.ext_update_count = 0.0;
        if self.stats.num_visible_zombies() != 0 || self.stats.num_chasing_zombies() != 0 {
            return;
        }
        if !next_bool(ctx, 3) {
            return;
        }
        if !idle && !self.sit_on_ground {
            return;
        }
        let _ = sm;
        self.report_event("EventDoExt");
    }

    /// `updateInteractKeyPanic()`.
    fn update_interact_key_panic(&mut self) {
        if self.player.player_index != 0 {
            return;
        }
        if self.player.input.is_interact_button_pressed() {
            self.player.context_panic += 0.6;
        }
    }

    /// `checkActionGroup()`: the `player` group on foot (the vehicle group
    /// is TODO(vehicles)).
    pub fn check_action_group(&mut self, registry: &mut StateRegistry<I>, groups: &mut ActionGroups) {
        let in_vehicle = self.environment.in_vehicle;
        let want = if in_vehicle { "player-vehicle" } else { "player" };
        let have = self.state.action_context.group().map(|g| g.name.clone());
        if have.as_deref() != Some(want) {
            self.initialize_player_states(registry, in_vehicle);
            let g = groups.get(want);
            self.set_action_group(g);
            if !in_vehicle {
                for v in [
                    "bEnteringVehicle",
                    "EnterAnimationFinished",
                    "bExitingVehicle",
                    "ExitAnimationFinished",
                    "bSwitchingSeat",
                    "SwitchSeatAnimationFinished",
                ] {
                    self.clear_variable(v);
                }
                self.vars.hit_reaction = String::new();
            }
        }
    }

    /// `checkReloading()`.
    fn check_reloading(&mut self, input: &dyn PlayerInput, out: &mut PlayerUpdate, cx: &StateCx<'_, I>) {
        let Some(item) = self.items.primary_hand_item() else { return };
        let item = &*item.borrow();
        if !crate::character::items::is_hand_weapon(item) || !item.is_ranged() {
            return;
        }
        // TODO(items): isReloadable, getWeaponReloadType.
        let _ = cx;
        if self.player.input.is_reload_weapon_button_pressed() {
            out.events.push(PlayerEvent::PressReload { rack: false, shift: false });
        } else if self.player.input.is_rack_firearm_button_pressed() {
            let shift = self.player.input.is_shift_key_down(input);
            out.events.push(PlayerEvent::PressReload { rack: true, shift });
        }
    }

    /// `checkWalkTo()`.
    fn check_walk_to(&self, input: &dyn PlayerInput) -> bool {
        !self.player.npc && self.player.player_index == 0 && self.player.input.is_walk_to_button_down(input)
    }

    /// `pressedMovement(ignoreBlock)` (sets `pressedRunButton`).
    pub fn pressed_movement(&mut self, input: &dyn PlayerInput, ignore_block: bool) -> bool {
        if self.player.npc {
            return false;
        }
        let owner = self.input_owner();
        let run = self.player.input.is_run_button_down(input, owner);
        self.set_var_bool("pressedRunButton", run);
        if !ignore_block && (self.vars.block_movement || self.player.input.ignore_inputs_for_direction) {
            return false;
        }
        self.player.input.is_input_move_axis_applied(input, owner)
    }

    /// `updateUseKey()`: `true` when a contextual action ran.
    fn update_use_key(&mut self, input: &dyn PlayerInput, out: &mut PlayerUpdate, cx: &mut StateCx<'_, I>) -> bool {
        if !self.is_local_player(cx.ctx) {
            return false;
        }
        let interact = self.player.input.is_interact_button_down();
        self.player.time_pressed_context += cx.ctx.realworld_seconds_since_last_update();
        if interact && self.player.time_pressed_context < 0.5 {
            self.player.press_context = true;
        } else {
            if self.player.press_context && (input.is_doing_text_entry() || !input.do_lua_key_pressed()) {
                self.player.press_context = false;
            }
            if self.vars.grapple_throw_out_window {
                self.player.press_context = false;
            }
            if self.player.press_context && self.do_context(input, out, cx) {
                self.clear_use_key_variables();
                return true;
            }
            if !interact {
                self.clear_use_key_variables();
            }
        }
        false
    }

    /// `clearUseKeyVariables()`.
    fn clear_use_key_variables(&mut self) {
        self.player.press_context = false;
        self.player.time_pressed_context = 0.0;
    }

    /// `IsoPlayer.getInputMoveVector(out)`.
    fn player_input_move_vector(&mut self, input: &dyn PlayerInput) -> (f32, f32) {
        let mut v = self.player.input.input_move_vector(input, self.input_owner());
        if self.vars.animating_backwards {
            v = (-v.0, -v.1);
        }
        if self.player.auto_walk && v.0 * v.0 + v.1 * v.1 == 0.0 {
            v = self.player.auto_walk_direction;
        }
        self.player.auto_walk_direction = v;
        v
    }

    /// `UpdateInputState(inputState)`.
    fn update_input_state(&mut self, input: &dyn PlayerInput, ctx: &dyn SimContext) -> InputState {
        let owner = self.input_owner();
        let c = &self.player.input;
        let mut st = InputState { is_attacking: self.player.is_charging && c.is_attack_button_down(input, owner), ..Default::default() };
        let auth = self.player.authorize_hand_to_hand_action;
        if c.is_melee_button_down(input, owner) && auth {
            st.melee = true;
            st.is_attacking = false;
        }
        // TODO(grappling): a grappled target on fire also grapples.
        if c.is_interact_button_down() && auth {
            st.grapple = true;
            st.is_attacking = false;
        }
        st.is_aiming = c.is_any_aim_key_down(input, owner) && self.get_variable_string("BumpFallType").is_empty();
        st.is_charging = c.is_any_aim_key_down(input, owner);
        st.running = c.is_run_button_down(input, owner);
        let allow_sprint = self.player.input.allow_sprint;
        if self.player.input.input_mode(input, owner) == super::CharacterInputMode::Gamepad {
            st.sprinting = st.running && allow_sprint && self.player.input.is_sprint_button_down(input, owner);
        } else if allow_sprint {
            if !input.dbl_tap_jog_to_sprint() {
                if self.player.input.is_sprint_button_down(input, owner) {
                    st.sprinting = true;
                    self.player.pressed_run_timer = 1.0;
                } else {
                    st.sprinting = false;
                }
            } else {
                let was = self.player.input.was_run_button_down(input);
                let is = self.player.input.is_run_button_down(input, owner);
                if !was && is && self.player.pressed_run_timer < 30.0 && self.player.pressed_run {
                    st.sprinting = true;
                }
                if was && !is {
                    st.sprinting = false;
                    self.player.pressed_run = true;
                }
                if !st.running {
                    st.sprinting = false;
                }
                if self.player.pressed_run {
                    self.player.pressed_run_timer += 1.0;
                }
                if self.player.pressed_run_timer > 30.0 {
                    self.player.pressed_run_timer = 0.0;
                    self.player.pressed_run = false;
                }
            }
        } else {
            st.sprinting = false;
        }
        if st.running || st.sprinting {
            self.player.input.force_aim = false;
        }
        if self.player.input.force_aim {
            st.is_aiming = true;
            st.is_charging = true;
        }
        if self.player.input.force_run {
            st.running = true;
        }
        if self.player.input.force_sprint {
            st.sprinting = true;
        }
        let _ = ctx;
        st
    }

    /// `updateMovementFromInput(moveVars)`.
    fn update_movement_from_input(&mut self, input: &dyn PlayerInput, mv: &mut MoveVars, cx: &mut StateCx<'_, I>) {
        *mv = MoveVars { new_facing: Some(self.vars.dir), ..Default::default() };
        // TutorialManager.stealControl: TODO(tutorial).
        if self.vars.block_movement || self.player.npc {
            return;
        }
        // TODO(grappling): picking up / putting down a body.
        if self.vars.fall_time > 2.0 {
            return;
        }
        let (aim_x, aim_y) = self.aim_vector(input);
        let (move_x, move_y) = self.player_input_move_vector(input);
        self.player.player_move_dir = (move_y + move_x, move_y - move_x);
        if move_x.abs() > 0.0 || move_y.abs() > 0.0 {
            self.player.just_moved = true;
        }
        let pmd = self.player.player_move_dir;
        if aim_x != 0.0 || aim_y != 0.0 {
            let (nx, ny) = normalize(aim_x, aim_y);
            mv.new_facing = Some(crate::character::state::vars::iso_direction_from_angle(nx, ny));
        } else if (move_x != 0.0 || move_y != 0.0) && pmd.0 * pmd.0 + pmd.1 * pmd.1 > 0.0 {
            let (nx, ny) = normalize(pmd.0, pmd.1);
            mv.new_facing = Some(crate::character::state::vars::iso_direction_from_angle(nx, ny));
        }
        mv.move_x = pmd.0;
        mv.move_y = pmd.1;
        self.resolve_strafe_direction_from_path(cx);
        let pmd = self.player.player_move_dir;
        if pmd.0 != 0.0 || pmd.1 != 0.0 {
            if self.is_strafing() {
                let (tx, ty) = normalize(pmd.0, -pmd.1);
                let mut angle = cx.world.rendered_angle(self.id).unwrap_or_else(|| self.anim_angle_radians());
                if angle as f64 > std::f64::consts::PI * 2.0 {
                    angle -= std::f32::consts::PI * 2.0;
                }
                if angle < 0.0 {
                    angle += std::f32::consts::PI * 2.0;
                }
                // Vector2.rotate(angle).
                let (c, s) = ((angle as f64).cos(), (angle as f64).sin());
                let rx = (tx as f64 * c - ty as f64 * s) as f32;
                let ry = (tx as f64 * s + ty as f64 * c) as f32;
                mv.strafe_x = rx;
                mv.strafe_y = ry;
                self.vars.ip_x = pmd.0;
                self.vars.ip_y = pmd.1;
            } else {
                mv.move_x = pmd.0;
                mv.move_y = pmd.1;
                self.set_forward_direction(pmd.0, pmd.1);
            }
        }
        if self.player.just_moved {
            cx.ctx.event(crate::character::SimEvent::SetGameSpeed(1));
        }
    }

    /// `isStrafing()` (`IsoPlayer`: the path strafes, or aiming).
    fn is_strafing(&self) -> bool {
        self.vars.path_strafing || self.is_player_aiming()
    }

    /// `resolveStrafeDirectionFromPath(playerMoveDir)`.
    fn resolve_strafe_direction_from_path(&mut self, cx: &StateCx<'_, I>) {
        let pmd = self.player.player_move_dir;
        if pmd != (0.0, 0.0) || self.path.path2.is_none() {
            return;
        }
        let Some(b) = cx.world.path_behavior(self.id) else { return };
        if b.stopping {
            return;
        }
        if b.is_strafing(self) {
            self.player.player_move_dir = normalize(b.target_x() - self.vars.x, b.target_y() - self.vars.y);
        } else if self.is_player_aiming() {
            self.player.player_move_dir = self.vars.req_movement;
        }
    }

    /// `updateAimingDelay()`.
    fn update_aiming_delay(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) {
        let aiming = self.perk_lv(chars, "Aiming");
        let mut m = 0.0f32;
        let firearm = self.items.primary_hand_item().and_then(|i| crate::character::firearm::FirearmStats::of(sm, &i.borrow()));
        if let Some(f) = &firearm {
            m = self.firearm_recoil_delay(chars, f) as f32 * (aiming as f32 / 30.0);
        }
        if self.is_player_aiming() && self.vars.recoil_delay <= 0.0 + m && !self.get_variable_boolean("isracking") {
            let marksman = if self.trait_on(sm, "MARKSMAN") { 0.1 } else { 0.0 };
            let v = self.vars.aiming_delay - 0.625 * ctx.multiplier() * (1.0 + 0.05 * aiming as f32 + marksman);
            self.vars.aiming_delay = v.max(0.0);
        } else if !self.is_player_aiming() {
            self.reset_aiming_delay(sm);
        }
    }

    /// `resetAimingDelay()`.
    pub fn reset_aiming_delay(&mut self, sm: &ScriptManager) {
        let w = self.items.primary_hand_item().filter(|i| crate::character::items::is_hand_weapon(&i.borrow()));
        match w {
            None => self.vars.aiming_delay = 0.0,
            Some(i) => {
                let mut d = crate::character::firearm::FirearmStats::of(sm, &i.borrow()).map_or(0, |f| f.aiming_time) as f32;
                d *= if self.trait_on(sm, "DEXTROUS") {
                    0.8
                } else if self.trait_on(sm, "ALL_THUMBS") {
                    1.2
                } else {
                    1.0
                };
                d *= if self.environment.in_vehicle { 1.5 } else { 1.0 };
                self.vars.aiming_delay = d;
            }
        }
    }

    /// `IsoPlayer.updateMovementRates()`.
    fn update_movement_rates(&mut self, cx: &mut StateCx<'_, I>) {
        self.calculate_walk_speed(cx);
        self.vars.idle_speed = 0.01 + self.moodle_level(cx.sm, "ENDURANCE") as f32 * 2.5 / 10.0;
        self.update_foot_injuries(cx);
        // updateInTreesInjuries: server only.
    }

    /// `calculateBaseSpeed()`.
    fn calculate_base_speed(&mut self, cx: &StateCx<'_, I>) -> f32 {
        let sm = cx.sm;
        let mut r = 0.8f32;
        r -= self.moodle_level(sm, "ENDURANCE") as f32 * 0.15;
        r -= self.moodle_level(sm, "HEAVY_LOAD") as f32 * 0.15;
        if self.moodle_level(sm, "PANIC") >= 3 && self.trait_on(sm, "ADRENALINE_JUNKIE") {
            r += (self.moodle_level(sm, "PANIC") + 1) as f32 / 20.0;
        }
        for i in super::attack::part::TORSO_UPPER..=super::attack::part::NECK {
            if let Some(p) = self.body_damage.body_part(i) {
                if p.has_injury() {
                    r -= 0.1;
                }
                if p.bandaged {
                    r += 0.05;
                }
            }
        }
        if let Some(p) = self.body_damage.body_part(super::attack::part::UPPERLEG_L) {
            let a = p.additional_pain_with(true);
            if a > 20.0 {
                r -= (a - 20.0) / 100.0;
            }
        }
        // TODO(items): calcRunSpeedModByBag for worn and held bags.
        let bag = 1.0f32;
        if let Some(sq) = self.current_square_xyz() {
            if cx.world.square_is_outside(sq) {
                let g = cx.world.ground_info(sq);
                if g.natural_floor {
                    r -= cx.ctx.wet_ground_final_value() * 0.25;
                }
                if g.sand {
                    r -= 0.05;
                }
            }
        }
        self.player.full_speed_mod = self.player.run_speed_modifier + (bag - 1.0);
        r * (1.0 - (1.0 - self.player.full_speed_mod).abs() / 2.0)
    }

    /// `IsoGameCharacter.calculateWalkSpeed()`.
    fn calculate_walk_speed(&mut self, cx: &mut StateCx<'_, I>) {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        // hasAttachedAnimals: TODO(animals).
        let injury = self.foot_injury_speed_modifier(ctx);
        self.set_var_float("WalkInjury", injury);
        let run_or_sprint = self.run_or_sprint(sm, ctx);
        let mut walk = self.calculate_base_speed(cx);
        let mut run = walk;
        run -= 0.15;
        run *= self.player.full_speed_mod;
        run += self.perk_lv(chars, "Sprinting") as f32 / 20.0;
        run -= (injury / 1.5).abs();
        if ctx.game_mode() == "Tutorial" {
            run = run.max(1.0);
        }
        let mut sneak = 1.0f32;
        if injury.abs() > 0.1 {
            if !run_or_sprint {
                sneak = if self.environment.sneaking { if self.vars.near_wall_crouching { 0.45 } else { 0.6 } } else { 1.0 };
            }
        } else {
            walk *= self.player.walk_speed_modifier;
        }
        self.vars.sneak_limp_speed_scale = sneak;
        if self.player.slow_factor > 0.0 {
            walk *= 0.05;
            run *= 0.05;
        }
        walk = walk.min(1.0);
        run = run.min(1.0);
        if self.body_damage.thermoregulator.is_some() {
            let m = crate::character::thermoregulator::Thermoregulator::movement_modifier(
                self.moodle_level(sm, "HYPOTHERMIA"),
                self.moodle_level(sm, "HYPERTHERMIA"),
            );
            walk *= m;
            run *= m;
        }
        if self.is_player_aiming() {
            let mut strafe = (0.9 + self.perk_lv(chars, "Nimble") as f32 / 10.0).min(1.5);
            let wm = ((if run_or_sprint { run } else { walk }) * 2.5).min(1.0);
            strafe *= wm;
            strafe = strafe.max(0.48);
            self.set_var_float("StrafeSpeed", strafe * 0.8);
        }
        if self.vars.in_trees_no_bush {
            if let Some(f) = self.current_square_xyz().and_then(|sq| cx.world.tree_slow_factor(sq, self.id)) {
                walk *= f;
                run *= f;
            }
        }
        self.set_var_float("WalkSpeed", if run_or_sprint { run } else { walk });
        self.set_var_float("RunSpeed", run);
    }

    /// `updateFootInjuries()` (shoes, glass and the floor are the world's:
    /// [`StateWorld::foot_injury_floor`]).
    ///
    /// [`StateWorld::foot_injury_floor`]: crate::character::state::StateWorld::foot_injury_floor
    fn update_foot_injuries(&mut self, cx: &mut StateCx<'_, I>) {
        let ctx = cx.ctx;
        if self.is_invulnerable(ctx) {
            return;
        }
        if self.environment.in_vehicle {
            self.player.foot_injury_timer = 0.0;
            return;
        }
        let sm = cx.sm;
        let shoes = self.items.worn().any(|i| crate::character::items::is_body_location(sm, &i.borrow(), "SHOES") && i.borrow().condition > 0);
        if shoes {
            return;
        }
        let Some(sq) = self.current_square_xyz() else { return };
        let floor = cx.world.foot_injury_floor(sq);
        if floor.broken_glass {
            // TODO(body damage): the glass shard wound on a random foot.
            let _ = ctx.rand_next_range(15, 17);
        }
        if floor.modifier == 0 {
            return;
        }
        let m1 = ctx.thirty_fps_multiplier() * 2.0;
        let (run, sprint) = (self.is_running(cx.sm), self.is_sprinting(cx.sm, ctx));
        let modifier = floor.modifier as f32;
        if self.player.is_walking && !run && !sprint {
            self.player.foot_injury_timer += modifier * m1;
        } else if run && !sprint {
            self.player.foot_injury_timer += modifier + 2.0 * m1;
        } else if sprint {
            self.player.foot_injury_timer += modifier + 5.0 * m1;
        } else {
            if self.player.foot_injury_timer > 0.0 {
                self.player.foot_injury_timer -= m1 / 3.0;
            }
            return;
        }
        let n = adjust_for_framerate(ctx, 8500 - fastfloor(self.player.foot_injury_timer));
        if ctx.rand_next(n) <= 0 {
            self.player.foot_injury_timer = 0.0;
            // TODO(body damage): the scratch/cut on a random foot.
            let _ = ctx.rand_next_range(15, 17);
        }
    }

    /// `IsoPlayer.updateEndurance()`.
    fn update_endurance(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        if self.is_animal() || ctx.is_client() {
            return Ok(());
        }
        if self.is_sitting() || self.is_resting {
            return self.update_endurance_while_sitting(sm, chars, ctx);
        }
        let g = ctx.zomboid_globals();
        let e = Self::stat_ref_pub(chars, "ENDURANCE")?;
        let fatigue = self.stats.get(&Self::stat_ref_pub(chars, "FATIGUE")?);
        let m = ctx.multiplier() as f64;
        let sneak = if self.environment.sneaking { 1.5f32 } else { 1.0 };
        let speed = self.environment.current_speed;
        let heavy = self.moodle_level(sm, "HEAVY_LOAD");
        let pacing = pacing_mod(self.perk_lv(chars, "Fitness"));
        let hyper = if self.moodle_level(sm, "HYPERTHERMIA") > 1 && self.moodle_level(sm, "HYPERTHERMIA") == 4 { 2.0 } else { 1.0 };
        let base_delta = |c: &Self| {
            let mut d = 1.4f32;
            if c.trait_on(sm, "OVERWEIGHT") {
                d = 2.9;
            }
            if c.trait_on(sm, "ATHLETIC") {
                d = 0.8;
            }
            d
        };
        let modv = if self.trait_on(sm, "ASTHMATIC") { 1.0f32 } else { 0.7 };
        let dragging = self.environment.dragging_corpse;
        if speed > 0.0 && (self.run_or_sprint(sm, ctx) || dragging) {
            let mut reduce = g.running_endurance_reduce;
            if self.is_sprinting(sm, ctx) || dragging {
                reduce = g.sprinting_endurance_reduce;
            }
            let mut d = base_delta(self);
            d *= 2.3;
            d *= pacing;
            d *= hyper;
            if heavy == 0 {
                self.stats.remove(&e, (reduce * d as f64 * 0.5 * modv as f64 * m * sneak as f64) as f32);
            } else {
                let w = match heavy {
                    1 => 1.5f32,
                    2 => 1.9,
                    3 => 2.3,
                    _ => 2.8,
                };
                self.stats.remove(&e, (reduce * d as f64 * 0.5 * modv as f64 * m * w as f64 * sneak as f64) as f32);
            }
        } else if speed > 0.0 && heavy > 2 {
            let mut d = base_delta(self);
            d *= 3.0;
            d *= pacing;
            d *= hyper;
            let w = match heavy {
                2 => 1.5f32,
                3 => 1.9,
                4 => 2.3,
                _ => 2.8,
            };
            self.stats.remove(
                &e,
                (g.running_endurance_reduce * d as f64 * 0.5 * modv as f64 * sneak as f64 * m * w as f64 / 2.0) as f32,
            );
        }
        if !self.player_moving {
            let mut mul = 1.0f32;
            mul *= 1.0 - fatigue * 0.85;
            mul *= ctx.multiplier();
            if heavy <= 1 {
                let v = g.imobile_endurance_reduce * ctx.endurance_regen_multiplier() * self.recovery_mod(sm, chars)? as f64 * mul as f64;
                self.stats.add(&e, v as f32);
            }
        }
        if !self.is_sprinting(sm, ctx) && !self.is_running(sm) && speed > 0.0 {
            let mut mul = 1.0f32;
            mul *= 1.0 - fatigue;
            mul *= ctx.multiplier();
            if self.moodle_level(sm, "ENDURANCE") < 2 {
                if heavy <= 1 {
                    let v = g.imobile_endurance_reduce / 4.0
                        * ctx.endurance_regen_multiplier()
                        * self.recovery_mod(sm, chars)? as f64
                        * mul as f64;
                    self.stats.add(&e, v as f32);
                }
            } else {
                self.stats.remove(&e, (g.running_endurance_reduce / 7.0 * sneak as f64) as f32);
            }
        }
        Ok(())
    }

    /// `IsoGameCharacter.updateInternal()` for a player (after
    /// `updateInternal2`), in the original order; the parts other systems
    /// own are noted.
    fn update_character_internal(&mut self, out: &mut PlayerUpdate, cx: &mut StateCx<'_, I>) -> ScriptResult {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        if self.current_square_xyz().is_none() {
            return Ok(());
        }
        // updateAlpha: rendering.
        self.update_ballistics_target(ctx, &mut *cx.world);
        // TODO(grappling): LetGoOfGrappled when not in a grappler state.
        if self.is_dead() {
            return Ok(());
        }
        // checkSCBADrain: TODO(items). Toxic buildings: TODO(world).
        let fps = ctx.thirty_fps_multiplier();
        if self.player.lunge_fall_timer > 0.0 {
            self.player.lunge_fall_timer -= fps;
        }
        let m = ctx.multiplier();
        if self.vars.melee_delay > 0.0 {
            self.vars.melee_delay -= 0.625 * m;
        }
        if self.vars.recoil_delay > 0.0 {
            self.vars.recoil_delay -= 0.625 * m;
        }
        // sx/sy, alarms (AmbientStreamManager), updateSeenVisibility: world.
        // updateMovementStatistics: TODO(statistics).
        // climbRopeTime: TODO(sheet ropes).
        self.vars.last_x = self.vars.x;
        self.vars.last_y = self.vars.y;
        self.vars.last_z = self.vars.z;
        // updateBeardAndHair: TODO(visual) (grows while asleep).
        // updateFalling: TODO(falling).
        self.update_internal_stats(sm, chars, ctx)?;
        // characterActions: the Lua timed action queue is the engine's.
        if ctx.do_character_stats() {
            self.body_damage_update(sm, chars, ctx)?;
            // updateBandages: TODO(items).
        }
        if ctx.do_character_stats() {
            self.calculate_stats(sm, chars, ctx)?;
        }
        if self.asleep {
            // TODO(sleep): IsoPlayer.processWakingUp.
        }
        self.update_idle_square_time(ctx);
        if !self.asleep {
            self.vars.last_x = self.vars.x;
            self.vars.last_y = self.vars.y;
            self.vars.last_z = self.vars.z;
            self.vars.state_event_delay_timer -= fps;
        }
        self.state_machine_update(cx);
        // ensureOnTile, RainManager: world.
        self.update_fire(cx);
        // updateTextObjects, blood impact position, recursiveItemUpdater,
        // attachedAnimSprite: rendering / items.
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        if self.is_god_mod(ctx) {
            for s in ["FATIGUE", "ENDURANCE", "TEMPERATURE", "HUNGER"] {
                self.stats.reset(&Self::stat_ref_pub(chars, s)?);
            }
            self.sleeping_pills_taken = 0;
        }
        self.update_movement_momentum(ctx);
        // effectiveEdibleBuffTimer: TODO(food buffs).
        self.update_dirt(out, sm, chars, ctx, cx)?;
        let aimed = self.vars.use_hand_weapon.and_then(|i| self.items.item(i)).is_some_and(|i| i.borrow().is_aimed_firearm());
        if aimed && self.is_player_aiming() {
            // setAngleFromAim, updateBallistics: the firearm code (#114).
            self.set_angle_from_aim_player(cx);
            self.update_ballistics(cx.sm, cx.chars, cx.ctx, &mut *cx.world);
        }
        Ok(())
    }

    /// `IsoPlayer.setAngleFromAim()` with the reticle on the ground the
    /// world reports ([`StateWorld::aim_reticle_iso`]).
    ///
    /// [`StateWorld::aim_reticle_iso`]: crate::character::state::StateWorld::aim_reticle_iso
    fn set_angle_from_aim_player(&mut self, cx: &mut StateCx<'_, I>) {
        let (rx, ry) = cx.world.aim_reticle_iso(self.id);
        let (vx, vy) = normalize(rx - self.vars.x, ry - self.vars.y);
        if vx * vx + vy * vy <= 0.0 {
            return;
        }
        self.set_forward_direction(vx, vy);
        if self.player.last_angle != (vx, vy) {
            self.player.last_angle = (vx, vy);
            self.player.dirty_recalc_grid_stack_time = 10.0;
        }
    }

    /// `updateMovementMomentum()`.
    fn update_movement_momentum(&mut self, ctx: &dyn SimContext) {
        let dt = ctx.time_delta();
        if self.player_moving && !self.is_player_aiming() {
            let time = self.vars.momentum_scalar * 0.55;
            if time >= 0.55 {
                self.vars.momentum_scalar = 1.0;
                return;
            }
            self.vars.momentum_scalar = ((time + dt) / 0.55).clamp(0.0, 1.0);
        } else {
            let time = (1.0 - self.vars.momentum_scalar) * 0.25;
            if time >= 0.25 {
                self.vars.momentum_scalar = 0.0;
                return;
            }
            self.vars.momentum_scalar = 1.0 - ((time + dt) / 0.25).clamp(0.0, 1.0);
        }
    }

    /// `updateDirt()`.
    fn update_dirt(&mut self, out: &mut PlayerUpdate, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, cx: &StateCx<'_, I>) -> ScriptResult {
        let mut n = 0;
        if self.is_running(sm) && next_bool(ctx, adjust_for_framerate(ctx, 3500)) {
            n = 1;
        }
        if self.is_sprinting(sm, ctx) && next_bool(ctx, adjust_for_framerate(ctx, 2500)) {
            n += ctx.rand_next_range(1, 3);
        }
        let t = self.stats.get(&Self::stat_ref_pub(chars, "TEMPERATURE")?);
        if t > 37.0 && next_bool(ctx, adjust_for_framerate(ctx, 5000)) {
            n += 1;
        }
        if t > 38.0 && next_bool(ctx, adjust_for_framerate(ctx, 3000)) {
            n += 1;
        }
        if self.player_moving {
            let sq = self.current_square_xyz();
            let g = sq.map(|s| cx.world.ground_info(s)).unwrap_or_default();
            let puddle = g.puddles_in_ground;
            let muddy = sq.is_some_and(|s| cx.world.square_is_outside(s)) && g.natural_floor && ctx.wet_ground_final_value() > 0.5;
            // `if (puddle) ++n; else if (muddy) ++n;` (the draws in that order).
            if puddle > 0.09 && next_bool(ctx, adjust_for_framerate(ctx, 1500))
                || muddy && next_bool(ctx, adjust_for_framerate(ctx, 3000))
            {
                n += 1;
            }
            if n > 0 {
                self.add_dirt(sm, ctx, n, true);
                out.events.push(PlayerEvent::AddDirt { count: n, all_layers: true });
            }
            n = 0;
            // `if (puddle) ++n; else if (muddy) ++n;` (the draws in that order).
            if puddle > 0.09 && next_bool(ctx, adjust_for_framerate(ctx, 1500))
                || muddy && next_bool(ctx, adjust_for_framerate(ctx, 3000))
            {
                n += 1;
            }
            let in_trees = sq.is_some_and(|s| cx.world.square_trees(s).0);
            if in_trees && next_bool(ctx, adjust_for_framerate(ctx, 1500)) {
                n += 1;
            }
            if n > 0 {
                self.add_dirt(sm, ctx, n, false);
                out.events.push(PlayerEvent::AddDirt { count: n, all_layers: false });
            }
        }
        Ok(())
    }

    /// `addDirt(null, count, allLayers)`: random parts (`OutfitRNG`) of the
    /// human visual and the worn items covering them get dirtier
    /// (`BloodClothingType.addDirt`).
    pub fn add_dirt(&mut self, sm: &ScriptManager, ctx: &dyn SimContext, count: i32, all_layers: bool) {
        use pz_core::random::outfit_rng;
        // `BloodBodyPartType.MAX.index()`.
        const MAX: usize = 17;
        let ids = self.items.item_visual_ids();
        for _ in 0..count {
            let part = outfit_rng::next_range(0, MAX as i32) as usize;
            let intensity = match ctx.clothing_degradation() {
                2 => outfit_rng::next_float(0.001, 0.01),
                3 => outfit_rng::next_float(0.05, 0.1),
                4 => outfit_rng::next_float(0.01, 0.05),
                _ => 0.0,
            };
            let covers = |c: &Self, id: i32| {
                c.items.item(id).is_some_and(|i| {
                    crate::character::items::covered_parts(sm, &i.borrow()).is_some_and(|p| p.contains(&part))
                        && crate::character::items::visual_hole(&i.borrow(), part) == 0.0
                })
            };
            if !all_layers {
                let hit = ids.iter().rev().copied().find(|&id| covers(self, id));
                match hit {
                    Some(id) => {
                        if intensity > 0.0 {
                            if let Some(i) = self.items.item(id) {
                                if let Some(v) = i.borrow_mut().visual.as_mut() {
                                    let d = v.get_dirt(part);
                                    v.set_dirt(part, d + intensity, MAX);
                                }
                            }
                            // TODO(items): calcTotalDirtLevel for clothing.
                        }
                    }
                    None => {
                        if let Some(h) = self.human_visual.as_mut() {
                            let d = h.get_dirt(part);
                            h.set_dirt(part, d + 0.05);
                        }
                    }
                }
                continue;
            }
            let Some(h) = self.human_visual.as_mut() else { continue };
            let d = h.get_dirt(part);
            h.set_dirt(part, d + 0.05);
            let mut current = h.get_dirt(part);
            if next_bool(ctx, ((current * 100.0) as i32 - 100).abs()) {
                continue;
            }
            for &id in &ids {
                if !covers(self, id) {
                    continue;
                }
                if intensity > 0.0 {
                    if let Some(i) = self.items.item(id) {
                        if let Some(v) = i.borrow_mut().visual.as_mut() {
                            let d = v.get_dirt(part);
                            v.set_dirt(part, d + intensity, MAX);
                            current = v.get_dirt(part);
                        }
                    }
                }
                if next_bool(ctx, ((current * 100.0) as i32 - 100).abs()) {
                    break;
                }
            }
        }
    }

    /// `IsoPlayer.postupdate()`: `IsoGameCharacter.postupdate` (the move
    /// and its collisions, the turning flags and the action context), the
    /// stale hit reaction, and the body parts' last state.
    pub fn postupdate_player(&mut self, cx: &mut StateCx<'_, I>) {
        let had_hit_reaction = !self.vars.hit_reaction.is_empty();
        self.moving_postupdate(cx);
        // postUpdateAnimating: the animation player is the animation's;
        // the action context runs here.
        self.update_action_context(cx);
        self.clear_attack_vars_post();
        if had_hit_reaction
            && !self.vars.hit_reaction.is_empty()
            && !self.is_current_state_id(ids::PLAYER_HIT_REACTION)
            && !self.is_current_state_id(ids::PLAYER_HIT_REACTION_PVP)
        {
            self.vars.hit_reaction = String::new();
        }
        self.body_damage.set_body_parts_last_state();
    }

    /// `clearHitInfo()` and `clearAttackVars()` of `postUpdateInternal`.
    fn clear_attack_vars_post(&mut self) {
        self.hit_info_list.clear();
        self.attack_vars = Default::default();
        if let Some(b) = self.ballistics_controller.as_mut() {
            b.post_update();
        }
    }
}

/// `getSprintMod()` for a Sprinting level.
pub fn sprint_mod(level: i32) -> f32 {
    match level {
        1 => 1.1,
        2 => 1.15,
        3 => 1.2,
        4 => 1.25,
        5 => 1.3,
        6 => 1.35,
        7 => 1.4,
        8 => 1.45,
        9 => 1.5,
        10 => 1.6,
        _ => 0.9,
    }
}

/// `getNimbleMod()` for a Nimble level.
pub fn nimble_mod(level: i32) -> f32 {
    match level {
        1 => 1.1,
        2 => 1.14,
        3 => 1.18,
        4 => 1.22,
        5 => 1.26,
        6 => 1.3,
        7 => 1.34,
        8 => 1.38,
        9 => 1.42,
        10 => 1.5,
        _ => 1.0,
    }
}

/// `getPacingMod()` for a Fitness level.
pub fn pacing_mod(level: i32) -> f32 {
    match level {
        1 => 0.8,
        2 => 0.75,
        3 => 0.7,
        4 => 0.65,
        5 => 0.6,
        6 => 0.57,
        7 => 0.53,
        8 => 0.49,
        9 => 0.46,
        10 => 0.43,
        _ => 0.9,
    }
}

