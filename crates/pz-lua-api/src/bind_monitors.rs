//! Build 42.22 diagnostic & admin monitor Lua bindings:
//! - `zombie.inventory.recipemanager.RecipeMonitor` (`ISRecipeMonitor.lua`)
//! - `zombie.network.PVPLogTool` & `zombie.network.PVPLogTool$PVPEvent`
//!   (`ISPVPLogToolUI.lua`, `ISKillAnimal.lua`, `ISKillAnimalInInventory.lua`)
//! - `zombie.core.skinnedmodel.advancedanimation.debug.AnimatorDebugMonitor`
//!   (`ISAnimDebugMonitor.lua`)

use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;

use pz_kahlua::{LuaObject, NativeFunction, TableRef, Thread, Value};

use crate::bind_misc_classes::{color_of, color_value_of};
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg};

pub const RECIPE_MONITOR_CLASS: &str = "zombie.inventory.recipemanager.RecipeMonitor";
pub const PVP_LOG_TOOL_CLASS: &str = "zombie.network.PVPLogTool";
pub const PVP_EVENT_CLASS: &str = "zombie.network.PVPLogTool$PVPEvent";
pub const ANIMATOR_DEBUG_MONITOR_CLASS: &str =
    "zombie.core.skinnedmodel.advancedanimation.debug.AnimatorDebugMonitor";

const PVP_MAX_EVENTS: usize = 10;
const ANIM_FLOAT_HISTORY: usize = 1024;

const COL_GRAY: [f32; 4] = [0.5, 0.5, 0.5, 1.0];
const COL_BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const COL_NEG: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const COL_POS: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const COL_HEADER: [f32; 4] = [0.0, 1.0, 1.0, 1.0];

// ---------------------------------------------------------------------------
// RecipeMonitor
// ---------------------------------------------------------------------------

struct RecipeMonitorState {
    enabled: bool,
    suspended: bool,
    monitor_id: i32,
    tab: usize,
    recipe: Value,
    recipe_name: String,
    recipe_lines: Vec<String>,
    lines: Vec<String>,
    colors: Vec<[f32; 4]>,
    save_dir: PathBuf,
}

impl Default for RecipeMonitorState {
    fn default() -> Self {
        Self {
            enabled: false,
            suspended: false,
            monitor_id: 0,
            tab: 0,
            recipe: Value::Nil,
            recipe_name: "none".to_string(),
            recipe_lines: Vec::new(),
            lines: Vec::new(),
            colors: Vec::new(),
            save_dir: PathBuf::from("RecipeMonitor"),
        }
    }
}

impl RecipeMonitorState {
    fn can_log(&self) -> bool {
        self.enabled && !self.suspended
    }

    fn start_monitor(&mut self) {
        if !self.enabled {
            return;
        }
        self.monitor_id = self.monitor_id.wrapping_add(1);
        self.tab = 0;
        self.lines.clear();
        self.colors.clear();
        self.recipe = Value::Nil;
        self.recipe_name = "none".to_string();
        self.recipe_lines.clear();
        let id = self.monitor_id;
        self.log_colored(&format!("MonitorID = {id}"), COL_HEADER);
    }

    fn log_colored(&mut self, text: &str, color: [f32; 4]) {
        if !self.can_log() {
            return;
        }
        let indent = "  ".repeat(self.tab);
        self.lines.push(format!("{indent}{text}"));
        self.colors.push(color);
    }

    fn save_to_file(&self) {
        if self.lines.is_empty() {
            return;
        }
        if std::fs::create_dir_all(&self.save_dir).is_err() {
            return;
        }
        let safe_name: String = self
            .recipe_name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
            .collect();
        let path = self
            .save_dir
            .join(format!("recipe_log_{}_{safe_name}.txt", self.monitor_id));
        let mut out = String::new();
        out.push_str("# Recipe at time of recording:\n");
        for l in &self.recipe_lines {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str("# Recipe monitor log:\n");
        for l in &self.lines {
            out.push_str(l);
            out.push('\n');
        }
        let _ = std::fs::write(path, out);
    }
}

thread_local! {
    static RECIPE_MONITOR: RefCell<RecipeMonitorState> = RefCell::new(RecipeMonitorState::default());
}

/// Sets the directory used by `RecipeMonitor.GetSaveDir()` / `SaveToFile()`.
pub fn set_recipe_monitor_save_dir(dir: PathBuf) {
    RECIPE_MONITOR.with(|rm| rm.borrow_mut().save_dir = dir);
}

/// Resets `RecipeMonitor` state between sessions/tests.
pub fn reset_recipe_monitor() {
    RECIPE_MONITOR.with(|rm| {
        let dir = rm.borrow().save_dir.clone();
        let mut st = RecipeMonitorState::default();
        st.save_dir = dir;
        *rm.borrow_mut() = st;
    });
}

fn describe_value(val: &Value) -> String {
    match val {
        Value::Nil => "nil".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => n.to_string(),
        Value::Str(s) => s.to_string(),
        Value::Table(_) => "table".to_string(),
        Value::Object(o) => o.class_name().to_string(),
        _ => "function".to_string(),
    }
}

fn register_recipe_monitor(thread: &mut Thread) {
    let t = TableRef::new();
    t.set_str("colGray", color_value_of(COL_GRAY[0], COL_GRAY[1], COL_GRAY[2], COL_GRAY[3]));
    t.set_str("colNeg", color_value_of(COL_NEG[0], COL_NEG[1], COL_NEG[2], COL_NEG[3]));
    t.set_str("colPos", color_value_of(COL_POS[0], COL_POS[1], COL_POS[2], COL_POS[3]));
    t.set_str("colHeader", color_value_of(COL_HEADER[0], COL_HEADER[1], COL_HEADER[2], COL_HEADER[3]));

    t.set_str(
        "Enable",
        Value::Native(NativeFunction::new("RecipeMonitor.Enable", |c| {
            let on = bool_arg(c, 0);
            RECIPE_MONITOR.with(|rm| rm.borrow_mut().enabled = on);
            Ok(0)
        })),
    );
    t.set_str(
        "IsEnabled",
        Value::Native(NativeFunction::new("RecipeMonitor.IsEnabled", |c| {
            let on = RECIPE_MONITOR.with(|rm| rm.borrow().enabled);
            Ok(c.push(on))
        })),
    );
    t.set_str(
        "getMonitorID",
        Value::Native(NativeFunction::new("RecipeMonitor.getMonitorID", |c| {
            let id = RECIPE_MONITOR.with(|rm| rm.borrow().monitor_id);
            Ok(c.push(id))
        })),
    );
    t.set_str(
        "StartMonitor",
        Value::Native(NativeFunction::new("RecipeMonitor.StartMonitor", |_c| {
            RECIPE_MONITOR.with(|rm| rm.borrow_mut().start_monitor());
            Ok(0)
        })),
    );
    t.set_str(
        "getColGray",
        Value::Native(NativeFunction::new("RecipeMonitor.getColGray", |c| {
            Ok(c.push(color_value_of(COL_GRAY[0], COL_GRAY[1], COL_GRAY[2], COL_GRAY[3])))
        })),
    );
    t.set_str(
        "getColBlack",
        Value::Native(NativeFunction::new("RecipeMonitor.getColBlack", |c| {
            Ok(c.push(color_value_of(COL_BLACK[0], COL_BLACK[1], COL_BLACK[2], COL_BLACK[3])))
        })),
    );
    t.set_str(
        "setRecipe",
        Value::Native(NativeFunction::new("RecipeMonitor.setRecipe", |c| {
            let val = c.arg(0);
            let name = match &val {
                Value::Nil => "none".to_string(),
                _ => describe_value(&val),
            };
            RECIPE_MONITOR.with(|rm| {
                let mut rm = rm.borrow_mut();
                rm.recipe = val;
                rm.recipe_name = name;
            });
            Ok(0)
        })),
    );
    t.set_str(
        "getRecipe",
        Value::Native(NativeFunction::new("RecipeMonitor.getRecipe", |c| {
            let val = RECIPE_MONITOR.with(|rm| rm.borrow().recipe.clone());
            Ok(c.push(val))
        })),
    );
    t.set_str(
        "getRecipeName",
        Value::Native(NativeFunction::new("RecipeMonitor.getRecipeName", |c| {
            let name = RECIPE_MONITOR.with(|rm| rm.borrow().recipe_name.clone());
            Ok(c.push(Value::str(&name)))
        })),
    );
    t.set_str(
        "getRecipeLines",
        Value::Native(NativeFunction::new("RecipeMonitor.getRecipeLines", |c| {
            let items: Vec<Value> = RECIPE_MONITOR.with(|rm| {
                rm.borrow()
                    .recipe_lines
                    .iter()
                    .map(|s| Value::str(s))
                    .collect()
            });
            Ok(c.push(crate::bind_java_util::list_value(items)))
        })),
    );
    t.set_str(
        "canLog",
        Value::Native(NativeFunction::new("RecipeMonitor.canLog", |c| {
            let ok = RECIPE_MONITOR.with(|rm| rm.borrow().can_log());
            Ok(c.push(ok))
        })),
    );
    t.set_str(
        "suspend",
        Value::Native(NativeFunction::new("RecipeMonitor.suspend", |_c| {
            RECIPE_MONITOR.with(|rm| rm.borrow_mut().suspended = true);
            Ok(0)
        })),
    );
    t.set_str(
        "resume",
        Value::Native(NativeFunction::new("RecipeMonitor.resume", |_c| {
            RECIPE_MONITOR.with(|rm| rm.borrow_mut().suspended = false);
            Ok(0)
        })),
    );
    t.set_str(
        "Log",
        Value::Native(NativeFunction::new("RecipeMonitor.Log", |c| {
            let text_s = str_arg(c, 0);
            let text = text_s.as_deref().unwrap_or("");
            let col = if c.nargs() >= 2 {
                color_of(&c.arg(1)).unwrap_or(COL_BLACK)
            } else {
                COL_BLACK
            };
            RECIPE_MONITOR.with(|rm| rm.borrow_mut().log_colored(text, col));
            Ok(0)
        })),
    );
    t.set_str(
        "LogBlanc",
        Value::Native(NativeFunction::new("RecipeMonitor.LogBlanc", |_c| {
            RECIPE_MONITOR.with(|rm| rm.borrow_mut().log_colored("", COL_BLACK));
            Ok(0)
        })),
    );
    t.set_str(
        "LogList",
        Value::Native(NativeFunction::new("RecipeMonitor.LogList", |c| {
            let label_s = str_arg(c, 0);
            let label = label_s.as_deref().unwrap_or("");
            RECIPE_MONITOR.with(|rm| rm.borrow_mut().log_colored(&format!("{label} {{"), COL_GRAY));
            RECIPE_MONITOR.with(|rm| rm.borrow_mut().log_colored("}", COL_GRAY));
            Ok(0)
        })),
    );
    t.set_str(
        "LogItem",
        Value::Native(NativeFunction::new("RecipeMonitor.LogItem", |c| {
            let label_s = str_arg(c, 0);
            let label = label_s.as_deref().unwrap_or("");
            let item = describe_value(&c.arg(1));
            RECIPE_MONITOR.with(|rm| {
                rm.borrow_mut()
                    .log_colored(&format!("{label} {item}"), COL_GRAY);
            });
            Ok(0)
        })),
    );
    t.set_str(
        "ResetTabs",
        Value::Native(NativeFunction::new("RecipeMonitor.ResetTabs", |_c| {
            RECIPE_MONITOR.with(|rm| rm.borrow_mut().tab = 0);
            Ok(0)
        })),
    );
    t.set_str(
        "SetTab",
        Value::Native(NativeFunction::new("RecipeMonitor.SetTab", |c| {
            let n = int_arg(c, 0).max(0) as usize;
            RECIPE_MONITOR.with(|rm| rm.borrow_mut().tab = n);
            Ok(0)
        })),
    );
    t.set_str(
        "IncTab",
        Value::Native(NativeFunction::new("RecipeMonitor.IncTab", |_c| {
            RECIPE_MONITOR.with(|rm| {
                let mut rm = rm.borrow_mut();
                rm.tab = rm.tab.saturating_add(1);
            });
            Ok(0)
        })),
    );
    t.set_str(
        "DecTab",
        Value::Native(NativeFunction::new("RecipeMonitor.DecTab", |_c| {
            RECIPE_MONITOR.with(|rm| {
                let mut rm = rm.borrow_mut();
                rm.tab = rm.tab.saturating_sub(1);
            });
            Ok(0)
        })),
    );
    t.set_str(
        "GetLines",
        Value::Native(NativeFunction::new("RecipeMonitor.GetLines", |c| {
            let items: Vec<Value> = RECIPE_MONITOR.with(|rm| {
                rm.borrow().lines.iter().map(|s| Value::str(s)).collect()
            });
            Ok(c.push(crate::bind_java_util::list_value(items)))
        })),
    );
    t.set_str(
        "GetColors",
        Value::Native(NativeFunction::new("RecipeMonitor.GetColors", |c| {
            let items: Vec<Value> = RECIPE_MONITOR.with(|rm| {
                rm.borrow()
                    .colors
                    .iter()
                    .map(|col| color_value_of(col[0], col[1], col[2], col[3]))
                    .collect()
            });
            Ok(c.push(crate::bind_java_util::list_value(items)))
        })),
    );
    t.set_str(
        "GetColorForLine",
        Value::Native(NativeFunction::new("RecipeMonitor.GetColorForLine", |c| {
            let idx = int_arg(c, 0);
            let col = RECIPE_MONITOR.with(|rm| {
                usize::try_from(idx)
                    .ok()
                    .and_then(|i| rm.borrow().colors.get(i).copied())
                    .unwrap_or(COL_BLACK)
            });
            Ok(c.push(color_value_of(col[0], col[1], col[2], col[3])))
        })),
    );
    t.set_str(
        "GetSaveDir",
        Value::Native(NativeFunction::new("RecipeMonitor.GetSaveDir", |c| {
            let dir = RECIPE_MONITOR.with(|rm| rm.borrow().save_dir.to_string_lossy().into_owned());
            Ok(c.push(Value::str(&dir)))
        })),
    );
    t.set_str(
        "SaveToFile",
        Value::Native(NativeFunction::new("RecipeMonitor.SaveToFile", |_c| {
            RECIPE_MONITOR.with(|rm| rm.borrow().save_to_file());
            Ok(0)
        })),
    );
    thread.globals().set_str("RecipeMonitor", Value::Table(t));
}

// ---------------------------------------------------------------------------
// PVPLogTool & PVPLogTool$PVPEvent
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct PvpEventInner {
    text: String,
    timestamp: String,
    wielder: String,
    target: String,
    x: f32,
    y: f32,
    z: f32,
    is_set: bool,
}

#[derive(Clone)]
struct LuaPvpEvent {
    inner: Rc<RefCell<PvpEventInner>>,
}

impl LuaPvpEvent {
    fn empty() -> Self {
        Self {
            inner: Rc::new(RefCell::new(PvpEventInner::default())),
        }
    }

    fn set(&self, text: String, wielder: String, target: String, x: f32, y: f32, z: f32) {
        let mut e = self.inner.borrow_mut();
        e.text = text;
        e.timestamp.clear();
        e.wielder = wielder;
        e.target = target;
        e.x = x;
        e.y = y;
        e.z = z;
        e.is_set = true;
    }

    fn clear(&self) {
        *self.inner.borrow_mut() = PvpEventInner::default();
    }
}

impl LuaObject for LuaPvpEvent {
    fn class_name(&self) -> &str {
        PVP_EVENT_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static PVP_EVENTS: RefCell<Vec<LuaPvpEvent>> = RefCell::new(
        (0..PVP_MAX_EVENTS).map(|_| LuaPvpEvent::empty()).collect()
    );
}

/// Resets `PVPLogTool` events between sessions/tests.
pub fn clear_pvp_events() {
    PVP_EVENTS.with(|evs| {
        for ev in evs.borrow().iter() {
            ev.clear();
        }
    });
}

fn push_pvp_event(text: String, wielder: String, target: String, x: f32, y: f32, z: f32) {
    PVP_EVENTS.with(|evs| {
        let mut evs = evs.borrow_mut();
        if evs.is_empty() {
            for _ in 0..PVP_MAX_EVENTS {
                evs.push(LuaPvpEvent::empty());
            }
        }
        // Shift newest to slot 0 (matching PVPLogTool.setEvent ring behavior).
        let last = evs.pop().unwrap_or_else(LuaPvpEvent::empty);
        last.set(text, wielder, target, x, y, z);
        evs.insert(0, last);
    });
}

fn character_from_value(
    val: &Value,
) -> Option<crate::bind_character_sim::LuaGameCharacterRef> {
    let Value::Object(o) = val else {
        return None;
    };
    o.as_any()
        .downcast_ref::<crate::bind_character_sim::CharacterObject>()
        .map(|co| co.0.clone())
}

fn actor_name_and_pos(val: &Value) -> (String, f32, f32, f32) {
    if let Some(ch) = character_from_value(val) {
        if let Ok(c) = ch.try_borrow() {
            let name = c
                .chat
                .username
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| c.descriptor.as_ref().map(|d| d.borrow().full_name()))
                .unwrap_or_else(|| format!("character#{}", c.id));
            return (name, c.vars.x, c.vars.y, c.vars.z);
        }
    }
    (describe_value(val), 0.0, 0.0, 0.0)
}

fn register_pvp_log_tool(thread: &mut Thread) {
    let ev_cls = expose_class(thread, PVP_EVENT_CLASS, None);
    ev_cls.method("getText", |c| {
        let ev = crate::expose::with_this::<LuaPvpEvent, _>(c, |e| e.inner.borrow().text.clone())?;
        Ok(c.push(Value::str(&ev)))
    });
    ev_cls.method("isSet", |c| {
        let set = crate::expose::with_this::<LuaPvpEvent, _>(c, |e| e.inner.borrow().is_set)?;
        Ok(c.push(set))
    });
    ev_cls.method("getX", |c| {
        let v = crate::expose::with_this::<LuaPvpEvent, _>(c, |e| e.inner.borrow().x as f64)?;
        Ok(c.push(v))
    });
    ev_cls.method("getY", |c| {
        let v = crate::expose::with_this::<LuaPvpEvent, _>(c, |e| e.inner.borrow().y as f64)?;
        Ok(c.push(v))
    });
    ev_cls.method("getZ", |c| {
        let v = crate::expose::with_this::<LuaPvpEvent, _>(c, |e| e.inner.borrow().z as f64)?;
        Ok(c.push(v))
    });
    ev_cls.method("reset", |c| {
        let argc = c.nargs();
        let (text, wielder, target, x, y, z) = if argc >= 7 {
            let a = str_arg(c, 1).map_or_else(String::new, |s| s.to_string());
            let b = str_arg(c, 2).map_or_else(String::new, |s| s.to_string());
            let d = str_arg(c, 3).map_or_else(String::new, |s| s.to_string());
            (
                format!("{a} {b} {d}"),
                a,
                b,
                num_arg(c, 4) as f32,
                num_arg(c, 5) as f32,
                num_arg(c, 6) as f32,
            )
        } else {
            let a = str_arg(c, 1).map_or_else(String::new, |s| s.to_string());
            let b = str_arg(c, 2).map_or_else(String::new, |s| s.to_string());
            (
                format!("{a} {b}"),
                a,
                b,
                num_arg(c, 3) as f32,
                num_arg(c, 4) as f32,
                num_arg(c, 5) as f32,
            )
        };
        crate::expose::with_this::<LuaPvpEvent, _>(c, |e| e.set(text, wielder, target, x, y, z))?;
        Ok(0)
    });

    let t = TableRef::new();
    t.set_str(
        "clearEvents",
        Value::Native(NativeFunction::new("PVPLogTool.clearEvents", |_c| {
            clear_pvp_events();
            Ok(0)
        })),
    );
    t.set_str(
        "getEvents",
        Value::Native(NativeFunction::new("PVPLogTool.getEvents", |c| {
            let list: Vec<Value> = PVP_EVENTS.with(|evs| {
                evs.borrow()
                    .iter()
                    .map(|e| Value::Object(Rc::new(e.clone())))
                    .collect()
            });
            Ok(c.push(crate::bind_java_util::list_value(list)))
        })),
    );
    t.set_str(
        "logSafety",
        Value::Native(NativeFunction::new("PVPLogTool.logSafety", |c| {
            let (name, x, y, z) = actor_name_and_pos(&c.arg(0));
            let event_s = str_arg(c, 1);
            let event = event_s.as_deref().unwrap_or("");
            let text = format!("Safety: \"{name}\" {event}");
            push_pvp_event(text, name, String::new(), x, y, z);
            Ok(0)
        })),
    );
    t.set_str(
        "logKill",
        Value::Native(NativeFunction::new("PVPLogTool.logKill", |c| {
            let (wielder, wx, wy, wz) = actor_name_and_pos(&c.arg(0));
            let (target, tx, ty, tz) = actor_name_and_pos(&c.arg(1));
            let (x, y, z) = if tx != 0.0 || ty != 0.0 || tz != 0.0 {
                (tx, ty, tz)
            } else {
                (wx, wy, wz)
            };
            let text = format!("Kill: \"{wielder}\" killed \"{target}\"");
            push_pvp_event(text, wielder, target, x, y, z);
            Ok(0)
        })),
    );
    t.set_str(
        "logCombat",
        Value::Native(NativeFunction::new("PVPLogTool.logCombat", |c| {
            let (wielder, wx, wy, wz) = actor_name_and_pos(&c.arg(0));
            let (target, tx, ty, tz) = actor_name_and_pos(&c.arg(1));
            let weapon_s = str_arg(c, 2);
            let weapon = weapon_s.as_deref().unwrap_or("");
            let dmg = num_arg(c, 3) as f32;
            let (x, y, z) = if tx != 0.0 || ty != 0.0 || tz != 0.0 {
                (tx, ty, tz)
            } else {
                (wx, wy, wz)
            };
            let text = format!("Combat: \"{wielder}\" hit \"{target}\" \"{weapon}\" {dmg:.2}");
            push_pvp_event(text, wielder, target, x, y, z);
            Ok(0)
        })),
    );
    thread.globals().set_str("PVPLogTool", Value::Table(t));
}

// ---------------------------------------------------------------------------
// AnimatorDebugMonitor
// ---------------------------------------------------------------------------

const DEFAULT_KNOWN_VARS: &[&str] = &[
    "aim",
    "attacktype",
    "baimatfloor",
    "battackfrombehind",
    "bdead",
    "bfalling",
    "bundervehicle",
    "canclimbdownrope",
    "collided",
    "collidetype",
    "fallonfront",
    "frombehind",
    "hashitreaction",
    "hitreaction",
    "intrees",
    "isattacking",
    "reanimatetimer",
];

struct KnownAnimVars {
    vars: BTreeSet<String>,
    dirty: bool,
}

impl Default for KnownAnimVars {
    fn default() -> Self {
        let mut vars = BTreeSet::new();
        for &v in DEFAULT_KNOWN_VARS {
            vars.insert(v.to_string());
        }
        Self { vars, dirty: true }
    }
}

thread_local! {
    static KNOWN_ANIM_VARS: RefCell<KnownAnimVars> = RefCell::new(KnownAnimVars::default());
}

struct AnimMonitorInner {
    target: Value,
    filters: HashMap<i32, bool>,
    do_tick_stamps: bool,
    custom_vars: Vec<String>,
    dirty: bool,
    dirty_float_list: bool,
    log_string: String,
    float_names: Vec<String>,
    selected_var: Option<String>,
    selected_values: Vec<f32>,
    selected_min: f32,
    selected_max: f32,
}

impl AnimMonitorInner {
    fn new(target: Value) -> Self {
        let mut filters = HashMap::new();
        for i in 0..=8 {
            filters.insert(i, true);
        }
        Self {
            target,
            filters,
            do_tick_stamps: false,
            custom_vars: Vec::new(),
            dirty: true,
            dirty_float_list: true,
            log_string: String::new(),
            float_names: Vec::new(),
            selected_var: None,
            selected_values: vec![0.0; ANIM_FLOAT_HISTORY],
            selected_min: -1.0,
            selected_max: 1.0,
        }
    }

    fn read_target_var(&self, key: &str) -> Option<f32> {
        let ch = character_from_value(&self.target)?;
        let c = ch.try_borrow().ok()?;
        let lower = key.to_ascii_lowercase();
        let val = c
            .game_variables
            .game_variables()
            .find(|(k, _)| *k == lower)
            .map(|(_, v)| v.as_float());
        val
    }

    fn refresh_snapshot(&mut self) {
        let Some(ch) = character_from_value(&self.target) else {
            return;
        };
        let Ok(c) = ch.try_borrow() else {
            return;
        };
        let mut lines = Vec::new();
        let mut floats = Vec::new();
        for (k, v) in c.game_variables.game_variables() {
            if self.filters.get(&4).copied().unwrap_or(true) {
                lines.push(format!("[variable] {k} = {}", v.as_string()));
            }
            if matches!(v, pz_anim::VariableValue::Float(_) | pz_anim::VariableValue::Int(_)) {
                floats.push(k.to_string());
            }
        }
        floats.sort();
        lines.sort();
        self.log_string = lines.join(" <LINE> ");
        self.float_names = floats;
    }
}

struct LuaAnimatorDebugMonitor {
    inner: RefCell<AnimMonitorInner>,
}

impl LuaObject for LuaAnimatorDebugMonitor {
    fn class_name(&self) -> &str {
        ANIMATOR_DEBUG_MONITOR_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn register_animator_debug_monitor(thread: &mut Thread) {
    let b = expose_class(thread, ANIMATOR_DEBUG_MONITOR_CLASS, None);
    b.static_fn("new", |c| {
        let target = c.arg(0);
        let mut inner = AnimMonitorInner::new(target);
        inner.refresh_snapshot();
        let mon = Rc::new(LuaAnimatorDebugMonitor {
            inner: RefCell::new(inner),
        });
        Ok(c.push(Value::Object(mon)))
    });
    b.static_fn("isKnownVarsDirty", |c| {
        let dirty = KNOWN_ANIM_VARS.with(|kv| kv.borrow().dirty);
        Ok(c.push(dirty))
    });
    b.static_fn("getKnownVariables", |c| {
        let vars: Vec<Value> = KNOWN_ANIM_VARS.with(|kv| {
            let mut kv = kv.borrow_mut();
            kv.dirty = false;
            kv.vars.iter().map(|s| Value::str(s)).collect()
        });
        Ok(c.push(crate::bind_java_util::list_value(vars)))
    });
    b.static_fn("registerVariable", |c| {
        if let Some(name) = str_arg(c, 0).or_else(|| str_arg(c, 1)) {
            let lower = name.to_ascii_lowercase();
            KNOWN_ANIM_VARS.with(|kv| {
                let mut kv = kv.borrow_mut();
                if kv.vars.insert(lower) {
                    kv.dirty = true;
                }
            });
        }
        Ok(0)
    });

    // Vanilla ISAnimDebugMonitor.lua calls `AnimatorDebugMonitor:isKnownVarsDirty()`
    // with colon syntax on the class table too, so expose them as methods as well.
    b.method("isKnownVarsDirty", |c| {
        let dirty = KNOWN_ANIM_VARS.with(|kv| kv.borrow().dirty);
        Ok(c.push(dirty))
    });
    b.method("getKnownVariables", |c| {
        let vars: Vec<Value> = KNOWN_ANIM_VARS.with(|kv| {
            let mut kv = kv.borrow_mut();
            kv.dirty = false;
            kv.vars.iter().map(|s| Value::str(s)).collect()
        });
        Ok(c.push(crate::bind_java_util::list_value(vars)))
    });

    b.method("getTarget", |c| {
        let val = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            m.inner.borrow().target.clone()
        })?;
        Ok(c.push(val))
    });
    b.method("setTarget", |c| {
        let target = c.arg(1);
        crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            let mut inner = m.inner.borrow_mut();
            inner.target = target;
            inner.dirty = true;
            inner.dirty_float_list = true;
            inner.refresh_snapshot();
        })?;
        Ok(0)
    });
    b.method("addCustomVariable", |c| {
        if let Some(var) = str_arg(c, 1) {
            let v = var.to_ascii_lowercase();
            crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
                let mut inner = m.inner.borrow_mut();
                if !inner.custom_vars.contains(&v) {
                    inner.custom_vars.push(v);
                    inner.dirty = true;
                }
            })?;
        }
        Ok(0)
    });
    b.method("removeCustomVariable", |c| {
        if let Some(var) = str_arg(c, 1) {
            let v = var.to_ascii_lowercase();
            crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
                let mut inner = m.inner.borrow_mut();
                inner.custom_vars.retain(|x| x != &v);
                inner.dirty = true;
            })?;
        }
        Ok(0)
    });
    b.method("setFilter", |c| {
        let idx = int_arg(c, 1);
        let on = bool_arg(c, 2);
        crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            let mut inner = m.inner.borrow_mut();
            inner.filters.insert(idx, on);
            inner.dirty = true;
            inner.refresh_snapshot();
        })?;
        Ok(0)
    });
    b.method("getFilter", |c| {
        let idx = int_arg(c, 1);
        let on = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            m.inner.borrow().filters.get(&idx).copied().unwrap_or(false)
        })?;
        Ok(c.push(on))
    });
    b.method("isDoTickStamps", |c| {
        let on = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            m.inner.borrow().do_tick_stamps
        })?;
        Ok(c.push(on))
    });
    b.method("setDoTickStamps", |c| {
        let on = bool_arg(c, 1);
        crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            m.inner.borrow_mut().do_tick_stamps = on;
        })?;
        Ok(0)
    });
    b.method("IsDirty", |c| {
        let dirty = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            m.inner.borrow().dirty
        })?;
        Ok(c.push(dirty))
    });
    b.method("getLogString", |c| {
        let s = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            let mut inner = m.inner.borrow_mut();
            inner.refresh_snapshot();
            inner.dirty = false;
            inner.log_string.clone()
        })?;
        Ok(c.push(Value::str(&s)))
    });
    b.method("IsDirtyFloatList", |c| {
        let dirty = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            m.inner.borrow().dirty_float_list
        })?;
        Ok(c.push(dirty))
    });
    b.method("getFloatNames", |c| {
        let list: Vec<Value> = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            let mut inner = m.inner.borrow_mut();
            inner.refresh_snapshot();
            inner.dirty_float_list = false;
            inner.float_names.iter().map(|s| Value::str(s)).collect()
        })?;
        Ok(c.push(crate::bind_java_util::list_value(list)))
    });
    b.method("setSelectedVariable", |c| {
        let name = str_arg(c, 1).map(|s| s.to_string());
        crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            let mut inner = m.inner.borrow_mut();
            inner.selected_var = name;
            inner.selected_min = -1.0;
            inner.selected_max = 1.0;
            inner.selected_values.fill(0.0);
        })?;
        Ok(0)
    });
    b.method("getSelectedVariable", |c| {
        let name = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            m.inner.borrow().selected_var.clone()
        })?;
        Ok(c.push(name.map_or(Value::Nil, |s| Value::str(&s))))
    });
    b.method("getSelectedVariableFloat", |c| {
        let v = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            let inner = m.inner.borrow();
            inner
                .selected_var
                .as_deref()
                .and_then(|k| inner.read_target_var(k))
                .unwrap_or(0.0) as f64
        })?;
        Ok(c.push(v))
    });
    b.method("getSelectedVarMinFloat", |c| {
        let s = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            format!("{}", m.inner.borrow().selected_min)
        })?;
        Ok(c.push(Value::str(&s)))
    });
    b.method("getSelectedVarMaxFloat", |c| {
        let s = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            format!("{}", m.inner.borrow().selected_max)
        })?;
        Ok(c.push(Value::str(&s)))
    });
    b.method("getSelectedVarFloatList", |c| {
        let list: Vec<Value> = crate::expose::with_this::<LuaAnimatorDebugMonitor, _>(c, |m| {
            m.inner
                .borrow()
                .selected_values
                .iter()
                .map(|&v| Value::Num(v as f64))
                .collect()
        })?;
        Ok(c.push(crate::bind_java_util::list_value(list)))
    });
}

pub fn register(thread: &mut Thread) {
    register_recipe_monitor(thread);
    register_pvp_log_tool(thread);
    register_animator_debug_monitor(thread);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::{self, load_string};

    fn run(t: &mut Thread, code: &str) -> Vec<Value> {
        let f = load_string(code, Some("bind_monitors_test"), t.globals().clone()).unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|(e, trace)| panic!("{e:?}\n{trace}"))
    }

    #[test]
    fn recipe_monitor_pvp_log_tool_and_animator_debug_monitor_lua_round_trip() {
        reset_recipe_monitor();
        clear_pvp_events();

        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_globals::register(&mut t);
        crate::bind_misc_classes::register(&mut t, None);
        register(&mut t);

        let out = run(
            &mut t,
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            -- RecipeMonitor
            assert(not RecipeMonitor.IsEnabled())
            RecipeMonitor.Enable(true)
            assert(RecipeMonitor.IsEnabled())
            RecipeMonitor.StartMonitor()
            assert(RecipeMonitor.getMonitorID() == 1)
            RecipeMonitor.IncTab()
            RecipeMonitor.Log("step 1", RecipeMonitor.colPos)
            RecipeMonitor.suspend()
            assert(not RecipeMonitor.canLog())
            RecipeMonitor.Log("ignored")
            RecipeMonitor.resume()
            assert(RecipeMonitor.canLog())
            RecipeMonitor.DecTab()
            RecipeMonitor.Log("done")
            local lines = RecipeMonitor.GetLines()
            assert(lines:size() == 3)
            assert(lines:get(1) == "  step 1")
            local col = RecipeMonitor.GetColorForLine(1)
            assert(col:getG() == 1.0 and col:getR() == 0.0)

            -- PVPLogTool
            local evs = PVPLogTool.getEvents()
            assert(evs:size() == 10)
            assert(not evs:get(0):isSet())
            PVPLogTool.logKill("Alice", "Rabbit")
            PVPLogTool.logCombat("Alice", "Bob", "Axe", 25.5)
            PVPLogTool.logSafety("Alice", "enabled")
            evs = PVPLogTool.getEvents()
            assert(evs:get(0):isSet())
            assert(string.find(evs:get(0):getText(), "Safety") ~= nil)
            assert(string.find(evs:get(1):getText(), "Combat") ~= nil)
            assert(string.find(evs:get(2):getText(), "Kill") ~= nil)
            PVPLogTool.clearEvents()
            assert(not PVPLogTool.getEvents():get(0):isSet())

            -- AnimatorDebugMonitor
            assert(AnimatorDebugMonitor:isKnownVarsDirty())
            local known = AnimatorDebugMonitor:getKnownVariables()
            assert(known:size() >= 10)
            assert(not AnimatorDebugMonitor:isKnownVarsDirty())
            local mon = AnimatorDebugMonitor.new(nil)
            assert(mon:getFilter(1))
            mon:setFilter(1, false)
            assert(not mon:getFilter(1))
            mon:setDoTickStamps(true)
            assert(mon:isDoTickStamps())
            mon:setSelectedVariable("aim")
            assert(mon:getSelectedVariable() == "aim")
            assert(mon:getSelectedVarFloatList():size() == 1024)

            return lines:size(), known:size()
            "#,
        );
        assert_eq!(out[0].as_num(), Some(3.0));
        assert!(out[1].as_num().unwrap() >= 10.0);

        reset_recipe_monitor();
        clear_pvp_events();
    }
}
