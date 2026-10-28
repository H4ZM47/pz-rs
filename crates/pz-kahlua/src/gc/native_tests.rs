use super::*;
use crate::{NativeCall, NativeFunction, TableRef, Thread};
use std::any::Any;
use std::cell::Cell;

#[derive(Default)]
struct Slots {
    values: Vec<Value>,
}

impl TracePayload for Slots {
    fn trace(&self, visitor: &mut TraceVisitor) {
        for value in &self.values {
            visitor.value(value);
        }
    }
    fn detach(&mut self) -> Option<Box<dyn Any>> {
        Some(Box::new(std::mem::take(&mut self.values)))
    }
}

struct Capture {
    first: Rc<RefCell<Slots>>,
    second: Rc<RefCell<Slots>>,
}

impl TracePayload for Capture {
    fn trace(&self, visitor: &mut TraceVisitor) {
        visitor.owner(&self.first);
        visitor.owner(&self.second);
    }
}

fn invoke(capture: &RefCell<Capture>, call: &mut NativeCall<'_>) -> crate::LuaResult<usize> {
    let owner = capture.borrow().first.clone();
    let value = owner.borrow().values[0].clone();
    Ok(call.push(value))
}

fn collect() {
    assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
}

#[test]
fn explicit_native_capture_and_shared_host_slots_preserve_roots_then_collect() {
    let env = TableRef::new();
    let owner = traceable(Slots {
        values: vec![Value::Num(137.0), env.clone().into(), env.clone().into()],
    });
    let native = NativeFunction::new_traced_capture(
        "slots",
        Capture {
            first: owner.clone(),
            second: owner.clone(),
        },
        invoke,
    );
    env.rawset(Value::Native(native.clone()), Value::Native(native.clone()));
    let env_weak = Rc::downgrade(&env.0);
    let owner_weak = Rc::downgrade(&owner);
    let native_weak = Rc::downgrade(&native);
    drop(env);
    drop(owner);
    collect();
    let mut thread = Thread::new(TableRef::new());
    assert_eq!(
        thread.call(&Value::Native(native.clone()), &[]).unwrap()[0].as_num(),
        Some(137.0)
    );
    assert_eq!(owner_weak.upgrade().unwrap().borrow().values.len(), 3);
    drop(native);
    collect();
    assert!(native_weak.upgrade().is_none());
    assert!(owner_weak.upgrade().is_none());
    assert!(env_weak.upgrade().is_none());
}

#[test]
fn retained_host_owner_and_raw_unregistered_owner_have_conservative_lifetimes() {
    for registered in [true, false] {
        let env = TableRef::new();
        let slots = Slots {
            values: vec![Value::Num(139.0), env.clone().into()],
        };
        let owner = if registered {
            traceable(slots)
        } else {
            Rc::new(RefCell::new(slots))
        };
        let native = NativeFunction::new_traced_capture(
            "slots",
            Capture {
                first: owner.clone(),
                second: owner.clone(),
            },
            invoke,
        );
        env.set_str("native", Value::Native(native.clone()));
        let native_weak = Rc::downgrade(&native);
        let owner_weak = Rc::downgrade(&owner);
        drop(native);
        drop(env);
        collect();
        assert_eq!(owner.borrow().values[0].as_num(), Some(139.0));
        drop(owner);
        collect();
        if registered {
            assert!(owner_weak.upgrade().is_none());
            assert!(native_weak.upgrade().is_none());
        } else {
            // The raw owner was never registered. Its real outgoing Value
            // holds are not subtracted, so its cycle remains retained.
            let owner = owner_weak.upgrade().unwrap();
            assert_eq!(owner.borrow().values[0].as_num(), Some(139.0));
            assert!(native_weak.upgrade().is_some());
            owner.borrow_mut().values.clear();
            drop(owner);
            assert!(native_weak.upgrade().is_none());
            assert!(owner_weak.upgrade().is_none());
        }
    }
}

struct BorrowProbe {
    dead: Weak<RefCell<Table>>,
    checked: Rc<Cell<bool>>,
}

impl TracePayload for BorrowProbe {
    fn trace(&self, _: &mut TraceVisitor) {}
}

fn probe_borrow(capture: &RefCell<BorrowProbe>, _: &mut NativeCall<'_>) -> crate::LuaResult<usize> {
    let borrow = capture.borrow();
    assert_eq!(collect_tables(), CollectionResult::Deferred);
    assert!(borrow.dead.upgrade().is_some());
    borrow.checked.set(true);
    Ok(0)
}

#[test]
fn held_capture_backing_borrow_defers_and_drains_after_the_native_returns() {
    let env = TableRef::new();
    env.set_str("self", env.clone());
    let dead = Rc::downgrade(&env.0);
    drop(env);
    let checked = Rc::new(Cell::new(false));
    let native = NativeFunction::new_traced_capture(
        "borrow",
        BorrowProbe {
            dead: dead.clone(),
            checked: checked.clone(),
        },
        probe_borrow,
    );
    let mut thread = Thread::new(TableRef::new());
    thread.call(&Value::Native(native), &[]).unwrap();
    assert!(checked.get());
    assert!(dead.upgrade().is_none());
    assert_eq!(collect_pending(), CollectionResult::NotRequested);
}

struct EmptyCapture;
impl TracePayload for EmptyCapture {
    fn trace(&self, _: &mut TraceVisitor) {}
}

fn empty_native() -> Rc<NativeFunction> {
    NativeFunction::new_traced_capture("empty", EmptyCapture, |_, _| Ok(0))
}

#[test]
fn native_and_host_weak_headers_are_bounded_without_collection_requests() {
    for _ in 0..10_000 {
        drop(empty_native());
    }
    COLLECTOR.with(|collector| {
        let collector = collector.borrow();
        assert!(collector.natives.len() <= 4);
        assert!(collector.hosts.len() <= 4);
    });
}

#[test]
fn native_and_host_registries_reserve_headroom_when_most_entries_are_live() {
    let mut roots: Vec<_> = (0..1024).map(|_| empty_native()).collect();
    // Stop at whichever registry reaches capacity first. Both registries
    // have one weak record per typed native, but need not share a capacity.
    loop {
        let full = COLLECTOR.with(|collector| {
            let collector = collector.borrow();
            collector.natives.len() == collector.natives.capacity()
                || collector.hosts.len() == collector.hosts.capacity()
        });
        if full {
            break;
        }
        roots.push(empty_native());
    }
    let (native_capacity, host_capacity) = COLLECTOR.with(|collector| {
        let collector = collector.borrow();
        (collector.natives.capacity(), collector.hosts.capacity())
    });
    let previous_count = roots.len();
    drop(roots.pop());
    drop(empty_native());
    COLLECTOR.with(|collector| {
        let collector = collector.borrow();
        if previous_count == native_capacity {
            assert!(collector.natives.capacity() >= native_capacity + roots.len());
        }
        if previous_count == host_capacity {
            assert!(collector.hosts.capacity() >= host_capacity + roots.len());
        }
    });
    collect();
    assert!(roots.iter().all(|native| Rc::strong_count(native) == 1));
}
