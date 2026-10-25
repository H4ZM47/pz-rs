//! What the vehicle code reads from and writes to `InventoryItem`s: the
//! vehicle-part values `Item.InstanceItem` copies from the script
//! (`brakeForce`, `wheelFriction`, ...), drainable uses
//! (`DrainableComboItem`), condition and weights.

use pz_inventory::context::NoOwner;
use pz_inventory::template::VehiclePartStats;
use pz_inventory::{InventoryItem, ItemClass, ItemContainer};
use std::cell::RefCell;
use std::rc::Rc;

/// A shared item (the original's object identity; the same type as
/// pz-lua-api's `bind_items::ItemRef`).
pub type ItemRef = Rc<RefCell<InventoryItem>>;

/// The vehicle-part values `Item.InstanceItem` copied from the script
/// (`brakeForce`, `wheelFriction`, ...).
pub fn stats(item: &InventoryItem) -> VehiclePartStats {
    item.vehicle_part.clone()
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
    let empty = item.template.weight_empty;
    if empty != 0.0 {
        item.custom_weight = true;
        let full = item.template.actual_weight;
        item.actual_weight = ((full - empty) * current_uses_float(item) + empty).max(0.0);
    }
}

/// `InventoryItem.setCondition(condition, doSound)` (without sounds).
pub fn set_condition(item: &mut InventoryItem, condition: i32) {
    let c = condition.max(0).min(item.condition_max);
    item.condition = c;
}

/// `getWeight()`.
pub fn weight(item: &InventoryItem) -> f32 {
    item.get_weight_with(crate::env::items().context())
}

/// `getUnequippedWeight()` (the fluid, ammo and bag contents included).
pub fn unequipped_weight(item: &InventoryItem) -> f32 {
    item.get_unequipped_weight(crate::env::items().context(), &NoOwner)
}

/// `getCapacityWeight()` of a vehicle part container (its parent is the
/// vehicle, so it is the contents weight).
pub fn capacity_weight(c: &ItemContainer) -> f32 {
    c.get_contents_weight_with(crate::env::items().context())
}

/// `ItemContainer.getCapacity()` of a vehicle part container.
pub fn container_capacity(c: &ItemContainer, occupied_seat: bool) -> i32 {
    let mut capacity = c.capacity;
    if occupied_seat {
        capacity /= 4;
    }
    capacity.min(1000)
}
