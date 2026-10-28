//! What a state reads from outside its owner: [`StateCx`] bundles the
//! script and character statics, the engine singletons ([`SimContext`])
//! and the [`StateWorld`] (other characters, squares, path finding).
//!
//! Every [`StateWorld`] method has the answer an empty world gives, so a
//! world only implements what it has.

use pz_scripts::ScriptManager;
use pz_inventory::floor_drop::{HostResult, RuntimeHostError};
use std::cell::RefCell;
use std::rc::Rc;

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
    /// `has(IsoObjectType.stairsTN)` and the other stair pieces.
    pub stairs: StairsPieces,
    /// `has(IsoFlagType.WallN)`.
    pub wall_n: bool,
    /// `has(IsoFlagType.WallW)`.
    pub wall_w: bool,
    /// `hasSlopedSurface()`.
    pub sloped_surface: bool,
    /// `getSlopedSurfaceHeightMax()`.
    pub sloped_surface_height_max: f32,
}

/// The stair pieces of a square (`IsoObjectType.stairsTN` ... `stairsBW`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StairsPieces {
    pub tn: bool,
    pub tw: bool,
    pub mn: bool,
    pub mw: bool,
    pub bn: bool,
    pub bw: bool,
}

impl StairsPieces {
    /// The top piece of a staircase (`stairsTN` or `stairsTW`).
    pub fn top(&self) -> bool {
        self.tn || self.tw
    }
}

impl SquareInfo {
    /// `getApparentZ(dx, dy)` with `PerformanceSettings.fboRenderChunk`
    /// (always on outside a debug toggle): the stairs' height, else the
    /// sloped surface's (`sloped`, the world's
    /// `getSlopedSurfaceHeight(dx, dy)`).
    pub fn apparent_z(&self, dx: f32, dy: f32, sloped: impl FnOnce(f32, f32) -> f32) -> f32 {
        let clamp = |v: f32| v.clamp(0.0, 1.0);
        let (dx, dy) = (clamp(dx), clamp(dy));
        let fudge = 0.1f32;
        // PZMath.lerp(a, b, t): a + (b - a) * t.
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let z = self.z as f32;
        let s = &self.stairs;
        if s.tn {
            z + lerp(0.6666 + fudge, 1.0, 1.0 - dy)
        } else if s.tw {
            z + lerp(0.6666 + fudge, 1.0, 1.0 - dx)
        } else if s.mn {
            z + lerp(0.3333 + fudge, 0.6666 + fudge, 1.0 - dy)
        } else if s.mw {
            z + lerp(0.3333 + fudge, 0.6666 + fudge, 1.0 - dx)
        } else if s.bn {
            z + lerp(0.01, 0.3333 + fudge, 1.0 - dy)
        } else if s.bw {
            z + lerp(0.01, 0.3333 + fudge, 1.0 - dx)
        } else {
            z + sloped(dx, dy)
        }
    }
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

    /// `getPathFindBehavior2().reset()`.
    fn path_reset(&mut self, _chr: &mut dyn PathMover) {}

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
    /// An animal in `AnimalZoneState` walking toward an unloaded square
    /// outside a building is virtualized
    /// (`AnimalPopulationManager.virtualizeAnimal`, `IsoMovingObject.postupdate`);
    /// true when that happened.
    fn virtualize_animal_if_unloaded(&mut self, _chr: i32, _next_x: i32, _next_y: i32) -> bool {
        false
    }
    /// The animal port's view of the world for the animal `chr`: the
    /// definitions and pz-animals' `AnimalWorld` bound to that animal.
    /// `None` (the default) leaves the animal logic out of an animal's
    /// update; the character part still runs.
    fn animal_host(&mut self, _chr: i32) -> Option<crate::character::animal::AnimalHost<'_>> {
        None
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

    // ---- firearms and ragdolls ----

    /// The physics world (`WorldSimulation` / the Bullet natives), owned by
    /// the world or engine.
    fn physics(&mut self) -> Option<&mut dyn crate::character::firearm::PhysicsWorld> {
        None
    }

    // ---- items on the floor ----

    /// The world a character's items fall into (`getSolidFloorAt`,
    /// `getApparentZ`, `Rand`, `AddWorldInventoryItem`): `pz-world`'s
    /// `square_mutation::FloorDropEnv`. `None`: nothing drops.
    fn floor(&mut self) -> Option<&mut dyn pz_inventory::floor_drop::FloorWorld> {
        None
    }
    /// `chr.primaryHandModel.getAttachmentWorldPosition(MUZZLE)`: `None`
    /// without a hand model, its script or a muzzle attachment.
    fn muzzle_attachment(&self, _chr: i32) -> Option<crate::character::firearm::MuzzleAttachment> {
        None
    }
    /// `IsoUtils.XToIso/YToIso(AimingReticle.getX/Y(playerIndex), 0)`: the
    /// aiming reticle on the floor, iso.
    fn aim_reticle_iso(&self, _chr: i32) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// The animation player's bone transforms for a ballistics target;
    /// `None` without an animation player or bone transforms.
    fn ballistics_skeleton(&self, _chr: i32) -> Option<crate::character::firearm::TargetSkeleton> {
        None
    }
    /// `LosUtil.getFirstBlockingIsoGridSquare(cell, x0, y0, z0, x1, y1, z1,
    /// false)`.
    fn first_blocking_square(&self, _from: (i32, i32, i32), _to: (i32, i32, i32)) -> crate::character::firearm::BlockingSquare {
        crate::character::firearm::BlockingSquare::default()
    }
    /// The shooter's vehicle for the drive-by hit chance (`None`: on foot).
    fn vehicle_aim(&self, _chr: i32) -> Option<crate::character::firearm::VehicleAim> {
        None
    }
    /// `square.isOutside()`.
    fn square_is_outside(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// The ragdoll controller's `getPelvisPositionX/Y/Z()` for `chr`, when
    /// its animation player has one (pz-ragdoll's `pelvis_position`).
    fn ragdoll_pelvis(&self, _chr: i32) -> Option<[f32; 3]> {
        None
    }

    // ---- the player control loop (#player-control) ----

    /// `UIManager.speedControls.getCurrentGameSpeed()`.
    fn game_speed(&self) -> i32 {
        1
    }
    /// `getDeferredMovement(tempo).getLengthSquared()`: the root motion the
    /// animation has for `chr` this frame (`runningTime`).
    fn deferred_movement_length_sq(&self, _chr: i32) -> f32 {
        0.0
    }
    /// The characters on a square (`getMovingObjects()`).
    fn characters_on_square(&self, _sq: (i32, i32, i32)) -> Vec<i32> {
        Vec::new()
    }
    /// `IsoMovingObject.separate()` for `chr` (pushing apart overlapping
    /// characters, bumps; the world owns the other characters).
    fn separate(&mut self, _chr: &mut GameCharacter<I>) {}
    /// `getPathFindBehavior2().shouldBeMoving()`.
    fn path_should_be_moving(&self, _chr: i32) -> bool {
        false
    }
    /// `getAdvancedAnimator().containsAnyIdleNodes()`.
    fn contains_any_idle_nodes(&self, _chr: i32) -> bool {
        false
    }
    /// `getAnimationPlayer().isReady()` (a loaded skinned model).
    fn animation_player_ready(&self, _chr: i32) -> bool {
        false
    }
    /// The animation player's `getRenderedAngle()` (`None`: not ready).
    fn rendered_angle(&self, _chr: i32) -> Option<f32> {
        None
    }
    /// `isPrimaryHandModelReady()` (the hand item's model is loaded).
    fn primary_hand_model_ready(&self, _chr: i32) -> bool {
        false
    }
    /// `(isInTrees, isInTreesNoBush)` on a square.
    fn square_trees(&self, _sq: (i32, i32, i32)) -> (bool, bool) {
        (false, false)
    }
    /// `BentFences.getCollapsedFence(square) != null`.
    fn collapsed_fence(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// `square.getTree().getSlowFactor(chr)`.
    fn tree_slow_factor(&self, _sq: (i32, i32, i32), _chr: i32) -> Option<f32> {
        None
    }
    /// The ground of a square for the walk speed and dirt.
    fn ground_info(&self, _sq: (i32, i32, i32)) -> crate::character::state::GroundInfo {
        Default::default()
    }
    /// What `updateFootInjuries` reads of the floor (no intact shoes).
    fn foot_injury_floor(&self, _sq: (i32, i32, i32)) -> crate::character::state::FootFloor {
        Default::default()
    }
    /// A vehicle near `chr` has a part it can use (`getUseablePart`):
    /// the context key is the vehicle's.
    fn vehicle_useable_part_near(&self, _chr: i32) -> bool {
        false
    }
    /// `doContextNSWE(dir)`: the action the square edge offers (sheet
    /// rope, door/window/frame, hutch, butcher hook, animal, furniture,
    /// curtain, grappled throws, fence, wall), in the original's order;
    /// at most the first that applies.
    fn context_actions_nswe(
        &mut self,
        _q: &crate::character::player::context::ContextQuery,
        _dir: IsoDirections,
    ) -> Vec<crate::character::player::ContextualAction> {
        Vec::new()
    }
    /// `doContextCorners(dir)` for a diagonal facing.
    fn context_actions_corners(
        &mut self,
        _q: &crate::character::player::context::ContextQuery,
        _dir: IsoDirections,
    ) -> Vec<crate::character::player::ContextualAction> {
        Vec::new()
    }
    /// `getContextDoorOrWindowOrWindowFrame(dir)` then
    /// `doContextDoorOrWindowOrWindowFrame` (the square behind).
    fn context_door_or_window(
        &mut self,
        _q: &crate::character::player::context::ContextQuery,
        _dir: IsoDirections,
    ) -> Option<crate::character::player::ContextualAction> {
        None
    }

    // ---- stairs, slopes and walls (`IsoMovingObject.postupdate`) ----

    /// `square.getSlopedSurfaceHeight(dx, dy)` (0 without a slope).
    fn sloped_surface_height(&self, _sq: (i32, i32, i32), _dx: f32, _dy: f32) -> f32 {
        0.0
    }
    /// `square.getWallType()`.
    fn wall_type(&self, _sq: (i32, i32, i32)) -> i32 {
        0
    }
    /// `IsoMovingObject.checkVaultOver()`'s world part for a player on
    /// `current` (at the square `at` of its position) facing `dir` with the
    /// collided flags `(n, s, w, e)`: the direction it vaults in
    /// (`isPlayerAbleToHopWallTo` and `isSafeToClimbOver` passed) and what
    /// `ClimbOverFenceState.setParams` reads of the square beyond. `None`
    /// when it does not vault.
    fn vault_over(&self, _current: (i32, i32, i32), _at: (i32, i32, i32), _dir: IsoDirections, _collided: (bool, bool, bool, bool)) -> Option<(IsoDirections, FenceClimb)> {
        None
    }

    // ---- windows (`ClimbThroughWindowState`, `climbThroughWindow`) ----

    /// `ClimbThroughWindowState.getClimbThroughWindowPositioningParams(chr,
    /// window, params)`'s world part for `chr` (of kind `kind`) and the
    /// window, window thumpable or window frame `window`. `None` when the
    /// object is none of these (`windowObject` stays null).
    fn window_climb(&self, _chr: &dyn pz_pathfind::mover::CharacterView, _kind: pz_world::square::MoverKind, _window: pz_pathfind::mover::ObjectRef) -> Option<WindowClimb> {
        None
    }
    /// `startSq.getWindowTo(endSq)`, else `getWindowThumpableTo`, else
    /// `getHoppableTo` (`ClimbThroughWindowState.getWindow`), with its
    /// class.
    fn window_between(&self, _start: (i32, i32, i32), _end: (i32, i32, i32)) -> Option<(pz_pathfind::mover::ObjectRef, ThumpKind)> {
        None
    }
    /// `window.getFirstCharacterClosing()` is closing it with
    /// `CloseWindowOutcome` `success`.
    fn window_closing(&self, _window: pz_pathfind::mover::ObjectRef) -> bool {
        false
    }
    /// `obj.rerouteCollide != null` (set on a square's wall when a window
    /// is created on it from the map).
    fn reroute_collide(&self, _obj: pz_pathfind::mover::ObjectRef) -> bool {
        false
    }
    /// `square.getBrokenGlass() != null`.
    fn has_broken_glass(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// The zombies among `square.getMovingObjects()`, in list order (the
    /// character being updated included when it is there).
    fn zombies_on_square(&self, _sq: (i32, i32, i32)) -> Vec<i32> {
        Vec::new()
    }
    /// `IsoThumpable.health -= damage`, then `destroy()` at 0 or below
    /// (`ClimbThroughWindowState`'s `OnFloor`).
    fn damage_thumpable(&mut self, _obj: pz_pathfind::mover::ObjectRef, _damage: i32) {}

    // ---- fences (`ClimbOverFenceState`) ----

    /// `startSq.getHoppableTo(endSq)` (`ClimbOverFenceState.getFence`).
    fn hoppable_between(&self, _start: (i32, i32, i32), _end: (i32, i32, i32)) -> Option<pz_pathfind::mover::ObjectRef> {
        None
    }
    /// What the fence code reads about a hoppable object.
    fn fence_info(&self, _fence: pz_pathfind::mover::ObjectRef) -> Option<FenceInfo> {
        None
    }
    /// `fence.damage = (short)(fence.damage - damage)`, then
    /// `fence.destroyFence(dir)` at 0 or below (two zombies climbing over
    /// it at once, `ClimbOverFenceState`'s `OnFloor`).
    fn damage_fence(&mut self, _fence: pz_pathfind::mover::ObjectRef, _damage: i32, _dir: IsoDirections) {}

    // ---- what animals read and do (`IsoAnimal`, `BaseAnimalBehavior`,
    // the `ai/states/animals` states; the animal-only world objects are
    // `pz-animals`' `AnimalWorld` through `animal_host`) ----

    /// `square.isFree(false)` (the characters on it not counted).
    fn square_is_free_ignoring_characters(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// `a.isBlockedTo(b)`.
    fn is_blocked_to(&self, _a: (i32, i32, i32), _b: (i32, i32, i32)) -> bool {
        false
    }
    /// `a.isWindowTo(b)`.
    fn is_window_to(&self, _a: (i32, i32, i32), _b: (i32, i32, i32)) -> bool {
        false
    }
    /// `a.isWallTo(b)`.
    fn is_wall_to(&self, _a: (i32, i32, i32), _b: (i32, i32, i32)) -> bool {
        false
    }
    /// `square.haveRoof`.
    fn square_have_roof(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// `square.has(IsoFlagType.solidtrans)`.
    fn square_has_solid_trans(&self, _sq: (i32, i32, i32)) -> bool {
        false
    }
    /// The player (by character id) a world sound's source is, when it is
    /// an `IsoPlayer` (`respondToSound`).
    fn sound_source_player(&self, _source: &pz_audio::world_sound::SoundSource) -> Option<i32> {
        None
    }
    /// The world sound's source is an `IsoRadio` or `IsoTelevision`.
    fn sound_source_is_radio_or_television(&self, _source: &pz_audio::world_sound::SoundSource) -> bool {
        false
    }
    /// `IsoAnimal.testCollideWithVehicles(vehicle, null)`'s vehicle part:
    /// `vehicle.shouldCollideWithCharacters()`,
    /// `testCollisionWithCharacter(animal, 0.3, pos) != null`, then
    /// `hitAnimal(animal)` and `applyImpulseFromHitObject(animal, 1)`; true
    /// when it hit.
    fn vehicle_test_collide_animal(&mut self, _vehicle: i16, _animal: &mut GameCharacter<I>) -> bool {
        false
    }
    /// `vehicle.getAnimals()` (character ids).
    fn vehicle_animals(&self, _vehicle: i16) -> Vec<i32> {
        Vec::new()
    }
    /// `getCell().getAnimals()` (character ids).
    fn cell_animals(&self) -> Vec<i32> {
        Vec::new()
    }
    /// `player.hitConsequences(null, animal, false, damage, false)`: an
    /// animal's attack connecting with a player (`AnimalAttackState`).
    fn animal_hit_player(&mut self, _player: i32, _animal: &mut GameCharacter<I>, _damage: f32) {}
    /// `IsoThumpable.animalHit(animal)` and the thump sound
    /// (`AnimalAttackState`); then `destroy()` when its health is at or
    /// below 0. Answers whether it was destroyed, and whether it is a door
    /// that is open afterwards.
    fn thumpable_animal_hit(&mut self, _obj: pz_pathfind::mover::ObjectRef, _animal: i32) -> (bool, bool) {
        (false, false)
    }
    /// `obj.Damage(damage)` and the thump sound (an animal thumping a door
    /// or another object).
    fn object_damage(&mut self, _obj: pz_pathfind::mover::ObjectRef, _damage: f32) {}

    // ---- windows, sheet ropes and walls (#208; the climb states are #204) ----

    /// The local player's input this frame, for the states' own
    /// `pressedMovement` / `pressedCancelAction` (`None`: nothing pressed).
    fn player_input(&self) -> Option<&dyn crate::character::player::PlayerInput> {
        None
    }
    /// An `IsoWindow` (`None`: no window, or not an `IsoWindow`).
    fn window_info(&self, _window: pz_pathfind::mover::ObjectRef) -> Option<WindowInfo> {
        None
    }
    /// `IsoWindow.ToggleWindow(chr)`.
    fn toggle_window(&mut self, _window: pz_pathfind::mover::ObjectRef, _chr: i32) {}
    /// `window.setPermaLocked(perma)` then `window.sync()`.
    fn set_window_perma_locked(&mut self, _window: pz_pathfind::mover::ObjectRef, _perma: bool) {}
    /// `IsoWindow.WeaponHit(chr, null)` (bare hands smash a window).
    fn window_weapon_hit(&mut self, _window: pz_pathfind::mover::ObjectRef, _chr: &GameCharacter<I>) {}
    /// `VehicleWindow.isDestroyed()` of a vehicle part (`None`: no window).
    fn vehicle_window_destroyed(&self, _vehicle: i16, _part: &str) -> Option<bool> {
        None
    }
    /// `VehicleWindow.hit(chr)`.
    fn vehicle_window_hit(&mut self, _vehicle: i16, _part: &str, _chr: &GameCharacter<I>) {}
    /// The sheet-rope view of a square (`None`: no square).
    fn sheet_rope_square(&self, _sq: (i32, i32, i32)) -> Option<SheetRopeSquare> {
        None
    }
    /// `square.getChunk().getMinLevel()` / `getMaxLevel()`.
    fn chunk_levels(&self, _sq: (i32, i32, i32)) -> (i32, i32) {
        (0, 0)
    }
    /// `ClimbSheetRopeState.calculateClimbOutcome`: what the top of a rope
    /// at `target` leads to when facing `dir` (a shut window there is
    /// opened, `ToggleWindow`).
    fn sheet_rope_exit(&mut self, _chr: &GameCharacter<I>, _target: (i32, i32, i32), _dir: IsoDirections) -> SheetRopeExit {
        SheetRopeExit::Undefined
    }
    /// `IsoPlayer.canClimbOverWall(dir)` without its `isSprinting()` test.
    fn can_climb_over_wall(&self, _chr: &GameCharacter<I>, _dir: IsoDirections) -> bool {
        false
    }
    /// `ClimbOverWallState.getFenceType(getFence(owner))` (`None`: no
    /// climbable wall between the squares).
    fn climb_wall_fence_type(&self, _start: (i32, i32, i32), _end: (i32, i32), _dir: IsoDirections) -> Option<i32> {
        None
    }

    // ---- falls (`handleLandingImpact`, #204) ----

    /// `sq.getFloor().getSprite().getName()`.
    fn floor_sprite_name(&self, _sq: (i32, i32, i32)) -> Option<String> {
        None
    }
}

/// What the window states read of an `IsoWindow`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowInfo {
    /// `getSquare()`.
    pub square: (i32, i32, i32),
    /// `isNorth()`.
    pub north: bool,
    /// `getObjectIndex() != -1`.
    pub in_world: bool,
    pub locked: bool,
    pub perma_locked: bool,
    /// `IsOpen()`.
    pub open: bool,
    /// `isDestroyed()`.
    pub destroyed: bool,
    /// `isInvincible()`.
    pub invincible: bool,
}

impl WindowInfo {
    /// `getOppositeSquare()`.
    pub fn opposite(&self) -> (i32, i32, i32) {
        let (x, y, z) = self.square;
        if self.north {
            (x, y - 1, z)
        } else {
            (x - 1, y, z)
        }
    }
}

/// A square as the sheet-rope states read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SheetRopeSquare {
    /// `IsoWindow.isSheetRopeHere(sq)`.
    pub sheet_rope: bool,
    /// `IsoWindow.canClimbHere(sq)`.
    pub can_climb_here: bool,
    /// `IsoWindow.isTopOfSheetRopeHere(sq)`.
    pub top: bool,
    /// `TreatAsSolidFloor()`.
    pub solid_floor: bool,
    /// `has(IsoFlagType.solidtrans)`.
    pub solidtrans: bool,
    /// `climbSheetN || climbSheetTopN`, and the other sides.
    pub n: bool,
    pub s: bool,
    pub w: bool,
    pub e: bool,
}

/// `ClimbSheetRopeState.ClimbStatus`, with the object it climbs through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SheetRopeExit {
    #[default]
    Undefined,
    Blocked,
    /// An `IsoWindow` or a window `IsoThumpable`.
    OpenWindow(pz_pathfind::mover::ObjectRef),
    WindowFrame(pz_pathfind::mover::ObjectRef),
    Fence,
}

/// The ground of a square (`hasNaturalFloor`, `hasSand`,
/// `getPuddlesInGround`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroundInfo {
    pub natural_floor: bool,
    pub sand: bool,
    pub puddles_in_ground: f32,
}

/// `updateFootInjuries`' floor: `getBrokenGlass() != null` and the
/// modifier (2: natural floor in a forest zone, 1: other floor outside a
/// building, 0: none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FootFloor {
    pub broken_glass: bool,
    pub modifier: i32,
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

/// What `getClimbThroughWindowPositioningParams` reads of the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowClimb {
    pub portal: ThumpKind,
    /// `canClimbThrough(chr)` of the window or thumpable (`true` for a
    /// frame: the positioning params do not ask it).
    pub can_climb: bool,
    /// `IsoWindowFrame.canClimbThrough(chr)` for a frame
    /// (`climbThroughWindowFrame`), else `can_climb`.
    pub portal_can_climb: bool,
    /// `IsoWindow`: `isDestroyed() && !isGlassRemoved()` (a player may be
    /// scratched).
    pub broken_glass: bool,
    /// `IsoThumpable`: `getName().equals("Barbed Fence")`.
    pub barbed: bool,
    pub north: bool,
    /// The object's square.
    pub square: (i32, i32, i32),
    pub climb_dir: IsoDirections,
    pub start: (i32, i32),
    pub opposite: (i32, i32),
    pub end: (i32, i32),
    pub z: i32,
    pub counter: bool,
    pub floor: bool,
    pub sheet_rope: bool,
}

/// What `ClimbOverFenceState` reads of a hoppable.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FenceInfo {
    /// The fence's square.
    pub square: (i32, i32, i32),
    /// `getProperties().has(HoppableN)`.
    pub hoppable_n: bool,
    /// `isMetalFence(fence)`.
    pub metal: bool,
    /// `ParameterFenceTypeLow.FenceType` value of `getFenceType(fence)`.
    pub fence_type_low: i32,
    /// `ParameterTripObstacleType.ObstacleType` value of `getTripType(fence)`.
    pub trip_type: i32,
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
    // One immediate first-fault carrier per outer action. Reborrows share
    // the same fault, so nested void State callbacks cannot hide it.
    host_fault: Rc<RefCell<Option<RuntimeHostError>>>,
}

impl<'a, I> StateCx<'a, I> {
    pub fn new(sm: &'a ScriptManager, chars: &'a Characters, ctx: &'a dyn SimContext, world: &'a mut dyn StateWorld<I>) -> Self {
        StateCx { sm, chars, ctx, world, host_fault: Rc::new(RefCell::new(None)) }
    }

    /// The same context with a shorter borrow (to pass on).
    pub fn reborrow(&mut self) -> StateCx<'_, I> {
        StateCx { sm: self.sm, chars: self.chars, ctx: self.ctx, world: &mut *self.world, host_fault: self.host_fault.clone() }
    }

    /// Stop this action at its first host failure. No callback/event queue is
    /// involved, and later failures must not replace the original domain.
    pub fn record_host_error(&self, error: RuntimeHostError) {
        let mut fault = self.host_fault.borrow_mut();
        if fault.is_none() { *fault = Some(error); }
    }

    pub fn has_host_error(&self) -> bool { self.host_fault.borrow().is_some() }

    /// Every production outer engine/Lua action boundary must return this
    /// result before any further action/state work. Inspecting never clears
    /// the first fault or turns failure into native absence.
    pub fn host_result(&self) -> HostResult<()> {
        match &*self.host_fault.borrow() {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}
