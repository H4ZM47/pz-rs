//! Explicit ownership of actual immutable Lua userdata allocations.

use super::{Collector, NodeId, RegistrationError, TraceVisitor, COLLECTOR};
use crate::value::LuaObject;
use std::any::TypeId;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

/// An interior borrow prevented a complete physical-slot description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceDeferred;

/// A static description of an actual userdata allocation's owning slots.
///
/// The physical-slot, no-callback and no-mutation requirements of
/// [`super::TracePayload`] apply here too. Report each owning slot once, not
/// temporary clones or transitive references. An unavailable interior borrow
/// must return [`TraceDeferred`], deferring the entire collection.
///
/// Userdata is immutable to this protocol and is never detached or rewritten.
/// Cycles need a detachable table, upvalue or host edge to be reclaimed;
/// immutable-only cycles remain retained. Unknown userdata remains opaque.
pub trait UserdataDescriptor<T: LuaObject>: 'static {
    fn trace(payload: &T, visitor: &mut TraceVisitor) -> Result<(), TraceDeferred>;
}

/// Register the actual typed userdata `Rc`, before erasing it to `Value::Object`.
///
/// Metadata is weak; the collector pins this allocation itself exactly once.
/// No wrapper allocation stands in for its reference count. Returns `true` for
/// first registration, `false` for the same allocation/descriptor again.
/// A different descriptor or an allocation already tracked under another node
/// kind is rejected without replacing the original record.
pub fn register_userdata<T: LuaObject, D: UserdataDescriptor<T>>(
    owner: &Rc<T>,
) -> Result<bool, RegistrationError> {
    let target = TypeId::of::<T>();
    // These exact allocation shapes belong to existing VM node protocols.
    // Reserving manually constructed instances too prevents later overlap.
    if target == TypeId::of::<RefCell<crate::table::Table>>()
        || target == TypeId::of::<RefCell<crate::vm::UpValue>>()
        || target == TypeId::of::<crate::vm::LuaClosure>()
        || target == TypeId::of::<crate::vm::NativeFunction>()
    {
        return Err(RegistrationError::VmPayloadReserved);
    }
    COLLECTOR
        .try_with(|collector| {
            let mut collector = collector
                .try_borrow_mut()
                .map_err(|_| RegistrationError::RegistryUnavailable)?;
            collector.register_userdata(Box::new(UserdataRecord::<T, D>(
                Rc::downgrade(owner),
                PhantomData,
            )))
        })
        .unwrap_or(Err(RegistrationError::RegistryUnavailable))
}

pub(super) fn address<T: ?Sized>(ptr: *const T) -> usize {
    ptr as *const () as usize
}

pub(super) trait UserdataWeak {
    fn address(&self) -> usize;
    fn target_type(&self) -> TypeId;
    fn descriptor(&self) -> TypeId;
    fn strong_count(&self) -> usize;
    fn upgrade(&self) -> Option<Box<dyn UserdataPin>>;
}

pub(super) trait UserdataPin {
    fn id(&self) -> NodeId;
    fn strong_count(&self) -> usize;
    fn trace(&self, visitor: &mut TraceVisitor) -> Result<(), TraceDeferred>;
}

struct UserdataRecord<T, D>(Weak<T>, PhantomData<fn() -> D>);
struct Pin<T, D>(Rc<T>, PhantomData<fn() -> D>);

impl<T: LuaObject, D: UserdataDescriptor<T>> UserdataWeak for UserdataRecord<T, D> {
    fn address(&self) -> usize {
        address(self.0.as_ptr())
    }

    fn target_type(&self) -> TypeId {
        TypeId::of::<T>()
    }

    fn descriptor(&self) -> TypeId {
        TypeId::of::<D>()
    }

    fn strong_count(&self) -> usize {
        self.0.strong_count()
    }

    fn upgrade(&self) -> Option<Box<dyn UserdataPin>> {
        self.0
            .upgrade()
            .map(|owner| Box::new(Pin::<T, D>(owner, PhantomData)) as Box<dyn UserdataPin>)
    }
}

impl<T: LuaObject, D: UserdataDescriptor<T>> UserdataPin for Pin<T, D> {
    fn id(&self) -> NodeId {
        NodeId::Userdata(address(Rc::as_ptr(&self.0)))
    }

    fn strong_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }

    fn trace(&self, visitor: &mut TraceVisitor) -> Result<(), TraceDeferred> {
        D::trace(&self.0, visitor)
    }
}

impl Collector {
    fn register_userdata(
        &mut self,
        owner: Box<dyn UserdataWeak>,
    ) -> Result<bool, RegistrationError> {
        let address = owner.address();
        if self.has_host_address(address) {
            return Err(RegistrationError::AllocationKindConflict);
        }
        if let Some(&index) = self.userdata_indices.get(&address) {
            let previous = &self.userdata[index];
            if previous.strong_count() != 0 {
                return if previous.target_type() != owner.target_type() {
                    Err(RegistrationError::AllocationKindConflict)
                } else if previous.descriptor() == owner.descriptor() {
                    Ok(false)
                } else {
                    Err(RegistrationError::DescriptorConflict)
                };
            }
            self.prune_userdata();
        }
        if self.userdata.len() == self.userdata.capacity() {
            let capacity = self.userdata.capacity();
            self.prune_userdata();
            if self.userdata.len() > capacity / 2 {
                self.userdata.reserve(capacity);
            }
        }
        self.userdata_indices.insert(address, self.userdata.len());
        self.userdata.push(owner);
        Ok(true)
    }

    pub(super) fn has_userdata_address(&mut self, address: usize) -> bool {
        let Some(&index) = self.userdata_indices.get(&address) else {
            return false;
        };
        if self.userdata[index].strong_count() != 0 {
            true
        } else {
            self.prune_userdata();
            false
        }
    }

    pub(super) fn prune_userdata(&mut self) {
        self.userdata.retain(|owner| owner.strong_count() != 0);
        self.userdata_indices.clear();
        self.userdata_indices.extend(
            self.userdata
                .iter()
                .enumerate()
                .map(|(index, owner)| (owner.address(), index)),
        );
    }
}

#[cfg(test)]
mod tests;
