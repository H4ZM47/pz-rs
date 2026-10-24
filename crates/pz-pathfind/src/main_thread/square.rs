//! `IsoGridSquare` queries the main-thread collision code needs that
//! `pz-world` does not have yet, on `pz-world` squares.

use pz_world::chunk::IsoGridSquare;
use pz_world::objects::{IsoObject, ObjectKind};
use pz_world::sprite::{IsoFlagType, PropertyContainer};
use pz_world::{EdgeFacing, GetSquare, GridSquareEdge, IsoDirections};

use super::los_util::TestResults;
use super::MainThreadMap;
use crate::mover::ObjectRef;

/// `IsoObject.VisionResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisionResult {
    NoEffect,
    Blocked,
    Unblocked,
}

fn same(a: &IsoGridSquare, b: &IsoGridSquare) -> bool {
    (a.x, a.y, a.z) == (b.x, b.y, b.z)
}

/// `square.getSpecialObjects()`, skipping identities no longer in
/// `objects`.
pub fn specials(sq: &IsoGridSquare) -> impl Iterator<Item = &IsoObject> {
    sq.special_objects
        .iter()
        .filter_map(move |&id| sq.objects.iter().find(|o| o.id == id))
}

impl<'w, 'a: 'w, G: GetSquare> MainThreadMap<'w, 'a, G> {
    fn props(&self, o: &IsoObject) -> Option<&'w PropertyContainer> {
        o.sprite.map(|s| &self.bits.sprites.sprites[s].properties)
    }

    fn obj_has(&self, o: &IsoObject, f: IsoFlagType) -> bool {
        self.props(o).is_some_and(|p| p.has(f))
    }

    /// `IsoGridSquare.isSolid()`.
    pub fn is_solid(&self, sq: &IsoGridSquare) -> bool {
        sq.has(self.bits.t.solid)
    }

    /// `IsoGridSquare.isSolidTrans()`.
    pub fn is_solid_trans(&self, sq: &IsoGridSquare) -> bool {
        sq.has(self.bits.t.solidtrans)
    }

    /// `square.solidFloorCached ? square.solidFloor :
    /// square.TreatAsSolidFloor()`.
    pub fn treat_as_solid_floor(&self, sq: &IsoGridSquare) -> bool {
        sq.treat_as_solid_floor(self.bits.sprites)
    }

    /// `IsoGridSquare.HasStairs()`.
    pub fn has_stairs(&self, sq: &IsoGridSquare) -> bool {
        sq.has_stairs(self.bits.sprites)
    }

    /// `IsoGridSquare.HasStairsNorth()`.
    pub fn has_stairs_north(&self, sq: &IsoGridSquare) -> bool {
        sq.has_stairs_north(self.bits.sprites)
    }

    /// `IsoGridSquare.HasStairsWest()`.
    pub fn has_stairs_west(&self, sq: &IsoGridSquare) -> bool {
        sq.has_stairs_west(self.bits.sprites)
    }

    /// `IsoGridSquare.HasElevatedFloor()`.
    pub fn has_elevated_floor(&self, sq: &IsoGridSquare) -> bool {
        sq.has_elevated_floor(self.bits.sprites)
    }

    /// `IsoGridSquare.getSlopedSurfaceHeightMin()`.
    pub fn sloped_surface_height_min(&self, sq: &IsoGridSquare) -> f32 {
        sq.surface.sloped_surface_height_min as f32 / 100.0
    }

    /// `IsoGridSquare.getSlopedSurfaceHeightMax()`.
    pub fn sloped_surface_height_max(&self, sq: &IsoGridSquare) -> f32 {
        sq.surface.sloped_surface_height_max as f32 / 100.0
    }

    /// `IsoGridSquare.getSlopedSurfaceHeight(dx, dy)`: the surface height
    /// (0..1) at a point of the square, 0 without a slope.
    pub fn sloped_surface_height_at(&self, sq: &IsoGridSquare, dx: f32, dy: f32) -> f32 {
        let Some(dir) = sq.sloped_surface_direction else {
            return 0.0;
        };
        let dx = dx.clamp(0.0, 1.0);
        let dy = dy.clamp(0.0, 1.0);
        let min = sq.surface.sloped_surface_height_min as f32;
        let max = sq.surface.sloped_surface_height_max as f32;
        let lerp = |t: f32| min + (max - min) * t;
        let z = match dir {
            IsoDirections::N => lerp(1.0 - dy),
            IsoDirections::S => lerp(dy),
            IsoDirections::W => lerp(1.0 - dx),
            IsoDirections::E => lerp(dx),
            _ => -1.0,
        };
        if z < 0.0 {
            0.0
        } else {
            z / 100.0
        }
    }

    /// `IsoGridSquare.hasIdenticalSlopedSurface(other)`.
    pub fn has_identical_sloped_surface(&self, a: &IsoGridSquare, b: &IsoGridSquare) -> bool {
        a.sloped_surface_direction == b.sloped_surface_direction
            && self.sloped_surface_height_min(a) == self.sloped_surface_height_min(b)
            && self.sloped_surface_height_max(a) == self.sloped_surface_height_max(b)
    }

    /// `IsoGridSquare.isSlopedSurfaceEdgeBlocked(edge)`.
    pub fn is_sloped_surface_edge_blocked(&self, sq: &IsoGridSquare, edge: IsoDirections) -> bool {
        sq.is_sloped_surface_edge_blocked(edge, self.bits.squares)
    }

    /// `IsoGridSquare.getStairsDirection()`.
    pub fn stairs_direction(&self, sq: &IsoGridSquare) -> Option<IsoDirections> {
        if self.has_stairs_north(sq) {
            Some(IsoDirections::N)
        } else if self.has_stairs_west(sq) {
            Some(IsoDirections::W)
        } else {
            None
        }
    }

    /// `IsoGridSquare.getStairsHeightMax()`. As in the original, bottom
    /// stairs report 0 (the middle-stairs test is repeated where the bottom
    /// one would be).
    pub fn stairs_height_max(&self, sq: &IsoGridSquare) -> f32 {
        let t = &self.bits.t;
        if sq.has_type(t.stairsTN) || sq.has_type(t.stairsTW) {
            1.0
        } else if sq.has_type(t.stairsMN) || sq.has_type(t.stairsMW) {
            0.66
        } else {
            0.0
        }
    }

    /// `IsoGridSquare.getStairsHeightMin()`.
    pub fn stairs_height_min(&self, sq: &IsoGridSquare) -> f32 {
        let t = &self.bits.t;
        if sq.has_type(t.stairsTN) || sq.has_type(t.stairsTW) {
            0.66
        } else if sq.has_type(t.stairsMN) || sq.has_type(t.stairsMW) {
            0.33
        } else {
            0.0
        }
    }

    /// `IsoGridSquare.getStairsHeight(edge)`.
    pub fn stairs_height(&self, sq: &IsoGridSquare, edge: IsoDirections) -> f32 {
        let Some(dir) = self.stairs_direction(sq) else {
            return 0.0;
        };
        if dir == edge {
            self.stairs_height_max(sq)
        } else if dir.rot180() == edge {
            self.stairs_height_min(sq)
        } else {
            -1.0
        }
    }

    /// `IsoGridSquare.isStairsEdgeBlocked(edge)`.
    pub fn is_stairs_edge_blocked(&self, sq: &IsoGridSquare, edge: IsoDirections) -> bool {
        if self.stairs_direction(sq).is_none() {
            return false;
        }
        let Some(sq2) = sq.get_adjacent_square(edge, self.bits.squares) else {
            return true;
        };
        self.stairs_height(sq, edge) != self.stairs_height(sq2, edge.rot180())
    }

    // ----- edge elements -----

    /// `IsoObject.getHoppableDirection()`.
    fn hoppable_direction(&self, o: &IsoObject) -> Option<EdgeFacing> {
        if self.obj_has(o, self.bits.t.HoppableN) {
            Some(EdgeFacing::NorthSouth)
        } else if self.obj_has(o, self.bits.t.HoppableW) {
            Some(EdgeFacing::EastWest)
        } else {
            None
        }
    }

    /// `IsoObject.getWindowFrameDirection()`.
    fn window_frame_direction(&self, o: &IsoObject) -> Option<EdgeFacing> {
        if self.obj_has(o, self.bits.t.WindowN) {
            Some(EdgeFacing::NorthSouth)
        } else if self.obj_has(o, self.bits.t.WindowW) {
            Some(EdgeFacing::EastWest)
        } else {
            None
        }
    }

    /// `IsoObject.isWindow()`: the sprite has `WindowN` or `WindowW`.
    pub fn is_window_object(&self, o: &IsoObject) -> bool {
        self.obj_has(o, self.bits.t.WindowN) || self.obj_has(o, self.bits.t.WindowW)
    }

    /// `IsoThumpable.isHoppable()`.
    fn thumpable_hoppable(&self, o: &IsoObject) -> bool {
        let ObjectKind::Thumpable(th) = &o.kind else {
            return false;
        };
        let hop = |p: Option<&PropertyContainer>| {
            p.is_some_and(|p| p.has(self.bits.t.HoppableN) || p.has(self.bits.t.HoppableW))
        };
        if th.is_door && !th.open {
            if let Some(cs) = th.closed_sprite {
                return hop(Some(&self.bits.sprites.sprites[cs].properties));
            }
        }
        hop(self.props(o)) || th.is_hoppable
    }

    /// `getWindow(facing)`: the first `IsoWindow` special object on that
    /// edge.
    fn has_window_facing(&self, sq: &IsoGridSquare, f: EdgeFacing) -> bool {
        specials(sq)
            .any(|o| matches!(&o.kind, ObjectKind::Window(w) if EdgeFacing::of_north(w.north) == f))
    }

    /// `getThumpableWindow(facing)`.
    fn has_thumpable_window_facing(&self, sq: &IsoGridSquare, f: EdgeFacing) -> bool {
        specials(sq).any(|o| {
            matches!(&o.kind, ObjectKind::Thumpable(th) if EdgeFacing::of_north(th.north) == f)
                && self.is_window_object(o)
        })
    }

    /// `getWindowFrame(facing)`: an `IsoWindowFrame` in `objects` on that
    /// edge.
    fn has_window_frame_facing(&self, sq: &IsoGridSquare, f: EdgeFacing) -> bool {
        sq.objects.iter().any(
            |o| matches!(&o.kind, ObjectKind::WindowFrame { north } if EdgeFacing::of_north(*north) == f),
        )
    }

    /// `getHoppableOrWindowFrame(facing)`: any object that is hoppable or a
    /// window frame (by its sprite) on that edge.
    fn has_hoppable_or_window_frame_facing(&self, sq: &IsoGridSquare, f: EdgeFacing) -> bool {
        sq.objects.iter().any(|o| {
            self.hoppable_direction(o) == Some(f) || self.window_frame_direction(o) == Some(f)
        })
    }

    /// `getHoppableThumpable(facing)`.
    fn has_hoppable_thumpable_facing(&self, sq: &IsoGridSquare, f: EdgeFacing) -> bool {
        specials(sq).any(|o| {
            matches!(&o.kind, ObjectKind::Thumpable(th) if EdgeFacing::of_north(th.north) == f)
                && self.thumpable_hoppable(o)
        })
    }

    /// `isAdjacentToEdgeElement(getter)` for a facing test: the north, west,
    /// east and south edges.
    fn adjacent_to(
        &self,
        sq: &IsoGridSquare,
        test: &dyn Fn(&IsoGridSquare, EdgeFacing) -> bool,
    ) -> bool {
        let g = self.bits.squares;
        [
            GridSquareEdge::North,
            GridSquareEdge::West,
            GridSquareEdge::East,
            GridSquareEdge::South,
        ]
        .into_iter()
        .any(|e| sq.is_edge_element(e, g, test))
    }

    /// `IsoGridSquare.isAdjacentToWindow()`: a window, window thumpable or
    /// window frame on one of the square's four edges.
    pub fn is_adjacent_to_window(&self, sq: &IsoGridSquare) -> bool {
        self.adjacent_to(sq, &|s, f| self.has_window_facing(s, f))
            || self.adjacent_to(sq, &|s, f| self.has_thumpable_window_facing(s, f))
            || self.adjacent_to(sq, &|s, f| self.has_window_frame_facing(s, f))
    }

    /// `IsoGridSquare.isAdjacentToHoppable()`.
    pub fn is_adjacent_to_hoppable(&self, sq: &IsoGridSquare) -> bool {
        self.adjacent_to(sq, &|s, f| self.has_hoppable_or_window_frame_facing(s, f))
            || self.adjacent_to(sq, &|s, f| self.has_hoppable_thumpable_facing(s, f))
    }

    /// `IsoGridSquare.hasBlockedDoor(facing)`: an `IsoDoor` (`isBlocked`) or
    /// door `IsoThumpable` (`isBlockedDoor`) on that edge that is closed or
    /// barricaded.
    pub fn has_blocked_door(&self, sq: &IsoGridSquare, f: EdgeFacing) -> bool {
        let hooks = self.bits.hooks;
        sq.objects.iter().any(|o| match &o.kind {
            ObjectKind::Door(d) => {
                EdgeFacing::of_north(d.north) == f && (!d.open || hooks.is_barricaded(sq, o))
            }
            ObjectKind::Thumpable(th) => {
                EdgeFacing::of_north(th.north) == f
                    && th.is_door
                    && (!th.open || hooks.is_barricaded(sq, o))
            }
            _ => false,
        })
    }

    // ----- edge objects between two squares (PathFindBehavior2) -----

    fn obj_ref(sq: &IsoGridSquare, o: &IsoObject) -> ObjectRef {
        ObjectRef {
            x: sq.x,
            y: sq.y,
            z: sq.z,
            id: o.id,
        }
    }

    /// The object an [`ObjectRef`] names.
    pub fn object(&self, r: ObjectRef) -> Option<&'a IsoObject> {
        self.square(r.x, r.y, r.z)?
            .objects
            .iter()
            .find(|o| o.id == r.id)
    }

    /// `getEdgeElementTo(next, getter)` with a per-square getter.
    fn edge_to(
        &self,
        this: &'a IsoGridSquare,
        next: &'a IsoGridSquare,
        get: &dyn Fn(&'a IsoGridSquare, EdgeFacing) -> Option<ObjectRef>,
    ) -> Option<ObjectRef> {
        this.get_edge_element_to(Some(next), self.bits.squares, get)
    }

    /// `IsoGridSquare.getDoorTo(next)`: an `IsoDoor` or door `IsoThumpable`
    /// special object on the edge between the squares.
    pub fn door_to(&self, this: &'a IsoGridSquare, next: &'a IsoGridSquare) -> Option<ObjectRef> {
        self.edge_to(this, next, &|s, f| {
            specials(s)
                .find(|o| match &o.kind {
                    ObjectKind::Door(d) => EdgeFacing::of_north(d.north) == f,
                    ObjectKind::Thumpable(t) => t.is_door && EdgeFacing::of_north(t.north) == f,
                    _ => false,
                })
                .map(|o| Self::obj_ref(s, o))
        })
    }

    /// `IsoGridSquare.getWindowTo(next)`.
    pub fn window_to(&self, this: &'a IsoGridSquare, next: &'a IsoGridSquare) -> Option<ObjectRef> {
        self.edge_to(this, next, &|s, f| {
            specials(s)
                .find(|o| matches!(&o.kind, ObjectKind::Window(w) if EdgeFacing::of_north(w.north) == f))
                .map(|o| Self::obj_ref(s, o))
        })
    }

    /// `IsoGridSquare.getWindowThumpableTo(next)`.
    pub fn window_thumpable_to(
        &self,
        this: &'a IsoGridSquare,
        next: &'a IsoGridSquare,
    ) -> Option<ObjectRef> {
        self.edge_to(this, next, &|s, f| {
            specials(s)
                .find(|o| {
                    matches!(&o.kind, ObjectKind::Thumpable(t) if EdgeFacing::of_north(t.north) == f)
                        && self.is_window_object(o)
                })
                .map(|o| Self::obj_ref(s, o))
        })
    }

    /// `IsoGridSquare.getWindowFrameTo(next)`.
    pub fn window_frame_to(
        &self,
        this: &'a IsoGridSquare,
        next: &'a IsoGridSquare,
    ) -> Option<ObjectRef> {
        self.edge_to(this, next, &|s, f| {
            s.objects
                .iter()
                .find(|o| matches!(&o.kind, ObjectKind::WindowFrame { north } if EdgeFacing::of_north(*north) == f))
                .map(|o| Self::obj_ref(s, o))
        })
    }

    /// `BarricadeAble.isBarricaded()` of the object.
    pub fn is_barricaded(&self, r: ObjectRef) -> bool {
        match (self.square(r.x, r.y, r.z), self.object(r)) {
            (Some(sq), Some(o)) => self.bits.hooks.is_barricaded(sq, o),
            _ => false,
        }
    }

    /// `hasBlockedWindow(facing)`: an `IsoWindow` on that edge that is
    /// closed and intact, or barricaded (`IsoWindow.isBlocked`).
    pub fn has_blocked_window(&self, sq: &IsoGridSquare, f: EdgeFacing) -> bool {
        sq.objects.iter().any(|o| match &o.kind {
            ObjectKind::Window(w) => {
                EdgeFacing::of_north(w.north) == f
                    && (!w.destroyed && !w.open || self.bits.hooks.is_barricaded(sq, o))
            }
            _ => false,
        })
    }

    /// `getWallExcludingList(bNorth, ignoreBlockingSprites)` with the
    /// (vanilla: empty) exclusion list.
    fn has_wall_excluding(&self, sq: &IsoGridSquare, north: bool) -> bool {
        let t = &self.bits.t;
        let tf = &self.bits.sprites.tables.flag;
        sq.objects.iter().any(|o| {
            let Some(s) = o.sprite else { return false };
            let spr = &self.bits.sprites.sprites[s];
            let p = &spr.properties;
            if p.has(tf.WallSE) || north && p.has(t.DoorWallN) || !north && p.has(t.DoorWallW) {
                return false;
            }
            north && (spr.cut_n || p.has(t.WallN)) || !north && (spr.cut_w || p.has(t.WallW))
        })
    }

    /// `IsoGridSquare.canReachTo(other)`.
    pub fn can_reach_to(&self, this: &IsoGridSquare, other: &IsoGridSquare) -> bool {
        if same(this, other) {
            return true;
        }
        let g = self.bits.squares;
        if (other.x - this.x).abs() > 1
            || (other.y - this.y).abs() > 1
            || other.z != this.z
            || this.is_edge_element_to(Some(other), g, &|s, f| self.has_blocked_window(s, f))
            || this.is_edge_element_to(Some(other), g, &|s, f| self.has_blocked_door(s, f))
        {
            return false;
        }
        if other.y < this.y && self.has_wall_excluding(this, true)
            || this.y < other.y && self.has_wall_excluding(other, true)
            || other.x < this.x && self.has_wall_excluding(this, false)
            || this.x < other.x && self.has_wall_excluding(other, false)
        {
            return false;
        }
        let t = &self.bits.t;
        if this.x > other.x && this.has_type(t.stairsTW)
            || this.y > other.y && this.has_type(t.stairsTN)
        {
            return false;
        }
        !this.is_wall_to(Some(other), self.bits.sprites, g)
    }

    // ----- vision -----

    /// `IsoObject.TestVision(from, to)` of an object on square `owner`.
    pub fn test_vision(
        &self,
        owner: &IsoGridSquare,
        o: &IsoObject,
        from: &IsoGridSquare,
        to: &IsoGridSquare,
    ) -> VisionResult {
        use VisionResult::*;
        let sprites = self.bits.sprites;
        let sprite_has = |name: &str| {
            o.sprite
                .is_some_and(|s| sprites.has_prop(&sprites.sprites[s], name))
        };
        // The edge test shared by doors, thumpables and windows.
        let edge = |north: bool| -> Option<bool> {
            if same(from, owner) {
                if north && to.y < from.y || !north && to.x < from.x {
                    return Some(true);
                }
            } else if north && to.y > from.y || !north && to.x > from.x {
                return Some(true);
            }
            None
        };
        match &o.kind {
            ObjectKind::Door(d) => {
                let mut door_trans = sprite_has("doorTrans");
                if sprite_has("GarageDoor") && d.open {
                    door_trans = true;
                }
                if d.open {
                    door_trans = true;
                } else if d.has_curtain && !d.curtain_open {
                    door_trans = false;
                }
                let north = if d.open { !d.north } else { d.north };
                if to.z != from.z {
                    return NoEffect;
                }
                match edge(north) {
                    Some(_) if door_trans => Unblocked,
                    Some(_) => Blocked,
                    None => NoEffect,
                }
            }
            ObjectKind::Thumpable(th) => {
                if th.can_pass_through {
                    return NoEffect;
                }
                let north = if th.open { !th.north } else { th.north };
                if to.z != from.z {
                    return NoEffect;
                }
                let door_trans = sprite_has("doorTrans");
                match edge(north) {
                    Some(_) if door_trans || self.is_window_object(o) => Unblocked,
                    Some(_) => Blocked,
                    None => NoEffect,
                }
            }
            ObjectKind::Window(w) => {
                if to.z != from.z {
                    return NoEffect;
                }
                match edge(w.north) {
                    Some(_) => Unblocked,
                    None => NoEffect,
                }
            }
            ObjectKind::Curtain(c) => {
                if to.z != from.z {
                    return NoEffect;
                }
                let ty = o.sprite.map(|s| sprites.sprites[s].tile_type);
                let tt = &sprites.tables.ty;
                let near = same(from, owner)
                    && (ty == Some(tt.curtainW) || ty == Some(tt.curtainN))
                    || !same(from, owner) && (ty == Some(tt.curtainE) || ty == Some(tt.curtainS));
                let blocked = if near {
                    c.north && to.y < from.y && !c.open || !c.north && to.x < from.x && !c.open
                } else {
                    c.north && to.y > from.y && !c.open || !c.north && to.x > from.x && !c.open
                };
                if blocked {
                    Blocked
                } else {
                    NoEffect
                }
            }
            ObjectKind::Barricade(b) => self.barricade_vision(owner, b, from, to),
            ObjectKind::Fire(_) => NoEffect,
            _ => Unblocked,
        }
    }

    /// `IsoBarricade.TestVision(from, to)`.
    fn barricade_vision(
        &self,
        owner: &IsoGridSquare,
        b: &pz_world::objects::IsoBarricade,
        from: &IsoGridSquare,
        to: &IsoGridSquare,
    ) -> VisionResult {
        let planks = b.plank_health.iter().filter(|&&h| h > 0).count();
        if b.metal_health <= 0 && planks <= 2 {
            return VisionResult::NoEffect;
        }
        if same(from, owner) {
            let dir = IsoDirections::ALL.get(b.dir as usize).copied();
            let blocked = dir == Some(IsoDirections::N) && to.y < from.y
                || dir == Some(IsoDirections::S) && to.y > from.y
                || dir == Some(IsoDirections::W) && to.x < from.x
                || dir == Some(IsoDirections::E) && to.x > from.x;
            if blocked {
                return VisionResult::Blocked;
            }
        } else if same(to, owner) {
            return self.barricade_vision(owner, b, to, from);
        }
        VisionResult::NoEffect
    }

    /// One special-object pass of `testVisionAdjacent` (`owner`'s special
    /// objects, from `this` to `sq3`). `Err` is an early return.
    fn vision_specials(
        &self,
        owner: &IsoGridSquare,
        this: &IsoGridSquare,
        sq3: &IsoGridSquare,
        ignore_doors: bool,
        ret: &mut TestResults,
    ) -> Result<(), TestResults> {
        use VisionResult::*;
        for obj in specials(owner) {
            let vis = self.test_vision(owner, obj, this, sq3);
            if vis == NoEffect {
                continue;
            }
            let th_door = matches!(&obj.kind, ObjectKind::Thumpable(t) if t.is_door);
            match (&obj.kind, vis) {
                (ObjectKind::Door(d), Unblocked) => {
                    *ret = if d.open {
                        TestResults::ClearThroughOpenDoor
                    } else {
                        TestResults::ClearThroughClosedDoor
                    };
                }
                (ObjectKind::Thumpable(_), Unblocked) if th_door => {
                    *ret = TestResults::ClearThroughOpenDoor;
                }
                (ObjectKind::Window(_), Unblocked) => *ret = TestResults::ClearThroughWindow,
                (ObjectKind::Door(_), Blocked) if !ignore_doors => {
                    return Err(TestResults::Blocked)
                }
                (ObjectKind::Thumpable(_), Blocked) if th_door && !ignore_doors => {
                    return Err(TestResults::Blocked)
                }
                (ObjectKind::Thumpable(_), Blocked) if self.is_window_object(obj) => {
                    return Err(TestResults::Blocked)
                }
                (ObjectKind::Curtain(_), Blocked)
                | (ObjectKind::Window(_), Blocked)
                | (ObjectKind::Barricade(_), Blocked) => return Err(TestResults::Blocked),
                _ => {}
            }
        }
        Ok(())
    }

    /// `IsoGridSquare.testVisionAdjacent(x, y, z, specialDiag,
    /// bIgnoreDoors)`.
    pub fn test_vision_adjacent(
        &self,
        this: &IsoGridSquare,
        x: i32,
        y: i32,
        z: i32,
        special_diag: bool,
        ignore_doors: bool,
    ) -> TestResults {
        if !(-1..=1).contains(&x) || !(-1..=1).contains(&y) || !(-1..=1).contains(&z) {
            return TestResults::Blocked;
        }
        if z == 1 && (x != 0 || y != 0) && self.has_elevated_floor(this) {
            if let Some(sq) = self.square(this.x, this.y, this.z + z) {
                return self.test_vision_adjacent(sq, x, y, 0, special_diag, ignore_doors);
            }
        }
        if z == -1 && (x != 0 || y != 0) {
            if let Some(sq) = self.square(this.x + x, this.y + y, this.z + z) {
                if self.has_elevated_floor(sq) {
                    return self.test_vision_adjacent(this, x, y, 0, special_diag, ignore_doors);
                }
            }
        }
        if x != 0 && y != 0 && special_diag {
            let mut test = self.do_diagonal_check(this, x, y, z, ignore_doors);
            if !self.is_server && test != TestResults::Blocked {
                if let Some(sq2) = self.square(this.x + x, this.y + y, this.z + z) {
                    test = self.do_diagonal_check(sq2, -x, -y, -z, ignore_doors);
                }
            }
            return test;
        }
        let sq3 = self.square(this.x + x, this.y + y, this.z + z);
        let mut ret = TestResults::Clear;
        match sq3 {
            Some(sq3) if sq3.z == this.z => {
                if let Err(r) = self.vision_specials(this, this, sq3, ignore_doors, &mut ret) {
                    return r;
                }
                if let Err(r) = self.vision_specials(sq3, this, sq3, ignore_doors, &mut ret) {
                    return r;
                }
            }
            Some(sq3) => {
                let ext = self.bits.sprites.tables.flag.exterior;
                if z > 0 && (this.z != -1 || sq3.z != 0) && sq3.has(ext) && !this.has(ext) {
                    ret = TestResults::Blocked;
                }
            }
            None => {}
        }
        if this.get_vision_matrix(x, y, z) {
            TestResults::Blocked
        } else {
            ret
        }
    }

    /// `IsoGridSquare.DoDiagnalCheck(x, y, z, bIgnoreDoors)`.
    fn do_diagonal_check(
        &self,
        this: &IsoGridSquare,
        x: i32,
        y: i32,
        z: i32,
        ignore_doors: bool,
    ) -> TestResults {
        let res = self.test_vision_adjacent(this, x, 0, z, false, ignore_doors);
        if res == TestResults::Blocked {
            return TestResults::Blocked;
        }
        let res2 = self.test_vision_adjacent(this, 0, y, z, false, ignore_doors);
        if res2 == TestResults::Blocked {
            return TestResults::Blocked;
        }
        if res == TestResults::ClearThroughWindow || res2 == TestResults::ClearThroughWindow {
            return TestResults::ClearThroughWindow;
        }
        self.test_vision_adjacent(this, x, y, z, false, ignore_doors)
    }
}
