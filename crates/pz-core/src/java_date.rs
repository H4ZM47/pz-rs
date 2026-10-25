//! `java.util.Locale`, `java.text.SimpleDateFormat` formatting and
//! `java.util.Date.toString()`, as the JVM the game bundles (Java 25, CLDR
//! locale data) does them for English text.
//!
//! What was checked against the bundled JRE (see the tests): pattern
//! compilation and its errors, every pattern letter for the English,
//! US, UK and root locales, CLDR week rules (first day of week, minimal
//! days in the first week) by region, the default date/time style patterns
//! of `en`, `en_US`, `en_CA`, `en_GB` and the root locale, locale strings
//! and English display names.
//!
//! Not modelled: month, day, era and AM/PM names in other languages (they
//! are formatted with the US English names), long zone names (`zzzz` prints the short
//! one), and instants before 1883, where the tz database's local mean time
//! differs from the JVM's zone data.

use crate::calendar::{calendar_fields, days_from_civil, JavaZone};

/// `java.util.Locale` (language, script, country, variant).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Locale {
    pub language: String,
    pub script: String,
    pub country: String,
    pub variant: String,
}

impl Locale {
    /// `Locale.of(language, country, variant)`: the language lower-cased,
    /// the country upper-cased (ASCII), the variant as given.
    pub fn of(language: &str, country: &str, variant: &str) -> Locale {
        Locale {
            language: language.to_ascii_lowercase(),
            script: String::new(),
            country: country.to_ascii_uppercase(),
            variant: variant.to_string(),
        }
    }

    /// `Locale.ROOT`.
    pub fn root() -> Locale {
        Locale::default()
    }

    /// `Locale.forLanguageTag(tag)` for `language[-Script][-REGION]` tags
    /// (`und` is the empty language).
    pub fn for_language_tag(tag: &str) -> Locale {
        let mut l = Locale::default();
        for (i, part) in tag.split(['-', '_']).enumerate() {
            let alpha = part.chars().all(|c| c.is_ascii_alphabetic());
            match (i, part.len()) {
                (0, 2..=8) if alpha => {
                    if !part.eq_ignore_ascii_case("und") {
                        l.language = part.to_ascii_lowercase();
                    }
                }
                (0, _) => return Locale::default(),
                (_, 4) if alpha && l.script.is_empty() && l.country.is_empty() => {
                    let mut s = part.to_ascii_lowercase();
                    s[..1].make_ascii_uppercase();
                    l.script = s;
                }
                (_, 2) if alpha && l.country.is_empty() => l.country = part.to_ascii_uppercase(),
                (_, 3) if part.chars().all(|c| c.is_ascii_digit()) && l.country.is_empty() => l.country = part.into(),
                _ => break,
            }
        }
        l
    }

    /// `Locale.getDefault()`: the JVM takes it from the user's settings;
    /// here from `LC_ALL` / `LC_MESSAGES` / `LANG` (`en_US.UTF-8`), else
    /// `en_US`.
    pub fn default_locale() -> Locale {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(v) = std::env::var(var) {
                let base = v.split(['.', '@']).next().unwrap_or("");
                let mut it = base.split('_');
                let lang = it.next().unwrap_or("");
                if !lang.is_empty() && lang.chars().all(|c| c.is_ascii_alphabetic()) && lang != "C" && lang != "POSIX" {
                    return Locale::of(lang, it.next().unwrap_or(""), "");
                }
            }
        }
        Locale::of("en", "US", "")
    }

    /// `toString()`: `en`, `en_US`, `en_US_WIN`, `en__WIN`, `zz_QQ_#Latn`.
    pub fn to_java_string(&self) -> String {
        let (l, c, v, s) = (&self.language, &self.country, &self.variant, &self.script);
        if l.is_empty() && c.is_empty() && v.is_empty() && s.is_empty() {
            return String::new();
        }
        let mut out = l.clone();
        if !c.is_empty() || !v.is_empty() || !s.is_empty() {
            out.push('_');
            out.push_str(c);
        }
        if !v.is_empty() {
            out.push('_');
            out.push_str(v);
        }
        if !s.is_empty() {
            out.push_str("_#");
            out.push_str(s);
        }
        out
    }

    /// `toLanguageTag()`: `und` for the root locale, `en-US`, and a
    /// variant as `-x-lvariant-...`.
    pub fn to_language_tag(&self) -> String {
        let mut out = if self.language.is_empty() { "und".to_string() } else { self.language.clone() };
        if !self.script.is_empty() {
            out.push('-');
            out.push_str(&self.script);
        }
        if !self.country.is_empty() {
            out.push('-');
            out.push_str(&self.country);
        }
        if !self.variant.is_empty() {
            out.push_str("-x-lvariant-");
            out.push_str(&self.variant.replace('_', "-"));
        }
        if out == "und" && !self.variant.is_empty() {
            return out;
        }
        out
    }

    /// `getDisplayLanguage(Locale.US)`; the code when no English name is
    /// known here.
    pub fn display_language(&self) -> String {
        let name = match self.language.as_str() {
            "en" => "English",
            "fr" => "French",
            "de" => "German",
            "it" => "Italian",
            "ja" => "Japanese",
            "ko" => "Korean",
            "zh" => "Chinese",
            "es" => "Spanish",
            "pt" => "Portuguese",
            "ru" => "Russian",
            "pl" => "Polish",
            "nl" => "Dutch",
            other => other,
        };
        name.to_string()
    }

    /// `getDisplayCountry(Locale.US)`; the code when no English name is
    /// known here.
    pub fn display_country(&self) -> String {
        let name = match self.country.as_str() {
            "US" => "United States",
            "GB" => "United Kingdom",
            "CA" => "Canada",
            "FR" => "France",
            "DE" => "Germany",
            "IT" => "Italy",
            "JP" => "Japan",
            "KR" => "South Korea",
            "CN" => "China",
            "TW" => "Taiwan",
            other => other,
        };
        name.to_string()
    }

    /// `getDisplayName(Locale.US)`: `English (United States, WIN)`.
    pub fn display_name(&self) -> String {
        let lang = self.display_language();
        let mut extra: Vec<String> = Vec::new();
        if !self.country.is_empty() {
            extra.push(self.display_country());
        }
        if !self.variant.is_empty() {
            extra.push(self.variant.clone());
        }
        match (lang.is_empty(), extra.is_empty()) {
            (_, true) => lang,
            (true, false) => extra.join(", "),
            (false, false) => format!("{lang} ({})", extra.join(", ")),
        }
    }

    /// CLDR week data as `Calendar.getInstance(locale)` reports it:
    /// (`getFirstDayOfWeek`, `getMinimalDaysInFirstWeek`). A locale without
    /// a region uses the US rules; an unlisted region the world rules.
    pub fn week_rules(&self) -> (i32, i32) {
        const SUNDAY_1: &str = "AG AS BD BR BS BT BW BZ CA CO DM DO ET GT GU HK HN ID IL IN JM JP KE KH KR LA MH MM \
                                MO MT MX MZ NI NP PA PE PH PK PR PY SA SG SV TH TT TW UM US VE VI WS YE ZA ZW";
        const MONDAY_4: &str = "AD AT AX BE BG CH CZ DE DK EE ES FI FJ FO FR GB GF GG GI GP GR HU IE IM IS IT JE LI \
                                LT LU MC MQ NL NO PL RE RU SE SJ SK SM VA";
        const SATURDAY_1: &str = "AF BH DJ DZ EG IQ IR JO KW LY OM QA SD SY";
        let c = self.country.as_str();
        let has = |list: &str| list.split_whitespace().any(|x| x == c);
        if c.is_empty() || has(SUNDAY_1) {
            (1, 1)
        } else if c == "PT" {
            (1, 4)
        } else if has(MONDAY_4) {
            (2, 4)
        } else if c == "MV" {
            (6, 1)
        } else if has(SATURDAY_1) {
            (7, 1)
        } else {
            (2, 1)
        }
    }

    /// `DateFormat.get{Date,Time,DateTime}Instance` patterns (`FULL` 0 ..
    /// `SHORT` 3): CLDR's for the root locale, `en_GB` and `en_CA`; every
    /// other locale gets the `en`/`en_US` patterns here.
    fn style_patterns(&self) -> ([&'static str; 4], [&'static str; 4], &'static str) {
        const US_TIME: [&str; 4] = ["h:mm:ss\u{202f}a zzzz", "h:mm:ss\u{202f}a z", "h:mm:ss\u{202f}a", "h:mm\u{202f}a"];
        const H24: [&str; 4] = ["HH:mm:ss zzzz", "HH:mm:ss z", "HH:mm:ss", "HH:mm"];
        if self.language.is_empty() && self.country.is_empty() {
            return (["y MMMM d, EEEE", "y MMMM d", "y MMM d", "y-MM-dd"], H24, " ");
        }
        match self.country.as_str() {
            "GB" if self.language == "en" => (["EEEE, d MMMM y", "d MMMM y", "d MMM y", "dd/MM/y"], H24, ", "),
            "CA" if self.language == "en" => (["EEEE, MMMM d, y", "MMMM d, y", "MMM d, y", "y-MM-dd"], US_TIME, ", "),
            _ => (["EEEE, MMMM d, y", "MMMM d, y", "MMM d, y", "M/d/yy"], US_TIME, ", "),
        }
    }

    /// `DateFormat.getDateInstance(style, locale)`'s pattern.
    pub fn date_pattern(&self, style: usize) -> String {
        self.style_patterns().0[style.min(3)].to_string()
    }

    /// `DateFormat.getTimeInstance(style, locale)`'s pattern.
    pub fn time_pattern(&self, style: usize) -> String {
        self.style_patterns().1[style.min(3)].to_string()
    }

    /// `DateFormat.getDateTimeInstance(dateStyle, timeStyle, locale)`'s
    /// pattern.
    pub fn date_time_pattern(&self, date: usize, time: usize) -> String {
        let (d, t, glue) = self.style_patterns();
        format!("{}{glue}{}", d[date.min(3)], t[time.min(3)])
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const SHORT_MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const SHORT_DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// `DateFormatSymbols` for a locale (English text; see the module notes).
#[derive(Clone, Copy, Debug)]
pub struct Symbols {
    pub am_pm: [&'static str; 2],
    pub eras: [&'static str; 2],
    pub months: [&'static str; 12],
    pub short_months: [&'static str; 12],
    pub days: [&'static str; 7],
    pub short_days: [&'static str; 7],
}

impl Symbols {
    /// CLDR's English variants as the JRE reports them: `en`/`en_US` (and
    /// every region the JRE has no `en_*` locale for), `en_AU`, `en_CA`,
    /// `en_IE`, the regions inheriting from `en_001` (lower-case am/pm,
    /// "Sept"), and the root locale (abbreviated names only, BCE/CE).
    /// Other languages get the US set.
    pub fn of(locale: &Locale) -> Symbols {
        // The JRE's `en_*` locales that inherit from `en_001`; other regions
        // fall back to `en`.
        const EN_001: &str = "001 150 AG AI AT BB BE BM BS BW BZ CC CH CK CM CX CY CZ DE DG DK DM ER ES FI FJ FK FM FR GB GD GG GH GI GM GS GY HK HU ID IL IM IN IO IT JE JM KE KI KN KY LC LR LS MG MO MS MT MU MV MW MY NA NF NG NL NO NR NU NZ PG PK PL PN PT PW RO RW SB SC SD SE SG SH SI SK SL SS SX SZ TC TK TO TT TV TZ UG VC VG VU WS ZA ZM ZW";
        let mut s = Symbols {
            am_pm: ["AM", "PM"],
            eras: ["BC", "AD"],
            months: MONTHS,
            short_months: SHORT_MONTHS,
            days: DAYS,
            short_days: SHORT_DAYS,
        };
        if locale.language.is_empty() {
            if locale.country.is_empty() {
                s.eras = ["BCE", "CE"];
                s.months = SHORT_MONTHS;
                s.days = SHORT_DAYS;
            }
            return s;
        }
        let c = locale.country.as_str();
        if locale.language != "en" || !matches!(c, "AU" | "CA" | "IE") && !EN_001.split(' ').any(|r| r == c) {
            return s;
        }
        match c {
            "CA" => s.am_pm = ["a.m.", "p.m."],
            "IE" => {
                s.am_pm = ["a.m.", "p.m."];
                s.short_months[8] = "Sept";
            }
            "AU" => {
                s.am_pm = ["am", "pm"];
                s.short_months[5] = "June";
                s.short_months[6] = "July";
                s.short_months[8] = "Sept";
            }
            _ => {
                s.am_pm = ["am", "pm"];
                s.short_months[8] = "Sept";
            }
        }
        s
    }
}

/// The pattern letters, in `DateFormat`'s field order.
const PATTERN_CHARS: &str = "GyMdkHmsSEDFwWahKzZYuXL";

/// A compiled pattern element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Literal(String),
    Field(char, usize),
}

/// `SimpleDateFormat.compile`: the pattern's fields and literal text, or
/// the `IllegalArgumentException` message.
pub fn compile(pattern: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out: Vec<Token> = Vec::new();
    let mut lit = String::new();
    let mut quoted = String::new();
    let mut in_quote = false;
    let mut last: Option<char> = None;
    let mut count = 0usize;
    let flush_field = |out: &mut Vec<Token>, lit: &mut String, last: &mut Option<char>, count: &mut usize| {
        if let Some(c) = last.take() {
            if !lit.is_empty() {
                out.push(Token::Literal(std::mem::take(lit)));
            }
            out.push(Token::Field(c, *count));
            *count = 0;
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            if i + 1 < chars.len() && chars[i + 1] == '\'' {
                i += 2;
                flush_field(&mut out, &mut lit, &mut last, &mut count);
                if in_quote {
                    quoted.push('\'');
                } else {
                    lit.push('\'');
                }
                continue;
            }
            if in_quote {
                lit.push_str(&std::mem::take(&mut quoted));
            } else {
                flush_field(&mut out, &mut lit, &mut last, &mut count);
            }
            in_quote = !in_quote;
            i += 1;
            continue;
        }
        if in_quote {
            quoted.push(c);
            i += 1;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            flush_field(&mut out, &mut lit, &mut last, &mut count);
            lit.push(c);
            i += 1;
            continue;
        }
        if !PATTERN_CHARS.contains(c) {
            return Err(format!("Illegal pattern character '{c}'"));
        }
        if last.is_none() || last == Some(c) {
            last = Some(c);
            count += 1;
        } else {
            flush_field(&mut out, &mut lit, &mut last, &mut count);
            last = Some(c);
            count = 1;
        }
        i += 1;
    }
    if in_quote {
        return Err("Unterminated quote".into());
    }
    flush_field(&mut out, &mut lit, &mut last, &mut count);
    if !lit.is_empty() {
        out.push(Token::Literal(lit));
    }
    for t in &out {
        if let Token::Field('X', n) = t {
            if *n > 3 {
                return Err(format!("invalid ISO 8601 format: length={n}"));
            }
        }
    }
    Ok(out)
}

/// `%0Nd` with a leading minus for negatives.
fn pad(v: i64, width: usize) -> String {
    if v < 0 {
        format!("-{:0width$}", -v, width = width)
    } else {
        format!("{v:0width$}")
    }
}

/// `getDayOfWeekDateOnOrBefore`: the last day on or before `day` (days
/// since 1970-01-01) that falls on `dow` (Sunday = 1).
fn on_or_before(day: i64, dow: i32) -> i64 {
    let d = (day + 4).rem_euclid(7) + 1;
    day - (d - i64::from(dow)).rem_euclid(7)
}

/// `GregorianCalendar.getWeekNumber(fixedDay1, fixedDate)`.
fn week_number(day1: i64, date: i64, (first, min): (i32, i32)) -> i64 {
    let mut day1st = on_or_before(day1 + 6, first);
    if day1st - day1 >= i64::from(min) {
        day1st -= 7;
    }
    (date - day1st).div_euclid(7) + 1
}

fn leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

/// The calendar fields of an instant as a `GregorianCalendar` in `zone`
/// with `locale`'s week rules computes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fields {
    /// Proleptic year (0 is 1 BC).
    pub year: i64,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
    pub milli: i32,
    pub day_of_week: i32,
    pub day_of_year: i32,
    pub week_of_year: i32,
    pub week_of_month: i32,
    pub week_year: i64,
    pub offset_ms: i64,
}

impl Fields {
    pub fn of(utc_ms: i64, zone: &JavaZone, locale: &Locale) -> Fields {
        let f = calendar_fields(zone, utc_ms);
        let rules = locale.week_rules();
        let year = i64::from(f.year);
        let date = days_from_civil(year, i64::from(f.month) + 1, i64::from(f.day_of_month));
        let jan1 = days_from_civil(year, 1, 1);
        let mut woy = week_number(jan1, date, rules);
        if woy == 0 {
            woy = week_number(days_from_civil(year - 1, 1, 1), jan1 - 1, rules);
        } else if woy >= 52 {
            let next_jan1 = jan1 + 365 + i64::from(leap(year));
            let next_jan1st = on_or_before(next_jan1 + 6, rules.0);
            if next_jan1st - next_jan1 >= i64::from(rules.1) && date >= next_jan1st - 7 {
                woy = 1;
            }
        }
        // `getWeekYear()`.
        let week_year = if f.month == 0 && woy >= 52 {
            year - 1
        } else if f.month != 0 && woy == 1 {
            year + 1
        } else {
            year
        };
        let month1 = days_from_civil(year, i64::from(f.month) + 1, 1);
        Fields {
            year,
            month: f.month,
            day: f.day_of_month,
            hour: f.hour_of_day,
            minute: f.minute,
            second: f.second,
            milli: f.millisecond,
            day_of_week: f.day_of_week,
            day_of_year: f.day_of_year,
            week_of_year: woy as i32,
            week_of_month: week_number(month1, date, rules) as i32,
            week_year,
            offset_ms: f.offset,
        }
    }
}

/// `TimeZone.getDisplayName(daylight, SHORT, Locale.US)` from the tz
/// abbreviation; numeric abbreviations (`+03`) become `GMT+03:00`.
fn zone_short_name(zone: &JavaZone, utc_ms: i64, offset_ms: i64) -> String {
    let abbr = zone.abbreviation_at(utc_ms);
    if abbr.is_empty() || abbr.starts_with(['+', '-']) || abbr.chars().any(|c| c.is_ascii_digit()) {
        let m = offset_ms / 60_000;
        let sign = if m < 0 { '-' } else { '+' };
        return format!("GMT{sign}{:02}:{:02}", m.abs() / 60, m.abs() % 60);
    }
    abbr
}

/// `SimpleDateFormat.format(new Date(utc_ms))` for compiled `tokens`.
pub fn format(tokens: &[Token], utc_ms: i64, zone: &JavaZone, locale: &Locale) -> String {
    let f = Fields::of(utc_ms, zone, locale);
    let sym = Symbols::of(locale);
    let mut out = String::new();
    // Year of era.
    let (era_ad, yoe) = if f.year >= 1 { (true, f.year) } else { (false, 1 - f.year) };
    let week_year = f.week_year;
    for t in tokens {
        let (c, n) = match t {
            Token::Literal(s) => {
                out.push_str(s);
                continue;
            }
            Token::Field(c, n) => (*c, *n),
        };
        let s = match c {
            'G' => sym.eras[usize::from(era_ad)].to_string(),
            'y' => {
                if n == 2 {
                    pad(yoe % 100, 2)
                } else {
                    pad(yoe, n)
                }
            }
            'Y' => {
                if n == 2 {
                    pad(week_year % 100, 2)
                } else {
                    pad(week_year, n)
                }
            }
            'M' | 'L' => match n {
                1 | 2 => pad(i64::from(f.month) + 1, n),
                3 => sym.short_months[f.month as usize].to_string(),
                _ => sym.months[f.month as usize].to_string(),
            },
            'd' => pad(f.day.into(), n),
            'k' => pad(if f.hour == 0 { 24 } else { f.hour }.into(), n),
            'H' => pad(f.hour.into(), n),
            'm' => pad(f.minute.into(), n),
            's' => pad(f.second.into(), n),
            'S' => pad(f.milli.into(), n),
            'E' => {
                let i = (f.day_of_week - 1) as usize;
                (if n >= 4 { sym.days[i] } else { sym.short_days[i] }).to_string()
            }
            'u' => pad(if f.day_of_week == 1 { 7 } else { f.day_of_week - 1 }.into(), n),
            'D' => pad(f.day_of_year.into(), n),
            'F' => pad(((f.day - 1) / 7 + 1).into(), n),
            'w' => pad(f.week_of_year.into(), n),
            'W' => pad(f.week_of_month.into(), n),
            'a' => sym.am_pm[usize::from(f.hour >= 12)].to_string(),
            'h' => pad(if f.hour % 12 == 0 { 12 } else { f.hour % 12 }.into(), n),
            'K' => pad((f.hour % 12).into(), n),
            'z' => zone_short_name(zone, utc_ms, f.offset_ms),
            'Z' => {
                let m = f.offset_ms / 60_000;
                let sign = if m < 0 { '-' } else { '+' };
                format!("{sign}{:04}", (m.abs() / 60) * 100 + m.abs() % 60)
            }
            'X' => {
                let m = f.offset_ms / 60_000;
                if m == 0 {
                    "Z".to_string()
                } else {
                    let sign = if m < 0 { '-' } else { '+' };
                    let (h, mm) = (m.abs() / 60, m.abs() % 60);
                    match n {
                        1 => format!("{sign}{h:02}"),
                        2 => format!("{sign}{h:02}{mm:02}"),
                        _ => format!("{sign}{h:02}:{mm:02}"),
                    }
                }
            }
            _ => String::new(),
        };
        out.push_str(&s);
    }
    out
}

/// `new Date(utc_ms).toString()`: `EEE MMM dd HH:mm:ss zzz yyyy` with an
/// unpadded year, in the default zone.
pub fn date_to_string(utc_ms: i64, zone: &JavaZone) -> String {
    let f = Fields::of(utc_ms, zone, &Locale::of("en", "US", ""));
    let yoe = if f.year >= 1 { f.year } else { 1 - f.year };
    format!(
        "{} {} {:02} {:02}:{:02}:{:02} {} {}",
        SHORT_DAYS[(f.day_of_week - 1) as usize],
        SHORT_MONTHS[f.month as usize],
        f.day,
        f.hour,
        f.minute,
        f.second,
        zone_short_name(zone, utc_ms, f.offset_ms),
        yoe
    )
}

/// Java `String.hashCode()`.
pub fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

#[cfg(test)]
mod tests;
