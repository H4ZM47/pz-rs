//! The UI draw list: what `UIElement.Draw*` and `UIManager.render` record
//! (the `SpriteRenderer.instance` calls of the original).
//!
//! The UI records [`UiDrawCmd`]s (texture sub-rectangles, free quads,
//! lines, text when no [`crate::TextRenderer`] is installed) and
//! [`SpriteRenderer::lower_into`] converts them, without loss, into a
//! `pz_render` draw list. All coordinates are absolute screen pixels, as in
//! the Java.
//!
//! Stencil clipping (`UIElement.setStencilRect` and friends) is recorded as
//! the `IndieGL` stencil, colour-mask and alpha-test calls the original
//! makes: each rectangle increments the stencil inside it and drawing
//! tests for equality with the level, so nested stencils intersect.

use pz_render::{Blend, DrawCmd, DrawList, Rgba, Texture};
use pz_text::UIFont;
use std::rc::Rc;

const GL_EQUAL: u32 = 514;
const GL_ALWAYS: u32 = 519;
const GL_KEEP: u32 = 7680;
const GL_REPLACE: u32 = 7681;
const GL_INCR: u32 = 7682;
const GL_STENCIL_TEST: u32 = 2960;
const GL_ALPHA_TEST: u32 = 3008;

/// Horizontal alignment of `TextManager.DrawString*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    /// `DrawString` / `DrawStringUntrimmed`.
    Left,
    /// `DrawStringCentre`.
    Centre,
    /// `DrawStringRight`.
    Right,
}

#[derive(Clone, Debug)]
pub enum UiDrawCmd {
    /// `SpriteRenderer.render/renderi` of an axis-aligned rectangle.
    /// `texture: None` draws a solid rectangle. `uv` is the sampled part of
    /// the texture as `[u0, v0, u1, v1]`, normalised to the texture itself
    /// (0..1 inside its own sub-rectangle, `u0 > u1` for a horizontal flip);
    /// `None` samples the whole texture.
    Quad { texture: Option<Texture>, x: f32, y: f32, w: f32, h: f32, color: Rgba, uv: Option<[f32; 4]> },
    /// `SpriteRenderer.render` with four free corners (top-left, top-right,
    /// bottom-right, bottom-left) and `renderPoly`.
    Quad4 { texture: Option<Texture>, points: [[f32; 2]; 4], color: Rgba },
    /// `SpriteRenderer.renderPoly(tex, x1..y4, r, g, b, a, u1..v4)`: four
    /// corners with their texture coordinates (in the texture's atlas, not
    /// normalised).
    Poly { texture: Option<Texture>, points: [[f32; 2]; 4], color: Rgba, uv: [[f32; 2]; 4] },
    /// `SpriteRenderer.renderline`.
    Line { texture: Option<Texture>, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, color: Rgba },
    /// A string drawn while no text renderer was installed.
    Text { font: Option<UIFont>, x: f32, y: f32, zoom: f32, text: Rc<str>, color: Rgba, align: TextAlign, trimmed: bool },
    /// Commands `pz_render` already understands (blend, scissor).
    Render(DrawCmd),
}

/// `SpriteRenderer.instance` for the UI, plus the stencil state that
/// `UIElement.stencilLevel` tracks.
#[derive(Default)]
pub struct SpriteRenderer {
    pub cmds: Vec<UiDrawCmd>,
    /// `UIElement.stencilLevel`.
    pub stencil_level: i32,
}

impl SpriteRenderer {
    pub fn clear(&mut self) {
        self.cmds.clear();
    }

    pub fn push(&mut self, cmd: UiDrawCmd) {
        self.cmds.push(cmd);
    }

    /// `SpriteRenderer.renderi(tex, x, y, w, h, r, g, b, a, null)`.
    pub fn renderi(&mut self, tex: Option<&Texture>, x: i32, y: i32, w: i32, h: i32, color: Rgba) {
        self.render(tex, x as f32, y as f32, w as f32, h as f32, color, None);
    }

    /// `SpriteRenderer.render(tex, x, y, w, h, r, g, b, a, [uvs])`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, tex: Option<&Texture>, x: f32, y: f32, w: f32, h: f32, color: Rgba, uv: Option<[f32; 4]>) {
        self.cmds.push(UiDrawCmd::Quad { texture: tex.cloned(), x, y, w, h, color, uv });
    }

    /// `SpriteRenderer.renderPoly(tex, x1..y4, r, g, b, a, u1..v4)`.
    pub fn render_poly_uv(&mut self, tex: Option<&Texture>, points: [[f32; 2]; 4], color: Rgba, uv: [[f32; 2]; 4]) {
        self.cmds.push(UiDrawCmd::Poly { texture: tex.cloned(), points, color, uv });
    }

    pub fn render4(&mut self, tex: Option<&Texture>, points: [[f32; 2]; 4], color: Rgba) {
        self.cmds.push(UiDrawCmd::Quad4 { texture: tex.cloned(), points, color });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn renderline(&mut self, tex: Option<&Texture>, x1: i32, y1: i32, x2: i32, y2: i32, color: Rgba, thickness: f32) {
        self.cmds.push(UiDrawCmd::Line {
            texture: tex.cloned(),
            x1: x1 as f32,
            y1: y1 as f32,
            x2: x2 as f32,
            y2: y2 as f32,
            thickness,
            color,
        });
    }

    pub fn set_blend(&mut self, b: Blend) {
        self.cmds.push(UiDrawCmd::Render(DrawCmd::SetBlend(b)));
    }

    /// `UIManager.render` resets `UIElement.stencilLevel = 0`.
    pub fn reset_stencil(&mut self) {
        self.stencil_level = 0;
    }

    fn gl(&mut self, c: DrawCmd) {
        self.cmds.push(UiDrawCmd::Render(c));
    }

    /// `setStencilRect` / `setStencilCircle` (absolute, already truncated):
    /// raise the stencil by one where the quad (or the circle texture's
    /// opaque pixels) lands, then draw only where it equals the new level.
    pub fn push_stencil(&mut self, x: i32, y: i32, w: i32, h: i32, circle: Option<&Texture>) {
        self.gl(DrawCmd::StencilMask(255));
        self.gl(DrawCmd::GlEnable(GL_STENCIL_TEST));
        self.gl(DrawCmd::GlEnable(GL_ALPHA_TEST));
        self.gl(DrawCmd::StencilFunc(GL_EQUAL, self.stencil_level, 255));
        self.stencil_level += 1;
        self.gl(DrawCmd::StencilOp(GL_KEEP, GL_KEEP, GL_INCR));
        self.gl(DrawCmd::ColorMask(false, false, false, false));
        self.renderi(circle, x, y, w, h, Rgba(1.0, 0.0, 0.0, 1.0));
        self.gl(DrawCmd::ColorMask(true, true, true, true));
        self.gl(DrawCmd::StencilOp(GL_KEEP, GL_KEEP, GL_KEEP));
        self.gl(DrawCmd::StencilFunc(GL_EQUAL, self.stencil_level, 255));
    }

    /// `clearStencilRect`.
    pub fn pop_stencil(&mut self) {
        if self.stencil_level > 0 {
            self.stencil_level -= 1;
        }
        if self.stencil_level > 0 {
            self.gl(DrawCmd::StencilFunc(GL_EQUAL, self.stencil_level, 255));
        } else {
            self.gl(DrawCmd::AlphaFunc(GL_ALWAYS, 0.0));
            self.gl(DrawCmd::GlDisable(GL_STENCIL_TEST));
            self.gl(DrawCmd::GlDisable(GL_ALPHA_TEST));
            self.gl(DrawCmd::StencilFunc(GL_ALWAYS, 255, 255));
            self.gl(DrawCmd::StencilOp(GL_KEEP, GL_KEEP, GL_KEEP));
            // GL_STENCIL_BUFFER_BIT | GL_DEPTH_BUFFER_BIT
            self.gl(DrawCmd::Clear(1280));
        }
    }

    /// `suspendStencil` (stencil and alpha test off, level kept).
    pub fn suspend_stencil(&mut self) {
        self.gl(DrawCmd::GlDisable(GL_STENCIL_TEST));
        self.gl(DrawCmd::GlDisable(GL_ALPHA_TEST));
    }

    /// `resumeStencil`.
    pub fn resume_stencil(&mut self) {
        self.gl(DrawCmd::GlEnable(GL_STENCIL_TEST));
        self.gl(DrawCmd::GlEnable(GL_ALPHA_TEST));
    }

    /// `repaintStencilRect` (only while a stencil is active): set the
    /// rectangle to the current level.
    pub fn repaint_stencil(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if self.stencil_level <= 0 {
            return;
        }
        self.gl(DrawCmd::StencilFunc(GL_ALWAYS, self.stencil_level, 255));
        self.gl(DrawCmd::StencilOp(GL_KEEP, GL_KEEP, GL_REPLACE));
        self.gl(DrawCmd::ColorMask(false, false, false, false));
        self.renderi(None, x, y, w, h, Rgba(1.0, 0.0, 0.0, 1.0));
        self.gl(DrawCmd::ColorMask(true, true, true, true));
        self.gl(DrawCmd::StencilOp(GL_KEEP, GL_KEEP, GL_KEEP));
        self.gl(DrawCmd::StencilFunc(GL_EQUAL, self.stencil_level, 255));
    }

    /// Converts into a `pz_render` draw list. Text recorded without a text
    /// renderer is dropped; everything else maps one to one.
    pub fn lower_into(&self, out: &mut DrawList) {
        for c in &self.cmds {
            match c {
                UiDrawCmd::Quad { texture, x, y, w, h, color, uv: None } => out.render(texture.as_ref(), *x, *y, *w, *h, *color),
                UiDrawCmd::Quad { texture, x, y, w, h, color, uv: Some(uv) } => {
                    // SpriteRenderer.render(..., u1, v1, ..., u4, v4) with the
                    // texture-local rectangle mapped into the texture's region.
                    let (u, v) = match texture {
                        Some(t) => {
                            let (su, sv) = (t.x_end - t.x_start, t.y_end - t.y_start);
                            let u0 = t.x_start + uv[0] * su;
                            let u1 = t.x_start + uv[2] * su;
                            let v0 = t.y_start + uv[1] * sv;
                            let v1 = t.y_start + uv[3] * sv;
                            ([u0, u1, u1, u0], [v0, v0, v1, v1])
                        }
                        None => ([0.0, 1.0, 1.0, 0.0], [0.0, 0.0, 1.0, 1.0]),
                    };
                    out.render_uv(texture.as_ref(), *x, *y, *w, *h, *color, u, v);
                }
                UiDrawCmd::Quad4 { texture, points, color } => {
                    out.render4(texture.as_ref(), points.map(|p| p[0]), points.map(|p| p[1]), *color);
                }
                UiDrawCmd::Poly { texture, points, color, uv } => {
                    out.render_poly_uv(texture.as_ref(), points.map(|p| p[0]), points.map(|p| p[1]), *color, uv.map(|p| p[0]), uv.map(|p| p[1]));
                }
                UiDrawCmd::Line { texture, x1, y1, x2, y2, thickness, color } => {
                    out.render_line(texture.as_ref(), *x1, *y1, *x2, *y2, *color, *thickness);
                }
                UiDrawCmd::Text { .. } => {}
                UiDrawCmd::Render(d) => out.cmds.push(d.clone()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_with_uvs_lowers_to_its_coordinates() {
        let mut s = SpriteRenderer::default();
        let mut t = pz_render::Texture::empty("t");
        t.ready = true;
        let pts = [[0.0, 0.0], [10.0, 0.0], [5.0, 5.0], [5.0, 5.0]];
        let uv = [[0.5, 0.125], [0.75, 0.125], [0.5, 0.3], [0.5, 0.3]];
        s.render_poly_uv(Some(&t), pts, Rgba(1.0, 1.0, 1.0, 1.0), uv);
        // A texture that is not ready draws nothing (renderPoly's check).
        s.render_poly_uv(Some(&pz_render::Texture::empty("t")), pts, Rgba(1.0, 1.0, 1.0, 1.0), uv);
        let mut dl = DrawList::default();
        s.lower_into(&mut dl);
        let sprites: Vec<_> = dl.cmds.iter().filter_map(|c| match c {
            DrawCmd::Sprite(sp) => Some(sp),
            _ => None,
        }).collect();
        assert_eq!(sprites.len(), 1);
        assert_eq!(sprites[0].u, [0.5, 0.75, 0.5, 0.5]);
        assert_eq!(sprites[0].v, [0.125, 0.125, 0.3, 0.3]);
        assert_eq!(sprites[0].x, [0.0, 10.0, 5.0, 5.0]);
    }

    #[test]
    fn nested_stencils_record_gl_and_unwind() {
        let mut s = SpriteRenderer::default();
        s.push_stencil(0, 0, 100, 100, None);
        s.push_stencil(50, 50, 100, 100, None);
        assert_eq!(s.stencil_level, 2);
        let funcs: Vec<i32> = s
            .cmds
            .iter()
            .filter_map(|c| match c {
                UiDrawCmd::Render(DrawCmd::StencilFunc(GL_EQUAL, r, _)) => Some(*r),
                _ => None,
            })
            .collect();
        assert_eq!(funcs, [0, 1, 1, 2]);
        s.pop_stencil();
        s.pop_stencil();
        assert!(matches!(s.cmds.last(), Some(UiDrawCmd::Render(DrawCmd::Clear(1280)))));
        s.pop_stencil();
        assert_eq!(s.stencil_level, 0);
        let mut dl = DrawList::default();
        s.lower_into(&mut dl);
        let quads = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::Sprite(_))).count();
        assert_eq!(quads, 2);
    }
}
