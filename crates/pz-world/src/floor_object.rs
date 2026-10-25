//! Stable runtime allocation for one floor object's complete payload.
//!
//! Moving a chunk or removing an object moves its [`FloorObject`] slot;
//! explicitly retained [`FloorObjectRef`]s still reach the same payload.
//! Cloning the slot is an archive/value copy, like cloning the surrounding
//! object or square. It creates a fresh payload and deep-copies its item.

use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use crate::objects::IsoWorldInventoryObject;

/// An explicit handle alias to the actual runtime floor payload.
pub type FloorObjectRef = Rc<RefCell<IsoWorldInventoryObject>>;

/// An owning slot of a floor object's payload, with archive `Clone` semantics.
#[derive(Debug)]
pub struct FloorObject {
    payload: FloorObjectRef,
}

impl FloorObject {
    /// Consumes a newly loaded or placed payload without copying its item.
    pub fn new(payload: IsoWorldInventoryObject) -> Self {
        Self {
            payload: Rc::new(RefCell::new(payload)),
        }
    }

    /// An explicit alias; unlike [`Self::clone`], this keeps runtime identity.
    pub fn ref_handle(&self) -> FloorObjectRef {
        Rc::clone(&self.payload)
    }

    /// An owning slot that explicitly aliases an existing runtime payload.
    pub fn retain(payload: &FloorObjectRef) -> Self {
        Self {
            payload: Rc::clone(payload),
        }
    }

    pub fn borrow(&self) -> Ref<'_, IsoWorldInventoryObject> {
        self.payload.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, IsoWorldInventoryObject> {
        self.payload.borrow_mut()
    }
}

impl Clone for FloorObject {
    fn clone(&self) -> Self {
        Self::new(self.payload.borrow().clone())
    }
}

impl PartialEq for FloorObject {
    fn eq(&self, other: &Self) -> bool {
        *self.payload.borrow() == *other.payload.borrow()
    }
}

#[cfg(test)]
mod tests;
