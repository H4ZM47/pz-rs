//! `AlarmClock` and `AlarmClockClothing` (`IAlarmClock`): the alarm's
//! state changes, `update()` and `isDigital()`. Both classes share the
//! logic; they differ in `stopRinging` (only `AlarmClock` plays the
//! "AlarmClockRingingStop" sound) and in what makes a clock digital.
//!
//! What the original does to the world (the cell's process-items list,
//! the repeating world sound, `ItemSoundManager`, network syncs) is
//! returned as [`AlarmEffects`] for the caller to apply.

use crate::item::{AlarmData, InventoryItem, ItemKind};

/// The game state `update()` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlarmTime {
    /// `GameTime.getHour()`.
    pub hour: i32,
    /// `GameTime.getMinutes()`.
    pub minutes: i32,
    /// `GameTime.getWorldAgeHours()`.
    pub world_age_hours: f64,
    /// `GameServer.server || GameClient.client` (minutes round down to
    /// tens).
    pub multiplayer: bool,
    pub server: bool,
    pub client: bool,
}

impl AlarmTime {
    /// From `GameTime.getTimeOfDay()` (`getHour`, `getMinutes`).
    pub fn from_time_of_day(time_of_day: f32, world_age_hours: f64) -> Self {
        AlarmTime {
            hour: ((time_of_day * 3600.0).floor() as f64 / 3600.0).floor() as i32,
            minutes: ((time_of_day - time_of_day as i32 as f32) * 60.0) as i32,
            world_age_hours,
            multiplayer: false,
            server: false,
            client: false,
        }
    }
}

/// What a call asks of the world.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AlarmEffects {
    /// `IsoWorld.instance.currentCell.addToProcessItems(this)` (and the
    /// world item to `getProcessWorldItems()`).
    pub add_to_process_items: bool,
    /// `currentCell.addToProcessItemsRemove(this)`.
    pub remove_from_process_items: bool,
    /// The alarm started ringing (`ringSince` set).
    pub started_ringing: bool,
    /// `stopRinging()` ran: stop the ring sound (and for `AlarmClock`
    /// with a ring sound playing, play "AlarmClockRingingStop" at the
    /// alarm square); `ItemSoundManager.removeItem`.
    pub stopped_ringing: bool,
    /// `WorldSoundManager.addSoundRepeating(null, sq, soundRadius, 3,
    /// false)` at the alarm square.
    pub world_sound: bool,
    /// `ItemSoundManager.addItem(this)`.
    pub item_sound: bool,
    /// `syncAlarmClock()` / `syncStopRinging()` (servers only).
    pub sync: bool,
}

/// The alarm fields of an alarm clock item.
pub fn alarm(it: &InventoryItem) -> Option<&AlarmData> {
    match &it.kind {
        ItemKind::AlarmClock(a) | ItemKind::AlarmClockClothing(_, a) => Some(a),
        _ => None,
    }
}

pub fn alarm_mut(it: &mut InventoryItem) -> Option<&mut AlarmData> {
    match &mut it.kind {
        ItemKind::AlarmClock(a) | ItemKind::AlarmClockClothing(_, a) => Some(a),
        _ => None,
    }
}

/// `isDigital()`: an `AlarmClock` whose display name (the constructor's
/// `name`) contains "Classic" is not; an `AlarmClockClothing` whose full
/// type does.
pub fn is_digital(it: &InventoryItem, display_name: &str) -> bool {
    match &it.kind {
        ItemKind::AlarmClock(_) => !display_name.contains("Classic"),
        ItemKind::AlarmClockClothing(..) => !it.get_full_type().contains("Classic"),
        _ => false,
    }
}

impl AlarmData {
    /// `isRinging()`.
    pub fn is_ringing(&self) -> bool {
        self.ring_since >= 0.0
    }

    /// `stopRinging()` (`GameTime.getMinutes()` is `minutes`).
    pub fn stop_ringing(&mut self, minutes: i32) -> AlarmEffects {
        self.ring_since = -1.0;
        self.force_dont_ring = minutes / 10 * 10;
        AlarmEffects {
            stopped_ringing: true,
            ..Default::default()
        }
    }

    /// `setAlarmSet(set)`: stops ringing, then sets the alarm and adds
    /// the clock to (or removes it from) the cell's process list.
    pub fn set_alarm_set(&mut self, set: bool, minutes: i32) -> AlarmEffects {
        let mut e = self.stop_ringing(minutes);
        self.alarm_set = set;
        if set {
            e.add_to_process_items = true;
        } else {
            e.remove_from_process_items = true;
        }
        e
    }

    /// `setHour(hour)`.
    pub fn set_hour(&mut self, hour: i32) {
        self.alarm_hour = hour;
        self.force_dont_ring = -1;
    }

    /// `setMinute(min)`.
    pub fn set_minute(&mut self, min: i32) {
        self.alarm_minutes = min;
        self.force_dont_ring = -1;
    }

    /// `setForceDontRing(min)`.
    pub fn set_force_dont_ring(&mut self, min: i32) {
        self.force_dont_ring = min;
    }

    /// `shouldUpdateInWorld()` / `!finishupdate()`.
    pub fn should_update(&self) -> bool {
        self.alarm_set
    }

    /// `update()`. `has_square` is whether `getAlarmSquare()` found a
    /// square (the outermost container's square, its parent's or the
    /// world item's).
    pub fn update(&mut self, now: &AlarmTime, has_square: bool) -> AlarmEffects {
        let mut e = AlarmEffects::default();
        if !self.alarm_set {
            return e;
        }
        let mut minutes = now.minutes;
        if now.multiplayer {
            minutes = minutes / 10 * 10;
        }
        if !self.is_ringing()
            && self.force_dont_ring != minutes
            && self.alarm_hour == now.hour
            && self.alarm_minutes == minutes
        {
            self.ring_since = now.world_age_hours;
            e.started_ringing = true;
            e.sync |= now.server;
        }
        if self.is_ringing() {
            // GameTime.minHours.
            self.ring_since = self.ring_since.min(now.world_age_hours);
            if !has_square || self.ring_since + 0.5 < now.world_age_hours {
                // stopRinging() reads the unrounded minutes.
                let s = self.stop_ringing(now.minutes);
                e.stopped_ringing = s.stopped_ringing;
                e.sync |= now.server;
            } else if !now.client {
                e.world_sound = true;
            }
            if !now.server && self.is_ringing() {
                e.item_sound = true;
            }
        }
        if self.force_dont_ring != minutes {
            self.force_dont_ring = -1;
        }
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: i32, minutes: i32, age: f64) -> AlarmTime {
        AlarmTime {
            hour,
            minutes,
            world_age_hours: age,
            multiplayer: false,
            server: false,
            client: false,
        }
    }

    #[test]
    fn rings_at_the_alarm_minute_and_stops() {
        let mut a = AlarmData::default();
        a.set_hour(7);
        a.set_minute(30);
        let e = a.set_alarm_set(true, 12);
        assert!(e.add_to_process_items);
        assert_eq!(a.force_dont_ring, 10);
        // A different minute: no ring, and forceDontRing clears.
        a.update(&at(7, 29, 100.0), true);
        assert!(!a.is_ringing());
        assert_eq!(a.force_dont_ring, -1);
        let e = a.update(&at(7, 30, 100.5), true);
        assert!(e.started_ringing && e.world_sound && e.item_sound);
        assert_eq!(a.ring_since, 100.5);
        // Half an hour later it stops, and the minute is blocked.
        let e = a.update(&at(8, 1, 101.01), true);
        assert!(e.stopped_ringing);
        assert!(!a.is_ringing());
        assert_eq!(a.force_dont_ring, -1);
        // Without a square it stops at once.
        a.set_hour(8);
        a.set_minute(10);
        let e = a.update(&at(8, 10, 102.0), false);
        assert!(e.started_ringing && e.stopped_ringing && !e.item_sound);
        assert_eq!(a.force_dont_ring, 10);
        let e = a.update(&at(8, 10, 102.01), false);
        assert!(!e.started_ringing);
    }

    #[test]
    fn defaults_are_the_field_initializers() {
        let a = AlarmData::default();
        assert_eq!((a.alarm_hour, a.alarm_minutes), (-1, -1));
        assert!(!a.is_ringing());
        assert_eq!(a.force_dont_ring, -1);
    }
}
