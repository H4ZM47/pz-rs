//! `zombie.worldMap.WorldMapSettings` — singleton map/minimap UI settings
//! store used by `ISMiniMap.lua` and `ISWorldMap.lua`.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use indexmap::IndexMap;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};

use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg};

pub const WORLD_MAP_SETTINGS_CLASS: &str = "zombie.worldMap.WorldMapSettings";

const DEFAULT_BOOLEANS: &[(&str, bool)] = &[
    ("WorldMap.HighlightStreet", true),
    ("WorldMap.Isometric", true),
    ("WorldMap.LargeStreetLabel", false),
    ("WorldMap.PlaceNames", true),
    ("WorldMap.Players", true),
    ("WorldMap.ShowPrintMedia", true),
    ("WorldMap.ShowStreetNames", true),
    ("WorldMap.ShowSymbolsUI", false),
    ("WorldMap.Symbols", true),
    ("WorldMap.TerrainImage", false),
    ("MiniMap.Isometric", true),
    ("MiniMap.ShowSymbols", false),
    ("MiniMap.StartVisible", true),
    ("MiniMap.TerrainImage", false),
];

const DEFAULT_DOUBLES: &[(&str, f64)] = &[
    ("WorldMap.CenterX", 0.0),
    ("WorldMap.CenterY", 0.0),
    ("WorldMap.Zoom", 0.0),
    ("MiniMap.Zoom", 19.0),
];

#[derive(Clone, Debug)]
struct SettingsState {
    file_version: i32,
    booleans: IndexMap<String, bool>,
    doubles: IndexMap<String, f64>,
}

impl Default for SettingsState {
    fn default() -> Self {
        let mut booleans = IndexMap::new();
        for &(k, v) in DEFAULT_BOOLEANS {
            booleans.insert(k.to_owned(), v);
        }
        let mut doubles = IndexMap::new();
        for &(k, v) in DEFAULT_DOUBLES {
            doubles.insert(k.to_owned(), v);
        }
        Self {
            file_version: 0,
            booleans,
            doubles,
        }
    }
}

/// Lua singleton for `zombie.worldMap.WorldMapSettings`.
pub struct LuaWorldMapSettings;

impl LuaObject for LuaWorldMapSettings {
    fn class_name(&self) -> &str {
        WORLD_MAP_SETTINGS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static STATE: RefCell<SettingsState> = RefCell::new(SettingsState::default());
    static INSTANCE: Rc<LuaWorldMapSettings> = Rc::new(LuaWorldMapSettings);
}

/// `WorldMapSettings.Reset()`.
pub fn reset() {
    STATE.with(|s| *s.borrow_mut() = SettingsState::default());
}

/// Expose `zombie.worldMap.WorldMapSettings`.
pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, WORLD_MAP_SETTINGS_CLASS, Some("java.lang.Object"));
    b.static_fn("getInstance", |c| {
        let v = INSTANCE.with(|inst| Value::Object(inst.clone()));
        Ok(c.push(v))
    });
    b.static_fn("Reset", |_| {
        reset();
        Ok(0)
    });
    b.method("getFileVersion", |c| {
        let v = STATE.with(|s| s.borrow().file_version);
        Ok(c.push(v))
    });
    b.method("getBoolean", |c| {
        let name = str_arg(c, 1);
        let v = name
            .and_then(|n| STATE.with(|s| s.borrow().booleans.get(n.as_ref()).copied()))
            .unwrap_or(false);
        Ok(c.push(v))
    });
    b.method("setBoolean", |c| {
        if let Some(name) = str_arg(c, 1) {
            let v = bool_arg(c, 2);
            STATE.with(|s| {
                s.borrow_mut().booleans.insert(name.as_ref().to_owned(), v);
            });
        }
        Ok(0)
    });
    b.method("getDouble", |c| {
        let default = num_arg(c, 2);
        let v = str_arg(c, 1)
            .and_then(|n| STATE.with(|s| s.borrow().doubles.get(n.as_ref()).copied()))
            .unwrap_or(default);
        Ok(c.push(v))
    });
    b.method("setDouble", |c| {
        if let Some(name) = str_arg(c, 1) {
            let v = num_arg(c, 2);
            STATE.with(|s| {
                s.borrow_mut().doubles.insert(name.as_ref().to_owned(), v);
            });
        }
        Ok(0)
    });
    b.method("getOptionCount", |c| {
        let n = STATE.with(|s| {
            let s = s.borrow();
            (s.booleans.len() + s.doubles.len()) as i32
        });
        Ok(c.push(n))
    });
    b.method("getOptionByIndex", |c| {
        let idx = int_arg(c, 1);
        let v = if idx >= 0 {
            STATE.with(|s| {
                let s = s.borrow();
                let i = idx as usize;
                s.booleans
                    .get_index(i)
                    .map(|(k, _)| Value::str(k))
                    .or_else(|| {
                        s.doubles
                            .get_index(i.saturating_sub(s.booleans.len()))
                            .map(|(k, _)| Value::str(k))
                    })
                    .unwrap_or(Value::Nil)
            })
        } else {
            Value::Nil
        };
        Ok(c.push(v))
    });
    b.method("save", |_| {
        STATE.with(|s| s.borrow_mut().file_version = 1);
        Ok(0)
    });
    b.method("load", |_| {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            if s.file_version == 0 {
                s.file_version = 1;
            }
        });
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::load_string;
    use pz_kahlua::TableRef;

    #[test]
    fn world_map_settings_get_set_and_save() {
        reset();
        let mut t = Thread::new(TableRef::new());
        register(&mut t);
        let f = load_string(
            r#"
            local s = WorldMapSettings.getInstance()
            local v0 = s:getFileVersion()
            s:setDouble("WorldMap.Zoom", 14.5)
            s:setBoolean("MiniMap.StartVisible", false)
            s:save()
            return v0, s:getFileVersion(), s:getDouble("WorldMap.Zoom", 0.0), s:getBoolean("MiniMap.StartVisible")
            "#,
            Some("wm_settings_test.lua"),
            t.globals().clone(),
        )
        .unwrap();
        let res = t.pcall(&f, &[]).unwrap();
        assert_eq!(res[0].as_num(), Some(0.0));
        assert_eq!(res[1].as_num(), Some(1.0));
        assert_eq!(res[2].as_num(), Some(14.5));
        assert!(!res[3].truthy());
        reset();
    }
}
