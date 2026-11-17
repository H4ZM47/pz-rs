//! `IsoDeadBody.container` as a live shared container.
//!
//! A loaded or new corpse keeps its container as a codec value. When Lua
//! first reaches it (`getContainer`), the value moves into
//! [`CorpseInventory`], as `IsoMannequin.runtime_container` does. From then
//! on the shared container is the only state: the codec fields are empty,
//! and the worn and attached entries hold the item objects. Saving writes a
//! snapshot and maps the items' current positions through the order
//! `CompressIdenticalItems` saves them in, as for a mannequin.
//!
//! Limit: an entry whose saved index named a record the item codec keeps
//! raw has no item object, so it is saved as `-1`.

use std::rc::Rc;

use pz_inventory::archive::ArchiveCopyCheck;
use pz_inventory::io::{SaveError, SaveResult};
use pz_inventory::shared::{ContainerRef, ItemRef, SharedContainer};
use pz_inventory::SavedItem;

use crate::objects::ItemContainer;
use crate::save::visual::IsoDeadBody;

/// What saving writes for `container`, `wornItems` and `attachedItems`.
#[derive(Debug, PartialEq)]
pub struct InventorySnapshot {
    pub container: ItemContainer,
    pub worn: Vec<(String, i16)>,
    pub attached: Vec<(String, i16)>,
    /// The indices are positions in `container.items`, which the writer
    /// maps through the saved-item order; else they are saved indices.
    pub positions: bool,
}

/// The live `container`, `wornItems` and `attachedItems` of a corpse.
#[derive(Debug)]
pub struct CorpseInventory {
    pub container: ContainerRef,
    /// `wornItems`: body location and item (`None`: the saved index named no
    /// decoded item).
    pub worn: Vec<(String, Option<ItemRef>)>,
    /// `attachedItems`: location and item, as [`Self::worn`].
    pub attached: Vec<(String, Option<ItemRef>)>,
}

impl CorpseInventory {
    fn position(&self, item: &Option<ItemRef>) -> i16 {
        item.as_ref()
            .and_then(|item| self.container.borrow().items.iter().position(|i| Rc::ptr_eq(i, item)))
            .map_or(-1, |i| i as i16)
    }
}

impl Clone for CorpseInventory {
    /// Archive/value copy: a new container with copied items and no owner
    /// links; the entries name the copies.
    fn clone(&self) -> Self {
        let container = self.container.borrow().deep_clone();
        let map = |entries: &[(String, Option<ItemRef>)]| {
            entries
                .iter()
                .map(|(location, item)| {
                    let i = self.position(item);
                    let copy = usize::try_from(i).ok().and_then(|i| container.borrow().items.get(i).cloned());
                    (location.clone(), copy)
                })
                .collect()
        };
        Self {
            worn: map(&self.worn),
            attached: map(&self.attached),
            container,
        }
    }
}

impl PartialEq for CorpseInventory {
    /// Value equality: the same container contents and entry positions.
    fn eq(&self, other: &Self) -> bool {
        let entries = |a: &Self, e: &[(String, Option<ItemRef>)]| -> Vec<(String, i16)> {
            e.iter().map(|(l, i)| (l.clone(), a.position(i))).collect()
        };
        self.container == other.container
            && entries(self, &self.worn) == entries(other, &other.worn)
            && entries(self, &self.attached) == entries(other, &other.attached)
    }
}

impl IsoDeadBody {
    /// `getContainer()`: the live container, made from the saved one on the
    /// first call. `None` when the corpse has no container.
    pub fn runtime_container(&mut self) -> Option<ContainerRef> {
        if let Some(r) = &self.runtime {
            return Some(r.container.clone());
        }
        let saved = self.container.take()?;
        // Saved index -> index among the decoded items (raw records are
        // kept apart by `SharedContainer`).
        let mut decoded = 0;
        let to_decoded: Vec<Option<usize>> = saved
            .items
            .iter()
            .map(|item| {
                matches!(item, SavedItem::Item(_)).then(|| {
                    decoded += 1;
                    decoded - 1
                })
            })
            .collect();
        let container = SharedContainer::from_container(saved);
        let entries = |e: Vec<(String, i16)>| -> Vec<(String, Option<ItemRef>)> {
            e.into_iter()
                .map(|(location, index)| {
                    let item = usize::try_from(index)
                        .ok()
                        .and_then(|i| to_decoded.get(i).copied().flatten())
                        .and_then(|i| container.borrow().items.get(i).cloned());
                    (location, item)
                })
                .collect()
        };
        let worn = entries(std::mem::take(&mut self.worn_items));
        let attached = entries(std::mem::take(&mut self.attached_items));
        self.runtime = Some(CorpseInventory {
            container: container.clone(),
            worn,
            attached,
        });
        Some(container)
    }

    /// `checkClothing(item)`: the worn and attached items that are no
    /// longer in the container leave `wornItems` and `attachedItems`.
    pub fn check_clothing(&mut self) {
        let Some(r) = &mut self.runtime else {
            return;
        };
        let container = r.container.clone();
        let inside = |item: &Option<ItemRef>| {
            item.as_ref()
                .is_some_and(|item| container.borrow().items.iter().any(|i| Rc::ptr_eq(i, item)))
        };
        r.worn.retain(|(_, item)| inside(item));
        r.attached.retain(|(_, item)| inside(item));
    }

    /// The container and the worn and attached entries as saving writes
    /// them: the saved fields, or a snapshot of the live container.
    pub fn inventory_snapshot(&self) -> SaveResult<Option<InventorySnapshot>> {
        let Some(r) = &self.runtime else {
            return Ok(self.container.clone().map(|container| InventorySnapshot {
                container,
                worn: self.worn_items.clone(),
                attached: self.attached_items.clone(),
                positions: false,
            }));
        };
        ArchiveCopyCheck::default()
            .container_ref(&r.container)
            .map_err(|e| SaveError(format!("IsoDeadBody inventory snapshot: {e}")))?;
        let container = r
            .container
            .try_borrow()
            .map_err(|_| SaveError("IsoDeadBody inventory snapshot: BorrowedContainer".into()))?
            .to_container();
        let entries = |e: &[(String, Option<ItemRef>)]| e.iter().map(|(l, i)| (l.clone(), r.position(i))).collect();
        Ok(Some(InventorySnapshot {
            container,
            worn: entries(&r.worn),
            attached: entries(&r.attached),
            positions: true,
        }))
    }
}

#[cfg(test)]
mod tests;
