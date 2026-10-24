//! `zombie.scripting.objects.XuiSkinScript` with its nested
//! `EntityUiScript`, `StyleInfoScript` and `ComponentUiScript`, and the
//! `zombie.entity.ComponentType` names it reads.

use super::{array_index_error, exception, parse_int, XuiColorsScript};
use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::ScriptType;

java_enum!(
    /// `zombie.entity.ComponentType` (constant names only).
    ComponentType,
    "zombie.entity.ComponentType",
    [
        Attributes, FluidContainer, SpriteConfig, Lua, Parts, Signals, Script, UiConfig, CraftLogic, FurnaceLogic,
        TestComponent, MashingLogic, DryingLogic, MetaTag, Resources, CraftBench, CraftRecipe, Durability,
        DryingCraftLogic, ContextMenuConfig, SpriteOverlayConfig, CraftBenchSounds, WallCoveringConfig, Undefined,
    ]
);

const PROTECTED_DEFAULT_NAME: &str = "default";

/// `XuiSkinScript.ComponentUiScript`.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentUiScript {
    pub lua_panel_class: Option<String>,
    pub xui_style: Option<String>,
    pub display_name: Option<String>,
    pub icon_path: Option<String>,
    pub list_order_z_set: bool,
    pub list_order_z: i32,
    pub enabled_set: bool,
    pub enabled: bool,
}

impl Default for ComponentUiScript {
    fn default() -> Self {
        ComponentUiScript {
            lua_panel_class: None,
            xui_style: None,
            display_name: None,
            icon_path: None,
            list_order_z_set: false,
            list_order_z: 0,
            enabled_set: false,
            enabled: true,
        }
    }
}

/// `XuiSkinScript.EntityUiScript`. `componentUiScriptMap` is a `HashMap`
/// keyed by enum identity hash in the original, so its order is not stable;
/// this keeps insertion order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityUiScript {
    pub lua_window_class: Option<String>,
    pub xui_style: Option<String>,
    pub lua_can_open_window: Option<String>,
    pub lua_open_window: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub build_description: Option<String>,
    pub icon_path: Option<String>,
    pub clear_components: bool,
    pub component_ui_script_map: Vec<(ComponentType, ComponentUiScript)>,
}

impl EntityUiScript {
    /// `getComponentUiScriptMap().get(type)`.
    pub fn component_ui_script(&self, t: ComponentType) -> Option<&ComponentUiScript> {
        self.component_ui_script_map.iter().find(|(k, _)| *k == t).map(|(_, v)| v)
    }

    /// `get`, inserting a new `ComponentUiScript` when absent.
    fn component_entry(&mut self, t: ComponentType) -> &mut ComponentUiScript {
        let i = match self.component_ui_script_map.iter().position(|(k, _)| *k == t) {
            Some(i) => i,
            None => {
                self.component_ui_script_map.push((t, ComponentUiScript::default()));
                self.component_ui_script_map.len() - 1
            }
        };
        &mut self.component_ui_script_map[i].1
    }

    /// `reset`.
    fn reset(&mut self) {
        *self = EntityUiScript::default();
    }
}

/// `XuiSkinScript.StyleInfoScript`. `None` values are Java `null`s (empty,
/// `nil` or `null` in the script).
#[derive(Clone, Debug, Default)]
pub struct StyleInfoScript {
    pub default_style_block: JHashMap<Option<String>>,
    pub style_blocks: JHashMap<JHashMap<Option<String>>>,
}

#[derive(Debug)]
pub struct XuiSkinScript {
    pub base: BaseScriptObject,
    imports: Vec<String>,
    default_entity_ui_script: EntityUiScript,
    entity_ui_script_map: JHashMap<EntityUiScript>,
    style_info_map: JHashMap<StyleInfoScript>,
    colors_script: XuiColorsScript,
}

impl XuiSkinScript {
    pub fn new() -> Self {
        XuiSkinScript {
            base: BaseScriptObject::new(ScriptType::XuiSkin),
            imports: Vec::new(),
            default_entity_ui_script: EntityUiScript::default(),
            entity_ui_script_map: JHashMap::new(),
            style_info_map: JHashMap::new(),
            colors_script: XuiColorsScript::new(),
        }
    }

    /// `getImports`.
    pub fn imports(&self) -> &[String] {
        &self.imports
    }

    /// `getDefaultEntityUiScript`.
    pub fn default_entity_ui_script(&self) -> &EntityUiScript {
        &self.default_entity_ui_script
    }

    /// `getEntityUiScriptMap`.
    pub fn entity_ui_script_map(&self) -> &JHashMap<EntityUiScript> {
        &self.entity_ui_script_map
    }

    /// `getStyleInfoMap`.
    pub fn style_info_map(&self) -> &JHashMap<StyleInfoScript> {
        &self.style_info_map
    }

    /// `getColorsScript`.
    pub fn colors_script(&self) -> &XuiColorsScript {
        &self.colors_script
    }

    /// `LoadStyleBlock`.
    fn load_style_block(&mut self, block: &Block) -> ScriptResult {
        let type_ = block.type_str();
        if !self.style_info_map.contains_key(type_) {
            self.style_info_map.insert(type_.to_string(), StyleInfoScript::default());
        }
        let style_info = self.style_info_map.get_mut(type_).unwrap();
        let map = match block.id.as_deref() {
            Some(id) if !java::is_null_or_whitespace(Some(id)) => {
                if java::eq_ignore_case(id, PROTECTED_DEFAULT_NAME) {
                    return Err(exception("Default is protected and cannot be used as style name."));
                }
                if id.contains('.') {
                    return Err(exception("Style name may not contain '.' (dot)."));
                }
                if !style_info.style_blocks.contains_key(id) {
                    style_info.style_blocks.insert(id.to_string(), JHashMap::new());
                }
                style_info.style_blocks.get_mut(id).unwrap()
            }
            _ => &mut style_info.default_style_block,
        };
        for value in &block.values {
            let key = java::trim(value.get_key());
            if key.is_empty() {
                continue;
            }
            let val = trim_surrounding_quotes(java::trim(value.get_value()))?;
            let stored = if val.is_empty() || java::eq_ignore_case(val, "nil") || java::eq_ignore_case(val, "null") {
                None
            } else {
                Some(val.to_string())
            };
            map.insert(key.to_string(), stored);
        }
        Ok(())
    }

    /// `LoadImports`.
    fn load_imports(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let s = java::trim(&value.string);
            if java::is_null_or_whitespace(Some(s)) {
                continue;
            }
            if java::eq_ignore_case(s, self.base.script_object_name()) {
                let msg = format!("Cannot import self: {}", self.base.script_object_name());
                sm.warn(&msg);
                if sm.debug {
                    return Err(exception(msg));
                }
            }
            self.imports.push(s.to_string());
        }
        Ok(())
    }

    /// `LoadEntityBlock`.
    fn load_entity_block(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        let entity = match block.id.as_deref() {
            Some(id) if !java::is_null_or_whitespace(Some(id)) => {
                if java::eq_ignore_case(id, PROTECTED_DEFAULT_NAME) {
                    return Err(exception("Default is protected and cannot be used as style name."));
                }
                if !self.entity_ui_script_map.contains_key(id) {
                    self.entity_ui_script_map.insert(id.to_string(), EntityUiScript::default());
                }
                self.entity_ui_script_map.get_mut(id).unwrap()
            }
            _ => &mut self.default_entity_ui_script,
        };
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            if java::eq_ignore_case(key, "luaWindowClass") {
                entity.lua_window_class = Some(val.to_string());
            } else if java::eq_ignore_case(key, "xuiStyle") {
                entity.xui_style = Some(val.to_string());
            } else if java::eq_ignore_case(key, "luaCanOpenWindow") {
                entity.lua_can_open_window = Some(val.to_string());
            } else if java::eq_ignore_case(key, "luaOpenWindow") {
                entity.lua_open_window = Some(val.to_string());
            } else if java::eq_ignore_case(key, "displayName") {
                // TODO(Translator): replaced by Translator.getRecipeName(val
                // without spaces) when that differs ignoring case.
                entity.display_name = Some(val.to_string());
            } else if java::eq_ignore_case(key, "description") {
                entity.description = Some(val.to_string());
            } else if java::eq_ignore_case(key, "buildDescription") {
                entity.build_description = Some(val.to_string());
            } else if java::eq_ignore_case(key, "icon") {
                entity.icon_path = Some(val.to_string());
            } else if java::eq_ignore_case(key, "clearComponents") {
                entity.clear_components = java::eq_ignore_case(val, "true");
            } else {
                sm.warn(&format!("Unknown line in script: {}", value.string));
            }
        }
        for child in &block.children {
            if java::eq_ignore_case("components", child.type_str()) {
                load_components(sm, child, entity)?;
            }
        }
        Ok(())
    }
}

/// `LoadComponents`: `ComponentType = luaClass[:style]` lines and nested
/// per-component blocks.
fn load_components(sm: &ScriptManager, block: &Block, entity: &mut EntityUiScript) -> ScriptResult {
    for value in &block.values {
        let key = java::trim(value.get_key());
        let val = java::trim(value.get_value());
        if key.is_empty() || val.is_empty() {
            continue;
        }
        let component_type = ComponentType::value_of(key)?;
        let mut lua_class = val;
        let mut style = None;
        if val.contains(':') {
            let s = java::split_char(val, ':');
            lua_class = s.first().copied().ok_or_else(|| array_index_error(0, s.len()))?;
            style = Some(*s.get(1).ok_or_else(|| array_index_error(1, s.len()))?);
        }
        let ui_info = entity.component_entry(component_type);
        ui_info.lua_panel_class = Some(lua_class.to_string());
        if let Some(style) = style {
            ui_info.xui_style = Some(style.to_string());
        }
    }
    for child in &block.children {
        load_component_block(sm, child, entity)?;
    }
    Ok(())
}

/// `LoadComponentBlock`.
fn load_component_block(sm: &ScriptManager, block: &Block, entity: &mut EntityUiScript) -> ScriptResult {
    let ui_info = entity.component_entry(ComponentType::value_of(block.type_str())?);
    for value in &block.values {
        let key = java::trim(value.get_key());
        let val = java::trim(value.get_value());
        if key.is_empty() || val.is_empty() {
            continue;
        }
        if java::eq_ignore_case(key, "luaPanelClass") {
            ui_info.lua_panel_class = Some(val.to_string());
        } else if java::eq_ignore_case(key, "xuiStyle") {
            ui_info.xui_style = Some(val.to_string());
        } else if java::eq_ignore_case(key, "displayName") {
            ui_info.display_name = Some(val.to_string());
        } else if java::eq_ignore_case(key, "icon") {
            ui_info.icon_path = Some(val.to_string());
        } else if java::eq_ignore_case(key, "listOrderZ") {
            ui_info.list_order_z = parse_int(val)?;
            ui_info.list_order_z_set = true;
        } else if java::eq_ignore_case(key, "enabled") {
            ui_info.enabled = java::eq_ignore_case(val, "true");
            ui_info.enabled_set = true;
        } else {
            sm.warn(&format!("Unknown line in script: {}", value.string));
        }
    }
    Ok(())
}

/// `StringUtils.trimSurroundingQuotes(s)`: a lone `"` throws, as
/// `substring(1, 0)` does.
fn trim_surrounding_quotes(s: &str) -> ScriptResult<&str> {
    if s.is_empty() {
        return Ok(s);
    }
    let s = java::trim(s);
    if s.starts_with('"') && s.ends_with('"') {
        if s.len() == 1 {
            return Err(ScriptError::new("java.lang.StringIndexOutOfBoundsException: begin 1, end 0, length 1"));
        }
        return Ok(&s[1..s.len() - 1]);
    }
    Ok(s)
}

impl Default for XuiSkinScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptObject for XuiSkinScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.XuiSkinScript"
    }

    /// `Load`.
    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            sm.warn(&format!("Unknown line in script: {}", value.string));
        }
        for child in &block.children {
            let t = child.type_str();
            if java::eq_ignore_case("imports", t) {
                self.load_imports(sm, child)?;
            } else if java::eq_ignore_case("entity", t) {
                self.load_entity_block(sm, child)?;
            } else if java::eq_ignore_case("colors", t) {
                self.colors_script.load_colors_block(child)?;
            } else {
                self.load_style_block(child)?;
            }
        }
        Ok(())
    }

    /// `reset`.
    fn reset(&mut self) {
        self.imports.clear();
        self.default_entity_ui_script.reset();
        self.entity_ui_script_map.clear();
        self.style_info_map.clear();
        self.colors_script.color_map_mut().clear();
    }
}
