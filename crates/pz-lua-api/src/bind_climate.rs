//! `zombie.GameTime` and `zombie.iso.weather` exposed to Lua under the
//! original names: `getGameTime()`, `getClimateManager()`,
//! `getClimateMoon()`, `ClimateManager` with its `ClimateFloat`,
//! `ClimateColor`, `ClimateBool`, `AirFront` and `DayInfo`, `WeatherPeriod`
//! and `WeatherStage`, `ThunderStorm` and `ThunderCloud`,
//! `ClimateForecaster` with `DayForecast` and `ForecastValue`,
//! `ClimateValues`, `ClimateColorInfo`, `ClimateMoon`, `ClimateHistory`,
//! `ErosionSeason` and `Temperature`.
//!
//! All objects are views into one shared [`GameTime`] (and the climate it
//! owns). The `public static final` constants of these classes are read
//! from the installed `projectzomboid.jar`.
//!
//! Events the climate raises while Lua called into it (or while the engine
//! runs [`update_game_time`]) fire after the call returns, so handlers can
//! use the climate again; `update` fires `OnClimateTick` between its two
//! halves, as the original does mid-update.
//!
//! Not exposed: the network `transmit*` calls are no-ops (single player),
//! `getAirTemperatureForCharacter`/`getAirTemperatureForSquare` and
//! `Temperature.getWindChillAmountForPlayer` (TODO(IsoGridSquare)),
//! `GameTime.getCalender` (TODO(PZCalendar)), `ErosionSeason.getWinterStartDay`.
//! `ClimateColorInfo.getInterior`/`getExterior` return a copy of the colour.

use crate::bind_java_util::{class_cast, list_value};
use crate::expose::{bool_arg, expose_class, expose_enum, global, int_arg, num_arg, str_arg, this, JavaEnum};
use crate::LuaManager;
use pz_classfile::class::Constant;
use pz_classfile::Jar;
use pz_core::game_time::{GameTime, GameTimeContext, GameTimeEvent};
use pz_core::weather::manager::{AirFront, FLOAT_MAX};
use pz_core::weather::{
    temperature, ClimateColorInfo, ClimateContext, ClimateEvent, ClimateManager, ClimateValues, ColorRef,
    ErosionSeason, PeriodId, SeasonConfig, WeatherPeriod,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const GAME_TIME_CLASS: &str = "zombie.GameTime";
pub const CLIMATE_MANAGER_CLASS: &str = "zombie.iso.weather.ClimateManager";
pub const CLIMATE_FLOAT_CLASS: &str = "zombie.iso.weather.ClimateManager$ClimateFloat";
pub const CLIMATE_COLOR_CLASS: &str = "zombie.iso.weather.ClimateManager$ClimateColor";
pub const CLIMATE_BOOL_CLASS: &str = "zombie.iso.weather.ClimateManager$ClimateBool";
pub const AIR_FRONT_CLASS: &str = "zombie.iso.weather.ClimateManager$AirFront";
pub const DAY_INFO_CLASS: &str = "zombie.iso.weather.ClimateManager$DayInfo";
pub const WEATHER_PERIOD_CLASS: &str = "zombie.iso.weather.WeatherPeriod";
pub const WEATHER_STAGE_CLASS: &str = "zombie.iso.weather.WeatherPeriod$WeatherStage";
pub const STR_LERP_VAL_CLASS: &str = "zombie.iso.weather.WeatherPeriod$StrLerpVal";
pub const THUNDER_STORM_CLASS: &str = "zombie.iso.weather.ThunderStorm";
pub const THUNDER_CLOUD_CLASS: &str = "zombie.iso.weather.ThunderStorm$ThunderCloud";
pub const CLIMATE_FORECASTER_CLASS: &str = "zombie.iso.weather.ClimateForecaster";
pub const DAY_FORECAST_CLASS: &str = "zombie.iso.weather.ClimateForecaster$DayForecast";
pub const FORECAST_VALUE_CLASS: &str = "zombie.iso.weather.ClimateForecaster$ForecastValue";
pub const CLIMATE_HISTORY_CLASS: &str = "zombie.iso.weather.ClimateHistory";
pub const CLIMATE_MOON_CLASS: &str = "zombie.iso.weather.ClimateMoon";
pub const CLIMATE_VALUES_CLASS: &str = "zombie.iso.weather.ClimateValues";
pub const CLIMATE_COLOR_INFO_CLASS: &str = "zombie.iso.weather.ClimateColorInfo";
pub const TEMPERATURE_CLASS: &str = "zombie.iso.weather.Temperature";
pub const EROSION_SEASON_CLASS: &str = "zombie.erosion.season.ErosionSeason";

/// The engine state the climate reads that has no port yet (sandbox
/// options, performance settings, players), with a fresh single-player
/// game's defaults. The owner updates it with [`set_climate_settings`].
#[derive(Clone, Debug, PartialEq)]
pub struct ClimateSettings {
    pub climate_cycle: i32,
    pub fog_cycle: i32,
    pub rain_modifier: i32,
    pub day_night_cycle: i32,
    pub fog_quality: i32,
    pub weather_shader: bool,
    pub meta_event: i32,
    pub helicopter: i32,
    pub lock_fps: i32,
    pub game_paused: bool,
    pub all_players_asleep: bool,
    pub game_speed: i32,
    /// `SandboxOptions.getDayLengthMinutes()` and its default.
    pub day_length_minutes: (i32, i32),
    pub players: [Option<(f32, f32)>; 4],
}

impl Default for ClimateSettings {
    fn default() -> Self {
        ClimateSettings {
            climate_cycle: 1,
            fog_cycle: 1,
            rain_modifier: 3,
            day_night_cycle: 1,
            fog_quality: 0,
            weather_shader: false,
            meta_event: 2,
            helicopter: 2,
            lock_fps: 60,
            game_paused: false,
            all_players_asleep: false,
            game_speed: 1,
            day_length_minutes: (60, 60),
            players: [None; 4],
        }
    }
}

/// `LuaEventManager.triggerEvent(name, args)`.
pub type EventSink = Rc<dyn Fn(&mut Thread, &str, &[Value])>;

thread_local! {
    static GAME_TIME: RefCell<Option<Rc<RefCell<GameTime>>>> = const { RefCell::new(None) };
    static SETTINGS: RefCell<ClimateSettings> = RefCell::new(ClimateSettings::default());
    static SINK: RefCell<Option<EventSink>> = const { RefCell::new(None) };
    static HANDLES: RefCell<HashMap<Obj, Value>> = RefCell::new(HashMap::new());
    static COLORS: RefCell<HashMap<usize, (ColorRef, Value)>> = RefCell::new(HashMap::new());
    static SNOW: RefCell<SnowEffects> = const { RefCell::new(SnowEffects { target: None, ice_queen: None }) };
}

/// The latest climate effects on the world that Lua calls and the climate
/// updates produced and the engine has not applied yet:
/// `IsoWorld.currentCell.setSnowTarget(target)` (before its
/// `enableSnowOnGround` check) and `ErosionIceQueen.instance.setSnow(b)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnowEffects {
    pub target: Option<i32>,
    pub ice_queen: Option<bool>,
}

/// Takes the pending [`SnowEffects`] (the engine applies them to the cell
/// and the ice queen).
pub fn take_snow_effects() -> SnowEffects {
    SNOW.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

/// Makes `gt` the `GameTime.instance` Lua sees.
pub fn install_game_time(gt: Rc<RefCell<GameTime>>) {
    GAME_TIME.with(|g| *g.borrow_mut() = Some(gt));
}

/// The shared `GameTime` (created on first use).
pub fn shared_game_time() -> Rc<RefCell<GameTime>> {
    GAME_TIME.with(|g| g.borrow_mut().get_or_insert_with(|| Rc::new(RefCell::new(GameTime::default()))).clone())
}

pub fn set_climate_settings(s: ClimateSettings) {
    SETTINGS.with(|c| *c.borrow_mut() = s);
}

pub fn climate_settings() -> ClimateSettings {
    SETTINGS.with(|c| c.borrow().clone())
}

/// Where climate and clock events go (`LuaEventManager.triggerEvent`).
pub fn install_event_sink(sink: EventSink) {
    SINK.with(|s| *s.borrow_mut() = Some(sink));
}

/// The context the bound climate runs in: [`ClimateSettings`], the shared
/// `Core`, the global `Rand`; events are collected and fired afterwards.
#[derive(Default)]
pub struct LuaClimateContext {
    settings: ClimateSettings,
    pub events: Vec<ClimateEvent>,
    pub clock_events: Vec<GameTimeEvent>,
}

impl LuaClimateContext {
    pub fn new() -> Self {
        LuaClimateContext { settings: climate_settings(), events: Vec::new(), clock_events: Vec::new() }
    }
}

impl ClimateContext for LuaClimateContext {
    fn climate_cycle(&self) -> i32 {
        self.settings.climate_cycle
    }
    fn fog_cycle(&self) -> i32 {
        self.settings.fog_cycle
    }
    fn rain_modifier(&self) -> i32 {
        self.settings.rain_modifier
    }
    fn day_night_cycle(&self) -> i32 {
        self.settings.day_night_cycle
    }
    fn fog_quality(&self) -> i32 {
        self.settings.fog_quality
    }
    fn weather_shader_active(&self) -> bool {
        self.settings.weather_shader
    }
    fn is_debug(&self) -> bool {
        crate::bind_core::shared_core().borrow().debug
    }
    fn game_mode(&self) -> String {
        crate::bind_core::shared_core().borrow().get_game_mode()
    }
    fn is_force_snow(&self) -> bool {
        crate::bind_core::shared_core().borrow().is_force_snow()
    }
    fn set_force_snow(&mut self, b: bool) {
        crate::bind_core::shared_core().borrow_mut().set_force_snow(b);
    }
    fn players(&self) -> [Option<(f32, f32)>; 4] {
        self.settings.players
    }
    fn current_game_speed(&self) -> i32 {
        self.settings.game_speed
    }
    fn disable_lightning_during_storms(&self) -> bool {
        crate::bind_core::shared_core().borrow().get_option_disable_lightning_during_storms()
    }
    fn event(&mut self, e: ClimateEvent) {
        self.events.push(e);
    }
}

impl GameTimeContext for LuaClimateContext {
    fn meta_event(&self) -> i32 {
        self.settings.meta_event
    }
    fn helicopter(&self) -> i32 {
        self.settings.helicopter
    }
    fn is_game_paused(&self) -> bool {
        self.settings.game_paused
    }
    fn all_players_asleep(&self) -> bool {
        self.settings.all_players_asleep
    }
    fn lock_fps(&self) -> i32 {
        self.settings.lock_fps
    }
    fn game_time_event(&mut self, e: GameTimeEvent) {
        self.clock_events.push(e);
    }
}

fn busy() -> LuaError {
    LuaError::Runtime { message: "GameTime is in use".into(), class: "java.lang.IllegalStateException" }
}

fn npe() -> LuaError {
    LuaError::Runtime { message: String::new(), class: "java.lang.NullPointerException" }
}

/// Fires collected events through the installed sink.
pub fn fire_events(thread: &mut Thread, ctx: LuaClimateContext) {
    SNOW.with(|s| {
        let mut s = s.borrow_mut();
        for e in &ctx.events {
            match *e {
                ClimateEvent::SnowTarget(t) => s.target = Some(t),
                ClimateEvent::IceQueenSnow(b) => s.ice_queen = Some(b),
                _ => {}
            }
        }
    });
    let Some(sink) = SINK.with(|s| s.borrow().clone()) else { return };
    for e in ctx.events {
        let (name, args): (&str, Vec<Value>) = match e {
            ClimateEvent::ClimateManagerInit => ("OnClimateManagerInit", vec![handle(Obj::Climate)]),
            ClimateEvent::ClimateTick => ("OnClimateTick", vec![handle(Obj::Climate)]),
            ClimateEvent::ClimateTickDebug => ("OnClimateTickDebug", vec![handle(Obj::Climate)]),
            ClimateEvent::WeatherPeriodStart(p) => ("OnWeatherPeriodStart", vec![handle(Obj::Period(p))]),
            ClimateEvent::WeatherPeriodStop(p) => ("OnWeatherPeriodStop", vec![handle(Obj::Period(p))]),
            ClimateEvent::WeatherPeriodComplete(p) => ("OnWeatherPeriodComplete", vec![handle(Obj::Period(p))]),
            ClimateEvent::WeatherPeriodStage(p) => ("OnWeatherPeriodStage", vec![handle(Obj::Period(p))]),
            ClimateEvent::InitModdedWeatherStage { period, stage, strength } => (
                "OnInitModdedWeatherStage",
                vec![handle(Obj::Period(period)), handle(Obj::Stage(period, stage)), fv(strength)],
            ),
            ClimateEvent::UpdateModdedWeatherStage { period, stage, strength } => (
                "OnUpdateModdedWeatherStage",
                vec![handle(Obj::Period(period)), handle(Obj::Stage(period, stage)), fv(strength)],
            ),
            ClimateEvent::ThunderEvent { x, y, strike, lightning, rumble } => (
                "OnThunderEvent",
                vec![Value::from(x), Value::from(y), Value::Bool(strike), Value::Bool(lightning), Value::Bool(rumble)],
            ),
            _ => continue,
        };
        sink(thread, name, &args);
    }
    for e in ctx.clock_events {
        if let GameTimeEvent::Lua(name) = e {
            sink(thread, name, &[]);
        }
    }
}

/// One engine frame of the clock and climate: `GameTime.update` then
/// `ClimateManager.update` (firing `OnClimateTick` mid-update), with the
/// events sent to Lua.
pub fn update_game_time(thread: &mut Thread) {
    let gt = shared_game_time();
    let mut ctx = LuaClimateContext::new();
    let go_on = {
        let mut g = gt.borrow_mut();
        g.update(&mut ctx);
        let clock = g.clock(&ctx);
        g.climate.update_begin(&clock, &mut ctx)
    };
    fire_events(thread, std::mem::take(&mut ctx));
    let mut ctx = LuaClimateContext::new();
    if go_on {
        let mut g = gt.borrow_mut();
        let clock = g.clock(&ctx);
        g.climate.update_end(&clock, &mut ctx);
        g.apply_climate_view_dist();
    }
    fire_events(thread, ctx);
}

// ---- world load -------------------------------------------------------------
//
// The original's order while a world loads (`GameLoadingState`'s loader
// thread, after `GameLoadingState.enter` ran `LuaManager.LoadDirBase
// ("server")`, so `ClimateMain.lua` has added its handler):
//
// 1. `GameTime.setInstance(new GameTime())` ([`install_game_time`]);
// 2. `ClimateManager.setInstance(new ClimateManager())`
//    ([`new_climate_manager`]): the constructor fires
//    `OnClimateManagerInit`, where `ClimateMain.lua` sets the night, fog,
//    cloud and season colours;
// 3. `IsoWorld.init`: `GameTime.init()` then `GameTime.load()` (`map_t.bin`,
//    whose climate part leaves the colours alone), ..., `ErosionGlobals.Boot`
//    (`ErosionMain.start`: [`start_erosion_season`], `OnInitSeasons`), ...,
//    `metaGrid.CreateStep2()`, `ClimateManager.init(metaGrid)`
//    ([`init_climate_manager`]), ..., the cell load, then
//    `ClimateManager.postCellLoadSetSnow()` ([`post_cell_load_set_snow`]).

/// `ClimateManager.setInstance(new ClimateManager())`: a new climate in the
/// shared `GameTime` (simplex offsets drawn from `Rand`, every season colour
/// black), then the constructor's `OnClimateManagerInit` with it.
///
/// The original fires the event from inside the constructor, before
/// `setInstance`, so a handler that called `getClimateManager()` would see
/// the previous instance; `ClimateMain.lua` only uses the argument.
pub fn new_climate_manager(thread: &mut Thread) {
    let gt = shared_game_time();
    let mut ctx = LuaClimateContext::new();
    let cm = ClimateManager::new(&mut ctx);
    gt.borrow_mut().climate = cm;
    fire_events(thread, ctx);
}

/// `ErosionMain.start()`'s season: `cfg` (`erosion.ini`'s season block)
/// with the sandbox `Temperature` option (1 very cold .. 5 very hot), set to
/// `GameTime.getDay()`/`getMonth()`/`getYear()`, then `OnInitSeasons` with
/// it. The season is kept as the climate's erosion season, which
/// [`init_climate_manager`] then takes (the original's `ClimateManager.init`
/// takes `ErosionMain.getInstance().getSeasons()`, the same object).
pub fn start_erosion_season(thread: &mut Thread, cfg: &SeasonConfig, temperature_modifier: i32) {
    let gt = shared_game_time();
    {
        let mut g = gt.borrow_mut();
        let s = ErosionSeason::from_config(cfg, temperature_modifier, g.day, g.month, g.year);
        g.climate.season = Some(s);
    }
    if let Some(sink) = SINK.with(|s| s.borrow().clone()) {
        sink(thread, "OnInitSeasons", &[handle(Obj::Season(SeasonRef::Erosion))]);
    }
}

/// `ClimateManager.getInstance().postCellLoadSetSnow()` on the shared
/// `GameTime` (`IsoWorld.init`, right after the cell load): the snow target
/// and the ice queen's state, returned for the engine to apply (also left
/// in [`take_snow_effects`]'s slot until taken).
pub fn post_cell_load_set_snow(thread: &mut Thread) -> SnowEffects {
    let gt = shared_game_time();
    let mut ctx = LuaClimateContext::new();
    gt.borrow().climate.post_cell_load_set_snow(&mut ctx);
    fire_events(thread, ctx);
    take_snow_effects()
}

/// `ClimateManager.getInstance().init(metaGrid)` on the shared `GameTime`:
/// `bounds` are the meta grid's `(minX, minY, maxX, maxY)` in cells (the
/// thunder storm's map edges), the season is the one
/// [`start_erosion_season`] made, and the forecaster fills its days. Returns
/// false (and does nothing) when no erosion season was started, where the
/// original would fail on the null season.
pub fn init_climate_manager(thread: &mut Thread, bounds: Option<(i32, i32, i32, i32)>) -> bool {
    let gt = shared_game_time();
    let mut ctx = LuaClimateContext::new();
    {
        let mut g = gt.borrow_mut();
        let Some(season) = g.climate.season.take() else { return false };
        let clock = g.clock(&ctx);
        g.climate.init(season, bounds, &clock, &mut ctx);
    }
    fire_events(thread, ctx);
    true
}

/// `GameTime.getInstance().getTimeDelta()` of the shared instance.
pub fn game_time_delta() -> f32 {
    shared_game_time().borrow().get_time_delta(&LuaClimateContext::new())
}

fn with_gt<R>(c: &mut NativeCall<'_>, f: impl FnOnce(&mut GameTime, &mut LuaClimateContext) -> R) -> LuaResult<R> {
    let gt = shared_game_time();
    let mut ctx = LuaClimateContext::new();
    let r = {
        let mut g = gt.try_borrow_mut().map_err(|_| busy())?;
        f(&mut g, &mut ctx)
    };
    fire_events(c.thread, ctx);
    Ok(r)
}

fn fv(f: f32) -> Value {
    Value::Num(f64::from(f))
}

fn farg(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

// ---- objects --------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Front {
    Period(PeriodId),
    Forecast(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SeasonRef {
    Erosion,
    Previous,
    Current,
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Obj {
    GameTime,
    Climate,
    Float(usize),
    Color(usize),
    Bool(usize),
    Front(Front),
    DayInfo(SeasonRef),
    Season(SeasonRef),
    Period(PeriodId),
    Stage(PeriodId, usize),
    Thunder,
    Cloud(usize),
    Forecaster,
    Forecast(usize),
    ForecastValue(usize, u8),
    History,
    Moon,
}

struct ClimateObject(Obj);

impl LuaObject for ClimateObject {
    fn class_name(&self) -> &str {
        match self.0 {
            Obj::GameTime => GAME_TIME_CLASS,
            Obj::Climate => CLIMATE_MANAGER_CLASS,
            Obj::Float(_) => CLIMATE_FLOAT_CLASS,
            Obj::Color(_) => CLIMATE_COLOR_CLASS,
            Obj::Bool(_) => CLIMATE_BOOL_CLASS,
            Obj::Front(_) => AIR_FRONT_CLASS,
            Obj::DayInfo(_) => DAY_INFO_CLASS,
            Obj::Season(_) => EROSION_SEASON_CLASS,
            Obj::Period(_) => WEATHER_PERIOD_CLASS,
            Obj::Stage(..) => WEATHER_STAGE_CLASS,
            Obj::Thunder => THUNDER_STORM_CLASS,
            Obj::Cloud(_) => THUNDER_CLOUD_CLASS,
            Obj::Forecaster => CLIMATE_FORECASTER_CLASS,
            Obj::Forecast(_) => DAY_FORECAST_CLASS,
            Obj::ForecastValue(..) => FORECAST_VALUE_CLASS,
            Obj::History => CLIMATE_HISTORY_CLASS,
            Obj::Moon => CLIMATE_MOON_CLASS,
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The canonical Lua object for `o` (Lua `==` is identity).
fn handle(o: Obj) -> Value {
    HANDLES.with(|h| h.borrow_mut().entry(o).or_insert_with(|| Value::Object(Rc::new(ClimateObject(o)))).clone())
}

fn obj_of(c: &NativeCall<'_>) -> LuaResult<Obj> {
    let o = this::<ClimateObject>(c)?;
    Ok(o.as_any().downcast_ref::<ClimateObject>().ok_or_else(npe)?.0)
}

/// A `ClimateColorInfo` (shared, so Lua writes land in the climate).
pub struct ColorInfoObject(pub ColorRef);

impl LuaObject for ColorInfoObject {
    fn class_name(&self) -> &str {
        CLIMATE_COLOR_INFO_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn color_value(c: &ColorRef) -> Value {
    let key = Rc::as_ptr(c) as usize;
    COLORS.with(|m| {
        m.borrow_mut()
            .entry(key)
            .or_insert_with(|| (c.clone(), Value::Object(Rc::new(ColorInfoObject(c.clone())))))
            .1
            .clone()
    })
}

fn color_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<ColorRef>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<ColorInfoObject>() {
            Some(ci) => Ok(Some(ci.0.clone())),
            None => Err(class_cast(&c.arg(i), CLIMATE_COLOR_INFO_CLASS)),
        },
        other => Err(class_cast(&other, CLIMATE_COLOR_INFO_CLASS)),
    }
}

fn this_color(c: &NativeCall<'_>) -> LuaResult<ColorRef> {
    color_arg(c, 0)?.ok_or_else(npe)
}

/// A `ClimateValues` from `getClimateValuesCopy()`.
pub struct ClimateValuesObject(pub RefCell<ClimateValues>);

impl LuaObject for ClimateValuesObject {
    fn class_name(&self) -> &str {
        CLIMATE_VALUES_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn period_mut(g: &mut GameTime, p: PeriodId) -> Option<&mut WeatherPeriod> {
    match p {
        PeriodId::Main => Some(&mut g.climate.weather_period),
        PeriodId::Forecast(id) => g.climate.climate_forecaster.by_id_mut(id).map(|f| &mut f.weather_period),
    }
}

fn front_mut(g: &mut GameTime, f: Front) -> Option<&mut AirFront> {
    match f {
        Front::Period(p) => period_mut(g, p).map(|w| &mut w.front_cache),
        Front::Forecast(id) => g.climate.climate_forecaster.by_id_mut(id).map(|f| &mut f.air_front),
    }
}

fn season_mut(g: &mut GameTime, s: SeasonRef) -> Option<&mut ErosionSeason> {
    let c = &mut g.climate;
    match s {
        SeasonRef::Erosion => c.season.as_mut(),
        SeasonRef::Previous => c.previous_day.as_mut().and_then(|d| d.season.as_mut()),
        SeasonRef::Current => c.current_day.as_mut().and_then(|d| d.season.as_mut()),
        SeasonRef::Next => c.next_day.as_mut().and_then(|d| d.season.as_mut()),
    }
}

fn day_info(g: &GameTime, s: SeasonRef) -> Option<&pz_core::weather::DayInfo> {
    let c = &g.climate;
    match s {
        SeasonRef::Previous => c.previous_day.as_ref(),
        SeasonRef::Current => c.current_day.as_ref(),
        SeasonRef::Next => c.next_day.as_ref(),
        SeasonRef::Erosion => None,
    }
}

/// `getSeason()`'s object: the current day's season when there is one.
fn season_handle(g: &GameTime) -> Value {
    if g.climate.current_day.as_ref().is_some_and(|d| d.season.is_some()) {
        handle(Obj::Season(SeasonRef::Current))
    } else if g.climate.season.is_some() {
        handle(Obj::Season(SeasonRef::Erosion))
    } else {
        Value::Nil
    }
}

/// Runs `f` on the object `this` names, with the shared `GameTime`.
fn on_obj<R>(
    c: &mut NativeCall<'_>,
    f: impl FnOnce(Obj, &mut GameTime, &mut LuaClimateContext) -> LuaResult<R>,
) -> LuaResult<R> {
    let o = obj_of(c)?;
    with_gt(c, |g, ctx| f(o, g, ctx))?
}

// ---- class constants ------------------------------------------------------

/// The `static final` primitive and string constants (`ConstantValue`) of
/// a class, from the jar.
pub fn class_constants(jar: &Jar, class: &str) -> Vec<(String, Value)> {
    let Ok(cf) = pz_classfile::load_class(jar, class) else { return Vec::new() };
    let mut out = Vec::new();
    for f in &cf.fields {
        let Some(i) = f.constant_value else { continue };
        if f.access & pz_classfile::class::ACC_STATIC == 0 {
            continue;
        }
        let v = match cf.constant(i) {
            Ok(Constant::Integer(v)) => match f.descriptor.as_str() {
                "Z" => Value::Bool(*v != 0),
                _ => Value::from(*v),
            },
            Ok(Constant::Float(v)) => fv(*v),
            Ok(Constant::Long(v)) => Value::Num(*v as f64),
            Ok(Constant::Double(v)) => Value::Num(*v),
            Ok(Constant::String(s)) => match cf.utf8(*s) {
                Ok(s) => Value::from(s),
                Err(_) => continue,
            },
            _ => continue,
        };
        out.push((f.name.clone(), v));
    }
    out
}

const CONSTANT_CLASSES: [(&str, &str); 6] = [
    (GAME_TIME_CLASS, "zombie/GameTime"),
    (CLIMATE_MANAGER_CLASS, "zombie/iso/weather/ClimateManager"),
    (WEATHER_PERIOD_CLASS, "zombie/iso/weather/WeatherPeriod"),
    (TEMPERATURE_CLASS, "zombie/iso/weather/Temperature"),
    (EROSION_SEASON_CLASS, "zombie/erosion/season/ErosionSeason"),
    (THUNDER_STORM_CLASS, "zombie/iso/weather/ThunderStorm"),
];

// ---- registration -----------------------------------------------------------

/// Register the classes and globals, with the constants from the game's jar.
pub fn register(mgr: &mut LuaManager) {
    let jar_path = mgr.fs.borrow().base.join("projectzomboid.jar");
    let constants: Vec<(&'static str, Vec<(String, Value)>)> = match Jar::open(&jar_path) {
        Ok(jar) => CONSTANT_CLASSES.iter().map(|(fqn, path)| (*fqn, class_constants(&jar, path))).collect(),
        Err(e) => {
            (mgr.thread.log)(&format!("bind_climate: {}: {e}", jar_path.display()));
            Vec::new()
        }
    };
    register_with(&mut mgr.thread, &constants);
}

/// Register with already-read class constants.
pub fn register_with(thread: &mut Thread, constants: &[(&'static str, Vec<(String, Value)>)]) {
    global(thread, "getGameTime", |c| Ok(c.push(handle(Obj::GameTime))));
    global(thread, "getClimateManager", |c| Ok(c.push(handle(Obj::Climate))));
    global(thread, "getClimateMoon", |c| Ok(c.push(handle(Obj::Moon))));
    register_game_time(thread);
    register_climate_manager(thread);
    register_layers(thread);
    register_period(thread);
    register_thunder(thread);
    register_forecaster(thread);
    register_misc(thread);
    for (fqn, fields) in constants {
        let b = expose_class(thread, fqn, None);
        for (name, v) in fields {
            b.field(name, v.clone());
        }
    }
}

macro_rules! getter {
    ($b:expr, $name:expr, |$g:ident| $e:expr) => {
        $b.method($name, |c| {
            let v = with_gt(c, |$g, _| $e)?;
            Ok(c.push(v))
        });
    };
}

macro_rules! fgetter {
    ($b:expr, $name:expr, |$g:ident| $e:expr) => {
        $b.method($name, |c| {
            let v: f32 = with_gt(c, |$g, _| $e)?;
            Ok(c.push(fv(v)))
        });
    };
}

macro_rules! setter {
    ($b:expr, $name:expr, |$g:ident, $c:ident| $e:expr) => {
        $b.method($name, |$c| {
            let args = (num_arg($c, 1), bool_arg($c, 1), int_arg($c, 1));
            with_gt($c, |$g, _| {
                let $c = args;
                $e
            })?;
            Ok(0)
        });
    };
}

fn register_game_time(thread: &mut Thread) {
    let b = expose_class(thread, GAME_TIME_CLASS, None);
    b.static_fn("getInstance", |c| Ok(c.push(handle(Obj::GameTime))));
    fgetter!(b, "getTimeOfDay", |g| g.time_of_day);
    setter!(b, "setTimeOfDay", |g, a| g.time_of_day = a.0 as f32);
    getter!(b, "getDay", |g| g.day);
    setter!(b, "setDay", |g, a| g.day = a.2);
    getter!(b, "getDayPlusOne", |g| g.day_plus_one());
    getter!(b, "getMonth", |g| g.month);
    setter!(b, "setMonth", |g, a| g.month = a.2);
    getter!(b, "getYear", |g| g.year);
    setter!(b, "setYear", |g, a| g.year = a.2);
    getter!(b, "getStartDay", |g| g.start_day);
    setter!(b, "setStartDay", |g, a| g.start_day = a.2);
    getter!(b, "getStartMonth", |g| g.start_month);
    setter!(b, "setStartMonth", |g, a| g.start_month = a.2);
    getter!(b, "getStartYear", |g| g.start_year);
    setter!(b, "setStartYear", |g, a| g.start_year = a.2);
    fgetter!(b, "getStartTimeOfDay", |g| g.start_time_of_day);
    setter!(b, "setStartTimeOfDay", |g, a| g.start_time_of_day = a.0 as f32);
    getter!(b, "getNightsSurvived", |g| g.nights_survived);
    setter!(b, "setNightsSurvived", |g, a| g.nights_survived = a.2);
    getter!(b, "getHour", |g| g.hour());
    getter!(b, "getMinutes", |g| g.minutes());
    getter!(b, "getWorldAgeHours", |g| g.world_age_hours());
    b.method("getWorldAgeDaysSinceBegin", |c| {
        // TODO(SandboxOptions): `getTimeSinceApo()` (1 in a new game).
        let v = with_gt(c, |g, _| g.world_age_days_since_begin(1))?;
        Ok(c.push(v))
    });
    b.method("getMinutesStamp", |c| {
        let v = with_gt(c, |g, _| g.minutes_stamp as f64)?;
        Ok(c.push(v))
    });
    type CtxF = fn(&GameTime, &LuaClimateContext) -> f32;
    let ctx_floats: [(&str, CtxF); 9] = [
        ("getMultiplier", |g, x| g.get_multiplier(x)),
        ("getUnmoddedMultiplier", |g, x| g.get_unmodded_multiplier(x)),
        ("getInvMultiplier", |g, x| g.get_inv_multiplier(x)),
        ("getThirtyFPSMultiplier", |g, x| g.get_thirty_fps_multiplier(x)),
        ("getTimeDelta", |g, x| g.get_time_delta(x)),
        ("getMultipliedSecondsSinceLastUpdate", |g, x| g.multiplied_seconds_since_last_update(x)),
        ("getGameWorldSecondsSinceLastUpdate", |g, x| g.game_world_seconds_since_last_update(x)),
        ("getRealworldSecondsSinceLastUpdate", |g, _| g.realworld_seconds_since_last_update()),
        ("getMultiplierInMenu", |g, _| g.get_multiplier_in_menu()),
    ];
    for (name, f) in ctx_floats {
        b.method(name, move |c| {
            let v = with_gt(c, |g, x| f(g, x))?;
            Ok(c.push(fv(v)))
        });
    }
    fgetter!(b, "getTrueMultiplier", |g| g.get_true_multiplier());
    setter!(b, "setMultiplier", |g, a| g.multiplier = a.0 as f32);
    fgetter!(b, "getMinutesPerDay", |g| g.minutes_per_day);
    setter!(b, "setMinutesPerDay", |g, a| g.minutes_per_day = a.0 as f32);
    fgetter!(b, "getDeltaMinutesPerDay", |g| g.get_delta_minutes_per_day());
    fgetter!(b, "getLastTimeOfDay", |g| g.last_time_of_day);
    setter!(b, "setLastTimeOfDay", |g, a| g.last_time_of_day = a.0 as f32);
    fgetter!(b, "getAmbient", |g| g.climate.get_ambient());
    setter!(b, "setAmbient", |g, a| g.ambient = a.0 as f32);
    fgetter!(b, "getAmbientMin", |g| g.ambient_min);
    setter!(b, "setAmbientMin", |g, a| g.set_ambient_min(a.0 as f32));
    fgetter!(b, "getAmbientMax", |g| g.ambient_max);
    setter!(b, "setAmbientMax", |g, a| g.set_ambient_max(a.0 as f32));
    fgetter!(b, "getNightMin", |g| g.get_night_min());
    setter!(b, "setNightMin", |g, a| g.set_night_min(a.0 as f32));
    fgetter!(b, "getNightMax", |g| g.get_night_max());
    setter!(b, "setNightMax", |g, a| g.set_night_max(a.0 as f32));
    fgetter!(b, "getNight", |g| g.climate.get_night_strength());
    fgetter!(b, "getNightTint", |g| g.climate.get_night_strength());
    fgetter!(b, "getViewDist", |g| g.climate.get_view_distance());
    fgetter!(b, "getViewDistMin", |g| g.view_dist_min);
    setter!(b, "setViewDistMin", |g, a| g.view_dist_min = a.0 as f32);
    fgetter!(b, "getViewDistMax", |g| g.view_dist_max);
    setter!(b, "setViewDistMax", |g, a| g.view_dist_max = a.0 as f32);
    getter!(b, "getDawn", |g| g.dawn);
    setter!(b, "setDawn", |g, a| g.dawn = a.2);
    getter!(b, "getDusk", |g| g.dusk);
    setter!(b, "setDusk", |g, a| g.dusk = a.2);
    fgetter!(b, "getMaxZombieCount", |g| g.max_zombie_count);
    setter!(b, "setMaxZombieCount", |g, a| g.max_zombie_count = a.0 as f32);
    fgetter!(b, "getMinZombieCount", |g| g.min_zombie_count);
    setter!(b, "setMinZombieCount", |g, a| g.min_zombie_count = a.0 as f32);
    // The original returns 0 from both start-count getters.
    b.method("getMaxZombieCountStart", |c| Ok(c.push(0.0)));
    b.method("getMinZombieCountStart", |c| Ok(c.push(0.0)));
    setter!(b, "setMaxZombieCountStart", |g, a| g.max_zombie_count_start = a.0 as f32);
    setter!(b, "setMinZombieCountStart", |g, a| g.min_zombie_count_start = a.0 as f32);
    setter!(b, "setTargetZombies", |g, a| g.target_zombies = a.2);
    getter!(b, "isThunderDay", |g| g.thunder_day);
    setter!(b, "setThunderDay", |g, a| g.thunder_day = a.1);
    b.method("isRainingToday", |c| Ok(c.push(true)));
    getter!(b, "getThunderStorm", |g| g.climate.get_is_thunder_storming());
    getter!(b, "getHelicopterDay", |g| g.helicopter_day1);
    getter!(b, "getHelicopterDay1", |g| g.helicopter_day1);
    setter!(b, "setHelicopterDay", |g, a| g.set_helicopter_day(a.2));
    getter!(b, "getHelicopterStartHour", |g| g.helicopter_time1_start);
    setter!(b, "setHelicopterStartHour", |g, a| g.set_helicopter_start_hour(a.2));
    getter!(b, "getHelicopterEndHour", |g| g.helicopter_time1_end);
    setter!(b, "setHelicopterEndHour", |g, a| g.set_helicopter_end_hour(a.2));
    b.method("isDay", |c| {
        let v = with_gt(c, |g, x| g.is_day(x))?;
        Ok(c.push(v))
    });
    b.method("isNight", |c| {
        let v = with_gt(c, |g, x| g.is_night(x))?;
        Ok(c.push(v))
    });
    b.method("isEndlessDay", |c| Ok(c.push(climate_settings().day_night_cycle == 2)));
    b.method("isEndlessNight", |c| Ok(c.push(climate_settings().day_night_cycle == 3)));
    b.method("getModData", |c| {
        let t = with_gt(c, |g, _| g.mod_data())?;
        Ok(c.push(t))
    });
    b.method("Lerp", |c| Ok(c.push(fv(GameTime::lerp(farg(c, 1), farg(c, 2), farg(c, 3))))));
    b.method("TimeLerp", |c| {
        let a = (farg(c, 1), farg(c, 2), farg(c, 3), farg(c, 4));
        let v = with_gt(c, |g, _| g.time_lerp(a.0, a.1, a.2, a.3))?;
        Ok(c.push(fv(v)))
    });
    b.method("daysInMonth", |c| Ok(c.push(GameTime::days_in_month(int_arg(c, 1), int_arg(c, 2)))));
    b.method("getTimeDeltaFromMultiplier", |c| {
        let m = farg(c, 1);
        let v = with_gt(c, |g, _| g.time_delta_from_multiplier(m))?;
        Ok(c.push(fv(v)))
    });
    b.method("getMultiplierFromTimeDelta", |c| {
        let m = farg(c, 1);
        let v = with_gt(c, |g, _| g.multiplier_from_time_delta(m))?;
        Ok(c.push(fv(v)))
    });
    b.method("minHours", |c| {
        let h = num_arg(c, 1);
        let v = with_gt(c, |g, _| h.min(g.world_age_hours()))?;
        Ok(c.push(v))
    });
    b.method("clampHours", |c| {
        let h = num_arg(c, 1);
        let v = with_gt(c, |g, _| h.clamp(0.0, g.world_age_hours().max(0.0)))?;
        Ok(c.push(v))
    });
}

fn register_climate_manager(thread: &mut Thread) {
    let b = expose_class(thread, CLIMATE_MANAGER_CLASS, None);
    b.static_fn("getInstance", |c| Ok(c.push(handle(Obj::Climate))));
    type F = fn(&ClimateManager) -> f32;
    let floats: [(&str, F); 33] = [
        ("getTemperature", ClimateManager::get_temperature),
        ("getHumidity", ClimateManager::get_humidity),
        ("getWindIntensity", ClimateManager::get_wind_intensity),
        ("getWindAngleIntensity", ClimateManager::get_wind_angle_intensity),
        ("getCorrectedWindAngleIntensity", ClimateManager::get_corrected_wind_angle_intensity),
        ("getWindPower", ClimateManager::get_wind_power),
        ("getWindspeedKph", ClimateManager::get_windspeed_kph),
        ("getMaxWindspeedKph", |_| 120.0),
        ("getMaxWindspeedMph", |_| 74.5645),
        ("getCloudIntensity", ClimateManager::get_cloud_intensity),
        ("getAmbient", ClimateManager::get_ambient),
        ("getViewDistance", ClimateManager::get_view_distance),
        ("getDayLightStrength", ClimateManager::get_day_light_strength),
        ("getNightStrength", ClimateManager::get_night_strength),
        ("getDesaturation", ClimateManager::get_desaturation),
        ("getGlobalLightIntensity", ClimateManager::get_global_light_intensity),
        ("getPrecipitationIntensity", ClimateManager::get_precipitation_intensity),
        ("getFogIntensity", ClimateManager::get_fog_intensity),
        ("getRainIntensity", ClimateManager::get_rain_intensity),
        ("getSnowIntensity", ClimateManager::get_snow_intensity),
        ("getSnowStrength", ClimateManager::get_snow_strength),
        ("getSnowFracNow", ClimateManager::get_snow_frac_now),
        ("getAirMass", ClimateManager::get_air_mass),
        ("getAirMassDaily", ClimateManager::get_air_mass_daily),
        ("getAirMassTemperature", ClimateManager::get_air_mass_temperature),
        ("getBaseTemperature", ClimateManager::get_base_temperature),
        ("getDayMeanTemperature", ClimateManager::get_day_mean_temperature),
        ("getWindAngleDegrees", ClimateManager::get_wind_angle_degrees),
        ("getWindAngleRadians", ClimateManager::get_wind_angle_radians),
        ("getWindSpeedMovement", ClimateManager::get_wind_speed_movement),
        ("getWeatherInterference", ClimateManager::get_weather_interference),
        ("getSeasonProgression", ClimateManager::get_season_progression),
        ("getSeasonStrength", ClimateManager::get_season_strength),
    ];
    for (name, f) in floats {
        b.method(name, move |c| {
            let v = with_gt(c, |g, _| f(&g.climate))?;
            Ok(c.push(fv(v)))
        });
    }
    // Doubles: the offsets and world age keep their precision.
    type D = fn(&ClimateManager) -> f64;
    let doubles: [(&str, D); 5] = [
        ("getSimplexOffsetA", |c| c.simplex_offset_a),
        ("getSimplexOffsetB", |c| c.simplex_offset_b),
        ("getSimplexOffsetC", |c| c.simplex_offset_c),
        ("getSimplexOffsetD", |c| c.simplex_offset_d),
        ("getWorldAgeHours", |c| c.world_age_hours),
    ];
    for (name, f) in doubles {
        b.method(name, move |c| {
            let v = with_gt(c, |g, _| f(&g.climate))?;
            Ok(c.push(v))
        });
    }
    type B = fn(&ClimateManager) -> bool;
    let bools: [(&str, B); 7] = [
        ("getPrecipitationIsSnow", ClimateManager::get_precipitation_is_snow),
        ("isRaining", ClimateManager::is_raining),
        ("isSnowing", ClimateManager::is_snowing),
        ("getIsThunderStorming", ClimateManager::get_is_thunder_storming),
        ("getEnabledSimulation", |c| !c.disable_simulation),
        ("getEnabledFxUpdate", |c| !c.disable_fx_update),
        ("getEnabledWeatherGeneration", ClimateManager::get_enabled_weather_generation),
    ];
    for (name, f) in bools {
        b.method(name, move |c| {
            let v = with_gt(c, |g, _| f(&g.climate))?;
            Ok(c.push(v))
        });
    }
    b.method("isUpdated", |c| {
        let v = with_gt(c, |g, _| g.climate.is_updated())?;
        Ok(c.push(v))
    });
    setter!(b, "setAmbient", |g, a| g.climate.set_ambient(a.0 as f32));
    setter!(b, "setViewDistance", |g, a| g.climate.set_view_distance(a.0 as f32));
    setter!(b, "setDayLightStrength", |g, a| g.climate.set_day_light_strength(a.0 as f32));
    setter!(b, "setPrecipitationIsSnow", |g, a| g.climate.set_precipitation_is_snow(a.1));
    setter!(b, "setNightStrength", |g, a| g.climate.set_night_strength(a.0 as f32));
    setter!(b, "setDesaturation", |g, a| g.climate.set_desaturation(a.0 as f32));
    b.method("setEnabledSimulation", |c| {
        let v = bool_arg(c, 1);
        with_gt(c, |g, x| g.climate.set_enabled_simulation(v, x))?;
        Ok(0)
    });
    b.method("setEnabledFxUpdate", |c| {
        let v = bool_arg(c, 1);
        with_gt(c, |g, x| g.climate.set_enabled_fx_update(v, x))?;
        Ok(0)
    });
    setter!(b, "setEnabledWeatherGeneration", |g, a| g.climate.set_enabled_weather_generation(a.1));
    getter!(b, "getSeasonId", |g| g.climate.get_season_id());
    b.method("getSeasonName", |c| {
        let v = with_gt(c, |g, _| g.climate.get_season_name())?;
        Ok(c.push(v.map_or(Value::Nil, Value::from)))
    });
    b.method("getSeasonNameTranslated", |c| {
        let v = with_gt(c, |g, _| g.climate.season.as_ref().and_then(|s| s.get_season_name_key()))?;
        Ok(c.push(v.map_or(Value::Nil, |k| Value::from(crate::bind_text::translate(k)))))
    });
    b.method("getSeason", |c| {
        let v = with_gt(c, |g, _| season_handle(g))?;
        Ok(c.push(v))
    });
    for (name, s) in
        [("getPreviousDay", SeasonRef::Previous), ("getCurrentDay", SeasonRef::Current), ("getNextDay", SeasonRef::Next)]
    {
        b.method(name, move |c| {
            let has = with_gt(c, |g, _| day_info(g, s).is_some())?;
            Ok(c.push(if has { handle(Obj::DayInfo(s)) } else { Value::Nil }))
        });
    }
    b.method("getWeatherPeriod", |c| Ok(c.push(handle(Obj::Period(PeriodId::Main)))));
    b.method("getThunderStorm", |c| Ok(c.push(handle(Obj::Thunder))));
    b.method("getClimateForecaster", |c| Ok(c.push(handle(Obj::Forecaster))));
    b.method("getClimateHistory", |c| Ok(c.push(handle(Obj::History))));
    b.method("getModData", |c| {
        let t = with_gt(c, |g, _| g.climate.get_mod_data())?;
        Ok(c.push(t))
    });
    for (name, max, mk) in [
        ("getClimateFloat", FLOAT_MAX as i32, Obj::Float as fn(usize) -> Obj),
        ("getClimateColor", 2, Obj::Color as fn(usize) -> Obj),
        ("getClimateBool", 1, Obj::Bool as fn(usize) -> Obj),
    ] {
        b.method(name, move |c| {
            let id = int_arg(c, 1);
            // "Climate: cannot get float override id." for others.
            Ok(c.push(if (0..max).contains(&id) { handle(mk(id as usize)) } else { Value::Nil }))
        });
    }
    b.method("getFloatMax", |c| Ok(c.push(FLOAT_MAX as i32)));
    b.method("getColorMax", |c| Ok(c.push(2)));
    b.method("getBoolMax", |c| Ok(c.push(1)));
    type CR = fn(&ClimateManager) -> ColorRef;
    let colors: [(&str, CR); 10] = [
        ("getGlobalLight", |c| c.get_global_light()),
        ("getColorNewFog", |c| c.get_color_new_fog()),
        ("getColNight", |c| c.col_night.clone()),
        ("getColNightNoMoon", |c| c.col_night_no_moon.clone()),
        ("getColNightMoon", |c| c.col_night_moon.clone()),
        ("getColFog", |c| c.col_fog.clone()),
        ("getColFogLegacy", |c| c.col_fog_legacy.clone()),
        ("getColFogNew", |c| c.col_fog_new.clone()),
        ("getFogTintStorm", |c| c.fog_tint_storm.clone()),
        ("getFogTintTropical", |c| c.fog_tint_tropical.clone()),
    ];
    for (name, f) in colors {
        b.method(name, move |c| {
            let v = with_gt(c, |g, _| f(&g.climate))?;
            Ok(c.push(color_value(&v)))
        });
    }
    for (name, seg) in [("setSeasonColorDawn", 0), ("setSeasonColorDay", 1), ("setSeasonColorDusk", 2)] {
        b.method(name, move |c| {
            let (t, s) = (int_arg(c, 1), int_arg(c, 2));
            let rgba = [farg(c, 3), farg(c, 4), farg(c, 5), farg(c, 6)];
            let ext = bool_arg(c, 7);
            let ok = t >= 0 && s >= 0 && t < 3 && s < 4;
            if !ok {
                return Err(LuaError::Runtime {
                    message: format!("Index {t} out of bounds for length 3"),
                    class: "java.lang.ArrayIndexOutOfBoundsException",
                });
            }
            with_gt(c, |g, _| g.climate.set_season_color(seg, t as usize, s as usize, rgba, ext))?;
            Ok(0)
        });
    }
    b.method("getSeasonColor", |c| {
        let (seg, t, s) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        let v = with_gt(c, |g, _| g.climate.get_season_color(seg, t.max(0) as usize, s.max(0) as usize))?;
        Ok(c.push(v.map_or(Value::Nil, |v| color_value(&v))))
    });
    b.method("getGlobalLightInternal", |c| {
        let v = with_gt(c, |g, _| g.climate.get_global_light_internal())?;
        Ok(c.push(Value::Object(Rc::new(pz_ui::ColorObject::new(v[0], v[1], v[2], v[3])))))
    });
    b.method("getFrontStrength", |c| {
        let v = with_gt(c, |g, x| {
            let clock = g.clock(x);
            g.climate.get_front_strength(&clock, x)
        })?;
        Ok(c.push(fv(v)))
    });
    b.method("stopWeatherAndThunder", |c| {
        with_gt(c, |g, x| g.climate.stop_weather_and_thunder(x))?;
        Ok(0)
    });
    b.method("triggerWinterIsComingStorm", |c| {
        with_gt(c, |g, x| {
            let clock = g.clock(x);
            g.climate.trigger_winter_is_coming_storm(&clock, x)
        })?;
        Ok(0)
    });
    b.method("triggerCustomWeather", |c| {
        let (s, warm) = (farg(c, 1), bool_arg(c, 2));
        let v = with_gt(c, |g, x| {
            let clock = g.clock(x);
            g.climate.trigger_custom_weather(s, warm, &clock, x)
        })?;
        Ok(c.push(v))
    });
    b.method("triggerCustomWeatherStage", |c| {
        let (s, d) = (int_arg(c, 1), farg(c, 2));
        let v = with_gt(c, |g, x| {
            let clock = g.clock(x);
            g.climate.trigger_custom_weather_stage(s, d, &clock, x)
        })?;
        Ok(c.push(v))
    });
    b.method("triggerKateBobIntroStorm", |c| {
        let (cx, cy, dur, s, p, a) = (int_arg(c, 1), int_arg(c, 2), num_arg(c, 3), farg(c, 4), farg(c, 5), farg(c, 6));
        let color = if c.nargs() > 8 { color_arg(c, 8)? } else { None };
        with_gt(c, |g, x| {
            let clock = g.clock(x);
            g.climate.trigger_kate_bob_intro_storm(cx, cy, dur, s, p, a, color, &clock, x)
        })?;
        Ok(0)
    });
    b.method("resetOverrides", |c| {
        with_gt(c, |g, _| g.climate.vars.reset_overrides())?;
        Ok(0)
    });
    b.method("resetModded", |c| {
        with_gt(c, |g, _| g.climate.vars.reset_modded())?;
        Ok(0)
    });
    b.method("resetAdmin", |c| {
        with_gt(c, |g, _| g.climate.vars.reset_admin())?;
        Ok(0)
    });
    b.method("forceDayInfoUpdate", |c| {
        with_gt(c, |g, x| {
            let clock = g.clock(x);
            g.climate.force_day_info_update(&clock, x)
        })?;
        Ok(0)
    });
    b.method("getClimateValuesCopy", |c| {
        let v = with_gt(c, |g, _| g.climate.get_climate_values_copy())?;
        Ok(c.push(Value::Object(Rc::new(ClimateValuesObject(RefCell::new(v))))))
    });
    b.method("CopyClimateValues", |c| {
        let Value::Object(o) = c.arg(1) else { return Err(npe()) };
        let Some(t) = o.as_any().downcast_ref::<ClimateValuesObject>() else {
            return Err(class_cast(&c.arg(1), CLIMATE_VALUES_CLASS));
        };
        with_gt(c, |g, _| g.climate.climate_values.copy_values(&mut t.0.borrow_mut()))?;
        Ok(0)
    });
    // Multiplayer requests: nothing to send in single player.
    for name in [
        "transmitServerStopWeather",
        "transmitServerTriggerStorm",
        "transmitServerTriggerLightning",
        "transmitServerStartRain",
        "transmitServerStopRain",
        "transmitRequestAdminVars",
        "transmitClientChangeAdminVars",
        "transmitStopWeather",
        "transmitTriggerStorm",
        "transmitTriggerTropical",
        "transmitTriggerBlizzard",
        "transmitGenerateWeather",
        "updateEveryTenMins",
        "launchFlare",
        "execute_Simulation",
    ] {
        b.method(name, |_| Ok(0));
    }
    b.method("postCellLoadSetSnow", |c| {
        with_gt(c, |g, ctx| g.climate.post_cell_load_set_snow(ctx))?;
        Ok(0)
    });
    b.static_fn("ToKph", |c| Ok(c.push(fv(farg(c, 0) * 120.0))));
    b.static_fn("ToMph", |c| Ok(c.push(fv(farg(c, 0) * 74.5645))));
    b.static_fn("clamp01", |c| Ok(c.push(fv(pz_core::weather::clamp01(farg(c, 0))))));
    b.static_fn("clamp", |c| Ok(c.push(fv(pz_core::weather::clamp(farg(c, 0), farg(c, 1), farg(c, 2))))));
    b.static_fn("lerp", |c| Ok(c.push(fv(pz_core::weather::lerp(farg(c, 0), farg(c, 1), farg(c, 2))))));
    b.static_fn("clerp", |c| Ok(c.push(fv(pz_core::weather::clerp(farg(c, 0), farg(c, 1), farg(c, 2))))));
    b.static_fn("normalizeRange", |c| Ok(c.push(fv(pz_core::weather::normalize_range(farg(c, 0), farg(c, 1))))));
    b.static_fn("posToPosNegRange", |c| Ok(c.push(fv(pz_core::weather::pos_to_pos_neg_range(farg(c, 0))))));
    b.static_fn("getWindAngleString", |c| Ok(c.push(pz_core::weather::wind_angle_string(farg(c, 0)))));
    type S = fn(&ClimateManager) -> f64;
    let statics: [(&str, S); 3] = [
        ("getWindNoiseBase", |c| c.wind_noise_base),
        ("getWindNoiseFinal", |c| c.wind_noise_final),
        ("getWindTickFinal", |c| c.wind_tick_final),
    ];
    for (name, f) in statics {
        b.static_fn(name, move |c| {
            let v = with_gt(c, |g, _| f(&g.climate))?;
            Ok(c.push(v))
        });
    }
}

fn float_of(g: &mut GameTime, i: usize) -> &mut pz_core::weather::ClimateFloat {
    &mut g.climate.vars.floats[i]
}

fn register_layers(thread: &mut Thread) {
    let b = expose_class(thread, CLIMATE_FLOAT_CLASS, None);
    type FG = fn(&pz_core::weather::ClimateFloat) -> Value;
    let gets: [(&str, FG); 13] = [
        ("getID", |f| Value::from(f.id)),
        ("getName", |f| Value::from(f.name)),
        ("getMin", |f| fv(f.min)),
        ("getMax", |f| fv(f.max)),
        ("getInternalValue", |f| fv(f.internal_value)),
        ("getOverride", |f| fv(f.override_value)),
        ("getOverrideInterpolate", |f| fv(f.interpolate)),
        ("isEnableOverride", |f| Value::Bool(f.is_override)),
        ("isEnableAdmin", |f| Value::Bool(f.is_admin_override)),
        ("getAdminValue", |f| fv(f.admin_value)),
        ("getModdedValue", |f| fv(f.modded_value)),
        ("getFinalValue", |f| fv(f.final_value)),
        ("isEnableModded", |f| Value::Bool(f.is_modded)),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = on_obj(c, |o, g, _| match o {
                Obj::Float(i) => Ok(f(float_of(g, i))),
                _ => Err(npe()),
            })?;
            Ok(c.push(v))
        });
    }
    type FS = fn(&mut pz_core::weather::ClimateFloat, f64, bool);
    let sets: [(&str, FS); 8] = [
        ("setEnableOverride", |f, _, b| f.set_enable_override(b)),
        ("setEnableAdmin", |f, _, b| f.set_enable_admin(b)),
        ("setAdminValue", |f, v, _| f.set_admin_value(v as f32)),
        ("setEnableModded", |f, _, b| f.set_enable_modded(b)),
        ("setModdedValue", |f, v, _| f.set_modded_value(v as f32)),
        ("setModdedInterpolate", |f, v, _| f.set_modded_interpolate(v as f32)),
        ("setFinalValue", |f, v, _| f.final_value = v as f32),
        ("setOverrideValue", |f, _, b| f.set_override_value(b)),
    ];
    for (name, f) in sets {
        b.method(name, move |c| {
            let (v, bb) = (num_arg(c, 1), bool_arg(c, 1));
            on_obj(c, |o, g, _| match o {
                Obj::Float(i) => {
                    f(float_of(g, i), v, bb);
                    Ok(())
                }
                _ => Err(npe()),
            })?;
            Ok(0)
        });
    }
    b.method("setOverride", |c| {
        let (t, i2) = (farg(c, 1), farg(c, 2));
        on_obj(c, |o, g, _| match o {
            Obj::Float(i) => {
                float_of(g, i).set_override(t, i2);
                Ok(())
            }
            _ => Err(npe()),
        })?;
        Ok(0)
    });

    let b = expose_class(thread, CLIMATE_COLOR_CLASS, None);
    type CG = fn(&pz_core::weather::ClimateColor) -> Value;
    let gets: [(&str, CG); 11] = [
        ("getID", |f| Value::from(f.id)),
        ("getName", |f| Value::from(f.name)),
        ("getInternalValue", |f| color_value(&f.internal_value)),
        ("getOverride", |f| color_value(&f.override_value)),
        ("getOverrideInterpolate", |f| fv(f.interpolate)),
        ("isEnableOverride", |f| Value::Bool(f.is_override)),
        ("isEnableAdmin", |f| Value::Bool(f.is_admin_override)),
        ("getAdminValue", |f| color_value(&f.admin_value)),
        ("getModdedValue", |f| color_value(&f.modded_value)),
        ("getFinalValue", |f| color_value(&f.final_value)),
        ("isEnableModded", |f| Value::Bool(f.is_modded)),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = on_obj(c, |o, g, _| match o {
                Obj::Color(i) => Ok(f(&g.climate.vars.colors[i])),
                _ => Err(npe()),
            })?;
            Ok(c.push(v))
        });
    }
    type CS = fn(&mut pz_core::weather::ClimateColor, f64, bool);
    let sets: [(&str, CS); 4] = [
        ("setEnableOverride", |f, _, b| f.set_enable_override(b)),
        ("setEnableAdmin", |f, _, b| f.set_enable_admin(b)),
        ("setEnableModded", |f, _, b| f.set_enable_modded(b)),
        ("setModdedInterpolate", |f, v, _| f.set_modded_interpolate(v as f32)),
    ];
    for (name, f) in sets {
        b.method(name, move |c| {
            let (v, bb) = (num_arg(c, 1), bool_arg(c, 1));
            on_obj(c, |o, g, _| match o {
                Obj::Color(i) => {
                    f(&mut g.climate.vars.colors[i], v, bb);
                    Ok(())
                }
                _ => Err(npe()),
            })?;
            Ok(0)
        });
    }
    // Setters taking a ClimateColorInfo copy it into the layer.
    type CT = fn(&pz_core::weather::ClimateColor) -> ColorRef;
    let copies: [(&str, CT); 3] = [
        ("setModdedValue", |f| f.modded_value.clone()),
        ("setFinalValue", |f| f.final_value.clone()),
        ("setAdminValue", |f| f.admin_value.clone()),
    ];
    for (name, f) in copies {
        b.method(name, move |c| {
            let n = c.nargs();
            let src = if n == 2 { Some(color_arg(c, 1)?.ok_or_else(npe)?) } else { None };
            let vals: Vec<f32> = (1..n).map(|i| farg(c, i)).collect();
            on_obj(c, |o, g, _| {
                let Obj::Color(i) = o else { return Err(npe()) };
                let dst = f(&g.climate.vars.colors[i]);
                match &src {
                    Some(s) => pz_core::weather::color::set_to(&dst, s),
                    // setAdminValue(r, g, b, a, r1, g1, b1, a1): exterior, then interior.
                    None if vals.len() == 8 => {
                        let mut d = dst.borrow_mut();
                        d.exterior.copy_from_slice(&vals[0..4]);
                        d.interior.copy_from_slice(&vals[4..8]);
                    }
                    None => return Err(npe()),
                }
                Ok(())
            })?;
            Ok(0)
        });
    }
    for (name, ext) in [("setAdminValueExterior", true), ("setAdminValueInterior", false)] {
        b.method(name, move |c| {
            let v = [farg(c, 1), farg(c, 2), farg(c, 3), farg(c, 4)];
            on_obj(c, |o, g, _| {
                let Obj::Color(i) = o else { return Err(npe()) };
                let mut d = g.climate.vars.colors[i].admin_value.borrow_mut();
                if ext {
                    d.exterior = v;
                } else {
                    d.interior = v;
                }
                Ok(())
            })?;
            Ok(0)
        });
    }
    b.method("setOverride", |c| {
        let src = color_arg(c, 1)?.ok_or_else(npe)?;
        let t = farg(c, 2);
        on_obj(c, |o, g, _| {
            let Obj::Color(i) = o else { return Err(npe()) };
            let v = *src.borrow();
            g.climate.vars.colors[i].set_override(&v, t);
            Ok(())
        })?;
        Ok(0)
    });

    let b = expose_class(thread, CLIMATE_BOOL_CLASS, None);
    type BG = fn(&pz_core::weather::ClimateBool) -> Value;
    let gets: [(&str, BG); 9] = [
        ("getID", |f| Value::from(f.id)),
        ("getName", |f| Value::from(f.name)),
        ("getInternalValue", |f| Value::Bool(f.internal_value)),
        ("getOverride", |f| Value::Bool(f.override_value)),
        ("isEnableOverride", |f| Value::Bool(f.is_override)),
        ("isEnableAdmin", |f| Value::Bool(f.is_admin_override)),
        ("getAdminValue", |f| Value::Bool(f.admin_value)),
        ("getModdedValue", |f| Value::Bool(f.modded_value)),
        ("isEnableModded", |f| Value::Bool(f.is_modded)),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = on_obj(c, |o, g, _| match o {
                Obj::Bool(i) => Ok(f(&g.climate.vars.bools[i])),
                _ => Err(npe()),
            })?;
            Ok(c.push(v))
        });
    }
    type BS = fn(&mut pz_core::weather::ClimateBool, bool);
    let sets: [(&str, BS); 7] = [
        ("setOverride", |f, b| f.set_override(b)),
        ("setEnableOverride", |f, b| f.set_enable_override(b)),
        ("setEnableAdmin", |f, b| f.set_enable_admin(b)),
        ("setAdminValue", |f, b| f.admin_value = b),
        ("setEnableModded", |f, b| f.set_enable_modded(b)),
        ("setModdedValue", |f, b| f.modded_value = b),
        ("setFinalValue", |f, b| f.final_value = b),
    ];
    for (name, f) in sets {
        b.method(name, move |c| {
            let v = bool_arg(c, 1);
            on_obj(c, |o, g, _| match o {
                Obj::Bool(i) => {
                    f(&mut g.climate.vars.bools[i], v);
                    Ok(())
                }
                _ => Err(npe()),
            })?;
            Ok(0)
        });
    }

    let b = expose_class(thread, AIR_FRONT_CLASS, None);
    type AG = fn(&AirFront) -> Value;
    let gets: [(&str, AG); 6] = [
        ("getDays", |f| fv(f.days)),
        ("getMaxNoise", |f| fv(f.max_noise)),
        ("getTotalNoise", |f| fv(f.total_noise)),
        ("getType", |f| Value::from(f.front_type)),
        ("getStrength", |f| fv(f.strength)),
        ("getAngleDegrees", |f| fv(f.front_wind_angle_degrees)),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = on_obj(c, |o, g, _| match o {
                Obj::Front(fr) => front_mut(g, fr).map(|a| f(a)).ok_or_else(npe),
                _ => Err(npe()),
            })?;
            Ok(c.push(v))
        });
    }
    type AS = fn(&mut AirFront, f64);
    let sets: [(&str, AS); 3] = [
        ("setFrontType", |f, v| f.set_front_type(v as i32)),
        ("setStrength", |f, v| f.strength = v as f32),
        ("addDaySample", |f, v| f.add_day_sample(v as f32)),
    ];
    for (name, f) in sets {
        b.method(name, move |c| {
            let v = num_arg(c, 1);
            on_obj(c, |o, g, _| match o {
                Obj::Front(fr) => front_mut(g, fr).map(|a| f(a, v)).ok_or_else(npe),
                _ => Err(npe()),
            })?;
            Ok(0)
        });
    }
    b.method("copyFrom", |c| {
        let Value::Object(src) = c.arg(1) else { return Err(npe()) };
        let Some(ClimateObject(Obj::Front(sf))) = src.as_any().downcast_ref::<ClimateObject>() else {
            return Err(class_cast(&c.arg(1), AIR_FRONT_CLASS));
        };
        let sf = *sf;
        on_obj(c, |o, g, _| {
            let Obj::Front(fr) = o else { return Err(npe()) };
            let s = front_mut(g, sf).ok_or_else(npe)?.clone();
            front_mut(g, fr).ok_or_else(npe)?.copy_from(&s);
            Ok(())
        })?;
        Ok(0)
    });

    let b = expose_class(thread, DAY_INFO_CLASS, None);
    type DG = fn(&pz_core::weather::DayInfo) -> Value;
    let gets: [(&str, DG); 6] = [
        ("getDay", |d| Value::from(d.day)),
        ("getMonth", |d| Value::from(d.month)),
        ("getYear", |d| Value::from(d.year)),
        ("getHour", |d| Value::from(d.hour)),
        ("getMinutes", |d| Value::from(d.minutes)),
        ("getDateValue", |d| Value::Num(d.date_value as f64)),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = on_obj(c, |o, g, _| match o {
                Obj::DayInfo(s) => day_info(g, s).map(f).ok_or_else(npe),
                _ => Err(npe()),
            })?;
            Ok(c.push(v))
        });
    }
    b.method("getSeason", |c| {
        let o = obj_of(c)?;
        Ok(c.push(match o {
            Obj::DayInfo(s) => handle(Obj::Season(s)),
            _ => Value::Nil,
        }))
    });
    b.method("set", |c| {
        let (d, m, y) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        on_obj(c, |o, g, _| {
            let Obj::DayInfo(s) = o else { return Err(npe()) };
            let cl = &mut g.climate;
            let di = match s {
                SeasonRef::Previous => cl.previous_day.as_mut(),
                SeasonRef::Current => cl.current_day.as_mut(),
                SeasonRef::Next => cl.next_day.as_mut(),
                SeasonRef::Erosion => None,
            };
            di.ok_or_else(npe)?.set(d, m, y);
            Ok(())
        })?;
        Ok(0)
    });
}

fn with_period<R>(c: &mut NativeCall<'_>, f: impl FnOnce(&mut WeatherPeriod, &mut LuaClimateContext) -> R) -> LuaResult<R> {
    on_obj(c, |o, g, x| match o {
        Obj::Period(p) => period_mut(g, p).map(|w| f(w, x)).ok_or_else(npe),
        _ => Err(npe()),
    })
}

fn register_period(thread: &mut Thread) {
    let b = expose_class(thread, WEATHER_PERIOD_CLASS, None);
    type PG = fn(&WeatherPeriod) -> Value;
    let gets: [(&str, PG); 19] = [
        ("isRunning", |w| Value::Bool(w.is_running)),
        ("getDuration", |w| Value::Num(w.duration)),
        ("getCurrentStageID", |w| Value::from(w.current_stage_id())),
        ("getWeatherNoise", |w| fv(w.weather_noise)),
        ("getCurrentStrength", |w| fv(w.current_strength)),
        ("getRainThreshold", |w| fv(w.rain_threshold)),
        ("isThunderStorm", |w| Value::Bool(w.is_thunder_storm)),
        ("isTropicalStorm", |w| Value::Bool(w.is_tropical_storm)),
        ("isBlizzard", |w| Value::Bool(w.is_blizzard)),
        ("getPrecipitationFinal", |w| fv(w.precipitation_final)),
        ("getTotalProgress", |w| fv(w.total_progress)),
        ("getStageProgress", |w| fv(w.stage_progress)),
        ("hasTropical", |w| Value::Bool(w.has_tropical())),
        ("hasStorm", |w| Value::Bool(w.has_storm())),
        ("hasBlizzard", |w| Value::Bool(w.has_blizzard())),
        ("hasHeavyRain", |w| Value::Bool(w.has_heavy_rain())),
        ("getTotalStrength", |w| fv(w.total_strength())),
        ("getWindAngleDegrees", |w| fv(w.wind_angle_degrees())),
        ("getFrontType", |w| Value::from(w.front_type())),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = with_period(c, |w, _| f(w))?;
            Ok(c.push(v))
        });
    }
    b.method("getPrintStuff", |c| {
        let v = with_period(c, |w, _| w.print_stuff)?;
        Ok(c.push(v))
    });
    b.method("setPrintStuff", |c| {
        let v = bool_arg(c, 1);
        with_period(c, |w, _| w.print_stuff = v)?;
        Ok(0)
    });
    b.method("setDummy", |c| {
        let v = bool_arg(c, 1);
        with_period(c, |w, _| w.is_dummy = v)?;
        Ok(0)
    });
    b.static_fn("getMaxTemperatureInfluence", |c| Ok(c.push(7.0)));
    b.method("getFrontCache", |c| {
        let o = obj_of(c)?;
        Ok(c.push(match o {
            Obj::Period(p) => handle(Obj::Front(Front::Period(p))),
            _ => Value::Nil,
        }))
    });
    b.method("getCurrentStage", |c| {
        let o = obj_of(c)?;
        let Obj::Period(p) = o else { return Err(npe()) };
        let idx = with_period(c, |w, _| w.current_stage().map(|_| w.current_stage.unwrap_or(0)))?;
        Ok(c.push(idx.map_or(Value::Nil, |i| handle(Obj::Stage(p, i)))))
    });
    b.method("getStageForWorldAge", |c| {
        let o = obj_of(c)?;
        let Obj::Period(p) = o else { return Err(npe()) };
        let h = num_arg(c, 1);
        let idx = with_period(c, |w, _| w.weather_stages.iter().position(|s| h >= s.stage_start && h < s.stage_end))?;
        Ok(c.push(idx.map_or(Value::Nil, |i| handle(Obj::Stage(p, i)))))
    });
    b.method("getWeatherStages", |c| {
        let o = obj_of(c)?;
        let Obj::Period(p) = o else { return Err(npe()) };
        let n = with_period(c, |w, _| w.weather_stages.len())?;
        Ok(c.push(list_value((0..n).map(|i| handle(Obj::Stage(p, i))).collect())))
    });
    type CR = fn(&WeatherPeriod) -> ColorRef;
    let colors: [(&str, CR); 7] = [
        ("getCloudColor", |w| w.cloud_color.clone()),
        ("getCloudColorReddish", |w| w.cloud_color_reddish.clone()),
        ("getCloudColorGreenish", |w| w.cloud_color_greenish.clone()),
        ("getCloudColorBlueish", |w| w.cloud_color_blueish.clone()),
        ("getCloudColorPurplish", |w| w.cloud_color_purplish.clone()),
        ("getCloudColorTropical", |w| w.cloud_color_tropical.clone()),
        ("getCloudColorBlizzard", |w| w.cloud_color_blizzard.clone()),
    ];
    for (name, f) in colors {
        b.method(name, move |c| {
            let v = with_period(c, |w, _| f(w))?;
            Ok(c.push(color_value(&v)))
        });
    }
    b.method("setCloudColor", |c| {
        let col = color_arg(c, 1)?.ok_or_else(npe)?;
        with_period(c, |w, _| w.cloud_color = col)?;
        Ok(0)
    });
    b.method("setKateBobStormProgress", |c| {
        let v = farg(c, 1);
        with_period(c, |w, _| w.set_kate_bob_storm_progress(v))?;
        Ok(0)
    });
    b.method("setKateBobStormCoords", |c| {
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        with_period(c, |w, _| w.set_kate_bob_storm_coords(x, y))?;
        Ok(0)
    });
    b.method("startCreateModdedPeriod", |c| {
        let (warm, s, a) = (bool_arg(c, 1), farg(c, 2), farg(c, 3));
        let o = obj_of(c)?;
        let v = with_gt(c, |g, x| {
            let clock = g.clock(x);
            let (sa, sb) = (g.climate.simplex_offset_a, g.climate.simplex_offset_b);
            match o {
                Obj::Period(p) => period_mut(g, p).map(|w| w.start_create_modded_period(warm, s, a, &clock, sa, sb, x)),
                _ => None,
            }
        })?;
        Ok(c.push(v.ok_or_else(npe)?))
    });
    b.method("endCreateModdedPeriod", |c| {
        let v = with_period(c, |w, x| w.end_create_modded_period(x))?;
        Ok(c.push(v))
    });
    b.method("createAndAddModdedStage", |c| {
        let o = obj_of(c)?;
        let Obj::Period(p) = o else { return Err(npe()) };
        let id = str_arg(c, 1).map(|s| s.to_string());
        let d = num_arg(c, 2);
        let idx = with_period(c, |w, x| match id {
            Some(id) => w.create_and_add_modded_stage(&id, d, x),
            None => None,
        })?;
        Ok(c.push(idx.map_or(Value::Nil, |i| handle(Obj::Stage(p, i)))))
    });
    b.method("createAndAddStage", |c| {
        let o = obj_of(c)?;
        let Obj::Period(p) = o else { return Err(npe()) };
        let (t, d) = (int_arg(c, 1), num_arg(c, 2));
        let idx = with_period(c, |w, x| w.create_and_add_stage(t, d, None, x))?;
        Ok(c.push(idx.map_or(Value::Nil, |i| handle(Obj::Stage(p, i)))))
    });
    b.method("stopWeatherPeriod", |c| {
        let o = obj_of(c)?;
        with_gt(c, |g, x| match o {
            Obj::Period(PeriodId::Main) => {
                let cl = &mut g.climate;
                cl.weather_period.stop_weather_period(Some(&mut cl.vars), x);
            }
            Obj::Period(p) => {
                if let Some(w) = period_mut(g, p) {
                    w.stop_weather_period(None, x);
                }
            }
            _ => {}
        })?;
        Ok(0)
    });

    let b = expose_class(thread, WEATHER_STAGE_CLASS, None);
    fn with_stage<R>(
        c: &mut NativeCall<'_>,
        f: impl FnOnce(&mut WeatherPeriod, usize) -> R,
    ) -> LuaResult<R> {
        on_obj(c, |o, g, _| match o {
            Obj::Stage(p, i) => match period_mut(g, p) {
                Some(w) if i < w.weather_stages.len() => Ok(f(w, i)),
                _ => Err(npe()),
            },
            _ => Err(npe()),
        })
    }
    type SG = fn(&WeatherPeriod, usize) -> Value;
    let gets: [(&str, SG); 9] = [
        ("getStageStart", |w, i| Value::Num(w.weather_stages[i].stage_start)),
        ("getStageEnd", |w, i| Value::Num(w.weather_stages[i].stage_end)),
        ("getStageDuration", |w, i| Value::Num(w.weather_stages[i].stage_duration)),
        ("getStageID", |w, i| Value::from(w.weather_stages[i].stage_id)),
        ("getModID", |w, i| w.weather_stages[i].mod_id.clone().map_or(Value::Nil, Value::from)),
        ("getLinearT", |w, i| fv(w.weather_stages[i].linear_t)),
        ("getParabolicT", |w, i| fv(w.weather_stages[i].parabolic_t)),
        ("getHasStartedCloud", |w, i| Value::Bool(w.weather_stages[i].has_started_cloud)),
        ("getStageCurrentStrength", |w, i| fv(w.stage_strength(i))),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = with_stage(c, |w, i| f(w, i))?;
            Ok(c.push(v))
        });
    }
    b.method("setStageID", |c| {
        let v = int_arg(c, 1);
        with_stage(c, |w, i| w.weather_stages[i].stage_id = v)?;
        Ok(0)
    });
    b.method("setTargetStrength", |c| {
        let v = farg(c, 1);
        with_stage(c, |w, i| w.weather_stages[i].target_strength = v)?;
        Ok(0)
    });
    b.method("setHasStartedCloud", |c| {
        let v = bool_arg(c, 1);
        with_stage(c, |w, i| w.weather_stages[i].set_has_started_cloud(v))?;
        Ok(0)
    });
    b.method("lerpEntryTo", |c| {
        let (m, e) = (int_arg(c, 1), int_arg(c, 2));
        use pz_core::weather::StrLerpVal;
        with_stage(c, |w, i| w.weather_stages[i].lerp_entry_to(StrLerpVal::from_value(m), StrLerpVal::from_value(e)))?;
        Ok(0)
    });

    let e = expose_enum(thread, STR_LERP_VAL_CLASS, &["Entry", "Target", "NextTarget", "None"]);
    e.method("getValue", |c| {
        let v = this::<JavaEnum>(c)?;
        let ord = v.as_any().downcast_ref::<JavaEnum>().map_or(3, |e| e.ordinal);
        Ok(c.push([1, 2, 3, 0][ord.clamp(0, 3) as usize]))
    });
    e.static_fn("fromValue", |c| {
        let v = pz_core::weather::StrLerpVal::from_value(int_arg(c, 0));
        Ok(c.push(crate::expose::enum_value(STR_LERP_VAL_CLASS, v.name())))
    });
}

fn register_thunder(thread: &mut Thread) {
    let b = expose_class(thread, THUNDER_STORM_CLASS, None);
    b.method("getClouds", |c| Ok(c.push(list_value((0..3).map(|i| handle(Obj::Cloud(i))).collect()))));
    b.method("HasActiveThunderClouds", |c| {
        let v = with_gt(c, |g, _| g.climate.thunder_storm.has_active_thunder_clouds())?;
        Ok(c.push(v))
    });
    b.method("isModifyingNight", |c| Ok(c.push(false)));
    b.method("stopAllClouds", |c| {
        with_gt(c, |g, _| g.climate.thunder_storm.stop_all_clouds())?;
        Ok(0)
    });
    b.method("stopCloud", |c| {
        let id = int_arg(c, 1);
        with_gt(c, |g, _| g.climate.thunder_storm.stop_cloud(id))?;
        Ok(0)
    });
    b.method("noise", |_| Ok(0));
    b.static_fn("getMapDiagonal", |c| {
        let v = with_gt(c, |g, _| g.climate.thunder_storm.map_diagonal())?;
        Ok(c.push(v))
    });
    for (name, enqueue) in [("triggerThunderEvent", false), ("enqueueThunderEvent", true)] {
        b.method(name, move |c| {
            let (x, y, s, l, r) = (int_arg(c, 1), int_arg(c, 2), bool_arg(c, 3), bool_arg(c, 4), bool_arg(c, 5));
            with_gt(c, |g, ctx| {
                let t = &mut g.climate.thunder_storm;
                if enqueue {
                    t.enqueue_thunder_event(x, y, s, l, r, ctx);
                } else {
                    t.trigger_thunder_event(x, y, s, l, r, ctx);
                }
            })?;
            Ok(0)
        });
    }
    b.method("startThunderCloud", |c| {
        if c.nargs() < 9 {
            // The 7-argument overload calls itself.
            return Err(LuaError::Runtime { message: String::new(), class: "java.lang.StackOverflowError" });
        }
        let a = (farg(c, 1), farg(c, 2), farg(c, 3), farg(c, 4), farg(c, 5), num_arg(c, 6), bool_arg(c, 7), farg(c, 8));
        let idx = with_gt(c, |g, ctx| {
            let w = g.world_age_hours();
            g.climate.thunder_storm.start_thunder_cloud(a.0, a.1, a.2, a.3, a.4, a.5, a.6, a.7, w, ctx)
        })?;
        Ok(c.push(idx.map_or(Value::Nil, |i| handle(Obj::Cloud(i)))))
    });

    let b = expose_class(thread, THUNDER_CLOUD_CLASS, None);
    type TG = fn(&pz_core::weather::ThunderCloud) -> Value;
    let gets: [(&str, TG); 6] = [
        ("getCurrentX", |t| Value::from(t.current_x)),
        ("getCurrentY", |t| Value::from(t.current_y)),
        ("getRadius", |t| fv(t.radius)),
        ("isRunning", |t| Value::Bool(t.is_running)),
        ("getStrength", |t| fv(t.strength)),
        ("lifeTime", |t| Value::Num(t.life_time())),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = on_obj(c, |o, g, _| match o {
                Obj::Cloud(i) => Ok(f(&g.climate.thunder_storm.clouds[i])),
                _ => Err(npe()),
            })?;
            Ok(c.push(v))
        });
    }
    b.method("setCenter", |c| {
        let (x, y, a) = (int_arg(c, 1), int_arg(c, 2), farg(c, 3));
        on_obj(c, |o, g, ctx| match o {
            Obj::Cloud(i) => {
                g.climate.thunder_storm.set_cloud_center(i, x, y, a, ctx);
                Ok(())
            }
            _ => Err(npe()),
        })?;
        Ok(0)
    });
}

fn register_forecaster(thread: &mut Thread) {
    let b = expose_class(thread, CLIMATE_FORECASTER_CLASS, None);
    b.method("getForecasts", |c| {
        let ids = with_gt(c, |g, _| g.climate.climate_forecaster.forecasts.iter().map(|f| f.id).collect::<Vec<_>>())?;
        Ok(c.push(list_value(ids.into_iter().map(|i| handle(Obj::Forecast(i))).collect())))
    });
    b.method("getForecast", |c| {
        let off = if c.nargs() > 1 { int_arg(c, 1) } else { 0 };
        let id = with_gt(c, |g, _| g.climate.climate_forecaster.get_forecast(off).map(|f| f.id))?;
        Ok(c.push(id.map_or(Value::Nil, |i| handle(Obj::Forecast(i)))))
    });
    b.method("getDaysTillFirstWeather", |c| {
        let v = with_gt(c, |g, _| g.climate.climate_forecaster.days_till_first_weather())?;
        Ok(c.push(v))
    });

    let b = expose_class(thread, DAY_FORECAST_CLASS, None);
    type FG = fn(&pz_core::weather::DayForecast) -> Value;
    let gets: [(&str, FG); 19] = [
        ("getIndexOffset", |f| Value::from(f.index_offset)),
        ("getName", |f| match f.name_date {
            Some((y, m, d)) => Value::from(format!("{}: {y} - {m} - {d}", crate::bind_text::translate("IGUI_Forecaster_Day"))),
            None => Value::from("Day x"),
        }),
        ("isWeatherStarts", |f| Value::Bool(f.weather_starts)),
        ("getWeatherStartTime", |f| fv(f.weather_start_time)),
        ("getWeatherEndTime", |f| fv(f.weather_end_time)),
        ("isChanceOnSnow", |f| Value::Bool(f.chance_on_snow)),
        ("getAirFrontString", |f| Value::from(f.air_front_string)),
        ("isHasFog", |f| Value::Bool(f.has_fog)),
        ("getMeanWindAngleString", |f| Value::from(f.mean_wind_angle_string())),
        ("getFogStrength", |f| fv(f.fog_strength)),
        ("getFogDuration", |f| fv(f.fog_duration)),
        ("isHasHeavyRain", |f| Value::Bool(f.has_heavy_rain)),
        ("isHasStorm", |f| Value::Bool(f.has_storm)),
        ("isHasTropicalStorm", |f| Value::Bool(f.has_tropical_storm)),
        ("isHasBlizzard", |f| Value::Bool(f.has_blizzard)),
        ("getWeatherStages", |f| list_value(f.weather_stages.iter().map(|&i| Value::from(i)).collect())),
        ("getDawn", |f| fv(f.dawn)),
        ("getDusk", |f| fv(f.dusk)),
        ("getDayLightHours", |f| fv(f.day_light_hours)),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = on_obj(c, |o, g, _| match o {
                Obj::Forecast(id) => g.climate.climate_forecaster.by_id(id).map(f).ok_or_else(npe),
                _ => Err(npe()),
            })?;
            Ok(c.push(v))
        });
    }
    for (k, name) in ["getTemperature", "getHumidity", "getWindDirection", "getWindPower", "getCloudiness"]
        .into_iter()
        .enumerate()
    {
        b.method(name, move |c| {
            let o = obj_of(c)?;
            Ok(c.push(match o {
                Obj::Forecast(id) => handle(Obj::ForecastValue(id, k as u8)),
                _ => Value::Nil,
            }))
        });
    }
    b.method("getWeatherPeriod", |c| {
        let o = obj_of(c)?;
        Ok(c.push(match o {
            Obj::Forecast(id) => handle(Obj::Period(PeriodId::Forecast(id))),
            _ => Value::Nil,
        }))
    });
    b.method("getAirFront", |c| {
        let o = obj_of(c)?;
        Ok(c.push(match o {
            Obj::Forecast(id) => handle(Obj::Front(Front::Forecast(id))),
            _ => Value::Nil,
        }))
    });
    b.method("getWeatherOverlap", |c| {
        let v = on_obj(c, |o, g, _| match o {
            Obj::Forecast(id) => Ok(g.climate.climate_forecaster.by_id(id).and_then(|f| f.weather_overlap)),
            _ => Err(npe()),
        })?;
        Ok(c.push(v.map_or(Value::Nil, |i| handle(Obj::Forecast(i)))))
    });

    let b = expose_class(thread, FORECAST_VALUE_CLASS, None);
    type VG = fn(&pz_core::weather::ForecastValue) -> f32;
    let gets: [(&str, VG); 9] = [
        ("getDayMin", |v| v.day_min),
        ("getDayMax", |v| v.day_max),
        ("getDayMean", |v| v.day_mean),
        ("getNightMin", |v| v.night_min),
        ("getNightMax", |v| v.night_max),
        ("getNightMean", |v| v.night_mean),
        ("getTotalMin", |v| v.total_min),
        ("getTotalMax", |v| v.total_max),
        ("getTotalMean", |v| v.total_mean),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = on_obj(c, |o, g, _| match o {
                Obj::ForecastValue(id, k) => {
                    let d = g.climate.climate_forecaster.by_id(id).ok_or_else(npe)?;
                    let fvv = [&d.temperature, &d.humidity, &d.wind_direction, &d.wind_power, &d.cloudiness][k as usize];
                    Ok(f(fvv))
                }
                _ => Err(npe()),
            })?;
            Ok(c.push(fv(v)))
        });
    }
}

fn register_misc(thread: &mut Thread) {
    expose_class(thread, CLIMATE_HISTORY_CLASS, None);

    let b = expose_class(thread, CLIMATE_MOON_CLASS, None);
    b.static_fn("getInstance", |c| Ok(c.push(handle(Obj::Moon))));
    b.method("getPhaseName", |c| {
        let v = with_gt(c, |g, _| g.climate.moon.get_phase_name())?;
        Ok(c.push(v))
    });
    fgetter!(b, "getMoonFloat", |g| g.climate.moon.get_moon_float());
    getter!(b, "getCurrentMoonPhase", |g| g.climate.moon.get_current_moon_phase());
    b.method("updatePhase", |c| {
        let (y, m, d) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        with_gt(c, |g, _| g.climate.moon.update_phase(y, m, d))?;
        Ok(0)
    });
    b.method("Reset", |c| {
        with_gt(c, |g, _| g.climate.moon.reset())?;
        Ok(0)
    });

    let b = expose_class(thread, CLIMATE_COLOR_INFO_CLASS, None);
    b.ctor(|c| {
        let n = c.nargs();
        let v: Vec<f32> = (0..n).map(|i| farg(c, i)).collect();
        let ci = match n {
            4 => ClimateColorInfo::new(v[0], v[1], v[2], v[3]),
            8 => ClimateColorInfo::new8(v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]),
            _ => ClimateColorInfo::default(),
        };
        Ok(c.push(color_value(&ci.shared())))
    });
    for (name, ext) in [("setExterior", true), ("setInterior", false)] {
        b.method(name, move |c| {
            let col = this_color(c)?;
            let v = [farg(c, 1), farg(c, 2), farg(c, 3), farg(c, 4)];
            let mut d = col.borrow_mut();
            if ext {
                d.exterior = v;
            } else {
                d.interior = v;
            }
            Ok(0)
        });
    }
    for (name, ext) in [("getExterior", true), ("getInterior", false)] {
        b.method(name, move |c| {
            let col = this_color(c)?;
            let d = col.borrow();
            let v = if ext { d.exterior } else { d.interior };
            Ok(c.push(Value::Object(Rc::new(pz_ui::ColorObject::new(v[0], v[1], v[2], v[3])))))
        });
    }
    b.method("setTo", |c| {
        let col = this_color(c)?;
        let src = color_arg(c, 1)?.ok_or_else(npe)?;
        pz_core::weather::color::set_to(&col, &src);
        Ok(0)
    });
    b.method("interp", |c| {
        let col = this_color(c)?;
        let to = color_arg(c, 1)?.ok_or_else(npe)?;
        let t = farg(c, 2);
        let dst = color_arg(c, 3)?.ok_or_else(npe)?;
        pz_core::weather::color::interp_into(&col, &to, t, &dst);
        Ok(c.push(color_value(&dst)))
    });
    b.method("scale", |c| {
        let col = this_color(c)?;
        col.borrow_mut().scale(farg(c, 1));
        Ok(0)
    });

    let b = expose_class(thread, CLIMATE_VALUES_CLASS, None);
    fn with_values<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut ClimateValues) -> R) -> LuaResult<R> {
        let o = this::<ClimateValuesObject>(c)?;
        let v = o.as_any().downcast_ref::<ClimateValuesObject>().ok_or_else(npe)?;
        let r = f(&mut v.0.borrow_mut());
        Ok(r)
    }
    type VG = fn(&ClimateValues) -> Value;
    let gets: [(&str, VG); 30] = [
        ("getTime", |v| fv(v.get_time())),
        ("getDawn", |v| fv(v.get_dawn())),
        ("getDusk", |v| fv(v.get_dusk())),
        ("getNoon", |v| fv(v.get_noon())),
        ("getAirMassNoiseFrequencyMod", |v| Value::Num(v.get_air_mass_noise_frequency_mod())),
        ("getNoiseAirmass", |v| fv(v.get_noise_airmass())),
        ("getAirMassTemperature", |v| fv(v.get_air_mass_temperature())),
        ("getBaseTemperature", |v| fv(v.get_base_temperature())),
        ("getDayLightLagged", |v| fv(v.get_day_light_lagged())),
        ("getNightLagged", |v| fv(v.get_night_lagged())),
        ("getTemperature", |v| fv(v.get_temperature())),
        ("isTemperatureIsSnow", |v| Value::Bool(v.is_temperature_is_snow())),
        ("getHumidity", |v| fv(v.get_humidity())),
        ("getWindIntensity", |v| fv(v.get_wind_intensity())),
        ("getWindAngleIntensity", |v| fv(v.get_wind_angle_intensity())),
        ("getWindAngleDegrees", |v| fv(v.get_wind_angle_degrees())),
        ("getNightStrength", |v| fv(v.get_night_strength())),
        ("getDayLightStrength", |v| fv(v.get_day_light_strength())),
        ("getAmbient", |v| fv(v.get_ambient())),
        ("getDesaturation", |v| fv(v.get_desaturation())),
        ("getDayLightStrengthBase", |v| fv(v.get_day_light_strength_base())),
        ("getLerpNight", |v| fv(v.get_lerp_night())),
        ("getCloudyT", |v| fv(v.get_cloudy_t())),
        ("getCloudIntensity", |v| fv(v.get_cloud_intensity())),
        ("getAirFrontAirmass", |v| fv(v.get_air_front_airmass())),
        ("getCacheWorldAgeHours", |v| Value::Num(v.get_cache_world_age_hours())),
        ("getDayMeanTemperature", |v| fv(v.get_day_mean_temperature())),
        ("isDayDoFog", |v| Value::Bool(v.is_day_do_fog())),
        ("getDayFogStrength", |v| fv(v.get_day_fog_strength())),
        ("getDayFogDuration", |v| fv(v.get_day_fog_duration())),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = with_values(c, |v| f(v))?;
            Ok(c.push(v))
        });
    }
    for (name, k) in [("getCacheYear", 0), ("getCacheMonth", 1), ("getCacheDay", 2)] {
        b.method(name, move |c| {
            let v = with_values(c, |v| [v.get_cache_year(), v.get_cache_month(), v.get_cache_day()][k])?;
            Ok(c.push(v))
        });
    }
    b.method("pollDate", |c| {
        let n = c.nargs();
        let a: Vec<i32> = (1..n).map(|i| int_arg(c, i)).collect();
        if a.len() < 3 {
            return Err(npe());
        }
        let (h, mi) = (a.get(3).copied().unwrap_or(0), a.get(4).copied().unwrap_or(0));
        let o = this::<ClimateValuesObject>(c)?;
        with_gt(c, |g, x| {
            let clock = g.clock(x);
            let env = pz_core::weather::values::ValuesEnv {
                simplex_offset_c: g.climate.simplex_offset_c,
                simplex_offset_d: g.climate.simplex_offset_d,
                rain_modifier: x.rain_modifier(),
                winter_is_coming: g.climate.winter_is_coming,
                day_night_cycle: x.day_night_cycle(),
            };
            let base = g.climate.season.clone().unwrap_or_default();
            if let Some(v) = o.as_any().downcast_ref::<ClimateValuesObject>() {
                v.0.borrow_mut().poll_date_ymdhm(a[0], a[1], a[2], h, mi, &clock, &base, &env);
            }
        })?;
        Ok(0)
    });
    b.method("getCopy", |c| {
        let v = with_values(c, |v| v.clone())?;
        let offs = with_gt(c, |g, _| g.climate.offsets())?;
        let mut copy = ClimateValues::new(offs);
        v.copy_values(&mut copy);
        Ok(c.push(Value::Object(Rc::new(ClimateValuesObject(RefCell::new(copy))))))
    });

    let b = expose_class(thread, EROSION_SEASON_CLASS, None);
    fn with_season<R>(c: &mut NativeCall<'_>, f: impl FnOnce(&mut ErosionSeason) -> R) -> LuaResult<R> {
        on_obj(c, |o, g, _| match o {
            Obj::Season(s) => season_mut(g, s).map(f).ok_or_else(npe),
            _ => Err(npe()),
        })
    }
    type EG = fn(&ErosionSeason) -> Value;
    let gets: [(&str, EG); 30] = [
        ("getLat", |s| Value::from(s.get_lat())),
        ("getTempMax", |s| Value::from(s.get_temp_max())),
        ("getTempMin", |s| Value::from(s.get_temp_min())),
        ("getTempDiff", |s| Value::from(s.get_temp_diff())),
        ("getSeasonLag", |s| Value::from(s.get_season_lag())),
        ("getHighNoon", |s| fv(s.get_high_noon())),
        ("getSeedA", |s| Value::from(s.get_seed_a())),
        ("getSeedB", |s| Value::from(s.get_seed_b())),
        ("getSeedC", |s| Value::from(s.get_seed_c())),
        ("getCurDayPercent", |s| fv(s.get_cur_day_percent())),
        ("getMaxDaylightWinter", |s| Value::Num(s.get_max_daylight_winter())),
        ("getMaxDaylightSummer", |s| Value::Num(s.get_max_daylight_summer())),
        ("getDusk", |s| fv(s.get_dusk())),
        ("getDawn", |s| fv(s.get_dawn())),
        ("getDaylight", |s| fv(s.get_daylight())),
        ("getDayTemperature", |s| fv(s.get_day_temperature())),
        ("getDayMeanTemperature", |s| fv(s.get_day_mean_temperature())),
        ("getSeason", |s| Value::from(s.get_season())),
        ("getDayHighNoon", |s| fv(s.get_day_high_noon())),
        ("getSeasonName", |s| s.get_season_name().map_or(Value::Nil, Value::from)),
        ("getSeasonDay", |s| fv(s.get_season_day())),
        ("getSeasonDays", |s| fv(s.get_season_days())),
        ("getSeasonStrength", |s| fv(s.get_season_strength())),
        ("getSeasonProgression", |s| fv(s.get_season_progression())),
        ("getDayNoiseVal", |s| fv(s.get_day_noise_val())),
        ("isRainDay", |s| Value::Bool(s.is_rain_day())),
        ("getRainDayStrength", |s| fv(s.get_rain_day_strength())),
        ("getRainYearAverage", |s| fv(s.get_rain_year_average())),
        ("isThunderDay", |s| Value::Bool(s.is_thunder_day())),
        ("isSunnyDay", |s| Value::Bool(s.is_sunny_day())),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = with_season(c, |s| f(s))?;
            Ok(c.push(v))
        });
    }
    b.method("getSeasonNameTranslated", |c| {
        let k = with_season(c, |s| s.get_season_name_key())?;
        Ok(c.push(k.map_or(Value::Nil, |k| Value::from(crate::bind_text::translate(k)))))
    });
    b.method("isSeason", |c| {
        let v = int_arg(c, 1);
        let r = with_season(c, |s| s.is_season(v))?;
        Ok(c.push(r))
    });
    b.method("setDay", |c| {
        let (d, m, y) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        with_season(c, |s| s.set_day(d, m, y))?;
        Ok(0)
    });
    b.method("setCurSeason", |c| {
        let v = int_arg(c, 1);
        with_season(c, |s| s.set_cur_season(v))?;
        Ok(0)
    });
    b.method("isEndlessDay", |c| Ok(c.push(climate_settings().day_night_cycle == 2)));
    b.method("isEndlessNight", |c| Ok(c.push(climate_settings().day_night_cycle == 3)));

    let b = expose_class(thread, TEMPERATURE_CLASS, None);
    b.static_fn("CelsiusToFahrenheit", |c| Ok(c.push(fv(temperature::celsius_to_fahrenheit(farg(c, 0))))));
    b.static_fn("FahrenheitToCelsius", |c| Ok(c.push(fv(temperature::fahrenheit_to_celsius(farg(c, 0))))));
    b.static_fn("WindchillCelsiusKph", |c| {
        Ok(c.push(fv(temperature::windchill_celsius_kph(farg(c, 0), farg(c, 1)))))
    });
    b.static_fn("getTrueInsulationValue", |c| Ok(c.push(fv(temperature::true_insulation_value(farg(c, 0))))));
    b.static_fn("getTrueWindresistanceValue", |c| {
        Ok(c.push(fv(temperature::true_windresistance_value(farg(c, 0)))))
    });
    let celsius = || crate::bind_core::shared_core().borrow().get_option_temperature_display_celsius();
    b.static_fn("getCelsiusPostfix", |c| Ok(c.push(temperature::CELSIUS_POSTFIX)));
    b.static_fn("getFahrenheitPostfix", |c| Ok(c.push(temperature::FAHRENHEIT_POSTFIX)));
    b.static_fn("getTemperaturePostfix", move |c| Ok(c.push(temperature::temperature_postfix(celsius()))));
    b.static_fn("getTemperatureString", move |c| {
        Ok(c.push(temperature::temperature_string(farg(c, 0), celsius())))
    });
    b.static_fn("getRoundedDisplayTemperature", move |c| {
        Ok(c.push(temperature::rounded_display_temperature(farg(c, 0), celsius())))
    });
    b.static_fn("getFractionForRealTimeRatePerMin", |c| {
        let (len, def) = climate_settings().day_length_minutes;
        Ok(c.push(fv(temperature::fraction_for_real_time_rate_per_min(farg(c, 0), len, def))))
    });
    b.static_fn("getValueColor", |c| {
        let v = temperature::value_color(farg(c, 0));
        Ok(c.push(Value::Object(Rc::new(pz_ui::ColorObject::new(v[0], v[1], v[2], v[3])))))
    });
    b.static_fn("reset", |_| Ok(0));
}

#[cfg(test)]
mod tests;
