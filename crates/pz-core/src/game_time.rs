//! The persistent part of `zombie.GameTime` and of the weather it saves
//! with it (`ClimateManager`, `WeatherPeriod`, `ThunderStorm`): the fields
//! `map_t.bin` carries, with the original's initial values. The running
//! clock and weather simulation are not ported.
//!
//! Saving and loading is in [`crate::game_time_save`].

use pz_kahlua::TableRef;

/// `zombie.GameTime` (saved fields).
#[derive(Clone, Debug)]
pub struct GameTime {
    pub multiplier: f32,
    pub nights_survived: i32,
    pub target_zombies: i32,
    pub last_time_of_day: f32,
    pub time_of_day: f32,
    pub day: i32,
    pub month: i32,
    pub year: i32,
    /// `table` (`getModData()`); `None` until created.
    pub table: Option<TableRef>,
    pub helicopter_day1: i32,
    pub helicopter_time1_start: i32,
    pub helicopter_time1_end: i32,
    pub climate: ClimateManager,
}

impl Default for GameTime {
    /// `new GameTime()`.
    fn default() -> Self {
        GameTime {
            multiplier: 1.0,
            nights_survived: 0,
            // `(int) minZombieCountStart`
            target_zombies: 750,
            last_time_of_day: 0.0,
            time_of_day: 9.0,
            day: 22,
            month: 7,
            year: 2012,
            table: None,
            helicopter_day1: 0,
            helicopter_time1_start: 0,
            helicopter_time1_end: 0,
            climate: ClimateManager::default(),
        }
    }
}

impl GameTime {
    /// `getModData()`: creates the table on first use.
    pub fn mod_data(&mut self) -> TableRef {
        self.table.get_or_insert_with(TableRef::new).clone()
    }
}

/// `ClimateManager.AirFront`.
#[derive(Clone, Debug, PartialEq)]
pub struct AirFront {
    pub days: f32,
    pub max_noise: f32,
    pub total_noise: f32,
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

/// `ClimateColorInfo`: interior then exterior RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimateColorInfo {
    pub interior: [f32; 4],
    pub exterior: [f32; 4],
}

impl ClimateColorInfo {
    /// `new ClimateColorInfo(r, g, b, a)`.
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        ClimateColorInfo {
            interior: [r, g, b, a],
            exterior: [r, g, b, a],
        }
    }
}

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
}

/// `WeatherPeriod.WeatherStage` (saved fields).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeatherStage {
    pub stage_id: i32,
    pub stage_duration: f64,
    pub stage_start: f64,
    pub entry_strength: f32,
    pub exit_strength: f32,
    pub target_strength: f32,
    pub lerp_mid_val: StrLerpVal,
    pub lerp_end_val: StrLerpVal,
    pub has_started_cloud: bool,
    pub mod_id: Option<String>,
    pub fog_strength: f32,
}

impl WeatherStage {
    /// `setStageDuration(time)`: at least one hour.
    pub fn set_stage_duration(&mut self, time: f64) {
        self.stage_duration = if time < 1.0 { 1.0 } else { time };
    }
}

/// `WeatherPeriod` (saved fields).
#[derive(Clone, Debug, PartialEq)]
pub struct WeatherPeriod {
    pub is_running: bool,
    pub weather_stage_index: i32,
    pub current_strength: f32,
    pub rain_threshold: f32,
    pub is_thunder_storm: bool,
    pub is_tropical_storm: bool,
    pub is_blizzard: bool,
    pub front_cache: AirFront,
    pub weather_stages: Vec<WeatherStage>,
    pub cloud_color: ClimateColorInfo,
}

impl Default for WeatherPeriod {
    fn default() -> Self {
        WeatherPeriod {
            is_running: false,
            weather_stage_index: 0,
            current_strength: 0.0,
            rain_threshold: 0.0,
            is_thunder_storm: false,
            is_tropical_storm: false,
            is_blizzard: false,
            front_cache: AirFront::default(),
            weather_stages: Vec::new(),
            cloud_color: ClimateColorInfo::new(0.4, 0.2, 0.2, 0.4),
        }
    }
}

/// `ThunderStorm.ThunderCloud` (saved fields).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThunderCloud {
    pub is_running: bool,
    pub start_x: i32,
    pub start_y: i32,
    pub end_x: i32,
    pub end_y: i32,
    pub radius: f32,
    pub angle: f32,
    pub strength: f32,
    pub thunder_ratio: f32,
    pub start_time: f64,
    pub end_time: f64,
    pub duration: f64,
    pub percentage_offset: f32,
}

/// `ClimateManager` (saved fields). The simplex offsets are random in the
/// original's `init()`; they start at 0 here, like the Java fields.
#[derive(Clone, Debug, Default)]
pub struct ClimateManager {
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
    /// `ThunderStorm.clouds` (always three).
    pub thunder_clouds: [ThunderCloud; 3],
    /// `modDataTable`.
    pub mod_data_table: Option<TableRef>,
}
