//! What the Java pathfinding layer reads and calls on the moving character.
//!
//! `zombie.pathfind.PathFindBehavior2`, `nativeCode.PathFindRequest.init`,
//! `nativeCode.PathRequestTask.init` and `nativeCode.PathfindNative.findPath`
//! work on an `IsoGameCharacter` (an `IsoPlayer`, `IsoZombie` or `IsoAnimal`,
//! the last being a subclass of `IsoPlayer`). The character port implements
//! [`PathMover`] for its types; this crate never sees them.
//!
//! The Java code tests the character's class with `instanceof`; here that is
//! [`PathMover::as_player`], [`PathMover::as_zombie`] and
//! [`PathMover::as_animal`]. Like the Java class hierarchy, an animal is also
//! a player: an `IsoAnimal` returns `Some` from both `as_player` and
//! `as_animal`. Code that wants "a player that is not an animal" (Java's
//! `instanceof IsoPlayer && !(instanceof IsoAnimal)`) checks both.
//!
//! Objects the character interacts with (doors, windows, window frames) are
//! named by an [`ObjectRef`] into `pz-world`'s squares. Vehicles are named by
//! their `BaseVehicle.vehicleId`.
//!
//! Game-wide settings the same code reads (`SandboxOptions`, `GameTime`,
//! `GameClient`/`GameServer`, `BentFences`) are in [`PathWorldHooks`].

use pz_world::objects::ObjectId;
use pz_world::IsoDirections;

use crate::path::Path;

/// Identity of a `Mover` (`zombie.ai.astar.Mover`). The Java side keys its
/// request map by object identity; the port uses a stable id the character
/// port hands out (for example `Mover.getID()` plus a kind tag).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MoverId(pub u64);

/// `AStarPathFinder.PathFindProgress`, the state of
/// `IsoGameCharacter.getFinder().progress`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PathFindProgress {
    #[default]
    NotRunning,
    Failed,
    Found,
    NotYetFound,
}

/// The `zombie.ai.State` instances `PathFindBehavior2` compares the
/// character's states against. Any other state is [`CharacterState::Other`]
/// with an id of the implementor's choosing, so that two different states
/// compare unequal (`PathFindBehavior2.update` checks whether the current
/// state changed during the update).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CharacterState {
    /// `ClimbOverFenceState`.
    ClimbOverFence,
    /// `ClimbThroughWindowState`.
    ClimbThroughWindow,
    /// `PlayerSitOnFurnitureState`.
    PlayerSitOnFurniture,
    /// `CollideWithWallState`.
    CollideWithWall,
    /// `LungeNetworkState`.
    LungeNetwork,
    /// `WalkTowardState`.
    WalkToward,
    /// `ZombieGetDownState`.
    ZombieGetDown,
    /// Any other state (or none).
    Other(u32),
}

/// An `IsoObject` on a square of the world: the square's coordinates and the
/// object's `pz-world` identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectRef {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub id: ObjectId,
}

/// What `IsoGameCharacter.faceThisObject(IsoObject)` is given by
/// `PathFindBehavior2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceTarget {
    /// A `BaseVehicle`, by `vehicleId`.
    Vehicle(i16),
    /// An `IsoWindow` or window `IsoThumpable`.
    Object(ObjectRef),
}

/// The part of a character another character's pathfinding reads: the
/// mover itself, and the goal character of `PathFindBehavior2.pathToCharacter`.
pub trait CharacterView {
    /// `IsoMovingObject.getX()`.
    fn x(&self) -> f32;
    /// `IsoMovingObject.getY()`.
    fn y(&self) -> f32;
    /// `IsoMovingObject.getZ()`.
    fn z(&self) -> f32;
    /// `IsoMovingObject.getCurrentSquare()` / `getSquare()`: the
    /// coordinates of the square the character stands on, if any.
    fn current_square(&self) -> Option<(i32, i32, i32)>;
    /// `IsoGameCharacter.getVehicle()`: the vehicle the character is in, by
    /// `vehicleId`.
    fn vehicle(&self) -> Option<i16>;
    /// `IsoGameCharacter.isSittingOnFurniture()`.
    fn is_sitting_on_furniture(&self) -> bool;
    /// `IsoGameCharacter.getSitOnFurnitureDirection()`.
    fn sit_on_furniture_direction(&self) -> Option<IsoDirections>;
}

/// The moving character (`IsoGameCharacter` as a `Mover`).
pub trait PathMover: CharacterView {
    // ----- identity and class -----

    /// The `Mover`'s identity (the key of `PathfindNativeThread.requestMap`).
    fn mover_id(&self) -> MoverId;

    /// `this instanceof IsoPlayer` (true for animals too, see the module doc).
    fn as_player(&self) -> Option<&dyn PlayerMover>;
    /// Mutable [`PathMover::as_player`].
    fn as_player_mut(&mut self) -> Option<&mut dyn PlayerMover>;
    /// `this instanceof IsoZombie`.
    fn as_zombie(&self) -> Option<&dyn ZombieMover>;
    /// Mutable [`PathMover::as_zombie`].
    fn as_zombie_mut(&mut self) -> Option<&mut dyn ZombieMover>;
    /// `this instanceof IsoAnimal`.
    fn as_animal(&self) -> Option<&dyn AnimalMover>;
    /// Mutable [`PathMover::as_animal`].
    fn as_animal_mut(&mut self) -> Option<&mut dyn AnimalMover>;

    /// `IsoGameCharacter.isAnimal()`.
    fn is_animal(&self) -> bool {
        self.as_animal().is_some()
    }
    /// `IsoGameCharacter.isZombie()`.
    fn is_zombie(&self) -> bool {
        self.as_zombie().is_some()
    }

    // ----- path finder state -----

    /// `getFinder().progress`.
    fn path_find_progress(&self) -> PathFindProgress;
    /// `getFinder().progress = p`.
    fn set_path_find_progress(&mut self, p: PathFindProgress);
    /// `IsoGameCharacter.setPath2(path)`. Java stores the behaviour's own
    /// `Path` object; the port's `PathFindBehavior2` owns the path and hands
    /// it here whenever Java would (re)assign it. `None` is `setPath2(null)`.
    fn set_path2(&mut self, path: Option<&Path>);
    /// `IsoGameCharacter.getPath2() != null`.
    fn has_path2(&self) -> bool;

    // ----- states -----

    /// `IsoGameCharacter.getCurrentState()`.
    fn current_state(&self) -> CharacterState;
    /// `IsoGameCharacter.isCurrentState(state)` (the current state or one of
    /// its sub-states).
    fn is_current_state(&self, state: CharacterState) -> bool;
    /// `IsoGameCharacter.getStateMachine().getPrevious()`.
    fn previous_state(&self) -> CharacterState;

    // ----- movement -----

    /// `IsoGameCharacter.getDeferredMovement(out)`: the animation's root
    /// motion this frame, `(x, y)`.
    fn deferred_movement(&self) -> (f32, f32);
    /// `IsoMovingObject.moveUnmodded(dx, dy)`.
    fn move_unmodded(&mut self, dx: f32, dy: f32);
    /// `IsoGameCharacter.setMoving(moving)`.
    fn set_moving(&mut self, moving: bool);
    /// `IsoGameCharacter.isAnimationUpdatingThisFrame()`.
    fn is_animation_updating_this_frame(&self) -> bool;
    /// `IsoGameCharacter.isPerformingNoAimShortStrafe()`.
    fn is_performing_no_aim_short_strafe(&self) -> bool;
    /// `IsoGameCharacter.set(PlayerStrafeState.STRAFE_SPEED, speed)` (the
    /// state's per-character float slot; `PathFindBehavior2` also sets the
    /// `StrafeSpeed` animation variable through
    /// [`PathMover::set_variable_float`]).
    fn set_strafe_state_speed(&mut self, speed: f32);

    // ----- facing -----

    /// `IsoGameCharacter.getForwardDirection()`, `(x, y)`.
    fn forward_direction(&self) -> (f32, f32);
    /// `IsoGameCharacter.setForwardDirection(x, y)`.
    fn set_forward_direction(&mut self, x: f32, y: f32);
    /// `IsoGameCharacter.getForwardDirection().normalize()` (in place,
    /// `Vector2.normalize`).
    fn normalize_forward_direction(&mut self);
    /// `IsoGameCharacter.DirectionFromVector(v)`.
    fn direction_from_vector(&mut self, x: f32, y: f32);
    /// `IsoGameCharacter.faceLocation(x, y)`.
    fn face_location(&mut self, x: f32, y: f32);
    /// `IsoGameCharacter.faceLocationF(x, y)`.
    fn face_location_f(&mut self, x: f32, y: f32);
    /// `IsoGameCharacter.faceDirection(dir)`.
    fn face_direction(&mut self, dir: IsoDirections);
    /// `IsoGameCharacter.faceThisObject(object)`.
    fn face_this_object(&mut self, target: FaceTarget);
    /// `IsoGameCharacter.shouldBeTurning()` (`IsoPlayer` overrides it).
    fn should_be_turning(&self) -> bool;
    /// `IsoGameCharacter.isAiming()`.
    fn is_aiming(&self) -> bool;
    /// `IsoGameCharacter.isAnimatingBackwards()`.
    fn is_animating_backwards(&self) -> bool;
    /// `AnimationPlayer ap = getAnimationPlayer(); if (ap != null &&
    /// ap.isReady()) ap.updateForwardDirection(this);`
    fn update_animation_forward_direction(&mut self);

    // ----- collision -----

    /// `IsoGameCharacter.setCollidable(collidable)`.
    fn set_collidable(&mut self, collidable: bool);
    /// `square.testCollideAdjacent(this, dx, dy, dz)` for the square at
    /// `from`. This is the mover-aware overload (no-clip players,
    /// `shouldIgnoreCollisionWithSquare`, special objects' `TestCollide`
    /// with `collideWith`/`setCollidedWithDoor`/`setCollidedObject` side
    /// effects, safehouse trespass); it belongs to the character's collision
    /// port. `PathFindBehavior2.closestPointOnPath` calls it with a zombie's
    /// `ghost` flag forced on ([`ZombieMover::set_ghost`]).
    fn test_collide_adjacent(&mut self, from: (i32, i32, i32), dx: i32, dy: i32, dz: i32) -> bool;

    // ----- obstacles -----

    /// `IsoGameCharacter.climbOverFence(dir)`.
    fn climb_over_fence(&mut self, dir: IsoDirections);
    /// `IsoGameCharacter.climbThroughWindow(IsoWindow)` or
    /// `climbThroughWindow(IsoThumpable)`, by the object's class.
    fn climb_through_window(&mut self, window: ObjectRef);
    /// `IsoGameCharacter.climbThroughWindowFrame(IsoWindowFrame)`.
    fn climb_through_window_frame(&mut self, frame: ObjectRef);
    /// `IsoWindow.canClimbThrough(this)`.
    fn can_climb_through_window(&self, window: ObjectRef) -> bool;
    /// `IsoDoor.couldBeOpen(this)` / `IsoThumpable.couldBeOpen(this)`.
    fn could_open_door(&self, door: ObjectRef) -> bool;
    /// `IsoDoor.ToggleDoor(this)` / `IsoThumpable.ToggleDoor(this)`, then
    /// `door.isOpen()` / `door.open`: whether the door is open afterwards
    /// (`PathFindBehavior2` fails the path when it is still closed). The
    /// toggle must be visible to the answer, so it cannot be deferred.
    fn toggle_door(&mut self, door: ObjectRef) -> bool;

    // ----- animation variables -----

    /// `IsoGameCharacter.getVariableBoolean(name)`.
    fn get_variable_boolean(&self, name: &str) -> bool;
    /// `IsoGameCharacter.setVariable(name, boolean)`.
    fn set_variable_bool(&mut self, name: &str, value: bool);
    /// `IsoGameCharacter.setVariable(name, float)`.
    fn set_variable_float(&mut self, name: &str, value: f32);
}

/// `IsoPlayer` (including `IsoAnimal`, a subclass).
pub trait PlayerMover {
    /// `IsoPlayer.isNpc()`.
    fn is_npc(&self) -> bool;
    /// `IsoPlayer.isLocalPlayer()`.
    fn is_local_player(&self) -> bool;
    /// `IsoPlayer.isLocal()`.
    fn is_local(&self) -> bool;
    /// `IsoPlayer.timeSinceCloseDoor`.
    fn time_since_close_door(&self) -> f32;
    /// `IsoGameCharacter.getClimbingFailChanceFloat()` (only asked for
    /// players).
    fn climbing_fail_chance_float(&self) -> f32;
    /// `IsoPlayer.climbOverWall(dir)`.
    fn climb_over_wall(&mut self, dir: IsoDirections);
}

/// `IsoZombie`.
pub trait ZombieMover {
    /// `IsoZombie.isCrawling()`.
    fn is_crawling(&self) -> bool;
    /// `IsoZombie.isCanWalk()`.
    fn is_can_walk(&self) -> bool;
    /// `IsoZombie.isCanCrawlUnderVehicle()`.
    fn is_can_crawl_under_vehicle(&self) -> bool;
    /// `IsoZombie.getTarget() != null` (also the `target` field
    /// `PathRequestTask.init` reads).
    fn has_target(&self) -> bool;
    /// `IsoZombie.isMovingToPlayerSound()`.
    fn is_moving_to_player_sound(&self) -> bool;
    /// `IsoZombie.isRemoteZombie()`.
    fn is_remote_zombie(&self) -> bool;
    /// `IsoZombie.allowRepathDelay`.
    fn allow_repath_delay(&self) -> f32;
    /// `IsoZombie.allowRepathDelay = v`.
    fn set_allow_repath_delay(&mut self, v: f32);
    /// `IsoZombie.running = running`.
    fn set_running(&mut self, running: bool);
    /// `IsoZombie.ghost`.
    fn is_ghost(&self) -> bool;
    /// `IsoZombie.ghost = ghost`.
    fn set_ghost(&mut self, ghost: bool);
    /// `ZombieGetDownState.instance().isNearStartXY(this)`.
    fn get_down_is_near_start_xy(&self) -> bool;
    /// `ZombieGetDownState.instance().setParams(this)`.
    fn get_down_set_params(&mut self);
}

/// `IsoAnimal`.
pub trait AnimalMover {
    /// `IsoAnimal.shouldBreakObstaclesDuringPathfinding()`.
    fn should_break_obstacles_during_pathfinding(&self) -> bool;
    /// `IsoAnimal.canClimbFences()`.
    fn can_climb_fences(&self) -> bool;
    /// `IsoAnimal.adef.animalSize`.
    fn animal_size(&self) -> f32;
}

/// Game-wide state the Java pathfinding layer reads.
pub trait PathWorldHooks {
    /// `SandboxOptions.instance.lore.speed.getValue()` (1 is sprinters).
    fn lore_speed(&self) -> i32;
    /// `SandboxOptions.instance.easyClimbing.getValue()`.
    fn easy_climbing(&self) -> bool;
    /// `BentFences.getInstance().isEnabled()`.
    fn bent_fences_enabled(&self) -> bool;
    /// `GameClient.client`.
    fn is_client(&self) -> bool;
    /// `GameServer.server`.
    fn is_server(&self) -> bool;
    /// `GameTime.getInstance().getMultiplier()`.
    fn game_time_multiplier(&self) -> f32;
    /// `GameTime.getInstance().getThirtyFPSMultiplier()`.
    fn thirty_fps_multiplier(&self) -> f32;
    /// `SpeedControls.instance.getCurrentGameSpeed()`.
    fn current_game_speed(&self) -> i32;
}

/// The mover class `PathfindNative.findPath` writes into the request (and
/// the queue `PathRequestTask.init` picks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoverClass {
    /// `IsoPlayer` that is not an `IsoAnimal`.
    Player,
    /// `IsoZombie`.
    Zombie,
    /// `IsoAnimal`.
    Animal,
    /// Any other `Mover`.
    Other,
}

/// What `PathFindRequest.init`, `PathRequestTask.init` and
/// `PathfindNative.findPath` read from the mover and the game when a request
/// is added. The Java side reads these on the main thread at `addRequest`
/// time, except `isNpc()`, which `findPath` reads on the pathfinding thread;
/// the port captures it at `addRequest` too.
#[derive(Clone, Debug, PartialEq)]
pub struct MoverSnapshot {
    pub id: MoverId,
    pub class: MoverClass,
    /// `IsoPlayer.isNpc()` (players only).
    pub is_npc: bool,
    pub can_crawl: bool,
    pub crawling: bool,
    pub ignore_crawl_cost: bool,
    pub can_thump: bool,
    pub can_climb_fences: bool,
    pub can_climb_tall_fences: bool,
    pub avoid_farming_plants: bool,
    pub has_target: bool,
    pub can_bend: bool,
    /// `PathRequestTask.queueNumber`: 1 player, 2 zombie with a target,
    /// 3 anything else.
    pub queue: u8,
}

impl MoverSnapshot {
    /// `PathFindRequest.init(finder, mover, ...)`'s flag part, plus the
    /// queue of `PathRequestTask.init` and the mover type and NPC flag of
    /// `PathfindNative.findPath`.
    pub fn capture(mover: &dyn PathMover, world: &dyn PathWorldHooks) -> MoverSnapshot {
        let mut s = MoverSnapshot {
            id: mover.mover_id(),
            class: MoverClass::Other,
            is_npc: false,
            can_crawl: false,
            crawling: false,
            ignore_crawl_cost: false,
            can_thump: false,
            can_climb_fences: false,
            can_climb_tall_fences: false,
            avoid_farming_plants: false,
            has_target: false,
            can_bend: false,
            queue: 3,
        };
        if let Some(a) = mover.as_animal() {
            s.class = MoverClass::Animal;
            s.can_thump = a.should_break_obstacles_during_pathfinding();
            s.can_climb_fences = a.can_climb_fences();
        }
        if let Some(z) = mover.as_zombie() {
            s.class = MoverClass::Zombie;
            s.can_crawl = z.is_crawling() || z.is_can_crawl_under_vehicle();
            s.crawling = z.is_crawling();
            s.ignore_crawl_cost = z.is_crawling() && !z.is_can_walk();
            s.can_thump = true;
            s.has_target = z.has_target() || z.is_moving_to_player_sound();
            s.can_bend = world.bent_fences_enabled();
            if z.has_target() {
                s.queue = 2;
            }
        }
        if mover.as_animal().is_none() {
            if let Some(p) = mover.as_player() {
                s.class = MoverClass::Player;
                s.can_climb_tall_fences =
                    world.easy_climbing() || p.climbing_fail_chance_float() >= 1.0;
                s.avoid_farming_plants = true;
                s.is_npc = p.is_npc();
                s.queue = 1;
            }
        }
        s
    }
}
