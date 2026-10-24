//! `IsoSprite` (map-relevant fields), `IsoSpriteGrid` and `IsoSpriteManager`.
//!
//! Sprites live in an arena; maps and grids hold arena indices. A sprite
//! replaced in `namedMap`/`intMap` by a duplicate `AddSprite` stays in the
//! arena (the original keeps it alive through the loader's buffer).

use std::collections::HashMap;

use super::properties::{PropertyContainer, TilePropertyAliasMap};
use super::types::{FasciaEdge, IsoObjectType, SpriteTables};

/// Index of a sprite in [`IsoSpriteManager::sprites`].
pub type SpriteIdx = usize;
/// Index of a grid in [`IsoSpriteManager::grids`].
pub type GridIdx = usize;

/// `IsoSprite.DEFAULT_SPRITE_ID`.
pub const DEFAULT_SPRITE_ID: i32 = 20_000_000;

/// `IsoSprite`: the fields set while loading tile definitions.
#[derive(Clone, Debug)]
pub struct IsoSprite {
    pub name: Option<String>,
    pub tileset_name: Option<String>,
    pub tile_sheet_index: i32,
    pub id: i32,
    pub tile_type: IsoObjectType,
    pub properties: PropertyContainer,
    pub firerequirement: i32,
    pub burnt_tile: Option<String>,
    pub force_ambient: bool,
    pub solidfloor: bool,
    pub can_be_removed: bool,
    pub attached_floor: bool,
    pub cut_w: bool,
    pub cut_n: bool,
    pub solid: bool,
    pub solid_trans: bool,
    pub invisible: bool,
    pub always_draw: bool,
    pub force_render: bool,
    pub move_with_wind: bool,
    pub is_bush: bool,
    /// `RL_DEFAULT` (0) or `RL_FLOOR` (1).
    pub render_layer: u8,
    pub wind_type: i32,
    pub treat_as_wall_order: bool,
    pub parent_object_name: Option<String>,
    pub sprite_grid: Option<GridIdx>,
    pub fascia_edge: Option<FasciaEdge>,
}

impl IsoSprite {
    pub fn new(tables: &SpriteTables) -> Self {
        IsoSprite {
            name: None,
            tileset_name: None,
            tile_sheet_index: 0,
            id: DEFAULT_SPRITE_ID,
            tile_type: tables.object_type_max(),
            properties: PropertyContainer::default(),
            firerequirement: 0,
            burnt_tile: None,
            force_ambient: false,
            solidfloor: false,
            can_be_removed: false,
            attached_floor: false,
            cut_w: false,
            cut_n: false,
            solid: false,
            solid_trans: false,
            invisible: false,
            always_draw: false,
            force_render: false,
            move_with_wind: false,
            is_bush: false,
            render_layer: 0,
            wind_type: 1,
            treat_as_wall_order: false,
            parent_object_name: None,
            sprite_grid: None,
            fascia_edge: None,
        }
    }

    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("")
    }
}

/// `IsoSpriteGrid`.
#[derive(Clone, Debug)]
pub struct IsoSpriteGrid {
    pub sprites: Vec<Option<SpriteIdx>>,
    pub width: i32,
    pub height: i32,
    pub levels: i32,
}

impl IsoSpriteGrid {
    pub fn new(width: i32, height: i32, levels: i32) -> Self {
        IsoSpriteGrid {
            sprites: vec![None; (width * height * levels).max(0) as usize],
            width,
            height,
            levels,
        }
    }

    pub fn is_valid_xyz(&self, x: i32, y: i32, z: i32) -> bool {
        x >= 0 && x < self.width && y >= 0 && y < self.height && z >= 0 && z < self.levels
    }

    pub fn sprite_index(&self, x: i32, y: i32, z: i32) -> i32 {
        if self.is_valid_xyz(x, y, z) {
            x + y * self.width + z * self.width * self.height
        } else {
            -1
        }
    }

    pub fn set_sprite(&mut self, x: i32, y: i32, z: i32, s: SpriteIdx) {
        if self.is_valid_xyz(x, y, z) {
            let i = self.sprite_index(x, y, z) as usize;
            self.sprites[i] = Some(s);
        }
    }

    pub fn get_sprite(&self, x: i32, y: i32, z: i32) -> Option<SpriteIdx> {
        if self.is_valid_xyz(x, y, z) {
            self.sprites[self.sprite_index(x, y, z) as usize]
        } else {
            None
        }
    }

    /// `getAnchorSprite`: first sprite of level 0.
    pub fn anchor_sprite(&self) -> Option<SpriteIdx> {
        (0..(self.width * self.height) as usize).find_map(|i| self.sprites[i])
    }

    /// `validate`: at least one level, none empty.
    pub fn validate(&self) -> bool {
        if self.levels == 0 {
            return false;
        }
        let per = (self.width * self.height) as usize;
        (0..self.levels as usize).all(|z| {
            self.sprites[z * per..(z + 1) * per]
                .iter()
                .any(|s| s.is_some())
        })
    }

    /// Position of a sprite as `(x, y, z)`.
    pub fn sprite_pos(&self, s: SpriteIdx) -> Option<(i32, i32, i32)> {
        let i = self.sprites.iter().position(|&x| x == Some(s))? as i32;
        let per = self.width * self.height;
        Some(((i % per) % self.width, (i % per) / self.width, i / per))
    }
}

/// `IsoSpriteManager` plus the property alias map its sprites use
/// (`TilePropertyAliasMap.instance` upstream).
#[derive(Clone, Debug)]
pub struct IsoSpriteManager {
    pub tables: SpriteTables,
    pub alias: TilePropertyAliasMap,
    pub sprites: Vec<IsoSprite>,
    pub grids: Vec<IsoSpriteGrid>,
    pub named_map: HashMap<String, SpriteIdx>,
    pub int_map: HashMap<i32, SpriteIdx>,
}

impl IsoSpriteManager {
    /// `new IsoSpriteManager()`: holds the empty sprite (`""`, id -1,
    /// `invisible`) in `namedMap` only.
    pub fn new(tables: SpriteTables) -> Self {
        let mut empty = IsoSprite::new(&tables);
        empty.name = Some(String::new());
        empty.id = -1;
        empty.properties.set_flag(tables.flag.invisible);
        let mut named_map = HashMap::new();
        named_map.insert(String::new(), 0);
        IsoSpriteManager {
            tables,
            alias: TilePropertyAliasMap::default(),
            sprites: vec![empty],
            grids: Vec::new(),
            named_map,
            int_map: HashMap::new(),
        }
    }

    fn new_sprite(&mut self) -> SpriteIdx {
        self.sprites.push(IsoSprite::new(&self.tables));
        self.sprites.len() - 1
    }

    /// `AddSprite(tex)`: a new sprite under `tex` in `namedMap`.
    pub fn add_sprite_named(&mut self, tex: &str) -> SpriteIdx {
        let s = self.new_sprite();
        self.named_map.insert(tex.to_string(), s);
        s
    }

    /// `AddSprite(tex, id)`: a new sprite in both maps. For a name already
    /// present the new sprite replaces the old one and takes the old id.
    pub fn add_sprite(&mut self, tex: &str, id: i32) -> SpriteIdx {
        let s = self.new_sprite();
        let id = match self.named_map.get(tex) {
            Some(&old) => self.sprites[old].id,
            None => id,
        };
        self.named_map.insert(tex.to_string(), s);
        self.sprites[s].id = id;
        self.int_map.insert(id, s);
        s
    }

    pub fn by_name(&self, name: &str) -> Option<&IsoSprite> {
        self.named_map.get(name).map(|&i| &self.sprites[i])
    }

    /// `getSprite(int)` / `IsoSprite.getSprite(manager, id)` (without
    /// `WorldConverter` tileset conversions).
    pub fn by_id(&self, id: i32) -> Option<&IsoSprite> {
        self.int_map.get(&id).map(|&i| &self.sprites[i])
    }

    /// `getProperties().get(name)` of a sprite.
    pub fn prop(&self, s: &IsoSprite, name: &str) -> Option<&str> {
        s.properties.get(&self.alias, name)
    }

    /// `getProperties().has(name)` of a sprite.
    pub fn has_prop(&self, s: &IsoSprite, name: &str) -> bool {
        s.properties.has_name(&self.alias, name)
    }
}

impl crate::chunk::SpriteLookup for IsoSpriteManager {
    fn sprite_id(&self, name: &str) -> Option<i32> {
        self.by_name(name).map(|s| s.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::SpriteLookup;

    #[test]
    fn add_sprite_duplicates() {
        let mut m = IsoSpriteManager::new(super::super::types::test_tables());
        assert_eq!(m.sprite_id(""), Some(-1));
        let a = m.add_sprite("a_0", 5);
        let b = m.add_sprite("a_0", 9);
        assert_ne!(a, b);
        assert_eq!(m.sprite_id("a_0"), Some(5));
        assert_eq!(m.named_map["a_0"], b);
        assert_eq!(m.int_map[&5], b);
        assert!(!m.int_map.contains_key(&9));
        let c = m.add_sprite_named("x.png");
        assert_eq!(m.sprites[c].id, DEFAULT_SPRITE_ID);
    }

    #[test]
    fn grid() {
        let mut g = IsoSpriteGrid::new(2, 1, 2);
        assert!(!g.validate());
        g.set_sprite(1, 0, 0, 7);
        g.set_sprite(0, 0, 1, 8);
        g.set_sprite(2, 0, 0, 9);
        assert!(g.validate());
        assert_eq!(g.anchor_sprite(), Some(7));
        assert_eq!(g.sprite_pos(8), Some((0, 0, 1)));
    }
}
