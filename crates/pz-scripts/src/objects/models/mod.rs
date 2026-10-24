//! Script classes of the `models` group: models, animation meshes,
//! mannequins, physics shapes, sounds, clocks, ragdolls, sprite models,
//! runtime animations and vehicle engine RPM tables.

pub mod animations_mesh;
pub mod clock;
pub mod game_sound;
pub mod mannequin;
pub mod model_attachment;
pub mod model_script;
pub mod physics_shape;
pub mod ragdoll;
pub mod runtime_animation;
pub mod sound_timeline;
pub mod sprite_model;
pub mod vehicle_engine_rpm;

use std::collections::HashSet;
use std::rc::Rc;

pub use animations_mesh::AnimationsMesh;
pub use clock::{ClockScript, HandScript};
pub use game_sound::{GameSound, GameSoundClip, GameSoundScript, MasterVolume};
pub use mannequin::MannequinScript;
pub use model_attachment::ModelAttachment;
pub use model_script::{AnimBoneWeight, ItemModelRefs, ModelScript};
pub use physics_shape::PhysicsShapeScript;
pub use ragdoll::{RagdollAnchor, RagdollBodyDynamics, RagdollBodyPartInfo, RagdollConstraint, RagdollScript};
pub use runtime_animation::{CopyFrame, CopyFrames, RuntimeAnimationCommand, RuntimeAnimationScript};
pub use sound_timeline::SoundTimelineScript;
pub use sprite_model::SpriteModel;
pub use vehicle_engine_rpm::{EngineRPMData, VehicleEngineRPM};

use crate::base::{ScriptError, ScriptRef, ScriptResult};
use crate::java;
use crate::math::Vector3;
use crate::module::ScriptModule;
use crate::script_type::ScriptType;

/// `createInstance` for the script types this group ports; `None` for
/// types it does not (yet) handle.
pub fn create_instance(type_: ScriptType, _module: &Rc<ScriptModule>, _name: &str, _token: &str) -> Option<ScriptRef> {
    Some(match type_ {
        ScriptType::Model => ScriptRef::new(ModelScript::new()),
        ScriptType::AnimationMesh => ScriptRef::new(AnimationsMesh::new()),
        ScriptType::Mannequin => ScriptRef::new(MannequinScript::new()),
        ScriptType::PhysicsShape => ScriptRef::new(PhysicsShapeScript::new()),
        ScriptType::Sound => ScriptRef::new(GameSoundScript::new()),
        ScriptType::SoundTimeline => ScriptRef::new(SoundTimelineScript::new()),
        ScriptType::Clock => ScriptRef::new(ClockScript::new()),
        ScriptType::Ragdoll => ScriptRef::new(RagdollScript::new()),
        ScriptType::SpriteModel => ScriptRef::new(SpriteModel::new()),
        ScriptType::RuntimeAnimation => ScriptRef::new(RuntimeAnimationScript::new()),
        ScriptType::VehicleEngineRPM => ScriptRef::new(VehicleEngineRPM::new()),
        _ => return None,
    })
}

/// Java static fields of this group's classes (reset by `ScriptManager.Reset`).
#[derive(Default)]
pub struct Statics {
    /// `ModelScript.reported`.
    pub reported: HashSet<String>,
    /// `RagdollScript.ragdollConstraintList`.
    pub ragdoll_constraints: Vec<RagdollConstraint>,
    /// `RagdollScript.ragdollAnchorList`.
    pub ragdoll_anchors: Vec<RagdollAnchor>,
    /// `RagdollScript.ragdollBodyPartInfoList`.
    pub ragdoll_body_part_info: Vec<RagdollBodyPartInfo>,
    /// `RagdollScript.ragdollBodyDynamicsList`.
    pub ragdoll_body_dynamics: Vec<RagdollBodyDynamics>,
    pub original_constraint_filename: Option<String>,
    pub original_anchor_filename: Option<String>,
    pub original_body_part_info_filename: Option<String>,
    pub original_body_dynamics_filename: Option<String>,
}

/// Group steps run at the end of `ScriptManager.Load`. `ModelScript.ScriptsLoaded`
/// needs every item's model references, so the items group calls it.
pub fn post_load(_sm: &crate::manager::ScriptManager) {}

/// `Float.parseFloat` with the original's exception text.
pub(crate) fn parse_float(s: &str) -> ScriptResult<f32> {
    java::parse_f32(s).ok_or_else(|| {
        if java::trim(s).is_empty() {
            ScriptError::new("java.lang.NumberFormatException: empty String")
        } else {
            ScriptError::new(format!("java.lang.NumberFormatException: For input string: \"{s}\""))
        }
    })
}

/// `Integer.parseInt` with the original's exception text.
pub(crate) fn parse_int(s: &str) -> ScriptResult<i32> {
    java::parse_i32(s)
        .ok_or_else(|| ScriptError::new(format!("java.lang.NumberFormatException: For input string: \"{s}\"")))
}

/// `PZMath.tryParseFloat`.
pub(crate) fn try_parse_float(s: &str, default: f32) -> f32 {
    if java::is_null_or_whitespace(Some(s)) {
        return default;
    }
    java::parse_f32(java::trim(s)).unwrap_or(default)
}

/// `PZMath.tryParseInt`.
pub(crate) fn try_parse_int(s: &str, default: i32) -> i32 {
    if java::is_null_or_whitespace(Some(s)) {
        return default;
    }
    java::parse_i32(java::trim(s)).unwrap_or(default)
}

/// `StringUtils.tryParseBoolean`.
pub(crate) fn try_parse_bool(s: &str) -> bool {
    if java::is_null_or_whitespace(Some(s)) {
        return false;
    }
    let t = java::trim(s);
    java::eq_ignore_case(t, "true") || t == "1" || t == "1.0"
}

/// `StringUtils.discardNullOrWhitespace`.
pub(crate) fn discard_null_or_whitespace(s: &str) -> Option<String> {
    (!java::is_null_or_whitespace(Some(s))).then(|| s.to_string())
}

/// `array[index]` on a `String[]`, with Java's bounds exception.
pub(crate) fn at<'a>(arr: &[&'a str], index: usize) -> ScriptResult<&'a str> {
    arr.get(index).copied().ok_or_else(|| {
        ScriptError::new(format!(
            "java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {}",
            arr.len()
        ))
    })
}

/// `String[] ss = s.split("="); k = ss[0].trim(); v = ss[1].trim();`
pub(crate) fn split_key_value(s: &str) -> ScriptResult<(&str, &str)> {
    let ss = java::split_char(s, '=');
    Ok((java::trim(at(&ss, 0)?), java::trim(at(&ss, 1)?)))
}

/// `BaseScriptObject.LoadVector3` / `ModelScript.LoadVector3f`: `split(" ")`
/// then `Float.parseFloat` on each component.
pub(crate) fn load_vector3(s: &str, v: &mut Vector3) -> ScriptResult {
    let ss = java::split_char(s, ' ');
    let x = parse_float(at(&ss, 0)?)?;
    let y = parse_float(at(&ss, 1)?)?;
    let z = parse_float(at(&ss, 2)?)?;
    v.set(x, y, z);
    Ok(())
}

/// `parseVector3f` of `PhysicsShapeScript`/`SpriteModel`: `split(" ")` then
/// `tryParseFloat(.., 0)`, set component by component.
pub(crate) fn parse_vector3_lenient(s: &str, v: &mut Vector3) -> ScriptResult {
    let ss = java::split_char(s, ' ');
    v.x = try_parse_float(at(&ss, 0)?, 0.0);
    v.y = try_parse_float(at(&ss, 1)?, 0.0);
    v.z = try_parse_float(at(&ss, 2)?, 0.0);
    Ok(())
}

/// `PZMath.max(float, float)`: `a > b ? a : b`.
pub(crate) fn pz_max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use crate::base::ScriptObject;
    use crate::manager::ScriptManager;

    /// Loads `body` into a fresh script, returning the script and the result.
    pub fn load<T: ScriptObject>(mut s: T, name: &str, body: &str) -> (T, crate::base::ScriptResult) {
        let sm = ScriptManager::new();
        s.init_load_pp(&sm, name);
        let r = s.load(&sm, name, body);
        (s, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_errors_carry_java_text() {
        assert_eq!(parse_float(" ").unwrap_err().0, "java.lang.NumberFormatException: empty String");
        assert_eq!(parse_int("x").unwrap_err().0, "java.lang.NumberFormatException: For input string: \"x\"");
        assert_eq!(
            split_key_value("abc").unwrap_err().0,
            "java.lang.ArrayIndexOutOfBoundsException: Index 1 out of bounds for length 1"
        );
        assert_eq!(split_key_value(" a = b = c").unwrap(), ("a", "b"));
    }

    #[test]
    fn try_parse_helpers() {
        assert_eq!(try_parse_float(" 2.5 ", 1.0), 2.5);
        assert_eq!(try_parse_float("bad", 1.0), 1.0);
        assert_eq!(try_parse_int("", 7), 7);
        assert!(try_parse_bool(" 1.0"));
        assert!(!try_parse_bool("yes"));
        assert_eq!(pz_max(f32::NAN, 0.01), 0.01);
    }
}
