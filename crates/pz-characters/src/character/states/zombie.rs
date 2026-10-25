//! The zombie states of `zombie.ai.states`: `ZombieIdleState`,
//! `WalkTowardState`, `PathFindState`, `LungeState`, `LungeNetworkState`
//! (its own part; the network walk is not modelled), `AttackState`,
//! `ThumpState`, `ZombieFallDownState`, `ZombieGetUpState`,
//! `ZombieOnGroundState`, `ZombieReanimateState`, `ZombieEatBodyState`,
//! `ZombieFaceTargetState`, `ZombieGetDownState`, `ZombieSittingState`,
//! `CrawlingZombieTurnState`, `FakeDeadZombieState`, `ZombieTurnAlerted`,
//! `ZombieGetUpFromCrawlState` and `ZombieFallingState`.
//!
//! `parameterZombieState.setState(...)` is the zombie's `sound_state`.
//! The animation-player calls (`setTargetAngle`, the thump track) are
//! TODO(anim): `ThumpState` counts the `ThumpFrame` events the animation
//! reports instead of scanning the track.

use pz_anim::{AnimEventRecord, AnimationVariableSource};

use super::ids;
use super::stagger_back::parse_boolean;
use crate::character::state::{
    EatTarget, TestResults, Param, State, StateCx, StateId, StateRef, ThumpKind, ThumpTarget,
};
use crate::character::zombie::math::*;
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

singleton!(ZombieIdleState, ids::ZOMBIE_IDLE, idle);
singleton!(WalkTowardState, ids::WALK_TOWARD, walk_toward);
singleton!(PathFindState, ids::PATH_FIND, path_find);
singleton!(LungeState, ids::LUNGE, lunge);
singleton!(LungeNetworkState, ids::LUNGE_NETWORK, lunge_network);
singleton!(AttackState, ids::ATTACK, attack);
singleton!(ThumpState, ids::THUMP, thump);
singleton!(ZombieFallDownState, ids::ZOMBIE_FALL_DOWN, fall_down);
singleton!(ZombieGetUpState, ids::ZOMBIE_GET_UP, get_up);
singleton!(ZombieOnGroundState, ids::ZOMBIE_ON_GROUND, on_ground);
singleton!(ZombieReanimateState, ids::ZOMBIE_REANIMATE, reanimate);
singleton!(ZombieEatBodyState, ids::ZOMBIE_EAT_BODY, eat_body);
singleton!(ZombieFaceTargetState, ids::ZOMBIE_FACE_TARGET, face_target);
singleton!(ZombieGetDownState, ids::ZOMBIE_GET_DOWN, get_down);
singleton!(ZombieSittingState, ids::ZOMBIE_SITTING, sitting);
singleton!(CrawlingZombieTurnState, ids::CRAWLING_ZOMBIE_TURN, crawling_turn);
singleton!(FakeDeadZombieState, ids::FAKE_DEAD_ZOMBIE, fake_dead);
singleton!(ZombieTurnAlerted, ids::ZOMBIE_TURN_ALERTED, turn_alerted);
singleton!(ZombieGetUpFromCrawlState, ids::ZOMBIE_GET_UP_FROM_CRAWL, get_up_from_crawl);
singleton!(ZombieFallingState, ids::ZOMBIE_FALLING, falling);
singleton!(ZombieHitReactionState, ids::ZOMBIE_HIT_REACTION, hit_reaction);
singleton!(FakeDeadAttackState, ids::FAKE_DEAD_ATTACK, fake_dead_attack);

/// Every zombie state ported here.
pub fn all<I: 'static>() -> Vec<StateRef<I>> {
    vec![
        idle(),
        walk_toward(),
        path_find(),
        lunge(),
        lunge_network(),
        attack(),
        thump(),
        fall_down(),
        get_up(),
        on_ground(),
        reanimate(),
        eat_body(),
        face_target(),
        get_down(),
        sitting(),
        crawling_turn(),
        fake_dead(),
        turn_alerted(),
        get_up_from_crawl(),
        falling(),
        hit_reaction(),
        fake_dead_attack(),
    ]
}

/// `ZombieIdleState.TICK_COUNT` (and the other states' `TICK_COUNT`s, one
/// per state).
pub const IDLE_TICK_COUNT: Param<i64> = Param::new(ids::ZOMBIE_IDLE, "tick_count", || 0);
pub const WALK_TICK_COUNT: Param<i64> = Param::new(ids::WALK_TOWARD, "tick_count", || 0);
pub const WALK_IGNORE_OFFSET: Param<Option<bool>> = Param::new(ids::WALK_TOWARD, "ignore_offset", || None);
pub const WALK_IGNORE_TIME: Param<i64> = Param::new(ids::WALK_TOWARD, "ignore_time", || 0);
pub const PATH_TICK_COUNT: Param<i64> = Param::new(ids::PATH_FIND, "tick_count", || 0);
pub const LUNGE_TICK_COUNT: Param<i64> = Param::new(ids::LUNGE, "tick_count", || 0);
pub const LUNGE_NETWORK_TICK_COUNT: Param<i64> = Param::new(ids::LUNGE_NETWORK, "tick_count", || 0);
/// `AttackState.SKIP_TEST_DEFENCE`.
pub const SKIP_TEST_DEFENCE: Param<bool> = Param::new(ids::ATTACK, "skip_test_defence", || false);
/// `ZombieGetUpState.PREV_STATE`.
pub const GET_UP_PREV_STATE: Param<Option<StateId>> = Param::new(ids::ZOMBIE_GET_UP, "prev_state", || None);
/// `ZombieEatBodyState.BODY`.
pub const EAT_BODY: Param<Option<i64>> = Param::new(ids::ZOMBIE_EAT_BODY, "body", || None);
/// `ZombieGetDownState` params.
pub const GET_DOWN_PREV_STATE: Param<Option<StateId>> = Param::new(ids::ZOMBIE_GET_DOWN, "prev_state", || None);
pub const GET_DOWN_WAIT_TIME: Param<f32> = Param::new(ids::ZOMBIE_GET_DOWN, "wait_time", || 0.0);
pub const GET_DOWN_START_X: Param<f32> = Param::new(ids::ZOMBIE_GET_DOWN, "start_x", || f32::NEG_INFINITY);
pub const GET_DOWN_START_Y: Param<f32> = Param::new(ids::ZOMBIE_GET_DOWN, "start_y", || f32::NEG_INFINITY);
/// `ZombieTurnAlerted.TARGET_ANGLE`.
pub const TURN_ALERTED_TARGET_ANGLE: Param<f32> = Param::new(ids::ZOMBIE_TURN_ALERTED, "target_angle", || 0.0);
/// `ZombieGetUpFromCrawlState.STATE`.
pub const GET_UP_FROM_CRAWL_STATE: Param<Option<StateId>> = Param::new(ids::ZOMBIE_GET_UP_FROM_CRAWL, "state", || None);
/// The `ThumpFrame` events the animation reported this tick (TODO(anim):
/// `ThumpAnimations.countThumpEventsBetween`).
pub const THUMP_FRAMES: Param<i32> = Param::new(ids::THUMP, "thump_frames", || 0);

fn is(e: &AnimEventRecord, name: &str) -> bool {
    e.event.event_name.eq_ignore_ascii_case(name)
}

fn tick<I: 'static>(owner: &mut GameCharacter<I>, p: Param<i64>, at_two: &'static str) {
    let t = p.get(&mut owner.state.params);
    if t == 2 {
        owner.zombie.sound_state = at_two;
    }
    p.set(&mut owner.state.params, t + 1);
}

/// `owner.die()` for a zombie.
fn die<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
    if !owner.vars.on_kill_done {
        let k = owner.vars.attacked_by;
        owner.kill_by(k, cx);
    }
    cx.ctx.event(SimEvent::Die);
}

/// The position of a character by id.
fn pos_of<I: 'static>(cx: &mut StateCx<'_, I>, id: i32) -> Option<(f32, f32, f32)> {
    cx.world.character_mut(id).map(|c| (c.vars.x, c.vars.y, c.vars.z))
}

/// `faceThisObject(character)` (`faceThisObjectAlt`).
pub fn face_character<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, id: i32) {
    if let Some((x, y, _)) = pos_of(cx, id) {
        let (dx, dy) = (x - owner.vars.x, y - owner.vars.y);
        if length(dx, dy) > 0.0 {
            owner.set_forward_direction(dx, dy);
        }
    }
}

/// The re-path the get-up and get-down states do on exit.
fn resume_path<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, prev: Option<StateId>, walk_after: bool) {
    if prev == Some(ids::PATH_FIND) {
        match owner.path_target_char(cx) {
            None => {
                owner.set_var_bool("bPathfind", true);
                owner.set_var_bool("bMoving", false);
            }
            Some(t) if owner.is_target_location_known() => owner.zombie_path_to_character(t, cx),
            Some(_) if owner.zombie.last_target_seen_x != -1 => {
                let z = &owner.zombie;
                let (x, y, zz) = (z.last_target_seen_x, z.last_target_seen_y, z.last_target_seen_z);
                owner.zombie_path_to_location(x, y, zz, cx);
            }
            Some(_) => {}
        }
    } else if walk_after && prev == Some(ids::WALK_TOWARD) {
        owner.set_var_bool("bPathFind", false);
        owner.set_var_bool("bMoving", true);
    }
}

// ---- ZombieIdleState ----

/// `pickRandomWanderInterval()`.
fn pick_random_wander_interval(ctx: &dyn crate::SimContext) -> f32 {
    let mut i = ctx.rand_next_range(400, 1000) as f32;
    if !ctx.is_raining() {
        i *= 1.5;
    }
    i
}

impl<I: 'static> State<I> for ZombieIdleState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.zombie.sound_source_target = None;
        owner.zombie.sound_attract = 0.0;
        owner.zombie.movex = 0.0;
        owner.zombie.movey = 0.0;
        owner.vars.state_event_delay_timer = pick_random_wander_interval(cx.ctx);
        IDLE_TICK_COUNT.set(&mut owner.state.params, 0);
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.zombie.movex = 0.0;
        owner.zombie.movey = 0.0;
        if cx.ctx.is_last_stand() {
            // The nearest living player (the world's players by id).
            let mut lowest = None;
            let mut lowest_dst = 1_000_000.0f32;
            for id in cx.world.characters_near(owner.vars.x, owner.vars.y, owner.vars.z, f32::MAX) {
                let Some(p) = cx.world.character_mut(id) else { continue };
                if !p.is_player() || p.is_dead() {
                    continue;
                }
                let d = distance_manhatten(p.vars.x, p.vars.y, owner.vars.x, owner.vars.y);
                if d < lowest_dst {
                    lowest_dst = d;
                    lowest = Some(id);
                }
            }
            if let Some(l) = lowest {
                owner.zombie_path_to_character(l, cx);
            }
            return;
        }
        if !owner.zombie.reanimated_for_grapple_only {
            owner.vars.on_floor = owner.vars.crawling;
        }
        tick(owner, IDLE_TICK_COUNT, "Idle");
        if owner.zombie.indoor_zombie || owner.zombie.useless {
            return;
        }
        if !owner.zombie.reanimated_for_grapple_only && owner.vars.state_event_delay_timer <= 0.0 {
            owner.vars.state_event_delay_timer = pick_random_wander_interval(cx.ctx);
            let x = fastfloor(owner.vars.x) + cx.ctx.rand_next(8) - 4;
            let y = fastfloor(owner.vars.y) + cx.ctx.rand_next(8) - 4;
            let z = fastfloor(owner.vars.z);
            if cx.world.square(x, y, z).is_some() && cx.world.square_is_free((x, y, z)) {
                owner.zombie_path_to_location(x, y, z, cx);
                owner.zombie.allow_repath_delay = 200.0;
            }
        }
    }
}

// ---- WalkTowardState ----

/// `WalkTowardState.calculateTargetLocation(zomb, location)`: the path
/// target, offset per zombie unless crawling or ignoring the offset.
pub fn walk_toward_target_location<I: 'static>(zomb: &GameCharacter<I>, cx: &StateCx<'_, I>) -> (f32, f32) {
    let (tx, ty, _) = zomb.path_target_f(cx);
    let dist = length(tx - zomb.vars.x, ty - zomb.vars.y);
    if dist < 0.025 {
        return (tx, ty);
    }
    let ignore = WALK_IGNORE_OFFSET.peek(&zomb.state.params).flatten();
    if !cx.ctx.is_server() && !zomb.vars.crawling && ignore == Some(false) {
        let scale = (dist / 2.0).min(4.0);
        let off = ((zomb.id + zomb.zombie.zombie_id) % 20) as f32 / 10.0 - 1.0;
        if distance_to(zomb.vars.x, zomb.vars.y, tx + off * scale, ty + off * scale) < dist {
            return (tx + off * scale, ty + off * scale);
        }
    }
    (tx, ty)
}

impl<I: 'static> State<I> for WalkTowardState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn is_moving(&self, _owner: &GameCharacter<I>) -> bool {
        true
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let now = cx.ctx.current_time_millis();
        if WALK_IGNORE_OFFSET.get(&mut owner.state.params).is_none() {
            WALK_IGNORE_OFFSET.set(&mut owner.state.params, Some(false));
            WALK_IGNORE_TIME.set(&mut owner.state.params, 0);
        }
        if WALK_IGNORE_OFFSET.get(&mut owner.state.params) == Some(true) && now - WALK_IGNORE_TIME.get(&mut owner.state.params) > 3000 {
            WALK_IGNORE_OFFSET.set(&mut owner.state.params, Some(false));
            WALK_IGNORE_TIME.set(&mut owner.state.params, 0);
        }
        WALK_TICK_COUNT.set(&mut owner.state.params, 0);
        if owner.zombie.useless {
            owner.change_state(Some(idle()), cx);
        }
        cx.world.walking_on_the_spot_reset(owner.id, owner.vars.x, owner.vars.y);
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if !owner.vars.crawling {
            owner.vars.on_floor = false;
        }
        let target = owner.vars.target;
        let target_vehicle = target.and_then(|t| cx.world.character_mut(t).and_then(|c| c.path.vehicle));
        if let Some(t) = target {
            if owner.is_target_location_known() {
                if let Some(mid) = cx.world.character_mut(t).map(|c| c.mover_id_of()) {
                    cx.world.path_to_character(owner, mid);
                    // TODO(vehicles): `chooseBestAttackPosition` for a target
                    // in a vehicle.
                }
            } else if owner.zombie.last_target_seen_x != -1 {
                let z = &owner.zombie;
                let (lx, ly, lz) = (z.last_target_seen_x, z.last_target_seen_y, z.last_target_seen_z);
                let is_target = cx.world.path_behavior(owner.id).is_some_and(|b| b.is_target_location(lx as f32 + 0.5, ly as f32 + 0.5, lz as f32));
                if !is_target {
                    owner.zombie_path_to_location(lx, ly, lz, cx);
                }
            }
        }
        let (ptx, pty, _) = owner.path_target(cx);
        if ptx == fastfloor(owner.vars.x) && pty == fastfloor(owner.vars.y) {
            match target.and_then(|t| pos_of(cx, t)) {
                None if target.is_none() => {
                    owner.set_var_bool("bPathfind", false);
                    owner.set_var_bool("bMoving", false);
                    return;
                }
                Some((_, _, tz)) if fastfloor(tz) != fastfloor(owner.vars.z) => {
                    owner.set_var_bool("bPathfind", true);
                    owner.set_var_bool("bMoving", false);
                    return;
                }
                _ => {}
            }
        }
        let mut collided_vehicle = owner.zombie.collided_with_vehicle;
        if target_vehicle.is_some() && owner.zombie.target_view.vehicle_adjacent {
            collided_vehicle = false;
        }
        let mut collided = owner.zombie.collided_this_frame;
        if collided && WALK_IGNORE_OFFSET.get(&mut owner.state.params) == Some(false) {
            WALK_IGNORE_OFFSET.set(&mut owner.state.params, Some(true));
            WALK_IGNORE_TIME.set(&mut owner.state.params, cx.ctx.current_time_millis());
            let (x, y, _) = owner.path_target_f(cx);
            let z = owner.vars.z;
            let (cxx, cyy) = (fastfloor(x) / 8, fastfloor(y) / 8);
            collided = if cx.world.chunk_loaded(cxx, cyy) {
                let ignore = owner.path_target_char(cx);
                cx.world.line_clear_collide(owner.vars.x, owner.vars.y, x, y, fastfloor(z), ignore, 1 | 2)
            } else {
                true
            };
        }
        if collided || collided_vehicle {
            owner.zombie.allow_repath_delay = 0.0;
            let (x, y, z) = owner.path_target(cx);
            owner.zombie_path_to_location(x, y, z, cx);
            if !owner.variable_boolean("bPathfind") {
                owner.set_var_bool("bPathfind", true);
                owner.set_var_bool("bMoving", false);
            }
            return;
        }
        let (tx, ty, _) = owner.path_target_f(cx);
        let (mut vx, mut vy) = (tx - owner.vars.x, ty - owner.vars.y);
        let mut dist = length(vx, vy);
        if dist < 0.25 {
            owner.vars.x = tx;
            owner.vars.y = ty;
            owner.zombie.next_x = tx;
            owner.zombie.next_y = ty;
            dist = 0.0;
        }
        if dist < 0.025 {
            owner.set_var_bool("bPathfind", false);
            owner.set_var_bool("bMoving", false);
            return;
        }
        if !cx.ctx.is_server() && !owner.vars.crawling && WALK_IGNORE_OFFSET.get(&mut owner.state.params) == Some(false) {
            let scale = (dist / 2.0).min(4.0);
            let off = ((owner.id + owner.zombie.zombie_id) % 20) as f32 / 10.0 - 1.0;
            if distance_to(owner.vars.x, owner.vars.y, tx + off * scale, ty + off * scale) < dist {
                vx = tx + off * scale - owner.vars.x;
                vy = ty + off * scale - owner.vars.y;
            }
        }
        owner.zombie.running = false;
        let (nx, ny) = normalize(vx, vy);
        if owner.vars.crawling {
            if owner.variable_string("TurnDirection").is_empty() {
                owner.set_forward_direction(nx, ny);
            }
        } else {
            owner.vars.dir = crate::character::state::vars::iso_direction_from_angle(nx, ny);
            owner.set_forward_direction(nx, ny);
        }
        if !owner.vars.turning && cx.world.walking_on_the_spot_check(owner.id, owner.vars.x, owner.vars.y) {
            owner.set_var_bool("bMoving", false);
        }
        tick(owner, WALK_TICK_COUNT, "Idle");
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.set_var_bool("bMoving", false);
    }
}

// ---- PathFindState ----

impl<I: 'static> State<I> for PathFindState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn is_moving(&self, owner: &GameCharacter<I>) -> bool {
        owner.vars.moving
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.set_var_bool("bPathfind", true);
        owner.set_var_bool("bMoving", false);
        PATH_TICK_COUNT.set(&mut owner.state.params, 0);
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        use crate::character::state::BehaviorResult;
        let result = cx.world.path_update(owner);
        match result {
            BehaviorResult::Failed => {
                owner.set_var_bool("bPathfind", false);
                owner.set_var_bool("bMoving", false);
            }
            BehaviorResult::Succeeded => {
                let (tx, ty, _) = owner.path_target_f(cx);
                let (tx, ty) = (fastfloor(tx), fastfloor(ty));
                if !cx.world.chunk_loaded(tx / 8, ty / 8) {
                    owner.set_var_bool("bPathfind", false);
                    owner.set_var_bool("bMoving", true);
                    return;
                }
                owner.set_var_bool("bPathfind", false);
                owner.set_var_bool("bMoving", false);
                owner.clear_path2();
            }
            _ => {
                if owner.is_zombie() {
                    tick(owner, PATH_TICK_COUNT, "Idle");
                }
            }
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if owner.is_zombie() {
            owner.zombie.allow_repath_delay = 0.0;
        }
        owner.set_var_bool("bPathfind", false);
        owner.set_var_bool("bMoving", false);
        owner.set_var_bool("ShouldBeCrawling", false);
        cx.world.path_cancel(owner);
        owner.path.progress = pz_pathfind::mover::PathFindProgress::NotRunning;
        owner.clear_path2();
    }
}

// ---- LungeState / LungeNetworkState ----

/// The lunge timer countdown both lunge states share.
fn lunge_tick<I: 'static>(owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
    owner.vars.on_floor = false;
    owner.vars.shootable = true;
    if owner.zombie.lunger {
        owner.zombie.walk_variant_use = Some("ZombieWalk3".into());
    }
    owner.zombie.lunge_timer -= cx.ctx.thirty_fps_multiplier();
    let ghost = owner.vars.target.and_then(|t| cx.world.character_mut(t)).is_some_and(|p| p.is_player() && p.is_ghost_mode(cx.ctx));
    if ghost {
        owner.zombie.lunge_timer = 0.0;
    }
    if owner.zombie.lunge_timer < 0.0 {
        owner.zombie.lunge_timer = 0.0;
    }
    if owner.zombie.lunge_timer <= 0.0 {
        owner.zombie.allow_repath_delay = 0.0;
    }
}

impl<I: 'static> State<I> for LungeState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn is_moving(&self, _owner: &GameCharacter<I>) -> bool {
        true
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let now = cx.ctx.current_time_millis();
        if now - owner.zombie.lunge_sound_time > 5000 {
            owner.zombie.lunge_sound_time = now;
        }
        owner.zombie.lunge_timer = 180.0;
        LUNGE_TICK_COUNT.set(&mut owner.state.params, 0);
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        lunge_tick(owner, cx);
        let (vx, vy) = owner.zombie.vector_to_target;
        owner.zombie.running = cx.ctx.lore_speed() == 1 && !owner.zombie.inactive || owner.zombie.speed_type == 1;
        let (nx, ny) = normalize(vx, vy);
        owner.set_forward_direction(nx, ny);
        let z = &owner.zombie;
        if !owner.is_target_location_known() && z.last_target_seen_x != -1 {
            let (lx, ly, lz) = (z.last_target_seen_x, z.last_target_seen_y, z.last_target_seen_z);
            let is_target = cx.world.path_behavior(owner.id).is_some_and(|b| b.is_target_location(lx as f32 + 0.5, ly as f32 + 0.5, lz as f32));
            if !is_target {
                owner.zombie.lunge_timer = 0.0;
                owner.zombie_path_to_location(lx, ly, lz, cx);
            }
        }
        tick(owner, LUNGE_TICK_COUNT, "LockTarget");
    }
}

impl<I: 'static> State<I> for LungeNetworkState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn is_moving(&self, _owner: &GameCharacter<I>) -> bool {
        true
    }
    /// The network walk (`WalkTowardNetworkState`) is not modelled.
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.zombie.lunge_timer = 180.0;
        LUNGE_NETWORK_TICK_COUNT.set(&mut owner.state.params, 0);
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        lunge_tick(owner, cx);
        tick(owner, LUNGE_NETWORK_TICK_COUNT, "LockTarget");
    }
}

// ---- AttackState ----

impl<I: 'static> State<I> for AttackState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn is_attacking(&self, _owner: &GameCharacter<I>) -> bool {
        true
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.state.params.clear(Self::ID);
        SKIP_TEST_DEFENCE.set(&mut owner.state.params, false);
        owner.zombie.attack_outcome = "start".into();
        owner.clear_var("AttackDidDamage");
        owner.clear_var("ZombieBiteDone");
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let target = owner.vars.target;
        if let Some(t) = target {
            if cx.world.character_mut(t).is_some_and(|c| c.variable_string("ZombieHitReaction") == "Chainsaw") {
                return;
            }
        }
        let mut outcome = owner.zombie.attack_outcome.clone();
        if outcome == "success" && owner.variable_boolean("bAttack") && owner.variable_string("targethitreaction").eq_ignore_ascii_case("EndDeath") {
            outcome = "enddeath".into();
            owner.zombie.attack_outcome = outcome.clone();
        }
        if outcome == "success"
            && !owner.variable_boolean("bAttack")
            && !owner.variable_boolean("AttackDidDamage")
            && owner.get_variable_named("ZombieBiteDone").is_none()
        {
            owner.zombie.attack_outcome = "interrupted".into();
        }
        let target_dead = target.and_then(|t| cx.world.character_mut(t)).map(|c| c.is_dead());
        if target_dead.unwrap_or(true) {
            owner.zombie.target_seen_time = 10.0;
        }
        if let Some(t) = target {
            if !SKIP_TEST_DEFENCE.get(&mut owner.state.params)
                && outcome != "started"
                && !owner.variable_string("PlayerHitReaction").is_empty()
                && cx.world.character_mut(t).is_some()
            {
                SKIP_TEST_DEFENCE.set(&mut owner.state.params, true);
                // TODO(combat): `targetChar.testDefense(zomb)`.
            }
        }
        owner.vars.shootable = true;
        if let Some(t) = target {
            if !owner.vars.crawling {
                if outcome != "fail" && outcome != "interrupted" {
                    face_character(owner, cx, t);
                }
                owner.vars.on_floor = false;
            }
        }
        let slow = owner.zombie.speed_type == 1;
        let mult = cx.ctx.multiplier();
        if let Some(t) = target {
            if slow && (outcome == "start" || outcome == "success") {
                if let Some(c) = cx.world.character_mut(t) {
                    if c.seen.slow_factor <= 0.0 {
                        c.seen.slow_timer = 30.0;
                    }
                    c.seen.slow_timer += mult;
                    if c.seen.slow_timer > 60.0 {
                        c.seen.slow_timer = 60.0;
                    }
                    c.seen.slow_factor += 0.03;
                    if c.seen.slow_factor >= 0.5 {
                        c.seen.slow_factor = 0.5;
                    }
                }
            }
            let me = owner.id;
            if let Some(c) = cx.world.character_mut(t) {
                c.seen.time_since_zombie_attack = 0;
                c.seen.last_targetted_by = Some(me);
            }
        }
        if !owner.vars.crawling {
            owner.set_var_str("AttackType", "bite");
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.clear_var("AttackOutcome");
        owner.clear_var("AttackType");
        owner.clear_var("PlayerHitReaction");
        owner.set_state_machine_locked(false);
        if let Some(t) = owner.vars.target {
            if cx.world.character_mut(t).is_some_and(|c| c.vars.on_floor) {
                owner.set_eat_body_target(Some(EatTarget::Character(t)), true, cx);
                owner.set_zombie_target(None);
            }
        }
        owner.zombie.allow_repath_delay = 0.0;
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        if is(e, "SetAttackOutcome") {
            owner.zombie.attack_outcome = if owner.variable_boolean("bAttack") { "success" } else { "fail" }.into();
        }
        if is(e, "AttackCollisionCheck") && !owner.zombie.no_teeth {
            if let Some(t) = owner.vars.target {
                let _ = attack_collision_check(owner, t, cx);
            }
        }
        if is(e, "EatBody") {
            owner.set_var_bool("EatingStarted", true);
            let t = owner.vars.target.map(EatTarget::Character);
            owner.set_eat_body_target(t, true, cx);
            owner.set_zombie_target(None);
        }
        if is(e, "SetState") {
            owner.zombie.sound_state = "Attack";
        }
    }
}

/// `AttackState.animEvent("AttackCollisionCheck")`.
fn attack_collision_check<I: 'static>(owner: &mut GameCharacter<I>, t: i32, cx: &mut StateCx<'_, I>) -> pz_scripts::ScriptResult {
    let (zx, zy) = (owner.vars.x, owner.vars.y);
    let forward = (owner.vars.forward_x, owner.vars.forward_y);
    let Some(tc) = cx.world.character_mut(t) else { return Ok(()) };
    tc.vars.hit_from_behind = tc.is_behind(zx, zy, forward);
    let side = tc.test_dot_side(zx, zy);
    let front = side == crate::character::combat::Side::Front;
    if front && !tc.vars.aim_at_floor && !tc.variable_string("AttackType").is_empty() {
        return Ok(());
    }
    if tc.variable_string("ZombieHitReaction") == "KnifeDeath" {
        let lvl = tc.perk_level(cx.chars.perks.constant("SmallBlade").as_ref()) + 1;
        let chance = 0.max(9 - lvl * 2);
        if crate::character::body_part::next_bool(cx.ctx, chance) {
            return Ok(());
        }
    }
    let reaction = owner.variable_string("PlayerHitReaction");
    trigger_player_reaction(owner, &reaction, cx)?;
    if let Some((tx, ty, _)) = pos_of(cx, t) {
        let (hx, hy) = normalize(owner.vars.x - tx, owner.vars.y - ty);
        owner.vars.hit_dir_x = hx;
        owner.vars.hit_dir_y = hy;
    }
    Ok(())
}

/// `AttackState.triggerPlayerReaction(hitReaction, owner)`: the bite or
/// scratch through `BodyDamage.AddRandomDamageFromZombie`.
pub fn trigger_player_reaction<I: 'static>(owner: &mut GameCharacter<I>, hit_reaction: &str, cx: &mut StateCx<'_, I>) -> pz_scripts::ScriptResult {
    let Some(t) = owner.vars.target else { return Ok(()) };
    let Some(tc) = cx.world.character_mut(t) else { return Ok(()) };
    if tc.is_animal() {
        owner.vars.target = None;
        return Ok(());
    }
    let (tx, ty, tz) = (tc.vars.x, tc.vars.y, tc.vars.z);
    let dist = distance_manhatten(owner.vars.x, owner.vars.y, tx, ty);
    if dist > 1.0 && !owner.vars.crawling {
        return Ok(());
    }
    if (owner.zombie.fake_dead || owner.vars.crawling) && dist > 1.3 {
        return Ok(());
    }
    let (dead, on_floor, reaction) = (tc.is_dead(), tc.vars.on_floor, tc.vars.hit_reaction.clone());
    if dead && reaction != "EndDeath" || on_floor {
        owner.set_eat_body_target(Some(EatTarget::Character(t)), true, cx);
        return Ok(());
    }
    if dead {
        return Ok(());
    }
    let (zx, zy) = (owner.vars.x, owner.vars.y);
    let forward = (owner.vars.forward_x, owner.vars.forward_y);
    let Some(tc) = cx.world.character_mut(t) else { return Ok(()) };
    tc.vars.hit_from_behind = tc.is_behind(zx, zy, forward);
    let side = tc.test_dot_side(zx, zy);
    use crate::character::combat::Side;
    let mut hit_reaction = hit_reaction.to_string();
    if side == Side::Left {
        hit_reaction.push_str("RIGHT");
    }
    if side == Side::Right {
        hit_reaction.push_str("LEFT");
    }
    let shove = tc.vars.do_shove;
    if shove && side == Side::Front && !tc.vars.aim_at_floor {
        return Ok(());
    }
    if shove && side != Side::Front && side != Side::Behind && cx.ctx.rand_next(100) > 75 {
        return Ok(());
    }
    if (owner.vars.z - tz).abs() >= 0.2 {
        return Ok(());
    }
    let me = (fastfloor(owner.vars.x), fastfloor(owner.vars.y), fastfloor(owner.vars.z));
    let them = (fastfloor(tx), fastfloor(ty), fastfloor(tz));
    let r = cx.world.los_line_clear(me, them, false);
    if r == TestResults::Blocked || r == TestResults::ClearThroughClosedDoor {
        return Ok(());
    }
    let t_sq = cx.world.character_mut(t).and_then(|c| c.path.current_square);
    if let (Some(a), Some(b)) = (t_sq, owner.current_square_xyz()) {
        if cx.world.is_something_to(a, b) {
            return Ok(());
        }
    }
    let attack = owner.zombie_attack_for(t, &hit_reaction, cx);
    let me_id = owner.id;
    let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
    let Some(tc) = cx.world.character_mut(t) else { return Ok(()) };
    tc.vars.attacked_by = Some(me_id);
    let outcome = tc.body_damage_add_random_damage_from_zombie(sm, chars, ctx, &attack)?;
    tc.body_damage_update(sm, chars, ctx)?;
    let target_dead = tc.is_dead();
    let asleep = tc.is_asleep();
    if target_dead {
        tc.health = 0.0;
    } else if asleep {
        tc.asleep = false;
    }
    owner.zombie.voice_choice = attack.bite_voice.voice_choice.get();
    owner.apply_zombie_attack_outcome(&outcome);
    owner.set_var_bool("AttackDidDamage", outcome.hit);
    if target_dead {
        owner.set_eat_body_target(Some(EatTarget::Character(t)), true, cx);
        owner.set_zombie_target(None);
    }
    Ok(())
}

// ---- ThumpState ----

impl<I: 'static> State<I> for ThumpState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if !cx.ctx.is_client() || !owner.zombie.remote {
            let t = match cx.ctx.rand_next(3) {
                0 => "DoorClaw",
                1 => "Door",
                _ => "DoorBang",
            };
            owner.set_var_str("ThumpType", t);
        }
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let Some(thump) = owner.zombie.thump_target else {
            self.exit(owner, cx);
            return;
        };
        let info = cx.world.thump_info(thump, owner.id);
        if let ThumpTarget::Object(_) = thump {
            match &info {
                Some(i) if i.square.is_some() => face_object(owner, i),
                _ => {
                    self.exit(owner, cx);
                    return;
                }
            }
        }
        if let Some(i) = &info {
            slide_away_from_edge(owner, thump, i);
        }
        let count = THUMP_FRAMES.get(&mut owner.state.params);
        THUMP_FRAMES.set(&mut owner.state.params, 0);
        for _ in 0..count {
            owner.state.action_context.clear_event("ThumpFrame");
            owner.zombie.time_thumping += 1;
            if owner.vars.time_since_seen_flesh < 5.0 {
                owner.zombie.time_thumping = 0;
            }
            let Some(target) = owner.zombie.thump_target else { break };
            cx.world.thump(target, owner);
            let thumpable_for = owner.zombie.thump_target.and_then(|t| cx.world.thump_info(t, owner.id));
            if !cx.ctx.all_players_asleep() {
                if let Some(tf) = thumpable_for.as_ref().filter(|i| i.thumpable_for.is_some()) {
                    if tf.thumpable_for_kind == ThumpKind::Window {
                        let flag = if cx.ctx.rand_next(3) == 0 { 2 } else { 3 };
                        owner.zombie.thump_flag = flag;
                        owner.set_thump_condition(tf.thump_condition);
                    } else {
                        let mut sound = tf.thump_sound.clone();
                        let claw = owner.variable_string("ThumpType").eq_ignore_ascii_case("DoorClaw");
                        if tf.thumpable_for_kind == ThumpKind::Barricade && tf.barricade_metal {
                            sound = "ZombieThumpMetal".into();
                        } else if tf.thumpable_for_kind == ThumpKind::Barricade && tf.barricade_planks > 0 {
                            if claw {
                                sound = "ZombieThumpWood".into();
                            }
                        } else if (tf.thumpable_for_kind == ThumpKind::Door || tf.thumpable_for_kind == ThumpKind::Thumpable && tf.is_door)
                            && tf.sound_prefix.eq_ignore_ascii_case("WoodDoor")
                            && claw
                        {
                            sound = "ZombieThumpWood".into();
                        }
                        owner.zombie.thump_flag = thump_flag_for_sound(&sound);
                        owner.set_thump_condition(tf.thump_condition);
                    }
                }
            }
            if !thump_target_valid(owner, cx) {
                break;
            }
        }
        if thump_target_valid(owner, cx) {
            return;
        }
        owner.zombie.thump_target = None;
        owner.zombie.time_thumping = 0;
        let Some(i) = info else { return };
        if i.kind == ThumpKind::Window && i.can_climb_through {
            if let ThumpTarget::Object(o) = thump {
                owner.path.requests.push(crate::character::path_mover::PathMoverRequest::ClimbThroughWindow(o));
            }
            return;
        }
        let door = i.kind == ThumpKind::Door || i.kind == ThumpKind::Thumpable && i.is_door;
        if door && (i.open || i.destroyed) && lunge_through_door(owner, i.square, i.opposite_square, cx) {
            return;
        }
        if owner.zombie.last_target_seen_x != -1 {
            let z = &owner.zombie;
            let (x, y, zz) = (z.last_target_seen_x, z.last_target_seen_y, z.last_target_seen_z);
            owner.zombie_path_to_location(x, y, zz, cx);
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.zombie.thump_target = None;
        owner.zombie.thump_timer = 200;
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        if is(e, "ThumpFrame") {
            let n = THUMP_FRAMES.get(&mut owner.state.params);
            THUMP_FRAMES.set(&mut owner.state.params, n + 1);
        }
    }
}

/// The `THUMP_FLAG_*` for a thump sound.
pub fn thump_flag_for_sound(sound: &str) -> i32 {
    match sound {
        "ZombieThumpGeneric" => 1,
        "ZombieThumpWindow" => 3,
        "ZombieThumpWindowExtra" => 2,
        "ZombieThumpMetal" => 4,
        "ZombieThumpGarageDoor" => 5,
        "ZombieThumpChainlinkFence" => 6,
        "ZombieThumpMetalPoleFence" | "ZombieThumpMetalPoleGate" => 7,
        "ZombieThumpWood" => 8,
        _ => 1,
    }
}

/// `faceThisObject(object)` for a thump target: its facing position.
fn face_object<I: 'static>(owner: &mut GameCharacter<I>, i: &crate::character::state::ThumpInfo) {
    let (dx, dy) = (i.facing_position.0 - owner.vars.x, i.facing_position.1 - owner.vars.y);
    let (nx, ny) = normalize(dx, dy);
    owner.vars.dir = crate::character::state::vars::iso_direction_from_angle(nx, ny);
    if length(nx, ny) > 0.0 {
        owner.set_forward_direction(nx, ny);
    }
}

/// `isThumpTargetValid(owner, thumpable)`.
fn thump_target_valid<I: 'static>(owner: &GameCharacter<I>, cx: &mut StateCx<'_, I>) -> bool {
    let Some(t) = owner.zombie.thump_target else { return false };
    let Some(i) = cx.world.thump_info(t, owner.id) else { return false };
    if i.destroyed {
        return false;
    }
    if let ThumpTarget::Vehicle(_) = t {
        return i.in_world;
    }
    if !i.in_world {
        return false;
    }
    let Some(sq) = i.square else { return false };
    if !cx.world.chunk_loaded(sq.0 / 8, sq.1 / 8) {
        return false;
    }
    i.thumpable_for.is_some()
}

/// `slideAwayFromEdge(owner, target)`.
fn slide_away_from_edge<I: 'static>(owner: &mut GameCharacter<I>, t: ThumpTarget, i: &crate::character::state::ThumpInfo) {
    if let ThumpTarget::Vehicle(_) = t {
        return;
    }
    let Some(sq) = i.square else { return };
    let solid = i.kind == ThumpKind::Thumpable && i.block_all_the_square;
    let north = i.north;
    let dist = if i.thumpable_for_kind == ThumpKind::Barricade && i.barricade_for_character { 0.47 } else { 0.4 };
    let (x, y) = (owner.vars.x, owner.vars.y);
    let n = |o: &mut GameCharacter<I>, sy: i32| {
        if y > sy as f32 - dist {
            o.zombie.next_y = sy as f32 - dist;
        }
    };
    let s = |o: &mut GameCharacter<I>, sy: i32| {
        if y < sy as f32 + dist {
            o.zombie.next_y = sy as f32 + dist;
        }
    };
    let w = |o: &mut GameCharacter<I>, sx: i32| {
        if x > sx as f32 - dist {
            o.zombie.next_x = sx as f32 - dist;
        }
    };
    let e = |o: &mut GameCharacter<I>, sx: i32| {
        if x < sx as f32 + dist {
            o.zombie.next_x = sx as f32 + dist;
        }
    };
    if solid {
        if y < sq.1 as f32 {
            n(owner, sq.1);
        } else if y > (sq.1 + 1) as f32 {
            s(owner, sq.1 + 1);
        }
        if x < sq.0 as f32 {
            w(owner, sq.0);
        } else if x > (sq.0 + 1) as f32 {
            e(owner, sq.0 + 1);
        }
    } else if north {
        if y < sq.1 as f32 {
            n(owner, sq.1);
        } else {
            s(owner, sq.1);
        }
    } else if x < sq.0 as f32 {
        w(owner, sq.0);
    } else {
        e(owner, sq.0);
    }
}

/// `findPlayer(x1, x2, y1, y2, z)`.
fn find_player<I: 'static>(cx: &StateCx<'_, I>, x1: i32, x2: i32, y1: i32, y2: i32, z: i32) -> Option<i32> {
    for y in y1..=y2 {
        for x in x1..=x2 {
            if cx.world.square(x, y, z).is_none() {
                continue;
            }
            if let Some(p) = cx.world.players_on_square((x, y, z)).first() {
                return Some(*p);
            }
        }
    }
    None
}

/// `lungeThroughDoor(z, sq, sq2)`.
fn lunge_through_door<I: 'static>(
    owner: &mut GameCharacter<I>,
    sq: Option<(i32, i32, i32)>,
    sq2: Option<(i32, i32, i32)>,
    cx: &mut StateCx<'_, I>,
) -> bool {
    let (Some(sq), Some(sq2)) = (sq, sq2) else { return false };
    let north = sq.1 > sq2.1;
    let cur = owner.current_square_xyz();
    let (sq3, player) = if cur == Some(sq) {
        let p = if north {
            find_player(cx, sq2.0 - 1, sq2.0 + 1, sq2.1 - 1, sq2.1, sq2.2)
        } else {
            find_player(cx, sq2.0 - 1, sq2.0, sq2.1 - 1, sq2.1 + 1, sq2.2)
        };
        (sq2, p)
    } else if cur == Some(sq2) {
        let p = if north {
            find_player(cx, sq.0 - 1, sq.0 + 1, sq.1, sq.1 + 1, sq.2)
        } else {
            find_player(cx, sq.0, sq.0 + 1, sq.1 - 1, sq.1 + 1, sq.2)
        };
        (sq, p)
    } else {
        return false;
    };
    let Some(p) = player else { return false };
    let Some((px, py, pz)) = pos_of(cx, p) else { return false };
    if cx.world.los_line_clear_collide(sq3, (fastfloor(px), fastfloor(py), fastfloor(pz)), false) {
        return false;
    }
    owner.set_zombie_target(Some(p));
    owner.zombie.vector_to_target = (px - owner.vars.x, py - owner.vars.y);
    owner.vars.time_since_seen_flesh = 0.0;
    owner.zombie.thump_target = None;
    true
}

// ---- falling, getting up, on the ground ----

impl<I: 'static> State<I> for ZombieFallDownState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.block_turning = true;
        owner.vars.stagger_back = false;
        if !owner.is_dead() {
            owner.vars.hit_reaction = String::new();
        }
        owner.set_eat_body_target(None, false, cx);
        owner.vars.sit_against_wall = false;
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.block_turning = false;
        owner.vars.on_floor = true;
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        if is(e, "FallOnFront") {
            owner.environment.fall_on_front = parse_boolean(e.event.parameter_value.as_deref());
        }
        if is(e, "PlayDeathSound") {
            cx.ctx.event(SimEvent::PlaySound("PlayDeathSound".into()));
        }
    }
}

impl<I: 'static> State<I> for ZombieGetUpState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let mut prev = owner.state.machine.previous().map(|p| p.id());
        if prev == Some(ids::ZOMBIE_GET_UP_FROM_CRAWL) {
            prev = GET_UP_FROM_CRAWL_STATE.get(&mut owner.state.params);
        }
        GET_UP_PREV_STATE.set(&mut owner.state.params, prev);
        owner.zombie.sound_state = "GettingUp";
        owner.vars.on_floor = true;
        if cx.ctx.is_client() {
            owner.vars.knocked_down = false;
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.path.collidable = true;
        owner.clear_var("SprinterTripped");
        owner.clear_var("ShouldStandUp");
        if owner.vars.hit_reaction.is_empty() {
            owner.vars.sit_against_wall = false;
        }
        owner.vars.knocked_down = false;
        owner.zombie.allow_repath_delay = 0.0;
        let prev = GET_UP_PREV_STATE.get(&mut owner.state.params);
        resume_path(owner, cx, prev, true);
    }
}

/// `ZombieOnGroundState.startReanimateTimer(zombie)`.
pub fn start_reanimate_timer<I: 'static>(owner: &mut GameCharacter<I>, ctx: &dyn crate::SimContext) {
    owner.vars.reanimate_timer = (ctx.rand_next(60) + 30) as f32;
}

impl<I: 'static> State<I> for ZombieOnGroundState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.path.collidable = false;
        if !owner.is_dead() {
            owner.vars.on_floor = true;
        }
        let on_stairs = owner.current_square_xyz().and_then(|s| cx.world.square(s.0, s.1, s.2)).is_some_and(|s| s.has_stairs);
        if owner.is_dead() || owner.is_fake_dead(cx.ctx, on_stairs) {
            die(owner, cx);
            return;
        }
        owner.vars.stagger_back = false;
        owner.vars.knocked_down = false;
        owner.vars.hit_reaction = String::new();
        owner.set_eat_body_target(None, false, cx);
        owner.vars.sit_against_wall = false;
        if owner.vars.become_crawler {
            return;
        }
        if cx.ctx.game_mode() != "Tutorial" {
            start_reanimate_timer(owner, cx.ctx);
        }
        owner.zombie.sound_state = "Idle";
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let on_stairs = owner.current_square_xyz().and_then(|s| cx.world.square(s.0, s.1, s.2)).is_some_and(|s| s.has_stairs);
        if owner.is_dead() || owner.is_fake_dead(cx.ctx, on_stairs) {
            die(owner, cx);
            return;
        }
        if owner.zombie.reanimated_for_grapple_only {
            die(owner, cx);
            return;
        }
        if owner.vars.become_crawler {
            if owner.zombie.being_stepped_on || owner.zombie.target_view.under_vehicle {
                return;
            }
            owner.vars.crawling = true;
            owner.zombie.can_walk = false;
            owner.zombie.reanimate = true;
            owner.vars.become_crawler = false;
            return;
        }
        owner.vars.reanimate_timer -= cx.ctx.thirty_fps_multiplier();
        if owner.vars.reanimate_timer <= 2.0 && owner.zombie.being_stepped_on && !owner.zombie.reanimated_player {
            start_reanimate_timer(owner, cx.ctx);
        }
    }
}

impl<I: 'static> State<I> for ZombieReanimateState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.clear_var("ReanimateAnim");
        owner.zombie.sound_state = "Idle";
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.clear_var("ReanimateAnim");
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        if is(e, "FallOnFront") {
            owner.environment.fall_on_front = parse_boolean(e.event.parameter_value.as_deref());
        }
        if is(e, "ReanimateAnimFinishing") {
            owner.zombie.reanimate = false;
            owner.environment.fall_on_front = true;
        }
    }
}

// ---- eating ----

impl<I: 'static> State<I> for ZombieEatBodyState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.state_event_delay_timer = cx.ctx.rand_next_float(1800.0, 3600.0);
        let on_knees = cx.ctx.rand_next(3) != 0;
        owner.set_var_bool("onknees", on_knees);
        if let Some(EatTarget::Body(b)) = owner.zombie.eat_body_target {
            // `isEatingOther(body)`: already listed by setEatBodyTarget.
            if EAT_BODY.get(&mut owner.state.params) != Some(b) {
                EAT_BODY.set(&mut owner.state.params, Some(b));
            }
        }
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let target = owner.zombie.eat_body_target;
        if owner.vars.state_event_delay_timer <= 0.0 {
            owner.set_eat_body_target(None, false, cx);
        } else if !cx.ctx.is_server() && cx.ctx.rand_next(crate::character::body_part::adjust_for_framerate(cx.ctx, 15)) == 0 {
            owner.zombie.sound_state = "Eating";
        }
        owner.vars.time_since_seen_flesh = 0.0;
        match target {
            Some(EatTarget::Character(c)) => face_character(owner, cx, c),
            Some(EatTarget::Body(b)) => {
                if let Some(body) = cx.world.dead_body(b) {
                    let (dx, dy) = (body.x - owner.vars.x, body.y - owner.vars.y);
                    if length(dx, dy) > 0.0 {
                        owner.set_forward_direction(dx, dy);
                    }
                }
            }
            None => {}
        }
        let ctx = cx.ctx;
        if ctx.rand_next(crate::character::body_part::adjust_for_framerate(ctx, 450)) == 0 {
            // `getChunk().addBloodSplat(x + Rand(-0.5, 0.5), y + Rand(-0.5, 0.5),
            // z, Rand.Next(8))`, then the giblets (`IsoZombieGiblets`, the
            // world's).
            ctx.rand_next_float(-0.5, 0.5);
            ctx.rand_next_float(-0.5, 0.5);
            let kind = ctx.rand_next(8);
            ctx.event(SimEvent::AddBloodSplat { kind });
            ctx.rand_next(6);
            ctx.rand_next_float(-0.2, 0.2);
            ctx.rand_next_float(-0.2, 0.2);
            if ctx.rand_next(4) == 0 {
                ctx.event(SimEvent::AddBlood { part: -1, scratched: true, bitten: false, all_layers: false });
            }
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if let Some(b) = EAT_BODY.get(&mut owner.state.params) {
            cx.world.set_eating(EatTarget::Body(b), owner.id, false);
        }
        if owner.zombie.sound_state == "Eating" {
            owner.zombie.sound_state = "Idle";
        }
    }
}

impl<I: 'static> State<I> for ZombieFaceTargetState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if let Some(t) = owner.vars.target {
            face_character(owner, cx, t);
        }
    }
}

// ---- crawling ----

/// `ZombieGetDownState.isNearStartXY(owner)`.
pub fn get_down_is_near_start_xy<I: 'static>(owner: &GameCharacter<I>) -> bool {
    let sx = GET_DOWN_START_X.peek(&owner.state.params).unwrap_or(f32::NEG_INFINITY);
    let sy = GET_DOWN_START_Y.peek(&owner.state.params).unwrap_or(f32::NEG_INFINITY);
    if sx == f32::NEG_INFINITY || sy == f32::NEG_INFINITY {
        return false;
    }
    owner.dist_to_squared_xy(sx, sy) <= 0.25
}

/// `ZombieGetDownState.setParams(owner)`.
pub fn get_down_set_params<I: 'static>(owner: &mut GameCharacter<I>, ctx: &dyn crate::SimContext) {
    let w = ctx.rand_next_float(60.0, 150.0);
    GET_DOWN_WAIT_TIME.set(&mut owner.state.params, w);
}

impl<I: 'static> State<I> for ZombieGetDownState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        let prev = owner.state.machine.previous().map(|p| p.id());
        GET_DOWN_PREV_STATE.set(&mut owner.state.params, prev);
        GET_DOWN_START_X.set(&mut owner.state.params, owner.vars.x);
        GET_DOWN_START_Y.set(&mut owner.state.params, owner.vars.y);
        owner.vars.state_event_delay_timer = GET_DOWN_WAIT_TIME.get(&mut owner.state.params);
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.state_event_delay_timer = 0.0;
        owner.zombie.allow_repath_delay = 0.0;
        let prev = GET_DOWN_PREV_STATE.get(&mut owner.state.params);
        resume_path(owner, cx, prev, true);
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        if is(e, "StartCrawling") && !owner.vars.crawling {
            owner.toggle_crawling(cx.ctx);
        }
    }
}

impl<I: 'static> State<I> for ZombieSittingState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.sit_against_wall = true;
        owner.vars.on_floor = true;
        owner.vars.knocked_down = false;
    }
}

impl<I: 'static> State<I> for CrawlingZombieTurnState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.zombie.allow_repath_delay = 0.0;
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        let left = e.event.parameter_value.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("left"));
        let dir = owner.vars.dir;
        if is(e, "TurnSome") {
            let (sx, sy) = dir_vector(dir);
            let (ex, ey) = dir_vector(if left { rot_left(dir) } else { rot_right(dir) });
            let t = e.event.time_pc;
            let (vx, vy) = (lerp(sx, ex, t), lerp(sy, ey, t));
            owner.set_forward_direction(vx, vy);
            return;
        }
        if is(e, "TurnComplete") {
            owner.set_forward_iso_direction(if left { rot_left(dir) } else { rot_right(dir) });
        }
    }
}

impl<I: 'static> State<I> for FakeDeadZombieState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.path.collidable = false;
        let on_stairs = owner.current_square_xyz().and_then(|s| cx.world.square(s.0, s.1, s.2)).is_some_and(|s| s.has_stairs);
        owner.set_fake_dead_on(cx.ctx, true, on_stairs);
        owner.vars.on_floor = true;
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if owner.is_dead() {
            if !cx.ctx.is_client() {
                cx.ctx.event(SimEvent::Die);
            }
        } else if cx.ctx.is_last_stand() {
            owner.set_fake_dead(cx.ctx, false);
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.set_fake_dead(cx.ctx, false);
    }
}

impl<I: 'static> State<I> for ZombieTurnAlerted {
    fn id(&self) -> StateId {
        Self::ID
    }
    /// `getAnimationPlayer().setTargetAngle(TARGET_ANGLE)` (TODO(anim)).
    fn enter(&self, _owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {}
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let (x, y, z) = owner.path_target(cx);
        owner.zombie_path_to_sound(x, y, z, cx);
        owner.zombie.alerted = false;
    }
}

impl<I: 'static> State<I> for ZombieGetUpFromCrawlState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let prev = owner.state.machine.previous().map(|p| p.id());
        GET_UP_FROM_CRAWL_STATE.set(&mut owner.state.params, prev);
        if owner.vars.crawling {
            owner.toggle_crawling(cx.ctx);
            owner.vars.on_floor = true;
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.zombie.allow_repath_delay = 0.0;
        let prev = GET_UP_FROM_CRAWL_STATE.get(&mut owner.state.params);
        resume_path(owner, cx, prev, false);
    }
}

impl<I: 'static> State<I> for ZombieFallingState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.clear_var("bLandAnimFinished");
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.clear_var("bLandAnimFinished");
        owner.vars.fall_time = 0.0;
        owner.vars.last_fall_speed = 0.0;
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        if is(e, "FallOnFront") {
            owner.environment.fall_on_front = parse_boolean(e.event.parameter_value.as_deref());
        }
    }
}

/// `ZombieHitReactionState.TURN_TO_PLAYER`, `HIT_REACTION_TIMER`.
pub const HIT_TURN_TO_PLAYER: Param<bool> = Param::new(ids::ZOMBIE_HIT_REACTION, "turn_to_player", || false);
pub const HIT_REACTION_TIMER: Param<f32> = Param::new(ids::ZOMBIE_HIT_REACTION, "hit_reaction_timer", || 0.0);

impl<I: 'static> State<I> for ZombieHitReactionState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.zombie.collide_while_hit = true;
        HIT_TURN_TO_PLAYER.set(&mut owner.state.params, false);
        HIT_REACTION_TIMER.set(&mut owner.state.params, 0.0);
        owner.clear_var("onknees");
        if owner.vars.sit_against_wall {
            owner.vars.hit_reaction = String::new();
        }
    }
    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let t = HIT_REACTION_TIMER.get(&mut owner.state.params) + cx.ctx.multiplier();
        HIT_REACTION_TIMER.set(&mut owner.state.params, t);
        if HIT_TURN_TO_PLAYER.get(&mut owner.state.params) {
            let d = crate::character::state::vars::iso_direction_from_angle(owner.vars.hit_dir_x, owner.vars.hit_dir_y);
            owner.vars.dir = if owner.vars.hit_from_behind { d } else { d.rot180() };
        }
    }
    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.zombie.collide_while_hit = true;
        if owner.vars.target.is_some() {
            owner.zombie.allow_repath_delay = 0.0;
        }
        owner.vars.stagger_back = false;
        if !owner.is_dead() {
            owner.vars.hit_reaction = String::new();
        }
        owner.set_eat_body_target(None, false, cx);
        owner.vars.sit_against_wall = false;
        owner.vars.shootable = true;
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        let param = parse_boolean(e.event.parameter_value.as_deref());
        if is(e, "DoDeath") && param && !owner.is_dead() {
            let by = owner.vars.attacked_by;
            owner.kill_by(by, cx);
            if let Some(a) = by.and_then(|a| cx.world.character_mut(a)) {
                a.zombie_kills += 1;
            }
        }
        if is(e, "PlayDeathSound") {
            cx.ctx.event(SimEvent::PlaySound("PlayDeathSound".into()));
        }
        if is(e, "FallOnFront") {
            owner.environment.fall_on_front = param;
        }
        if is(e, "Collide") && owner.zombie.speed_type == 1 {
            owner.zombie.collide_while_hit = false;
        }
        if is(e, "ZombieTurnToPlayer") {
            HIT_TURN_TO_PLAYER.set(&mut owner.state.params, param);
        }
        if is(e, "CancelKnockDown") && param {
            owner.vars.knocked_down = false;
        }
        if is(e, "KnockDown") {
            owner.vars.on_floor = true;
            owner.vars.knocked_down = true;
        }
        if is(e, "SplatBlood") {
            let ctx = cx.ctx;
            for _ in 0..3 {
                ctx.event(SimEvent::AddBlood { part: -1, scratched: true, bitten: false, all_layers: false });
            }
            for _ in 0..10 {
                ctx.rand_next_float(-0.5, 0.5);
                ctx.rand_next_float(-0.5, 0.5);
                let kind = ctx.rand_next(8);
                ctx.event(SimEvent::AddBloodSplat { kind });
                ctx.rand_next(5);
                ctx.rand_next_float(-0.2, 0.2);
                ctx.rand_next_float(-0.2, 0.2);
            }
        }
        if is(e, "SetState") && !owner.is_dead() {
            owner.zombie.sound_state = "Hit";
        }
    }
}

impl<I: 'static> State<I> for FakeDeadAttackState {
    fn id(&self) -> StateId {
        Self::ID
    }
    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let (vx, vy) = owner.zombie.vector_to_target;
        owner.vars.dir = crate::character::state::vars::iso_direction_from_angle(vx, vy);
        let (fx, fy) = owner.vars.dir.delta();
        owner.set_forward_direction(fx as f32, fy as f32);
        owner.set_fake_dead(cx.ctx, false);
        owner.path.collidable = true;
        cx.ctx.event(SimEvent::EmitterPlaySound(format!("{}Attack", owner.voice_prefix())));
        if let Some(t) = owner.vars.target {
            let (sm, chars) = (cx.sm, cx.chars);
            if let Some(p) = cx.world.character_mut(t) {
                if p.is_player() && !p.has_trait(sm, "DESENSITIZED").unwrap_or(false) {
                    if let Ok(panic) = chars.sim.character_stats.field("PANIC") {
                        let v = p.body_damage.panic_increase_value * 3.0;
                        p.stats.add(&panic, v);
                    }
                }
            }
        }
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        if is(e, "AttackCollisionCheck") && !owner.is_dead() {
            let tpos = owner.vars.target.and_then(|t| pos_of(cx, t)).map(|(x, y, _)| (x, y));
            if owner.is_target_in_cone(tpos, 1.5, 0.9) {
                if let Some(t) = owner.vars.target {
                    // TODO(vehicles): `couldCrawlerAttackPassenger`.
                    let in_vehicle = cx.world.character_mut(t).is_some_and(|c| c.path.vehicle.is_some());
                    if !in_vehicle {
                        let attack = owner.zombie_attack_for(t, "", cx);
                        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
                        if let Some(tc) = cx.world.character_mut(t) {
                            if let Ok(out) = tc.body_damage_add_random_damage_from_zombie(sm, chars, ctx, &attack) {
                                owner.zombie.voice_choice = attack.bite_voice.voice_choice.get();
                                owner.apply_zombie_attack_outcome(&out);
                            }
                        }
                    }
                }
            }
        }
        if is(e, "FallOnFront") {
            owner.environment.fall_on_front = parse_boolean(e.event.parameter_value.as_deref());
        }
        if is(e, "ActiveAnimFinishing") {
            owner.vars.crawling = true;
        }
    }
}

/// `IsoZombie.initializeStates()`: (action-state name, state class), the
/// common part then the crawler's or the walker's.
pub const AI_COMMON: &[(&str, &str)] = &[
    ("idle", "zombie.ai.states.ZombieIdleState"),
    ("attack-network", "zombie.ai.states.AttackNetworkState"),
    ("attackvehicle-network", "zombie.ai.states.GenericDefaultState"),
    ("fakedead-attack-network", "zombie.ai.states.GenericDefaultState"),
    ("lunge-network", "zombie.ai.states.LungeNetworkState"),
    ("walktoward-network", "zombie.ai.states.WalkTowardNetworkState"),
    ("corpseThrownOutWindow-onback", "zombie.ai.states.GrappledThrownOutWindowState"),
    ("corpseThrownOutWindow-onfront", "zombie.ai.states.GrappledThrownOutWindowState"),
    ("corpseThrownOverFence-onback", "zombie.ai.states.GrappledThrownOverFenceState"),
    ("corpseThrownoverFence-onfront", "zombie.ai.states.GrappledThrownOverFenceState"),
    ("corpseThrownIntoContainer-onback", "zombie.ai.states.GrappledThrownIntoContainerState"),
    ("corpseThrownIntoContainer-onfront", "zombie.ai.states.GrappledThrownIntoContainerState"),
];

const HIT: &str = "zombie.ai.states.ZombieHitReactionState";
const FALL: &str = "zombie.ai.states.ZombieFallDownState";
const STAGGER: &str = "zombie.ai.states.StaggerBackState";

/// The crawler's states.
pub const AI_CRAWLER: &[(&str, &str)] = &[
    ("attack", "zombie.ai.states.AttackState"),
    ("fakedead", "zombie.ai.states.FakeDeadZombieState"),
    ("fakedead-attack", "zombie.ai.states.FakeDeadAttackState"),
    ("getup", "zombie.ai.states.ZombieGetUpFromCrawlState"),
    ("hitreaction", HIT),
    ("hitreaction-fencewindow", HIT),
    ("hitreaction-knockeddown", HIT),
    ("hitreaction-ragdoll", HIT),
    ("hitreaction-shothead-bwd", FALL),
    ("hitreaction-shothead-bwd-ragdoll", FALL),
    ("hitreaction-shothead-fwd", FALL),
    ("hitreaction-shothead-fwd-ragdoll", FALL),
    ("hitreaction-shothead-fwd02", FALL),
    ("hitreaction-shothead-fwd02-ragdoll", FALL),
    ("hitreaction-onfloor", HIT),
    ("hitreaction-gettingup", HIT),
    ("hitreaction-whilesitting", HIT),
    ("hitreaction-hit", HIT),
    ("onground", "zombie.ai.states.ZombieOnGroundState"),
    ("falldown-ragdoll", "zombie.ai.states.ZombieRagdollOnGroundState"),
    ("pathfind", "zombie.ai.states.PathFindState"),
    ("reanimate", "zombie.ai.states.ZombieReanimateState"),
    ("staggerback", STAGGER),
    ("staggerback-knockeddown", STAGGER),
    ("staggerback-ragdoll", STAGGER),
    ("staggerback-knockeddown-ragdoll", STAGGER),
    ("vehicleCollision-staggerback", STAGGER),
    ("thump", "zombie.ai.states.ThumpState"),
    ("turn", "zombie.ai.states.CrawlingZombieTurnState"),
    ("walktoward", "zombie.ai.states.WalkTowardState"),
];

/// The walker's states.
pub const AI_WALKER: &[(&str, &str)] = &[
    ("attack", "zombie.ai.states.AttackState"),
    ("attackvehicle", "zombie.vehicles.AttackVehicleState"),
    ("bumped", "zombie.ai.states.BumpedState"),
    ("climbfence", "zombie.ai.states.ClimbOverFenceState"),
    ("climbwindow", "zombie.ai.states.ClimbThroughWindowState"),
    ("eatbody", "zombie.ai.states.ZombieEatBodyState"),
    ("falldown", FALL),
    ("vehicleCollision-falldown", FALL),
    ("vehicleCollision-ragdoll", FALL),
    ("falldown-eatingbody", FALL),
    ("falldown-knifedeath", FALL),
    ("falldown-knifedeath-ragdoll", FALL),
    ("falldown-onknees", FALL),
    ("falldown-whilesitting", FALL),
    ("falldown-headleft", FALL),
    ("falldown-headright", FALL),
    ("falldown-headtop", FALL),
    ("falldown-speardeath", FALL),
    ("falldown-speardeath-ragdoll", FALL),
    ("falldown-speardeath1", FALL),
    ("falldown-speardeath2", FALL),
    ("falldown-uppercut", FALL),
    ("falldown-ragdoll", FALL),
    ("falling", "zombie.ai.states.ZombieFallingState"),
    ("face-target", "zombie.ai.states.ZombieFaceTargetState"),
    ("fakedead", "zombie.ai.states.FakeDeadZombieState"),
    ("fakedead-attack", "zombie.ai.states.FakeDeadAttackState"),
    ("getdown", "zombie.ai.states.ZombieGetDownState"),
    ("getup", "zombie.ai.states.ZombieGetUpState"),
    ("getup-fromOnBack", "zombie.ai.states.ZombieGetUpState"),
    ("getup-fromOnFront", "zombie.ai.states.ZombieGetUpState"),
    ("getup-fromSitting", "zombie.ai.states.ZombieGetUpState"),
    ("hitreaction", HIT),
    ("hitreaction-fencewindow", HIT),
    ("hitreaction-knockeddown", HIT),
    ("hitreaction-ragdoll", HIT),
    ("hitreaction-shothead-bwd", FALL),
    ("hitreaction-shothead-bwd-ragdoll", FALL),
    ("hitreaction-shothead-fwd", FALL),
    ("hitreaction-shothead-fwd-ragdoll", FALL),
    ("hitreaction-shothead-fwd02", FALL),
    ("hitreaction-shothead-fwd02-ragdoll", FALL),
    ("hitreaction-onfloor", HIT),
    ("hitreaction-gettingup", HIT),
    ("hitreaction-whilesitting", HIT),
    ("hitreaction-hit", HIT),
    ("idle", "zombie.ai.states.ZombieIdleState"),
    ("lunge", "zombie.ai.states.LungeState"),
    ("onground", "zombie.ai.states.ZombieOnGroundState"),
    ("falldown-ragdoll", FALL),
    ("onground-ragdoll", "zombie.ai.states.ZombieRagdollOnGroundState"),
    ("pathfind", "zombie.ai.states.PathFindState"),
    ("sitting", "zombie.ai.states.ZombieSittingState"),
    ("staggerback", STAGGER),
    ("staggerback-knockeddown", STAGGER),
    ("staggerback-ragdoll", STAGGER),
    ("staggerback-knockeddown-ragdoll", STAGGER),
    ("knockeddown-headLeft", FALL),
    ("knockeddown-headRight", FALL),
    ("knockeddown-headTop", FALL),
    ("knockeddown-shotChestL", FALL),
    ("knockeddown-shotChestR", FALL),
    ("knockeddown-shotLegL", FALL),
    ("knockeddown-shotLegR", FALL),
    ("knockeddown-shotShoulderL", FALL),
    ("knockeddown-shotShoulderR", FALL),
    ("knockeddown-uppercut", FALL),
    ("vehicleCollision", "zombie.ai.states.VehicleCollisionState"),
    ("vehicleCollision-staggerback", STAGGER),
    ("vehicleCollision-bumped", "zombie.ai.states.VehicleCollisionMinorStaggerState"),
    ("vehicleCollision-onground", "zombie.ai.states.VehicleCollisionOnGroundState"),
    ("vehicleCollision-onground-dead", "zombie.ai.states.VehicleCollisionOnGroundState"),
    ("thump", "zombie.ai.states.ThumpState"),
    ("turnalerted", "zombie.ai.states.ZombieTurnAlerted"),
    ("walktoward", "zombie.ai.states.WalkTowardState"),
    ("grappled", "zombie.ai.states.GenericDefaultState"),
];

impl<I: 'static> GameCharacter<I> {
    /// `IsoZombie.initializeStates()`: clears the map and registers the
    /// zombie's states (crawler or walker) from `registry`.
    pub fn initialize_zombie_states(&mut self, registry: &mut crate::character::state::StateRegistry<I>) {
        self.clear_ai_state_map();
        let rest = if self.vars.crawling { AI_CRAWLER } else { AI_WALKER };
        for (name, class) in AI_COMMON.iter().chain(rest.iter()) {
            let s = registry.get_or_opaque(class);
            self.register_ai_state(name, s);
        }
        self.set_default_state_to(Some(super::generic_default::instance()));
    }

    /// `GetAnimSetName()`.
    pub fn zombie_anim_set_name(&self) -> &'static str {
        if self.vars.crawling {
            "zombie-crawler"
        } else {
            "zombie"
        }
    }
}
