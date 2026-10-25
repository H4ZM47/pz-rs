//! `pz_pathfind::mover`'s character side: [`PathMover`] and
//! [`PlayerMover`] for [`GameCharacter`] (the zombie and animal views are
//! the zombie and animal ports'; until then they are `None`).
//!
//! The obstacle calls (`climbOverFence`, `climbThroughWindow`,
//! `ToggleDoor`, ...) need the world's objects; they are queued in
//! [`PathMoverState::requests`]. The climbs run right after the path
//! update that asked for them ([`GameCharacter::run_path_requests`]); the
//! rest is for the owner of the world.

use pz_pathfind::mover::{
    AnimalMover, CharacterState, CharacterView, FaceTarget, MoverId, ObjectRef, PathFindProgress, PathMover, PlayerMover,
    ZombieMover,
};
use pz_pathfind::Path;
use pz_world::square::IsoDirections;

use super::state::StateId;
use super::states::ids;
use super::{CharacterKind, GameCharacter};
use pz_anim::AnimationVariableSource;

/// An obstacle call `PathFindBehavior2` made on the character.
#[derive(Clone, Debug, PartialEq)]
pub enum PathMoverRequest {
    ClimbOverFence(IsoDirections),
    ClimbThroughWindow(ObjectRef),
    ClimbThroughWindowFrame(ObjectRef),
    ToggleDoor(ObjectRef),
    ClimbOverWall(IsoDirections),
    FaceObject(FaceTarget),
}

/// The pathfinding fields of `IsoGameCharacter` (`finder`, `path2`) and
/// the inputs other systems supply.
#[derive(Clone, Debug, Default)]
pub struct PathMoverState {
    /// `getFinder().progress`.
    pub progress: PathFindProgress,
    /// `path2`.
    pub path2: Option<Path>,
    /// `collidable`.
    pub collidable: bool,
    /// `getDeferredMovement` (from the animation player).
    pub deferred_movement: (f32, f32),
    /// The current square's coordinates (`getCurrentSquare()`).
    pub current_square: Option<(i32, i32, i32)>,
    /// `getVehicle()` by id.
    pub vehicle: Option<i16>,
    /// `getSitOnFurnitureDirection()`.
    pub sit_on_furniture_direction: Option<IsoDirections>,
    /// `IsoPlayer.timeSinceCloseDoor`.
    pub time_since_close_door: f32,
    /// `getClimbingFailChanceFloat()` as the last update computed it (the
    /// getter needs the perk and trait statics).
    pub climbing_fail_chance: f32,
    /// `isNpc()`.
    pub npc: bool,
    /// Obstacle calls for the world's owner, oldest first.
    pub requests: Vec<PathMoverRequest>,
    /// Toggles a door and answers whether it is open afterwards.
    pub door_hook: Option<DoorHook>,
    /// A world hook for `IsoWindow.canClimbThrough(chr)`
    /// (`checkDoorHoppableWindow`).
    pub window_hook: Option<WindowHook>,
}

/// A world hook for `IsoWindow.canClimbThrough(chr)` of this character.
#[derive(Clone)]
pub struct WindowHook(pub std::rc::Rc<dyn Fn(ObjectRef) -> bool>);

impl std::fmt::Debug for WindowHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WindowHook")
    }
}

/// A world hook for `IsoDoor`/`IsoThumpable.ToggleDoor(chr)` + `isOpen()`.
#[derive(Clone)]
pub struct DoorHook(pub std::rc::Rc<dyn Fn(ObjectRef) -> bool>);

impl std::fmt::Debug for DoorHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DoorHook")
    }
}

impl<I: 'static> GameCharacter<I> {
    /// Performs the climbs `PathFindBehavior2.update()` asked for
    /// (`climbOverFence`, `climbThroughWindow`, `climbThroughWindowFrame`,
    /// `IsoPlayer.climbOverWall`),
    /// oldest first; the other requests stay queued. Call it after each
    /// `StateWorld::path_update` of the character.
    pub fn run_path_requests(&mut self, cx: &mut super::state::StateCx<'_, I>) {
        if !self.path.requests.iter().any(|r| {
            matches!(
                r,
                PathMoverRequest::ClimbOverFence(_)
                    | PathMoverRequest::ClimbThroughWindow(_)
                    | PathMoverRequest::ClimbThroughWindowFrame(_)
                    | PathMoverRequest::ClimbOverWall(_)
            )
        }) {
            return;
        }
        let all = std::mem::take(&mut self.path.requests);
        let mut rest = Vec::new();
        for r in all {
            match r {
                PathMoverRequest::ClimbOverFence(d) => self.do_climb_over_fence(d, cx),
                PathMoverRequest::ClimbThroughWindow(w) => self.climb_through_window(w, cx),
                PathMoverRequest::ClimbThroughWindowFrame(f) => self.climb_through_window_frame(f, cx),
                PathMoverRequest::ClimbOverWall(d) => {
                    self.climb_over_wall(d, cx);
                }
                other => rest.push(other),
            }
        }
        rest.append(&mut self.path.requests);
        self.path.requests = rest;
    }
}

/// The `CharacterState` of a state class.
pub fn character_state(id: Option<StateId>) -> CharacterState {
    let Some(id) = id else { return CharacterState::Other(0) };
    match id {
        ids::CLIMB_OVER_FENCE => CharacterState::ClimbOverFence,
        ids::CLIMB_THROUGH_WINDOW => CharacterState::ClimbThroughWindow,
        ids::PLAYER_SIT_ON_FURNITURE => CharacterState::PlayerSitOnFurniture,
        ids::COLLIDE_WITH_WALL => CharacterState::CollideWithWall,
        ids::LUNGE_NETWORK => CharacterState::LungeNetwork,
        ids::WALK_TOWARD => CharacterState::WalkToward,
        ids::ZOMBIE_GET_DOWN => CharacterState::ZombieGetDown,
        // Distinct per class: a stable hash of the class name.
        StateId(name) => {
            let h = name.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
            CharacterState::Other(h | 1)
        }
    }
}

fn state_id_of(s: CharacterState) -> Option<StateId> {
    Some(match s {
        CharacterState::ClimbOverFence => ids::CLIMB_OVER_FENCE,
        CharacterState::ClimbThroughWindow => ids::CLIMB_THROUGH_WINDOW,
        CharacterState::PlayerSitOnFurniture => ids::PLAYER_SIT_ON_FURNITURE,
        CharacterState::CollideWithWall => ids::COLLIDE_WITH_WALL,
        CharacterState::LungeNetwork => ids::LUNGE_NETWORK,
        CharacterState::WalkToward => ids::WALK_TOWARD,
        CharacterState::ZombieGetDown => ids::ZOMBIE_GET_DOWN,
        CharacterState::Other(_) => return None,
    })
}

impl<I: 'static> CharacterView for GameCharacter<I> {
    fn is_on_floor(&self) -> bool {
        self.vars.on_floor
    }
    fn is_dead(&self) -> bool {
        GameCharacter::is_dead(self)
    }
    fn is_reanimated_for_grappling(&self) -> bool {
        self.is_zombie() && self.zombie.reanimated_for_grapple_only
    }
    fn x(&self) -> f32 {
        self.vars.x
    }
    fn y(&self) -> f32 {
        self.vars.y
    }
    fn z(&self) -> f32 {
        self.vars.z
    }
    fn current_square(&self) -> Option<(i32, i32, i32)> {
        self.path.current_square
    }
    fn vehicle(&self) -> Option<i16> {
        self.path.vehicle
    }
    fn is_sitting_on_furniture(&self) -> bool {
        self.sitting_on_furniture
    }
    fn sit_on_furniture_direction(&self) -> Option<IsoDirections> {
        self.path.sit_on_furniture_direction
    }
}

impl<I: 'static> PathMover for GameCharacter<I> {
    fn mover_id(&self) -> MoverId {
        let kind = match self.kind {
            CharacterKind::Player => 1u64,
            CharacterKind::Survivor => 2,
            CharacterKind::Zombie => 3,
            CharacterKind::Animal => 4,
        };
        MoverId((kind << 32) | self.id as u32 as u64)
    }
    fn as_player(&self) -> Option<&dyn PlayerMover> {
        matches!(self.kind, CharacterKind::Player | CharacterKind::Animal).then_some(self as &dyn PlayerMover)
    }
    fn as_player_mut(&mut self) -> Option<&mut dyn PlayerMover> {
        if matches!(self.kind, CharacterKind::Player | CharacterKind::Animal) {
            Some(self)
        } else {
            None
        }
    }
    /// `ZombieMover` for zombies (`zombie::mover`).
    fn as_zombie(&self) -> Option<&dyn ZombieMover> {
        if self.is_zombie() {
            Some(self)
        } else {
            None
        }
    }
    fn as_zombie_mut(&mut self) -> Option<&mut dyn ZombieMover> {
        if self.is_zombie() {
            Some(self)
        } else {
            None
        }
    }
    /// TODO(animal port).
    fn as_animal(&self) -> Option<&dyn AnimalMover> {
        None
    }
    fn as_animal_mut(&mut self) -> Option<&mut dyn AnimalMover> {
        None
    }
    fn is_animal(&self) -> bool {
        self.kind == CharacterKind::Animal
    }
    fn is_zombie(&self) -> bool {
        self.kind == CharacterKind::Zombie
    }

    fn path_find_progress(&self) -> PathFindProgress {
        self.path.progress
    }
    fn set_path_find_progress(&mut self, p: PathFindProgress) {
        self.path.progress = p;
    }
    fn set_path2(&mut self, path: Option<&Path>) {
        self.path.path2 = path.cloned();
    }
    fn has_path2(&self) -> bool {
        self.path.path2.is_some()
    }

    fn current_state(&self) -> CharacterState {
        character_state(self.current_state_id())
    }
    fn is_current_state(&self, state: CharacterState) -> bool {
        match state_id_of(state) {
            Some(id) => self.is_current_state_id(id),
            None => character_state(self.current_state_id()) == state,
        }
    }
    fn previous_state(&self) -> CharacterState {
        character_state(self.state.machine.previous().map(|s| s.id()))
    }

    fn deferred_movement(&self) -> (f32, f32) {
        self.path.deferred_movement
    }
    /// `moveUnmodded(dx, dy)`: the next position (collision is the world's).
    /// `moveUnmodded(dx, dy)`: the next position (`moveUnmoddedInternal`;
    /// the move itself is `postupdate`'s). IsoZombie's sprinter clamp
    /// applies in the attack states only, which move through
    /// `doDeferredMovement` ([`GameCharacter::move_unmodded_step`]).
    fn move_unmodded(&mut self, dx: f32, dy: f32) {
        self.vars.next_x += dx;
        self.vars.next_y += dy;
        self.vars.req_movement = (dx, dy);
    }
    fn set_moving(&mut self, moving: bool) {
        self.vars.moving = moving;
    }
    fn is_animation_updating_this_frame(&self) -> bool {
        true
    }
    /// `isPerformingNoAimShortStrafe()`: strafing without aiming.
    fn is_performing_no_aim_short_strafe(&self) -> bool {
        self.vars.path_strafing && !GameCharacter::is_aiming(self)
    }
    fn set_strafe_state_speed(&mut self, speed: f32) {
        super::states::STRAFE_SPEED.set(&mut self.state.params, speed);
    }

    fn forward_direction(&self) -> (f32, f32) {
        (self.vars.forward_x, self.vars.forward_y)
    }
    fn set_forward_direction(&mut self, x: f32, y: f32) {
        GameCharacter::set_forward_direction(self, x, y);
    }
    fn normalize_forward_direction(&mut self) {
        let (x, y) = (self.vars.forward_x, self.vars.forward_y);
        if x * x + y * y != 0.0 {
            (self.vars.forward_x, self.vars.forward_y) = super::zombie::math::normalize(x, y);
        }
    }
    /// `DirectionFromVector(v)`: `setForwardIsoDirection(fromAngle(v))`.
    fn direction_from_vector(&mut self, x: f32, y: f32) {
        let d = super::state::vars::iso_direction_from_angle(x, y);
        self.face_direction(d);
    }
    fn face_location(&mut self, x: f32, y: f32) {
        self.face_location_f(x + 0.5, y + 0.5);
    }
    fn face_location_f(&mut self, x: f32, y: f32) {
        let dx = x - self.vars.x;
        let dy = y - self.vars.y;
        if dx.abs() <= 1.0e-7 && dy.abs() <= 1.0e-7 {
            return;
        }
        GameCharacter::set_forward_direction(self, dx, dy);
    }
    /// `faceDirection(dir)`: `dir`, then the forward vector from it.
    fn face_direction(&mut self, dir: IsoDirections) {
        let (dx, dy) = dir.delta();
        GameCharacter::set_forward_direction(self, dx as f32, dy as f32);
        self.vars.dir = dir;
    }
    fn face_this_object(&mut self, target: FaceTarget) {
        self.path.requests.push(PathMoverRequest::FaceObject(target));
    }
    /// `shouldBeTurning()`: the twist is not modelled (no animation
    /// player), so only a blocked turn answers.
    fn should_be_turning(&self) -> bool {
        false
    }
    fn is_aiming(&self) -> bool {
        GameCharacter::is_aiming(self)
    }
    fn is_animating_backwards(&self) -> bool {
        self.vars.animating_backwards
    }
    fn update_animation_forward_direction(&mut self) {}

    fn set_collidable(&mut self, collidable: bool) {
        self.path.collidable = collidable;
    }
    /// TODO(world collision): the mover-aware `testCollideAdjacent`.
    fn test_collide_adjacent(&mut self, _from: (i32, i32, i32), _dx: i32, _dy: i32, _dz: i32) -> bool {
        false
    }

    fn climb_over_fence(&mut self, dir: IsoDirections) {
        self.path.requests.push(PathMoverRequest::ClimbOverFence(dir));
    }
    fn climb_through_window(&mut self, window: ObjectRef) {
        self.path.requests.push(PathMoverRequest::ClimbThroughWindow(window));
    }
    fn climb_through_window_frame(&mut self, frame: ObjectRef) {
        self.path.requests.push(PathMoverRequest::ClimbThroughWindowFrame(frame));
    }
    /// `IsoWindow.canClimbThrough(chr)` through the world's window hook
    /// (the trait gives no world); without a hook no window is climbable.
    fn can_climb_through_window(&self, window: ObjectRef) -> bool {
        self.path.window_hook.as_ref().is_some_and(|h| (h.0)(window))
    }
    /// TODO(world objects): `couldBeOpen(chr)`.
    fn could_open_door(&self, _door: ObjectRef) -> bool {
        false
    }
    /// `ToggleDoor(chr)` then `isOpen()`, through the world's door hook
    /// (TODO(pz-world doors): call the door objects directly). Without a
    /// hook the door stays shut.
    fn toggle_door(&mut self, door: ObjectRef) -> bool {
        self.path.requests.push(PathMoverRequest::ToggleDoor(door));
        match self.path.door_hook.clone() {
            Some(h) => (h.0)(door),
            None => false,
        }
    }

    fn get_variable_boolean(&self, name: &str) -> bool {
        AnimationVariableSource::get_variable_boolean(self, name)
    }
    fn set_variable_bool(&mut self, name: &str, value: bool) {
        AnimationVariableSource::set_variable_bool(self, name, value);
    }
    fn set_variable_float(&mut self, name: &str, value: f32) {
        AnimationVariableSource::set_variable_float(self, name, value);
    }
}

impl<I: 'static> PlayerMover for GameCharacter<I> {
    fn is_npc(&self) -> bool {
        self.path.npc
    }
    fn is_local_player(&self) -> bool {
        self.in_players
    }
    fn is_local(&self) -> bool {
        !self.vars.remote
    }
    fn time_since_close_door(&self) -> f32 {
        self.path.time_since_close_door
    }
    fn climbing_fail_chance_float(&self) -> f32 {
        self.path.climbing_fail_chance
    }
    fn climb_over_wall(&mut self, dir: IsoDirections) {
        self.path.requests.push(PathMoverRequest::ClimbOverWall(dir));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::tests::{fixture, player};

    #[test]
    fn mover_views_and_states() {
        let (sm, mut chars) = fixture();
        let mut c = player(&sm, &mut chars);
        c.id = 7;
        assert!(c.as_player().is_some() && c.as_zombie().is_none());
        assert_eq!(PathMover::current_state(&c), CharacterState::Other(0));
        assert_ne!(character_state(Some(ids::IDLE)), character_state(Some(ids::BUMPED)));
        assert_eq!(character_state(Some(ids::WALK_TOWARD)), CharacterState::WalkToward);
        c.vars.x = 1.0;
        c.vars.y = 1.0;
        c.face_location(2.5, 0.5);
        assert_eq!(PathMover::forward_direction(&c), (1.0, 0.0));
        c.face_direction(IsoDirections::S);
        assert_eq!(c.vars.dir, IsoDirections::S);
        PathMover::set_variable_bool(&mut c, "bPathfind", true);
        assert!(PathMover::get_variable_boolean(&c, "bpathfind"));
        let door = ObjectRef { x: 1, y: 2, z: 0, id: pz_world::objects::ObjectId::default() };
        assert!(!c.toggle_door(door));
        c.path.door_hook = Some(DoorHook(std::rc::Rc::new(|_| true)));
        assert!(c.toggle_door(door));
        c.path.requests.clear();
        c.climb_over_fence(IsoDirections::N);
        assert_eq!(c.path.requests, [PathMoverRequest::ClimbOverFence(IsoDirections::N)]);
    }
}
