use super::*;
use std::any::Any;
use std::cell::Cell;

#[test]
fn actual_pointer_survives_held_body_but_queries_and_mutations_fail_explicitly() {
    let table = TableRef::new();
    let key = Value::str("x");
    let mut retired = RetiredTableValues::new();
    table
        .try_rawset(&key, &Value::Num(1.0), &mut retired)
        .unwrap();
    let guard = table.try_borrow_body_mut().unwrap();
    assert!(table.ptr_eq(&table.clone()));
    assert_eq!(table.try_is_empty(), Err(TableAccessError::Borrowed));
    assert!(matches!(
        table.try_get_own(&key),
        Err(TableAccessError::Borrowed)
    ));
    assert!(matches!(
        table.try_rawget(&key),
        Err(TableAccessError::Borrowed)
    ));
    assert!(matches!(
        table.try_entries(),
        Err(TableAccessError::Borrowed)
    ));
    assert_eq!(
        table.try_wipe(&mut retired),
        Err(TableAccessError::Borrowed)
    );
    assert_eq!(
        table.try_rawset(&key, &Value::Nil, &mut retired),
        Err(TableAccessError::Borrowed)
    );
    assert!(retired.is_empty());
    drop(guard);
    assert_eq!(table.try_get_own(&key).unwrap().as_num(), Some(1.0));
}

struct FinalDropProbe {
    outer: Rc<RefCell<()>>,
    dropped: Rc<Cell<bool>>,
}
impl crate::LuaObject for FinalDropProbe {
    fn class_name(&self) -> &str {
        "test.FinalDropProbe"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn to_lua_string(&self) -> String {
        panic!("retirement bookkeeping must not invoke userdata formatting")
    }
}
impl Drop for FinalDropProbe {
    fn drop(&mut self) {
        assert!(
            self.outer.try_borrow_mut().is_ok(),
            "final drop occurred under outer guard"
        );
        self.dropped.set(true);
    }
}

#[test]
fn wipe_retires_both_physical_key_and_value_and_keeps_metatable() {
    let outer = Rc::new(RefCell::new(()));
    let key_dropped = Rc::new(Cell::new(false));
    let value_dropped = Rc::new(Cell::new(false));
    let table = TableRef::new();
    let meta = TableRef::new();
    let mut retired = RetiredTableValues::new();
    table
        .try_set_metatable(&Some(meta.clone()), &mut retired)
        .unwrap();
    let key = Value::Object(Rc::new(FinalDropProbe {
        outer: outer.clone(),
        dropped: key_dropped.clone(),
    }));
    let value = Value::Object(Rc::new(FinalDropProbe {
        outer: outer.clone(),
        dropped: value_dropped.clone(),
    }));
    table.try_rawset(&key, &value, &mut retired).unwrap();
    drop(key);
    drop(value);
    let guard = outer.borrow_mut();
    table.try_wipe(&mut retired).unwrap();
    assert_eq!(retired.len(), 2);
    assert_eq!(format!("{retired:?}"), "RetiredTableValues { values: 2 }");
    assert!(table.try_is_empty().unwrap());
    assert!(table.try_metatable().unwrap().unwrap().ptr_eq(&meta));
    assert!(!key_dropped.get() && !value_dropped.get());
    drop(guard);
    drop(retired);
    assert!(key_dropped.get() && value_dropped.get());
}

#[test]
fn update_keeps_original_key_allocation_and_removal_retires_both_slots() {
    let table = TableRef::new();
    let first = Value::str("key");
    let second = Value::str("key");
    let mut retired = RetiredTableValues::new();
    table
        .try_rawset(&first, &Value::Num(1.0), &mut retired)
        .unwrap();
    table
        .try_rawset(&second, &Value::Num(2.0), &mut retired)
        .unwrap();
    let entries = table.try_entries().unwrap();
    let (Value::Str(actual), Value::Str(original)) = (&entries[0].0, &first) else {
        panic!()
    };
    assert!(Rc::ptr_eq(actual, original));
    assert_eq!(retired.len(), 1);
    table
        .try_rawset(&second, &Value::Nil, &mut retired)
        .unwrap();
    assert_eq!(retired.len(), 3);
    assert!(table.try_is_empty().unwrap());
}

#[test]
fn raw_lookup_uses_metatable_but_own_presence_does_not_and_cycles_are_explicit() {
    let table = TableRef::new();
    let meta = TableRef::new();
    let mut retired = RetiredTableValues::new();
    meta.try_rawset(&Value::str("x"), &Value::Bool(true), &mut retired)
        .unwrap();
    table
        .try_set_metatable(&Some(meta.clone()), &mut retired)
        .unwrap();
    assert!(table.try_is_empty().unwrap());
    assert!(matches!(
        table.try_rawget(&Value::str("x")).unwrap(),
        Value::Bool(true)
    ));
    assert!(table.try_get_own(&Value::str("x")).unwrap().is_nil());
    meta.try_set_metatable(&Some(table.clone()), &mut retired)
        .unwrap();
    assert!(matches!(
        table.try_rawget(&Value::str("missing")),
        Err(TableAccessError::TraversalLimit)
    ));
    meta.try_set_metatable(&None, &mut retired).unwrap();
}

#[test]
fn boxed_float_is_real_immutable_userdata_not_a_double() {
    let first = BoxedFloat::value(0.1);
    let second = BoxedFloat::value(0.1);
    assert_eq!(first.type_name(), "userdata");
    assert_eq!(first.as_num(), None);
    assert!(!first.raw_equals(&second));
    assert!(first.raw_equals(&first.clone()));
    assert!(!first.raw_equals(&Value::Num(0.1)));
    let Value::Object(actual) = first else {
        panic!()
    };
    assert_eq!(actual.class_name(), "java.lang.Float");
    assert_eq!(
        actual.as_any().downcast_ref::<BoxedFloat>().unwrap().get(),
        0.1f32
    );
    assert_eq!(actual.to_lua_string(), "0.1");
    assert_eq!(
        BoxedFloat::new(f32::from_bits(0x7fc0_0012)).canonical_bits(),
        0x7fc0_0000
    );
}

#[test]
fn boxed_float_leaf_does_not_keep_an_unrooted_table_cycle_alive() {
    let table = TableRef::new();
    let weak = Rc::downgrade(&table.0);
    let float = BoxedFloat::new(1.0);
    assert_eq!(BoxedFloat::observe(&float), Ok(false));
    table.set_str("self", Value::Table(table.clone()));
    table.set_str("float", Value::Object(float.clone()));
    drop(table);
    let result = crate::gc::collect_tables();
    assert!(
        matches!(result, crate::gc::CollectionResult::Collected(_)),
        "{result:?}"
    );
    assert!(weak.upgrade().is_none());
    assert_eq!(float.get(), 1.0);
}
