//! One canonical runtime allocation for an entire world object.
//!
//! Slots move with their square/chunk. `Clone` is an independent archive
//! copy; retaining a runtime receiver is always an explicit handle operation.
use crate::objects::IsoObject;
use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

pub type IsoObjectRef = Rc<RefCell<IsoObject>>;

#[derive(Debug)]
pub struct ObjectSlot {
    object: IsoObjectRef,
}
impl ObjectSlot {
    pub fn new(object: IsoObject) -> Self {
        Self {
            object: Rc::new(RefCell::new(object)),
        }
    }
    pub fn ref_handle(&self) -> IsoObjectRef {
        Rc::clone(&self.object)
    }
    pub fn retain(object: &IsoObjectRef) -> Self {
        Self {
            object: Rc::clone(object),
        }
    }
    pub fn borrow(&self) -> Ref<'_, IsoObject> {
        self.object.borrow()
    }
    pub fn borrow_mut(&self) -> RefMut<'_, IsoObject> {
        self.object.borrow_mut()
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.object, &other.object)
    }
    pub fn ptr_eq_ref(&self, other: &IsoObjectRef) -> bool {
        Rc::ptr_eq(&self.object, other)
    }
}
impl From<IsoObject> for ObjectSlot {
    fn from(object: IsoObject) -> Self {
        Self::new(object)
    }
}
impl Clone for ObjectSlot {
    fn clone(&self) -> Self {
        Self::new(self.borrow().clone())
    }
}
impl PartialEq for ObjectSlot {
    fn eq(&self, other: &Self) -> bool {
        *self.borrow() == *other.borrow()
    }
}

#[cfg(test)]
mod tests;

/// Pending host-owned receivers are not part of an archive snapshot or
/// equality. Their destruction is explicitly deferred to a host boundary.
#[derive(Debug, Default)]
pub(crate) struct ObjectRetirements(pub Vec<ObjectSlot>);
impl Clone for ObjectRetirements {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for ObjectRetirements {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
