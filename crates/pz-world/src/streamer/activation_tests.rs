use super::*;
use crate::save::chunk_file::{finish_square_erosion, ChunkActivationStep, HookCtx};
use crate::save::items::AllItems;
use crate::save::NoSaveHooks;
use crate::{IsoGridSquare, IsoObject, NoHooks, ObjectKind, ObjectSlot};
use std::cell::RefCell;
use std::rc::Rc;

struct Rand;
impl GameRand for Rand {
    fn next(&mut self, _: i32) -> i32 {
        0
    }
}
struct Log(Rc<RefCell<Vec<&'static str>>>);
impl ChunkLoadHook for Log {
    fn load_grid_square(&mut self, _: &mut IsoChunk, _: i32, _: i32, _: i32, _: &mut HookCtx<'_>) {
        self.0.borrow_mut().push("erosion");
    }
    fn chunk_loaded(&mut self, _: &mut IsoChunk, _: &mut HookCtx<'_>) {
        self.0.borrow_mut().push("chunk");
    }
}

#[test]
fn deferred_handoff_waits_for_live_activation_and_observers_with_or_without_erosion() {
    for erosion in [false, true] {
        let dir = std::env::temp_dir().join(format!(
            "pz-activation-{}-{}-{erosion}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut meta = IsoMetaGrid::empty();
        meta.max_x = 1;
        meta.max_y = 1;
        let mut sprites = IsoSpriteManager::new(crate::sprite::test_tables());
        let fix2x = Fix2xTable::default();
        let mut rand = Rand;
        let object_tables = ObjectTables::new(vec![1; 8], vec![(0, 0)]).unwrap();
        let mut object_hooks = NoHooks;
        let save_tables = SaveTables::for_tests();
        let save_settings = SaveSettings::default();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut hook = Log(calls.clone());
        let mut ctx = StreamCtx {
            save_dir: &dir,
            meta: &meta,
            sprites: &mut sprites,
            fix2x: &fix2x,
            rand: &mut rand,
            object_tables: &object_tables,
            object_settings: ObjectLoadSettings::default(),
            object_hooks: &mut object_hooks,
            save_tables: &save_tables,
            save_settings: &save_settings,
            items: &AllItems,
            save_hooks: &NoSaveHooks,
            brand_new: None,
            chunk_hook: if erosion { Some(&mut hook) } else { None },
            no_save: true,
            vehicles_db: None,
            vehicle_saver: None,
            vehicle_meta: None,
            vehicle_ids: None,
        };
        let mut cell = IsoCell::default();
        let mut streamer = ChunkStreamer::new(1, 3);
        streamer.set_deferred_world_activation(true);
        streamer.load_world(2, 3, &mut cell, &mut ctx);
        assert_eq!(streamer.pending_world_activations(), 1);
        assert_eq!(
            cell.chunks.len(),
            1,
            "next chunk prepared before activation"
        );
        assert_eq!(streamer.load_grid_square.len(), 8);
        assert!(calls.borrow().is_empty());
        assert!(!streamer.take_events().iter().any(|e| matches!(
            e,
            StreamEvent::HandedOff { .. } | StreamEvent::GenAnimalsPath { .. }
        )));
        let mut cursor = streamer.pop_pending_world_activation().unwrap();
        let (wx, wy) = (cursor.wx, cursor.wy);
        assert!(
            !streamer.process_next_pending_load_grid_square(&mut cell, &mut ctx),
            "an engine-owned cursor must block the next preparation"
        );
        assert_eq!(
            streamer.finish_deferred_world_activation(&mut cursor, &mut cell, &mut ctx),
            Err(ChunkActivationError::IncompleteCursor)
        );
        let mut square = IsoGridSquare::new(wx * 8, wy * 8, 0);
        square
            .objects
            .push(ObjectSlot::new(IsoObject::new(3, ObjectKind::Object, None)));
        cell.chunks
            .get_mut(&(wx, wy))
            .unwrap()
            .set_square(0, 0, 0, Some(square.into()));
        loop {
            match cursor.next_step(&cell).unwrap() {
                ChunkActivationStep::Object { square, object, .. } => {
                    assert!(square.try_borrow_mut().is_ok());
                    object.borrow_mut().key_id = 19;
                    calls.borrow_mut().push("object");
                }
                ChunkActivationStep::SquareReady { square, x, y, z } => {
                    finish_square_erosion(
                        &mut cell,
                        wx,
                        wy,
                        x,
                        y,
                        z,
                        &square,
                        ctx.sprites,
                        chunk_hook(&mut ctx.chunk_hook),
                    )
                    .unwrap();
                    calls.borrow_mut().push("observer");
                }
                ChunkActivationStep::Complete => break,
            }
        }
        assert!(!streamer
            .take_events()
            .iter()
            .any(|e| matches!(e, StreamEvent::HandedOff { .. })));
        streamer
            .finish_deferred_world_activation(&mut cursor, &mut cell, &mut ctx)
            .unwrap();
        let expected = if erosion {
            vec!["object", "erosion", "observer", "chunk"]
        } else {
            vec!["object", "observer"]
        };
        assert_eq!(calls.borrow().as_slice(), expected.as_slice());
        assert_eq!(
            streamer
                .take_events()
                .iter()
                .filter(|e| matches!(e, StreamEvent::HandedOff { .. }))
                .count(),
            1
        );
        assert_eq!(
            streamer.finish_deferred_world_activation(&mut cursor, &mut cell, &mut ctx),
            Err(ChunkActivationError::AlreadyPublished)
        );
        assert!(streamer.take_events().is_empty());
        let mut completed = 1;
        while streamer.process_next_pending_load_grid_square(&mut cell, &mut ctx) {
            assert_eq!(streamer.pending_world_activations(), 1);
            assert_eq!(cell.chunks.len(), completed + 1);
            let mut cursor = streamer.pop_pending_world_activation().unwrap();
            assert!(matches!(
                cursor.next_step(&cell).unwrap(),
                ChunkActivationStep::Complete
            ));
            streamer
                .finish_deferred_world_activation(&mut cursor, &mut cell, &mut ctx)
                .unwrap();
            completed += 1;
        }
        assert_eq!(completed, 9);
        assert_eq!(cell.chunks.len(), 9);
        assert!(streamer.load_grid_square.is_empty());
        assert_eq!(
            streamer
                .take_events()
                .iter()
                .filter(|e| matches!(e, StreamEvent::HandedOff { .. }))
                .count(),
            8
        );

        // A normal frame grants its original count once: n=9,width=13
        // permits3 handoffs, not all9 and not a new grant after each cursor.
        let mut frame_streamer = ChunkStreamer::new(1, 13);
        frame_streamer.set_deferred_world_activation(true);
        frame_streamer.set_initial_pos(0, 2, 3);
        for wx in 1..=3 {
            for wy in 2..=4 {
                frame_streamer.load_chunk(0, wx, wy, wx + 4, wy + 3);
            }
        }
        frame_streamer.serve(&[], Serve::UntilIdle, &mut ctx);
        assert_eq!(frame_streamer.load_grid_square.len(), 9);
        let mut frame_cell = IsoCell::default();
        frame_streamer.chunk_map_update(0, &mut frame_cell, &mut ctx);
        let mut frame_count = 0;
        while let Some(mut cursor) = frame_streamer.pop_pending_world_activation() {
            assert!(matches!(
                cursor.next_step(&frame_cell).unwrap(),
                ChunkActivationStep::Complete
            ));
            frame_streamer
                .finish_deferred_world_activation(&mut cursor, &mut frame_cell, &mut ctx)
                .unwrap();
            frame_count += 1;
            frame_streamer.process_next_pending_load_grid_square(&mut frame_cell, &mut ctx);
        }
        assert_eq!(frame_count, 3);
        assert_eq!(frame_cell.chunks.len(), 3);
        assert_eq!(frame_streamer.load_grid_square.len(), 6);
        assert!(!frame_streamer.process_next_pending_load_grid_square(&mut frame_cell, &mut ctx));
        // The legacy default still finishes erosion and publishes synchronously.
        let mut legacy = ChunkStreamer::new(1, 1);
        let mut legacy_cell = IsoCell::default();
        legacy.load_world(2, 3, &mut legacy_cell, &mut ctx);
        assert_eq!(legacy.pending_world_activations(), 0);
        assert_eq!(
            legacy
                .take_events()
                .iter()
                .filter(|e| matches!(e, StreamEvent::HandedOff { wx: 2, wy: 3 }))
                .count(),
            1
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
