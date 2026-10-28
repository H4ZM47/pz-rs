use super::*;
use pz_world::designation as core;
struct Restore {
    designation: Option<core::DesignationObserver>,
    statistics: Option<stats::StatisticsObserver>,
}
impl Restore {
    fn new() -> Self {
        pz_inventory::component_host::drain_retired_components();
        let _ = gc::collect_tables();
        let old = Self {
            designation: core::install_designation_observer(Some(designation::observe_designation)),
            statistics: stats::install_observer(Some(observe_statistics)),
        };
        install_statistics_observer();
        old
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        core::install_designation_observer(self.designation);
        stats::install_observer(self.statistics);
        pz_inventory::component_host::drain_retired_components();
        super::super::retry_pending();
        let _ = gc::collect_tables();
    }
}
struct Host;
impl core::DesignationHost for Host {
    fn rand_next(&mut self, _: i32) -> pz_entity::runtime_load::HostResult<i32> {
        Ok(345)
    }
    fn current_cell_present(&mut self) -> pz_entity::runtime_load::HostResult<bool> {
        Ok(false)
    }
    fn corner_square(
        &mut self,
        _: core::DesignationCorner<'_>,
    ) -> pz_entity::runtime_load::HostResult<Option<SquareRef>> {
        panic!("current cell absent");
    }
}
fn actual_graph() -> (
    core::DesignationRegistryRef,
    core::DesignationZoneRef,
    core::DesignationPoolRef,
    stats::StatisticRef,
    stats::CounterRef,
    String,
) {
    let registry = core::new_registry();
    let mut retired = RetiredComponents::new();
    let zone = core::try_new_animal(
        &registry,
        Some("PoolGC"),
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
    let mut stats_retired = stats::StatisticsRetired::default();
    let globals = stats::try_class_statics(&mut stats_retired).unwrap();
    let statistic = stats::try_instance(&globals, &mut stats_retired).unwrap();
    let map = statistic.borrow().counters_slot().unwrap().clone();
    let counter = map
        .borrow()
        .entries()
        .values()
        .find(|counter| {
            let capture = counter.borrow().lambda_slot().unwrap().clone();
            let c = capture.borrow();
            c.owner_slot()
                .unwrap()
                .clone()
                .downcast::<RefCell<DesignationPositionPool>>()
                .is_ok_and(|owner| Rc::ptr_eq(&owner, &pool))
        })
        .unwrap()
        .clone();
    let name = counter.borrow().name.clone().unwrap();
    pz_inventory::component_host::retire_components(retired);
    drop(stats_retired);
    pz_inventory::component_host::drain_retired_components();
    (registry, zone, pool, statistic, counter, name)
}
fn reset(registry: &core::DesignationRegistryRef) {
    let mut retired = RetiredComponents::new();
    core::try_reset_base(registry, &mut retired).unwrap();
    core::try_reset_animal(registry, &mut retired).unwrap();
    pz_inventory::component_host::retire_components(retired);
}
#[test]
fn static_counter_roots_pool_set_but_not_zone_and_removal_collects_cycle() {
    let _restore = Restore::new();
    let (registry, zone, pool, statistic, counter, name) = actual_graph();
    let set = {
        let b = pool.borrow();
        let PositionPoolAuthority::Native { contains_set, .. } = &b.authority else {
            panic!("native");
        };
        contains_set.as_ref().unwrap().clone()
    };
    let weak_zone = Rc::downgrade(&zone);
    let weak_pool = Rc::downgrade(&pool);
    let weak_set = Rc::downgrade(&set);
    let weak_counter = Rc::downgrade(&counter);
    let value = counter.borrow().value_slot().unwrap().clone();
    reset(&registry);
    drop((registry, zone, pool, set, counter));
    pz_inventory::component_host::drain_retired_components();
    super::super::retry_pending();
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak_zone.upgrade().is_none());
    assert!(
        weak_pool.upgrade().is_some()
            && weak_set.upgrade().is_some()
            && weak_counter.upgrade().is_some()
    );
    let mut retired = stats::StatisticsRetired::default();
    stats::try_remove_counter(&statistic, &name, &mut retired).unwrap();
    drop(retired);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(
        weak_counter.upgrade().is_none()
            && weak_pool.upgrade().is_none()
            && weak_set.upgrade().is_none()
    );
    assert_eq!(
        value.borrow().get(),
        0.0,
        "retaining leaf atomic has no reverse pool edge"
    );
}
#[test]
fn held_pool_defers_physical_collection_and_retained_counter_keeps_same_capture() {
    let _restore = Restore::new();
    let (registry, zone, pool, statistic, counter, name) = actual_graph();
    let weak = Rc::downgrade(&pool);
    reset(&registry);
    drop((registry, zone));
    pz_inventory::component_host::drain_retired_components();
    let mut retired = stats::StatisticsRetired::default();
    stats::try_remove_counter(&statistic, &name, &mut retired).unwrap();
    drop(retired);
    {
        let _held = pool.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    }
    drop(pool);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak.upgrade().is_some());
    let capture = counter.borrow().lambda_slot().unwrap().clone();
    let owner = capture
        .borrow()
        .owner_slot()
        .unwrap()
        .clone()
        .downcast::<RefCell<DesignationPositionPool>>()
        .unwrap();
    assert!(Rc::ptr_eq(&owner, &weak.upgrade().unwrap()));
    drop((owner, capture, counter));
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak.upgrade().is_none());
}
#[test]
fn deque_and_set_aliases_are_two_physical_slots_and_gc_does_not_execute_size_reader() {
    let _restore = Restore::new();
    let (registry, zone, pool, statistic, counter, name) = actual_graph();
    let (deque, set) = {
        let b = pool.borrow();
        let PositionPoolAuthority::Native {
            queue,
            contains_set,
            ..
        } = &b.authority
        else {
            panic!("native");
        };
        (
            queue.as_ref().unwrap().clone(),
            contains_set.as_ref().unwrap().clone(),
        )
    };
    // Explicit physical-slot fixture, not a claim about unported release().
    let position = Rc::new(RefCell::new(Position3d {
        x: 1.0,
        y: 2.0,
        z: 3.0,
    }));
    observe_position(&position);
    deque
        .borrow_mut()
        .entries
        .push_back(pz_entity::EntityOwner::from_ref(&position));
    set.borrow_mut()
        .entries
        .push(pz_entity::EntityOwner::from_ref(&position));
    observe_pool_deque(&deque);
    observe_pool_set(&set);
    let weak = Rc::downgrade(&position);
    reset(&registry);
    drop((registry, zone, position));
    pz_inventory::component_host::drain_retired_components();
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert_eq!(weak.upgrade().unwrap().borrow().z, 3.0);
    let mut retired = stats::StatisticsRetired::default();
    stats::try_remove_counter(&statistic, &name, &mut retired).unwrap();
    drop((retired, counter, pool, deque, set));
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak.upgrade().is_none());
}
