use super::*;
use crate::save::chunk_file::{load_square, save_square};
use pz_inventory::{shared, InventoryItem, ItemClass, ItemTemplate};
use std::rc::Rc;

#[test]
fn nonfloor_world_flag_keeps_partial_receiver_and_stops_after_its_record() {
    let sprites = sprites();
    let tables = SaveTables::for_tests();
    let settings = SaveSettings::default();
    let context = ctx(&sprites, &tables, &settings, &AllItems);

    for special in [false, true] {
        let mut source = IsoGridSquare::new(80, 160, 0);
        for key_id in [11, 22, 33] {
            let id = source.new_object_id();
            let mut object = IsoObject::new(id, ObjectKind::Object, None);
            object.key_id = key_id;
            source.objects.push(object.into());
        }
        let first_id = source.objects[0].borrow().id;
        let bad_id = source.objects[1].borrow().id;
        source.special_objects.push(first_id);
        if special {
            source.special_objects.push(bad_id);
        }
        // Raw native membership can flag a non-floor main object. Its save
        // succeeds; the floor cast happens only while loading that membership.
        source.world_objects.push(bad_id);
        let mut serialized = Writer::new();
        save_square(&mut serialized, &context, &source).unwrap();

        let mut prefix = Reader::new(&serialized.buf);
        ErosionSquareData::load(&mut prefix).unwrap();
        assert_eq!(prefix.i8().unwrap(), 1 | 4); // Three main records.
        let records_start = prefix.pos;
        let mut expected_records = Writer::new();
        expected_records.i8(2); // Valid first record is special only.
        save_object(&mut expected_records, &context, &source.objects[0].borrow()).unwrap();
        expected_records.i8(4 | if special { 2 } else { 0 });
        save_object(&mut expected_records, &context, &source.objects[1].borrow()).unwrap();
        let expected_stop = records_start + expected_records.buf.len();
        assert_eq!(
            &serialized.buf[records_start..expected_stop],
            expected_records.buf.as_slice()
        );

        let mut receiver = IsoGridSquare::new(80, 160, 0);
        let existing_id = receiver.new_object_id();
        let item = shared::new_item(InventoryItem::create(
            Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Existing")),
            700,
        ));
        let existing = ObjectSlot::new(IsoObject::new(
            existing_id,
            ObjectKind::WorldInventory(crate::floor_object::FloorObject::new(
                IsoWorldInventoryObject {
                    xoff: 0.25,
                    yoff: 0.5,
                    zoff: 0.0,
                    offset_x: 0.0,
                    offset_y: 0.0,
                    item: crate::floor_item::FloorItem::retain(&item),
                    drop_time: 1.0,
                    ignore_remove_sandbox: false,
                    extended_placement: false,
                },
            )),
            None,
        ));
        let retained = existing.ref_handle();
        receiver.objects.push(existing);
        receiver.special_objects.push(existing_id);
        receiver.world_objects.push(existing_id);
        receiver.overlay_done = true;
        receiver.have_roof = true;

        let mut reader = Reader::new(&serialized.buf);
        let error = load_square(&mut reader, &context, WORLD_VERSION, &mut receiver).unwrap_err();
        assert_eq!(
            error.0,
            "ClassCastException: world object flag on a non-item"
        );
        assert_eq!(reader.pos, expected_stop);
        assert_eq!(receiver.objects.len(), 3);
        assert!(receiver.objects[0].ptr_eq_ref(&retained));
        let first = receiver.objects[1].borrow();
        let bad = receiver.objects[2].borrow();
        assert_eq!((first.key_id, bad.key_id), (11, 22));
        let mut expected_special = vec![existing_id, first.id];
        if special {
            expected_special.push(bad.id);
        }
        assert_eq!(receiver.special_objects, expected_special);
        assert_eq!(receiver.world_objects, [existing_id]);
        // Header fields after the record loop have not been processed.
        assert!(receiver.overlay_done && receiver.have_roof);
        let ObjectKind::WorldInventory(floor) = &retained.borrow().kind else {
            panic!("existing receiver item was replaced")
        };
        assert!(Rc::ptr_eq(&floor.borrow().item.item_ref().unwrap(), &item));

        // The following record, including its membership byte, is untouched.
        assert_eq!(reader.i8().unwrap(), 0);
        let following = load_object(&mut reader, &context, WORLD_VERSION, 999)
            .unwrap()
            .unwrap();
        assert_eq!(following.key_id, 33);
    }
}
