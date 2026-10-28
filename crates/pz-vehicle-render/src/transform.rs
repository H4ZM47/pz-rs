//! `BaseVehicle.updateTransform`: the body's render transform, the
//! vehicle transform the part models hang from, and each part model's
//! transform (wheels with suspension, steering and spin; models attached
//! to a parent part's model; the rest at their script offsets).

use pz_iso_render::joml::Matrix4f;
use pz_vehicles::jmath::Quaternionf;
use pz_vehicles::BaseVehicle;

use crate::joml::{attachment_transform, rotation, translation_rotate_scale};
use crate::models::{VehicleModel, VehicleModelSet};

const DEG: f32 = std::f32::consts::PI / 180.0;

fn from_mat4(m: &pz_model::Mat4) -> Matrix4f {
    Matrix4f::from_cols(*m)
}

/// `ModelInstanceRenderData.makeBoneTransform`: the bone's model-space
/// transform, identity without a player or bone.
pub fn bone_transform(player: Option<&pz_anim::AnimationPlayer>, bone: Option<&str>) -> Matrix4f {
    let (Some(p), Some(b)) = (player, bone) else {
        return Matrix4f::identity();
    };
    if b.trim().is_empty() {
        return Matrix4f::identity();
    }
    let i = p.skinning_bone_index(b, -1);
    if i < 0 {
        return Matrix4f::identity();
    }
    from_mat4(&p.model_transform(i as usize).to_mat4())
}

/// `ModelInstanceRenderData.preMultiplyMeshTransform`.
pub fn pre_multiply_mesh_transform(t: &Matrix4f, model: &VehicleModel) -> Matrix4f {
    match &model.mesh_transform {
        Some(m) => from_mat4(m).mul(t),
        None => *t,
    }
}

/// `ModelInstanceRenderData.postMultiplyMeshTransform`.
pub fn post_multiply_mesh_transform(t: &Matrix4f, model: &VehicleModel) -> Matrix4f {
    match &model.mesh_transform {
        Some(m) => t.mul(&from_mat4(m)),
        None => *t,
    }
}

/// `BaseVehicle.initTransform(parentInstance, parentScript, script,
/// attachmentNameParent, attachmentNameSelf)`.
fn init_transform(
    v: &BaseVehicle,
    parent: &VehicleModel,
    parent_player: Option<&pz_anim::AnimationPlayer>,
    me: Option<&VehicleModel>,
    parent_name: &str,
    self_name: Option<&str>,
) -> Matrix4f {
    let mut t = Matrix4f::identity();
    let pa = parent.attachment(parent_name).cloned().or_else(|| {
        let s = v.script()?;
        let s = s.borrow();
        let a = s.attachment_by_id(Some(parent_name))?;
        let (o, r) = (a.offset(), a.rotate());
        Some(crate::models::Attachment {
            id: a.id().to_string(),
            offset: [o.x, o.y, o.z],
            rotate: [r.x, r.y, r.z],
            scale: a.scale(),
            bone: a.bone().map(str::to_string),
        })
    });
    if let Some(a) = pa {
        t = bone_transform(parent_player, a.bone.as_deref());
        t = t.scale1(1.0 / parent.scale);
        t = t.mul(&attachment_transform(a.offset, a.rotate, a.scale));
    }
    if let (Some(me), Some(name)) = (me, self_name) {
        if let Some(a) = me.attachment(name) {
            // ModelInstanceRenderData.invertAttachmentSelfTransform is off.
            t = t.mul(&attachment_transform(a.offset, a.rotate, a.scale));
        }
    }
    t
}

impl VehicleModelSet {
    /// `BaseVehicle.updateTransform`.
    pub fn update_transform(&mut self, v: &BaseVehicle) {
        let Some(script) = v.script() else { return };
        let s = script.borrow();
        let scale = s.model_scale();
        let scale2 = if self.instance.scale != 1.0 {
            self.instance.scale
        } else {
            1.0
        };
        let mut chassis = v.world_transform().rotation();
        chassis.y *= -1.0;
        chassis.z *= -1.0;
        let chassis_m = rotation(chassis);
        let invert_x = if self.body.as_ref().is_some_and(|b| b.invert_x) {
            -1.0
        } else {
            1.0
        };
        let (off, rot) = match s.model() {
            Some(m) => (
                [m.offset.x, m.offset.y, m.offset.z],
                [m.rotate.x, m.rotate.y, m.rotate.z],
            ),
            None => ([0.0; 3], [0.0; 3]),
        };
        let q = Quaternionf::rotation_xyz(rot[0] * DEG, rot[1] * DEG, rot[2] * DEG);
        let s12 = scale * scale2;
        self.render_transform = chassis_m.mul(&translation_rotate_scale(
            [-off[0], off[1], off[2]],
            q,
            s12 * invert_x,
            s12,
            s12,
        ));
        self.vehicle_transform = chassis_m.mul(&translation_rotate_scale(
            [-off[0], off[1], off[2]],
            Quaternionf::default(),
            scale,
            scale,
            scale,
        ));
        let rest = s.suspension_rest_length();
        let wheels: Vec<_> = (0..s.wheel_count()).map(|i| s.wheel(i).offset).collect();
        let model_scale = s.model_scale();
        drop(s);
        let wheel_info = v.st().wheel_info;
        let impulses: Vec<bool> = v
            .st()
            .impulses_from_squished_bodies
            .iter()
            .map(|i| i.as_ref().is_some_and(|i| i.enable))
            .collect();
        for i in 0..self.parts.len() {
            let p = &self.parts[i];
            let sm = &p.script_model;
            let off = [sm.offset.x, sm.offset.y, sm.offset.z];
            let rot = [sm.rotate.x, sm.rotate.y, sm.rotate.z];
            let scale1 = sm.scale;
            let (scale2, inv2) = match &p.model {
                Some(m) => (m.scale, if m.invert_x { -1.0 } else { 1.0 }),
                None => (1.0, 1.0),
            };
            let ryz = if p.wheel_index == -1 { -1.0 } else { 1.0 };
            let q = Quaternionf::rotation_xyz(rot[0] * DEG, rot[1] * DEG * ryz, rot[2] * DEG * ryz);
            let s12 = scale1 * scale2;
            if p.wheel_index == -1 {
                let parent = p.part.parent().and_then(|pp| self.part_model_index(&pp));
                if let (Some(pi), Some(an)) = (
                    parent.filter(|_| p.part.script_part().is_some_and(|sp| sp.parent.is_some())),
                    sm.attachment_name_parent.clone(),
                ) {
                    let Some(parent_model) = self.parts[pi].model.clone() else {
                        continue;
                    };
                    let me = self.parts[i].model.clone();
                    let att = init_transform(
                        v,
                        &parent_model,
                        self.part_player(pi),
                        me.as_deref(),
                        &an,
                        self.parts[i].script_model.attachment_name_self.as_deref(),
                    );
                    let att = pre_multiply_mesh_transform(&att, &parent_model);
                    let scale3 = if self.parts[i].script_model.ignore_vehicle_scale {
                        1.5 / model_scale
                    } else {
                        1.0
                    };
                    let r = self.parts[pi]
                        .render_transform
                        .mul(&att)
                        .scale1(s12 * scale3);
                    self.parts[i].render_transform = r;
                    continue;
                }
                let r =
                    translation_rotate_scale([-off[0], off[1], off[2]], q, s12 * inv2, s12, s12);
                self.parts[i].render_transform = self.vehicle_transform.mul(&r);
                continue;
            }
            let wi = p.wheel_index as usize;
            let w = wheel_info[wi];
            let wo = wheels[wi];
            let corpse = if impulses.get(wi).copied().unwrap_or(false) {
                0.05f32
            } else {
                0.0
            };
            let t = if w.suspension_length == 0.0 {
                Matrix4f::translation(-(wo.x / scale), wo.y / scale, wo.z / scale)
            } else {
                Matrix4f::translation(
                    -(wo.x / scale),
                    (wo.y + rest - w.suspension_length) / scale + corpse * 0.5,
                    wo.z / scale,
                )
            };
            let r = Matrix4f::identity()
                .mul(&t)
                .rotate_y(-w.steering)
                .rotate_x(w.rotation);
            let m = translation_rotate_scale([-off[0], off[1], off[2]], q, s12 * inv2, s12, s12);
            self.parts[i].render_transform = self.vehicle_transform.mul(&r.mul(&m));
        }
    }
}
