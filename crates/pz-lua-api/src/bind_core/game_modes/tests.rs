use super::*;
use pz_core::core::Core;
use pz_kahlua::{LuaError, LuaObject, TableRef, Thread};
use pz_scripts::native_game_modes::{NativeGameModeStatics, NativeGameModeStaticsRef};
use pz_scripts::registry::RegistryKey;

fn statics() -> NativeGameModeStaticsRef {
    let declarations = [("SANDBOX", "Sandbox"), ("TUTORIAL", "Tutorial")].map(|(field, id)| {
        pz_classfile::StaticInit {
            class: "zombie/core/GameMode".into(),
            field: field.into(),
            value: pz_classfile::Value::Call {
                class: "zombie/core/GameMode".into(),
                name: "registerBase".into(),
                descriptor: String::new(),
                args: vec![pz_classfile::Value::Str(id.into())],
            },
        }
    });
    Rc::new(NativeGameModeStatics::from_class_initializers(&declarations).unwrap())
}
fn setup(core: Rc<RefCell<Core>>, modes: &NativeGameModeStaticsRef) -> Thread {
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    let manager = Rc::new(pz_scripts::ScriptManager::with_native_game_mode_statics(
        modes,
    ));
    crate::bind_registry::register(&mut thread, manager);
    crate::bind_core::register(&mut thread, None, core);
    thread
}
fn call(thread: &mut Thread, source: &str) -> LuaResult<Vec<Value>> {
    let function = pz_kahlua::stdlib::load_string(
        source,
        Some("actual Core GameMode"),
        thread.globals().clone(),
    )
    .unwrap();
    thread
        .pcall_ex(&function, &[])
        .map_err(|(_, _, error)| error)
}
fn booleans(values: Vec<Value>) {
    assert!(!values.is_empty());
    assert!(values
        .iter()
        .all(|value| value.raw_equals(&Value::Bool(true))));
}
fn host(result: LuaResult<Vec<Value>>, expected: &str) {
    assert!(matches!(result, Err(LuaError::Runtime { class, .. }) if class == expected));
}

#[test]
fn game_mode_methods_return_canonical_keys_and_read_live_identity() {
    let _collection = pz_kahlua::gc::defer_collection();
    let modes = statics();
    let core = Rc::new(RefCell::new(Core::new()));
    core.borrow_mut()
        .install_game_mode_statics(modes.clone())
        .unwrap();
    let mut thread = setup(core.clone(), &modes);
    booleans(call(&mut thread, "return getCore() == Core.getInstance(), getCore():getGameMode() == GameMode.SANDBOX, getCore():isTutorial() == false").unwrap());
    booleans(call(&mut thread, "getCore():setGameMode(GameMode.TUTORIAL); return getCore():isTutorial(), getCore():getGameMode() == GameMode.TUTORIAL").unwrap());
    booleans(call(&mut thread, "getCore():setGameMode('base:TUTORIAL'); return getCore():isTutorial(), getCore():getGameMode() == GameMode.TUTORIAL").unwrap());
    let tutorial = modes.constant("TUTORIAL").unwrap();
    let duplicate = Rc::new(RegistryKey {
        name: tutorial.name.clone(),
        location: tutorial.location.clone(),
        item_key: None,
    });
    thread.globals().set_str(
        "duplicate",
        crate::bind_registry::key_value(crate::bind_registry::GAME_MODE_CLASS, Some(&duplicate)),
    );
    booleans(call(&mut thread, "getCore():setGameMode(duplicate); return getCore():getGameMode() == duplicate, getCore():getGameMode() ~= GameMode.TUTORIAL, getCore():isTutorial() == false").unwrap());
    booleans(call(&mut thread, "local mode=GameMode.register('fixture:custom'); getCore():setGameMode(mode); return getCore():getGameMode()==mode, getCore():isTutorial()==false").unwrap());
    booleans(call(&mut thread, "getCore():setGameMode('fixture:missing'); return getCore():getGameMode()==nil, getCore():isTutorial()==false").unwrap());
    assert!(core.borrow().get_game_mode_ref().is_none());
    core.borrow_mut()
        .set_game_mode_ref(Some(tutorial.clone()))
        .unwrap();
    booleans(
        call(
            &mut thread,
            "return getCore():isTutorial(), getCore():getGameMode()==GameMode.TUTORIAL",
        )
        .unwrap(),
    );
}

#[test]
fn rebinding_lua_preserves_nullable_mode_and_class_static_owners() {
    let _collection = pz_kahlua::gc::defer_collection();
    let modes = statics();
    let core = Rc::new(RefCell::new(Core::new()));
    core.borrow_mut()
        .install_game_mode_statics(modes.clone())
        .unwrap();
    let mut first = setup(core.clone(), &modes);
    let retained = call(&mut first, "return GameMode.TUTORIAL")
        .unwrap()
        .remove(0);
    core.borrow_mut().set_game_mode_ref(None).unwrap();
    core.borrow_mut()
        .install_game_mode_statics(modes.clone())
        .unwrap();
    let mut second = setup(core.clone(), &modes);
    second.globals().set_str("retained", retained);
    booleans(call(&mut second, "return GameMode.TUTORIAL==retained, getCore():getGameMode()==nil, getCore():isTutorial()==false").unwrap());
    call(&mut second, "getCore():setGameMode(retained)").unwrap();
    assert!(core.borrow().is_tutorial().unwrap());
}

struct Hostile;
impl LuaObject for Hostile {
    fn class_name(&self) -> &str {
        panic!("do not call unknown class metadata")
    }
    fn as_any(&self) -> &dyn Any {
        panic!("do not call unknown virtual as_any")
    }
    fn to_lua_string(&self) -> String {
        panic!("do not format rejected GameMode")
    }
}

#[test]
fn mode_conversion_and_arity_faults_precede_body_access_and_stay_typed() {
    let _collection = pz_kahlua::gc::defer_collection();
    let modes = statics();
    let core = Rc::new(RefCell::new(Core::new()));
    core.borrow_mut()
        .install_game_mode_statics(modes.clone())
        .unwrap();
    let mut thread = setup(core.clone(), &modes);
    thread
        .globals()
        .set_str("hostile", Value::Object(Rc::new(Hostile)));
    {
        let _body = core.borrow_mut();
        for source in [
            "local f=getCore().isTutorial; return f(hostile)",
            "return getCore():isTutorial(nil)",
            "getCore():setGameMode(hostile)",
            "getCore():setGameMode(false)",
            "getCore():setGameMode({})",
            "getCore():setGameMode(nil)",
            "getCore():setGameMode()",
            "local f=getCore().getGameMode; return f(nil)",
        ] {
            host(call(&mut thread, source), "pz.host.Unsupported");
        }
        host(
            call(&mut thread, "return getCore():isTutorial()"),
            "pz.host.Borrowed",
        );
        host(
            call(&mut thread, "return getCore():getGameMode()"),
            "pz.host.Borrowed",
        );
        host(
            call(&mut thread, "getCore():setGameMode(GameMode.TUTORIAL)"),
            "pz.host.Borrowed",
        );
    }
    {
        let _registry = modes.registry().borrow_mut();
        host(
            call(&mut thread, "getCore():setGameMode('Tutorial')"),
            "pz.host.Borrowed",
        );
        booleans(call(&mut thread, "return getCore():isTutorial()==false").unwrap());
    }
    host(
        call(&mut thread, "getCore():setGameMode('')"),
        "java.lang.IllegalArgumentException",
    );
    booleans(
        call(
            &mut thread,
            "return getCore():getGameMode()==GameMode.SANDBOX",
        )
        .unwrap(),
    );
}

#[test]
fn build_42_21_core_exposes_string_modes_and_keeps_custom_names() {
    let core = Rc::new(RefCell::new(Core::new()));
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    crate::bind_core::register(&mut thread, None, core.clone());
    booleans(call(&mut thread, r#"
        local c = getCore()
        local defaultMode = c:getGameMode()
        c:setGameMode("A Really CD DA")
        local custom = c:getGameMode()
        c:setGameMode("LastStand")
        c:setOptionZoomLevels1x("50;100")
        return c:getOptionZoomLevels1x() == "50;100", c:getOptionTexture2x(),
               c.isOptionShowConnectionInfo == nil, defaultMode == "Sandbox", custom == "A Really CD DA",
               c:getGameMode() == "LastStand", c.isTutorial == nil
    "#).unwrap());
    assert!(!core.borrow().add_zombie_on_cell_load);
    assert!(call(&mut thread, "getCore():setGameMode(nil)").is_err());
    assert_eq!(core.borrow().game_mode_name(), None);
}
