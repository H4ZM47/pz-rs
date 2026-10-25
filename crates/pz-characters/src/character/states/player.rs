//! The player states that only touch the owner (and the attacker by id):
//! `PlayerKnockedDown`, `PlayerFallDownState`, `PlayerHitReactionState`,
//! `PlayerHitReactionPVPState`, `PlayerOnGroundState`, `PlayerGetUpState`,
//! `CollideWithWallState`, `PlayerAimState`, `PlayerStrafeState`.
//!
//! `setParams` is the network replication of the state; for a local
//! character (single player) it stores the values in the state's params,
//! which is what these ports do.

use pz_anim::{AnimEventRecord, AnimationVariableSource};

use super::stagger_back::parse_boolean;
use super::{ids, STRAFE_SPEED};
use crate::character::state::{Param, State, StateCx, StateId, StateRef, SyncFlags};
use crate::character::{GameCharacter, SimEvent};

macro_rules! singleton {
    ($ty:ident, $id:expr, $fn:ident) => {
        pub struct $ty;
        impl $ty {
            pub const ID: StateId = $id;
        }
        /// The singleton (`instance()`).
        pub fn $fn<I: 'static>() -> StateRef<I> {
            static S: $ty = $ty;
            &S
        }
    };
}

singleton!(PlayerKnockedDown, ids::PLAYER_KNOCKED_DOWN, knocked_down);
singleton!(PlayerFallDownState, ids::PLAYER_FALL_DOWN, fall_down);
singleton!(PlayerHitReactionState, ids::PLAYER_HIT_REACTION, hit_reaction);
singleton!(PlayerHitReactionPVPState, ids::PLAYER_HIT_REACTION_PVP, hit_reaction_pvp);
singleton!(PlayerOnGroundState, ids::PLAYER_ON_GROUND, on_ground);
singleton!(PlayerGetUpState, ids::PLAYER_GET_UP, get_up);
singleton!(CollideWithWallState, ids::COLLIDE_WITH_WALL, collide_with_wall);
singleton!(PlayerAimState, ids::PLAYER_AIM, aim);
singleton!(PlayerStrafeState, ids::PLAYER_STRAFE, strafe);

fn param_bool(e: &AnimEventRecord) -> bool {
    parse_boolean(e.event.parameter_value.as_deref())
}

fn is(e: &AnimEventRecord, name: &str) -> bool {
    e.event.event_name.eq_ignore_ascii_case(name)
}

/// `owner.die()` for a dead owner: `Kill` once, then the corpse (the
/// world's).
fn die<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
    if !owner.vars.on_kill_done {
        let k = owner.vars.attacked_by;
        owner.kill_by(k, cx);
    }
    cx.ctx.event(SimEvent::Die);
}

impl<I: 'static> State<I> for PlayerKnockedDown {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        owner.vars.block_movement = true;
        owner.vars.hit_reaction = String::new();
        owner.vars.block_turning = true;
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if owner.is_dead() {
            if !cx.ctx.is_server() && !cx.ctx.is_client() {
                owner.kill_by(None, cx);
            }
        } else {
            owner.vars.reanimate_timer -= cx.ctx.thirty_fps_multiplier();
        }
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        if is(e, "FallOnFront") || is(e, "FallOnBack") {
            owner.environment.fall_on_front = param_bool(e);
        }
        if is(e, "setSitOnGround") {
            owner.sit_on_ground = param_bool(e);
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = false;
        owner.vars.block_movement = false;
        owner.vars.knocked_down = false;
        owner.vars.on_floor = true;
        owner.vars.block_turning = false;
        owner.vars.was_knocked_down = true;
    }
}

/// `PlayerFallDownState.ON_FLOOR`, `KNOCKED_DOWN`, `DEAD`.
pub const FALL_ON_FLOOR: Param<bool> = Param::new(ids::PLAYER_FALL_DOWN, "on_floor", || false);
pub const FALL_KNOCKED_DOWN: Param<bool> = Param::new(ids::PLAYER_FALL_DOWN, "knocked_down", || false);
pub const FALL_DEAD: Param<bool> = Param::new(ids::PLAYER_FALL_DOWN, "dead", || false);

fn fall_down_params<I: 'static>(owner: &mut GameCharacter<I>) {
    let (f, k, d) = (owner.vars.on_floor, owner.vars.knocked_down, owner.is_dead());
    FALL_ON_FLOOR.set(&mut owner.state.params, f);
    FALL_KNOCKED_DOWN.set(&mut owner.state.params, k);
    FALL_DEAD.set(&mut owner.state.params, d);
}

impl<I: 'static> State<I> for PlayerFallDownState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, false, false)
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        owner.clear_variable("bKnockedDown");
        if owner.is_dead() && !cx.ctx.is_server() && !cx.ctx.is_client() {
            owner.kill_by(None, cx);
        }
        fall_down_params(owner);
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = false;
        owner.vars.on_floor = true;
        fall_down_params(owner);
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        if cx.ctx.is_client() && is(e, "FallOnFront") {
            owner.environment.fall_on_front = param_bool(e);
        }
    }
}

/// The attacker's reactions in the hit-reaction states' events.
fn attacker_reaction<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord, pvp: bool) {
    let attacker = owner.vars.attacked_by;
    if is(e, "PushAwayZombie") {
        if let Some(a) = attacker.and_then(|id| cx.world.character_mut(id)) {
            a.vars.hit_force = 0.03;
            if a.is_zombie() {
                a.vars.player_attack_position = None;
                a.vars.stagger_back = true;
            }
        }
    }
    if is(e, "Defend") {
        if let Some(a) = attacker.and_then(|id| cx.world.character_mut(id)) {
            a.vars.hit_reaction = "BiteDefended".into();
        }
    }
    if is(e, "DeathSound") {
        if owner.vars.playing_death_sound {
            return;
        }
        owner.vars.playing_death_sound = true;
        if owner.is_player() && !pvp {
            cx.ctx.event(SimEvent::PlayerVoiceSound("DeathEaten"));
        } else {
            let female = owner.descriptor.as_ref().is_some_and(|d| d.borrow().is_female());
            let s = if female { "FemaleBeingEatenDeath" } else { "MaleBeingEatenDeath" };
            cx.ctx.event(SimEvent::PlaySound(s.into()));
        }
    }
    if is(e, "Death") {
        owner.vars.on_floor = true;
        if !cx.ctx.is_server() {
            owner.kill_by(attacker, cx);
        }
    }
}

impl<I: 'static> State<I> for PlayerHitReactionState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        if owner.environment.busy_with_action || owner.vars.has_timed_actions {
            cx.ctx.event(SimEvent::ForceStopAction);
        }
        owner.vars.is_aiming = false;
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = false;
        owner.vars.hit_reaction = String::new();
    }
    /// Only with a zombie attacker.
    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        let zombie_attacker = owner.vars.attacked_by.and_then(|id| cx.world.character_mut(id)).is_some_and(|a| a.is_zombie());
        if !zombie_attacker {
            return;
        }
        attacker_reaction(owner, cx, e, false);
    }
}

impl<I: 'static> State<I> for PlayerHitReactionPVPState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if owner.environment.busy_with_action || owner.vars.has_timed_actions {
            cx.ctx.event(SimEvent::ForceStopAction);
        }
        owner.sit_on_ground = false;
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = false;
        owner.vars.hit_reaction = String::new();
        owner.set_variable_bool("hitpvp", false);
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        attacker_reaction(owner, cx, e, true);
    }
}

impl<I: 'static> State<I> for PlayerOnGroundState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        owner.vars.block_movement = true;
        owner.set_variable_bool("bAnimEnd", false);
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if owner.is_dead() {
            die(owner, cx);
        } else {
            owner.vars.reanimate_timer -= cx.ctx.thirty_fps_multiplier();
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = false;
        owner.vars.block_movement = false;
    }
}

/// `PlayerGetUpState.FORCE`, `MOVING`, `ISO_DIRECTION`.
pub const GET_UP_FORCE: Param<bool> = Param::new(ids::PLAYER_GET_UP, "force", || false);
pub const GET_UP_MOVING: Param<bool> = Param::new(ids::PLAYER_GET_UP, "moving", || false);
pub const GET_UP_ISO_DIRECTION: Param<Option<pz_world::square::IsoDirections>> =
    Param::new(ids::PLAYER_GET_UP, "iso_direction", || None);

impl<I: 'static> State<I> for PlayerGetUpState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, false, false)
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let (f, m) = (owner.get_variable_boolean("forceGetUp"), owner.get_variable_boolean("isMoving"));
        GET_UP_FORCE.set(&mut owner.state.params, f);
        GET_UP_MOVING.set(&mut owner.state.params, m);
        owner.vars.ignore_movement = true;
        owner.vars.initiate_attack = false;
        owner.vars.attack_started = false;
        owner.vars.attack_type = String::new();
        owner.vars.block_movement = true;
        owner.vars.force_run = false;
        owner.vars.force_sprint = false;
        let quick = owner.get_variable_boolean("pressedRunButton");
        owner.set_variable_bool("getUpQuick", quick);
        if owner.moodles.moodle_level_of(cx.sm, "PANIC").unwrap_or(0) > 1 {
            owner.set_variable_bool("getUpQuick", true);
        }
        if owner.get_variable_boolean("pressedMovement") {
            owner.set_variable_bool("getUpWalk", true);
        }
        let dir = owner.vars.dir;
        GET_UP_ISO_DIRECTION.set(&mut owner.state.params, Some(dir));
        if owner.get_variable_boolean("SittingOnFurniture") {
            // faceDirection(getDir()) and the seat's StandUp sound.
            let (dx, dy) = crate::character::combat::direction_vector(dir);
            owner.set_forward_direction(dx, dy);
            owner.vars.dir = dir;
            cx.ctx.event(SimEvent::PlaySound("StandUpFabric".into()));
        } else {
            cx.ctx.event(SimEvent::PlaySound("SurfaceStandUp".into()));
        }
        if cx.ctx.is_client() {
            owner.vars.knocked_down = false;
        }
        if owner.vars.was_knocked_down {
            owner.vars.block_turning = true;
        }
    }
    fn execute(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        if !owner.vars.on_bed {
            return;
        }
        let dir_str = owner.get_variable_string("OnBedDirection");
        let start = GET_UP_ISO_DIRECTION.get(&mut owner.state.params);
        use pz_world::square::IsoDirections as D;
        let angle: f32 = match dir_str.as_str() {
            "Foot" => return,
            "FootLeft" | "HeadLeft" => match start {
                Some(D::N) => 180.0,
                Some(D::W) => 90.0,
                Some(D::E) => 270.0,
                _ => 0.0,
            },
            "FootRight" | "HeadRight" => match start {
                Some(D::N) => 0.0,
                Some(D::W) => 270.0,
                Some(D::E) => 90.0,
                _ => 180.0,
            },
            _ => 0.0,
        };
        owner.vars.block_turning = true;
        let rad = angle * (std::f32::consts::PI / 180.0);
        let (vx, vy) = ((rad as f64).cos() as f32, (rad as f64).sin() as f32);
        let (x, y) = (owner.vars.x + vx, owner.vars.y + vy);
        let (dx, dy) = (x - owner.vars.x, y - owner.vars.y);
        if dx.abs() > 1.0e-7 || dy.abs() > 1.0e-7 {
            owner.set_forward_direction(dx, dy);
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.clear_variable("getUpWalk");
        if owner.vars.on_bed {
            owner.vars.block_turning = false;
            owner.vars.hide_weapon_model = false;
        }
        if owner.sitting_on_furniture || owner.get_variable_boolean("sitonground") {
            owner.vars.hide_weapon_model = false;
        }
        owner.vars.ignore_movement = false;
        owner.environment.fall_on_front = false;
        owner.vars.on_floor = false;
        owner.vars.block_movement = false;
        // TODO(world): the chair's `setSatChair(false)` and the eject from
        // solid furniture.
        owner.vars.on_bed = false;
        owner.sitting_on_furniture = false;
        owner.path.sit_on_furniture_direction = None;
        owner.sit_on_ground = false;
        if owner.vars.was_knocked_down {
            owner.vars.was_knocked_down = false;
            owner.vars.block_turning = false;
        }
    }
    fn is_processed_on_exit(&self) -> bool {
        true
    }
}

/// `CollideWithWallState.COLLIDE_TYPE`.
pub const COLLIDE_TYPE: Param<Option<String>> = Param::new(ids::COLLIDE_WITH_WALL, "collide_type", || Some("wall".into()));

impl<I: 'static> State<I> for CollideWithWallState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, false, true, false)
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        use pz_world::square::IsoDirections as D;
        owner.vars.ignore_movement = true;
        if owner.is_player() {
            owner.vars.is_aiming = false;
        }
        for (hit, d) in [(owner.vars.collided_n, D::N), (owner.vars.collided_s, D::S), (owner.vars.collided_e, D::E), (owner.vars.collided_w, D::W)] {
            if hit {
                owner.vars.dir = d;
            }
        }
        owner.vars.collide_type = "wall".into();
        let t = Some(owner.vars.collide_type.clone());
        COLLIDE_TYPE.set(&mut owner.state.params, t);
    }
    fn execute(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.last_collide_time = 70.0;
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.clear_variable("PlayerVoiceSound");
        owner.vars.collide_type = String::new();
        owner.vars.ignore_movement = false;
        COLLIDE_TYPE.set(&mut owner.state.params, None);
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        let param = e.event.parameter_value.clone().unwrap_or_default();
        if is(e, "PlayCollideSound") {
            cx.ctx.event(SimEvent::PlaySound(param.clone()));
        }
        if is(e, "PlayerVoiceSound") {
            if owner.get_variable_boolean("PlayerVoiceSound") || !owner.is_player() {
                return;
            }
            owner.set_variable_bool("PlayerVoiceSound", true);
            cx.ctx.event(SimEvent::PlayerVoiceSoundNamed(param));
        }
    }
}

/// `PlayerAimState.AIM`, `AIM_FLOOR`, `AIM_FLOOR_DISTANCE`.
pub const AIM: Param<Option<bool>> = Param::new(ids::PLAYER_AIM, "aim", || None);
pub const AIM_FLOOR: Param<bool> = Param::new(ids::PLAYER_AIM, "aim_floor", || false);
pub const AIM_FLOOR_DISTANCE: Param<f32> = Param::new(ids::PLAYER_AIM, "aim_floor_distance", || 0.0);

fn aim_params<I: 'static>(owner: &mut GameCharacter<I>) {
    let a = owner.is_aiming();
    let (f, d) = (owner.vars.aim_at_floor, owner.vars.aim_at_floor_target_distance);
    AIM.set(&mut owner.state.params, Some(a));
    AIM_FLOOR.set(&mut owner.state.params, f);
    AIM_FLOOR_DISTANCE.set(&mut owner.state.params, d);
}

impl<I: 'static> State<I> for PlayerAimState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, false, false)
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        aim_params(owner);
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        aim_params(owner);
    }
    fn is_processed_on_enter(&self) -> bool {
        true
    }
    fn is_processed_on_exit(&self) -> bool {
        true
    }
}

/// `PlayerStrafeState.AIM`.
pub const STRAFE_AIM: Param<bool> = Param::new(ids::PLAYER_STRAFE, "aim", || false);

fn strafe_params<I: 'static>(owner: &mut GameCharacter<I>) {
    let a = owner.is_aiming();
    let s = owner.get_variable_float("StrafeSpeed", 1.0);
    STRAFE_AIM.set(&mut owner.state.params, a);
    STRAFE_SPEED.set(&mut owner.state.params, s);
}

impl<I: 'static> State<I> for PlayerStrafeState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, false, false)
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        strafe_params(owner);
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        strafe_params(owner);
    }
    fn is_processed_on_enter(&self) -> bool {
        true
    }
    fn is_processed_on_exit(&self) -> bool {
        true
    }
}
