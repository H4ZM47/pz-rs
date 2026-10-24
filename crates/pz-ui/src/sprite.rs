//! The UI draw list: what `UIElement.Draw*` and `UIManager.render` record
//! (the `SpriteRenderer.instance` calls of the original).
//!
//! `pz_render::DrawCmd` only has axis-aligned quads, blend and scissor, so
//! the UI records the richer [`UiDrawCmd`] (texture sub-rectangles, free
//! quads, lines, text when no [`crate::TextRenderer`] is installed) and
//! [`SpriteRenderer::lower_into`] converts it for a `pz_render` backend.
//! All coordinates are absolute screen pixels, as in the Java.
//!
//! Stencil clipping (`UIElement.setStencilRect` and friends) is recorded as
//! scissor rectangles: the original increments a stencil level inside each
//! rectangle and tests for equality, which makes nested stencils the
//! intersection of their rectangles. `setStencilCircle` clips to the
//! circle's bounding box; `repaintStencilRect` (which widens the current
//! stencil) is recorded as [`UiDrawCmd::StencilRepaint`] for backends with
//! real stencil support and ignored when lowering.

use pz_render::{Blend, DrawCmd, DrawList, Rgba, Texture};
use pz_text::UIFont;
use std::rc::Rc;

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
    /// `SpriteRenderer.renderline`.
    Line { texture: Option<Texture>, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, color: Rgba },
    /// A string drawn while no text renderer was installed.
    Text { font: Option<UIFont>, x: f32, y: f32, zoom: f32, text: Rc<str>, color: Rgba, align: TextAlign, trimmed: bool },
    /// `repaintStencilRect`: mark `[x, y, w, h]` as inside the current
    /// stencil level.
    StencilRepaint([i32; 4]),
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
    clip_stack: Vec<[i32; 4]>,
    stencil_suspended: bool,
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
        self.clip_stack.clear();
        self.stencil_suspended = false;
    }

    /// Current scissor `[x, y, w, h]`, if any stencil is active.
    pub fn current_clip(&self) -> Option<[i32; 4]> {
        if self.stencil_suspended {
            None
        } else {
            self.clip_stack.last().copied()
        }
    }

    fn emit_clip(&mut self) {
        let c = self.current_clip();
        self.cmds.push(UiDrawCmd::Render(DrawCmd::Clip(c)));
    }

    /// `setStencilRect` / `setStencilCircle` (absolute, already truncated).
    pub fn push_stencil(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let r = match self.clip_stack.last() {
            Some(top) if self.stencil_level > 0 => intersect(*top, [x, y, w, h]),
            _ => [x, y, w, h],
        };
        if self.stencil_level == 0 {
            self.clip_stack.clear();
        }
        self.stencil_level += 1;
        self.clip_stack.push(r);
        self.stencil_suspended = false;
        self.emit_clip();
    }

    /// `clearStencilRect`.
    pub fn pop_stencil(&mut self) {
        if self.stencil_level > 0 {
            self.stencil_level -= 1;
            self.clip_stack.pop();
        }
        if self.stencil_level == 0 {
            self.clip_stack.clear();
            self.stencil_suspended = false;
        }
        self.emit_clip();
    }

    /// `suspendStencil` (stencil test off, level kept).
    pub fn suspend_stencil(&mut self) {
        self.stencil_suspended = true;
        self.emit_clip();
    }

    /// `resumeStencil`.
    pub fn resume_stencil(&mut self) {
        self.stencil_suspended = false;
        self.emit_clip();
    }

    /// `repaintStencilRect` (only while a stencil is active).
    pub fn repaint_stencil(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if self.stencil_level <= 0 {
            return;
        }
        self.cmds.push(UiDrawCmd::StencilRepaint([x, y, w, h]));
    }

    /// Best-effort conversion into a `pz_render` draw list. Free quads and
    /// lines become their bounding rectangles; text recorded without a text
    /// renderer is dropped.
    pub fn lower_into(&self, out: &mut DrawList) {
        for c in &self.cmds {
            match c {
                UiDrawCmd::Quad { texture, x, y, w, h, color, uv } => {
                    let t = match (texture, uv) {
                        (Some(t), Some(uv)) => Some(apply_sub_uv(t, *uv)),
                        (t, _) => t.clone(),
                    };
                    out.cmds.push(DrawCmd::Quad { texture: t, x: *x, y: *y, w: *w, h: *h, color: *color });
                }
                UiDrawCmd::Quad4 { texture, points, color } => {
                    let (x0, y0, x1, y1) = bounds(points.iter().map(|p| (p[0], p[1])));
                    out.cmds.push(DrawCmd::Quad { texture: texture.clone(), x: x0, y: y0, w: x1 - x0, h: y1 - y0, color: *color });
                }
                UiDrawCmd::Line { texture, x1, y1, x2, y2, thickness, color } => {
                    let half = thickness / 2.0;
                    let (bx0, by0, bx1, by1) = bounds([(*x1, *y1), (*x2, *y2)].into_iter());
                    let (bx0, by0, bx1, by1) = if (y1 - y2).abs() < f32::EPSILON {
                        (bx0, by0 - half, bx1, by1 + half)
                    } else if (x1 - x2).abs() < f32::EPSILON {
                        (bx0 - half, by0, bx1 + half, by1)
                    } else {
                        (bx0, by0, bx1, by1)
                    };
                    out.cmds.push(DrawCmd::Quad {
                        texture: texture.clone(),
                        x: bx0,
                        y: by0,
                        w: (bx1 - bx0).max(*thickness),
                        h: (by1 - by0).max(*thickness),
                        color: *color,
                    });
                }
                UiDrawCmd::Text { .. } | UiDrawCmd::StencilRepaint(_) => {}
                UiDrawCmd::Render(d) => out.cmds.push(d.clone()),
            }
        }
    }
}

fn bounds(points: impl Iterator<Item = (f32, f32)>) -> (f32, f32, f32, f32) {
    let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in points {
        b.0 = b.0.min(x);
        b.1 = b.1.min(y);
        b.2 = b.2.max(x);
        b.3 = b.3.max(y);
    }
    b
}

fn intersect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    [x0, y0, (x1 - x0).max(0), (y1 - y0).max(0)]
}

/// Narrow `t.uv` to the texture-local normalised `[u0, v0, u1, v1]`.
///
/// ASSUMPTION (to reconcile with pz-render): `Texture::uv` is
/// `[x, y, w, h]` in page pixels. A zero `uv` is treated as the whole
/// `width` x `height` image.
pub fn apply_sub_uv(t: &Texture, uv: [f32; 4]) -> Texture {
    let mut r = t.clone();
    let [bx, by, mut bw, mut bh] = t.uv;
    if bw == 0.0 && bh == 0.0 {
        bw = t.width as f32;
        bh = t.height as f32;
    }
    r.uv = [bx + uv[0] * bw, by + uv[1] * bh, (uv[2] - uv[0]) * bw, (uv[3] - uv[1]) * bh];
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_stencils_intersect_and_unwind() {
        let mut s = SpriteRenderer::default();
        s.push_stencil(0, 0, 100, 100);
        s.push_stencil(50, 50, 100, 100);
        assert_eq!(s.current_clip(), Some([50, 50, 50, 50]));
        s.pop_stencil();
        assert_eq!(s.current_clip(), Some([0, 0, 100, 100]));
        s.pop_stencil();
        assert_eq!(s.current_clip(), None);
        s.pop_stencil();
        assert_eq!(s.stencil_level, 0);
    }
}
