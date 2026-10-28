//! Registration describes the foreign backing itself, without owning adapters.
use super::*;
use crate::{LuaObject, NativeFunction, TableRef};
use std::any::Any;
use std::cell::Cell;

struct ForeignOwner {
    table: Option<TableRef>,
    number: i32,
    drops: Rc<Cell<usize>>,
}

impl Drop for ForeignOwner {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

struct Descriptor;
impl TraceDescriptor<ForeignOwner> for Descriptor {
    fn trace(payload: &ForeignOwner, visitor: &mut TraceVisitor) {
        if let Some(table) = &payload.table {
            visitor.table(table);
        }
    }

    fn detach(payload: &mut ForeignOwner) -> Option<Box<dyn Any>> {
        payload
            .table
            .take()
            .map(|table| Box::new(table) as Box<dyn Any>)
    }
}

struct OtherDescriptor;
impl TraceDescriptor<ForeignOwner> for OtherDescriptor {
    fn trace(_: &ForeignOwner, _: &mut TraceVisitor) {
        panic!("conflicting descriptor must not replace the registered one");
    }
}

struct Capture(Rc<RefCell<ForeignOwner>>);
impl TracePayload for Capture {
    fn trace(&self, visitor: &mut TraceVisitor) {
        visitor.owner(&self.0);
    }
}

fn cycle(registered: bool, drops: &Rc<Cell<usize>>) -> Rc<RefCell<ForeignOwner>> {
    let table = TableRef::new();
    let owner = Rc::new(RefCell::new(ForeignOwner {
        table: Some(table.clone()),
        number: 42,
        drops: drops.clone(),
    }));
    if registered {
        assert_eq!(register_existing::<_, Descriptor>(&owner), Ok(true));
    }
    let native =
        NativeFunction::new_traced_capture("foreign", Capture(owner.clone()), |_, _| Ok(0));
    table.set_str("callback", Value::Native(native));
    owner
}

fn collect() {
    assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
}

#[test]
fn existing_owner_preserves_raw_roots_and_collects_after_idempotent_registration() {
    let drops = Rc::new(Cell::new(0));
    let first = cycle(true, &drops);
    let second = cycle(true, &drops);
    let weak_first = Rc::downgrade(&first);
    let weak_second = Rc::downgrade(&second);
    let table = Rc::downgrade(&first.borrow().table.as_ref().unwrap().0);
    let strong_before = Rc::strong_count(&first);
    for _ in 0..100 {
        assert_eq!(register_existing::<_, Descriptor>(&first), Ok(false));
    }
    assert_eq!(Rc::strong_count(&first), strong_before);
    collect();
    assert_eq!(first.borrow().number, 42);
    assert!(TableRef(table.upgrade().unwrap())
        .get_str("callback")
        .is_function());
    drop(first);
    collect();
    assert!(weak_first.upgrade().is_none());
    assert!(table.upgrade().is_none());
    assert_eq!(drops.get(), 1);
    assert_eq!(second.borrow().number, 42);
    drop(second);
    collect();
    assert!(weak_second.upgrade().is_none());
    assert_eq!(drops.get(), 2);
}

#[test]
fn conflicting_descriptor_is_rejected_without_changing_graph_or_default_registration() {
    let drops = Rc::new(Cell::new(0));
    let owner = cycle(true, &drops);
    assert_eq!(
        register_existing::<_, OtherDescriptor>(&owner),
        Err(RegistrationError::DescriptorConflict)
    );
    assert_eq!(register_existing::<_, Descriptor>(&owner), Ok(false));
    collect();
    assert_eq!(owner.borrow().number, 42);
    drop(owner);
    collect();
    assert_eq!(drops.get(), 1);

    struct DefaultOwner;
    impl TracePayload for DefaultOwner {
        fn trace(&self, _: &mut TraceVisitor) {}
    }
    struct Replacement;
    impl TraceDescriptor<DefaultOwner> for Replacement {
        fn trace(_: &DefaultOwner, _: &mut TraceVisitor) {
            panic!("default traceable allocation must share the deduplication domain");
        }
    }
    let owner = traceable(DefaultOwner);
    assert_eq!(
        register_existing::<_, Replacement>(&owner),
        Err(RegistrationError::DescriptorConflict)
    );
    collect();
    assert_eq!(Rc::strong_count(&owner), 1);
}

#[test]
fn unregistered_allocations_of_same_type_remain_conservative() {
    let drops = Rc::new(Cell::new(0));
    let tracked = cycle(true, &drops);
    let unknown = cycle(false, &drops);
    let weak = Rc::downgrade(&unknown);
    drop(tracked);
    drop(unknown);
    collect();
    assert_eq!(drops.get(), 1);
    let unknown = weak
        .upgrade()
        .expect("unregistered outgoing table remains a root");
    assert_eq!(unknown.borrow().number, 42);
    let retired = unknown.borrow_mut().table.take();
    drop(retired);
    drop(unknown);
    collect();
    assert!(weak.upgrade().is_none());
    assert_eq!(drops.get(), 2);
}

struct ReentrantProbe {
    owner: Weak<RefCell<ForeignOwner>>,
    checked: Rc<Cell<bool>>,
}
impl LuaObject for ReentrantProbe {
    fn class_name(&self) -> &str {
        "AuthoredRetirementProbe"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl Drop for ReentrantProbe {
    fn drop(&mut self) {
        let owner = self
            .owner
            .upgrade()
            .expect("collector pin remains until retirement completes");
        owner
            .try_borrow_mut()
            .expect("all host guards must end before destructors")
            .number = 43;
        assert_eq!(collect_tables(), CollectionResult::Deferred);
        let fresh = TableRef::new();
        fresh.set_str("self", fresh.clone());
        self.checked.set(true);
    }
}

#[test]
fn borrowed_existing_owner_defers_entire_pass_and_retired_drops_can_reenter() {
    let drops = Rc::new(Cell::new(0));
    let owner = cycle(true, &drops);
    // The observer needs its own unreachable cycle to stay alive until
    // the collector pins it; registry metadata and the probe are weak.
    let observer = cycle(true, &drops);
    let observer_weak = Rc::downgrade(&observer);
    let checked = Rc::new(Cell::new(false));
    owner.borrow().table.as_ref().unwrap().set_str(
        "probe",
        Value::Object(Rc::new(ReentrantProbe {
            owner: observer_weak.clone(),
            checked: checked.clone(),
        })),
    );
    drop(observer);
    let unrelated = TableRef::new();
    unrelated.set_str("self", unrelated.clone());
    let unrelated_weak = Rc::downgrade(&unrelated.0);
    drop(unrelated);
    let held = owner.borrow_mut();
    assert_eq!(collect_tables(), CollectionResult::Deferred);
    assert!(!checked.get());
    assert!(
        unrelated_weak.upgrade().is_some(),
        "no partial sweep on borrow failure"
    );
    drop(held);
    let weak = Rc::downgrade(&owner);
    drop(owner);
    collect();
    assert!(checked.get());
    assert!(weak.upgrade().is_none());
    assert!(observer_weak.upgrade().is_none());
    assert!(unrelated_weak.upgrade().is_none());
    assert_eq!(drops.get(), 2);
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
}

#[test]
fn registration_during_registry_borrow_fails_conservatively_then_can_retry() {
    let drops = Rc::new(Cell::new(0));
    let owner = cycle(false, &drops);
    COLLECTOR.with(|collector| {
        let _held = collector.borrow();
        assert_eq!(
            register_existing::<_, Descriptor>(&owner),
            Err(RegistrationError::RegistryUnavailable)
        );
    });
    assert_eq!(register_existing::<_, Descriptor>(&owner), Ok(true));
    drop(owner);
    collect();
    assert_eq!(drops.get(), 1);
}

#[test]
fn vm_table_and_public_upvalue_backings_cannot_be_registered_as_host_nodes() {
    struct UpvalueDescriptor;
    impl TraceDescriptor<UpValue> for UpvalueDescriptor {
        fn trace(_: &UpValue, _: &mut TraceVisitor) {
            panic!("VM payload descriptor must never be installed");
        }
    }
    struct TableDescriptor;
    impl TraceDescriptor<Table> for TableDescriptor {
        fn trace(_: &Table, _: &mut TraceVisitor) {
            panic!("VM payload descriptor must never be installed");
        }
    }
    let upvalue = UpValue::closed(Value::Nil);
    let table = TableRef::new();
    assert_eq!(
        register_existing::<_, UpvalueDescriptor>(&upvalue),
        Err(RegistrationError::VmPayloadReserved)
    );
    assert_eq!(
        register_existing::<_, TableDescriptor>(&table.0),
        Err(RegistrationError::VmPayloadReserved)
    );
    let raw = Rc::new(RefCell::new(UpValue::Closed(Value::Nil)));
    assert_eq!(
        register_existing::<_, UpvalueDescriptor>(&raw),
        Err(RegistrationError::VmPayloadReserved)
    );
    // The existing VM records keep their one physical allocation identity.
    collect();
    assert_eq!(Rc::strong_count(&upvalue), 1);
    assert_eq!(Rc::strong_count(&table.0), 1);
}

#[test]
fn existing_owner_registry_prunes_dead_headers_and_indexes_live_aliases() {
    let drops = Rc::new(Cell::new(0));
    let mut roots = Vec::new();
    for number in 0..2048 {
        let owner = Rc::new(RefCell::new(ForeignOwner {
            table: None,
            number,
            drops: drops.clone(),
        }));
        assert_eq!(register_existing::<_, Descriptor>(&owner), Ok(true));
        roots.push(owner);
    }
    for owner in roots.iter().rev() {
        assert_eq!(register_existing::<_, Descriptor>(owner), Ok(false));
    }
    collect();
    for (number, owner) in roots.iter().enumerate() {
        assert_eq!(owner.borrow().number, number as i32);
        assert_eq!(Rc::strong_count(owner), 1);
    }
    drop(roots);
    collect();
    assert_eq!(drops.get(), 2048);
    COLLECTOR.with(|collector| assert!(collector.borrow().host_indices.is_empty()));
    // A fresh registry verifies bounded dead headers without inheriting the
    // deliberate headroom of the preceding 2048 live allocations.
    let churn_drops = std::thread::spawn(|| {
        let drops = Rc::new(Cell::new(0));
        for number in 0..10_000 {
            let owner = Rc::new(RefCell::new(ForeignOwner {
                table: None,
                number,
                drops: drops.clone(),
            }));
            assert_eq!(register_existing::<_, Descriptor>(&owner), Ok(true));
        }
        COLLECTOR.with(|collector| {
            let collector = collector.borrow();
            assert!(collector.hosts.len() <= 4);
            assert_eq!(collector.host_indices.len(), collector.hosts.len());
            for (index, host) in collector.hosts.iter().enumerate() {
                assert_eq!(collector.host_indices.get(&host.id()), Some(&index));
            }
        });
        collect();
        drops.get()
    })
    .join()
    .unwrap();
    assert_eq!(churn_drops, 10_000);
}
