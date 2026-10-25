//! `ClimbOverFenceState`: hopping a low fence (players and zombies).
//!
//! Not here: the fence and trip sounds (`PlayFenceSound`,
//! `PlayTripSound`), a player's voice and endurance, the fence damage when
//! two zombies climb it at once (`OnFloor`), `CheckAttack`
//! (`attackFromWindowsLunge`) and the multiplayer paths.

use pz_world::square::IsoDirections;

use crate::character::state::{FenceClimb, Param, State, StateCx, StateId, StateRef};
use crate::character::zombie::math::{distance_manhatten, fastfloor, normalize};
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

/// `IsoZombie.shouldDoFenceLunge()`.
fn should_do_fence_lunge<I: 'static>(z: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) -> bool {
    if !cx.ctx.lore_zombies_fence_lunge() {
        return false;
    }
    if cx.ctx.rand_next(3) == 0 {
        return false;
    }
    let Some(t) = z.vars.target else { return false };
    let (zx, zy, zz) = (z.vars.x, z.vars.y, z.vars.z);
    let Some((tx, ty, tz, veh)) = cx.world.character_mut(t).map(|c| (c.vars.x, c.vars.y, c.vars.z, c.path.vehicle)) else { return false };
    if fastfloor(tz) != fastfloor(zz) || veh.is_some() {
        return false;
    }
    distance_manhatten(zx, zy, tx, ty) < 3.9
}

/// `setLungeXVars(zombie)`.
fn set_lunge_x_vars<I: 'static>(z: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
    let Some(t) = z.vars.target else { return };
    let Some((tx, ty)) = cx.world.character_mut(t).map(|c| (c.vars.x, c.vars.y)) else { return };
    z.set_var_float("FenceLungeX", 0.0);
    z.set_var_float("FenceLungeY", 0.0);
    let (fx, fy) = (z.vars.forward_x, z.vars.forward_y);
    let (x, y) = (z.vars.x, z.vars.y);
    // PZMath.testSideOfLine
    let d = (tx - x) * ((y + fy) - y) - (ty - y) * ((x + fx) - x);
    // getDotWithForwardDirection: both vectors normalized.
    let (vx, vy) = normalize(tx - x, ty - y);
    let (lx, ly) = normalize(z.look_vector().0, z.look_vector().1);
    let angle_rad = ((vx * lx + vy * ly) as f64).acos() as f32;
    let angle_deg = (57.295776f32 * angle_rad).clamp(0.0, 90.0);
    let lunge_x = if d > 0.0 {
        -angle_deg / 90.0
    } else if d < 0.0 {
        angle_deg / 90.0
    } else {
        0.0
    };
    z.set_var_float("FenceLungeX", lunge_x);
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
        if RUN.get(&mut owner.state.params) {
            owner.set_var_bool("VaultOverRun", true);
            // TODO(stats): the endurance cost of a running vault.
        } else if SPRINT.get(&mut owner.state.params) {
            owner.set_var_bool("VaultOverSprint", true);
        }
        let counter = COUNTER.get(&mut owner.state.params);
        owner.set_var_bool("ClimbingFence", true);
        owner.set_var_bool("ClimbFenceStarted", false);
        owner.set_var_bool("ClimbFenceFinished", false);
        owner.set_var_str("ClimbFenceOutcome", if counter { "obstacle" } else { "success" });
        owner.clear_var("ClimbFenceFlopped");
        // TODO(players): shouldFallAfterVaultOver for a running vault.
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
        let outcome = owner.get_variable_java("ClimbFenceOutcome");
        OUTCOME.set(&mut owner.state.params, outcome);
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
                // TODO(objects): the fence damage of two zombies at once.
                owner.set_var_bool("ClimbFenceFlopped", true);
            }
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
