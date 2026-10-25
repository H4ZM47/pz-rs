//! One vehicle's frame: `BaseVehicle.render` → `IsoSprite.renderVehicle`
//! → `ModelSlotRenderData` (vehicle branch: square depth, centre-of-mass
//! offset, ambient, the instance's lights in vehicle space) →
//! `ModelInstanceRenderData.RenderVehicle` → `Model.DrawVehicle` for the
//! body and each part model; and `BaseVehicle.renderShadow`.

use std::rc::Rc;

use pz_iso_render::joml::Matrix4f;
use pz_iso_render::models::{encode_light, ModelBasis, ModelCamera, LEVEL_HEIGHT};
use pz_iso_render::objects::{ObjectRender, ShadowRequest};
use pz_iso_render::shadows::SlopedSurface;
use pz_render::iso::{self, fastfloor};
use pz_render::model::{ModelDraw, ModelLight, ModelPass, VehicleDraw, VehicleShader};
use pz_render::{DrawCmd, Texture};
use pz_vehicles::BaseVehicle;

use crate::models::{VehicleModel, VehicleModelSet};
use crate::transform::post_multiply_mesh_transform;

/// `BaseVehicle.centerOfMassMagic`.
pub const CENTER_OF_MASS_MAGIC: f32 = 0.7;

/// A light the lighting system gives the vehicle's square
/// (`IsoGridSquare.ResultLight` copied by `ModelManager.getSquareLighting`
/// into `VehicleModelInstance.lights`), in world coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SquareLight {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub radius: i32,
}

impl SquareLight {
    /// `ModelManager.getSquareLighting`: the vehicle square's result lights
    /// (`square.lighting[playerIndex].getResultLight(i)`, e.g.
    /// `WorldLighting::result_lights`), at most five, copied as they are.
    pub fn from_result_lights(lights: &[pz_lighting::jni::ResultLightInfo]) -> Vec<SquareLight> {
        lights
            .iter()
            .take(5)
            .map(|l| SquareLight {
                x: l.x as f32,
                y: l.y as f32,
                z: l.z as f32,
                r: l.r,
                g: l.g,
                b: l.b,
                radius: l.radius,
            })
            .collect()
    }
}

/// What a vehicle's render needs from the world besides the vehicle.
#[derive(Clone, Debug)]
pub struct VehicleFrame {
    /// `getAlpha(playerIndex)`.
    pub alpha: f32,
    /// `square.interpolateLight` at the vehicle's position (the player's
    /// ambient when it has no square).
    pub ambient: [f32; 3],
    /// The square's result lights (up to five).
    pub lights: Vec<SquareLight>,
    /// `SkyBox.getTextureShift()`.
    pub sky_texture_shift: f32,
    /// `SkyBox.getTextureCurrent()` / `getTexturePrev()` for the
    /// reflective shaders.
    pub sky_textures: [Option<Texture>; 2],
    /// The current square's `lighting[playerIndex].lightInfo()` colour
    /// (the shadow's alpha), and whether the square is seen.
    pub square_light: [f32; 3],
    pub seen: bool,
    /// The sloped surface under the vehicle, if any.
    pub slope: Option<SlopedSurface>,
    /// `media/vehicleShadow.png` (the shadow when the skin has none).
    pub default_shadow: Option<Texture>,
}

/// The per-slot values of a vehicle's draws (`ModelSlotRenderData.init`).
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleSlot {
    /// `squareDepth`.
    pub square_depth: f32,
    /// `centerOfMassY`.
    pub center_of_mass_y: f32,
    /// `ModelView` after `translate(0, centerOfMassY, 0)`.
    pub model_view: Matrix4f,
    pub projection: Matrix4f,
    /// `ModelInstance.targetDepth`.
    pub target_depth: f32,
    /// `effectLights` in vehicle space.
    pub lights: [SquareLight; 5],
}

fn joml(m: &pz_iso_render::trees::Mat4) -> Matrix4f {
    Matrix4f::from_cols(m.0)
}

/// `ModelSlotRenderData.init` (vehicle branch) and the camera part of
/// `Model.DrawVehicle`.
pub fn vehicle_slot(v: &BaseVehicle, frame: &VehicleFrame, cam: &ModelCamera) -> VehicleSlot {
    let (x, y, z) = (v.x(), v.y(), v.z());
    let origin_y = v.st().jni_transform.origin.y;
    let mut com = origin_y - CENTER_OF_MASS_MAGIC;
    com -= z * 3.0 * 0.816_496_7;
    let square_depth = iso::square_depth(
        fastfloor(cam.cam.x),
        fastfloor(cam.cam.y),
        x,
        y,
        origin_y / LEVEL_HEIGHT,
    );
    let mv = joml(&cam.model_view(x, y, z, ModelBasis::Vehicle)).translate(0.0, com, 0.0);
    let projection = joml(&cam.projection);
    let mvp = projection.mul(&mv);
    let d0 = mvp.transform([0.0, 0.0, 0.0, 1.0])[2];
    let target_depth = square_depth - (d0 + 1.0) / 2.0 + 0.5;
    let mut lights = [SquareLight::default(); 5];
    for (i, l) in frame.lights.iter().take(5).enumerate() {
        if l.radius <= 0 {
            continue;
        }
        let local = v.local_pos(l.x, l.y, l.z + 0.75);
        lights[i] = SquareLight {
            x: local.x,
            y: local.y,
            z: local.z,
            ..*l
        };
    }
    VehicleSlot {
        square_depth,
        center_of_mass_y: com,
        model_view: mv,
        projection,
        target_depth,
        lights,
    }
}

/// One model of the slot as `ModelInstanceRenderData` holds it.
struct Instance<'a> {
    model: &'a VehicleModel,
    xfrm: Matrix4f,
    /// `instData.tex`.
    tex: Option<Texture>,
    palette: Option<Vec<[[f32; 4]; 4]>>,
    is_sub: bool,
}

/// `BaseVehicle.render` (with `sprite.renderVehicle`): the vehicle's model
/// draws, in the slot's order (body, then the part models).
pub fn vehicle_draws(
    v: &BaseVehicle,
    set: &VehicleModelSet,
    frame: &VehicleFrame,
    cam: &ModelCamera,
) -> Vec<DrawCmd> {
    let mut out = Vec::new();
    let slot = vehicle_slot(v, frame, cam);
    let lights: [ModelLight; 5] = std::array::from_fn(|i| {
        let l = slot.lights[i];
        encode_light(
            [l.x, l.y, l.z],
            [l.r, l.g, l.b],
            l.radius as f32,
            [v.x(), v.y(), v.z()],
            None,
        )
    });
    let mut instances = Vec::new();
    if let Some(body) = &set.body {
        instances.push(Instance {
            model: body,
            xfrm: post_multiply_mesh_transform(&set.render_transform, body),
            tex: set.skin.tex.clone(),
            palette: None,
            is_sub: false,
        });
    }
    for (i, p) in set.parts.iter().enumerate() {
        let Some(m) = &p.model else { continue };
        let tex = m.texture.clone().or_else(|| set.skin.tex.clone());
        let palette = (!m.is_static)
            .then(|| set.part_player(i).map(|pl| pl.palette()))
            .flatten();
        instances.push(Instance {
            model: m,
            xfrm: post_multiply_mesh_transform(&p.render_transform, m),
            tex,
            palette,
            is_sub: true,
        });
    }
    let skin = &set.skin;
    let textures = [
        skin.tex.clone(),
        skin.rust.clone(),
        skin.mask.clone(),
        skin.lights.clone(),
        skin.damage1_overlay.clone(),
        skin.damage1_shell.clone(),
        skin.damage2_overlay.clone(),
        skin.damage2_shell.clone(),
        frame.sky_textures[0].clone(),
        frame.sky_textures[1].clone(),
    ];
    for inst in instances {
        // RenderVehicle: nothing to draw without a texture.
        if inst.tex.is_none() && inst.model.texture.is_none() {
            continue;
        }
        let mv = slot.model_view.mul(&inst.xfrm);
        let mvp = slot.projection.mul(&mv);
        let identity = Matrix4f::identity().m;
        let base = ModelDraw {
            mesh: inst.model.mesh.clone(),
            texture: inst.tex.clone(),
            mvp: mvp.m,
            transform: identity,
            palette: Rc::new(inst.palette.unwrap_or_default()),
            target_depth: slot.target_depth,
            final_scale: 1.0,
            uv_scale: [1.0, 1.0],
            lights,
            ambient: frame.ambient,
            tint: [1.0; 3],
            hue: 0.0,
            alpha: frame.alpha,
            pass: ModelPass::Character { cull: true },
        };
        match inst.model.shader {
            Some((shader, noreflect)) if shader.is_vehicle_shader() => {
                out.push(DrawCmd::VehicleModel(Box::new(VehicleDraw {
                    base,
                    shader,
                    noreflect,
                    invert_x: inst.model.invert_x,
                    textures: textures.clone(),
                    uniforms: set.instance.uniforms(frame.alpha, frame.sky_texture_shift),
                })));
            }
            Some((VehicleShader::Wheel, _)) => {
                let mut t: pz_render::model::VehicleTextures = Default::default();
                t[0] = inst.tex.clone();
                let mut u = pz_render::model::VehicleUniforms::default();
                // Alpha (setShaderAlpha) for sub-models; a body with the
                // wheel shader keeps the uniform's default.
                u.paint_color[3] = if inst.is_sub { frame.alpha } else { 1.0 };
                out.push(DrawCmd::VehicleModel(Box::new(VehicleDraw {
                    base,
                    shader: VehicleShader::Wheel,
                    noreflect: true,
                    invert_x: inst.model.invert_x,
                    textures: t,
                    uniforms: u,
                })));
            }
            _ => {
                // A part model with another shader (basicEffect).
                out.push(DrawCmd::Model(Box::new(base)));
            }
        }
    }
    out
}

/// `BaseVehicle.getShadowTexture`.
pub fn shadow_texture(set: &VehicleModelSet, frame: &VehicleFrame) -> Option<Texture> {
    set.skin
        .shadow
        .clone()
        .or_else(|| frame.default_shadow.clone())
}

/// `BaseVehicle.renderShadow` (FBO render path): the shadow quad over the
/// vehicle's shadow polygon.
pub fn vehicle_shadow(
    v: &BaseVehicle,
    set: &VehicleModelSet,
    frame: &VehicleFrame,
) -> Option<ShadowRequest> {
    v.script()?;
    v.st().square?;
    if !frame.seen {
        return None;
    }
    let tex = shadow_texture(set, frame)?;
    let mut alpha = 0.6 * frame.alpha;
    let [r, g, b] = frame.square_light;
    alpha *= (r + g + b) / 3.0;
    let _ = v.poly();
    let sc = v.st().shadow_coord;
    let z = fastfloor(v.z()) as f32;
    Some(ShadowRequest::Quad {
        origin: [v.x(), v.y(), z],
        corners: [
            [sc.x2, sc.y2],
            [sc.x1, sc.y1],
            [sc.x4, sc.y4],
            [sc.x3, sc.y3],
        ],
        rgba: [1.0, 1.0, 1.0, 0.8 * alpha],
        texture: Some(tex),
        vehicle: true,
        slope: frame.slope,
    })
}

/// The vehicle as `FBORenderCell.renderMovingObject` draws it: its shadow
/// request, then its model draws.
pub fn render_vehicle_object(
    v: &BaseVehicle,
    set: &VehicleModelSet,
    frame: &VehicleFrame,
    cam: &ModelCamera,
) -> ObjectRender {
    ObjectRender {
        shadow: vehicle_shadow(v, set, frame),
        draws: vehicle_draws(v, set, frame, cam),
        ..Default::default()
    }
}
