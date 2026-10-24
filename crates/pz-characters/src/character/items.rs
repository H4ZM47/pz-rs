//! The character's items (`IsoGameCharacter.inventory`, the hand items and
//! `wornItems`) and the `IsoGameCharacter` methods the simulation reads
//! them through: clothing defense, spikes, dirty and bloody clothing, the
//! corpse sickness defense, the inventory weight and `getItemVisuals`.
//! `zombie.characterTextures.BloodClothingType`'s covered parts are here
//! too.
//!
//! Items are `pz-inventory` [`InventoryItem`]s in the character's
//! [`ItemContainer`]; the hand and worn items are the ids of items in it
//! (the original holds the same objects in both places).
//!
//! What `pz-inventory` does not have yet is read here from the item's
//! script (`getScriptItem()`), or is a marked STUB:
//! * `getVisual()` is the item's stored [`InventoryItem::visual`] (the
//!   original makes it from the `ClothingItem` asset);
//! * `getCurrentCondition`, `Clothing.setWetness`, `flushWetness`,
//!   `getWaterResistance` and `getUnequippedWeight` are written out here;
//! * `isFakeEquipped`, `Clothing.hasFilter`/`hasTank` are false.

use std::cell::RefCell;
use std::rc::Rc;

use pz_inventory::{InventoryItem, ItemContainer, ItemKind};
use pz_scripts::objects::items::{get_item, Item, ItemBodyLocation, ItemTag};
use pz_scripts::ScriptManager;

use super::context::SimContext;
use super::GameCharacter;

/// `BloodBodyPartType` indices.
pub mod blood_part {
    pub const HAND_L: usize = 0;
    pub const HAND_R: usize = 1;
    pub const FORE_ARM_L: usize = 2;
    pub const FORE_ARM_R: usize = 3;
    pub const UPPER_ARM_L: usize = 4;
    pub const UPPER_ARM_R: usize = 5;
    pub const TORSO_UPPER: usize = 6;
    pub const TORSO_LOWER: usize = 7;
    pub const HEAD: usize = 8;
    pub const NECK: usize = 9;
    pub const GROIN: usize = 10;
    pub const UPPER_LEG_L: usize = 11;
    pub const UPPER_LEG_R: usize = 12;
    pub const LOWER_LEG_L: usize = 13;
    pub const LOWER_LEG_R: usize = 14;
    pub const FOOT_L: usize = 15;
    pub const FOOT_R: usize = 16;
    pub const BACK: usize = 17;
    /// `BloodBodyPartType.MAX.index()`.
    pub const MAX: usize = 18;
}

/// `BloodClothingType.fromString(name).coveredParts` (the enum's
/// constructor arguments, parents first).
pub fn blood_clothing_covered_parts(name: &str) -> Option<Vec<usize>> {
    use blood_part::*;
    let shirt_no_sleeves = || vec![TORSO_UPPER, TORSO_LOWER, BACK];
    let shirt = || {
        let mut v = shirt_no_sleeves();
        v.extend([UPPER_ARM_L, UPPER_ARM_R]);
        v
    };
    let shirt_long = || {
        let mut v = shirt();
        v.extend([FORE_ARM_L, FORE_ARM_R]);
        v
    };
    let shorts = || vec![GROIN, UPPER_LEG_L, UPPER_LEG_R];
    Some(match name {
        "Apron" => vec![TORSO_UPPER, TORSO_LOWER, UPPER_LEG_L, UPPER_LEG_R],
        "ShirtNoSleeves" | "JumperNoSleeves" => shirt_no_sleeves(),
        "Shirt" => shirt(),
        "ShirtLongSleeves" | "Jumper" => shirt_long(),
        "Jacket" => {
            let mut v = shirt_long();
            v.push(NECK);
            v
        }
        "LongJacket" => {
            let mut v = shirt_long();
            v.extend([NECK, GROIN, UPPER_LEG_L, UPPER_LEG_R]);
            v
        }
        "ShortsShort" => shorts(),
        "Trousers" => {
            let mut v = shorts();
            v.extend([LOWER_LEG_L, LOWER_LEG_R]);
            v
        }
        "Shoes" => vec![FOOT_L, FOOT_R],
        "FullHelmet" | "Head" => vec![HEAD],
        "Bag" => vec![BACK],
        "Hands" => vec![HAND_L, HAND_R],
        "Neck" => vec![NECK],
        "Groin" => vec![GROIN],
        "UpperBody" => vec![TORSO_UPPER],
        "LowerBody" => vec![TORSO_LOWER],
        "LowerLegs" => vec![LOWER_LEG_L, LOWER_LEG_R],
        "UpperLegs" => vec![UPPER_LEG_L, UPPER_LEG_R],
        "LowerArms" => vec![FORE_ARM_L, FORE_ARM_R],
        "UpperArms" => vec![UPPER_ARM_L, UPPER_ARM_R],
        "Hand_L" => vec![HAND_L],
        "Hand_R" => vec![HAND_R],
        "ForeArm_L" => vec![FORE_ARM_L],
        "ForeArm_R" => vec![FORE_ARM_R],
        "UpperArm_L" => vec![UPPER_ARM_L],
        "UpperArm_R" => vec![UPPER_ARM_R],
        "UpperLeg_L" => vec![UPPER_LEG_L],
        "UpperLeg_R" => vec![UPPER_LEG_R],
        "LowerLeg_L" => vec![LOWER_LEG_L],
        "LowerLeg_R" => vec![LOWER_LEG_R],
        "Foot_L" => vec![FOOT_L],
        "Foot_R" => vec![FOOT_R],
        _ => return None,
    })
}

/// `getScriptItem()`: the item's script by full type.
pub fn script_item(sm: &ScriptManager, item: &InventoryItem) -> Option<Rc<RefCell<Item>>> {
    get_item(sm, &item.get_full_type())
}

/// `BloodClothingType.getCoveredParts(getBloodClothingType())`; `None`
/// when the item (script) has no blood clothing type.
pub fn covered_parts(sm: &ScriptManager, item: &InventoryItem) -> Option<Vec<usize>> {
    let script = script_item(sm, item)?;
    let script = script.borrow();
    let types = script.blood_clothing_type.as_ref()?;
    Some(types.iter().filter_map(|t| blood_clothing_covered_parts(t)).flatten().collect())
}

/// `hasTag(ItemTag.<field>)`.
pub fn has_tag(sm: &ScriptManager, item: &InventoryItem, field: &str) -> bool {
    ItemTag::constant(sm, field).is_some_and(|k| item.has_tag(&k.location.to_string()))
}

/// `instanceof Clothing`.
pub fn is_clothing(item: &InventoryItem) -> bool {
    matches!(item.kind, ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..))
}

/// `instanceof HandWeapon`.
pub fn is_hand_weapon(item: &InventoryItem) -> bool {
    matches!(item.kind, ItemKind::Weapon(_))
}

/// `getVisual()`: STUB, the item's stored visual (the original makes it
/// from the `ClothingItem` asset, and has none without one).
pub fn visual(item: &InventoryItem) -> Option<&pz_inventory::visual::ItemVisual> {
    item.visual.as_ref()
}

fn visual_value(item: &InventoryItem, array: usize, part: usize) -> f32 {
    visual(item)
        .and_then(|v| v.arrays[array].as_ref())
        .and_then(|a| a.get(part))
        .map_or(0.0, |&b| (b as u8) as f32 / 255.0)
}

/// `getVisual().getBlood(part)` (0 without a visual).
pub fn visual_blood(item: &InventoryItem, part: usize) -> f32 {
    visual_value(item, 0, part)
}

/// `getVisual().getDirt(part)`.
pub fn visual_dirt(item: &InventoryItem, part: usize) -> f32 {
    visual_value(item, 1, part)
}

/// `getVisual().getHole(part)`.
pub fn visual_hole(item: &InventoryItem, part: usize) -> f32 {
    visual_value(item, 2, part)
}

/// `getCurrentCondition()`.
pub fn current_condition(item: &InventoryItem) -> f32 {
    item.condition as f32 / item.condition_max as f32 * 100.0
}

/// `getBodyLocation()` is `ItemBodyLocation.<field>`.
pub fn is_body_location(sm: &ScriptManager, item: &InventoryItem, field: &str) -> bool {
    let Some(script) = script_item(sm, item) else { return false };
    let script = script.borrow();
    match (&script.body_location, ItemBodyLocation::constant(sm, field)) {
        (Some(a), Some(b)) => a.location == b.location,
        _ => false,
    }
}

/// `isEquippedNoSprint()` (`scriptItem.equippedNoSprint`).
pub fn is_equipped_no_sprint(sm: &ScriptManager, item: &InventoryItem) -> bool {
    script_item(sm, item).is_some_and(|s| s.borrow().equipped_no_sprint)
}

/// `isProtectFromRainWhileEquipped()`.
pub fn is_protect_from_rain_while_equipped(sm: &ScriptManager, item: &InventoryItem) -> bool {
    script_item(sm, item).is_some_and(|s| s.borrow().protect_from_rain_when_equipped)
}

/// `Clothing.getWaterResistance()` (the script's value, copied at
/// creation).
pub fn water_resistance(item: &InventoryItem) -> f32 {
    item.template.waterresist
}

/// `Clothing.setWetness(percent)`: clamped to `[0, 100]`.
pub fn set_wetness(item: &mut InventoryItem, percent: f32) {
    if let ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) = &mut item.kind {
        c.wetness = percent.clamp(0.0, 100.0);
    }
}

/// `Clothing.flushWetness()`: `updateWetness(true)` (STUB in
/// `pz-inventory`) unless never updated, then marks it not updated.
pub fn flush_wetness(item: &mut InventoryItem) {
    let last = match &item.kind {
        ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) => c.last_wetness_update,
        _ => return,
    };
    if last < 0.0 {
        return;
    }
    item.update_wetness();
    if let ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) = &mut item.kind {
        c.last_wetness_update = -1.0;
    }
}

/// `getContentsWeight()` (`InventoryContainer`: its container's).
fn contents_weight(item: &InventoryItem) -> f32 {
    match &item.kind {
        ItemKind::Container(c) => c.container.get_contents_weight(),
        _ => 0.0,
    }
}

/// `getUnequippedWeight()`: `getActualWeight() + getContentsWeight()`.
pub fn unequipped_weight(item: &InventoryItem) -> f32 {
    item.get_actual_weight() + contents_weight(item)
}

/// `getEquippedWeight()`.
pub fn equipped_weight(item: &InventoryItem, ctx: &dyn SimContext) -> f32 {
    (item.get_actual_weight() + contents_weight(item))
        * ctx.zomboid_globals().equipped_or_worn_encumbrance_multiplier as f32
}

/// `getHotbarEquippedWeight()`.
pub fn hotbar_equipped_weight(sm: &ScriptManager, item: &InventoryItem, ctx: &dyn SimContext) -> f32 {
    if has_tag(sm, item, "LIGHT_WHEN_ATTACHED") {
        return equipped_weight(item, ctx);
    }
    (item.get_actual_weight() + contents_weight(item)) * 0.7
}

/// `Clothing.getDefForPart(part, bite, bullet)`. The patches and the
/// condition check are `pz-inventory`'s (its defense getters).
fn def_for_part(sm: &ScriptManager, item: &InventoryItem, part: usize, bite: bool, bullet: bool) -> f32 {
    if visual_hole(item, part) > 0.0 {
        return 0.0;
    }
    let mut defense = item.get_scratch_defense();
    if bite {
        defense = item.get_bite_defense();
    }
    if bullet {
        defense = item.get_bullet_defense();
    }
    if part == blood_part::NECK {
        if let Some(s) = script_item(sm, item) {
            let m = s.borrow().neck_protection_modifier;
            if m < 1.0 {
                defense *= m;
            }
        }
    }
    // STUB: `getPatchType(part)` (`ClothingData::patches`) is not applied.
    defense
}

/// The character's items.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterItems {
    /// `inventory`.
    pub inventory: ItemContainer,
    /// `primaryHandItem`: the id of an item in `inventory`.
    pub primary_hand_item: Option<i32>,
    /// `secondaryHandItem`.
    pub secondary_hand_item: Option<i32>,
    /// `wornItems`, in order: ids of items in `inventory`.
    pub worn_items: Vec<i32>,
}

impl Default for CharacterItems {
    fn default() -> Self {
        CharacterItems {
            inventory: ItemContainer::new(Some("none")),
            primary_hand_item: None,
            secondary_hand_item: None,
            worn_items: Vec::new(),
        }
    }
}

impl CharacterItems {
    /// The item `id` in the inventory.
    pub fn item(&self, id: i32) -> Option<&InventoryItem> {
        self.inventory.items.iter().filter_map(|i| i.as_item()).find(|i| i.id == id)
    }

    /// The item `id` in the inventory, mutably.
    pub fn item_mut(&mut self, id: i32) -> Option<&mut InventoryItem> {
        self.inventory.items.iter_mut().filter_map(|i| i.as_item_mut()).find(|i| i.id == id)
    }

    /// `getPrimaryHandItem()`.
    pub fn primary_hand_item(&self) -> Option<&InventoryItem> {
        self.primary_hand_item.and_then(|id| self.item(id))
    }

    /// `getSecondaryHandItem()`.
    pub fn secondary_hand_item(&self) -> Option<&InventoryItem> {
        self.secondary_hand_item.and_then(|id| self.item(id))
    }

    /// `getWornItems()` items, in order.
    pub fn worn(&self) -> impl Iterator<Item = &InventoryItem> {
        self.worn_items.iter().filter_map(|&id| self.item(id))
    }

    /// `getItemVisuals(itemVisuals)`: the worn items that have a visual
    /// (`WornItems.getItemVisuals`), as their items.
    pub fn item_visuals(&self) -> Vec<&InventoryItem> {
        self.worn().filter(|i| visual(i).is_some()).collect()
    }

    /// Ids of [`Self::item_visuals`].
    pub fn item_visual_ids(&self) -> Vec<i32> {
        self.item_visuals().iter().map(|i| i.id).collect()
    }

    /// `isEquipped(item)`: worn or in a hand.
    pub fn is_equipped(&self, id: i32) -> bool {
        self.worn_items.contains(&id) || self.primary_hand_item == Some(id) || self.secondary_hand_item == Some(id)
    }

    /// `getInventoryWeight()` (`getInventory().getCapacityWeight()` for a
    /// character without unlimited carry). STUB parts: `isFakeEquipped`
    /// is false; container contents weigh `ItemContainer::get_contents_weight`.
    pub fn inventory_weight(&self, sm: &ScriptManager, ctx: &dyn SimContext) -> f32 {
        let mut total = 0.0f32;
        for item in self.inventory.items.iter().filter_map(|i| i.as_item()) {
            let equipped = self.is_equipped(item.id);
            if item.attached_slot > -1 && !equipped {
                total += hotbar_equipped_weight(sm, item, ctx);
                continue;
            }
            if equipped {
                total += equipped_weight(item, ctx);
                continue;
            }
            total += unequipped_weight(item);
        }
        total
    }

    /// `bodyPartHasTag(part, tag)`.
    fn body_part_has_tag(&self, sm: &ScriptManager, part: i32, field: &str) -> bool {
        for item in self.item_visuals().into_iter().rev() {
            let Some(parts) = covered_parts(sm, item) else { continue };
            for &p in &parts {
                if !is_clothing(item) || p as i32 != part || visual_hole(item, p) != 0.0 || !has_tag(sm, item, field) {
                    continue;
                }
                return true;
            }
        }
        false
    }

    /// `bodyPartIsSpiked(part)`.
    pub fn body_part_is_spiked(&self, sm: &ScriptManager, part: i32) -> bool {
        self.body_part_has_tag(sm, part, "SPIKED")
    }

    /// `bodyPartIsSpikedBehind(part)`.
    pub fn body_part_is_spiked_behind(&self, sm: &ScriptManager, part: i32) -> bool {
        self.body_part_has_tag(sm, part, "SPIKED_BEHIND")
    }

    /// `getBodyPartClothingDefense(part, bite, bullet)`.
    pub fn body_part_clothing_defense(&self, sm: &ScriptManager, part: i32, bite: bool, bullet: bool) -> f32 {
        let mut result = 0.0f32;
        for item in self.item_visuals().into_iter().rev() {
            let Some(parts) = covered_parts(sm, item) else { continue };
            for &p in &parts {
                if !is_clothing(item) || p as i32 != part || visual_hole(item, p) != 0.0 {
                    continue;
                }
                result += def_for_part(sm, item, p, bite, bullet);
                break;
            }
        }
        result.min(100.0)
    }

    fn clothing_matches(&self, sm: &ScriptManager, part: i32, test: impl Fn(&InventoryItem, usize) -> bool) -> bool {
        for item in self.item_visuals().into_iter().rev() {
            let Some(parts) = covered_parts(sm, item) else { continue };
            for &p in &parts {
                if is_clothing(item) && p as i32 == part && test(item, p) {
                    return true;
                }
            }
        }
        false
    }

    /// `hasDirtyClothing(part)`.
    pub fn has_dirty_clothing(&self, sm: &ScriptManager, part: i32) -> bool {
        self.clothing_matches(sm, part, |i, p| visual_dirt(i, p) > 0.15)
    }

    /// `hasBloodyClothing(part)`.
    pub fn has_bloody_clothing(&self, sm: &ScriptManager, part: i32) -> bool {
        self.clothing_matches(sm, part, |i, p| visual_blood(i, p) > 0.25)
    }

    /// `getCorpseSicknessDefense(rate, drain)`. The gas mask and SCBA
    /// branches need `Clothing.hasFilter`/`hasTank`/`getUsedDelta`, which
    /// `pz-inventory` lacks: STUB, `get_gas_mask_drain()` stands for the
    /// used delta and no filter is drained.
    pub fn corpse_sickness_defense(&self, sm: &ScriptManager, ctx: &dyn SimContext, rate: f32, drain: bool) -> f32 {
        let g = ctx.zomboid_globals();
        let mut defense = 0.0f32;
        for item in self.worn() {
            if is_clothing(item) {
                let used_delta = item.get_gas_mask_drain();
                let has_tank = false;
                let has_filter = false;
                if has_tag(sm, item, "SCBA") && item.activated && has_tank && used_delta > 0.0 {
                    return g.standard_gas_mask_defense;
                }
                if item.get_corpse_sickness_defense() > defense {
                    defense = item.get_corpse_sickness_defense();
                }
                let mask = ["GAS_MASK", "RESPIRATOR", "IMPROVISED_GAS_MASK"].iter().any(|t| has_tag(sm, item, t));
                if mask && has_filter && used_delta > 0.0 {
                    let d = if has_tag(sm, item, "IMPROVISED_GAS_MASK") {
                        g.improvised_gas_mask_defense
                    } else {
                        g.standard_gas_mask_defense
                    };
                    defense = defense.max(d);
                    if drain && rate > 0.0 {
                        // STUB: `drainGasMask(rate * corpseSicknessFilterDrainMultiplier)`.
                        let _ = rate as f64 * g.corpse_sickness_filter_drain_multiplier;
                    }
                }
            }
            if defense >= g.standard_gas_mask_defense {
                return g.standard_gas_mask_defense;
            }
        }
        defense
    }

    /// `isProtectedFromToxic(drain)`: STUB like
    /// [`Self::corpse_sickness_defense`] (no tank or filter is known).
    pub fn is_protected_from_toxic(&self, sm: &ScriptManager) -> bool {
        for item in self.worn() {
            if !is_clothing(item) {
                continue;
            }
            let used_delta = item.get_gas_mask_drain();
            let (has_tank, has_filter) = (false, false);
            if has_tag(sm, item, "SCBA") && item.activated && has_tank && used_delta > 0.0 {
                return true;
            }
            let mask = ["GAS_MASK", "RESPIRATOR", "IMPROVISED_GAS_MASK"].iter().any(|t| has_tag(sm, item, t));
            if mask && has_filter && used_delta > 0.0 {
                return true;
            }
        }
        false
    }

    /// A hand item `isEquippedNoSprint()` (`canSprint`).
    pub fn hand_item_equipped_no_sprint(&self, sm: &ScriptManager) -> bool {
        self.primary_hand_item().is_some_and(|i| is_equipped_no_sprint(sm, i))
            || self.secondary_hand_item().is_some_and(|i| is_equipped_no_sprint(sm, i))
    }

    /// The `TriggerSneezeCough` tissue: uses one use of a `Tissue`,
    /// `ToiletPaper` or `MUFFLE_SNEEZE` item in the primary hand, else in
    /// the secondary hand (`setCurrentUses(uses - 1)`, then `Use()` at 0).
    /// Whether one was used.
    pub fn use_tissue(&mut self, sm: &ScriptManager) -> bool {
        let is_tissue = |i: &InventoryItem| {
            i.get_type() == "Tissue" || i.get_type() == "ToiletPaper" || has_tag(sm, i, "MUFFLE_SNEEZE")
        };
        let primary = self.primary_hand_item().filter(|i| is_tissue(i)).map(|i| i.id);
        let target = match primary {
            Some(id) => Some(id),
            None => self
                .secondary_hand_item()
                .filter(|i| is_tissue(i) && i.get_current_uses() > 0)
                .map(|i| i.id),
        };
        let Some(item) = target.and_then(|id| self.item_mut(id)) else { return false };
        if item.get_current_uses() <= 0 {
            return false;
        }
        item.set_current_uses(item.get_current_uses() - 1);
        if item.get_current_uses() <= 0 {
            item.use_();
        }
        true
    }
}

impl<I> GameCharacter<I> {
    /// `getInventory().getCapacityWeight()`.
    pub fn capacity_weight(&self, sm: &ScriptManager, ctx: &dyn SimContext) -> f32 {
        self.items.inventory_weight(sm, ctx)
    }
}
