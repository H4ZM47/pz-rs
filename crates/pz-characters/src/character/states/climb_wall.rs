//! `ClimbOverWallState` (a player climbing over a tall wall or fence),
//! with `IsoPlayer.climbOverWall` / `canClimbOverWall`.
//!
//! The wall checks are the world's ([`StateWorld::can_climb_over_wall`],
//! [`StateWorld::climb_wall_fence_type`]). Not here: the music intensity
//! event, `setAnimated(true)` (the renderer's), the zombie's network
//! `isClimbing` flag and the network `processOnEnter/Exit`.
//!
//! [`StateWorld::can_climb_over_wall`]: crate::character::state::StateWorld::can_climb_over_wall
//! [`StateWorld::climb_wall_fence_type`]: crate::character::state::StateWorld::climb_wall_fence_type

use pz_anim::{AnimEventRecord, AnimationVariableSource};
use pz_world::square::IsoDirections;

use super::ids;
use crate::character::state::{Param, State, StateCx, StateId, StateRef, SyncFlags};
use crate::character::{GameCharacter, SimEvent};

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

/// `ClimbOverWallState.instance()`.
pub fn climb_over_wall<I: 'static>() -> StateRef<I> {
    static S: ClimbOverWallState = ClimbOverWallState;
    &S
}

/// `ClimbOverWallState.setParams(owner, dir)`: the squares, then a local
/// player's struggle and success draws.
pub fn set_params<I: 'static>(
    owner: &mut GameCharacter<I>,
    dir: IsoDirections,
    cx: &mut StateCx<'_, I>,
) {
    // The original dereferences the square.
    let Some((x, y, z)) = owner.path.current_square else {
        return;
    };
    let (mut end_x, mut end_y) = (x, y);
    match dir {
        IsoDirections::N => end_y -= 1,
        IsoDirections::S => end_y += 1,
        IsoDirections::W => end_x -= 1,
        IsoDirections::E => end_x += 1,
        // IllegalArgumentException("invalid direction").
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
            let heavy_load = owner.moodles.moodle_level_of(cx.sm, "HEAVY_LOAD").unwrap_or(0);
            let strength = owner.perk_level(cx.chars.perks.constant("Strength").as_ref());
            let tutorial = cx.ctx.game_mode() == "Tutorial";
            let (struggle, success) = wall_draws(struggle_chance, heavy_load, strength, tutorial, |n| cx.ctx.rand_next(n));
            owner.player.climb_over_wall_struggle = struggle;
            owner.player.climb_over_wall_success = success;
        }
    }
    let (struggle, success) = (
        owner.player.climb_over_wall_struggle,
        owner.player.climb_over_wall_success,
    );
    STRUGGLE.set(&mut owner.state.params, struggle);
    SUCCESS.set(&mut owner.state.params, success);
}

/// `setParams`' draws for a local player without easy climbing: the
/// struggle (`Rand.NextBool(chance / 2)`, never in the tutorial) and the
/// success (`!NextBool(chance)`, or with a fail chance of 0 and no heavy
/// load `Next(100) <= max(1, strength)`). `next` is `Rand.Next(int)`.
pub fn wall_draws(
    struggle_chance: i32,
    heavy_load: i32,
    strength: i32,
    tutorial: bool,
    mut next: impl FnMut(i32) -> i32,
) -> (bool, bool) {
    let mut struggle = next(struggle_chance / 2) == 0;
    if tutorial {
        struggle = false;
    }
    let mut success = false;
    if struggle_chance > 0 {
        success = next(struggle_chance) != 0;
    } else if heavy_load == 0 {
        success = next(100) <= strength.max(1);
    }
    (struggle, success)
}

impl<I: 'static> GameCharacter<I> {
    /// `IsoPlayer.canClimbOverWall(dir)`.
    pub fn can_climb_over_wall(&self, dir: IsoDirections, cx: &StateCx<'_, I>) -> bool {
        if self.is_sprinting(cx.sm, cx.ctx) {
            return false;
        }
        cx.world.can_climb_over_wall(self, dir)
    }

    /// `IsoPlayer.climbOverWall(dir)`.
    pub fn climb_over_wall(&mut self, dir: IsoDirections, cx: &mut StateCx<'_, I>) -> bool {
        if !self.can_climb_over_wall(dir, cx) {
            return false;
        }
        self.drop_heavy_items_in(cx);
        set_params(self, dir, cx);
        self.report_event("EventClimbWall");
        true
    }
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
        let reduce = cx.ctx.zomboid_globals().running_endurance_reduce;
        let endurance = cx.chars.sim.character_stats.field("ENDURANCE").ok();
        if let Some(e) = &endurance {
            owner.stats.remove(e, (reduce * 1200.0) as f32);
        }
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
            owner.set_var_str(
                "ClimbFenceOutcome",
                if success { "success" } else { "fail" },
            );
            owner.set_var_bool("ClimbFenceStruggle", struggle);
        } else {
            let s = SUCCESS.get(&mut owner.state.params);
            let st = STRUGGLE.get(&mut owner.state.params);
            owner.set_var_str("ClimbFenceOutcome", if s { "success" } else { "fail" });
            owner.set_var_bool("ClimbFenceStruggle", st);
        }
        // isLocalPlayer: triggerMusicIntensityEvent("ClimbWall") (the
        // music system's).
        if !success {
            // getPathFindBehavior2().reset(), cancel(), setPath2(null).
            cx.world.path_cancel(owner);
            owner.path.path2 = None;
        }
        STRAIN.set(&mut owner.state.params, 0.0);
    }

    fn execute(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>) {
        // The original dereferences the direction.
        let Some(dir) = DIR.get(&mut owner.state.params) else {
            return;
        };
        owner.set_forward_iso_direction(dir);
        let nimble = owner.perk_level(cx.chars.perks.constant("Nimble").as_ref());
        let strength = owner.perk_level(cx.chars.perks.constant("Strength").as_ref());
        let skill_factor = (nimble + strength * 2) as f32 / 3.0;
        let heavy = owner
            .moodles
            .moodle_level_of(cx.sm, "HEAVY_LOAD")
            .unwrap_or(0);
        let m = cx.ctx.multiplier();
        let pain = (0.02 * m as f64 * (heavy + 1) as f64) as f32
            * ((15.0 - skill_factor) / 10.0)
            * (m / 0.8);
        owner.add_both_arm_muscle_strain(cx, pain);
        let strain = STRAIN.get(&mut owner.state.params);
        STRAIN.set(&mut owner.state.params, strain + pain);
        if !owner.get_variable_boolean("ClimbFenceStarted") {
            let start_x = START_X.get(&mut owner.state.params);
            let start_y = START_Y.get(&mut owner.state.params);
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
        for v in [
            "ClimbingFence",
            "ClimbFenceFinished",
            "ClimbFenceOutcome",
            "ClimbFenceStarted",
            "ClimbFenceStruggle",
            "PlayerVoiceSound",
        ] {
            owner.clear_var(v);
        }
        owner.vars.ignore_movement = false;
        owner.vars.hide_weapon_model = false;
    }

    fn anim_event(
        &self,
        owner: &mut GameCharacter<I>,
        cx: &mut StateCx<'_, I>,
        e: &AnimEventRecord,
    ) {
        let name = e.event.event_name.as_str();
        let param = e.event.parameter_value.clone().unwrap_or_default();
        if name.eq_ignore_ascii_case("PlayFenceSound") {
            let p = &mut owner.state.params;
            let start = (START_X.get(p), START_Y.get(p), Z.get(p));
            let end = (END_X.get(p), END_Y.get(p));
            let Some(dir) = DIR.get(p) else { return };
            let Some(fence_type) = cx.world.climb_wall_fence_type(start, end, dir) else {
                return;
            };
            cx.ctx.event(SimEvent::ObstacleSound {
                name: param.clone(),
                parameter: "FenceTypeHigh",
                value: fence_type as f32,
                movement_speed: false,
            });
        }
        if name.eq_ignore_ascii_case("PlayerVoiceSound") {
            if owner.get_variable_boolean("PlayerVoiceSound") || !owner.is_player() {
                return;
            }
            owner.set_var_bool("PlayerVoiceSound", true);
            cx.ctx.event(SimEvent::PlayerVoiceSoundNamed(param));
        }
    }

    fn is_ignore_collide(
        &self,
        owner: &GameCharacter<I>,
        from: (i32, i32, i32),
        to: (i32, i32, i32),
    ) -> bool {
        let p = &owner.state.params;
        if !SUCCESS.peek(p).unwrap_or(false) {
            return false;
        }
        let g = |q: Param<i32>| q.peek(p).unwrap_or(0);
        let (start_x, start_y, end_x, end_y, z) =
            (g(START_X), g(START_Y), g(END_X), g(END_Y), g(Z));
        if z != from.2 || z != to.2 {
            return false;
        }
        let (x1, y1, x2, y2) = (
            start_x.min(end_x),
            start_y.min(end_y),
            start_x.max(end_x),
            start_y.max(end_y),
        );
        let (x3, y3, x4, y4) = (
            from.0.min(to.0),
            from.1.min(to.1),
            from.0.max(to.0),
            from.1.max(to.1),
        );
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
    use super::*;
    use crate::character::state::StateWorld;
    use crate::character::tests::{fixture, player, Recorder};

    #[derive(Default)]
    struct Wall {
        cancelled: usize,
    }

    impl StateWorld<()> for Wall {
        fn can_climb_over_wall(&self, chr: &GameCharacter<()>, dir: IsoDirections) -> bool {
            dir == IsoDirections::N && chr.path.current_square == Some((5, 5, 0))
        }
        fn climb_wall_fence_type(&self, start: (i32, i32, i32), end: (i32, i32), dir: IsoDirections) -> Option<i32> {
            assert_eq!((start, end, dir), ((5, 5, 0), (5, 4), IsoDirections::N));
            Some(1)
        }
        fn path_cancel(&mut self, _chr: &mut dyn pz_pathfind::mover::PathMover) {
            self.cancelled += 1;
        }
    }

    #[test]
    fn climbing_a_wall_draws_struggle_and_success_like_the_original() {
        let (sm, mut chars) = fixture();
        let ctx = Recorder::default();
        let mut w = Wall::default();
        let mut p = player(&sm, &mut chars);
        (p.vars.x, p.vars.y) = (5.5, 5.7);
        p.path.current_square = Some((5, 5, 0));
        for seed in 0..20 {
            pz_core::random::set_seed(seed);
            let r = pz_core::random::Rand::new(seed);
            let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
            assert!(!p.climb_over_wall(IsoDirections::W, &mut cx));
            assert!(p.climb_over_wall(IsoDirections::N, &mut cx));
            // No skills: fail chance 0, so `NextBool(0)` (no draw) struggles
            // and an unencumbered climber has `Next(100) <= 1`.
            assert!(p.player.climb_over_wall_struggle);
            assert_eq!(p.player.climb_over_wall_success, r.next(100) <= 1, "seed {seed}");
            assert_eq!(pz_core::random::next(1 << 30), r.next(1 << 30));
            let p_ = &mut p.state.params;
            assert_eq!((START_X.get(p_), START_Y.get(p_), END_X.get(p_), END_Y.get(p_), DIR.get(p_)), (5, 5, 5, 4, Some(IsoDirections::N)));
        }
        p.player.climb_over_wall_success = true;
        SUCCESS.set(&mut p.state.params, true);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        let s = climb_over_wall::<()>();
        s.enter(&mut p, &mut cx);
        assert_eq!(p.variable_string("ClimbFenceOutcome"), "success");
        assert!(p.variable_boolean("ClimbFenceStruggle"));
        assert!(s.is_ignore_collide(&p, (5, 5, 0), (5, 4, 0)));
        assert!(!s.is_ignore_collide(&p, (5, 5, 0), (6, 5, 0)));
        // Before the climb starts it moves an eighth of the way to 0.15
        // inside the wall's square.
        s.execute(&mut p, &mut cx);
        assert_eq!(p.vars.dir, IsoDirections::N);
        let mult = cx.ctx.thirty_fps_multiplier() / 8.0;
        assert_eq!(p.vars.y, 5.7 + (5.15 - 5.7) * mult);
        let ev = AnimEventRecord {
            state_name: None,
            layer_depth: -1,
            track: None,
            event: std::rc::Rc::new(pz_anim::AnimEvent { event_name: "PlayFenceSound".into(), parameter_value: Some("ClimbWall".into()), ..Default::default() }),
        };
        s.anim_event(&mut p, &mut cx, &ev);
        s.exit(&mut p, &mut cx);
        assert!(!p.vars.ignore_movement);
        // A failed climb drops the path.
        p.player.climb_over_wall_success = false;
        s.enter(&mut p, &mut cx);
        assert_eq!(p.variable_string("ClimbFenceOutcome"), "fail");
        let _ = cx;
        assert_eq!(w.cancelled, 1);
        assert!(ctx.events.borrow().iter().any(|e| e.contains("FenceTypeHigh") && e.contains("value: 1.0")));
    }
}
