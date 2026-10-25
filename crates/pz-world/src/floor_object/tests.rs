use std::rc::Rc;

use pz_inventory::{shared, InventoryItem, ItemClass, ItemTemplate};

use super::*;
use crate::floor_item::FloorItem;
use crate::{IsoChunk, IsoGridSquare, IsoObject, ObjectKind};

fn object(id: u32) -> IsoObject {
    let mut item = InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Container, "Base", "Bag")),
        1,
    );
    item.id = 7;
    let item = shared::new_item(item);
    let bag = shared::inventory_of(&item.borrow()).unwrap();
    let child = shared::new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Rock")),
        2,
    ));
    shared::add_item(&bag, &child);
    IsoObject::new(
        id,
        ObjectKind::WorldInventory(FloorObject::new(IsoWorldInventoryObject {
            xoff: 0.25,
            yoff: 0.5,
            zoff: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            item: FloorItem::retain(&item),
            drop_time: 2.5,
            ignore_remove_sandbox: false,
            extended_placement: false,
        })),
        None,
    )
}

fn handle(object: &IsoObject) -> FloorObjectRef {
    let ObjectKind::WorldInventory(floor) = &object.kind else {
        panic!()
    };
    floor.ref_handle()
}

#[test]
fn floor_object_explicit_handles_alias_but_archive_clone_is_deep() {
    let obj = object(1);
    let actual = handle(&obj);
    let retained = FloorObject::retain(&actual);
    assert!(Rc::ptr_eq(&actual, &retained.ref_handle()));
    let copied_object = obj.clone();
    let copied = handle(&copied_object);
    assert!(!Rc::ptr_eq(&actual, &copied));
    let item = actual.borrow().item.item_ref().unwrap();
    let copy_item = copied.borrow().item.item_ref().unwrap();
    assert!(!Rc::ptr_eq(&item, &copy_item));
    let inventory = shared::inventory_of(&item.borrow()).unwrap();
    let copied_inventory = shared::inventory_of(&copy_item.borrow()).unwrap();
    assert!(!Rc::ptr_eq(&inventory, &copied_inventory));
    assert!(Rc::ptr_eq(
        &copied_inventory
            .borrow()
            .containing_item
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap(),
        &copy_item
    ));
    assert!(!Rc::ptr_eq(
        &inventory.borrow().items[0],
        &copied_inventory.borrow().items[0]
    ));
    retained.borrow_mut().xoff = 0.75;
    retained.borrow_mut().extended_placement = true;
    retained.borrow().item.borrow_item_mut().unwrap().favorite = true;
    assert_eq!(actual.borrow().xoff, 0.75);
    assert!(actual.borrow().extended_placement);
    assert!(item.borrow().favorite);
    assert_eq!(copied.borrow().xoff, 0.25);
    assert!(!copied.borrow().extended_placement);
    assert!(!copy_item.borrow().favorite);
}

#[test]
fn retained_floor_object_survives_removal_moves_drop_and_same_id_replacement() {
    let original = object(1);
    let actual = handle(&original);
    let weak = Rc::downgrade(&actual);
    let mut square = IsoGridSquare::new(0, 0, 0);
    square.objects.push((original).into());
    square.world_objects.push(1);
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(0, 0, 0, Some((square).into()));
    let mut chunks = vec![chunk];
    chunks.reserve(64); // Chunks and their square/object vectors may move.
    assert!(Rc::ptr_eq(
        &actual,
        &handle(&chunks[0].get_grid_square(0, 0, 0).unwrap().borrow().objects[0].borrow())
    ));
    let detached = chunks[0]
        .get_grid_square_mut(0, 0, 0)
        .unwrap()
        .borrow_mut()
        .objects
        .remove(0);
    drop(detached);
    let replacement = object(1);
    let other = handle(&replacement);
    chunks[0]
        .get_grid_square_mut(0, 0, 0)
        .unwrap()
        .borrow_mut()
        .objects
        .push((replacement).into());
    assert!(!Rc::ptr_eq(&actual, &other));
    actual.borrow_mut().drop_time = 91.0;
    assert_eq!(other.borrow().drop_time, 2.5);
    let moved_chunk = chunks.pop().unwrap();
    drop(moved_chunk);
    assert_eq!(actual.borrow().drop_time, 91.0);
    assert!(weak.upgrade().is_some());
    drop(actual);
    assert!(weak.upgrade().is_none());
}

#[test]
fn archive_square_copies_in_distinct_chunks_have_independent_floor_allocations() {
    let mut square = IsoGridSquare::new(0, 0, 0);
    square.objects.push((object(1)).into());
    square.world_objects.push(1);
    let original = handle(&square.objects[0].borrow());
    let copy_square = square.try_archive_copy().unwrap();
    let copy = handle(&copy_square.objects[0].borrow());
    assert!(!Rc::ptr_eq(&original, &copy));
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(0, 0, 0, Some((square).into()));
    // IsoChunk deliberately has no Clone implementation. Its value-owned
    // square is explicitly copied into a separate archive chunk here.
    let mut copy_chunk = IsoChunk::new(0, 0);
    copy_chunk.set_square(
        0,
        0,
        0,
        Some(
            chunk
                .get_grid_square(0, 0, 0)
                .unwrap()
                .borrow()
                .try_archive_copy()
                .unwrap()
                .into(),
        ),
    );
    let copied_chunk_owner = handle(
        &copy_chunk
            .get_grid_square(0, 0, 0)
            .unwrap()
            .borrow()
            .objects[0]
            .borrow(),
    );
    assert!(!Rc::ptr_eq(&original, &copied_chunk_owner));
    original.borrow_mut().ignore_remove_sandbox = true;
    assert!(!copy.borrow().ignore_remove_sandbox);
    assert!(!copied_chunk_owner.borrow().ignore_remove_sandbox);
}

#[test]
fn floor_object_raw_payload_remains_opaque_and_archive_independent() {
    let obj = object(1);
    let actual = handle(&obj);
    let raw = vec![0, 3, 0xff, 0, 0, 0, 42, 0xde, 0xad];
    actual.borrow_mut().item = FloorItem::from_saved(pz_inventory::SavedItem::Raw(raw.clone()));
    let copy = obj.clone();
    let copied = handle(&copy);
    assert!(!Rc::ptr_eq(&actual, &copied));
    assert!(actual.borrow().item.item_ref().is_none());
    assert!(copied.borrow().item.item_ref().is_none());
    let env = pz_inventory::env::NoItemEnv;
    assert_eq!(actual.borrow().item.record(&env).unwrap(), raw);
    assert_eq!(copied.borrow().item.record(&env).unwrap(), raw);
    actual.borrow_mut().ignore_remove_sandbox = true;
    assert!(!copied.borrow().ignore_remove_sandbox);
}
