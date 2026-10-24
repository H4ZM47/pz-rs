//! The model camera and uniforms: `Core.DoPushIsoStuff`'s projection and
//! model-view for a model at a world position, `targetDepth`, and
//! `Shader.setLight`'s light encoding. The draws themselves are
//! [`pz_render::model::ModelDraw`]s, made by the character, vehicle and
//! item code.

use pz_render::iso::{self, fastfloor};
use pz_render::model::ModelLight;

use crate::color::ColorInfo;
use crate::trees::Mat4;
use crate::Camera;

/// `Core.scale`.
pub(crate) fn core_scale() -> f32 {
    (std::f64::consts::SQRT_2 / 2.0 / 10.0) as f32 / 1.5
}

/// One level in model space (`√6`).
pub const LEVEL_HEIGHT: f32 = 2.44949;

/// What kind of model a model-view is built for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModelBasis {
    /// Characters and items: `S(-1.5, 1.5, 1.5) * Ry(angle + π) *
    /// T(0, -0.48, 0)`.
    Character { angle: f32 },
    /// Vehicles: `S(-1, 1, 1) * Ry(π)`.
    Vehicle,
}

/// The model camera for one frame: `Core.DoPushIsoStuff` with the camera
/// centred on `(cx, cy, cz)`.
#[derive(Clone, Copy, Debug)]
pub struct ModelCamera {
    pub projection: Mat4,
    pub cx: f32,
    pub cy: f32,
    pub cz: f32,
    /// The camera's character position, for square depths.
    pub cam: Camera,
}

impl ModelCamera {
    /// For an offscreen buffer of `width` x `height` pixels (screen size
    /// times zoom) centred on the camera's character.
    pub fn new(cam: &Camera, width: f32, height: f32) -> Self {
        let (w, h) = (width / 1920.0, height / 1920.0);
        ModelCamera { projection: Mat4::ortho(-w / 2.0, w / 2.0, -h / 2.0, h / 2.0, -10.0, 10.0), cx: cam.x, cy: cam.y, cz: cam.z, cam: *cam }
    }

    /// The model-view for a model at world `(ox, oy, oz)`.
    // The original writes the angles as float literals.
    #[allow(clippy::approx_constant)]
    pub fn model_view(&self, ox: f32, oy: f32, oz: f32, basis: ModelBasis) -> Mat4 {
        let ts = iso::TILE_SCALE as f32 / 2.0;
        let mv = Mat4::scaling(core_scale())
            .scale(ts, ts, ts)
            .rotate_x(0.523_598_8)
            .rotate_y(2.356_194_5)
            .translate(-(ox - self.cx), (oz - self.cz) * LEVEL_HEIGHT, -(oy - self.cy));
        match basis {
            ModelBasis::Character { angle } => mv.scale(-1.5, 1.5, 1.5).rotate_y(angle + std::f32::consts::PI).translate(0.0, -0.48, 0.0),
            ModelBasis::Vehicle => mv.scale(-1.0, 1.0, 1.0).rotate_y(std::f32::consts::PI),
        }
    }

    /// `ModelViewProjection` for a model-view.
    pub fn mvp(&self, model_view: &Mat4) -> Mat4 {
        self.projection.mul_mat(model_view)
    }

    /// `targetDepth` (`performRenderCharacter`): the square depth of
    /// `(x, y, z)` less the projected origin's depth, so the origin draws
    /// at the square's depth; `bias` is 1e-4 for characters, 0 for
    /// vehicles.
    pub fn target_depth(&self, mvp: &Mat4, x: f32, y: f32, z: f32, bias: f32) -> f32 {
        let square = iso::square_depth(fastfloor(self.cam.x), fastfloor(self.cam.y), x + self.cam.fix_jiggly_square_x, y + self.cam.fix_jiggly_square_y, z);
        let d0 = mvp.transform([0.0, 0.0, 0.0])[2];
        square - (d0 + 1.0) / 2.0 + 0.5 - bias
    }
}

/// `Shader.setLight`: a light at world `light` (or, with no `angle`, a
/// direction) of colour `rgb` and `radius`, for a model at `obj` turned by
/// `angle`, in the model's space.
pub fn encode_light(light: [f32; 3], rgb: [f32; 3], radius: f32, obj: [f32; 3], angle: Option<f32>) -> ModelLight {
    if rgb[0] + rgb[1] + rgb[2] == 0.0 || radius <= 0.0 {
        return ModelLight { direction: [0.0, 1.0, 0.0], colour: [0.0; 3] };
    }
    let mut v = match angle {
        Some(_) => [light[0] - obj[0], light[1] - obj[1], light[2] - obj[2]],
        None => light,
    };
    let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let norm = |v: [f32; 3]| {
        let l = len(v);
        [v[0] / l, v[1] / l, v[2] / l]
    };
    let dist = len(v);
    v = if dist < 1.0e-4 { [0.0, 0.0, 1.0] } else { norm(v) };
    if let Some(a) = angle {
        let r = -(a as f64);
        let (vx, vy) = (v[0] as f64, v[1] as f64);
        v[0] = (vx * r.cos() - vy * r.sin()) as f32;
        v[1] = (vx * r.sin() + vy * r.cos()) as f32;
    }
    v = [v[0], v[2], v[1]];
    if len(v) < 1.0e-4 {
        v = [0.0, 1.0, 0.0];
    }
    v = norm(v);
    let del = (1.0 - dist / radius).clamp(0.0, 1.0);
    let c = |x: f32| crate::color::clamp(x * del, 0.0, 1.0);
    ModelLight { direction: [-v[0], v[1], v[2]], colour: [c(rgb[0]), c(rgb[1]), c(rgb[2])] }
}

/// `IsoGridSquare.interpolateLight`: the square's four floor-corner lights
/// blended at the fractional position `(fx, fy)`.
pub fn interpolate_light(vert_lights: &[u32; 8], fx: f32, fy: f32) -> ColorInfo {
    use crate::color::{alpha, blue, green, red};
    let (x, y) = (fx.clamp(0.0, 1.0), fy.clamp(0.0, 1.0));
    let c = |v: u32| [red(v), green(v), blue(v), alpha(v)];
    let (tl, tr, br, bl) = (c(vert_lights[0]), c(vert_lights[1]), c(vert_lights[2]), c(vert_lights[3]));
    let interp = |a: [f32; 4], b: [f32; 4], d: f32| std::array::from_fn::<f32, 4, _>(|i| a[i] + (b[i] - a[i]) * d);
    let f = interp(interp(tl, tr, x), interp(bl, br, x), y);
    ColorInfo::new(f[0], f[1], f[2], f[3])
}

/// The lights `ModelInstance.updateLights` gives a character with no point
/// lights nearby, outside or in a room: two fill lights from the
/// north-west and south-east above, each a quarter of 1.7 × the ambient,
/// and the ambient uniform (0.45 × the square light).
pub fn fill_lights(ambient: ColorInfo, pos: [f32; 3], angle: f32) -> ([ModelLight; 5], [f32; 3]) {
    let a = [ambient.r * 1.7 / 4.0, ambient.g * 1.7 / 4.0, ambient.b * 1.7 / 4.0];
    let none = ModelLight { direction: [0.0, 1.0, 0.0], colour: [0.0; 3] };
    let l3 = encode_light([pos[0] - 2.0, pos[1] - 2.0, pos[2] + 1.0], a, 5000.0, pos, Some(angle));
    let l4 = encode_light([pos[0] + 2.0, pos[1] + 2.0, pos[2] + 1.0], a, 5000.0, pos, Some(angle));
    ([none, none, none, l3, l4], [ambient.r * 0.45, ambient.g * 0.45, ambient.b * 0.45])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model one square east of the camera centre lands 64 pixels right
    /// and 32 down at zoom 1 (tile scale 2), as tiles do.
    #[test]
    fn model_projection_matches_tiles() {
        let cam = Camera::centred(10.0, 10.0, 0.0, 1920, 1080, 1.0);
        let mc = ModelCamera::new(&cam, 1920.0, 1080.0);
        let mv = mc.model_view(11.0, 10.0, 0.0, ModelBasis::Vehicle);
        let c = mc.mvp(&mv).transform([0.0, 0.0, 0.0]);
        let (px, py) = (c[0] * 1920.0 / 2.0, -c[1] * 1080.0 / 2.0);
        assert!((px - 64.0).abs() < 0.01 && (py - 32.0).abs() < 0.01, "{px} {py}");
        let up = mc.mvp(&mc.model_view(10.0, 10.0, 1.0, ModelBasis::Vehicle)).transform([0.0, 0.0, 0.0]);
        assert!((-up[1] * 1080.0 / 2.0 + 192.0).abs() < 0.05, "{:?}", up);
    }

    /// Model depth is in tile depth units: one square east is nearer by
    /// the same amount `calculateDepth` gives.
    #[test]
    fn model_depth_matches_tile_depth() {
        let cam = Camera::centred(10.0, 10.0, 0.0, 1920, 1080, 1.0);
        let mc = ModelCamera::new(&cam, 1920.0, 1080.0);
        let mvp0 = mc.mvp(&mc.model_view(10.5, 10.5, 0.0, ModelBasis::Vehicle));
        let d0 = mvp0.transform([0.0, 0.0, 0.0])[2];
        let d1 = mvp0.transform([-1.0, 0.0, 0.0])[2];
        // Model -x is world +x (the vehicle basis mirrors x).
        let window_step = (d1 - d0) / 2.0;
        assert!((window_step + 0.023_093_667 / 16.0).abs() < 1e-6, "{window_step}");
    }

    #[test]
    fn lights_point_at_the_model() {
        let l = encode_light([12.0, 10.0, 0.0], [1.0, 1.0, 1.0], 4.0, [10.0, 10.0, 0.0], Some(0.0));
        assert_eq!(l.colour, [0.5, 0.5, 0.5]);
        assert!((l.direction[0] + 1.0).abs() < 1e-6);
        let none = encode_light([0.0; 3], [0.0; 3], 1.0, [0.0; 3], Some(0.0));
        assert_eq!(none.direction, [0.0, 1.0, 0.0]);
    }
}
