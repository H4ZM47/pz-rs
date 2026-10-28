//! `UIElement.Draw*`, text drawing and stencil clipping.
//!
//! Coordinates are local to the element; each method adds
//! `getAbsoluteX/Y()` (and usually the element's own scroll) exactly as the
//! Java does, including its `int` truncations and off-screen culling
//! (`top + h < 0 || top > 4096`). Commands go to the manager's
//! [`crate::SpriteRenderer`].
//!
//! Texture UVs are texture-local (`getXStart() == 0`, `getXEnd() == 1`);
//! mapping into the pack page happens when lowering (see
//! [`crate::SpriteRenderer::lower_into`]).

use crate::element::UIElement;
use crate::manager::with_ui;
use crate::sprite::{SpriteRenderer, TextAlign, UiDrawCmd};
use pz_render::{Rgba, Texture};
use pz_text::UIFont;
use std::rc::Rc;

/// Resolved inputs for one `UIElement.DrawItemIcon` call. The caller reads
/// the item's actual texture handles and fluid component before drawing;
/// this value does not own or replace those game objects.
#[derive(Clone, Copy, Debug)]
pub struct InventoryItemIcon<'a> {
    pub texture: Option<&'a Texture>,
    /// `InventoryItem.getR/getG/getB` (`col`, not `colorRed/Green/Blue`).
    pub color: [f32; 3],
    pub color_mask: Option<&'a Texture>,
    pub fluid_mask: Option<&'a Texture>,
    /// The item's fluid component, or its actual world item's component
    /// when the item itself has none.
    pub fluid: Option<InventoryItemIconFluid>,
}

/// The fluid reads consumed by a single icon draw. Division happens as
/// `float`, as in the original, before the mask's double geometry.
#[derive(Clone, Copy, Debug)]
pub struct InventoryItemIconFluid {
    pub color: [f32; 3],
    pub amount: f32,
    pub capacity: f32,
}

fn rgba(r: f64, g: f64, b: f64, a: f64) -> Rgba {
    Rgba(r as f32, g as f32, b as f32, a as f32)
}

fn sprite<R>(f: impl FnOnce(&mut SpriteRenderer) -> R) -> R {
    with_ui(|m| f(&mut m.sprite))
}

fn tx(t: Option<&Texture>) -> Option<&Texture> {
    t
}

/// `PZMath.clamp(v, lo, hi)`.
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// The texture-local UVs of the sub-rectangle `(sx, sy, sw, sh)` (pixels),
/// clamped to the texture like `DrawSubTextureRGBA`.
fn sub_uv(t: &Texture, sx: f64, sy: f64, sw: f64, sh: f64) -> [f32; 4] {
    let (w, h) = ((t.width as i32) as f32, (t.height as i32) as f32);
    let fx = clamp(sx as f32, 0.0, w);
    let fy = clamp(sy as f32, 0.0, h);
    let fw = clamp((fx as f64 + sw) as f32, 0.0, w) - fx;
    let fh = clamp((fy as f64 + sh) as f32, 0.0, h) - fy;
    [fx / w, fy / h, (fx + fw) / w, (fy + fh) / h]
}

/// `TextManager.DrawString*` through the installed renderer, or recorded
/// as [`UiDrawCmd::Text`] when there is none.
#[allow(clippy::too_many_arguments)]
pub fn draw_string(font: Option<UIFont>, x: f64, y: f64, zoom: f32, text: &str, color: Rgba, align: TextAlign, trimmed: bool) {
    with_ui(|m| match m.text.clone() {
        Some(tr) => tr.draw(&mut m.sprite, font, x, y, zoom, text, color, align, trimmed),
        None => m.sprite.push(UiDrawCmd::Text {
            font,
            x: x as f32,
            y: y as f32,
            zoom,
            text: Rc::from(text),
            color,
            align,
            trimmed,
        }),
    })
}

/// `TextManager.MeasureStringX`.
pub fn measure_string_x(font: Option<UIFont>, s: &str) -> i32 {
    with_ui(|m| m.text.clone()).map_or(0, |t| t.measure_x(font, s))
}

/// `TextManager.MeasureStringY`.
pub fn measure_string_y(font: Option<UIFont>, s: &str) -> i32 {
    with_ui(|m| m.text.clone()).map_or(0, |t| t.measure_y(font, s))
}

/// `TextManager.getFontFromEnum(font).getWidth(s, start, end, xadvance)`.
pub fn font_width_range(font: Option<UIFont>, s: &str, start: i32, end: i32, xadvance: bool) -> i32 {
    with_ui(|m| m.text.clone()).map_or(0, |t| t.width_range(font, s, start, end, xadvance))
}

/// `TextManager.getFontHeight`.
pub fn font_height(font: Option<UIFont>) -> i32 {
    with_ui(|m| m.text.clone()).map_or(0, |t| t.font_height(font))
}

impl UIElement {
    fn abs_scrolled(&self) -> (f64, f64) {
        let (xs, ys) = {
            let s = self.st();
            (s.x_scroll, s.y_scroll)
        };
        (self.get_absolute_x() + xs, self.get_absolute_y() + ys)
    }

    fn abs(&self) -> (f64, f64) {
        (self.get_absolute_x(), self.get_absolute_y())
    }

    fn scrolls(&self) -> (f64, f64) {
        let s = self.st();
        (s.x_scroll, s.y_scroll)
    }

    // ---- text ----

    /// `DrawText(UIFont, String, x, y, zoom, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_zoom(&self, font: Option<UIFont>, text: &str, x: f64, y: f64, zoom: f64, r: f64, g: f64, b: f64, a: f64) {
        let (ax, ay) = self.abs_scrolled();
        draw_string(font, x + ax, y + ay, zoom as f32, text, rgba(r, g, b, a), TextAlign::Left, true);
    }

    /// `DrawText(String, x, y, r, g, b, a)` and the `(…, width, height, …)`
    /// overload, which ignores the size: default font.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_default(&self, text: &str, x: f64, y: f64, r: f64, g: f64, b: f64, a: f64) {
        let (ax, ay) = self.abs_scrolled();
        draw_string(None, x + ax, y + ay, 1.0, text, rgba(r, g, b, a), TextAlign::Left, true);
    }

    /// `DrawText(UIFont, String, x, y, r, g, b, a)`: skipped when the line
    /// is far off screen.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(&self, font: Option<UIFont>, text: Option<&str>, x: f64, y: f64, r: f64, g: f64, b: f64, a: f64) {
        let Some(text) = text else { return };
        let (ax, ay) = self.abs_scrolled();
        let top = (y + ay) as i32;
        if top + 100 < 0 || top > 4096 {
            return;
        }
        draw_string(font, x + ax, y + ay, 1.0, text, rgba(r, g, b, a), TextAlign::Left, true);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_untrimmed(&self, font: Option<UIFont>, text: Option<&str>, x: f64, y: f64, r: f64, g: f64, b: f64, a: f64) {
        let Some(text) = text else { return };
        let (ax, ay) = self.abs_scrolled();
        draw_string(font, x + ax, y + ay, 1.0, text, rgba(r, g, b, a), TextAlign::Left, false);
    }

    /// `DrawTextCentre([UIFont,] String, x, y, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_centre(&self, font: Option<UIFont>, text: &str, x: f64, y: f64, r: f64, g: f64, b: f64, a: f64) {
        let (ax, ay) = self.abs_scrolled();
        draw_string(font, x + ax, y + ay, 1.0, text, rgba(r, g, b, a), TextAlign::Centre, true);
    }

    /// `DrawTextRight([UIFont,] String, x, y, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_right(&self, font: Option<UIFont>, text: &str, x: f64, y: f64, r: f64, g: f64, b: f64, a: f64) {
        let (ax, ay) = self.abs_scrolled();
        draw_string(font, x + ax, y + ay, 1.0, text, rgba(r, g, b, a), TextAlign::Right, true);
    }

    /// `drawTextWithBackground(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_with_background(
        &self,
        font: Option<UIFont>,
        text: &str,
        x: f64,
        y: f64,
        c: [f64; 4],
        pad_x: f64,
        pad_y: f64,
        bg: [f64; 4],
    ) {
        let fh = font_height(font);
        let tw = measure_string_x(font, text);
        self.draw_texture_scaled_col(None, x - pad_x, y - pad_y, tw as f64 + pad_x * 2.0, fh as f64 + pad_y * 2.0, bg[0], bg[1], bg[2], bg[3]);
        self.draw_text(font, Some(text), x, y, c[0], c[1], c[2], c[3]);
    }

    // ---- textures ----

    /// `DrawTextureAngle(tex, centerX, centerY, angle, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_angle(&self, tex: Option<&Texture>, cx: f64, cy: f64, angle: f64, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let Some(t) = tex else { return };
        let dx = ((t.width as i32) / 2) as f32 as f64;
        let dy = ((t.height as i32) / 2) as f32 as f64;
        let rad = (180.0 + angle).to_radians();
        let (xc, xs) = (rad.cos() * dx, rad.sin() * dx);
        let (yc, ys) = (rad.cos() * dy, rad.sin() * dy);
        let (ox, oy) = (self.get_absolute_x() + cx, self.get_absolute_y() + cy);
        let pts = [
            [xc - ys + ox, yc + xs + oy],
            [-xc - ys + ox, yc - xs + oy],
            [-xc + ys + ox, -yc - xs + oy],
            [xc + ys + ox, -yc + xs + oy],
        ];
        let pts = pts.map(|p| [p[0] as f32, p[1] as f32]);
        sprite(|s| s.render4(tx(tex), pts, rgba(r, g, b, a)));
    }

    /// `DrawTexture(tex, tlx, tly, trx, try, brx, bry, blx, bly, r, g, b, a)`:
    /// absolute corners, no element offset (as in the original).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_4(&self, tex: Option<&Texture>, p: [f64; 8], r: f64, g: f64, b: f64, a: f64) {
        let pts = [[p[0] as f32, p[1] as f32], [p[2] as f32, p[3] as f32], [p[4] as f32, p[5] as f32], [p[6] as f32, p[7] as f32]];
        sprite(|s| s.render4(tx(tex), pts, rgba(r, g, b, a)));
    }

    /// `DrawTexture(tex, x, y, alpha)`.
    pub fn draw_texture(&self, tex: Option<&Texture>, x: f64, y: f64, alpha: f64) {
        if !self.is_visible() {
            return;
        }
        let (mut dx, mut dy) = (x + self.get_absolute_x(), y + self.get_absolute_y());
        let (mut gh, mut w, mut h) = (0.0, 0.0, 0.0);
        if let Some(t) = tex {
            dx += t.offset_x as f64;
            dy += t.offset_y as f64;
            gh = (t.height as i32) as f64;
            w = (t.width as i32) as f64;
            h = (t.height as i32) as f64;
        }
        let (xs, ys) = self.scrolls();
        let top = (dy + ys) as i32;
        if top as f64 + gh < 0.0 || top > 4096 {
            return;
        }
        sprite(|s| s.renderi(tx(tex), (dx + xs) as i32, (dy + ys) as i32, w as i32, h as i32, rgba(1.0, 1.0, 1.0, alpha)));
    }

    /// `DrawTextureCol(tex, x, y, color)`.
    pub fn draw_texture_col(&self, tex: Option<&Texture>, x: f64, y: f64, col: Rgba) {
        if !self.is_visible() {
            return;
        }
        let (mut dx, mut dy) = (x + self.get_absolute_x(), y + self.get_absolute_y());
        let (mut w, mut h) = (0, 0);
        if let Some(t) = tex {
            dx += t.offset_x as f64;
            dy += t.offset_y as f64;
            w = t.width as i32;
            h = t.height as i32;
        }
        let (xs, ys) = self.scrolls();
        let top = (dy + ys) as i32;
        if top + h < 0 || top > 4096 {
            return;
        }
        sprite(|s| s.renderi(tx(tex), (dx + xs) as i32, (dy + ys) as i32, w, h, col));
    }

    /// `DrawTextureColor(tex, x, y, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_color(&self, tex: Option<&Texture>, x: f64, y: f64, r: f64, g: f64, b: f64, a: f64) {
        self.draw_texture_col(tex, x, y, rgba(r, g, b, a));
    }

    /// `DrawTextureScaled(tex, x, y, w, h, alpha)`.
    pub fn draw_texture_scaled(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, alpha: f64) {
        if !self.is_visible() {
            return;
        }
        let (dx, dy) = (x + self.get_absolute_x(), y + self.get_absolute_y());
        let (xs, ys) = self.scrolls();
        sprite(|s| s.renderi(tx(tex), (dx + xs) as i32, (dy + ys) as i32, w as i32, h as i32, rgba(1.0, 1.0, 1.0, alpha)));
    }

    /// `DrawTextureScaledUniform(tex, x, y, scale, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_scaled_uniform(&self, tex: Option<&Texture>, x: f64, y: f64, scale: f64, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let Some(t) = tex else { return };
        let dx = x + self.get_absolute_x() + t.offset_x as f64 * scale;
        let dy = y + self.get_absolute_y() + t.offset_y as f64 * scale;
        let (xs, ys) = self.scrolls();
        let (w, h) = (((t.width as i32) as f64 * scale) as i32, ((t.height as i32) as f64 * scale) as i32);
        sprite(|s| s.renderi(Some(t), (dx + xs) as i32, (dy + ys) as i32, w, h, rgba(r, g, b, a)));
    }

    /// Shared body of the three `DrawTextureScaledAspect*` variants.
    #[allow(clippy::too_many_arguments)]
    fn aspect(&self, t: &Texture, x: f64, y: f64, mut w: f64, mut h: f64, use_orig: bool, max: bool) -> (f64, f64, f64, f64) {
        let mut dx = x + self.get_absolute_x();
        let mut dy = y + self.get_absolute_y();
        if (t.width as i32) > 0 && (t.height as i32) > 0 && w > 0.0 && h > 0.0 {
            let (bw, bh) = if use_orig { (t.width_orig() as f64, t.height_orig() as f64) } else { ((t.width as i32) as f64, (t.height as i32) as f64) };
            let ratio = if max { (w / bw).max(h / bh) } else { (w / bw).min(h / bh) };
            let (ow, oh) = (w, h);
            w = (t.width as i32) as f64 * ratio;
            h = (t.height as i32) as f64 * ratio;
            dx -= (w - ow) / 2.0;
            dy -= (h - oh) / 2.0;
        }
        (dx, dy, w, h)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_scaled_aspect(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let Some(t) = tex else { return };
        let (dx, dy, w, h) = self.aspect(t, x, y, w, h, true, false);
        let (xs, ys) = self.scrolls();
        sprite(|s| s.renderi(Some(t), (dx + xs) as i32, (dy + ys) as i32, w as i32, h as i32, rgba(r, g, b, a)));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_scaled_aspect2(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let Some(t) = tex else { return };
        let (dx, dy, w, h) = self.aspect(t, x, y, w, h, false, false);
        let (xs, ys) = self.scrolls();
        sprite(|s| {
            s.render(Some(t), (dx + xs) as i32 as f32, (dy + ys) as i32 as f32, w as i32 as f32, h as i32 as f32, rgba(r, g, b, a), None)
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_scaled_aspect3(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let Some(t) = tex else { return };
        let (dx, dy, w, h) = self.aspect(t, x, y, w, h, true, true);
        let (xs, ys) = self.scrolls();
        sprite(|s| s.renderi(Some(t), (dx + xs) as i32, (dy + ys) as i32, w as i32, h as i32, rgba(r, g, b, a)));
    }

    /// `DrawTextureScaledCol(tex, x, y, w, h, r, g, b, a)` and
    /// `DrawTextureScaledColor` (`drawRect` in Lua uses a nil texture).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_scaled_col(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let (dx, dy) = (x + self.get_absolute_x(), y + self.get_absolute_y());
        let (xs, ys) = self.scrolls();
        let top = (dy + ys) as i32;
        if top as f64 + h < 0.0 || top > 4096 {
            return;
        }
        sprite(|s| s.renderi(tx(tex), (dx + xs) as i32, (dy + ys) as i32, w as i32, h as i32, rgba(r, g, b, a)));
    }

    /// `DrawTextureScaledCol(tex, x, y, w, h, Color)` (no culling).
    pub fn draw_texture_scaled_col_c(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, col: Rgba) {
        if !self.is_visible() {
            return;
        }
        let (dx, dy) = (x + self.get_absolute_x(), y + self.get_absolute_y());
        let (xs, ys) = self.scrolls();
        sprite(|s| s.render(tx(tex), (dx + xs) as i32 as f32, (dy + ys) as i32 as f32, w as i32 as f32, h as i32 as f32, col, None));
    }

    /// `DrawLine(tex, x1, y1, x2, y2, thickness, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_line(&self, tex: Option<&Texture>, x1: f64, y1: f64, x2: f64, y2: f64, thickness: f32, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let (ax, ay) = self.abs();
        let (xs, ys) = self.scrolls();
        let p = |v: f64, o: f64, s: f64| (v + o + s) as i32;
        sprite(|s| s.renderline(tx(tex), p(x1, ax, xs), p(y1, ay, ys), p(x2, ax, xs), p(y2, ay, ys), rgba(r, g, b, a), thickness));
    }

    /// `DrawPolygon(tex, x1, y1, ..., x4, y4, r, g, b, a)` (no visibility
    /// check in the original).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_polygon(&self, tex: Option<&Texture>, p: [f64; 8], r: f64, g: f64, b: f64, a: f64) {
        let (dx, dy) = self.abs_scrolled();
        let pts = [
            [(p[0] + dx) as f32, (p[1] + dy) as f32],
            [(p[2] + dx) as f32, (p[3] + dy) as f32],
            [(p[4] + dx) as f32, (p[5] + dy) as f32],
            [(p[6] + dx) as f32, (p[7] + dy) as f32],
        ];
        sprite(|s| s.render4(tx(tex), pts, rgba(r, g, b, a)));
    }

    /// Legacy binding entry until `DrawItemIcon` supplies the resolved
    /// item inputs to [`Self::draw_inventory_item_icon`].
    pub fn draw_item_icon(&self) {}

    /// `DrawItemIcon(item, x, y, alpha, width, height)` after resolving the
    /// native item's textures, tint and fluid reads. Texture descriptors
    /// must reflect the current shared handles, including offset changes.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_inventory_item_icon(
        &self,
        icon: &InventoryItemIcon<'_>,
        x: f64,
        y: f64,
        alpha: f64,
        width: f64,
        height: f64,
    ) {
        if !self.is_visible() {
            return;
        }
        let Some(texture) = icon.texture else { return };
        let [r, g, b] = if icon.color_mask.is_some() { [1.0; 3] } else { icon.color };
        if let (Some(fluid), Some(mask)) = (icon.fluid, icon.fluid_mask) {
            self.draw_texture_icon(Some(texture), x, y, width, height, r as f64, g as f64, b as f64, alpha);
            let ratio = (fluid.amount / fluid.capacity) as f64;
            let [r, g, b] = fluid.color;
            self.draw_texture_icon_mask(Some(mask), ratio, x, y, width, height, r as f64, g as f64, b as f64, alpha);
        } else {
            self.draw_texture_scaled_aspect(Some(texture), x, y, width, height, r as f64, g as f64, b as f64, alpha);
        }
        if let Some(mask) = icon.color_mask {
            let [r, g, b] = icon.color;
            self.draw_texture_icon_mask(Some(mask), 1.0, x, y, width, height, r as f64, g as f64, b as f64, alpha);
        }
    }

    /// `DrawScriptItemIcon(scriptItem, ...)`. TODO(world): needs script
    /// `Item`.
    pub fn draw_script_item_icon(&self) {}

    fn icon_fit(&self, t: &Texture, x: f64, y: f64, mut w: f64, mut h: f64) -> (f64, f64, f64, f64) {
        let mut dx = x + self.get_absolute_x();
        let mut dy = y + self.get_absolute_y();
        if (t.width as i32) > 0 && (t.height as i32) > 0 && w > 0.0 && h > 0.0 {
            let ratio = (w / t.width_orig() as f64).min(h / t.height_orig() as f64);
            let ox = (t.offset_x as f64 * ratio) as i32 as f64;
            let oy = (t.offset_y as f64 * ratio) as i32 as f64;
            w = ((t.width as i32) as f64 * ratio) as i32 as f64;
            h = ((t.height as i32) as f64 * ratio) as i32 as f64;
            dx += ox;
            dy += oy;
        }
        (dx, dy, w, h)
    }

    /// `DrawTextureIcon(tex, x, y, w, h, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_icon(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let Some(t) = tex else { return };
        let (dx, dy, w, h) = self.icon_fit(t, x, y, w, h);
        let (xs, ys) = self.scrolls();
        sprite(|s| s.renderi(Some(t), (dx + xs) as i32, (dy + ys) as i32, w as i32, h as i32, rgba(r, g, b, a)));
    }

    /// Bottom-up partial draw shared by `DrawTextureIconMask` and
    /// `DrawTexturePercentageBottomUp` (positions already resolved).
    #[allow(clippy::too_many_arguments)]
    fn bottom_up(&self, t: &Texture, ratio: f64, mut dx: f64, mut dy: f64, w: f64, mut h: f64, c: Rgba) {
        let hh = (h * ratio) as i32 as f64;
        dy += h - hh;
        h = hh;
        let th = ((t.height as i32) as f64 * ratio) as i32 as f64;
        let sub_y = (t.height as i32) as f64 - th;
        let (xs, ys) = self.scrolls();
        dx += xs;
        dy += ys;
        dx = dx as i32 as f64;
        dy = dy as i32 as f64;
        if dy + h < 0.0 || dy > 4096.0 {
            return;
        }
        let uv = sub_uv(t, 0.0, sub_y, (t.width as i32) as f64, th);
        sprite(|s| s.render(Some(t), dx as f32, dy as f32, w as f32, h as f32, c, Some(uv)));
    }

    /// `DrawTextureIconMask(tex, yRatio, x, y, w, h, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_icon_mask(&self, tex: Option<&Texture>, y_ratio: f64, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let Some(t) = tex else { return };
        let (dx, dy, w, h) = self.icon_fit(t, x, y, w, h);
        // `PZMath.max(0.15f, (float)yRatio)` returns its second argument
        // for NaN. Rust's f32::max would instead replace NaN with 0.15.
        let ratio = y_ratio as f32;
        let ratio = if 0.15f32 > ratio { 0.15f32 } else { ratio } as f64;
        self.bottom_up(t, ratio, dx, dy, w, h, rgba(r, g, b, a));
    }

    /// `DrawTexturePercentage(tex, ratio, x, y, w, h, r, g, b, a)`: left part.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_percentage(&self, tex: Option<&Texture>, ratio: f64, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let Some(t) = tex else { return };
        let (mut dx, mut dy) = (x + self.get_absolute_x(), y + self.get_absolute_y());
        let w = (w * ratio) as i32 as f64;
        let sub_w = (t.width as i32) as f64 * ratio;
        let (xs, ys) = self.scrolls();
        dx += xs;
        dy += ys;
        dx = dx as i32 as f64;
        dy = dy as i32 as f64;
        if dy + h < 0.0 || dy > 4096.0 {
            return;
        }
        let uv = sub_uv(t, 0.0, 0.0, sub_w, (t.height as i32) as f64);
        sprite(|s| s.render(Some(t), dx as f32, dy as f32, w as f32, h as f32, rgba(r, g, b, a), Some(uv)));
    }

    /// `DrawTexturePercentageBottomUp(tex, ratio, x, y, w, h, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_percentage_bottom_up(&self, tex: Option<&Texture>, ratio: f64, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        if !self.is_visible() {
            return;
        }
        let Some(t) = tex else { return };
        let (dx, dy) = (x + self.get_absolute_x(), y + self.get_absolute_y());
        self.bottom_up(t, ratio, dx, dy, w, h, rgba(r, g, b, a));
    }

    /// `DrawSubTextureRGBA(tex, subX, subY, subW, subH, x, y, w, h, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_sub_texture_rgba(&self, tex: Option<&Texture>, sub: [f64; 4], x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        let Some(t) = tex else { return };
        if !self.is_visible() || sub[2] <= 0.0 || sub[3] <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let (ax, ay) = self.abs_scrolled();
        let dx = x + ax + t.offset_x as f64;
        let dy = y + ay + t.offset_y as f64;
        if dy + h < 0.0 || dy > 4096.0 {
            return;
        }
        let uv = sub_uv(t, sub[0], sub[1], sub[2], sub[3]);
        sprite(|s| s.render(Some(t), dx as f32, dy as f32, w as f32, h as f32, rgba(r, g, b, a), Some(uv)));
    }

    /// `DrawTextureTiled(tex, x, y, w, h, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_tiled(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        let Some(t) = tex else { return };
        if !self.is_visible() || w <= 0.0 || h <= 0.0 {
            return;
        }
        let (tw, th) = ((t.width as i32) as f64, (t.height as i32) as f64);
        if tw <= 0.0 || th <= 0.0 {
            return;
        }
        let mut y1 = y;
        while y1 < y + h {
            let mut x1 = x;
            while x1 < x + w {
                let w1 = if x1 + tw > x + w { x + w - x1 } else { tw };
                let h1 = if y1 + th > y + h { y + h - y1 } else { th };
                self.draw_sub_texture_rgba(tex, [0.0, 0.0, w1, h1], x1, y1, w1, h1, r, g, b, a);
                x1 += tw;
            }
            y1 += th;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_tiled_x(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        let Some(t) = tex else { return };
        if !self.is_visible() || w <= 0.0 || h <= 0.0 {
            return;
        }
        let (tw, th) = ((t.width as i32) as f64, (t.height as i32) as f64);
        if tw <= 0.0 {
            return;
        }
        let mut x1 = x;
        while x1 < x + w {
            let w1 = if x1 + tw > x + w { x + w - x1 } else { tw };
            self.draw_sub_texture_rgba(tex, [0.0, 0.0, w1, th], x1, y, w1, th, r, g, b, a);
            x1 += tw;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_tiled_y(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        let Some(t) = tex else { return };
        if !self.is_visible() || w <= 0.0 || h <= 0.0 {
            return;
        }
        let (tw, th) = ((t.width as i32) as f64, (t.height as i32) as f64);
        if th <= 0.0 {
            return;
        }
        let mut y1 = y;
        while y1 < y + h {
            let h1 = if y1 + th > y + h { y + h - y1 } else { th };
            self.draw_sub_texture_rgba(tex, [0.0, 0.0, tw, h1], x, y1, tw, h1, r, g, b, a);
            y1 += th;
        }
    }

    /// `DrawTextureTiledYOffset`: vertical tiling continuing the pattern
    /// from absolute `y` (`y % texHeight`).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_tiled_y_offset(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, r: f64, g: f64, b: f64, a: f64) {
        let Some(t) = tex else { return };
        if !self.is_visible() || w <= 0.0 || h <= 0.0 {
            return;
        }
        let th = (t.height as i32) as f64;
        if th <= 0.0 {
            return;
        }
        let mut sub_y = y % th;
        let mut y1 = y;
        while y1 < y + h {
            let mut h1 = th - sub_y;
            if h1 == 0.0 {
                sub_y = 0.0;
                h1 = th;
            }
            if y1 + h1 > y + h {
                h1 = y + h - y1;
            }
            self.draw_sub_texture_rgba(tex, [0.0, sub_y, w, h1], x, y1, w, h1, r, g, b, a);
            sub_y = 0.0;
            y1 += h1;
        }
    }

    /// `DrawTextureIgnoreOffset(tex, x, y, w, h, color)`.
    pub fn draw_texture_ignore_offset(&self, tex: Option<&Texture>, x: f64, y: f64, w: i32, h: i32, col: Rgba) {
        if !self.is_visible() {
            return;
        }
        let (ax, ay) = self.abs();
        let (xs, ys) = self.scrolls();
        sprite(|s| s.render(tx(tex), (x + ax + xs) as i32 as f32, (y + ay + ys) as i32 as f32, w as f32, h as f32, col, None));
    }

    /// `DrawTexture_FlippedX` and `DrawTexture_FlippedXIgnoreOffset`
    /// (identical in the original).
    pub fn draw_texture_flipped_x(&self, tex: Option<&Texture>, x: f64, y: f64, w: i32, h: i32, col: Rgba) {
        if !self.is_visible() {
            return;
        }
        let (ax, ay) = self.abs();
        let (xs, ys) = self.scrolls();
        sprite(|s| s.render(tx(tex), (x + ax + xs) as f32, (y + ay + ys) as f32, w as f32, h as f32, col, Some([1.0, 0.0, 0.0, 1.0])));
    }

    /// `DrawUVSliceTexture(tex, x, y, w, h, color, xStart, yStart, xEnd, yEnd)`.
    /// The original's 3-decimal truncation of page UVs is left to the
    /// backend (UVs here are texture-local).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_uv_slice_texture(&self, tex: Option<&Texture>, x: f64, y: f64, w: f64, h: f64, col: Rgba, xs0: f64, ys0: f64, xe: f64, ye: f64) {
        if !self.is_visible() {
            return;
        }
        let Some(t) = tex else { return };
        let mut dx = x + self.get_absolute_x() + t.offset_x as f64;
        let mut dy = y + self.get_absolute_y() + t.offset_y as f64;
        let mut x2 = dx + w;
        let mut y2 = dy + h;
        dx += xs0 * w;
        dy += ys0 * h;
        x2 -= (1.0 - xe) * w;
        y2 -= (1.0 - ye) * h;
        let (sx, sy) = self.scrolls();
        let px = dx as f32 + (sx as i32) as f32;
        let py = dy as f32 + (sy as i32) as f32;
        // The texture coordinates in the texture's own space, truncated to
        // 1/1000 as the original does, then back relative to the texture.
        let (xs, ys, xe0, ye0) = (t.x_start as f64, t.y_start as f64, t.x_end as f64, t.y_end as f64);
        let (xtot, ytot) = (xe0 - xs, ye0 - ys);
        let trunc = |v: f64| ((v * 1000.0) as i32) as f32 / 1000.0;
        let us = trunc(xs + xs0 * xtot);
        let vs = trunc(ys + ys0 * ytot);
        let ue = trunc(xe0 - (1.0 - xe) * xtot);
        let ve = trunc(ye0 - (1.0 - ye) * ytot);
        let rel = |v: f32, start: f64, tot: f64| if tot == 0.0 { 0.0 } else { ((v as f64 - start) / tot) as f32 };
        let uv = [rel(us, xs, xtot), rel(vs, ys, ytot), rel(ue, xs, xtot), rel(ve, ys, ytot)];
        sprite(|s| s.render(Some(t), px, py, (x2 - dx) as f32, (y2 - dy) as f32, col, Some(uv)));
    }

    // ---- stencil ----

    pub fn suspend_stencil(&self) {
        sprite(|s| s.suspend_stencil());
    }
    pub fn resume_stencil(&self) {
        sprite(|s| s.resume_stencil());
    }
    /// `setStencilRect(x, y, w, h)` (local coordinates, no scroll).
    pub fn set_stencil_rect(&self, x: f64, y: f64, w: f64, h: f64) {
        let (ax, ay) = self.abs();
        sprite(|s| s.push_stencil((x + ax) as i32, (y + ay) as i32, w as i32, h as i32, None));
    }
    /// `setStencilCircle`: the stencil takes the shape of
    /// `media/ui/circle.png` (transparent pixels are discarded).
    pub fn set_stencil_circle(&self, x: f64, y: f64, w: f64, h: f64) {
        let (ax, ay) = self.abs();
        let circle = pz_render::get_shared_texture("media/ui/circle.png");
        sprite(|s| s.push_stencil((x + ax) as i32, (y + ay) as i32, w as i32, h as i32, circle.as_ref()));
    }
    pub fn clear_stencil_rect(&self) {
        sprite(|s| s.pop_stencil());
    }
    pub fn repaint_stencil_rect(&self, x: f64, y: f64, w: f64, h: f64) {
        let (ax, ay) = self.abs();
        sprite(|s| s.repaint_stencil((x + ax) as i32, (y + ay) as i32, w as i32, h as i32));
    }
}

#[cfg(test)]
mod item_icon_tests;
