//! Checks for the bounded, synchronous archive copies used by world fixtures.
//!
//! This is a value-tree copy policy, not a native object-graph clone. Repeated
//! bag children are copied per occurrence. Cycles and excessively deep or large
//! trees are rejected before the existing recursive `Clone` implementations run.
use crate::moddata::ItemModData;
use pz_kahlua::{TableRef, Value};
use std::collections::HashSet;
use std::rc::Rc;

use crate::lua_table::{LuaTable, LuaValue};
use crate::shared::{ContainerRef, ItemRef};
use crate::{InventoryItem, ItemContainer, ItemKind, SavedItem};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveCopyError {
    /// A runtime world object may contain the native null item reference;
    /// SavedItem has no representation for that state.
    NullItem,
    BorrowedItem,
    BorrowedContainer,
    BorrowedComponent,
    BorrowedTable,
    CyclicTable,
    CyclicContainer,
    DepthLimit,
    SizeLimit,
}

impl std::fmt::Display for ArchiveCopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "inventory archive copy: {self:?}")
    }
}
impl std::error::Error for ArchiveCopyError {}

/// One check per complete copy operation. After checking, copy immediately on
/// the same thread without callbacks or source mutation. Checking alone is not
/// a durable permission to clone later. The existing item/container clones do
/// not run callbacks; opaque parent/source-grid links are deliberately omitted.
#[derive(Default)]
pub struct ArchiveCopyCheck {
    active_containers: HashSet<usize>,
    active_tables: HashSet<usize>,
    occurrences: usize,
}

impl ArchiveCopyCheck {
    const MAX_DEPTH: usize = 64;
    const MAX_OCCURRENCES: usize = 16_384;

    fn enter(&mut self, depth: usize) -> Result<(), ArchiveCopyError> {
        if depth > Self::MAX_DEPTH {
            return Err(ArchiveCopyError::DepthLimit);
        }
        self.occurrences += 1;
        if self.occurrences > Self::MAX_OCCURRENCES {
            return Err(ArchiveCopyError::SizeLimit);
        }
        Ok(())
    }

    pub fn item_ref(&mut self, item: &ItemRef) -> Result<(), ArchiveCopyError> {
        let item = item
            .try_borrow()
            .map_err(|_| ArchiveCopyError::BorrowedItem)?;
        self.item(&item, 0)
    }

    pub fn saved_item(&mut self, item: &SavedItem) -> Result<(), ArchiveCopyError> {
        self.saved(item, 0)
    }

    pub fn container(&mut self, container: &ItemContainer) -> Result<(), ArchiveCopyError> {
        self.enter(0)?;
        for item in &container.items {
            self.saved(item, 1)?;
        }
        Ok(())
    }

    /// Validate the live container graph before an explicit value archive.
    pub fn container_ref(&mut self, container: &ContainerRef) -> Result<(), ArchiveCopyError> {
        self.shared_container(container, 0)
    }

    pub fn shared_container_body(
        &mut self,
        container: &crate::shared::SharedContainer,
    ) -> Result<(), ArchiveCopyError> {
        self.enter(0)?;
        for item in &container.items {
            self.item_ref(item)?;
        }
        for item in &container.raw_items {
            self.saved_item(item)?;
        }
        Ok(())
    }
    pub fn table(&mut self, table: &LuaTable) -> Result<(), ArchiveCopyError> {
        self.table_at(table, 0)
    }

    pub fn optional_table(&mut self, table: &Option<LuaTable>) -> Result<(), ArchiveCopyError> {
        if let Some(table) = table {
            self.table(table)?;
        }
        Ok(())
    }
    pub fn optional_mod_data(
        &mut self,
        table: &Option<ItemModData>,
    ) -> Result<(), ArchiveCopyError> {
        if let Some(table) = table {
            self.mod_table_at(table, 0)?;
        }
        Ok(())
    }

    fn mod_table_at(&mut self, table: &ItemModData, depth: usize) -> Result<(), ArchiveCopyError> {
        match table {
            ItemModData::Archive(table) => self.table_at(table, depth),
            ItemModData::Live(table) => self.live_table_at(table, depth),
        }
    }
    fn live_table_at(&mut self, table: &TableRef, depth: usize) -> Result<(), ArchiveCopyError> {
        self.enter(depth)?;
        if !self.active_tables.insert(table.ptr_id()) {
            return Err(ArchiveCopyError::CyclicTable);
        }
        let result = (|| {
            for (key, value) in table
                .try_entries()
                .map_err(|_| ArchiveCopyError::BorrowedTable)?
            {
                for value in [key, value] {
                    match value {
                        Value::Table(table) => self.live_table_at(&table, depth + 1)?,
                        _ => self.enter(depth + 1)?,
                    }
                }
            }
            if let Some(meta) = table
                .try_metatable()
                .map_err(|_| ArchiveCopyError::BorrowedTable)?
            {
                self.live_table_at(&meta, depth + 1)?;
            }
            Ok(())
        })();
        self.active_tables.remove(&table.ptr_id());
        result
    }

    fn table_at(&mut self, table: &LuaTable, depth: usize) -> Result<(), ArchiveCopyError> {
        self.enter(depth)?;
        for (_, value) in &table.entries {
            if let LuaValue::Table(table) = value {
                self.table_at(table, depth + 1)?;
            } else {
                self.enter(depth + 1)?;
            }
        }
        Ok(())
    }

    fn saved(&mut self, item: &SavedItem, depth: usize) -> Result<(), ArchiveCopyError> {
        match item {
            SavedItem::Item(item) => self.item(item, depth),
            SavedItem::Raw(_) => self.enter(depth),
        }
    }

    fn item(&mut self, item: &InventoryItem, depth: usize) -> Result<(), ArchiveCopyError> {
        self.enter(depth)?;
        for entry in item.components.entry_slots() {
            if let pz_entity::RuntimeEntryView::Live { component, .. } = entry {
                let body = component
                    .try_borrow()
                    .map_err(|_| ArchiveCopyError::BorrowedComponent)?;
                body.check_archive_copy()
                    .map_err(|_| ArchiveCopyError::BorrowedComponent)?;
            }
        }
        if let Some(table) = &item.table {
            self.mod_table_at(table, depth + 1)?;
        }
        match &item.kind {
            ItemKind::Container(bag) => self.shared_container(&bag.container, depth + 1)?,
            ItemKind::Weapon(weapon) => {
                for attachment in &weapon.attachments {
                    self.item(attachment, depth + 1)?;
                }
            }
            ItemKind::Plain
            | ItemKind::Food(_)
            | ItemKind::Clothing(_)
            | ItemKind::AlarmClockClothing(..)
            | ItemKind::Literature(_)
            | ItemKind::AlarmClock(_)
            | ItemKind::Key { .. }
            | ItemKind::Moveable(_)
            | ItemKind::Radio(..)
            | ItemKind::Map { .. } => {}
        }
        Ok(())
    }

    fn shared_container(
        &mut self,
        container: &ContainerRef,
        depth: usize,
    ) -> Result<(), ArchiveCopyError> {
        self.enter(depth)?;
        let key = Rc::as_ptr(container) as usize;
        if !self.active_containers.insert(key) {
            return Err(ArchiveCopyError::CyclicContainer);
        }
        let result = (|| {
            let container = container
                .try_borrow()
                .map_err(|_| ArchiveCopyError::BorrowedContainer)?;
            for item in &container.items {
                let item = item
                    .try_borrow()
                    .map_err(|_| ArchiveCopyError::BorrowedItem)?;
                self.item(&item, depth + 1)?;
            }
            // This public field normally holds Raw records, but Clone copies
            // every entry, including a manually supplied decoded bag.
            for item in &container.raw_items {
                self.saved(item, depth + 1)?;
            }
            Ok(())
        })();
        self.active_containers.remove(&key);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::ContainerData;
    use crate::{ItemClass, ItemTemplate};

    fn item(class: ItemClass) -> InventoryItem {
        InventoryItem::create(Rc::new(ItemTemplate::new(class, "Base", "ArchiveTest")), 1)
    }

    #[test]
    fn checks_cycles_through_bag_items_raw_records_and_weapon_attachments() {
        let bag = crate::shared::new_item(item(ItemClass::Container));
        let inventory = crate::shared::inventory_of(&bag.borrow()).unwrap();
        inventory.borrow_mut().items.push(bag.clone());
        assert_eq!(
            ArchiveCopyCheck::default().item_ref(&bag),
            Err(ArchiveCopyError::CyclicContainer)
        );
        inventory.borrow_mut().items.clear();
        let mut nested = item(ItemClass::Container);
        nested.kind = ItemKind::Container(Box::new(ContainerData {
            weight_reduction: 0,
            container: inventory.clone(),
        }));
        let mut weapon = item(ItemClass::Weapon);
        let ItemKind::Weapon(data) = &mut weapon.kind else {
            unreachable!()
        };
        data.attachments.push(nested);
        inventory
            .borrow_mut()
            .raw_items
            .push(SavedItem::Item(Box::new(weapon)));
        assert_eq!(
            ArchiveCopyCheck::default().item_ref(&bag),
            Err(ArchiveCopyError::CyclicContainer)
        );
        inventory.borrow_mut().raw_items.clear();
        assert!(ArchiveCopyCheck::default().item_ref(&bag).is_ok());
    }

    #[test]
    fn counts_repeated_dag_occurrences_and_bounds_owned_attachment_and_table_depth() {
        let bag = crate::shared::new_item(item(ItemClass::Container));
        let inventory = crate::shared::inventory_of(&bag.borrow()).unwrap();
        let child = crate::shared::new_item(item(ItemClass::Normal));
        inventory.borrow_mut().items = vec![child; ArchiveCopyCheck::MAX_OCCURRENCES];
        assert_eq!(
            ArchiveCopyCheck::default().item_ref(&bag),
            Err(ArchiveCopyError::SizeLimit)
        );
        inventory.borrow_mut().items.clear();

        let child = crate::shared::new_item(item(ItemClass::Normal));
        inventory.borrow_mut().items = vec![child.clone(), child];
        assert!(ArchiveCopyCheck::default().item_ref(&bag).is_ok());
        inventory.borrow_mut().items.clear();

        let mut root = item(ItemClass::Normal);
        for _ in 0..=ArchiveCopyCheck::MAX_DEPTH {
            let mut parent = item(ItemClass::Weapon);
            let ItemKind::Weapon(weapon) = &mut parent.kind else {
                unreachable!()
            };
            weapon.attachments.push(root);
            root = parent;
        }
        assert_eq!(
            ArchiveCopyCheck::default().saved_item(&SavedItem::Item(Box::new(root))),
            Err(ArchiveCopyError::DepthLimit)
        );
        let mut table = LuaTable::default();
        for _ in 0..=ArchiveCopyCheck::MAX_DEPTH {
            table = LuaTable {
                entries: vec![(
                    crate::lua_table::LuaKey::Str("nested".into()),
                    LuaValue::Table(table),
                )],
            };
        }
        assert_eq!(
            ArchiveCopyCheck::default().table(&table),
            Err(ArchiveCopyError::DepthLimit)
        );
    }
}

impl InventoryItem {
    /// Independent bounded Rust archive copy. Actual opaque ModData objects
    /// retain their VM references, but the runtime worldItem backlink is
    /// omitted. A copied floor graph must explicitly bind its own new owner.
    /// This does not implement native debug createCloneItem.
    pub fn try_archive_copy(&self) -> Result<Self, ArchiveCopyError> {
        ArchiveCopyCheck::default().item(self, 0)?;
        Ok(self.clone())
    }
}
