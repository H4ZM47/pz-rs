//! `zombie.scripting.objects.ModelScript`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::parser::Block;
use crate::script_type::ScriptType;

use super::{discard_null_or_whitespace, load_vector3, parse_float, try_parse_float, ModelAttachment};

pub const DEFAULT_SHADER_NAME: &str = "basicEffect";

/// An attachment shared between `attachments` and `attachmentById`, as the
/// original's object references are.
pub type AttachmentRef = Rc<RefCell<ModelAttachment>>;

/// `zombie.core.skinnedmodel.advancedanimation.AnimBoneWeight`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimBoneWeight {
    pub bone_name: Option<String>,
    pub weight: f32,
    pub include_descendants: bool,
}

impl AnimBoneWeight {
    /// `new AnimBoneWeight(boneName, weight)`.
    pub fn new(bone_name: &str, weight: f32) -> Self {
        AnimBoneWeight { bone_name: Some(bone_name.to_string()), weight, include_descendants: true }
    }
}

#[derive(Debug)]
pub struct ModelScript {
    pub base: BaseScriptObject,
    pub file_name: Option<String>,
    pub name: Option<String>,
    pub mesh_name: Option<String>,
    pub texture_name: Option<String>,
    pub shader_name: Option<String>,
    pub is_static: bool,
    model_manager_key: Option<String>,
    pub scale: f32,
    pub attachments: Vec<AttachmentRef>,
    pub attachment_by_id: JHashMap<AttachmentRef>,
    pub invert_x: bool,
    pub post_process: Option<String>,
    // TODO(model loading): `loadedModel` (zombie.core.skinnedmodel.model.Model).
    pub bone_weights: Vec<AnimBoneWeight>,
    pub animations_mesh: Option<String>,
    pub cull_face: i32,
}

impl Default for ModelScript {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelScript {
    pub fn new() -> Self {
        ModelScript {
            base: BaseScriptObject::new(ScriptType::Model),
            file_name: None,
            name: None,
            mesh_name: None,
            texture_name: None,
            shader_name: None,
            is_static: true,
            model_manager_key: None,
            scale: 1.0,
            attachments: Vec::new(),
            attachment_by_id: JHashMap::new(),
            invert_x: false,
            post_process: None,
            bone_weights: Vec::new(),
            animations_mesh: None,
            cull_face: -1,
        }
    }

    /// `LoadAttachment`.
    fn load_attachment(&mut self, block: &Block) -> ScriptResult<AttachmentRef> {
        let existing = block.id.as_deref().and_then(|id| self.attachment_by_id(id));
        let attachment = match existing {
            Some(a) => a,
            None => {
                let id = block.id.as_deref().ok_or_else(|| {
                    ScriptError::new("java.lang.NullPointerException: Cannot invoke \"String.intern()\" because \"block.id\" is null")
                })?;
                let a = Rc::new(RefCell::new(ModelAttachment::new(id)?));
                self.attachments.push(a.clone());
                self.attachment_by_id.insert(id.to_string(), a.clone());
                a
            }
        };
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            let mut a = attachment.borrow_mut();
            match k {
                "bone" => a.set_bone(v),
                "offset" => load_vector3(v, a.offset_mut())?,
                "rotate" => load_vector3(v, a.rotate_mut())?,
                "scale" => a.set_scale(try_parse_float(v, 1.0)),
                _ => {}
            }
        }
        Ok(attachment)
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `getFullType`: `module.name + "." + name`.
    pub fn full_type_name(&self) -> String {
        format!(
            "{}.{}",
            self.base.module_name().as_deref().unwrap_or("null"),
            self.name.as_deref().unwrap_or("null")
        )
    }

    /// `getMeshName`.
    pub fn mesh_name(&self) -> Option<&str> {
        self.mesh_name.as_deref()
    }

    /// `getTextureName()`: the mesh name when no texture is set.
    pub fn texture_name(&self) -> Option<&str> {
        self.texture_name_or_null(false)
    }

    /// `getTextureName(allowNull)`.
    pub fn texture_name_or_null(&self, allow_null: bool) -> Option<&str> {
        if java::is_null_or_whitespace(self.texture_name.as_deref()) && !allow_null {
            return self.mesh_name.as_deref();
        }
        self.texture_name.as_deref()
    }

    /// `getShaderName`.
    pub fn shader_name(&self) -> &str {
        match self.shader_name.as_deref() {
            Some(s) if !java::is_null_or_whitespace(Some(s)) => s,
            _ => DEFAULT_SHADER_NAME,
        }
    }

    /// `getModelManagerKey`.
    pub fn model_manager_key(&self) -> Option<&str> {
        self.model_manager_key.as_deref()
    }

    pub fn set_model_manager_key(&mut self, key: Option<String>) {
        self.model_manager_key = key;
    }

    /// `getFileName`.
    pub fn file_name(&self) -> Option<&str> {
        self.file_name.as_deref()
    }

    /// `getAttachmentCount`.
    pub fn attachment_count(&self) -> usize {
        self.attachments.len()
    }

    /// `getAttachment(index)`.
    pub fn attachment(&self, index: usize) -> Option<AttachmentRef> {
        self.attachments.get(index).cloned()
    }

    /// `getAttachmentById(String)`.
    pub fn attachment_by_id(&self, id: &str) -> Option<AttachmentRef> {
        self.attachment_by_id.get(id).cloned()
    }

    /// `addAttachment`.
    pub fn add_attachment(&mut self, attach: AttachmentRef) -> AttachmentRef {
        let id = attach.borrow().id().to_string();
        self.attachments.push(attach.clone());
        self.attachment_by_id.insert(id, attach.clone());
        attach
    }

    /// `addAttachmentAt`.
    pub fn add_attachment_at(&mut self, index: usize, attach: AttachmentRef) -> AttachmentRef {
        let id = attach.borrow().id().to_string();
        self.attachments.insert(index, attach.clone());
        self.attachment_by_id.insert(id, attach.clone());
        attach
    }

    /// `removeAttachment(ModelAttachment)`.
    pub fn remove_attachment(&mut self, attach: &AttachmentRef) -> AttachmentRef {
        if let Some(i) = self.attachments.iter().position(|a| Rc::ptr_eq(a, attach)) {
            self.attachments.remove(i);
        }
        let id = attach.borrow().id().to_string();
        self.attachment_by_id.remove(&id);
        attach.clone()
    }

    /// `removeAttachment(int)`.
    pub fn remove_attachment_at(&mut self, index: usize) -> AttachmentRef {
        let a = self.attachments.remove(index);
        let id = a.borrow().id().to_string();
        self.attachment_by_id.remove(&id);
        a
    }

    /// `ModelAttachment.setId` on an attachment owned by this script
    /// (`beforeRenameAttachment` / `afterRenameAttachment`).
    pub fn rename_attachment(&mut self, attach: &AttachmentRef, id: &str) -> ScriptResult {
        if java::is_null_or_whitespace(Some(id)) {
            return attach.borrow_mut().set_id(id);
        }
        let old = attach.borrow().id().to_string();
        self.attachment_by_id.remove(&old);
        attach.borrow_mut().set_id(id)?;
        self.attachment_by_id.insert(id.to_string(), attach.clone());
        Ok(())
    }

    /// `scaleAttachmentOffset`.
    pub fn scale_attachment_offset(&mut self, scale: f32) {
        for a in &self.attachments {
            a.borrow_mut().offset_mut().mul(scale);
        }
    }

    /// `isStatic`.
    pub fn is_static(&self) -> bool {
        self.is_static
    }
}

impl ScriptObject for ModelScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.ModelScript"
    }

    fn init_load_pp(&mut self, sm: &ScriptManager, name: &str) {
        self.base.init_load_pp(name);
        self.file_name = sm.current_file_name();
        self.name = Some(name.to_string());
    }

    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for child in &block.children {
            if child.type_.as_deref() == Some("attachment") {
                self.load_attachment(child)?;
            }
        }
        let mut undo_core_scale = false;
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            let is = |s: &str| java::eq_ignore_case(k, s);
            if is("mesh") {
                self.mesh_name = Some(v.to_string());
            } else if is("scale") {
                self.scale = parse_float(v)?;
            } else if is("shader") {
                self.shader_name = Some(v.to_string());
            } else if is("static") {
                self.is_static = java::parse_bool(v);
            } else if is("texture") {
                self.texture_name = Some(v.to_string());
            } else if is("invertX") {
                self.invert_x = java::parse_bool(v);
            } else if is("cullFace") {
                if java::eq_ignore_case("Back", v) {
                    self.cull_face = 1029;
                } else if java::eq_ignore_case("Front", v) {
                    self.cull_face = 1028;
                } else if java::eq_ignore_case("None", v) {
                    self.cull_face = 0;
                }
            } else if is("postProcess") {
                self.post_process = Some(v.to_string());
            } else if is("undoCoreScale") {
                undo_core_scale = java::parse_bool(v);
            } else if is("boneWeight") {
                let ss = java::split_ws(v);
                if ss.len() == 2 {
                    let mut w = AnimBoneWeight::new(ss[0], try_parse_float(ss[1], 1.0));
                    w.include_descendants = false;
                    self.bone_weights.push(w);
                }
            } else if is("animationsMesh") {
                self.animations_mesh = discard_null_or_whitespace(v);
            }
        }
        if undo_core_scale {
            self.scale *= 0.6666667;
        }
        Ok(())
    }

    /// `reset`: attachments, `fileName`, `postProcess` and `animationsMesh`
    /// are kept, as in the original.
    fn reset(&mut self) {
        self.invert_x = false;
        self.name = None;
        self.mesh_name = None;
        self.texture_name = None;
        self.shader_name = None;
        self.is_static = true;
        self.model_manager_key = None;
        self.scale = 1.0;
        self.bone_weights.clear();
        self.cull_face = -1;
    }
}

/// The model references of one item that `ModelScript.ScriptsLoaded`
/// checks, read after `Item.resolveModelScripts`.
#[derive(Clone, Debug, Default)]
pub struct ItemModelRefs {
    pub full_name: String,
    pub static_model: Option<String>,
    pub weapon_sprite: Option<String>,
    pub world_static_model: Option<String>,
    pub clothing_item: Option<String>,
    pub is_food: bool,
}

/// `checkMesh`. `file_exists` stands for
/// `ZomboidFileSystem.instance.activeFileMap.containsKey`.
fn check_mesh(sm: &ScriptManager, object: &str, mesh: Option<&str>, file_exists: &dyn Fn(&str) -> bool) {
    let Some(mesh) = mesh.filter(|m| !java::is_null_or_whitespace(Some(m))) else { return };
    let lower = java::to_lower(mesh);
    if !(file_exists(&format!("media/models_x/{lower}.fbx"))
        || file_exists(&format!("media/models_x/{lower}.x"))
        || file_exists(&format!("media/models/{lower}.txt")))
    {
        sm.statics.models.borrow_mut().reported.insert(mesh.to_string());
        sm.script_warn(&format!("no such mesh \"{mesh}\" for {object}"));
    }
}

/// `checkTexture` (client side).
fn check_texture(sm: &ScriptManager, object: &str, texture: Option<&str>, file_exists: &dyn Fn(&str) -> bool) {
    let Some(texture) = texture.filter(|t| !java::is_null_or_whitespace(Some(t))) else { return };
    let lower = java::to_lower(texture);
    if !file_exists(&format!("media/textures/{lower}.png")) {
        sm.statics.models.borrow_mut().reported.insert(texture.to_string());
        sm.script_warn(&format!("no such texture \"{texture}\" for {object}"));
    }
}

/// `check(object, model, clothingItem)`.
fn check(
    sm: &ScriptManager,
    object: &str,
    model: Option<&str>,
    clothing_item: Option<&str>,
    file_exists: &dyn Fn(&str) -> bool,
) {
    let Some(model) = model.filter(|m| !java::is_null_or_whitespace(Some(m))) else { return };
    if sm.statics.models.borrow().reported.contains(model) {
        return;
    }
    match sm.get::<ModelScript>(ScriptType::Model, model) {
        None => {
            sm.statics.models.borrow_mut().reported.insert(model.to_string());
            sm.script_warn(&format!("no such model \"{model}\" for {object}"));
        }
        Some(ms) => {
            let ms = ms.borrow();
            check_mesh(sm, &ms.full_type_name(), ms.mesh_name(), file_exists);
            if java::is_null_or_whitespace(clothing_item) {
                check_texture(sm, &ms.full_type_name(), ms.texture_name(), file_exists);
            }
        }
    }
}

/// `ModelScript.ScriptsLoaded`: warns about item models, meshes and
/// textures that do not exist, and about food models whose
/// Burnt/Cooked/Rotten variants have a different number of attachments.
pub fn scripts_loaded(sm: &ScriptManager, items: &[ItemModelRefs], file_exists: &dyn Fn(&str) -> bool) {
    sm.statics.models.borrow_mut().reported.clear();
    for item in items {
        check(sm, &item.full_name, item.static_model.as_deref(), None, file_exists);
        check(sm, &item.full_name, item.weapon_sprite.as_deref(), None, file_exists);
        check(sm, &item.full_name, item.world_static_model.as_deref(), item.clothing_item.as_deref(), file_exists);
        if !item.is_food {
            continue;
        }
        let Some(static_model) = item.static_model.as_deref().filter(|m| !java::is_null_or_whitespace(Some(m)))
        else {
            continue;
        };
        let Some(ms) = sm.get::<ModelScript>(ScriptType::Model, static_model) else { continue };
        let (count, ms_name) = {
            let ms = ms.borrow();
            (ms.attachment_count(), ms.name.clone())
        };
        if count == 0 {
            continue;
        }
        for suffix in ["Burnt", "Cooked", "Rotten"] {
            let Some(ms2) = sm.get::<ModelScript>(ScriptType::Model, &format!("{static_model}{suffix}")) else {
                continue;
            };
            let ms2 = ms2.borrow();
            check_texture(sm, ms2.name().unwrap_or("null"), ms2.texture_name.as_deref(), file_exists);
            if ms2.attachment_count() != count {
                sm.script_warn(&format!(
                    "different number of attachments on {} and {}",
                    ms_name.as_deref().unwrap_or("null"),
                    ms2.name().unwrap_or("null")
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::load;
    use super::*;

    #[test]
    fn loads_values_and_attachments() {
        let body = "model Box { mesh = Props/Box, texture = box_tex, scale = 0.5, static = FALSE, \
                    cullFace = front, boneWeight = Bip01 0.25, boneWeight = bad, undoCoreScale = true, \
                    attachment world { offset = 1 2 3, rotate = 0 90 0, bone = Root, scale = 0, } \
                    attachment world { bone = , } }";
        let (m, r) = load(ModelScript::new(), "Box", body);
        r.unwrap();
        assert_eq!(m.mesh_name(), Some("Props/Box"));
        assert_eq!(m.texture_name(), Some("box_tex"));
        assert_eq!(m.scale, 0.5 * 0.6666667);
        assert!(!m.is_static);
        assert_eq!(m.cull_face, 1028);
        assert_eq!(m.bone_weights, vec![AnimBoneWeight { bone_name: Some("Bip01".into()), weight: 0.25, include_descendants: false }]);
        assert_eq!(m.attachment_count(), 1);
        let a = m.attachment_by_id("world").unwrap();
        let a = a.borrow();
        assert_eq!(a.offset(), &crate::math::Vector3::new(1.0, 2.0, 3.0));
        assert_eq!(a.bone(), None);
        assert_eq!(a.scale(), 0.01);
        assert_eq!(m.shader_name(), DEFAULT_SHADER_NAME);
    }

    #[test]
    fn bad_numbers_throw() {
        let (_, r) = load(ModelScript::new(), "B", "model B { scale = x, }");
        assert_eq!(r.unwrap_err().0, "java.lang.NumberFormatException: For input string: \"x\"");
        let (_, r) = load(ModelScript::new(), "B", "model B { attachment a { offset = 1 2, } }");
        assert!(r.unwrap_err().0.contains("Index 2 out of bounds for length 2"));
    }

    #[test]
    fn texture_falls_back_to_mesh() {
        let (m, _) = load(ModelScript::new(), "B", "model B { mesh = M, }");
        assert_eq!(m.texture_name(), Some("M"));
        assert_eq!(m.texture_name_or_null(true), None);
    }

    #[test]
    fn reset_keeps_attachments() {
        let (mut m, _) = load(ModelScript::new(), "B", "model B { mesh = M, attachment a { bone = X, } }");
        m.reset();
        assert_eq!(m.mesh_name(), None);
        assert_eq!(m.attachment_count(), 1);
    }
}
