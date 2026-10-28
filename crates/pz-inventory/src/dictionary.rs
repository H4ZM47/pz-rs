//! The item part of `WorldDictionary.bin` (`DictionaryData.loadFromByteBuffer`):
//! registry id to full type, and the flags that decide whether
//! `InventoryItemFactory.CreateItem(registryID)` can create the item.

use std::collections::{HashMap, HashSet};

use crate::io::{err, Reader, SaveResult, Writer};

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
    fn save(&self, w: &mut Writer, mod_ids: &[String], modules: &[String]) -> SaveResult<()> {
        fn index(w: &mut Writer, list: &[String], value: &str) -> SaveResult<()> {
            let i = list.iter().position(|s| s == value).ok_or_else(|| {
                crate::io::SaveError(format!("dictionary name not indexed: {value}"))
            })?;
            if list.len() > 127 {
                w.i16(
                    i16::try_from(i)
                        .map_err(|_| crate::io::SaveError("dictionary index overflow".into()))?,
                );
            } else {
                w.i8(i as i8);
            }
            Ok(())
        }
        w.i16(self.registry_id);
        index(w, modules, &self.module_name)?;
        w.string(&self.name);
        let mut bits = u8::from(self.is_modded)
            | (u8::from(self.exists_as_vanilla) << 1)
            | (u8::from(self.obsolete) << 2)
            | (u8::from(self.removed) << 3);
        if let Some(overrides) = &self.mod_overrides {
            if overrides.len() > i8::MAX as usize {
                return err("too many dictionary mod overrides");
            }
            bits |= 16;
            if overrides.len() != 1 {
                bits |= 32;
            }
        }
        w.i8(bits as i8);
        if self.is_modded {
            index(w, mod_ids, &self.mod_id)?;
        }
        if let Some(overrides) = &self.mod_overrides {
            if overrides.len() != 1 {
                w.i8(overrides.len() as i8);
            }
            for id in overrides {
                index(w, mod_ids, id)?;
            }
        }
        Ok(())
    }

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
        if b.remaining() != 0 {
            return err("dictionary ByteBlock size mismatch");
        }
        out.push(reg);
    }
    Ok(out)
}

fn save_registers(w: &mut Writer, registers: &[Register], scripts: bool) -> SaveResult<()> {
    w.i32(registers.len() as i32);
    for register in registers {
        w.string(&register.name);
        let mut b = Writer::new();
        b.i16(register.next_id);
        b.i32(register.entries.len() as i32);
        for entry in &register.entries {
            let short_name = entry.name.strip_prefix("Base.").filter(|s| !s.is_empty());
            b.i8(i8::from(short_name.is_some()) | (i8::from(entry.is_loaded) << 1));
            b.i16(entry.registry_id);
            if scripts {
                b.i64(entry.version.ok_or_else(|| {
                    crate::io::SaveError("script dictionary version missing".into())
                })?);
            }
            b.string(short_name.unwrap_or(&entry.name));
        }
        w.i32(b.buf.len() as i32);
        w.bytes(&b.buf);
    }
    Ok(())
}

/// The tables of a `WorldDictionary.bin`.
#[derive(Clone, Debug, Default)]
pub struct WorldDictionary {
    pub version: i32,
    /// Next unused id, shared by item and entity scripts. These counters
    /// are part of the file, not inferred from the currently loaded mods.
    pub next_info_id: i16,
    pub next_object_name_id: i8,
    pub next_sprite_name_id: i32,
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
    /// `DictionaryData.reset`: the first item/entity and object ids are 0.
    pub fn new() -> Self {
        Self {
            version: 1,
            ..Self::default()
        }
    }

    /// Reads the whole file (`DictionaryData.loadFromByteBuffer`).
    pub fn load(bytes: &[u8]) -> SaveResult<Self> {
        let r = &mut Reader::new(bytes);
        let version = r.i32()?;
        let next_info_id = r.i16()?;
        let next_object_name_id = r.i8()?;
        let next_sprite_name_id = r.i32()?;
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
            next_info_id,
            next_object_name_id,
            next_sprite_name_id,
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

    /// `DictionaryData.saveToByteBuffer`, including the string/script
    /// registers. Existing ids, counters and removed entries survive a
    /// load/save cycle even if their mod is no longer installed.
    pub fn save(&self) -> SaveResult<Vec<u8>> {
        let mut w = Writer::new();
        w.i32(1);
        w.i16(self.next_info_id);
        w.i8(self.next_object_name_id);
        w.i32(self.next_sprite_name_id);
        let mut items: Vec<_> = self.items.values().collect();
        let mut entities: Vec<_> = self.entities.values().collect();
        items.sort_by_key(|i| i.registry_id);
        entities.sort_by_key(|i| i.registry_id);
        let mut mods = Vec::new();
        let mut modules = Vec::new();
        for info in items.iter().chain(&entities) {
            if !mods.contains(&info.mod_id) {
                mods.push(info.mod_id.clone());
            }
            for id in info.mod_overrides.iter().flatten() {
                if !mods.contains(id) {
                    mods.push(id.clone());
                }
            }
            if !modules.contains(&info.module_name) {
                modules.push(info.module_name.clone());
            }
        }
        for names in [&mods, &modules] {
            w.i32(names.len() as i32);
            for name in names {
                w.string(name);
            }
        }
        for infos in [&items, &entities] {
            w.i32(infos.len() as i32);
            for info in infos {
                info.save(&mut w, &mods, &modules)?;
            }
        }
        let mut objects: Vec<_> = self.object_names.iter().collect();
        objects.sort_by_key(|(id, _)| **id);
        w.i32(objects.len() as i32);
        for (&id, name) in objects {
            w.i8(id);
            w.string(name);
        }
        let mut sprites: Vec<_> = self.sprite_names.iter().collect();
        sprites.sort_by_key(|(id, _)| **id);
        w.i32(sprites.len() as i32);
        for (&id, name) in sprites {
            w.i32(id);
            w.string(name);
        }
        save_registers(&mut w, &self.string_registers, false)?;
        save_registers(&mut w, &self.script_registers, true)?;
        Ok(w.buf)
    }

    /// `parseInfoLoadList` followed by `parseCurrentInfoSet`. Call items
    /// before entities, using the original load-list iteration order.
    /// New obsolete scripts do not consume an id; old missing scripts
    /// retain theirs and are marked removed so old saves stay readable.
    pub fn reconcile_scripts(
        &mut self,
        items: &[DictionaryInfo],
        entities: &[DictionaryInfo],
    ) -> SaveResult<()> {
        fn parse(
            map: &mut HashMap<i16, DictionaryInfo>,
            current: &[DictionaryInfo],
            next: &mut i16,
            used: &mut HashSet<i16>,
        ) -> SaveResult<()> {
            let mut ids: HashMap<String, i16> = map
                .values()
                .map(|i| (i.full_type.clone(), i.registry_id))
                .collect();
            let mut loaded = HashSet::new();
            for info in current {
                if let Some(&id) = ids.get(&info.full_type) {
                    let stored = map.get_mut(&id).unwrap();
                    if stored.removed && !info.obsolete {
                        stored.removed = false;
                    }
                    if stored.mod_id != info.mod_id {
                        stored.mod_id = info.mod_id.clone();
                        stored.is_modded = info.mod_id != "pz-vanilla";
                    }
                    if info.obsolete {
                        stored.obsolete = true;
                        stored.removed = true;
                    }
                    loaded.insert(id);
                } else if !info.obsolete {
                    if *next >= i16::MAX {
                        return err("Max item ID value reached for WorldDictionary!");
                    }
                    let id = *next;
                    if !used.insert(id) {
                        return err(format!("dictionary next id {id} is already assigned"));
                    }
                    *next += 1;
                    let mut info = info.clone();
                    info.registry_id = id;
                    info.removed = false;
                    ids.insert(info.full_type.clone(), id);
                    map.insert(id, info);
                    loaded.insert(id);
                }
            }
            for (&id, info) in map.iter_mut() {
                if !loaded.contains(&id) {
                    info.removed = true;
                }
            }
            Ok(())
        }
        let mut used = self
            .items
            .keys()
            .chain(self.entities.keys())
            .copied()
            .collect();
        parse(&mut self.items, items, &mut self.next_info_id, &mut used)?;
        parse(
            &mut self.entities,
            entities,
            &mut self.next_info_id,
            &mut used,
        )
    }

    /// `parseObjectNameLoadList`: never remove old erosion object names.
    pub fn register_object_names(&mut self, names: &[String], debug: bool) -> SaveResult<()> {
        for name in names {
            if self.object_id(name).is_some() {
                continue;
            }
            if self.next_object_name_id >= i8::MAX {
                if debug {
                    return err("Max value for object names reached.");
                }
                continue;
            }
            let id = self.next_object_name_id;
            if self.object_names.contains_key(&id) {
                return err(format!("dictionary object id {id} is already assigned"));
            }
            self.next_object_name_id += 1;
            self.object_names.insert(id, name.clone());
        }
        Ok(())
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

    /// `getEntityRegistryID(fullType)`.
    pub fn entity_id(&self, full_type: &str) -> Option<i16> {
        self.entities
            .values()
            .find(|i| i.full_type == full_type)
            .map(|i| i.registry_id)
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

    fn info(full_type: &str) -> DictionaryInfo {
        let (module, name) = full_type.split_once('.').unwrap();
        DictionaryInfo {
            registry_id: -1,
            module_name: module.into(),
            name: name.into(),
            full_type: full_type.into(),
            mod_id: "pz-vanilla".into(),
            is_modded: false,
            exists_as_vanilla: true,
            obsolete: false,
            removed: false,
            mod_overrides: None,
        }
    }

    #[test]
    fn allocates_shared_ids_preserves_retired_entries_and_counter_on_reload() {
        let mut d = WorldDictionary::new();
        let a = info("Example.Pebble");
        let mut obsolete = info("Example.Deprecated");
        obsolete.obsolete = true;
        d.reconcile_scripts(&[a.clone(), obsolete], &[info("Example.Workbench")])
            .unwrap();
        assert_eq!(d.item_id("Example.Pebble"), Some(0));
        assert_eq!(d.item_id("Example.Deprecated"), None);
        assert_eq!(d.entity_id("Example.Workbench"), Some(1));
        assert_eq!(d.next_info_id, 2);
        d.register_object_names(&["Fern".into(), "Shrub".into(), "Fern".into()], false)
            .unwrap();
        assert_eq!(d.object_id("Fern"), Some(0));
        assert_eq!(d.next_object_name_id, 2);

        let mut d = WorldDictionary::load(&d.save().unwrap()).unwrap();
        d.reconcile_scripts(&[info("Example.Cord")], &[]).unwrap();
        assert_eq!(d.item_id("Example.Cord"), Some(2));
        assert!(d.items[&0].removed && d.entities[&1].removed);
        d.register_object_names(&["Shrub".into(), "Sapling".into()], false)
            .unwrap();
        assert_eq!(d.object_id("Fern"), Some(0));
        assert_eq!(d.object_id("Sapling"), Some(2));

        let mut modded = a;
        modded.mod_id = "example-mod".into();
        modded.is_modded = true;
        d.reconcile_scripts(&[modded.clone()], &[]).unwrap();
        assert_eq!(d.item_id("Example.Pebble"), Some(0));
        assert!(!d.items[&0].removed);
        assert_eq!(d.items[&0].mod_id, "example-mod");
        modded.obsolete = true;
        d.reconcile_scripts(&[modded], &[]).unwrap();
        assert!(d.items[&0].obsolete && d.items[&0].removed);
        assert_eq!(d.next_info_id, 3);
    }

    #[test]
    fn serializes_flags_registers_wide_indices_and_counters() {
        let mut d = WorldDictionary::new();
        let infos: Vec<_> = (0..128)
            .map(|n| {
                let mut i = info(&format!("Module{n}.Token"));
                i.mod_id = format!("mod-{n}");
                i.is_modded = true;
                i
            })
            .collect();
        d.reconcile_scripts(&infos, &[info("EntityModule.Widget")])
            .unwrap();
        d.items.get_mut(&0).unwrap().mod_overrides = Some(vec!["old-a".into(), "old-b".into()]);
        d.items.get_mut(&1).unwrap().mod_overrides = Some(vec!["old-a".into()]);
        d.items.get_mut(&2).unwrap().mod_overrides = Some(Vec::new());
        d.items.get_mut(&3).unwrap().removed = true;
        d.items.get_mut(&4).unwrap().obsolete = true;
        d.next_sprite_name_id = 81;
        d.sprite_names.insert(80, "example_sprite".into());
        d.string_registers.push(Register {
            name: "Generic".into(),
            next_id: 6,
            entries: vec![RegisterEntry {
                registry_id: 5,
                name: "Base.Layout".into(),
                version: None,
                is_loaded: true,
            }],
        });
        d.script_registers.push(Register {
            name: "SpriteConfigs".into(),
            next_id: 10,
            entries: vec![RegisterEntry {
                registry_id: 9,
                name: "Example.Grid".into(),
                version: Some(i64::MIN + 12),
                is_loaded: false,
            }],
        });
        let saved = d.save().unwrap();
        let loaded = WorldDictionary::load(&saved).unwrap();
        assert_eq!(loaded.items, d.items);
        assert_eq!(loaded.entities, d.entities);
        assert_eq!(loaded.sprite_names, d.sprite_names);
        assert_eq!(loaded.next_info_id, 129);
        assert_eq!(loaded.next_sprite_name_id, 81);
        assert_eq!(loaded.string_registers, d.string_registers);
        assert_eq!(loaded.script_registers, d.script_registers);
        assert_eq!(loaded.save().unwrap(), saved);
    }

    #[test]
    fn does_not_reuse_ids_at_allocation_limits_or_bad_saved_counters() {
        let mut d = WorldDictionary::new();
        d.next_info_id = i16::MAX;
        assert!(d.reconcile_scripts(&[info("Example.Limit")], &[]).is_err());
        d.next_info_id = 0;
        d.reconcile_scripts(&[info("Example.Pebble")], &[]).unwrap();
        d.next_info_id = 0;
        assert!(d.reconcile_scripts(&[info("Example.New")], &[]).is_err());
        assert_eq!(d.item_type(0), Some("Example.Pebble"));
        d.next_object_name_id = i8::MAX;
        d.register_object_names(&["Overflow".into()], false)
            .unwrap();
        assert_eq!(d.object_id("Overflow"), None);
        assert!(d.register_object_names(&["Overflow".into()], true).is_err());
    }

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
