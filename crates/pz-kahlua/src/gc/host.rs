//! Explicit ownership descriptions for the small set of reviewed host payloads
//! participating in collection. Opaque native captures need no implementation.

use super::{value_node, Collector, NodeId, COLLECTOR};
use crate::vm::{LuaClosure, NativeFunction, UpValueRef};
use crate::{TableRef, Value};
use std::any::{Any, TypeId};
use std::cell::{RefCell, RefMut};
use std::rc::{Rc, Weak};

/// An audited host payload's physical strong-reference slots.
///
/// `trace` must report each owning slot exactly once, borrowing the actual
/// slot rather than a cloned handle. Repeated slots pointing to the same
/// allocation must each be reported. Do not report transitive references,
/// weak references, temporary clones or references owned by shared backing
/// allocations. An omitted/opaque slot stays an external root. Reporting
/// nonexistent or duplicate slots can incorrectly classify live data.
///
/// These methods are collector bookkeeping, not Lua callbacks. They must not
/// execute user code, allocate new Lua/host nodes, mutate graph ownership or
/// invoke collection. `detach` must move out the reported mutable payload
/// without dropping its contents. The collector keeps that returned payload
/// alive until every heap borrow has been released. Immutable capture records
/// can use the default no-op detach; their outgoing owners must then break any
/// represented cycles. Never clear a live payload to force reclamation.
pub trait TracePayload: 'static {
    fn trace(&self, visitor: &mut TraceVisitor);

    fn detach(&mut self) -> Option<Box<dyn Any>> {
        None
    }
}

/// An IDs-only visitor. Methods inspect borrowed slots and add no strong Rc
/// pins. Each call describes one actual owning slot, not an entire subgraph.
#[derive(Default)]
pub struct TraceVisitor {
    pub(super) targets: Vec<NodeId>,
}

impl TraceVisitor {
    pub fn value(&mut self, slot: &Value) {
        if let Some(id) = value_node(slot) {
            self.targets.push(id);
        }
    }

    pub fn table(&mut self, slot: &TableRef) {
        self.targets.push(NodeId::Table(slot.ptr_id()));
    }

    pub fn closure(&mut self, slot: &Rc<LuaClosure>) {
        self.targets
            .push(NodeId::Closure(Rc::as_ptr(slot) as usize));
    }

    pub fn upvalue(&mut self, slot: &UpValueRef) {
        self.targets
            .push(NodeId::Upvalue(Rc::as_ptr(slot) as usize));
    }

    pub fn native(&mut self, slot: &Rc<NativeFunction>) {
        self.targets.push(NodeId::Native(Rc::as_ptr(slot) as usize));
    }

    pub fn owner<T: TracePayload>(&mut self, slot: &Rc<RefCell<T>>) {
        self.targets.push(owner_id::<T>(Rc::as_ptr(slot)));
    }
}

/// Allocate and weakly register one host backing allocation. Clones of the
/// returned Rc must not be separately registered. Raw Rc construction remains
/// supported conservatively: its outgoing references are external roots.
pub fn traceable<T: TracePayload>(payload: T) -> Rc<RefCell<T>> {
    let owner = Rc::new(RefCell::new(payload));
    let weak: Box<dyn HostWeak> = Box::new(PayloadWeak(Rc::downgrade(&owner)));
    let _ = COLLECTOR.try_with(|collector| {
        let mut collector = collector.borrow_mut();
        super::prepare_registration(&mut collector.hosts, |owner| owner.strong_count() != 0);
        collector.hosts.push(weak);
    });
    owner
}

pub(super) fn owner_id<T: TracePayload>(ptr: *const RefCell<T>) -> NodeId {
    NodeId::Host(TypeId::of::<T>(), ptr as usize)
}

pub(super) trait HostWeak {
    fn strong_count(&self) -> usize;
    fn upgrade(&self) -> Option<Box<dyn HostPin>>;
}

pub(super) trait HostPin {
    fn id(&self) -> NodeId;
    fn strong_count(&self) -> usize;
    fn borrow(&self) -> Result<Box<dyn HostGuard + '_>, ()>;
}

pub(super) trait HostGuard {
    fn trace(&self, visitor: &mut TraceVisitor);
    fn detach(&mut self) -> Option<Box<dyn Any>>;
}

struct PayloadWeak<T>(Weak<RefCell<T>>);
struct PayloadPin<T>(Rc<RefCell<T>>);
struct PayloadGuard<'a, T>(RefMut<'a, T>);

impl<T: TracePayload> HostWeak for PayloadWeak<T> {
    fn strong_count(&self) -> usize {
        self.0.strong_count()
    }

    fn upgrade(&self) -> Option<Box<dyn HostPin>> {
        self.0
            .upgrade()
            .map(|owner| Box::new(PayloadPin(owner)) as Box<dyn HostPin>)
    }
}

impl<T: TracePayload> HostPin for PayloadPin<T> {
    fn id(&self) -> NodeId {
        owner_id::<T>(Rc::as_ptr(&self.0))
    }

    fn strong_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }

    fn borrow(&self) -> Result<Box<dyn HostGuard + '_>, ()> {
        self.0
            .try_borrow_mut()
            .map(|guard| Box::new(PayloadGuard(guard)) as Box<dyn HostGuard>)
            .map_err(|_| ())
    }
}

impl<T: TracePayload> HostGuard for PayloadGuard<'_, T> {
    fn trace(&self, visitor: &mut TraceVisitor) {
        self.0.trace(visitor);
    }

    fn detach(&mut self) -> Option<Box<dyn Any>> {
        self.0.detach()
    }
}

impl Collector {
    pub(super) fn prune_hosts(&mut self) {
        self.hosts.retain(|owner| owner.strong_count() != 0);
    }
}
