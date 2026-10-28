use super::*;
use pz_kahlua::Value;

fn read_number(owner: &Rc<dyn Any>) -> StatisticsResult<f64> {
    let value = owner.downcast_ref::<RefCell<f64>>().unwrap();
    let number = *value
        .try_borrow()
        .map_err(|_| borrowed("test actual pool"))?;
    Ok(number)
}
fn globals() -> (ClassStaticsRef, StatisticsRetired) {
    let mut retired = StatisticsRetired::default();
    let globals = try_class_statics(&mut retired).unwrap();
    (globals, retired)
}
fn fields(statistic: &StatisticRef) -> (CounterMapRef, NumberMapRef, TableRef, TableRef) {
    let b = statistic.borrow();
    (
        b.counters_slot().unwrap().clone(),
        b.statistics_slot().unwrap().clone(),
        b.local_table_slot().unwrap().clone(),
        b.remote_table_slot().unwrap().clone(),
    )
}
#[test]
fn permanent_singleton_two_tables_and_monotonic_counters_capture_same_owner() {
    let (globals, mut retired) = globals();
    let again = try_class_statics(&mut retired).unwrap();
    assert!(Rc::ptr_eq(&globals, &again));
    let statistic = try_instance(&globals, &mut retired).unwrap();
    assert!(Rc::ptr_eq(
        &statistic,
        &try_instance(&again, &mut retired).unwrap()
    ));
    let (map, numbers, local, remote) = fields(&statistic);
    assert!(!local.ptr_eq(&remote));
    assert_eq!(statistic.borrow().name.as_deref(), Some("pool"));
    let id = globals.borrow().id_slot().unwrap().clone();
    let start = id.borrow().get();
    let pool = Rc::new(RefCell::new(3.0));
    let first = try_register_object_pool_counter(
        &globals,
        &pool,
        Some("test.same"),
        read_number,
        &mut retired,
    )
    .unwrap();
    let second =
        try_register_object_pool_counter(&globals, &pool, None, read_number, &mut retired).unwrap();
    let name = first.borrow().name.clone().unwrap();
    assert_eq!(name, format!("test.same-{}", start.wrapping_add(1)));
    assert_eq!(
        second.borrow().name.as_deref(),
        Some(format!("null-{}", start.wrapping_add(2)).as_str())
    );
    assert!(Rc::ptr_eq(
        &first,
        &try_counter_by_name(&statistic, &name, &mut retired)
            .unwrap()
            .unwrap()
    ));
    let capture = first.borrow().lambda_slot().unwrap().clone();
    let owner = capture
        .borrow()
        .owner_slot()
        .unwrap()
        .clone()
        .downcast::<RefCell<f64>>()
        .unwrap();
    assert!(Rc::ptr_eq(&owner, &pool));
    *pool.borrow_mut() = 8.0;
    assert_eq!(try_counter_get(&first, &mut retired).unwrap(), 8.0);
    let value = first.borrow().value_slot().unwrap().clone();
    value.borrow().set(41.0);
    try_counter_clear(&first, &mut retired).unwrap();
    assert_eq!(value.borrow().get(), 0.0);
    assert!(Rc::ptr_eq(first.borrow().lambda_slot().unwrap(), &capture));
    assert_eq!(try_counter_get(&first, &mut retired).unwrap(), 8.0);
    let (same_map, same_numbers, same_local, same_remote) = fields(&statistic);
    assert!(
        Rc::ptr_eq(&map, &same_map)
            && Rc::ptr_eq(&numbers, &same_numbers)
            && local.ptr_eq(&same_local)
            && remote.ptr_eq(&same_remote)
    );
    try_remove_counter(&statistic, &name, &mut retired).unwrap();
    let second_name = second.borrow().name.clone().unwrap();
    try_remove_counter(&statistic, &second_name, &mut retired).unwrap();
}
#[test]
fn native_atomic_increment_wraps_and_double_preserves_raw_float_bits() {
    let id = AtomicLongValue(AtomicI64::new(i64::MAX));
    assert_eq!(id.increment_and_get(), i64::MIN);
    assert_eq!(id.get(), i64::MIN);
    let value = AtomicDoubleValue(AtomicU64::new(0));
    for bits in [(-0.0f64).to_bits(), 0x7ff8_0000_0000_0198] {
        value.set(f64::from_bits(bits));
        assert_eq!(value.get().to_bits(), bits);
    }
}
#[test]
fn empty_update_wipes_same_local_table_keeps_remote_and_finishes_pending_delete() {
    // Isolated actual class-static carrier avoids resetting production statics.
    let globals = Rc::new(RefCell::new(ClassStatics {
        id: Some(Rc::new(RefCell::new(AtomicLongValue(AtomicI64::new(0))))),
        instance: None,
        initializing: false,
    }));
    let mut retired = StatisticsRetired::default();
    let statistic = try_instance(&globals, &mut retired).unwrap();
    let (_, numbers, local, remote) = fields(&statistic);
    numbers.borrow_mut().values.insert("old".into(), 1.0);
    local.set_str("old", Value::Num(2.0));
    remote.set_str("keep", Value::Num(3.0));
    try_update(&statistic, &mut retired).unwrap();
    assert!(numbers.borrow().values.is_empty());
    assert!(local.try_is_empty().unwrap());
    assert_eq!(remote.get_str("keep").as_num(), Some(3.0));
    let (_, _, same, _) = fields(&statistic);
    assert!(local.ptr_eq(&same));
    let pool = Rc::new(RefCell::new(0.0));
    let counter = try_register_object_pool_counter(
        &globals,
        &pool,
        Some("deletion"),
        read_number,
        &mut retired,
    )
    .unwrap();
    let name = counter.borrow().name.clone().unwrap();
    try_remove_counter(&statistic, &name, &mut retired).unwrap();
    statistic.borrow_mut().counter_for_delete = Some(counter);
    try_update(&statistic, &mut retired).unwrap();
    assert!(statistic.borrow().counter_for_delete_slot().is_none());
}
#[test]
fn nonempty_update_preserves_native_clear_prefix_without_invoking_reader() {
    fn forbidden(_: &Rc<dyn Any>) -> StatisticsResult<f64> {
        panic!("native iteration unsupported before Counter.get")
    }
    let (globals, mut retired) = globals();
    let pool = Rc::new(RefCell::new(0.0));
    let counter = try_register_object_pool_counter(
        &globals,
        &pool,
        Some("not-iterated"),
        forbidden,
        &mut retired,
    )
    .unwrap();
    let name = counter.borrow().name.clone().unwrap();
    let statistic = try_instance(&globals, &mut retired).unwrap();
    let (_, numbers, local, remote) = fields(&statistic);
    numbers.borrow_mut().values.insert("old".into(), 1.0);
    local.set_str("old", Value::Bool(true));
    remote.set_str("keep", Value::Bool(true));
    assert_eq!(
        try_update(&statistic, &mut retired),
        Err(unsupported(
            "Statistic.update native HashMap values iterator"
        ))
    );
    assert!(numbers.borrow().values.is_empty() && local.try_is_empty().unwrap());
    assert!(!remote.try_is_empty().unwrap());
    assert!(try_counter_by_name(&statistic, &name, &mut retired)
        .unwrap()
        .is_some());
    try_remove_counter(&statistic, &name, &mut retired).unwrap();
}
#[test]
fn held_local_table_stops_after_number_map_clear_before_counter_iterator() {
    let (globals, mut retired) = globals();
    let statistic = try_instance(&globals, &mut retired).unwrap();
    let (map, numbers, local, _) = fields(&statistic);
    numbers.borrow_mut().values.insert("old".into(), 2.0);
    local.set_str("old", Value::Num(4.0));
    let held = local.try_borrow_body_mut().unwrap();
    let held_map = map.borrow_mut();
    assert_eq!(
        try_update(&statistic, &mut retired),
        Err(borrowed("Statistic.update localTable wipe"))
    );
    assert!(numbers.borrow().values.is_empty());
    drop(held_map);
    drop(held);
    assert_eq!(local.get_str("old").as_num(), Some(4.0));
}
#[test]
fn held_counter_map_fails_store_after_id_and_complete_counter_fields() {
    thread_local! {static COUNTER:RefCell<Option<std::rc::Weak<RefCell<Counter>>>>=const{RefCell::new(None)};}
    fn observer(v: StatisticsObserved<'_>) {
        if let StatisticsObserved::Counter(c) = v {
            assert!(c.try_borrow_mut().is_ok());
            COUNTER.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(c)));
        }
    }
    struct Restore(Option<StatisticsObserver>);
    impl Drop for Restore {
        fn drop(&mut self) {
            install_observer(self.0);
        }
    }
    let _restore = Restore(install_observer(Some(observer)));
    let (globals, mut retired) = globals();
    let statistic = try_instance(&globals, &mut retired).unwrap();
    let map = statistic.borrow().counters_slot().unwrap().clone();
    let id = globals.borrow().id_slot().unwrap().clone();
    let before = id.borrow().get();
    let held = map.borrow_mut();
    let pool = Rc::new(RefCell::new(0.0));
    assert!(matches!(
        try_register_object_pool_counter(
            &globals,
            &pool,
            Some("partial"),
            read_number,
            &mut retired
        ),
        Err(StatisticsError::Borrowed("Statistic.store map put"))
    ));
    assert_eq!(id.borrow().get(), before.wrapping_add(1));
    let counter = COUNTER.with(|slot| slot.borrow().as_ref().unwrap().upgrade().unwrap());
    let c = counter.borrow();
    assert!(c.value_slot().is_some() && c.lambda_slot().is_some() && c.perishable);
    assert_eq!(c.tooltip.as_deref(), Some("Objects in the pool"));
    let name = c.name.clone().unwrap();
    drop(c);
    drop(held);
    assert!(try_counter_by_name(&statistic, &name, &mut retired)
        .unwrap()
        .is_none());
}
#[test]
fn removed_counter_owners_retire_after_outer_guard_instead_of_final_drop_in_map() {
    struct Probe {
        outer: Rc<RefCell<()>>,
        safe: Rc<Cell<bool>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.safe.set(self.outer.try_borrow_mut().is_ok());
        }
    }
    fn reader(_: &Rc<dyn Any>) -> StatisticsResult<f64> {
        Ok(0.0)
    }
    let (globals, mut retired) = globals();
    let statistic = try_instance(&globals, &mut retired).unwrap();
    let outer = Rc::new(RefCell::new(()));
    let safe = Rc::new(Cell::new(false));
    let pool = Rc::new(RefCell::new(Probe {
        outer: outer.clone(),
        safe: safe.clone(),
    }));
    let weak = Rc::downgrade(&pool);
    let counter =
        try_register_object_pool_counter(&globals, &pool, Some("retire"), reader, &mut retired)
            .unwrap();
    let name = counter.borrow().name.clone().unwrap();
    drop((counter, pool));
    drop(retired);
    let mut retired = StatisticsRetired::default();
    let held = outer.borrow_mut();
    try_remove_counter(&statistic, &name, &mut retired).unwrap();
    assert!(weak.upgrade().is_some());
    assert!(!safe.get());
    drop(held);
    drop(retired);
    assert!(weak.upgrade().is_none() && safe.get());
}
