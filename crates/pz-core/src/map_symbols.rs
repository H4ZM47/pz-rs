//! `zombie.worldMap.symbols.MapSymbolDefinitions`: the catalogue of
//! texture symbols a player can stamp on the world map, filled at boot by
//! `shared/Definitions/MapSymbolDefinitions.lua`.
//!
//! Engine-only data (no textures): the caller decides whether the texture
//! exists, which picks the default symbol size like the original does.

use std::collections::HashMap;
use std::rc::Rc;

/// Symbol size `addTexture(id, path[, tab])` uses when the texture loads.
pub const SIZE_TEXTURE_FOUND: i32 = 20;
/// Symbol size `addTexture(id, path[, tab])` uses when it does not.
pub const SIZE_TEXTURE_MISSING: i32 = 18;

/// `MapSymbolDefinitions.MapSymbolDefinition`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapSymbolDefinition {
    pub id: String,
    pub texture_path: String,
    pub width: i32,
    pub height: i32,
    /// `null` unless a tab was given.
    pub tab: Option<String>,
}

/// The definitions in insertion order plus an id index. Re-adding an id
/// appends another entry and repoints the index, as `ArrayList` +
/// `HashMap.put` do.
#[derive(Debug, Default)]
pub struct MapSymbolDefinitions {
    list: Vec<Rc<MapSymbolDefinition>>,
    by_id: HashMap<String, Rc<MapSymbolDefinition>>,
}

impl MapSymbolDefinitions {
    pub fn new() -> Self {
        Self::default()
    }

    /// `addTexture(id, path, width, height, tab)`.
    pub fn add_texture_sized(
        &mut self,
        id: &str,
        path: &str,
        width: i32,
        height: i32,
        tab: Option<&str>,
    ) {
        let def = Rc::new(MapSymbolDefinition {
            id: id.to_string(),
            texture_path: path.to_string(),
            width,
            height,
            tab: tab.map(str::to_string),
        });
        self.list.push(def.clone());
        self.by_id.insert(id.to_string(), def);
    }

    /// `addTexture(id, path)` / `addTexture(id, path, tab)`: 20x20 when the
    /// texture exists, 18x18 otherwise.
    pub fn add_texture(&mut self, id: &str, path: &str, tab: Option<&str>, texture_found: bool) {
        let size = if texture_found {
            SIZE_TEXTURE_FOUND
        } else {
            SIZE_TEXTURE_MISSING
        };
        self.add_texture_sized(id, path, size, size, tab);
    }

    pub fn symbol_count(&self) -> usize {
        self.list.len()
    }

    pub fn symbol_by_index(&self, index: usize) -> Option<&Rc<MapSymbolDefinition>> {
        self.list.get(index)
    }

    pub fn symbol_by_id(&self, id: &str) -> Option<&Rc<MapSymbolDefinition>> {
        self.by_id.get(id)
    }

    /// `Reset()` body: drop every definition.
    pub fn clear(&mut self) {
        self.list.clear();
        self.by_id.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_lookup_reset() {
        let mut d = MapSymbolDefinitions::new();
        d.add_texture("A", "media/a.png", Some("Tab"), true);
        d.add_texture("B", "media/b.png", None, false);
        d.add_texture_sized("A", "media/a2.png", 5, 6, None);
        assert_eq!(d.symbol_count(), 3);
        assert_eq!(d.symbol_by_index(0).unwrap().width, 20);
        assert_eq!(d.symbol_by_index(0).unwrap().tab.as_deref(), Some("Tab"));
        assert_eq!(d.symbol_by_index(1).unwrap().height, 18);
        assert_eq!(d.symbol_by_id("A").unwrap().texture_path, "media/a2.png");
        assert!(d.symbol_by_id("C").is_none());
        d.clear();
        assert_eq!(d.symbol_count(), 0);
        assert!(d.symbol_by_id("A").is_none());
    }
}
