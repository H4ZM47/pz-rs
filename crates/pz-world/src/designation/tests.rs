use super::*;
use crate::{IsoGridSquare, IsoObject, ObjectKind, ObjectSlot, SquareSlot};
use std::collections::VecDeque;

struct Host {
    random: i32,
    cells: VecDeque<bool>,
    corners: VecDeque<bool>,
    trace: Vec<String>,
    mutate_after_first: Option<DesignationZoneRef>,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            random: 7,
            cells: VecDeque::from([false]),
            corners: VecDeque::new(),
            trace: Vec::new(),
            mutate_after_first: None,
        }
    }
}
impl DesignationHost for Host {
    fn rand_next(&mut self, bound: i32) -> HostResult<i32> {
        self.trace.push(format!("rand:{bound}"));
        Ok(self.random)
    }
    fn current_cell_present(&mut self) -> HostResult<bool> {
        self.trace.push("cell".into());
        Ok(self.cells.pop_front().unwrap_or(false))
    }
    fn corner_square(&mut self, corner: DesignationCorner<'_>) -> HostResult<Option<SquareRef>> {
        let (x, y, z) = corner.read_coordinates()?;
        self.trace.push(format!("corner:{x},{y},{z}"));
        if let Some(zone) = self.mutate_after_first.take() {
            let mut zone = zone.try_borrow_mut().unwrap();
            zone.w = 20;
            zone.h = 30;
        }
        Ok(self
            .corners
            .pop_front()
            .unwrap_or(false)
            .then(|| SquareSlot::new(IsoGridSquare::new(x, y, z)).ref_handle()))
    }
    fn sync_zone(&mut self, zone: &DesignationZoneRef) -> HostResult<()> {
        assert!(zone.try_borrow_mut().is_ok());
        self.trace.push("sync".into());
        Ok(())
    }
}
fn list(registry: &DesignationRegistryRef, animal: bool) -> DesignationListRef {
    let registry = registry.borrow();
    if animal {
        registry.animal_zones_slot().unwrap().clone()
    } else {
        registry.all_zones_slot().unwrap().clone()
    }
}
fn zones(list: &DesignationListRef) -> Vec<DesignationZoneRef> {
    let list = list.borrow();
    let DesignationList::Zones(zones) = &*list else {
        panic!("fixture zone list")
    };
    zones.clone()
}
fn animal_data_projection(registry: &DesignationRegistryRef, number: i32) -> DesignationZoneRef {
    seed_animal_data_projection(
        registry,
        &crate::meta_save::DesignationZone {
            id: 100000.0 + f64::from(number),
            name: "pen".into(),
            zone_type: "AnimalZone".into(),
            x: 1,
            y: 2,
            z: 3,
            w: 4,
            h: 6,
            hour_last_seen: 0,
        },
        &mut RetiredComponents::new(),
    )
    .unwrap()
}
fn record(id: f64, name: &str, x: i32, y: i32, w: i32, h: i32, hour: i32) -> Vec<u8> {
    let mut out = Vec::new();
    out.put_f64(id);
    for v in [x, y, 3, h, w] {
        out.put_i32(v);
    }
    out.write_string_utf(Some("AnimalZone"));
    out.write_string_utf(Some(name));
    out.put_i32(hour);
    out
}

#[test]
fn separate_class_lists_and_live_query_order_in_explicit_data_projection() {
    let registry = new_registry();
    let all = list(&registry, false);
    let animals = list(&registry, true);
    assert!(!Rc::ptr_eq(&all, &animals));
    let first = animal_data_projection(&registry, 7);
    let second = animal_data_projection(&registry, 7);
    try_remove_base_zone(
        &registry,
        Some(&second),
        false,
        &mut Host::default(),
        &mut RetiredComponents::new(),
    )
    .unwrap();
    assert_eq!(zones(&all).len(), 1);
    assert_eq!(zones(&animals).len(), 2);
    assert!(Rc::ptr_eq(&zones(&all)[0], &first));
    assert!(Rc::ptr_eq(&zones(&animals)[1], &second));
    assert!(Rc::ptr_eq(
        &try_get_zone(&registry, 1, 2, 3).unwrap().unwrap(),
        &first
    ));
    first.borrow_mut().x = 100;
    assert!(Rc::ptr_eq(
        &try_get_zone(&registry, 1, 2, 3).unwrap().unwrap(),
        &second
    ));
    assert!(try_get_zone(&registry, 1, 2, 4).unwrap().is_none());
    assert!(try_get_zone(&registry, 5, 2, 3).unwrap().is_none());
    {
        let mut z = second.borrow_mut();
        z.x = i32::MAX - 2;
        z.w = 9;
    }
    assert!(try_get_zone(&registry, i32::MAX - 1, 2, 3)
        .unwrap()
        .is_none());
    let held = registry.borrow_mut();
    assert!(matches!(
        try_get_zone(&registry, 0, 0, 0),
        Err(RuntimeHostError::Borrowed(_))
    ));
    drop(held);
    let held = animals.borrow_mut();
    assert!(matches!(
        try_get_zone(&registry, 0, 0, 0),
        Err(RuntimeHostError::Borrowed(_))
    ));
    drop(held);
}

#[test]
fn native_random_collision_skips_base_add_but_registers_actual_animal_pool_then_checks() {
    let registry = new_registry();
    let mut host = Host::default();
    let mut retired = RetiredComponents::new();
    let base = try_new_zone(
        &registry,
        DesignationZoneKind::Base,
        Some("Generic"),
        Some("first"),
        0,
        0,
        0,
        3,
        3,
        false,
        &mut host,
        &mut retired,
    )
    .unwrap();
    host.trace.clear();
    let animal = try_new_animal(
        &registry,
        Some("second"),
        0,
        0,
        0,
        3,
        3,
        true,
        &mut host,
        &mut retired,
    )
    .unwrap();
    assert_eq!(host.trace, vec!["rand:9999999", "cell"]);
    let all = zones(&list(&registry, false));
    assert_eq!(all.len(), 1);
    assert!(Rc::ptr_eq(&all[0], &base));
    assert!(Rc::ptr_eq(&zones(&list(&registry, true))[0], &animal));
    assert!(animal.borrow().position3d_pool.is_some());
}

#[test]
fn current_zone_body_fault_is_not_absent_and_null_zone_does_not_read_object() {
    let registry = new_registry();
    let object = ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, None)).ref_handle();
    let square = SquareSlot::new(IsoGridSquare::new(1, 2, 3)).ref_handle();
    let held = object.borrow_mut();
    try_add_item_on_ground(&registry, &object, &square).unwrap();
    let zone = animal_data_projection(&registry, 8);
    assert!(matches!(
        try_add_item_on_ground(&registry, &object, &square),
        Err(RuntimeHostError::Unsupported {
            operation: "DesignationZoneAnimal.isItemFood",
            ..
        })
    ));
    let held_zone = zone.borrow_mut();
    assert!(matches!(
        try_get_zone(&registry, 1, 2, 3),
        Err(RuntimeHostError::Borrowed(_))
    ));
    drop(held_zone);
    drop(held);
}

#[test]
fn existing_load_keeps_identity_position_and_uses_double_equals_without_constructor() {
    let registry = new_registry();
    let zone = animal_data_projection(&registry, 4);
    zone.borrow_mut().id = f64::NAN;
    let bytes = record(
        f64::from_bits(0x7ff0000000000001),
        "renamed",
        90,
        91,
        -5,
        22,
        77,
    );
    let mut reader = JavaReader::new(&bytes);
    let mut host = Host::default();
    let loaded = try_load_zone(
        &registry,
        &mut reader,
        &mut host,
        &mut RetiredComponents::new(),
    )
    .unwrap()
    .unwrap();
    assert!(Rc::ptr_eq(&loaded, &zone));
    assert_eq!(reader.position(), bytes.len());
    assert!(host.trace.is_empty());
    let z = zone.borrow();
    assert_eq!(
        (z.x, z.y, z.z, z.w, z.h, z.hour_last_seen),
        (1, 2, 3, -5, 22, 77)
    );
    assert_eq!(z.name.as_deref(), Some("renamed"));
    drop(z);
    zone.borrow_mut().id = -0.0;
    assert!(try_get_zone_by_id(&registry, 0.0).unwrap().is_none());
    assert!(Rc::ptr_eq(
        &try_get_zone_by_id(&registry, -0.0).unwrap().unwrap(),
        &zone
    ));
}

#[test]
fn load_header_truncation_does_not_query_registry_or_construct() {
    let registry = new_registry();
    let held = registry.borrow_mut();
    let bytes = record(5.0, "pen", 0, 0, 3, 4, 9);
    let mut reader = JavaReader::new(&bytes[..bytes.len() - 1]);
    let mut host = Host::default();
    assert!(matches!(
        try_load_zone(
            &registry,
            &mut reader,
            &mut host,
            &mut RetiredComponents::new()
        ),
        Err(DesignationIoError::Io(_))
    ));
    assert_eq!(reader.position(), bytes.len() - 4);
    assert!(host.trace.is_empty());
    drop(held);
    assert!(zones(&list(&registry, false)).is_empty());
}

#[test]
fn native_constructor_check_gate_retains_pool_memberships_random_id_and_consumed_header() {
    let registry = new_registry();
    let bytes = record(800.0, "loaded", 10, 11, 3, 4, 71);
    let mut reader = JavaReader::new(&bytes);
    let mut host = Host {
        random: 33,
        cells: VecDeque::from([true, true]),
        corners: VecDeque::from([true, true]),
        ..Host::default()
    };
    let mut retired = RetiredComponents::new();
    assert!(matches!(
        try_load_zone(&registry, &mut reader, &mut host, &mut retired),
        Err(DesignationIoError::Host(RuntimeHostError::Unsupported {
            operation: "DesignationZoneAnimal.getAllDZones during check",
            ..
        }))
    ));
    assert_eq!(reader.position(), bytes.len());
    let base = zones(&list(&registry, false));
    let animals = zones(&list(&registry, true));
    assert_eq!(base.len(), 1);
    assert_eq!(animals.len(), 1);
    assert!(Rc::ptr_eq(&base[0], &animals[0]));
    assert_eq!(
        host.trace,
        vec![
            "rand:9999999",
            "cell",
            "corner:10,11,3",
            "corner:12,14,3",
            "cell"
        ]
    );
    let z = base[0].borrow();
    assert_eq!(z.id, 100033.0);
    assert_eq!(z.hour_last_seen, 0);
    assert!(z.position3d_pool.is_some());
    assert!([
        &z.animals,
        &z.corpses,
        &z.troughs,
        &z.hutchs,
        &z.food_on_ground,
        &z.near_water_squares,
        &z.roof_areas
    ]
    .iter()
    .all(|v| v.is_some()));
    assert_eq!(z.name.as_deref(), Some("loaded"));
    assert!(!retired.is_empty());
}

#[test]
fn both_corner_queries_run_and_second_reads_mutated_current_zone() {
    let registry = new_registry();
    let zone = animal_data_projection(&registry, 8);
    let mut host = Host {
        cells: VecDeque::from([true]),
        corners: VecDeque::from([false, true]),
        mutate_after_first: Some(zone.clone()),
        ..Host::default()
    };
    assert!(!try_is_fully_streamed(&zone, &mut host).unwrap());
    assert_eq!(host.trace, vec!["cell", "corner:1,2,3", "corner:20,31,3"]);
    zone.borrow_mut().nb_of_dung = 17;
    let mut host = Host::default();
    try_check(&registry, &zone, &mut host, &mut RetiredComponents::new()).unwrap();
    assert_eq!(zone.borrow().nb_of_dung, 17);
}

#[test]
fn explicit_data_pool_nonempty_roof_stops_at_release_after_native_list_clear_prefix() {
    let registry = new_registry();
    let zone = animal_data_projection(&registry, 5);
    let object = ObjectSlot::new(IsoObject::new(2, ObjectKind::Object, None)).ref_handle();
    let (food, troughs, roof) = {
        let z = zone.borrow();
        (
            z.food_on_ground.clone().unwrap(),
            z.troughs.clone().unwrap(),
            z.roof_areas.clone().unwrap(),
        )
    };
    {
        let mut body = food.borrow_mut();
        let DesignationList::WorldObjects(values) = &mut *body else {
            panic!("food")
        };
        values.push(object.clone());
    }
    {
        let mut body = troughs.borrow_mut();
        let DesignationList::Owners(values) = &mut *body else {
            panic!("troughs")
        };
        values.push(EntityOwner::from_ref(&object));
    }
    {
        let mut body = roof.borrow_mut();
        let DesignationList::RoofAreas(values) = &mut *body else {
            panic!("roof")
        };
        values.push(EntityOwner::from_ref(&object));
    }
    {
        let mut z = zone.borrow_mut();
        z.nb_of_dung = 6;
        z.nb_of_feather = 8;
    }
    let mut host = Host {
        cells: VecDeque::from([true, true]),
        corners: VecDeque::from([true, true]),
        ..Host::default()
    };
    let mut retired = RetiredComponents::new();
    assert!(matches!(
        try_check(&registry, &zone, &mut host, &mut retired),
        Err(RuntimeHostError::Unsupported {
            operation: "ObjectPool.release data-projection receiver",
            ..
        })
    ));
    assert!(food.borrow().is_empty());
    assert_eq!(troughs.borrow().len(), 1);
    assert_eq!(roof.borrow().len(), 1);
    assert_eq!(
        (zone.borrow().nb_of_dung, zone.borrow().nb_of_feather),
        (0, 0)
    );
    assert!(!retired.is_empty());
}

#[test]
fn check_rereads_current_cell_and_nonempty_actor_fault_precedes_clears() {
    let registry = new_registry();
    let zone = animal_data_projection(&registry, 19);
    registry.borrow_mut().last_update = 88;
    zone.borrow_mut().nb_of_dung = 12;
    let mut host = Host {
        cells: VecDeque::from([true, false]),
        corners: VecDeque::from([true, true]),
        ..Host::default()
    };
    try_check(&registry, &zone, &mut host, &mut RetiredComponents::new()).unwrap();
    assert_eq!(registry.borrow().last_update, 0);
    assert_eq!(zone.borrow().nb_of_dung, 12);
    let actor = ObjectSlot::new(IsoObject::new(3, ObjectKind::Object, None)).ref_handle();
    let animals = zone.borrow().animals.clone().unwrap();
    {
        let mut list = animals.borrow_mut();
        let DesignationList::Owners(values) = &mut *list else {
            panic!("animals")
        };
        values.push(EntityOwner::from_ref(&actor));
    }
    let mut host = Host {
        cells: VecDeque::from([true, true]),
        corners: VecDeque::from([true, true]),
        ..Host::default()
    };
    assert!(matches!(
        try_check(&registry, &zone, &mut host, &mut RetiredComponents::new()),
        Err(RuntimeHostError::Unsupported {
            operation: "Designation animal.setDZone during check",
            ..
        })
    ));
    assert_eq!(animals.borrow().len(), 1);
    assert_eq!(zone.borrow().nb_of_dung, 12);
}

#[test]
fn base_animal_type_string_does_not_promote_class_or_subclass_membership() {
    let registry = new_registry();
    let mut host = Host::default();
    let mut retired = RetiredComponents::new();
    let zone = try_new_zone(
        &registry,
        DesignationZoneKind::Base,
        Some("AnimalZone"),
        Some("base"),
        1,
        2,
        3,
        5,
        8,
        true,
        &mut host,
        &mut retired,
    )
    .unwrap();
    assert_eq!(zone.borrow().kind(), DesignationZoneKind::Base);
    assert!(zone.borrow().animals.is_none());
    assert_eq!(zones(&list(&registry, false)).len(), 1);
    assert!(zones(&list(&registry, true)).is_empty());
    assert!(try_get_zone(&registry, 1, 2, 3).unwrap().is_none());
    assert!(Rc::ptr_eq(
        &try_get_base_zone(&registry, 1, 2, 3).unwrap().unwrap(),
        &zone
    ));
    assert!(Rc::ptr_eq(
        &try_get_zone_by_name_and_type(&registry, Some("AnimalZone"), Some("base"))
            .unwrap()
            .unwrap(),
        &zone
    ));
    assert_eq!(host.trace, vec!["rand:9999999", "sync"]);
    let held = registry.borrow_mut();
    let default = new_base_default();
    assert_eq!(default.borrow().id, 0.0);
    assert!(default.borrow().streamed);
    assert!(default.borrow().name.is_none());
    drop(held);
}

#[test]
fn reset_and_base_remove_preserve_permanent_lists_and_retained_zone_slots() {
    let registry = new_registry();
    let zone = animal_data_projection(&registry, 9);
    let base = list(&registry, false);
    let animals = list(&registry, true);
    let food = zone.borrow().food_on_ground.clone().unwrap();
    let mut host = Host::default();
    let mut retired = RetiredComponents::new();
    try_remove_base_zone(&registry, Some(&zone), true, &mut host, &mut retired).unwrap();
    assert!(base.borrow().is_empty());
    assert_eq!(animals.borrow().len(), 1);
    assert_eq!(host.trace, vec!["sync"]);
    try_reset_base(&registry, &mut retired).unwrap();
    try_reset_animal(&registry, &mut retired).unwrap();
    assert!(Rc::ptr_eq(&base, &list(&registry, false)));
    assert!(Rc::ptr_eq(&animals, &list(&registry, true)));
    assert!(animals.borrow().is_empty());
    assert!(Rc::ptr_eq(
        &food,
        zone.borrow().food_on_ground.as_ref().unwrap()
    ));
    let held = registry.borrow_mut();
    try_remove_base_zone(&registry, None, true, &mut host, &mut retired).unwrap();
    drop(held);
}

#[test]
fn save_reads_actual_base_list_and_preserves_count_prefix_on_selected_body_fault() {
    let registry = new_registry();
    let first = animal_data_projection(&registry, 3);
    let second = animal_data_projection(&registry, 3);
    try_remove_base_zone(
        &registry,
        Some(&second),
        false,
        &mut Host::default(),
        &mut RetiredComponents::new(),
    )
    .unwrap();
    first.borrow_mut().name = Some("current".into());
    let mut out = Vec::new();
    try_save_registry(&registry, &mut out).unwrap();
    let mut r = JavaReader::new(&out);
    assert_eq!(r.get_i32().unwrap(), 1);
    assert_eq!(r.get_f64().unwrap(), 100003.0);
    for v in [1, 2, 3, 6, 4] {
        assert_eq!(r.get_i32().unwrap(), v);
    }
    assert_eq!(r.read_string_utf().unwrap(), "AnimalZone");
    assert_eq!(r.read_string_utf().unwrap(), "current");
    let held = first.borrow_mut();
    let mut prefix = vec![42];
    assert!(matches!(
        try_save_registry(&registry, &mut prefix),
        Err(RuntimeHostError::Borrowed(_))
    ));
    assert_eq!(prefix, vec![42, 0, 0, 0, 1]);
    drop(held);
}

#[test]
fn observer_runs_after_guards_and_clear_retirement_defers_reentrant_final_drop() {
    thread_local! {static SEEN:Cell<usize>=const {Cell::new(0)};}
    fn observer(value: DesignationObserved<'_>) {
        match value {
            DesignationObserved::Registry(v) => assert!(v.try_borrow_mut().is_ok()),
            DesignationObserved::List(v) => assert!(v.try_borrow_mut().is_ok()),
            DesignationObserved::Zone(v, kind) => {
                let v = v.try_borrow_mut().unwrap();
                assert_eq!(v.kind(), kind);
            }
            DesignationObserved::Pool(v) => assert!(v.try_borrow_mut().is_ok()),
            DesignationObserved::PoolDeque(v) => assert!(v.try_borrow_mut().is_ok()),
            DesignationObserved::PoolSet(v) => assert!(v.try_borrow_mut().is_ok()),
            DesignationObserved::PoolAllocator(v) => assert!(v.try_borrow_mut().is_ok()),
            DesignationObserved::Position(v) => assert!(v.try_borrow_mut().is_ok()),
        }
        SEEN.with(|v| v.set(v.get() + 1));
    }
    struct Restore(Option<DesignationObserver>);
    impl Drop for Restore {
        fn drop(&mut self) {
            install_designation_observer(self.0);
        }
    }
    let _restore = Restore(install_designation_observer(Some(observer)));
    let registry = new_registry();
    let zone = animal_data_projection(&registry, 12);
    struct Probe {
        outer: Rc<RefCell<()>>,
        released: Rc<Cell<bool>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.released.set(self.outer.try_borrow_mut().is_ok());
        }
    }
    let outer = Rc::new(RefCell::new(()));
    let released = Rc::new(Cell::new(false));
    let probe = Rc::new(RefCell::new(Probe {
        outer: outer.clone(),
        released: released.clone(),
    }));
    let weak = Rc::downgrade(&probe);
    let hutch = zone.borrow().hutchs.clone().unwrap();
    {
        let mut body = hutch.borrow_mut();
        let DesignationList::Owners(v) = &mut *body else {
            panic!("hutchs")
        };
        v.push(EntityOwner::from_ref(&probe));
    }
    drop(probe);
    let guard = outer.borrow_mut();
    let mut retired = RetiredComponents::new();
    let mut host = Host {
        cells: VecDeque::from([true, true]),
        corners: VecDeque::from([true, true]),
        ..Host::default()
    };
    assert!(try_check(&registry, &zone, &mut host, &mut retired).is_err());
    assert!(weak.upgrade().is_some());
    assert!(!released.get());
    drop(guard);
    drop(retired);
    assert!(weak.upgrade().is_none());
    assert!(released.get());
    assert!(SEEN.with(Cell::get) > 10);
}

#[test]
fn native_counter_store_host_fault_retains_base_lists_without_animal_publication() {
    use pz_core::pool_statistics as stats;
    let mut stats_retired = stats::StatisticsRetired::default();
    let globals = stats::try_class_statics(&mut stats_retired).unwrap();
    let statistic = stats::try_instance(&globals, &mut stats_retired).unwrap();
    let map = statistic.borrow().counters_slot().unwrap().clone();
    let id = globals.borrow().id_slot().unwrap().clone();
    let before = id.borrow().get();
    let registry = new_registry();
    let mut host = Host::default();
    let mut retired = RetiredComponents::new();
    let held = map.borrow_mut();
    assert!(matches!(
        try_new_animal(
            &registry,
            Some("partial-super"),
            0,
            0,
            0,
            2,
            2,
            false,
            &mut host,
            &mut retired
        ),
        Err(RuntimeHostError::Borrowed("Statistic.store map put"))
    ));
    assert_eq!(id.borrow().get(), before.wrapping_add(1));
    assert_eq!(host.trace, vec!["rand:9999999"]);
    let all = zones(&list(&registry, false));
    assert_eq!(all.len(), 1);
    assert!(zones(&list(&registry, true)).is_empty());
    let body = all[0].borrow();
    assert!(body.position3d_pool.is_none());
    assert!([
        &body.animals,
        &body.corpses,
        &body.troughs,
        &body.hutchs,
        &body.food_on_ground,
        &body.near_water_squares,
        &body.roof_areas
    ]
    .iter()
    .all(|slot| slot.is_some()));
    drop(body);
    drop(held);
}
