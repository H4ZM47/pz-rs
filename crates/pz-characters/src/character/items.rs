//! The character's items (`IsoGameCharacter.inventory`, the hand items and
//! `wornItems`) and the `IsoGameCharacter` methods the simulation reads
//! them through: clothing defense, spikes, dirty and bloody clothing, the
//! corpse sickness defense, the inventory weight and `getItemVisuals`.
//! `zombie.characterTextures.BloodClothingType`'s covered parts are here
//! too.
//!
//! Items are `pz-inventory` [`InventoryItem`]s, shared ([`ItemRef`]) in
//! the character's shared container ([`pz_inventory::ContainerRef`], the
//! object Lua's `getInventory()` returns); the hand and worn items are the
//! ids of items in it (the original holds the same objects in both
//! places).
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

use pz_inventory::shared::{self, ContainerRef, ItemRef};
use pz_inventory::visual::human::HumanStyles;
use pz_inventory::{InventoryItem, ItemKind, SharedContainer};
use pz_scripts::objects::items::{get_item, Item, ItemBodyLocation, ItemTag};
use pz_scripts::registry::RegistryKey;
use pz_scripts::{ScriptManager, ScriptResult};

use super::context::SimContext;
use super::item_owner::CharacterItemContext;
use super::GameCharacter;
use crate::worn_items::{BodyLocationGroup, BodyLocationGroupRef, WornItems};

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
        ItemKind::Container(c) => c.container.borrow().get_contents_weight(),
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
#[derive(Debug)]
pub struct CharacterItems {
    /// `inventory`: shared with Lua (`getInventory()`), so both see the
    /// same container and item objects.
    pub inventory: ContainerRef,
    /// `primaryHandItem`: the id of an item in `inventory`.
    pub primary_hand_item: Option<i32>,
    /// `secondaryHandItem`.
    pub secondary_hand_item: Option<i32>,
    /// `wornItems`: the ids of items in `inventory` with their body
    /// locations, in the group's location order. `None` until
    /// `initWornItems` (the constructors' [`GameCharacter::init_worn_items`])
    /// or a player load.
    pub worn_items: Option<WornItems<i32>>,
}

impl Default for CharacterItems {
    fn default() -> Self {
        CharacterItems {
            // `new ItemContainer()`, explored (the `IsoGameCharacter`
            // constructor).
            inventory: SharedContainer { explored: true, ..Default::default() }.into_ref(),
            primary_hand_item: None,
            secondary_hand_item: None,
            worn_items: None,
        }
    }
}

impl CharacterItems {
    /// The items directly in the inventory, in order.
    pub fn inventory_items(&self) -> Vec<ItemRef> {
        self.inventory.borrow().items.clone()
    }

    /// The item `id` in the inventory.
    pub fn item(&self, id: i32) -> Option<ItemRef> {
        self.inventory.borrow().items.iter().find(|i| i.borrow().id == id).cloned()
    }

    /// Adds `item` at the end of the inventory (the list part of
    /// `AddItem`).
    pub fn add_item(&self, item: InventoryItem) -> ItemRef {
        let r = shared::new_item(item);
        shared::add_item(&self.inventory, &r);
        r
    }

    /// `getPrimaryHandItem()`.
    pub fn primary_hand_item(&self) -> Option<ItemRef> {
        self.primary_hand_item.and_then(|id| self.item(id))
    }

    /// `getSecondaryHandItem()`.
    pub fn secondary_hand_item(&self) -> Option<ItemRef> {
        self.secondary_hand_item.and_then(|id| self.item(id))
    }

    /// `wornItems.getLocation(item)` for the worn item `id`.
    pub fn worn_location(&self, id: i32) -> Option<Rc<RegistryKey>> {
        self.worn_items.as_ref().and_then(|w| w.location(&id))
    }

    /// The ids of the worn items, in order (none without `wornItems`).
    pub fn worn_ids(&self) -> Vec<i32> {
        self.worn_items.as_ref().map_or_else(Vec::new, |w| w.items().iter().map(|w| w.item).collect())
    }

    /// `wornItems.contains(item)` for the item `id` (false without
    /// `wornItems`).
    pub fn is_worn(&self, id: i32) -> bool {
        self.worn_items.as_ref().is_some_and(|w| w.contains(&id))
    }

    /// `getWornItems()` items, in order (the worn ids that are items of the
    /// inventory).
    pub fn worn(&self) -> impl Iterator<Item = ItemRef> + use<'_> {
        self.worn_ids().into_iter().filter_map(|id| self.item(id))
    }

    /// Test support: makes `wornItems` the items `ids`, in order, each at a
    /// location of its own in a new body location group (for code that
    /// reads the worn items but not their locations).
    pub fn wear_in_order(&mut self, ids: &[i32]) {
        let mut reg = pz_scripts::registry::Registry::new("item_body_location");
        let mut group = BodyLocationGroup::new("Test").expect("non-empty id");
        let keys: Vec<_> = (0..ids.len())
            .map(|i| Some(reg.register_id(&format!("test:worn{i}"), false).expect("new id")))
            .collect();
        for k in &keys {
            group.get_or_create_location(k.clone()).expect("location");
        }
        let mut worn = WornItems::new(Rc::new(RefCell::new(group)));
        for (k, &id) in keys.into_iter().zip(ids) {
            worn.set_item(k, Some(id)).expect("known location");
        }
        self.worn_items = Some(worn);
    }

    /// `getItemVisuals(itemVisuals)`: the worn items that have a visual
    /// (`WornItems.getItemVisuals`), as their items.
    pub fn item_visuals(&self) -> Vec<ItemRef> {
        self.worn().filter(|i| visual(&i.borrow()).is_some()).collect()
    }

    /// Ids of [`Self::item_visuals`].
    pub fn item_visual_ids(&self) -> Vec<i32> {
        self.item_visuals().iter().map(|i| i.borrow().id).collect()
    }

    /// `isEquipped(item)`: worn or in a hand.
    pub fn is_equipped(&self, id: i32) -> bool {
        self.is_worn(id) || self.primary_hand_item == Some(id) || self.secondary_hand_item == Some(id)
    }

    /// `bodyPartHasTag(part, tag)`.
    fn body_part_has_tag(&self, sm: &ScriptManager, part: i32, field: &str) -> bool {
        for item in self.item_visuals().into_iter().rev() {
            let item = &*item.borrow();
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
            let item = &*item.borrow();
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
            let item = &*item.borrow();
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

    /// `getCorpseSicknessDefense(rate, drain)`: an active SCBA with air,
    /// else the best worn defense, a gas mask with a filter (drained by
    /// `rate * corpseSicknessFilterDrainMultiplier` when `drain`).
    pub fn corpse_sickness_defense(&mut self, sm: &ScriptManager, ctx: &dyn SimContext, rate: f32, drain: bool) -> ScriptResult<f32> {
        let g = ctx.zomboid_globals();
        let ictx = CharacterItemContext::new(sm, ctx);
        let mut defense = 0.0f32;
        for id in self.worn_ids() {
            let Some(item) = self.item(id) else { continue };
            let item = &mut *item.borrow_mut();
            if is_clothing(item) {
                if has_tag(sm, item, "SCBA") && item.activated && item.has_tank() && item.get_used_delta() > 0.0 {
                    return Ok(g.standard_gas_mask_defense);
                }
                if item.get_corpse_sickness_defense() > defense {
                    defense = item.get_corpse_sickness_defense();
                }
                let mask = ["GAS_MASK", "RESPIRATOR", "IMPROVISED_GAS_MASK"].iter().any(|t| has_tag(sm, item, t));
                if mask && item.has_filter() && item.get_used_delta() > 0.0 {
                    let d = if has_tag(sm, item, "IMPROVISED_GAS_MASK") {
                        g.improvised_gas_mask_defense
                    } else {
                        g.standard_gas_mask_defense
                    };
                    defense = defense.max(d);
                    if drain && rate > 0.0 {
                        let r = (rate as f64 * g.corpse_sickness_filter_drain_multiplier) as f32;
                        item.drain_gas_mask(r, &ictx).map_err(|_| crate::npe())?;
                    }
                }
            }
            if defense >= g.standard_gas_mask_defense {
                return Ok(g.standard_gas_mask_defense);
            }
        }
        Ok(defense)
    }

    /// `isProtectedFromToxic(false)`: an active SCBA with air or a gas mask
    /// with a filter. (The original's `getUsedDelta` also stores 0 in
    /// the mod data when it is missing; that is not done here.)
    pub fn is_protected_from_toxic(&self, sm: &ScriptManager) -> bool {
        for item in self.worn() {
            let item = &*item.borrow();
            if !is_clothing(item) {
                continue;
            }
            if has_tag(sm, item, "SCBA") && item.activated && item.has_tank() && item.used_delta() > 0.0 {
                return true;
            }
            let mask = ["GAS_MASK", "RESPIRATOR", "IMPROVISED_GAS_MASK"].iter().any(|t| has_tag(sm, item, t));
            if mask && item.has_filter() && item.used_delta() > 0.0 {
                return true;
            }
        }
        false
    }

    /// `isProtectedFromToxic(true)`: as [`Self::is_protected_from_toxic`]
    /// (with `getUsedDelta` storing a missing 0), draining the protecting
    /// gas mask's filter by 0.01.
    pub fn is_protected_from_toxic_drain(&mut self, sm: &ScriptManager, ctx: &dyn SimContext) -> ScriptResult<bool> {
        let ictx = CharacterItemContext::new(sm, ctx);
        for id in self.worn_ids() {
            let Some(item) = self.item(id) else { continue };
            let item = &mut *item.borrow_mut();
            if !is_clothing(item) {
                continue;
            }
            if has_tag(sm, item, "SCBA") && item.activated && item.has_tank() && item.get_used_delta() > 0.0 {
                return Ok(true);
            }
            let mask = ["GAS_MASK", "RESPIRATOR", "IMPROVISED_GAS_MASK"].iter().any(|t| has_tag(sm, item, t));
            if !mask || !item.has_filter() || item.get_used_delta() <= 0.0 {
                continue;
            }
            item.drain_gas_mask(0.01, &ictx).map_err(|_| crate::npe())?;
            return Ok(true);
        }
        Ok(false)
    }

    /// A hand item `isEquippedNoSprint()` (`canSprint`).
    pub fn hand_item_equipped_no_sprint(&self, sm: &ScriptManager) -> bool {
        self.primary_hand_item().is_some_and(|i| is_equipped_no_sprint(sm, &i.borrow()))
            || self.secondary_hand_item().is_some_and(|i| is_equipped_no_sprint(sm, &i.borrow()))
    }
}

impl<I> GameCharacter<I> {
    /// The `TriggerSneezeCough` tissue: uses one use of a `Tissue`,
    /// `ToiletPaper` or `MUFFLE_SNEEZE` item in the primary hand, else in
    /// the secondary hand (`setCurrentUses(uses - 1)`, then `Use()` at 0,
    /// which takes a used-up tissue out of the hand and the inventory).
    /// Whether one was used.
    pub fn use_tissue(&mut self, sm: &ScriptManager, ctx: &dyn SimContext) -> bool {
        let is_tissue = |i: &InventoryItem| {
            i.get_type() == "Tissue" || i.get_type() == "ToiletPaper" || has_tag(sm, i, "MUFFLE_SNEEZE")
        };
        let items = &self.items;
        let target = match items.primary_hand_item().filter(|i| is_tissue(&i.borrow())) {
            Some(i) => i,
            None => match items.secondary_hand_item().filter(|i| is_tissue(&i.borrow())) {
                Some(i) => i,
                None => return false,
            },
        };
        let target = target.borrow();
        if target.get_current_uses() <= 0 {
            return false;
        }
        let id = target.id;
        drop(target);
        let ictx = CharacterItemContext::new(sm, ctx);
        let Some(uses) = self.with_held_item(id, |item, _| {
            item.set_current_uses_with(item.get_current_uses() - 1, Some(&ictx));
            item.get_current_uses()
        }) else {
            return false;
        };
        if uses <= 0 {
            self.use_held_item(sm, ctx, id, false, false, false);
        }
        true
    }

    /// `setPrimaryHandItem(item)` with the item `id` (`None` for `null`):
    /// the weapon in use becomes the item if it is a `HandWeapon` (else
    /// none), then the hand and the `Weapon` variable are set. Returns
    /// whether it changed (the caller fires `OnEquipPrimary`). The equip
    /// parent, animal, ballistics cache and hand models are not modelled.
    pub fn set_primary_hand_item(&mut self, sm: &ScriptManager, id: Option<i32>) -> bool {
        if self.items.primary_hand_item == id {
            return false;
        }
        let weapon = id.filter(|&i| self.items.item(i).is_some_and(|i| is_hand_weapon(&i.borrow())));
        self.set_use_hand_weapon(weapon);
        self.items.primary_hand_item = id;
        let w = self.weapon_type(sm);
        self.set_var_str("Weapon", w.type_name());
        true
    }

    /// `setSecondaryHandItem(item)`: as [`Self::set_primary_hand_item`]
    /// without the weapon in use (`OnEquipSecondary`).
    pub fn set_secondary_hand_item(&mut self, sm: &ScriptManager, id: Option<i32>) -> bool {
        if self.items.secondary_hand_item == id {
            return false;
        }
        self.items.secondary_hand_item = id;
        let w = self.weapon_type(sm);
        self.set_var_str("Weapon", w.type_name());
        true
    }

    /// `removeFromHands(item)`: `setPrimaryHandItem(null)` and
    /// `setSecondaryHandItem(null)` for the hands holding the item `id`.
    /// Returns which hands changed (primary, secondary).
    pub fn remove_from_hands(&mut self, sm: &ScriptManager, id: i32) -> (bool, bool) {
        let primary = self.items.primary_hand_item == Some(id) && self.set_primary_hand_item(sm, None);
        let secondary = self.items.secondary_hand_item == Some(id) && self.set_secondary_hand_item(sm, None);
        (primary, secondary)
    }

    /// `getInventory().getCapacityWeight()`: `getInventoryWeight()` (the
    /// unlimited-carry and ghost-mode cheats are not modelled).
    pub fn capacity_weight(&self, sm: &ScriptManager, ctx: &dyn SimContext) -> f32 {
        self.inventory_weight(sm, ctx)
    }

    /// `getInventoryWeight()`: hotbar items not equipped, equipped (or
    /// fake-equipped) items and the rest, each with its weight rule.
    pub fn inventory_weight(&self, sm: &ScriptManager, ctx: &dyn SimContext) -> f32 {
        let ictx = CharacterItemContext::new(sm, ctx);
        let mut total = 0.0f32;
        for item in self.items.inventory_items() {
            let item = &*item.borrow();
            let equipped = self.items.is_equipped(item.id);
            if item.attached_slot > -1 && !equipped {
                total += item.get_hotbar_equipped_weight(&ictx, self);
                continue;
            }
            if equipped || item.is_fake_equipped(self) {
                total += item.get_equipped_weight(&ictx, self);
                continue;
            }
            total += item.get_unequipped_weight(&ictx, self);
        }
        total
    }
}

/// `ItemBodyLocation.<field>` as a worn item location.
fn body_location_key(sm: &ScriptManager, field: &str) -> Option<Rc<RegistryKey>> {
    ItemBodyLocation::constant(sm, field)
}

/// The `getClothingItem_*` / `setClothingItem_*` locations
/// (`ItemBodyLocation` fields), by suffix.
pub const CLOTHING_ITEM_LOCATIONS: [(&str, &str); 6] =
    [("Head", "HAT"), ("Torso", "TSHIRT"), ("Back", "BACK"), ("Hands", "HANDS"), ("Legs", "PANTS"), ("Feet", "SHOES")];

impl<I> GameCharacter<I> {
    /// `initWornItems(name)` with the group `BodyLocations.getGroup(name)`:
    /// new, empty worn items, then `onWornItemsChanged()`.
    pub fn init_worn_items(&mut self, sm: &ScriptManager, group: BodyLocationGroupRef) {
        self.items.worn_items = Some(WornItems::new(group));
        self.on_worn_items_changed(sm);
    }

    /// `wornItems`, which the original dereferences
    /// (`NullPointerException` without).
    fn worn_items_or_npe(&self) -> ScriptResult<&WornItems<i32>> {
        self.items.worn_items.as_ref().ok_or_else(crate::npe)
    }

    /// `getWornItem(location)`: the id of the item worn there.
    pub fn worn_item(&self, location: Option<&Rc<RegistryKey>>) -> ScriptResult<Option<i32>> {
        self.worn_items_or_npe()?.item(location)
    }

    /// `getClothingItem_<suffix>()` (see [`CLOTHING_ITEM_LOCATIONS`]).
    pub fn clothing_item(&self, sm: &ScriptManager, field: &str) -> ScriptResult<Option<i32>> {
        self.worn_item(body_location_key(sm, field).as_ref())
    }

    /// `isEquippedClothing(item)`: `wornItems.contains(item)`.
    pub fn is_equipped_clothing(&self, id: Option<i32>) -> ScriptResult<bool> {
        let worn = self.worn_items_or_npe()?;
        Ok(id.is_some_and(|id| worn.contains(&id)))
    }

    /// `hasWornTag(tag)` for `ItemTag.<field>` (the worn ids that are not
    /// items of the inventory are skipped).
    pub fn has_worn_tag(&self, sm: &ScriptManager, field: &str) -> bool {
        self.items.worn().any(|i| has_tag(sm, &i.borrow(), field))
    }

    /// `onWornItemsChanged()`: whether the clothes allow ragdolling (no
    /// worn `NO_RAGDOLL` item). The server's `OnClothingUpdated()` and the
    /// player's model reset and shoe sound parameter are not modelled.
    pub fn on_worn_items_changed(&mut self, sm: &ScriptManager) {
        self.worn_clothing_can_ragdoll = !self.has_worn_tag(sm, "NO_RAGDOLL");
    }

    /// `setWornItem(location, item, forceDropTooHeavy)` with the item `id`
    /// (`None` for `null`): nothing when the same item is already there,
    /// else `WornItems.setItem` (replacing the item at a single-item
    /// location and dropping the items at exclusive locations), a
    /// player's mohawk flattened under a hat, the clothing wetness marked
    /// changed and `onWornItemsChanged()`. Returns whether it changed.
    /// `styles` are the hair style tables `getHairModel()` checks the
    /// stored model against (`None`: the stored model is read as it is).
    ///
    /// STUB: the cell's process-item lists, the model reset, the network
    /// syncs and `forceDropTooHeavy` (an `IsoPlayer` with no room for the
    /// replaced item drops it on the square: TODO(world)) are not modelled.
    pub fn set_worn_item(
        &mut self,
        sm: &ScriptManager,
        styles: Option<&dyn HumanStyles>,
        location: Option<Rc<RegistryKey>>,
        id: Option<i32>,
        force_drop_too_heavy: bool,
    ) -> ScriptResult<bool> {
        let _ = force_drop_too_heavy;
        let current = self.worn_item(location.as_ref())?;
        if id == current {
            return Ok(false);
        }
        let worn = self.items.worn_items.as_mut().ok_or_else(crate::npe)?;
        worn.set_item(location.clone(), id)?;
        if self.is_player() {
            let hat = |f| body_location_key(sm, f).is_some_and(|k| location.as_ref().is_some_and(|l| l.location == k.location));
            if let Some(mut hv) = self.human_visual.take() {
                let hair = match styles {
                    Some(styles) => hv.get_hair_model(self, styles),
                    None => hv.hair_model.clone(),
                };
                if hair.is_some_and(|h| h.contains("Mohawk")) && (hat("HAT") || hat("FULL_HAT")) {
                    hv.set_hair_model(Some("MohawkFlat"));
                }
                self.human_visual = Some(hv);
            }
        }
        if let Some(w) = &mut self.clothing_wetness {
            w.changed = true;
        }
        self.on_worn_items_changed(sm);
        Ok(true)
    }

    /// `removeWornItem(item, forceDropTooHeavy)`:
    /// `setWornItem(wornItems.getLocation(item), null, forceDropTooHeavy)`
    /// (nothing for an item that is not worn).
    pub fn remove_worn_item(
        &mut self,
        sm: &ScriptManager,
        styles: Option<&dyn HumanStyles>,
        id: Option<i32>,
        force_drop_too_heavy: bool,
    ) -> ScriptResult<bool> {
        let worn = self.worn_items_or_npe()?;
        let location = id.and_then(|id| worn.location(&id));
        self.set_worn_item(sm, styles, location, None, force_drop_too_heavy)
    }

    /// `clearWornItems()`: nothing without `wornItems`.
    pub fn clear_worn_items(&mut self, sm: &ScriptManager) {
        let Some(w) = &mut self.items.worn_items else { return };
        w.clear();
        if let Some(w) = &mut self.clothing_wetness {
            w.changed = true;
        }
        self.on_worn_items_changed(sm);
    }
}

/// `ItemVisual.setHole(part)`.
fn set_visual_hole(item: &mut InventoryItem, part: usize) {
    if let Some(v) = item.visual.as_mut() {
        let a = v.arrays[2].get_or_insert_with(|| vec![0; blood_part::MAX]);
        if part < a.len() {
            a[part] = -1;
        }
    }
}

/// `InventoryItem.setCondition(c)` (clamped; the sounds are the item's).
fn set_condition(item: &mut InventoryItem, c: i32) {
    item.condition = c.max(0).min(item.condition_max);
}

impl CharacterItems {
    /// `BloodClothingType.addHole(part, humanVisual, itemVisuals,
    /// allLayers)` on the worn items (the human visual's own hole is the
    /// renderer's). Returns `addedHole`.
    pub fn add_hole(&mut self, sm: &ScriptManager, ctx: &dyn SimContext, part: usize, all_layers: bool) -> bool {
        let mut added = false;
        for id in self.item_visual_ids().into_iter().rev() {
            let Some(item_ref) = self.item(id) else { continue };
            let item = &mut *item_ref.borrow_mut();
            let Some(script) = script_item(sm, item) else { continue };
            if item.condition <= 0 {
                continue;
            }
            let Some(parts) = covered_parts(sm, item) else { continue };
            if !parts.contains(&part) || visual_hole(item, part) != 0.0 {
                continue;
            }
            let clothing = is_clothing(item);
            let (can_holes, lower_chance) = {
                let s = script.borrow();
                (s.can_have_holes, s.condition_lower_chance)
            };
            let n_parts = parts.len() as i32;
            if clothing && can_holes {
                set_visual_hole(item, part);
                if let ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) = &mut item.kind {
                    if let Some(p) = &mut c.patches {
                        p.retain(|(i, _)| *i as usize != part);
                    }
                }
                let loss = 1.max(item.condition_max / n_parts.max(1)) as f32;
                let c = (item.condition as f32 - loss) as i32;
                set_condition(item, c);
                added = true;
            } else if clothing && !can_holes && super::body_part::next_bool(ctx, lower_chance) {
                let c = item.condition - 1;
                set_condition(item, c);
            } else if !clothing && can_holes {
                set_visual_hole(item, part);
                added = true;
            }
            if !all_layers {
                break;
            }
        }
        added
    }

    /// `addHoleFromZombieAttacks(part, scratch)`: the outer clothing on the
    /// part may tear.
    pub fn add_hole_from_zombie_attacks(&mut self, sm: &ScriptManager, ctx: &dyn SimContext, part: usize, scratch: bool) -> bool {
        let mut hit = None;
        for id in self.item_visual_ids().into_iter().rev() {
            let Some(item) = self.item(id) else { continue };
            let item = &*item.borrow();
            if script_item(sm, item).is_none() {
                continue;
            }
            let Some(parts) = covered_parts(sm, item) else { continue };
            if parts.contains(&part) {
                hit = Some(id);
                break;
            }
        }
        let mut base_def = 0.0f32;
        if let Some(item) = hit.and_then(|id| self.item(id)) {
            let item = &*item.borrow();
            if is_clothing(item) {
                base_def = 30.0f32.max(100.0 - def_for_part(sm, item, part, !scratch, false) / 1.5);
            }
        }
        if (ctx.rand_next(100) as f32) < base_def {
            self.add_hole(sm, ctx, part, false);
            return true;
        }
        false
    }

    /// `helmetFallFromWornItems(hitHead)`: worn clothing with a chance to
    /// fall comes off (the original's loop skips the item after a removed
    /// one). Returns the removed items (the world makes them fall; the
    /// caller runs `onWornItemsChanged()` when there are any).
    pub fn helmet_fall_from_worn_items(&mut self, sm: &ScriptManager, ctx: &dyn SimContext, hit_head: bool) -> Vec<ItemRef> {
        let mut removed = Vec::new();
        let mut i = 0;
        while let Some(id) = self.worn_items.as_ref().and_then(|w| w.item_by_index(i)) {
            i += 1;
            let Some(item_ref) = self.item(id) else { continue };
            let item = &*item_ref.borrow();
            if !is_clothing(item) {
                continue;
            }
            let chance = script_item(sm, item).map_or(0, |s| s.borrow().chance_to_fall);
            let mut chance_to_fall = chance;
            if hit_head {
                chance_to_fall += 40;
            }
            if chance <= 0 || ctx.rand_next(100) > chance_to_fall {
                continue;
            }
            if shared::remove_item(&self.inventory, &item_ref) {
                removed.push(item_ref.clone());
            }
            if let Some(w) = &mut self.worn_items {
                w.remove(&id);
            }
            // `i` keeps pointing past the removed slot.
        }
        removed
    }
}
