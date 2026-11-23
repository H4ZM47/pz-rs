use super::*;
use pz_entity::{Component, Components, EntityOwner, FluidContainer};
use pz_inventory::shared::{self, ItemRef};
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use pz_kahlua::{stdlib, TableRef, Thread};
use pz_world::floor_object::{FloorObject, FloorObjectRef};
use pz_world::objects::{IsoWorldInventoryObject, ObjectKind};
use pz_world::{IsoObject, ObjectSlot};

struct Restore {
    item: Option<shared::ItemObserver>,
    object: Option<pz_world::object_slot::ObjectObserver>,
}
impl Restore {
    fn new() -> Self {
        Self {
            item: shared::install_item_observer(Some(crate::component_gc::observe_item)),
            object: pz_world::object_slot::install_object_observer(Some(
                crate::component_gc::observe_object,
            )),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        shared::install_item_observer(self.item);
        pz_world::object_slot::install_object_observer(self.object);
        pz_inventory::component_host::drain_retired_components();
        crate::component_gc::retry_pending();
        let _ = pz_kahlua::gc::collect_tables();
    }
}
fn item() -> ItemRef {
    shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Normal,
            "Base",
            "WorldOwnerFixture",
        )),
        1,
    ))
    .unwrap()
}
fn floor(item: &ItemRef, fluid: bool) -> (IsoObjectRef, FloorObjectRef) {
    let floor = FloorObject::new(IsoWorldInventoryObject {
        xoff: 0.0,
        yoff: 0.0,
        zoff: 0.0,
        offset_x: 0.0,
        offset_y: 0.0,
        item: pz_world::floor_item::FloorItem::retain(item),
        drop_time: 0.0,
        ignore_remove_sandbox: false,
        extended_placement: false,
    });
    let payload = floor.ref_handle();
    let mut object = IsoObject::new(1, ObjectKind::WorldInventory(floor), None);
    if fluid {
        object.components =
            pz_inventory::item_components::ItemComponents::from_values_unowned(Components {
                list: vec![Component::FluidContainer(FluidContainer::default())],
            })
            .unwrap();
    }
    (ObjectSlot::new(object).ref_handle(), payload)
}
fn thread() -> Thread {
    let mut t = Thread::new(TableRef::new());
    stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    crate::bind_globals::register(&mut t);
    crate::bind_misc_classes::register(&mut t, None);
    crate::bind_fluid_container::register(&mut t);
    crate::bind_items::register(&mut t);
    t
}
fn run(t: &mut Thread, source: &str) -> Vec<Value> {
    let f = stdlib::load_string(source, Some("actual-world-owner"), t.globals().clone()).unwrap();
    t.pcall(&f, &[])
        .unwrap_or_else(|(e, trace)| panic!("{}\n{trace}", pz_kahlua::exec::java_value_of(&e)))
}
fn failure(t: &mut Thread, source: &str) -> String {
    let f = stdlib::load_string(
        source,
        Some("actual-world-owner-error"),
        t.globals().clone(),
    )
    .unwrap();
    t.pcall(&f, &[]).unwrap_err().0.as_str().unwrap().to_owned()
}

#[test]
fn world_item_getter_is_null_only_for_empty_slot_and_cold_identity_uses_native_class_stamp() {
    let _restore = Restore::new();
    let mut t = thread();
    let item = item();
    assert!(get_world_item(&item).unwrap().is_nil());
    let (object, payload) = floor(&item, false);
    assert!(
        get_world_item(&item).unwrap().is_nil(),
        "constructor alone does not publish backlink"
    );
    pz_inventory::world_item::set_world_item_ref(&item, Some(&EntityOwner::from_ref(&object)))
        .unwrap();
    t.globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    let guard = object.borrow_mut();
    let values = run(&mut t, "return item:getWorldItem(), item:getWorldItem()");
    assert!(
        matches!((&values[0], &values[1]), (Value::Object(a), Value::Object(b)) if Rc::ptr_eq(a,b))
    );
    let world = object_from_value(&values[0]).unwrap();
    assert!(Rc::ptr_eq(&world.object, &object));
    assert_eq!(world.class_name(), WORLD_INVENTORY_OBJECT);
    drop(guard);
    assert!(Rc::ptr_eq(
        payload.borrow().item.item_slot().unwrap(),
        &item
    ));
    crate::component_gc::retry_pending();
    assert!(crate::component_gc::take_registration_faults().is_empty());
}

#[test]
fn public_world_setter_only_assigns_one_field_and_null_does_not_remove_floor_item() {
    let _restore = Restore::new();
    let mut t = thread();
    let item = item();
    let other = self::item();
    let (a, pa) = floor(&item, false);
    let (b, pb) = floor(&other, false);
    t.globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    t.globals().set_str("a", object_value(&a).unwrap());
    t.globals().set_str("b", object_value(&b).unwrap());
    let values = run(&mut t, "item:setWorldItem(a); local first=item:getWorldItem(); item:setWorldItem(b); return first==a,item:getWorldItem()==b");
    assert!(values.iter().all(|v| matches!(v, Value::Bool(true))));
    assert!(Rc::ptr_eq(pa.borrow().item.item_slot().unwrap(), &item));
    assert!(Rc::ptr_eq(pb.borrow().item.item_slot().unwrap(), &other));
    assert!(run(&mut t, "item:setWorldItem(nil); return item:getWorldItem()")[0].is_nil());
    assert!(Rc::ptr_eq(pa.borrow().item.item_slot().unwrap(), &item));
    assert!(Rc::ptr_eq(pb.borrow().item.item_slot().unwrap(), &other));
}

#[test]
fn world_component_getter_and_component_owner_share_the_complete_object_userdata() {
    let _restore = Restore::new();
    let mut t = thread();
    let item = item();
    let (object, _) = floor(&item, true);
    pz_inventory::world_item::set_world_item_ref(&item, Some(&EntityOwner::from_ref(&object)))
        .unwrap();
    let component = object
        .borrow()
        .components
        .get_ref(pz_entity::ComponentType::FluidContainer)
        .unwrap()
        .unwrap();
    t.globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    t.globals().set_str(
        "fluid",
        crate::bind_fluid_container::fluid_value(&component).unwrap(),
    );
    let values = run(&mut t, "local world=item:getWorldItem(); return world:getFluidContainer()==fluid,fluid:getOwner()==world,item:getFluidContainer()==nil");
    assert!(values.iter().all(|v| matches!(v, Value::Bool(true))));
    let held = object.borrow_mut();
    let values = run(&mut t, "return fluid:getOwner()==item:getWorldItem()");
    assert!(matches!(values[0], Value::Bool(true)));
    assert!(
        failure(&mut t, "return item:getWorldItem():getFluidContainer()")
            .ends_with(" pz.host.Borrowed")
    );
    drop(held);
}

#[test]
fn world_argument_and_actual_authority_errors_are_explicit_and_never_fake_absence() {
    let _restore = Restore::new();
    let mut t = thread();
    let item = item();
    t.globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    assert!(failure(&mut t, "item:setWorldItem({})")
        .contains("expected argument of type IsoWorldInventoryObject, got KahluaTableImpl"));
    assert!(item.borrow().world_item_slot().is_none());
    {
        let _guard = item.borrow_mut();
        assert!(failure(&mut t, "return item:getWorldItem()").ends_with(" pz.host.Borrowed"));
        assert!(failure(&mut t, "item:setWorldItem(nil)").ends_with(" pz.host.Borrowed"));
    }
    let wrong = self::item();
    pz_inventory::world_item::set_world_item_ref(&item, Some(&EntityOwner::from_ref(&wrong)))
        .unwrap();
    assert!(matches!(
        get_world_item(&item),
        Err(LuaError::Runtime {
            class: "pz.host.Unsupported",
            ..
        })
    ));
    pz_inventory::world_item::set_world_item_ref(&item, None).unwrap();
}

#[test]
fn last_displaced_world_graph_is_retired_past_outer_foreign_guard() {
    struct Probe(Rc<RefCell<()>>, Rc<std::cell::Cell<usize>>);
    impl LuaObject for Probe {
        fn class_name(&self) -> &str {
            "test.WorldRetirementProbe"
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            assert!(self.0.try_borrow_mut().is_ok());
            self.1.set(self.1.get() + 1);
        }
    }
    let _restore = Restore::new();
    let item = item();
    let old_item = self::item();
    let foreign = Rc::new(RefCell::new(()));
    let drops = Rc::new(std::cell::Cell::new(0));
    pz_inventory::moddata::set_mod_data_value_ref(
        &old_item,
        "probe",
        Some(Value::Object(Rc::new(Probe(
            foreign.clone(),
            drops.clone(),
        )))),
    )
    .unwrap();
    let (object, payload) = floor(&old_item, false);
    pz_inventory::world_item::set_world_item_ref(&item, Some(&EntityOwner::from_ref(&object)))
        .unwrap();
    drop(old_item);
    drop(payload);
    drop(object);
    pz_inventory::component_host::drain_retired_components();
    {
        let _guard = foreign.borrow_mut();
        set_world_item(&item, &Value::Nil).unwrap();
        assert_eq!(drops.get(), 0);
    }
    pz_inventory::component_host::drain_retired_components();
    assert_eq!(drops.get(), 1);
}

#[test]
fn conflicting_native_class_is_permanent_typed_fault_and_retry_cannot_validate_cached_userdata() {
    let _restore = Restore::new();
    let _ = take_native_class_faults();
    let item = item();
    let (object, _) = floor(&item, false);
    let cached = object_value(&object).unwrap();
    assert!(matches!(
        remember_class(&object, ISO_OBJECT),
        Err(LuaError::Runtime {
            class: "pz.host.InvalidState",
            ..
        })
    ));
    assert!(matches!(
        object_value(&object),
        Err(LuaError::Runtime {
            class: "pz.host.InvalidState",
            ..
        })
    ));
    assert_eq!(
        crate::component_gc::register_object(&object).unwrap(),
        crate::component_gc::MetadataStatus::InvalidClass
    );
    crate::component_gc::retry_pending();
    assert!(matches!(
        ensure_class(&object, WORLD_INVENTORY_OBJECT),
        Err(LuaError::Runtime {
            class: "pz.host.InvalidState",
            ..
        })
    ));
    assert!(matches!(
        fluid_component(&cached),
        Err(LuaError::Runtime {
            class: "pz.host.InvalidState",
            ..
        })
    ));
    assert!(matches!(
        set_world_item(&item, &cached),
        Err(LuaError::Runtime {
            class: "pz.host.InvalidState",
            ..
        })
    ));
    assert!(item.borrow().world_item_slot().is_none());
    pz_inventory::world_item::set_world_item_ref(&item, Some(&EntityOwner::from_ref(&object)))
        .unwrap();
    assert!(matches!(
        get_world_item(&item),
        Err(LuaError::Runtime {
            class: "pz.host.InvalidState",
            ..
        })
    ));
    let faults = take_native_class_faults();
    assert_eq!(
        faults,
        vec![NativeClassFault {
            address: Rc::as_ptr(&object) as usize,
            expected: WORLD_INVENTORY_OBJECT,
            observed: ISO_OBJECT
        }]
    );
    assert!(take_native_class_faults().is_empty());
    assert!(
        matches!(
            object_value(&object),
            Err(LuaError::Runtime {
                class: "pz.host.InvalidState",
                ..
            })
        ),
        "diagnostic drain does not heal allocation metadata"
    );
    set_world_item(&item, &Value::Nil).unwrap();
}

#[test]
fn ten_thousand_live_world_allocations_and_cache_hits_use_amortized_weak_pruning() {
    let _restore = Restore::new();
    let classes_before = CLASS_PRUNES.with(|count| count.get());
    let userdata_before = USERDATA_PRUNES.with(|count| count.get());
    let objects: Vec<_> = (0..10_000)
        .map(|id| ObjectSlot::new(IsoObject::new(id, ObjectKind::Object, None)).ref_handle())
        .collect();
    let values: Vec<_> = objects
        .iter()
        .map(|object| object_value(object).unwrap())
        .collect();
    assert!(
        CLASS_PRUNES.with(|count| count.get()) - classes_before < 32,
        "class metadata must not scan all live objects on each observation"
    );
    assert!(
        USERDATA_PRUNES.with(|count| count.get()) - userdata_before < 32,
        "userdata cache must not scan all live objects on each creation"
    );
    let classes_after = CLASS_PRUNES.with(|count| count.get());
    let userdata_after = USERDATA_PRUNES.with(|count| count.get());
    for _ in 0..100 {
        let alias = object_value(&objects[0]).unwrap();
        assert!(
            matches!((&alias, &values[0]), (Value::Object(a), Value::Object(b)) if Rc::ptr_eq(a,b))
        );
    }
    assert_eq!(CLASS_PRUNES.with(|count| count.get()), classes_after);
    assert_eq!(USERDATA_PRUNES.with(|count| count.get()), userdata_after);
}

#[test]
fn container_count_counts_the_main_and_secondary_containers() {
    let _restore = Restore::new();
    let mut t = thread();
    let plain = ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, None)).ref_handle();
    let mut fridge = IsoObject::new(2, ObjectKind::Object, None);
    fridge.container = Some(pz_inventory::ItemContainer::new(Some("fridge")));
    fridge
        .secondary_containers
        .push(pz_inventory::ItemContainer::new(Some("freezer")));
    let fridge = ObjectSlot::new(fridge).ref_handle();
    let mut only_secondary = IsoObject::new(3, ObjectKind::Object, None);
    only_secondary
        .secondary_containers
        .push(pz_inventory::ItemContainer::new(Some("freezer")));
    let only_secondary = ObjectSlot::new(only_secondary).ref_handle();
    for (name, object) in [("plain", &plain), ("fridge", &fridge), ("secondary", &only_secondary)] {
        t.globals().set_str(name, object_value(object).unwrap());
    }
    let values = run(
        &mut t,
        "return plain:getContainerCount(), fridge:getContainerCount(), secondary:getContainerCount()",
    );
    let counts: Vec<f64> = values.iter().map(|v| v.as_num().unwrap()).collect();
    assert_eq!(counts, [0.0, 2.0, 1.0]);
}

#[test]
fn container_getters_share_one_live_container_with_the_object() {
    let _restore = Restore::new();
    let mut t = thread();
    crate::bind_item_container::register(&mut t);
    let mut fridge = IsoObject::new(2, ObjectKind::Object, None);
    let mut main = pz_inventory::ItemContainer::new(Some("fridge"));
    main.items.push(pz_inventory::SavedItem::Item(Box::new(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Milk")),
        1,
    ))));
    fridge.container = Some(main);
    fridge
        .secondary_containers
        .push(pz_inventory::ItemContainer::new(Some("freezer")));
    let fridge = ObjectSlot::new(fridge).ref_handle();
    let mut only_secondary = IsoObject::new(3, ObjectKind::Object, None);
    only_secondary
        .secondary_containers
        .push(pz_inventory::ItemContainer::new(Some("freezer")));
    let only_secondary = ObjectSlot::new(only_secondary).ref_handle();
    t.globals().set_str("fridge", object_value(&fridge).unwrap());
    t.globals().set_str("secondary", object_value(&only_secondary).unwrap());
    let values = run(
        &mut t,
        r#"
        local a, b = fridge:getContainerByIndex(0), fridge:getContainerByIndex(1)
        a:setExplored(true)
        return a:getType(), b:getType(),
            fridge:getContainerByIndex(2) == nil and fridge:getContainerByIndex(-1) == nil,
            fridge:getContainer() == a and fridge:getItemContainer() == a
                and fridge:getContainerByIndex(0) == a,
            fridge:getContainerByType("freezer") == b and fridge:getContainerByType("crate") == nil,
            fridge:getContainerByEitherType("crate", "freezer") == b,
            fridge:getContainerIndex(a), fridge:getContainerIndex(b), fridge:getContainerIndex(nil),
            a:getParent() == fridge, a:getItems():size(), fridge:getContainerCount(),
            secondary:getContainer() == nil, secondary:getContainerByIndex(0):getType(),
            secondary:getContainerIndex(nil)
        "#,
    );
    assert_eq!(values[0].as_str(), Some("fridge"));
    assert_eq!(values[1].as_str(), Some("freezer"));
    for i in [2, 3, 4, 5, 9, 12] {
        assert!(matches!(values[i], Value::Bool(true)), "value {i}");
    }
    let nums: Vec<f64> = [6, 7, 8, 10, 11, 14].iter().map(|&i| values[i].as_num().unwrap()).collect();
    assert_eq!(nums, [0.0, 1.0, -1.0, 1.0, 2.0, 0.0]);
    assert_eq!(values[13].as_str(), Some("freezer"));
    // The shared containers are the object's state: the save snapshot sees
    // the Lua change, and the codec fields are empty.
    let body = fridge.borrow();
    assert!(body.container.is_none() && body.secondary_containers.is_empty());
    assert_eq!(body.container_count(), 2);
    assert_eq!(
        body.container_types(),
        [Some("fridge".to_owned()), Some("freezer".to_owned())]
    );
    let saved = body.saved_containers().unwrap();
    assert!(saved[0].explored);
    assert_eq!(saved[0].items.len(), 1);
}

#[test]
fn mod_data_is_one_live_table_that_container_names_and_save_read() {
    let _restore = Restore::new();
    let mut t = thread();
    crate::bind_item_container::register(&mut t);
    let mut crate_ = IsoObject::new(4, ObjectKind::Object, None);
    crate_.container = Some(pz_inventory::ItemContainer::new(Some("crate")));
    let crate_ = ObjectSlot::new(crate_).ref_handle();
    t.globals().set_str("crate", object_value(&crate_).unwrap());
    let values = run(
        &mut t,
        r#"
        local before = crate:hasModData()
        local c = crate:getContainerByIndex(0)
        local unnamed = c:getCustomName()
        crate:getModData().crate_customContainerName = "Mine"
        return before, unnamed, c:getCustomName(), crate:hasModData(),
            crate:getModData() == crate:getModData()
        "#,
    );
    assert!(matches!(values[0], Value::Bool(false)));
    assert!(values[1].is_nil());
    assert_eq!(values[2].as_str(), Some("Mine"));
    assert!(matches!(values[3], Value::Bool(true)));
    assert!(matches!(values[4], Value::Bool(true)));
    let body = crate_.borrow();
    let saved = body.saved_mod_data().unwrap().unwrap();
    assert_eq!(saved.entries.len(), 1);
}

#[test]
fn door_state_and_square_are_readable_from_lua() {
    let _restore = Restore::new();
    let mut t = thread();
    let door = pz_world::objects::IsoDoor {
        north: true,
        locked: true,
        ..Default::default()
    };
    let object = ObjectSlot::new(IsoObject::new(1, ObjectKind::Door(door), None)).ref_handle();
    t.globals().set_str("door", object_value(&object).unwrap());
    let v = run(
        &mut t,
        "return instanceof(door, 'IsoDoor'), door:IsOpen(), door:getNorth(), door:isLocked(), \
         door:isLockedByKey(), door:getSquare()",
    );
    let b = |i: usize| matches!(v[i], Value::Bool(true));
    assert_eq!((b(0), b(1), b(2), b(3), b(4)), (true, false, true, true, false));
    // Not on a square yet.
    assert!(v[5].is_nil());
    if let ObjectKind::Door(d) = &mut object.borrow_mut().kind {
        d.open = true;
    }
    assert!(matches!(run(&mut t, "return door:IsOpen()")[0], Value::Bool(true)));
}

#[test]
fn get_sprite_returns_the_tile_sprite_or_nil() {
    let _restore = Restore::new();
    let mut t = thread();
    crate::bind_sprites::register(&mut t);
    let mut sprites = pz_world::sprite::IsoSpriteManager::new(pz_world::sprite::test_tables());
    let bin = sprites.add_sprite_named("trashcontainers_01_16");
    sprites.sprites[bin].name = Some("trashcontainers_01_16".into());
    let old = crate::bind_sprites::manager().ok();
    crate::bind_sprites::install_sprite_manager(Some(Rc::new(RefCell::new(sprites))));
    let tiled = ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, Some(bin))).ref_handle();
    let bare = ObjectSlot::new(IsoObject::new(2, ObjectKind::Object, None)).ref_handle();
    t.globals().set_str("tiled", object_value(&tiled).unwrap());
    t.globals().set_str("bare", object_value(&bare).unwrap());
    let values = run(
        &mut t,
        "return tiled:getSprite():getName(), bare:getSprite() == nil, \
         tiled:getSprite():getProperties():has('IsTrashCan')",
    );
    crate::bind_sprites::install_sprite_manager(old);
    assert_eq!(values[0].as_str(), Some("trashcontainers_01_16"));
    assert!(matches!(values[1], Value::Bool(true)));
    assert!(matches!(values[2], Value::Bool(false)));
}

#[test]
fn base_object_answers_the_loot_window_fire_and_bbq_checks() {
    let _restore = Restore::new();
    let mut t = thread();
    let mut named = IsoObject::new(1, ObjectKind::Object, None);
    named.name = Some(pz_world::objects::ObjectName::Str("Campfire".into()));
    let named = ObjectSlot::new(named).ref_handle();
    let plain = ObjectSlot::new(IsoObject::new(2, ObjectKind::Object, None)).ref_handle();
    let mut lit = pz_world::objects::IsoBarbecue::default();
    lit.lit = true;
    lit.fuel_amount_minutes = 30;
    let bbq = ObjectSlot::new(IsoObject::new(3, ObjectKind::Barbecue(lit), None)).ref_handle();
    t.globals().set_str("named", object_value(&named).unwrap());
    t.globals().set_str("plain", object_value(&plain).unwrap());
    t.globals().set_str("bbq", object_value(&bbq).unwrap());
    let values = run(
        &mut t,
        "return named:getName(), plain:getName() == nil, plain:isFireInteractionObject(), \
         plain:isPropaneBBQ(), plain:isLit(), plain:hasPropaneTank(), \
         bbq:isFireInteractionObject(), bbq:isPropaneBBQ(), bbq:isLit(), bbq:hasFuel(), \
         bbq:hasPropaneTank(), plain.hasFuel == nil",
    );
    assert_eq!(values[0].as_str(), Some("Campfire"));
    let flags: Vec<bool> = values[1..].iter().map(|v| matches!(v, Value::Bool(true))).collect();
    // A barbecue without a sprite is not a propane one, so it has no tank.
    assert_eq!(flags, [true, false, false, false, false, true, false, true, true, false, true]);
}

#[test]
fn special_tooltip_flag_is_read_and_set_through_lua() {
    let _restore = Restore::new();
    let mut t = thread();
    let item = item();
    let (object, _) = floor(&item, false);
    t.globals().set_str("o", object_value(&object).unwrap());
    let values = run(&mut t, "local before = o:haveSpecialTooltip(); o:setSpecialTooltip(true); return before, o:haveSpecialTooltip()");
    assert!(matches!(values[..], [Value::Bool(false), Value::Bool(true)]));
    assert!(object.borrow().special_tooltip);
}
