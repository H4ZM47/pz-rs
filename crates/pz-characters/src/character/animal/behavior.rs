//! `zombie.characters.animals.behavior.BaseAnimalBehavior` on the animal's
//! character: its fields are `pz-animals`' [`AnimalBehavior`]
//! (`AnimalFields::animal.behavior`), its methods are `behavior_*` methods
//! of [`GameCharacter`] that move the animal through the state machine and
//! the path finder.
//!
//! The behaviour's world lookups (designation zones, troughs, hutches,
//! food on the ground, water, puddles and grass) go through `pz-animals`'
//! `AnimalWorld` ([`StateWorld::animal_host`]); squares and other
//! characters through the [`StateWorld`].
//!
//! [`AnimalBehavior`]: pz_animals::AnimalBehavior
//! [`StateWorld::animal_host`]: crate::character::state::StateWorld::animal_host
//! [`StateWorld`]: crate::character::state::StateWorld

use pz_animals::{
    character_ref, character_ref_id, square_ref, square_ref_xyz, BehaviorAction, BehaviorObject,
    EatFollowUp,
};
use pz_core::random::JavaRandom;

use super::actions::{dir_from_angle, dist_proper, sq_dist_proper};
use crate::character::fire::CtxRand;
use crate::character::state::StateCx;
use crate::character::states::ids;
use crate::character::zombie::math::{
    distance_manhatten, distance_to, fastfloor, length, normalize, set_length,
};
use crate::character::GameCharacter;

/// `Rand.NextBool(n)`.
fn next_bool<I>(cx: &StateCx<'_, I>, n: i32) -> bool {
    cx.ctx.rand_next(n) == 0
}

/// `shuffleList(list)` / `shuffleListSq(list)`: a fresh `java.util.Random`,
/// its first `nextInt()` discarded, then a forward Fisher-Yates.
fn shuffle<T>(seed: i64, a: &mut [T]) {
    let r = JavaRandom::new(seed);
    r.next_int_unbounded();
    let n = a.len();
    for i in 0..n {
        let change = i + r.next_int((n - i) as i32) as usize;
        a.swap(i, change);
    }
}

/// `canEatThis(item)` for the definition's `eatTypeTrough`.
fn can_eat_this(types: &[String], item: &pz_animals::env::FeedItem) -> bool {
    if types.iter().any(|t| t == "All" || *t == item.full_type)
        || item
            .animal_feed_type
            .as_ref()
            .is_some_and(|f| types.contains(f))
    {
        return true;
    }
    match &item.kind {
        pz_animals::env::FeedItemKind::Food { food_type, .. } => {
            food_type.as_ref().is_some_and(|f| types.contains(f))
        }
        _ => false,
    }
}

impl<I: 'static> GameCharacter<I> {
    fn beh(&self) -> &pz_animals::AnimalBehavior {
        &self.af().animal.behavior
    }

    fn beh_mut(&mut self) -> &mut pz_animals::AnimalBehavior {
        &mut self.af_mut().animal.behavior
    }

    fn set_doing_behavior(&mut self, b: bool) {
        let v = self.path.vehicle.is_some();
        self.beh_mut().set_doing_behavior(b, v);
    }

    fn own_square(&self) -> Option<(i32, i32, i32)> {
        self.path.current_square
    }

    /// `pickRandomWanderInterval()`.
    pub fn behavior_pick_random_wander_interval(&self, cx: &StateCx<'_, I>) -> f32 {
        let adef = self.adef();
        let ultra = cx
            .ctx
            .animal_sandbox()
            .ultra_speed_doesnot_affect_to_animals;
        let raining = cx.ctx.rain_manager_is_raining();
        self.af().animal.wander_interval(
            &adef,
            ultra,
            cx.ctx.multiplier(),
            raining,
            &mut CtxRand(cx.ctx),
        )
    }

    /// The mother's character id.
    fn mother_id(&self) -> Option<i32> {
        self.af().animal.mother.and_then(character_ref_id)
    }

    /// `wanderIdle()`.
    pub fn behavior_wander_idle(&mut self, cx: &mut StateCx<'_, I>) {
        let m = cx.ctx.multiplier();
        {
            let b = self.beh_mut();
            if b.block_movement {
                b.blocked_for += m;
                if b.blocked_for >= 8000.0 {
                    b.block_movement = false;
                    b.blocked_for = 0.0;
                }
            }
        }
        {
            let f = self.af();
            let b = &f.animal.behavior;
            if b.is_doing_behavior
                || b.block_movement
                || f.animal.alerted
                || f.animal.data.going_to_mom
                || self.animal_is_eating()
                || self.animal_is_moving()
                || f.last_sound_responded_to.is_some()
                || f.walk_to_char_luring
                || f.animal.drink_from_trough.is_some()
                || f.animal.eat_from_trough.is_some()
                || f.animal.eat_from_ground.is_some()
            {
                return;
            }
        }
        if self.vars.on_fire && !self.animal_is_moving() {
            self.set_var_bool("animalRunning", true);
            let Some(sq) = self.own_square() else {
                return;
            };
            let x = cx.ctx.rand_next_range(sq.0 - 10, sq.0 + 10);
            let y = cx.ctx.rand_next_range(sq.1 - 10, sq.1 + 10);
            self.animal_path_to_location(x, y, sq.2, cx);
            return;
        }
        if self.vars.state_event_delay_timer < -1000.0 && self.animal_is_moving() {
            self.vars.state_event_delay_timer = 0.0;
            self.animal_stop_all_movement_now(cx);
            if cx.has_host_error() { return; }
        }
        let adef = self.adef();
        if adef.sit_randomly {
            self.behavior_check_sit(cx);
        }
        if self.animal_is_sitting() {
            if next_bool(cx, adef.idle_emote_chance)
                && self.variable_string("sittingAnim").is_empty()
                && adef.sitting_type_nbr > 0
            {
                let n = cx.ctx.rand_next_range(1, adef.sitting_type_nbr + 1);
                self.set_var_str("sittingAnim", &format!("sit{n}"));
            }
            return;
        }
        if let Some(mid) = self.mother_id() {
            let msq = self.other(mid, cx).and_then(|o| o.square);
            if let Some(msq) = msq {
                if self.animal_can_go_there(msq, cx) {
                    if self.beh().go_to_mom_timer > 0.0 {
                        self.beh_mut().go_to_mom_timer -= m;
                    } else {
                        let dist = self.own_square().map_or(-1.0, |s| sq_dist_proper(s, msq));
                        if dist >= 6.0 && self.current_state_id() == Some(ids::ANIMAL_IDLE) {
                            self.set_var_bool("animalRunning", true);
                            self.beh_mut().go_to_mom_timer = 300.0;
                            self.animal_path_to_location(msq.0, msq.1, msq.2, cx);
                            return;
                        }
                    }
                }
            }
        }
        {
            let b = self.beh_mut();
            if b.idle_anim_timer > 0.0 {
                b.idle_anim_timer -= m;
                if b.idle_anim_timer < 0.0 {
                    b.idle_anim_timer = 0.0;
                }
            }
        }
        if next_bool(cx, adef.idle_emote_chance)
            && self.variable_string("idleAction").is_empty()
            && self.beh().idle_anim_timer == 0.0
        {
            if self.af().animal.stress_level < 10.0 && adef.happy_anim > 0 && next_bool(cx, 3) {
                let n = cx.ctx.rand_next_range(1, adef.happy_anim + 1);
                self.set_var_str("idleAction", &format!("happy{n}"));
            } else if adef.idle_type_nbr > 0 {
                let n = cx.ctx.rand_next_range(1, adef.idle_type_nbr + 1);
                self.set_var_str("idleAction", &format!("idle{n}"));
            }
            self.beh_mut().idle_anim_timer = 1500.0;
        }
        if self.af().animal.moving_to_food.is_some() {
            self.vars.state_event_delay_timer = self.behavior_pick_random_wander_interval(cx);
        }
        if self.vars.state_event_delay_timer <= 0.0 {
            self.behavior_check_behavior(cx);
            if cx.has_host_error() { return; }
            self.set_var_bool("animalRunning", false);
            let mut x_ref = self.vars.x as i32;
            let mut y_ref = self.vars.y as i32;
            let d = &self.af().animal.data;
            if let Some(p) = d.attached_player.and_then(character_ref_id) {
                if let Some(o) = self.other(p, cx) {
                    x_ref = fastfloor(o.x);
                    y_ref = fastfloor(o.y);
                }
            } else if d.attached_tree.is_some() {
                x_ref = d.attached_tree_x;
                y_ref = d.attached_tree_y;
            } else if let Some(msq) = self
                .mother_id()
                .and_then(|mid| self.other(mid, cx))
                .and_then(|o| o.square)
            {
                x_ref = msq.0;
                y_ref = msq.1;
            }
            self.vars.state_event_delay_timer = self.behavior_pick_random_wander_interval(cx);
            let (h, t) = self.hunger_thirst();
            if h > 0.3 {
                self.vars.state_event_delay_timer = 200.0;
            }
            let mut x = x_ref + cx.ctx.rand_next(16) - 8;
            let mut y = y_ref + cx.ctx.rand_next(16) - 8;
            if self.af().animal.d_zone.is_some() && h < 0.9 && t < 0.9 {
                let z = self.own_square().map_or(0, |s| s.2);
                let zone_at = |cx: &mut StateCx<'_, I>, id: i32, x: i32, y: i32| {
                    cx.world
                        .animal_host(id)
                        .and_then(|h| h.world.zone_at(x as f32, y as f32, z as f32))
                };
                let mut zone = zone_at(cx, self.id, x, y);
                let mut tries = 0;
                while zone.is_none() && tries < 100 {
                    x = x_ref + cx.ctx.rand_next(16) - 8;
                    y = y_ref + cx.ctx.rand_next(16) - 8;
                    zone = zone_at(cx, self.id, x, y);
                    tries += 1;
                }
                if cx.ctx.rain_manager_is_raining() && cx.ctx.rain_manager_intensity() as f64 > 0.05
                {
                    let mut roofed = Vec::new();
                    for z in self.af().animal.connected_d_zone.clone() {
                        let areas = cx
                            .world
                            .animal_host(self.id)
                            .map(|h| h.world.zone_roof_areas(z))
                            .unwrap_or_default();
                        if areas.is_empty() || roofed.iter().any(|(r, _)| *r == z) {
                            continue;
                        }
                        roofed.push((z, areas));
                    }
                    if !roofed.is_empty() {
                        let i = cx.ctx.rand_next_range(0, roofed.len() as i32) as usize;
                        let areas = &roofed[i].1;
                        let p = areas[cx.ctx.rand_next_range(0, areas.len() as i32) as usize];
                        x = p.0 as i32;
                        y = p.1 as i32;
                    }
                }
            }
            let zi = self.zi();
            if cx.world.square(x, y, zi).is_some() && cx.world.square_is_free((x, y, zi)) {
                if adef.periodic_run && next_bool(cx, 5) {
                    self.set_var_bool("animalRunning", true);
                }
                self.animal_path_to_location(x, y, zi, cx);
            }
        }
    }

    /// `walkedOnSpot()`.
    pub fn behavior_walked_on_spot(&mut self, cx: &mut StateCx<'_, I>) {
        self.vars.moving = false;
        cx.world.path_reset(self);
        self.vars.state_event_delay_timer = 200.0;
    }

    /// `goAttack(opponent)`.
    pub fn behavior_go_attack(&mut self, opponent: i32, cx: &mut StateCx<'_, I>) {
        if self.beh().block_movement
            || self.beh().behavior_action == Some(BehaviorAction::FightAnimal)
        {
            return;
        }
        let o = self.other(opponent, cx);
        let ok = o
            .as_ref()
            .is_some_and(|o| !o.dead && o.square.is_some() && o.exists && !o.vehicle);
        if !ok {
            self.af_mut().fighting_opponent = None;
            self.beh_mut().attack_animal_timer = 0.0;
            return;
        }
        let o = o.expect("checked");
        let b = self.beh_mut();
        b.behavior_action = Some(BehaviorAction::FightAnimal);
        b.behavior_object = Some(BehaviorObject::Character(character_ref(opponent)));
        self.set_doing_behavior(true);
        self.animal_stop_all_movement_now(cx);
        if cx.has_host_error() { return; }
        self.animal_path_to_character(opponent, cx);
        if o.animal {
            let me = self.id;
            if let Some(f) = cx
                .world
                .character_mut(opponent)
                .and_then(|c| c.animal.as_mut())
            {
                f.fighting_opponent = Some(me);
                f.animal.behavior.block_movement = true;
            }
        }
    }

    /// `checkSit()`.
    pub fn behavior_check_sit(&mut self, cx: &mut StateCx<'_, I>) {
        let (h, _) = self.hunger_thirst();
        let attached = self.af().animal.data.attached_player.is_some();
        if h > 0.2 || attached {
            self.clear_var("idleAction");
            let b = self.beh_mut();
            b.sit_in_time = 0;
            b.sit_out_time = 0;
        }
        let Some(sq) = self.own_square() else {
            return;
        };
        let roof = cx.world.square_have_roof(sq);
        let raining = cx.ctx.rain_manager_is_raining();
        if raining && cx.ctx.rain_manager_intensity() > 0.05 && !roof {
            self.clear_var("idleAction");
            let b = self.beh_mut();
            b.sit_in_time = 0;
            b.sit_out_time = 0;
        }
        if h > 0.3 || attached {
            return;
        }
        if raining && !roof {
            return;
        }
        let secs = cx.ctx.calendar_time_in_millis() / 1000;
        if self.beh().sit_in_time == 0 && self.beh().sit_out_time == 0 {
            let a = cx.ctx.rand_next_range(10800, 25200);
            let sit_in = (secs as i32).wrapping_add(a);
            let b = cx.ctx.rand_next_range(3600, 7200);
            let bh = self.beh_mut();
            bh.sit_in_time = sit_in;
            bh.sit_out_time = sit_in.wrapping_add(b);
        }
        if self.beh().sit_in_time > 0 && secs > self.beh().sit_in_time as i64 {
            self.set_var_str("idleAction", "sit");
            self.beh_mut().sit_in_time = 0;
        }
        if self.beh().sit_out_time > 0 && secs > self.beh().sit_out_time as i64 {
            self.clear_var("idleAction");
            self.beh_mut().sit_out_time = 0;
        }
    }

    /// `BaseAnimalBehavior.update()`.
    pub fn behavior_update(&mut self, cx: &mut StateCx<'_, I>) {
        self.behavior_flee_from_attacker(cx);
        self.behavior_flee_from_chr(cx);
        self.behavior_follow_chr(cx);
        let m = cx.ctx.multiplier();
        self.beh_mut().update_attack_timer(m);
        self.behavior_update_acceptance(cx);
        self.behavior_update_going_to_hutch(cx);
        if cx.has_host_error() { return; }
        let (h, t) = self.hunger_thirst();
        if self.beh().behavior_check_timer > 150.0 && (h > 0.5 || t > 0.5) {
            self.beh_mut().behavior_check_timer = 150.0;
        }
        if self.beh().behavior_check_timer > 0.0 {
            let b = self.beh_mut();
            b.behavior_check_timer -= m;
            if b.behavior_check_timer < 0.0 {
                b.behavior_check_timer = 0.0;
            }
        } else {
            self.beh_mut().behavior_check_timer = 500.0;
            self.behavior_check_behavior(cx);
            if cx.has_host_error() { return; }
        }
        if self.beh().is_doing_behavior {
            self.beh_mut().behavior_failsafe += m;
            if self.beh().behavior_failsafe > self.beh().behavior_max_time {
                self.beh_mut().behavior_failsafe = 0.0;
                self.set_doing_behavior(false);
                let b = self.beh_mut();
                b.behavior_action = None;
                b.behavior_object = None;
            }
        }
    }

    fn behavior_update_going_to_hutch(&mut self, cx: &mut StateCx<'_, I>) {
        let b = self.beh();
        if !(b.is_doing_behavior && b.behavior_action == Some(BehaviorAction::EnterHutch)) {
            return;
        }
        let Some(BehaviorObject::Hutch(h)) = b.behavior_object else {
            return;
        };
        if b.hutch_path_timer <= -1.0 {
            return;
        }
        let m = cx.ctx.multiplier();
        self.beh_mut().hutch_path_timer -= m;
        if self.beh().hutch_path_timer <= 0.0 {
            self.animal_stop_all_movement_now(cx);
            if cx.has_host_error() { return; }
            let target = cx.world.animal_host(self.id).and_then(|w| {
                let sq = w.world.hutch_square(h).and_then(square_ref_xyz)?;
                let (ex, ey) = w.world.hutch_enter_spot(h);
                Some((sq.0 + ex, sq.1 + ey, sq.2))
            });
            if let Some((x, y, z)) = target {
                self.animal_path_to_location(x, y, z, cx);
            }
            self.beh_mut().hutch_path_timer = -1.0;
        }
    }

    /// `doBehaviorAction()`.
    pub fn behavior_do_behavior_action(&mut self, cx: &mut StateCx<'_, I>) {
        if self.beh().is_doing_behavior {
            self.set_doing_behavior(false);
            let Some(action) = self.beh().behavior_action else {
                // System.out.println("action was null, object: ...")
                return;
            };
            match action {
                BehaviorAction::EatTrough => self.behavior_eat_from_trough(cx),
                BehaviorAction::EatGround => self.behavior_eat_from_ground(cx),
                BehaviorAction::DrinkGround => self.behavior_eat_from_ground(cx),
                BehaviorAction::EatMom => {
                    self.behavior_eat_from_mom_mother(cx);
                    self.behavior_eat_from_mom();
                }
                BehaviorAction::Drink => self.behavior_drink_from_trough(cx),
                BehaviorAction::Fertilize => self.behavior_fertilize(cx),
                BehaviorAction::EnterHutch => self.behavior_enter_hutch(cx),
                BehaviorAction::FightAnimal => self.behavior_fight_animal(cx),
                BehaviorAction::DrinkFromRiver => self.behavior_drink_square(cx, true),
                BehaviorAction::DrinkFromPuddle => self.behavior_drink_square(cx, false),
                BehaviorAction::EatGrass => self.behavior_eat_grass(cx),
            }
            if cx.has_host_error() { return; }
            self.behavior_reset_behavior_action();
        }
        self.beh_mut().timer_flee_again = 0.0;
    }

    /// `fightAnimal()`.
    pub fn behavior_fight_animal(&mut self, cx: &mut StateCx<'_, I>) {
        let opp = self.af().fighting_opponent.and_then(|o| self.other(o, cx));
        let Some(o) = opp.filter(|o| !o.dead && o.square.is_some() && o.exists) else {
            if let Some(oid) = self.af().fighting_opponent {
                if let Some(f) = cx.world.character_mut(oid).and_then(|c| c.animal.as_mut()) {
                    f.fighting_opponent = None;
                }
            }
            self.af_mut().fighting_opponent = None;
            return;
        };
        let adef = self.adef();
        if dist_proper((o.x, o.y), (self.vars.x, self.vars.y)) <= adef.attack_dist as f32 {
            self.animal_stop_all_movement_now(cx);
            if cx.has_host_error() { return; }
            self.face_character(o.x, o.y);
            self.af_mut().atk_target = Some(o.id);
            self.beh_mut().attack_animal_timer = adef.attack_timer as f32;
            let me = self.id;
            let mut animal = false;
            if let Some(c) = cx.world.character_mut(o.id) {
                let t = c
                    .animal
                    .as_ref()
                    .and_then(|f| f.def.as_ref())
                    .map(|d| d.adef.attack_timer);
                if let Some(f) = c.animal.as_mut() {
                    animal = true;
                    f.atk_target = Some(me);
                    f.animal.behavior.attack_animal_timer = t.unwrap_or(0) as f32;
                    f.animal.behavior.block_movement = true;
                }
            }
            if !animal {
                self.beh_mut().attack_animal_timer = 1500.0;
            }
        }
        if let Some(f) = cx.world.character_mut(o.id).and_then(|c| c.animal.as_mut()) {
            f.fighting_opponent = None;
        }
        self.af_mut().fighting_opponent = None;
    }

    /// `enterHutch()`.
    fn behavior_enter_hutch(&mut self, cx: &mut StateCx<'_, I>) {
        let Some(BehaviorObject::Hutch(h)) = self.beh().behavior_object else {
            return;
        };
        let Some(own) = self.own_square() else {
            return;
        };
        let added = cx.world.animal_host(self.id).is_some_and(|w| {
            let Some(hsq) = w.world.hutch_square(h).and_then(square_ref_xyz) else {
                return false;
            };
            !w.world.hutch_door_closed(h)
                && w.world.hutch_inside_count(h) < w.world.hutch_max_animals(h)
                && sq_dist_proper(hsq, own) < 2.0
                && w.world.hutch_add_animal_inside(h)
        });
        if added {
            if let Some(mut host) = cx.world.animal_host(self.id) {
                self.animal_remove_from_world(&mut host);
            }
            // removeFromSquare.
            self.path.current_square = None;
            self.af_mut().exists_in_world = false;
        }
    }

    /// `resetBehaviorAction()`.
    pub fn behavior_reset_behavior_action(&mut self) {
        self.vars.turn_delta_normal = self.adef().turn_delta;
        let b = self.beh_mut();
        b.behavior_action = None;
        b.behavior_object = None;
        self.af_mut().fighting_opponent = None;
    }

    /// `fertilize()`.
    fn behavior_fertilize(&mut self, cx: &mut StateCx<'_, I>) {
        let Some(BehaviorObject::Animal(r)) = self.beh().behavior_object else {
            return;
        };
        let Some(fid) = character_ref_id(r) else {
            return;
        };
        let fsq = cx.world.character_mut(fid).map(|c| {
            if let Some(f) = c.animal.as_mut() {
                f.animal.behavior.block_movement = false;
            }
            c.path.current_square
        });
        if let (Some(Some(fsq)), Some(own)) = (fsq, self.own_square()) {
            if sq_dist_proper(fsq, own) >= 3.0 {
                self.behavior_reset_behavior_action();
                return;
            }
        }
        let ctx = cx.ctx;
        let Some(mut female) = cx.world.take_character(fid) else {
            return;
        };
        let mut failure = None;
        if let Some(host) = cx.world.animal_host(self.id) {
            let clock = super::CtxClock(ctx);
            let sandbox = ctx.animal_sandbox();
            let actx = pz_animals::AnimalCtx {
                defs: host.defs,
                clock: &clock,
                sandbox: &sandbox,
            };
            if let (Some(ff), Some(mf)) = (female.animal.as_mut(), self.animal.as_mut()) {
                failure = ff.animal.fertilize(
                    pz_animals::Male::Other(&mut mf.animal),
                    false,
                    actx,
                    &mut CtxRand(ctx),
                ).err();
            }
        }
        cx.world.put_character(female);
        if let Some(error) = failure { cx.record_animal_error(error); }
    }

    fn eat_anim(&mut self, cx: &mut StateCx<'_, I>) {
        self.set_var_str("idleAction", "eat");
        let n = self.adef().eating_type_nbr;
        if n > 0 {
            let k = cx.ctx.rand_next_range(1, n + 1);
            self.set_var_str("eatingAnim", &format!("eat{k}"));
        }
    }

    fn trough_square(
        &self,
        t: pz_animals::ObjectRef,
        cx: &mut StateCx<'_, I>,
    ) -> Option<(i32, i32, i32)> {
        cx.world
            .animal_host(self.id)
            .and_then(|h| h.world.trough_square(t))
            .and_then(square_ref_xyz)
    }

    fn world_item_square(
        &self,
        o: pz_animals::ObjectRef,
        cx: &mut StateCx<'_, I>,
    ) -> Option<(i32, i32, i32)> {
        cx.world
            .animal_host(self.id)
            .and_then(|h| h.world.world_item_square(o))
            .and_then(square_ref_xyz)
    }

    /// `drinkFromTrough()`.
    fn behavior_drink_from_trough(&mut self, cx: &mut StateCx<'_, I>) {
        let Some(BehaviorObject::Trough(t)) = self.beh().behavior_object else {
            return;
        };
        let tsq = self.trough_square(t, cx);
        if let (Some(own), Some(tsq)) = (self.own_square(), tsq) {
            if sq_dist_proper(own, tsq) > self.adef().dist_to_eat {
                self.af_mut().animal.ignored_trough.push(t);
                return;
            }
        }
        self.eat_anim(cx);
        self.af_mut().animal.drink_from_trough = Some(t);
    }

    /// `eatFromTrough()`.
    fn behavior_eat_from_trough(&mut self, cx: &mut StateCx<'_, I>) {
        let Some(BehaviorObject::Trough(t)) = self.beh().behavior_object else {
            return;
        };
        let tsq = self.trough_square(t, cx);
        if let (Some(own), Some(tsq)) = (self.own_square(), tsq) {
            if sq_dist_proper(own, tsq) > self.adef().dist_to_eat {
                self.af_mut().animal.ignored_trough.push(t);
                return;
            }
        }
        self.eat_anim(cx);
        self.af_mut().animal.eat_from_trough = Some(t);
    }

    /// `eatFromGround()` and `drinkFromGround()` (the same body).
    fn behavior_eat_from_ground(&mut self, cx: &mut StateCx<'_, I>) {
        let Some(BehaviorObject::WorldItem(o)) = self.beh().behavior_object else {
            return;
        };
        let osq = self.world_item_square(o, cx);
        if let (Some(own), Some(osq)) = (self.own_square(), osq) {
            if sq_dist_proper(own, osq) > self.adef().dist_to_eat {
                return;
            }
        }
        self.eat_anim(cx);
        self.af_mut().animal.eat_from_ground = Some(o);
    }

    /// `drinkFromRiver()` (`river`) and `drinkFromPuddle()`.
    fn behavior_drink_square(&mut self, cx: &mut StateCx<'_, I>, river: bool) {
        let Some(BehaviorObject::Square(r)) = self.beh().behavior_object else {
            return;
        };
        if let (Some(own), Some(s)) = (self.own_square(), square_ref_xyz(r)) {
            if sq_dist_proper(own, s) > self.adef().dist_to_eat {
                return;
            }
        }
        self.eat_anim(cx);
        let a = &mut self.af_mut().animal;
        if river {
            a.drink_from_river = Some(r);
        } else {
            a.drink_from_puddle = Some(r);
        }
    }

    /// `eatGrass()`.
    fn behavior_eat_grass(&mut self, cx: &mut StateCx<'_, I>) {
        self.animal_stop_all_movement_now(cx);
        if cx.has_host_error() { return; }
        self.eat_anim(cx);
        let a = &mut self.af_mut().animal;
        a.behavior.wander_mul_mod = 0.0;
        a.data.eating_grass = true;
    }

    /// `eatFromMom()`.
    fn behavior_eat_from_mom(&mut self) {
        // The mother is reached through the world in the caller's context;
        // eatFromMom only reads it.
        self.set_var_str("idleAction", "eat");
        self.set_var_str("eatingAnim", "feed");
    }

    /// `clearIdleAction()`.
    fn clear_idle_action(&mut self) {
        let v = self.variable_string("idleAction");
        if !v.is_empty() && v.starts_with("idle") {
            self.clear_var("idleAction");
        }
    }

    /// `checkBehavior()`.
    pub fn behavior_check_behavior(&mut self, cx: &mut StateCx<'_, I>) {
        if self.beh().is_doing_behavior {
            return;
        }
        if !self.beh().block_movement {
            self.af_mut().fighting_opponent = None;
        }
        self.clear_idle_action();
        let in_vehicle = self.path.vehicle.is_some();
        if in_vehicle {
            self.set_doing_behavior(false);
            self.af_mut().lured_by = None;
            self.beh_mut().block_movement = false;
            self.clear_var("idleAction");
        }
        if !in_vehicle && !self.af().exists_in_world
            || self.af().lured_by.is_some()
            || self.beh().block_movement
            || self.beh().is_doing_behavior
            || !self.variable_string("idleAction").is_empty()
        {
            return;
        }
        let go = |c: &mut Self| {
            if c.path.vehicle.is_none() {
                c.set_doing_behavior(true);
                c.vars.turn_delta_normal = 0.9;
            }
        };
        let selected = self.behavior_check_drink_behavior(cx);
        if cx.has_host_error() { return; }
        if selected {
            go(self);
            return;
        }
        let selected = self.behavior_check_eat_behavior(cx);
        if cx.has_host_error() { return; }
        if selected {
            go(self);
            return;
        }
        let selected = self.behavior_check_fertilize_female(cx);
        if cx.has_host_error() { return; }
        if selected {
            go(self);
            return;
        }
        let selected = self.behavior_call_to_hutch(cx);
        if cx.has_host_error() { return; }
        if selected {
            go(self);
            return;
        }
        let selected = self.behavior_check_attack_behavior(cx);
        if cx.has_host_error() { return; }
        if selected && self.path.vehicle.is_none() {
            self.set_doing_behavior(true);
            self.vars.turn_delta_normal = 0.9;
        }
    }

    /// `callToHutch(null, false)`.
    fn behavior_call_to_hutch(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        match self.animal_env(cx, |a, c, env| a.behavior_call_to_hutch(None, false, c, env)) {
            Some(Ok(called)) => called,
            Some(Err(error)) => { cx.record_animal_error(error); false }
            // The existing absent-world profile remains an explicit host gate.
            None => false,
        }
    }

    /// `checkAttackBehavior()`.
    fn behavior_check_attack_behavior(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        if pz_animals::AnimalCharacter::is_female(self) {
            return false;
        }
        let baby = match self.af().animal.is_baby() {
            Ok(baby) => baby,
            Err(error) => { cx.record_animal_error(error); return false; }
        };
        if baby { return false; }
        let adef = self.adef();
        if adef.dont_attack_other_male {
            return false;
        }
        if !self.animal_can_do_action(cx) || self.has_hit_reaction() {
            return false;
        }
        if self.beh().attack_animal_timer > 0.0 {
            return false;
        }
        let season = pz_animals::Animal::in_mating_season(
            &adef,
            cx.ctx.animal_sandbox().animal_mating_season,
            cx.ctx.month(),
        );
        if !season || self.af().animal.age() < self.af().animal.min_age_for_baby_of(&adef) {
            return false;
        }
        let my_type = self.af().animal.animal_type.clone().unwrap_or_default();
        for z in self.af().animal.connected_d_zone.clone() {
            let animals = cx
                .world
                .animal_host(self.id)
                .map(|h| h.world.zone_animals_connected(z))
                .unwrap_or_default();
            for r in animals {
                let Some(aid) = character_ref_id(r) else {
                    continue;
                };
                if aid == self.id {
                    continue;
                }
                let ok = cx.world.character_mut(aid).is_some_and(|c| {
                    let female = pz_animals::AnimalCharacter::is_female(c);
                    let eating = c.animal_is_eating();
                    let Some(f) = c.animal.as_ref() else {
                        return false;
                    };
                    let Some(d) = f.def.as_ref() else {
                        return false;
                    };
                    !female
                        && f.fighting_opponent.is_none()
                        && f.animal
                            .animal_type
                            .as_deref()
                            .is_some_and(|t| t.eq_ignore_ascii_case(&my_type))
                        && f.animal.age() >= f.animal.min_age_for_baby_of(&d.adef)
                        && !f.animal.behavior.is_doing_behavior
                        && !eating
                });
                if !ok {
                    continue;
                }
                self.behavior_go_attack(aid, cx);
                if cx.has_host_error() { return false; }
                self.af_mut().fighting_opponent = Some(aid);
                return true;
            }
        }
        false
    }

    /// `checkFertilizeFemale()`.
    fn behavior_check_fertilize_female(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        self.animal_env(cx, |a, c, env| {
            a.data_find_female_to_inseminate(None, c, env)
        });
        let list = self.af().animal.data.animal_to_inseminate.clone();
        if list.is_empty() {
            return false;
        }
        let i = cx.ctx.rand_next_range(0, list.len() as i32) as usize;
        let female = list[i];
        let Some(fid) = character_ref_id(female) else {
            return false;
        };
        self.with_other(fid, cx, |f, cx| {
            if let Some(ff) = f.animal.as_mut() {
                ff.animal.behavior.block_movement = true;
            }
            f.animal_stop_all_movement_now(cx);
        });
        if cx.has_host_error() { return false; }
        self.animal_stop_all_movement_now(cx);
        if cx.has_host_error() { return false; }
        let b = self.beh_mut();
        b.behavior_action = Some(BehaviorAction::Fertilize);
        b.behavior_object = Some(BehaviorObject::Animal(female));
        let l = &mut self.af_mut().animal.data.animal_to_inseminate;
        if let Some(p) = l.iter().position(|x| *x == female) {
            l.remove(p);
        }
        self.animal_path_to_character(fid, cx);
        true
    }

    /// `getRandomRiverSq()`.
    fn random_river_sq(&mut self, cx: &mut StateCx<'_, I>) -> Option<pz_animals::ObjectRef> {
        let z = self.af().animal.d_zone?;
        let mut all = cx
            .world
            .animal_host(self.id)
            .map(|h| h.world.zone_near_water_squares_connected(z))
            .unwrap_or_default();
        if all.is_empty() {
            return None;
        }
        shuffle(cx.ctx.new_random_seed(), &mut all);
        all.first().copied()
    }

    /// `getNearestWaterSquare(sq)`.
    fn nearest_water_square(
        &mut self,
        sq: (i32, i32, i32),
        cx: &mut StateCx<'_, I>,
    ) -> Option<(i32, i32, i32)> {
        for x in sq.0 - 1..sq.0 + 2 {
            for y in sq.1 - 1..sq.1 + 2 {
                if cx.world.square(x, y, sq.2).is_none() {
                    continue;
                }
                let water = cx
                    .world
                    .animal_host(self.id)
                    .is_some_and(|h| h.world.square_has_water_object(square_ref(x, y, sq.2)));
                if water {
                    return Some((x, y, sq.2));
                }
            }
        }
        None
    }

    /// The square the animal paths to for an action, or does the action
    /// at once when within one square (`tryDrink*`, `tryEat*`).
    fn go_or_do(
        &mut self,
        target: Option<(i32, i32, i32)>,
        cx: &mut StateCx<'_, I>,
        vehicle_check: bool,
    ) {
        let far = match (target, self.own_square()) {
            (Some(t), Some(own)) => sq_dist_proper(t, own) > 1.0,
            _ => false,
        };
        if far {
            let t = target.expect("far");
            self.animal_stop_all_movement_now(cx);
            if cx.has_host_error() { return; }
            self.animal_path_to_location(t.0, t.1, t.2, cx);
        } else if !vehicle_check || self.path.vehicle.is_none() {
            self.set_doing_behavior(true);
            self.behavior_do_behavior_action(cx);
            if cx.has_host_error() { return; }
        }
    }

    /// `tryDrinkFromRiver()`.
    fn try_drink_from_river(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        if self.af().animal.d_zone.is_none() {
            return false;
        }
        let Some(near) = self.random_river_sq(cx) else {
            return false;
        };
        self.beh_mut().behavior_action = Some(BehaviorAction::DrinkFromRiver);
        let near_xyz = cx
            .world
            .animal_host(self.id)
            .map(|h| h.world.square_xyz(near))
            .unwrap_or_default();
        let Some(river) = self.nearest_water_square(near_xyz, cx) else {
            return false;
        };
        self.beh_mut().behavior_object = Some(BehaviorObject::Square(square_ref(
            river.0, river.1, river.2,
        )));
        let close = self
            .own_square()
            .is_some_and(|own| sq_dist_proper(near_xyz, own) <= 1.0);
        if close {
            if self.path.vehicle.is_none() {
                self.set_doing_behavior(true);
                self.behavior_do_behavior_action(cx);
                if cx.has_host_error() { return false; }
            }
        } else {
            self.animal_stop_all_movement_now(cx);
            if cx.has_host_error() { return false; }
            self.animal_path_to_location(near_xyz.0, near_xyz.1, near_xyz.2, cx);
        }
        true
    }

    /// `checkDrinkBehavior()`.
    fn behavior_check_drink_behavior(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        let baby = match self.af().animal.is_baby() {
            Ok(baby) => baby,
            Err(error) => { cx.record_animal_error(error); return false; }
        };
        if baby { return false; }
        let (_, t) = self.hunger_thirst();
        if t < self.adef().thirst_hunger_trigger
            || self.af().fighting_opponent.is_some()
        {
            return false;
        }
        let selected = self.try_drink_from_river(cx);
        if cx.has_host_error() { return false; }
        if selected {
            return true;
        }
        let selected = self.try_drink_from_puddle(cx);
        if cx.has_host_error() { return false; }
        if selected {
            return true;
        }
        for z in self.af().animal.connected_d_zone.clone() {
            if self.try_drink_from_ground(z, cx) {
                return true;
            }
        }
        self.try_drink_from_trough(cx)
    }

    /// The connected zones contain the zone at `sq`.
    fn connected_contains_zone_at(&mut self, sq: (i32, i32, i32), cx: &mut StateCx<'_, I>) -> bool {
        let z = cx
            .world
            .animal_host(self.id)
            .and_then(|h| h.world.zone_at(sq.0 as f32, sq.1 as f32, sq.2 as f32));
        z.is_some_and(|z| self.af().animal.connected_d_zone.contains(&z))
    }

    /// `tryAndGetPuddle(searchRadius)`: the puddle's square.
    pub fn behavior_try_and_get_puddle(
        &mut self,
        mut radius: i32,
        cx: &mut StateCx<'_, I>,
    ) -> Option<(i32, i32, i32)> {
        if radius <= 0 {
            radius = self.af().animal.search_radius;
        }
        let own = self.own_square()?;
        let has = |c: &mut Self, cx: &mut StateCx<'_, I>, s: (i32, i32, i32)| {
            cx.world
                .animal_host(c.id)
                .is_some_and(|h| h.world.square_has_puddle_floor(square_ref(s.0, s.1, s.2)))
        };
        if has(self, cx, own) {
            return Some(own);
        }
        for x in own.0 - radius..=own.0 + radius {
            for y in own.1 - radius..=own.1 + radius {
                let s = (x, y, own.2);
                if cx.world.square(x, y, own.2).is_none()
                    || !self.connected_contains_zone_at(s, cx)
                    || !has(self, cx, s)
                {
                    continue;
                }
                return Some(s);
            }
        }
        None
    }

    /// `tryAndGetGrassFloor()`: the grass floor's square.
    pub fn behavior_try_and_get_grass_floor(
        &mut self,
        cx: &mut StateCx<'_, I>,
    ) -> Option<(i32, i32, i32)> {
        let own = self.own_square()?;
        let has = |c: &mut Self, cx: &mut StateCx<'_, I>, s: (i32, i32, i32)| {
            cx.world
                .animal_host(c.id)
                .is_some_and(|h| h.world.square_has_grass_floor(square_ref(s.0, s.1, s.2)))
        };
        if has(self, cx, own) {
            return Some(own);
        }
        let r = self.af().animal.search_radius;
        for x in own.0 - r..=own.0 + r {
            for y in own.1 - r..=own.1 + r {
                let s = (x, y, own.2);
                if cx.world.square(x, y, own.2).is_none()
                    || !self.connected_contains_zone_at(s, cx)
                    || !has(self, cx, s)
                {
                    continue;
                }
                return Some(s);
            }
        }
        None
    }

    /// `tryDrinkFromPuddle()`.
    fn try_drink_from_puddle(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        if self.path.vehicle.is_some() || self.own_square().is_none() {
            return false;
        }
        if let Some(p) = self.behavior_try_and_get_puddle(0, cx) {
            let b = self.beh_mut();
            b.behavior_action = Some(BehaviorAction::DrinkFromPuddle);
            b.behavior_object = Some(BehaviorObject::Square(square_ref(p.0, p.1, p.2)));
            self.go_or_do(Some(p), cx, true);
            return true;
        }
        if self.vars.state_event_delay_timer > 50.0 {
            self.vars.state_event_delay_timer = 50.0;
        }
        false
    }

    /// `tryDrinkFromGround(zone)`.
    fn try_drink_from_ground(
        &mut self,
        zone: pz_animals::ObjectRef,
        cx: &mut StateCx<'_, I>,
    ) -> bool {
        let food = cx
            .world
            .animal_host(self.id)
            .map(|h| h.world.zone_food_on_ground(zone))
            .unwrap_or_default();
        for o in food {
            let pure = cx
                .world
                .animal_host(self.id)
                .is_some_and(|h| h.world.world_item_is_pure_water(o));
            if !pure {
                continue;
            }
            let b = self.beh_mut();
            b.behavior_action = Some(BehaviorAction::DrinkGround);
            b.behavior_object = Some(BehaviorObject::WorldItem(o));
            let sq = self.world_item_square(o, cx);
            self.go_or_do(sq, cx, true);
            return true;
        }
        false
    }

    /// `getRandomTroughList()`.
    fn random_trough_list(&mut self, cx: &mut StateCx<'_, I>) -> Vec<pz_animals::ObjectRef> {
        let mut result = Vec::new();
        for z in self.af().animal.connected_d_zone.clone() {
            result.extend(
                cx.world
                    .animal_host(self.id)
                    .map(|h| h.world.zone_troughs(z))
                    .unwrap_or_default(),
            );
        }
        shuffle(cx.ctx.new_random_seed(), &mut result);
        result
    }

    /// `tryDrinkFromTrough()`.
    fn try_drink_from_trough(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        for t in self.random_trough_list(cx) {
            let water = cx
                .world
                .animal_host(self.id)
                .map_or(0.0, |h| h.world.trough_water(t));
            if self.af().animal.ignored_trough.contains(&t) || water <= 0.0 {
                continue;
            }
            let b = self.beh_mut();
            b.behavior_action = Some(BehaviorAction::Drink);
            b.behavior_object = Some(BehaviorObject::Trough(t));
            self.go_or_do_trough(t, cx, true);
            return true;
        }
        false
    }

    fn go_or_do_trough(
        &mut self,
        t: pz_animals::ObjectRef,
        cx: &mut StateCx<'_, I>,
        vehicle_check: bool,
    ) {
        let tsq = self.trough_square(t, cx);
        let far = match (tsq, self.own_square()) {
            (Some(s), Some(own)) => sq_dist_proper(s, own) > 1.0,
            _ => false,
        };
        if far {
            self.animal_stop_all_movement_now(cx);
            if cx.has_host_error() { return; }
            self.animal_path_to_trough(t, cx);
        } else if !vehicle_check || self.path.vehicle.is_none() {
            self.set_doing_behavior(true);
            self.behavior_do_behavior_action(cx);
            if cx.has_host_error() { return; }
        }
    }

    /// `checkEatBehavior()`.
    pub fn behavior_check_eat_behavior(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        let (h, _) = self.hunger_thirst();
        if h < self.adef().thirst_hunger_trigger || self.af().fighting_opponent.is_some() {
            return false;
        }
        if self.path.vehicle.is_some() {
            return self.behavior_eat_from_vehicle(cx);
        }
        let baby = match self.af().animal.is_baby() {
            Ok(baby) => baby,
            Err(error) => { cx.record_animal_error(error); return false; }
        };
        if baby {
            let selected = self.try_to_eat_from_mom(true, cx);
            if cx.has_host_error() { return false; }
            if selected { return true; }
        }
        for z in self.af().animal.connected_d_zone.clone() {
            if self.try_eat_from_ground(z, cx) {
                return true;
            }
        }
        if self.try_eat_from_trough(cx) {
            return true;
        }
        self.try_eat_grass(cx)
    }

    /// `forceEatFromMom()`.
    pub fn behavior_force_eat_from_mom(&mut self, cx: &mut StateCx<'_, I>) {
        self.animal_stop_all_movement_now(cx);
        if cx.has_host_error() { return; }
        self.try_to_eat_from_mom(false, cx);
        if cx.has_host_error() { return; }
        self.beh_mut().behavior_check_timer = 500.0;
    }

    /// `tryToEatFromMom(callOtherBabies)`.
    fn try_to_eat_from_mom(&mut self, call_others: bool, cx: &mut StateCx<'_, I>) -> bool {
        let Some(mid) = self.mother_id() else {
            return false;
        };
        let ok = cx.world.character_mut(mid).is_some_and(|m| {
            let idle = m.current_state_id() == Some(ids::ANIMAL_IDLE);
            m.animal.as_ref().is_some_and(|f| {
                f.initialized
                    && !f.animal.behavior.is_doing_behavior
                    && f.exists_in_world
                    && f.animal.have_enough_milk_to_feed_from()
                    && idle
            })
        });
        if !ok {
            return false;
        }
        self.animal_stop_all_movement_now(cx);
        if cx.has_host_error() { return false; }
        let msq = self.with_other(mid, cx, |m, cx| {
            if let Some(f) = m.animal.as_mut() {
                f.animal.behavior.block_movement = true;
            }
            m.animal_stop_all_movement_now(cx);
            m.path.current_square
        });
        if cx.has_host_error() { return false; }
        if let Some(Some(s)) = msq {
            self.animal_path_to_location(s.0, s.1, s.2, cx);
        }
        let b = self.beh_mut();
        b.behavior_object = Some(BehaviorObject::Animal(character_ref(mid)));
        b.behavior_action = Some(BehaviorAction::EatMom);
        self.set_doing_behavior(true);
        if call_others {
            let babies = cx
                .world
                .character_mut(mid)
                .and_then(|m| m.animal.as_ref())
                .and_then(|f| f.animal.babies.clone())
                .unwrap_or_default();
            let me = character_ref(self.id);
            for b in babies {
                if b == me {
                    continue;
                }
                if let Some(bid) = character_ref_id(b) {
                    self.with_other(bid, cx, |c, cx| c.behavior_force_eat_from_mom(cx));
                    if cx.has_host_error() { return false; }
                }
            }
        }
        true
    }

    /// `eatFromMom()`'s mother part: unblocked, faced.
    pub(crate) fn behavior_eat_from_mom_mother(&mut self, cx: &mut StateCx<'_, I>) {
        let Some(mid) = self.mother_id() else {
            return;
        };
        let pos = cx.world.character_mut(mid).map(|m| {
            if let Some(f) = m.animal.as_mut() {
                f.animal.behavior.block_movement = false;
            }
            (m.vars.x, m.vars.y)
        });
        if let Some((x, y)) = pos {
            self.face_character(x, y);
        }
    }

    /// `tryEatFromGround(zone)`.
    fn try_eat_from_ground(
        &mut self,
        zone: pz_animals::ObjectRef,
        cx: &mut StateCx<'_, I>,
    ) -> bool {
        let Some(types) = self.adef().eat_type_trough.clone() else {
            return false;
        };
        let food = cx
            .world
            .animal_host(self.id)
            .map(|h| h.world.zone_food_on_ground(zone))
            .unwrap_or_default();
        for o in food {
            let item = cx.world.animal_host(self.id).and_then(|h| {
                h.world
                    .world_item(o)
                    .and_then(|(i, _)| h.world.feed_item(i))
            });
            if !item.is_some_and(|i| can_eat_this(&types, &i)) {
                continue;
            }
            let b = self.beh_mut();
            b.behavior_action = Some(BehaviorAction::EatGround);
            b.behavior_object = Some(BehaviorObject::WorldItem(o));
            let sq = self.world_item_square(o, cx);
            self.go_or_do(sq, cx, false);
            return true;
        }
        false
    }

    /// `canEatFromTrough(trough)` (the behaviour's: no rotten food).
    fn behavior_can_eat_from_trough(
        &mut self,
        t: pz_animals::ObjectRef,
        cx: &mut StateCx<'_, I>,
    ) -> bool {
        let Some(types) = self.adef().eat_type_trough.clone() else {
            return false;
        };
        let Some(h) = cx.world.animal_host(self.id) else {
            return false;
        };
        if !h.world.trough_has_container(t) {
            return false;
        }
        for i in h.world.trough_items(t) {
            let Some(item) = h.world.feed_item(i) else {
                continue;
            };
            if let pz_animals::env::FeedItemKind::Food { rotten: true, .. } = item.kind {
                continue;
            }
            if can_eat_this(&types, &item) {
                return true;
            }
        }
        false
    }

    /// `tryEatFromTrough()`.
    fn try_eat_from_trough(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        for t in self.random_trough_list(cx) {
            if self.af().animal.ignored_trough.contains(&t)
                || !self.behavior_can_eat_from_trough(t, cx)
            {
                continue;
            }
            let b = self.beh_mut();
            b.behavior_action = Some(BehaviorAction::EatTrough);
            b.behavior_object = Some(BehaviorObject::Trough(t));
            self.go_or_do_trough(t, cx, false);
            return true;
        }
        false
    }

    /// `tryEatGrass()`.
    fn try_eat_grass(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        if self.adef().eat_grass && self.own_square().is_some() {
            if let Some(f) = self.behavior_try_and_get_grass_floor(cx) {
                let b = self.beh_mut();
                b.behavior_action = Some(BehaviorAction::EatGrass);
                b.behavior_object = Some(BehaviorObject::Square(square_ref(f.0, f.1, f.2)));
                self.go_or_do(Some(f), cx, true);
                return true;
            }
        }
        if self.vars.state_event_delay_timer <= 50.0 {
            self.vars.state_event_delay_timer = 50.0;
        }
        false
    }

    /// `eatFromVehicle()`.
    pub fn behavior_eat_from_vehicle(&mut self, cx: &mut StateCx<'_, I>) -> bool {
        let baby = match self.af().animal.is_baby() {
            Ok(baby) => baby,
            Err(error) => { cx.record_animal_error(error); return false; }
        };
        let adef = self.adef();
        if baby && adef.need_mom {
            if let Some(mid) = self.mother_id() {
                let enough = cx
                    .world
                    .character_mut(mid)
                    .and_then(|m| m.animal.as_ref())
                    .is_some_and(|f| f.animal.have_enough_milk_to_feed_from());
                if enough {
                    let (hs, ts) = (self.af().hunger.clone(), self.af().thirst.clone());
                    let h = self.stats.get(&hs);
                    self.stats.set(&hs, h - 0.2);
                    let t = self.stats.get(&ts);
                    self.stats.set(&ts, t - 0.2);
                    let hb = adef.hunger_boost;
                    let r = cx.ctx.rand_next_float(0.2 / hb, 0.5 / hb);
                    let ctx = cx.ctx;
                    if let Some(mut mom) = cx.world.take_character(mid) {
                        if let Some(host) = cx.world.animal_host(self.id) {
                            let clock = super::CtxClock(ctx);
                            let sandbox = ctx.animal_sandbox();
                            let actx = pz_animals::AnimalCtx {
                                defs: host.defs,
                                clock: &clock,
                                sandbox: &sandbox,
                            };
                            if let Some(f) = mom.animal.as_mut() {
                                let q = f.animal.data.milk_qty - r;
                                f.animal.data_set_milk_quantity(q, actx, &mut CtxRand(ctx));
                            }
                        }
                        cx.world.put_character(mom);
                    }
                    return true;
                }
            }
        }
        let Some(v) = self.path.vehicle else {
            return false;
        };
        let items = cx.world.animal_host(self.id).and_then(|h| {
            h.world
                .vehicle_food_items(v as u16 as pz_animals::ObjectRef)
        });
        let Some(item) = items.and_then(|l| l.first().copied()) else {
            return false;
        };
        if let Some(Err(error)) = self.animal_env(cx, |a, c, env| a.data_eat_item(item, false, c, env)) {
            cx.record_animal_error(error);
            return false;
        }
        true
    }

    /// `updateAcceptance()`.
    fn behavior_update_acceptance(&mut self, cx: &mut StateCx<'_, I>) {
        let Some((id, _)) = self.af().animal.held_by else {
            return;
        };
        let m = cx.ctx.multiplier();
        let a = &mut self.af_mut().animal;
        let mut r = CtxRand(cx.ctx);
        let mut level = a.acceptance_level(id, &mut r);
        level += cx.ctx.rand_next_float(0.002, 0.005) * ((100.0 - a.stress_level) / 100.0) * m;
        level = level.min(100.0);
        a.set_debug_acceptance(id, level);
    }

    /// `followChr()`.
    fn behavior_follow_chr(&mut self, cx: &mut StateCx<'_, I>) {
        let f = self.af();
        if f.animal.behavior.block_movement
            || f.animal.eat_from_trough.is_some()
            || f.animal.eat_from_ground.is_some()
        {
            return;
        }
        let Some(p) = f.animal.data.attached_player.and_then(character_ref_id) else {
            self.set_var_float("animalSpeed", 1.0);
            self.beh_mut().follow_chr_timer = 0.0;
            return;
        };
        if self.beh().follow_chr_timer > 0.0 {
            let m = cx.ctx.multiplier();
            self.beh_mut().follow_chr_timer -= m;
            return;
        }
        self.beh_mut().follow_chr_timer = 150.0;
        let Some((px, py, pz, moving)) = cx
            .world
            .character_mut(p)
            .map(|c| (c.vars.x, c.vars.y, c.zi(), c.vars.is_player_moving))
        else {
            return;
        };
        let dist = distance_to(self.vars.x, self.vars.y, px, py);
        if moving && dist > 2.0 {
            self.walk_to_chr(px, py, pz, cx);
        }
        if dist < 4.0 && !self.vars.moving {
            return;
        }
        if dist >= 4.0 && !moving {
            self.walk_to_chr(px, py, pz, cx);
        }
        if dist >= 15.0 {
            let me = self.id;
            if let Some(c) = cx.world.character_mut(p) {
                c.player.attached_animals.retain(|a| *a != me);
            }
            self.af_mut().animal.data.attached_player = None;
        }
    }

    /// `walkToChr(chr, pushedLength)` (the length is not used).
    fn walk_to_chr(&mut self, px: f32, py: f32, pz: i32, cx: &mut StateCx<'_, I>) {
        self.set_var_float("animalSpeed", 1.2);
        let (vx, vy) = (px - self.vars.x, py - self.vars.y);
        let (vx, vy) = set_length(vx, vy, length(vx, vy) + -1.0);
        let (x, y) = (fastfloor(self.vars.x + vx), fastfloor(self.vars.y + vy));
        self.animal_path_to_location(x, y, pz, cx);
        self.set_var_bool("animalRunning", length(vx, vy) > 5.0);
    }

    /// `wildAnimalFLeeFromAttacker()`.
    fn wild_animal_flee_from_attacker(&mut self, cx: &mut StateCx<'_, I>) {
        if !self.beh().wild_and_hurt {
            return;
        }
        let m = cx.ctx.multiplier();
        self.beh_mut().wild_drop_dead_timer -= m;
        if let Some(sq) = self.own_square() {
            if self.environment.square_has_chunk && next_bool(cx, 5) {
                let splat = |c: &mut Self, cx: &mut StateCx<'_, I>| {
                    let x = sq.0 as f32 + cx.ctx.rand_next_float(-0.8, 0.8);
                    let y = sq.1 as f32 + cx.ctx.rand_next_float(-0.8, 0.8);
                    let k = cx.ctx.rand_next(20);
                    if let Some(h) = cx.world.animal_host(c.id) {
                        h.world
                            .add_blood_splat(square_ref(sq.0, sq.1, sq.2), x, y, sq.2 as f32, k);
                    }
                };
                splat(self, cx);
                if next_bool(cx, 2) {
                    splat(self, cx);
                }
                if next_bool(cx, 2) {
                    splat(self, cx);
                }
            }
        }
        if self.beh().wild_drop_dead_timer <= 0.0 {
            self.animal_set_health(0.0);
        }
    }

    /// Paths `go` away from where it is, as the flee code does: once when
    /// not facing that way, once more when still not moving.
    fn flee_path(&mut self, gx: f32, gy: f32, go: (i32, i32), cx: &mut StateCx<'_, I>) {
        let (x, y, z) = (self.xi() + go.0, self.yi() + go.1, self.zi());
        if self.vars.dir != dir_from_angle(gx, gy) {
            self.animal_path_to_location(x, y, z, cx);
        }
        if !self.animal_is_moving() {
            self.animal_path_to_location(x, y, z, cx);
        }
    }

    /// `fleeFromAttacker()`.
    fn behavior_flee_from_attacker(&mut self, cx: &mut StateCx<'_, I>) {
        let Some(att) = self.vars.attacked_by else {
            self.remove_attacked();
            return;
        };
        let now = cx.ctx.calendar_time_in_millis();
        let opp_dist = match self.af().fighting_opponent.and_then(|o| self.other(o, cx)) {
            Some(o) => distance_manhatten(self.vars.x, self.vars.y, o.x, o.y),
            None => 0.0,
        };
        if now - self.af().animal.attacked_timer > 1_100_000 || opp_dist > 40.0 {
            self.remove_attacked();
            return;
        }
        if let Some(p) = self
            .af()
            .animal
            .data
            .attached_player
            .and_then(character_ref_id)
        {
            let me = self.id;
            if let Some(c) = cx.world.character_mut(p) {
                c.player.attached_animals.retain(|a| *a != me);
            }
            self.af_mut().animal.data.attached_player = None;
        }
        self.af_mut().animal.data_set_attached_tree(None, None);
        let Some(a) = self.other(att, cx) else {
            return;
        };
        if self.adef().attack_back && a.iso_player {
            let hidden = cx
                .world
                .character_mut(att)
                .is_some_and(|c| c.is_ghost_mode(cx.ctx));
            self.af_mut().fighting_opponent = if hidden { None } else { Some(att) };
            return;
        }
        let (vx, vy) = (self.vars.x - a.x, self.vars.y - a.y);
        let len = length(vx, vy);
        let (gx, gy) = set_length(vx, vy, 20.0f32.max(20.0 - len));
        let go = (fastfloor(gx), fastfloor(gy));
        self.set_var_bool("animalRunning", true);
        if !self.animal_is_moving() && !self.animal_is_attacking(cx) {
            self.behavior_reset_behavior_action();
            self.flee_path(gx, gy, go, cx);
        }
        if self.af().animal.wild {
            self.wild_animal_flee_from_attacker(cx);
        }
    }

    /// `removeAttacked()`.
    fn remove_attacked(&mut self) {
        self.vars.attacked_by = None;
        self.af_mut().animal.attacked_timer = -1;
    }

    /// `fleeFromChr()`.
    pub fn behavior_flee_from_chr(&mut self, cx: &mut StateCx<'_, I>) {
        let f = self.af();
        let Some(s) = f.spotted_chr else {
            return;
        };
        if Some(s) == f.atk_target
            || f.animal.data.attached_player == Some(character_ref(s))
            || f.fighting_opponent == Some(s)
        {
            return;
        }
        let m = cx.ctx.multiplier();
        {
            let b = self.beh_mut();
            if b.timer_flee_again > 0.0 {
                b.timer_flee_again -= m;
                if b.timer_flee_again < 0.0 {
                    b.timer_flee_again = 0.0;
                }
            }
        }
        let Some(o) = self.other(s, cx) else {
            return;
        };
        let craven = self.af().animal.has_genetic_disorder("craven");
        let wild = self.af().animal.wild;
        let adef = self.adef();
        if !craven && !wild && o.iso_player && !adef.always_flee_humans {
            let (pm, running) = cx.world.character_mut(s).map_or((false, false), |c| {
                (c.vars.is_player_moving, c.is_running(cx.sm))
            });
            if !pm {
                return;
            }
            let mut base = 0i32;
            base += (self.af().animal.stress_level / 2.2) as i32;
            let accept = self
                .af_mut()
                .animal
                .acceptance_level(o.online_id, &mut CtxRand(cx.ctx));
            base -= (accept / 2.0) as i32;
            if accept < 70.0 {
                let dist = distance_manhatten(self.vars.x, self.vars.y, o.x, o.y);
                base += (dist * 5.0) as i32;
            }
            if running {
                base *= 4;
            }
            base = base.max(0);
            if base == 0 {
                return;
            }
            if accept >= 40.0 && self.af().animal.stress_level < 70.0 {
                return;
            }
            if cx.ctx.rand_next(10000) > base {
                return;
            }
        }
        if !craven && self.af().lured_by.is_none() && !wild && o.zombie && !adef.flee_zombies {
            return;
        }
        let (vx, vy) = (self.vars.x - o.x, self.vars.y - o.y);
        let len = length(vx, vy);
        let (mut gx, mut gy) = set_length(vx, vy, 10.0f32.max(10.0 - len));
        if wild {
            let len = length(gx, gy);
            (gx, gy) = set_length(gx, gy, 30.0f32.max(30.0 - len));
        }
        let go = (fastfloor(gx), fastfloor(gy));
        self.set_var_bool("animalRunning", go.1.abs() > 5 || go.0.abs() > 5);
        if wild {
            self.set_var_bool("animalRunning", true);
        }
        if !wild && o.iso_player {
            let acc = self
                .af_mut()
                .animal
                .acceptance_level(o.online_id, &mut CtxRand(cx.ctx));
            if acc > 50.0 {
                self.set_var_bool("animalRunning", false);
            }
        }
        if self.af().animal.stress_level > 50.0 {
            self.set_var_bool("animalRunning", true);
        }
        if !self.animal_is_moving()
            && self.beh().timer_flee_again == 0.0
            && !self.animal_is_attacking(cx)
        {
            self.behavior_reset_behavior_action();
            self.beh_mut().timer_flee_again = 200.0;
            // playStressedSound(): the voice slot's sound (FMOD).
            self.flee_path(gx, gy, go, cx);
        }
    }

    /// `forceFleeFromChr(chr)` (the character at `pos`).
    pub fn behavior_force_flee_from_chr(
        &mut self,
        _chr: i32,
        pos: (f32, f32),
        cx: &mut StateCx<'_, I>,
    ) {
        let (vx, vy) = (self.vars.x - pos.0, self.vars.y - pos.1);
        let len = length(vx, vy);
        let (gx, gy) = set_length(vx, vy, 10.0f32.max(10.0 - len));
        let go = (fastfloor(gx), fastfloor(gy));
        self.set_var_bool("animalRunning", true);
        self.behavior_reset_behavior_action();
        self.beh_mut().timer_flee_again = 200.0;
        // playStressedSound().
        self.flee_path(gx, gy, go, cx);
    }

    /// `spotted(other, false, dist)`.
    pub fn behavior_spotted(&mut self, other: i32, dist: f32, cx: &mut StateCx<'_, I>) {
        self.af_mut().spotted_chr = None;
        let m = cx.ctx.multiplier();
        {
            let b = self.beh_mut();
            if b.last_alerted > 0.0 {
                b.last_alerted -= m;
            }
            if b.last_alerted < 0.0 {
                b.last_alerted = 0.0;
            }
        }
        if cx.ctx.is_client() {
            return;
        }
        let Some(own) = self.own_square() else {
            return;
        };
        let Some(o) = self.other(other, cx) else {
            return;
        };
        let Some(osq) = o.square else {
            return;
        };
        if cx.world.square_smoke(own) {
            return;
        }
        let ghost = cx
            .world
            .character_mut(other)
            .is_some_and(|c| c.is_ghost_mode(cx.ctx));
        if o.iso_player && ghost {
            return;
        }
        if o.dead {
            return;
        }
        let wild = self.af().animal.wild;
        let adef = self.adef();
        if o.iso_player && !wild && dist < 10.0 {
            let running = cx
                .world
                .character_mut(other)
                .is_some_and(|c| c.is_running(cx.sm));
            let a = &mut self.af_mut().animal;
            let mut level = a.acceptance_level(o.online_id, &mut CtxRand(cx.ctx));
            level +=
                cx.ctx.rand_next_float(5.0e-4, 6.0e-4) * ((100.0 - a.stress_level) / 100.0) * m;
            level = level.min(100.0);
            a.set_debug_acceptance(o.online_id, level);
            if running {
                a.change_stress(m / 1000.0);
            }
            let stress = self.af().animal.stress_level;
            if !self.animal_is_moving()
                && self.af().fighting_opponent.is_none()
                && adef.attack_if_stressed
                && stress > 80.0
                && level < 30.0
                && !self.beh().is_doing_behavior
                && self.af().atk_target.is_none()
                && self.beh().attack_animal_timer == 0.0
                && next_bool(cx, 300 - stress as i32)
            {
                self.af_mut().fighting_opponent = Some(other);
                self.set_var_bool("animalRunning", true);
                self.behavior_go_attack(other, cx);
                return;
            }
        }
        if o.zombie && !adef.flee_zombies {
            return;
        }
        if wild && dist < 3.0 {
            self.af_mut().spotted_chr = Some(other);
            self.behavior_flee_from_chr(cx);
            return;
        }
        let mut spotted = false;
        if wild && o.iso_player {
            let info = cx.world.character_mut(other).map(|c| {
                let perk = |n: &str| c.perk_level(cx.chars.perks.constant(n).as_ref());
                (
                    c.vars.is_player_moving,
                    c.is_sneaking(),
                    c.is_running(cx.sm),
                    perk("Tracking"),
                    perk("Sneak"),
                    perk("Lightfoot"),
                    perk("Nimble"),
                )
            });
            if let Some((moving, sneaking, running, tracking, sneak, lightfoot, nimble)) = info {
                if moving {
                    let mut base = 8000.0f32;
                    if sneaking {
                        base = 800.0;
                    }
                    if running {
                        base = 500000.0;
                    }
                    let sd = adef.spotting_dist as f32;
                    if dist <= sd {
                        base *= 1.0 + (sd - dist);
                    }
                    base /= (tracking + 2) as f32 * 0.5;
                    base /= (sneak + 2) as f32 * 0.3;
                    base /= (lightfoot + 2) as f32 * 0.25;
                    base /= (nimble + 2) as f32 * 0.25;
                    let (tx, ty) = (o.x - self.vars.x, o.y - self.vars.y);
                    if osq.2 != own.2 {
                        let dif = (osq.2 - own.2).abs() * 5 + 1;
                        base /= dif as f32;
                    }
                    let (tx, ty) = normalize(tx, ty);
                    let (lx, ly) = self.look_vector();
                    let cos = lx * tx + ly * ty;
                    if cos < -0.4 {
                        base /= 16.0;
                    } else if cos < -0.2 {
                        base /= 3.0;
                    } else if cos < 0.2 {
                        // The original has a "< -0.0" and a "< 0.2" branch,
                        // both dividing by 1.5.
                        base /= 1.5;
                    } else if cos <= 0.4 {
                        base *= 3.0;
                    } else if cos <= 0.6 {
                        base *= 11.0;
                    } else if cos <= 0.8 {
                        base *= 24.0;
                    } else {
                        base *= 44.0;
                    }
                    let adj = crate::character::body_part::adjust_for_framerate(cx.ctx, 25000);
                    let r = cx.ctx.rand_next(adj);
                    if r > base as i32 {
                        self.spotted_tracking_xp(other, sneaking, dist, &adef, cx);
                        return;
                    }
                    spotted = true;
                }
            }
        }
        if o.zombie && dist <= 10.0 {
            spotted = true;
            let r = cx.ctx.rand_next_float(5.0e-5, 9.0e-5);
            let a = &mut self.af_mut().animal;
            a.set_debug_stress(a.stress_level + r * m / (14.0 - dist));
            if a.stress_level > 100.0 {
                a.set_debug_stress(100.0);
            }
            if dist <= 6.0 {
                self.af_mut().spotted_chr = Some(other);
                self.behavior_flee_from_chr(cx);
                return;
            }
        }
        // ensureOnTile() for a missing square: both have one here.
        if dist < adef.spotting_dist as f32 && spotted {
            if adef.can_be_alerted && self.beh().last_alerted <= 0.0 {
                let f = self.af_mut();
                f.animal.alerted = true;
                f.alerted_chr = Some(other);
            } else {
                self.af_mut().spotted_chr = Some(other);
                self.behavior_flee_from_chr(cx);
            }
        }
    }

    /// The tracking, sneak, nimble and lightfoot XP a sneaking player gets
    /// for not being spotted by a wild animal (`spotted`).
    fn spotted_tracking_xp(
        &mut self,
        other: i32,
        sneaking: bool,
        dist: f32,
        adef: &pz_animals::AnimalDefinitions,
        cx: &mut StateCx<'_, I>,
    ) {
        let outside = cx
            .world
            .character_mut(other)
            .and_then(|c| c.path.current_square)
            .is_some_and(|s| cx.world.square_is_outside(s));
        if !(sneaking && outside && dist <= adef.spotting_dist as f32 && adef.add_tracking_xp) {
            return;
        }
        if cx.ctx.is_client() {
            return;
        }
        let moving = cx
            .world
            .character_mut(other)
            .is_some_and(|c| c.vars.is_player_moving);
        let n200 = crate::character::body_part::adjust_for_framerate(cx.ctx, 200);
        let n385 = crate::character::body_part::adjust_for_framerate(cx.ctx, 385);
        let mut xp: Vec<(&str, f32)> = Vec::new();
        if moving && next_bool(cx, n200) || !moving && next_bool(cx, n200) {
            xp.push(("Tracking", cx.ctx.rand_next_float(1.0, 3.0)));
        }
        if sneaking {
            for p in ["Sneak", "Nimble", "Lightfoot"] {
                if moving && next_bool(cx, n385) {
                    xp.push((p, cx.ctx.rand_next_float(1.0, 3.0)));
                }
            }
        }
        // GameServer.addXp takes (int) of the amount.
        let server = cx.ctx.is_server();
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        if let Some(c) = cx.world.character_mut(other) {
            for (p, a) in xp {
                let a = if server { (a as i32) as f32 } else { a };
                let _ = c.add_xp(sm, chars, chars.perks.constant(p).as_ref(), a, ctx);
            }
        }
    }

    /// `AnimalEatState`'s `idleActionEnd`: drink, drink from the ground, or
    /// eat, with the behaviour calls `AnimalData.eat()` makes.
    pub fn behavior_after_idle_action_end(&mut self, cx: &mut StateCx<'_, I>) {
        let f = &self.af().animal;
        if f.drink_from_trough.is_some()
            || f.drink_from_river.is_some()
            || f.drink_from_puddle.is_some()
        {
            if let Some(Err(error)) = self.animal_env(cx, |a, c, env| a.data_drink(c, env)) {
                cx.record_animal_error(error);
            }
            return;
        }
        if let Some(o) = f.eat_from_ground {
            let pure = cx
                .world
                .animal_host(self.id)
                .is_some_and(|h| h.world.world_item_is_pure_water(o));
            if pure {
                self.animal_env(cx, |a, c, env| a.data_drink_from_ground(c, env));
                return;
            }
        }
        let follow = match self.animal_env(cx, |a, c, env| a.data_eat(c, env)) {
            Some(Ok(follow)) => follow,
            Some(Err(error)) => { cx.record_animal_error(error); return; }
            None => return,
        };
        match follow {
            EatFollowUp::None => {}
            EatFollowUp::CheckBehavior => {
                self.set_doing_behavior(false);
                self.behavior_check_behavior(cx);
                if cx.has_host_error() { return; }
            }
            EatFollowUp::ResetBehavior => {
                self.set_doing_behavior(false);
                self.behavior_reset_behavior_action();
            }
            EatFollowUp::Grass { check } => {
                if check {
                    self.set_doing_behavior(false);
                    self.behavior_check_behavior(cx);
                    if cx.has_host_error() { return; }
                }
                self.with_animal(|a, c| a.data_reset_eating_check(c));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffle_is_java_util_random() {
        // new Random(42): nextInt(), then the swaps of a 3-list.
        let mut v = [1, 2, 3];
        shuffle(42, &mut v);
        let r = JavaRandom::new(42);
        r.next_int_unbounded();
        let mut w = [1, 2, 3];
        for i in 0..3 {
            let c = i + r.next_int((3 - i) as i32) as usize;
            w.swap(i, c);
        }
        assert_eq!(v, w);
    }

    #[test]
    fn attack_decision_checks_female_before_propagating_missing_stage() {
        use pz_animals::AnimalCharacter;
        use crate::character::{state::{StateCx, NullWorld}, tests::{fixture, Recorder}};
        let (sm, mut chars) = fixture();
        let mut c = GameCharacter::<i32>::new_animal(&sm, &mut chars).unwrap();
        let ctx = Recorder::default(); let mut world = NullWorld;
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
        AnimalCharacter::set_female(&mut c, true);
        assert!(!c.behavior_check_attack_behavior(&mut cx));
        assert!(cx.state_result().is_ok(), "female short circuit must not call isBaby");
        AnimalCharacter::set_female(&mut c, false);
        assert!(!c.behavior_check_attack_behavior(&mut cx));
        assert_eq!(cx.state_result().unwrap_err().animal().unwrap().0, "NullPointerException: currentStage");
    }


    // A real state-machine exit records the existing animal error domain.
    // It witnesses native throw-like prefix stopping without inventing a
    // Throwable implementation or changing the production diagnostic.
    struct StopFault;
    impl crate::character::state::State<i32> for StopFault {
        fn id(&self) -> crate::character::state::StateId {
            crate::character::state::StateId("authored.AnimalStopFault")
        }
        fn exit(&self, _: &mut GameCharacter<i32>, cx: &mut StateCx<'_, i32>) {
            cx.record_animal_error(pz_animals::JavaException("authored animal stop state exit".into()));
        }
    }

    struct SelectionWorld {
        defs: pz_animals::AnimalDefinitionTables,
        other: GameCharacter<i32>,
        host_queries: usize,
        path_resets: usize,
        other_world: ConnectedAnimal,
    }
    struct ConnectedAnimal(i32);
    impl pz_animals::AnimalWorld for ConnectedAnimal {
        fn zone_animals_connected(&self, _: pz_animals::ObjectRef) -> Vec<pz_animals::ObjectRef> {
            vec![character_ref(self.0)]
        }
    }
    impl SelectionWorld {
        fn new(other: GameCharacter<i32>) -> Self {
            let id = other.id;
            Self { defs: Default::default(), other, host_queries: 0, path_resets: 0, other_world: ConnectedAnimal(id) }
        }
    }
    impl crate::character::state::StateWorld<i32> for SelectionWorld {
        fn animal_host(&mut self, _: i32) -> Option<crate::character::animal::AnimalHost<'_>> {
            self.host_queries += 1;
            // Definitions and world callbacks borrow disjoint actual owners.
            // The small callback object only supplies the connected animal.
            Some(crate::character::animal::AnimalHost { defs: &self.defs, world: &mut self.other_world })
        }
        fn character_mut(&mut self, id: i32) -> Option<&mut GameCharacter<i32>> {
            (self.other.id == id).then_some(&mut self.other)
        }
        fn path_reset(&mut self, _: &mut dyn pz_pathfind::mover::PathMover) {
            self.path_resets += 1;
        }
    }

    fn selection_animal(
        sm: &pz_scripts::ScriptManager,
        chars: &mut crate::Characters,
        id: i32,
        baby: bool,
    ) -> GameCharacter<i32> {
        let mut c = GameCharacter::<i32>::new_animal(sm, chars).unwrap();
        c.id = id;
        c.path.current_square = Some((id, 0, 0));
        let f = c.animal_fields_mut().unwrap();
        f.def = Some(crate::character::animal::AnimalDefValues::of(&pz_animals::AnimalDefinitions::default()));
        f.exists_in_world = true;
        f.animal.animal_type = Some("fixture".into());
        f.animal.data.current_stage = Some(pz_animals::AnimalGrowStage {
            next_stage: baby.then(|| "adult".into()),
            ..Default::default()
        });
        pz_animals::AnimalCharacter::set_female(&mut c, false);
        c
    }

    #[test]
    fn failed_mother_stop_does_not_fall_through_to_food_selection() {
        use crate::character::{state::{NullWorld, StateWorld}, tests::{fixture, Recorder}};
        let (sm, mut chars) = fixture();
        let mut c = selection_animal(&sm, &mut chars, 1, true);
        c.animal_fields_mut().unwrap().animal.mother = Some(character_ref(2));
        c.animal_fields_mut().unwrap().animal.connected_d_zone = vec![4];
        let hunger = chars.sim.character_stats.field("HUNGER").unwrap();
        c.stats.set(&hunger, 0.9);
        c.vars.state_event_delay_timer = 23.0;
        let mut mother = selection_animal(&sm, &mut chars, 2, false);
        mother.animal_fields_mut().unwrap().initialized = true;
        mother.animal_fields_mut().unwrap().animal.data.milk_qty = 1.0;
        let ctx = Recorder::default();
        let mut setup = NullWorld;
        let mut setup_cx = StateCx::new(&sm, &chars, &ctx, &mut setup);
        c.change_state(Some(&StopFault), &mut setup_cx);
        mother.change_state(Some(crate::character::states::animal::AnimalIdleState::instance::<i32>()), &mut setup_cx);
        assert!(setup_cx.state_result().is_ok());
        let mut world = SelectionWorld::new(mother);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
        assert!(!c.behavior_check_eat_behavior(&mut cx));
        assert_eq!(cx.state_result().unwrap_err().animal().unwrap().0, "authored animal stop state exit");
        assert_eq!(world.path_resets, 1, "completed stop prefix");
        assert_eq!(world.host_queries, 0, "failed mother selection must not query ground/trough food");
        assert_eq!(c.vars.state_event_delay_timer, 23.0, "failed mother selection must not run grass fallback");
        assert!(c.animal_fields().unwrap().animal.behavior.behavior_action.is_none());
        assert!(!world.character_mut(2).unwrap().animal_fields().unwrap().animal.behavior.block_movement);
    }

    #[test]
    fn failed_attack_stop_keeps_action_prefix_without_publishing_opponents() {
        use crate::character::{state::{NullWorld, StateWorld}, tests::{fixture, Recorder}};
        let (sm, mut chars) = fixture();
        let mut c = selection_animal(&sm, &mut chars, 1, false);
        c.animal_fields_mut().unwrap().animal.connected_d_zone = vec![4];
        let opponent = selection_animal(&sm, &mut chars, 2, false);
        let ctx = Recorder::default();
        let mut setup = NullWorld;
        let mut setup_cx = StateCx::new(&sm, &chars, &ctx, &mut setup);
        c.change_state(Some(&StopFault), &mut setup_cx);
        assert!(setup_cx.state_result().is_ok());
        let mut world = SelectionWorld::new(opponent);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
        assert!(!c.behavior_check_attack_behavior(&mut cx));
        assert_eq!(cx.state_result().unwrap_err().animal().unwrap().0, "authored animal stop state exit");
        assert_eq!(world.path_resets, 1, "goAttack reached the actual stop prefix");
        assert_eq!(world.host_queries, 1);
        assert_eq!(c.animal_fields().unwrap().animal.behavior.behavior_action, Some(BehaviorAction::FightAnimal));
        assert!(c.animal_fields().unwrap().fighting_opponent.is_none(), "caller must not publish fightingOpponent after failed goAttack");
        let opponent = world.character_mut(2).unwrap().animal_fields().unwrap();
        assert!(opponent.fighting_opponent.is_none());
        assert!(!opponent.animal.behavior.block_movement);
    }
}
