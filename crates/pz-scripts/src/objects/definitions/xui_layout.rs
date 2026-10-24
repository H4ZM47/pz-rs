//! `zombie.scripting.objects.XuiLayoutScript` (used for `xuiLayout`,
//! `xuiStyle` and `xuiDefaultStyle`), with `XuiScriptType`.

use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::{self, Block};
use crate::script_type::ScriptType;

/// `zombie.scripting.ui.XuiScriptType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum XuiScriptType {
    Layout,
    Style,
    DefaultStyle,
    Reference,
}

/// `XuiScript.ReadLuaClassValue`: the first non-empty `xuiLuaClass` value.
pub fn read_lua_class_value(block: &Block) -> Option<&str> {
    block.values.iter().find_map(|value| {
        let key = java::trim(value.get_key());
        let val = java::trim(value.get_value());
        (!key.is_empty() && !val.is_empty() && java::eq_ignore_case(key, "xuiLuaClass")).then_some(val)
    })
}

/// Holds the raw body until `XuiManager.ParseScripts` turns it into an
/// `XuiScript` via `preParse` and `parseScript`.
#[derive(Debug)]
pub struct XuiLayoutScript {
    pub base: BaseScriptObject,
    name: Option<String>,
    total_file: Option<String>,
    has_parsed: bool,
    script_type: XuiScriptType,
    block: Option<Block>,
    lua_class: Option<String>,
}

impl XuiLayoutScript {
    pub fn new(script_type: ScriptType, xui_script_type: XuiScriptType) -> Self {
        XuiLayoutScript {
            base: BaseScriptObject::new(script_type),
            name: None,
            total_file: None,
            has_parsed: false,
            script_type: xui_script_type,
            block: None,
            lua_class: None,
        }
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `getScriptType`.
    pub fn script_type(&self) -> XuiScriptType {
        self.script_type
    }

    /// The body passed to `Load`.
    pub fn total_file(&self) -> Option<&str> {
        self.total_file.as_deref()
    }

    pub fn has_parsed(&self) -> bool {
        self.has_parsed
    }

    /// The `xuiLuaClass` that `preParse` read, which it passes to
    /// `XuiScript.CreateScriptForClass(name, luaClass, true, scriptType)`.
    pub fn lua_class(&self) -> Option<&str> {
        self.lua_class.as_deref()
    }

    /// `preParse`: parses the stored body and reads its lua class. Creating
    /// the `XuiScript` (`getXuiScript`) is the UI system's part.
    /// TODO(XuiManager)
    pub fn pre_parse(&mut self) -> ScriptResult {
        let mut root = parser::parse(self.total_file.as_deref().unwrap_or(""));
        if root.children.is_empty() {
            return Err(index_error());
        }
        let block = root.children.swap_remove(0);
        self.lua_class = read_lua_class_value(&block).map(str::to_string);
        self.block = Some(block);
        Ok(())
    }

    /// `parseScript`: once only, hands over the block `preParse` stored for
    /// `XuiScript.Load`. TODO(XuiManager)
    pub fn parse_script(&mut self) -> Option<Block> {
        if self.has_parsed {
            return None;
        }
        self.has_parsed = true;
        self.block.take()
    }
}

impl ScriptObject for XuiLayoutScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.XuiLayoutScript"
    }

    /// `Load`.
    fn load(&mut self, _sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        self.has_parsed = false;
        self.total_file = Some(body.to_string());
        self.name = Some(name.to_string());
        self.base.load_common_block_str(body)
    }
}
