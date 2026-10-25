use super::*;
use pz_core::calendar::JavaZone;
use pz_core::weather::calendar::set_default_zone;
use pz_core::weather::{ClimateClock, SeasonConfig};
use pz_kahlua::TableRef;

fn setup() -> Thread {
    set_default_zone(Some(JavaZone::utc()));
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    let constants = vec![(
        CLIMATE_MANAGER_CLASS,
        vec![("FLOAT_TEMPERATURE".to_string(), Value::from(4))],
    )];
    register_with(&mut t, &constants);
    let mut gt = GameTime {
        day: 10,
        month: 0,
        year: 1994,
        multiplier: 20.0,
        helicopter_day1: -1,
        ..GameTime::default()
    };
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
    run(
        &mut t,
        "function assert(v, m) if not v then error(m or 'assertion failed') end return v end",
    );
    t
}

fn run(t: &mut Thread, src: &str) -> Vec<Value> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
    t.pcall(&f, &[]).map_err(|(m, _)| format!("{m:?}")).unwrap()
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
    let r = run(
        &mut t,
        "return ticks, getClimateManager():getTemperature(), lastT",
    );
    assert!(r[0].as_num().unwrap() > 0.0);
    let temp = r[1].as_num().unwrap();
    // The modded layer pins the temperature unless weather overrides it.
    assert!(
        temp == 12.5
            || shared_game_time()
                .borrow()
                .climate
                .weather_period
                .is_running,
        "{temp}"
    );
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
    let Some(path) = std::env::var_os("PZ_JAR") else {
        return;
    };
    let jar = Jar::open(std::path::Path::new(&path)).unwrap();
    let c = class_constants(&jar, "zombie/iso/weather/ClimateManager");
    let get = |n: &str| c.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_num());
    assert_eq!(get("FLOAT_TEMPERATURE"), Some(Some(4.0)));
    assert_eq!(get("MAX_WINDSPEED_KPH"), Some(Some(120.0)));
    let w = class_constants(&jar, "zombie/iso/weather/WeatherPeriod");
    assert!(w.iter().any(|(k, _)| k == "STAGE_KATEBOB_STORM"));
}
