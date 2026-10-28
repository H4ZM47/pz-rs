use super::*;
use crate::value::LuaObject;
use crate::{TableRef, Thread};
use std::any::Any;
use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};

fn unreachable_table() -> Weak<RefCell<Table>> {
    let table = TableRef::new();
    table.set_str("self", table.clone());
    Rc::downgrade(&table.0)
}

#[test]
fn deferral_alone_neither_requests_nor_runs_collection() {
    let _ = collect_tables();
    let weak = unreachable_table();
    let guard = defer_collection();
    assert_eq!(collect_pending(), CollectionResult::NotRequested);
    drop(guard);
    assert!(weak.upgrade().is_some());
    assert_eq!(collect_pending(), CollectionResult::NotRequested);
    assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
    assert!(weak.upgrade().is_none());
}

#[test]
fn nested_guards_keep_explicit_and_queued_requests_pending_without_drop_drain() {
    let weak = unreachable_table();
    let outer = defer_collection();
    let inner = defer_collection();
    assert_eq!(collect_tables(), CollectionResult::Deferred);
    assert_eq!(collect_pending(), CollectionResult::Deferred);
    drop(outer);
    assert_eq!(collect_pending(), CollectionResult::Deferred);
    assert!(weak.upgrade().is_some());
    drop(inner);
    assert!(weak.upgrade().is_some(), "last guard Drop must not collect");
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert!(weak.upgrade().is_none());
    assert_eq!(collect_pending(), CollectionResult::NotRequested);
}

#[test]
fn unwind_restores_depth_but_leaves_request_for_explicit_safe_drain() {
    let weak = unreachable_table();
    let outer = defer_collection();
    let failure = catch_unwind(AssertUnwindSafe(|| {
        let _inner = defer_collection();
        request_collection();
        panic!("authored host callback failure");
    }));
    assert!(failure.is_err());
    assert_eq!(collect_pending(), CollectionResult::Deferred);
    drop(outer);
    assert!(weak.upgrade().is_some());
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert!(weak.upgrade().is_none());
}

#[test]
fn real_interpreter_call_boundaries_respect_enclosing_host_scope() {
    let weak = unreachable_table();
    let observed = weak.clone();
    let function = Value::Native(NativeFunction::new("request", move |call| {
        request_collection();
        let inner = Value::Native(NativeFunction::new("inner", |_| {
            assert_eq!(collect_tables(), CollectionResult::Deferred);
            Ok(0)
        }));
        call.thread.call(&inner, &[])?;
        assert!(observed.upgrade().is_some());
        Ok(0)
    }));
    let mut thread = Thread::new(TableRef::new());
    let guard = defer_collection();
    thread.call(&function, &[]).unwrap();
    assert!(weak.upgrade().is_some());
    assert_eq!(collect_pending(), CollectionResult::Deferred);
    drop(guard);
    assert!(weak.upgrade().is_some());
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert!(weak.upgrade().is_none());
}

struct ReentrantDrop {
    first: Rc<RefCell<usize>>,
    second: Rc<RefCell<usize>>,
    dropped: Rc<Cell<bool>>,
}

impl LuaObject for ReentrantDrop {
    fn class_name(&self) -> &str {
        "test.DeferralDrop"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for ReentrantDrop {
    fn drop(&mut self) {
        *self.first.borrow_mut() += 1;
        *self.second.borrow_mut() += 1;
        self.dropped.set(true);
        // A final destructor can allocate and request another pass, but
        // cannot start a sweep recursively inside the current sweep.
        let _table = TableRef::new();
        assert_eq!(collect_tables(), CollectionResult::Deferred);
    }
}

#[test]
fn final_drop_reenters_two_host_cells_only_after_their_guards_end() {
    let first = Rc::new(RefCell::new(0));
    let second = Rc::new(RefCell::new(0));
    let dropped = Rc::new(Cell::new(false));
    let table = TableRef::new();
    table.set_str("self", table.clone());
    table.set_str(
        "drop",
        Value::Object(Rc::new(ReentrantDrop {
            first: first.clone(),
            second: second.clone(),
            dropped: dropped.clone(),
        })),
    );
    drop(table);
    let guard = defer_collection();
    {
        let _first = first.borrow_mut();
        let _second = second.borrow_mut();
        request_collection();
        assert_eq!(collect_pending(), CollectionResult::Deferred);
        assert!(!dropped.get());
    }
    drop(guard);
    assert!(!dropped.get());
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert!(dropped.get());
    assert_eq!((*first.borrow(), *second.borrow()), (1, 1));
    // The destructor's nested request survived the completed outer pass.
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert_eq!(collect_pending(), CollectionResult::NotRequested);
}

struct ReentrantHost {
    self_ref: Option<Rc<RefCell<ReentrantHost>>>,
    _probe: ReentrantDrop,
}

impl TracePayload for ReentrantHost {
    fn trace(&self, visitor: &mut TraceVisitor) {
        if let Some(owner) = &self.self_ref {
            visitor.owner(owner);
        }
    }

    fn detach(&mut self) -> Option<Box<dyn Any>> {
        self.self_ref
            .take()
            .map(|owner| Box::new(owner) as Box<dyn Any>)
    }
}

#[test]
fn final_collector_node_pin_drop_also_waits_for_host_guards() {
    let first = Rc::new(RefCell::new(0));
    let second = Rc::new(RefCell::new(0));
    let dropped = Rc::new(Cell::new(false));
    let owner = traceable(ReentrantHost {
        self_ref: None,
        _probe: ReentrantDrop {
            first: first.clone(),
            second: second.clone(),
            dropped: dropped.clone(),
        },
    });
    owner.borrow_mut().self_ref = Some(owner.clone());
    let weak = Rc::downgrade(&owner);
    drop(owner);
    let guard = defer_collection();
    {
        let _first = first.borrow_mut();
        let _second = second.borrow_mut();
        assert_eq!(collect_tables(), CollectionResult::Deferred);
        assert!(!dropped.get());
    }
    drop(guard);
    assert!(weak.upgrade().is_some());
    // detach moves only the self edge. The host body and probe survive
    // until the collector's final node pin drops after all heap guards.
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert!(weak.upgrade().is_none());
    assert!(dropped.get());
    assert_eq!((*first.borrow(), *second.borrow()), (1, 1));
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert_eq!(collect_pending(), CollectionResult::NotRequested);
}

#[test]
fn deferral_depth_is_local_to_the_current_thread() {
    let weak = unreachable_table();
    let guard = defer_collection();
    request_collection();
    std::thread::spawn(|| {
        let weak = unreachable_table();
        assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
        assert!(weak.upgrade().is_none());
    })
    .join()
    .unwrap();
    assert_eq!(collect_pending(), CollectionResult::Deferred);
    drop(guard);
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert!(weak.upgrade().is_none());
}
