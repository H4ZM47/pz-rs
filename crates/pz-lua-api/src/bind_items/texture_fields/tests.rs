use super::*;
use pz_inventory::item::ItemKind;
use pz_inventory::item_texture::CapturedScriptTextures;
use pz_inventory::{InventoryItem, ItemClass, ItemEnv, ItemTemplate};
use pz_kahlua::{LuaError, LuaObject, TableRef};
use pz_render::Texture;
use std::cell::{Cell, RefCell};
use std::rc::Weak;

struct Boundary;
impl Drop for Boundary {
    fn drop(&mut self) {
        pz_inventory::component_host::drain_retired_components();
    }
}
struct Env;
impl ItemEnv for Env {}

fn texture(name: &str) -> TextureRef {
    Rc::new(LuaTexture::new(Texture::empty(name)))
}
fn item(class: ItemClass, capture: CapturedScriptTextures) -> ItemRef {
    let mut template = ItemTemplate::new(class, "Base", "TextureFields");
    template.texture_capture = Some(capture);
    pz_inventory::env::create_from_template_ref(&Env, Rc::new(template), 1).unwrap()
}
fn thread(item: &ItemRef) -> Thread {
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    crate::bind_items::register(&mut thread);
    thread
        .globals()
        .set_str("item", crate::bind_items::try_item_value(item).unwrap());
    thread
}
fn call(thread: &mut Thread, source: &str) -> LuaResult<Vec<Value>> {
    let function = pz_kahlua::stdlib::load_string(
        source,
        Some("actual item texture fields"),
        thread.globals().clone(),
    )
    .unwrap();
    thread
        .pcall_ex(&function, &[])
        .map_err(|(_, _, error)| error)
}
fn host_class(result: LuaResult<Vec<Value>>, expected: &str) {
    assert!(matches!(result, Err(LuaError::Runtime { class, .. }) if class == expected));
}
fn food(item: &ItemRef, f: impl FnOnce(&mut pz_inventory::item::FoodData)) {
    let mut item = item.borrow_mut();
    let ItemKind::Food(food) = &mut item.kind else {
        panic!("Food fixture")
    };
    f(food);
}

#[test]
fn setters_preserve_actual_identity_with_borrowed_texture_bodies_and_accept_null() {
    let _boundary = Boundary;
    let _collection = pz_kahlua::gc::defer_collection();
    let initial = texture("same-name");
    let replacement = texture("same-name");
    let video = Rc::new(LuaTexture::with_class(
        Texture::empty("video"),
        pz_render::lua::VIDEO_TEXTURE_CLASS,
    ));
    let item = item(
        ItemClass::Normal,
        CapturedScriptTextures::new(Some(initial.clone()), Vec::new()),
    );
    let mut thread = thread(&item);
    thread
        .globals()
        .set_str("initial", Value::Object(initial.clone()));
    thread
        .globals()
        .set_str("replacement", Value::Object(replacement.clone()));
    thread
        .globals()
        .set_str("video", Value::Object(video.clone()));
    let _initial = initial.0.borrow_mut();
    let _replacement = replacement.0.borrow_mut();
    let _video = video.0.borrow_mut();
    let values = call(
        &mut thread,
        "local result = item:setTexture(replacement); \
         local same = item:getTexture() == replacement and item:getIcon() ~= initial; \
         item:setIcon(video); \
         local subclass = item:getTex() == video and item:getIcon() == video; \
         item:setTexture(nil); \
         return result == nil, same, subclass, item:getTexture() == nil, item:getIcon() == nil",
    )
    .unwrap();
    assert_eq!(values.len(), 5);
    assert!(values
        .iter()
        .all(|value| value.raw_equals(&Value::Bool(true))));
    call(&mut thread, "item:setIcon(replacement)").unwrap();
    assert!(Rc::ptr_eq(
        &pz_inventory::item_texture::get_texture_ref(&item)
            .unwrap()
            .unwrap(),
        &replacement,
    ));
    call(&mut thread, "item:setIcon(nil)").unwrap();
    assert!(pz_inventory::item_texture::get_texture_ref(&item)
        .unwrap()
        .is_none());
    assert!(!pz_inventory::moddata::has_mod_data_ref(&item).unwrap());
}

#[test]
fn base_assignment_preserves_food_special_slots_and_moddata() {
    let _boundary = Boundary;
    let _collection = pz_kahlua::gc::defer_collection();
    let initial = texture("base");
    let cooked = texture("cooked");
    let replacement = texture("replacement");
    let item = item(
        ItemClass::Food,
        CapturedScriptTextures::new(Some(initial), vec![None, Some(cooked.clone()), None]),
    );
    food(&item, |food| food.cooked = true);
    let mut thread = thread(&item);
    thread.globals().set_str("cooked", Value::Object(cooked));
    thread
        .globals()
        .set_str("replacement", Value::Object(replacement.clone()));
    let values = call(
        &mut thread,
        "item:setIcon(replacement); \
         return item:getTexture() == replacement, item:getIcon() == replacement, item:getTex() == cooked",
    )
    .unwrap();
    assert!(values
        .iter()
        .all(|value| value.raw_equals(&Value::Bool(true))));
    // A non-string override is ignored by the real getter, and assignment
    // does not create or overwrite ModData.
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    table.set_str("customInventoryIcon", Value::Bool(true));
    call(&mut thread, "item:setTexture(nil)").unwrap();
    let values = call(
        &mut thread,
        "return item:getTexture() == nil, item:getTex() == cooked, \
         item:getModData().customInventoryIcon == true",
    )
    .unwrap();
    assert!(values
        .iter()
        .all(|value| value.raw_equals(&Value::Bool(true))));
}

#[test]
fn world_texture_getter_uses_live_virtual_food_precedence_and_genuine_nulls() {
    let _boundary = Boundary;
    let _collection = pz_kahlua::gc::defer_collection();
    let mut capture = CapturedScriptTextures::new(None, Vec::new());
    capture.world_name = Some("world-base".into());
    capture.special_world_names = vec![
        "world-rotten".into(),
        "world-cooked".into(),
        "world-burnt".into(),
    ];
    let item = item(ItemClass::Food, capture);
    let mut thread = thread(&item);
    for (burnt, age, cooked, expected) in [
        (false, 0.0, false, "world-base"),
        (false, 0.0, true, "world-cooked"),
        (false, 5.0, true, "world-rotten"),
        (true, 5.0, true, "world-burnt"),
    ] {
        food(&item, |food| {
            food.burnt = burnt;
            food.age = age;
            food.off_age_max = 5;
            food.cooked = cooked;
        });
        assert!(
            call(&mut thread, "return item:getWorldTexture()").unwrap()[0]
                .raw_equals(&Value::from(expected))
        );
    }
    // A missing special name remains null even when a base name exists.
    {
        let mut body = item.borrow_mut();
        let pz_inventory::item_texture::ItemTextures::Live(live) = &mut body.textures else {
            panic!("native item fixture")
        };
        live.food_world_names[2] = None;
    }
    assert!(call(&mut thread, "return item:getWorldTexture()").unwrap()[0].is_nil());
    let empty = item_for_null();
    thread
        .globals()
        .set_str("empty", crate::bind_items::try_item_value(&empty).unwrap());
    assert!(call(&mut thread, "return empty:getWorldTexture()").unwrap()[0].is_nil());
}

fn item_for_null() -> ItemRef {
    item(
        ItemClass::Normal,
        CapturedScriptTextures::new(None, Vec::new()),
    )
}

#[test]
fn texture_frontends_preserve_typed_borrow_and_archive_failures_without_mutation() {
    let _boundary = Boundary;
    let _collection = pz_kahlua::gc::defer_collection();
    let initial = texture("initial");
    let item = item(
        ItemClass::Normal,
        CapturedScriptTextures::new(Some(initial.clone()), Vec::new()),
    );
    let mut thread = thread(&item);
    {
        let _guard = item.borrow_mut();
        for source in [
            "item:setTexture(nil)",
            "item:setIcon(nil)",
            "return item:getWorldTexture()",
        ] {
            host_class(call(&mut thread, source), "pz.host.Borrowed");
        }
    }
    assert!(Rc::ptr_eq(
        &pz_inventory::item_texture::get_texture_ref(&item)
            .unwrap()
            .unwrap(),
        &initial,
    ));
    let archive = Rc::new(RefCell::new(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Archive")),
        2,
    )));
    thread.globals().set_str(
        "archive",
        crate::bind_items::try_item_value(&archive).unwrap(),
    );
    for source in [
        "archive:setTexture(nil)",
        "archive:setIcon(nil)",
        "return archive:getWorldTexture()",
    ] {
        host_class(call(&mut thread, source), "pz.host.Unsupported");
    }
}

struct Opaque {
    drops: Rc<Cell<usize>>,
    free_at_drop: Rc<Cell<bool>>,
    item: Weak<RefCell<InventoryItem>>,
}
impl LuaObject for Opaque {
    fn class_name(&self) -> &str {
        panic!("texture conversion must not ask unknown userdata for class metadata")
    }
    fn as_any(&self) -> &dyn Any {
        panic!("texture conversion must not call virtual as_any")
    }
    fn to_lua_string(&self) -> String {
        panic!("texture conversion must not format unknown userdata")
    }
}
impl Drop for Opaque {
    fn drop(&mut self) {
        self.free_at_drop.set(
            self.item
                .upgrade()
                .is_none_or(|item| item.try_borrow_mut().is_ok()),
        );
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn invalid_receivers_arguments_and_arity_are_rejected_before_body_access() {
    let _boundary = Boundary;
    let _collection = pz_kahlua::gc::defer_collection();
    let item = item_for_null();
    let mut thread = thread(&item);
    let _guard = item.borrow_mut();
    for source in [
        "local f = item.setTexture; return f()",
        "local f = item.setIcon; return f(nil,nil)",
        "local f = item.getWorldTexture; return f({})",
        "item:setTexture()",
        "item:setIcon(nil,nil)",
        "item:getWorldTexture(nil)",
        "item:setTexture(false)",
        "item:setIcon('texture-name')",
        "item:setTexture({})",
        "item:setIcon(item)",
    ] {
        host_class(call(&mut thread, source), "pz.host.Unsupported");
    }
}

#[test]
fn rejected_texture_owner_retires_after_the_outer_item_guard() {
    let _boundary = Boundary;
    let item = item_for_null();
    let mut thread = thread(&item);
    let drops = Rc::new(Cell::new(0));
    let free = Rc::new(Cell::new(false));
    let function = pz_kahlua::stdlib::load_string(
        "local set = item.setTexture; return set(item, ...)",
        Some("rejected texture ownership"),
        thread.globals().clone(),
    )
    .unwrap();
    let collection = pz_kahlua::gc::defer_collection();
    let guard = item.borrow_mut();
    let opaque = Value::Object(Rc::new(Opaque {
        drops: drops.clone(),
        free_at_drop: free.clone(),
        item: Rc::downgrade(&item),
    }));
    let result = thread
        .pcall_ex(&function, &[opaque])
        .map_err(|(_, _, error)| error);
    host_class(result, "pz.host.Unsupported");
    assert_eq!(drops.get(), 0);
    drop(guard);
    drop(collection);
    pz_inventory::component_host::drain_retired_components();
    assert_eq!(drops.get(), 1);
    assert!(free.get());
}
