//! `zombie.Lua.LuaManager`: environment setup, file loading order and
//! `require`.

use crate::events::{EventManager, EVENT_NAMES, HOOK_NAMES};
use pz_fs::GameFs;
use pz_kahlua::vm::LuaClosure;
use pz_kahlua::{stdlib, LuaError, NativeFunction, TableRef, Thread, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Script loading state shared with the `require` global.
#[derive(Default)]
pub struct Loader {
    /// `LuaManager.paths`: `media/lua/<sub>/` roots searched by `require`.
    pub paths: Vec<String>,
    loaded: HashSet<String>,
    loading: HashSet<String>,
    loaded_return: HashMap<String, Value>,
    pub errors: Vec<String>,
}

pub struct LuaManager {
    pub thread: Thread,
    pub fs: Rc<GameFs>,
    pub events: EventManager,
    pub hooks: EventManager,
    pub loader: Rc<RefCell<Loader>>,
}

impl LuaManager {
    /// `LuaManager.init()`: fresh environment, Kahlua stdlib (including the
    /// game's `stdlib.lua`), events, hooks and the exposed game API.
    pub fn init(fs: Rc<GameFs>) -> Result<Self, LuaError> {
        let env = TableRef::new();
        let mut thread = Thread::new(env.clone());
        let stdlib_src = fs.resolve("stdlib.lua").and_then(|p| std::fs::read(p).ok());
        let stdlib_src = stdlib_src.map(|b| String::from_utf8_lossy(&b).into_owned());
        stdlib::install(&mut thread, stdlib_src.as_deref())?;
        let events = EventManager::register(&env, "Events", EVENT_NAMES);
        let hooks = EventManager::register(&env, "Hook", HOOK_NAMES);
        let loader = Rc::new(RefCell::new(Loader::default()));
        let mgr = LuaManager { thread, fs, events, hooks, loader };
        mgr.register_require();
        let mut mgr = mgr;
        crate::register_all(&mut mgr);
        Ok(mgr)
    }

    fn register_require(&self) {
        let loader = self.loader.clone();
        let fs = self.fs.clone();
        self.thread.globals().set_str(
            "require",
            Value::Native(NativeFunction::new("require", move |c| {
                let f = pz_kahlua::stdlib::string_arg(c, 1, "require")?;
                let mut name = f.to_string();
                if !name.ends_with(".lua") {
                    name.push_str(".lua");
                }
                let paths = loader.borrow().paths.clone();
                for rel in paths {
                    if let Some(abs) = fs.resolve(&format!("{rel}{name}")) {
                        let abs = abs.to_string_lossy().replace('\\', "/");
                        let r = run_lua(c.thread, &loader, &abs);
                        return Ok(c.push(r));
                    }
                }
                (c.thread.out)(&format!("require(\"{f}\") failed"));
                Ok(c.push(Value::Nil))
            })),
        );
    }

    /// `LoadDirBase(sub)`: run every Lua file of `media/lua/<sub>` in
    /// case-insensitive path order. TODO: mod directories.
    pub fn load_dir_base(&mut self, sub: &str) {
        let rel = format!("media/lua/{sub}/");
        {
            let mut l = self.loader.borrow_mut();
            if !l.paths.contains(&rel) {
                l.paths.push(rel);
            }
        }
        let files = self.fs.lua_files(sub);
        let mut done = HashSet::new();
        for f in files {
            if !done.insert(f.clone()) {
                continue;
            }
            let Some(abs) = self.fs.resolve(&f) else {
                self.loader.borrow_mut().errors.push(format!("couldn't find \"{f}\""));
                continue;
            };
            let abs = abs.to_string_lossy().replace('\\', "/");
            run_lua(&mut self.thread, &self.loader, &abs);
        }
    }

    /// `LoadDirBase()`: shared then client.
    pub fn load_all_client(&mut self) {
        self.load_dir_base("shared");
        self.load_dir_base("client");
    }

    pub fn trigger_event(&mut self, name: &str, params: &[Value]) -> bool {
        let ev = self.events.clone();
        ev.trigger(&mut self.thread, name, params)
    }
}

/// `RunLua(filename)`: compile and run once; later calls return the cached
/// first return value.
pub fn run_lua(thread: &mut Thread, loader: &Rc<RefCell<Loader>>, filename: &str) -> Value {
    let filename = filename.replace('\\', "/");
    {
        let l = loader.borrow();
        if l.loading.contains(&filename) {
            drop(l);
            (thread.out)(&format!("recursive require(): {filename}"));
            return Value::Nil;
        }
        if l.loaded.contains(&filename) {
            return l.loaded_return.get(&filename).cloned().unwrap_or_default();
        }
    }
    loader.borrow_mut().loading.insert(filename.clone());
    let r = run_lua_internal(thread, loader, &filename);
    loader.borrow_mut().loading.remove(&filename);
    r
}

fn run_lua_internal(thread: &mut Thread, loader: &Rc<RefCell<Loader>>, filename: &str) -> Value {
    let base = filename.rsplit('/').next().unwrap_or(filename).to_string();
    let src = match std::fs::read(filename) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => {
            loader.borrow_mut().errors.push(format!("{filename}: {e}"));
            return Value::Nil;
        }
    };
    let proto = match pz_kahlua::compiler::compile(&src, Some(&base), Some(&base), Some(filename)) {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("Error found in LUA file: {filename}: {e}");
            (thread.log)(&msg);
            loader.borrow_mut().errors.push(msg);
            return Value::Nil;
        }
    };
    let f = Value::Closure(LuaClosure::new(Rc::new(proto), thread.globals().clone()));
    let ret = match thread.pcall(&f, &[]) {
        Ok(v) => v.into_iter().next().unwrap_or_default(),
        Err((msg, trace)) => {
            let m = format!("{filename}: {}\n{trace}", pz_kahlua::exec::java_value_of(&msg));
            (thread.log)(&m);
            loader.borrow_mut().errors.push(m);
            Value::Nil
        }
    };
    let mut l = loader.borrow_mut();
    l.loaded.insert(filename.to_string());
    if ret.is_nil() {
        l.loaded_return.remove(filename);
    } else {
        l.loaded_return.insert(filename.to_string(), ret.clone());
    }
    ret
}
