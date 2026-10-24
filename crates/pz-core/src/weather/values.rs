//! `zombie.iso.weather.ClimateValues` (the noise-driven base climate for a
//! moment in time) and `ClimateManager.DayInfo`.

use super::calendar::{JavaCalendar, DAY_OF_MONTH, HOUR_OF_DAY, MINUTE, MONTH, YEAR};
use super::season::ErosionSeason;
use super::simplex::noise2;
use super::{air_mass_noise_frequency_mod, clamp, clamp01, clerp, lerp, time_lerp_hours, ClimateClock};
use crate::random::JavaRandom;

/// `ClimateManager.DayInfo`.
#[derive(Clone, Debug, Default)]
pub struct DayInfo {
    pub day: i32,
    pub month: i32,
    pub year: i32,
    pub hour: i32,
    pub minutes: i32,
    pub date_value: i64,
    pub calendar: Option<JavaCalendar>,
    pub season: Option<ErosionSeason>,
}

impl DayInfo {
    /// `set(day, month, year)`.
    pub fn set(&mut self, day: i32, month: i32, year: i32) {
        let c = JavaCalendar::new(year, month, day, 0, 0);
        self.date_value = c.time_in_millis();
        self.calendar = Some(c);
        self.day = day;
        self.month = month;
        self.year = year;
    }

    /// The season (always set once the climate filled the day).
    pub fn season(&self) -> &ErosionSeason {
        self.season.as_ref().expect("DayInfo.season")
    }
}

/// `ClimateManager.setDayInfo(dayInfo, day, month, year, dayOffset)`;
/// `base` is the season a missing one is cloned from.
pub fn set_day_info(d: &mut DayInfo, day: i32, month: i32, year: i32, offset: i32, base: &ErosionSeason) {
    let mut c = JavaCalendar::new(year, month, day, 0, 0);
    c.add(DAY_OF_MONTH, offset);
    d.day = c.get(DAY_OF_MONTH);
    d.month = c.get(MONTH);
    d.year = c.get(YEAR);
    d.date_value = c.time_in_millis();
    d.calendar = Some(c);
    let s = d.season.get_or_insert_with(|| base.java_clone());
    s.day_night_cycle = base.day_night_cycle;
    s.set_day(d.day, d.month, d.year);
}

/// What `ClimateValues.updateValues` reads from outside: the manager's
/// current simplex offsets C and D (the seed uses the live values),
/// `SandboxOptions.getRainModifier()`, `ClimateManager.winterIsComing` and
/// `SandboxOptions.dayNightCycle`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValuesEnv {
    pub simplex_offset_c: f64,
    pub simplex_offset_d: f64,
    pub rain_modifier: i32,
    pub winter_is_coming: bool,
    pub day_night_cycle: i32,
}

/// `ClimateValues`.
#[derive(Clone, Debug)]
pub struct ClimateValues {
    simplex_offset_a: f64,
    simplex_offset_b: f64,
    simplex_offset_c: f64,
    simplex_offset_d: f64,
    time: f32,
    dawn: f32,
    dusk: f32,
    noon: f32,
    day_mean_temperature: f32,
    air_mass_noise_frequency_mod: f64,
    noise_airmass: f32,
    air_mass_temperature: f32,
    base_temperature: f32,
    day_light_lagged: f32,
    night_lagged: f32,
    temperature: f32,
    temperature_is_snow: bool,
    humidity: f32,
    wind_intensity: f32,
    wind_angle_intensity: f32,
    wind_angle_degrees: f32,
    night_strength: f32,
    day_light_strength: f32,
    ambient: f32,
    desaturation: f32,
    day_light_strength_base: f32,
    lerp_night: f32,
    cloudy_t: f32,
    cloud_intensity: f32,
    air_front_airmass: f32,
    day_do_fog: bool,
    day_fog_strength: f32,
    day_fog_duration: f32,
    test_current_day: Option<Box<DayInfo>>,
    test_next_day: Option<Box<DayInfo>>,
    cache_world_age_hours: f64,
    cache_year: i32,
    cache_month: i32,
    cache_day: i32,
    seeded_random: JavaRandom,
}

impl ClimateValues {
    /// `new ClimateValues(clim)`: the offsets are the manager's at this time.
    pub fn new(offsets: [f64; 4]) -> Self {
        ClimateValues {
            simplex_offset_a: offsets[0],
            simplex_offset_b: offsets[1],
            simplex_offset_c: offsets[2],
            simplex_offset_d: offsets[3],
            time: 0.0,
            dawn: 0.0,
            dusk: 0.0,
            noon: 0.0,
            day_mean_temperature: 0.0,
            air_mass_noise_frequency_mod: 0.0,
            noise_airmass: 0.0,
            air_mass_temperature: 0.0,
            base_temperature: 0.0,
            day_light_lagged: 0.0,
            night_lagged: 0.0,
            temperature: 0.0,
            temperature_is_snow: false,
            humidity: 0.0,
            wind_intensity: 0.0,
            wind_angle_intensity: 0.0,
            wind_angle_degrees: 0.0,
            night_strength: 0.0,
            day_light_strength: 0.0,
            ambient: 0.0,
            desaturation: 0.0,
            day_light_strength_base: 0.0,
            lerp_night: 0.0,
            cloudy_t: 0.0,
            cloud_intensity: 0.0,
            air_front_airmass: 0.0,
            day_do_fog: false,
            day_fog_strength: 0.0,
            day_fog_duration: 0.0,
            test_current_day: None,
            test_next_day: None,
            cache_world_age_hours: 0.0,
            cache_year: 0,
            cache_month: 0,
            cache_day: 0,
            seeded_random: JavaRandom::new(1984),
        }
    }

    /// `getCopy()`: a new instance (fresh `Random(1984)`, the manager's
    /// current offsets) with these values copied.
    pub fn get_copy(&self, manager_offsets: [f64; 4]) -> ClimateValues {
        let mut c = ClimateValues::new(manager_offsets);
        self.copy_values(&mut c);
        c
    }

    /// `CopyValues(copy)`.
    pub fn copy_values(&self, c: &mut ClimateValues) {
        c.time = self.time;
        c.dawn = self.dawn;
        c.dusk = self.dusk;
        c.noon = self.noon;
        c.day_mean_temperature = self.day_mean_temperature;
        c.air_mass_noise_frequency_mod = self.air_mass_noise_frequency_mod;
        c.noise_airmass = self.noise_airmass;
        c.air_mass_temperature = self.air_mass_temperature;
        c.base_temperature = self.base_temperature;
        c.day_light_lagged = self.day_light_lagged;
        c.night_lagged = self.night_lagged;
        c.temperature = self.temperature;
        c.temperature_is_snow = self.temperature_is_snow;
        c.humidity = self.humidity;
        c.wind_intensity = self.wind_intensity;
        c.wind_angle_intensity = self.wind_angle_intensity;
        c.wind_angle_degrees = self.wind_angle_degrees;
        c.night_strength = self.night_strength;
        c.day_light_strength = self.day_light_strength;
        c.ambient = self.ambient;
        c.desaturation = self.desaturation;
        c.day_light_strength_base = self.day_light_strength_base;
        c.lerp_night = self.lerp_night;
        c.cloudy_t = self.cloudy_t;
        c.cloud_intensity = self.cloud_intensity;
        c.air_front_airmass = self.air_front_airmass;
        c.day_do_fog = self.day_do_fog;
        c.day_fog_strength = self.day_fog_strength;
        c.day_fog_duration = self.day_fog_duration;
        c.cache_world_age_hours = self.cache_world_age_hours;
        c.cache_year = self.cache_year;
        c.cache_month = self.cache_month;
        c.cache_day = self.cache_day;
    }

    /// `pollDate(year, month, dayOfMonth, hourOfDay, minute)`.
    #[allow(clippy::too_many_arguments)]
    pub fn poll_date_ymdhm(
        &mut self,
        year: i32,
        month: i32,
        day: i32,
        hour: i32,
        minute: i32,
        clock: &ClimateClock,
        base: &ErosionSeason,
        env: &ValuesEnv,
    ) {
        self.poll_date(&JavaCalendar::new(year, month, day, hour, minute), clock, base, env);
    }

    /// `pollDate(calendar)`: the values at that moment, with the world age
    /// offset from the clock's.
    pub fn poll_date(&mut self, cal: &JavaCalendar, clock: &ClimateClock, base: &ErosionSeason, env: &ValuesEnv) {
        let mut cur = self.test_current_day.take().unwrap_or_default();
        let mut next = self.test_next_day.take().unwrap_or_default();
        let current_world_age_hours = clock.world_age_hours;
        let (d, m, y) = (cal.get(DAY_OF_MONTH), cal.get(MONTH), cal.get(YEAR));
        set_day_info(&mut cur, d, m, y, 0, base);
        set_day_info(&mut next, d, m, y, 1, base);
        let now = JavaCalendar::new(clock.year, clock.month, clock.day_plus_one, clock.hour, clock.minutes);
        let difference = now.minutes_until(cal) as f64 / 60.0;
        let poll_world_age_hours = current_world_age_hours + difference;
        let poll_time = cal.get(HOUR_OF_DAY) as f32 + cal.get(MINUTE) as f32 / 60.0;
        self.update_values(poll_world_age_hours, poll_time, &cur, &next, env);
        self.test_current_day = Some(cur);
        self.test_next_day = Some(next);
    }

    /// `updateValues(worldAgeHours, time, currentDay, nextDay)`.
    pub fn update_values(&mut self, world_age_hours: f64, time: f32, cur: &DayInfo, next: &DayInfo, env: &ValuesEnv) {
        let cs = cur.season();
        let ns = next.season();
        if cur.year != self.cache_year || cur.month != self.cache_month || cur.day != self.cache_day {
            let base_seed_a = env.simplex_offset_c as i32;
            let base_seed_b = env.simplex_offset_d as i32;
            let mut seed = ((cur.year - 1990).wrapping_mul(100_000)) as i64;
            seed += (cur.month.wrapping_mul(cur.day).wrapping_mul(1234)) as i64;
            seed += ((cur.year - 1990).wrapping_mul(cur.month).wrapping_mul(10_000)) as i64;
            seed += (base_seed_b.wrapping_sub(base_seed_a).wrapping_mul(cur.day)) as i64;
            self.seeded_random.set_seed(seed);
            self.day_fog_strength = 0.0;
            self.day_do_fog = false;
            self.day_fog_duration = 0.0;
            let r = self.seeded_random.next_int(1000) as f32;
            self.day_do_fog = r < 200.0;
            if self.day_do_fog {
                self.day_fog_duration = 4.0;
                if r < 25.0 {
                    self.day_fog_strength = 1.0;
                    self.day_fog_duration += 2.0;
                } else {
                    self.day_fog_strength = self.seeded_random.next_float();
                }
                let mut mean_temp = cs.get_day_mean_temperature();
                let airmass_sample = noise2(
                    self.simplex_offset_a,
                    (world_age_hours + 12.0 - 48.0) / air_mass_noise_frequency_mod(env.rain_modifier),
                ) as f32;
                mean_temp += airmass_sample * 8.0;
                let randy = self.seeded_random.next_float();
                if mean_temp < 0.0 {
                    self.day_fog_duration += 5.0 * self.day_fog_strength;
                    self.day_fog_duration += 8.0 * randy;
                } else if mean_temp < 10.0 {
                    self.day_fog_duration += 2.5 * self.day_fog_strength;
                    self.day_fog_duration += 5.0 * randy;
                } else if mean_temp < 20.0 {
                    self.day_fog_duration += 1.5 * self.day_fog_strength;
                    self.day_fog_duration += 2.5 * randy;
                } else {
                    self.day_fog_duration += 1.0 * self.day_fog_strength;
                    self.day_fog_duration += 1.0 * randy;
                }
                if self.day_fog_duration > 24.0 - cs.get_dawn() {
                    self.day_fog_duration = 24.0 - cs.get_dawn() - 1.0;
                }
            }
        }
        self.cache_world_age_hours = world_age_hours;
        self.cache_year = cur.year;
        self.cache_month = cur.month;
        self.cache_day = cur.day;
        self.time = time;
        self.dawn = cs.get_dawn();
        self.dusk = cs.get_dusk();
        self.noon = cs.get_day_high_noon();
        self.day_mean_temperature = cs.get_day_mean_temperature();
        let day_lerp_val = time / 24.0;
        let day_perc_time_now = lerp(day_lerp_val, cs.get_cur_day_percent(), ns.get_cur_day_percent());
        self.air_mass_noise_frequency_mod = air_mass_noise_frequency_mod(env.rain_modifier);
        let fm = self.air_mass_noise_frequency_mod;
        self.noise_airmass = noise2(self.simplex_offset_a, world_age_hours / fm) as f32;
        let noise_humidity = noise2(self.simplex_offset_c, world_age_hours / fm) as f32;
        self.air_mass_temperature = noise2(self.simplex_offset_a, (world_age_hours - 48.0) / fm) as f32;
        let daymidpoint = world_age_hours.floor() + 12.0;
        self.air_front_airmass = noise2(self.simplex_offset_a, daymidpoint / fm) as f32;
        let cur_mean_temperature = clerp(day_lerp_val, cs.get_day_mean_temperature(), ns.get_day_mean_temperature());
        self.base_temperature = cur_mean_temperature + self.air_mass_temperature * 8.0;
        let mut ddusk = self.dusk + 4.0;
        if ddusk >= 24.0 {
            ddusk -= 24.0;
        }
        self.day_light_lagged = time_lerp_hours(time, self.dawn + 4.0, ddusk, true);
        let mut temp_mod = 5.0 * (1.0 - self.day_light_lagged);
        self.night_lagged = time_lerp_hours(time, ddusk, self.dawn + 4.0, true);
        temp_mod += 5.0 * self.night_lagged;
        self.temperature = self.base_temperature + 1.0 - temp_mod;
        self.temperature_is_snow = self.temperature < 0.0 || env.winter_is_coming;
        let mut cur_temp_offset = self.temperature;
        cur_temp_offset = (45.0 - cur_temp_offset) / 90.0;
        cur_temp_offset = clamp01(1.0 - cur_temp_offset);
        let noise_hum = (1.0 + noise_humidity) * 0.5;
        self.humidity = noise_hum * cur_temp_offset;
        let wind_mod = 1.0 - (self.air_mass_temperature + 1.0) * 0.5;
        let wind_mod2 = 1.0 - day_perc_time_now * 0.4;
        let noise_wind_base = noise2(world_age_hours / 40.0, self.simplex_offset_a) as f32;
        let mut wind_base = (noise_wind_base + 1.0) * 0.5;
        wind_base *= wind_mod * wind_mod2;
        wind_base *= 0.65;
        self.wind_intensity = wind_base;
        self.wind_angle_intensity = noise2(world_age_hours / 80.0, self.simplex_offset_b) as f32;
        let mut noise_wind_angle_base = noise2(world_age_hours / 40.0, self.simplex_offset_d) as f32;
        noise_wind_angle_base = (noise_wind_angle_base + 1.0) * 0.5;
        self.wind_angle_degrees = 360.0 * noise_wind_angle_base;
        if env.day_night_cycle == 3 {
            self.day_light_strength = 0.0;
            self.night_strength = 1.0;
        } else {
            self.lerp_night = time_lerp_hours(time, self.dusk, self.dawn, true);
            self.lerp_night = clamp(0.0, 1.0, self.lerp_night * 2.0);
            self.night_strength = self.lerp_night;
        }
        if env.day_night_cycle == 2 {
            self.day_light_strength = 1.0 - 0.15 * day_perc_time_now - 0.2 * self.wind_intensity;
            self.night_strength = 0.0;
        } else {
            self.day_light_strength_base = 1.0 - self.night_strength;
            let modd = 1.0 - 0.15 * day_perc_time_now - 0.2 * self.wind_intensity;
            self.day_light_strength_base *= modd;
            self.day_light_strength = self.day_light_strength_base;
        }
        self.ambient = self.day_light_strength;
        let desat_curr = (1.0 - cs.get_cur_day_percent()) * 0.4;
        let desat_next = (1.0 - ns.get_cur_day_percent()) * 0.4;
        self.desaturation = lerp(day_lerp_val, desat_curr, desat_next);
        self.cloudy_t = 1.0 - clamp01((self.air_mass_temperature + 0.8) * 0.625);
        self.cloudy_t *= 0.8;
        self.cloudy_t = clamp01(self.cloudy_t + self.wind_intensity);
        self.cloud_intensity = clamp01(self.wind_intensity * 2.0);
        self.cloud_intensity -= self.cloud_intensity * 0.5 * self.night_strength;
    }

    pub fn get_time(&self) -> f32 {
        self.time
    }
    pub fn get_dawn(&self) -> f32 {
        self.dawn
    }
    pub fn get_dusk(&self) -> f32 {
        self.dusk
    }
    pub fn get_noon(&self) -> f32 {
        self.noon
    }
    pub fn get_air_mass_noise_frequency_mod(&self) -> f64 {
        self.air_mass_noise_frequency_mod
    }
    pub fn get_noise_airmass(&self) -> f32 {
        self.noise_airmass
    }
    pub fn get_air_mass_temperature(&self) -> f32 {
        self.air_mass_temperature
    }
    pub fn get_base_temperature(&self) -> f32 {
        self.base_temperature
    }
    pub fn get_day_light_lagged(&self) -> f32 {
        self.day_light_lagged
    }
    pub fn get_night_lagged(&self) -> f32 {
        self.night_lagged
    }
    pub fn get_temperature(&self) -> f32 {
        self.temperature
    }
    pub fn is_temperature_is_snow(&self) -> bool {
        self.temperature_is_snow
    }
    pub fn get_humidity(&self) -> f32 {
        self.humidity
    }
    pub fn get_wind_intensity(&self) -> f32 {
        self.wind_intensity
    }
    pub fn get_wind_angle_intensity(&self) -> f32 {
        self.wind_angle_intensity
    }
    pub fn get_wind_angle_degrees(&self) -> f32 {
        self.wind_angle_degrees
    }
    pub fn get_night_strength(&self) -> f32 {
        self.night_strength
    }
    pub fn get_day_light_strength(&self) -> f32 {
        self.day_light_strength
    }
    pub fn get_ambient(&self) -> f32 {
        self.ambient
    }
    pub fn get_desaturation(&self) -> f32 {
        self.desaturation
    }
    pub fn get_day_light_strength_base(&self) -> f32 {
        self.day_light_strength_base
    }
    pub fn get_lerp_night(&self) -> f32 {
        self.lerp_night
    }
    pub fn get_cloudy_t(&self) -> f32 {
        self.cloudy_t
    }
    pub fn get_cloud_intensity(&self) -> f32 {
        self.cloud_intensity
    }
    pub fn get_air_front_airmass(&self) -> f32 {
        self.air_front_airmass
    }
    pub fn get_cache_world_age_hours(&self) -> f64 {
        self.cache_world_age_hours
    }
    pub fn get_cache_year(&self) -> i32 {
        self.cache_year
    }
    pub fn get_cache_month(&self) -> i32 {
        self.cache_month
    }
    pub fn get_cache_day(&self) -> i32 {
        self.cache_day
    }
    pub fn get_day_mean_temperature(&self) -> f32 {
        self.day_mean_temperature
    }
    pub fn is_day_do_fog(&self) -> bool {
        self.day_do_fog
    }
    pub fn get_day_fog_strength(&self) -> f32 {
        self.day_fog_strength
    }
    pub fn get_day_fog_duration(&self) -> f32 {
        self.day_fog_duration
    }
}
