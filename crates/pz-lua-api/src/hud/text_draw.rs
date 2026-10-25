//! `zombie.ui.TextDrawObject` (BBCode-ish rich text: `[col=r,g,b]`,
//! `[fnt=Name]`, `[img=name]`, `[br/]`, `[cdt=seconds]`, closed by `[/]`)
//! and `zombie.ui.TextDrawHorizontal`.

use super::host::with_host;
use pz_render::{DrawCmd, Rgba, Texture};
use pz_text::UIFont;
use pz_ui::{with_ui, TextAlign, UiDrawCmd};
use std::cell::RefCell;
use std::rc::Rc;

pub const TEXT_DRAW_OBJECT: &str = "zombie.ui.TextDrawObject";
pub const TEXT_DRAW_HORIZONTAL: &str = "zombie.ui.TextDrawHorizontal";

/// `TextDrawHorizontal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextDrawHorizontal {
    Left,
    #[default]
    Center,
    Right,
}

impl TextDrawHorizontal {
    pub const NAMES: [&'static str; 3] = ["Left", "Center", "Right"];
    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    pub fn from_name(n: &str) -> Option<Self> {
        match n {
            "Left" => Some(Self::Left),
            "Center" => Some(Self::Center),
            "Right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// `Character.isWhitespace`.
fn java_is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | '\u{001C}'..='\u{001F}'
    ) || (c.is_whitespace()
        && !matches!(c, '\u{00A0}' | '\u{2007}' | '\u{202F}')
        && !c.is_control())
}

/// `String.trim()`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `String.split(",")`: trailing empty strings dropped.
fn split_comma(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.split(',').map(str::to_string).collect();
    while v.len() > 1 && v.last().is_some_and(String::is_empty) {
        v.pop();
    }
    if v.len() == 1 && v[0].is_empty() && !s.is_empty() {
        v.clear();
    }
    v
}

/// `String.split("\\s+")` (`\s` is `[ \t\n\x0B\f\r]`).
fn split_ws(s: &str) -> Vec<String> {
    let is = |c: char| matches!(c, ' ' | '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r');
    let mut v: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_ws = false;
    for c in s.chars() {
        if is(c) {
            if !in_ws {
                v.push(std::mem::take(&mut cur));
                in_ws = true;
            }
        } else {
            in_ws = false;
            cur.push(c);
        }
    }
    v.push(cur);
    // A leading separator gives a leading empty string only when it has
    // width (always, here); trailing empties are removed.
    while v.len() > 1 && v.last().is_some_and(String::is_empty) {
        v.pop();
    }
    if v.len() == 1 && v[0].is_empty() && !s.is_empty() {
        v.clear();
    }
    v
}

/// `Float.parseFloat` for the `[cdt=]` value.
fn java_parse_float(s: &str) -> Option<f32> {
    let t = java_trim(s);
    let t = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    if t.is_empty() || t.contains(|c: char| c.is_ascii_alphabetic() && !matches!(c, 'e' | 'E')) {
        return None;
    }
    t.parse::<f32>().ok()
}

#[derive(Clone)]
struct DrawElement {
    text: String,
    scramble_text: String,
    current_scramble_val: f32,
    f: UIFont,
    r: f32,
    g: f32,
    b: f32,
    w: i32,
    h: i32,
    is_image: bool,
    use_font: bool,
    use_color: bool,
    tex: Option<Texture>,
    is_text_image: bool,
}

impl Default for DrawElement {
    fn default() -> Self {
        DrawElement {
            text: String::new(),
            scramble_text: String::new(),
            current_scramble_val: 0.0,
            f: UIFont::AutoNormSmall,
            r: 1.0,
            g: 1.0,
            b: 1.0,
            w: 0,
            h: 0,
            is_image: false,
            use_font: false,
            use_color: false,
            tex: None,
            is_text_image: false,
        }
    }
}

impl DrawElement {
    /// `reset()` (keeps `currentScrambleVal`, as the original does).
    fn reset(&mut self) {
        let c = self.current_scramble_val;
        *self = DrawElement::default();
        self.current_scramble_val = c;
    }

    fn add_text(&mut self, t: &str) {
        self.text.push_str(t);
    }

    /// `scrambleText(val)`: each word dotted out with chance `val`.
    fn scramble(&mut self, val: f32) {
        if val == self.current_scramble_val {
            return;
        }
        self.current_scramble_val = val;
        let threshold = (val * 100.0) as i32;
        self.scramble_text.clear();
        for word in split_ws(&self.text) {
            let r = pz_core::random::next(100);
            if r > threshold {
                self.scramble_text.push_str(&word);
            } else {
                self.scramble_text
                    .extend(std::iter::repeat_n('.', word.encode_utf16().count()));
            }
            self.scramble_text.push(' ');
        }
    }

    /// `softclone()`: the colour and font carry over a line break.
    fn softclone(&self) -> DrawElement {
        let mut c = DrawElement::default();
        if self.use_color {
            (c.r, c.g, c.b, c.use_color) = (self.r, self.g, self.b, true);
        }
        if self.use_font {
            (c.f, c.use_font) = (self.f, true);
        }
        c
    }
}

#[derive(Default)]
struct DrawLine {
    elements: Vec<DrawElement>,
    h: i32,
    w: i32,
    char_w: i32,
}

/// The 9-slice `ChatElement.backdropTexture` (`NineGridTexture`).
#[derive(Clone)]
pub struct NineGridTexture {
    parts: [Option<Texture>; 9],
    outer: i32,
}

impl NineGridTexture {
    /// `new NineGridTexture(base, outer)`.
    pub fn new(base: &str, outer: i32) -> Self {
        let n = [
            "topleft", "topmid", "topright", "left", "mid", "right", "botleft", "botmid",
            "botright",
        ];
        NineGridTexture {
            parts: n.map(|s| pz_render::get_shared_texture(&format!("{base}_{s}"))),
            outer,
        }
    }

    /// `renderInnerBased(x, y, w, h, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_inner_based(&self, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        let o = self.outer;
        let y = y + 5;
        let h = h - 7;
        let p = &self.parts;
        with_ui(|m| {
            let s = &mut m.sprite;
            s.renderi(p[0].as_ref(), x - o, y - o, o, o, c);
            s.renderi(p[1].as_ref(), x, y - o, w, o, c);
            s.renderi(p[2].as_ref(), x + w, y - o, o, o, c);
            s.renderi(p[3].as_ref(), x - o, y, o, h, c);
            s.renderi(p[4].as_ref(), x, y, w, h, c);
            s.renderi(p[5].as_ref(), x + w, y, o, h, c);
            s.renderi(p[6].as_ref(), x - o, y + h, o, o, c);
            s.renderi(p[7].as_ref(), x, y + h, w, o, c);
            s.renderi(p[8].as_ref(), x + w, y + h, o, o, c);
        });
    }
}

thread_local! {
    /// `ChatElement.backdropTexture`: null until a `ChatElement` exists.
    static BACKDROP: RefCell<Option<NineGridTexture>> = const { RefCell::new(None) };
    /// `TextDrawObject.renderBatch`.
    static RENDER_BATCH: RefCell<Vec<RenderBatch>> = const { RefCell::new(Vec::new()) };
}

/// `new ChatElement(...)`'s `backdropTexture = new NineGridTexture(
/// "NineGridBlack", 5)` when unset; the chat port calls this.
pub fn ensure_chat_backdrop() {
    BACKDROP.with(|b| {
        let mut b = b.borrow_mut();
        if b.is_none() {
            *b = Some(NineGridTexture::new("NineGridBlack", 5));
        }
    });
}

/// `ChatElement.backdropTexture` (for the chat port).
pub fn chat_backdrop() -> Option<NineGridTexture> {
    BACKDROP.with(|b| b.borrow().clone())
}

#[cfg(test)]
pub(crate) fn clear_chat_backdrop() {
    BACKDROP.with(|b| *b.borrow_mut() = None);
}

struct RenderBatch {
    player_num: i32,
    element: Rc<RefCell<TextDrawObject>>,
    horz: TextDrawHorizontal,
    x: f64,
    y: f64,
    color: Rgba,
    draw_outlines: bool,
}

/// `zombie.ui.TextDrawObject`.
pub struct TextDrawObject {
    valid_images: Vec<String>,
    valid_fonts: Vec<String>,
    lines: Vec<DrawLine>,
    width: i32,
    height: i32,
    max_chars_line: i32,
    default_font_enum: UIFont,
    /// `defaultFont != null` (set by the first `ReadString`).
    default_font_loaded: bool,
    original: String,
    unformatted: String,
    has_opened: bool,
    draw_background: bool,
    allow_images: bool,
    allow_chat_icons: bool,
    allow_colors: bool,
    allow_fonts: bool,
    allow_bbcode: bool,
    allow_any_image: bool,
    allow_line_breaks: bool,
    equalize_line_heights: bool,
    enabled: bool,
    visible_radius: i32,
    scramble_val: f32,
    outline: Rgba,
    default_color: Rgba,
    hear_range: i32,
    internal_clock: f32,
    custom_tag: String,
    custom_image_max_dim: i32,
    default_horz: TextDrawHorizontal,
}

impl Default for TextDrawObject {
    fn default() -> Self {
        Self::new(255, 255, 255, true, true, true, true, true, false)
    }
}

impl TextDrawObject {
    /// `new TextDrawObject(r, g, b, allowBBcode, allowImages, allowChatIcons,
    /// allowColors, allowFonts, equalizeLineHeights)`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        r: i32,
        g: i32,
        b: i32,
        bbcode: bool,
        images: bool,
        chat_icons: bool,
        colors: bool,
        fonts: bool,
        equalize: bool,
    ) -> Self {
        let mut t = TextDrawObject {
            valid_images: [
                "Icon_music_notes",
                "media/ui/CarKey.png",
                "media/ui/ArrowUp.png",
                "media/ui/ArrowDown.png",
            ]
            .map(String::from)
            .to_vec(),
            valid_fonts: ["Small", "Dialogue", "Medium", "Code", "Large", "Massive"]
                .map(String::from)
                .to_vec(),
            lines: Vec::new(),
            width: 0,
            height: 0,
            max_chars_line: -1,
            default_font_enum: UIFont::Dialogue,
            default_font_loaded: false,
            original: String::new(),
            unformatted: String::new(),
            has_opened: false,
            draw_background: false,
            allow_images: true,
            allow_chat_icons: true,
            allow_colors: true,
            allow_fonts: true,
            allow_bbcode: true,
            allow_any_image: false,
            allow_line_breaks: true,
            equalize_line_heights: false,
            enabled: true,
            visible_radius: -1,
            scramble_val: 0.0,
            outline: Rgba(0.0, 0.0, 0.0, 1.0),
            default_color: Rgba(1.0, 1.0, 1.0, 1.0),
            hear_range: -1,
            internal_clock: 0.0,
            custom_tag: "default".into(),
            custom_image_max_dim: 18,
            default_horz: TextDrawHorizontal::Center,
        };
        t.set_settings(bbcode, images, chat_icons, colors, fonts, equalize);
        t.set_default_colors_i(r, g, b, 255);
        t
    }

    pub fn set_enabled(&mut self, v: bool) {
        self.enabled = v;
    }
    pub fn get_enabled(&self) -> bool {
        self.enabled
    }
    pub fn set_visible_radius(&mut self, v: i32) {
        self.visible_radius = v;
    }
    pub fn get_visible_radius(&self) -> i32 {
        self.visible_radius
    }
    pub fn set_draw_background(&mut self, v: bool) {
        self.draw_background = v;
    }
    pub fn set_allow_images(&mut self, v: bool) {
        self.allow_images = v;
    }
    pub fn set_allow_chat_icons(&mut self, v: bool) {
        self.allow_chat_icons = v;
    }
    pub fn set_allow_colors(&mut self, v: bool) {
        self.allow_colors = v;
    }
    pub fn set_allow_fonts(&mut self, v: bool) {
        self.allow_fonts = v;
    }
    pub fn set_allow_bbcode(&mut self, v: bool) {
        self.allow_bbcode = v;
    }
    pub fn set_allow_any_image(&mut self, v: bool) {
        self.allow_any_image = v;
    }
    pub fn set_allow_line_breaks(&mut self, v: bool) {
        self.allow_line_breaks = v;
    }
    pub fn set_equalize_line_heights(&mut self, v: bool) {
        self.equalize_line_heights = v;
        self.calculate_dimensions();
    }
    pub fn set_settings(
        &mut self,
        bbcode: bool,
        images: bool,
        chat_icons: bool,
        colors: bool,
        fonts: bool,
        equalize: bool,
    ) {
        self.allow_images = images;
        self.allow_chat_icons = chat_icons;
        self.allow_colors = colors;
        self.allow_fonts = fonts;
        self.allow_bbcode = bbcode;
        self.equalize_line_heights = equalize;
    }
    pub fn set_custom_tag(&mut self, t: &str) {
        self.custom_tag = t.into();
    }
    pub fn get_custom_tag(&self) -> &str {
        &self.custom_tag
    }
    pub fn set_valid_images(&mut self, v: Vec<String>) {
        self.valid_images = v;
    }
    pub fn set_valid_fonts(&mut self, v: Vec<String>) {
        self.valid_fonts = v;
    }
    /// `setMaxCharsPerLine(n)`: re-reads the text wrapped at `n`.
    pub fn set_max_chars_per_line(&mut self, n: i32) {
        if n <= 0 {
            return;
        }
        let s = self.original.clone();
        self.read_string_with(self.default_font_enum, &s, n);
    }
    pub fn set_custom_image_max_dimensions(&mut self, d: i32) {
        if d < 1 {
            return;
        }
        self.custom_image_max_dim = d;
        self.calculate_dimensions();
    }
    pub fn set_outline_colors(&mut self, c: Rgba) {
        self.outline = c;
    }
    pub fn set_outline_colors_i(&mut self, r: i32, g: i32, b: i32, a: i32) {
        self.outline = Rgba(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        );
    }
    pub fn set_default_colors(&mut self, c: Rgba) {
        self.default_color = c;
    }
    pub fn set_default_colors_i(&mut self, r: i32, g: i32, b: i32, a: i32) {
        self.default_color = Rgba(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        );
    }
    /// `setHorizontalAlign(String)`: "left", "center", "right"; others
    /// keep the current alignment.
    pub fn set_horizontal_align_str(&mut self, s: &str) {
        match s {
            "left" => self.default_horz = TextDrawHorizontal::Left,
            "center" => self.default_horz = TextDrawHorizontal::Center,
            "right" => self.default_horz = TextDrawHorizontal::Right,
            _ => {}
        }
    }
    pub fn set_horizontal_align(&mut self, h: TextDrawHorizontal) {
        self.default_horz = h;
    }
    pub fn get_horizontal_align(&self) -> TextDrawHorizontal {
        self.default_horz
    }
    pub fn get_original(&self) -> &str {
        &self.original
    }
    /// `getUnformatted()`: the scrambled words while scrambled.
    pub fn get_unformatted(&self) -> String {
        if self.scramble_val > 0.0 {
            let mut s = String::new();
            for l in &self.lines {
                for e in l.elements.iter().filter(|e| !e.is_image) {
                    s.push_str(&e.scramble_text);
                }
            }
            return s;
        }
        self.unformatted.clone()
    }
    pub fn get_width(&self) -> i32 {
        self.width
    }
    pub fn get_height(&self) -> i32 {
        self.height
    }
    pub fn get_default_font_enum(&self) -> UIFont {
        self.default_font_enum
    }
    pub fn is_null_or_zero_length(&self) -> bool {
        self.original.is_empty()
    }
    pub fn get_internal_clock(&self) -> f32 {
        self.internal_clock
    }
    pub fn set_internal_tick_clock(&mut self, t: f32) {
        if t > 0.0 {
            self.internal_clock = t;
        }
    }
    /// `updateInternalTickClock()`: by `1.25 * GameTime.getMultiplier()`.
    pub fn update_internal_tick_clock(&mut self) -> f32 {
        let m = with_host(|h| h.game_time.get_multiplier());
        self.update_internal_tick_clock_by(1.25 * m)
    }
    pub fn update_internal_tick_clock_by(&mut self, d: f32) -> f32 {
        if self.internal_clock <= 0.0 {
            return 0.0;
        }
        self.internal_clock -= d;
        if self.internal_clock <= 0.0 {
            self.internal_clock = 0.0;
        }
        self.internal_clock
    }
    pub fn set_scramble_val(&mut self, v: f32) {
        if self.scramble_val == v {
            return;
        }
        self.scramble_val = v;
        if v > 0.0 {
            for l in &mut self.lines {
                for e in l.elements.iter_mut().filter(|e| !e.is_image) {
                    e.scramble(v);
                }
            }
        }
    }
    pub fn get_scramble_val(&self) -> f32 {
        self.scramble_val
    }
    pub fn set_hear_range(&mut self, r: i32) {
        self.hear_range = r.max(0);
    }
    pub fn get_hear_range(&self) -> i32 {
        self.hear_range
    }

    fn is_valid_font(&self, f: &str) -> bool {
        self.valid_fonts
            .iter()
            .any(|s| s == f && crate::bind_ui::ui_font_from_name(f).is_some())
    }

    fn is_valid_image(&self, i: &str) -> bool {
        self.valid_images.iter().any(|s| s == i)
    }

    fn try_color_int(s: &str) -> i32 {
        if s.is_empty() || s.encode_utf16().count() > 3 {
            return -1;
        }
        // Integer.parseInt: an optional sign, then digits.
        let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return -1;
        }
        match s.parse::<i32>() {
            Ok(i) if (0..256).contains(&i) => i,
            _ => -1,
        }
    }

    fn read_tag_value(chars: &[char], pos: usize) -> Option<String> {
        if chars.get(pos) != Some(&'=') {
            return None;
        }
        let mut s = String::new();
        for &c in &chars[pos + 1..] {
            if c == ']' {
                return Some(s);
            }
            s.push(c);
        }
        None
    }

    /// `Clear()`.
    pub fn clear(&mut self) {
        self.original.clear();
        self.unformatted.clear();
        self.reset();
    }

    fn reset(&mut self) {
        self.lines.clear();
        self.lines.push(DrawLine {
            elements: vec![DrawElement::default()],
            ..Default::default()
        });
        self.enabled = true;
        self.scramble_val = 0.0;
    }

    fn cur_line(&mut self) -> &mut DrawLine {
        self.lines.last_mut().unwrap()
    }

    fn cur(&mut self) -> &mut DrawElement {
        self.cur_line().elements.last_mut().unwrap()
    }

    fn add_new_line(&mut self) {
        let e = self.cur().softclone();
        self.lines.push(DrawLine {
            elements: vec![e],
            ..Default::default()
        });
    }

    fn add_text(&mut self, w: &str) {
        self.cur().add_text(w);
        self.cur_line().char_w += w.encode_utf16().count() as i32;
    }

    fn add_word(&mut self, word: &str) {
        let len = word.chars().count() as i32;
        if self.max_chars_line <= 0 || self.cur_line().char_w + len < self.max_chars_line {
            self.add_text(word);
            return;
        }
        let chars: Vec<char> = word.chars().collect();
        let m = self.max_chars_line;
        for i in 0..len / m + 1 {
            let s = i * m;
            let e = if s + m < len { s + m } else { len };
            if s >= e {
                continue;
            }
            if i > 0 || self.cur_line().char_w != 0 {
                self.add_new_line();
            }
            let part: String = chars[s as usize..e as usize].iter().collect();
            self.add_text(&part);
        }
    }

    fn add_new_element(&mut self) {
        if self.cur().text.is_empty() {
            self.cur().reset();
        } else {
            self.cur_line().elements.push(DrawElement::default());
        }
    }

    /// `readTag(chars, pos, tag)`: the index to continue from, or -1.
    fn read_tag(&mut self, chars: &[char], pos: usize, tag: &str) -> i32 {
        if self.allow_fonts && tag == "fnt" {
            if let Some(val) = Self::read_tag_value(chars, pos) {
                if self.is_valid_font(&val) {
                    self.add_new_element();
                    let f = crate::bind_ui::ui_font_from_name(&val).unwrap();
                    let e = self.cur();
                    e.f = f;
                    e.use_font = true;
                    self.has_opened = true;
                    return (pos + val.chars().count() + 1) as i32;
                }
            }
        } else if (self.allow_images || self.allow_chat_icons) && tag == "img" {
            if let Some(img) = Self::read_tag_value(chars, pos) {
                if !java_trim(&img).is_empty() {
                    self.add_new_element();
                    let imglen = img.chars().count();
                    let cols = split_comma(&img);
                    let img = if cols.len() > 1 { cols[0].clone() } else { img };
                    let mut text = java_trim(&img).to_string();
                    if text == "music" {
                        text = "Icon_music_notes".into();
                    }
                    let (chat_icons, images, any) = (
                        self.allow_chat_icons,
                        self.allow_images,
                        self.allow_any_image,
                    );
                    let valid = self.is_valid_image(&text);
                    let e = self.cur();
                    e.is_image = true;
                    e.text = text;
                    if chat_icons && valid {
                        e.tex = pz_render::get_shared_texture(&e.text);
                        e.is_text_image = true;
                    } else if images {
                        e.tex = pz_render::get_shared_texture(&format!("Item_{}", e.text));
                        if e.tex.is_none() {
                            e.tex = pz_render::get_shared_texture(&format!(
                                "media/ui/Container_{}",
                                e.text
                            ));
                        }
                        if e.tex.is_some() {
                            e.is_text_image = false;
                            e.text = format!("Item_{}", e.text);
                        }
                    }
                    if any && e.tex.is_none() {
                        e.tex = pz_render::get_shared_texture(&e.text);
                        if e.tex.is_some() {
                            e.is_text_image = false;
                        }
                    }
                    if cols.len() == 4 {
                        let (r, g, b) = (
                            Self::try_color_int(&cols[1]),
                            Self::try_color_int(&cols[2]),
                            Self::try_color_int(&cols[3]),
                        );
                        if r != -1 && g != -1 && b != -1 {
                            e.use_color = true;
                            (e.r, e.g, e.b) =
                                (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
                        }
                    }
                    self.add_new_element();
                    return (pos + imglen + 1) as i32;
                }
            }
        } else if self.allow_colors && tag == "col" {
            if let Some(color) = Self::read_tag_value(chars, pos) {
                let cols = split_comma(&color);
                if cols.len() == 3 {
                    let (r, g, b) = (
                        Self::try_color_int(&cols[0]),
                        Self::try_color_int(&cols[1]),
                        Self::try_color_int(&cols[2]),
                    );
                    if r != -1 && g != -1 && b != -1 {
                        self.add_new_element();
                        let e = self.cur();
                        e.use_color = true;
                        (e.r, e.g, e.b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
                        self.has_opened = true;
                        return (pos + color.chars().count() + 1) as i32;
                    }
                }
            }
        } else if tag == "cdt" {
            if let Some(time) = Self::read_tag_value(chars, pos) {
                // A bad number logs and keeps the clock.
                self.internal_clock =
                    java_parse_float(&time).map_or(self.internal_clock, |t| t * 60.0);
                return (pos + time.chars().count() + 1) as i32;
            }
        }
        -1
    }

    /// `setDefaultFont(f)`.
    pub fn set_default_font(&mut self, f: UIFont) {
        if f != self.default_font_enum {
            let s = self.original.clone();
            self.read_string_with(f, &s, self.max_chars_line);
        }
    }

    /// `ReadString(str)`.
    pub fn read_string(&mut self, s: &str) {
        self.read_string_with(self.default_font_enum, s, self.max_chars_line);
    }

    /// `ReadString(font, str, maxLineWidth)`.
    pub fn read_string_with(&mut self, font: UIFont, s: &str, max_line: i32) {
        self.reset();
        if !self.default_font_loaded || font != self.default_font_enum {
            self.default_font_enum = font;
            self.default_font_loaded = true;
        }
        self.max_chars_line = max_line;
        self.original = s.to_string();
        let chars: Vec<char> = s.chars().collect();
        self.has_opened = false;
        let mut word = String::new();
        let mut i = 0usize;
        while i < chars.len() {
            let c = chars[i];
            if self.allow_bbcode && c == '[' {
                if !word.is_empty() {
                    self.add_word(&word);
                    word.clear();
                }
                if i + 4 < chars.len() {
                    let tag: String = [chars[i + 1], chars[i + 2], chars[i + 3]]
                        .iter()
                        .collect::<String>()
                        .to_lowercase();
                    if self.allow_line_breaks && tag == "br/" {
                        self.add_new_line();
                        i += 5;
                        continue;
                    }
                    if !self.has_opened {
                        let pos = self.read_tag(&chars, i + 4, &tag);
                        if pos >= 0 {
                            i = pos as usize + 1;
                            continue;
                        }
                    }
                }
                if self.has_opened
                    && i + 2 < chars.len()
                    && chars[i + 1] == '/'
                    && chars[i + 2] == ']'
                {
                    self.has_opened = false;
                    self.add_new_element();
                    i += 3;
                    continue;
                }
            }
            if java_is_whitespace(c) && i > 0 && !java_is_whitespace(chars[i - 1]) {
                self.add_word(&word);
                word.clear();
            }
            word.push(c);
            self.unformatted.push(c);
            i += 1;
        }
        if !word.is_empty() {
            self.add_word(&word);
        }
        self.calculate_dimensions();
    }

    /// `calculateDimensions()`.
    pub fn calculate_dimensions(&mut self) {
        self.width = 0;
        self.height = 0;
        let mut max_h = 0;
        let def = self.default_font_loaded.then_some(self.default_font_enum);
        for d in &mut self.lines {
            d.h = 0;
            d.w = 0;
            for de in &mut d.elements {
                de.w = 0;
                de.h = 0;
                if let (true, Some(t)) = (de.is_image, de.tex.as_ref()) {
                    let (tw, th) = (t.width as i32, t.height as i32);
                    if de.is_text_image {
                        (de.w, de.h) = (tw, th);
                    } else {
                        (de.w, de.h) = ((tw as f32 * 0.75) as i32, (th as f32 * 0.75) as i32);
                    }
                } else if de.use_font {
                    de.w = pz_ui::draw::measure_string_x(Some(de.f), &de.text);
                    de.h = pz_ui::draw::measure_string_y(Some(de.f), &de.text);
                } else if let Some(f) = def {
                    de.w = pz_ui::draw::measure_string_x(Some(f), &de.text);
                    de.h = pz_ui::draw::measure_string_y(Some(f), &de.text);
                }
                d.w += de.w;
                if de.h > d.h {
                    d.h = de.h;
                }
            }
            if d.w > self.width {
                self.width = d.w;
            }
            self.height += d.h;
            if d.h > max_h {
                max_h = d.h;
            }
        }
        if self.equalize_line_heights {
            self.height = 0;
            for l in &mut self.lines {
                l.h = max_h;
                self.height += max_h;
            }
        }
    }

    /// `Draw(x, y)` and its overloads: `DrawRaw` in the default colour
    /// unless given.
    pub fn draw(&mut self, x: f64, y: f64, outlines: bool, alpha: Option<f32>) {
        let mut c = self.default_color;
        if let Some(a) = alpha {
            c.3 = a;
        }
        self.draw_raw(self.default_horz, x, y, c, outlines);
    }

    /// `DrawRaw(horz, x, y, r, g, b, a, drawOutlines)`.
    pub fn draw_raw(&mut self, horz: TextDrawHorizontal, x: f64, y: f64, c: Rgba, outlines: bool) {
        let (sw, sh) = {
            let core = crate::bind_core::shared_core();
            let core = core.borrow();
            (core.get_screen_width(), core.get_screen_height())
        };
        let mut draw_x = match horz {
            TextDrawHorizontal::Center => x - (self.width / 2) as f64,
            TextDrawHorizontal::Right => x - self.width as f64,
            TextDrawHorizontal::Left => x,
        };
        let mut draw_y = y;
        if draw_x - 20.0 >= sw as f64
            || draw_x + self.width as f64 + 20.0 <= 0.0
            || draw_y - 20.0 >= sh as f64
            || draw_y + self.height as f64 + 20.0 <= 0.0
        {
            return;
        }
        // PerformanceSettings.fboRenderChunk (on in B42).
        with_ui(|m| {
            let cmd = if m.rendering && m.use_ui_fbo {
                // TODO(render): SpriteRenderer.setDefaultStyle(UIFBOStyle.instance).
                DrawCmd::BlendFuncSeparate(770, 771, 1, 771)
            } else {
                DrawCmd::BlendFunc(770, 771)
            };
            m.sprite.push(UiDrawCmd::Render(cmd));
        });
        if self.draw_background {
            if let Some(b) = chat_backdrop() {
                b.render_inner_based(
                    draw_x as i32,
                    draw_y as i32,
                    self.width,
                    self.height,
                    Rgba(0.0, 0.0, 0.0, 0.4 * c.3),
                );
            }
        }
        let o = self.outline;
        let outline_a = if outlines && c.3 < 1.0 {
            o.3 * c.3
        } else {
            o.3
        };
        let oc = Rgba(o.0, o.1, o.2, outline_a);
        let def = self.default_font_loaded.then_some(self.default_font_enum);
        let text_at = |f: UIFont, x: f64, y: f64, s: &str, col: Rgba| {
            pz_ui::draw::draw_string(
                Some(f),
                x as f32 as f64,
                y as f32 as f64,
                1.0,
                s,
                col,
                TextAlign::Left,
                true,
            );
        };
        let offs = [(-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)];
        for line in &self.lines {
            draw_x = match horz {
                TextDrawHorizontal::Center => x - (line.w / 2) as f64,
                TextDrawHorizontal::Right => x - line.w as f64,
                TextDrawHorizontal::Left => x,
            };
            for e in &line.elements {
                let y_off = (line.h / 2 - e.h / 2) as f64;
                let s = if self.scramble_val > 0.0 {
                    &e.scramble_text
                } else {
                    &e.text
                };
                if e.is_image && e.tex.is_some() {
                    let t = e.tex.as_ref();
                    with_ui(|m| {
                        let sp = &mut m.sprite;
                        if outlines && e.is_text_image {
                            for (dx, dy) in offs {
                                sp.renderi(
                                    t,
                                    (draw_x + dx) as i32,
                                    (draw_y + y_off + dy) as i32,
                                    e.w,
                                    e.h,
                                    oc,
                                );
                            }
                        }
                        let col = if e.use_color {
                            Rgba(e.r, e.g, e.b, c.3)
                        } else if e.is_text_image {
                            c
                        } else {
                            Rgba(1.0, 1.0, 1.0, c.3)
                        };
                        sp.renderi(t, draw_x as i32, (draw_y + y_off) as i32, e.w, e.h, col);
                    });
                } else if e.use_font {
                    if outlines {
                        for (dx, dy) in offs {
                            text_at(e.f, draw_x + dx, draw_y + y_off + dy, s, oc);
                        }
                    }
                    text_at(e.f, draw_x, draw_y + y_off, s, c);
                } else if let Some(f) = def {
                    if outlines {
                        for (dx, dy) in offs {
                            text_at(f, draw_x + dx, draw_y + y_off + dy, s, oc);
                        }
                    }
                    let col = if e.use_color {
                        Rgba(e.r, e.g, e.b, c.3)
                    } else {
                        c
                    };
                    text_at(f, draw_x, draw_y + y_off, s, col);
                }
                draw_x += e.w as f64;
            }
            draw_y += line.h as f64;
        }
    }
}

/// `AddBatchedDraw(horz, x, y, r, g, b, a, drawOutlines)`: queued for
/// `RenderBatch(playerNum)` under the current player (not on a server).
pub fn add_batched_draw(
    o: &Rc<RefCell<TextDrawObject>>,
    horz: TextDrawHorizontal,
    x: f64,
    y: f64,
    color: Rgba,
    outlines: bool,
) {
    if with_host(|h| h.network.is_server()) {
        return;
    }
    let player_num = with_host(|h| h.players.player_index());
    RENDER_BATCH.with(|b| {
        b.borrow_mut().push(RenderBatch {
            player_num,
            element: o.clone(),
            horz,
            x,
            y,
            color,
            draw_outlines: outlines,
        })
    });
}

/// `TextDrawObject.RenderBatch(playerNum)`: draws and drops that player's
/// queued draws, in order.
pub fn render_batch(player: i32) {
    let mine: Vec<RenderBatch> = RENDER_BATCH.with(|b| {
        let mut b = b.borrow_mut();
        let (m, rest): (Vec<_>, Vec<_>) = b.drain(..).partition(|r| r.player_num == player);
        *b = rest;
        m
    });
    for r in mine {
        r.element
            .borrow_mut()
            .draw_raw(r.horz, r.x, r.y, r.color, r.draw_outlines);
    }
}

/// `TextDrawObject.NoRender(playerNum)`: drops that player's queued draws.
pub fn no_render(player: i32) {
    RENDER_BATCH.with(|b| b.borrow_mut().retain(|r| r.player_num != player));
}

// ---- Lua ------------------------------------------------------------------------

use crate::expose::{
    bool_arg, enum_value, expose_class, expose_enum, int_arg, num_arg, str_arg, with_this, JavaEnum,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};

/// A `TextDrawObject` as Lua holds it.
pub struct LuaTextDrawObject(pub Rc<RefCell<TextDrawObject>>);

impl LuaObject for LuaTextDrawObject {
    fn class_name(&self) -> &str {
        TEXT_DRAW_OBJECT
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn horz_arg(v: &Value) -> Option<TextDrawHorizontal> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == TEXT_DRAW_HORIZONTAL)
            .and_then(|e| TextDrawHorizontal::from_name(e.name)),
        _ => None,
    }
}

fn font_arg(v: &Value) -> Option<UIFont> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == crate::bind_ui::UIFONT)
            .and_then(|e| crate::bind_ui::ui_font_from_name(e.name)),
        _ => None,
    }
}

fn this(c: &NativeCall<'_>) -> LuaResult<Rc<RefCell<TextDrawObject>>> {
    with_this::<LuaTextDrawObject, _>(c, |o| o.0.clone())
}

/// Every argument from `from` an integral number: the `int` overload of
/// `setDefaultColors` / `setOutlineColors` (0..255), else the `float` one.
fn colors_arg(c: &NativeCall<'_>, from: usize) -> Rgba {
    let n = c.nargs() - from;
    let ints = (from..c.nargs()).all(|i| matches!(c.arg(i), Value::Num(v) if v.fract() == 0.0));
    let get = |i: usize| num_arg(c, from + i);
    let a = if n >= 4 {
        get(3)
    } else if ints {
        255.0
    } else {
        1.0
    };
    if ints {
        let i = |v: f64| v as i32 as f32 / 255.0;
        Rgba(i(get(0)), i(get(1)), i(get(2)), i(a))
    } else {
        Rgba(get(0) as f32, get(1) as f32, get(2) as f32, a as f32)
    }
}

/// The `Draw` / `AddBatchedDraw` overloads as (horz, x, y, colour,
/// outlines), from the argument count after `self`.
fn draw_args(
    c: &NativeCall<'_>,
    o: &TextDrawObject,
) -> LuaResult<(TextDrawHorizontal, f64, f64, Rgba, bool)> {
    let n = c.nargs() - 1;
    let d = o.default_color;
    Ok(match n {
        2 => (o.default_horz, num_arg(c, 1), num_arg(c, 2), d, false),
        3 => (
            o.default_horz,
            num_arg(c, 1),
            num_arg(c, 2),
            d,
            bool_arg(c, 3),
        ),
        4 => (
            o.default_horz,
            num_arg(c, 1),
            num_arg(c, 2),
            Rgba(d.0, d.1, d.2, num_arg(c, 4) as f32),
            bool_arg(c, 3),
        ),
        7 => {
            let f = |i| num_arg(c, i) as f32;
            (
                o.default_horz,
                num_arg(c, 1),
                num_arg(c, 2),
                Rgba(f(3), f(4), f(5), f(6)),
                bool_arg(c, 7),
            )
        }
        8 => {
            let h = horz_arg(&c.arg(1))
                .ok_or_else(|| LuaError::runtime("Expected TextDrawHorizontal"))?;
            let f = |i| num_arg(c, i) as f32;
            (
                h,
                num_arg(c, 2),
                num_arg(c, 3),
                Rgba(f(4), f(5), f(6), f(7)),
                bool_arg(c, 8),
            )
        }
        _ => return Err(LuaError::runtime("No implementation found")),
    })
}

/// Register `TextDrawObject` and `TextDrawHorizontal`.
pub fn register(thread: &mut Thread) {
    expose_enum(thread, TEXT_DRAW_HORIZONTAL, &TextDrawHorizontal::NAMES);
    let b = expose_class(thread, TEXT_DRAW_OBJECT, None);
    b.ctor(|c| {
        let o = match c.nargs() {
            0 => TextDrawObject::default(),
            4 => TextDrawObject::new(
                int_arg(c, 0),
                int_arg(c, 1),
                int_arg(c, 2),
                bool_arg(c, 3),
                true,
                true,
                true,
                true,
                false,
            ),
            _ => TextDrawObject::new(
                int_arg(c, 0),
                int_arg(c, 1),
                int_arg(c, 2),
                bool_arg(c, 3),
                bool_arg(c, 4),
                bool_arg(c, 5),
                bool_arg(c, 6),
                bool_arg(c, 7),
                bool_arg(c, 8),
            ),
        };
        Ok(c.push(Value::Object(Rc::new(LuaTextDrawObject(Rc::new(
            RefCell::new(o),
        ))))))
    });
    macro_rules! setter {
        ($name:literal, $arg:ident, $m:ident) => {
            b.method($name, |c| {
                let v = $arg(c, 1);
                this(c)?.borrow_mut().$m(v);
                Ok(0)
            });
        };
    }
    macro_rules! getter {
        ($name:literal, $m:ident) => {
            b.method($name, |c| {
                let v = this(c)?.borrow().$m();
                Ok(c.push(v))
            });
        };
    }
    setter!("setEnabled", bool_arg, set_enabled);
    getter!("getEnabled", get_enabled);
    setter!("setVisibleRadius", int_arg, set_visible_radius);
    b.method("getVisibleRadius", |c| {
        let v = this(c)?.borrow().get_visible_radius();
        Ok(c.push(v as f64))
    });
    setter!("setDrawBackground", bool_arg, set_draw_background);
    setter!("setAllowImages", bool_arg, set_allow_images);
    setter!("setAllowChatIcons", bool_arg, set_allow_chat_icons);
    setter!("setAllowColors", bool_arg, set_allow_colors);
    setter!("setAllowFonts", bool_arg, set_allow_fonts);
    setter!("setAllowBBcode", bool_arg, set_allow_bbcode);
    setter!("setAllowAnyImage", bool_arg, set_allow_any_image);
    setter!("setAllowLineBreaks", bool_arg, set_allow_line_breaks);
    setter!(
        "setEqualizeLineHeights",
        bool_arg,
        set_equalize_line_heights
    );
    b.method("setSettings", |c| {
        let a = |i| bool_arg(c, i);
        this(c)?
            .borrow_mut()
            .set_settings(a(1), a(2), a(3), a(4), a(5), a(6));
        Ok(0)
    });
    b.method("setCustomTag", |c| {
        let s = str_arg(c, 1).unwrap_or_else(|| Rc::from(""));
        this(c)?.borrow_mut().set_custom_tag(&s);
        Ok(0)
    });
    b.method("getCustomTag", |c| {
        let s = this(c)?.borrow().get_custom_tag().to_string();
        Ok(c.push(Value::str(&s)))
    });
    setter!("setMaxCharsPerLine", int_arg, set_max_chars_per_line);
    setter!(
        "setCustomImageMaxDimensions",
        int_arg,
        set_custom_image_max_dimensions
    );
    b.method("setOutlineColors", |c| {
        let col = colors_arg(c, 1);
        this(c)?.borrow_mut().set_outline_colors(col);
        Ok(0)
    });
    b.method("setDefaultColors", |c| {
        let col = colors_arg(c, 1);
        this(c)?.borrow_mut().set_default_colors(col);
        Ok(0)
    });
    b.method("setHorizontalAlign", |c| {
        let o = this(c)?;
        match c.arg(1) {
            Value::Str(s) => o.borrow_mut().set_horizontal_align_str(&s),
            v => match horz_arg(&v) {
                Some(h) => o.borrow_mut().set_horizontal_align(h),
                None => {
                    return Err(LuaError::Runtime {
                        message: String::new(),
                        class: "java.lang.NullPointerException",
                    })
                }
            },
        }
        Ok(0)
    });
    b.method("getHorizontalAlign", |c| {
        let h = this(c)?.borrow().get_horizontal_align();
        Ok(c.push(enum_value(TEXT_DRAW_HORIZONTAL, h.name())))
    });
    b.method("getOriginal", |c| {
        let s = this(c)?.borrow().get_original().to_string();
        Ok(c.push(Value::str(&s)))
    });
    b.method("getUnformatted", |c| {
        let s = this(c)?.borrow().get_unformatted();
        Ok(c.push(Value::str(&s)))
    });
    b.method("getWidth", |c| {
        let v = this(c)?.borrow().get_width();
        Ok(c.push(v as f64))
    });
    b.method("getHeight", |c| {
        let v = this(c)?.borrow().get_height();
        Ok(c.push(v as f64))
    });
    b.method("getDefaultFontEnum", |c| {
        let f = this(c)?.borrow().get_default_font_enum();
        Ok(c.push(enum_value(crate::bind_ui::UIFONT, f.name())))
    });
    getter!("isNullOrZeroLength", is_null_or_zero_length);
    b.method("getInternalClock", |c| {
        let v = this(c)?.borrow().get_internal_clock();
        Ok(c.push(v as f64))
    });
    b.method("setInternalTickClock", |c| {
        let v = num_arg(c, 1) as f32;
        this(c)?.borrow_mut().set_internal_tick_clock(v);
        Ok(0)
    });
    b.method("updateInternalTickClock", |c| {
        let o = this(c)?;
        let v = if c.nargs() > 1 {
            o.borrow_mut()
                .update_internal_tick_clock_by(num_arg(c, 1) as f32)
        } else {
            o.borrow_mut().update_internal_tick_clock()
        };
        Ok(c.push(v as f64))
    });
    b.method("setScrambleVal", |c| {
        let v = num_arg(c, 1) as f32;
        this(c)?.borrow_mut().set_scramble_val(v);
        Ok(0)
    });
    b.method("getScrambleVal", |c| {
        let v = this(c)?.borrow().get_scramble_val();
        Ok(c.push(v as f64))
    });
    setter!("setHearRange", int_arg, set_hear_range);
    b.method("getHearRange", |c| {
        let v = this(c)?.borrow().get_hear_range();
        Ok(c.push(v as f64))
    });
    b.method("Clear", |c| {
        this(c)?.borrow_mut().clear();
        Ok(0)
    });
    b.method("setDefaultFont", |c| {
        let f = font_arg(&c.arg(1)).ok_or_else(|| LuaError::runtime("Expected UIFont"))?;
        this(c)?.borrow_mut().set_default_font(f);
        Ok(0)
    });
    b.method("ReadString", |c| {
        let o = this(c)?;
        let s = |v: Value| match v {
            Value::Nil => String::new(),
            v => pz_kahlua::exec::java_value_of(&v),
        };
        let mut o = o.borrow_mut();
        match c.nargs() - 1 {
            1 => o.read_string(&s(c.arg(1))),
            2 => {
                let f = o.default_font_enum;
                o.read_string_with(f, &s(c.arg(1)), int_arg(c, 2));
            }
            _ => {
                let f = font_arg(&c.arg(1)).ok_or_else(|| LuaError::runtime("Expected UIFont"))?;
                o.read_string_with(f, &s(c.arg(2)), int_arg(c, 3));
            }
        }
        Ok(0)
    });
    b.method("calculateDimensions", |c| {
        this(c)?.borrow_mut().calculate_dimensions();
        Ok(0)
    });
    b.method("Draw", |c| {
        let o = this(c)?;
        let (h, x, y, col, out) = draw_args(c, &o.borrow())?;
        o.borrow_mut().draw_raw(h, x, y, col, out);
        Ok(0)
    });
    b.method("DrawRaw", |c| {
        let o = this(c)?;
        let (h, x, y, col, out) = draw_args(c, &o.borrow())?;
        o.borrow_mut().draw_raw(h, x, y, col, out);
        Ok(0)
    });
    b.method("AddBatchedDraw", |c| {
        let o = this(c)?;
        let (h, x, y, col, out) = draw_args(c, &o.borrow())?;
        add_batched_draw(&o, h, x, y, col, out);
        Ok(0)
    });
    b.static_fn("RenderBatch", |c| {
        render_batch(int_arg(c, 0));
        Ok(0)
    });
    b.static_fn("NoRender", |c| {
        no_render(int_arg(c, 0));
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::super::host::{install, HudHost};
    use super::super::test_util::*;
    use super::*;
    use pz_ui::tooltip::java_float_to_string as jf;

    /// The oracle's monospace fonts: 8 px a character; Large 24, Medium
    /// 20, others 16 px lines.
    struct Mono;
    impl pz_ui::TextRenderer for Mono {
        fn measure_x(&self, _: Option<UIFont>, s: &str) -> i32 {
            8 * s.encode_utf16().count() as i32
        }
        fn measure_y(&self, f: Option<UIFont>, s: &str) -> i32 {
            (1 + s.matches('\n').count() as i32) * self.font_height(f)
        }
        fn font_height(&self, f: Option<UIFont>) -> i32 {
            match f {
                Some(UIFont::Large) => 24,
                Some(UIFont::Medium) => 20,
                _ => 16,
            }
        }
        #[allow(clippy::too_many_arguments)]
        fn draw(
            &self,
            out: &mut pz_ui::SpriteRenderer,
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

    fn flush(got: &mut Vec<String>, label: &str) {
        got.push(format!("case {label}"));
        let v: Vec<String> = with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    UiDrawCmd::Quad {
                        texture,
                        x,
                        y,
                        w,
                        h,
                        color,
                        ..
                    } => Some(format!(
                        "draw {} {} {} {} {} {} {} {} {}",
                        texture
                            .as_ref()
                            .map_or("-".to_string(), |t| t.name.to_string()),
                        *x as i32,
                        *y as i32,
                        *w as i32,
                        *h as i32,
                        jf(color.0),
                        jf(color.1),
                        jf(color.2),
                        jf(color.3)
                    )),
                    UiDrawCmd::Text {
                        font,
                        x,
                        y,
                        text,
                        color,
                        ..
                    } => Some(format!(
                        "text {} left {} {} [{}] {} {} {} {}",
                        font.map_or("?", |f| f.name()),
                        jf(*x),
                        jf(*y),
                        text,
                        jf(color.0),
                        jf(color.1),
                        jf(color.2),
                        jf(color.3)
                    )),
                    _ => None,
                })
                .collect()
        });
        got.extend(v);
        with_ui(|m| m.sprite.clear());
    }

    fn st(t: &TextDrawObject, l: &str) -> String {
        format!(
            "{l} w {} h {} clock {} [{}] [{}]",
            t.get_width(),
            t.get_height(),
            jf(t.get_internal_clock()),
            t.get_unformatted().replace('\n', "|"),
            t.get_original()
        )
    }

    /// Parity with the original `TextDrawObject` (oracle `textdraw.txt`).
    #[test]
    fn matches_text_draw_oracle() {
        let Some(text) = oracle("textdraw.txt") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        install_textures_only(&oracle_textures_ex(&text));
        install(HudHost::default());
        let _t = thread();
        pz_ui::UIManager::set_text_renderer(Some(Rc::new(Mono)));
        pz_core::random::set_seed(7);
        crate::bind_core::shared_core()
            .borrow_mut()
            .set_screen_size(1280, 720);
        clear_chat_backdrop();
        with_ui(|m| m.sprite.clear());
        let mut got = Vec::new();
        let texts = [
            "Hello world",
            "[col=255,0,0]Red text[/] normal [fnt=Large]big[/] [img=music] [br/]second line [img=Axe] [img=Crate,10,20,30] end",
            "[col=300,0,0]x [fnt=Bogus]y [xyz=1]z [img= ] [col=1,2]w [BR/]q",
            "[col=1,2,3][fnt=Large]nested[/] after [/]closed",
            "[img=media/ui/CarKey.png] key [img=Anything] any [img=Missing,1,2,3] gone",
            "  leading  and  trailing  ",
            "[cdt=2.5]Timer [cdt=abc]bad",
            "",
            "a\nb tab\there",
        ];
        let mut t = TextDrawObject::new(255, 255, 255, true, true, true, true, true, false);
        for (k, s) in texts.iter().enumerate() {
            t.read_string_with(UIFont::Medium, s, -1);
            got.push(st(&t, &format!("read {k}")));
            t.draw(400.0, 300.0, false, None);
            flush(&mut got, &format!("draw {k}"));
        }
        t.set_allow_any_image(true);
        t.read_string(texts[4]);
        got.push(st(&t, "anyimage"));
        t.draw_raw(
            TextDrawHorizontal::Left,
            100.0,
            50.0,
            Rgba(0.5, 0.6, 0.7, 0.8),
            true,
        );
        flush(&mut got, "left outlined");
        t.set_allow_bbcode(false);
        t.read_string(texts[1]);
        got.push(st(&t, "no bbcode"));
        t.set_allow_bbcode(true);
        t.read_string_with(UIFont::Small, "Supercalifragilistic word here and more", 8);
        got.push(st(&t, "wrapped"));
        t.set_horizontal_align_str("right");
        t.set_outline_colors_i(10, 20, 30, 255);
        t.draw(600.0, 100.0, true, Some(0.5));
        flush(&mut got, "right outlined");
        t.set_max_chars_per_line(12);
        got.push(st(&t, "wrapped 12"));
        t.set_equalize_line_heights(true);
        t.read_string_with(UIFont::Medium, "small [fnt=Large]LARGE[/] [br/]next", -1);
        got.push(st(&t, "equalized"));
        t.set_horizontal_align_str("center");
        t.set_draw_background(true);
        t.draw(300.0, 200.0, false, None);
        flush(&mut got, "no backdrop");
        ensure_chat_backdrop();
        t.draw_raw(
            t.get_horizontal_align(),
            300.0,
            200.0,
            Rgba(1.0, 1.0, 1.0, 0.5),
            false,
        );
        flush(&mut got, "backdrop");
        t.draw(-500.0, 10.0, false, None);
        flush(&mut got, "culled left");
        t.draw(1300.0, 10.0, false, None);
        flush(&mut got, "culled right");
        t.draw(640.0, 739.0, false, None);
        flush(&mut got, "edge bottom");
        t.set_default_font(UIFont::Large);
        got.push(st(&t, "default font"));
        t.set_default_colors(Rgba(0.25, 0.5, 0.75, 1.0));
        t.draw(10.0, 10.0, false, None);
        flush(&mut got, "default colours");
        t.read_string_with(
            UIFont::Medium,
            "the quick brown fox jumps [col=0,255,0]over the[/] lazy dog",
            -1,
        );
        t.set_scramble_val(0.5);
        got.push(st(&t, "scrambled"));
        t.draw(400.0, 400.0, false, None);
        flush(&mut got, "scrambled");
        t.set_scramble_val(0.9);
        got.push(st(&t, "scrambled more"));
        t.read_string_with(UIFont::Medium, "[cdt=1]x", -1);
        let a = t.update_internal_tick_clock_by(25.0);
        let b = t.update_internal_tick_clock_by(50.0);
        let c = t.update_internal_tick_clock_by(5.0);
        got.push(format!("tick {} {} {}", jf(a), jf(b), jf(c)));
        let u = Rc::new(RefCell::new(TextDrawObject::default()));
        u.borrow_mut().read_string("batched");
        let (h, col) = (u.borrow().default_horz, u.borrow().default_color);
        add_batched_draw(&u, h, 50.0, 60.0, col, false);
        add_batched_draw(&u, h, 70.0, 80.0, col, true);
        render_batch(1);
        flush(&mut got, "batch 1");
        render_batch(0);
        flush(&mut got, "batch 0");
        render_batch(0);
        flush(&mut got, "batch again");
        add_batched_draw(&u, h, 5.0, 6.0, col, false);
        no_render(0);
        render_batch(0);
        flush(&mut got, "no render");
        t.clear();
        got.push(st(&t, "cleared"));
        let want: Vec<String> = text
            .lines()
            .filter(|l| !l.starts_with("fake ") && !l.starts_with("tex "))
            .map(str::to_string)
            .collect();
        // Multi-line strings in the oracle split across lines; compare the
        // joined text.
        assert_eq!(got.join("\n"), want.join("\n"));
    }

    /// `ISMoveableInfoWindow`'s use from Lua.
    #[test]
    fn lua_methods() {
        install_textures(&[]);
        install(HudHost::default());
        let mut t = thread();
        pz_ui::UIManager::set_text_renderer(Some(Rc::new(Mono)));
        run(
            &mut t,
            "o = TextDrawObject.new(205, 200, 177, true, true, true, true, true, true)
             o:setHorizontalAlign('center'); o:ReadString(UIFont.Medium, 'Title', -1)
             w = o:getWidth(); h = o:getHeight(); al = o:getHorizontalAlign()
             o:setHorizontalAlign(TextDrawHorizontal.Right); al2 = o:getHorizontalAlign()
             o:Clear(); empty = o:isNullOrZeroLength(); o:Draw(10, 20)",
        );
        assert_eq!(eval(&mut t, "w").as_num(), Some(40.0));
        assert_eq!(eval(&mut t, "h").as_num(), Some(20.0));
        assert!(eval(&mut t, "al == TextDrawHorizontal.Center").truthy());
        assert!(eval(&mut t, "al2 == TextDrawHorizontal.Right").truthy());
        assert!(eval(&mut t, "empty").truthy());
    }
}
