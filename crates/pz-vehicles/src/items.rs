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
    item.get_max_uses()
}

/// `getCurrentUsesFloat()`.
pub fn current_uses_float(item: &InventoryItem) -> f32 {
    item.get_current_uses_float()
}

/// `setCurrentUsesFloat(f)` / `setUsedDelta(f)` (with
/// `DrainableComboItem.updateWeight`).
pub fn set_current_uses_float(item: &mut InventoryItem, f: f32) {
    item.set_current_uses_float(f, Some(crate::env::items().context()));
}

/// `DrainableComboItem.updateWeight`.
pub fn update_weight(item: &mut InventoryItem) {
    item.update_weight(Some(crate::env::items().context()));
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
