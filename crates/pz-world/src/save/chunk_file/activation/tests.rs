use super::*;
use crate::{IsoGridSquare, IsoObject, ObjectKind, ObjectSlot, SquareSlot};

fn object(id: u32) -> ObjectSlot {
    ObjectSlot::new(IsoObject::new(id, ObjectKind::Object, None))
}
fn cell() -> (IsoCell, SquareRef) {
    let mut square = IsoGridSquare::new(16, 24, 0);
    square.objects.push(object(1));
    let square = SquareSlot::new(square);
    let retained = square.ref_handle();
    let mut chunk = IsoChunk::new(2, 3);
    chunk.set_square(0, 0, 0, Some(square));
    let mut cell = IsoCell::default();
    cell.chunks.insert((2, 3), chunk);
    (cell, retained)
}

#[test]
fn live_object_index_observes_mutations_and_returns_guard_free_actual_receivers() {
    let (cell, square) = cell();
    let second = object(1);
    let second_ref = second.ref_handle();
    square.borrow_mut().objects.push(second);
    let mut cursor = ChunkActivationCursor::new(&cell.chunks[&(2, 3)]);
    let ChunkActivationStep::Object {
        square: selected,
        object: first,
        index: 0,
        ..
    } = cursor.next_step(&cell).unwrap()
    else {
        panic!()
    };
    assert!(Rc::ptr_eq(&square, &selected));
    assert!(selected.try_borrow_mut().is_ok());
    assert!(first.try_borrow_mut().is_ok());
    // Native index advances after the first call. Removing that slot shifts
    // second to index0; the appended actual third owner is reached at index1.
    let removed = square.borrow_mut().objects.remove(0);
    let third = object(1);
    let third_ref = third.ref_handle();
    square.borrow_mut().objects.push(third);
    let ChunkActivationStep::Object {
        object: actual,
        index: 1,
        ..
    } = cursor.next_step(&cell).unwrap()
    else {
        panic!()
    };
    assert!(Rc::ptr_eq(&actual, &third_ref));
    assert!(!Rc::ptr_eq(&actual, &second_ref));
    assert!(matches!(
        cursor.next_step(&cell).unwrap(),
        ChunkActivationStep::SquareReady { .. }
    ));
    assert!(matches!(
        cursor.next_step(&cell).unwrap(),
        ChunkActivationStep::Complete
    ));
    assert!(cursor.is_complete());
    drop(removed);
}

#[test]
fn level_then_x_then_y_order_and_each_observer_phase_follows_its_objects() {
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_min_max_level(-1, 1);
    for (id, (x, y, z)) in [
        (1, (1, 0, -1)),
        (2, (0, 2, 0)),
        (3, (0, 1, 0)),
        (4, (0, 0, 1)),
    ] {
        let mut square = IsoGridSquare::new(x, y, z);
        square.objects.push(object(id));
        chunk.set_square(x, y, z, Some(square.into()));
    }
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    let mut cursor = ChunkActivationCursor::new(&cell.chunks[&(0, 0)]);
    let mut phases = Vec::new();
    loop {
        match cursor.next_step(&cell).unwrap() {
            ChunkActivationStep::Object { x, y, z, .. } => phases.push(("object", x, y, z)),
            ChunkActivationStep::SquareReady { square, x, y, z } => {
                assert!(square.try_borrow_mut().is_ok());
                phases.push(("ready", x, y, z));
            }
            ChunkActivationStep::Complete => break,
        }
    }
    assert_eq!(
        phases,
        vec![
            ("object", 1, 0, -1),
            ("ready", 1, 0, -1),
            ("object", 0, 1, 0),
            ("ready", 0, 1, 0),
            ("object", 0, 2, 0),
            ("ready", 0, 2, 0),
            ("object", 0, 0, 1),
            ("ready", 0, 0, 1)
        ]
    );
}

#[test]
fn borrowed_square_retries_without_advancing_and_displaced_receiver_never_retargets() {
    let (mut cell, square) = cell();
    let mut cursor = ChunkActivationCursor::new(&cell.chunks[&(2, 3)]);
    let guard = square.borrow_mut();
    assert!(matches!(
        cursor.next_step(&cell),
        Err(ChunkActivationError::BorrowedSquare { .. })
    ));
    drop(guard);
    assert!(matches!(
        cursor.next_step(&cell).unwrap(),
        ChunkActivationStep::Object { index: 0, .. }
    ));
    let mut replacement = IsoGridSquare::new(16, 24, 0);
    replacement.objects.push(object(1));
    let replacement = SquareSlot::new(replacement);
    let replacement_ref = replacement.ref_handle();
    cell.chunks
        .get_mut(&(2, 3))
        .unwrap()
        .set_square(0, 0, 0, Some(replacement));
    assert!(matches!(
        cursor.next_step(&cell),
        Err(ChunkActivationError::DisplacedSquare { x: 0, y: 0, z: 0 })
    ));
    let old = cursor.skip_square().unwrap();
    assert!(Rc::ptr_eq(&old, &square));
    assert!(!Rc::ptr_eq(&old, &replacement_ref));
    let mut sprites = IsoSpriteManager::new(crate::sprite::test_tables());
    assert!(matches!(
        finish_square_erosion(&mut cell, 2, 3, 0, 0, 0, &old, &mut sprites, None),
        Err(ChunkActivationError::DisplacedSquare { .. })
    ));
    drop(old);
}

#[test]
fn optional_erosion_runs_after_activation_and_chunk_notification_is_explicit() {
    #[derive(Default)]
    struct Log(Vec<&'static str>);
    impl ChunkLoadHook for Log {
        fn load_grid_square(
            &mut self,
            chunk: &mut IsoChunk,
            x: i32,
            y: i32,
            z: i32,
            _: &mut HookCtx<'_>,
        ) {
            let square = chunk.get_grid_square(x, y, z).unwrap();
            assert!(square.try_borrow_mut().is_ok());
            self.0.push("erosion");
        }
        fn chunk_loaded(&mut self, _: &mut IsoChunk, _: &mut HookCtx<'_>) {
            self.0.push("chunk");
        }
    }
    let (mut cell, square) = cell();
    let mut cursor = ChunkActivationCursor::new(&cell.chunks[&(2, 3)]);
    let mut log = Log::default();
    let mut sprites = IsoSpriteManager::new(crate::sprite::test_tables());
    assert!(matches!(
        cursor.next_step(&cell).unwrap(),
        ChunkActivationStep::Object { .. }
    ));
    log.0.push("object");
    assert!(matches!(
        cursor.next_step(&cell).unwrap(),
        ChunkActivationStep::SquareReady { .. }
    ));
    finish_square_erosion(
        &mut cell,
        2,
        3,
        0,
        0,
        0,
        &square,
        &mut sprites,
        Some(&mut log),
    )
    .unwrap();
    log.0.push("observer");
    assert!(matches!(
        cursor.next_step(&cell).unwrap(),
        ChunkActivationStep::Complete
    ));
    assert_eq!(log.0, ["object", "erosion", "observer"]);
    finish_chunk_erosion(&mut cell, 2, 3, &mut sprites, Some(&mut log)).unwrap();
    assert_eq!(log.0, ["object", "erosion", "observer", "chunk"]);
    assert!(cell.chunks[&(2, 3)]
        .get_grid_square(0, 0, 0)
        .is_some_and(|s| Rc::ptr_eq(&s, &square)));
    finish_square_erosion(&mut cell, 2, 3, 0, 0, 0, &square, &mut sprites, None).unwrap();
}

#[test]
fn erosion_unwind_restores_published_actual_chunk_and_payload() {
    struct Panic;
    impl ChunkLoadHook for Panic {
        fn load_grid_square(
            &mut self,
            _: &mut IsoChunk,
            _: i32,
            _: i32,
            _: i32,
            _: &mut HookCtx<'_>,
        ) {
            panic!("authored erosion failure");
        }
        fn chunk_loaded(&mut self, _: &mut IsoChunk, _: &mut HookCtx<'_>) {}
    }
    let (mut cell, square) = cell();
    let mut sprites = IsoSpriteManager::new(crate::sprite::test_tables());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        finish_square_erosion(
            &mut cell,
            2,
            3,
            0,
            0,
            0,
            &square,
            &mut sprites,
            Some(&mut Panic),
        )
        .unwrap();
    }));
    assert!(result.is_err());
    assert!(cell.chunks[&(2, 3)]
        .get_grid_square(0, 0, 0)
        .is_some_and(|s| Rc::ptr_eq(&s, &square)));
    assert_eq!(square.borrow().objects.len(), 1);
}
