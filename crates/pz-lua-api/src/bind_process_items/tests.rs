use super::*;
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use pz_kahlua::{TableRef, Thread};

fn item(id: i32) -> ItemRef {
    let mut i = InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Rock")),
        1,
    );
    i.id = id;
    pz_inventory::shared::new_item(i)
}

fn thread() -> Thread {
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    crate::bind_world::register(&mut t);
    run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end");
    t
}

fn run(t: &mut Thread, src: &str) -> Vec<Value> {
    let f = pz_kahlua::stdlib::load_string(src, Some("process-items-test"), t.globals().clone())
        .unwrap();
    t.call(&f, &[]).unwrap()
}

#[test]
fn process_queue_live_identity_mutation_and_retained_old_cell() {
    crate::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    let mut t = thread();
    let a = item(11);
    let b = item(11);
    t.globals().set_str("a", item_value(&a));
    t.globals().set_str("b", item_value(&b));
    run(
        &mut t,
        r#"
        local c = IsoCell.getInstance()
        items, pending = c:getProcessItems(), c:getProcessItemsRemove()
        assert(items == c:getProcessItems() and pending == c:getProcessItemsRemove())
        c:addToProcessItems(a) c:addToProcessItems(a) c:addToProcessItems(b)
        assert(items:size() == 2 and items:get(0) == a and items:get(1) == b)
        c:addToProcessItemsRemove(a) c:addToProcessItemsRemove(a)
        assert(pending:size() == 1 and pending:contains(a) and items:size() == 2)
        c:addToProcessItems(a)
        assert(pending:isEmpty() and items:size() == 2)
        local bulk = ArrayList.new() bulk:add(a) bulk:add(b) bulk:add(nil)
        c:addToProcessItemsRemove(bulk)
        assert(pending:size() == 2)
        pending:remove(a)
        items:add(a) items:add(nil) pending:add(nil)
        assert(items:size() == 4 and pending:size() == 2)
    "#,
    );
    let removed =
        crate::bind_world::with_world(|w| w.cell.as_ref().unwrap().process_items.drain_remove());
    drop(removed);
    run(&mut t, "assert(items:size() == 2 and items:get(0) == a and items:get(1) == a and pending:isEmpty())");
    crate::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    run(
        &mut t,
        r#"
        local fresh = IsoCell.getInstance():getProcessItems()
        assert(fresh ~= items and fresh:isEmpty() and items:size() == 2)
        items:clear() items:add(b)
        assert(items:get(0) == b and fresh:isEmpty())
    "#,
    );
    crate::bind_world::set_cell(None);
}

#[test]
fn process_container_enter_and_no_cell_failure() {
    let c = Rc::new(RefCell::new(pz_inventory::shared::SharedContainer::new(
        -1,
        "inventory",
    )));
    let a = item(3);
    c.borrow_mut().items.push(a.clone());
    crate::bind_world::set_cell(None);
    assert!(add_container_items(&c).is_err());
    crate::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    add_container_items(&c).unwrap();
    add_container_items(&c).unwrap();
    crate::bind_world::with_world(|w| {
        let q = &w.cell.as_ref().unwrap().process_items;
        assert_eq!(q.items.borrow().len(), 1);
        assert!(Rc::ptr_eq(q.items.borrow()[0].as_ref().unwrap(), &a));
    });
    crate::bind_world::set_cell(None);
}

#[test]
fn process_bulk_cast_failure_keeps_prefix_and_client_gate_precedes_cast() {
    crate::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    let mut t = thread();
    let a = item(7);
    let b = item(7);
    t.globals().set_str("a", item_value(&a));
    t.globals().set_str("b", item_value(&b));
    let f = pz_kahlua::stdlib::load_string(
        "bad = ArrayList.new() bad:add(a) bad:add('wrong class') bad:add(b) IsoCell.getInstance():addToProcessItems(bad)",
        Some("partial-cast"), t.globals().clone(),
    ).unwrap();
    let error = t.call(&f, &[]).unwrap_err();
    assert!(matches!(
        error,
        LuaError::Runtime {
            class: "java.lang.ClassCastException",
            ..
        }
    ));
    crate::bind_world::with_world(|w| {
        let q = &w.cell.as_ref().unwrap().process_items;
        assert_eq!(q.items.borrow().len(), 1);
        assert!(Rc::ptr_eq(q.items.borrow()[0].as_ref().unwrap(), &a));
    });
    let core = crate::bind_core::shared_core();
    let old_client = core.borrow().is_client;
    core.borrow_mut().is_client = true;
    run(&mut t, "IsoCell.getInstance():addToProcessItems(bad)");
    let c = Rc::new(RefCell::new(pz_inventory::shared::SharedContainer::new(
        -1,
        "inventory",
    )));
    c.borrow_mut().items.push(b);
    add_container_items(&c).unwrap();
    core.borrow_mut().is_client = old_client;
    crate::bind_world::with_world(|w| {
        assert_eq!(
            w.cell.as_ref().unwrap().process_items.items.borrow().len(),
            1
        )
    });
    let f = pz_kahlua::stdlib::load_string(
        "IsoCell.getInstance():addToProcessItemsRemove(bad)",
        Some("remove-partial-cast"),
        t.globals().clone(),
    )
    .unwrap();
    assert!(matches!(
        t.call(&f, &[]).unwrap_err(),
        LuaError::Runtime {
            class: "java.lang.ClassCastException",
            ..
        }
    ));
    crate::bind_world::with_world(|w| {
        assert_eq!(
            w.cell.as_ref().unwrap().process_items.remove.borrow().len(),
            1
        )
    });
    crate::bind_world::set_cell(None);
}

#[test]
fn process_container_floor_skips_sound_removal_but_nonfloor_retires() {
    crate::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    let old_core = crate::bind_core::shared_core();
    crate::bind_core::install_core(Rc::new(RefCell::new(pz_core::core::Core::default())));
    crate::item_sound::reset();
    let mut t = thread();
    crate::bind_item_container::register(&mut t);
    let a = item(4);
    let container = Rc::new(RefCell::new(pz_inventory::shared::SharedContainer::new(
        1, "floor",
    )));
    container.borrow_mut().items.push(a.clone());
    t.globals().set_str(
        "container",
        crate::bind_item_container::container_value(&container),
    );
    crate::item_sound::add_item(Some(a.clone()));
    let manager = crate::item_sound::shared_manager();
    let mut sound = pz_sound::system::SoundSystem::silent();
    crate::item_sound::update_manager(&manager, &mut sound, 17, false);
    let emitter = manager.borrow().emitter_for(&a).unwrap();
    run(
        &mut t,
        "container:addItemsToProcessItems() container:removeItemsFromProcessItems()",
    );
    crate::bind_world::with_world(|w| {
        let q = &w.cell.as_ref().unwrap().process_items;
        assert_eq!(q.items.borrow().len(), 1);
        assert_eq!(q.remove.borrow().len(), 1);
    });
    crate::item_sound::update_manager(&manager, &mut sound, 17, false);
    assert!(
        Rc::ptr_eq(&emitter, &manager.borrow().emitter_for(&a).unwrap()),
        "floor proxy leaves the world item's sound alone"
    );
    container.borrow_mut().kind = "inventory".into();
    run(&mut t, "container:removeItemsFromProcessItems()");
    crate::item_sound::update_manager(&manager, &mut sound, 17, false);
    assert!(manager.borrow().emitter_for(&a).is_none());
    assert_eq!(
        sound.world_emitters.free_count(),
        0,
        "sound stop is delayed until the next phase"
    );
    crate::item_sound::update_manager(&manager, &mut sound, 17, false);
    assert_eq!(sound.world_emitters.free_count(), 1);
    crate::item_sound::reset();
    crate::bind_world::set_cell(None);
    crate::bind_core::install_core(old_core);
}

#[test]
fn process_cell_replacement_drops_final_item_owners_outside_world_borrow() {
    struct ReenterWorld(Rc<std::cell::Cell<bool>>);
    impl Drop for ReenterWorld {
        fn drop(&mut self) {
            crate::bind_world::with_world(|w| {
                assert!(w.cell.is_some(), "replacement already installed")
            });
            self.0.set(true);
        }
    }
    crate::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    let dropped = Rc::new(std::cell::Cell::new(false));
    let bag = pz_inventory::shared::new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Container, "Base", "Bag")),
        1,
    ));
    let weak = Rc::downgrade(&bag);
    pz_inventory::shared::inventory_of(&bag.borrow())
        .unwrap()
        .borrow_mut()
        .parent = Some(Rc::new(ReenterWorld(dropped.clone())));
    crate::bind_world::with_world(|w| {
        let q = &w.cell.as_ref().unwrap().process_items;
        q.add(Some(bag.clone()), false);
        q.queue_remove(Some(bag.clone()));
    });
    drop(bag);
    crate::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    assert!(weak.upgrade().is_none());
    assert!(dropped.get());
    crate::bind_world::set_cell(None);
}

#[test]
fn process_live_list_and_set_do_not_cache_removed_item_owners() {
    for set in [false, true] {
        let queue = pz_world::process_items::ProcessItemQueues::default();
        let item = item(99);
        let weak = Rc::downgrade(&item);
        if set {
            queue.queue_remove(Some(item.clone()));
        } else {
            queue.add(Some(item.clone()), false);
        }
        let backing = if set { &queue.remove } else { &queue.items };
        let mut t = thread();
        t.globals().set_str("retained", queue_value(backing, set));
        run(&mut t, "assert(retained:size() == 1)");
        queue.queue_remove(Some(item.clone()));
        drop(item);
        drop(queue.drain_remove());
        // Do not refresh or release the retained view before checking.
        assert!(
            weak.upgrade().is_none(),
            "retained live view kept a stale item snapshot (set={set})"
        );
        assert!(!t.globals().get_str("retained").is_nil());
    }
}

#[test]
fn process_failed_live_mutation_releases_snapshot_but_returned_item_stays_live() {
    let queue = pz_world::process_items::ProcessItemQueues::default();
    let item = item(99);
    let weak = Rc::downgrade(&item);
    queue.add(Some(item.clone()), false);
    let mut t = thread();
    t.globals()
        .set_str("retained", queue_value(&queue.items, false));
    run(
        &mut t,
        "held = retained:get(0) assert(not pcall(function() retained:add('wrong class') end))",
    );
    queue.queue_remove(Some(item.clone()));
    drop(item);
    drop(queue.drain_remove());
    assert!(
        weak.upgrade().is_some(),
        "returned item is an external owning reference"
    );
    t.globals().set_str("held", Value::Nil);
    assert!(
        weak.upgrade().is_none(),
        "failed write retained mirrored item values"
    );
}
