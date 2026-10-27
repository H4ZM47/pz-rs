use super::*;
use std::cell::Cell;
use std::rc::Weak;

fn drain_all() {
    for _ in 0..16 {
        if drain_retired() == 0 {
            break;
        }
    }
}
fn install(retired: &mut NativeListRetired) -> NativeListNRef {
    install_selected_ready_empty_list_role(retired).unwrap();
    list_of_empty_ready(retired).unwrap()
}
fn array(list: &NativeListRef) -> NativeArrayRef {
    list.borrow().array_slot().unwrap().clone()
}

#[test]
fn absent_ready_profile_is_explicit_and_getter_never_allocates_or_installs() {
    let mut retired = NativeListRetired::new();
    assert!(matches!(
        list_of_empty_ready(&mut retired),
        Err(ListError::Unsupported(
            "selected READY List.of EMPTY_LIST not installed"
        ))
    ));
    READY_EMPTY_LIST.with(|role| {
        let role = role.borrow();
        assert!(role.state == ReadyState::Uninstalled && role.list.is_none());
    });
    let target = allocate_for_collection(&mut retired).unwrap();
    assert!(target.receiver_slot().borrow().array_slot().is_none());
    assert!(matches!(
        list_of_empty_ready(&mut retired),
        Err(ListError::Unsupported(
            "selected READY List.of EMPTY_LIST not installed"
        ))
    ));
    READY_EMPTY_LIST.with(|role| assert!(role.borrow().list.is_none()));
}

thread_local! { static EVENTS:RefCell<Vec<&'static str>>=const {RefCell::new(Vec::new())}; }
fn record_install(observed: ListObserved<'_>) {
    let event = match observed {
        ListObserved::ListN(list) => {
            let body = list.try_borrow().unwrap();
            assert!(!body.allow_nulls_scalar());
            READY_EMPTY_LIST.with(|role| {
                let role = role
                    .try_borrow()
                    .expect("no role-carrier guard spans metadata observer");
                assert!(role.state == ReadyState::Installing && role.list.is_none());
            });
            if body.elements_slot().is_none() {
                "raw ListN"
            } else {
                "completed ListN"
            }
        }
        ListObserved::Array(array) => {
            assert_eq!(array.len(), 0);
            "zero array"
        }
        _ => panic!("no mutable list/iterator allocation during selected role install"),
    };
    EVENTS.with(|events| events.borrow_mut().push(event));
}
#[test]
fn explicit_install_observes_raw_listn_then_array_then_complete_before_static_publication() {
    let mut retired = NativeListRetired::new();
    let old = install_observer(Some(record_install));
    install_selected_ready_empty_list_role(&mut retired).unwrap();
    install_observer(old);
    assert_eq!(
        EVENTS.with(|events| events.borrow().clone()),
        ["raw ListN", "zero array", "completed ListN"]
    );
    let first = list_of_empty_ready(&mut retired).unwrap();
    let weak = Rc::downgrade(&first);
    let id = Rc::as_ptr(&first);
    install_selected_ready_empty_list_role(&mut retired).unwrap();
    assert!(Rc::ptr_eq(
        &first,
        &list_of_empty_ready(&mut retired).unwrap()
    ));
    drop((first, retired));
    drain_all();
    assert_eq!(
        Rc::as_ptr(&weak.upgrade().unwrap()),
        id,
        "permanent class role survives transient owner drains"
    );
}

#[test]
fn source_toarray_and_same_target_constructor_keep_all_actual_array_roles_distinct() {
    let mut retired = NativeListRetired::new();
    let source = install(&mut retired);
    let backing = source.borrow().elements_slot().unwrap().clone();
    let default = new_default(&mut retired).unwrap();
    let capacity_zero = new_capacity(0, &mut retired).unwrap();
    let first = list_n_empty_to_array(&source, &mut retired).unwrap();
    let second = list_n_empty_to_array(&source, &mut retired).unwrap();
    let roles = [
        backing.clone(),
        array(&default),
        array(&capacity_zero),
        first,
        second,
    ];
    for i in 0..roles.len() {
        assert_eq!(roles[i].len(), 0);
        for j in 0..i {
            assert!(!roles[i].ptr_eq(&roles[j]));
        }
    }
    let target = allocate_for_collection(&mut retired).unwrap();
    let actual = target.receiver_slot().clone();
    actual.borrow_mut().mod_count = 17; // prove the later actual superclass write, not allocation's zero default
    let completed = target
        .initialize(KnownCollectionRef::EmptyReadyListN(&source), &mut retired)
        .unwrap();
    assert!(Rc::ptr_eq(&actual, &completed));
    assert_eq!(
        (
            completed.borrow().size_scalar(),
            completed.borrow().mod_count_scalar()
        ),
        (0, 0)
    );
    assert!(
        array(&completed).ptr_eq(&array(&capacity_zero)) && !array(&completed).ptr_eq(&backing)
    );
    add_raw(&completed, &Value::str("Recipe"), &mut retired).unwrap();
    assert_eq!(
        array(&completed).len(),
        1,
        "Collection-empty backing grows differently from default capacity10"
    );
    assert_eq!(
        get_raw(&completed, 0, &mut retired).unwrap().as_str(),
        Some("Recipe")
    );
}

#[test]
fn singleton_pointer_is_readable_under_body_guard_and_zero_copy_needs_no_element_body() {
    let mut retired = NativeListRetired::new();
    let source = install(&mut retired);
    let body = source.borrow_mut();
    assert!(Rc::ptr_eq(
        &source,
        &list_of_empty_ready(&mut retired).unwrap()
    ));
    assert_eq!(observe_list_n(&source), Ok(ObservationStatus::Deferred));
    assert_eq!(
        list_n_empty_to_array(&source, &mut retired).err(),
        Some(ListError::Borrowed("ListN toArray first elements field"))
    );
    let backing = body.elements_slot().unwrap().clone();
    drop(body);
    let _held_elements = backing.owner_slot().borrow_mut();
    let result = list_n_empty_to_array(&source, &mut retired).unwrap();
    assert_eq!(result.len(), 0);
    assert!(!result.ptr_eq(&backing));
    assert_eq!(retry_observation(), Ok(ObservationStatus::Complete));
}

#[test]
fn constructor_superclass_write_precedes_source_body_and_source_failure_keeps_raw_prefix() {
    let mut retired = NativeListRetired::new();
    let source = install(&mut retired);
    let target = allocate_for_collection(&mut retired).unwrap();
    let actual = target.receiver_slot().clone();
    actual.borrow_mut().mod_count = 23;
    let target_held = actual.borrow_mut();
    let source_held = source.borrow_mut();
    let result = target.initialize(KnownCollectionRef::EmptyReadyListN(&source), &mut retired);
    assert_eq!(
        result.err(),
        Some(ListError::Borrowed(
            "ArrayList superclass modCount initializer"
        ))
    );
    assert_eq!(target_held.mod_count_scalar(), 23);
    assert!(target_held.array_slot().is_none());
    drop(target_held);
    let target = allocate_for_collection(&mut retired).unwrap();
    let actual = target.receiver_slot().clone();
    actual.borrow_mut().mod_count = 29;
    let result = target.initialize(KnownCollectionRef::EmptyReadyListN(&source), &mut retired);
    assert_eq!(
        result.err(),
        Some(ListError::Borrowed("ListN toArray first elements field"))
    );
    assert_eq!(actual.borrow().mod_count_scalar(), 0);
    assert_eq!(actual.borrow().size_scalar(), 0);
    assert!(actual.borrow().array_slot().is_none());
    drop(source_held);
}

struct SourceFailure(Rc<Cell<usize>>);
#[test]
fn raw_target_survives_original_source_resolution_error_until_safe_outer_drain() {
    drain_all();
    let deferred = gc::defer_collection();
    let mut retired = NativeListRetired::new();
    let target = allocate_for_collection(&mut retired).unwrap();
    let weak = Rc::downgrade(target.receiver_slot());
    let original = Rc::new(SourceFailure(Rc::new(Cell::new(31))));
    let observed: Result<(), Rc<SourceFailure>> = (|| {
        // Represents argument evaluation AFTER raw NEW. This original error is
        // outer-caller-owned; the low token never formats or substitutes it.
        let result: Result<NativeListNRef, Rc<SourceFailure>> = Err(original.clone());
        let source = result?;
        let _ = target
            .initialize(KnownCollectionRef::EmptyReadyListN(&source), &mut retired)
            .unwrap();
        Ok(())
    })();
    match observed {
        Err(error) => {
            assert!(Rc::ptr_eq(&error, &original));
            assert_eq!(error.0.get(), 31);
        }
        _ => panic!("source error lost"),
    }
    {
        let actual = weak.upgrade().unwrap();
        assert!(actual.borrow().array_slot().is_none());
        assert_eq!(actual.borrow().mod_count_scalar(), 0);
    }
    drop(retired);
    assert_eq!(drain_retired(), 0);
    assert!(weak.upgrade().is_some());
    drop(deferred);
    drain_all();
    assert!(weak.upgrade().is_none());
}

thread_local! {static FAILED_ROLE:RefCell<Option<Weak<RefCell<NativeListN>>>>=const {RefCell::new(None)};}
fn fail_completed_role(observed: ListObserved<'_>) {
    if let ListObserved::ListN(list) = observed {
        if list.borrow().elements_slot().is_some() {
            FAILED_ROLE.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(list)));
            panic!("injected selected READY role metadata unwind after constructor");
        }
    }
}
#[test]
fn failed_role_install_preserves_native_receiver_prefix_without_false_ready_or_retry() {
    std::thread::spawn(|| {
        let mut retired = NativeListRetired::new();
        let old = install_observer(Some(fail_completed_role));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            install_selected_ready_empty_list_role(&mut retired)
        }));
        install_observer(old);
        assert!(result.is_err());
        READY_EMPTY_LIST.with(|role| {
            let role = role.borrow();
            assert!(role.state == ReadyState::Failed && role.list.is_none());
        });
        let weak = FAILED_ROLE.with(|slot| slot.borrow_mut().take().unwrap());
        {
            let node = weak.upgrade().unwrap();
            let body = node.borrow();
            assert_eq!(body.elements_slot().unwrap().len(), 0);
            assert!(!body.allow_nulls_scalar());
        }
        assert_eq!(
            install_selected_ready_empty_list_role(&mut retired),
            Err(ListError::Unsupported(
                "failed selected READY List.of role installation"
            ))
        );
        assert!(matches!(
            list_of_empty_ready(&mut retired),
            Err(ListError::Unsupported(
                "failed selected READY List.of role installation"
            ))
        ));
        drop(retired);
        drain_all();
        assert!(weak.upgrade().is_none());
    })
    .join()
    .unwrap();
}

#[test]
fn listn_descriptor_is_intrinsic_and_detach_moves_only_elements_without_empty_fallback() {
    let mut retired = NativeListRetired::new();
    let source = install(&mut retired);
    struct Conflict;
    impl gc::TraceDescriptor<NativeListN> for Conflict {
        fn trace(_: &NativeListN, _: &mut gc::TraceVisitor) {
            panic!("must not replace intrinsic descriptor")
        }
    }
    assert_eq!(
        gc::register_existing::<_, Conflict>(&source),
        Err(gc::RegistrationError::DescriptorConflict)
    );
    let detached = { source.borrow_mut().take_for_gc() };
    assert!(detached.is_some());
    assert!(!source.borrow().allow_nulls_scalar());
    assert_eq!(NativeListN::CLASS, "java.util.ImmutableCollections$ListN");
    assert!(Rc::ptr_eq(
        &source,
        &list_of_empty_ready(&mut retired).unwrap()
    ));
    assert_eq!(
        list_n_empty_to_array(&source, &mut retired).err(),
        Some(ListError::Unsupported("ListN missing actual elements"))
    );
    drop(detached);
}

#[test]
fn structural_debug_never_walks_self_containing_or_held_backing_values() {
    struct Hostile {
        _same_list: NativeListRef,
    }
    impl crate::LuaObject for Hostile {
        fn class_name(&self) -> &str {
            panic!("Debug must not invoke Java class metadata")
        }
        fn as_any(&self) -> &dyn std::any::Any {
            panic!("Debug must not invoke virtual as_any")
        }
        fn to_lua_string(&self) -> String {
            panic!("Debug must not invoke userdata formatter")
        }
    }
    let mut retired = NativeListRetired::new();
    let list = new_default(&mut retired).unwrap();
    let value = Value::Object(Rc::new(Hostile {
        _same_list: list.clone(),
    }));
    add_raw(&list, &value, &mut retired).unwrap();
    drop(value);
    let array = array(&list);
    let held_array = array.owner_slot().borrow_mut();
    let text = format!("{:?}", list.borrow());
    assert!(text.starts_with("NativeArrayList { size: 1, mod_count: 1, element_data_ptr: Some("));
    drop(held_array);
    let held_list = list.borrow_mut();
    let text = format!("{:?}", list);
    assert!(text.contains("borrowed"));
    drop(held_list);
    // The hostile owner is intentionally opaque to GC. Test-only physical
    // detach releases its self-cycle after guards, without native clear/reset.
    let detached = { array.owner_slot().borrow_mut().take_for_gc() };
    drop(detached);
}

#[test]
fn failed_constructor_pins_source_before_target_fault_and_releases_values_after_outer_guard() {
    struct Probe {
        foreign: Rc<RefCell<usize>>,
        drops: Rc<Cell<usize>>,
    }
    impl crate::LuaObject for Probe {
        fn class_name(&self) -> &str {
            "test.CollectionSourceDropProbe"
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            *self.foreign.borrow_mut() += 1;
            self.drops.set(self.drops.get() + 1);
        }
    }
    drain_all();
    let foreign = Rc::new(RefCell::new(0));
    let drops = Rc::new(Cell::new(0));
    let mut creation = NativeListRetired::new();
    let source = new_default(&mut creation).unwrap();
    let value = Value::Object(Rc::new(Probe {
        foreign: foreign.clone(),
        drops: drops.clone(),
    }));
    add_raw(&source, &value, &mut creation).unwrap();
    drop(value);
    drop(creation);
    drain_all();
    let weak_source = Rc::downgrade(&source);
    let mut retired = NativeListRetired::new();
    let token = allocate_for_collection(&mut retired).unwrap();
    let actual = token.receiver_slot().clone();
    let held_target = actual.borrow_mut();
    let deferred = gc::defer_collection();
    let held_foreign = foreign.borrow_mut();
    let result = token.initialize(KnownCollectionRef::ArrayList(&source), &mut retired);
    assert_eq!(
        result.err(),
        Some(ListError::Borrowed(
            "ArrayList superclass modCount initializer"
        ))
    );
    drop(source);
    assert!(
        weak_source.upgrade().is_some(),
        "source argument pinned before the first target-body fault"
    );
    drop(retired);
    assert_eq!(drain_retired(), 0);
    assert!(weak_source.upgrade().is_some());
    assert_eq!(drops.get(), 0);
    drop(held_target);
    drop(held_foreign);
    drop(deferred);
    drain_all();
    assert!(weak_source.upgrade().is_none());
    assert_eq!(drops.get(), 1);
    assert_eq!(*foreign.borrow(), 1);
}

thread_local! { static TEMPORARY_ARRAY:RefCell<Option<Weak<RefCell<NativeObjectArray>>>>=const {RefCell::new(None)}; }
fn record_temporary_array(observed: ListObserved<'_>) {
    if let ListObserved::Array(array) = observed {
        TEMPORARY_ARRAY.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(array.owner_slot())));
    }
}
#[test]
fn constructor_size_publication_precedes_empty_static_fault_and_retains_toarray_temporary() {
    let mut retired = NativeListRetired::new();
    let source = install(&mut retired);
    let token = allocate_for_collection(&mut retired).unwrap();
    let actual = token.receiver_slot().clone();
    {
        let mut body = actual.borrow_mut();
        body.size = 41;
        body.mod_count = 43;
    }
    let old = install_observer(Some(record_temporary_array));
    EMPTY_ARRAYS.with(|carrier| {
        let held_static = carrier.borrow_mut();
        let result = token.initialize(KnownCollectionRef::EmptyReadyListN(&source), &mut retired);
        assert_eq!(
            result.err(),
            Some(ListError::Borrowed("ArrayList class statics"))
        );
        assert_eq!(actual.borrow().mod_count_scalar(), 0);
        assert_eq!(
            actual.borrow().size_scalar(),
            0,
            "native size write occurs before reading EMPTY static"
        );
        assert!(
            actual.borrow().array_slot().is_none(),
            "backing publication has not completed"
        );
        assert!(held_static.empty.is_some());
    });
    install_observer(old);
    let temporary = TEMPORARY_ARRAY.with(|slot| slot.borrow_mut().take().unwrap());
    assert!(
        temporary.upgrade().is_some(),
        "source.toArray fresh result remains a retired native local"
    );
    drop(retired);
    drain_all();
    assert!(temporary.upgrade().is_none());
}
