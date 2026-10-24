//! `IsoZombie`'s senses: spotting characters (`spotted`, `spottedNew`,
//! `spottedOld`, `updateVisionRadius`, `isTargetVisible`) and responding
//! to sounds (`RespondToSound`, `setTurnAlertedValues`).
//!
//! Light comes from the world's lighting (`square_light`), squares and
//! rooms from the world, sounds from the world's `WorldSoundManager`.

use pz_scripts::ScriptResult;
use pz_world::square::IsoDirections;

use super::math::*;
use super::GameCharacter;
use crate::character::state::{LosResult, StateCx};
use crate::character::states::ids;
use pz_audio::world_sound::{Listener, ListenerSquare, WorldSound};

/// What a zombie reads about the character it may spot.
#[derive(Clone, Debug, PartialEq)]
pub struct OtherView {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub square: Option<(i32, i32, i32)>,
    pub dead: bool,
    pub player: bool,
    pub ghost: bool,
    pub sneaking: bool,
    pub running: bool,
    pub aiming: bool,
    pub sneak_spot_mod: f32,
    pub inconspicuous: bool,
    pub conspicuous: bool,
    pub vehicle: Option<i16>,
    pub vehicle_speed_kmh: f32,
    pub torch_strength: f32,
    pub movement_last_frame: (f32, f32),
    pub player_index: i32,
    pub remote_id: i32,
    pub near_wall: f32,
    pub could_be_seen_this_frame: bool,
    pub seen_this_frame: bool,
    pub just_moved: bool,
    pub on_floor: bool,
    pub reanimated_corpse: bool,
}

impl<I: 'static> GameCharacter<I> {
    /// The view a zombie has of this character.
    pub fn other_view(&self, cx: &StateCx<'_, I>) -> ScriptResult<OtherView> {
        let sm = cx.sm;
        let sneak = self.perk_level(cx.chars.perks.constant("Sneak").as_ref());
        Ok(OtherView {
            id: self.id,
            x: self.vars.x,
            y: self.vars.y,
            z: self.vars.z,
            square: self.path.current_square,
            dead: self.is_dead(),
            player: self.is_player(),
            ghost: self.is_player() && self.is_ghost_mode(cx.ctx),
            sneaking: self.is_sneaking(),
            running: self.is_running(sm),
            aiming: self.is_aiming(),
            sneak_spot_mod: sneak_spot_mod(sneak),
            inconspicuous: self.is_player() && self.has_trait(sm, "INCONSPICUOUS").unwrap_or(false),
            conspicuous: self.is_player() && self.has_trait(sm, "CONSPICUOUS").unwrap_or(false),
            vehicle: self.path.vehicle,
            vehicle_speed_kmh: self.environment.vehicle_speed_kmh,
            torch_strength: self.seen.torch_strength,
            movement_last_frame: self.seen.movement_last_frame,
            player_index: self.seen.player_index,
            remote_id: self.seen.remote_id,
            near_wall: self.seen.near_wall,
            could_be_seen_this_frame: self.seen.could_be_seen_this_frame,
            seen_this_frame: self.seen.seen_this_frame,
            just_moved: self.seen.just_moved,
            on_floor: self.vars.on_floor,
            reanimated_corpse: self.seen.reanimated_corpse.is_some(),
        })
    }
}

/// `getSneakSpotMod()` for a Sneak level.
pub fn sneak_spot_mod(level: i32) -> f32 {
    let r: f32 = match level {
        1 => 0.9,
        2 => 0.8,
        3 => 0.75,
        4 => 0.7,
        5 => 0.65,
        6 => 0.6,
        7 => 0.55,
        8 => 0.5,
        9 => 0.45,
        10 => 0.4,
        _ => 0.95,
    };
    r * 1.2
}

/// The cone factor both spotting variants apply.
fn cone(cos_angle: f32, chance: f32) -> f32 {
    if cos_angle < -0.4 {
        0.0
    } else if cos_angle < -0.2 {
        chance / 8.0
    } else if cos_angle < -0.0 {
        chance / 4.0
    } else if cos_angle < 0.2 {
        chance / 2.0
    } else if cos_angle <= 0.4 {
        chance * 2.0
    } else if cos_angle <= 0.6 {
        chance * 8.0
    } else if cos_angle <= 0.8 {
        chance * 16.0
    } else {
        chance * 32.0
    }
}

/// The other, the zombie's square and the other's square.
type SpotPrelude = (OtherView, (i32, i32, i32), (i32, i32, i32));

/// What a spot check decided.
#[derive(Clone, Debug, PartialEq)]
pub struct SpotResult {
    /// The final percentage chance compared against the roll.
    pub chance: f32,
    pub success: bool,
}

impl<I: 'static> GameCharacter<I> {
    fn other_of(&self, other: i32, cx: &mut StateCx<'_, I>) -> ScriptResult<Option<OtherView>> {
        let Some(o) = cx.world.character_mut(other) else { return Ok(None) };
        let o: &GameCharacter<I> = o;
        // The view needs only shared access; reborrow the context.
        let view = {
            let cx2 = StateCx { sm: cx.sm, chars: cx.chars, ctx: cx.ctx, world: &mut crate::character::state::NullWorld };
            o.other_view(&cx2)?
        };
        Ok(Some(view))
    }

    /// `spotted(other, bForced)`: the new or old spotting logic by
    /// `lore.spottedLogic`. Returns the spot check, if one was rolled.
    pub fn spotted(&mut self, other: i32, forced: bool, cx: &mut StateCx<'_, I>) -> ScriptResult<Option<SpotResult>> {
        if cx.ctx.lore_spotted_logic() {
            self.spotted_new(other, forced, cx)
        } else {
            self.spotted_old(other, forced, cx)
        }
    }

    /// The checks both variants start with; `None` stops.
    fn spot_prelude(&mut self, other: i32, cx: &mut StateCx<'_, I>) -> ScriptResult<Option<SpotPrelude>> {
        if cx.ctx.is_client() && self.zombie.remote {
            if let Some(t) = self.vars.target {
                if let Some((tx, ty)) = cx.world.character_mut(t).map(|t| (t.vars.x, t.vars.y)) {
                    self.zombie.vector_to_target = (tx - self.vars.x, ty - self.vars.y);
                    if self.is_current_state_id(ids::LUNGE_NETWORK) {
                        let (x, y) = normalize(self.zombie.vector_to_target.0, self.zombie.vector_to_target.1);
                        self.set_forward_direction(x, y);
                    }
                }
            }
            return Ok(None);
        }
        let Some(my_sq) = self.current_square_xyz() else { return Ok(None) };
        let Some(o) = self.other_of(other, cx)? else { return Ok(None) };
        let Some(o_sq) = o.square else { return Ok(None) };
        if self.zombie.reanimated_for_grapple_only || self.zombie.useless || cx.world.square_smoke(my_sq) {
            self.set_zombie_target(None);
            self.zombie.spotted_last = None;
            return Ok(None);
        }
        if o.dead {
            return Ok(None);
        }
        if o.player && o.ghost {
            return Ok(None);
        }
        Ok(Some((o, my_sq, o_sq)))
    }

    /// `spottedNew(other, bForced)`.
    pub fn spotted_new(&mut self, other: i32, forced: bool, cx: &mut StateCx<'_, I>) -> ScriptResult<Option<SpotResult>> {
        let Some((o, my_sq, o_sq)) = self.spot_prelude(other, cx)? else { return Ok(None) };
        let ctx = cx.ctx;
        let mut chance = 100.0f32;
        let player_index = if o.player && !ctx.is_server() { o.player_index } else { 0 };
        let light = cx.world.square_light(o_sq, player_index).clamp(0.0, 1.0);
        let light = if light.is_nan() { 0.0 } else { light };
        chance *= light;
        if o_sq.2 != my_sq.2 {
            let dif = (o_sq.2 - my_sq.2).abs() * 5 + 1;
            chance /= dif as f32;
        }
        let (mut tx, mut ty) = (o.x - self.vars.x, o.y - self.vars.y);
        let mut view_dist = ctx.view_dist();
        let len = length(tx, ty);
        if len > view_dist {
            return Ok(None);
        }
        if ctx.is_server() {
            self.zombie.indoor_zombie = false;
        }
        if len < view_dist {
            view_dist = len;
        }
        view_dist *= 1.1;
        if view_dist > ctx.view_dist_max() {
            view_dist = ctx.view_dist_max();
        }
        (tx, ty) = normalize(tx, ty);
        let (fx, fy) = self.look_vector_exact();
        let cos_angle = fx * tx + fy * ty;
        self.update_vision_radius(cx);
        if self.dist_to_xy(o.x, o.y) > self.zombie.vision_radius_result {
            chance = 0.0;
        }
        if view_dist > 0.5 {
            chance = cone(cos_angle, chance);
        }
        self.mark_targeted(chance, cx);
        let movement = length(o.movement_last_frame.0, o.movement_last_frame.1);
        let mut movement_mod = 0.8f32;
        if movement == 0.5 {
            movement_mod = 1.0;
        } else if movement == 1.0 {
            movement_mod = 1.5;
        } else if movement == 1.5 {
            movement_mod = 2.0;
        }
        chance *= movement_mod;
        if view_dist < 5.0 && (!o.running && !o.sneaking && !o.aiming || o.running) {
            chance *= 3.0;
        }
        let mut sneaking_mod = o.sneak_spot_mod;
        if o.player && !o.sneaking {
            sneaking_mod = 1.0;
        }
        chance *= sneaking_mod;
        if self.farther_than_target(&o, cx) {
            return Ok(None);
        }
        if forced {
            chance = 1_000_000.0;
        }
        if self.zombie.bonus_spot_time > 0.0 {
            chance *= 5.0;
        }
        if self.zombie.sight == 1 {
            chance *= 2.5;
        }
        if self.zombie.sight == 3 {
            chance *= 0.45;
        }
        if self.zombie.inactive {
            chance *= 0.25;
        }
        let mut trait_mod = 1.0f32;
        if o.player && o.inconspicuous {
            trait_mod = 0.8;
        }
        if o.player && o.conspicuous {
            trait_mod = 1.2;
        }
        chance *= trait_mod;
        let mut shelter_mod = 1.0f32;
        if my_sq != o_sq && o.player && o.sneaking {
            let x_diff = (my_sq.0 - o_sq.0).abs();
            let y_diff = (my_sq.1 - o_sq.1).abs();
            shelter_mod = if x_diff > y_diff {
                if my_sq.0 - o_sq.0 > 0 {
                    self.obstacle_mod(o_sq, IsoDirections::E, cx)
                } else {
                    self.obstacle_mod(o_sq, IsoDirections::W, cx)
                }
            } else if my_sq.1 - o_sq.1 > 0 {
                self.obstacle_mod(o_sq, IsoDirections::S, cx)
            } else {
                self.obstacle_mod(o_sq, IsoDirections::N, cx)
            };
        }
        chance *= shelter_mod;
        let between = cx.world.vehicle_between((o.x, o.y, o.z + 0.1), (self.vars.x, self.vars.y, self.vars.z + 0.1));
        let len2 = length(o.x - self.vars.x, o.y - self.vars.y);
        let car_mod = if between && o.vehicle.is_none() {
            if len2 < 1.5 {
                0.5
            } else {
                0.0
            }
        } else {
            1.0
        };
        chance *= car_mod;
        chance /= self.zombie.worn_items_vision_modifier;
        if self.zombie.eat_body_target.is_some() && chance > 0.0 {
            chance *= 0.5;
        }
        self.spot_roll_and_react(o, forced, chance, view_dist, 720.0, true, cx)
    }

    /// `spottedOld(other, bForced)`.
    pub fn spotted_old(&mut self, other: i32, forced: bool, cx: &mut StateCx<'_, I>) -> ScriptResult<Option<SpotResult>> {
        let Some((o, my_sq, o_sq)) = self.spot_prelude(other, cx)? else { return Ok(None) };
        let ctx = cx.ctx;
        let mut chance = 200.0f32;
        let player_index = if o.player && !ctx.is_server() { o.player_index } else { 0 };
        let mut delta = cx.world.square_light(o_sq, player_index);
        let mut delta2 = cx.world.ambient_for_player(player_index);
        let mut mdelta = cx.world.square_light(my_sq, player_index);
        mdelta = mdelta * mdelta * mdelta;
        delta = delta.clamp(0.0, 1.0);
        mdelta = mdelta.clamp(0.0, 1.0);
        let difdelta = 1.0 - (delta - mdelta);
        if delta < 0.2 {
            delta = 0.2;
        }
        if delta2 < 0.2 {
            delta2 = 0.2;
        }
        let o_room = cx.world.square_room(o_sq);
        let my_room = cx.world.square_room(my_sq);
        if o_room != my_room {
            chance = 50.0;
            if o_room.is_some() && my_room.is_none() || o_room.is_none() && my_room.is_some() {
                chance = 20.0;
                if o.aiming || o.sneaking {
                    chance = if delta < 0.4 { 0.0 } else { 10.0 };
                } else if length(o.movement_last_frame.0, o.movement_last_frame.1) <= 0.04 && delta < 0.4 {
                    chance = 10.0;
                }
            }
        }
        let (mut tx, mut ty) = (o.x - self.vars.x, o.y - self.vars.y);
        if o_sq.2 != my_sq.2 {
            let dif = (o_sq.2 - my_sq.2).abs() * 5 + 1;
            chance /= dif as f32;
        }
        let mut view_dist = ctx.view_dist();
        let len = length(tx, ty);
        if len > view_dist {
            return Ok(None);
        }
        if ctx.is_server() {
            self.zombie.indoor_zombie = false;
        }
        if len < view_dist {
            view_dist = len;
        }
        view_dist *= 1.1;
        if view_dist > ctx.view_dist_max() {
            view_dist = ctx.view_dist_max();
        }
        (tx, ty) = normalize(tx, ty);
        let (fx, fy) = self.look_vector_exact();
        let cos_angle = fx * tx + fy * ty;
        self.update_vision_radius(cx);
        if self.dist_to_xy(o.x, o.y) > self.zombie.vision_radius_result {
            chance -= 10000.0;
        }
        if view_dist > 0.5 {
            chance = cone(cos_angle, chance);
        }
        self.mark_targeted(chance, cx);
        chance *= difdelta;
        let dif = fastfloor(o.z) - fastfloor(self.vars.z);
        if dif >= 1 {
            chance /= dif as f32 * 3.0;
        }
        let dist_alpha = (view_dist / ctx.view_dist()).clamp(0.0, 1.0);
        chance *= 1.0 - dist_alpha;
        chance *= 1.0 - dist_alpha;
        chance *= 1.0 - dist_alpha;
        let close_delta = (view_dist / 10.0).clamp(0.0, 1.0);
        chance *= 1.0 + (1.0 - close_delta) * 10.0;
        let movement = length(o.movement_last_frame.0, o.movement_last_frame.1);
        if o.torch_strength > 0.0 {
            chance *= 3.0;
        }
        if movement < 0.01 {
            chance *= 0.5;
        } else if o.sneaking {
            chance *= 0.4;
        } else if o.aiming {
            chance *= 0.75;
        } else if movement < 0.06 {
            chance *= 0.8;
        } else if movement >= 0.06 {
            chance *= 2.4;
        }
        if self.zombie.eat_body_target.is_some() {
            chance *= 0.6;
        }
        if view_dist < 5.0 && (!o.running && !o.sneaking && !o.aiming || o.running) {
            chance *= 3.0;
        }
        if self.zombie.spotted_last == Some(o.id) && self.vars.time_since_seen_flesh < 120.0 {
            chance = 1000.0;
        }
        chance *= o.sneak_spot_mod;
        chance *= delta2;
        if self.farther_than_target(&o, cx) {
            return Ok(None);
        }
        chance *= 0.3;
        if forced {
            chance = 1_000_000.0;
        }
        if self.zombie.bonus_spot_time > 0.0 {
            chance = 1_000_000.0;
        }
        chance *= 1.2;
        if self.zombie.sight == 1 {
            chance *= 2.5;
        }
        if self.zombie.sight == 3 {
            chance *= 0.45;
        }
        if self.zombie.inactive {
            chance *= 0.25;
        }
        chance *= 0.25;
        if o.player && o.inconspicuous {
            chance *= 0.5;
        }
        if o.player && o.conspicuous {
            chance *= 2.0;
        }
        chance *= 1.6;
        if my_sq != o_sq && o.player && o.sneaking {
            let x_diff = (my_sq.0 - o_sq.0).abs();
            let y_diff = (my_sq.1 - o_sq.1).abs();
            let adj = |d: IsoDirections| {
                let (dx, dy) = d.delta();
                (o_sq.0 + dx, o_sq.1 + dy, o_sq.2)
            };
            let exists = |sq: (i32, i32, i32)| cx.world.square(sq.0, sq.1, sq.2).is_some();
            let (square_test, solid_test) = if x_diff > y_diff {
                if my_sq.0 - o_sq.0 > 0 {
                    (Some(adj(IsoDirections::E)).filter(|s| exists(*s)), None)
                } else {
                    (Some(o_sq), Some(adj(IsoDirections::W)).filter(|s| exists(*s)))
                }
            } else if my_sq.1 - o_sq.1 > 0 {
                (Some(adj(IsoDirections::S)).filter(|s| exists(*s)), None)
            } else {
                (Some(o_sq), Some(adj(IsoDirections::N)).filter(|s| exists(*s)))
            };
            if let Some(st) = square_test {
                let mut bonus = o.near_wall;
                if bonus == 1.0 {
                    if let Some(solid) = solid_test {
                        bonus = cx.world.grid_sneak_modifier(solid, true);
                    }
                }
                if bonus > 1.0 {
                    let dist = distance_manhatten(st.0 as f32, st.1 as f32, o.x, o.y);
                    if dist > 1.0 {
                        bonus /= dist;
                    }
                    chance /= bonus;
                }
            }
        }
        chance /= self.zombie.worn_items_vision_modifier;
        if self.zombie.eat_body_target.is_some() && chance > 0.0 {
            chance *= 0.5;
        }
        self.spot_roll_and_react(o, forced, chance, view_dist, 120.0, false, cx)
    }

    /// `player.targetedByZombie = true` for the current target player in
    /// view.
    fn mark_targeted(&mut self, chance: f32, cx: &mut StateCx<'_, I>) {
        if chance <= 0.0 {
            return;
        }
        let Some(t) = self.vars.target else { return };
        let Some(sq) = self.current_square_xyz() else { return };
        let server = cx.ctx.is_server();
        let (is_player, remote, index) = match cx.world.character_mut(t) {
            Some(p) => (p.is_player(), p.seen.remote_id, p.seen.player_index),
            None => return,
        };
        if is_player && !server && remote == -1 && cx.world.square_can_see(sq, index) {
            if let Some(p) = cx.world.character_mut(t) {
                p.seen.targeted_by_zombie = true;
                p.seen.last_targeted = 0.0;
            }
        }
    }

    /// The other is farther (Manhattan) than the current, different
    /// target.
    fn farther_than_target(&mut self, o: &OtherView, cx: &mut StateCx<'_, I>) -> bool {
        let Some(t) = self.vars.target else { return false };
        if t == o.id {
            return false;
        }
        let Some((tx, ty)) = cx.world.character_mut(t).map(|t| (t.vars.x, t.vars.y)) else { return false };
        let dist_other = distance_manhatten(self.vars.x, self.vars.y, o.x, o.y);
        let dist_current = distance_manhatten(self.vars.x, self.vars.y, tx, ty);
        dist_other > dist_current
    }

    /// The roll and what follows it (shared by both variants).
    #[allow(clippy::too_many_arguments)]
    fn spot_roll_and_react(
        &mut self,
        o: OtherView,
        forced: bool,
        mut chance: f32,
        view_dist: f32,
        bonus_time: f32,
        update_can_see: bool,
        cx: &mut StateCx<'_, I>,
    ) -> ScriptResult<Option<SpotResult>> {
        let ctx = cx.ctx;
        chance = fastfloor(chance) as f32;
        chance = chance.min(400.0);
        chance /= 400.0;
        chance = 0.0f32.max(chance);
        chance = 1.0f32.min(chance);
        let mp = ctx.multiplier();
        chance = (1.0 - ((1.0 - chance) as f64).powf(mp as f64)) as f32;
        chance *= 100.0;
        let success = (ctx.rand_next(10000) as f32 / 100.0) < chance;
        let result = SpotResult { chance, success };
        if !success {
            if chance > 20.0 && o.player && view_dist < 15.0 {
                if let Some(p) = cx.world.character_mut(o.id) {
                    p.seen.could_be_seen_this_frame = true;
                }
            }
            let (could, seen) = cx
                .world
                .character_mut(o.id)
                .map_or((false, false), |p| (p.seen.could_be_seen_this_frame, p.seen.seen_this_frame));
            let can_add = o.player && !could && !seen && o.sneaking && o.just_moved;
            let random_multiplier = 1100.0f32;
            for perk in ["Sneak", "Lightfoot"] {
                if can_add && ctx.rand_next((random_multiplier * ctx.inv_multiplier()) as i32) as f32 == 0.0 && !ctx.is_server() && !ctx.is_client() {
                    let p_ref = cx.chars.perks.constant(perk);
                    if let Some(p) = cx.world.character_mut(o.id) {
                        p.add_xp(cx.sm, cx.chars, p_ref.as_ref(), 1.0, ctx)?;
                    }
                }
            }
            return Ok(Some(result));
        }
        if o.player {
            if let Some(p) = cx.world.character_mut(o.id) {
                p.seen.seen_this_frame = true;
            }
        }
        if !forced {
            self.zombie.bonus_spot_time = bonus_time;
        }
        self.zombie.last_target_seen_x = fastfloor(o.x);
        self.zombie.last_target_seen_y = fastfloor(o.y);
        self.zombie.last_target_seen_z = fastfloor(o.z);
        if self.is_current_state_id(ids::STAGGER_BACK) {
            return Ok(Some(result));
        }
        if self.vars.target != Some(o.id) {
            self.zombie.target_seen_time = 0.0;
            if ctx.is_server() && !self.zombie.remote {
                let now = ctx.current_time_millis();
                self.add_aggro(o.id, 1.0, now);
            }
        }
        self.set_zombie_target(Some(o.id));
        self.zombie.vector_to_target = (o.x - self.vars.x, o.y - self.vars.y);
        let vec_len = length(self.zombie.vector_to_target.0, self.zombie.vector_to_target.1);
        if !forced {
            self.vars.time_since_seen_flesh = 0.0;
            self.zombie.target_seen_time += ctx.realworld_seconds_since_last_update();
            if update_can_see {
                self.zombie.can_see_target = self.is_target_visible(cx);
            }
        }
        let same_as_last = self.zombie.spotted_last == Some(o.id);
        if same_as_last && self.is_current_state_id(ids::LUNGE) && self.zombie.lunge_timer > 0.0 {
            return Ok(Some(result));
        }
        if same_as_last && self.is_current_state_id(ATTACK_VEHICLE) {
            return Ok(Some(result));
        }
        if fastfloor(self.vars.z) == fastfloor(o.z)
            && (vec_len <= 3.5 || o.vehicle.is_some() && vec_len <= 4.0)
            && self.vars.state_event_delay_timer <= 0.0
            && !cx.world.line_clear_collide(self.vars.x, self.vars.y, o.x, o.y, fastfloor(self.vars.z), Some(o.id), super::vars::LCC_DEFAULT)
        {
            self.set_zombie_target(Some(o.id));
            if self.is_current_state_id(ids::LUNGE) {
                return Ok(Some(result));
            }
        }
        self.zombie.spotted_last = Some(o.id);
        let my_sq = self.current_square_xyz();
        if !self.zombie.ghost && !my_sq.is_some_and(|sq| cx.world.square_smoke(sq)) {
            self.set_zombie_target(Some(o.id));
            if self.zombie.allow_repath_delay > 0.0 {
                return Ok(Some(result));
            }
            if let Some(vehicle) = o.vehicle {
                if (self.is_current_state_id(ids::PATH_FIND) || self.is_current_state_id(ids::WALK_TOWARD))
                    && self.path_target_char(cx) == Some(o.id)
                {
                    return Ok(Some(result));
                }
                if self.is_current_state_id(ATTACK_VEHICLE) {
                    return Ok(Some(result));
                }
                if o.vehicle_speed_kmh.abs() > 0.8 && cx.world.vehicle_dist_squared(vehicle, self.vars.x, self.vars.y) <= 16.0 {
                    return Ok(Some(result));
                }
                self.zombie_path_to_character(o.id, cx);
                self.zombie.allow_repath_delay = 10.0;
                return Ok(Some(result));
            }
            self.zombie_path_to_character(o.id, cx);
            if ctx.rand_next(5) == 0 {
                self.zombie.spot_sound_delay = 200;
            }
            self.zombie.allow_repath_delay = 480.0;
        }
        Ok(Some(result))
    }

    /// `closeSneakBonusCoeff(sq, dir)`.
    fn close_sneak_bonus_coeff(&self, sq: (i32, i32, i32), dir: IsoDirections, cx: &StateCx<'_, I>) -> Option<f32> {
        let table = &cx.chars.sim.zombie_close_sneak_bonus;
        for name in cx.world.square_sprite_names(sq) {
            let Some((_, dirs, value)) = table.iter().find(|(n, _, _)| *n == name) else { continue };
            let letter = match dir {
                IsoDirections::N => "N",
                IsoDirections::S => "S",
                IsoDirections::W => "W",
                IsoDirections::E => "E",
                _ => continue,
            };
            if dirs.contains(letter) {
                return Some(*value / 100.0);
            }
        }
        None
    }

    /// `getObstacleMod(sq, dir)`.
    fn obstacle_mod(&self, sq: (i32, i32, i32), dir: IsoDirections, cx: &StateCx<'_, I>) -> f32 {
        if let Some(c) = self.close_sneak_bonus_coeff(sq, dir, cx) {
            return 1.0 - c;
        }
        let (dx, dy) = dir.delta();
        let next = (sq.0 + dx, sq.1 + dy, sq.2);
        if cx.world.square(next.0, next.1, next.2).is_some() {
            let next_dir = match dir {
                IsoDirections::S => IsoDirections::N,
                IsoDirections::E => IsoDirections::W,
                d => d,
            };
            if let Some(c) = self.close_sneak_bonus_coeff(next, next_dir, cx) {
                return 1.0 - c;
            }
        }
        1.0
    }

    /// `updateVisionRadius()`.
    pub fn update_vision_radius(&mut self, cx: &mut StateCx<'_, I>) {
        let ctx = cx.ctx;
        let rain = ctx.rain_intensity() * 2.5;
        let fog = ctx.fog_intensity() * 7.0;
        let target_player = self.vars.target.and_then(|t| {
            cx.world.character_mut(t).filter(|c| c.is_player()).map(|c| (c.path.current_square, c.seen.player_index))
        });
        let darkness = match target_player {
            Some((Some(sq), index)) => (1.0 - cx.world.square_light_level(sq, index)) * 5.0,
            _ => (1.0 - ctx.day_light_strength().min(ctx.ambient())) * 5.0,
        };
        let adjusted = self.vision_radius_adjusted(ctx, darkness, rain, fog);
        self.zombie.vision_radius_result = adjusted.clamp(10.0, 20.0);
    }

    /// `getVisionRadiusAdjusted(darkness, rain, fog)`.
    pub fn vision_radius_adjusted(&self, ctx: &dyn crate::SimContext, darkness: f32, rain: f32, fog: f32) -> f32 {
        let mut v = 20.0 - darkness.max(rain + fog);
        if self.zombie.sight == 1 || ctx.lore_sight() == 1 {
            v *= 1.75;
        }
        if self.zombie.sight == 3 || ctx.lore_sight() == 3 {
            v *= 0.35;
        }
        v /= self.zombie.worn_items_vision_modifier;
        if self.zombie.eat_body_target.is_some() {
            v *= 0.5;
        }
        v
    }

    /// `isTargetVisible()`: the target is a player who could see the
    /// zombie's square.
    pub fn is_target_visible(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        let Some(t) = self.vars.target else { return false };
        let Some(sq) = self.current_square_xyz() else { return false };
        let index = match cx.world.character_mut(t) {
            Some(p) if p.is_player() => p.seen.player_index,
            _ => return false,
        };
        cx.world.square_could_see(sq, index)
    }

    /// The zombie as `WorldSoundManager` hears with it.
    pub fn listener(&self, cx: &StateCx<'_, I>) -> Listener {
        let ctx = cx.ctx;
        let square = match self.current_square_xyz() {
            None => ListenerSquare::None,
            Some(sq) => {
                let (wx, wy) = (sq.0.div_euclid(8), sq.1.div_euclid(8));
                if cx.world.chunk_loaded(wx, wy) {
                    ListenerSquare::Chunk(wx, wy)
                } else {
                    ListenerSquare::NoChunk
                }
            }
        };
        let weather = 1.0 - ctx.rain_intensity() * 0.33 - ctx.fog_intensity() * 0.1;
        Listener {
            x: self.vars.x,
            y: self.vars.y,
            z: self.vars.z,
            square,
            hearing: self.zombie.hearing,
            hearing_scale: self.worn_items_hearing_multiplier() * weather,
            sound_source_target: self.zombie.sound_source_target,
            wild: false,
        }
    }

    /// `isRespondingToPlayerSound()`.
    pub fn is_responding_to_player_sound(&self) -> bool {
        self.zombie.time_since_respond_to_sound < 1000.0 && (self.zombie.sound_source_is_player || self.zombie.sound_source_is_player_base)
    }

    /// `isMovingToPlayerSound()` with the behaviour's goal.
    pub fn is_moving_to_player_sound_with(&self, goal_sound: bool) -> bool {
        if (self.is_current_state_id(ids::PATH_FIND) || self.is_current_state_id(ids::WALK_TOWARD)) && goal_sound {
            return self.zombie.sound_source_is_player || self.zombie.sound_source_is_player_base;
        }
        false
    }

    /// `RespondToSound()`.
    pub fn respond_to_sound(&mut self, cx: &mut StateCx<'_, I>) {
        let ctx = cx.ctx;
        if self.zombie.ghost || self.zombie.useless || ctx.is_server() || ctx.is_client() && self.zombie.remote {
            return;
        }
        let me = (fastfloor(self.vars.x), fastfloor(self.vars.y), fastfloor(self.vars.z));
        if (self.is_current_state_id(ids::PATH_FIND) || self.is_current_state_id(ids::WALK_TOWARD))
            && self.path_goal_sound(cx)
            && fastfloor(self.vars.z) == self.path_target(cx).2
            && self.zombie.sound_source_repeating
        {
            let (px, py, _) = self.path_target(cx);
            let d2 = self.dist_to_squared_xy(px as f32, py as f32);
            if d2 < 16.0 && cx.world.los_line_clear(me, (px, py, me.2), false) != LosResult::Blocked && !self.environment.near_siren_vehicle {
                self.set_var_bool("bPathfind", false);
                self.set_var_bool("bMoving", false);
                self.clear_path2();
            }
        }
        if self.zombie.sound_react_delay > 0.0 {
            self.zombie.sound_react_delay -= ctx.thirty_fps_multiplier();
            if self.zombie.sound_react_delay < 0.0 {
                self.zombie.sound_react_delay = 0.0;
            }
            if self.zombie.sound_react_delay > 0.0 {
                return;
            }
        }
        let ear = self.listener(cx);
        let (picked, tmp_attract) = {
            let Some((mgr, sw)) = cx.world.world_sounds() else { return self.react_to_delayed_sound(cx) };
            let mut sound = mgr.get_sound_zomb(sw, &ear);
            let attract = mgr.get_sound_attract(sw, sound, &ear);
            if attract <= 0.0 {
                sound = None;
            }
            let mut tmp_attract = 0.0f32;
            if sound.is_some() {
                tmp_attract = attract;
                self.zombie.sound_attract = tmp_attract;
                self.zombie.sound_attract_timeout = 60.0;
            } else if self.zombie.sound_attract_timeout > 0.0 {
                self.zombie.sound_attract_timeout -= ctx.thirty_fps_multiplier();
                if self.zombie.sound_attract_timeout < 0.0 {
                    self.zombie.sound_attract_timeout = 0.0;
                }
            }
            let b = mgr.get_biggest_sound_zomb(sw, me.0, me.1, me.2, true, Some(&ear));
            if b.sound.is_some() && (self.zombie.sound_attract_timeout == 0.0 || self.zombie.sound_attract * 2.0 < b.attract) {
                sound = b.sound;
                tmp_attract = b.attract;
            }
            (sound.map(|s| mgr.sound(s).clone()), tmp_attract)
        };
        let mut sound = picked;
        if let Some(s) = &sound {
            if s.repeating && s.z == fastfloor(self.vars.z) {
                let d = self.dist_to_squared_xy(s.x as f32, s.y as f32);
                if d < 25.0 && cx.world.los_line_clear(me, (s.x, s.y, me.2), false) != LosResult::Blocked {
                    sound = None;
                }
            }
        }
        if self.should_stop_thumping_to_respond_to_sound(sound.as_ref(), cx) {
            self.zombie.thump_target = None;
        }
        if let Some(s) = &sound {
            self.zombie.sound_attract = tmp_attract;
            self.zombie.sound_source_target = s.source.map(|src| src.id);
            self.zombie.sound_source_target_is_zombie = s.source.is_some_and(|src| src.is_zombie);
            self.zombie.sound_react_delay = ctx.rand_next_range(0, 16) as f32;
            self.zombie.delayed_sound = (s.x, s.y, s.z);
            self.zombie.sound_source_repeating = s.repeating;
            self.zombie.sound_source_is_player = s.source_is_player;
            self.zombie.sound_source_is_player_base = s.source_is_player_base;
        }
        self.react_to_delayed_sound(cx);
    }

    /// The second half of `RespondToSound`: head for a sound once its
    /// reaction delay ran out.
    fn react_to_delayed_sound(&mut self, cx: &mut StateCx<'_, I>) {
        let ctx = cx.ctx;
        if self.zombie.delayed_sound.0 == -1 || self.zombie.sound_react_delay != 0.0 {
            return;
        }
        let (mut sx, mut sy, sz) = self.zombie.delayed_sound;
        self.zombie.delayed_sound.0 = -1;
        let dist3 = distance_manhatten(self.vars.x, self.vars.y, sx as f32, sy as f32) / 2.5;
        sx += ctx.rand_next_range(-dist3 as i32, dist3 as i32);
        sy += ctx.rand_next_range(-dist3 as i32, dist3 as i32);
        if (self.is_current_state_id(ids::PATH_FIND) || self.is_current_state_id(ids::WALK_TOWARD))
            && (self.path_goal_location(cx) || self.path_goal_sound(cx))
        {
            let (tx, ty, _) = self.path_target_f(cx);
            if !is_similar_direction(self.vars.x, self.vars.y, sx as f32, sy as f32, tx, ty, 0.5) {
                self.set_turn_alerted_values(sx, sy);
                self.zombie_path_to_sound(sx, sy, sz, cx);
                let pt = self.path_target(cx);
                self.zombie.last_heard_sound = pt;
                self.zombie.allow_repath_delay = 120.0;
                self.zombie.time_since_respond_to_sound = 0.0;
            }
            return;
        }
        if self.zombie.time_since_respond_to_sound < 60.0 {
            return;
        }
        let (fx, fy) = (self.vars.forward_x, self.vars.forward_y);
        if !is_similar_direction(self.vars.x, self.vars.y, sx as f32, sy as f32, self.vars.x + fx, self.vars.y + fy, 0.5) {
            self.set_turn_alerted_values(sx, sy);
        }
        let me = (fastfloor(self.vars.x), fastfloor(self.vars.y), fastfloor(self.vars.z));
        if cx.world.los_line_clear(me, (sx, sy, sz), false) != LosResult::Blocked {
            self.zombie_path_to_sound(sx, sy, sz, cx);
            self.zombie.last_heard_sound = self.path_target(cx);
        } else {
            let mut offset = 2;
            if ctx.rain_intensity() > 0.5 {
                offset = 5;
            }
            if ctx.lore_hearing() == 1 {
                offset -= 2;
            }
            if ctx.lore_hearing() == 3 {
                offset += 2;
            }
            offset = 10.min(offset).max(2);
            let heard_x = -offset + (ctx.math_random() * (offset * 2 + 1) as f64) as i32;
            let heard_y = -offset + (ctx.math_random() * (offset * 2 + 1) as f64) as i32;
            self.zombie_path_to_sound(sx + heard_x, sy + heard_y, sz, cx);
            let pt = self.path_target(cx);
            self.zombie.last_heard_sound = (pt.0 + heard_x, pt.1 + heard_y, pt.2);
        }
        self.zombie.allow_repath_delay = 120.0;
        self.zombie.time_since_respond_to_sound = 0.0;
    }

    /// `shouldStopThumpingToRespondToSound(sound)`.
    fn should_stop_thumping_to_respond_to_sound(&mut self, sound: Option<&WorldSound>, cx: &mut StateCx<'_, I>) -> bool {
        let Some(s) = sound else { return false };
        if s.source_is_zombie || s.radius < 10 || !self.is_current_state_id(ids::THUMP) {
            return false;
        }
        if let Some(t) = self.vars.target {
            if let Some((tx, ty)) = cx.world.character_mut(t).map(|c| (c.vars.x, c.vars.y)) {
                if self.is_target_location_known() && self.dot_with_forward_direction(tx, ty) > 0.0 {
                    return false;
                }
                if !self.is_target_location_known()
                    && self.dot_with_forward_direction(self.zombie.last_target_seen_x as f32, self.zombie.last_target_seen_y as f32) > 0.0
                {
                    return false;
                }
            }
        }
        self.dot_with_forward_direction(s.x as f32, s.y as f32) < 0.0
    }

    /// `setTurnAlertedValues(soundX, soundY)`: the `turnalertedvalue`
    /// variable and `ZombieTurnAlerted`'s target angle.
    pub fn set_turn_alerted_values(&mut self, sound_x: i32, sound_y: i32) {
        let (dx, dy) = (self.vars.x - (sound_x as f32 + 0.5), self.vars.y - (sound_y as f32 + 0.5));
        let mut rad_sound = direction_neg(dx, dy);
        rad_sound = if rad_sound < 0.0 { rad_sound.abs() } else { (std::f64::consts::PI * 2.0 - rad_sound as f64) as f32 };
        let mut deg_sound = (rad_sound as f64).to_degrees();
        let (zx, zy) = dir_vector(self.vars.dir.rot180());
        let (zx, zy) = normalize(zx, zy);
        let mut rad_z = direction_neg(zx, zy);
        rad_z = if rad_z < 0.0 { rad_z.abs() } else { std::f32::consts::PI * 2.0 - rad_z };
        let mut deg_z = (rad_z as f64).to_degrees();
        if deg_z as i32 == 360 {
            deg_z = 0.0;
        }
        if deg_sound as i32 == 360 {
            deg_sound = 0.0;
        }
        let mut t = "0";
        if deg_sound > deg_z {
            let s = (deg_sound - deg_z) as i32;
            if s > 350 || s <= 35 {
                t = "45R";
            }
            if s > 35 && s <= 80 {
                t = "90R";
            }
            if s > 80 && s <= 125 {
                t = "135R";
            }
            if s > 125 && s <= 170 {
                t = "180R";
            }
            if s > 170 && s < 215 {
                t = "180L";
            }
            if (215..260).contains(&s) {
                t = "135L";
            }
            if (260..305).contains(&s) {
                t = "90L";
            }
            if (305..350).contains(&s) {
                t = "45L";
            }
        } else {
            let s = (deg_z - deg_sound) as i32;
            if s > 10 && s <= 55 {
                t = "45L";
            }
            if s > 55 && s <= 100 {
                t = "90L";
            }
            if s > 100 && s <= 145 {
                t = "135L";
            }
            if s > 145 && s <= 190 {
                t = "180L";
            }
            if s > 190 && s < 235 {
                t = "180R";
            }
            if (235..280).contains(&s) {
                t = "135R";
            }
            if (280..325).contains(&s) {
                t = "90R";
            }
            if !(10..325).contains(&s) {
                t = "45R";
            }
        }
        self.zombie.turn_alerted_value = t.to_string();
        self.set_var_str("turnalertedvalue", t);
        let angle = direction(sound_x as f32 + 0.5 - self.vars.x, sound_y as f32 + 0.5 - self.vars.y);
        let id = ids::ZOMBIE_TURN_ALERTED;
        self.state.params.clear(id);
        crate::character::states::zombie::TURN_ALERTED_TARGET_ANGLE.set(&mut self.state.params, angle);
        self.zombie.alerted = true;
    }
}

/// `AttackVehicleState` (not ported; compared by id).
const ATTACK_VEHICLE: crate::character::state::StateId = crate::character::state::StateId("zombie.vehicles.AttackVehicleState");
