//! A `java.util.GregorianCalendar` object over [`crate::calendar`]: the
//! climate code constructs calendars, moves them with `add(DAY_OF_MONTH)` /
//! `add(HOUR_OF_DAY)` and reads `get` and `getTime().getTime()`. Day lengths
//! around daylight-saving changes (23 or 25 hours) feed into
//! `ErosionSeason.dayDiff` and `ClimateValues.pollDate`, so the zone
//! matters.

use crate::calendar::{calendar_fields, calendar_millis, CalendarFields, JavaZone};
use std::cell::RefCell;

pub const YEAR: i32 = 1;
pub const MONTH: i32 = 2;
pub const DAY_OF_MONTH: i32 = 5;
pub const DAY_OF_YEAR: i32 = 6;
pub const DAY_OF_WEEK: i32 = 7;
pub const HOUR_OF_DAY: i32 = 11;
pub const MINUTE: i32 = 12;
pub const SECOND: i32 = 13;
pub const MILLISECOND: i32 = 14;

thread_local! {
    static ZONE_OVERRIDE: RefCell<Option<JavaZone>> = const { RefCell::new(None) };
}

/// The zone new calendars use: [`JavaZone::default_zone`] unless
/// [`set_default_zone`] replaced it on this thread (`TimeZone.setDefault`).
pub fn default_zone() -> JavaZone {
    ZONE_OVERRIDE.with(|z| z.borrow().clone()).unwrap_or_else(JavaZone::default_zone)
}

/// `TimeZone.setDefault(zone)` for this thread (`None` restores the host's).
pub fn set_default_zone(zone: Option<JavaZone>) {
    ZONE_OVERRIDE.with(|z| *z.borrow_mut() = zone);
}

/// A `GregorianCalendar` holding an instant in a zone.
#[derive(Clone, Debug)]
pub struct JavaCalendar {
    time: i64,
    zone: JavaZone,
}

impl PartialEq for JavaCalendar {
    fn eq(&self, o: &Self) -> bool {
        self.time == o.time
    }
}

fn utc_day(y: i32, m: i32, d: i32) -> i64 {
    calendar_millis(&JavaZone::utc(), y, m, d, 0, 0, 0, 0)
}

impl JavaCalendar {
    /// `new GregorianCalendar(year, month, dayOfMonth, hourOfDay, minute)`
    /// in the default zone (lenient).
    pub fn new(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> Self {
        Self::new_in(default_zone(), year, month, day, hour, minute)
    }

    /// `new GregorianCalendar(year, month, dayOfMonth)`.
    pub fn date(year: i32, month: i32, day: i32) -> Self {
        Self::new(year, month, day, 0, 0)
    }

    pub fn new_in(zone: JavaZone, year: i32, month: i32, day: i32, hour: i32, minute: i32) -> Self {
        let time = calendar_millis(&zone, year, month, day, hour, minute, 0, 0);
        JavaCalendar { time, zone }
    }

    /// `getTime().getTime()` / `getTimeInMillis()`.
    pub fn time_in_millis(&self) -> i64 {
        self.time
    }

    fn fields(&self) -> CalendarFields {
        calendar_fields(&self.zone, self.time)
    }

    /// `get(field)` for the fields the climate code reads.
    pub fn get(&self, field: i32) -> i32 {
        let f = self.fields();
        match field {
            YEAR => f.year,
            MONTH => f.month,
            DAY_OF_MONTH => f.day_of_month,
            DAY_OF_YEAR => f.day_of_year,
            DAY_OF_WEEK => f.day_of_week,
            HOUR_OF_DAY => f.hour_of_day,
            MINUTE => f.minute,
            SECOND => f.second,
            MILLISECOND => f.millisecond,
            _ => 0,
        }
    }

    /// `add(field, amount)` for `DAY_OF_MONTH`/`DAY_OF_YEAR` (keeps the
    /// wall-clock time, with `GregorianCalendar.add`'s offset correction)
    /// and `HOUR_OF_DAY`/`MINUTE` (moves the instant).
    pub fn add(&mut self, field: i32, amount: i32) {
        match field {
            HOUR_OF_DAY => self.time += amount as i64 * 3_600_000,
            MINUTE => self.time += amount as i64 * 60_000,
            DAY_OF_MONTH | DAY_OF_YEAR if amount != 0 => {
                let f = self.fields();
                let (ty, tm, td) = (f.year, f.month, f.day_of_month + amount);
                let wall = calendar_millis(&JavaZone::utc(), ty, tm, td, f.hour_of_day, f.minute, f.second, f.millisecond);
                let mut zone_offset = f.offset;
                self.time = wall - zone_offset;
                zone_offset -= self.fields().offset;
                if zone_offset != 0 {
                    self.time += zone_offset;
                    let g = self.fields();
                    if utc_day(g.year, g.month, g.day_of_month) != utc_day(ty, tm, td) {
                        self.time -= zone_offset;
                    }
                }
            }
            _ => {}
        }
    }

    /// `ChronoUnit.MINUTES.between(self.toInstant(), other.toInstant())`.
    pub fn minutes_until(&self, other: &JavaCalendar) -> i64 {
        (other.time - self.time) / 60_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daylight_saving_days() {
        let z = JavaZone::named("America/Chicago").unwrap();
        // 1993: daylight time from 4 April to 31 October.
        let before = JavaCalendar::new_in(z.clone(), 1993, 3, 4, 0, 0);
        let after = JavaCalendar::new_in(z.clone(), 1993, 3, 5, 0, 0);
        assert_eq!(after.time_in_millis() - before.time_in_millis(), 23 * 3_600_000);
        let mut c = before.clone();
        c.add(DAY_OF_MONTH, 1);
        assert_eq!(c, after);
        let mut c = before.clone();
        c.add(HOUR_OF_DAY, 3);
        assert_eq!(c.get(HOUR_OF_DAY), 4);
        let c = JavaCalendar::new_in(z, 1993, 11, 32, 0, 0);
        assert_eq!((c.get(YEAR), c.get(MONTH), c.get(DAY_OF_MONTH)), (1994, 0, 1));
    }
}
