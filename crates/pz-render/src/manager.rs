//! `Texture.getSharedTexture` and the tables behind it: the shared texture
//! table, `nullTextures`, the texture packs mounted by
//! `GameWindow.LoadTexturePack` and the asset tables of `TextureAssetManager`
//! / `TextureIDAssetManager`. Loading is synchronous here; the original
//! queues file tasks but callers only ever see a ready or failed texture.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::rc::Rc;

use pz_fs::pack::{SubTexture, TexturePackDevice};
use pz_fs::GameFs;

use crate::flags;
use crate::image::{BooleanGrid, ImageData, Mask, MipLevel};
use crate::{Texture, TextureLoader};

/// `TextureID`: one decoded image, shared by every texture cut from it.
#[derive(Debug)]
pub struct TextureData {
    /// Asset path: a file path, or `@pack@/<pack>/<page>`.
    pub path: String,
    pub flags: i32,
    pub width: i32,
    pub height: i32,
    pub width_hw: i32,
    pub height_hw: i32,
    pub solid: bool,
    /// Decoded pixels; `None` once taken by the backend, or when loading
    /// failed.
    pub image: Option<ImageData>,
    pub mask: Option<Rc<BooleanGrid>>,
    pub ready: bool,
    /// Bumped when the pixels are respecified after creation
    /// ([`TextureManager::tex_image`]); a backend re-uploads on change.
    pub version: u32,
}

impl TextureData {
    /// GL_NEAREST minification (else linear, or trilinear with mip maps).
    pub fn min_nearest(&self) -> bool {
        self.flags & flags::FILTER_MIN_NEAREST != 0
    }

    pub fn mag_nearest(&self) -> bool {
        self.flags & flags::FILTER_MAG_NEAREST != 0
    }

    pub fn mipmaps(&self) -> bool {
        self.flags & flags::MIPMAPS != 0 && self.flags & flags::FBO == 0
    }

    pub fn clamp_to_edge(&self) -> bool {
        self.flags & flags::CLAMP_TO_EDGE != 0
    }

    /// The levels `TextureID.generateHwId` uploads: every premultiplied mip
    /// level for `MIPMAPS` textures, else the base image as decoded.
    pub fn upload_levels(&self) -> Vec<MipLevel> {
        match &self.image {
            None => Vec::new(),
            Some(img) if self.mipmaps() => img.mip_maps(),
            Some(img) => vec![img.data.clone()],
        }
    }
}

/// `GameWindow.TexturePack`.
#[derive(Debug)]
pub struct TexturePack {
    pub pack_name: String,
    pub file_name: String,
    pub mod_id: Option<String>,
    pub textures: HashMap<String, SubTexture>,
}

pub struct TextureManager {
    /// `TextureID.useCompression`: adds `COMPRESS` to loose textures.
    pub use_compression: bool,
    data: Vec<TextureData>,
    data_by_path: HashMap<String, u32>,
    /// `TextureAssetManager`'s table: asset path -> texture.
    assets: HashMap<String, Texture>,
    /// `s_sharedTextureTable`.
    shared: HashMap<String, Texture>,
    /// `nullTextures`: names known not to exist.
    null_textures: HashSet<String>,
    /// `BucketManager.Shared()`.
    bucket: HashMap<String, Texture>,
    /// `GameWindow.texturePacks`, in load order.
    pub packs: Vec<TexturePack>,
    /// `FileSystemImpl.texturepackDevices`.
    devices: HashMap<String, (TexturePackDevice, i32)>,
    /// `GameWindow.texturePackTextures`.
    pack_textures: HashMap<String, SubTexture>,
    white: Option<Texture>,
    error: Option<Texture>,
}

impl Default for TextureManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TextureManager {
    pub fn new() -> Self {
        TextureManager {
            use_compression: true,
            data: Vec::new(),
            data_by_path: HashMap::new(),
            assets: HashMap::new(),
            shared: HashMap::new(),
            null_textures: HashSet::new(),
            bucket: HashMap::new(),
            packs: Vec::new(),
            devices: HashMap::new(),
            pack_textures: HashMap::new(),
            white: None,
            error: None,
        }
    }

    /// Backing images, indexed by [`Texture::id`].
    pub fn data(&self, id: u32) -> Option<&TextureData> {
        self.data.get(id as usize)
    }

    pub fn data_mut(&mut self, id: u32) -> Option<&mut TextureData> {
        self.data.get_mut(id as usize)
    }

    pub fn data_len(&self) -> usize {
        self.data.len()
    }

    /// A [`TextureLoader`] over this manager and a file system.
    pub fn loader<'a>(&'a mut self, fs: &'a GameFs) -> SharedTextures<'a> {
        SharedTextures { manager: self, fs }
    }

    // ---- texture packs ----------------------------------------------------

    /// `GameWindow.LoadTexturePack(pack, flags, modID)` (mounting the
    /// device and collecting its sub-textures). Call
    /// [`TextureManager::set_texture_pack_lookup`] afterwards.
    pub fn load_texture_pack(&mut self, fs: &GameFs, pack: &str, flags: i32, mod_id: Option<&str>) {
        let file_name = fs.get_string(&format!("media/texturepacks/{pack}.pack"));
        let mut textures = HashMap::new();
        match TexturePackDevice::open(pack, &file_name, flags) {
            Ok(dev) => {
                textures = dev.sub_textures();
                self.devices.insert(pack.to_string(), (dev, flags));
            }
            Err(e) => eprintln!("texturepack: {pack}: {e}"),
        }
        self.packs.push(TexturePack { pack_name: pack.to_string(), file_name, mod_id: mod_id.map(str::to_string), textures });
    }

    /// `GameWindow.setTexturePackLookup()`: base-game packs first, then
    /// packs of loaded mods; for duplicate names the earlier-loaded pack in
    /// each group wins, and mod packs win over the game's.
    pub fn set_texture_pack_lookup(&mut self, mod_ids: &[String]) {
        self.pack_textures.clear();
        for p in self.packs.iter().rev().filter(|p| p.mod_id.is_none()) {
            self.pack_textures.extend(p.textures.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        for p in self.packs.iter().rev() {
            if p.mod_id.as_ref().is_some_and(|m| mod_ids.contains(m)) {
                self.pack_textures.extend(p.textures.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        self.on_texture_packs_changed();
    }

    /// `Texture.onTexturePacksChanged()`.
    pub fn on_texture_packs_changed(&mut self) {
        self.null_textures.clear();
        self.shared.clear();
    }

    /// `GameWindow.init`'s UI packs, then the lookup.
    pub fn load_ui_packs(&mut self, fs: &GameFs) {
        for p in ["UI", "UI2", "IconsMoveables", "RadioIcons", "ApComUI", "Mechanics", "WeatherFx"] {
            self.load_texture_pack(fs, p, flags::FILTER_MAG_NEAREST, None);
        }
        self.set_texture_pack_lookup(fs.mod_ids());
    }

    // ---- getSharedTexture -------------------------------------------------

    /// `Texture.getSharedTexture(name)`.
    pub fn shared_texture(&mut self, fs: &GameFs, name: &str) -> Option<Texture> {
        let f = if self.use_compression { flags::COMPRESS } else { 0 };
        self.shared_texture_flags(fs, name, f)
    }

    /// `Texture.getSharedTexture(name, flags)`.
    pub fn shared_texture_flags(&mut self, fs: &GameFs, name: &str, flags: i32) -> Option<Texture> {
        if self.null_textures.contains(name) {
            return None;
        }
        if let Some(t) = self.shared.get(name) {
            return Some(t.clone());
        }
        let mut sub = name.replace('\\', "/");
        if !sub.ends_with(".txt") && !sub.contains("/mods/") {
            if sub.ends_with(".png") {
                sub.truncate(sub.rfind('.').unwrap());
            }
            let sub = sub[sub.rfind('/').map_or(0, |i| i + 1)..].to_string();
            // `TexturePackPage.getTexture`: the legacy page tables are empty
            // in B42, so only its ".png" recursion remains.
            if sub.contains(".png") {
                if let Some(t) = self.shared_texture(fs, &sub) {
                    self.shared.insert(name.to_string(), t.clone());
                    return Some(t);
                }
            }
            if let Some(st) = self.pack_textures.get(&sub).cloned() {
                let t = self.load_sub_texture(&st);
                self.shared.insert(name.to_string(), t.clone());
                return Some(t);
            }
        }
        if let Some(st) = self.pack_textures.get(name).cloned() {
            let t = self.load_sub_texture(&st);
            self.shared.insert(name.to_string(), t.clone());
            return Some(t);
        }
        if let Some(t) = self.bucket.get(name).cloned() {
            self.shared.insert(name.to_string(), t.clone());
            return Some(t);
        }
        if !name.contains('.') {
            self.null_textures.insert(name.to_string());
            return None;
        }
        let known = fs.lookup(name).map(str::to_string);
        let path = known.clone().unwrap_or_else(|| name.to_string());
        if !fs.validate_prefix(&path) {
            eprintln!("Failed to load shared texture: {name}: invalid prefix");
            return None;
        }
        if known.is_none() && fs.resolve(&path).is_none() {
            self.null_textures.insert(name.to_string());
            eprintln!("{name} is a null texture.");
            return None;
        }
        let t = self.load_file(fs, &path, flags);
        self.bucket.insert(name.to_string(), t.clone());
        self.shared.insert(name.to_string(), t.clone());
        Some(t)
    }

    /// `Texture.trygetTexture(name)`: also looks in `media/textures/`.
    pub fn try_get_texture(&mut self, fs: &GameFs, name: &str) -> Option<Texture> {
        if let Some(t) = self.shared_texture(fs, name) {
            return Some(t);
        }
        if name.contains("..") || name.contains("\0.\0.") {
            eprintln!("Failed to load texture: {name}");
            return None;
        }
        let mut file = format!("media/textures/{name}");
        if !name.ends_with(".png") {
            file.push_str(".png");
        }
        if let Some(t) = self.shared.get(&file) {
            return Some(t.clone());
        }
        let path = fs.lookup(&file)?.to_string();
        let f = if self.use_compression { flags::COMPRESS } else { 0 };
        let t = self.load_file(fs, &path, f);
        self.bucket.insert(file.clone(), t.clone());
        self.shared.insert(file, t.clone());
        Some(t)
    }

    /// A texture made from RGBA pixels (`new Texture(w, h, flags)` filled
    /// from a buffer), registered under `name`.
    pub fn texture_from_pixels(&mut self, name: &str, width: i32, height: i32, rgba: Vec<u8>, flags: i32) -> Texture {
        let img = ImageData::from_pixels(width, height, rgba);
        self.texture_from_image(name, name, img, flags)
    }

    /// `new Texture(w, h, flags)` with no pixels: a texture a backend draws
    /// into (an FBO target), registered under `name`. Its image lives only
    /// on the GPU.
    pub fn render_texture(&mut self, name: &str, width: i32, height: i32, flags: i32) -> Texture {
        let img = ImageData::new(width, height);
        let mut d = Self::data_from_image(name, Some(img), flags);
        d.image = None;
        let id = self.add_data(d);
        self.whole_texture(id, name)
    }

    /// `new VideoTexture(file, width, height)`: `Texture(width, height, 0)`
    /// (a blank power-of-two image) with texture coordinates 0..1, since
    /// every frame respecifies the image at exactly `width` x `height`
    /// ([`Self::tex_image`]). `path` must be unique.
    pub fn video_texture(&mut self, path: &str, width: i32, height: i32) -> Texture {
        let img = ImageData::new(width, height);
        let id = self.add_data(Self::data_from_image(path, Some(img), 0));
        let mut t = self.whole_texture(id, "");
        t.x_start = 0.0;
        t.y_start = 0.0;
        t.x_end = 1.0;
        t.y_end = 1.0;
        t
    }

    /// `glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, width, height, ...,
    /// GL_UNSIGNED_BYTE, rgba)` on texture `id` after `GL_NEAREST`
    /// filtering and clamping were set: replaces the image with `rgba`
    /// (`width * height * 4` bytes, rows of `width` pixels).
    pub fn tex_image(&mut self, id: u32, width: i32, height: i32, rgba: Vec<u8>) {
        let Some(d) = self.data.get_mut(id as usize) else { return };
        let img = ImageData {
            width,
            height,
            width_hw: width,
            height_hw: height,
            solid: true,
            data: MipLevel { width, height, pixels: rgba },
            mask: None,
            preserve_transparent_color: false,
        };
        d.flags = (d.flags | flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST | flags::CLAMP_TO_EDGE) & !flags::MIPMAPS;
        d.width = width;
        d.height = height;
        d.width_hw = width;
        d.height_hw = height;
        d.image = Some(img);
        d.ready = true;
        d.version = d.version.wrapping_add(1);
    }

    /// `TextureID.destroy()`: frees the image; the texture draws nothing
    /// afterwards.
    pub fn destroy_data(&mut self, id: u32) {
        if let Some(d) = self.data.get_mut(id as usize) {
            d.image = None;
            d.ready = false;
            d.version = d.version.wrapping_add(1);
        }
    }

    /// `Texture.getWhite()`: a 32x32 white texture, also registered as
    /// `white.png`, `media/white.png` and `media/ui/white.png`.
    pub fn white(&mut self) -> Texture {
        if let Some(t) = &self.white {
            return t.clone();
        }
        let img = ImageData::from_pixels(32, 32, vec![0xFF; 32 * 32 * 4]);
        let t = self.texture_from_image("white", "white", img, 0);
        for k in ["white.png", "media/white.png", "media/ui/white.png"] {
            self.shared.insert(k.to_string(), t.clone());
        }
        self.white = Some(t.clone());
        t
    }

    /// `Texture.getErrorTexture()`: an 8x8 red/white checkerboard on 32x32.
    pub fn error_texture(&mut self) -> Texture {
        if let Some(t) = &self.error {
            return t.clone();
        }
        let mut px = vec![0u8; 32 * 32 * 4];
        let mut row_start_red = true;
        let mut red = true;
        for index in 0..64 {
            let (row, col) = (index / 8, index % 8);
            if row > 0 && col == 0 {
                row_start_red = !row_start_red;
                red = row_start_red;
            }
            let rgba: [u8; 4] = if red { [0xFF, 0, 0, 0xFF] } else { [0xFF; 4] };
            red = !red;
            for y in 0..4 {
                for x in 0..4 {
                    let i = ((row * 4 + y) * 32 + col * 4 + x) * 4;
                    px[i..i + 4].copy_from_slice(&rgba);
                }
            }
        }
        let t = self.texture_from_image("EngineErrorTexture", "EngineErrorTexture", ImageData::from_pixels(32, 32, px), 0);
        self.shared.insert("EngineErrorTexture.png".into(), t.clone());
        self.error = Some(t.clone());
        t
    }

    // ---- asset loading ----------------------------------------------------

    fn add_data(&mut self, d: TextureData) -> u32 {
        let id = self.data.len() as u32;
        self.data_by_path.insert(d.path.clone(), id);
        self.data.push(d);
        id
    }

    fn data_from_image(path: &str, img: Option<ImageData>, flags: i32) -> TextureData {
        match img {
            Some(mut img) => TextureData {
                path: path.to_string(),
                flags,
                width: img.width,
                height: img.height,
                width_hw: img.width_hw,
                height_hw: img.height_hw,
                solid: img.solid,
                mask: img.mask.take().map(Rc::new),
                image: Some(img),
                ready: true,
                version: 0,
            },
            None => TextureData {
                path: path.to_string(),
                flags,
                width: 0,
                height: 0,
                width_hw: 0,
                height_hw: 0,
                solid: true,
                image: None,
                mask: None,
                ready: false,
                version: 0,
            },
        }
    }

    /// `new Texture(TextureID, name)` for an in-memory image.
    fn texture_from_image(&mut self, path: &str, name: &str, img: ImageData, flags: i32) -> Texture {
        let id = self.add_data(Self::data_from_image(path, Some(img), flags));
        self.whole_texture(id, name)
    }

    /// `Texture.onBeforeReady` for a texture covering its whole image.
    fn whole_texture(&self, id: u32, name: &str) -> Texture {
        let d = &self.data[id as usize];
        let mut t = Texture::empty(name);
        t.id = Some(id);
        t.ready = d.ready;
        t.solid = d.solid;
        t.width_hw = d.width_hw;
        t.height_hw = d.height_hw;
        if d.ready {
            t.width = d.width as u32;
            t.height = d.height as u32;
            t.x_end = d.width as f32 / d.width_hw as f32;
            t.y_end = d.height as f32 / d.height_hw as f32;
            t.uv = [0.0, 0.0, d.width as f32, d.height as f32];
            if let Some(g) = &d.mask {
                t.mask = Some(Rc::new(Mask::copy_of(g, d.width, d.height)));
            }
        }
        t
    }

    /// `TextureAssetManager.load(path)` for a loose image file.
    fn load_file(&mut self, fs: &GameFs, path: &str, flags: i32) -> Texture {
        if let Some(t) = self.assets.get(path) {
            return t.clone();
        }
        let id = match self.data_by_path.get(path) {
            Some(&id) => id,
            None => {
                let img = fs
                    .resolve(path)
                    .and_then(|p| File::open(p).ok())
                    .and_then(|f| ImageData::from_png(BufReader::new(f), false).map_err(|e| eprintln!("{path}: {e}")).ok());
                self.add_data(Self::data_from_image(path, img, flags))
            }
        };
        let t = self.whole_texture(id, path);
        self.assets.insert(path.to_string(), t.clone());
        t
    }

    /// `TextureAssetManager.load("@pack/<pack>/<page>/<name>")`: a region of
    /// a pack page, which is decoded once (`@pack@/<pack>/<page>`).
    fn load_sub_texture(&mut self, st: &SubTexture) -> Texture {
        let key = format!("@pack/{}/{}/{}", st.pack_name, st.page_name, st.info.name);
        if let Some(t) = self.assets.get(&key) {
            return t.clone();
        }
        let page_key = format!("@pack@/{}/{}", st.pack_name, st.page_name);
        let id = match self.data_by_path.get(&page_key) {
            Some(&id) => id,
            None => {
                let (img, flags) = match self.devices.get(&st.pack_name) {
                    Some((dev, pack_flags)) => {
                        let alpha = dev.is_alpha(&st.page_name);
                        let img = dev
                            .open_page(&st.page_name)
                            .and_then(|r| ImageData::from_png(r, alpha))
                            .map_err(|e| eprintln!("{page_key}: {e}"))
                            .ok();
                        (img, pack_flags | if alpha { flags::CREATE_MASK } else { 0 })
                    }
                    None => (None, 0),
                };
                self.add_data(Self::data_from_image(&page_key, img, flags))
            }
        };
        let d = &self.data[id as usize];
        let i = &st.info;
        let mut t = Texture::empty(&i.name);
        t.id = Some(id);
        t.width = i.w.max(0) as u32;
        t.height = i.h.max(0) as u32;
        t.offset_x = i.ox as f32;
        t.offset_y = i.oy as f32;
        t.width_orig = i.fx;
        t.height_orig = i.fy;
        t.width_hw = d.width_hw;
        t.height_hw = d.height_hw;
        t.uv = [i.x as f32, i.y as f32, i.w as f32, i.h as f32];
        t.ready = d.ready;
        if d.ready {
            t.solid = d.solid;
            t.set_region(i.x, i.y, i.w, i.h);
            if let Some(g) = &d.mask {
                t.mask = Some(Rc::new(Mask::sub_mask(g.clone(), i.x, i.y, i.w, i.h)));
            }
        }
        self.assets.insert(key, t.clone());
        t
    }
}

/// [`TextureLoader`] over a [`TextureManager`] and the file system.
pub struct SharedTextures<'a> {
    pub manager: &'a mut TextureManager,
    pub fs: &'a GameFs,
}

impl TextureLoader for SharedTextures<'_> {
    fn shared_texture(&mut self, path: &str) -> Option<Texture> {
        self.manager.shared_texture(self.fs, path)
    }

    fn image_size(&self, id: u32) -> Option<(i32, i32)> {
        self.manager.data(id).map(|d| (d.width, d.height))
    }
}
