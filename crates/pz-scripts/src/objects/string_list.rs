//! `zombie.scripting.objects.StringListScript`.

use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::script_type::ScriptType;

#[derive(Debug)]
pub struct StringListScript {
    pub base: BaseScriptObject,
    values: Vec<String>,
}

impl StringListScript {
    pub fn new() -> Self {
        StringListScript { base: BaseScriptObject::new(ScriptType::StringList), values: Vec::new() }
    }

    /// `getValues`.
    pub fn values(&self) -> &[String] {
        &self.values
    }
}

impl Default for StringListScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptObject for StringListScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.StringListScript"
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for value in &block.values {
            if java::is_null_or_whitespace(Some(&value.string)) {
                continue;
            }
            let s = java::trim(&value.string);
            if self.values.iter().any(|v| v == s) {
                sm.warn(&format!("Stringlist <{name}> double string entry: {s}"));
                continue;
            }
            self.values.push(s.to_string());
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.values.clear();
    }

    fn pre_reload(&mut self) {
        self.values.clear();
    }
}
