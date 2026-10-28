//! Combat: `zombie.CombatManager`'s melee resolution, `CombatConfig`,
//! `WeaponType`, the `HandWeapon` combat getters and the
//! `IsoGameCharacter` / `IsoLivingCharacter` / `IsoPlayer` / `IsoZombie`
//! hit helpers (`Hit`, `processHitDamage`, `hitConsequences`, `Kill`,
//! `calculateCritChance`, `testDotSide`, `isBehind`).
//!
//! A hit involves two characters; the target is `self` and the wielder is
//! passed separately with a copy of the weapon item (the weapon lives in
//! the wielder's inventory).
//!
//! Ranged combat (ballistics, `calculateHitChanceData`, hit reactions by
//! shot direction) is in [`super::firearm`].

use pz_anim::AnimationVariableSource;
use pz_inventory::InventoryItem;
use pz_scripts::objects::items::{Item, WeaponCategory};
use pz_scripts::{ScriptManager, ScriptResult};

use super::body_damage::WeaponUser;
use super::items::{is_hand_weapon, script_item};
use super::state::StateCx;
use super::{CharacterKind, GameCharacter, SimContext, SimEvent};
use crate::Characters;

/// `CombatConfigKey` defaults (`CombatConfig` with nothing overridden).
pub mod config {
    pub const BASE_WEAPON_DAMAGE_MULTIPLIER: f32 = 0.3;
    pub const WEAPON_LEVEL_DAMAGE_MULTIPLIER_INCREMENT: f32 = 0.1;
    pub const PLAYER_RECEIVED_DAMAGE_MULTIPLIER: f32 = 0.4;
    pub const NON_PLAYER_RECEIVED_DAMAGE_MULTIPLIER: f32 = 1.5;
    pub const HEAD_HIT_DAMAGE_SPLIT_MODIFIER: f32 = 3.0;
    pub const LEG_HIT_DAMAGE_SPLIT_MODIFIER: f32 = 0.05;
    pub const ADDITIONAL_CRITICAL_HIT_CHANCE_FROM_BEHIND: f32 = 30.0;
    pub const ADDITIONAL_CRITICAL_HIT_CHANCE_DEFAULT: f32 = 5.0;
    pub const RECOIL_DELAY: f32 = 10.0;
    pub const GLOBAL_MELEE_DAMAGE_REDUCTION_MULTIPLIER: f32 = 0.15;
    pub const DAMAGE_PENALTY_ONE_HANDED_TWO_HANDED_WEAPON_MULTIPLIER: f32 = 0.5;
    pub const ENDURANCE_LOSS_TWO_HANDED_PENALTY_DIVISOR: f32 = 1.5;
    pub const ENDURANCE_LOSS_TWO_HANDED_PENALTY_SCALE: f32 = 10.0;
    pub const ENDURANCE_LOSS_FLOOR_SHOVE_MULTIPLIER: f32 = 2.0;
    pub const ENDURANCE_LOSS_CLOSE_KILL_MODIFIER: f32 = 0.2;
    pub const ENDURANCE_LOSS_BASE_SCALE: f32 = 0.28;
    pub const ENDURANCE_LOSS_WEIGHT_MODIFIER: f32 = 0.3;
    pub const ENDURANCE_LOSS_FINAL_MULTIPLIER: f32 = 0.04;
    // ---- FIREARM ----
    pub const POINT_BLANK_DISTANCE: f32 = 3.5;
    pub const LOW_LIGHT_THRESHOLD: f32 = 0.75;
    pub const LOW_LIGHT_TO_HIT_MAXIMUM_PENALTY: f32 = 50.0;
    pub const POINT_BLANK_TO_HIT_MAXIMUM_BONUS: f32 = 40.0;
    pub const POINT_BLANK_DROP_OFF_TO_HIT_PENALTY: f32 = 0.7;
    pub const POST_SHOT_AIMING_DELAY_RECOIL_MODIFIER: f32 = 0.25;
    pub const POST_SHOT_AIMING_DELAY_AIMING_MODIFIER: f32 = 0.05;
    pub const OPTIMAL_RANGE_TO_HIT_MAXIMUM_BONUS: f32 = 15.0;
    pub const OPTIMAL_RANGE_DROP_OFF_TO_HIT_PENALTY: f32 = 4.0;
    pub const OPTIMAL_RANGE_DROP_OFF_TO_HIT_PENALTY_INCREMENT: f32 = 0.3;
    pub const MINIMUM_TO_HIT_CHANCE: f32 = 5.0;
    pub const MAXIMUM_START_TO_HIT_CHANCE: f32 = 95.0;
    pub const MAXIMUM_TO_HIT_CHANCE: f32 = 100.0;
    pub const MOVING_TO_HIT_PENALTY: f32 = 5.0;
    pub const RUNNING_TO_HIT_PENALTY: f32 = 15.0;
    pub const SPRINTING_TO_HIT_PENALTY: f32 = 25.0;
    pub const MARKSMAN_TRAIT_TO_HIT_BONUS: f32 = 20.0;
    pub const ARM_PAIN_TO_HIT_MODIFIER: f32 = 0.1;
    pub const PANIC_TO_HIT_BASE_PENALTY: f32 = 4.0;
    pub const PANIC_TO_HIT_DISTANCE_MODIFIER: f32 = 0.5;
    pub const STRESS_TO_HIT_BASE_PENALTY: f32 = 4.0;
    pub const STRESS_TO_HIT_DISTANCE_MODIFIER: f32 = 0.5;
    pub const TIRED_TO_HIT_BASE_PENALTY: f32 = 2.5;
    pub const ENDURANCE_TO_HIT_BASE_PENALTY: f32 = 2.5;
    pub const DRUNK_TO_HIT_BASE_PENALTY: f32 = 4.0;
    pub const DRUNK_TO_HIT_DISTANCE_MODIFIER: f32 = 0.5;
    pub const WIND_INTENSITY_TO_HIT_PENALTY: f32 = 6.0;
    pub const WIND_INTENSITY_TO_HIT_AIMING_MODIFIER: f32 = 0.2;
    pub const WIND_INTENSITY_TO_HIT_MINIMUM_MARKSMAN_MODIFIER: f32 = 0.6;
    pub const WIND_INTENSITY_TO_HIT_MAXIMUM_MARKSMAN_MODIFIER: f32 = 1.0;
    pub const RAIN_INTENSITY_TO_HIT_DISTANCE_MODIFIER: f32 = 0.5;
    pub const FOG_INTENSITY_DISTANCE_MODIFIER: f32 = 10.0;
    pub const POINT_BLANK_MAXIMUM_DISTANCE_MODIFIER: f32 = 1.0;
    pub const SIGHTLESS_TO_HIT_BASE_DISTANCE: f32 = 15.0;
    pub const SIGHTLESS_TO_HIT_PRONE_MODIFIER: f32 = 2.0;
    pub const SIGHTLESS_AIM_DELAY_TO_HIT_DISTANCE_MODIFIER: f32 = 0.1;
    pub const PIERCING_BULLET_DAMAGE_REDUCTION: f32 = 5.0;
    pub const FIREARM_RECOIL_MUSCLE_STRAIN_MODIFIER: f32 = 0.05;
    pub const DRIVEBY_DOT_OPTIMAL_ANGLE: f32 = -0.7;
    pub const DRIVEBY_DOT_MAXIMUM_ANGLE: f32 = -0.1;
    pub const DRIVEBY_DOT_TO_HIT_MAXIMUM_PENALTY: f32 = 40.0;
    // ---- BALLISTICS ----
    pub const BALLISTICS_CONTROLLER_DISTANCE_THRESHOLD: f32 = 2.75;
}

/// `zombie.inventory.types.WeaponType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeaponType {
    Unarmed,
    TwoHanded,
    OneHanded,
    Heavy,
    Knife,
    Spear,
    Handgun,
    Firearm,
    Throwing,
    Chainsaw,
}

impl WeaponType {
    /// `getType()`.
    pub fn type_name(self) -> &'static str {
        match self {
            WeaponType::Unarmed => "",
            WeaponType::TwoHanded => "2handed",
            WeaponType::OneHanded => "1handed",
            WeaponType::Heavy => "heavy",
            WeaponType::Knife => "knife",
            WeaponType::Spear => "spear",
            WeaponType::Handgun => "handgun",
            WeaponType::Firearm => "firearm",
            WeaponType::Throwing => "throwing",
            WeaponType::Chainsaw => "chainsaw",
        }
    }

    /// `isRanged()`.
    pub fn is_ranged(self) -> bool {
        matches!(self, WeaponType::Handgun | WeaponType::Firearm | WeaponType::Throwing)
    }

    /// `WeaponType.getWeaponType(HandWeapon)`.
    pub fn of_weapon(w: &WeaponStats) -> WeaponType {
        let swing = w.swing_anim.as_str();
        if swing.eq_ignore_ascii_case("Stab") {
            return WeaponType::Knife;
        }
        if swing.eq_ignore_ascii_case("Heavy") {
            return WeaponType::Heavy;
        }
        if swing.eq_ignore_ascii_case("Throw") {
            return WeaponType::Throwing;
        }
        if w.ranged {
            return if w.two_hand { WeaponType::Firearm } else { WeaponType::Handgun };
        }
        if w.two_hand {
            if swing.eq_ignore_ascii_case("Spear") {
                return WeaponType::Spear;
            }
            if w.item_type == "Chainsaw" {
                return WeaponType::Chainsaw;
            }
            return WeaponType::TwoHanded;
        }
        WeaponType::OneHanded
    }
}

/// The `HandWeapon` fields combat reads: the instance's (damage,
/// condition) and the script's (the rest, which `InstanceItem` copies).
/// Sharpness and weapon parts are not applied. TODO(weapon parts).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeaponStats {
    /// `InventoryItem.id` (identity of the hand items).
    pub id: i32,
    /// `getType()` (the script name).
    pub item_type: String,
    pub min_damage: f32,
    pub max_damage: f32,
    pub critical_chance: f32,
    pub crit_dmg_multiplier: f32,
    pub knockdown_mod: f32,
    pub push_back_mod: f32,
    pub condition_lower_chance: i32,
    pub condition: i32,
    pub endurance_mod: f32,
    pub two_hand: bool,
    pub knock_back_on_no_death: bool,
    pub splat_blood_on_no_death: bool,
    pub use_endurance: bool,
    pub use_self: bool,
    pub always_knockdown: bool,
    pub swing_anim: String,
    /// `getWeight()`.
    pub weight: f32,
    pub max_range: f32,
    pub min_range: f32,
    pub hit_angle_mod: f32,
    pub ranged: bool,
    pub aimed_firearm: bool,
    /// The weapon category ids (`weaponCategories`).
    pub categories: Vec<String>,
    /// `"Explosives".equals(displayCategory)`.
    pub explosive: bool,
    /// `isBareHands()` (by type; TODO(item tags): the `BARE_HANDS` tag).
    pub bare_hands: bool,
    pub max_hit_count: i32,
    pub min_angle: f32,
    pub max_angle: f32,
}

impl WeaponStats {
    /// The stats of a hand weapon item (its script looked up in `sm`).
    pub fn of(sm: &ScriptManager, item: &InventoryItem) -> Option<WeaponStats> {
        if !is_hand_weapon(item) {
            return None;
        }
        let script = script_item(sm, item);
        let guard = script.as_ref().map(|s| s.borrow());
        let s: Option<&Item> = guard.as_deref();
        let get = |f: fn(&Item) -> f32, d: f32| s.map_or(d, f);
        let item_type = item.type_.to_string();
        Some(WeaponStats {
            id: item.id,
            bare_hands: item_type == "BareHands",
            item_type,
            min_damage: item.get_min_damage(),
            max_damage: item.get_max_damage(),
            critical_chance: get(|s| s.critical_chance, 20.0),
            crit_dmg_multiplier: get(|s| s.crit_dmg_multiplier, 0.0),
            knockdown_mod: get(|s| s.knockdown_mod, 1.0),
            push_back_mod: get(|s| s.push_back_mod, 1.0),
            condition_lower_chance: s.map_or(10, |s| s.condition_lower_chance),
            condition: item.condition,
            endurance_mod: get(|s| s.endurance_mod, 1.0),
            two_hand: s.is_some_and(|s| s.two_hand_weapon),
            knock_back_on_no_death: s.is_none_or(|s| s.knock_back_on_no_death),
            splat_blood_on_no_death: s.is_some_and(|s| s.splat_blood_on_no_death),
            use_endurance: s.is_none_or(|s| s.use_endurance),
            use_self: s.is_some_and(|s| s.use_self),
            always_knockdown: s.is_some_and(|s| s.always_knockdown),
            swing_anim: s.and_then(|s| s.swing_anim.clone()).unwrap_or_default(),
            weight: get(|s| s.actual_weight, 1.0).max(0.0),
            max_range: get(|s| s.max_range, 1.0),
            min_range: get(|s| s.min_range, 0.0),
            // `Math.toRadians(script.hitAngleMod)` (`Item.InstanceItem`).
            hit_angle_mod: (get(|s| s.hit_angle_mod, 0.0) as f64).to_radians() as f32,
            ranged: item.is_ranged(),
            aimed_firearm: item.is_aimed_firearm(),
            categories: item.get_weapon_categories().to_vec(),
            explosive: s.and_then(|s| s.display_category.as_deref()) == Some("Explosives"),
            max_hit_count: s.map_or(1, |s| s.max_hit_count),
            min_angle: item.weapon_data().map_or(0.0, |d| d.min_angle),
            max_angle: item.weapon_data().map_or(1.0, |d| d.max_angle),
        })
    }

    /// `isMelee()`: any weapon category.
    pub fn is_melee(&self) -> bool {
        !self.categories.is_empty()
    }

    /// `isOfWeaponCategory(WeaponCategory.<field>)`.
    pub fn is_of_category(&self, sm: &ScriptManager, field: &str) -> bool {
        WeaponCategory::constant(sm, field)
            .is_some_and(|k| self.categories.iter().any(|c| c.eq_ignore_ascii_case(&k.location.to_string())))
    }

    /// `getEffectiveWeight()`.
    pub fn effective_weight(&self) -> f32 {
        self.weight.max(1.0)
    }

    /// `getCriticalDamageMultiplier()`.
    pub fn critical_damage_multiplier(&self) -> f32 {
        self.crit_dmg_multiplier
    }
}

/// The perk-level table `getHittingMod` / `getShovingMod` share.
fn strength_mod(level: i32) -> f32 {
    match level {
        1 => 0.8,
        2 => 0.85,
        3 => 0.9,
        4 => 0.95,
        5 => 1.0,
        6 => 1.05,
        7 => 1.1,
        8 => 1.15,
        9 => 1.2,
        10 => 1.25,
        _ => 0.75,
    }
}

/// `IsoGameCharacter.Side` (`testDotSide`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Front,
    Behind,
    Left,
    Right,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Front => "FRONT",
            Side::Behind => "BEHIND",
            Side::Left => "LEFT",
            Side::Right => "RIGHT",
        }
    }
}

/// `Vector2.normalize()`.
fn normalize(x: f32, y: f32) -> (f32, f32) {
    super::zombie::math::normalize(x, y)
}

/// `Vector2.rotate(rad)`.
fn rotate(x: f32, y: f32, rad: f32) -> (f32, f32) {
    let c = (rad as f64).cos();
    let s = (rad as f64).sin();
    ((x as f64 * c - y as f64 * s) as f32, (x as f64 * s + y as f64 * c) as f32)
}

/// `IsoMovingObject.getVectorFromDirection(dir)`: normalized (diagonals
/// are `(±1, ±1) / sqrt 2` after `Vector2.normalize`).
pub fn direction_vector(dir: pz_world::square::IsoDirections) -> (f32, f32) {
    use pz_world::square::IsoDirections as D;
    let v = match dir {
        D::S => (0.0, 1.0),
        D::N => (0.0, -1.0),
        D::E => (1.0, 0.0),
        D::W => (-1.0, 0.0),
        D::NW => (-1.0, -1.0),
        D::NE => (1.0, -1.0),
        D::SW => (-1.0, 1.0),
        D::SE => (1.0, 1.0),
    };
    normalize(v.0, v.1)
}

/// `CombatManager.applyGlobalDamageReductionMultipliers`.
pub fn apply_global_damage_reduction_multipliers(w: &WeaponStats, damage: f32) -> f32 {
    if w.is_melee() {
        damage * config::GLOBAL_MELEE_DAMAGE_REDUCTION_MULTIPLIER
    } else {
        damage
    }
}

/// `CombatManager.applyWeaponLevelDamageModifier`.
pub fn apply_weapon_level_damage_modifier(weapon_level: i32, damage: f32) -> f32 {
    let level = weapon_level.clamp(0, 10);
    damage * (config::BASE_WEAPON_DAMAGE_MULTIPLIER + level as f32 * config::WEAPON_LEVEL_DAMAGE_MULTIPLIER_INCREMENT)
}

/// `CombatManager.applyPlayerReceivedDamageModifier`.
pub fn apply_player_received_damage_modifier(target_is_player: bool, damage: f32) -> f32 {
    damage
        * if target_is_player {
            config::PLAYER_RECEIVED_DAMAGE_MULTIPLIER
        } else {
            config::NON_PLAYER_RECEIVED_DAMAGE_MULTIPLIER
        }
}

/// `CombatManager.applyOneHandedDamagePenalty`.
pub fn apply_one_handed_damage_penalty(w: &WeaponStats, in_both_hands: bool, damage: f32) -> f32 {
    let wrong = w.two_hand && !in_both_hands;
    damage * if wrong { config::DAMAGE_PENALTY_ONE_HANDED_TWO_HANDED_WEAPON_MULTIPLIER } else { 1.0 }
}

impl<I: 'static> GameCharacter<I> {
    fn perk(&self, chars: &Characters, field: &str) -> i32 {
        self.perk_level(chars.perks.constant(field).as_ref())
    }

    fn moodle_lvl(&self, sm: &ScriptManager, field: &str) -> i32 {
        self.moodles.moodle_level_of(sm, field).unwrap_or(0)
    }

    /// `getHittingMod()`.
    pub fn hitting_mod(&self, chars: &Characters) -> f32 {
        strength_mod(self.perk(chars, "Strength"))
    }

    /// `getShovingMod()`.
    pub fn shoving_mod(&self, chars: &Characters) -> f32 {
        strength_mod(self.perk(chars, "Strength"))
    }

    /// `getFatigueMod()`.
    pub fn fatigue_mod(&self, chars: &Characters) -> f32 {
        match self.perk(chars, "Fitness") {
            1 => 0.95,
            2 => 0.92,
            3 => 0.89,
            4 => 0.87,
            5 => 0.85,
            6 => 0.83,
            7 => 0.81,
            8 => 0.79,
            9 => 0.77,
            10 => 0.75,
            _ => 1.0,
        }
    }

    /// `isItemInBothHands(item)`.
    pub fn is_item_in_both_hands(&self, id: i32) -> bool {
        self.items.primary_hand_item == Some(id) && self.items.secondary_hand_item == Some(id)
    }

    /// `WeaponType.getWeaponType(chr)` (sets `rangedWeapon` like the
    /// original).
    pub fn weapon_type(&mut self, sm: &ScriptManager) -> WeaponType {
        let t = self.weapon_type_of(sm);
        self.set_variable_bool("rangedWeapon", matches!(t, WeaponType::Handgun | WeaponType::Firearm | WeaponType::Throwing));
        t
    }

    /// `WeaponType.getWeaponType(chr)` without the variable side effect.
    pub fn weapon_type_of(&self, sm: &ScriptManager) -> WeaponType {
        let Some(item) = self.items.primary_hand_item() else { return WeaponType::Unarmed };
        let item = &*item.borrow();
        let Some(w) = WeaponStats::of(sm, item) else { return WeaponType::Unarmed };
        let swing = w.swing_anim.as_str();
        if swing.eq_ignore_ascii_case("Stab") {
            return WeaponType::Knife;
        }
        if swing.eq_ignore_ascii_case("Heavy") {
            return WeaponType::Heavy;
        }
        if swing.eq_ignore_ascii_case("Throw") {
            return WeaponType::Throwing;
        }
        let both = self.is_item_in_both_hands(w.id);
        if !w.ranged {
            if both && w.two_hand {
                if swing.eq_ignore_ascii_case("Spear") {
                    return WeaponType::Spear;
                }
                if w.item_type == "Chainsaw" {
                    return WeaponType::Chainsaw;
                }
                return WeaponType::TwoHanded;
            }
            return WeaponType::OneHanded;
        }
        if both && w.two_hand {
            WeaponType::Firearm
        } else {
            WeaponType::Handgun
        }
    }

    /// `getWeaponLevel()`.
    pub fn weapon_level(&self, sm: &ScriptManager, chars: &Characters) -> i32 {
        let wt = self.weapon_type_of(sm);
        let Some(w) = self.items.primary_hand_item().and_then(|i| WeaponStats::of(sm, &i.borrow())) else { return 0 };
        let mut level = -1;
        if wt != WeaponType::Unarmed {
            if w.is_of_category(sm, "AXE") {
                level = self.perk(chars, "Axe");
            }
            for (cat, perk) in [("SPEAR", "Spear"), ("SMALL_BLADE", "SmallBlade"), ("LONG_BLADE", "LongBlade"), ("BLUNT", "Blunt"), ("SMALL_BLUNT", "SmallBlunt")] {
                if w.is_of_category(sm, cat) {
                    level += self.perk(chars, perk);
                }
            }
        }
        if level > 10 {
            level = 10;
        }
        if level == -1 {
            0
        } else {
            level
        }
    }

    /// `HandWeapon.getDamageMod(chr)`.
    pub fn weapon_damage_mod(&self, sm: &ScriptManager, chars: &Characters, w: &WeaponStats) -> f32 {
        for (cat, perk) in [("BLUNT", "Blunt"), ("AXE", "Axe"), ("SPEAR", "Spear")] {
            let level = self.perk(chars, perk);
            if w.is_of_category(sm, cat) {
                if (3..=6).contains(&level) {
                    return 1.1;
                }
                if level >= 7 {
                    return 1.2;
                }
            }
        }
        1.0
    }

    /// `HandWeapon.getFatigueMod(chr)`.
    pub fn weapon_fatigue_mod(&self, sm: &ScriptManager, chars: &Characters, w: &WeaponStats) -> f32 {
        for (cat, perk) in [("BLUNT", "Blunt"), ("AXE", "Axe"), ("SPEAR", "Spear")] {
            if w.is_of_category(sm, cat) && self.perk(chars, perk) >= 8 {
                return 0.8;
            }
        }
        1.0
    }

    /// `HandWeapon.getKnockbackMod(chr)`.
    pub fn weapon_knockback_mod(&self, sm: &ScriptManager, chars: &Characters, w: &WeaponStats) -> f32 {
        if w.is_of_category(sm, "AXE") && self.perk(chars, "Axe") >= 6 {
            2.0
        } else {
            1.0
        }
    }

    /// `HandWeapon.getRangeMod(chr)`.
    pub fn weapon_range_mod(&self, sm: &ScriptManager, chars: &Characters, w: &WeaponStats) -> f32 {
        for (cat, perk) in [("BLUNT", "Blunt"), ("AXE", "Axe"), ("SPEAR", "Spear")] {
            if w.is_of_category(sm, cat) && self.perk(chars, perk) >= 7 {
                return 1.2;
            }
        }
        1.0
    }

    /// `HandWeapon.getStaggerBackTimeMod(wielder, target)`; `self` is the
    /// wielder.
    pub fn weapon_stagger_back_time_mod(&self, sm: &ScriptManager, chars: &Characters, w: &WeaponStats, target_is_zombie: bool) -> f32 {
        if target_is_zombie && w.is_of_category(sm, "SMALL_BLADE") {
            return 0.0;
        }
        w.push_back_mod * self.weapon_knockback_mod(sm, chars, w) * self.shoving_mod(chars)
    }

    /// `getLookVector()` (no animation player: the forward direction's
    /// angle).
    pub fn look_vector(&self) -> (f32, f32) {
        let a = self.direction_angle_radians();
        ((a as f64).cos() as f32, (a as f64).sin() as f32)
    }

    /// `testDotSide(target)` with the target at `(tx, ty)`.
    pub fn test_dot_side(&self, tx: f32, ty: f32) -> Side {
        let (lx, ly) = self.look_vector();
        let (px, py) = (self.vars.x, self.vars.y);
        let (dx, dy) = normalize(tx - px, ty - py);
        let dot = dx * lx + dy * ly;
        if dot as f64 > 0.7 {
            return Side::Front;
        }
        if dot < 0.0 && (dot as f64) < -0.5 {
            return Side::Behind;
        }
        let d = (tx - px) * ((py + ly) - py) - (ty - py) * ((px + lx) - px);
        if d > 0.0 {
            Side::Right
        } else {
            Side::Left
        }
    }

    /// `isBehind(chr)`: whether `self` is behind the character at
    /// `(cx, cy)` facing `forward`.
    pub fn is_behind(&self, cx: f32, cy: f32, forward: (f32, f32)) -> bool {
        let (tx, ty) = normalize(cx - self.vars.x, cy - self.vars.y);
        let (fx, fy) = normalize(forward.0, forward.1);
        tx * fx + ty * fy > 0.6
    }

    /// `IsoPlayer.calculateCritChance(target)` without the world (a ranged
    /// weapon's weather penalty reads no square).
    pub fn calculate_crit_chance(&self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, target: &GameCharacter<I>) -> i32 {
        self.calculate_crit_chance_in(sm, chars, ctx, None, target)
    }

    /// `IsoPlayer.calculateCritChance(target)`; the world gives the ranged
    /// weather penalty its square.
    pub fn calculate_crit_chance_in(
        &self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        world: Option<&dyn super::state::StateWorld<I>>,
        target: &GameCharacter<I>,
    ) -> i32 {
        if self.vars.do_shove {
            let mut base = 35.0f32;
            if target.is_player() {
                base = 20.0;
            }
            base -= (self.moodle_lvl(sm, "ENDURANCE") * 5) as f32;
            base -= (self.moodle_lvl(sm, "HEAVY_LOAD") * 5) as f32;
            base -= self.moodle_lvl(sm, "PANIC") as f32 * 1.3;
            base += (self.perk(chars, "Strength") * 2) as f32;
            return base as i32;
        }
        let Some(item) = self.items.primary_hand_item() else { return 0 };
        let item = &*item.borrow();
        let Some(w) = WeaponStats::of(sm, item) else { return 0 };
        let mut base = w.critical_chance;
        if w.always_knockdown {
            return 100;
        }
        if self.weapon_type_of(sm).is_ranged() {
            use super::firearm::hit_chance as hc;
            let f = super::firearm::FirearmStats::of(sm, item).expect("a hand weapon");
            base += (f.aiming_perk_crit_modifier * self.perk(chars, "Aiming")) as f32;
            let dist_sq = super::firearm::ranged::iso_dist_sq3((self.vars.x, self.vars.y, 0.0), (target.vars.x, target.vars.y, 0.0));
            let dist = (dist_sq as f64).sqrt() as f32;
            let max = self.firearm_max_sight_range(sm, chars, &f);
            let min = self.firearm_min_sight_range(chars, &f);
            let delay = 0.0f32.max(self.vars.aiming_delay);
            base += (hc::distance_modifier_sightless(dist, false) - hc::aim_delay_penalty_sightless(delay, dist))
                .max(hc::distance_modifier(dist, min, max, false) - hc::aim_delay_penalty(delay, dist, min, max));
            base -= self.moodles_penalty(sm, ctx, dist);
            let square = world.and_then(|wd| {
                let sq = target.square_xyz();
                wd.square(sq.0, sq.1, sq.2).map(|_| sq)
            });
            base -= self.weather_penalty(sm, chars, ctx, world, &f, square, dist);
            base -= self.move_penalty(chars, dist);
            if self.has_trait(sm, "MARKSMAN").unwrap_or(false) {
                base += 10.0;
            }
        } else {
            if w.two_hand && !self.is_item_in_both_hands(w.id) {
                base -= base / 3.0;
            }
            if self.vars.charge_time < 2.0 {
                base -= base / 5.0;
            }
            let mut skill = self.perk(chars, "Blunt");
            for (cat, perk) in [("AXE", "Axe"), ("LONG_BLADE", "LongBlade"), ("SPEAR", "Spear"), ("SMALL_BLADE", "SmallBlade"), ("SMALL_BLUNT", "SmallBlunt")] {
                if w.is_of_category(sm, cat) {
                    skill = self.perk(chars, perk);
                }
            }
            base += skill as f32 * 3.0;
            base -= (self.moodle_lvl(sm, "ENDURANCE") * 5) as f32;
            base -= (self.moodle_lvl(sm, "HEAVY_LOAD") * 5) as f32;
            base -= self.moodle_lvl(sm, "PANIC") as f32 * 1.3;
        }
        if target.is_zombie() {
            match ctx.lore_toughness() {
                1 => base -= 6.0,
                3 => base += 6.0,
                _ => {}
            }
        }
        base.clamp(10.0, 90.0) as i32
    }

    /// `processHitDamage(weapon, wielder, damageSplit, ignoreDamage,
    /// modDelta)` on `self` (the target): sets `hitForce`, returns the
    /// damage.
    #[allow(clippy::too_many_arguments)]
    pub fn process_hit_damage(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        w: &WeaponStats,
        wielder: &GameCharacter<I>,
        damage_split: f32,
        ignore_damage: bool,
        mod_delta: f32,
    ) -> f32 {
        if w.explosive {
            return damage_split * mod_delta;
        }
        let mut damage = damage_split * mod_delta;
        let mut dmg_test = damage;
        if ignore_damage {
            dmg_test /= 2.7;
        }
        let mut f = dmg_test * wielder.shoving_mod(chars);
        if f > 1.0 {
            f = 1.0;
        }
        self.vars.hit_force = f;
        if wielder.has_trait(sm, "STRONG").unwrap_or(false) && !w.ranged {
            self.vars.hit_force *= 1.4;
        }
        if wielder.has_trait(sm, "WEAK").unwrap_or(false) && !w.ranged {
            self.vars.hit_force *= 0.6;
        }
        let mut del = wielder.stat(chars, "ENDURANCE").unwrap_or(1.0);
        del *= wielder.vars.knockback_attack_mod;
        if del < 0.5 {
            del *= 1.3;
            if del < 0.4 {
                del = 0.4;
            }
            self.vars.hit_force *= del;
        }
        let wielder_player = wielder.is_player();
        if wielder_player && !ignore_damage {
            self.vars.hit_force *= 2.0;
        }
        if wielder_player && !wielder.vars.do_shove && !ignore_damage {
            let (ox, oy) = normalize(self.vars.x - wielder.vars.x, self.vars.y - wielder.vars.y);
            let (dx, dy) = direction_vector(self.vars.dir);
            if ox * dx + oy * dy > -0.3 {
                damage *= 1.5;
            }
        }
        damage = apply_player_received_damage_modifier(self.is_player(), damage);
        damage = apply_weapon_level_damage_modifier(wielder.weapon_level(sm, chars), damage);
        if wielder_player && wielder.vars.aim_at_floor && !ignore_damage && !wielder.vars.do_shove && !w.ranged {
            damage *= 5.0f32.max(w.critical_damage_multiplier());
        }
        if wielder.vars.critical_hit && !ignore_damage {
            damage *= 2.0f32.max(w.critical_damage_multiplier());
        }
        apply_one_handed_damage_penalty(w, wielder.is_item_in_both_hands(w.id), damage)
    }

    /// `calculateHitDirection(weapon, wielder)` (no attack target square).
    pub fn calculate_hit_direction(&mut self, w: &WeaponStats, wielder_x: f32, wielder_y: f32) {
        let (x, y) = normalize(self.vars.x - wielder_x, self.vars.y - wielder_y);
        let (x, y) = (x * w.push_back_mod, y * w.push_back_mod);
        let (x, y) = rotate(x, y, w.hit_angle_mod);
        self.vars.hit_dir_x = x;
        self.vars.hit_dir_y = y;
    }

    /// `Hit(weapon, wielder, damageSplit, ignoreDamage, modDelta)` with
    /// the `IsoZombie` / `IsoPlayer` overrides of `Hit` and
    /// `hitConsequences`. Returns the damage.
    pub fn hit(
        &mut self,
        weapon_item: &InventoryItem,
        wielder: &mut GameCharacter<I>,
        mut damage_split: f32,
        mut ignore_damage: bool,
        mut mod_delta: f32,
        cx: &mut StateCx<'_, I>,
    ) -> ScriptResult<f32> {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        let Some(w) = WeaponStats::of(sm, weapon_item) else { return Ok(0.0) };
        if self.is_zombie() {
            let types = &chars.sim.body_part_types;
            let torso_upper = types.index("Torso_Upper")? as i32;
            let torso_lower = types.index("Torso_Lower")? as i32;
            let mut part = ctx.rand_next_range(torso_upper, torso_lower + 1);
            if super::body_part::next_bool(ctx, 7) {
                part = types.index("Head")? as i32;
            }
            if wielder.vars.critical_hit && super::body_part::next_bool(ctx, 3) {
                part = types.index("Head")? as i32;
            }
            self.vars.last_hit_part = part;
            wielder.vars.last_hit_character = Some(self.id);
        }
        if w.is_melee() && !ignore_damage && self.is_zombie() {
            self.vars.hit_time += 1;
            if self.vars.hit_time >= 4 {
                damage_split *= (self.vars.hit_time as f32 - 2.0) * 1.5;
            }
        }
        if wielder.is_player() && wielder.vars.do_shove && !wielder.vars.aim_at_floor {
            ignore_damage = true;
            mod_delta *= 1.5;
        }
        ctx.event(SimEvent::WeaponHitCharacter { wielder: wielder.id, target: self.id, damage: damage_split });
        ctx.event(SimEvent::PlayerGetDamage { kind: "WEAPONHIT", amount: damage_split });
        if ctx.trigger_hook("WeaponHitCharacter") {
            return Ok(0.0);
        }
        if self.vars.avoid_damage {
            self.vars.avoid_damage = false;
            return Ok(0.0);
        }
        if self.vars.no_damage {
            ignore_damage = true;
            self.vars.no_damage = false;
        }
        self.vars.stagger_time_mod = wielder.weapon_stagger_back_time_mod(sm, chars, &w, self.is_zombie());
        ctx.event(SimEvent::WorldSound { radius: 5, volume: 1 });
        self.calculate_hit_direction(&w, wielder.vars.x, wielder.vars.y);
        self.vars.attacked_by = Some(wielder.id);
        let damage = self.process_hit_damage(sm, chars, &w, wielder, damage_split, ignore_damage, mod_delta);
        self.hit_consequences(&w, weapon_item, wielder, ignore_damage, damage, cx)?;
        if self.is_zombie() {
            self.vars.time_since_seen_flesh = 0.0;
            let dx = self.vars.x - wielder.vars.x;
            let dy = self.vars.y - wielder.vars.y;
            let close = ((dx * dx + dy * dy) as f64).sqrt() as f32 <= 0.9;
            use super::states::ids;
            let attacking = [ids::ATTACK, ids::ATTACK_NETWORK, ids::LUNGE, ids::LUNGE_NETWORK].iter().any(|s| self.is_current_state_id(*s));
            if !self.is_dead() && !self.vars.on_floor && !ignore_damage && wielder.is_player() && close && attacking {
                self.vars.hit_force = 0.5;
                self.change_state(Some(super::states::stagger_back::instance()), cx);
            }
        }
        Ok(damage)
    }

    /// `hitConsequences(weapon, wielder, ignoreDamage, damage, false)`.
    fn hit_consequences(
        &mut self,
        w: &WeaponStats,
        weapon_item: &InventoryItem,
        wielder: &mut GameCharacter<I>,
        ignore_damage: bool,
        damage: f32,
        cx: &mut StateCx<'_, I>,
    ) -> ScriptResult {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        if self.is_animal() && self.animal.is_some() {
            // IsoAnimal.hitConsequences replaces the character's.
            let aiming = wielder.perk_level(chars.perks.constant("Aiming").as_ref());
            return self.animal_hit_consequences(wielder, aiming, ignore_damage, damage, cx);
        }
        match self.kind {
            CharacterKind::Player | CharacterKind::Animal => {
                let hit_reaction = wielder.get_variable_string("ZombieHitReaction");
                if hit_reaction == "Shot" && wielder.is_player() {
                    let chance = wielder.calculate_crit_chance(sm, chars, ctx, self);
                    wielder.vars.critical_hit = ctx.rand_next(100) < chance;
                }
                self.vars.knocked_down = wielder.vars.critical_hit;
                if wielder.is_player() {
                    if !self.vars.hit_reaction.is_empty() {
                        self.state.action_context.report_event(None, "washitpvpagain");
                    }
                    self.state.action_context.report_event(None, "washitpvp");
                    self.set_variable_bool("hitpvp", true);
                } else {
                    self.state.action_context.report_event(None, "washit");
                }
                if ignore_damage {
                    wielder.add_xp(sm, chars, chars.perks.constant("Strength").as_ref(), 2.0, ctx)?;
                    self.vars.hit_force = 0.5f32.min(self.vars.hit_force);
                    self.vars.hit_reaction = "HitReaction".into();
                    let side = self.test_dot_side(wielder.vars.x, wielder.vars.y);
                    self.vars.hit_from_behind = side == Side::Behind;
                    return Ok(());
                }
                // `weapon.getUsingPlayer()` is the entity system's user (a
                // crafting player), not the wielder: null for a swing.
                let user: Option<&WeaponUser<'_>> = None;
                self.damage_from_weapon(sm, chars, ctx, weapon_item, user, -1)?;
                let mut reaction = hit_reaction;
                if reaction == "Bite" {
                    match self.test_dot_side(wielder.vars.x, wielder.vars.y) {
                        Side::Right => reaction.push_str("LEFT"),
                        Side::Left => reaction.push_str("RIGHT"),
                        _ => {}
                    }
                    if !reaction.is_empty() {
                        self.vars.hit_reaction = reaction;
                    }
                } else if !self.vars.knocked_down {
                    self.vars.hit_reaction = "HitReaction".into();
                }
                let _ = (w, damage);
                Ok(())
            }
            _ => {
                if self.is_zombie() && self.vars.only_jaw_stab && !self.vars.close_killed {
                    return Ok(());
                }
                self.base_hit_consequences(w, wielder, ignore_damage, damage, cx)?;
                if self.is_zombie() {
                    self.state.action_context.report_event(None, "wasHit");
                    let hw = super::firearm::HitWeapon::of(cx.sm, weapon_item);
                    self.process_hit(&hw, wielder, cx);
                    self.vars.target = Some(wielder.id);
                    self.vars.knocked_down = wielder.vars.critical_hit || self.vars.on_floor || self.vars.always_knocked_down;
                    if self.should_become_crawler(cx.ctx, wielder) {
                        self.vars.become_crawler = true;
                    }
                }
                Ok(())
            }
        }
    }

    /// `IsoGameCharacter.hitConsequences`.
    fn base_hit_consequences(
        &mut self,
        w: &WeaponStats,
        wielder: &mut GameCharacter<I>,
        ignore_damage: bool,
        damage: f32,
        cx: &mut StateCx<'_, I>,
    ) -> ScriptResult {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        if !ignore_damage {
            let d = apply_global_damage_reduction_multipliers(w, damage);
            self.apply_damage_checked(ctx, d);
        }
        if self.is_dead() {
            if !self.vars.on_kill_done {
                if self.is_zombie() {
                    wielder.zombie_kills += 1;
                }
                if !ctx.is_server() && !ctx.is_client() {
                    self.kill_by(Some(wielder.id), cx);
                }
            }
            return Ok(());
        }
        if w.splat_blood_on_no_death {
            ctx.event(SimEvent::SplatBlood { amount: 2, speed: 0.2 });
        }
        if w.ranged {
            return Ok(());
        }
        if w.knock_back_on_no_death && !ctx.is_client() && matches!(wielder.kind, CharacterKind::Player | CharacterKind::Animal) {
            wielder.add_xp(sm, chars, chars.perks.constant("Strength").as_ref(), 2.0, ctx)?;
        }
        Ok(())
    }

    /// `CombatManager.applyDamage(chr, damage)` / `applyDamage(float)`:
    /// not when invulnerable; health floors at 0.
    pub fn apply_damage_checked(&mut self, ctx: &dyn SimContext, damage: f32) {
        if self.is_invulnerable(ctx) {
            return;
        }
        self.health -= damage;
        if self.health < 0.0 {
            self.health = 0.0;
        }
    }

    /// `IsoZombie.shouldBecomeCrawler(attacker)`.
    fn should_become_crawler(&self, ctx: &dyn SimContext, attacker: &GameCharacter<I>) -> bool {
        if self.vars.become_crawler {
            return true;
        }
        if self.vars.crawling || ctx.game_mode() == "LastStand" || self.is_dead() || self.vars.close_killed {
            return false;
        }
        let player = attacker.is_player();
        if player && !attacker.vars.aim_at_floor && attacker.vars.do_shove {
            return false;
        }
        let mut chance = 30;
        if player && attacker.vars.aim_at_floor && attacker.vars.do_shove {
            chance = if self.vars.hit_legs_while_on_floor { 7 } else { 15 };
        }
        super::body_part::next_bool(ctx, chance)
    }

    /// `attackCollisionCheck`'s melee damage for one character target:
    /// the drawn damage with the weapon, strength, pain, trait, split,
    /// moodle and stomp modifiers. `split` is the target's position in
    /// the hit list (1 for the first). Returns `(damageSplit, rangeDel)`.
    pub fn melee_damage_split(
        &self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        w: &WeaponStats,
        split: i32,
        distance: f32,
    ) -> ScriptResult<(f32, f32)> {
        let (min, max) = (w.min_damage, w.max_damage);
        let two_handed_ok = !w.two_hand || self.is_item_in_both_hands(w.id);
        let mut damage = ctx.rand_next_float(min, max);
        if !w.ranged {
            damage *= self.weapon_damage_mod(sm, chars, w) * self.hitting_mod(chars);
        }
        if !two_handed_ok && !w.ranged && max > min {
            damage -= min;
        }
        if !w.ranged {
            let types = &chars.sim.body_part_types;
            let (from, to) = if !self.vars.aim_at_floor || !self.vars.do_shove {
                (types.index("Hand_L")?, types.index("UpperArm_R")?)
            } else {
                (types.index("UpperLeg_L")?, types.index("Foot_R")?)
            };
            let mut pain = 0.0f32;
            for x in from..=to {
                pain += self.body_damage.parts[x].pain(ctx);
            }
            if pain > 10.0 {
                damage /= (pain / 10.0).clamp(1.0, 30.0);
            }
            damage *= self.character_traits.trait_damage_dealt_reduction_modifier(sm)?;
        }
        let mut damage_split = damage;
        if !w.ranged {
            damage_split = damage / (split as f32 * 0.5);
        }
        let mut range_del = if w.ranged { 0.5 } else { distance / w.max_range };
        range_del *= 2.0;
        if range_del < 0.3 {
            range_del = 1.0;
        }
        let panic = self.moodle_lvl(sm, "PANIC");
        if !w.ranged && panic > 1 {
            damage_split -= panic as f32 * 0.1;
        }
        let stress = self.moodle_lvl(sm, "STRESS");
        if !w.ranged && stress > 1 {
            damage_split -= stress as f32 * 0.1;
        }
        if damage_split < 0.0 {
            damage_split = 0.1;
        }
        if self.vars.aim_at_floor && self.vars.do_shove {
            damage_split = ctx.rand_next_float(0.7, 1.0) + self.perk(chars, "Strength") as f32 * 0.2;
            let shoes = self.items.worn().find(|i| super::items::is_body_location(sm, &i.borrow(), "SHOES"));
            damage_split = match shoes.and_then(|s| script_item(sm, &s.borrow())) {
                Some(s) => damage_split * s.borrow().stomp_power,
                None => damage_split * 0.5,
            };
        }
        if !w.ranged {
            let fac = |l: i32| match l {
                1 => 0.5,
                2 => 0.2,
                3 => 0.1,
                4 => 0.05,
                _ => 1.0,
            };
            damage_split *= fac(self.moodle_lvl(sm, "ENDURANCE"));
            damage_split *= fac(self.moodle_lvl(sm, "TIRED"));
        }
        Ok((damage_split, range_del))
    }

    /// `CombatManager.calculateTotalDefense(partHit, target, weapon)` on
    /// `self` (the target).
    pub fn total_defense(&self, sm: &ScriptManager, ctx: &dyn SimContext, part: i32, w: &WeaponStats) -> f32 {
        let mut d = self.items.body_part_clothing_defense(sm, part, false, w.ranged) * 0.5;
        d += self.items.body_part_clothing_defense(sm, part, true, w.ranged);
        d *= ctx.lore_zombies_armor_factor() as f32;
        let max = ctx.lore_zombies_max_defense().min(100);
        if d > max as f32 {
            d = max as f32;
        }
        d
    }

    /// `applyMeleeHitLocationDamage(target, weapon, partHit, hitHead,
    /// hitLegs, damageSplit)` on `self` (the target), without the blood.
    #[allow(clippy::too_many_arguments)]
    pub fn melee_hit_location_damage(
        &self,
        sm: &ScriptManager,
        ctx: &dyn SimContext,
        w: &WeaponStats,
        part: i32,
        hit_head: i32,
        hit_legs: bool,
        mut damage_split: f32,
    ) -> f32 {
        if hit_head > 0 {
            damage_split *= config::HEAD_HIT_DAMAGE_SPLIT_MODIFIER;
        }
        if hit_legs {
            damage_split *= config::LEG_HIT_DAMAGE_SPLIT_MODIFIER;
        }
        let def = self.total_defense(sm, ctx, part, w);
        damage_split * (1.0 - def / 100.0).abs()
    }

    /// `CombatManager.processWeaponEndurance(chr, weapon)`.
    pub fn process_weapon_endurance(&mut self, sm: &ScriptManager, chars: &Characters, w: &WeaponStats) -> ScriptResult {
        if !w.use_endurance {
            return Ok(());
        }
        let weight = w.effective_weight();
        let mut two = 0.0f32;
        if w.two_hand && !self.is_item_in_both_hands(w.id) {
            two = weight / 1.5 / 10.0;
        }
        let val = (weight * 0.18 * self.weapon_fatigue_mod(sm, chars, w) * self.fatigue_mod(chars) * w.endurance_mod * 0.3 + two) * 0.04;
        let m = self.character_traits.trait_endurance_loss_modifier(sm)?;
        self.stats.remove(&chars.sim.character_stats.field("ENDURANCE")?, val * m);
        Ok(())
    }

    /// `CombatManager.applyMeleeEnduranceLoss(attacker, target, weapon,
    /// damage)` on `self` (the attacker).
    pub fn apply_melee_endurance_loss(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        w: &WeaponStats,
        target_health: f32,
        target_close_killed: bool,
        damage: f32,
    ) -> ScriptResult {
        if !w.is_melee() || !w.use_endurance {
            return Ok(());
        }
        let wrong = w.two_hand && !self.is_item_in_both_hands(w.id);
        let two = if wrong {
            w.weight / config::ENDURANCE_LOSS_TWO_HANDED_PENALTY_DIVISOR / config::ENDURANCE_LOSS_TWO_HANDED_PENALTY_SCALE
        } else {
            0.0
        };
        let mut loss = (w.weight
            * config::ENDURANCE_LOSS_BASE_SCALE
            * self.weapon_fatigue_mod(sm, chars, w)
            * self.fatigue_mod(chars)
            * w.endurance_mod
            * config::ENDURANCE_LOSS_WEIGHT_MODIFIER
            + two)
            * config::ENDURANCE_LOSS_FINAL_MULTIPLIER;
        if self.is_player() && self.vars.aim_at_floor && self.vars.do_shove {
            loss *= config::ENDURANCE_LOSS_FLOOR_SHOVE_MULTIPLIER;
        }
        let m = if damage <= 0.0 {
            1.0
        } else if target_close_killed {
            config::ENDURANCE_LOSS_CLOSE_KILL_MODIFIER
        } else {
            (damage.min(target_health) / w.max_damage).min(1.0)
        };
        self.stats.remove(&chars.sim.character_stats.field("ENDURANCE")?, loss * m);
        Ok(())
    }

    /// `CombatManager.checkForConditionLoss`.
    pub fn check_for_condition_loss(&self, w: &WeaponStats, process: bool) -> bool {
        if w.bare_hands {
            return false;
        }
        let has = w.condition > 0;
        if has && w.aimed_firearm {
            return process || self.vars.ranged_weapon_empty;
        }
        process && has
    }

    /// `SwipeStatePlayer.exit`'s stomp foot damage.
    pub fn stomp_foot_damage(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        if !(self.vars.aim_at_floor && self.vars.do_shove) {
            return Ok(());
        }
        let shoes_id = self.items.worn().find(|i| super::items::is_body_location(sm, &i.borrow(), "SHOES")).map(|i| i.borrow().id);
        let mut rand_foot = 10;
        let mut reduce = false;
        match shoes_id.and_then(|id| self.items.item(id)) {
            None => rand_foot = 3,
            Some(shoes) => {
                let clc = script_item(sm, &shoes.borrow()).map_or(0, |s| s.borrow().condition_lower_chance);
                rand_foot += clc / 2;
                if ctx.rand_next(clc) == 0 {
                    reduce = true;
                }
            }
        }
        if reduce {
            // TODO(items): `Clothing.reduceCondition` (the condition loss
            // and its holes).
            if let Some(s) = shoes_id.and_then(|id| self.items.item(id)) {
                s.borrow_mut().condition -= 1;
            }
        }
        if ctx.rand_next(rand_foot) == 0 {
            let foot = chars.sim.body_part_types.index("Foot_R")?;
            let (a, b) = if shoes_id.is_none() { (5, 10) } else { (1, 5) };
            let dmg = ctx.rand_next_range(a, b) as f32;
            self.body_damage.parts[foot].add_damage(dmg);
            let pain = ctx.rand_next_range(a, b) as f32;
            self.body_damage.parts[foot].additional_pain += pain;
        }
        Ok(())
    }

    /// `Kill(killer)`: health to zero and, once, `onKilled`.
    pub fn kill(&mut self, cx: &mut StateCx<'_, I>) {
        let killer = self.vars.attacked_by;
        self.kill_by(killer, cx);
    }

    /// `Kill(killer, weapon, isGory, null)`.
    pub fn kill_by(&mut self, killer: Option<i32>, cx: &mut StateCx<'_, I>) {
        self.vars.attacked_by = killer;
        self.set_health(cx.ctx, 0.0);
        self.body_damage.overall_body_health = 0.0;
        if !self.vars.on_kill_done {
            cx.ctx.event(SimEvent::Killed { attacker: killer });
            self.vars.on_kill_done = true;
        }
    }
}
