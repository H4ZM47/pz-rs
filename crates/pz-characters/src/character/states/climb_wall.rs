//! `ClimbOverWallState`: a player pulling itself over a tall wall, with
//! the struggle and success rolls of `setParams`, the endurance it costs,
//! the arm strain of each frame and the wall's sound.
//!
//! Not here: `setAnimated(true)` (the legs sprite's `animate`, the
//! renderer's) and the multiplayer paths (`processOnEnter` /
//! `processOnExit`, a remote player's `NetworkCharacterAI`).

use pz_anim::{AnimEventRecord, AnimationVariableSource};
use pz_world::square::IsoDirections;

use crate::character::state::{Param, State, StateCx, StateId, StateRef, SyncFlags};
use crate::character::{GameCharacter, SimEvent};

use super::ids;

pub const START_X: Param<i32> = Param::new(ids::CLIMB_OVER_WALL, "start_x", || 0);
pub const START_Y: Param<i32> = Param::new(ids::CLIMB_OVER_WALL, "start_y", || 0);
pub const Z: Param<i32> = Param::new(ids::CLIMB_OVER_WALL, "z", || 0);
pub const END_X: Param<i32> = Param::new(ids::CLIMB_OVER_WALL, "end_x", || 0);
pub const END_Y: Param<i32> = Param::new(ids::CLIMB_OVER_WALL, "end_y", || 0);
pub const DIR: Param<Option<IsoDirections>> = Param::new(ids::CLIMB_OVER_WALL, "dir", || None);
pub const STRUGGLE: Param<bool> = Param::new(ids::CLIMB_OVER_WALL, "struggle", || false);
pub const SUCCESS: Param<bool> = Param::new(ids::CLIMB_OVER_WALL, "success", || false);
pub const STRAIN: Param<f32> = Param::new(ids::CLIMB_OVER_WALL, "strain", || 0.0);

/// `ClimbOverWallState`.
pub struct ClimbOverWallState;

/// The singleton (`instance()`).
pub fn climb_over_wall<I: 'static>() -> StateRef<I> {
    static S: ClimbOverWallState = ClimbOverWallState;
    &S
}

/// `ClimbOverWallState.setParams(owner, dir)`: the squares of the climb
/// and, for a local player, the struggle and success rolls (none with
/// easy climbing).
pub fn set_params<I: 'static>(owner: &mut GameCharacter<I>, dir: IsoDirections, cx: &mut StateCx<'_, I>) {
    // The original reads getSquare() without a null check.
    let Some((x, y, z)) = owner.path.current_square else { return };
    let (mut end_x, mut end_y) = (x, y);
    match dir {
        IsoDirections::N => end_y -= 1,
        IsoDirections::S => end_y += 1,
        IsoDirections::W => end_x -= 1,
        IsoDirections::E => end_x += 1,
        // The original throws IllegalArgumentException("invalid direction").
        _ => return,
    }
    let p = &mut owner.state.params;
    START_X.set(p, x);
    START_Y.set(p, y);
    Z.set(p, z);
    END_X.set(p, end_x);
    END_Y.set(p, end_y);
    DIR.set(p, Some(dir));
    if owner.is_local_player(cx.ctx) {
        if cx.ctx.easy_climbing() {
            owner.player.climb_over_wall_struggle = false;
            owner.player.climb_over_wall_success = true;
        } else {
            let struggle_chance = owner.climbing_fail_chance_int(cx);
            // Rand.NextBool(n): Next(n) == 0 (always for n <= 0).
            let mut struggle = cx.ctx.rand_next(struggle_chance / 2) == 0;
            if cx.ctx.game_mode() == "Tutorial" {
                struggle = false;
            }
            let mut success = false;
            if struggle_chance > 0 {
                success = cx.ctx.rand_next(struggle_chance) != 0;
            } else if owner.moodles.moodle_level_of(cx.sm, "HEAVY_LOAD").unwrap_or(0) == 0 {
                let chance = owner.perk_level(cx.chars.perks.constant("Strength").as_ref()).max(1);
                success = cx.ctx.rand_next(100) <= chance;
            }
            owner.player.climb_over_wall_struggle = struggle;
            owner.player.climb_over_wall_success = success;
        }
    }
    let (struggle, success) = (owner.player.climb_over_wall_struggle, owner.player.climb_over_wall_success);
    STRUGGLE.set(&mut owner.state.params, struggle);
    SUCCESS.set(&mut owner.state.params, success);
}

impl<I: 'static> State<I> for ClimbOverWallState {
    fn id(&self) -> StateId {
        ids::CLIMB_OVER_WALL
    }

    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, true, false)
    }

    fn enter(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        owner.vars.ignore_movement = true;
        owner.vars.hide_weapon_model = true;
        let endurance = cx.chars.sim.character_stats.field("ENDURANCE").ok();
        let reduce = cx.ctx.zomboid_globals().running_endurance_reduce;
        if let Some(e) = &endurance {
            owner.stats.remove(e, (reduce * 1200.0) as f32);
        }
        // The original casts the owner to IsoPlayer.
        let struggle = owner.player.climb_over_wall_struggle;
        if struggle {
            if let Some(e) = &endurance {
                owner.stats.remove(e, (reduce * 500.0) as f32);
            }
        }
        let success = owner.player.climb_over_wall_success;
        owner.set_var_bool("ClimbFenceFinished", false);
        owner.set_var_bool("ClimbFenceStarted", false);
        if !owner.vars.remote {
            owner.set_var_str("ClimbFenceOutcome", if success { "success" } else { "fail" });
            owner.set_var_bool("ClimbFenceStruggle", struggle);
        } else {
            let s = SUCCESS.get(&mut owner.state.params);
            owner.set_var_str("ClimbFenceOutcome", if s { "success" } else { "fail" });
            let st = STRUGGLE.get(&mut owner.state.params);
            owner.set_var_bool("ClimbFenceStruggle", st);
        }
        if owner.is_local_player(cx.ctx) {
            cx.ctx.event(SimEvent::MusicIntensityEvent("ClimbWall"));
        }
        if !success {
            // getPathFindBehavior2() is never null for a character.
            cx.world.path_reset(&mut *owner);
            cx.world.path_cancel(&mut *owner);
            owner.path.path2 = None;
        }
        STRAIN.set(&mut owner.state.params, 0.0);
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        let dir = DIR.get(&mut owner.state.params);
        // setAnimated(true): the legs sprite's, the renderer's.
        // setDir(dir) is setForwardIsoDirection(dir); the original passes
        // a null dir on and throws at the switch below.
        let Some(dir) = dir else { return };
        owner.set_forward_iso_direction(dir);
        let perk = |c: &GameCharacter<I>, id: &str| c.perk_level(cx.chars.perks.constant(id).as_ref());
        let skill = (perk(owner, "Nimble") + perk(owner, "Strength") * 2) as f32 / 3.0;
        let m = cx.ctx.multiplier();
        let heavy = owner.moodles.moodle_level_of(cx.sm, "HEAVY_LOAD").unwrap_or(0);
        let pain = (0.02 * m as f64 * (heavy + 1) as f64) as f32 * ((15.0 - skill) / 10.0) * (m / 0.8);
        owner.add_both_arm_muscle_strain(cx.chars, cx.ctx, pain);
        let strain = STRAIN.get(&mut owner.state.params);
        STRAIN.set(&mut owner.state.params, strain + pain);
        if !owner.get_variable_boolean("ClimbFenceStarted") {
            let p = &mut owner.state.params;
            let (start_x, start_y) = (START_X.get(p), START_Y.get(p));
            let (mut ideal_x, mut ideal_y) = (owner.vars.x, owner.vars.y);
            match dir {
                IsoDirections::N => ideal_y = start_y as f32 + 0.15,
                IsoDirections::S => ideal_y = (start_y + 1) as f32 - 0.15,
                IsoDirections::W => ideal_x = start_x as f32 + 0.15,
                IsoDirections::E => ideal_x = (start_x + 1) as f32 - 0.15,
                _ => {}
            }
            let mult = cx.ctx.thirty_fps_multiplier() / 8.0;
            owner.set_x(owner.vars.x + (ideal_x - owner.vars.x) * mult);
            owner.set_y(owner.vars.y + (ideal_y - owner.vars.y) * mult);
        }
    }

    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        for v in ["ClimbingFence", "ClimbFenceFinished", "ClimbFenceOutcome", "ClimbFenceStarted", "ClimbFenceStruggle", "PlayerVoiceSound"] {
            owner.clear_var(v);
        }
        owner.vars.ignore_movement = false;
        owner.vars.hide_weapon_model = false;
        // A zombie's getNetworkCharacterAI().isClimbing: multiplayer.
    }

    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, e: &AnimEventRecord) {
        let name = e.event.event_name.as_str();
        let param = e.event.parameter_value.as_deref().unwrap_or("");
        if name.eq_ignore_ascii_case("PlayFenceSound") {
            let p = &owner.state.params;
            let g = |k: Param<i32>| k.peek(p).unwrap_or(0);
            let z = g(Z);
            let (start, end) = ((g(START_X), g(START_Y), z), (g(END_X), g(END_Y), z));
            let Some(dir) = DIR.peek(p).flatten() else { return };
            if cx.world.square(start.0, start.1, start.2).is_none() || cx.world.square(end.0, end.1, end.2).is_none() {
                return;
            }
            let Some(fence_type) = cx.world.climb_wall_fence_type(start, (end.0, end.1), dir) else { return };
            cx.ctx.event(SimEvent::ObstacleSound { name: param.to_string(), parameter: "FenceTypeHigh", value: fence_type as f32, movement_speed: false });
        }
        if name.eq_ignore_ascii_case("PlayerVoiceSound") {
            if owner.get_variable_boolean("PlayerVoiceSound") || !owner.is_player() {
                return;
            }
            owner.set_var_bool("PlayerVoiceSound", true);
            cx.ctx.event(SimEvent::PlayerVoiceSoundNamed(param.to_string()));
        }
    }

    fn is_ignore_collide(&self, owner: &GameCharacter<I>, from: (i32, i32, i32), to: (i32, i32, i32)) -> bool {
        let p = &owner.state.params;
        if !SUCCESS.peek(p).unwrap_or(false) {
            return false;
        }
        let g = |k: Param<i32>| k.peek(p).unwrap_or(0);
        let (sx, sy, ex, ey, z) = (g(START_X), g(START_Y), g(END_X), g(END_Y), g(Z));
        if z != from.2 || z != to.2 {
            return false;
        }
        let (x1, y1, x2, y2) = (sx.min(ex), sy.min(ey), sx.max(ex), sy.max(ey));
        let (x3, y3, x4, y4) = (from.0.min(to.0), from.1.min(to.1), from.0.max(to.0), from.1.max(to.1));
        x1 <= x3 && y1 <= y3 && x2 >= x4 && y2 >= y4
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
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::*;
    use crate::character::state::{SquareInfo, StateWorld};
    use crate::character::tests::{fixture, player, Recorder};
    use crate::character::{SimContext, ZomboidGlobals};

    /// A wall with fence type 3 north of (4, 5, 0); paths reset and
    /// cancelled are counted.
    #[derive(Default)]
    struct Wall {
        resets: i32,
    }

    impl StateWorld<()> for Wall {
        fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
            Some(SquareInfo { x, y, z, solid_floor: true, ..Default::default() })
        }
        fn climb_wall_fence_type(&self, start: (i32, i32, i32), end: (i32, i32), dir: IsoDirections) -> Option<i32> {
            (start == (4, 5, 0) && end == (4, 4) && dir == IsoDirections::N).then_some(3)
        }
        fn path_reset(&mut self, _chr: &mut dyn pz_pathfind::mover::PathMover) {
            self.resets += 1;
        }
    }

    /// Scripted `Rand.Next` answers, easy climbing and the game mode.
    #[derive(Default)]
    struct Ctx {
        rec: Recorder,
        ints: RefCell<VecDeque<i32>>,
        easy: bool,
        tutorial: bool,
    }

    impl SimContext for Ctx {
        fn zomboid_globals(&self) -> &ZomboidGlobals {
            &self.rec.globals
        }
        fn event(&self, e: SimEvent) {
            self.rec.event(e)
        }
        fn rand_next(&self, max: i32) -> i32 {
            self.ints.borrow_mut().pop_front().unwrap_or((max - 1).max(0))
        }
        fn easy_climbing(&self) -> bool {
            self.easy
        }
        fn game_mode(&self) -> &str {
            if self.tutorial {
                "Tutorial"
            } else {
                "Sandbox"
            }
        }
    }

    fn climber(sm: &pz_scripts::ScriptManager, chars: &mut crate::Characters) -> GameCharacter<()> {
        let mut c = player(sm, chars);
        c.path.current_square = Some((4, 5, 0));
        (c.vars.x, c.vars.y) = (4.5, 5.5);
        c
    }

    #[test]
    fn set_params_rolls_the_struggle_and_success() {
        let (sm, mut chars) = fixture();
        let mut w = Wall::default();
        // No skills, no zombies: a fail chance of 0. The strength roll
        // (Next(100) <= max(Strength, 1)) decides, and NextBool(0) always
        // struggles.
        let ctx = Ctx::default();
        ctx.ints.borrow_mut().extend([0, 1]);
        let mut c = climber(&sm, &mut chars);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        set_params(&mut c, IsoDirections::N, &mut cx);
        assert!(c.player.climb_over_wall_struggle && c.player.climb_over_wall_success);
        assert_eq!((END_X.get(&mut c.state.params), END_Y.get(&mut c.state.params)), (4, 4));
        let ctx = Ctx::default();
        ctx.ints.borrow_mut().extend([0, 2]);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        set_params(&mut c, IsoDirections::W, &mut cx);
        assert!(!c.player.climb_over_wall_success);
        assert_eq!((END_X.get(&mut c.state.params), END_Y.get(&mut c.state.params)), (3, 5));
        // The tutorial never struggles; easy climbing always succeeds.
        let ctx = Ctx { tutorial: true, ..Default::default() };
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        set_params(&mut c, IsoDirections::N, &mut cx);
        assert!(!c.player.climb_over_wall_struggle);
        let ctx = Ctx { easy: true, ..Default::default() };
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        set_params(&mut c, IsoDirections::S, &mut cx);
        assert!(!c.player.climb_over_wall_struggle && c.player.climb_over_wall_success);
        assert!(SUCCESS.get(&mut c.state.params));
        // Not a side of the square: nothing changes.
        set_params(&mut c, IsoDirections::NE, &mut cx);
        assert_eq!(DIR.get(&mut c.state.params), Some(IsoDirections::S));
    }

    #[test]
    fn a_failed_climb_costs_endurance_and_drops_the_path() {
        let (sm, mut chars) = fixture();
        let mut w = Wall::default();
        let mut ctx = Ctx::default();
        ctx.rec.globals.running_endurance_reduce = 0.0002;
        ctx.ints.borrow_mut().extend([0, 2]);
        let mut c = climber(&sm, &mut chars);
        let endurance = chars.sim.character_stats.field("ENDURANCE").unwrap();
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        set_params(&mut c, IsoDirections::N, &mut cx);
        let s = climb_over_wall::<()>();
        c.change_state(Some(s), &mut cx);
        let _ = cx;
        // 0.0002 * 1200 for the climb and * 500 for the struggle.
        assert!((c.stats.get(&endurance) - (1.0 - 0.24 - 0.1)).abs() < 1e-6);
        assert_eq!(c.get_variable_java("ClimbFenceOutcome"), "fail");
        assert!(c.get_variable_boolean("ClimbFenceStruggle"));
        assert_eq!(w.resets, 1);
        assert_eq!(ctx.rec.events.borrow()[0], format!("{:?}", SimEvent::MusicIntensityEvent("ClimbWall")));
        // A failed climb walks through nothing.
        assert!(!s.is_ignore_collide(&c, (4, 5, 0), (4, 4, 0)));
    }

    #[test]
    fn a_climb_slides_to_the_wall_strains_the_arms_and_sounds_the_wall() {
        let (sm, mut chars) = fixture();
        let mut w = Wall::default();
        let ctx = Ctx { easy: true, ..Default::default() };
        let mut c = climber(&sm, &mut chars);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        set_params(&mut c, IsoDirections::N, &mut cx);
        let s = climb_over_wall::<()>();
        c.change_state(Some(s), &mut cx);
        assert_eq!(c.get_variable_java("ClimbFenceOutcome"), "success");
        assert!(s.is_ignore_collide(&c, (4, 5, 0), (4, 4, 0)));
        assert!(!s.is_ignore_collide(&c, (4, 5, 0), (5, 5, 0)));
        assert!(!s.is_ignore_collide(&c, (4, 5, 1), (4, 4, 1)));
        s.execute(&mut c, &mut cx);
        assert_eq!(c.vars.dir, IsoDirections::N);
        // y moves an eighth of the thirty-fps step towards 5.15.
        let step = ctx.thirty_fps_multiplier() / 8.0;
        assert!((c.vars.y - (5.5 + (5.15 - 5.5) * step)).abs() < 1e-6);
        // 0.02 * 1 * (15 - 0) / 10 * (1 / 0.8) per frame.
        let pain = 0.02 * 1.5 * 1.25;
        assert!((STRAIN.get(&mut c.state.params) - pain).abs() < 1e-6);
        // Started: no more sliding.
        c.set_var_bool("ClimbFenceStarted", true);
        let y = c.vars.y;
        s.execute(&mut c, &mut cx);
        assert_eq!(c.vars.y, y);
        let ev = |n: &str, p: &str| AnimEventRecord {
            state_name: None,
            layer_depth: -1,
            track: None,
            event: std::rc::Rc::new(pz_anim::AnimEvent { event_name: n.into(), parameter_value: Some(p.into()), ..Default::default() }),
        };
        s.anim_event(&mut c, &mut cx, &ev("PlayFenceSound", "ClimbWall"));
        s.anim_event(&mut c, &mut cx, &ev("PlayerVoiceSound", "ClimbHigh"));
        s.anim_event(&mut c, &mut cx, &ev("PlayerVoiceSound", "ClimbHigh"));
        s.exit(&mut c, &mut cx);
        let _ = cx;
        assert!(!c.vars.ignore_movement && c.get_variable_java("ClimbFenceOutcome").is_empty());
        let events = ctx.rec.events.borrow();
        let sound = format!(
            "{:?}",
            SimEvent::ObstacleSound { name: "ClimbWall".into(), parameter: "FenceTypeHigh", value: 3.0, movement_speed: false }
        );
        assert!(events.contains(&sound));
        // The voice plays once per climb.
        let voice = format!("{:?}", SimEvent::PlayerVoiceSoundNamed("ClimbHigh".into()));
        assert_eq!(events.iter().filter(|e| **e == voice).count(), 1);
    }
}
