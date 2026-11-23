//! What the interact key finds on the squares around a player
//! (`IsoPlayer.getContextDoorOrWindowOrWindowFrame`,
//! `doContextDoorOrWindowOrWindowFrame` and the helpers it calls), and the
//! square checks of `doContextHutch`, `doContextButcherHook`,
//! `doContextAnimalInteraction`, `doContextRestOnFurniture` and
//! `doContextHopOverFence`.
//!
//! The order of the branches is the character's
//! (`pz_characters::character::player::context`); this module answers what
//! each branch sees of the cell.
//!
//! Not ported, and failing with `RuntimeHostError::Unsupported` where the
//! original would have looked: the curtains of doors, windows and frames
//! (shift + interact), the animal on a square, and grappled targets.

use pz_characters::character::player::context::{ContextQuery, ContextSpecial};
use pz_characters::character::player::{ContextualAction, ContextualActionKind};
use pz_inventory::floor_drop::{HostResult, RuntimeHostError};
use pz_pathfind::mover::ObjectRef;
use pz_world::object_slot::IsoObjectRef;
use pz_world::objects::{IsoObject, ObjectKind};
use pz_world::{EdgeFacing, IsoDirections, IsoGridSquare, SquareRef};

use crate::{StatesWorldAdapter, Thumper};

fn unsupported<T>(operation: &'static str) -> HostResult<T> {
    Err(RuntimeHostError::Unsupported { operation, kind: None })
}

fn oref(sq: &IsoGridSquare, o: &IsoObject) -> ObjectRef {
    ObjectRef { x: sq.x, y: sq.y, z: sq.z, id: o.id }
}

fn action(kind: ContextualActionKind, dir: IsoDirections, obj: ObjectRef) -> ContextualAction {
    ContextualAction::new(kind, Some(dir), Some((obj.x, obj.y, obj.z)), Some(obj.id as i64))
}

impl StatesWorldAdapter<'_> {
    fn adjacent_of(&self, p: (i32, i32, i32), dir: IsoDirections) -> Option<SquareRef> {
        let (dx, dy) = dir.delta();
        self.cell.get_grid_square(p.0 + dx, p.1 + dy, p.2)
    }

    /// `IsoDoor.getSpriteEdge(ignoreOpen)` / `IsoThumpable.getSpriteEdge`:
    /// the edge the object is on (an open door stands on the edge its
    /// open sprite is attached to).
    fn sprite_edge(&self, o: &IsoObject, ignore_open: bool) -> Option<IsoDirections> {
        let edge = |north: bool| if north { IsoDirections::N } else { IsoDirections::W };
        let (north, open, door) = match &o.kind {
            ObjectKind::Door(d) => (d.north, d.open, true),
            ObjectKind::Thumpable(t) if t.is_door || self.env().is_window(o) => (t.north, t.open, false),
            _ => return None,
        };
        if !open || ignore_open {
            return Some(edge(north));
        }
        let env = self.env();
        if door && env.obj_has_prop(o, "GarageDoor") {
            return Some(edge(north));
        }
        if env.obj_has_flag(o, "attachedE") {
            return Some(IsoDirections::E);
        }
        if env.obj_has_flag(o, "attachedS") {
            return Some(IsoDirections::S);
        }
        Some(if north { IsoDirections::W } else { IsoDirections::N })
    }

    /// `IsoGridSquare.getOpenDoor(dir)`: an open door or thumpable in the
    /// special objects whose open edge is `dir`.
    fn get_open_door(&self, sq: &IsoGridSquare, dir: IsoDirections) -> Option<IsoObjectRef> {
        sq.get_special_objects().find(|o| {
            let o = o.borrow();
            let open = match &o.kind {
                ObjectKind::Door(d) => d.open,
                ObjectKind::Thumpable(t) => t.open,
                _ => false,
            };
            open && self.sprite_edge(&o, false) == Some(dir)
        })
    }

    /// `IsoGridSquare.getDoorOrWindowOrWindowFrame(dir, ignoreOpen)`, the
    /// last object first.
    fn get_door_or_window_or_frame(&self, sq: &IsoGridSquare, dir: IsoDirections, ignore_open: bool) -> Option<IsoObjectRef> {
        let edge = |north: bool| if north { IsoDirections::N } else { IsoDirections::W };
        sq.objects.iter().rev().find(|slot| {
            let o = slot.borrow();
            match &o.kind {
                ObjectKind::Door(_) | ObjectKind::Thumpable(_) => self.sprite_edge(&o, ignore_open) == Some(dir),
                ObjectKind::Window(w) => edge(w.north) == dir,
                ObjectKind::WindowFrame { north } => edge(*north) == dir,
                _ => false,
            }
        }).map(|slot| slot.ref_handle())
    }

    /// `getContextDoorOrWindowOrWindowFrame(dir)`.
    pub fn context_door_or_window_object_at(&self, q: &ContextQuery, dir: IsoDirections) -> Option<ObjectRef> {
        let cur_owner = self.at(q.square)?;
        let cur = cur_owner.borrow();
        let adj_owner = self.adjacent_of(q.square, dir);
        let hooks = self.env().hooks();
        let found = |sq: &IsoGridSquare, o: IsoObjectRef| Some(oref(sq, &o.borrow()));
        let blocked = |adj: &IsoGridSquare| self.env().is_blocked_to(&cur, Some(adj));
        use IsoDirections as D;
        match dir {
            D::N => {
                if let Some(o) = self.get_open_door(&cur, dir) {
                    return found(&cur, o);
                }
                if let Some(o) = self.get_door_or_window_or_frame(&cur, dir, true) {
                    return found(&cur, o);
                }
                if let Some(o) = cur.get_door_facing(EdgeFacing::NorthSouth, &hooks) {
                    return found(&cur, o);
                }
                let adj_owner = adj_owner?;
                let adj = adj_owner.borrow();
                if blocked(&adj) {
                    return None;
                }
                self.get_open_door(&adj, D::S).and_then(|o| found(&adj, o))
            }
            D::S => {
                if let Some(o) = self.get_open_door(&cur, dir) {
                    return found(&cur, o);
                }
                let adj_owner = adj_owner?;
                let adj = adj_owner.borrow();
                let b = blocked(&adj);
                if let Some(o) = self.get_door_or_window_or_frame(&adj, D::N, b) {
                    return found(&adj, o);
                }
                adj.get_door_facing(EdgeFacing::NorthSouth, &hooks).and_then(|o| found(&adj, o))
            }
            D::W => {
                if let Some(o) = self.get_open_door(&cur, dir) {
                    return found(&cur, o);
                }
                if let Some(o) = self.get_door_or_window_or_frame(&cur, dir, true) {
                    return found(&cur, o);
                }
                if let Some(o) = cur.get_door_facing(EdgeFacing::EastWest, &hooks) {
                    return found(&cur, o);
                }
                let adj_owner = adj_owner?;
                let adj = adj_owner.borrow();
                if blocked(&adj) {
                    return None;
                }
                self.get_open_door(&adj, D::E).and_then(|o| found(&adj, o))
            }
            D::E => {
                if let Some(o) = self.get_open_door(&cur, dir) {
                    return found(&cur, o);
                }
                let adj_owner = adj_owner?;
                let adj = adj_owner.borrow();
                let b = blocked(&adj);
                if let Some(o) = self.get_door_or_window_or_frame(&adj, D::W, b) {
                    return found(&adj, o);
                }
                adj.get_door_facing(EdgeFacing::EastWest, &hooks).and_then(|o| found(&adj, o))
            }
            _ => None,
        }
    }

    fn context_thumper(q: &ContextQuery) -> Thumper {
        Thumper {
            id: q.chr,
            x: q.x,
            y: q.y,
            z: q.square.2 as f32,
            current_square: Some(q.square),
            zombie: false,
            animal: false,
            crawling: false,
            strength: 0,
            cognition: 0,
        }
    }

    /// The `doContextThrowGrappledTargetOutWindow` test the window
    /// branches run first: with a grappled target it throws it out the
    /// window, which is not ported.
    fn context_grapple_guard(q: &ContextQuery) -> HostResult<()> {
        if q.grappling {
            return unsupported("IsoPlayer.doContextThrowGrappledTargetOutWindow");
        }
        Ok(())
    }

    /// `doContextDoorOrWindowOrWindowFrame(dir, obj)`.
    pub fn context_do_door_or_window_at(
        &self,
        q: &ContextQuery,
        dir: IsoDirections,
        obj: ObjectRef,
    ) -> HostResult<Vec<ContextualAction>> {
        use ContextualActionKind as K;
        let Some((sq_owner, o_owner)) = self.find(obj) else { return Ok(Vec::new()) };
        let sq = sq_owner.borrow();
        let o = o_owner.borrow();
        let env = self.env();
        let adj = self.adjacent_of(q.square, dir);
        // `IsoWindow.isTopOfSheetRopeHere(adj)` and `canClimbDownSheetRope`.
        let rope_top = adj.as_ref().is_some_and(|a| {
            let a = a.borrow();
            ["climbSheetTopN", "climbSheetTopS", "climbSheetTopW", "climbSheetTopE"].into_iter().any(|n| env.sq_has(&a, n))
                && env.can_climb_down_sheet_rope(Some(&a))
        });
        let thumper = Self::context_thumper(q);
        let invincible = env.sq_has(&sq, "makeWindowInvincible");
        let safe = |dir| self.is_safe_to_climb_over(q.square, dir);
        let one = |kind| Ok(vec![action(kind, dir, obj)]);
        let long_press = q.time_pressed_context >= 0.5;
        match &o.kind {
            ObjectKind::Door(_) | ObjectKind::Thumpable(_) if matches!(&o.kind, ObjectKind::Door(_)) || matches!(&o.kind, ObjectKind::Thumpable(t) if t.is_door) => {
                // doContextDoor / doContextThumpableDoor
                if let ObjectKind::Door(d) = &o.kind {
                    if q.shift && d.has_curtain && q.ticks_since_pressed_movement > 15.0 {
                        return unsupported("IsoDoor.isFacingSheet");
                    }
                }
                if long_press && o.is_hoppable(self.sprites) && !q.ignore_auto_vault {
                    if q.grappling {
                        return unsupported("IsoPlayer.doContextThrowGrappledTargetOverFence");
                    }
                    return one(K::ClimbOverFence);
                }
                one(K::ToggleDoor)
            }
            ObjectKind::Window(w) if !invincible => {
                // doContextWindow
                if q.shift {
                    // HasCurtains(): the curtain on either side of the window.
                    return unsupported("IsoWindow.HasCurtains");
                }
                let barricaded = env.hooks().object_barricaded(&sq, &o);
                if long_press {
                    if self.window_can_climb_through(&sq, &o, Some(&thumper)) {
                        Self::context_grapple_guard(q)?;
                        return one(K::ClimbThroughWindow);
                    }
                    if !w.perma_locked && !barricaded && !w.open && !w.destroyed {
                        return one(K::ToggleWindow);
                    }
                    return Ok(Vec::new());
                }
                if w.health > 0 && !w.destroyed {
                    if self.barricade_for_character(&sq, w.north, Some(&thumper)).is_none() {
                        Self::context_grapple_guard(q)?;
                        return one(K::ToggleWindow);
                    }
                } else if w.glass_removed && !barricaded {
                    Self::context_grapple_guard(q)?;
                    if safe(dir) || sq.have_sheet_rope || rope_top {
                        return one(K::ClimbThroughWindow);
                    }
                }
                Ok(Vec::new())
            }
            ObjectKind::Thumpable(_) if !invincible => {
                // doContextThumpableWindow
                if q.shift {
                    return unsupported("IsoThumpable.HasCurtains");
                }
                if !self.thumpable_can_climb_through(&sq, &o, Some(&thumper)) {
                    return Ok(Vec::new());
                }
                Self::context_grapple_guard(q)?;
                if long_press || safe(dir) || sq.have_sheet_rope || rope_top {
                    return one(K::ClimbThroughWindow);
                }
                Ok(Vec::new())
            }
            ObjectKind::WindowFrame { north } => {
                // doContextWindowFrame
                if q.shift {
                    return unsupported("IsoWindowFrame.getCurtain");
                }
                if (long_press || safe(dir) || rope_top) && self.frame_can_climb_through(&sq, *north, Some(&thumper)) {
                    Self::context_grapple_guard(q)?;
                    return one(K::ClimbThroughWindow);
                }
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    /// The square checks of the hutch, butcher hook, animal and furniture
    /// branches: the neighbour's first, then the player's own square.
    pub fn context_special_at(
        &self,
        q: &ContextQuery,
        dir: IsoDirections,
        which: ContextSpecial,
    ) -> HostResult<Option<ContextualAction>> {
        use ContextualActionKind as K;
        if q.grappling {
            return Ok(None);
        }
        let squares = [self.adjacent_of(q.square, dir), self.at(q.square)];
        match which {
            ContextSpecial::Hutch => {
                for sq in squares.iter().flatten() {
                    let sq = sq.borrow();
                    let hutch = sq.get_special_objects().find(|o| matches!(o.borrow().kind, ObjectKind::Hutch(_)));
                    if let Some(h) = hutch {
                        return Ok(Some(action(K::OpenHutch, dir, oref(&sq, &h.borrow()))));
                    }
                }
            }
            ContextSpecial::ButcherHook => {
                for sq in squares.iter().flatten() {
                    let sq = sq.borrow();
                    let hook = sq.objects.iter().find(|o| matches!(o.borrow().kind, ObjectKind::ButcherHook));
                    if let Some(h) = hook {
                        return Ok(Some(action(K::OpenButcherHook, dir, oref(&sq, &h.borrow()))));
                    }
                }
            }
            ContextSpecial::AnimalInteraction => {
                for sq in squares.iter().flatten() {
                    let sq = sq.borrow();
                    let here = (sq.x, sq.y, sq.z);
                    let animal = self
                        .engine
                        .moving_objects_on_square(here)
                        .into_iter()
                        .any(|id| self.engine.character(id).is_some_and(|c| c.animal));
                    // TODO(animals): `getAnimals()` leaves out an animal on a
                    // hook; the action needs the animal's object.
                    if animal {
                        return unsupported("IsoPlayer.doContextAnimalInteraction");
                    }
                }
            }
            ContextSpecial::RestOnFurniture => {
                for sq in squares.iter().flatten() {
                    let sq = sq.borrow();
                    let env = self.env();
                    let bed = sq.objects.iter().find(|o| env.obj_has_flag(&o.borrow(), "bed"));
                    if let Some(b) = bed {
                        return Ok(Some(action(K::RestOnFurniture, dir, oref(&sq, &b.borrow()))));
                    }
                }
            }
        }
        Ok(None)
    }

    /// The `HoppableN/W` square of `doContextHopOverFence(dir)` and its
    /// object (`getHoppable(north)`).
    pub fn context_hoppable_at(&self, q: &ContextQuery, dir: IsoDirections) -> Option<((i32, i32, i32), Option<i64>)> {
        let (owner, flag, north) = match dir {
            IsoDirections::N => (self.at(q.square)?, "HoppableN", true),
            IsoDirections::W => (self.at(q.square)?, "HoppableW", false),
            IsoDirections::S => (self.adjacent_of(q.square, dir)?, "HoppableN", true),
            IsoDirections::E => (self.adjacent_of(q.square, dir)?, "HoppableW", false),
            _ => return None,
        };
        let sq = owner.borrow();
        if !self.env().sq_has(&sq, flag) {
            return None;
        }
        let object = self.env().collide().get_hoppable(&sq, north).map(|id| id as i64);
        Some(((sq.x, sq.y, sq.z), object))
    }

    /// `current.has(IsoFlagType.climbSheetN/S/W/E)` for `dir`.
    pub fn context_sheet_rope_side_at(&self, q: &ContextQuery, dir: IsoDirections) -> bool {
        let flag = match dir {
            IsoDirections::N => "climbSheetN",
            IsoDirections::S => "climbSheetS",
            IsoDirections::W => "climbSheetW",
            IsoDirections::E => "climbSheetE",
            _ => return false,
        };
        self.at(q.square).is_some_and(|sq| self.env().sq_has(&sq.borrow(), flag))
    }
}
