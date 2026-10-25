//! `ClimbSheetRopeState` and `ClimbDownSheetRopeState`: climbing up or
//! down a sheet rope, with the climb speed, the chance to fall off each
//! frame, the arm strain, and what happens at the top (through a window
//! or frame, over a fence, or back down when the way is blocked).
//!
//! The squares are the world's ([`StateWorld::sheet_rope_square`],
//! [`StateWorld::chunk_levels`], [`StateWorld::sheet_rope_exit`]). Not
//! here: the singletons' `numberOfFallingChecks` (a debug print) and the
//! multiplayer paths (`processOnEnter` / `processOnExit`).
//!
//! [`StateWorld::sheet_rope_square`]: crate::character::state::StateWorld::sheet_rope_square
//! [`StateWorld::chunk_levels`]: crate::character::state::StateWorld::chunk_levels
//! [`StateWorld::sheet_rope_exit`]: crate::character::state::StateWorld::sheet_rope_exit

use pz_world::square::IsoDirections;

use crate::character::state::{ClimbData, Param, SheetRopeExit, SheetRopeSquare, State, StateCx, StateId, StateRef, SyncFlags};
use crate::character::zombie::math::fastfloor;
use crate::character::GameCharacter;

use super::ids;

/// `ClimbSheetRopeState.SPEED`, `CLIMB`.
pub const SPEED: Param<f32> = Param::new(ids::CLIMB_SHEET_ROPE, "speed", || 0.0);
pub const CLIMB: Param<bool> = Param::new(ids::CLIMB_SHEET_ROPE, "climb", || false);
/// `ClimbDownSheetRopeState.SPEED`, `CLIMB`.
pub const DOWN_SPEED: Param<f32> = Param::new(ids::CLIMB_DOWN_SHEET_ROPE, "speed", || 0.0);
pub const DOWN_CLIMB: Param<bool> = Param::new(ids::CLIMB_DOWN_SHEET_ROPE, "climb", || false);

/// `ClimbSheetRopeState.FallChanceScale`.
const FALL_CHANCE_SCALE: f32 = 100.0;
/// `ClimbDownSheetRopeState.ClimbDownFallChanceScale`.
const CLIMB_DOWN_FALL_CHANCE_SCALE: f32 = 300.0;

/// `ClimbSheetRopeState`.
pub struct ClimbSheetRopeState;
/// `ClimbDownSheetRopeState`.
pub struct ClimbDownSheetRopeState;

/// The singleton (`ClimbSheetRopeState.instance()`).
pub fn climb_sheet_rope<I: 'static>() -> StateRef<I> {
    static S: ClimbSheetRopeState = ClimbSheetRopeState;
    &S
}

/// The singleton (`ClimbDownSheetRopeState.instance()`).
pub fn climb_down_sheet_rope<I: 'static>() -> StateRef<I> {
    static S: ClimbDownSheetRopeState = ClimbDownSheetRopeState;
    &S
}

/// `createClimbData(chr)`.
pub fn create_climb_data<I: 'static>(chr: &mut GameCharacter<I>) {
    if chr.vars.climb_data.is_none() {
        chr.vars.climb_data = Some(ClimbData::default());
    }
}

fn climb_data<I: 'static>(chr: &mut GameCharacter<I>) -> &mut ClimbData {
    chr.vars.climb_data.get_or_insert_with(ClimbData::default)
}

/// The sheet-rope view of `sq`.
fn rope_at<I: 'static>(cx: &StateCx<'_, I>, sq: (i32, i32, i32)) -> Option<SheetRopeSquare> {
    cx.world.sheet_rope_square(sq)
}

/// The current square's sheet-rope view (`None` without a square).
fn rope_here<I: 'static>(chr: &GameCharacter<I>, cx: &StateCx<'_, I>) -> Option<SheetRopeSquare> {
    chr.path.current_square.and_then(|p| rope_at(cx, p))
}

/// `setIdealDirection(chr)`: faces the rope of the current square and
/// picks the spot on the square to climb from.
pub fn set_ideal_direction<I: 'static>(chr: &mut GameCharacter<I>, cx: &StateCx<'_, I>) {
    let Some(sq) = rope_here(chr, cx) else { return };
    let set = |c: &mut GameCharacter<I>, dir: IsoDirections, x: f32, y: f32| {
        c.set_forward_iso_direction(dir);
        let d = climb_data(c);
        d.idealx = x;
        d.idealy = y;
    };
    if sq.n {
        set(chr, IsoDirections::N, 0.54, 0.39);
    }
    if sq.s {
        set(chr, IsoDirections::S, 0.118, 0.5756);
    }
    if sq.w {
        set(chr, IsoDirections::W, 0.4, 0.7);
    }
    if sq.e {
        set(chr, IsoDirections::E, 0.5417, 0.3144);
    }
}

/// `applyIdealDirection(chr)`: a quarter of the way to the spot on the
/// square.
pub fn apply_ideal_direction<I: 'static>(chr: &mut GameCharacter<I>) {
    let (ix, iy) = {
        let d = climb_data(chr);
        (d.idealx, d.idealy)
    };
    let mut ox = chr.vars.x - fastfloor(chr.vars.x) as f32;
    let mut oy = chr.vars.y - fastfloor(chr.vars.y) as f32;
    if ox != ix {
        let dif = (ix - ox) / 4.0;
        ox += dif;
        chr.set_x(fastfloor(chr.vars.x) as f32 + ox);
    }
    if oy != iy {
        let dif = (iy - oy) / 4.0;
        oy += dif;
        chr.set_y(fastfloor(chr.vars.y) as f32 + oy);
    }
    chr.vars.next_x = chr.vars.x;
    chr.vars.next_y = chr.vars.y;
}

/// `fallChanceCalculation(chr)` of either state (`scale` 100 up, 300
/// down): the time on the rope grows by the frame.
fn fall_chance_calculation<I: 'static>(chr: &mut GameCharacter<I>, scale: f32, cx: &mut StateCx<'_, I>) -> f32 {
    let mut fall = chr.climbing_fail_chance_float(cx) + 1.0;
    let m = cx.ctx.multiplier();
    chr.vars.climb_rope_time += m;
    fall *= scale;
    fall /= if m < 1.0 { 1.0 } else { m as i32 as f32 };
    climb_data(chr).fall_chance = fall;
    fall
}

/// `calculateClimb(chr)`: the top of the rope above, and what is beyond
/// it.
fn calculate_climb<I: 'static>(chr: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
    set_ideal_direction(chr, cx);
    apply_ideal_direction(chr);
    // getCurrentSquare().getChunk().getMaxLevel(): the original
    // dereferences the square.
    if let Some(max_level) = chr.path.current_square.filter(|&p| rope_at(cx, p).is_some()).map(|p| cx.world.chunk_levels(p).1) {
        let (x, y) = (fastfloor(chr.vars.x), fastfloor(chr.vars.y));
        let mut z = fastfloor(chr.vars.z);
        while z <= max_level {
            if rope_at(cx, (x, y, z)).is_some_and(|s| s.top) {
                let dir = chr.forward_iso_direction();
                let exit = cx.world.sheet_rope_exit(&*chr, (x, y, z), dir);
                let d = climb_data(chr);
                d.target_square = Some((x, y, z));
                d.target_climb_height = z;
                // calculateClimbOutcome sets climbTargetIsoObject only for
                // a window or a frame.
                if let SheetRopeExit::OpenWindow(o) | SheetRopeExit::WindowFrame(o) = exit {
                    d.climb_target = Some(o);
                }
                d.exit_blocked = Some(exit);
                break;
            }
            z += 1;
        }
    }
    fall_chance_calculation(chr, FALL_CHANCE_SCALE, cx);
}

/// `calculateClimbDown(chr)`: the first floor below.
fn calculate_climb_down<I: 'static>(chr: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
    set_ideal_direction(chr, cx);
    apply_ideal_direction(chr);
    if let Some(min_level) = chr.path.current_square.filter(|&p| rope_at(cx, p).is_some()).map(|p| cx.world.chunk_levels(p).0) {
        let (x, y) = (fastfloor(chr.vars.x), fastfloor(chr.vars.y));
        let mut z = fastfloor(chr.vars.z);
        while z >= min_level {
            // The original dereferences a missing square.
            if rope_at(cx, (x, y, z)).is_some_and(|s| s.solidtrans || s.solid_floor) {
                let d = climb_data(chr);
                d.target_square = Some((x, y, z));
                d.target_climb_height = z;
                break;
            }
            z -= 1;
        }
    }
    fall_chance_calculation(chr, CLIMB_DOWN_FALL_CHANCE_SCALE, cx);
}

/// The shared part of both `execute`s after the height: the fall check,
/// the arm strain (`strain` 0.02 up, 0.007 down) and the grid stack.
fn rope_frame<I: 'static>(chr: &mut GameCharacter<I>, scale: f32, strain: f64, cx: &mut StateCx<'_, I>) {
    fall_chance_calculation(chr, scale, cx);
    let fall_chance = climb_data(chr).fall_chance;
    let can_fall_check = chr.vars.climb_rope_time > fall_chance * 10.0;
    // Rand.NextBool((int)fallChance)
    let fall = cx.ctx.rand_next(fall_chance as i32) == 0;
    let here = rope_here(chr, cx).is_some_and(|s| s.sheet_rope);
    if !here {
        chr.set_collidable(true);
        chr.vars.climbing = false;
        chr.vars.falling = true;
        chr.clear_var("ClimbRope");
    } else if can_fall_check && !cx.ctx.easy_climbing() && fall && !chr.vars.remote {
        chr.fall_from_rope();
    }
    let perk = |c: &GameCharacter<I>, id: &str| c.perk_level(cx.chars.perks.constant(id).as_ref());
    let skill = (perk(chr, "Nimble") + perk(chr, "Strength").max(perk(chr, "Fitness")) * 2) as f32 / 3.0;
    let m = cx.ctx.multiplier();
    let heavy = chr.moodles.moodle_level_of(cx.sm, "HEAVY_LOAD").unwrap_or(0);
    let pain = (strain * m as f64 * (heavy + 1) as f64) as f32 * ((15.0 - skill) / 10.0) * (m / 0.8);
    chr.add_both_arm_muscle_strain(cx.chars, cx.ctx, pain);
    if chr.is_local_player(cx.ctx) {
        chr.player.dirty_recalc_grid_stack_time = 2.0;
    }
}

fn enter_rope<I: 'static>(chr: &mut GameCharacter<I>) {
    chr.vars.ignore_movement = true;
    chr.vars.hide_weapon_model = true;
    chr.vars.climbing = true;
    chr.set_var_bool("ClimbRope", true);
}

fn exit_rope<I: 'static>(chr: &mut GameCharacter<I>) {
    chr.set_collidable(true);
    chr.vars.ignore_movement = false;
    chr.vars.hide_weapon_model = false;
    chr.vars.climbing = false;
    chr.clear_var("ClimbRope");
}

impl ClimbSheetRopeState {
    /// `setParams(chr, stage)`: a local climber's speed and climbing flag.
    fn set_params<I: 'static>(chr: &mut GameCharacter<I>, cx: &StateCx<'_, I>) {
        if !chr.vars.remote {
            let speed = chr.climb_rope_speed(cx.sm, cx.chars, false);
            SPEED.set(&mut chr.state.params, speed);
            let climbing = chr.vars.climbing;
            CLIMB.set(&mut chr.state.params, climbing);
        }
    }

    /// `finishClimbing(chr)`: on the top square, then through or over
    /// what is beyond, or back down.
    fn finish_climbing<I: 'static>(chr: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let d = *climb_data(chr);
        chr.set_z(d.target_climb_height as f32);
        chr.path.current_square = d.target_square;
        chr.set_collidable(true);
        // The original switches on exitBlocked.ordinal() (null without a
        // top).
        match d.exit_blocked {
            Some(SheetRopeExit::Blocked) => chr.climb_down_sheet_rope(cx),
            Some(SheetRopeExit::OpenWindow(_)) => {
                if let Some(w) = d.climb_target {
                    chr.climb_through_window(w, cx);
                }
            }
            Some(SheetRopeExit::WindowFrame(_)) => {
                if let Some(f) = d.climb_target {
                    chr.climb_through_window_frame(f, cx);
                }
            }
            Some(SheetRopeExit::Fence) => {
                let dir = chr.forward_iso_direction();
                chr.do_climb_over_fence(dir, cx);
            }
            _ => {}
        }
    }
}

impl<I: 'static> State<I> for ClimbSheetRopeState {
    fn id(&self) -> StateId {
        ids::CLIMB_SHEET_ROPE
    }

    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, true, true)
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        enter_rope(owner);
        Self::set_params(owner, cx);
        create_climb_data(owner);
        calculate_climb(owner, cx);
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        apply_ideal_direction(owner);
        let mut speed = owner.climb_rope_speed(cx.sm, cx.chars, false);
        if owner.vars.remote {
            speed = SPEED.peek(&owner.state.params).unwrap_or(speed);
        }
        // getSpriteDef().animFrameIncrease
        owner.zombie.anim_frame_increase = speed;
        let height = owner.vars.z + speed / 10.0 * cx.ctx.multiplier();
        owner.set_z(height);
        let (fall_height, climb_height) = {
            let d = climb_data(owner);
            (d.target_fall_height, d.target_climb_height)
        };
        // The original's negated comparison (true for a NaN height).
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let below_fall = !(height >= fall_height);
        if below_fall && height > climb_height as f32 {
            Self::finish_climbing(owner, cx);
            return;
        }
        rope_frame(owner, FALL_CHANCE_SCALE, 0.02, cx);
    }

    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        exit_rope(owner);
        Self::set_params(owner, cx);
    }

    fn is_processed_on_enter(&self) -> bool {
        true
    }

    fn is_processed_on_exit(&self) -> bool {
        true
    }
}

impl ClimbDownSheetRopeState {
    fn set_params<I: 'static>(chr: &mut GameCharacter<I>, cx: &StateCx<'_, I>) {
        if !chr.vars.remote {
            let speed = chr.climb_rope_speed(cx.sm, cx.chars, true);
            DOWN_SPEED.set(&mut chr.state.params, speed);
            let climbing = chr.vars.climbing;
            DOWN_CLIMB.set(&mut chr.state.params, climbing);
        }
    }

    /// `finishClimbing(chr)`: on the floor below.
    fn finish_climbing<I: 'static>(chr: &mut GameCharacter<I>) {
        let h = climb_data(chr).target_climb_height;
        chr.set_z(h as f32);
        chr.state.params.clear(ids::CLIMB_DOWN_SHEET_ROPE);
        chr.clear_var("ClimbRope");
        chr.set_collidable(true);
    }
}

impl<I: 'static> State<I> for ClimbDownSheetRopeState {
    fn id(&self) -> StateId {
        ids::CLIMB_DOWN_SHEET_ROPE
    }

    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, true, true)
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        enter_rope(owner);
        Self::set_params(owner, cx);
        create_climb_data(owner);
        calculate_climb_down(owner, cx);
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        apply_ideal_direction(owner);
        let mut speed = owner.climb_rope_speed(cx.sm, cx.chars, true);
        if owner.vars.remote {
            speed = DOWN_SPEED.peek(&owner.state.params).unwrap_or(speed);
        }
        owner.zombie.anim_frame_increase = speed;
        // getCurrentSquare().getChunk().getMinLevel()
        let min_level = owner.path.current_square.filter(|&p| rope_at(cx, p).is_some()).map_or(0, |p| cx.world.chunk_levels(p).0);
        let height = (owner.vars.z - speed / 10.0 * cx.ctx.multiplier()).max(min_level as f32);
        owner.set_z(height);
        if height <= climb_data(owner).target_climb_height as f32 {
            Self::finish_climbing(owner);
            return;
        }
        rope_frame(owner, CLIMB_DOWN_FALL_CHANCE_SCALE, 0.007, cx);
    }

    fn exit(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        exit_rope(owner);
        Self::set_params(owner, cx);
    }

    fn is_processed_on_enter(&self) -> bool {
        true
    }

    fn is_processed_on_exit(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::character::state::StateWorld;
    use crate::character::tests::{fixture, player, Recorder};
    use pz_anim::AnimationVariableSource;

    /// A north rope on (4, 5) from a floor on z 0 to its top on z 2, in a
    /// chunk of levels 0 to 2; beyond the top is `beyond`.
    struct Rope {
        rope: HashMap<(i32, i32, i32), SheetRopeSquare>,
        beyond: SheetRopeExit,
        asked: Vec<((i32, i32, i32), IsoDirections)>,
    }

    impl Default for Rope {
        fn default() -> Self {
            let r = SheetRopeSquare { sheet_rope: true, can_climb_here: true, n: true, ..Default::default() };
            Rope {
                rope: HashMap::from([
                    ((4, 5, 0), SheetRopeSquare { solid_floor: true, ..r }),
                    ((4, 5, 1), r),
                    ((4, 5, 2), SheetRopeSquare { top: true, ..r }),
                ]),
                beyond: SheetRopeExit::Blocked,
                asked: Vec::new(),
            }
        }
    }

    impl StateWorld<()> for Rope {
        fn sheet_rope_square(&self, sq: (i32, i32, i32)) -> Option<SheetRopeSquare> {
            self.rope.get(&sq).copied()
        }
        fn chunk_levels(&self, _sq: (i32, i32, i32)) -> (i32, i32) {
            (0, 2)
        }
        fn sheet_rope_exit(&mut self, _chr: &GameCharacter<()>, target: (i32, i32, i32), dir: IsoDirections) -> SheetRopeExit {
            self.asked.push((target, dir));
            self.beyond
        }
    }

    fn climber(sm: &pz_scripts::ScriptManager, chars: &mut crate::Characters, z: i32) -> GameCharacter<()> {
        let mut c = player(sm, chars);
        c.path.current_square = Some((4, 5, z));
        (c.vars.x, c.vars.y, c.vars.z) = (4.1, 5.9, z as f32);
        c
    }

    #[test]
    fn entering_the_climb_finds_the_top_and_faces_the_rope() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Rope::default();
        let mut c = climber(&sm, &mut chars, 0);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.change_state(Some(climb_sheet_rope()), &mut cx);
        let _ = cx;
        assert!(c.vars.climbing && c.get_variable_boolean("ClimbRope"));
        assert_eq!(c.forward_iso_direction(), IsoDirections::N);
        // A quarter of the way to (0.54, 0.39) in the square.
        assert!((c.vars.x - (4.1 + (0.54 - 0.1) / 4.0)).abs() < 1e-5);
        assert!((c.vars.y - (5.9 + (0.39 - 0.9) / 4.0)).abs() < 1e-5);
        let d = c.vars.climb_data.unwrap();
        assert_eq!((d.target_square, d.target_climb_height, d.exit_blocked), (Some((4, 5, 2)), 2, Some(SheetRopeExit::Blocked)));
        assert_eq!(w.asked, [((4, 5, 2), IsoDirections::N)]);
        // No skills: (0 + 1) * 100 over a multiplier of 1; one frame on the
        // rope so far.
        assert_eq!((d.fall_chance, c.vars.climb_rope_time), (100.0, 1.0));
        assert_eq!(SPEED.peek(&c.state.params), Some(c.climb_rope_speed(&sm, &chars, false)));
    }

    #[test]
    fn a_blocked_top_sends_the_climber_back_down() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Rope::default();
        let mut c = climber(&sm, &mut chars, 0);
        let s = climb_sheet_rope::<()>();
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.change_state(Some(s), &mut cx);
        // A frame below the top climbs a little.
        s.execute(&mut c, &mut cx);
        assert!(c.vars.z > 0.0 && c.vars.z < 0.01);
        assert!(c.vars.climbing);
        // Past the top: on the top square, then back down the rope.
        c.vars.z = 2.0;
        s.execute(&mut c, &mut cx);
        let _ = cx;
        assert_eq!((c.vars.z, c.path.current_square), (2.0, Some((4, 5, 2))));
        assert!(c.state.action_context.has_event_occurred("EventClimbDownRope", None));
    }

    #[test]
    fn climbing_off_the_rope_starts_a_fall() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Rope::default();
        let mut c = climber(&sm, &mut chars, 0);
        let s = climb_sheet_rope::<()>();
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.change_state(Some(s), &mut cx);
        // The square under the climber has no rope any more.
        c.path.current_square = Some((6, 6, 0));
        s.execute(&mut c, &mut cx);
        assert!(!c.vars.climbing && c.vars.falling && !c.get_variable_boolean("ClimbRope"));
        s.exit(&mut c, &mut cx);
        assert!(c.path.collidable && !c.vars.ignore_movement);
    }

    #[test]
    fn climbing_down_stops_on_the_floor() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Rope::default();
        let mut c = climber(&sm, &mut chars, 2);
        let s = climb_down_sheet_rope::<()>();
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.change_state(Some(s), &mut cx);
        let d = c.vars.climb_data.unwrap();
        assert_eq!((d.target_square, d.target_climb_height), (Some((4, 5, 0)), 0));
        // (0 + 1) * 300 going down.
        assert_eq!(d.fall_chance, 300.0);
        s.execute(&mut c, &mut cx);
        assert!(c.vars.z < 2.0 && c.get_variable_boolean("ClimbRope"));
        c.vars.z = 0.001;
        s.execute(&mut c, &mut cx);
        let _ = cx;
        assert_eq!(c.vars.z, 0.0);
        assert!(!c.get_variable_boolean("ClimbRope") && c.path.collidable);
    }
}
