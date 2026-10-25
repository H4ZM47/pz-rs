//! `zombie.ui.ObjectTooltip`'s drawing and layout API, as item and object
//! tooltips (`InventoryItem.DoTooltip`, `IsoObject.DoSpecialTooltip`) use
//! it: [`ObjectTooltip`] (the widget's state, its measure-only drawing
//! helpers, `beginLayout`/`endLayout`, the font statics) and
//! [`Layout`] / [`LayoutItem`] (`ObjectTooltip.Layout` / `LayoutItem`:
//! label/value rows, progress bars and their `render`).
//!
//! This is the stable surface tooltip content is written against; the
//! widget's lifecycle (`show`/`hide`/`update`/`render`, `UIManager`'s
//! shared tooltip, the Lua classes) builds on it.
//!
//! Get the tooltip of an element with [`ObjectTooltip::of`]; every
//! drawing call takes that element (positions are local to it).

use crate::draw::{font_height, measure_string_x};
use crate::element::UIElement;
use crate::ext::UIElementExt;
use pz_kahlua::Value;
use pz_render::{Rgba, Texture};
use pz_text::UIFont;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const OBJECT_TOOLTIP: &str = "zombie.ui.ObjectTooltip";
const SUPERS: &[&str] = &["zombie.ui.UIElement"];

/// `ObjectTooltip`'s statics.
struct Statics {
    line_spacing: i32,
    pad_left: i32,
    pad_right: i32,
    pad_top: i32,
    pad_bottom: i32,
    font_size: String,
    font: UIFont,
    /// `Core.getGoodHighlitedColor()` / `getBadHighlitedColor()` as last
    /// set with [`ObjectTooltip::set_highlight_colors`].
    good: Rgba,
    bad: Rgba,
}

thread_local! {
    static STATICS: RefCell<Statics> = RefCell::new(Statics {
        line_spacing: 14,
        pad_left: -1,
        pad_right: 0,
        pad_top: 0,
        pad_bottom: 0,
        font_size: "Small".into(),
        font: UIFont::Small,
        good: Rgba(0.0, 1.0, 0.0, 1.0),
        bad: Rgba(1.0, 0.0, 0.0, 1.0),
    });
}

fn statics<R>(f: impl FnOnce(&mut Statics) -> R) -> R {
    STATICS.with(|s| f(&mut s.borrow_mut()))
}

/// `Float.toString(float)`.
pub fn java_float_to_string(f: f32) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let sci = format!("{f:e}");
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut out = String::new();
    if mant.starts_with('-') {
        out.push('-');
    }
    if (1.0e-3..1.0e7).contains(&f.abs()) {
        if exp >= 0 {
            let int_len = (exp + 1) as usize;
            let mut d = digits;
            while d.len() < int_len {
                d.push('0');
            }
            let (i, frac) = d.split_at(int_len);
            out.push_str(i);
            out.push('.');
            out.push_str(if frac.is_empty() { "0" } else { frac });
        } else {
            out.push_str("0.");
            for _ in 0..(-exp - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        }
    } else {
        let (first, rest) = digits.split_at(1);
        out.push_str(first);
        out.push('.');
        out.push_str(if rest.is_empty() { "0" } else { rest });
        out.push('E');
        out.push_str(&exp.to_string());
    }
    out
}

/// `zombie.ui.ObjectTooltip`'s instance state.
pub struct ObjectTooltip {
    pub pad_left: Cell<i32>,
    pub pad_top: Cell<i32>,
    pub pad_right: Cell<i32>,
    pub pad_bottom: Cell<i32>,
    measure_only: Cell<bool>,
    weight_of_stack: Cell<f32>,
    /// `getCharacter()` (the Lua character object).
    character: RefCell<Option<Value>>,
    /// `texture` (`Texture.getSharedTexture("black")`).
    pub texture: Option<Texture>,
}

impl ObjectTooltip {
    /// The tooltip state of `el`, if it is an `ObjectTooltip`.
    pub fn of(el: &UIElement) -> Option<&ObjectTooltip> {
        el.ext::<ObjectTooltip>()
    }

    /// `new ObjectTooltip()`: 130 x 130, no default draw, the line spacing
    /// and pads of the current font (`tooltip_font` is
    /// `Core.getOptionTooltipFont()`).
    pub fn create(tooltip_font: &str) -> Rc<UIElement> {
        let t = ObjectTooltip {
            pad_left: Cell::new(5),
            pad_top: Cell::new(5),
            pad_right: Cell::new(5),
            pad_bottom: Cell::new(5),
            measure_only: Cell::new(false),
            weight_of_stack: Cell::new(0.0),
            character: RefCell::new(None),
            texture: pz_render::get_shared_texture("black"),
        };
        let e = UIElement::new_ext(None, Box::new(t));
        {
            let mut s = e.st_mut();
            s.width = 130.0;
            s.height = 130.0;
            s.default_draw = false;
        }
        let font = statics(|s| s.font);
        statics(|s| s.line_spacing = font_height(Some(font)));
        Self::check_font(tooltip_font);
        e
    }

    /// `checkFont()` with `Core.getOptionTooltipFont()`: the font for
    /// "Small"/"Medium"/"Large", its line height, and pads of one "0"
    /// wide (half that top and bottom).
    pub fn check_font(tooltip_font: &str) {
        let changed = statics(|s| s.font_size != tooltip_font);
        if changed {
            let font = match tooltip_font {
                "Large" => UIFont::Large,
                "Medium" => UIFont::Medium,
                _ => UIFont::Small,
            };
            let lh = font_height(Some(font));
            statics(|s| {
                s.font_size = tooltip_font.to_string();
                s.font = font;
                s.line_spacing = lh;
            });
        }
        let font = statics(|s| s.font);
        let char_width = measure_string_x(Some(font), "0");
        statics(|s| {
            s.pad_left = char_width;
            s.pad_right = char_width;
            s.pad_top = char_width / 2;
            s.pad_bottom = char_width / 2;
        });
    }

    /// `Core.getGoodHighlitedColor()` / `getBadHighlitedColor()`, which
    /// `LayoutItem::set_value_right` reads (the engine keeps them current).
    pub fn set_highlight_colors(good: Rgba, bad: Rgba) {
        statics(|s| {
            s.good = good;
            s.bad = bad;
        });
    }

    /// `getFont()`.
    pub fn font() -> UIFont {
        statics(|s| s.font)
    }

    /// The tooltip font option the font was chosen for.
    pub fn font_size() -> String {
        statics(|s| s.font_size.clone())
    }

    /// `getLineSpacing()`.
    pub fn line_spacing() -> i32 {
        statics(|s| s.line_spacing)
    }

    /// `beginLayout()`: the pads become the font's, and a new layout.
    pub fn begin_layout(&self) -> Layout {
        let (l, r, t, b) = statics(|s| (s.pad_left, s.pad_right, s.pad_top, s.pad_bottom));
        self.pad_left.set(l);
        self.pad_right.set(r);
        self.pad_top.set(t);
        self.pad_bottom.set(b);
        Layout::default()
    }

    /// `endLayout(layout)`: the layout (and its `next` chain) is done.
    pub fn end_layout(&self, layout: Layout) {
        drop(layout);
    }

    pub fn set_measure_only(&self, b: bool) {
        self.measure_only.set(b);
    }
    pub fn is_measure_only(&self) -> bool {
        self.measure_only.get()
    }
    pub fn weight_of_stack(&self) -> f32 {
        self.weight_of_stack.get()
    }
    pub fn set_weight_of_stack(&self, w: f32) {
        self.weight_of_stack.set(w);
    }
    /// `getCharacter()`.
    pub fn character(&self) -> Option<Value> {
        self.character.borrow().clone()
    }
    /// `setCharacter(chr)`.
    pub fn set_character(&self, chr: Option<Value>) {
        *self.character.borrow_mut() = chr;
    }

    // ---- drawing (skipped while measuring) ----

    /// `DrawText(font, text, x, y, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(
        &self,
        el: &UIElement,
        font: Option<UIFont>,
        text: &str,
        x: f64,
        y: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        if self.measure_only.get() {
            return;
        }
        el.draw_text(font, Some(text), x, y, r, g, b, a);
    }

    /// `DrawTextCentre(font, text, x, y, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_centre(
        &self,
        el: &UIElement,
        font: Option<UIFont>,
        text: &str,
        x: f64,
        y: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        if self.measure_only.get() {
            return;
        }
        el.draw_text_centre(font, text, x, y, r, g, b, a);
    }

    /// `DrawTextRight(font, text, x, y, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_right(
        &self,
        el: &UIElement,
        font: Option<UIFont>,
        text: &str,
        x: f64,
        y: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        if self.measure_only.get() {
            return;
        }
        el.draw_text_right(font, text, x, y, r, g, b, a);
    }

    /// `DrawValueRight(value, x, y, highGood)`: signed, green when good.
    pub fn draw_value_right(&self, el: &UIElement, value: i32, x: i32, y: i32, high_good: bool) {
        let mut s = value.to_string();
        let (mut r, mut g, mut b) = (0.3f32, 1.0f32, 0.2f32);
        if value > 0 {
            s = format!("+{s}");
        }
        if (value < 0 && high_good) || (value > 0 && !high_good) {
            r = 0.8;
            g = 0.3;
            b = 0.2;
        }
        self.draw_text_right(
            el,
            Some(Self::font()),
            &s,
            x as f64,
            y as f64,
            r as f64,
            g as f64,
            b as f64,
            1.0,
        );
    }

    /// `DrawValueRightNoPlus(int value, x, y)`.
    pub fn draw_value_right_no_plus_int(&self, el: &UIElement, value: i32, x: i32, y: i32) {
        self.draw_text_right(
            el,
            Some(Self::font()),
            &value.to_string(),
            x as f64,
            y as f64,
            1.0,
            1.0,
            1.0,
            1.0,
        );
    }

    /// `DrawValueRightNoPlus(float value, x, y)`: one decimal, rounded up
    /// from .09.
    pub fn draw_value_right_no_plus_float(&self, el: &UIElement, value: f32, x: i32, y: i32) {
        let val = (((value as f64 + 0.01) * 10.0) as i32) as f32 / 10.0;
        self.draw_text_right(
            el,
            Some(Self::font()),
            &java_float_to_string(val),
            x as f64,
            y as f64,
            1.0,
            1.0,
            1.0,
            1.0,
        );
    }

    /// `DrawTextureScaled(tex, x, y, w, h, alpha)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_scaled(
        &self,
        el: &UIElement,
        tex: Option<&Texture>,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        alpha: f64,
    ) {
        if self.measure_only.get() {
            return;
        }
        el.draw_texture_scaled(tex, x, y, w, h, alpha);
    }

    /// `DrawTextureScaledAspect(tex, x, y, w, h, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_scaled_aspect(
        &self,
        el: &UIElement,
        tex: Option<&Texture>,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        if self.measure_only.get() {
            return;
        }
        el.draw_texture_scaled_aspect(tex, x, y, w, h, r, g, b, a);
    }

    /// `DrawProgressBar(x, y, w, h, f, r, g, b, a)`: a grey frame, the
    /// unfilled part and the filled part.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_progress_bar(
        &self,
        el: &UIElement,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        f: f32,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        if self.measure_only.get() {
            return;
        }
        let f = f.clamp(0.0, 1.0);
        let done = (w as f32 * f).floor() as i32;
        let (x, y, w, h) = (x as f64, y as f64, w as f64, h as f64);
        el.draw_texture_scaled_col(None, x - 1.0, y - 1.0, w + 2.0, h, 0.25, 0.25, 0.25, 1.0);
        if f != 0.0 && f != 1.0 {
            el.draw_texture_scaled_col(
                None,
                x + done as f64,
                y,
                w - done as f64,
                h - 2.0,
                0.5,
                0.5,
                0.5,
                1.0,
            );
        }
        el.draw_texture_scaled_col(None, x, y, done as f64, h - 2.0, r, g, b, a);
    }

    /// `adjustWidth(textX, text)`: widen to fit `text` drawn at `textX`.
    pub fn adjust_width(&self, el: &UIElement, text_x: i32, text: &str) {
        let w = measure_string_x(Some(Self::font()), text);
        if (text_x + w + self.pad_right.get()) as f32 > el.get_width() as f32 {
            el.set_width((text_x + w + self.pad_right.get()) as f64);
        }
    }
}

impl UIElementExt for ObjectTooltip {
    fn class_name(&self) -> &'static str {
        OBJECT_TOOLTIP
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `ObjectTooltip.Layout`: rows of label, value or progress, laid out in
/// two columns; `next` chains a further layout below.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub items: Vec<LayoutItem>,
    pub min_label_width: i32,
    pub min_value_width: i32,
    pub next: Option<Box<Layout>>,
    pub next_pad_y: i32,
    pub offset_y: i32,
}

impl Layout {
    /// `addItem()`.
    pub fn add_item(&mut self) -> &mut LayoutItem {
        self.items.push(LayoutItem::default());
        self.items.last_mut().unwrap()
    }

    /// `setMinLabelWidth(w)`.
    pub fn set_min_label_width(&mut self, w: i32) {
        self.min_label_width = w;
    }

    /// `setMinValueWidth(w)`.
    pub fn set_min_value_width(&mut self, w: i32) {
        self.min_value_width = w;
    }

    /// `render(left, top, ui)`: measures the rows, widens the tooltip to
    /// fit, draws them from `top` down (then `next`), and returns the y
    /// below the last row.
    pub fn render(&mut self, left: i32, top: i32, el: &UIElement) -> i32 {
        let tip = ObjectTooltip::of(el).expect("an ObjectTooltip");
        let font = Some(ObjectTooltip::font());
        let mut width_label = self.min_label_width;
        let mut width_value = self.min_value_width;
        let mut width_value_right = self.min_value_width;
        let mut width_progress = 0;
        let mut width_total = 0;
        let pad_x = measure_string_x(font, "W").max(8);
        let mut mid = 0;
        for item in &mut self.items {
            item.calc_sizes();
            if item.has_value {
                width_label = width_label.max(item.label_width);
                width_value = width_value.max(item.value_width);
                width_value_right = width_value_right.max(item.value_width_right);
                width_progress = width_progress.max(item.progress_width);
                mid = mid.max(item.label_width.max(self.min_label_width) + pad_x);
                width_total = width_total.max(
                    width_label + pad_x + width_value.max(width_value_right).max(width_progress),
                );
                continue;
            }
            if item.could_have_value {
                width_label = width_label.max(item.label_width);
            }
            width_total = width_total.max(item.label_width);
        }
        if (left + width_total + tip.pad_right.get()) as f32 > el.get_width() as f32 {
            el.set_width((left + width_total + tip.pad_right.get()) as f64);
        }
        let mut top = top;
        for item in &self.items {
            item.render(left, top, mid, width_value_right, el);
            top += item.height;
        }
        match self.next.as_mut() {
            Some(next) => {
                let pad = next.next_pad_y;
                next.render(left, top + pad, el)
            }
            None => top,
        }
    }
}

/// `ObjectTooltip.LayoutItem`: one row.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutItem {
    pub label: Option<String>,
    pub r0: f32,
    pub g0: f32,
    pub b0: f32,
    pub a0: f32,
    pub has_value: bool,
    pub could_have_value: bool,
    pub value: Option<String>,
    pub right_justify: bool,
    pub r1: f32,
    pub g1: f32,
    pub b1: f32,
    pub a1: f32,
    pub progress_fraction: f32,
    pub label_width: i32,
    pub value_width: i32,
    pub value_width_right: i32,
    pub progress_width: i32,
    pub height: i32,
}

impl Default for LayoutItem {
    fn default() -> Self {
        LayoutItem {
            label: None,
            r0: 0.0,
            g0: 0.0,
            b0: 0.0,
            a0: 0.0,
            has_value: false,
            could_have_value: false,
            value: None,
            right_justify: false,
            r1: 0.0,
            g1: 0.0,
            b1: 0.0,
            a1: 0.0,
            progress_fraction: -1.0,
            label_width: 0,
            value_width: 0,
            value_width_right: 0,
            progress_width: 0,
            height: 0,
        }
    }
}

impl LayoutItem {
    /// `setLabel(label, r, g, b, a)`.
    pub fn set_label(&mut self, label: &str, r: f32, g: f32, b: f32, a: f32) {
        self.label = Some(label.to_string());
        (self.r0, self.g0, self.b0, self.a0) = (r, g, b, a);
    }

    /// `setValue(value, r, g, b, a)` (right-justified).
    pub fn set_value(&mut self, value: &str, r: f32, g: f32, b: f32, a: f32) {
        self.value = Some(value.to_string());
        (self.r1, self.g1, self.b1, self.a1) = (r, g, b, a);
        self.has_value = true;
        self.right_justify = true;
    }

    /// `setValueRight(value, highGood)`: signed, in the good or bad
    /// highlight colour.
    pub fn set_value_right(&mut self, value: i32, high_good: bool) {
        let mut s = value.to_string();
        if value > 0 {
            s = format!("+{s}");
        }
        self.value = Some(s);
        let (good, bad) = statics(|s| (s.good, s.bad));
        let c = if (value < 0 && high_good) || (value > 0 && !high_good) {
            bad
        } else {
            good
        };
        (self.r1, self.g1, self.b1) = (c.0, c.1, c.2);
        self.a1 = 1.0;
        self.has_value = true;
        self.right_justify = true;
    }

    /// `setValueRightNoPlus(float)`: two decimals, rounded half up, white.
    pub fn set_value_right_no_plus_float(&mut self, value: f32) {
        let v = (((value + 0.005) * 100.0) as i32) as f32 / 100.0;
        self.value = Some(java_float_to_string(v));
        (self.r1, self.g1, self.b1, self.a1) = (1.0, 1.0, 1.0, 1.0);
        self.has_value = true;
        self.right_justify = true;
    }

    /// `setValueRightNoPlus(int)`: white.
    pub fn set_value_right_no_plus_int(&mut self, value: i32) {
        self.value = Some(value.to_string());
        (self.r1, self.g1, self.b1, self.a1) = (1.0, 1.0, 1.0, 1.0);
        self.has_value = true;
        self.right_justify = true;
    }

    /// `setProgress(fraction, r, g, b, a)`: an 80 px bar in the value
    /// column.
    pub fn set_progress(&mut self, fraction: f32, r: f32, g: f32, b: f32, a: f32) {
        self.progress_fraction = fraction;
        (self.r1, self.g1, self.b1, self.a1) = (r, g, b, a);
        self.has_value = true;
    }

    /// `calcSizes()`.
    pub fn calc_sizes(&mut self) {
        let font = Some(ObjectTooltip::font());
        self.progress_width = 0;
        self.value_width_right = 0;
        self.value_width = 0;
        self.label_width = 0;
        if let Some(l) = &self.label {
            self.label_width = measure_string_x(font, l);
        }
        if self.has_value {
            if let Some(v) = &self.value {
                let w = measure_string_x(font, v);
                self.value_width = if self.right_justify { 0 } else { w };
                self.value_width_right = if self.right_justify { w } else { 0 };
            } else if self.progress_fraction != -1.0 {
                self.progress_width = 80;
            }
        }
        let lines_of = |s: &str| 1 + s.chars().filter(|&c| c == '\n').count() as i32;
        let mut lines = 1;
        if let Some(l) = &self.label {
            lines = lines.max(lines_of(l));
        }
        if self.has_value {
            if let Some(v) = &self.value {
                lines = lines.max(lines_of(v));
            }
        }
        self.height = lines * ObjectTooltip::line_spacing();
    }

    /// `render(x, y, mid, right, ui)`.
    pub fn render(&self, x: i32, y: i32, mid: i32, right: i32, el: &UIElement) {
        let tip = ObjectTooltip::of(el).expect("an ObjectTooltip");
        let font = Some(ObjectTooltip::font());
        if let Some(l) = &self.label {
            tip.draw_text(
                el,
                font,
                l,
                x as f64,
                y as f64,
                self.r0 as f64,
                self.g0 as f64,
                self.b0 as f64,
                self.a0 as f64,
            );
        }
        let (r, g, b, a) = (
            self.r1 as f64,
            self.g1 as f64,
            self.b1 as f64,
            self.a1 as f64,
        );
        if let Some(v) = &self.value {
            if self.right_justify {
                tip.draw_text_right(el, font, v, (x + mid + right) as f64, y as f64, r, g, b, a);
            } else {
                tip.draw_text(el, font, v, (x + mid) as f64, y as f64, r, g, b, a);
            }
        }
        if self.progress_fraction != -1.0 {
            let h = match ObjectTooltip::font_size().as_str() {
                "Medium" => 6,
                "Large" => 7,
                _ => 5,
            };
            tip.draw_progress_bar(
                el,
                x + mid,
                y + ObjectTooltip::line_spacing() / 2 - 1,
                right,
                h,
                self.progress_fraction,
                r,
                g,
                b,
                a,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprite::{SpriteRenderer, TextAlign, UiDrawCmd};
    use crate::text::TextRenderer;
    use crate::{with_ui, UIManager};

    /// 8 px per character; 16/20/24 px lines for Small/Medium/Large.
    struct Mono;
    fn lh(f: Option<UIFont>) -> i32 {
        match f {
            Some(UIFont::Large) => 24,
            Some(UIFont::Medium) => 20,
            _ => 16,
        }
    }
    impl TextRenderer for Mono {
        fn measure_x(&self, _: Option<UIFont>, s: &str) -> i32 {
            8 * s.chars().count() as i32
        }
        fn measure_y(&self, f: Option<UIFont>, _: &str) -> i32 {
            lh(f)
        }
        fn font_height(&self, f: Option<UIFont>) -> i32 {
            lh(f)
        }
        fn draw(
            &self,
            out: &mut SpriteRenderer,
            font: Option<UIFont>,
            x: f64,
            y: f64,
            zoom: f32,
            text: &str,
            color: Rgba,
            align: TextAlign,
            trimmed: bool,
        ) {
            out.push(UiDrawCmd::Text {
                font,
                x: x as f32,
                y: y as f32,
                zoom,
                text: Rc::from(text),
                color,
                align,
                trimmed,
            });
        }
    }

    fn jf(v: f32) -> String {
        java_float_to_string(v)
    }

    fn lines() -> Vec<String> {
        with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    UiDrawCmd::Text {
                        font,
                        x,
                        y,
                        text,
                        color,
                        align,
                        ..
                    } => {
                        let a = match align {
                            TextAlign::Left => "left",
                            TextAlign::Right => "right",
                            TextAlign::Centre => "centre",
                        };
                        Some(format!(
                            "text {:?} {a} {} {} [{text}] {} {} {} {}",
                            font.unwrap(),
                            jf(*x),
                            jf(*y),
                            jf(color.0),
                            jf(color.1),
                            jf(color.2),
                            jf(color.3)
                        ))
                    }
                    UiDrawCmd::Quad {
                        texture: None,
                        x,
                        y,
                        w,
                        h,
                        color,
                        ..
                    } => Some(format!(
                        "draw - {} {} {} {} {} {} {} {}",
                        *x as i32,
                        *y as i32,
                        *w as i32,
                        *h as i32,
                        jf(color.0),
                        jf(color.1),
                        jf(color.2),
                        jf(color.3)
                    )),
                    _ => None,
                })
                .collect()
        })
    }

    /// The layout of `TooltipOracle` (labels, values of every kind,
    /// progress bars, a multi-line label, a min label width, a chained
    /// layout), measured and drawn for each tooltip font.
    fn scenario(out: &mut Vec<String>) {
        for f in ["Small", "Medium", "Large"] {
            let e = ObjectTooltip::create(f);
            let t = ObjectTooltip::of(&e).unwrap();
            e.set_x(100.0);
            e.set_y(50.0);
            for measure in 0..2 {
                t.set_measure_only(measure == 1);
                e.set_width(130.0);
                let mut l = t.begin_layout();
                l.set_min_label_width(40);
                let i = l.add_item();
                i.set_label("Weight:", 1.0, 1.0, 0.8, 1.0);
                i.set_value_right_no_plus_float(1.234);
                let i = l.add_item();
                i.set_label("Condition:", 1.0, 1.0, 0.8, 1.0);
                i.set_progress(0.4, 0.2, 0.9, 0.3, 1.0);
                let i = l.add_item();
                i.set_label("Bonus:", 1.0, 1.0, 1.0, 1.0);
                i.set_value_right(3, true);
                let i = l.add_item();
                i.set_label("Penalty:", 1.0, 1.0, 1.0, 1.0);
                i.set_value_right(-2, true);
                let i = l.add_item();
                i.set_label("Count:", 1.0, 1.0, 1.0, 1.0);
                i.set_value_right_no_plus_int(12);
                let i = l.add_item();
                i.set_label("Two\nlines without a value", 0.5, 0.5, 0.5, 1.0);
                let i = l.add_item();
                i.set_label("Name", 1.0, 1.0, 1.0, 1.0);
                i.set_value("A long value text", 1.0, 0.5, 0.5, 1.0);
                let i = l.add_item();
                i.set_label("Full", 1.0, 1.0, 1.0, 1.0);
                i.set_progress(1.0, 1.0, 1.0, 1.0, 1.0);
                let mut n = t.begin_layout();
                n.next_pad_y = 7;
                let i = n.add_item();
                i.set_label("Chained", 1.0, 1.0, 1.0, 1.0);
                i.set_progress(0.0, 1.0, 0.0, 0.0, 1.0);
                l.next = Some(Box::new(n));
                let y = l.render(t.pad_left.get(), t.pad_top.get(), &e);
                t.end_layout(l);
                t.adjust_width(&e, 5, "An adjusted width line of text");
                t.draw_value_right(&e, -5, 60, 70, false);
                t.draw_value_right_no_plus_float(&e, 2.96, 60, 80);
                t.draw_value_right_no_plus_int(&e, 7, 60, 90);
                out.push(format!(
                    "result {f} {measure} y {y} w {} pads {} {} {} {} line {}",
                    jf(e.get_width() as f32),
                    t.pad_left.get(),
                    t.pad_top.get(),
                    t.pad_right.get(),
                    t.pad_bottom.get(),
                    ObjectTooltip::line_spacing()
                ));
                out.push(format!("case tooltip {f} {measure}"));
                out.extend(lines());
                with_ui(|m| m.sprite.clear());
            }
        }
    }

    fn setup() {
        UIManager::reset();
        UIManager::set_text_renderer(Some(Rc::new(Mono)));
    }

    #[test]
    fn layout_measures_and_draws() {
        setup();
        let mut got = Vec::new();
        scenario(&mut got);
        let small: Vec<&str> = got
            .iter()
            .take_while(|l| !l.starts_with("result Small 1"))
            .map(String::as_str)
            .collect();
        assert_eq!(
            small[0],
            "result Small 0 y 171 w 253.0 pads 8 4 8 4 line 16"
        );
        // Right-justified values end at left + mid + the value column.
        assert!(small.contains(&"text Small right 332.0 54.0 [1.23] 1.0 1.0 1.0 1.0"));
        assert!(small.contains(&"text Small right 332.0 86.0 [+3] 0.0 1.0 0.0 1.0"));
        // Measuring only: nothing drawn, same size.
        let i = got
            .iter()
            .position(|l| l.starts_with("result Small 1"))
            .unwrap();
        assert_eq!(got[i], "result Small 1 y 171 w 253.0 pads 8 4 8 4 line 16");
        assert_eq!(got[i + 1], "case tooltip Small 1");
        assert!(got[i + 2].starts_with("result Medium 0"));
    }

    #[test]
    fn java_float_strings() {
        assert_eq!(java_float_to_string(1.23), "1.23");
        assert_eq!(java_float_to_string(2.0), "2.0");
        assert_eq!(java_float_to_string(0.0001), "1.0E-4");
        assert_eq!(java_float_to_string(-0.5), "-0.5");
    }

    /// Parity with the original `ObjectTooltip`/`Layout`/`LayoutItem`
    /// (oracle `tooltip.txt`, run with a monospace `TextManager`): every
    /// text and bar draw, the resulting y, width and pads, for all three
    /// tooltip fonts, drawn and measure-only.
    #[test]
    fn matches_tooltip_oracle() {
        let Ok(dir) = std::env::var("PZ_HUD_ORACLE_DIR") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        let text = std::fs::read_to_string(std::path::Path::new(&dir).join("tooltip.txt")).unwrap();
        setup();
        let mut got = Vec::new();
        scenario(&mut got);
        assert_eq!(got.join("\n"), text.trim_end_matches('\n'));
    }
}
