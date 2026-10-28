use super::*;

fn chicago() -> JavaZone {
    JavaZone::named("America/Chicago").unwrap()
}

fn fmt(p: &str, t: i64, l: &Locale) -> String {
    format(&compile(p).unwrap(), t, &chicago(), l)
}

const T: i64 = 1_729_850_706_789; // 2024-10-25 05:05:06.789 CDT, a Friday

#[test]
fn shipped_pattern() {
    // MapSpawnSelect: SimpleDateFormat("yyyy-MM-dd_HH-mm-ss", Locale.ENGLISH).
    let en = Locale::of("en", "", "");
    assert_eq!(fmt("yyyy-MM-dd_HH-mm-ss", T, &en), "2024-10-25_05-05-06");
    assert_eq!(fmt("yyyy-MM-dd_HH-mm-ss", 0, &en), "1969-12-31_18-00-00");
}

#[test]
fn letters_like_the_jre() {
    // Expected strings are the bundled JRE's output (-Duser.timezone=America/Chicago).
    let en = Locale::of("en", "", "");
    let gb = Locale::of("en", "GB", "");
    let cases: &[(&str, i64, &Locale, &str)] = &[
        ("y yy yyy yyyy yyyyy", T, &en, "2024 24 2024 2024 02024"),
        ("y yy yyy yyyy yyyyy", 978_307_200_000, &en, "2000 00 2000 2000 02000"),
        ("M MM MMM MMMM MMMMM L LL LLL LLLL", 1_710_054_000_000, &en, "3 03 Mar March March 3 03 Mar March"),
        (
            "d dd D DDD F E EE EEE EEEE EEEEE u uu",
            1_710_054_000_000,
            &en,
            "10 10 70 070 2 Sun Sun Sun Sunday Sunday 7 07",
        ),
        (
            "H HH k kk K KK h hh a aa m mm s ss S SS SSS SSSS",
            0,
            &en,
            "18 18 18 18 6 06 6 06 PM PM 0 00 0 00 0 00 000 0000",
        ),
        (
            "H HH k kk K KK h hh a aa m mm s ss S SS SSS SSSS",
            T,
            &en,
            "5 05 5 05 5 05 5 05 AM AM 5 05 6 06 789 789 789 0789",
        ),
        ("w ww W Y YY YYYY G GGGG", 1_735_689_599_000, &en, "1 01 5 2025 25 2025 AD AD"),
        ("w ww W Y YY YYYY G GGGG", 1_704_067_200_000, &en, "1 01 6 2024 24 2024 AD AD"),
        ("w ww W Y YY YYYY G GGGG", 1_704_067_200_000, &gb, "52 52 4 2023 23 2023 AD AD"),
        ("w ww W Y YY YYYY G GGGG", 1_710_054_000_000, &gb, "10 10 1 2024 24 2024 AD AD"),
        ("w ww W Y YY YYYY G GGGG", 978_307_200_000, &gb, "52 52 4 2000 00 2000 AD AD"),
        ("z zz zzz Z ZZ ZZZZZ X XX XXX", T, &en, "CDT CDT CDT -0500 -0500 -0500 -05 -0500 -05:00"),
        ("z Z X", 0, &en, "CST -0600 -06"),
        ("'it''s' '' 'o''clock' hh 'h' ''", T, &en, "it's ' o'clock 05 h '"),
    ];
    for (p, t, l, want) in cases {
        assert_eq!(&fmt(p, *t, l), want, "{p} at {t} in {l:?}");
    }
    assert_eq!(fmt("X", 0, &en), "-06");
    assert_eq!(format(&compile("X XXX").unwrap(), 0, &JavaZone::utc(), &en), "Z Z");
}

#[test]
fn compile_errors() {
    assert_eq!(compile("yyyy-MM-dd q"), Err("Illegal pattern character 'q'".into()));
    assert_eq!(compile("'unterminated"), Err("Unterminated quote".into()));
    assert_eq!(compile("abc"), Err("Illegal pattern character 'b'".into()));
    assert_eq!(compile("XXXX"), Err("invalid ISO 8601 format: length=4".into()));
    assert_eq!(
        compile("yyyy-MM").unwrap(),
        vec![Token::Field('y', 4), Token::Literal("-".into()), Token::Field('M', 2)]
    );
}

#[test]
fn date_to_string_like_java() {
    assert_eq!(date_to_string(T, &chicago()), "Fri Oct 25 05:05:06 CDT 2024");
    assert_eq!(date_to_string(0, &chicago()), "Wed Dec 31 18:00:00 CST 1969");
}

#[test]
fn locales() {
    let us = Locale::of("EN", "us", "");
    assert_eq!(
        (us.to_java_string(), us.to_language_tag(), us.display_name()),
        ("en_US".into(), "en-US".into(), "English (United States)".into())
    );
    let root = Locale::root();
    assert_eq!(
        (root.to_java_string(), root.to_language_tag(), root.display_name()),
        ("".into(), "und".into(), "".into())
    );
    let v = Locale::of("en", "US", "WIN");
    assert_eq!(
        (v.to_java_string(), v.to_language_tag(), v.display_name()),
        ("en_US_WIN".into(), "en-US-x-lvariant-WIN".into(), "English (United States, WIN)".into())
    );
    assert_eq!(Locale::for_language_tag("zz-Latn-QQ").to_java_string(), "zz_QQ_#Latn");
    assert_eq!(Locale::for_language_tag("en-US"), us);
    assert_eq!(Locale::of("xx", "YY", "").display_name(), "xx (YY)");
    assert_eq!(Locale::of("ko", "KR", "").display_name(), "Korean (South Korea)");
    // CLDR week rules.
    assert_eq!(Locale::of("en", "", "").week_rules(), (1, 1));
    assert_eq!(Locale::of("en", "GB", "").week_rules(), (2, 4));
    assert_eq!(Locale::of("pt", "PT", "").week_rules(), (1, 4));
    assert_eq!(Locale::of("ar", "EG", "").week_rules(), (7, 1));
    assert_eq!(Locale::of("en", "ZZ", "").week_rules(), (2, 1));
    // Style patterns.
    assert_eq!(us.date_time_pattern(3, 3), "M/d/yy, h:mm\u{202f}a");
    assert_eq!(Locale::of("en", "GB", "").date_time_pattern(0, 1), "EEEE, d MMMM y, HH:mm:ss z");
    assert_eq!(root.date_time_pattern(3, 2), "y-MM-dd HH:mm:ss");
    assert_eq!(Locale::of("en", "CA", "").date_pattern(3), "y-MM-dd");
    assert_eq!(java_string_hash("yyyy-MM-dd_HH-mm-ss") as u32, 0xb61f11ff);
}

/// `PZ_SDF_ORACLE=<file>`: lines `locale|pattern|millis|expected` from the
/// bundled JRE run with `-Duser.timezone=America/Chicago`; checks the
/// English locales and the root locale (other languages print localized
/// names, which are not modelled) for instants after 1883.
#[test]
fn oracle_file() {
    let Ok(path) = std::env::var("PZ_SDF_ORACLE") else { return };
    let text = std::fs::read_to_string(path).unwrap();
    let mut n = 0;
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() != 4 || !(parts[0].is_empty() || parts[0] == "en" || parts[0].starts_with("en_")) {
            continue;
        }
        let Ok(t) = parts[2].parse::<i64>() else { continue };
        if t < -2_717_640_000_000 || parts[1].contains("zzzz") {
            continue;
        }
        let mut it = parts[0].split('_');
        let l = Locale::of(it.next().unwrap_or(""), it.next().unwrap_or(""), "");
        assert_eq!(fmt(parts[1], t, &l), parts[3], "{line}");
        n += 1;
    }
    assert!(n > 100, "{n}");
}
