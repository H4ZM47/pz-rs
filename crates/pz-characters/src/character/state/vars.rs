//! The character's animation variables: `IsoGameCharacter.gameVariables`
//! (an `AnimationVariableSource`) with the callback slots
//! `registerVariableCallbacks` adds (`IsoGameCharacter`,
//! `IsoLivingCharacter`, `IsoPlayer`, `FallDamage`,
//! `BaseGrappleable.RegisterGrappleVariables`), and
//! `IsoGameCharacter.getVariable`, which looks in the action states'
//! variables first.
//!
//! The callback slots are [`CharVar`]s: each reads (and, when the original
//! registered a setter, writes) a field of [`CharacterVars`] or computes
//! its value like the original getter. Every other name is an ordinary
//! slot in [`GameCharacter::game_variables`].
//!
//! Values the original computes from systems not ported here (the
//! animation player's twist, ragdolls, grappling, the vehicle) read fields
//! the owner of those systems fills; they default to the value a character
//! without them has.
//!
//! TODO(network): `setVariable` on a server zombie is a no-op and a local
//! client player sends `VariableSync` packets; neither is modelled.

use std::cell::RefCell;
use std::collections::HashMap;

use pz_anim::{AnimationVariableHandle, AnimationVariableSource, VariableValue};
use pz_anim::vars::{equals_ignore_case, java_float_to_string, AnimationVariableType};

use super::super::{CharacterKind, GameCharacter};

/// The plain fields behind the callback variables and the combat and
/// movement code (`IsoMovingObject`, `IsoGameCharacter`,
/// `IsoLivingCharacter` and `IsoPlayer` fields), with their initial values.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterVars {
    // ---- IsoMovingObject ----
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub last_x: f32,
    pub last_y: f32,
    pub last_z: f32,
    /// `onFloor`.
    pub on_floor: bool,
    /// `closeKilled`.
    pub close_killed: bool,
    /// `hitFromAngle`.
    pub hit_from_angle: f32,
    /// `dir` (`getDir()`), kept in step with the forward direction.
    pub dir: pz_world::square::IsoDirections,
    // ---- IsoGameCharacter ----
    /// `forwardDirection` (normalized).
    pub forward_x: f32,
    pub forward_y: f32,
    /// `doRender` (`isRendered`).
    pub do_render: bool,
    pub state_event_delay_timer: f32,
    pub hit_reaction: String,
    pub collide_type: String,
    pub bump_type: String,
    /// `isBumpDone`.
    pub bump_done: bool,
    pub bump_fall: bool,
    pub bump_staggered: bool,
    pub bump_fall_type: String,
    pub on_bed: bool,
    pub near_wall_crouching: bool,
    /// `canClimbDownSheetRopeInCurrentSquare()` (from the square).
    pub can_climb_down_rope: bool,
    pub hit_from_behind: bool,
    pub killed_by_fall: bool,
    /// `isInTreesNoBush()` (from the square).
    pub in_trees_no_bush: bool,
    pub ranged_weapon_empty: bool,
    pub move_delta: f32,
    /// `turnDeltaNormal` (`setTurnDelta`).
    pub turn_delta_normal: f32,
    pub turn_delta_running: f32,
    pub turn_delta_sprinting: f32,
    pub max_twist: f32,
    /// `getAnimAngle()`, `getTwist()`, `getTargetTwist()`,
    /// `getShoulderTwist()`, `getExcessTwist()`, `getNumTwistBones()`,
    /// `getAnimAngleStepDelta()`, `getAnimAngleTwistDelta()` (from the
    /// animation player).
    pub anim_angle: f32,
    pub twist: f32,
    pub target_twist: f32,
    pub shoulder_twist: f32,
    pub excess_twist: f32,
    pub num_twist_bones: i32,
    pub angle_step_delta: f32,
    pub angle_twist_delta: f32,
    /// `isTurning`, `isTurning90`, `isTurningAround`.
    pub turning: bool,
    pub turning90: bool,
    pub turning_around: bool,
    /// `isMoving` (`bMoving`).
    pub moving: bool,
    pub been_moving_for: f32,
    pub been_sprinting_for: f32,
    pub momentum_scalar: f32,
    /// `hasTimedActions()` (the timed-action queue is not ported).
    pub has_timed_actions: bool,
    /// `isCrit`.
    pub critical_hit: bool,
    pub knocked_down: bool,
    pub was_knocked_down: bool,
    pub falling: bool,
    pub ragdoll_fall: bool,
    pub fall_time: f32,
    pub last_fall_speed: f32,
    /// `isAiming` (the field; [`GameCharacter::is_aiming`] is the getter).
    pub is_aiming: bool,
    /// `isIgnoringAimingInput()`.
    pub ignoring_aiming_input: bool,
    pub aim_at_floor: bool,
    pub aim_at_floor_target_distance: f32,
    pub current_vertical_aim_angle: f32,
    pub target_vertical_aim_angle: f32,
    /// `isPerformingAttackAnim`, `isPerformingShoveAnim`,
    /// `isPerformingStompAnim`.
    pub performing_attack_animation: bool,
    pub performing_shove_animation: bool,
    pub performing_stomp_animation: bool,
    /// `isPerformingGrappleGrabAnimation()` (grappling is not ported).
    pub performing_grapple_grab_animation: bool,
    pub shove_stomp_anim: bool,
    pub animating_backwards: bool,
    pub grapple_throw_out_window: bool,
    pub grapple_throw_over_fence: bool,
    pub grapple_throw_into_container: bool,
    pub editing_ragdoll: bool,
    pub vehicle_collision: bool,
    pub use_physic_hit_reaction: bool,
    /// `isHeadLookAround()`, `getHeadLookHorizontal()`,
    /// `getHeadLookVertical()`.
    pub head_look_around: bool,
    pub head_look_horizontal: f32,
    pub head_look_vertical: f32,
    pub hit_force: f32,
    /// `hitDir`.
    pub hit_dir_x: f32,
    pub hit_dir_y: f32,
    /// `hitDirEnum` (`HitDirEnum` constant name).
    pub hit_dir_enum: &'static str,
    pub recoil_var_x: f32,
    pub recoil_var_y: f32,
    pub hide_equipped_hand_l: bool,
    pub hide_equipped_hand_r: bool,
    pub has_target: bool,
    /// `isPrecisionAimKeyDown()` (input).
    pub precision_aim_key_down: bool,
    /// `getShoutType()`, `getShoutItemModel()`.
    pub shout_type: String,
    pub shout_item_model: String,
    /// `FallDamage`: `isDamagingFall`, `isLightFall`, `isHardFall`,
    /// `isMoreThanHardFall`, `getFallImpactSeverity`.
    pub fall_damaging: bool,
    pub fall_light: bool,
    pub fall_hard: bool,
    pub fall_more_than_hard: bool,
    pub fall_impact_severity: &'static str,
    /// `staggerTimeMod`.
    pub stagger_time_mod: f32,
    pub knockback_attack_mod: f32,
    pub avoid_damage: bool,
    pub no_damage: bool,
    pub last_hit_count: i32,
    pub reanimate_timer: f32,
    pub melee_delay: f32,
    pub recoil_delay: f32,
    pub aiming_delay: f32,
    /// `useHandWeapon` (by item id).
    pub use_hand_weapon: Option<i32>,
    pub speed_mod: f32,
    pub combat_speed_modifier: f32,
    pub block_turning: bool,
    pub ignore_movement: bool,
    pub ignore_stagger_back: bool,
    pub damaged_by_vehicle: bool,
    pub climbing: bool,
    pub on_kill_done: bool,
    /// `getAttackedBy()` (by character id).
    pub attacked_by: Option<i32>,
    /// `lastHitCharacter`.
    pub last_hit_character: Option<i32>,
    // ---- IsoLivingCharacter ----
    pub do_shove: bool,
    pub do_grapple: bool,
    pub target_on_ground: Option<i32>,
    // ---- IsoPlayer ----
    pub combat_speed: f32,
    /// `IsoPlayer.turnDelta`.
    pub player_turn_delta: f32,
    pub block_movement: bool,
    pub initiate_attack: bool,
    pub walk_speed: f32,
    pub walk_injury: f32,
    pub run_speed: f32,
    pub idle_speed: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub windspeed: f32,
    pub wind_force: f32,
    pub ip_x: f32,
    pub ip_y: f32,
    /// `getAttackTypeAnimationKey()`.
    pub attack_type: String,
    pub attack_from_behind: bool,
    /// `isUnderVehicle()`.
    pub under_vehicle: bool,
    /// `IsoPlayer.isAttacking()` (`attackStarted`).
    pub attack_started: bool,
    pub banned_attacking: bool,
    pub melee_pressed: bool,
    pub grapple_pressed: bool,
    pub can_let_go_of_grappled: bool,
    /// `getWeaponType()` / `setWeaponType` (`Weapon`).
    pub weapon_type: String,
    pub remote: bool,
    pub performing_an_action: bool,
    pub attack_variation_x: f32,
    pub attack_variation_y: f32,
    pub sneak_limp_speed_scale: f32,
    pub is_player_moving: bool,
    pub path_strafing: bool,
    /// `isOverEncumbered()` as the last update computed it (the getter
    /// needs the inventory weight).
    pub over_encumbered: bool,
    /// `IsoPlayer.chargeTime`.
    pub charge_time: f32,
    /// `IsoLivingCharacter.useChargeDelta`.
    pub use_charge_delta: f32,
    /// `isMeleeButtonDown()`, `isManualFloorAtkButtonDown()` (input).
    pub melee_button_down: bool,
    pub manual_floor_attack_button_down: bool,
    /// `clickSound`.
    pub click_sound: Option<String>,
    /// `isShootable()`.
    pub shootable: bool,
    pub deferred_movement_enabled: bool,
    /// `IsoZombie.playerAttackPosition` (`testDotSide` name).
    pub player_attack_position: Option<String>,
    pub playing_death_sound: bool,
    pub hide_weapon_model: bool,
    /// `collidedN/S/E/W` (from the movement code).
    pub collided_n: bool,
    pub collided_s: bool,
    pub collided_e: bool,
    pub collided_w: bool,
    pub last_collide_time: f32,
    /// `IsoPlayer.ignoreAutoVault`.
    pub ignore_auto_vault: bool,
    /// `IsoPlayer.hasObstacleOnPath` (a remote player's).
    pub has_obstacle_on_path: bool,
    /// `IsoMovingObject.nx`, `ny` (`getNextX/Y`), `reqMovement`, `impulsex/y`.
    pub next_x: f32,
    pub next_y: f32,
    pub req_movement: (f32, f32),
    pub impulse_x: f32,
    pub impulse_y: f32,
    /// `altCollide`, `collidedThisFrame`, `collidedWithVehicle`,
    /// `collidedWithDoor`.
    pub alt_collide: bool,
    pub collided_this_frame: bool,
    pub collided_with_vehicle: bool,
    pub collided_with_door: bool,
    /// `collidedObject`: the special object the last collision hit.
    pub collided_object: Option<pz_pathfind::mover::ObjectRef>,
    /// `last` (the square before this frame's move).
    pub last_square: Option<(i32, i32, i32)>,
    /// `isRagdollSimulationActive()`, `IsoPlayer.isNoClip()`, `destroyed`.
    pub ragdoll_simulation_active: bool,
    pub no_clip: bool,
    pub destroyed: bool,
    /// `IsoPlayer.lastCollidedW/N`.
    pub last_collided_w: bool,
    pub last_collided_n: bool,
    // ---- IsoZombie (read and written by the shared combat code) ----
    pub crawling: bool,
    /// `onFire`.
    pub on_fire: bool,
    /// `fireKillRate`.
    pub fire_kill_rate: f32,
    /// `fireSpreadProbability`.
    pub fire_spread_probability: i32,
    /// The `AttachAnim("Fire", ...)` sprite's random offsets while
    /// burning (the renderer draws it).
    pub fire_anim: Option<(i32, i32)>,
    pub sit_against_wall: bool,
    /// `lastHitPart` (`BodyPartType` index).
    pub last_hit_part: i32,
    /// `hitTime`.
    pub hit_time: i32,
    pub time_since_seen_flesh: f32,
    pub only_jaw_stab: bool,
    /// `target` (by character id).
    pub target: Option<i32>,
    pub always_knocked_down: bool,
    pub become_crawler: bool,
    /// `IsoZombie.staggerBack` (`setStaggerBack`).
    pub stagger_back: bool,
    pub hit_legs_while_on_floor: bool,
    pub hit_head_while_on_floor: i32,
}

impl Default for CharacterVars {
    fn default() -> Self {
        CharacterVars {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            last_x: 0.0,
            last_y: 0.0,
            last_z: 0.0,
            on_floor: false,
            close_killed: false,
            hit_from_angle: 0.0,
            dir: pz_world::square::IsoDirections::N,
            forward_x: 1.0,
            forward_y: 0.0,
            do_render: true,
            state_event_delay_timer: 0.0,
            hit_reaction: String::new(),
            collide_type: String::new(),
            bump_type: String::new(),
            bump_done: false,
            bump_fall: false,
            bump_staggered: false,
            bump_fall_type: String::new(),
            on_bed: false,
            near_wall_crouching: false,
            can_climb_down_rope: false,
            hit_from_behind: false,
            killed_by_fall: false,
            in_trees_no_bush: false,
            ranged_weapon_empty: false,
            move_delta: 1.0,
            turn_delta_normal: 1.0,
            turn_delta_running: 0.8,
            turn_delta_sprinting: 0.75,
            max_twist: 15.0,
            anim_angle: 0.0,
            twist: 0.0,
            target_twist: 0.0,
            shoulder_twist: 0.0,
            excess_twist: 0.0,
            num_twist_bones: 0,
            angle_step_delta: 0.0,
            angle_twist_delta: 0.0,
            turning: false,
            turning90: false,
            turning_around: false,
            moving: false,
            been_moving_for: 0.0,
            been_sprinting_for: 0.0,
            momentum_scalar: 0.0,
            has_timed_actions: false,
            critical_hit: false,
            knocked_down: false,
            was_knocked_down: false,
            falling: false,
            ragdoll_fall: false,
            fall_time: 0.0,
            last_fall_speed: 0.0,
            is_aiming: false,
            ignoring_aiming_input: false,
            aim_at_floor: false,
            aim_at_floor_target_distance: 0.0,
            current_vertical_aim_angle: 0.0,
            target_vertical_aim_angle: 0.0,
            performing_attack_animation: false,
            performing_shove_animation: false,
            performing_stomp_animation: false,
            performing_grapple_grab_animation: false,
            shove_stomp_anim: false,
            animating_backwards: false,
            grapple_throw_out_window: false,
            grapple_throw_over_fence: false,
            grapple_throw_into_container: false,
            editing_ragdoll: false,
            vehicle_collision: false,
            use_physic_hit_reaction: false,
            head_look_around: false,
            head_look_horizontal: 0.0,
            head_look_vertical: 0.0,
            hit_force: 0.0,
            hit_dir_x: 0.0,
            hit_dir_y: 0.0,
            hit_dir_enum: "FRONT",
            recoil_var_x: 0.0,
            recoil_var_y: 0.0,
            hide_equipped_hand_l: false,
            hide_equipped_hand_r: false,
            has_target: false,
            precision_aim_key_down: false,
            shout_type: String::new(),
            shout_item_model: String::new(),
            fall_damaging: false,
            fall_light: false,
            fall_hard: false,
            fall_more_than_hard: false,
            fall_impact_severity: "None",
            stagger_time_mod: 1.0,
            knockback_attack_mod: 1.0,
            avoid_damage: false,
            no_damage: false,
            last_hit_count: 0,
            reanimate_timer: 0.0,
            melee_delay: 0.0,
            recoil_delay: 0.0,
            aiming_delay: 0.0,
            use_hand_weapon: None,
            speed_mod: 1.0,
            combat_speed_modifier: 1.0,
            block_turning: false,
            ignore_movement: false,
            ignore_stagger_back: false,
            damaged_by_vehicle: false,
            climbing: false,
            on_kill_done: false,
            attacked_by: None,
            on_fire: false,
            fire_kill_rate: 0.0038,
            fire_spread_probability: 6,
            fire_anim: None,
            last_hit_character: None,
            do_shove: false,
            do_grapple: false,
            target_on_ground: None,
            combat_speed: 1.0,
            player_turn_delta: 0.0,
            block_movement: false,
            initiate_attack: false,
            walk_speed: 0.0,
            walk_injury: 0.0,
            run_speed: 0.0,
            idle_speed: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            windspeed: 0.0,
            wind_force: 0.0,
            ip_x: 0.0,
            ip_y: 0.0,
            attack_type: String::new(),
            attack_from_behind: false,
            under_vehicle: false,
            attack_started: false,
            banned_attacking: false,
            melee_pressed: false,
            grapple_pressed: false,
            can_let_go_of_grappled: false,
            weapon_type: String::new(),
            remote: false,
            performing_an_action: false,
            attack_variation_x: 0.0,
            attack_variation_y: 0.0,
            sneak_limp_speed_scale: 1.0,
            is_player_moving: false,
            path_strafing: false,
            over_encumbered: false,
            charge_time: 0.0,
            use_charge_delta: 0.0,
            melee_button_down: false,
            manual_floor_attack_button_down: false,
            click_sound: None,
            shootable: true,
            deferred_movement_enabled: true,
            player_attack_position: None,
            playing_death_sound: false,
            hide_weapon_model: false,
            collided_n: false,
            collided_s: false,
            collided_e: false,
            collided_w: false,
            last_collide_time: 0.0,
            ignore_auto_vault: false,
            has_obstacle_on_path: false,
            next_x: 0.0,
            next_y: 0.0,
            req_movement: (0.0, 0.0),
            impulse_x: 0.0,
            impulse_y: 0.0,
            alt_collide: false,
            collided_this_frame: false,
            collided_with_vehicle: false,
            collided_with_door: false,
            collided_object: None,
            last_square: None,
            ragdoll_simulation_active: false,
            no_clip: false,
            destroyed: false,
            last_collided_w: false,
            last_collided_n: false,
            crawling: false,
            sit_against_wall: false,
            last_hit_part: 0,
            hit_time: 0,
            time_since_seen_flesh: 100000.0,
            only_jaw_stab: false,
            target: None,
            always_knocked_down: false,
            become_crawler: false,
            stagger_back: false,
            hit_legs_while_on_floor: false,
            hit_head_while_on_floor: 0,
        }
    }
}

/// Which classes register a callback variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    /// `IsoGameCharacter` (every character).
    Character,
    /// `IsoLivingCharacter` (players and survivors).
    Living,
    /// `IsoPlayer` (players and animals).
    Player,
    /// `IsoZombie` (registered last, so it replaces the others).
    Zombie,
}

macro_rules! char_vars {
    ($($v:ident = $name:literal, $scope:ident, $kind:ident;)*) => {
        /// A callback variable slot (`AnimationVariableSlotCallback*`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum CharVar { $($v,)* }

        const DEFS: &[(&str, CharVar, Scope, AnimationVariableType)] = &[
            $(($name, CharVar::$v, Scope::$scope, AnimationVariableType::$kind),)*
        ];
    };
}

char_vars! {
    IsRendered = "isRendered", Character, Boolean;
    StateEventDelayTimer = "stateeventdelaytimer", Character, Float;
    HasHitReaction = "hashitreaction", Character, Boolean;
    HitReaction = "hitreaction", Character, String;
    CollideType = "collidetype", Character, String;
    FootInjuryType = "footInjuryType", Character, String;
    BumpType = "bumptype", Character, String;
    OnBed = "onbed", Character, Boolean;
    SittingOnFurniture = "sittingonfurniture", Character, Boolean;
    SitOnGround = "sitonground", Character, Boolean;
    NearWallCrouching = "nearWallCrouching", Character, Boolean;
    CanClimbDownRope = "canclimbdownrope", Character, Boolean;
    FromBehind = "frombehind", Character, Boolean;
    FallOnFront = "fallonfront", Character, Boolean;
    KilledByFall = "killedbyfall", Character, Boolean;
    InTrees = "intrees", Character, Boolean;
    Bumped = "bumped", Character, Boolean;
    BumpDone = "BumpDone", Character, Boolean;
    BumpFall = "BumpFall", Character, Boolean;
    BumpFallType = "BumpFallType", Character, String;
    BumpStaggered = "BumpStaggered", Character, Boolean;
    OnFloor = "bonfloor", Character, Boolean;
    IsProne = "isProne", Character, Boolean;
    IsGettingUp = "isGettingUp", Character, Boolean;
    RangedWeaponEmpty = "rangedweaponempty", Character, Boolean;
    FootInjury = "footInjury", Character, Boolean;
    ChopTreeSpeed = "ChopTreeSpeed", Character, Float;
    MoveDelta = "MoveDelta", Character, Float;
    TurnDelta = "TurnDelta", Character, Float;
    Angle = "angle", Character, Float;
    AnimAngle = "animAngle", Character, Float;
    Twist = "twist", Character, Float;
    TargetTwist = "targetTwist", Character, Float;
    MaxTwist = "maxTwist", Character, Float;
    ShoulderTwist = "shoulderTwist", Character, Float;
    ExcessTwist = "excessTwist", Character, Float;
    NumTwistBones = "numTwistBones", Character, Int;
    AngleStepDelta = "angleStepDelta", Character, Float;
    AngleTwistDelta = "angleTwistDelta", Character, Float;
    IsTurning = "isTurning", Character, Boolean;
    IsTurning90 = "isTurning90", Character, Boolean;
    IsTurningAround = "isTurningAround", Character, Boolean;
    Moving = "bMoving", Character, Boolean;
    BeenMovingFor = "beenMovingFor", Character, Float;
    PreviousState = "previousState", Character, String;
    MomentumScalar = "momentumScalar", Character, Float;
    HasTimedActions = "hasTimedActions", Character, Boolean;
    IsOverEncumbered = "isOverEncumbered", Character, Boolean;
    CriticalHit = "CriticalHit", Character, Boolean;
    KnockedDown = "bKnockedDown", Character, Boolean;
    Falling = "bfalling", Character, Boolean;
    Dead = "bdead", Character, Boolean;
    FallTime = "fallTime", Character, Float;
    FallSpeedSeverity = "fallSpeedSeverity", Character, String;
    Aim = "aim", Character, Boolean;
    AimAtFloor = "bAimAtfloor", Character, Boolean;
    AimAtFloorAmount = "aimAtFloorAmount", Character, Float;
    VerticalAimAngle = "verticalAimAngle", Character, Float;
    AttackAnim = "AttackAnim", Character, Boolean;
    ShoveAnim = "ShoveAnim", Character, Boolean;
    StompAnim = "StompAnim", Character, Boolean;
    IsStompAnim = "isStompAnim", Character, Boolean;
    PerformingHostileAnim = "PerformingHostileAnim", Character, Boolean;
    FireMode = "FireMode", Character, String;
    ShoutType = "ShoutType", Character, String;
    ShoutItemModel = "ShoutItemModel", Character, String;
    IsAnimatingBackwards = "isAnimatingBackwards", Character, Boolean;
    DoGrapple = "bDoGrapple", Character, Boolean;
    DoContinueGrapple = "bDoContinueGrapple", Character, Boolean;
    IsGrappling = "bIsGrappling", Character, Boolean;
    GrappleResult = "grappleResult", Character, String;
    SharedGrappleType = "sharedGrappleType", Character, String;
    SharedGrappleAnimNode = "sharedGrappleAnimNode", Character, String;
    SharedGrappleTime = "sharedGrappleTime", Character, Float;
    SharedGrappleFraction = "sharedGrappleFraction", Character, Float;
    GrappleOffsetForward = "grappleOffsetForward", Character, Float;
    GrappleOffsetBehaviour = "grappleOffsetBehaviour", Character, String;
    BearingToGrappledTarget = "bearingToGrappledTarget", Character, Float;
    BeingGrappled = "bBeingGrappled", Character, Boolean;
    GrappledBy = "grappledBy", Character, String;
    GrappledByType = "grappledByType", Character, String;
    GrappleGrabAnim = "GrappleGrabAnim", Character, Boolean;
    GrappleAnim = "GrappleAnim", Character, Boolean;
    AnyGrappleAnim = "AnyGrappleAnim", Character, Boolean;
    GrappleThrowOutWindow = "GrappleThrowOutWindow", Character, Boolean;
    GrappleThrowOverFence = "GrappleThrowOverFence", Character, Boolean;
    GrappleThrowIntoContainer = "GrappleThrowIntoContainer", Character, Boolean;
    CanRagdoll = "canRagdoll", Character, Boolean;
    IsEditingRagdoll = "isEditingRagdoll", Character, Boolean;
    IsRagdoll = "isRagdoll", Character, Boolean;
    IsSimulationActive = "isSimulationActive", Character, Boolean;
    IsUpright = "isUpright", Character, Boolean;
    IsOnBack = "isOnBack", Character, Boolean;
    IsRagdollFall = "isRagdollFall", Character, Boolean;
    IsVehicleCollision = "isVehicleCollision", Character, Boolean;
    UsePhysicHitReaction = "usePhysicHitReaction", Character, Boolean;
    UseRagdollVehicleCollision = "useRagdollVehicleCollision", Character, Boolean;
    HeadLookAround = "bHeadLookAround", Character, Boolean;
    LookHorizontal = "lookHorizontal", Character, Float;
    LookVertical = "lookVertical", Character, Float;
    HitForce = "hitforce", Character, Float;
    HitDir = "hitDir", Character, String;
    HitDirX = "hitDir.x", Character, Float;
    HitDirY = "hitDir.y", Character, Float;
    RecoilVarX = "recoilVarX", Character, Float;
    RecoilVarY = "recoilVarY", Character, Float;
    HideEquippedHandL = "hideEquippedHandL", Character, Boolean;
    HideEquippedHandR = "hideEquippedHandR", Character, Boolean;
    AimingMode = "aimingMode", Character, String;
    HasTarget = "hasTarget", Character, Boolean;
    IsUnarmed = "isUnarmed", Character, Boolean;
    IsMeleeWeaponEquipped = "isMeleeWeaponEquipped", Character, Boolean;
    IsRangedWeaponEquipped = "isRangedWeaponEquipped", Character, Boolean;
    LandLight = "bLandLight", Character, Boolean;
    LandLightMask = "bLandLightMask", Character, Boolean;
    HardFall = "bHardFall", Character, Boolean;
    HardFall2 = "bHardFall2", Character, Boolean;
    FallImpactSeverity = "fallImpactSeverity", Character, String;
    DoShove = "bDoShove", Living, Boolean;
    DoStomp = "bDoStomp", Living, Boolean;
    CombatSpeed = "CombatSpeed", Player, Float;
    PlayerTurnDelta = "TurnDelta", Player, Float;
    BlockMovement = "blockMovement", Player, Boolean;
    Sneaking = "sneaking", Player, Boolean;
    InitiateAttack = "initiateAttack", Player, Boolean;
    IsMoving = "isMoving", Player, Boolean;
    IsRunning = "isRunning", Player, Boolean;
    Run = "run", Player, Boolean;
    IsSprinting = "isSprinting", Player, Boolean;
    Sprint = "sprint", Player, Boolean;
    IsStrafing = "isStrafing", Player, Boolean;
    WalkSpeed = "WalkSpeed", Player, Float;
    WalkInjury = "WalkInjury", Player, Float;
    RunSpeed = "RunSpeed", Player, Float;
    IdleSpeed = "IdleSpeed", Player, Float;
    DeltaX = "DeltaX", Player, Float;
    DeltaY = "DeltaY", Player, Float;
    Windspeed = "Windspeed", Player, Float;
    WindForce = "WindForce", Player, Float;
    Ipx = "IPX", Player, Float;
    Ipy = "IPY", Player, Float;
    AttackType = "attacktype", Player, String;
    AttackFromBehind = "attackfrombehind", Player, Boolean;
    UnderVehicle = "bundervehicle", Player, Boolean;
    ReanimateTimer = "reanimatetimer", Player, Float;
    IsAttacking = "isattacking", Player, Boolean;
    BeenSprintingFor = "beensprintingfor", Player, Float;
    BannedAttacking = "bannedAttacking", Player, Boolean;
    MeleePressed = "meleePressed", Player, Boolean;
    GrapplePressed = "grapplePressed", Player, Boolean;
    CanLetGoOfGrappled = "canLetGoOfGrappled", Player, Boolean;
    Weapon = "Weapon", Player, String;
    Client = "bClient", Player, Boolean;
    Remote = "bRemote", Player, Boolean;
    IsPerformingAnAction = "IsPerformingAnAction", Player, Boolean;
    ShoveAiming = "bShoveAiming", Player, Boolean;
    GrappleAiming = "bGrappleAiming", Player, Boolean;
    AttackVariationX = "AttackVariationX", Player, Float;
    AttackVariationY = "AttackVariationY", Player, Float;
    SneakLimpSpeedScale = "sneakLimpSpeedScale", Player, Float;
    ZClient = "bClient", Zombie, Boolean;
    ZMultiplayer = "bMultiplayer", Zombie, Boolean;
    ZMovingNetwork = "bMovingNetwork", Zombie, Boolean;
    ZHitHeadType = "hitHeadType", Zombie, Int;
    ZRealState = "realState", Zombie, String;
    ZReanimatedForGrappleOnly = "bIsReanimatedForGrappleOnly", Zombie, Boolean;
    ZAttack = "battack", Zombie, Boolean;
    ZIsFacingTarget = "isFacingTarget", Zombie, Boolean;
    ZTargetSeenTime = "targetSeenTime", Zombie, Float;
    ZAttackVehicle = "battackvehicle", Zombie, Boolean;
    ZEatBodyTarget = "beatbodytarget", Zombie, Boolean;
    ZBecomeCrawler = "bbecomecrawler", Zombie, Boolean;
    ZFakeDead = "bfakedead", Zombie, Boolean;
    ZHasTarget = "bHasTarget", Zombie, Boolean;
    ZCanSeeTarget = "bCanSeeTarget", Zombie, Boolean;
    ZShouldSprint = "shouldSprint", Zombie, Boolean;
    ZKnockedDown = "bknockeddown", Zombie, Boolean;
    ZLunge = "blunge", Zombie, Boolean;
    ZStopLunging = "bstoplunging", Zombie, Boolean;
    ZPassengerExposed = "bpassengerexposed", Zombie, Boolean;
    ZTargetIsSmallVehicle = "bistargetissmallvehicle", Zombie, Boolean;
    ZReanimate = "breanimate", Zombie, Boolean;
    ZStaggerBack = "bstaggerback", Zombie, Boolean;
    ZUnbalancedLevel = "unbalancedLevel", Zombie, Float;
    ZThump = "bthump", Zombie, Boolean;
    ZUnderVehicle = "bundervehicle", Zombie, Boolean;
    ZBeingSteppedOn = "bBeingSteppedOn", Zombie, Boolean;
    ZDistanceToTarget = "distancetotarget", Zombie, Float;
    ZLastTargetSeen = "lasttargetseen", Zombie, Boolean;
    ZLungeTimer = "lungetimer", Zombie, Float;
    ZReanimateTimer = "reanimatetimer", Zombie, Float;
    ZTurnDirection = "turndirection", Zombie, String;
    ZAlerted = "alerted", Zombie, Boolean;
    ZWalkType = "zombiewalktype", Zombie, String;
    ZCrawlerType = "crawlertype", Zombie, Int;
    ZGetUpFromCrawl = "bGetUpFromCrawl", Zombie, Boolean;
    ZPlayerAttackPosition = "playerattackposition", Zombie, String;
    ZEatSpeed = "eatspeed", Zombie, Float;
    ZIsSitting = "issitting", Zombie, Boolean;
    ZKnifeDeath = "bKnifeDeath", Zombie, Boolean;
    ZJawStabAttach = "bJawStabAttach", Zombie, Boolean;
    ZPathFindPrediction = "bPathFindPrediction", Zombie, Boolean;
    ZCrawling = "bCrawling", Zombie, Boolean;
    ZAttackDidDamage = "AttackDidDamage", Zombie, Boolean;
    ZAttackOutcome = "AttackOutcome", Zombie, String;
}

/// Per handle index: the slot for (non-living characters, living
/// non-players, players). Later registrations replace earlier ones, as
/// `setVariable(slot)` does.
type SlotTable = HashMap<i32, [Option<CharVar>; 3]>;

thread_local! {
    static TABLE: RefCell<Option<SlotTable>> = const { RefCell::new(None) };
}

fn slot_index(kind: CharacterKind) -> usize {
    match kind {
        CharacterKind::Zombie => 0,
        CharacterKind::Survivor => 1,
        CharacterKind::Player | CharacterKind::Animal => 2,
    }
}

impl CharVar {
    /// The slot `kind`'s characters have under `handle`.
    pub fn lookup(kind: CharacterKind, handle: &AnimationVariableHandle) -> Option<CharVar> {
        TABLE.with(|t| {
            let mut t = t.borrow_mut();
            let table = t.get_or_insert_with(|| {
                let mut m: SlotTable = HashMap::new();
                for &(name, var, scope, _) in DEFS {
                    let h = AnimationVariableHandle::alloc(name).expect("named");
                    let e = m.entry(h.index()).or_default();
                    match scope {
                        Scope::Character => {
                            e[0] = Some(var);
                            e[1] = Some(var);
                            e[2] = Some(var);
                        }
                        Scope::Living => {
                            e[1] = Some(var);
                            e[2] = Some(var);
                        }
                        Scope::Player => e[2] = Some(var),
                        Scope::Zombie => e[0] = Some(var),
                    }
                }
                m
            });
            table.get(&handle.index()).and_then(|e| e[slot_index(kind)])
        })
    }

    /// The slot's name as registered.
    pub fn name(self) -> &'static str {
        DEFS.iter().find(|d| d.1 == self).map_or("", |d| d.0)
    }

    /// The slot's value type.
    pub fn kind(self) -> AnimationVariableType {
        DEFS.iter().find(|d| d.1 == self).map_or(AnimationVariableType::String, |d| d.3)
    }

    /// The value `clear()` restores (the registered default, else the
    /// type's zero).
    pub fn default_value(self) -> VariableValue {
        match self {
            CharVar::IsRendered => VariableValue::Bool(true),
            CharVar::ChopTreeSpeed | CharVar::MoveDelta | CharVar::TurnDelta => VariableValue::Float(1.0),
            CharVar::MaxTwist => VariableValue::Float(15.0),
            CharVar::CombatSpeed | CharVar::SneakLimpSpeedScale => VariableValue::Float(1.0),
            _ => match self.kind() {
                AnimationVariableType::String => VariableValue::String(String::new()),
                AnimationVariableType::Float => VariableValue::Float(0.0),
                AnimationVariableType::Int => VariableValue::Int(0),
                AnimationVariableType::Boolean => VariableValue::Bool(false),
            },
        }
    }

    /// The getter.
    pub fn get<I>(self, c: &GameCharacter<I>) -> VariableValue {
        if let Some(v) = crate::character::zombie::vars::get(self, c) {
            return v;
        }
        use VariableValue::{Bool as B, Float as F, Int, String as S};
        let v = &c.vars;
        match self {
            CharVar::IsRendered => B(v.do_render),
            CharVar::StateEventDelayTimer => F(v.state_event_delay_timer),
            CharVar::HasHitReaction => B(!v.hit_reaction.is_empty()),
            CharVar::HitReaction => S(v.hit_reaction.clone()),
            CharVar::CollideType => S(v.collide_type.clone()),
            CharVar::FootInjuryType => S(c.foot_injury_type().to_string()),
            CharVar::BumpType => S(v.bump_type.clone()),
            CharVar::OnBed => B(v.on_bed),
            CharVar::SittingOnFurniture => B(c.sitting_on_furniture),
            CharVar::SitOnGround => B(c.sit_on_ground),
            CharVar::NearWallCrouching => B(v.near_wall_crouching),
            CharVar::CanClimbDownRope => B(v.can_climb_down_rope),
            CharVar::FromBehind => B(v.hit_from_behind),
            CharVar::FallOnFront => B(c.environment.fall_on_front),
            CharVar::KilledByFall => B(v.killed_by_fall),
            CharVar::InTrees => B(c.is_in_trees_no_bush()),
            CharVar::Bumped => B(c.is_bumped()),
            CharVar::BumpDone => B(v.bump_done),
            CharVar::BumpFall => B(v.bump_fall),
            CharVar::BumpFallType => S(v.bump_fall_type.clone()),
            CharVar::BumpStaggered => B(v.bump_staggered),
            CharVar::OnFloor => B(v.on_floor),
            CharVar::IsProne => B(c.is_prone()),
            CharVar::IsGettingUp => B(c.is_getting_up()),
            CharVar::RangedWeaponEmpty => B(v.ranged_weapon_empty),
            CharVar::FootInjury => B(!pz_anim::vars::is_null_or_whitespace(c.foot_injury_type())),
            CharVar::ChopTreeSpeed => F(if c.has_trait_named("axeman") { 1.0 } else { 0.8 }),
            CharVar::MoveDelta => F(v.move_delta),
            CharVar::TurnDelta => F(c.turn_delta()),
            CharVar::Angle => F(c.direction_angle()),
            CharVar::AnimAngle => F(v.anim_angle),
            CharVar::Twist => F(v.twist),
            CharVar::TargetTwist => F(v.target_twist),
            CharVar::MaxTwist => F(v.max_twist),
            CharVar::ShoulderTwist => F(v.shoulder_twist),
            CharVar::ExcessTwist => F(v.excess_twist),
            CharVar::NumTwistBones => Int(v.num_twist_bones),
            CharVar::AngleStepDelta => F(v.angle_step_delta),
            CharVar::AngleTwistDelta => F(v.angle_twist_delta),
            CharVar::IsTurning => B(v.turning),
            CharVar::IsTurning90 => B(v.turning90),
            CharVar::IsTurningAround => B(v.turning_around),
            CharVar::Moving => B(v.moving),
            CharVar::BeenMovingFor => F(v.been_moving_for),
            CharVar::PreviousState => S(c.previous_action_context_state_name()),
            CharVar::MomentumScalar => F(v.momentum_scalar),
            CharVar::HasTimedActions => B(v.has_timed_actions),
            CharVar::IsOverEncumbered => B(c.environment_over_encumbered()),
            CharVar::CriticalHit => B(v.critical_hit),
            CharVar::KnockedDown => B(v.knocked_down),
            CharVar::Falling => B(c.is_falling()),
            CharVar::Dead => B(c.is_dead()),
            CharVar::FallTime => F(v.fall_time),
            CharVar::FallSpeedSeverity => S(fall_severity(v.last_fall_speed).to_string()),
            CharVar::Aim => B(c.is_aiming()),
            CharVar::AimAtFloor => B(v.aim_at_floor),
            CharVar::AimAtFloorAmount => F(c.aim_at_floor_amount()),
            CharVar::VerticalAimAngle => F(v.current_vertical_aim_angle),
            CharVar::AttackAnim => B(v.performing_attack_animation),
            CharVar::ShoveAnim => B(v.performing_shove_animation),
            CharVar::StompAnim => B(v.performing_stomp_animation),
            CharVar::IsStompAnim => B(v.shove_stomp_anim),
            CharVar::PerformingHostileAnim => B(c.is_performing_hostile_animation()),
            CharVar::FireMode => S(c.fire_mode()),
            CharVar::ShoutType => S(v.shout_type.clone()),
            CharVar::ShoutItemModel => S(v.shout_item_model.clone()),
            CharVar::IsAnimatingBackwards => B(v.animating_backwards),
            CharVar::DoGrapple => B(v.do_grapple),
            CharVar::DoContinueGrapple | CharVar::IsGrappling | CharVar::BeingGrappled | CharVar::GrappleAnim => B(false),
            CharVar::GrappleResult
            | CharVar::SharedGrappleType
            | CharVar::SharedGrappleAnimNode
            | CharVar::GrappledBy
            | CharVar::GrappledByType => S(String::new()),
            CharVar::GrappleOffsetBehaviour => S("NONE".into()),
            CharVar::SharedGrappleTime
            | CharVar::SharedGrappleFraction
            | CharVar::GrappleOffsetForward
            | CharVar::BearingToGrappledTarget => F(0.0),
            CharVar::GrappleGrabAnim => B(v.performing_grapple_grab_animation),
            CharVar::AnyGrappleAnim => B(v.performing_grapple_grab_animation),
            CharVar::GrappleThrowOutWindow => B(v.grapple_throw_out_window),
            CharVar::GrappleThrowOverFence => B(v.grapple_throw_over_fence),
            CharVar::GrappleThrowIntoContainer => B(v.grapple_throw_into_container),
            CharVar::CanRagdoll => B(c.ragdoll.can_ragdoll),
            CharVar::IsRagdoll => B(c.ragdoll.is_ragdolling),
            CharVar::IsSimulationActive => B(v.ragdoll_simulation_active),
            CharVar::IsUpright | CharVar::IsOnBack | CharVar::UseRagdollVehicleCollision => B(false),
            CharVar::IsEditingRagdoll => B(v.editing_ragdoll),
            CharVar::IsRagdollFall => B(v.ragdoll_fall),
            CharVar::IsVehicleCollision => B(v.vehicle_collision),
            CharVar::UsePhysicHitReaction => B(v.use_physic_hit_reaction),
            CharVar::HeadLookAround => B(v.head_look_around),
            CharVar::LookHorizontal => F(v.head_look_horizontal),
            CharVar::LookVertical => F(v.head_look_vertical),
            CharVar::HitForce => F(v.hit_force),
            CharVar::HitDir => S(v.hit_dir_enum.to_string()),
            CharVar::HitDirX => F(v.hit_dir_x),
            CharVar::HitDirY => F(v.hit_dir_y),
            CharVar::RecoilVarX => F(v.recoil_var_x),
            CharVar::RecoilVarY => F(v.recoil_var_y),
            CharVar::HideEquippedHandL => B(v.hide_equipped_hand_l),
            CharVar::HideEquippedHandR => B(v.hide_equipped_hand_r),
            CharVar::AimingMode => S(c.aiming_mode().to_string()),
            CharVar::HasTarget => B(v.has_target),
            CharVar::IsUnarmed => B(c.is_unarmed()),
            CharVar::IsMeleeWeaponEquipped => B(c.is_melee_weapon_equipped()),
            CharVar::IsRangedWeaponEquipped => B(c.is_ranged_weapon_equipped()),
            CharVar::LandLight => B(v.fall_damaging),
            CharVar::LandLightMask => B(v.fall_light && v.fall_damaging),
            CharVar::HardFall => B(v.fall_hard),
            CharVar::HardFall2 => B(v.fall_more_than_hard),
            CharVar::FallImpactSeverity => S(v.fall_impact_severity.to_string()),
            CharVar::DoShove => B(v.do_shove),
            CharVar::DoStomp => B(v.do_shove && v.aim_at_floor),
            CharVar::CombatSpeed => F(v.combat_speed),
            CharVar::PlayerTurnDelta => F(v.player_turn_delta),
            CharVar::BlockMovement => B(v.block_movement),
            CharVar::Sneaking => B(c.environment.sneaking),
            CharVar::InitiateAttack => B(v.initiate_attack),
            CharVar::IsMoving => B(v.is_player_moving),
            CharVar::IsRunning | CharVar::Run => B(c.running),
            CharVar::IsSprinting | CharVar::Sprint => B(c.environment.sprinting),
            CharVar::IsStrafing => B(v.path_strafing || c.is_aiming()),
            CharVar::WalkSpeed => F(v.walk_speed),
            CharVar::WalkInjury => F(v.walk_injury),
            CharVar::RunSpeed => F(v.run_speed),
            CharVar::IdleSpeed => F(v.idle_speed),
            CharVar::DeltaX => F(v.delta_x),
            CharVar::DeltaY => F(v.delta_y),
            CharVar::Windspeed => F(v.windspeed),
            CharVar::WindForce => F(v.wind_force),
            CharVar::Ipx => F(v.ip_x),
            CharVar::Ipy => F(v.ip_y),
            CharVar::AttackType => S(v.attack_type.clone()),
            CharVar::AttackFromBehind => B(v.attack_from_behind),
            CharVar::UnderVehicle => B(v.under_vehicle),
            CharVar::ReanimateTimer => F(v.reanimate_timer),
            CharVar::IsAttacking => B(!v.attack_type.is_empty()),
            CharVar::BeenSprintingFor => F(v.been_sprinting_for),
            CharVar::BannedAttacking => B(v.banned_attacking),
            CharVar::MeleePressed => B(v.melee_pressed),
            CharVar::GrapplePressed => B(v.grapple_pressed),
            CharVar::CanLetGoOfGrappled => B(v.can_let_go_of_grappled),
            CharVar::Weapon => S(v.weapon_type.clone()),
            CharVar::Client => B(false),
            CharVar::Remote => B(v.remote),
            CharVar::IsPerformingAnAction => B(v.performing_an_action),
            CharVar::ShoveAiming => B(c.is_aiming() && v.do_shove),
            CharVar::GrappleAiming => B(c.is_aiming() && v.do_grapple),
            CharVar::AttackVariationX => F(v.attack_variation_x),
            CharVar::AttackVariationY => F(v.attack_variation_y),
            CharVar::SneakLimpSpeedScale => F(v.sneak_limp_speed_scale),
            _ => B(false),
        }
    }

    /// The setter with the value converted to the slot's type; `false`
    /// for a read-only slot (the original warns and ignores it).
    pub fn set<I>(self, c: &mut GameCharacter<I>, value: VariableValue) -> bool {
        let value = value.converted_to(self.kind());
        if let Some(done) = crate::character::zombie::vars::set(self, c, &value) {
            return done;
        }
        let b = || value.as_bool();
        let f = || value.as_float();
        let s = || value.as_string();
        let v = &mut c.vars;
        match self {
            CharVar::IsRendered => v.do_render = b(),
            CharVar::HitReaction => {
                let s = s();
                if v.hit_reaction != s {
                    v.hit_reaction = s;
                }
            }
            CharVar::CollideType => v.collide_type = s(),
            CharVar::BumpType => c.set_bump_type(&s()),
            CharVar::OnBed => v.on_bed = b(),
            CharVar::SittingOnFurniture => c.sitting_on_furniture = b(),
            CharVar::SitOnGround => c.sit_on_ground = b(),
            CharVar::NearWallCrouching => v.near_wall_crouching = b(),
            CharVar::FromBehind => v.hit_from_behind = b(),
            CharVar::FallOnFront => c.environment.fall_on_front = b(),
            CharVar::KilledByFall => v.killed_by_fall = b(),
            CharVar::BumpDone => v.bump_done = b(),
            CharVar::BumpFall => v.bump_fall = b(),
            CharVar::BumpFallType => v.bump_fall_type = s(),
            CharVar::BumpStaggered => v.bump_staggered = b(),
            CharVar::OnFloor => v.on_floor = b(),
            CharVar::RangedWeaponEmpty => v.ranged_weapon_empty = b(),
            CharVar::MoveDelta => v.move_delta = f(),
            CharVar::TurnDelta => v.turn_delta_normal = f(),
            CharVar::Angle => c.set_direction_angle(f()),
            CharVar::MaxTwist => v.max_twist = f(),
            CharVar::IsTurning => v.turning = b(),
            CharVar::IsTurning90 => v.turning90 = b(),
            CharVar::IsTurningAround => v.turning_around = b(),
            CharVar::Moving => v.moving = b(),
            CharVar::MomentumScalar => v.momentum_scalar = f(),
            CharVar::CriticalHit => v.critical_hit = b(),
            CharVar::KnockedDown => v.knocked_down = b(),
            CharVar::AttackAnim => v.performing_attack_animation = b(),
            CharVar::ShoveAnim => v.performing_shove_animation = b(),
            CharVar::StompAnim => v.performing_stomp_animation = b(),
            CharVar::IsStompAnim => v.shove_stomp_anim = b(),
            CharVar::IsAnimatingBackwards => v.animating_backwards = b(),
            CharVar::GrappleResult | CharVar::SharedGrappleAnimNode => {}
            CharVar::GrappleOffsetForward | CharVar::GrappleOffsetBehaviour => {}
            CharVar::GrappleGrabAnim => v.performing_grapple_grab_animation = b(),
            CharVar::GrappleThrowOutWindow => v.grapple_throw_out_window = b(),
            CharVar::GrappleThrowOverFence => v.grapple_throw_over_fence = b(),
            CharVar::GrappleThrowIntoContainer => v.grapple_throw_into_container = b(),
            CharVar::IsEditingRagdoll => v.editing_ragdoll = b(),
            CharVar::IsRagdollFall => v.ragdoll_fall = b(),
            CharVar::IsVehicleCollision => v.vehicle_collision = b(),
            CharVar::UsePhysicHitReaction => v.use_physic_hit_reaction = b(),
            CharVar::RecoilVarX => v.recoil_var_x = f(),
            CharVar::RecoilVarY => v.recoil_var_y = f(),
            CharVar::HideEquippedHandL => v.hide_equipped_hand_l = b(),
            CharVar::HideEquippedHandR => v.hide_equipped_hand_r = b(),
            CharVar::CombatSpeed => v.combat_speed = f(),
            CharVar::PlayerTurnDelta => v.player_turn_delta = f(),
            CharVar::BlockMovement => v.block_movement = b(),
            CharVar::Sneaking => c.environment.sneaking = b(),
            CharVar::InitiateAttack => v.initiate_attack = b(),
            CharVar::IsRunning | CharVar::Run => c.running = b(),
            CharVar::IsSprinting | CharVar::Sprint => c.environment.sprinting = b(),
            CharVar::WalkSpeed => v.walk_speed = f(),
            CharVar::WalkInjury => v.walk_injury = f(),
            CharVar::RunSpeed => v.run_speed = f(),
            CharVar::IdleSpeed => v.idle_speed = f(),
            CharVar::DeltaX => v.delta_x = f(),
            CharVar::DeltaY => v.delta_y = f(),
            CharVar::Windspeed => v.windspeed = f(),
            CharVar::WindForce => v.wind_force = f(),
            CharVar::Ipx => v.ip_x = f(),
            CharVar::Ipy => v.ip_y = f(),
            CharVar::Weapon => v.weapon_type = s(),
            CharVar::IsPerformingAnAction => v.performing_an_action = b(),
            CharVar::SneakLimpSpeedScale => v.sneak_limp_speed_scale = f(),
            _ => return false,
        }
        true
    }

    /// Whether the slot has no setter.
    pub fn is_read_only(self) -> bool {
        if let Some(ro) = crate::character::zombie::vars::is_read_only(self) {
            return ro;
        }
        matches!(
            self,
            CharVar::StateEventDelayTimer
                | CharVar::HasHitReaction
                | CharVar::FootInjuryType
                | CharVar::CanClimbDownRope
                | CharVar::InTrees
                | CharVar::Bumped
                | CharVar::IsProne
                | CharVar::IsGettingUp
                | CharVar::FootInjury
                | CharVar::ChopTreeSpeed
                | CharVar::AnimAngle
                | CharVar::Twist
                | CharVar::TargetTwist
                | CharVar::ShoulderTwist
                | CharVar::ExcessTwist
                | CharVar::NumTwistBones
                | CharVar::AngleStepDelta
                | CharVar::AngleTwistDelta
                | CharVar::BeenMovingFor
                | CharVar::PreviousState
                | CharVar::HasTimedActions
                | CharVar::IsOverEncumbered
                | CharVar::Falling
                | CharVar::Dead
                | CharVar::FallTime
                | CharVar::FallSpeedSeverity
                | CharVar::Aim
                | CharVar::AimAtFloor
                | CharVar::AimAtFloorAmount
                | CharVar::VerticalAimAngle
                | CharVar::PerformingHostileAnim
                | CharVar::FireMode
                | CharVar::ShoutType
                | CharVar::ShoutItemModel
                | CharVar::DoGrapple
                | CharVar::DoContinueGrapple
                | CharVar::IsGrappling
                | CharVar::SharedGrappleType
                | CharVar::SharedGrappleTime
                | CharVar::SharedGrappleFraction
                | CharVar::BearingToGrappledTarget
                | CharVar::BeingGrappled
                | CharVar::GrappledBy
                | CharVar::GrappledByType
                | CharVar::GrappleAnim
                | CharVar::AnyGrappleAnim
                | CharVar::CanRagdoll
                | CharVar::IsRagdoll
                | CharVar::IsSimulationActive
                | CharVar::IsUpright
                | CharVar::IsOnBack
                | CharVar::UseRagdollVehicleCollision
                | CharVar::HeadLookAround
                | CharVar::LookHorizontal
                | CharVar::LookVertical
                | CharVar::HitForce
                | CharVar::HitDir
                | CharVar::HitDirX
                | CharVar::HitDirY
                | CharVar::AimingMode
                | CharVar::HasTarget
                | CharVar::IsUnarmed
                | CharVar::IsMeleeWeaponEquipped
                | CharVar::IsRangedWeaponEquipped
                | CharVar::LandLight
                | CharVar::LandLightMask
                | CharVar::HardFall
                | CharVar::HardFall2
                | CharVar::FallImpactSeverity
                | CharVar::DoShove
                | CharVar::DoStomp
                | CharVar::IsMoving
                | CharVar::IsStrafing
                | CharVar::AttackType
                | CharVar::AttackFromBehind
                | CharVar::UnderVehicle
                | CharVar::ReanimateTimer
                | CharVar::IsAttacking
                | CharVar::BeenSprintingFor
                | CharVar::BannedAttacking
                | CharVar::MeleePressed
                | CharVar::GrapplePressed
                | CharVar::CanLetGoOfGrappled
                | CharVar::Client
                | CharVar::Remote
                | CharVar::ShoveAiming
                | CharVar::GrappleAiming
                | CharVar::AttackVariationX
                | CharVar::AttackVariationY
        )
    }
}

/// `FallingConstants.getIsoImpactSpeedFromHeight(height)`.
pub fn iso_impact_speed_from_height(fall_height: f32) -> f32 {
    let impact_time = ((fall_height * 2.0 / 5.001_041_4) as f64).sqrt() as f32;
    5.001_041_4 * impact_time
}

/// `FallingConstants.getFallSeverity(isoFallSpeed)` (the `FallSeverity`
/// constant name).
pub fn fall_severity(speed: f32) -> &'static str {
    let falling = iso_impact_speed_from_height(0.35);
    let hard = iso_impact_speed_from_height(1.5);
    let severe = iso_impact_speed_from_height(2.5);
    let lethal = iso_impact_speed_from_height(3.5);
    if speed < falling {
        return "None";
    }
    if speed < hard {
        return "Light";
    }
    if speed < severe {
        return "Hard";
    }
    if speed < lethal {
        return "Severe";
    }
    "Lethal"
}

impl<I: 'static> GameCharacter<I> {
    /// `getFootInjuryType()`.
    pub fn foot_injury_type(&self) -> &'static str {
        if !self.is_player() {
            return "";
        }
        let foot = |f: &str| self.body_damage.parts.iter().find(|p| p.type_field == f);
        let (Some(l), Some(r)) = (foot("Foot_L"), foot("Foot_R")) else { return "" };
        let heavy = |p: &super::super::BodyPart| {
            p.have_bullet || p.burn_time > 5.0 || p.bitten || p.deep_wounded || p.splint || p.fracture_time > 0.0 || p.have_glass
        };
        if !self.running {
            if heavy(l) {
                return "leftheavy";
            }
            if heavy(r) {
                return "rightheavy";
            }
        }
        let light = |p: &super::super::BodyPart| p.scratch_time > 5.0 || p.cut_time > 7.0 || p.burn_time > 0.0;
        if light(l) {
            return "leftlight";
        }
        if light(r) {
            return "rightlight";
        }
        ""
    }

    /// Whether a trait whose registry path is `path` is set (the callback
    /// getters have no `ScriptManager`).
    pub(crate) fn has_trait_named(&self, path: &str) -> bool {
        self.character_traits.traits().iter().any(|(k, on)| *on && k.path().eq_ignore_ascii_case(path))
    }

    /// `isInTreesNoBush()`: never in `BumpedState`, else the square's
    /// answer.
    pub fn is_in_trees_no_bush(&self) -> bool {
        if self.is_current_state_id(crate::character::states::ids::BUMPED) {
            return false;
        }
        self.vars.in_trees_no_bush
    }

    /// `isBumped()`.
    pub fn is_bumped(&self) -> bool {
        !pz_anim::vars::is_null_or_whitespace(&self.vars.bump_type)
    }

    /// `setBumpType(bumpType)`: entering or leaving a bump sets
    /// `BumpStaggered`.
    pub fn set_bump_type(&mut self, bump_type: &str) {
        if equals_ignore_case(&self.vars.bump_type, bump_type) {
            self.vars.bump_type = bump_type.to_string();
            return;
        }
        let was = self.is_bumped();
        self.vars.bump_type = bump_type.to_string();
        let is = self.is_bumped();
        if is != was {
            self.vars.bump_staggered = is;
        }
    }

    /// `isProne()` (`IsoMovingObject`; `IsoZombie` adds its own states via
    /// [`Self::zombie_prone_states`]).
    pub fn is_prone(&self) -> bool {
        if self.vars.on_floor {
            return true;
        }
        if self.is_zombie() {
            return self.zombie_prone_states();
        }
        false
    }

    /// `IsoZombie.isProne()` past `onFloor`: crawling, eating, fake dead,
    /// on the ground, sitting against a wall.
    fn zombie_prone_states(&self) -> bool {
        use crate::character::states::ids;
        self.vars.crawling
            || self.is_current_state_id(ids::ZOMBIE_EAT_BODY)
            || self.is_current_state_id(ids::FAKE_DEAD_ZOMBIE)
            || self.is_current_state_id(ids::ZOMBIE_ON_GROUND)
            || self.vars.sit_against_wall
    }

    /// `isStanding()`.
    pub fn is_standing(&self) -> bool {
        !self.is_prone()
    }

    /// `isGettingUp()`: in `PlayerGetUpState` (players) or
    /// `ZombieGetUpState` (zombies).
    pub fn is_getting_up(&self) -> bool {
        use crate::character::states::ids;
        match self.kind {
            CharacterKind::Player | CharacterKind::Animal => self.current_state_id() == Some(ids::PLAYER_GET_UP),
            CharacterKind::Zombie => self.current_state_id() == Some(ids::ZOMBIE_GET_UP),
            CharacterKind::Survivor => false,
        }
    }

    /// `getTurnDelta()` (`IsoGameCharacter`; the `IsoPlayer` override only
    /// differs for the invisible-sprint debug cheat).
    pub fn turn_delta(&self) -> f32 {
        if self.environment.sprinting {
            return self.vars.turn_delta_sprinting;
        }
        if self.running {
            return self.vars.turn_delta_running;
        }
        self.vars.turn_delta_normal
    }

    /// `getDirectionAngleRadians()`: `Vector2.getDirection` of the forward
    /// direction.
    pub fn direction_angle_radians(&self) -> f32 {
        crate::character::zombie::math::direction(self.vars.forward_x, self.vars.forward_y)
    }

    /// `getDirectionAngle()`.
    pub fn direction_angle(&self) -> f32 {
        57.295776 * self.direction_angle_radians()
    }

    /// `setDirectionAngle(degrees)`.
    pub fn set_direction_angle(&mut self, degrees: f32) {
        let rads = std::f32::consts::PI / 180.0 * degrees;
        self.set_forward_direction((rads as f64).cos() as f32, (rads as f64).sin() as f32);
    }

    /// `setForwardDirection(x, y)`: normalized, with `dir` following
    /// (`IsoDirections.fromAngle`). A zero vector is left unnormalized (the
    /// original throws).
    pub fn set_forward_direction(&mut self, x: f32, y: f32) {
        if x * x + y * y == 0.0 {
            // The original sets the direction from the zero vector (east)
            // before it throws "Forward Direction cannot be zero length".
            self.vars.forward_x = x;
            self.vars.forward_y = y;
            self.vars.dir = iso_direction_from_angle(x, y);
            return;
        }
        // `Vector2.normalize()`: a length already within 1e-5 of 1 stays.
        (self.vars.forward_x, self.vars.forward_y) = crate::character::zombie::math::normalize(x, y);
        self.vars.dir = iso_direction_from_angle(x, y);
    }

    /// `isAiming()`.
    pub fn is_aiming(&self) -> bool {
        if self.is_performing_hostile_animation() {
            return true;
        }
        if self.vars.ignoring_aiming_input {
            return false;
        }
        self.vars.is_aiming
    }

    /// `isPerformingHostileAnimation()`.
    pub fn is_performing_hostile_animation(&self) -> bool {
        let v = &self.vars;
        v.performing_attack_animation
            || v.performing_shove_animation
            || v.performing_stomp_animation
            || v.performing_grapple_grab_animation
    }

    /// `getAimAtFloorAmount()`.
    pub fn aim_at_floor_amount(&self) -> f32 {
        let angle = self.vars.current_vertical_aim_angle;
        if (angle - 0.0).abs() <= 0.1 {
            return 0.0;
        }
        (-angle / 90.0).clamp(0.0, 1.0)
    }

    /// `isFalling()` (the chunk's minimum level is taken as 0).
    pub fn is_falling(&self) -> bool {
        (self.vars.falling || self.vars.last_fall_speed > iso_impact_speed_from_height(0.35)) && self.vars.z > 0.0
    }

    /// `getFireMode()`.
    pub fn fire_mode(&self) -> String {
        self.items
            .primary_hand_item()
            .filter(|i| super::super::items::is_hand_weapon(i))
            .and_then(|i| i.weapon_data().and_then(|w| w.fire_mode.clone()))
            .unwrap_or_default()
    }

    /// `isUnarmed()`.
    pub fn is_unarmed(&self) -> bool {
        !self.items.primary_hand_item().is_some_and(super::super::items::is_hand_weapon)
    }

    /// `isMeleeWeaponEquipped()`.
    pub fn is_melee_weapon_equipped(&self) -> bool {
        self.items.primary_hand_item().is_some_and(|i| super::super::items::is_hand_weapon(i) && !i.is_ranged())
    }

    /// `isRangedWeaponEquipped()`.
    pub fn is_ranged_weapon_equipped(&self) -> bool {
        self.items.primary_hand_item().is_some_and(|i| super::super::items::is_hand_weapon(i) && i.is_ranged())
    }

    /// `getAimingMode()` (the `AimingMode` constant name).
    pub fn aiming_mode(&self) -> &'static str {
        let target = self.vars.has_target;
        if !self.is_aiming() {
            return if self.sitting_on_furniture { "MELEE" } else { "NOT_AIMING" };
        }
        if self.is_melee_weapon_equipped() {
            return if target { "MELEE_TARGET_FOUND" } else { "MELEE" };
        }
        if self.is_ranged_weapon_equipped() {
            if self.vars.precision_aim_key_down {
                return if target { "RANGED_PRECISE_TARGET_FOUND" } else { "RANGED_PRECISE" };
            }
            return if target { "RANGED_TARGET_FOUND" } else { "RANGED" };
        }
        if self.is_unarmed() {
            return if target { "UNARMED_TARGET_FOUND" } else { "UNARMED" };
        }
        "NOT_AIMING"
    }

    /// `isOverEncumbered()` needs the inventory weight with the script
    /// manager; the callback slot reads the flag the update stores.
    fn environment_over_encumbered(&self) -> bool {
        self.vars.over_encumbered
    }

    /// The callback slot `kind`'s characters have under `handle`.
    pub fn callback_slot(&self, handle: &AnimationVariableHandle) -> Option<CharVar> {
        CharVar::lookup(self.kind, handle)
    }
}

/// `IsoDirections.fromAngle(dx, dy)`.
pub fn iso_direction_from_angle(x: f32, y: f32) -> pz_world::square::IsoDirections {
    let angle = (y as f64).atan2(x as f64) as f32;
    let i = (6 - (8.5f32 + 1.273_239_5 * angle) as i32) & 7;
    pz_world::square::IsoDirections::ALL[i as usize]
}

impl<I> AnimationVariableSource for GameCharacter<I> {
    /// `IsoGameCharacter.getVariable(handle)`: the action states'
    /// variables, then the callback slots, then the game variables.
    fn get_variable(&self, handle: &AnimationVariableHandle) -> Option<VariableValue> {
        if let Some(v) = self.state.action_context.get_variable(handle) {
            return Some(v);
        }
        if let Some(var) = self.callback_slot(handle) {
            return Some(var.get(self));
        }
        self.game_variables.get_variable(handle)
    }

    fn set_existing_variable(&mut self, handle: &AnimationVariableHandle, value: VariableValue) -> bool {
        if let Some(var) = self.callback_slot(handle) {
            var.set(self, value);
            return true;
        }
        self.game_variables.set_existing_variable(handle, value)
    }

    /// `setVariable(key, value)`: the existing slot takes the value (a
    /// read-only callback slot ignores it); otherwise a slot of the value's
    /// type is created.
    fn set_variable(&mut self, name: &str, value: VariableValue) -> bool {
        let Some(h) = AnimationVariableHandle::alloc(name) else { return true };
        if let Some(var) = self.callback_slot(&h) {
            var.set(self, value);
            return true;
        }
        self.game_variables.set_variable(name, value)
    }

    /// `clearVariable(key)`: a writable callback slot gets its default.
    fn clear_variable(&mut self, name: &str) -> bool {
        let Some(h) = AnimationVariableHandle::alloc(name) else { return true };
        if let Some(var) = self.callback_slot(&h) {
            if !var.is_read_only() {
                var.set(self, var.default_value());
            }
            return true;
        }
        self.game_variables.clear_variable(name)
    }
}

impl<I: 'static> GameCharacter<I> {
    /// `getVariableString(key)`.
    pub fn variable_string(&self, key: &str) -> String {
        self.get_variable_string(key)
    }

    /// `getVariableFloat(key, default)`.
    pub fn variable_float(&self, key: &str, default: f32) -> f32 {
        self.get_variable_float(key, default)
    }

    /// `getVariableBoolean(key)`.
    pub fn variable_boolean(&self, key: &str) -> bool {
        self.get_variable_boolean(key)
    }

    /// `getVariableBoolean(key, default)`.
    pub fn variable_boolean_or(&self, key: &str, default: bool) -> bool {
        self.get_variable_named(key).map_or(default, |v| v.as_bool())
    }

    /// `setVariable(key, String)`.
    pub fn set_var_str(&mut self, key: &str, value: &str) {
        self.set_variable_string(key, value);
    }

    /// `setVariable(key, boolean)`.
    pub fn set_var_bool(&mut self, key: &str, value: bool) {
        self.set_variable_bool(key, value);
    }

    /// `setVariable(key, float)`.
    pub fn set_var_float(&mut self, key: &str, value: f32) {
        self.set_variable_float(key, value);
    }

    /// `clearVariable(key)`.
    pub fn clear_var(&mut self, key: &str) {
        self.clear_variable(key);
    }

    /// `clearVariables()`: every slot of the game variables (callback slots
    /// back to their defaults).
    pub fn clear_variables(&mut self) {
        for &(_, var, _, _) in DEFS {
            let Some(h) = AnimationVariableHandle::alloc(var.name()) else { continue };
            if self.callback_slot(&h) == Some(var) && !var.is_read_only() {
                var.set(self, var.default_value());
            }
        }
        let names: Vec<String> = self.game_variables.game_variables().map(|(k, _)| k.to_string()).collect();
        for n in names {
            self.game_variables.clear_variable(&n);
        }
    }

    /// `GetVariable(key)` (`getVariableString`).
    pub fn get_variable_java(&self, key: &str) -> String {
        self.get_variable_string(key)
    }
}

/// `String.valueOf(float)` for callers formatting variable values.
pub fn float_string(f: f32) -> String {
    java_float_to_string(f)
}
