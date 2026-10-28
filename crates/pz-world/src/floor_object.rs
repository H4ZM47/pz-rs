//! Stable runtime allocation for one floor object's complete payload.
//!
//! Moving a chunk or removing an object moves its [`FloorObject`] slot;
//! explicitly retained [`FloorObjectRef`]s still reach the same payload.
//! Cloning the slot is an archive/value copy, like cloning the surrounding
//! object or square. It creates a fresh payload and deep-copies its item.

use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use crate::objects::IsoWorldInventoryObject;

pub(crate) mod load_host;
pub mod runtime;

/// An explicit handle alias to the actual runtime floor payload.
pub type FloorObjectRef = Rc<RefCell<IsoWorldInventoryObject>>;

/// An owning slot of a floor object's payload, with archive `Clone` semantics.
#[derive(Debug)]
pub struct FloorObject {
    // None only after the collector moves the physical owning edge. This is
    // not an empty native floor object or an archive stand-in.
    payload: Option<FloorObjectRef>,
}

impl FloorObject {
    /// Consumes a newly loaded or placed payload without copying its item.
    pub fn new(payload: IsoWorldInventoryObject) -> Self {
        Self {
            payload: Some(Rc::new(RefCell::new(payload))),
        }
    }

    /// An explicit alias; unlike [`Self::clone`], this keeps runtime identity.
    pub fn ref_handle(&self) -> FloorObjectRef {
        Rc::clone(self.payload.as_ref().expect("collected floor payload"))
    }

    /// The physical owning slot; tracing must not synthesize a transitive edge.
    pub fn payload_slot(&self) -> Option<&FloorObjectRef> {
        self.payload.as_ref()
    }

    /// Collector bookkeeping only. No native lifecycle, reset or pool calls.
    pub fn take_payload_for_gc(&mut self) -> Option<FloorObjectRef> {
        self.payload.take()
    }

    /// An owning slot that explicitly aliases an existing runtime payload.
    pub fn retain(payload: &FloorObjectRef) -> Self {
        Self {
            payload: Some(Rc::clone(payload)),
        }
    }

    pub fn borrow(&self) -> Ref<'_, IsoWorldInventoryObject> {
        self.payload
            .as_ref()
            .expect("collected floor payload")
            .borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, IsoWorldInventoryObject> {
        self.payload
            .as_ref()
            .expect("collected floor payload")
            .borrow_mut()
    }
}

impl Clone for FloorObject {
    fn clone(&self) -> Self {
        match self.payload.as_ref() {
            Some(payload) => Self::new(payload.borrow().clone()),
            None => Self { payload: None },
        }
    }
}

impl PartialEq for FloorObject {
    fn eq(&self, other: &Self) -> bool {
        match (&self.payload, &other.payload) {
            (Some(left), Some(right)) => *left.borrow() == *right.borrow(),
            (None, None) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
