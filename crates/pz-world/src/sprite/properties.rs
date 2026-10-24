//! `TilePropertyAliasMap` (`zombie.core`) and `PropertyContainer`
//! (`zombie.core.properties`).
//!
//! The original keeps the alias map as a global singleton; here it is passed
//! to the `PropertyContainer` methods that need it.

use std::collections::HashMap;

use super::types::{IsoFlagType, SpriteTables};
use crate::java::hashmap_order;

/// `IsoWorld.PropertyValueMap`: property name to the distinct values seen, as
/// a `HashMap<String, ArrayList<String>>`. Key insertion order is kept so the
/// Java iteration order can be reproduced.
#[derive(Clone, Debug, Default)]
pub struct PropertyValueMap {
    keys: Vec<String>,
    map: HashMap<String, Vec<String>>,
}

impl PropertyValueMap {
    pub fn get(&self, key: &str) -> Option<&Vec<String>> {
        self.map.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Vec<String>> {
        self.map.get_mut(key)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// `HashMap.put`: replaces the value; a new key goes to the end.
    pub fn put(&mut self, key: &str, values: Vec<String>) {
        if self.map.insert(key.to_string(), values).is_none() {
            self.keys.push(key.to_string());
        }
    }

    /// The values list for `key`, created empty if missing.
    pub fn entry(&mut self, key: &str) -> &mut Vec<String> {
        if !self.map.contains_key(key) {
            self.keys.push(key.to_string());
        }
        self.map.entry(key.to_string()).or_default()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Keys in `java.util.HashMap` iteration order.
    pub fn java_order(&self) -> Vec<&str> {
        let keys: Vec<&str> = self.keys.iter().map(|s| s.as_str()).collect();
        hashmap_order(&keys)
    }

    pub fn clear(&mut self) {
        self.keys.clear();
        self.map.clear();
    }
}

/// `TilePropertyAliasMap.TileProperty`.
#[derive(Clone, Debug, Default)]
pub struct TileProperty {
    pub property_name: String,
    pub possible_values: Vec<String>,
    /// Value to index; a repeated value maps to its last index.
    pub id_map: HashMap<String, i32>,
}

/// `TilePropertyAliasMap`: property names and values interned to ids.
#[derive(Clone, Debug, Default)]
pub struct TilePropertyAliasMap {
    pub property_to_id: HashMap<String, i32>,
    pub properties: Vec<TileProperty>,
}

impl TilePropertyAliasMap {
    /// `Generate(propertyValueMap)`: one property per key in the map's
    /// iteration order, then `EntityScriptName` with the game entity script
    /// names (`ScriptManager.getAllGameEntities()`).
    pub fn generate(map: &PropertyValueMap, entity_script_names: &[String]) -> Self {
        let mut a = TilePropertyAliasMap::default();
        for key in map.java_order() {
            a.register(key, map.get(key).expect("key"));
        }
        a.register("EntityScriptName", entity_script_names);
        a
    }

    fn register(&mut self, key: &str, values: &[String]) {
        assert!(
            self.properties.len() < i16::MAX as usize,
            "too many properties defined"
        );
        assert!(
            values.len() <= i16::MAX as usize,
            "too many property values defined for {key}"
        );
        // `IsoPropertyType.lookupOrDefaultStr(key)` is `key` either way.
        let property = key.to_string();
        self.property_to_id
            .insert(property.clone(), self.properties.len() as i32);
        let mut p = TileProperty {
            property_name: property,
            possible_values: values.to_vec(),
            id_map: HashMap::new(),
        };
        for (i, v) in values.iter().enumerate() {
            p.id_map.insert(v.clone(), i as i32);
        }
        self.properties.push(p);
    }

    /// `getIDFromPropertyName`, `-1` when unknown.
    pub fn id_from_property_name(&self, name: &str) -> i32 {
        self.property_to_id.get(name).copied().unwrap_or(-1)
    }

    /// `getIDFromPropertyValue`: `0` for an unknown value.
    pub fn id_from_property_value(&self, property: i32, value: &str) -> i32 {
        let p = &self.properties[property as usize];
        if p.possible_values.is_empty() {
            return 0;
        }
        p.id_map.get(value).copied().unwrap_or(0)
    }

    /// `getPropertyValueString`.
    pub fn property_value_string(&self, property: i32, value: i32) -> &str {
        let p = &self.properties[property as usize];
        if p.possible_values.is_empty() {
            return "";
        }
        &p.possible_values[value as usize]
    }
}

/// Surface values derived from the properties (`PropertyContainer.initSurface`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceInfo {
    pub surface: i32,
    pub is_surface_offset: bool,
    pub is_table: bool,
    pub is_table_top: bool,
    pub stack_replace_tile_offset: i32,
    pub item_height: i32,
    pub sloped_surface_height_min: i32,
    pub sloped_surface_height_max: i32,
}

/// `PropertyContainer`: flag bitsets plus a `short -> short` map of property
/// id to value id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PropertyContainer {
    sprite_flags1: u64,
    sprite_flags2: u64,
    /// `(property id, value id)` in insertion order.
    entries: Vec<(i16, i16)>,
}

impl PropertyContainer {
    pub fn flags(&self) -> (u64, u64) {
        (self.sprite_flags1, self.sprite_flags2)
    }

    pub fn has(&self, flag: IsoFlagType) -> bool {
        let i = flag.index();
        let flags = if i < 64 {
            self.sprite_flags1
        } else {
            self.sprite_flags2
        };
        flags & (1u64 << (i & 63)) != 0
    }

    pub fn set_flag(&mut self, flag: IsoFlagType) {
        let i = flag.index();
        if i / 64 == 0 {
            self.sprite_flags1 |= 1u64 << (i % 64);
        } else {
            self.sprite_flags2 |= 1u64 << (i % 64);
        }
    }

    pub fn unset_flag(&mut self, flag: IsoFlagType) {
        let i = flag.index();
        if i / 64 == 0 {
            self.sprite_flags1 &= !(1u64 << (i % 64));
        } else {
            self.sprite_flags2 &= !(1u64 << (i % 64));
        }
    }

    fn find(&self, p: i16) -> Option<usize> {
        self.entries.iter().position(|&(k, _)| k == p)
    }

    /// `set(propName, propValue, checkIsoFlagType)`. With
    /// `check_iso_flag_type` a name that is an `IsoFlagType` sets only the
    /// flag. An unknown property name is ignored; an unknown value is stored
    /// as value id 0.
    pub fn set(
        &mut self,
        tables: &SpriteTables,
        alias: &TilePropertyAliasMap,
        name: &str,
        value: &str,
        check_iso_flag_type: bool,
    ) {
        if check_iso_flag_type {
            if let Some(e) = tables.flag_from_string(name) {
                self.set_flag(e);
                return;
            }
        }
        self.set_raw(alias, name, value);
    }

    /// `set(propName, propValue, false)`.
    pub fn set_raw(&mut self, alias: &TilePropertyAliasMap, name: &str, value: &str) {
        let p = alias.id_from_property_name(name);
        if p == -1 {
            return;
        }
        let v = alias.id_from_property_value(p, value);
        let (p, v) = (p as i16, v as i16);
        match self.find(p) {
            Some(i) => self.entries[i].1 = v,
            None => self.entries.push((p, v)),
        }
    }

    /// `unset(String)`.
    pub fn unset(&mut self, alias: &TilePropertyAliasMap, name: &str) {
        let p = alias.id_from_property_name(name) as i16;
        if let Some(i) = self.find(p) {
            self.entries.remove(i);
        }
    }

    /// `has(String)`.
    pub fn has_name(&self, alias: &TilePropertyAliasMap, name: &str) -> bool {
        let p = alias.id_from_property_name(name) as i16;
        self.find(p).is_some()
    }

    /// `get(String)` (`Val` in older builds).
    pub fn get<'a>(&self, alias: &'a TilePropertyAliasMap, name: &str) -> Option<&'a str> {
        let p = alias.id_from_property_name(name);
        let i = self.find(p as i16)?;
        Some(alias.property_value_string(p, self.entries[i].1 as i32))
    }

    /// `propertyEquals`: case-insensitive, `false` when missing.
    pub fn property_equals(&self, alias: &TilePropertyAliasMap, name: &str, value: &str) -> bool {
        self.get(alias, name)
            .is_some_and(|v| super::equals_ignore_case(v, value))
    }

    /// Raw `(property id, value id)` entries.
    pub fn entries(&self) -> &[(i16, i16)] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `(name, value)` pairs sorted by name, like `getPropertyNames()`.
    pub fn property_pairs<'a>(&self, alias: &'a TilePropertyAliasMap) -> Vec<(&'a str, &'a str)> {
        let mut v: Vec<(&str, &str)> = self
            .entries
            .iter()
            .map(|&(p, val)| {
                (
                    alias.properties[p as usize].property_name.as_str(),
                    alias.property_value_string(p as i32, val as i32),
                )
            })
            .collect();
        v.sort();
        v
    }

    /// `getFlagsList()`.
    pub fn flags_list(&self, tables: &SpriteTables) -> Vec<IsoFlagType> {
        (0..tables.flag_count() as u8)
            .map(IsoFlagType)
            .filter(|f| self.has(*f))
            .collect()
    }

    /// `initSurface()` (without `SlopedSurfaceDirection`).
    pub fn surface_info(&self, alias: &TilePropertyAliasMap) -> SurfaceInfo {
        let mut s = SurfaceInfo::default();
        for &(p, v) in &self.entries {
            let tp = &alias.properties[p as usize];
            let val = tp
                .possible_values
                .get(v as usize)
                .map(|s| s.as_str())
                .unwrap_or("");
            match tp.property_name.as_str() {
                "Surface" => {
                    if let Ok(px) = val.parse::<i32>() {
                        if (0..=127).contains(&px) {
                            s.surface = px;
                        }
                    }
                }
                "IsSurfaceOffset" => s.is_surface_offset = true,
                "IsTable" => s.is_table = true,
                "IsTableTop" => s.is_table_top = true,
                "StackReplaceTileOffset" => {
                    if let Ok(x) = val.parse::<i32>() {
                        s.stack_replace_tile_offset = x as i16 as i32;
                    }
                }
                "ItemHeight" => {
                    if let Ok(px) = val.parse::<i32>() {
                        if (0..=127).contains(&px) {
                            s.item_height = px;
                        }
                    }
                }
                "SlopedSurfaceHeightMin" => {
                    s.sloped_surface_height_min = super::try_parse_int(val, 0).clamp(0, 100);
                }
                "SlopedSurfaceHeightMax" => {
                    s.sloped_surface_height_max = super::try_parse_int(val, 0).clamp(0, 100);
                }
                _ => {}
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alias() -> TilePropertyAliasMap {
        let mut m = PropertyValueMap::default();
        m.entry("Surface")
            .extend(["10".to_string(), "200".to_string()]);
        m.entry("IsTable").push(String::new());
        m.entry("Facing")
            .extend(["N".to_string(), "S".to_string(), "N".to_string()]);
        TilePropertyAliasMap::generate(&m, &[])
    }

    #[test]
    fn set_get_flags() {
        let t = super::super::types::test_tables();
        let a = alias();
        let mut p = PropertyContainer::default();
        p.set(&t, &a, "solid", "", true);
        assert!(p.has(t.flag.solid));
        p.set(&t, &a, "Facing", "N", true);
        // A repeated value maps to its last index; the string is the same.
        assert_eq!(p.entries()[0].1, 2);
        assert_eq!(p.get(&a, "Facing"), Some("N"));
        p.set(&t, &a, "Facing", "W", true);
        assert_eq!(p.get(&a, "Facing"), Some("N"));
        p.set(&t, &a, "Unknown", "x", true);
        assert!(!p.has_name(&a, "Unknown"));
        p.set(&t, &a, "Surface", "200", false);
        assert_eq!(p.surface_info(&a).surface, 0);
        p.set(&t, &a, "Surface", "10", false);
        p.set(&t, &a, "IsTable", "", false);
        let s = p.surface_info(&a);
        assert_eq!((s.surface, s.is_table), (10, true));
        p.unset(&a, "Facing");
        assert!(!p.has_name(&a, "Facing"));
        p.set_flag(t.flag.open);
        assert!(p.has(t.flag.open));
        p.unset_flag(t.flag.open);
        assert!(!p.has(t.flag.open));
        let last = alias().properties.last().unwrap().property_name.clone();
        assert_eq!(last, "EntityScriptName");
    }
}
