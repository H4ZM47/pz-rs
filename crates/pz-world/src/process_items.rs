//! `IsoCell`'s live inventory-item process list and pending-removal set.
//!
//! These hold the actual shared inventory objects. Saved world-container
//! codecs are not converted into a second set of runtime item identities.
//! Per-item `update`/`finishupdate` and world-inventory-object processing
//! remain separate from this queue owner.

use pz_inventory::shared::ItemRef;
use std::cell::RefCell;
use std::rc::Rc;

/// Java collections can contain null when mutated directly through Lua.
pub type ProcessItemEntries = Rc<RefCell<Vec<Option<ItemRef>>>>;

/// `InventoryItem` inherits Java object equality: its numeric ID is not
/// the identity used by `ArrayList.contains` or `HashSet`.
pub fn same_item(a: &Option<ItemRef>, b: &Option<ItemRef>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

#[derive(Default)]
pub struct ProcessItemQueues {
    /// `processItems`, including any direct ArrayList mutations.
    pub items: ProcessItemEntries,
    /// `processItemsRemove`. Writers must retain set semantics.
    pub remove: ProcessItemEntries,
}

/// Removed owning slots. The caller drops these after releasing any
/// enclosing world/cell borrow, since an item's Lua owners may reenter it.
pub struct RemovedProcessItems {
    _slots: Vec<Option<ItemRef>>,
    _pending: Vec<Option<ItemRef>>,
}

impl ProcessItemQueues {
    pub fn add(&self, item: Option<ItemRef>, client: bool) {
        let Some(item) = item else { return };
        if client {
            return;
        }
        let item = Some(item);
        // Keep removed strong references alive until both collection
        // borrows end; item destruction may release Lua-owned values.
        let cancelled = {
            let mut pending = self.remove.borrow_mut();
            pending
                .iter()
                .position(|i| same_item(i, &item))
                .map(|n| pending.remove(n))
        };
        {
            let mut items = self.items.borrow_mut();
            if !items.iter().any(|i| same_item(i, &item)) {
                items.push(item);
            }
        }
        drop(cancelled);
    }

    pub fn add_all(&self, items: impl IntoIterator<Item = ItemRef>, client: bool) {
        for item in items {
            self.add(Some(item), client);
        }
    }

    pub fn queue_remove(&self, item: Option<ItemRef>) {
        let Some(item) = item else { return };
        let item = Some(item);
        let mut pending = self.remove.borrow_mut();
        if !pending.iter().any(|i| same_item(i, &item)) {
            pending.push(item);
        }
    }

    pub fn queue_remove_all(&self, items: impl IntoIterator<Item = ItemRef>) {
        for item in items {
            self.queue_remove(Some(item));
        }
    }

    /// `ProcessRemoveItems`: remove all occurrences and clear the pending
    /// set. Run before and after the process phase. No item is dropped
    /// while either collection has a mutable borrow.
    pub fn drain_remove(&self) -> RemovedProcessItems {
        let pending = std::mem::take(&mut *self.remove.borrow_mut());
        let mut removed = Vec::new();
        {
            let mut items = self.items.borrow_mut();
            let old = std::mem::take(&mut *items);
            for item in old {
                if pending.iter().any(|p| same_item(p, &item)) {
                    removed.push(item);
                } else {
                    items.push(item);
                }
            }
        }
        RemovedProcessItems {
            _slots: removed,
            _pending: pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};

    fn item(id: i32) -> ItemRef {
        let mut i = InventoryItem::create(
            Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Rock")),
            1,
        );
        i.id = id;
        pz_inventory::shared::new_item(i)
    }

    #[test]
    fn identity_dedup_cancel_and_ordered_drain() {
        let q = ProcessItemQueues::default();
        let a = item(7);
        let b = item(7);
        q.add_all([a.clone(), b.clone(), a.clone()], false);
        assert_eq!(q.items.borrow().len(), 2);
        q.queue_remove(Some(a.clone()));
        q.queue_remove(Some(a.clone()));
        assert_eq!(q.remove.borrow().len(), 1);
        assert_eq!(q.items.borrow().len(), 2, "removal is deferred");
        q.add(Some(a.clone()), false);
        assert!(q.remove.borrow().is_empty(), "re-add cancels cell removal");
        q.queue_remove(Some(a.clone()));
        q.items.borrow_mut().push(Some(a));
        q.drain_remove();
        assert_eq!(
            q.items.borrow().len(),
            1,
            "all duplicate list slots removed"
        );
        assert!(Rc::ptr_eq(q.items.borrow()[0].as_ref().unwrap(), &b));
        assert!(q.remove.borrow().is_empty());
        q.drain_remove();
        assert_eq!(q.items.borrow().len(), 1);
    }

    #[test]
    fn nulls_client_gate_and_retained_cell_collections() {
        let q = ProcessItemQueues::default();
        let a = item(1);
        q.add(None, false);
        q.queue_remove(None);
        q.add(Some(a.clone()), true);
        assert!(q.items.borrow().is_empty());
        q.add(Some(a.clone()), false);
        q.queue_remove(Some(a.clone()));
        q.add(Some(a), true);
        assert_eq!(
            q.remove.borrow().len(),
            1,
            "client add does not cancel removal"
        );
        q.items.borrow_mut().push(None);
        q.remove.borrow_mut().push(None);
        let retained = q.items.clone();
        q.drain_remove();
        assert!(retained.borrow().is_empty());
        retained.borrow_mut().push(Some(item(2)));
        drop(q);
        assert_eq!(retained.borrow().len(), 1);
        assert!(ProcessItemQueues::default().items.borrow().is_empty());
    }
}
