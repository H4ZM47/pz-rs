//! `InventoryItem.loadItem`/`saveWithSize`: an item record, decoded when
//! the codec supports its class, else kept as its bytes.

use crate::env::ItemEnv;
use crate::io::{err, Reader, SaveResult, Writer};
use crate::item::InventoryItem;
use crate::template::ItemClass;

/// One item as saved in a container or object.
#[derive(Clone, Debug, PartialEq)]
pub enum SavedItem {
    /// A decoded item.
    Item(Box<InventoryItem>),
    /// An item kept as the bytes after the length (registry id, save type,
    /// item), re-saved unchanged.
    Raw(Vec<u8>),
}

impl SavedItem {
    /// `getRegistry_id()`.
    pub fn registry_id(&self) -> i16 {
        match self {
            SavedItem::Item(i) => i.registry_id,
            SavedItem::Raw(d) => i16::from_be_bytes([d[0], d[1]]),
        }
    }

    /// `id`.
    pub fn id(&self) -> i32 {
        match self {
            SavedItem::Item(i) => i.id,
            SavedItem::Raw(d) => i32::from_be_bytes(d[3..7].try_into().unwrap()),
        }
    }

    pub fn set_id(&mut self, id: i32) {
        match self {
            SavedItem::Item(i) => i.id = id,
            SavedItem::Raw(d) => d[3..7].copy_from_slice(&id.to_be_bytes()),
        }
    }

    pub fn as_item(&self) -> Option<&InventoryItem> {
        match self {
            SavedItem::Item(i) => Some(i),
            SavedItem::Raw(_) => None,
        }
    }

    pub fn as_item_mut(&mut self) -> Option<&mut InventoryItem> {
        match self {
            SavedItem::Item(i) => Some(i),
            SavedItem::Raw(_) => None,
        }
    }

    /// `getFullType()` (raw records: the dictionary name of the id).
    pub fn full_type(&self, env: &dyn ItemEnv) -> String {
        match self {
            SavedItem::Item(i) => i.get_full_type(),
            SavedItem::Raw(_) => env
                .full_type(self.registry_id())
                .unwrap_or_else(|| format!("#{}", self.registry_id())),
        }
    }

    /// Whether this is an `InventoryContainer` holding items (such items
    /// are never grouped). Raw containers count as non-empty.
    pub fn is_nonempty_container(&self, env: &dyn ItemEnv) -> bool {
        match self {
            SavedItem::Item(i) => match &i.kind {
                crate::item::ItemKind::Container(c) => !c.container.items.is_empty(),
                _ => false,
            },
            SavedItem::Raw(_) => env
                .template(self.registry_id())
                .is_some_and(|t| t.class == ItemClass::Container),
        }
    }

    /// `InventoryItem.loadItem`: `None` when the original drops the item
    /// (its id cannot be created).
    pub fn load(r: &mut Reader<'_>, wv: i32, env: &dyn ItemEnv) -> SaveResult<Option<Self>> {
        let len = r.i32()?;
        if len <= 0 {
            return err(format!(
                "InventoryItem.loadItem() invalid item data length: {len}"
            ));
        }
        if len < 7 {
            return err(format!("item record of {len} bytes"));
        }
        let data = r.bytes(len as usize)?;
        let reg = i16::from_be_bytes([data[0], data[1]]);
        if !env.exists(reg) {
            return Ok(None);
        }
        Ok(Some(Self::decode(data, wv, env)))
    }

    /// Decodes a record's bytes, falling back to raw.
    pub fn decode(data: &[u8], wv: i32, env: &dyn ItemEnv) -> Self {
        let reg = i16::from_be_bytes([data[0], data[1]]);
        let raw = |why: &str| {
            env.note_raw(reg, why);
            SavedItem::Raw(data.to_vec())
        };
        let Some(t) = env.template(reg) else {
            return raw("no script");
        };
        if !env.decodes(&t) {
            return raw(t.class.java_name());
        }
        let Some(mut item) = env.create_item(reg) else {
            return raw("not creatable");
        };
        let mut sub = Reader::new(&data[3..]);
        match item.load(&mut sub, wv, env) {
            Ok(()) if sub.remaining() == 0 => SavedItem::Item(Box::new(item)),
            Ok(()) => raw("length mismatch"),
            Err(_) => raw("load error"),
        }
    }

    /// The record without the length (what `save` writes).
    pub fn record(&self, env: &dyn ItemEnv) -> SaveResult<Vec<u8>> {
        match self {
            SavedItem::Item(i) => {
                let mut w = Writer::new();
                i.save(&mut w, env)?;
                Ok(w.buf)
            }
            SavedItem::Raw(d) => Ok(d.clone()),
        }
    }

    /// `saveWithSize`.
    pub fn save(&self, w: &mut Writer, env: &dyn ItemEnv) -> SaveResult<()> {
        let rec = self.record(env)?;
        w.i32(rec.len() as i32);
        w.bytes(&rec);
        Ok(())
    }
}
