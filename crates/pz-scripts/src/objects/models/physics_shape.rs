//! `zombie.scripting.objects.PhysicsShapeScript`.

use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::math::Vector3;
use crate::script_type::ScriptType;

use super::{parse_float, parse_vector3_lenient};

#[derive(Debug)]
pub struct PhysicsShapeScript {
    pub base: BaseScriptObject,
    pub mesh_name: Option<String>,
    pub translate: Vector3,
    pub rotate: Vector3,
    pub scale: f32,
    pub post_process: Option<String>,
    pub all_meshes: bool,
}

impl Default for PhysicsShapeScript {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsShapeScript {
    pub fn new() -> Self {
        PhysicsShapeScript {
            base: BaseScriptObject::new(ScriptType::PhysicsShape),
            mesh_name: None,
            translate: Vector3::default(),
            rotate: Vector3::default(),
            scale: 1.0,
            post_process: None,
            all_meshes: false,
        }
    }
}

impl ScriptObject for PhysicsShapeScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.PhysicsShapeScript"
    }

    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        let mut undo_core_scale = false;
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            let is = |s: &str| java::eq_ignore_case(k, s);
            if is("mesh") {
                self.mesh_name = Some(v.to_string());
            } else if is("translate") {
                parse_vector3_lenient(v, &mut self.translate)?;
            } else if is("rotate") {
                parse_vector3_lenient(v, &mut self.rotate)?;
            } else if is("scale") {
                self.scale = parse_float(v)?;
            } else if is("allMeshes") {
                self.all_meshes = java::parse_bool(v);
            } else if is("postProcess") {
                self.post_process = Some(v.to_string());
            } else if is("undoCoreScale") {
                undo_core_scale = java::parse_bool(v);
            }
        }
        if undo_core_scale {
            self.scale *= 0.6666667;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::load;
    use super::*;

    #[test]
    fn loads_vectors_leniently() {
        let (s, r) = load(PhysicsShapeScript::new(), "S", "physicsShape S { mesh = M, translate = 1 x 3, scale = 3, }");
        r.unwrap();
        assert_eq!(s.translate, Vector3::new(1.0, 0.0, 3.0));
        assert_eq!(s.scale, 3.0);
        let (_, r) = load(PhysicsShapeScript::new(), "S", "physicsShape S { rotate = 1, }");
        assert!(r.is_err());
    }
}
