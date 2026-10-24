//! `FBORenderShadows`: the drop shadows of characters, corpses and
//! vehicles. Shadows are collected while the frame's moving objects are
//! drawn (`addShadow`) and drawn per level in the translucent loop
//! (`renderMain(z)`), after the level's translucent floors and before its
//! translucent objects: one flat `NewShadow.png` quad per shadow, laid in
//! the floor plane under the model camera and depth-tested (`LEQUAL`, no
//! writes) at the square depths of its corners.

use pz_render::iso::{self, fastfloor};
use pz_render::{gl, DrawCmd, Texture};

use crate::models::{ModelCamera, LEVEL_HEIGHT};
use crate::trees::Mat4;

/// `IsoDirections` of a sloped surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlopeDirection {
    N,
    S,
    W,
    E,
}

/// The sloped surface of the square under a shadow's origin
/// (`IsoGridSquare.getSlopedSurfaceDirection` / `HeightMin` / `HeightMax`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlopedSurface {
    pub square_x: i32,
    pub square_y: i32,
    pub direction: SlopeDirection,
    pub height_min: f32,
    pub height_max: f32,
}

/// `FBORenderShadows.Shadow`.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    pub ox: f32,
    pub oy: f32,
    pub oz: f32,
    /// Corners: front-left, front-right, back-right, back-left.
    pub corners: [[f32; 2]; 4],
    pub slope_angle_x: f32,
    pub slope_angle_y: f32,
    pub depths: [f32; 4],
    pub rgba: [f32; 4],
    pub texture: Texture,
    pub vehicle: bool,
}

/// `Vector2.getDirection`.
fn direction(x: f32, y: f32) -> f32 {
    y.atan2(x)
}

impl Shadow {
    /// `calculateSlopeAngles`.
    fn set_slope_angles(&mut self, slope: Option<&SlopedSurface>) {
        self.slope_angle_x = 0.0;
        self.slope_angle_y = 0.0;
        let Some(s) = slope else { return };
        let (lo, hi) = (s.height_min, s.height_max);
        match s.direction {
            SlopeDirection::N => self.slope_angle_x = direction(1.0, (hi - lo) * LEVEL_HEIGHT),
            SlopeDirection::S => self.slope_angle_x = direction(1.0, (lo - hi) * LEVEL_HEIGHT),
            SlopeDirection::W => self.slope_angle_y = direction(1.0, (lo - hi) * LEVEL_HEIGHT),
            SlopeDirection::E => self.slope_angle_y = direction(1.0, (hi - lo) * LEVEL_HEIGHT),
        }
    }

    /// `calculateApparentZ`: the height of the slope's plane under
    /// `(x, y)` (the plane extended ten squares about the square).
    fn apparent_z(&self, slope: Option<&SlopedSurface>, x: f32, y: f32, z: f32) -> f32 {
        if self.slope_angle_x == 0.0 && self.slope_angle_y == 0.0 {
            return z;
        }
        let Some(s) = slope else { return z };
        let (lo, hi) = (s.height_min, s.height_max);
        let delta = hi - lo;
        let dx = (x - (s.square_x - 5) as f32) % 10.0;
        let dy = (y - (s.square_y - 5) as f32) % 10.0;
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let z2 = match s.direction {
            SlopeDirection::N => lerp(lo - 4.0 * delta, hi + 5.0 * delta, 1.0 - dy / 10.0),
            SlopeDirection::S => lerp(lo - 5.0 * delta, hi + 4.0 * delta, dy / 10.0),
            SlopeDirection::W => lerp(lo - 4.0 * delta, hi + 5.0 * delta, 1.0 - dx / 10.0),
            SlopeDirection::E => lerp(lo - 5.0 * delta, hi + 4.0 * delta, dx / 10.0),
        };
        if z2 == -1.0 {
            return z;
        }
        fastfloor(z) as f32 + z2
    }

    /// The model-view `render` draws this shadow with: the model camera at
    /// the shadow's origin, x and z mirrored, lowered to the feet
    /// (`-0.72`), then tilted to the slope.
    // The original writes the angles as float literals.
    #[allow(clippy::approx_constant)]
    pub fn model_view(&self, cam: &ModelCamera) -> Mat4 {
        let ts = iso::TILE_SCALE as f32 / 2.0;
        let dif_x = self.ox - cam.cx;
        let dif_y = self.oy - cam.cy;
        let dif_z = (self.oz - cam.cz) * LEVEL_HEIGHT;
        let mut mv = Mat4::scaling(crate::models::core_scale())
            .scale(ts, ts, ts)
            .rotate_x(0.523_598_8)
            .rotate_y(2.356_194_5)
            .translate(-dif_x, dif_z, -dif_y)
            .scale(-1.0, 1.0, -1.0)
            .translate(0.0, -0.719_999_97, 0.0);
        if self.slope_angle_x != 0.0 {
            mv = mv.rotate_x(self.slope_angle_x);
        }
        if self.slope_angle_y != 0.0 {
            mv = mv.rotate_z(self.slope_angle_y);
        }
        mv
    }

    /// The shadow's quad (`addQuadDepth`): corners projected to normalised
    /// device coordinates, each with its square depth.
    pub fn quad(&self, cam: &ModelCamera) -> DrawCmd {
        let mvp = cam.mvp(&self.model_view(cam));
        let pos: [[f32; 3]; 4] = std::array::from_fn(|i| {
            let [x, y] = self.corners[i];
            let c = mvp.transform([x - self.ox, 0.0, y - self.oy]);
            [c[0], c[1], self.depths[i]]
        });
        let [r, g, b, a] = self.rgba;
        DrawCmd::ProjectedDepthQuad {
            texture: self.texture.clone(),
            pos,
            uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            color: crate::color::to_abgr(r, g, b, a),
        }
    }
}

/// The parameters of a character's drop shadow
/// (`IsoGameCharacter.ShadowParams`): half width and the forward and back
/// lengths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowParams {
    pub w: f32,
    pub fm: f32,
    pub bm: f32,
}

impl Default for ShadowParams {
    /// The parameters of a character without an animation player.
    fn default() -> Self {
        ShadowParams {
            w: 0.45,
            fm: 1.4,
            bm: 1.125,
        }
    }
}

/// `FBORenderShadows`: one frame's shadows.
#[derive(Default)]
pub struct Shadows {
    pub shadows: Vec<Shadow>,
    /// `media/textures/NewShadow.png` (`clear` loads it); shadows are not
    /// added while it is missing.
    pub drop_shadow: Option<Texture>,
}

impl Shadows {
    /// `clear`: a new frame's empty list, with the drop-shadow texture.
    pub fn clear(&mut self, drop_shadow: Option<Texture>) {
        self.shadows.clear();
        self.drop_shadow = drop_shadow;
    }

    /// `addShadow(x, y, z, forward, w, fm, bm, r, g, b, a, bAnimal)`: a
    /// drop shadow under a character at `(x, y, z)` facing `forward`, lit
    /// by its square's light colour.
    #[allow(clippy::too_many_arguments)]
    pub fn add_character_shadow(
        &mut self,
        cam: &ModelCamera,
        pos: [f32; 3],
        forward: [f32; 2],
        params: ShadowParams,
        rgb: [f32; 3],
        a: f32,
        animal: bool,
        slope: Option<&SlopedSurface>,
    ) {
        let Some(texture) = self.drop_shadow.clone() else {
            return;
        };
        let ShadowParams {
            mut w,
            mut fm,
            mut bm,
        } = params;
        if !animal {
            w = w.max(0.65);
            fm = fm.max(0.65);
            bm = bm.max(0.65);
        }
        let [x, y, z] = pos;
        let len = (forward[0] * forward[0] + forward[1] * forward[1]).sqrt();
        let (nx, ny) = (forward[0] / len, forward[1] / len);
        // forwardN x (0, 0, 1), scaled by w.
        let (px, py) = (ny * w, -nx * w);
        let (fx, fy) = (x + forward[0] * fm, y + forward[1] * fm);
        let (bx, by) = (x - forward[0] * bm, y - forward[1] * bm);
        let corners = [
            [fx - px, fy - py],
            [fx + px, fy + py],
            [bx + px, by + py],
            [bx - px, by - py],
        ];
        let [r, g, b] = rgb;
        let shadow_alpha = a * ((r + g + b) / 3.0) * 0.66;
        self.add_shadow(
            cam,
            [x, y, z],
            corners,
            [r, g, b, shadow_alpha],
            texture,
            false,
            slope,
        );
    }

    /// `addShadow(ox, oy, oz, x0, y0, ..., texture, bVehicle)`: a quad in
    /// the floor plane about origin `o`; each corner's depth is its square
    /// depth (on the slope, if any) less a small bias, slightly larger for
    /// shadows behind the camera's character.
    #[allow(clippy::too_many_arguments)]
    pub fn add_shadow(
        &mut self,
        cam: &ModelCamera,
        o: [f32; 3],
        corners: [[f32; 2]; 4],
        rgba: [f32; 4],
        texture: Texture,
        vehicle: bool,
        slope: Option<&SlopedSurface>,
    ) {
        if rgba[3] <= 0.0 {
            return;
        }
        let [ox, oy, oz] = o;
        let mut shadow = Shadow {
            ox,
            oy,
            oz,
            corners,
            slope_angle_x: 0.0,
            slope_angle_y: 0.0,
            depths: [0.0; 4],
            rgba,
            texture,
            vehicle,
        };
        shadow.set_slope_angles(slope);
        let (px, py) = (cam.cam.x, cam.cam.y);
        let adjust = if ox + oy < px + py {
            -1.4e-4
        } else {
            -1.499_999_9e-4
        };
        shadow.depths = corners.map(|[x, y]| {
            let z = shadow.apparent_z(slope, x, y, oz);
            iso::square_depth(fastfloor(px), fastfloor(py), x, y, z) + adjust
        });
        self.shadows.push(shadow);
    }

    /// `renderMain(z)`: draws (and removes) the shadows whose origin is on
    /// level `z`.
    pub fn render_main(&mut self, z: i32, cam: &ModelCamera, out: &mut Vec<DrawCmd>) {
        let mut level = Vec::new();
        self.shadows.retain(|s| {
            if fastfloor(s.oz) == z {
                level.push(s.clone());
                false
            } else {
                true
            }
        });
        if level.is_empty() {
            return;
        }
        out.extend([
            DrawCmd::GlEnable(gl::DEPTH_TEST),
            DrawCmd::DepthFunc(gl::LEQUAL),
            DrawCmd::DepthMask(false),
            // IndieGL.glDefaultBlendFuncA.
            DrawCmd::BlendFunc(gl::ONE, gl::ONE_MINUS_SRC_ALPHA),
        ]);
        out.extend(level.iter().map(|s| s.quad(cam)));
        // GLStateRenderThread.restore: the sprite renderer's default blend.
        out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    }

    /// `endRender`.
    pub fn end_render(&mut self) {
        self.shadows.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Camera;

    fn tex() -> Texture {
        let mut t = Texture::empty("NewShadow");
        t.id = Some(1);
        t
    }

    #[test]
    fn character_shadow_corners_and_alpha() {
        let cam = Camera::centred(10.5, 10.5, 0.0, 1920, 1080, 1.0);
        let mc = ModelCamera::new(&cam, 1920.0, 1080.0);
        let mut s = Shadows::default();
        s.add_character_shadow(
            &mc,
            [10.5, 10.5, 0.0],
            [1.0, 0.0],
            ShadowParams::default(),
            [1.0, 1.0, 1.0],
            1.0,
            false,
            None,
        );
        assert!(s.shadows.is_empty(), "no texture, no shadow");
        s.clear(Some(tex()));
        s.add_character_shadow(
            &mc,
            [10.5, 10.5, 0.0],
            [1.0, 0.0],
            ShadowParams::default(),
            [0.5, 0.5, 0.5],
            1.0,
            false,
            None,
        );
        let sh = &s.shadows[0];
        // w is raised to 0.65; facing +x the perpendicular is -y.
        assert_eq!(sh.corners[0], [10.5 + 1.4, 10.5 + 0.65]);
        assert_eq!(sh.corners[2], [10.5 - 1.125, 10.5 - 0.65]);
        assert!((sh.rgba[3] - 0.33).abs() < 1e-6);
        // Not behind the camera's character: the larger bias.
        let d = iso::square_depth(10, 10, sh.corners[1][0], sh.corners[1][1], 0.0) - 1.499_999_9e-4;
        assert_eq!(sh.depths[1], d);
    }

    #[test]
    fn shadows_render_per_level() {
        let cam = Camera::centred(10.5, 10.5, 0.0, 1920, 1080, 1.0);
        let mc = ModelCamera::new(&cam, 1920.0, 1080.0);
        let mut s = Shadows::default();
        s.clear(Some(tex()));
        s.add_character_shadow(
            &mc,
            [10.5, 10.5, 0.0],
            [0.0, 1.0],
            ShadowParams::default(),
            [1.0; 3],
            1.0,
            false,
            None,
        );
        s.add_character_shadow(
            &mc,
            [10.5, 10.5, 1.0],
            [0.0, 1.0],
            ShadowParams::default(),
            [1.0; 3],
            1.0,
            false,
            None,
        );
        let mut out = Vec::new();
        s.render_main(0, &mc, &mut out);
        assert_eq!(
            out.iter()
                .filter(|c| matches!(c, DrawCmd::ProjectedDepthQuad { .. }))
                .count(),
            1
        );
        assert_eq!(s.shadows.len(), 1);
        // Under the camera's character: facing +y the quad's centre is
        // (fm - bm) / 2 squares south, 64 px a square to the left.
        let DrawCmd::ProjectedDepthQuad { pos, .. } = &out[4] else {
            panic!()
        };
        let cx = pos.iter().map(|p| p[0]).sum::<f32>() / 4.0 * 1920.0 / 2.0;
        assert!((cx + (1.4 - 1.125) / 2.0 * 64.0).abs() < 0.01, "{cx}");
        s.render_main(1, &mc, &mut out);
        assert!(s.shadows.is_empty());
    }

    #[test]
    fn slope_raises_the_far_corners() {
        let cam = Camera::centred(10.5, 10.5, 0.0, 1920, 1080, 1.0);
        let mc = ModelCamera::new(&cam, 1920.0, 1080.0);
        let mut s = Shadows::default();
        let slope = SlopedSurface {
            square_x: 10,
            square_y: 10,
            direction: SlopeDirection::N,
            height_min: 0.0,
            height_max: 0.5,
        };
        s.add_shadow(
            &mc,
            [10.5, 10.5, 0.0],
            [[10.0, 10.0], [11.0, 10.0], [11.0, 11.0], [10.0, 11.0]],
            [0.0, 0.0, 0.0, 0.5],
            tex(),
            false,
            Some(&slope),
        );
        let sh = &s.shadows[0];
        assert!(sh.slope_angle_x > 0.0 && sh.slope_angle_y == 0.0);
        // The north-west corner is 0.5 into the ten-square plane from its
        // north edge: lerp(min - 4d, max + 5d, 0.5) = 0.5 levels up.
        let adjust = -1.499_999_9e-4;
        assert_eq!(
            sh.depths[0],
            iso::square_depth(10, 10, 10.0, 10.0, 0.5) + adjust
        );
        assert!(sh.depths[0] < iso::square_depth(10, 10, 10.0, 10.0, 0.0) + adjust);
    }
}
