//! `FBORenderTrees`: trees drawn every frame as billboards through a 3D
//! camera (`vboRenderer_PositionColorUV` with a per-run `userDepth`), with
//! `DepthStagger` keeping overlapping billboards on the same diagonal at
//! distinct depths. Recording trees into chunk textures
//! (`FBORenderChunkCamera`) is not ported; the original draws trees in the
//! per-frame pass.

use std::collections::HashMap;

use pz_render::iso::{self, fastfloor, x_to_screen};
use pz_render::{gl, DrawCmd, Texture};

use crate::chunk::{JUMBO_L_WIDTH, JUMBO_XL_WIDTH, JUMBO_XXL_WIDTH};
use crate::Camera;

/// `Core.scale`.
fn core_scale() -> f32 {
    (std::f64::consts::SQRT_2 / 2.0 / 10.0) as f32 / 1.5
}

/// A column-major 4x4 matrix with JOML's post-multiplying operations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [[f32; 4]; 4]);

impl Mat4 {
    pub fn identity() -> Self {
        let mut m = [[0.0; 4]; 4];
        for (i, c) in m.iter_mut().enumerate() {
            c[i] = 1.0;
        }
        Mat4(m)
    }

    fn mul(&self, o: &Mat4) -> Mat4 {
        let mut r = [[0.0f32; 4]; 4];
        for (c, col) in r.iter_mut().enumerate() {
            for (row, v) in col.iter_mut().enumerate() {
                *v = (0..4).map(|k| self.0[k][row] * o.0[c][k]).sum();
            }
        }
        Mat4(r)
    }

    /// `scaling(s)`.
    pub fn scaling(s: f32) -> Self {
        let mut m = Mat4::identity();
        m.0[0][0] = s;
        m.0[1][1] = s;
        m.0[2][2] = s;
        m
    }

    /// `scale(x, y, z)`.
    pub fn scale(&self, x: f32, y: f32, z: f32) -> Self {
        let mut s = Mat4::identity();
        s.0[0][0] = x;
        s.0[1][1] = y;
        s.0[2][2] = z;
        self.mul(&s)
    }

    /// `rotate(angle, 1, 0, 0)`.
    pub fn rotate_x(&self, a: f32) -> Self {
        let (s, c) = ((a as f64).sin() as f32, (a as f64).cos() as f32);
        let mut r = Mat4::identity();
        r.0[1][1] = c;
        r.0[1][2] = s;
        r.0[2][1] = -s;
        r.0[2][2] = c;
        self.mul(&r)
    }

    /// `rotate(angle, 0, 1, 0)`.
    pub fn rotate_y(&self, a: f32) -> Self {
        let (s, c) = ((a as f64).sin() as f32, (a as f64).cos() as f32);
        let mut r = Mat4::identity();
        r.0[0][0] = c;
        r.0[0][2] = -s;
        r.0[2][0] = s;
        r.0[2][2] = c;
        self.mul(&r)
    }

    /// `translate(x, y, z)`.
    pub fn translate(&self, x: f32, y: f32, z: f32) -> Self {
        let mut t = Mat4::identity();
        t.0[3][0] = x;
        t.0[3][1] = y;
        t.0[3][2] = z;
        self.mul(&t)
    }

    /// `setOrtho(l, r, b, t, n, f)` (GL depth range).
    pub fn ortho(l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) -> Self {
        let mut m = Mat4::identity();
        m.0[0][0] = 2.0 / (r - l);
        m.0[1][1] = 2.0 / (t - b);
        m.0[2][2] = 2.0 / (n - f);
        m.0[3][0] = (r + l) / (l - r);
        m.0[3][1] = (t + b) / (b - t);
        m.0[3][2] = (f + n) / (n - f);
        m
    }

    pub fn transform(&self, p: [f32; 3]) -> [f32; 4] {
        std::array::from_fn(|row| self.0[0][row] * p[0] + self.0[1][row] * p[1] + self.0[2][row] * p[2] + self.0[3][row])
    }
}

/// `DepthStagger`: per diagonal, billboards that overlap on screen get
/// offsets 0, 1, -1, 2, -2, ….
#[derive(Default)]
pub struct DepthStagger {
    billboards: HashMap<i32, Vec<(i32, i32, i32)>>,
}

impl DepthStagger {
    pub fn add_billboard(&mut self, depth: i32, screen_x: i32, width: i32) -> i32 {
        let list = self.billboards.entry(depth).or_default();
        let overlap: Vec<i32> = list.iter().filter(|(x, w, _)| screen_x < x + w && screen_x + width > *x).map(|b| b.2).collect();
        let mut offset = None;
        for i in 0..10 {
            if !overlap.contains(&i) {
                offset = Some(i);
                break;
            }
            if !overlap.contains(&-i) {
                offset = Some(-i);
                break;
            }
        }
        let offset = offset.expect("too much overlap");
        // Newest first, as the original's linked list.
        list.insert(0, (screen_x, width, offset));
        offset
    }
}

/// One `FBORenderTrees.Tree`.
#[derive(Clone, Debug)]
pub struct Tree {
    pub texture: Option<Texture>,
    pub texture2: Option<Texture>,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub color: [f32; 4],
    pub transparent: bool,
    pub cutaway_alpha: f32,
    pub fade_alpha: f32,
    pub depth_offset: i32,
}

/// The trees of one frame (`FBORenderTrees.current` plus the shared
/// `depthStagger`/`depthDone`).
#[derive(Default)]
pub struct Trees {
    pub trees: Vec<Tree>,
    stagger: DepthStagger,
    depth_done: HashMap<i64, i32>,
}

impl Trees {
    /// `addTree`, with the tree's square position.
    #[allow(clippy::too_many_arguments)]
    pub fn add(&mut self, texture: Option<Texture>, texture2: Option<Texture>, x: f32, y: f32, z: f32, color: [f32; 4], transparent: bool, cutaway_alpha: f32, fade_alpha: f32) {
        let (x, y) = (x + 1.0, y + 1.0);
        let key = fastfloor(x) as i64 + fastfloor(y) as i64 * (1 << 20);
        let depth_offset = match self.depth_done.get(&key) {
            Some(&d) => d,
            None => {
                let w = |t: &Option<Texture>| t.as_ref().map_or(0, |t| t.width_orig());
                let screen_width = w(&texture).max(w(&texture2));
                let screen_x = fastfloor(x_to_screen(x, y)) - screen_width / 2;
                let d = self.stagger.add_billboard(fastfloor(x) + fastfloor(y), screen_x, screen_width);
                self.depth_done.insert(key, d);
                d
            }
        };
        self.trees.push(Tree { texture, texture2, x, y, z, color, transparent, cutaway_alpha, fade_alpha, depth_offset });
    }

    /// `FBORenderTrees.render` in the per-frame pass, for a camera whose
    /// offscreen buffer is `width` x `height` (already zoomed).
    pub fn render(&self, cam: &Camera, width: f32, height: f32, out: &mut Vec<DrawCmd>) {
        let sw = width / 1920.0;
        let sh = height / 1920.0;
        let projection = Mat4::ortho(-sw / 2.0, sw / 2.0, -sh / 2.0, sh / 2.0, -10.0, 10.0);
        for t in &self.trees {
            out.push(DrawCmd::GlEnable(gl::DEPTH_TEST));
            out.push(DrawCmd::DepthMask(true));
            out.push(DrawCmd::DepthFunc(gl::LEQUAL));
            let mut color = t.color;
            if t.transparent {
                color[3] = t.cutaway_alpha.min(t.fade_alpha);
                out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
            }
            let mv = model_view(cam, t.x, t.y, t.z);
            let mvp = projection.mul(&mv);
            self.render_tree_textures(cam, t, &mvp, color, out);
        }
    }

    fn render_tree_textures(&self, cam: &Camera, t: &Tree, mvp: &Mat4, color: [f32; 4], out: &mut Vec<DrawCmd>) {
        let dy_floor = (32 * iso::TILE_SCALE) as f32 / 2.0;
        if let Some(tex) = &t.texture {
            let y1 = tex.offset_y;
            let y2 = (tex.height_orig() as f32 - dy_floor).min(tex.offset_y + tex.height as f32);
            let y3 = tex.offset_y + tex.height as f32;
            render_texture(cam, t, tex, false, y1, y2, mvp, color, out);
            render_texture(cam, t, tex, false, y2, y3, mvp, color, out);
        }
        if let Some(tex) = &t.texture2 {
            render_texture(cam, t, tex, true, tex.offset_y, tex.offset_y + tex.height as f32, mvp, color, out);
        }
    }
}

/// `FBORenderTrees.pushModelViewMatrix` for a still camera centred on its
/// character.
// The original writes the angles as float literals.
#[allow(clippy::approx_constant)]
fn model_view(cam: &Camera, ox: f32, oy: f32, oz: f32) -> Mat4 {
    let (cx, cy) = (cam.x, cam.y);
    let dif_x = (ox - cx) as f64;
    let dif_y = (oy - cy) as f64;
    let dif_z = ((oz - cam.z) * 2.44949) as f64;
    let use_angle = 2.356_194_5f32;
    Mat4::scaling(core_scale())
        .scale(iso::TILE_SCALE as f32 / 2.0, iso::TILE_SCALE as f32 / 2.0, iso::TILE_SCALE as f32 / 2.0)
        .rotate_x(0.523_598_8)
        .rotate_y(2.356_194_5)
        .translate(-(dif_x as f32), dif_z as f32, -(dif_y as f32))
        .scale(-1.0, 1.0, 1.0)
        .rotate_y(use_angle + std::f32::consts::PI)
        .translate(0.0, -0.719_999_97, 0.0)
}

/// `FBORenderTrees.renderTexture`: one textured slice of the billboard.
#[allow(clippy::too_many_arguments)]
fn render_texture(cam: &Camera, t: &Tree, tex: &Texture, second: bool, top: f32, bottom: f32, mvp: &Mat4, color: [f32; 4], out: &mut Vec<DrawCmd>) {
    let sqrt2 = std::f32::consts::SQRT_2;
    let (units_x, units_y) = match tex.width_orig() {
        w if w == JUMBO_L_WIDTH => (6, 8),
        w if w == JUMBO_XL_WIDTH => (10, 12),
        w if w == JUMBO_XXL_WIDTH => (14, 16),
        _ => (2, 4),
    };
    // calculateDepth: the square's depth less the projected origin's.
    let origin = mvp.transform([0.0, 0.0, 0.0]);
    let depth_buffer_value = origin[2];
    let mut target = iso::square_depth(fastfloor(cam.x), fastfloor(cam.y), t.x - 0.5, t.y - 0.5, t.z) - (depth_buffer_value + 1.0) / 2.0;
    if second {
        target -= 1.0e-4;
    }
    target += t.depth_offset as f32 * 2.0e-4;
    if top > tex.offset_y {
        target -= 0.0015;
    }
    let (wo, ho) = (tex.width_orig() as f32, tex.height_orig() as f32);
    let px = |p: f32| p / wo * (units_x as f32 / sqrt2);
    let py = |p: f32| p / ho * units_y as f32;
    let (x1, x2) = (tex.offset_x, tex.offset_x + tex.width as f32);
    let dx = units_x as f32 / sqrt2 / 2.0;
    let x_l = px(x1) - dx;
    let x_r = px(x2) - dx;
    let y_t = units_y as f32 - py(top);
    let y_b = py(ho - bottom);
    let y_start = tex.y_start + (top - tex.offset_y) / tex.height as f32 * (tex.y_end - tex.y_start);
    let y_end = tex.y_start + (bottom - tex.offset_y) / tex.height as f32 * (tex.y_end - tex.y_start);
    let k = 0.816_496_7f32;
    let corners = [[x_l, y_t * k], [x_r, y_t * k], [x_r, y_b * k], [x_l, y_b * k]];
    let pos = corners.map(|[x, y]| {
        let c = mvp.transform([x, y, 0.0]);
        let clip = (c[2] + 1.0) / 2.0 + target;
        [c[0], c[1], clip]
    });
    let uv = [[tex.x_start, y_start], [tex.x_end, y_start], [tex.x_end, y_end], [tex.x_start, y_end]];
    let col = crate::color::to_abgr(color[0], color[1], color[2], color[3]);
    out.push(DrawCmd::ProjectedQuad { texture: tex.clone(), pos, uv, color: col, nearest: true });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stagger_offsets_alternate() {
        let mut s = DepthStagger::default();
        assert_eq!(s.add_billboard(10, 0, 100), 0);
        assert_eq!(s.add_billboard(10, 50, 100), 1);
        assert_eq!(s.add_billboard(10, 60, 100), -1);
        assert_eq!(s.add_billboard(10, 500, 100), 0);
        assert_eq!(s.add_billboard(11, 0, 100), 0);
    }

    #[test]
    fn ortho_and_rotation() {
        let p = Mat4::ortho(-1.0, 1.0, -1.0, 1.0, -10.0, 10.0);
        assert_eq!(p.transform([0.5, -0.5, 0.0]), [0.5, -0.5, 0.0, 1.0]);
        let r = Mat4::identity().rotate_y(std::f32::consts::FRAC_PI_2).transform([1.0, 0.0, 0.0]);
        assert!((r[2] + 1.0).abs() < 1e-6 && r[0].abs() < 1e-6, "{r:?}");
    }
}
