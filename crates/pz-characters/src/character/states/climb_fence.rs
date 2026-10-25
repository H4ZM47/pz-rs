//! `ClimbOverFenceState`: hopping a low fence (players and zombies), with
//! the fence damage when two zombies climb it at once (`OnFloor`) and the
//! fence and trip sounds (`PlayFenceSound`, `PlayTripSound`).
//!
//! Not here: `CheckAttack` (`attackFromWindowsLunge` needs the animation
//! player's bones), a player's music intensity event and the multiplayer
//! paths.

use pz_world::square::IsoDirections;

use crate::character::state::{FenceClimb, MoveDeltaModifiers, Param, State, StateCx, StateId, StateRef};
use crate::character::SimEvent;
use pz_pathfind::mover::ObjectRef;
use crate::character::zombie::math::fastfloor;
use super::climb_window::{set_lunge_x_vars, should_do_fence_lunge, zombie_sound_state};
use crate::character::GameCharacter;
use pz_anim::{AnimEventRecord, AnimationVariableSource};

use super::ids;

pub const START_X: Param<i32> = Param::new(ids::CLIMB_OVER_FENCE, "start_x", || 0);
pub const START_Y: Param<i32> = Param::new(ids::CLIMB_OVER_FENCE, "start_y", || 0);
pub const Z: Param<i32> = Param::new(ids::CLIMB_OVER_FENCE, "z", || 0);
pub const END_X: Param<i32> = Param::new(ids::CLIMB_OVER_FENCE, "end_x", || 0);
pub const END_Y: Param<i32> = Param::new(ids::CLIMB_OVER_FENCE, "end_y", || 0);
pub const DIR: Param<Option<IsoDirections>> = Param::new(ids::CLIMB_OVER_FENCE, "dir", || None);
pub const ZOMBIE_ON_FLOOR: Param<bool> = Param::new(ids::CLIMB_OVER_FENCE, "zombie_on_floor", || false);
pub const PREV_STATE: Param<Option<StateId>> = Param::new(ids::CLIMB_OVER_FENCE, "prev_state", || None);
pub const SCRATCH: Param<bool> = Param::new(ids::CLIMB_OVER_FENCE, "scratch", || false);
pub const COUNTER: Param<bool> = Param::new(ids::CLIMB_OVER_FENCE, "counter", || false);
pub const SOLID_FLOOR: Param<bool> = Param::new(ids::CLIMB_OVER_FENCE, "solid_floor", || false);
pub const SHEET_ROPE: Param<bool> = Param::new(ids::CLIMB_OVER_FENCE, "sheet_rope", || false);
pub const RUN: Param<bool> = Param::new(ids::CLIMB_OVER_FENCE, "run", || false);
pub const SPRINT: Param<bool> = Param::new(ids::CLIMB_OVER_FENCE, "sprint", || false);
pub const COLLIDABLE: Param<bool> = Param::new(ids::CLIMB_OVER_FENCE, "collidable", || false);
pub const OUTCOME: Param<String> = Param::new(ids::CLIMB_OVER_FENCE, "outcome", || "success".to_string());

/// `ClimbOverFenceState`.
pub struct ClimbOverFenceState;

/// The singleton (`instance()`).
pub fn climb_over_fence<I: 'static>() -> StateRef<I> {
    static S: ClimbOverFenceState = ClimbOverFenceState;
    &S
}

/// `ClimbOverFenceState.setParams(owner, dir)` (the square beyond the
/// fence as the world read it).
pub fn set_params<I: 'static>(owner: &mut GameCharacter<I>, dir: IsoDirections, f: FenceClimb, cx: &StateCx<'_, I>) {
    let Some((x, y, z)) = owner.path.current_square else { return };
    let (mut ex, mut ey) = (x, y);
    match dir {
        IsoDirections::N => ey -= 1,
        IsoDirections::S => ey += 1,
        IsoDirections::W => ex -= 1,
        IsoDirections::E => ex += 1,
        // The original throws.
        _ => return,
    }
    let run = owner.is_running(cx.sm);
    let sprint = owner.is_sprinting(cx.sm, cx.ctx);
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
    SCRATCH.set(p, false);
    COUNTER.set(p, f.counter);
    SOLID_FLOOR.set(p, f.solid_floor);
    SHEET_ROPE.set(p, f.sheet_rope);
    RUN.set(p, run);
    SPRINT.set(p, sprint);
    COLLIDABLE.set(p, false);
}

/// `getFence(owner)`: the hoppable between the climb's start and end
/// squares.
fn get_fence<I: 'static>(owner: &GameCharacter<I>, cx: &StateCx<'_, I>) -> Option<ObjectRef> {
    let p = &owner.state.params;
    let g = |k: Param<i32>| k.peek(p).unwrap_or(0);
    let (start, end) = ((g(START_X), g(START_Y), g(Z)), (g(END_X), g(END_Y), g(Z)));
    cx.world.square(start.0, start.1, start.2)?;
    cx.world.square(end.0, end.1, end.2)?;
    cx.world.hoppable_between(start, end)
}

/// `countZombiesClimbingOver(fence)`: the zombies with a target climbing
/// over `fence`, on its square and the square beyond (north or west).
fn count_zombies_climbing_over<I: 'static>(owner: &GameCharacter<I>, fence: Option<ObjectRef>, cx: &mut StateCx<'_, I>) -> i32 {
    let Some(fence) = fence else { return 0 };
    let Some(info) = cx.world.fence_info(fence) else { return 0 };
    let sq = info.square;
    let beyond = if info.hoppable_n { (sq.0, sq.1 - 1, sq.2) } else { (sq.0 - 1, sq.1, sq.2) };
    let mut count = 0;
    for s in [sq, beyond] {
        if cx.world.square(s.0, s.1, s.2).is_none() {
            continue;
        }
        for id in cx.world.zombies_on_square(s) {
            let climbing = |z: &GameCharacter<I>, cx: &StateCx<'_, I>| {
                z.vars.target.is_some() && z.is_current_state_id(ids::CLIMB_OVER_FENCE) && get_fence(z, cx) == Some(fence)
            };
            let yes = if id == owner.id {
                climbing(owner, cx)
            } else {
                let Some(z) = cx.world.take_character(id) else { continue };
                let yes = climbing(&z, cx);
                cx.world.put_character(z);
                yes
            };
            if yes {
                count += 1;
            }
        }
    }
    count
}

/// `shouldFallAfterVaultOver(owner)`: a running or sprinting vault's trip.
fn should_fall_after_vault_over<I: 'static>(owner: &GameCharacter<I>, cx: &StateCx<'_, I>) -> bool {
    let mut chance = 0.0f32;
    if owner.get_variable_boolean("VaultOverSprint") {
        chance = 10.0;
    }
    let level = |m: &str| owner.moodles.moodle_level_of(cx.sm, m).unwrap_or(0);
    chance += (level("ENDURANCE") * 10) as f32;
    chance += (level("DRUNK") * 10) as f32;
    chance += (level("HEAVY_LOAD") * 13) as f32;
    chance += (level("PAIN") * 5) as f32;
    let pain = cx
        .chars
        .sim
        .body_part_types
        .index("Torso_Lower")
        .ok()
        .and_then(|i| owner.part_slot(cx.chars, i as i32).ok())
        .and_then(|i| owner.body_damage.body_part(i))
        .map_or(0.0, |p| p.additional_pain_with(true));
    if pain > 20.0 {
        chance += (pain - 20.0) / 10.0;
    }
    let has = |t: &str| owner.has_trait(cx.sm, t).unwrap_or(false);
    if has("CLUMSY") {
        chance += 10.0;
    }
    if has("GRACEFUL") {
        chance -= 10.0;
    }
    // The original checks VERY_UNDERWEIGHT twice.
    if has("VERY_UNDERWEIGHT") {
        chance += 20.0;
    }
    if has("VERY_UNDERWEIGHT") {
        chance += 10.0;
    }
    if has("OBESE") {
        chance += 20.0;
    }
    if has("OVERWEIGHT") {
        chance += 10.0;
    }
    chance -= owner.perk_level(cx.chars.perks.constant("Fitness").as_ref()) as f32;
    (cx.ctx.rand_next(100) as f32) < chance
}

fn slide(cur: f32, to: f32, step: f32) -> f32 {
    if to > cur {
        step.min(to - cur)
    } else {
        (-step).max(to - cur)
    }
}

fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    // PZMath.clamp(float, float, float)
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

impl<I: 'static> State<I> for ClimbOverFenceState {
    fn id(&self) -> StateId {
        ids::CLIMB_OVER_FENCE
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.set_var_float("FenceLungeX", 0.0);
        owner.set_var_float("FenceLungeY", 0.0);
        owner.vars.ignore_movement = true;
        let endurance = cx.chars.sim.character_stats.field("ENDURANCE").ok();
        let reduce = cx.ctx.zomboid_globals().running_endurance_reduce;
        if RUN.get(&mut owner.state.params) {
            owner.set_var_bool("VaultOverRun", true);
            if let Some(e) = &endurance {
                owner.stats.remove(e, (reduce * 300.0) as f32);
            }
        } else if SPRINT.get(&mut owner.state.params) {
            owner.set_var_bool("VaultOverSprint", true);
            if let Some(e) = &endurance {
                owner.stats.remove(e, (reduce * 700.0) as f32);
            }
        }
        let counter = COUNTER.get(&mut owner.state.params);
        owner.set_var_bool("ClimbingFence", true);
        owner.set_var_bool("ClimbFenceStarted", false);
        owner.set_var_bool("ClimbFenceFinished", false);
        owner.set_var_str("ClimbFenceOutcome", if counter { "obstacle" } else { "success" });
        owner.clear_var("ClimbFenceFlopped");
        if (owner.get_variable_boolean("VaultOverRun") || owner.get_variable_boolean("VaultOverSprint")) && should_fall_after_vault_over(owner, cx) {
            owner.set_var_str("ClimbFenceOutcome", "fall");
        }
        if !counter && owner.is_zombie() && should_do_fence_lunge(owner, cx) {
            owner.set_var_str("ClimbFenceOutcome", "lunge");
            set_lunge_x_vars(owner, cx);
        }
        if !SOLID_FLOOR.get(&mut owner.state.params) {
            owner.set_var_str("ClimbFenceOutcome", "falling");
        }
        if !owner.is_zombie() && SHEET_ROPE.get(&mut owner.state.params) {
            owner.set_var_str("ClimbFenceOutcome", "rope");
        }
        if !owner.vars.remote {
            let outcome = owner.get_variable_java("ClimbFenceOutcome");
            OUTCOME.set(&mut owner.state.params, outcome);
        } else {
            let outcome = OUTCOME.get(&mut owner.state.params);
            owner.set_var_str("ClimbFenceOutcome", &outcome);
        }
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let p = &mut owner.state.params;
        let dir = DIR.get(p);
        let (end_x, end_y) = (END_X.get(p), END_Y.get(p));
        if let Some(d) = dir.filter(|d| matches!(d, IsoDirections::N | IsoDirections::S | IsoDirections::W | IsoDirections::E)) {
            owner.set_forward_iso_direction(d);
        }
        let outcome = owner.get_variable_java("ClimbFenceOutcome");
        let ns = matches!(dir, Some(IsoDirections::N | IsoDirections::S));
        let we = matches!(dir, Some(IsoDirections::W | IsoDirections::E));
        if outcome != "lunge" {
            if ns {
                let x = clamp(owner.vars.x, end_x as f32 + 0.05, (end_x + 1) as f32 - 0.05);
                owner.vars.next_x = x;
                owner.vars.x = x;
            } else if we {
                let y = clamp(owner.vars.y, end_y as f32 + 0.05, (end_y + 1) as f32 - 0.05);
                owner.vars.next_y = y;
                owner.vars.y = y;
            }
        }
        let started = owner.get_variable_boolean("ClimbFenceStarted");
        if started && !["back", "fallback", "obstacle", "obstacleEnd"].contains(&outcome.as_str()) && !outcome.eq_ignore_ascii_case("lunge") {
            let p = &mut owner.state.params;
            let (mut x, mut y) = (START_X.get(p) as f32, START_Y.get(p) as f32);
            match dir {
                Some(IsoDirections::N) => y -= 0.1,
                Some(IsoDirections::S) => y += 1.1,
                Some(IsoDirections::W) => x -= 0.1,
                Some(IsoDirections::E) => x += 1.1,
                _ => {}
            }
            let step = 0.05 * cx.ctx.thirty_fps_multiplier();
            if fastfloor(owner.vars.x) != fastfloor(x) && we {
                let dx = slide(owner.vars.x, x, step);
                owner.vars.x += dx;
                owner.vars.next_x = owner.vars.x;
            }
            if fastfloor(owner.vars.y) != fastfloor(y) && ns {
                let dy = slide(owner.vars.y, y, step);
                owner.vars.y += dy;
                owner.vars.next_y = owner.vars.y;
            }
        }
        if owner.is_zombie() {
            let down = ZOMBIE_ON_FLOOR.get(&mut owner.state.params);
            owner.vars.on_floor = down;
            owner.vars.knocked_down = down;
            owner.environment.fall_on_front = down;
        }
        if started && outcome == "fall" {
            owner.vars.falling = true;
        }
    }

    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if owner.is_player() && owner.get_variable_java("ClimbFenceOutcome") == "fall" {
            owner.environment.sprinting = false;
        }
        for v in ["ClimbingFence", "ClimbFenceFinished", "ClimbFenceOutcome", "ClimbFenceStarted", "ClimbFenceFlopped", "PlayerVoiceSound", "VaultOverSprint", "VaultOverRun"] {
            owner.clear_var(v);
        }
        owner.vars.ignore_movement = false;
        if owner.is_zombie() {
            owner.zombie.allow_repath_delay = 0.0;
            let prev = PREV_STATE.get(&mut owner.state.params);
            if prev == Some(ids::PATH_FIND) {
                let target_char = cx.world.path_behavior(owner.id).and_then(|b| b.target_char());
                match target_char {
                    None => {
                        owner.set_var_bool("bPathfind", true);
                        owner.set_var_bool("bMoving", false);
                    }
                    Some(t) => {
                        if owner.is_target_location_known() {
                            owner.zombie_path_to_character(t.0 as u32 as i32, cx);
                        } else if owner.zombie.last_target_seen_x != -1 {
                            let (lx, ly, lz) = (owner.zombie.last_target_seen_x, owner.zombie.last_target_seen_y, owner.zombie.last_target_seen_z);
                            owner.zombie_path_to_location(lx, ly, lz, cx);
                        }
                    }
                }
            } else if prev == Some(ids::WALK_TOWARD) || prev == Some(ids::WALK_TOWARD_NETWORK) {
                owner.set_var_bool("bPathFind", false);
                owner.set_var_bool("bMoving", true);
            }
        }
    }

    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        let name = e.event.event_name.as_str();
        let param = e.event.parameter_value.as_deref().unwrap_or("");
        if name.eq_ignore_ascii_case("VaultOverStarted") {
            if owner.get_variable_java("ClimbFenceOutcome") == "fall" {
                owner.report_event("EventFallClimb");
                owner.set_var_bool("BumpDone", true);
                owner.environment.fall_on_front = true;
            }
        } else if name.eq_ignore_ascii_case("SetCollidable") {
            COLLIDABLE.set(&mut owner.state.params, param.eq_ignore_ascii_case("true"));
        } else if name.eq_ignore_ascii_case("OnFloor") {
            let down = param.eq_ignore_ascii_case("true");
            ZOMBIE_ON_FLOOR.set(&mut owner.state.params, down);
            if down {
                set_lunge_x_vars(owner, cx);
                let fence = get_fence(owner, cx);
                if count_zombies_climbing_over(owner, fence, cx) >= 2 {
                    if let Some((f, info)) = fence.and_then(|f| cx.world.fence_info(f).map(|i| (f, i))) {
                        let damage = cx.ctx.rand_next_range(7, 12) / if info.metal { 2 } else { 1 };
                        if let Some(dir) = DIR.get(&mut owner.state.params) {
                            cx.world.damage_fence(f, damage, dir);
                        }
                    }
                }
                owner.set_var_bool("ClimbFenceFlopped", true);
            }
        } else if name.eq_ignore_ascii_case("PlayFenceSound") {
            if !cx.ctx.is_listener_in_range(owner.vars.x, owner.vars.y, 70.0) {
                return;
            }
            let Some(info) = get_fence(owner, cx).and_then(|f| cx.world.fence_info(f)) else { return };
            let (parameter, value) = if owner.is_zombie() { ("TripObstacleType", info.trip_type) } else { ("FenceTypeLow", info.fence_type_low) };
            cx.ctx.event(SimEvent::ObstacleSound { name: param.to_string(), parameter, value: value as f32, movement_speed: owner.is_player() });
        } else if name.eq_ignore_ascii_case("PlayTripSound") {
            if !cx.ctx.is_listener_in_range(owner.vars.x, owner.vars.y, 70.0) {
                return;
            }
            let Some(info) = get_fence(owner, cx).and_then(|f| cx.world.fence_info(f)) else { return };
            // The original casts the owner to IsoPlayer here.
            cx.ctx.event(SimEvent::ObstacleSound { name: param.to_string(), parameter: "TripObstacleType", value: info.trip_type as f32, movement_speed: true });
        } else if name.eq_ignore_ascii_case("VaultSprintFallLanded") {
            owner.drop_hand_items_in(cx);
            owner.fallen_on_knees_soft(cx);
        } else if name.eq_ignore_ascii_case("FallenOnKnees") {
            owner.fallen_on_knees_soft(cx);
        } else if name.eq_ignore_ascii_case("PlayerVoiceSound") {
            if owner.get_variable_boolean("PlayerVoiceSound") || !owner.is_player() {
                return;
            }
            owner.set_var_bool("PlayerVoiceSound", true);
            cx.ctx.event(SimEvent::PlayerVoiceSoundNamed(param.to_string()));
        } else if name.eq_ignore_ascii_case("SetState") && owner.is_zombie() {
            if let Some(s) = zombie_sound_state(param) {
                owner.zombie.sound_state = s;
            }
        }
    }

    fn get_delta_modifiers(&self, owner: &GameCharacter<I>, m: &mut MoveDeltaModifiers) {
        if owner.path.path2.is_some() && owner.is_player() {
            m.set_max_turn_delta(2.0);
        }
    }

    fn is_ignore_collide(&self, owner: &GameCharacter<I>, from: (i32, i32, i32), to: (i32, i32, i32)) -> bool {
        let p = &owner.state.params;
        let g = |k: Param<i32>| k.peek(p).unwrap_or(0);
        let (sx, sy, ex, ey, z) = (g(START_X), g(START_Y), g(END_X), g(END_Y), g(Z));
        if z != from.2 || z != to.2 {
            return false;
        }
        let (x1, y1, x2, y2) = (sx.min(ex), sy.min(ey), sx.max(ex), sy.max(ey));
        let (x3, y3, x4, y4) = (from.0.min(to.0), from.1.min(to.1), from.0.max(to.0), from.1.max(to.1));
        x1 <= x3 && y1 <= y3 && x2 >= x4 && y2 >= y4
    }

    fn can_ragdoll(&self, owner: &GameCharacter<I>) -> bool {
        if owner.variable_boolean_or("ClimbingFence", false) {
            return false;
        }
        !owner.is_zombie() || !owner.vars.on_floor
    }

    fn is_processed_on_enter(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::state::{FenceInfo, SquareInfo, StateWorld};
    use crate::character::tests::{fixture, Recorder};
    use crate::character::CharacterKind;

    const FENCE: ObjectRef = ObjectRef { x: 3, y: 3, z: 0, id: 1 };

    /// A west fence on (3, 3); zombies 7 and 8 on its square.
    #[derive(Default)]
    struct Fenced {
        others: Vec<GameCharacter<()>>,
        damage: Vec<(i32, IsoDirections)>,
    }

    impl StateWorld<()> for Fenced {
        fn take_character(&mut self, id: i32) -> Option<GameCharacter<()>> {
            let i = self.others.iter().position(|c| c.id == id)?;
            Some(self.others.remove(i))
        }
        fn put_character(&mut self, c: GameCharacter<()>) {
            self.others.push(c);
        }
        fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
            Some(SquareInfo { x, y, z, solid_floor: true, ..Default::default() })
        }
        fn hoppable_between(&self, a: (i32, i32, i32), b: (i32, i32, i32)) -> Option<ObjectRef> {
            let xs = [a.0, b.0];
            (xs.contains(&3) && xs.contains(&2) && a.1 == 3 && b.1 == 3).then_some(FENCE)
        }
        fn fence_info(&self, f: ObjectRef) -> Option<FenceInfo> {
            (f == FENCE).then(|| FenceInfo { square: (3, 3, 0), hoppable_n: false, metal: true, ..Default::default() })
        }
        fn zombies_on_square(&self, sq: (i32, i32, i32)) -> Vec<i32> {
            if sq == (3, 3, 0) {
                vec![7, 8]
            } else {
                Vec::new()
            }
        }
        fn damage_fence(&mut self, f: ObjectRef, d: i32, dir: IsoDirections) {
            assert_eq!(f, FENCE);
            self.damage.push((d, dir));
        }
    }

    fn climber(sm: &pz_scripts::ScriptManager, id: i32, cx: &mut StateCx<'_, ()>) -> GameCharacter<()> {
        let mut z = GameCharacter::new(sm, CharacterKind::Zombie);
        z.id = id;
        z.vars.x = 3.5;
        z.vars.y = 3.5;
        z.vars.target = Some(1);
        z.path.current_square = Some((3, 3, 0));
        set_params(&mut z, IsoDirections::W, FenceClimb { counter: false, solid_floor: true, sheet_rope: false }, cx);
        z.change_state(Some(climb_over_fence()), cx);
        z
    }

    fn on_floor(down: bool) -> AnimEventRecord {
        AnimEventRecord {
            state_name: None,
            layer_depth: -1,
            track: None,
            event: std::rc::Rc::new(pz_anim::AnimEvent { event_name: "OnFloor".into(), parameter_value: Some(down.to_string()), ..Default::default() }),
        }
    }

    #[test]
    fn two_zombies_on_a_fence_damage_it() {
        let (sm, chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Fenced::default();
        let (mut z, other) = {
            let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
            (climber(&sm, 7, &mut cx), climber(&sm, 8, &mut cx))
        };
        // Alone: no damage.
        {
            let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
            climb_over_fence::<()>().anim_event(&mut z, &mut cx, &on_floor(true));
        }
        assert!(w.damage.is_empty());
        assert!(z.get_variable_boolean("ClimbFenceFlopped"));
        w.others.push(other);
        {
            let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
            climb_over_fence::<()>().anim_event(&mut z, &mut cx, &on_floor(true));
        }
        // A metal fence takes half of Rand.Next(7, 12).
        assert_eq!(w.damage.len(), 1);
        assert!((3..=5).contains(&w.damage[0].0));
        assert_eq!(w.damage[0].1, IsoDirections::W);
        assert_eq!(w.others.len(), 1);
    }
}
