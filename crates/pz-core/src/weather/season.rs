//! `zombie.erosion.season.ErosionSeason` (the yearly temperature, season
//! and daylight curves the climate reads) with `zombie.erosion.utils.Noise2D`
//! and the `ErosionConfig.Season` defaults `ErosionMain` initializes it
//! from.

use super::calendar::{JavaCalendar, DAY_OF_MONTH};
use super::simplex::PERM;

/// `erosion.utils.Noise2D`: layered 2D Perlin noise.
#[derive(Clone, Debug, Default)]
pub struct Noise2D {
    layers: Vec<NoiseLayer>,
}

#[derive(Clone, Debug)]
struct NoiseLayer {
    freq: f32,
    amp: f32,
    p: Box<[i32; 512]>,
}

fn floor_f(v: f32) -> f64 {
    (v as f64).floor()
}

impl Noise2D {
    fn fade(t: f32) -> f32 {
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    fn lerp(t: f32, a: f32, b: f32) -> f32 {
        a + t * (b - a)
    }

    fn noise(orig_x: f32, orig_y: f32, p: &[i32; 512]) -> f32 {
        let x = (orig_x as f64 - floor_f(orig_x / 255.0) * 255.0).floor() as i32 as usize;
        let y = (orig_y as f64 - floor_f(orig_y / 255.0) * 255.0).floor() as i32;
        let fx = Self::fade(orig_x - floor_f(orig_x) as f32);
        let fy = Self::fade(orig_y - floor_f(orig_y) as f32);
        let aa = (p[x] + y) as usize;
        let ab = (p[x] + y + 1) as usize;
        let ba = (p[x + 1] + y) as usize;
        let bb = (p[x + 1] + y + 1) as usize;
        let pm = |i: usize| PERM[p[i] as usize] as f32;
        Self::lerp(fy, Self::lerp(fx, pm(aa), pm(ba)), Self::lerp(fx, pm(ab), pm(bb)))
    }

    /// `layeredNoise(x, y)`.
    pub fn layered_noise(&self, x: f32, y: f32) -> f32 {
        let mut fn_ = 0.0f32;
        let mut max_sum = 0.0f32;
        for l in &self.layers {
            max_sum += l.amp;
            fn_ += Self::noise(x * l.freq, y * l.freq, &l.p) * l.amp;
        }
        fn_ / max_sum / 255.0
    }

    /// `addLayer(seed, freq, amp)`.
    pub fn add_layer(&mut self, seed: i32, freq: f32, amp: f32) {
        let wrap = |v: i32| (v as f64 - floor_f(v as f32 / 256.0) * 256.0).floor() as i32;
        let seed = wrap(seed);
        let mut p = Box::new([0i32; 512]);
        for (i, &v) in PERM.iter().enumerate() {
            let ii = wrap(seed + i as i32) as usize;
            p[ii] = v as i32;
            p[256 + ii] = v as i32;
        }
        self.layers.push(NoiseLayer { freq, amp, p });
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.layers.clear();
    }
}

/// `ErosionConfig.Season` (the `erosion.ini` season block), with the
/// defaults of a new world.
#[derive(Clone, Debug, PartialEq)]
pub struct SeasonConfig {
    pub lat: i32,
    pub temp_max: i32,
    pub temp_min: i32,
    pub temp_diff: i32,
    pub season_lag: i32,
    pub noon: f32,
    pub seed_a: i32,
    pub seed_b: i32,
    pub seed_c: i32,
    /// `jan` .. `dec`.
    pub rain: [f32; 12],
}

impl Default for SeasonConfig {
    fn default() -> Self {
        SeasonConfig {
            lat: 38,
            temp_max: 25,
            temp_min: 0,
            temp_diff: 7,
            season_lag: 31,
            noon: 12.5,
            seed_a: 64,
            seed_b: 128,
            seed_c: 255,
            rain: [0.39, 0.35, 0.39, 0.4, 0.35, 0.37, 0.29, 0.26, 0.23, 0.23, 0.3, 0.32],
        }
    }
}

pub const SEASON_DEFAULT: i32 = 0;
pub const SEASON_SPRING: i32 = 1;
pub const SEASON_SUMMER: i32 = 2;
pub const SEASON_SUMMER2: i32 = 3;
pub const SEASON_AUTUMN: i32 = 4;
pub const SEASON_WINTER: i32 = 5;
pub const NUM_SEASONS: i32 = 6;

/// `getSeasonName()` by season id.
pub const SEASON_NAMES: [&str; 6] = ["Default", "Spring", "Early Summer", "Late Summer", "Autumn", "Winter"];
/// The `Translator` keys of `getSeasonNameTranslated()` by season id.
pub const SEASON_NAME_KEYS: [&str; 6] = [
    "IGUI_Season_Default",
    "IGUI_Season_Spring",
    "IGUI_Season_SummerE",
    "IGUI_Season_SummerL",
    "IGUI_Season_Autumn",
    "IGUI_Season_Winter",
];

#[derive(Clone, Debug)]
struct YearData {
    year: i32,
    win_sols: JavaCalendar,
    sum_sols: JavaCalendar,
    win_sols_unx: i64,
    sum_sols_unx: i64,
    hottest_day: JavaCalendar,
    coldest_day: JavaCalendar,
    hottest_day_unx: i64,
    coldest_day_unx: i64,
    winter_s: f32,
    winter_e: f32,
    winter_start_day: JavaCalendar,
    winter_end_day: JavaCalendar,
    winter_end_day_unx: i64,
    winter_start_day_unx: i64,
    summer_s: f32,
    summer_e: f32,
    summer_start_day: JavaCalendar,
    summer_end_day: JavaCalendar,
    summer_start_day_unx: i64,
    summer_end_day_unx: i64,
    last_summer_str: f32,
    last_winter_str: f32,
    summer_str: f32,
    winter_str: f32,
    next_summer_str: f32,
    next_winter_str: f32,
}

impl Default for YearData {
    fn default() -> Self {
        let c = JavaCalendar::new_in(crate::calendar::JavaZone::utc(), 1970, 0, 1, 0, 0);
        YearData {
            year: i32::MIN,
            win_sols: c.clone(),
            sum_sols: c.clone(),
            win_sols_unx: 0,
            sum_sols_unx: 0,
            hottest_day: c.clone(),
            coldest_day: c.clone(),
            hottest_day_unx: 0,
            coldest_day_unx: 0,
            winter_s: 0.0,
            winter_e: 0.0,
            winter_start_day: c.clone(),
            winter_end_day: c.clone(),
            winter_end_day_unx: 0,
            winter_start_day_unx: 0,
            summer_s: 0.0,
            summer_e: 0.0,
            summer_start_day: c.clone(),
            summer_end_day: c,
            summer_start_day_unx: 0,
            summer_end_day_unx: 0,
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
#[derive(Clone, Debug)]
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
    zero_day: JavaCalendar,
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
    /// `SandboxOptions.dayNightCycle` as `isEndlessDay`/`isEndlessNight`
    /// read it (1 normal, 2 endless day, 3 endless night); the owner sets it.
    pub day_night_cycle: i32,
}

impl Default for ErosionSeason {
    /// `new ErosionSeason()` (before `init`).
    fn default() -> Self {
        ErosionSeason {
            lat: 38,
            temp_max: 25,
            temp_min: 0,
            temp_diff: 7,
            high_noon: 12.5,
            high_noon_current: 12.5,
            season_lag: 31,
            rain: [0.0; 12],
            su_sol: 0.0,
            wi_sol: 0.0,
            zero_day: JavaCalendar::new(1970, 0, 1, 0, 0),
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
            per: Noise2D::default(),
            seed_a: 64,
            seed_b: 128,
            seed_c: 255,
            day_night_cycle: 1,
        }
    }
}

fn radian(a: f64) -> f64 {
    a * (std::f64::consts::PI / 180.0)
}

fn degree(a: f64) -> f64 {
    a * 57.29577951308232
}

fn clerp_d(t: f64, a: f64, b: f64) -> f64 {
    let t2 = (1.0 - (t * std::f64::consts::PI).cos()) / 2.0;
    a * (1.0 - t2) + b * t2
}

fn day_diff(a: &JavaCalendar, b: &JavaCalendar) -> f32 {
    let diff = a.time_in_millis() - b.time_in_millis();
    (diff / 86_400_000).abs() as f32
}

impl ErosionSeason {
    /// `ErosionMain`'s set-up: the config with the sandbox `Temperature`
    /// option applied (1 very cold .. 5 very hot), `init`, `setRain`, then
    /// `setDay(day, month, year)` with `GameTime.getDay()` (0-based).
    pub fn from_config(cfg: &SeasonConfig, temperature_modifier: i32, day: i32, month: i32, year: i32) -> Self {
        let (mut temp_min, mut temp_max) = (cfg.temp_min, cfg.temp_max);
        match temperature_modifier {
            1 => {
                temp_min -= 10;
                temp_max -= 10;
            }
            2 => {
                temp_min -= 5;
                temp_max -= 5;
            }
            4 => {
                temp_min = (temp_min as f64 + 7.5) as i32;
                temp_max += 4;
            }
            5 => {
                temp_min += 15;
                temp_max += 8;
            }
            _ => {}
        }
        let mut s = ErosionSeason::default();
        s.init(cfg.lat, temp_max, temp_min, cfg.temp_diff, cfg.season_lag, cfg.noon, cfg.seed_a, cfg.seed_b, cfg.seed_c);
        s.set_rain(cfg.rain);
        s.set_day(day, month, year);
        s
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
        self.winter_mod = if self.temp_min < 0 { 0.05 * -self.temp_min as f32 } else { 0.02 * -self.temp_min as f32 };
        self.summer_mod = if self.temp_max < 0 { 0.05 * self.temp_max as f32 } else { 0.02 * self.temp_max as f32 };
        let tilt = radian(23.44).tan();
        self.su_sol = 2.0 * degree((-radian(self.lat as f64).tan() * tilt).acos()) / 15.0;
        self.wi_sol = 2.0 * degree((radian(self.lat as f64).tan() * tilt).acos()) / 15.0;
        self.per.reset();
        self.per.add_layer(seed_a, 8.0, 2.0);
        self.per.add_layer(seed_b, 6.0, 4.0);
        self.per.add_layer(seed_c, 4.0, 6.0);
        self.year_data = Default::default();
    }

    /// `setRain(jan, .., dec)`.
    pub fn set_rain(&mut self, rain: [f32; 12]) {
        self.rain = rain;
        let mut total = 0.0f32;
        for v in rain {
            total += v;
        }
        self.rain_year_average = ((365.0f32 * (total / 12.0)) as f64).floor() as i32 as f32;
    }

    /// `clone()`: a fresh season with the same configuration (no day set).
    pub fn java_clone(&self) -> ErosionSeason {
        let mut c = ErosionSeason::default();
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
        c.day_night_cycle = self.day_night_cycle;
        c
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
    /// `getSeasonName()` (the index is clamped like an array access would
    /// throw; callers never set an out-of-range season except via
    /// `setCurSeason`).
    pub fn get_season_name(&self) -> Option<&'static str> {
        SEASON_NAMES.get(self.cur_season as usize).copied()
    }
    pub fn get_season_name_key(&self) -> Option<&'static str> {
        SEASON_NAME_KEYS.get(self.cur_season as usize).copied()
    }
    pub fn is_season(&self, season: i32) -> bool {
        season == self.cur_season
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
    pub fn set_cur_season(&mut self, season: i32) {
        self.cur_season = season;
    }
    pub fn is_endless_day(&self) -> bool {
        self.day_night_cycle == 2
    }
    pub fn is_endless_night(&self) -> bool {
        self.day_night_cycle == 3
    }

    /// `getWinterStartDay(day, month, year)`.
    pub fn get_winter_start_day(&self, day: i32, month: i32, year: i32) -> JavaCalendar {
        let v = JavaCalendar::date(year, month, day).time_in_millis();
        if v < self.year_data[0].winter_end_day_unx {
            self.year_data[0].winter_start_day.clone()
        } else {
            self.year_data[1].winter_start_day.clone()
        }
    }

    /// `setDay(day, month, year)`.
    pub fn set_day(&mut self, day: i32, month: i32, year: i32) {
        let date = JavaCalendar::new(year, month, day, 0, 0);
        let value = date.time_in_millis();
        self.set_year_data(year);
        self.set_season_data(value as f32, &date, month);
        self.set_daylight_data(value, &date);
    }

    fn set_year_data(&mut self, year: i32) {
        if self.year_data[1].year == year {
            return;
        }
        for i in 0..3 {
            let cur = year + i as i32 - 1;
            let (wm, sm, lag) = (self.winter_mod, self.summer_mod, self.season_lag);
            let winter_s = self.per.layered_noise((64 + cur) as f32, 64.0);
            let winter_e = self.per.layered_noise(64.0, (64 + cur) as f32);
            let summer_s = self.per.layered_noise((128 + cur) as f32, 128.0);
            let summer_e = self.per.layered_noise(128.0, (128 + cur) as f32);
            let y = &mut self.year_data[i];
            y.year = cur;
            y.win_sols = JavaCalendar::date(cur, 11, 22);
            y.sum_sols = JavaCalendar::date(cur, 5, 22);
            y.win_sols_unx = y.win_sols.time_in_millis();
            y.sum_sols_unx = y.sum_sols.time_in_millis();
            y.hottest_day = JavaCalendar::date(cur, 5, 22);
            y.coldest_day = JavaCalendar::date(cur, 11, 22);
            y.hottest_day.add(DAY_OF_MONTH, lag);
            y.coldest_day.add(DAY_OF_MONTH, lag);
            y.hottest_day_unx = y.hottest_day.time_in_millis();
            y.coldest_day_unx = y.coldest_day.time_in_millis();
            y.winter_s = winter_s;
            y.winter_e = winter_e;
            y.winter_start_day = JavaCalendar::date(cur, 11, 22);
            y.winter_end_day = JavaCalendar::date(cur, 11, 22);
            y.winter_start_day.add(DAY_OF_MONTH, -((40.0f32 + 40.0 * wm + 20.0 * winter_s) as f64).floor() as i32);
            y.winter_end_day.add(DAY_OF_MONTH, ((40.0f32 + 40.0 * wm + 20.0 * winter_e) as f64).floor() as i32);
            y.winter_start_day_unx = y.winter_start_day.time_in_millis();
            y.winter_end_day_unx = y.winter_end_day.time_in_millis();
            y.summer_s = summer_s;
            y.summer_e = summer_e;
            y.summer_start_day = JavaCalendar::date(cur, 5, 22);
            y.summer_end_day = JavaCalendar::date(cur, 5, 22);
            y.summer_start_day.add(DAY_OF_MONTH, -((40.0f32 + 40.0 * sm + 20.0 * summer_s) as f64).floor() as i32);
            y.summer_end_day.add(DAY_OF_MONTH, ((40.0f32 + 40.0 * sm + 20.0 * summer_e) as f64).floor() as i32);
            y.summer_start_day_unx = y.summer_start_day.time_in_millis();
            y.summer_end_day_unx = y.summer_end_day.time_in_millis();
        }
        let (y0s, y0w) = (
            self.year_data[0].summer_s + self.year_data[0].summer_e - 1.0,
            self.year_data[0].winter_s + self.year_data[0].winter_e - 1.0,
        );
        let (y2s, y2w) = (
            self.year_data[2].summer_s + self.year_data[2].summer_e - 1.0,
            self.year_data[2].winter_s + self.year_data[2].winter_e - 1.0,
        );
        let y1 = &mut self.year_data[1];
        y1.last_summer_str = y0s;
        y1.last_winter_str = y0w;
        y1.summer_str = y1.summer_s + y1.summer_e - 1.0;
        y1.winter_str = y1.winter_s + y1.winter_e - 1.0;
        y1.next_summer_str = y2s;
        y1.next_winter_str = y2w;
    }

    fn set_season_data(&mut self, day_value: f32, date: &JavaCalendar, month: i32) {
        let yd = &self.year_data;
        let f = |v: i64| v as f32;
        let (season, sd, ed) = if day_value < f(yd[0].winter_end_day_unx) {
            (5, yd[0].winter_start_day.clone(), yd[0].winter_end_day.clone())
        } else if day_value < f(yd[1].summer_start_day_unx) {
            (1, yd[0].winter_end_day.clone(), yd[1].summer_start_day.clone())
        } else if day_value < f(yd[1].summer_end_day_unx) {
            (2, yd[1].summer_start_day.clone(), yd[1].summer_end_day.clone())
        } else if day_value < f(yd[1].winter_start_day_unx) {
            (4, yd[1].summer_end_day.clone(), yd[1].winter_start_day.clone())
        } else {
            (5, yd[1].winter_start_day.clone(), yd[1].winter_end_day.clone())
        };
        self.cur_season = season;
        self.cur_season_day = day_diff(date, &sd);
        self.cur_season_days = day_diff(&sd, &ed);
        self.cur_season_strength = self.cur_season_days / 90.0 - 1.0;
        self.cur_season_progression = self.cur_season_day / self.cur_season_days;
        let half = (self.temp_diff / 2) as f32;
        let (tmax, tmin) = (self.temp_max as f32, self.temp_min as f32);
        let y1 = &yd[1];
        let (temp_a, temp_b, day_perc) =
            if day_value < f(yd[0].coldest_day_unx) && day_value >= f(yd[0].hottest_day_unx) {
                (
                    tmax + half * y1.last_summer_str,
                    tmin + half * y1.last_winter_str,
                    day_diff(date, &yd[0].hottest_day) / day_diff(&yd[0].hottest_day, &yd[0].coldest_day),
                )
            } else if day_value < f(yd[1].hottest_day_unx) && day_value >= f(yd[0].coldest_day_unx) {
                (
                    tmin + half * y1.last_winter_str,
                    tmax + half * y1.summer_str,
                    day_diff(date, &yd[0].coldest_day) / day_diff(&yd[1].hottest_day, &yd[0].coldest_day),
                )
            } else if day_value < f(yd[1].coldest_day_unx) && day_value >= f(yd[1].hottest_day_unx) {
                (
                    tmax + half * y1.summer_str,
                    tmin + half * y1.winter_str,
                    day_diff(date, &yd[1].hottest_day) / day_diff(&yd[1].hottest_day, &yd[1].coldest_day),
                )
            } else {
                (
                    tmin + half * y1.winter_str,
                    tmax + half * y1.next_summer_str,
                    day_diff(date, &yd[1].coldest_day) / day_diff(&yd[1].coldest_day, &yd[2].hottest_day),
                )
            };
        let day_average_temp = clerp_d(day_perc as f64, temp_a as f64, temp_b as f64) as f32;
        let super_offset = day_diff(&self.zero_day, date) / 20.0;
        self.day_noise_val = self.per.layered_noise(super_offset, 0.0);
        let modifier = self.day_noise_val * 2.0 - 1.0;
        self.day_temperature = day_average_temp + self.temp_diff as f32 * modifier;
        self.day_mean_temperature = day_average_temp;
        self.is_thunder_day = false;
        self.is_rain_day = false;
        self.is_sunny_day = false;
        let r = self.rain[month as usize];
        let rain_mod = if 0.1 + r <= 1.0 { 0.1 + r } else { 1.0 };
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

    fn set_daylight_data(&mut self, day_value: i64, date: &JavaCalendar) {
        let yd = &self.year_data;
        let (h1, start, end) = if day_value < yd[1].win_sols_unx && day_value >= yd[1].sum_sols_unx {
            (false, yd[1].sum_sols.clone(), yd[1].win_sols.clone())
        } else if day_value >= yd[1].win_sols_unx {
            (true, yd[1].win_sols.clone(), yd[2].sum_sols.clone())
        } else {
            (true, yd[0].win_sols.clone(), yd[1].sum_sols.clone())
        };
        let h_cycle_perc = day_diff(date, &start) / day_diff(&start, &end);
        let mut perc2 = h_cycle_perc;
        if h1 {
            self.day_daylight = clerp_d(h_cycle_perc as f64, self.wi_sol, self.su_sol) as f32;
        } else {
            self.day_daylight = clerp_d(h_cycle_perc as f64, self.su_sol, self.wi_sol) as f32;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::calendar::set_default_zone;

    #[test]
    fn seasons_through_a_year() {
        set_default_zone(Some(crate::calendar::JavaZone::utc()));
        let mut s = ErosionSeason::from_config(&SeasonConfig::default(), 3, 9, 6, 1993);
        assert_eq!(s.get_season(), SEASON_SUMMER);
        assert!(s.get_dawn() > 5.0 && s.get_dawn() < 8.0, "{}", s.get_dawn());
        assert!(s.get_dusk() > 19.0 && s.get_dusk() < 23.0, "{}", s.get_dusk());
        let summer = s.get_day_mean_temperature();
        s.set_day(15, 0, 1994);
        assert_eq!(s.get_season(), SEASON_WINTER);
        assert!(s.get_day_mean_temperature() < summer);
        assert!(s.get_daylight() < 12.0);
        let c = s.java_clone();
        assert_eq!(c.get_season(), 0);
        assert_eq!(c.get_rain_year_average(), s.get_rain_year_average());
    }
}
