//! `ModelSlotRenderData.initModel`/`init` and `performRenderCharacter`
//! (`ModelInstanceRenderData.init`/`transformToParent`/`RenderCharacter`,
//! `Model.DrawChar`, `Shader.startCharacter`): the per-frame draw list of a
//! character, as [`pz_render::model::ModelDraw`]s in the original's order
//! (the body, then the slot's instances depth-first).

use std::rc::Rc;

use pz_anim::math::Matrix4f;
use pz_anim::skeleton::Skeleton;
use pz_anim::AnimationPlayer;
use pz_iso_render::models::{ModelBasis, ModelCamera};
use pz_model::Mat4;
use pz_render::model::{MeshData, ModelDraw, ModelLight, ModelPass};
use pz_render::Texture;

use crate::data::ModelAttachment;
use crate::lights::EffectLight;
use crate::model_tree::{ModelInstance, ModelSlot, TextureSource};

/// A loaded model's mesh (`ModelMesh`).
#[derive(Clone)]
pub struct LoadedMesh {
    pub mesh: Rc<MeshData>,
    /// `ModelMesh.transform` (column-major).
    pub transform: Option<Mat4>,
    /// The mesh's own skeleton (`model.tag`), for skinned meshes.
    pub skeleton: Option<Rc<Skeleton>>,
}

/// Meshes and textures by name (the model and texture asset managers).
pub trait ModelResources {
    /// `media/models_X/<mesh_name>` (static or skinned); `None` while not
    /// loaded (the instance is skipped, `isReady() == false`).
    fn mesh(&self, mesh_name: &str, is_static: bool) -> Option<LoadedMesh>;
    /// The texture an instance shows.
    fn texture(&self, source: &TextureSource) -> Option<Texture>;
}

/// The character state one frame's draws need.
pub struct CharacterFrame<'a> {
    pub player: &'a AnimationPlayer,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getAlpha(playerIndex)`.
    pub alpha: f32,
    /// `playerData.effectLightsMain`.
    pub effect_lights: &'a [EffectLight; 5],
    /// `playerData.currentAmbient`.
    pub ambient: [f32; 3],
    /// `finalScale` (animals' size; 1 for people).
    pub final_scale: f32,
}

/// `AnimatedModelInstanceRenderData.xfrm` in column-vector form: the
/// matrix the shader's `transform` receives.
pub type Xfrm = Mat4;

const IDENT: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// `a * b` (column-major).
pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [[0.0f32; 4]; 4];
    for c in 0..4 {
        for row in 0..4 {
            r[c][row] = a[0][row] * b[c][0]
                + a[1][row] * b[c][1]
                + a[2][row] * b[c][2]
                + a[3][row] * b[c][3];
        }
    }
    r
}

fn rot_x(a: f32) -> Mat4 {
    let (s, c) = (a.sin(), a.cos());
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, c, s, 0.0],
        [0.0, -s, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn rot_y(a: f32) -> Mat4 {
    let (s, c) = (a.sin(), a.cos());
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn rot_z(a: f32) -> Mat4 {
    let (s, c) = (a.sin(), a.cos());
    [
        [c, s, 0.0, 0.0],
        [-s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn scaling(s: f32) -> Mat4 {
    [
        [s, 0.0, 0.0, 0.0],
        [0.0, s, 0.0, 0.0],
        [0.0, 0.0, s, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// `makeAttachmentTransform`: `translation(offset).rotateXYZ(rotate°).scale(scale)`.
pub fn attachment_transform(a: &ModelAttachment) -> Mat4 {
    let d = std::f32::consts::PI / 180.0;
    let mut m = IDENT;
    let (o, r) = (a.offset(), a.rotate());
    m[3] = [o.x, o.y, o.z, 1.0];
    m = mul(&m, &rot_x(r.x * d));
    m = mul(&m, &rot_y(r.y * d));
    m = mul(&m, &rot_z(r.z * d));
    mul(&m, &scaling(a.scale()))
}

/// `makeBoneTransform`: the bone's model-space transform.
pub fn bone_transform(player: &AnimationPlayer, bone: &str) -> Mat4 {
    let i = player.skinning_bone_index(bone, -1);
    if i < 0 || bone.trim().is_empty() {
        return IDENT;
    }
    player.model_transform(i as usize).to_mat4()
}

/// `transformToParent(parentData)`.
pub fn transform_to_parent(
    inst: &ModelInstance,
    mesh: Option<&LoadedMesh>,
    parent: Option<(&ModelInstance, &Xfrm)>,
    player: &AnimationPlayer,
) -> Xfrm {
    let Some((pinst, pxfrm)) = parent else {
        return IDENT;
    };
    let mut x = *pxfrm;
    let mut parent_att = pinst.attachment(inst.attachment_name_parent.as_deref());
    if parent_att.is_none() && inst.parent_bone_name.is_some() {
        parent_att = pinst.attachment(inst.parent_bone_name.as_deref());
    }
    match parent_att {
        None => {
            if let Some(b) = &inst.parent_bone_name {
                x = mul(&bone_transform(player, b), &x);
            }
        }
        Some(a) => {
            let a = a.borrow();
            x = mul(&bone_transform(player, a.bone().unwrap_or("")), &x);
            x = mul(&x, &attachment_transform(&a));
        }
    }
    let mut self_att = inst.attachment(inst.attachment_name_self.as_deref());
    if self_att.is_none() && inst.parent_bone_name.is_some() {
        self_att = inst.attachment(inst.parent_bone_name.as_deref());
    }
    if let Some(a) = self_att {
        x = mul(&x, &attachment_transform(&a.borrow()));
    }
    if let Some(t) = mesh.and_then(|m| m.transform.as_ref()) {
        x = mul(&x, t);
    }
    if inst.scale != 1.0 {
        x = mul(&x, &scaling(inst.scale));
    }
    x
}

/// One instance's render data (`ModelInstanceRenderData`).
#[derive(Clone)]
pub struct InstanceRenderData {
    pub instance: usize,
    pub parent: Option<usize>,
    pub xfrm: Xfrm,
    pub mesh: LoadedMesh,
    /// The skin palette (skinned meshes).
    pub palette: Option<Rc<Vec<Mat4>>>,
}

/// `ModelSlotRenderData.initModel` + `init`'s transforms: the instances in
/// render order with their transforms and palettes.
pub fn init_render_data(
    slot: &ModelSlot,
    player: &AnimationPlayer,
    res: &dyn ModelResources,
) -> Vec<InstanceRenderData> {
    let mut out: Vec<InstanceRenderData> = Vec::new();
    let body = slot.instance(slot.model);
    let body_data = res.mesh(&body.mesh_name, body.is_static).map(|mesh| {
        out.push(make_data(slot.model, None, IDENT, mesh, body, player));
        out.len() - 1
    });
    fn walk(
        slot: &ModelSlot,
        list: &[usize],
        parent: Option<usize>,
        player: &AnimationPlayer,
        res: &dyn ModelResources,
        out: &mut Vec<InstanceRenderData>,
    ) {
        for &i in list {
            let inst = slot.instance(i);
            let Some(mesh) = res.mesh(&inst.mesh_name, inst.is_static) else {
                continue;
            };
            let pdata = parent.map(|p| (slot.instance(out[p].instance), &out[p].xfrm));
            let x = transform_to_parent(inst, Some(&mesh), pdata, player);
            out.push(make_data(i, parent, x, mesh, inst, player));
            let me = out.len() - 1;
            walk(slot, &inst.sub, Some(me), player, res, out);
        }
    }
    walk(slot, &slot.sub, body_data, player, res, &mut out);
    out
}

fn make_data(
    instance: usize,
    parent: Option<usize>,
    xfrm: Xfrm,
    mesh: LoadedMesh,
    inst: &ModelInstance,
    player: &AnimationPlayer,
) -> InstanceRenderData {
    let palette = (!inst.is_static).then(|| {
        Rc::new(match &mesh.skeleton {
            Some(s) => player.palette_for(s),
            None => player.palette(),
        })
    });
    InstanceRenderData {
        instance,
        parent,
        xfrm,
        mesh,
        palette,
    }
}

/// `performRenderCharacter`: the frame's draws for one character.
pub fn render_character(
    slot: &ModelSlot,
    frame: &CharacterFrame,
    camera: &ModelCamera,
    res: &dyn ModelResources,
) -> Vec<ModelDraw> {
    let mut draws = Vec::new();
    if frame.alpha < 0.01 {
        return draws;
    }
    let angle = frame.player.rendered_angle();
    let mv = camera.model_view(frame.x, frame.y, frame.z, ModelBasis::Character { angle });
    let mvp = camera.mvp(&mv);
    let target_depth = camera.target_depth(&mvp, frame.x, frame.y, frame.z, 1.0e-4);
    let obj = [frame.x, frame.y, frame.z];
    let lights: [ModelLight; 5] =
        std::array::from_fn(|i| frame.effect_lights[i].to_model_light(obj, Some(angle)));
    let ambient = [
        frame.ambient[0] * 0.45,
        frame.ambient[1] * 0.45,
        frame.ambient[2] * 0.45,
    ];
    for d in init_render_data(slot, frame.player, res) {
        let inst = slot.instance(d.instance);
        if inst.tex == TextureSource::None && inst.model_texture.is_none() {
            continue;
        }
        let texture = res.texture(&inst.tex).or_else(|| {
            inst.model_texture
                .as_ref()
                .and_then(|t| res.texture(&TextureSource::File(t.clone())))
        });
        let cull = inst
            .model_script
            .as_ref()
            .is_none_or(|s| s.borrow().cull_face != 0);
        draws.push(ModelDraw {
            mesh: d.mesh.mesh.clone(),
            texture,
            mvp: mvp.0,
            transform: if inst.is_static { d.xfrm } else { IDENT },
            palette: d.palette.clone().unwrap_or_default(),
            target_depth,
            final_scale: frame.final_scale,
            uv_scale: [1.0, 1.0],
            lights,
            ambient,
            tint: inst.tint,
            hue: inst.hue,
            alpha: frame.alpha,
            pass: ModelPass::Character { cull },
        });
    }
    draws
}

/// The LWJGL form of an [`Xfrm`] (for comparing with the original).
pub fn xfrm_to_lwjgl(x: &Xfrm) -> Matrix4f {
    Matrix4f::from_mat4(x)
}
