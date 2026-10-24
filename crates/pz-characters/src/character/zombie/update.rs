//! `IsoZombie.update` (`updateInternal`) and `postupdate`, with the parts
//! they call: the eat-body target, the corpse search, `tryThump`,
//! `Wander`, the attack hand-off to `BodyDamage.AddRandomDamageFromZombie`
//! and the crawler/walker action group switch.
//!
//! `super.update()` (`IsoGameCharacter.update`: movement, collisions,
//! falling, the animation) is the movement code's; this update runs the
//! state machine in its place (`stateMachine.update()`).

use pz_scripts::ScriptResult;

use super::math::*;
use super::GameCharacter;
use crate::character::body_damage::{ZombieAttack, ZombieAttackOutcome};
use crate::character::state::{ActionGroups, EatTarget, StateCx, StateRegistry, ThumpKind, ThumpTarget};
use crate::character::states::ids;
use crate::character::SimEvent;

/// A meal's position and square.
type EatPos = (f32, f32, f32, Option<(i32, i32, i32)>);

impl<I: 'static> GameCharacter<I> {
    /// `setEatBodyTarget(target, force)` (draws the eat speed first).
    pub fn set_eat_body_target(&mut self, target: Option<EatTarget>, force: bool, cx: &mut StateCx<'_, I>) {
        let speed = cx.ctx.rand_next_float(0.64, 0.96);
        self.set_eat_body_target_speed(target, force, speed, cx);
    }

    /// `setEatBodyTarget(target, force, eatSpeed)`.
    pub fn set_eat_body_target_speed(&mut self, target: Option<EatTarget>, force: bool, speed: f32, cx: &mut StateCx<'_, I>) {
        if target == self.zombie.eat_body_target {
            return;
        }
        if !force {
            if let Some(t) = target {
                if self.eating_count(t, cx) >= 3 {
                    return;
                }
            }
        }
        if let Some(old) = self.zombie.eat_body_target {
            self.set_eating(old, false, cx);
        }
        self.zombie.eat_body_target = target;
        let Some(t) = target else { return };
        self.set_eating(t, true, cx);
        self.zombie.eat_speed = speed;
    }

    fn eating_count(&self, t: EatTarget, cx: &mut StateCx<'_, I>) -> usize {
        match t {
            EatTarget::Character(c) => cx.world.character_mut(c).map_or(0, |c| c.seen.eating_zombies.len()),
            b => cx.world.eating_count(b),
        }
    }

    fn set_eating(&self, t: EatTarget, on: bool, cx: &mut StateCx<'_, I>) {
        let me = self.id;
        match t {
            EatTarget::Character(c) => {
                if let Some(c) = cx.world.character_mut(c) {
                    if on {
                        c.seen.eating_zombies.push(me);
                    } else if let Some(i) = c.seen.eating_zombies.iter().position(|z| *z == me) {
                        c.seen.eating_zombies.remove(i);
                    }
                }
            }
            b => cx.world.set_eating(b, me, on),
        }
    }

    fn eat_target_pos(&self, t: EatTarget, cx: &mut StateCx<'_, I>) -> Option<EatPos> {
        match t {
            EatTarget::Character(c) => cx.world.character_mut(c).map(|c| (c.vars.x, c.vars.y, c.vars.z, c.path.current_square)),
            EatTarget::Body(b) => cx.world.dead_body(b).map(|b| (b.x, b.y, b.z, Some((fastfloor(b.x), fastfloor(b.y), fastfloor(b.z))))),
        }
    }

    /// `updateEatBodyTarget()` (the `beatbodytarget` slot runs it).
    pub fn update_eat_body_target(&mut self, cx: &mut StateCx<'_, I>) {
        if let Some(b) = self.zombie.body_to_eat {
            if self.is_current_state_id(ids::ZOMBIE_IDLE) {
                if let Some(body) = cx.world.dead_body(b) {
                    if self.dist_to_squared_xy(body.x, body.y) <= 1.0 && fastfloor(self.vars.z) == fastfloor(body.z) {
                        self.set_eat_body_target(Some(EatTarget::Body(b)), false, cx);
                        self.zombie.body_to_eat = None;
                    }
                }
            }
        }
        let Some(t) = self.zombie.eat_body_target else { return };
        if let EatTarget::Body(b) = t {
            if !cx.world.dead_body(b).is_some_and(|b| b.in_world) {
                self.set_eat_body_target(None, false, cx);
            }
        }
        if let Some(tg) = self.vars.target {
            let on_floor = cx.world.character_mut(tg).is_some_and(|c| c.vars.on_floor);
            if !on_floor && Some(EatTarget::Character(tg)) != self.zombie.eat_body_target {
                self.set_eat_body_target(None, false, cx);
            }
        }
        if let Some(EatTarget::Character(c)) = self.zombie.eat_body_target {
            let info = cx.world.character_mut(c).map(|p| {
                (
                    p.is_player(),
                    p.seen.reanimated_corpse.is_some(),
                    !p.is_dead(),
                    p.vars.on_floor,
                    p.is_current_state_id(ids::PLAYER_HIT_REACTION),
                )
            });
            if let Some((player, corpse, alive, floor, hit)) = info {
                if player && corpse {
                    self.set_eat_body_target(None, false, cx);
                }
                if player && alive && !floor && !hit {
                    self.set_eat_body_target(None, false, cx);
                }
            }
        }
        if let Some(t) = self.zombie.eat_body_target {
            if !self.is_current_state_id(ids::ZOMBIE_EAT_BODY) {
                if let Some((x, y, _, _)) = self.eat_target_pos(t, cx) {
                    if self.dist_to_squared_xy(x, y) > 1.0 {
                        self.set_eat_body_target(None, false, cx);
                    }
                }
            }
        }
        if let Some(t) = self.zombie.eat_body_target {
            if let (Some((_, _, _, Some(sq))), Some(me)) = (self.eat_target_pos(t, cx), self.current_square_xyz()) {
                if cx.world.is_something_to(me, sq) {
                    self.set_eat_body_target(None, false, cx);
                }
            }
        }
    }

    /// `updateSearchForCorpse()`.
    pub fn update_search_for_corpse(&mut self, cx: &mut StateCx<'_, I>) {
        if self.vars.crawling || self.vars.target.is_some() || self.zombie.eat_body_target.is_some() {
            self.zombie.check_for_corpse_timer = 10000.0;
            self.zombie.body_to_eat = None;
            return;
        }
        if let Some(b) = self.zombie.body_to_eat {
            match cx.world.dead_body(b) {
                Some(body) if body.in_world => {
                    let eating_other = false;
                    if !eating_other && body.eating >= 3 {
                        self.zombie.body_to_eat = None;
                    }
                }
                _ => self.zombie.body_to_eat = None,
            }
        }
        if self.zombie.body_to_eat.is_none() {
            self.zombie.check_for_corpse_timer -= cx.ctx.thirty_fps_multiplier();
            if self.zombie.check_for_corpse_timer <= 0.0 {
                self.zombie.check_for_corpse_timer = 10000.0;
                let mut bodies = Vec::new();
                for x in -10..10 {
                    for y in -10..10 {
                        let sq = (fastfloor(self.vars.x + x as f32), fastfloor(self.vars.y + y as f32), fastfloor(self.vars.z));
                        let Some(c) = cx.world.dead_body_at(sq) else { continue };
                        if c.skeleton || c.zombie || c.eating >= 3 || c.animal || c.animal_skeleton {
                            continue;
                        }
                        if cx.world.line_clear_collide(self.vars.x, self.vars.y, c.x, c.y, fastfloor(self.vars.z), None, 2) {
                            continue;
                        }
                        bodies.push(c.id);
                    }
                }
                if !bodies.is_empty() {
                    let i = cx.ctx.rand_next(bodies.len() as i32) as usize;
                    self.zombie.body_to_eat = Some(bodies[i]);
                }
            }
        }
        let Some(b) = self.zombie.body_to_eat else { return };
        if !self.is_current_state_id(ids::ZOMBIE_IDLE) {
            return;
        }
        let Some(body) = cx.world.dead_body(b) else { return };
        if self.dist_to_squared_xy(body.x, body.y) > 1.0 {
            let (vx, vy) = set_length(self.vars.x - body.x, self.vars.y - body.y, 0.5);
            self.zombie_path_to_location_f(body.x + vx, body.y + vy, body.z, cx);
        }
    }

    /// `tryThump(square)`: climb through a window, hop a low fence or door,
    /// or pick the thump target in front.
    pub fn try_thump(&mut self, square: Option<(i32, i32, i32)>, cx: &mut StateCx<'_, I>) -> bool {
        if self.zombie.ghost || self.vars.crawling {
            return false;
        }
        let moving = [ids::PATH_FIND, ids::LUNGE, ids::LUNGE_NETWORK, ids::WALK_TOWARD, ids::WALK_TOWARD_NETWORK]
            .iter()
            .any(|s| self.is_current_state_id(*s));
        if !moving {
            return false;
        }
        let feeler = square.or_else(|| {
            let (vx, vy) = set_length(self.zombie.next_x - self.vars.last_x, self.zombie.next_y - self.vars.last_y, 0.5);
            let sq = (fastfloor(self.vars.x + vx), fastfloor(self.vars.y + vy), fastfloor(self.vars.z));
            cx.world.square(sq.0, sq.1, sq.2).map(|_| sq)
        });
        let (Some(feeler), Some(cur)) = (feeler, self.current_square_xyz()) else { return false };
        let Some(obj) = cx.world.special_object_between(cur, feeler) else { return false };
        let Some(i) = cx.world.thump_info(obj, self.id) else { return false };
        let ThumpTarget::Object(oref) = obj else { return false };
        use crate::character::path_mover::PathMoverRequest as R;
        if (i.kind == ThumpKind::Window || i.kind == ThumpKind::WindowFrame) && i.can_climb_through {
            if !self.is_facing_location(i.facing_position.0, i.facing_position.1, 0.8) {
                return false;
            }
            self.path.requests.push(if i.kind == ThumpKind::Window { R::ClimbThroughWindow(oref) } else { R::ClimbThroughWindowFrame(oref) });
            return true;
        }
        if (i.kind == ThumpKind::Door || i.kind == ThumpKind::Thumpable && !i.can_climb_through) && i.hoppable && !i.tall_hoppable {
            if !self.is_facing_location(i.facing_position.0, i.facing_position.1, 0.8) {
                return false;
            }
            let dir = cardinal_direction_to(cur, i.square.unwrap_or(cur), i.north);
            self.path.requests.push(R::ClimbOverFence(dir));
            return true;
        }
        if i.kind == ThumpKind::Thumpable && i.can_climb_through {
            self.path.requests.push(R::ClimbThroughWindow(oref));
            return true;
        }
        if i.thumpable_for.is_some() {
            let (dx, dy) = (feeler.0 - cur.0, feeler.1 - cur.1);
            let (ax, ay) = (dx.abs(), dy.abs());
            use pz_world::square::IsoDirections as D;
            let mut dir = D::N;
            if dx < 0 && ax > ay {
                dir = D::S;
            }
            if dx < 0 && ax <= ay {
                dir = D::SW;
            }
            if dx > 0 && ax > ay {
                dir = D::W;
            }
            if dx > 0 && ax <= ay {
                dir = D::SE;
            }
            if dy < 0 && ax < ay {
                dir = D::N;
            }
            if dy < 0 && ax >= ay {
                dir = D::NW;
            }
            if dy > 0 && ax < ay {
                dir = D::E;
            }
            if dy > 0 && ax >= ay {
                dir = D::NE;
            }
            if self.vars.dir == dir {
                let chasing_sound = self.path_goal_sound(cx)
                    && [ids::PATH_FIND, ids::WALK_TOWARD, ids::WALK_TOWARD_NETWORK].iter().any(|s| self.is_current_state_id(*s));
                if cx.ctx.lore_thump_no_chasing() || self.vars.target.is_some() || chasing_sound {
                    let mut t = obj;
                    if i.kind == ThumpKind::Window {
                        if let Some(tf) = i.thumpable_for {
                            t = tf;
                        }
                    }
                    self.zombie.thump_target = Some(t);
                    self.clear_path2();
                }
            }
            return true;
        }
        false
    }

    /// `Wander()`.
    pub fn wander(&mut self, cx: &mut StateCx<'_, I>) {
        self.change_state(Some(crate::character::states::zombie::idle()), cx);
    }

    /// The zombie as `AddRandomDamageFromZombie` sees it, attacking `target`.
    pub fn zombie_attack_for(&mut self, target: i32, hit_reaction: &str, cx: &mut StateCx<'_, I>) -> ZombieAttack {
        let (sm, chars) = (cx.sm, cx.chars);
        let bite = crate::character::body_damage::BiteVoice {
            voice_prefix: self.voice_prefix(),
            speed_type: self.zombie.speed_type,
            voice_choice: std::cell::Cell::new(self.zombie.voice_choice),
        };
        let me = (self.id, self.vars.target, self.vars.crawling, self.vars.x, self.vars.y, self.current_state_id());
        let (melee, dot, current_reaction, tsq, tx, ty) = match cx.world.character_mut(target) {
            Some(t) => {
                let level = t.weapon_level(sm, chars);
                (
                    melee_combat_mod(level),
                    t.test_dot_side(me.3, me.4).name().to_string(),
                    t.vars.hit_reaction.clone(),
                    t.path.current_square,
                    t.vars.x,
                    t.vars.y,
                )
            }
            None => (-5, String::new(), String::new(), None, 0.0, 0.0),
        };
        let surrounding = |cx: &mut StateCx<'_, I>, include_crawlers: bool| -> i32 {
            let Some(sq) = tsq else { return 0 };
            let mut count = 0;
            let mut check = |id: i32, target_of: Option<i32>, crawling: bool, x: f32, y: f32, st: Option<crate::character::state::StateId>| {
                if target_of != Some(target) {
                    return;
                }
                if distance_to_squared(x, y, tx, ty) >= 0.809_999_94 {
                    return;
                }
                if crawling && !include_crawlers {
                    return;
                }
                let attacking = [ids::ATTACK, ids::ATTACK_NETWORK, ids::LUNGE, ids::LUNGE_NETWORK].iter().any(|s| st == Some(*s));
                if attacking {
                    count += 1;
                }
                let _ = id;
            };
            let near = (fastfloor(me.3) - sq.0).abs() <= 1 && (fastfloor(me.4) - sq.1).abs() <= 1;
            if near {
                check(me.0, me.1, me.2, me.3, me.4, me.5);
            }
            for id in cx.world.characters_near(tx, ty, sq.2 as f32, 2.0) {
                let Some(z) = cx.world.character_mut(id) else { continue };
                if !z.is_zombie() {
                    continue;
                }
                let Some(zs) = z.path.current_square else { continue };
                if (zs.0 - sq.0).abs() > 1 || (zs.1 - sq.1).abs() > 1 || zs.2 != sq.2 {
                    continue;
                }
                check(id, z.vars.target, z.vars.crawling, z.vars.x, z.vars.y, z.current_state_id());
            }
            count
        };
        let s1 = surrounding(cx, false);
        let s2 = surrounding(cx, cx.ctx.lore_zombies_crawlers_drag_down());
        ZombieAttack {
            hit_reaction: Some(hit_reaction.to_string()),
            part_index: -1,
            melee_combat_mod: melee,
            dot_side: dot,
            surrounding_attacking_zombies: s1,
            drag_down_zombies: s2,
            current_hit_reaction: current_reaction,
            crawling: self.vars.crawling,
            inactive: self.zombie.inactive,
            cant_bite: self.zombie.no_teeth,
            bite_voice: bite,
        }
    }

    /// `targetChar.getBodyDamage().AddRandomDamageFromZombie(zombie, ...)`
    /// then `getBodyDamage().Update()` is the body damage owner's tick.
    pub fn body_damage_add_random_damage_from_zombie(
        &mut self,
        sm: &pz_scripts::ScriptManager,
        chars: &crate::Characters,
        ctx: &dyn crate::SimContext,
        attack: &ZombieAttack,
    ) -> ScriptResult<ZombieAttackOutcome> {
        let outcome = self.add_random_damage_from_zombie(sm, chars, ctx, attack)?;
        if let Some(r) = &outcome.hit_reaction {
            self.vars.hit_reaction = r.clone();
        }
        Ok(outcome)
    }

    /// The zombie's side of `AddRandomDamageFromZombie`'s outcome.
    pub fn apply_zombie_attack_outcome(&mut self, o: &ZombieAttackOutcome) {
        self.zombie.scratch = o.scratch;
        self.zombie.laceration = o.laceration;
        if let Some(s) = o.zombie_state {
            self.zombie.sound_state = s;
        }
        if o.zombie_killed {
            self.health = 0.0;
        }
    }

    /// `crawling` changed: `initializeStates()` and the action group
    /// (`zombie-crawler` or `zombie`), as `updateInternal` does.
    pub fn ensure_zombie_action_group(&mut self, registry: &mut StateRegistry<I>, groups: &mut ActionGroups) {
        let want = self.zombie_anim_set_name();
        let have = self.state.action_context.group().map(|g| g.name.clone());
        if have.as_deref() != Some(want) {
            self.initialize_zombie_states(registry);
            let g = groups.get(want);
            self.set_action_group(g);
        }
    }

    /// `IsoZombie.updateInternal()` (without the movement and network).
    pub fn update_zombie(&mut self, registry: &mut StateRegistry<I>, groups: &mut ActionGroups, cx: &mut StateCx<'_, I>) -> ScriptResult {
        let ctx = cx.ctx;
        if ctx.is_debug() {
            if let Some(o) = self.zombie.outfit_name.clone().filter(|o| o.contains("Debug")) {
                if o.contains("Immortal") {
                    self.zombie.immortal_tutorial_zombie = true;
                }
                if o.contains("Useless") {
                    self.zombie.useless = true;
                }
            }
        }
        self.make_inactive(ctx, ctx.is_zombie_inactivity_phase());
        if self.zombie.persistent_id != 0 && self.is_dead() {
            self.zombie.persistent_id = 0;
        }
        self.ensure_zombie_action_group(registry, groups);
        if self.zombie.thump_timer > 0 {
            self.zombie.thump_timer -= 1;
        }
        if self.zombie.spot_sound_delay > 0 {
            self.zombie.spot_sound_delay -= 1;
        }
        // updateCharacterTextureAnimTime.
        let moving = self.is_current_state_id(ids::WALK_TOWARD) || self.is_current_state_id(ids::PATH_FIND);
        self.zombie.character_texture_anim_duration = if moving { 0.67 } else { 2.0 };
        self.zombie.character_texture_anim_time += ctx.time_delta();
        if self.zombie.character_texture_anim_time > self.zombie.character_texture_anim_duration {
            self.zombie.character_texture_anim_time %= self.zombie.character_texture_anim_duration;
        }
        let mult = ctx.multiplier();
        self.zombie.bonus_spot_time = (self.zombie.bonus_spot_time - mult).clamp(0.0, f32::MAX);
        self.vars.time_since_seen_flesh = (self.vars.time_since_seen_flesh + mult).clamp(0.0, f32::MAX);
        self.path.collidable = true;
        if ctx.is_last_stand()
            && !self.is_current_state_id(ids::THUMP)
            && !self.is_current_state_id(ids::ATTACK)
            && self.vars.time_since_seen_flesh > 120.0
            && ctx.rand_next(36000) == 0
        {
            let mut lowest = None;
            let mut lowest_dst = 1_000_000.0f32;
            for id in cx.world.characters_near(self.vars.x, self.vars.y, self.vars.z, f32::MAX) {
                let Some(p) = cx.world.character_mut(id) else { continue };
                let d = distance_manhatten(p.vars.x, p.vars.y, self.vars.x, self.vars.y);
                if p.is_player() && d < lowest_dst && !p.is_dead() {
                    lowest_dst = d;
                    lowest = Some(id);
                }
            }
            if let Some(l) = lowest {
                self.zombie.allow_repath_delay = -1.0;
                self.zombie_path_to_character(l, cx);
            }
            return Ok(());
        }
        if self.zombie.bonus_spot_time > 0.0 {
            if let Some(s) = self.zombie.spotted_last {
                if cx.world.character_mut(s).is_some_and(|c| !c.is_dead()) {
                    self.spotted(s, true, cx)?;
                }
            }
        }
        // super.update(): the state machine.
        self.state_machine_update(cx);
        if [ids::CLIMB_THROUGH_WINDOW, ids::CLIMB_OVER_FENCE, ids::CRAWLING_ZOMBIE_TURN].iter().any(|s| self.is_current_state_id(*s)) {
            return Ok(());
        }
        let cur = self.current_state_id();
        if [ids::STAGGER_BACK, ids::BURNT_TO_DEATH, ids::FAKE_DEAD_ZOMBIE, ids::ZOMBIE_FALL_DOWN, ids::ZOMBIE_ON_GROUND, ids::ZOMBIE_HIT_REACTION, ids::ZOMBIE_GET_UP]
            .iter()
            .any(|s| cur == Some(*s))
        {
            return Ok(());
        }
        if cur == Some(ids::PATH_FIND) && self.path.progress == pz_pathfind::mover::PathFindProgress::NotYetFound {
            if !self.vars.crawling {
                // `def.animFrameIncrease` for the legacy idle animation.
                self.zombie.anim_frame_increase = (0.08 + ctx.rand_next(1000) as f32 / 8000.0) * 0.5;
            }
        } else if cur != Some(ids::ATTACK) && (self.zombie.next_x != self.vars.x || self.zombie.next_y != self.vars.y) {
            if self.zombie.walk_variant_use.is_none() || cur != Some(ids::LUNGE) && cur != Some(ids::LUNGE_NETWORK) {
                self.zombie.walk_variant_use = Some(self.zombie.walk_variant.clone());
            }
            if self.vars.crawling {
                self.zombie.walk_variant_use = Some("ZombieCrawl".into());
            }
            if ![ids::ZOMBIE_IDLE, ids::STAGGER_BACK, ids::THUMP, ids::FAKE_DEAD_ZOMBIE].iter().any(|s| cur == Some(*s)) {
                self.vars.shootable = true;
            }
        }
        self.vars.shootable = true;
        self.try_thump(None, cx);
        // damageSheetRope: the roll comes first.
        if ctx.rand_next(30) == 0 {
            if let Some(sq) = self.current_square_xyz() {
                if cx.world.has_sheet_rope(sq) {
                    let dmg = ctx.rand_next_range(5, 15);
                    cx.world.damage_sheet_rope(sq, dmg as f32);
                }
            }
        }
        self.zombie.allow_repath_delay = (self.zombie.allow_repath_delay - mult).clamp(0.0, f32::MAX);
        if self.vars.time_since_seen_flesh > self.zombie.memory as f32 && self.vars.target.is_some() {
            self.set_zombie_target(None);
        }
        if let Some(t) = self.vars.target {
            if cx.world.character_mut(t).is_some_and(|c| c.seen.reanimated_corpse.is_some()) {
                self.set_zombie_target(None);
            }
        }
        if let Some(t) = self.vars.target {
            if let Some((tx, ty, _)) = cx.world.character_mut(t).map(|c| (c.vars.x, c.vars.y, c.vars.z)) {
                self.zombie.vector_to_target = (tx - self.vars.x, ty - self.vars.y);
            }
            // updateZombieTripping.
            if self.zombie.speed_type == 1
                && self.vars.bump_type.is_empty()
                && crate::character::body_part::next_bool(ctx, crate::character::body_part::adjust_for_framerate(ctx, 750))
                && !self.zombie.remote
            {
                self.set_bump_type("trippingFromSprint");
            }
        }
        if let Some(sq) = self.current_square_xyz() {
            if cx.world.has_farming_plant(sq) {
                let chance = ((100.0 / ctx.true_multiplier()) as i32).max(1);
                if crate::character::body_part::next_bool(ctx, chance) {
                    cx.world.destroy_farming_plant(sq);
                }
            }
            if cx.world.has_lit_campfire(sq) && self.vars.moving {
                let chance = ((10000.0 / ctx.true_multiplier()) as i32).max(1);
                if crate::character::body_part::next_bool(ctx, chance) {
                    cx.world.put_out_campfire(sq);
                }
            }
        }
        if !self.is_current_state_id(ids::PATH_FIND) && !self.is_current_state_id(ids::WALK_TOWARD) && !self.is_current_state_id(ids::CLIMB_THROUGH_WINDOW) {
            self.zombie.last_heard_sound = (-1, -1, -1);
        }
        if self.vars.time_since_seen_flesh > 240.0 && self.zombie.time_since_respond_to_sound > 5.0 {
            self.respond_to_sound(cx);
        }
        self.zombie.time_since_respond_to_sound += ctx.thirty_fps_multiplier();
        self.update_search_for_corpse(cx);
        Ok(())
    }

    /// `IsoZombie.postupdate()`: the chasing count, the action context
    /// (with the zombie variable snapshot), balance and `canSeeTarget`.
    pub fn postupdate_zombie(&mut self, cx: &mut StateCx<'_, I>) {
        if let Some(t) = self.vars.target {
            if let Some(p) = cx.world.character_mut(t) {
                if p.is_player() {
                    p.stats.num_chasing_zombies += 1;
                }
            }
        }
        self.update_eat_body_target(cx);
        self.refresh_zombie_view(cx);
        self.update_action_context(cx);
        self.apply_zombie_var_effects();
        if self.zombie.unbalanced_level > 0.0 {
            let m = cx.ctx.multiplier();
            self.zombie.unbalanced_level = lerp_ease_out_quad(self.zombie.unbalanced_level, 0.0, 0.5 * m).clamp(0.0, 1.0);
        }
        self.zombie.can_see_target = self.is_target_visible(cx);
        if self.zombie.target_seen_time > 0.0 && !self.zombie.can_see_target {
            self.zombie.target_seen_time = 0.0;
        }
    }

    /// `IsoZombie.OnAnimEvent_IsAlmostUp`: also stops sitting.
    pub fn zombie_on_is_almost_up(&mut self) {
        self.vars.sit_against_wall = false;
    }

    /// `IsoZombie.Hit`'s extra outcome: the event the Lua side gets.
    pub fn zombie_hit_event(&self, cx: &StateCx<'_, I>) {
        cx.ctx.event(SimEvent::PlayerGetDamage { kind: "ZOMBIEHIT", amount: 0.0 });
    }
}

/// `getMeleeCombatMod()` for a weapon level.
pub fn melee_combat_mod(level: i32) -> i32 {
    match level {
        1 => -2,
        2 => 0,
        3 => 1,
        4 => 2,
        5 => 3,
        6 => 4,
        7 | 8 => 5,
        9 => 6,
        l if l >= 10 => 7,
        _ => -5,
    }
}

/// `getCardinalDirectionTo(square, north)` for a hop: the side of the
/// edge the character is on.
fn cardinal_direction_to(cur: (i32, i32, i32), sq: (i32, i32, i32), north: bool) -> pz_world::square::IsoDirections {
    use pz_world::square::IsoDirections as D;
    if north {
        if cur.1 < sq.1 {
            D::S
        } else {
            D::N
        }
    } else if cur.0 < sq.0 {
        D::E
    } else {
        D::W
    }
}
