//! The actual inventory owners used by `WornItems` visual conversion.
//!
//! The host invokes ordinary item factory/visual/condition/container APIs. Each
//! host call happens without a worn-list, group or item guard held here, so
//! condition events and other real callbacks can inspect these same owners.
use std::cell::RefCell;
use std::rc::Rc;

use crate::item::ItemKind;
use crate::shared::{ContainerRef, ItemRef};
use crate::visual::ItemVisual;
use pz_scripts::registry::RegistryKey;
use pz_scripts::{ScriptError, ScriptResult};

use super::{ItemIdentity, WornItems};

/// The actual worn membership, not a collection of copied items or local IDs.
pub type InventoryWornItemsRef = Rc<RefCell<WornItems<ItemRef>>>;

impl ItemIdentity for ItemRef {
    fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(self, other)
    }
}

/// Runtime dependencies of `setFromItemVisuals`/`addItemsToItemContainer`.
/// Implementations must use the supplied actual item; they must not replace it
/// with an archive copy, allocate fallback IDs, or invent missing visuals/keys.
pub trait WornItemsInventoryHost {
    /// Ordinary `InventoryItemFactory.CreateItem(type)`. Native null means
    /// `Ok(None)` and skips only this visual. Preserve ordinary factory hooks.
    fn create_item(&mut self, item_type: &str) -> ScriptResult<Option<ItemRef>>;

    /// Call actual `item.getVisual()`. If it is nonnull, copy the source into
    /// that visual, then `item.synchWithVisual()` using live assets/owner/RNG.
    /// A null visual skips both copying and synchronization.
    fn copy_visual_and_sync(&mut self, item: &ItemRef, visual: &ItemVisual) -> ScriptResult;

    /// Actual `getBodyLocation()` for Clothing, `canBeEquipped()` for EVERY
    /// other subclass. Return the existing registry key, including native null.
    fn body_location(
        &mut self,
        item: &ItemRef,
        clothing: bool,
    ) -> ScriptResult<Option<Rc<RegistryKey>>>;

    /// Actual `getVisual().getHolesNumber()`; null visual is an error here,
    /// unlike the optional visual in `setFromItemVisuals` above.
    fn get_holes_number(&mut self, item: &ItemRef) -> ScriptResult<i32>;

    /// Actual `setConditionNoSound`, including any real OnBreak outcome.
    /// Dispatch callbacks after releasing the item's mutable guard.
    fn set_condition_no_sound(&mut self, item: &ItemRef, condition: i32) -> ScriptResult;

    /// Actual `container.AddItem(item)` with this SAME owner, retaining native
    /// container/move behavior. No worn-list mutation is implied by insertion.
    fn add_item(&mut self, container: &ContainerRef, item: &ItemRef) -> ScriptResult;
}

fn borrowed(what: &str) -> ScriptError {
    ScriptError::new(format!("host borrow conflict: {what}"))
}

/// `WornItems.setFromItemVisuals`: clear before the first factory request,
/// create/convert in input order and use existing body-location replacement,
/// exclusion, identity and ordering rules. Errors preserve the completed prefix.
///
/// The input is the caller's visual sequence, not an item inventory snapshot.
/// Callers must not hold a RefCell guard backing that sequence across this API.
pub fn set_from_item_visuals<H: WornItemsInventoryHost + ?Sized>(
    worn: &InventoryWornItemsRef,
    visuals: &[ItemVisual],
    host: &mut H,
) -> ScriptResult {
    let retired = {
        let mut worn = worn.try_borrow_mut().map_err(|_| borrowed("WornItems"))?;
        std::mem::take(&mut worn.items)
    };
    drop(retired);
    for visual in visuals {
        let Some(item) = host.create_item(&visual.full_type)? else {
            continue;
        };
        host.copy_visual_and_sync(&item, visual)?;
        let clothing = {
            let item = item.try_borrow().map_err(|_| borrowed("InventoryItem"))?;
            matches!(
                item.kind,
                ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..)
            )
        };
        let location = host.body_location(&item, clothing)?;
        // setItem may evict an old owner, including on a partially mutating
        // error. These temporary actual aliases prevent its final Drop under
        // the worn/group guards; they are not authoritative parallel storage.
        let prior = worn
            .try_borrow()
            .map_err(|_| borrowed("WornItems"))?
            .items
            .clone();
        let result = {
            let mut worn = worn.try_borrow_mut().map_err(|_| borrowed("WornItems"))?;
            worn.set_item(location, Some(item.clone()))
        };
        drop(prior);
        result?;
    }
    Ok(())
}

/// `WornItems.addItemsToItemContainer`: read the current index/size each time,
/// compute condition from visual holes, set it without sound, then add the same
/// item to the actual container. No rollback or clearing on partial failure.
pub fn add_items_to_item_container<H: WornItemsInventoryHost + ?Sized>(
    worn: &InventoryWornItemsRef,
    container: &ContainerRef,
    host: &mut H,
) -> ScriptResult {
    let mut index = 0;
    loop {
        let item = {
            let worn = worn.try_borrow().map_err(|_| borrowed("WornItems"))?;
            let Some(entry) = worn.items.get(index) else {
                break;
            };
            entry.item.clone()
        };
        let holes = host.get_holes_number(&item)?;
        let max = item
            .try_borrow()
            .map_err(|_| borrowed("InventoryItem"))?
            .condition_max;
        let condition = max.wrapping_sub(holes.wrapping_mul(3));
        host.set_condition_no_sound(&item, condition)?;
        host.add_item(container, &item)?;
        index += 1;
    }
    Ok(())
}

#[cfg(test)]
#[path = "inventory_tests.rs"]
mod tests;
