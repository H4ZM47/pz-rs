//! `IsoPlayer.doContext()`: the interact key's contextual action (doors,
//! windows, fences, walls, sheet ropes, ...). The direction split and the
//! choice are ported here; what each direction offers is the world's
//! ([`StateWorld::context_actions_nswe`], [`StateWorld::context_actions_corners`],
//! [`StateWorld::context_door_or_window`]) and performing the chosen action
//! is [`PlayerEvent::ContextualAction`].
//!
//! [`StateWorld::context_actions_nswe`]: crate::character::state::StateWorld::context_actions_nswe
//! [`StateWorld::context_actions_corners`]: crate::character::state::StateWorld::context_actions_corners
//! [`StateWorld::context_door_or_window`]: crate::character::state::StateWorld::context_door_or_window

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

impl<I: 'static> GameCharacter<I> {
    /// `doContext()`.
    pub fn do_context(&mut self, input: &dyn PlayerInput, out: &mut PlayerUpdate, cx: &mut StateCx<'_, I>) -> bool {
        use IsoDirections as D;
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
            grappling: false,
        };
        let mut actions: Vec<ContextualAction> = Vec::new();
        let nswe = |d: IsoDirections, actions: &mut Vec<ContextualAction>, cx: &mut StateCx<'_, I>| {
            let (ddx, ddy) = d.delta();
            if cx.world.square(square.0 + ddx, square.1 + ddy, square.2).is_some() {
                actions.extend(cx.world.context_actions_nswe(&q, d));
            }
        };
        let mut lx = self.vars.x - crate::character::zombie::math::fastfloor(self.vars.x) as f32;
        let mut ly = self.vars.y - crate::character::zombie::math::fastfloor(self.vars.y) as f32;
        let (o1, o2) = match dir {
            D::NW => {
                if ly < lx {
                    nswe(D::N, &mut actions, cx);
                    nswe(D::W, &mut actions, cx);
                    (D::S, Some(D::E))
                } else {
                    nswe(D::W, &mut actions, cx);
                    nswe(D::N, &mut actions, cx);
                    (D::E, Some(D::S))
                }
            }
            D::NE => {
                lx = 1.0 - lx;
                if ly < lx {
                    nswe(D::N, &mut actions, cx);
                    nswe(D::E, &mut actions, cx);
                    (D::S, Some(D::W))
                } else {
                    nswe(D::E, &mut actions, cx);
                    nswe(D::N, &mut actions, cx);
                    (D::W, Some(D::S))
                }
            }
            D::SE => {
                lx = 1.0 - lx;
                ly = 1.0 - ly;
                if ly < lx {
                    nswe(D::S, &mut actions, cx);
                    nswe(D::E, &mut actions, cx);
                    (D::N, Some(D::W))
                } else {
                    nswe(D::E, &mut actions, cx);
                    nswe(D::S, &mut actions, cx);
                    (D::W, Some(D::N))
                }
            }
            D::SW => {
                ly = 1.0 - ly;
                if ly < lx {
                    nswe(D::S, &mut actions, cx);
                    nswe(D::W, &mut actions, cx);
                    (D::N, Some(D::E))
                } else {
                    nswe(D::W, &mut actions, cx);
                    nswe(D::S, &mut actions, cx);
                    (D::E, Some(D::N))
                }
            }
            d => {
                nswe(d, &mut actions, cx);
                (IsoDirections::ALL[(d as usize + 4) & 7], None)
            }
        };
        if actions.is_empty() && dx != 0 && dy != 0 {
            actions.extend(cx.world.context_actions_corners(&q, dir));
        }
        let count = actions.len();
        for o in std::iter::once(o1).chain(o2) {
            if let Some(a) = cx.world.context_door_or_window(&q, o) {
                actions.push(a);
            }
        }
        for a in actions.iter_mut().skip(count) {
            a.behind = true;
        }
        if actions.is_empty() {
            out.events.push(PlayerEvent::ContextKey { millis });
            return false;
        }
        if let Some(ca) = pick_best(&actions) {
            out.events.push(PlayerEvent::ContextualAction(ca.clone()));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
