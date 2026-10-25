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
    square.world_objects.extend([7, 7]);
    let square = SquareSlot::new(square);
    let copy = square.try_archive_copy().unwrap();
    let copy_body = copy.borrow();
    assert!(!square.ptr_eq(&copy));
    assert!(copy_body.objects[0].ptr_eq(&copy_body.objects[1]));
    assert!(!copy_body.objects[0].ptr_eq(&copy_body.objects[2]));
    assert!(!copy_body.objects[0].ptr_eq_ref(&actual));
    assert_eq!(copy_body.world_objects, [7, 7]);
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
