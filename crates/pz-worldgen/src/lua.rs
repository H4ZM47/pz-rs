//! Running the game's world-generation Lua: the `worldgen` table
//! (`media/lua/server/WorldGen/**`), `biome_map_config`
//! (`media/lua/server/metazones/BiomeMapConfig.lua`), each map's
//! `WorldGenOverride.lua` (`WorldGenChunk.runLuaOverride`) and the map
//! zones in `objects.lua`.
//!
//! Only the parts of `LuaManager` these files use are provided: the Kahlua
//! stdlib, `require` over `media/lua/shared` and `media/lua/server`, and
//! `WorldGenUtils.INSTANCE:getFiles/getFilesNum/getFile`.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_fs::GameFs;
use pz_kahlua::vm::LuaClosure;
use pz_kahlua::{stdlib, LuaError, NativeFunction, TableRef, Thread, Value};

/// Search roots of `require`, in `LuaManager.paths` order.
const PATHS: [&str; 2] = ["media/lua/shared/", "media/lua/server/"];

struct Loader {
    fs: Rc<GameFs>,
    loaded: HashSet<String>,
    errors: Vec<String>,
    /// `WorldGenUtils.files` (the last `getFiles` result).
    files: Vec<String>,
}

/// A Kahlua environment holding the world-generation definitions.
pub struct WorldGenLua {
    pub thread: Thread,
    loader: Rc<RefCell<Loader>>,
}

impl WorldGenLua {
    /// A fresh environment with the stdlib (and the game's `stdlib.lua`
    /// when present), `require` and `WorldGenUtils`.
    pub fn new(fs: Rc<GameFs>) -> Result<Self, LuaError> {
        let env = TableRef::new();
        let mut thread = Thread::new(env);
        thread.out = Box::new(|_| {});
        thread.log = Box::new(|_| {});
        let src = fs
            .resolve("stdlib.lua")
            .and_then(|p| std::fs::read(p).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        stdlib::install(&mut thread, src.as_deref())?;
        let loader = Rc::new(RefCell::new(Loader {
            fs,
            loaded: HashSet::new(),
            errors: Vec::new(),
            files: Vec::new(),
        }));
        let lua = WorldGenLua { thread, loader };
        lua.register();
        Ok(lua)
    }

    fn register(&self) {
        let g = self.thread.globals().clone();
        let loader = self.loader.clone();
        g.set_str(
            "require",
            Value::Native(NativeFunction::new("require", move |c| {
                let f = stdlib::string_arg(c, 1, "require")?;
                let mut name = f.to_string();
                if !name.ends_with(".lua") {
                    name.push_str(".lua");
                }
                let fs = loader.borrow().fs.clone();
                for rel in PATHS {
                    if let Some(abs) = fs.resolve(&format!("{rel}{name}")) {
                        let r = run_file(c.thread, &loader, &abs);
                        return Ok(c.push(r));
                    }
                }
                loader
                    .borrow_mut()
                    .errors
                    .push(format!("require(\"{f}\") failed"));
                Ok(c.push(Value::Nil))
            })),
        );
        let utils = TableRef::new();
        let instance = TableRef::new();
        let l = self.loader.clone();
        instance.set_str(
            "getFiles",
            Value::Native(NativeFunction::new("getFiles", move |c| {
                let base = stdlib::string_arg(c, 2, "getFiles")?;
                let mut ld = l.borrow_mut();
                ld.files = get_files(&ld.fs, &base);
                Ok(0)
            })),
        );
        let l = self.loader.clone();
        instance.set_str(
            "getFilesNum",
            Value::Native(NativeFunction::new("getFilesNum", move |c| {
                let n = l.borrow().files.len();
                Ok(c.push(Value::Num(n as f64)))
            })),
        );
        let l = self.loader.clone();
        instance.set_str(
            "getFile",
            Value::Native(NativeFunction::new("getFile", move |c| {
                let i = stdlib::number_arg(c, 2, "getFile")? as i64;
                let f = l.borrow().files.get(i as usize).cloned();
                match f {
                    Some(f) => Ok(c.push(Value::from(f))),
                    None => Err(LuaError::runtime("IndexOutOfBoundsException")),
                }
            })),
        );
        utils.set_str("INSTANCE", instance);
        g.set_str("WorldGenUtils", utils);
    }

    /// `require(name)`.
    pub fn require(&mut self, name: &str) -> Value {
        let fs = self.loader.borrow().fs.clone();
        let mut name = name.to_string();
        if !name.ends_with(".lua") {
            name.push_str(".lua");
        }
        for rel in PATHS {
            if let Some(abs) = fs.resolve(&format!("{rel}{name}")) {
                return run_file(&mut self.thread, &self.loader, &abs);
            }
        }
        self.loader
            .borrow_mut()
            .errors
            .push(format!("require(\"{name}\") failed"));
        Value::Nil
    }

    /// `LuaManager.RunLua(path)` (cached per path).
    pub fn run(&mut self, path: &Path) -> Value {
        run_file(&mut self.thread, &self.loader, path)
    }

    /// Loads what world generation reads, the way the game's server Lua
    /// startup does: `WorldGen/WorldGen` (which pulls in features, biomes,
    /// selection, veins, roads and priorities through
    /// `WorldGenUtils.getFiles`), then every other file under
    /// `media/lua/server/WorldGen` in case-insensitive path order (the
    /// prefabs), then `metazones/BiomeMapConfig`, then each map's
    /// `WorldGenOverride.lua`.
    pub fn load_world_gen(&mut self, maps: &[&str]) {
        self.require("WorldGen/WorldGen");
        let fs = self.loader.borrow().fs.clone();
        let mut rest: Vec<String> = fs
            .lua_files("server")
            .into_iter()
            .filter(|f| f.starts_with("media/lua/server/worldgen/"))
            .collect();
        rest.sort_by_key(|a| a.to_lowercase());
        for f in rest {
            if let Some(abs) = fs.resolve(&f) {
                run_file(&mut self.thread, &self.loader, &abs);
            }
        }
        self.require("metazones/BiomeMapConfig");
        for m in maps {
            if let Some(p) = fs.resolve(&format!("media/maps/{m}/WorldGenOverride.lua")) {
                if p.exists() {
                    run_file(&mut self.thread, &self.loader, &p);
                }
            }
        }
    }

    /// A global table.
    pub fn global_table(&self, name: &str) -> Option<TableRef> {
        self.thread.globals().get_str(name).as_table().cloned()
    }

    /// Errors met while loading (failed requires, Lua errors).
    pub fn errors(&self) -> Vec<String> {
        self.loader.borrow().errors.clone()
    }
}

/// `WorldGenUtils.getFiles(basePath)`: the `.lua` files under
/// `media/<basePath>`, depth first in directory-listing order like
/// `Files.walk`, relative to the game directory.
fn get_files(fs: &GameFs, base: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(root) = fs.resolve(&format!("media/{base}")) else {
        return out;
    };
    walk(&fs.base, &root, &mut out);
    out
}

fn walk(base: &Path, p: &Path, out: &mut Vec<String>) {
    if p.is_file() {
        if p.to_string_lossy().ends_with("lua") {
            if let Ok(rel) = p.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        return;
    }
    let Ok(rd) = std::fs::read_dir(p) else { return };
    for e in rd.flatten() {
        walk(base, &e.path(), out);
    }
}

fn run_file(thread: &mut Thread, loader: &Rc<RefCell<Loader>>, path: &Path) -> Value {
    let key: PathBuf = path.to_path_buf();
    let key = key.to_string_lossy().replace('\\', "/");
    // Paths are resolved case-insensitively, so the cache is too.
    if !loader.borrow_mut().loaded.insert(key.to_lowercase()) {
        return Value::Nil;
    }
    let src = match std::fs::read(path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => {
            loader.borrow_mut().errors.push(format!("{key}: {e}"));
            return Value::Nil;
        }
    };
    let base = key.rsplit('/').next().unwrap_or(&key).to_string();
    let proto = match pz_kahlua::compiler::compile(&src, Some(&base), Some(&base), Some(&key)) {
        Ok(p) => p,
        Err(e) => {
            loader.borrow_mut().errors.push(format!("{key}: {e}"));
            return Value::Nil;
        }
    };
    let f = Value::Closure(LuaClosure::new(Rc::new(proto), thread.globals().clone()));
    match thread.pcall(&f, &[]) {
        Ok(v) => v.into_iter().next().unwrap_or_default(),
        Err((msg, _)) => {
            let m = format!("{key}: {}", pz_kahlua::exec::java_value_of(&msg));
            loader.borrow_mut().errors.push(m);
            Value::Nil
        }
    }
}

/// Runs a data-only Lua file (such as a map's `objects.lua`) in a fresh
/// environment and returns the global `name` it defines.
pub fn load_data_table(path: &Path, name: &str) -> Result<Option<TableRef>, String> {
    let src = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let src = String::from_utf8_lossy(&src);
    let mut thread = Thread::new(TableRef::new());
    thread.out = Box::new(|_| {});
    thread.log = Box::new(|_| {});
    stdlib::install(&mut thread, None).map_err(|e| e.to_string())?;
    let proto = pz_kahlua::compiler::compile(&src, Some("objects.lua"), Some("objects.lua"), None)
        .map_err(|e| e.to_string())?;
    let f = Value::Closure(LuaClosure::new(Rc::new(proto), thread.globals().clone()));
    thread
        .pcall(&f, &[])
        .map_err(|(m, _)| pz_kahlua::exec::java_value_of(&m))?;
    Ok(thread.globals().get_str(name).as_table().cloned())
}
