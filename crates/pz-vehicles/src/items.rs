//! What the vehicle code reads from and writes to `InventoryItem`s: the
//! vehicle-part values `Item.InstanceItem` copies from the script
//! (`brakeForce`, `wheelFriction`, ...), drainable uses
//! (`DrainableComboItem`), condition and weights.

use pz_inventory::item::ItemKind;
use pz_inventory::{InventoryItem, ItemClass, ItemContainer, SavedItem};
use std::cell::RefCell;
use std::rc::Rc;

/// A shared item (the original's object identity; the same type as
/// pz-lua-api's `bind_items::ItemRef`).
pub type ItemRef = Rc<RefCell<InventoryItem>>;

/// The vehicle-part values of an item script (`InventoryItem` fields set
/// by `Item.InstanceItem`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemStats {
    pub brake_force: f32,
    pub wheel_friction: f32,
    pub suspension_damping: f32,
    pub suspension_compression: f32,
    pub engine_loudness: f32,
    pub durability: f32,
    pub chance_to_spawn_damaged: i32,
    pub condition_lower_normal: f32,
    pub condition_lower_offroad: f32,
    pub condition_affects_capacity: bool,
    /// `ReplaceOnDeplete` / `WeightEmpty` (`DrainableComboItem.updateWeight`).
    pub weight_empty: f32,
}

impl ItemStats {
    /// The values of a pz-scripts item.
    pub fn from_script(item: &pz_scripts::objects::items::Item) -> Self {
        ItemStats {
            brake_force: item.brake_force,
            wheel_friction: item.wheel_friction,
            suspension_damping: item.suspension_damping,
            suspension_compression: item.suspension_compression,
            engine_loudness: item.engine_loudness,
            durability: item.durability,
            chance_to_spawn_damaged: item.chance_to_spawn_damaged,
            condition_lower_normal: item.condition_lower_normal,
            condition_lower_offroad: item.condition_lower_offroad,
            condition_affects_capacity: item.condition_affects_capacity,
            weight_empty: item.weight_empty,
        }
    }
}

/// The vehicle-part values of an item.
pub fn stats(item: &InventoryItem) -> ItemStats {
    crate::env::items().stats(&item.get_full_type())
}

/// `instanceof DrainableComboItem`.
pub fn is_drainable(item: &InventoryItem) -> bool {
    item.template.class == ItemClass::Drainable
}

/// `useDelta` (from the script).
pub fn use_delta(item: &InventoryItem) -> f32 {
    item.template.use_delta
}

/// `getMaxUses()`.
pub fn max_uses(item: &InventoryItem) -> i32 {
    if is_drainable(item) {
        (1.0f32 / use_delta(item)).floor() as i32
    } else {
        1
    }
}

/// `getCurrentUsesFloat()`.
pub fn current_uses_float(item: &InventoryItem) -> f32 {
    if is_drainable(item) {
        item.uses as f32 * use_delta(item)
    } else {
        item.uses as f32 / max_uses(item) as f32
    }
}

/// `setCurrentUsesFloat(f)` / `setUsedDelta(f)`.
pub fn set_current_uses_float(item: &mut InventoryItem, f: f32) {
    let f = crate::jmath::clamp(f, 0.0, 1.0);
    item.uses = crate::jmath::java_round_f(f / use_delta(item));
    if is_drainable(item) {
        update_weight(item);
    }
}

/// `DrainableComboItem.updateWeight` for items with `WeightEmpty`
/// (`ReplaceOnDeplete` items are not vehicle parts).
pub fn update_weight(item: &mut InventoryItem) {
    let s = stats(item);
    if s.weight_empty != 0.0 {
        item.custom_weight = true;
        let full = item.template.actual_weight;
        item.actual_weight =
            ((full - s.weight_empty) * current_uses_float(item) + s.weight_empty).max(0.0);
    }
}

/// `InventoryItem.setCondition(condition, doSound)` (without sounds).
pub fn set_condition(item: &mut InventoryItem, condition: i32) {
    let c = condition.max(0).min(item.condition_max);
    item.condition = c;
}

/// `getWeight()`.
pub fn weight(item: &InventoryItem) -> f32 {
    item.weight.max(0.0)
}

/// `getActualWeight()`: 0 for items whose display name is their full type.
pub fn actual_weight(item: &InventoryItem) -> f32 {
    if item.name == item.get_full_type() {
        return 0.0;
    }
    item.actual_weight.max(0.0)
}

/// `getUnequippedWeight()` (`getActualWeight() + getContentsWeight()`;
/// fluid and ammo contents are not modelled).
pub fn unequipped_weight(item: &InventoryItem) -> f32 {
    let contents = match &item.kind {
        ItemKind::Container(c) => contents_weight(&c.container),
        _ => 0.0,
    };
    actual_weight(item) + contents
}

/// `ItemContainer.getContentsWeight()` (raw item records weigh nothing).
pub fn contents_weight(c: &ItemContainer) -> f32 {
    let mut total = 0.0f32;
    for i in &c.items {
        if let SavedItem::Item(item) = i {
            total += unequipped_weight(item);
        }
    }
    total
}

/// `getCapacityWeight()` of a vehicle part container (its parent is the
/// vehicle).
pub fn capacity_weight(c: &ItemContainer) -> f32 {
    contents_weight(c)
}

/// `ItemContainer.getCapacity()` of a vehicle part container.
pub fn container_capacity(c: &ItemContainer, occupied_seat: bool) -> i32 {
    let mut capacity = c.capacity;
    if occupied_seat {
        capacity /= 4;
    }
    capacity.min(1000)
}
