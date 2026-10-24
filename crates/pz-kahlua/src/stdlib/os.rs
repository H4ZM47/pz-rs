//! `OsLib`: `os.date`, `os.difftime`, `os.time`.
//!
//! The original's calendar (`OsLib.tzone`) is UTC, so `os.date` without `!`
//! formats UTC too; only `%c` (Java `Date.toString()`) uses the local time
//! zone. Calendars are lenient `GregorianCalendar`s: Julian before
//! 1582-10-15, out-of-range table fields roll over.
//!
//! Original quirks preserved: `%j`/`yday` count from 0 on January 1st, `%I`
//! is 00-11, `%e` pads "05" with a space (" 05"), `%y` is not zero-padded,
//! `%Z` is "UTC", an unknown conversion inserts "null", the `*t` table has
//! a `milli` field and no `isdst`, and `os.time()` keeps milliseconds.

use super::{arg, java_class_name, number_arg, reg_named, string_arg, table_cast};
use crate::table::TableRef;
use crate::value::Value;
use crate::vm::{LuaError, LuaResult, NativeCall};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn register(env: &TableRef) {
    let os = TableRef::new();
    // OsLib has no toString(): Java prints class@identityHash.
    let name = |i: usize| format!("se.krka.kahlua.stdlib.OsLib@{:x}", pseudo_hash(&os, i));
    reg_named(&os, "date", &name(0), date);
    reg_named(&os, "difftime", &name(1), difftime);
    reg_named(&os, "time", &name(2), time);
    env.set_str("os", os);
}

fn pseudo_hash(t: &TableRef, i: usize) -> u32 {
    let p = (t.ptr_id() as u64).wrapping_add(i as u64 * 0x9E37_79B9);
    (((p >> 4) ^ (p >> 20)) as u32 & 0x7FFF_FFFF) | 0x1000_0000
}

const MS_PER_DAY: i64 = 86_400_000;
const MS_PER_WEEK: i64 = 604_800_000;
const SHORT_DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const LONG_DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const SHORT_MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const LONG_MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];

fn now_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

// ---------------------------------------------------------------------------
// Natives
// ---------------------------------------------------------------------------

fn time(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    if c.nargs() == 0 {
        return Ok(c.push(now_millis() as f64 * 0.001));
    }
    let t = table_cast(&arg(c, 1, "time")?)?;
    let ms = date_from_table(&t)?;
    Ok(c.push(ms as f64 * 0.001))
}

fn difftime(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let t2 = number_arg(c, 1, "difftime")?;
    let t1 = number_arg(c, 2, "difftime")?;
    Ok(c.push(t2 - t1))
}

fn date(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    if c.nargs() == 0 {
        return Ok(c.push(getdate("%c", now_millis())?));
    }
    let format = string_arg(c, 1, "date")?;
    if c.nargs() == 1 {
        return Ok(c.push(getdate(&format, now_millis())?));
    }
    let t = number_arg(c, 2, "date")?;
    let time = (t * 1000.0) as i64;
    Ok(c.push(getdate(&format, time)?))
}

/// `OsLib.getdate(format, time)`.
fn getdate(format: &str, time: i64) -> LuaResult<Value> {
    let f: Vec<u16> = format.encode_utf16().collect();
    let Some(&first) = f.first() else {
        return Err(LuaError::Runtime {
            message: "Index 0 out of bounds for length 0".into(),
            class: "java.lang.StringIndexOutOfBoundsException",
        });
    };
    // Both branches use a UTC calendar ('!' selects UTC explicitly; the
    // default `tzone` is UTC as well).
    let si = usize::from(first == u16::from(b'!'));
    let rest = &f[si..];
    if rest.starts_with(&[u16::from(b'*'), u16::from(b't')]) {
        return Ok(Value::Table(table_from_date(time)));
    }
    Ok(Value::from(format_time(rest, time)))
}

// ---------------------------------------------------------------------------
// Calendar arithmetic (lenient GregorianCalendar in UTC)
// ---------------------------------------------------------------------------

/// Days since the epoch of 1582-10-15, the first Gregorian day.
const CUTOVER_DAY: i64 = -141_427;

fn gregorian_days(y: i64, m: i64, d: i64) -> i64 {
    // Howard Hinnant's days_from_civil, linear in `d`; `m` is 1..=12.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn gregorian_civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn julian_raw(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(4);
    let yoe = y - era * 4;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    era * 1461 + yoe * 365 + doy
}

/// Julian 1582-10-04 is the day before the cutover.
fn julian_days(y: i64, m: i64, d: i64) -> i64 {
    julian_raw(y, m, d) - julian_raw(1582, 10, 4) + CUTOVER_DAY - 1
}

fn julian_civil(days: i64) -> (i64, i64, i64) {
    let z = days - (CUTOVER_DAY - 1) + julian_raw(1582, 10, 4);
    let era = z.div_euclid(1461);
    let doe = z - era * 1461;
    let yoe = (doe / 365).min(3);
    let doy = doe - 365 * yoe;
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 4 + i64::from(m <= 2), m, d)
}

/// `GregorianCalendar.computeTime` from YEAR (proleptic, AD era), MONTH
/// (0-based), DAY_OF_MONTH, HOUR_OF_DAY, MINUTE, SECOND, MILLISECOND.
fn ms_from_fields(year: i64, month0: i64, day: i64, hour: i64, min: i64, sec: i64, milli: i64) -> i64 {
    let tod = ((hour * 60 + min) * 60 + sec) * 1000 + milli;
    let extra_days = tod.div_euclid(MS_PER_DAY);
    let tod = tod.rem_euclid(MS_PER_DAY);
    let y = year + month0.div_euclid(12);
    let m = month0.rem_euclid(12) + 1;
    let gfd = gregorian_days(y, m, 1) + day - 1 + extra_days;
    let jfd = julian_days(y, m, 1) + day - 1 + extra_days;
    let fd = if year >= 1582 && gfd >= CUTOVER_DAY { gfd } else { jfd };
    fd * MS_PER_DAY + tod
}

/// Calendar fields of an instant.
#[derive(Clone, Copy, Debug)]
struct Fields {
    /// Proleptic year (0 = 1 BC).
    year: i64,
    month0: i64,
    day: i64,
    hour: i64,
    min: i64,
    sec: i64,
    milli: i64,
    /// DAY_OF_WEEK: 1 = Sunday .. 7 = Saturday.
    dow: i64,
}

impl Fields {
    fn of(ms: i64) -> Fields {
        let days = ms.div_euclid(MS_PER_DAY);
        let tod = ms.rem_euclid(MS_PER_DAY);
        let (year, m, day) = if days >= CUTOVER_DAY { gregorian_civil(days) } else { julian_civil(days) };
        Fields {
            year,
            month0: m - 1,
            day,
            hour: tod / 3_600_000,
            min: tod / 60_000 % 60,
            sec: tod / 1000 % 60,
            milli: tod % 1000,
            dow: (days + 4).rem_euclid(7) + 1,
        }
    }
    /// The YEAR field (year of era).
    fn year_of_era(&self) -> i64 {
        if self.year <= 0 {
            1 - self.year
        } else {
            self.year
        }
    }
    /// The same instant moved to January `day` (`c2.set(MONTH, 0); c2.set(DAY_OF_MONTH, day)`).
    fn in_january(&self, day: i64) -> i64 {
        ms_from_fields(self.year, 0, day, self.hour, self.min, self.sec, self.milli)
    }
}

/// `OsLib.getDayOfYear` (0 on January 1st).
fn day_of_year(ms: i64) -> i64 {
    let diff = ms - Fields::of(ms).in_january(1);
    (diff as f64 / 8.64e7).ceil() as i32 as i64
}

/// `OsLib.getWeekOfYear`, including its off-by-one first-week rules.
fn week_of_year(ms: i64, week_starts_sunday: bool, jan1_midweek: bool) -> i64 {
    let f = Fields::of(ms);
    let dow = Fields::of(f.in_january(1)).dow;
    let day = if week_starts_sunday && dow != 1 {
        7 - dow + 1
    } else if dow != 2 {
        7 - dow + 1 + 1
    } else {
        1
    };
    let diff = ms - f.in_january(day);
    let mut w = diff / MS_PER_WEEK;
    if jan1_midweek && 7 - dow >= 4 {
        w += 1;
    }
    w
}

fn table_from_date(ms: i64) -> TableRef {
    let f = Fields::of(ms);
    let t = TableRef::new();
    t.set_str("year", f.year_of_era() as f64);
    t.set_str("month", (f.month0 + 1) as f64);
    t.set_str("day", f.day as f64);
    t.set_str("hour", f.hour as f64);
    t.set_str("min", f.min as f64);
    t.set_str("sec", f.sec as f64);
    t.set_str("wday", f.dow as f64);
    t.set_str("yday", day_of_year(ms) as f64);
    t.set_str("milli", f.milli as f64);
    t
}

/// `KahluaUtil.fromDouble`: `(Double) o` unboxed.
fn from_double(v: &Value) -> LuaResult<f64> {
    match v {
        Value::Num(n) => Ok(*n),
        Value::Nil => Err(LuaError::Runtime {
            message: "Cannot invoke \"java.lang.Double.doubleValue()\" because \"o\" is null".into(),
            class: "java.lang.NullPointerException",
        }),
        other => Err(LuaError::Runtime {
            message: format!("class {} cannot be cast to class java.lang.Double", java_class_name(other)),
            class: "java.lang.ClassCastException",
        }),
    }
}

/// `OsLib.getDateFromTable`, as epoch milliseconds.
fn date_from_table(t: &TableRef) -> LuaResult<i64> {
    let year = from_double(&t.get_str("year"))? as i32;
    let month0 = (from_double(&t.get_str("month"))? as i32).wrapping_sub(1);
    let day = from_double(&t.get_str("day"))? as i32;
    let opt = |key: &str| -> LuaResult<i64> {
        let v = t.get_str(key);
        Ok(if v.is_nil() { 0 } else { from_double(&v)? as i32 as i64 })
    };
    let hour = opt("hour")?;
    let min = opt("min")?;
    let sec = opt("sec")?;
    let milli = opt("milli")?;
    Ok(ms_from_fields(year as i64, month0 as i64, day as i64, hour, min, sec, milli))
}

// ---------------------------------------------------------------------------
// strftime
// ---------------------------------------------------------------------------

/// `OsLib.formatTime`.
fn format_time(format: &[u16], ms: i64) -> String {
    let mut out: Vec<u16> = Vec::with_capacity(format.len() + 16);
    let mut i = 0;
    while i < format.len() {
        if format[i] != u16::from(b'%') || i + 1 == format.len() {
            out.push(format[i]);
        } else {
            i += 1;
            out.extend(strftime(format[i], ms).encode_utf16());
        }
        i += 1;
    }
    String::from_utf16_lossy(&out)
}

fn format_2_digits(v: i64) -> String {
    if v < 10 {
        format!("0{v}")
    } else {
        v.to_string()
    }
}

fn fmt_ascii(f: &str, ms: i64) -> String {
    format_time(&f.encode_utf16().collect::<Vec<_>>(), ms)
}

/// `OsLib.strftime`; unknown conversions return Java `null`, which the
/// `StringBuilder` appends as "null".
fn strftime(conv: u16, ms: i64) -> String {
    let f = Fields::of(ms);
    let conv = if conv < 0x80 { conv as u8 } else { 0 };
    match conv {
        b'a' => SHORT_DAYS[(f.dow - 1) as usize].into(),
        b'A' => LONG_DAYS[(f.dow - 1) as usize].into(),
        b'b' | b'h' => SHORT_MONTHS[f.month0 as usize].into(),
        b'B' => LONG_MONTHS[f.month0 as usize].into(),
        b'c' => date_to_string(ms, &local_zone()),
        b'C' => (f.year_of_era() / 100).to_string(),
        b'd' => format_2_digits(f.day),
        b'D' => fmt_ascii("%m/%d/%y", ms),
        b'e' => {
            if f.day < 10 {
                format!(" {}", format_2_digits(f.day))
            } else {
                format_2_digits(f.day)
            }
        }
        b'H' => format_2_digits(f.hour),
        b'I' => format_2_digits(f.hour % 12),
        b'j' => day_of_year(ms).to_string(),
        b'm' => format_2_digits(f.month0 + 1),
        b'M' => format_2_digits(f.min),
        b'n' => "\n".into(),
        b'p' => if f.hour < 12 { "AM" } else { "PM" }.into(),
        b'r' => fmt_ascii("%I:%M:%S %p", ms),
        b'R' => fmt_ascii("%H:%M", ms),
        b'S' => format_2_digits(f.sec),
        b'U' => week_of_year(ms, true, false).to_string(),
        b'V' => week_of_year(ms, false, true).to_string(),
        b'w' => (f.dow - 1).to_string(),
        b'W' => week_of_year(ms, false, false).to_string(),
        b'y' => (f.year_of_era() % 100).to_string(),
        b'Y' => f.year_of_era().to_string(),
        b'Z' => "UTC".into(),
        _ => "null".into(),
    }
}

/// Java `Date.toString()`: `EEE MMM dd HH:mm:ss zzz yyyy` in `zone`.
fn date_to_string(ms: i64, zone: &Zone) -> String {
    let (offset, abbr) = zone.lookup(ms.div_euclid(1000));
    let f = Fields::of(ms + offset * 1000);
    format!(
        "{} {} {:02} {:02}:{:02}:{:02} {} {}",
        SHORT_DAYS[(f.dow - 1) as usize],
        SHORT_MONTHS[f.month0 as usize],
        f.day,
        f.hour,
        f.min,
        f.sec,
        java_zone_name(&abbr, offset),
        f.year_of_era()
    )
}

/// Java's short zone names come from CLDR; tzdb abbreviations agree for
/// most zones. Numeric tzdb abbreviations ("+03") become Java's custom
/// "GMT+03:00" form.
fn java_zone_name(abbr: &str, offset: i64) -> String {
    if !abbr.starts_with(['+', '-']) {
        return abbr.to_string();
    }
    if offset == 0 {
        return "GMT".into();
    }
    let sign = if offset < 0 { '-' } else { '+' };
    let a = offset.abs();
    format!("GMT{sign}{:02}:{:02}", a / 3600, a / 60 % 60)
}

// ---------------------------------------------------------------------------
// Local time zone (TZif files and POSIX TZ strings)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct ZoneType {
    offset: i64,
    abbr: String,
}

/// POSIX TZ rule: `std offset [dst [offset] [,start[/time],end[/time]]]`.
#[derive(Clone, Debug)]
struct Posix {
    std: ZoneType,
    dst: Option<(ZoneType, Rule, Rule)>,
}

#[derive(Clone, Copy, Debug)]
enum RuleDate {
    /// `Jn`: 1..=365, February 29th never counted.
    Julian1(i64),
    /// `n`: 0..=365, counting February 29th.
    Julian0(i64),
    /// `Mm.w.d`.
    MonthWeek(i64, i64, i64),
}

#[derive(Clone, Copy, Debug)]
struct Rule {
    date: RuleDate,
    time: i64,
}

#[derive(Clone, Debug, Default)]
struct Zone {
    transitions: Vec<i64>,
    indices: Vec<u8>,
    types: Vec<ZoneType>,
    footer: Option<Posix>,
}

impl Zone {
    fn utc() -> Zone {
        Zone { footer: Some(Posix { std: ZoneType { offset: 0, abbr: "UTC".into() }, dst: None }), ..Zone::default() }
    }

    /// Offset (seconds east of UTC) and abbreviation in effect at `t`.
    fn lookup(&self, t: i64) -> (i64, String) {
        let after_last = self.transitions.last().is_none_or(|&l| t >= l);
        if after_last {
            if let Some(p) = &self.footer {
                let z = p.at(t);
                return (z.offset, z.abbr);
            }
        }
        let n = self.transitions.partition_point(|&x| x <= t);
        let ty = if n == 0 { 0 } else { self.indices[n - 1] as usize };
        match self.types.get(ty) {
            Some(z) => (z.offset, z.abbr.clone()),
            None => (0, "GMT".into()),
        }
    }

    fn parse_tzif(data: &[u8]) -> Option<Zone> {
        if data.len() < 44 || &data[..4] != b"TZif" {
            return None;
        }
        let be32 = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
        let counts = |d: &[u8]| -> [usize; 6] { std::array::from_fn(|i| be32(&d[20 + 4 * i..])) };
        let version = data[4];
        let [isut, isstd, leap, timecnt, typecnt, charcnt] = counts(data);
        let v1_len = timecnt * 5 + typecnt * 6 + charcnt + leap * 8 + isstd + isut;
        let (body, tsize, cnt) = if version >= b'2' {
            let h2 = data.get(44 + v1_len..)?;
            if h2.len() < 44 || &h2[..4] != b"TZif" {
                return None;
            }
            (&h2[44..], 8, counts(h2))
        } else {
            (&data[44..], 4, [isut, isstd, leap, timecnt, typecnt, charcnt])
        };
        let [isut, isstd, leap, timecnt, typecnt, charcnt] = cnt;
        let mut pos = 0;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = body.get(pos..pos + n)?;
            pos += n;
            Some(s)
        };
        let times = take(timecnt * tsize)?;
        let transitions = times
            .chunks(tsize)
            .map(|c| if tsize == 8 { i64::from_be_bytes(c.try_into().unwrap()) } else { be32(c) as u32 as i32 as i64 })
            .collect();
        let indices = take(timecnt)?.to_vec();
        let raw_types = take(typecnt * 6)?.to_vec();
        let chars = take(charcnt)?.to_vec();
        take(leap * (tsize + 4) + isstd + isut)?;
        let types = raw_types
            .chunks(6)
            .map(|c| {
                let offset = be32(c) as u32 as i32 as i64;
                let ai = c[5] as usize;
                let end = chars[ai.min(chars.len())..].iter().position(|&b| b == 0).map_or(chars.len(), |p| p + ai);
                ZoneType { offset, abbr: String::from_utf8_lossy(&chars[ai.min(end)..end]).into_owned() }
            })
            .collect();
        let footer = if tsize == 8 {
            let rest = &body[pos..];
            let s = std::str::from_utf8(rest).ok()?.trim_matches('\n');
            Posix::parse(s.lines().next().unwrap_or(""))
        } else {
            None
        };
        Some(Zone { transitions, indices, types, footer })
    }
}

impl Posix {
    fn at(&self, t: i64) -> ZoneType {
        let Some((dst, start, end)) = &self.dst else { return self.std.clone() };
        let (year, _, _) = gregorian_civil((t + self.std.offset).div_euclid(86_400));
        let s = start.utc(year, self.std.offset);
        let e = end.utc(year, dst.offset);
        let in_dst = if s < e { t >= s && t < e } else { !(t >= e && t < s) };
        if in_dst {
            dst.clone()
        } else {
            self.std.clone()
        }
    }

    fn parse(s: &str) -> Option<Posix> {
        let b = s.as_bytes();
        let mut i = 0;
        let std_abbr = parse_name(b, &mut i)?;
        let std_off = -parse_hms(b, &mut i)?;
        let std = ZoneType { offset: std_off, abbr: std_abbr };
        if i >= b.len() {
            return Some(Posix { std, dst: None });
        }
        let dst_abbr = parse_name(b, &mut i)?;
        let dst_off = if i < b.len() && b[i] != b',' { -parse_hms(b, &mut i)? } else { std_off + 3600 };
        let dst = ZoneType { offset: dst_off, abbr: dst_abbr };
        let (start, end) = if i < b.len() && b[i] == b',' {
            i += 1;
            let start = parse_rule(b, &mut i)?;
            if b.get(i) != Some(&b',') {
                return None;
            }
            i += 1;
            (start, parse_rule(b, &mut i)?)
        } else {
            (
                Rule { date: RuleDate::MonthWeek(3, 2, 0), time: 7200 },
                Rule { date: RuleDate::MonthWeek(11, 1, 0), time: 7200 },
            )
        };
        Some(Posix { std, dst: Some((dst, start, end)) })
    }
}

impl Rule {
    /// The transition instant in `year`, with `offset` in effect before it.
    fn utc(&self, year: i64, offset: i64) -> i64 {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let jan1 = gregorian_days(year, 1, 1);
        let day = match self.date {
            RuleDate::Julian1(n) => jan1 + n - 1 + i64::from(leap && n >= 60),
            RuleDate::Julian0(n) => jan1 + n,
            RuleDate::MonthWeek(m, w, d) => {
                let first = gregorian_days(year, m, 1);
                let dow1 = (first + 4).rem_euclid(7);
                let mut day = 1 + (d - dow1).rem_euclid(7) + 7 * (w - 1);
                let month_len = gregorian_days(year + i64::from(m == 12), m % 12 + 1, 1) - first;
                while day > month_len {
                    day -= 7;
                }
                first + day - 1
            }
        };
        day * 86_400 + self.time - offset
    }
}

fn parse_name(b: &[u8], i: &mut usize) -> Option<String> {
    let start = *i;
    if b.get(*i) == Some(&b'<') {
        let end = b[start..].iter().position(|&c| c == b'>')? + start;
        *i = end + 1;
        return Some(String::from_utf8_lossy(&b[start + 1..end]).into_owned());
    }
    while *i < b.len() && b[*i].is_ascii_alphabetic() {
        *i += 1;
    }
    (*i - start >= 3).then(|| String::from_utf8_lossy(&b[start..*i]).into_owned())
}

/// `[+-]hh[:mm[:ss]]` in seconds.
fn parse_hms(b: &[u8], i: &mut usize) -> Option<i64> {
    let mut sign = 1;
    match b.get(*i) {
        Some(b'-') => {
            sign = -1;
            *i += 1;
        }
        Some(b'+') => *i += 1,
        _ => {}
    }
    let mut parts = [0i64; 3];
    for (k, part) in parts.iter_mut().enumerate() {
        if k > 0 {
            if b.get(*i) != Some(&b':') {
                break;
            }
            *i += 1;
        }
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *part = *part * 10 + (b[*i] - b'0') as i64;
            *i += 1;
        }
        if *i == start {
            return None;
        }
    }
    Some(sign * (parts[0] * 3600 + parts[1] * 60 + parts[2]))
}

fn parse_num(b: &[u8], i: &mut usize) -> Option<i64> {
    let start = *i;
    let mut v = 0i64;
    while *i < b.len() && b[*i].is_ascii_digit() {
        v = v * 10 + (b[*i] - b'0') as i64;
        *i += 1;
    }
    (*i > start).then_some(v)
}

fn parse_rule(b: &[u8], i: &mut usize) -> Option<Rule> {
    let date = match b.get(*i)? {
        b'J' => {
            *i += 1;
            RuleDate::Julian1(parse_num(b, i)?)
        }
        b'M' => {
            *i += 1;
            let m = parse_num(b, i)?;
            if b.get(*i) != Some(&b'.') {
                return None;
            }
            *i += 1;
            let w = parse_num(b, i)?;
            if b.get(*i) != Some(&b'.') {
                return None;
            }
            *i += 1;
            RuleDate::MonthWeek(m, w, parse_num(b, i)?)
        }
        _ => RuleDate::Julian0(parse_num(b, i)?),
    };
    let time = if b.get(*i) == Some(&b'/') {
        *i += 1;
        parse_hms(b, i)?
    } else {
        7200
    };
    Some(Rule { date, time })
}

/// The JVM default zone: `TZ`, else `/etc/localtime`; unknown zones fall
/// back to GMT as Java's does.
fn load_local_zone() -> Zone {
    let gmt = || Zone {
        footer: Some(Posix { std: ZoneType { offset: 0, abbr: "GMT".into() }, dst: None }),
        ..Zone::default()
    };
    let from_file = |p: &str| std::fs::read(p).ok().and_then(|d| Zone::parse_tzif(&d));
    match std::env::var("TZ") {
        Ok(tz) => {
            let tz = tz.strip_prefix(':').unwrap_or(&tz);
            if tz.is_empty() || tz == "UTC" {
                return Zone::utc();
            }
            let path = if tz.starts_with('/') { tz.to_string() } else { format!("/usr/share/zoneinfo/{tz}") };
            if !tz.contains("..") {
                if let Some(z) = from_file(&path) {
                    return z;
                }
            }
            match Posix::parse(tz) {
                Some(p) => Zone { footer: Some(p), ..Zone::default() },
                None => gmt(),
            }
        }
        Err(_) => from_file("/etc/localtime").unwrap_or_else(gmt),
    }
}

thread_local! {
    static LOCAL_ZONE: std::rc::Rc<Zone> = std::rc::Rc::new(load_local_zone());
}

fn local_zone() -> std::rc::Rc<Zone> {
    LOCAL_ZONE.with(|z| z.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::{LuaClosure, Thread};
    use std::rc::Rc;

    fn try_run(src: &str) -> LuaResult<Vec<Value>> {
        let env = TableRef::new();
        let mut t = Thread::new(env.clone());
        register(&env);
        let p = crate::compiler::compile(src, Some("test"), None, None)?;
        let f = Value::Closure(LuaClosure::new(Rc::new(p), env));
        t.call(&f, &[])
    }

    fn run1(src: &str) -> Value {
        try_run(src).unwrap_or_else(|e| panic!("{src}: {e}")).into_iter().next().unwrap_or_default()
    }

    fn date(fmt: &str, t: i64) -> String {
        match run1(&format!("return os.date('{fmt}', {t})")) {
            Value::Str(s) => s.to_string(),
            v => panic!("{v:?}"),
        }
    }

    fn num(src: &str) -> f64 {
        run1(src).as_num().unwrap_or_else(|| panic!("{src}: not a number"))
    }

    #[test]
    fn formats_utc_by_default() {
        assert_eq!(date("%Y-%m-%d %H:%M:%S", 0), "1970-01-01 00:00:00");
        assert_eq!(date("!%Y-%m-%d %H:%M:%S", 1_000_000_000), "2001-09-09 01:46:40");
        assert_eq!(date("%a %A %b %B %h", 0), "Thu Thursday Jan January Jan");
        assert_eq!(date("%d|%e|%j|%w|%y|%C|%Z", 0), "01| 01|0|4|70|19|UTC");
        assert_eq!(date("%D %R %r", 13 * 3600 + 5 * 60 + 9), "01/01/70 13:05 01:05:09 PM");
        assert_eq!(date("%I %p", 0), "00 AM");
        assert_eq!(date("%I %p", 12 * 3600), "00 PM");
        assert_eq!(date("%y", 1_104_537_600), "5"); // 2005, not zero-padded
        assert_eq!(date("%j", 31 * 86400), "31");
        assert_eq!(date("%Q|%|100%", 0), "null|null100%");
        assert_eq!(date("%n", 0), "\n");
    }

    #[test]
    fn week_numbers_follow_the_original() {
        // 1970-01-01 is a Thursday.
        assert_eq!(date("%U %W %V", 0), "0 0 0");
        assert_eq!(date("%U %W %V", 9 * 86400), "1 0 0");
        assert_eq!(date("%U %W %V", 11 * 86400), "1 1 1");
        // 2023-01-01 is a Sunday: the Sunday rule falls through to Jan 8.
        let jan15_2023 = 1_673_740_800;
        assert_eq!(date("%a %U %W %V", jan15_2023), "Sun 1 1 2");
    }

    #[test]
    fn date_table() {
        let t = run1("return os.date('!*t', 86400 * 31 + 3723.5)");
        let t = t.as_table().unwrap();
        let get = |k: &str| t.get_str(k);
        assert_eq!(get("year").as_num(), Some(1970.0));
        assert_eq!(get("month").as_num(), Some(2.0));
        assert_eq!(get("day").as_num(), Some(1.0));
        assert_eq!(get("hour").as_num(), Some(1.0));
        assert_eq!(get("min").as_num(), Some(2.0));
        assert_eq!(get("sec").as_num(), Some(3.0));
        assert_eq!(get("milli").as_num(), Some(500.0));
        assert_eq!(get("wday").as_num(), Some(1.0)); // Sunday
        assert_eq!(get("yday").as_num(), Some(31.0));
        assert!(get("isdst").is_nil());
        let keys: Vec<String> = t.keys().iter().map(|k| k.as_str().unwrap().to_string()).collect();
        assert_eq!(keys, ["year", "month", "day", "hour", "min", "sec", "wday", "yday", "milli"]);
    }

    #[test]
    fn time_from_table() {
        assert_eq!(num("return os.time({year = 1970, month = 1, day = 2})"), 86400.0);
        assert_eq!(num("return os.time({year = 2000, month = 13, day = 1})"), 978_307_200.0);
        assert_eq!(num("return os.time({year = 2000, month = 3, day = 0, hour = 12})"), 951_825_600.0);
        assert_eq!(num("return os.time({year = 1970, month = 1, day = 1, sec = 1, milli = 250})"), 1.25);
        assert_eq!(num("return os.difftime(10, 2.5)"), 7.5);
        let now = num("return os.time()");
        assert!(now > 1.6e9);
        assert!(matches!(run1("return os.date()"), Value::Str(_)));
    }

    #[test]
    fn errors() {
        let msg = |src: &str| match try_run(src) {
            Err(e) => match e.pcall_message() {
                Value::Str(s) => s.to_string(),
                v => format!("{v:?}"),
            },
            Ok(v) => panic!("{src}: {v:?}"),
        };
        assert_eq!(
            msg("return os.time({month = 1, day = 1})"),
            "Cannot invoke \"java.lang.Double.doubleValue()\" because \"o\" is null java.lang.NullPointerException"
        );
        assert_eq!(
            msg("return os.time({year = '1970', month = 1, day = 1})"),
            "class java.lang.String cannot be cast to class java.lang.Double java.lang.ClassCastException"
        );
        assert_eq!(msg("return os.date('')"), "Index 0 out of bounds for length 0 java.lang.StringIndexOutOfBoundsException");
        assert_eq!(msg("return os.date(nil)"), "missing argument #1 to 'date' java.lang.RuntimeException");
        assert_eq!(msg("return os.difftime(1)"), "missing argument #2 to 'difftime' java.lang.RuntimeException");
    }

    #[test]
    fn calendar_arithmetic() {
        assert_eq!(gregorian_days(1970, 1, 1), 0);
        assert_eq!(gregorian_days(1582, 10, 15), CUTOVER_DAY);
        assert_eq!(julian_days(1582, 10, 4), CUTOVER_DAY - 1);
        assert_eq!(julian_civil(CUTOVER_DAY - 1), (1582, 10, 4));
        assert_eq!(gregorian_civil(CUTOVER_DAY), (1582, 10, 15));
        for d in [-800_000, -141_428, 0, 12_345, 2_000_000] {
            let (y, m, dd) = julian_civil(d);
            assert_eq!(julian_days(y, m, dd), d);
            let (y, m, dd) = gregorian_civil(d);
            assert_eq!(gregorian_days(y, m, dd), d);
        }
        // Lenient fields across the cutover land on the Julian calendar.
        let ms = ms_from_fields(1582, 9, 4, 0, 0, 0, 0);
        assert_eq!(ms / MS_PER_DAY, CUTOVER_DAY - 1);
        assert_eq!(date("%Y-%m-%d", (CUTOVER_DAY - 1) * 86400), "1582-10-04");
        // Proleptic years 0 and -1 are 1 and 2 BC: YEAR is the year of era.
        assert_eq!(date("%Y", ms_from_fields(0, 5, 1, 0, 0, 0, 0) / 1000), "1");
        assert_eq!(date("%Y", ms_from_fields(-1, 5, 1, 0, 0, 0, 0) / 1000), "2");
    }

    #[test]
    fn date_to_string_uses_the_zone() {
        let la = Zone { footer: Posix::parse("PST8PDT,M3.2.0,M11.1.0"), ..Zone::default() };
        assert_eq!(date_to_string(0, &la), "Wed Dec 31 16:00:00 PST 1969");
        assert_eq!(date_to_string(1_593_561_600_000, &la), "Tue Jun 30 17:00:00 PDT 2020");
        assert_eq!(date_to_string(1_000, &Zone::utc()), "Thu Jan 01 00:00:01 UTC 1970");
        let syd = Zone { footer: Posix::parse("AEST-10AEDT,M10.1.0,M4.1.0/3"), ..Zone::default() };
        assert_eq!(date_to_string(1_577_836_800_000, &syd), "Wed Jan 01 11:00:00 AEDT 2020");
        assert_eq!(date_to_string(1_593_561_600_000, &syd), "Wed Jul 01 10:00:00 AEST 2020");
        let num = Zone { footer: Posix::parse("<+03>-3"), ..Zone::default() };
        assert_eq!(date_to_string(0, &num), "Thu Jan 01 03:00:00 GMT+03:00 1970");
        if let Ok(d) = std::fs::read("/usr/share/zoneinfo/America/New_York") {
            let ny = Zone::parse_tzif(&d).expect("tzif");
            assert_eq!(date_to_string(0, &ny), "Wed Dec 31 19:00:00 EST 1969");
            assert_eq!(date_to_string(1_593_561_600_000, &ny), "Tue Jun 30 20:00:00 EDT 2020");
            assert_eq!(date_to_string(4_102_444_800_000, &ny), "Thu Dec 31 19:00:00 EST 2099");
        }
    }
}
