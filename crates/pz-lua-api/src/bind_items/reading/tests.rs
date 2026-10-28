use super::*;

use pz_inventory::item::ItemKind;
use pz_inventory::template::ScriptItemRef;
use pz_inventory::{InventoryItem, ItemClass, ItemEnv, ItemTemplate};
use pz_kahlua::{LuaError, LuaObject, TableRef};
use pz_scripts::{ScriptLoadMode, ScriptManager, ScriptObject};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

struct Boundary;
impl Drop for Boundary {
    fn drop(&mut self) {
        pz_inventory::component_host::drain_retired_components();
    }
}

struct Env;
impl ItemEnv for Env {}

fn thread() -> Thread {
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    crate::bind_items::register(&mut thread);
    thread
}

/// A preselected actual Literature factory branch with no Icon parameter.
/// This reaches nullable capture without an installed texture provider and
/// tests the instance/argument path, not script-type census.
fn literature() -> (ItemRef, ScriptItemRef, Rc<ScriptManager>) {
    let manager = Rc::new(ScriptManager::new());
    manager.enable_texture_capture();
    manager
        .parse_script(ScriptLoadMode::Init, "module Base {}")
        .unwrap();
    let mut definition = pz_scripts::objects::items::Item::new();
    definition.set_module(&manager.get_module("Base").unwrap());
    definition.init_load_pp(&manager, "Reading");
    definition
        .load(&manager, "Reading", "item Reading {}")
        .unwrap();
    assert!(definition.texture_capture_complete());
    definition.skill_trained = Some("Carpentry".into());
    definition.lvl_skill_trained = 2;
    definition.num_levels_trained = 3;
    definition.number_of_pages = 240;
    let definition = ScriptItemRef(Rc::new(RefCell::new(definition)));
    let mut template = ItemTemplate::new(ItemClass::Literature, "Base", "Reading");
    template.script = Some(definition.clone());
    // Deliberately different captured values cannot back runtime getters.
    template.number_of_pages = 999;
    template.skill_trained = Some("Snapshot".into());
    template.lvl_skill_trained = 40;
    template.num_levels_trained = 10;
    let item = pz_inventory::env::create_from_template_ref(&Env, Rc::new(template), 1).unwrap();
    (item, definition, manager)
}

fn publish(thread: &Thread, name: &str, item: &ItemRef) {
    thread
        .globals()
        .set_str(name, crate::bind_items::try_item_value(item).unwrap());
}

fn call(thread: &mut Thread, source: &str) -> LuaResult<Vec<Value>> {
    let function = pz_kahlua::stdlib::load_string(
        source,
        Some("actual-literature-reading"),
        thread.globals().clone(),
    )
    .unwrap();
    thread
        .pcall_ex(&function, &[])
        .map_err(|(_, _, error)| error)
}

fn field(item: &ItemRef, edit: impl FnOnce(&mut pz_inventory::item::LiteratureData)) {
    let mut body = item.borrow_mut();
    let ItemKind::Literature(data) = &mut body.kind else {
        panic!("actual Literature fixture")
    };
    edit(data);
}

fn host_class(result: LuaResult<Vec<Value>>, class: &'static str) {
    assert!(
        matches!(&result, Err(LuaError::Runtime { class: actual, .. }) if *actual == class),
        "expected {class}, got {result:?}"
    );
}

fn capability(result: LuaResult<Vec<Value>>, operation: &str) {
    assert!(
        matches!(&result, Err(LuaError::Runtime { class: "pz.host.Unsupported", message }) if message == operation),
        "expected capability {operation}, got {result:?}"
    );
}

#[test]
fn real_lua_reading_getters_use_runtime_fields_and_wrapping_formula_not_script_snapshot() {
    let _boundary = Boundary;
    let mut thread = thread();
    let (item, definition, _manager) = literature();
    publish(&thread, "book", &item);
    {
        let _collection = pz_kahlua::gc::defer_collection();
        let mut script = definition.0.borrow_mut();
        assert_eq!(script.max_level_trained(), 5);
        script.skill_trained = Some("ChangedScript".into());
        script.lvl_skill_trained = 99;
        script.num_levels_trained = 20;
        script.number_of_pages = 600;
        let values = call(&mut thread, "return book:getSkillTrained(), book:getLvlSkillTrained(), book:getMaxLevelTrained(), book:getNumLevelsTrained(), book:getNumberOfPages(), book.getMinLevelTrained, book.getNumberPages").unwrap();
        assert_eq!(values[0].as_str(), Some("Carpentry"));
        for (value, expected) in values[1..5].iter().zip([2.0, 4.0, 3.0, 240.0]) {
            assert_eq!(value.as_num(), Some(expected));
        }
        assert!(values[5].is_nil() && values[6].is_nil());
    }
    field(&item, |data| {
        data.skill_trained = None;
        data.lvl_skill_trained = -1;
        data.num_levels_trained = 0;
        data.number_of_pages = 31;
    });
    let values = call(
        &mut thread,
        "return book:getSkillTrained(), book:getMaxLevelTrained(), book:getNumberOfPages()",
    )
    .unwrap();
    assert!(values[0].is_nil());
    assert_eq!(values[1].as_num(), Some(-2.0));
    assert_eq!(values[2].as_num(), Some(31.0));
    field(&item, |data| {
        data.lvl_skill_trained = i32::MAX;
        data.num_levels_trained = 2;
    });
    assert_eq!(
        call(&mut thread, "return book:getMaxLevelTrained()").unwrap()[0].as_num(),
        Some(f64::from(i32::MIN))
    );
}

#[test]
fn real_lua_is_literature_uses_immutable_class_while_needed_runtime_body_faults() {
    let _boundary = Boundary;
    let mut thread = thread();
    let (item, _, _manager) = literature();
    let ordinary = pz_inventory::shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Ordinary")),
        2,
    ))
    .unwrap();
    publish(&thread, "book", &item);
    publish(&thread, "ordinary", &ordinary);
    let _collection = pz_kahlua::gc::defer_collection();
    let _book_guard = item.borrow_mut();
    let _ordinary_guard = ordinary.borrow_mut();
    let values = call(
        &mut thread,
        "return book:IsLiterature(), ordinary:IsLiterature(), book.getSkillTrained",
    )
    .unwrap();
    assert!(matches!(values[0], Value::Bool(true)));
    assert!(matches!(values[1], Value::Bool(false)));
    assert!(values[2].is_function());
    host_class(
        call(&mut thread, "return book:getSkillTrained()"),
        "pz.host.Borrowed",
    );
    host_class(
        call(&mut thread, "return book:getNumberOfPages()"),
        "pz.host.Borrowed",
    );
    host_class(
        call(
            &mut thread,
            "local get = book.getNumberOfPages; return get(ordinary)",
        ),
        "pz.host.Unsupported",
    );
    host_class(
        call(&mut thread, "return book:getMaxLevelTrained(nil)"),
        "pz.host.Unsupported",
    );
}

struct Opaque {
    drops: Rc<Cell<usize>>,
    body_free_at_drop: Rc<Cell<bool>>,
    item: Weak<RefCell<InventoryItem>>,
}
impl LuaObject for Opaque {
    fn class_name(&self) -> &str {
        "fixture.OpaqueReadingArgument"
    }
    fn as_any(&self) -> &dyn Any {
        panic!("argument conversion must not call a virtual as_any")
    }
    fn to_lua_string(&self) -> String {
        panic!("argument conversion must not format unknown userdata")
    }
}
impl Drop for Opaque {
    fn drop(&mut self) {
        self.body_free_at_drop
            .set(self.item.upgrade().is_none_or(|item| {
                let available = item.try_borrow_mut().is_ok();
                available
            }));
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn reading_argument_faults_are_typed_and_rejected_owners_retire_after_outer_body_guard() {
    let _boundary = Boundary;
    let mut thread = thread();
    let (item, _, _manager) = literature();
    publish(&thread, "book", &item);
    for source in [
        "local f = book.getSkillTrained; return f()",
        "local f = book.getNumberOfPages; return f(nil)",
        "local f = book.getNumLevelsTrained; return f({})",
        "return book:getSkillTrained(1)",
        "return book:getLearnedRecipes(nil)",
        "return book:IsLiterature(nil)",
    ] {
        host_class(call(&mut thread, source), "pz.host.Unsupported");
    }
    assert!(item.borrow().mod_data_slot().is_none());
    let drops = Rc::new(Cell::new(0));
    let body_free = Rc::new(Cell::new(false));
    let opaque = Value::Object(Rc::new(Opaque {
        drops: drops.clone(),
        body_free_at_drop: body_free.clone(),
        item: Rc::downgrade(&item),
    }));
    let function = pz_kahlua::stdlib::load_string(
        "local get = book.getSkillTrained; return get(...)",
        Some("reading-rejected-argument"),
        thread.globals().clone(),
    )
    .unwrap();
    let collection = pz_kahlua::gc::defer_collection();
    let guard = item.borrow_mut();
    let result = thread
        .pcall_ex(&function, &[opaque])
        .map_err(|(_, _, error)| error);
    host_class(result, "pz.host.Unsupported");
    assert_eq!(drops.get(), 0);
    drop(guard);
    drop(collection);
    pz_inventory::component_host::drain_retired_components();
    assert_eq!(drops.get(), 1);
    assert!(body_free.get());
}

#[test]
fn learned_recipes_null_is_genuine_and_needed_table_or_script_borrows_remain_visible() {
    let _boundary = Boundary;
    let mut thread = thread();
    let (item, definition, _manager) = literature();
    publish(&thread, "book", &item);
    assert!(item.borrow().mod_data_slot().is_none());
    let values = call(&mut thread, "return book:getLearnedRecipes()").unwrap();
    assert_eq!(values.len(), 1);
    assert!(values[0].is_nil());
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    assert!(table.is_empty());
    thread.globals().set_str("retained", table.clone());
    assert!(matches!(
        call(&mut thread, "return book:getModData() == retained").unwrap()[0],
        Value::Bool(true)
    ));
    {
        let _collection = pz_kahlua::gc::defer_collection();
        let _guard = table.try_borrow_body_mut().unwrap();
        host_class(
            call(&mut thread, "return book:getLearnedRecipes()"),
            "pz.host.Borrowed",
        );
    }
    {
        let _collection = pz_kahlua::gc::defer_collection();
        let _guard = definition.0.borrow_mut();
        host_class(
            call(&mut thread, "return book:getLearnedRecipes()"),
            "pz.host.Borrowed",
        );
    }
    assert!(call(&mut thread, "return book:getLearnedRecipes()").unwrap()[0].is_nil());
    assert!(pz_inventory::moddata::get_mod_data_ref(&item)
        .unwrap()
        .ptr_eq(&table));
}

#[test]
fn learned_recipe_key_stops_at_native_list_allocation_before_override_script_or_key_removal() {
    let _boundary = Boundary;
    let mut thread = thread();
    let (item, definition, _manager) = literature();
    publish(&thread, "book", &item);
    field(&item, |data| {
        data.learned_recipes = Some(vec!["OldOverride".into()])
    });
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    let nested = TableRef::new();
    for value in [
        Value::str("NewRecipe"),
        Value::Num(3.0),
        Value::Table(nested),
    ] {
        pz_inventory::moddata::set_mod_data_value_ref(&item, "learnedRecipe", Some(value.clone()))
            .unwrap();
        let _collection = pz_kahlua::gc::defer_collection();
        let _script_guard = definition.0.borrow_mut();
        capability(
            call(&mut thread, "return book:getLearnedRecipes()"),
            "Literature.getLearnedRecipes ArrayList allocation",
        );
        assert!(table
            .try_rawget(&Value::str("learnedRecipe"))
            .unwrap()
            .raw_equals(&value));
        field(&item, |data| {
            assert_eq!(
                data.learned_recipes.as_deref(),
                Some(&[String::from("OldOverride")][..])
            );
        });
    }
    pz_inventory::moddata::set_mod_data_value_ref(&item, "learnedRecipe", None).unwrap();
    let _collection = pz_kahlua::gc::defer_collection();
    let _script_guard = definition.0.borrow_mut();
    capability(
        call(&mut thread, "return book:getLearnedRecipes()"),
        "Literature.getLearnedRecipes actual override List return",
    );
}

#[test]
fn learned_recipes_script_list_and_null_script_are_capabilities_after_actual_table_prefix() {
    let _boundary = Boundary;
    let mut thread = thread();
    let (item, definition, _manager) = literature();
    publish(&thread, "book", &item);
    definition.0.borrow_mut().learned_recipes = Some(vec!["ScriptRecipe".into()]);
    capability(
        call(&mut thread, "return book:getLearnedRecipes()"),
        "Literature.getLearnedRecipes actual script List return",
    );
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    assert!(table.is_empty());
    field(&item, |data| assert!(data.learned_recipes.is_none()));
    // Set up the actual nullable slot; the factory's setter is crate-private.
    // Keep the displaced reference until the item body guard has ended.
    let old_script = {
        let mut body = item.borrow_mut();
        let pz_inventory::item_texture::ItemTextures::Live(live) = &mut body.textures else {
            panic!("actual captured Literature fixture")
        };
        live.script_item.take()
    };
    drop(old_script);
    pz_inventory::shared::observe_item(&item);
    capability(
        call(&mut thread, "return book:getLearnedRecipes()"),
        "Literature.getLearnedRecipes null scriptItem receiver",
    );
    assert!(pz_inventory::moddata::get_mod_data_ref(&item)
        .unwrap()
        .ptr_eq(&table));
}
