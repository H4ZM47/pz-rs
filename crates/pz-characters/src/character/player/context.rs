//! `IsoPlayer.doContext()`: the interact key's contextual action (doors,
//! windows, fences, walls, sheet ropes, ...). The direction split, the order
//! of the branches of `doContextNSWE` and `doContextCorners` and the choice
//! are ported here; what the world offers on a square edge (the door,
//! window or frame, the hutch, the bed, the fence) is the world's
//! ([`StateWorld::context_door_or_window_object`] and its neighbours), and
//! performing the chosen action is [`StateWorld::context_toggle_door`] for a
//! door and [`PlayerEvent::ContextualAction`] for the rest.
//!
//! [`StateWorld::context_door_or_window_object`]: crate::character::state::StateWorld::context_door_or_window_object
//! [`StateWorld::context_toggle_door`]: crate::character::state::StateWorld::context_toggle_door

use pz_inventory::floor_drop::{HostResult, RuntimeHostError};
use pz_world::square::IsoDirections;

use super::{PlayerEvent, PlayerInput, PlayerUpdate};
use crate::character::state::StateCx;
use crate::character::GameCharacter;

/// `ContextualAction.Action`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextualActionKind {
    None,
    AnimalInteraction,
    ClimbOverFence,
    ClimbOverWall,
    ClimbSheetRope,
    ClimbThroughWindow,
    OpenButcherHook,
    OpenHutch,
    RestOnFurniture,
    ThrowGrappledTargetOutWindow,
    ThrowGrappledOverFence,
    ThrowGrappledIntoContainer,
    ToggleCurtain,
    ToggleDoor,
    ToggleWindow,
}

impl ContextualActionKind {
    /// The action's `priority`.
    pub fn priority(self) -> i32 {
        use ContextualActionKind::*;
        match self {
            None => 0,
            AnimalInteraction | OpenButcherHook | OpenHutch | RestOnFurniture => 90,
            ClimbOverFence | ClimbOverWall | ClimbSheetRope | ClimbThroughWindow | ThrowGrappledTargetOutWindow
            | ThrowGrappledOverFence | ThrowGrappledIntoContainer | ToggleCurtain | ToggleDoor | ToggleWindow => 100,
        }
    }
}

/// `ContextualAction`: the object is the world's (by square and index).
#[derive(Clone, Debug, PartialEq)]
pub struct ContextualAction {
    pub action: ContextualActionKind,
    pub dir: Option<IsoDirections>,
    pub square: Option<(i32, i32, i32)>,
    /// The world's handle of the object (`IsoObject`), if any.
    pub object: Option<i64>,
    pub priority: i32,
    pub behind: bool,
}

impl ContextualAction {
    /// `ContextualAction.alloc(action, dir, square, object)`.
    pub fn new(action: ContextualActionKind, dir: Option<IsoDirections>, square: Option<(i32, i32, i32)>, object: Option<i64>) -> Self {
        ContextualAction { action, dir, square, object, priority: action.priority(), behind: false }
    }
}

/// What `doContextNSWE` and the door helpers see of the player.
#[derive(Clone, Copy, Debug)]
pub struct ContextQuery {
    pub chr: i32,
    pub square: (i32, i32, i32),
    pub x: f32,
    pub y: f32,
    pub shift: bool,
    pub ticks_since_pressed_movement: f32,
    pub grappling: bool,
    /// `timePressedContext`: how long the interact key has been held.
    pub time_pressed_context: f32,
    /// `ignoreAutoVault`.
    pub ignore_auto_vault: bool,
}

/// The square checks of `doContextHutch`, `doContextButcherHook`,
/// `doContextAnimalInteraction` and `doContextRestOnFurniture`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextSpecial {
    Hutch,
    ButcherHook,
    AnimalInteraction,
    RestOnFurniture,
}

/// `pickBestContextualAction`.
pub fn pick_best(actions: &[ContextualAction]) -> Option<&ContextualAction> {
    let mut best: Option<&ContextualAction> = None;
    let mut best_behind: Option<&ContextualAction> = None;
    let (mut max, mut max_behind) = (-1, -1);
    for ca in actions {
        if ca.behind {
            if ca.priority > max_behind {
                best_behind = Some(ca);
                max_behind = ca.priority;
            }
        } else if ca.priority > max {
            best = Some(ca);
            max = ca.priority;
        }
    }
    match (best, best_behind) {
        (None, b) => b,
        (Some(b), Some(bb)) if bb.priority > b.priority => Some(bb),
        (b, _) => b,
    }
}

/// The unported branches fail like this rather than offering nothing.
fn unsupported<T>(operation: &'static str) -> HostResult<T> {
    Err(RuntimeHostError::Unsupported { operation, kind: None })
}

impl<I: 'static> GameCharacter<I> {
    /// `doContext()`.
    pub fn do_context(&mut self, input: &dyn PlayerInput, out: &mut PlayerUpdate, cx: &mut StateCx<'_, I>) -> bool {
        if self.player.ignore_context_key || self.vars.block_movement {
            return false;
        }
        // TODO(grappling): isPickingUpBody / isPuttingDownBody.
        let millis = (self.player.time_pressed_context * 1000.0).floor() as f64;
        if self.is_sitting() {
            out.events.push(PlayerEvent::ContextKey { millis });
            return true;
        }
        if cx.world.vehicle_useable_part_near(self.id) {
            return false;
        }
        let Some(square) = self.current_square_xyz() else { return false };
        let dir = self.vars.dir;
        let (dx, dy) = dir.delta();
        if cx.world.square(square.0 + dx, square.1 + dy, square.2).is_none() {
            return false;
        }
        let q = ContextQuery {
            chr: self.id,
            square,
            x: self.vars.x,
            y: self.vars.y,
            shift: self.player.input.is_shift_key_down(input),
            ticks_since_pressed_movement: self.player.ticks_since_pressed_movement,
            grappling: self.is_grappling(),
            time_pressed_context: self.player.time_pressed_context,
            ignore_auto_vault: self.vars.ignore_auto_vault,
        };
        let actions = match self.context_actions(&q, dir, cx) {
            Ok(actions) => actions,
            Err(error) => {
                cx.record_host_error(error);
                return false;
            }
        };
        if actions.is_empty() {
            out.events.push(PlayerEvent::ContextKey { millis });
            return false;
        }
        if let Some(ca) = pick_best(&actions) {
            // `performContextualAction`: a door is toggled by the world; the
            // rest is told to the engine.
            if ca.action == ContextualActionKind::ToggleDoor {
                if let Err(error) = cx.world.context_toggle_door(self.id, ca) {
                    cx.record_host_error(error);
                    return false;
                }
            } else {
                out.events.push(PlayerEvent::ContextualAction(ca.clone()));
            }
        }
        true
    }

    /// `doContext()` up to `pickBestContextualAction`: the list of actions
    /// `contextualActions` holds (the squares behind flagged `behind`).
    fn context_actions(&self, q: &ContextQuery, dir: IsoDirections, cx: &mut StateCx<'_, I>) -> HostResult<Vec<ContextualAction>> {
        use IsoDirections as D;
        let mut actions: Vec<ContextualAction> = Vec::new();
        let mut lx = self.vars.x - crate::character::zombie::math::fastfloor(self.vars.x) as f32;
        let mut ly = self.vars.y - crate::character::zombie::math::fastfloor(self.vars.y) as f32;
        let (o1, o2) = match dir {
            D::NW => {
                if ly < lx {
                    self.do_context_nswe(q, D::N, &mut actions, cx)?;
                    self.do_context_nswe(q, D::W, &mut actions, cx)?;
                    (D::S, Some(D::E))
                } else {
                    self.do_context_nswe(q, D::W, &mut actions, cx)?;
                    self.do_context_nswe(q, D::N, &mut actions, cx)?;
                    (D::E, Some(D::S))
                }
            }
            D::NE => {
                lx = 1.0 - lx;
                if ly < lx {
                    self.do_context_nswe(q, D::N, &mut actions, cx)?;
                    self.do_context_nswe(q, D::E, &mut actions, cx)?;
                    (D::S, Some(D::W))
                } else {
                    self.do_context_nswe(q, D::E, &mut actions, cx)?;
                    self.do_context_nswe(q, D::N, &mut actions, cx)?;
                    (D::W, Some(D::S))
                }
            }
            D::SE => {
                lx = 1.0 - lx;
                ly = 1.0 - ly;
                if ly < lx {
                    self.do_context_nswe(q, D::S, &mut actions, cx)?;
                    self.do_context_nswe(q, D::E, &mut actions, cx)?;
                    (D::N, Some(D::W))
                } else {
                    self.do_context_nswe(q, D::E, &mut actions, cx)?;
                    self.do_context_nswe(q, D::S, &mut actions, cx)?;
                    (D::W, Some(D::N))
                }
            }
            D::SW => {
                ly = 1.0 - ly;
                if ly < lx {
                    self.do_context_nswe(q, D::S, &mut actions, cx)?;
                    self.do_context_nswe(q, D::W, &mut actions, cx)?;
                    (D::N, Some(D::E))
                } else {
                    self.do_context_nswe(q, D::W, &mut actions, cx)?;
                    self.do_context_nswe(q, D::S, &mut actions, cx)?;
                    (D::E, Some(D::N))
                }
            }
            d => {
                self.do_context_nswe(q, d, &mut actions, cx)?;
                (IsoDirections::ALL[(d as usize + 4) & 7], None)
            }
        };
        let (dx, dy) = dir.delta();
        if actions.is_empty() && dx != 0 && dy != 0 {
            self.do_context_corners(q, dir, &mut actions, cx)?;
        }
        let count = actions.len();
        for o in std::iter::once(o1).chain(o2) {
            if let Some(obj) = cx.world.context_door_or_window_object(q, o) {
                let found = cx.world.context_do_door_or_window(q, o, obj)?;
                actions.extend(found);
            }
        }
        for a in actions.iter_mut().skip(count) {
            a.behind = true;
        }
        Ok(actions)
    }

    /// `doContextNSWE(dir)`: `true` when a branch applied. A branch adds
    /// its action to `actions` and ends the search (the door branch ends it
    /// even when the object offers nothing).
    fn do_context_nswe(
        &self,
        q: &ContextQuery,
        dir: IsoDirections,
        actions: &mut Vec<ContextualAction>,
        cx: &mut StateCx<'_, I>,
    ) -> HostResult<bool> {
        debug_assert!(matches!(dir, IsoDirections::N | IsoDirections::S | IsoDirections::W | IsoDirections::E));
        let (dx, dy) = dir.delta();
        if cx.world.square(q.square.0 + dx, q.square.1 + dy, q.square.2).is_none() {
            return Ok(false);
        }
        if self.do_context_climb_sheet_rope(q, dir, actions, cx) {
            return Ok(true);
        }
        if let Some(obj) = cx.world.context_door_or_window_object(q, dir) {
            let found = cx.world.context_do_door_or_window(q, dir, obj)?;
            actions.extend(found);
            return Ok(true);
        }
        if !q.shift && Self::do_context_specials(q, dir, actions, cx, true)? {
            return Ok(true);
        }
        if q.shift && q.ticks_since_pressed_movement > 15.0 {
            if Self::do_context_specials(q, dir, actions, cx, false)? {
                return Ok(true);
            }
            // `doContextToggleCurtain`: shift and the interact key on a
            // curtain of a door or window frame.
            return unsupported("IsoPlayer.doContextToggleCurtain");
        }
        if q.grappling {
            return unsupported("IsoPlayer.doContextThrowGrappledTargetOverFence");
        }
        if self.do_context_hop_over_fence(q, dir, actions, cx) {
            return Ok(true);
        }
        Ok(self.do_context_climb_over_wall(q, dir, actions, cx))
    }

    /// `doContextCorners(dir)` for a diagonal facing.
    fn do_context_corners(
        &self,
        q: &ContextQuery,
        dir: IsoDirections,
        actions: &mut Vec<ContextualAction>,
        cx: &mut StateCx<'_, I>,
    ) -> HostResult<bool> {
        if !q.shift && Self::do_context_specials(q, dir, actions, cx, true)? {
            return Ok(true);
        }
        if q.shift && q.ticks_since_pressed_movement > 15.0 {
            return Self::do_context_specials(q, dir, actions, cx, false);
        }
        Ok(false)
    }

    /// `doContextHutch`, then (`all`) `doContextButcherHook`,
    /// `doContextAnimalInteraction` and `doContextRestOnFurniture`: the
    /// first that applies adds its action.
    fn do_context_specials(
        q: &ContextQuery,
        dir: IsoDirections,
        actions: &mut Vec<ContextualAction>,
        cx: &mut StateCx<'_, I>,
        all: bool,
    ) -> HostResult<bool> {
        let rest = [ContextSpecial::ButcherHook, ContextSpecial::AnimalInteraction, ContextSpecial::RestOnFurniture];
        let which = std::iter::once(ContextSpecial::Hutch).chain(rest.into_iter().filter(|_| all));
        for which in which {
            if let Some(a) = cx.world.context_special(q, dir, which)? {
                actions.push(a);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `doContextClimbSheetRope(dir)`.
    fn do_context_climb_sheet_rope(
        &self,
        q: &ContextQuery,
        dir: IsoDirections,
        actions: &mut Vec<ContextualAction>,
        cx: &mut StateCx<'_, I>,
    ) -> bool {
        if q.grappling || !cx.world.context_sheet_rope_side(q, dir) || !self.can_climb_sheet_rope(Some(q.square), cx) {
            return false;
        }
        actions.push(ContextualAction::new(ContextualActionKind::ClimbSheetRope, Some(dir), Some(q.square), None));
        true
    }

    /// The test both `doContextHopOverFence` and `doContextClimbOverWall`
    /// start with: not ignoring the auto vault, not grappling, and the
    /// landing safe (or the key held for half a second).
    fn context_may_climb_over(q: &ContextQuery, dir: IsoDirections, cx: &StateCx<'_, I>) -> bool {
        if q.ignore_auto_vault || q.grappling {
            return false;
        }
        let safe = cx.world.context_safe_to_climb_over(q, dir);
        !(q.time_pressed_context < 0.5 && !safe)
    }

    /// `doContextHopOverFence(dir)`.
    fn do_context_hop_over_fence(
        &self,
        q: &ContextQuery,
        dir: IsoDirections,
        actions: &mut Vec<ContextualAction>,
        cx: &mut StateCx<'_, I>,
    ) -> bool {
        if !Self::context_may_climb_over(q, dir, cx) {
            return false;
        }
        let Some((square, object)) = cx.world.context_hoppable(q, dir) else { return false };
        actions.push(ContextualAction::new(ContextualActionKind::ClimbOverFence, Some(dir), Some(square), object));
        true
    }

    /// `doContextClimbOverWall(dir)`.
    fn do_context_climb_over_wall(
        &self,
        q: &ContextQuery,
        dir: IsoDirections,
        actions: &mut Vec<ContextualAction>,
        cx: &mut StateCx<'_, I>,
    ) -> bool {
        if !Self::context_may_climb_over(q, dir, cx) || !self.can_climb_over_wall(dir, cx) {
            return false;
        }
        actions.push(ContextualAction::new(ContextualActionKind::ClimbOverWall, Some(dir), None, None));
        true
    }
}

#[cfg(test)]
mod tests {
    use pz_pathfind::mover::ObjectRef;

    use super::*;
    use crate::character::player::input::NoInput;
    use crate::character::player::PlayerUpdate;
    use crate::character::state::{SquareInfo, StateWorld};
    use crate::character::tests::{fixture, player, Recorder};

    /// Doors on square edges, as the world's helpers would find them.
    #[derive(Default)]
    struct Doors {
        /// (the square the player stands on, the direction it is found in).
        doors: Vec<((i32, i32, i32), IsoDirections)>,
        toggled: Vec<Option<i64>>,
        refuse: bool,
    }

    impl StateWorld<()> for Doors {
        fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
            Some(SquareInfo { x, y, z, ..Default::default() })
        }
        fn context_door_or_window_object(&self, q: &ContextQuery, dir: IsoDirections) -> Option<ObjectRef> {
            self.doors
                .iter()
                .any(|&(sq, d)| sq == q.square && d == dir)
                .then(|| ObjectRef { x: q.square.0, y: q.square.1, z: q.square.2, id: 7 })
        }
        fn context_do_door_or_window(
            &mut self,
            q: &ContextQuery,
            dir: IsoDirections,
            obj: ObjectRef,
        ) -> HostResult<Vec<ContextualAction>> {
            if self.refuse {
                return unsupported("test");
            }
            Ok(vec![ContextualAction::new(ContextualActionKind::ToggleDoor, Some(dir), Some(q.square), Some(obj.id as i64))])
        }
        fn context_toggle_door(&mut self, _chr: i32, action: &ContextualAction) -> HostResult<()> {
            self.toggled.push(action.object);
            Ok(())
        }
    }

    /// Presses the interact key facing `dir` from the middle of (4, 5, 0).
    fn press(world: &mut Doors, dir: IsoDirections) -> (bool, PlayerUpdate, bool) {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut c = player(&sm, &mut chars);
        c.path.current_square = Some((4, 5, 0));
        (c.vars.x, c.vars.y) = (4.5, 5.5);
        c.vars.dir = dir;
        let mut out = PlayerUpdate::default();
        let mut cx = StateCx::new(&sm, &chars, &ctx, world);
        let acted = c.do_context(&NoInput, &mut out, &mut cx);
        (acted, out, cx.has_host_error())
    }

    #[test]
    fn a_door_in_front_is_toggled() {
        let mut world = Doors { doors: vec![((4, 5, 0), IsoDirections::N)], ..Default::default() };
        let (acted, out, failed) = press(&mut world, IsoDirections::N);
        assert!(acted && !failed);
        assert_eq!(world.toggled, vec![Some(7)]);
        assert!(out.events.is_empty());
    }

    #[test]
    fn nothing_in_front_offers_no_action() {
        // A door at the side is neither in front nor behind.
        let mut world = Doors { doors: vec![((4, 5, 0), IsoDirections::E)], ..Default::default() };
        let (acted, out, failed) = press(&mut world, IsoDirections::N);
        assert!(!acted && !failed);
        assert!(world.toggled.is_empty());
        assert_eq!(out.events, vec![PlayerEvent::ContextKey { millis: 0.0 }]);
    }

    #[test]
    fn a_diagonal_facing_checks_both_edges() {
        // Facing north-west from the middle of the square: west, then north.
        for edge in [IsoDirections::N, IsoDirections::W] {
            let mut world = Doors { doors: vec![((4, 5, 0), edge)], ..Default::default() };
            let (acted, _, failed) = press(&mut world, IsoDirections::NW);
            assert!(acted && !failed, "{edge:?}");
            assert_eq!(world.toggled, vec![Some(7)], "{edge:?}");
        }
        // A door on the far edges is not offered.
        let mut world = Doors { doors: vec![((4, 5, 0), IsoDirections::NE)], ..Default::default() };
        assert!(!press(&mut world, IsoDirections::NW).0);
    }

    #[test]
    fn a_door_behind_is_found_when_nothing_is_in_front() {
        // Facing north-west, the squares behind are east and south.
        let mut world = Doors { doors: vec![((4, 5, 0), IsoDirections::S)], ..Default::default() };
        let (acted, _, failed) = press(&mut world, IsoDirections::NW);
        assert!(acted && !failed);
        assert_eq!(world.toggled, vec![Some(7)]);
    }

    #[test]
    fn an_unported_branch_stops_the_key_with_a_host_error() {
        let mut world = Doors { doors: vec![((4, 5, 0), IsoDirections::N)], refuse: true, ..Default::default() };
        let (acted, out, failed) = press(&mut world, IsoDirections::N);
        assert!(!acted && failed);
        assert!(world.toggled.is_empty() && out.events.is_empty());
    }

    #[test]
    fn best_prefers_front_unless_behind_higher() {
        let mut a = ContextualAction::new(ContextualActionKind::ClimbOverFence, None, None, None);
        let mut b = ContextualAction::new(ContextualActionKind::ToggleDoor, None, None, None);
        b.behind = true;
        b.priority = 110;
        assert_eq!(pick_best(&[a.clone(), b.clone()]).unwrap().action, ContextualActionKind::ToggleDoor);
        a.priority = 120;
        assert_eq!(pick_best(&[a.clone(), b]).unwrap().action, ContextualActionKind::ClimbOverFence);
    }
}
