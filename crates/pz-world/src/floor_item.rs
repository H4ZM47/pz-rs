//! The live item of an `IsoWorldInventoryObject`.
//!
//! Decoded records become shared items once; placing an item retains the
//! supplied object. [`FloorItem::clone`] is an archive/value copy, like
//! cloning the surrounding chunk. Use [`FloorItem::item_ref`] when the
//! caller needs another handle to the live object instead.

use std::cell::{Ref, RefMut};
use std::rc::Rc;

use pz_inventory::env::ItemEnv;
use pz_inventory::io::{SaveResult, Writer};
use pz_inventory::saved::SavedItem;
use pz_inventory::shared::{self, ItemRef};
use pz_inventory::InventoryItem;

#[derive(Debug)]
enum Payload {
    Live(ItemRef),
    /// The native nullable item field (also the collector's detached slot).
    Null,
    // Only SavedItem::Raw enters this variant. Keeping the codec value
    // delegates its framing and registry-ID behavior without re-decoding.
    Raw(SavedItem),
}

/// One floor object's authoritative item, or its undecodable saved record.
#[derive(Debug)]
pub struct FloorItem {
    payload: Payload,
}

impl FloorItem {
    pub fn null() -> Self {
        Self {
            payload: Payload::Null,
        }
    }

    pub fn item_slot(&self) -> Option<&ItemRef> {
        match &self.payload {
            Payload::Live(item) => Some(item),
            Payload::Null | Payload::Raw(_) => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self.payload, Payload::Null)
    }

    pub fn is_raw(&self) -> bool {
        matches!(self.payload, Payload::Raw(_))
    }

    pub fn replace_item(&mut self, item: Option<&ItemRef>) -> Self {
        std::mem::replace(self, item.map_or_else(Self::null, Self::retain))
    }

    pub fn take_item_for_gc(&mut self) -> Option<ItemRef> {
        if matches!(self.payload, Payload::Live(_)) {
            match std::mem::replace(&mut self.payload, Payload::Null) {
                Payload::Live(item) => Some(item),
                _ => unreachable!(),
            }
        } else {
            None
        }
    }
    /// Consumes a loaded record, promoting a decoded item exactly once.
    /// Unknown records remain raw, including on subsequent reads/saves.
    pub fn from_saved(saved: SavedItem) -> Self {
        let payload = match saved {
            SavedItem::Item(item) => Payload::Live(shared::new_item(*item)),
            raw @ SavedItem::Raw(_) => Payload::Raw(raw),
        };
        Self { payload }
    }

    /// Retains the actual supplied item when it is placed on a square.
    pub fn retain(item: &ItemRef) -> Self {
        Self {
            payload: Payload::Live(Rc::clone(item)),
        }
    }

    /// Another handle to the live object; raw records have no item object.
    pub fn item_ref(&self) -> Option<ItemRef> {
        match &self.payload {
            Payload::Live(item) => Some(Rc::clone(item)),
            Payload::Null | Payload::Raw(_) => None,
        }
    }

    pub fn borrow_item(&self) -> Option<Ref<'_, InventoryItem>> {
        match &self.payload {
            Payload::Live(item) => Some(item.borrow()),
            Payload::Null | Payload::Raw(_) => None,
        }
    }

    pub fn borrow_item_mut(&self) -> Option<RefMut<'_, InventoryItem>> {
        match &self.payload {
            Payload::Live(item) => Some(item.borrow_mut()),
            Payload::Null | Payload::Raw(_) => None,
        }
    }

    pub fn registry_id(&self) -> Option<i16> {
        match &self.payload {
            Payload::Live(item) => Some(item.borrow().registry_id),
            Payload::Raw(raw) => Some(raw.registry_id()),
            Payload::Null => None,
        }
    }

    /// The item record without its length, read from the live owner.
    pub fn record(&self, env: &dyn ItemEnv) -> SaveResult<Vec<u8>> {
        match &self.payload {
            Payload::Live(item) => {
                let mut w = Writer::new();
                item.borrow().save(&mut w, env)?;
                Ok(w.buf)
            }
            Payload::Raw(raw) => raw.record(env),
            Payload::Null => Err(pz_inventory::io::SaveError(
                "null native floor item has no saved record".into(),
            )),
        }
    }

    /// `InventoryItem.saveWithSize`, without a parallel mutable snapshot.
    pub fn save(&self, w: &mut Writer, env: &dyn ItemEnv) -> SaveResult<()> {
        match &self.payload {
            Payload::Live(item) => item.borrow().save_with_size(w, env),
            Payload::Raw(raw) => raw.save(w, env),
            Payload::Null => Err(pz_inventory::io::SaveError(
                "null native floor item cannot be saved".into(),
            )),
        }
    }

    /// An explicit archive snapshot. Nested bag contents are copied by
    /// value; live container/parent handles are not substituted into it.
    pub fn to_saved_snapshot(&self) -> SavedItem {
        self.try_to_saved_snapshot()
            .expect("floor-item archive snapshot needs available, bounded data")
    }

    /// Fallible archive snapshot of the same actual floor item. Live VM
    /// tables must be copied independently, never shared by Rust Clone.
    pub fn try_to_saved_snapshot(
        &self,
    ) -> Result<SavedItem, pz_inventory::archive::ArchiveCopyError> {
        match &self.payload {
            Payload::Live(item) => {
                let item = item
                    .try_borrow()
                    .map_err(|_| pz_inventory::archive::ArchiveCopyError::BorrowedItem)?;
                item.try_archive_copy()
                    .map(|copy| SavedItem::Item(Box::new(copy)))
            }
            Payload::Raw(raw) => Ok(raw.clone()),
            // SavedItem has no null carrier. A native null is never fabricated
            // into raw bytes by this explicit archive-only adapter.
            Payload::Null => Err(pz_inventory::archive::ArchiveCopyError::NullItem),
        }
    }
}

impl From<SavedItem> for FloorItem {
    fn from(saved: SavedItem) -> Self {
        Self::from_saved(saved)
    }
}

impl Clone for FloorItem {
    fn clone(&self) -> Self {
        match &self.payload {
            Payload::Null => Self::null(),
            _ => Self::from_saved(self.to_saved_snapshot()),
        }
    }
}

impl PartialEq for FloorItem {
    fn eq(&self, other: &Self) -> bool {
        match (&self.payload, &other.payload) {
            (Payload::Live(a), Payload::Live(b)) => *a.borrow() == *b.borrow(),
            (Payload::Raw(a), Payload::Raw(b)) => a == b,
            (Payload::Null, Payload::Null) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
