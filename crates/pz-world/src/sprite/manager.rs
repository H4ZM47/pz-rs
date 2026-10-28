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
    /// `CellLoader.glassRemovedWindowSpriteMap`: glass-removed window sprite
    /// to its intact window sprite (filled by
    /// [`IsoSpriteManager::loaded_tile_definitions`]).
    pub glass_removed_window_sprites: HashMap<SpriteIdx, SpriteIdx>,
    /// `CellLoader.smashedWindowSpriteMap`: smashed window sprite to its
    /// intact window sprite.
    pub smashed_window_sprites: HashMap<SpriteIdx, SpriteIdx>,
    /// `SpriteConfigManager.objectInfos` (filled by
    /// [`IsoSpriteManager::post_tile_definitions`]).
    pub sprite_configs: HashMap<String, SpriteConfigInfo>,
    /// The `SpriteConfig` script of each game entity script that has one:
    /// the entity's full type (what `ScriptsDictionary.spriteConfigs` names
    /// the script by) to the config's `getName()` (the `objectInfos` key).
    pub sprite_config_scripts: HashMap<String, String>,
}

/// `SpriteConfigManager.ObjectInfo`: per face (in face-id order), the
/// face's tile sprites and whether each is the face's master tile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpriteConfigInfo {
    pub faces: Vec<HashMap<String, bool>>,
}

impl SpriteConfigInfo {
    /// `getFaceForSprite(sprite)` then `getTileInfoForSprite(sprite)`:
    /// the tile's `isMaster()`, `None` when no face has the sprite.
    pub fn tile_is_master(&self, sprite: &str) -> Option<bool> {
        self.faces.iter().find_map(|f| f.get(sprite).copied())
    }
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
            glass_removed_window_sprites: HashMap::new(),
            smashed_window_sprites: HashMap::new(),
            sprite_configs: HashMap::new(),
            sprite_config_scripts: HashMap::new(),
        }
    }

    /// `SpriteConfig.isMultiSquareMaster()` of a sprite config whose script
    /// is the config named `config_name`, once the component is on an
    /// `IsoObject` whose sprite is `sprite` (`initObjectInfo`): the object
    /// info, the face holding the sprite and its tile must exist, and the
    /// tile must be the face's master.
    pub fn is_multi_square_master(&self, config_name: &str, sprite: Option<&str>) -> bool {
        let (Some(info), Some(sprite)) = (self.sprite_configs.get(config_name), sprite) else {
            return false;
        };
        info.tile_is_master(sprite).unwrap_or(false)
    }

    fn new_sprite(&mut self) -> SpriteIdx {
        self.sprites.push(IsoSprite::new(&self.tables));
        self.sprites.len() - 1
    }

    /// Explicit tile-arena data projection, not native unmanaged allocation.
    /// Native CreateSprite uses runtime::create_runtime_sprite and keeps its
    /// actual allocation outside this arena and both manager maps.
    pub fn create_sprite_data_projection(&mut self) -> SpriteIdx {
        self.new_sprite()
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

    /// `IsoSprite.getSprite(manager, spr, offset)`: the sprite named like
    /// `spr` with `offset` added to the number after the last `_`. `Err` is
    /// the `NumberFormatException` of a non-numeric suffix.
    pub fn sprite_with_offset(
        &self,
        s: SpriteIdx,
        offset: i32,
    ) -> Result<Option<SpriteIdx>, String> {
        let name = self.sprites[s].name();
        if !name.contains('_') {
            return Ok(None);
        }
        let split = super::java_split(name, '_');
        let last = split
            .last()
            .ok_or_else(|| "ArrayIndexOutOfBoundsException".to_string())?;
        let last = crate::java::java_trim(last);
        let id = crate::java::parse_int(last)
            .ok_or_else(|| format!("NumberFormatException: For input string: \"{last}\""))?;
        let prefix = &name[..name.rfind('_').unwrap()];
        Ok(self
            .named_map
            .get(&format!("{prefix}_{}", id.wrapping_add(offset)))
            .copied())
    }

    /// `IsoSprite.getSprite(manager, id)` as an index (without
    /// `WorldConverter` tileset conversions).
    pub fn idx_by_id(&self, id: i32) -> Option<SpriteIdx> {
        self.int_map.get(&id).copied()
    }

    /// `namedMap` keys in `java.util.HashMap` iteration order, assuming the
    /// keys were inserted in sprite creation order.
    pub fn named_map_java_order(&self) -> Vec<(&str, SpriteIdx)> {
        let mut keys: Vec<(&str, SpriteIdx)> = self
            .named_map
            .iter()
            .map(|(k, &v)| (k.as_str(), v))
            .collect();
        keys.sort_by_key(|&(k, v)| (v, k));
        let names: Vec<&str> = keys.iter().map(|&(k, _)| k).collect();
        crate::java::hashmap_order(&names)
            .into_iter()
            .map(|k| (k, self.named_map[k]))
            .collect()
    }

    /// `IsoWorld.loadedTileDefinitions` (the sprite part): builds the
    /// smashed / glass-removed window maps, marks `fixtures_railings_01`
    /// sprites `NeverCutaway` and sets `HasLightOnSprite` where a
    /// `<tileset>_on_<index>` sprite with textures exists. `has_textures`
    /// is `!IsoSprite.hasNoTextures()`; a dedicated server has no textures.
    pub fn loaded_tile_definitions(
        &mut self,
        has_textures: &dyn Fn(&IsoSprite) -> bool,
    ) -> Result<(), String> {
        self.glass_removed_window_sprites.clear();
        self.smashed_window_sprites.clear();
        let order: Vec<SpriteIdx> = self
            .named_map_java_order()
            .into_iter()
            .map(|(_, s)| s)
            .collect();
        let f = self.tables.flag;
        for s in order {
            let props = &self.sprites[s].properties;
            if props.has(f.windowW) || props.has(f.windowN) {
                for (key, smashed) in [("GlassRemovedOffset", false), ("SmashedTileOffset", true)] {
                    let Some(val) = self.sprites[s].properties.get(&self.alias, key) else {
                        continue;
                    };
                    let offset = super::try_parse_int(val, 0);
                    if offset == 0 {
                        continue;
                    }
                    if let Some(s2) = self.sprite_with_offset(s, offset)? {
                        if smashed {
                            self.smashed_window_sprites.insert(s2, s);
                        } else {
                            self.glass_removed_window_sprites.insert(s2, s);
                        }
                    }
                }
            }
            if self.sprites[s].name().starts_with("fixtures_railings_01") {
                self.sprites[s].properties.set_flag(f.NeverCutaway);
            }
            let spr = &self.sprites[s];
            let on = format!(
                "{}_on_{}",
                spr.tileset_name.as_deref().unwrap_or("null"),
                spr.tile_sheet_index
            );
            let lit = self
                .named_map
                .get(&on)
                .is_some_and(|&s1| has_textures(&self.sprites[s1]));
            if lit {
                self.sprites[s].properties.set_flag(f.HasLightOnSprite);
            } else {
                self.sprites[s].properties.unset_flag(f.HasLightOnSprite);
            }
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sprite_duplicates() {
        let mut m = IsoSpriteManager::new(super::super::types::test_tables());
        assert_eq!(m.by_name("").map(|s| s.id), Some(-1));
        let a = m.add_sprite("a_0", 5);
        let b = m.add_sprite("a_0", 9);
        assert_ne!(a, b);
        assert_eq!(m.by_name("a_0").map(|s| s.id), Some(5));
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
