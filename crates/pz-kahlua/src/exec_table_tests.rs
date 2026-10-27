use super::*;
use crate::gc;
use crate::LuaObject;
use std::any::Any;
use std::cell::Cell;

fn thread() -> Thread {
    let mut thread = Thread::new(TableRef::new());
    crate::stdlib::install(&mut thread, None).unwrap();
    thread
}

fn function(thread: &Thread, source: &str) -> Value {
    crate::stdlib::load_string(
        source,
        Some("actual-table-access"),
        thread.globals().clone(),
    )
    .unwrap()
}

#[test]
fn actual_lua_table_operations_report_held_body_instead_of_nil_or_rust_panic() {
    let mut thread = thread();
    let table = TableRef::new();
    table.set_str("x", 7.0);
    thread.globals().set_str("actual", table.clone());
    for source in [
        "return actual.x",
        "actual.x = 9",
        "actual.y = 9",
        "return rawget(actual, 'x')",
        "rawset(actual, 'x', 9)",
        "return getmetatable(actual)",
        "setmetatable(actual, {})",
        "return #actual",
        "return tostring(actual)",
        "table.wipe(actual)",
        "return table.isempty(actual)",
        "return table.pairs(actual)",
        "return table.concat(actual)",
        "return unpack(actual)",
        "table.insert(actual, 8)",
        "table.remove(actual)",
    ] {
        let closure = function(&thread, source);
        let held = table.try_borrow_body_mut().unwrap();
        let error = thread.pcall_ex(&closure, &[]).expect_err(source);
        assert!(
            error
                .2
                .to_string()
                .contains("actual table body is borrowed"),
            "{source}: {}",
            error.2
        );
        drop(held);
        assert!(table.get_str("x").raw_equals(&Value::Num(7.0)));
    }
}

#[test]
fn meta_body_and_live_iterator_reads_are_fallible_without_freezing_snapshots() {
    let mut thread = thread();
    let table = TableRef::new();
    let meta = TableRef::new();
    table.set_metatable(Some(meta.clone()));
    thread.globals().set_str("actual", table.clone());
    let read = function(&thread, "return actual.missing");
    let held = meta.try_borrow_body_mut().unwrap();
    assert!(thread
        .pcall_ex(&read, &[])
        .unwrap_err()
        .2
        .to_string()
        .contains("actual table body is borrowed"));
    drop(held);
    table.set_str("x", 1.0);
    let iterator = thread
        .call1(&function(&thread, "return table.pairs(actual)"), &[])
        .unwrap();
    let held = table.try_borrow_body_mut().unwrap();
    assert!(thread
        .pcall_ex(&iterator, &[])
        .unwrap_err()
        .2
        .to_string()
        .contains("actual table body is borrowed"));
    drop(held);
    let fresh = thread
        .call1(&function(&thread, "return table.pairs(actual)"), &[])
        .unwrap();
    table.set_str("x", 2.0);
    let result = thread.call(&fresh, &[]).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].as_str(), Some("x"));
    assert!(result[1].raw_equals(&Value::Num(2.0)));
}

#[test]
fn lua_newindex_callback_can_reenter_actual_table_without_a_body_guard() {
    let mut thread = thread();
    let f = function(&thread, "local t = {}; setmetatable(t, {__newindex=function(self,k,v) rawset(self,k,v+1) end}); t.x=4; return t.x");
    assert!(thread.call1(&f, &[]).unwrap().raw_equals(&Value::Num(5.0)));
}

#[test]
fn native_calls_and_index_metamethods_preserve_typed_host_faults() {
    for class in ["pz.host.Borrowed", "pz.host.Unsupported"] {
        let mut thread = thread();
        let native = Value::Native(NativeFunction::new("actual host access", move |_| {
            Err(LuaError::Runtime { class, message: "needed actual owner".into() })
        }));
        let error = thread.pcall_ex(&native, &[]).unwrap_err().2;
        assert!(matches!(error, LuaError::Runtime {class: actual, message} if actual == class && message == "needed actual owner"));
        let table = TableRef::new();
        let meta = TableRef::new();
        meta.set_str("__index", native);
        table.set_metatable(Some(meta));
        thread.globals().set_str("actual", table);
        let closure = function(&thread, "return actual.missing");
        let error = thread.pcall_ex(&closure, &[]).unwrap_err().2;
        assert!(matches!(error, LuaError::Runtime {class: actual, message} if actual == class && message == "needed actual owner"));
    }
}

#[test]
fn native_java_exceptions_keep_original_direct_and_metamethod_wrapping() {
    let mut thread = thread();
    let native = Value::Native(NativeFunction::new("actual Java access", |_| {
        Err(LuaError::Runtime { class: "java.lang.IllegalArgumentException", message: "native detail".into() })
    }));
    let assert_native = |error| {
        assert!(matches!(error, LuaError::Runtime {class: "java.lang.RuntimeException", message} if message == "java.lang.IllegalArgumentException native detail in actual Java access"));
    };
    assert_native(thread.pcall_ex(&native, &[]).unwrap_err().2);
    let table = TableRef::new();
    let meta = TableRef::new();
    meta.set_str("__index", native);
    table.set_metatable(Some(meta));
    thread.globals().set_str("actual", table);
    let closure = function(&thread, "return actual.missing");
    assert_native(thread.pcall_ex(&closure, &[]).unwrap_err().2);
}

struct ReentrantDrop {
    first: Rc<RefCell<usize>>,
    second: Rc<RefCell<usize>>,
    dropped: Rc<Cell<bool>>,
}
impl LuaObject for ReentrantDrop {
    fn class_name(&self) -> &str {
        "test.RetiredTableDrop"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl Drop for ReentrantDrop {
    fn drop(&mut self) {
        *self.first.borrow_mut() += 1;
        *self.second.borrow_mut() += 1;
        self.dropped.set(true);
        let _new_table = TableRef::new();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    }
}

#[test]
fn rejected_owned_mutator_inputs_survive_until_outer_host_guards_end() {
    for operation in ["table_set", "setmetatable", "insert_at"] {
        let mut thread = thread();
        let table = TableRef::new();
        let first = Rc::new(RefCell::new(0));
        let second = Rc::new(RefCell::new(0));
        let dropped = Rc::new(Cell::new(false));
        let deferral = gc::defer_collection();
        let first_guard = first.borrow_mut();
        let second_guard = second.borrow_mut();
        let held = table.try_borrow_body_mut().unwrap();
        let owner = Value::Object(Rc::new(ReentrantDrop {
            first: first.clone(),
            second: second.clone(),
            dropped: dropped.clone(),
        }));
        let result = match operation {
            "table_set" => {
                thread.table_set(&Value::Table(table.clone()), &Value::str("rejected"), owner)
            }
            "setmetatable" => {
                let meta = TableRef::new();
                meta.set_str("owner", owner);
                crate::stdlib::setmetatable(
                    &mut thread,
                    &Value::Table(table.clone()),
                    Some(meta),
                    true,
                )
            }
            "insert_at" => crate::stdlib::insert_at(&mut thread, &table, 1, owner),
            _ => unreachable!(),
        };
        assert!(
            matches!(
                result,
                Err(LuaError::Runtime {
                    class: "pz.host.Borrowed",
                    ..
                })
            ),
            "{operation}"
        );
        assert!(!dropped.get());
        assert_eq!(gc::drain_retired_table_values(), 0);
        drop(held);
        drop(first_guard);
        drop(second_guard);
        drop(deferral);
        assert!(!dropped.get());
        assert!(gc::drain_retired_table_values() > 0);
        assert!(dropped.get());
        assert!(table.is_empty());
        assert!(matches!(
            gc::collect_pending(),
            gc::CollectionResult::Collected(_)
        ));
    }
}

#[test]
fn actual_lua_removed_owners_wait_for_both_outer_host_guards_and_explicit_boundary() {
    for source in [
        "actual.drop=nil",
        "rawset(actual, 'drop', nil)",
        "table.wipe(actual)",
        "setmetatable(actual,nil)",
    ] {
        let mut thread = thread();
        let table = TableRef::new();
        let first = Rc::new(RefCell::new(0));
        let second = Rc::new(RefCell::new(0));
        let dropped = Rc::new(Cell::new(false));
        let owner = Value::Object(Rc::new(ReentrantDrop {
            first: first.clone(),
            second: second.clone(),
            dropped: dropped.clone(),
        }));
        if source.starts_with("setmetatable") {
            let meta = TableRef::new();
            meta.set_str("drop", owner);
            table.set_metatable(Some(meta));
        } else {
            table.set_str("drop", owner);
        }
        thread.globals().set_str("actual", table.clone());
        let f = function(&thread, source);
        let deferral = gc::defer_collection();
        let first_guard = first.borrow_mut();
        let second_guard = second.borrow_mut();
        thread.call(&f, &[]).unwrap();
        assert!(!dropped.get());
        assert_eq!(gc::drain_retired_table_values(), 0);
        drop(first_guard);
        assert_eq!(gc::drain_retired_table_values(), 0);
        drop(second_guard);
        drop(deferral);
        assert!(
            !dropped.get(),
            "deferral Drop must never run final destructors"
        );
        assert!(gc::drain_retired_table_values() > 0);
        assert!(dropped.get());
        assert_eq!((*first.borrow(), *second.borrow()), (1, 1));
        assert!(matches!(
            gc::collect_pending(),
            gc::CollectionResult::Collected(_)
        ));
    }
}
