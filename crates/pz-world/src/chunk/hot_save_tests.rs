use super::*;
use crate::ObjectSlot;

#[test]
fn retained_square_flags_original_chunk_after_coordinate_replacement() {
    let mut old = IsoChunk::new(0, 0);
    let square = SquareSlot::new(IsoGridSquare::new(1, 1, 0));
    let original_square = square.ref_handle();
    old.set_square(1, 1, 0, Some(square));
    let original_state = old.hot_save.clone();
    let mut replacement = IsoChunk::new(0, 0);
    replacement.set_square(1, 1, 0, Some(IsoGridSquare::new(1, 1, 0).into()));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), old);
    let old = cell.chunks.insert((0, 0), replacement).unwrap();
    drop(old);
    let mut body = crate::IsoObject::new(1, crate::ObjectKind::Object, None);
    let mut retired = pz_entity::RetiredComponents::new();
    body.replace_square(Some(&original_square), &mut retired);
    let object = ObjectSlot::new(body).ref_handle();
    crate::square_mutation::try_flag_object_for_hot_save(&object).unwrap();
    assert!(original_state.borrow().requires_hot_save);
    assert!(!cell.chunks[&(0, 0)].hot_save.borrow().requires_hot_save);
    original_state.borrow_mut().requires_hot_save = false;
    original_state.borrow_mut().prevent_hot_save = true;
    crate::square_mutation::try_flag_object_for_hot_save(&object).unwrap();
    assert!(!original_state.borrow().requires_hot_save);
}

#[test]
fn archive_square_is_detached_then_binds_destination_actual_chunk_state() {
    let mut source = IsoChunk::new(0, 0);
    source.set_square(1, 1, 0, Some(IsoGridSquare::new(1, 1, 0).into()));
    let original = source.get_grid_square(1, 1, 0).unwrap();
    let copied = original.borrow().try_archive_copy().unwrap();
    assert!(copied.chunk_hot_save.is_none());
    let mut destination = IsoChunk::new(0, 0);
    destination.set_square(1, 1, 0, Some(copied.into()));
    let copy = destination.get_grid_square(1, 1, 0).unwrap();
    assert!(Rc::ptr_eq(
        copy.borrow().chunk_hot_save.as_ref().unwrap(),
        &destination.hot_save
    ));
    assert!(!Rc::ptr_eq(
        copy.borrow().chunk_hot_save.as_ref().unwrap(),
        original.borrow().chunk_hot_save.as_ref().unwrap()
    ));
}

#[test]
fn hot_save_borrow_failure_never_flags_replacement_or_clears_existing_state() {
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(1, 1, 0, Some(IsoGridSquare::new(1, 1, 0).into()));
    let square = chunk.get_grid_square(1, 1, 0).unwrap();
    let mut body = crate::IsoObject::new(1, crate::ObjectKind::Object, None);
    body.replace_square(Some(&square), &mut pz_entity::RetiredComponents::new());
    let object = ObjectSlot::new(body).ref_handle();
    let guard = chunk.hot_save.borrow_mut();
    assert_eq!(
        crate::square_mutation::try_flag_object_for_hot_save(&object),
        Err(pz_entity::runtime_load::RuntimeHostError::Borrowed(
            "flagForHotSave actual chunk"
        ))
    );
    drop(guard);
    assert!(!chunk.hot_save.borrow().requires_hot_save);
    crate::square_mutation::try_flag_object_for_hot_save(&object).unwrap();
    assert!(chunk.hot_save.borrow().requires_hot_save);
}

#[test]
fn held_incoming_square_is_retired_without_publishing_chunk_membership() {
    let mut chunk = IsoChunk::new(0, 0);
    let slot = SquareSlot::new(IsoGridSquare::new(1, 1, 0));
    let square = slot.ref_handle();
    let guard = square.borrow();
    assert_eq!(
        chunk.try_set_square(1, 1, 0, Some(slot)),
        Err(pz_entity::runtime_load::RuntimeHostError::Borrowed(
            "set_square chunk association"
        ))
    );
    assert!(chunk.get_grid_square(1, 1, 0).is_none());
    assert!(guard.chunk_hot_save.is_none());
    drop(guard);
    assert!(!chunk.retired_squares.is_empty());
}
