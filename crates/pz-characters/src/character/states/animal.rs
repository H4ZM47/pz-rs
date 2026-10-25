//! The twelve `zombie.ai.states.animals` states. Every one is constructed
//! with all four sync flags false. Their bodies call the animal's
//! behaviour (`character/animal/behavior.rs`) and `IsoAnimal` methods
//! (`character/animal/actions.rs`).
//!
//! What they do through animation events (`idleActionEnd`,
//! `AttackConnect`, `ActiveAnimFinishing`, `Climbed`, `ClimbDone`) is in
//! their `anim_event`; the sound events (`PlayBreedSound`,
//! `PlayDeathSound`, the eating sounds) and footsteps are FMOD's and only
//! clear what the original clears.

use std::cell::Cell;

use pz_anim::AnimEventRecord;
use pz_animals::{character_ref_id, AnimalZoneJunction, BehaviorAction, BehaviorObject};
use pz_world::square::IsoDirections;

use crate::character::animal::actions::{dist_proper, sq_dist_proper, sq_dist_proper_xy};
use crate::character::state::{Param, State, StateCx, StateId, StateRef, SyncFlags};
use crate::character::zombie::math::{
    dir_vector, direction, direction_neg, fastfloor, normalize, rot_left,
};
use crate::character::GameCharacter;

use super::ids;

const NONE: SyncFlags = SyncFlags::new(false, false, false, false);

fn is(e: &AnimEventRecord, name: &str) -> bool {
    e.event.event_name.eq_ignore_ascii_case(name)
}

/// `changeState(state, null, false)` to a registered animal state.
fn change_to<I: 'static>(owner: &mut GameCharacter<I>, s: StateRef<I>, cx: &mut StateCx<'_, I>) {
    owner.change_state(Some(s), cx);
}

macro_rules! animal_state {
    ($ty:ident, $id:expr, $f:ident) => {
        pub struct $ty;
        static $f: $ty = $ty;
        impl $ty {
            pub const ID: StateId = $id;
            /// `instance()`.
            pub fn instance<I: 'static>() -> StateRef<I> {
                &$f
            }
        }
    };
}

animal_state!(AnimalAlertedState, ids::ANIMAL_ALERTED, ALERTED);
animal_state!(AnimalAttackState, ids::ANIMAL_ATTACK, ATTACK);
animal_state!(
    AnimalClimbOverFenceState,
    ids::ANIMAL_CLIMB_OVER_FENCE,
    CLIMB
);
animal_state!(AnimalEatState, ids::ANIMAL_EAT, EAT);
animal_state!(AnimalFalldownState, ids::ANIMAL_FALLDOWN, FALLDOWN);
animal_state!(AnimalFollowWallState, ids::ANIMAL_FOLLOW_WALL, FOLLOW_WALL);
animal_state!(
    AnimalHitReactionState,
    ids::ANIMAL_HIT_REACTION,
    HIT_REACTION
);
animal_state!(AnimalIdleState, ids::ANIMAL_IDLE, IDLE);
animal_state!(AnimalOnGroundState, ids::ANIMAL_ON_GROUND, ON_GROUND);
animal_state!(AnimalPathFindState, ids::ANIMAL_PATH_FIND, PATH_FIND);
animal_state!(AnimalWalkState, ids::ANIMAL_WALK, WALK);
animal_state!(AnimalZoneState, ids::ANIMAL_ZONE, ZONE);

/// The twelve states, for the registry.
pub fn all<I: 'static>() -> Vec<StateRef<I>> {
    vec![
        AnimalAlertedState::instance(),
        AnimalAttackState::instance(),
        AnimalClimbOverFenceState::instance(),
        AnimalEatState::instance(),
        AnimalFalldownState::instance(),
        AnimalFollowWallState::instance(),
        AnimalHitReactionState::instance(),
        AnimalIdleState::instance(),
        AnimalOnGroundState::instance(),
        AnimalPathFindState::instance(),
        AnimalWalkState::instance(),
        AnimalZoneState::instance(),
    ]
}

/// `isLocalPlayer()` of an animal.
fn local<I: 'static>(owner: &GameCharacter<I>, cx: &StateCx<'_, I>) -> bool {
    owner.animal_is_local(cx.ctx)
}

// ---- AnimalIdleState ----

/// `AnimalIdleState.PET_SOUND_TIME`.
pub const PET_SOUND_TIME: Param<i64> = Param::new(ids::ANIMAL_IDLE, "pet_sound_time", || 0);

impl<I: 'static> State<I> for AnimalIdleState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if !local(owner, cx) {
            return;
        }
        owner.behavior_wander_idle(cx);
        if let Some(t) = owner.af().animal.eat_from_trough {
            owner.face_animal_object(t, cx);
        }
        if let Some(t) = owner.af().animal.drink_from_trough {
            owner.face_animal_object(t, cx);
        }
        if owner.animal_is_eating() {
            change_to(owner, AnimalEatState::instance(), cx);
        }
        if owner.variable_boolean("bMoving") {
            change_to(owner, AnimalWalkState::instance(), cx);
        }
    }
    fn anim_event(
        &self,
        owner: &mut GameCharacter<I>,
        cx: &mut StateCx<'_, I>,
        e: &AnimEventRecord,
    ) {
        if is(e, "idleActionEnd") {
            if owner.variable_string("sittingAnim").is_empty()
                && owner.variable_string("idleAction") != "sit"
            {
                owner.clear_var("idleAction");
            }
            owner.clear_var("sittingAnim");
        }
        if is(e, "PlayBreedSound") {
            if e.event
                .parameter_value
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case("petting"))
            {
                let t = PET_SOUND_TIME.get(&mut owner.state.params);
                let now = cx.ctx.current_time_millis();
                if now - t < 5000 {
                    return;
                }
                PET_SOUND_TIME.set(&mut owner.state.params, now);
            }
            // onPlayBreedSoundEvent: the breed's sound (FMOD).
            owner.clear_var("PlayBreedSound");
        }
    }
}

// ---- AnimalWalkState ----

impl<I: 'static> State<I> for AnimalWalkState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn is_moving(&self, _owner: &GameCharacter<I>) -> bool {
        true
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        if !owner.vars.remote {
            owner.set_var_bool("bMoving", true);
        }
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if !local(owner, cx) {
            return;
        }
        if owner.vars.collided_this_frame {
            let (x, y, z) = owner.path_target(cx);
            owner.animal_path_to_location(x, y, z, cx);
            if !owner.variable_boolean("bPathfind") {
                owner.set_var_bool("bPathfind", true);
                owner.set_var_bool("bMoving", true);
            }
            return;
        }
        let (tx, ty, _) = owner.path_target_f(cx);
        let dx = tx - owner.vars.x;
        let mut dy = ty - owner.vars.y;
        if dx == 0.0 && dy == 0.0 {
            dy = 1.0e-4;
        }
        owner.set_forward_direction(dx, dy);
        if dx.abs() < 0.5 && dy.abs() < 0.5 {
            owner.set_var_bool("bMoving", false);
        }
        if cx
            .world
            .walking_on_the_spot_check(owner.id, owner.vars.x, owner.vars.y)
            && owner.af().spotted_chr.is_none()
        {
            owner.set_var_bool("bMoving", false);
            owner.vars.moving = false;
            cx.world.path_reset(owner);
            owner.path.path2 = None;
            owner.behavior_walked_on_spot(cx);
        }
        let b = &owner.af().animal.behavior;
        if b.is_doing_behavior && b.behavior_action == Some(BehaviorAction::FightAnimal) {
            let target = match b.behavior_object {
                Some(BehaviorObject::Character(r)) | Some(BehaviorObject::Animal(r)) => {
                    character_ref_id(r)
                }
                _ => None,
            };
            if let Some(o) = target.and_then(|t| owner.other(t, cx)) {
                if dist_proper((owner.vars.x, owner.vars.y), (o.x, o.y))
                    <= owner.adef().attack_dist as f32
                {
                    self.exit(owner, cx);
                }
            }
        }
        if local(owner, cx) {
            owner.behavior_wander_idle(cx);
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if !owner.vars.remote {
            owner.set_var_bool("bMoving", false);
            owner.behavior_do_behavior_action(cx);
        }
    }
}

// ---- AnimalPathFindState ----

/// `AnimalPathFindState.TICK_COUNT`.
pub const TICK_COUNT: Param<i64> = Param::new(ids::ANIMAL_PATH_FIND, "tick_count", || 0);

impl<I: 'static> State<I> for AnimalPathFindState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn is_moving(&self, owner: &GameCharacter<I>) -> bool {
        owner.vars.moving
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.set_var_bool("bPathfind", true);
        owner.set_var_bool("bMoving", false);
        // IngameState.instance.numberTicks (only for the debugger).
        TICK_COUNT.set(&mut owner.state.params, 0);
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        use crate::character::state::BehaviorResult;
        let result = cx.world.path_update(owner);
        owner.run_path_requests(cx);
        match result {
            BehaviorResult::Failed => {
                let (x, y, z) = owner.path_target(cx);
                owner.animal_path_failed(cx);
                owner.af_mut().animal.should_follow_wall = true;
                cx.world.path_to_location(owner, x, y, z);
                owner.set_var_bool("bMoving", true);
            }
            BehaviorResult::Succeeded => {
                let (tx, ty, _) = owner.path_target_f(cx);
                let (tx, ty) = (tx as i32, ty as i32);
                if !cx.world.chunk_loaded(tx / 8, ty / 8) {
                    owner.set_var_bool("bPathfind", false);
                    owner.set_var_bool("bMoving", true);
                    return;
                }
                owner.set_var_bool("bPathfind", false);
                owner.set_var_bool("bMoving", false);
                owner.path.path2 = None;
            }
            _ => {}
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.set_var_bool("bPathfind", false);
        owner.set_var_bool("bMoving", false);
        cx.world.path_cancel_request(owner);
        owner.path.progress = pz_pathfind::mover::PathFindProgress::NotRunning;
        owner.path.path2 = None;
        owner.behavior_do_behavior_action(cx);
    }
}

// ---- AnimalEatState ----

impl<I: 'static> State<I> for AnimalEatState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    // enter / exit: the eating and feeding sounds (FMOD).
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.state_event_delay_timer = 300.0;
        let a = &owner.af().animal;
        let (et, eg, dt, dr, dp) = (
            a.eat_from_trough,
            a.eat_from_ground,
            a.drink_from_trough,
            a.drink_from_river,
            a.drink_from_puddle,
        );
        if let Some(t) = et {
            owner.face_animal_object(t, cx);
        }
        if let Some(t) = eg {
            owner.face_animal_object(t, cx);
        }
        if let Some(t) = dt {
            owner.face_animal_object(t, cx);
        }
        if let Some(s) = dr {
            owner.face_floor(s, cx);
        }
        if let Some(s) = dp {
            owner.face_floor(s, cx);
        }
        owner.set_var_bool("bMoving", false);
    }
    fn anim_event(
        &self,
        owner: &mut GameCharacter<I>,
        cx: &mut StateCx<'_, I>,
        e: &AnimEventRecord,
    ) {
        if is(e, "idleActionEnd") {
            owner.clear_var("eatingAnim");
            owner.clear_var("idleAction");
            owner.behavior_after_idle_action_end(cx);
        }
    }
}

// ---- AnimalAlertedState ----

thread_local! {
    /// The singleton's `alertedFor` and `spottedDist` (instance fields of
    /// the one state object, shared by every animal).
    static ALERTED_FOR: Cell<f32> = const { Cell::new(0.0) };
    static SPOTTED_DIST: Cell<f32> = const { Cell::new(0.0) };
}

/// `alertedFor` and `spottedDist` of the singleton (for tests and traces).
pub fn alerted_state_fields() -> (f32, f32) {
    (
        ALERTED_FOR.with(|c| c.get()),
        SPOTTED_DIST.with(|c| c.get()),
    )
}

impl AnimalAlertedState {
    /// `setTurnAlertedValues(animal, chr)`.
    pub fn set_turn_alerted_values<I: 'static>(
        owner: &mut GameCharacter<I>,
        chr: (f32, f32),
        cx: &StateCx<'_, I>,
    ) {
        let (vx, vy) = (owner.vars.x - chr.0, owner.vars.y - chr.1);
        let mut rad_sound = direction_neg(vx, vy);
        rad_sound = if rad_sound < 0.0 {
            rad_sound.abs()
        } else {
            (std::f64::consts::PI * 2.0 - rad_sound as f64) as f32
        };
        let mut deg_sound = (rad_sound as f64).to_degrees();
        let (zx, zy) = dir_vector(owner.vars.dir.rot180());
        let (zx, zy) = normalize(zx, zy);
        let mut rad_zombie = direction_neg(zx, zy);
        rad_zombie = if rad_zombie < 0.0 {
            rad_zombie.abs()
        } else {
            std::f32::consts::PI * 2.0 - rad_zombie
        };
        let mut deg_zombie = (rad_zombie as f64).to_degrees();
        if deg_zombie as i32 == 360 {
            deg_zombie = 0.0;
        }
        if deg_sound as i32 == 360 {
            deg_sound = 0.0;
        }
        let deg_x = if deg_sound > deg_zombie {
            let mut sum = (deg_sound - deg_zombie) as i32;
            if sum > 180 {
                sum = 180 - (sum - 180);
                sum as f32 / 180.0 - sum as f32 / 180.0 * 2.0
            } else {
                sum as f32 / 180.0
            }
        } else {
            let sum = (deg_zombie - deg_sound) as i32;
            sum as f32 / 180.0 - sum as f32 / 180.0 * 2.0
        };
        if cx.ctx.is_client() {
            let d = direction(chr.0 - owner.vars.x, chr.1 - owner.vars.y);
            owner.set_var_float("AlertX", d);
        } else {
            owner.set_var_float("AlertX", deg_x);
        }
        owner.set_var_float("AlertY", 0.0);
    }
}

impl<I: 'static> State<I> for AnimalAlertedState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        ALERTED_FOR.with(|c| c.set(0.0));
        if let Some(sq) = owner
            .af()
            .alerted_chr
            .and_then(|a| owner.other(a, cx))
            .and_then(|o| o.square)
        {
            let d = sq_dist_proper_xy(sq, owner.vars.x as i32, owner.vars.y as i32);
            SPOTTED_DIST.with(|c| c.set(d));
        }
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let Some(o) = owner
            .af()
            .alerted_chr
            .and_then(|a| owner.other(a, cx))
            .filter(|o| o.square.is_some())
        else {
            owner.set_default_state(cx);
            return;
        };
        AnimalAlertedState::set_turn_alerted_values(owner, (o.x, o.y), cx);
        if ALERTED_FOR.with(|c| c.get()) > 1000.0 {
            owner.af_mut().animal.alerted = false;
            owner.af_mut().alerted_chr = None;
            owner.set_default_state(cx);
        }
        let m = cx.ctx.multiplier();
        ALERTED_FOR.with(|c| c.set(c.get() + m));
        if let Some(o) = owner.af().alerted_chr.and_then(|a| owner.other(a, cx)) {
            if let Some(sq) = o.square {
                let dist = sq_dist_proper_xy(sq, owner.vars.x as i32, owner.vars.y as i32);
                let spotted = SPOTTED_DIST.with(|c| c.get());
                if dist < spotted - 2.0 || dist <= 4.0 {
                    let f = owner.af_mut();
                    f.spotted_chr = f.alerted_chr;
                    f.animal.behavior.last_alerted = 10000.0;
                    f.animal.alerted = false;
                    f.alerted_chr = None;
                }
            }
        }
    }
}

// ---- AnimalAttackState ----

impl<I: 'static> State<I> for AnimalAttackState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if let Some(o) = owner.af().atk_target.and_then(|t| owner.other(t, cx)) {
            owner.face_character(o.x, o.y);
        }
        if let Some(t) = owner.af().thump_target {
            owner.face_thump_target(t, cx);
            let tsq = cx
                .world
                .thump_info(crate::character::state::ThumpTarget::Object(t), owner.id)
                .and_then(|i| i.square);
            if let (Some(tsq), Some(own)) = (tsq, owner.path.current_square) {
                if sq_dist_proper(tsq, own) >= 2.0 {
                    owner.af_mut().thump_target = None;
                }
            }
        }
    }
    fn anim_event(
        &self,
        owner: &mut GameCharacter<I>,
        cx: &mut StateCx<'_, I>,
        e: &AnimEventRecord,
    ) {
        if is(e, "AttackConnect") {
            owner.vars.performing_attack_animation = false;
            if let Some(t) = owner.af().thump_target {
                let kind = cx
                    .world
                    .thump_info(crate::character::state::ThumpTarget::Object(t), owner.id)
                    .map(|i| i.kind);
                if kind == Some(crate::character::state::ThumpKind::Thumpable) {
                    let (destroyed, open_door) = cx.world.thumpable_animal_hit(t, owner.id);
                    if destroyed {
                        owner.af_mut().thump_target = None;
                        let (x, y, z) = owner.path_target(cx);
                        owner.animal_path_to_location(x, y, z, cx);
                    }
                    if open_door {
                        owner.af_mut().thump_target = None;
                        let (x, y, z) = owner.path_target(cx);
                        owner.animal_path_to_location(x, y, z, cx);
                    }
                    return;
                }
            }
            let target = owner.af().atk_target.and_then(|t| owner.other(t, cx));
            if let Some(t) = target.as_ref().filter(|t| t.animal) {
                fight_exchange(owner, t.id, true, cx);
            } else if let Some(p) = target.as_ref().filter(|t| t.iso_player) {
                if p.vehicle {
                    owner.af_mut().atk_target = None;
                    return;
                }
                if dist_proper((p.x, p.y), (owner.vars.x, owner.vars.y))
                    > owner.adef().attack_dist as f32 + 0.5
                {
                    owner.af_mut().atk_target = None;
                    return;
                }
                if cx
                    .world
                    .character_mut(p.id)
                    .is_some_and(|c| c.is_ghost_mode(cx.ctx))
                {
                    owner.af_mut().atk_target = None;
                    return;
                }
                if cx.world.line_clear_collide(
                    owner.vars.x,
                    owner.vars.y,
                    p.x,
                    p.y,
                    fastfloor(p.z),
                    None,
                    3,
                ) {
                    owner.af_mut().atk_target = None;
                    return;
                }
                if !cx.ctx.is_client() {
                    let damage = owner.animal_damage();
                    cx.world.animal_hit_player(p.id, owner, damage);
                }
            } else if let Some(t) = owner.af().thump_target {
                let info = cx
                    .world
                    .thump_info(crate::character::state::ThumpTarget::Object(t), owner.id);
                if info
                    .as_ref()
                    .is_some_and(|i| i.kind == crate::character::state::ThumpKind::Door)
                {
                    if info.as_ref().is_some_and(|i| i.open) {
                        owner.af_mut().thump_target = None;
                        let (x, y, z) = owner.path_target(cx);
                        owner.animal_path_to_location(x, y, z, cx);
                        return;
                    }
                }
                let damage = 100.0 * owner.animal_damage();
                cx.world.object_damage(t, damage);
            }
            owner.af_mut().animal.behavior.block_movement = false;
            owner.af_mut().atk_target = None;
        } else if is(e, "ActiveAnimFinishing") {
            owner.vars.performing_attack_animation = false;
            owner.af_mut().animal.behavior.block_movement = false;
            let target = owner
                .af()
                .atk_target
                .and_then(|t| owner.other(t, cx))
                .filter(|t| t.animal);
            if let Some(t) = target {
                fight_exchange(owner, t.id, false, cx);
            }
            owner.af_mut().atk_target = None;
        }
    }
}

/// The animal-against-animal part of `AttackConnect` (`check_line`: the
/// hits need a clear line) and of `ActiveAnimFinishing`: the target is hit,
/// hits back when it is attacking this animal, and both step apart.
fn fight_exchange<I: 'static>(
    owner: &mut GameCharacter<I>,
    target: i32,
    check_line: bool,
    cx: &mut StateCx<'_, I>,
) {
    let Some(mut t) = cx.world.take_character(target) else {
        return;
    };
    let blocked = check_line
        && cx.world.line_clear_collide(
            owner.vars.x,
            owner.vars.y,
            t.vars.x,
            t.vars.y,
            fastfloor(t.vars.z),
            None,
            3,
        );
    let my_dmg = owner.animal_damage();
    if !blocked {
        t.animal_hit_by_animal(my_dmg, false, cx);
    }
    t.af_mut().animal.behavior.block_movement = false;
    let attacks_back = t.animal_is_attacking(cx) && t.af().atk_target == Some(owner.id);
    if !blocked && attacks_back {
        let d = t.animal_damage();
        owner.animal_hit_by_animal(d, false, cx);
    }
    let (txi, tyi, tzi) = (t.xi(), t.yi(), t.zi());
    let (oxi, oyi, ozi) = (owner.xi(), owner.yi(), owner.zi());
    cx.world.put_character(t);
    owner.animal_path_to_location(txi - 3, tyi, tzi, cx);
    owner.with_other(target, cx, |t, cx| {
        t.animal_path_to_location(oxi + 3, oyi, ozi, cx);
        t.af_mut().animal.behavior.block_movement = false;
        t.af_mut().atk_target = None;
    });
    if check_line {
        owner.af_mut().atk_target = None;
    }
}

// ---- AnimalClimbOverFenceState ----

pub const START_X: Param<i32> = Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "start_x", || 0);
pub const START_Y: Param<i32> = Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "start_y", || 0);
pub const Z: Param<i32> = Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "z", || 0);
pub const END_X: Param<i32> = Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "end_x", || 0);
pub const END_Y: Param<i32> = Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "end_y", || 0);
pub const DIR: Param<Option<IsoDirections>> =
    Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "dir", || None);
pub const ZOMBIE_ON_FLOOR: Param<bool> =
    Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "zombie_on_floor", || false);
pub const PREV_STATE: Param<Option<StateId>> =
    Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "prev_state", || None);
pub const COUNTER: Param<bool> = Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "counter", || false);
pub const SOLID_FLOOR: Param<bool> =
    Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "solid_floor", || false);
pub const SHEET_ROPE: Param<bool> =
    Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "sheet_rope", || false);
pub const RUN: Param<bool> = Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "run", || false);
pub const SPRINT: Param<bool> = Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "sprint", || false);
pub const COLLIDABLE: Param<bool> =
    Param::new(ids::ANIMAL_CLIMB_OVER_FENCE, "collidable", || false);

/// `AnimalClimbOverFenceState.setParams(owner, dir)`.
pub fn set_climb_params<I: 'static>(
    owner: &mut GameCharacter<I>,
    dir: IsoDirections,
    cx: &mut StateCx<'_, I>,
) {
    let Some((x, y, z)) = owner.path.current_square else {
        return;
    };
    let (mut ex, mut ey) = (x, y);
    match dir {
        IsoDirections::N => ey -= 1,
        IsoDirections::S => ey += 1,
        IsoDirections::W => ex -= 1,
        IsoDirections::E => ex += 1,
        // IllegalArgumentException("invalid direction").
        _ => return,
    }
    let opposite = cx.world.square(ex, ey, z);
    let counter = opposite.is_some() && cx.world.square_has_solid_trans((ex, ey, z));
    let floor = opposite.as_ref().is_some_and(|s| s.solid_floor);
    let sheet = opposite.is_some() && cx.world.has_sheet_rope((ex, ey, z));
    let running = owner.is_running(cx.sm);
    let sprinting = owner.is_sprinting(cx.sm, cx.ctx);
    let prev = owner.current_state_id();
    let p = &mut owner.state.params;
    START_X.set(p, x);
    START_Y.set(p, y);
    Z.set(p, z);
    END_X.set(p, ex);
    END_Y.set(p, ey);
    DIR.set(p, Some(dir));
    ZOMBIE_ON_FLOOR.set(p, false);
    PREV_STATE.set(p, prev);
    COUNTER.set(p, counter);
    SOLID_FLOOR.set(p, floor);
    SHEET_ROPE.set(p, sheet);
    RUN.set(p, running);
    SPRINT.set(p, sprinting);
    COLLIDABLE.set(p, false);
}

impl<I: 'static> State<I> for AnimalClimbOverFenceState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let refuse = |o: &mut GameCharacter<I>| {
            o.clear_var("climbDown");
            o.set_var_bool("ClimbFence", false);
        };
        if !owner.adef().can_climb_fences {
            refuse(owner);
            return;
        }
        let dir = DIR.get(&mut owner.state.params);
        let Some(cur) = owner.path.current_square else {
            return;
        };
        let opposite = dir.map(|d| {
            let (dx, dy) = d.delta();
            (cur.0 + dx, cur.1 + dy, cur.2)
        });
        let Some(op) = opposite.filter(|o| cx.world.square(o.0, o.1, o.2).is_some()) else {
            refuse(owner);
            return;
        };
        if cx.world.hoppable_between(cur, op).is_none() {
            refuse(owner);
            return;
        }
        owner.set_var_bool("ClimbingFence", true);
    }
    fn execute(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        // setAnimated(true): the legs sprite.
        let dir = DIR.get(&mut owner.state.params);
        if let Some(
            d @ (IsoDirections::N | IsoDirections::S | IsoDirections::W | IsoDirections::E),
        ) = dir
        {
            owner.vars.dir = d;
        }
        if owner.variable_boolean("ClimbFenceStarted")
            && owner
                .variable_string("ClimbFenceOutcome")
                .eq_ignore_ascii_case("fall")
        {
            owner.vars.falling = true;
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.clear_var("ClimbFence");
        owner.clear_var("climbDown");
        if let Some(d) = DIR.get(&mut owner.state.params) {
            owner.vars.dir = d.rot180();
        }
    }
    fn anim_event(
        &self,
        owner: &mut GameCharacter<I>,
        _cx: &mut StateCx<'_, I>,
        e: &AnimEventRecord,
    ) {
        if is(e, "Climbed") {
            owner.set_var_bool("climbDown", true);
            if let Some(d) = DIR.get(&mut owner.state.params) {
                DIR.set(&mut owner.state.params, Some(rot_left(rot_left(d))));
            }
        }
        if is(e, "ClimbDone") {
            owner.clear_var("climbDown");
            owner.set_var_bool("ClimbFence", false);
        }
    }
    fn is_ignore_collide(
        &self,
        owner: &GameCharacter<I>,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
    ) -> bool {
        let p = &owner.state.params;
        let g = |q: Param<i32>| q.peek(p).unwrap_or(0);
        let (sx, sy, ex, ey, z) = (g(START_X), g(START_Y), g(END_X), g(END_Y), g(Z));
        if z != from.2 || z != to.2 {
            return false;
        }
        let (x1, y1, x2, y2) = (sx.min(ex), sy.min(ey), sx.max(ex), sy.max(ey));
        let (x3, y3, x4, y4) = (
            from.0.min(to.0),
            from.1.min(to.1),
            from.0.max(to.0),
            from.1.max(to.1),
        );
        x1 <= x3 && y1 <= y3 && x2 >= x4 && y2 >= y4
    }
}

// ---- AnimalFollowWallState ----

pub const REPATHDELAY: Param<f32> = Param::new(ids::ANIMAL_FOLLOW_WALL, "repathdelay", || 0.0);
pub const TIMETOSTOP_FOLLOWING_WALL: Param<f32> =
    Param::new(ids::ANIMAL_FOLLOW_WALL, "timetostop_following_wall", || 0.0);
pub const CW: Param<Option<bool>> = Param::new(ids::ANIMAL_FOLLOW_WALL, "cw", || None);
pub const CURRENTDIR: Param<Option<IsoDirections>> =
    Param::new(ids::ANIMAL_FOLLOW_WALL, "currentdir", || None);

impl AnimalFollowWallState {
    /// `decideRotation(animal, collideDir)`.
    pub fn decide_rotation<I: 'static>(
        owner: &mut GameCharacter<I>,
        collide: IsoDirections,
        cx: &StateCx<'_, I>,
    ) -> bool {
        if let Some(r) = CW.get(&mut owner.state.params) {
            return r;
        }
        use IsoDirections::*;
        let d = owner.vars.dir;
        let cw = if d == collide {
            cx.ctx.rand_next(2) == 0
        } else {
            match collide {
                N => d == NE || d == E,
                E => d == SE || d == S,
                S => d == SW || d == W,
                W => d == NW || d == N,
                _ => false,
            }
        };
        CW.set(&mut owner.state.params, Some(cw));
        cw
    }

    fn collide<I: 'static>(
        owner: &GameCharacter<I>,
        dx: i32,
        dy: i32,
        cx: &StateCx<'_, I>,
    ) -> bool {
        let Some(cur) = owner.path.current_square else {
            return false;
        };
        cx.world
            .test_collide_adjacent(owner.mover_id_of(), cur, dx, dy, 0)
    }

    /// `go(x, y, animal)`.
    fn go<I: 'static>(x: i32, y: i32, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        cx.world.path_reset(owner);
        let z = owner.zi();
        cx.world.path_to_location(owner, x, y, z);
    }

    /// `checkNoCollide(animal)`.
    pub fn check_no_collide<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if cx.ctx.rand_next(5) != 0 {
            return;
        }
        let cur_dir = CURRENTDIR.get(&mut owner.state.params);
        let (ddx, ddy) = owner.vars.dir.delta();
        let side = |owner: &mut GameCharacter<I>,
                    cx: &mut StateCx<'_, I>,
                    a: (i32, i32),
                    b: (i32, i32)| {
            if Self::collide(owner, a.0, a.1, cx) || Self::collide(owner, b.0, b.1, cx) {
                return;
            }
            let (x, y, z) = (owner.xi(), owner.yi(), owner.zi());
            if cx.ctx.rand_next(2) == 0 && !Self::collide(owner, b.0, b.1, cx) {
                owner.af_mut().animal.should_follow_wall = false;
                owner.animal_path_to_location(x + b.0 * 7, y + b.1 * 7, z, cx);
                owner.set_var_bool("bMoving", true);
            } else if !Self::collide(owner, a.0, a.1, cx) {
                owner.af_mut().animal.should_follow_wall = false;
                owner.animal_path_to_location(x + a.0 * 7, y + a.1 * 7, z, cx);
                owner.set_var_bool("bMoving", true);
            }
        };
        if ddy == 0
            && (ddx == 1 && cur_dir == Some(IsoDirections::E)
                || ddx == -1 && cur_dir == Some(IsoDirections::W))
        {
            side(owner, cx, (0, -1), (0, 1));
            return;
        }
        if ddx == 0
            && (ddy == 1 && cur_dir == Some(IsoDirections::S)
                || ddy == -1 && cur_dir == Some(IsoDirections::N))
        {
            side(owner, cx, (-1, 0), (1, 0));
        }
    }

    /// `followWall(animal)`.
    pub fn follow_wall<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.af_mut().animal.following_wall = true;
        let cur_dir = CURRENTDIR.get(&mut owner.state.params);
        let (x, y) = (owner.vars.x as i32, owner.vars.y as i32);
        // (collision side, its direction, cw free test, cw target, cw dir,
        // ccw free test, ccw target, ccw dir, fallback target)
        type Side = (
            (i32, i32),
            IsoDirections,
            (i32, i32),
            (i32, i32),
            IsoDirections,
            (i32, i32),
            (i32, i32),
            IsoDirections,
            (i32, i32),
        );
        let sides: [Side; 4] = [
            (
                (0, -1),
                IsoDirections::N,
                (1, 0),
                (40, 4),
                IsoDirections::E,
                (-1, 0),
                (-40, 4),
                IsoDirections::W,
                (0, 40),
            ),
            (
                (1, 0),
                IsoDirections::E,
                (-1, 1),
                (-4, 40),
                IsoDirections::S,
                (0, -1),
                (-4, -40),
                IsoDirections::N,
                (-40, 0),
            ),
            (
                (0, 1),
                IsoDirections::S,
                (-1, 0),
                (-40, -4),
                IsoDirections::W,
                (1, 0),
                (40, -4),
                IsoDirections::E,
                (0, -40),
            ),
            (
                (-1, 0),
                IsoDirections::W,
                (0, -1),
                (4, -40),
                IsoDirections::N,
                (0, 1),
                (4, 40),
                IsoDirections::S,
                (40, 0),
            ),
        ];
        for (c, cdir, cwt, cwg, cwd, ccwt, ccwg, ccwd, fb) in sides {
            if !Self::collide(owner, c.0, c.1, cx) {
                continue;
            }
            let cw = Self::decide_rotation(owner, cdir, cx);
            if cw && !Self::collide(owner, cwt.0, cwt.1, cx) {
                if cur_dir != Some(cwd) {
                    Self::go(x + cwg.0, y + cwg.1, owner, cx);
                    CURRENTDIR.set(&mut owner.state.params, Some(cwd));
                }
                return;
            }
            if !cw && !Self::collide(owner, ccwt.0, ccwt.1, cx) {
                if cur_dir != Some(ccwd) {
                    Self::go(x + ccwg.0, y + ccwg.1, owner, cx);
                    CURRENTDIR.set(&mut owner.state.params, Some(ccwd));
                }
                return;
            }
            Self::go(x + fb.0, y + fb.1, owner, cx);
            return;
        }
    }

    /// `updateParams(animal)`.
    fn update_params<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let t = TIMETOSTOP_FOLLOWING_WALL.get(&mut owner.state.params) - cx.ctx.multiplier();
        if t <= 0.0 {
            cx.world.path_reset(owner);
            owner.set_var_bool("bMoving", false);
            let a = &mut owner.af_mut().animal;
            a.following_wall = false;
            a.should_follow_wall = false;
            return;
        }
        TIMETOSTOP_FOLLOWING_WALL.set(&mut owner.state.params, t);
    }
}

impl<I: 'static> State<I> for AnimalFollowWallState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn is_moving(&self, _owner: &GameCharacter<I>) -> bool {
        true
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let Some(cur) = owner.path.current_square else {
            return;
        };
        REPATHDELAY.set(&mut owner.state.params, 0.0);
        let (tx, ty, _) = owner.path_target(cx);
        let mut timer = sq_dist_proper_xy(cur, tx, ty) * 80.0;
        if timer < 500.0 {
            timer = 500.0;
        }
        TIMETOSTOP_FOLLOWING_WALL.set(&mut owner.state.params, timer);
        CW.set(&mut owner.state.params, None);
        CURRENTDIR.set(&mut owner.state.params, None);
        owner.set_var_bool("bMoving", true);
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        AnimalFollowWallState::update_params(owner, cx);
        let repath = REPATHDELAY.get(&mut owner.state.params);
        if repath > 0.0 {
            REPATHDELAY.set(&mut owner.state.params, repath - cx.ctx.multiplier());
            return;
        }
        REPATHDELAY.set(&mut owner.state.params, 10.0);
        if owner.vars.collided_this_frame {
            AnimalFollowWallState::follow_wall(owner, cx);
        }
        AnimalFollowWallState::check_no_collide(owner, cx);
        let (tx, ty, _) = owner.path_target_f(cx);
        let dx = tx - owner.vars.x;
        let mut dy = ty - owner.vars.y;
        if dx == 0.0 && dy == 0.0 {
            dy = 1.0e-4;
        }
        owner.set_forward_direction(dx, dy);
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.set_var_bool("bMoving", false);
        let a = &mut owner.af_mut().animal;
        a.following_wall = false;
        a.should_follow_wall = false;
        owner.behavior_do_behavior_action(cx);
    }
}

// ---- AnimalHitReactionState, AnimalFalldownState, AnimalOnGroundState ----

impl<I: 'static> State<I> for AnimalHitReactionState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.hit_reaction = String::new();
    }
}

/// `owner.die()` for a dead animal.
fn die<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
    if !owner.vars.on_kill_done {
        let k = owner.vars.attacked_by;
        owner.kill_by(k, cx);
    }
    cx.ctx.event(crate::character::SimEvent::Die);
}

impl<I: 'static> State<I> for AnimalFalldownState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if cx.ctx.is_server() && owner.is_dead() {
            die(owner, cx);
        }
    }
    // animEvent: PlayDeathSound (setDoDeathSound(false), playDeadSound())
    // and PlayBreedSound are FMOD's.
}

impl<I: 'static> State<I> for AnimalOnGroundState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.path.collidable = false;
        if owner.is_dead() {
            die(owner, cx);
        }
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if owner.is_dead() {
            die(owner, cx);
        }
    }
}

// ---- AnimalZoneState ----

/// `AnimalZoneState`'s inner states (`ZoneState` subclasses).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoneState {
    /// `StateFollow` (`nextRestTime`).
    Follow {
        next_rest_time: f64,
    },
    /// `StateSleep` (`wakeTime`).
    Sleep {
        wake_time: f64,
    },
    MoveFromEat,
    MoveToEat,
    /// `StateEat` (`startTime`).
    Eat {
        start_time: f64,
    },
}

pub const ACTION: Param<Option<String>> = Param::new(ids::ANIMAL_ZONE, "action", || None);
pub const STATE: Param<Option<ZoneState>> = Param::new(ids::ANIMAL_ZONE, "state", || None);

impl AnimalZoneState {
    fn zone_action<I: 'static>(
        owner: &mut GameCharacter<I>,
        zone: (i64, i64),
        cx: &mut StateCx<'_, I>,
    ) -> Option<String> {
        cx.world
            .animal_host(owner.id)
            .and_then(|h| h.world.animal_zone_action(zone))
    }

    fn update<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let Some(st) = STATE.get(&mut owner.state.params) else {
            return;
        };
        let age = cx.ctx.world_age_hours();
        let dist = cx.ctx.thirty_fps_multiplier() * 0.2;
        match st {
            ZoneState::Follow { next_rest_time } => {
                Self::move_along_path(owner, st, dist, cx);
                if ACTION.get(&mut owner.state.params).as_deref() == Some("Eat") {
                    STATE.set(&mut owner.state.params, Some(ZoneState::MoveToEat));
                    return;
                }
                // The state object may have been replaced by a junction's
                // visit; nextRestTime is the Follow object's own field.
                let mut next = next_rest_time;
                if next < 0.0 {
                    next = cx.ctx.world_age_hours() + cx.ctx.rand_next_range(2, 5) as f64 / 60.0;
                    if let Some(ZoneState::Follow { .. }) = STATE.get(&mut owner.state.params) {
                        STATE.set(
                            &mut owner.state.params,
                            Some(ZoneState::Follow {
                                next_rest_time: next,
                            }),
                        );
                    }
                }
                if cx.ctx.world_age_hours() > next {
                    let wake =
                        cx.ctx.world_age_hours() + cx.ctx.rand_next_range(1, 3) as f64 / 60.0;
                    STATE.set(
                        &mut owner.state.params,
                        Some(ZoneState::Sleep { wake_time: wake }),
                    );
                }
            }
            ZoneState::Sleep { wake_time } => {
                if age > wake_time {
                    ACTION.set(&mut owner.state.params, Some("Follow".into()));
                    STATE.set(
                        &mut owner.state.params,
                        Some(ZoneState::Follow {
                            next_rest_time: -1.0,
                        }),
                    );
                }
            }
            ZoneState::MoveFromEat | ZoneState::MoveToEat => {
                Self::move_along_path(owner, st, dist, cx)
            }
            ZoneState::Eat { start_time } => {
                if age - start_time > 0.033_333_333_333_333_33 {
                    STATE.set(&mut owner.state.params, Some(ZoneState::MoveFromEat));
                }
            }
        }
    }

    /// `visitJunction(zone, bEndPoint, junctions)` of the inner state.
    fn visit_junction<I: 'static>(
        owner: &mut GameCharacter<I>,
        st: ZoneState,
        zone: (i64, i64),
        end_point: bool,
        junctions: &[AnimalZoneJunction],
        cx: &mut StateCx<'_, I>,
    ) -> Option<AnimalZoneJunction> {
        match st {
            ZoneState::Follow { .. } => {
                let mut possible = Vec::new();
                for j in junctions {
                    let a = Self::zone_action(owner, j.zone_other, cx);
                    if a.as_deref() == Some("Eat") {
                        return Some(*j);
                    }
                    if a.as_deref() == Some("Follow") {
                        possible.push(*j);
                    }
                }
                if Self::zone_action(owner, zone, cx).as_deref() == Some("Follow")
                    && !end_point
                    && cx.ctx.rand_next(possible.len() as i32 + 1) == 0
                {
                    return None;
                }
                pick_random(&possible, cx)
            }
            ZoneState::MoveFromEat => {
                let mut possible = Vec::new();
                for j in junctions {
                    if Self::zone_action(owner, j.zone_other, cx).as_deref() == Some("Follow") {
                        possible.push(*j);
                    }
                }
                let j = pick_random(&possible, cx)?;
                STATE.set(
                    &mut owner.state.params,
                    Some(ZoneState::Follow {
                        next_rest_time: -1.0,
                    }),
                );
                Some(j)
            }
            _ => None,
        }
    }

    /// `reachedEnd()` of the inner state.
    fn reached_end<I: 'static>(owner: &mut GameCharacter<I>, st: ZoneState, cx: &StateCx<'_, I>) {
        if st == ZoneState::MoveToEat {
            STATE.set(
                &mut owner.state.params,
                Some(ZoneState::Eat {
                    start_time: cx.ctx.world_age_hours(),
                }),
            );
        }
    }

    /// `moveAlongPath(action, distance)`.
    fn move_along_path<I: 'static>(
        owner: &mut GameCharacter<I>,
        st: ZoneState,
        distance: f32,
        cx: &mut StateCx<'_, I>,
    ) {
        let Some(mut zone) = owner.af().animal.animal_zone else {
            owner.af_mut().animal.animal_zone = None;
            return;
        };
        let id = owner.id;
        let (x, y) = (owner.vars.x, owner.vars.y);
        let Some((t, len)) = cx.world.animal_host(id).map(|h| {
            (
                h.world.animal_zone_closest_point_on_polyline(zone, x, y),
                h.world.animal_zone_polyline_length(zone),
            )
        }) else {
            return;
        };
        let forward = owner.af().animal.move_forward_on_zone;
        let mut t2 = if forward {
            t + distance / len
        } else {
            t - distance / len
        };
        let junctions = cx
            .world
            .animal_host(id)
            .map(|h| h.world.animal_zone_junctions_between(zone, t, t2))
            .unwrap_or_default();
        if !junctions.is_empty() {
            let end = junctions[0].first_point_on_zone1 || junctions[0].last_point_on_zone1;
            if let Some(j) = Self::visit_junction(owner, st, zone, end, &junctions, cx) {
                zone = j.zone_other;
                let (d, l) = cx
                    .world
                    .animal_host(id)
                    .map(|h| {
                        (
                            h.world.animal_zone_distance_of_point_from_start(
                                zone,
                                j.point_index_other,
                            ),
                            h.world.animal_zone_polyline_length(zone),
                        )
                    })
                    .unwrap_or((0.0, 1.0));
                t2 = d / l;
                let fwd =
                    j.first_point_on_zone2 || !j.last_point_on_zone2 && cx.ctx.rand_next(2) == 0;
                owner.af_mut().animal.move_forward_on_zone = fwd;
                t2 = if fwd { t2 + 1.0 / l } else { t2 - 1.0 / l };
                let a = Self::zone_action(owner, zone, cx);
                ACTION.set(&mut owner.state.params, a);
            }
        }
        // The state object reachedEnd() acts on is the one this call
        // started with.
        if t2 >= 1.0 && forward {
            t2 = 1.0;
            owner.af_mut().animal.move_forward_on_zone = false;
            Self::reached_end(owner, st, cx);
        }
        if t2 <= 0.0 && forward {
            t2 = 0.0;
            owner.af_mut().animal.move_forward_on_zone = true;
            Self::reached_end(owner, st, cx);
        }
        let Some(((px, py), (mut dx, mut dy))) = cx.world.animal_host(id).map(|h| {
            (
                h.world.animal_zone_point_on_polyline(zone, t2),
                h.world.animal_zone_direction_on_polyline(zone, t2),
            )
        }) else {
            return;
        };
        owner.vars.next_x = px;
        owner.vars.next_y = py;
        if !owner.af().animal.move_forward_on_zone {
            dx *= -1.0;
            dy *= -1.0;
        }
        owner.set_forward_direction(dx, dy);
        owner.af_mut().animal.animal_zone = Some(zone);
    }
}

/// `PZArrayUtil.pickRandom(list)`.
fn pick_random<T: Copy, I>(list: &[T], cx: &StateCx<'_, I>) -> Option<T> {
    if list.is_empty() {
        return None;
    }
    Some(list[cx.ctx.rand_next(list.len() as i32) as usize])
}

impl<I: 'static> State<I> for AnimalZoneState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn sync_flags(&self) -> SyncFlags {
        NONE
    }
    fn is_moving(&self, owner: &GameCharacter<I>) -> bool {
        owner.vars.moving
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let a = owner
            .af()
            .animal
            .animal_zone
            .and_then(|z| AnimalZoneState::zone_action(owner, z, cx));
        ACTION.set(&mut owner.state.params, a);
        STATE.set(
            &mut owner.state.params,
            Some(ZoneState::Follow {
                next_rest_time: -1.0,
            }),
        );
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        AnimalZoneState::update(owner, cx);
    }
}
