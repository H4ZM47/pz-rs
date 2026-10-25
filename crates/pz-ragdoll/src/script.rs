//! `RagdollScript`: the `ragdoll` script blocks (`media/scripts/ragdolls`)
//! and the float arrays `toBullet` uploads to the physics library.

use pz_bullet::PzBullet;

/// `RagdollConstraint`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RagdollConstraint {
    pub joint: i32,
    pub constraint_type: i32,
    pub part_a: i32,
    pub part_b: i32,
    pub axis_a: [f32; 3],
    pub axis_b: [f32; 3],
    pub position_offset_a: [f32; 3],
    pub position_offset_b: [f32; 3],
    pub limit: [f32; 3],
    pub limit_extended: [f32; 3],
}

/// `RagdollAnchor`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RagdollAnchor {
    pub bone: i32,
    pub body_part: i32,
    pub reverse: bool,
    pub original: bool,
    pub enabled: bool,
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
    pub offset: [f32; 3],
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
}

/// The four lists `RagdollScript` keeps (static in the original), in load
/// order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RagdollScripts {
    pub constraints: Vec<RagdollConstraint>,
    pub anchors: Vec<RagdollAnchor>,
    pub body_part_infos: Vec<RagdollBodyPartInfo>,
    pub body_dynamics: Vec<RagdollBodyDynamics>,
}

/// `Integer.parseInt` (0 on error, where the original would throw and
/// skip the block).
fn int(v: &str) -> i32 {
    v.parse().unwrap_or(0)
}

fn float(v: &str) -> f32 {
    v.parse().unwrap_or(0.0)
}

/// `Boolean.parseBoolean`.
fn boolean(v: &str) -> bool {
    v.eq_ignore_ascii_case("true")
}

/// `BaseScriptObject.LoadVector3`.
fn vec3(v: &str) -> [f32; 3] {
    let mut it = v.split(' ');
    let mut f = || float(it.next().unwrap_or(""));
    [f(), f(), f()]
}

impl RagdollScripts {
    /// `ScriptManager` loading one script file: every `ragdoll` block of
    /// every module goes through `RagdollScript.Load`, in file order.
    pub fn load_file(&mut self, text: &str) {
        let root = pz_scripts::parser::parse(text);
        for module in &root.children {
            for block in &module.children {
                if block.type_.as_deref() == Some("ragdoll") {
                    self.load_block(block);
                }
            }
        }
    }

    /// `RagdollScript.Load`: the block's id picks the list.
    pub fn load_block(&mut self, block: &pz_scripts::parser::Block) {
        let id = block.id.as_deref().unwrap_or("");
        let kv: Vec<(&str, &str)> = block
            .values
            .iter()
            .filter_map(|v| {
                let mut ss = v.string.split('=');
                let k = ss.next()?.trim();
                let val = ss.next()?.trim();
                Some((k, val))
            })
            .collect();
        if id.contains("_constraint") {
            let mut c = RagdollConstraint::default();
            for (k, v) in kv {
                match k {
                    "joint" => c.joint = int(v),
                    "constraintType" => c.constraint_type = int(v),
                    "constraintPartA" => c.part_a = int(v),
                    "constraintPartB" => c.part_b = int(v),
                    "constraintAxisA" => c.axis_a = vec3(v),
                    "constraintAxisB" => c.axis_b = vec3(v),
                    "constraintPositionOffsetA" => c.position_offset_a = vec3(v),
                    "constraintPositionOffsetB" => c.position_offset_b = vec3(v),
                    "constraintLimit" => c.limit = vec3(v),
                    "constraintLimitExtended" => c.limit_extended = vec3(v),
                    _ => {}
                }
            }
            self.constraints.push(c);
        } else if id.contains("_anchor") {
            let mut a = RagdollAnchor::default();
            for (k, v) in kv {
                match k {
                    "bone" => a.bone = int(v),
                    "bodyPart" => a.body_part = int(v),
                    "enabled" => a.enabled = boolean(v),
                    "reverse" => a.reverse = boolean(v),
                    "original" => a.original = boolean(v),
                    _ => {}
                }
            }
            self.anchors.push(a);
        } else if id.contains("_bodypartinfo") {
            let mut p = RagdollBodyPartInfo::default();
            for (k, v) in kv {
                match k {
                    "part" => p.part = int(v),
                    "calculateLength" => p.calculate_length = boolean(v),
                    "radius" => p.radius = float(v),
                    "height" => p.height = float(v),
                    "gap" => p.gap = float(v),
                    "shape" => p.shape = int(v),
                    "mass" => p.mass = float(v),
                    "offset" => p.offset = vec3(v),
                    _ => {}
                }
            }
            self.body_part_infos.push(p);
        } else if id.contains("_bodydynamics") {
            let mut d = RagdollBodyDynamics::default();
            for (k, v) in kv {
                match k {
                    "part" => d.part = int(v),
                    "linearDamping" => d.linear_damping = float(v),
                    "angularDamping" => d.angular_damping = float(v),
                    "deactivationTime" => d.deactivation_time = float(v),
                    "linearSleepingThreshold" => d.linear_sleeping_threshold = float(v),
                    "angularSleepingThreshold" => d.angular_sleeping_threshold = float(v),
                    "friction" => d.friction = float(v),
                    "rollingFriction" => d.rolling_friction = float(v),
                    _ => {}
                }
            }
            self.body_dynamics.push(d);
        }
    }

    /// `uploadConstraints`' array: 22 floats per joint.
    pub fn constraint_params(&self) -> Vec<f32> {
        let mut p = Vec::with_capacity(22 * self.constraints.len());
        for c in &self.constraints {
            p.extend([
                c.joint as f32,
                c.constraint_type as f32,
                c.part_a as f32,
                c.part_b as f32,
            ]);
            for v in [
                c.axis_a,
                c.axis_b,
                c.position_offset_a,
                c.position_offset_b,
                c.limit,
                c.limit_extended,
            ] {
                p.extend(v);
            }
        }
        p
    }

    /// `uploadBodyPartInfo`'s array: 10 floats per part.
    pub fn body_part_info_params(&self) -> Vec<f32> {
        let mut p = Vec::with_capacity(10 * self.body_part_infos.len());
        for b in &self.body_part_infos {
            p.extend([
                b.part as f32,
                if b.calculate_length { 1.0 } else { 0.0 },
                b.radius,
                b.height,
                b.gap,
                b.shape as f32,
                b.mass,
            ]);
            p.extend(b.offset);
        }
        p
    }

    /// `uploadAnchors`' array: 5 floats per bone.
    pub fn anchor_params(&self) -> Vec<f32> {
        let f = |b: bool| if b { 1.0 } else { 0.0 };
        let mut p = Vec::with_capacity(5 * self.anchors.len());
        for a in &self.anchors {
            p.extend([
                a.bone as f32,
                a.body_part as f32,
                f(a.reverse),
                f(a.original),
                f(a.enabled),
            ]);
        }
        p
    }

    /// `uploadBodyDynamics`' array: 8 floats per part.
    pub fn body_dynamics_params(&self) -> Vec<f32> {
        let mut p = Vec::with_capacity(8 * self.body_dynamics.len());
        for d in &self.body_dynamics {
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

    /// `RagdollScript.toBullet(liveUpdate)`: dynamics, constraints, anchors,
    /// then body parts.
    pub fn to_bullet(&self, bullet: &mut PzBullet, live_update: bool) {
        // The arrays are never null, so the natives cannot fail.
        let _ =
            bullet.define_ragdoll_body_dynamics(Some(&self.body_dynamics_params()), live_update);
        let _ = bullet.define_ragdoll_constraints(Some(&self.constraint_params()), live_update);
        let _ = bullet.define_ragdoll_anchors(Some(&self.anchor_params()), live_update);
        let _ =
            bullet.define_ragdoll_body_part_info(Some(&self.body_part_info_params()), live_update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocks_into_upload_arrays() {
        let mut s = RagdollScripts::default();
        s.load_file(
            "module Base\n{\n  ragdoll JOINT_X_constraint\n  {\n    joint = 3,\n    constraintType = 4,\n    constraintPartA = 1,\n    constraintPartB = 2,\n    constraintAxisA = 0.0 1.5 0.0,\n    constraintLimit = 0.5 0.25 0.125,\n  }\n  ragdoll Bip01_anchor\n  {\n    bone = 1,\n    bodyPart = 0,\n    reverse = false,\n    original = true,\n    enabled = true,\n  }\n}\n",
        );
        let c = s.constraint_params();
        assert_eq!(c.len(), 22);
        assert_eq!(&c[..6], &[3.0, 4.0, 1.0, 2.0, 0.0, 1.5]);
        assert_eq!(&c[16..19], &[0.5, 0.25, 0.125]);
        assert_eq!(s.anchor_params(), vec![1.0, 0.0, 0.0, 1.0, 1.0]);
    }
}
