//! The actual nullable `InventoryItem.worldItem` field.
//!
//! This owns one strong reference directly to the complete world allocation.
//! Inventory has no dependency on the world crate: the caller validates the
//! erased allocation's actual native type before assigning it. This is never
//! a locator, current-square lookup, weak reference, or copied world body.
//!
//! Native get/set access only the field. They neither transfer components nor
//! edit world/container lists. Native item save omits it and item load leaves
//! it unchanged. Rust archive copies deliberately clear it and Rust data
//! equality ignores it; neither policy implements a native clone operation.

use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::{EntityOwner, RetiredComponents};

use crate::shared::{self, ItemRef};
use crate::InventoryItem;

/// Runtime-only field whose trait policy prevents derived InventoryItem
/// archive clones from accidentally retaining the source world graph.
#[derive(Debug, Default)]
pub(crate) struct WorldItemSlot(Option<EntityOwner>);

impl Clone for WorldItemSlot {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for WorldItemSlot {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl InventoryItem {
    /// Borrow the one physical owning edge for GC without cloning or reading
    /// the world allocation's body. None alone means Java null.
    pub fn world_item_slot(&self) -> Option<&EntityOwner> {
        self.world_item.0.as_ref()
    }

    /// Native field assignment on an already borrowed item. The caller must
    /// have validated the incoming actual world allocation before this phase.
    /// Retain `retired` until every outer item/world/sprite guard has ended,
    /// then notify shared::observe_item after releasing the item guard.
    pub fn replace_world_item(
        &mut self,
        new: Option<&EntityOwner>,
        retired: &mut RetiredComponents,
    ) {
        let previous = std::mem::replace(&mut self.world_item.0, new.cloned());
        if let Some(previous) = previous {
            retired.push_owner(previous);
        }
    }

    /// Collector-only physical detach. It invokes no native lifecycle/reset
    /// operation and does not mutate the retained world object's item field.
    /// The caller owns retirement of the returned pin outside all guards.
    pub fn take_world_item_for_gc(&mut self) -> Option<EntityOwner> {
        self.world_item.0.take()
    }
}

/// Pointer-only getter. Holding the world object's body is harmless; a held
/// mutable item body is an explicit host borrow failure rather than null.
pub fn get_world_item_ref(item: &ItemRef) -> HostResult<Option<EntityOwner>> {
    let result = (|| {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("inventory world-item getter"))?;
        Ok(body.world_item_slot().cloned())
    })();
    shared::observe_item(item);
    result
}

/// Field-only nullable setter over the actual ItemRef. Borrowed incoming
/// ownership is cloned into the retirement sink before attempting mutation,
/// so even a rejected input cannot finally drop under an outer foreign guard.
/// The world/Lua adapters validate the incoming erased type; an invalid type
/// is a host capability error there, never an implicit null here.
pub fn set_world_item_ref(item: &ItemRef, new: Option<&EntityOwner>) -> HostResult<()> {
    let mut retired = RetiredComponents::new();
    if let Some(new) = new {
        retired.push_owner(new.clone());
    }
    let result = (|| {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("inventory world-item setter"))?;
        body.replace_world_item(new, &mut retired);
        Ok(())
    })();
    crate::component_host::retire_components(retired);
    shared::observe_item(item);
    result
}

#[cfg(test)]
mod tests;
