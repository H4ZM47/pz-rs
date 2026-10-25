//! What a state reads from outside its owner: [`StateCx`] bundles the
//! script and character statics, the engine singletons ([`SimContext`])
//! and the [`StateWorld`] (other characters, squares, path finding).
//!
//! Every [`StateWorld`] method has the answer an empty world gives, so a
//! world only implements what it has.

use pz_scripts::ScriptManager;

use crate::character::{GameCharacter, SimContext};
use crate::Characters;

pub use pz_pathfind::behavior::BehaviorResult;
use pz_pathfind::mover::{MoverId, PathMover};
pub use pz_world::los::TestResults;

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

    /// Takes the character with `id` out of the world for a call that
    /// needs it and another character mutably (a hit); it goes back with
    /// [`Self::put_character`].
    fn take_character(&mut self, _id: i32) -> Option<GameCharacter<I>> {
        None
    }

    /// Returns a character taken with [`Self::take_character`].
    fn put_character(&mut self, _chr: GameCharacter<I>) {}

    /// The ids of the characters `CombatManager` considers (the cell's
    /// moving objects near `(x, y, z)`).
    fn characters_near(&self, _x: f32, _y: f32, _z: f32, _radius: f32) -> Vec<i32> {
        Vec::new()
    }

    /// `getCell().getGridSquare(x, y, z)`.
    fn square(&self, _x: i32, _y: i32, _z: i32) -> Option<SquareInfo> {
        None
    }

    /// `getPathFindBehavior2().pathToLocation(x, y, z)` for `chr` (the
    /// world owns each character's `pz_pathfind::behavior::PathFindBehavior2`
    /// and the `PathfindService`, keyed by `chr.mover_id()`, which carries
    /// the character's kind and id: see [`GameCharacter::mover_id_of`]).
    fn path_to_location(&mut self, _chr: &mut dyn PathMover, _x: i32, _y: i32, _z: i32) {}

    /// `getPathFindBehavior2().pathToCharacter(target)`; `target` is the
    /// target's `PathMover::mover_id()` (kind and id).
    fn path_to_character(&mut self, _chr: &mut dyn PathMover, _target: MoverId) {}

    /// `getPathFindBehavior2().update()`.
    fn path_update(&mut self, _chr: &mut dyn PathMover) -> BehaviorResult {
        BehaviorResult::Failed
    }

    /// `getPathFindBehavior2().cancel()`.
    fn path_cancel(&mut self, _chr: &mut dyn PathMover) {}

    /// `LosUtil.lineClear(cell, x0, y0, z0, x1, y1, z1, ignoreDoors)`
    /// between two squares (`pz_world::los::line_clear` over the world's
    /// squares).
    fn los_line_clear(&self, _from: (i32, i32, i32), _to: (i32, i32, i32), _ignore_doors: bool) -> TestResults {
        TestResults::Clear
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
