//! `java.util.Locale`, `java.util.Date`, `java.text.SimpleDateFormat` and
//! `zombie.util.PZCalendar` (the Lua globals `Calendar` and `PZCalendar`)
//! as the game exposes them. `MapSpawnSelect` names a new save with
//! `SimpleDateFormat.new("yyyy-MM-dd_HH-mm-ss", Locale.ENGLISH)
//! :format(Calendar.getInstance():getTime())`.
//!
//! Formatting is [`pz_core::java_date`] in the JVM's default time zone
//! ([`pz_core::weather::calendar::default_zone`]); see that module for
//! what was checked against the bundled JRE and what is not modelled.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pz_core::calendar::{calendar_fields, calendar_millis, JavaZone};
use pz_core::java_date::{self, Locale, Token};
use pz_core::weather::calendar::default_zone;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};

use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, this};

pub const LOCALE: &str = "java.util.Locale";
pub const DATE: &str = "java.util.Date";
pub const SIMPLE_DATE_FORMAT: &str = "java.text.SimpleDateFormat";
pub const PZ_CALENDAR: &str = "zombie.util.PZCalendar";

fn illegal(msg: impl Into<String>) -> LuaError {
    LuaError::Runtime { message: msg.into(), class: "java.lang.IllegalArgumentException" }
}

fn npe() -> LuaError {
    LuaError::Runtime { message: "null".into(), class: "java.lang.NullPointerException" }
}

// ---- Locale --------------------------------------------------------------------

/// A `java.util.Locale` object.
#[derive(Debug)]
pub struct LocaleObj(pub Locale);

impl LuaObject for LocaleObj {
    fn class_name(&self) -> &str {
        LOCALE
    }
    fn to_lua_string(&self) -> String {
        self.0.to_java_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn locale_value(l: Locale) -> Value {
    Value::Object(Rc::new(LocaleObj(l)))
}

fn locale_of(v: &Value) -> Option<Locale> {
    match v {
        Value::Object(o) => o.as_any().downcast_ref::<LocaleObj>().map(|l| l.0.clone()),
        _ => None,
    }
}

fn self_locale(c: &NativeCall<'_>) -> LuaResult<Locale> {
    let o = this::<LocaleObj>(c)?;
    Ok(o.as_any().downcast_ref::<LocaleObj>().unwrap().0.clone())
}

fn string_args(c: &NativeCall<'_>, n: usize) -> LuaResult<Vec<String>> {
    (0..n).map(|i| str_arg(c, i).map(|s| s.to_string()).ok_or_else(npe)).collect()
}

fn new_locale(c: &NativeCall<'_>) -> LuaResult<Value> {
    let n = c.nargs().clamp(1, 3);
    let a = string_args(c, n)?;
    let get = |i: usize| a.get(i).map_or("", String::as_str);
    Ok(locale_value(Locale::of(get(0), get(1), get(2))))
}

fn register_locale(thread: &mut Thread) {
    let b = expose_class(thread, LOCALE, None);
    let consts: [(&str, &str, &str); 19] = [
        ("ENGLISH", "en", ""),
        ("FRENCH", "fr", ""),
        ("GERMAN", "de", ""),
        ("ITALIAN", "it", ""),
        ("JAPANESE", "ja", ""),
        ("KOREAN", "ko", ""),
        ("CHINESE", "zh", ""),
        ("SIMPLIFIED_CHINESE", "zh", "CN"),
        ("TRADITIONAL_CHINESE", "zh", "TW"),
        ("FRANCE", "fr", "FR"),
        ("GERMANY", "de", "DE"),
        ("ITALY", "it", "IT"),
        ("JAPAN", "ja", "JP"),
        ("KOREA", "ko", "KR"),
        ("UK", "en", "GB"),
        ("US", "en", "US"),
        ("CANADA", "en", "CA"),
        ("CANADA_FRENCH", "fr", "CA"),
        ("ROOT", "", ""),
    ];
    for (name, l, c) in consts {
        b.field(name, locale_value(Locale::of(l, c, "")));
    }
    // `CHINA` and `PRC` are `SIMPLIFIED_CHINESE`, `TAIWAN` is
    // `TRADITIONAL_CHINESE` (the same objects).
    let simplified = b.statics.get_str("SIMPLIFIED_CHINESE");
    b.field("CHINA", simplified.clone());
    b.field("PRC", simplified);
    b.field("TAIWAN", b.statics.get_str("TRADITIONAL_CHINESE"));
    b.field("PRIVATE_USE_EXTENSION", "x");
    b.field("UNICODE_LOCALE_EXTENSION", "u");
    b.ctor(|c| {
        let v = new_locale(c)?;
        Ok(c.push(v))
    });
    b.static_fn("of", |c| {
        let v = new_locale(c)?;
        Ok(c.push(v))
    });
    b.static_fn("getDefault", |c| Ok(c.push(locale_value(Locale::default_locale()))));
    b.static_fn("forLanguageTag", |c| {
        let tag = string_args(c, 1)?;
        Ok(c.push(locale_value(Locale::for_language_tag(&tag[0]))))
    });
    type Getter = fn(&Locale) -> String;
    let getters: [(&str, Getter); 10] = [
        ("getLanguage", |l| l.language.clone()),
        ("getCountry", |l| l.country.clone()),
        ("getVariant", |l| l.variant.clone()),
        ("getScript", |l| l.script.clone()),
        ("toString", Locale::to_java_string),
        ("toLanguageTag", Locale::to_language_tag),
        ("getDisplayName", Locale::display_name),
        ("getDisplayLanguage", Locale::display_language),
        ("getDisplayCountry", Locale::display_country),
        ("getDisplayVariant", |l| l.variant.clone()),
    ];
    for (name, f) in getters {
        b.method(name, move |c| {
            let l = self_locale(c)?;
            Ok(c.push(f(&l)))
        });
    }
    b.method("equals", |c| {
        let l = self_locale(c)?;
        Ok(c.push(locale_of(&c.arg(1)).is_some_and(|o| o == l)))
    });
}

// ---- Date ----------------------------------------------------------------------

/// A `java.util.Date` (milliseconds since the epoch). The class is not
/// exposed; Lua only passes these on and prints them.
#[derive(Debug)]
pub struct DateObj(pub i64);

impl LuaObject for DateObj {
    fn class_name(&self) -> &str {
        DATE
    }
    fn to_lua_string(&self) -> String {
        java_date::date_to_string(self.0, &default_zone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn date_value(ms: i64) -> Value {
    Value::Object(Rc::new(DateObj(ms)))
}

// ---- PZCalendar ----------------------------------------------------------------

/// `zombie.util.PZCalendar` over `Calendar.getInstance()`: a lenient
/// `GregorianCalendar` in the default zone with the default locale's week
/// rules.
pub struct PZCalendar {
    time: Cell<i64>,
    zone: JavaZone,
    locale: Locale,
}

impl LuaObject for PZCalendar {
    fn class_name(&self) -> &str {
        PZ_CALENDAR
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl PZCalendar {
    /// `PZCalendar.getInstance()`: the current time.
    pub fn now() -> PZCalendar {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        PZCalendar { time: Cell::new(ms), zone: default_zone(), locale: Locale::default_locale() }
    }

    /// `get(field)` for `Calendar`'s 17 fields.
    pub fn get(&self, field: i32) -> Result<i64, String> {
        let t = self.time.get();
        let f = java_date::Fields::of(t, &self.zone, &self.locale);
        let ad = f.year >= 1;
        let dst = self.zone.is_dst_at(t);
        Ok(match field {
            0 => i64::from(ad),
            1 => {
                if ad {
                    f.year
                } else {
                    1 - f.year
                }
            }
            2 => f.month.into(),
            3 => f.week_of_year.into(),
            4 => f.week_of_month.into(),
            5 => f.day.into(),
            6 => f.day_of_year.into(),
            7 => f.day_of_week.into(),
            8 => ((f.day - 1) / 7 + 1).into(),
            9 => i64::from(f.hour >= 12),
            10 => (f.hour % 12).into(),
            11 => f.hour.into(),
            12 => f.minute.into(),
            13 => f.second.into(),
            14 => f.milli.into(),
            15 => f.offset_ms - if dst { 3_600_000 } else { 0 },
            16 => {
                if dst {
                    3_600_000
                } else {
                    0
                }
            }
            _ => return Err(format!("Index {field} out of bounds for length 17")),
        })
    }

    /// `set(year, month, dayOfMonth, hourOfDay, minute)`: seconds and
    /// milliseconds stay; out-of-range values roll over (lenient).
    pub fn set(&self, year: i32, month: i32, day: i32, hour: i32, minute: i32) {
        let f = calendar_fields(&self.zone, self.time.get());
        self.time.set(calendar_millis(&self.zone, year, month, day, hour, minute, f.second, f.millisecond));
    }
}

/// `GregorianCalendar.isLeapYear(year)`: Julian rule up to the 1582
/// cutover year.
pub fn is_leap_year(year: i32) -> bool {
    if year & 3 != 0 {
        return false;
    }
    if year > 1582 {
        return year % 100 != 0 || year % 400 == 0;
    }
    true
}

fn self_calendar(c: &NativeCall<'_>) -> LuaResult<Rc<dyn LuaObject>> {
    this::<PZCalendar>(c)
}

fn with_calendar<R>(c: &NativeCall<'_>, f: impl FnOnce(&PZCalendar) -> R) -> LuaResult<R> {
    let o = self_calendar(c)?;
    Ok(f(o.as_any().downcast_ref::<PZCalendar>().unwrap()))
}

fn register_calendar(thread: &mut Thread) {
    let b = expose_class(thread, PZ_CALENDAR, None);
    // `java.util.Calendar`'s public int constants, as Kahlua exposes them
    // on the class.
    let consts: &[(&str, i32)] = &[
        ("ERA", 0),
        ("YEAR", 1),
        ("MONTH", 2),
        ("WEEK_OF_YEAR", 3),
        ("WEEK_OF_MONTH", 4),
        ("DATE", 5),
        ("DAY_OF_MONTH", 5),
        ("DAY_OF_YEAR", 6),
        ("DAY_OF_WEEK", 7),
        ("DAY_OF_WEEK_IN_MONTH", 8),
        ("AM_PM", 9),
        ("HOUR", 10),
        ("HOUR_OF_DAY", 11),
        ("MINUTE", 12),
        ("SECOND", 13),
        ("MILLISECOND", 14),
        ("ZONE_OFFSET", 15),
        ("DST_OFFSET", 16),
        ("FIELD_COUNT", 17),
        ("SUNDAY", 1),
        ("MONDAY", 2),
        ("TUESDAY", 3),
        ("WEDNESDAY", 4),
        ("THURSDAY", 5),
        ("FRIDAY", 6),
        ("SATURDAY", 7),
        ("JANUARY", 0),
        ("FEBRUARY", 1),
        ("MARCH", 2),
        ("APRIL", 3),
        ("MAY", 4),
        ("JUNE", 5),
        ("JULY", 6),
        ("AUGUST", 7),
        ("SEPTEMBER", 8),
        ("OCTOBER", 9),
        ("NOVEMBER", 10),
        ("DECEMBER", 11),
        ("UNDECIMBER", 12),
        ("AM", 0),
        ("PM", 1),
        ("ALL_STYLES", 0),
        ("SHORT", 1),
        ("LONG", 2),
        ("NARROW_FORMAT", 4),
        ("NARROW_STANDALONE", 32772),
        ("SHORT_FORMAT", 1),
        ("LONG_FORMAT", 2),
        ("SHORT_STANDALONE", 32769),
        ("LONG_STANDALONE", 32770),
    ];
    for (n, v) in consts {
        b.field(n, f64::from(*v));
    }
    b.static_fn("getInstance", |c| Ok(c.push(Value::Object(Rc::new(PZCalendar::now())))));
    b.method("get", |c| {
        let field = int_arg(c, 1);
        let r = with_calendar(c, |cal| cal.get(field))?
            .map_err(|m| LuaError::Runtime { message: m, class: "java.lang.ArrayIndexOutOfBoundsException" })?;
        Ok(c.push(r as f64))
    });
    b.method("set", |c| {
        let a: Vec<i32> = (1..=5).map(|i| int_arg(c, i)).collect();
        with_calendar(c, |cal| cal.set(a[0], a[1], a[2], a[3], a[4]))?;
        Ok(0)
    });
    b.method("setTimeInMillis", |c| {
        // `long` parameter: Kahlua converts the Lua double with `longValue`.
        let ms = num_arg(c, 1) as i64;
        with_calendar(c, |cal| cal.time.set(ms))?;
        Ok(0)
    });
    b.method("getTime", |c| {
        let ms = with_calendar(c, |cal| cal.time.get())?;
        Ok(c.push(date_value(ms)))
    });
    b.method("getTimeInMillis", |c| {
        let ms = with_calendar(c, |cal| cal.time.get())?;
        Ok(c.push(ms as f64))
    });
    b.method("isLeapYear", |c| {
        self_calendar(c)?;
        Ok(c.push(is_leap_year(int_arg(c, 1))))
    });
    // Kahlua puts the class table under its simple name, `PZCalendar`;
    // LuaManager also sets the global `Calendar` to it.
    let t = b.statics.clone();
    thread.globals().set_str("Calendar", t);
}

// ---- SimpleDateFormat ----------------------------------------------------------

/// A `java.text.SimpleDateFormat`.
pub struct SimpleDateFormat {
    pattern: RefCell<String>,
    tokens: RefCell<Vec<Token>>,
    locale: Locale,
    zone: JavaZone,
    lenient: Cell<bool>,
}

impl SimpleDateFormat {
    /// `new SimpleDateFormat(pattern, locale)`.
    pub fn new(pattern: &str, locale: Locale) -> Result<SimpleDateFormat, String> {
        let tokens = java_date::compile(pattern)?;
        Ok(SimpleDateFormat {
            pattern: RefCell::new(pattern.to_string()),
            tokens: RefCell::new(tokens),
            locale,
            zone: default_zone(),
            lenient: Cell::new(true),
        })
    }

    /// `format(new Date(ms))`.
    pub fn format(&self, ms: i64) -> String {
        java_date::format(&self.tokens.borrow(), ms, &self.zone, &self.locale)
    }
}

impl LuaObject for SimpleDateFormat {
    fn class_name(&self) -> &str {
        SIMPLE_DATE_FORMAT
    }
    fn to_lua_string(&self) -> String {
        format!("SimpleDateFormat [locale: \"{}\", pattern: \"{}\"]", self.locale.display_name(), self.pattern.borrow())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn sdf_value(pattern: &str, locale: Locale) -> LuaResult<Value> {
    let f = SimpleDateFormat::new(pattern, locale).map_err(illegal)?;
    Ok(Value::Object(Rc::new(f)))
}

fn with_sdf<R>(c: &NativeCall<'_>, f: impl FnOnce(&SimpleDateFormat) -> R) -> LuaResult<R> {
    let o = this::<SimpleDateFormat>(c)?;
    Ok(f(o.as_any().downcast_ref::<SimpleDateFormat>().unwrap()))
}

/// A style argument (`FULL` 0 .. `SHORT` 3); others throw like
/// `DateFormat.get*Instance`.
fn style_arg(c: &NativeCall<'_>, i: usize, what: &str) -> LuaResult<usize> {
    let s = int_arg(c, i);
    if !(0..=3).contains(&s) {
        return Err(illegal(format!("Illegal {what} style {s}")));
    }
    Ok(s as usize)
}

fn locale_arg_or_default(c: &NativeCall<'_>, i: usize) -> LuaResult<Locale> {
    if c.nargs() <= i {
        return Ok(Locale::default_locale());
    }
    locale_of(&c.arg(i)).ok_or_else(npe)
}

fn register_simple_date_format(thread: &mut Thread) {
    let b = expose_class(thread, SIMPLE_DATE_FORMAT, None);
    let fields = [
        "ERA_FIELD",
        "YEAR_FIELD",
        "MONTH_FIELD",
        "DATE_FIELD",
        "HOUR_OF_DAY1_FIELD",
        "HOUR_OF_DAY0_FIELD",
        "MINUTE_FIELD",
        "SECOND_FIELD",
        "MILLISECOND_FIELD",
        "DAY_OF_WEEK_FIELD",
        "DAY_OF_YEAR_FIELD",
        "DAY_OF_WEEK_IN_MONTH_FIELD",
        "WEEK_OF_YEAR_FIELD",
        "WEEK_OF_MONTH_FIELD",
        "AM_PM_FIELD",
        "HOUR1_FIELD",
        "HOUR0_FIELD",
        "TIMEZONE_FIELD",
    ];
    for (i, n) in fields.iter().enumerate() {
        b.field(n, i as f64);
    }
    for (n, v) in [("FULL", 0.0), ("LONG", 1.0), ("MEDIUM", 2.0), ("SHORT", 3.0), ("DEFAULT", 2.0)] {
        b.field(n, v);
    }
    // new SimpleDateFormat(), (pattern), (pattern, locale).
    b.ctor(|c| {
        let v = match c.nargs() {
            0 => {
                let l = Locale::default_locale();
                sdf_value(&l.date_time_pattern(3, 3), l)?
            }
            n => {
                let p = str_arg(c, 0).ok_or_else(npe)?;
                let l = if n >= 2 { locale_of(&c.arg(1)).ok_or_else(npe)? } else { Locale::default_locale() };
                sdf_value(&p, l)?
            }
        };
        Ok(c.push(v))
    });
    b.static_fn("getInstance", |c| {
        let l = Locale::default_locale();
        let v = sdf_value(&l.date_time_pattern(3, 3), l)?;
        Ok(c.push(v))
    });
    b.static_fn("getDateInstance", |c| {
        let style = if c.nargs() >= 1 { style_arg(c, 0, "date")? } else { 2 };
        let l = locale_arg_or_default(c, 1)?;
        let v = sdf_value(&l.date_pattern(style), l)?;
        Ok(c.push(v))
    });
    b.static_fn("getTimeInstance", |c| {
        let style = if c.nargs() >= 1 { style_arg(c, 0, "time")? } else { 2 };
        let l = locale_arg_or_default(c, 1)?;
        let v = sdf_value(&l.time_pattern(style), l)?;
        Ok(c.push(v))
    });
    b.static_fn("getDateTimeInstance", |c| {
        let (d, t) = if c.nargs() >= 2 { (style_arg(c, 0, "date")?, style_arg(c, 1, "time")?) } else { (2, 2) };
        let l = locale_arg_or_default(c, 2)?;
        let v = sdf_value(&l.date_time_pattern(d, t), l)?;
        Ok(c.push(v))
    });
    b.method("format", |c| {
        let ms = match c.arg(1) {
            Value::Object(o) => match o.as_any().downcast_ref::<DateObj>() {
                Some(d) => d.0,
                None => return Err(illegal("Cannot format given Object as a Date")),
            },
            // `DateFormat.format(Object)`: a Number becomes
            // `new Date(number.longValue())`.
            Value::Num(n) => n as i64,
            Value::Nil => return Err(npe()),
            _ => return Err(illegal("Cannot format given Object as a Date")),
        };
        let s = with_sdf(c, |f| f.format(ms))?;
        Ok(c.push(s))
    });
    for name in ["toPattern", "toLocalizedPattern"] {
        b.method(name, |c| {
            let p = with_sdf(c, |f| f.pattern.borrow().clone())?;
            Ok(c.push(p))
        });
    }
    for name in ["applyPattern", "applyLocalizedPattern"] {
        b.method(name, |c| {
            let p = str_arg(c, 1).ok_or_else(npe)?;
            let tokens = java_date::compile(&p).map_err(illegal)?;
            with_sdf(c, |f| {
                *f.tokens.borrow_mut() = tokens;
                *f.pattern.borrow_mut() = p.to_string();
            })?;
            Ok(0)
        });
    }
    b.method("setLenient", |c| {
        let v = bool_arg(c, 1);
        with_sdf(c, |f| f.lenient.set(v))?;
        Ok(0)
    });
    b.method("isLenient", |c| {
        let v = with_sdf(c, |f| f.lenient.get())?;
        Ok(c.push(v))
    });
    b.method("toString", |c| {
        let s = with_sdf(c, SimpleDateFormat::to_lua_string)?;
        Ok(c.push(s))
    });
    b.method("hashCode", |c| {
        let h = with_sdf(c, |f| java_date::java_string_hash(&f.pattern.borrow()))?;
        Ok(c.push(f64::from(h)))
    });
    b.method("equals", |c| {
        let me = with_sdf(c, |f| (f.pattern.borrow().clone(), f.locale.clone(), f.lenient.get()))?;
        let other = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<SimpleDateFormat>()
                .map(|f| (f.pattern.borrow().clone(), f.locale.clone(), f.lenient.get())),
            _ => None,
        };
        Ok(c.push(other.is_some_and(|o| o == me)))
    });
    b.method("clone", |c| {
        let v = with_sdf(c, |f| SimpleDateFormat {
            pattern: RefCell::new(f.pattern.borrow().clone()),
            tokens: RefCell::new(f.tokens.borrow().clone()),
            locale: f.locale.clone(),
            zone: f.zone.clone(),
            lenient: Cell::new(f.lenient.get()),
        })?;
        Ok(c.push(Value::Object(Rc::new(v))))
    });
}

pub fn register(thread: &mut Thread) {
    register_locale(thread);
    register_calendar(thread);
    register_simple_date_format(thread);
}

#[cfg(test)]
mod tests;
