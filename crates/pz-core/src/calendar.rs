//! `java.util.GregorianCalendar` arithmetic in the JVM's default time zone
//! (`TimeZone.getDefault()`), for code the original runs through
//! `Calendar`/`GregorianCalendar` (erosion seasons, climate, `GameTime`).
//!
//! Zone rules come from the host's tz database (the `TZ` variable, else
//! the system zone), which is where the JVM takes its default zone from.
//! Dates use the proleptic Gregorian calendar; the original's Julian
//! cutover in 1582 is never reached by game dates.

use std::sync::OnceLock;

use jiff::civil::DateTime;
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::Timestamp;

const DAY_MS: i64 = 86_400_000;

/// A `java.util.TimeZone`.
#[derive(Clone, Debug)]
pub struct JavaZone(TimeZone);

impl JavaZone {
    /// `TimeZone.getDefault()`: read once, as the JVM caches it.
    pub fn default_zone() -> JavaZone {
        static ZONE: OnceLock<TimeZone> = OnceLock::new();
        JavaZone(
            ZONE.get_or_init(|| TimeZone::try_system().unwrap_or(TimeZone::UTC))
                .clone(),
        )
    }

    /// `TimeZone.getTimeZone("UTC")`.
    pub fn utc() -> JavaZone {
        JavaZone(TimeZone::UTC)
    }

    /// A tz database zone by name (`"America/New_York"`).
    pub fn named(name: &str) -> Option<JavaZone> {
        TimeZone::get(name).ok().map(JavaZone)
    }

    /// `getOffset(date)`: the UTC offset in milliseconds at an instant.
    pub fn offset_at(&self, utc_ms: i64) -> i64 {
        match Timestamp::from_millisecond(utc_ms) {
            Ok(ts) => self.0.to_offset(ts).seconds() as i64 * 1000,
            Err(_) => 0,
        }
    }

    /// The tz database's abbreviation for the zone at an instant (`CST`,
    /// `CDT`, or a numeric one such as `+03`).
    pub fn abbreviation_at(&self, utc_ms: i64) -> String {
        match Timestamp::from_millisecond(utc_ms) {
            Ok(ts) => self.0.to_offset_info(ts).abbreviation().to_string(),
            Err(_) => String::new(),
        }
    }

    /// Whether daylight saving time is in effect at an instant.
    pub fn is_dst_at(&self, utc_ms: i64) -> bool {
        match Timestamp::from_millisecond(utc_ms) {
            Ok(ts) => self.0.to_offset_info(ts).dst().is_dst(),
            Err(_) => false,
        }
    }

    /// The offset `GregorianCalendar.computeTime` subtracts from a wall
    /// time (milliseconds since the local epoch). In a spring-forward gap
    /// the offset before the transition applies, so the time moves forward;
    /// in a fall-back overlap the offset after it applies (the later
    /// instant).
    pub fn offset_at_wall(&self, wall_ms: i64) -> i64 {
        let Ok(ts) = Timestamp::from_millisecond(wall_ms) else {
            return 0;
        };
        let dt: DateTime = ts.to_zoned(TimeZone::UTC).datetime();
        let off = match self.0.to_ambiguous_timestamp(dt).offset() {
            AmbiguousOffset::Unambiguous { offset } => offset,
            AmbiguousOffset::Gap { before, .. } => before,
            AmbiguousOffset::Fold { after, .. } => after,
        };
        off.seconds() as i64 * 1000
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (`m` 1..=12).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `(year, month 1..=12, day)` of a day number since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Wall-clock milliseconds since the local epoch of lenient calendar
/// fields (`month` is 0-based and may be out of range, as may the others).
pub fn wall_millis(
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    ms: i32,
) -> i64 {
    let y = year as i64 + (month as i64).div_euclid(12);
    let m = (month as i64).rem_euclid(12) + 1;
    let days = days_from_civil(y, m, 1) + (day as i64 - 1);
    let tod = hour as i64 * 3_600_000 + minute as i64 * 60_000 + second as i64 * 1000 + ms as i64;
    days * DAY_MS + tod
}

/// `new GregorianCalendar(year, month, day, hour, minute, second)` with
/// `MILLISECOND` set, as `getTimeInMillis()`.
#[allow(clippy::too_many_arguments)]
pub fn calendar_millis(
    zone: &JavaZone,
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    ms: i32,
) -> i64 {
    let wall = wall_millis(year, month, day, hour, minute, second, ms);
    wall - zone.offset_at_wall(wall)
}

/// The calendar fields of an instant (`Calendar.get`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalendarFields {
    pub year: i32,
    /// 0-based, as `Calendar.MONTH`.
    pub month: i32,
    pub day_of_month: i32,
    pub hour_of_day: i32,
    pub minute: i32,
    pub second: i32,
    pub millisecond: i32,
    /// `Calendar.SUNDAY` (1) to `Calendar.SATURDAY` (7).
    pub day_of_week: i32,
    /// 1-based.
    pub day_of_year: i32,
    /// `ZONE_OFFSET + DST_OFFSET`, in milliseconds.
    pub offset: i64,
}

/// `setTimeInMillis(utc_ms)` followed by reading the fields.
pub fn calendar_fields(zone: &JavaZone, utc_ms: i64) -> CalendarFields {
    let offset = zone.offset_at(utc_ms);
    let wall = utc_ms + offset;
    let days = wall.div_euclid(DAY_MS);
    let tod = wall.rem_euclid(DAY_MS);
    let (y, m, d) = civil_from_days(days);
    CalendarFields {
        year: y as i32,
        month: m as i32 - 1,
        day_of_month: d as i32,
        hour_of_day: (tod / 3_600_000) as i32,
        minute: (tod / 60_000 % 60) as i32,
        second: (tod / 1000 % 60) as i32,
        millisecond: (tod % 1000) as i32,
        // 1970-01-01 was a Thursday (5).
        day_of_week: ((days + 4).rem_euclid(7) + 1) as i32,
        day_of_year: (days - days_from_civil(y, 1, 1) + 1) as i32,
        offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_like_gregorian_calendar() {
        let ny = JavaZone::named("America/New_York").unwrap();
        // 2021-03-14 02:30 is in the gap: it becomes 03:30 EDT.
        let gap = calendar_millis(&ny, 2021, 2, 14, 2, 30, 0, 0);
        assert_eq!(calendar_fields(&ny, gap).hour_of_day, 3);
        // 2021-11-07 01:30 happens twice: the later (EST) instant wins.
        let fold = calendar_millis(&ny, 2021, 10, 7, 1, 30, 0, 0);
        assert_eq!(calendar_fields(&ny, fold).offset, -5 * 3_600_000);
        let f = calendar_fields(&JavaZone::utc(), 0);
        assert_eq!(
            (
                f.year,
                f.month,
                f.day_of_month,
                f.day_of_week,
                f.day_of_year
            ),
            (1970, 0, 1, 5, 1)
        );
        assert_eq!(
            calendar_millis(&JavaZone::utc(), 1993, 13, 0, 0, 0, 0, 0),
            wall_millis(1994, 1, 0, 0, 0, 0, 0)
        );
    }
}
