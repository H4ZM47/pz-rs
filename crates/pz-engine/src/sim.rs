//! The `GameTime` / `Core` singletons as the character simulation reads
//! them during one game frame (`pz_characters::character::SimContext`).

use pz_characters::character::{SimContext, SimEvent};
use pz_core::game_time::GameTime;
use pz_core::weather::ClimateManager;
use std::cell::{Ref, RefCell};
use std::rc::Rc;

/// A frame's snapshot of `GameTime.getInstance()` and `Core`, with the
/// shared `GameTime` for `ClimateManager.getInstance()`. Events the
/// simulation raises are collected in [`FrameSim::events`].
pub struct FrameSim {
    pub game_time: Rc<RefCell<GameTime>>,
    /// `Core.bDebug`.
    pub debug: bool,
    /// `Core.getInstance().getGameMode()`.
    pub game_mode: String,
    /// `GameTime.getMultiplier()`.
    pub multiplier: f32,
    /// `GameTime.getTrueMultiplier()`.
    pub true_multiplier: f32,
    /// `GameTime.getMinutesPerDay()`.
    pub minutes_per_day: f32,
    /// `GameTime.getMinutes()`.
    pub minutes: i32,
    /// `GameTime.getWorldAgeHours()`.
    pub world_age_hours: f64,
    /// `GameTime.getCalender().getTimeInMillis()`.
    pub calendar_millis: i64,
    /// `GameTime.getRealworldSecondsSinceLastUpdate()`.
    pub realworld_seconds: f32,
    /// `PerformanceSettings.getLockFPS()`.
    pub lock_fps: i32,
    pub events: RefCell<Vec<SimEvent>>,
}

impl FrameSim {
    /// The shared `GameTime` (`bind_climate::shared_game_time`) and `Core`
    /// as they are now.
    pub fn capture() -> Self {
        let game_time = pz_lua_api::bind_climate::shared_game_time();
        let ctx = pz_lua_api::bind_climate::LuaClimateContext::new();
        let (debug, game_mode) = {
            let c = pz_lua_api::bind_core::shared_core();
            let c = c.borrow();
            (c.debug, c.game_mode.clone())
        };
        let lock_fps = pz_lua_api::bind_climate::climate_settings().lock_fps;
        let s = {
            let g = game_time.borrow();
            FrameSim {
                game_time: game_time.clone(),
                debug,
                game_mode,
                multiplier: g.get_multiplier(&ctx),
                true_multiplier: g.get_true_multiplier(),
                minutes_per_day: g.minutes_per_day,
                minutes: g.minutes(),
                world_age_hours: g.world_age_hours(),
                calendar_millis: g.calendar_time_in_millis(),
                realworld_seconds: g.realworld_seconds_since_last_update(),
                lock_fps,
                events: RefCell::default(),
            }
        };
        s
    }

    /// The events raised since the last call.
    pub fn take_events(&self) -> Vec<SimEvent> {
        std::mem::take(&mut self.events.borrow_mut())
    }
}

impl SimContext for FrameSim {
    fn is_debug(&self) -> bool {
        self.debug
    }
    fn game_mode(&self) -> &str {
        &self.game_mode
    }
    fn multiplier(&self) -> f32 {
        self.multiplier
    }
    fn minutes_per_day(&self) -> f32 {
        self.minutes_per_day
    }
    fn thirty_fps_multiplier(&self) -> f32 {
        self.multiplier / 1.6
    }
    fn minutes(&self) -> i32 {
        self.minutes
    }
    fn calendar_time_in_millis(&self) -> i64 {
        self.calendar_millis
    }
    fn world_age_hours(&self) -> f64 {
        self.world_age_hours
    }
    fn climate(&self) -> Option<Ref<'_, ClimateManager>> {
        self.game_time.try_borrow().ok().map(|g| Ref::map(g, |g| &g.climate))
    }
    fn wet_ground_final_value(&self) -> f32 {
        // Movement and ground dirt query IsoPuddles at the point of use,
        // including changes made by Lua after this frame was captured.
        pz_lua_api::bind_puddles::shared_puddles().borrow().state.wet_ground.get()
    }
    fn lock_fps(&self) -> i32 {
        self.lock_fps
    }
    fn inv_multiplier(&self) -> f32 {
        1.0 / self.multiplier
    }
    fn realworld_seconds_since_last_update(&self) -> f32 {
        self.realworld_seconds
    }
    fn true_multiplier(&self) -> f32 {
        self.true_multiplier
    }
    fn event(&self, e: SimEvent) {
        self.events.borrow_mut().push(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_sim_reads_live_puddle_controls_after_capture() {
        let owner = pz_lua_api::bind_puddles::shared_puddles();
        struct RestorePuddles(pz_iso_render::weather::PuddlesState);
        impl Drop for RestorePuddles {
            fn drop(&mut self) {
                pz_lua_api::bind_puddles::shared_puddles().borrow_mut().state = self.0;
            }
        }
        let _restore = RestorePuddles(owner.borrow().state);
        owner.borrow_mut().state = Default::default();
        let frame = FrameSim::capture();
        let mut thread = pz_kahlua::Thread::new(pz_kahlua::TableRef::new());
        pz_lua_api::bind_puddles::register(&mut thread);
        let mut run = |source| {
            let closure = pz_kahlua::stdlib::load_string(source, Some("wet-ground"), thread.globals().clone()).unwrap();
            thread.pcall(&closure, &[]).unwrap_or_else(|(error, trace)| panic!("{error:?}\n{trace}"));
        };
        run("getPuddlesManager():getPuddlesFloat(1):setFinalValue(0.75)");
        assert_eq!(frame.wet_ground_final_value(), 0.75);
        run("local wet = getPuddlesManager():getPuddlesFloat(1); wet:setAdminValue(0.25); wet:setEnableAdmin(true)");
        assert_eq!(frame.wet_ground_final_value(), 0.25);
        run("getPuddlesManager():getPuddlesFloat(1):setEnableAdmin(false)");
        assert_eq!(frame.wet_ground_final_value(), 0.75);
    }
}
