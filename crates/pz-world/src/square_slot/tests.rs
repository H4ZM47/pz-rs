use super::*;
use crate::objects::IsoDoor;
use crate::square::NoSquareHooks;
use crate::{GetSquare, IsoCell, IsoChunk, IsoObject, ObjectKind, SquareOwnerQueries};

fn square(x: i32, y: i32, id: u32) -> SquareSlot {
    let mut sq = IsoGridSquare::new(x, y, 0);
    sq.objects
        .push(IsoObject::new(id, ObjectKind::Object, None).into());
    sq.into()
}
#[test]
fn stored_square_alias_survives_replacement_level_removal_and_chunk_drop() {
    let slot = square(1, 1, 7);
    let actual = slot.ref_handle();
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(1, 1, 0, Some(slot));
    drop(chunk.take_retired_owners());
    assert!(Rc::ptr_eq(
        &actual,
        &chunk.get_grid_square(1, 1, 0).unwrap()
    ));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    assert!(Rc::ptr_eq(
        &actual,
        &GetSquare::get_grid_square(&cell, 1, 1, 0).unwrap()
    ));
    actual.borrow_mut().room_id = 55;
    cell.chunks
        .get_mut(&(0, 0))
        .unwrap()
        .set_square(1, 1, 0, Some(square(1, 1, 7)));
    let replacement = cell.get_grid_square(1, 1, 0).unwrap();
    assert!(!Rc::ptr_eq(&actual, &replacement));
    drop(cell.take_retired_owners());
    assert_eq!(actual.borrow().objects.len(), 1);
    assert_eq!(actual.borrow().room_id, 55);
    cell.chunks
        .get_mut(&(0, 0))
        .unwrap()
        .set_min_max_level(1, 1);
    drop(cell.take_retired_owners());
    drop(cell);
    assert_eq!(replacement.borrow().objects[0].borrow().id, 7);
    actual.borrow().objects[0].borrow_mut().key_id = 991;
    assert_ne!(replacement.borrow().objects[0].borrow().key_id, 991);
}
#[test]
fn square_archive_copy_has_independent_objects_and_no_pending_roots() {
    let slot = square(0, 0, 4);
    let actual = slot.ref_handle();
    let old = ObjectSlot::new(IsoObject::new(9, ObjectKind::Object, None));
    let weak = Rc::downgrade(&old.ref_handle());
    actual.borrow_mut().retire_object(old);
    let copy = slot.clone();
    assert!(!slot.ptr_eq(&copy));
    assert!(!slot.borrow().objects[0].ptr_eq(&copy.borrow().objects[0]));
    assert!(copy.borrow_mut().take_retired_objects().is_empty());
    actual.borrow_mut().room_id = 77;
    actual.borrow().objects[0].borrow_mut().key_id = 88;
    assert_ne!(copy.borrow().room_id, 77);
    assert_ne!(copy.borrow().objects[0].borrow().key_id, 88);
    drop(actual.borrow_mut().take_retired_objects());
    assert!(weak.upgrade().is_none());
}
#[test]
fn retirement_partitions_mixed_borrowed_batch_in_order_and_retries() {
    let handles: Vec<_> = (0..32)
        .map(|x| square(x, 0, x as u32).ref_handle())
        .collect();
    let mut pending: Vec<_> = handles.iter().map(SquareSlot::retain).collect();
    for s in &handles {
        s.borrow_mut()
            .retire_object(IsoObject::new(99, ObjectKind::Object, None).into());
    }
    let borrowed: Vec<_> = handles
        .iter()
        .skip(1)
        .step_by(2)
        .map(|s| s.borrow())
        .collect();
    let mut retired = RetiredWorldOwners::default();
    drain_squares(&mut pending, &mut retired);
    assert_eq!(
        retired
            .squares
            .iter()
            .map(|s| s.borrow().x)
            .collect::<Vec<_>>(),
        (0..32).step_by(2).collect::<Vec<_>>()
    );
    assert_eq!(
        pending.iter().map(|s| s.borrow().x).collect::<Vec<_>>(),
        (1..32).step_by(2).collect::<Vec<_>>()
    );
    assert_eq!(retired.objects.len(), 16);
    assert!(handles.iter().all(|s| s.borrow().objects.len() == 1));
    drop(borrowed);
    let mut retried = RetiredWorldOwners::default();
    drain_squares(&mut pending, &mut retried);
    assert!(pending.is_empty());
    assert_eq!(retried.objects.len(), 16);
    assert_eq!(
        retried
            .squares
            .iter()
            .map(|s| s.borrow().x)
            .collect::<Vec<_>>(),
        (1..32).step_by(2).collect::<Vec<_>>()
    );
}
#[test]
fn paired_edge_query_uses_retained_receiver_instead_of_coordinate_replacement() {
    let mut old = IsoGridSquare::new(1, 1, 0);
    old.objects.push(
        IsoObject::new(
            7,
            ObjectKind::Door(IsoDoor {
                north: true,
                ..Default::default()
            }),
            None,
        )
        .into(),
    );
    old.special_objects.push(7);
    let old = SquareSlot::new(old).ref_handle();
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(1, 1, 0, Some(square(1, 1, 7)));
    chunk.set_square(1, 0, 0, Some(square(1, 0, 8)));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    let other = cell.get_grid_square(1, 0, 0).unwrap();
    let (found, object) = old
        .get_door_to(Some(&other), &cell, &NoSquareHooks)
        .unwrap();
    assert!(Rc::ptr_eq(&old, &found));
    assert!(old.borrow().objects[0].ptr_eq_ref(&object));
    assert!(cell
        .get_grid_square(1, 1, 0)
        .unwrap()
        .get_door_to(Some(&other), &cell, &NoSquareHooks)
        .is_none());
}

#[test]
fn cell_retries_borrowed_retired_square_without_stripping_its_contents() {
    let old = square(0, 0, 3);
    let actual = old.ref_handle();
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(0, 0, 0, Some(old));
    drop(chunk.take_retired_owners());
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    let pending = ObjectSlot::new(IsoObject::new(22, ObjectKind::Object, None));
    let pending_weak = Rc::downgrade(&pending.ref_handle());
    actual.borrow_mut().retire_object(pending);
    let mut held = actual.borrow_mut();
    held.have_roof = true;
    cell.chunks
        .get_mut(&(0, 0))
        .unwrap()
        .set_square(0, 0, 0, Some(square(0, 0, 3)));
    let first = cell.take_retired_owners();
    assert!(first.squares.iter().all(|s| !s.ptr_eq_ref(&actual)));
    drop(first);
    assert!(pending_weak.upgrade().is_some());
    assert_eq!(held.objects.len(), 1);
    drop(held);
    let next = cell.take_retired_owners();
    assert!(next.squares.iter().any(|s| s.ptr_eq_ref(&actual)));
    assert!(pending_weak.upgrade().is_some());
    drop(next);
    assert!(pending_weak.upgrade().is_none());
    assert!(actual.borrow().have_roof);
    assert_eq!(actual.borrow().objects.len(), 1);
}

#[test]
fn terminal_never_installed_chunk_transfers_borrowed_square_retry_roots() {
    for previously_retired in [false, true] {
        let slot = square(0, 0, 8);
        let actual = slot.ref_handle();
        let mut chunk = IsoChunk::new(0, 0);
        chunk.set_square(0, 0, 0, Some(slot));
        drop(chunk.take_retired_owners());
        let pending = ObjectSlot::new(IsoObject::new(33, ObjectKind::Object, None));
        let weak = Rc::downgrade(&pending.ref_handle());
        actual.borrow_mut().retire_object(pending);
        let held = actual.borrow_mut();
        if previously_retired {
            chunk.set_square(0, 0, 0, Some(square(0, 0, 8)));
        }
        let mut cell = IsoCell::default();
        for slot in chunk.take_square_owners_for_retirement() {
            cell.retire_square(slot);
        }
        drop(chunk);
        let first = cell.take_retired_owners();
        assert!(first.squares.iter().all(|s| !s.ptr_eq_ref(&actual)));
        drop(first);
        assert!(weak.upgrade().is_some());
        assert_eq!(held.objects.len(), 1);
        drop(held);
        let retired = cell.take_retired_owners();
        assert!(retired.squares.iter().any(|s| s.ptr_eq_ref(&actual)));
        drop(retired);
        assert!(weak.upgrade().is_none());
        assert_eq!(actual.borrow().objects[0].borrow().id, 8);
    }
}
