use super::*;
use crate::{IsoObject, ObjectKind, ObjectSlot};
use pz_kahlua::gc::{
    collect_tables, register_userdata, CollectionResult, TraceDeferred, UserdataDescriptor,
};
use pz_kahlua::{LuaObject, TableRef};
use std::any::Any;
use std::cell::Cell;
use std::rc::Weak;

fn object(id: u32) -> IsoObjectRef {
    ObjectSlot::new(IsoObject::new(id, ObjectKind::Object, None)).ref_handle()
}

#[test]
fn physical_membership_keeps_duplicates_detached_owners_and_same_id_distinctions() {
    let list = WorldObjects::new_ref();
    let a = object(7);
    let equal = object(7);
    let weak = Rc::downgrade(&a);
    let baseline = Rc::strong_count(&a);
    list.borrow_mut().push_object(&a);
    list.borrow_mut()
        .push_entry(WorldObjectEntry::Foreign(Value::Nil));
    list.borrow_mut().push_object(&a);
    assert_eq!(Rc::strong_count(&a), baseline + 2);
    assert_eq!(register_world_objects(&list), Ok(true));
    assert_eq!(register_world_objects(&list), Ok(false));
    assert_eq!(Rc::strong_count(&a), baseline + 2);
    assert!(!list.borrow().contains_object(&equal));
    let removed = list.borrow_mut().remove_first_object(&a).unwrap();
    assert!(list.borrow().contains_object(&a));
    assert_eq!(list.borrow().len(), 2);
    assert!(matches!(
        list.borrow().entry(0),
        Some(WorldObjectEntry::Foreign(Value::Nil))
    ));
    drop(removed);
    drop(a);
    assert!(weak.upgrade().is_some());
    drop(list);
    assert!(weak.upgrade().is_none());
}

struct DropProbe {
    list: Weak<RefCell<WorldObjects>>,
    host: Rc<RefCell<()>>,
    sprites: Rc<RefCell<()>>,
    drops: Rc<Cell<usize>>,
}
impl LuaObject for DropProbe {
    fn class_name(&self) -> &str {
        panic!("opaque methods must not run")
    }
    fn as_any(&self) -> &dyn Any {
        panic!("opaque methods must not run")
    }
}
impl Drop for DropProbe {
    fn drop(&mut self) {
        let list = self
            .list
            .upgrade()
            .expect("list pinned across deferred drop");
        let _list = list
            .try_borrow_mut()
            .expect("entry dropped under list guard");
        let _host = self
            .host
            .try_borrow_mut()
            .expect("entry dropped under outer guard");
        let _sprites = self
            .sprites
            .try_borrow_mut()
            .expect("entry dropped under sprite guard");
        self.drops.set(self.drops.get() + 1);
    }
}
fn foreign(
    list: &WorldObjectsRef,
    host: &Rc<RefCell<()>>,
    sprites: &Rc<RefCell<()>>,
    drops: &Rc<Cell<usize>>,
) -> WorldObjectEntry {
    WorldObjectEntry::Foreign(Value::Object(Rc::new(DropProbe {
        list: Rc::downgrade(list),
        host: host.clone(),
        sprites: sprites.clone(),
        drops: drops.clone(),
    })))
}

#[test]
fn every_removal_and_rejected_input_moves_final_owners_outside_enclosing_guards() {
    let list = WorldObjects::new_ref();
    let host = Rc::new(RefCell::new(()));
    let sprites = Rc::new(RefCell::new(()));
    let drops = Rc::new(Cell::new(0));
    for operation in 0..5 {
        let host_guard = host.borrow_mut();
        let sprite_guard = sprites.borrow_mut();
        let mut guard = list.borrow_mut();
        let entry = foreign(&list, &host, &sprites, &drops);
        let old = match operation {
            0 => {
                guard.push_entry(entry);
                guard.remove_entry(0).unwrap()
            }
            1 => {
                guard.push_entry(entry);
                guard
                    .replace_entry(0, WorldObjectEntry::Foreign(Value::Nil))
                    .unwrap()
            }
            2 => guard.insert_entry(999, entry).unwrap_err().1,
            3 => guard.replace_entry(999, entry).unwrap_err().1,
            _ => {
                guard.push_entry(entry);
                guard.take_entries().pop().unwrap()
            }
        };
        assert_eq!(drops.get(), operation);
        drop(guard);
        drop(host_guard);
        drop(sprite_guard);
        drop(old);
        assert_eq!(drops.get(), operation + 1);
        let rest = list.borrow_mut().take_entries();
        drop(rest);
    }
}

#[test]
fn structural_version_tracks_only_structural_changes_and_empty_clear() {
    let mut list = WorldObjects::default();
    let a = object(1);
    list.push_object(&a);
    let version = list.structural_version();
    drop(
        list.replace_entry(0, WorldObjectEntry::Foreign(Value::Nil))
            .unwrap(),
    );
    assert_eq!(list.structural_version(), version);
    assert!(list.remove_first_object(&a).is_none());
    assert!(list.remove_entry(99).is_none());
    drop(
        list.insert_entry(99, WorldObjectEntry::Foreign(Value::Nil))
            .unwrap_err(),
    );
    assert_eq!(list.structural_version(), version);
    drop(list.take_entries());
    drop(list.take_entries());
    assert_eq!(list.structural_version(), version.wrapping_add(2));
    list.structural_version = u32::MAX;
    list.push_object(&a);
    assert_eq!(list.structural_version(), 0);
}

struct Link {
    list: WorldObjectsRef,
}
impl LuaObject for Link {
    fn class_name(&self) -> &str {
        panic!("collector must not call userdata methods")
    }
    fn as_any(&self) -> &dyn Any {
        panic!("collector must not call userdata methods")
    }
}
struct LinkDescriptor;
impl UserdataDescriptor<Link> for LinkDescriptor {
    fn trace(payload: &Link, visitor: &mut TraceVisitor) -> Result<(), TraceDeferred> {
        visitor.owner(&payload.list);
        Ok(())
    }
}

#[test]
fn physical_list_descriptor_preserves_live_and_borrowed_graph_then_collects_actual_cycle() {
    let list = WorldObjects::new_ref();
    assert_eq!(register_world_objects(&list), Ok(true));
    let table = TableRef::new();
    let link = Rc::new(Link { list: list.clone() });
    assert_eq!(register_userdata::<_, LinkDescriptor>(&link), Ok(true));
    table.set_str("link", Value::Object(link.clone()));
    list.borrow_mut()
        .push_entry(WorldObjectEntry::Foreign(Value::Table(table.clone())));
    list.borrow_mut()
        .push_entry(WorldObjectEntry::Foreign(Value::Table(table.clone())));
    let host = Rc::new(RefCell::new(()));
    let sprites = Rc::new(RefCell::new(()));
    let drops = Rc::new(Cell::new(0));
    list.borrow_mut()
        .push_entry(foreign(&list, &host, &sprites, &drops));
    let weak_list = Rc::downgrade(&list);
    let weak_link = Rc::downgrade(&link);
    drop(table);
    drop(link);
    let count = Rc::strong_count(&list);
    let guard = list.borrow();
    assert!(matches!(collect_tables(), CollectionResult::Deferred));
    assert_eq!(guard.len(), 3);
    drop(guard);
    assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
    assert_eq!(Rc::strong_count(&list), count);
    assert_eq!(list.borrow().len(), 3);
    assert_eq!(drops.get(), 0);
    drop(list);
    assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
    assert!(weak_list.upgrade().is_none());
    assert!(weak_link.upgrade().is_none());
    assert_eq!(drops.get(), 1);
}

#[test]
fn raw_list_registration_retries_without_replacing_owners_and_conflicts_stay_errors() {
    let list = WorldObjects::new_ref();
    let table = TableRef::new();
    let link = Rc::new(Link { list: list.clone() });
    register_userdata::<_, LinkDescriptor>(&link).unwrap();
    table.set_str("link", Value::Object(link.clone()));
    list.borrow_mut()
        .push_entry(WorldObjectEntry::Foreign(Value::Table(table.clone())));
    let weak_list = Rc::downgrade(&list);
    let weak_link = Rc::downgrade(&link);
    drop(table);
    drop(link);
    drop(list);
    assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
    let list = weak_list
        .upgrade()
        .expect("unregistered list remains conservative");
    assert_eq!(list.borrow().len(), 1);
    assert_eq!(register_world_objects(&list), Ok(true));
    struct ConflictingDescriptor;
    impl TraceDescriptor<WorldObjects> for ConflictingDescriptor {
        fn trace(_: &WorldObjects, _: &mut TraceVisitor) {}
    }
    assert_eq!(
        pz_kahlua::gc::register_existing::<_, ConflictingDescriptor>(&list),
        Err(RegistrationError::DescriptorConflict)
    );
    assert_eq!(register_world_objects(&list), Ok(false));
    drop(list);
    assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
    assert!(weak_list.upgrade().is_none());
    assert!(weak_link.upgrade().is_none());
}
