use super::*;
use pz_inventory::template::ScriptItemRef;
use pz_inventory::{InventoryItem, ItemClass, ItemEnv, ItemTemplate};
use pz_kahlua::{LuaError, LuaObject, TableRef};
use pz_scripts::{ScriptLoadMode, ScriptManager, ScriptObject};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
struct Boundary;
impl Drop for Boundary {
    fn drop(&mut self) {
        pz_inventory::component_host::drain_retired_components();
    }
}
struct Env;
impl ItemEnv for Env {}
fn item() -> (ItemRef, ScriptItemRef, Rc<ScriptManager>) {
    let manager = Rc::new(ScriptManager::new());
    manager.enable_texture_capture();
    manager
        .parse_script(ScriptLoadMode::Init, "module Base {}")
        .unwrap();
    let mut s = pz_scripts::objects::items::Item::new();
    s.set_module(&manager.get_module("Base").unwrap());
    s.init_load_pp(&manager, "Flags");
    s.load(&manager, "Flags", "item Flags {}").unwrap();
    assert!(s.texture_capture_complete());
    let s = ScriptItemRef(Rc::new(RefCell::new(s)));
    let mut t = ItemTemplate::new(ItemClass::Normal, "Base", "Flags");
    t.script = Some(s.clone());
    let item = pz_inventory::env::create_from_template_ref(&Env, Rc::new(t), 1).unwrap();
    (item, s, manager)
}
fn character(
    kind: pz_characters::CharacterKind,
) -> (crate::bind_character_sim::LuaGameCharacterRef, Value) {
    let c = Rc::new(RefCell::new(
        crate::bind_character_sim::LuaGameCharacter::new(&ScriptManager::new(), kind),
    ));
    let value = Value::Object(Rc::new(
        crate::bind_character_sim::CharacterObject::try_new(&c).unwrap(),
    ));
    (c, value)
}
fn setup(item: &ItemRef) -> Thread {
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_items::register(&mut t);
    t.globals()
        .set_str("item", crate::bind_items::try_item_value(item).unwrap());
    t
}
fn call(t: &mut Thread, source: &str) -> LuaResult<Vec<Value>> {
    let f = pz_kahlua::stdlib::load_string(source, Some("actual row flags"), t.globals().clone())
        .unwrap();
    t.pcall_ex(&f, &[]).map_err(|(_, _, e)| e)
}
fn cap(result: LuaResult<Vec<Value>>, message: &str) {
    assert!(
        matches!(result,Err(LuaError::Runtime{class:"pz.host.Unsupported",message:actual}) if actual==message)
    );
}
#[test]
fn actual_lua_negative_index_ignores_busy_player_script_and_accepts_animal_subclass() {
    let _boundary = Boundary;
    let (item, script, _manager) = item();
    let mut t = setup(&item);
    let (player, pv) = character(pz_characters::CharacterKind::Player);
    let (animal, av) = character(pz_characters::CharacterKind::Animal);
    t.globals().set_str("player", pv);
    t.globals().set_str("animal", av);
    let _p = player.borrow_mut();
    let _a = animal.borrow_mut();
    let _s = script.0.borrow_mut();
    let values=call(&mut t,"return item:hasBeenSeen(player),item:hasBeenHeard(player),item:hasBeenSeen(nil),item:hasBeenHeard(animal)").unwrap();
    assert_eq!(values.len(), 4);
    assert!(values.iter().all(|v| v.raw_equals(&Value::Bool(false))));
}
#[test]
fn actual_lua_positive_category_stops_at_real_needed_service_and_item_borrow_stays_typed() {
    let _boundary = Boundary;
    let (item, script, _manager) = item();
    let mut t = setup(&item);
    item.borrow_mut().recorded_media_index = 0;
    script.0.borrow_mut().recorded_media_cat = Some("Retail-VHS".into());
    cap(
        call(&mut t, "return item:hasBeenSeen(nil)"),
        "InventoryItem.hasBeenSeen actual ZomboidRadio context",
    );
    assert!(
        call(&mut t, "return item:hasBeenHeard(nil)").unwrap()[0].raw_equals(&Value::Bool(false))
    );
    script.0.borrow_mut().recorded_media_cat = Some("Audio".into());
    assert!(
        call(&mut t, "return item:hasBeenSeen(nil)").unwrap()[0].raw_equals(&Value::Bool(false))
    );
    cap(
        call(&mut t, "return item:hasBeenHeard(nil)"),
        "InventoryItem.hasBeenHeard actual ZomboidRadio context",
    );
    let _held = item.borrow_mut();
    assert!(matches!(
        call(&mut t, "return item:hasBeenSeen(nil)"),
        Err(LuaError::Runtime {
            class: "pz.host.Borrowed",
            ..
        })
    ));
}
struct Hostile;
impl LuaObject for Hostile {
    fn class_name(&self) -> &str {
        panic!("unknown class callback")
    }
    fn as_any(&self) -> &dyn Any {
        panic!("unknown as_any callback")
    }
    fn to_lua_string(&self) -> String {
        panic!("unknown formatter")
    }
}
#[test]
fn instance_receiver_then_arity_then_player_conversion_precedes_any_item_body() {
    let _boundary = Boundary;
    let (item, _script, _manager) = item();
    let mut t = setup(&item);
    t.globals()
        .set_str("hostile", Value::Object(Rc::new(Hostile)));
    let (_z, z) = character(pz_characters::CharacterKind::Zombie);
    t.globals().set_str("zombie", z);
    let held = item.borrow_mut();
    cap(
        call(&mut t, "return item.hasBeenSeen(nil,hostile,true)"),
        "media read InventoryItem receiver conversion",
    );
    cap(
        call(&mut t, "return item:hasBeenSeen(hostile,true)"),
        "media read argument count conversion",
    );
    cap(
        call(&mut t, "return item:hasBeenSeen()"),
        "media read argument count conversion",
    );
    cap(
        call(&mut t, "return item:hasBeenSeen(hostile)"),
        "media read nullable IsoPlayer argument conversion",
    );
    cap(
        call(&mut t, "return item:hasBeenHeard(zombie)"),
        "media read nullable IsoPlayer argument conversion",
    );
    drop(held);
}
#[test]
fn rejected_last_owned_argument_stays_pinned_until_outside_foreign_guard() {
    let _boundary = Boundary;
    struct Probe {
        outer: Rc<RefCell<()>>,
        safe: Rc<Cell<bool>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.safe.set(self.outer.try_borrow_mut().is_ok());
        }
    }
    impl LuaObject for Probe {
        fn class_name(&self) -> &str {
            panic!("no class conversion callback")
        }
        fn as_any(&self) -> &dyn Any {
            panic!("no virtual as_any")
        }
    }
    let (item, _script, _manager) = item();
    let mut t = setup(&item);
    let outer = Rc::new(RefCell::new(()));
    let safe = Rc::new(Cell::new(false));
    let probe = Rc::new(Probe {
        outer: outer.clone(),
        safe: safe.clone(),
    });
    let weak = Rc::downgrade(&probe);
    let args = vec![
        crate::bind_items::try_item_value(&item).unwrap(),
        Value::Object(probe),
    ];
    let function = pz_kahlua::stdlib::load_string(
        "local self, player = ...; return self:hasBeenSeen(player)",
        Some("rejected row input"),
        t.globals().clone(),
    )
    .unwrap();
    // The real outer host boundary defers automatic VM/retirement drains
    // while its foreign guard is live, just as WORLD entrypoints do.
    let collection = pz_kahlua::gc::defer_collection();
    let held = outer.borrow_mut();
    assert!(matches!(
        t.pcall_ex(&function, &args),
        Err((
            _,
            _,
            LuaError::Runtime {
                class: "pz.host.Unsupported",
                ..
            }
        ))
    ));
    drop(args);
    assert!(weak.upgrade().is_some() && !safe.get());
    drop(held);
    drop(collection);
    pz_inventory::component_host::drain_retired_components();
    assert!(weak.upgrade().is_none() && safe.get());
}
