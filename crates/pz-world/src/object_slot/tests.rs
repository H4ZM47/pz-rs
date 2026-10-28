use super::*;
use crate::ObjectSlot;
use crate::{IsoGridSquare, ObjectKind};

#[test]
fn runtime_alias_and_archive_copy_are_distinct() {
    let slot = ObjectSlot::new(IsoObject::new(7, ObjectKind::Object, None));
    let actual = slot.ref_handle();
    let alias = ObjectSlot::retain(&actual);
    let archive = slot.clone();
    assert!(slot.ptr_eq(&alias));
    assert!(!slot.ptr_eq(&archive));
    actual.borrow_mut().name = Some(crate::objects::ObjectName::Str("late".into()));
    assert_eq!(alias.borrow().name, actual.borrow().name);
    assert_ne!(archive.borrow().name, actual.borrow().name);
}

#[test]
fn square_move_remove_and_same_id_replacement_do_not_retarget_handles() {
    let slot = ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, None));
    let actual = slot.ref_handle();
    let mut square = IsoGridSquare::new(1, 2, 0);
    square.objects.push((slot).into());
    let archive = square.try_archive_copy().unwrap();
    assert!(!archive.objects[0].ptr_eq_ref(&actual));
    let removed = square.objects.remove(0);
    square
        .objects
        .push((ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, None))).into());
    drop(square);
    assert!(removed.ptr_eq_ref(&actual));
    actual.borrow_mut().key_id = 901;
    assert_eq!(removed.borrow().key_id, 901);
    assert_ne!(archive.objects[0].borrow().key_id, 901);
}

#[test]
fn discarded_objects_retire_through_square_chunk_and_cell_without_entering_archive_clones() {
    use crate::{IsoCell, IsoChunk};
    for operation in [
        "remove",
        "replace",
        "clear",
        "replace-square",
        "shrink-level",
    ] {
        let mut square = IsoGridSquare::new(0, 0, 0);
        let slot = ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, None));
        let weak = Rc::downgrade(&slot.ref_handle());
        square.objects.push(slot);
        let mut chunk = IsoChunk::new(0, 0);
        chunk.set_square(0, 0, 0, Some((square).into()));
        // Discard placement's temporary unwind pin: the actual slot is still live.
        drop(chunk.take_retired_owners());
        let mut cell = IsoCell::default();
        cell.chunks.insert((0, 0), chunk);
        match operation {
            "replace-square" => cell.chunks.get_mut(&(0, 0)).unwrap().set_square(
                0,
                0,
                0,
                Some((IsoGridSquare::new(0, 0, 0)).into()),
            ),
            "shrink-level" => cell
                .chunks
                .get_mut(&(0, 0))
                .unwrap()
                .set_min_max_level(1, 1),
            _ => {
                let square_owner = cell.get_grid_square_mut(0, 0, 0).unwrap();
                let mut square = square_owner.borrow_mut();
                match operation {
                    "remove" => drop(square.remove_object_slot(0)),
                    "replace" => square.replace_object_slot(
                        0,
                        ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, None)),
                    ),
                    "clear" => square.clear_objects(),
                    _ => unreachable!(),
                }
                let mut archive = square.try_archive_copy().unwrap();
                assert!(archive.take_retired_objects().is_empty());
            }
        }
        assert!(weak.upgrade().is_some(), "{operation}");
        let retired = cell.take_retired_owners();
        assert!(weak.upgrade().is_some());
        assert!(cell.take_retired_owners().is_empty());
        drop(retired);
        assert!(weak.upgrade().is_none(), "{operation}");
    }
}

#[test]
fn incoming_square_owner_survives_index_unwind_until_explicit_retirement() {
    let mut chunk = crate::IsoChunk::new(0, 0);
    let mut square = IsoGridSquare::new(0, 0, 0);
    let slot = ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, None));
    let weak = Rc::downgrade(&slot.ref_handle());
    square.objects.push(slot);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        chunk.set_square(-1, 0, 0, Some((square).into()))
    }));
    assert!(result.is_err());
    assert!(weak.upgrade().is_some());
    let retired = chunk.take_retired_owners();
    assert!(weak.upgrade().is_some());
    drop(retired);
    assert!(weak.upgrade().is_none());
}
