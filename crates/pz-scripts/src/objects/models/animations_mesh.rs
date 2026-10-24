//! `zombie.scripting.objects.AnimationsMesh`.

use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::script_type::ScriptType;

use super::try_parse_bool;

#[derive(Debug)]
pub struct AnimationsMesh {
    pub base: BaseScriptObject,
    pub mesh_file: Option<String>,
    pub keep_mesh_animations: bool,
    pub post_process: Option<String>,
    pub animation_directories: Vec<String>,
    pub animation_prefixes: Vec<String>,
    // TODO(model loading): `modelMesh` (ModelMesh), cleared by `reset`.
}

impl Default for AnimationsMesh {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimationsMesh {
    pub fn new() -> Self {
        AnimationsMesh {
            base: BaseScriptObject::new(ScriptType::AnimationMesh),
            mesh_file: None,
            keep_mesh_animations: false,
            post_process: None,
            animation_directories: Vec::new(),
            animation_prefixes: Vec::new(),
        }
    }
}

impl ScriptObject for AnimationsMesh {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.AnimationsMesh"
    }

    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            if java::eq_ignore_case(k, "meshFile") {
                self.mesh_file = Some(v.to_string());
            } else if java::eq_ignore_case(k, "animationDirectory") {
                self.animation_directories.push(v.to_string());
            } else if java::eq_ignore_case(k, "animationPrefix") {
                self.animation_prefixes.push(v.to_string());
            } else if java::eq_ignore_case(k, "keepMeshAnimations") {
                self.keep_mesh_animations = try_parse_bool(v);
            } else if java::eq_ignore_case(k, "postProcess") {
                self.post_process = Some(v.to_string());
            }
        }
        Ok(())
    }

    /// `reset`: `keepMeshAnimations` and `postProcess` are kept.
    fn reset(&mut self) {
        self.mesh_file = None;
        self.animation_directories.clear();
        self.animation_prefixes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::load;
    use super::*;

    #[test]
    fn loads_lists() {
        let (m, r) = load(
            AnimationsMesh::new(),
            "Human",
            "animationsMesh Human { meshFile = Body, animationDirectory = a, animationDirectory = b, keepMeshAnimations = 1, }",
        );
        r.unwrap();
        assert_eq!(m.mesh_file.as_deref(), Some("Body"));
        assert_eq!(m.animation_directories, vec!["a", "b"]);
        assert!(m.keep_mesh_animations);
    }
}
