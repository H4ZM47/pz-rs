use super::*;

fn lua() -> Thread {
    pz_core::weather::calendar::set_default_zone(JavaZone::named("America/Chicago"));
    let mut t = Thread::new(pz_kahlua::TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    register(&mut t);
    t
}

fn run(t: &mut Thread, src: &str) -> Result<Vec<Value>, LuaError> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
    t.call(&f, &[])
}

#[test]
fn map_spawn_select_save_name() {
    let mut t = lua();
    // MapSpawnSelect.lua's default save name, on a fixed instant.
    let r = run(
        &mut t,
        r#"
        local sdf = SimpleDateFormat.new("yyyy-MM-dd_HH-mm-ss", Locale.ENGLISH);
        local cal = Calendar.getInstance()
        cal:setTimeInMillis(1729850706789)
        return sdf:format(cal:getTime()), tostring(cal:getTime()), tostring(sdf), sdf:toPattern(),
            sdf:format(1729850706789.9), Calendar == PZCalendar
    "#,
    )
    .unwrap();
    assert_eq!(r[0].as_str(), Some("2024-10-25_05-05-06"));
    assert_eq!(r[1].as_str(), Some("Fri Oct 25 05:05:06 CDT 2024"));
    assert_eq!(r[2].as_str(), Some("SimpleDateFormat [locale: \"English\", pattern: \"yyyy-MM-dd_HH-mm-ss\"]"));
    assert_eq!(r[3].as_str(), Some("yyyy-MM-dd_HH-mm-ss"));
    assert_eq!(r[4].as_str(), Some("2024-10-25_05-05-06"));
    assert!(r[5].truthy());
    // The real clock gives a well-formed name.
    let r = run(&mut t, r#"return SimpleDateFormat.new("yyyy-MM-dd_HH-mm-ss", Locale.ENGLISH):format(Calendar.getInstance():getTime())"#)
        .unwrap();
    let s = r[0].as_str().unwrap();
    assert_eq!(s.len(), 19);
    assert!(s.starts_with("20") && &s[10..11] == "_");
}

#[test]
fn calendar_fields_and_set() {
    let mut t = lua();
    let r = run(
        &mut t,
        r#"
        local c = Calendar.getInstance()
        c:setTimeInMillis(1729850706789)
        local before = {c:get(Calendar.YEAR), c:get(Calendar.MONTH), c:get(Calendar.DATE), c:get(Calendar.HOUR_OF_DAY),
            c:get(Calendar.HOUR), c:get(Calendar.AM_PM), c:get(Calendar.DAY_OF_WEEK), c:get(Calendar.DAY_OF_YEAR),
            c:get(Calendar.ERA), c:get(Calendar.ZONE_OFFSET), c:get(Calendar.DST_OFFSET), c:get(Calendar.MILLISECOND)}
        c:set(2024, 13, 1, 25, 61)
        return before, c:getTimeInMillis(), c:isLeapYear(2000), c:isLeapYear(1900), c:isLeapYear(1500), c:isLeapYear(1582)
    "#,
    )
    .unwrap();
    let before = r[0].as_table().unwrap();
    let got: Vec<f64> = (1..=12).map(|i| before.get_int(i).as_num().unwrap()).collect();
    assert_eq!(got, [2024.0, 9.0, 25.0, 5.0, 5.0, 0.0, 6.0, 299.0, 1.0, -21_600_000.0, 3_600_000.0, 789.0]);
    // Month 13, hour 25, minute 61 roll over: 2025-02-02 02:01:06.789 CST.
    let want =
        pz_core::calendar::calendar_millis(&JavaZone::named("America/Chicago").unwrap(), 2025, 1, 2, 2, 1, 6, 789);
    assert_eq!(r[1].as_num(), Some(want as f64));
    assert!(r[2].truthy() && !r[3].truthy() && r[4].truthy() && !r[5].truthy());
    let e = run(&mut t, "return Calendar.getInstance():get(17)").unwrap_err();
    assert!(format!("{e:?}").contains("ArrayIndexOutOfBounds"), "{e:?}");
}

#[test]
fn simple_date_format_api() {
    let mut t = lua();
    let r = run(
        &mut t,
        r#"
        local f = SimpleDateFormat.new("EEE, d MMM yyyy", Locale.UK)
        local g = f:clone()
        f:applyPattern("hh 'o''clock' a")
        local us = SimpleDateFormat.getDateTimeInstance(SimpleDateFormat.SHORT, SimpleDateFormat.SHORT, Locale.US)
        local gb = SimpleDateFormat.getDateInstance(SimpleDateFormat.FULL, Locale.UK)
        return f:format(0), g:format(0), us:toPattern(), gb:format(0), f:isLenient(), g:equals(f), f:hashCode()
    "#,
    )
    .unwrap();
    assert_eq!(r[0].as_str(), Some("06 o'clock pm"));
    assert_eq!(r[1].as_str(), Some("Wed, 31 Dec 1969"));
    assert_eq!(r[2].as_str(), Some("M/d/yy, h:mm\u{202f}a"));
    assert_eq!(r[3].as_str(), Some("Wednesday, 31 December 1969"));
    assert!(r[4].truthy() && !r[5].truthy());
    assert_eq!(r[6].as_num(), Some(f64::from(java_date::java_string_hash("hh 'o''clock' a"))));
    for (src, class) in [
        ("SimpleDateFormat.new('yyyy q')", "IllegalArgumentException"),
        ("SimpleDateFormat.new(\"'x\")", "IllegalArgumentException"),
        ("SimpleDateFormat.new('yyyy'):format('x')", "IllegalArgumentException"),
        ("SimpleDateFormat.new('yyyy'):format(nil)", "NullPointerException"),
        ("SimpleDateFormat.getDateInstance(7)", "IllegalArgumentException"),
    ] {
        let e = run(&mut t, src).unwrap_err();
        assert!(format!("{e:?}").contains(class), "{src}: {e:?}");
    }
}

#[test]
fn locale_api() {
    let mut t = lua();
    let r = run(
        &mut t,
        r#"
        return tostring(Locale.US), Locale.US:toLanguageTag(), Locale.ROOT:toLanguageTag(), Locale.CHINA == Locale.PRC,
            Locale.CHINA == Locale.SIMPLIFIED_CHINESE, Locale.new("EN", "us"):equals(Locale.US), Locale.KOREA:getDisplayName(),
            Locale.of("en", "US", "WIN"):toString(), Locale.forLanguageTag("fr-CA"):equals(Locale.CANADA_FRENCH),
            Locale.TAIWAN:getCountry(), Locale.UK:getDisplayCountry()
    "#,
    )
    .unwrap();
    assert_eq!(r[0].as_str(), Some("en_US"));
    assert_eq!(r[1].as_str(), Some("en-US"));
    assert_eq!(r[2].as_str(), Some("und"));
    assert!(r[3].truthy() && r[4].truthy() && r[5].truthy());
    assert_eq!(r[6].as_str(), Some("Korean (South Korea)"));
    assert_eq!(r[7].as_str(), Some("en_US_WIN"));
    assert!(r[8].truthy());
    assert_eq!(r[9].as_str(), Some("TW"));
    assert_eq!(r[10].as_str(), Some("United Kingdom"));
}
