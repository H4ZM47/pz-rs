//! `zombie.core.fonts.AngelCodeFont`: BMFont text-format `.fnt` loading,
//! measurement and glyph quads.
//!
//! The original indexes glyphs by Java `char`, so measurement works on
//! UTF-16 code units; the `&str` helpers convert first. Display-list
//! caching is never enabled in the original (`displayListCaching` is never
//! set), so it is not ported.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pz_render::{DrawList, Rgba, Texture, TextureLoader};

/// `AngelCodeFont.CharDef`.
#[derive(Clone, Debug, Default)]
pub struct CharDef {
    pub id: i32,
    pub x: i16,
    pub y: i16,
    pub width: i16,
    pub height: i16,
    pub xoffset: i16,
    pub yoffset: i16,
    pub xadvance: i16,
    pub page: i16,
    /// Second chars of kerning pairs, sorted (as signed shorts).
    pub kerning_second: Option<Vec<i16>>,
    pub kerning_amount: Vec<i16>,
    /// `CharDef.image`: the glyph's sub-texture, once its page is loaded.
    pub image: Option<Texture>,
}

impl CharDef {
    /// `CharDef.getKerning`: binary search of the sorted second chars.
    pub fn get_kerning(&self, other: i32) -> i32 {
        let Some(second) = &self.kerning_second else {
            return 0;
        };
        let (mut low, mut high) = (0i32, second.len() as i32 - 1);
        while low <= high {
            let mid = ((low + high) as u32 >> 1) as i32;
            let v = second[mid as usize] as i32;
            if v < other {
                low = mid + 1;
            } else if v > other {
                high = mid - 1;
            } else {
                return self.kerning_amount[mid as usize] as i32;
            }
        }
        0
    }

    /// `CharDef.init`: the glyph rectangle inside its page texture.
    fn init(&mut self, page: &Texture) {
        let mut t = page.clone();
        t.name = format!("{}_{}_{}", page.name, self.x, self.y).into();
        t.uv = [
            page.uv[0] + self.x as f32,
            page.uv[1] + self.y as f32,
            self.width as f32,
            self.height as f32,
        ];
        self.image = Some(t);
    }
}

#[derive(Debug)]
pub enum FontError {
    /// `FileNotFoundException` from opening the `.fnt`.
    NotFound(String),
    /// A malformed line (`NumberFormatException`, `NoSuchElementException`,
    /// or a kerning pair for a char that does not exist).
    Parse(String),
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FontError::NotFound(p) => write!(f, "{p} (No such file or directory)"),
            FontError::Parse(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for FontError {}

#[derive(Debug, Default)]
pub struct AngelCodeFont {
    /// Indexed by UTF-16 code unit, `max id + 1` long.
    pub chars: Vec<Option<CharDef>>,
    line_height: i32,
    font_image: Option<Texture>,
    pages: HashMap<i16, Texture>,
    fnt_file: Option<PathBuf>,
    sdf: bool,
}

/// Normalises a path the way the constructor does: no leading '/', '/'
/// separators.
fn normalise(path: &str) -> String {
    path.strip_prefix('/').unwrap_or(path).replace('\\', "/")
}

/// `java.util.StringTokenizer(line, " =")`.
type TokenIter<'a> = std::iter::Filter<std::str::Split<'a, [char; 2]>, fn(&&str) -> bool>;

struct Tokens<'a>(TokenIter<'a>);

impl<'a> Tokens<'a> {
    fn new(line: &'a str) -> Self {
        Tokens(line.split([' ', '=']).filter(|t| !t.is_empty()))
    }

    fn next(&mut self) -> Result<&'a str, FontError> {
        self.0
            .next()
            .ok_or_else(|| FontError::Parse("NoSuchElementException".into()))
    }

    fn short(&mut self) -> Result<i16, FontError> {
        let t = self.next()?;
        parse_java_int(t)
            .and_then(|v| i16::try_from(v).ok())
            .ok_or_else(|| FontError::Parse(format!("Value out of range. Value:\"{t}\" Radix:10")))
    }

    fn int(&mut self) -> Result<i32, FontError> {
        let t = self.next()?;
        parse_java_int(t).ok_or_else(|| FontError::Parse(format!("For input string: \"{t}\"")))
    }
}

/// `Integer.parseInt(s)`: optional sign, ASCII digits only.
fn parse_java_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `BufferedReader.readLine` splitting: '\n', '\r' or "\r\n".
fn read_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < b.len() {
        if b[i] == b'\n' || b[i] == b'\r' {
            out.push(&text[start..i]);
            if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < b.len() {
        out.push(&text[start..]);
    }
    out
}

impl AngelCodeFont {
    /// `new AngelCodeFont(fntFile, imgFile)`: `fnt_file` is a game-relative
    /// path resolved through the game file system (`resolve`, the original's
    /// `ZomboidFileSystem.getString`); `img_file` is handed to the texture
    /// loader as given.
    pub fn load(
        fnt_file: &str,
        img_file: Option<&str>,
        resolve: &dyn Fn(&str) -> Option<PathBuf>,
        loader: &mut dyn TextureLoader,
    ) -> Result<AngelCodeFont, FontError> {
        let mut font = AngelCodeFont::default();
        if let Some(img) = img_file.filter(|s| !s.chars().all(char::is_whitespace)) {
            font.font_image = loader.shared_texture(img);
        }
        let path = normalise(fnt_file);
        let file = resolve(&path).unwrap_or_else(|| PathBuf::from(&path));
        let bytes = std::fs::read(&file).map_err(|_| FontError::NotFound(path.clone()))?;
        font.fnt_file = Some(file);
        font.parse_fnt(&String::from_utf8_lossy(&bytes), Some(loader))?;
        Ok(font)
    }

    /// `new AngelCodeFont(fntFile, image)`: an absolute `.fnt` with an
    /// already loaded image (page textures are not loaded).
    pub fn from_file(fnt_file: &Path, image: Option<Texture>) -> Result<AngelCodeFont, FontError> {
        let bytes = std::fs::read(fnt_file)
            .map_err(|_| FontError::NotFound(fnt_file.display().to_string()))?;
        let mut font = AngelCodeFont {
            font_image: image,
            ..AngelCodeFont::default()
        };
        font.parse_fnt(&String::from_utf8_lossy(&bytes), None)?;
        Ok(font)
    }

    /// Parses `.fnt` text without loading any textures (for measurement).
    pub fn parse(text: &str) -> Result<AngelCodeFont, FontError> {
        let mut font = AngelCodeFont::default();
        font.parse_fnt(text, None)?;
        Ok(font)
    }

    fn parse_fnt(
        &mut self,
        text: &str,
        mut loader: Option<&mut dyn TextureLoader>,
    ) -> Result<(), FontError> {
        let lines = read_lines(text);
        let mut kerning: HashMap<i16, Vec<i16>> = HashMap::new();
        let mut char_defs: Vec<CharDef> = Vec::new();
        let mut max_char = 0;
        // The first two lines ("info", "common") are skipped unread.
        for line in lines.iter().skip(2) {
            if line.starts_with("page") {
                let mut t = Tokens::new(line);
                t.next()?;
                t.next()?;
                let id = t.short()?;
                t.next()?;
                let file = t.next()?.replace('"', "");
                let parent = self.fnt_file.as_deref().and_then(Path::parent);
                let parent = parent.map_or_else(|| "null".to_string(), |p| p.display().to_string());
                let file = format!("{parent}/{file}").replace('\\', "/");
                if let Some(loader) = loader.as_deref_mut() {
                    match loader.shared_texture(&file) {
                        Some(tex) => {
                            self.pages.insert(id, tex);
                        }
                        None => eprintln!("AngelCodeFont failed to load page {id} texture {file}"),
                    }
                }
            }
            if !line.starts_with("chars c") && line.starts_with("char") {
                if let Some(def) = self.parse_char(line)? {
                    max_char = max_char.max(def.id);
                    char_defs.push(def);
                }
            }
            if line.starts_with("kernings c") || !line.starts_with("kerning") {
                continue;
            }
            let mut t = Tokens::new(line);
            t.next()?;
            t.next()?;
            let first = t.short()?;
            t.next()?;
            let second = t.int()?;
            t.next()?;
            let offset = t.int()?;
            let v = kerning.entry(first).or_default();
            v.push(second as i16);
            v.push(offset as i16);
        }
        self.chars = vec![None; max_char as usize + 1];
        for def in char_defs {
            let id = def.id as usize;
            self.chars[id] = Some(def);
        }
        for (key, values) in kerning {
            let def = usize::try_from(key)
                .ok()
                .and_then(|k| self.chars.get_mut(k))
                .and_then(Option::as_mut)
                .ok_or_else(|| FontError::Parse(format!("kerning for missing char {key}")))?;
            let second: Vec<i16> = values.iter().step_by(2).copied().collect();
            let amount: Vec<i16> = values.iter().skip(1).step_by(2).copied().collect();
            let mut sorted = second.clone();
            sorted.sort();
            let mut new_amount = amount.clone();
            for (i, s) in sorted.iter().enumerate() {
                if let Some(j) = second.iter().position(|v| v == s) {
                    new_amount[i] = amount[j];
                }
            }
            def.kerning_second = Some(sorted);
            def.kerning_amount = new_amount;
        }
        Ok(())
    }

    fn parse_char(&mut self, line: &str) -> Result<Option<CharDef>, FontError> {
        let mut t = Tokens::new(line);
        t.next()?;
        t.next()?;
        let mut def = CharDef {
            id: t.int()?,
            ..CharDef::default()
        };
        if def.id < 0 {
            return Ok(None);
        }
        t.next()?;
        def.x = t.short()?;
        t.next()?;
        def.y = t.short()?;
        t.next()?;
        def.width = t.short()?;
        t.next()?;
        def.height = t.short()?;
        t.next()?;
        def.xoffset = t.short()?;
        t.next()?;
        def.yoffset = t.short()?;
        t.next()?;
        def.xadvance = t.short()?;
        t.next()?;
        def.page = t.short()?;
        let image = self.pages.get(&def.page).or(self.font_image.as_ref());
        if let Some(image) = image {
            def.init(image);
        }
        if def.id != 32 {
            self.line_height = self.line_height.max(def.height as i32 + def.yoffset as i32);
        }
        Ok(Some(def))
    }

    fn char_def(&self, id: u16) -> Option<&CharDef> {
        self.chars.get(id as usize).and_then(Option::as_ref)
    }

    /// `getLineHeight()`: the tallest `height + yoffset` of any non-space
    /// glyph (the file's `common lineHeight` is not used).
    pub fn line_height(&self) -> i32 {
        self.line_height
    }

    pub fn is_sdf(&self) -> bool {
        self.sdf
    }

    pub fn set_sdf(&mut self, b: bool) {
        self.sdf = b;
    }

    /// `isLoading()`: textures load synchronously here, so a font is never
    /// waiting on one.
    pub fn is_loading(&self) -> bool {
        false
    }

    /// `getWidth(text)`.
    pub fn width(&self, text: &str) -> i32 {
        let u: Vec<u16> = text.encode_utf16().collect();
        self.get_width(&u, 0, u.len() as i32 - 1, false)
    }

    /// `getWidth(text, start, end, xadvance)` over UTF-16 code units: the
    /// widest line; the last glyph counts its `width` unless `xadvance`.
    pub fn get_width(&self, text: &[u16], start: i32, end: i32, xadvance: bool) -> i32 {
        let num_chars = end - start + 1;
        let (mut max_width, mut width) = (0, 0);
        let mut last: Option<&CharDef> = None;
        for i in 0..num_chars.max(0) {
            let id = text[(start + i) as usize];
            if id == 0x0a {
                width = 0;
                continue;
            }
            let Some(def) = self.char_def(id) else {
                continue;
            };
            if let Some(l) = last {
                width += l.get_kerning(id as i32);
            }
            last = Some(def);
            width += if xadvance || i < num_chars - 1 {
                def.xadvance as i32
            } else {
                def.width as i32
            };
            max_width = max_width.max(width);
        }
        max_width
    }

    /// `getHeight(text)`.
    pub fn height(&self, text: &str) -> i32 {
        let u: Vec<u16> = text.encode_utf16().collect();
        self.get_height(&u, false, false)
    }

    /// `getHeight(text, returnActualHeight, returnOffset)`.
    pub fn get_height(&self, text: &[u16], actual: bool, offset: bool) -> i32 {
        let mut lines = 1;
        let mut max_height = 0;
        let mut min_offset = 1_000_000;
        for &id in text {
            if id == 0x0a {
                lines += 1;
                max_height = 0;
                continue;
            }
            if id == 0x20 {
                continue;
            }
            let Some(def) = self.char_def(id) else {
                continue;
            };
            max_height = max_height.max(def.height as i32 + def.yoffset as i32);
            min_offset = min_offset.min(def.yoffset as i32);
        }
        if actual {
            return max_height - min_offset;
        }
        if offset {
            return min_offset;
        }
        lines * self.line_height
    }

    /// `getYOffset(text)`: smallest `yoffset` on the first line. `None`
    /// where the original throws (a char past the last glyph id).
    pub fn get_y_offset(&self, text: &[u16]) -> Option<i32> {
        let stop = text.iter().position(|&c| c == 0x0a).unwrap_or(text.len());
        let mut min = 10_000;
        for &id in &text[..stop] {
            if let Some(def) = self.chars.get(id as usize)? {
                min = min.min(def.yoffset as i32);
            }
        }
        Some(min)
    }

    /// `drawString(x, y, text, r, g, b, a)`.
    pub fn draw_string(&self, dl: &mut DrawList, x: f32, y: f32, text: &str, color: Rgba) {
        self.draw_string_scaled(dl, x, y, 0.0, text, color);
    }

    /// `drawString(x, y, scale, text, r, g, b, a)`: `scale <= 0` draws at
    /// integer pixel positions, otherwise scaled at float positions.
    pub fn draw_string_scaled(
        &self,
        dl: &mut DrawList,
        x: f32,
        y: f32,
        scale: f32,
        text: &str,
        color: Rgba,
    ) {
        let u: Vec<u16> = text.encode_utf16().collect();
        self.render(dl, x as i32, y as i32, scale, &u, color);
    }

    /// `drawString(..., startIndex, endIndex)` over UTF-16 code units.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_range(
        &self,
        dl: &mut DrawList,
        x: f32,
        y: f32,
        scale: f32,
        text: &[u16],
        start: usize,
        end: usize,
        color: Rgba,
    ) {
        self.render(dl, x as i32, y as i32, scale, &text[start..=end], color);
    }

    /// `AngelCodeFont.render` + `CharDef.draw`.
    fn render(
        &self,
        dl: &mut DrawList,
        xoff: i32,
        yoff: i32,
        scale: f32,
        text: &[u16],
        color: Rgba,
    ) {
        let (mut x, mut y) = (0.0f32, 0.0f32);
        let mut last: Option<&CharDef> = None;
        for &c in text {
            let mut id = c as usize;
            if id == 10 {
                x = 0.0;
                y += self.line_height as f32;
                continue;
            }
            if id >= self.chars.len() {
                id = 63;
            }
            // The original indexes chars['?'] unchecked; a font with fewer
            // than 64 glyph slots would throw there instead of skipping.
            let Some(def) = self.chars.get(id).and_then(Option::as_ref) else {
                continue;
            };
            if let Some(l) = last {
                let k = l.get_kerning(id as i32) as f32;
                x += if scale > 0.0 { k * scale } else { k };
            }
            last = Some(def);
            let tex = def.image.as_ref();
            if scale > 0.0 {
                dl.quad(
                    tex,
                    x + def.xoffset as f32 * scale + xoff as f32,
                    y + def.yoffset as f32 * scale + yoff as f32,
                    def.width as f32 * scale,
                    def.height as f32 * scale,
                    color,
                );
                x += def.xadvance as f32 * scale;
            } else {
                // SpriteRenderer.renderi: integer position.
                let qx = (x + def.xoffset as f32 + xoff as f32) as i32;
                let qy = (y + def.yoffset as f32 + yoff as f32) as i32;
                dl.quad(
                    tex,
                    qx as f32,
                    qy as f32,
                    def.width as f32,
                    def.height as f32,
                    color,
                );
                x += def.xadvance as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FNT: &str = "info face=\"T\"\r\ncommon lineHeight=99\r\npage id=0 file=\"t_0.png\"\r\nchars count=4\r\n\
char id=32 x=0 y=0 width=0 height=0 xoffset=0 yoffset=20 xadvance=4 page=0 chnl=15\r\n\
char id=65 x=1 y=2 width=8 height=10 xoffset=1 yoffset=3 xadvance=9 page=0 chnl=15\r\n\
char id=66 x=10 y=2 width=7 height=11 xoffset=0 yoffset=2 xadvance=8 page=0 chnl=15\r\n\
char id=86 x=20 y=2 width=9 height=10 xoffset=0 yoffset=3 xadvance=9 page=0 chnl=15\r\n\
kernings count=3\r\nkerning first=65  second=86  amount=-2\r\nkerning first=65  second=66  amount=1\r\n\
kerning first=86 second=65 amount=-3\r\n";

    #[test]
    fn metrics() {
        let f = AngelCodeFont::parse(FNT).unwrap();
        assert_eq!(f.line_height(), 13);
        assert_eq!(f.width("A"), 8);
        // A(9) + kern(A,B)=1 + B.width(7)
        assert_eq!(f.width("AB"), 17);
        // A(9) - 2 + V(9)
        assert_eq!(f.width("AV"), 16);
        assert_eq!(f.width("AV\nA"), 16);
        assert_eq!(f.height("A\nB"), 26);
        let u: Vec<u16> = "AB".encode_utf16().collect();
        assert_eq!(f.get_height(&u, true, false), 11);
        assert_eq!(f.get_height(&u, false, true), 2);
        assert_eq!(f.get_y_offset(&u), Some(2));
        assert_eq!(
            f.chars[65].as_ref().unwrap().kerning_second,
            Some(vec![66, 86])
        );
    }

    #[test]
    fn draws_integer_quads() {
        let f = AngelCodeFont::parse(FNT).unwrap();
        let mut dl = DrawList::default();
        f.draw_string(&mut dl, 10.7, 5.2, "AV", Rgba(1.0, 1.0, 1.0, 1.0));
        let xs: Vec<f32> = dl
            .cmds
            .iter()
            .map(|c| match c {
                pz_render::DrawCmd::Quad { x, .. } => *x,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(xs, vec![11.0, 17.0]);
    }
}
