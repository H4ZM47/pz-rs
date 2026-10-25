//! Final-owner drops must run after the actual WORLD and sprite guards.
use super::*;
use std::cell::Cell;
use std::rc::Weak;

use pz_inventory::{shared, InventoryItem, ItemClass, ItemTemplate};
use pz_world::floor_item::FloorItem;
use pz_world::objects::{IsoObject, IsoWorldInventoryObject, ObjectKind};
use pz_world::streamer::{PlayerPos, Serve, StreamEvent};
use pz_world::{IsoChunk, IsoGridSquare};

struct Fixture {
    world: IsoWorld,
    old_world: Option<pz_lua_api::bind_world::WorldState>,
    old_core: Rc<RefCell<pz_core::core::Core>>,
    dir: PathBuf,
}
impl Fixture {
    fn new(no_save: bool) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pz-stream-retirement-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let old_world = with_world(std::mem::take);
        let old_core = pz_lua_api::bind_core::shared_core();
        let mut core = pz_core::core::Core::new();
        core.set_no_save(no_save);
        pz_lua_api::bind_core::install_core(Rc::new(RefCell::new(core)));
        let mut meta = IsoMetaGrid::empty();
        (meta.max_x, meta.max_y) = (1, 1);
        with_world(|w| {
            w.meta_grid = Some(meta);
            w.cell = Some(IsoCell::default());
        });
        let mut world = IsoWorld::new(&dir, &dir, dir.clone());
        world.sprites = Some(Rc::new(RefCell::new(IsoSpriteManager::new(
            pz_world::sprite::test_tables(),
        ))));
        world.object_tables = Some(ObjectTables::new(vec![1; 8], vec![(0, 0)]).unwrap());
        world.save_tables = Some(
            SaveTables::from_text("0:IsoObject\n27:WorldInventoryItem\nbloodBodyParts=7").unwrap(),
        );
        world.streamer = Some(ChunkStreamer::new(1, 1));
        world
            .with_stream(|s, cell, ctx| s.load_world(10, 10, cell, ctx))
            .unwrap();
        world.streamer.as_mut().unwrap().take_events();
        Self {
            world,
            old_world: Some(old_world),
            old_core,
            dir,
        }
    }
    fn install_bag(&mut self) -> (Weak<RefCell<InventoryItem>>, Rc<Cell<usize>>) {
        let drops = Rc::new(Cell::new(0));
        let bag = shared::new_item(InventoryItem::create(
            Rc::new(ItemTemplate::new(
                ItemClass::Container,
                "Base",
                "AuthoredGuardBag",
            )),
            1,
        ));
        let weak = Rc::downgrade(&bag);
        let probe = Probe {
            drops: drops.clone(),
            sprites: self.world.sprites.as_ref().unwrap().clone(),
        };
        shared::inventory_of(&bag.borrow())
            .unwrap()
            .borrow_mut()
            .parent = Some(Rc::new(probe));
        let mut square = IsoGridSquare::new(80, 80, 0);
        square
            .objects
            .push(pz_world::ObjectSlot::new(IsoObject::new(
                1,
                ObjectKind::WorldInventory(pz_world::floor_object::FloorObject::new(
                    IsoWorldInventoryObject {
                        xoff: 0.0,
                        yoff: 0.0,
                        zoff: 0.0,
                        offset_x: 0.0,
                        offset_y: 0.0,
                        item: FloorItem::retain(&bag),
                        drop_time: 0.0,
                        ignore_remove_sandbox: false,
                        extended_placement: false,
                    },
                )),
                None,
            )));
        square.world_objects.push(1);
        let mut chunk = IsoChunk::new(10, 10);
        chunk.set_square(0, 0, 0, Some(square));
        // Discard construction pins while membership still owns the object, so
        // the probes below rely on the particular removal path being tested.
        drop(chunk.take_retired_objects());
        let old = with_world(|w| w.cell.as_mut().unwrap().chunks.insert((10, 10), chunk));
        drop(old);
        (weak, drops)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let retired = with_world(|w| std::mem::replace(w, self.old_world.take().unwrap()));
        drop(retired);
        pz_lua_api::bind_core::install_core(self.old_core.clone());
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
struct Probe {
    drops: Rc<Cell<usize>>,
    sprites: Rc<RefCell<IsoSpriteManager>>,
}
impl Drop for Probe {
    fn drop(&mut self) {
        with_world(|w| {
            assert!(w.cell.is_some(), "drop reentered wrong world");
        });
        let _sprites = self
            .sprites
            .try_borrow_mut()
            .expect("sprite manager still borrowed at floor owner drop");
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn stop_drops_final_bag_only_after_world_and_sprite_guards_end() {
    let mut f = Fixture::new(true);
    let (weak, drops) = f.install_bag();
    f.world
        .with_stream(|s, cell, ctx| {
            s.stop(cell, ctx);
            assert_eq!(drops.get(), 0);
            assert!(weak.upgrade().is_some());
            assert!(!cell.chunks.contains_key(&(10, 10)));
        })
        .unwrap();
    assert_eq!(drops.get(), 1);
    assert!(weak.upgrade().is_none());
    assert!(matches!(
        f.world.streamer.as_mut().unwrap().take_events().as_slice(),
        [StreamEvent::Removed { wx: 10, wy: 10 }]
    ));
}

#[test]
fn queued_save_all_outcomes_drop_final_bag_outside_both_guards() {
    for mode in ["success", "error", "no_save"] {
        let mut f = Fixture::new(mode == "no_save");
        let (weak, drops) = f.install_bag();
        f.world
            .with_stream(|s, cell, ctx| {
                s.update(
                    &[Some(PlayerPos::moved(80.0, 80.0, 88.0, 80.0))],
                    Serve::Jobs(0),
                    cell,
                    ctx,
                )
            })
            .unwrap();
        assert_eq!(drops.get(), 0, "unload must retain actual pending save");
        if mode == "error" {
            let blocked = f.dir.join("blocked");
            std::fs::write(&blocked, b"file").unwrap();
            f.world.save_dir = blocked;
        }
        f.world.streamer.as_mut().unwrap().take_events();
        f.world
            .with_stream(|s, _cell, ctx| {
                s.save_now(ctx);
                assert_eq!(drops.get(), 0, "{mode}: premature final drop");
                assert!(weak.upgrade().is_some());
            })
            .unwrap();
        assert_eq!(drops.get(), 1, "{mode}");
        assert!(weak.upgrade().is_none());
        let events = f.world.streamer.as_mut().unwrap().take_events();
        match mode {
            "success" => assert!(matches!(
                events.as_slice(),
                [StreamEvent::Saved { written: true, .. }]
            )),
            "no_save" => assert!(matches!(
                events.as_slice(),
                [StreamEvent::Saved { written: false, .. }]
            )),
            _ => assert!(matches!(
                events.as_slice(),
                [StreamEvent::SaveFailed { .. }]
            )),
        }
    }
}

#[test]
fn unwind_keeps_retired_payload_for_next_safe_boundary_and_preserves_user_root() {
    for retain_user_root in [false, true] {
        let mut f = Fixture::new(true);
        let (weak, drops) = f.install_bag();
        let held = retain_user_root.then(|| weak.upgrade().unwrap());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.world.with_stream(|s, cell, ctx| {
                s.stop(cell, ctx);
                panic!("authored frame unwind")
            });
        }));
        assert!(result.is_err());
        assert_eq!(
            drops.get(),
            0,
            "unwind must leave retirement ownership pinned"
        );
        assert!(weak.upgrade().is_some());
        f.world
            .with_stream(|_, _, _| assert_eq!(drops.get(), 0))
            .unwrap();
        if let Some(held) = held {
            assert_eq!(drops.get(), 0, "caller-retained item is still an owner");
            assert!(Rc::ptr_eq(&held, &weak.upgrade().unwrap()));
            drop(held);
        }
        assert_eq!(drops.get(), 1);
        assert!(weak.upgrade().is_none());
    }
}

#[test]
fn whole_object_retirement_releases_parents_only_at_stream_or_frame_boundaries() {
    for stream_boundary in [false, true] {
        for retain_user_root in [false, true] {
            for operation in ["remove", "replace", "clear", "set_square", "drop_levels"] {
                let mut f = Fixture::new(true);
                let (weak_item, drops) = f.install_bag();
                let retained = with_world(|w| {
                    let slot = &w
                        .cell
                        .as_ref()
                        .unwrap()
                        .get_grid_square(80, 80, 0)
                        .unwrap()
                        .objects[0];
                    slot.borrow_mut().key_id = 837;
                    retain_user_root.then(|| slot.ref_handle())
                });
                if stream_boundary {
                    f.world
                        .with_stream(|_, cell, _| {
                            retire_object_path(cell, operation);
                            assert_eq!(drops.get(), 0);
                        })
                        .unwrap();
                } else {
                    // Simulate a nonstream update with the actual sprite owner
                    // borrowed outside WORLD. Generic with_world must not drain.
                    let sprites = f.world.sprites.as_ref().unwrap().clone();
                    {
                        let _sprites = sprites.borrow_mut();
                        with_world(|w| {
                            let cell = w.cell.as_mut().unwrap();
                            retire_object_path(cell, operation);
                        });
                        assert_eq!(drops.get(), 0);
                    }
                    assert_eq!(drops.get(), 0);
                    drain_retired_world_objects();
                }
                if let Some(owner) = retained {
                    assert_eq!(drops.get(), 0);
                    {
                        let object = owner.borrow();
                        assert_eq!(
                            object.key_id, 837,
                            "retirement must not reset retained base fields"
                        );
                        let ObjectKind::WorldInventory(floor) = &object.kind else {
                            panic!("retained object subtype lost")
                        };
                        assert!(Rc::ptr_eq(
                            &floor.borrow().item.item_ref().unwrap(),
                            &weak_item.upgrade().unwrap()
                        ));
                    }
                    drop(owner);
                }
                assert_eq!(drops.get(), 1);
                assert!(weak_item.upgrade().is_none());
            }
        }
    }
}

fn retire_object_path(cell: &mut IsoCell, operation: &str) {
    match operation {
        "remove" => drop(
            cell.get_grid_square_mut(80, 80, 0)
                .unwrap()
                .remove_object_slot(0),
        ),
        "replace" => cell
            .get_grid_square_mut(80, 80, 0)
            .unwrap()
            .replace_object_slot(
                0,
                pz_world::ObjectSlot::new(IsoObject::new(2, ObjectKind::Object, None)),
            ),
        "clear" => cell.get_grid_square_mut(80, 80, 0).unwrap().clear_objects(),
        "set_square" => cell.chunks.get_mut(&(10, 10)).unwrap().set_square(
            0,
            0,
            0,
            Some(IsoGridSquare::new(80, 80, 0)),
        ),
        "drop_levels" => cell
            .chunks
            .get_mut(&(10, 10))
            .unwrap()
            .set_min_max_level(1, 1),
        _ => unreachable!("unknown authored retirement path"),
    }
}
