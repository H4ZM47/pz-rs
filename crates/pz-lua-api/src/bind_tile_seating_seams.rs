//! Build 42.22 Lua bindings for:
//! - `zombie.seating.SeatingManager` & `SeatingFile$Tile`
//! - `zombie.seams.SeamManager` & `SeamFile$Tile`
//! - `zombie.tileDepth.TileDepthTextureManager`
//! - `zombie.tileDepth.TileGeometryManager` & `TileGeometryFile$Tile`

use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, Thread, Value};

use crate::bind_java_util::JavaList;
use crate::expose::expose_class;

const SEATING_MGR_CLASS: &str = "zombie.seating.SeatingManager";
const SEATING_TILE_CLASS: &str = "zombie.seating.SeatingFile$Tile";
const SEAM_MGR_CLASS: &str = "zombie.seams.SeamManager";
const SEAM_TILE_CLASS: &str = "zombie.seams.SeamFile$Tile";
const DEPTH_MGR_CLASS: &str = "zombie.tileDepth.TileDepthTextureManager";
const GEOM_MGR_CLASS: &str = "zombie.tileDepth.TileGeometryManager";
const GEOM_TILE_CLASS: &str = "zombie.tileDepth.TileGeometryFile$Tile";

#[derive(Clone, Debug, Default)]
struct SeatingPos {
    id: String,
    properties: HashMap<String, String>,
}

#[derive(Clone, Debug, Default)]
struct SeatingTile {
    col: i32,
    row: i32,
    positions: Vec<SeatingPos>,
    properties: HashMap<String, String>,
}

#[derive(Default)]
struct SeatingState {
    /// mod_id -> (tileset, col, row) -> tile
    tiles: BTreeMap<String, HashMap<(String, i32, i32), Rc<RefCell<SeatingTile>>>>,
}

impl SeatingState {
    fn ensure_mod(&mut self, mod_id: &str) {
        self.tiles.entry(mod_id.to_string()).or_default();
    }

    fn get_tile(
        &self,
        mod_id: &str,
        tileset: &str,
        col: i32,
        row: i32,
    ) -> Option<Rc<RefCell<SeatingTile>>> {
        self.tiles
            .get(mod_id)?
            .get(&(tileset.to_string(), col, row))
            .cloned()
    }

    fn get_or_create_tile(
        &mut self,
        mod_id: &str,
        tileset: &str,
        col: i32,
        row: i32,
    ) -> Rc<RefCell<SeatingTile>> {
        self.tiles
            .entry(mod_id.to_string())
            .or_default()
            .entry((tileset.to_string(), col, row))
            .or_insert_with(|| {
                Rc::new(RefCell::new(SeatingTile {
                    col,
                    row,
                    ..Default::default()
                }))
            })
            .clone()
    }

    fn first_tile(&self, tileset: &str, col: i32, row: i32) -> Option<Rc<RefCell<SeatingTile>>> {
        for map in self.tiles.values() {
            if let Some(t) = map.get(&(tileset.to_string(), col, row)) {
                return Some(t.clone());
            }
        }
        None
    }
}

#[derive(Clone, Debug, Default)]
struct SeamTile {
    col: i32,
    row: i32,
    properties: HashMap<String, String>,
    join_e: Vec<String>,
    join_s: Vec<String>,
    join_below_e: Vec<String>,
    join_below_s: Vec<String>,
    master_tile: Option<String>,
}

#[derive(Default)]
struct SeamState {
    tiles: BTreeMap<String, HashMap<(String, i32, i32), Rc<RefCell<SeamTile>>>>,
}

impl SeamState {
    fn ensure_mod(&mut self, mod_id: &str) {
        self.tiles.entry(mod_id.to_string()).or_default();
    }

    fn get_tile(
        &self,
        mod_id: &str,
        tileset: &str,
        col: i32,
        row: i32,
    ) -> Option<Rc<RefCell<SeamTile>>> {
        self.tiles
            .get(mod_id)?
            .get(&(tileset.to_string(), col, row))
            .cloned()
    }

    fn get_or_create_tile(
        &mut self,
        mod_id: &str,
        tileset: &str,
        col: i32,
        row: i32,
    ) -> Rc<RefCell<SeamTile>> {
        self.tiles
            .entry(mod_id.to_string())
            .or_default()
            .entry((tileset.to_string(), col, row))
            .or_insert_with(|| {
                Rc::new(RefCell::new(SeamTile {
                    col,
                    row,
                    ..Default::default()
                }))
            })
            .clone()
    }

    fn highest_priority_tile(
        &self,
        tileset: &str,
        col: i32,
        row: i32,
    ) -> Option<Rc<RefCell<SeamTile>>> {
        for map in self.tiles.values() {
            if let Some(t) = map.get(&(tileset.to_string(), col, row)) {
                return Some(t.clone());
            }
        }
        None
    }
}

#[derive(Clone, Debug, Default)]
struct GeomTile {
    col: i32,
    row: i32,
    properties: HashMap<String, String>,
    geometry: Vec<Value>,
}

#[derive(Default)]
struct GeomState {
    tiles: BTreeMap<String, HashMap<(String, i32, i32), Rc<RefCell<GeomTile>>>>,
}

impl GeomState {
    fn ensure_mod(&mut self, mod_id: &str) {
        self.tiles.entry(mod_id.to_string()).or_default();
    }

    fn get_tile(
        &self,
        mod_id: &str,
        tileset: &str,
        col: i32,
        row: i32,
    ) -> Option<Rc<RefCell<GeomTile>>> {
        self.tiles
            .get(mod_id)?
            .get(&(tileset.to_string(), col, row))
            .cloned()
    }

    fn get_or_create_tile(
        &mut self,
        mod_id: &str,
        tileset: &str,
        col: i32,
        row: i32,
    ) -> Rc<RefCell<GeomTile>> {
        self.tiles
            .entry(mod_id.to_string())
            .or_default()
            .entry((tileset.to_string(), col, row))
            .or_insert_with(|| {
                Rc::new(RefCell::new(GeomTile {
                    col,
                    row,
                    ..Default::default()
                }))
            })
            .clone()
    }
}

#[derive(Default)]
struct DepthState {
    loading_tasks: i32,
}

thread_local! {
    static SEATING: RefCell<SeatingState> = RefCell::new(SeatingState::default());
    static SEAMS: RefCell<SeamState> = RefCell::new(SeamState::default());
    static GEOM: RefCell<GeomState> = RefCell::new(GeomState::default());
    static DEPTH: RefCell<DepthState> = RefCell::new(DepthState::default());
}

/// Reset session state on world teardown.
pub fn reset() {
    SEATING.with(|s| *s.borrow_mut() = SeatingState::default());
    SEAMS.with(|s| *s.borrow_mut() = SeamState::default());
    GEOM.with(|g| *g.borrow_mut() = GeomState::default());
    DEPTH.with(|d| *d.borrow_mut() = DepthState::default());
}

macro_rules! simple_handle {
    ($name:ident, $class:expr) => {
        #[derive(Debug)]
        struct $name;
        impl LuaObject for $name {
            fn class_name(&self) -> &'static str {
                $class
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
    };
}

simple_handle!(SeatingMgrHandle, SEATING_MGR_CLASS);
simple_handle!(SeamMgrHandle, SEAM_MGR_CLASS);
simple_handle!(DepthMgrHandle, DEPTH_MGR_CLASS);
simple_handle!(GeomMgrHandle, GEOM_MGR_CLASS);

#[derive(Debug)]
struct SeatingTileObj(Rc<RefCell<SeatingTile>>);
impl LuaObject for SeatingTileObj {
    fn class_name(&self) -> &'static str {
        SEATING_TILE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct SeamTileObj(Rc<RefCell<SeamTile>>);
impl LuaObject for SeamTileObj {
    fn class_name(&self) -> &'static str {
        SEAM_TILE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct GeomTileObj(Rc<RefCell<GeomTile>>);
impl LuaObject for GeomTileObj {
    fn class_name(&self) -> &'static str {
        GEOM_TILE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn str_arg(c: &NativeCall<'_>, idx: usize) -> Option<String> {
    c.arg(idx).as_str().map(str::to_string)
}

fn i32_arg(c: &NativeCall<'_>, idx: usize) -> i32 {
    c.arg(idx).as_num().unwrap_or(0.0) as i32
}

fn split_tile_name(tile_name: &str) -> Option<(&str, i32, i32)> {
    let idx = tile_name.rfind('_')?;
    let tileset = &tile_name[..idx];
    let tile_idx: i32 = tile_name[idx + 1..].parse().ok()?;
    Some((tileset, tile_idx % 8, tile_idx / 8))
}

fn list_of_strings(items: &[String]) -> Value {
    Value::Object(Rc::new(JavaList::new(
        items.iter().map(|s| Value::str(s)).collect(),
    )))
}

/// Register `SeatingManager`, `SeamManager`, `TileDepthTextureManager`, and
/// `TileGeometryManager`.
pub fn register(thread: &mut Thread) {
    // SeatingFile$Tile
    let st = expose_class(thread, SEATING_TILE_CLASS, Some("java.lang.Object"));
    st.method("getPositionCount", |c| {
        let n = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<SeatingTileObj>()
                .map(|t| t.0.borrow().positions.len())
                .unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(n as f64))
    });
    st.method("isEmpty", |c| {
        let empty = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<SeatingTileObj>()
                .is_some_and(|t| {
                    let b = t.0.borrow();
                    b.positions.is_empty() && b.properties.is_empty()
                }),
            _ => true,
        };
        Ok(c.push(empty))
    });

    // SeatingManager
    let sm = expose_class(thread, SEATING_MGR_CLASS, Some("java.lang.Object"));
    sm.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(SeatingMgrHandle))))
    });
    sm.method("init", |_| {
        SEATING.with(|s| s.borrow_mut().ensure_mod("game"));
        Ok(0)
    });
    sm.method("initGameData", |_| {
        SEATING.with(|s| s.borrow_mut().ensure_mod("game"));
        Ok(0)
    });
    sm.method("initModData", |_| Ok(0));
    sm.method("mergeAfterEditing", |_| Ok(0));
    sm.method("fixDefaultPositions", |_| Ok(0));
    sm.method("write", |_| Ok(0));
    sm.method("Reset", |_| {
        SEATING.with(|s| *s.borrow_mut() = SeatingState::default());
        Ok(0)
    });
    sm.method("getModIDs", |c| {
        let ids: Vec<String> = SEATING.with(|s| s.borrow().tiles.keys().cloned().collect());
        Ok(c.push(list_of_strings(&ids)))
    });
    sm.method("addTilePosition", |c| {
        let (Some(mod_id), Some(ts), Some(pos_id)) =
            (str_arg(c, 1), str_arg(c, 2), str_arg(c, 5))
        else {
            return Ok(c.push(-1.0));
        };
        let (col, row) = (i32_arg(c, 3), i32_arg(c, 4));
        let idx = SEATING.with(|s| {
            let tile = s.borrow_mut().get_or_create_tile(&mod_id, &ts, col, row);
            let mut b = tile.borrow_mut();
            b.positions.push(SeatingPos {
                id: pos_id,
                properties: HashMap::new(),
            });
            (b.positions.len() - 1) as i32
        });
        Ok(c.push(idx as f64))
    });
    sm.method("removeTilePosition", |c| {
        if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            let (col, row, idx) = (i32_arg(c, 3), i32_arg(c, 4), i32_arg(c, 5));
            SEATING.with(|s| {
                if let Some(tile) = s.borrow().get_tile(&mod_id, &ts, col, row) {
                    let mut b = tile.borrow_mut();
                    if idx >= 0 && (idx as usize) < b.positions.len() {
                        b.positions.remove(idx as usize);
                    }
                }
            });
        }
        Ok(0)
    });
    sm.method("getTilePositionCount", |c| {
        let count = if c.nargs() >= 5 {
            let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) else {
                return Ok(c.push(0.0));
            };
            SEATING.with(|s| {
                s.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .map(|t| t.borrow().positions.len())
                    .unwrap_or(0)
            })
        } else if c.nargs() >= 4 {
            let Some(ts) = str_arg(c, 1) else {
                return Ok(c.push(0.0));
            };
            SEATING.with(|s| {
                s.borrow()
                    .first_tile(&ts, i32_arg(c, 2), i32_arg(c, 3))
                    .map(|t| t.borrow().positions.len())
                    .unwrap_or(0)
            })
        } else {
            0
        };
        Ok(c.push(count as f64))
    });
    sm.method("getTilePositionID", |c| {
        let out = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            let (col, row, idx) = (i32_arg(c, 3), i32_arg(c, 4), i32_arg(c, 5));
            SEATING.with(|s| {
                s.borrow()
                    .get_tile(&mod_id, &ts, col, row)
                    .and_then(|t| t.borrow().positions.get(idx as usize).map(|p| p.id.clone()))
            })
        } else {
            None
        };
        Ok(c.push(out.map_or(Value::Nil, |s| Value::str(&s))))
    });
    sm.method("hasTilePositionWithID", |c| {
        let has = if let (Some(mod_id), Some(ts), Some(pos_id)) =
            (str_arg(c, 1), str_arg(c, 2), str_arg(c, 5))
        {
            SEATING.with(|s| {
                s.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .is_some_and(|t| t.borrow().positions.iter().any(|p| p.id == pos_id))
            })
        } else {
            false
        };
        Ok(c.push(has))
    });
    sm.method("getTilePositionProperty", |c| {
        let out = if let (Some(mod_id), Some(ts), Some(key)) =
            (str_arg(c, 1), str_arg(c, 2), str_arg(c, 6))
        {
            let (col, row, idx) = (i32_arg(c, 3), i32_arg(c, 4), i32_arg(c, 5));
            SEATING.with(|s| {
                s.borrow()
                    .get_tile(&mod_id, &ts, col, row)
                    .and_then(|t| {
                        t.borrow()
                            .positions
                            .get(idx as usize)
                            .and_then(|p| p.properties.get(&key).cloned())
                    })
            })
        } else {
            None
        };
        Ok(c.push(out.map_or(Value::Nil, |s| Value::str(&s))))
    });
    sm.method("setTilePositionProperty", |c| {
        if let (Some(mod_id), Some(ts), Some(key)) =
            (str_arg(c, 1), str_arg(c, 2), str_arg(c, 6))
        {
            let (col, row, idx) = (i32_arg(c, 3), i32_arg(c, 4), i32_arg(c, 5));
            let val = str_arg(c, 7);
            SEATING.with(|s| {
                if let Some(tile) = s.borrow().get_tile(&mod_id, &ts, col, row) {
                    if let Some(pos) = tile.borrow_mut().positions.get_mut(idx as usize) {
                        match val {
                            Some(v) => {
                                pos.properties.insert(key, v);
                            }
                            None => {
                                pos.properties.remove(&key);
                            }
                        }
                    }
                }
            });
        }
        Ok(0)
    });
    sm.method("getTileProperty", |c| {
        let out = if c.nargs() >= 6 {
            let (Some(mod_id), Some(ts), Some(key)) =
                (str_arg(c, 1), str_arg(c, 2), str_arg(c, 5))
            else {
                return Ok(c.push(Value::Nil));
            };
            SEATING.with(|s| {
                s.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .and_then(|t| t.borrow().properties.get(&key).cloned())
            })
        } else {
            let (Some(ts), Some(key)) = (str_arg(c, 1), str_arg(c, 4)) else {
                return Ok(c.push(Value::Nil));
            };
            SEATING.with(|s| {
                s.borrow()
                    .first_tile(&ts, i32_arg(c, 2), i32_arg(c, 3))
                    .and_then(|t| t.borrow().properties.get(&key).cloned())
            })
        };
        Ok(c.push(out.map_or(Value::Nil, |s| Value::str(&s))))
    });
    sm.method("setTileProperty", |c| {
        if let (Some(mod_id), Some(ts), Some(key)) =
            (str_arg(c, 1), str_arg(c, 2), str_arg(c, 5))
        {
            let val = str_arg(c, 6);
            SEATING.with(|s| {
                let tile =
                    s.borrow_mut()
                        .get_or_create_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4));
                match val {
                    Some(v) => {
                        tile.borrow_mut().properties.insert(key, v);
                    }
                    None => {
                        tile.borrow_mut().properties.remove(&key);
                    }
                }
            });
        }
        Ok(0)
    });
    sm.method("getTile", |c| {
        let out = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            SEATING.with(|s| {
                s.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .map(|rc| Value::Object(Rc::new(SeatingTileObj(rc))))
            })
        } else {
            None
        };
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    sm.method("getOrCreateTile", |c| {
        let out = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            SEATING.with(|s| {
                let rc =
                    s.borrow_mut()
                        .get_or_create_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4));
                Some(Value::Object(Rc::new(SeatingTileObj(rc))))
            })
        } else {
            None
        };
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    sm.method("getFacingDirection", |c| Ok(c.push(Value::str("S"))));
    sm.method("getAdjacentPosition", |c| Ok(c.push(false)));
    sm.method("getAnimationTrackFraction", |c| Ok(c.push(0.0)));

    // SeamFile$Tile
    let seam_t = expose_class(thread, SEAM_TILE_CLASS, Some("java.lang.Object"));
    seam_t.method("getMasterTileName", |c| {
        let out = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<SeamTileObj>()
                .and_then(|t| t.0.borrow().master_tile.clone()),
            _ => None,
        };
        Ok(c.push(out.map_or(Value::Nil, |s| Value::str(&s))))
    });
    seam_t.method("isMasterTile", |c| {
        let ok = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<SeamTileObj>()
                .is_some_and(|t| t.0.borrow().master_tile.is_none()),
            _ => false,
        };
        Ok(c.push(ok))
    });

    // SeamManager
    let seam = expose_class(thread, SEAM_MGR_CLASS, Some("java.lang.Object"));
    seam.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(SeamMgrHandle))))
    });
    seam.method("init", |_| {
        SEAMS.with(|s| s.borrow_mut().ensure_mod("game"));
        Ok(0)
    });
    seam.method("initGameData", |_| {
        SEAMS.with(|s| s.borrow_mut().ensure_mod("game"));
        Ok(0)
    });
    seam.method("initModData", |_| Ok(0));
    seam.method("write", |_| Ok(0));
    seam.method("Reset", |_| {
        SEAMS.with(|s| *s.borrow_mut() = SeamState::default());
        Ok(0)
    });
    seam.method("getModIDs", |c| {
        let ids: Vec<String> = SEAMS.with(|s| s.borrow().tiles.keys().cloned().collect());
        Ok(c.push(list_of_strings(&ids)))
    });
    seam.method("getHighestPriorityTile", |c| {
        let out = str_arg(c, 1).and_then(|ts| {
            SEAMS.with(|s| {
                s.borrow()
                    .highest_priority_tile(&ts, i32_arg(c, 2), i32_arg(c, 3))
                    .map(|rc| Value::Object(Rc::new(SeamTileObj(rc))))
            })
        });
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    seam.method("getHighestPriorityTileFromName", |c| {
        let out = str_arg(c, 1).and_then(|name| {
            let (ts, col, row) = split_tile_name(&name)?;
            SEAMS.with(|s| {
                s.borrow()
                    .highest_priority_tile(ts, col, row)
                    .map(|rc| Value::Object(Rc::new(SeamTileObj(rc))))
            })
        });
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    seam.method("getTileProperty", |c| {
        let out = if let (Some(mod_id), Some(ts), Some(key)) =
            (str_arg(c, 1), str_arg(c, 2), str_arg(c, 5))
        {
            SEAMS.with(|s| {
                s.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .and_then(|t| t.borrow().properties.get(&key).cloned())
            })
        } else {
            None
        };
        Ok(c.push(out.map_or(Value::Nil, |s| Value::str(&s))))
    });
    seam.method("setTileProperty", |c| {
        if let (Some(mod_id), Some(ts), Some(key)) =
            (str_arg(c, 1), str_arg(c, 2), str_arg(c, 5))
        {
            let val = str_arg(c, 6);
            SEAMS.with(|s| {
                let tile =
                    s.borrow_mut()
                        .get_or_create_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4));
                match val {
                    Some(v) => {
                        tile.borrow_mut().properties.insert(key, v);
                    }
                    None => {
                        tile.borrow_mut().properties.remove(&key);
                    }
                }
            });
        }
        Ok(0)
    });
    for (mname, sel) in [
        ("getTileJoinE", 0u8),
        ("getTileJoinS", 1u8),
        ("getTileJoinBelowE", 2u8),
        ("getTileJoinBelowS", 3u8),
    ] {
        seam.method(mname, move |c| {
            let items = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
                let (col, row) = (i32_arg(c, 3), i32_arg(c, 4));
                let allocate = matches!(c.arg(5), Value::Bool(true));
                SEAMS.with(|s| {
                    let tile = if allocate {
                        Some(s.borrow_mut().get_or_create_tile(&mod_id, &ts, col, row))
                    } else {
                        s.borrow().get_tile(&mod_id, &ts, col, row)
                    };
                    tile.map(|t| {
                        let b = t.borrow();
                        let v = match sel {
                            0 => &b.join_e,
                            1 => &b.join_s,
                            2 => &b.join_below_e,
                            _ => &b.join_below_s,
                        };
                        list_of_strings(v)
                    })
                })
            } else {
                None
            };
            Ok(c.push(items.unwrap_or(Value::Nil)))
        });
    }
    seam.method("getTile", |c| {
        let out = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            SEAMS.with(|s| {
                s.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .map(|rc| Value::Object(Rc::new(SeamTileObj(rc))))
            })
        } else {
            None
        };
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    seam.method("getTileFromName", |c| {
        let out = if let (Some(mod_id), Some(name)) = (str_arg(c, 1), str_arg(c, 2)) {
            split_tile_name(&name).and_then(|(ts, col, row)| {
                SEAMS.with(|s| {
                    s.borrow()
                        .get_tile(&mod_id, ts, col, row)
                        .map(|rc| Value::Object(Rc::new(SeamTileObj(rc))))
                })
            })
        } else {
            None
        };
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    seam.method("getOrCreateTile", |c| {
        let out = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            SEAMS.with(|s| {
                let rc =
                    s.borrow_mut()
                        .get_or_create_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4));
                Some(Value::Object(Rc::new(SeamTileObj(rc))))
            })
        } else {
            None
        };
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    seam.method("isMasterTile", |c| {
        let ok = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            SEAMS.with(|s| {
                s.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .is_some_and(|t| t.borrow().master_tile.is_none())
            })
        } else {
            false
        };
        Ok(c.push(ok))
    });
    seam.method("getMasterTileName", |c| {
        let out = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            SEAMS.with(|s| {
                s.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .and_then(|t| t.borrow().master_tile.clone())
            })
        } else {
            None
        };
        Ok(c.push(out.map_or(Value::Nil, |s| Value::str(&s))))
    });

    // TileDepthTextureManager
    let dm = expose_class(thread, DEPTH_MGR_CLASS, Some("java.lang.Object"));
    dm.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(DepthMgrHandle))))
    });
    dm.method("init", |_| Ok(0));
    dm.method("initGameData", |_| Ok(0));
    dm.method("initModData", |_| Ok(0));
    dm.method("mergeAfterEditing", |_| Ok(0));
    dm.method("reloadTileset", |_| Ok(0));
    dm.method("loadTilesetPixelsIfNeeded", |_| Ok(0));
    dm.method("saveTileset", |_| Ok(0));
    dm.method("initSprites", |_| Ok(0));
    dm.method("loadedTileDefinitions", |_| Ok(0));
    dm.method("Reset", |_| {
        DEPTH.with(|d| *d.borrow_mut() = DepthState::default());
        Ok(0)
    });
    dm.method("addedLoadTask", |_| {
        DEPTH.with(|d| d.borrow_mut().loading_tasks += 1);
        Ok(0)
    });
    dm.method("finishedLoadTask", |_| {
        DEPTH.with(|d| {
            let mut s = d.borrow_mut();
            s.loading_tasks = (s.loading_tasks - 1).max(0);
        });
        Ok(0)
    });
    dm.method("isLoadingFinished", |c| {
        let done = DEPTH.with(|d| d.borrow().loading_tasks <= 0);
        Ok(c.push(done))
    });
    dm.method("getTexture", |c| Ok(c.push(Value::Nil)));
    dm.method("getTextureFromTileName", |c| Ok(c.push(Value::Nil)));
    dm.method("getDefaultDepthTexture", |c| Ok(c.push(Value::Nil)));
    dm.method("getBillboardDepthTexture", |c| Ok(c.push(Value::Nil)));
    dm.method("getPresetTilesetDepthTexture", |c| Ok(c.push(Value::Nil)));
    dm.method("getPresetDepthTexture", |c| Ok(c.push(Value::Nil)));
    dm.method("getEmptyDepthTexture", |c| Ok(c.push(Value::Nil)));

    // TileGeometryFile$Tile
    let gt = expose_class(thread, GEOM_TILE_CLASS, Some("java.lang.Object"));
    gt.method("isEmpty", |c| {
        let empty = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<GeomTileObj>()
                .is_some_and(|t| {
                    let b = t.0.borrow();
                    b.geometry.is_empty() && b.properties.is_empty()
                }),
            _ => true,
        };
        Ok(c.push(empty))
    });

    // TileGeometryManager
    let gm = expose_class(thread, GEOM_MGR_CLASS, Some("java.lang.Object"));
    gm.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(GeomMgrHandle))))
    });
    gm.method("init", |_| {
        GEOM.with(|g| g.borrow_mut().ensure_mod("game"));
        Ok(0)
    });
    gm.method("initGameData", |_| {
        GEOM.with(|g| g.borrow_mut().ensure_mod("game"));
        Ok(0)
    });
    gm.method("initModData", |_| Ok(0));
    gm.method("loadedTileDefinitions", |_| Ok(0));
    gm.method("initSpriteProperties", |_| Ok(0));
    gm.method("write", |_| Ok(0));
    gm.method("Reset", |_| {
        GEOM.with(|g| *g.borrow_mut() = GeomState::default());
        Ok(0)
    });
    gm.method("getModIDs", |c| {
        let ids: Vec<String> = GEOM.with(|g| g.borrow().tiles.keys().cloned().collect());
        Ok(c.push(list_of_strings(&ids)))
    });
    gm.method("setGeometry", |c| {
        if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            let items = match c.arg(5) {
                Value::Object(o) => o
                    .as_any()
                    .downcast_ref::<JavaList>()
                    .map(|l| l.items.borrow().clone())
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            GEOM.with(|g| {
                let tile =
                    g.borrow_mut()
                        .get_or_create_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4));
                tile.borrow_mut().geometry = items;
            });
        }
        Ok(0)
    });
    gm.method("copyGeometry", |c| {
        if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            if let Value::Object(o) = c.arg(5) {
                if let Some(list) = o.as_any().downcast_ref::<JavaList>() {
                    GEOM.with(|g| {
                        if let Some(tile) =
                            g.borrow().get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                        {
                            list.items.borrow_mut().extend(tile.borrow().geometry.clone());
                        }
                    });
                }
            }
        }
        Ok(0)
    });
    gm.method("getGeometry", |c| {
        let items = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            GEOM.with(|g| {
                g.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .map(|t| Value::Object(Rc::new(JavaList::new(t.borrow().geometry.clone()))))
            })
        } else {
            None
        };
        Ok(c.push(items.unwrap_or(Value::Nil)))
    });
    gm.method("getTileProperty", |c| {
        let out = if let (Some(mod_id), Some(ts), Some(key)) =
            (str_arg(c, 1), str_arg(c, 2), str_arg(c, 5))
        {
            GEOM.with(|g| {
                g.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .and_then(|t| t.borrow().properties.get(&key).cloned())
            })
        } else {
            None
        };
        Ok(c.push(out.map_or(Value::Nil, |s| Value::str(&s))))
    });
    gm.method("setTileProperty", |c| {
        if let (Some(mod_id), Some(ts), Some(key)) =
            (str_arg(c, 1), str_arg(c, 2), str_arg(c, 5))
        {
            let val = str_arg(c, 6);
            GEOM.with(|g| {
                let tile =
                    g.borrow_mut()
                        .get_or_create_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4));
                match val {
                    Some(v) => {
                        tile.borrow_mut().properties.insert(key, v);
                    }
                    None => {
                        tile.borrow_mut().properties.remove(&key);
                    }
                }
            });
        }
        Ok(0)
    });
    gm.method("getTile", |c| {
        let out = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            GEOM.with(|g| {
                g.borrow()
                    .get_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4))
                    .map(|rc| Value::Object(Rc::new(GeomTileObj(rc))))
            })
        } else {
            None
        };
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    gm.method("getOrCreateTile", |c| {
        let out = if let (Some(mod_id), Some(ts)) = (str_arg(c, 1), str_arg(c, 2)) {
            GEOM.with(|g| {
                let rc =
                    g.borrow_mut()
                        .get_or_create_tile(&mod_id, &ts, i32_arg(c, 3), i32_arg(c, 4));
                Some(Value::Object(Rc::new(GeomTileObj(rc))))
            })
        } else {
            None
        };
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn seating_seam_depth_and_geometry_managers_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            local sm = SeatingManager.getInstance()
            sm:init()
            local idx = sm:addTilePosition("game", "furniture_seating_01", 2, 3, "Front")
            assert(idx == 0)
            assert(sm:getTilePositionCount("game", "furniture_seating_01", 2, 3) == 1)
            assert(sm:hasTilePositionWithID("game", "furniture_seating_01", 2, 3, "Front"))
            sm:setTilePositionProperty("game", "furniture_seating_01", 2, 3, 0, "facing", "S")
            assert(sm:getTilePositionProperty("game", "furniture_seating_01", 2, 3, 0, "facing") == "S")
            sm:setTileProperty("game", "furniture_seating_01", 2, 3, "custom", "yes")
            assert(sm:getTileProperty("game", "furniture_seating_01", 2, 3, "custom") == "yes")

            local seam = SeamManager.getInstance()
            seam:init()
            seam:setTileProperty("game", "walls_01", 1, 2, "seam", "edge")
            assert(seam:getTileProperty("game", "walls_01", 1, 2, "seam") == "edge")
            assert(seam:getTileFromName("game", "walls_01_17") ~= nil)
            assert(seam:getTileJoinE("game", "walls_01", 1, 2, true):size() == 0)

            local dm = TileDepthTextureManager.getInstance()
            assert(dm:isLoadingFinished() == true)
            dm:addedLoadTask()
            assert(dm:isLoadingFinished() == false)
            dm:finishedLoadTask()
            assert(dm:isLoadingFinished() == true)

            local gm = TileGeometryManager.getInstance()
            gm:init()
            gm:setTileProperty("game", "walls_01", 0, 1, "plane", "W")
            assert(gm:getTileProperty("game", "walls_01", 0, 1, "plane") == "W")
            local list = ArrayList.new()
            list:add("box0")
            gm:setGeometry("game", "walls_01", 0, 1, list)
            assert(gm:getGeometry("game", "walls_01", 0, 1):size() == 1)
            "#,
            Some("tile_seating_seams_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        reset();
    }
}
