//! Explicit ownership descriptions for the small set of reviewed host payloads
//! participating in collection. Opaque native captures need no implementation.

use super::{value_node, Collector, NodeId, COLLECTOR};
use crate::vm::{LuaClosure, NativeFunction, UpValueRef};
use crate::{TableRef, Value};
use std::any::{Any, TypeId};
use std::cell::{RefCell, RefMut};
use std::marker::PhantomData;
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

/// A capture-free ownership description for an existing backing allocation.
///
/// A host can implement this trait on its own descriptor type even when the
/// payload type belongs to another crate. No descriptor instance or extra
/// owning adapter is allocated. The physical-slot and no-callback requirements
/// of [`TracePayload`] apply equally to these methods. Registering does not
/// change the payload, its identity, or its owners.
pub trait TraceDescriptor<T: 'static>: 'static {
    fn trace(payload: &T, visitor: &mut TraceVisitor);

    fn detach(_payload: &mut T) -> Option<Box<dyn Any>> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationError {
    /// Tables and upvalues belong to their existing VM node protocols.
    /// Their RefCell payload types cannot also become host nodes.
    VmPayloadReserved,
    /// This allocation already has a different ownership description.
    DescriptorConflict,
    /// Thread-local teardown or an enclosing registry borrow prevented registration.
    /// The allocation remains an ordinary conservative external owner.
    RegistryUnavailable,
}

/// Weakly register the actual existing `Rc`, without cloning an owning handle.
///
/// Returns `true` on first registration and `false` for the same allocation and
/// descriptor again. A different descriptor is rejected before changing the
/// existing record. Allocation identity always uses `T` and the backing address,
/// independently of the descriptor type. Unregistered owners remain supported
/// conservatively; their outgoing references remain external roots.
/// VM `Table` and `UpValue` payload types are reserved, including manually
/// constructed instances. Their standard VM nodes must never be counted again
/// as host nodes for the same physical allocation.
pub fn register_existing<T: 'static, D: TraceDescriptor<T>>(
    owner: &Rc<RefCell<T>>,
) -> Result<bool, RegistrationError> {
    if TypeId::of::<T>() == TypeId::of::<crate::table::Table>()
        || TypeId::of::<T>() == TypeId::of::<crate::vm::UpValue>()
    {
        return Err(RegistrationError::VmPayloadReserved);
    }
    COLLECTOR
        .try_with(|collector| {
            let mut collector = collector
                .try_borrow_mut()
                .map_err(|_| RegistrationError::RegistryUnavailable)?;
            collector.register_host(Box::new(PayloadWeak::<T, D>(
                Rc::downgrade(owner),
                PhantomData,
            )))
        })
        .unwrap_or(Err(RegistrationError::RegistryUnavailable))
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

    /// One physical host-owner slot. Use `table` or `upvalue` for VM nodes;
    /// those payload types are reserved from host registration.
    pub fn owner<T: 'static>(&mut self, slot: &Rc<RefCell<T>>) {
        self.targets.push(owner_id::<T>(Rc::as_ptr(slot)));
    }
}

/// Allocate and weakly register one host backing allocation. Clones of the
/// returned Rc need not be separately registered. Raw Rc construction remains
/// supported conservatively: its outgoing references are external roots.
pub fn traceable<T: TracePayload>(payload: T) -> Rc<RefCell<T>> {
    let owner = Rc::new(RefCell::new(payload));
    let _ = register_existing::<T, PayloadTrace<T>>(&owner);
    owner
}

pub(super) fn owner_id<T: 'static>(ptr: *const RefCell<T>) -> NodeId {
    NodeId::Host(TypeId::of::<T>(), ptr as usize)
}

pub(super) trait HostWeak {
    fn id(&self) -> NodeId;
    fn descriptor(&self) -> TypeId;
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

struct PayloadTrace<T>(PhantomData<fn() -> T>);

impl<T: TracePayload> TraceDescriptor<T> for PayloadTrace<T> {
    fn trace(payload: &T, visitor: &mut TraceVisitor) {
        payload.trace(visitor);
    }

    fn detach(payload: &mut T) -> Option<Box<dyn Any>> {
        payload.detach()
    }
}

struct PayloadWeak<T, D>(Weak<RefCell<T>>, PhantomData<fn() -> D>);
struct PayloadPin<T, D>(Rc<RefCell<T>>, PhantomData<fn() -> D>);
struct PayloadGuard<'a, T, D>(RefMut<'a, T>, PhantomData<fn() -> D>);

impl<T: 'static, D: TraceDescriptor<T>> HostWeak for PayloadWeak<T, D> {
    fn id(&self) -> NodeId {
        owner_id::<T>(self.0.as_ptr())
    }

    fn descriptor(&self) -> TypeId {
        TypeId::of::<D>()
    }

    fn strong_count(&self) -> usize {
        self.0.strong_count()
    }

    fn upgrade(&self) -> Option<Box<dyn HostPin>> {
        self.0
            .upgrade()
            .map(|owner| Box::new(PayloadPin::<T, D>(owner, PhantomData)) as Box<dyn HostPin>)
    }
}

impl<T: 'static, D: TraceDescriptor<T>> HostPin for PayloadPin<T, D> {
    fn id(&self) -> NodeId {
        owner_id::<T>(Rc::as_ptr(&self.0))
    }

    fn strong_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }

    fn borrow(&self) -> Result<Box<dyn HostGuard + '_>, ()> {
        self.0
            .try_borrow_mut()
            .map(|guard| Box::new(PayloadGuard::<T, D>(guard, PhantomData)) as Box<dyn HostGuard>)
            .map_err(|_| ())
    }
}

impl<T: 'static, D: TraceDescriptor<T>> HostGuard for PayloadGuard<'_, T, D> {
    fn trace(&self, visitor: &mut TraceVisitor) {
        D::trace(&self.0, visitor);
    }

    fn detach(&mut self) -> Option<Box<dyn Any>> {
        D::detach(&mut self.0)
    }
}

impl Collector {
    fn register_host(&mut self, owner: Box<dyn HostWeak>) -> Result<bool, RegistrationError> {
        let id = owner.id();
        if let Some(&index) = self.host_indices.get(&id) {
            let previous = &self.hosts[index];
            if previous.strong_count() != 0 {
                return if previous.descriptor() == owner.descriptor() {
                    Ok(false)
                } else {
                    Err(RegistrationError::DescriptorConflict)
                };
            }
            // Remove dead metadata before considering a reused allocation ID.
            self.prune_hosts();
        }
        if self.hosts.len() == self.hosts.capacity() {
            let capacity = self.hosts.capacity();
            self.prune_hosts();
            if self.hosts.len() > capacity / 2 {
                self.hosts.reserve(capacity);
            }
        }
        self.host_indices.insert(id, self.hosts.len());
        self.hosts.push(owner);
        Ok(true)
    }

    pub(super) fn prune_hosts(&mut self) {
        self.hosts.retain(|owner| owner.strong_count() != 0);
        self.host_indices.clear();
        self.host_indices.extend(
            self.hosts
                .iter()
                .enumerate()
                .map(|(index, owner)| (owner.id(), index)),
        );
    }
}
