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
    hearing_players: pz_lua_api::bind_audio::HearingPlayers,
    input: std::cell::OnceCell<pz_player_input::EnginePlayerInput>,
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
    /// Local-player callback phases read animation/simulation timing at use
    /// time. Keep all other frame policy and the same pending event owner.
    pub(crate) fn player_context(&self) -> PlayerSim<'_> {
        PlayerSim(self)
    }

    /// The shared `GameTime` (`bind_climate::shared_game_time`) and `Core`
    /// as they are now.
    pub fn capture() -> Self {
        let game_time = pz_lua_api::bind_climate::shared_game_time();
        let ctx = pz_lua_api::bind_climate::LuaClimateContext::new();
        let (debug, game_mode) = {
            let c = pz_lua_api::bind_core::shared_core();
            let c = c.borrow();
            (c.debug, c.game_mode_name().unwrap_or("null").to_owned())
        };
        let lock_fps = pz_lua_api::bind_climate::climate_settings().lock_fps;
        let s = {
            let g = game_time.borrow();
            FrameSim {
                game_time: game_time.clone(),
                hearing_players: pz_lua_api::bind_audio::HearingPlayers::capture(),
                input: Default::default(),
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

    pub(crate) fn player_input(&self) -> &pz_player_input::EnginePlayerInput {
        self.input.get_or_init(pz_player_input::EnginePlayerInput::capture)
    }

    /// The events raised since the last call.
    pub fn take_events(&self) -> Vec<SimEvent> {
        std::mem::take(&mut self.events.borrow_mut())
    }
}

/// A player-only timing view. This does not alter the clocks used by the
/// world's other moving objects, population, or model update scheduling.
pub(crate) struct PlayerSim<'a>(&'a FrameSim);

impl SimContext for PlayerSim<'_> {
    fn hydro_power_on(&self) -> bool { self.0.hydro_power_on() }
    fn lore_hearing(&self) -> i32 { self.0.lore_hearing() }
    fn is_listener_in_range(&self, x: f32, y: f32, range: f32) -> bool { self.0.is_listener_in_range(x, y, range) }
    fn easy_climbing(&self) -> bool { self.0.easy_climbing() }
    fn is_debug(&self) -> bool {
        self.0.is_debug()
    }
    fn game_mode(&self) -> &str {
        self.0.game_mode()
    }
    fn multiplier(&self) -> f32 {
        let context = pz_lua_api::bind_climate::LuaClimateContext::new();
        self.0.game_time.borrow().get_multiplier(&context)
    }
    fn thirty_fps_multiplier(&self) -> f32 {
        self.multiplier() / 1.6
    }
    fn inv_multiplier(&self) -> f32 {
        1.0 / self.multiplier()
    }
    fn multiplier_bias(&self) -> f32 {
        self.0.game_time.borrow().multiplier_bias
    }
    fn time_delta(&self) -> f32 {
        let context = pz_lua_api::bind_climate::LuaClimateContext::new();
        self.0.game_time.borrow().get_time_delta(&context)
    }
    fn realworld_seconds_since_last_update(&self) -> f32 {
        self.0
            .game_time
            .borrow()
            .realworld_seconds_since_last_update()
    }
    fn true_multiplier(&self) -> f32 {
        self.0.game_time.borrow().get_true_multiplier()
    }
    fn minutes_per_day(&self) -> f32 {
        self.0.game_time.borrow().minutes_per_day
    }
    fn minutes(&self) -> i32 {
        self.0.game_time.borrow().minutes()
    }
    fn calendar_time_in_millis(&self) -> i64 {
        self.0.game_time.borrow().calendar_time_in_millis()
    }
    fn world_age_hours(&self) -> f64 {
        self.0.game_time.borrow().world_age_hours()
    }
    fn time_of_day(&self) -> f32 {
        self.0.game_time.borrow().time_of_day
    }
    fn last_time_of_day(&self) -> f32 {
        self.0.game_time.borrow().last_time_of_day
    }
    fn num_players(&self) -> i32 {
        pz_lua_api::bind_players::num_players()
    }
    fn climate(&self) -> Option<Ref<'_, ClimateManager>> {
        self.0.climate()
    }
    fn wet_ground_final_value(&self) -> f32 {
        self.0.wet_ground_final_value()
    }
    fn clothing_degradation(&self) -> i32 {
        self.0.clothing_degradation()
    }
    fn blood_level(&self) -> i32 {
        self.0.blood_level()
    }
    fn firearm_use_damage_chance(&self) -> i32 {
        self.0.firearm_use_damage_chance()
    }
    fn firearm_moodle_multiplier(&self) -> f64 {
        self.0.firearm_moodle_multiplier()
    }
    fn firearm_weather_multiplier(&self) -> f64 {
        self.0.firearm_weather_multiplier()
    }
    fn firearm_head_gear_effect(&self) -> bool {
        self.0.firearm_head_gear_effect()
    }
    fn firearm_jam_multiplier(&self) -> f64 {
        self.0.firearm_jam_multiplier()
    }
    fn max_active_ragdolls(&self) -> i32 {
        self.0.max_active_ragdolls()
    }
    fn debug_disable_ragdolls(&self) -> bool {
        self.0.debug_disable_ragdolls()
    }
    fn debug_always_hit_target(&self) -> bool {
        self.0.debug_always_hit_target()
    }
    fn lock_fps(&self) -> i32 {
        self.0.lock_fps()
    }
    fn event(&self, event: SimEvent) {
        self.0.event(event);
    }
}

impl SimContext for FrameSim {
    fn hydro_power_on(&self) -> bool { pz_lua_api::bind_world::power::hydro_power_on() }
    fn lore_hearing(&self) -> i32 {
        pz_lua_api::bind_sandbox::current_double("ZombieLore.Hearing").unwrap_or(2.0) as i32
    }
    fn is_listener_in_range(&self, x: f32, y: f32, range: f32) -> bool { self.hearing_players.in_range(x, y, range) }
    fn easy_climbing(&self) -> bool { pz_lua_api::bind_sandbox::current_bool("EasyClimbing").unwrap_or(false) }
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
        self.game_time
            .try_borrow()
            .ok()
            .map(|g| Ref::map(g, |g| &g.climate))
    }
    fn wet_ground_final_value(&self) -> f32 {
        // Movement and ground dirt query IsoPuddles at the point of use,
        // including changes made by Lua after this frame was captured.
        pz_lua_api::bind_puddles::shared_puddles()
            .borrow()
            .state
            .wet_ground
            .get()
    }
    fn clothing_degradation(&self) -> i32 {
        pz_lua_api::bind_sandbox::current_int("ClothingDegradation").unwrap_or(3)
    }
    fn blood_level(&self) -> i32 {
        pz_lua_api::bind_sandbox::current_int("BloodLevel").unwrap_or(3)
    }
    fn firearm_use_damage_chance(&self) -> i32 {
        pz_lua_api::bind_sandbox::current_int("FirearmUseDamageChance").unwrap_or(2)
    }
    fn firearm_moodle_multiplier(&self) -> f64 {
        pz_lua_api::bind_sandbox::current_double("FirearmMoodleMultiplier").unwrap_or(1.0)
    }
    fn firearm_weather_multiplier(&self) -> f64 {
        pz_lua_api::bind_sandbox::current_double("FirearmWeatherMultiplier").unwrap_or(1.0)
    }
    fn firearm_head_gear_effect(&self) -> bool {
        pz_lua_api::bind_sandbox::current_bool("FirearmHeadGearEffect").unwrap_or(true)
    }
    fn firearm_jam_multiplier(&self) -> f64 {
        pz_lua_api::bind_sandbox::current_double("FirearmJamMultiplier").unwrap_or(0.0)
    }
    fn max_active_ragdolls(&self) -> i32 {
        pz_lua_api::bind_core::shared_core()
            .try_borrow()
            .map_or(20, |c| c.get_max_active_ragdolls())
    }
    fn debug_disable_ragdolls(&self) -> bool {
        self.debug
            && pz_core::debug::debug_options()
                .try_borrow()
                .is_ok_and(|o| o.get_boolean("Animation.DisableRagdolls"))
    }
    fn debug_always_hit_target(&self) -> bool {
        self.debug
            && pz_core::debug::debug_options()
                .try_borrow()
                .is_ok_and(|o| o.get_boolean("Character.Debug.AlwaysHitTarget"))
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
    fn player_sim_reads_callback_timing_and_keeps_the_frame_event_owner() {
        struct RestoreTime(Rc<RefCell<GameTime>>);
        impl Drop for RestoreTime {
            fn drop(&mut self) {
                pz_lua_api::bind_climate::install_game_time(self.0.clone());
            }
        }
        let _restore = RestoreTime(pz_lua_api::bind_climate::shared_game_time());
        let time = Rc::new(RefCell::new(GameTime::default()));
        pz_lua_api::bind_climate::install_game_time(time.clone());
        let frame = FrameSim::capture();
        let before = frame.multiplier;
        let player = frame.player_context();
        let mut thread = pz_kahlua::Thread::new(pz_kahlua::TableRef::new());
        pz_lua_api::bind_climate::register_with(&mut thread, &[]);
        let callback = pz_kahlua::stdlib::load_string(
            "getGameTime():setMultiplier(4); getGameTime():setMinutesPerDay(90); getGameTime():setTimeOfDay(13.5)",
            Some("player-clock-callback"),
            thread.globals().clone(),
        )
        .unwrap();
        thread.pcall(&callback, &[]).unwrap();
        let clock = pz_lua_api::bind_climate::LuaClimateContext::new();
        let expected = time.borrow().get_multiplier(&clock);
        assert_ne!(before, expected);
        assert_eq!(
            frame.multiplier(),
            before,
            "changed the other frame consumers"
        );
        assert_eq!(player.multiplier(), expected);
        assert_eq!(player.thirty_fps_multiplier(), expected / 1.6);
        assert_eq!(player.inv_multiplier(), 1.0 / expected);
        assert_eq!(player.time_delta(), time.borrow().get_time_delta(&clock));
        assert_eq!(player.minutes_per_day(), 90.0);
        assert_eq!(player.minutes(), time.borrow().minutes());
        assert_eq!(player.calendar_time_in_millis(), time.borrow().calendar_time_in_millis());
        assert_eq!(player.world_age_hours(), time.borrow().world_age_hours());
        time.borrow_mut().fps_multiplier = 2.0;
        assert_eq!(
            player.realworld_seconds_since_last_update(),
            time.borrow().realworld_seconds_since_last_update()
        );
        player.event(SimEvent::StopAllActionQueue);
        assert!(matches!(
            frame.take_events().as_slice(),
            [SimEvent::StopAllActionQueue]
        ));
    }

    #[test]
    fn frame_sim_reads_live_puddle_controls_after_capture() {
        let owner = pz_lua_api::bind_puddles::shared_puddles();
        struct RestorePuddles(pz_iso_render::weather::PuddlesState);
        impl Drop for RestorePuddles {
            fn drop(&mut self) {
                pz_lua_api::bind_puddles::shared_puddles()
                    .borrow_mut()
                    .state = self.0;
            }
        }
        let _restore = RestorePuddles(owner.borrow().state);
        owner.borrow_mut().state = Default::default();
        let frame = FrameSim::capture();
        let mut thread = pz_kahlua::Thread::new(pz_kahlua::TableRef::new());
        pz_lua_api::bind_puddles::register(&mut thread);
        let mut run = |source| {
            let closure = pz_kahlua::stdlib::load_string(
                source,
                Some("wet-ground"),
                thread.globals().clone(),
            )
            .unwrap();
            thread
                .pcall(&closure, &[])
                .unwrap_or_else(|(error, trace)| panic!("{error:?}\n{trace}"));
        };
        run("getPuddlesManager():getPuddlesFloat(1):setFinalValue(0.75)");
        assert_eq!(frame.wet_ground_final_value(), 0.75);
        run("local wet = getPuddlesManager():getPuddlesFloat(1); wet:setAdminValue(0.25); wet:setEnableAdmin(true)");
        assert_eq!(frame.wet_ground_final_value(), 0.25);
        run("getPuddlesManager():getPuddlesFloat(1):setEnableAdmin(false)");
        assert_eq!(frame.wet_ground_final_value(), 0.75);
    }

    #[test]
    fn frame_sim_reads_live_climate_wind_and_core_ragdoll_limit() {
        let frame = FrameSim::capture();
        let player = frame.player_context();
        let expected_wind = pz_lua_api::bind_climate::shared_game_time()
            .borrow()
            .climate
            .get_wind_intensity();
        assert_eq!(frame.wind_intensity(), expected_wind);
        assert_eq!(player.wind_intensity(), expected_wind);
        let expected_max = pz_lua_api::bind_core::shared_core()
            .borrow()
            .get_max_active_ragdolls();
        assert_eq!(frame.max_active_ragdolls(), expected_max);
        assert_eq!(player.max_active_ragdolls(), expected_max);
    }
}
