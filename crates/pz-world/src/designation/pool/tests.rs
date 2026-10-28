use super::*;
fn statistic() -> (statistics::StatisticRef, statistics::StatisticsRetired) {
    let mut retired = statistics::StatisticsRetired::default();
    let globals = statistics::try_class_statics(&mut retired).unwrap();
    let statistic = statistics::try_instance(&globals, &mut retired).unwrap();
    (statistic, retired)
}
fn counter_for_pool(
    statistic: &statistics::StatisticRef,
    pool: &DesignationPoolRef,
) -> statistics::CounterRef {
    let map = statistic.borrow().counters_slot().unwrap().clone();
    let body = map.borrow();
    body.entries()
        .values()
        .find(|counter| {
            let capture = counter.borrow().lambda_slot().unwrap().clone();
            let capture = capture.borrow();
            capture
                .owner_slot()
                .unwrap()
                .clone()
                .downcast::<RefCell<DesignationPositionPool>>()
                .is_ok_and(|owner| Rc::ptr_eq(&owner, pool))
        })
        .unwrap()
        .clone()
}
#[test]
fn native_pool_counter_and_contains_set_capture_the_same_actual_receiver() {
    let mut retired = RetiredComponents::new();
    let pool = try_construct_native(&mut retired).unwrap();
    let (statistic, mut stats_retired) = statistic();
    let counter = counter_for_pool(&statistic, &pool);
    let (queue, set, allocator, max) = {
        let b = pool.borrow();
        let PositionPoolAuthority::Native {
            queue,
            contains_set,
            allocator,
            max_size,
        } = &b.authority
        else {
            panic!("native");
        };
        (
            queue.clone().unwrap(),
            contains_set.clone().unwrap(),
            allocator.clone().unwrap(),
            *max_size,
        )
    };
    assert_eq!(max, 1024);
    assert!(queue.borrow().entries.is_empty());
    assert!(Rc::ptr_eq(set.borrow().pool.as_ref().unwrap(), &pool));
    assert_eq!(
        set.borrow().auto_compaction_factor.to_bits(),
        0.0f32.to_bits()
    );
    assert!(allocator.try_borrow_mut().is_ok());
    assert_eq!(
        statistics::try_counter_get(&counter, &mut stats_retired).unwrap(),
        0.0
    );
    let position = Rc::new(RefCell::new(Position3d::default()));
    queue
        .borrow_mut()
        .entries
        .push_back(EntityOwner::from_ref(&position));
    assert_eq!(
        statistics::try_counter_get(&counter, &mut stats_retired).unwrap(),
        1.0
    );
    statistics::try_counter_clear(&counter, &mut stats_retired).unwrap();
    assert_eq!(
        statistics::try_counter_get(&counter, &mut stats_retired).unwrap(),
        1.0
    );
    let held = queue.borrow_mut();
    assert!(matches!(
        statistics::try_counter_get(&counter, &mut stats_retired),
        Err(statistics::StatisticsError::Borrowed(
            "ObjectPool.size actual deque"
        ))
    ));
    drop(held);
    let PositionPoolAuthority::Native { contains_set, .. } = &pool.borrow().authority else {
        unreachable!()
    };
    assert!(
        contains_set.as_ref().unwrap().borrow().entries.is_empty(),
        "no fabricated set publication from direct test queue mutation"
    );
}
#[test]
fn held_map_failure_keeps_native_pool_fields_null_after_counter_super_prefix() {
    let (statistic, mut stats_retired) = statistic();
    let globals = statistics::try_class_statics(&mut stats_retired).unwrap();
    let id = globals.borrow().id_slot().unwrap().clone();
    let old = id.borrow().get();
    let map = statistic.borrow().counters_slot().unwrap().clone();
    let held = map.borrow_mut();
    let pool = Rc::new(RefCell::new(DesignationPositionPool::default()));
    assert!(matches!(
        register_super(&pool, &globals, &mut stats_retired),
        Err(RuntimeHostError::Borrowed("Statistic.store map put"))
    ));
    assert_eq!(id.borrow().get(), old.wrapping_add(1));
    let b = pool.borrow();
    assert!(matches!(
        &b.authority,
        PositionPoolAuthority::Native {
            queue: None,
            contains_set: None,
            allocator: None,
            max_size: 0
        }
    ));
    drop(b);
    drop(held);
}
#[test]
fn successful_super_store_survives_later_queue_publication_host_fault() {
    let (statistic, mut stats_retired) = statistic();
    let globals = statistics::try_class_statics(&mut stats_retired).unwrap();
    let pool = Rc::new(RefCell::new(DesignationPositionPool::default()));
    register_super(&pool, &globals, &mut stats_retired).unwrap();
    let counter = counter_for_pool(&statistic, &pool);
    let allocator = Rc::new(RefCell::new(PositionAllocator));
    let mut retired = RetiredComponents::new();
    let held = pool.borrow_mut();
    assert!(matches!(
        finish_fields(&pool, &allocator, &mut retired),
        Err(RuntimeHostError::Borrowed("ObjectPool deque publication"))
    ));
    drop(held);
    assert!(matches!(
        statistics::try_counter_get(&counter, &mut stats_retired),
        Err(statistics::StatisticsError::Unsupported(
            "ObjectPool.size null deque diagnostic"
        ))
    ));
    let weak = Rc::downgrade(&pool);
    drop((pool, stats_retired, retired));
    assert!(
        weak.upgrade().is_some(),
        "published counter strongly owns partial actual pool"
    );
    let name = counter.borrow().name.clone().unwrap();
    let mut retired = statistics::StatisticsRetired::default();
    statistics::try_remove_counter(&statistic, &name, &mut retired).unwrap();
    drop((counter, retired));
    assert!(weak.upgrade().is_none());
}
#[test]
fn release_empty_skips_held_pool_and_nonempty_stops_before_debug_or_queue_writes() {
    let mut retired = RetiredComponents::new();
    let pool = try_construct_native(&mut retired).unwrap();
    let roof = Rc::new(RefCell::new(DesignationList::RoofAreas(Vec::new())));
    let held = pool.borrow_mut();
    try_release_roofs(&pool, &roof, &mut retired).unwrap();
    drop(held);
    let position = Rc::new(RefCell::new(Position3d::default()));
    {
        let mut b = roof.borrow_mut();
        let DesignationList::RoofAreas(v) = &mut *b else {
            unreachable!()
        };
        v.push(EntityOwner::from_ref(&position));
    }
    assert!(matches!(
        try_release_roofs(&pool, &roof, &mut retired),
        Err(RuntimeHostError::Unsupported {
            operation: "ObjectPool.release actual objectPoolContains debug option",
            ..
        })
    ));
    assert_eq!(roof.borrow().len(), 1);
    let b = pool.borrow();
    let PositionPoolAuthority::Native {
        queue,
        contains_set,
        ..
    } = &b.authority
    else {
        unreachable!()
    };
    assert!(
        queue.as_ref().unwrap().borrow().entries.is_empty()
            && contains_set.as_ref().unwrap().borrow().entries.is_empty()
    );
}
#[test]
fn native_zone_reset_preserves_permanent_counter_map_and_incrementing_id() {
    struct Host;
    impl DesignationHost for Host {
        fn rand_next(&mut self, _: i32) -> HostResult<i32> {
            Ok(47)
        }
        fn current_cell_present(&mut self) -> HostResult<bool> {
            Ok(false)
        }
        fn corner_square(&mut self, _: DesignationCorner<'_>) -> HostResult<Option<SquareRef>> {
            panic!("null cell");
        }
    }
    let registry = new_registry();
    let mut retired = RetiredComponents::new();
    let zone = try_new_animal(
        &registry,
        Some("native"),
        0,
        0,
        0,
        2,
        2,
        false,
        &mut Host,
        &mut retired,
    )
    .unwrap();
    let pool = zone.borrow().position3d_pool.as_ref().unwrap().clone();
    let (statistic, mut stats_retired) = statistic();
    let counter = counter_for_pool(&statistic, &pool);
    let name = counter.borrow().name.clone().unwrap();
    let map = statistic.borrow().counters_slot().unwrap().clone();
    let before = map.borrow().entries().len();
    let globals = statistics::try_class_statics(&mut stats_retired).unwrap();
    let id = globals.borrow().id_slot().unwrap().clone();
    let number = id.borrow().get();
    try_reset_base(&registry, &mut retired).unwrap();
    try_reset_animal(&registry, &mut retired).unwrap();
    assert_eq!(map.borrow().entries().len(), before);
    assert_eq!(id.borrow().get(), number);
    assert!(Rc::ptr_eq(
        &counter,
        &statistics::try_counter_by_name(&statistic, &name, &mut stats_retired)
            .unwrap()
            .unwrap()
    ));
}
