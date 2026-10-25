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
            Payload::Raw(_) => None,
        }
    }

    pub fn borrow_item(&self) -> Option<Ref<'_, InventoryItem>> {
        match &self.payload {
            Payload::Live(item) => Some(item.borrow()),
            Payload::Raw(_) => None,
        }
    }

    pub fn borrow_item_mut(&self) -> Option<RefMut<'_, InventoryItem>> {
        match &self.payload {
            Payload::Live(item) => Some(item.borrow_mut()),
            Payload::Raw(_) => None,
        }
    }

    pub fn registry_id(&self) -> i16 {
        match &self.payload {
            Payload::Live(item) => item.borrow().registry_id,
            Payload::Raw(raw) => raw.registry_id(),
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
        }
    }

    /// `InventoryItem.saveWithSize`, without a parallel mutable snapshot.
    pub fn save(&self, w: &mut Writer, env: &dyn ItemEnv) -> SaveResult<()> {
        match &self.payload {
            Payload::Live(item) => item.borrow().save_with_size(w, env),
            Payload::Raw(raw) => raw.save(w, env),
        }
    }

    /// An explicit archive snapshot. Nested bag contents are copied by
    /// value; live container/parent handles are not substituted into it.
    pub fn to_saved_snapshot(&self) -> SavedItem {
        match &self.payload {
            Payload::Live(item) => SavedItem::Item(Box::new(item.borrow().clone())),
            Payload::Raw(raw) => raw.clone(),
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
        Self::from_saved(self.to_saved_snapshot())
    }
}

impl PartialEq for FloorItem {
    fn eq(&self, other: &Self) -> bool {
        match (&self.payload, &other.payload) {
            (Payload::Live(a), Payload::Live(b)) => *a.borrow() == *b.borrow(),
            (Payload::Raw(a), Payload::Raw(b)) => a == b,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
