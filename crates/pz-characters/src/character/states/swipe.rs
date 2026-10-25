//! `SwipeStatePlayer` with `CombatManager.calculateAttackVars` and the
//! melee part of `CombatManager.attackCollisionCheck` / `calculateHitInfoList`.
//!
//! Targets come from [`StateWorld::characters_near`]; their nearest point
//! is the character's position (the original also tries the head and
//! pelvis bones of the animation player, TODO(anim bones)), and line of
//! sight is [`StateWorld::line_clear`] (windows, barricades and transparent
//! walls between are TODO(world objects)). Ranged attacks are
//! TODO(ballistics); grappling is not ported.
//!
//! [`StateWorld::characters_near`]: crate::character::state::StateWorld::characters_near
//! [`StateWorld::line_clear`]: crate::character::state::StateWorld::line_clear

use pz_anim::{AnimEventKind, AnimEventRecord, AnimationVariableSource};
use pz_inventory::InventoryItem;
use pz_scripts::ScriptResult;

use super::ids;
use super::stagger_back::parse_boolean;
use crate::character::combat::{WeaponStats, WeaponType};
use crate::character::firearm::FirearmStats;
use crate::character::state::{Param, State, StateCx, StateId, StateRef, SyncFlags};
use crate::character::{GameCharacter, SimEvent};

pub const ID: StateId = ids::SWIPE_STATE_PLAYER;

/// `LOWER_CONDITION`, `ATTACKED`.
pub const LOWER_CONDITION: Param<bool> = Param::new(ID, "lower_condition", || false);
pub const ATTACKED: Param<bool> = Param::new(ID, "attacked", || true);

pub struct SwipeStatePlayer;

static INSTANCE: SwipeStatePlayer = SwipeStatePlayer;

/// `SwipeStatePlayer.instance()`.
pub fn instance<I: 'static>() -> StateRef<I> {
    &INSTANCE
}

/// `HitInfo` for a character target.
#[derive(Clone, Debug, PartialEq)]
pub struct HitInfo {
    pub target: i32,
    pub dot: f32,
    pub dist_sq: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `chance` (melee: 100).
    pub chance: i32,
}

/// `AttackVars` (melee part).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttackVars {
    /// The weapon's item id; `None` is the bare hands.
    pub weapon: Option<i32>,
    pub aim_at_floor: bool,
    pub close_kill: bool,
    pub do_shove: bool,
    pub use_charge_delta: f32,
    pub recoil_delay: i32,
    pub target_on_ground: Option<i32>,
    pub target_standing: Option<i32>,
    pub target_distance: f32,
    pub targets_standing: Vec<HitInfo>,
    pub targets_prone: Vec<HitInfo>,
}

/// The bare hands (`IsoLivingCharacter.bareHands`, `Base.BareHands`).
pub(crate) fn bare_hands_stats() -> WeaponStats {
    // Base.BareHands script values are data; the fields combat reads for a
    // shove only need identity and the flags below.
    WeaponStats {
        id: -1,
        item_type: "BareHands".into(),
        bare_hands: true,
        min_damage: 0.0,
        max_damage: 0.0,
        max_range: 0.8,
        min_angle: 0.65,
        max_angle: 1.0,
        max_hit_count: 1,
        use_endurance: true,
        categories: vec![],
        ..WeaponStats::default()
    }
}

impl<I: 'static> GameCharacter<I> {
    /// The attack weapon (`attackVars.getWeapon`): the primary hand
    /// weapon, else the bare hands.
    pub fn attack_weapon(&self, sm: &pz_scripts::ScriptManager) -> (WeaponStats, Option<InventoryItem>) {
        if let Some(id) = self.attack_vars.weapon {
            if let Some(item) = self.items.item(id) {
                if let Some(w) = WeaponStats::of(sm, item) {
                    return (w, Some(item.clone()));
                }
            }
        }
        (bare_hands_stats(), self.bare_hands.clone())
    }

    /// `getDotWithForwardDirection(x, y)`.
    pub fn dot_with_forward_direction(&self, x: f32, y: f32) -> f32 {
        let dx = x - self.vars.x;
        let dy = y - self.vars.y;
        let len = ((dx * dx + dy * dy) as f64).sqrt() as f32;
        let (vx, vy) = if len == 0.0 { (dx, dy) } else { (dx / len, dy / len) };
        let (fx, fy) = self.look_vector();
        vx * fx + vy * fy
    }

    /// `calcValidTarget` / `getNearestMeleeTargetPosAndDot` on the target's
    /// position.
    fn valid_melee_target(&self, t: &GameCharacter<I>, w: &WeaponStats, range: f32, ctx_line_clear: bool) -> Option<HitInfo> {
        if t.is_dead() || !t.vars.shootable || t.vars.hit_reaction.contains("Death") {
            return None;
        }
        let dx = t.vars.x - self.vars.x;
        let dy = t.vars.y - self.vars.y;
        if !w.ranged && dx * dx + dy * dy > 9.0 {
            return None;
        }
        let dz = (t.vars.z - self.vars.z).abs();
        if !w.ranged && dz >= 0.5 {
            return None;
        }
        let dot = self.dot_with_forward_direction(t.vars.x, t.vars.y).clamp(-1.0, 1.0);
        let oz = (self.vars.z.floor()) * 3.0;
        let tz = (t.vars.z.floor()) * 3.0;
        let dist_sq = dx * dx + dy * dy + (tz - oz) * (tz - oz);
        if dot < 0.0 || dist_sq > range * range || !ctx_line_clear {
            return None;
        }
        Some(HitInfo { target: t.id, dot, dist_sq, x: t.vars.x, y: t.vars.y, z: t.vars.z, chance: 100 })
    }

    /// `CombatManager.calculateAttackVars(owner)` (melee).
    pub fn calculate_attack_vars(&mut self, cx: &mut StateCx<'_, I>) {
        let sm = cx.sm;
        let primary = self.items.primary_hand_item().filter(|i| crate::character::items::is_hand_weapon(i)).map(|i| i.id);
        let attacking = self.is_performing_hostile_animation();
        let mut v = AttackVars {
            weapon: primary,
            do_shove: self.vars.do_shove,
            use_charge_delta: self.vars.use_charge_delta,
            ..AttackVars::default()
        };
        if v.weapon.is_none() || v.do_shove {
            v.do_shove = true;
            v.aim_at_floor = false;
            v.weapon = None;
        }
        self.attack_vars.weapon = v.weapon;
        let (w, w_item) = self.attack_weapon(sm);
        let firearm = w_item.as_ref().and_then(|i| FirearmStats::of(sm, i));
        // `calcValidTargets`: an aimed firearm picks its standing targets by
        // the muzzle ray (`useBallistics`).
        let use_ballistics = self.ballistics_controller.is_some() && w.aimed_firearm;
        let weapon_range = w.max_range * self.weapon_range_mod(sm, cx.chars, &w);
        let range = cx.ctx.ignore_prone_zombie_range().max(weapon_range + if use_ballistics { 0.0 } else { 1.0 });
        let threshold = crate::character::combat::config::BALLISTICS_CONTROLLER_DISTANCE_THRESHOLD;
        if use_ballistics {
            let muzzle = cx.world.muzzle_attachment(self.id);
            let mut st = cx.chars.combat.borrow_mut();
            self.calculate_ballistics(&mut st, muzzle.as_ref(), range);
        }
        let (start, end) = {
            let st = cx.chars.combat.borrow();
            (st.ballistics_start_position, st.ballistics_end_position)
        };
        let ballistic_hit = |h: &HitInfo| crate::character::firearm::ranged::is_point_within_distance(start, end, [h.x, h.y, h.z], threshold);
        let bare = bare_hands_stats();
        let mut too_close: Option<HitInfo> = None;
        let ids = cx.world.characters_near(self.vars.x, self.vars.y, self.vars.z, range);
        let from = self.square_xyz();
        for id in ids {
            if id == self.id {
                continue;
            }
            let to = {
                let Some(t) = cx.world.character_mut(id) else { continue };
                t.square_xyz()
            };
            let line = los_allows_melee(cx.world.los_line_clear(from, to, false));
            let Some(t) = cx.world.character_mut(id) else { continue };
            let Some(h) = self.valid_melee_target(t, &w, range, line) else { continue };
            {
                let mut st = cx.chars.combat.borrow_mut();
                st.temp_vector4f = [h.x, h.y, h.z, h.dot];
            }
            if t.is_standing() && use_ballistics {
                // `isMeleeTargetTooCloseToShoot` (with the bare hands' reach).
                let in_angle = !(h.dot < w.min_angle || h.dot > w.max_angle);
                let dx = h.x - self.vars.x;
                let dy = h.y - self.vars.y;
                let reach = bare.max_range;
                let in_reach = (t.vars.z - self.vars.z).abs() < 0.5 && dx * dx + dy * dy < reach * reach;
                if in_angle && in_reach && !ballistic_hit(&h) && too_close.as_ref().is_none_or(|c| h.dist_sq < c.dist_sq) {
                    too_close = Some(h.clone());
                }
                if !ballistic_hit(&h) {
                    continue;
                }
                v.targets_standing.push(h);
                continue;
            }
            if !t.is_standing() && w.ranged && self.ballistics_controller.is_some() {
                if !ballistic_hit(&h) {
                    continue;
                }
                v.targets_prone.push(h);
                continue;
            }
            let (min, max) = if t.is_standing() { (w.min_angle, w.max_angle) } else { (w.min_angle / 1.5, w.max_angle) };
            if h.dot < min || h.dot > max {
                continue;
            }
            if t.is_standing() {
                v.targets_standing.push(h);
            } else {
                v.targets_prone.push(h);
            }
        }
        if let Some(c) = too_close {
            v.targets_standing = vec![c];
        }
        let by_dist = |a: &HitInfo, b: &HitInfo| a.dist_sq.partial_cmp(&b.dist_sq).unwrap_or(std::cmp::Ordering::Equal);
        v.targets_standing.sort_by(by_dist);
        v.targets_prone.sort_by(by_dist);
        if !attacking {
            self.vars.aim_at_floor = false;
        }
        let mut lowest = f32::MAX;
        if let Some(best) = v.targets_standing.first() {
            v.target_standing = Some(best.target);
            v.target_distance = (best.dist_sq as f64).sqrt() as f32;
            lowest = best.dist_sq;
        } else if let Some(best) = v.targets_prone.first() {
            if cx.ctx.option_auto_prone_attack() || self.vars.do_shove {
                let d = (best.dist_sq as f64).sqrt() as f32;
                if !attacking {
                    self.vars.aim_at_floor = true;
                }
                v.aim_at_floor = !self.vars.do_shove || d < 0.6;
                v.target_on_ground = Some(best.target);
                v.target_distance = d;
            }
        }
        if lowest < w.min_range * w.min_range {
            if self.stats.num_chasing_zombies() <= 1 && self.weapon_type_of(sm) == WeaponType::Knife {
                v.close_kill = true;
                self.attack_vars = v;
                return;
            }
            v.do_shove = true;
            v.aim_at_floor = false;
            v.use_charge_delta = 3.0;
        }
        if let Some(f) = firearm.filter(|f| f.ranged) {
            if !f.fire_mode.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("Auto")) {
                v.recoil_delay = self.firearm_recoil_delay(cx.chars, &f);
            }
        }
        self.attack_vars = v;
    }

    /// `calculateHitInfoList` (melee): the shove or weapon hit list,
    /// nearest first, cut to the weapon's hit count.
    pub(crate) fn melee_hit_list(&mut self, cx: &mut StateCx<'_, I>) -> Vec<HitInfo> {
        let sm = cx.sm;
        let (w, _) = self.attack_weapon(sm);
        let mut max_hit = w.max_hit_count;
        if self.attack_vars.do_shove {
            max_hit = if self.weapon_type_of(sm) != WeaponType::Unarmed { 3 } else { 1 };
        }
        if !w.ranged && !cx.ctx.multi_hit_zombies() {
            max_hit = 1;
        }
        if w.bare_hands && (self.items.primary_hand_item().is_none() || self.attack_vars.target_on_ground.is_some()) {
            max_hit = 1;
        }
        if max_hit <= 0 {
            return Vec::new();
        }
        let range = w.max_range * self.weapon_range_mod(sm, cx.chars, &w);
        let aim_floor = self.attack_vars.aim_at_floor;
        let on_ground = self.attack_vars.target_on_ground;
        let mut list: Vec<HitInfo> = Vec::new();
        let ids = cx.world.characters_near(self.vars.x, self.vars.y, self.vars.z, 3.0);
        let from = self.square_xyz();
        for id in ids {
            if id == self.id {
                continue;
            }
            let to = match cx.world.character_mut(id) {
                Some(t) => t.square_xyz(),
                None => continue,
            };
            let line = los_allows_melee(cx.world.los_line_clear(from, to, false));
            let Some(t) = cx.world.character_mut(id) else { continue };
            let prone = t.is_prone();
            if prone && !aim_floor && !self.attack_vars.do_shove {
                continue;
            }
            let hittable = Some(id) == on_ground || (t.vars.shootable && !prone && !aim_floor) || (t.vars.shootable && prone && aim_floor);
            if !hittable {
                continue;
            }
            let Some(mut h) = self.valid_melee_target(t, &w, 3.0, line) else { continue };
            let (min, max) = if prone { (w.min_angle / 1.5, w.max_angle) } else { (w.min_angle, w.max_angle) };
            if h.dot < min || h.dot > max {
                continue;
            }
            // isMeleeAttackRange.
            let r = range;
            if h.dist_sq >= r * r {
                continue;
            }
            h.chance = 100;
            if self.attack_vars.do_shove && Some(id) == on_ground {
                list.clear();
                list.push(h);
                break;
            }
            list.push(h);
        }
        // MeleeTargetComparator: nearest first.
        list.sort_by(|a, b| a.dist_sq.partial_cmp(&b.dist_sq).unwrap_or(std::cmp::Ordering::Equal));
        list.truncate(max_hit as usize);
        list
    }

    /// `CombatManager.attackCollisionCheck(owner, weapon, swipeState,
    /// NONE)` for melee weapons and shoves.
    pub fn attack_collision_check(&mut self, cx: &mut StateCx<'_, I>) -> ScriptResult {
        let (sm, chars) = (cx.sm, cx.chars);
        // `getWeapon(owner)`: the weapon in use; a ranged one takes the
        // firearm path.
        if let Some(item) = self.vars.use_hand_weapon.and_then(|id| self.items.item(id)).cloned() {
            if item.is_ranged() {
                return self.ranged_attack_collision_check(cx, &item);
            }
        }
        if self.vars.performing_shove_animation {
            self.vars.do_shove = true;
        }
        let (w, weapon_item) = self.attack_weapon(sm);
        cx.ctx.event(SimEvent::WeaponSwingHitPoint);
        let ignore_damage = self.vars.do_shove && !self.vars.shove_stomp_anim;
        self.attack_vars.aim_at_floor = self.vars.aim_at_floor;
        self.attack_vars.do_shove = self.vars.do_shove || self.vars.performing_shove_animation;
        let hits = self.melee_hit_list(cx);
        let hit_count = hits.len() as i32;
        if !cx.ctx.is_client() {
            self.process_weapon_endurance(sm, chars, &w)?;
        }
        self.vars.last_hit_count = hit_count;
        let mut split = 1;
        let mut process_condition_loss = false;
        let types = &chars.sim.body_part_types;
        for h in &hits {
            let hit = cx.ctx.rand_next(100) <= h.chance;
            if !hit {
                continue;
            }
            let Some(t) = cx.world.character_mut(h.target) else { continue };
            let (tx, ty) = (t.vars.x, t.vars.y);
            let target_is_zombie = t.is_zombie();
            let distance = {
                let dx = tx - self.vars.x;
                let dy = ty - self.vars.y;
                ((dx * dx + dy * dy) as f64).sqrt() as f32
            };
            // Floor hits: the bone checks are TODO(anim bones); a floor
            // attack hits the head like a missed leg check would not.
            let (mut hit_head, hit_legs) = (0, false);
            if self.vars.aim_at_floor && !w.ranged {
                hit_head = cx.ctx.rand_next_range(0, 3) + 1;
            }
            let (mut damage_split, mut range_del) = self.melee_damage_split(sm, chars, cx.ctx, &w, split, distance)?;
            split += 1;
            self.vars.knockback_attack_mod = 1.0;
            if self.get_variable_string("ZombieHitReaction") == "KnifeDeath" {
                range_del *= 1000.0;
                self.vars.knockback_attack_mod = 0.0;
                cx.ctx.event(SimEvent::WorldSound { radius: 4, volume: 4 });
                self.attack_vars.close_kill = true;
            } else {
                self.attack_vars.close_kill = false;
                cx.ctx.event(SimEvent::WorldSound { radius: 8, volume: 8 });
                if cx.ctx.rand_next(3) == 0 || (self.vars.aim_at_floor && self.vars.do_shove) {
                    cx.ctx.event(SimEvent::WorldSound { radius: 10, volume: 10 });
                } else if cx.ctx.rand_next(7) == 0 {
                    cx.ctx.event(SimEvent::WorldSound { radius: 16, volume: 16 });
                }
            }
            let close_kill = self.attack_vars.close_kill;
            let behind = {
                let t = cx.world.character_mut(h.target).expect("present above");
                t.vars.close_killed = close_kill;
                t.vars.hit_from_angle = h.dot;
                (t.vars.x, t.vars.y, (t.vars.forward_x, t.vars.forward_y))
            };
            if target_is_zombie {
                let is_behind = self.is_behind(behind.0, behind.1, behind.2);
                let t = cx.world.character_mut(h.target).expect("present above");
                t.vars.hit_from_behind = is_behind;
                t.vars.player_attack_position = Some(t.test_dot_side(self.vars.x, self.vars.y).name().to_string());
                t.vars.hit_head_while_on_floor = hit_head;
                t.vars.hit_legs_while_on_floor = hit_legs;
                process_condition_loss = true;
            }
            if w.is_melee() {
                let head = types.index("Head")? as i32;
                let neck = types.index("Neck")? as i32;
                let part = if hit_head > 0 {
                    cx.ctx.rand_next_range(head, neck + 1)
                } else if hit_legs {
                    cx.ctx.rand_next_range(types.index("Groin")? as i32, types.index("Foot_R")? as i32 + 1)
                } else {
                    cx.ctx.rand_next_range(types.index("Hand_L")? as i32, neck + 1)
                };
                let t = cx.world.character_mut(h.target).expect("present above");
                damage_split = t.melee_hit_location_damage(sm, cx.ctx, &w, part, hit_head, hit_legs, damage_split);
                // TODO(items): resolveSpikedArmorDamage, applyKnifeDeathEffect.
            }
            let Some(item) = weapon_item.as_ref() else { continue };
            let Some(mut target) = cx.world.take_character(h.target) else { continue };
            let damage = target.hit(item, self, damage_split, ignore_damage, range_del, cx);
            let (health, close_killed, dead) = (target.health, target.vars.close_killed, target.is_dead());
            cx.world.put_character(target);
            let damage = damage?;
            self.apply_melee_endurance_loss(sm, chars, &w, health, close_killed, damage)?;
            cx.ctx.event(SimEvent::WeaponHitXp { target: h.target, damage: damage_split });
            process_condition_loss = true;
            self.process_maintenance_check(cx, &w)?;
            if dead {
                self.stats.remove(&chars.sim.character_stats.field("STRESS")?, 0.02);
            }
        }
        let process = self.check_for_condition_loss(&w, process_condition_loss);
        LOWER_CONDITION.set(&mut self.state.params, process);
        ATTACKED.set(&mut self.state.params, true);
        Ok(())
    }

    /// `CombatManager.processMaintenanceCheck(owner, weapon, target)` for a
    /// character target.
    fn process_maintenance_check(&mut self, cx: &mut StateCx<'_, I>, w: &WeaponStats) -> ScriptResult {
        if !self.is_actually_attacking_with_melee_weapon(cx.sm) || cx.ctx.is_client() {
            return Ok(());
        }
        self.weapon_damage_check(cx, w.id, 0, 1.0);
        let next_bool = |n| crate::character::body_part::next_bool(cx.ctx, n);
        if next_bool(2) && !(self.vars.aim_at_floor && self.vars.do_shove) {
            if w.two_hand && !self.is_item_in_both_hands(w.id) && next_bool(3) {
                return Ok(());
            }
            if !self.vars.do_shove {
                let mut amount = 2.0f32;
                if w.condition_lower_chance > 10 {
                    amount = amount * 10.0 / w.condition_lower_chance as f32;
                }
                self.add_xp(cx.sm, cx.chars, cx.chars.perks.constant("Maintenance").as_ref(), amount, cx.ctx)?;
            }
        }
        Ok(())
    }

    /// `isActuallyAttackingWithMeleeWeapon()`.
    pub fn is_actually_attacking_with_melee_weapon(&self, sm: &pz_scripts::ScriptManager) -> bool {
        let Some(w) = self.items.primary_hand_item().and_then(|i| WeaponStats::of(sm, i)) else { return false };
        if self.attack_vars.weapon.is_none() || w.bare_hands || w.ranged {
            return false;
        }
        !self.vars.do_shove
    }

    /// `InventoryItem.damageCheck(skill, multiplier)` on the equipped
    /// weapon `id` (sharpness and head condition are TODO(attributes)).
    pub fn weapon_damage_check(&mut self, cx: &mut StateCx<'_, I>, id: i32, skill: i32, multiplier: f32) -> bool {
        let multiplier = multiplier.max(0.0);
        let equipped = self.items.primary_hand_item == Some(id) || self.items.secondary_hand_item == Some(id);
        let mut skill = skill;
        if equipped {
            skill += self.perk_level(cx.chars.perks.constant("Maintenance").as_ref()) + self.weapon_level(cx.sm, cx.chars) / 2;
        }
        let Some(item) = self.items.item(id) else { return false };
        let clc = WeaponStats::of(cx.sm, item).map_or(10, |w| w.condition_lower_chance);
        if crate::character::body_part::next_bool(cx.ctx, (clc as f32 * multiplier + skill as f32) as i32) {
            if let Some(item) = self.items.item_mut(id) {
                item.condition -= 1;
            }
            return true;
        }
        false
    }
}

impl<I: 'static> State<I> for SwipeStatePlayer {
    fn id(&self) -> StateId {
        ID
    }
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, false, false, true)
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if owner.vars.hit_reaction == "HitReaction" {
            owner.clear_variable("HitReaction");
        }
        owner.vars.initiate_attack = false;
        LOWER_CONDITION.set(&mut owner.state.params, false);
        ATTACKED.set(&mut owner.state.params, false);
        owner.calculate_attack_vars(cx);
        // doAttack.
        owner.vars.use_charge_delta = 2.0f32.min(90.0) / 25.0;
        let primary = owner.items.primary_hand_item().filter(|i| crate::character::items::is_hand_weapon(i)).map(|i| i.id);
        let attack_item = if owner.attack_vars.do_shove { None } else { primary };
        // `bareHands` is a HandWeapon too: the weapon in use is cleared to
        // it (by id: `None`).
        owner.set_use_hand_weapon(attack_item);
        owner.vars.recoil_delay = owner.attack_vars.recoil_delay as f32;
        if let Some(f) = attack_item.and_then(|id| owner.items.item(id)).and_then(|i| FirearmStats::of(cx.sm, i)) {
            owner.set_aiming_delay_after_shot(cx.chars, &f);
        } else if owner.bare_hands.is_some() {
            // `setAimingDelay(player, bareHands)`: no recoil, no aiming time.
            let max = owner.items.primary_hand_item().filter(|i| crate::character::items::is_hand_weapon(i)).map_or(0.0, |i| {
                FirearmStats::of(cx.sm, i).map_or(0.0, |f| f.aiming_time as f32)
            });
            owner.vars.aiming_delay = crate::character::firearm::hit_chance::clamp(owner.vars.aiming_delay, 0.0, max);
        }
        if let Some(f) = attack_item.and_then(|id| owner.items.item(id)).and_then(|i| FirearmStats::of(cx.sm, i)) {
            if !owner.vars.remote {
                owner.vars.recoil_var_y = 0.0;
                let recoil = f.recoil_delay as f32;
                let current = owner.firearm_recoil_delay(cx.chars, &f) as f32;
                owner.vars.recoil_var_x = if current < recoil && recoil != 0.0 { 1.0 - current / recoil } else { 0.0 };
            }
            if f.fire_mode.as_deref() == Some("Auto") {
                let rate = owner.items.item(f.id).and_then(|i| i.weapon_data()).map_or(0.0, |d| d.cyclic_rate_multiplier);
                owner.set_variable_float("autoShootSpeed", if rate > 0.0 { rate } else { 1.0 });
                owner.set_variable_float("autoShootVarY", 0.0);
                owner.set_variable_float("autoShootVarX", 1.0);
            } else {
                let v = 0.8 + (1.0 - owner.firearm_recoil_delay(cx.chars, &f) as f32 / f.recoil_delay as f32);
                owner.set_variable_float("singleShootSpeed", 0.5f32.max(v));
            }
        }
        owner.set_variable_bool("ShotDone", false);
        owner.vars.performing_shove_animation = false;
        owner.vars.performing_grapple_grab_animation = false;
        let aim_at_floor_anim = owner.attack_vars.aim_at_floor;
        let mut stomp = false;
        cx.ctx.event(SimEvent::WeaponSwing);
        if cx.ctx.trigger_hook("WeaponSwing") {
            owner.revert_to_previous_state(instance(), cx);
        }
        cx.ctx.event(SimEvent::StopAllActionQueue);
        let was_shove = owner.vars.do_shove;
        owner.vars.aim_at_floor = owner.attack_vars.aim_at_floor;
        owner.vars.do_shove = owner.attack_vars.do_shove;
        owner.vars.use_charge_delta = owner.attack_vars.use_charge_delta;
        owner.vars.target_on_ground = owner.attack_vars.target_on_ground;
        let (w, _) = owner.attack_weapon(cx.sm);
        if !owner.vars.do_shove && !was_shove && !w.ranged {
            owner.clear_variable("PlayedSwingSound");
        } else if owner.vars.do_shove || was_shove {
            stomp = owner.vars.target_on_ground.is_some() && owner.attack_vars.target_distance < 0.6;
            if stomp && !cx.ctx.option_auto_prone_attack() {
                // isStompingDisabled: no manual floor or melee button held.
                stomp = owner.vars.melee_button_down || owner.vars.manual_floor_attack_button_down;
            }
            cx.ctx.event(SimEvent::PlaySound(if stomp { "AttackStomp" } else { "AttackShove" }.into()));
        }
        owner.clear_variable("PistolWhipAnim");
        owner.vars.shove_stomp_anim = stomp;
        owner.set_variable_bool("AimFloorAnim", aim_at_floor_anim);
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if !owner.environment.dragging_corpse {
            cx.ctx.event(SimEvent::StopAllActionQueue);
        }
    }

    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        let name = e.event.event_name.as_str();
        let is = |n: &str| name.eq_ignore_ascii_case(n);
        let b = parse_boolean(e.event.parameter_value.as_deref());
        if is("ActiveAnimFinishing") || is("NonLoopedAnimFadeOut") {
            if LOWER_CONDITION.get(&mut owner.state.params) {
                LOWER_CONDITION.set(&mut owner.state.params, false);
                // WeaponLowerConditionEvent: the music intensity event.
            }
        } else if is("AttackAnim") {
            owner.vars.performing_attack_animation = b;
        } else if is("BlockTurn") {
            owner.vars.ignore_movement = b;
        } else if is("ShoveAnim") {
            owner.vars.performing_shove_animation = b;
        } else if is("StompAnim") {
            owner.vars.performing_stomp_animation = b;
        } else if is("GrappleGrabAnim") {
            owner.vars.performing_grapple_grab_animation = b;
        } else if is("AttackCollisionCheck") {
            if !ATTACKED.get(&mut owner.state.params) && owner.is_local_player(cx.ctx) {
                if let Err(err) = owner.attack_collision_check(cx) {
                    cx.ctx.event(SimEvent::Error(err.to_string()));
                }
            }
        } else if is("BlockMovement") {
            if cx.ctx.attack_block_movements() {
                owner.set_variable_bool("SlowingMovement", b);
            }
        } else if is("WeaponEmptyCheck") {
            if let Some(s) = owner.vars.click_sound.clone() {
                if owner.is_local_player(cx.ctx) {
                    cx.ctx.event(SimEvent::PlaySound(s));
                    owner.vars.recoil_delay = 10.0;
                }
            }
        } else if is("SetVariable") {
            if let AnimEventKind::SetVariable { reference, .. } = &e.event.kind {
                if reference.name().eq_ignore_ascii_case("ShotDone") {
                    let rack = owner.use_hand_weapon_rack_after_shoot(cx.sm);
                    let v = owner.get_variable_boolean("ShotDone") && rack;
                    owner.set_variable_bool("ShotDone", v);
                }
            }
        } else if is("ShotDone") {
            if owner.use_hand_weapon_rack_after_shoot(cx.sm) {
                owner.set_variable_bool("ShotDone", true);
            }
        } else if is("SetMeleeDelay") {
            owner.vars.melee_delay = pz_anim::vars::try_parse_float(e.event.parameter_value.as_deref().unwrap_or(""), 0.0);
        } else if is("PlaySwingSound") {
            if owner.is_local_player(cx.ctx) && !owner.get_variable_boolean("PlayedSwingSound") {
                owner.set_variable_bool("PlayedSwingSound", true);
                cx.ctx.event(SimEvent::SwingSound(e.event.parameter_value.clone().unwrap_or_default()));
            }
        } else if is("PlayerVoiceSound") {
            if !owner.get_variable_boolean("PlayerVoiceSound") {
                owner.set_variable_bool("PlayerVoiceSound", true);
                if owner.is_player() {
                    cx.ctx.event(SimEvent::PlayerVoiceSoundNamed(e.event.parameter_value.clone().unwrap_or_default()));
                }
            }
        } else if is("PistolWhipAnim") {
            let v = pz_anim::vars::try_parse_boolean(e.event.parameter_value.as_deref().unwrap_or(""));
            owner.set_variable_bool("PistolWhipAnim", v);
        } else if is("SitGroundStarted") {
            owner.set_variable_string("SitGroundAnim", "Idle");
        }
    }

    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.environment.sprinting = false;
        owner.player.input.force_sprint = false;
        owner.vars.ignore_movement = false;
        owner.vars.performing_shove_animation = false;
        owner.vars.performing_stomp_animation = false;
        owner.vars.performing_grapple_grab_animation = false;
        owner.vars.performing_attack_animation = false;
        owner.vars.shove_stomp_anim = false;
        owner.set_variable_bool("AimFloorAnim", false);
        owner.vars.block_movement = false;
        if let Err(err) = owner.stomp_foot_damage(cx.sm, cx.chars, cx.ctx) {
            cx.ctx.event(SimEvent::Error(err.to_string()));
        }
        let (w, _) = owner.attack_weapon(cx.sm);
        owner.clear_variable("ZombieHitReaction");
        owner.vars.attack_started = false;
        owner.vars.attack_type = String::new();
        owner.vars.do_shove = false;
        owner.vars.do_grapple = false;
        for v in ["RackWeapon", "PlayedSwingSound", "PlayerVoiceSound", "PistolWhipAnim", "SlowingMovement"] {
            owner.clear_variable(v);
        }
        let attacked = ATTACKED.get(&mut owner.state.params);
        if !w.bare_hands {
            let cond = owner.items.item(w.id).map_or(0, |i| i.condition);
            if cond <= 0 || (attacked && w.use_self) {
                if owner.items.primary_hand_item == Some(w.id) {
                    owner.items.primary_hand_item = None;
                }
                if owner.items.secondary_hand_item == Some(w.id) {
                    owner.items.secondary_hand_item = None;
                }
            }
        }
        if owner.vars.ranged_weapon_empty {
            owner.vars.recoil_delay = 10.0;
        }
        owner.vars.ranged_weapon_empty = false;
        owner.vars.click_sound = None;
        if attacked {
            cx.ctx.event(SimEvent::PlayerAttackFinished);
        }
    }

    fn is_attacking(&self, _owner: &GameCharacter<I>) -> bool {
        false
    }
}

/// `CombatManager`'s line-of-sight test for a melee target: not
/// `Blocked` and not `ClearThroughClosedDoor`.
fn los_allows_melee(r: crate::character::state::TestResults) -> bool {
    use crate::character::state::TestResults;
    r != TestResults::Blocked && r != TestResults::ClearThroughClosedDoor
}

impl<I: 'static> GameCharacter<I> {
    /// `(fastfloor(x), fastfloor(y), fastfloor(z))`.
    pub fn square_xyz(&self) -> (i32, i32, i32) {
        let f = |v: f32| {
            let i = v as i32;
            if v < i as f32 { i - 1 } else { i }
        };
        (f(self.vars.x), f(self.vars.y), f(self.vars.z))
    }

    /// The character's `PathMover::mover_id()` (kind and id), for the world's
    /// path calls.
    pub fn mover_id_of(&self) -> pz_pathfind::mover::MoverId {
        pz_pathfind::mover::PathMover::mover_id(self)
    }
}
