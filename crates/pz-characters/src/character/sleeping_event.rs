//! `SleepingEvent` and `SleepingEventData`: the character's part of
//! falling asleep, of the checks while asleep, and of waking up.
//!
//! The callers own the world reads: `ClimateManager.isRaining() &&
//! isExposedToPrecipitation(chr)` comes in as `rain_exposed`. The UI and
//! game part of `wakeUp` (the fade-in, game speed 1 and `GameWindow.save`
//! with all players asleep) goes out as [`SimEvent::SleepingEventWakeUp`].

use pz_scripts::{ScriptError, ScriptManager, ScriptResult};

use super::context::{SimContext, SimEvent};
use super::GameCharacter;
use crate::Characters;

/// `SleepingEventData` (`IsoGameCharacter.sleepingEventData`, not saved).
/// `openDoor` and `weakestWindow` are only set by the zombie intruder
/// check, which is not ported.
#[derive(Clone, Debug, PartialEq)]
pub struct SleepingEventData {
    /// `forceWakeUpTime`: the hour of sleep when intruders wake the player.
    pub force_wake_up_time: i32,
    /// `zombiesIntruders`.
    pub zombies_intruders: bool,
    /// `nightmareWakeUp`: the hour of sleep when a nightmare wakes the
    /// player.
    pub nightmare_wake_up: i32,
    /// `raining`.
    pub raining: bool,
    /// `wasRainingAtStart`.
    pub was_raining_at_start: bool,
    /// `rainTimeStartHours`.
    pub rain_time_start_hours: f64,
    /// `sleepingTime`: the hours the player meant to sleep.
    pub sleeping_time: f32,
    /// `fastWakeup`.
    pub fast_wakeup: bool,
}

impl Default for SleepingEventData {
    /// `new SleepingEventData()`.
    fn default() -> Self {
        Self {
            force_wake_up_time: -1,
            zombies_intruders: true,
            nightmare_wake_up: -1,
            raining: false,
            was_raining_at_start: false,
            rain_time_start_hours: -1.0,
            sleeping_time: 8.0,
            fast_wakeup: false,
        }
    }
}

impl SleepingEventData {
    /// `reset()`.
    pub fn reset(&mut self) {
        *self = Self { zombies_intruders: false, ..Self::default() };
    }

    /// `getHoursSinceRainStarted()`.
    pub fn hours_since_rain_started(&self, ctx: &dyn SimContext) -> f64 {
        ctx.world_age_hours() - self.rain_time_start_hours
    }
}

/// What `SleepingEvent.update` asks the caller to do after the character
/// part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepingUpdate {
    /// The player sleeps on.
    Asleep,
    /// The player woke up.
    WokeUp,
    /// `spawnZombieIntruders(player)` then a wake-up: not ported.
    ZombieIntruders,
}

impl<I: 'static> GameCharacter<I> {
    /// `getOrCreateSleepingEventData()`.
    pub fn sleeping_event_data(&mut self) -> &mut SleepingEventData {
        self.sleeping_event_data.get_or_insert_with(SleepingEventData::default)
    }

    /// `SleepingEvent.setPlayerFallAsleep(player, sleepFor, forceIntruders,
    /// forceNightmare)`. `rain_exposed` is `ClimateManager.isRaining() &&
    /// isExposedToPrecipitation(player)`. Answers whether the zombie
    /// intruder check runs next: `SleepingEvent.zombiesInvasion` is never
    /// set in 42.21, so only `forceIntruders` reaches it.
    #[allow(clippy::too_many_arguments)]
    pub fn set_player_fall_asleep(
        &mut self,
        sleep_for: i32,
        force_intruders: bool,
        force_nightmare: bool,
        rain_exposed: bool,
        sm: &ScriptManager,
        ctx: &dyn SimContext,
    ) -> ScriptResult<bool> {
        self.sleeping_event_data().reset();
        if rain_exposed {
            let world_age = ctx.world_age_hours();
            let data = self.sleeping_event_data();
            data.raining = true;
            data.was_raining_at_start = true;
            data.rain_time_start_hours = world_age;
        }
        self.sleeping_event_data().sleeping_time = sleep_for as f32;
        self.time_of_sleep = ctx.time_of_day();
        self.do_delay_to_sleep(sm, ctx)?;
        self.check_nightmare(sleep_for, sm, ctx)?;
        if force_nightmare {
            ctx.rand_next_range(3, sleep_for - 2);
        }
        if self.sleeping_event_data().nightmare_wake_up > -1 {
            return Ok(false);
        }
        Ok(force_intruders)
    }

    /// `SleepingEvent.doDelayToSleep(player)`.
    fn do_delay_to_sleep(&mut self, sm: &ScriptManager, ctx: &dyn SimContext) -> ScriptResult {
        let mut f = 0.3_f32;
        if self.has_trait(sm, "INSOMNIAC")? {
            f = 1.0;
        }
        let pain = self.moodles.moodle_level_of(sm, "PAIN")?;
        if pain > 0 {
            f += 1.0 + pain as f32 * 0.2;
        }
        if self.moodles.moodle_level_of(sm, "STRESS")? > 0 {
            f *= 1.2;
        }
        let bed = self.bed_type.as_str();
        if bed.contains("averageBedPillow") {
            f *= 1.0;
        }
        if bed == "badBed" {
            f *= 1.3;
        } else if bed.contains("badBedPillow") {
            f *= 1.25;
        } else if bed == "goodBed" {
            f *= 0.8;
        } else if bed.contains("goodBedPillow") {
            f *= 0.6;
        } else if bed == "floor" {
            f *= 1.6;
        } else if bed == "floorPillow" {
            f *= 1.45;
        }
        if self.has_trait(sm, "NIGHT_OWL")? {
            f *= 0.5;
        }
        if self.sleeping_tablet_effect > 1000.0 {
            f = 0.1;
        }
        if f > 2.0 {
            f = 2.0;
        }
        let delay = ctx.rand_next_float(0.0, f);
        self.delay_to_actually_sleep = ctx.time_of_day() + delay;
        Ok(())
    }

    /// `SleepingEvent.checkNightmare(player, sleepFor)`.
    fn check_nightmare(&mut self, sleep_for: i32, sm: &ScriptManager, ctx: &dyn SimContext) -> ScriptResult {
        if ctx.is_client() {
            return Ok(());
        }
        if sleep_for >= 3 {
            let mut chance = 5;
            if self.has_trait(sm, "DESENSITIZED")? {
                chance += 5;
            }
            chance += self.moodles.moodle_level_of(sm, "STRESS")? * 10;
            if ctx.rand_next(100) < chance {
                self.sleeping_event_data().nightmare_wake_up = ctx.rand_next_range(3, sleep_for - 2);
            }
        }
        Ok(())
    }

    /// `SleepingEvent.update(player)` for an asleep player, after
    /// `GameTime.update` added the passing time to `asleepTime`.
    /// `rain_exposed` is `ClimateManager.isRaining() &&
    /// isExposedToPrecipitation(player)`, read only while it rains.
    pub fn sleeping_event_update(
        &mut self,
        rain_exposed: impl FnOnce() -> ScriptResult<bool>,
        is_raining: bool,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult<SleepingUpdate> {
        let asleep_hour = self.asleep_time as i32;
        if self.stats.num_very_close_zombies() > 0 {
            self.wake_up_alarm("WakeZombies", chars, ctx)?;
            return Ok(SleepingUpdate::WokeUp);
        }
        if self.sleeping_event_data().nightmare_wake_up == asleep_hour {
            self.wake_up_alarm("WakeNightmare", chars, ctx)?;
            return Ok(SleepingUpdate::WokeUp);
        }
        let data = self.sleeping_event_data();
        if data.force_wake_up_time == asleep_hour && data.zombies_intruders {
            return Ok(SleepingUpdate::ZombieIntruders);
        }
        // updateRain.
        if !is_raining {
            let data = self.sleeping_event_data();
            data.raining = false;
            data.was_raining_at_start = false;
            data.rain_time_start_hours = -1.0;
        } else if rain_exposed()? {
            let world_age = ctx.world_age_hours();
            let data = self.sleeping_event_data();
            if !data.was_raining_at_start {
                if !data.raining {
                    data.rain_time_start_hours = world_age;
                }
                // `getHoursSinceRainStarted() < 1/6` jumps to the next
                // statement either way.
            }
            data.raining = true;
        }
        // updateSnow, updateTemperature and updateWetness change nothing.
        Ok(SleepingUpdate::Asleep)
    }

    /// The zombie and nightmare wake-ups of `SleepingEvent.update`: panic
    /// and stress, `WorldSoundManager.addSound(player, x, y, z, 6, 1)`,
    /// `setMusicWakeState`, `fastWakeup` and `wakeUp`.
    fn wake_up_alarm(&mut self, _music: &'static str, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        self.stats.add(&Self::stat_ref_pub(chars, "PANIC")?, 70.0);
        self.stats.add(&Self::stat_ref_pub(chars, "STRESS")?, 0.5);
        ctx.event(SimEvent::AddSound { radius: 6, volume: 1 });
        // SoundManager.setMusicWakeState: TODO(sound).
        self.sleeping_event_data().fast_wakeup = true;
        self.sleeping_event_wake_up(chars, ctx)
    }

    /// `IsoPlayer.processWakingUp()`, while asleep.
    pub fn process_waking_up(&mut self, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        if self.force_wake_up_time == 0.0 {
            self.force_wake_up_time = 9.0;
        }
        let mut time = ctx.time_of_day();
        let mut last = ctx.last_time_of_day();
        if last > time {
            if last < self.force_wake_up_time {
                time += 24.0;
            } else {
                last -= 24.0;
            }
        }
        let mut wake = time >= self.force_wake_up_time && last < self.force_wake_up_time;
        if self.asleep_time > 16.0 {
            wake = true;
        }
        if self.is_local_player(ctx) && (ctx.is_client() || ctx.num_players() > 1) && !wake {
            return Err(ScriptError::new(
                "IsoPlayer.processWakingUp: waking up on pressedAim or pressedMovement with more than one player is not ported",
            ));
        }
        if self.force_wake_up {
            wake = true;
        }
        if self.asleep && wake {
            self.force_wake_up = false;
            // SoundManager.setMusicWakeState(player, "WakeNormal"): TODO(sound).
            self.sleeping_event_wake_up(chars, ctx)?;
            self.force_wake_up_time = -1.0;
            // GameClient.sendPlayer: TODO(network).
            self.player.dirty_recalc_grid_stack_time = 20.0;
        }
        Ok(())
    }

    /// `SleepingEvent.wakeUp(chr)` (`remote = chr.isRemote()`, false for
    /// a local character). The fade-in, game speed and save go out as
    /// [`SimEvent::SleepingEventWakeUp`] before the character wakes.
    pub fn sleeping_event_wake_up(&mut self, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let fast_wakeup = self.sleeping_event_data().fast_wakeup;
        // GameClient.client: the WakeUpPlayer packet, TODO(network).
        ctx.event(SimEvent::SleepingEventWakeUp { fast_wakeup });
        if self.is_player() {
            self.last_hour_sleeped = self.hours_survived as i32;
        }
        self.force_wake_up_time = -1.0;
        self.asleep = false;
        let sleeping_time = self.sleeping_event_data().sleeping_time;
        let f7 = sleeping_time / 8.0;
        let fatigue = Self::stat_ref_pub(chars, "FATIGUE")?;
        let neck = super::player::attack::part::NECK;
        let neck_hurt = |chr: &mut Self, chance: i32, damage: (f32, f32), pain: (f32, f32)| -> ScriptResult {
            if ctx.rand_next(chance) == 0 {
                let d = ctx.rand_next_float(damage.0, damage.1);
                let part = chr.body_damage.body_part_mut(neck).ok_or_else(|| ScriptError::new("BodyPartType.Neck"))?;
                part.add_damage(d);
                let p = ctx.rand_next_float(pain.0, pain.1);
                part.additional_pain += p;
            }
            Ok(())
        };
        match self.bed_type.as_str() {
            "goodBed" | "goodBedPillow" => {
                self.stats.remove(&fatigue, ctx.rand_next_float(0.05, 0.12) * f7);
            }
            "badBed" => {
                self.stats.add(&fatigue, ctx.rand_next_float(0.1, 0.2) * f7);
                neck_hurt(self, 5, (5.0, 15.0), (30.0, 50.0))?;
            }
            "badBedPillow" => {
                self.stats.add(&fatigue, ctx.rand_next_float(0.1, 0.2) * f7);
                neck_hurt(self, 10, (2.5, 7.5), (15.0, 25.0))?;
            }
            "floor" => {
                self.stats.add(&fatigue, ctx.rand_next_float(0.15, 0.25) * f7);
                neck_hurt(self, 5, (10.0, 20.0), (30.0, 50.0))?;
            }
            "floorPillow" => {
                self.stats.add(&fatigue, ctx.rand_next_float(0.15, 0.25) * f7);
                neck_hurt(self, 10, (5.0, 10.0), (15.0, 25.0))?;
            }
            "averageBedPillow" => neck_hurt(self, 20, (1.5, 6.0), (5.0, 15.0))?,
            _ => neck_hurt(self, 10, (3.0, 12.0), (10.0, 30.0))?,
        }
        self.sleeping_event_data().reset();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    use super::*;
    use crate::character::thermoregulator::body_tests::body_fixture;
    use crate::character::tests::{player, Recorder};
    use crate::character::ZomboidGlobals;

    /// Scripted randoms: `Rand.Next(n)` answers from `ints` (then
    /// `n - 1`), a range its minimum, a float `float` (the bounds are
    /// kept); a settable clock.
    struct Ctx {
        rec: Recorder,
        ints: RefCell<VecDeque<i32>>,
        float: Cell<f32>,
        float_bounds: RefCell<Vec<(f32, f32)>>,
        time: Cell<(f32, f32)>,
    }

    impl Default for Ctx {
        fn default() -> Self {
            Self {
                rec: Recorder::default(),
                ints: RefCell::default(),
                float: Cell::new(0.0),
                float_bounds: RefCell::default(),
                time: Cell::new((9.0, 9.0)),
            }
        }
    }

    impl SimContext for Ctx {
        fn zomboid_globals(&self) -> &ZomboidGlobals {
            &self.rec.globals
        }
        fn event(&self, e: SimEvent) {
            self.rec.event(e)
        }
        fn rand_next(&self, max: i32) -> i32 {
            self.ints.borrow_mut().pop_front().unwrap_or((max - 1).max(0))
        }
        fn rand_next_range(&self, min: i32, _max: i32) -> i32 {
            min
        }
        fn rand_next_float(&self, min: f32, max: f32) -> f32 {
            self.float_bounds.borrow_mut().push((min, max));
            self.float.get()
        }
        fn time_of_day(&self) -> f32 {
            self.time.get().0
        }
        fn last_time_of_day(&self) -> f32 {
            self.time.get().1
        }
    }

    fn events(ctx: &Ctx) -> Vec<String> {
        ctx.rec.events.borrow().clone()
    }

    #[test]
    fn falling_asleep_on_the_floor_sets_the_delay_and_the_nightmare() {
        let (sm, mut chars) = body_fixture();
        let mut c = player(&sm, &mut chars);
        c.bed_type = "floor".into();
        let ctx = Ctx::default();
        ctx.time.set((22.0, 21.9));
        ctx.float.set(0.25);
        assert_eq!(c.set_player_fall_asleep(7, false, false, false, &sm, &ctx), Ok(false));
        assert_eq!((c.time_of_sleep, c.delay_to_actually_sleep), (22.0, 22.25));
        // 0.3, times 1.6 on the floor.
        assert_eq!(*ctx.float_bounds.borrow(), [(0.0, 0.3 * 1.6)]);
        let data = c.sleeping_event_data.clone().unwrap();
        assert_eq!((data.sleeping_time, data.nightmare_wake_up, data.zombies_intruders), (7.0, -1, false));
        assert!(!data.raining);

        // Rand.Next(100) = 4 < 5: a nightmare at Rand.Next(3, 5).
        ctx.ints.borrow_mut().push_back(4);
        assert_eq!(c.set_player_fall_asleep(7, true, false, true, &sm, &ctx), Ok(false));
        let data = c.sleeping_event_data.clone().unwrap();
        assert_eq!(data.nightmare_wake_up, 3);
        assert!(data.raining && data.was_raining_at_start);

        // Without a nightmare, forceIntruders reaches the intruder check.
        assert_eq!(c.set_player_fall_asleep(7, true, false, false, &sm, &ctx), Ok(true));
        // Short sleeps have no nightmare roll.
        assert_eq!(c.set_player_fall_asleep(2, false, false, false, &sm, &ctx), Ok(false));
        assert!(ctx.ints.borrow().is_empty());
    }

    #[test]
    fn waking_up_from_the_floor_tires_and_may_hurt_the_neck() {
        let (sm, mut chars) = body_fixture();
        let mut c = player(&sm, &mut chars);
        let neck = crate::character::player::attack::part::NECK;
        let ctx = Ctx::default();
        c.bed_type = "floor".into();
        c.asleep = true;
        c.hours_survived = 5.7;
        c.force_wake_up_time = 6.0;
        c.sleeping_event_data().sleeping_time = 4.0;
        let health = c.body_damage.body_part(neck).unwrap().health();
        ctx.float.set(0.2);
        // Rand.Next(5) == 0: the neck hurts.
        ctx.ints.borrow_mut().push_back(0);
        c.sleeping_event_wake_up(&chars, &ctx).unwrap();
        assert_eq!(events(&ctx), [format!("{:?}", SimEvent::SleepingEventWakeUp { fast_wakeup: false })]);
        assert!(!c.asleep);
        assert_eq!((c.last_hour_sleeped, c.force_wake_up_time), (5, -1.0));
        let fatigue = GameCharacter::<()>::stat_ref_pub(&chars, "FATIGUE").unwrap();
        // Rand(0.15, 0.25) * sleepingTime / 8.
        assert_eq!(c.stats.get(&fatigue), 0.2 * 0.5);
        assert_eq!(ctx.float_bounds.borrow()[..], [(0.15, 0.25), (10.0, 20.0), (30.0, 50.0)]);
        let part = c.body_damage.body_part(neck).unwrap();
        assert!(part.health() < health);
        assert_eq!(part.additional_pain, 0.2);
        let mut reset = SleepingEventData::default();
        reset.reset();
        assert_eq!(c.sleeping_event_data, Some(reset));
    }

    #[test]
    fn process_waking_up_waits_for_the_hour_across_midnight() {
        let (sm, mut chars) = body_fixture();
        let mut c = player(&sm, &mut chars);
        let ctx = Ctx::default();
        c.asleep = true;
        c.force_wake_up_time = 6.0;
        for time in [(23.5, 23.4), (0.1, 23.9), (5.9, 5.8)] {
            ctx.time.set(time);
            c.process_waking_up(&chars, &ctx).unwrap();
            assert!(c.asleep, "{time:?}");
        }
        ctx.time.set((6.01, 5.99));
        c.process_waking_up(&chars, &ctx).unwrap();
        assert!(!c.asleep);
        assert_eq!((c.force_wake_up_time, c.player.dirty_recalc_grid_stack_time), (-1.0, 20.0));

        // A wake-up hour of 0 is 9; more than 16 hours asleep always wakes.
        c.asleep = true;
        c.force_wake_up_time = 0.0;
        ctx.time.set((8.0, 7.9));
        c.process_waking_up(&chars, &ctx).unwrap();
        assert_eq!((c.asleep, c.force_wake_up_time), (true, 9.0));
        c.asleep_time = 16.5;
        c.process_waking_up(&chars, &ctx).unwrap();
        assert!(!c.asleep);
    }

    #[test]
    fn very_close_zombies_wake_the_sleeper_fast() {
        let (sm, mut chars) = body_fixture();
        let mut c = player(&sm, &mut chars);
        let ctx = Ctx::default();
        c.asleep = true;
        let r = c.sleeping_event_update(|| unreachable!("dry"), false, &chars, &ctx).unwrap();
        assert_eq!(r, SleepingUpdate::Asleep);
        assert!(c.asleep);
        c.stats.last_very_close_zombies = 1;
        let r = c.sleeping_event_update(|| unreachable!("dry"), false, &chars, &ctx).unwrap();
        assert_eq!(r, SleepingUpdate::WokeUp);
        assert!(!c.asleep);
        assert_eq!(
            events(&ctx),
            [
                format!("{:?}", SimEvent::AddSound { radius: 6, volume: 1 }),
                format!("{:?}", SimEvent::SleepingEventWakeUp { fast_wakeup: true }),
            ]
        );
        let panic = GameCharacter::<()>::stat_ref_pub(&chars, "PANIC").unwrap();
        assert!(c.stats.get(&panic) > 0.0);
    }

    #[test]
    fn rain_while_asleep_is_kept_only_when_exposed() {
        let (sm, mut chars) = body_fixture();
        let mut c = player(&sm, &mut chars);
        let ctx = Ctx::default();
        c.asleep = true;
        c.sleeping_event_update(|| Ok(true), true, &chars, &ctx).unwrap();
        let data = c.sleeping_event_data.clone().unwrap();
        assert!(data.raining && !data.was_raining_at_start);
        c.sleeping_event_update(|| unreachable!("dry"), false, &chars, &ctx).unwrap();
        let data = c.sleeping_event_data.clone().unwrap();
        assert!(!data.raining);
        assert_eq!(data.rain_time_start_hours, -1.0);
    }
}
