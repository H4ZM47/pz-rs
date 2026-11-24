//! Build 42.22 `zombie.worldMap.MapDefinitions`:
//! reads `LootMaps.Init` string keys from Lua into the cached map-definition
//! list used by `MapItem` / `InventoryItemFactory` (`ItemEnv::map_definitions`)
//! and `WorldDictionary` (`ScriptItems::map_definitions`).

use crate::expose::expose_class;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub const MAP_DEFINITIONS_CLASS: &str = "zombie.worldMap.MapDefinitions";

pub struct MapDefinitionsObject;

impl LuaObject for MapDefinitionsObject {
    fn class_name(&self) -> &str {
        MAP_DEFINITIONS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static INSTANCE: RefCell<Option<Rc<MapDefinitionsObject>>> = const { RefCell::new(None) };
    static DEFINITIONS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn instance() -> Rc<MapDefinitionsObject> {
    INSTANCE.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| Rc::new(MapDefinitionsObject))
            .clone()
    })
}

/// `MapDefinitions.initDefinitionsFromLua()`: read every string key in the
/// global Lua `LootMaps.Init` table (if present) into `definitions`.
pub fn init_from_lua(thread: &Thread) {
    let mut defs = Vec::new();
    if let Value::Table(loot_maps) = thread.globals().get_str("LootMaps") {
        if let Value::Table(init) = loot_maps.get_str("Init") {
            if let Ok(keys) = init.try_keys() {
                for key in keys {
                    if let Value::Str(s) = key {
                        defs.push(s.to_string());
                    }
                }
            }
        }
    }
    DEFINITIONS.with(|d| *d.borrow_mut() = defs);
}

/// Snapshot of `MapDefinitions.definitions` for `ItemEnv` / `ScriptItems`.
pub fn definitions() -> Vec<String> {
    DEFINITIONS.with(|d| d.borrow().clone())
}

/// `MapDefinitions.pickRandom()`: returns a random definition name using
/// `Rand`, or `None` when no definitions are loaded.
pub fn pick_random() -> Option<String> {
    DEFINITIONS.with(|d| {
        let defs = d.borrow();
        if defs.is_empty() {
            return None;
        }
        let idx = pz_core::random::next(defs.len() as i32).max(0) as usize;
        defs.get(idx).cloned()
    })
}

/// `MapDefinitions.Reset()`: clears the cached definitions.
pub fn reset() {
    DEFINITIONS.with(|d| d.borrow_mut().clear());
}

pub fn register(thread: &mut Thread) {
    let cls = expose_class(thread, MAP_DEFINITIONS_CLASS, Some("java.lang.Object"));
    cls.static_fn("getInstance", |c| Ok(c.push(Value::Object(instance()))));
    cls.static_fn("Reset", |_c| {
        reset();
        Ok(0)
    });
    cls.method("initDefinitionsFromLua", |c| {
        init_from_lua(c.thread);
        Ok(0)
    });
    cls.method("pickRandom", |c| {
        let empty = DEFINITIONS.with(|d| d.borrow().is_empty());
        if empty {
            init_from_lua(c.thread);
        }
        match pick_random() {
            Some(name) => Ok(c.push(Value::str(&name))),
            None => Ok(c.push(Value::Nil)),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    #[test]
    fn map_definitions_reads_loot_maps_init_and_resets() {
        reset();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        register(&mut t);

        let f = stdlib::load_string(
            r#"
            LootMaps = {
                Init = {
                    MuldraughMap = function() end,
                    RosewoodMap = function() end,
                }
            }
            local md = MapDefinitions.getInstance()
            local picked = md:pickRandom()
            return picked
            "#,
            Some("map_defs"),
            t.globals().clone(),
        )
        .unwrap();
        let out = t.pcall(&f, &[]).unwrap();
        assert!(matches!(out[0].as_str(), Some("MuldraughMap" | "RosewoodMap")));

        let defs = definitions();
        assert_eq!(defs, vec!["MuldraughMap".to_string(), "RosewoodMap".to_string()]);

        let r = stdlib::load_string("MapDefinitions.Reset()", Some("reset"), t.globals().clone()).unwrap();
        t.pcall(&r, &[]).unwrap();
        assert!(definitions().is_empty());
    }
}
