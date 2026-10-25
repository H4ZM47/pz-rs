//! Gameplay getters and mutators under the original Java names (snake
//! case), for the character simulation.
//!
//! Every function here either ports the original method (marked "Port")
//! or is a marked stub ("STUB") with the behaviour it still lacks.

use crate::container::ItemContainer;
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

    /// STUB of `Use()`: decrements the uses only. The original also
    /// replaces depleted drainables (`ReplaceOnDeplete`), removes the item
    /// from its container and syncs; those need the container owner.
    pub fn use_(&mut self) {
        self.uses -= 1;
    }

    /// STUB of `UseAndSync()`: [`Self::use_`] without the network sync.
    pub fn use_and_sync(&mut self) {
        self.use_();
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

    /// STUB of `WeaponType.getWeaponType(HandWeapon)`: needs the
    /// `WeaponType` table from `pz-classfile` and the swing animation;
    /// returns `None` until ported.
    pub fn get_weapon_type(&self) -> Option<&'static str> {
        None
    }

    /// Port of `Clothing.getScratchDefense()` (blade defense).
    pub fn get_scratch_defense(&self) -> f32 {
        self.template.scratch_defense
    }

    /// Port of `Clothing.getBiteDefense()`.
    pub fn get_bite_defense(&self) -> f32 {
        self.template.bite_defense
    }

    /// Port of `Clothing.getBulletDefense()`.
    pub fn get_bullet_defense(&self) -> f32 {
        self.template.bullet_defense
    }

    /// Port of `getCorpseSicknessDefense()`.
    pub fn get_corpse_sickness_defense(&self) -> f32 {
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

    /// STUB of `Clothing.isSpikedFront()`/`isSpikedBehind()`: runtime
    /// flags (not saved), default false.
    pub fn is_spiked_front(&self) -> bool {
        self.clothing().is_some_and(|c| c.spiked_front)
    }

    pub fn is_spiked_behind(&self) -> bool {
        self.clothing().is_some_and(|c| c.spiked_behind)
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

    /// STUB of the gas mask filter drain (`ClothingFilter`/`Durability`
    /// components): the components are opaque until `pz-entity` lands.
    pub fn get_gas_mask_drain(&self) -> f32 {
        0.0
    }

    /// STUB of `ClothingWetness.updateWetness` for this item: needs the
    /// character's body parts and climate; not ported.
    pub fn update_wetness(&mut self) {}

    /// Port of `getActualWeight()`.
    pub fn get_actual_weight(&self) -> f32 {
        self.actual_weight
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

    /// The `HandWeapon` fields, for a weapon.
    pub fn weapon_data(&self) -> Option<&WeaponData> {
        self.weapon()
    }

    /// The `HandWeapon` fields, mutably.
    pub fn weapon_data_mut(&mut self) -> Option<&mut WeaponData> {
        match &mut self.kind {
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
    /// STUB of `ItemContainer.getCapacityWeight()`/`getContentsWeight()`:
    /// sums `getUnequippedWeight()` approximated by `getActualWeight()` of
    /// decoded items. The original's equipped/worn weight rules,
    /// `InventoryContainer` weight reduction and fluid weights are not
    /// ported yet.
    pub fn get_contents_weight(&self) -> f32 {
        self.items
            .iter()
            .filter_map(|i| i.as_item())
            .map(|i| i.get_actual_weight())
            .sum()
    }
}
