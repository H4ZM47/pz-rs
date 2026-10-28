//! `OpenWindowState`, `CloseWindowState` and `SmashWindowState` (a
//! player opening, closing or smashing an `IsoWindow`, or smashing a
//! vehicle window), with `IsoGameCharacter.openWindow` / `closeWindow` /
//! `smashWindow` / `smashCarWindow`, `isClimbingThroughWindow` and
//! `exert`.
//!
//! The window itself is the world's ([`StateWorld::window_info`],
//! [`StateWorld::toggle_window`], [`StateWorld::window_weapon_hit`]).
//! Not here: the debug cheat that unlocks windows on enter,
//! `IsoPlayer.updateLOS()` (TODO(los)), the network `setParams(stage)`
//! (only the multiplayer code calls it) and `processOnEnter`.
//!
//! [`StateWorld::window_info`]: crate::character::state::StateWorld::window_info
//! [`StateWorld::toggle_window`]: crate::character::state::StateWorld::toggle_window
//! [`StateWorld::window_weapon_hit`]: crate::character::state::StateWorld::window_weapon_hit

use pz_anim::AnimEventRecord;
use pz_pathfind::mover::{FaceTarget, ObjectRef, PathMover};
use pz_scripts::ScriptManager;
use pz_world::square::IsoDirections;

use super::ids;
use crate::character::items::{is_hand_weapon, script_item};
use crate::character::state::{Param, State, StateCx, StateId, StateRef, SyncFlags, WindowInfo};
use crate::character::{GameCharacter, SimEvent};
use crate::Characters;

/// `OpenWindowState.WINDOW`.
pub const OPEN_WINDOW: Param<Option<ObjectRef>> = Param::new(ids::OPEN_WINDOW, "window", || None);
/// `CloseWindowState.ISO_WINDOW`.
pub const CLOSE_WINDOW: Param<Option<ObjectRef>> =
    Param::new(ids::CLOSE_WINDOW, "iso_window", || None);
/// `SmashWindowState.SCRATCHED`.
pub const SMASH_SCRATCHED: Param<bool> = Param::new(ids::SMASH_WINDOW, "scratched", || false);
/// `SmashWindowState.ISO_WINDOW`.
pub const SMASH_ISO_WINDOW: Param<Option<ObjectRef>> =
    Param::new(ids::SMASH_WINDOW, "iso_window", || None);
/// `SmashWindowState.VEHICLE_WINDOW`, `VEHICLE` and `VEHICLE_PART`: the
/// vehicle (`vehicleId`) and the window's part id.
pub const SMASH_VEHICLE_WINDOW: Param<Option<(i16, String)>> =
    Param::new(ids::SMASH_WINDOW, "vehicle_window", || None);
/// `SmashWindowState.CLIMB_THROUGH_WINDOW`.
pub const SMASH_CLIMB_THROUGH_WINDOW: Param<bool> =
    Param::new(ids::SMASH_WINDOW, "climb_through_window", || false);

/// `OpenWindowState`.
pub struct OpenWindowState;
/// `CloseWindowState`.
pub struct CloseWindowState;
/// `SmashWindowState`.
pub struct SmashWindowState;

/// `OpenWindowState.instance()`.
pub fn open_window<I: 'static>() -> StateRef<I> {
    static S: OpenWindowState = OpenWindowState;
    &S
}

/// `CloseWindowState.instance()`.
pub fn close_window<I: 'static>() -> StateRef<I> {
    static S: CloseWindowState = CloseWindowState;
    &S
}

/// `SmashWindowState.instance()`.
pub fn smash_window<I: 'static>() -> StateRef<I> {
    static S: SmashWindowState = SmashWindowState;
    &S
}

/// `CloseWindowState.getWindow(owner)`: the window `owner` is closing
/// (`IsoWindow.getFirstCharacterClosing`).
pub fn closing_window<I: 'static>(owner: &GameCharacter<I>) -> Option<ObjectRef> {
    if !owner.is_current_state_id(ids::CLOSE_WINDOW) {
        return None;
    }
    CLOSE_WINDOW.peek(&owner.state.params).flatten()
}

/// The facing towards a window from the character's side
/// (`window.isNorth()` and the square's `y` / `x`).
fn face_window<I: 'static>(owner: &mut GameCharacter<I>, w: &WindowInfo) {
    let dir = if w.north {
        if (w.square.1 as f32) < owner.vars.y {
            IsoDirections::N
        } else {
            IsoDirections::S
        }
    } else if (w.square.0 as f32) < owner.vars.x {
        IsoDirections::W
    } else {
        IsoDirections::E
    };
    owner.set_forward_iso_direction(dir);
}

/// `player.pressedMovement(false) || player.pressedCancelAction()`.
fn pressed_movement_or_cancel<I: 'static>(
    owner: &mut GameCharacter<I>,
    cx: &StateCx<'_, I>,
) -> bool {
    let Some(input) = cx.world.player_input() else {
        return false;
    };
    if owner.pressed_movement(input, false) {
        return true;
    }
    owner.pressed_cancel_action(input)
}

/// A square's coordinates.
type Square = (i32, i32, i32);

/// The start and end squares of a window climb `c` is in with
/// `BlockWindow` set (`ClimbThroughWindowState.getWindow`'s squares; the
/// squares' existence is the caller's `window_between`).
fn climbing_through<I: 'static>(c: &GameCharacter<I>) -> Option<(Square, Square)> {
    if !c.is_current_state_id(ids::CLIMB_THROUGH_WINDOW) || !c.variable_boolean("BlockWindow") {
        return None;
    }
    let p = super::climb_window::PARAMS
        .peek(&c.state.params)
        .flatten()?;
    Some(((p.start_x, p.start_y, p.z), (p.end_x, p.end_y, p.z)))
}

/// The window's param value.
fn info<I: 'static>(cx: &StateCx<'_, I>, w: Option<ObjectRef>) -> Option<(ObjectRef, WindowInfo)> {
    let w = w?;
    cx.world.window_info(w).map(|i| (w, i))
}

/// `"x".equalsIgnoreCase(event.parameterValue)` with a missing value.
fn param_is(e: &AnimEventRecord, s: &str) -> bool {
    e.event
        .parameter_value
        .as_deref()
        .is_some_and(|p| p.eq_ignore_ascii_case(s))
}

impl<I: 'static> GameCharacter<I> {
    /// `exert(f)`: endurance spent, a tenth less for joggers.
    pub fn exert(&mut self, sm: &ScriptManager, chars: &Characters, mut f: f32) {
        if self.has_trait(sm, "JOGGER").unwrap_or(false) {
            f *= 0.9;
        }
        if let Ok(e) = chars.sim.character_stats.field("ENDURANCE") {
            self.stats.remove(&e, f);
        }
    }

    /// `IsoPlayer.pressedCancelAction()` (single player).
    pub fn pressed_cancel_action(&self, input: &dyn crate::character::player::PlayerInput) -> bool {
        if self.player.npc {
            return false;
        }
        self.player
            .input
            .is_cancel_action_button_down(input, self.input_owner())
    }

    /// `openWindow(w)`: `OpenWindowState.setParams` and `EventOpenWindow`.
    pub fn open_window(&mut self, window: ObjectRef, cx: &StateCx<'_, I>) {
        let Some(w) = cx.world.window_info(window) else {
            return;
        };
        if w.invincible {
            return;
        }
        // OpenWindowState.setParams(owner, window).
        self.state.params.clear(ids::OPEN_WINDOW);
        OPEN_WINDOW.set(&mut self.state.params, Some(window));
        self.report_event("EventOpenWindow");
    }

    /// `closeWindow(w)`.
    pub fn close_window(&mut self, window: ObjectRef, cx: &StateCx<'_, I>) {
        let Some(w) = cx.world.window_info(window) else {
            return;
        };
        if w.invincible {
            return;
        }
        self.state.params.clear(ids::CLOSE_WINDOW);
        CLOSE_WINDOW.set(&mut self.state.params, Some(window));
        self.report_event("EventCloseWindow");
    }

    /// `smashWindow(w)`.
    pub fn smash_window(&mut self, window: ObjectRef, cx: &StateCx<'_, I>) {
        let Some(w) = cx.world.window_info(window) else {
            return;
        };
        if w.invincible {
            return;
        }
        self.state.params.clear(ids::SMASH_WINDOW);
        SMASH_ISO_WINDOW.set(&mut self.state.params, Some(window));
        self.report_event("EventSmashWindow");
    }

    /// `smashCarWindow(part)`: the window of `part` on `vehicle`.
    pub fn smash_car_window(&mut self, vehicle: i16, part: &str) {
        self.state.params.clear(ids::SMASH_WINDOW);
        SMASH_VEHICLE_WINDOW.set(&mut self.state.params, Some((vehicle, part.to_string())));
        self.report_event("EventSmashWindow");
    }

    /// `isClimbingThroughWindow(window)`.
    pub fn is_climbing_through_window(&self, window: ObjectRef, cx: &StateCx<'_, I>) -> bool {
        climbing_through(self).is_some_and(|(s, e)| {
            cx.world
                .window_between(s, e)
                .is_some_and(|(w, _)| w == window)
        })
    }

    /// `window.getFirstCharacterClimbingThrough() != null`: someone on the
    /// window's square or the opposite one climbs through it.
    fn window_climbed_through(
        &self,
        window: ObjectRef,
        w: &WindowInfo,
        cx: &mut StateCx<'_, I>,
    ) -> bool {
        for sq in [w.square, w.opposite()] {
            if cx.world.square(sq.0, sq.1, sq.2).is_none() {
                continue;
            }
            for id in cx.world.characters_on_square(sq) {
                let squares = if id == self.id {
                    climbing_through(self)
                } else {
                    let Some(c) = cx.world.character_mut(id) else {
                        continue;
                    };
                    climbing_through(c)
                };
                if squares.is_some_and(|(s, e)| {
                    cx.world
                        .window_between(s, e)
                        .is_some_and(|(o, _)| o == window)
                }) {
                    return true;
                }
            }
        }
        false
    }

    /// The hands hold no `HandWeapon`.
    fn no_hand_weapon(&self) -> bool {
        let primary = self
            .items
            .primary_hand_item()
            .is_some_and(|i| is_hand_weapon(&i.borrow()));
        let secondary = self
            .items
            .secondary_hand_item()
            .is_some_and(|i| is_hand_weapon(&i.borrow()));
        !primary && !secondary
    }
}

/// `OpenWindowState.exert(owner)`.
fn open_exert<I: 'static>(owner: &mut GameCharacter<I>, cx: &StateCx<'_, I>) {
    let level = owner.perk_level(cx.chars.perks.constant("Fitness").as_ref());
    let f = match level {
        0 => 0.011,
        1 => 0.01,
        2 => 0.009,
        3 => 0.008,
        4 => 0.007,
        5 => 0.006,
        6 => 0.005,
        7 => 0.004,
        8 => 0.003,
        9 => 0.0025,
        10 => 0.002,
        _ => 0.012,
    };
    owner.exert(cx.sm, cx.chars, f);
}

/// `CloseWindowState.exert(owner)` (per frame, nothing at level 0).
fn close_exert<I: 'static>(owner: &mut GameCharacter<I>, cx: &StateCx<'_, I>) {
    let delta = cx.ctx.thirty_fps_multiplier();
    let level = owner.perk_level(cx.chars.perks.constant("Fitness").as_ref());
    let f: f32 = match level {
        1 => 0.01,
        2 => 0.009,
        3 => 0.008,
        4 => 0.007,
        5 => 0.006,
        6 => 0.005,
        7 => 0.004,
        8 => 0.003,
        9 => 0.0025,
        10 => 0.002,
        _ => return,
    };
    owner.exert(cx.sm, cx.chars, f * delta);
}

impl OpenWindowState {
    /// `onAttemptFinished(owner, window)`.
    fn on_attempt_finished<I: 'static>(
        owner: &mut GameCharacter<I>,
        window: ObjectRef,
        w: &WindowInfo,
        cx: &mut StateCx<'_, I>,
    ) {
        open_exert(owner, cx);
        if w.perma_locked {
            // `getEmitter().isPlaying("WindowIsLocked")` has an empty body.
            owner.set_var_str("OpenWindowOutcome", "fail");
            owner.set_var_str("StopAfterAnimLooped", "fail");
            return;
        }
        let base_perma_lock_chance = if owner.has_trait(cx.sm, "BURGLAR").unwrap_or(false) {
            5
        } else {
            10
        };
        let exterior = owner
            .current_square_xyz()
            .is_some_and(|s| cx.world.square_is_outside(s));
        if w.locked && exterior {
            if cx.ctx.rand_next(100) < base_perma_lock_chance {
                cx.ctx
                    .event(SimEvent::EmitterPlaySound("BreakLockOnWindow".into()));
                cx.world.set_window_perma_locked(window, true);
                OPEN_WINDOW.set(&mut owner.state.params, None);
                owner.set_var_str("OpenWindowOutcome", "fail");
                owner.set_var_str("StopAfterAnimLooped", "fail");
                return;
            }
            let strength = owner.perk_level(cx.chars.perks.constant("Strength").as_ref());
            let success = (strength > 7 && cx.ctx.rand_next(100) < 20)
                || (strength > 5 && cx.ctx.rand_next(100) < 10)
                || (strength > 3 && cx.ctx.rand_next(100) < 6)
                || (strength > 1 && cx.ctx.rand_next(100) < 4)
                || cx.ctx.rand_next(100) <= 1;
            if success {
                owner.set_var_str("OpenWindowOutcome", "success");
            }
        } else {
            owner.set_var_str("OpenWindowOutcome", "success");
        }
    }

    /// `onSuccess(owner, window)`.
    fn on_success<I: 'static>(
        owner: &mut GameCharacter<I>,
        window: ObjectRef,
        w: &WindowInfo,
        cx: &mut StateCx<'_, I>,
    ) {
        owner.set_var_str("StopAfterAnimLooped", "success");
        owner.player.context_panic = 0.0;
        if w.in_world && !w.open && owner.is_local_player(cx.ctx) {
            cx.world.toggle_window(window, owner.id);
        }
    }
}

impl<I: 'static> State<I> for OpenWindowState {
    fn id(&self) -> StateId {
        ids::OPEN_WINDOW
    }

    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, true, true)
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        owner.vars.hide_weapon_model = true;
        // The original dereferences the window here.
        if let Some((_, w)) = info(cx, OPEN_WINDOW.get(&mut owner.state.params)) {
            face_window(owner, &w);
        }
        owner.set_var_bool("bOpenWindow", true);
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if !owner.variable_boolean("bOpenWindow") {
            return;
        }
        if pressed_movement_or_cancel(owner, cx) {
            owner.set_var_bool("bOpenWindow", false);
            return;
        }
        let window = OPEN_WINDOW.get(&mut owner.state.params);
        let Some((window, w)) = info(cx, window).filter(|(_, w)| w.in_world) else {
            owner.set_var_bool("bOpenWindow", false);
            return;
        };
        if owner.player.context_panic > 5.0 {
            owner.player.context_panic = 0.0;
            owner.set_var_bool("bOpenWindow", false);
            owner.smash_window(window, cx);
            SMASH_CLIMB_THROUGH_WINDOW.set(&mut owner.state.params, true);
            return;
        }
        owner.player.collidable = true;
        owner.path.collidable = true;
        face_window(owner, &w);
        if cx.ctx.is_tutorial() {
            let t30 = cx.ctx.thirty_fps_multiplier();
            let (wx, wy) = (w.square.0 as f32, w.square.1 as f32);
            if owner.vars.x != wx + 0.5 && w.north {
                super::climb_window::slide_x(owner, wx + 0.5, t30);
            }
            if owner.vars.y != wy + 0.5 && !w.north {
                super::climb_window::slide_y(owner, wy + 0.5, t30);
            }
        }
    }

    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = false;
        owner.clear_var("bOpenWindow");
        owner.clear_var("OpenWindowOutcome");
        owner.clear_var("StopAfterAnimLooped");
        owner.vars.hide_weapon_model = false;
    }

    fn anim_event(
        &self,
        owner: &mut GameCharacter<I>,
        cx: &mut StateCx<'_, I>,
        e: &AnimEventRecord,
    ) {
        if !owner.variable_boolean("bOpenWindow") {
            return;
        }
        let window = OPEN_WINDOW.get(&mut owner.state.params);
        let Some((window, w)) = info(cx, window) else {
            owner.set_var_bool("bOpenWindow", false);
            return;
        };
        let name = e.event.event_name.as_str();
        if name.eq_ignore_ascii_case("WindowAnimLooped") {
            if param_is(e, "start") {
                let exterior = owner
                    .current_square_xyz()
                    .is_some_and(|s| cx.world.square_is_outside(s));
                let o = if !(w.perma_locked || w.locked && exterior) {
                    "success"
                } else {
                    "struggle"
                };
                owner.set_var_str("OpenWindowOutcome", o);
                return;
            }
            let stop = owner.variable_string("StopAfterAnimLooped");
            if param_is(e, &stop) {
                owner.set_var_bool("bOpenWindow", false);
            }
        }
        if name.eq_ignore_ascii_case("WindowOpenAttempt") {
            Self::on_attempt_finished(owner, window, &w, cx);
        } else if name.eq_ignore_ascii_case("WindowOpenSuccess") {
            Self::on_success(owner, window, &w, cx);
        } else if name.eq_ignore_ascii_case("WindowStruggleSound")
            && owner.variable_string("OpenWindowOutcome") == "struggle"
        {
            cx.ctx.event(SimEvent::PlaySound("WindowIsLocked".into()));
        }
    }

    fn is_doing_action_that_can_be_cancelled(&self) -> bool {
        true
    }
}

impl CloseWindowState {
    fn on_attempt_finished<I: 'static>(
        owner: &mut GameCharacter<I>,
        window: ObjectRef,
        w: &WindowInfo,
        cx: &mut StateCx<'_, I>,
    ) {
        close_exert(owner, cx);
        if w.perma_locked {
            cx.ctx
                .event(SimEvent::EmitterPlaySound("WindowIsLocked".into()));
            owner.set_var_str("CloseWindowOutcome", "fail");
            owner.set_var_str("StopAfterAnimLooped", "fail");
        } else {
            // `randStruggle` (from the panic moodle) is unused.
            let o = if owner.window_climbed_through(window, w, cx) {
                "struggle"
            } else {
                "success"
            };
            owner.set_var_str("CloseWindowOutcome", o);
        }
    }

    fn on_success<I: 'static>(
        owner: &mut GameCharacter<I>,
        window: ObjectRef,
        w: &WindowInfo,
        cx: &mut StateCx<'_, I>,
    ) {
        owner.set_var_str("StopAfterAnimLooped", "success");
        owner.player.context_panic = 0.0;
        if w.in_world && w.open && owner.is_local_player(cx.ctx) {
            cx.world.toggle_window(window, owner.id);
        }
    }
}

impl<I: 'static> State<I> for CloseWindowState {
    fn id(&self) -> StateId {
        ids::CLOSE_WINDOW
    }

    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, true, true)
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        owner.vars.hide_weapon_model = true;
        if let Some((_, w)) = info(cx, CLOSE_WINDOW.get(&mut owner.state.params)) {
            face_window(owner, &w);
        }
        owner.set_var_bool("bCloseWindow", true);
        owner.clear_var("BlockWindow");
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        if !owner.variable_boolean("bCloseWindow") {
            return;
        }
        if pressed_movement_or_cancel(owner, cx) {
            owner.set_var_bool("bCloseWindow", false);
            return;
        }
        let window = CLOSE_WINDOW.get(&mut owner.state.params);
        let Some((_, w)) = info(cx, window).filter(|(_, w)| w.in_world) else {
            owner.set_var_bool("bCloseWindow", false);
            return;
        };
        owner.player.collidable = true;
        owner.path.collidable = true;
        face_window(owner, &w);
    }

    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.clear_var("BlockWindow");
        owner.clear_var("bCloseWindow");
        owner.clear_var("CloseWindowOutcome");
        owner.clear_var("StopAfterAnimLooped");
        owner.vars.ignore_movement = false;
        owner.vars.hide_weapon_model = false;
    }

    fn anim_event(
        &self,
        owner: &mut GameCharacter<I>,
        cx: &mut StateCx<'_, I>,
        e: &AnimEventRecord,
    ) {
        if !owner.variable_boolean("bCloseWindow") {
            return;
        }
        let window = CLOSE_WINDOW.get(&mut owner.state.params);
        let Some((window, w)) = info(cx, window) else {
            owner.set_var_bool("bCloseWindow", false);
            return;
        };
        let name = e.event.event_name.as_str();
        if name.eq_ignore_ascii_case("WindowAnimLooped") {
            if param_is(e, "start") {
                let o = if w.perma_locked || owner.window_climbed_through(window, &w, cx) {
                    "struggle"
                } else {
                    "success"
                };
                owner.set_var_str("CloseWindowOutcome", o);
                return;
            }
            let stop = owner.variable_string("StopAfterAnimLooped");
            if param_is(e, &stop) {
                owner.set_var_bool("bCloseWindow", false);
            }
        }
        if name.eq_ignore_ascii_case("WindowCloseAttempt") {
            Self::on_attempt_finished(owner, window, &w, cx);
        } else if name.eq_ignore_ascii_case("WindowCloseSuccess") {
            Self::on_success(owner, window, &w, cx);
        }
    }

    fn is_doing_action_that_can_be_cancelled(&self) -> bool {
        true
    }
}

impl<I: 'static> State<I> for SmashWindowState {
    fn id(&self) -> StateId {
        ids::SMASH_WINDOW
    }

    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, false, true, false)
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        owner.set_var_bool("bSmashWindow", true);
        if let Some(item) = owner.items.primary_hand_item() {
            let item = item.borrow();
            if let Some(w) = crate::character::combat::WeaponStats::of(cx.sm, &item) {
                if w.ranged {
                    cx.ctx.event(SimEvent::PlaySound("AttackShove".into()));
                } else {
                    let sound = script_item(cx.sm, &item)
                        .map(|s| {
                            s.borrow()
                                .swing_sound
                                .clone()
                                .unwrap_or_else(|| "BaseballBatSwing".into())
                        })
                        .unwrap_or_else(|| "BaseballBatSwing".into());
                    if !sound.trim().is_empty() {
                        cx.ctx.event(SimEvent::PlaySound(sound));
                    }
                }
            }
        }
        if cx.ctx.is_client() {
            let scratch = owner.no_hand_weapon();
            SMASH_SCRATCHED.set(&mut owner.state.params, scratch);
        }
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let iso = info(cx, SMASH_ISO_WINDOW.get(&mut owner.state.params));
        let vehicle = SMASH_VEHICLE_WINDOW
            .get(&mut owner.state.params)
            .and_then(|(v, p)| cx.world.vehicle_window_destroyed(v, &p).map(|d| (v, d)));
        if iso.is_none() && vehicle.is_none() {
            owner.set_var_bool("bSmashWindow", false);
            return;
        }
        if pressed_movement_or_cancel(owner, cx) {
            owner.set_var_bool("bSmashWindow", false);
            return;
        }
        if !owner.variable_boolean("bSmashWindow") {
            return;
        }
        if let Some((_, w)) = iso {
            if !w.in_world || w.destroyed && !owner.variable_boolean("OwnerSmashedIt") {
                owner.set_var_bool("bSmashWindow", false);
                return;
            }
            face_window(owner, &w);
        } else if let Some((v, destroyed)) = vehicle {
            PathMover::face_this_object(owner, FaceTarget::Vehicle(v));
            if destroyed && !owner.variable_boolean("OwnerSmashedIt") {
                owner.set_var_bool("bSmashWindow", false);
            }
        }
    }

    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = false;
        owner.clear_var("bSmashWindow");
        owner.clear_var("OwnerSmashedIt");
    }

    fn anim_event(
        &self,
        owner: &mut GameCharacter<I>,
        cx: &mut StateCx<'_, I>,
        e: &AnimEventRecord,
    ) {
        let name = e.event.event_name.as_str();
        if let Some((window, _)) = info(cx, SMASH_ISO_WINDOW.get(&mut owner.state.params)) {
            if name.eq_ignore_ascii_case("AttackCollisionCheck") {
                owner.set_var_bool("OwnerSmashedIt", true);
                // `IsoPlayer.getInstance().contextPanic`: the local player.
                owner.player.context_panic = 0.0;
                if !cx.ctx.is_client() {
                    cx.world.window_weapon_hit(window, owner);
                    if owner.no_hand_weapon() {
                        let _ = owner.set_scratched_window(cx.sm, cx.chars, cx.ctx);
                        cx.ctx.event(SimEvent::PlayerVoiceSound("PainFromGlassCut"));
                    }
                }
            } else if name.eq_ignore_ascii_case("ActiveAnimFinishing") {
                owner.set_var_bool("bSmashWindow", false);
                if SMASH_CLIMB_THROUGH_WINDOW.get(&mut owner.state.params) {
                    owner.climb_through_window(window, cx);
                }
            }
        } else if let Some((v, part)) = SMASH_VEHICLE_WINDOW.get(&mut owner.state.params) {
            if cx.world.vehicle_window_destroyed(v, &part).is_none() {
                return;
            }
            if name.eq_ignore_ascii_case("AttackCollisionCheck") {
                owner.set_var_bool("OwnerSmashedIt", true);
                owner.player.context_panic = 0.0;
                if owner.is_local_player(cx.ctx) && !cx.ctx.is_client() {
                    cx.world.vehicle_window_hit(v, &part, owner);
                    if owner.no_hand_weapon() {
                        let _ = owner.set_scratched_window(cx.sm, cx.chars, cx.ctx);
                        cx.ctx.event(SimEvent::PlayerVoiceSound("PainFromGlassCut"));
                    }
                }
            } else if name.eq_ignore_ascii_case("ActiveAnimFinishing") {
                owner.set_var_bool("bSmashWindow", false);
            }
        }
    }

    fn is_doing_action_that_can_be_cancelled(&self) -> bool {
        true
    }

    fn is_processed_on_enter(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::state::{SquareInfo, StateWorld, ThumpKind};
    use crate::character::tests::{fixture, player, Recorder};
    use crate::character::CharacterKind;
    use std::rc::Rc;

    const WIN: ObjectRef = ObjectRef { x: 5, y: 5, z: 0, id: 4 };

    /// A west window on (5, 5); the player stands east of it.
    #[derive(Default)]
    struct Win {
        info: WindowInfo,
        outside: bool,
        toggled: Vec<i32>,
        perma: Vec<bool>,
        hits: usize,
        others: Vec<GameCharacter<()>>,
    }

    impl StateWorld<()> for Win {
        fn character_mut(&mut self, id: i32) -> Option<&mut GameCharacter<()>> {
            self.others.iter_mut().find(|c| c.id == id)
        }
        fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
            Some(SquareInfo { x, y, z, solid_floor: true, ..Default::default() })
        }
        fn characters_on_square(&self, sq: (i32, i32, i32)) -> Vec<i32> {
            self.others.iter().filter(|c| c.path.current_square == Some(sq)).map(|c| c.id).collect()
        }
        fn window_between(&self, _a: (i32, i32, i32), _b: (i32, i32, i32)) -> Option<(ObjectRef, ThumpKind)> {
            Some((WIN, ThumpKind::Window))
        }
        fn square_is_outside(&self, _sq: (i32, i32, i32)) -> bool {
            self.outside
        }
        fn window_info(&self, w: ObjectRef) -> Option<WindowInfo> {
            (w == WIN).then_some(self.info)
        }
        fn toggle_window(&mut self, _w: ObjectRef, chr: i32) {
            self.toggled.push(chr);
            self.info.open = !self.info.open;
        }
        fn set_window_perma_locked(&mut self, _w: ObjectRef, perma: bool) {
            self.perma.push(perma);
            self.info.perma_locked = perma;
        }
        fn window_weapon_hit(&mut self, _w: ObjectRef, _chr: &GameCharacter<()>) {
            self.hits += 1;
            self.info.destroyed = true;
        }
    }

    fn world() -> Win {
        Win { info: WindowInfo { square: (5, 5, 0), north: false, in_world: true, ..Default::default() }, ..Default::default() }
    }

    fn ev(name: &str, param: &str) -> AnimEventRecord {
        AnimEventRecord {
            state_name: None,
            layer_depth: -1,
            track: None,
            event: Rc::new(pz_anim::AnimEvent { event_name: name.into(), parameter_value: Some(param.into()), ..Default::default() }),
        }
    }

    fn at_window(p: &mut GameCharacter<()>) {
        (p.vars.x, p.vars.y) = (5.6, 5.5);
        p.path.current_square = Some((5, 5, 0));
        p.id = 1;
    }

    #[test]
    fn opening_an_unlocked_window_toggles_it_on_success() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = world();
        let mut p = player(&sm, &mut chars);
        at_window(&mut p);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        p.open_window(WIN, &cx);
        assert_eq!(OPEN_WINDOW.peek(&p.state.params), Some(Some(WIN)));
        let s = open_window::<()>();
        s.enter(&mut p, &mut cx);
        // East of a west window: facing west.
        assert_eq!(p.vars.dir, IsoDirections::W);
        assert!(p.variable_boolean("bOpenWindow") && p.vars.ignore_movement);
        s.execute(&mut p, &mut cx);
        assert!(p.variable_boolean("bOpenWindow"));
        s.anim_event(&mut p, &mut cx, &ev("WindowAnimLooped", "start"));
        assert_eq!(p.variable_string("OpenWindowOutcome"), "success");
        s.anim_event(&mut p, &mut cx, &ev("WindowOpenAttempt", ""));
        s.anim_event(&mut p, &mut cx, &ev("WindowOpenSuccess", ""));
        assert_eq!(p.variable_string("StopAfterAnimLooped"), "success");
        // The loop that matches StopAfterAnimLooped ends the state.
        s.anim_event(&mut p, &mut cx, &ev("WindowAnimLooped", "success"));
        assert!(!p.variable_boolean("bOpenWindow"));
        s.exit(&mut p, &mut cx);
        assert!(!p.vars.ignore_movement);
        let _ = cx;
        assert_eq!(w.toggled, [1]);
        assert!(w.info.open);
    }

    #[test]
    fn a_locked_window_from_outside_struggles_and_may_break_its_lock() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = world();
        w.info.locked = true;
        w.outside = true;
        let mut p = player(&sm, &mut chars);
        at_window(&mut p);
        for seed in 0..40 {
            pz_core::random::set_seed(seed);
            let r = pz_core::random::Rand::new(seed);
            w.info.perma_locked = false;
            w.perma.clear();
            p.clear_var("OpenWindowOutcome");
            let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
            p.open_window(WIN, &cx);
            let s = open_window::<()>();
            s.enter(&mut p, &mut cx);
            s.anim_event(&mut p, &mut cx, &ev("WindowAnimLooped", "start"));
            assert_eq!(p.variable_string("OpenWindowOutcome"), "struggle");
            s.anim_event(&mut p, &mut cx, &ev("WindowOpenAttempt", ""));
            let _ = cx;
            // Strength 0: one perma-lock draw (1 in 10), then one success
            // draw (`<= 1` of 100).
            if r.next(100) < 10 {
                assert_eq!(w.perma, [true], "seed {seed}");
                assert_eq!(p.variable_string("OpenWindowOutcome"), "fail");
                assert_eq!(OPEN_WINDOW.peek(&p.state.params), Some(None));
            } else {
                let ok = r.next(100) <= 1;
                assert!(w.perma.is_empty());
                assert_eq!(p.variable_string("OpenWindowOutcome"), if ok { "success" } else { "struggle" }, "seed {seed}");
            }
            assert_eq!(pz_core::random::next(1 << 30), r.next(1 << 30), "seed {seed}: draw count");
        }
    }

    #[test]
    fn panic_while_opening_smashes_the_window_and_climbs_through() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = world();
        let mut p = player(&sm, &mut chars);
        at_window(&mut p);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        p.open_window(WIN, &cx);
        let s = open_window::<()>();
        s.enter(&mut p, &mut cx);
        p.player.context_panic = 6.0;
        s.execute(&mut p, &mut cx);
        assert_eq!(p.player.context_panic, 0.0);
        assert!(!p.variable_boolean("bOpenWindow"));
        assert_eq!(SMASH_ISO_WINDOW.peek(&p.state.params), Some(Some(WIN)));
        assert!(SMASH_CLIMB_THROUGH_WINDOW.get(&mut p.state.params));
        // The smash: bare hands scratch and hit the window once.
        let sw = smash_window::<()>();
        sw.enter(&mut p, &mut cx);
        sw.execute(&mut p, &mut cx);
        assert!(p.variable_boolean("bSmashWindow"));
        sw.anim_event(&mut p, &mut cx, &ev("AttackCollisionCheck", ""));
        assert!(p.variable_boolean("OwnerSmashedIt"));
        // Destroyed, but the owner smashed it: the state goes on.
        sw.execute(&mut p, &mut cx);
        assert!(p.variable_boolean("bSmashWindow"));
        sw.anim_event(&mut p, &mut cx, &ev("ActiveAnimFinishing", ""));
        assert!(!p.variable_boolean("bSmashWindow"));
        sw.exit(&mut p, &mut cx);
        let _ = cx;
        assert_eq!(w.hits, 1);
        assert!(ctx.events.borrow().iter().any(|e| e.contains("PainFromGlassCut")));
    }

    #[test]
    fn closing_struggles_while_someone_climbs_through() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = world();
        w.info.open = true;
        let mut z = GameCharacter::new(&sm, CharacterKind::Zombie);
        z.id = 9;
        z.path.current_square = Some((4, 5, 0));
        crate::character::states::climb_window::PARAMS.set(
            &mut z.state.params,
            Some(crate::character::states::climb_window::Positioning {
                window: WIN,
                portal: ThumpKind::Window,
                can_climb: true,
                climb_dir: IsoDirections::E,
                start_x: 4,
                start_y: 5,
                z: 0,
                opposite_x: 5,
                opposite_y: 5,
                end_x: 5,
                end_y: 5,
                scratch: false,
                is_counter: false,
                is_floor: true,
                is_sheet_rope: false,
            }),
        );
        {
            let mut empty = Win::default();
            let mut cx = StateCx::new(&sm, &chars, &ctx, &mut empty);
            z.change_state(Some(crate::character::states::climb_window::climb_through_window()), &mut cx);
        }
        z.set_var_bool("BlockWindow", true);
        w.others.push(z);
        let mut p = player(&sm, &mut chars);
        at_window(&mut p);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        p.close_window(WIN, &cx);
        let s = close_window::<()>();
        s.enter(&mut p, &mut cx);
        s.anim_event(&mut p, &mut cx, &ev("WindowAnimLooped", "start"));
        assert_eq!(p.variable_string("CloseWindowOutcome"), "struggle");
        // Nobody climbing: success, and the window shuts.
        cx.world.character_mut(9).unwrap().clear_var("BlockWindow");
        s.anim_event(&mut p, &mut cx, &ev("WindowCloseAttempt", ""));
        assert_eq!(p.variable_string("CloseWindowOutcome"), "success");
        s.anim_event(&mut p, &mut cx, &ev("WindowCloseSuccess", ""));
        let _ = cx;
        assert!(!w.info.open);
    }
}
