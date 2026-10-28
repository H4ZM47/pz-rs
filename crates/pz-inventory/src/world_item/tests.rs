use super::*;
use crate::io::{Reader, Writer};
use crate::{ItemClass, ItemEnv, ItemTemplate};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Env;
impl ItemEnv for Env {}

fn value() -> InventoryItem {
    InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "WorldLink")),
        1,
    )
}

fn actual() -> ItemRef {
    crate::component_host::drain_retired_components();
    shared::try_new_item(value()).unwrap()
}

thread_local! {
    static OBSERVATIONS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

fn observe(item: &ItemRef) {
    let busy = usize::from(item.try_borrow_mut().is_err());
    OBSERVATIONS.with(|slot| {
        let (count, borrowed) = slot.get();
        slot.set((count + 1, borrowed + busy));
    });
}

struct ObserverRestore(Option<shared::ItemObserver>);
impl ObserverRestore {
    fn install() -> Self {
        OBSERVATIONS.with(|slot| slot.set((0, 0)));
        Self(shared::install_item_observer(Some(observe)))
    }
}
impl Drop for ObserverRestore {
    fn drop(&mut self) {
        shared::install_item_observer(self.0);
    }
}

#[test]
fn constructor_is_actual_null_and_getter_returns_same_allocation_under_world_body_guard() {
    let item = actual();
    let _observer = ObserverRestore::install();
    assert!(get_world_item_ref(&item).unwrap().is_none());
    assert!(item.borrow().world_item_slot().is_none());

    // A complete stand-in owner is sufficient here: this crate must not know
    // the pz-world type, and the real type validation belongs to that adapter.
    let world = Rc::new(RefCell::new(String::from("complete world allocation")));
    let owner = EntityOwner::from_ref(&world);
    set_world_item_ref(&item, Some(&owner)).unwrap();
    let guard = world.borrow_mut();
    let first = get_world_item_ref(&item).unwrap().unwrap();
    let second = get_world_item_ref(&item).unwrap().unwrap();
    assert!(first.ptr_eq(&owner));
    assert!(second.ptr_eq(&first));
    assert!(Rc::ptr_eq(&first.try_handle::<String>().unwrap(), &world));
    let before = Rc::strong_count(&world);
    {
        let body = item.borrow();
        assert!(body.world_item_slot().unwrap().ptr_eq(&owner));
        assert_eq!(
            Rc::strong_count(&world),
            before,
            "physical tracing is borrowed"
        );
    }
    assert_eq!(OBSERVATIONS.with(Cell::get), (4, 0));
    assert_eq!(&*guard, "complete world allocation");
}

#[test]
fn assignment_is_field_only_nullable_and_same_owner_replacement_preserves_identity() {
    let item = actual();
    let world = Rc::new(RefCell::new(5));
    let owner = EntityOwner::from_ref(&world);
    let table = crate::moddata::get_mod_data_ref(&item).unwrap();
    item.borrow_mut().id = 72;
    item.borrow_mut().favorite = true;
    let container = shared::container_of(&item);

    set_world_item_ref(&item, Some(&owner)).unwrap();
    set_world_item_ref(&item, Some(&owner)).unwrap();
    assert!(get_world_item_ref(&item).unwrap().unwrap().ptr_eq(&owner));
    set_world_item_ref(&item, None).unwrap();
    assert!(get_world_item_ref(&item).unwrap().is_none());
    assert_eq!(item.borrow().id, 72);
    assert!(item.borrow().favorite);
    assert_eq!(*world.borrow(), 5);
    assert!(crate::moddata::get_mod_data_ref(&item)
        .unwrap()
        .ptr_eq(&table));
    assert_eq!(shared::container_of(&item).is_some(), container.is_some());
    assert!(item.borrow().components.is_empty());
    crate::component_host::drain_retired_components();
}

#[test]
fn held_item_is_explicit_borrow_failure_and_observer_runs_on_both_results() {
    let item = actual();
    let _observer = ObserverRestore::install();
    let world = Rc::new(RefCell::new(1));
    let owner = EntityOwner::from_ref(&world);
    {
        let guard = item.borrow_mut();
        assert!(matches!(
            get_world_item_ref(&item),
            Err(RuntimeHostError::Borrowed("inventory world-item getter"))
        ));
        assert!(matches!(
            set_world_item_ref(&item, Some(&owner)),
            Err(RuntimeHostError::Borrowed("inventory world-item setter"))
        ));
        assert!(guard.world_item_slot().is_none());
    }
    set_world_item_ref(&item, Some(&owner)).unwrap();
    assert!(get_world_item_ref(&item).unwrap().unwrap().ptr_eq(&owner));
    // Only the caller's two deliberately held guards can obstruct metadata.
    assert_eq!(OBSERVATIONS.with(Cell::get), (4, 2));
    crate::component_host::drain_retired_components();
}

struct DropProbe {
    foreign: Rc<RefCell<()>>,
    drops: Rc<Cell<usize>>,
}
impl Drop for DropProbe {
    fn drop(&mut self) {
        let _guard = self
            .foreign
            .try_borrow_mut()
            .expect("world owner final drop must occur outside outer foreign guard");
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn replaced_owner_and_rejected_incoming_owner_retire_after_outer_guard() {
    let item = actual();
    let foreign = Rc::new(RefCell::new(()));
    let drops = Rc::new(Cell::new(0));
    let world = Rc::new(RefCell::new(DropProbe {
        foreign: foreign.clone(),
        drops: drops.clone(),
    }));
    let owner = EntityOwner::from_ref(&world);
    set_world_item_ref(&item, Some(&owner)).unwrap();
    drop(owner);
    drop(world);
    crate::component_host::drain_retired_components();
    {
        let _foreign_guard = foreign.borrow_mut();
        set_world_item_ref(&item, None).unwrap();
        assert_eq!(drops.get(), 0);
    }
    crate::component_host::drain_retired_components();
    assert_eq!(drops.get(), 1);

    let incoming = Rc::new(RefCell::new(DropProbe {
        foreign: foreign.clone(),
        drops: drops.clone(),
    }));
    let owner = EntityOwner::from_ref(&incoming);
    {
        let _foreign_guard = foreign.borrow_mut();
        let guard = item.borrow_mut();
        assert!(set_world_item_ref(&item, Some(&owner)).is_err());
        drop(owner);
        drop(incoming);
        assert_eq!(drops.get(), 1);
        assert!(guard.world_item_slot().is_none());
    }
    crate::component_host::drain_retired_components();
    assert_eq!(drops.get(), 2);
}

#[test]
fn low_level_replacement_and_gc_take_move_physical_pin_without_world_mutation() {
    let mut item = value();
    let world = Rc::new(RefCell::new(19));
    let owner = EntityOwner::from_ref(&world);
    let mut retired = RetiredComponents::new();
    item.replace_world_item(Some(&owner), &mut retired);
    assert!(retired.is_empty());
    let guard = world.borrow_mut();
    item.replace_world_item(Some(&owner), &mut retired);
    assert_eq!(retired.len(), 1);
    let taken = item.take_world_item_for_gc().unwrap();
    assert!(taken.ptr_eq(&owner));
    assert!(item.world_item_slot().is_none());
    assert!(item.take_world_item_for_gc().is_none());
    assert_eq!(*guard, 19);
    retired.push_owner(taken);
    drop(guard);
    drop(retired);
}

#[test]
fn rust_archive_copy_and_equality_exclude_source_world_owner() {
    let mut item = value();
    let world = Rc::new(RefCell::new(23));
    let owner = EntityOwner::from_ref(&world);
    let mut retired = RetiredComponents::new();
    item.replace_world_item(Some(&owner), &mut retired);
    let before = Rc::strong_count(&world);
    let _guard = world.borrow_mut();
    let copied = item.try_archive_copy().unwrap();
    assert!(copied.world_item_slot().is_none());
    assert!(item.world_item_slot().unwrap().ptr_eq(&owner));
    assert_eq!(
        item, copied,
        "Rust archive equality excludes runtime backlink"
    );
    let legacy = item.clone();
    assert!(legacy.world_item_slot().is_none());
    assert_eq!(Rc::strong_count(&world), before);
}

#[test]
fn saved_and_shared_container_archives_clear_each_nested_items_world_slot() {
    let item = actual();
    let world = Rc::new(RefCell::new(29));
    let owner = EntityOwner::from_ref(&world);
    set_world_item_ref(&item, Some(&owner)).unwrap();
    let container = crate::SharedContainer::default().into_ref();
    shared::add_item(&container, &item);
    let _world_guard = world.borrow_mut();

    let snapshot = container.borrow().try_to_container().unwrap();
    let child = snapshot.items[0].as_item().unwrap();
    assert!(child.world_item_slot().is_none());
    let copied_snapshot = snapshot.items[0].try_archive_copy().unwrap();
    assert!(copied_snapshot
        .as_item()
        .unwrap()
        .world_item_slot()
        .is_none());

    let copied_container = container.borrow().try_deep_clone().unwrap();
    let copied_child = copied_container.borrow().items[0].clone();
    assert!(!Rc::ptr_eq(&copied_child, &item));
    assert!(copied_child.borrow().world_item_slot().is_none());
    assert!(item.borrow().world_item_slot().unwrap().ptr_eq(&owner));
}

#[test]
fn native_item_save_omits_link_and_load_preserves_it_on_success_and_error() {
    let mut item = value();
    let mut expected = Writer::new();
    item.save(&mut expected, &Env).unwrap();
    let world = Rc::new(RefCell::new(31));
    let owner = EntityOwner::from_ref(&world);
    let mut retired = RetiredComponents::new();
    item.replace_world_item(Some(&owner), &mut retired);
    let mut written = Writer::new();
    item.save(&mut written, &Env).unwrap();
    assert_eq!(written.buf, expected.buf);

    let _guard = world.borrow_mut();
    // load starts after the registry id and save type, on this same receiver.
    let mut reader = Reader::new(&written.buf[3..]);
    item.load(&mut reader, 249, &Env).unwrap();
    assert_eq!(reader.remaining(), 0);
    assert!(item.world_item_slot().unwrap().ptr_eq(&owner));

    // Complete id and header, then truncate while reading optional uses.
    // This executes the native scalar-reset prefix before the failure.
    let malformed = [0, 0, 0, 42, 1];
    let mut reader = Reader::new(&malformed);
    assert!(item.load(&mut reader, 249, &Env).is_err());
    assert_eq!(reader.pos, 5);
    assert_eq!(item.id, 42);
    assert!(item.world_item_slot().unwrap().ptr_eq(&owner));
}
