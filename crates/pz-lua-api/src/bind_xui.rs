//! `zombie.scripting.ui.XuiManager`, `XuiSkin` (with `EntityUiStyle` and
//! `ComponentUiStyle`), `XuiLuaStyle` with its `XuiVar` family, and the
//! `XuiAutoApply`, `XuiVarType` and `XuiScriptType` enums: the XUI skin
//! runtime `ISXuiSkin.build` reads styles from.
//!
//! The script objects (`XuiSkinScript`, `XuiConfigScript`, `XuiColorsScript`)
//! come from `pz-scripts`. `XuiManager.ParseScripts` runs lazily on the first
//! call from Lua (the original runs it as the XUI script buckets finish
//! loading): it registers the style variables named in the `xuiConfig`
//! scripts, adds the `xuiColors` to the game colours, creates one `XuiSkin`
//! per `xuiSkin` script and loads them.
//!
//! Not ported: `XuiScript` and the layout side of `XuiManager`
//! (`GetLayout`, `GetStyle`, `GetDefaultStyle`, the `GetAll*` lists and
//! `Get*Script`), which `ISXuiBuilder` uses; they raise
//! `pz.host.Unsupported`. `XuiLuaStyle` mutators (`setValue`,
//! `setAutoApplyMode`, `setUiOrder`) are not exposed either.

use crate::bind_colors::{color_lua_value, game_color, named_color, register_game_color};
use crate::bind_java_util::list_value;
use crate::bind_misc_classes::Color;
use crate::bind_render::{texture_error, texture_value_ref};
use crate::bind_text::{font_value, translate};
use crate::bind_world_object::unsupported;
use crate::expose::{expose_class, expose_enum, str_arg, with_this};
use pz_core::debug::DebugType;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};
use pz_scripts::objects::definitions::{
    ComponentType, EntityUiScript, StyleInfoScript, XuiColorsScript, XuiConfigScript, XuiSkinScript, XuiVarType,
};
use pz_scripts::{ScriptManager, ScriptType};
use pz_text::UIFont;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub const XUI_MANAGER: &str = "zombie.scripting.ui.XuiManager";
pub const XUI_SKIN: &str = "zombie.scripting.ui.XuiSkin";
pub const ENTITY_UI_STYLE: &str = "zombie.scripting.ui.XuiSkin$EntityUiStyle";
pub const COMPONENT_UI_STYLE: &str = "zombie.scripting.ui.XuiSkin$ComponentUiStyle";
pub const XUI_LUA_STYLE: &str = "zombie.scripting.ui.XuiLuaStyle";
pub const XUI_VAR: &str = "zombie.scripting.ui.XuiLuaStyle$XuiVar";
pub const XUI_AUTO_APPLY: &str = "zombie.scripting.ui.XuiAutoApply";
pub const XUI_VAR_TYPE: &str = "zombie.scripting.ui.XuiVarType";
pub const XUI_SCRIPT_TYPE: &str = "zombie.scripting.ui.XuiScriptType";

const AUTO_APPLY: [&str; 5] = ["No", "Always", "IfSet", "IfSetAndKeyExists", "Forbidden"];
const IF_SET: usize = 2;
const SCRIPT_TYPES: [&str; 4] = ["Layout", "Style", "DefaultStyle", "Reference"];
const DEFAULT_SKIN: &str = "default";
const FALLBACK_ICON: &str = "media/inventory/Question_On.png";

// ---- XuiLuaStyle.XuiVar ----------------------------------------------------------------------

/// A style variable's value. `Font` holds the `UIFont` constant name.
#[derive(Clone)]
enum Val {
    Null,
    Str(String),
    List(Vec<String>),
    Num(f64),
    Bool(bool),
    Font(String),
    Color(Rc<Color>),
}

/// `XuiLuaStyle.XuiVar` and its eight allowed subclasses (`kind`).
#[derive(Clone)]
struct XuiVar {
    kind: XuiVarType,
    key: String,
    ui_order: i32,
    auto_apply: usize,
    default: Val,
    value: Option<Val>,
}

impl XuiVar {
    /// The static prototype `ReadConfigs` registers for `key`.
    fn new(kind: XuiVarType, key: &str) -> XuiVar {
        let default = match kind {
            XuiVarType::StringList => Val::List(Vec::new()),
            XuiVarType::Double => Val::Num(0.0),
            XuiVarType::Boolean => Val::Bool(false),
            XuiVarType::FontType => Val::Font(UIFont::Small.name().to_string()),
            _ => Val::Null,
        };
        XuiVar { kind, key: key.to_string(), ui_order: 1000, auto_apply: IF_SET, default, value: None }
    }

    /// `value()`: the set value, else the default.
    fn value(&self) -> &Val {
        self.value.as_ref().unwrap_or(&self.default)
    }

    fn class_name(&self) -> &'static str {
        match self.kind {
            XuiVarType::StringList => "zombie.scripting.ui.XuiLuaStyle$XuiStringList",
            XuiVarType::TranslateString => "zombie.scripting.ui.XuiLuaStyle$XuiTranslateString",
            XuiVarType::Double => "zombie.scripting.ui.XuiLuaStyle$XuiDouble",
            XuiVarType::Boolean => "zombie.scripting.ui.XuiLuaStyle$XuiBoolean",
            XuiVarType::FontType => "zombie.scripting.ui.XuiLuaStyle$XuiFontType",
            XuiVarType::Color => "zombie.scripting.ui.XuiLuaStyle$XuiColor",
            XuiVarType::Texture => "zombie.scripting.ui.XuiLuaStyle$XuiTexture",
            _ => "zombie.scripting.ui.XuiLuaStyle$XuiString",
        }
    }

    /// `fromString`; a parse failure is logged and leaves the value as it was.
    fn from_string(&mut self, s: Option<&str>, style: &str, colors: &dyn Fn(&str) -> Option<Rc<Color>>) {
        let Some(s) = s else {
            // `XuiVar.setValue(null)` for a nil value in the script.
            self.value = Some(Val::Null);
            return;
        };
        let parsed = match self.kind {
            XuiVarType::StringList => Ok(Val::List(java_split(s, ':').iter().map(|p| p.trim().to_string()).collect())),
            XuiVarType::Double => {
                s.trim().parse::<f64>().map(Val::Num).map_err(|_| format!("java.lang.NumberFormatException: {s}"))
            }
            XuiVarType::Boolean => Ok(Val::Bool(s.eq_ignore_ascii_case("true"))),
            XuiVarType::FontType => {
                let name = if s.starts_with("UIFont.") { &s[s.find('.').map_or(0, |i| i + 1)..] } else { s };
                UIFont::from_string(name)
                    .map(|f| Val::Font(f.name().to_string()))
                    .ok_or_else(|| format!("java.lang.IllegalArgumentException: No enum constant UIFont.{name}"))
            }
            XuiVarType::Color => parse_color(s, style, colors).map(Val::Color),
            _ => Ok(Val::Str(s.to_string())),
        };
        match parsed {
            Ok(v) => self.value = Some(v),
            Err(e) => DebugType::General.error(None, &format!("{e} ({style})")),
        }
    }
}

/// `String.split(":")`: trailing empty strings are dropped.
fn java_split(s: &str, sep: char) -> Vec<&str> {
    if s.is_empty() {
        return vec![s];
    }
    let mut parts: Vec<&str> = s.split(sep).collect();
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

fn parse_float(s: &str) -> Result<f32, String> {
    s.trim().parse::<f32>().map_err(|_| format!("java.lang.NumberFormatException: {s}"))
}

/// `XuiColor.fromString`: a skin or game colour name, else `r:g:b[:a]` in
/// 0..1 or `rgb:r:g:b[:a]` in 0..255.
fn parse_color(s: &str, style: &str, colors: &dyn Fn(&str) -> Option<Rc<Color>>) -> Result<Rc<Color>, String> {
    let mut color = colors(s).or_else(|| named_color(s));
    if color.is_none() && s.contains(':') {
        let c = Color::default();
        let parts = java_split(s, ':');
        if parts.len() < 3 {
            DebugType::General.error(None, &format!("Color has too few components: {s} ({style})"));
        }
        let channels = [&c.r, &c.g, &c.b, &c.a];
        if parts.len() > 1 && parts[0].trim().eq_ignore_ascii_case("rgb") {
            for (i, p) in parts.iter().enumerate().skip(1).take(4) {
                channels[i - 1].set(parse_float(p)? / 255.0);
            }
        } else {
            for (i, p) in parts.iter().enumerate().take(4) {
                channels[i].set(parse_float(p)?);
            }
        }
        color = Some(Rc::new(c));
    }
    color.ok_or_else(|| format!("java.lang.Exception: Unknown color: {s}"))
}

// ---- XuiLuaStyle -----------------------------------------------------------------------------

/// `XuiLuaStyle`: the variables one lua class's style sets, in load order.
struct XuiLuaStyle {
    lua_class: String,
    style_name: String,
    vars: Vec<XuiVar>,
    index: HashMap<String, usize>,
}

impl XuiLuaStyle {
    fn new(lua_class: &str, style_name: &str) -> XuiLuaStyle {
        XuiLuaStyle {
            lua_class: lua_class.to_string(),
            style_name: style_name.to_string(),
            vars: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// `copyVarsFrom`.
    fn copy_vars_from(&mut self, other: &XuiLuaStyle) {
        self.vars = other.vars.clone();
        self.index = other.index.clone();
    }

    /// `loadVar`: false for a nil value on a var that does not take it.
    fn load_var(
        &mut self,
        registry: &HashMap<String, XuiVarType>,
        colors: &dyn Fn(&str) -> Option<Rc<Color>>,
        key: &str,
        value: Option<&str>,
    ) -> Result<bool, String> {
        let i = match self.index.get(key) {
            Some(&i) => i,
            None => {
                let Some(&kind) = registry.get(key) else {
                    return Err(format!(
                        "java.lang.Exception: Var not registered: {key} ({}.{})",
                        self.lua_class, self.style_name
                    ));
                };
                self.vars.push(XuiVar::new(kind, key));
                self.index.insert(key.to_string(), self.vars.len() - 1);
                self.vars.len() - 1
            }
        };
        let label = format!("{}.{}", self.lua_class, self.style_name);
        self.vars[i].from_string(value, &label, colors);
        Ok(true)
    }
}

type StyleRef = Rc<RefCell<XuiLuaStyle>>;

/// `XuiSkin.StyleInfo`.
#[derive(Default)]
struct StyleInfo {
    default_style: Option<StyleRef>,
    styles: HashMap<String, StyleRef>,
}

// ---- XuiSkin ---------------------------------------------------------------------------------

/// `XuiSkin.ComponentUiStyle`.
#[derive(Clone)]
struct ComponentUiStyle {
    lua_panel_class: Option<String>,
    xui_style: Option<String>,
    display_name: Option<String>,
    icon_path: Option<String>,
    list_order_z: i32,
    enabled: bool,
}

impl Default for ComponentUiStyle {
    fn default() -> Self {
        ComponentUiStyle {
            lua_panel_class: None,
            xui_style: None,
            display_name: None,
            icon_path: None,
            list_order_z: 0,
            enabled: true,
        }
    }
}

/// `XuiSkin.EntityUiStyle`.
#[derive(Clone, Default)]
struct EntityUiStyle {
    lua_window_class: Option<String>,
    xui_style: Option<String>,
    lua_can_open_window: Option<String>,
    lua_open_window: Option<String>,
    display_name: Option<String>,
    description: Option<String>,
    build_description: Option<String>,
    icon_path: Option<String>,
    components: Vec<(ComponentType, Rc<ComponentUiStyle>)>,
}

impl EntityUiStyle {
    /// `EntityUiStyle.Load`: overlays the fields the script sets.
    fn load(&mut self, script: &EntityUiScript) {
        let set = |dst: &mut Option<String>, src: &Option<String>| {
            if src.is_some() {
                dst.clone_from(src);
            }
        };
        set(&mut self.lua_window_class, &script.lua_window_class);
        set(&mut self.xui_style, &script.xui_style);
        set(&mut self.lua_can_open_window, &script.lua_can_open_window);
        set(&mut self.lua_open_window, &script.lua_open_window);
        set(&mut self.display_name, &script.display_name);
        set(&mut self.description, &script.description);
        set(&mut self.build_description, &script.build_description);
        set(&mut self.icon_path, &script.icon_path);
        if script.clear_components {
            self.components.clear();
        }
        for (ty, cs) in &script.component_ui_script_map {
            let i = match self.components.iter().position(|(t, _)| t == ty) {
                Some(i) => i,
                None => {
                    self.components.push((*ty, Rc::new(ComponentUiStyle::default())));
                    self.components.len() - 1
                }
            };
            let mut c = (*self.components[i].1).clone();
            set(&mut c.lua_panel_class, &cs.lua_panel_class);
            set(&mut c.xui_style, &cs.xui_style);
            set(&mut c.display_name, &cs.display_name);
            set(&mut c.icon_path, &cs.icon_path);
            if cs.list_order_z_set {
                c.list_order_z = cs.list_order_z;
            }
            if cs.enabled_set {
                c.enabled = cs.enabled;
            }
            self.components[i].1 = Rc::new(c);
        }
    }

    fn component(&self, ty: ComponentType) -> Option<Rc<ComponentUiStyle>> {
        self.components.iter().find(|(t, _)| *t == ty).map(|(_, c)| c.clone())
    }
}

/// `XuiSkin` after `Load`.
#[derive(Default)]
struct SkinData {
    color_map: HashMap<String, Rc<Color>>,
    default_entity: Rc<EntityUiStyle>,
    entities: HashMap<String, Rc<EntityUiStyle>>,
    styles: HashMap<String, StyleInfo>,
}

struct Skin {
    name: String,
    script: Rc<RefCell<XuiSkinScript>>,
    has_loaded: Cell<bool>,
    data: RefCell<SkinData>,
}

impl LuaObject for Skin {
    fn class_name(&self) -> &str {
        XUI_SKIN
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Skin {
    /// `colorInternal(name, true, true)`.
    fn color(&self, name: &str) -> Option<Rc<Color>> {
        self.data.borrow().color_map.get(name).cloned().or_else(|| named_color(name))
    }

    fn entity_style(&self, name: Option<&str>) -> Rc<EntityUiStyle> {
        let data = self.data.borrow();
        match name {
            None => data.default_entity.clone(),
            Some(n) => data.entities.get(n).cloned().unwrap_or_else(|| {
                DebugType::General.warn(None, &format!("Could not find entity ui style '{n}', using default"));
                data.default_entity.clone()
            }),
        }
    }

    fn style(&self, lua_class: &str, style_name: Option<&str>) -> Option<StyleRef> {
        let data = self.data.borrow();
        let info = data.styles.get(lua_class)?;
        match style_name {
            None => info.default_style.clone(),
            Some(n) => info.styles.get(n).cloned(),
        }
    }
}

/// `XuiSkin.Load`.
fn load_skin(
    skin: &Rc<Skin>,
    skins: &HashMap<String, Rc<Skin>>,
    sm: &ScriptManager,
    registry: &HashMap<String, XuiVarType>,
) {
    if skin.has_loaded.replace(true) {
        return;
    }
    let script = skin.script.borrow();
    let mut scripts: Vec<Rc<RefCell<XuiSkinScript>>> = Vec::new();
    let mut colors: HashMap<String, Rc<Color>> = HashMap::new();
    for name in script.imports() {
        let imported = skins.get(name.as_str());
        let import_script = sm.get::<XuiSkinScript>(ScriptType::XuiSkin, name);
        let (Some(imported), Some(import_script)) = (imported, import_script) else {
            DebugType::General
                .error(None, &format!("java.lang.Exception: Skin import not found: {name} in {}", skin.name));
            return;
        };
        load_skin(imported, skins, sm, registry);
        load_colors(&mut colors, &import_script.borrow().colors_script());
        scripts.push(import_script);
    }
    load_colors(&mut colors, script.colors_script());

    let mut default_entity = EntityUiStyle::default();
    let mut styles: HashMap<String, StyleInfo> = HashMap::new();
    let lookup = |n: &str| colors.get(n).cloned().or_else(|| named_color(n));
    let own = skin.script.clone();
    let all: Vec<&Rc<RefCell<XuiSkinScript>>> = scripts.iter().chain(std::iter::once(&own)).collect();
    for s in &all {
        let s = s.borrow();
        default_entity.load(s.default_entity_ui_script());
        for (lua_class, info) in s.style_info_map().iter() {
            load_default_style(&mut styles, skin, registry, &lookup, lua_class, info);
        }
    }
    let mut entities: HashMap<String, Rc<EntityUiStyle>> = HashMap::new();
    for s in &all {
        let s = s.borrow();
        for (name, entity_script) in s.entity_ui_script_map().iter() {
            let mut style = entities.get(name).map_or_else(|| default_entity.clone(), |e| (**e).clone());
            style.load(entity_script);
            entities.insert(name.to_string(), Rc::new(style));
        }
        for (lua_class, info) in s.style_info_map().iter() {
            load_styles(&mut styles, registry, &lookup, lua_class, info);
        }
    }
    *skin.data.borrow_mut() =
        SkinData { color_map: colors.clone(), default_entity: Rc::new(default_entity), entities, styles };
}

/// `XuiSkin.LoadColors`: `colorMap.putAll(script.getColorsScript().getColorMap())`.
fn load_colors(map: &mut HashMap<String, Rc<Color>>, script: &XuiColorsScript) {
    for (name, c) in script.color_map().iter() {
        map.insert(name.to_string(), Rc::new(Color::rgba(c.r, c.g, c.b, c.a)));
    }
}

/// `XuiSkin.LoadDefaultStyle`.
fn load_default_style(
    styles: &mut HashMap<String, StyleInfo>,
    _skin: &Rc<Skin>,
    registry: &HashMap<String, XuiVarType>,
    colors: &dyn Fn(&str) -> Option<Rc<Color>>,
    lua_class: &str,
    info: &StyleInfoScript,
) {
    let entry = styles.entry(lua_class.to_string()).or_default();
    let style = entry
        .default_style
        .get_or_insert_with(|| Rc::new(RefCell::new(XuiLuaStyle::new(lua_class, DEFAULT_SKIN))))
        .clone();
    for (k, v) in info.default_style_block.iter() {
        report(style.borrow_mut().load_var(registry, colors, k, v.as_deref()), k, v.as_deref());
    }
}

/// `XuiSkin.LoadStyle`.
fn load_styles(
    styles: &mut HashMap<String, StyleInfo>,
    registry: &HashMap<String, XuiVarType>,
    colors: &dyn Fn(&str) -> Option<Rc<Color>>,
    lua_class: &str,
    info: &StyleInfoScript,
) {
    let entry = styles.entry(lua_class.to_string()).or_default();
    for (style_name, block) in info.style_blocks.iter() {
        let style = match entry.styles.get(style_name) {
            Some(s) => s.clone(),
            None => {
                let mut s = XuiLuaStyle::new(lua_class, style_name);
                if let Some(d) = &entry.default_style {
                    s.copy_vars_from(&d.borrow());
                }
                let s = Rc::new(RefCell::new(s));
                entry.styles.insert(style_name.to_string(), s.clone());
                s
            }
        };
        for (k, v) in block.iter() {
            report(style.borrow_mut().load_var(registry, colors, k, v.as_deref()), k, v.as_deref());
        }
    }
}

fn report(r: Result<bool, String>, key: &str, value: Option<&str>) {
    match r {
        Ok(true) => {}
        Ok(false) => DebugType::General.warn(None, &format!("Could not load var: {key} = {value:?}")),
        Err(e) => DebugType::General.error(None, &e),
    }
}

// ---- XuiManager ------------------------------------------------------------------------------

#[derive(Default)]
struct State {
    parsed: bool,
    registry: HashMap<String, XuiVarType>,
    skins: HashMap<String, Rc<Skin>>,
    default_skin: Option<Rc<Skin>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// `XuiLuaStyle.ReadConfigs`: the variable names each `xuiConfig` declares.
fn read_configs(configs: &[Rc<RefCell<XuiConfigScript>>]) -> HashMap<String, XuiVarType> {
    let mut registry = HashMap::new();
    let mut seen: HashMap<XuiVarType, Vec<String>> = HashMap::new();
    for config in configs {
        for (ty, keys) in config.borrow().var_configs() {
            for key in keys {
                if seen.iter().any(|(t, set)| t != ty && set.contains(key)) {
                    DebugType::General.error(
                        None,
                        &format!("Key '{key}' duplicate from another value block. this var type = {}", ty.name()),
                    );
                    continue;
                }
                seen.entry(*ty).or_default().push(key.clone());
                registry.entry(key.clone()).or_insert(*ty);
            }
        }
    }
    registry
}

/// The `xuiColors` part of `XuiManager.ParseScripts`: each colour becomes a
/// game colour, or updates the one already registered.
fn register_global_colors(sm: &ScriptManager) {
    for script in sm.get_all::<XuiColorsScript>(ScriptType::XuiColor) {
        for (name, c) in script.borrow().color_map().iter() {
            let color = Rc::new(Color::rgba(c.r, c.g, c.b, c.a));
            if let Some(existing) = game_color(name) {
                existing.r.set(c.r);
                existing.g.set(c.g);
                existing.b.set(c.b);
                existing.a.set(c.a);
            } else if named_color(name).is_none() {
                register_game_color(name, color);
            } else {
                DebugType::General.error(None, &format!("XuiColor '{name}' is already a non-game color"));
            }
        }
    }
}

/// `XuiManager.ParseScripts` without the layout scripts, once the skin
/// scripts are loaded.
fn ensure_parsed() {
    if STATE.with(|s| s.borrow().parsed) {
        return;
    }
    let sm = crate::bind_registry::shared_script_manager();
    let skin_scripts = sm.get_all::<XuiSkinScript>(ScriptType::XuiSkin);
    if skin_scripts.is_empty() {
        return;
    }
    let registry = read_configs(&sm.get_all::<XuiConfigScript>(ScriptType::XuiConfig));
    register_global_colors(&sm);
    let mut skins: HashMap<String, Rc<Skin>> = HashMap::new();
    for script in skin_scripts {
        let (name, module) = {
            let s = script.borrow();
            (s.base.script_object_name().to_string(), s.base.module_name())
        };
        if module.as_deref() != Some("Base") {
            DebugType::General.warn(None, &format!("XuiSkin must be in module Base: {name}"));
            continue;
        }
        let skin = Rc::new(Skin { name: name.clone(), script, has_loaded: Cell::new(false), data: RefCell::default() });
        skins.insert(name, skin);
    }
    let all: Vec<Rc<Skin>> = skins.values().cloned().collect();
    for skin in &all {
        load_skin(skin, &skins, &sm, &registry);
    }
    let default_skin = skins.get(DEFAULT_SKIN).cloned();
    STATE.with(|s| *s.borrow_mut() = State { parsed: true, registry, skins, default_skin });
}

fn default_skin() -> Option<Rc<Skin>> {
    ensure_parsed();
    STATE.with(|s| s.borrow().default_skin.clone())
}

/// `XuiManager.GetSkin(name)`: the default skin when `name` is unknown.
fn get_skin(name: Option<&str>) -> Option<Rc<Skin>> {
    ensure_parsed();
    STATE.with(|s| {
        let s = s.borrow();
        let found = name.and_then(|n| s.skins.get(n).cloned());
        if found.is_none() {
            if let Some(n) = name {
                DebugType::General.warn(None, &format!("XuiSkin not found: {n}, using default"));
            }
        }
        found.or_else(|| s.default_skin.clone())
    })
}

// ---- Lua objects -----------------------------------------------------------------------------

fn skin_value(s: Option<Rc<Skin>>) -> Value {
    s.map_or(Value::Nil, |s| Value::Object(s))
}

struct StyleObj(StyleRef);

impl LuaObject for StyleObj {
    fn class_name(&self) -> &str {
        XUI_LUA_STYLE
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn to_lua_string(&self) -> String {
        let s = self.0.borrow();
        format!("{XUI_LUA_STYLE}@0[xuiLuaClass={}, xuiStyleName={}]", s.lua_class, s.style_name)
    }
}

fn style_value(s: Option<StyleRef>) -> Value {
    s.map_or(Value::Nil, |s| Value::Object(Rc::new(StyleObj(s))))
}

struct VarObj {
    style: StyleRef,
    idx: usize,
}

impl VarObj {
    fn var<R>(&self, f: impl FnOnce(&XuiVar) -> R) -> R {
        f(&self.style.borrow().vars[self.idx])
    }
}

impl LuaObject for VarObj {
    fn class_name(&self) -> &str {
        self.var(XuiVar::class_name)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[XUI_VAR]
    }
}

fn val_value(v: &Val) -> Value {
    match v {
        Val::Null => Value::Nil,
        Val::Str(s) => Value::str(s),
        Val::List(l) => list_value(l.iter().map(|s| Value::str(s)).collect()),
        Val::Num(n) => Value::Num(*n),
        Val::Bool(b) => Value::Bool(*b),
        Val::Font(f) => UIFont::from_string(f).map_or(Value::Nil, font_value),
        Val::Color(c) => color_lua_value(c),
    }
}

/// `XuiVar.value()`; translated for `TranslateString`.
fn var_value(v: &XuiVar) -> Value {
    match (v.kind, v.value()) {
        (XuiVarType::TranslateString, Val::Str(s)) => Value::str(&translate(s)),
        (_, other) => val_value(other),
    }
}

fn color_channel(v: &XuiVar, f: impl Fn(&Color) -> f32) -> f64 {
    match v.value() {
        Val::Color(c) => f64::from(f(c)),
        _ => 0.0,
    }
}

struct EntityStyleObj(Rc<EntityUiStyle>);
struct ComponentStyleObj(Rc<ComponentUiStyle>);

impl LuaObject for EntityStyleObj {
    fn class_name(&self) -> &str {
        ENTITY_UI_STYLE
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl LuaObject for ComponentStyleObj {
    fn class_name(&self) -> &str {
        COMPONENT_UI_STYLE
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn opt_str(s: &Option<String>) -> Value {
    s.as_deref().map_or(Value::Nil, Value::str)
}

fn translated(s: &Option<String>) -> Value {
    s.as_deref().map_or(Value::Nil, |s| Value::str(&translate(s)))
}

/// The icon: `Texture.trygetTexture(path)`, else the question-mark texture.
fn icon_value(path: &Option<String>) -> LuaResult<Value> {
    let Some(path) = path else { return Ok(Value::Nil) };
    let texture = match pz_render::try_get_texture_ref(path).map_err(texture_error)? {
        Some(t) => Some(t),
        None => pz_render::get_shared_texture_ref(FALLBACK_ICON).map_err(texture_error)?,
    };
    Ok(texture_value_ref(texture.as_ref()))
}

fn component_arg(c: &NativeCall<'_>, i: usize) -> Option<ComponentType> {
    match c.arg(i) {
        Value::Object(o) => ComponentType::value_of(&o.to_lua_string()).ok(),
        _ => None,
    }
}

fn unsupported_layouts(name: &str) -> pz_kahlua::LuaError {
    unsupported(format!("XuiManager.{name}: XuiScript layouts are not ported"))
}

pub fn register(thread: &mut Thread) {
    expose_enum(thread, XUI_AUTO_APPLY, &AUTO_APPLY);
    let var_types: Vec<&'static str> = XuiVarType::ALL.iter().map(|t| t.name()).collect();
    expose_enum(thread, XUI_VAR_TYPE, &var_types);
    expose_enum(thread, XUI_SCRIPT_TYPE, &SCRIPT_TYPES);

    let m = expose_class(thread, XUI_MANAGER, None);
    m.static_fn("getDefaultSkinName", |c| Ok(c.push(DEFAULT_SKIN)));
    m.static_fn("GetDefaultSkin", |c| Ok(c.push(skin_value(default_skin()))));
    m.static_fn("GetSkin", |c| {
        let name = str_arg(c, 0);
        Ok(c.push(skin_value(get_skin(name.as_deref()))))
    });
    for name in [
        "GetCombinedScripts",
        "GetAllLayouts",
        "GetAllStyles",
        "GetAllDefaultStyles",
        "GetLayoutScript",
        "GetStyleScript",
        "GetDefaultStyleScript",
        "GetLayout",
        "GetStyle",
        "GetDefaultStyle",
    ] {
        m.static_fn(name, move |_| Err(unsupported_layouts(name)));
    }

    let s = expose_class(thread, XUI_SKIN, None);
    s.static_fn("Default", |c| Ok(c.push(skin_value(default_skin()))));
    s.static_fn("getDefaultSkinName", |c| Ok(c.push(DEFAULT_SKIN)));
    s.method("getName", |c| {
        let v = with_this::<Skin, _>(c, |s| s.name.clone())?;
        Ok(c.push(v))
    });
    s.method("color", |c| {
        let name = str_arg(c, 1);
        let color = with_this::<Skin, _>(c, |s| name.as_deref().and_then(|n| s.color(n)))?;
        Ok(c.push(color.as_ref().map_or(Value::Nil, color_lua_value)))
    });
    s.method("getDefault", |c| {
        let class = str_arg(c, 1);
        let style = with_this::<Skin, _>(c, |s| class.as_deref().and_then(|k| s.style(k, None)))?;
        Ok(c.push(style_value(style)))
    });
    s.method("get", |c| {
        let (class, name) = (str_arg(c, 1), str_arg(c, 2));
        let style = with_this::<Skin, _>(c, |s| class.as_deref().and_then(|k| s.style(k, name.as_deref())))?;
        Ok(c.push(style_value(style)))
    });
    s.method("getEntityUiStyle", |c| {
        let name = str_arg(c, 1);
        let style = with_this::<Skin, _>(c, |s| s.entity_style(name.as_deref()))?;
        Ok(c.push(Value::Object(Rc::new(EntityStyleObj(style)))))
    });
    s.method("getEntityDisplayName", |c| {
        let name = str_arg(c, 1);
        let style = with_this::<Skin, _>(c, |s| s.entity_style(name.as_deref()))?;
        Ok(c.push(translated(&style.display_name)))
    });
    s.method("getComponentUiStyle", |c| {
        let (name, ty) = (str_arg(c, 1), component_arg(c, 2));
        let style = with_this::<Skin, _>(c, |s| s.entity_style(name.as_deref()))?;
        let component = ty.and_then(|t| style.component(t));
        Ok(c.push(component.map_or(Value::Nil, |x| Value::Object(Rc::new(ComponentStyleObj(x))))))
    });

    let e = expose_class(thread, ENTITY_UI_STYLE, None);
    macro_rules! entity_getter {
        ($name:literal, $f:expr) => {
            e.method($name, |c| {
                let v = with_this::<EntityStyleObj, _>(c, |s| $f(&*s.0))?;
                Ok(c.push(v))
            });
        };
    }
    entity_getter!("getLuaWindowClass", |s: &EntityUiStyle| opt_str(&s.lua_window_class));
    entity_getter!("getXuiStyle", |s: &EntityUiStyle| opt_str(&s.xui_style));
    entity_getter!("getDisplayName", |s: &EntityUiStyle| translated(&s.display_name));
    entity_getter!("getDescription", |s: &EntityUiStyle| translated(&s.description));
    entity_getter!("getBuildDescription", |s: &EntityUiStyle| translated(&s.build_description));
    e.method("getIcon", |c| {
        let path = with_this::<EntityStyleObj, _>(c, |s| s.0.icon_path.clone())?;
        let v = icon_value(&path)?;
        Ok(c.push(v))
    });
    for (name, open) in [("getLuaCanOpenWindow", false), ("getLuaOpenWindow", true)] {
        e.method(name, move |c| {
            let f = with_this::<EntityStyleObj, _>(c, |s| {
                if open {
                    s.0.lua_open_window.clone()
                } else {
                    s.0.lua_can_open_window.clone()
                }
            })?;
            let v = f.map_or(Value::Nil, |f| crate::bind_item_container::function_object(c.thread, &f));
            Ok(c.push(v))
        });
    }
    e.method("getComponentUiStyle", |c| {
        let ty = component_arg(c, 1);
        let x = with_this::<EntityStyleObj, _>(c, |s| ty.and_then(|t| s.0.component(t)))?;
        Ok(c.push(x.map_or(Value::Nil, |x| Value::Object(Rc::new(ComponentStyleObj(x))))))
    });
    e.method("isComponentEnabled", |c| {
        let ty = component_arg(c, 1);
        let v = with_this::<EntityStyleObj, _>(c, |s| ty.and_then(|t| s.0.component(t)).is_some_and(|x| x.enabled))?;
        Ok(c.push(v))
    });

    let k = expose_class(thread, COMPONENT_UI_STYLE, None);
    macro_rules! component_getter {
        ($name:literal, $f:expr) => {
            k.method($name, |c| {
                let v = with_this::<ComponentStyleObj, _>(c, |s| $f(&*s.0))?;
                Ok(c.push(v))
            });
        };
    }
    component_getter!("getLuaPanelClass", |s: &ComponentUiStyle| opt_str(&s.lua_panel_class));
    component_getter!("getXuiStyle", |s: &ComponentUiStyle| opt_str(&s.xui_style));
    component_getter!("getDisplayName", |s: &ComponentUiStyle| translated(&s.display_name));
    component_getter!("isEnabled", |s: &ComponentUiStyle| Value::Bool(s.enabled));
    component_getter!("getListOrderZ", |s: &ComponentUiStyle| Value::Num(f64::from(s.list_order_z)));
    k.method("getIcon", |c| {
        let path = with_this::<ComponentStyleObj, _>(c, |s| s.0.icon_path.clone())?;
        let v = icon_value(&path)?;
        Ok(c.push(v))
    });

    let st = expose_class(thread, XUI_LUA_STYLE, None);
    st.method("getXuiLuaClass", |c| {
        let v = with_this::<StyleObj, _>(c, |s| s.0.borrow().lua_class.clone())?;
        Ok(c.push(v))
    });
    st.method("getXuiStyleName", |c| {
        let v = with_this::<StyleObj, _>(c, |s| s.0.borrow().style_name.clone())?;
        Ok(c.push(v))
    });
    st.method("getVar", |c| {
        let key = str_arg(c, 1);
        let v = with_this::<StyleObj, _>(c, |s| {
            let idx = key.as_deref().and_then(|k| s.0.borrow().index.get(k).copied());
            idx.map_or(Value::Nil, |idx| Value::Object(Rc::new(VarObj { style: s.0.clone(), idx })))
        })?;
        Ok(c.push(v))
    });
    st.method("getVars", |c| {
        let items = with_this::<StyleObj, _>(c, |s| {
            let n = s.0.borrow().vars.len();
            (0..n).map(|idx| Value::Object(Rc::new(VarObj { style: s.0.clone(), idx }))).collect::<Vec<_>>()
        })?;
        Ok(c.push(list_value(items)))
    });

    let v = expose_class(thread, XUI_VAR, None);
    macro_rules! var_getter {
        ($name:literal, $f:expr) => {
            v.method($name, |c| {
                let r = with_this::<VarObj, _>(c, |o| o.var($f))?;
                Ok(c.push(r))
            });
        };
    }
    var_getter!("getType", |x: &XuiVar| crate::expose::enum_value(XUI_VAR_TYPE, x.kind.name()));
    var_getter!("getLuaTableKey", |x: &XuiVar| Value::str(&x.key));
    var_getter!("getUiOrder", |x: &XuiVar| Value::Num(f64::from(x.ui_order)));
    var_getter!("getAutoApplyMode", |x: &XuiVar| crate::expose::enum_value(XUI_AUTO_APPLY, AUTO_APPLY[x.auto_apply]));
    var_getter!("isValueSet", |x: &XuiVar| Value::Bool(x.value.is_some()));
    var_getter!("value", var_value);
    var_getter!("getValueString", |x: &XuiVar| Value::str(&match x.value() {
        Val::Null => "null".to_string(),
        Val::Str(s) => s.clone(),
        Val::List(l) => format!("[{}]", l.join(", ")),
        Val::Num(n) => format!("{n:?}"),
        Val::Bool(b) => b.to_string(),
        Val::Font(f) => f.clone(),
        Val::Color(c) => format!("{},{},{},{}", c.r.get(), c.g.get(), c.b.get(), c.a.get()),
    }));
    for (fqn, _) in [
        ("zombie.scripting.ui.XuiLuaStyle$XuiString", ()),
        ("zombie.scripting.ui.XuiLuaStyle$XuiStringList", ()),
        ("zombie.scripting.ui.XuiLuaStyle$XuiTranslateString", ()),
        ("zombie.scripting.ui.XuiLuaStyle$XuiDouble", ()),
        ("zombie.scripting.ui.XuiLuaStyle$XuiBoolean", ()),
        ("zombie.scripting.ui.XuiLuaStyle$XuiFontType", ()),
    ] {
        expose_class(thread, fqn, Some(XUI_VAR));
    }
    let col = expose_class(thread, "zombie.scripting.ui.XuiLuaStyle$XuiColor", Some(XUI_VAR));
    col.method("getR", |c| Ok(c.push(with_this::<VarObj, _>(c, |o| o.var(|x| color_channel(x, |k| k.r.get())))?)));
    col.method("getG", |c| Ok(c.push(with_this::<VarObj, _>(c, |o| o.var(|x| color_channel(x, |k| k.g.get())))?)));
    col.method("getB", |c| Ok(c.push(with_this::<VarObj, _>(c, |o| o.var(|x| color_channel(x, |k| k.b.get())))?)));
    col.method("getA", |c| Ok(c.push(with_this::<VarObj, _>(c, |o| o.var(|x| color_channel(x, |k| k.a.get())))?)));
    let tex = expose_class(thread, "zombie.scripting.ui.XuiLuaStyle$XuiTexture", Some(XUI_VAR));
    tex.method("getTexture", |c| {
        let name = with_this::<VarObj, _>(c, |o| {
            o.var(|x| match x.value() {
                Val::Str(s) => Some(s.clone()),
                _ => None,
            })
        })?;
        let Some(name) = name else { return Ok(c.push(Value::Nil)) };
        let texture = pz_render::get_shared_texture_ref(&name).map_err(texture_error)?;
        Ok(c.push(texture_value_ref(texture.as_ref())))
    });
}

#[cfg(test)]
mod tests;
