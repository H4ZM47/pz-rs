//! What a state reads from outside its owner: [`StateCx`] bundles the
//! script and character statics, the engine singletons ([`SimContext`])
//! and the [`StateWorld`] (other characters, squares, path finding).
//!
//! Every [`StateWorld`] method has the answer an empty world gives, so a
//! world only implements what it has.

use pz_scripts::ScriptManager;

use crate::character::{GameCharacter, SimContext};
use crate::Characters;

/// `PathFindBehavior2.BehaviorResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BehaviorResult {
    Working,
    Failed,
    Succeeded,
}

/// The part of `PathFindBehavior2` the states use. The character side of
/// the pathfinder is `pz_pathfind::mover::PathMover` (implemented in
/// [`crate::character::path_mover`]); this is the behaviour object's
/// interface until pz-pathfind ports `PathFindBehavior2`.
/// TODO(pz-pathfind): replace with its `PathFindBehavior2`.
pub trait PathFindBehavior {
    /// `pathToLocation(x, y, z)`.
    fn path_to_location(&mut self, _x: i32, _y: i32, _z: i32) {}
    /// `pathToLocationF(x, y, z)`.
    fn path_to_location_f(&mut self, _x: f32, _y: f32, _z: f32) {}
    /// `pathToCharacter(target)` (by character id).
    fn path_to_character(&mut self, _target: i32) {}
    /// `update()`.
    fn update(&mut self) -> BehaviorResult {
        BehaviorResult::Failed
    }
    /// `cancel()`.
    fn cancel(&mut self) {}
    /// `isStrafing()`.
    fn is_strafing(&self) -> bool {
        false
    }
    /// `getTargetChar()` (by character id).
    fn target_char(&self) -> Option<i32> {
        None
    }
}

/// What the states read about a square (`IsoGridSquare`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SquareInfo {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// `isInARoom()`.
    pub in_room: bool,
    /// `TreatAsSolidFloor()`.
    pub solid_floor: bool,
    /// `HasStairs()`.
    pub has_stairs: bool,
}

/// The world as the states see it. Characters other than the one being
/// updated are reached by id; the owner is never handed out again while it
/// is borrowed.
pub trait StateWorld<I> {
    /// The character with `id` (`IsoMovingObject.getID`), if it is not the
    /// one currently borrowed.
    fn character_mut(&mut self, _id: i32) -> Option<&mut GameCharacter<I>> {
        None
    }

    /// The ids of the characters `CombatManager` considers (the cell's
    /// moving objects near `(x, y, z)`).
    fn characters_near(&self, _x: f32, _y: f32, _z: f32, _radius: f32) -> Vec<i32> {
        Vec::new()
    }

    /// `getCell().getGridSquare(x, y, z)`.
    fn square(&self, _x: i32, _y: i32, _z: i32) -> Option<SquareInfo> {
        None
    }

    /// `getPathFindBehavior2()` of the character with `id`.
    fn path_find(&mut self, _id: i32) -> Option<&mut dyn PathFindBehavior> {
        None
    }

    /// `LosUtil.lineClear` / `PolygonalMap2.lineClearCollide` between two
    /// points on one level (`pz_pathfind::main_thread::los_util` over the
    /// world's squares).
    fn line_clear(&self, _x0: f32, _y0: f32, _x1: f32, _y1: f32, _z: i32) -> bool {
        true
    }
}

/// A world with nothing in it.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullWorld;

impl<I> StateWorld<I> for NullWorld {}

/// The context a state runs in.
pub struct StateCx<'a, I> {
    pub sm: &'a ScriptManager,
    pub chars: &'a Characters,
    pub ctx: &'a dyn SimContext,
    pub world: &'a mut dyn StateWorld<I>,
}

impl<'a, I> StateCx<'a, I> {
    pub fn new(sm: &'a ScriptManager, chars: &'a Characters, ctx: &'a dyn SimContext, world: &'a mut dyn StateWorld<I>) -> Self {
        StateCx { sm, chars, ctx, world }
    }

    /// The same context with a shorter borrow (to pass on).
    pub fn reborrow(&mut self) -> StateCx<'_, I> {
        StateCx { sm: self.sm, chars: self.chars, ctx: self.ctx, world: &mut *self.world }
    }
}
