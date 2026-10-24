//! One world quad as `TextureDraw` holds it while the `Consumer<TextureDraw>`
//! modifiers (`FloorShaper`, `WallShaper`, `TileDepthModifier`, …) adjust
//! it, and those modifiers.

use pz_render::{DrawCmd, Rgba, Sprite, Texture, WorldShader};

use crate::color;

/// The fields of `TextureDraw` the world modifiers touch.
#[derive(Clone, Debug)]
pub struct TexDraw {
    pub sprite: Sprite,
    /// `tex1` and its corner coordinates (`tex1U0..3`, `tex1V0..3`).
    pub tex1: Option<Texture>,
    pub uv1: [[f32; 2]; 4],
}

impl TexDraw {
    /// `SpriteRenderer.render(tex, x, y, w, h, r, g, b, a, modifier)` before
    /// the modifier runs; `None` when nothing would be drawn.
    pub fn new(tex: &Texture, x: f32, y: f32, w: f32, h: f32, c: Rgba) -> Option<Self> {
        let mut dl = pz_render::DrawList::default();
        dl.render(Some(tex), x, y, w, h, c);
        match dl.cmds.pop()? {
            DrawCmd::Sprite(sprite) => Some(TexDraw { sprite, tex1: None, uv1: [[0.0; 2]; 4] }),
            _ => None,
        }
    }

    pub fn col(&self, i: usize) -> u32 {
        self.sprite.color[i]
    }

    pub fn set_col(&mut self, i: usize, c: u32) {
        self.sprite.color[i] = c;
    }

    /// The draw command, with `shader`'s depth texture coordinates taken
    /// from this draw.
    pub fn into_cmd(self, shader: WorldShader) -> DrawCmd {
        let shader = match shader {
            WorldShader::TileDepth { depth, front, far, opaque, .. } => WorldShader::TileDepth { depth, uv1: self.uv1, front, far, opaque },
            s => s,
        };
        DrawCmd::WorldSprite(self.sprite, shader)
    }
}

/// `FloorShaper` and its `DeDiamond`/`Diamond` variants: per-corner vertex
/// light blended over the quad colour.
#[derive(Clone, Copy, Debug, Default)]
pub struct FloorShaper {
    pub col: [u32; 4],
    pub tint: u32,
    pub shore: bool,
    pub water_depth: [f32; 4],
}

impl FloorShaper {
    /// `FloorShaper.accept`.
    pub fn accept(&self, d: &mut TexDraw) {
        for i in 0..4 {
            d.set_col(i, color::blend_bgr(d.col(i), self.col[i]));
        }
        if self.shore {
            for i in 0..4 {
                d.set_col(i, color::set_alpha(d.col(i), 1.0 - self.water_depth[i]));
            }
        }
        if self.tint != 0 {
            for i in 0..4 {
                d.set_col(i, color::tint_abgr(d.col(i), self.tint));
            }
        }
    }

    /// `FloorShaperDeDiamond.accept`: the diamond's corner lights moved to
    /// the rectangle's corners.
    pub fn accept_de_diamond(&self, d: &mut TexDraw) {
        let plain = FloorShaper { tint: 0, ..*self };
        plain.accept(d);
        let c = self.col;
        let rotated = [
            color::lerp_abgr(c[0], c[3], 0.5),
            color::lerp_abgr(c[1], c[0], 0.5),
            color::lerp_abgr(c[2], c[1], 0.5),
            color::lerp_abgr(c[3], c[2], 0.5),
        ];
        for (i, r) in rotated.into_iter().enumerate() {
            d.set_col(i, color::blend_bgr(d.col(i), r));
        }
        if self.tint != 0 {
            for i in 0..4 {
                d.set_col(i, color::tint_abgr(d.col(i), self.tint));
            }
        }
    }

    /// `FloorShaperDiamond.accept`: the quad reshaped into the floor
    /// diamond (`DiamondShaper`, with the chunk-texture texel padding).
    pub fn accept_diamond(&self, d: &mut TexDraw, texture_scale: i32) {
        self.accept(d);
        diamond(d, texture_scale);
    }
}

/// `DiamondShaper.accept` with `fboRenderChunk` on.
pub fn diamond(d: &mut TexDraw, texture_scale: i32) {
    let s = &mut d.sprite;
    let Some(tex) = s.texture.as_ref() else { return };
    let (dx, dy) = (texture_scale as f32, texture_scale as f32);
    let (du, dv) = (dx, dy);
    let x0 = s.x[0] - dx;
    let y0 = s.y[0] - dy;
    let x1 = s.x[1] + dx;
    let y1 = s.y[1] - dy;
    let y3 = s.y[3] + dy;
    let y2 = s.y[2] + dy;
    let width = x1 - x0;
    let height = y2 - y1;
    let x_half = x0 + width * 0.5;
    let y_half = y1 + height * 0.5;
    let reshape = |u: [f32; 4], v: [f32; 4], one_u: f32, one_v: f32| {
        let u0 = u[0] - one_u * du;
        let v0 = v[0] - one_v * dv;
        let u1 = u[1] + one_u * du;
        let v1 = v[1] - one_v * dv;
        let v2 = v[2] + one_v * dv;
        let v3 = v[3] + one_v * dv;
        let u_half = u0 + (u1 - u0) * 0.5;
        let v_half = v1 + (v2 - v0) * 0.5;
        ([u_half, u1, u_half, u0], [v0, v_half, v3, v_half])
    };
    let (u, v) = reshape(s.u, s.v, 1.0 / tex.width_hw as f32, 1.0 / tex.height_hw as f32);
    s.x = [x_half, x1, x_half, x0];
    s.y = [y0, y_half, y3, y_half];
    s.u = u;
    s.v = v;
    if let Some(t1) = d.tex1.as_ref() {
        let uu = d.uv1.map(|c| c[0]);
        let vv = d.uv1.map(|c| c[1]);
        let (u, v) = reshape(uu, vv, 1.0 / t1.width_hw as f32, 1.0 / t1.height_hw as f32);
        d.uv1 = std::array::from_fn(|i| [u[i], v[i]]);
    }
}

/// Which part of a wall quad a `WallShaper` keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallPart {
    /// `WallShaperWhole`.
    Whole,
    /// `WallShaperN`: the right half.
    N,
    /// `WallShaperW`: the left half.
    W,
    /// `WallShaperSliceN`: a 5-pixel strip at the left.
    SliceN,
    /// `WallShaperSliceW`: a 7-pixel strip at the right.
    SliceW,
}

/// `WallShaper`: per-corner wall light.
#[derive(Clone, Copy, Debug)]
pub struct WallShaper {
    pub col: [u32; 4],
    pub tint: u32,
    pub part: WallPart,
}

impl WallShaper {
    pub fn accept(&self, d: &mut TexDraw) {
        for i in 0..4 {
            d.set_col(i, color::blend_bgr(d.col(i), self.col[i]));
        }
        if self.tint != 0 {
            for i in 0..4 {
                d.set_col(i, color::tint_abgr(d.col(i), self.tint));
            }
        }
        let half = |a: f32, b: f32| a * 0.5 + b * 0.5;
        let s = &mut d.sprite;
        let width_hw = |t: Option<&Texture>| t.map_or(1.0, |t| t.width_hw as f32);
        match self.part {
            WallPart::Whole => {}
            WallPart::N => {
                s.x[0] = half(s.x[0], s.x[1]);
                s.x[3] = half(s.x[2], s.x[3]);
                s.u[0] = half(s.u[0], s.u[1]);
                s.u[3] = half(s.u[2], s.u[3]);
                if d.tex1.is_some() {
                    d.uv1[0][0] = half(d.uv1[0][0], d.uv1[1][0]);
                    d.uv1[3][0] = half(d.uv1[2][0], d.uv1[3][0]);
                }
            }
            WallPart::W => {
                s.x[1] = half(s.x[0], s.x[1]);
                s.x[2] = half(s.x[2], s.x[3]);
                s.u[1] = half(s.u[0], s.u[1]);
                s.u[2] = half(s.u[2], s.u[3]);
                if d.tex1.is_some() {
                    d.uv1[1][0] = half(d.uv1[0][0], d.uv1[1][0]);
                    d.uv1[2][0] = half(d.uv1[2][0], d.uv1[3][0]);
                }
            }
            WallPart::SliceN => {
                let frac = 5.0 / width_hw(s.texture.as_ref());
                s.x[1] = s.x[0] + 5.0;
                s.x[2] = s.x[3] + 5.0;
                s.u[1] = s.u[0] + frac;
                s.u[2] = s.u[3] + frac;
                if let Some(t1) = d.tex1.as_ref() {
                    let frac = 5.0 / t1.width_hw as f32;
                    d.uv1[1][0] = d.uv1[0][0] + frac;
                    d.uv1[2][0] = d.uv1[3][0] + frac;
                }
            }
            WallPart::SliceW => {
                let frac = 7.0 / width_hw(s.texture.as_ref());
                s.x[0] = s.x[1] - 7.0;
                s.x[3] = s.x[2] - 7.0;
                s.u[0] = s.u[1] - frac;
                s.u[3] = s.u[2] - frac;
                if let Some(t1) = d.tex1.as_ref() {
                    let frac = 7.0 / t1.width_hw as f32;
                    d.uv1[0][0] = d.uv1[1][0] - frac;
                    d.uv1[3][0] = d.uv1[2][0] - frac;
                }
            }
        }
    }
}

/// A depth texture as `TileDepthModifier` uses it: the texture, its trim
/// offset and its image-space size.
#[derive(Clone, Debug)]
pub struct DepthTexture {
    pub texture: Texture,
}

/// `TileDepthModifier.accept`: depth-texture coordinates for the part of
/// the sprite's texture that overlaps the depth texture. `soff` is the
/// sprite's `soffX/soffY`, `scale` the sprite instance scale.
pub fn tile_depth_modifier(d: &mut TexDraw, depth: &Texture, soff: (i32, i32), scale: (f32, f32)) {
    let Some(tex0) = d.sprite.texture.as_ref() else { return };
    let (sx, sy) = (soff.0 as f32, soff.1 as f32);
    let x0 = (sx + tex0.offset_x * scale.0).max(depth.offset_x);
    let x1 = (sx + (tex0.offset_x + tex0.width as f32) * scale.0).min(depth.offset_x + depth.width as f32);
    let y0 = (sy + tex0.offset_y * scale.1).max(depth.offset_y);
    let y2 = (sy + (tex0.offset_y + tex0.height as f32) * scale.1).min(depth.offset_y + depth.height as f32);
    let (x2, x3, y1, y3) = (x1, x0, y0, y2);
    let (xs, ys) = (depth.x_start, depth.y_start);
    let (whw, hhw) = (depth.width_hw as f32, depth.height_hw as f32);
    let u = |x: f32| xs + (x - depth.offset_x) / whw;
    let v = |y: f32| ys + (y - depth.offset_y) / hhw;
    d.uv1 = [[u(x0), v(y0)], [u(x1), v(y1)], [u(x2), v(y2)], [u(x3), v(y3)]];
    d.tex1 = Some(depth.clone());
}
