use super::*;
use crate::floor_item::FloorItem;
use crate::objects::{IsoObject, IsoWorldInventoryObject};
use crate::{ObjectKind, ObjectSlot};
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use std::cell::Cell;

fn item(name: &str) -> ItemRef {
    pz_inventory::shared::new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", name)),
        1,
    ))
}
fn world(item: Option<&ItemRef>) -> IsoObjectRef {
    ObjectSlot::new(IsoObject::new(
        7,
        ObjectKind::WorldInventory(super::super::FloorObject::new(IsoWorldInventoryObject {
            xoff: 0.2,
            yoff: 0.3,
            zoff: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            item: item.map_or_else(FloorItem::null, FloorItem::retain),
            drop_time: -1.0,
            ignore_remove_sandbox: false,
            extended_placement: false,
        })),
        None,
    ))
    .ref_handle()
}
thread_local! { static CLASS: Cell<Option<&'static str>> = const { Cell::new(None) }; }
fn stamp(_: &IsoObjectRef, class: &'static str) {
    CLASS.with(|slot| slot.set(Some(class)));
}

#[test]
fn allocation_stamps_full_class_without_inventing_constructor_backlink() {
    let old = crate::object_slot::install_object_observer(Some(stamp));
    let item = item("Stamp");
    let object = world(Some(&item));
    assert!(item.borrow().world_item_slot().is_none());
    let guard = object.borrow_mut();
    assert_eq!(
        CLASS.with(Cell::get),
        Some("zombie.iso.objects.IsoWorldInventoryObject")
    );
    drop(guard);
    let mut retired = RetiredComponents::new();
    publish_item_backlink(&object, &item, &mut retired).unwrap();
    assert!(item
        .borrow()
        .world_item_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&object)));
    item.borrow_mut().replace_world_item(None, &mut retired);
    crate::object_slot::install_object_observer(old);
}

#[test]
fn physical_gc_takes_move_only_real_edges_and_keep_scalar_native_state() {
    let item = item("Edges");
    item.borrow_mut().condition = 41;
    let object = world(Some(&item));
    let floor = floor_ref(&object).unwrap();
    let mut retired = RetiredComponents::new();
    publish_item_backlink(&object, &item, &mut retired).unwrap();
    let edge = item.borrow_mut().take_world_item_for_gc().unwrap();
    assert!(edge.ptr_eq(&EntityOwner::from_ref(&object)));
    let owned_item = floor.borrow_mut().item.take_item_for_gc().unwrap();
    assert!(Rc::ptr_eq(&owned_item, &item));
    assert!(floor.borrow().item.is_null());
    assert_eq!(floor.borrow().xoff, 0.2);
    let owned_floor = match &mut object.borrow_mut().kind {
        ObjectKind::WorldInventory(slot) => slot.take_payload_for_gc().unwrap(),
        _ => unreachable!(),
    };
    assert!(Rc::ptr_eq(&owned_floor, &floor));
    assert_eq!(item.borrow().condition, 41);
    assert_eq!(object.borrow().id, 7);
}

#[derive(Default)]
struct Host {
    calls: Vec<&'static str>,
    fail_sound: bool,
}
impl SwapPrefixHost for Host {
    fn remove_process_item(&mut self, _: &ItemRef) -> HostResult<()> {
        self.calls.push("process");
        Ok(())
    }
    fn remove_item_sound(&mut self, _: &ItemRef) -> HostResult<()> {
        self.calls.push("sound");
        if self.fail_sound {
            Err(RuntimeHostError::Unsupported {
                operation: "authored sound fixture failure",
                kind: None,
            })
        } else {
            Ok(())
        }
    }
    fn item_name(&mut self, item: &ItemRef) -> HostResult<String> {
        self.calls.push("name");
        Ok(item.borrow().get_name().into())
    }
}

#[test]
fn swap_existing_backlink_error_keeps_native_post_assignment_prefix() {
    let old = item("Old");
    let incoming = item("New");
    {
        let mut old = old.borrow_mut();
        old.id = 99;
        old.world_scale = 0.7;
        old.world_x_rotation = 12.0;
        old.world_y_rotation = 13.0;
        old.world_z_rotation = 14.0;
    }
    let object = world(Some(&old));
    let other = world(None);
    let mut retired = RetiredComponents::new();
    publish_item_backlink(&object, &old, &mut retired).unwrap();
    publish_item_backlink(&other, &incoming, &mut retired).unwrap();
    let mut host = Host::default();
    let error = swap_item_prefix(&object, Some(&incoming), &mut host, &mut retired).unwrap_err();
    assert!(
        matches!(error, RuntimeHostError::NativeException(ref error) if error.0 == "IllegalArgumentException: newItem.getWorldItem() != null")
    );
    assert_eq!(host.calls, ["process", "sound"]);
    assert!(old.borrow().world_item_slot().is_none());
    assert!(Rc::ptr_eq(&item_ref(&object).unwrap().unwrap(), &incoming));
    assert!(incoming
        .borrow()
        .world_item_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&other)));
    let value = incoming.borrow();
    assert_eq!(
        (
            value.id,
            value.world_scale,
            value.world_x_rotation,
            value.world_y_rotation,
            value.world_z_rotation
        ),
        (99, 0.7, 12.0, 13.0, 14.0)
    );
    drop(value);
    incoming.borrow_mut().replace_world_item(None, &mut retired);
}

#[test]
fn swap_sound_failure_precedes_old_clear_and_new_assignment_and_null_does_nothing() {
    let old = item("Old");
    let incoming = item("New");
    let object = world(Some(&old));
    let mut retired = RetiredComponents::new();
    publish_item_backlink(&object, &old, &mut retired).unwrap();
    let mut host = Host {
        fail_sound: true,
        ..Default::default()
    };
    assert!(swap_item_prefix(&object, Some(&incoming), &mut host, &mut retired).is_err());
    assert_eq!(host.calls, ["process", "sound"]);
    assert!(Rc::ptr_eq(&item_ref(&object).unwrap().unwrap(), &old));
    assert!(old
        .borrow()
        .world_item_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&object)));
    let held = object.borrow_mut();
    assert!(swap_item_prefix(&object, None, &mut host, &mut retired)
        .unwrap()
        .is_none());
    drop(held);
    old.borrow_mut().replace_world_item(None, &mut retired);
}

#[test]
fn successful_swap_prefix_returns_actual_owned_continuation_not_completed_tail() {
    let old = item("Old");
    let incoming = item("New");
    let object = world(Some(&old));
    let mut retired = RetiredComponents::new();
    publish_item_backlink(&object, &old, &mut retired).unwrap();
    let continuation =
        swap_item_prefix(&object, Some(&incoming), &mut Host::default(), &mut retired)
            .unwrap()
            .unwrap();
    assert!(Rc::ptr_eq(&continuation.object, &object));
    assert!(Rc::ptr_eq(&continuation.item, &incoming));
    assert!(incoming
        .borrow()
        .world_item_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&object)));
    assert!(old.borrow().world_item_slot().is_none());
    incoming.borrow_mut().replace_world_item(None, &mut retired);
}

#[test]
fn copied_whole_floor_graph_binds_new_owner_without_retaining_source_link() {
    let item = item("Archive");
    let object = world(Some(&item));
    let mut retired = RetiredComponents::new();
    publish_item_backlink(&object, &item, &mut retired).unwrap();
    let copied = ObjectSlot::retain(&object).clone().ref_handle();
    let copied_item = item_ref(&copied).unwrap().unwrap();
    assert!(!Rc::ptr_eq(&item, &copied_item));
    assert!(!Rc::ptr_eq(&object, &copied));
    assert!(copied_item
        .borrow()
        .world_item_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&copied)));
    assert!(item
        .borrow()
        .world_item_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&object)));
    item.borrow_mut().replace_world_item(None, &mut retired);
    copied_item
        .borrow_mut()
        .replace_world_item(None, &mut retired);
    pz_inventory::component_host::drain_retired_components();
}
