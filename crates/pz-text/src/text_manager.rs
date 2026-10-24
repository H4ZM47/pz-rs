//! `zombie.ui.TextManager`, `FontsFile` and `FontsFileFont`.

use std::collections::HashMap;
use std::rc::Rc;

use pz_render::{DrawList, Rgba, TextureLoader};

use crate::font::{AngelCodeFont, FontError};
use crate::script_parser::{self, java_trim};
use crate::translator::equals_ignore_case;
use crate::UIFont;

/// `FontsFileFont`: one `font <Id> { fnt, img, type }` block.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontsFileFont {
    pub id: String,
    pub fnt: String,
    pub img: Option<String>,
    pub r#type: Option<String>,
}

/// `FontsFile.read`: `fonts.txt` into `fonts` (later files override ids).
/// Returns false when the file is missing or malformed; fonts read before
/// a malformed block stay in the map, as in the original.
pub fn read_fonts_file(path: &std::path::Path, fonts: &mut HashMap<String, FontsFileFont>) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let text = String::from_utf8_lossy(&bytes);
    // BufferedReader.readLine() + append: line terminators are dropped.
    let joined: String = text.split('\n').flat_map(|l| l.split('\r')).collect();
    match fonts_from_string(&joined, fonts) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("ERROR: {e}");
            false
        }
    }
}

fn fonts_from_string(
    contents: &str,
    fonts: &mut HashMap<String, FontsFileFont>,
) -> Result<(), String> {
    let block = script_parser::parse(&script_parser::strip_comments(contents));
    let version = block
        .get_value("VERSION")
        .and_then(|v| java_trim(&v.value()).parse::<i32>().ok())
        .unwrap_or(-1);
    if version != 1 {
        return Err("invalid or missing VERSION".into());
    }
    for b in &block.children {
        if !equals_ignore_case(&b.r#type, "font") {
            return Err(format!("unknown block type \"{}\"", b.r#type));
        }
        let Some(id) = b.id.clone().filter(|s| !is_blank(s)) else {
            eprintln!("WARN: missing or empty font id");
            continue;
        };
        let fnt = b.get_value("fnt").map(|v| v.value());
        let Some(fnt) = fnt.filter(|s| !is_blank(s)) else {
            eprintln!("WARN: missing or empty value \"fnt\"");
            continue;
        };
        let img = b
            .get_value("img")
            .map(|v| v.value())
            .filter(|s| !is_blank(s))
            .map(|s| java_trim(&s).to_string());
        let ty = b
            .get_value("type")
            .map(|v| java_trim(&v.value()).to_string())
            .filter(|s| !is_blank(s));
        let font = FontsFileFont {
            id: id.clone(),
            fnt: java_trim(&fnt).to_string(),
            img,
            r#type: ty,
        };
        fonts.insert(id, font);
    }
    Ok(())
}

fn is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// `TextManager.DeferedTextDraw`.
#[derive(Clone, Debug)]
pub struct DeferedTextDraw {
    pub font: Option<UIFont>,
    pub x: f64,
    pub y: f64,
    pub str: String,
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

/// What `TextManager.Init` reads from `Core` and the translator.
#[derive(Clone, Debug)]
pub struct TextOptions {
    /// `Translator.getLanguage().name()`.
    pub language: String,
    /// `Core.getOptionEnableDyslexicFont()`.
    pub dyslexic: bool,
    /// `Core.getOptionFontSizeReal()` (1 = default, 2..5 = 1x..4x).
    pub font_size: i32,
}

/// `zombie.ui.TextManager`.
pub struct TextManager {
    pub enum_to_font: Vec<Option<Rc<AngelCodeFont>>>,
    /// `normal[points - 11]`: zomboidNormal11..24.
    pub normal: Vec<Option<Rc<AngelCodeFont>>>,
    /// `TextManager.font`, `font2`, ...: snapshots taken at the end of
    /// `Init`, before the `MediumNew`/`AutoNorm*` slots are cleared.
    font: Option<Rc<AngelCodeFont>>,
    pub current_code_font: UIFont,
    pub todo_text_list: Vec<DeferedTextDraw>,
    /// `GameServer.server && !ServerGUI.isCreated()`: measuring returns 0.
    pub server_without_gui: bool,
}

impl Default for TextManager {
    fn default() -> Self {
        TextManager {
            enum_to_font: vec![None; UIFont::ALL.len()],
            normal: vec![None; 14],
            font: None,
            current_code_font: UIFont::CodeSmall,
            todo_text_list: Vec::new(),
            server_without_gui: false,
        }
    }
}

fn rgba(r: f64, g: f64, b: f64, a: f64) -> Rgba {
    Rgba(r as f32, g as f32, b as f32, a as f32)
}

impl TextManager {
    /// `getFontFilePath`: size dir of the language, the language, then the
    /// same under EN, then `media/fonts/`, else `media/<file>`.
    fn font_file_path(
        known: &dyn Fn(&str) -> bool,
        lang: &str,
        size_dir: Option<&str>,
        file: &str,
    ) -> String {
        let mut candidates = Vec::new();
        if let Some(sd) = size_dir {
            candidates.push(format!("media/fonts/{lang}/{sd}/{file}"));
        }
        candidates.push(format!("media/fonts/{lang}/{file}"));
        if lang != "EN" {
            if let Some(sd) = size_dir {
                candidates.push(format!("media/fonts/EN/{sd}/{file}"));
            }
            candidates.push(format!("media/fonts/EN/{file}"));
        }
        candidates.push(format!("media/fonts/{file}"));
        candidates
            .into_iter()
            .find(|p| known(p))
            .unwrap_or_else(|| format!("media/{file}"))
    }

    /// `TextManager.Init`. `resolve` is `ZomboidFileSystem.getString`
    /// (`None` when the file is not known).
    pub fn init(
        &mut self,
        opts: &TextOptions,
        resolve: &dyn Fn(&str) -> Option<std::path::PathBuf>,
        loader: &mut dyn TextureLoader,
    ) -> Result<(), FontError> {
        let lang = opts.language.as_str();
        let mut map: HashMap<String, FontsFileFont> = HashMap::new();
        let read = |p: &str, map: &mut HashMap<String, FontsFileFont>| {
            if let Some(path) = resolve(p) {
                read_fonts_file(&path, map);
            }
        };
        if opts.dyslexic {
            read(&format!("media/fonts/{lang}/fontsDyslexic.txt"), &mut map);
        }
        if map.is_empty() {
            read("media/fonts/EN/fonts.txt", &mut map);
            if lang != "EN" {
                read(&format!("media/fonts/{lang}/fonts.txt"), &mut map);
            }
        }
        let size_dir = match opts.font_size {
            2 => Some("1x"),
            3 => Some("2x"),
            4 => Some("3x"),
            5 => Some("4x"),
            _ => None,
        };
        self.enum_to_font.iter_mut().for_each(|f| *f = None);
        self.normal.iter_mut().for_each(|f| *f = None);
        let known = |p: &str| resolve(p).is_some();
        let mut loaded: HashMap<String, Rc<AngelCodeFont>> = HashMap::new();
        for ui in UIFont::ALL {
            let Some(ff) = map.get(ui.name()) else {
                continue;
            };
            let fnt = Self::font_file_path(&known, lang, size_dir, &ff.fnt);
            let img = ff
                .img
                .as_ref()
                .map(|i| Self::font_file_path(&known, lang, size_dir, i));
            let key = format!("{fnt}|{}", img.as_deref().unwrap_or("null"));
            if let Some(f) = loaded.get(&key) {
                self.enum_to_font[ui as usize] = Some(f.clone());
                continue;
            }
            let mut f = AngelCodeFont::load(&fnt, img.as_deref(), resolve, loader)?;
            f.set_sdf(
                ff.r#type
                    .as_deref()
                    .is_some_and(|t| equals_ignore_case(t, "sdf")),
            );
            let f = Rc::new(f);
            self.enum_to_font[ui as usize] = Some(f.clone());
            loaded.insert(key, f);
        }
        if self.enum_to_font[UIFont::DebugConsole as usize].is_none() {
            self.enum_to_font[UIFont::DebugConsole as usize] =
                self.enum_to_font[UIFont::Small as usize].clone();
        }
        for i in 0..self.normal.len() {
            let n = i + 11;
            let f = AngelCodeFont::load(
                &format!("media/fonts/zomboidNormal{n}.fnt"),
                Some(&format!("media/fonts/zomboidNormal{n}_0")),
                resolve,
                loader,
            )?;
            self.normal[i] = Some(Rc::new(f));
        }
        self.font = self.enum_to_font[UIFont::Small as usize].clone();
        for f in [
            UIFont::MediumNew,
            UIFont::AutoNormSmall,
            UIFont::AutoNormMedium,
            UIFont::AutoNormLarge,
        ] {
            self.enum_to_font[f as usize] = None;
        }
        Ok(())
    }

    /// Puts `font` in the slot for `f` (as `Init` would for a fonts.txt
    /// entry); setting `Small` also sets `TextManager.font`.
    pub fn set_font(&mut self, f: UIFont, font: AngelCodeFont) {
        let font = Rc::new(font);
        if f == UIFont::Small {
            self.font = Some(font.clone());
        }
        self.enum_to_font[f as usize] = Some(font);
    }

    /// `TextManager.font` (the `Small` font).
    pub fn font(&self) -> Option<&AngelCodeFont> {
        self.font.as_deref()
    }

    /// `getFontFromEnum`: unset slots fall back to `Small`.
    pub fn get_font_from_enum(&self, font: Option<UIFont>) -> Option<&AngelCodeFont> {
        match font {
            None => self.font(),
            Some(f) => self.enum_to_font[f as usize].as_deref().or(self.font()),
        }
    }

    /// `getNormalFromFontSize(points)`.
    pub fn get_normal_from_font_size(&self, points: i32) -> Option<&AngelCodeFont> {
        self.normal
            .get((points - 11) as usize)
            .and_then(|f| f.as_deref())
    }

    pub fn get_font_height(&self, font: Option<UIFont>) -> i32 {
        self.get_font_from_enum(font)
            .map_or(0, AngelCodeFont::line_height)
    }

    pub fn is_sdf(&self, font: Option<UIFont>) -> bool {
        self.get_font_from_enum(font)
            .is_some_and(AngelCodeFont::is_sdf)
    }

    /// `getAllFonts`: every `UIFont`, sorted by name ignoring case.
    pub fn get_all_fonts(&self) -> Vec<UIFont> {
        let mut v = UIFont::ALL.to_vec();
        v.sort_by(|a, b| compare_ignore_case(a.name(), b.name()));
        v
    }

    pub fn is_any_font_loading(&self) -> bool {
        self.enum_to_font
            .iter()
            .chain(&self.normal)
            .flatten()
            .any(|f| f.is_loading())
    }

    /// `isUsingNonEnglishFonts`.
    pub fn is_using_non_english_fonts(&self, lang: &str, known: &dyn Fn(&str) -> bool) -> bool {
        !lang.eq_ignore_ascii_case("EN") && known(&format!("media/fonts/{lang}/fonts.txt"))
    }

    pub fn get_current_code_font(&self) -> UIFont {
        self.current_code_font
    }

    // --- drawing -----------------------------------------------------------

    /// `DrawString(x, y, str)` / `DrawString(x, y, str, r, g, b, a)`: the Small font.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_default(
        &self,
        dl: &mut DrawList,
        x: f64,
        y: f64,
        s: &str,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        if let Some(f) = self.font() {
            f.draw_string(dl, x as f32, y as f32, s, rgba(r, g, b, a));
        }
    }

    /// `DrawString(font, x, y, str, r, g, b, a)` (also `DrawStringUntrimmed`).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string(
        &self,
        dl: &mut DrawList,
        font: Option<UIFont>,
        x: f64,
        y: f64,
        s: &str,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        if let Some(f) = self.get_font_from_enum(font) {
            f.draw_string(dl, x as f32, y as f32, s, rgba(r, g, b, a));
        }
    }

    /// `DrawString(font, x, y, zoom, str, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_zoom(
        &self,
        dl: &mut DrawList,
        font: Option<UIFont>,
        x: f64,
        y: f64,
        zoom: f64,
        s: &str,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        if let Some(f) = self.get_font_from_enum(font) {
            f.draw_string_scaled(dl, x as f32, y as f32, zoom as f32, s, rgba(r, g, b, a));
        }
    }

    /// `DrawStringCentre(font, x, y, str, r, g, b, a)`: `x - width / 2`
    /// (integer division).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_centre(
        &self,
        dl: &mut DrawList,
        font: Option<UIFont>,
        x: f64,
        y: f64,
        s: &str,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        if let Some(f) = self.get_font_from_enum(font) {
            let x = x - (f.width(s) / 2) as f64;
            f.draw_string(dl, x as f32, y as f32, s, rgba(r, g, b, a));
        }
    }

    /// `DrawStringRight(font, x, y, str, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_right(
        &self,
        dl: &mut DrawList,
        font: Option<UIFont>,
        x: f64,
        y: f64,
        s: &str,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        if let Some(f) = self.get_font_from_enum(font) {
            let x = x - f.width(s) as f64;
            f.draw_string(dl, x as f32, y as f32, s, rgba(r, g, b, a));
        }
    }

    /// `DrawStringCentreDefered`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_string_centre_defered(
        &mut self,
        font: Option<UIFont>,
        x: f64,
        y: f64,
        s: &str,
        r: f64,
        g: f64,
        b: f64,
        a: f64,
    ) {
        self.todo_text_list.push(DeferedTextDraw {
            font,
            x,
            y,
            str: s.to_string(),
            r,
            g,
            b,
            a,
        });
    }

    /// `DrawTextFromGameWorld`.
    pub fn draw_text_from_game_world(&mut self, dl: &mut DrawList) {
        let list = std::mem::take(&mut self.todo_text_list);
        for d in &list {
            self.draw_string_centre(dl, d.font, d.x, d.y, &d.str, d.r, d.g, d.b, d.a);
        }
    }

    // --- measuring ---------------------------------------------------------

    /// `MeasureStringX(font, str)`.
    pub fn measure_string_x(&self, font: Option<UIFont>, s: Option<&str>) -> i32 {
        if self.server_without_gui {
            return 0;
        }
        let Some(s) = s else { return 0 };
        self.get_font_from_enum(font).map_or(0, |f| f.width(s))
    }

    fn measure_x16(&self, font: Option<UIFont>, s: &[u16]) -> i32 {
        if self.server_without_gui {
            return 0;
        }
        self.get_font_from_enum(font)
            .map_or(0, |f| f.get_width(s, 0, s.len() as i32 - 1, false))
    }

    /// `MeasureStringY(font, str, returnActualHeight, returnOffset)`.
    pub fn measure_string_y_ex(
        &self,
        font: Option<UIFont>,
        s: Option<&str>,
        actual: bool,
        offset: bool,
    ) -> i32 {
        let (Some(font), Some(s)) = (font, s) else {
            return 0;
        };
        if self.server_without_gui {
            return 0;
        }
        let u: Vec<u16> = s.encode_utf16().collect();
        self.get_font_from_enum(Some(font))
            .map_or(0, |f| f.get_height(&u, actual, offset))
    }

    /// `MeasureStringY`.
    pub fn measure_string_y(&self, font: Option<UIFont>, s: Option<&str>) -> i32 {
        self.measure_string_y_ex(font, s, false, false)
    }

    /// `MeasureStringYReal`.
    pub fn measure_string_y_real(&self, font: Option<UIFont>, s: Option<&str>) -> i32 {
        self.measure_string_y_ex(font, s, true, false)
    }

    /// `MeasureStringYOffset`.
    pub fn measure_string_y_offset(&self, font: Option<UIFont>, s: Option<&str>) -> i32 {
        self.measure_string_y_ex(font, s, false, true)
    }

    /// `CentreStringYOffset`.
    pub fn centre_string_y_offset(&self, font: Option<UIFont>, s: Option<&str>) -> i32 {
        self.measure_string_y_offset(font, s)
            - (self.measure_string_y(font, s) - self.measure_string_y_real(font, s)) / 2
    }

    /// `MeasureFont`: fixed sizes for the classic fonts, else line height.
    pub fn measure_font(&self, font: Option<UIFont>) -> i32 {
        match font {
            Some(UIFont::Small) => 10,
            Some(UIFont::Dialogue | UIFont::Medium) => 20,
            Some(UIFont::Large) => 24,
            Some(UIFont::Massive | UIFont::MainMenu1 | UIFont::MainMenu2) => 30,
            f => self.get_font_height(f),
        }
    }

    /// `WrapText(font, str, maxWidth)`.
    pub fn wrap_text(&self, font: Option<UIFont>, s: &str, max_width: i32) -> String {
        self.wrap_text_ex(font, s, max_width, -1, "")
    }

    /// `WrapText(font, str, maxWidth, maxLines, maxLinesSuffix)`.
    pub fn wrap_text_ex(
        &self,
        font: Option<UIFont>,
        s: &str,
        max_width: i32,
        max_lines: i32,
        suffix: &str,
    ) -> String {
        let s: Vec<u16> = s.encode_utf16().collect();
        let suffix: Vec<u16> = suffix.encode_utf16().collect();
        let mx = |v: &[u16]| self.measure_x16(font, v);
        let mut out: Vec<Vec<u16>> = Vec::new();
        let space_width = mx(&[0x20]);
        for line in java_split_newlines(&s) {
            if mx(&line) <= max_width {
                out.push(line);
                continue;
            }
            let words = java_split_char(&line, 0x20);
            let mut new_line: Vec<Vec<u16>> = Vec::new();
            let mut cur = 0;
            for (wi, word) in words.iter().enumerate() {
                let new_len = cur + mx(word);
                if new_len > max_width {
                    if new_line.is_empty() {
                        let len = word.len() as i64;
                        let approx =
                            ((max_width as f64 / new_len as f64 * len as f64).floor() as i32 - 1)
                                .max(5) as i64;
                        let lines = (len as f64 / approx as f64).ceil() as i64;
                        for l in 0..(lines - 1).max(0) {
                            let mut part =
                                word[(l * approx) as usize..((l + 1) * approx) as usize].to_vec();
                            part.push(0x2d);
                            out.push(part);
                        }
                        // The original throws for an empty word here
                        // (substring(-approx)); that needs maxWidth < 0.
                        let start = ((lines - 1) * approx).max(0) as usize;
                        let final_part = word[start..].to_vec();
                        cur = mx(&final_part) + space_width;
                        new_line.push(final_part);
                    } else {
                        out.push(join(&new_line, 0x20));
                        new_line.clear();
                        new_line.push(word.clone());
                        cur = mx(word) + space_width;
                    }
                } else {
                    new_line.push(word.clone());
                    cur += mx(word) + space_width;
                }
                if wi == words.len() - 1 {
                    out.push(join(&new_line, 0x20));
                    cur = 0;
                    new_line.clear();
                }
            }
        }
        if max_lines > 0 && out.len() > max_lines as usize {
            let i = max_lines as usize - 1;
            let suffix_len = mx(&suffix);
            if max_width - mx(&out[i]) < suffix_len {
                let keep = out[i].len().saturating_sub(suffix.len());
                out[i].truncate(keep);
            }
            out[i].extend_from_slice(&suffix);
            out.truncate(max_lines as usize);
        }
        String::from_utf16_lossy(&join(&out, 0x0a))
    }
}

fn join(parts: &[Vec<u16>], sep: u16) -> Vec<u16> {
    let mut v = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            v.push(sep);
        }
        v.extend_from_slice(p);
    }
    v
}

/// Drops trailing empty strings, as `String.split` does.
fn trim_trailing_empty(mut v: Vec<Vec<u16>>) -> Vec<Vec<u16>> {
    while v.last().is_some_and(Vec::is_empty) {
        v.pop();
    }
    v
}

/// `s.split("\\r?\\n")`.
fn java_split_newlines(s: &[u16]) -> Vec<Vec<u16>> {
    if !s.contains(&0x0a) {
        return vec![s.to_vec()];
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &c) in s.iter().enumerate() {
        if c == 0x0a {
            let end = if i > start && s[i - 1] == 0x0d {
                i - 1
            } else {
                i
            };
            out.push(s[start..end].to_vec());
            start = i + 1;
        }
    }
    out.push(s[start..].to_vec());
    trim_trailing_empty(out)
}

/// `s.split(" ")` (single-char fast path).
fn java_split_char(s: &[u16], sep: u16) -> Vec<Vec<u16>> {
    if !s.contains(&sep) {
        return vec![s.to_vec()];
    }
    trim_trailing_empty(s.split(|&c| c == sep).map(<[u16]>::to_vec).collect())
}

/// `String.compareToIgnoreCase`.
fn compare_ignore_case(a: &str, b: &str) -> std::cmp::Ordering {
    let fold = |c: char| {
        c.to_uppercase()
            .next()
            .unwrap_or(c)
            .to_lowercase()
            .next()
            .unwrap_or(c)
    };
    a.chars().map(fold).cmp(b.chars().map(fold))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_java() {
        let u = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(java_split_newlines(&u("a\r\nb\n\n")), vec![u("a"), u("b")]);
        assert!(java_split_newlines(&u("\n")).is_empty());
        assert_eq!(
            java_split_char(&u(" a  b "), 0x20),
            vec![u(""), u("a"), u(""), u("b")]
        );
    }

    #[test]
    fn fonts_file() {
        let mut m = HashMap::new();
        let src = "VERSION = 1,font Small{    fnt = a.fnt,    img = a_0.png,}font Title{    fnt = t.fnt,    type = sdf,}";
        fonts_from_string(src, &mut m).unwrap();
        assert_eq!(m["Small"].img.as_deref(), Some("a_0.png"));
        assert_eq!(m["Title"].r#type.as_deref(), Some("sdf"));
        assert!(fonts_from_string("font X {fnt = x,}", &mut HashMap::new()).is_err());
    }

    fn manager() -> TextManager {
        let fnt = "info\ncommon\nchar id=32 x=0 y=0 width=0 height=0 xoffset=0 yoffset=0 xadvance=3 page=0\n\
char id=45 x=0 y=0 width=3 height=1 xoffset=0 yoffset=5 xadvance=4 page=0\n\
char id=97 x=0 y=0 width=5 height=6 xoffset=0 yoffset=2 xadvance=6 page=0\n";
        let mut tm = TextManager::default();
        tm.enum_to_font[UIFont::Small as usize] = Some(Rc::new(AngelCodeFont::parse(fnt).unwrap()));
        tm.font = tm.enum_to_font[UIFont::Small as usize].clone();
        tm
    }

    #[test]
    fn wrap_text() {
        let tm = manager();
        assert_eq!(tm.measure_string_x(Some(UIFont::Medium), Some("aa")), 11);
        assert_eq!(
            tm.wrap_text(Some(UIFont::Small), "aa aa aa", 26),
            "aa aa\naa"
        );
        // A word wider than the line is split with hyphens.
        assert_eq!(tm.wrap_text(None, "aaaaaaaaaaaa", 30), "aaaaa-\naaaaa-\naa");
        assert_eq!(
            tm.wrap_text_ex(None, "aa aa aa aa", 11, 2, "..."),
            "aa\naa..."
        );
    }
}
