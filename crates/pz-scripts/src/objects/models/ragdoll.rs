//! `zombie.scripting.objects.RagdollScript` and its data classes
//! (`RagdollConstraint`, `RagdollAnchor`, `RagdollBodyPartInfo`,
//! `RagdollBodyDynamics`). Loaded entries go into static lists
//! (`sm.statics.models`), which the original never clears.

use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::math::Vector3;
use crate::parser::Block;
use crate::script_type::ScriptType;

use super::{load_vector3, parse_float, parse_int, split_key_value, Statics};

pub const BONE_NAMES: [&str; 35] = [
    "Dummy01", "Bip01", "Bip01_Pelvis", "Bip01_Spine", "Bip01_Spine1", "Bip01_Neck", "Bip01_Head",
    "Bip01_L_Clavicle", "Bip01_L_UpperArm", "Bip01_L_Forearm", "Bip01_L_Hand", "Bip01_L_Finger0",
    "Bip01_L_Finger1", "Bip01_R_Clavicle", "Bip01_R_UpperArm", "Bip01_R_Forearm", "Bip01_R_Hand",
    "Bip01_R_Finger0", "Bip01_R_Finger1", "Bip01_BackPack", "Bip01_L_Thigh", "Bip01_L_Calf", "Bip01_L_Foot",
    "Bip01_L_Toe0", "Bip01_R_Thigh", "Bip01_R_Calf", "Bip01_R_Foot", "Bip01_R_Toe0", "Bip01_DressFront",
    "Bip01_DressFront02", "Bip01_DressBack", "Bip01_DressBack02", "Bip01_Prop1", "Bip01_Prop2",
    "Translation_Data",
];
pub const NUMBER_OF_CONSTRAINT_ATTRIBUTES: usize = 22;
pub const NUMBER_OF_ANCHOR_ATTRIBUTES: usize = 5;
pub const NUMBER_OF_BODY_PART_INFO: usize = 10;
pub const NUMBER_OF_BODY_DYNAMICS: usize = 8;

/// `RagdollConstraint`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RagdollConstraint {
    pub joint: i32,
    pub constraint_type: i32,
    pub constraint_part_a: i32,
    pub constraint_part_b: i32,
    pub constraint_axis_a: Vector3,
    pub constraint_axis_b: Vector3,
    pub constraint_position_offset_a: Vector3,
    pub constraint_position_offset_b: Vector3,
    pub constraint_limit: Vector3,
    pub constraint_limit_extended: Vector3,
    pub default_constraint_axis_a: Vector3,
    pub default_constraint_axis_b: Vector3,
    pub default_constraint_position_offset_a: Vector3,
    pub default_constraint_position_offset_b: Vector3,
    pub default_constraint_limit: Vector3,
    pub default_constraint_limit_extended: Vector3,
}

/// `RagdollAnchor`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RagdollAnchor {
    pub bone: i32,
    pub body_part: i32,
    pub reverse: bool,
    pub original: bool,
    pub enabled: bool,
    pub default_body_part: i32,
    pub default_reverse: bool,
    pub default_original: bool,
    pub default_enabled: bool,
}

/// `RagdollBodyPartInfo`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RagdollBodyPartInfo {
    pub part: i32,
    pub calculate_length: bool,
    pub radius: f32,
    pub height: f32,
    pub gap: f32,
    pub shape: i32,
    pub mass: f32,
    pub offset: Vector3,
    pub default_calculate_length: bool,
    pub default_radius: f32,
    pub default_height: f32,
    pub default_gap: f32,
    pub default_shape: i32,
    pub default_mass: f32,
    pub default_offset: Vector3,
}

/// `RagdollBodyDynamics`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RagdollBodyDynamics {
    pub part: i32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub deactivation_time: f32,
    pub linear_sleeping_threshold: f32,
    pub angular_sleeping_threshold: f32,
    pub friction: f32,
    pub rolling_friction: f32,
    pub default_linear_damping: f32,
    pub default_angular_damping: f32,
    pub default_deactivation_time: f32,
    pub default_linear_sleeping_threshold: f32,
    pub default_angular_sleeping_threshold: f32,
    pub default_friction: f32,
    pub default_rolling_friction: f32,
}

#[derive(Debug)]
pub struct RagdollScript {
    pub base: BaseScriptObject,
}

impl Default for RagdollScript {
    fn default() -> Self {
        Self::new()
    }
}

fn load_constraint(block: &Block) -> ScriptResult<RagdollConstraint> {
    let mut c = RagdollConstraint::default();
    for value in &block.values {
        let (k, v) = split_key_value(&value.string)?;
        match k {
            "joint" => c.joint = parse_int(v)?,
            "constraintType" => c.constraint_type = parse_int(v)?,
            "constraintPartA" => c.constraint_part_a = parse_int(v)?,
            "constraintPartB" => c.constraint_part_b = parse_int(v)?,
            "constraintAxisA" => {
                load_vector3(v, &mut c.constraint_axis_a)?;
                load_vector3(v, &mut c.default_constraint_axis_a)?;
            }
            "constraintAxisB" => {
                load_vector3(v, &mut c.constraint_axis_b)?;
                load_vector3(v, &mut c.default_constraint_axis_b)?;
            }
            "constraintPositionOffsetA" => {
                load_vector3(v, &mut c.constraint_position_offset_a)?;
                load_vector3(v, &mut c.default_constraint_position_offset_a)?;
            }
            "constraintPositionOffsetB" => {
                load_vector3(v, &mut c.constraint_position_offset_b)?;
                load_vector3(v, &mut c.default_constraint_position_offset_b)?;
            }
            "constraintLimit" => {
                load_vector3(v, &mut c.constraint_limit)?;
                load_vector3(v, &mut c.default_constraint_limit)?;
            }
            "constraintLimitExtended" => {
                load_vector3(v, &mut c.constraint_limit_extended)?;
                load_vector3(v, &mut c.default_constraint_limit_extended)?;
            }
            _ => {}
        }
    }
    Ok(c)
}

fn load_anchor(block: &Block) -> ScriptResult<RagdollAnchor> {
    let mut a = RagdollAnchor::default();
    for value in &block.values {
        let (k, v) = split_key_value(&value.string)?;
        match k {
            "bone" => a.bone = parse_int(v)?,
            "bodyPart" => {
                a.body_part = parse_int(v)?;
                a.default_body_part = a.body_part;
            }
            "enabled" => {
                a.enabled = java::parse_bool(v);
                a.default_enabled = a.enabled;
            }
            "reverse" => {
                a.reverse = java::parse_bool(v);
                a.default_reverse = a.reverse;
            }
            "original" => {
                a.original = java::parse_bool(v);
                a.default_original = a.original;
            }
            _ => {}
        }
    }
    Ok(a)
}

fn load_body_part_info(block: &Block) -> ScriptResult<RagdollBodyPartInfo> {
    let mut p = RagdollBodyPartInfo::default();
    for value in &block.values {
        let (k, v) = split_key_value(&value.string)?;
        match k {
            "part" => p.part = parse_int(v)?,
            "calculateLength" => {
                p.default_calculate_length = java::parse_bool(v);
                p.calculate_length = p.default_calculate_length;
            }
            "radius" => {
                p.default_radius = parse_float(v)?;
                p.radius = p.default_radius;
            }
            "height" => {
                p.default_height = parse_float(v)?;
                p.height = p.default_height;
            }
            "gap" => {
                p.default_gap = parse_float(v)?;
                p.gap = p.default_gap;
            }
            "shape" => {
                p.default_shape = parse_int(v)?;
                p.shape = p.default_shape;
            }
            "mass" => {
                p.default_mass = parse_float(v)?;
                p.mass = p.default_mass;
            }
            "offset" => {
                load_vector3(v, &mut p.offset)?;
                load_vector3(v, &mut p.default_offset)?;
            }
            _ => {}
        }
    }
    Ok(p)
}

/// `loadBodyDynamics`.
fn load_body_dynamics(block: &Block) -> ScriptResult<RagdollBodyDynamics> {
    let mut d = RagdollBodyDynamics::default();
    for value in &block.values {
        let (k, v) = split_key_value(&value.string)?;
        let (cur, def) = match k {
            "part" => {
                d.part = parse_int(v)?;
                continue;
            }
            "linearDamping" => (&mut d.linear_damping, &mut d.default_linear_damping),
            "angularDamping" => (&mut d.angular_damping, &mut d.default_angular_damping),
            "deactivationTime" => (&mut d.deactivation_time, &mut d.default_deactivation_time),
            "linearSleepingThreshold" => (&mut d.linear_sleeping_threshold, &mut d.default_linear_sleeping_threshold),
            "angularSleepingThreshold" => {
                (&mut d.angular_sleeping_threshold, &mut d.default_angular_sleeping_threshold)
            }
            "friction" => (&mut d.friction, &mut d.default_friction),
            "rollingFriction" => (&mut d.rolling_friction, &mut d.default_rolling_friction),
            _ => continue,
        };
        *def = parse_float(v)?;
        *cur = *def;
    }
    Ok(d)
}

fn b(v: bool) -> f32 {
    if v {
        1.0
    } else {
        0.0
    }
}

impl RagdollScript {
    pub fn new() -> Self {
        RagdollScript { base: BaseScriptObject::new(ScriptType::Ragdoll) }
    }

    /// `toBullet`: the four upload calls. Run from the Ragdoll collection's
    /// `PostLoadScripts` (routed through `ScriptHooks::post_load_scripts`).
    pub fn to_bullet(statics: &Statics, live_update: bool) {
        // TODO(bullet): skipped on a dedicated server; pass each array to
        // `Bullet.defineRagdoll*`.
        let _ = (
            Self::body_dynamics_params(statics),
            Self::constraint_params(statics),
            Self::anchor_params(statics),
            Self::body_part_info_params(statics),
            live_update,
        );
    }

    /// `resetConstraintsToDefaultValues` (without the Bullet upload).
    pub fn reset_constraints_to_default_values(statics: &mut Statics) {
        for c in &mut statics.ragdoll_constraints {
            c.constraint_axis_a = c.default_constraint_axis_a;
            c.constraint_axis_b = c.default_constraint_axis_b;
            c.constraint_position_offset_a = c.default_constraint_position_offset_a;
            c.constraint_position_offset_b = c.default_constraint_position_offset_b;
            c.constraint_limit = c.default_constraint_limit;
            c.constraint_limit_extended = c.default_constraint_limit_extended;
        }
        // TODO(bullet): uploadConstraints(true).
    }

    /// `resetAnchorsToDefaultValues` (without the Bullet upload).
    pub fn reset_anchors_to_default_values(statics: &mut Statics) {
        for a in &mut statics.ragdoll_anchors {
            a.body_part = a.default_body_part;
            a.reverse = a.default_reverse;
            a.enabled = a.default_enabled;
        }
        // TODO(bullet): uploadAnchors(true).
    }

    /// `resetBodyPartInfoToDefaultValues` (without the Bullet upload).
    pub fn reset_body_part_info_to_default_values(statics: &mut Statics) {
        for p in &mut statics.ragdoll_body_part_info {
            p.calculate_length = p.default_calculate_length;
            p.radius = p.default_radius;
            p.height = p.default_height;
            p.gap = p.default_gap;
            p.shape = p.default_shape;
            p.mass = p.default_mass;
            p.offset = p.default_offset;
        }
        // TODO(bullet): uploadBodyPartInfo(true).
    }

    /// `resetBodyDynamicsToDefaultValues` (without the Bullet upload).
    pub fn reset_body_dynamics_to_default_values(statics: &mut Statics) {
        for d in &mut statics.ragdoll_body_dynamics {
            d.linear_damping = d.default_linear_damping;
            d.angular_damping = d.default_angular_damping;
            d.deactivation_time = d.default_deactivation_time;
            d.linear_sleeping_threshold = d.default_linear_sleeping_threshold;
            d.angular_sleeping_threshold = d.default_angular_sleeping_threshold;
            d.friction = d.default_friction;
            d.rolling_friction = d.default_rolling_friction;
        }
        // TODO(bullet): uploadBodyDynamics(true).
    }

    /// The array `uploadConstraints` passes to Bullet.
    pub fn constraint_params(statics: &Statics) -> Vec<f32> {
        let mut p = Vec::with_capacity(NUMBER_OF_CONSTRAINT_ATTRIBUTES * statics.ragdoll_constraints.len());
        for c in &statics.ragdoll_constraints {
            p.extend([c.joint as f32, c.constraint_type as f32, c.constraint_part_a as f32, c.constraint_part_b as f32]);
            for v in [
                c.constraint_axis_a,
                c.constraint_axis_b,
                c.constraint_position_offset_a,
                c.constraint_position_offset_b,
                c.constraint_limit,
                c.constraint_limit_extended,
            ] {
                p.extend([v.x, v.y, v.z]);
            }
        }
        p
    }

    /// The array `uploadBodyPartInfo` passes to Bullet.
    pub fn body_part_info_params(statics: &Statics) -> Vec<f32> {
        let mut p = Vec::with_capacity(NUMBER_OF_BODY_PART_INFO * statics.ragdoll_body_part_info.len());
        for i in &statics.ragdoll_body_part_info {
            p.extend([
                i.part as f32,
                b(i.calculate_length),
                i.radius,
                i.height,
                i.gap,
                i.shape as f32,
                i.mass,
                i.offset.x,
                i.offset.y,
                i.offset.z,
            ]);
        }
        p
    }

    /// The array `uploadAnchors` passes to Bullet.
    pub fn anchor_params(statics: &Statics) -> Vec<f32> {
        let mut p = Vec::with_capacity(NUMBER_OF_ANCHOR_ATTRIBUTES * statics.ragdoll_anchors.len());
        for a in &statics.ragdoll_anchors {
            p.extend([a.bone as f32, a.body_part as f32, b(a.reverse), b(a.original), b(a.enabled)]);
        }
        p
    }

    /// The array `uploadBodyDynamics` passes to Bullet.
    pub fn body_dynamics_params(statics: &Statics) -> Vec<f32> {
        let mut p = Vec::with_capacity(NUMBER_OF_BODY_DYNAMICS * statics.ragdoll_body_dynamics.len());
        for d in &statics.ragdoll_body_dynamics {
            p.extend([
                d.part as f32,
                d.linear_damping,
                d.angular_damping,
                d.deactivation_time,
                d.linear_sleeping_threshold,
                d.angular_sleeping_threshold,
                d.friction,
                d.rolling_friction,
            ]);
        }
        p
    }

    // TODO(debug tools): `write*ToFile` (ragdoll editor output) need the
    // `RagdollJoint`/`RagdollBodyPart` enums.
}

impl ScriptObject for RagdollScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.RagdollScript"
    }

    fn init_load_pp(&mut self, sm: &ScriptManager, name: &str) {
        self.base.init_load_pp(name);
        let Some(file) = sm.current_file_name() else { return };
        let mut st = sm.statics.models.borrow_mut();
        if file.contains("_constraints") {
            st.original_constraint_filename = Some(file);
        } else if file.contains("_anchors") {
            st.original_anchor_filename = Some(file);
        } else if file.contains("_bodypartinfo") {
            st.original_body_part_info_filename = Some(file);
        } else if file.contains("_bodydynamics") {
            st.original_body_dynamics_filename = Some(file);
        }
    }

    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        let id = block.id.as_deref().ok_or_else(|| {
            ScriptError::new("java.lang.NullPointerException: Cannot invoke \"String.contains(java.lang.CharSequence)\" because \"block.id\" is null")
        })?;
        let mut st = sm.statics.models.borrow_mut();
        if id.contains("_constraint") {
            st.ragdoll_constraints.push(load_constraint(block)?);
        } else if id.contains("_anchor") {
            st.ragdoll_anchors.push(load_anchor(block)?);
        } else if id.contains("_bodypartinfo") {
            st.ragdoll_body_part_info.push(load_body_part_info(block)?);
        } else if id.contains("_bodydynamics") {
            st.ragdoll_body_dynamics.push(load_body_dynamics(block)?);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_into_static_lists() {
        let sm = ScriptManager::new();
        let mut r = RagdollScript::new();
        r.load(&sm, "x", "ragdoll Neck_constraint { joint = 2, constraintAxisA = 0 1 0, constraintLimit = 1 2 3, }")
            .unwrap();
        r.load(&sm, "x", "ragdoll Head_anchor { bone = 6, bodyPart = 3, enabled = true, }").unwrap();
        r.load(&sm, "x", "ragdoll Head_bodydynamics { part = 1, friction = 0.5, }").unwrap();
        r.load(&sm, "x", "ragdoll Head_bodypartinfo { part = 1, mass = 2, offset = 0 0 1, }").unwrap();
        let st = sm.statics.models.borrow();
        assert_eq!(st.ragdoll_constraints[0].joint, 2);
        assert_eq!(st.ragdoll_constraints[0].default_constraint_axis_a, Vector3::new(0.0, 1.0, 0.0));
        assert_eq!(RagdollScript::anchor_params(&st), vec![6.0, 3.0, 0.0, 0.0, 1.0]);
        assert_eq!(st.ragdoll_body_dynamics[0].default_friction, 0.5);
        assert_eq!(RagdollScript::body_part_info_params(&st).len(), NUMBER_OF_BODY_PART_INFO);
        assert_eq!(RagdollScript::constraint_params(&st).len(), NUMBER_OF_CONSTRAINT_ATTRIBUTES);
    }

    #[test]
    fn malformed_values_throw() {
        let sm = ScriptManager::new();
        let mut r = RagdollScript::new();
        assert!(r.load(&sm, "x", "ragdoll A_anchor { bone = x, }").is_err());
        assert!(r.load(&sm, "x", "ragdoll A_anchor { bone, }").is_err());
    }
}
