//! The `zombie.ai.states` singletons ported so far, and the class ids of
//! all of them ([`ids`]) for states compared by identity before they are
//! ported.
//!
//! Each ported state is a unit struct with an `instance::<I>()`
//! returning its [`StateRef`]; [`all`] lists them for the registry.

use super::state::{Param, StateId, StateRef};

pub mod climb_fence;
pub mod climb_rope;
pub mod climb_wall;
pub mod climb_window;
pub mod generic_default;
pub mod idle;
pub mod player;
pub mod player_ai;
pub mod player_movement;
pub mod stagger_back;
pub mod swipe;
pub mod zombie;

/// Every `zombie.ai.states` class (the jar's names).
pub mod ids {
    use super::StateId;

    pub const ATTACK_NETWORK: StateId = StateId("zombie.ai.states.AttackNetworkState");
    pub const ATTACK: StateId = StateId("zombie.ai.states.AttackState");
    pub const BUMPED: StateId = StateId("zombie.ai.states.BumpedState");
    pub const BURNT_TO_DEATH: StateId = StateId("zombie.ai.states.BurntToDeath");
    pub const CLIMB_DOWN_SHEET_ROPE: StateId = StateId("zombie.ai.states.ClimbDownSheetRopeState");
    pub const CLIMB_OVER_FENCE: StateId = StateId("zombie.ai.states.ClimbOverFenceState");
    pub const CLIMB_OVER_WALL: StateId = StateId("zombie.ai.states.ClimbOverWallState");
    pub const CLIMB_SHEET_ROPE: StateId = StateId("zombie.ai.states.ClimbSheetRopeState");
    pub const CLIMB_THROUGH_WINDOW: StateId = StateId("zombie.ai.states.ClimbThroughWindowState");
    pub const CLOSE_WINDOW: StateId = StateId("zombie.ai.states.CloseWindowState");
    pub const COLLIDE_WITH_WALL: StateId = StateId("zombie.ai.states.CollideWithWallState");
    pub const CRAWLING_ZOMBIE_TURN: StateId = StateId("zombie.ai.states.CrawlingZombieTurnState");
    pub const FAKE_DEAD_ATTACK: StateId = StateId("zombie.ai.states.FakeDeadAttackState");
    pub const FAKE_DEAD_ZOMBIE: StateId = StateId("zombie.ai.states.FakeDeadZombieState");
    pub const FISHING: StateId = StateId("zombie.ai.states.FishingState");
    pub const FITNESS: StateId = StateId("zombie.ai.states.FitnessState");
    pub const FORECAST_BEATEN_PLAYER: StateId = StateId("zombie.ai.states.ForecastBeatenPlayerState");
    pub const GENERIC_DEFAULT: StateId = StateId("zombie.ai.states.GenericDefaultState");
    pub const GRAPPLED_THROWN_INTO_CONTAINER: StateId = StateId("zombie.ai.states.GrappledThrownIntoContainerState");
    pub const GRAPPLED_THROWN_OUT_WINDOW: StateId = StateId("zombie.ai.states.GrappledThrownOutWindowState");
    pub const GRAPPLED_THROWN_OVER_FENCE: StateId = StateId("zombie.ai.states.GrappledThrownOverFenceState");
    pub const IDLE: StateId = StateId("zombie.ai.states.IdleState");
    pub const LUNGE_NETWORK: StateId = StateId("zombie.ai.states.LungeNetworkState");
    pub const LUNGE: StateId = StateId("zombie.ai.states.LungeState");
    pub const OPEN_WINDOW: StateId = StateId("zombie.ai.states.OpenWindowState");
    pub const PATH_FIND: StateId = StateId("zombie.ai.states.PathFindState");
    pub const PLAYER_ACTIONS: StateId = StateId("zombie.ai.states.PlayerActionsState");
    pub const PLAYER_AIM: StateId = StateId("zombie.ai.states.PlayerAimState");
    pub const PLAYER_DRAGGING_CORPSE: StateId = StateId("zombie.ai.states.PlayerDraggingCorpse");
    pub const PLAYER_EMOTE: StateId = StateId("zombie.ai.states.PlayerEmoteState");
    pub const PLAYER_EXT: StateId = StateId("zombie.ai.states.PlayerExtState");
    pub const PLAYER_FALL_DOWN: StateId = StateId("zombie.ai.states.PlayerFallDownState");
    pub const PLAYER_FALLING: StateId = StateId("zombie.ai.states.PlayerFallingState");
    pub const PLAYER_GET_UP: StateId = StateId("zombie.ai.states.PlayerGetUpState");
    pub const PLAYER_HIT_REACTION_PVP: StateId = StateId("zombie.ai.states.PlayerHitReactionPVPState");
    pub const PLAYER_HIT_REACTION: StateId = StateId("zombie.ai.states.PlayerHitReactionState");
    pub const PLAYER_KNOCKED_DOWN: StateId = StateId("zombie.ai.states.PlayerKnockedDown");
    pub const PLAYER_ON_BED: StateId = StateId("zombie.ai.states.PlayerOnBedState");
    pub const PLAYER_ON_GROUND: StateId = StateId("zombie.ai.states.PlayerOnGroundState");
    pub const PLAYER_SIT_ON_FURNITURE: StateId = StateId("zombie.ai.states.PlayerSitOnFurnitureState");
    pub const PLAYER_SIT_ON_GROUND: StateId = StateId("zombie.ai.states.PlayerSitOnGroundState");
    pub const PLAYER_STRAFE: StateId = StateId("zombie.ai.states.PlayerStrafeState");
    pub const SMASH_WINDOW: StateId = StateId("zombie.ai.states.SmashWindowState");
    pub const STAGGER_BACK: StateId = StateId("zombie.ai.states.StaggerBackState");
    pub const SWIPE_STATE_PLAYER: StateId = StateId("zombie.ai.states.SwipeStatePlayer");
    pub const THUMP: StateId = StateId("zombie.ai.states.ThumpState");
    pub const VEHICLE_COLLISION_MINOR_STAGGER: StateId = StateId("zombie.ai.states.VehicleCollisionMinorStaggerState");
    pub const VEHICLE_COLLISION_ON_GROUND: StateId = StateId("zombie.ai.states.VehicleCollisionOnGroundState");
    pub const VEHICLE_COLLISION: StateId = StateId("zombie.ai.states.VehicleCollisionState");
    pub const WALK_TOWARD_NETWORK: StateId = StateId("zombie.ai.states.WalkTowardNetworkState");
    pub const WALK_TOWARD: StateId = StateId("zombie.ai.states.WalkTowardState");
    pub const ZOMBIE_EAT_BODY: StateId = StateId("zombie.ai.states.ZombieEatBodyState");
    pub const ZOMBIE_FACE_TARGET: StateId = StateId("zombie.ai.states.ZombieFaceTargetState");
    pub const ZOMBIE_FALL_DOWN: StateId = StateId("zombie.ai.states.ZombieFallDownState");
    pub const ZOMBIE_FALLING: StateId = StateId("zombie.ai.states.ZombieFallingState");
    pub const ZOMBIE_GET_DOWN: StateId = StateId("zombie.ai.states.ZombieGetDownState");
    pub const ZOMBIE_GET_UP_FROM_CRAWL: StateId = StateId("zombie.ai.states.ZombieGetUpFromCrawlState");
    pub const ZOMBIE_GET_UP: StateId = StateId("zombie.ai.states.ZombieGetUpState");
    pub const ZOMBIE_HIT_REACTION: StateId = StateId("zombie.ai.states.ZombieHitReactionState");
    pub const ZOMBIE_IDLE: StateId = StateId("zombie.ai.states.ZombieIdleState");
    pub const ZOMBIE_ON_GROUND: StateId = StateId("zombie.ai.states.ZombieOnGroundState");
    pub const ZOMBIE_RAGDOLL_ON_GROUND: StateId = StateId("zombie.ai.states.ZombieRagdollOnGroundState");
    pub const ZOMBIE_REANIMATE: StateId = StateId("zombie.ai.states.ZombieReanimateState");
    pub const ZOMBIE_SITTING: StateId = StateId("zombie.ai.states.ZombieSittingState");
    pub const ZOMBIE_TURN_ALERTED: StateId = StateId("zombie.ai.states.ZombieTurnAlerted");
    pub const PLAYER_MILK_ANIMAL: StateId = StateId("zombie.ai.states.player.PlayerMilkAnimalState");
    pub const PLAYER_MOVEMENT: StateId = StateId("zombie.ai.states.player.PlayerMovementState");
    pub const PLAYER_PET_ANIMAL: StateId = StateId("zombie.ai.states.player.PlayerPetAnimalState");
    pub const PLAYER_SHEAR_ANIMAL: StateId = StateId("zombie.ai.states.player.PlayerShearAnimalState");
    pub const ANIMAL_ALERTED: StateId = StateId("zombie.ai.states.animals.AnimalAlertedState");
    pub const ANIMAL_ATTACK: StateId = StateId("zombie.ai.states.animals.AnimalAttackState");
    pub const ANIMAL_CLIMB_OVER_FENCE: StateId = StateId("zombie.ai.states.animals.AnimalClimbOverFenceState");
    pub const ANIMAL_EAT: StateId = StateId("zombie.ai.states.animals.AnimalEatState");
    pub const ANIMAL_FALLDOWN: StateId = StateId("zombie.ai.states.animals.AnimalFalldownState");
    pub const ANIMAL_FOLLOW_WALL: StateId = StateId("zombie.ai.states.animals.AnimalFollowWallState");
    pub const ANIMAL_HIT_REACTION: StateId = StateId("zombie.ai.states.animals.AnimalHitReactionState");
    pub const ANIMAL_IDLE: StateId = StateId("zombie.ai.states.animals.AnimalIdleState");
    pub const ANIMAL_ON_GROUND: StateId = StateId("zombie.ai.states.animals.AnimalOnGroundState");
    pub const ANIMAL_PATH_FIND: StateId = StateId("zombie.ai.states.animals.AnimalPathFindState");
    pub const ANIMAL_WALK: StateId = StateId("zombie.ai.states.animals.AnimalWalkState");
    pub const ANIMAL_ZONE: StateId = StateId("zombie.ai.states.animals.AnimalZoneState");
}

/// `PlayerStrafeState.STRAFE_SPEED`.
pub const STRAFE_SPEED: Param<f32> = Param::new(ids::PLAYER_STRAFE, "strafe_speed", || 1.0);

/// The ported states.
pub fn all<I: 'static>() -> Vec<StateRef<I>> {
    vec![
        generic_default::instance(),
        idle::instance(),
        stagger_back::instance(),
        swipe::instance(),
        player::knocked_down(),
        player::fall_down(),
        player::hit_reaction(),
        player::hit_reaction_pvp(),
        player::on_ground(),
        player::get_up(),
        player::collide_with_wall(),
        player::aim(),
        player::strafe(),
        player_movement::instance(),
        climb_fence::climb_over_fence(),
        climb_window::climb_through_window(),
        climb_wall::climb_over_wall(),
        climb_rope::climb_sheet_rope(),
        climb_rope::climb_down_sheet_rope(),
    ]
    .into_iter()
    .chain(zombie::all())
    .collect()
}
