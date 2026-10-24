//! Gameplay getters and mutators under the original Java names (snake
//! case), for the character simulation.
//!
//! Every function here either ports the original method (marked "Port")
//! or is a marked stub ("STUB") with the behaviour it still lacks.

use crate::container::ItemContainer;
use crate::context::{ItemContext, NoOwner};
use crate::gameplay::UseOutcome;
use crate::item::{ClothingData, FoodData, InventoryItem, ItemKind, WeaponData};
use crate::template::ItemClass;

/// `ItemTag` ids the simulation asks for.
pub mod tags {
    pub const LOW_ALCOHOL: &str = "base:lowalcohol";
    pub const EGG: &str = "base:egg";
    pub const HANDGUARD: &str = "base:handguard";
    pub const GOOD_FROZEN: &str = "base:goodfrozen";
}

impl InventoryItem {
    /// Port of `hasTag(ItemTag)` (by registry id, e.g. [`tags::EGG`]).
    pub fn has_tag(&self, id: &str) -> bool {
        self.template.has_tag(id)
    }

    /// Port of `isAlcoholic()`.
    pub fn is_alcoholic(&self) -> bool {
        self.template.alcoholic
    }

    /// Port of `getReduceInfectionPower()` (cooked food: x1.3).
    pub fn get_reduce_infection_power(&self) -> f32 {
        let p = self.template.reduce_infection_power;
        if self.is_cooked() {
            p * 1.3
        } else {
            p
        }
    }

    /// Port of `isCooked()` (only food cooks).
    pub fn is_cooked(&self) -> bool {
        self.food().is_some_and(|f| f.cooked)
    }

    /// Port of `getAge()`.
    pub fn get_age(&self) -> f32 {
        self.food().map_or(0.0, |f| f.age)
    }

    /// Port of `getOffAge()`.
    pub fn get_off_age(&self) -> i32 {
        self.food().map_or(1_000_000_000, |f| f.off_age)
    }

    /// Port of `getOffAgeMax()`.
    pub fn get_off_age_max(&self) -> i32 {
        self.food().map_or(1_000_000_000, |f| f.off_age_max)
    }

    /// Port of `getFatigueChange()` (food: its own field).
    pub fn get_fatigue_change(&self) -> f32 {
        match self.food() {
            Some(f) => f.fatigue_change,
            None => self.template.fatigue_change / 100.0,
        }
    }

    /// Port of `getStressChange()` (`Food` adjusts it for state).
    pub fn get_stress_change(&self) -> f32 {
        let Some(f) = self.food() else {
            return self.template.stress_change / 100.0;
        };
        if f.egg.is_some() {
            f.stress_change
        } else if f.burnt {
            f.stress_change / 4.0
        } else if f.age >= f.off_age as f32 && f.age < f.off_age_max as f32 {
            f.stress_change / 1.3
        } else if f.age >= f.off_age_max as f32 {
            f.stress_change / 2.0
        } else if f.cooked {
            f.stress_change * 1.3
        } else {
            f.stress_change
        }
    }

    /// Port of `getBoredomChange()` (`Food` adjusts it for state).
    pub fn get_boredom_change(&self) -> f32 {
        let Some(f) = self.food() else {
            return self.template.boredom_change;
        };
        let mut r = f.boredom_change;
        if f.egg.is_some() {
            return r;
        }
        r += self.food_state_penalty(f);
        r
    }

    /// Port of `getUnhappyChange()` (`Food` adjusts it for state).
    pub fn get_unhappy_change(&self) -> f32 {
        let Some(f) = self.food() else {
            return self.template.unhappy_change;
        };
        let mut r = f.unhappy_change;
        if f.egg.is_some() {
            return r;
        }
        r += self.food_state_penalty(f);
        let t = &self.template;
        if t.bad_cold && f.is_cookable && f.cooked && f.heat < 1.3 {
            r += 2.0;
        }
        if t.good_hot && f.is_cookable && f.cooked && f.heat > 1.3 {
            r -= 2.0;
        }
        r
    }

    fn food_state_penalty(&self, f: &FoodData) -> f32 {
        let good_frozen = self.type_ == "Icecream" || self.has_tag(tags::GOOD_FROZEN);
        let mut r = 0.0;
        if f.frozen && !good_frozen {
            r += 30.0;
        }
        if f.burnt {
            r += 20.0;
        }
        if f.age >= f.off_age as f32 && f.age < f.off_age_max as f32 {
            r += 10.0;
        }
        if f.age >= f.off_age_max as f32 {
            r += 20.0;
        }
        r
    }

    /// Port of `Food.getHungerChange()`.
    pub fn get_hunger_change(&self) -> f32 {
        let Some(f) = self.food() else { return 0.0 };
        let h = f.hung_change;
        if h != 0.0 {
            if f.cooked {
                return h * 1.3;
            }
            let sign = if h < 0.0 { -1.0 } else { 1.0 };
            let a = h.abs();
            if f.burnt {
                return (a / 3.0).max(0.01) * sign;
            }
            if f.age >= f.off_age as f32 && f.age < f.off_age_max as f32 {
                return (a / 1.3).max(0.01) * sign;
            }
            if f.age >= f.off_age_max as f32 {
                return (a / 2.2).max(0.01) * sign;
            }
        }
        h
    }

    /// Port of `Food.getHungChange()`.
    pub fn get_hung_change(&self) -> f32 {
        self.food().map_or(0.0, |f| f.hung_change)
    }

    /// Port of `Food.getBaseHunger()`.
    pub fn get_base_hunger(&self) -> f32 {
        self.food().map_or(0.0, |f| f.base_hunger)
    }

    /// Port of `Food.getPoisonPower()`.
    pub fn get_poison_power(&self) -> i32 {
        self.food().map_or(0, |f| f.poison_power)
    }

    /// Port of `Food.isTainted()` (runtime flag, not saved).
    pub fn is_tainted(&self) -> bool {
        self.food().is_some_and(|f| f.tainted)
    }

    /// Port of `Food.isbDangerousUncooked()`.
    pub fn is_b_dangerous_uncooked(&self) -> bool {
        self.food().is_some_and(|f| f.dangerous_uncooked)
    }

    /// Port of `Food.isBurnt()`.
    pub fn is_burnt(&self) -> bool {
        self.food().is_some_and(|f| f.burnt)
    }

    /// Port of `getOnEat()` (`Food`, `DrainableComboItem`).
    pub fn get_on_eat(&self) -> Option<&str> {
        self.template.on_eat.as_deref()
    }

    /// Port of `getCurrentUses()` (`Food` with a base hunger counts its
    /// remaining hunger).
    pub fn get_current_uses(&self) -> i32 {
        match self.food() {
            Some(f) if f.base_hunger != 0.0 => (f.hung_change * 100.0).abs() as i32,
            _ => self.uses,
        }
    }

    /// Port of `setCurrentUses(int)` (`Food` consumes hunger; the
    /// drainable weight update is STUB).
    pub fn set_current_uses(&mut self, uses: i32) {
        let cur = self.get_current_uses();
        match &mut self.kind {
            ItemKind::Food(f) => {
                let uses = uses.max(0);
                if f.base_hunger == 0.0 {
                    self.uses = uses;
                } else {
                    // consumeHunger((current - new) / 100)
                    let used = (cur - uses) as f32 / 100.0;
                    let pct = (used / f.hung_change).abs();
                    f.multiply_food_values(1.0 - pct);
                }
            }
            _ => self.uses = uses,
        }
    }

    /// Port of `getMaxUses()`: `DrainableComboItem` uses
    /// `floor(1 / useDelta)`, every other item 1.
    pub fn get_max_uses(&self) -> i32 {
        if self.template.class == ItemClass::Drainable {
            (1.0 / self.template.use_delta).floor() as i32
        } else {
            1
        }
    }

    /// Port of `Use()` for an item in a container without a parent and
    /// without script lookups: the item changes as in the original (a
    /// drainable loses a use, so a used-up tissue reaches -1; food
    /// consumes hunger) and the outcome says what the container must do
    /// (`added` names a `ReplaceOnUse`/`ReplaceOnDeplete` item without
    /// creating it). [`InventoryItem::use_with`] takes the owner and game
    /// state; [`ItemContainer::use_item`] applies the outcome.
    pub fn use_(&mut self) -> UseOutcome {
        self.use_with(false, false, false, &NoOwner, &Offline)
    }

    /// Port of `UseAndSync()`: `Use(false, false, GameServer.server)` (not
    /// a server here, so no sync).
    pub fn use_and_sync(&mut self) -> UseOutcome {
        self.use_()
    }

    /// Port of `HandWeapon.getWeaponCategories()` (registry ids).
    pub fn get_weapon_categories(&self) -> &[String] {
        &self.template.weapon_categories
    }

    /// Port of `isAimedFirearm()`.
    pub fn is_aimed_firearm(&self) -> bool {
        self.template.is_aimed_firearm
    }

    /// Port of `isRanged()`.
    pub fn is_ranged(&self) -> bool {
        self.template.ranged
    }

    /// Port of `HandWeapon.getMinDamage()`.
    pub fn get_min_damage(&self) -> f32 {
        self.weapon().map_or(0.0, |w| w.min_damage)
    }

    /// Port of `HandWeapon.getMaxDamage()`.
    pub fn get_max_damage(&self) -> f32 {
        self.weapon().map_or(0.0, |w| w.max_damage)
    }

    /// Port of `WeaponType.getWeaponType(HandWeapon)`, as the enum
    /// constant's name (`"KNIFE"`, ...). `None` for items that are not
    /// weapons, or without a swing animation (the original throws).
    pub fn get_weapon_type(&self) -> Option<&'static str> {
        self.weapon_type().map(WeaponType::name)
    }

    /// [`Self::get_weapon_type`] as the enum.
    pub fn weapon_type(&self) -> Option<WeaponType> {
        if !matches!(self.kind, ItemKind::Weapon(_)) {
            return None;
        }
        let swing = self.template.swing_anim.as_deref()?;
        let is = |s: &str| swing.eq_ignore_ascii_case(s);
        Some(if is("Stab") {
            WeaponType::Knife
        } else if is("Heavy") {
            WeaponType::Heavy
        } else if is("Throw") {
            WeaponType::Throwing
        } else if self.is_ranged() {
            if self.is_two_hand_weapon() {
                WeaponType::Firearm
            } else {
                WeaponType::Handgun
            }
        } else if self.is_two_hand_weapon() {
            if is("Spear") {
                WeaponType::Spear
            } else if self.type_ == "Chainsaw" {
                WeaponType::Chainsaw
            } else {
                WeaponType::TwoHanded
            }
        } else {
            WeaponType::OneHanded
        })
    }

    /// Port of `Clothing.getScratchDefense()` (blade defense): 0 when
    /// broken; the script value capped at 100 (`setScratchDefense`).
    /// Other items: the script value.
    pub fn get_scratch_defense(&self) -> f32 {
        self.clothing_defense(self.template.scratch_defense)
    }

    /// Port of `Clothing.getBiteDefense()`.
    pub fn get_bite_defense(&self) -> f32 {
        self.clothing_defense(self.template.bite_defense)
    }

    /// Port of `Clothing.getBulletDefense()`.
    pub fn get_bullet_defense(&self) -> f32 {
        self.clothing_defense(self.template.bullet_defense)
    }

    fn clothing_defense(&self, v: f32) -> f32 {
        if self.clothing().is_none() {
            return v;
        }
        if self.condition <= 0 {
            return 0.0;
        }
        v.min(100.0)
    }

    /// Port of `Clothing.getCorpseSicknessDefense()` without the filter
    /// part (0 when broken); [`Self::get_corpse_sickness_defense_with`]
    /// adds the gas mask filter.
    pub fn get_corpse_sickness_defense(&self) -> f32 {
        if self.clothing().is_some() && self.condition <= 0 {
            return 0.0;
        }
        self.template.corpse_sickness_defense
    }

    /// Port of `Clothing.getInsulation()`.
    pub fn get_insulation(&self) -> f32 {
        self.template.insulation
    }

    /// Port of `Clothing.getWindresistance()`.
    pub fn get_windresistance(&self) -> f32 {
        self.template.windresist
    }

    /// Whether this clothing spikes attackers from the front: the
    /// original has no such item method; `IsoGameCharacter.bodyPartIsSpiked`
    /// tests the worn clothing's `Spiked` tag, which this reads.
    pub fn is_spiked_front(&self) -> bool {
        self.clothing().is_some() && self.has_tag(crate::gameplay::tags::SPIKED)
    }

    /// The `SpikedBehind` tag (`bodyPartIsSpikedBehind`).
    pub fn is_spiked_behind(&self) -> bool {
        self.clothing().is_some() && self.has_tag(crate::gameplay::tags::SPIKED_BEHIND)
    }

    /// Port of `Clothing.getDirtyness()`.
    pub fn get_dirtyness(&self) -> f32 {
        self.clothing().map_or(0.0, |c| c.dirtyness)
    }

    /// Port of `Clothing.getBloodlevel()`.
    pub fn get_bloodlevel(&self) -> f32 {
        match &self.kind {
            ItemKind::Weapon(w) => w.blood_level,
            _ => self.clothing().map_or(0.0, |c| c.blood_level),
        }
    }

    /// Port of `Clothing.getWetness()`.
    pub fn get_wetness(&self) -> f32 {
        self.clothing().map_or(0.0, |c| c.wetness)
    }

    /// The gas mask filter (or SCBA tank) left: `Clothing.getUsedDelta()`
    /// without its write of 0 when missing. See
    /// [`Self::get_used_delta`], [`Self::drain_gas_mask`].
    pub fn get_gas_mask_drain(&self) -> f32 {
        self.used_delta()
    }

    /// Port of the part of `Clothing.updateWetness(true)` that needs no
    /// time or place: clothing without a blood clothing type dries at
    /// once. [`Self::update_wetness_with`] is the whole method.
    pub fn update_wetness(&mut self) {
        if self.clothing().is_some() && self.template.blood_clothing_type.is_none() {
            self.set_wetness(0.0);
        }
    }

    /// Port of `getActualWeight()` (with the `name == fullType` rule and
    /// the `HandWeapon` and `Food` overrides) without script lookups:
    /// food with a `ReplaceOnUse` or extra items needs
    /// [`Self::get_actual_weight_with`].
    pub fn get_actual_weight(&self) -> f32 {
        self.actual_weight_impl(None)
    }

    fn food(&self) -> Option<&FoodData> {
        match &self.kind {
            ItemKind::Food(f) => Some(f),
            _ => None,
        }
    }

    fn weapon(&self) -> Option<&WeaponData> {
        match &self.kind {
            ItemKind::Weapon(w) => Some(w),
            _ => None,
        }
    }

    fn clothing(&self) -> Option<&ClothingData> {
        match &self.kind {
            ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) => Some(c),
            _ => None,
        }
    }
}

impl FoodData {
    /// Port of `Food.multiplyFoodValues`.
    pub fn multiply_food_values(&mut self, p: f32) {
        self.boredom_change *= p;
        self.unhappy_change *= p;
        self.hung_change *= p;
        self.flu_reduction = (self.flu_reduction as f32 * p) as i32;
        self.thirst_change *= p;
        self.pain_reduction *= p;
        self.food_sickness_change = (self.food_sickness_change as f32 * p) as i32;
        self.end_change *= p;
        self.stress_change *= p;
        self.fatigue_change *= p;
        self.calories *= p;
        self.carbohydrates *= p;
        self.proteins *= p;
        self.lipids *= p;
        self.poison_power = (self.poison_power as f32 * p) as i32;
    }
}

impl ItemContainer {
    /// Port of `ItemContainer.getContentsWeight()` without script lookups
    /// or fluids (no ammo or fluid weight):
    /// [`Self::get_contents_weight_with`] is the whole method; the
    /// equipped and worn rules (`getInventoryWeight`) are the character's.
    pub fn get_contents_weight(&self) -> f32 {
        self.get_contents_weight_with(&Offline)
    }
}

/// `zombie.inventory.types.WeaponType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// The enum constant's name.
    pub fn name(self) -> &'static str {
        match self {
            WeaponType::Unarmed => "UNARMED",
            WeaponType::TwoHanded => "TWO_HANDED",
            WeaponType::OneHanded => "ONE_HANDED",
            WeaponType::Heavy => "HEAVY",
            WeaponType::Knife => "KNIFE",
            WeaponType::Spear => "SPEAR",
            WeaponType::Handgun => "HANDGUN",
            WeaponType::Firearm => "FIREARM",
            WeaponType::Throwing => "THROWING",
            WeaponType::Chainsaw => "CHAINSAW",
        }
    }
}

/// An [`ItemContext`] with no scripts, no climate and zero globals: what
/// the context-free ports above run with.
pub struct Offline;
impl ItemContext for Offline {
    fn world_age_hours(&self) -> f64 {
        0.0
    }
    fn multiplier(&self) -> f32 {
        1.0
    }
    fn rain_intensity(&self) -> f32 {
        0.0
    }
    fn is_raining(&self) -> bool {
        false
    }
    fn equipped_or_worn_encumbrance_multiplier(&self) -> f64 {
        1.0
    }
    fn basic_mask_defense(&self) -> f32 {
        0.0
    }
    fn improvised_gas_mask_defense(&self) -> f32 {
        0.0
    }
    fn standard_gas_mask_defense(&self) -> f32 {
        0.0
    }
    fn script(&self, _name: &str) -> Option<std::rc::Rc<crate::template::ItemTemplate>> {
        None
    }
}
