//! `CombatManager.processHit` and what it calls: the hit reaction by shot
//! direction (`resolveHitReaction`, `calculateShotDirection`,
//! `processTargetedHit`), the blood (`applyBlood`), the ragdoll body part
//! (`getBodyPart`) and the combat data the ragdoll hit reaction reads
//! (`createCombatData`). Melee hits on zombies go through here too.

use std::rc::Rc;

use pz_anim::AnimationVariableSource;
use pz_inventory::InventoryItem;
use pz_scripts::registry::RegistryKey;
use pz_scripts::ScriptManager;

use super::ballistics::CombatDamageData;
use super::physics::RagdollBodyPart;
use super::weapon::FirearmStats;
use crate::character::combat::WeaponStats;
use crate::character::items::blood_part;
use crate::character::state::StateCx;
use crate::character::states::ids;
use crate::character::{GameCharacter, SimContext, SimEvent};

/// `zombie.combat.HitReaction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitReaction {
    None,
    Shot,
    ShotHeadFwd,
    ShotHeadFwd02,
    ShotHeadBwd,
    ShotBelly,
    ShotBellyStep,
    ShotBellyStepBehind,
    ShotChest,
    ShotChestL,
    ShotChestR,
    ShotChestStepL,
    ShotChestStepR,
    ShotShoulderL,
    ShotShoulderR,
    ShotShoulderStepL,
    ShotShoulderStepR,
    ShotLegL,
    ShotLegR,
    HeadLeft,
    HeadTop,
    Uppercut,
    KnifeDeath,
    HeadRight,
    HitSpear1,
    HitSpear2,
    OnKnees,
    Eating,
    GettingUpFront,
    Floor,
}

impl HitReaction {
    const ALL: [HitReaction; 30] = [
        HitReaction::None,
        HitReaction::Shot,
        HitReaction::ShotHeadFwd,
        HitReaction::ShotHeadFwd02,
        HitReaction::ShotHeadBwd,
        HitReaction::ShotBelly,
        HitReaction::ShotBellyStep,
        HitReaction::ShotBellyStepBehind,
        HitReaction::ShotChest,
        HitReaction::ShotChestL,
        HitReaction::ShotChestR,
        HitReaction::ShotChestStepL,
        HitReaction::ShotChestStepR,
        HitReaction::ShotShoulderL,
        HitReaction::ShotShoulderR,
        HitReaction::ShotShoulderStepL,
        HitReaction::ShotShoulderStepR,
        HitReaction::ShotLegL,
        HitReaction::ShotLegR,
        HitReaction::HeadLeft,
        HitReaction::HeadTop,
        HitReaction::Uppercut,
        HitReaction::KnifeDeath,
        HitReaction::HeadRight,
        HitReaction::HitSpear1,
        HitReaction::HitSpear2,
        HitReaction::OnKnees,
        HitReaction::Eating,
        HitReaction::GettingUpFront,
        HitReaction::Floor,
    ];

    /// `getValue()`.
    pub fn value(self) -> &'static str {
        match self {
            HitReaction::None => "",
            HitReaction::Shot => "Shot",
            HitReaction::ShotHeadFwd => "ShotHeadFwd",
            HitReaction::ShotHeadFwd02 => "ShotHeadFwd02",
            HitReaction::ShotHeadBwd => "ShotHeadBwd",
            HitReaction::ShotBelly => "ShotBelly",
            HitReaction::ShotBellyStep => "ShotBellyStep",
            HitReaction::ShotBellyStepBehind => "ShotBellyStepBehind",
            HitReaction::ShotChest => "ShotChest",
            HitReaction::ShotChestL => "ShotChestL",
            HitReaction::ShotChestR => "ShotChestR",
            HitReaction::ShotChestStepL => "ShotChestStepL",
            HitReaction::ShotChestStepR => "ShotChestStepR",
            HitReaction::ShotShoulderL => "ShotShoulderL",
            HitReaction::ShotShoulderR => "ShotShoulderR",
            HitReaction::ShotShoulderStepL => "ShotShoulderStepL",
            HitReaction::ShotShoulderStepR => "ShotShoulderStepR",
            HitReaction::ShotLegL => "ShotLegL",
            HitReaction::ShotLegR => "ShotLegR",
            HitReaction::HeadLeft => "HeadLeft",
            HitReaction::HeadTop => "HeadTop",
            HitReaction::Uppercut => "Uppercut",
            HitReaction::KnifeDeath => "KnifeDeath",
            HitReaction::HeadRight => "HeadRight",
            HitReaction::HitSpear1 => "HitSpearDeath1",
            HitReaction::HitSpear2 => "HitSpearDeath2",
            HitReaction::OnKnees => "OnKnees",
            HitReaction::Eating => "Eating",
            HitReaction::GettingUpFront => "GettingUpFront",
            HitReaction::Floor => "Floor",
        }
    }

    /// `fromString(value)` (case-insensitive; `NONE` when unknown).
    pub fn from_string(v: &str) -> HitReaction {
        Self::ALL.iter().copied().find(|h| h.value().eq_ignore_ascii_case(v)).unwrap_or(HitReaction::None)
    }
}

/// `zombie.combat.ShotDirection`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotDirection {
    North,
    South,
    Left,
    Right,
}

impl ShotDirection {
    /// `getValue()`.
    pub fn value(self) -> &'static str {
        match self {
            ShotDirection::North => "North",
            ShotDirection::South => "South",
            ShotDirection::Left => "Left",
            ShotDirection::Right => "Right",
        }
    }
}

/// The parts of the weapon `processHit` reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HitWeapon {
    pub ranged: bool,
    /// `isOfWeaponCategory(BLUNT)` / `(UNARMED)`.
    pub blunt: bool,
    pub unarmed: bool,
    pub ammo_type: Option<Rc<RegistryKey>>,
    pub explosive: bool,
    pub physics_object: Option<String>,
}

impl HitWeapon {
    /// The fields of a weapon item.
    pub fn of(sm: &ScriptManager, item: &InventoryItem) -> HitWeapon {
        let w = WeaponStats::of(sm, item);
        let f = FirearmStats::of(sm, item);
        HitWeapon {
            ranged: item.is_ranged(),
            blunt: w.as_ref().is_some_and(|w| w.is_of_category(sm, "BLUNT")),
            unarmed: w.as_ref().is_some_and(|w| w.is_of_category(sm, "UNARMED")),
            ammo_type: f.as_ref().and_then(|f| f.ammo_type.clone()),
            explosive: w.as_ref().is_some_and(|w| w.explosive),
            physics_object: f.and_then(|f| f.physics_object),
        }
    }
}

/// `CombatManager.getBodyPart(hitReaction, shotDirection)`.
pub fn body_part_for(ctx: &dyn SimContext, r: HitReaction, d: ShotDirection) -> RagdollBodyPart {
    use HitReaction as H;
    use RagdollBodyPart as P;
    match r {
        H::ShotHeadFwd | H::ShotHeadFwd02 | H::ShotHeadBwd => P::Head,
        H::ShotChest | H::ShotChestL | H::ShotChestR | H::ShotChestStepL | H::ShotChestStepR => P::Spine,
        H::ShotBelly | H::ShotBellyStep | H::ShotBellyStepBehind => P::Pelvis,
        H::ShotLegL | H::ShotLegR => {
            let lower = ctx.rand_next(2) == 0;
            match (d == ShotDirection::Left, lower) {
                (true, true) => P::LeftLowerLeg,
                (true, false) => P::LeftUpperLeg,
                (false, true) => P::RightLowerLeg,
                (false, false) => P::RightUpperLeg,
            }
        }
        H::ShotShoulderL | H::ShotShoulderStepL | H::ShotShoulderR | H::ShotShoulderStepR => {
            let lower = ctx.rand_next(2) == 0;
            match (d == ShotDirection::Left, lower) {
                (true, true) => P::LeftLowerArm,
                (true, false) => P::LeftUpperArm,
                (false, true) => P::RightLowerArm,
                (false, false) => P::RightUpperArm,
            }
        }
        _ => P::Spine,
    }
}

/// `CombatManager.calculateShotDirection(wielder, target)` from the two
/// forward directions.
pub fn calculate_shot_direction(ctx: &dyn SimContext, player_fwd: (f32, f32), zombie_fwd: (f32, f32)) -> ShotDirection {
    let cross = (player_fwd.0 * zombie_fwd.1 - player_fwd.1 * zombie_fwd.0) as f64;
    let sign = if cross >= 0.0 { 1.0 } else { -1.0 };
    let dot = (player_fwd.0 * zombie_fwd.0 + player_fwd.1 * zombie_fwd.1) as f64;
    let mut angle = dot.acos() * sign;
    if angle < 0.0 {
        angle += std::f64::consts::PI * 2.0;
    }
    let deg = angle.to_degrees();
    use ShotDirection as S;
    if deg < 45.0 {
        let r = ctx.rand_next(9);
        return if r > 6 {
            S::Left
        } else if r > 4 {
            S::Right
        } else {
            S::South
        };
    }
    if deg < 90.0 {
        return if ctx.rand_next(4) == 0 { S::South } else { S::Right };
    }
    if deg < 135.0 {
        return S::Right;
    }
    if deg < 180.0 {
        return if ctx.rand_next(4) == 0 { S::North } else { S::Right };
    }
    if deg < 225.0 {
        let r = ctx.rand_next(9);
        return if r > 6 {
            S::Left
        } else if r > 4 {
            S::Right
        } else {
            S::North
        };
    }
    if deg < 270.0 {
        return if ctx.rand_next(4) == 0 { S::North } else { S::Left };
    }
    if deg < 315.0 {
        return S::Left;
    }
    if ctx.rand_next(4) == 0 {
        S::South
    } else {
        S::Left
    }
}

/// `CombatManager.resolveHitReaction(wielder, target, shotDirection)`.
pub fn resolve_hit_reaction(ctx: &dyn SimContext, critical: bool, hit_from_behind: bool, d: ShotDirection) -> HitReaction {
    use HitReaction as H;
    let left = d == ShotDirection::Left;
    let lr = |l: H, r: H| if left { l } else { r };
    match d {
        ShotDirection::North => {
            if critical {
                return H::ShotHeadBwd;
            }
            if hit_from_behind {
                H::ShotBellyStepBehind
            } else if ctx.rand_next(2) == 0 {
                H::ShotBelly
            } else {
                H::ShotBellyStep
            }
        }
        ShotDirection::South => {
            if critical {
                return if ctx.rand_next(2) == 0 { H::ShotHeadFwd } else { H::ShotHeadFwd02 };
            }
            if hit_from_behind {
                H::ShotBellyStepBehind
            } else if ctx.rand_next(2) == 0 {
                H::ShotBelly
            } else {
                H::ShotBellyStep
            }
        }
        ShotDirection::Left | ShotDirection::Right => {
            if critical && ctx.rand_next(4) == 0 {
                return H::ShotHeadBwd;
            }
            if hit_from_behind {
                return match ctx.rand_next(3) {
                    0 => lr(H::ShotChestL, H::ShotChestR),
                    1 => lr(H::ShotLegL, H::ShotLegR),
                    _ => lr(H::ShotShoulderStepL, H::ShotShoulderStepR),
                };
            }
            match ctx.rand_next(5) {
                0 => lr(H::ShotChestL, H::ShotChestR),
                1 => lr(H::ShotChestStepL, H::ShotChestStepR),
                2 => lr(H::ShotLegL, H::ShotLegR),
                3 => lr(H::ShotShoulderL, H::ShotShoulderR),
                _ => lr(H::ShotShoulderStepL, H::ShotShoulderStepR),
            }
        }
    }
}

impl<I: 'static> GameCharacter<I> {
    /// `canRagdoll()`: [`Self::can_ragdoll_uncapped`] and room for one more
    /// ragdoll simulation when the character has no controller yet.
    pub fn can_ragdoll(&self, ctx: &dyn SimContext) -> bool {
        if !self.ragdoll.has_controller && ctx.active_ragdoll_simulations() >= ctx.max_active_ragdolls() {
            return false;
        }
        self.can_ragdoll_uncapped(ctx)
    }

    /// `RagdollSettingsManager.usePhysicHitReaction(chr)` with no forced
    /// reaction: the sandbox frequency (100) against `Rand.Next(0f, 100f)`.
    fn ragdoll_settings_use_physic_hit_reaction(ctx: &dyn SimContext) -> bool {
        if ctx.option_use_physics_hit_reaction() {
            let frequency = 100.0f32;
            let chance = ctx.rand_next_float(0.0, 100.0);
            return frequency > 0.0 && chance <= frequency;
        }
        false
    }

    /// `CombatManager.setUsePhysicHitReaction(weapon, chr)`.
    pub fn set_use_physic_hit_reaction_for(&mut self, ctx: &dyn SimContext, ranged: bool) {
        if !self.can_ragdoll(ctx) {
            self.vars.use_physic_hit_reaction = false;
            return;
        }
        self.vars.use_physic_hit_reaction = Self::ragdoll_settings_use_physic_hit_reaction(ctx) && ranged;
    }

    /// `CombatManager.applyBlood(weapon, target, hitReaction, direction)`
    /// on `self` (the target).
    fn apply_blood(&self, ctx: &dyn SimContext, w: &HitWeapon, r: HitReaction, d: ShotDirection) {
        if ctx.is_client() && self.is_player() {
            return;
        }
        use HitReaction as H;
        let crit = self.vars.critical_hit;
        let blood = |part: usize, scratched: bool, bitten: bool| {
            ctx.event(SimEvent::CharacterAddBlood { chr: self.id, part: part as i32, scratched, bitten, all_layers: true });
        };
        let left = d == ShotDirection::Left;
        match r {
            H::ShotHeadFwd | H::ShotHeadFwd02 | H::ShotHeadBwd => blood(blood_part::HEAD, false, true),
            H::ShotChest | H::ShotChestL | H::ShotChestR | H::ShotChestStepL | H::ShotChestStepR => {
                blood(blood_part::TORSO_UPPER, !crit, crit)
            }
            H::ShotBelly | H::ShotBellyStep => blood(blood_part::TORSO_LOWER, !crit, crit),
            H::ShotLegL | H::ShotLegR => {
                let lower = ctx.rand_next(2) == 0;
                let part = match (left, lower) {
                    (true, true) => blood_part::LOWER_LEG_L,
                    (true, false) => blood_part::UPPER_LEG_L,
                    (false, true) => blood_part::LOWER_LEG_R,
                    (false, false) => blood_part::UPPER_LEG_R,
                };
                blood(part, !crit, crit);
            }
            H::ShotShoulderL | H::ShotShoulderStepL | H::ShotShoulderR | H::ShotShoulderStepR => {
                let lower = ctx.rand_next(2) == 0;
                let part = match (left, lower) {
                    (true, true) => blood_part::FORE_ARM_L,
                    (true, false) => blood_part::UPPER_ARM_L,
                    (false, true) => blood_part::FORE_ARM_R,
                    (false, false) => blood_part::UPPER_ARM_R,
                };
                blood(part, !crit, crit);
            }
            _ => {
                if w.blunt {
                    let p = ctx.rand_next_range(blood_part::UPPER_ARM_L as i32, blood_part::GROIN as i32);
                    blood(p as usize, false, false);
                } else if !w.unarmed {
                    let p = ctx.rand_next_range(blood_part::UPPER_ARM_L as i32, blood_part::GROIN as i32);
                    blood(p as usize, false, true);
                }
            }
        }
    }

    /// `CombatManager.createCombatData(weapon, attacker, target, part)` on
    /// `self` (the target).
    fn create_combat_data(&mut self, ctx: &dyn SimContext, w: &HitWeapon, attacker: (i32, [f32; 3]), part: RagdollBodyPart) {
        if !self.vars.use_physic_hit_reaction || !self.can_ragdoll(ctx) {
            return;
        }
        let event = self.vars.hit_reaction.clone();
        let id = self.id;
        if let Some(t) = self.ballistics_target.as_mut() {
            t.combat_damage_data = CombatDamageData {
                event,
                target: id,
                attacker: attacker.0,
                body_part: part,
                weapon_ammo_type: w.ammo_type.clone(),
                weapon_explosive: w.explosive,
                weapon_physics_object: w.physics_object.clone(),
                attacker_position: attacker.1,
            };
            t.combat_damage_data_processed = false;
        }
    }

    /// `eatBodyTarget != null` for a zombie and the `onknees` variable.
    fn eating_reaction(&self) -> Option<HitReaction> {
        if self.is_zombie() && self.zombie.eat_body_target.is_some() {
            return Some(if self.get_variable_boolean("onknees") { HitReaction::OnKnees } else { HitReaction::Eating });
        }
        None
    }

    /// `CombatManager.processTargetedHit(...)` on `self` (the target).
    #[allow(clippy::too_many_arguments)]
    fn process_targeted_hit(
        &mut self,
        ctx: &dyn SimContext,
        w: &HitWeapon,
        attacker: (i32, [f32; 3]),
        part: RagdollBodyPart,
        d: ShotDirection,
    ) {
        use HitReaction as H;
        use RagdollBodyPart as P;
        let random = ctx.rand_next(2);
        let mut r = match part {
            P::Pelvis => {
                let mut r = H::ShotBellyStep;
                if d == ShotDirection::North {
                    if self.vars.hit_from_behind {
                        r = H::ShotBellyStepBehind;
                    } else if random == 0 {
                        r = H::ShotBelly;
                    }
                }
                r
            }
            P::Spine => match d {
                ShotDirection::Left => {
                    if random == 0 {
                        H::ShotChestL
                    } else {
                        H::ShotChestStepL
                    }
                }
                ShotDirection::Right => {
                    if random == 0 {
                        H::ShotChestR
                    } else {
                        H::ShotChestStepR
                    }
                }
                _ => H::ShotChest,
            },
            P::Head => {
                let mut r = if random == 0 { H::ShotHeadFwd } else { H::ShotHeadFwd02 };
                if (d == ShotDirection::Left || d == ShotDirection::Right) && ctx.rand_next(4) == 0 {
                    r = H::ShotHeadBwd;
                }
                r
            }
            P::LeftUpperLeg | P::LeftLowerLeg => H::ShotLegL,
            P::RightUpperLeg | P::RightLowerLeg => H::ShotLegR,
            P::LeftUpperArm | P::LeftLowerArm => {
                if self.vars.hit_from_behind {
                    H::ShotShoulderStepL
                } else {
                    H::ShotShoulderL
                }
            }
            P::RightUpperArm | P::RightLowerArm => {
                if self.vars.hit_from_behind {
                    H::ShotShoulderStepR
                } else {
                    H::ShotShoulderR
                }
            }
            // The original throws IllegalStateException.
            P::Count => H::None,
        };
        self.apply_blood(ctx, w, r, d);
        if let Some(e) = self.eating_reaction() {
            r = e;
        }
        self.vars.hit_reaction = r.value().to_string();
        self.create_combat_data(ctx, w, attacker, part);
    }

    /// `CombatManager.processHit(weapon, wielder, target)` on `self` (the
    /// target): the hit reaction, blood and ragdoll data of a hit; returns
    /// the `RagdollBodyPart` ordinal the hit lands on.
    pub fn process_hit(&mut self, w: &HitWeapon, wielder: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) -> i32 {
        let (chars, ctx) = (cx.chars, cx.ctx);
        let mut r = HitReaction::from_string(&wielder.get_variable_string("ZombieHitReaction"));
        self.set_use_physic_hit_reaction_for(ctx, w.ranged);
        let d = calculate_shot_direction(ctx, (wielder.vars.forward_x, wielder.vars.forward_y), (self.vars.forward_x, self.vars.forward_y));
        let attacker = (wielder.id, [wielder.vars.x, wielder.vars.y, wielder.vars.z]);
        if r == HitReaction::Shot {
            let chance = wielder.calculate_crit_chance_in(cx.sm, chars, ctx, Some(&*cx.world), self);
            wielder.vars.critical_hit = ctx.rand_next(100) < chance;
            let targeted = match wielder.ballistics_controller.as_ref() {
                Some(c) if c.is_camera_target(self.id) => c.cached_targeted_body_part(self.id),
                _ => RagdollBodyPart::Count.ordinal(),
            };
            if targeted != RagdollBodyPart::Count.ordinal() {
                self.process_targeted_hit(ctx, w, attacker, RagdollBodyPart::from_ordinal(targeted), d);
                return targeted;
            }
            r = resolve_hit_reaction(ctx, wielder.vars.critical_hit, self.vars.hit_from_behind, d);
        }
        self.apply_blood(ctx, w, r, d);
        if let Some(e) = self.eating_reaction() {
            r = e;
        }
        if r == HitReaction::Floor && self.is_current_state_id(ids::ZOMBIE_GET_UP) && self.environment.fall_on_front {
            r = HitReaction::GettingUpFront;
        }
        // `isAnimalRunningToDeathPosition()`: animals are not ported.
        if r != HitReaction::None {
            self.vars.hit_reaction = r.value().to_string();
        } else {
            if self.is_zombie() {
                self.vars.stagger_back = true;
            }
            self.vars.hit_reaction = String::new();
        }
        let part = body_part_for(ctx, r, d);
        self.create_combat_data(ctx, w, attacker, part);
        part.ordinal()
    }
}
