use super::*;
use pz_kahlua::TableRef;

struct Restore {
    context: Option<DesignationContext>,
    observer: Option<designation::DesignationObserver>,
}
impl Restore {
    fn new() -> Self {
        pz_inventory::component_host::drain_retired_components();
        let _ = pz_kahlua::gc::collect_tables();
        let _ = take_native_class_faults();
        Self {
            context: install_context(None),
            observer: designation::install_designation_observer(Some(
                crate::component_gc::observe_designation,
            )),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        install_context(self.context.take());
        designation::install_designation_observer(self.observer);
        pz_inventory::component_host::drain_retired_components();
        crate::component_gc::retry_pending();
        let _ = pz_kahlua::gc::collect_tables();
        let _ = take_native_class_faults();
    }
}
struct Host {
    calls: Rc<RefCell<Vec<&'static str>>>,
    next: i32,
}
impl DesignationHost for Host {
    fn rand_next(&mut self, bound: i32) -> HostResult<i32> {
        assert_eq!(bound, 9_999_999);
        self.calls.borrow_mut().push("rand");
        self.next += 1;
        Ok(self.next)
    }
    fn current_cell_present(&mut self) -> HostResult<bool> {
        self.calls.borrow_mut().push("cell");
        Ok(false)
    }
    fn corner_square(
        &mut self,
        _: designation::DesignationCorner<'_>,
    ) -> HostResult<Option<pz_world::SquareRef>> {
        panic!("actual null-cell branch must not query a square")
    }
    fn sync_zone(&mut self, zone: &DesignationZoneRef) -> HostResult<()> {
        assert!(zone.try_borrow_mut().is_ok());
        self.calls.borrow_mut().push("sync");
        Ok(())
    }
}
fn fixture() -> (
    Thread,
    DesignationRegistryRef,
    DesignationHostRef,
    Rc<RefCell<Vec<&'static str>>>,
) {
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    register(&mut thread);
    let registry = designation::new_registry();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let host: DesignationHostRef = Rc::new(RefCell::new(Box::new(Host {
        calls: calls.clone(),
        next: 0,
    })));
    install_context(Some(DesignationContext::new(&registry, &host)));
    (thread, registry, host, calls)
}
// Explicit offline profile for independent query/reset/GC fixtures. It is
// never exposed as a Lua constructor or substituted for a Native failure.
fn seed_offline_zone(
    thread: &mut Thread,
    registry: &DesignationRegistryRef,
    name: &str,
    x: i32,
    y: i32,
    z: i32,
    w: i32,
    h: i32,
) -> DesignationZoneRef {
    let mut retired = RetiredComponents::new();
    let zone = designation::seed_animal_data_projection(
        registry,
        &pz_world::meta_save::DesignationZone {
            id: 700001.0,
            x,
            y,
            z,
            w,
            h,
            zone_type: "AnimalZone".into(),
            name: name.into(),
            hour_last_seen: 0,
        },
        &mut retired,
    )
    .unwrap();
    pz_inventory::component_host::retire_components(retired);
    thread.globals().set_str("zone", zone_value(&zone).unwrap());
    zone
}
fn call(thread: &mut Thread, source: &str) -> LuaResult<Vec<Value>> {
    let closure = pz_kahlua::stdlib::load_string(
        source,
        Some("actual designation"),
        thread.globals().clone(),
    )
    .unwrap();
    thread
        .pcall_ex(&closure, &[])
        .map_err(|(_, _, error)| error)
}
fn has_error(value: LuaResult<Vec<Value>>, class: &'static str) {
    assert!(
        matches!(value,Err(LuaError::Runtime{class:actual,..}) if actual==class),
        "{value:?}"
    );
}
fn zones_len(registry: &DesignationRegistryRef, animal: bool) -> usize {
    let body = registry.borrow();
    let list = if animal {
        body.animal_zones_slot()
    } else {
        body.all_zones_slot()
    }
    .unwrap();
    let count = list.borrow().len();
    count
}

#[test]
fn real_lua_queries_over_offline_fixture_and_native_base_constructors_preserve_identity() {
    let _restore = Restore::new();
    let (mut thread, registry, _host, calls) = fixture();
    seed_offline_zone(&mut thread, &registry, "Pasture", 3, 4, 2, 6, 4);
    let result=call(&mut thread,"return zone, DesignationZoneAnimal.getZone(4,5,2), DesignationZoneAnimal.getZone(4,5), DesignationZoneAnimal.getZoneById(zone:getId()), DesignationZoneAnimal.getZoneByName('Pasture'), zone:getX(),zone:getY(),zone:getW(),zone:getH(),DesignationZoneAnimal.getType()").unwrap();
    assert!(result[..5].iter().all(|value| value.raw_equals(&result[0])));
    assert_eq!(
        result[5..9].iter().map(Value::as_num).collect::<Vec<_>>(),
        vec![Some(3.0), Some(4.0), Some(6.0), Some(4.0)]
    );
    assert_eq!(result[9].as_str(), Some("AnimalZone"));
    assert!(
        calls.borrow().is_empty(),
        "querying the offline fixture must not invent constructor host calls"
    );
    assert_eq!(
        (zones_len(&registry, false), zones_len(&registry, true)),
        (1, 1)
    );
    let result=call(&mut thread,"base=DesignationZone.addZone('AnimalZone','BaseOnly',30,40,2,35,45);return base,DesignationZone.getZone(31,41,2),DesignationZoneAnimal.getZone(31,41,2)").unwrap();
    assert!(result[0].raw_equals(&result[1]));
    assert!(result[2].is_nil());
    let Value::Object(base) = &result[0] else {
        panic!("base userdata")
    };
    assert_eq!(base.class_name(), BASE);
    assert_eq!(
        (zones_len(&registry, false), zones_len(&registry, true)),
        (2, 1)
    );
}

#[test]
fn unavailable_context_and_held_actual_bodies_never_become_missing_zone() {
    let _restore = Restore::new();
    let (mut thread, registry, host, calls) = fixture();
    {
        let _held = host.borrow_mut();
        assert!(call(&mut thread,"local z=DesignationZone.new();return z:getName(),z:getId(),DesignationZone.getZone(0,0,0)").is_ok());
    }
    assert!(calls.borrow().is_empty());
    {
        let _held = registry.borrow_mut();
        has_error(
            call(&mut thread, "return DesignationZoneAnimal.getZone(0,0,0)"),
            "pz.host.Borrowed",
        );
    }
    seed_offline_zone(&mut thread, &registry, "A", 0, 0, 0, 2, 2);
    let zone = designation::try_get_zone(&registry, 0, 0, 0)
        .unwrap()
        .unwrap();
    let value = zone_value(&zone).unwrap();
    {
        let _held = zone.borrow_mut();
        assert!(zone_value(&zone).unwrap().raw_equals(&value));
        has_error(call(&mut thread, "return zone:getX()"), "pz.host.Borrowed");
        has_error(
            call(&mut thread, "return DesignationZoneAnimal.getZone(0,0,0)"),
            "pz.host.Borrowed",
        );
        assert_eq!(
            call(&mut thread, "return DesignationZoneAnimal.getType()").unwrap()[0].as_str(),
            Some("AnimalZone")
        );
    }
    install_context(None);
    assert!(call(&mut thread, "DesignationZone.removeZone(nil,true)").is_ok());
    has_error(
        call(
            &mut thread,
            "return DesignationZoneAnimal.getZone(100,100,0)",
        ),
        "pz.host.Unsupported",
    );
    assert_eq!(
        call(&mut thread, "return zone:getName()").unwrap()[0].as_str(),
        Some("A")
    );
    has_error(
        call(&mut thread, "zone:setName(nil)"),
        "pz.host.Unsupported",
    );
    assert!(call(&mut thread, "return zone:getName()").unwrap()[0].is_nil());
}

#[test]
fn class_resets_preserve_real_list_allocations_and_retained_zone_without_cross_clearing() {
    let _restore = Restore::new();
    let (mut thread, registry, _host, _calls) = fixture();
    let all = registry.borrow().all_zones_slot().unwrap().clone();
    let animals = registry.borrow().animal_zones_slot().unwrap().clone();
    let seeded = seed_offline_zone(&mut thread, &registry, "Kept", 0, 0, 0, 4, 4);
    let original = zone_value(&seeded).unwrap();
    call(&mut thread, "DesignationZone.Reset()").unwrap();
    let result = call(
        &mut thread,
        "return DesignationZone.getZone(1,1,0),DesignationZoneAnimal.getZone(1,1,0)",
    )
    .unwrap();
    assert!(result[0].is_nil());
    assert!(result[1].raw_equals(&original));
    call(&mut thread, "DesignationZoneAnimal.Reset()").unwrap();
    assert!(call(&mut thread, "return DesignationZoneAnimal.getZone(1,1,0)").unwrap()[0].is_nil());
    assert_eq!(
        call(&mut thread, "return zone:getName()").unwrap()[0].as_str(),
        Some("Kept")
    );
    assert!(Rc::ptr_eq(
        &all,
        registry.borrow().all_zones_slot().unwrap()
    ));
    assert!(Rc::ptr_eq(
        &animals,
        registry.borrow().animal_zones_slot().unwrap()
    ));
    assert!(all.borrow().is_empty() && animals.borrow().is_empty());
}

#[test]
fn name_write_precedes_sync_host_failure_and_base_remove_preserves_subclass_membership() {
    let _restore = Restore::new();
    let (mut thread, registry, host, calls) = fixture();
    seed_offline_zone(&mut thread, &registry, "Before", 0, 0, 0, 4, 4);
    calls.borrow_mut().clear();
    {
        let _held = host.borrow_mut();
        has_error(
            call(&mut thread, "zone:setName('After')"),
            "pz.host.Borrowed",
        );
    }
    assert_eq!(
        call(&mut thread, "return zone:getName()").unwrap()[0].as_str(),
        Some("After")
    );
    assert!(calls.borrow().is_empty());
    call(
        &mut thread,
        "DesignationZone.removeZone('AnimalZone','After')",
    )
    .unwrap();
    assert_eq!(
        (zones_len(&registry, false), zones_len(&registry, true)),
        (0, 1)
    );
    assert_eq!(
        call(
            &mut thread,
            "return DesignationZoneAnimal.getZone(1,1,0):getName()"
        )
        .unwrap()[0]
            .as_str(),
        Some("After")
    );
    has_error(
        call(&mut thread, "DesignationZoneAnimal.removeZone(zone,false)"),
        "pz.host.Unsupported",
    );
    assert_eq!(zones_len(&registry, true), 1);
    call(
        &mut thread,
        "temporary=DesignationZone.new('Generic','Held',20,20,0,21,21,false)",
    )
    .unwrap();
    let temporary = designation::try_get_base_zone(&registry, 20, 20, 0)
        .unwrap()
        .unwrap();
    {
        let _held = temporary.borrow_mut();
        assert!(call(&mut thread, "DesignationZone.removeZone(temporary,false)").is_ok());
    }
    assert_eq!(zones_len(&registry, false), 0);
}

#[test]
fn native_list_apis_report_missing_typed_backend_without_snapshot_or_fake_empty_value() {
    let _restore = Restore::new();
    let (mut thread, registry, _host, _calls) = fixture();
    seed_offline_zone(&mut thread, &registry, "A", 0, 0, 0, 2, 2);
    assert_eq!(
        call(&mut thread, "return DesignationZoneAnimal.getType()").unwrap()[0].as_str(),
        Some("AnimalZone")
    );
    let aliases = call(&mut thread, "return zone.getType,zone.addNewRoof").unwrap();
    assert!(
        aliases.iter().all(Value::is_nil),
        "Native exposer excludes static methods from instance index"
    );
    for source in [
        "DesignationZoneAnimal.addNewRoof(1,2,3)",
        "return DesignationZoneAnimal.getAllZones()",
        "return DesignationZone.getAllZonesByType('AnimalZone')",
        "return DesignationZone.allZones",
        "return DesignationZoneAnimal.designationAnimalZoneList",
        "return DesignationZone.lastUpdate",
        "return zone:getFoodOnGround()",
    ] {
        has_error(call(&mut thread, source), "pz.host.Unsupported");
    }
    let color = thread
        .globals()
        .get_str("DesignationZoneAnimal")
        .as_table()
        .unwrap()
        .get_str("ZONE_COLOR_R");
    let Value::Object(color) = color else {
        panic!("Native reflected float remains boxed")
    };
    assert_eq!(color.class_name(), "java.lang.Float");
    let color = (&*color as &dyn Any)
        .downcast_ref::<pz_kahlua::table::BoxedFloat>()
        .unwrap();
    assert_eq!(color.get().to_bits(), 0x3e4ccccd);
}

#[test]
fn conflicting_native_kind_metadata_remains_faulted_after_retry_and_rejects_cached_userdata() {
    let _restore = Restore::new();
    let (_thread, registry, _host, _calls) = fixture();
    let mut retired = RetiredComponents::new();
    let zone = designation::seed_animal_data_projection(
        &registry,
        &pz_world::meta_save::DesignationZone {
            id: 700001.0,
            x: 0,
            y: 0,
            z: 0,
            w: 2,
            h: 2,
            zone_type: "AnimalZone".into(),
            name: "Conflict".into(),
            hour_last_seen: 0,
        },
        &mut retired,
    )
    .unwrap();
    pz_inventory::component_host::retire_components(retired);
    let _canonical = zone_value(&zone).unwrap();
    crate::component_gc::observe_designation(designation::DesignationObserved::Zone(
        &zone,
        DesignationZoneKind::Base,
    ));
    has_error(
        zone_value(&zone).map(|value| vec![value]),
        "pz.host.InvalidState",
    );
    crate::component_gc::retry_pending();
    has_error(
        zone_value(&zone).map(|value| vec![value]),
        "pz.host.InvalidState",
    );
    let faults = take_native_class_faults();
    assert_eq!(faults.len(), 1);
    assert_eq!((faults[0].expected, faults[0].observed), (ANIMAL, BASE));
}

#[test]
fn real_lua_animal_constructor_preserves_base_and_seven_lists_before_counter_capability_fault() {
    let _restore = Restore::new();
    let (mut thread, registry, _host, calls) = fixture();
    let error = call(
        &mut thread,
        "return DesignationZoneAnimal.new('Partial',9,8,2,3,4,false)",
    )
    .unwrap_err();
    assert!(
        matches!(error,LuaError::Runtime {class:"pz.host.Unsupported",message} if message=="ObjectPoolCounter PoolStatistic constructor")
    );
    assert_eq!(&*calls.borrow(), &["rand"]);
    assert_eq!(
        (zones_len(&registry, false), zones_len(&registry, true)),
        (1, 0)
    );
    let zone = designation::try_get_base_zone(&registry, 4, 5, 2)
        .unwrap()
        .unwrap();
    {
        let body = zone.borrow();
        assert_eq!((body.x, body.y, body.w, body.h), (3, 4, 6, 4));
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
        .all(|list| list.is_some()));
        assert!(body.position3d_pool.is_none());
        assert_eq!(body.id, 100001.0);
    }
    let value = zone_value(&zone).unwrap();
    let queried = call(
        &mut thread,
        "return DesignationZone.getZoneByName('Partial'),DesignationZoneAnimal.getZone(4,5,2)",
    )
    .unwrap();
    assert!(queried[0].raw_equals(&value));
    assert!(queried[1].is_nil());
}
