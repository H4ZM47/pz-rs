//! Build 42.22 Lua bindings for:
//! - `zombie.entity.components.fluids.FluidUtil`
//! - `zombie.core.TradingState`
//! - `zombie.inventory.recipemanager.RecipeMonitor`
//! - `zombie.worldMap.WorldMapSettings`

use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, TableRef, Thread, Value};

use crate::bind_java_util::JavaList;
use crate::expose::expose_class;

const FLUID_UTIL_CLASS: &str = "zombie.entity.components.fluids.FluidUtil";
const TRADING_STATE_CLASS: &str = "zombie.core.TradingState";
const RECIPE_MONITOR_CLASS: &str = "zombie.inventory.recipemanager.RecipeMonitor";
const WORLD_MAP_SETTINGS_CLASS: &str = "zombie.worldMap.WorldMapSettings";
const MAP_CONFIG_OPT_CLASS: &str = "zombie.config.ConfigOption";

const UNIT_L: f32 = 1.0;
const UNIT_DL: f32 = 0.1;
const UNIT_CL: f32 = 0.01;
const UNIT_ML: f32 = 0.001;
const UNIT_DML: f32 = 0.0001;
const UNIT_CML: f32 = 0.00001;
const UNIT_UL: f32 = 0.000001;
const MIN_UNIT: f32 = UNIT_DML;
const MIN_CONTAINER_CAPACITY: f32 = 0.05;
const TRANSFER_ACTION_TIME_PER_LITER: f32 = 40.0;
const MIN_TRANSFER_ACTION_TIME: f32 = 15.0;

#[derive(Default)]
struct RecipeMonitorState {
    enabled: bool,
    suspended: bool,
    monitor_id: i32,
    tabs: usize,
    recipe_name: String,
    recipe: Option<Value>,
    lines: Vec<String>,
    recipe_lines: Vec<String>,
}

#[derive(Clone, Debug)]
enum MapOptionValue {
    Bool { default: bool, value: bool },
    Double { default: f64, value: f64, min: f64, max: f64 },
}

#[derive(Clone, Debug)]
struct MapOption {
    name: String,
    val: MapOptionValue,
}

struct WorldMapSettingsState {
    options: BTreeMap<String, Rc<RefCell<MapOption>>>,
    file_version: i32,
}

impl Default for WorldMapSettingsState {
    fn default() -> Self {
        let mut s = Self {
            options: BTreeMap::new(),
            file_version: 1,
        };
        for (name, def) in [
            ("WorldMap.Isometric", true),
            ("WorldMap.ShowSymbolsUI", true),
            ("WorldMap.ShowPlayers", true),
            ("WorldMap.ShowRemotePlayers", true),
            ("WorldMap.ShowPlayerNames", true),
            ("WorldMap.HideUnvisited", true),
            ("WorldMap.HitTest", false),
            ("WorldMap.ImagePyramid", false),
            ("WorldMap.LineUpscaling", true),
            ("WorldMap.OutlineHorizontal", false),
            ("WorldMap.ShowCellGrid", false),
            ("WorldMap.ShowTileGrid", false),
            ("WorldMap.ShowUnvisitedGrid", true),
            ("WorldMap.TerrainImage", false),
            ("WorldMap.ColorblindPatterns", false),
            ("MiniMap.Isometric", true),
            ("MiniMap.ShowSymbols", true),
            ("MiniMap.StartVisible", true),
        ] {
            s.insert_bool(name, def);
        }
        s.insert_double("MiniMap.Zoom", 19.0, 12.0, 24.0);
        s
    }
}

impl WorldMapSettingsState {
    fn insert_bool(&mut self, name: &str, def: bool) {
        self.options.insert(
            name.to_string(),
            Rc::new(RefCell::new(MapOption {
                name: name.to_string(),
                val: MapOptionValue::Bool {
                    default: def,
                    value: def,
                },
            })),
        );
    }

    fn insert_double(&mut self, name: &str, def: f64, min: f64, max: f64) {
        self.options.insert(
            name.to_string(),
            Rc::new(RefCell::new(MapOption {
                name: name.to_string(),
                val: MapOptionValue::Double {
                    default: def,
                    value: def,
                    min,
                    max,
                },
            })),
        );
    }

    fn set_boolean(&mut self, name: &str, v: bool) {
        if let Some(opt) = self.options.get(name) {
            opt.borrow_mut().val = MapOptionValue::Bool {
                default: v,
                value: v,
            };
        } else {
            self.insert_bool(name, v);
        }
    }

    fn get_boolean(&self, name: &str) -> bool {
        self.options
            .get(name)
            .map(|o| match o.borrow().val {
                MapOptionValue::Bool { value, .. } => value,
                MapOptionValue::Double { value, .. } => value != 0.0,
            })
            .unwrap_or(false)
    }

    fn set_double(&mut self, name: &str, v: f64) {
        if let Some(opt) = self.options.get(name) {
            let mut guard = opt.borrow_mut();
            match &mut guard.val {
                MapOptionValue::Double { value, min, max, .. } => {
                    *value = v.clamp(*min, *max);
                }
                MapOptionValue::Bool { .. } => {
                    guard.val = MapOptionValue::Double {
                        default: v,
                        value: v,
                        min: f64::NEG_INFINITY,
                        max: f64::INFINITY,
                    };
                }
            }
        } else {
            self.insert_double(name, v, f64::NEG_INFINITY, f64::INFINITY);
        }
    }

    fn get_double(&self, name: &str, default_val: f64) -> f64 {
        self.options
            .get(name)
            .map(|o| match o.borrow().val {
                MapOptionValue::Double { value, .. } => value,
                MapOptionValue::Bool { value, .. } => {
                    if value {
                        1.0
                    } else {
                        0.0
                    }
                }
            })
            .unwrap_or(default_val)
    }
}

thread_local! {
    static RECIPE_MONITOR: RefCell<RecipeMonitorState> = RefCell::new(RecipeMonitorState::default());
    static MAP_SETTINGS: RefCell<WorldMapSettingsState> = RefCell::new(WorldMapSettingsState::default());
}

/// Reset session state on world teardown.
pub fn reset() {
    RECIPE_MONITOR.with(|m| *m.borrow_mut() = RecipeMonitorState::default());
    MAP_SETTINGS.with(|s| *s.borrow_mut() = WorldMapSettingsState::default());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TradingStateVal(&'static str, usize);

impl LuaObject for TradingStateVal {
    fn class_name(&self) -> &'static str {
        TRADING_STATE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct WorldMapSettingsHandle;

impl LuaObject for WorldMapSettingsHandle {
    fn class_name(&self) -> &'static str {
        WORLD_MAP_SETTINGS_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct MapOptionObj(Rc<RefCell<MapOption>>);

impl LuaObject for MapOptionObj {
    fn class_name(&self) -> &'static str {
        MAP_CONFIG_OPT_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn decimal_format(v: f32, max_frac: usize) -> String {
    let formatted = format!("{:.prec$}", v, prec = max_frac);
    if max_frac == 0 || !formatted.contains('.') {
        return formatted;
    }
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn unit_text(key: &str, fallback: &str) -> String {
    let translated = crate::item_tooltip::tr(key);
    if translated == key {
        fallback.to_string()
    } else {
        translated
    }
}

fn format_amount_liter1000(amount: f32) -> String {
    format!("{}{}", decimal_format(amount, 0), unit_text("IGUI_Liter", "L"))
}

fn format_amount_liter10(amount: f32) -> String {
    format!("{}{}", decimal_format(amount, 1), unit_text("IGUI_Liter", "L"))
}

fn format_amount_liter(amount: f32) -> String {
    format!("{}{}", decimal_format(amount, 2), unit_text("IGUI_Liter", "L"))
}

fn format_amount_milli(amount: f32) -> String {
    format!(
        "{}{}",
        (amount * 1000.0 + 0.5).floor() as i32,
        unit_text("IGUI_Milliliter", "mL")
    )
}

fn format_amount(amount: f32) -> String {
    if amount >= 1000.0 {
        format_amount_liter1000(amount)
    } else if amount >= 10.0 {
        format_amount_liter10(amount)
    } else if amount >= 1.0 {
        format_amount_liter(amount)
    } else {
        format_amount_milli(amount)
    }
}

fn format_fraction(n: f32, d: f32) -> String {
    let amount = n.max(d);
    let liter = unit_text("IGUI_Liter", "L");
    if amount >= 1000.0 {
        return format!("{} / {}{liter}", decimal_format(n, 0), decimal_format(d, 0));
    }
    if amount >= 10.0 {
        return format!("{} / {}{liter}", decimal_format(n, 1), decimal_format(d, 1));
    }
    if amount >= 1.0 {
        return format!("{} / {}{liter}", decimal_format(n, 2), decimal_format(d, 2));
    }
    let ml = unit_text("IGUI_Milliliter", "mL");
    format!(
        "{} / {}{ml}",
        (n * 1000.0 + 0.5).floor() as i32,
        (d * 1000.0 + 0.5).floor() as i32
    )
}

fn f32_arg(c: &NativeCall<'_>, idx: usize) -> f32 {
    c.arg(idx).as_num().unwrap_or(0.0) as f32
}

/// Register `FluidUtil`, `TradingState`, `RecipeMonitor`, and `WorldMapSettings`.
pub fn register(thread: &mut Thread) {
    // zombie.entity.components.fluids.FluidUtil
    let fu = expose_class(thread, FLUID_UTIL_CLASS, Some("java.lang.Object"));
    fu.field("UNIT_L", Value::Num(f64::from(UNIT_L)));
    fu.field("UNIT_dL", Value::Num(f64::from(UNIT_DL)));
    fu.field("UNIT_cL", Value::Num(f64::from(UNIT_CL)));
    fu.field("UNIT_mL", Value::Num(f64::from(UNIT_ML)));
    fu.field("UNIT_dmL", Value::Num(f64::from(UNIT_DML)));
    fu.field("UNIT_cmL", Value::Num(f64::from(UNIT_CML)));
    fu.field("UNIT_uL", Value::Num(f64::from(UNIT_UL)));
    fu.field("MIN_UNIT", Value::Num(f64::from(MIN_UNIT)));
    fu.field(
        "MIN_CONTAINER_CAPACITY",
        Value::Num(f64::from(MIN_CONTAINER_CAPACITY)),
    );
    fu.field(
        "TRANSFER_ACTION_TIME_PER_LITER",
        Value::Num(f64::from(TRANSFER_ACTION_TIME_PER_LITER)),
    );
    fu.field(
        "MIN_TRANSFER_ACTION_TIME",
        Value::Num(f64::from(MIN_TRANSFER_ACTION_TIME)),
    );
    fu.static_fn("getUnitLiter", |c| Ok(c.push(f64::from(UNIT_L))));
    fu.static_fn("getUnitDeciLiter", |c| Ok(c.push(f64::from(UNIT_DL))));
    fu.static_fn("getUnitCentiLiter", |c| Ok(c.push(f64::from(UNIT_CL))));
    fu.static_fn("getUnitMilliLiter", |c| Ok(c.push(f64::from(UNIT_ML))));
    fu.static_fn("getUnitDeciMilliLiter", |c| Ok(c.push(f64::from(UNIT_DML))));
    fu.static_fn("getUnitCentiMilliLiter", |c| Ok(c.push(f64::from(UNIT_CML))));
    fu.static_fn("getUnitMicroLiter", |c| Ok(c.push(f64::from(UNIT_UL))));
    fu.static_fn("getMinUnit", |c| Ok(c.push(f64::from(MIN_UNIT))));
    fu.static_fn("getMinContainerCapacity", |c| {
        Ok(c.push(f64::from(MIN_CONTAINER_CAPACITY)))
    });
    fu.static_fn("getTransferActionTimePerLiter", |c| {
        Ok(c.push(f64::from(TRANSFER_ACTION_TIME_PER_LITER)))
    });
    fu.static_fn("getMinTransferActionTime", |c| {
        Ok(c.push(f64::from(MIN_TRANSFER_ACTION_TIME)))
    });
    fu.static_fn("getAmountFormatted", |c| {
        Ok(c.push(Value::str(&format_amount(f32_arg(c, 0)))))
    });
    fu.static_fn("getFractionFormatted", |c| {
        Ok(c.push(Value::str(&format_fraction(f32_arg(c, 0), f32_arg(c, 1)))))
    });
    fu.static_fn("getAmountLiter1000", |c| {
        Ok(c.push(Value::str(&format_amount_liter1000(f32_arg(c, 0)))))
    });
    fu.static_fn("getAmountLiter10", |c| {
        Ok(c.push(Value::str(&format_amount_liter10(f32_arg(c, 0)))))
    });
    fu.static_fn("getAmountLiter", |c| {
        Ok(c.push(Value::str(&format_amount_liter(f32_arg(c, 0)))))
    });
    fu.static_fn("getAmountMilli", |c| {
        Ok(c.push(Value::str(&format_amount_milli(f32_arg(c, 0)))))
    });
    fu.static_fn("roundTransfer", |c| {
        let v = f32_arg(c, 0);
        let rounded = (v * 100.0).round() / 100.0;
        Ok(c.push(f64::from(rounded)))
    });

    // zombie.core.TradingState
    let ts_variants = [
        TradingStateVal("WindowWasClosed", 0),
        TradingStateVal("DealWasSealed", 1),
        TradingStateVal("DealWasUnsealed", 2),
        TradingStateVal("DealWasFinalized", 3),
    ];
    let ts = expose_class(thread, TRADING_STATE_CLASS, Some("java.lang.Object"));
    for v in ts_variants {
        ts.field(v.0, Value::Object(Rc::new(v)));
    }
    ts.method("name", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<TradingStateVal>()
                .map(|v| v.0)
                .unwrap_or(""),
            _ => "",
        };
        Ok(c.push(Value::str(s)))
    });
    ts.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<TradingStateVal>()
                .map(|v| v.0)
                .unwrap_or(""),
            _ => "",
        };
        Ok(c.push(Value::str(s)))
    });
    ts.method("ordinal", |c| {
        let ord = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<TradingStateVal>()
                .map(|v| v.1)
                .unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(ord as f64))
    });
    ts.static_fn("values", move |c| {
        let tbl = TableRef::new();
        for (idx, v) in ts_variants.iter().enumerate() {
            tbl.rawset(Value::Num((idx + 1) as f64), Value::Object(Rc::new(*v)));
        }
        Ok(c.push(Value::Table(tbl)))
    });
    ts.static_fn("valueOf", move |c| {
        let arg0 = c.arg(0);
        let found = arg0.as_str().and_then(|name| {
            ts_variants
                .iter()
                .find(|v| v.0 == name)
                .map(|v| Value::Object(Rc::new(*v)))
        });
        Ok(c.push(found.unwrap_or(Value::Nil)))
    });

    // zombie.inventory.recipemanager.RecipeMonitor
    let rm = expose_class(thread, RECIPE_MONITOR_CLASS, Some("java.lang.Object"));
    rm.static_fn("Enable", |c| {
        let on = matches!(c.arg(0), Value::Bool(true));
        RECIPE_MONITOR.with(|m| m.borrow_mut().enabled = on);
        Ok(0)
    });
    rm.static_fn("IsEnabled", |c| {
        Ok(c.push(RECIPE_MONITOR.with(|m| m.borrow().enabled)))
    });
    rm.static_fn("getMonitorID", |c| {
        Ok(c.push(RECIPE_MONITOR.with(|m| m.borrow().monitor_id) as f64))
    });
    rm.static_fn("StartMonitor", |_| {
        RECIPE_MONITOR.with(|m| {
            let mut s = m.borrow_mut();
            s.monitor_id += 1;
            s.lines.clear();
            s.recipe_lines.clear();
            s.tabs = 0;
        });
        Ok(0)
    });
    rm.static_fn("setRecipe", |c| {
        let v = c.arg(0);
        RECIPE_MONITOR.with(|m| {
            let mut s = m.borrow_mut();
            s.recipe = (!v.is_nil()).then_some(v);
        });
        Ok(0)
    });
    rm.static_fn("getRecipeName", |c| {
        let s = RECIPE_MONITOR.with(|m| m.borrow().recipe_name.clone());
        Ok(c.push(Value::str(&s)))
    });
    rm.static_fn("getRecipe", |c| {
        let v = RECIPE_MONITOR.with(|m| m.borrow().recipe.clone());
        Ok(c.push(v.unwrap_or(Value::Nil)))
    });
    rm.static_fn("getRecipeLines", |c| {
        let items = RECIPE_MONITOR.with(|m| {
            m.borrow()
                .recipe_lines
                .iter()
                .map(|s| Value::str(s))
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    rm.static_fn("canLog", |c| {
        let ok = RECIPE_MONITOR.with(|m| {
            let s = m.borrow();
            s.enabled && !s.suspended
        });
        Ok(c.push(ok))
    });
    rm.static_fn("suspend", |_| {
        RECIPE_MONITOR.with(|m| m.borrow_mut().suspended = true);
        Ok(0)
    });
    rm.static_fn("resume", |_| {
        RECIPE_MONITOR.with(|m| m.borrow_mut().suspended = false);
        Ok(0)
    });
    rm.static_fn("Log", |c| {
        let arg0 = c.arg(0);
        if let Some(msg) = arg0.as_str() {
            RECIPE_MONITOR.with(|m| {
                let mut s = m.borrow_mut();
                if s.enabled && !s.suspended {
                    let prefix = "  ".repeat(s.tabs);
                    s.lines.push(format!("{prefix}{msg}"));
                }
            });
        }
        Ok(0)
    });
    rm.static_fn("LogBlanc", |_| {
        RECIPE_MONITOR.with(|m| {
            let mut s = m.borrow_mut();
            if s.enabled && !s.suspended {
                s.lines.push(String::new());
            }
        });
        Ok(0)
    });
    rm.static_fn("LogList", |_| Ok(0));
    rm.static_fn("LogInit", |_| Ok(0));
    rm.static_fn("LogSources", |_| Ok(0));
    rm.static_fn("LogItem", |_| Ok(0));
    rm.static_fn("ResetTabs", |_| {
        RECIPE_MONITOR.with(|m| m.borrow_mut().tabs = 0);
        Ok(0)
    });
    rm.static_fn("SetTab", |c| {
        let n = c.arg(0).as_num().unwrap_or(0.0).max(0.0) as usize;
        RECIPE_MONITOR.with(|m| m.borrow_mut().tabs = n);
        Ok(0)
    });
    rm.static_fn("IncTab", |_| {
        RECIPE_MONITOR.with(|m| {
            let mut s = m.borrow_mut();
            s.tabs = s.tabs.saturating_add(1);
        });
        Ok(0)
    });
    rm.static_fn("DecTab", |_| {
        RECIPE_MONITOR.with(|m| {
            let mut s = m.borrow_mut();
            s.tabs = s.tabs.saturating_sub(1);
        });
        Ok(0)
    });
    rm.static_fn("GetLines", |c| {
        let items = RECIPE_MONITOR.with(|m| m.borrow().lines.iter().map(|s| Value::str(s)).collect());
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    rm.static_fn("GetColors", |c| {
        Ok(c.push(Value::Object(Rc::new(JavaList::new(Vec::new())))))
    });
    rm.static_fn("GetColorForLine", |c| Ok(c.push(Value::Nil)));
    rm.static_fn("GetSaveDir", |c| Ok(c.push(Value::str(""))));
    rm.static_fn("SaveToFile", |_| Ok(0));

    // zombie.config.ConfigOption methods (for WorldMapSettings options)
    let opt = expose_class(thread, MAP_CONFIG_OPT_CLASS, Some("java.lang.Object"));
    opt.method("getName", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<MapOptionObj>()
                .map(|m| m.0.borrow().name.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        Ok(c.push(Value::str(&s)))
    });
    opt.method("getType", |c| {
        let ty = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<MapOptionObj>()
                .map(|m| match m.0.borrow().val {
                    MapOptionValue::Bool { .. } => "boolean",
                    MapOptionValue::Double { .. } => "double",
                })
                .unwrap_or("boolean"),
            _ => "boolean",
        };
        Ok(c.push(Value::str(ty)))
    });
    opt.method("getValue", |c| {
        let val = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<MapOptionObj>()
                .map(|m| match m.0.borrow().val {
                    MapOptionValue::Bool { value, .. } => Value::Bool(value),
                    MapOptionValue::Double { value, .. } => Value::Num(value),
                })
                .unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
        Ok(c.push(val))
    });
    opt.method("setValue", |c| {
        if let Value::Object(o) = c.arg(0) {
            if let Some(m) = o.as_any().downcast_ref::<MapOptionObj>() {
                let mut guard = m.0.borrow_mut();
                match (&mut guard.val, c.arg(1)) {
                    (MapOptionValue::Bool { value, .. }, Value::Bool(b)) => *value = b,
                    (MapOptionValue::Double { value, min, max, .. }, Value::Num(n)) => {
                        *value = n.clamp(*min, *max);
                    }
                    _ => {}
                }
            }
        }
        Ok(0)
    });
    opt.method("resetToDefault", |c| {
        if let Value::Object(o) = c.arg(0) {
            if let Some(m) = o.as_any().downcast_ref::<MapOptionObj>() {
                let mut guard = m.0.borrow_mut();
                match &mut guard.val {
                    MapOptionValue::Bool { default, value } => *value = *default,
                    MapOptionValue::Double { default, value, .. } => *value = *default,
                }
            }
        }
        Ok(0)
    });

    // zombie.worldMap.WorldMapSettings
    let wms = expose_class(thread, WORLD_MAP_SETTINGS_CLASS, Some("java.lang.Object"));
    wms.field("VERSION1", Value::Num(1.0));
    wms.field("VERSION", Value::Num(1.0));
    wms.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(WorldMapSettingsHandle))))
    });
    wms.static_fn("Reset", |_| {
        MAP_SETTINGS.with(|s| *s.borrow_mut() = WorldMapSettingsState::default());
        Ok(0)
    });
    wms.method("getOptionCount", |c| {
        let n = MAP_SETTINGS.with(|s| s.borrow().options.len());
        Ok(c.push(n as f64))
    });
    wms.method("getOptionByIndex", |c| {
        let idx = c.arg(1).as_num().unwrap_or(-1.0) as isize;
        let out = if idx >= 0 {
            MAP_SETTINGS.with(|s| {
                s.borrow()
                    .options
                    .values()
                    .nth(idx as usize)
                    .cloned()
                    .map(|rc| Value::Object(Rc::new(MapOptionObj(rc))))
            })
        } else {
            None
        };
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    wms.method("getOptionByName", |c| {
        let arg1 = c.arg(1);
        let out = arg1.as_str().and_then(|name| {
            MAP_SETTINGS.with(|s| {
                s.borrow()
                    .options
                    .get(name)
                    .cloned()
                    .map(|rc| Value::Object(Rc::new(MapOptionObj(rc))))
            })
        });
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    wms.method("setBoolean", |c| {
        let arg1 = c.arg(1);
        if let Some(name) = arg1.as_str() {
            let v = matches!(c.arg(2), Value::Bool(true));
            MAP_SETTINGS.with(|s| s.borrow_mut().set_boolean(name, v));
        }
        Ok(0)
    });
    wms.method("getBoolean", |c| {
        let arg1 = c.arg(1);
        let v = arg1
            .as_str()
            .is_some_and(|name| MAP_SETTINGS.with(|s| s.borrow().get_boolean(name)));
        Ok(c.push(v))
    });
    wms.method("setDouble", |c| {
        let arg1 = c.arg(1);
        if let Some(name) = arg1.as_str() {
            let v = c.arg(2).as_num().unwrap_or(0.0);
            MAP_SETTINGS.with(|s| s.borrow_mut().set_double(name, v));
        }
        Ok(0)
    });
    wms.method("getDouble", |c| {
        let arg1 = c.arg(1);
        let def = c.arg(2).as_num().unwrap_or(0.0);
        let v = arg1
            .as_str()
            .map(|name| MAP_SETTINGS.with(|s| s.borrow().get_double(name, def)))
            .unwrap_or(def);
        Ok(c.push(v))
    });
    wms.method("getFileVersion", |c| {
        Ok(c.push(MAP_SETTINGS.with(|s| s.borrow().file_version) as f64))
    });
    wms.method("save", |_| Ok(0));
    wms.method("load", |_| Ok(0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluid_util_trading_state_recipe_monitor_and_map_settings_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            assert(FluidUtil.getUnitLiter() == 1.0)
            assert(math.abs(FluidUtil.getMinContainerCapacity() - 0.05) < 0.0001)
            assert(FluidUtil.getAmountFormatted(0.25) == "250mL")
            assert(FluidUtil.getAmountFormatted(1.5) == "1.5L")
            assert(FluidUtil.getFractionFormatted(0.5, 1.0) == "0.5 / 1L")
            assert(math.abs(FluidUtil.roundTransfer(0.336) - 0.34) < 0.0001)

            assert(TradingState.DealWasSealed:name() == "DealWasSealed")
            assert(TradingState.valueOf("DealWasFinalized"):ordinal() == 3)
            assert(#TradingState.values() == 4)

            RecipeMonitor.Enable(true)
            assert(RecipeMonitor.IsEnabled() and RecipeMonitor.canLog())
            RecipeMonitor.StartMonitor()
            RecipeMonitor.Log("check inputs")
            RecipeMonitor.IncTab()
            RecipeMonitor.Log("found hammer")
            assert(RecipeMonitor.GetLines():size() == 2)
            RecipeMonitor.Enable(false)

            local ms = WorldMapSettings.getInstance()
            assert(ms:getOptionCount() > 10)
            assert(ms:getBoolean("WorldMap.Isometric") == true)
            ms:setBoolean("WorldMap.Isometric", false)
            assert(ms:getBoolean("WorldMap.Isometric") == false)
            ms:setDouble("MiniMap.Zoom", 20.5)
            assert(math.abs(ms:getDouble("MiniMap.Zoom", 0.0) - 20.5) < 0.001)
            local opt = ms:getOptionByName("WorldMap.Isometric")
            assert(opt ~= nil and opt:getName() == "WorldMap.Isometric")
            "#,
            Some("fluidutil_mapsettings_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        reset();
    }
}
