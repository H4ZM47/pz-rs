//! `zombie.iso.weather.ClimateManager` with its `AirFront`, `ClimateFloat`,
//! `ClimateColor`, `ClimateBool` (internal / modded / override / admin
//! layers), `SeasonColor` and `DayInfo` handling.
//!
//! The network packets (`transmitClimatePacket`, `receiveClimatePacket`
//! and the admin `writeAdmin`/`readAdmin`) are not ported: on a server the
//! climate raises [`ClimateEvent::TransmitClimatePacket`] instead.
//! TODO(network).

use super::calendar::{JavaCalendar, DAY_OF_MONTH, MONTH, YEAR};
use super::color::{interp_into, set_to, ClimateColorInfo, ColorRef};
use super::forecaster::ClimateForecaster;
use super::moon::ClimateMoon;
use super::period::{PeriodInitEnv, PeriodUpdateEnv, WeatherPeriod};
use super::season::ErosionSeason;
use super::simplex::noise2;
use super::temperature::windchill_celsius_kph;
use super::thunder::ThunderStorm;
use super::values::{set_day_info, ClimateValues, DayInfo, ValuesEnv};
use super::{clamp, clamp01, clerp, jmin, lerp, time_lerp_hours, to_radians, ClimateClock, ClimateContext, ClimateEvent, PeriodId};
use pz_kahlua::TableRef;

pub const FRONT_COLD: i32 = -1;
pub const FRONT_STATIONARY: i32 = 0;
pub const FRONT_WARM: i32 = 1;
pub const MAX_WINDSPEED_KPH: f32 = 120.0;
pub const MAX_WINDSPEED_MPH: f32 = 74.5645;
pub const AVG_FAV_AIR_TEMPERATURE: f32 = 22.0;

pub const FLOAT_DESATURATION: usize = 0;
pub const FLOAT_GLOBAL_LIGHT_INTENSITY: usize = 1;
pub const FLOAT_NIGHT_STRENGTH: usize = 2;
pub const FLOAT_PRECIPITATION_INTENSITY: usize = 3;
pub const FLOAT_TEMPERATURE: usize = 4;
pub const FLOAT_FOG_INTENSITY: usize = 5;
pub const FLOAT_WIND_INTENSITY: usize = 6;
pub const FLOAT_WIND_ANGLE_INTENSITY: usize = 7;
pub const FLOAT_CLOUD_INTENSITY: usize = 8;
pub const FLOAT_AMBIENT: usize = 9;
pub const FLOAT_VIEW_DISTANCE: usize = 10;
pub const FLOAT_DAYLIGHT_STRENGTH: usize = 11;
pub const FLOAT_HUMIDITY: usize = 12;
pub const FLOAT_MAX: usize = 13;
pub const COLOR_GLOBAL_LIGHT: usize = 0;
pub const COLOR_NEW_FOG: usize = 1;
pub const COLOR_MAX: usize = 2;
pub const BOOL_IS_SNOW: usize = 0;
pub const BOOL_MAX: usize = 1;

/// `ClimateManager.AirFront`.
#[derive(Clone, Debug, PartialEq)]
pub struct AirFront {
    pub days: f32,
    pub max_noise: f32,
    pub total_noise: f32,
    /// `type`.
    pub front_type: i32,
    pub strength: f32,
    pub front_wind_angle_degrees: f32,
    pub noise_cache: [f32; 2],
}

impl Default for AirFront {
    /// `new AirFront()` (`reset()`).
    fn default() -> Self {
        AirFront {
            days: 0.0,
            max_noise: 0.0,
            total_noise: 0.0,
            front_type: 0,
            strength: 0.0,
            front_wind_angle_degrees: 0.0,
            noise_cache: [-1.0; 2],
        }
    }
}

impl AirFront {
    /// `reset()`.
    pub fn reset(&mut self) {
        *self = AirFront::default();
    }

    /// `setFrontType(type)`: resets, then sets the type.
    pub fn set_front_type(&mut self, t: i32) {
        self.reset();
        self.front_type = t;
    }

    /// `addDaySample(noiseval)`.
    pub fn add_day_sample(&mut self, noiseval: f32) {
        self.days += 1.0;
        if self.front_type == 1 && noiseval <= 0.0 || self.front_type == -1 && noiseval >= 0.0 {
            self.strength = 0.0;
            return;
        }
        let abs = noiseval.abs();
        if abs > self.max_noise {
            self.max_noise = abs;
        }
        self.total_noise += abs;
        let mut cache_value = 0.0f32;
        for i in (0..self.noise_cache.len()).rev() {
            if self.noise_cache[i] > cache_value {
                cache_value = self.noise_cache[i];
            }
            if i < self.noise_cache.len() - 1 {
                self.noise_cache[i + 1] = self.noise_cache[i];
            }
        }
        self.noise_cache[0] = abs;
        if abs > cache_value {
            cache_value = abs;
        }
        self.strength = cache_value * 0.75 + self.max_noise * 0.25;
    }

    /// `copyFrom(other)` (not the noise cache).
    pub fn copy_from(&mut self, o: &AirFront) {
        self.days = o.days;
        self.max_noise = o.max_noise;
        self.total_noise = o.total_noise;
        self.front_type = o.front_type;
        self.strength = o.strength;
        self.front_wind_angle_degrees = o.front_wind_angle_degrees;
    }
}

/// `ClimateManager.ClimateFloat`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClimateFloat {
    pub internal_value: f32,
    pub final_value: f32,
    pub is_override: bool,
    pub override_value: f32,
    pub is_override_value: bool,
    pub override_internal: f32,
    pub interpolate: f32,
    pub is_modded: bool,
    pub modded_value: f32,
    pub mod_interpolate: f32,
    pub is_admin_override: bool,
    pub admin_value: f32,
    pub min: f32,
    pub max: f32,
    pub id: i32,
    pub name: &'static str,
}

impl Default for ClimateFloat {
    fn default() -> Self {
        ClimateFloat {
            internal_value: 0.0,
            final_value: 0.0,
            is_override: false,
            override_value: 0.0,
            is_override_value: false,
            override_internal: 0.0,
            interpolate: 0.0,
            is_modded: false,
            modded_value: 0.0,
            mod_interpolate: 0.0,
            is_admin_override: false,
            admin_value: 0.0,
            min: 0.0,
            max: 1.0,
            id: 0,
            name: "",
        }
    }
}

impl ClimateFloat {
    /// `setOverride(targ, inter)`.
    pub fn set_override(&mut self, targ: f32, inter: f32) {
        self.override_value = targ;
        self.interpolate = inter;
        self.is_override = true;
    }
    /// `setOverrideValue(overrideValue)`.
    pub fn set_override_value(&mut self, b: bool) {
        self.is_override_value = b;
        self.is_override = b;
    }
    pub fn set_enable_override(&mut self, b: bool) {
        self.is_override = b;
    }
    pub fn set_enable_admin(&mut self, b: bool) {
        self.is_admin_override = b;
    }
    /// `setAdminValue(f)`: clamped to `[min, max]`.
    pub fn set_admin_value(&mut self, f: f32) {
        self.admin_value = clamp(self.min, self.max, f);
    }
    pub fn set_enable_modded(&mut self, b: bool) {
        self.is_modded = b;
    }
    /// `setModdedValue(f)`: clamped to `[min, max]`.
    pub fn set_modded_value(&mut self, f: f32) {
        self.modded_value = clamp(self.min, self.max, f);
    }
    /// `setModdedInterpolate(f)`.
    pub fn set_modded_interpolate(&mut self, f: f32) {
        self.mod_interpolate = clamp01(f);
    }
    /// `calculate()`.
    pub fn calculate(&mut self, client: bool) {
        if self.is_admin_override && !client {
            self.final_value = self.admin_value;
            return;
        }
        if self.is_modded && self.mod_interpolate > 0.0 {
            self.internal_value = lerp(self.mod_interpolate, self.internal_value, self.modded_value);
        }
        self.final_value = if self.is_override && self.interpolate > 0.0 {
            if self.is_override_value {
                lerp(self.interpolate, self.override_internal, self.override_value)
            } else {
                lerp(self.interpolate, self.internal_value, self.override_value)
            }
        } else {
            self.internal_value
        };
    }
}

/// `ClimateManager.ClimateColor`.
#[derive(Clone, Debug)]
pub struct ClimateColor {
    pub internal_value: ColorRef,
    pub final_value: ColorRef,
    pub is_override: bool,
    pub override_value: ColorRef,
    pub interpolate: f32,
    pub is_modded: bool,
    pub modded_value: ColorRef,
    pub mod_interpolate: f32,
    pub is_admin_override: bool,
    pub admin_value: ColorRef,
    pub id: i32,
    pub name: &'static str,
}

impl Default for ClimateColor {
    fn default() -> Self {
        let n = || ClimateColorInfo::default().shared();
        ClimateColor {
            internal_value: n(),
            final_value: n(),
            is_override: false,
            override_value: n(),
            interpolate: 0.0,
            is_modded: false,
            modded_value: n(),
            mod_interpolate: 0.0,
            is_admin_override: false,
            admin_value: n(),
            id: 0,
            name: "",
        }
    }
}

impl ClimateColor {
    /// `setOverride(targ, inter)`: copies `targ`.
    pub fn set_override(&mut self, targ: &ClimateColorInfo, inter: f32) {
        *self.override_value.borrow_mut() = *targ;
        self.interpolate = inter;
        self.is_override = true;
    }
    pub fn set_enable_override(&mut self, b: bool) {
        self.is_override = b;
    }
    pub fn set_enable_admin(&mut self, b: bool) {
        self.is_admin_override = b;
    }
    pub fn set_enable_modded(&mut self, b: bool) {
        self.is_modded = b;
    }
    /// `setModdedInterpolate(f)`.
    pub fn set_modded_interpolate(&mut self, f: f32) {
        self.mod_interpolate = clamp01(f);
    }
    /// `calculate()`.
    pub fn calculate(&mut self, client: bool) {
        if self.is_admin_override && !client {
            set_to(&self.final_value, &self.admin_value);
            return;
        }
        if self.is_modded && self.mod_interpolate > 0.0 {
            interp_into(&self.internal_value, &self.modded_value, self.mod_interpolate, &self.internal_value);
        }
        if self.is_override && self.interpolate > 0.0 {
            interp_into(&self.internal_value, &self.override_value, self.interpolate, &self.final_value);
        } else {
            set_to(&self.final_value, &self.internal_value);
        }
    }
}

/// `ClimateManager.ClimateBool`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClimateBool {
    pub internal_value: bool,
    pub final_value: bool,
    pub is_override: bool,
    pub override_value: bool,
    pub is_modded: bool,
    pub modded_value: bool,
    pub is_admin_override: bool,
    pub admin_value: bool,
    pub id: i32,
    pub name: &'static str,
}

impl ClimateBool {
    /// `setOverride(b)`.
    pub fn set_override(&mut self, b: bool) {
        self.is_override = true;
        self.override_value = b;
    }
    pub fn set_enable_override(&mut self, b: bool) {
        self.is_override = b;
    }
    pub fn set_enable_admin(&mut self, b: bool) {
        self.is_admin_override = b;
    }
    pub fn set_enable_modded(&mut self, b: bool) {
        self.is_modded = b;
    }
    /// `calculate()`.
    pub fn calculate(&mut self, client: bool) {
        if self.is_admin_override && !client {
            self.final_value = self.admin_value;
        } else if self.is_modded {
            self.final_value = self.modded_value;
        } else {
            self.final_value = if self.is_override { self.override_value } else { self.internal_value };
        }
    }
}

/// The climate's `climateFloats`, `climateColors` and `climateBooleans`.
#[derive(Clone, Debug)]
pub struct ClimateVars {
    pub floats: [ClimateFloat; FLOAT_MAX],
    pub colors: [ClimateColor; COLOR_MAX],
    pub bools: [ClimateBool; BOOL_MAX],
}

/// The names `setup()` gives the floats, by id.
pub const FLOAT_NAMES: [&str; FLOAT_MAX] = [
    "DESATURATION",
    "GLOBAL_LIGHT_INTENSITY",
    "NIGHT_STRENGTH",
    "PRECIPITATION_INTENSITY",
    "TEMPERATURE",
    "FOG_INTENSITY",
    "WIND_INTENSITY",
    "WIND_ANGLE_INTENSITY",
    "CLOUD_INTENSITY",
    "AMBIENT",
    "VIEW_DISTANCE",
    "DAYLIGHT_STRENGTH",
    "HUMIDITY",
];

impl Default for ClimateVars {
    /// `setup()`.
    fn default() -> Self {
        let mut floats: [ClimateFloat; FLOAT_MAX] = Default::default();
        for (i, f) in floats.iter_mut().enumerate() {
            f.id = i as i32;
            f.name = FLOAT_NAMES[i];
        }
        floats[FLOAT_TEMPERATURE].min = -80.0;
        floats[FLOAT_TEMPERATURE].max = 80.0;
        floats[FLOAT_WIND_ANGLE_INTENSITY].min = -1.0;
        floats[FLOAT_VIEW_DISTANCE].min = 0.0;
        floats[FLOAT_VIEW_DISTANCE].max = 100.0;
        let mut colors: [ClimateColor; COLOR_MAX] = Default::default();
        colors[0].id = 0;
        colors[0].name = "GLOBAL_LIGHT";
        colors[1].id = 1;
        colors[1].name = "COLOR_NEW_FOG";
        *colors[1].internal_value.borrow_mut() = ClimateColorInfo::new(0.9, 0.9, 0.95, 1.0);
        let mut bools: [ClimateBool; BOOL_MAX] = Default::default();
        bools[0].name = "IS_SNOW";
        ClimateVars { floats, colors, bools }
    }
}

impl ClimateVars {
    /// `resetOverrides()`.
    pub fn reset_overrides(&mut self) {
        self.colors.iter_mut().for_each(|c| c.set_enable_override(false));
        self.floats.iter_mut().for_each(|c| c.set_enable_override(false));
        self.bools.iter_mut().for_each(|c| c.set_enable_override(false));
    }
    /// `resetModded()`.
    pub fn reset_modded(&mut self) {
        self.colors.iter_mut().for_each(|c| c.set_enable_modded(false));
        self.floats.iter_mut().for_each(|c| c.set_enable_modded(false));
        self.bools.iter_mut().for_each(|c| c.set_enable_modded(false));
    }
    /// `resetAdmin()`.
    pub fn reset_admin(&mut self) {
        self.colors.iter_mut().for_each(|c| c.set_enable_admin(false));
        self.floats.iter_mut().for_each(|c| c.set_enable_admin(false));
        self.bools.iter_mut().for_each(|c| c.set_enable_admin(false));
    }
    /// `updateOnTick()`: every layer's `calculate()`.
    pub fn calculate(&mut self, client: bool) {
        self.colors.iter_mut().for_each(|c| c.calculate(client));
        self.floats.iter_mut().for_each(|c| c.calculate(client));
        self.bools.iter_mut().for_each(|c| c.calculate(client));
    }
}

/// `ClimateManager.SeasonColor`: a colour per temperature (warm, normal,
/// cloudy) and season (summer, fall, winter, spring).
#[derive(Clone, Debug)]
pub struct SeasonColor {
    pub final_col: ColorRef,
    temp_col: [ColorRef; 3],
    pub colors: [[ColorRef; 4]; 3],
    pub ignore_normal: bool,
}

impl SeasonColor {
    fn new(ignore_normal: bool) -> Self {
        let n = || ClimateColorInfo::default().shared();
        SeasonColor {
            final_col: n(),
            temp_col: [n(), n(), n()],
            colors: std::array::from_fn(|_| std::array::from_fn(|_| n())),
            ignore_normal,
        }
    }

    /// `update(temperatureLerp, seasonLerp, seasonFrom, seasonTo)`.
    pub fn update(&self, temperature_lerp: f32, season_lerp: f32, from: usize, to: usize) -> ColorRef {
        for i in 0..3 {
            if self.ignore_normal && i == 1 {
                continue;
            }
            interp_into(&self.colors[i][from], &self.colors[i][to], season_lerp, &self.temp_col[i]);
        }
        if !self.ignore_normal {
            if temperature_lerp < 0.5 {
                interp_into(&self.temp_col[0], &self.temp_col[1], temperature_lerp * 2.0, &self.final_col);
            } else {
                let cf = 1.0 - (temperature_lerp - 0.5) * 2.0;
                interp_into(&self.temp_col[2], &self.temp_col[1], cf, &self.final_col);
            }
        } else {
            interp_into(&self.temp_col[0], &self.temp_col[2], temperature_lerp, &self.final_col);
        }
        self.final_col.clone()
    }
}

/// The climate values `RenderSettings.PlayerRenderSettings` copies each
/// frame (the `cm*` fields, after `ThunderStorm.applyLightningForPlayer`),
/// plus what the weather shader, fog and weather effects read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateRenderValues {
    /// `getGlobalLight()`.
    pub cm_global_light: ClimateColorInfo,
    /// `getGlobalLightIntensity()`.
    pub cm_global_light_intensity: f32,
    /// `getAmbient()`.
    pub cm_ambient: f32,
    /// `getDayLightStrength()`.
    pub cm_day_light_strength: f32,
    /// `getNightStrength()`.
    pub cm_night_strength: f32,
    /// `getDesaturation()`.
    pub cm_desaturation: f32,
    /// `getViewDistance()`.
    pub cm_view_distance: f32,
    /// `getFogIntensity()`.
    pub cm_fog_intensity: f32,
    /// `ClimateMoon.getInstance().getMoonFloat()`.
    pub moon_float: f32,
    /// `getColorNewFog()`.
    pub color_new_fog: ClimateColorInfo,
    /// `getPrecipitationIntensity()`.
    pub precipitation_intensity: f32,
    /// `getPrecipitationIsSnow()`.
    pub precipitation_is_snow: bool,
    /// `getWindIntensity()`.
    pub wind_intensity: f32,
    /// `getWindAngleIntensity()`.
    pub wind_angle_intensity: f32,
    /// `(float)windTickFinal * (float)windTickFinal` (`IsoWeatherFX`'s
    /// precipitation wind).
    pub wind_precip_intensity: f32,
    /// `getCloudIntensity()`.
    pub cloud_intensity: f32,
    /// `getSnowFracNow()`.
    pub snow_frac_now: f32,
    /// `getHumidity()`.
    pub humidity: f32,
    /// `getTemperature()`.
    pub temperature: f32,
    /// `getWindAngleDegrees()`.
    pub wind_angle_degrees: f32,
}

/// What `getAirTemperatureForSquare` reads about the square and vehicle.
#[derive(Clone, Copy)]
pub struct AirTemperatureQuery<'a> {
    /// `square.isInARoom()`.
    pub in_room: bool,
    /// `square.getZ()`.
    pub z: i32,
    /// `vehicle.getInsideTemperature()` when there is a vehicle.
    pub vehicle_inside_temperature: Option<f32>,
    /// `IsoWorld.instance.isHydroPowerOn()`.
    pub hydro_power_on: bool,
    /// `IsoWorld.instance.getCell().getHeatSourceHighestTemperature(temp,
    /// x, y, z)`; `None` when there are no heat sources (it returns `temp`).
    pub heat_source_highest: Option<&'a dyn Fn(f32) -> f32>,
}

/// One `IsoHeatSource` on the square's level as
/// `IsoCell.getHeatSourceHighestTemperature` sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct HeatSource {
    /// `IsoUtils.DistanceToSquared(x, y, source.x, source.y)`.
    pub dist_sq: f32,
    pub radius: i32,
    pub temperature: i32,
    /// The source square's `isInARoom()`; `None` when the square is not
    /// loaded.
    pub source_in_room: Option<bool>,
    /// `LosUtil.lineClear` is `Clear` or `ClearThroughOpenDoor`.
    pub line_clear: bool,
}

/// `IsoCell.getHeatSourceHighestTemperature(surroundingAirTemperature, x,
/// y, z)` over the sources on the square's level, in the cell's order.
pub fn heat_source_highest_temperature(surrounding: f32, sources: &[HeatSource]) -> f32 {
    let mut temperature = surrounding;
    for h in sources {
        let m = match h.source_in_room {
            None => 0.0,
            Some(false) => (surrounding - 30.0).clamp(-15.0, 5.0),
            Some(true) => (surrounding - 30.0).clamp(-7.0, 7.0),
        };
        let t = lerp(
            (1.0 - (h.dist_sq as f64).sqrt() / h.radius as f64) as f32,
            surrounding,
            h.temperature as f32 + m,
        );
        if t <= temperature || h.dist_sq >= (h.radius * h.radius) as f32 || !h.line_clear {
            continue;
        }
        temperature = t;
    }
    temperature
}

/// `ClimateManager`.
#[derive(Clone, Debug)]
pub struct ClimateManager {
    // Saved (`save`/`load`).
    pub simplex_offset_a: f64,
    pub simplex_offset_b: f64,
    pub simplex_offset_c: f64,
    pub simplex_offset_d: f64,
    pub current_front: AirFront,
    pub snow_frac_now: f32,
    pub snow_strength: f32,
    pub can_do_winter_sprites: bool,
    pub day_do_fog: bool,
    pub day_fog_strength: f32,
    pub weather_period: WeatherPeriod,
    pub thunder_storm: ThunderStorm,
    /// `modDataTable`.
    pub mod_data_table: Option<TableRef>,

    pub disable_simulation: bool,
    pub disable_fx_update: bool,
    pub disable_weather_generation: bool,
    /// `season` (`ErosionMain.getInstance().getSeasons()`, set by `init`).
    pub season: Option<ErosionSeason>,
    pub last_minute_stamp: i64,
    pub air_mass: f32,
    pub air_mass_daily: f32,
    pub air_mass_temperature: f32,
    pub base_temperature: f32,
    pub snow_fall: f32,
    pub snow_melt_strength: f32,
    pub was_force_snow: bool,
    pub wind_power: f32,
    pub world_age_hours: f64,
    pub tick_is_climate_tick: bool,
    pub tick_is_day_change: bool,
    pub last_hour_stamp: i32,
    pub tick_is_hour_change: bool,
    pub tick_is_ten_mins: bool,
    pub col_day: ColorRef,
    pub col_dusk: ColorRef,
    pub col_dawn: ColorRef,
    pub col_night: ColorRef,
    pub col_night_no_moon: ColorRef,
    pub col_night_moon: ColorRef,
    pub col_temp: ColorRef,
    pub col_fog: ColorRef,
    pub col_fog_legacy: ColorRef,
    pub col_fog_new: ColorRef,
    pub fog_tint_storm: ColorRef,
    pub fog_tint_tropical: ColorRef,
    /// `winterIsComing`, `theDescendingFog`, `aStormIsComing` (statics set
    /// by `init` from the game mode).
    pub winter_is_coming: bool,
    pub the_descending_fog: bool,
    pub a_storm_is_coming: bool,
    pub climate_values: ClimateValues,
    pub climate_forecaster: ClimateForecaster,
    pub day_light_lagged: f32,
    pub night_lagged: f32,
    pub vars: ClimateVars,
    pub weather_override: i32,
    pub fog_override: i32,
    /// `windNoiseOffset`, `windNoiseBase`, `windNoiseFinal`, `windTickFinal`
    /// (statics).
    pub wind_noise_offset: f64,
    pub wind_noise_base: f64,
    pub wind_noise_final: f64,
    pub wind_tick_final: f64,
    pub fog_lerp_value: f32,
    pub season_color_dawn: SeasonColor,
    pub season_color_day: SeasonColor,
    pub season_color_dusk: SeasonColor,
    pub previous_day: Option<DayInfo>,
    pub current_day: Option<DayInfo>,
    pub next_day: Option<DayInfo>,
    pub network_lerp: f32,
    pub network_update_stamp: i64,
    pub network_lerp_time: f32,
    pub climate_values_fronts: Option<ClimateValues>,
    /// `ClimateMoon.getInstance()`.
    pub moon: ClimateMoon,
    /// `IsoPlayer.getInstance().isAlwaysDayCheat()` (the owner sets it).
    pub always_day_cheat: bool,
    /// The last `gt.setViewDistMin/Max` values (`GameTime` copies them).
    pub view_dist_min_max: Option<(f32, f32)>,
    /// `ErosionMain`'s `day`, `month`, `year` (the date its season was last
    /// set to; `init` takes it from the clock).
    pub erosion_date: (i32, i32, i32),
}

impl Default for ClimateManager {
    /// `new ClimateManager()` with the simplex offsets at 0 (the original
    /// draws them from `Rand`; see [`ClimateManager::new`]).
    fn default() -> Self {
        ClimateManager::with_offsets([0.0; 4])
    }
}

impl ClimateManager {
    /// `new ClimateManager()`: the offsets come from `Rand` in the
    /// original's order, then `OnClimateManagerInit`.
    pub fn new(ctx: &mut dyn ClimateContext) -> Self {
        let a = ctx.rand_next_range(0, 8000) as f64;
        let b = ctx.rand_next_range(8000, 16000) as f64;
        let c = ctx.rand_next_range(0, -8000) as f64;
        let d = ctx.rand_next_range(-8000, -16000) as f64;
        let m = ClimateManager::with_offsets([a, b, c, d]);
        ctx.event(ClimateEvent::ClimateManagerInit);
        m
    }

    /// The constructor with given simplex offsets (no event).
    pub fn with_offsets(o: [f64; 4]) -> Self {
        let n = |c: ClimateColorInfo| c.shared();
        let values = ClimateValues::new(o);
        let wp_values = values.get_copy(o);
        ClimateManager {
            simplex_offset_a: o[0],
            simplex_offset_b: o[1],
            simplex_offset_c: o[2],
            simplex_offset_d: o[3],
            current_front: AirFront::default(),
            snow_frac_now: 0.0,
            snow_strength: 0.0,
            can_do_winter_sprites: false,
            day_do_fog: false,
            day_fog_strength: 0.0,
            weather_period: WeatherPeriod::new(PeriodId::Main, wp_values),
            thunder_storm: ThunderStorm::default(),
            mod_data_table: None,
            disable_simulation: false,
            disable_fx_update: false,
            disable_weather_generation: false,
            season: None,
            last_minute_stamp: -1,
            air_mass: 0.0,
            air_mass_daily: 0.0,
            air_mass_temperature: 0.0,
            base_temperature: 0.0,
            snow_fall: 0.0,
            snow_melt_strength: 0.0,
            was_force_snow: false,
            wind_power: 0.0,
            world_age_hours: 0.0,
            tick_is_climate_tick: false,
            tick_is_day_change: false,
            last_hour_stamp: -1,
            tick_is_hour_change: false,
            tick_is_ten_mins: false,
            col_day: n(ClimateColorInfo::default()),
            col_dusk: n(ClimateColorInfo::default()),
            col_dawn: n(ClimateColorInfo::default()),
            col_night: n(ClimateColorInfo::new(0.33, 0.33, 0.33, 0.4)),
            col_night_no_moon: n(ClimateColorInfo::new(0.33, 0.33, 0.33, 0.4)),
            col_night_moon: n(ClimateColorInfo::new(0.33, 0.33, 0.33, 0.4)),
            col_temp: n(ClimateColorInfo::default()),
            col_fog: n(ClimateColorInfo::new(0.4, 0.4, 0.4, 0.8)),
            col_fog_legacy: n(ClimateColorInfo::new(0.3, 0.3, 0.3, 0.8)),
            col_fog_new: n(ClimateColorInfo::new8(0.5, 0.5, 0.55, 0.4, 0.5, 0.5, 0.55, 0.8)),
            fog_tint_storm: n(ClimateColorInfo::new(0.5, 0.45, 0.4, 1.0)),
            fog_tint_tropical: n(ClimateColorInfo::new(0.8, 0.75, 0.55, 1.0)),
            winter_is_coming: false,
            the_descending_fog: false,
            a_storm_is_coming: false,
            climate_values: values,
            climate_forecaster: ClimateForecaster::default(),
            day_light_lagged: 0.0,
            night_lagged: 0.0,
            vars: ClimateVars::default(),
            weather_override: 0,
            fog_override: 0,
            wind_noise_offset: 0.0,
            wind_noise_base: 0.0,
            wind_noise_final: 0.0,
            wind_tick_final: 0.0,
            fog_lerp_value: 0.0,
            season_color_dawn: SeasonColor::new(true),
            season_color_day: SeasonColor::new(true),
            season_color_dusk: SeasonColor::new(false),
            previous_day: None,
            current_day: None,
            next_day: None,
            network_lerp: 0.0,
            network_update_stamp: 0,
            network_lerp_time: 5000.0,
            climate_values_fronts: None,
            moon: ClimateMoon::default(),
            always_day_cheat: false,
            view_dist_min_max: None,
            erosion_date: (0, 0, 0),
        }
    }

    /// The four simplex offsets.
    pub fn offsets(&self) -> [f64; 4] {
        [self.simplex_offset_a, self.simplex_offset_b, self.simplex_offset_c, self.simplex_offset_d]
    }

    /// The end of `load`: `climateValues = new ClimateValues(this)` with the
    /// loaded offsets, and the weather period's stage bookkeeping.
    pub fn after_load(&mut self) {
        self.climate_values = ClimateValues::new(self.offsets());
        if self.weather_period.is_running {
            self.weather_period.after_load();
        }
    }

    // ---- accessors -------------------------------------------------------

    fn f(&self, i: usize) -> f32 {
        self.vars.floats[i].final_value
    }

    pub fn get_climate_float(&self, id: i32) -> Option<&ClimateFloat> {
        usize::try_from(id).ok().and_then(|i| self.vars.floats.get(i))
    }
    pub fn get_global_light(&self) -> ColorRef {
        self.vars.colors[COLOR_GLOBAL_LIGHT].final_value.clone()
    }
    pub fn get_global_light_internal(&self) -> [f32; 4] {
        self.vars.colors[COLOR_GLOBAL_LIGHT].internal_value.borrow().exterior
    }
    pub fn get_color_new_fog(&self) -> ColorRef {
        self.vars.colors[COLOR_NEW_FOG].final_value.clone()
    }
    pub fn get_global_light_intensity(&self) -> f32 {
        self.f(FLOAT_GLOBAL_LIGHT_INTENSITY)
    }
    pub fn set_night_strength(&mut self, b: f32) {
        self.vars.floats[FLOAT_NIGHT_STRENGTH].final_value = clamp(0.0, 1.0, b);
    }
    pub fn get_desaturation(&self) -> f32 {
        self.f(FLOAT_DESATURATION)
    }
    pub fn set_desaturation(&mut self, v: f32) {
        self.vars.floats[FLOAT_DESATURATION].final_value = v;
    }
    pub fn get_air_mass(&self) -> f32 {
        self.air_mass
    }
    pub fn get_air_mass_daily(&self) -> f32 {
        self.air_mass_daily
    }
    pub fn get_air_mass_temperature(&self) -> f32 {
        self.air_mass_temperature
    }
    pub fn get_day_light_strength(&self) -> f32 {
        if self.always_day_cheat {
            1.0
        } else {
            self.f(FLOAT_DAYLIGHT_STRENGTH)
        }
    }
    pub fn get_night_strength(&self) -> f32 {
        if self.always_day_cheat {
            0.0
        } else {
            self.f(FLOAT_NIGHT_STRENGTH)
        }
    }
    /// `getDayMeanTemperature()` (0 before the first update; the original
    /// throws).
    pub fn get_day_mean_temperature(&self) -> f32 {
        self.current_day.as_ref().and_then(|d| d.season.as_ref()).map_or(0.0, |s| s.get_day_mean_temperature())
    }
    pub fn get_temperature(&self) -> f32 {
        self.f(FLOAT_TEMPERATURE)
    }
    pub fn get_base_temperature(&self) -> f32 {
        self.base_temperature
    }
    pub fn get_snow_strength(&self) -> f32 {
        self.snow_strength
    }
    pub fn get_precipitation_is_snow(&self) -> bool {
        self.vars.bools[BOOL_IS_SNOW].final_value
    }
    pub fn get_precipitation_intensity(&self) -> f32 {
        self.f(FLOAT_PRECIPITATION_INTENSITY)
    }
    pub fn get_fog_intensity(&self) -> f32 {
        self.f(FLOAT_FOG_INTENSITY)
    }
    pub fn get_wind_intensity(&self) -> f32 {
        self.f(FLOAT_WIND_INTENSITY)
    }
    pub fn get_wind_angle_intensity(&self) -> f32 {
        self.f(FLOAT_WIND_ANGLE_INTENSITY)
    }
    pub fn get_corrected_wind_angle_intensity(&self) -> f32 {
        (self.f(FLOAT_WIND_ANGLE_INTENSITY) + 1.0) * 0.5
    }
    pub fn get_wind_power(&self) -> f32 {
        self.wind_power
    }
    pub fn get_windspeed_kph(&self) -> f32 {
        self.wind_power * 120.0
    }
    pub fn get_cloud_intensity(&self) -> f32 {
        self.f(FLOAT_CLOUD_INTENSITY)
    }
    pub fn get_ambient(&self) -> f32 {
        if self.always_day_cheat {
            1.0
        } else {
            self.f(FLOAT_AMBIENT)
        }
    }
    pub fn get_view_distance(&self) -> f32 {
        self.f(FLOAT_VIEW_DISTANCE)
    }
    pub fn get_humidity(&self) -> f32 {
        self.f(FLOAT_HUMIDITY)
    }
    /// `getWindAngleDegrees()`.
    pub fn get_wind_angle_degrees(&self) -> f32 {
        let w = self.f(FLOAT_WIND_ANGLE_INTENSITY);
        let mut a = if w > 0.0 {
            lerp(w, 45.0, 225.0)
        } else if w > -0.25 {
            lerp(w.abs(), 45.0, 0.0)
        } else {
            lerp(w.abs() - 0.25, 360.0, 180.0)
        };
        if a > 360.0 {
            a -= 360.0;
        }
        if a < 0.0 {
            a += 360.0;
        }
        a
    }
    pub fn get_wind_angle_radians(&self) -> f32 {
        to_radians(self.get_wind_angle_degrees() as f64) as f32
    }
    /// `getWindSpeedMovement()`.
    pub fn get_wind_speed_movement(&self) -> f32 {
        let w = self.get_wind_intensity();
        if w < 0.15 {
            0.0
        } else {
            (w - 0.15) / 0.85
        }
    }
    /// `getWindForceMovement(character, angle)`; `outside` is
    /// `character.square != null && !character.square.isInARoom()`.
    pub fn get_wind_force_movement(&self, outside: bool, angle: f32) -> f32 {
        if !outside {
            return 0.0;
        }
        use std::f64::consts::PI;
        let mut wf = angle - self.get_wind_angle_radians();
        if wf as f64 > PI * 2.0 {
            wf = (wf as f64 - PI * 2.0) as f32;
        }
        if wf < 0.0 {
            wf = (wf as f64 + PI * 2.0) as f32;
        }
        if wf as f64 > PI {
            wf = (PI - (wf as f64 - PI)) as f32;
        }
        (wf as f64 / PI) as f32
    }
    pub fn is_raining(&self) -> bool {
        self.get_precipitation_intensity() > 0.0 && !self.get_precipitation_is_snow()
    }
    pub fn get_rain_intensity(&self) -> f32 {
        if self.is_raining() {
            self.get_precipitation_intensity()
        } else {
            0.0
        }
    }
    pub fn is_snowing(&self) -> bool {
        self.get_precipitation_intensity() > 0.0 && self.get_precipitation_is_snow()
    }
    pub fn get_snow_intensity(&self) -> f32 {
        if self.is_snowing() {
            self.get_precipitation_intensity()
        } else {
            0.0
        }
    }
    pub fn set_ambient(&mut self, f: f32) {
        self.vars.floats[FLOAT_AMBIENT].final_value = f;
    }
    pub fn set_view_distance(&mut self, f: f32) {
        self.vars.floats[FLOAT_VIEW_DISTANCE].final_value = f;
    }
    pub fn set_day_light_strength(&mut self, f: f32) {
        self.vars.floats[FLOAT_DAYLIGHT_STRENGTH].final_value = f;
    }
    pub fn set_precipitation_is_snow(&mut self, b: bool) {
        self.vars.bools[BOOL_IS_SNOW].final_value = b;
    }
    /// `getSeason()`: the current day's season, else the erosion season.
    pub fn get_season(&self) -> Option<&ErosionSeason> {
        self.current_day.as_ref().and_then(|d| d.season.as_ref()).or(self.season.as_ref())
    }
    pub fn get_season_name(&self) -> Option<&'static str> {
        self.season.as_ref().and_then(|s| s.get_season_name())
    }
    pub fn get_season_id(&self) -> i32 {
        self.season.as_ref().map_or(0, |s| s.get_season() as i8 as i32)
    }
    pub fn get_season_progression(&self) -> f32 {
        self.season.as_ref().map_or(0.0, |s| s.get_season_progression())
    }
    pub fn get_season_strength(&self) -> f32 {
        self.season.as_ref().map_or(0.0, |s| s.get_season_strength())
    }
    pub fn get_snow_frac_now(&self) -> f32 {
        self.snow_frac_now
    }
    /// `getIsThunderStorming()`.
    pub fn get_is_thunder_storming(&self) -> bool {
        let w = &self.weather_period;
        w.is_running && (w.is_thunder_storm || w.is_tropical_storm)
    }
    /// `getWeatherInterference()`.
    pub fn get_weather_interference(&self) -> f32 {
        let w = &self.weather_period;
        if !w.is_running {
            0.0
        } else if w.is_thunder_storm || w.is_tropical_storm || w.is_blizzard {
            0.7 * w.current_strength
        } else {
            0.35 * w.current_strength
        }
    }
    /// `getModData()`.
    pub fn get_mod_data(&mut self) -> TableRef {
        self.mod_data_table.get_or_insert_with(TableRef::new).clone()
    }
    /// `setEnabledSimulation(b)`.
    pub fn set_enabled_simulation(&mut self, b: bool, ctx: &dyn ClimateContext) {
        self.disable_simulation = if !ctx.is_client() && !ctx.is_server() { !b } else { false };
    }
    /// `setEnabledFxUpdate(b)`.
    pub fn set_enabled_fx_update(&mut self, b: bool, ctx: &dyn ClimateContext) {
        self.disable_fx_update = if !ctx.is_client() && !ctx.is_server() { !b } else { false };
    }
    /// `getEnabledWeatherGeneration()` (returns the *disable* flag, as the
    /// original does).
    pub fn get_enabled_weather_generation(&self) -> bool {
        self.disable_weather_generation
    }
    pub fn set_enabled_weather_generation(&mut self, b: bool) {
        self.disable_weather_generation = !b;
    }
    /// `isUpdated()`.
    pub fn is_updated(&self) -> bool {
        self.last_minute_stamp != -1
    }
    /// `Reset()`.
    pub fn reset(&mut self) {
        self.last_hour_stamp = -1;
        self.last_minute_stamp = -1;
        if let Some(d) = &mut self.current_day {
            d.year = -1;
            d.month = -1;
            d.day = -1;
        }
    }
    /// `getClimateValuesCopy()`.
    pub fn get_climate_values_copy(&self) -> ClimateValues {
        self.climate_values.get_copy(self.offsets())
    }

    /// `getAirTemperatureForSquare(square, vehicle, doWindChill)`; `None`
    /// for a null square.
    pub fn get_air_temperature_for_square(&self, q: Option<&AirTemperatureQuery<'_>>, wind_chill: bool) -> f32 {
        let mut temp = self.get_temperature();
        let Some(q) = q else { return temp };
        let in_room = q.in_room;
        let vehicle = q.vehicle_inside_temperature;
        if in_room || vehicle.is_some() {
            let electricity = q.hydro_power_on;
            if temp <= 22.0 {
                if in_room && electricity {
                    temp = 22.0;
                }
                let mut m = 22.0 - temp;
                if q.z < 1 {
                    temp += m * (0.4 + 0.2 * self.day_light_lagged);
                } else {
                    m = (m as f64 * 0.85) as f32;
                    temp += m * (0.4 + 0.2 * self.day_light_lagged);
                }
            } else {
                if in_room && electricity {
                    temp = 22.0;
                }
                let mut m = temp - 22.0;
                if q.z < 1 {
                    m = (m as f64 * 0.85) as f32;
                    temp -= m * (0.4 + 0.2 * self.day_light_lagged);
                } else {
                    temp -= m * (0.4 + 0.2 * self.day_light_lagged + 0.2 * self.night_lagged);
                }
                if !in_room && vehicle.is_some() {
                    temp = temp + m + m * self.day_light_lagged;
                }
            }
        } else if wind_chill {
            temp = windchill_celsius_kph(temp, self.get_windspeed_kph());
        }
        if let Some(h) = q.heat_source_highest {
            let hs = h(temp);
            if hs > temp {
                temp = hs;
            }
        }
        if let Some(vt) = vehicle {
            temp += if !in_room || vt > 0.0 { vt } else { 0.0 };
        }
        temp
    }

    /// `Temperature.getWindChillAmountForPlayer(player)`: `None` query means
    /// no square; `in_vehicle` is `player.getVehicle() != null`.
    pub fn wind_chill_amount(&self, q: Option<&AirTemperatureQuery<'_>>, in_vehicle: bool) -> f32 {
        if in_vehicle || q.is_some_and(|q| q.in_room) {
            return 0.0;
        }
        let air = self.get_air_temperature_for_square(q, true);
        if air < self.get_temperature() {
            self.get_temperature() - air
        } else {
            0.0
        }
    }

    /// `getFrontStrength()` (in debug it recomputes the front first).
    pub fn get_front_strength(&mut self, clock: &ClimateClock, ctx: &dyn ClimateContext) -> f32 {
        if ctx.is_debug() {
            let mut f = self.current_front.clone();
            self.calculate_weather_front_strength(clock.year, clock.month, clock.day_plus_one, &mut f, clock, ctx);
            self.current_front = f;
        }
        self.current_front.strength
    }

    fn values_env(&self, ctx: &dyn ClimateContext) -> ValuesEnv {
        ValuesEnv {
            simplex_offset_c: self.simplex_offset_c,
            simplex_offset_d: self.simplex_offset_d,
            rain_modifier: ctx.rain_modifier(),
            winter_is_coming: self.winter_is_coming,
            day_night_cycle: ctx.day_night_cycle(),
        }
    }

    fn base_season(&mut self, ctx: &dyn ClimateContext) -> &ErosionSeason {
        let s = self.season.get_or_insert_with(ErosionSeason::default);
        s.day_night_cycle = ctx.day_night_cycle();
        s
    }

    /// `CalculateWeatherFrontStrength(year, month, day, front)`.
    pub fn calculate_weather_front_strength(
        &mut self,
        year: i32,
        month: i32,
        day: i32,
        front: &mut AirFront,
        clock: &ClimateClock,
        ctx: &dyn ClimateContext,
    ) {
        let env = self.values_env(ctx);
        let offsets = self.offsets();
        self.base_season(ctx);
        let base = self.season.as_ref().expect("season");
        let fronts = self.climate_values_fronts.get_or_insert_with(|| self.climate_values.get_copy(offsets));
        calculate_front(fronts, year, month, day, front, clock, base, &env);
    }

    // ---- init / update ---------------------------------------------------

    /// `init(metaGrid)`: `bounds` are the meta grid's cell bounds
    /// `(minX, minY, maxX, maxY)`; `season` is the erosion season.
    pub fn init(
        &mut self,
        season: ErosionSeason,
        bounds: Option<(i32, i32, i32, i32)>,
        clock: &ClimateClock,
        ctx: &mut dyn ClimateContext,
    ) {
        self.season = Some(season);
        self.erosion_date = (clock.day_plus_one - 1, clock.month, clock.year);
        if let Some((min_x, min_y, max_x, max_y)) = bounds {
            let t = &mut self.thunder_storm;
            t.map_min_x = min_x * 256 - 4000;
            t.map_max_x = max_x * 256 + 4000;
            t.map_min_y = min_y * 256 - 4000;
            t.map_max_y = max_y * 256 + 4000;
        }
        self.wind_noise_offset = 0.0;
        let mode = ctx.game_mode();
        self.winter_is_coming = mode == "Winter is Coming";
        self.the_descending_fog = mode == "The Descending Fog";
        self.a_storm_is_coming = mode == "A Storm is Coming";
        let mut f = std::mem::take(&mut self.climate_forecaster);
        f.init(self, clock, ctx);
        self.climate_forecaster = f;
    }

    /// `updateEveryTenMins()`.
    pub fn update_every_ten_mins(&mut self) {
        self.tick_is_ten_mins = true;
    }

    /// `ErosionMain.mainTimer`'s date check: the erosion season moves to
    /// `GameTime.getDay()` (0-based) when the date changed.
    pub fn erosion_every_ten_minutes(&mut self, day: i32, month: i32, year: i32) {
        if self.erosion_date != (day, month, year) {
            self.erosion_date = (day, month, year);
            if let Some(s) = &mut self.season {
                s.set_day(day, month, year);
            }
        }
    }

    /// `update()`.
    pub fn update(&mut self, clock: &ClimateClock, ctx: &mut dyn ClimateContext) {
        if self.update_begin(clock, ctx) {
            self.update_end(clock, ctx);
        }
    }

    /// `update()` up to and including `OnClimateTick`; false when the
    /// simulation is disabled (the rest is skipped).
    pub fn update_begin(&mut self, clock: &ClimateClock, ctx: &mut dyn ClimateContext) -> bool {
        self.tick_is_climate_tick = false;
        self.tick_is_hour_change = false;
        self.tick_is_day_change = false;
        self.world_age_hours = clock.world_age_hours;
        self.base_season(ctx);
        if self.last_minute_stamp != clock.minutes_stamp {
            self.last_minute_stamp = clock.minutes_stamp;
            self.tick_is_climate_tick = true;
            self.update_day_info(clock.day_plus_one, clock.month, clock.year);
            let d = self.current_day.as_mut().expect("currentDay");
            d.hour = clock.hour;
            d.minutes = clock.minutes;
            if clock.hour != self.last_hour_stamp {
                self.tick_is_hour_change = true;
                self.last_hour_stamp = clock.hour;
            }
            let (y, m, dd) = (d.year, d.month, d.day);
            self.moon.update_phase(y, m, dd);
        }
        if self.disable_simulation {
            return false;
        }
        if self.tick_is_day_change && !ctx.is_client() {
            let mut f = std::mem::take(&mut self.climate_forecaster);
            f.update_day_change(self, clock, ctx);
            self.climate_forecaster = f;
        }
        if ctx.is_client() {
            self.network_lerp = 1.0;
            let now = ctx.current_time_millis();
            if (now as f32) < self.network_update_stamp as f32 + self.network_lerp_time {
                self.network_lerp = (now - self.network_update_stamp) as f32 / self.network_lerp_time;
                if self.network_lerp < 0.0 {
                    self.network_lerp = 0.0;
                }
            }
            for f in &mut self.vars.floats {
                f.interpolate = self.network_lerp;
            }
            for c in &mut self.vars.colors {
                c.interpolate = self.network_lerp;
            }
        }
        if self.tick_is_climate_tick && !ctx.is_client() {
            self.update_sandbox_overrides(clock, ctx);
            self.update_values(clock, ctx);
            let season_is_winter = self.get_season().is_some_and(|s| s.is_season(5));
            let mut env = PeriodUpdateEnv {
                vars: &mut self.vars,
                thunder: &mut self.thunder_storm,
                fog_tint_storm: &self.fog_tint_storm,
                fog_tint_tropical: &self.fog_tint_tropical,
                season_is_winter,
                winter_is_coming: self.winter_is_coming,
                fog_quality: ctx.fog_quality(),
                world_age_hours: clock.world_age_hours,
            };
            self.weather_period.update(self.world_age_hours, &mut env, ctx);
        }
        if self.tick_is_climate_tick {
            ctx.event(ClimateEvent::ClimateTick);
        }
        true
    }

    /// The rest of `update()` after `OnClimateTick`.
    pub fn update_end(&mut self, clock: &ClimateClock, ctx: &mut dyn ClimateContext) {
        self.vars.calculate(ctx.is_client());
        self.update_wind_tick(clock, ctx);
        self.wind_power = self.vars.floats[FLOAT_WIND_INTENSITY].final_value;
        let mut dl = self.vars.floats[FLOAT_DAYLIGHT_STRENGTH].final_value;
        self.thunder_storm.update(self.world_age_hours, &mut dl, clock.multiplied_seconds, ctx);
        self.vars.floats[FLOAT_DAYLIGHT_STRENGTH].final_value = dl;
        if ctx.is_client() || self.tick_is_climate_tick {
            self.update_snow(ctx);
        }
        if !ctx.is_client() {
            self.update_view_distance();
        }
        if self.tick_is_climate_tick && ctx.is_debug() && !ctx.is_server() {
            ctx.event(ClimateEvent::ClimateTickDebug);
        }
        if self.tick_is_climate_tick && ctx.is_server() && self.tick_is_ten_mins {
            ctx.event(ClimateEvent::TransmitClimatePacket(0));
            self.tick_is_ten_mins = false;
        }
    }

    fn update_sandbox_overrides(&mut self, clock: &ClimateClock, ctx: &mut dyn ClimateContext) {
        let mut setting_changed = false;
        let minute_lerp = clock.time_of_day % 1.0;
        let cycle = ctx.climate_cycle();
        let overriding = cycle > 1;
        let endless = cycle > 2;
        let no_weather = cycle == 2;
        let endless_rain = cycle == 3;
        let endless_snow = cycle == 5;
        let endless_blizzard = cycle == 6;
        let freezing = endless_snow || endless_blizzard;
        let fl = &mut self.vars.floats;
        if self.weather_override != cycle {
            self.weather_override = cycle;
            self.disable_weather_generation = self.weather_override != 1;
            fl[FLOAT_PRECIPITATION_INTENSITY].set_override_value(overriding);
            fl[FLOAT_WIND_INTENSITY].set_override_value(overriding);
            fl[FLOAT_TEMPERATURE].set_override_value(freezing);
            ctx.set_force_snow(freezing);
            if no_weather {
                ctx.set_force_snow(false);
                fl[FLOAT_PRECIPITATION_INTENSITY].set_override(0.0, 1.0);
                fl[FLOAT_WIND_INTENSITY].set_override(0.0, 1.0);
                fl[FLOAT_TEMPERATURE].set_override_value(false);
            } else if endless {
                fl[FLOAT_PRECIPITATION_INTENSITY].override_internal = 0.5;
                fl[FLOAT_WIND_INTENSITY].override_internal = 0.3;
                if freezing {
                    fl[FLOAT_TEMPERATURE].override_internal = ctx.rand_next_float(-20.0, 0.0);
                }
            }
            setting_changed = overriding;
        }
        if endless {
            fl[FLOAT_PRECIPITATION_INTENSITY].interpolate = minute_lerp;
            fl[FLOAT_WIND_INTENSITY].interpolate = minute_lerp;
            fl[FLOAT_TEMPERATURE].interpolate = minute_lerp;
            if self.tick_is_hour_change || setting_changed {
                ctx.set_force_snow(freezing);
                self.vars.bools[BOOL_IS_SNOW].set_override(freezing);
                let calm = endless_rain || endless_snow;
                let p = &mut fl[FLOAT_PRECIPITATION_INTENSITY];
                p.override_internal = p.override_value;
                let v = ctx.rand_next_float(if calm { 0.1 } else { 0.5 }, if calm { 0.5 } else { 1.0 });
                p.set_override(v, minute_lerp);
                let w = &mut fl[FLOAT_WIND_INTENSITY];
                w.override_internal = w.override_value;
                let v = ctx.rand_next_float(if calm { 0.0 } else { 0.3 }, if calm { 0.3 } else { 1.0 });
                w.set_override(v, minute_lerp);
                if endless_snow || endless_blizzard {
                    let t = &mut fl[FLOAT_TEMPERATURE];
                    t.override_internal = t.override_value;
                    let v = ctx.rand_next_float(-20.0, 0.0);
                    t.set_override(v, minute_lerp);
                }
            }
        }
        let blizzard_fog = endless_blizzard && ctx.fog_cycle() != 2;
        let fog_cycle = if blizzard_fog { 4 } else { ctx.fog_cycle() };
        let overriding_fog = fog_cycle > 1;
        let no_fog = fog_cycle == 2;
        let endless_fog = fog_cycle >= 3;
        let fog = &mut fl[FLOAT_FOG_INTENSITY];
        if self.fog_override != fog_cycle {
            self.fog_override = fog_cycle;
            fog.set_enable_override(overriding_fog);
            fog.set_override_value(overriding_fog);
            if no_fog {
                fog.set_override(0.0, 1.0);
            } else if endless_fog {
                fog.override_internal = 0.5;
            }
            setting_changed = overriding_fog;
        }
        if endless_fog {
            fog.interpolate = minute_lerp;
            if self.tick_is_hour_change || setting_changed {
                fog.override_internal = fog.override_value;
                let v = ctx.rand_next_float(0.1, 1.0);
                fog.set_override(v, minute_lerp);
            }
        }
    }

    fn update_wind_tick(&mut self, clock: &ClimateClock, ctx: &dyn ClimateContext) {
        if ctx.is_server() {
            return;
        }
        let wind = self.vars.floats[FLOAT_WIND_INTENSITY].final_value;
        self.wind_noise_offset += (4.0e-4 + 6.0e-4 * wind as f64) * clock.multiplier as f64;
        self.wind_noise_base = noise2(0.0, self.wind_noise_offset);
        self.wind_noise_final = self.wind_noise_base;
        if self.wind_noise_final > 0.0 {
            self.wind_noise_final *= 0.04 + 0.1 * wind as f64;
        } else {
            self.wind_noise_final *= 0.04 + 0.1 * wind as f64 + (0.05 * (wind * wind)) as f64;
        }
        let w = clamp01(wind + self.wind_noise_final as f32);
        self.wind_tick_final = w as f64;
    }

    fn update_snow(&mut self, ctx: &mut dyn ClimateContext) {
        let target = |f: f32| (f * 100.0) as i32;
        if ctx.is_client() {
            ctx.event(ClimateEvent::SnowTarget(target(self.snow_frac_now)));
            ctx.event(ClimateEvent::IceQueenSnow(self.can_do_winter_sprites && self.snow_frac_now > 0.2));
            return;
        }
        if ctx.is_force_snow() {
            self.snow_frac_now = 0.7;
            ctx.event(ClimateEvent::SnowTarget(target(self.snow_frac_now)));
            ctx.event(ClimateEvent::IceQueenSnow(self.snow_frac_now > 0.2));
            self.was_force_snow = true;
            return;
        }
        if self.was_force_snow {
            self.snow_frac_now = if self.snow_strength > 7.5 { 1.0 } else { self.snow_strength / 7.5 };
            ctx.event(ClimateEvent::SnowTarget(target(self.snow_frac_now)));
            ctx.event(ClimateEvent::IceQueenSnow(self.can_do_winter_sprites && self.snow_frac_now > 0.2));
            self.was_force_snow = false;
        }
        let p = self.vars.floats[FLOAT_PRECIPITATION_INTENSITY].final_value;
        let snow = self.vars.bools[BOOL_IS_SNOW].final_value;
        if !self.tick_is_hour_change {
            self.can_do_winter_sprites = self.season.as_ref().is_some_and(|s| s.is_season(5)) || self.winter_is_coming;
            if snow && p > self.snow_fall {
                self.snow_fall = p;
            }
            let t = self.vars.floats[FLOAT_TEMPERATURE].final_value;
            if t > 0.0 {
                let mut melt = t / 10.0;
                melt = melt * 0.2 + melt * 0.8 * self.vars.floats[FLOAT_DAYLIGHT_STRENGTH].final_value;
                if melt > self.snow_melt_strength {
                    self.snow_melt_strength = melt;
                }
            }
            if !snow && p > 0.0 {
                self.snow_melt_strength += p;
            }
        } else {
            self.snow_strength += self.snow_fall;
            self.snow_strength -= self.snow_melt_strength;
            self.snow_strength = clamp(0.0, 10.0, self.snow_strength);
            self.snow_frac_now = if self.snow_strength > 7.5 { 1.0 } else { self.snow_strength / 7.5 };
            ctx.event(ClimateEvent::SnowTarget(target(self.snow_frac_now)));
            ctx.event(ClimateEvent::IceQueenSnow(self.can_do_winter_sprites && self.snow_frac_now > 0.2));
            self.snow_fall = 0.0;
            self.snow_melt_strength = 0.0;
        }
    }

    fn update_view_distance(&mut self) {
        let view_mod = self.vars.floats[FLOAT_DAYLIGHT_STRENGTH].final_value;
        let fog_mod = self.vars.floats[FLOAT_FOG_INTENSITY].final_value;
        let mut min = 19.0 - fog_mod * 8.0;
        let mut max = min + 4.0 + 7.0 * view_mod * (1.0 - fog_mod);
        min *= 3.0;
        max *= 3.0;
        self.view_dist_min_max = Some((min, max));
        let v = min + (max - min) * view_mod;
        let f = &mut self.vars.floats[FLOAT_VIEW_DISTANCE];
        f.internal_value = v;
        f.final_value = v;
    }

    /// `forceDayInfoUpdate()`.
    pub fn force_day_info_update(&mut self, clock: &ClimateClock, ctx: &dyn ClimateContext) {
        self.base_season(ctx);
        if let Some(d) = &mut self.current_day {
            d.day = -1;
            d.month = -1;
            d.year = -1;
        }
        self.update_day_info(clock.day_plus_one, clock.month, clock.year);
        let d = self.current_day.as_mut().expect("currentDay");
        d.hour = clock.hour;
        d.minutes = clock.minutes;
    }

    fn update_day_info(&mut self, day: i32, month: i32, year: i32) {
        self.tick_is_day_change = false;
        let changed = match &self.current_day {
            None => true,
            Some(d) => d.day != day || d.month != month || d.year != year,
        };
        if !changed {
            return;
        }
        self.tick_is_day_change = self.current_day.is_some();
        let base = self.season.clone().unwrap_or_default();
        let cur = self.current_day.get_or_insert_with(DayInfo::default);
        set_day_info(cur, day, month, year, 0, &base);
        let prev = self.previous_day.get_or_insert_with(|| DayInfo { season: Some(base.java_clone()), ..Default::default() });
        set_day_info(prev, day, month, year, -1, &base);
        let next = self.next_day.get_or_insert_with(|| DayInfo { season: Some(base.java_clone()), ..Default::default() });
        set_day_info(next, day, month, year, 1, &base);
    }

    fn update_values(&mut self, clock: &ClimateClock, ctx: &mut dyn ClimateContext) {
        let env = self.values_env(ctx);
        let cur = self.current_day.take().expect("currentDay");
        let next = self.next_day.take().expect("nextDay");
        self.climate_values.update_values(self.world_age_hours, clock.time_of_day, &cur, &next, &env);
        self.current_day = Some(cur);
        self.next_day = Some(next);
        let cv = &self.climate_values;
        self.air_mass = cv.get_noise_airmass();
        self.air_mass_temperature = cv.get_air_mass_temperature();
        if self.tick_is_hour_change {
            let air_type = if self.air_mass < 0.0 { -1 } else { 1 };
            if self.current_front.front_type != air_type {
                if !self.disable_weather_generation && (!self.winter_is_coming || clock.world_age_hours > 96.0) {
                    let mut front = self.current_front.clone();
                    if self.the_descending_fog {
                        front.front_type = -1;
                        front.strength = ctx.rand_next_float(0.2, 0.45);
                    } else {
                        self.calculate_weather_front_strength(clock.year, clock.month, clock.day_plus_one, &mut front, clock, ctx);
                    }
                    self.current_front = front.clone();
                    self.init_weather_period(&front, clock, -1, -1.0, ctx);
                }
                self.current_front.set_front_type(air_type);
            }
            // The original evaluates this condition for nothing; only its
            // `Rand` draw matters.
            if !self.winter_is_coming
                && !self.the_descending_fog
                && clock.world_age_hours >= 72.0
                && clock.world_age_hours <= 96.0
                && !self.disable_weather_generation
                && !self.weather_period.is_running
            {
                ctx.rand_next_range(0, 1000);
            }
        }
        let cv = &self.climate_values;
        self.day_do_fog = cv.is_day_do_fog();
        self.day_fog_strength = cv.get_day_fog_strength();
        self.day_fog_strength = if ctx.fog_quality() == 2 {
            0.5 + 0.5 * self.day_fog_strength
        } else {
            0.2 + 0.8 * self.day_fog_strength
        };
        self.base_temperature = cv.get_base_temperature();
        self.day_light_lagged = cv.get_day_light_lagged();
        self.night_lagged = cv.get_day_light_lagged();
        let fl = &mut self.vars.floats;
        fl[FLOAT_TEMPERATURE].internal_value = cv.get_temperature();
        self.vars.bools[BOOL_IS_SNOW].internal_value = cv.is_temperature_is_snow();
        fl[FLOAT_HUMIDITY].internal_value = cv.get_humidity();
        fl[FLOAT_WIND_INTENSITY].internal_value = cv.get_wind_intensity();
        fl[FLOAT_WIND_ANGLE_INTENSITY].internal_value = cv.get_wind_angle_intensity();
        self.wind_power = fl[FLOAT_WIND_INTENSITY].internal_value;
        self.current_front.front_wind_angle_degrees = cv.get_wind_angle_degrees();
        fl[FLOAT_CLOUD_INTENSITY].internal_value = cv.get_cloud_intensity();
        fl[FLOAT_PRECIPITATION_INTENSITY].internal_value = 0.0;
        fl[FLOAT_NIGHT_STRENGTH].internal_value = cv.get_night_strength();
        fl[FLOAT_DAYLIGHT_STRENGTH].internal_value = cv.get_day_light_strength();
        fl[FLOAT_AMBIENT].internal_value = cv.get_ambient();
        fl[FLOAT_DESATURATION].internal_value = cv.get_desaturation();
        let season = self.season.as_ref().expect("season");
        let cur_season = season.get_season();
        let prog = season.get_season_progression();
        let (tval, from, to) = match cur_season {
            2 => (0.5 + prog * 0.5, 3, 0),
            3 => (prog * 0.5, 0, 1),
            4 if prog < 0.5 => (0.5 + prog, 0, 1),
            4 => (prog - 0.5, 1, 2),
            5 if prog < 0.5 => (0.5 + prog, 1, 2),
            5 => (prog - 0.5, 2, 3),
            1 if prog < 0.5 => (0.5 + prog, 2, 3),
            1 => (prog - 0.5, 3, 0),
            _ => (0.0, 0, 0),
        };
        let cloudy_t = cv.get_cloudy_t();
        self.col_dawn = self.season_color_dawn.update(cloudy_t, tval, from, to);
        self.col_day = self.season_color_day.update(cloudy_t, tval, from, to);
        self.col_dusk = self.season_color_dusk.update(cloudy_t, tval, from, to);
        let time = cv.get_time();
        let dawn = cv.get_dawn();
        let dusk = cv.get_dusk();
        let noon = cv.get_noon();
        let fog_duration = cv.get_day_fog_duration();
        let shader = ctx.weather_shader_active();
        let fog_quality = ctx.fog_quality();
        if !self.the_descending_fog {
            if self.day_do_fog && self.day_fog_strength > 0.0 && time > dawn - 2.0 && time < dawn + fog_duration {
                let mut lerpfog = time_lerp_hours(time, dawn - 2.0, dawn + fog_duration, true);
                lerpfog = clamp(0.0, 1.0, lerpfog * (fog_duration / 3.0));
                self.fog_lerp_value = lerpfog;
                fl[FLOAT_CLOUD_INTENSITY].internal_value = lerp(lerpfog, fl[FLOAT_CLOUD_INTENSITY].internal_value, 0.0);
                let fog_val = self.day_fog_strength;
                fl[FLOAT_FOG_INTENSITY].internal_value = clerp(lerpfog, 0.0, fog_val);
                let d = fl[FLOAT_DESATURATION].internal_value;
                fl[FLOAT_DESATURATION].internal_value = if shader && fog_quality != 2 {
                    clerp(lerpfog, d, 0.65 * fog_val)
                } else {
                    clerp(lerpfog, d, 0.8 * fog_val)
                };
            } else {
                fl[FLOAT_FOG_INTENSITY].internal_value = 0.0;
            }
        } else {
            fl[FLOAT_FOG_INTENSITY].internal_value =
                if clock.world_age_hours < 72.0 { clock.world_age_hours as f32 / 72.0 } else { 1.0 };
            fl[FLOAT_CLOUD_INTENSITY].internal_value =
                jmin(fl[FLOAT_CLOUD_INTENSITY].internal_value, 1.0 - fl[FLOAT_FOG_INTENSITY].internal_value);
            if self.weather_period.is_running {
                fl[FLOAT_FOG_INTENSITY].internal_value = jmin(fl[FLOAT_FOG_INTENSITY].internal_value, 0.6);
            }
            if fog_quality == 2 {
                fl[FLOAT_FOG_INTENSITY].internal_value *= 0.93;
                fl[FLOAT_DESATURATION].internal_value = 0.8 * fl[FLOAT_FOG_INTENSITY].internal_value;
            } else {
                fl[FLOAT_DESATURATION].internal_value = 0.65 * fl[FLOAT_FOG_INTENSITY].internal_value;
            }
        }
        fl[FLOAT_HUMIDITY].internal_value =
            clamp01(fl[FLOAT_HUMIDITY].internal_value + fl[FLOAT_FOG_INTENSITY].internal_value * 0.6);
        let day_max = cv.get_day_light_strength_base();
        let dusk_dawn_min = if shader { 0.8 } else { 0.25 } * cv.get_day_light_strength_base();
        let gl = self.vars.colors[COLOR_GLOBAL_LIGHT].internal_value.clone();
        if time < dawn || time > dusk {
            let total = 24.0 - dusk + dawn;
            let t = if time > dusk { (time - dusk) / total } else { (24.0 - dusk + time) / total };
            interp_into(&self.col_dusk, &self.col_dawn, t, &gl);
            fl[FLOAT_GLOBAL_LIGHT_INTENSITY].internal_value = lerp(cv.get_lerp_night(), dusk_dawn_min, 0.4);
        } else if time < noon + 2.0 {
            let t = (time - dawn) / (noon + 2.0 - dawn);
            interp_into(&self.col_dawn, &self.col_day, t, &gl);
            fl[FLOAT_GLOBAL_LIGHT_INTENSITY].internal_value = lerp(t, dusk_dawn_min, day_max);
        } else {
            let t = (time - (noon + 2.0)) / (dusk - (noon + 2.0));
            interp_into(&self.col_day, &self.col_dusk, t, &gl);
            fl[FLOAT_GLOBAL_LIGHT_INTENSITY].internal_value = lerp(t, day_max, dusk_dawn_min);
        }
        let fog_i = fl[FLOAT_FOG_INTENSITY].internal_value;
        if fog_i > 0.0 {
            let fog_col = if shader {
                if fog_quality == 2 {
                    &self.col_fog
                } else {
                    &self.col_fog_new
                }
            } else {
                &self.col_fog_legacy
            };
            interp_into(&gl, fog_col, fog_i, &gl);
            fl[FLOAT_GLOBAL_LIGHT_INTENSITY].internal_value =
                clerp(self.fog_lerp_value, fl[FLOAT_GLOBAL_LIGHT_INTENSITY].internal_value, 0.8);
        }
        interp_into(&self.col_night_no_moon, &self.col_night_moon, self.moon.get_moon_float(), &self.col_night);
        interp_into(&gl, &self.col_night, fl[FLOAT_NIGHT_STRENGTH].internal_value, &gl);
    }

    fn init_weather_period(
        &mut self,
        front: &AirFront,
        clock: &ClimateClock,
        stage: i32,
        duration: f32,
        ctx: &mut dyn ClimateContext,
    ) {
        let values_env = self.values_env(ctx);
        let mode = ctx.game_mode();
        self.base_season(ctx);
        let base = self.season.clone().expect("season");
        let current = self.get_season().cloned().unwrap_or_else(|| base.clone());
        let env = PeriodInitEnv {
            clock,
            base_season: &base,
            current_season: &current,
            simplex_offset_a: self.simplex_offset_a,
            simplex_offset_b: self.simplex_offset_b,
            values_env,
            game_mode: &mode,
        };
        self.weather_period.init(
            front,
            self.world_age_hours,
            clock.year,
            clock.month,
            clock.day_plus_one,
            stage,
            duration,
            &env,
            ctx,
        );
    }

    /// `stopWeatherAndThunder()`.
    pub fn stop_weather_and_thunder(&mut self, ctx: &mut dyn ClimateContext) {
        if ctx.is_client() {
            return;
        }
        self.weather_period.stop_weather_period(Some(&mut self.vars), ctx);
        self.thunder_storm.stop_all_clouds();
        if ctx.is_server() {
            ctx.event(ClimateEvent::TransmitClimatePacket(1));
        }
    }

    /// `triggerWinterIsComingStorm()`.
    pub fn trigger_winter_is_coming_storm(&mut self, clock: &ClimateClock, ctx: &mut dyn ClimateContext) {
        if !ctx.is_client() && !self.weather_period.is_running {
            let mut front = AirFront::default();
            front.copy_from(&self.current_front);
            front.strength = 0.95;
            front.front_type = 1;
            self.init_weather_period(&front, clock, -1, -1.0, ctx);
        }
    }

    /// `triggerCustomWeather(strength, warmFront)`.
    pub fn trigger_custom_weather(&mut self, strength: f32, warm: bool, clock: &ClimateClock, ctx: &mut dyn ClimateContext) -> bool {
        if ctx.is_client() || self.weather_period.is_running {
            return false;
        }
        let front = AirFront { strength, front_type: if warm { 1 } else { -1 }, ..AirFront::default() };
        self.init_weather_period(&front, clock, -1, -1.0, ctx);
        true
    }

    /// `triggerCustomWeatherStage(stage, duration)`.
    pub fn trigger_custom_weather_stage(
        &mut self,
        stage: i32,
        duration: f32,
        clock: &ClimateClock,
        ctx: &mut dyn ClimateContext,
    ) -> bool {
        if ctx.is_client() || self.weather_period.is_running {
            return false;
        }
        let front = AirFront { strength: 0.95, front_type: 1, ..AirFront::default() };
        self.init_weather_period(&front, clock, stage, duration, ctx);
        true
    }

    /// `triggerKateBobIntroStorm(centerX, centerY, duration, strength,
    /// initialProgress, angle, initialPuddles, cloudcolor)`. The puddle
    /// values (`IsoPuddles`) are not ported.
    #[allow(clippy::too_many_arguments)]
    pub fn trigger_kate_bob_intro_storm(
        &mut self,
        center_x: i32,
        center_y: i32,
        duration: f64,
        strength: f32,
        initial_progress: f32,
        angle: f32,
        cloud_color: Option<ColorRef>,
        clock: &ClimateClock,
        ctx: &mut dyn ClimateContext,
    ) {
        if ctx.is_client() {
            return;
        }
        self.stop_weather_and_thunder(ctx);
        let (a, b) = (self.simplex_offset_a, self.simplex_offset_b);
        let wp = &mut self.weather_period;
        if wp.start_create_modded_period(true, strength, angle, clock, a, b, ctx) {
            wp.set_kate_bob_storm_progress(initial_progress);
            wp.set_kate_bob_storm_coords(center_x, center_y);
            wp.create_and_add_stage(super::period::STAGE_KATEBOB_STORM, duration, None, ctx);
            wp.create_and_add_stage(super::period::STAGE_HEAVY_PRECIP, duration / 2.0, None, ctx);
            wp.create_and_add_stage(super::period::STAGE_CLEARING, duration / 4.0, None, ctx);
            wp.end_create_modded_period(ctx);
            wp.cloud_color = cloud_color.unwrap_or_else(|| wp.cloud_color_blueish.clone());
        }
    }

    /// `getSeasonColor(segment, temperature, season)`.
    pub fn get_season_color(&self, segment: i32, temperature: usize, season: usize) -> Option<ColorRef> {
        let s = match segment {
            0 => &self.season_color_dawn,
            1 => &self.season_color_day,
            2 => &self.season_color_dusk,
            _ => return None,
        };
        s.colors.get(temperature).and_then(|r| r.get(season)).cloned()
    }

    /// `setSeasonColorDawn/Day/Dusk(temperature, season, r, g, b, a,
    /// exterior)` (`segment` 0 dawn, 1 day, 2 dusk).
    #[allow(clippy::too_many_arguments)]
    pub fn set_season_color(&mut self, segment: i32, temperature: usize, season: usize, rgba: [f32; 4], exterior: bool) {
        if let Some(c) = self.get_season_color(segment, temperature, season) {
            let mut c = c.borrow_mut();
            if exterior {
                c.exterior = rgba;
            } else {
                c.interior = rgba;
            }
        }
    }

    /// What `RenderSettings` copies for player `player_index` this frame
    /// (`updateRenderSettings` then `applyLightningForPlayer`).
    /// `weather_shader` is `SceneShaderStore.weatherShader != null &&
    /// Core.getOffscreenBuffer() != null`.
    pub fn render_values(&self, player_index: usize, weather_shader: bool) -> ClimateRenderValues {
        let mut v = ClimateRenderValues {
            cm_global_light: *self.get_global_light().borrow(),
            cm_global_light_intensity: self.get_global_light_intensity(),
            cm_ambient: self.get_ambient(),
            cm_day_light_strength: self.get_day_light_strength(),
            cm_night_strength: self.get_night_strength(),
            cm_desaturation: self.get_desaturation(),
            cm_view_distance: self.get_view_distance(),
            cm_fog_intensity: self.get_fog_intensity(),
            moon_float: self.moon.get_moon_float(),
            color_new_fog: *self.get_color_new_fog().borrow(),
            precipitation_intensity: self.get_precipitation_intensity(),
            precipitation_is_snow: self.get_precipitation_is_snow(),
            wind_intensity: self.get_wind_intensity(),
            wind_angle_intensity: self.get_wind_angle_intensity(),
            wind_precip_intensity: self.wind_tick_final as f32 * self.wind_tick_final as f32,
            cloud_intensity: self.get_cloud_intensity(),
            snow_frac_now: self.snow_frac_now,
            humidity: self.get_humidity(),
            temperature: self.get_temperature(),
            wind_angle_degrees: self.get_wind_angle_degrees(),
        };
        self.thunder_storm.apply_lightning_for_player(&mut v, player_index, weather_shader);
        v
    }
}

/// `CalculateWeatherFrontStrength` on a given `climateValuesFronts`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn calculate_front(
    fronts: &mut ClimateValues,
    year: i32,
    month: i32,
    day: i32,
    front: &mut AirFront,
    clock: &ClimateClock,
    base: &ErosionSeason,
    env: &ValuesEnv,
) {
    let mut cal = JavaCalendar::new(year, month, day, 0, 0);
    cal.add(DAY_OF_MONTH, -3);
    let target = front.front_type;
    for _ in 0..4 {
        fronts.poll_date(&cal, clock, base, env);
        let airmass = fronts.get_air_front_airmass();
        let t = if airmass < 0.0 { -1 } else { 1 };
        if t == target {
            front.add_day_sample(airmass);
        }
        cal.add(DAY_OF_MONTH, 1);
    }
    let _ = (YEAR, MONTH);
}
