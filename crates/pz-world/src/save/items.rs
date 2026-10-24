//! Inventory items as opaque records, `CompressIdenticalItems` and
//! `ItemContainer.save`/`load`.
//!
//! `InventoryItem.saveWithSize` writes a 4-byte length before the item
//! (registry id, save type, id, item fields), and `loadItem` uses it to
//! skip what it did not read. That lets chunk files be read and written
//! exactly without porting the item classes: each item is kept as its
//! length-delimited bytes. What this cannot reproduce is an item whose own
//! `load` changes it (`Clothing.lastWetnessUpdate` is reset to the world
//! age, for example); such items save back unchanged here.

use super::io::{err, Reader, SaveResult, Writer};

/// One saved `InventoryItem` (`saveWithSize` minus the length).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemBlob {
    /// The bytes after the length: `registry id (short)`, save type, item.
    pub data: Vec<u8>,
}

impl ItemBlob {
    /// `InventoryItem.getRegistry_id()` as saved.
    pub fn registry_id(&self) -> i16 {
        i16::from_be_bytes([self.data[0], self.data[1]])
    }

    /// `InventoryItem.id`.
    pub fn id(&self) -> i32 {
        i32::from_be_bytes(self.data[3..7].try_into().unwrap())
    }

    /// `InventoryItem.loadItem` framing.
    pub fn load(r: &mut Reader<'_>) -> SaveResult<Self> {
        let len = r.i32()?;
        if len <= 0 {
            return err(format!(
                "InventoryItem.loadItem() invalid item data length: {len}"
            ));
        }
        if len < 7 {
            return err(format!("item record of {len} bytes"));
        }
        Ok(ItemBlob {
            data: r.bytes(len as usize)?.to_vec(),
        })
    }

    pub fn save(&self, w: &mut Writer) {
        w.i32(self.data.len() as i32);
        w.bytes(&self.data);
    }
}

/// A run of identical items as `CompressIdenticalItems` writes it: one
/// record and the ids of the copies after the first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRun {
    pub item: ItemBlob,
    pub other_ids: Vec<i32>,
}

impl ItemRun {
    pub fn count(&self) -> usize {
        1 + self.other_ids.len()
    }
}

/// What item loading needs from the item system (`InventoryItemFactory`,
/// `ScriptManager`). The default knows every registry id.
pub trait ItemRegistry {
    /// `InventoryItemFactory.CreateItem(registryID) != null`.
    fn exists(&self, _registry_id: i16) -> bool {
        true
    }
    /// `getFullType()` of the item, for world-item removal.
    fn full_type(&self, _registry_id: i16) -> Option<&str> {
        None
    }
    /// `ScriptManager.FindItem(fullType).getObsolete()`.
    fn obsolete(&self, _full_type: &str) -> bool {
        false
    }
}

/// [`ItemRegistry`] that knows every item.
pub struct AllItems;
impl ItemRegistry for AllItems {}

/// `CompressIdenticalItems.load`: the runs, dropping those whose item the
/// registry cannot create (the original skips their bytes).
pub fn load_items(r: &mut Reader<'_>, reg: &dyn ItemRegistry) -> SaveResult<Vec<ItemRun>> {
    let count = r.i16()?;
    let mut runs = Vec::new();
    for _ in 0..count.max(0) {
        let identical = r.i32()?;
        let item = ItemBlob::load(r)?;
        let mut other_ids = Vec::new();
        for _ in 1..identical.max(1) {
            other_ids.push(r.i32()?);
        }
        if reg.exists(item.registry_id()) {
            runs.push(ItemRun { item, other_ids });
        }
    }
    Ok(runs)
}

/// `CompressIdenticalItems.save` of already-grouped runs.
pub fn save_items(w: &mut Writer, runs: &[ItemRun]) {
    w.i16(runs.len() as i16);
    for run in runs {
        w.i32(run.count() as i32);
        run.item.save(w);
        for &id in &run.other_ids {
            w.i32(id);
        }
    }
}

/// `CompressIdenticalItems.save(output, item)`: one optional item.
pub fn save_single(w: &mut Writer, item: Option<&ItemBlob>) {
    match item {
        None => w.i16(0),
        Some(i) => {
            w.i16(1);
            w.i32(1);
            i.save(w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(reg: i16, id: i32) -> ItemBlob {
        let mut d = reg.to_be_bytes().to_vec();
        d.push(0xff);
        d.extend_from_slice(&id.to_be_bytes());
        d.push(0);
        ItemBlob { data: d }
    }

    struct No7;
    impl ItemRegistry for No7 {
        fn exists(&self, r: i16) -> bool {
            r != 7
        }
    }

    #[test]
    fn runs_round_trip_and_drop() {
        let runs = vec![
            ItemRun {
                item: blob(3, 10),
                other_ids: vec![11, 12],
            },
            ItemRun {
                item: blob(7, 20),
                other_ids: vec![],
            },
        ];
        let mut w = Writer::new();
        save_items(&mut w, &runs);
        let back = load_items(&mut Reader::new(&w.buf), &AllItems).unwrap();
        assert_eq!(back, runs);
        assert_eq!(back[0].item.registry_id(), 3);
        assert_eq!(back[0].item.id(), 10);
        let mut r = Reader::new(&w.buf);
        let kept = load_items(&mut r, &No7).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(r.remaining(), 0);
    }
}
