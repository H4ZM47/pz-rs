//! The item part of `WorldDictionary.bin` (`DictionaryData.loadFromByteBuffer`):
//! registry id to full type, and the flags that decide whether
//! `InventoryItemFactory.CreateItem(registryID)` can create the item.

use std::collections::HashMap;

use crate::io::{err, Reader, SaveResult};

/// `ItemInfo` (`DictionaryInfo`) as saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictionaryInfo {
    pub registry_id: i16,
    pub module_name: String,
    pub name: String,
    pub full_type: String,
    /// `modId` (`pz-vanilla` when not modded).
    pub mod_id: String,
    pub is_modded: bool,
    pub exists_as_vanilla: bool,
    pub obsolete: bool,
    pub removed: bool,
    pub mod_overrides: Option<Vec<String>>,
}

impl DictionaryInfo {
    fn load(r: &mut Reader<'_>, mod_ids: &[String], modules: &[String]) -> SaveResult<Self> {
        let idx = |r: &mut Reader<'_>, list: &[String]| -> SaveResult<String> {
            let i = if list.len() > 127 {
                r.i16()? as i32
            } else {
                r.i8()? as i32
            };
            match list.get(i.max(0) as usize).filter(|_| i >= 0) {
                Some(s) => Ok(s.clone()),
                None => err(format!("dictionary index {i} out of range")),
            }
        };
        let registry_id = r.i16()?;
        let module_name = idx(r, modules)?;
        let name = r.string()?;
        let bits = r.i8()? as u8;
        let (mod_id, is_modded) = if bits & 1 != 0 {
            (idx(r, mod_ids)?, true)
        } else {
            ("pz-vanilla".to_string(), false)
        };
        let mut mod_overrides = None;
        if bits & 16 != 0 {
            let mut v = Vec::new();
            if bits & 32 == 0 {
                v.push(idx(r, mod_ids)?);
            } else {
                let n = r.i8()?;
                for _ in 0..n.max(0) {
                    v.push(idx(r, mod_ids)?);
                }
            }
            mod_overrides = Some(v);
        }
        Ok(DictionaryInfo {
            registry_id,
            full_type: format!("{module_name}.{name}"),
            module_name,
            name,
            mod_id,
            is_modded,
            exists_as_vanilla: bits & 2 != 0,
            obsolete: bits & 4 != 0,
            removed: bits & 8 != 0,
            mod_overrides,
        })
    }
}

/// The item and entity tables of a `WorldDictionary.bin`.
#[derive(Clone, Debug, Default)]
pub struct WorldDictionary {
    pub version: i32,
    pub items: HashMap<i16, DictionaryInfo>,
    pub entities: HashMap<i16, DictionaryInfo>,
    /// `objectIdToNameMap`.
    pub object_names: HashMap<i8, String>,
    /// `spriteIdToNameMap`.
    pub sprite_names: HashMap<i32, String>,
}

impl WorldDictionary {
    /// Reads the file up to the sprite names (the string and script
    /// registers after them are not needed for items).
    pub fn load(bytes: &[u8]) -> SaveResult<Self> {
        let r = &mut Reader::new(bytes);
        let version = r.i32()?;
        let _next_info_id = r.i16()?;
        let _next_object_name_id = r.i8()?;
        let _next_sprite_name_id = r.i32()?;
        let n = r.i32()?;
        let mut mod_ids = Vec::new();
        for _ in 0..n.max(0) {
            mod_ids.push(r.string()?);
        }
        let n = r.i32()?;
        let mut modules = Vec::new();
        for _ in 0..n.max(0) {
            modules.push(r.string()?);
        }
        let mut d = WorldDictionary {
            version,
            ..Default::default()
        };
        let n = r.i32()?;
        for _ in 0..n.max(0) {
            let i = DictionaryInfo::load(r, &mod_ids, &modules)?;
            d.items.insert(i.registry_id, i);
        }
        let n = r.i32()?;
        for _ in 0..n.max(0) {
            let i = DictionaryInfo::load(r, &mod_ids, &modules)?;
            d.entities.insert(i.registry_id, i);
        }
        let n = r.i32()?;
        for _ in 0..n.max(0) {
            let id = r.i8()?;
            d.object_names.insert(id, r.string()?);
        }
        let n = r.i32()?;
        for _ in 0..n.max(0) {
            let id = r.i32()?;
            d.sprite_names.insert(id, r.string()?);
        }
        Ok(d)
    }

    /// `getItemTypeFromID`.
    pub fn item_type(&self, id: i16) -> Option<&str> {
        self.items.get(&id).map(|i| i.full_type.as_str())
    }

    /// `getItemRegistryID(fullType)`.
    pub fn item_id(&self, full_type: &str) -> Option<i16> {
        self.items
            .values()
            .find(|i| i.full_type == full_type)
            .map(|i| i.registry_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::Writer;

    #[test]
    fn reads_items() {
        let mut w = Writer::new();
        w.i32(1);
        w.i16(3);
        w.i8(0);
        w.i32(0);
        w.i32(1);
        w.string("mymod");
        w.i32(2);
        w.string("Base");
        w.string("Farming");
        w.i32(2);
        // vanilla item
        w.i16(1);
        w.i8(0);
        w.string("Axe");
        w.i8(2);
        // modded, obsolete
        w.i16(2);
        w.i8(1);
        w.string("Hoe");
        w.i8(1 | 4);
        w.i8(0);
        w.i32(0);
        w.i32(1);
        w.i8(5);
        w.string("Door");
        w.i32(0);
        let d = WorldDictionary::load(&w.buf).unwrap();
        assert_eq!(d.item_type(1), Some("Base.Axe"));
        assert_eq!(d.items[&2].mod_id, "mymod");
        assert!(d.items[&2].obsolete && d.items[&1].exists_as_vanilla);
        assert_eq!(d.item_id("Farming.Hoe"), Some(2));
        assert_eq!(d.object_names[&5], "Door");
    }
}
