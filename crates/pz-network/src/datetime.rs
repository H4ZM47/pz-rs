//! `java.time.LocalDateTime` as the server list uses it: local wall-clock
//! time, stored in the database as `yyyy-MM-dd HH:mm:ss`.

use jiff::civil::{Date, DateTime, Time};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LocalDateTime(pub DateTime);

impl LocalDateTime {
    /// `LocalDateTime.now()` in the system time zone.
    pub fn now() -> LocalDateTime {
        LocalDateTime(jiff::Zoned::now().datetime())
    }

    /// `format(AccountDBHelper.formatter)`.
    pub fn format_db(&self) -> String {
        let d = self.0;
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            d.year(),
            d.month(),
            d.day(),
            d.hour(),
            d.minute(),
            d.second()
        )
    }

    /// `LocalDateTime.parse(s, AccountDBHelper.formatter)` (`ResolverStyle.SMART`):
    /// fields have exactly their pattern width, a day past the end of its
    /// month is clamped to the last day, anything else invalid is `None`.
    pub fn parse_db(s: &str) -> Option<LocalDateTime> {
        let b = s.as_bytes();
        if b.len() != 19
            || b[4] != b'-'
            || b[7] != b'-'
            || b[10] != b' '
            || b[13] != b':'
            || b[16] != b':'
        {
            return None;
        }
        let num = |r: std::ops::Range<usize>| -> Option<i32> {
            b[r].iter().try_fold(0i32, |n, &c| {
                c.is_ascii_digit().then(|| n * 10 + (c - b'0') as i32)
            })
        };
        let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
        let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
        if y < 1
            || !(1..=12).contains(&mo)
            || !(1..=31).contains(&d)
            || h > 23
            || mi > 59
            || sec > 59
        {
            return None;
        }
        let first = Date::new(y as i16, mo as i8, 1).ok()?;
        let d = d.min(first.days_in_month() as i32);
        let date = Date::new(y as i16, mo as i8, d as i8).ok()?;
        let time = Time::new(h as i8, mi as i8, sec as i8, 0).ok()?;
        Some(LocalDateTime(date.to_datetime(time)))
    }

    /// `LocalDateTime.toString()`: `uuuu-MM-ddTHH:mm`, then `:ss` unless
    /// seconds and nanos are zero, then the fraction in 3, 6 or 9 digits.
    pub fn to_java_string(&self) -> String {
        let d = self.0;
        let mut s = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}",
            d.year(),
            d.month(),
            d.day(),
            d.hour(),
            d.minute()
        );
        let nanos = d.subsec_nanosecond();
        if d.second() > 0 || nanos > 0 {
            s += &format!(":{:02}", d.second());
            if nanos > 0 {
                if nanos % 1_000_000 == 0 {
                    s += &format!(".{:03}", nanos / 1_000_000);
                } else if nanos % 1000 == 0 {
                    s += &format!(".{:06}", nanos / 1000);
                } else {
                    s += &format!(".{nanos:09}");
                }
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_like_java() {
        let t = LocalDateTime::parse_db("2026-02-30 07:05:00").unwrap();
        assert_eq!(t.format_db(), "2026-02-28 07:05:00");
        assert_eq!(t.to_java_string(), "2026-02-28T07:05");
        assert!(LocalDateTime::parse_db("2026-2-03 07:05:00").is_none());
        assert!(LocalDateTime::parse_db("2026-02-32 07:05:00").is_none());
        assert!(LocalDateTime::parse_db("2026-02-03 24:00:00").is_none());
        assert!(LocalDateTime::parse_db("").is_none());
        let t = LocalDateTime(jiff::civil::date(2026, 10, 25).at(8, 1, 2, 340_000_000));
        assert_eq!(t.to_java_string(), "2026-10-25T08:01:02.340");
        assert_eq!(t.format_db(), "2026-10-25 08:01:02");
    }
}
