//! Canonical runtime allocation for an entire grid square.
//!
//! Retaining a receiver is explicit. `Clone` makes an independent archive
//! allocation, including independent object slots, without pending host roots.
use crate::{IsoGridSquare, ObjectSlot};
use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

pub type SquareRef = Rc<RefCell<IsoGridSquare>>;

#[derive(Debug)]
pub struct SquareSlot {
    square: SquareRef,
}
impl SquareSlot {
    pub fn new(square: IsoGridSquare) -> Self {
        Self {
            square: Rc::new(RefCell::new(square)),
        }
    }
    pub fn ref_handle(&self) -> SquareRef {
        Rc::clone(&self.square)
    }
    pub fn retain(square: &SquareRef) -> Self {
        Self {
            square: Rc::clone(square),
        }
    }
    pub fn borrow(&self) -> Ref<'_, IsoGridSquare> {
        self.square.borrow()
    }
    pub fn borrow_mut(&self) -> RefMut<'_, IsoGridSquare> {
        self.square.borrow_mut()
    }
    pub(crate) fn square_borrow_for_retirement(
        &self,
    ) -> Result<RefMut<'_, IsoGridSquare>, std::cell::BorrowMutError> {
        self.square.try_borrow_mut()
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.square, &other.square)
    }
    pub fn ptr_eq_ref(&self, other: &SquareRef) -> bool {
        Rc::ptr_eq(&self.square, other)
    }
}
impl From<IsoGridSquare> for SquareSlot {
    fn from(square: IsoGridSquare) -> Self {
        Self::new(square)
    }
}
impl Clone for SquareSlot {
    fn clone(&self) -> Self {
        Self::new(self.borrow().clone())
    }
}
impl PartialEq for SquareSlot {
    fn eq(&self, other: &Self) -> bool {
        *self.borrow() == *other.borrow()
    }
}

/// Owners detached from live storage. Drop only after releasing world and
/// sprite guards. This bundle is host bookkeeping, never serialized.
#[derive(Default)]
pub struct RetiredWorldOwners {
    pub objects: Vec<ObjectSlot>,
    pub squares: Vec<SquareSlot>,
}
impl RetiredWorldOwners {
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty() && self.squares.is_empty()
    }
    pub fn append(&mut self, other: &mut Self) {
        self.objects.append(&mut other.objects);
        self.squares.append(&mut other.squares);
    }
}

/// Take host-only roots without changing a retained square's game data.
/// Occupied receivers stay queued and are retried at a later host boundary.
pub(crate) fn drain_squares(pending: &mut Vec<SquareSlot>, retired: &mut RetiredWorldOwners) {
    let slots = std::mem::take(pending);
    for slot in slots {
        let handle = slot.ref_handle();
        let Ok(mut square) = handle.try_borrow_mut() else {
            pending.push(slot);
            continue;
        };
        retired.objects.extend(square.take_retired_objects());
        drop(square);
        retired.squares.push(slot);
    }
}

#[cfg(test)]
mod tests;
