use super::*;
use pz_entity::{Component, Components, EntityOwner, FluidContainer};
use pz_inventory::{ItemClass, ItemTemplate};
use pz_world::designation as core;

struct Restore {
    designation: Option<core::DesignationObserver>,
    item: Option<pz_inventory::shared::ItemObserver>,
    object: Option<pz_world::object_slot::ObjectObserver>,
}
impl Restore {
    fn new() -> Self {
        pz_inventory::component_host::drain_retired_components();
        let _ = gc::collect_tables();
        Self {
            designation: core::install_designation_observer(Some(observe_designation)),
            item: pz_inventory::shared::install_item_observer(Some(super::super::observe_item)),
            object: pz_world::object_slot::install_object_observer(Some(
                super::super::observe_object,
            )),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        core::install_designation_observer(self.designation);
        pz_inventory::shared::install_item_observer(self.item);
        pz_world::object_slot::install_object_observer(self.object);
        pz_inventory::component_host::drain_retired_components();
        super::super::retry_pending();
        let _ = gc::collect_tables();
    }
}
fn graph() -> (
    DesignationRegistryRef,
    DesignationZoneRef,
    DesignationListRef,
    ItemRef,
    IsoObjectRef,
    ComponentRef,
) {
    let registry = core::new_registry();
    let mut retired = RetiredComponents::new();
    // Deliberately named offline projection: GC tests physical actual slots,
    // not successful Native ObjectPoolCounter construction.
    let zone = core::seed_animal_data_projection(
        &registry,
        &pz_world::meta_save::DesignationZone {
            id: 700009.0,
            x: 0,
            y: 0,
            z: 0,
            w: 4,
            h: 4,
            zone_type: "AnimalZone".into(),
            name: "GcZone".into(),
            hour_last_seen: 0,
        },
        &mut retired,
    )
    .unwrap();
    pz_inventory::component_host::retire_components(retired);
    let food = zone.borrow().food_on_ground.as_ref().unwrap().clone();
    let item = pz_inventory::shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "ZoneFood")),
        1,
    ))
    .unwrap();
    let floor = pz_world::floor_object::FloorObject::new(IsoWorldInventoryObject {
        xoff: 0.0,
        yoff: 0.0,
        zoff: 0.0,
        offset_x: 0.0,
        offset_y: 0.0,
        item: pz_world::floor_item::FloorItem::retain(&item),
        drop_time: 0.0,
        ignore_remove_sandbox: false,
        extended_placement: false,
    });
    let mut object = IsoObject::new(1, ObjectKind::WorldInventory(floor), None);
    object.components =
        pz_inventory::item_components::ItemComponents::from_values_unowned(Components {
            list: vec![Component::FluidContainer(FluidContainer::default())],
        })
        .unwrap();
    let object = pz_world::ObjectSlot::new(object).ref_handle();
    let component = object
        .borrow()
        .components
        .get_ref(pz_entity::ComponentType::FluidContainer)
        .unwrap()
        .unwrap();
    pz_inventory::world_item::set_world_item_ref(&item, Some(&EntityOwner::from_ref(&object)))
        .unwrap();
    let value = crate::bind_designation::zone_value(&zone).unwrap();
    pz_inventory::moddata::set_mod_data_value_ref(&item, "retainedDesignation", Some(value))
        .unwrap();
    {
        let mut list = food.borrow_mut();
        let DesignationList::WorldObjects(objects) = &mut *list else {
            panic!("actual food list")
        };
        objects.push(object.clone());
    }
    // The fixture installs a real full-object membership in the actual final
    // list, then invokes the same pure adoption observer, without a DTO view.
    observe_designation(core::DesignationObserved::List(&food));
    pz_inventory::component_host::drain_retired_components();
    (registry, zone, food, item, object, component)
}
fn reset(registry: &DesignationRegistryRef) {
    let mut retired = RetiredComponents::new();
    core::try_reset_base(registry, &mut retired).unwrap();
    core::try_reset_animal(registry, &mut retired).unwrap();
    pz_inventory::component_host::retire_components(retired);
}

#[test]
fn actual_zone_food_full_object_item_table_cycle_keeps_retained_root_then_collects_without_native_reset(
) {
    let _restore = Restore::new();
    let (registry, zone, food, item, object, component) = graph();
    let color = component.borrow().fluid_color_slot().unwrap().clone();
    *color.borrow_mut() = [0.1, 0.3, 0.5, 0.7];
    zone.borrow_mut().nb_of_dung = 17;
    zone.borrow_mut().nb_of_feather = 29;
    let all = registry.borrow().all_zones_slot().unwrap().clone();
    let animal = registry.borrow().animal_zones_slot().unwrap().clone();
    let wz = Rc::downgrade(&zone);
    let wl = Rc::downgrade(&food);
    let wi = Rc::downgrade(&item);
    let wo = Rc::downgrade(&object);
    let wc = Rc::downgrade(&component);
    reset(&registry);
    assert!(Rc::ptr_eq(
        &all,
        registry.borrow().all_zones_slot().unwrap()
    ));
    assert!(Rc::ptr_eq(
        &animal,
        registry.borrow().animal_zones_slot().unwrap()
    ));
    assert!(all.borrow().is_empty() && animal.borrow().is_empty());
    drop((registry, food, item, object, component));
    pz_inventory::component_host::drain_retired_components();
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(wi.upgrade().is_some() && wo.upgrade().is_some());
    assert_eq!(
        (zone.borrow().nb_of_dung, zone.borrow().nb_of_feather),
        (17, 29)
    );
    {
        let _held = zone.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    }
    {
        let food = wl.upgrade().unwrap();
        let _held = food.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    }
    drop(zone);
    crate::bind_world::with_world(|_| {
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred)
    });
    pz_inventory::component_host::drain_retired_components();
    super::super::retry_pending();
    assert!(matches!(
        gc::collect_pending(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(
        wz.upgrade().is_none()
            && wl.upgrade().is_none()
            && wi.upgrade().is_none()
            && wo.upgrade().is_none()
            && wc.upgrade().is_none()
    );
    assert_eq!(*color.borrow(), [0.1, 0.3, 0.5, 0.7]);
}

#[test]
fn retaining_actual_food_list_roots_zone_cycle_but_empty_actual_position_pool_has_no_reverse_owner()
{
    let _restore = Restore::new();
    for retain_food in [true, false] {
        let (registry, zone, food, item, object, component) = graph();
        let pool = zone.borrow().position3d_pool.as_ref().unwrap().clone();
        let weak_zone = Rc::downgrade(&zone);
        let weak_item = Rc::downgrade(&item);
        reset(&registry);
        drop((registry, zone, item, object, component));
        let retained_food = retain_food.then_some(food.clone());
        drop(food);
        pz_inventory::component_host::drain_retired_components();
        {
            let _held = pool.borrow_mut();
            assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
        }
        assert!(matches!(
            gc::collect_tables(),
            gc::CollectionResult::Collected(_)
        ));
        assert_eq!(weak_zone.upgrade().is_some(), retain_food);
        assert_eq!(weak_item.upgrade().is_some(), retain_food);
        assert!(pool.borrow().released_positions.is_empty());
        drop(retained_food);
        assert!(matches!(
            gc::collect_tables(),
            gc::CollectionResult::Collected(_)
        ));
        assert!(weak_zone.upgrade().is_none() && weak_item.upgrade().is_none());
        assert!(
            pool.borrow().released_positions.is_empty(),
            "GC does not perform Native pool release/reset"
        );
    }
}
