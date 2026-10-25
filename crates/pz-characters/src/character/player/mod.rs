//! The local player's control loop: `IsoPlayer.update` (`updateInternal1`
//! / `updateInternal2`), the player part of
//! `IsoGameCharacter.updateInternal`, `IsoPlayer.postupdate`, the input it
//! reads ([`PlayerInput`], [`CharacterInput`]) and `AimingReticle`.
//!
//! The engine calls, per frame and per local player: `preupdate_player`,
//! `frame_step` (the input component's toggles), `update_player`, and after
//! all objects updated `postupdate_player`. What the original does through
//! systems that are not characters' (camera, sound, music, Lua events, the
//! UI) is a [`PlayerEvent`] or a `StateWorld` hook.

pub mod aiming;
pub mod attack;
pub mod context;
pub mod control;
pub mod input;
pub mod update;
pub mod vision;

pub use aiming::AimingMode;
pub use context::{ContextualAction, ContextualActionKind};
pub use control::{CharacterInput, CharacterInputMode, InputOwner, TimedInputHandler};
pub use input::{JoypadAxis2dBinding, NoInput, PlayerInput};
pub use vision::PlayerRenderInputs;

/// `IsoPlayer`'s control-loop fields (and the `IsoGameCharacter` ones only
/// the player loop uses).
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerControl {
    /// IsoPlayer.isFarming; maintained by Lua farming/timed actions.
    pub is_farming: bool,
    /// `playerIndex`.
    pub player_index: i32,
    /// `CharacterInputComponent`.
    pub input: CharacterInput,
    /// `AimingReticle.aimingPositions[playerIndex]` (screen pixels).
    pub aiming_position: (f32, f32),
    /// `playerMoveDir`.
    pub player_move_dir: (f32, f32),
    /// `IsoMovingObject.movementLastFrame`.
    pub movement_last_frame: (f32, f32),
    /// `lastAngle`.
    pub last_angle: (f32, f32),
    /// `justMoved`.
    pub just_moved: bool,
    /// `isWalking`.
    pub is_walking: bool,
    /// `waiting`.
    pub waiting: bool,
    /// `runningTime`.
    pub running_time: f32,
    /// `slowTimer`, `slowFactor`.
    pub slow_timer: f32,
    pub slow_factor: f32,
    /// `closestZombie`.
    pub closest_zombie: f32,
    /// `contextPanic`.
    pub context_panic: f32,
    /// `ticksSincePressedMovement`.
    pub ticks_since_pressed_movement: f32,
    /// `ticksSinceSeenZombie`.
    pub ticks_since_seen_zombie: i32,
    /// `lastSeenZombieTime` (hours).
    pub last_seen_zombie_time: f64,
    /// `pressContext`, `timePressedContext`.
    pub press_context: bool,
    pub time_pressed_context: f32,
    /// `isCharging`, `isChargingLT`, `useChargeTime` (`chargeTime` is
    /// [`crate::character::state::CharacterVars::charge_time`]).
    pub is_charging: bool,
    pub is_charging_lt: bool,
    pub use_charge_time: f32,
    /// `pressedRun`, `pressedRunTimer` (the double-tap sprint).
    pub pressed_run: bool,
    pub pressed_run_timer: f32,
    /// `lastAttackWasHandToHand`.
    pub last_attack_was_hand_to_hand: bool,
    /// `letGoAfterContextIsReleased`.
    pub let_go_after_context_is_released: bool,
    /// `timeSinceOpenDoor` (`timeSinceCloseDoor` is the path state's).
    pub time_since_open_door: f32,
    /// `lastTargeted`, `targetedByZombie`.
    pub last_targeted: f32,
    pub targeted_by_zombie: bool,
    /// `dirtyRecalcGridStackTime`.
    pub dirty_recalc_grid_stack_time: f32,
    /// `extUpdateCount`.
    pub ext_update_count: f32,
    /// `autoWalk`, `autoWalkDirection`.
    pub auto_walk: bool,
    pub auto_walk_direction: (f32, f32),
    /// `hypothermiaCache`, `hyperthermiaCache`.
    pub hypothermia_cache: i32,
    pub hyperthermia_cache: i32,
    /// `blurFactor`, `blurFactorTarget`.
    pub blur_factor: f32,
    pub blur_factor_target: f32,
    /// `isWearingNightVisionGoggles` (only `setWearingNightVisionGoggles`
    /// sets it).
    pub wearing_night_vision_goggles: bool,
    /// `attackAnimThrowTimer` (ms of [`PlayerInput::current_time_ms`]).
    pub attack_anim_throw_timer: i64,
    /// The clock of the last update (`System.currentTimeMillis()`).
    pub now_ms: i64,
    /// `footInjuryTimer`.
    pub foot_injury_timer: f32,
    /// `weight` (`IsoMovingObject`).
    pub weight: f32,
    /// `runSpeedModifier`, `walkSpeedModifier`, `fullSpeedMod`.
    pub run_speed_modifier: f32,
    pub walk_speed_modifier: f32,
    pub full_speed_mod: f32,
    /// `authorizeMeleeAction`/`authorizeShoveStomp`
    /// (`isAuthorizedHandToHandAction` / `isAuthorizedHandToHand`).
    pub authorize_hand_to_hand_action: bool,
    pub authorize_hand_to_hand: bool,
    /// `deathFinished`.
    pub death_finished: bool,
    /// `collidable` (`setCollidable`).
    pub collidable: bool,
    /// `seenThisFrame`, `couldBeSeenThisFrame`.
    pub seen_this_frame: bool,
    pub could_be_seen_this_frame: bool,
    /// `characterActions.get(0).blockMovementEtc` (the Lua timed action
    /// queue is the engine's).
    pub action_blocks_movement: bool,
    /// `characterActions.get(0).overrideAnimation`.
    pub action_overrides_animation: bool,
    /// `!characterActions.isEmpty()`.
    pub has_actions: bool,
    /// `forceOverrideAnim`.
    pub force_override_anim: bool,
    /// `ignoreContextKey`.
    pub ignore_context_key: bool,
    /// `ClimateManager.getWindSpeedMovement()` and
    /// `getWindForceMovement(chr, angle)` as the climate gives them (set by
    /// the engine before the update; `windspeed`/`windForce` copy them).
    pub climate_wind_speed_movement: f32,
    pub climate_wind_force_movement: f32,
    /// `lungeFallTimer`.
    pub lunge_fall_timer: f32,
    /// `isNpc()`.
    pub npc: bool,
    /// `climbOverWallSuccess`, `climbOverWallStruggle`.
    pub climb_over_wall_success: bool,
    pub climb_over_wall_struggle: bool,
}

impl Default for PlayerControl {
    fn default() -> Self {
        PlayerControl {
            player_index: 0,
            input: CharacterInput::default(),
            aiming_position: (0.0, 0.0),
            player_move_dir: (0.0, 0.0),
            movement_last_frame: (0.0, 0.0),
            last_angle: (0.0, 0.0),
            just_moved: false,
            is_walking: false,
            waiting: false,
            running_time: 0.0,
            slow_timer: 0.0,
            slow_factor: 0.0,
            closest_zombie: 1_000_000.0,
            context_panic: 0.0,
            ticks_since_pressed_movement: 0.0,
            ticks_since_seen_zombie: 9_999_999,
            last_seen_zombie_time: 2.0,
            press_context: false,
            time_pressed_context: 0.0,
            is_charging: false,
            is_charging_lt: false,
            use_charge_time: 0.0,
            pressed_run: false,
            pressed_run_timer: 0.0,
            last_attack_was_hand_to_hand: false,
            let_go_after_context_is_released: false,
            time_since_open_door: 0.0,
            last_targeted: 1e8,
            targeted_by_zombie: false,
            dirty_recalc_grid_stack_time: 2.0,
            ext_update_count: 0.0,
            auto_walk: false,
            auto_walk_direction: (0.0, 0.0),
            hypothermia_cache: -1,
            hyperthermia_cache: -1,
            blur_factor: 0.0,
            blur_factor_target: 0.0,
            wearing_night_vision_goggles: false,
            attack_anim_throw_timer: 0,
            now_ms: 0,
            foot_injury_timer: 0.0,
            weight: 1.0,
            run_speed_modifier: 1.0,
            walk_speed_modifier: 1.0,
            full_speed_mod: 1.0,
            authorize_hand_to_hand_action: true,
            authorize_hand_to_hand: true,
            death_finished: false,
            collidable: true,
            seen_this_frame: false,
            could_be_seen_this_frame: false,
            is_farming: false,
            action_blocks_movement: false,
            action_overrides_animation: false,
            has_actions: false,
            force_override_anim: false,
            ignore_context_key: false,
            climate_wind_speed_movement: 0.0,
            climate_wind_force_movement: 0.0,
            lunge_fall_timer: 0.0,
            npc: false,
            climb_over_wall_success: false,
            climb_over_wall_struggle: false,
        }
    }
}

/// What the control loop reports to the engine (the parts of the
/// original that are camera, sound, music, UI or Lua). Collected in
/// [`PlayerUpdate::events`].
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    /// `LuaEventManager.triggerEvent(name, player)`: `OnPlayerMove`,
    /// `OnPlayerUpdate`.
    LuaEvent(&'static str),
    /// `triggerEvent("OnContextKey", player, millis)`.
    ContextKey { millis: f64 },
    /// `triggerEvent("OnPressWalkTo", 0, 0, 0)`.
    PressWalkTo,
    /// `triggerEvent("OnPressReloadButton"/"OnPressRackButton", ...)`.
    PressReload { rack: bool, shift: bool },
    /// `performContextualAction(action)`: the world or Lua performs it.
    ContextualAction(ContextualAction),
    /// `IsoReticle.trySetTargetReticleMode(index, mode)` (`true`: RETICLE).
    ReticleMode { reticle: bool },
    /// `addDirt(null, count, allLayers)` (its `OutfitRNG` draws are the
    /// visual's).
    AddDirt { count: i32, all_layers: bool },
    /// `StopAllActionQueueAiming/Running/Walking`.
    StopActionQueue(&'static str),
    /// `Mouse.setCursorVisible(false)` (`updateCursorVisibility`).
    HideCursor,
}

/// The events of one `update_player`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerUpdate {
    pub events: Vec<PlayerEvent>,
}

impl<I: 'static> super::GameCharacter<I> {
    /// The [`InputOwner`] of this player's input component.
    pub fn input_owner(&self) -> InputOwner {
        use pz_anim::AnimationVariableSource;
        InputOwner {
            player_index: self.player.player_index,
            climb_started: self.get_variable_boolean("ClimbWindowStarted") || self.get_variable_boolean("ClimbFenceStarted"),
        }
    }

    /// `ECSEntity.frameStep()` for the player's input component (between
    /// `preupdate` and `update`).
    pub fn frame_step(&mut self, input: &dyn PlayerInput) {
        let owner = self.input_owner();
        self.player.input.frame_step(input, owner);
    }
}
