use super::*;
use pz_kahlua::gc::{self, CollectionResult};
use pz_kahlua::LuaObject;
use std::any::Any;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Weak;
use std::sync::atomic::{AtomicU64, Ordering};

const MODULE: &str = "local nested={value=151}; local function get() return nested.value end; Events.Pulse.Add(get); return {nested=nested, get=get}";

struct Directory(PathBuf);
impl Directory {
    fn new(files: &[(&str, &str)]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pz-loader-gc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("media/lua/shared")).unwrap();
        let directory = Self(path);
        for (name, source) in files {
            directory.write(name, source);
        }
        directory
    }
    fn write(&self, name: &str, source: &str) {
        std::fs::write(self.0.join("media/lua/shared").join(name), source).unwrap();
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Isolate these reviewed ownership paths from unrelated exposed engine
// globals. Uses the actual stdlib, EventManager, Loader and require/reload
// implementations, reading real temporary Lua files through GameFs.
fn fixture(files: &[(&str, &str)]) -> (Directory, LuaManager) {
    let directory = Directory::new(files);
    let env = TableRef::new();
    let mut thread = Thread::new(env.clone());
    stdlib::install(&mut thread, None).unwrap();
    thread.out = Box::new(|_| {});
    thread.log = Box::new(|_| {});
    let mut fs = GameFs::new(&directory.0);
    fs.set_cache_dir(directory.0.join("cache"));
    let manager = LuaManager {
        thread,
        fs: Rc::new(RefCell::new(fs)),
        events: EventManager::register(&env, "Events", &["Pulse"]),
        hooks: EventManager::register(&env, "Hook", &[]),
        loader: traceable(Loader {
            paths: vec!["media/lua/shared/".into()],
            ..Loader::default()
        }),
    };
    manager.register_require();
    manager.register_reload_lua_file();
    (directory, manager)
}

fn require(manager: &mut LuaManager, name: &str) -> Value {
    let function = manager.thread.globals().get_str("require");
    manager
        .thread
        .pcall(&function, &[name.into()])
        .unwrap()
        .remove(0)
}

fn collect() {
    assert!(matches!(
        gc::collect_tables(),
        CollectionResult::Collected(_)
    ));
}

fn get_callback(module: &Value) -> Rc<LuaClosure> {
    let Value::Closure(callback) = module.as_table().unwrap().get_str("get") else {
        panic!("expected callback")
    };
    callback
}

#[test]
fn retired_loader_event_and_nested_cached_return_environment_are_reclaimed() {
    let (_directory, mut manager) = fixture(&[("Module.lua", MODULE)]);
    let module = require(&mut manager, "Module");
    let callback = Rc::downgrade(&get_callback(&module));
    let loader = Rc::downgrade(&manager.loader);
    let event = Rc::downgrade(&manager.events.add_event("Pulse"));
    let fs = Rc::downgrade(&manager.fs);
    let Value::Native(require) = manager.thread.globals().get_str("require") else {
        unreachable!()
    };
    let require_weak = Rc::downgrade(&require);
    drop(require);
    drop(module);
    drop(manager);
    collect();
    assert!(callback.upgrade().is_none());
    assert!(loader.upgrade().is_none());
    assert!(event.upgrade().is_none());
    assert!(require_weak.upgrade().is_none());
    assert!(fs.upgrade().is_none());
}

#[test]
fn retained_loader_preserves_real_cached_values_callbacks_and_environment() {
    let (_directory, mut manager) = fixture(&[("Module.lua", MODULE)]);
    let module = require(&mut manager, "Module");
    let callback = Rc::downgrade(&get_callback(&module));
    let loader = manager.loader.clone();
    drop(module);
    drop(manager);
    collect();
    let cached = loader
        .borrow()
        .loaded_return
        .values()
        .next()
        .unwrap()
        .clone();
    assert_eq!(
        cached
            .as_table()
            .unwrap()
            .get_str("nested")
            .as_table()
            .unwrap()
            .get_str("value")
            .as_num(),
        Some(151.0)
    );
    let mut thread = Thread::new(TableRef::new());
    assert_eq!(
        thread
            .call(&Value::Closure(get_callback(&cached)), &[])
            .unwrap()[0]
            .as_num(),
        Some(151.0)
    );
    assert!(loader.borrow().loading.is_empty());
    assert_eq!(loader.borrow().loaded.len(), 1);
    drop(cached);
    drop(loader);
    collect();
    assert!(callback.upgrade().is_none());
}

#[test]
fn retained_require_and_reload_handles_keep_shared_loader_and_cached_identity() {
    let (directory, mut manager) = fixture(&[("Module.lua", MODULE)]);
    let module = require(&mut manager, "Module");
    let identity = module.ptr_id();
    let callback = Rc::downgrade(&get_callback(&module));
    let loader = Rc::downgrade(&manager.loader);
    let require = manager.thread.globals().get_str("require");
    let reload = manager.thread.globals().get_str("reloadLuaFile");
    let path = manager
        .fs
        .borrow()
        .resolve("media/lua/shared/Module.lua")
        .unwrap()
        .to_string_lossy()
        .into_owned();
    drop(module);
    drop(manager);
    collect();
    let mut thread = Thread::new(TableRef::new());
    let cached = thread.call(&require, &["Module".into()]).unwrap().remove(0);
    assert_eq!(cached.ptr_id(), identity);
    assert_eq!(
        thread
            .call(&Value::Closure(get_callback(&cached)), &[])
            .unwrap()[0]
            .as_num(),
        Some(151.0)
    );
    directory.write("Module.lua", "return {version=167}");
    let reloaded = thread.call(&reload, &[path.into()]).unwrap().remove(0);
    assert_ne!(reloaded.ptr_id(), identity);
    assert_eq!(
        reloaded.as_table().unwrap().get_str("version").as_num(),
        Some(167.0)
    );
    assert_eq!(
        cached
            .as_table()
            .unwrap()
            .get_str("nested")
            .as_table()
            .unwrap()
            .get_str("value")
            .as_num(),
        Some(151.0)
    );
    let again = thread
        .call(&require, &["Module.lua".into()])
        .unwrap()
        .remove(0);
    assert!(again.raw_equals(&reloaded));
    drop(again);
    drop(reloaded);
    drop(cached);
    drop(require);
    // Reload still owns the real Loader even after require is released.
    collect();
    assert!(loader.upgrade().is_some());
    drop(reload);
    collect();
    assert!(loader.upgrade().is_none());
    assert!(callback.upgrade().is_none());
}

#[test]
fn nested_require_and_duplicate_cached_return_slots_keep_identity_and_collect() {
    let (_directory, mut manager) = fixture(&[
        ("Inner.lua", MODULE),
        ("Outer.lua", "return require('Inner')"),
        ("Nil.lua", "return nil"),
        ("False.lua", "return false"),
    ]);
    let outer = require(&mut manager, "Outer");
    let inner = require(&mut manager, "Inner");
    assert!(outer.raw_equals(&inner));
    assert!(require(&mut manager, "Nil").is_nil());
    assert!(matches!(require(&mut manager, "False"), Value::Bool(false)));
    let callback = Rc::downgrade(&get_callback(&inner));
    assert_eq!(manager.loader.borrow().loaded.len(), 4);
    assert_eq!(manager.loader.borrow().loaded_return.len(), 3);
    assert!(manager.loader.borrow().loading.is_empty());
    collect();
    assert_eq!(
        get_callback(&outer)
            .env
            .borrow()
            .get_str("Events")
            .as_table()
            .unwrap()
            .get_str("Pulse")
            .as_table()
            .unwrap()
            .size(),
        2
    );
    drop(inner);
    drop(outer);
    drop(manager);
    collect();
    assert!(callback.upgrade().is_none());
}

#[test]
fn loader_borrows_defer_all_collection_without_erasing_another_retired_cache() {
    let (_live_directory, mut live) = fixture(&[("Module.lua", MODULE)]);
    drop(require(&mut live, "Module"));
    let (_dead_directory, mut retired) = fixture(&[("Module.lua", MODULE)]);
    let module = require(&mut retired, "Module");
    let callback = Rc::downgrade(&get_callback(&module));
    drop(module);
    drop(retired);
    let borrow = live.loader.borrow();
    assert_eq!(gc::collect_tables(), CollectionResult::Deferred);
    assert!(callback.upgrade().is_some());
    drop(borrow);
    let borrow = live.loader.borrow_mut();
    assert_eq!(gc::collect_pending(), CollectionResult::Deferred);
    assert!(callback.upgrade().is_some());
    drop(borrow);
    assert!(matches!(
        gc::collect_pending(),
        CollectionResult::Collected(_)
    ));
    assert!(callback.upgrade().is_none());
    assert_eq!(live.loader.borrow().loaded_return.len(), 1);
}

#[test]
fn require_compile_runtime_recursive_and_reload_failure_semantics_are_unchanged() {
    let (directory, mut manager) = fixture(&[
        ("Compile.lua", "local ="),
        ("Runtime.lua", "error('expected-runtime-failure')"),
        (
            "Recursive.lua",
            "local r=require('Recursive'); return {innerWasNil=r==nil}",
        ),
        ("Replace.lua", "return {version=173}"),
    ]);
    assert!(require(&mut manager, "Missing").is_nil());
    assert!(require(&mut manager, "Compile").is_nil());
    assert!(require(&mut manager, "Runtime").is_nil());
    let recursive = require(&mut manager, "Recursive");
    assert!(matches!(
        recursive.as_table().unwrap().get_str("innerWasNil"),
        Value::Bool(true)
    ));
    assert!(manager.loader.borrow().loading.is_empty());
    assert_eq!(manager.loader.borrow().errors.len(), 2);
    // Compile failures are retryable, while a runtime failure is recorded
    // as loaded and stays nil until explicitly reloaded.
    directory.write("Compile.lua", "return 179");
    directory.write("Runtime.lua", "return 181");
    assert_eq!(require(&mut manager, "Compile").as_num(), Some(179.0));
    assert!(require(&mut manager, "Runtime").is_nil());
    let old = require(&mut manager, "Replace");
    let reload = manager.thread.globals().get_str("reloadLuaFile");
    assert!(manager
        .thread
        .pcall(&reload, &["../Replace.lua".into()])
        .unwrap()[0]
        .is_nil());
    let outside = directory
        .0
        .with_file_name(format!("pz-loader-outside-{}", std::process::id()))
        .join("No.lua");
    let outside_error = manager
        .thread
        .pcall(&reload, &[outside.to_string_lossy().as_ref().into()])
        .unwrap_err()
        .0;
    assert!(pz_kahlua::exec::java_value_of(&outside_error).contains("Invalid prefix found for:"));
    let path = manager
        .fs
        .borrow()
        .resolve("media/lua/shared/Replace.lua")
        .unwrap()
        .to_string_lossy()
        .into_owned();
    directory.write("Replace.lua", "return {version=191}");
    let replaced = manager
        .thread
        .pcall(&reload, &[path.into()])
        .unwrap()
        .remove(0);
    assert_eq!(
        old.as_table().unwrap().get_str("version").as_num(),
        Some(173.0)
    );
    assert_eq!(
        replaced.as_table().unwrap().get_str("version").as_num(),
        Some(191.0)
    );
    collect();
    assert!(manager.loader.borrow().loading.is_empty());
    assert_eq!(manager.loader.borrow().errors.len(), 2);
}

struct ReentrantDrop {
    loader: Weak<RefCell<Loader>>,
    callback: Weak<LuaClosure>,
    dropped: Rc<Cell<bool>>,
}
impl LuaObject for ReentrantDrop {
    fn class_name(&self) -> &str {
        "test.LoaderReentrantDrop"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl Drop for ReentrantDrop {
    fn drop(&mut self) {
        assert!(self.loader.upgrade().unwrap().try_borrow_mut().is_ok());
        assert!(self
            .callback
            .upgrade()
            .unwrap()
            .env
            .try_borrow_mut()
            .is_ok());
        drop(traceable(Loader::default()));
        gc::request_collection();
        assert_eq!(gc::collect_tables(), CollectionResult::Deferred);
        self.dropped.set(true);
    }
}

#[test]
fn loader_cache_payload_drops_after_all_borrows_and_can_reenter_registration() {
    let (_directory, mut manager) = fixture(&[(
        "Module.lua",
        "local function get() return 193 end; return {get=get, probe=probe}",
    )]);
    let callback =
        stdlib::load_string("return 197", None, manager.thread.globals().clone()).unwrap();
    let Value::Closure(callback) = callback else {
        unreachable!()
    };
    let callback_weak = Rc::downgrade(&callback);
    // Keep this closure in its environment so the probe can inspect its
    // actual heap borrow while white payloads are being destroyed.
    manager
        .thread
        .globals()
        .set_str("kept", Value::Closure(callback.clone()));
    let dropped = Rc::new(Cell::new(false));
    manager.thread.globals().set_str(
        "probe",
        Value::Object(Rc::new(ReentrantDrop {
            loader: Rc::downgrade(&manager.loader),
            callback: callback_weak.clone(),
            dropped: dropped.clone(),
        })),
    );
    drop(callback);
    drop(require(&mut manager, "Module"));
    drop(manager);
    collect();
    assert!(dropped.get());
    assert!(callback_weak.upgrade().is_none());
    assert!(matches!(
        gc::collect_pending(),
        CollectionResult::Collected(_)
    ));
}

#[test]
fn raw_loader_construction_remains_conservative_without_weakening_its_cached_values() {
    let (_directory, mut manager) = fixture(&[("Module.lua", MODULE)]);
    manager.loader = Rc::new(RefCell::new(Loader {
        paths: vec!["media/lua/shared/".into()],
        ..Loader::default()
    }));
    manager.register_require();
    manager.register_reload_lua_file();
    let module = require(&mut manager, "Module");
    let callback = Rc::downgrade(&get_callback(&module));
    let loader = Rc::downgrade(&manager.loader);
    drop(module);
    drop(manager);
    collect();
    let owner = loader.upgrade().unwrap();
    assert_eq!(owner.borrow().loaded_return.len(), 1);
    assert!(callback.upgrade().is_some());
    // Explicit fixture cleanup breaks this intentionally opaque cycle.
    owner.borrow_mut().loaded_return.clear();
    drop(owner);
    collect();
    assert!(callback.upgrade().is_none());
    assert!(loader.upgrade().is_none());
}
