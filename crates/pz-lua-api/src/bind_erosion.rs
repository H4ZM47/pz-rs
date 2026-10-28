//! Lua exposure of the erosion system (`zombie.erosion.ErosionMain`,
//! `ErosionSeason`, `ErosionConfig` and `ErosionConfig.Debug`) with the
//! method names the game's Lua calls.
//!
//! The engine hands the running [`ErosionMain`] to [`set_instance`] (what
//! `ErosionMain.getInstance()` returns) together with [`ErosionLuaHooks`]
//! for the calls that need the world (`LoadGridsquare`, `ChunkLoaded`,
//! `mainTimer`'s game date).

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_erosion::season::ErosionSeason;
use pz_erosion::ErosionMain;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, Thread, Value};

use crate::expose::{expose_class, int_arg, num_arg, with_this};

/// A callback taking a Lua value (a square or chunk object).
pub type ValueHook = Box<dyn Fn(&Value)>;
/// The current `GameTime` day, month and year.
pub type DateHook = Box<dyn Fn() -> (i32, i32, i32)>;

/// Engine callbacks behind the static `ErosionMain` functions.
#[derive(Default)]
pub struct ErosionLuaHooks {
    /// `ErosionMain.LoadGridsquare(square)`.
    pub load_grid_square: Option<ValueHook>,
    /// `ErosionMain.ChunkLoaded(chunk)`.
    pub chunk_loaded: Option<ValueHook>,
    /// `GameTime` day, month, year for `mainTimer()`.
    pub game_date: Option<DateHook>,
    /// `DebugUpdateMapNow()` (`updateMapNow` over the loaded chunks).
    pub update_map_now: Option<Box<dyn Fn()>>,
}

struct Instance {
    main: Rc<RefCell<ErosionMain>>,
    hooks: Rc<ErosionLuaHooks>,
}

thread_local! {
    static INSTANCE: RefCell<Option<Instance>> = const { RefCell::new(None) };
}

/// Makes `main` the `ErosionMain.instance` Lua sees (`None`: `Reset()`).
pub fn set_instance(main: Option<(Rc<RefCell<ErosionMain>>, Rc<ErosionLuaHooks>)>) {
    INSTANCE.with(|i| *i.borrow_mut() = main.map(|(main, hooks)| Instance { main, hooks }));
}

fn instance() -> Option<(Rc<RefCell<ErosionMain>>, Rc<ErosionLuaHooks>)> {
    INSTANCE.with(|i| i.borrow().as_ref().map(|i| (i.main.clone(), i.hooks.clone())))
}

pub struct LuaErosionMain {
    pub main: Rc<RefCell<ErosionMain>>,
    pub hooks: Rc<ErosionLuaHooks>,
}

impl LuaObject for LuaErosionMain {
    fn class_name(&self) -> &str {
        "zombie.erosion.ErosionMain"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Where an `ErosionSeason` Lua object lives: the running erosion's
/// season, or a standalone one (e.g. passed to `OnInitSeasons`).
#[derive(Clone)]
pub enum SeasonRef {
    Main(Rc<RefCell<ErosionMain>>),
    Own(Rc<RefCell<ErosionSeason>>),
}

impl SeasonRef {
    fn with<R>(&self, f: impl FnOnce(&mut ErosionSeason) -> R) -> R {
        match self {
            SeasonRef::Main(m) => f(&mut m.borrow_mut().season),
            SeasonRef::Own(s) => f(&mut s.borrow_mut()),
        }
    }
}

pub struct LuaErosionSeason(pub SeasonRef);

impl LuaObject for LuaErosionSeason {
    fn class_name(&self) -> &str {
        "zombie.erosion.season.ErosionSeason"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct LuaErosionConfig(pub Rc<RefCell<ErosionMain>>);

impl LuaObject for LuaErosionConfig {
    fn class_name(&self) -> &str {
        "zombie.erosion.ErosionConfig"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct LuaErosionDebug(pub Rc<RefCell<ErosionMain>>);

impl LuaObject for LuaErosionDebug {
    fn class_name(&self) -> &str {
        "zombie.erosion.ErosionConfig$Debug"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn obj(o: impl LuaObject) -> Value {
    Value::Object(Rc::new(o))
}

fn main_of(c: &NativeCall<'_>) -> pz_kahlua::LuaResult<(Rc<RefCell<ErosionMain>>, Rc<ErosionLuaHooks>)> {
    with_this::<LuaErosionMain, _>(c, |m| (m.main.clone(), m.hooks.clone()))
}

fn season_of(c: &NativeCall<'_>) -> pz_kahlua::LuaResult<SeasonRef> {
    with_this::<LuaErosionSeason, _>(c, |s| s.0.clone())
}

macro_rules! season_getters {
    ($b:expr, $($name:literal => $f:ident as $t:ty),* $(,)?) => {
        $(
            $b.method($name, |c| {
                let s = season_of(c)?;
                let v = s.with(|s| s.$f()) as $t;
                Ok(c.push(v))
            });
        )*
    };
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.erosion.ErosionMain", None);
    b.static_fn("getInstance", |c| {
        let v = instance().map_or(Value::Nil, |(main, hooks)| obj(LuaErosionMain { main, hooks }));
        Ok(c.push(v))
    });
    b.static_fn("LoadGridsquare", |c| {
        if let Some((_, hooks)) = instance() {
            if let Some(f) = &hooks.load_grid_square {
                f(&c.arg(0));
            }
        }
        Ok(0)
    });
    b.static_fn("ChunkLoaded", |c| {
        if let Some((_, hooks)) = instance() {
            if let Some(f) = &hooks.chunk_loaded {
                f(&c.arg(0));
            }
        }
        Ok(0)
    });
    b.static_fn("EveryTenMinutes", |_| {
        if let Some((main, hooks)) = instance() {
            main_timer(&main, &hooks);
        }
        Ok(0)
    });
    b.method("getEtick", |c| {
        let (m, _) = main_of(c)?;
        let v = m.borrow().get_etick();
        Ok(c.push(v))
    });
    b.method("getSeasons", |c| {
        let (m, _) = main_of(c)?;
        Ok(c.push(obj(LuaErosionSeason(SeasonRef::Main(m)))))
    });
    b.method("getConfig", |c| {
        let (m, _) = main_of(c)?;
        Ok(c.push(obj(LuaErosionConfig(m))))
    });
    b.method("mainTimer", |c| {
        let (m, h) = main_of(c)?;
        main_timer(&m, &h);
        Ok(0)
    });
    b.method("DebugUpdateMapNow", |c| {
        let (_, h) = main_of(c)?;
        if let Some(f) = &h.update_map_now {
            f();
        }
        Ok(0)
    });
    b.method("snowCheck", |c| {
        let (m, _) = main_of(c)?;
        m.borrow_mut().snow_check();
        Ok(0)
    });
    b.method("getSnowFraction", |c| {
        let (m, _) = main_of(c)?;
        let v = m.borrow().get_snow_fraction();
        Ok(c.push(v))
    });
    b.method("getSnowFractionYesterday", |c| {
        let (m, _) = main_of(c)?;
        let v = m.borrow().get_snow_fraction_yesterday();
        Ok(c.push(v))
    });
    b.method("isSnow", |c| {
        let (m, _) = main_of(c)?;
        let v = m.borrow().is_snow();
        Ok(c.push(v))
    });

    let b = expose_class(thread, "zombie.erosion.season.ErosionSeason", None);
    for (name, v) in [
        ("SEASON_DEFAULT", 0),
        ("SEASON_SPRING", 1),
        ("SEASON_SUMMER", 2),
        ("SEASON_SUMMER2", 3),
        ("SEASON_AUTUMN", 4),
        ("SEASON_WINTER", 5),
        ("NUM_SEASONS", 6),
    ] {
        b.field(name, v);
    }
    season_getters!(b,
        "getLat" => get_lat as i32,
        "getTempMax" => get_temp_max as i32,
        "getTempMin" => get_temp_min as i32,
        "getTempDiff" => get_temp_diff as i32,
        "getSeasonLag" => get_season_lag as i32,
        "getHighNoon" => get_high_noon as f64,
        "getSeedA" => get_seed_a as i32,
        "getSeedB" => get_seed_b as i32,
        "getSeedC" => get_seed_c as i32,
        "getCurDayPercent" => get_cur_day_percent as f64,
        "getMaxDaylightWinter" => get_max_daylight_winter as f64,
        "getMaxDaylightSummer" => get_max_daylight_summer as f64,
        "getDusk" => get_dusk as f64,
        "getDawn" => get_dawn as f64,
        "getDaylight" => get_daylight as f64,
        "getDayTemperature" => get_day_temperature as f64,
        "getDayMeanTemperature" => get_day_mean_temperature as f64,
        "getSeason" => get_season as i32,
        "getDayHighNoon" => get_day_high_noon as f64,
        "getSeasonDay" => get_season_day as f64,
        "getSeasonDays" => get_season_days as f64,
        "getSeasonStrength" => get_season_strength as f64,
        "getSeasonProgression" => get_season_progression as f64,
        "getDayNoiseVal" => get_day_noise_val as f64,
        "isRainDay" => is_rain_day as bool,
        "getRainDayStrength" => get_rain_day_strength as f64,
        "getRainYearAverage" => get_rain_year_average as f64,
        "isThunderDay" => is_thunder_day as bool,
        "isSunnyDay" => is_sunny_day as bool,
        "isEndlessDay" => is_endless_day as bool,
        "isEndlessNight" => is_endless_night as bool,
    );
    b.method("getSeasonName", |c| {
        let s = season_of(c)?;
        let v = s.with(|s| s.get_season_name().to_string());
        Ok(c.push(v))
    });
    b.method("getSeasonNameTranslated", |c| {
        let s = season_of(c)?;
        let v = s.with(|s| s.get_season_name_translated().to_string());
        Ok(c.push(v))
    });
    b.method("isSeason", |c| {
        let s = season_of(c)?;
        let n = int_arg(c, 1);
        let v = s.with(|s| s.is_season(n));
        Ok(c.push(v))
    });
    b.method("setCurSeason", |c| {
        let s = season_of(c)?;
        let n = int_arg(c, 1);
        s.with(|s| s.set_cur_season(n));
        Ok(0)
    });
    b.method("setDay", |c| {
        let s = season_of(c)?;
        let (d, m, y) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        s.with(|s| s.set_day(d, m, y));
        Ok(0)
    });
    b.method("init", |c| {
        let s = season_of(c)?;
        let a: Vec<i32> = (1..=9).map(|i| int_arg(c, i)).collect();
        let noon = num_arg(c, 6) as f32;
        s.with(|s| s.init(a[0], a[1], a[2], a[3], a[4], noon, a[6], a[7], a[8]));
        Ok(0)
    });
    b.method("setRain", |c| {
        let s = season_of(c)?;
        let mut rain = [0f32; 12];
        for (i, r) in rain.iter_mut().enumerate() {
            *r = num_arg(c, i + 1) as f32;
        }
        s.with(|s| s.set_rain(rain));
        Ok(0)
    });
    b.method("clone", |c| {
        let s = season_of(c)?;
        let copy = s.with(|s| s.clone_season());
        Ok(c.push(obj(LuaErosionSeason(SeasonRef::Own(Rc::new(RefCell::new(copy)))))))
    });

    let b = expose_class(thread, "zombie.erosion.ErosionConfig", None);
    b.method("getDebug", |c| {
        let m = with_this::<LuaErosionConfig, _>(c, |x| x.0.clone())?;
        Ok(c.push(obj(LuaErosionDebug(m))))
    });
    let b = expose_class(thread, "zombie.erosion.ErosionConfig$Debug", None);
    b.method("getEnabled", |c| {
        let v = with_this::<LuaErosionDebug, _>(c, |x| x.0.borrow().cfg.debug.enabled)?;
        Ok(c.push(v))
    });
    b.method("getStartDay", |c| {
        let v = with_this::<LuaErosionDebug, _>(c, |x| x.0.borrow().cfg.debug.startday)?;
        Ok(c.push(v))
    });
    b.method("getStartMonth", |c| {
        let v = with_this::<LuaErosionDebug, _>(c, |x| x.0.borrow().cfg.debug.startmonth)?;
        Ok(c.push(v))
    });
}

fn main_timer(main: &Rc<RefCell<ErosionMain>>, hooks: &ErosionLuaHooks) {
    let mut m = main.borrow_mut();
    let (d, mo, y) = hooks.game_date.as_ref().map_or((m.day, m.month, m.year), |f| f());
    m.main_timer(d, mo, y);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    fn run(t: &mut Thread, src: &str) -> Vec<Value> {
        let f = stdlib::load_string(src, Some("test"), t.globals().clone()).expect("compile");
        t.call(&f, &[]).unwrap_or_else(|e| panic!("runtime error: {e}"))
    }

    #[test]
    fn season_methods_from_lua() {
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        set_instance(None);
        assert!(run(&mut t, "return ErosionMain.getInstance()")[0].is_nil());
        // A synthetic permutation table; no game data.
        let perm: std::sync::Arc<[i32]> = (0..256).map(|i| (i * 37 + 11) % 256).collect::<Vec<i32>>().into();
        let names = (0..6).map(|i| format!("n{i}")).collect();
        let season = Rc::new(RefCell::new(ErosionSeason::new(perm, names, None)));
        t.globals().set_str("S", obj(LuaErosionSeason(SeasonRef::Own(season.clone()))));
        let r = run(
            &mut t,
            "S:init(50, 20, -10, 7, S:getSeasonLag(), 12.5, 1, 2, 3)
             S:setRain(0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1,0.1)
             S:setDay(15, 0, 1994)
             return S:getLat(), S:getSeason(), S:isSeason(S:getSeason()), S:getSeasonName(), ErosionSeason.SEASON_WINTER",
        );
        assert_eq!(r[0].as_num(), Some(50.0));
        let cur = season.borrow().get_season();
        assert_eq!(r[1].as_num(), Some(cur as f64));
        assert!(matches!(r[2], Value::Bool(true)));
        assert_eq!(r[3].as_str().map(|s| s.to_string()), Some(format!("n{cur}")));
        assert_eq!(r[4].as_num(), Some(5.0));
    }
}
