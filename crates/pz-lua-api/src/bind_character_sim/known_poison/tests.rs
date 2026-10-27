use super::*;
use pz_classfile::{StaticInit, Value as ClassValue};
use pz_scripts::native_item_keys::{NativeItemKeyStatics, NativeItemKeyStaticsRef};
use pz_scripts::ScriptObject;

struct Restore {
    script: Rc<ScriptManager>,
    core: Rc<RefCell<pz_core::core::Core>>,
}
impl Restore {
    fn new() -> Self {
        Self {
            script: crate::bind_registry::shared_script_manager(),
            core: crate::bind_core::shared_core(),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        crate::bind_registry::install_script_manager(self.script.clone());
        crate::bind_core::install_core(self.core.clone());
        pz_inventory::component_host::drain_retired_components();
    }
}
fn globals() -> NativeItemKeyStaticsRef {
    let init = |class: &str, field: &str, id: &str| StaticInit {
        class: class.into(),
        field: field.into(),
        value: ClassValue::Call {
            class: class.into(),
            name: "registerBase".into(),
            descriptor: String::new(),
            args: vec![ClassValue::Str(id.into())],
        },
    };
    Rc::new(
        NativeItemKeyStatics::from_class_initializers(
            &[
                init("zombie/scripting/objects/ItemTag", "NO_DETECT", "noDetect"),
                init(
                    "zombie/scripting/objects/ItemTag",
                    "SHOW_POISON",
                    "showPoison",
                ),
            ],
            &[init("zombie/scripting/objects/ItemType", "FOOD", "Food")],
        )
        .unwrap(),
    )
}
fn setup() -> (
    Thread,
    super::super::LuaGameCharacterRef,
    Rc<ScriptManager>,
    NativeItemKeyStaticsRef,
) {
    let globals = globals();
    let manager = ScriptManager::with_native_item_key_statics(&globals);
    let (mut thread, character, _) = super::super::tests::setup_with(manager);
    let manager = crate::bind_registry::shared_script_manager();
    manager.enable_texture_capture();
    manager
        .parse_script(pz_scripts::ScriptLoadMode::Init, "module Base {}")
        .unwrap();
    crate::bind_items::register(&mut thread);
    let mut chars = Characters::default();
    let cooking = chars.perks.new_perk("Cooking").unwrap();
    chars.perks.perks.push(("Cooking".into(), cooking));
    let builder =
        crate::expose::expose_class(&mut thread, super::super::ISO_GAME_CHARACTER_CLASS, None);
    register(&builder, &manager, &Rc::new(RefCell::new(chars)));
    character.borrow_mut().descriptor = None;
    (thread, character, manager, globals)
}
fn actual(
    manager: &ScriptManager,
    class: pz_inventory::ItemClass,
) -> (pz_inventory::shared::ItemRef, ScriptItemRef) {
    struct Env;
    impl pz_inventory::ItemEnv for Env {}
    let mut definition = pz_scripts::objects::items::Item::new();
    let module = manager.get_module("Base").unwrap();
    definition.set_module(&module);
    definition.init_load_pp(manager, "PoisonFixture");
    definition
        .load(manager, "PoisonFixture", "item PoisonFixture {} ")
        .unwrap();
    let definition = ScriptItemRef(Rc::new(RefCell::new(definition)));
    let mut template = pz_inventory::ItemTemplate::new(class, "Base", "PoisonFixture");
    template.script = Some(definition.clone());
    let item = pz_inventory::env::create_from_template_ref(&Env, Rc::new(template), 1).unwrap();
    (item, definition)
}
fn call(thread: &mut Thread, source: &str) -> LuaResult<Vec<Value>> {
    let function = pz_kahlua::stdlib::load_string(
        source,
        Some("native-known-poison"),
        thread.globals().clone(),
    )
    .unwrap();
    thread
        .pcall_ex(&function, &[])
        .map_err(|(_, _, error)| error)
}
fn boolean(thread: &mut Thread, source: &str) -> bool {
    match call(thread, source).unwrap().as_slice() {
        [Value::Bool(value)] => *value,
        values => panic!("boolean result: {values:?}"),
    }
}
#[test]
fn real_lua_tag_precedence_static_identity_and_nonfood_use_actual_script_fields() {
    let _restore = Restore::new();
    let (mut thread, character, manager, globals) = setup();
    let (item, definition) = actual(&manager, pz_inventory::ItemClass::Normal);
    definition.0.borrow_mut().item_tags.add(Some(
        globals.constant("item_tag", "NO_DETECT").unwrap().clone(),
    ));
    definition.0.borrow_mut().item_tags.add(Some(
        globals.constant("item_tag", "SHOW_POISON").unwrap().clone(),
    ));
    thread
        .globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    thread.globals().set_str(
        "script",
        crate::bind_scripts::script_value(Some(&pz_scripts::ScriptRef::from_shared(
            definition.0.clone(),
        ))),
    );
    let held_character = character.borrow_mut();
    assert!(!boolean(&mut thread, "return player:isKnownPoison(item)"));
    assert!(
        boolean(&mut thread, "return player:isKnownPoison(script)"),
        "script overload ignores NO_DETECT"
    );
    drop(held_character);
    globals
        .registry("item_tag")
        .unwrap()
        .borrow_mut()
        .register_id("noDetect", true)
        .unwrap_err();
    assert!(
        !boolean(&mut thread, "return player:isKnownPoison(item)"),
        "static field does not retarget duplicate registration"
    );
    assert!(boolean(
        &mut thread,
        "return ItemTag.NO_DETECT ~= ItemTag.get(ResourceLocation.of('noDetect'))"
    ));
}
#[test]
fn real_lua_food_fields_and_needed_body_failures_preserve_typed_error_and_lazy_order() {
    let _restore = Restore::new();
    let (mut thread, character, manager, _) = setup();
    let (item, _) = actual(&manager, pz_inventory::ItemClass::Food);
    thread
        .globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    call(
        &mut thread,
        "item:setPoisonPower(4); item:setPoisonDetectionLevel(-1); item:setHerbalistType(nil)",
    )
    .unwrap();
    assert!(item.borrow().mod_data_slot().is_none());
    let held_character = character.borrow_mut();
    assert!(matches!(
        call(&mut thread, "return player:isKnownPoison(item)"),
        Err(LuaError::Runtime {
            class: "pz.host.Borrowed",
            ..
        })
    ));
    assert!(item.borrow().mod_data_slot().is_none());
    drop(held_character);
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    table.set_str("addedPoisonBy", "Bob Smith");
    assert!(boolean(&mut thread, "return player:isKnownPoison(item)"));
    assert!(boolean(&mut thread, "return item:getPoisonPower()==4 and item:getPoisonDetectionLevel()==-1 and item:getHerbalistType()==nil"));
    let held_item = item.borrow_mut();
    assert!(matches!(
        call(&mut thread, "return player:isKnownPoison(item)"),
        Err(LuaError::Runtime {
            class: "pz.host.Borrowed",
            ..
        })
    ));
    assert!(matches!(
        call(&mut thread, "item:setPoisonPower(7)"),
        Err(LuaError::Runtime {
            class: "pz.host.Borrowed",
            ..
        })
    ));
    drop(held_item);
    call(&mut thread, "item:setHerbalistType('Berry')").unwrap();
    let core = crate::bind_core::shared_core();
    let held_core = core.borrow_mut();
    assert!(matches!(
        call(&mut thread, "return player:isKnownPoison(item)"),
        Err(LuaError::Runtime {
            class: "pz.host.Borrowed",
            ..
        })
    ));
    drop(held_core);
}
#[test]
fn production_reset_lua_stops_at_unported_earlier_render_prefix_without_resetting_keys() {
    let _restore = Restore::new();
    let (mut thread, _, _, globals) = setup();
    let registry = globals.registry("item_tag").unwrap();
    let key = registry
        .borrow_mut()
        .register_id("mod:retained", false)
        .unwrap();
    let core = Rc::new(RefCell::new(pz_core::core::Core::new()));
    crate::bind_core::install_core(core.clone());
    let _context = crate::bind_core::register(&mut thread, None, core.clone());
    assert!(matches!(
        call(&mut thread, "getCore():ResetLua('default','test')"),
        Err(LuaError::Runtime {
            class: "pz.host.Unsupported",
            ..
        })
    ));
    assert!(Rc::ptr_eq(
        &registry.borrow().get_id("mod:retained").unwrap().unwrap(),
        &key
    ));
    assert!(core.borrow_mut().take_requests().is_empty());
}

#[test]
fn retained_static_key_userdata_survives_environment_reset_without_strong_cache_owner() {
    let _restore = Restore::new();
    let globals = globals();
    let key = globals.constant("item_tag", "NO_DETECT").unwrap();
    let first = crate::bind_registry::key_value(crate::bind_registry::ITEM_TAG_CLASS, Some(key));
    crate::bind_registry::reset_session_handles();
    let second_manager = ScriptManager::with_native_item_key_statics(&globals);
    let same_key = second_manager
        .native_item_key_statics()
        .unwrap()
        .constant("item_tag", "NO_DETECT")
        .unwrap();
    let second =
        crate::bind_registry::key_value(crate::bind_registry::ITEM_TAG_CLASS, Some(same_key));
    assert!(first.raw_equals(&second));
    let weak = match &first {
        Value::Object(object) => Rc::downgrade(object),
        _ => unreachable!(),
    };
    drop((first, second));
    assert!(
        weak.upgrade().is_none(),
        "canonical metadata owns no key userdata"
    );
}

#[test]
fn food_primitive_setters_reject_invalid_conversion_without_mutating_actual_fields() {
    let _restore = Restore::new();
    let (mut thread, _, manager, _) = setup();
    let (item, _) = actual(&manager, pz_inventory::ItemClass::Food);
    thread
        .globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    call(
        &mut thread,
        "item:setPoisonPower(19); item:setPoisonDetectionLevel(7)",
    )
    .unwrap();
    for method in ["setPoisonPower", "setPoisonDetectionLevel"] {
        for args in ["", "nil", "'12'", "{}", "true", "1, 2"] {
            assert!(
                matches!(
                    call(&mut thread, &format!("item:{method}({args})")),
                    Err(LuaError::Runtime {
                        class: "pz.host.Unsupported",
                        ..
                    })
                ),
                "invalid conversion {method}({args})"
            );
            let body = item.borrow();
            let pz_inventory::ItemKind::Food(food) = &body.kind else {
                unreachable!()
            };
            assert_eq!((food.poison_power, food.poison_detection_level), (19, 7));
        }
    }
    call(
        &mut thread,
        "item:setHerbalistType('Current'); item:setHerbalistType(nil)",
    )
    .unwrap();
    assert!(matches!(
        call(&mut thread, "item:setHerbalistType()"),
        Err(LuaError::Runtime {
            class: "pz.host.Unsupported",
            ..
        })
    ));
    assert!(matches!(
        call(&mut thread, "return item:getPoisonPower(1)"),
        Err(LuaError::Runtime {
            class: "pz.host.Unsupported",
            ..
        })
    ));
    assert!(matches!(
        call(&mut thread, "return player:isKnownPoison()"),
        Err(LuaError::Runtime {
            class: "pz.host.Unsupported",
            ..
        })
    ));
}
