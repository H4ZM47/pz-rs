//! `ClothingDecal`, `ClothingDecalGroup` and `ClothingDecals`
//! (`media/clothing/clothingDecals.xml`, decals in
//! `media/clothing/clothingDecals/<name>.xml`).
//!
//! Differences from the original, all on input it cannot handle either:
//! * A group without `<name>` is skipped by name lookups (the original
//!   throws `NullPointerException` when it reaches one).
//! * Groups that include each other recursively stop after
//!   [`MAX_GROUP_DEPTH`] levels and pick nothing (the original overflows
//!   the stack).
//! * Decal files are read without `PZXmlUtil`'s `x_include`/`x_extends`
//!   resolution and file GUIDs. TODO(PZXmlUtil).

use super::xml::{parse_root, Element};
use super::{base_file, mod_files, parse_file};
use pz_core::random::outfit_rng;
use pz_fs::java::is_null_or_whitespace;
use pz_fs::GameFs;
use pz_scripts::java::eq_ignore_case;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

pub const CLOTHING_DECALS_FILE: &str = "media/clothing/clothingDecals.xml";

/// Group nesting at which [`ClothingDecalGroup::get_decals`] gives up.
pub const MAX_GROUP_DEPTH: usize = 512;

/// `ClothingDecal`: a texture drawn at a rectangle of the clothing texture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClothingDecal {
    /// The name it was loaded by (not read from the file).
    pub name: Option<String>,
    pub texture: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ClothingDecal {
    /// Unmarshal a `<clothingDecal>` document.
    pub fn from_xml(bytes: &[u8]) -> Result<Self, String> {
        let e = parse_root(bytes, "clothingDecal")?;
        Ok(ClothingDecal {
            name: None,
            texture: e.string_field("texture"),
            x: e.int_field("x").unwrap_or(0),
            y: e.int_field("y").unwrap_or(0),
            width: e.int_field("width").unwrap_or(0),
            height: e.int_field("height").unwrap_or(0),
        })
    }

    /// `isValid()`.
    pub fn is_valid(&self) -> bool {
        !self.texture.as_deref().is_none_or(is_null_or_whitespace)
            && self.x >= 0
            && self.y >= 0
            && self.width > 0
            && self.height > 0
    }
}

/// `ClothingDecalGroup`: decal names plus included groups.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClothingDecalGroup {
    pub name: Option<String>,
    pub decals: Vec<String>,
    pub groups: Vec<String>,
}

impl ClothingDecalGroup {
    pub fn from_element(e: &Element) -> Self {
        ClothingDecalGroup {
            name: e.string_field("name"),
            decals: e.string_list("decal"),
            groups: e.string_list("group"),
        }
    }

    /// `getDecals(decals)`: this group's decals, then each included group's
    /// (looked up in `all`), depth first. `false` if the nesting exceeded
    /// [`MAX_GROUP_DEPTH`].
    pub fn get_decals(&self, all: &ClothingDecals, out: &mut Vec<String>) -> bool {
        self.get_decals_depth(all, out, 0)
    }

    fn get_decals_depth(&self, all: &ClothingDecals, out: &mut Vec<String>, depth: usize) -> bool {
        if depth > MAX_GROUP_DEPTH {
            return false;
        }
        out.extend(self.decals.iter().cloned());
        for g in &self.groups {
            if let Some(group) = all.find_group(g) {
                if !group.get_decals_depth(all, out, depth + 1) {
                    return false;
                }
            }
        }
        true
    }

    /// `getRandomDecal()`: one of [`Self::get_decals`] by `OutfitRNG`.
    pub fn get_random_decal(&self, all: &ClothingDecals) -> Option<String> {
        let mut decals = Vec::new();
        if !self.get_decals(all, &mut decals) {
            eprintln!(
                "ERROR: clothing decal group \"{}\" includes itself",
                self.name.as_deref().unwrap_or("null")
            );
            return None;
        }
        outfit_rng::pick_random(&decals).cloned()
    }
}

/// `ClothingDecals`.
#[derive(Debug, Default)]
pub struct ClothingDecals {
    pub groups: Vec<ClothingDecalGroup>,
    /// `cachedDecals`: decals loaded by name; a failed load is retried.
    cached: RefCell<HashMap<String, Option<Rc<ClothingDecal>>>>,
}

impl ClothingDecals {
    /// Unmarshal a `<clothingDecals>` document.
    pub fn from_xml(bytes: &[u8]) -> Result<Self, String> {
        let root = parse_root(bytes, "clothingDecals")?;
        Ok(ClothingDecals {
            groups: root
                .children_named("group")
                .map(ClothingDecalGroup::from_element)
                .collect(),
            cached: RefCell::default(),
        })
    }

    /// `Parse(filename)`.
    pub fn parse(path: &str) -> Option<Self> {
        parse_file(path, Self::from_xml)
    }

    /// `init()`: the game's file, then each mod's groups (a group whose
    /// name matches replaces it in place).
    pub fn init(fs: &mut GameFs) -> Option<Self> {
        let mut inst = Self::parse(&base_file(fs, CLOTHING_DECALS_FILE))?;
        for (_mod_id, m) in mod_files(fs, CLOTHING_DECALS_FILE, Self::parse) {
            for group in m.groups {
                match inst.find_group_index(group.name.as_deref()) {
                    Some(i) => inst.groups[i] = group,
                    None => inst.groups.push(group),
                }
            }
        }
        Some(inst)
    }

    fn find_group_index(&self, name: Option<&str>) -> Option<usize> {
        let name = name.filter(|n| !is_null_or_whitespace(n))?;
        self.groups
            .iter()
            .position(|g| g.name.as_deref().is_some_and(|gn| eq_ignore_case(gn, name)))
    }

    /// `FindGroup(name)`: by name ignoring case.
    pub fn find_group(&self, name: &str) -> Option<&ClothingDecalGroup> {
        self.find_group_index(Some(name)).map(|i| &self.groups[i])
    }

    /// `getRandomDecal(groupName)`.
    pub fn get_random_decal(&self, group: &str) -> Option<String> {
        self.find_group(group)?.get_random_decal(self)
    }

    /// `getDecal(name)`: `media/clothing/clothingDecals/<name>.xml` through
    /// the active file map, cached by name. A file that fails to load is
    /// logged and `None`.
    pub fn get_decal(&self, fs: &GameFs, name: &str) -> Option<Rc<ClothingDecal>> {
        if is_null_or_whitespace(name) {
            return None;
        }
        if let Some(Some(d)) = self.cached.borrow().get(name) {
            return Some(d.clone());
        }
        let file = fs.get_string(&format!("media/clothing/clothingDecals/{name}.xml"));
        let path = if Path::new(&file).is_absolute() {
            file.clone()
        } else {
            fs.base.join(&file).to_string_lossy().into_owned()
        };
        let loaded = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| ClothingDecal::from_xml(&b));
        let decal = match loaded {
            Ok(mut d) => {
                d.name = Some(name.to_string());
                Some(Rc::new(d))
            }
            Err(e) => {
                eprintln!("Failed to load ClothingDecal: {file} ({e})");
                None
            }
        };
        self.cached
            .borrow_mut()
            .insert(name.to_string(), decal.clone());
        decal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &[u8] = b"<clothingDecals><group><name>G1</name><decal>a</decal><group>G2</group><group>missing</group>\
        <decal>b</decal></group><group><name>g2</name><decal>c</decal></group><group><decal>z</decal></group>\
        <group><name>Loop</name><group>loop</group></group></clothingDecals>";

    #[test]
    fn groups_resolve_depth_first() {
        let d = ClothingDecals::from_xml(DOC).unwrap();
        assert_eq!(d.groups[2].name, None);
        let mut out = Vec::new();
        assert!(d.find_group("g1").unwrap().get_decals(&d, &mut out));
        assert_eq!(out, ["a", "b", "c"]);
        assert!(d.find_group(" ").is_none());
        assert!(d.get_random_decal("nope").is_none());
        assert!(d.get_random_decal("Loop").is_none());
        // One decal: no draw.
        outfit_rng::set_seed(5);
        let first = outfit_rng::next(1000);
        outfit_rng::set_seed(5);
        assert_eq!(d.get_random_decal("G2").as_deref(), Some("c"));
        assert_eq!(outfit_rng::next(1000), first);
    }

    #[test]
    fn decal_binding() {
        let d = ClothingDecal::from_xml(b"<clothingDecal><texture>t/x</texture><x>1</x><y> 2</y><width>3</width><height>4</height></clothingDecal>").unwrap();
        assert!(d.is_valid());
        let bad = ClothingDecal::from_xml(
            b"<clothingDecal><texture>  </texture><x>-1</x><width>bad</width></clothingDecal>",
        )
        .unwrap();
        assert_eq!((bad.x, bad.width), (-1, 0));
        assert!(!bad.is_valid());
    }
}
