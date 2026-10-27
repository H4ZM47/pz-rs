//! Canonical runtime allocation for an entire grid square.
//!
//! Retaining a receiver is explicit. `try_archive_copy` makes a checked,
//! independent archive allocation without pending host roots.
use crate::{IsoGridSquare, ObjectSlot};
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::rc::Rc;

pub type SquareRef = Rc<RefCell<IsoGridSquare>>;
pub type SquareObserver = fn(&SquareRef);
thread_local! {
    static SQUARE_OBSERVER: Cell<Option<SquareObserver>> = const { Cell::new(None) };
}
pub fn install_square_observer(observer: Option<SquareObserver>) -> Option<SquareObserver> {
    SQUARE_OBSERVER.with(|slot| slot.replace(observer))
}
pub fn observe_square(square: &SquareRef) {
    if let Some(observer) = SQUARE_OBSERVER.with(Cell::get) {
        observer(square);
    }
}

#[derive(Debug)]
pub struct SquareSlot {
    square: SquareRef,
}
impl SquareSlot {
    pub fn new(square: IsoGridSquare) -> Self {
        let square = Rc::new(RefCell::new(square));
        observe_square(&square);
        Self { square }
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
    pub fn try_archive_copy(&self) -> Result<Self, crate::SquareArchiveError> {
        crate::try_archive_copy_square(&self.square)
    }
    pub fn try_archive_copy_with_policy(
        &self,
        policy: crate::archive::ForeignArchivePolicy,
    ) -> Result<Self, crate::SquareArchiveError> {
        crate::try_archive_copy_square_with_policy(&self.square, policy)
    }
}
impl From<IsoGridSquare> for SquareSlot {
    fn from(square: IsoGridSquare) -> Self {
        Self::new(square)
    }
}

/// Owners detached from live storage. Drop only after releasing world and
/// sprite guards. This bundle is host bookkeeping, never serialized.
#[derive(Default)]
pub struct RetiredWorldOwners {
    pub objects: Vec<ObjectSlot>,
    pub squares: Vec<SquareSlot>,
    pub components: pz_entity::RetiredComponents,
}
impl RetiredWorldOwners {
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty() && self.squares.is_empty() && self.components.is_empty()
    }
    pub fn append(&mut self, other: &mut Self) {
        self.objects.append(&mut other.objects);
        self.squares.append(&mut other.squares);
        self.components
            .append(std::mem::take(&mut other.components));
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
