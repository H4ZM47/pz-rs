//! `TileDepthTextures` (the tilesets of one mod, or of the game),
//! `TileDepthTextureManager` (all of them, merged), and
//! `TileDepthMapManager` (the wall/floor presets).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_fs::GameFs;

use crate::{Texture, TextureManager};

use super::assignments::TileDepthTextureAssignmentManager;
use super::texture::{TileDepthTexture, TilesetDepthTexture, TILESET_COLUMNS};

/// "Does the sprite texture with this name exist" (`Texture.getSharedTexture`
/// != null), given the texture manager.
pub type HasTexture<'a> = dyn FnMut(&mut TextureManager, &str) -> bool + 'a;

/// Rows `getTilesetRows` probes (0..=63).
const MAX_TILESET_ROWS: i32 = 64;
/// Name of the whole-tile default sheet (`DEPTH_whole_tile.png`).
pub const WHOLE_TILE: &str = "whole_tile";
/// Name of the billboard sheet (`DEPTH_billboard.png`).
pub const BILLBOARD: &str = "billboard";
/// Name of the preset sheet (`DEPTH_preset_depthmaps_01.png`).
pub const PRESET_TILESET: &str = "preset_depthmaps_01";

/// `TileDepthTextures`: the depth tilesets found in one `media/depthmaps`.
#[derive(Debug, Default)]
pub struct TileDepthTextures {
    /// `modId`: `game` for the base game, `None` for the merged set.
    pub mod_id: Option<String>,
    /// `mediaAbsPath`.
    pub media_dir: Option<PathBuf>,
    /// `tilesetRows`.
    tileset_rows: HashMap<String, i32>,
    tilesets: HashMap<String, TilesetDepthTexture>,
}

impl TileDepthTextures {
    pub fn new(mod_id: Option<&str>, media_dir: Option<&Path>) -> Self {
        TileDepthTextures { mod_id: mod_id.map(str::to_string), media_dir: media_dir.map(Path::to_path_buf), ..Default::default() }
    }

    /// `getExistingTileset(name)`.
    pub fn tileset(&self, name: &str) -> Option<&TilesetDepthTexture> {
        self.tilesets.get(name)
    }

    /// Every tileset, in no particular order.
    pub fn tilesets(&self) -> impl Iterator<Item = &TilesetDepthTexture> {
        self.tilesets.values()
    }

    /// `getTexture(tileset, index)` on a tileset already loaded.
    pub fn texture(&self, tileset: &str, index: i32) -> Option<&Rc<TileDepthTexture>> {
        self.tilesets.get(tileset)?.tile(index)
    }

    /// `getTextureFromTileName(name)`: `<tileset>_<index>`.
    pub fn texture_from_tile_name(&self, tile_name: &str) -> Option<&Rc<TileDepthTexture>> {
        let (tileset, index) = split_tile_name(tile_name)?;
        self.texture(tileset, index)
    }

    /// `getTilesetRows(name, false)`: one past the last row of eight tile
    /// names `<name>_<index>` that has any texture, 0 for none.
    fn probe_tileset_rows(tileset: &str, tm: &mut TextureManager, has_texture: &mut HasTexture) -> i32 {
        for row in (0..MAX_TILESET_ROWS).rev() {
            for col in 0..TILESET_COLUMNS {
                if has_texture(tm, &format!("{tileset}_{}", col + row * TILESET_COLUMNS)) {
                    return row + 1;
                }
            }
        }
        0
    }

    /// `loadDepthTextureImages()` with its load tasks run at once: every
    /// `DEPTH_<tileset>.png` in `<media>/depthmaps` (except the whole-tile
    /// and preset sheets) whose tileset has sprite textures becomes an
    /// eight-column, 2x tileset with as many rows as the sprites need.
    fn load_depth_texture_images(&mut self, tm: &mut TextureManager, has_texture: &mut HasTexture, keep_pixels: bool, errors: &mut Vec<String>) {
        let Some(media) = self.media_dir.clone() else { return };
        let Ok(entries) = std::fs::read_dir(media.join("depthmaps")) else { return };
        let mut tasks = Vec::new();
        for e in entries.flatten() {
            let path = e.path();
            if !path.is_file() || !path.to_string_lossy().ends_with(".png") {
                continue;
            }
            let file_name = e.file_name().to_string_lossy().into_owned();
            if !file_name.starts_with("DEPTH_")
                || !file_name.ends_with(".png")
                || file_name.eq_ignore_ascii_case("DEPTH_whole_tile.png")
                || file_name.eq_ignore_ascii_case("DEPTH_preset_depthmaps_01.png")
            {
                continue;
            }
            let tileset = file_name[6..file_name.len() - 4].to_string();
            let rows = Self::probe_tileset_rows(&tileset, tm, has_texture);
            self.tileset_rows.insert(tileset, rows);
            tasks.push(file_name);
        }
        // `LoadTask.call`: the name is derived again, slightly differently.
        tasks.sort();
        for file_name in tasks {
            let tileset = file_name.replacen("DEPTH_", "", 1).replace(".png", "");
            if !self.tilesets.contains_key(&tileset) {
                self.create_tileset(&tileset, tm, keep_pixels, errors);
            }
        }
    }

    /// `createTileset(name, true)`: `None` when the tileset has no rows.
    fn create_tileset(&mut self, name: &str, tm: &mut TextureManager, keep_pixels: bool, errors: &mut Vec<String>) -> Option<&TilesetDepthTexture> {
        let rows = self.tileset_rows.get(name).copied().unwrap_or(0);
        if rows == 0 {
            return None;
        }
        let media = self.media_dir.clone().unwrap_or_default();
        let mut tileset = TilesetDepthTexture::new(&media, name, TILESET_COLUMNS, rows, true);
        load_if_exists(&mut tileset, tm, keep_pixels, errors);
        self.tilesets.insert(name.to_string(), tileset);
        self.tilesets.get(name)
    }

    /// `mergeTilesets(other)`.
    fn merge_tilesets(&mut self, other: &TileDepthTextures) {
        for tileset in other.tilesets.values() {
            self.merge_tileset(tileset);
        }
    }

    /// `mergeTileset(other)`: a tileset of the other's size is created on
    /// first sight, then filled from the other's non-empty tiles.
    fn merge_tileset(&mut self, other: &TilesetDepthTexture) {
        let media = self.media_dir.clone().unwrap_or_default();
        let merged = self.tilesets.entry(other.name().to_string()).or_insert_with(|| {
            TilesetDepthTexture::new(&media, other.name(), other.width() / other.tile_width(), other.height() / other.tile_height(), other.is_2x())
        });
        merged.merge_tileset(other);
    }
}

/// `load()` guarded by `fileExists()`, collecting the logged errors.
fn load_if_exists(tileset: &mut TilesetDepthTexture, tm: &mut TextureManager, keep_pixels: bool, errors: &mut Vec<String>) {
    if tileset.file_exists() {
        if let Err(e) = tileset.load(tm, keep_pixels) {
            errors.push(format!("{}: {e}", tileset.path().display()));
        }
    }
}

/// `<tileset>_<index>` split at the last '_' (`PZMath.tryParseInt` on the
/// index, -1 meaning "none").
pub fn split_tile_name(tile_name: &str) -> Option<(&str, i32)> {
    let p = tile_name.rfind('_')?;
    let index = pz_fs::java::try_parse_int(&tile_name[p + 1..], -1);
    (index != -1).then(|| (&tile_name[..p], index))
}

/// `TileDepthTextureManager.ModData`.
#[derive(Debug)]
pub struct ModData {
    pub mod_id: String,
    pub media_dir: PathBuf,
    pub textures: TileDepthTextures,
}

/// `TileDepthMapManager.TileDepthPreset`: the cells of the preset sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TileDepthPreset {
    Floor,
    WDoorFrame,
    NDoorFrame,
    WWall,
    NWall,
    NWWall,
    SEWall,
}

impl TileDepthPreset {
    /// `Max.index`: the number of preset cells.
    pub const MAX: usize = 8;

    /// Column of the preset in the preset sheet (`index()`).
    pub fn index(self) -> usize {
        match self {
            TileDepthPreset::Floor => 0,
            TileDepthPreset::WDoorFrame => 2,
            TileDepthPreset::NDoorFrame => 3,
            TileDepthPreset::WWall => 4,
            TileDepthPreset::NWall => 5,
            TileDepthPreset::NWWall => 6,
            TileDepthPreset::SEWall => 7,
        }
    }
}

/// `TileDepthTextureManager` together with `TileDepthMapManager`'s presets.
///
/// Loading is synchronous: [`TileDepthTextureManager::init`] does what
/// `init()` and its queued load tasks do, then merges the sets
/// (`initMergedTilesets`).
#[derive(Debug)]
pub struct TileDepthTextureManager {
    /// `modData`: mods with a `tileGeometry.txt`, in load order, then the
    /// game.
    pub mod_data: Vec<ModData>,
    merged: TileDepthTextures,
    default_tileset: Option<TilesetDepthTexture>,
    billboard_tileset: Option<TilesetDepthTexture>,
    preset_tileset: Option<TilesetDepthTexture>,
    /// `TileDepthMapManager.presets`.
    presets: Vec<Texture>,
    /// `emptyDepthTextures`, one per size.
    empty_textures: Vec<Texture>,
    /// `TilesetDepthTexture.keepPixels` for every tileset: keep the float
    /// pixels after the upload (the editor's mode). Off in the game.
    pub keep_pixels: bool,
    /// Files that failed to decode.
    pub errors: Vec<String>,
}

impl TileDepthTextureManager {
    fn empty() -> Self {
        TileDepthTextureManager {
            mod_data: Vec::new(),
            merged: TileDepthTextures::new(None, None),
            default_tileset: None,
            billboard_tileset: None,
            preset_tileset: None,
            presets: Vec::new(),
            empty_textures: Vec::new(),
            keep_pixels: false,
            errors: Vec::new(),
        }
    }

    /// `init()` then `TileDepthMapManager.init()`. Tileset rows are probed
    /// with `Texture.getSharedTexture` on the texture manager, so the tile
    /// texture packs must be loaded first, as in the original.
    pub fn init(fs: &mut GameFs, tm: &mut TextureManager) -> Self {
        let media = Self::media_dirs(fs);
        let fs: &GameFs = fs;
        let mut probe = |tm: &mut TextureManager, name: &str| tm.shared_texture(fs, name).is_some();
        Self::init_from_dirs(&media, Path::new(fs.media_root()), tm, &mut probe, false)
    }

    /// [`TileDepthTextureManager::init`] with another test for "the sprite
    /// texture `<tileset>_<index>` exists".
    pub fn init_with(fs: &mut GameFs, tm: &mut TextureManager, has_texture: &mut dyn FnMut(&str) -> bool) -> Self {
        let media = Self::media_dirs(fs);
        let mut probe = |_: &mut TextureManager, name: &str| has_texture(name);
        Self::init_from_dirs(&media, Path::new(fs.media_root()), tm, &mut probe, false)
    }

    /// Media directories of the loaded mods that have a `tileGeometry.txt`
    /// (`ChooseGameInfo.Mod.mediaFile.common`), in load order.
    fn media_dirs(fs: &mut GameFs) -> Vec<(String, PathBuf)> {
        mod_media_dirs(fs, "tileGeometry.txt")
    }

    /// The loading behind [`TileDepthTextureManager::init`]: `mods` are
    /// (mod id, media directory) pairs, `game_media` the game's `media/`.
    pub fn init_from_dirs(
        mods: &[(String, PathBuf)],
        game_media: &Path,
        tm: &mut TextureManager,
        has_texture: &mut HasTexture,
        keep_pixels: bool,
    ) -> Self {
        let mut m = Self::empty();
        m.keep_pixels = keep_pixels;
        let mut errors = Vec::new();
        for (id, media) in mods {
            m.mod_data.push(Self::load_mod_data(id, media, tm, has_texture, keep_pixels, &mut errors));
        }
        let mut game = Self::load_mod_data("game", game_media, tm, has_texture, keep_pixels, &mut errors);
        // `getDefaultDepthTexture` / `getBillboardDepthTexture` /
        // `getPresetTilesetDepthTexture` load lazily from the game's media.
        let one = |name: &str, columns: i32, tm: &mut TextureManager, errors: &mut Vec<String>| {
            let mut t = TilesetDepthTexture::new(game_media, name, columns, 1, true);
            load_if_exists(&mut t, tm, keep_pixels, errors);
            t
        };
        let preset = one(PRESET_TILESET, TileDepthPreset::MAX as i32, tm, &mut errors);
        // `hackAddPresetTilesetDepthTexture`.
        game.textures.tilesets.insert(PRESET_TILESET.to_string(), preset.clone());
        m.preset_tileset = Some(preset);
        m.default_tileset = Some(one(WHOLE_TILE, 1, tm, &mut errors));
        m.billboard_tileset = Some(one(BILLBOARD, 1, tm, &mut errors));
        m.mod_data.push(game);
        // `initMergedTilesets`.
        for data in &m.mod_data {
            m.merged.merge_tilesets(&data.textures);
        }
        // `TileDepthMapManager.init`.
        m.presets = (0..TileDepthPreset::MAX as i32)
            .map(|x| {
                let tile = m.preset_tileset.as_ref().and_then(|t| t.tile_at(x, 0)).cloned();
                m.texture_of_opt(tile.as_deref(), tm)
            })
            .collect();
        m.errors = errors;
        m
    }

    fn load_mod_data(id: &str, media: &Path, tm: &mut TextureManager, has_texture: &mut HasTexture, keep_pixels: bool, errors: &mut Vec<String>) -> ModData {
        let mut textures = TileDepthTextures::new(Some(id), Some(media));
        textures.load_depth_texture_images(tm, has_texture, keep_pixels, errors);
        ModData { mod_id: id.to_string(), media_dir: media.to_path_buf(), textures }
    }
}

impl TileDepthTextureManager {
    /// The merged tilesets: for every tile, the first non-empty one in
    /// [`TileDepthTextureManager::mod_data`] order (mods before the game).
    pub fn merged(&self) -> &TileDepthTextures {
        &self.merged
    }

    /// `getModData(modID)`.
    pub fn mod_data(&self, mod_id: &str) -> Option<&ModData> {
        self.mod_data.iter().find(|d| d.mod_id == mod_id)
    }

    /// `getTexture(tilesetName, tileIndex)`: the merged tile, `None` for an
    /// unknown tileset or an index outside it. The tile may be empty.
    pub fn texture(&self, tileset: &str, index: i32) -> Option<&Rc<TileDepthTexture>> {
        self.merged.texture(tileset, index)
    }

    /// `getTextureFromTileName(tileName)`.
    pub fn texture_from_tile_name(&self, tile_name: &str) -> Option<&Rc<TileDepthTexture>> {
        self.merged.texture_from_tile_name(tile_name)
    }

    /// `getDefaultDepthTexture()`: the whole-tile depth used for objects
    /// with nothing better.
    pub fn default_depth_texture(&self) -> Option<&Rc<TileDepthTexture>> {
        self.default_tileset.as_ref()?.tile_at(0, 0)
    }

    /// `getBillboardDepthTexture()`: used for fire sprites.
    pub fn billboard_depth_texture(&self) -> Option<&Rc<TileDepthTexture>> {
        self.billboard_tileset.as_ref()?.tile_at(0, 0)
    }

    /// `getPresetTilesetDepthTexture()`.
    pub fn preset_tileset(&self) -> Option<&TilesetDepthTexture> {
        self.preset_tileset.as_ref()
    }

    /// `getPresetDepthTexture(col, row)`.
    pub fn preset_depth_texture(&self, col: i32, row: i32) -> Option<&Rc<TileDepthTexture>> {
        self.preset_tileset.as_ref()?.tile_at(col, row)
    }

    /// `TileDepthMapManager.getTextureForPreset(preset)`.
    pub fn texture_for_preset(&self, preset: TileDepthPreset) -> &Texture {
        &self.presets[preset.index()]
    }

    /// `TileDepthMapManager.presets`, indexed by preset column.
    pub fn presets(&self) -> &[Texture] {
        &self.presets
    }

    /// `getEmptyDepthTexture(width, height)`: an all-zero ("no depth")
    /// texture of that size, shared by every empty tile.
    pub fn empty_depth_texture(&mut self, width: i32, height: i32, tm: &mut TextureManager) -> Texture {
        if let Some(t) = self.empty_textures.iter().find(|t| t.width as i32 == width && t.height as i32 == height) {
            return t.clone();
        }
        let name = format!("DEPTH_Empty_{width}x{height}");
        let size = (crate::image::pow2(width) * crate::image::pow2(height) * 4) as usize;
        // `new Texture(w, h, name, 0)`: no pixels are ever uploaded; GL
        // leaves them undefined, zero here.
        let t = tm.texture_from_pixels(&name, width, height, vec![0; size], 0);
        self.empty_textures.push(t.clone());
        t
    }

    /// `TileDepthTexture.getTexture()`: the uploaded texture, or the empty
    /// texture of the tile's size for a tile that never had depth.
    pub fn texture_of(&mut self, tile: &TileDepthTexture, tm: &mut TextureManager) -> Texture {
        match tile.uploaded_texture() {
            Some(t) => t.clone(),
            None => self.empty_depth_texture(tile.width(), tile.height(), tm),
        }
    }

    fn texture_of_opt(&mut self, tile: Option<&TileDepthTexture>, tm: &mut TextureManager) -> Texture {
        match tile {
            Some(t) => self.texture_of(t, tm),
            // Never reached: the preset sheet always has eight cells.
            None => self.empty_depth_texture(128, 256, tm),
        }
    }

    /// `IsoSprite.depthTexture` as `initSprites` assigns it: the sprite's
    /// own non-empty tile (first mod data that has one), else, for a sprite
    /// with a tileset name, the non-empty merged tile it is assigned to in
    /// `tileDepthTextureAssignments.txt`.
    pub fn sprite_depth_texture(&self, sprite_name: &str, has_tileset_name: bool, assignments: &TileDepthTextureAssignmentManager) -> Option<&Rc<TileDepthTexture>> {
        if let Some((tileset, index)) = split_tile_name(sprite_name) {
            // `TilesetDepthTexture.initSprites` matches tile names exactly.
            if format!("{tileset}_{index}") == sprite_name {
                for data in &self.mod_data {
                    if let Some(tile) = data.textures.texture(tileset, index) {
                        if !tile.is_empty() {
                            return Some(tile);
                        }
                    }
                }
            }
        }
        if !has_tileset_name {
            return None;
        }
        let other = assignments.assigned_tile_name(sprite_name)?;
        self.texture_from_tile_name(other).filter(|t| !t.is_empty())
    }
}

/// `<common>/media` of every loaded, available mod whose media directory
/// holds `marker`, in load order.
pub(super) fn mod_media_dirs(fs: &mut GameFs, marker: &str) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for id in fs.mod_ids().to_vec() {
        let Some(dir) = fs.available_mod_dir(&id) else { continue };
        let Some(info) = fs.mod_info(&dir) else { continue };
        let media = PathBuf::from(&info.common_dir).join("media");
        if media.join(marker).exists() {
            out.push((id, media));
        }
    }
    out
}
