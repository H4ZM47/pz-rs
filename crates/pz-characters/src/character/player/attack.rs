//! Attack input: `IsoLivingCharacter.AttemptAttack`, `IsoPlayer.DoAttack`,
//! `CombatManager.pressedAttack` (which leads the action context into
//! `SwipeStatePlayer` through `initiateAttack`/`attackStarted`), and the
//! injury speed modifiers `calculateWalkSpeed` / `calculateCombatSpeed` use.

use pz_anim::AnimationVariableSource;
use pz_scripts::ScriptResult;

use super::PlayerUpdate;
use crate::character::body_part::BodyPart;
use crate::character::combat::{config, WeaponStats, WeaponType};
use crate::character::state::StateCx;
use crate::character::states::ids;
use crate::character::GameCharacter;

/// Body part indices (`BodyPartType.ToIndex`).
pub mod part {
    pub const HAND_R: usize = 1;
    pub const FOREARM_R: usize = 3;
    pub const UPPERARM_R: usize = 5;
    pub const TORSO_UPPER: usize = 6;
    pub const NECK: usize = 9;
    pub const GROIN: usize = 10;
    pub const UPPERLEG_L: usize = 11;
    pub const FOOT_L: usize = 15;
    pub const FOOT_R: usize = 16;
    pub const MAX: usize = 17;
}

/// `WeaponType.getPossibleAttack()`: `(attack type id, weight)`.
pub fn possible_attacks(t: WeaponType) -> &'static [(&'static str, i64)] {
    match t {
        WeaponType::Unarmed | WeaponType::Handgun | WeaponType::Firearm | WeaponType::Throwing => &[("", 10)],
        WeaponType::TwoHanded | WeaponType::OneHanded | WeaponType::Knife => &[("default", 20), ("overhead", 10), ("uppercut", 10)],
        WeaponType::Heavy => &[("default", 20), ("overhead", 10)],
        WeaponType::Spear | WeaponType::Chainsaw => &[("default", 10)],
    }
}

/// `WeaponType.isCanMiss()`.
pub fn can_miss(t: WeaponType) -> bool {
    !t.is_ranged()
}

/// `calcFractureInjurySpeed`.
fn calc_fracture_injury_speed(p: &BodyPart) -> f32 {
    let mut r = 0.4f32;
    if p.fracture_time > 10.0 {
        r = 0.7;
    }
    if p.fracture_time > 20.0 {
        r = 1.0;
    }
    if p.splint_factor > 0.0 {
        r -= 0.2;
        r -= (p.splint_factor / 10.0).min(0.8);
    }
    r.max(0.0)
}

impl<I: 'static> GameCharacter<I> {
    /// `calculateInjurySpeed(bodyPart, doPain)`.
    pub fn calculate_injury_speed(&self, p: &BodyPart, index: usize, do_pain: bool, ctx: &dyn crate::character::SimContext) -> f32 {
        let mut m = 0.0f32;
        let foot = index == part::FOOT_L || index == part::FOOT_R;
        if foot
            && (p.burn_time > 5.0
                || p.stitch_time > 0.0
                || p.bite_time > 0.0
                || p.deep_wounded
                || p.splint
                || p.fracture_time > 0.0
                || p.have_glass)
        {
            m = 1.0;
            if p.stitch_time > 0.0 {
                m = 0.7;
            }
            if p.bandaged {
                m *= 0.7;
            }
            if p.fracture_time > 0.0 {
                m = calc_fracture_injury_speed(p);
            }
        }
        if p.have_bullet {
            return 1.0;
        }
        if p.scratch_time > 2.0
            || p.cut_time > 5.0
            || p.burn_time > 0.0
            || p.deep_wound_time > 0.0
            || p.splint
            || p.fracture_time > 0.0
            || p.bite_time > 0.0
        {
            m += p.scratch_time / p.scratch_speed_modifier
                + p.cut_time / p.cut_speed_modifier
                + p.burn_time / p.burn_speed_modifier
                + p.deep_wound_time / p.deep_wound_speed_modifier;
            m += p.bite_time / 20.0;
            if p.bandaged {
                m /= 2.0;
            }
            if p.fracture_time > 0.0 {
                m = calc_fracture_injury_speed(p);
            }
        }
        if do_pain {
            let pain = p.pain(ctx);
            if pain > 20.0 {
                m += pain / 10.0;
            }
        }
        m
    }

    /// `getFootInjurySpeedModifier()`.
    pub fn foot_injury_speed_modifier(&self, ctx: &dyn crate::character::SimContext) -> f32 {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        let mut left = true;
        for i in part::GROIN..part::MAX {
            let s = self.body_damage.body_part(i).map_or(0.0, |p| self.calculate_injury_speed(p, i, false, ctx));
            if left {
                l += s;
            } else {
                r += s;
            }
            left = !left;
        }
        if l > r {
            -(l + r)
        } else {
            l + r
        }
    }

    /// `getArmsInjurySpeedModifier()`.
    fn arms_injury_speed_modifier(&self, ctx: &dyn crate::character::SimContext) -> f32 {
        let mut speed = 1.0f32;
        for i in [part::HAND_R, part::FOREARM_R, part::UPPERARM_R] {
            let m = self.body_damage.body_part(i).map_or(0.0, |p| self.calculate_injury_speed(p, i, true, ctx));
            if m > 0.0 {
                speed -= m;
            }
        }
        speed
    }

    fn primary_weapon(&self, sm: &pz_scripts::ScriptManager) -> Option<WeaponStats> {
        self.items.primary_hand_item().and_then(|i| WeaponStats::of(sm, &i.borrow()))
    }

    fn perk_lvl(&self, chars: &crate::Characters, f: &str) -> i32 {
        self.perk_level(chars.perks.constant(f).as_ref())
    }

    fn moodle_lv(&self, sm: &pz_scripts::ScriptManager, f: &str) -> i32 {
        self.moodles.moodle_level_of(sm, f).unwrap_or(0)
    }

    /// `calculateCombatSpeed()`.
    pub fn calculate_combat_speed(&self, sm: &pz_scripts::ScriptManager, chars: &crate::Characters, ctx: &dyn crate::character::SimContext) -> ScriptResult<f32> {
        let mut s = 0.8f32;
        let weapon = self.primary_weapon(sm);
        if let Some(item) = self.items.primary_hand_item().filter(|i| crate::character::items::is_hand_weapon(&i.borrow())) {
            s *= crate::character::items::script_item(sm, &item.borrow()).map_or(1.0, |i| i.borrow().base_speed);
        }
        let wt = self.weapon_type_of(sm);
        let secondary = self.items.secondary_hand_item().map(|i| i.borrow().id);
        if let Some(w) = &weapon {
            if w.two_hand && secondary != Some(w.id) {
                s *= 0.77;
            }
            if w.categories.iter().any(|c| c == "Axe") {
                s *= if self.has_trait(sm, "AXEMAN")? { 1.0 } else { 0.8 };
            }
        }
        s -= self.moodle_lv(sm, "ENDURANCE") as f32 * 0.07;
        s -= self.moodle_lv(sm, "HEAVY_LOAD") as f32 * 0.07;
        s += self.weapon_level(sm, chars) as f32 * 0.03;
        s += self.perk_lvl(chars, "Fitness") as f32 * 0.02;
        if self.items.secondary_hand_item().is_some_and(|i| matches!(i.borrow().kind, pz_inventory::ItemKind::Container(_))) {
            s *= 0.95;
        }
        s *= ctx.rand_next_float(1.1, 1.2);
        s *= self.vars.combat_speed_modifier;
        s *= self.arms_injury_speed_modifier(ctx);
        if self.body_damage.thermoregulator.is_some() {
            let hypo = self.moodle_lv(sm, "HYPOTHERMIA");
            let hyper = self.moodle_lv(sm, "HYPERTHERMIA");
            s *= crate::character::thermoregulator::Thermoregulator::combat_modifier(hypo, hyper);
        }
        s = s.min(1.6);
        s = s.max(0.8);
        if weapon.as_ref().is_some_and(|w| w.two_hand) && wt == WeaponType::Heavy {
            s *= 1.2;
        }
        Ok(s)
    }

    /// `canPerformHandToHandCombat()` (the hand models' mask variables are
    /// the renderer's; TODO(items): `getItemReplacementPrimaryHand`).
    pub fn can_perform_hand_to_hand_combat(&self) -> bool {
        true
    }

    /// `IsoPlayer.clearHandToHandAttack()`.
    pub fn clear_hand_to_hand_attack(&mut self) {
        self.vars.do_shove = false;
        self.vars.do_grapple = false;
        self.vars.initiate_attack = false;
        self.vars.attack_started = false;
        self.vars.attack_type = String::new();
    }

    /// `isDoHandToHandAttack()`.
    pub fn is_do_hand_to_hand_attack(&self) -> bool {
        self.vars.do_shove || self.vars.do_grapple
    }

    /// `IsoLivingCharacter.AttemptAttack(chargeDelta)` then
    /// `IsoPlayer.DoAttack(chargeDelta)`.
    pub fn attempt_attack(&mut self, charge_delta: f32, out: &mut PlayerUpdate, cx: &mut StateCx<'_, I>) -> ScriptResult<bool> {
        let _ = charge_delta;
        if self.items.primary_hand_item().is_some_and(|i| crate::character::items::is_hand_weapon(&i.borrow())) {
            self.calculate_attack_vars(cx);
            self.vars.do_shove = self.attack_vars.do_shove;
            // TODO(grappling): attackVars.doGrapple.
            self.vars.do_grapple = false;
            if cx.ctx.trigger_hook("Attack") {
                return Ok(false);
            }
        }
        // DoAttack(chargeDelta, null).
        if !self.player.authorize_hand_to_hand_action {
            return Ok(false);
        }
        self.vars.click_sound = None;
        self.pressed_attack(out, cx)?;
        Ok(false)
    }

    /// `CombatManager.pressedAttack(player)` (local player).
    pub fn pressed_attack(&mut self, out: &mut PlayerUpdate, cx: &mut StateCx<'_, I>) -> ScriptResult {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        let was_sprinting = self.is_sprinting(sm, ctx);
        self.environment.sprinting = false;
        self.player.input.force_sprint = false;
        if self.vars.attack_started || self.is_current_state_id(ids::PLAYER_HIT_REACTION) || self.is_current_state_id(ids::FISHING) {
            return Ok(());
        }
        if !self.can_perform_hand_to_hand_combat() {
            self.clear_hand_to_hand_attack();
            return Ok(());
        }
        if !self.vars.do_shove && !self.is_weapon_ready(cx) {
            return Ok(());
        }
        if !self.vars.attack_started {
            self.set_var_bool("StartedAttackWhileSprinting", was_sprinting);
        }
        self.vars.initiate_attack = true;
        self.vars.attack_started = true;
        self.vars.critical_hit = false;
        self.vars.attack_from_behind = false;
        let wt = if self.vars.do_shove { WeaponType::Unarmed } else { self.weapon_type_of(sm) };
        // WeightedList.getRandom().
        let list = possible_attacks(wt);
        let total: i64 = list.iter().map(|e| e.1).sum();
        let roll = ctx.rand_next(total as i32) as i64;
        let mut sum = 0i64;
        let mut pick = list[0].0;
        for (t, w) in list {
            sum += w;
            if roll < sum {
                pick = t;
                break;
            }
        }
        self.vars.attack_type = pick.to_string();
        self.vars.combat_speed = self.calculate_combat_speed(sm, chars, ctx)?;
        self.calculate_attack_vars(cx);
        if self.variable_string("Weapon") == "throwing" && !self.attack_vars.do_shove {
            self.player.attack_anim_throw_timer = self.player.now_ms + 2000;
            self.vars.is_aiming = true;
        }
        if self.attack_vars.do_shove && !self.player.authorize_hand_to_hand {
            self.vars.do_shove = false;
            self.vars.initiate_attack = false;
            self.vars.attack_started = false;
            self.vars.attack_type = String::new();
            return Ok(());
        }
        let weapon_id = self.attack_vars.weapon;
        self.vars.use_hand_weapon = weapon_id;
        self.vars.aim_at_floor = self.attack_vars.aim_at_floor;
        self.vars.do_shove = self.attack_vars.do_shove;
        self.vars.target_on_ground = self.attack_vars.target_on_ground;
        let firearm = weapon_id
            .and_then(|id| self.items.item(id))
            .and_then(|i| crate::character::firearm::FirearmStats::of(sm, &i.borrow()));
        if let Some(f) = firearm.as_ref().filter(|_| wt.is_ranged()) {
            self.vars.recoil_delay = self.firearm_recoil_delay(chars, f) as f32;
            // TODO(network): PlayerEmptyShot for an empty magazine.
        }
        match ctx.rand_next_range(0, 3) {
            0 => self.vars.attack_variation_x = ctx.rand_next_float(-1.0, -0.5),
            1 => self.vars.attack_variation_x = 0.0,
            2 => self.vars.attack_variation_x = ctx.rand_next_float(0.5, 1.0),
            _ => {}
        }
        self.vars.attack_variation_y = 0.0;
        // calculateHitInfoList.
        if let Some(f) = firearm.as_ref().filter(|f| f.ranged) {
            self.calculate_ranged_hit_info_list(cx, f)?;
        } else {
            self.hit_info_list = self.melee_hit_list(cx);
        }
        let bare = weapon_id.is_none();
        let closest = self.hit_info_list.first().map(|h| h.target);
        let Some(target) = closest else {
            if self.is_player_aiming() && !self.vars.melee_pressed && !bare {
                self.vars.do_shove = false;
            }
            if self.is_player_aiming() && !self.vars.grapple_pressed && !bare {
                self.vars.do_grapple = false;
            }
            self.player.last_attack_was_hand_to_hand = self.is_do_hand_to_hand_attack();
            if can_miss(wt) && !self.vars.aim_at_floor {
                self.vars.attack_type = "miss".into();
            }
            return Ok(());
        };
        let Some(t) = cx.world.take_character(target) else { return Ok(()) };
        let r = self.pressed_attack_on(&t, wt, cx);
        cx.world.put_character(t);
        let crit_chance = r?;
        if let Some(t) = cx.world.character_mut(target) {
            if !t.vars.on_floor {
                t.vars.hit_from_behind = self.vars.attack_from_behind;
            }
        }
        let _ = out;
        let _ = crit_chance;
        self.player.last_attack_was_hand_to_hand = self.is_do_hand_to_hand_attack();
        Ok(())
    }

    /// The part of `pressedAttack` with a closest target: the attack type,
    /// `attackFromBehind` and the critical hit roll.
    fn pressed_attack_on(&mut self, t: &GameCharacter<I>, wt: WeaponType, cx: &mut StateCx<'_, I>) -> ScriptResult<f32> {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        // TODO(world objects): SPEAR_STAB through a transparent wall.
        let fwd = (self.vars.forward_x, self.vars.forward_y);
        self.vars.attack_from_behind = self.is_behind(t.vars.x, t.vars.y, fwd);
        let dist = crate::character::zombie::math::distance_to(t.vars.x, t.vars.y, self.vars.x, self.vars.y);
        self.set_var_float("TargetDist", dist);
        let mut crit = self.calculate_crit_chance_in(sm, chars, ctx, Some(&*cx.world), t) as f32;
        let fake_spear = |c: &Self| c.items.primary_hand_item().is_none_or(|i| crate::character::items::has_tag(sm, &i.borrow(), "FAKE_SPEAR"));
        if !self.attack_vars.aim_at_floor && dist > 1.25 && wt == WeaponType::Spear {
            // TODO(world): getClosestTo(target) (no other character is closer).
            self.vars.attack_type = "overhead".into();
            if fake_spear(self) {
                crit += 30.0;
            }
        }
        if self.vars.attack_from_behind {
            if t.is_zombie() && t.vars.target.is_none() && fake_spear(self) {
                crit += config::ADDITIONAL_CRITICAL_HIT_CHANCE_FROM_BEHIND;
            } else {
                crit += config::ADDITIONAL_CRITICAL_HIT_CHANCE_DEFAULT;
            }
        }
        if t.is_player() && wt.is_ranged() && !self.is_do_hand_to_hand_attack() {
            if let Some(f) = self.attack_vars.weapon.and_then(|id| self.items.item(id)).and_then(|i| crate::character::firearm::FirearmStats::of(sm, &i.borrow())) {
                crit = f.stop_power * (1.0 + self.perk_lvl(chars, "Aiming") as f32 / 15.0);
            }
        }
        let no_crit = self.items.primary_hand_item().is_some_and(|i| crate::character::items::has_tag(sm, &i.borrow(), "NO_CRITICALS"));
        if no_crit {
            crit = 0.0;
        }
        self.vars.critical_hit = (ctx.rand_next(100) as f32) < crit;
        if self.vars.attack_from_behind && self.attack_vars.close_kill && t.is_zombie() && t.vars.target.is_none() && !fake_spear(self) {
            self.vars.critical_hit = true;
        }
        if self.vars.critical_hit && !self.attack_vars.close_kill && !self.vars.do_shove && wt == WeaponType::Knife {
            self.vars.critical_hit = false;
        }
        if self.stats.num_chasing_zombies() > 1 && self.attack_vars.close_kill && !self.vars.do_shove && wt == WeaponType::Knife {
            self.vars.critical_hit = false;
        }
        if no_crit {
            self.vars.critical_hit = false;
        }
        if self.vars.critical_hit {
            self.vars.combat_speed *= 1.1;
        }
        Ok(crit)
    }

    /// `isWeaponReady()`: no timed action overrides the hand models, the
    /// primary hand model is ready (the bare hands always are, else
    /// [`StateWorld::primary_hand_model_ready`])
    /// and an aimed firearm has its muzzle attachment.
    ///
    /// [`StateWorld::primary_hand_model_ready`]: crate::character::state::StateWorld::primary_hand_model_ready
    pub fn is_weapon_ready(&self, cx: &StateCx<'_, I>) -> bool {
        // IsoLivingCharacter.isPrimaryHandModelReady: the bare hands (no
        // useHandWeapon) are always ready.
        let bare = self.vars.use_hand_weapon.and_then(|i| self.items.item(i)).is_none_or(|i| i.borrow().type_.as_str() == "BareHands");
        let ready = bare || cx.world.primary_hand_model_ready(self.id);
        if !ready {
            return false;
        }
        let aimed = self.vars.use_hand_weapon.and_then(|i| self.items.item(i)).is_some_and(|i| i.borrow().is_aimed_firearm());
        !aimed || cx.world.muzzle_attachment(self.id).is_some()
    }

    /// `CanAttack()`.
    pub fn can_attack(&mut self, cx: &StateCx<'_, I>) -> bool {
        if self.vars.performing_attack_animation
            || self.get_variable_boolean("IsRacking")
            || self.get_variable_boolean("IsUnloading")
            || !self.variable_string("RackWeapon").is_empty()
        {
            return false;
        }
        if self.sit_on_ground {
            return false;
        }
        let sm = cx.sm;
        if let Some(item) = self.items.primary_hand_item() {
            let item = &*item.borrow();
            if crate::character::items::is_hand_weapon(item) && WeaponStats::of(sm, item).is_some_and(|w| !w.swing_anim.is_empty()) {
                self.vars.use_hand_weapon = Some(item.id);
            }
        }
        let Some(id) = self.vars.use_hand_weapon else { return true };
        let Some(item) = self.items.item(id) else { return true };
        let item = &*item.borrow();
        if item.condition <= 0 {
            // TODO(items): drop the broken weapon from the hands.
            self.vars.use_hand_weapon = None;
            return false;
        }
        if !self.is_weapon_ready(cx) {
            return false;
        }
        // TODO(items): isCantAttackWithLowestEndurance.
        true
    }

    /// `AttemptAttack`'s `bareHands` test: no hand weapon in the primary
    /// hand.
    pub fn primary_is_bare_hands(&self) -> bool {
        !self.items.primary_hand_item().is_some_and(|i| crate::character::items::is_hand_weapon(&i.borrow()))
    }

}
