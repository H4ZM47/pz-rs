use super::*;
use pz_inventory::{shared, ItemTemplate};
use pz_kahlua::{TableRef, Thread};
use std::rc::Rc;

struct Scripts(Rc<pz_scripts::ScriptManager>);
impl Scripts {
    fn empty() -> Self {
        let old = crate::bind_registry::shared_script_manager();
        crate::bind_registry::install_script_manager(Rc::new(pz_scripts::ScriptManager::new()));
        Self(old)
    }
}
impl Drop for Scripts {
    fn drop(&mut self) {
        crate::bind_registry::install_script_manager(self.0.clone());
    }
}

fn fixture(class: ItemClass) -> ItemRef {
    shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(class, "Base", "ActualModDataTooltip")),
        1,
    ))
    .unwrap()
}

fn set(item: &ItemRef, key: &str, value: Value) {
    pz_inventory::moddata::set_mod_data_value_ref(item, key, Some(value)).unwrap();
}

#[test]
fn fishing_defaults_write_same_actual_table_before_native_missing_script_failure() {
    let _scripts = Scripts::empty();
    let item = fixture(ItemClass::Weapon);
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    let mut thread = Thread::new(TableRef::new());
    let cx = Cx::new(&mut thread, None);
    let mut layout = Layout::default();
    let error = fishing_rows(&cx, &item, &mut layout).unwrap_err();
    assert!(matches!(
        error,
        pz_kahlua::LuaError::Runtime {
            class: "java.lang.NullPointerException",
            ..
        }
    ));
    assert_eq!(
        layout.items.len(),
        2,
        "default line lookup fails after condition progress and line label"
    );
    assert_eq!(
        table
            .try_rawget(&Value::str("fishing_LineCondition"))
            .unwrap()
            .as_num(),
        Some(1.0)
    );
    assert_eq!(
        table
            .try_rawget(&Value::str("fishing_LineType"))
            .unwrap()
            .as_str(),
        Some("FishingLine")
    );
    assert!(table
        .try_rawget(&Value::str("fishing_HookType"))
        .unwrap()
        .is_nil());
    assert!(table.ptr_eq(&pz_inventory::moddata::get_mod_data_ref(&item).unwrap()));
}

#[test]
fn fishing_wrong_double_keeps_label_prefix_and_does_not_write_defaults() {
    let item = fixture(ItemClass::Weapon);
    set(&item, "fishing_LineCondition", Value::str("wrong type"));
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    let mut thread = Thread::new(TableRef::new());
    let cx = Cx::new(&mut thread, None);
    let mut layout = Layout::default();
    let error = fishing_rows(&cx, &item, &mut layout).unwrap_err();
    assert!(matches!(
        error,
        pz_kahlua::LuaError::Runtime {
            class: "java.lang.ClassCastException",
            ..
        }
    ));
    assert_eq!(layout.items.len(), 1);
    assert_eq!(
        table
            .try_rawget(&Value::str("fishing_LineCondition"))
            .unwrap()
            .as_str(),
        Some("wrong type")
    );
    assert!(table
        .try_rawget(&Value::str("fishing_LineType"))
        .unwrap()
        .is_nil());
}

#[test]
fn held_live_table_is_explicit_tooltip_fault_after_label_without_fake_absence() {
    let item = fixture(ItemClass::Weapon);
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    let mut thread = Thread::new(TableRef::new());
    let cx = Cx::new(&mut thread, None);
    let mut layout = Layout::default();
    let guard = table.try_borrow_body_mut().unwrap();
    let error = fishing_rows(&cx, &item, &mut layout).unwrap_err();
    assert!(matches!(
        error,
        pz_kahlua::LuaError::Runtime {
            class: "pz.host.Borrowed",
            ..
        }
    ));
    assert_eq!(layout.items.len(), 1);
    drop(guard);
    assert!(
        table.try_is_empty().unwrap(),
        "borrow failure must not synthesize missing-key defaults"
    );
}

#[test]
fn learned_recipe_moves_to_actual_list_and_removes_key_seen_by_retained_table() {
    let item = fixture(ItemClass::Literature);
    set(&item, "learnedRecipe", Value::str("ActualRecipe"));
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    assert_eq!(
        learned_recipes(&item).unwrap(),
        Some(vec!["ActualRecipe".into()])
    );
    assert!(table
        .try_rawget(&Value::str("learnedRecipe"))
        .unwrap()
        .is_nil());
    let body = item.borrow();
    let ItemKind::Literature(data) = &body.kind else {
        panic!("real literature body")
    };
    assert_eq!(
        data.learned_recipes.as_ref().unwrap(),
        &["ActualRecipe".to_string()]
    );
}

#[test]
fn learned_recipe_wrong_type_errors_before_publishing_list_or_removing_value() {
    let item = fixture(ItemClass::Literature);
    set(&item, "learnedRecipe", Value::Num(17.0));
    let error = learned_recipes(&item).unwrap_err();
    assert!(matches!(
        error,
        pz_kahlua::LuaError::Runtime {
            class: "java.lang.ClassCastException",
            ..
        }
    ));
    assert_eq!(
        item.borrow()
            .mod_data_value("learnedRecipe")
            .unwrap()
            .unwrap()
            .as_num(),
        Some(17.0)
    );
    let body = item.borrow();
    let ItemKind::Literature(data) = &body.kind else {
        panic!("real literature body")
    };
    assert!(data.learned_recipes.is_none());
}

#[test]
fn false_no_label_is_present_in_the_same_live_authority() {
    let item = fixture(ItemClass::Food);
    set(&item, "NoLabel", Value::Bool(false));
    assert!(mod_has(&item, "NoLabel").unwrap());
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    let mut retired = pz_kahlua::table::RetiredTableValues::new();
    table
        .try_rawset(&Value::str("NoLabel"), &Value::Nil, &mut retired)
        .unwrap();
    drop(retired);
    assert!(!mod_has(&item, "NoLabel").unwrap());
}

#[test]
fn held_nested_mod_data_table_formatter_returns_host_borrow_then_actual_type_identity() {
    let item = fixture(ItemClass::Literature);
    let nested = TableRef::new();
    nested.set_str("Type", Value::str("ActualTitle"));
    set(&item, "literatureTitle", Value::Table(nested.clone()));
    let value = mod_value(&item, "literatureTitle").unwrap().unwrap();
    let guard = nested.try_borrow_body_mut().unwrap();
    let error = super::super::mod_java_string(&value).unwrap_err();
    assert!(matches!(
        error,
        pz_kahlua::LuaError::Runtime {
            class: "pz.host.Borrowed",
            ..
        }
    ));
    drop(guard);
    assert_eq!(
        super::super::mod_java_string(&value).unwrap(),
        format!("ActualTitle 0x{}", pz_kahlua::exec::identity_hash(&value))
    );
    let retained = value.as_table().unwrap();
    assert!(retained.ptr_eq(&nested));
}

#[test]
fn opaque_mod_data_object_uses_native_formatter_without_an_extra_as_any_callback() {
    struct OpaqueStringValue;
    impl pz_kahlua::value::LuaObject for OpaqueStringValue {
        fn class_name(&self) -> &str {
            "test.OpaqueStringValue"
        }
        fn as_any(&self) -> &dyn std::any::Any {
            panic!("Native Object.toString does not invoke as_any")
        }
        fn to_lua_string(&self) -> String {
            "actual-opaque-value".into()
        }
    }
    let item = fixture(ItemClass::Literature);
    set(
        &item,
        "literatureTitle",
        Value::Object(Rc::new(OpaqueStringValue)),
    );
    let value = mod_value(&item, "literatureTitle").unwrap().unwrap();
    assert_eq!(
        super::super::mod_java_string(&value).unwrap(),
        "actual-opaque-value"
    );
}
