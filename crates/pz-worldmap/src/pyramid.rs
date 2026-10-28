//! `zombie.worldMap.ImagePyramid` and `WorldMapImages`: a zip of 256x256
//! PNG tiles per zoom level (`<z>/tile<x>x<y>.png` plus `pyramid.txt`)
//! drawn under or instead of the vector features.
//!
//! The original decodes tiles on a file thread and uploads at most five
//! per frame. Here a requested tile is decoded when the queue is next
//! checked (the same at-most-five-per-frame budget), so a tile becomes
//! ready a frame after it is first asked for, as at best in the game.

use pz_classfile::Jar;
use pz_render::Texture;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub const TILE_SIZE: i32 = 256;
pub const GL_CLAMP_TO_EDGE: i32 = 33071;
pub const GL_REPEAT: i32 = 10497;
pub const GL_LINEAR: i32 = 9729;
pub const GL_NEAREST: i32 = 9728;

/// Creates and frees the GPU textures of decoded tiles. Installed by the
/// UI layer (pz-lua-api, over the shared `TextureManager`).
pub trait TileTextures {
    /// A texture named `name` of `width` x `height` from RGBA `pixels`
    /// laid out at the power-of-two size, with the pyramid's filters and
    /// wrapping.
    #[allow(clippy::too_many_arguments)]
    fn create(
        &mut self,
        name: &str,
        width: i32,
        height: i32,
        pixels: Vec<u8>,
        min_nearest: bool,
        mag_nearest: bool,
        clamp: bool,
    ) -> Option<Texture>;
    fn destroy(&mut self, _t: &Texture) {}
}

thread_local! {
    static MAX_REQUEST_NUMBER: Cell<Option<i32>> = const { Cell::new(None) };
    static MAKER: RefCell<Option<Box<dyn TileTextures>>> = const { RefCell::new(None) };
    /// `WorldMapImages.s_filenameToImages`.
    static IMAGES: RefCell<HashMap<String, Rc<WorldMapImages>>> = RefCell::new(HashMap::new());
}

/// Sample the native class-static request limit once. Actual Lua hosts
/// provide the current Core.debug before the first image-pyramid creation.
/// Later calls preserve that first value, including across map resets.
pub fn initialize_class(debug: bool) -> i32 {
    MAX_REQUEST_NUMBER.with(|limit| match limit.get() {
        Some(value) => value,
        None => {
            let value = if debug { 10_000 } else { i32::MAX };
            limit.set(Some(value));
            value
        }
    })
}

pub fn class_initialized() -> bool {
    MAX_REQUEST_NUMBER.with(|limit| limit.get().is_some())
}

/// Installs the tile texture maker.
pub fn install_tile_textures(m: Box<dyn TileTextures>) {
    MAKER.with(|x| *x.borrow_mut() = Some(m));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureState {
    Init,
    Loading,
    Ready,
    Cancelled,
}

/// `ImagePyramid.PyramidTexture`.
#[derive(Debug)]
pub struct PyramidTexture {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub key: String,
    request_number: i32,
    required_this_frame: bool,
    pub texture: Option<Texture>,
    pub state: TextureState,
}

impl PyramidTexture {
    pub fn is_ready(&self) -> bool {
        self.state == TextureState::Ready && self.texture.is_some()
    }
}

/// `ImagePyramid`.
pub struct ImagePyramid {
    zip: Option<Jar>,
    textures: HashMap<String, PyramidTexture>,
    missing: HashSet<String>,
    queue: Vec<String>,
    request_number: i32,
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    image_width: i32,
    image_height: i32,
    pub resolution: f32,
    pub clamp_s: i32,
    pub clamp_t: i32,
    pub min_filter: i32,
    pub mag_filter: i32,
    pub min_z: i32,
    pub max_z: i32,
    max_textures: i32,
    textures_loaded_this_frame: i32,
    required: HashSet<String>,
}

/// `getKey(x, y, z)`.
fn key(x: i32, y: i32, z: i32) -> String {
    format!("{x}_{y}_{z}")
}

fn parse_int(s: &str, d: i32) -> i32 {
    s.trim().parse().unwrap_or(d)
}

impl ImagePyramid {
    /// `setZipFile(zipFile)`. Standalone callers use a nondebug class
    /// profile unless they explicitly initialize_class first. The actual
    /// Lua host samples its real Core.debug before reaching this method.
    pub fn from_zip(path: &str) -> Self {
        initialize_class(false);
        let mut p = ImagePyramid {
            zip: Jar::open(path).ok(),
            textures: HashMap::new(),
            missing: HashSet::new(),
            queue: Vec::new(),
            request_number: 0,
            min_x: 0,
            min_y: 0,
            max_x: 0,
            max_y: 0,
            image_width: -1,
            image_height: -1,
            resolution: 1.0,
            clamp_s: GL_CLAMP_TO_EDGE,
            clamp_t: GL_CLAMP_TO_EDGE,
            min_filter: GL_LINEAR,
            mag_filter: GL_NEAREST,
            min_z: i32::MAX,
            max_z: i32::MIN,
            max_textures: 100,
            textures_loaded_this_frame: 0,
            required: HashSet::new(),
        };
        p.read_info_file();
        if p.image_width == -1 {
            p.image_width = p.max_x - p.min_x;
            p.image_height = p.max_y - p.min_y;
        }
        p.resolution = (p.max_x - p.min_x) as f32 / p.image_width as f32;
        if let Some(zip) = &p.zip {
            // The top-level directories are the zoom levels.
            let mut zs = HashSet::new();
            for n in zip.names() {
                if let Some((dir, _)) = n.split_once('/') {
                    if !dir.is_empty() {
                        zs.insert(dir.to_string());
                    }
                }
            }
            for d in zs {
                let z = parse_int(&d, -1);
                p.min_z = p.min_z.min(z);
                p.max_z = p.max_z.max(z);
            }
        }
        p
    }

    /// `readInfoFile()`: `pyramid.txt`.
    fn read_info_file(&mut self) {
        let Some(text) = self.zip.as_ref().and_then(|z| z.read("pyramid.txt").ok()) else {
            return;
        };
        let text = String::from_utf8_lossy(&text).into_owned();
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("bounds=") {
                let ss: Vec<&str> = v.split(' ').collect();
                if ss.len() == 4 {
                    self.min_x = parse_int(ss[0], -1);
                    self.min_y = parse_int(ss[1], -1);
                    self.max_x = parse_int(ss[2], -1);
                    self.max_y = parse_int(ss[3], -1);
                }
            } else if let Some(v) = line.strip_prefix("clampS=") {
                match v.trim().to_ascii_lowercase().as_str() {
                    "clamp_to_edge" => self.clamp_s = GL_CLAMP_TO_EDGE,
                    "repeat" => self.clamp_s = GL_REPEAT,
                    _ => {}
                }
            } else if let Some(v) = line.strip_prefix("clampT=") {
                match v.trim().to_ascii_lowercase().as_str() {
                    "clamp_to_edge" => self.clamp_t = GL_CLAMP_TO_EDGE,
                    "repeat" => self.clamp_t = GL_REPEAT,
                    _ => {}
                }
            } else if let Some(v) = line.strip_prefix("minFilter=") {
                match v.trim().to_ascii_lowercase().as_str() {
                    "linear" => self.min_filter = GL_LINEAR,
                    "nearest" => self.min_filter = GL_NEAREST,
                    _ => {}
                }
            } else if let Some(v) = line.strip_prefix("magFilter=") {
                match v.trim().to_ascii_lowercase().as_str() {
                    "linear" => self.mag_filter = GL_LINEAR,
                    "nearest" => self.mag_filter = GL_NEAREST,
                    _ => {}
                }
            } else if let Some(v) = line.strip_prefix("imageSize=") {
                let ss: Vec<&str> = v.split(' ').collect();
                if ss.len() == 2 {
                    self.image_width = parse_int(ss[0], -1);
                    self.image_height = parse_int(ss[1], -1);
                }
            } else if let Some(v) = line.strip_prefix("resolution=") {
                self.resolution = v.trim().parse().unwrap_or(1.0);
            }
        }
    }

    /// `isValidTile(x, y, z)`.
    pub fn is_valid_tile(&self, x: i32, y: i32, z: i32) -> bool {
        if z < self.min_z || z > self.max_z || x < 0 || y < 0 {
            return false;
        }
        let w = self.image_width / 1i32.wrapping_shl(z as u32);
        let h = self.image_height / 1i32.wrapping_shl(z as u32);
        x <= w / 256 && y <= h / 256
    }

    /// `calculateMetersPerTile(z)`.
    pub fn calculate_meters_per_tile(&self, z: i32) -> f64 {
        let full = (self.image_width as f64 / 16.0).ceil() * 16.0;
        let fudge = full / self.image_width as f64;
        let scaled = ((full / 1i32.wrapping_shl(z as u32) as f64) as i32) as f64;
        let resolution = fudge * self.max_x.wrapping_sub(self.min_x) as f64 / scaled;
        256.0 * resolution
    }

    fn bump(&mut self) -> i32 {
        let n = self.request_number;
        self.request_number = self.request_number.wrapping_add(1);
        n
    }

    /// Native reset happens after the current tile has received its old
    /// request number and has been published in the cache. Stable sorting
    /// preserves this container's order for ties; matching Java HashMap's
    /// values order on tied requests remains a separate container gap.
    fn reset_request_numbers_if_needed(&mut self) {
        let limit = MAX_REQUEST_NUMBER.with(|limit| limit.get().expect("class initialized"));
        if self.request_number < limit {
            return;
        }
        let mut keys: Vec<String> = self.textures.keys().cloned().collect();
        keys.sort_by_key(|key| self.textures[key].request_number);
        self.request_number = 1;
        for key in keys {
            let n = self.bump();
            self.textures.get_mut(&key).unwrap().request_number = n;
        }
    }

    /// `checkQueue()`: decodes queued tiles, at most five per frame.
    fn check_queue(&mut self) {
        while self.textures_loaded_this_frame < 5 && !self.queue.is_empty() {
            let k = self.queue.remove(0);
            let Some(t) = self.textures.get(&k) else {
                continue;
            };
            if t.state != TextureState::Loading {
                continue;
            }
            let (x, y, z) = (t.x, t.y, t.z);
            let png = self
                .zip
                .as_ref()
                .and_then(|zip| zip.read(&format!("{z}/tile{x}x{y}.png")).ok());
            let img = png.and_then(|b| pz_render::image::ImageData::from_png(&b[..], false).ok());
            let Some(img) = img else {
                let t = self.textures.get_mut(&k).unwrap();
                t.request_number = 0;
                t.state = TextureState::Init;
                self.missing.insert(k);
                continue;
            };
            let (min_n, mag_n, clamp) = (
                self.min_filter == GL_NEAREST,
                self.mag_filter == GL_NEAREST,
                self.clamp_s == GL_CLAMP_TO_EDGE,
            );
            let tex = MAKER.with(|m| {
                m.borrow_mut().as_mut().and_then(|m| {
                    m.create(
                        &format!("pyramid:{k}"),
                        img.width,
                        img.height,
                        img.data.pixels,
                        min_n,
                        mag_n,
                        clamp,
                    )
                })
            });
            let t = self.textures.get_mut(&k).unwrap();
            if let Some(old) = t.texture.take() {
                MAKER.with(|m| {
                    if let Some(m) = m.borrow_mut().as_mut() {
                        m.destroy(&old)
                    }
                });
            }
            t.texture = tex;
            t.state = TextureState::Ready;
            self.textures_loaded_this_frame += 1;
        }
    }

    fn start_loading(&mut self, k: &str) {
        let t = self.textures.get_mut(k).unwrap();
        if t.state == TextureState::Loading {
            return;
        }
        t.state = TextureState::Loading;
        self.queue.retain(|q| q != k);
        self.queue.push(k.to_string());
    }

    /// `getTexture(x, y, z)`: the tile, queued for loading when needed.
    pub fn get_texture(&mut self, x: i32, y: i32, z: i32) -> Option<&PyramidTexture> {
        self.check_queue();
        if !self.is_valid_tile(x, y, z) {
            return None;
        }
        let k = key(x, y, z);
        if self.textures.contains_key(&k) {
            let n = self.bump();
            let t = self.textures.get_mut(&k).unwrap();
            t.request_number = n;
            t.required_this_frame = true;
            self.reset_request_numbers_if_needed();
            let t = self.textures.get(&k).unwrap();
            if !(t.is_ready()
                || t.state != TextureState::Init && t.state != TextureState::Cancelled)
            {
                self.start_loading(&k);
            }
            return self.textures.get(&k);
        }
        if self.missing.contains(&k) {
            return None;
        }
        let exists = self
            .zip
            .as_ref()
            .is_some_and(|zip| zip.contains(&format!("{z}/tile{x}x{y}.png")));
        if !exists {
            self.missing.insert(k);
            return None;
        }
        if !self.check_texture_cache(x, y, z, &k) {
            return None;
        }
        self.start_loading(&k);
        self.textures.get(&k)
    }

    /// `getReadyTexture(x, y, z)`.
    pub fn get_ready_texture(&mut self, x: i32, y: i32, z: i32) -> Option<&PyramidTexture> {
        self.check_queue();
        let k = key(x, y, z);
        if !self.textures.get(&k).is_some_and(|t| t.is_ready()) {
            return None;
        }
        let n = self.bump();
        let t = self.textures.get_mut(&k).unwrap();
        t.request_number = n;
        t.required_this_frame = true;
        self.reset_request_numbers_if_needed();
        self.textures.get(&k)
    }

    /// `getLowerResTexture(x, y, z)`: the nearest ready ancestor tile.
    pub fn get_lower_res_texture(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<(i32, i32, i32, Texture)> {
        let mut div = 2;
        for z1 in z + 1..=self.max_z {
            let x1 = crate::renderer::fast_floor(x as f32 / div as f32);
            let y1 = crate::renderer::fast_floor(y as f32 / div as f32);
            if let Some(t) = self.get_ready_texture(x1, y1, z1) {
                return Some((t.x, t.y, t.z, t.texture.clone()?));
            }
            div *= 2;
        }
        div /= 2;
        let x1 = crate::renderer::fast_floor(x as f32 / div as f32);
        let y1 = crate::renderer::fast_floor(y as f32 / div as f32);
        let mz = self.max_z;
        if self.is_valid_tile(x1, y1, mz) {
            if let Some(t) = self.get_texture(x1, y1, mz) {
                if t.is_ready() {
                    return Some((t.x, t.y, t.z, t.texture.clone()?));
                }
            }
        }
        None
    }

    /// `checkTextureCache(x, y, z, key)`: a new slot, or the least
    /// recently requested tile not needed this frame.
    fn check_texture_cache(&mut self, x: i32, y: i32, z: i32, k: &str) -> bool {
        let n = self.request_number;
        let fresh = |k: &str| PyramidTexture {
            x,
            y,
            z,
            key: k.to_string(),
            request_number: n,
            required_this_frame: true,
            texture: None,
            state: TextureState::Init,
        };
        if (self.textures.len() as i32) < self.max_textures {
            self.bump();
            self.textures.insert(k.to_string(), fresh(k));
            self.reset_request_numbers_if_needed();
            return true;
        }
        let max_z = self.max_z;
        let oldest = self
            .textures
            .values()
            .filter(|t| t.z != max_z && !t.required_this_frame)
            .min_by_key(|t| t.request_number)
            .map(|t| t.key.clone());
        match oldest {
            None if z == max_z => {
                self.max_textures = self.max_textures.wrapping_add(1);
                self.bump();
                self.textures.insert(k.to_string(), fresh(k));
                self.reset_request_numbers_if_needed();
                true
            }
            None => false,
            Some(old) => {
                let mut t = self.textures.remove(&old).unwrap();
                self.queue.retain(|q| *q != old);
                t.x = x;
                t.y = y;
                t.z = z;
                t.key = k.to_string();
                t.request_number = self.bump();
                t.required_this_frame = true;
                t.state = TextureState::Init;
                self.textures.insert(k.to_string(), t);
                self.reset_request_numbers_if_needed();
                true
            }
        }
    }

    fn count_required(&mut self) {
        let required = self.required.len() as i32;
        let w = self.image_width / 1i32.wrapping_shl(self.max_z as u32);
        let h = self.image_height / 1i32.wrapping_shl(self.max_z as u32);
        let across = (w as f32 / 256.0).ceil() as i32;
        let down = (h as f32 / 256.0).ceil() as i32;
        let span = across.wrapping_mul(down);
        self.max_textures = self.max_textures.max(required.wrapping_add(span));
    }

    /// `calculateRequiredTiles(rasterizeXY, ptz)`.
    pub fn calculate_required_tiles(&mut self, xy: &[i32], ptz: i32) {
        self.required.clear();
        if ptz < self.max_z {
            for p in xy.chunks(2) {
                if p.len() < 2 || !self.is_valid_tile(p[0], p[1], ptz) {
                    continue;
                }
                let k = key(p[0], p[1], ptz);
                if let Some(t) = self.textures.get_mut(&k) {
                    t.required_this_frame = true;
                }
                self.required.insert(k);
            }
        }
        self.count_required();
    }

    /// `calculateTilesCoveringCellF(cellX, cellY, metersPerTile, out)`.
    pub fn tiles_covering_cell_f(&self, cell_x: i32, cell_y: i32, mpt: f32) -> Option<[i32; 4]> {
        let c = |v: i32, a: i32, b: i32| {
            if v < a {
                a
            } else if v > b {
                b
            } else {
                v
            }
        };
        let left = cell_x.wrapping_mul(256);
        let top = cell_y.wrapping_mul(256);
        let right = cell_x.wrapping_add(1).wrapping_mul(256);
        let bottom = cell_y.wrapping_add(1).wrapping_mul(256);
        let x1 = c(self.min_x, left, right);
        let y1 = c(self.min_y, top, bottom);
        let x2 = c(self.max_x, left, right);
        let y2 = c(self.max_y, top, bottom);
        if x1 == x2 || y1 == y2 {
            return None;
        }
        let f = crate::renderer::fast_floor;
        Some([
            f(left.wrapping_sub(self.min_x) as f32 / mpt),
            f(top.wrapping_sub(self.min_y) as f32 / mpt),
            f(right.wrapping_sub(self.min_x) as f32 / mpt),
            f(bottom.wrapping_sub(self.min_y) as f32 / mpt),
        ])
    }

    /// `calculateTilesCoveringCell(cellX, cellY, ptz, out)`.
    pub fn tiles_covering_cell(&self, cell_x: i32, cell_y: i32, ptz: i32) -> Option<[i32; 4]> {
        self.tiles_covering_cell_f(cell_x, cell_y, self.calculate_meters_per_tile(ptz) as f32)
    }

    /// `calculateRequiredTilesForCells(rasterizeXY, ptz)`.
    pub fn calculate_required_tiles_for_cells(&mut self, xy: &[i32], ptz: i32) {
        self.required.clear();
        if ptz < self.max_z {
            let mpt = self.calculate_meters_per_tile(ptz) as f32;
            for p in xy.chunks(2) {
                if p.len() < 2 {
                    continue;
                }
                let Some([x0, y0, x1, y1]) = self.tiles_covering_cell_f(p[0], p[1], mpt) else {
                    continue;
                };
                for ty in y0..=y1 {
                    for tx in x0..=x1 {
                        if !self.is_valid_tile(tx, ty, ptz) {
                            continue;
                        }
                        let k = key(tx, ty, ptz);
                        if let Some(t) = self.textures.get_mut(&k) {
                            t.required_this_frame = true;
                        }
                        self.required.insert(k);
                    }
                }
            }
        }
        self.count_required();
    }

    /// `startFrame()`.
    pub fn start_frame(&mut self) {
        self.textures_loaded_this_frame = 0;
        for t in self.textures.values_mut() {
            t.required_this_frame = false;
        }
    }

    /// `endFrame()`: loads not needed this frame are cancelled.
    pub fn end_frame(&mut self) {
        let max_z = self.max_z;
        let mut cancelled = Vec::new();
        for t in self.textures.values_mut() {
            if t.z == max_z || t.required_this_frame || t.state != TextureState::Loading {
                continue;
            }
            t.state = TextureState::Cancelled;
            cancelled.push(t.key.clone());
        }
        self.queue.retain(|k| !cancelled.contains(k));
    }
}

/// `WorldMapImages`: one pyramid zip, shared between maps.
pub struct WorldMapImages {
    pub absolute_path: Rc<str>,
    pub pyramid: RefCell<ImagePyramid>,
}

impl WorldMapImages {
    /// `getOrCreateWithFileName(absolutePath)`: `None` when the file does
    /// not exist.
    pub fn get_or_create(path: &str) -> Option<Rc<WorldMapImages>> {
        if !std::path::Path::new(path).exists() {
            return None;
        }
        if let Some(i) = IMAGES.with(|m| m.borrow().get(path).cloned()) {
            return Some(i);
        }
        let i = Rc::new(WorldMapImages {
            absolute_path: path.into(),
            pyramid: RefCell::new(ImagePyramid::from_zip(path)),
        });
        IMAGES.with(|m| m.borrow_mut().insert(path.to_string(), i.clone()));
        Some(i)
    }

    pub fn get_min_x(&self) -> i32 {
        self.pyramid.borrow().min_x
    }
    pub fn get_min_y(&self) -> i32 {
        self.pyramid.borrow().min_y
    }
    pub fn get_max_x(&self) -> i32 {
        self.pyramid.borrow().max_x
    }
    pub fn get_max_y(&self) -> i32 {
        self.pyramid.borrow().max_y
    }
    pub fn get_width_in_squares(&self) -> i32 {
        self.get_max_x() - self.get_min_x() + 1
    }
    pub fn get_height_in_squares(&self) -> i32 {
        self.get_max_y() - self.get_min_y() + 1
    }
    pub fn get_resolution(&self) -> f32 {
        self.pyramid.borrow().resolution
    }
    /// `getZoom(zoomF)`: the tile level for a map zoom.
    pub fn get_zoom(&self, zoom_f: f32) -> i32 {
        let ptz = if zoom_f as f64 >= 16.0 {
            0
        } else if zoom_f >= 15.0 {
            1
        } else if zoom_f >= 14.0 {
            2
        } else if zoom_f >= 13.0 {
            3
        } else {
            4
        };
        let p = self.pyramid.borrow();
        // PZMath.clamp(int, min, max) without the ordering check.
        if ptz < p.min_z {
            p.min_z
        } else if ptz > p.max_z {
            p.max_z
        } else {
            ptz
        }
    }
    /// `endsWith(File.separator + fileName)`.
    pub fn matches_file_name(&self, file_name: &str) -> bool {
        self.absolute_path.ends_with(&format!("/{file_name}"))
            || self.absolute_path.ends_with(&format!("\\{file_name}"))
    }
}

/// `WorldMapImages.startFrame()`.
pub fn start_frame() {
    IMAGES.with(|m| {
        for i in m.borrow().values() {
            i.pyramid.borrow_mut().start_frame();
        }
    });
}

/// `WorldMapImages.endFrame()`.
pub fn end_frame() {
    IMAGES.with(|m| {
        for i in m.borrow().values() {
            i.pyramid.borrow_mut().end_frame();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored (uncompressed) zip of `files`.
    fn zip(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let off = out.len() as u32;
            let mut h = vec![0x50, 0x4b, 3, 4, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes());
            out.extend(h);
            out.extend(name.as_bytes());
            out.extend(data);
            let mut c = vec![
                0x50, 0x4b, 1, 2, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ];
            c.extend((data.len() as u32).to_le_bytes());
            c.extend((data.len() as u32).to_le_bytes());
            c.extend((name.len() as u16).to_le_bytes());
            c.extend([0u8; 12]);
            c.extend(off.to_le_bytes());
            c.extend(name.as_bytes());
            central.extend(c);
        }
        let cd_off = out.len() as u32;
        out.extend(&central);
        let mut e = vec![0x50, 0x4b, 5, 6, 0, 0, 0, 0];
        e.extend((files.len() as u16).to_le_bytes());
        e.extend((files.len() as u16).to_le_bytes());
        e.extend((central.len() as u32).to_le_bytes());
        e.extend(cd_off.to_le_bytes());
        e.extend(0u16.to_le_bytes());
        out.extend(e);
        out
    }

    fn png_tile() -> Vec<u8> {
        let mut v = Vec::new();
        {
            let mut e = png::Encoder::new(&mut v, 256, 256);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .unwrap()
                .write_image_data(&vec![200u8; 256 * 256 * 4])
                .unwrap();
        }
        v
    }

    struct Fake(usize);
    impl TileTextures for Fake {
        fn create(
            &mut self,
            name: &str,
            w: i32,
            h: i32,
            _p: Vec<u8>,
            _a: bool,
            _b: bool,
            _c: bool,
        ) -> Option<Texture> {
            self.0 += 1;
            let mut t = Texture::empty(name);
            t.ready = true;
            t.width = w as u32;
            t.height = h as u32;
            Some(t)
        }
    }

    #[test]
    fn tile_validity_masks_the_signed_int_zoom_shift() {
        let mut p = ImagePyramid::from_zip("");
        p.image_width = 2048;
        p.image_height = 1024;
        p.min_z = -64;
        p.max_z = 64;
        for z in [-64, -32, 0, 32, 64] {
            assert!(p.is_valid_tile(8, 4, z), "masked zoom {z}");
            assert!(!p.is_valid_tile(9, 4, z));
            assert!(!p.is_valid_tile(8, 5, z));
        }
        for z in [-1, 31, 63] {
            assert!(p.is_valid_tile(0, 0, z));
            assert!(!p.is_valid_tile(1, 0, z));
        }
        assert!(!p.is_valid_tile(-1, 0, 32));
        assert!(!p.is_valid_tile(0, 0, 65), "zoom bounds precede the shift");
    }

    #[test]
    fn cell_covering_wraps_native_bounds_and_subtraction_before_float_conversion() {
        let mut p = ImagePyramid::from_zip("");
        p.min_x = 10;
        p.min_y = 20;
        p.max_x = 200;
        p.max_y = 200;
        assert_eq!(
            p.tiles_covering_cell_f(0, 0, 1.0),
            Some([-10, -20, 246, 236])
        );
        assert_eq!(
            p.tiles_covering_cell_f(i32::MAX / 256, 0, 1.0),
            None,
            "wrapped cell upper edge is accepted without ordering assertion"
        );
        assert_eq!(
            p.tiles_covering_cell_f(i32::MAX, i32::MAX, 1.0),
            None,
            "cell multiplication and addition wrap before clamping"
        );
        p.min_x = i32::MIN;
        p.min_y = i32::MIN;
        p.max_x = 100;
        p.max_y = 100;
        assert_eq!(
            p.tiles_covering_cell_f(0, 0, 1.0),
            Some([i32::MIN, i32::MIN, i32::MIN + 256, i32::MIN + 256]),
            "ISUB precedes float conversion"
        );
    }

    #[test]
    fn meters_per_tile_uses_signed_int_shift_before_double_conversion() {
        let mut p = ImagePyramid::from_zip("");
        p.image_width = i32::MAX;
        p.min_x = 0;
        p.max_x = i32::MAX;
        // Native ceil(width/16)*16 is 2^31. ISHL produces -2^31 at
        // shift31; the following d2i therefore yields -1, not +1.
        for z in [-1, 31, 63] {
            assert_eq!(p.calculate_meters_per_tile(z), -549_755_813_888.0);
        }
        assert_eq!(
            p.calculate_meters_per_tile(-32),
            p.calculate_meters_per_tile(0)
        );
        assert_eq!(
            p.calculate_meters_per_tile(32),
            p.calculate_meters_per_tile(0)
        );
        assert_eq!(
            p.calculate_meters_per_tile(33),
            p.calculate_meters_per_tile(1)
        );
        p.image_width = 16;
        p.min_x = i32::MAX;
        p.max_x = i32::MIN;
        assert_eq!(
            p.calculate_meters_per_tile(0),
            16.0,
            "native ISUB wraps the bound span"
        );
    }

    #[test]
    fn required_tile_capacity_masks_zoom_and_wraps_signed_int_product() {
        let mut p = ImagePyramid::from_zip("");
        p.image_width = 2048;
        p.image_height = 1024;
        p.min_z = 0;
        p.max_z = 32;
        p.max_textures = 0;
        p.calculate_required_tiles(&[0, 0], 31);
        assert_eq!(p.required.len(), 1);
        assert_eq!(
            p.max_textures, 33,
            "masked maxZ32 covers eight by four tiles"
        );
        p.image_width = i32::MAX;
        p.image_height = i32::MAX;
        p.max_z = 0;
        p.max_textures = 17;
        // Each f32 ceil is 2^23; Native IMUL wraps 2^46 to zero.
        p.calculate_required_tiles(&[], 0);
        assert_eq!(p.max_textures, 17);
    }

    fn cached_ready_tile(p: &mut ImagePyramid, x: i32, request_number: i32) {
        let key = key(x, 0, 0);
        p.textures.insert(
            key.clone(),
            PyramidTexture {
                x,
                y: 0,
                z: 0,
                key,
                request_number,
                required_this_frame: false,
                texture: Some(Texture::empty("cached")),
                state: TextureState::Ready,
            },
        );
    }

    fn cached_pyramid() -> ImagePyramid {
        let mut p = ImagePyramid::from_zip("");
        p.image_width = 2048;
        p.image_height = 1024;
        p.min_z = 0;
        p.max_z = 1;
        p
    }

    #[test]
    fn cached_request_reset_ranks_the_current_tile_after_its_assignment() {
        assert_eq!(initialize_class(true), 10_000);
        assert_eq!(
            initialize_class(false),
            10_000,
            "class limit is sampled once"
        );
        for ready_getter in [false, true] {
            let mut p = cached_pyramid();
            cached_ready_tile(&mut p, 0, 10);
            cached_ready_tile(&mut p, 1, 20);
            cached_ready_tile(&mut p, 2, 30);
            p.request_number = 9_999;
            let tile = if ready_getter {
                p.get_ready_texture(0, 0, 0).unwrap()
            } else {
                p.get_texture(0, 0, 0).unwrap()
            };
            assert!(tile.is_ready() && tile.required_this_frame);
            assert_eq!(
                tile.request_number, 3,
                "current tile participates in the reset"
            );
            assert_eq!(p.textures[&key(1, 0, 0)].request_number, 1);
            assert_eq!(p.textures[&key(2, 0, 0)].request_number, 2);
            assert_eq!(p.request_number, 4);
        }
    }

    #[test]
    fn new_and_reused_slots_are_published_before_request_number_reset() {
        assert_eq!(initialize_class(true), 10_000);
        for reuse in [false, true] {
            let mut p = cached_pyramid();
            cached_ready_tile(&mut p, 0, 10);
            cached_ready_tile(&mut p, 1, 20);
            p.max_textures = if reuse { 2 } else { 3 };
            p.request_number = 9_999;
            let new_key = key(2, 0, 0);
            assert!(p.check_texture_cache(2, 0, 0, &new_key));
            let count = if reuse { 2 } else { 3 };
            assert_eq!(p.textures.len(), count as usize);
            assert_eq!(p.textures[&new_key].request_number, count);
            assert!(p.textures[&new_key].required_this_frame);
            assert_eq!(p.request_number, count + 1);
            assert_eq!(p.textures.contains_key(&key(0, 0, 0)), !reuse);
        }
    }

    #[test]
    fn cached_request_increment_preserves_native_signed_wrap() {
        assert_eq!(initialize_class(false), i32::MAX);
        let mut p = cached_pyramid();
        cached_ready_tile(&mut p, 0, 10);
        p.request_number = i32::MAX;
        assert_eq!(p.get_texture(0, 0, 0).unwrap().request_number, i32::MAX);
        assert_eq!(p.request_number, i32::MIN);
        assert_eq!(
            p.get_ready_texture(0, 0, 0).unwrap().request_number,
            i32::MIN
        );
        assert_eq!(p.request_number, i32::MIN + 1);
    }

    #[test]
    fn info_levels_and_loading_budget() {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "pz-pyramid-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut files = vec![
            (
                "pyramid.txt",
                b"VERSION=1\nbounds=0 0 2048 1024\nimageSize=2048 1024\nmagFilter=linear\n"
                    .to_vec(),
            ),
            ("0/", Vec::new()),
            ("1/", Vec::new()),
        ];
        let tile = png_tile();
        let names: Vec<String> = (0..8).map(|x| format!("0/tile{x}x0.png")).collect();
        for n in &names {
            files.push((n.as_str(), tile.clone()));
        }
        let path = dir.join("pyramid.zip");
        std::fs::write(&path, zip(&files)).unwrap();
        install_tile_textures(Box::new(Fake(0)));
        let mut p = ImagePyramid::from_zip(path.to_str().unwrap());
        assert_eq!(
            (p.min_x, p.max_x, p.max_y, p.min_z, p.max_z),
            (0, 2048, 1024, 0, 1)
        );
        assert_eq!(p.mag_filter, GL_LINEAR);
        assert_eq!(p.calculate_meters_per_tile(1), 512.0);
        assert!(p.is_valid_tile(8, 4, 0) && !p.is_valid_tile(9, 0, 0));
        p.start_frame();
        for x in 0..8 {
            assert!(!p.get_texture(x, 0, 0).unwrap().is_ready());
        }
        // A tile not in the zip is missing.
        assert!(p.get_texture(0, 1, 0).is_none());
        // Queued tiles decode as the queue is next checked, at most five
        // per frame.
        let ready = (0..8)
            .filter(|&x| p.get_ready_texture(x, 0, 0).is_some())
            .count();
        assert_eq!(ready, 5);
        p.start_frame();
        let ready = (0..8)
            .filter(|&x| p.get_texture(x, 0, 0).is_some_and(|t| t.is_ready()))
            .count();
        assert_eq!(ready, 8);
        assert!(p.get_lower_res_texture(3, 0, 0).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
