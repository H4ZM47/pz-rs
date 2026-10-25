//! Real floor-item owners must outlive the streamer's caller borrows.
use super::*;
use std::cell::Cell;
use std::rc::{Rc, Weak};

use crate::floor_item::FloorItem;
use crate::objects::{IsoObject, IsoWorldInventoryObject, ObjectKind};
use pz_inventory::{shared, InventoryItem, ItemClass, ItemTemplate};

struct DropProbe(Rc<Cell<usize>>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

fn bag_chunk(
    wx: i32,
    wy: i32,
    drops: Rc<Cell<usize>>,
) -> (IsoChunk, Weak<std::cell::RefCell<InventoryItem>>) {
    let bag = shared::new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(
            ItemClass::Container,
            "Base",
            "AuthoredRetirementBag",
        )),
        1,
    ));
    shared::inventory_of(&bag.borrow())
        .unwrap()
        .borrow_mut()
        .parent = Some(Rc::new(DropProbe(drops)));
    let weak = Rc::downgrade(&bag);
    let mut square = IsoGridSquare::new(wx * 8, wy * 8, 0);
    square.objects.push(
        (IsoObject::new(
            1,
            ObjectKind::WorldInventory(crate::floor_object::FloorObject::new(
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
        ))
        .into(),
    );
    square.world_objects.push(1);
    let mut chunk = IsoChunk::new(wx, wy);
    chunk.set_square(0, 0, 0, Some((square).into()));
    (chunk, weak)
}

#[test]
fn queued_save_success_error_and_no_save_keep_actual_bag_until_retirement_drain() {
    for mode in ["success", "error", "no_save"] {
        let mut f = Fixture::new();
        f.no_save = mode == "no_save";
        let blocked = f.dir.join("not-a-directory");
        if mode == "error" {
            std::fs::write(&blocked, b"block").unwrap();
        }
        let mut s = ChunkStreamer::new(1, 1);
        let drops = Rc::new(Cell::new(0));
        let (chunk, weak) = bag_chunk(10, 10, drops.clone());
        s.save_queue.push_back(QueuedSave {
            wx: 10,
            wy: 10,
            chunk,
        });
        {
            let mut ctx = f.ctx();
            if mode == "error" {
                ctx.save_dir = &blocked;
            }
            assert!(s.save_worker_update(None, &mut ctx));
        }
        let events = s.take_events();
        assert_eq!(events.len(), 2, "{mode}: {events:?}");
        match mode {
            "success" => assert!(matches!(
                events[0],
                StreamEvent::Saved { written: true, .. }
            )),
            "no_save" => assert!(matches!(
                events[0],
                StreamEvent::Saved { written: false, .. }
            )),
            _ => assert!(matches!(events[0], StreamEvent::SaveFailed { .. })),
        }
        assert!(matches!(events[1], StreamEvent::HotSave));
        assert_eq!(drops.get(), 0, "{mode}: payload died inside save");
        assert!(weak.upgrade().is_some());
        let retired = s.take_retired_chunks();
        assert_eq!(retired.len(), 1);
        assert!(s.take_retired_chunks().is_empty());
        assert_eq!(drops.get(), 0, "taking must transfer, not clone/drop");
        drop(retired);
        assert_eq!(drops.get(), 1);
        assert!(weak.upgrade().is_none());
    }
}

#[test]
fn user_retained_item_keeps_exact_owner_after_chunk_retirement() {
    let mut f = Fixture::new();
    f.no_save = true;
    let mut s = ChunkStreamer::new(1, 1);
    let drops = Rc::new(Cell::new(0));
    let (chunk, weak) = bag_chunk(10, 10, drops.clone());
    let held = weak.upgrade().unwrap();
    s.save_queue.push_back(QueuedSave {
        wx: 10,
        wy: 10,
        chunk,
    });
    s.save_now(&mut f.ctx());
    drop(s.take_retired_chunks());
    assert_eq!(drops.get(), 0);
    assert!(Rc::ptr_eq(&held, &weak.upgrade().unwrap()));
    drop(held);
    assert_eq!(drops.get(), 1);
}

#[test]
fn rejected_handoff_and_replaced_cell_chunk_are_retired_without_losing_new_chunk() {
    let mut f = Fixture::new();
    let mut cell = IsoCell::default();
    let mut s = ChunkStreamer::new(1, 1);
    let drops = Rc::new(Cell::new(0));
    let id = s.new_chunk(10, 10);
    let (chunk, weak) = bag_chunk(10, 10, drops.clone());
    assert!(!s.hand_off(id, chunk, &mut cell, &mut f.ctx()));
    assert_eq!(short(&s.take_events()), ["D 10,10"]);
    assert!(weak.upgrade().is_some());
    drop(s.take_retired_chunks());
    assert_eq!(drops.get(), 1);

    let (old, weak) = bag_chunk(10, 10, drops.clone());
    cell.chunks.insert((10, 10), old);
    s.set_initial_pos(0, 10, 10);
    let id = s.new_chunk(10, 10);
    s.records.get_mut(&id).unwrap().refs.push(0);
    let mut new = IsoChunk::new(10, 10);
    new.saved.loot_respawn_hour = 43;
    assert!(s.hand_off(id, new, &mut cell, &mut f.ctx()));
    assert_eq!(cell.chunks[&(10, 10)].saved.loot_respawn_hour, 43);
    assert!(matches!(
        s.take_events()[0],
        StreamEvent::HandedOff { wx: 10, wy: 10 }
    ));
    assert!(weak.upgrade().is_some());
    assert_eq!(drops.get(), 1);
    drop(s.take_retired_chunks());
    assert_eq!(drops.get(), 2);
}

#[test]
fn stop_retires_loaded_chunk_but_preserves_pending_served_ownership() {
    let mut f = Fixture::new();
    let mut cell = IsoCell::default();
    let mut s = ChunkStreamer::new(1, 1);
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    drop(s.take_retired_chunks());
    s.take_events();
    f.log.0.clear();
    let drops = Rc::new(Cell::new(0));
    let (loaded, loaded_weak) = bag_chunk(10, 10, drops.clone());
    cell.chunks.insert((10, 10), loaded);
    let pending_id = s.new_chunk(11, 10);
    let (pending, pending_weak) = bag_chunk(11, 10, drops.clone());
    s.load_grid_square.push_back((pending_id, pending));
    s.stop(&mut cell, &mut f.ctx());
    assert_eq!(short(&s.take_events()), ["X 10,10"]);
    assert_eq!(f.log.0, ["unloaded 10,10"]);
    assert!(!cell.chunks.contains_key(&(10, 10)));
    assert!(loaded_weak.upgrade().is_some());
    assert!(pending_weak.upgrade().is_some());
    drop(s.take_retired_chunks());
    assert_eq!(drops.get(), 1);
    assert!(loaded_weak.upgrade().is_none());
    assert!(
        pending_weak.upgrade().is_some(),
        "stop must not clear pending handoffs"
    );
    drop(s);
    assert_eq!(drops.get(), 2);
}

struct PanicUnload;
impl ChunkLoadHook for PanicUnload {
    fn chunk_loaded(&mut self, _: &mut IsoChunk, _: &mut HookCtx<'_>) {}
    fn load_grid_square(&mut self, _: &mut IsoChunk, _: i32, _: i32, _: i32, _: &mut HookCtx<'_>) {}
    fn chunk_unloaded(&mut self, _: i32, _: i32) {
        panic!("authored unload failure")
    }
}

#[test]
fn unload_hook_unwind_retires_owner_without_publishing_partial_save() {
    for stop in [false, true] {
        let mut f = Fixture::new();
        let mut cell = IsoCell::default();
        let mut s = ChunkStreamer::new(1, 1);
        s.load_world(10, 10, &mut cell, &mut f.ctx());
        drop(s.take_retired_chunks());
        s.take_events();
        let drops = Rc::new(Cell::new(0));
        let (chunk, weak) = bag_chunk(10, 10, drops.clone());
        cell.chunks.insert((10, 10), chunk);
        let id = s.get_chunk(0, 0, 0).unwrap();
        let mut hook = PanicUnload;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut ctx = f.ctx();
            ctx.chunk_hook = Some(&mut hook);
            if stop {
                s.stop(&mut cell, &mut ctx);
            } else {
                s.remove_from_world(id, &mut cell, &mut ctx);
            }
        }));
        assert!(result.is_err());
        assert_eq!(drops.get(), 0);
        assert!(weak.upgrade().is_some());
        assert!(
            s.save_queue.is_empty(),
            "failed unload must not become a pending save"
        );
        s.take_events();
        s.save_now(&mut f.ctx());
        assert!(
            s.take_events().is_empty(),
            "failed unload must never be saved"
        );
        drop(s.take_retired_chunks());
        assert_eq!(drops.get(), 1);
        assert!(weak.upgrade().is_none());
    }
}

struct GenerateExtra {
    drops: Rc<Cell<usize>>,
    weak: Option<Weak<std::cell::RefCell<InventoryItem>>>,
}
impl BrandNewHook for GenerateExtra {
    fn on_new_chunk_loaded(&mut self, _: &mut IsoChunk, _: &IsoMetaGrid, _: &mut ChunkLoadEnv<'_>) {
    }
    fn generate_chunks(
        &mut self,
        chunks: &mut ChunksCache,
        _: &IsoMetaGrid,
        _: &mut ChunkLoadEnv<'_>,
    ) {
        let (wx, wy) = chunks
            .coords()
            .iter()
            .copied()
            .find(|&c| c != (10, 10))
            .unwrap();
        let (chunk, weak) = bag_chunk(wx, wy, self.drops.clone());
        *chunks.get_mut(wx, wy).unwrap() = chunk;
        self.weak = Some(weak);
    }
}

#[test]
fn brand_new_extra_chunks_are_retained_until_caller_drains_them() {
    let mut f = Fixture::new();
    let mut s = ChunkStreamer::new(1, 1);
    let drops = Rc::new(Cell::new(0));
    let mut gen = GenerateExtra {
        drops: drops.clone(),
        weak: None,
    };
    let id = s.new_chunk(10, 10);
    {
        let mut ctx = f.ctx();
        ctx.brand_new = Some(&mut gen);
        s.do_chunk(id, &mut ctx);
    }
    assert_eq!(drops.get(), 0);
    assert!(gen.weak.as_ref().unwrap().upgrade().is_some());
    assert_eq!(s.load_grid_square.len(), 1);
    let retired = s.take_retired_chunks();
    assert_eq!(retired.len(), 3);
    drop(retired);
    assert_eq!(drops.get(), 1);
}

#[test]
fn retained_bag_mutation_after_unload_is_written_by_real_pending_save() {
    struct BagEnv(Rc<ItemTemplate>);
    impl pz_inventory::env::ItemEnv for BagEnv {
        fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
            (id == 1).then(|| self.0.clone())
        }
    }
    let mut f = Fixture::new();
    let mut s = ChunkStreamer::new(1, 1);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    drop(s.take_retired_chunks());
    s.take_events();
    let drops = Rc::new(Cell::new(0));
    let (chunk, weak) = bag_chunk(10, 10, drops.clone());
    cell.chunks.insert((10, 10), chunk);
    let held = weak.upgrade().unwrap();
    let id = s.get_chunk(0, 0, 0).unwrap();
    s.remove_from_world(id, &mut cell, &mut f.ctx());
    assert_eq!(s.save_queue.len(), 1);
    assert!(
        s.retired_chunks.is_empty(),
        "successful unload transfers pin to pending save"
    );
    held.borrow_mut().favorite = true;
    held.borrow_mut().condition = 2;
    s.save_now(&mut f.ctx());
    let env = BagEnv(Rc::new(ItemTemplate::new(
        ItemClass::Container,
        "Base",
        "AuthoredRetirementBag",
    )));
    let bytes = std::fs::read(chunk_path(&f.dir, 10, 10)).unwrap();
    let mut ctx = f.save_ctx();
    ctx.items = &env;
    let reloaded = load_chunk(&bytes, 10, 10, &ctx).unwrap().chunk;
    let object = reloaded.get_grid_square(0, 0, 0).unwrap().borrow().objects[0].ref_handle();
    let ObjectKind::WorldInventory(floor) = &object.borrow().kind else {
        panic!("saved floor item missing")
    };
    let floor = floor.borrow();
    let saved_bag = floor.item.borrow_item().unwrap();
    assert!(saved_bag.favorite, "queue wrote a stale item snapshot");
    assert_eq!(saved_bag.condition, 2);
    drop(s.take_retired_chunks());
    assert_eq!(drops.get(), 0);
    assert!(Rc::ptr_eq(&held, &weak.upgrade().unwrap()));
    drop(held);
    assert_eq!(drops.get(), 1);
}
