use super::*;
use pz_entity::{Component, Components, EntityOwner, FluidContainer};
use pz_inventory::{ItemClass, ItemTemplate};
use pz_world::floor_object::FloorObject;
use pz_world::objects::IsoWorldInventoryObject;

struct Restore {
    item: Option<pz_inventory::shared::ItemObserver>,
    object: Option<pz_world::object_slot::ObjectObserver>,
}
impl Restore {
    fn new() -> Self {
        pz_inventory::component_host::drain_retired_components();
        let _ = gc::collect_tables();
        Self {
            item: pz_inventory::shared::install_item_observer(Some(observe_item)),
            object: pz_world::object_slot::install_object_observer(Some(observe_object)),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        pz_inventory::shared::install_item_observer(self.item);
        pz_world::object_slot::install_object_observer(self.object);
        pz_inventory::component_host::drain_retired_components();
        retry_pending();
        let _ = gc::collect_tables();
    }
}
fn graph() -> (ItemRef, IsoObjectRef, FloorObjectRef, ComponentRef) {
    let item = pz_inventory::shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "WorldCycle")),
        1,
    ))
    .unwrap();
    let floor = FloorObject::new(IsoWorldInventoryObject {
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
    let payload = floor.ref_handle();
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
    // Allocation observers have already registered actual object/payload/item
    // slots before this public field assignment closes the native cycle.
    pz_inventory::world_item::set_world_item_ref(&item, Some(&EntityOwner::from_ref(&object)))
        .unwrap();
    pz_inventory::component_host::drain_retired_components();
    (item, object, payload, component)
}

#[test]
fn each_real_world_cycle_root_preserves_the_same_graph_then_unrooted_cycle_collects() {
    let _restore = Restore::new();
    for root_kind in 0..5 {
        let (item, object, floor, component) = graph();
        let wi = Rc::downgrade(&item);
        let wo = Rc::downgrade(&object);
        let wf = Rc::downgrade(&floor);
        let wc = Rc::downgrade(&component);
        let retained: Box<dyn std::any::Any> = match root_kind {
            0 => Box::new(item.clone()),
            1 => Box::new(object.clone()),
            2 => Box::new(floor.clone()),
            3 => Box::new(component.clone()),
            _ => Box::new(crate::bind_world_object::object_value(&object).unwrap()),
        };
        drop(item);
        drop(object);
        drop(floor);
        drop(component);
        assert!(matches!(
            gc::collect_tables(),
            gc::CollectionResult::Collected(_)
        ));
        {
            let item = wi
                .upgrade()
                .expect("retained real root must preserve actual item");
            let object = wo
                .upgrade()
                .expect("retained real root must preserve full object");
            let floor = wf
                .upgrade()
                .expect("retained real root must preserve payload");
            assert!(wc.upgrade().is_some());
            assert!(item
                .borrow()
                .world_item_slot()
                .unwrap()
                .ptr_eq(&EntityOwner::from_ref(&object)));
            assert!(Rc::ptr_eq(floor.borrow().item.item_slot().unwrap(), &item));
        }
        drop(retained);
        assert!(matches!(
            gc::collect_tables(),
            gc::CollectionResult::Collected(_)
        ));
        assert!(wi.upgrade().is_none());
        assert!(wo.upgrade().is_none());
        assert!(wf.upgrade().is_none());
        assert!(wc.upgrade().is_none());
    }
}

#[test]
fn any_held_actual_item_object_or_floor_body_defers_collection_and_retry_is_metadata_only() {
    let _restore = Restore::new();
    let (item, object, floor, component) = graph();
    {
        let _guard = item.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    }
    {
        let _guard = object.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
        assert!(crate::bind_world_object::get_world_item(&item).is_ok());
    }
    {
        let _guard = floor.borrow_mut();
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
    }
    retry_pending();
    assert!(item
        .borrow()
        .world_item_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&object)));
    assert!(Rc::ptr_eq(floor.borrow().item.item_slot().unwrap(), &item));
    let weak = Rc::downgrade(&object);
    drop(item);
    drop(object);
    drop(floor);
    drop(component);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(weak.upgrade().is_none());
}

#[test]
fn retaining_actual_color_only_does_not_root_world_floor_item_or_component_or_reset_color() {
    let _restore = Restore::new();
    let (item, object, floor, component) = graph();
    let color = component.borrow().fluid_color_slot().unwrap().clone();
    *color.borrow_mut() = [0.2, 0.4, 0.6, 0.8];
    let wi = Rc::downgrade(&item);
    let wo = Rc::downgrade(&object);
    let wf = Rc::downgrade(&floor);
    let wc = Rc::downgrade(&component);
    drop(item);
    drop(object);
    drop(floor);
    drop(component);
    assert!(matches!(
        gc::collect_tables(),
        gc::CollectionResult::Collected(_)
    ));
    assert!(
        wi.upgrade().is_none()
            && wo.upgrade().is_none()
            && wf.upgrade().is_none()
            && wc.upgrade().is_none()
    );
    assert_eq!(*color.borrow(), [0.2, 0.4, 0.6, 0.8]);
}
