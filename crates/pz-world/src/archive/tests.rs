use super::*;
use crate::floor_item::FloorItem;
use crate::floor_object::FloorObject;
use crate::objects::{IsoObject, IsoWorldInventoryObject};
use pz_inventory::archive::ArchiveCopyError;
use pz_inventory::shared::{self, ItemRef};
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use std::rc::Rc;

fn item(class: ItemClass) -> ItemRef {
    shared::new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(class, "Base", "ArchiveTest")),
        1,
    ))
}

fn floor(item: &ItemRef) -> ObjectSlot {
    ObjectSlot::new(IsoObject::new(
        7,
        ObjectKind::WorldInventory(FloorObject::new(IsoWorldInventoryObject {
            xoff: 0.0,
            yoff: 0.0,
            zoff: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            item: FloorItem::retain(item),
            drop_time: 0.0,
            ignore_remove_sandbox: false,
            extended_placement: false,
        })),
        None,
    ))
}

#[test]
fn archive_maps_duplicate_whole_objects_without_merging_equal_distinct_owners() {
    let mut square = IsoGridSquare::new(1, 2, 0);
    let bag = item(ItemClass::Container);
    let child = item(ItemClass::Key);
    shared::add_item(&shared::inventory_of(&bag.borrow()).unwrap(), &child);
    let actual = floor(&bag).ref_handle();
    square.objects.push(ObjectSlot::retain(&actual));
    square.objects.push(ObjectSlot::retain(&actual));
    // Same values and ID, but an independent actual object allocation.
    square.objects.push(floor(&bag));
    square.world_objects.borrow_mut().push_object(&actual);
    square.world_objects.borrow_mut().push_object(&actual);
    let detached = floor(&bag).ref_handle();
    square.world_objects.borrow_mut().push_object(&detached);
    square.world_objects.borrow_mut().push_object(&actual);
    let square = SquareSlot::new(square);
    let copy = square.try_archive_copy().unwrap();
    let copy_body = copy.borrow();
    assert!(!square.ptr_eq(&copy));
    assert!(copy_body.objects[0].ptr_eq(&copy_body.objects[1]));
    assert!(!copy_body.objects[0].ptr_eq(&copy_body.objects[2]));
    assert!(!copy_body.objects[0].ptr_eq_ref(&actual));
    assert!(!Rc::ptr_eq(
        &square.borrow().world_objects,
        &copy_body.world_objects
    ));
    for index in [0, 1, 3] {
        let Some(crate::WorldObjectEntry::Object(owner)) =
            copy_body.world_objects.borrow().entry(index)
        else {
            panic!()
        };
        assert!(copy_body.objects[0].ptr_eq_ref(&owner));
    }
    let Some(crate::WorldObjectEntry::Object(detached_copy)) =
        copy_body.world_objects.borrow().entry(2)
    else {
        panic!()
    };
    assert!(!Rc::ptr_eq(&detached, &detached_copy));
    assert!(!copy_body.objects[0].ptr_eq_ref(&detached_copy));
    assert!(!copy_body.objects[2].ptr_eq_ref(&detached_copy));
    let copied = copy_body.objects[0].borrow();
    let ObjectKind::WorldInventory(copied_floor) = &copied.kind else {
        panic!()
    };
    let copied_bag = copied_floor.borrow().item.item_ref().unwrap();
    assert!(!Rc::ptr_eq(&bag, &copied_bag));
    let copied_inventory = shared::inventory_of(&copied_bag.borrow()).unwrap();
    assert!(!Rc::ptr_eq(&child, &copied_inventory.borrow().items[0]));
    child.borrow_mut().key_id = 999;
    assert_ne!(copied_inventory.borrow().items[0].borrow().key_id, 999);
    assert!(copy_body.objects[0].ptr_eq(&copy_body.objects[1]));
}

struct OriginalObject(crate::IsoObjectRef);
impl pz_kahlua::LuaObject for OriginalObject {
    fn class_name(&self) -> &str {
        panic!("archive must not invoke foreign methods")
    }
    fn as_any(&self) -> &dyn std::any::Any {
        panic!("archive must not inspect foreign payloads")
    }
}

#[test]
fn default_archive_rejects_first_foreign_index_without_mutating_any_memberships() {
    use pz_kahlua::{TableRef, Value};
    let mut square = IsoGridSquare::new(0, 0, 0);
    let actual = floor(&item(ItemClass::Key)).ref_handle();
    square.objects.push(ObjectSlot::retain(&actual));
    let foreign = Rc::new(OriginalObject(actual.clone()));
    for value in [
        Value::Nil,
        Value::Num(3.0),
        Value::Table(TableRef::new()),
        Value::Object(foreign.clone()),
    ] {
        let kind = value.type_name();
        square.world_objects.borrow_mut().push_object(&actual);
        square
            .world_objects
            .borrow_mut()
            .push_entry(crate::WorldObjectEntry::Foreign(value));
        square.world_objects.borrow_mut().push_object(&actual);
        assert_eq!(
            square.try_archive_copy().unwrap_err(),
            SquareArchiveError::UnsupportedForeign { index: 1, kind }
        );
        assert_eq!(square.objects.len(), 1);
        assert!(square.objects[0].ptr_eq_ref(&actual));
        assert_eq!(square.world_objects.borrow().len(), 3);
        let removed = square.world_objects.borrow_mut().take_entries();
        drop(removed);
    }
    assert!(Rc::ptr_eq(&foreign.0, &actual));
}

#[test]
fn explicit_foreign_retention_shares_exact_vm_referents_but_not_native_owners_or_list() {
    use pz_kahlua::{TableRef, Value};
    let mut source = IsoGridSquare::new(0, 0, 0);
    let actual = floor(&item(ItemClass::Key)).ref_handle();
    source.objects.push(ObjectSlot::retain(&actual));
    let original = Rc::new(OriginalObject(actual.clone()));
    let table = TableRef::new();
    table.set_str("original", Value::Object(original.clone()));
    source.world_objects.borrow_mut().push_object(&actual);
    source
        .world_objects
        .borrow_mut()
        .push_entry(crate::WorldObjectEntry::Foreign(Value::Table(
            table.clone(),
        )));
    source
        .world_objects
        .borrow_mut()
        .push_entry(crate::WorldObjectEntry::Foreign(Value::Object(
            original.clone(),
        )));
    source
        .world_objects
        .borrow_mut()
        .push_entry(crate::WorldObjectEntry::Foreign(Value::Nil));
    let copy = source
        .try_archive_copy_with_policy(ForeignArchivePolicy::RetainForeignHandles)
        .unwrap();
    assert!(!Rc::ptr_eq(&source.world_objects, &copy.world_objects));
    assert!(!copy.objects[0].ptr_eq_ref(&actual));
    let Some(crate::WorldObjectEntry::Object(mapped)) = copy.world_objects.borrow().entry(0) else {
        panic!()
    };
    assert!(copy.objects[0].ptr_eq_ref(&mapped));
    let Some(crate::WorldObjectEntry::Foreign(Value::Table(retained))) =
        copy.world_objects.borrow().entry(1)
    else {
        panic!()
    };
    assert!(retained.ptr_eq(&table));
    retained.set_str("shared", 91);
    assert_eq!(table.get_str("shared").as_num(), Some(91.0));
    let Value::Object(inside) = retained.get_str("original") else {
        panic!()
    };
    let expected: Rc<dyn pz_kahlua::LuaObject> = original.clone();
    assert!(Rc::ptr_eq(&inside, &expected));
    let Some(crate::WorldObjectEntry::Foreign(Value::Object(opaque))) =
        copy.world_objects.borrow().entry(2)
    else {
        panic!()
    };
    assert!(Rc::ptr_eq(&opaque, &expected));
    assert!(Rc::ptr_eq(&original.0, &actual));
    let removed = copy.world_objects.borrow_mut().take_entries();
    drop(removed);
    assert_eq!(source.world_objects.borrow().len(), 4);
}

#[test]
fn archive_checks_borrowed_and_cyclic_detached_native_members() {
    let source = IsoGridSquare::new(0, 0, 0);
    let bag = item(ItemClass::Container);
    let inventory = shared::inventory_of(&bag.borrow()).unwrap();
    let detached = floor(&bag).ref_handle();
    source.world_objects.borrow_mut().push_object(&detached);
    let list_guard = source.world_objects.borrow_mut();
    assert_eq!(
        source.try_archive_copy().unwrap_err(),
        SquareArchiveError::BorrowedWorldObjects
    );
    drop(list_guard);
    let guard = detached.borrow_mut();
    assert_eq!(
        source.try_archive_copy().unwrap_err(),
        SquareArchiveError::BorrowedWorldObject(0)
    );
    drop(guard);
    let guard = inventory.borrow_mut();
    assert_eq!(
        source.try_archive_copy().unwrap_err(),
        SquareArchiveError::Inventory {
            location: ArchiveLocation::WorldObject(0),
            cause: ArchiveCopyError::BorrowedContainer,
        }
    );
    drop(guard);
    inventory.borrow_mut().items.push(bag.clone());
    assert_eq!(
        source.try_archive_copy().unwrap_err(),
        SquareArchiveError::Inventory {
            location: ArchiveLocation::WorldObject(0),
            cause: ArchiveCopyError::CyclicContainer,
        }
    );
    inventory.borrow_mut().items.clear();
    assert!(source.try_archive_copy().is_ok());
    assert!(source.objects.is_empty());
}

#[test]
fn archive_reports_square_object_floor_and_nested_inventory_borrows() {
    let bag = item(ItemClass::Container);
    let child = item(ItemClass::Key);
    let inventory = shared::inventory_of(&bag.borrow()).unwrap();
    shared::add_item(&inventory, &child);
    let object = floor(&bag).ref_handle();
    let payload = match &object.borrow().kind {
        ObjectKind::WorldInventory(floor) => floor.ref_handle(),
        _ => unreachable!(),
    };
    let mut body = IsoGridSquare::new(0, 0, 0);
    body.objects.push(ObjectSlot::retain(&object));
    let square = SquareSlot::new(body);

    let guard = square.borrow_mut();
    assert_eq!(
        square.try_archive_copy().unwrap_err(),
        SquareArchiveError::BorrowedSquare
    );
    drop(guard);
    let guard = object.borrow_mut();
    assert_eq!(
        square.try_archive_copy().unwrap_err(),
        SquareArchiveError::BorrowedObject(0)
    );
    drop(guard);
    let guard = payload.borrow_mut();
    assert_eq!(
        square.try_archive_copy().unwrap_err(),
        SquareArchiveError::BorrowedFloor(0)
    );
    drop(guard);
    let location = ArchiveLocation::Object(0);
    let guard = child.borrow_mut();
    assert_eq!(
        square.try_archive_copy().unwrap_err(),
        SquareArchiveError::Inventory {
            location,
            cause: ArchiveCopyError::BorrowedItem,
        }
    );
    drop(guard);
    let guard = inventory.borrow_mut();
    assert_eq!(
        square.try_archive_copy().unwrap_err(),
        SquareArchiveError::Inventory {
            location,
            cause: ArchiveCopyError::BorrowedContainer,
        }
    );
    drop(guard);
    assert!(square.try_archive_copy().is_ok());
    assert!(square.borrow().objects[0].ptr_eq_ref(&object));
    assert!(Rc::ptr_eq(&inventory.borrow().items[0], &child));
}

#[test]
fn archive_checks_saved_bags_in_base_and_secondary_containers_before_copying() {
    for secondary in [false, true] {
        let value = InventoryItem::create(
            Rc::new(ItemTemplate::new(
                ItemClass::Container,
                "Base",
                "ArchiveBag",
            )),
            1,
        );
        let inventory = shared::inventory_of(&value).unwrap();
        let mut container = pz_inventory::ItemContainer::new(Some("fixture"));
        container
            .items
            .push(pz_inventory::SavedItem::Item(Box::new(value)));
        let mut object = IsoObject::new(3, ObjectKind::Object, None);
        if secondary {
            object.secondary_containers.push(container);
        } else {
            object.container = Some(container);
        }
        let mut square = IsoGridSquare::new(0, 0, 0);
        square.objects.push(object.into());
        let guard = inventory.borrow_mut();
        assert_eq!(
            square.try_archive_copy().unwrap_err(),
            SquareArchiveError::Inventory {
                location: ArchiveLocation::Object(0),
                cause: ArchiveCopyError::BorrowedContainer,
            }
        );
        drop(guard);
        assert!(square.try_archive_copy().is_ok());
    }
}

#[test]
fn archive_checks_nested_saved_items_in_traps_chargers_and_hutch_eggs() {
    use crate::objects::{HutchMaster, IsoCarBatteryCharger, IsoHutch, IsoTrap, NestBox};
    for case in 0..4 {
        let value = InventoryItem::create(
            Rc::new(ItemTemplate::new(ItemClass::Container, "Base", "SavedBag")),
            1,
        );
        let inventory = shared::inventory_of(&value).unwrap();
        let saved = pz_inventory::SavedItem::Item(Box::new(value));
        let kind = match case {
            0 => ObjectKind::Trap(Box::new(IsoTrap {
                sensor_range: 0,
                fire_starting_chance: 0,
                fire_starting_energy: 0,
                fire_range: 0,
                explosion_power: 0,
                explosion_range: 0,
                explosion_duration: 0,
                explosion_start_time: 0.0,
                smoke_range: 0,
                noise_range: 0,
                noise_duration: 0,
                noise_start_time: 0.0,
                extra_damage: 0.0,
                remote_control_id: 0,
                timer_before_explosion: 0,
                count_down_sound: String::new(),
                explosion_sound: String::new(),
                weapon: Some(saved),
            })),
            1 | 2 => {
                let (item, battery) = if case == 1 {
                    (Some(saved), None)
                } else {
                    (None, Some(saved))
                };
                ObjectKind::CarBatteryCharger(Box::new(IsoCarBatteryCharger {
                    item,
                    battery,
                    activated: false,
                    last_update: 0.0,
                    charge_rate: 0.0,
                }))
            }
            _ => ObjectKind::Hutch(Box::new(IsoHutch {
                linked_x: 0,
                linked_y: 0,
                linked_z: 0,
                master: Some(HutchMaster {
                    sprite_name: String::new(),
                    open: false,
                    open_egg_hatch: false,
                    saved_x: 0,
                    saved_y: 0,
                    saved_z: 0,
                    animals: Vec::new(),
                    hutch_dirt: 0.0,
                    nest_box_dirt: 0.0,
                    nest_boxes: vec![NestBox {
                        eggs: vec![saved],
                        animal: None,
                    }],
                }),
            })),
        };
        let mut square = IsoGridSquare::new(0, 0, 0);
        square.objects.push(IsoObject::new(7, kind, None).into());
        let guard = inventory.borrow_mut();
        assert_eq!(
            square.try_archive_copy().unwrap_err(),
            SquareArchiveError::Inventory {
                location: ArchiveLocation::Object(0),
                cause: ArchiveCopyError::BorrowedContainer,
            }
        );
        drop(guard);
        assert!(square.try_archive_copy().is_ok());
    }
}

#[test]
fn archive_checks_pushable_codec_table_depth() {
    use pz_inventory::lua_table::{LuaKey, LuaTable, LuaValue};
    let mut table = LuaTable::default();
    for _ in 0..66 {
        table = LuaTable {
            entries: vec![(LuaKey::Str("nested".into()), LuaValue::Table(table))],
        };
    }
    let pushable = crate::save::movers::IsoPushableObject {
        wheelie_bin: false,
        base: crate::save::visual::MovingBase {
            offset_x: 0.0,
            offset_y: 0.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            dir: 0,
            mod_data: Some(table),
        },
    };
    let mut square = IsoGridSquare::new(0, 0, 0);
    square
        .objects
        .push(IsoObject::new(7, ObjectKind::Pushable(Box::new(pushable)), None).into());
    assert_eq!(
        square.try_archive_copy().unwrap_err(),
        SquareArchiveError::Inventory {
            location: ArchiveLocation::Object(0),
            cause: ArchiveCopyError::DepthLimit,
        }
    );
}
