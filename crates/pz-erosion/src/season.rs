//! `zombie.erosion.season.ErosionSeason`: the season, daylight and daily
//! weather model.
//!
//! The original uses `GregorianCalendar` in the JVM's default time zone;
//! [`CalendarZone`] supplies that zone's UTC offset (UTC by default, which
//! is what a JVM started with `-Duser.timezone=UTC` computes).

use std::sync::Arc;

use crate::noise::Noise2D;

pub const SEASON_DEFAULT: i32 = 0;
pub const SEASON_SPRING: i32 = 1;
pub const SEASON_SUMMER: i32 = 2;
pub const SEASON_SUMMER2: i32 = 3;
pub const SEASON_AUTUMN: i32 = 4;
pub const SEASON_WINTER: i32 = 5;
pub const NUM_SEASONS: i32 = 6;

const DAY_MS: i64 = 86_400_000;

/// UTC offset in milliseconds of the calendar's time zone at a local
/// (wall-clock) time given as milliseconds since the local epoch.
pub type CalendarZone = Arc<dyn Fn(i64) -> i64 + Send + Sync>;

/// Days since 1970-01-01 of a proleptic Gregorian date (`m` 1..=12).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `new GregorianCalendar(year, month, day)` (lenient fields, midnight)
/// followed by `add(DAY_OF_MONTH, add_days)`, as `getTime().getTime()`.
fn calendar_millis(zone: &CalendarZone, year: i32, month: i32, day: i32, add_days: i64) -> i64 {
    let y = year as i64 + (month as i64).div_euclid(12);
    let m = (month as i64).rem_euclid(12) + 1;
    let days = days_from_civil(y, m, 1) + (day as i64 - 1) + add_days;
    let local = days * DAY_MS;
    local - zone(local)
}

/// One calendar date as the original keeps it (`GregorianCalendar`), by
/// its millisecond value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalDate(pub i64);

#[derive(Clone, Debug)]
struct YearData {
    year: i32,
    win_sols: CalDate,
    sum_sols: CalDate,
    hottest_day: CalDate,
    coldest_day: CalDate,
    winter_s: f32,
    winter_e: f32,
    winter_start_day: CalDate,
    winter_end_day: CalDate,
    summer_s: f32,
    summer_e: f32,
    summer_start_day: CalDate,
    summer_end_day: CalDate,
    last_summer_str: f32,
    last_winter_str: f32,
    summer_str: f32,
    winter_str: f32,
    next_summer_str: f32,
    next_winter_str: f32,
}

impl Default for YearData {
    fn default() -> Self {
        YearData {
            year: i32::MIN,
            win_sols: CalDate::default(),
            sum_sols: CalDate::default(),
            hottest_day: CalDate::default(),
            coldest_day: CalDate::default(),
            winter_s: 0.0,
            winter_e: 0.0,
            winter_start_day: CalDate::default(),
            winter_end_day: CalDate::default(),
            summer_s: 0.0,
            summer_e: 0.0,
            summer_start_day: CalDate::default(),
            summer_end_day: CalDate::default(),
            last_summer_str: 0.0,
            last_winter_str: 0.0,
            summer_str: 0.0,
            winter_str: 0.0,
            next_summer_str: 0.0,
            next_winter_str: 0.0,
        }
    }
}

/// `ErosionSeason`.
#[derive(Clone)]
pub struct ErosionSeason {
    lat: i32,
    temp_max: i32,
    temp_min: i32,
    temp_diff: i32,
    high_noon: f32,
    high_noon_current: f32,
    season_lag: i32,
    rain: [f32; 12],
    su_sol: f64,
    wi_sol: f64,
    zero_day: CalDate,
    is_h1: bool,
    year_data: [YearData; 3],
    cur_season: i32,
    cur_season_day: f32,
    cur_season_days: f32,
    cur_season_strength: f32,
    cur_season_progression: f32,
    day_mean_temperature: f32,
    day_temperature: f32,
    day_noise_val: f32,
    is_rain_day: bool,
    rain_year_average: f32,
    rain_day_strength: f32,
    is_thunder_day: bool,
    is_sunny_day: bool,
    day_dusk: f32,
    day_dawn: f32,
    day_daylight: f32,
    winter_mod: f32,
    summer_mod: f32,
    summer_tilt: f32,
    cur_day_percent: f32,
    per: Noise2D,
    seed_a: i32,
    seed_b: i32,
    seed_c: i32,
    /// `names` (read from the game's constructor).
    pub names: Vec<String>,
    /// `namesTranslated` (`Translator.getText("IGUI_Season_...")`).
    pub names_translated: Vec<String>,
    /// `SandboxOptions.dayNightCycle` (2: endless day, 3: endless night).
    pub day_night_cycle: i32,
    zone: CalendarZone,
}

impl std::fmt::Debug for ErosionSeason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErosionSeason").field("season", &self.cur_season).finish_non_exhaustive()
    }
}

fn clerp(t: f64, a: f64, b: f64) -> f64 {
    let t2 = (1.0 - libm::cos(t * std::f64::consts::PI)) / 2.0;
    a * (1.0 - t2) + b * t2
}

fn radian(a: f64) -> f64 {
    a * (std::f64::consts::PI / 180.0)
}

fn degree(a: f64) -> f64 {
    a * 57.295_779_513_082_32
}

/// `dayDiff(date1, date2)`.
fn day_diff(a: CalDate, b: CalDate) -> f32 {
    ((a.0 - b.0) / DAY_MS).abs() as f32
}

impl ErosionSeason {
    /// `new ErosionSeason()` (the field defaults of the original
    /// constructor, which `init` overwrites).
    pub fn new(perm: Arc<[i32]>, names: Vec<String>, zone: Option<CalendarZone>) -> Self {
        let zone: CalendarZone = zone.unwrap_or_else(|| Arc::new(|_| 0));
        let zero_day = CalDate(calendar_millis(&zone, 1970, 0, 1, 0));
        ErosionSeason {
            lat: 0,
            temp_max: 0,
            temp_min: 0,
            temp_diff: 0,
            high_noon: 0.0,
            high_noon_current: 0.0,
            season_lag: 0,
            rain: [0.0; 12],
            su_sol: 0.0,
            wi_sol: 0.0,
            zero_day,
            is_h1: false,
            year_data: Default::default(),
            cur_season: 0,
            cur_season_day: 0.0,
            cur_season_days: 0.0,
            cur_season_strength: 0.0,
            cur_season_progression: 0.0,
            day_mean_temperature: 0.0,
            day_temperature: 0.0,
            day_noise_val: 0.0,
            is_rain_day: false,
            rain_year_average: 0.0,
            rain_day_strength: 0.0,
            is_thunder_day: false,
            is_sunny_day: false,
            day_dusk: 0.0,
            day_dawn: 0.0,
            day_daylight: 0.0,
            winter_mod: 0.0,
            summer_mod: 0.0,
            summer_tilt: 0.0,
            cur_day_percent: 0.0,
            per: Noise2D::new(perm),
            seed_a: 0,
            seed_b: 0,
            seed_c: 0,
            names_translated: names.clone(),
            names,
            day_night_cycle: 1,
            zone,
        }
    }

    /// The calendar time zone.
    pub fn zone(&self) -> CalendarZone {
        self.zone.clone()
    }

    fn cal(&self, year: i32, month: i32, day: i32, add: i64) -> CalDate {
        CalDate(calendar_millis(&self.zone, year, month, day, add))
    }

    /// `init(lat, tempMax, tempMin, tempDiff, seasonLag, noon, seedA, seedB, seedC)`.
    #[allow(clippy::too_many_arguments)]
    pub fn init(
        &mut self,
        lat: i32,
        temp_max: i32,
        temp_min: i32,
        temp_diff: i32,
        season_lag: i32,
        noon: f32,
        seed_a: i32,
        seed_b: i32,
        seed_c: i32,
    ) {
        self.lat = lat;
        self.temp_max = temp_max;
        self.temp_min = temp_min;
        self.temp_diff = temp_diff;
        self.season_lag = season_lag;
        self.high_noon = noon;
        self.high_noon_current = noon;
        self.seed_a = seed_a;
        self.seed_b = seed_b;
        self.seed_c = seed_c;
        self.summer_tilt = 2.0;
        self.winter_mod =
            if self.temp_min < 0 { 0.05 * -(self.temp_min as f32) } else { 0.02 * -(self.temp_min as f32) };
        self.summer_mod = if self.temp_max < 0 { 0.05 * self.temp_max as f32 } else { 0.02 * self.temp_max as f32 };
        // `Math.tan` / `Math.acos` are fdlibm (`StrictMath`) in the original.
        let lat = libm::tan(radian(self.lat as f64));
        let tilt = libm::tan(radian(23.44));
        self.su_sol = 2.0 * degree(libm::acos(-lat * tilt)) / 15.0;
        self.wi_sol = 2.0 * degree(libm::acos(lat * tilt)) / 15.0;
        self.per.reset();
        self.per.add_layer(seed_a, 8.0, 2.0);
        self.per.add_layer(seed_b, 6.0, 4.0);
        self.per.add_layer(seed_c, 4.0, 6.0);
        self.year_data = Default::default();
    }

    pub fn get_lat(&self) -> i32 {
        self.lat
    }
    pub fn get_temp_max(&self) -> i32 {
        self.temp_max
    }
    pub fn get_temp_min(&self) -> i32 {
        self.temp_min
    }
    pub fn get_temp_diff(&self) -> i32 {
        self.temp_diff
    }
    pub fn get_season_lag(&self) -> i32 {
        self.season_lag
    }
    pub fn get_high_noon(&self) -> f32 {
        self.high_noon
    }
    pub fn get_seed_a(&self) -> i32 {
        self.seed_a
    }
    pub fn get_seed_b(&self) -> i32 {
        self.seed_b
    }
    pub fn get_seed_c(&self) -> i32 {
        self.seed_c
    }

    /// `setRain(jan, ..., dec)`.
    pub fn set_rain(&mut self, rain: [f32; 12]) {
        self.rain = rain;
        let mut total = 0.0f32;
        for v in self.rain {
            total += v;
        }
        self.rain_year_average = (365.0f32 * (total / 12.0)).floor() as i32 as f32;
    }

    /// `clone()`: same climate parameters, no day set.
    pub fn clone_season(&self) -> Self {
        let mut c = ErosionSeason::new(self.per_perm(), self.names.clone(), Some(self.zone.clone()));
        c.names_translated = self.names_translated.clone();
        c.day_night_cycle = self.day_night_cycle;
        c.init(
            self.lat,
            self.temp_max,
            self.temp_min,
            self.temp_diff,
            self.season_lag,
            self.high_noon,
            self.seed_a,
            self.seed_b,
            self.seed_c,
        );
        c.set_rain(self.rain);
        c
    }

    fn per_perm(&self) -> Arc<[i32]> {
        self.per.perm()
    }

    pub fn get_cur_day_percent(&self) -> f32 {
        self.cur_day_percent
    }
    pub fn get_max_daylight_winter(&self) -> f64 {
        self.wi_sol
    }
    pub fn get_max_daylight_summer(&self) -> f64 {
        self.su_sol
    }
    pub fn get_dusk(&self) -> f32 {
        self.day_dusk
    }
    pub fn get_dawn(&self) -> f32 {
        self.day_dawn
    }
    pub fn get_daylight(&self) -> f32 {
        self.day_daylight
    }
    pub fn get_day_temperature(&self) -> f32 {
        self.day_temperature
    }
    pub fn get_day_mean_temperature(&self) -> f32 {
        self.day_mean_temperature
    }
    pub fn get_season(&self) -> i32 {
        self.cur_season
    }
    pub fn get_day_high_noon(&self) -> f32 {
        self.high_noon_current
    }
    pub fn get_season_name(&self) -> &str {
        &self.names[self.cur_season as usize]
    }
    pub fn get_season_name_translated(&self) -> &str {
        &self.names_translated[self.cur_season as usize]
    }
    pub fn is_season(&self, season: i32) -> bool {
        season == self.cur_season
    }

    /// `getWinterStartDay(day, month, year)` as milliseconds.
    pub fn get_winter_start_day(&self, day: i32, month: i32, year: i32) -> CalDate {
        let v = self.cal(year, month, day, 0);
        if v.0 < self.year_data[0].winter_end_day.0 {
            self.year_data[0].winter_start_day
        } else {
            self.year_data[1].winter_start_day
        }
    }

    pub fn get_season_day(&self) -> f32 {
        self.cur_season_day
    }
    pub fn get_season_days(&self) -> f32 {
        self.cur_season_days
    }
    pub fn get_season_strength(&self) -> f32 {
        self.cur_season_strength
    }
    pub fn get_season_progression(&self) -> f32 {
        self.cur_season_progression
    }
    pub fn get_day_noise_val(&self) -> f32 {
        self.day_noise_val
    }
    pub fn is_rain_day(&self) -> bool {
        self.is_rain_day
    }
    pub fn get_rain_day_strength(&self) -> f32 {
        self.rain_day_strength
    }
    pub fn get_rain_year_average(&self) -> f32 {
        self.rain_year_average
    }
    pub fn is_thunder_day(&self) -> bool {
        self.is_thunder_day
    }
    pub fn is_sunny_day(&self) -> bool {
        self.is_sunny_day
    }

    /// `setDay(day, month, year)` (`month` 0-based, fields lenient).
    pub fn set_day(&mut self, day: i32, month: i32, year: i32) {
        if year == 0 {
            eprintln!("NOTICE: year value is 0?");
        }
        let day_date = self.cal(year, month, day, 0);
        self.set_year_data(year);
        self.set_season_data(day_date.0 as f32, day_date, month);
        self.set_daylight_data(day_date.0, day_date);
    }

    fn set_year_data(&mut self, year: i32) {
        if self.year_data[1].year == year {
            return;
        }
        for i in 0..3 {
            let cur_year = year.wrapping_add(i as i32 - 1);
            let lag = self.season_lag as i64;
            let winter_s = self.per.layered_noise(64i32.wrapping_add(cur_year) as f32, 64.0);
            let winter_e = self.per.layered_noise(64.0, 64i32.wrapping_add(cur_year) as f32);
            let summer_s = self.per.layered_noise(128i32.wrapping_add(cur_year) as f32, 128.0);
            let summer_e = self.per.layered_noise(128.0, 128i32.wrapping_add(cur_year) as f32);
            let ws = (-((40.0f32 + 40.0 * self.winter_mod + 20.0 * winter_s) as f64).floor()) as i32;
            let we = ((40.0f32 + 40.0 * self.winter_mod + 20.0 * winter_e) as f64).floor() as i32;
            let ss = (-((40.0f32 + 40.0 * self.summer_mod + 20.0 * summer_s) as f64).floor()) as i32;
            let se = ((40.0f32 + 40.0 * self.summer_mod + 20.0 * summer_e) as f64).floor() as i32;
            let d = YearData {
                year: cur_year,
                win_sols: self.cal(cur_year, 11, 22, 0),
                sum_sols: self.cal(cur_year, 5, 22, 0),
                hottest_day: self.cal(cur_year, 5, 22, lag),
                coldest_day: self.cal(cur_year, 11, 22, lag),
                winter_s,
                winter_e,
                winter_start_day: self.cal(cur_year, 11, 22, ws as i64),
                winter_end_day: self.cal(cur_year, 11, 22, we as i64),
                summer_s,
                summer_e,
                summer_start_day: self.cal(cur_year, 5, 22, ss as i64),
                summer_end_day: self.cal(cur_year, 5, 22, se as i64),
                ..self.year_data[i].clone()
            };
            self.year_data[i] = d;
        }
        let (y0, y2) = (self.year_data[0].clone(), self.year_data[2].clone());
        let y1 = &mut self.year_data[1];
        y1.last_summer_str = y0.summer_s + y0.summer_e - 1.0;
        y1.last_winter_str = y0.winter_s + y0.winter_e - 1.0;
        y1.summer_str = y1.summer_s + y1.summer_e - 1.0;
        y1.winter_str = y1.winter_s + y1.winter_e - 1.0;
        y1.next_summer_str = y2.summer_s + y2.summer_e - 1.0;
        y1.next_winter_str = y2.winter_s + y2.winter_e - 1.0;
    }

    fn set_season_data(&mut self, day_value: f32, day_date: CalDate, month: i32) {
        let yd = &self.year_data;
        let lt = |d: CalDate| day_value < d.0 as f32;
        let (season, sd, ed) = if lt(yd[0].winter_end_day) {
            (5, yd[0].winter_start_day, yd[0].winter_end_day)
        } else if lt(yd[1].summer_start_day) {
            (1, yd[0].winter_end_day, yd[1].summer_start_day)
        } else if lt(yd[1].summer_end_day) {
            (2, yd[1].summer_start_day, yd[1].summer_end_day)
        } else if lt(yd[1].winter_start_day) {
            (4, yd[1].summer_end_day, yd[1].winter_start_day)
        } else {
            (5, yd[1].winter_start_day, yd[1].winter_end_day)
        };
        self.cur_season = season;
        self.cur_season_day = day_diff(day_date, sd);
        self.cur_season_days = day_diff(sd, ed);
        self.cur_season_strength = self.cur_season_days / 90.0 - 1.0;
        self.cur_season_progression = self.cur_season_day / self.cur_season_days;
        let half = (self.temp_diff / 2) as f32;
        let (tmax, tmin) = (self.temp_max as f32, self.temp_min as f32);
        let y1 = &yd[1];
        let ge = |d: CalDate| day_value >= d.0 as f32;
        let (temp_a, temp_b, day_perc) = if lt(yd[0].coldest_day) && ge(yd[0].hottest_day) {
            (
                tmax + half * y1.last_summer_str,
                tmin + half * y1.last_winter_str,
                day_diff(day_date, yd[0].hottest_day) / day_diff(yd[0].hottest_day, yd[0].coldest_day),
            )
        } else if lt(yd[1].hottest_day) && ge(yd[0].coldest_day) {
            (
                tmin + half * y1.last_winter_str,
                tmax + half * y1.summer_str,
                day_diff(day_date, yd[0].coldest_day) / day_diff(yd[1].hottest_day, yd[0].coldest_day),
            )
        } else if lt(yd[1].coldest_day) && ge(yd[1].hottest_day) {
            (
                tmax + half * y1.summer_str,
                tmin + half * y1.winter_str,
                day_diff(day_date, yd[1].hottest_day) / day_diff(yd[1].hottest_day, yd[1].coldest_day),
            )
        } else {
            (
                tmin + half * y1.winter_str,
                tmax + half * y1.next_summer_str,
                day_diff(day_date, yd[1].coldest_day) / day_diff(yd[1].coldest_day, yd[2].hottest_day),
            )
        };
        let day_average_temp = clerp(day_perc as f64, temp_a as f64, temp_b as f64) as f32;
        let super_offset = day_diff(self.zero_day, day_date) / 20.0;
        self.day_noise_val = self.per.layered_noise(super_offset, 0.0);
        let modifier = self.day_noise_val * 2.0 - 1.0;
        self.day_temperature = day_average_temp + self.temp_diff as f32 * modifier;
        self.day_mean_temperature = day_average_temp;
        self.is_thunder_day = false;
        self.is_rain_day = false;
        self.is_sunny_day = false;
        // `rain[month]` throws upstream for a month outside 0..12.
        let rain = self.rain[month as usize];
        let rain_mod = if 0.1f32 + rain <= 1.0 { 0.1f32 + rain } else { 1.0 };
        if rain_mod > 0.0 && self.day_noise_val < rain_mod {
            self.is_rain_day = true;
            self.rain_day_strength = 1.0 - self.day_noise_val / rain_mod;
            let tmp = self.per.layered_noise(0.0, super_offset);
            if tmp as f64 > 0.6 {
                self.is_thunder_day = true;
            }
        }
        if self.day_noise_val as f64 > 0.6 {
            self.is_sunny_day = true;
        }
    }

    fn set_daylight_data(&mut self, day_value: i64, day_date: CalDate) {
        let y = &self.year_data;
        let (cycle_start, cycle_end) = if day_value < y[1].win_sols.0 && day_value >= y[1].sum_sols.0 {
            self.is_h1 = false;
            (y[1].sum_sols, y[1].win_sols)
        } else {
            self.is_h1 = true;
            if day_value >= y[1].win_sols.0 {
                (y[1].win_sols, y[2].sum_sols)
            } else {
                (y[0].win_sols, y[1].sum_sols)
            }
        };
        let h_cycle_perc = day_diff(day_date, cycle_start) / day_diff(cycle_start, cycle_end);
        let mut perc2 = h_cycle_perc;
        if self.is_h1 {
            self.day_daylight = clerp(h_cycle_perc as f64, self.wi_sol, self.su_sol) as f32;
        } else {
            self.day_daylight = clerp(h_cycle_perc as f64, self.su_sol, self.wi_sol) as f32;
            perc2 = 1.0 - perc2;
        }
        self.cur_day_percent = perc2;
        if self.is_endless_night() {
            self.high_noon_current = 0.0;
            self.day_dawn = 0.0;
            self.day_dusk = 0.0;
            self.day_daylight = 0.0;
            return;
        }
        if self.is_endless_day() {
            self.day_dawn = 0.0;
            self.day_dusk = 0.0;
            self.day_daylight = 24.0;
            return;
        }
        self.high_noon_current = self.high_noon + self.summer_tilt * perc2;
        self.day_dawn = self.high_noon_current - self.day_daylight / 2.0;
        self.day_dusk = self.high_noon_current + self.day_daylight / 2.0;
    }

    /// `setCurSeason(season)`.
    pub fn set_cur_season(&mut self, season: i32) {
        self.cur_season = season;
    }

    pub fn is_endless_day(&self) -> bool {
        self.day_night_cycle == 2
    }

    pub fn is_endless_night(&self) -> bool {
        self.day_night_cycle == 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_perm() -> Arc<[i32]> {
        // Any permutation of 0..256 exercises the code paths.
        (0..256).map(|i| (i * 7 + 3) % 256).collect::<Vec<i32>>().into()
    }

    #[test]
    fn calendar() {
        let utc: CalendarZone = Arc::new(|_| 0);
        assert_eq!(calendar_millis(&utc, 1970, 0, 1, 0), 0);
        assert_eq!(calendar_millis(&utc, 1970, 0, 2, 0), DAY_MS);
        // Lenient fields: day 0 is the last day of the previous month.
        assert_eq!(calendar_millis(&utc, 2000, 2, 0, 0), calendar_millis(&utc, 2000, 1, 29, 0));
        assert_eq!(calendar_millis(&utc, 1999, 12, 1, 0), calendar_millis(&utc, 2000, 0, 1, 0));
        assert_eq!(calendar_millis(&utc, 1993, 6, 9, 0) / DAY_MS, 8590);
        let est: CalendarZone = Arc::new(|_| -5 * 3_600_000);
        assert_eq!(calendar_millis(&est, 1970, 0, 1, 0), 5 * 3_600_000);
    }

    #[test]
    fn season_over_a_year() {
        let names = ["a", "b", "c", "d", "e", "f"].map(String::from).to_vec();
        let mut s = ErosionSeason::new(synthetic_perm(), names, None);
        s.init(38, 25, 0, 7, 31, 12.5, 64, 128, 255);
        s.set_rain([0.3; 12]);
        assert_eq!(s.get_rain_year_average(), 109.0);
        let mut seen = std::collections::HashSet::new();
        for d in 0..365 {
            s.set_day(d, 0, 1993);
            seen.insert(s.get_season());
            assert!(s.get_daylight() > 0.0 && s.get_daylight() < 24.0);
            assert!(s.get_season_day() <= s.get_season_days());
        }
        assert!(seen.contains(&SEASON_WINTER) && seen.contains(&SEASON_SUMMER));
        s.day_night_cycle = 2;
        s.set_day(10, 6, 1993);
        assert_eq!(s.get_daylight(), 24.0);
        let c = s.clone_season();
        assert_eq!(c.get_lat(), 38);
    }
}
