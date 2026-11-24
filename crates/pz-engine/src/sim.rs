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
    zomboid_globals: pz_characters::character::ZomboidGlobals,
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
                zomboid_globals: pz_lua_api::bind_character_sim::zomboid_globals(),
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
    fn zomboid_globals(&self) -> &pz_characters::character::ZomboidGlobals {
        self.0.zomboid_globals()
    }
    fn hydro_power_on(&self) -> bool { self.0.hydro_power_on() }
    fn lore_hearing(&self) -> i32 { self.0.lore_hearing() }
    fn is_listener_in_range(&self, x: f32, y: f32, range: f32) -> bool { self.0.is_listener_in_range(x, y, range) }
    fn easy_climbing(&self) -> bool { self.0.easy_climbing() }
    fn debug_always_trip_over_fence(&self) -> bool { self.0.debug_always_trip_over_fence() }
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
    fn month(&self) -> i32 {
        self.0.game_time.borrow().month
    }
    fn time_since_apo(&self) -> i32 {
        self.0.time_since_apo()
    }
    fn animal_sandbox(&self) -> pz_characters::pz_animals::AnimalSandbox {
        self.0.animal_sandbox()
    }
    fn rain_manager_is_raining(&self) -> bool {
        self.0.rain_manager_is_raining()
    }
    fn rain_manager_intensity(&self) -> f32 {
        self.0.rain_manager_intensity()
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
    fn lock_fps(&self) -> i32 {
        self.0.lock_fps()
    }
    fn wind_intensity(&self) -> f32 {
        self.0.wind_intensity()
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
    fn active_ragdoll_simulations(&self) -> i32 {
        self.0.active_ragdoll_simulations()
    }
    fn max_active_ragdolls(&self) -> i32 {
        self.0.max_active_ragdolls()
    }
    fn event(&self, event: SimEvent) {
        self.0.event(event);
    }
}

impl SimContext for FrameSim {
    fn zomboid_globals(&self) -> &pz_characters::character::ZomboidGlobals {
        &self.zomboid_globals
    }
    fn hydro_power_on(&self) -> bool { pz_lua_api::bind_world::power::hydro_power_on() }
    fn lore_hearing(&self) -> i32 {
        pz_lua_api::bind_sandbox::current_double("ZombieLore.Hearing").unwrap_or(2.0) as i32
    }
    fn is_listener_in_range(&self, x: f32, y: f32, range: f32) -> bool { self.hearing_players.in_range(x, y, range) }
    fn easy_climbing(&self) -> bool { pz_lua_api::bind_sandbox::current_bool("EasyClimbing").unwrap_or(false) }
    fn debug_always_trip_over_fence(&self) -> bool {
        self.debug
            && pz_core::debug::debug_options()
                .borrow()
                .get_boolean("Character.Debug.AlwaysTripOverFence")
    }
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
    fn time_of_day(&self) -> f32 {
        self.game_time
            .try_borrow()
            .map(|g| g.time_of_day)
            .unwrap_or(0.0)
    }
    fn last_time_of_day(&self) -> f32 {
        self.game_time
            .try_borrow()
            .map(|g| g.last_time_of_day)
            .unwrap_or(0.0)
    }
    fn month(&self) -> i32 {
        self.game_time.try_borrow().map(|g| g.month).unwrap_or(7)
    }
    fn time_since_apo(&self) -> i32 {
        pz_lua_api::bind_sandbox::current_double("TimeSinceApo")
            .map(|v| v as i32)
            .unwrap_or(1)
    }
    fn animal_sandbox(&self) -> pz_characters::pz_animals::AnimalSandbox {
        let mut s = pz_characters::pz_animals::AnimalSandbox::default();
        if let Some(v) = pz_lua_api::bind_sandbox::current_double("AnimalStatsModifier") {
            s.animal_stats_modifier = v as i32;
        }
        if let Some(v) = pz_lua_api::bind_sandbox::current_double("AnimalMetaStatsModifier") {
            s.animal_meta_stats_modifier = v as i32;
        }
        if let Some(v) = pz_lua_api::bind_sandbox::current_double("AnimalPregnancyTime") {
            s.animal_pregnancy_time = v as i32;
        }
        if let Some(v) = pz_lua_api::bind_sandbox::current_double("AnimalAgeModifier") {
            s.animal_age_modifier = v as i32;
        }
        if let Some(v) = pz_lua_api::bind_sandbox::current_double("AnimalMilkIncModifier") {
            s.animal_milk_inc_modifier = v as i32;
        }
        if let Some(v) = pz_lua_api::bind_sandbox::current_double("AnimalWoolIncModifier") {
            s.animal_wool_inc_modifier = v as i32;
        }
        if let Some(v) = pz_lua_api::bind_sandbox::current_bool("AnimalMetaPredator") {
            s.animal_meta_predator = v;
        }
        if let Some(v) = pz_lua_api::bind_sandbox::current_bool("AnimalMatingSeason") {
            s.animal_mating_season = v;
        }
        s
    }
    fn rain_manager_is_raining(&self) -> bool {
        self.game_time
            .try_borrow()
            .is_ok_and(|g| g.climate.is_raining())
    }
    fn rain_manager_intensity(&self) -> f32 {
        self.game_time
            .try_borrow()
            .map(|g| g.climate.get_rain_intensity())
            .unwrap_or(0.0)
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
    fn wind_intensity(&self) -> f32 {
        self.game_time
            .try_borrow()
            .map(|g| g.climate.get_wind_intensity())
            .unwrap_or(0.0)
    }
    fn firearm_use_damage_chance(&self) -> i32 {
        pz_lua_api::bind_sandbox::current_double("FirearmUseDamageChance")
            .map(|v| v as i32)
            .unwrap_or(2)
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
    fn active_ragdoll_simulations(&self) -> i32 {
        crate::world_ragdolls::active_ragdoll_simulations()
    }
    fn max_active_ragdolls(&self) -> i32 {
        pz_lua_api::bind_core::shared_core()
            .borrow()
            .get_max_active_ragdolls()
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
    fn debug_always_trip_over_fence_requires_debug_and_option() {
        let core = pz_lua_api::bind_core::shared_core();
        let key = "Character.Debug.AlwaysTripOverFence";
        let prev_debug = core.borrow().debug;
        let (prev_opts, prev_opts_debug) = {
            let opts = pz_core::debug::debug_options();
            let o = opts.borrow();
            (o.options.clone(), o.core_debug)
        };
        struct Restore(
            bool,
            Vec<(pz_core::debug::DebugOptionDef, pz_core::config::ConfigOption)>,
            bool,
        );
        impl Drop for Restore {
            fn drop(&mut self) {
                pz_lua_api::bind_core::shared_core().borrow_mut().debug = self.0;
                let opts = pz_core::debug::debug_options();
                let mut o = opts.borrow_mut();
                o.options = std::mem::take(&mut self.1);
                o.core_debug = self.2;
            }
        }
        let _restore = Restore(prev_debug, prev_opts, prev_opts_debug);

        {
            let opts = pz_core::debug::debug_options();
            let mut o = opts.borrow_mut();
            o.core_debug = true;
            o.set_definitions(vec![pz_core::debug::DebugOptionDef {
                name: key.into(),
                default_value: false,
                debug_only: true,
            }]);
            o.set_boolean(key, true);
        }

        core.borrow_mut().debug = false;
        let non_debug = FrameSim::capture();
        assert!(!non_debug.debug_always_trip_over_fence());
        assert!(!non_debug.player_context().debug_always_trip_over_fence());

        core.borrow_mut().debug = true;
        let debug_on = FrameSim::capture();
        assert!(debug_on.debug_always_trip_over_fence());
        assert!(debug_on.player_context().debug_always_trip_over_fence());

        pz_core::debug::debug_options().borrow_mut().set_boolean(key, false);
        assert!(!debug_on.debug_always_trip_over_fence());
    }

    #[test]
    fn zomboid_globals_load_reaches_frame_sim_player_sim_and_item_env() {
        use pz_inventory::context::ItemContext;
        struct Restore(Option<pz_characters::character::ZomboidGlobals>);
        impl Drop for Restore {
            fn drop(&mut self) {
                pz_lua_api::bind_character_sim::set_zomboid_globals(self.0.take());
            }
        }
        let _restore = Restore(None);

        let env = pz_kahlua::TableRef::new();
        let zg = pz_kahlua::TableRef::new();
        zg.set_str("HungerIncrease", 0.0000096);
        zg.set_str("ThirstIncrease", 0.0000080);
        zg.set_str("FatigueIncrease", 0.0000345);
        zg.set_str("StressDecrease", 0.00003);
        zg.set_str("ImobileEnduranceIncrease", 0.000031);
        zg.set_str("EquippedOrWornEncumbranceMultiplier", 0.3);
        zg.set_str("CorpseSicknessFilterDrainMultiplier", 0.3);
        env.set_str("ZomboidGlobals", zg);

        let loaded = pz_lua_api::bind_character_sim::load_zomboid_globals(&env);
        assert_eq!(loaded.hunger_increase, 0.0000096);
        assert_eq!(loaded.stress_reduction, 0.00003);
        assert_eq!(loaded.imobile_endurance_reduce, 0.000031);
        assert_eq!(loaded.sitting_endurance_multiplier, 5.0);
        assert_eq!(loaded.equipped_or_worn_encumbrance_multiplier, 0.3);

        let sim = FrameSim::capture();
        assert_eq!(sim.zomboid_globals().hunger_increase, 0.0000096);
        assert_eq!(sim.player_context().zomboid_globals().thirst_increase, 0.0000080);
        assert_eq!(
            sim.player_context()
                .zomboid_globals()
                .corpse_sickness_filter_drain_multiplier,
            0.3
        );
        let item_env = pz_lua_api::bind_items::GameItemEnv::new(None);
        assert_eq!(item_env.equipped_or_worn_encumbrance_multiplier(), 0.3);
    }

    #[test]
    fn sim_context_reads_live_calendar_month_and_animal_sandbox() {
        let prev_time = pz_lua_api::bind_climate::shared_game_time();
        let time = Rc::new(RefCell::new(GameTime::default()));
        time.borrow_mut().month = 10;
        time.borrow_mut().time_of_day = 14.25;
        pz_lua_api::bind_climate::install_game_time(time);

        let frame = FrameSim::capture();
        assert_eq!(frame.month(), 10);
        assert_eq!(frame.player_context().month(), 10);
        assert!((frame.time_of_day() - 14.25).abs() < 0.001);
        let sandbox = frame.animal_sandbox();
        assert_eq!(sandbox.animal_stats_modifier, 4);
        assert!(sandbox.animal_mating_season);

        pz_lua_api::bind_climate::install_game_time(prev_time);
    }
}
