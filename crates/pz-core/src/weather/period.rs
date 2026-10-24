//! `zombie.iso.weather.WeatherPeriod`: a weather front's sequence of
//! stages (showers, storms, blizzards, ...) and the overrides it drives into
//! the climate while it runs.

use super::calendar::JavaCalendar;
use super::color::{ClimateColorInfo, ColorRef};
use super::manager::{
    AirFront, ClimateVars, BOOL_IS_SNOW, COLOR_GLOBAL_LIGHT, COLOR_NEW_FOG, FLOAT_AMBIENT, FLOAT_CLOUD_INTENSITY,
    FLOAT_DAYLIGHT_STRENGTH, FLOAT_DESATURATION, FLOAT_FOG_INTENSITY, FLOAT_GLOBAL_LIGHT_INTENSITY,
    FLOAT_NIGHT_STRENGTH, FLOAT_PRECIPITATION_INTENSITY, FLOAT_TEMPERATURE, FLOAT_WIND_ANGLE_INTENSITY,
    FLOAT_WIND_INTENSITY,
};
use super::season::ErosionSeason;
use super::simplex::noise2;
use super::thunder::ThunderStorm;
use super::values::{ClimateValues, ValuesEnv};
use super::{clamp, clamp01, clerp, jmax, jmin, rain_time_multiplier_mod, ClimateClock, ClimateContext, ClimateEvent, PeriodId};
use crate::random::JavaRandom;
use std::rc::Rc;

pub const STAGE_START: i32 = 0;
pub const STAGE_SHOWERS: i32 = 1;
pub const STAGE_HEAVY_PRECIP: i32 = 2;
pub const STAGE_STORM: i32 = 3;
pub const STAGE_CLEARING: i32 = 4;
pub const STAGE_MODERATE: i32 = 5;
pub const STAGE_DRIZZLE: i32 = 6;
pub const STAGE_BLIZZARD: i32 = 7;
pub const STAGE_TROPICAL_STORM: i32 = 8;
pub const STAGE_INTERMEZZO: i32 = 9;
pub const STAGE_MODDED: i32 = 10;
pub const STAGE_KATEBOB_STORM: i32 = 11;
pub const STAGE_MAX: i32 = 12;
pub const FRONT_STRENGTH_THRESHOLD: f32 = 0.1;
pub const MAX_TEMPERATURE_INFLUENCE: f32 = 7.0;

/// `WeatherPeriod.StrLerpVal`: only these ids exist; others load as `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StrLerpVal {
    Entry,
    Target,
    NextTarget,
    #[default]
    None,
}

impl StrLerpVal {
    pub fn value(self) -> i32 {
        match self {
            StrLerpVal::Entry => 1,
            StrLerpVal::Target => 2,
            StrLerpVal::NextTarget => 3,
            StrLerpVal::None => 0,
        }
    }
    /// `fromValue(id)`.
    pub fn from_value(id: i32) -> Self {
        match id {
            1 => StrLerpVal::Entry,
            2 => StrLerpVal::Target,
            3 => StrLerpVal::NextTarget,
            _ => StrLerpVal::None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            StrLerpVal::Entry => "Entry",
            StrLerpVal::Target => "Target",
            StrLerpVal::NextTarget => "NextTarget",
            StrLerpVal::None => "None",
        }
    }
    pub fn ordinal(self) -> i32 {
        match self {
            StrLerpVal::Entry => 0,
            StrLerpVal::Target => 1,
            StrLerpVal::NextTarget => 2,
            StrLerpVal::None => 3,
        }
    }
}

/// `WeatherPeriod.WeatherStage`. The previous/next links are indices into
/// the period's stage list.
#[derive(Clone, Debug, PartialEq)]
pub struct WeatherStage {
    pub stage_id: i32,
    pub stage_duration: f64,
    pub stage_start: f64,
    pub stage_end: f64,
    pub entry_strength: f32,
    pub exit_strength: f32,
    pub target_strength: f32,
    pub lerp_mid_val: StrLerpVal,
    pub lerp_end_val: StrLerpVal,
    pub has_started_cloud: bool,
    pub mod_id: Option<String>,
    pub fog_strength: f32,
    pub linear_t: f32,
    pub parabolic_t: f32,
    pub is_cycle_first_half: bool,
    pub creation_finished: bool,
    pub previous_stage: Option<usize>,
    pub next_stage: Option<usize>,
}

impl Default for WeatherStage {
    fn default() -> Self {
        WeatherStage {
            stage_id: 0,
            stage_duration: 0.0,
            stage_start: 0.0,
            stage_end: 0.0,
            entry_strength: 0.0,
            exit_strength: 0.0,
            target_strength: 0.0,
            lerp_mid_val: StrLerpVal::None,
            lerp_end_val: StrLerpVal::None,
            has_started_cloud: false,
            mod_id: None,
            fog_strength: 0.0,
            linear_t: 0.0,
            parabolic_t: 0.0,
            is_cycle_first_half: true,
            creation_finished: false,
            previous_stage: None,
            next_stage: None,
        }
    }
}

impl WeatherStage {
    /// `new WeatherStage(id)`.
    pub fn new(id: i32) -> Self {
        WeatherStage { stage_id: id, ..Default::default() }
    }

    /// `setStageDuration(time)`: at least one hour.
    pub fn set_stage_duration(&mut self, time: f64) {
        self.stage_duration = if time < 1.0 { 1.0 } else { time };
    }

    /// `overrideStageDuration(time)`.
    pub fn override_stage_duration(&mut self, time: f64) {
        self.stage_duration = time;
    }

    /// `startStage(exitTime)`.
    pub fn start_stage(&mut self, exit_time: f64) {
        self.stage_start = exit_time;
        self.stage_end = exit_time + self.stage_duration;
        self.has_started_cloud = false;
    }

    /// `setStageStart(worldAgeHours)`: returns the stage end.
    pub fn set_stage_start(&mut self, t: f64) -> f64 {
        self.stage_start = t;
        self.stage_end = t + self.stage_duration;
        self.stage_end
    }

    /// `lerpEntryTo(mid, end)` (ignored once the period is linked).
    pub fn lerp_entry_to(&mut self, mid: StrLerpVal, end: StrLerpVal) {
        if !self.creation_finished {
            self.lerp_mid_val = mid;
            self.lerp_end_val = end;
        }
    }

    /// `setHasStartedCloud(b)`: the original always sets true.
    pub fn set_has_started_cloud(&mut self, _b: bool) {
        self.has_started_cloud = true;
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.previous_stage = None;
        self.next_stage = None;
        self.is_cycle_first_half = true;
        self.has_started_cloud = false;
        self.lerp_mid_val = StrLerpVal::None;
        self.lerp_end_val = StrLerpVal::None;
        self.entry_strength = 0.0;
        self.exit_strength = 0.0;
        self.mod_id = None;
        self.creation_finished = false;
        self.fog_strength = 0.0;
    }

    fn lerp_value(&self, v: StrLerpVal, next_target: Option<f32>) -> f32 {
        match v {
            StrLerpVal::Entry => self.entry_strength,
            StrLerpVal::Target => self.target_strength,
            StrLerpVal::NextTarget => next_target.unwrap_or(0.0),
            StrLerpVal::None => 0.0,
        }
    }

    /// `getStageCurrentStrength()`; `next_target` is the linked next stage's
    /// `targetStrength`.
    pub fn stage_current_strength(&self, next_target: Option<f32>) -> f32 {
        let m = self.lerp_value(self.lerp_mid_val, next_target);
        let e = self.lerp_value(self.lerp_end_val, next_target);
        if self.lerp_mid_val == StrLerpVal::None {
            return clerp(self.linear_t, self.entry_strength, e);
        }
        if self.is_cycle_first_half {
            clerp(self.parabolic_t, self.entry_strength, m)
        } else {
            clerp(self.parabolic_t, e, m)
        }
    }

    fn update_t(&mut self, hour: f64, kate_bob_progress: f32) {
        self.linear_t = if hour < self.stage_start {
            0.0
        } else if hour > self.stage_end {
            1.0
        } else {
            ((hour - self.stage_start) / self.stage_duration) as f32
        };
        if self.stage_id == STAGE_KATEBOB_STORM {
            self.linear_t = kate_bob_progress + (1.0 - kate_bob_progress) * self.linear_t;
        }
        if self.linear_t < 0.5 {
            self.parabolic_t = self.linear_t * 2.0;
            self.is_cycle_first_half = true;
        } else {
            self.parabolic_t = 2.0 - self.linear_t * 2.0;
            self.is_cycle_first_half = false;
        }
    }
}

/// What `WeatherPeriod.init` reads from the climate and `GameTime`.
pub struct PeriodInitEnv<'a> {
    pub clock: &'a ClimateClock,
    /// `ClimateManager.season` (what `setDayInfo` clones).
    pub base_season: &'a ErosionSeason,
    /// `ClimateManager.getSeason()`.
    pub current_season: &'a ErosionSeason,
    pub simplex_offset_a: f64,
    pub simplex_offset_b: f64,
    pub values_env: ValuesEnv,
    /// `IsoWorld.instance.getGameMode()`.
    pub game_mode: &'a str,
}

/// What `WeatherPeriod.update` drives and reads in the climate.
pub struct PeriodUpdateEnv<'a> {
    pub vars: &'a mut ClimateVars,
    pub thunder: &'a mut ThunderStorm,
    pub fog_tint_storm: &'a ColorRef,
    pub fog_tint_tropical: &'a ColorRef,
    /// `getSeason().isSeason(WINTER)`.
    pub season_is_winter: bool,
    pub winter_is_coming: bool,
    pub fog_quality: i32,
    pub world_age_hours: f64,
}

/// `WeatherPeriod`.
#[derive(Clone, Debug)]
pub struct WeatherPeriod {
    pub id: PeriodId,
    pub is_running: bool,
    pub weather_stage_index: i32,
    pub current_strength: f32,
    pub rain_threshold: f32,
    pub is_thunder_storm: bool,
    pub is_tropical_storm: bool,
    pub is_blizzard: bool,
    pub front_cache: AirFront,
    pub weather_stages: Vec<WeatherStage>,
    /// `cloudColor` (may alias one of the preset colours).
    pub cloud_color: ColorRef,
    pub cloud_color_reddish: ColorRef,
    pub cloud_color_greenish: ColorRef,
    pub cloud_color_blueish: ColorRef,
    pub cloud_color_purplish: ColorRef,
    pub cloud_color_tropical: ColorRef,
    pub cloud_color_blizzard: ColorRef,
    pub start_time: f64,
    pub duration: f64,
    pub current_time: f64,
    /// `currentStage` as an index into `weather_stages`.
    pub current_stage: Option<usize>,
    pub total_progress: f32,
    pub stage_progress: f32,
    pub weather_noise: f32,
    pub temperature_influence: f32,
    pub wind_angle_dir_mod: f32,
    pub precipitation_final: f32,
    /// `kateBobStormProgress` (a static in the original).
    pub kate_bob_storm_progress: f32,
    pub kate_bob_storm_x: i32,
    pub kate_bob_storm_y: i32,
    pub print_stuff: bool,
    pub is_dummy: bool,
    has_started_init: bool,
    seeded_random: JavaRandom,
    pub climate_values: ClimateValues,
}

impl Default for WeatherPeriod {
    fn default() -> Self {
        WeatherPeriod::new(PeriodId::Main, ClimateValues::new([0.0; 4]))
    }
}

impl WeatherPeriod {
    /// `new WeatherPeriod(climmgr, ts)`; `values` is
    /// `climmgr.getClimateValuesCopy()`.
    pub fn new(id: PeriodId, values: ClimateValues) -> Self {
        WeatherPeriod {
            id,
            is_running: false,
            weather_stage_index: 0,
            current_strength: 0.0,
            rain_threshold: 0.0,
            is_thunder_storm: false,
            is_tropical_storm: false,
            is_blizzard: false,
            front_cache: AirFront::default(),
            weather_stages: Vec::new(),
            cloud_color: ClimateColorInfo::new(0.4, 0.2, 0.2, 0.4).shared(),
            cloud_color_reddish: ClimateColorInfo::new(0.66, 0.12, 0.12, 0.4).shared(),
            cloud_color_greenish: ClimateColorInfo::new(0.32, 0.48, 0.12, 0.4).shared(),
            cloud_color_blueish: ClimateColorInfo::new(0.16, 0.48, 0.48, 0.4).shared(),
            cloud_color_purplish: ClimateColorInfo::new(0.66, 0.12, 0.66, 0.4).shared(),
            cloud_color_tropical: ClimateColorInfo::new(0.4, 0.2, 0.2, 0.4).shared(),
            cloud_color_blizzard: ClimateColorInfo::new8(0.12, 0.13, 0.21, 0.5, 0.38, 0.4, 0.5, 0.8).shared(),
            start_time: 0.0,
            duration: 0.0,
            current_time: 0.0,
            current_stage: None,
            total_progress: 0.0,
            stage_progress: 0.0,
            weather_noise: 0.0,
            temperature_influence: 0.0,
            wind_angle_dir_mod: 1.0,
            precipitation_final: 0.0,
            kate_bob_storm_progress: 0.45,
            kate_bob_storm_x: 2000,
            kate_bob_storm_y: 2000,
            print_stuff: true,
            is_dummy: false,
            has_started_init: false,
            seeded_random: JavaRandom::new(1984),
            climate_values: values,
        }
    }

    /// `getCurrentStage()`.
    pub fn current_stage(&self) -> Option<&WeatherStage> {
        self.current_stage.and_then(|i| self.weather_stages.get(i))
    }

    /// `getCurrentStageID()`.
    pub fn current_stage_id(&self) -> i32 {
        self.current_stage().map_or(-1, |s| s.stage_id)
    }

    /// `getTotalStrength()`.
    pub fn total_strength(&self) -> f32 {
        self.front_cache.strength
    }

    /// `getWindAngleDegrees()`.
    pub fn wind_angle_degrees(&self) -> f32 {
        self.front_cache.front_wind_angle_degrees
    }

    /// `getFrontType()`.
    pub fn front_type(&self) -> i32 {
        self.front_cache.front_type
    }

    fn has_stage(&self, id: i32) -> bool {
        self.weather_stages.iter().any(|s| s.stage_id == id)
    }
    pub fn has_tropical(&self) -> bool {
        self.has_stage(STAGE_TROPICAL_STORM)
    }
    pub fn has_storm(&self) -> bool {
        self.has_stage(STAGE_STORM)
    }
    pub fn has_blizzard(&self) -> bool {
        self.has_stage(STAGE_BLIZZARD)
    }
    pub fn has_heavy_rain(&self) -> bool {
        self.has_stage(STAGE_HEAVY_PRECIP)
    }

    /// `getStageForWorldAge(worldAgeHours)`.
    pub fn stage_for_world_age(&self, h: f64) -> Option<&WeatherStage> {
        self.weather_stages.iter().find(|s| h >= s.stage_start && h < s.stage_end)
    }

    /// `setKateBobStormProgress(progress)`.
    pub fn set_kate_bob_storm_progress(&mut self, p: f32) {
        self.kate_bob_storm_progress = clamp01(p);
    }

    /// `setKateBobStormCoords(x, y)`.
    pub fn set_kate_bob_storm_coords(&mut self, x: i32, y: i32) {
        self.kate_bob_storm_x = x;
        self.kate_bob_storm_y = y;
    }

    fn next_target(&self, idx: usize) -> Option<f32> {
        self.weather_stages[idx].next_stage.and_then(|n| self.weather_stages.get(n)).map(|s| s.target_strength)
    }

    /// `getStageCurrentStrength()` of stage `idx`.
    pub fn stage_strength(&self, idx: usize) -> f32 {
        self.weather_stages[idx].stage_current_strength(self.next_target(idx))
    }

    fn rand_range_f(&self, min: f32, max: f32) -> f32 {
        if min == max {
            return min;
        }
        let (min, max) = if min > max { (max, max) } else { (min, max) };
        min + self.seeded_random.next_float() * (max - min)
    }

    fn rand_range_i(&self, min: i32, max: i32) -> i32 {
        if min == max {
            return min;
        }
        let (min, max) = if min > max { (max, max) } else { (min, max) };
        min + self.seeded_random.next_int(max - min)
    }

    /// `reseed(year, month, day)`.
    pub fn reseed(&mut self, year: i32, month: i32, day: i32, simplex_a: f64, simplex_b: f64) {
        let base_seed_a = simplex_a as i32;
        let base_seed_b = simplex_b as i32;
        let mut seed = ((year - 1990).wrapping_mul(100_000)) as i64;
        seed += month.wrapping_mul(day).wrapping_mul(1234) as i64;
        seed += (year - 1990).wrapping_mul(month).wrapping_mul(10_000) as i64;
        seed += base_seed_b.wrapping_sub(base_seed_a).wrapping_mul(day) as i64;
        self.seeded_random.set_seed(seed);
    }

    /// `init(front, hoursSinceStart, year, month, day, doThisStageOnly,
    /// singleStageDuration)` (`-1, -1` for a generated pattern).
    #[allow(clippy::too_many_arguments)]
    pub fn init(
        &mut self,
        front: &AirFront,
        hours_since_start: f64,
        year: i32,
        month: i32,
        day: i32,
        stage_only: i32,
        single_stage_duration: f32,
        env: &PeriodInitEnv<'_>,
        ctx: &mut dyn ClimateContext,
    ) {
        self.climate_values.poll_date(
            &JavaCalendar::new(year, month, day, 0, 0),
            env.clock,
            env.base_season,
            &env.values_env,
        );
        self.reseed(year, month, day, env.simplex_offset_a, env.simplex_offset_b);
        self.has_started_init = false;
        if !self.start_init(front, hours_since_start, ctx) {
            return;
        }
        if (0..STAGE_MAX).contains(&stage_only) {
            self.create_single_stage(stage_only, single_stage_duration as f64, ctx);
        } else {
            self.create_weather_pattern(env, ctx);
        }
        ctx.event(ClimateEvent::WeatherPeriodStart(self.id));
        self.end_init(ctx);
    }

    fn start_init(&mut self, front: &AirFront, hours_since_start: f64, ctx: &mut dyn ClimateContext) -> bool {
        if self.is_running || ctx.is_client() || front.strength < 0.1 {
            return false;
        }
        self.start_time = hours_since_start;
        self.front_cache.copy_from(front);
        let a = self.front_cache.front_wind_angle_degrees;
        self.wind_angle_dir_mod = if (90.0..270.0).contains(&a) { 1.0 } else { -1.0 };
        self.has_started_init = true;
        true
    }

    fn end_init(&mut self, ctx: &mut dyn ClimateContext) -> bool {
        if self.has_started_init && !self.is_running && !ctx.is_client() && !self.weather_stages.is_empty() {
            self.current_strength = 0.0;
            self.total_progress = 0.0;
            self.stage_progress = 0.0;
            self.is_running = true;
            if ctx.is_server() && !self.is_dummy {
                ctx.event(ClimateEvent::TransmitClimatePacket(1));
            }
            self.has_started_init = false;
            return true;
        }
        self.has_started_init = false;
        false
    }

    /// `startCreateModdedPeriod(warmFront, strength, angle)`.
    #[allow(clippy::too_many_arguments)]
    pub fn start_create_modded_period(
        &mut self,
        warm_front: bool,
        strength: f32,
        angle: f32,
        clock: &ClimateClock,
        simplex_a: f64,
        simplex_b: f64,
        ctx: &mut dyn ClimateContext,
    ) -> bool {
        let mut front = AirFront::default();
        let wind_angle = clamp(0.0, 360.0, angle);
        front.set_front_type(if warm_front { 1 } else { -1 });
        front.front_wind_angle_degrees = wind_angle;
        front.strength = clamp01(strength);
        self.reseed(clock.year, clock.month, clock.day_plus_one, simplex_a, simplex_b);
        self.has_started_init = false;
        if !self.start_init(&front, clock.world_age_hours, ctx) {
            return false;
        }
        self.clear_current_weather_stages();
        true
    }

    /// `endCreateModdedPeriod()`.
    pub fn end_create_modded_period(&mut self, ctx: &mut dyn ClimateContext) -> bool {
        if !self.end_init(ctx) {
            return false;
        }
        self.link_weather_stages();
        self.duration = self.weather_stages.iter().fold(0.0, |d, s| d + s.stage_duration);
        self.weather_stage_index = 0;
        self.weather_stages[0].start_stage(self.start_time);
        self.current_stage = Some(0);
        true
    }

    /// `stopWeatherPeriod()`; `vars` is the climate whose overrides reset
    /// (`None` for a dummy).
    pub fn stop_weather_period(&mut self, vars: Option<&mut ClimateVars>, ctx: &mut dyn ClimateContext) {
        self.clear_current_weather_stages();
        self.current_stage = None;
        self.reset_overrides(vars);
        self.is_running = false;
        self.total_progress = 0.0;
        self.stage_progress = 0.0;
        ctx.event(ClimateEvent::WeatherPeriodStop(self.id));
    }

    fn reset_overrides(&self, vars: Option<&mut ClimateVars>) {
        if !self.is_dummy {
            if let Some(v) = vars {
                v.reset_overrides();
            }
        }
    }

    fn link_weather_stages(&mut self) {
        let n = self.weather_stages.len();
        for (i, s) in self.weather_stages.iter_mut().enumerate() {
            s.previous_stage = i.checked_sub(1);
            s.next_stage = if i + 1 < n { Some(i + 1) } else { None };
            s.creation_finished = true;
        }
    }

    /// `clearCurrentWeatherStages()`.
    pub fn clear_current_weather_stages(&mut self) {
        self.weather_stages.clear();
    }

    fn create_single_stage(&mut self, stage: i32, duration: f64, ctx: &mut dyn ClimateContext) {
        if stage == STAGE_TROPICAL_STORM {
            self.cloud_color = self.cloud_color_tropical.clone();
        } else if stage == STAGE_BLIZZARD {
            self.cloud_color = self.cloud_color_blizzard.clone();
        }
        self.clear_current_weather_stages();
        self.create_and_add_stage(STAGE_START, 1.0, None, ctx);
        self.create_and_add_stage(stage, duration, None, ctx);
        self.create_and_add_stage(STAGE_CLEARING, 1.0, None, ctx);
        self.link_weather_stages();
        self.duration = self.weather_stages.iter().fold(0.0, |d, s| d + s.stage_duration);
        self.weather_stage_index = 0;
        self.weather_stages[0].start_stage(self.start_time);
        self.current_stage = Some(0);
    }

    fn create_weather_pattern(&mut self, env: &PeriodInitEnv<'_>, ctx: &mut dyn ClimateContext) {
        self.clear_current_weather_stages();
        let season = env.current_season;
        let mean_temperature = self.climate_values.get_day_mean_temperature();
        let mut storm_chance = 0.0f32;
        let mut tropical_chance = 0.0f32;
        let mut blizzard_chance = 0.0f32;
        let mut rain_time_multiplier = 1.0f32;
        let mut rnd = self.rand_range_f(0.0, 100.0);
        let winter_is_coming = env.game_mode == "Winter is Coming";
        let season_id = if winter_is_coming { 5 } else { season.get_season() };
        let strength = self.front_cache.strength;
        match season_id {
            5 => {
                self.cloud_color =
                    if rnd < 45.0 { self.cloud_color_purplish.clone() } else { self.cloud_color_blueish.clone() };
                storm_chance = 10.0;
                tropical_chance = 0.0;
                if mean_temperature < 5.5 {
                    blizzard_chance = clamp(0.0, 85.0, (5.5 - mean_temperature) * 3.0);
                    blizzard_chance += 25.0;
                    if mean_temperature < 2.5 {
                        blizzard_chance += 55.0;
                    } else if mean_temperature < 0.0 {
                        blizzard_chance += 75.0;
                    }
                    if blizzard_chance > 95.0 {
                        blizzard_chance = 95.0;
                    }
                } else {
                    blizzard_chance = 0.0;
                }
                if winter_is_coming {
                    blizzard_chance = if strength > 0.75 { 100.0 } else { 75.0 };
                    if strength > 0.5 {
                        rain_time_multiplier = 1.45;
                    }
                }
            }
            1 => {
                self.cloud_color =
                    if rnd < 75.0 { self.cloud_color_greenish.clone() } else { self.cloud_color_blueish.clone() };
                storm_chance = 75.0;
                tropical_chance = 10.0;
                rain_time_multiplier = 1.25;
            }
            2 => {
                self.cloud_color =
                    if rnd < 25.0 { self.cloud_color_greenish.clone() } else { self.cloud_color_reddish.clone() };
                storm_chance = 60.0;
                tropical_chance = 55.0;
            }
            3 => {
                self.cloud_color = self.cloud_color_reddish.clone();
                storm_chance = 75.0;
                tropical_chance = 80.0;
                rain_time_multiplier = 1.15;
            }
            4 => {
                self.cloud_color = if rnd < 50.0 {
                    self.cloud_color_reddish.clone()
                } else if rnd < 75.0 {
                    self.cloud_color_purplish.clone()
                } else {
                    self.cloud_color_blueish.clone()
                };
                storm_chance = 100.0;
                tropical_chance = 25.0;
                rain_time_multiplier = 1.35;
            }
            _ => {}
        }
        rain_time_multiplier *= rain_time_multiplier_mod(env.values_env.rain_modifier);
        let mut stages: Vec<WeatherStage> = Vec::new();
        if self.front_cache.front_type == 1 {
            let mut is_blizzard = false;
            let mut is_tropical = false;
            let mut is_storm = false;
            if strength > 0.75 {
                if tropical_chance > 0.0 && self.rand_range_f(0.0, 100.0) < tropical_chance {
                    is_tropical = true;
                } else if blizzard_chance > 0.0 && self.rand_range_f(0.0, 100.0) < blizzard_chance {
                    is_blizzard = true;
                }
            }
            if !is_blizzard
                && !is_tropical
                && strength > 0.5
                && storm_chance > 0.0
                && self.rand_range_f(0.0, 100.0) < storm_chance
            {
                is_storm = true;
            }
            let max_duration = self.rand_range_f(24.0, 48.0) * strength;
            let mut dur = 0.0f32;
            if is_tropical {
                let d = 8.0 + self.rand_range_f(0.0, 16.0 * strength);
                stages.push(self.create_stage(STAGE_TROPICAL_STORM, d as f64, None, None, ctx));
                self.cloud_color = self.cloud_color_tropical.clone();
                if self.rand_range_f(0.0, 100.0) < 60.0 * strength {
                    let d = 5.0 + self.rand_range_f(0.0, 5.0 * strength);
                    stages.push(self.create_stage(STAGE_STORM, d as f64, None, None, ctx));
                }
                if self.rand_range_f(0.0, 100.0) < 30.0 * strength {
                    let d = 5.0 + self.rand_range_f(0.0, 5.0 * strength);
                    stages.push(self.create_stage(STAGE_STORM, d as f64, None, None, ctx));
                }
            } else if is_blizzard {
                let d = 24.0 + self.rand_range_f(0.0, 24.0 * strength);
                stages.push(self.create_stage(STAGE_BLIZZARD, d as f64, None, None, ctx));
                self.cloud_color = self.cloud_color_blizzard.clone();
            } else if is_storm {
                let d = 5.0 + self.rand_range_f(0.0, 5.0 * strength);
                stages.push(self.create_stage(STAGE_STORM, d as f64, None, None, ctx));
                for (chance, spread) in [(70.0f32, 4.0f32), (50.0, 4.0), (25.0, 3.0), (12.5, 2.0)] {
                    if self.rand_range_f(0.0, 100.0) < chance * strength {
                        let d = 4.0 + self.rand_range_f(0.0, spread * strength);
                        stages.push(self.create_stage(STAGE_STORM, d as f64, None, None, ctx));
                    }
                }
            }
            for s in &stages {
                dur = (dur as f64 + s.stage_duration) as f32;
            }
            while dur < max_duration {
                let ws = match self.rand_range_i(0, 10) {
                    0 => {
                        let d = 1.0 + self.rand_range_f(0.0, 3.0 * strength);
                        self.create_stage(STAGE_MODERATE, d as f64, None, None, ctx)
                    }
                    1..=3 => {
                        let d = 2.0 + self.rand_range_f(0.0, 4.0 * strength);
                        self.create_stage(STAGE_SHOWERS, d as f64, None, None, ctx)
                    }
                    _ => {
                        let d = 2.0 + self.rand_range_f(0.0, 4.0 * strength);
                        self.create_stage(STAGE_HEAVY_PRECIP, d as f64, None, None, ctx)
                    }
                };
                dur = (dur as f64 + ws.stage_duration) as f32;
                stages.push(ws);
            }
        } else {
            if Rc::ptr_eq(&self.cloud_color, &self.cloud_color_reddish) {
                rnd = self.rand_range_f(0.0, 100.0);
                self.cloud_color =
                    if rnd < 50.0 { self.cloud_color_blueish.clone() } else { self.cloud_color_purplish.clone() };
            }
            let max_duration = self.rand_range_f(12.0, 24.0) * strength;
            let mut dur = 0.0f32;
            while dur < max_duration {
                let id = match self.rand_range_i(0, 10) {
                    0 => STAGE_SHOWERS,
                    1..=4 => STAGE_DRIZZLE,
                    _ => STAGE_MODERATE,
                };
                let d = 2.0 + self.rand_range_f(0.0, 3.0 * strength);
                let ws = self.create_stage(id, d as f64, None, None, ctx);
                dur = (dur as f64 + ws.stage_duration) as f32;
                stages.push(ws);
            }
        }
        // `Collections.shuffle(stages, seededRandom)`.
        for i in (2..=stages.len()).rev() {
            let j = self.seeded_random.next_int(i as i32) as usize;
            stages.swap(i - 1, j);
        }
        let mut chance = self.rand_range_f(30.0, 60.0);
        let d = 1.0 + self.rand_range_f(0.0, 2.0 * strength);
        let s = self.create_stage(STAGE_START, d as f64, None, None, ctx);
        self.weather_stages.push(s);
        let n = stages.len();
        for (i, st) in stages.into_iter().enumerate() {
            self.weather_stages.push(st);
            if i >= n - 1 || self.rand_range_f(0.0, 100.0) >= chance {
                continue;
            }
            let d = 1.0 + self.rand_range_f(0.0, 2.0 * strength);
            let s = self.create_stage(STAGE_CLEARING, d as f64, None, None, ctx);
            self.weather_stages.push(s);
            let d = 1.0 + self.rand_range_f(0.0, 3.0 * strength);
            let s = self.create_stage(STAGE_INTERMEZZO, d as f64, None, None, ctx);
            self.weather_stages.push(s);
            chance = self.rand_range_f(30.0, 60.0);
        }
        if self.weather_stages.last().map(|s| s.stage_id) != Some(STAGE_INTERMEZZO) {
            let d = 2.0 + self.rand_range_f(0.0, 3.0 * strength);
            let s = self.create_stage(STAGE_CLEARING, d as f64, None, None, ctx);
            self.weather_stages.push(s);
        }
        for s in &mut self.weather_stages {
            s.stage_duration *= rain_time_multiplier as f64;
        }
        self.link_weather_stages();
        self.duration = self.weather_stages.iter().fold(0.0, |d, s| d + s.stage_duration);
        let mut t = self.start_time;
        for s in &mut self.weather_stages {
            t = s.set_stage_start(t);
        }
        self.weather_stage_index = 0;
        self.weather_stages[0].start_stage(self.start_time);
        self.current_stage = Some(0);
    }

    /// `createAndAddModdedStage(moddedID, duration)`; the new stage's index.
    pub fn create_and_add_modded_stage(&mut self, mod_id: &str, duration: f64, ctx: &mut dyn ClimateContext) -> Option<usize> {
        self.create_and_add_stage(STAGE_MODDED, duration, Some(mod_id.to_string()), ctx)
    }

    /// `createAndAddStage(typeid, duration[, moddedID])`; the new stage's
    /// index.
    pub fn create_and_add_stage(
        &mut self,
        type_id: i32,
        duration: f64,
        mod_id: Option<String>,
        ctx: &mut dyn ClimateContext,
    ) -> Option<usize> {
        if self.is_running || !self.has_started_init || type_id == STAGE_MODDED && mod_id.is_none() {
            return None;
        }
        let idx = self.weather_stages.len();
        let s = self.create_stage(type_id, duration, mod_id, Some(idx), ctx);
        self.weather_stages.push(s);
        Some(idx)
    }

    fn create_stage(
        &mut self,
        type_id: i32,
        duration: f64,
        mod_id: Option<String>,
        index: Option<usize>,
        ctx: &mut dyn ClimateContext,
    ) -> WeatherStage {
        let mut stage = WeatherStage { stage_id: type_id, mod_id, ..WeatherStage::default() };
        stage.set_stage_duration(duration);
        let strength = self.front_cache.strength;
        use StrLerpVal::*;
        match type_id {
            STAGE_START => stage.lerp_entry_to(None, NextTarget),
            STAGE_SHOWERS => {
                stage.target_strength = strength * 0.5;
                stage.lerp_entry_to(Target, NextTarget);
            }
            STAGE_HEAVY_PRECIP => {
                stage.target_strength = strength;
                stage.lerp_entry_to(Target, Target);
            }
            STAGE_TROPICAL_STORM => {
                stage.target_strength = 1.0;
                stage.lerp_entry_to(Target, Target);
                stage.fog_strength = 0.6 + self.rand_range_f(0.0, 0.4);
            }
            STAGE_STORM | STAGE_KATEBOB_STORM => {
                stage.target_strength = strength;
                stage.lerp_entry_to(Target, Target);
                if self.rand_range_i(0, 100) < 33 {
                    stage.fog_strength = 0.1 + self.rand_range_f(0.0, 0.4);
                }
            }
            STAGE_CLEARING => {
                stage.target_strength = strength * 0.25;
                stage.lerp_entry_to(Target, None);
            }
            STAGE_MODERATE => {
                stage.target_strength = strength * 0.5;
                stage.lerp_entry_to(Target, NextTarget);
            }
            STAGE_DRIZZLE => {
                stage.target_strength = strength * 0.25;
                stage.lerp_entry_to(Target, NextTarget);
            }
            STAGE_BLIZZARD => {
                stage.target_strength = 1.0;
                stage.lerp_entry_to(Target, Target);
                stage.fog_strength = 0.55 + self.rand_range_f(0.0, 0.2);
            }
            STAGE_INTERMEZZO => {
                stage.target_strength = 0.0;
                stage.lerp_entry_to(Target, NextTarget);
            }
            STAGE_MODDED => {
                if let Some(i) = index {
                    ctx.event(ClimateEvent::InitModdedWeatherStage { period: self.id, stage: i, strength });
                }
            }
            _ => {}
        }
        stage
    }

    fn pushup(&self, idx: usize) -> f32 {
        let s = &self.weather_stages[idx];
        let half = self.front_cache.strength * 0.5;
        if s.linear_t < 0.1 {
            clerp(((self.current_time - s.stage_start) / (s.stage_duration * 0.1)) as f32, 0.0, half)
        } else if s.linear_t > 0.9 {
            clerp(1.0 - ((s.stage_end - self.current_time) / (s.stage_duration * 0.1)) as f32, half, 0.0)
        } else {
            half
        }
    }

    fn update_current_stage(&mut self, env: &mut PeriodUpdateEnv<'_>, ctx: &mut dyn ClimateContext) {
        if self.is_dummy {
            return;
        }
        let Some(idx) = self.current_stage else { return };
        self.is_blizzard = false;
        self.is_thunder_storm = false;
        self.is_tropical_storm = false;
        let st = self.weather_stages[idx].clone();
        let f = &mut env.vars.floats;
        match st.stage_id {
            STAGE_START => {
                self.rain_threshold = 0.35 - self.front_cache.strength * 0.2;
                f[FLOAT_FOG_INTENSITY].set_override(0.0, st.linear_t);
            }
            STAGE_SHOWERS => {
                f[FLOAT_FOG_INTENSITY].set_override(0.0, 1.0);
                let lerpmod = clamp01(st.parabolic_t * 3.0);
                f[FLOAT_WIND_INTENSITY].set_override(0.1 * self.weather_noise, lerpmod);
                f[FLOAT_WIND_ANGLE_INTENSITY].set_override(0.0, lerpmod);
            }
            STAGE_HEAVY_PRECIP => {
                let pushup = self.pushup(idx);
                self.weather_noise = pushup + self.weather_noise * (1.0 - pushup);
                f[FLOAT_FOG_INTENSITY].set_override(0.0, 1.0);
                let lerpmod = clamp01(st.parabolic_t * 3.0);
                f[FLOAT_WIND_INTENSITY].set_override(0.5 * self.weather_noise, lerpmod);
                f[FLOAT_WIND_ANGLE_INTENSITY].set_override(0.7 * self.weather_noise * self.wind_angle_dir_mod, lerpmod);
            }
            STAGE_TROPICAL_STORM | STAGE_STORM | STAGE_KATEBOB_STORM => {
                if st.stage_id == STAGE_TROPICAL_STORM {
                    self.is_tropical_storm = true;
                }
                self.is_thunder_storm = !self.is_tropical_storm;
                if !st.has_started_cloud {
                    let angl = self.front_cache.front_wind_angle_degrees;
                    let mut strength = self.front_cache.strength;
                    let mut radius = 8000.0 * strength;
                    let event_freq = strength;
                    let mut thunder_ratio = 0.6 * strength;
                    let duration = st.stage_duration;
                    let mut target_random_player = strength as f64 > 0.7;
                    let mut clouds = ctx.rand_next_range(1, 3);
                    if st.stage_id == STAGE_TROPICAL_STORM {
                        clouds = 1;
                        radius = 15000.0;
                        thunder_ratio = 0.8;
                        target_random_player = true;
                        strength = 1.0;
                    }
                    for _ in 0..clouds {
                        let offset = if st.stage_id == STAGE_KATEBOB_STORM { self.kate_bob_storm_progress } else { 0.0 };
                        let cloud = env.thunder.start_thunder_cloud(
                            strength,
                            angl,
                            radius,
                            event_freq,
                            thunder_ratio,
                            duration,
                            target_random_player,
                            offset,
                            env.world_age_hours,
                            ctx,
                        );
                        if st.stage_id == STAGE_KATEBOB_STORM && target_random_player {
                            if let Some(c) = cloud {
                                env.thunder.set_cloud_center(c, self.kate_bob_storm_x, self.kate_bob_storm_y, angl, ctx);
                            }
                        }
                        target_random_player = false;
                    }
                    self.weather_stages[idx].has_started_cloud = true;
                }
                let pushup = self.pushup(idx);
                self.weather_noise = pushup + self.weather_noise * (1.0 - pushup);
                let lerpmod = clamp01(st.parabolic_t * 3.0);
                let f = &mut env.vars.floats;
                if st.stage_id == STAGE_TROPICAL_STORM {
                    f[FLOAT_WIND_INTENSITY].set_override(0.4 + 0.6 * self.weather_noise, lerpmod);
                } else {
                    f[FLOAT_WIND_INTENSITY].set_override(0.2 + 0.5 * self.weather_noise, lerpmod);
                }
                f[FLOAT_WIND_ANGLE_INTENSITY].set_override(0.7 * self.weather_noise * self.wind_angle_dir_mod, lerpmod);
                if env.fog_quality != 2 {
                    if st.fog_strength > 0.0 {
                        f[FLOAT_FOG_INTENSITY].set_override(st.fog_strength, lerpmod);
                        let tint = if st.stage_id == STAGE_TROPICAL_STORM { env.fog_tint_tropical } else { env.fog_tint_storm };
                        env.vars.colors[COLOR_NEW_FOG].set_override(&tint.borrow(), lerpmod);
                    } else {
                        f[FLOAT_FOG_INTENSITY].set_override(0.0, 1.0);
                    }
                }
            }
            STAGE_CLEARING => f[FLOAT_FOG_INTENSITY].set_override(0.0, 1.0 - st.linear_t),
            STAGE_MODERATE | STAGE_DRIZZLE | STAGE_INTERMEZZO => f[FLOAT_FOG_INTENSITY].set_override(0.0, 1.0),
            STAGE_BLIZZARD => {
                self.is_blizzard = true;
                let pushup = self.pushup(idx);
                self.weather_noise = pushup + self.weather_noise * (1.0 - pushup);
                let lerpmod = clamp01(st.parabolic_t * 3.0);
                f[FLOAT_WIND_INTENSITY].set_override(0.75 + 0.25 * self.weather_noise, lerpmod);
                f[FLOAT_WIND_ANGLE_INTENSITY].set_override(0.7 * self.weather_noise * self.wind_angle_dir_mod, lerpmod);
                if env.fog_quality != 2 {
                    if st.fog_strength > 0.0 {
                        f[FLOAT_FOG_INTENSITY].set_override(st.fog_strength, lerpmod);
                    } else {
                        f[FLOAT_FOG_INTENSITY].set_override(1.0, lerpmod);
                    }
                }
            }
            STAGE_MODDED => ctx.event(ClimateEvent::UpdateModdedWeatherStage {
                period: self.id,
                stage: idx,
                strength: self.front_cache.strength,
            }),
            _ => {
                env.vars.reset_overrides();
                self.is_running = false;
                if ctx.is_server() {
                    ctx.event(ClimateEvent::TransmitClimatePacket(1));
                }
            }
        }
    }

    /// `update(hoursSinceStart)`.
    pub fn update(&mut self, hours_since_start: f64, env: &mut PeriodUpdateEnv<'_>, ctx: &mut dyn ClimateContext) {
        if ctx.is_client() || self.is_dummy {
            return;
        }
        if !self.is_running || self.current_stage().is_none() || self.weather_stage_index < 0 || self.weather_stages.is_empty() {
            if self.is_running {
                env.vars.reset_overrides();
                self.is_running = false;
                ctx.event(ClimateEvent::WeatherPeriodComplete(self.id));
                if ctx.is_server() {
                    ctx.event(ClimateEvent::TransmitClimatePacket(1));
                }
            }
            return;
        }
        let cur = self.current_stage.unwrap_or(0);
        if self.current_time > self.weather_stages[cur].stage_end {
            self.weather_stage_index += 1;
            ctx.event(ClimateEvent::WeatherPeriodStage(self.id));
            if self.weather_stage_index as usize >= self.weather_stages.len() {
                self.is_running = false;
                self.current_stage = None;
                env.vars.reset_overrides();
                if ctx.is_server() {
                    ctx.event(ClimateEvent::TransmitClimatePacket(1));
                }
                return;
            }
            self.weather_stages[cur].exit_strength = self.current_strength;
            let n = self.weather_stage_index as usize;
            self.current_stage = Some(n);
            self.weather_stages[n].entry_strength = self.current_strength;
            self.weather_stages[n].start_stage(hours_since_start);
        }
        let idx = self.current_stage.unwrap_or(0);
        self.current_time = hours_since_start;
        let s3 = 0.3 * self.front_cache.strength;
        self.weather_noise = s3 + noise2(hours_since_start, 24000.0) as f32 * (1.0 - s3);
        self.weather_noise = (self.weather_noise + 1.0) * 0.5;
        let kb = self.kate_bob_storm_progress;
        self.weather_stages[idx].update_t(self.current_time, kb);
        self.stage_progress = self.weather_stages[idx].linear_t;
        self.total_progress = (self.current_time - self.weather_stages[0].stage_start) as f32 / self.duration as f32;
        self.total_progress = clamp01(self.total_progress);
        self.current_strength = self.stage_strength(idx);
        self.update_current_stage(env, ctx);
        let temperature_mod = clamp(-1.0, 1.0, self.current_strength * 2.0) * 7.0;
        let temp_internal = env.vars.floats[FLOAT_TEMPERATURE].internal_value;
        self.temperature_influence =
            if self.front_cache.front_type == 1 { temp_internal - temperature_mod } else { temp_internal + temperature_mod };
        if !self.is_running {
            return;
        }
        if self.weather_noise > self.rain_threshold {
            self.precipitation_final = (self.weather_noise - self.rain_threshold) / (1.0 - self.rain_threshold);
            self.precipitation_final *= self.current_strength;
        } else {
            self.precipitation_final = 0.0;
        }
        let stage_id = self.weather_stages[idx].stage_id;
        let parabolic_t = self.weather_stages[idx].parabolic_t;
        let v = &mut *env.vars;
        let night = v.floats[FLOAT_NIGHT_STRENGTH].internal_value;
        let current_darkness = self.precipitation_final;
        let str_night_fix = current_darkness * (1.0 - night);
        let mut val_cloud = 0.5f32;
        val_cloud += 0.5 * (1.0 - night);
        val_cloud = jmax(val_cloud, v.floats[FLOAT_CLOUD_INTENSITY].internal_value);
        let mut base_amb_mod = 0.55f32;
        if env.fog_quality != 2 && stage_id == STAGE_TROPICAL_STORM {
            base_amb_mod += 0.35 * parabolic_t;
        }
        let mut ambience = 1.0 - base_amb_mod * current_darkness;
        ambience = jmin(ambience, 1.0 - night);
        if env.fog_quality != 2 && stage_id == STAGE_BLIZZARD {
            val_cloud *= 1.0 - 0.75 * parabolic_t;
        }
        let cs = self.current_strength;
        v.floats[FLOAT_CLOUD_INTENSITY].set_override(val_cloud, cs);
        v.floats[FLOAT_PRECIPITATION_INTENSITY].set_override(self.precipitation_final, 1.0);
        v.colors[COLOR_GLOBAL_LIGHT].set_override(&self.cloud_color.borrow(), str_night_fix);
        v.floats[FLOAT_GLOBAL_LIGHT_INTENSITY].set_override(0.4, str_night_fix);
        v.floats[FLOAT_DESATURATION].set_override(0.3, cs);
        v.floats[FLOAT_TEMPERATURE].set_override(self.temperature_influence, cs);
        v.floats[FLOAT_AMBIENT].set_override(ambience, current_darkness);
        v.floats[FLOAT_DAYLIGHT_STRENGTH].set_override(ambience, current_darkness);
        if v.floats[FLOAT_TEMPERATURE].final_value < 0.0 && env.season_is_winter || env.winter_is_coming {
            v.bools[BOOL_IS_SNOW].set_override(true);
        } else {
            v.bools[BOOL_IS_SNOW].set_enable_override(false);
        }
    }

    /// The part of `load` after the stages are read: the wind direction,
    /// links, duration and current stage.
    pub fn after_load(&mut self) {
        let a = self.front_cache.front_wind_angle_degrees;
        self.wind_angle_dir_mod = if (90.0..270.0).contains(&a) { 1.0 } else { -1.0 };
        for s in &mut self.weather_stages {
            s.stage_end = s.stage_start + s.stage_duration;
        }
        self.link_weather_stages();
        self.duration = self.weather_stages.iter().fold(0.0, |d, s| d + s.stage_duration);
        if self.weather_stage_index >= 0 && (self.weather_stage_index as usize) < self.weather_stages.len() {
            self.current_stage = Some(self.weather_stage_index as usize);
        } else {
            // "WeatherPeriod: Couldnt load stages correctly."
            self.is_running = false;
        }
    }

    /// `readNetWeatherData` with the period stopped (`isRunning == false`).
    pub fn clear_net_weather_data(&mut self) {
        self.is_running = false;
        self.is_thunder_storm = false;
        self.is_tropical_storm = false;
        self.is_blizzard = false;
        self.current_strength = 0.0;
        self.duration = 0.0;
        self.total_progress = 0.0;
        self.stage_progress = 0.0;
    }
}
