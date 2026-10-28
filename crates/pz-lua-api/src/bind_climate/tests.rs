use super::*;
use pz_core::calendar::JavaZone;
use pz_core::weather::calendar::set_default_zone;
use pz_core::weather::{ClimateClock, SeasonConfig};
use pz_kahlua::TableRef;

/// The per-thread state the climate reads: the time zone, the climate
/// settings and a fixed `Rand` sequence. `new ClimateManager()`, the
/// forecaster and the weather draw from `RandStandard`, which is seeded
/// from the wall clock; unseeded, about one run in twenty started a
/// weather period that lit (or darkened) the global light the tests check.
fn reset_climate_state() {
    set_default_zone(Some(JavaZone::utc()));
    set_climate_settings(ClimateSettings::default());
    pz_core::random::with_standard(|r| r.set_seed(1994));
}

fn setup() -> Thread {
    reset_climate_state();
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    let constants = vec![(CLIMATE_MANAGER_CLASS, vec![("FLOAT_TEMPERATURE".to_string(), Value::from(4))])];
    register_with(&mut t, &constants);
    let mut gt = GameTime { day: 10, month: 0, year: 1994, multiplier: 20.0, helicopter_day1: -1, ..GameTime::default() };
    gt.climate = ClimateManager::with_offsets([10.0, 9000.0, -10.0, -9000.0]);
    gt.set_minutes_stamp();
    let season = ErosionSeason::from_config(&SeasonConfig::default(), 3, gt.day, gt.month, gt.year);
    let mut ctx = LuaClimateContext::new();
    let clock: ClimateClock = gt.clock(&ctx);
    gt.climate.init(season, None, &clock, &mut ctx);
    install_game_time(Rc::new(RefCell::new(gt)));
    install_event_sink(Rc::new(move |t: &mut Thread, name: &str, _a: &[Value]| {
        let f = t.globals().get_str("onEvent");
        if !matches!(f, Value::Nil) {
            let _ = t.pcall(&f, &[Value::from(name)]);
        }
    }));
    run(&mut t, "function assert(v, m) if not v then error(m or 'assertion failed') end return v end");
    t
}

fn run(t: &mut Thread, src: &str) -> Vec<Value> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
    t.pcall(&f, &[]).map_err(|(m, _)| format!("{m:?}")).unwrap()
}

#[test]
fn session_reset_retires_climate_wrappers_without_replacing_game_time() {
    let _thread = setup();
    let game_time = shared_game_time();
    let table = game_time.borrow_mut().mod_data();
    table.set_str("clockData", 42.0);
    let old_handle = handle(Obj::GameTime);
    let color = Rc::new(RefCell::new(ClimateColorInfo::default()));
    let color_weak = Rc::downgrade(&color);
    let wrapped_color = color_value(&color);
    drop(wrapped_color);
    drop(color);
    assert!(color_weak.upgrade().is_some());
    let sink_owner = Rc::new(());
    let sink_weak = Rc::downgrade(&sink_owner);
    install_event_sink(Rc::new(move |_, _, _| {
        let _keep_alive = sink_owner.clone();
    }));

    crate::session_reset::reset_session_bindings();
    assert!(sink_weak.upgrade().is_none());
    assert!(color_weak.upgrade().is_none(), "old climate color wrapper survived reset");
    assert!(!old_handle.raw_equals(&handle(Obj::GameTime)));
    assert!(Rc::ptr_eq(&game_time, &shared_game_time()), "reset replaced simulation time");
    assert!(table.ptr_eq(&game_time.borrow_mut().mod_data()));
    assert_eq!(table.get_str("clockData").as_num(), Some(42.0));
}

#[test]
fn clock_and_climate_from_lua() {
    let mut t = setup();
    for _ in 0..400 {
        update_game_time(&mut t);
    }
    let r = run(
        &mut t,
        r#"
        ticks = 0
        function onEvent(name)
            if name == "OnClimateTick" then
                ticks = ticks + 1
                lastT = getClimateManager():getTemperature()
            end
        end
        local gt = getGameTime()
        local cm = getClimateManager()
        assert(gt == GameTime.getInstance() and cm == ClimateManager.getInstance())
        local f = cm:getClimateFloat(ClimateManager.FLOAT_TEMPERATURE)
        assert(f:getName() == "TEMPERATURE" and f:getMin() == -80)
        f:setModdedValue(12.5); f:setModdedInterpolate(1); f:setEnableModded(true)
        local c = cm:getColNight()
        c:setExterior(0.1, 0.2, 0.3, 0.4)
        assert(cm:getColNight() == c)
        return gt:getYear(), gt:getMonth(), cm:getSeason():getSeasonName(),
            cm:getClimateForecaster():getForecasts():size(), cm:getHumidity() >= 0,
            getClimateMoon():getPhaseName(), cm:getClimateBool(5)
        "#,
    );
    assert_eq!(r[0].as_num(), Some(1994.0));
    assert_eq!(r[2].as_str(), Some("Winter"));
    assert_eq!(r[3].as_num(), Some(40.0));
    assert!(r[4].truthy());
    assert!(r[5].as_str().is_some());
    assert!(matches!(r[6], Value::Nil));
    for _ in 0..300 {
        update_game_time(&mut t);
    }
    let r = run(&mut t, "return ticks, getClimateManager():getTemperature(), lastT");
    assert!(r[0].as_num().unwrap() > 0.0);
    let temp = r[1].as_num().unwrap();
    // The modded layer pins the temperature unless weather overrides it.
    assert!(temp == 12.5 || shared_game_time().borrow().climate.weather_period.is_running, "{temp}");
    assert!(r[2].as_num().is_some());
}

#[test]
fn weather_from_lua() {
    let mut t = setup();
    update_game_time(&mut t);
    let r = run(
        &mut t,
        r#"
        local cm = getClimateManager()
        assert(cm:triggerCustomWeatherStage(3, 5))
        local wp = cm:getWeatherPeriod()
        local stages = wp:getWeatherStages()
        local st = wp:getCurrentStage()
        local ids = {}
        for i = 0, stages:size() - 1 do ids[#ids + 1] = stages:get(i):getStageID() end
        local id = st:getStageID()
        cm:stopWeatherAndThunder()
        return wp:isRunning(), table.concat(ids, ","), id, wp:getCloudColor() ~= nil,
            cm:getThunderStorm():getClouds():size(), Temperature.CelsiusToFahrenheit(100)
        "#,
    );
    assert!(!r[0].truthy());
    assert_eq!(r[1].as_str(), Some("0,3,4"));
    assert_eq!(r[2].as_num(), Some(0.0));
    assert!(r[3].truthy());
    assert_eq!(r[4].as_num(), Some(3.0));
    assert_eq!(r[5].as_num(), Some(212.0));
}

#[test]
fn modded_period_events_see_the_stage() {
    let mut t = setup();
    update_game_time(&mut t);
    let r = run(
        &mut t,
        r#"
        local seen
        function onEvent(name) if name == "OnInitModdedWeatherStage" then seen = true end end
        local wp = getClimateManager():getWeatherPeriod()
        assert(wp:startCreateModdedPeriod(true, 0.8, 120))
        local s = wp:createAndAddModdedStage("MyMod", 3)
        s:setTargetStrength(0.7)
        s:lerpEntryTo(2, 2)
        local ok = wp:endCreateModdedPeriod()
        return ok, seen, s:getModID(), wp:isRunning()
        "#,
    );
    assert!(r[0].truthy());
    assert!(r[1].truthy());
    assert_eq!(r[2].as_str(), Some("MyMod"));
    assert!(r[3].truthy());
}

/// With `PZ_JAR` pointing at `projectzomboid.jar`, the class constants come
/// from it.
#[test]
fn constants_from_the_jar() {
    let Some(path) = std::env::var_os("PZ_JAR") else { return };
    let jar = Jar::open(std::path::Path::new(&path)).unwrap();
    let c = class_constants(&jar, "zombie/iso/weather/ClimateManager");
    let get = |n: &str| c.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_num());
    assert_eq!(get("FLOAT_TEMPERATURE"), Some(Some(4.0)));
    assert_eq!(get("MAX_WINDSPEED_KPH"), Some(Some(120.0)));
    let w = class_constants(&jar, "zombie/iso/weather/WeatherPeriod");
    assert!(w.iter().any(|(k, _)| k == "STAGE_KATEBOB_STORM"));
}

/// A Lua state with an `Events` table like the game's (`Events.X.Add`) that
/// the event sink dispatches to, and a fresh `GameTime` at noon.
fn world_load_setup() -> Thread {
    reset_climate_state();
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    register_with(&mut t, &[]);
    let gt = GameTime { day: 10, month: 6, year: 1993, time_of_day: 12.0, multiplier: 20.0, helicopter_day1: -1, ..GameTime::default() };
    install_game_time(Rc::new(RefCell::new(gt)));
    install_event_sink(Rc::new(move |t: &mut Thread, name: &str, args: &[Value]| {
        let f = t.globals().get_str("triggerEvent");
        let mut a = vec![Value::from(name)];
        a.extend_from_slice(args);
        t.pcall(&f, &a).map_err(|(m, _)| format!("{m:?}")).unwrap();
    }));
    run(
        &mut t,
        r#"
        function assert(v, m) if not v then error(m or 'assertion failed') end return v end
        Events = setmetatable({}, { __index = function(t, k)
            local e = { list = {} }
            e.Add = function(f) table.insert(e.list, f) end
            rawset(t, k, e)
            return e
        end })
        function triggerEvent(name, ...)
            local l = Events[name].list
            for i = 1, #l do l[i](...) end
        end
        "#,
    );
    t
}

/// `new ClimateManager()` → `init` → frames, as a world load runs them.
fn load_climate(t: &mut Thread) -> [f32; 4] {
    new_climate_manager(t);
    start_erosion_season(t, &SeasonConfig::default(), 3);
    assert!(init_climate_manager(t, Some((0, 0, 60, 50))));
    for _ in 0..30 {
        update_game_time(t);
    }
    let v = shared_game_time().borrow().climate.render_values(0, false);
    v.cm_global_light.exterior
}

#[test]
fn climate_manager_init_runs_the_lua_season_colours() {
    // Nobody sets the season colours: the global light stays (nearly)
    // black.
    let mut t = world_load_setup();
    let dark = load_climate(&mut t);
    assert!(dark[..3].iter().all(|&c| c < 0.01), "{dark:?}");

    // A handler like ClimateMain.lua's (invented colours).
    let mut t = world_load_setup();
    run(
        &mut t,
        r#"
        seasons = 0
        Events.OnInitSeasons.Add(function(s) seasons = seasons + 1; seasonName = s:getSeasonName() end)
        Events.OnClimateManagerInit.Add(function(clim)
            initCalls = (initCalls or 0) + 1
            local c = clim:getColNightNoMoon()
            c:setExterior(0.25, 0.25, 0.25, 0.8)
            for temp = 0, 2 do
                for season = 0, 3 do
                    clim:setSeasonColorDawn(temp, season, 0.5, 0.6, 0.7, 0.75, true)
                    clim:setSeasonColorDawn(temp, season, 0.1, 0.2, 0.3, 0.3, false)
                    clim:setSeasonColorDay(temp, season, 0.8, 0.75, 0.7, 0.8, true)
                    clim:setSeasonColorDay(temp, season, 0.4, 0.3, 0.2, 0.2, false)
                    clim:setSeasonColorDusk(temp, season, 0.9, 0.45, 0.2, 0.8, true)
                    clim:setSeasonColorDusk(temp, season, 0.2, 0.1, 0.05, 0.6, false)
                end
            end
        end)
        "#,
    );
    let lit = load_climate(&mut t);
    let r = run(&mut t, "return initCalls, seasons, seasonName");
    assert_eq!(r[0].as_num(), Some(1.0));
    assert_eq!(r[1].as_num(), Some(1.0));
    assert_eq!(r[2].as_str(), Some("Early Summer"));
    assert!(lit[..3].iter().all(|&c| c > 0.1), "{lit:?}");
    let g = shared_game_time();
    let g = g.borrow();
    assert_eq!(g.climate.thunder_storm.map_max_x, 60 * 256 + 4000);
    assert_eq!(g.climate.climate_forecaster.forecasts.len(), 40);
    assert_eq!(g.climate.col_night_no_moon.borrow().exterior, [0.25, 0.25, 0.25, 0.8]);
}

#[test]
fn post_cell_load_snow_and_night() {
    let mut t = setup();
    update_game_time(&mut t);
    take_snow_effects();
    {
        let gt = shared_game_time();
        let mut g = gt.borrow_mut();
        g.climate.snow_frac_now = 0.57;
        g.climate.can_do_winter_sprites = true;
    }
    let e = post_cell_load_set_snow(&mut t);
    assert_eq!(e, SnowEffects { target: Some(57), ice_queen: Some(true) });
    assert_eq!(take_snow_effects(), SnowEffects::default());
    shared_game_time().borrow_mut().climate.snow_frac_now = 0.2;
    let r = run(&mut t, "getClimateManager():postCellLoadSetSnow(); return getGameTime():getNight(), getClimateManager():getNightStrength()");
    assert_eq!(take_snow_effects(), SnowEffects { target: Some(20), ice_queen: Some(false) });
    assert_eq!(r[0].as_num(), r[1].as_num());
    let night = shared_game_time().borrow().get_night();
    assert_eq!(r[0].as_num(), Some(f64::from(night)));
}

fn puddles_setup() -> (Thread, pz_fs::GameFs) {
    let mut t = setup();
    crate::bind_puddles::register(&mut t);
    crate::bind_puddles::shared_puddles().borrow_mut().state = Default::default();
    crate::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    let fs = pz_fs::GameFs::new(std::env::temp_dir().join("pz-climate-puddles-missing-assets"));
    let gt = shared_game_time();
    let mut g = gt.borrow_mut();
    use pz_core::weather::manager::{BOOL_IS_SNOW, FLOAT_PRECIPITATION_INTENSITY};
    g.climate.vars.floats[FLOAT_PRECIPITATION_INTENSITY].set_enable_admin(true);
    g.climate.vars.floats[FLOAT_PRECIPITATION_INTENSITY].set_admin_value(1.0);
    g.climate.vars.bools[BOOL_IS_SNOW].set_enable_admin(true);
    g.climate.vars.bools[BOOL_IS_SNOW].admin_value = false;
    // Subzero temperatures add ground moisture in the original equation;
    // pin positive temperature so the final phase exercises dry ground.
    g.climate.vars.floats[pz_core::weather::manager::FLOAT_TEMPERATURE].set_enable_admin(true);
    g.climate.vars.floats[pz_core::weather::manager::FLOAT_TEMPERATURE].set_admin_value(10.0);
    drop(g);
    (t, fs)
}

fn puddle_step() -> f32 {
    let gt = shared_game_time();
    let g = gt.borrow();
    let rain = g.climate.get_rain_intensity();
    rain * rain * 0.05 * g.get_thirty_fps_multiplier(&LuaClimateContext::new()) * 0.01
}

#[test]
fn puddles_require_real_weather_fx_advance_once_and_obey_disabled_paths() {
    let (mut t, fs) = puddles_setup();
    let owner = crate::bind_puddles::shared_puddles();
    update_game_time(&mut t);
    assert_eq!(owner.borrow().state, Default::default(), "cell alone bypassed WeatherFX gate");
    assert!(crate::bind_puddles::init_weather_fx(&fs));
    update_game_time(&mut t);
    let first = owner.borrow().state.puddles_size.get();
    assert!((first - puddle_step()).abs() < 1e-6, "{first}");
    assert!(first > 0.0);
    update_game_time(&mut t);
    assert!((owner.borrow().state.puddles_size.get() - first - puddle_step()).abs() < 1e-6);

    let frozen = owner.borrow().state;
    shared_game_time().borrow_mut().climate.disable_fx_update = true;
    update_game_time(&mut t);
    assert_eq!(owner.borrow().state, frozen);
    shared_game_time().borrow_mut().climate.disable_fx_update = false;
    shared_game_time().borrow_mut().climate.disable_simulation = true;
    update_game_time(&mut t);
    assert_eq!(owner.borrow().state, frozen);

    let gt = shared_game_time();
    {
        let mut g = gt.borrow_mut();
        g.climate.disable_simulation = false;
        g.climate.vars.floats[pz_core::weather::manager::FLOAT_PRECIPITATION_INTENSITY].set_admin_value(0.0);
    }
    owner.borrow_mut().state.wet_ground.set(0.0);
    owner.borrow_mut().state.puddles_size.set(0.1);
    update_game_time(&mut t);
    let drying = 2.0e-5 * gt.borrow().get_thirty_fps_multiplier(&LuaClimateContext::new());
    assert!((owner.borrow().state.puddles_size.get() - (0.1 - drying)).abs() < 1e-6);
    crate::bind_world::set_cell(None);
}

#[test]
fn puddles_sample_live_climate_after_debug_callbacks() {
    let (mut t, fs) = puddles_setup();
    assert!(crate::bind_puddles::init_weather_fx(&fs));
    crate::bind_core::shared_core().borrow_mut().debug = true;
    shared_game_time().borrow_mut().climate.last_minute_stamp = -1;
    t.globals().set_str("RAIN_ID", pz_core::weather::manager::FLOAT_PRECIPITATION_INTENSITY as i32);
    run(&mut t, r#"
        debugTicks = 0
        function onEvent(name)
            if name == "OnClimateTickDebug" then
                debugTicks = debugTicks + 1
                before = getPuddlesManager():getPuddlesSize()
                getPuddlesManager():getPuddlesFloat(3):setFinalValue(0.2)
                getClimateManager():getClimateFloat(RAIN_ID):setFinalValue(0.4)
            end
        end
    "#);
    update_game_time(&mut t);
    assert_eq!(t.globals().get_str("debugTicks").as_num(), Some(1.0));
    assert_eq!(t.globals().get_str("before").as_num(), Some(0.0));
    let owner = crate::bind_puddles::shared_puddles();
    assert!((owner.borrow().state.rain.get() - 0.4).abs() < 1e-6);
    assert!((owner.borrow().state.puddles_size.get() - (0.2 + puddle_step())).abs() < 1e-6);
    let fx = crate::bind_world::with_world(|w| w.weather_fx.clone().unwrap());
    assert!((fx.borrow().precipitation_intensity.target() - 0.43).abs() < 1e-6);
    crate::bind_world::set_cell(None);
}

#[test]
fn debug_callback_can_disable_puddle_fx_before_current_frame_advances() {
    let (mut t, fs) = puddles_setup();
    assert!(crate::bind_puddles::init_weather_fx(&fs));
    crate::bind_core::shared_core().borrow_mut().debug = true;
    shared_game_time().borrow_mut().climate.last_minute_stamp = -1;
    run(&mut t, r#"
        function onEvent(name)
            if name == "OnClimateTickDebug" then
                getClimateManager():setEnabledFxUpdate(false)
            end
        end
    "#);
    update_game_time(&mut t);
    assert!(shared_game_time().borrow().climate.disable_fx_update);
    assert_eq!(crate::bind_puddles::shared_puddles().borrow().state, Default::default());
    crate::bind_world::set_cell(None);
}

#[test]
fn rain_manager_statics_from_lua() {
    let mut t = setup();
    let r = run(
        &mut t,
        "events = {} \
         function onEvent(n) table.insert(events, n) end \
         assert(RainManager.isRaining() == getClimateManager():isRaining()) \
         assert(RainManager.getRainIntensity() == getClimateManager():getPrecipitationIntensity()) \
         RainManager.setRandRainMin(3) RainManager.setRandRainMax(9) \
         RainManager.startRaining() \
         RainManager.stopRaining() \
         return events[1], #events",
    );
    assert_eq!(r[0].as_str(), Some("OnRainStop"));
    assert_eq!(r[1].as_num(), Some(1.0));
    assert_eq!(
        rain_statics(),
        RainStatics { rand_rain: 9, rand_rain_min: 3, rand_rain_max: 9, rain_desired_intensity: 0.0, stop_rain: true }
    );
    reset_rain_manager();
    assert!(!rain_statics().stop_rain);
}

#[test]
fn rain_manager_fresh_class_state_matches_installed_initializer() {
    // A fresh thread exercises the real thread-local producer, without
    // inheriting another test's stopRaining mutations.
    let initial = std::thread::spawn(rain_statics).join().unwrap();
    assert_eq!(initial, RainStatics {
        rand_rain: 0, rand_rain_min: 0, rand_rain_max: 0,
        rain_desired_intensity: 1.0, stop_rain: false,
    });
}
