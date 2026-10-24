//! What the main-thread queries need beyond `pz-world`'s square queries:
//! the [`SquareHooks`] that describes `pz-world`'s own thumpable, barricade,
//! fire and door-curtain objects to those queries, and the edge-object
//! getters as [`ObjectRef`]s.

use pz_world::chunk::IsoGridSquare;
use pz_world::objects::{IsoObject, IsoThumpable, ObjectKind};
use pz_world::sprite::PropertyContainer;
use pz_world::square::{SpecialObject, SquareHooks, ThumpableInfo};
use pz_world::{EdgeFacing, GetSquare, IsoDirections, IsoSpriteManager};

use super::MainThreadMap;
use crate::mover::ObjectRef;
use crate::square_bits::SquareBitsHooks;

/// [`SquareHooks`] over the objects `pz-world` loads: `IsoThumpable`,
/// `IsoBarricade` and `IsoFire` objects are reported from their
/// [`ObjectKind`], door curtains from `IsoDoor`, and barricades through
/// the [`SquareBitsHooks`] the pathfinder's square view uses.
pub struct WorldObjectHooks<'a> {
    pub sprites: &'a IsoSpriteManager,
    pub bits_hooks: &'a dyn SquareBitsHooks,
}

impl WorldObjectHooks<'_> {
    fn props(&self, s: Option<pz_world::sprite::SpriteIdx>) -> Option<&PropertyContainer> {
        s.map(|s| &self.sprites.sprites[s].properties)
    }

    /// `IsoThumpable.isHoppable()`: the closed sprite's hoppable flags for
    /// a closed door, else the sprite's, else the `isHoppable` field.
    fn thumpable_hoppable(&self, o: &IsoObject, th: &IsoThumpable) -> bool {
        let f = &self.sprites.tables.flag;
        let hop = |p: Option<&PropertyContainer>| {
            p.is_some_and(|p| p.has(f.HoppableN) || p.has(f.HoppableW))
        };
        if th.is_door && !th.open && th.closed_sprite.is_some() {
            return hop(self.props(th.closed_sprite));
        }
        hop(self.props(o.sprite)) || th.is_hoppable
    }
}

impl SquareHooks for WorldObjectHooks<'_> {
    fn is_barricaded(&self, sq: &IsoGridSquare, obj: &IsoObject) -> bool {
        self.bits_hooks.is_barricaded(sq, obj)
    }

    /// `IsoThumpable.isBlockedDoor(facing)` of a thumpable in `objects`.
    fn has_blocked_thumpable_door(&self, sq: &IsoGridSquare, facing: EdgeFacing) -> bool {
        sq.objects.iter().any(|o| match &o.kind {
            ObjectKind::Thumpable(th) => {
                EdgeFacing::of_north(th.north) == facing
                    && th.is_door
                    && (!th.open || self.bits_hooks.is_barricaded(sq, o))
            }
            _ => false,
        })
    }

    fn special_object(&self, _sq: &IsoGridSquare, obj: &IsoObject) -> Option<SpecialObject> {
        match &obj.kind {
            ObjectKind::Thumpable(th) => Some(SpecialObject::Thumpable(ThumpableInfo {
                north: th.north,
                open: th.open,
                is_door: th.is_door,
                is_hoppable: self.thumpable_hoppable(obj, th),
                can_pass_through: th.can_pass_through,
            })),
            ObjectKind::Barricade(b) => {
                let planks = b.plank_health.iter().filter(|&&h| h > 0).count();
                Some(SpecialObject::Barricade {
                    dir: IsoDirections::ALL[(b.dir as usize) & 7],
                    blocks_vision: b.metal_health > 0 || planks > 2,
                })
            }
            ObjectKind::Fire(_) => Some(SpecialObject::Fire),
            _ => None,
        }
    }

    fn door_curtain(&self, _sq: &IsoGridSquare, obj: &IsoObject) -> Option<bool> {
        match &obj.kind {
            ObjectKind::Door(d) if d.has_curtain => Some(d.curtain_open),
            _ => None,
        }
    }
}

fn obj_ref((sq, o): (&IsoGridSquare, &IsoObject)) -> ObjectRef {
    ObjectRef {
        x: sq.x,
        y: sq.y,
        z: sq.z,
        id: o.id,
    }
}

impl<'w, 'a: 'w, G: GetSquare> MainThreadMap<'w, 'a, G> {
    /// The object an [`ObjectRef`] names.
    pub fn object(&self, r: ObjectRef) -> Option<&'a IsoObject> {
        self.square(r.x, r.y, r.z)?
            .objects
            .iter()
            .find(|o| o.id == r.id)
    }

    /// `BarricadeAble.isBarricaded()` of the object.
    pub fn is_barricaded(&self, r: ObjectRef) -> bool {
        match (self.square(r.x, r.y, r.z), self.object(r)) {
            (Some(sq), Some(o)) => self.bits.hooks.is_barricaded(sq, o),
            _ => false,
        }
    }

    /// `IsoGridSquare.getDoorTo(next)`.
    pub fn door_to(&self, this: &'a IsoGridSquare, next: &'a IsoGridSquare) -> Option<ObjectRef> {
        this.get_door_to(Some(next), self.squares(), &self.hooks)
            .map(obj_ref)
    }

    /// `IsoGridSquare.getWindowTo(next)`, with the square the window is on.
    pub fn window_to(&self, this: &'a IsoGridSquare, next: &'a IsoGridSquare) -> Option<ObjectRef> {
        this.get_edge_element_to(Some(next), self.squares(), &|s: &'a IsoGridSquare, f| {
            s.get_window_facing(f).map(|o| (s, o))
        })
        .map(obj_ref)
    }

    /// `IsoGridSquare.getWindowThumpableTo(next)`.
    pub fn window_thumpable_to(
        &self,
        this: &'a IsoGridSquare,
        next: &'a IsoGridSquare,
    ) -> Option<ObjectRef> {
        this.get_window_thumpable_to(Some(next), self.sprites(), self.squares(), &self.hooks)
            .map(obj_ref)
    }

    /// `IsoGridSquare.getWindowFrameTo(next)`.
    pub fn window_frame_to(
        &self,
        this: &'a IsoGridSquare,
        next: &'a IsoGridSquare,
    ) -> Option<ObjectRef> {
        this.get_window_frame_to(Some(next), self.squares())
            .map(obj_ref)
    }
}
