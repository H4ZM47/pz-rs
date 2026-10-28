use super::*;
use pz_lua_api::bind_world::WorldState;
use pz_world::objects::ObjectTables;
use pz_world::save::SaveTables;
use pz_world::streamer::{ChunkStreamer, StreamEvent};
use pz_world::{
    IsoCell, IsoGridSquare, IsoObject, IsoSpriteManager, ObjectKind, ObjectSlot, SquareSlot,
};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Restore {
    world: Option<WorldState>,
    core: Rc<RefCell<pz_core::core::Core>>,
    dir: PathBuf,
}
impl Drop for Restore {
    fn drop(&mut self) {
        let retired = with_world(|world| std::mem::replace(world, self.world.take().unwrap()));
        drop(retired);
        pz_lua_api::bind_core::install_core(self.core.clone());
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn failed_activation_preserves_cursor_pins_and_blocks_later_handoffs() {
    let dir = std::env::temp_dir().join(format!(
        "pz-activation-error-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).unwrap();
    let _restore = Restore {
        world: Some(with_world(std::mem::take)),
        core: pz_lua_api::bind_core::shared_core(),
        dir: dir.clone(),
    };
    let mut core = pz_core::core::Core::default();
    core.set_no_save(true);
    pz_lua_api::bind_core::install_core(Rc::new(RefCell::new(core)));
    let mut meta = pz_world::meta_grid::IsoMetaGrid::empty();
    (meta.max_x, meta.max_y) = (1, 1);
    with_world(|world| {
        world.cell = Some(IsoCell::default());
        world.meta_grid = Some(meta);
    });
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&dir)));
    fs.borrow_mut().set_cache_dir(&dir);
    let mut lua = LuaManager::init(fs).unwrap();
    let mut world = IsoWorld::new(&dir, &dir, dir.clone());
    world.sprites = Some(Rc::new(RefCell::new(IsoSpriteManager::new(
        pz_world::sprite::test_tables(),
    ))));
    world.object_tables = Some(ObjectTables::new(vec![1; 8], vec![(0, 0)]).unwrap());
    world.save_tables = Some(SaveTables::from_text("0:IsoObject\nbloodBodyParts=7").unwrap());
    world.streamer = Some(ChunkStreamer::new(1, 3));
    world
        .streamer
        .as_mut()
        .unwrap()
        .set_deferred_world_activation(true);
    world
        .with_stream(|streamer, cell, ctx| streamer.load_world(2, 3, cell, ctx))
        .unwrap();
    let (wx, wy) = with_world(|state| {
        let cell = state.cell.as_ref().unwrap();
        assert_eq!(cell.chunks.len(), 1);
        *cell.chunks.keys().next().unwrap()
    });
    let object = ObjectSlot::new(IsoObject::new(11, ObjectKind::Object, None));
    let object_weak = Rc::downgrade(&object.ref_handle());
    let mut square = IsoGridSquare::new(wx * 8, wy * 8, 0);
    square.objects.push(object);
    let square = SquareSlot::new(square);
    let square_weak = Rc::downgrade(&square.ref_handle());
    with_world(|state| {
        state
            .cell
            .as_mut()
            .unwrap()
            .chunks
            .get_mut(&(wx, wy))
            .unwrap()
            .set_square(0, 0, 0, Some(square));
    });
    // Release construction pins; only canonical membership remains before
    // the real host coordinator selects and retains its current receiver.
    world.with_stream(|_, _, _| ()).unwrap();
    let error = world.activate_streamed_chunks(&mut lua).unwrap_err();
    assert!(error.contains("sprites, regions and native door tables"));
    let (cursor, retained_error) = world.failed_chunk_activation.as_ref().unwrap();
    assert_eq!((cursor.wx, cursor.wy), (wx, wy));
    assert!(!cursor.is_complete());
    assert_eq!(retained_error, &error);
    assert!(!world
        .streamer
        .as_mut()
        .unwrap()
        .take_events()
        .iter()
        .any(|event| {
            matches!(
                event,
                StreamEvent::HandedOff { .. } | StreamEvent::GenAnimalsPath { .. }
            )
        }));
    assert_eq!(
        world.streamer.as_ref().unwrap().pending_world_activations(),
        0
    );
    assert!(!world
        .with_stream(|streamer, cell, ctx| {
            streamer.process_next_pending_load_grid_square(cell, ctx)
        })
        .unwrap());
    assert_eq!(
        with_world(|state| state.cell.as_ref().unwrap().chunks.len()),
        1
    );

    let removed = with_world(|state| state.cell.as_mut().unwrap().chunks.remove(&(wx, wy)));
    drop(removed);
    assert!(
        square_weak.upgrade().is_some(),
        "failed cursor lost its actual square pin"
    );
    assert!(
        object_weak.upgrade().is_some(),
        "failed cursor lost its receiver's object"
    );
    assert_eq!(world.activate_streamed_chunks(&mut lua).unwrap_err(), error);
    assert!(world.streamer.as_mut().unwrap().take_events().is_empty());

    // Explicit abort retires the failure owner outside WORLD/sprite guards.
    // Merely logging/retrying the failure above never loses it or publishes.
    let failed = world.failed_chunk_activation.take();
    drop(failed);
    assert!(square_weak.upgrade().is_none());
    assert!(object_weak.upgrade().is_none());
}
