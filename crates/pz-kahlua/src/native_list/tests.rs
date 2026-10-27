use super::*;
use crate::{LuaObject, NativeFunction, TableRef};
use std::any::Any;
use std::cell::Cell;
use std::rc::Weak;

fn array(list: &NativeListRef) -> NativeArrayRef {
    list.borrow().array_slot().unwrap().clone()
}
fn fill(values: &[Value], capacity: i32, retired: &mut NativeListRetired) -> NativeListRef {
    let list = new_capacity(capacity, retired).unwrap();
    for value in values {
        add_raw(&list, value, retired).unwrap();
    }
    list
}
fn java_is(e: &ListError, name: &str) -> bool {
    matches!(e,ListError::Java {class,..} if *class==name)
}
fn drain_all() {
    for _ in 0..12 {
        if drain_retired() == 0 {
            break;
        }
    }
}

#[test]
fn two_permanent_empty_arrays_and_actual_copy_have_distinct_owners() {
    let mut retired = NativeListRetired::new();
    let default = new_default(&mut retired).unwrap();
    let explicit = new_capacity(0, &mut retired).unwrap();
    assert!(!array(&default).ptr_eq(&array(&explicit)));
    let default_id = array(&default).ptr_id();
    let empty_id = array(&explicit).ptr_id();
    let copy = to_array(&default, &mut retired).unwrap();
    assert_eq!(copy.len(), 0);
    assert!(!copy.ptr_eq(&array(&default)) && !copy.ptr_eq(&array(&explicit)));
    let copied_list = new_from_actual(&default, &mut retired).unwrap();
    assert_eq!(array(&copied_list).ptr_id(), empty_id);
    drop((default, explicit, copied_list, copy, retired));
    drain_all();
    let mut retired = NativeListRetired::new();
    assert_eq!(
        array(&new_default(&mut retired).unwrap()).ptr_id(),
        default_id
    );
    assert_eq!(
        array(&new_capacity(0, &mut retired).unwrap()).ptr_id(),
        empty_id
    );
}

#[test]
fn growth_publishes_new_array_and_keeps_old_fixed_values() {
    let mut retired = NativeListRetired::new();
    let shared = TableRef::new();
    let list = fill(&[Value::Table(shared.clone())], 1, &mut retired);
    let old = array(&list);
    let copy = new_from_actual(&list, &mut retired).unwrap();
    assert!(!array(&copy).ptr_eq(&old));
    assert!(get_raw(&copy, 0, &mut retired)
        .unwrap()
        .raw_equals(&Value::Table(shared.clone())));
    add_raw(&list, &Value::Bool(true), &mut retired).unwrap();
    assert_eq!(old.len(), 1);
    assert_eq!(array(&list).len(), 2);
    assert!(!old.ptr_eq(&array(&list)));
    assert!(old.owner.borrow().value_slots().unwrap()[0].raw_equals(&Value::Table(shared)));
    let def = new_default(&mut retired).unwrap();
    let a = array(&def);
    let _held = a.owner.borrow_mut();
    ensure_capacity(&def, 10, &mut retired).unwrap();
    assert_eq!(def.borrow().mod_count_scalar(), 0);
    // Default-empty growth reads immutable array length, not element contents.
    add_raw(&def, &Value::Nil, &mut retired).unwrap();
    assert_eq!(array(&def).len(), 10);
}

thread_local! {static FAILED_CTOR:RefCell<Option<Weak<RefCell<NativeArrayList>>>>=const {RefCell::new(None)};}
fn record_list(v: ListObserved<'_>) {
    if let ListObserved::List(list) = v {
        if list.borrow().array_slot().is_none() {
            FAILED_CTOR.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(list)));
        }
    }
}
#[test]
fn negative_capacity_keeps_raw_receiver_and_failed_prefix_until_safe_drain() {
    let mut retired = NativeListRetired::new();
    let _warm = new_default(&mut retired).unwrap();
    let previous = install_observer(Some(record_list));
    let error = match new_capacity(-7, &mut retired) {
        Err(e) => e,
        Ok(_) => panic!("negative ctor succeeded"),
    };
    install_observer(previous);
    assert!(java_is(&error, "java.lang.IllegalArgumentException"));
    let weak = FAILED_CTOR.with(|slot| slot.borrow_mut().take().unwrap());
    {
        let raw = weak.upgrade().unwrap();
        assert!(raw.borrow().array_slot().is_none());
        assert_eq!(raw.borrow().size_scalar(), 0);
    }
    let deferred = gc::defer_collection();
    drop(retired);
    assert_eq!(drain_retired(), 0);
    assert!(weak.upgrade().is_some());
    drop(deferred);
    drain_all();
    assert!(weak.upgrade().is_none());
}

#[test]
fn exact_mod_count_range_and_clear_borrow_prefixes() {
    let mut retired = NativeListRetired::new();
    let list = fill(&[Value::Num(1.0)], 2, &mut retired);
    list.borrow_mut().mod_count = i32::MAX;
    add_raw(&list, &Value::Num(2.0), &mut retired).unwrap();
    assert_eq!(list.borrow().mod_count_scalar(), i32::MIN);
    set_raw(&list, 0, &Value::Num(9.0), &mut retired).unwrap();
    assert_eq!(list.borrow().mod_count_scalar(), i32::MIN);
    assert!(matches!(
        insert_raw(&list, 3, &Value::Nil, &mut retired),
        Err(ListError::Java { .. })
    ));
    assert_eq!(list.borrow().mod_count_scalar(), i32::MIN);
    let a = array(&list);
    let held = a.owner.borrow_mut();
    assert!(matches!(
        get_raw(&list, -1, &mut retired),
        Err(ListError::Java { .. })
    ));
    assert_eq!(
        clear(&list, &mut retired),
        Err(ListError::Borrowed("Object[] element write"))
    );
    assert_eq!(list.borrow().size_scalar(), 0);
    assert_eq!(list.borrow().mod_count_scalar(), i32::MIN + 1);
    assert_eq!(held.value_slots().unwrap()[0].as_num(), Some(9.0));
}

#[test]
fn allocation_boundary_keeps_mod_increment_and_original_array() {
    let mut retired = NativeListRetired::new();
    let list = new_default(&mut retired).unwrap();
    let original = array(&list);
    assert_eq!(
        ensure_capacity(&list, MAX_HOST_CAPACITY + 1, &mut retired),
        Err(ListError::Unsupported(
            "Object[] host allocation capacity bound"
        ))
    );
    assert_eq!(list.borrow().mod_count_scalar(), 1);
    assert!(array(&list).ptr_eq(&original));
    assert_eq!(size(&list, &mut retired).unwrap(), 0);
    trim_to_size(&list, &mut retired).unwrap();
    assert_eq!(list.borrow().mod_count_scalar(), 2);
}

struct GrowDuringEquals {
    list: NativeListRef,
    seen: Vec<String>,
}
impl JavaEquals for GrowDuringEquals {
    type Error = ListError;
    fn equals(&mut self, search: &Value, stored: &Value) -> ListResult<bool> {
        assert_eq!(search.as_str(), Some("search"));
        self.seen.push(stored.as_str().unwrap().to_owned());
        if self.seen.len() == 1 {
            let mut r = NativeListRetired::new();
            add_raw(&self.list, &Value::str("third"), &mut r)?;
            set_raw(&self.list, 1, &Value::str("new array"), &mut r)?;
            gc::collect_tables(); // all captured input/local owners remain roots
            Ok(false)
        } else {
            Ok(stored.as_str() == Some("old second"))
        }
    }
}
#[test]
fn index_search_captures_old_array_across_guard_free_growth_callback() {
    let mut retired = NativeListRetired::new();
    let list = fill(
        &[Value::str("first"), Value::str("old second")],
        2,
        &mut retired,
    );
    let old = array(&list);
    let mut eq = GrowDuringEquals {
        list: list.clone(),
        seen: Vec::new(),
    };
    assert_eq!(
        index_of(&list, &Value::str("search"), &mut eq, &mut retired).unwrap(),
        1
    );
    assert_eq!(eq.seen, ["first", "old second"]);
    assert!(!old.ptr_eq(&array(&list)));
    assert_eq!(
        get_raw(&list, 1, &mut retired).unwrap().as_str(),
        Some("new array")
    );
}
struct ChangeDuringEquals {
    list: NativeListRef,
    calls: usize,
}
impl JavaEquals for ChangeDuringEquals {
    type Error = ListError;
    fn equals(&mut self, _: &Value, stored: &Value) -> ListResult<bool> {
        self.calls += 1;
        if self.calls == 1 {
            set_raw(
                &self.list,
                1,
                &Value::str("changed"),
                &mut NativeListRetired::new(),
            )?;
            Ok(false)
        } else {
            Ok(stored.as_str() == Some("changed"))
        }
    }
}
#[test]
fn captured_array_reads_later_same_array_mutation_without_snapshot() {
    let mut retired = NativeListRetired::new();
    let list = fill(&[Value::str("a"), Value::str("b")], 3, &mut retired);
    let before = array(&list);
    let mut eq = ChangeDuringEquals {
        list: list.clone(),
        calls: 0,
    };
    assert_eq!(
        index_of(&list, &Value::str("search"), &mut eq, &mut retired).unwrap(),
        1
    );
    assert!(before.ptr_eq(&array(&list)));
}
struct ClearDuringEquals {
    list: NativeListRef,
}
impl JavaEquals for ClearDuringEquals {
    type Error = ListError;
    fn equals(&mut self, _: &Value, _: &Value) -> ListResult<bool> {
        clear(&self.list, &mut NativeListRetired::new())?;
        Ok(true)
    }
}
#[test]
fn object_removal_after_reentrant_clear_preserves_native_negative_size_prefix() {
    let mut retired = NativeListRetired::new();
    let list = fill(&[Value::str("stored")], 1, &mut retired);
    let mut eq = ClearDuringEquals { list: list.clone() };
    let e = remove_object(&list, &Value::str("search"), &mut eq, &mut retired).unwrap_err();
    assert!(
        matches!(e,SearchError::List(ref e) if java_is(e,"java.lang.ArrayIndexOutOfBoundsException"))
    );
    assert_eq!(list.borrow().size_scalar(), -1);
    assert_eq!(list.borrow().mod_count_scalar(), 3);
}
struct OriginalError(Rc<Cell<usize>>);
struct FailingEquals {
    error: Rc<OriginalError>,
}
impl JavaEquals for FailingEquals {
    type Error = Rc<OriginalError>;
    fn equals(&mut self, _: &Value, _: &Value) -> Result<bool, Self::Error> {
        Err(self.error.clone())
    }
}
#[test]
fn callback_error_is_same_original_value_without_normalization() {
    let mut retired = NativeListRetired::new();
    let list = fill(&[Value::Bool(true)], 1, &mut retired);
    let original = Rc::new(OriginalError(Rc::new(Cell::new(17))));
    let mut eq = FailingEquals {
        error: original.clone(),
    };
    match contains(&list, &Value::Bool(true), &mut eq, &mut retired) {
        Err(SearchError::Equals(error)) => {
            assert!(Rc::ptr_eq(&error, &original));
            assert_eq!(error.0.get(), 17);
        }
        _ => panic!("callback error lost"),
    }
}

struct Opaque;
impl LuaObject for Opaque {
    fn class_name(&self) -> &str {
        panic!("equals must not call opaque class callback")
    }
    fn as_any(&self) -> &dyn Any {
        panic!("equals must not call virtual as_any")
    }
}
#[test]
fn intrinsic_equals_class_bits_and_unknown_same_pointer_capability() {
    let mut eq = IntrinsicEquals;
    assert!(eq
        .equals(
            &Value::Num(f64::NAN),
            &Value::Num(f64::from_bits(0x7ff8_0000_0000_1234))
        )
        .unwrap());
    assert!(!eq.equals(&Value::Num(-0.0), &Value::Num(0.0)).unwrap());
    let float = crate::table::BoxedFloat::value(-0.0);
    assert!(eq
        .equals(&float, &crate::table::BoxedFloat::value(-0.0))
        .unwrap());
    assert!(!eq.equals(&float, &Value::Num(-0.0)).unwrap());
    let unknown = Value::Object(Rc::new(Opaque));
    assert_eq!(
        eq.equals(&unknown, &unknown),
        Err(ListError::Unsupported(
            "Object.equals unknown receiver class"
        ))
    );
}

#[test]
fn contains_all_keeps_raw_float_and_empty_source_skips_held_receiver() {
    let mut retired = NativeListRetired::new();
    let float = crate::table::BoxedFloat::value(-0.0);
    let list = fill(&[float.clone(), Value::Nil], 2, &mut retired);
    let source = fill(&[float], 1, &mut retired);
    assert!(contains_all_actual(&list, &source, &mut IntrinsicEquals, &mut retired).unwrap());
    let empty = new_default(&mut retired).unwrap();
    let _held = list.borrow_mut();
    assert!(contains_all_actual(&list, &empty, &mut IntrinsicEquals, &mut retired).unwrap());
}

#[test]
fn iterator_no_failfast_hasnext_but_next_checks_mod_before_exhaustion() {
    let mut retired = NativeListRetired::new();
    let list = new_default(&mut retired).unwrap();
    let it = iterator_new(&list, &mut retired).unwrap();
    add_raw(&list, &Value::Num(1.0), &mut retired).unwrap();
    assert!(iterator_has_next(&it, &mut retired).unwrap());
    assert!(
        matches!(iterator_next_raw(&it,&mut retired),Err(ref e) if java_is(e,"java.util.ConcurrentModificationException"))
    );
    assert_eq!(it.borrow().cursor(), 0);
    let fresh = iterator_new(&list, &mut retired).unwrap();
    set_raw(&list, 0, &Value::Num(9.0), &mut retired).unwrap();
    assert_eq!(
        iterator_next_raw(&fresh, &mut retired).unwrap().as_num(),
        Some(9.0)
    );
    assert!(!iterator_has_next(&fresh, &mut retired).unwrap());
    assert!(
        matches!(iterator_next_raw(&fresh,&mut retired),Err(ref e) if java_is(e,"java.util.NoSuchElementException"))
    );
}

#[test]
fn iterator_advances_before_array_access_fault_and_raw_float_stays_actual() {
    let mut retired = NativeListRetired::new();
    let value = crate::table::BoxedFloat::value(-0.0);
    let list = fill(&[value.clone()], 1, &mut retired);
    let it = iterator_new(&list, &mut retired).unwrap();
    let a = array(&list);
    let held = a.owner.borrow_mut();
    assert!(matches!(
        iterator_next_raw(&it, &mut retired),
        Err(ListError::Borrowed("Object[] element read"))
    ));
    assert_eq!((it.borrow().cursor(), it.borrow().last_ret()), (1, 0));
    drop(held);
    let fresh = iterator_new(&list, &mut retired).unwrap();
    let got = iterator_next_raw(&fresh, &mut retired).unwrap();
    assert!(got.raw_equals(&value));
    assert_eq!(got.type_name(), "userdata");
    assert!(got.as_num().is_none());
}

#[test]
fn iterator_remove_preconditions_host_fault_and_success_current_slots() {
    let mut retired = NativeListRetired::new();
    let list = fill(&[Value::Num(1.0), Value::Num(2.0)], 2, &mut retired);
    let it = iterator_new(&list, &mut retired).unwrap();
    add_raw(&list, &Value::Num(3.0), &mut retired).unwrap();
    assert!(
        matches!(iterator_remove(&it,&mut retired),Err(ref e) if java_is(e,"java.lang.IllegalStateException"))
    );
    let it = iterator_new(&list, &mut retired).unwrap();
    iterator_next_raw(&it, &mut retired).unwrap();
    let a = array(&list);
    let held = a.owner.borrow_mut();
    assert_eq!(
        iterator_remove(&it, &mut retired),
        Err(ListError::Borrowed("Object[] element read"))
    );
    assert_eq!(it.borrow().last_ret(), 0);
    drop(held);
    iterator_remove(&it, &mut retired).unwrap();
    assert_eq!((it.borrow().cursor(), it.borrow().last_ret()), (0, -1));
    assert_eq!(
        iterator_next_raw(&it, &mut retired).unwrap().as_num(),
        Some(2.0)
    );
}

#[test]
fn add_all_self_copies_before_mutation_and_empty_source_still_increments_mod() {
    let mut retired = NativeListRetired::new();
    let list = fill(&[Value::str("a"), Value::Nil], 2, &mut retired);
    assert!(add_all_actual(&list, &list, &mut retired).unwrap());
    assert_eq!(size(&list, &mut retired).unwrap(), 4);
    assert_eq!(get_raw(&list, 2, &mut retired).unwrap().as_str(), Some("a"));
    assert!(get_raw(&list, 3, &mut retired).unwrap().is_nil());
    let empty = new_default(&mut retired).unwrap();
    let before = list.borrow().mod_count_scalar();
    assert!(!add_all_actual(&list, &empty, &mut retired).unwrap());
    assert_eq!(list.borrow().mod_count_scalar(), before + 1);
}

struct DropProbe {
    foreign: Rc<RefCell<usize>>,
    drops: Rc<Cell<usize>>,
}
impl LuaObject for DropProbe {
    fn class_name(&self) -> &str {
        "test.DropProbe"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl Drop for DropProbe {
    fn drop(&mut self) {
        *self.foreign.borrow_mut() += 1;
        self.drops.set(self.drops.get() + 1);
    }
}
#[test]
fn failed_write_and_removed_return_owner_wait_for_outer_foreign_guard() {
    drain_all();
    let foreign = Rc::new(RefCell::new(0));
    let drops = Rc::new(Cell::new(0));
    let def = gc::defer_collection();
    let held = foreign.borrow_mut();
    let list = {
        let mut r = NativeListRetired::new();
        new_default(&mut r).unwrap()
    };
    let value = Value::Object(Rc::new(DropProbe {
        foreign: foreign.clone(),
        drops: drops.clone(),
    }));
    let list_held = list.borrow_mut();
    {
        let mut r = NativeListRetired::new();
        assert!(matches!(
            add_raw(&list, &value, &mut r),
            Err(ListError::Borrowed(_))
        ));
    }
    drop(value);
    assert_eq!(drain_retired(), 0);
    assert_eq!(drops.get(), 0);
    drop(list_held);
    drop(held);
    drop(def);
    drain_all();
    assert_eq!(drops.get(), 1);
    let mut r = NativeListRetired::new();
    let value = Value::Object(Rc::new(DropProbe {
        foreign: foreign.clone(),
        drops: drops.clone(),
    }));
    add_raw(&list, &value, &mut r).unwrap();
    drop(value);
    drop(r);
    let def = gc::defer_collection();
    let held = foreign.borrow_mut();
    {
        let mut r = NativeListRetired::new();
        let removed = remove_index_raw(&list, 0, &mut r).unwrap();
        drop(removed);
    }
    assert_eq!(drain_retired(), 0);
    assert_eq!(drops.get(), 1);
    drop(held);
    drop(def);
    drain_all();
    assert_eq!(drops.get(), 2);
}

struct PanicEquals;
impl JavaEquals for PanicEquals {
    type Error = ();
    fn equals(&mut self, _: &Value, _: &Value) -> Result<bool, ()> {
        panic!("test callback unwind")
    }
}
#[test]
fn callback_unwind_schedules_input_pins_without_foreign_final_drop() {
    drain_all();
    let foreign = Rc::new(RefCell::new(0));
    let drops = Rc::new(Cell::new(0));
    let mut r = NativeListRetired::new();
    let list = fill(&[Value::Bool(true)], 1, &mut r);
    drop(r);
    drain_all();
    let def = gc::defer_collection();
    let held = foreign.borrow_mut();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut r = NativeListRetired::new();
        let search = Value::Object(Rc::new(DropProbe {
            foreign: foreign.clone(),
            drops: drops.clone(),
        }));
        let _ = index_of(&list, &search, &mut PanicEquals, &mut r);
    }));
    assert!(result.is_err());
    assert_eq!(drops.get(), 0);
    assert_eq!(drain_retired(), 0);
    drop(held);
    drop(def);
    drain_all();
    assert_eq!(drops.get(), 1);
}

struct ListCapture(NativeListRef);
impl gc::TracePayload for ListCapture {
    fn trace(&self, t: &mut gc::TraceVisitor) {
        t.owner(&self.0);
    }
}
#[test]
fn intrinsic_physical_array_list_cycle_collects_after_roots_and_retirements_end() {
    drain_all();
    let mut r = NativeListRetired::new();
    let list = new_default(&mut r).unwrap();
    let table = TableRef::new();
    table.set_str(
        "list",
        Value::Native(NativeFunction::new_traced_capture(
            "list",
            ListCapture(list.clone()),
            |_, _| Ok(0),
        )),
    );
    add_raw(&list, &Value::Table(table.clone()), &mut r).unwrap();
    let weak = Rc::downgrade(&list);
    let weak_array = Rc::downgrade(array(&list).owner_slot());
    let weak_table = Rc::downgrade(&table.0);
    drop((r, table));
    drain_all();
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert_eq!(list.borrow().size_scalar(), 1);
    drop(list);
    drain_all();
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    drain_all();
    assert!(
        weak.upgrade().is_none()
            && weak_array.upgrade().is_none()
            && weak_table.upgrade().is_none()
    );
}

#[test]
fn collector_detach_preserves_scalars_and_explicit_missing_states() {
    let mut r = NativeListRetired::new();
    let list = fill(&[Value::Bool(true)], 1, &mut r);
    let it = iterator_new(&list, &mut r).unwrap();
    let a = array(&list);
    let detached = { list.borrow_mut().take_for_gc() };
    assert_eq!(list.borrow().size_scalar(), 1);
    assert_eq!(list.borrow().mod_count_scalar(), 1);
    assert_eq!(
        get_raw(&list, 0, &mut r).err(),
        Some(ListError::Unsupported(
            "ArrayList missing actual elementData"
        ))
    );
    let detached_it = { it.borrow_mut().take_for_gc() };
    assert_eq!(it.borrow().last_ret(), -1);
    assert_eq!(
        iterator_next_raw(&it, &mut r).err(),
        Some(ListError::Unsupported("Itr detached actual list"))
    );
    let detached_array = { a.owner.borrow_mut().take_for_gc() };
    assert_eq!(a.len(), 1);
    assert_eq!(
        array_get(&a, 0, &mut r).err(),
        Some(ListError::Unsupported("Object[] detached slots"))
    );
    drop((detached, detached_it, detached_array));
}

thread_local! { static STATIC_ARRAY_OBSERVATIONS:Cell<usize>=const {Cell::new(0)}; static STATIC_LIST_OBSERVATIONS:Cell<usize>=const {Cell::new(0)}; }
fn fail_second_static_array(observed: ListObserved<'_>) {
    match observed {
        ListObserved::Array(_) => {
            let count = STATIC_ARRAY_OBSERVATIONS.with(|n| {
                n.set(n.get() + 1);
                n.get()
            });
            if count == 2 {
                EMPTY_ARRAYS.with(|arrays| {
                    let arrays = arrays
                        .try_borrow()
                        .expect("no static carrier guard crosses observer");
                    assert!(arrays.empty.is_some());
                    assert!(arrays.default.is_none());
                    assert!(arrays.state == ClassArrayState::Initializing);
                });
                panic!("injected second static-array metadata observer fault");
            }
        }
        ListObserved::List(_) => STATIC_LIST_OBSERVATIONS.with(|n| n.set(n.get() + 1)),
        ListObserved::Iterator(_) | ListObserved::ListN(_) => {}
    }
}
#[test]
fn static_empty_publication_survives_default_observer_unwind_without_retry() {
    // A fresh thread has actual never-initialized class statics. Never reset
    // permanent roots merely to manufacture a test initialization state.
    std::thread::spawn(|| {
        let previous = install_observer(Some(fail_second_static_array));
        let mut retired = NativeListRetired::new();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| new_default(&mut retired)));
        install_observer(previous);
        assert!(result.is_err());
        let empty = EMPTY_ARRAYS.with(|arrays| {
            let arrays = arrays.borrow();
            assert!(arrays.default.is_none());
            assert!(arrays.state == ClassArrayState::Failed);
            arrays.empty.as_ref().unwrap().clone()
        });
        let weak = Rc::downgrade(empty.owner_slot());
        let id = empty.ptr_id();
        drop(empty);
        assert_eq!(
            STATIC_LIST_OBSERVATIONS.with(Cell::get),
            0,
            "class bootstrap precedes raw receiver allocation"
        );
        drop(retired);
        drain_all();
        assert!(weak.upgrade().is_some());
        let mut retry = NativeListRetired::new();
        assert!(matches!(
            new_default(&mut retry),
            Err(ListError::Unsupported(
                "incomplete ArrayList class static initialization"
            ))
        ));
        assert_eq!(
            STATIC_ARRAY_OBSERVATIONS.with(Cell::get),
            2,
            "failed class state must not silently reallocate/retry"
        );
        EMPTY_ARRAYS.with(|arrays| {
            let arrays = arrays.borrow();
            assert_eq!(arrays.empty.as_ref().unwrap().ptr_id(), id);
            assert!(arrays.default.is_none());
        });
    })
    .join()
    .unwrap();
}
