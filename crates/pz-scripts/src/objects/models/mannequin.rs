//! `zombie.scripting.objects.MannequinScript`.

use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::script_type::ScriptType;

use super::{discard_null_or_whitespace, try_parse_bool};

#[derive(Debug)]
pub struct MannequinScript {
    pub base: BaseScriptObject,
    name: Option<String>,
    female: bool,
    model_script_name: Option<String>,
    texture: Option<String>,
    anim_set: Option<String>,
    anim_state: Option<String>,
    pose: Option<String>,
    outfit: Option<String>,
}

impl Default for MannequinScript {
    fn default() -> Self {
        Self::new()
    }
}

impl MannequinScript {
    pub fn new() -> Self {
        MannequinScript {
            base: BaseScriptObject::new(ScriptType::Mannequin),
            name: None,
            female: true,
            model_script_name: None,
            texture: None,
            anim_set: None,
            anim_state: None,
            pose: None,
            outfit: None,
        }
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `isFemale`.
    pub fn is_female(&self) -> bool {
        self.female
    }

    pub fn set_female(&mut self, b: bool) {
        self.female = b;
    }

    /// `getModelScriptName`.
    pub fn model_script_name(&self) -> Option<&str> {
        self.model_script_name.as_deref()
    }

    pub fn set_model_script_name(&mut self, s: &str) {
        self.model_script_name = discard_null_or_whitespace(s);
    }

    /// `getTexture`.
    pub fn texture(&self) -> Option<&str> {
        self.texture.as_deref()
    }

    pub fn set_texture(&mut self, s: &str) {
        self.texture = discard_null_or_whitespace(s);
    }

    /// `getAnimSet`.
    pub fn anim_set(&self) -> Option<&str> {
        self.anim_set.as_deref()
    }

    pub fn set_anim_set(&mut self, s: &str) {
        self.anim_set = discard_null_or_whitespace(s);
    }

    /// `getAnimState`.
    pub fn anim_state(&self) -> Option<&str> {
        self.anim_state.as_deref()
    }

    pub fn set_anim_state(&mut self, s: &str) {
        self.anim_state = discard_null_or_whitespace(s);
    }

    /// `getPose`.
    pub fn pose(&self) -> Option<&str> {
        self.pose.as_deref()
    }

    pub fn set_pose(&mut self, s: &str) {
        self.pose = discard_null_or_whitespace(s);
    }

    /// `getOutfit`.
    pub fn outfit(&self) -> Option<&str> {
        self.outfit.as_deref()
    }

    pub fn set_outfit(&mut self, s: &str) {
        self.outfit = discard_null_or_whitespace(s);
    }
}

impl ScriptObject for MannequinScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.MannequinScript"
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        self.name = Some(name.to_string());
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            let is = |s: &str| java::eq_ignore_case(k, s);
            if is("female") {
                self.female = try_parse_bool(v);
            } else if is("model") {
                self.model_script_name = discard_null_or_whitespace(v);
            } else if is("texture") {
                self.texture = discard_null_or_whitespace(v);
            } else if is("animSet") {
                self.anim_set = discard_null_or_whitespace(v);
            } else if is("animState") {
                self.anim_state = discard_null_or_whitespace(v);
            } else if is("pose") {
                self.pose = discard_null_or_whitespace(v);
            } else if is("outfit") {
                self.outfit = discard_null_or_whitespace(v);
            }
        }
        Ok(())
    }

    /// `reset`: `female` and `name` are kept.
    fn reset(&mut self) {
        self.model_script_name = None;
        self.texture = None;
        self.anim_set = None;
        self.anim_state = None;
        self.pose = None;
        self.outfit = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::load;
    use super::*;

    #[test]
    fn loads_fields() {
        let (m, r) = load(MannequinScript::new(), "M1", "mannequin M1 { female = false, model = FemaleBody, pose = , }");
        r.unwrap();
        assert_eq!(m.name(), Some("M1"));
        assert!(!m.is_female());
        assert_eq!(m.model_script_name(), Some("FemaleBody"));
        assert_eq!(m.pose(), None);
    }
}
