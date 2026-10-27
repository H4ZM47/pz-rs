use super::*;
use pz_inventory::{InventoryItem, ItemClass, ItemKind, ItemTemplate};
use pz_kahlua::{LuaError, LuaObject, TableRef, Thread};
use std::cell::RefCell;
use std::rc::Rc;
struct Boundary;
impl Drop for Boundary {
    fn drop(&mut self) {
        pz_inventory::component_host::drain_retired_components();
    }
}
fn setup() -> (Thread, LuaGameCharacterRef) {
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_items::register(&mut t);
    let b = crate::expose::expose_class(&mut t, super::super::ISO_GAME_CHARACTER_CLASS, None);
    crate::expose::expose_class(
        &mut t,
        super::super::ISO_PLAYER_CLASS,
        Some(super::super::ISO_GAME_CHARACTER_CLASS),
    );
    register(&b);
    let c = Rc::new(RefCell::new(super::super::LuaGameCharacter::new(
        &pz_scripts::ScriptManager::new(),
        CharacterKind::Player,
    )));
    t.globals().set_str(
        "player",
        Value::Object(Rc::new(CharacterObject::try_new(&c).unwrap())),
    );
    (t, c)
}
fn item(t: &Thread, name: &str, class: ItemClass) -> pz_inventory::shared::ItemRef {
    let item = pz_inventory::shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(class, "Base", name)),
        1,
    ))
    .unwrap();
    t.globals()
        .set_str(name, crate::bind_items::try_item_value(&item).unwrap());
    item
}
fn call(t: &mut Thread, s: &str) -> LuaResult<Vec<Value>> {
    let f =
        pz_kahlua::stdlib::load_string(s, Some("actual map flag"), t.globals().clone()).unwrap();
    t.pcall_ex(&f, &[]).map_err(|(_, _, e)| e)
}
fn cap(result: LuaResult<Vec<Value>>, operation: &str) {
    assert!(
        matches!(result,Err(LuaError::Runtime{class:"pz.host.Unsupported",message}) if message==operation)
    );
}
#[test]
fn nonmap_and_null_item_return_false_while_item_and_character_bodies_are_held() {
    let _boundary = Boundary;
    let (mut t, c) = setup();
    let item = item(&t, "ordinary", ItemClass::Normal);
    let _item = item.borrow_mut();
    let _character = c.borrow_mut();
    let values = call(
        &mut t,
        "return player:hasReadMap(ordinary),player:hasReadMap(nil)",
    )
    .unwrap();
    assert!(values.iter().all(|v| v.raw_equals(&Value::Bool(false))));
}
#[test]
fn map_stash_uses_actual_read_set_and_needed_map_id_fault_precedes_character() {
    let _boundary = Boundary;
    let (mut t, c) = setup();
    let map = item(&t, "map", ItemClass::Map);
    {
        let mut b = map.borrow_mut();
        let ItemKind::Map { map_id, .. } = &mut b.kind else {
            unreachable!()
        };
        *map_id = "ArchiveShadow".into();
    }
    c.borrow_mut()
        .read_print_media
        .insert("StashTitle".into(), ());
    c.borrow_mut().read_print_media.insert("".into(), ());
    for stash in ["StashTitle", ""] {
        map.borrow_mut().stash_map = Some(stash.into());
        assert!(call(&mut t, "return player:hasReadMap(map)").unwrap()[0]
            .raw_equals(&Value::Bool(true)));
    }
    map.borrow_mut().stash_map = Some("stashtitle".into());
    assert!(
        call(&mut t, "return player:hasReadMap(map)").unwrap()[0].raw_equals(&Value::Bool(false))
    );
    map.borrow_mut().stash_map = None;
    let character = c.borrow_mut();
    cap(
        call(&mut t, "return player:hasReadMap(map)"),
        "MapItem.getMapID actual nullable authority",
    );
    let held = map.borrow_mut();
    assert!(
        matches!(call(&mut t,"return player:hasReadMap(map)"),Err(LuaError::Runtime{class:"pz.host.Borrowed",message}) if message=="MapItem media-id stash field")
    );
    drop(held);
    drop(character);
    map.borrow_mut().stash_map = Some("StashTitle".into());
    let _held = c.borrow_mut();
    assert!(
        matches!(call(&mut t,"return player:hasReadMap(map)"),Err(LuaError::Runtime{class:"pz.host.Borrowed",message}) if message=="character readPrintMedia field")
    );
}
#[test]
fn map_frontend_never_calls_unknown_object_converters_or_reads_unused_character_body() {
    let _boundary = Boundary;
    struct Hostile;
    impl LuaObject for Hostile {
        fn class_name(&self) -> &str {
            panic!("no unknown class callback")
        }
        fn as_any(&self) -> &dyn Any {
            panic!("no virtual as_any")
        }
        fn to_lua_string(&self) -> String {
            panic!("no formatter")
        }
    }
    let (mut t, c) = setup();
    t.globals()
        .set_str("hostile", Value::Object(Rc::new(Hostile)));
    let _held = c.borrow_mut();
    cap(
        call(&mut t, "return player.hasReadMap(hostile,nil,true)"),
        "hasReadMap character receiver conversion",
    );
    cap(
        call(&mut t, "return player:hasReadMap(hostile,true)"),
        "hasReadMap argument count conversion",
    );
    cap(
        call(&mut t, "return player:hasReadMap(hostile)"),
        "read-state nullable InventoryItem argument conversion",
    );
}
