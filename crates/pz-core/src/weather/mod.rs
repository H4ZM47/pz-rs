//! `zombie.iso.weather`: the climate simulation (`ClimateManager` and its
//! `ClimateValues`, `WeatherPeriod`, `ThunderStorm`, `ClimateForecaster`,
//! `ClimateMoon`, `Temperature`, `SimplexNoise`) plus `ErosionSeason`, which
//! it is driven by.
//!
//! The engine singletons the original reads (`GameTime`, `SandboxOptions`,
//! `Core`, `IsoPlayer`, `PerformanceSettings`, `Rand`, `LuaEventManager`)
//! come in through [`ClimateClock`] (a snapshot of `GameTime`) and
//! [`ClimateContext`]. Rendering (`WeatherShader`, `IsoWeatherFX`, fog
//! drawing, `WorldFlares`, `SkyBox`, `IsoWater`, `IsoPuddles`) is not part
//! of this module; [`ClimateManager::render_values`] is what it reads.

pub mod calendar;
pub mod color;
pub mod forecaster;
pub mod manager;
pub mod moon;
pub mod period;
pub mod season;
pub mod simplex;
pub mod temperature;
pub mod thunder;
pub mod values;

pub use color::{ClimateColorInfo, ColorRef};
pub use forecaster::{ClimateForecaster, DayForecast, ForecastValue};
pub use manager::{
    AirFront, ClimateBool, ClimateColor, ClimateFloat, ClimateManager, ClimateRenderValues, ClimateVars,
};
pub use moon::ClimateMoon;
pub use period::{StrLerpVal, WeatherPeriod, WeatherStage};
pub use season::{ErosionSeason, SeasonConfig};
pub use thunder::{ThunderCloud, ThunderStorm};
pub use values::{ClimateValues, DayInfo};

/// The `GameTime` values the climate reads, taken once per call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateClock {
    pub year: i32,
    pub month: i32,
    /// `getDayPlusOne()`.
    pub day_plus_one: i32,
    /// `getHour()`.
    pub hour: i32,
    /// `getMinutes()`.
    pub minutes: i32,
    pub time_of_day: f32,
    /// `getWorldAgeHours()`.
    pub world_age_hours: f64,
    /// `getMinutesStamp()`.
    pub minutes_stamp: i64,
    /// `getMultiplier()`.
    pub multiplier: f32,
    /// `getMultipliedSecondsSinceLastUpdate()` (`GameTime.AnimTimer`).
    pub multiplied_seconds: f32,
}

impl Default for ClimateClock {
    /// A fresh `GameTime` (22 July 2012, 09:00).
    fn default() -> Self {
        ClimateClock {
            year: 2012,
            month: 7,
            day_plus_one: 23,
            hour: 9,
            minutes: 0,
            time_of_day: 9.0,
            world_age_hours: 2.0,
            minutes_stamp: 120,
            multiplier: 0.8,
            multiplied_seconds: 0.016666668,
        }
    }
}

/// The Lua events and engine side effects the climate raises, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum ClimateEvent {
    /// `OnClimateManagerInit`.
    ClimateManagerInit,
    /// `OnClimateTick`.
    ClimateTick,
    /// `OnClimateTickDebug`.
    ClimateTickDebug,
    /// `OnWeatherPeriodStart`.
    WeatherPeriodStart(PeriodId),
    /// `OnWeatherPeriodStop`.
    WeatherPeriodStop(PeriodId),
    /// `OnWeatherPeriodComplete`.
    WeatherPeriodComplete(PeriodId),
    /// `OnWeatherPeriodStage`.
    WeatherPeriodStage(PeriodId),
    /// `OnInitModdedWeatherStage(period, stage, strength)`; the stage is
    /// the period's stage at this index.
    InitModdedWeatherStage { period: PeriodId, stage: usize, strength: f32 },
    /// `OnUpdateModdedWeatherStage(period, stage, strength)`.
    UpdateModdedWeatherStage { period: PeriodId, stage: usize, strength: f32 },
    /// `OnThunderEvent(x, y, doStrike, doLightning, doRumble)`.
    ThunderEvent { x: i32, y: i32, strike: bool, lightning: bool, rumble: bool },
    /// The `Thunder` (strike) or `RumbleThunder` sound at a position.
    ThunderSound { x: i32, y: i32, rumble: bool },
    /// `IsoWorld.currentCell.setSnowTarget(target)`.
    SnowTarget(i32),
    /// `ErosionIceQueen.instance.setSnow(b)`.
    IceQueenSnow(bool),
    /// `transmitClimatePacket(ServerOnly, type, null)` on a server.
    TransmitClimatePacket(u8),
}

/// Which `WeatherPeriod` an event is about: the climate's, or the dummy of
/// a forecast day (by [`DayForecast::id`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeriodId {
    Main,
    Forecast(usize),
}

/// The engine state the climate reads and the side effects it causes.
/// Every default is a fresh single-player game with default sandbox
/// options.
pub trait ClimateContext {
    /// `GameClient.client`.
    fn is_client(&self) -> bool {
        false
    }
    /// `GameServer.server`.
    fn is_server(&self) -> bool {
        false
    }
    /// `Core.debug`.
    fn is_debug(&self) -> bool {
        false
    }
    /// `SandboxOptions.climateCycle` (1 normal .. 6 endless blizzard).
    fn climate_cycle(&self) -> i32 {
        1
    }
    /// `SandboxOptions.fogCycle` (1 normal, 2 none, 3 endless).
    fn fog_cycle(&self) -> i32 {
        1
    }
    /// `SandboxOptions.getRainModifier()` (1 .. 5, 3 normal).
    fn rain_modifier(&self) -> i32 {
        3
    }
    /// `SandboxOptions.dayNightCycle` (1 normal, 2 endless day, 3 endless
    /// night).
    fn day_night_cycle(&self) -> i32 {
        1
    }
    /// `PerformanceSettings.fogQuality`.
    fn fog_quality(&self) -> i32 {
        0
    }
    /// `SceneShaderStore.weatherShader != null &&
    /// Core.getInstance().getOffscreenBuffer() != null`.
    fn weather_shader_active(&self) -> bool {
        false
    }
    /// `IsoWorld.instance.getGameMode()` (`Core.gameMode`).
    fn game_mode(&self) -> String {
        "Sandbox".into()
    }
    /// `Core.getInstance().isForceSnow()`.
    fn is_force_snow(&self) -> bool {
        false
    }
    /// `Core.getInstance().setForceSnow(b)`.
    fn set_force_snow(&mut self, _b: bool) {}
    /// `Rand.Next(int max)`.
    fn rand_next(&mut self, max: i32) -> i32 {
        crate::random::next(max)
    }
    /// `Rand.Next(int min, int max)`.
    fn rand_next_range(&mut self, min: i32, max: i32) -> i32 {
        crate::random::next_range(min, max)
    }
    /// `Rand.Next(float min, float max)`.
    fn rand_next_float(&mut self, min: f32, max: f32) -> f32 {
        crate::random::next_float(min, max)
    }
    /// `IsoPlayer.players[i]`'s position (`getX()`, `getY()`) for
    /// `i < IsoPlayer.numPlayers`; `None` for an empty slot.
    fn players(&self) -> [Option<(f32, f32)>; 4] {
        [None; 4]
    }
    /// `SpeedControls.instance.getCurrentGameSpeed()`.
    fn current_game_speed(&self) -> i32 {
        1
    }
    /// `Core.soundDisabled || FMODManager.getNumListeners() <= 0`: thunder
    /// sounds are not played.
    fn sound_disabled(&self) -> bool {
        false
    }
    /// `Core.getInstance().getOptionDisableLightningDuringStorms()`.
    fn disable_lightning_during_storms(&self) -> bool {
        false
    }
    /// `System.currentTimeMillis()` (network interpolation on a client).
    fn current_time_millis(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
    /// Receives events and side effects.
    fn event(&mut self, _e: ClimateEvent) {}
}

/// A [`ClimateContext`] with every default.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultClimateContext;

impl ClimateContext for DefaultClimateContext {}

/// `Math.min(float, float)`.
pub(crate) fn jmin(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() {
            a
        } else {
            b
        }
    } else if a <= b {
        a
    } else {
        b
    }
}

/// `Math.max(float, float)`.
pub(crate) fn jmax(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if b.is_nan() {
        b
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_positive() {
            a
        } else {
            b
        }
    } else if a >= b {
        a
    } else {
        b
    }
}

/// `ClimateManager.clamp(min, max, val)`.
pub fn clamp(min: f32, max: f32, val: f32) -> f32 {
    jmax(min, jmin(max, val))
}

/// `ClimateManager.clamp01(val)`.
pub fn clamp01(val: f32) -> f32 {
    clamp(0.0, 1.0, val)
}

/// `ClimateManager.lerp(t, a, b)`.
pub fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

/// `ClimateManager.clerp(t, a, b)`: cosine interpolation.
pub fn clerp(t: f32, a: f32, b: f32) -> f32 {
    let t2 = (1.0 - (t as f64 * std::f64::consts::PI).cos()) as f32 / 2.0;
    a * (1.0 - t2) + b * t2
}

/// `ClimateManager.normalizeRange(v, n)`.
pub fn normalize_range(v: f32, n: f32) -> f32 {
    clamp(0.0, 1.0, v / n)
}

/// `ClimateManager.posToPosNegRange(v)`.
pub fn pos_to_pos_neg_range(v: f32) -> f32 {
    if v > 0.5 {
        (v - 0.5) * 2.0
    } else if v < 0.5 {
        -((0.5 - v) * 2.0)
    } else {
        0.0
    }
}

/// `ClimateManager.getAirMassNoiseFrequencyMod(sandboxRain)`.
pub fn air_mass_noise_frequency_mod(rain: i32) -> f64 {
    match rain {
        1 => 300.0,
        2 => 240.0,
        4 => 145.0,
        5 => 120.0,
        _ => 166.0,
    }
}

/// `ClimateManager.getRainTimeMultiplierMod(sandboxRain)`.
pub fn rain_time_multiplier_mod(rain: i32) -> f32 {
    match rain {
        1 => 0.5,
        2 => 0.75,
        4 => 1.25,
        5 => 1.5,
        _ => 1.0,
    }
}

/// `ClimateManager.getTimeLerpHours(cur, min, max, doClerp)`.
pub fn time_lerp_hours(cur: f32, min: f32, max: f32, do_clerp: bool) -> f32 {
    time_lerp(clamp(0.0, 1.0, cur / 24.0), clamp(0.0, 1.0, min / 24.0), clamp(0.0, 1.0, max / 24.0), do_clerp)
}

/// `ClimateManager.getTimeLerp(cur, min, max, doClerp)`: 0 → 1 → 0 over
/// `[min, max]`, wrapping when `min > max`.
pub fn time_lerp(cur: f32, min: f32, max: f32, do_clerp: bool) -> f32 {
    let f = |t: f32, a: f32, b: f32| if do_clerp { clerp(t, a, b) } else { lerp(t, a, b) };
    if min <= max {
        if cur < min || cur > max {
            return 0.0;
        }
        let c = cur - min;
        let mid = (max - min) * 0.5;
        if c < mid {
            return f(c / mid, 0.0, 1.0);
        }
        return f((c - mid) / mid, 1.0, 0.0);
    }
    if cur < min && cur > max {
        return 0.0;
    }
    let minoffset = 1.0 - min;
    let c = if cur >= min { cur - min } else { cur + minoffset };
    let mid = (max + minoffset) * 0.5;
    if c < mid {
        f(c / mid, 0.0, 1.0)
    } else {
        f((c - mid) / mid, 1.0, 0.0)
    }
}

/// `ClimateManager.getWindAngleString(angle)`.
pub fn wind_angle_string(angle: f32) -> &'static str {
    const ANGLES: [f32; 9] = [22.5, 67.5, 112.5, 157.5, 202.5, 247.5, 292.5, 337.5, 382.5];
    const NAMES: [&str; 9] = ["SE", "S", "SW", "W", "NW", "N", "NE", "E", "SE"];
    for (a, n) in ANGLES.iter().zip(NAMES) {
        if angle < *a {
            return n;
        }
    }
    NAMES[8]
}

/// `Math.toRadians(deg)`.
pub(crate) fn to_radians(deg: f64) -> f64 {
    deg * 0.017453292519943295
}

#[cfg(test)]
mod tests;
