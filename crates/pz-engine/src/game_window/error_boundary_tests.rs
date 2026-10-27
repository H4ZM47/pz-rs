use super::*;
use pz_kahlua::{LuaError, NativeFunction, TableRef, Value};

fn manager() -> LuaManager {
    LuaManager::init(Rc::new(RefCell::new(pz_fs::GameFs::new(std::env::temp_dir())))).unwrap()
}

#[test]
fn headless_script_failure_returns_the_original_table_payload() {
    let _deferral = pz_kahlua::gc::defer_collection();
    let mut lua = manager();
    let payload = TableRef::new();
    lua.thread.globals().set_str("payload", Value::Table(payload.clone()));
    let error = run_headless_source(&mut lua, "error(payload); after=true").unwrap_err();
    match error.lua_error() {
        Some(LuaError::Kahlua(Value::Table(actual))) => assert!(actual.ptr_eq(&payload)),
        _ => panic!("headless harness erased the original payload"),
    }
    assert!(lua.thread.globals().get_str("after").is_nil());
}

#[test]
fn headless_compile_and_native_failures_are_errors_before_later_mutations() {
    let _deferral = pz_kahlua::gc::defer_collection();
    let mut lua = manager();
    assert!(run_headless_source(&mut lua, "local = invalid").unwrap_err().lua_error().is_some());
    lua.thread.globals().set_str("fail", Value::Native(NativeFunction::new("fail", |_| {
        Err(LuaError::Runtime { class: "pz.host.Borrowed", message: "same original phase".into() })
    })));
    let error = run_headless_source(&mut lua, "fail(); after=true").unwrap_err();
    assert!(matches!(error.lua_error(), Some(LuaError::Runtime {
        class: "pz.host.Borrowed", message
    }) if message == "same original phase"));
    assert!(lua.thread.globals().get_str("after").is_nil());
}
