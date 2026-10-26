//! One canonical runtime allocation for an entire world object.
//!
//! Slots move with their square/chunk. `Clone` is an independent archive
//! copy; retaining a runtime receiver is always an explicit handle operation.
use crate::objects::IsoObject;
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::rc::Rc;

pub type IsoObjectRef = Rc<RefCell<IsoObject>>;

pub type ObjectObserver = fn(&IsoObjectRef);

thread_local! {
    static OBJECT_OBSERVER: Cell<Option<ObjectObserver>> = const { Cell::new(None) };
}

/// Metadata only: observers register physical allocations without invoking
/// Lua, collection, component lifecycle, or modifying component membership.
pub fn install_object_observer(observer: Option<ObjectObserver>) -> Option<ObjectObserver> {
    OBJECT_OBSERVER.with(|slot| slot.replace(observer))
}

pub fn observe_object(object: &IsoObjectRef) {
    let observer = OBJECT_OBSERVER.with(Cell::get);
    if let Some(observer) = observer { observer(object); }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectBindingError {
    AlreadyBound,
    BorrowedComponent(pz_entity::ComponentType),
    ExistingOwner(pz_entity::ComponentType),
}

/// Retains the actual failed receiver and completed owner-binding prefix.
/// A caller must retire it after its enclosing world/sprite/entity guards.
#[derive(Debug)]
#[must_use]
pub struct ObjectBindingFailure {
    pub error: ObjectBindingError,
    pub object: IsoObjectRef,
}

#[derive(Debug)]
pub struct ObjectSlot {
    object: IsoObjectRef,
}
impl ObjectSlot {
    pub fn new(object: IsoObject) -> Self {
        Self::try_new(object).unwrap_or_else(|failure| {
            panic!("object allocation requires unowned, unborrowed components: {:?}", failure.error)
        })
    }

    /// Bind data assembled by a constructor/archive to its actual allocation.
    /// This is not native add/remove/connect/manager lifecycle; those phases
    /// require the guard-free host coordinator at their real call sites.
    pub fn try_new(mut object: IsoObject) -> Result<Self, ObjectBindingFailure> {
        if object.components.is_bound() {
            return Err(ObjectBindingFailure { error: ObjectBindingError::AlreadyBound, object: Rc::new(RefCell::new(object)) });
        }
        object.components.mark_bound();
        let object = Rc::new(RefCell::new(object));
        // Register every actual owner/component before introducing cycles.
        observe_object(&object);
        let components: Vec<_> = object.borrow().components.entry_slots().filter_map(|entry| match entry {
            pz_entity::RuntimeEntryView::Live { kind, component } => Some((kind, Rc::clone(component))),
            pz_entity::RuntimeEntryView::Encoded(_) => None,
        }).collect();
        let owner = pz_entity::EntityOwner::from_ref(&object);
        for (kind, component) in &components {
            let mut body = match component.try_borrow_mut() {
                Ok(body) => body,
                Err(_) => return Err(ObjectBindingFailure { error: ObjectBindingError::BorrowedComponent(*kind), object }),
            };
            if body.owner_slot().is_some() {
                drop(body);
                return Err(ObjectBindingFailure { error: ObjectBindingError::ExistingOwner(*kind), object });
            }
            let former = body.replace_owner(Some(&owner));
            debug_assert!(former.is_none());
        }
        Ok(Self { object })
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
