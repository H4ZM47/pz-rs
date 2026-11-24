//! `zombie.seating.SeatingManager` (`media/seating.txt` parser, merged
//! game+mod tileset registry, and Lua API used by `ISRestAction.lua`,
//! `ISGetOnBedAction.lua`, `ISWorldObjectContextMenu.lua`, and
//! `TileGeometryEditor_EditMode.lua`).

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use pz_fs::GameFs;
use pz_kahlua::{LuaObject, Thread, Value};

use crate::bind_misc_classes::{vector3f_value, Vector3f};
use crate::bind_world::square_coords;
use crate::bind_world_object::{object_from_value, object_square};
use crate::expose::{expose_class, int_arg, str_arg};

pub const SEATING_MANAGER_CLASS: &str = "zombie.seating.SeatingManager";
const COORD_MULT: f32 = 10000.0;
const DEFAULT_FACING: &str = "S";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SeatingPosition {
    pub id: String,
    pub translate: [f32; 3],
    pub properties: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SeatingTile {
    pub col: i32,
    pub row: i32,
    pub positions: Vec<SeatingPosition>,
    pub properties: HashMap<String, String>,
}

impl SeatingTile {
    pub fn position_by_id(&self, id: &str) -> Option<&SeatingPosition> {
        self.positions.iter().find(|p| p.id == id)
    }

    pub fn position_by_id_mut(&mut self, id: &str) -> Option<&mut SeatingPosition> {
        self.positions.iter_mut().find(|p| p.id == id)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SeatingTileset {
    pub name: String,
    pub tiles: HashMap<(i32, i32), SeatingTile>,
}

#[derive(Clone, Debug, Default)]
pub struct SeatingData {
    pub tilesets: HashMap<String, SeatingTileset>,
}

impl SeatingData {
    pub fn get_tile(&self, tileset: &str, col: i32, row: i32) -> Option<&SeatingTile> {
        self.tilesets.get(tileset)?.tiles.get(&(col, row))
    }

    pub fn get_or_create_tile(&mut self, tileset: &str, col: i32, row: i32) -> &mut SeatingTile {
        let ts = self
            .tilesets
            .entry(tileset.to_string())
            .or_insert_with(|| SeatingTileset {
                name: tileset.to_string(),
                tiles: HashMap::new(),
            });
        ts.tiles.entry((col, row)).or_insert_with(|| SeatingTile {
            col,
            row,
            positions: Vec::new(),
            properties: HashMap::new(),
        })
    }

    pub fn merge_from(&mut self, other: &SeatingData) {
        for (name, ts) in &other.tilesets {
            let dst = self
                .tilesets
                .entry(name.clone())
                .or_insert_with(|| SeatingTileset {
                    name: name.clone(),
                    tiles: HashMap::new(),
                });
            for (&xy, tile) in &ts.tiles {
                dst.tiles.insert(xy, tile.clone());
            }
        }
    }
}

/// Parses a Build 42.22 `media/seating.txt` file into [`SeatingData`].
pub fn parse_seating_txt(src: &str) -> SeatingData {
    let cleaned = strip_block_comments(src);
    let mut data = SeatingData::default();
    let mut version = 3i32;

    let mut cur_tileset: Option<String> = None;
    let mut cur_tile: Option<SeatingTile> = None;
    let mut cur_pos: Option<SeatingPosition> = None;
    let mut in_tile_props = false;
    let mut in_pos_props = false;
    let mut pending_block: Option<String> = None;
    let mut stack: Vec<String> = Vec::new();

    for raw_line in cleaned.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "{" {
            let kind = pending_block.take().unwrap_or_default();
            match kind.as_str() {
                "tile" => cur_tile = Some(SeatingTile::default()),
                "position" => cur_pos = Some(SeatingPosition::default()),
                "properties" => {
                    if cur_pos.is_some() {
                        in_pos_props = true;
                    } else if cur_tile.is_some() {
                        in_tile_props = true;
                    }
                }
                _ => {}
            }
            stack.push(kind);
            continue;
        }
        if line == "}" {
            if let Some(kind) = stack.pop() {
                match kind.as_str() {
                    "properties" => {
                        if in_pos_props {
                            in_pos_props = false;
                        } else {
                            in_tile_props = false;
                        }
                    }
                    "position" => {
                        if let (Some(tile), Some(pos)) = (cur_tile.as_mut(), cur_pos.take()) {
                            tile.positions.push(pos);
                        }
                    }
                    "tile" => {
                        if let (Some(ts_name), Some(tile)) = (cur_tileset.as_deref(), cur_tile.take())
                        {
                            let ts = data.tilesets.entry(ts_name.to_string()).or_insert_with(|| {
                                SeatingTileset {
                                    name: ts_name.to_string(),
                                    tiles: HashMap::new(),
                                }
                            });
                            ts.tiles.insert((tile.col, tile.row), tile);
                        }
                    }
                    "tileset" => {
                        cur_tileset = None;
                    }
                    _ => {}
                }
            }
            continue;
        }

        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = v.trim().trim_end_matches(',').trim();
            if in_pos_props {
                if let Some(pos) = cur_pos.as_mut() {
                    pos.properties.insert(key.to_string(), val.to_string());
                }
            } else if in_tile_props {
                if let Some(tile) = cur_tile.as_mut() {
                    tile.properties.insert(key.to_string(), val.to_string());
                }
            } else if let Some(pos) = cur_pos.as_mut() {
                match key {
                    "id" => pos.id = val.to_string(),
                    "translate" => {
                        let parts: Vec<f32> = val
                            .split_whitespace()
                            .filter_map(|p| parse_coord(p, version))
                            .collect();
                        if parts.len() >= 3 {
                            pos.translate = [parts[0], parts[1], parts[2]];
                        }
                    }
                    "translateX" => {
                        if let Some(x) = parse_coord(val, version) {
                            pos.translate[0] = x;
                        }
                    }
                    "translateY" => {
                        if let Some(y) = parse_coord(val, version) {
                            pos.translate[1] = y;
                        }
                    }
                    "translateZ" => {
                        if let Some(z) = parse_coord(val, version) {
                            pos.translate[2] = z;
                        }
                    }
                    _ => {}
                }
            } else if let Some(tile) = cur_tile.as_mut() {
                if key == "xy" {
                    let mut it = val.split_whitespace();
                    if let (Some(xs), Some(ys)) = (it.next(), it.next()) {
                        if let (Ok(c), Ok(r)) = (xs.parse::<i32>(), ys.parse::<i32>()) {
                            tile.col = c;
                            tile.row = r;
                        }
                    }
                }
            } else if stack.last().map(String::as_str) == Some("tileset") && key == "name" {
                cur_tileset = Some(val.to_string());
            } else if key == "VERSION" {
                if let Ok(v) = val.parse::<i32>() {
                    version = v;
                }
            }
        } else {
            pending_block = Some(line.trim_end_matches('{').trim().to_string());
            if line.ends_with('{') {
                let kind = pending_block.take().unwrap_or_default();
                match kind.as_str() {
                    "tile" => cur_tile = Some(SeatingTile::default()),
                    "position" => cur_pos = Some(SeatingPosition::default()),
                    "properties" => {
                        if cur_pos.is_some() {
                            in_pos_props = true;
                        } else if cur_tile.is_some() {
                            in_tile_props = true;
                        }
                    }
                    _ => {}
                }
                stack.push(kind);
            }
        }
    }

    data
}

fn parse_coord(s: &str, version: i32) -> Option<f32> {
    if version >= 2 {
        if let Ok(i) = s.parse::<i32>() {
            return Some(i as f32 / COORD_MULT);
        }
    }
    s.parse::<f32>().ok()
}

fn strip_block_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Splits a sprite name like `"furniture_seating_indoor_02_11"` into
/// `("furniture_seating_indoor_02", col, row)` using the 8-column tilesheet layout.
pub fn split_sprite_name(name: &str) -> Option<(&str, i32, i32)> {
    let idx = name.rfind('_')?;
    let tileset = &name[..idx];
    let tile_id: i32 = name[idx + 1..].parse().ok()?;
    if tile_id < 0 {
        return None;
    }
    Some((tileset, tile_id % 8, tile_id / 8))
}

#[derive(Clone, Debug, Default)]
pub struct SeatingManager {
    pub mod_order: Vec<String>,
    pub mod_data: HashMap<String, SeatingData>,
    pub merged: SeatingData,
}

impl SeatingManager {
    pub fn init_from_game_dir(&mut self, game_dir: &Path) {
        self.reset();
        let path = game_dir.join("media/seating.txt");
        if let Ok(src) = std::fs::read_to_string(&path) {
            let parsed = parse_seating_txt(&src);
            self.mod_order.push("game".to_string());
            self.mod_data.insert("game".to_string(), parsed);
        }
        self.merge_after_editing();
    }

    pub fn init_from_fs(&mut self, fs: &GameFs) {
        self.reset();
        let game_path = fs.base.join("media/seating.txt");
        if let Ok(src) = std::fs::read_to_string(&game_path) {
            let parsed = parse_seating_txt(&src);
            self.mod_order.push("game".to_string());
            self.mod_data.insert("game".to_string(), parsed);
        }
        for mod_id in fs.mod_ids() {
            if let Some(dir) = fs.mod_dir(mod_id) {
                let base = Path::new(dir);
                for candidate in [
                    base.join("common/media/seating.txt"),
                    base.join("media/seating.txt"),
                ] {
                    if let Ok(src) = std::fs::read_to_string(&candidate) {
                        let parsed = parse_seating_txt(&src);
                        self.mod_order.push(mod_id.clone());
                        self.mod_data.insert(mod_id.clone(), parsed);
                        break;
                    }
                }
            }
        }
        self.merge_after_editing();
    }

    pub fn load_mod_text(&mut self, mod_id: &str, src: &str) {
        if !self.mod_order.iter().any(|m| m == mod_id) {
            self.mod_order.push(mod_id.to_string());
        }
        self.mod_data
            .insert(mod_id.to_string(), parse_seating_txt(src));
        self.merge_after_editing();
    }

    pub fn merge_after_editing(&mut self) {
        self.merged = SeatingData::default();
        for mod_id in &self.mod_order {
            if let Some(data) = self.mod_data.get(mod_id) {
                self.merged.merge_from(data);
            }
        }
    }

    pub fn reset(&mut self) {
        self.mod_order.clear();
        self.mod_data.clear();
        self.merged = SeatingData::default();
    }

    fn data_for(&self, mod_id: Option<&str>) -> Option<&SeatingData> {
        match mod_id {
            Some(id) => self.mod_data.get(id),
            None => Some(&self.merged),
        }
    }

    fn data_for_mut(&mut self, mod_id: &str) -> &mut SeatingData {
        if !self.mod_order.iter().any(|m| m == mod_id) {
            self.mod_order.push(mod_id.to_string());
        }
        self.mod_data.entry(mod_id.to_string()).or_default()
    }

    pub fn get_tile_position_count(
        &self,
        mod_id: Option<&str>,
        tileset: &str,
        col: i32,
        row: i32,
    ) -> i32 {
        self.data_for(mod_id)
            .and_then(|d| d.get_tile(tileset, col, row))
            .map_or(0, |t| t.positions.len() as i32)
    }

    pub fn get_facing_direction(
        &self,
        mod_id: Option<&str>,
        tileset: &str,
        col: i32,
        row: i32,
    ) -> String {
        self.data_for(mod_id)
            .and_then(|d| d.get_tile(tileset, col, row))
            .and_then(|t| t.positions.first())
            .map(|p| p.id.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_FACING.to_string())
    }

    pub fn get_translation(
        &self,
        mod_id: Option<&str>,
        tileset: &str,
        tile_id: i32,
        facing: &str,
    ) -> Option<[f32; 3]> {
        if tile_id < 0 {
            return None;
        }
        let tile = self
            .data_for(mod_id)?
            .get_tile(tileset, tile_id % 8, tile_id / 8)?;
        tile.position_by_id(facing)
            .or_else(|| tile.positions.first())
            .map(|p| p.translate)
    }
}

thread_local! {
    static INSTANCE: RefCell<Rc<RefCell<SeatingManager>>> =
        RefCell::new(Rc::new(RefCell::new(SeatingManager::default())));
}

/// Shared [`SeatingManager`] singleton (`SeatingManager.getInstance()`).
pub fn shared_seating_manager() -> Rc<RefCell<SeatingManager>> {
    INSTANCE.with(|m| m.borrow().clone())
}

struct LuaSeatingManager;

impl LuaObject for LuaSeatingManager {
    fn class_name(&self) -> &str {
        SEATING_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn object_sprite_name(val: &Value) -> Option<String> {
    let wo = object_from_value(val)?;
    let body = wo.object.try_borrow().ok()?;
    if let Some(name) = &body.sprite_name {
        if let Ok(sprites) = crate::bind_sprites::manager() {
            if let Ok(sm) = sprites.try_borrow() {
                if let Ok(Some(resolved)) =
                    crate::bind_world_object::resolve_sprite_name(name, &sm)
                {
                    return Some(resolved);
                }
            }
        }
        if let pz_world::objects::SpriteName::Str(s) = name {
            return Some(s.clone());
        }
    }
    if let Ok(sprites) = crate::bind_sprites::manager() {
        if let Ok(sm) = sprites.try_borrow() {
            if let Some(idx) = body.sprite.tile_projection() {
                let n = sm.sprites[idx].name();
                if !n.is_empty() {
                    return Some(n.to_string());
                }
            }
        }
    }
    None
}

fn facing_side_offset(facing: &str, side: &str) -> (f32, f32) {
    // Base forward vector for the furniture facing direction (in ISO world XY).
    let (fx, fy) = match facing {
        "N" => (0.0f32, -1.0f32),
        "S" => (0.0, 1.0),
        "W" => (-1.0, 0.0),
        "E" => (1.0, 0.0),
        _ => (0.0, 1.0),
    };
    // Rotate relative to facing: Front = +forward, Left = +90° CCW, Right = -90° CW.
    let (dx, dy) = match side {
        "Left" => (fy, -fx),
        "Right" => (-fy, fx),
        _ => (fx, fy),
    };
    (dx * 0.7, dy * 0.7)
}

pub fn register(thread: &mut Thread, fs: Option<&Rc<RefCell<GameFs>>>) {
    if let Some(fs) = fs {
        if let Ok(fs_ref) = fs.try_borrow() {
            shared_seating_manager().borrow_mut().init_from_fs(&fs_ref);
        }
    }

    let b = expose_class(thread, SEATING_MANAGER_CLASS, None);
    b.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaSeatingManager))))
    });

    b.method("getModIDs", |c| {
        let mgr = shared_seating_manager();
        let ids: Vec<Value> = mgr
            .borrow()
            .mod_order
            .iter()
            .map(|s| Value::str(s))
            .collect();
        Ok(c.push(crate::bind_java_util::list_value(ids)))
    });

    b.method("mergeAfterEditing", |_c| {
        shared_seating_manager().borrow_mut().merge_after_editing();
        Ok(0)
    });

    b.method("write", |_c| Ok(0));

    b.method("Reset", |_c| {
        shared_seating_manager().borrow_mut().reset();
        Ok(0)
    });

    b.method("addTilePosition", |c| {
        let mod_id = str_arg(c, 1).map_or_else(|| "game".to_string(), |s| s.to_string());
        let tileset = str_arg(c, 2).map_or_else(String::new, |s| s.to_string());
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let pos_id = str_arg(c, 5).map_or_else(|| DEFAULT_FACING.to_string(), |s| s.to_string());
        let mgr = shared_seating_manager();
        let mut mgr = mgr.borrow_mut();
        let tile = mgr.data_for_mut(&mod_id).get_or_create_tile(&tileset, col, row);
        tile.positions.push(SeatingPosition {
            id: pos_id,
            translate: [0.0; 3],
            properties: HashMap::new(),
        });
        let idx = (tile.positions.len() - 1) as i32;
        Ok(c.push(idx))
    });

    b.method("removeTilePosition", |c| {
        let mod_id_s = str_arg(c, 1);
        let tileset_s = str_arg(c, 2);
        let mod_id = mod_id_s.as_deref().unwrap_or("game");
        let tileset = tileset_s.as_deref().unwrap_or("");
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let index = int_arg(c, 5);
        if index >= 0 {
            let mgr = shared_seating_manager();
            let mut mgr = mgr.borrow_mut();
            if let Some(ts) = mgr
                .mod_data
                .get_mut(mod_id)
                .and_then(|d| d.tilesets.get_mut(tileset))
            {
                if let Some(tile) = ts.tiles.get_mut(&(col, row)) {
                    if (index as usize) < tile.positions.len() {
                        tile.positions.remove(index as usize);
                    }
                }
            }
        }
        Ok(0)
    });

    // getTilePositionCount(IsoObject) | (tileset, col, row) | (modID, tileset, col, row)
    b.method("getTilePositionCount", |c| {
        let mgr = shared_seating_manager();
        let mgr = mgr.borrow();
        let count = match c.nargs() {
            2 => {
                let obj = c.arg(1);
                object_sprite_name(&obj)
                    .and_then(|n| {
                        let (ts, col, row) = split_sprite_name(&n)?;
                        Some(mgr.get_tile_position_count(None, ts, col, row))
                    })
                    .unwrap_or(0)
            }
            4 => {
                let ts_s = str_arg(c, 1);
                let ts = ts_s.as_deref().unwrap_or("");
                mgr.get_tile_position_count(None, ts, int_arg(c, 2), int_arg(c, 3))
            }
            n if n >= 5 => {
                let mod_id = str_arg(c, 1);
                let ts_s = str_arg(c, 2);
                let ts = ts_s.as_deref().unwrap_or("");
                mgr.get_tile_position_count(mod_id.as_deref(), ts, int_arg(c, 3), int_arg(c, 4))
            }
            _ => 0,
        };
        Ok(c.push(count))
    });

    b.method("getTilePositionID", |c| {
        let mod_id = str_arg(c, 1);
        let ts_s = str_arg(c, 2);
        let ts = ts_s.as_deref().unwrap_or("");
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let index = int_arg(c, 5);
        let mgr = shared_seating_manager();
        let id = mgr
            .borrow()
            .data_for(mod_id.as_deref())
            .and_then(|d| d.get_tile(ts, col, row))
            .and_then(|t| usize::try_from(index).ok().and_then(|i| t.positions.get(i)))
            .map(|p| p.id.clone());
        Ok(c.push(id.map_or(Value::Nil, |s| Value::str(&s))))
    });

    b.method("hasTilePositionWithID", |c| {
        let mod_id = str_arg(c, 1);
        let ts_s = str_arg(c, 2);
        let ts = ts_s.as_deref().unwrap_or("");
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let id_s = str_arg(c, 5);
        let id = id_s.as_deref().unwrap_or("");
        let mgr = shared_seating_manager();
        let has = mgr
            .borrow()
            .data_for(mod_id.as_deref())
            .and_then(|d| d.get_tile(ts, col, row))
            .is_some_and(|t| t.position_by_id(id).is_some());
        Ok(c.push(has))
    });

    b.method("getTilePositionTranslate", |c| {
        let mod_id = str_arg(c, 1);
        let ts_s = str_arg(c, 2);
        let ts = ts_s.as_deref().unwrap_or("");
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let index = int_arg(c, 5);
        let mgr = shared_seating_manager();
        let v = mgr
            .borrow()
            .data_for(mod_id.as_deref())
            .and_then(|d| d.get_tile(ts, col, row))
            .and_then(|t| usize::try_from(index).ok().and_then(|i| t.positions.get(i)))
            .map_or([0.0; 3], |p| p.translate);
        Ok(c.push(vector3f_value(v)))
    });

    b.method("getTilePositionProperty", |c| {
        let mod_id = str_arg(c, 1);
        let ts_s = str_arg(c, 2);
        let ts = ts_s.as_deref().unwrap_or("");
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let index = int_arg(c, 5);
        let key_s = str_arg(c, 6);
        let key = key_s.as_deref().unwrap_or("");
        let mgr = shared_seating_manager();
        let val = mgr
            .borrow()
            .data_for(mod_id.as_deref())
            .and_then(|d| d.get_tile(ts, col, row))
            .and_then(|t| usize::try_from(index).ok().and_then(|i| t.positions.get(i)))
            .and_then(|p| p.properties.get(key))
            .cloned();
        Ok(c.push(val.map_or(Value::Nil, |s| Value::str(&s))))
    });

    b.method("setTilePositionProperty", |c| {
        let mod_id = str_arg(c, 1).map_or_else(|| "game".to_string(), |s| s.to_string());
        let ts = str_arg(c, 2).map_or_else(String::new, |s| s.to_string());
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let index = int_arg(c, 5);
        let key = str_arg(c, 6).map_or_else(String::new, |s| s.to_string());
        let val = str_arg(c, 7).map(|s| s.to_string());
        if let Ok(i) = usize::try_from(index) {
            let mgr = shared_seating_manager();
            let mut mgr = mgr.borrow_mut();
            let tile = mgr.data_for_mut(&mod_id).get_or_create_tile(&ts, col, row);
            if let Some(pos) = tile.positions.get_mut(i) {
                match val {
                    Some(v) if !v.is_empty() => {
                        pos.properties.insert(key, v);
                    }
                    _ => {
                        pos.properties.remove(&key);
                    }
                }
            }
        }
        Ok(0)
    });

    b.method("getTileProperty", |c| {
        let mgr = shared_seating_manager();
        let mgr = mgr.borrow();
        let val = if c.nargs() >= 6 {
            let mod_id = str_arg(c, 1);
            let ts_s = str_arg(c, 2);
            let key_s = str_arg(c, 5);
            let ts = ts_s.as_deref().unwrap_or("");
            let key = key_s.as_deref().unwrap_or("");
            mgr.data_for(mod_id.as_deref())
                .and_then(|d| d.get_tile(ts, int_arg(c, 3), int_arg(c, 4)))
                .and_then(|t| t.properties.get(key))
                .cloned()
        } else {
            let ts_s = str_arg(c, 1);
            let key_s = str_arg(c, 4);
            let ts = ts_s.as_deref().unwrap_or("");
            let key = key_s.as_deref().unwrap_or("");
            mgr.data_for(None)
                .and_then(|d| d.get_tile(ts, int_arg(c, 2), int_arg(c, 3)))
                .and_then(|t| t.properties.get(key))
                .cloned()
        };
        Ok(c.push(val.map_or(Value::Nil, |s| Value::str(&s))))
    });

    b.method("setTileProperty", |c| {
        let mod_id = str_arg(c, 1).map_or_else(|| "game".to_string(), |s| s.to_string());
        let ts = str_arg(c, 2).map_or_else(String::new, |s| s.to_string());
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let key = str_arg(c, 5).map_or_else(String::new, |s| s.to_string());
        let val = str_arg(c, 6).map(|s| s.to_string());
        let mgr = shared_seating_manager();
        let mut mgr = mgr.borrow_mut();
        let tile = mgr.data_for_mut(&mod_id).get_or_create_tile(&ts, col, row);
        match val {
            Some(v) if !v.is_empty() => {
                tile.properties.insert(key, v);
            }
            _ => {
                tile.properties.remove(&key);
            }
        }
        Ok(0)
    });

    // getFacingDirection(IsoObject | IsoSprite) | (tileset, col, row) | (modID, tileset, col, row)
    b.method("getFacingDirection", |c| {
        let mgr = shared_seating_manager();
        let mgr = mgr.borrow();
        let facing = match c.nargs() {
            2 => {
                let obj = c.arg(1);
                object_sprite_name(&obj)
                    .and_then(|n| {
                        let (ts, col, row) = split_sprite_name(&n)?;
                        Some(mgr.get_facing_direction(None, ts, col, row))
                    })
                    .unwrap_or_else(|| DEFAULT_FACING.to_string())
            }
            4 => {
                let ts_s = str_arg(c, 1);
                let ts = ts_s.as_deref().unwrap_or("");
                mgr.get_facing_direction(None, ts, int_arg(c, 2), int_arg(c, 3))
            }
            n if n >= 5 => {
                let mod_id = str_arg(c, 1);
                let ts_s = str_arg(c, 2);
                let ts = ts_s.as_deref().unwrap_or("");
                mgr.get_facing_direction(mod_id.as_deref(), ts, int_arg(c, 3), int_arg(c, 4))
            }
            _ => DEFAULT_FACING.to_string(),
        };
        Ok(c.push(Value::str(&facing)))
    });

    // getTranslation(tileset, tileId, facing, outVec3) | (modID, tileset, tileId, facing, outVec3)
    b.method("getTranslation", |c| {
        let mgr = shared_seating_manager();
        let mgr = mgr.borrow();
        let (tr, out_idx) = if c.nargs() >= 6 {
            let mod_id = str_arg(c, 1);
            let ts_s = str_arg(c, 2);
            let ts = ts_s.as_deref().unwrap_or("");
            let tile_id = int_arg(c, 3);
            let facing_s = str_arg(c, 4);
            let facing = facing_s.as_deref().unwrap_or(DEFAULT_FACING);
            (mgr.get_translation(mod_id.as_deref(), ts, tile_id, facing), 5)
        } else {
            let ts_s = str_arg(c, 1);
            let ts = ts_s.as_deref().unwrap_or("");
            let tile_id = int_arg(c, 2);
            let facing_s = str_arg(c, 3);
            let facing = facing_s.as_deref().unwrap_or(DEFAULT_FACING);
            (mgr.get_translation(None, ts, tile_id, facing), 4)
        };
        let v = tr.unwrap_or([0.0; 3]);
        let out_val = c.arg(out_idx);
        if let Value::Object(o) = &out_val {
            if let Some(vec3) = o.as_any().downcast_ref::<Vector3f>() {
                vec3.set(v);
                return Ok(c.push(out_val));
            }
        }
        Ok(c.push(vector3f_value(v)))
    });

    // getAdjacentPosition(character, object, sitDirection, side, animState, animNode, worldPos)
    b.method("getAdjacentPosition", |c| {
        let obj_val = c.arg(2);
        let sit_dir_s = str_arg(c, 3);
        let side_s = str_arg(c, 4);
        let sit_dir = sit_dir_s.as_deref().unwrap_or(DEFAULT_FACING);
        let side = side_s.as_deref().unwrap_or("Front");
        let out_val = c.arg(7);

        let Some(wo) = object_from_value(&obj_val) else {
            return Ok(c.push(false));
        };
        let sq_val = object_square(wo)?;
        let Some((sx, sy, sz)) = square_coords(&sq_val) else {
            return Ok(c.push(false));
        };

        // Check if this side is explicitly blocked in `seating.txt` properties.
        if let Some(name) = object_sprite_name(&obj_val) {
            if let Some((ts, col, row)) = split_sprite_name(&name) {
                let mgr = shared_seating_manager();
                let mgr = mgr.borrow();
                if let Some(tile) = mgr.merged.get_tile(ts, col, row) {
                    if let Some(pos) = tile.position_by_id(sit_dir).or_else(|| tile.positions.first())
                    {
                        let block_key = match side {
                            "Left" => Some("BlockLeft"),
                            "Right" => Some("BlockRight"),
                            _ => None,
                        };
                        if let Some(bk) = block_key {
                            if pos
                                .properties
                                .get(bk)
                                .is_some_and(|v| v.eq_ignore_ascii_case("true"))
                            {
                                return Ok(c.push(false));
                            }
                        }
                    }
                }
            }
        }

        let (dx, dy) = facing_side_offset(sit_dir, side);
        let wp = [sx as f32 + 0.5 + dx, sy as f32 + 0.5 + dy, sz as f32];
        if let Value::Object(o) = &out_val {
            if let Some(vec3) = o.as_any().downcast_ref::<Vector3f>() {
                vec3.set(wp);
            }
        }
        Ok(c.push(true))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::{self, load_string};
    use pz_kahlua::TableRef;
    use pz_world::objects::{IsoObject, ObjectKind, SpriteName};
    use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoSpriteManager};

    fn run(t: &mut Thread, code: &str) -> Vec<Value> {
        let f = load_string(code, Some("bind_seating_test"), t.globals().clone()).unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|(e, trace)| panic!("{e:?}\n{trace}"))
    }

    #[test]
    fn seating_manager_parses_v3_and_serves_lua_queries() {
        let prev_obj =
            pz_world::object_slot::install_object_observer(Some(crate::component_gc::observe_object));
        shared_seating_manager().borrow_mut().reset();
        let sample = r#"
        seating
        {
            VERSION = 3,
            tileset
            {
                name = furniture_seating_indoor_02,
                /* furniture_seating_indoor_02_1 */
                tile
                {
                    xy = 1 0,
                    position
                    {
                        id = E,
                        translate = 5000 10000 2500,
                        properties
                        {
                            BlockLeft = true,
                        }
                    }
                }
            }
        }
        "#;
        shared_seating_manager()
            .borrow_mut()
            .load_mod_text("game", sample);

        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_globals::register(&mut t);
        crate::bind_misc_classes::register(&mut t, None);
        crate::bind_world::register(&mut t);
        register(&mut t, None);

        let sprites = Rc::new(RefCell::new(IsoSpriteManager::new(
            pz_world::sprite::test_tables(),
        )));
        crate::bind_sprites::install_sprite_manager(Some(sprites));

        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(1, 2);
        let mut sq = IsoGridSquare::new(10, 20, 0);
        let mut chair = IsoObject::new(1, ObjectKind::Object, None);
        chair.sprite_name = Some(SpriteName::Str("furniture_seating_indoor_02_1".into()));
        sq.objects.push(chair.into());
        chunk.set_square(2, 4, 0, Some(sq.into()));
        cell.chunks.insert((1, 2), chunk);
        crate::bind_world::set_cell(Some(cell));

        let out = run(
            &mut t,
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            local sm = SeatingManager.getInstance()
            local sq = getCell():getGridSquare(10, 20, 0)
            local chair = sq:getObjects():get(0)

            assert(sm:getTilePositionCount(chair) == 1)
            assert(sm:getFacingDirection(chair) == "E")
            assert(sm:hasTilePositionWithID("game", "furniture_seating_indoor_02", 1, 0, "E"))
            assert(sm:getTilePositionID("game", "furniture_seating_indoor_02", 1, 0, 0) == "E")
            assert(sm:getTilePositionProperty("game", "furniture_seating_indoor_02", 1, 0, 0, "BlockLeft") == "true")

            local tr = sm:getTilePositionTranslate("game", "furniture_seating_indoor_02", 1, 0, 0)
            assert(math.abs(tr:x() - 0.5) < 0.001)
            assert(math.abs(tr:y() - 1.0) < 0.001)
            assert(math.abs(tr:z() - 0.25) < 0.001)

            local wp = Vector3f.new()
            assert(not sm:getAdjacentPosition(nil, chair, "E", "Left", "sitonfurniture", "SitOnFurnitureLeft", wp))
            assert(sm:getAdjacentPosition(nil, chair, "E", "Front", "sitonfurniture", "SitOnFurnitureFront", wp))
            assert(wp:x() > 11.0 and math.abs(wp:y() - 20.5) < 0.01)

            -- Edit & merge
            local idx = sm:addTilePosition("game", "furniture_seating_indoor_02", 1, 0, "S")
            assert(idx == 1)
            sm:setTilePositionProperty("game", "furniture_seating_indoor_02", 1, 0, idx, "Custom", "yes")
            assert(sm:getTilePositionProperty("game", "furniture_seating_indoor_02", 1, 0, idx, "Custom") == "yes")
            sm:mergeAfterEditing()
            assert(sm:getTilePositionCount(chair) == 2)

            return sm:getFacingDirection(chair), sm:getTilePositionCount(chair)
            "#,
        );
        assert_eq!(out[0].as_str(), Some("E"));
        assert_eq!(out[1].as_num(), Some(2.0));

        crate::bind_world::set_cell(None);
        crate::bind_sprites::install_sprite_manager(None);
        shared_seating_manager().borrow_mut().reset();
        pz_world::object_slot::install_object_observer(prev_obj);
    }
}
