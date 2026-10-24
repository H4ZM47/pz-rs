//! `zombie.iso.SpriteModel`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptRef, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::math::Vector3;
use crate::parser::Block;
use crate::script_type::ScriptType;

use super::{at, discard_null_or_whitespace, parse_vector3_lenient, try_parse_float, ModelAttachment, ModelScript};

#[derive(Debug)]
pub struct SpriteModel {
    pub base: BaseScriptObject,
    pub model_script_name: Option<String>,
    pub texture_name: Option<String>,
    pub translate: Vector3,
    pub rotate: Vector3,
    pub scale: f32,
    pub animation_name: Option<String>,
    pub animation_time: f32,
    pub runtime_string: Option<String>,
}

impl Default for SpriteModel {
    fn default() -> Self {
        Self::new()
    }
}

fn required<'a>(block: &'a Block, key: &str) -> ScriptResult<&'a str> {
    block.get_value(key).map(|v| v.get_value()).ok_or_else(|| {
        ScriptError::new(
            "java.lang.NullPointerException: Cannot invoke \"zombie.scripting.ScriptParser$Value.getValue()\" \
             because the return value of \"zombie.scripting.ScriptParser$Block.getValue(String)\" is null",
        )
    })
}

fn invalid(runtime_string: &str) -> ScriptError {
    ScriptError::new(format!("java.lang.RuntimeException: invalid SpriteModel runtime string \"{runtime_string}\""))
}

impl SpriteModel {
    pub fn new() -> Self {
        Self::with_type(ScriptType::SpriteModel)
    }

    /// `SpriteModel(ScriptType)`, for subclasses.
    pub fn with_type(type_: ScriptType) -> Self {
        SpriteModel {
            base: BaseScriptObject::new(type_),
            model_script_name: None,
            texture_name: None,
            translate: Vector3::default(),
            rotate: Vector3::default(),
            scale: 1.0,
            animation_name: None,
            animation_time: -1.0,
            runtime_string: None,
        }
    }

    /// `set(other)`: copies everything but the runtime string.
    pub fn set(&mut self, other: &SpriteModel) -> &mut Self {
        self.model_script_name = other.model_script_name.clone();
        self.texture_name = other.texture_name.clone();
        self.translate = other.translate;
        self.rotate = other.rotate;
        self.scale = other.scale;
        self.animation_name = other.animation_name.clone();
        self.animation_time = other.animation_time;
        self
    }

    /// `getModelScriptName`.
    pub fn model_script_name(&self) -> Option<&str> {
        self.model_script_name.as_deref()
    }

    pub fn set_model_script_name(&mut self, s: Option<String>) {
        self.model_script_name = s;
    }

    /// `getTextureName`.
    pub fn texture_name(&self) -> Option<&str> {
        self.texture_name.as_deref()
    }

    pub fn set_texture_name(&mut self, s: &str) {
        self.texture_name = discard_null_or_whitespace(s);
    }

    /// `getAnimationName`.
    pub fn animation_name(&self) -> Option<&str> {
        self.animation_name.as_deref()
    }

    /// `getRuntimeString`.
    pub fn runtime_string(&self) -> Option<&str> {
        self.runtime_string.as_deref()
    }

    pub fn set_runtime_string(&mut self, s: &str) {
        self.runtime_string = discard_null_or_whitespace(s);
    }

    fn set_door(&mut self, tileset: &str, tile_index: i32, translate: Vector3, rotate: Vector3, animation: &str) {
        self.model_script_name = Some(format!("Base.{tileset}_{tile_index}"));
        self.translate = translate;
        self.rotate = rotate;
        self.scale = 0.6666667;
        self.animation_name = Some(animation.to_string());
        self.animation_time = 0.0;
    }

    /// `parseRuntimeString`: door sprites (`standard_door`/`pair_door`)
    /// whose models are generated at runtime.
    pub fn parse_runtime_string(
        &mut self,
        sm: &ScriptManager,
        tileset: &str,
        tile_column: i32,
        tile_row: i32,
        runtime_string: Option<&str>,
    ) -> ScriptResult {
        let Some(rs) = runtime_string else { return Ok(()) };
        if rs.contains("standard_door") {
            self.parse_standard_door(sm, tileset, tile_column, tile_row, rs)
        } else if rs.contains("pair_door") {
            self.parse_pair_door(sm, tileset, tile_column, tile_row, rs)
        } else {
            Ok(())
        }
    }

    /// `parseStandardDoor`.
    fn parse_standard_door(
        &mut self,
        sm: &ScriptManager,
        tileset: &str,
        tile_column: i32,
        tile_row: i32,
        rs: &str,
    ) -> ScriptResult {
        let ss = java::split_char(java::trim(rs), ' ');
        if !java::eq_ignore_case("standard_door", at(&ss, 0)?) {
            return Err(ScriptError::new(format!(
                "java.lang.RuntimeException: expected \"standard_door\" but got \"{rs}\""
            )));
        }
        let edge = at(&ss, 1)?;
        let state = at(&ss, 2)?;
        let tile_index = tile_column + tile_row * 8;
        let first_door = tile_column / 4 * 4;
        let animation = match state {
            "closed" => "Open",
            "open" => "Close",
            _ => return Err(invalid(rs)),
        };
        let texture_index = first_door + 2 + tile_row * 8;
        let mesh = match edge {
            "w" => {
                self.set_door(tileset, tile_index, Vector3::new(-0.469, 0.0, -0.375), Vector3::new(0.0, 90.0, 0.0), animation);
                "IsoObject/door_w_se.glb"
            }
            "n" => {
                self.set_door(tileset, tile_index, Vector3::new(-0.383, 0.0, -0.445), Vector3::new(0.0, 0.0, 0.0), animation);
                "IsoObject/door_n_sw.glb"
            }
            _ => return Err(invalid(rs)),
        };
        self.runtime_string = Some(java::trim(rs).to_string());
        let ms = self.create_door_model_script_if_needed(sm, tileset, texture_index, mesh)?;
        let mut ms = ms.borrow_mut();
        let (o1, r1, o2, r2) = match edge {
            "n" => (
                Vector3::new(-0.3997, 1.8943, 0.0),
                Vector3::new(180.0, 0.0, 180.0),
                Vector3::new(-0.3997, 1.8943, -0.0581),
                Vector3::default(),
            ),
            _ => (
                Vector3::new(0.3997, 1.8943, 0.0),
                Vector3::new(180.0, 0.0, 180.0),
                Vector3::new(0.3997, 1.8943, -0.0585),
                Vector3::default(),
            ),
        };
        set_curtain(&mut ms, "curtain1", o1, r1)?;
        set_curtain(&mut ms, "curtain2", o2, r2)
    }

    /// `parsePairDoor`.
    fn parse_pair_door(
        &mut self,
        sm: &ScriptManager,
        tileset: &str,
        tile_column: i32,
        tile_row: i32,
        rs: &str,
    ) -> ScriptResult {
        let ss = java::split_char(java::trim(rs), ' ');
        let first = at(&ss, 0)?;
        if !java::eq_ignore_case("pair_door", first) {
            return Err(ScriptError::new(format!("java.lang.RuntimeException: expected \"pair_door\" but got \"{first}\"")));
        }
        let left_right = at(&ss, 1)?;
        let edge = at(&ss, 2)?;
        let state = at(&ss, 3)?;
        let tile_index = tile_column + tile_row * 8;
        let left = java::eq_ignore_case("left", left_right);
        let right = java::eq_ignore_case("right", left_right);
        let closed = match state {
            "closed" => true,
            "open" => false,
            _ => return Err(invalid(rs)),
        };
        let animation = if closed { "Open" } else { "Close" };
        let row = tile_row * 8;
        let (texture_index, translate, rotate, mesh) = match (edge, left, right) {
            ("w", true, _) => (
                if closed { 4 + row } else { 2 + row },
                Vector3::new(-0.492, 0.0, -0.1),
                Vector3::new(0.0, 90.0, 0.0),
                if closed { "IsoObject/door_pair_left_w.glb" } else { "IsoObject/door_pair_left_w_open.glb" },
            ),
            ("w", false, true) => {
                (7 + row, Vector3::new(-0.49, 0.0, 0.1), Vector3::new(0.0, 90.0, 0.0), "IsoObject/door_pair_right_w.glb")
            }
            ("n", true, _) => (
                if closed { tile_column + 1 + row } else { tile_column - 1 + row },
                Vector3::new(0.1, 0.0, -0.47),
                Vector3::default(),
                "IsoObject/door_pair_left_n.glb",
            ),
            ("n", false, true) => (
                if closed { 5 + row } else { 7 + row },
                Vector3::new(-0.117, 0.0, -0.47),
                Vector3::default(),
                if closed { "IsoObject/door_pair_right_n.glb" } else { "IsoObject/door_pair_right_n_open.glb" },
            ),
            _ => return Err(invalid(rs)),
        };
        self.set_door(tileset, tile_index, translate, rotate, animation);
        self.runtime_string = Some(java::trim(rs).to_string());
        let ms = self.create_door_model_script_if_needed(sm, tileset, texture_index, mesh)?;
        let mut ms = ms.borrow_mut();
        let n = java::eq_ignore_case("n", edge);
        let w = java::eq_ignore_case("w", edge);
        let flip = Vector3::new(180.0, 0.0, 180.0);
        let c1 = if n && left {
            Some(Vector3::new(0.0, 1.8943, 0.027928859))
        } else if n && right {
            Some(Vector3::new(-0.0, 1.8943, 0.028792206))
        } else if w && (left || right) {
            Some(Vector3::new(0.0, 1.8943, 0.02863244))
        } else {
            None
        };
        set_curtain_opt(&mut ms, "curtain1", c1.map(|o| (o, flip)))?;
        let c2 = if n && (left || right) {
            Some(Vector3::new(0.0, 1.8943, -0.027928859))
        } else if w && (left || right) {
            Some(Vector3::new(0.0, 1.8943, -0.029316932))
        } else {
            None
        };
        set_curtain_opt(&mut ms, "curtain2", c2.map(|o| (o, Vector3::default())))
    }

    /// `createDoorModelScriptIfNeeded`.
    fn create_door_model_script_if_needed(
        &self,
        sm: &ScriptManager,
        tileset: &str,
        texture_index: i32,
        mesh: &str,
    ) -> ScriptResult<Rc<RefCell<ModelScript>>> {
        let full = self.model_script_name.clone().unwrap_or_default();
        let short = full.get("Base.".len()..).unwrap_or("").to_string();
        let ms = match sm.get::<ModelScript>(ScriptType::Model, &full) {
            Some(ms) => ms,
            None => {
                let mut ms = ModelScript::new();
                if let Some(m) = sm.get_module("Base") {
                    ms.set_module(&m);
                }
                ms.init_load_pp(sm, &short);
                let r = ScriptRef::new(ms);
                sm.add_runtime_script(r.clone())?;
                r.downcast::<ModelScript>().expect("ModelScript")
            }
        };
        {
            let mut m = ms.borrow_mut();
            // TODO(fs): `ZomboidFileSystem.getMediaPath` makes this absolute.
            m.file_name = Some("media/scripts/models_runtime.txt".to_string());
            m.name = Some(short);
            m.mesh_name = Some(mesh.to_string());
            m.texture_name = Some(format!("{tileset}_{texture_index}"));
            m.scale = if mesh.ends_with(".glb") { 1.0 } else { 0.01 };
            m.shader_name = Some("door".to_string());
            m.is_static = false;
            let p1 = mesh.rfind('/').map_or(0, |p| p + 1);
            let p2 = mesh.rfind('.').unwrap_or(mesh.len());
            m.animations_mesh = Some(mesh.get(p1..p2).unwrap_or("").to_string());
            // TODO(model loading): `loadedModel = null`.
        }
        Ok(ms)
    }
}

fn set_curtain(ms: &mut ModelScript, id: &str, offset: Vector3, rotate: Vector3) -> ScriptResult {
    set_curtain_opt(ms, id, Some((offset, rotate)))
}

/// Finds or adds the attachment, sets its bone to `DoorBone`, then its
/// offset and rotation when given.
fn set_curtain_opt(ms: &mut ModelScript, id: &str, place: Option<(Vector3, Vector3)>) -> ScriptResult {
    let a = match ms.attachment_by_id(id) {
        Some(a) => a,
        None => ms.add_attachment(Rc::new(RefCell::new(ModelAttachment::new(id)?))),
    };
    let mut a = a.borrow_mut();
    a.set_bone("DoorBone");
    if let Some((o, r)) = place {
        *a.offset_mut() = o;
        *a.rotate_mut() = r;
    }
    Ok(())
}

impl ScriptObject for SpriteModel {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.iso.SpriteModel"
    }

    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        self.model_script_name = Some(java::trim(required(block, "modelScript")?).to_string());
        if let Some(v) = block.get_value("texture") {
            self.texture_name = discard_null_or_whitespace(java::trim(v.get_value()));
        }
        parse_vector3_lenient(java::trim(required(block, "translate")?), &mut self.translate)?;
        parse_vector3_lenient(java::trim(required(block, "rotate")?), &mut self.rotate)?;
        self.scale = try_parse_float(java::trim(required(block, "scale")?), 1.0);
        if let Some(v) = block.get_value("animation") {
            self.animation_name = discard_null_or_whitespace(java::trim(v.get_value()));
        }
        if let Some(v) = block.get_value("animationTime") {
            self.animation_time = try_parse_float(v.get_value(), -1.0);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::load;
    use super::*;

    #[test]
    fn loads_required_values() {
        let body = "spriteModel s_0 { modelScript = Base.Thing, texture = , translate = 0 0.5 0, rotate = 0 90 0, scale = 2, animation = Idle, animationTime = 0.5, }";
        let (s, r) = load(SpriteModel::new(), "s_0", body);
        r.unwrap();
        assert_eq!(s.model_script_name(), Some("Base.Thing"));
        assert_eq!(s.texture_name(), None);
        assert_eq!(s.translate, Vector3::new(0.0, 0.5, 0.0));
        assert_eq!(s.scale, 2.0);
        assert_eq!(s.animation_name(), Some("Idle"));
        assert_eq!(s.animation_time, 0.5);
    }

    #[test]
    fn missing_value_is_npe() {
        let (_, r) = load(SpriteModel::new(), "s", "spriteModel s { modelScript = X, }");
        assert!(r.unwrap_err().0.starts_with("java.lang.NullPointerException"));
    }

    #[test]
    fn runtime_door_string() {
        let sm = ScriptManager::new();
        let mut s = SpriteModel::new();
        assert!(s.parse_runtime_string(&sm, "doors_01", 1, 0, Some("standard_door x closed")).is_err());
        assert!(s.parse_runtime_string(&sm, "doors_01", 1, 0, Some("pair_door left n ajar")).is_err());
        s.parse_runtime_string(&sm, "doors_01", 1, 0, Some("nothing")).unwrap();
        assert_eq!(s.model_script_name(), None);
    }
}
