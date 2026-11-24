//! Build 42.22 tile-depth & seam editor/query Lua bindings:
//! - `zombie.tileDepth.TileGeometryManager` (`TileGeometryEditor_*.lua`)
//! - `zombie.tileDepth.TileDepthTextureManager` (`TileGeometryEditor_*.lua`)
//! - `zombie.seams.SeamManager` (`SeamEditorUI*.lua`, `media/seams.txt`)

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_fs::GameFs;
use pz_kahlua::{LuaObject, Thread, Value};
use pz_render::tile_depth::geometry::TileGeometryProperties;
use pz_scripts::parser as script_parser;

use crate::expose::{bool_arg, expose_class, int_arg, str_arg};

pub const TILE_GEOMETRY_MANAGER_CLASS: &str = "zombie.tileDepth.TileGeometryManager";
pub const TILE_DEPTH_TEXTURE_MANAGER_CLASS: &str = "zombie.tileDepth.TileDepthTextureManager";
pub const TILE_DEPTH_TEXTURE_CLASS: &str = "zombie.tileDepth.TileDepthTexture";
pub const SEAM_MANAGER_CLASS: &str = "zombie.seams.SeamManager";

// ---------------------------------------------------------------------------
// TileGeometryManager
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TileGeometryRegistry {
    mod_order: Vec<String>,
    mod_files: HashMap<String, TileGeometryProperties>,
}

impl TileGeometryRegistry {
    fn reset(&mut self) {
        self.mod_order.clear();
        self.mod_files.clear();
    }

    fn init_from_fs(&mut self, fs: &GameFs) {
        self.reset();
        let media = fs.media_file("");
        if let Some(props) = TileGeometryProperties::read(&media) {
            self.mod_order.push("game".to_string());
            self.mod_files.insert("game".to_string(), props);
        }
    }

    fn load_mod_text(&mut self, mod_id: &str, text: &str) {
        let props = TileGeometryProperties::parse(text);
        if !self.mod_order.iter().any(|m| m == mod_id) {
            self.mod_order.push(mod_id.to_string());
        }
        self.mod_files.insert(mod_id.to_string(), props);
    }

    fn sprite_key(tileset: &str, col: i32, row: i32) -> String {
        format!("{tileset}_{}", col + row * 8)
    }

    fn get_tile_property(
        &self,
        mod_id: &str,
        tileset: &str,
        col: i32,
        row: i32,
        key: &str,
    ) -> Option<String> {
        let sprite = Self::sprite_key(tileset, col, row);
        self.mod_files
            .get(mod_id)?
            .tiles
            .get(&sprite)?
            .get(key)
            .cloned()
    }

    fn set_tile_property(
        &mut self,
        mod_id: &str,
        tileset: &str,
        col: i32,
        row: i32,
        key: &str,
        val: Option<&str>,
    ) {
        if !self.mod_order.iter().any(|m| m == mod_id) {
            self.mod_order.push(mod_id.to_string());
        }
        let sprite = Self::sprite_key(tileset, col, row);
        let props = self.mod_files.entry(mod_id.to_string()).or_default();
        match val {
            Some(v) if !v.is_empty() => {
                props
                    .tiles
                    .entry(sprite)
                    .or_default()
                    .insert(key.to_string(), v.to_string());
            }
            _ => {
                if let Some(map) = props.tiles.get_mut(&sprite) {
                    map.remove(key);
                }
            }
        }
    }
}

thread_local! {
    static TILE_GEOMETRY_MGR: RefCell<TileGeometryRegistry> =
        RefCell::new(TileGeometryRegistry::default());
}

struct LuaTileGeometryManager;

impl LuaObject for LuaTileGeometryManager {
    fn class_name(&self) -> &str {
        TILE_GEOMETRY_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// TileDepthTextureManager
// ---------------------------------------------------------------------------

struct LuaTileDepthTextureManager;

impl LuaObject for LuaTileDepthTextureManager {
    fn class_name(&self) -> &str {
        TILE_DEPTH_TEXTURE_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaTileDepthTexture {
    name: String,
    index: i32,
    width: i32,
    height: i32,
}

impl LuaObject for LuaTileDepthTexture {
    fn class_name(&self) -> &str {
        TILE_DEPTH_TEXTURE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// SeamManager (`media/seams.txt`)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeamTile {
    pub col: i32,
    pub row: i32,
    pub east: Vec<String>,
    pub south: Vec<String>,
    pub below_east: Vec<String>,
    pub below_south: Vec<String>,
    pub properties: HashMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct SeamTileset {
    pub tiles: HashMap<(i32, i32), SeamTile>,
}

#[derive(Clone, Debug, Default)]
pub struct SeamModData {
    pub tilesets: HashMap<String, SeamTileset>,
}

impl SeamModData {
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let root = script_parser::parse(&script_parser::strip_comments(text));
        let Some(file) = root.children.first() else {
            return out;
        };
        for ts in file.children.iter().filter(|b| b.type_str() == "tileset") {
            let Some(name) = ts
                .get_value("name")
                .map(|v| v.get_value().trim().to_string())
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let entry = out.tilesets.entry(name).or_default();
            for tile_blk in ts.children.iter().filter(|b| b.type_str() == "tile") {
                let Some((col, row)) = tile_blk.get_value("xy").and_then(|v| {
                    let (c, r) = v.get_value().trim().split_once('x')?;
                    Some((c.trim().parse::<i32>().ok()?, r.trim().parse::<i32>().ok()?))
                }) else {
                    continue;
                };
                let mut tile = SeamTile {
                    col,
                    row,
                    ..SeamTile::default()
                };
                for child in &tile_blk.children {
                    match child.type_str() {
                        "east" => {
                            for v in &child.values {
                                let k = v.get_key().trim();
                                if !k.is_empty() {
                                    tile.east.push(k.to_string());
                                }
                            }
                        }
                        "south" => {
                            for v in &child.values {
                                let k = v.get_key().trim();
                                if !k.is_empty() {
                                    tile.south.push(k.to_string());
                                }
                            }
                        }
                        "belowEast" => {
                            for v in &child.values {
                                let k = v.get_key().trim();
                                if !k.is_empty() {
                                    tile.below_east.push(k.to_string());
                                }
                            }
                        }
                        "belowSouth" => {
                            for v in &child.values {
                                let k = v.get_key().trim();
                                if !k.is_empty() {
                                    tile.below_south.push(k.to_string());
                                }
                            }
                        }
                        "properties" => {
                            for v in &child.values {
                                let k = v.get_key().trim();
                                if !k.is_empty() {
                                    tile.properties
                                        .insert(k.to_string(), v.get_value().trim().to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                entry.tiles.insert((col, row), tile);
            }
        }
        out
    }

    pub fn get_tile(&self, tileset: &str, col: i32, row: i32) -> Option<&SeamTile> {
        self.tilesets.get(tileset)?.tiles.get(&(col, row))
    }

    pub fn get_or_create_tile(&mut self, tileset: &str, col: i32, row: i32) -> &mut SeamTile {
        self.tilesets
            .entry(tileset.to_string())
            .or_default()
            .tiles
            .entry((col, row))
            .or_insert_with(|| SeamTile {
                col,
                row,
                ..SeamTile::default()
            })
    }
}

#[derive(Default)]
pub struct SeamRegistry {
    mod_order: Vec<String>,
    mod_data: HashMap<String, SeamModData>,
}

impl SeamRegistry {
    pub fn reset(&mut self) {
        self.mod_order.clear();
        self.mod_data.clear();
    }

    pub fn init_from_fs(&mut self, fs: &GameFs) {
        self.reset();
        let path = fs.media_file("seams.txt");
        if let Ok(text) = std::fs::read_to_string(path) {
            self.load_mod_text("game", &text);
        }
    }

    pub fn load_mod_text(&mut self, mod_id: &str, text: &str) {
        let data = SeamModData::parse(text);
        if !self.mod_order.iter().any(|m| m == mod_id) {
            self.mod_order.push(mod_id.to_string());
        }
        self.mod_data.insert(mod_id.to_string(), data);
    }

    fn data_mut(&mut self, mod_id: &str) -> &mut SeamModData {
        if !self.mod_order.iter().any(|m| m == mod_id) {
            self.mod_order.push(mod_id.to_string());
        }
        self.mod_data.entry(mod_id.to_string()).or_default()
    }

    fn tile_join(
        &mut self,
        mod_id: &str,
        tileset: &str,
        col: i32,
        row: i32,
        create: bool,
        pick: fn(&SeamTile) -> &Vec<String>,
    ) -> Option<Vec<String>> {
        if create {
            let tile = self.data_mut(mod_id).get_or_create_tile(tileset, col, row);
            return Some(pick(tile).clone());
        }
        self.mod_data
            .get(mod_id)?
            .get_tile(tileset, col, row)
            .map(|t| pick(t).clone())
    }

    fn master_tile_name(&self, mod_id: &str, tileset: &str, col: i32, row: i32) -> Option<String> {
        self.mod_data
            .get(mod_id)?
            .get_tile(tileset, col, row)?
            .properties
            .get("master")
            .cloned()
    }

    fn is_master_tile(&self, mod_id: &str, tileset: &str, col: i32, row: i32) -> bool {
        let target = format!("{tileset}_{}", col + row * 8);
        let Some(data) = self.mod_data.get(mod_id) else {
            return false;
        };
        data.tilesets.values().any(|ts| {
            ts.tiles
                .values()
                .any(|t| t.properties.get("master").is_some_and(|m| m == &target))
        })
    }
}

thread_local! {
    static SEAM_MGR: RefCell<SeamRegistry> = RefCell::new(SeamRegistry::default());
}

struct LuaSeamManager;

impl LuaObject for LuaSeamManager {
    fn class_name(&self) -> &str {
        SEAM_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn register_tile_geometry_manager(thread: &mut Thread) {
    let b = expose_class(thread, TILE_GEOMETRY_MANAGER_CLASS, None);
    b.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaTileGeometryManager))))
    });
    b.method("getModIDs", |c| {
        let ids: Vec<Value> = TILE_GEOMETRY_MGR.with(|m| {
            m.borrow()
                .mod_order
                .iter()
                .map(|s| Value::str(s))
                .collect()
        });
        Ok(c.push(crate::bind_java_util::list_value(ids)))
    });
    b.method("getTileProperty", |c| {
        let mod_s = str_arg(c, 1);
        let ts_s = str_arg(c, 2);
        let key_s = str_arg(c, 5);
        let val = TILE_GEOMETRY_MGR.with(|m| {
            m.borrow().get_tile_property(
                mod_s.as_deref().unwrap_or("game"),
                ts_s.as_deref().unwrap_or(""),
                int_arg(c, 3),
                int_arg(c, 4),
                key_s.as_deref().unwrap_or(""),
            )
        });
        Ok(c.push(val.map_or(Value::Nil, |s| Value::str(&s))))
    });
    b.method("setTileProperty", |c| {
        let mod_s = str_arg(c, 1);
        let ts_s = str_arg(c, 2);
        let key_s = str_arg(c, 5);
        let val_s = str_arg(c, 6);
        TILE_GEOMETRY_MGR.with(|m| {
            m.borrow_mut().set_tile_property(
                mod_s.as_deref().unwrap_or("game"),
                ts_s.as_deref().unwrap_or(""),
                int_arg(c, 3),
                int_arg(c, 4),
                key_s.as_deref().unwrap_or(""),
                val_s.as_deref(),
            );
        });
        Ok(0)
    });
    b.method("initSpriteProperties", |_c| Ok(0));
    b.method("write", |_c| Ok(0));
    b.method("Reset", |_c| {
        TILE_GEOMETRY_MGR.with(|m| m.borrow_mut().reset());
        Ok(0)
    });
}

fn register_tile_depth_texture_manager(thread: &mut Thread) {
    let tex = expose_class(thread, TILE_DEPTH_TEXTURE_CLASS, None);
    tex.method("getName", |c| {
        let s = crate::expose::with_this::<LuaTileDepthTexture, _>(c, |t| t.name.clone())?;
        Ok(c.push(Value::str(&s)))
    });
    tex.method("getIndex", |c| {
        let idx = crate::expose::with_this::<LuaTileDepthTexture, _>(c, |t| t.index)?;
        Ok(c.push(idx))
    });
    tex.method("getWidth", |c| {
        let w = crate::expose::with_this::<LuaTileDepthTexture, _>(c, |t| t.width)?;
        Ok(c.push(w))
    });
    tex.method("getHeight", |c| {
        let h = crate::expose::with_this::<LuaTileDepthTexture, _>(c, |t| t.height)?;
        Ok(c.push(h))
    });
    tex.method("isEmpty", |c| Ok(c.push(false)));

    let b = expose_class(thread, TILE_DEPTH_TEXTURE_MANAGER_CLASS, None);
    b.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaTileDepthTextureManager))))
    });
    b.method("getTexture", |c| {
        let (ts, idx) = if c.nargs() >= 4 {
            let ts_s = str_arg(c, 2);
            (ts_s.as_deref().unwrap_or("").to_string(), int_arg(c, 3))
        } else {
            let ts_s = str_arg(c, 1);
            (ts_s.as_deref().unwrap_or("").to_string(), int_arg(c, 2))
        };
        if ts.is_empty() || idx < 0 {
            return Ok(c.push(Value::Nil));
        }
        let tex = Rc::new(LuaTileDepthTexture {
            name: format!("{ts}_{idx}"),
            index: idx,
            width: 128,
            height: 256,
        });
        Ok(c.push(Value::Object(tex)))
    });
    b.method("getTextureFromTileName", |c| {
        let name_s = if c.nargs() >= 3 {
            str_arg(c, 2)
        } else {
            str_arg(c, 1)
        };
        let Some(name) = name_s.filter(|s| !s.is_empty()) else {
            return Ok(c.push(Value::Nil));
        };
        let idx = name
            .rsplit_once('_')
            .and_then(|(_, s)| s.parse::<i32>().ok())
            .unwrap_or(0);
        let tex = Rc::new(LuaTileDepthTexture {
            name: name.to_string(),
            index: idx,
            width: 128,
            height: 256,
        });
        Ok(c.push(Value::Object(tex)))
    });
    b.method("getDefaultDepthTexture", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaTileDepthTexture {
            name: "default".to_string(),
            index: 0,
            width: 128,
            height: 256,
        }))))
    });
    b.method("getBillboardDepthTexture", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaTileDepthTexture {
            name: "billboard".to_string(),
            index: 0,
            width: 128,
            height: 256,
        }))))
    });
    b.method("isLoadingFinished", |c| Ok(c.push(true)));
    for noop in [
        "mergeAfterEditing",
        "reloadTileset",
        "loadTilesetPixelsIfNeeded",
        "saveTileset",
        "initSprites",
        "Reset",
    ] {
        b.method(noop, |_c| Ok(0));
    }
}

fn register_seam_manager(thread: &mut Thread) {
    let b = expose_class(thread, SEAM_MANAGER_CLASS, None);
    b.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaSeamManager))))
    });
    b.method("getModIDs", |c| {
        let ids: Vec<Value> = SEAM_MGR.with(|m| {
            m.borrow()
                .mod_order
                .iter()
                .map(|s| Value::str(s))
                .collect()
        });
        Ok(c.push(crate::bind_java_util::list_value(ids)))
    });
    b.method("getTileProperty", |c| {
        let mod_s = str_arg(c, 1);
        let ts_s = str_arg(c, 2);
        let key_s = str_arg(c, 5);
        let val = SEAM_MGR.with(|m| {
            m.borrow()
                .mod_data
                .get(mod_s.as_deref().unwrap_or("game"))?
                .get_tile(ts_s.as_deref().unwrap_or(""), int_arg(c, 3), int_arg(c, 4))?
                .properties
                .get(key_s.as_deref().unwrap_or(""))
                .cloned()
        });
        Ok(c.push(val.map_or(Value::Nil, |s| Value::str(&s))))
    });
    b.method("setTileProperty", |c| {
        let mod_s = str_arg(c, 1);
        let ts_s = str_arg(c, 2);
        let col = int_arg(c, 3);
        let row = int_arg(c, 4);
        let key_s = str_arg(c, 5);
        let val_s = str_arg(c, 6);
        SEAM_MGR.with(|m| {
            let mut mgr = m.borrow_mut();
            let tile = mgr
                .data_mut(mod_s.as_deref().unwrap_or("game"))
                .get_or_create_tile(ts_s.as_deref().unwrap_or(""), col, row);
            let key = key_s.as_deref().unwrap_or("");
            match val_s.as_deref() {
                Some(v) if !v.is_empty() => {
                    tile.properties.insert(key.to_string(), v.to_string());
                }
                _ => {
                    tile.properties.remove(key);
                }
            }
        });
        Ok(0)
    });
    for (method, picker) in [
        ("getTileJoinE", (|t: &SeamTile| &t.east) as fn(&SeamTile) -> &Vec<String>),
        ("getTileJoinS", (|t: &SeamTile| &t.south) as fn(&SeamTile) -> &Vec<String>),
        (
            "getTileJoinBelowE",
            (|t: &SeamTile| &t.below_east) as fn(&SeamTile) -> &Vec<String>,
        ),
        (
            "getTileJoinBelowS",
            (|t: &SeamTile| &t.below_south) as fn(&SeamTile) -> &Vec<String>,
        ),
    ] {
        b.method(method, move |c| {
            let mod_s = str_arg(c, 1);
            let ts_s = str_arg(c, 2);
            let col = int_arg(c, 3);
            let row = int_arg(c, 4);
            let create = bool_arg(c, 5);
            let res = SEAM_MGR.with(|m| {
                m.borrow_mut().tile_join(
                    mod_s.as_deref().unwrap_or("game"),
                    ts_s.as_deref().unwrap_or(""),
                    col,
                    row,
                    create,
                    picker,
                )
            });
            match res {
                Some(list) => {
                    let vals: Vec<Value> = list.iter().map(|s| Value::str(s)).collect();
                    Ok(c.push(crate::bind_java_util::list_value(vals)))
                }
                None => Ok(c.push(Value::Nil)),
            }
        });
    }
    b.method("getMasterTileName", |c| {
        let mod_s = str_arg(c, 1);
        let ts_s = str_arg(c, 2);
        let res = SEAM_MGR.with(|m| {
            m.borrow().master_tile_name(
                mod_s.as_deref().unwrap_or("game"),
                ts_s.as_deref().unwrap_or(""),
                int_arg(c, 3),
                int_arg(c, 4),
            )
        });
        Ok(c.push(res.map_or(Value::Nil, |s| Value::str(&s))))
    });
    b.method("isMasterTile", |c| {
        let mod_s = str_arg(c, 1);
        let ts_s = str_arg(c, 2);
        let res = SEAM_MGR.with(|m| {
            m.borrow().is_master_tile(
                mod_s.as_deref().unwrap_or("game"),
                ts_s.as_deref().unwrap_or(""),
                int_arg(c, 3),
                int_arg(c, 4),
            )
        });
        Ok(c.push(res))
    });
    b.method("write", |_c| Ok(0));
    b.method("Reset", |_c| {
        SEAM_MGR.with(|m| m.borrow_mut().reset());
        Ok(0)
    });
}

pub fn register(thread: &mut Thread, fs: Option<&Rc<RefCell<GameFs>>>) {
    if let Some(fs) = fs {
        if let Ok(fs_ref) = fs.try_borrow() {
            TILE_GEOMETRY_MGR.with(|m| m.borrow_mut().init_from_fs(&fs_ref));
            SEAM_MGR.with(|m| m.borrow_mut().init_from_fs(&fs_ref));
        }
    }
    register_tile_geometry_manager(thread);
    register_tile_depth_texture_manager(thread);
    register_seam_manager(thread);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::{self, load_string};
    use pz_kahlua::TableRef;

    fn run(t: &mut Thread, code: &str) -> Vec<Value> {
        let f = load_string(code, Some("bind_tile_depth_test"), t.globals().clone()).unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|(e, trace)| panic!("{e:?}\n{trace}"))
    }

    #[test]
    fn tile_geometry_depth_texture_and_seam_manager_lua_round_trip() {
        TILE_GEOMETRY_MGR.with(|m| {
            let mut m = m.borrow_mut();
            m.reset();
            m.load_mod_text(
                "game",
                r#"
                tileGeometry {
                    VERSION = 2,
                    tileset {
                        name = sheet_01,
                        tile {
                            xy = 1x1,
                            properties {
                                Surface = 29,
                                Translucent = true,
                            }
                        }
                    }
                }
                "#,
            );
        });
        SEAM_MGR.with(|m| {
            let mut m = m.borrow_mut();
            m.reset();
            m.load_mod_text(
                "game",
                r#"
                seams {
                    VERSION = 1,
                    tileset {
                        name = roofs_01,
                        tile {
                            xy = 0x0,
                            east {
                                roofs_01_0 =,
                                roofs_01_8 =,
                            }
                            belowSouth {
                                roofs_01_6 =,
                            }
                        }
                        tile {
                            xy = 1x0,
                            properties {
                                master = roofs_01_0,
                            }
                        }
                    }
                }
                "#,
            );
        });

        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_globals::register(&mut t);
        register(&mut t, None);

        let out = run(
            &mut t,
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            local gm = TileGeometryManager.getInstance()
            assert(gm:getTileProperty("game", "sheet_01", 1, 1, "Surface") == "29")
            gm:setTileProperty("game", "sheet_01", 1, 1, "Surface", "32")
            assert(gm:getTileProperty("game", "sheet_01", 1, 1, "Surface") == "32")
            assert(gm:getModIDs():size() == 1)

            local dm = TileDepthTextureManager.getInstance()
            assert(dm:isLoadingFinished())
            local dt = dm:getTexture("game", "sheet_01", 9)
            assert(dt ~= nil and dt:getName() == "sheet_01_9" and dt:getIndex() == 9)
            local dt2 = dm:getTextureFromTileName("game", "roofs_01_6")
            assert(dt2 ~= nil and dt2:getIndex() == 6)

            local sm = SeamManager.getInstance()
            local east = sm:getTileJoinE("game", "roofs_01", 0, 0, false)
            assert(east ~= nil and east:size() == 2 and east:get(0) == "roofs_01_0")
            local bs = sm:getTileJoinBelowS("game", "roofs_01", 0, 0, false)
            assert(bs ~= nil and bs:size() == 1 and bs:get(0) == "roofs_01_6")
            assert(sm:getMasterTileName("game", "roofs_01", 1, 0) == "roofs_01_0")
            assert(sm:isMasterTile("game", "roofs_01", 0, 0))
            assert(not sm:isMasterTile("game", "roofs_01", 1, 0))

            return east:size(), dt:getIndex()
            "#,
        );
        assert_eq!(out[0].as_num(), Some(2.0));
        assert_eq!(out[1].as_num(), Some(9.0));

        TILE_GEOMETRY_MGR.with(|m| m.borrow_mut().reset());
        SEAM_MGR.with(|m| m.borrow_mut().reset());
    }
}
