//! `zombie.GameTime`: the clock, its multipliers and the day/month/year
//! rollover, with the climate ([`ClimateManager`]) it saves with.
//!
//! Saving and loading is in [`crate::game_time_save`]. The engine
//! singletons `update` reads (sandbox options, players, speed controls,
//! `Rand`) come through [`GameTimeContext`]; the systems it pokes
//! (ambient sounds, the helicopter, `ZomboidRadio`, Lua events) are
//! [`GameTimeEvent`]s.

pub use crate::weather::manager::AirFront;
pub use crate::weather::{ClimateColorInfo, ClimateManager, StrLerpVal, ThunderCloud, WeatherPeriod, WeatherStage};
use crate::weather::calendar::JavaCalendar;
use crate::weather::{ClimateClock, ClimateContext};
use pz_kahlua::TableRef;

pub const MINUTES_PER_HOUR: f32 = 60.0;
pub const SECONDS_PER_HOUR: f32 = 3600.0;
pub const MULTIPLIER: f32 = 0.8;
pub const START_OF_NEW_DAY: f32 = 7.0;
pub const THIRTY_FPS_SCALE: f32 = 1.6;

/// What `GameTime.update` asks other systems to do, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum GameTimeEvent {
    /// `AmbientStreamManager.instance.addRandomAmbient()`.
    AddRandomAmbient,
    /// `AmbientStreamManager.instance.doGunEvent()`.
    DoGunEvent,
    /// `IsoWorld.instance.helicopter.pickRandomTarget()`.
    HelicopterPickRandomTarget,
    /// Living players' `setHoursSurvived(+hours)` and the asleep players'
    /// `setAsleepTime(+hours)` / `SleepingEvent.update`.
    PlayersTimePassed(f32),
    /// `ZomboidRadio.getInstance().UpdateScripts(hour, minutes)`.
    RadioUpdateScripts { hour: i32, minutes: i32 },
    /// `GameServer.syncClock()`.
    SyncClock,
    /// `LuaEventManager.triggerEvent(name)` (`EveryHours`, `EveryDays`,
    /// `EveryTenMinutes`, `EveryOneMinute`).
    Lua(&'static str),
}

/// The engine state `GameTime` reads beyond the climate's.
pub trait GameTimeContext: ClimateContext {
    /// `SandboxOptions.metaEvent` (1 never, 2 sometimes, 3 often).
    fn meta_event(&self) -> i32 {
        2
    }
    /// `SandboxOptions.helicopter` (1 never .. 4 often).
    fn helicopter(&self) -> i32 {
        2
    }
    /// `GameTime.isGamePaused()`.
    fn is_game_paused(&self) -> bool {
        false
    }
    /// `IsoPlayer.getInstance() != null && IsoPlayer.allPlayersAsleep()`.
    fn all_players_asleep(&self) -> bool {
        false
    }
    /// `PerformanceSettings.getLockFPS()`.
    fn lock_fps(&self) -> i32 {
        60
    }
    /// `Core.lastStand`.
    fn last_stand(&self) -> bool {
        false
    }
    /// `DebugOptions.instance.freezeTimeOfDay`.
    fn freeze_time_of_day(&self) -> bool {
        false
    }
    /// `GameTime.getSlomoMultiplier()`.
    fn slomo_multiplier(&self) -> f32 {
        1.0
    }
    /// `GameServer.fastForward` / `GameClient.fastForward &&
    /// GameWindow.isIngameState()`.
    fn fast_forward(&self) -> bool {
        false
    }
    /// `ServerOptions.instance.fastForwardMultiplier`.
    fn fast_forward_multiplier(&self) -> f64 {
        40.0
    }
    /// `IsoWorld.instance.helicopter.isActive()`.
    fn helicopter_active(&self) -> bool {
        false
    }
    /// Receives the side effects.
    fn game_time_event(&mut self, _e: GameTimeEvent) {}
}

impl GameTimeContext for crate::weather::DefaultClimateContext {}

/// `zombie.GameTime`.
#[derive(Clone, Debug)]
pub struct GameTime {
    // Saved.
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

    pub fps_multiplier: f32,
    pub moon: f32,
    pub server_time_of_day: f32,
    pub server_last_time_of_day: f32,
    pub server_new_days: i32,
    pub light_source_update: f32,
    pub multiplier_bias: f32,
    pub last_last_time_of_day: f32,
    pub per_object_multiplier: f32,
    pub ambient: f32,
    pub ambient_max: f32,
    pub ambient_min: f32,
    pub start_day: i32,
    pub start_month: i32,
    pub start_year: i32,
    pub start_time_of_day: f32,
    pub max_zombie_count_start: f32,
    pub min_zombie_count_start: f32,
    pub max_zombie_count: f32,
    pub min_zombie_count: f32,
    pub view_dist_max: f32,
    pub view_dist_min: f32,
    pub hours_survived: f64,
    pub minutes_per_day: f32,
    pub gun_fire_event_today: bool,
    pub num_gun_fire_events: i32,
    pub gun_fire_times: [f32; 5],
    pub last_clock_sync: i64,
    pub minutes_mod: i32,
    pub thunder_day: bool,
    pub random_ambient_today: bool,
    pub dusk: i32,
    pub dawn: i32,
    pub night_min: f32,
    pub night_max: f32,
    pub minutes_stamp: i64,
    pub previous_minute_stamp: i64,
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
            fps_multiplier: 1.0,
            moon: 0.0,
            server_time_of_day: 0.0,
            server_last_time_of_day: 0.0,
            server_new_days: 0,
            light_source_update: 0.0,
            multiplier_bias: 1.0,
            last_last_time_of_day: 0.0,
            per_object_multiplier: 1.0,
            ambient: 0.9,
            ambient_max: 1.0,
            ambient_min: 0.24,
            start_day: 22,
            start_month: 7,
            start_year: 2012,
            start_time_of_day: 9.0,
            max_zombie_count_start: 750.0,
            min_zombie_count_start: 750.0,
            max_zombie_count: 750.0,
            min_zombie_count: 750.0,
            view_dist_max: 42.0,
            view_dist_min: 19.0,
            hours_survived: 0.0,
            minutes_per_day: 30.0,
            gun_fire_event_today: false,
            num_gun_fire_events: 1,
            gun_fire_times: [0.0; 5],
            last_clock_sync: 0,
            minutes_mod: -1,
            thunder_day: true,
            random_ambient_today: true,
            dusk: 3,
            dawn: 12,
            night_min: 0.0,
            night_max: 1.0,
            minutes_stamp: 0,
            previous_minute_stamp: 0,
        }
    }
}

impl GameTime {
    /// `getModData()`: creates the table on first use.
    pub fn mod_data(&mut self) -> TableRef {
        self.table.get_or_insert_with(TableRef::new).clone()
    }

    /// The values the climate reads, now.
    pub fn clock(&self, ctx: &dyn GameTimeContext) -> ClimateClock {
        ClimateClock {
            year: self.year,
            month: self.month,
            day_plus_one: self.day_plus_one(),
            hour: self.hour(),
            minutes: self.minutes(),
            time_of_day: self.time_of_day,
            world_age_hours: self.world_age_hours(),
            minutes_stamp: self.minutes_stamp,
            multiplier: self.get_multiplier(ctx),
            multiplied_seconds: self.multiplied_seconds_since_last_update(ctx),
        }
    }

    /// `ClimateManager.update()` on this clock; copies the view distance it
    /// sets back (`setViewDistMin/Max`).
    pub fn update_climate(&mut self, ctx: &mut dyn GameTimeContext) {
        let clock = self.clock(ctx);
        self.climate.update(&clock, ctx);
        self.apply_climate_view_dist();
    }

    /// Copies the climate's last `setViewDistMin/Max`.
    pub fn apply_climate_view_dist(&mut self) {
        if let Some((min, max)) = self.climate.view_dist_min_max {
            self.view_dist_min = min;
            self.view_dist_max = max;
        }
    }

    /// `init()`: the start date and time, and the helicopter day.
    pub fn init(&mut self, ctx: &mut dyn GameTimeContext) {
        self.day = self.start_day;
        self.time_of_day = self.start_time_of_day;
        self.month = self.start_month;
        self.year = self.start_year;
        if ctx.helicopter() != 1 {
            self.helicopter_day1 = ctx.rand_next_range(6, 10);
            self.helicopter_time1_start = ctx.rand_next_range(9, 19);
            self.helicopter_time1_end = self.helicopter_time1_start + ctx.rand_next(4) + 1;
        }
        self.set_minutes_stamp();
    }

    /// `Lerp(start, end, delta)` (delta clamped to [0, 1]).
    pub fn lerp(start: f32, end: f32, delta: f32) -> f32 {
        let delta = delta.clamp(0.0, 1.0);
        start + (end - start) * delta
    }

    /// `TimeLerp(startVal, endVal, startTime, endTime)`.
    pub fn time_lerp(&self, start_val: f32, end_val: f32, start_time: f32, end_time: f32) -> f32 {
        let mut tod = self.time_of_day;
        let mut start = start_time;
        let mut end = end_time;
        if end < start {
            end += 24.0;
        }
        let mut reverse = false;
        if tod > end && tod > start || tod < end && tod < start {
            reverse = true;
            start += 24.0;
            std::mem::swap(&mut start, &mut end);
            if tod < start {
                tod += 24.0;
            }
        }
        let dist = end - start;
        let current = tod - start;
        let mut delta = 0.0f32;
        if current > dist {
            delta = 1.0;
        }
        if current < dist && current > 0.0 {
            delta = current / dist;
        }
        if reverse {
            delta = 1.0 - delta;
        }
        delta = (delta - 0.5) * 2.0;
        let sign = if (delta as f64) < 0.0 { -1.0 } else { 1.0 };
        delta = delta.abs();
        delta = 1.0 - delta;
        delta = (delta as f64).powf(8.0) as f32;
        delta = 1.0 - delta;
        delta *= sign;
        delta = delta * 0.5 + 0.5;
        Self::lerp(start_val, end_val, delta)
    }

    /// `daysInMonth(year, month)`.
    pub fn days_in_month(year: i32, month: i32) -> i32 {
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let d = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        d[month as usize]
    }

    pub fn get_delta_minutes_per_day(&self) -> f32 {
        30.0 / self.minutes_per_day
    }
    /// `getNight()`: the climate's night strength.
    pub fn get_night(&self) -> f32 {
        self.climate.get_night_strength()
    }
    pub fn get_night_min(&self) -> f32 {
        1.0 - self.night_min
    }
    pub fn set_night_min(&mut self, v: f32) {
        self.night_min = 1.0 - v;
    }
    pub fn get_night_max(&self) -> f32 {
        1.0 - self.night_max
    }
    pub fn set_night_max(&mut self, v: f32) {
        self.night_max = 1.0 - v;
    }

    /// `getMinutes()`.
    pub fn minutes(&self) -> i32 {
        ((self.time_of_day - self.time_of_day as i32 as f32) * 60.0) as i32
    }

    /// `getHour()`.
    pub fn hour(&self) -> i32 {
        let sec = ((self.time_of_day * 3600.0) as f64).floor();
        (sec / 3600.0).floor() as i32
    }

    /// `getDayPlusOne()`.
    pub fn day_plus_one(&self) -> i32 {
        self.day + 1
    }

    /// `getWorldAgeHours()` (computed in floats, as the original does).
    pub fn world_age_hours(&self) -> f64 {
        let mut e = self.nights_survived as f32 * 24.0;
        e = if self.time_of_day >= 7.0 { e + (self.time_of_day - 7.0) } else { e + (self.time_of_day + 17.0) };
        e as f64
    }

    /// `getWorldAgeDaysSinceBegin()` with `SandboxOptions.getTimeSinceApo()`.
    pub fn world_age_days_since_begin(&self, time_since_apo: i32) -> f64 {
        self.world_age_hours() / 24.0 + ((time_since_apo - 1) * 30) as f64
    }

    /// `setMinutesStamp()`.
    pub fn set_minutes_stamp(&mut self) {
        self.minutes_stamp = self.world_age_hours() as i64 * 60 + self.minutes() as i64;
    }

    /// `getMultiplier()`.
    pub fn get_multiplier(&self, ctx: &dyn GameTimeContext) -> f32 {
        if !ctx.is_server() && !ctx.is_client() && ctx.all_players_asleep() {
            return 200.0 * (30.0 / ctx.lock_fps() as f32);
        }
        let mut m = 1.0f32;
        if ctx.fast_forward() && (ctx.is_server() || ctx.is_client()) {
            m = ctx.fast_forward_multiplier() as f32 / self.get_delta_minutes_per_day();
        }
        m *= self.multiplier;
        m *= self.fps_multiplier;
        m *= self.multiplier_bias;
        m *= self.per_object_multiplier;
        m *= ctx.slomo_multiplier();
        m * 0.8
    }

    /// `getUnmoddedMultiplier()`.
    pub fn get_unmodded_multiplier(&self, ctx: &dyn GameTimeContext) -> f32 {
        if !ctx.is_server() && !ctx.is_client() && ctx.all_players_asleep() {
            return 200.0 * (30.0 / ctx.lock_fps() as f32);
        }
        self.multiplier * self.fps_multiplier * self.per_object_multiplier
    }

    /// `getTimeDelta()`.
    pub fn get_time_delta(&self, ctx: &dyn GameTimeContext) -> f32 {
        self.time_delta_from_multiplier(self.get_multiplier(ctx))
    }
    /// `getTimeDeltaFromMultiplier(m)`.
    pub fn time_delta_from_multiplier(&self, m: f32) -> f32 {
        m / 0.8 / self.multiplier_bias / 60.0
    }
    /// `getMultiplierFromTimeDelta(d)`.
    pub fn multiplier_from_time_delta(&self, d: f32) -> f32 {
        d * 0.8 * self.multiplier_bias * 60.0
    }
    /// `getInvMultiplier()`.
    pub fn get_inv_multiplier(&self, ctx: &dyn GameTimeContext) -> f32 {
        1.0 / self.get_multiplier(ctx)
    }
    /// `getTrueMultiplier()`.
    pub fn get_true_multiplier(&self) -> f32 {
        self.multiplier * self.per_object_multiplier
    }
    /// `getThirtyFPSMultiplier()`.
    pub fn get_thirty_fps_multiplier(&self, ctx: &dyn GameTimeContext) -> f32 {
        self.get_multiplier(ctx) / 1.6
    }
    /// `getMultiplierInMenu()`.
    pub fn get_multiplier_in_menu(&self) -> f32 {
        self.fps_multiplier * 0.8
    }
    /// `getRealworldSecondsSinceLastUpdate()`.
    pub fn realworld_seconds_since_last_update(&self) -> f32 {
        0.016666668 * self.fps_multiplier
    }
    /// `getMultipliedSecondsSinceLastUpdate()`.
    pub fn multiplied_seconds_since_last_update(&self, ctx: &dyn GameTimeContext) -> f32 {
        0.016666668 * self.get_unmodded_multiplier(ctx)
    }
    /// `getGameWorldSecondsSinceLastUpdate()`.
    pub fn game_world_seconds_since_last_update(&self, ctx: &dyn GameTimeContext) -> f32 {
        let dif = 1440.0 / self.minutes_per_day;
        self.get_time_delta(ctx) * dif
    }

    /// `isDay()`.
    pub fn is_day(&self, ctx: &dyn GameTimeContext) -> bool {
        if ctx.day_night_cycle() == 2 {
            return true;
        }
        match self.climate.get_season() {
            Some(s) => self.time_of_day >= s.get_dawn() && self.time_of_day <= s.get_dusk(),
            None => false,
        }
    }
    /// `isNight()`.
    pub fn is_night(&self, ctx: &dyn GameTimeContext) -> bool {
        ctx.day_night_cycle() == 3 || !self.is_day(ctx)
    }

    pub fn set_helicopter_day(&mut self, d: i32) {
        self.helicopter_day1 = d.max(0);
    }
    pub fn set_helicopter_start_hour(&mut self, h: i32) {
        self.helicopter_time1_start = h.clamp(0, 24);
    }
    pub fn set_helicopter_end_hour(&mut self, h: i32) {
        self.helicopter_time1_end = h.clamp(0, 24);
    }
    pub fn set_ambient_max(&mut self, v: f32) {
        self.ambient_max = v.clamp(0.0, 1.0);
    }
    pub fn set_ambient_min(&mut self, v: f32) {
        self.ambient_min = v.clamp(0.0, 1.0);
    }

    /// `getCalender().getTimeInMillis()` (seconds and milliseconds at 0;
    /// the original keeps the wall clock's from `new GregorianCalendar()`).
    pub fn calendar_time_in_millis(&self) -> i64 {
        JavaCalendar::new(self.year, self.month, self.day, self.time_of_day as i32, self.minutes()).time_in_millis()
    }

    fn advance_one_day(&mut self, ctx: &mut dyn GameTimeContext) {
        self.day += 1;
        if self.day >= Self::days_in_month(self.year, self.month) {
            self.day = 0;
            self.month += 1;
            if self.month >= 12 {
                self.month = 0;
                self.year += 1;
            }
        }
        ctx.game_time_event(GameTimeEvent::Lua("EveryDays"));
    }

    fn do_meta_events(&mut self, ctx: &mut dyn GameTimeContext) {
        let meta = match ctx.meta_event() {
            1 => -1,
            3 => 2,
            _ => 3,
        };
        self.gun_fire_event_today = meta != -1 && ctx.rand_next(meta) == 0;
        if self.gun_fire_event_today {
            self.num_gun_fire_events = 1;
            for n in 0..self.num_gun_fire_events as usize {
                self.gun_fire_times[n] = ctx.rand_next(18000) as f32 / 1000.0 + 7.0;
            }
        }
        self.random_ambient_today = true;
    }

    /// `update(bSleeping)`: advances the clock one frame. The climate's own
    /// `update` is separate ([`GameTime::update_climate`]), as in the
    /// original (`IsoWorld.update`).
    pub fn update(&mut self, ctx: &mut dyn GameTimeContext) {
        let ms = ctx.current_time_millis();
        let meta_sandbox = match ctx.meta_event() {
            1 => -1,
            3 => 6000,
            _ => 9000,
        };
        if !ctx.is_client() && self.random_ambient_today && meta_sandbox != -1 {
            // `Rand.AdjustForFramerate(metaSandbox)`.
            let chance = if ctx.is_server() {
                (meta_sandbox as f32 * 0.33333334) as i32
            } else {
                (meta_sandbox as f32 * (ctx.lock_fps() as f32 / 30.0)) as i32
            };
            if ctx.rand_next(chance) == 0 && !ctx.is_game_paused() {
                ctx.game_time_event(GameTimeEvent::AddRandomAmbient);
                self.random_ambient_today = ctx.meta_event() == 3 && ctx.rand_next(3) == 0;
            }
        }
        if ctx.is_server() || !ctx.is_client() {
            if self.gun_fire_event_today {
                for n in 0..self.num_gun_fire_events as usize {
                    if self.time_of_day > self.gun_fire_times[n] && self.last_last_time_of_day < self.gun_fire_times[n] {
                        ctx.game_time_event(GameTimeEvent::DoGunEvent);
                    }
                }
            }
            if self.nights_survived == self.helicopter_day1
                && self.time_of_day > self.helicopter_time1_start as f32
                && self.time_of_day < self.helicopter_time1_end as f32
                && !ctx.helicopter_active()
                && ctx.rand_next((800.0 * self.get_inv_multiplier(ctx)) as i32) == 0
            {
                self.helicopter_time1_start = (self.helicopter_time1_start as f32 + 0.5) as i32;
                ctx.game_time_event(GameTimeEvent::HelicopterPickRandomTarget);
            }
            let heli = ctx.helicopter();
            if self.nights_survived > self.helicopter_day1 && (heli == 3 || heli == 4) {
                if heli == 3 {
                    self.helicopter_day1 = self.nights_survived + ctx.rand_next_range(10, 16);
                }
                if heli == 4 {
                    self.helicopter_day1 = self.nights_survived + ctx.rand_next_range(6, 10);
                }
                self.helicopter_time1_start = ctx.rand_next_range(9, 19);
                self.helicopter_time1_end = self.helicopter_time1_start + ctx.rand_next(4) + 1;
            }
        }
        let previous_hour = self.hour();
        let last_time_of_day = self.time_of_day;
        if !ctx.is_game_paused() {
            let mut time = 1.0 / self.minutes_per_day / 60.0 * self.get_multiplier(ctx) / 2.0;
            if ctx.last_stand() {
                time = 1.0 / self.minutes_per_day / 60.0 * self.get_unmodded_multiplier(ctx) / 2.0;
            }
            if ctx.freeze_time_of_day() {
                time = 0.0;
            }
            self.time_of_day += time;
            ctx.game_time_event(GameTimeEvent::PlayersTimePassed(time));
        }
        let mut sync_clock = ctx.is_server() && (ms - self.last_clock_sync >= 10000 || ctx.fast_forward());
        if last_time_of_day <= 7.0 && self.time_of_day > 7.0 {
            self.nights_survived += 1;
            if ctx.is_server() {
                sync_clock = true;
            }
            if !ctx.is_client() {
                self.do_meta_events(ctx);
            }
        }
        if ctx.is_client() {
            if self.time_of_day >= 24.0 {
                self.time_of_day -= 24.0;
            }
            while self.server_new_days > 0 {
                self.server_new_days -= 1;
                self.advance_one_day(ctx);
            }
        } else if self.time_of_day >= 24.0 {
            self.time_of_day -= 24.0;
            self.advance_one_day(ctx);
            if ctx.is_server() {
                sync_clock = true;
            }
        }
        if !self.climate.thunder_storm.is_modifying_night() {
            self.ambient = self.time_lerp(self.ambient_min, self.ambient_max, self.dusk as f32, self.dawn as f32);
        }
        self.set_minutes_stamp();
        if self.hour() != previous_hour {
            ctx.game_time_event(GameTimeEvent::Lua("EveryHours"));
        }
        let now = self.minutes();
        if now / 10 != self.minutes_mod {
            // `ErosionMain.EveryTenMinutes()`: the erosion season follows the
            // date (TODO(erosion): the rest of `mainTimer`).
            if !ctx.is_client() {
                self.climate.erosion_every_ten_minutes(self.day, self.month, self.year);
            }
            self.climate.update_every_ten_mins();
            ctx.game_time_event(GameTimeEvent::Lua("EveryTenMinutes"));
            self.minutes_mod = now / 10;
            ctx.game_time_event(GameTimeEvent::RadioUpdateScripts { hour: self.hour(), minutes: now });
        }
        if self.previous_minute_stamp != self.minutes_stamp {
            ctx.game_time_event(GameTimeEvent::Lua("EveryOneMinute"));
            self.previous_minute_stamp = self.minutes_stamp;
        }
        if sync_clock {
            ctx.game_time_event(GameTimeEvent::SyncClock);
            self.last_clock_sync = ms;
        }
    }
}
