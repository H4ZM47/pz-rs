//! `zombie.scripting.ScriptManager` and `ScriptBucketCollection`: finding
//! script files (game, then mods), splitting them into modules, and the
//! cross-module lookups the rest of the game uses.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_fs::GameFs;

use crate::base::{ScriptError, ScriptObject, ScriptRef, ScriptResult};
use crate::java::{self, JHashMap};
use crate::module::ScriptModule;
use crate::parser::{self, Block};
use crate::script_type::{Flag, ScriptLoadMode, ScriptType};

pub const BASE: &str = "Base";
pub const BASE_MODULE: &str = "Base.";
pub const VANILLA_ID: &str = "pz-vanilla";

/// `ScriptManager.bucketCollectionList` order after the constructor's
/// stable sort (templates first).
pub const COLLECTION_ORDER: &[ScriptType] = &[
    ScriptType::VehicleTemplate,
    ScriptType::EntityTemplate,
    ScriptType::Item,
    ScriptType::Recipe,
    ScriptType::UniqueRecipe,
    ScriptType::EvolvedRecipe,
    ScriptType::Fixing,
    ScriptType::AnimationMesh,
    ScriptType::Clock,
    ScriptType::Mannequin,
    ScriptType::Model,
    ScriptType::PhysicsShape,
    ScriptType::Sound,
    ScriptType::SoundTimeline,
    ScriptType::SpriteModel,
    ScriptType::Vehicle,
    ScriptType::RuntimeAnimation,
    ScriptType::VehicleEngineRPM,
    ScriptType::ItemConfig,
    ScriptType::Entity,
    ScriptType::XuiConfig,
    ScriptType::XuiLayout,
    ScriptType::XuiStyle,
    ScriptType::XuiDefaultStyle,
    ScriptType::XuiColor,
    ScriptType::XuiSkin,
    ScriptType::ItemFilter,
    ScriptType::FluidFilter,
    ScriptType::CraftRecipe,
    ScriptType::StringList,
    ScriptType::EnergyDefinition,
    ScriptType::FluidDefinition,
    ScriptType::TimedAction,
    ScriptType::Ragdoll,
    ScriptType::PhysicsHitReaction,
    ScriptType::CharacterTraitDefinition,
    ScriptType::CharacterProfessionDefinition,
];

/// Severity of a loader diagnostic (the original's `DebugLog` channels).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub level: LogLevel,
    pub message: String,
}

/// Engine systems the original `ScriptManager` calls into directly. Each
/// method names the Java call it stands for; they are no-ops until the
/// owning workstream provides an implementation.
pub trait ScriptHooks {
    /// `ScriptBucket.onScriptLoad` overrides (`GameSounds.OnReloadSound`).
    fn on_script_load(&self, _sm: &ScriptManager, _type: ScriptType, _mode: ScriptLoadMode, _script: &ScriptRef) {}
    /// `ItemTags.Init(getAllItems())` after items load.
    fn items_loaded(&self, _sm: &ScriptManager) {}
    /// `Energy.PreReloadScripts`, `Fluid.PreReloadScripts`.
    fn pre_reload_scripts(&self, _sm: &ScriptManager, _type: ScriptType) -> ScriptResult {
        Ok(())
    }
    /// `XuiManager.ParseScripts`, `CraftRecipeManager.Init`, `Energy.Init`,
    /// `Fluid.Init`, `RagdollScript.toBullet`.
    fn post_load_scripts(&self, _sm: &ScriptManager, _type: ScriptType, _mode: ScriptLoadMode) -> ScriptResult {
        Ok(())
    }
    /// `RecipeManager.LoadedAfterLua`.
    fn on_loaded_after_lua(&self, _sm: &ScriptManager, _type: ScriptType) -> ScriptResult {
        Ok(())
    }
    /// `SpriteConfigManager.InitScriptsPostTileDef`.
    fn on_post_tile_definitions(&self, _sm: &ScriptManager, _type: ScriptType) -> ScriptResult {
        Ok(())
    }
    /// The calls at the end of `ScriptManager.Load` (`WorldDictionary`,
    /// `RecipeManager`, `GameSounds`, `ModelScript`, `Translator` checks).
    fn scripts_loaded(&self, _sm: &ScriptManager) {}
}

struct NoHooks;
impl ScriptHooks for NoHooks {}

/// `ScriptBucketCollection`: one script type across every module.
pub struct ScriptBucketCollection {
    script_type: ScriptType,
    script_modules: RefCell<Vec<Rc<ScriptModule>>>,
    all_scripts: RefCell<Vec<ScriptRef>>,
    full_type_to_script_map: RefCell<JHashMap<ScriptRef>>,
    load_files: RefCell<Vec<String>>,
}

impl ScriptBucketCollection {
    fn new(script_type: ScriptType) -> Self {
        ScriptBucketCollection {
            script_type,
            script_modules: RefCell::default(),
            all_scripts: RefCell::default(),
            full_type_to_script_map: RefCell::default(),
            load_files: RefCell::default(),
        }
    }

    pub fn script_type(&self) -> ScriptType {
        self.script_type
    }

    pub fn is_template(&self) -> bool {
        self.script_type.is_template()
    }

    pub fn reset(&self) {
        self.script_modules.borrow_mut().clear();
        self.all_scripts.borrow_mut().clear();
        self.full_type_to_script_map.borrow_mut().clear();
        self.load_files.borrow_mut().clear();
    }

    pub fn has_full_type(&self, type_: &str) -> bool {
        self.full_type_to_script_map.borrow().contains_key(type_)
    }

    pub fn get_full_type(&self, type_: &str) -> Option<ScriptRef> {
        self.full_type_to_script_map.borrow().get(type_).cloned()
    }

    pub fn put_full_type(&self, script: &ScriptRef) -> ScriptResult {
        let ft = script.full_type()?;
        self.full_type_to_script_map.borrow_mut().insert(ft, script.clone());
        Ok(())
    }

    pub fn load_files(&self) -> Vec<String> {
        self.load_files.borrow().clone()
    }

    fn set_reload_buckets(&self, reload: bool) {
        for m in self.script_modules.borrow().iter() {
            m.bucket(self.script_type).set_reload(reload);
        }
    }

    fn register_module(&self, module: &Rc<ScriptModule>) {
        self.script_modules.borrow_mut().push(module.clone());
    }

    /// `getScript(name)`.
    pub fn get_script(&self, sm: &ScriptManager, name: &str) -> Option<ScriptRef> {
        if self.script_type.has_flag(Flag::CacheFullType) && name.contains('.') {
            if let Some(s) = self.get_full_type(name) {
                return Some(s);
            }
        }
        let module = if !name.contains('.') { sm.get_module(BASE) } else { sm.get_module(name) }?;
        module.bucket(self.script_type).get(sm, &module, ScriptManager::get_item_name(name))
    }

    /// `getAllScripts()`: every enabled module's scripts, from each
    /// bucket's map in `HashMap` order (or its list for `FromList` types).
    pub fn get_all_scripts(&self) -> Vec<ScriptRef> {
        if !self.script_type.has_flag(Flag::Clear) && !self.all_scripts.borrow().is_empty() {
            return self.all_scripts.borrow().clone();
        }
        let mut all = Vec::new();
        for m in self.script_modules.borrow().iter() {
            if m.is_disabled() {
                continue;
            }
            let b = m.bucket(self.script_type);
            if self.script_type.has_flag(Flag::FromList) {
                all.extend(b.script_list());
            } else {
                all.extend(b.script_map_values());
            }
        }
        self.on_sort_all_scripts(&mut all);
        *self.all_scripts.borrow_mut() = all.clone();
        all
    }

    fn on_sort_all_scripts(&self, scripts: &mut [ScriptRef]) {
        if self.script_type == ScriptType::Mannequin {
            // String.CASE_INSENSITIVE_ORDER on getName(); List.sort is stable.
            scripts.sort_by(|a, b| case_insensitive_cmp(&a.name(), &b.name()));
        }
    }

    pub fn clear_all_scripts_cache(&self) {
        self.all_scripts.borrow_mut().clear();
    }

    fn load_scripts(&self, sm: &ScriptManager, mode: ScriptLoadMode) -> ScriptResult {
        let modules = self.script_modules.borrow().clone();
        for m in modules {
            let bucket = m.bucket(self.script_type);
            bucket.load_scripts(sm, mode);
            {
                let mut lf = self.load_files.borrow_mut();
                for f in bucket.load_files() {
                    if !lf.contains(&f) {
                        lf.push(f);
                    }
                }
            }
            for script in bucket.script_list() {
                self.put_full_type(&script)?;
            }
        }
        if self.script_type == ScriptType::Item {
            sm.hooks().items_loaded(sm);
        }
        Ok(())
    }

    fn pre_reload_scripts(&self, sm: &ScriptManager) -> ScriptResult {
        sm.hooks().pre_reload_scripts(sm, self.script_type)?;
        sm.println(self.script_type, "<- PreReloadScripts ->");
        for s in self.get_all_scripts() {
            s.borrow_mut().pre_reload();
        }
        self.all_scripts.borrow_mut().clear();
        Ok(())
    }

    fn post_load_scripts(&self, sm: &ScriptManager, mode: ScriptLoadMode) -> ScriptResult {
        sm.hooks().post_load_scripts(sm, self.script_type, mode)
    }

    pub fn has_load_errors(&self, only_critical: bool) -> bool {
        self.script_modules.borrow().iter().any(|m| {
            m.bucket(self.script_type).has_load_errors() && !(only_critical && !is_critical(self.script_type))
        })
    }

    fn on_scripts_loaded(&self, sm: &ScriptManager, mode: ScriptLoadMode) -> ScriptResult {
        sm.println(self.script_type, "<- OnScriptsLoaded ->");
        for s in self.get_all_scripts() {
            s.borrow_mut().on_scripts_loaded(sm, mode)?;
        }
        Ok(())
    }

    fn on_loaded_after_lua(&self, sm: &ScriptManager) -> ScriptResult {
        sm.println(self.script_type, "<- OnLoadedAfterLua ->");
        for s in self.get_all_scripts() {
            s.borrow_mut().on_loaded_after_lua(sm)?;
        }
        sm.hooks().on_loaded_after_lua(sm, self.script_type)
    }

    fn on_post_tile_definitions(&self, sm: &ScriptManager) -> ScriptResult {
        sm.println(self.script_type, "<- OnPostTileDefinitions ->");
        sm.hooks().on_post_tile_definitions(sm, self.script_type)
    }

    fn on_post_world_dictionary_init(&self, sm: &ScriptManager) -> ScriptResult {
        sm.println(self.script_type, "<- OnPostWorldDictionaryInit ->");
        for s in self.get_all_scripts() {
            s.borrow_mut().on_post_world_dictionary_init(sm)?;
        }
        Ok(())
    }
}

/// `ScriptType.isCritical()` is always false in this build.
fn is_critical(_t: ScriptType) -> bool {
    false
}

/// `String.CASE_INSENSITIVE_ORDER`.
pub fn case_insensitive_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let fold = |c: u16| -> u16 {
        let up = char::from_u32(c as u32).map(|ch| {
            let mut u = ch.to_uppercase();
            match (u.next(), u.next()) {
                (Some(x), None) if (x as u32) <= 0xFFFF => x as u16,
                _ => c,
            }
        });
        let up = up.unwrap_or(c);
        char::from_u32(up as u32)
            .map(|ch| {
                let mut l = ch.to_lowercase();
                match (l.next(), l.next()) {
                    (Some(x), None) if (x as u32) <= 0xFFFF => x as u16,
                    _ => up,
                }
            })
            .unwrap_or(up)
    };
    a.encode_utf16().map(fold).cmp(b.encode_utf16().map(fold))
}

/// One mod's script roots, in `ZomboidFileSystem.getModIDs()` order: its
/// common folder and its versioned folder, either of which may be absent.
#[derive(Clone, Debug, Default)]
pub struct ModScriptDirs {
    pub id: String,
    pub common_dir: Option<PathBuf>,
    pub version_dir: Option<PathBuf>,
}

pub struct ScriptManager {
    /// `Core.debug`.
    pub debug: bool,
    current_file_name: RefCell<Option<String>>,
    load_file_names: RefCell<Vec<String>>,
    module_map: RefCell<JHashMap<Rc<ScriptModule>>>,
    module_list: RefCell<Vec<Rc<ScriptModule>>>,
    current_loading_module: RefCell<Option<Rc<ScriptModule>>>,
    module_aliases: RefCell<HashMap<String, String>>,
    cached_modules: RefCell<HashMap<String, Rc<ScriptModule>>>,
    collections: Vec<ScriptBucketCollection>,
    has_load_errors: Cell<bool>,
    checksum: RefCell<String>,
    temp_file_to_mod_map: RefCell<HashMap<String, String>>,
    /// Game, then mod roots, each indexed once (`activeFileMap` sources).
    roots: RefCell<Vec<GameFs>>,
    current_load_file_mod: RefCell<Option<String>>,
    current_load_file_abs_path: RefCell<Option<String>>,
    current_load_file_name: RefCell<Option<String>>,
    current_script_object: RefCell<Option<String>>,
    debug_types: RefCell<HashSet<ScriptType>>,
    hooks: RefCell<Rc<dyn ScriptHooks>>,
    log: RefCell<Vec<LogLine>>,
    /// Per-type state owned by individual script classes (Java statics).
    pub statics: crate::objects::Statics,
    /// `Registries` (the original's static registries).
    pub registries: crate::registry::Registries,
    /// Built-in name tables (perks, ragdoll bones, vehicle enums).
    pub builtin_tables: RefCell<crate::registry::builtin::BuiltinTables>,
}

impl Default for ScriptManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptManager {
    pub fn new() -> Self {
        ScriptManager {
            debug: false,
            current_file_name: RefCell::default(),
            load_file_names: RefCell::default(),
            module_map: RefCell::default(),
            module_list: RefCell::default(),
            current_loading_module: RefCell::default(),
            module_aliases: RefCell::default(),
            cached_modules: RefCell::default(),
            collections: COLLECTION_ORDER.iter().map(|&t| ScriptBucketCollection::new(t)).collect(),
            has_load_errors: Cell::new(false),
            checksum: RefCell::default(),
            temp_file_to_mod_map: RefCell::default(),
            roots: RefCell::default(),
            current_load_file_mod: RefCell::default(),
            current_load_file_abs_path: RefCell::default(),
            current_load_file_name: RefCell::default(),
            current_script_object: RefCell::default(),
            debug_types: RefCell::default(),
            hooks: RefCell::new(Rc::new(NoHooks)),
            log: RefCell::default(),
            statics: Default::default(),
            registries: Default::default(),
            builtin_tables: Default::default(),
        }
    }

    /// Registers the game's built-in registry keys, read from the installed
    /// `projectzomboid.jar` (the `Registries` bootstrap).
    pub fn load_builtin_registries(&self, jar_path: &Path) -> ScriptResult {
        let jar = pz_classfile::Jar::open(jar_path).map_err(|e| ScriptError::new(format!("{}: {e}", jar_path.display())))?;
        crate::registry::builtin::register_builtin(&self.registries, &jar)?;
        *self.builtin_tables.borrow_mut() = crate::registry::builtin::BuiltinTables::load(&jar)?;
        crate::objects::items::load_builtin_constants(self, jar_path)
    }

    pub fn set_hooks(&self, hooks: Rc<dyn ScriptHooks>) {
        *self.hooks.borrow_mut() = hooks;
    }

    pub fn hooks(&self) -> Rc<dyn ScriptHooks> {
        self.hooks.borrow().clone()
    }

    // ---- diagnostics -------------------------------------------------

    fn push_log(&self, level: LogLevel, message: String) {
        self.log.borrow_mut().push(LogLine { level, message });
    }

    /// Everything the loader logged, oldest first.
    pub fn take_log(&self) -> Vec<LogLine> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    /// `DebugLog.log`.
    pub fn log(&self, msg: &str) {
        self.push_log(LogLevel::Info, msg.to_string());
    }

    /// `DebugType.General.warn`.
    pub fn warn(&self, msg: &str) {
        self.push_log(LogLevel::Warn, msg.to_string());
    }

    /// `DebugType.General.debugln`.
    pub fn debugln(&self, msg: &str) {
        self.push_log(LogLevel::Debug, msg.to_string());
    }

    /// `DebugType.Script.debugln`.
    pub fn script_debugln(&self, msg: &str) {
        self.push_log(LogLevel::Debug, msg.to_string());
    }

    /// `DebugType.Script.warn`.
    pub fn script_warn(&self, msg: &str) {
        self.push_log(LogLevel::Warn, msg.to_string());
    }

    /// `DebugType.Script.error`.
    pub fn script_error(&self, msg: &str) {
        self.push_log(LogLevel::Error, msg.to_string());
    }

    /// `ExceptionLogger.logException`.
    pub fn log_exception(&self, e: &ScriptError) {
        self.push_log(LogLevel::Error, e.0.clone());
    }

    pub fn enable_debug(&self, type_: ScriptType, enable: bool) {
        let mut d = self.debug_types.borrow_mut();
        if enable {
            d.insert(type_);
        } else {
            d.remove(&type_);
        }
    }

    pub fn is_debug_enabled(&self, type_: ScriptType) -> bool {
        self.debug_types.borrow().contains(&type_)
    }

    /// `ScriptManager.println(type, msg)`: only for types with debug on.
    pub fn println(&self, type_: ScriptType, msg: &str) {
        if self.is_debug_enabled(type_) {
            self.push_log(LogLevel::Debug, format!("[{}] {msg}", type_.name()));
        }
    }

    // ---- load state ----------------------------------------------------

    pub fn current_file_name(&self) -> Option<String> {
        self.current_file_name.borrow().clone()
    }

    pub fn current_load_file_mod(&self) -> Option<String> {
        self.current_load_file_mod.borrow().clone()
    }

    pub fn current_load_file_abs_path(&self) -> Option<String> {
        self.current_load_file_abs_path.borrow().clone()
    }

    pub fn current_load_file_name(&self) -> Option<String> {
        self.current_load_file_name.borrow().clone()
    }

    pub fn current_loading_module(&self) -> Option<Rc<ScriptModule>> {
        self.current_loading_module.borrow().clone()
    }

    pub(crate) fn set_current_loading_module(&self, m: Option<Rc<ScriptModule>>) {
        *self.current_loading_module.borrow_mut() = m;
    }

    /// `ScriptBucket.getCurrentScriptObject()`.
    pub fn current_script_object(&self) -> Option<String> {
        self.current_script_object.borrow().clone()
    }

    pub(crate) fn set_current_script_object(&self, s: Option<String>) {
        *self.current_script_object.borrow_mut() = s;
    }

    /// `ScriptParser.parse(body)` as called from a script's `Load`, which
    /// tags nested blocks with the script being loaded.
    pub fn parse(&self, body: &str) -> Block {
        parser::parse_with(body, self.current_script_object.borrow().as_deref())
    }

    /// `ScriptBucket.onScriptLoad` overrides: `VehicleScript.Loaded()` for
    /// vehicles, then the engine hook.
    pub(crate) fn on_script_load(&self, type_: ScriptType, mode: ScriptLoadMode, script: &ScriptRef) -> ScriptResult {
        if type_ == ScriptType::Vehicle {
            crate::objects::vehicles::on_script_load(self, mode, script)?;
        }
        self.hooks().on_script_load(self, type_, mode, script);
        Ok(())
    }

    pub fn checksum(&self) -> String {
        self.checksum.borrow().clone()
    }

    // ---- modules -------------------------------------------------------

    pub fn collection(&self, type_: ScriptType) -> &ScriptBucketCollection {
        self.collections
            .iter()
            .find(|c| c.script_type() == type_)
            .unwrap_or_else(|| panic!("no collection for {}", type_.name()))
    }

    pub fn collections(&self) -> &[ScriptBucketCollection] {
        &self.collections
    }

    pub fn module_list(&self) -> Vec<Rc<ScriptModule>> {
        self.module_list.borrow().clone()
    }

    pub fn add_module_alias(&self, alias: &str, module: &str) {
        self.module_aliases.borrow_mut().insert(alias.to_string(), module.to_string());
    }

    /// `getItemName`: the part after the first '.'.
    pub fn get_item_name(name: &str) -> &str {
        match name.find('.') {
            Some(p) => &name[p + 1..],
            None => name,
        }
    }

    /// `getModule(name)`: defaults to `Base` when nothing matches.
    pub fn get_module(&self, name: &str) -> Option<Rc<ScriptModule>> {
        self.get_module_default(name, true)
    }

    /// `getModule(name, defaultToBase)`.
    pub fn get_module_default(&self, name: &str, default_to_base: bool) -> Option<Rc<ScriptModule>> {
        if java::trim(name) == BASE || name.starts_with(BASE_MODULE) {
            return self.module_map.borrow().get(BASE).cloned();
        }
        if let Some(m) = self.cached_modules.borrow().get(name) {
            return Some(m.clone());
        }
        let mut name = name.to_string();
        if let Some(a) = self.module_aliases.borrow().get(&name) {
            name = a.clone();
        }
        if let Some(m) = self.cached_modules.borrow().get(&name) {
            return Some(m.clone());
        }
        let mut ret = None;
        if let Some(m) = self.module_map.borrow().get(&name) {
            ret = if m.is_disabled() { None } else { Some(m.clone()) };
        }
        if let Some(m) = ret {
            self.cached_modules.borrow_mut().insert(name, m.clone());
            return Some(m);
        }
        if let Some(idx) = name.find('.') {
            ret = self.get_module(&name[..idx]);
        }
        if let Some(m) = ret {
            self.cached_modules.borrow_mut().insert(name, m.clone());
            return Some(m);
        }
        if default_to_base {
            self.module_map.borrow().get(BASE).cloned()
        } else {
            None
        }
    }

    /// `getModuleNoDisableCheck`.
    pub fn get_module_no_disable_check(&self, name: &str) -> Option<Rc<ScriptModule>> {
        let mut name = name.to_string();
        if let Some(a) = self.module_aliases.borrow().get(&name) {
            name = a.clone();
        }
        if let Some(m) = self.module_map.borrow().get(&name) {
            return Some(m.clone());
        }
        if name.contains('.') {
            let first = java::split(&name, |c| c == '.', false);
            return self.get_module(first.first().copied().unwrap_or(""));
        }
        None
    }

    // ---- loading -------------------------------------------------------

    /// `LoadFile(loadMode, filename, bLoadJar)`. `filename` is a lower-case
    /// path relative to the game (or mod) folder.
    pub fn load_file(&self, mode: ScriptLoadMode, filename: &str, load_jar: bool) -> ScriptResult {
        self.script_debugln(&format!("{filename}{}", if load_jar { " bLoadJar" } else { "" }));
        if filename.contains(".tmx") {
            // IsoWorld.mapPath/mapUseJar: legacy map files are not scripts.
            let map_path = &filename[..filename.rfind('/').unwrap_or(0)];
            self.script_debugln(&format!(
                "  file is a .tmx (map) file. Set mapPath to {map_path}{}",
                if load_jar { " mapUseJar" } else { "" }
            ));
            return Ok(());
        }
        if !filename.ends_with(".txt") {
            self.script_warn(&format!(" file is not a .txt (script) file: {filename}"));
            return Ok(());
        }
        let path = self.resolve_active_file(filename).ok_or_else(|| {
            ScriptError::new(format!("java.io.FileNotFoundException: {filename} (No such file or directory)"))
        })?;
        let bytes = std::fs::read(&path).map_err(|e| ScriptError::new(format!("java.io.FileNotFoundException: {e}")))?;
        // BufferedReader.readLine over UTF-8 (malformed input replaced),
        // each line re-terminated with '\n'.
        let text = String::from_utf8_lossy(&bytes);
        let mut buf = String::with_capacity(text.len() + 1);
        for line in read_lines(&text) {
            buf.push_str(line);
            buf.push('\n');
        }
        let total_file = parser::strip_comments(&buf);
        *self.current_file_name.borrow_mut() = Some(filename.to_string());
        self.register_load_file_name(filename);
        let r = self.parse_script(mode, &total_file);
        *self.current_file_name.borrow_mut() = None;
        r
    }

    fn register_load_file_name(&self, name: &str) {
        let mut l = self.load_file_names.borrow_mut();
        if !l.iter().any(|n| n == name) {
            l.push(name.to_string());
        }
    }

    /// `ParseScript`: each top-level token of a file.
    pub fn parse_script(&self, mode: ScriptLoadMode, total_file: &str) -> ScriptResult {
        for token in parser::parse_tokens(total_file) {
            self.create_from_token(mode, &token)?;
        }
        Ok(())
    }

    fn create_from_token(&self, mode: ScriptLoadMode, token: &str) -> ScriptResult {
        let token = java::trim(token);
        if !token.starts_with("module") {
            return Ok(());
        }
        let first_open = java::index_of(token, "{", 0);
        let last_close = java::last_index_of_all(token, "}");
        let waypoint = java::split(token, |c| c == '{' || c == '}', false);
        let name = java::trim(&waypoint.first().copied().unwrap_or("").replace("module", "")).to_string();
        if first_open + 1 > last_close || last_close < 0 {
            return Err(ScriptError::new(format!(
                "java.lang.StringIndexOutOfBoundsException: begin {}, end {}, length {}",
                first_open + 1,
                last_close,
                java::utf16_len(token)
            )));
        }
        let actual = &token[(first_open + 1) as usize..last_close as usize];
        let existing = self.module_map.borrow().get(&name).cloned();
        let module = match existing {
            Some(m) => m,
            None => {
                self.script_debugln(&format!("Adding new module: {name}"));
                let m = ScriptModule::new();
                self.module_map.borrow_mut().insert(name.clone(), m.clone());
                self.module_list.borrow_mut().push(m.clone());
                for c in &self.collections {
                    c.register_module(&m);
                }
                m
            }
        };
        module.load(self, mode, &name, actual)
    }

    /// `searchFolders`: every `.txt` file below `dir`, as a lower-cased path
    /// relative to `base`.
    fn search_folders(base: &Path, dir: &Path, out: &mut Vec<String>) {
        if dir.is_dir() {
            if dir.to_string_lossy().contains("tempNotWorking") {
                return;
            }
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                Self::search_folders(base, &e.path(), out);
            }
        } else if dir.to_string_lossy().to_lowercase().ends_with(".txt") {
            let abs = dir.to_string_lossy().to_lowercase();
            let base_l = base.to_string_lossy().to_lowercase();
            let rel = abs.strip_prefix(&base_l).map(|r| r.trim_start_matches(['/', '\\'])).unwrap_or(&abs);
            out.push(rel.replace('\\', "/"));
        }
    }

    /// `ZomboidFileSystem.getString`: the active file for a relative path,
    /// where later roots (mods, then their versioned folders) override.
    fn resolve_active_file(&self, rel: &str) -> Option<PathBuf> {
        self.roots.borrow().iter().rev().find_map(|root| {
            let p = root.resolve(rel)?;
            p.is_file().then_some(p)
        })
    }

    /// Whether `rel` (a lower-case game path) resolves to a file in the game
    /// or an active mod (`ZomboidFileSystem.activeFileMap`).
    pub fn active_file_exists(&self, rel: &str) -> bool {
        self.resolve_active_file(rel).is_some()
    }

    /// `ScriptManager.Load()`: reads every script file of the game and the
    /// given mods (in load order), then loads all script types.
    pub fn load(&self, fs: &GameFs, mods: &[ModScriptDirs]) -> ScriptResult {
        self.load_file_names.borrow_mut().clear();
        self.temp_file_to_mod_map.borrow_mut().clear();
        let mut roots = vec![fs.base.clone()];
        for m in mods {
            roots.extend(m.common_dir.iter().cloned());
            roots.extend(m.version_dir.iter().cloned());
        }
        *self.roots.borrow_mut() = roots.into_iter().map(GameFs::new).collect();
        if let Err(e) = self.load_files(fs, mods) {
            self.log_exception(&e);
        }
        self.load_scripts(ScriptLoadMode::Init, ScriptType::ALL);
        if self.debug && self.has_load_errors(false) {
            return Err(ScriptError::new("java.io.IOException: Script load errors."));
        }
        crate::objects::post_load(self);
        self.hooks().scripts_loaded(self);
        Ok(())
    }

    fn load_files(&self, fs: &GameFs, mods: &[ModScriptDirs]) -> ScriptResult {
        let mut game_files = Vec::new();
        let scripts_dir = fs.resolve("media/scripts").unwrap_or_else(|| fs.base.join("media").join("scripts"));
        Self::search_folders(&fs.base, &scripts_dir, &mut game_files);
        for f in &game_files {
            if let Some(abs) = self.resolve_active_file(f) {
                self.temp_file_to_mod_map.borrow_mut().insert(abs.to_string_lossy().into_owned(), VANILLA_ID.to_string());
            }
        }
        let mut mod_files = Vec::new();
        for m in mods {
            if m.id == VANILLA_ID {
                return Err(ScriptError::new("java.lang.RuntimeException: Warning mod id is named pz-vanilla!"));
            }
            for dir in [&m.common_dir, &m.version_dir].into_iter().flatten() {
                let first = mod_files.len();
                let scripts = GameFs::new(dir.clone())
                    .resolve("media/scripts")
                    .unwrap_or_else(|| dir.join("media").join("scripts"));
                Self::search_folders(dir, &scripts, &mut mod_files);
                for f in &mod_files[first..] {
                    if let Some(abs) = self.resolve_active_file(f) {
                        self.temp_file_to_mod_map.borrow_mut().insert(abs.to_string_lossy().into_owned(), m.id.clone());
                    }
                }
            }
        }
        game_files.sort_by(|a, b| script_file_cmp(a, b));
        mod_files.sort_by(|a, b| script_file_cmp(a, b));
        game_files.extend(mod_files);
        let mut done = HashSet::new();
        for s in game_files {
            if !done.insert(s.clone()) {
                continue;
            }
            let abs = self.resolve_active_file(&s).map(|p| p.to_string_lossy().into_owned());
            *self.current_load_file_name.borrow_mut() = abs.as_deref().map(file_name);
            *self.current_load_file_mod.borrow_mut() =
                abs.as_ref().and_then(|a| self.temp_file_to_mod_map.borrow().get(a).cloned());
            *self.current_load_file_abs_path.borrow_mut() = abs;
            self.load_file(ScriptLoadMode::Init, &s, false)?;
        }
        Ok(())
    }

    /// `ReloadScripts(types)`.
    pub fn reload_scripts(&self, types: &[ScriptType]) {
        self.debugln(&format!("Reloading scripts = {types:?}"));
        self.load_scripts(ScriptLoadMode::Reload, types);
    }

    fn load_scripts(&self, mode: ScriptLoadMode, types: &[ScriptType]) {
        if let Err(e) = self.load_scripts_inner(mode, types) {
            self.log_exception(&e);
            self.has_load_errors.set(true);
        }
    }

    fn load_scripts_inner(&self, mode: ScriptLoadMode, types: &[ScriptType]) -> ScriptResult {
        let selected = |c: &&ScriptBucketCollection| types.contains(&c.script_type());
        if mode == ScriptLoadMode::Reload {
            for c in self.collections.iter().filter(selected) {
                c.set_reload_buckets(true);
                c.pre_reload_scripts(self)?;
            }
            let names = self.load_file_names.borrow().clone();
            for name in names {
                let abs = self.resolve_active_file(&name).map(|p| p.to_string_lossy().into_owned());
                *self.current_load_file_name.borrow_mut() = abs.as_deref().map(file_name);
                *self.current_load_file_mod.borrow_mut() =
                    abs.as_ref().and_then(|a| self.temp_file_to_mod_map.borrow().get(a).cloned());
                *self.current_load_file_abs_path.borrow_mut() = abs;
                self.load_file(ScriptLoadMode::Reload, &name, true)?;
            }
            for c in &self.collections {
                c.set_reload_buckets(false);
            }
        }
        for c in self.collections.iter().filter(selected) {
            c.load_scripts(self, mode)?;
        }
        for c in self.collections.iter().filter(selected) {
            c.post_load_scripts(self, mode)?;
        }
        for c in self.collections.iter().filter(selected) {
            c.on_scripts_loaded(self, mode)?;
        }
        if mode == ScriptLoadMode::Reload {
            for c in self.collections.iter().filter(selected) {
                c.on_loaded_after_lua(self)?;
                c.on_post_tile_definitions(self)?;
                c.on_post_world_dictionary_init(self)?;
            }
        }
        Ok(())
    }

    /// `LoadedAfterLua`.
    pub fn loaded_after_lua(&self) {
        for c in &self.collections {
            if let Err(e) = c.on_loaded_after_lua(self) {
                self.log_exception(&e);
                self.has_load_errors.set(true);
            }
        }
    }

    /// `PostTileDefinitions`.
    pub fn post_tile_definitions(&self) {
        for c in &self.collections {
            if let Err(e) = c.on_post_tile_definitions(self) {
                self.log_exception(&e);
                self.has_load_errors.set(true);
            }
        }
    }

    /// `PostWorldDictionaryInit`.
    pub fn post_world_dictionary_init(&self) {
        for c in &self.collections {
            if let Err(e) = c.on_post_world_dictionary_init(self) {
                self.log_exception(&e);
                self.has_load_errors.set(true);
            }
        }
    }

    pub fn has_load_errors(&self, only_critical: bool) -> bool {
        self.has_load_errors.get() || self.collections.iter().any(|c| c.has_load_errors(only_critical))
    }

    /// `Reset`.
    pub fn reset(&self) {
        for m in self.module_list.borrow().iter() {
            m.reset();
        }
        for c in &self.collections {
            c.reset();
        }
        self.module_map.borrow_mut().clear();
        self.module_list.borrow_mut().clear();
        self.module_aliases.borrow_mut().clear();
        self.cached_modules.borrow_mut().clear();
        self.has_load_errors.set(false);
        crate::objects::reset_statics(self);
    }

    // ---- generic lookups ---------------------------------------------

    /// `getScriptsForType`: all scripts of a type (empty for
    /// `EntityComponent`, which has no collection).
    pub fn get_scripts_for_type(&self, type_: ScriptType) -> Vec<ScriptRef> {
        match self.collections.iter().find(|c| c.script_type() == type_) {
            Some(c) => c.get_all_scripts(),
            None => {
                self.warn(&format!("Type has no bucket collection: {}", type_.name()));
                Vec::new()
            }
        }
    }

    /// The per-type `getX(name)` lookups (`getItem`, `getModelScript`, ...).
    pub fn get_script(&self, type_: ScriptType, name: &str) -> Option<ScriptRef> {
        self.collection(type_).get_script(self, name)
    }

    /// Typed form of [`get_script`](Self::get_script).
    pub fn get<T: ScriptObject>(&self, type_: ScriptType, name: &str) -> Option<Rc<RefCell<T>>> {
        self.get_script(type_, name)?.downcast::<T>()
    }

    /// The per-type `getAllX()` lists.
    pub fn get_all<T: ScriptObject>(&self, type_: ScriptType) -> Vec<Rc<RefCell<T>>> {
        self.collection(type_).get_all_scripts().iter().filter_map(|s| s.downcast::<T>()).collect()
    }

    /// `addModelScript` / `addSpriteModel`: registers a script created at
    /// runtime with its module and the full-type cache.
    pub fn add_runtime_script(&self, script: ScriptRef) -> ScriptResult {
        let (module, type_) = {
            let s = script.borrow();
            (s.base().module(), s.base().script_object_type())
        };
        let module = module.ok_or_else(|| ScriptError::new("java.lang.NullPointerException: module"))?;
        module.bucket(type_).add_script(script.clone());
        let c = self.collection(type_);
        if type_ == ScriptType::Model {
            c.clear_all_scripts_cache();
        }
        c.put_full_type(&script)
    }

    /// `FindItem(name, moduleDefaultsToBase)`. The original's fallback loop
    /// over all modules re-queries the first module, so it never finds more.
    pub fn find_item(&self, name: &str, module_defaults_to_base: bool) -> Option<ScriptRef> {
        let items = self.collection(ScriptType::Item);
        if name.contains('.') && items.has_full_type(name) {
            return items.get_full_type(name);
        }
        let module = self.get_module_default(name, module_defaults_to_base)?;
        module.get(self, ScriptType::Item, Self::get_item_name(name))
    }

    /// `getSpecificItem` / `getSpecificEntity`: requires `Module.Type`.
    pub fn get_specific(&self, type_: ScriptType, name: &str) -> Option<ScriptRef> {
        if !name.contains('.') {
            let what = if type_ == ScriptType::Entity { "getSpecificEntity" } else { "getSpecificItem" };
            self.log(&format!("ScriptManager.{what} requires a full type name, cannot find: {name}"));
            return None;
        }
        let c = self.collection(type_);
        if c.has_full_type(name) {
            return c.get_full_type(name);
        }
        let idx = name.find('.').unwrap();
        let module = self.get_module_default(&name[..idx], false)?;
        module.get(self, type_, &name[idx + 1..])
    }

    /// `resolveItemType`: `Module.Type` for a bare item type, searching the
    /// given module, then every enabled module; `???.Type` when unknown.
    pub fn resolve_item_type(&self, module: &ScriptModule, item_type: &str) -> Option<String> {
        self.resolve_type(ScriptType::Item, module, item_type, false)
    }

    /// `resolveModelScript`.
    pub fn resolve_model_script(&self, module: &ScriptModule, name: &str) -> Option<String> {
        self.resolve_type(ScriptType::Model, module, name, true)
    }

    fn resolve_type(&self, type_: ScriptType, module: &ScriptModule, t: &str, skip_self: bool) -> Option<String> {
        if java::is_null_or_whitespace(Some(t)) {
            return None;
        }
        if t.contains('.') {
            return Some(t.to_string());
        }
        // ModelScript.getFullType uses the model's own `name` field.
        let full_type = |s: ScriptRef| match s.downcast::<crate::objects::models::ModelScript>() {
            Some(m) => Some(m.borrow().full_type_name()),
            None => s.full_type().ok(),
        };
        if let Some(s) = module.get(self, type_, t) {
            return full_type(s);
        }
        for m in self.module_list() {
            if (skip_self && std::ptr::eq(&*m, module)) || m.is_disabled() {
                continue;
            }
            if let Some(s) = m.get(self, type_, t) {
                return full_type(s);
            }
        }
        Some(format!("???.{t}"))
    }

    /// `getItemsByType`: the item named `type_` in every enabled module.
    pub fn get_items_by_type(&self, type_: &str) -> ScriptResult<Vec<ScriptRef>> {
        if java::is_null_or_whitespace(Some(type_)) {
            return Err(ScriptError::new(format!("java.lang.IllegalArgumentException: invalid type \"{type_}\"")));
        }
        let items = self.collection(ScriptType::Item);
        let mut out = Vec::new();
        for m in self.module_list() {
            if m.is_disabled() {
                continue;
            }
            let ft = if type_.contains('.') { type_.to_string() } else { format!("{}.{type_}", m.name()) };
            if let Some(i) = items.get_full_type(&ft) {
                out.push(i);
            }
        }
        Ok(out)
    }
}

/// `BufferedReader.readLine` line splitting: `\n`, `\r` or `\r\n`.
fn read_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\n' => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            b'\r' => {
                out.push(&text[start..i]);
                if i + 1 < b.len() && b[i + 1] == b'\n' {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < b.len() {
        out.push(&text[start..]);
    }
    out
}

/// `FileName.getName`.
pub fn file_name(path: &str) -> String {
    let i = path.rfind(['/', '\\']).map_or(0, |p| p + 1);
    path[i..].to_string()
}

/// The comparator in `ScriptManager.Load`: `template_*` files first, then
/// `String.compareTo` on the relative path.
fn script_file_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let t1 = file_name(a).starts_with("template_");
    let t2 = file_name(b).starts_with("template_");
    match (t1, t2) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => java::compare(a, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_lines_like_buffered_reader() {
        assert_eq!(read_lines("a\r\nb\rc\n\nd"), vec!["a", "b", "c", "", "d"]);
        assert_eq!(read_lines("a\n"), vec!["a"]);
        assert!(read_lines("").is_empty());
    }

    #[test]
    fn template_files_sort_first() {
        let mut v = vec!["media/scripts/b.txt", "media/scripts/z/template_x.txt", "media/scripts/a.txt"];
        v.sort_by(|a, b| script_file_cmp(a, b));
        assert_eq!(v, vec!["media/scripts/z/template_x.txt", "media/scripts/a.txt", "media/scripts/b.txt"]);
    }

    #[test]
    fn collection_order_is_complete() {
        assert_eq!(COLLECTION_ORDER.len(), ScriptType::ALL.len() - 1);
    }
}
