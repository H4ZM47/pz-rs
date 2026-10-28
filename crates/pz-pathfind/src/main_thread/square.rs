//! The edge-object getters of `PathFindBehavior2` as [`ObjectRef`]s, on top
//! of `pz-world`'s square queries.

use pz_world::object_slot::IsoObjectRef;
use pz_world::{GetSquare, SquareOwnerQueries, SquareRef};

use super::MainThreadMap;
use crate::mover::ObjectRef;

fn obj_ref((sq, o): (SquareRef, IsoObjectRef)) -> ObjectRef {
    let sq = sq.borrow();
    ObjectRef {
        x: sq.x,
        y: sq.y,
        z: sq.z,
        id: o.borrow().id,
    }
}

impl<'w, 'a: 'w, G: GetSquare> MainThreadMap<'w, 'a, G> {
    /// The object an [`ObjectRef`] names.
    pub fn object(&self, r: ObjectRef) -> Option<IsoObjectRef> {
        let square = self.square(r.x, r.y, r.z)?;
        let object = square
            .borrow()
            .objects
            .iter()
            .find(|o| o.borrow().id == r.id)
            .map(|o| o.ref_handle());
        object
    }

    /// `BarricadeAble.isBarricaded()` of the object.
    pub fn is_barricaded(&self, r: ObjectRef) -> bool {
        match (self.square(r.x, r.y, r.z), self.object(r)) {
            (Some(sq), Some(o)) => self.hooks.object_barricaded(&sq.borrow(), &o.borrow()),
            _ => false,
        }
    }

    /// `IsoGridSquare.getDoorTo(next)`.
    pub fn door_to(&self, this: &SquareRef, next: &SquareRef) -> Option<ObjectRef> {
        this.get_door_to(Some(next), self.squares(), &self.hooks)
            .map(obj_ref)
    }

    /// `IsoGridSquare.getWindowTo(next)`, with the square the window is on.
    pub fn window_to(&self, this: &SquareRef, next: &SquareRef) -> Option<ObjectRef> {
        this.get_edge_element_to(Some(next), self.squares(), &|s: &SquareRef, f| {
            s.borrow().get_window_facing(f).map(|o| (s.clone(), o))
        })
        .map(obj_ref)
    }

    /// `IsoGridSquare.getWindowThumpableTo(next)`.
    pub fn window_thumpable_to(&self, this: &SquareRef, next: &SquareRef) -> Option<ObjectRef> {
        this.get_window_thumpable_to(Some(next), self.sprites(), self.squares(), &self.hooks)
            .map(obj_ref)
    }

    /// `IsoGridSquare.getWindowFrameTo(next)`.
    pub fn window_frame_to(&self, this: &SquareRef, next: &SquareRef) -> Option<ObjectRef> {
        this.get_window_frame_to(Some(next), self.squares())
            .map(obj_ref)
    }
}
