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
use pz_pathfind::behavior::PathFindBehavior2;
pub use pz_world::los::TestResults;
pub use pz_world::square::{CollideAdjacent, CollideMover, HoppableAction, IsoDirections};

use pz_audio::world_sound::{SoundWorld, WorldSoundManager};

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
    /// `HasStairsNorth()`.
    pub has_stairs_north: bool,
    /// `HasStairsWest()`.
    pub has_stairs_west: bool,
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

    /// `PathfindNative.instance.cancelRequest(chr)`: drops `chr`'s pending
    /// request (`PathfindService::cancel_request(chr.mover_id())`); its
    /// `PathFindBehavior2` is left as it is.
    fn path_cancel_request(&mut self, _chr: &mut dyn PathMover) {}

    /// `LosUtil.lineClear(cell, x0, y0, z0, x1, y1, z1, ignoreDoors)`
    /// between two squares (`pz_world::los::line_clear` over the world's
    /// squares).
    fn los_line_clear(&self, _from: (i32, i32, i32), _to: (i32, i32, i32), _ignore_doors: bool) -> TestResults {
        TestResults::Clear
    }

    // ---- what zombies read and do (`IsoZombie`, the zombie states) ----

    /// `WorldSoundManager.instance` (`pz_audio::world_sound`) and the cell
    /// it reads.
    fn world_sounds(&self) -> Option<(&WorldSoundManager, &dyn SoundWorld)> {
        None
    }
    /// `(r + g + b) / 3` of `square.lighting[playerIndex].lightInfo()`
    /// (pz-lighting's).
    fn square_light(&self, _sq: (i32, i32, i32), _player_index: i32) -> f32 {
        1.0
    }
    /// `square.getLightLevel(playerIndex)`.
    fn square_light_level(&self, _sq: (i32, i32, i32), _player_index: i32) -> f32 {
        1.0
    }
    /// `RenderSettings.getInstance().getAmbientForPlayer(playerIndex)`.
    fn ambient_for_player(&self, _player_index: i32) -> f32 {
        1.0
    }
    /// `square.getRoom()` by id.
    fn square_room(&self, _sq: (i32, i32, i32)) -> Option<i64> {
        None
    }
    /// `square.getBuilding()` by id.
    fn square_building(&self, _sq: (i32, i32, i32)) -> Option<i64> {
        None
    }
    /// `square.getProperties().has(IsoFlagType.smoke)`.
    fn square_smoke(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// `square.isCanSee(playerIndex)`.
    fn square_can_see(&self, _sq: (i32, i32, i32), _player_index: i32) -> bool {
        false
    }
    /// `square.isCouldSee(playerIndex)`.
    fn square_could_see(&self, _sq: (i32, i32, i32), _player_index: i32) -> bool {
        false
    }
    /// The sprite names of `square.getObjects()` in order.
    fn square_sprite_names(&self, _sq: (i32, i32, i32)) -> Vec<String> {
        Vec::new()
    }
    /// `square.getGridSneakModifier(onlySolidTrans)`.
    fn grid_sneak_modifier(&self, _sq: (i32, i32, i32), _only_solid_trans: bool) -> f32 {
        1.0
    }
    /// `square.isSomethingTo(other)`.
    fn is_something_to(&self, _a: (i32, i32, i32), _b: (i32, i32, i32)) -> bool {
        false
    }
    /// `square.isFree(true)`.
    fn square_is_free(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// `square.isSameStaircase(x, y, z)`.
    fn same_staircase(&self, _sq: (i32, i32, i32), _x: i32, _y: i32, _z: i32) -> bool {
        false
    }
    /// `BaseVehicle.getIntersectPoint` of any vehicle on the segment
    /// (`IsoZombie.isVehicleBetween`).
    fn vehicle_between(&self, _from: (f32, f32, f32), _to: (f32, f32, f32)) -> bool {
        false
    }
    /// `DistToSquared(vehicle)` from `(x, y)` (the closest point of the
    /// vehicle's polygon).
    fn vehicle_dist_squared(&self, _vehicle: i16, _x: f32, _y: f32) -> f32 {
        f32::MAX
    }
    /// `IsoWorld.instance.currentCell.getChunk(wx, wy) != null`.
    fn chunk_loaded(&self, _wx: i32, _wy: i32) -> bool {
        true
    }
    /// `PolygonalMap2.lineClearCollide(x0, y0, x1, y1, z, ignore, flags)`:
    /// true when something is in the way.
    #[allow(clippy::too_many_arguments)]
    fn line_clear_collide(&self, _x0: f32, _y0: f32, _x1: f32, _y1: f32, _z: i32, _ignore: Option<i32>, _flags: i32) -> bool {
        false
    }
    /// `LosUtil.lineClearCollide(x0, y0, z0, x1, y1, z1, ignoreDoors)`:
    /// true when blocked.
    fn los_line_clear_collide(&self, _from: (i32, i32, i32), _to: (i32, i32, i32), _ignore_doors: bool) -> bool {
        false
    }
    /// `PolygonalMap2.canStandAt(x, y, z, null, false, true)`.
    fn can_stand_at(&self, _x: f32, _y: f32, _z: i32) -> bool {
        true
    }
    /// The character's `PathFindBehavior2` (read only).
    fn path_behavior(&self, _id: i32) -> Option<&PathFindBehavior2> {
        None
    }
    /// `getPathFindBehavior2().pathToSound(x, y, z)`.
    fn path_to_sound(&mut self, _chr: &mut dyn PathMover, _x: i32, _y: i32, _z: i32) {}
    /// `getPathFindBehavior2().pathToLocationF(x, y, z)`.
    fn path_to_location_f(&mut self, _chr: &mut dyn PathMover, _x: f32, _y: f32, _z: f32) {}
    /// `getPathFindBehavior2().walkingOnTheSpot.reset(x, y)`.
    fn walking_on_the_spot_reset(&mut self, _id: i32, _x: f32, _y: f32) {}
    /// `getPathFindBehavior2().walkingOnTheSpot.check(x, y)`.
    fn walking_on_the_spot_check(&mut self, _id: i32, _x: f32, _y: f32) -> bool {
        false
    }
    /// `current.testCollideSpecialObjects(feeler)`: the door, window,
    /// window frame or thumpable between two squares.
    fn special_object_between(&self, _from: (i32, i32, i32), _to: (i32, i32, i32)) -> Option<ThumpTarget> {
        None
    }
    /// What the zombie code reads about a thump target, as `chr` sees it.
    fn thump_info(&self, _target: ThumpTarget, _chr: i32) -> Option<ThumpInfo> {
        None
    }
    /// `target.Thump(chr)`: the object's damage and sounds (the object's
    /// code; it may clear `chr`'s thump target).
    fn thump(&mut self, _target: ThumpTarget, _chr: &mut GameCharacter<I>) {}
    /// The square has a sheet rope (`climbSheet*` flag and `getSheetRope()`).
    fn has_sheet_rope(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// `sheetRope.sheetRopeHealth -= damage` and the sprite damage.
    fn damage_sheet_rope(&mut self, _sq: (i32, i32, i32), _damage: f32) {}
    /// `square.hasFarmingPlant()`.
    fn has_farming_plant(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// `square.destroyFarmingPlant()`.
    fn destroy_farming_plant(&mut self, _sq: (i32, i32, i32)) {}
    /// `square.hasLitCampfire()`.
    fn has_lit_campfire(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// `square.putOutCampfire()`.
    fn put_out_campfire(&mut self, _sq: (i32, i32, i32)) {}
    /// `PolygonalMap2.instance.resolveCollision(chr, nx, ny)`: the obstacle
    /// clip of the next position (pz-pathfind's `PathHost::resolve_collision`,
    /// one `CollideWithObstacles` per world, called in game order).
    fn resolve_collision(&mut self, _chr: &dyn pz_pathfind::mover::CharacterView, nx: f32, ny: f32) -> (f32, f32) {
        (nx, ny)
    }
    /// `PolygonalMap2.instance.resolveCollision(chr, radius, finalPos)`
    /// (pz-pathfind's `PathHost::resolve_collision_radius`), for
    /// `slideAwayFromWalls` with `includePolyCollisions`: whether `chr`
    /// with `radius` overlaps a vehicle obstacle, and `finalPos`.
    fn resolve_collision_radius(&mut self, chr: &dyn pz_pathfind::mover::CharacterView, _radius: f32) -> (bool, (f32, f32)) {
        (false, (chr.x(), chr.y()))
    }
    /// `FireCheck()`, `SpreadFire()` and `ReduceHealthWhenBurning()` in
    /// that order, as `IsoGameCharacter.update` calls them: the world owns
    /// `IsoFireManager` and the squares, so it runs
    /// [`crate::character::fire::update_character_fire`] (or the three
    /// `pz_world::fire::character` functions) over `chr` with `rand`.
    fn character_fire(
        &mut self,
        _chr: &mut dyn pz_world::fire::character::FireCharacter,
        _frame: pz_world::fire::character::CharacterFireFrame,
        _rand: &mut dyn pz_world::java::GameRand,
    ) {
    }
    /// `getGlobalMovementMod(true)`'s vegetation part for one square (the
    /// character's, then its feeler square): when it has a tree or a bush,
    /// `doTreeNoises()` and `setRenderEffect(Vegetation_Rustle)` on each of
    /// its trees and bushes past the first object; a new effect draws
    /// `Rand.Next(45f, 60f)` (`ObjectRenderEffects.getNew`).
    fn rustle_vegetation(&mut self, _chr: i32, _sq: (i32, i32, i32), _ctx: &dyn crate::character::SimContext) {}
    /// `square.testCollideAdjacent(mover, dx, dy, dz)` from `from`.
    fn test_collide_adjacent(&self, _mover: MoverId, _from: (i32, i32, i32), _dx: i32, _dy: i32, _dz: i32) -> bool {
        false
    }
    /// `checkBreakHoppable()`, `checkHitHoppable()` and
    /// `checkBreakBendableFence(current)` after a square-edge collision,
    /// done by the world. The character code asks
    /// [`Self::hoppable_actions`] instead and acts on the answer; this stays
    /// for worlds that drive characters they own.
    fn collided_with_hoppables(&mut self, _chr: i32, _current: Option<(i32, i32, i32)>) {}
    /// `square.testCollideAdjacent(mover, dx, dy, dz)` from `from` with the
    /// mover (pz-world's `CollideEnv::test_collide_adjacent`): the special
    /// object it hits comes back for the mover's `collideWith(obj)`,
    /// `setCollidedWithDoor` and `setCollidedObject`. The default is the
    /// mover-blind [`Self::test_collide_adjacent`].
    #[allow(clippy::too_many_arguments)]
    fn test_collide_adjacent_mover(
        &self,
        mover: &dyn CollideMover,
        id: MoverId,
        from: (i32, i32, i32),
        dx: i32,
        dy: i32,
        dz: i32,
    ) -> CollideAdjacent {
        let _ = mover;
        CollideAdjacent { blocked: self.test_collide_adjacent(id, from, dx, dy, dz), ..Default::default() }
    }
    /// `checkBreakHoppable()`, `checkHitHoppable()` and
    /// `checkBreakBendableFence(current)` of `mover` (character `chr`) after
    /// a square-edge collision from `last` (pz-world's
    /// `CollideEnv::hoppable_actions`): the thump targets and fence climbs
    /// the character then does.
    fn hoppable_actions(&self, _mover: &dyn CollideMover, _chr: i32, _last: (i32, i32, i32)) -> Vec<HoppableAction> {
        Vec::new()
    }
    /// `climbOverFence(dir)`'s world checks from the character's square
    /// `from`: `IsoWindow.canClimbThroughHelper(chr, current, opposite,
    /// north)`, `current.isPlayerAbleToHopWallTo(dir, opposite)` and
    /// `BentFences.checkDamageHoppableFence` (which may damage the fence),
    /// then what `ClimbOverFenceState.setParams` reads of the opposite
    /// square. `None` when the climb is refused.
    fn fence_climb(&mut self, _chr: &dyn pz_pathfind::mover::CharacterView, _zombie: bool, _from: (i32, i32, i32), _dir: IsoDirections) -> Option<FenceClimb> {
        None
    }
    /// A zombie path finding toward an unloaded square outside a building
    /// is virtualized (`ZombiePopulationManager.virtualizeZombie`); true
    /// when that happened.
    fn virtualize_if_unloaded(&mut self, _chr: i32, _next_x: i32, _next_y: i32) -> bool {
        false
    }
    // ---- the character's animation (`pz_anim::StateAnimHooks` on the
    // world's animator and player for `chr`) ----

    /// `ThumpState.execute`'s thump count this frame
    /// (`StateAnimHooks::thumps_this_frame` with the character's
    /// `isAnimationUpdatingThisFrame()` and `getAnimationTimeDelta()`).
    fn thumps_this_frame(&mut self, _chr: i32) -> i32 {
        0
    }
    /// `ZombieTurnAlerted.enter`: `StateAnimHooks::turn_alerted_enter`.
    fn turn_alerted_enter(&mut self, _chr: i32, _target_angle: f32) {}
    /// `doDeferredMovement`'s step: `StateAnimHooks::deferred_movement_step`.
    fn deferred_movement_step(&mut self, _chr: i32, _input: &pz_anim::DeferredMovementInput) -> Option<(f32, f32)> {
        None
    }
    /// The non-ghost players standing on `sq` (`getMovingObjects()`).
    fn players_on_square(&self, _sq: (i32, i32, i32)) -> Vec<i32> {
        Vec::new()
    }
    /// `square.getDeadBody()` for the corpse search.
    fn dead_body_at(&self, _sq: (i32, i32, i32)) -> Option<CorpseInfo> {
        None
    }
    /// A corpse by id (`IsoDeadBody`).
    fn dead_body(&self, _id: i64) -> Option<CorpseInfo> {
        None
    }
    /// `getEatingZombies()` of a corpse or character: add or remove `zombie`.
    fn set_eating(&mut self, _target: EatTarget, _zombie: i32, _eating: bool) {}
    /// `getEatingZombies().size()`.
    fn eating_count(&self, _target: EatTarget) -> usize {
        0
    }
}

/// A `Thumpable`: an object on a square or a vehicle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumpTarget {
    Object(pz_pathfind::mover::ObjectRef),
    Vehicle(i16),
}

/// What `ClimbOverFenceState.setParams` reads of the square beyond the
/// fence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FenceClimb {
    /// `oppositeSq.has(solidtrans)` (`COUNTER`: the climb ends on a
    /// counter).
    pub counter: bool,
    /// `oppositeSq.TreatAsSolidFloor()`.
    pub solid_floor: bool,
    /// `canClimbDownSheetRope(oppositeSq)`.
    pub sheet_rope: bool,
}

/// The class of a thumpable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThumpKind {
    Door,
    Window,
    WindowFrame,
    Thumpable,
    Barricade,
    Vehicle,
    #[default]
    Other,
}

/// What the zombie code reads about a thump target.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThumpInfo {
    pub kind: ThumpKind,
    /// `getSquare()`.
    pub square: Option<(i32, i32, i32)>,
    /// `getNorth()` (or a barricade's N/S direction).
    pub north: bool,
    pub destroyed: bool,
    /// `IsoDoor.isOpen()` / `IsoThumpable.open`.
    pub open: bool,
    /// `IsoThumpable.isDoor()`.
    pub is_door: bool,
    pub hoppable: bool,
    pub tall_hoppable: bool,
    /// `canClimbThrough(chr)`.
    pub can_climb_through: bool,
    /// `getThumpableFor(chr)`.
    pub thumpable_for: Option<ThumpTarget>,
    /// The kind of `getThumpableFor(chr)`.
    pub thumpable_for_kind: ThumpKind,
    /// `getThumpableFor(chr).getThumpCondition()`.
    pub thump_condition: f32,
    /// The thumpable's thump sound (`IsoDoor/IsoThumpable.getThumpSound()`,
    /// a sprite's `ThumpSound`, else generic).
    pub thump_sound: String,
    /// `getSoundPrefix()`.
    pub sound_prefix: String,
    /// `IsoThumpable.isBlockAllTheSquare()`.
    pub block_all_the_square: bool,
    /// The barricade (`getThumpableFor`) is metal or metal bars.
    pub barricade_metal: bool,
    /// The barricade's plank count.
    pub barricade_planks: i32,
    /// The barricade is the one `IsoBarricade.GetBarricadeForCharacter`
    /// gives for this target.
    pub barricade_for_character: bool,
    /// `getOppositeSquare()` (doors).
    pub opposite_square: Option<(i32, i32, i32)>,
    /// `getObjectIndex() != -1` (or a vehicle's moving index).
    pub in_world: bool,
    /// `getFacingPosition()`.
    pub facing_position: (f32, f32),
}

/// What the corpse search reads about an `IsoDeadBody`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CorpseInfo {
    pub id: i64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub skeleton: bool,
    pub zombie: bool,
    pub animal: bool,
    pub animal_skeleton: bool,
    /// `getStaticMovingObjectIndex() != -1`.
    pub in_world: bool,
    /// `getEatingZombies().size()`.
    pub eating: usize,
}

/// What a zombie eats: a corpse or a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EatTarget {
    Body(i64),
    Character(i32),
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
