//! `zombie.iso.weather.ClimateForecaster`: forty sampled days (ten past,
//! today, twenty-nine ahead), each with a dummy `WeatherPeriod` for the
//! front that starts that day.

use super::calendar::{JavaCalendar, DAY_OF_MONTH, HOUR_OF_DAY, MONTH, YEAR};
use super::manager::{AirFront, ClimateManager};
use super::period::{PeriodInitEnv, WeatherPeriod, MAX_TEMPERATURE_INFLUENCE};
use super::values::{ClimateValues, ValuesEnv};
use super::{wind_angle_string, ClimateClock, ClimateContext, PeriodId};

const OFFSET_TODAY: i32 = 10;
const COUNT: usize = 40;

/// `ClimateForecaster.ForecastValue`: day, night and total min/max/mean.
#[derive(Clone, Debug, PartialEq)]
pub struct ForecastValue {
    pub day_min: f32,
    pub day_max: f32,
    pub day_mean: f32,
    day_mean_ticks: i32,
    pub night_min: f32,
    pub night_max: f32,
    pub night_mean: f32,
    night_mean_ticks: i32,
    pub total_min: f32,
    pub total_max: f32,
    pub total_mean: f32,
    total_mean_ticks: i32,
}

impl Default for ForecastValue {
    fn default() -> Self {
        ForecastValue {
            day_min: 10000.0,
            day_max: -10000.0,
            day_mean: 0.0,
            day_mean_ticks: 0,
            night_min: 10000.0,
            night_max: -10000.0,
            night_mean: 0.0,
            night_mean_ticks: 0,
            total_min: 10000.0,
            total_max: -10000.0,
            total_mean: 0.0,
            total_mean_ticks: 0,
        }
    }
}

impl ForecastValue {
    fn add(&mut self, v: f32, is_day: bool) {
        if is_day {
            if v < self.day_min {
                self.day_min = v;
            }
            if v > self.day_max {
                self.day_max = v;
            }
            self.day_mean += v;
            self.day_mean_ticks += 1;
        } else {
            if v < self.night_min {
                self.night_min = v;
            }
            if v > self.night_max {
                self.night_max = v;
            }
            self.night_mean += v;
            self.night_mean_ticks += 1;
        }
        if v < self.total_min {
            self.total_min = v;
        }
        if v > self.total_max {
            self.total_max = v;
        }
        self.total_mean += v;
        self.total_mean_ticks += 1;
    }

    fn calculate(&mut self) {
        self.total_mean = if self.total_mean_ticks <= 0 { 0.0 } else { self.total_mean / self.total_mean_ticks as f32 };
        if self.day_mean_ticks <= 0 {
            self.day_min = self.total_min;
            self.day_max = self.total_max;
            self.day_mean = self.total_mean;
        } else {
            self.day_mean /= self.day_mean_ticks as f32;
        }
        if self.night_mean_ticks <= 0 {
            self.night_min = self.total_min;
            self.night_max = self.total_max;
            self.night_mean = self.total_mean;
        } else {
            self.night_mean /= self.night_mean_ticks as f32;
        }
    }
}

/// `ClimateForecaster.DayForecast`.
#[derive(Clone, Debug)]
pub struct DayForecast {
    /// Which of the forty objects this is (stable while they rotate).
    pub id: usize,
    pub index_offset: i32,
    /// The date `getName()` shows (`IGUI_Forecaster_Day` + ": y - m - d");
    /// `None` until sampled ("Day x").
    pub name_date: Option<(i32, i32, i32)>,
    pub weather_period: WeatherPeriod,
    pub temperature: ForecastValue,
    pub humidity: ForecastValue,
    pub wind_direction: ForecastValue,
    pub wind_power: ForecastValue,
    pub cloudiness: ForecastValue,
    pub weather_starts: bool,
    pub weather_start_time: f32,
    pub weather_end_time: f32,
    pub chance_on_snow: bool,
    pub air_front_string: &'static str,
    pub has_fog: bool,
    pub fog_strength: f32,
    pub fog_duration: f32,
    pub air_front: AirFront,
    /// `weatherOverlap` by [`DayForecast::id`].
    pub weather_overlap: Option<usize>,
    pub has_heavy_rain: bool,
    pub has_storm: bool,
    pub has_tropical_storm: bool,
    pub has_blizzard: bool,
    pub dawn: f32,
    pub dusk: f32,
    pub day_light_hours: f32,
    pub weather_stages: Vec<i32>,
}

impl DayForecast {
    fn new(id: usize, values: ClimateValues) -> Self {
        let mut wp = WeatherPeriod::new(PeriodId::Forecast(id), values);
        wp.is_dummy = true;
        DayForecast {
            id,
            index_offset: 0,
            name_date: None,
            weather_period: wp,
            temperature: ForecastValue::default(),
            humidity: ForecastValue::default(),
            wind_direction: ForecastValue::default(),
            wind_power: ForecastValue::default(),
            cloudiness: ForecastValue::default(),
            weather_starts: false,
            weather_start_time: 0.0,
            weather_end_time: 24.0,
            chance_on_snow: false,
            air_front_string: "",
            has_fog: false,
            fog_strength: 0.0,
            fog_duration: 0.0,
            air_front: AirFront::default(),
            weather_overlap: None,
            has_heavy_rain: false,
            has_storm: false,
            has_tropical_storm: false,
            has_blizzard: false,
            dawn: 0.0,
            dusk: 0.0,
            day_light_hours: 0.0,
            weather_stages: Vec::new(),
        }
    }

    /// `getMeanWindAngleString()`.
    pub fn mean_wind_angle_string(&self) -> &'static str {
        wind_angle_string(self.wind_direction.total_mean)
    }

    fn reset(&mut self, ctx: &mut dyn ClimateContext) {
        self.weather_period.stop_weather_period(None, ctx);
        self.temperature = ForecastValue::default();
        self.humidity = ForecastValue::default();
        self.wind_direction = ForecastValue::default();
        self.wind_power = ForecastValue::default();
        self.cloudiness = ForecastValue::default();
        self.weather_starts = false;
        self.weather_start_time = 0.0;
        self.weather_end_time = 24.0;
        self.chance_on_snow = false;
        self.has_fog = false;
        self.fog_strength = 0.0;
        self.fog_duration = 0.0;
        self.weather_overlap = None;
        self.has_heavy_rain = false;
        self.has_storm = false;
        self.has_tropical_storm = false;
        self.has_blizzard = false;
        self.weather_stages.clear();
    }
}

/// `ClimateForecaster`.
#[derive(Clone, Debug, Default)]
pub struct ClimateForecaster {
    climate_values: Option<ClimateValues>,
    /// `forecasts` in order (index 10 is today); empty before `init`.
    pub forecasts: Vec<DayForecast>,
}

impl ClimateForecaster {
    /// `getForecast(offset)`.
    pub fn get_forecast(&self, offset: i32) -> Option<&DayForecast> {
        let t = OFFSET_TODAY + offset;
        usize::try_from(t).ok().and_then(|i| self.forecasts.get(i))
    }

    /// The forecast with this [`DayForecast::id`].
    pub fn by_id(&self, id: usize) -> Option<&DayForecast> {
        self.forecasts.iter().find(|f| f.id == id)
    }

    pub fn by_id_mut(&mut self, id: usize) -> Option<&mut DayForecast> {
        self.forecasts.iter_mut().find(|f| f.id == id)
    }

    /// `getDaysTillFirstWeather()`.
    pub fn days_till_first_weather(&self) -> i32 {
        let mut first = -1;
        for i in 10..self.forecasts.len().saturating_sub(1) {
            if self.forecasts[i].weather_starts && first < 0 {
                first = i as i32;
            }
        }
        first
    }

    /// `init(climateManager)`.
    pub fn init(&mut self, cm: &mut ClimateManager, clock: &ClimateClock, ctx: &mut dyn ClimateContext) {
        self.climate_values = Some(cm.get_climate_values_copy());
        self.forecasts.clear();
        for i in 0..COUNT {
            let offset = i as i32 - OFFSET_TODAY;
            let mut df = DayForecast::new(i, cm.get_climate_values_copy());
            df.index_offset = offset;
            self.sample_day(cm, &mut df, offset, clock, ctx);
            self.forecasts.push(df);
        }
    }

    /// `updateDayChange(climateManager)`.
    pub fn update_day_change(&mut self, cm: &mut ClimateManager, clock: &ClimateClock, ctx: &mut dyn ClimateContext) {
        if self.forecasts.len() != COUNT {
            return;
        }
        let mut first = self.forecasts.remove(0);
        for (i, f) in self.forecasts.iter_mut().enumerate() {
            f.index_offset = i as i32 - OFFSET_TODAY;
        }
        first.reset(ctx);
        let offset = COUNT as i32 - 1 - OFFSET_TODAY;
        self.sample_day(cm, &mut first, offset, clock, ctx);
        first.index_offset = offset;
        self.forecasts.push(first);
    }

    fn weather_overlap(&self, index: usize, hour: f32) -> Option<usize> {
        let start = index.saturating_sub(10);
        if start == index {
            return None;
        }
        for i in start..index {
            let f = &self.forecasts[i];
            if !f.weather_starts {
                continue;
            }
            let days = f.weather_period.duration as f32 / 24.0;
            let mut end = i as f32 + f.weather_start_time / 24.0;
            let stamp = index as f32 + hour / 24.0;
            end += days;
            if end > stamp {
                return Some(f.id);
            }
        }
        None
    }

    fn sample_day(
        &mut self,
        cm: &mut ClimateManager,
        df: &mut DayForecast,
        day_offset: i32,
        clock: &ClimateClock,
        ctx: &mut dyn ClimateContext,
    ) {
        let mut cal = JavaCalendar::new(clock.year, clock.month, clock.day_plus_one, 0, 0);
        cal.add(DAY_OF_MONTH, day_offset);
        let mut last_front_warm = true;
        let index = (day_offset + OFFSET_TODAY) as usize;
        df.weather_overlap = self.weather_overlap(index, 0.0);
        df.weather_period.stop_weather_period(None, ctx);
        df.name_date = Some((cal.get(YEAR), cal.get(MONTH) + 1, cal.get(DAY_OF_MONTH)));
        let env = ValuesEnv {
            simplex_offset_c: cm.simplex_offset_c,
            simplex_offset_d: cm.simplex_offset_d,
            rain_modifier: ctx.rain_modifier(),
            winter_is_coming: cm.winter_is_coming,
            day_night_cycle: ctx.day_night_cycle(),
        };
        let mut base = cm.season.clone().unwrap_or_default();
        base.day_night_cycle = ctx.day_night_cycle();
        let mut values = self.climate_values.take().unwrap_or_else(|| cm.get_climate_values_copy());
        for i in 0..24 {
            if i != 0 {
                cal.add(HOUR_OF_DAY, 1);
            }
            values.poll_date(&cal, clock, &base, &env);
            if i == 0 {
                last_front_warm = values.get_noise_airmass() >= 0.0;
                df.air_front_string = if last_front_warm { "WARM" } else { "COLD" };
                df.dawn = values.get_dawn();
                df.dusk = values.get_dusk();
                df.day_light_hours = df.dusk - df.dawn;
            }
            let na = values.get_noise_airmass();
            if !df.weather_starts && (last_front_warm && na < 0.0 || !last_front_warm && na >= 0.0) {
                let last_type = if na >= 0.0 { -1 } else { 1 };
                df.air_front.set_front_type(last_type);
                let (y, m, d) = (cal.get(YEAR), cal.get(MONTH), cal.get(DAY_OF_MONTH));
                cm.calculate_weather_front_strength(y, m, d, &mut df.air_front, clock, ctx);
                df.air_front.front_wind_angle_degrees = values.get_wind_angle_degrees();
                if df.air_front.strength >= 0.1 {
                    let overlap = self.weather_overlap(index, i as f32);
                    let overlap_strength =
                        overlap.and_then(|id| self.by_id(id)).map_or(-1.0, |o| o.weather_period.total_strength());
                    if overlap_strength < 0.1 {
                        df.weather_starts = true;
                        df.weather_start_time = i as f32;
                        let mode = ctx.game_mode();
                        let current = cm.get_season().cloned().unwrap_or_else(|| base.clone());
                        let penv = PeriodInitEnv {
                            clock,
                            base_season: &base,
                            current_season: &current,
                            simplex_offset_a: cm.simplex_offset_a,
                            simplex_offset_b: cm.simplex_offset_b,
                            values_env: env,
                            game_mode: &mode,
                        };
                        let front = df.air_front.clone();
                        df.weather_period.init(&front, values.get_cache_world_age_hours(), y, m, d, -1, -1.0, &penv, ctx);
                    }
                }
                if !df.weather_starts {
                    last_front_warm = !last_front_warm;
                }
            }
            let is_day_time = i as f32 > values.get_dawn() && i as f32 <= values.get_dusk();
            let mut temperature = values.get_temperature();
            let humidity = values.get_humidity();
            let mut wind_direction = values.get_wind_angle_degrees();
            let mut wind_power = values.get_wind_intensity();
            let mut cloudiness = values.get_cloud_intensity();
            if df.weather_starts || df.weather_overlap.is_some() {
                let wp = if df.weather_starts {
                    Some(&df.weather_period)
                } else {
                    df.weather_overlap.and_then(|id| self.by_id(id)).map(|o| &o.weather_period)
                };
                if let Some(wp) = wp {
                    wind_direction = wp.wind_angle_degrees();
                    let total = wp.total_strength();
                    match wp.stage_for_world_age(values.get_cache_world_age_hours()).map(|s| s.stage_id) {
                        Some(id) => {
                            if !df.weather_stages.contains(&id) {
                                df.weather_stages.push(id);
                            }
                            match id {
                                7 => {
                                    df.chance_on_snow = true;
                                    wind_power = 0.75 + 0.25 * total;
                                    temperature -= MAX_TEMPERATURE_INFLUENCE * wind_power;
                                    cloudiness = 0.5 + 0.5 * wind_power;
                                    df.has_blizzard = true;
                                }
                                2 => {
                                    wind_power = 0.5 * total;
                                    temperature -= MAX_TEMPERATURE_INFLUENCE * wind_power;
                                    cloudiness = 0.5 + 0.5 * wind_power;
                                    df.has_heavy_rain = true;
                                }
                                3 => {
                                    wind_power = 0.2 + 0.5 * total;
                                    temperature -= MAX_TEMPERATURE_INFLUENCE * wind_power;
                                    cloudiness = 0.5 + 0.5 * wind_power;
                                    df.has_storm = true;
                                }
                                8 => {
                                    wind_power = 0.4 + 0.6 * total;
                                    temperature -= MAX_TEMPERATURE_INFLUENCE * wind_power;
                                    cloudiness = 0.5 + 0.5 * wind_power;
                                    df.has_tropical_storm = true;
                                }
                                _ => {
                                    if id == 1 {
                                        df.has_heavy_rain = true;
                                    }
                                    temperature -= MAX_TEMPERATURE_INFLUENCE * 0.25;
                                    cloudiness = 0.35 + 0.5 * total;
                                }
                            }
                        }
                        None => {
                            if df.weather_overlap.is_some() && (i as f32) < df.weather_end_time {
                                df.weather_end_time = i as f32;
                            }
                        }
                    }
                }
                if temperature < 0.0 {
                    df.chance_on_snow = true;
                }
            }
            df.temperature.add(temperature, is_day_time);
            df.humidity.add(humidity, is_day_time);
            df.wind_direction.add(wind_direction, is_day_time);
            df.wind_power.add(wind_power, is_day_time);
            df.cloudiness.add(cloudiness, is_day_time);
        }
        df.temperature.calculate();
        df.humidity.calculate();
        df.wind_direction.calculate();
        df.wind_power.calculate();
        df.cloudiness.calculate();
        df.has_fog = values.is_day_do_fog();
        df.fog_strength = values.get_day_fog_strength();
        df.fog_duration = values.get_day_fog_duration();
        self.climate_values = Some(values);
    }
}
