//! The `TextManager.instance` interface `UIElement` draws text through.
//!
//! The real `TextManager` (AngelCode fonts per `UIFont`) belongs to the
//! engine; it implements [`TextRenderer`] and installs it with
//! [`crate::UIManager::set_text_renderer`]. Until then text is recorded as
//! [`crate::UiDrawCmd::Text`] and measures as zero.

use crate::sprite::{SpriteRenderer, TextAlign};
use pz_render::Rgba;
use pz_text::UIFont;

pub trait TextRenderer {
    /// `TextManager.MeasureStringX(font, s)`.
    fn measure_x(&self, font: Option<UIFont>, s: &str) -> i32;
    /// `TextManager.MeasureStringY(font, s)`.
    fn measure_y(&self, font: Option<UIFont>, s: &str) -> i32;
    /// `TextManager.getFontHeight(font)`.
    fn font_height(&self, font: Option<UIFont>) -> i32;
    /// `getFontFromEnum(font).getWidth(s, start, end, xadvance)`: the
    /// width of the characters `start..=end`; the last glyph counts its
    /// `xadvance` rather than its width when `xadvance`. Defaults to
    /// measuring the sub-string.
    fn width_range(&self, font: Option<UIFont>, s: &str, start: i32, end: i32, _xadvance: bool) -> i32 {
        if end < start {
            return 0;
        }
        let sub: String = s.chars().skip(start.max(0) as usize).take((end - start + 1) as usize).collect();
        self.measure_x(font, &sub)
    }
    /// `TextManager.DrawString*`. `font: None` is the manager's default
    /// font (the overloads without a `UIFont`, or a font the port does not
    /// know); `trimmed` is false for `DrawStringUntrimmed`. `(x, y)` is the
    /// absolute anchor point; alignment is relative to it.
    #[allow(clippy::too_many_arguments)]
    fn draw(&self, out: &mut SpriteRenderer, font: Option<UIFont>, x: f64, y: f64, zoom: f32, text: &str, color: Rgba, align: TextAlign, trimmed: bool);
}
