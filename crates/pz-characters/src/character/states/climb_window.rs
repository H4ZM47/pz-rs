//! `ClimbThroughWindowState`: climbing through a window, a window
//! thumpable or a window frame (players and zombies), with
//! `IsoGameCharacter.climbThroughWindow` / `climbThroughWindowFrame` and
//! the positioning params (`ClimbThroughWindowPositioningParams`).
//!
//! The squares are the world's ([`StateWorld::window_climb`],
//! [`StateWorld::window_between`], ...). Not here: `CheckAttack`
//! (`attackFromWindowsLunge` needs the animation player's bones), a
//! player's music intensity events and `dirtyRecalcGridStackTime`, and the
//! multiplayer paths.
//!
//! [`StateWorld::window_climb`]: crate::character::state::StateWorld::window_climb
//! [`StateWorld::window_between`]: crate::character::state::StateWorld::window_between

use pz_pathfind::mover::ObjectRef;
use pz_world::square::{IsoDirections, MoverKind};

use crate::character::state::{MoveDeltaModifiers, Param, State, StateCx, StateId, StateRef, ThumpKind, WindowClimb};
use crate::character::zombie::math::{fastfloor, normalize};
use crate::character::{FloorCx, GameCharacter, SimEvent};
use pz_anim::{AnimEventRecord, AnimationVariableSource};

use super::ids;

/// `ClimbThroughWindowState.PARAMS`: the positioning params (`None`
/// before `setParams`).
pub const PARAMS: Param<Option<Positioning>> = Param::new(ids::CLIMB_THROUGH_WINDOW, "params", || None);
pub const PREV_STATE: Param<Option<StateId>> = Param::new(ids::CLIMB_THROUGH_WINDOW, "prev_state", || None);
pub const ZOMBIE_ON_FLOOR: Param<bool> = Param::new(ids::CLIMB_THROUGH_WINDOW, "zombie_on_floor", || false);
pub const OUTCOME: Param<String> = Param::new(ids::CLIMB_THROUGH_WINDOW, "outcome", || "success".to_string());
pub const SCRATCHED: Param<bool> = Param::new(ids::CLIMB_THROUGH_WINDOW, "scratched", || false);

/// `ClimbThroughWindowPositioningParams`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Positioning {
    pub window: ObjectRef,
    pub portal: ThumpKind,
    pub can_climb: bool,
    pub climb_dir: IsoDirections,
    pub start_x: i32,
    pub start_y: i32,
    pub z: i32,
    pub opposite_x: i32,
    pub opposite_y: i32,
    pub end_x: i32,
    pub end_y: i32,
    pub scratch: bool,
    pub is_counter: bool,
    pub is_floor: bool,
    pub is_sheet_rope: bool,
}

/// `ClimbThroughWindowState`.
pub struct ClimbThroughWindowState;

/// The singleton (`instance()`).
pub fn climb_through_window<I: 'static>() -> StateRef<I> {
    static S: ClimbThroughWindowState = ClimbThroughWindowState;
    &S
}

fn mover_kind<I: 'static>(c: &GameCharacter<I>) -> MoverKind {
    crate::character::collide::mover_kind(c.kind)
}

/// `getClimbThroughWindowPositioningParams(chr, window, params)`: the
/// world's part, then a player's chance of a scratch (a broken pane, or
/// a barbed fence thumpable).
pub fn positioning_params<I: 'static>(chr: &GameCharacter<I>, window: ObjectRef, cx: &StateCx<'_, I>) -> Option<(WindowClimb, Positioning)> {
    let w = cx.world.window_climb(chr, mover_kind(chr), window)?;
    let player = chr.is_player();
    let scratch = match w.portal {
        ThumpKind::Window => player && w.broken_glass && cx.ctx.rand_next(2) == 0,
        ThumpKind::Thumpable => player && w.barbed && cx.ctx.rand_next(101) > 75,
        _ => false,
    };
    let p = Positioning {
        window,
        portal: w.portal,
        can_climb: w.can_climb,
        climb_dir: w.climb_dir,
        start_x: w.start.0,
        start_y: w.start.1,
        z: w.z,
        opposite_x: w.opposite.0,
        opposite_y: w.opposite.1,
        end_x: w.end.0,
        end_y: w.end.1,
        scratch,
        is_counter: w.counter,
        is_floor: w.floor,
        is_sheet_rope: w.sheet_rope,
    };
    Some((w, p))
}

/// `ClimbThroughWindowState.setParams(owner, obj)`.
pub fn set_params<I: 'static>(owner: &mut GameCharacter<I>, window: ObjectRef, cx: &StateCx<'_, I>) {
    // The original throws for an object that is no portal.
    let Some((_, p)) = positioning_params(owner, window, cx) else { return };
    let prev = owner.current_state_id();
    let params = &mut owner.state.params;
    PARAMS.set(params, Some(p));
    ZOMBIE_ON_FLOOR.set(params, false);
    PREV_STATE.set(params, prev);
    SCRATCHED.set(params, p.scratch);
}

impl<I: 'static> GameCharacter<I> {
    /// `climbThroughWindow(IsoObject)`: an `IsoWindow` or `IsoThumpable`
    /// the character can climb through puts it in the window's opening,
    /// then `setParams` and `EventClimbWindow`.
    pub fn climb_through_window(&mut self, window: ObjectRef, cx: &mut StateCx<'_, I>) {
        if cx.has_host_error() { return; }
        let Some(w) = cx.world.window_climb(&*self, mover_kind(self), window) else { return };
        if !matches!(w.portal, ThumpKind::Window | ThumpKind::Thumpable) || !w.portal_can_climb {
            return;
        }
        self.drop_heavy_items_in(cx);
        if cx.has_host_error() { return; }
        let ox = self.vars.x - fastfloor(self.vars.x) as f32;
        let oy = self.vars.y - fastfloor(self.vars.y) as f32;
        let (wx, wy) = (w.square.0 as f32, w.square.1 as f32);
        let x_mod = if wx > self.vars.x && !w.north { -1 } else { 0 };
        let y_mod = if wy > self.vars.y && w.north { -1 } else { 0 };
        self.set_x(wx + ox + x_mod as f32);
        self.set_y(wy + oy + y_mod as f32);
        set_params(self, window, cx);
        self.report_event("EventClimbWindow");
    }

    /// `climbThroughWindowFrame(frame)`.
    pub fn climb_through_window_frame(&mut self, frame: ObjectRef, cx: &mut StateCx<'_, I>) {
        if cx.has_host_error() { return; }
        let Some(w) = cx.world.window_climb(&*self, mover_kind(self), frame) else { return };
        if w.portal != ThumpKind::WindowFrame || !w.portal_can_climb {
            return;
        }
        self.drop_heavy_items_in(cx);
        if cx.has_host_error() { return; }
        set_params(self, frame, cx);
        self.report_event("EventClimbWindow");
    }

    /// `dropHeavyItems()` into the states' world ([`crate::character::state::StateWorld::floor`];
    /// nothing drops without one). `climbThroughWindow*`,
    /// `climbThroughWindowFrame`, `climbSheetRope`, `climbDownSheetRope`
    /// and `IsoPlayer.climbOverWall` run it first.
    pub fn drop_heavy_items_in(&mut self, cx: &mut StateCx<'_, I>) {
        if cx.has_host_error() { return; }
        let (sm, ctx) = (cx.sm, cx.ctx);
        if let Some(floor) = cx.world.floor() {
            if let Err(error) = self.drop_heavy_items(sm, FloorCx::new(ctx, floor)) {
                cx.record_host_error(error);
            }
        }
    }

    /// `dropHandItems()` into the states' world ([`crate::character::state::StateWorld::floor`];
    /// nothing drops without one).
    pub fn drop_hand_items_in(&mut self, cx: &mut StateCx<'_, I>) {
        if cx.has_host_error() { return; }
        let (sm, ctx) = (cx.sm, cx.ctx);
        if let Some(floor) = cx.world.floor() {
            if let Err(error) = self.drop_hand_items(sm, FloorCx::new(ctx, floor)) {
                cx.record_host_error(error);
            }
        }
    }
}

fn positioning<I: 'static>(owner: &GameCharacter<I>) -> Option<Positioning> {
    PARAMS.peek(&owner.state.params).flatten()
}

/// `slideX(owner, x)`.
pub fn slide_x<I: 'static>(owner: &mut GameCharacter<I>, x: f32, thirty_fps: f32) {
    let mut dx = 0.05 * thirty_fps;
    dx = if x > owner.vars.x { dx.min(x - owner.vars.x) } else { (-dx).max(x - owner.vars.x) };
    owner.set_x(owner.vars.x + dx);
    owner.vars.next_x = owner.vars.x;
}

/// `slideY(owner, y)`.
pub fn slide_y<I: 'static>(owner: &mut GameCharacter<I>, y: f32, thirty_fps: f32) {
    let mut dy = 0.05 * thirty_fps;
    dy = if y > owner.vars.y { dy.min(y - owner.vars.y) } else { (-dy).max(y - owner.vars.y) };
    owner.set_y(owner.vars.y + dy);
    owner.vars.next_y = owner.vars.y;
}

/// `slideCharacterToWindowOpening(chr, params)`.
pub fn slide_character_to_window_opening<I: 'static>(chr: &mut GameCharacter<I>, p: &Positioning, thirty_fps: f32) {
    let d = p.climb_dir;
    if matches!(d, IsoDirections::N | IsoDirections::S) {
        let start_x = p.start_x as f32 + 0.5;
        if chr.vars.x != start_x {
            slide_x(chr, start_x, thirty_fps);
        }
    }
    if matches!(d, IsoDirections::W | IsoDirections::E) {
        let start_y = p.start_y as f32 + 0.5;
        if chr.vars.y != start_y {
            slide_y(chr, start_y, thirty_fps);
        }
    }
}

/// `setLungeXVars(zombie)` (shared with `ClimbOverFenceState`).
pub fn set_lunge_x_vars<I: 'static>(z: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
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

/// `IsoZombie.shouldDoFenceLunge()` (shared with `ClimbOverFenceState`).
pub fn should_do_fence_lunge<I: 'static>(z: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) -> bool {
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
    crate::character::zombie::math::distance_manhatten(zx, zy, tx, ty) < 3.9
}

/// `ParameterZombieState.State.valueOf(name)`.
pub fn zombie_sound_state(name: &str) -> Option<&'static str> {
    [
        "Idle",
        "Eating",
        "SearchTarget",
        "LockTarget",
        "AttackScratch",
        "AttackLacerate",
        "AttackBite",
        "Hit",
        "Death",
        "Reanimate",
        "Pushed",
        "GettingUp",
        "Attack",
        "RunOver",
    ]
    .into_iter()
    .find(|s| *s == name)
}

/// `BloodBodyPartType` indices.
const BLOOD_TORSO_UPPER: i32 = 6;
const BLOOD_HEAD: i32 = 8;
const BLOOD_NECK: i32 = 9;

impl ClimbThroughWindowState {
    /// `isWindowClosing(owner)`: someone closes the window before the
    /// climb started.
    fn is_window_closing<I: 'static>(&self, owner: &mut GameCharacter<I>, p: &Positioning, cx: &StateCx<'_, I>) -> bool {
        if owner.get_variable_boolean("ClimbWindowStarted") {
            return false;
        }
        if owner.path.current_square != Some((p.start_x, p.start_y, p.z)) || cx.world.square(p.start_x, p.start_y, p.z).is_none() {
            return false;
        }
        let Some((w, kind)) = self.get_window(owner, p, cx) else { return false };
        if kind != ThumpKind::Window || !cx.world.window_closing(w) {
            return false;
        }
        if owner.is_zombie() {
            owner.vars.hit_reaction = "HeadLeft".into();
        } else {
            owner.set_var_bool("ClimbWindowFinished", true);
        }
        true
    }

    /// `getWindow(owner)`.
    fn get_window<I: 'static>(&self, owner: &GameCharacter<I>, p: &Positioning, cx: &StateCx<'_, I>) -> Option<(ObjectRef, ThumpKind)> {
        if !owner.is_current_state_id(ids::CLIMB_THROUGH_WINDOW) {
            return None;
        }
        let (start, end) = ((p.start_x, p.start_y, p.z), (p.end_x, p.end_y, p.z));
        cx.world.square(start.0, start.1, start.2)?;
        cx.world.square(end.0, end.1, end.2)?;
        cx.world.window_between(start, end)
    }

    /// `checkForFallingBack(sq, owner)`: a zombie on the far side throws the
    /// climber back.
    fn check_for_falling_back<I: 'static>(owner: &mut GameCharacter<I>, sq: (i32, i32, i32), cx: &mut StateCx<'_, I>) {
        let fitness = cx.chars.perks.constant("Fitness");
        for id in cx.world.zombies_on_square(sq) {
            let Some(z) = (if id == owner.id { None } else { cx.world.character_mut(id) }) else { continue };
            if z.vars.on_floor || z.vars.sit_against_wall {
                continue;
            }
            let success = z.variable_string("AttackOutcome").eq_ignore_ascii_case("success");
            if success || cx.ctx.rand_next(5 + owner.perk_level(fitness.as_ref())) == 0 {
                owner.set_var_str("ClimbWindowOutcome", "fallback");
                continue;
            }
            owner.set_var_str("ClimbWindowOutcome", "back");
        }
    }

    /// `checkForFallingFront(sq, owner)`.
    fn check_for_falling_front<I: 'static>(owner: &mut GameCharacter<I>, sq: (i32, i32, i32), cx: &mut StateCx<'_, I>) {
        for id in cx.world.zombies_on_square(sq) {
            let Some(z) = (if id == owner.id { None } else { cx.world.character_mut(id) }) else { continue };
            if z.vars.on_floor || z.vars.sit_against_wall || !z.variable_string("AttackOutcome").eq_ignore_ascii_case("success") {
                continue;
            }
            owner.set_var_str("ClimbWindowOutcome", "fallfront");
        }
    }
}

impl<I: 'static> State<I> for ClimbThroughWindowState {
    fn id(&self) -> StateId {
        ids::CLIMB_THROUGH_WINDOW
    }

    fn sync_flags(&self) -> crate::character::state::SyncFlags {
        crate::character::state::SyncFlags::new(true, false, true, false)
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        owner.vars.hide_weapon_model = true;
        let Some(p) = positioning(owner) else { return };
        let counter = p.is_counter;
        owner.set_var_bool("ClimbWindowStarted", false);
        owner.set_var_bool("ClimbWindowEnd", false);
        owner.set_var_bool("ClimbWindowFinished", false);
        owner.clear_var("ClimbWindowGetUpBack");
        owner.clear_var("ClimbWindowGetUpFront");
        owner.set_var_str("ClimbWindowOutcome", if counter { "obstacle" } else { "success" });
        owner.clear_var("ClimbWindowFlopped");
        if !counter && owner.is_zombie() && should_do_fence_lunge(owner, cx) {
            set_lunge_x_vars(owner, cx);
            owner.set_var_str("ClimbWindowOutcome", "lunge");
        }
        if !p.is_floor {
            owner.set_var_str("ClimbWindowOutcome", "fall");
        }
        if !owner.is_zombie() && p.is_sheet_rope {
            owner.set_var_str("ClimbWindowOutcome", "rope");
        }
        // setParams(owner, Stage.Enter): a local character keeps its outcome.
        if !owner.vars.remote {
            let o = owner.get_variable_java("ClimbWindowOutcome");
            OUTCOME.set(&mut owner.state.params, o);
        } else {
            let o = OUTCOME.get(&mut owner.state.params);
            owner.set_var_str("ClimbWindowOutcome", &o);
        }
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let Some(mut p) = positioning(owner) else { return };
        if self.is_window_closing(owner, &p, cx) {
            return;
        }
        owner.set_forward_iso_direction(p.climb_dir);
        let outcome = owner.get_variable_java("ClimbWindowOutcome");
        let opposite = (p.opposite_x, p.opposite_y, p.z);
        if owner.is_zombie() {
            let down = ZOMBIE_ON_FLOOR.get(&mut owner.state.params);
            if !owner.environment.fall_on_front && down && cx.world.square(opposite.0, opposite.1, opposite.2).is_some() && cx.world.has_broken_glass(opposite) {
                for part in [BLOOD_HEAD, BLOOD_HEAD, BLOOD_HEAD, BLOOD_HEAD, BLOOD_HEAD, BLOOD_NECK, BLOOD_NECK, BLOOD_NECK, BLOOD_NECK, BLOOD_TORSO_UPPER, BLOOD_TORSO_UPPER, BLOOD_TORSO_UPPER] {
                    cx.ctx.event(SimEvent::AddBlood { part, scratched: true, bitten: true, all_layers: true });
                }
            }
            owner.vars.on_floor = down;
            owner.vars.knocked_down = down;
            owner.environment.fall_on_front = down;
        }
        let t30 = cx.ctx.thirty_fps_multiplier();
        if !owner.get_variable_boolean("ClimbWindowStarted") {
            slide_character_to_window_opening(owner, &p, t30);
        }
        if owner.is_player() && outcome.eq_ignore_ascii_case("obstacle") {
            let (ex, ey) = (p.end_x as f32 + 0.5, p.end_y as f32 + 0.5);
            if owner.dist_to_squared_xy(ex, ey) < 0.5625 {
                owner.set_var_str("ClimbWindowOutcome", "obstacleEnd");
            }
        }
        if owner.is_player() && !owner.get_variable_boolean("ClimbWindowEnd") && !["fallfront", "back", "fallback"].contains(&outcome.as_str()) {
            if cx.world.square(opposite.0, opposite.1, opposite.2).is_some() {
                Self::check_for_falling_back(owner, opposite, cx);
                if owner.path.current_square != Some(opposite) && cx.world.square(opposite.0, opposite.1, opposite.2).is_some_and(|s| s.solid_floor) {
                    if let Some(cur) = owner.path.current_square {
                        Self::check_for_falling_front(owner, cur, cx);
                    }
                }
            }
            let drunk = owner.moodles.moodle_level_of(cx.sm, "DRUNK").unwrap_or(0);
            if drunk > 1 && owner.get_variable_java("ClimbWindowOutcome") == outcome {
                let intoxication = cx.chars.sim.character_stats.field("INTOXICATION").map_or(0.0, |s| owner.stats.get(&s));
                if (cx.ctx.rand_next(2000) as f32) < intoxication {
                    // Rand.NextBool(2): Next(2) == 0.
                    let o = if cx.ctx.rand_next(2) == 0 { "fallback" } else { "fallfront" };
                    owner.set_var_str("ClimbWindowOutcome", o);
                }
            }
        }
        let started = owner.get_variable_boolean("ClimbWindowStarted");
        if started && !["back", "fallback", "lunge", "obstacle", "obstacleEnd"].contains(&outcome.as_str()) {
            let (mut x, mut y) = (p.start_x as f32, p.start_y as f32);
            match p.climb_dir {
                IsoDirections::N => y -= 0.1,
                IsoDirections::S => y += 1.1,
                IsoDirections::W => x -= 0.1,
                IsoDirections::E => x += 1.1,
                _ => {}
            }
            let we = matches!(p.climb_dir, IsoDirections::W | IsoDirections::E);
            let ns = matches!(p.climb_dir, IsoDirections::N | IsoDirections::S);
            if fastfloor(owner.vars.x) != fastfloor(x) && we {
                slide_x(owner, x, t30);
            }
            if fastfloor(owner.vars.y) != fastfloor(y) && ns {
                slide_y(owner, y, t30);
            }
        }
        if started && p.scratch {
            p.scratch = false;
            PARAMS.set(&mut owner.state.params, Some(p));
            let _ = owner.set_scratched_window(cx.sm, cx.chars, cx.ctx);
            if owner.is_player() {
                cx.ctx.event(SimEvent::PlayerVoiceSound("PainFromGlassCut"));
            }
        }
        if started && owner.get_variable_java("ClimbWindowOutcome").eq_ignore_ascii_case("fall") {
            owner.vars.falling = true;
        }
    }

    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = false;
        owner.vars.hide_weapon_model = false;
        let o = owner.get_variable_java("ClimbWindowOutcome");
        if ["fall", "fallback", "fallfront"].iter().any(|s| o.eq_ignore_ascii_case(s)) {
            owner.vars.hit_reaction = String::new();
        }
        for v in ["ClimbWindowFinished", "ClimbWindowOutcome", "ClimbWindowStarted", "ClimbWindowFlopped", "PlayerVoiceSound"] {
            owner.clear_var(v);
        }
        if owner.is_zombie() {
            owner.vars.on_floor = false;
            owner.vars.knocked_down = false;
            owner.zombie.allow_repath_delay = 0.0;
            let prev = PREV_STATE.get(&mut owner.state.params);
            if prev == Some(ids::PATH_FIND) {
                let target_char = cx.world.path_behavior(owner.id).and_then(|b| b.target_char());
                match target_char {
                    None => {
                        owner.set_var_bool("bPathFind", true);
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
        owner.state.params.clear(ids::CLIMB_THROUGH_WINDOW);
    }

    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        let name = e.event.event_name.as_str();
        let param = e.event.parameter_value.as_deref().unwrap_or("");
        let zombie = owner.is_zombie();
        // CheckAttack: attackFromWindowsLunge (TODO(anim bones)).
        if name.eq_ignore_ascii_case("OnFloor") && zombie {
            let down = param.eq_ignore_ascii_case("true");
            ZOMBIE_ON_FLOOR.set(&mut owner.state.params, down);
            if down {
                set_lunge_x_vars(owner, cx);
                let w = positioning(owner).and_then(|p| self.get_window(owner, &p, cx));
                if let Some((w, ThumpKind::Thumpable)) = w {
                    if owner.vars.target.is_some() {
                        let damage = cx.ctx.rand_next_range(10, 20);
                        cx.world.damage_thumpable(w, damage);
                    }
                }
                owner.set_var_bool("ClimbWindowFlopped", true);
            }
        }
        if name.eq_ignore_ascii_case("PlayerVoiceSound") {
            if owner.get_variable_boolean("PlayerVoiceSound") || !owner.is_player() {
                return;
            }
            owner.set_var_bool("PlayerVoiceSound", true);
            cx.ctx.event(SimEvent::PlayerVoiceSoundNamed(param.to_string()));
        }
        if name.eq_ignore_ascii_case("PlayWindowSound") {
            if !cx.ctx.is_listener_in_range(owner.vars.x, owner.vars.y, 70.0) {
                return;
            }
            cx.ctx.event(SimEvent::ObstacleSound { name: param.to_string(), parameter: "TripObstacleType", value: 9.0, movement_speed: false });
        }
        if name.eq_ignore_ascii_case("SetState") {
            if !zombie {
                return;
            }
            if let Some(s) = zombie_sound_state(param) {
                owner.zombie.sound_state = s;
            }
        }
    }

    fn get_delta_modifiers(&self, owner: &GameCharacter<I>, m: &mut MoveDeltaModifiers) {
        let player = owner.is_player();
        if owner.path.path2.is_some() && player {
            m.set_max_turn_delta(2.0);
        }
        if player && owner.get_variable_boolean("isTurning") {
            m.set_max_turn_delta(2.0);
        }
    }

    fn is_ignore_collide(&self, owner: &GameCharacter<I>, from: (i32, i32, i32), to: (i32, i32, i32)) -> bool {
        let Some(p) = positioning(owner) else { return false };
        if p.z != from.2 || p.z != to.2 {
            return false;
        }
        let (x1, y1, x2, y2) = (p.start_x.min(p.end_x), p.start_y.min(p.end_y), p.start_x.max(p.end_x), p.start_y.max(p.end_y));
        let (x3, y3, x4, y4) = (from.0.min(to.0), from.1.min(to.1), from.0.max(to.0), from.1.max(to.1));
        x1 <= x3 && y1 <= y3 && x2 >= x4 && y2 >= y4
    }

    fn can_ragdoll(&self, owner: &GameCharacter<I>) -> bool {
        if owner.variable_boolean_or("ClimbWindowStarted", false) {
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
    use crate::character::state::{SquareInfo, StateWorld};
    use crate::character::tests::{fixture, Recorder};
    use crate::character::CharacterKind;
    use pz_pathfind::mover::CharacterView;

    const WIN: ObjectRef = ObjectRef { x: 5, y: 5, z: 0, id: 3 };

    /// A north window on (5, 5), floors everywhere, broken glass beyond.
    #[derive(Default)]
    struct Windowed {
        others: Vec<GameCharacter<()>>,
        thumpable: bool,
        damaged: Vec<i32>,
        floor: crate::character::floor_drop::tests::TestFloor,
    }

    impl StateWorld<()> for Windowed {
        fn character_mut(&mut self, id: i32) -> Option<&mut GameCharacter<()>> {
            self.others.iter_mut().find(|c| c.id == id)
        }
        fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
            Some(SquareInfo { x, y, z, solid_floor: true, ..Default::default() })
        }
        fn window_climb(&self, chr: &dyn CharacterView, _k: MoverKind, w: ObjectRef) -> Option<WindowClimb> {
            (w == WIN).then(|| {
                let from_south = 5.0 < chr.y();
                WindowClimb {
                    portal: if self.thumpable { ThumpKind::Thumpable } else { ThumpKind::Window },
                    can_climb: true,
                    portal_can_climb: true,
                    broken_glass: false,
                    barbed: false,
                    north: true,
                    square: (5, 5, 0),
                    climb_dir: if from_south { IsoDirections::N } else { IsoDirections::S },
                    start: if from_south { (5, 5) } else { (5, 4) },
                    opposite: if from_south { (5, 4) } else { (5, 5) },
                    end: if from_south { (5, 4) } else { (5, 5) },
                    z: 0,
                    counter: false,
                    floor: true,
                    sheet_rope: false,
                }
            })
        }
        fn window_between(&self, _a: (i32, i32, i32), _b: (i32, i32, i32)) -> Option<(ObjectRef, ThumpKind)> {
            Some((WIN, if self.thumpable { ThumpKind::Thumpable } else { ThumpKind::Window }))
        }
        fn has_broken_glass(&self, sq: (i32, i32, i32)) -> bool {
            sq == (5, 4, 0)
        }
        fn damage_thumpable(&mut self, _o: ObjectRef, d: i32) {
            self.damaged.push(d);
        }
        fn floor(&mut self) -> Option<&mut dyn pz_inventory::floor_drop::FloorWorld> {
            Some(&mut self.floor)
        }
    }

    fn zombie(sm: &pz_scripts::ScriptManager) -> GameCharacter<()> {
        let mut z = GameCharacter::new(sm, CharacterKind::Zombie);
        z.id = 7;
        z.vars.x = 5.3;
        z.vars.y = 5.6;
        z.path.current_square = Some((5, 5, 0));
        z
    }

    #[test]
    fn a_zombie_climbs_a_window_and_flops_on_the_glass() {
        let (sm, chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Windowed { thumpable: true, ..Default::default() };
        let mut z = zombie(&sm);
        z.vars.target = Some(1);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        // From the south of a north window: no offset in y.
        z.climb_through_window(WIN, &mut cx);
        let p = positioning(&z).unwrap();
        assert_eq!((p.climb_dir, p.start_x, p.start_y, p.end_y), (IsoDirections::N, 5, 5, 4));
        assert_eq!((z.vars.x, z.vars.y), (5.3, 5.6));
        let s = climb_through_window::<()>();
        z.change_state(Some(s), &mut cx);
        // No target in the world: no fence lunge.
        assert_eq!(z.get_variable_java("ClimbWindowOutcome"), "success");
        assert!(s.is_ignore_collide(&z, (5, 5, 0), (5, 4, 0)));
        assert!(!s.is_ignore_collide(&z, (5, 5, 0), (6, 5, 0)));
        // Before the climb starts it slides to the opening's centre.
        s.execute(&mut z, &mut cx);
        assert_eq!(z.vars.dir, IsoDirections::N);
        assert!(z.vars.x > 5.3 && z.vars.x <= 5.5);
        // On the floor: a thumpable window with a target loses 10..20.
        let ev = |n: &str, p: &str| AnimEventRecord {
            state_name: None,
            layer_depth: -1,
            track: None,
            event: std::rc::Rc::new(pz_anim::AnimEvent { event_name: n.into(), parameter_value: Some(p.into()), ..Default::default() }),
        };
        s.anim_event(&mut z, &mut cx, &ev("OnFloor", "true"));
        assert!(ZOMBIE_ON_FLOOR.get(&mut z.state.params));
        assert!(z.get_variable_boolean("ClimbWindowFlopped"));
        z.set_var_bool("ClimbWindowStarted", true);
        let before = ctx.events.borrow().len();
        s.execute(&mut z, &mut cx);
        assert!(z.vars.on_floor && z.vars.knocked_down);
        // Broken glass beyond: 12 blood splats on the head, neck and torso.
        assert_eq!(ctx.events.borrow().len() - before, 12);
        // Started: it slides towards the far square.
        assert!(z.vars.y < 5.6);
        s.exit(&mut z, &mut cx);
        assert!(!z.vars.on_floor);
        assert!(positioning(&z).is_none());
        let _ = cx;
        assert_eq!(w.damaged.len(), 1);
        assert!((10..20).contains(&w.damaged[0]));
    }

    #[test]
    fn a_climb_from_the_north_side_moves_into_the_opening() {
        let (sm, chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Windowed::default();
        let mut z = zombie(&sm);
        z.vars.y = 4.4;
        z.path.current_square = Some((5, 4, 0));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        z.climb_through_window(WIN, &mut cx);
        // `w.getY() > getY() && north`: one square up from the window.
        assert_eq!(z.vars.y, 5.0 + (4.4 - 4.0) - 1.0);
        let p = positioning(&z).unwrap();
        assert_eq!((p.climb_dir, p.start_y, p.end_y), (IsoDirections::S, 4, 5));
    }

    #[test]
    fn climbing_through_a_window_drops_heavy_hand_items() {
        use crate::character::tests::{give_item, player};
        use pz_inventory::ItemClass;
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Windowed::default();
        let mut p = player(&sm, &mut chars);
        (p.vars.x, p.vars.y) = (5.3, 5.6);
        p.path.current_square = Some((5, 5, 0));
        give_item(&sm, &mut p, ItemClass::Container, "Duffel", 1, |_| {}, |_| {});
        give_item(&sm, &mut p, ItemClass::Normal, "Spoon", 2, |_| {}, |_| {});
        p.set_primary_hand_item(&sm, Some(2));
        p.set_secondary_hand_item(&sm, Some(1));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        p.climb_through_window(WIN, &mut cx);
        assert!(positioning(&p).is_some());
        let _ = cx;
        // The bag falls on the square the climb starts from; the spoon stays.
        assert_eq!(w.floor.put.iter().map(|d| (d.0, d.1)).collect::<Vec<_>>(), [((5, 5, 0), 1)]);
        assert_eq!((p.items.primary_hand_item, p.items.secondary_hand_item), (Some(2), None));
        assert!(p.items.item(1).is_none());
    }
}
