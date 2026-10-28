use super::*;
use std::fmt::Write as _;

fn rgb(r: f32, g: f32, b: f32) -> Rc<Color> {
    Rc::new(Color::rgba(r, g, b, 1.0))
}

/// A small set registered the way `<clinit>` does.
fn sample() -> Colors {
    let mut c = Colors::default();
    let add = |field: &str, col: Rc<Color>, c: &mut Colors| c.fields.push((field.to_string(), col));
    let x = c.add_game_color("UI_Background", rgb(0.0, 0.0, 0.0));
    add("UI_Background", x, &mut c);
    let x = c.add_color_cb("SherwoodGreen", rgb(0.0, 0.239, 0.188));
    add("CB_G0_SherwoodGreen", x, &mut c);
    let x = c.add_color("White", rgb(1.0, 1.0, 1.0));
    add("White", x, &mut c);
    let x = c.add_color("Silver", rgb(0.753, 0.753, 0.753));
    add("Silver", x, &mut c);
    let x = c.add_color("Magenta", rgb(1.0, 0.0, 1.0));
    add("Magenta", x.clone(), &mut c);
    add("Fuchsia", x, &mut c);
    c
}

fn lua() -> pz_kahlua::Thread {
    let mut t = pz_kahlua::Thread::new(pz_kahlua::TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    crate::bind_misc_classes::register(&mut t, None);
    t
}

fn run(t: &mut pz_kahlua::Thread, src: &str) -> Vec<Value> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
    t.call(&f, &[]).unwrap()
}

#[test]
fn info_hex_uses_the_int_cast_quirk() {
    let n = ColNfo::new("Silver", rgb(0.753, 0.753, 0.753), ColorSet::Standard);
    assert_eq!((n.r_int, n.hex.as_str()), (0, "#000000"));
    let n = ColNfo::new("Magenta", rgb(1.0, 0.0, 1.0), ColorSet::Standard);
    assert_eq!((n.r_int, n.g_int, n.hex.as_str()), (255, 0, "#ff00ff"));
}

#[test]
fn lookups_from_lua() {
    let mut t = lua();
    register_with(&mut t, sample());
    let r = run(
        &mut t,
        r#"
        local w = Colors.GetColorByName("WHITE")
        return w == Colors.White, Colors.Fuchsia == Colors.Magenta, Colors.GetColorByName("nope") == nil,
            Colors.GetColorsCount(), Colors.GetColorNames():size(), Colors.GetColorNameFromIndex(0),
            Colors.GetColorFromIndex(0) == Colors.White, Colors.ColorExists("silver"), Colors.CB_ColorExists("white"),
            Colors.CB_GetColorByName("sherwoodgreen") == Colors.CB_G0_SherwoodGreen, Colors.CB_GetColorsCount(),
            Colors.getNameFromColor(Colors.Fuchsia), Colors.getNameFromColor(Colors.CB_G0_SherwoodGreen),
            Colors.GetColorInfo("Magenta"):getHex(), Colors.GetColorInfo("UI_Background"):getColorSet() == ColorSet.Game,
            Colors.GetColorInfo("SherwoodGreen") == nil, Colors.GetColorInfo("Silver"):getColorSetIndex()
    "#,
    );
    let b = |i: usize| r[i].truthy();
    assert!(b(0) && b(1) && b(2), "{r:?}");
    assert_eq!((r[3].as_num(), r[4].as_num()), (Some(3.0), Some(4.0)));
    // Names include the game colour, the colour list does not.
    assert_eq!(r[5].as_str(), Some("UI_Background"));
    assert!(b(6) && b(7) && !b(8) && b(9));
    assert_eq!(r[10].as_num(), Some(1.0));
    assert_eq!(r[11].as_str(), Some("Magenta"));
    assert_eq!(r[12].as_str(), Some(""));
    assert_eq!(r[13].as_str(), Some("#ff00ff"));
    assert!(b(14) && b(15));
    assert_eq!(r[16].as_num(), Some(1.0));
}

#[test]
fn add_game_color_updates_or_registers() {
    let mut t = lua();
    register_with(&mut t, sample());
    let r = run(
        &mut t,
        r#"
        local bg = Colors.UI_Background
        local c = Color.new(0.5, 0.25, 0.125)
        local ret = Colors.AddGameColor("UI_Background", c)
        local names = Colors.GetColorNames()
        local before = names:size()
        local d = Color.new(0.1, 0.2, 0.3)
        Colors.AddGameColor("Silver", d)
        return ret == c, bg:getRedFloat(), Colors.GetColorByName("ui_background") == bg, before, names:size(),
            Colors.GetColorByName("silver") == d, Colors.GetColorInfo("Silver"):getColorSet() == ColorSet.Game,
            Colors.GetColorFromIndex(1) == Colors.Silver, Colors.GetColorsCount()
    "#,
    );
    assert!(r[0].truthy());
    // An existing game colour is updated in place.
    assert_eq!(r[1].as_num(), Some(0.5));
    assert!(r[2].truthy());
    // A standard colour of the same name is shadowed by a new game colour
    // (and its name listed twice), but stays in the colour list.
    assert_eq!((r[3].as_num(), r[4].as_num()), (Some(4.0), Some(5.0)));
    assert!(r[5].truthy() && r[6].truthy() && r[7].truthy());
    assert_eq!(r[8].as_num(), Some(3.0));
}

#[test]
fn index_errors_and_null_names() {
    let mut t = lua();
    register_with(&mut t, sample());
    let f =
        pz_kahlua::stdlib::load_string("return Colors.GetColorFromIndex(9)", Some("t"), t.globals().clone()).unwrap();
    let e = t.call(&f, &[]).unwrap_err();
    assert!(format!("{e:?}").contains("IndexOutOfBounds"), "{e:?}");
    let f =
        pz_kahlua::stdlib::load_string("return Colors.GetColorByName(nil)", Some("t"), t.globals().clone()).unwrap();
    assert!(t.call(&f, &[]).is_err());
}

fn jf(v: f32) -> String {
    pz_core::config::float_to_string(v)
}

fn col(v: &Value) -> String {
    let [r, g, b, a] = crate::bind_misc_classes::color_of(v).unwrap();
    format!("{},{},{},{}", jf(r), jf(g), jf(b), jf(a))
}

/// The game's `Colors` from its jar; with `PZ_COLORS_DUMP=<file>`, written
/// in the format of the reflection dump of the original class for diffing.
#[test]
fn game_jar_colors() {
    let Some(game) = std::env::var_os("PZ_GAME_DIR") else { return };
    let jar = Jar::open(std::path::PathBuf::from(game).join("projectzomboid.jar")).unwrap();
    let colors = Colors::load(&jar).unwrap();
    let fields: Vec<String> = colors.fields.iter().map(|f| f.0.clone()).collect();
    assert!(fields.iter().any(|f| f == "Fuchsia") && colors.main.colors.len() > 100);
    let mut t = lua();
    register_with(&mut t, colors);
    let Some(out) = std::env::var_os("PZ_COLORS_DUMP") else { return };
    let colors_t = t.globals().get_str("Colors").as_table().unwrap().clone();
    let mut call = |name: &str, args: &[Value]| -> Value {
        let f = colors_t.get_str(name);
        t.call(&f, args).unwrap().into_iter().next().unwrap_or_default()
    };
    let mut s = String::new();
    for p in ["", "CB_"] {
        let names = call(&format!("{p}GetColorNames"), &[]);
        let names: Vec<String> = match &names {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaList>()
                .unwrap()
                .items
                .borrow()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect(),
            _ => panic!(),
        };
        let n = call(&format!("{p}GetColorsCount"), &[]).as_num().unwrap() as usize;
        writeln!(s, "{p}count {n} names {}", names.len()).unwrap();
        for i in 0..n {
            let c = call(&format!("{p}GetColorFromIndex"), &[(i as f64).into()]);
            let nm = call(&format!("{p}GetColorNameFromIndex"), &[(i as f64).into()]);
            writeln!(s, "{p}I {i} {} {}", col(&c), nm.as_str().unwrap()).unwrap();
        }
        for nm in &names {
            let c = call(&format!("{p}GetColorByName"), &[Value::str(nm)]);
            let e = call(&format!("{p}ColorExists"), &[Value::str(&nm.to_uppercase())]);
            writeln!(s, "{p}N {nm} {} {}", col(&c), e.truthy()).unwrap();
        }
    }
    for f in fields {
        let c = colors_t.get_str(&f);
        let name = call("getNameFromColor", std::slice::from_ref(&c));
        let name = name.as_str().unwrap().to_string();
        let nfo = call("GetColorInfo", &[Value::str(&name)]);
        let info = match &nfo {
            Value::Object(o) => {
                let n = o.as_any().downcast_ref::<ColNfo>().unwrap();
                format!("{} {} {}", n.hex, ColorSet::NAMES[n.color_set.index() as usize], n.r_int)
            }
            _ => "null".into(),
        };
        writeln!(s, "F {f} {} {name} {info}", col(&c)).unwrap();
    }
    std::fs::write(out, s).unwrap();
}
