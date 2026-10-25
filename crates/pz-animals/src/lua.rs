//! A Kahlua environment holding the animal definition tables, for tools and
//! tests that run without the full `LuaManager`.
//!
//! In the game the tables live in `LuaManager.env` after
//! `LoadDirBase("shared")`; the loaders of this crate read them from any
//! environment. Here only the files under
//! `media/lua/shared/Definitions/animal/` are run, in `LoadDirBase` order
//! (case-insensitive path order, then mods), with the Kahlua stdlib, the
//! game's `stdlib.lua` and the `copyTable` global they use. No other shared
//! file writes the animal definition globals.

use std::path::Path;
use std::rc::Rc;

use pz_fs::GameFs;
use pz_kahlua::vm::LuaClosure;
use pz_kahlua::{stdlib, LuaError, NativeFunction, TableRef, Thread, Value};

/// `LoadDirBase` key prefix of the animal definition files.
pub const DEFINITIONS_DIR: &str = "media/lua/shared/definitions/animal/";

/// `LuaManager.copyTable(null, from)`: deep copy of nested tables in
/// iteration order.
pub fn copy_table(from: &TableRef) -> TableRef {
    let to = TableRef::new();
    for (k, v) in from.entries() {
        match v {
            Value::Table(t) => to.rawset(k, Value::Table(copy_table(&t))),
            v => to.rawset(k, v),
        }
    }
    to
}

pub struct DefinitionsLua {
    pub thread: Thread,
    /// Files that failed to read, compile or run.
    pub errors: Vec<String>,
}

impl DefinitionsLua {
    /// A fresh environment with the definition files of `fs` run.
    pub fn load(fs: &GameFs) -> Result<Self, LuaError> {
        let mut thread = Thread::new(TableRef::new());
        thread.out = Box::new(|_| {});
        thread.log = Box::new(|_| {});
        let src = fs
            .resolve("stdlib.lua")
            .and_then(|p| std::fs::read(p).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        stdlib::install(&mut thread, src.as_deref())?;
        thread.globals().set_str(
            "copyTable",
            Value::Native(NativeFunction::new("copyTable", |c| {
                let r = match c.arg(0) {
                    Value::Table(t) => copy_table(&t),
                    _ => TableRef::new(),
                };
                Ok(c.push(r))
            })),
        );
        let mut lua = DefinitionsLua {
            thread,
            errors: Vec::new(),
        };
        for f in fs.lua_files("shared") {
            if !f.starts_with(DEFINITIONS_DIR) {
                continue;
            }
            match fs.absolute_path(&f) {
                Some(abs) => {
                    let abs = abs.to_string();
                    lua.run(Path::new(&abs));
                }
                None => lua.errors.push(format!("couldn't find \"{f}\"")),
            }
        }
        Ok(lua)
    }

    /// The environment (`LuaManager.env`).
    pub fn env(&self) -> TableRef {
        self.thread.globals().clone()
    }

    /// `RunLua(path)`.
    pub fn run(&mut self, path: &Path) {
        let key = path.to_string_lossy().replace('\\', "/");
        let src = match std::fs::read(path) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => {
                self.errors.push(format!("{key}: {e}"));
                return;
            }
        };
        let base = key.rsplit('/').next().unwrap_or(&key).to_string();
        let proto = match pz_kahlua::compiler::compile(&src, Some(&base), Some(&base), Some(&key)) {
            Ok(p) => p,
            Err(e) => {
                self.errors.push(format!("{key}: {e}"));
                return;
            }
        };
        let f = Value::Closure(LuaClosure::new(
            Rc::new(proto),
            self.thread.globals().clone(),
        ));
        if let Err((msg, _)) = self.thread.pcall(&f, &[]) {
            self.errors
                .push(format!("{key}: {}", pz_kahlua::exec::java_value_of(&msg)));
        }
    }
}
