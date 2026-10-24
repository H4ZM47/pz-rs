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

/// A `DictionaryStringInfo` / `DictionaryScriptInfo` entry of a register.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterEntry {
    pub registry_id: i16,
    /// `string` / script `name` (a leading `Base.` is stored compressed).
    pub name: String,
    /// The script version hash (script registers only).
    pub version: Option<i64>,
    pub is_loaded: bool,
}

/// A `StringDictionary.StringRegister` or `ScriptsDictionary.ScriptRegister`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Register {
    pub name: String,
    pub next_id: i16,
    pub entries: Vec<RegisterEntry>,
}

impl Register {
    /// Id to name.
    pub fn name_of(&self, id: i16) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.registry_id == id)
            .map(|e| e.name.as_str())
    }

    /// Name to id.
    pub fn id_of(&self, name: &str) -> Option<i16> {
        self.entries
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.registry_id)
    }
}

/// `StringDictionary`/`ScriptsDictionary.loadFromByteBuffer`: named
/// registers, each a `ByteBlock` (an int length, then the content).
fn load_registers(r: &mut Reader<'_>, scripts: bool) -> SaveResult<Vec<Register>> {
    let n = r.i32()?;
    let mut out = Vec::new();
    for _ in 0..n.max(0) {
        let name = r.string()?;
        let len = r.i32()?;
        if len < 0 {
            return err("negative ByteBlock length");
        }
        let block = r.bytes(len as usize)?;
        let b = &mut Reader::new(block);
        let mut reg = Register {
            name,
            next_id: b.i16()?,
            entries: Vec::new(),
        };
        let count = b.i32()?;
        for _ in 0..count.max(0) {
            let h = b.i8()? as u8;
            let registry_id = b.i16()?;
            let version = if scripts { Some(b.i64()?) } else { None };
            let s = b.string()?;
            reg.entries.push(RegisterEntry {
                registry_id,
                name: if h & 1 != 0 { format!("Base.{s}") } else { s },
                version,
                is_loaded: h & 2 != 0,
            });
        }
        out.push(reg);
    }
    Ok(out)
}

/// The tables of a `WorldDictionary.bin`.
#[derive(Clone, Debug, Default)]
pub struct WorldDictionary {
    pub version: i32,
    pub items: HashMap<i16, DictionaryInfo>,
    pub entities: HashMap<i16, DictionaryInfo>,
    /// `objectIdToNameMap`.
    pub object_names: HashMap<i8, String>,
    /// `spriteIdToNameMap`.
    pub sprite_names: HashMap<i32, String>,
    /// `StringDictionary` registers, in file order.
    pub string_registers: Vec<Register>,
    /// `ScriptsDictionary` registers, in file order.
    pub script_registers: Vec<Register>,
}

impl WorldDictionary {
    /// Reads the whole file (`DictionaryData.loadFromByteBuffer`).
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
        if r.remaining() > 0 {
            d.string_registers = load_registers(r, false)?;
        }
        if r.remaining() > 0 {
            d.script_registers = load_registers(r, true)?;
        }
        Ok(d)
    }

    /// `getItemTypeFromID`.
    pub fn item_type(&self, id: i16) -> Option<&str> {
        self.items.get(&id).map(|i| i.full_type.as_str())
    }

    /// `getObjectNameFromID`.
    pub fn object_name(&self, id: i8) -> Option<&str> {
        self.object_names.get(&id).map(String::as_str)
    }

    /// `getIdForObjectName`.
    pub fn object_id(&self, name: &str) -> Option<i8> {
        self.object_names
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(&id, _)| id)
    }

    /// `getSpriteNameFromID`.
    pub fn sprite_name(&self, id: i32) -> Option<&str> {
        self.sprite_names.get(&id).map(String::as_str)
    }

    /// `getIdForSpriteName`.
    pub fn sprite_id(&self, name: &str) -> Option<i32> {
        self.sprite_names
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(&id, _)| id)
    }

    /// `getEntityTypeFromID`.
    pub fn entity_type(&self, id: i16) -> Option<&str> {
        self.entities.get(&id).map(|i| i.full_type.as_str())
    }

    /// A string register by name (`StringDictionary.Generic`, ...).
    pub fn string_register(&self, name: &str) -> Option<&Register> {
        self.string_registers.iter().find(|r| r.name == name)
    }

    /// A script register by name.
    pub fn script_register(&self, name: &str) -> Option<&Register> {
        self.script_registers.iter().find(|r| r.name == name)
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
        // one string register
        w.i32(1);
        w.string("Generic");
        let mut b = Writer::new();
        b.i16(2);
        b.i32(1);
        b.i8(3);
        b.i16(1);
        b.string("Thing");
        w.i32(b.buf.len() as i32);
        w.bytes(&b.buf);
        w.i32(0);
        let d = WorldDictionary::load(&w.buf).unwrap();
        assert_eq!(d.item_type(1), Some("Base.Axe"));
        assert_eq!(d.items[&2].mod_id, "mymod");
        assert!(d.items[&2].obsolete && d.items[&1].exists_as_vanilla);
        assert_eq!(d.item_id("Farming.Hoe"), Some(2));
        assert_eq!(d.object_name(5), Some("Door"));
        assert_eq!(d.object_id("Door"), Some(5));
        let g = d.string_register("Generic").unwrap();
        assert_eq!(g.name_of(1), Some("Base.Thing"));
        assert!(g.entries[0].is_loaded);
    }
}
