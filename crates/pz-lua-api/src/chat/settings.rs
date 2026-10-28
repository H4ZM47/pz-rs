//! `zombie.chat.ChatSettings` and `ChatSettings.FontSize`.

use crate::bind_misc_classes::Color;
use pz_text::UIFont;
use std::rc::Rc;

/// `ChatSettings.infinityRange`.
pub const INFINITY_RANGE: f32 = -1.0;

/// `ChatSettings.FontSize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontSize {
    NotDefine,
    Small,
    Medium,
    Large,
}

impl FontSize {
    pub fn name(self) -> &'static str {
        match self {
            FontSize::NotDefine => "NotDefine",
            FontSize::Small => "Small",
            FontSize::Medium => "Medium",
            FontSize::Large => "Large",
        }
    }
}

/// `new Color(float r, float g, float b, float a)` (clamped).
pub fn color_f(r: f32, g: f32, b: f32, a: f32) -> Rc<Color> {
    Rc::new(Color::rgba(r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a.clamp(0.0, 1.0)))
}

/// `new Color(int r, int g, int b, int a)`.
pub fn color_i(r: i32, g: i32, b: i32, a: i32) -> Rc<Color> {
    Rc::new(Color::rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0))
}

/// `Color.white`.
pub fn white() -> Rc<Color> {
    color_f(1.0, 1.0, 1.0, 1.0)
}

/// `zombie.chat.ChatSettings`.
#[derive(Clone)]
pub struct ChatSettings {
    pub unique: bool,
    pub font_color: Rc<Color>,
    pub font: UIFont,
    font_size: FontSize,
    pub bold: bool,
    pub allow_images: bool,
    pub allow_chat_icons: bool,
    pub allow_colors: bool,
    pub allow_fonts: bool,
    pub allow_bbcode: bool,
    pub equalize_line_heights: bool,
    pub show_author: bool,
    pub show_timestamp: bool,
    pub show_chat_title: bool,
    pub use_only_active_tab: bool,
    pub range: f32,
    zombie_attraction_range: f32,
}

impl Default for ChatSettings {
    fn default() -> Self {
        ChatSettings {
            unique: true,
            font_color: white(),
            font: UIFont::Dialogue,
            font_size: FontSize::Medium,
            bold: true,
            allow_images: false,
            allow_chat_icons: false,
            allow_colors: false,
            allow_fonts: false,
            allow_bbcode: false,
            equalize_line_heights: false,
            show_author: true,
            show_timestamp: true,
            show_chat_title: true,
            use_only_active_tab: false,
            range: INFINITY_RANGE,
            zombie_attraction_range: INFINITY_RANGE,
        }
    }
}

impl ChatSettings {
    pub fn new() -> Self {
        Self::default()
    }

    /// `getFontSize()`: the constant name in lower case.
    pub fn get_font_size(&self) -> String {
        self.font_size.name().to_lowercase()
    }

    pub fn font_size(&self) -> FontSize {
        self.font_size
    }

    /// `setFontSize(String)`: "small"/"Small", ... anything else is
    /// `NotDefine`.
    pub fn set_font_size(&mut self, s: &str) {
        self.font_size = match s {
            "small" | "Small" => FontSize::Small,
            "medium" | "Medium" => FontSize::Medium,
            "large" | "Large" => FontSize::Large,
            _ => FontSize::NotDefine,
        };
    }

    pub fn set_font_color_f(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.font_color = color_f(r, g, b, a);
    }

    /// `getZombieAttractionRange()`: the range when unset.
    pub fn get_zombie_attraction_range(&self) -> f32 {
        if self.zombie_attraction_range == INFINITY_RANGE {
            self.range
        } else {
            self.zombie_attraction_range
        }
    }

    pub fn set_zombie_attraction_range(&mut self, r: f32) {
        self.zombie_attraction_range = r;
    }
}
