//! `TileDepthTexture` (one tile's depth map) and `TilesetDepthTexture` (a
//! `DEPTH_<tileset>.png` sheet cut into tiles).

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::image::ImageData;
use crate::{flags, Texture, TextureManager};

use super::clamp::{clamp_pixel_to_upper_floor, CLAMP_ROWS};

/// Columns of every tileset sheet (`TileDepthTextures.createTileset`).
pub const TILESET_COLUMNS: i32 = 8;
/// Width of a 1x tile; 2x tiles are twice as large.
pub const TILE_WIDTH_1X: i32 = 64;
/// Height of a 1x tile.
pub const TILE_HEIGHT_1X: i32 = 128;

/// Filters of a depth texture: `updateGPUTexture` sets `GL_NEAREST` for
/// both. The wrap mode stays `GL_REPEAT` (the `TextureID` is created with
/// flags 0, so no `CLAMP_TO_EDGE`).
pub const DEPTH_TEXTURE_FLAGS: i32 = flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST;

/// `TileDepthTexture`: the depth of every pixel of one tile, -1 where the
/// tile is transparent, else 0 (front) to 1 (back).
#[derive(Debug)]
pub struct TileDepthTexture {
    tileset: Rc<str>,
    index: i32,
    columns: i32,
    width: i32,
    height: i32,
    name: String,
    /// `pixels`; `None` for an empty tile, and after the GPU upload unless
    /// the tileset keeps its pixels (`isKeepPixels`).
    pixels: Option<Box<[f32]>>,
    empty: bool,
    /// The uploaded depth texture (`texture`), set for every tile that had
    /// depth when loaded.
    texture: Option<Texture>,
}

impl TileDepthTexture {
    /// `new TileDepthTexture(tileset, tileIndex)`: no pixels, empty.
    fn new(tileset: &TilesetDepthTexture, index: i32) -> Self {
        TileDepthTexture {
            tileset: tileset.name.clone(),
            index,
            columns: tileset.columns,
            width: tileset.tile_width(),
            height: tileset.tile_height(),
            name: format!("{}_{}", tileset.name, index),
            pixels: None,
            empty: true,
            texture: None,
        }
    }

    /// Name of the tileset this tile was cut from.
    pub fn tileset_name(&self) -> &str {
        &self.tileset
    }

    pub fn index(&self) -> i32 {
        self.index
    }

    pub fn column(&self) -> i32 {
        self.index % self.columns
    }

    pub fn row(&self) -> i32 {
        self.index / self.columns
    }

    /// `<tileset>_<index>`, the name of the sprite it belongs to.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// `isEmpty()`: no pixel of the tile has depth.
    pub fn is_empty(&self) -> bool {
        self.empty
    }

    /// `getPixels()`.
    pub fn pixels(&self) -> Option<&[f32]> {
        self.pixels.as_deref()
    }

    /// `getPixel(x, y)`: -1 without pixels.
    pub fn pixel(&self, x: i32, y: i32) -> f32 {
        match &self.pixels {
            None => -1.0,
            Some(p) => p[(x + y * self.width) as usize],
        }
    }

    /// The uploaded texture, `None` for a tile that was empty when loaded.
    /// [`super::TileDepthTextureManager::texture_of`] substitutes the empty
    /// texture as `getTexture()` does.
    pub fn uploaded_texture(&self) -> Option<&Texture> {
        self.texture.as_ref()
    }

    /// `load(pixels, bb, stride, left, top)`: reads the tile from an RGBA
    /// image: transparent pixels have no depth, the others take the blue
    /// channel. Non-empty tiles are uploaded at once (`updateGPUTexture`).
    fn load(&mut self, rgba: &[u8], stride: usize, left: i32, top: i32, keep_pixels: bool, tm: &mut TextureManager) {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut pixels = vec![0.0f32; w * h];
        self.empty = true;
        for y in 0..h {
            let row_start = (top as usize + y) * stride;
            for x in 0..w {
                let p = row_start + (left as usize + x) * 4;
                let (b, a) = (rgba[p + 2], rgba[p + 3]);
                pixels[x + y * w] = if a == 0 { -1.0 } else { b as f32 / 255.0 };
                if self.empty && a != 0 {
                    self.empty = false;
                }
            }
        }
        if self.empty {
            self.pixels = None;
        } else {
            self.pixels = Some(pixels.into_boxed_slice());
            self.update_gpu_texture(keep_pixels, tm);
        }
    }

    /// `updateGPUTexture()`.
    fn update_gpu_texture(&mut self, keep_pixels: bool, tm: &mut TextureManager) {
        let bytes = depth_bytes(self);
        self.empty = bytes.empty;
        let rgba = expand_red(&bytes.values, self.width, self.height);
        let t = tm.texture_from_pixels(&format!("DEPTH_{}", self.name), self.width, self.height, rgba, DEPTH_TEXTURE_FLAGS);
        self.texture = Some(t);
        if !keep_pixels {
            self.pixels = None;
        }
    }
}

/// What `updateGPUTexture` uploads: one byte per pixel (`GL_RED`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepthBytes {
    pub values: Vec<u8>,
    /// Recomputed `empty`: no pixel had depth.
    pub empty: bool,
}

/// The byte conversion of `updateGPUTexture`: pixels in the top 64 rows are
/// clamped behind the floor above (except in `roofs_` tilesets), depth is
/// stored as `floor(d * 255)` with 0 reserved for "no depth" (so real depth
/// 0 becomes 1).
pub fn depth_bytes(tile: &TileDepthTexture) -> DepthBytes {
    let (w, h) = (tile.width, tile.height);
    let clamp = !tile.tileset.starts_with("roofs_");
    let mut values = vec![0u8; (w * h) as usize];
    let mut empty = true;
    for y in 0..h {
        for x in 0..w {
            let mut pixel = tile.pixel(x, y);
            if pixel >= 0.0 && (y as usize) < CLAMP_ROWS && clamp {
                pixel = clamp_pixel_to_upper_floor(x as usize, y as usize, pixel);
            }
            let i = (x + y * w) as usize;
            if pixel < 0.0 {
                values[i] = 0;
            } else {
                pixel = if pixel > 1.0 { 1.0 } else { pixel };
                let mut v = ((pixel * 255.0) as f64).floor() as i32 & 0xFF;
                if v == 0 {
                    v = 1;
                }
                values[i] = v as u8;
            }
            if pixel >= 0.0 && empty {
                empty = false;
            }
        }
    }
    DepthBytes { values, empty }
}

/// A `GL_RED` image as RGBA: the value in red, as sampling a single-channel
/// texture returns it (`r, 0, 0, 1`).
fn expand_red(values: &[u8], width: i32, height: i32) -> Vec<u8> {
    debug_assert_eq!(values.len(), (width * height) as usize);
    let mut rgba = Vec::with_capacity(values.len() * 4);
    for &v in values {
        rgba.extend_from_slice(&[v, 0, 0, 255]);
    }
    rgba
}

/// `TilesetDepthTexture`: the tiles of one `DEPTH_<name>.png`.
#[derive(Clone, Debug)]
pub struct TilesetDepthTexture {
    name: Rc<str>,
    columns: i32,
    rows: i32,
    is_2x: bool,
    /// Every tile slot. The original creates tiles on first use
    /// (`getOrCreateTile`); here every slot holds a tile from the start, an
    /// empty one until loaded, which no caller can tell apart.
    tiles: Vec<Rc<TileDepthTexture>>,
    /// `getAbsoluteFileName()`.
    path: PathBuf,
}

impl TilesetDepthTexture {
    /// `new TilesetDepthTexture(owner, name, columns, rows, b2x)`, with the
    /// owner's media directory.
    pub fn new(media_dir: &Path, name: &str, columns: i32, rows: i32, is_2x: bool) -> Self {
        let mut t = TilesetDepthTexture {
            name: Rc::from(name),
            columns,
            rows,
            is_2x,
            tiles: Vec::new(),
            path: media_dir.join("depthmaps").join(format!("DEPTH_{name}.png")),
        };
        t.tiles = (0..columns * rows).map(|i| Rc::new(TileDepthTexture::new(&t, i))).collect();
        t
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn columns(&self) -> i32 {
        self.columns
    }

    pub fn rows(&self) -> i32 {
        self.rows
    }

    pub fn is_2x(&self) -> bool {
        self.is_2x
    }

    pub fn tile_width(&self) -> i32 {
        TILE_WIDTH_1X * if self.is_2x { 2 } else { 1 }
    }

    pub fn tile_height(&self) -> i32 {
        TILE_HEIGHT_1X * if self.is_2x { 2 } else { 1 }
    }

    pub fn width(&self) -> i32 {
        self.columns * self.tile_width()
    }

    pub fn height(&self) -> i32 {
        self.rows * self.tile_height()
    }

    pub fn tile_count(&self) -> i32 {
        self.columns * self.rows
    }

    /// `getRelativeFileName()`.
    pub fn relative_file_name(&self) -> String {
        format!("media/depthmaps/DEPTH_{}.png", self.name)
    }

    /// `getAbsoluteFileName()`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `fileExists()`.
    pub fn file_exists(&self) -> bool {
        self.path.exists()
    }

    /// `getOrCreateTile(index)`: `None` outside the sheet.
    pub fn tile(&self, index: i32) -> Option<&Rc<TileDepthTexture>> {
        if index < 0 || index >= self.tile_count() {
            return None;
        }
        self.tiles.get(index as usize)
    }

    /// `getOrCreateTile(col, row)`.
    pub fn tile_at(&self, col: i32, row: i32) -> Option<&Rc<TileDepthTexture>> {
        self.tile(col + row * self.columns)
    }

    /// Every tile slot, in index order.
    pub fn tiles(&self) -> &[Rc<TileDepthTexture>] {
        &self.tiles
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.tiles.iter().all(|t| t.is_empty())
    }

    /// `load()`: decodes the sheet and loads every tile it covers; tiles
    /// outside the image stay empty. A file that cannot be decoded leaves
    /// the tileset as it is (the original logs it).
    pub fn load(&mut self, tm: &mut TextureManager, keep_pixels: bool) -> std::io::Result<()> {
        let img = ImageData::from_png(BufReader::new(File::open(&self.path)?), false)?;
        self.load_image(&img, tm, keep_pixels);
        Ok(())
    }

    /// `load()` from an image already decoded.
    pub fn load_image(&mut self, img: &ImageData, tm: &mut TextureManager, keep_pixels: bool) {
        let (tw, th) = (self.tile_width(), self.tile_height());
        let stride = 4 * img.width_hw as usize;
        for row in 0..self.rows {
            for col in 0..self.columns {
                if col >= img.width / tw || row >= img.height / th {
                    continue;
                }
                let i = (col + row * self.columns) as usize;
                let mut tile = TileDepthTexture::new(self, i as i32);
                tile.load(&img.data.pixels, stride, col * tw, row * th, keep_pixels, tm);
                self.tiles[i] = Rc::new(tile);
            }
        }
    }

    /// `reload()`: `load()` when the file exists.
    pub fn reload(&mut self, tm: &mut TextureManager, keep_pixels: bool) -> std::io::Result<()> {
        if self.file_exists() {
            self.load(tm, keep_pixels)?;
        }
        Ok(())
    }

    /// `mergeTileset(other)`: fills this tileset's empty tiles with the
    /// other's non-empty ones (sharing them), over the overlapping columns
    /// and rows.
    pub fn merge_tileset(&mut self, other: &TilesetDepthTexture) {
        let rows = self.rows.min(other.rows);
        let columns = self.columns.min(other.columns);
        for row in 0..rows {
            for col in 0..columns {
                let i_self = (col + row * self.columns) as usize;
                let i_other = (col + row * other.columns) as usize;
                if !self.tiles[i_self].is_empty() || other.tiles[i_other].is_empty() {
                    continue;
                }
                self.tiles[i_self] = other.tiles[i_other].clone();
            }
        }
    }
}
