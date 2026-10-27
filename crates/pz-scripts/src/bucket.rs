//! `zombie.scripting.ScriptBucket` (and `ScriptBucket.Template`): the
//! per-module, per-type store that collects script bodies while files are
//! parsed and turns them into loaded script objects.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use crate::base::{ScriptError, ScriptRef, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::objects;
use crate::script_type::{Flag, ScriptLoadMode, ScriptType};

/// Guards the import walk in `get`, which recurses without bound in the
/// original (a module import cycle overflows the Java stack).
const MAX_IMPORT_DEPTH: usize = 64;

struct LoadData {
    name: String,
    script_bodies: Vec<String>,
    script: ScriptRef,
    reloaded: bool,
    added_on_reload: bool,
}

impl LoadData {
    fn new(script: ScriptRef) -> Self {
        LoadData {
            name: script.name(),
            script_bodies: Vec::new(),
            script,
            reloaded: false,
            added_on_reload: false,
        }
    }
}

/// A bucket's `scriptMap`: a `HashMap`, or for `RuntimeAnimation` a
/// `TreeMap(String.CASE_INSENSITIVE_ORDER)` (sorted, case-insensitive keys).
enum ScriptMap {
    Hash(JHashMap<ScriptRef>),
    CaseInsensitive(Vec<(String, ScriptRef)>),
}

impl Default for ScriptMap {
    fn default() -> Self {
        ScriptMap::Hash(JHashMap::new())
    }
}

impl ScriptMap {
    fn ci_search(v: &[(String, ScriptRef)], key: &str) -> Result<usize, usize> {
        v.binary_search_by(|(k, _)| crate::manager::case_insensitive_cmp(k, key))
    }

    fn get(&self, key: &str) -> Option<&ScriptRef> {
        match self {
            ScriptMap::Hash(m) => m.get(key),
            ScriptMap::CaseInsensitive(v) => Self::ci_search(v, key).ok().map(|i| &v[i].1),
        }
    }

    /// `put`: TreeMap keeps the first key's spelling and replaces the value.
    fn insert(&mut self, key: String, value: ScriptRef) {
        match self {
            ScriptMap::Hash(m) => {
                m.insert(key, value);
            }
            ScriptMap::CaseInsensitive(v) => match Self::ci_search(v, &key) {
                Ok(i) => v[i].1 = value,
                Err(i) => v.insert(i, (key, value)),
            },
        }
    }

    fn values(&self) -> Vec<ScriptRef> {
        match self {
            ScriptMap::Hash(m) => m.values().cloned().collect(),
            ScriptMap::CaseInsensitive(v) => v.iter().map(|(_, s)| s.clone()).collect(),
        }
    }

    fn clear(&mut self) {
        match self {
            ScriptMap::Hash(m) => m.clear(),
            ScriptMap::CaseInsensitive(v) => v.clear(),
        }
    }
}

#[derive(Default)]
struct Inner {
    load_data: JHashMap<LoadData>,
    load_files: Vec<String>,
    dot_in_name: HashSet<String>,
    script_list: Vec<ScriptRef>,
    script_map: ScriptMap,
    reload: bool,
    has_load_errors: bool,
}

pub struct ScriptBucket {
    script_type: ScriptType,
    verbose: Cell<bool>,
    inner: RefCell<Inner>,
}

impl ScriptBucket {
    pub fn new(script_type: ScriptType) -> Self {
        let mut inner = Inner::default();
        if script_type == ScriptType::RuntimeAnimation {
            inner.script_map = ScriptMap::CaseInsensitive(Vec::new());
        }
        ScriptBucket { script_type, verbose: Cell::new(false), inner: RefCell::new(inner) }
    }

    pub fn script_type(&self) -> ScriptType {
        self.script_type
    }

    /// `ScriptBucket.Template` subclasses.
    pub fn is_template(&self) -> bool {
        self.script_type.is_template()
    }

    pub(crate) fn set_reload(&self, reload: bool) {
        self.inner.borrow_mut().reload = reload;
    }

    pub fn is_verbose(&self) -> bool {
        self.verbose.get() || self.script_type.default_verbose()
    }

    pub fn set_verbose(&self, b: bool) {
        self.verbose.set(b);
    }

    pub fn has_load_errors(&self) -> bool {
        self.inner.borrow().has_load_errors
    }

    pub(crate) fn set_load_error(&self) {
        self.inner.borrow_mut().has_load_errors = true;
    }

    /// `getScriptList`: scripts in load order.
    pub fn script_list(&self) -> Vec<ScriptRef> {
        self.inner.borrow().script_list.clone()
    }

    /// `getScriptMap().values()` in `HashMap` order.
    pub fn script_map_values(&self) -> Vec<ScriptRef> {
        self.inner.borrow().script_map.values()
    }

    /// Definitions encountered during script loading, including obsolete
    /// or disabled scripts omitted from the runtime map. WorldDictionary
    /// receives its callbacks before those filters run.
    pub fn loaded_definitions(&self) -> Vec<ScriptRef> {
        self.inner.borrow().load_data.values().map(|data| data.script.clone()).collect()
    }

    pub fn script_map_get(&self, name: &str) -> Option<ScriptRef> {
        self.inner.borrow().script_map.get(name).cloned()
    }

    pub fn load_files(&self) -> Vec<String> {
        self.inner.borrow().load_files.clone()
    }

    pub fn reset(&self) {
        let mut i = self.inner.borrow_mut();
        i.load_data.clear();
        i.load_files.clear();
        i.dot_in_name.clear();
        i.script_list.clear();
        i.script_map.clear();
    }

    /// Adds a script directly (`ScriptManager.addModelScript` and friends).
    pub fn add_script(&self, script: ScriptRef) {
        let name = script.name();
        let mut i = self.inner.borrow_mut();
        i.script_list.push(script.clone());
        if name.contains('.') {
            i.dot_in_name.insert(name.clone());
        }
        i.script_map.insert(name, script);
    }

    /// `CreateFromTokenPP`: claims `token` if its header type is this
    /// bucket's tag. Returns whether it was claimed (false after an error).
    pub fn create_from_token_pp(
        &self,
        module: &Rc<ScriptModule>,
        sm: &ScriptManager,
        mode: ScriptLoadMode,
        type_: &str,
        token: &str,
    ) -> bool {
        let r = if self.is_template() {
            self.create_template_from_token(module, sm, type_, token)
        } else {
            self.create_from_token(module, sm, mode, type_, token)
        };
        match r {
            Ok(claimed) => claimed,
            Err(e) => {
                sm.log_exception(&e);
                self.set_load_error();
                false
            }
        }
    }

    fn new_script(&self, module: &Rc<ScriptModule>, sm: &ScriptManager, name: &str, token: &str) -> ScriptRef {
        let script = objects::create_instance(self.script_type, module, name, token);
        {
            let mut s = script.borrow_mut();
            s.set_module(module);
            s.init_load_pp(sm, name);
        }
        script
    }

    fn create_from_token(
        &self,
        module: &Rc<ScriptModule>,
        sm: &ScriptManager,
        mode: ScriptLoadMode,
        type_: &str,
        token: &str,
    ) -> ScriptResult<bool> {
        let tag = self.script_type.script_tag();
        if tag != type_ {
            return Ok(false);
        }
        let waypoint = java::split(token, |c| c == '{' || c == '}', false);
        let name = java::trim(&waypoint.first().copied().unwrap_or("").replace(tag, "")).to_string();
        let current_file = sm.current_file_name();
        if mode == ScriptLoadMode::Init {
            let mut i = self.inner.borrow_mut();
            if let Some(f) = &current_file {
                if !i.load_files.contains(f) {
                    i.load_files.push(f.clone());
                }
            }
        }
        let mod_id = sm.current_load_file_mod();
        let existing = self.inner.borrow().load_data.get(&name).map(|d| d.script.clone());
        if let Some(script) = existing {
            if mode == ScriptLoadMode::Init {
                script.borrow_mut().init_load_pp(sm, &name);
                let mut i = self.inner.borrow_mut();
                let data = i.load_data.get_mut(&name).unwrap();
                data.reloaded = false;
                data.script_bodies.push(token.to_string());
                drop(i);
                script.borrow_mut().base_mut().add_loaded_script_body(mod_id.as_deref(), token);
                sm.println(self.script_type, &format!(": Add ScriptBody: '{name}' {}", script.borrow().base().debug_string()));
            } else if mode == ScriptLoadMode::Reload && self.inner.borrow().reload {
                if self.script_type.has_flag(Flag::NewInstanceOnReload) {
                    let script = self.new_script(module, sm, &name, token);
                    let mut data = LoadData::new(script.clone());
                    data.script_bodies.push(token.to_string());
                    script.borrow_mut().base_mut().add_loaded_script_body(mod_id.as_deref(), token);
                    data.reloaded = true;
                    data.added_on_reload = false;
                    let key = data.name.clone();
                    self.inner.borrow_mut().load_data.insert(key, data);
                    return Ok(true);
                }
                let clear = {
                    let i = self.inner.borrow();
                    let d = i.load_data.get(&name).unwrap();
                    !d.reloaded && !d.script_bodies.is_empty()
                };
                if clear {
                    self.inner.borrow_mut().load_data.get_mut(&name).unwrap().script_bodies.clear();
                    script.borrow_mut().base_mut().reset_loaded_script_bodies();
                }
                {
                    let mut i = self.inner.borrow_mut();
                    let d = i.load_data.get_mut(&name).unwrap();
                    d.reloaded = true;
                    d.script_bodies.push(token.to_string());
                }
                script.borrow_mut().base_mut().add_loaded_script_body(mod_id.as_deref(), token);
                sm.println(self.script_type, &format!(": Reload ScriptBody: '{name}' {}", script.borrow().base().debug_string()));
            }
        } else if mode == ScriptLoadMode::Init || self.script_type.has_flag(Flag::AllowNewScriptDiscoveryOnReload) {
            let script = self.new_script(module, sm, &name, token);
            let mut data = LoadData::new(script.clone());
            data.script_bodies.push(token.to_string());
            script.borrow_mut().base_mut().add_loaded_script_body(mod_id.as_deref(), token);
            data.reloaded = mode == ScriptLoadMode::Reload;
            data.added_on_reload = mode == ScriptLoadMode::Reload;
            let key = data.name.clone();
            sm.println(self.script_type, &format!(": New ScriptBody: '{key}' {}", script.borrow().base().debug_string()));
            self.inner.borrow_mut().load_data.insert(key, data);
        } else {
            sm.warn("Found new script but was unable to load, possibly due to not being allowed during reload...");
            sm.log(&format!(
                ">>> : Load ScriptBody: '{name}', File: {}",
                current_file.as_deref().unwrap_or("null")
            ));
            if !self.script_type.has_flag(Flag::AllowNewScriptDiscoveryOnReload) {
                sm.log(&format!(
                    ">>> : Discovery of new scripts during reload not allowed for scripts of type: {}",
                    self.script_type.name()
                ));
                if sm.debug {
                    return Err(ScriptError::new("java.lang.Exception: Not allowed"));
                }
            }
        }
        Ok(true)
    }

    /// `ScriptBucket.Template.CreateFromTokenPP`: `template <tag> <name> {`.
    fn create_template_from_token(
        &self,
        module: &Rc<ScriptModule>,
        sm: &ScriptManager,
        type_: &str,
        token: &str,
    ) -> ScriptResult<bool> {
        if type_ != "template" {
            return Ok(false);
        }
        let waypoint = java::split(token, |c| c == '{' || c == '}', false);
        let type_and_name = waypoint.first().copied().unwrap_or("").replace("template", "");
        let split = java::split_ws(java::trim(&type_and_name));
        if split.len() != 2 {
            return Ok(false);
        }
        let (type1, name) = (java::trim(split[0]), java::trim(split[1]));
        if self.script_type.script_tag() != type1 {
            return Ok(false);
        }
        // Templates are created without setModule; their constructor takes
        // the module instead.
        let script = objects::create_instance(self.script_type, module, name, token);
        script.borrow_mut().init_load_pp(sm, name);
        let key = script.name();
        let mut i = self.inner.borrow_mut();
        if key.contains('.') {
            i.dot_in_name.insert(key.clone());
        }
        i.script_map.insert(key.clone(), script);
        drop(i);
        sm.println(self.script_type, &format!("Loaded template: {key}"));
        Ok(true)
    }

    /// `LoadScripts`: loads every collected script from its bodies, in
    /// `HashMap` order of the script names.
    pub fn load_scripts(&self, sm: &ScriptManager, mode: ScriptLoadMode) {
        if self.is_template() {
            let mut i = self.inner.borrow_mut();
            let values = i.script_map.values();
            i.script_list = values;
            return;
        }
        let names: Vec<String> = self.inner.borrow().load_data.keys().map(str::to_string).collect();
        for name in names {
            let r = self.load_one(sm, mode, &name);
            sm.set_current_script_object(None);
            if sm.texture_capture_aborted() { return; }
            if let Err(e) = r {
                sm.log_exception(&e);
                self.set_load_error();
            }
        }
    }

    fn load_one(&self, sm: &ScriptManager, mode: ScriptLoadMode, key: &str) -> ScriptResult {
        let (script, bodies, data_name, added_on_reload) = {
            let i = self.inner.borrow();
            let d = i.load_data.get(key).unwrap();
            (d.script.clone(), d.script_bodies.clone(), d.name.clone(), d.added_on_reload)
        };
        sm.set_current_script_object(script.full_type().ok());
        {
            let mut i = self.inner.borrow_mut();
            let d = i.load_data.get_mut(key).unwrap();
            if mode == ScriptLoadMode::Reload && !d.reloaded {
                return Ok(());
            }
            d.reloaded = false;
        }
        let tag = self.script_type.script_tag();
        if self.is_verbose() {
            sm.debugln(&format!("[{tag}] load script = {}", script.name()));
        }
        if mode == ScriptLoadMode::Reload && self.script_type.has_flag(Flag::ResetOnceOnReload) {
            script.borrow_mut().reset();
        }
        let reset_start = if mode == ScriptLoadMode::Reload { 0 } else { 1 };
        let mut keep = true;
        for (index, body) in bodies.iter().enumerate() {
            if self.script_type.has_flag(Flag::ResetExisting) && index >= reset_start {
                script.borrow_mut().reset();
            }
            let r = script.borrow_mut().load(sm, &data_name, body);
            // A typed host capability fault is neither a native parse error
            // nor a successful definition: stop before diagnostics/publication.
            if sm.texture_capture_aborted() { return Ok(()); }
            match r {
                Ok(()) => {
                    sm.println(self.script_type, &format!(" - Load: '{data_name}' {}", script.borrow().base().debug_string()));
                }
                Err(e) => {
                    if self.script_type.has_flag(Flag::RemoveLoadError) {
                        sm.log(&format!("[{tag}] removing script due to load error = {}", script.name()));
                        keep = false;
                    }
                    self.set_load_error();
                    sm.log_exception(&e);
                    break;
                }
            }
        }
        if keep {
            let (obsolete, enabled, debug_only) = {
                let s = script.borrow();
                (s.get_obsolete(), s.base().enabled, s.base().debug_only)
            };
            if obsolete {
                sm.script_debugln(&format!("[{tag}] ignoring script, obsolete = {}", script.name()));
                return Ok(());
            }
            if !enabled {
                sm.script_debugln(&format!("[{tag}] ignoring script, disabled = {}", script.name()));
                return Ok(());
            }
            if debug_only && !sm.debug {
                sm.script_debugln(&format!("[{tag}] ignoring script, is debug only = {}", script.name()));
                return Ok(());
            }
            script.borrow_mut().base_mut().calculate_script_version()?;
            if mode == ScriptLoadMode::Init || added_on_reload {
                sm.on_script_load(self.script_type, mode, &script)?;
                let name = script.name();
                let mut i = self.inner.borrow_mut();
                i.script_map.insert(name.clone(), script.clone());
                i.script_list.push(script.clone());
                if name.contains('.') {
                    i.dot_in_name.insert(name);
                }
            } else if mode == ScriptLoadMode::Reload {
                sm.on_script_load(self.script_type, mode, &script)?;
            }
        }
        self.inner.borrow_mut().load_data.get_mut(key).unwrap().added_on_reload = false;
        Ok(())
    }

    /// `get(name)`: a dotted name not known locally goes through the
    /// manager; otherwise the local map, then each imported module.
    pub fn get(&self, sm: &ScriptManager, module: &ScriptModule, name: &str) -> Option<ScriptRef> {
        self.get_depth(sm, module, name, 0)
    }

    fn get_depth(&self, sm: &ScriptManager, module: &ScriptModule, name: &str, depth: usize) -> Option<ScriptRef> {
        if name.contains('.') && !self.inner.borrow().dot_in_name.contains(name) {
            return sm.collection(self.script_type).get_script(sm, name);
        }
        if let Some(s) = self.inner.borrow().script_map.get(name) {
            return Some(s.clone());
        }
        if self.script_type.has_flag(Flag::SeekImports) && depth < MAX_IMPORT_DEPTH {
            let imports = module.imports.borrow().clone();
            for module_name in imports {
                let Some(m) = sm.get_module(&module_name) else { continue };
                if let Some(s) = m.bucket(self.script_type).get_depth(sm, &m, name, depth + 1) {
                    return Some(s);
                }
            }
        }
        None
    }
}
