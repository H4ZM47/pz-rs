//! Text: `zombie.core.Translator` (translation tables) and
//! `zombie.ui.TextManager` / `AngelCodeFont` (bitmap fonts).

use std::collections::HashMap;

/// `UIFont` enum, in original ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UIFont {
    Small, Medium, Large, Massive, MainMenu1, MainMenu2, Cred1, Cred2, NewSmall, NewMedium,
    NewLarge, Code, MediumNew, AutoNormSmall, AutoNormMedium, AutoNormLarge, Dialogue, Intro,
    Handwritten, DebugConsole, Title, SdfRegular, SdfBold, SdfItalic, SdfBoldItalic,
}

/// `Translator`: key -> translated string for the active language.
#[derive(Default)]
pub struct Translator {
    pub language: String,
    pub table: HashMap<String, String>,
}

impl Translator {
    /// `Translator.getText(key)`: missing keys return the key itself.
    pub fn get_text<'a>(&'a self, key: &'a str) -> &'a str {
        self.table.get(key).map(String::as_str).unwrap_or(key)
    }
}

/// Glyph metrics of an AngelCode `.fnt` font.
#[derive(Clone, Copy, Debug, Default)]
pub struct Glyph {
    pub x: u16, pub y: u16, pub w: u16, pub h: u16,
    pub xoff: i16, pub yoff: i16, pub xadvance: i16, pub page: u8,
}

#[derive(Default)]
pub struct AngelCodeFont {
    pub line_height: i32,
    pub glyphs: HashMap<u32, Glyph>,
    pub kerning: HashMap<(u32, u32), i16>,
    pub pages: Vec<String>,
}

impl AngelCodeFont {
    /// `AngelCodeFont.getWidth`. TODO: port exact kerning/rounding rules.
    pub fn width(&self, s: &str) -> i32 {
        s.chars().map(|c| self.glyphs.get(&(c as u32)).map_or(0, |g| g.xadvance as i32)).sum()
    }
}
