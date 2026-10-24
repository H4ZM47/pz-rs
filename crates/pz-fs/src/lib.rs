//! `zombie.ZomboidFileSystem`: locating game media and mod files.
//!
//! At start-up the original walks `media/` once and records every file (and
//! every directory under `media/maps/`) in `activeFileMap`, keyed by its
//! lower-cased path relative to the install (`media/lua/client/foo.lua`).
//! Loading a mod walks the mod's `common/` and version folders and overwrites
//! entries, so later mods override earlier ones and the game. Every lookup
//! (`getString`) goes through that map; nothing else touches the disk.
//!
//! Also here: mod discovery and load order (`ChooseGameInfo`, `loadMods`),
//! `LuaManager.LoadDirBase` ordering, and `.pack` reading
//! (`zombie.fileSystem.TexturePackDevice`).

pub mod java;
pub mod jfile;
pub mod mods;
pub mod pack;
pub mod script_parser;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use jfile::SEP;
pub use mods::{ActiveMods, GameVersion, ModInfo, PackFile, TileDef};

pub struct GameFs {
    /// Directory containing `media/` (the original's `base`), canonical.
    pub base: PathBuf,
    base_str: String,
    /// `base.lowercaseUri`'s path.
    base_lower_uri: String,
    /// `workdir`: `<base>/media`, canonical.
    media: String,
    /// `activeFileMap`: lower-cased relative path -> absolute path.
    pub active_file_map: HashMap<String, String>,
    all_absolute_paths: HashSet<String>,
    /// `relativeMap`: memoised `getRelativeFile` results.
    relative_map: Mutex<HashMap<String, String>>,
    /// `Core.getGameVersion()`.
    pub game_version: GameVersion,
    /// `TextureID.useCompressionOption`; feeds mod pack flags.
    pub texture_compression: bool,
    /// `getCacheDir()` / `Core.getMyDocumentFolder()`: `~/Zomboid`.
    cache_dir: PathBuf,
    mod_folders_order: Vec<String>,
    /// Steam: `SteamWorkshop.GetInstalledItemFolders()` (each holds `mods/`).
    pub steam_item_folders: Vec<PathBuf>,
    /// Steam: `SteamWorkshop.getStageFolders()` (each holds `Contents/mods/`).
    pub workshop_stage_folders: Vec<PathBuf>,
    mod_folders: Option<Vec<String>>,
    mod_id_to_dir: HashMap<String, String>,
    /// `modDirToMod` (`getModInfoForDir`): one `Mod` per directory.
    mod_dir_to_mod: HashMap<String, ModInfo>,
    /// `ChooseGameInfo.Mods`: mod id -> directory of the chosen `Mod`.
    mods_by_id: HashMap<String, String>,
    missing_mods: HashSet<String>,
    /// `mods`: ids loaded by the last `load_mods`, in load order.
    mods: Vec<String>,
    loaded_packs: HashSet<String>,
}

impl GameFs {
    /// `ZomboidFileSystem.init()` with `base` as the working directory.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        let base: PathBuf = base.into();
        let cwd = std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let base_str = jfile::canonical(&base.to_string_lossy(), &cwd);
        let base_lower_uri = jfile::uri_path(&java::lower(&base_str));
        let media = jfile::canonical(&format!("{base_str}{SEP}media"), &base_str);
        let mut fs = GameFs {
            base: PathBuf::from(&base_str),
            base_str,
            base_lower_uri,
            media,
            active_file_map: HashMap::new(),
            all_absolute_paths: HashSet::new(),
            relative_map: Mutex::new(HashMap::new()),
            game_version: GameVersion::current(),
            texture_compression: true,
            cache_dir: default_cache_dir(),
            mod_folders_order: vec!["workshop".into(), "steam".into(), "mods".into()],
            steam_item_folders: Vec::new(),
            workshop_stage_folders: Vec::new(),
            mod_folders: None,
            mod_id_to_dir: HashMap::new(),
            mod_dir_to_mod: HashMap::new(),
            mods_by_id: HashMap::new(),
            missing_mods: HashSet::new(),
            mods: Vec::new(),
            loaded_packs: HashSet::new(),
        };
        let mut load_list = Vec::new();
        search_folders(Path::new(&fs.media), &mut load_list);
        for entry in load_list {
            let rel = fs.relative_file(&entry);
            let mut abs = jfile::absolute(&entry, &fs.base_str);
            if Path::new(&abs).is_dir() {
                abs.push(SEP);
            }
            fs.active_file_map.insert(java::lower(&rel), abs.clone());
            fs.all_absolute_paths.insert(abs);
        }
        fs
    }

    /// `setCacheDir`: the user folder holding `mods/`, `Saves/`, ...
    pub fn set_cache_dir(&mut self, dir: impl Into<PathBuf>) {
        self.cache_dir = dir.into();
        self.reset_mod_folders();
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// `setModFoldersOrder("workshop,steam,mods")`.
    pub fn set_mod_folders_order(&mut self, s: &str) {
        self.mod_folders_order = s.split(',').map(str::to_string).collect();
    }

    /// `resetModFolders`.
    pub fn reset_mod_folders(&mut self) {
        self.mod_folders = None;
    }

    /// `getMediaFile(sub)`.
    pub fn media_file(&self, sub: &str) -> PathBuf {
        PathBuf::from(&self.media).join(sub)
    }

    /// `getMediaRootPath()`.
    pub fn media_root(&self) -> &str {
        &self.media
    }

    /// `getRelativeFile(string)`: relative to the install when inside it,
    /// returned unchanged when outside.
    pub fn relative_file(&self, s: &str) -> String {
        self.relative_file_to(&self.base_lower_uri, s)
    }

    /// `getRelativeFile(URI root, String)`; `root` is a lower-case URI path.
    pub fn relative_file_to(&self, root: &str, s: &str) -> String {
        let abs = jfile::absolute(s, &self.base_str);
        let uri = jfile::uri_path(&java::lower(&abs));
        match jfile::relativize(root, &uri) {
            None => s.to_string(),
            Some(mut rel) => {
                if s.ends_with('/') && !rel.ends_with('/') {
                    rel.push('/');
                }
                rel
            }
        }
    }

    fn relative_key(&self, s: &str) -> String {
        let lower = java::lower(s);
        let mut cache = self.relative_map.lock().unwrap();
        if let Some(rel) = cache.get(&lower) {
            return rel.clone();
        }
        let rel = java::lower(&self.relative_file(s));
        cache.insert(lower, rel.clone());
        rel
    }

    /// `getString(str)` when the path is known: the absolute path of the
    /// active (possibly mod-overridden) file.
    pub fn lookup(&self, s: &str) -> Option<&str> {
        self.active_file_map.get(&self.relative_key(s)).map(String::as_str)
    }

    /// `getString(str)`: the known absolute path, or `s` itself.
    pub fn get_string(&self, s: &str) -> String {
        self.lookup(s).unwrap_or(s).to_string()
    }

    /// `getDirectoryString(str)` when known: also tries with or without a
    /// trailing '/'.
    pub fn directory_lookup(&self, s: &str) -> Option<&str> {
        if let Some(p) = self.lookup(s) {
            return Some(p);
        }
        match s.strip_suffix('/') {
            Some(t) => self.lookup(t),
            None => self.lookup(&format!("{s}/")),
        }
    }

    /// `getDirectoryString(str)`.
    pub fn get_directory_string(&self, s: &str) -> String {
        self.directory_lookup(s).unwrap_or(s).to_string()
    }

    /// `isKnownFile(str)`.
    pub fn is_known_file(&self, s: &str) -> bool {
        self.all_absolute_paths.contains(s) || self.lookup(s).is_some()
    }

    /// `getAbsolutePath(rel)`: direct map lookup of the lower-cased key.
    pub fn absolute_path(&self, rel: &str) -> Option<&str> {
        self.active_file_map.get(&java::lower(rel)).map(String::as_str)
    }

    /// `getAllPathsContaining(str)` (unordered, like the HashMap it scans).
    pub fn all_paths_containing(&self, s: &str) -> Vec<&str> {
        self.active_file_map.iter().filter(|(k, _)| k.contains(s)).map(|(_, v)| v.as_str()).collect()
    }

    /// Opens a path the way the original does: `getString(rel)`, then a
    /// file relative to the working directory (the install). So
    /// "media/lua/client/Foo.lua" finds the active, possibly mod-overridden
    /// file in any case, and "stdlib.lua" finds the file beside `media/`.
    /// `None` when neither exists.
    pub fn resolve(&self, rel: &str) -> Option<PathBuf> {
        if let Some(p) = self.lookup(rel) {
            return Some(PathBuf::from(p));
        }
        let p = PathBuf::from(jfile::absolute(rel, &self.base_str));
        p.exists().then_some(p)
    }

    /// `LuaManager.LoadDirBase(sub)` order: game files under
    /// `media/lua/<sub>` sorted case-insensitively, then each loaded mod's
    /// files (common folder, then version folder) sorted per mod, with
    /// duplicates dropped. Entries are lower-cased keys; use
    /// [`GameFs::absolute_path`] to get the file that wins.
    pub fn lua_files(&self, sub: &str) -> Vec<String> {
        let mut game = Vec::new();
        let dir = self.media_file(&format!("lua{SEP}{sub}"));
        let base_uri = self.base_lower_uri.clone();
        self.lua_search(&base_uri, &dir.to_string_lossy(), &mut game);
        let mut mod_files = Vec::new();
        for id in &self.mods {
            // `load_mods` already resolved and checked every loaded mod.
            let Some(m) = self.mods_by_id.get(id).and_then(|d| self.mod_dir_to_mod.get(d)) else { continue };
            if m.available != Some(true) {
                continue;
            }
            let (common, version) = (m.common_dir.clone(), m.version_dir.clone());
            let mut list = Vec::new();
            for root in [common, version] {
                let uri = jfile::uri_path(&java::lower(&jfile::canonical(&root, &self.base_str)));
                let media = canonical_child(&root, "media");
                let lua = canonical_child(&media, "lua");
                let subdir = canonical_child(&lua, sub);
                self.lua_search(&uri, &subdir, &mut list);
            }
            list.sort_by(|a, b| java::case_insensitive_cmp(a, b));
            mod_files.extend(list);
        }
        game.sort_by(|a, b| java::case_insensitive_cmp(a, b));
        game.extend(mod_files);
        let mut seen = HashSet::new();
        game.retain(|s| seen.insert(s.clone()));
        game
    }

    /// `LuaManager.searchFolders(URI, File)`.
    fn lua_search(&self, root: &str, path: &str, out: &mut Vec<String>) {
        let p = Path::new(path);
        if p.is_dir() {
            let Ok(rd) = std::fs::read_dir(p) else { return };
            let parent = jfile::canonical(path, &self.base_str);
            for e in rd.flatten() {
                let child = format!("{parent}{SEP}{}", e.file_name().to_string_lossy());
                self.lua_search(root, &child, out);
            }
        } else {
            let abs = jfile::absolute(path, &self.base_str);
            if abs.to_lowercase().ends_with(".lua") {
                out.push(java::lower(&self.relative_file_to(root, &abs)));
            }
        }
    }

    // ---- mods -------------------------------------------------------------

    /// `getModVersionDirName(modDir)`: the newest `<major>.<minor>` folder not
    /// newer than the game, else "42.0".
    pub fn mod_version_dir_name(&self, mod_dir: &str) -> String {
        let game = self.game_version.as_int();
        let mut name = GameVersion::min_required().to_string();
        let mut best = GameVersion::min_required().as_int();
        if let Ok(rd) = std::fs::read_dir(mod_dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                let v = mods::game_version_int_from_name(&n);
                if v < best || v > game {
                    continue;
                }
                name = n;
                best = v;
            }
        }
        name
    }

    /// `getAllModFolders(out)`: mod directories (those with a mod.info in
    /// `common/` or the version folder) in search order.
    pub fn all_mod_folders(&mut self) -> Vec<String> {
        if let Some(f) = &self.mod_folders {
            return f.clone();
        }
        let mut roots: Vec<String> = Vec::new();
        for s in self.mod_folders_order.clone() {
            if s == "workshop" {
                for f in &self.workshop_stage_folders {
                    let p = f.join("Contents").join("mods");
                    if p.exists() {
                        roots.push(jfile::absolute(&p.to_string_lossy(), &self.base_str));
                    }
                }
            }
            if s == "steam" {
                for f in &self.steam_item_folders {
                    let p = f.join("mods");
                    if p.exists() {
                        roots.push(jfile::absolute(&p.to_string_lossy(), &self.base_str));
                    }
                }
            }
            if s == "mods" {
                let p = self.cache_dir.join("mods");
                if !p.exists() {
                    let _ = std::fs::create_dir_all(&p);
                }
                roots.push(jfile::absolute(&p.to_string_lossy(), &self.base_str));
            }
        }
        let mut result = Vec::new();
        for root in roots {
            let Ok(rd) = std::fs::read_dir(&root) else { continue };
            for e in rd.flatten() {
                let path = e.path();
                if !path.is_dir() {
                    continue;
                }
                if e.file_name().to_string_lossy().eq_ignore_ascii_case("examplemod") {
                    eprintln!("refusing to list {}", e.file_name().to_string_lossy());
                    continue;
                }
                let p = path.to_string_lossy().into_owned();
                let ver = self.mod_version_dir_name(&p);
                if !path.join("common").join("mod.info").exists() && !path.join(&ver).join("mod.info").exists() {
                    continue;
                }
                result.push(jfile::absolute(&p, &self.base_str));
            }
        }
        self.mod_folders = Some(result.clone());
        result
    }

    /// `getModInfoForDir(dir)`: creates the per-directory `Mod` on demand.
    fn mod_for_dir(&mut self, dir: &str) -> &mut ModInfo {
        if !self.mod_dir_to_mod.contains_key(dir) {
            let common = jfile::absolute(&format!("{dir}{SEP}common"), &self.base_str);
            let version = jfile::absolute(&format!("{dir}{SEP}{}", self.mod_version_dir_name(dir)), &self.base_str);
            self.mod_dir_to_mod.insert(dir.to_string(), ModInfo::new(dir, common, version));
        }
        self.mod_dir_to_mod.get_mut(dir).unwrap()
    }

    /// `ChooseGameInfo.readModInfoAux(dir)`; returns the mod's directory
    /// (its identity) when the mod.info was read successfully.
    fn read_mod_info_aux(&mut self, dir: &str) -> Option<String> {
        let compression = self.texture_compression;
        let m = self.mod_for_dir(dir);
        if m.read {
            return m.valid.then(|| dir.to_string());
        }
        m.read = true;
        let mut path = format!("{}{SEP}mod.info", m.version_dir);
        if !Path::new(&path).exists() {
            path = format!("{}{SEP}mod.info", m.common_dir);
            if !Path::new(&path).exists() {
                eprintln!("can't find \"{dir}\"");
                return None;
            }
        }
        let resolved = self.get_string(&path);
        let bytes = std::fs::read(&resolved).ok()?;
        let text = java::decode_utf8(&bytes);
        let m = self.mod_dir_to_mod.get_mut(dir).unwrap();
        if !m.apply_mod_info(&text, compression, |p| Path::new(p).exists()) {
            return None;
        }
        // TODO(pz-text): `Translator.readModTranslation(mod)`.
        m.valid = true;
        Some(dir.to_string())
    }

    /// `ChooseGameInfo.readModInfo(dir)`: also registers the mod by id,
    /// preferring the directory that comes first in the search order.
    pub fn read_mod_info(&mut self, dir: &str) -> Option<&ModInfo> {
        let dir = self.read_mod_info_aux(dir)?;
        self.register_mod(&dir);
        self.mod_dir_to_mod.get(&dir)
    }

    fn register_mod(&mut self, dir: &str) {
        let id = self.mod_dir_to_mod[dir].id.clone();
        match self.mods_by_id.get(&id).cloned() {
            None => {
                self.mods_by_id.insert(id, dir.to_string());
            }
            Some(existing) if existing != dir => {
                let folders = self.all_mod_folders();
                let idx = |d: &str| folders.iter().position(|f| f == d).map_or(-1, |i| i as i64);
                if idx(dir) < idx(&existing) {
                    self.mods_by_id.insert(id, dir.to_string());
                }
            }
            Some(_) => {}
        }
    }

    /// `ChooseGameInfo.getModDetails(id)`; returns the mod's directory.
    pub fn mod_details(&mut self, id: &str) -> Option<String> {
        if self.missing_mods.contains(id) {
            return None;
        }
        if let Some(dir) = self.mods_by_id.get(id) {
            return Some(dir.clone());
        }
        let dir = self.mod_id_to_dir.get(id).cloned();
        if dir.is_none() {
            for folder in self.all_mod_folders() {
                let Some(found) = self.read_mod_info_aux(&folder) else { continue };
                self.register_mod(&found);
                let found_id = self.mod_dir_to_mod[&found].id.clone();
                self.mod_id_to_dir.entry(found_id.clone()).or_insert_with(|| folder.clone());
                if found_id == id {
                    return Some(found);
                }
            }
        }
        match dir.and_then(|d| self.read_mod_info_aux(&d)) {
            Some(d) => {
                self.register_mod(&d);
                Some(d)
            }
            None => {
                self.missing_mods.insert(id.to_string());
                None
            }
        }
    }

    /// The `Mod` read from `dir` (see [`GameFs::mod_details`]).
    pub fn mod_info(&self, dir: &str) -> Option<&ModInfo> {
        self.mod_dir_to_mod.get(dir)
    }

    /// `ChooseGameInfo.getAvailableModDetails(id)`; returns the directory.
    pub fn available_mod_dir(&mut self, id: &str) -> Option<String> {
        let dir = self.mod_details(id)?;
        self.is_available(&dir).then_some(dir)
    }

    /// `Mod.isAvailable()`: version range of the mod and, recursively, of
    /// everything it requires. Cached per mod, as in the original.
    fn is_available(&mut self, dir: &str) -> bool {
        if let Some(a) = self.mod_dir_to_mod[dir].available {
            return a;
        }
        let game = self.game_version.clone();
        let ok = self.mod_dir_to_mod[dir].is_available_self(&game) && {
            let mut seen = vec![self.mod_dir_to_mod[dir].id.clone()];
            self.is_available_required(dir, &mut seen)
        };
        self.mod_dir_to_mod.get_mut(dir).unwrap().available = Some(ok);
        ok
    }

    fn is_available_required(&mut self, dir: &str, seen: &mut Vec<String>) -> bool {
        let Some(require) = self.mod_dir_to_mod[dir].require.clone() else { return true };
        let game = self.game_version.clone();
        for r in require {
            let id = java::trim(&r).to_string();
            if seen.contains(&id) {
                continue;
            }
            seen.push(id.clone());
            let Some(d) = self.mod_details(&id) else { return false };
            if !self.mod_dir_to_mod[&d].is_available_self(&game) || !self.is_available_required(&d, seen) {
                return false;
            }
        }
        true
    }

    /// `setModIdToDir(id, dir)`.
    pub fn set_mod_id_to_dir(&mut self, id: &str, dir: &str) {
        self.mod_id_to_dir.entry(id.to_string()).or_insert_with(|| dir.to_string());
    }

    /// `getModDir(id)`.
    pub fn mod_dir(&self, id: &str) -> Option<&str> {
        self.mod_id_to_dir.get(id).map(String::as_str)
    }

    /// `loadModAndRequired`: `Some(id)` names the mod that was missing.
    fn load_mod_and_required(&mut self, id: &str, ordered: &mut Vec<String>) -> Option<String> {
        if id.is_empty() {
            return None;
        }
        if id.eq_ignore_ascii_case("examplemod") {
            eprintln!("refusing to load {id}");
            return None;
        }
        if ordered.iter().any(|o| o == id) {
            return None;
        }
        let Some(dir) = self.available_mod_dir(id) else {
            eprintln!("required mod \"{id}\" not found");
            return Some(id.to_string());
        };
        if let Some(require) = self.mod_dir_to_mod[&dir].require.clone() {
            if let Some(fail) = self.load_mods_aux(&require, ordered) {
                return Some(fail);
            }
        }
        ordered.push(id.to_string());
        None
    }

    /// `loadModsAux(toLoad, ordered)`.
    pub fn load_mods_aux(&mut self, to_load: &[String], ordered: &mut Vec<String>) -> Option<String> {
        to_load.iter().find_map(|id| self.load_mod_and_required(id, ordered))
    }

    /// `loadMods(List)`: orders mods after their requirements, then lets each
    /// override files in the active file map.
    pub fn load_mods(&mut self, to_load: &[String]) {
        let mut ordered = Vec::new();
        for id in to_load {
            self.load_mod_and_required(id, &mut ordered);
        }
        self.mods = ordered.clone();
        for id in ordered {
            self.load_mod(&id);
        }
    }

    /// `getModIDs()`.
    pub fn mod_ids(&self) -> &[String] {
        &self.mods
    }

    /// `loadMod(id)`.
    pub fn load_mod(&mut self, id: &str) {
        let Some(dir) = self.mod_id_to_dir.get(id).cloned() else { return };
        eprintln!("loading {id}");
        let (common, version) = {
            let m = self.mod_for_dir(&dir);
            (m.common_dir.clone(), m.version_dir.clone())
        };
        for root in [common, version] {
            // `new File(dir.toLowerCase()).toURI()`.
            let root_uri = jfile::uri_path(&jfile::absolute(&java::lower(&root), &self.base_str));
            let mut list = Vec::new();
            search_folders(Path::new(&root), &mut list);
            for entry in list {
                let rel = java::lower(&self.relative_file_to(&root_uri, &entry));
                if self.active_file_map.contains_key(&rel) && !rel.ends_with("mod.info") && !rel.ends_with("poster.png") {
                    eprintln!("mod \"{id}\" overrides {rel}");
                }
                let abs = jfile::absolute(&entry, &self.base_str);
                self.active_file_map.insert(rel, abs.clone());
                self.all_absolute_paths.insert(abs);
            }
        }
    }

    /// `readDefaultModsTxt`: `mods/default.txt`, after folding in (and
    /// deleting) a legacy `mods/loaded.txt`.
    pub fn read_default_mods(&mut self) -> ActiveMods {
        let dir = self.cache_dir.join("mods");
        let loaded = dir.join("loaded.txt");
        if loaded.exists() {
            if let Ok(bytes) = std::fs::read(&loaded) {
                let text = java::decode_utf8(&bytes);
                let mut current = mods::read_active_mods(&dir.join("default.txt")).unwrap_or_default();
                current.mods.extend(java::read_lines(&text).into_iter().map(java::trim).filter(|s| !s.is_empty()).map(str::to_string));
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(dir.join("default.txt"), mods::active_mods_to_string(&current));
            }
            let _ = std::fs::remove_file(&loaded);
        }
        mods::read_active_mods(&dir.join("default.txt")).unwrap_or_default()
    }

    /// `loadModPackFiles`: the texture packs the loaded mods declare, as
    /// `(pack name, flags, mod id)` in the order `GameWindow.LoadTexturePack`
    /// should be called. Packs already returned are skipped.
    pub fn mod_pack_files(&mut self) -> Vec<(String, i32, String)> {
        let mut out = Vec::new();
        for id in self.mods.clone() {
            let Some(dir) = self.available_mod_dir(&id) else { continue };
            for pack in self.mod_dir_to_mod[&dir].packs.clone() {
                let path = format!("media/texturepacks/{}.pack", pack.name);
                if !self.active_file_map.contains_key(&java::lower(&self.relative_file(&path))) {
                    eprintln!("pack file \"{}\" needed by {id} not found", pack.name);
                    continue;
                }
                let file = self.get_string(&path);
                if self.loaded_packs.contains(&file) {
                    continue;
                }
                out.push((pack.name.clone(), pack.flags, id.clone()));
                self.loaded_packs.insert(file);
            }
        }
        out
    }
}

/// `getCacheDir()` default: `<user.home>/Zomboid`.
fn default_cache_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_default();
    PathBuf::from(home).join("Zomboid")
}

/// `getCanonicalFile(parent, child)`: the first entry of `parent` whose name
/// equals `child` ignoring case, else `parent/child`.
fn canonical_child(parent: &str, child: &str) -> String {
    if let Ok(rd) = std::fs::read_dir(parent) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if java::case_insensitive_cmp(&n, child).is_eq() {
                return format!("{parent}{SEP}{n}");
            }
        }
    }
    format!("{parent}{SEP}{child}")
}

/// `ZomboidFileSystem.searchFolders(File)`: every file, plus directories
/// whose path contains `media/maps/`. Paths use '/' and lose any "./".
fn search_folders(dir: &Path, out: &mut Vec<String>) {
    let abs = dir.to_string_lossy();
    if dir.is_dir() {
        let path = abs.replace('\\', "/").replace("./", "");
        if path.contains("media/maps/") {
            out.push(path);
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            search_folders(&PathBuf::from(format!("{abs}{SEP}{}", e.file_name().to_string_lossy())), out);
        }
    } else {
        out.push(abs.replace('\\', "/").replace("./", ""));
    }
}

#[cfg(test)]
mod tests;
