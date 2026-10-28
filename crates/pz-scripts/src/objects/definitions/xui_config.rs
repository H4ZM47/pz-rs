//! `zombie.scripting.objects.XuiConfigScript`, with `XuiVarType` and
//! `XuiLuaStyle.ALLOWED_VAR_TYPES`.

use super::exception;
use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::ScriptType;

java_enum!(
    /// `zombie.scripting.ui.XuiVarType`.
    XuiVarType,
    "zombie.scripting.ui.XuiVarType",
    [
        Function, String, StringList, TranslateString, Integer, Double, Float, Boolean, FontType, VectorPosAlign,
        TextAlign, Color, Unit, Vector, Texture,
    ]
);

/// `XuiLuaStyle.ALLOWED_VAR_TYPES`, in `EnumSet` (ordinal) order.
pub const ALLOWED_VAR_TYPES: [XuiVarType; 8] = [
    XuiVarType::String,
    XuiVarType::StringList,
    XuiVarType::TranslateString,
    XuiVarType::Double,
    XuiVarType::Boolean,
    XuiVarType::FontType,
    XuiVarType::Color,
    XuiVarType::Texture,
];

/// `varConfigs`. The original is a `HashMap` keyed by enum identity hash,
/// so its iteration order is not stable; this keeps `ALLOWED_VAR_TYPES`
/// order.
#[derive(Debug)]
pub struct XuiConfigScript {
    pub base: BaseScriptObject,
    var_configs: Vec<(XuiVarType, Vec<String>)>,
}

impl XuiConfigScript {
    pub fn new() -> Self {
        XuiConfigScript {
            base: BaseScriptObject::new(ScriptType::XuiConfig),
            var_configs: ALLOWED_VAR_TYPES.iter().map(|&t| (t, Vec::new())).collect(),
        }
    }

    /// `getVarConfigs`.
    pub fn var_configs(&self) -> &[(XuiVarType, Vec<String>)] {
        &self.var_configs
    }

    /// `varConfigs.get(varType)`.
    pub fn var_config(&self, var_type: XuiVarType) -> Option<&[String]> {
        self.var_configs.iter().find(|(t, _)| *t == var_type).map(|(_, l)| l.as_slice())
    }

    /// `LoadVarTypeBlock`.
    fn load_var_type_block(&mut self, sm: &ScriptManager, var_type: XuiVarType, block: &Block) -> ScriptResult {
        for value in &block.values {
            if java::is_null_or_whitespace(Some(&value.string)) {
                continue;
            }
            let key = java::trim(&value.string);
            if self.other_types_contains_key(sm, key, var_type) {
                return Err(exception(format!(
                    "Key '{key}' duplicate from another value block. this var type = {}",
                    var_type.name()
                )));
            }
            let list = &mut self.var_configs.iter_mut().find(|(t, _)| *t == var_type).unwrap().1;
            if !list.iter().any(|k| k == key) {
                list.push(key.to_string());
            }
        }
        Ok(())
    }

    /// `otherTypesContainsKey`.
    fn other_types_contains_key(&self, sm: &ScriptManager, key: &str, ignore_type: XuiVarType) -> bool {
        for (t, list) in &self.var_configs {
            if *t == ignore_type || !list.iter().any(|k| k == key) {
                continue;
            }
            sm.warn(&format!("Duplicate key '{key}' in var type: {}", t.name()));
            return true;
        }
        false
    }
}

impl Default for XuiConfigScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptObject for XuiConfigScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.XuiConfigScript"
    }

    /// `Load`.
    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for child in &block.children {
            let var_type = XuiVarType::value_of(child.type_str())?;
            if !ALLOWED_VAR_TYPES.contains(&var_type) {
                return Err(exception(format!("VarType not allowed: {}", var_type.name())));
            }
            self.load_var_type_block(sm, var_type, child)?;
        }
        Ok(())
    }

    /// `reset`.
    fn reset(&mut self) {
        for (_, list) in &mut self.var_configs {
            list.clear();
        }
    }
}
