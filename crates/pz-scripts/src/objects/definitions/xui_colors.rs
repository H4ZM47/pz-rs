//! `zombie.scripting.objects.XuiColorsScript`.

use super::{parse_float, Color};
use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::ScriptType;

#[derive(Debug)]
pub struct XuiColorsScript {
    pub base: BaseScriptObject,
    name: Option<String>,
    color_map: JHashMap<Color>,
}

impl XuiColorsScript {
    pub fn new() -> Self {
        XuiColorsScript { base: BaseScriptObject::new(ScriptType::XuiColor), name: None, color_map: JHashMap::new() }
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `getColorMap`.
    pub fn color_map(&self) -> &JHashMap<Color> {
        &self.color_map
    }

    pub fn color_map_mut(&mut self) -> &mut JHashMap<Color> {
        &mut self.color_map
    }

    /// `LoadColorsBlock`: `key = r:g:b[:a]` in 0..1, or `key = rgb:r:g:b[:a]`
    /// in 0..255.
    pub(crate) fn load_colors_block(&mut self, block: &Block) -> ScriptResult {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            let mut color = Color::default();
            let split = java::split_char(val, ':');
            if split.len() > 1 && java::eq_ignore_case(java::trim(split[0]), "rgb") {
                for (i, s) in split.iter().enumerate().skip(1) {
                    let v = || -> ScriptResult<f32> { Ok(parse_float(java::trim(s))? / 255.0) };
                    match i {
                        1 => color.r = v()?,
                        2 => color.g = v()?,
                        3 => color.b = v()?,
                        4 => color.a = v()?,
                        _ => {}
                    }
                }
            } else {
                for (i, s) in split.iter().enumerate() {
                    let v = || parse_float(java::trim(s));
                    match i {
                        0 => color.r = v()?,
                        1 => color.g = v()?,
                        2 => color.b = v()?,
                        3 => color.a = v()?,
                        _ => {}
                    }
                }
            }
            self.color_map.insert(key.to_string(), color);
        }
        Ok(())
    }
}

impl Default for XuiColorsScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptObject for XuiColorsScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.XuiColorsScript"
    }

    /// `Load`.
    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        self.name = Some(name.to_string());
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        self.load_colors_block(block)
    }
}
