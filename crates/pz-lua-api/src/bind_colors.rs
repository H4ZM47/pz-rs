//! `zombie.core.Colors`, `Colors.ColNfo` and `Colors.ColorSet`: the named
//! colour tables (standard, game and colour-blind sets) Lua looks colours
//! up in (`namedColorToTable`, `Colors.GetColorByName`, ...).
//!
//! The constants come from `Colors.<clinit>` in the installed
//! `projectzomboid.jar`, read with `pz-classfile` like the `Color` and
//! `ImmutableColor` named colours in [`crate::bind_misc_classes`]; nothing
//! of them is shipped. Colours are shared `zombie.core.Color` objects, so
//! Lua sees the same object from `Colors.Red`, `GetColorByName("red")` and
//! `GetColorFromIndex`, and `AddGameColor` can update one in place.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pz_classfile::{Jar, Value as JValue};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};

use crate::bind_java_util::JavaList;
use crate::bind_misc_classes::Color;
use crate::expose::{enum_value, expose_class, expose_enum, int_arg, str_arg, with_this};

pub const COLORS: &str = "zombie.core.Colors";
pub const COL_NFO: &str = "zombie.core.Colors$ColNfo";
pub const COLOR_SET: &str = "zombie.core.Colors$ColorSet";
const COLORS_CLASS: &str = "zombie/core/Colors";
const COLOR_CLASS: &str = "zombie/core/Color";

/// `Colors.ColorSet`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSet {
    Game,
    Standard,
    ColorBlind,
}

impl ColorSet {
    const NAMES: [&'static str; 3] = ["Game", "Standard", "ColorBlind"];

    fn index(self) -> i32 {
        self as i32
    }

    fn from_name(n: &str) -> Option<ColorSet> {
        [ColorSet::Game, ColorSet::Standard, ColorSet::ColorBlind]
            .into_iter()
            .find(|s| Self::NAMES[s.index() as usize] == n)
    }
}

/// `Colors.ColNfo`.
pub struct ColNfo {
    pub color_set: ColorSet,
    pub name: String,
    pub hex: String,
    pub color: Rc<Color>,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub r_int: i32,
    pub g_int: i32,
    pub b_int: i32,
}

impl ColNfo {
    pub fn new(name: &str, color: Rc<Color>, color_set: ColorSet) -> ColNfo {
        let (r, g, b) = (color.r.get(), color.g.get(), color.b.get());
        // `(int)this.r * 255`: the cast binds first, so each is 0 or 255
        // (or a multiple of 255 for components above 1).
        let int = |v: f32| (v as i32).wrapping_mul(255);
        let (r_int, g_int, b_int) = (int(r), int(g), int(b));
        ColNfo {
            color_set,
            name: name.to_string(),
            hex: format!("#{}{}{}", hex2(r_int), hex2(g_int), hex2(b_int)),
            color,
            r,
            g,
            b,
            r_int,
            g_int,
            b_int,
        }
    }
}

/// `%02x` of an int (two's complement for negatives, as Java prints it).
fn hex2(v: i32) -> String {
    format!("{:02x}", v as u32)
}

impl LuaObject for ColNfo {
    fn class_name(&self) -> &str {
        COL_NFO
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The `Colors` instance `Colors.new()` returns (the class has only
/// statics).
struct ColorsObject;

impl LuaObject for ColorsObject {
    fn class_name(&self) -> &str {
        COLORS
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// One table set: standard and game colours, or the colour-blind ones.
#[derive(Default)]
struct Table {
    info_map: HashMap<String, Rc<ColNfo>>,
    colors: Vec<Rc<Color>>,
    color_map: HashMap<String, Rc<Color>>,
    /// The live `ArrayList<String>` the names getter returns.
    names: Option<Rc<JavaList>>,
    set: HashSet<String>,
}

impl Table {
    fn names(&mut self) -> Rc<JavaList> {
        self.names.get_or_insert_with(|| Rc::new(JavaList::new(Vec::new()))).clone()
    }
}

/// `Colors`' static tables.
#[derive(Default)]
pub struct Colors {
    main: Table,
    cb: Table,
    /// `infoList`, every colour of every set in registration order.
    info_list: Vec<Rc<ColNfo>>,
    /// The `public static final Color` fields, in declaration order.
    fields: Vec<(String, Rc<Color>)>,
}

/// Java `String.toLowerCase()` for the colour names.
fn lower(s: &str) -> String {
    s.to_lowercase()
}

impl Colors {
    /// `addColor(name, color)` (standard set).
    fn add_color(&mut self, name: &str, color: Rc<Color>) -> Rc<Color> {
        let nfo = Rc::new(ColNfo::new(name, color.clone(), ColorSet::Standard));
        self.main.info_map.insert(name.to_string(), nfo.clone());
        self.info_list.push(nfo);
        self.main.colors.push(color.clone());
        self.main.color_map.insert(lower(name), color.clone());
        self.main.names().items.borrow_mut().push(Value::str(name));
        self.main.set.insert(lower(name));
        color
    }

    /// `AddGameColor(name, color)`: updates an existing game colour in
    /// place, else registers a game colour (replacing a standard one of the
    /// same name in the info and colour maps).
    pub fn add_game_color(&mut self, name: &str, color: Rc<Color>) -> Rc<Color> {
        if let Some(nfo) = self.main.info_map.get(name) {
            if nfo.color_set == ColorSet::Game {
                let c = &nfo.color;
                c.r.set(color.r.get());
                c.g.set(color.g.get());
                c.b.set(color.b.get());
                c.a.set(color.a.get());
                return color;
            }
        }
        let nfo = Rc::new(ColNfo::new(name, color.clone(), ColorSet::Game));
        self.main.info_map.insert(name.to_string(), nfo.clone());
        self.info_list.push(nfo);
        self.main.color_map.insert(lower(name), color.clone());
        self.main.names().items.borrow_mut().push(Value::str(name));
        self.main.set.insert(lower(name));
        color
    }

    /// `addColorCB(name, color)`.
    fn add_color_cb(&mut self, name: &str, color: Rc<Color>) -> Rc<Color> {
        let nfo = Rc::new(ColNfo::new(name, color.clone(), ColorSet::ColorBlind));
        self.info_list.push(nfo.clone());
        self.cb.info_map.insert(name.to_string(), nfo);
        self.cb.colors.push(color.clone());
        self.cb.color_map.insert(lower(name), color.clone());
        self.cb.names().items.borrow_mut().push(Value::str(name));
        self.cb.set.insert(lower(name));
        color
    }

    /// Runs `Colors.<clinit>` as read from `jar`: each `addColor`,
    /// `AddGameColor` and `addColorCB` call in order, and the static fields
    /// they (or an alias such as `Fuchsia = Magenta`) are stored in.
    pub fn load(jar: &Jar) -> Result<Colors, String> {
        let cf = pz_classfile::load_class(jar, COLORS_CLASS).map_err(|e| format!("{e:?}"))?;
        let inits = pz_classfile::static_inits(&cf).map_err(|e| format!("{e:?}"))?;
        let mut c = Colors::default();
        for init in inits {
            if init.class != COLORS_CLASS {
                continue;
            }
            let color = match &init.value {
                JValue::Call { class, name, args, .. } if class == COLORS_CLASS => {
                    let (Some(JValue::Str(n)), Some(JValue::New { class: cc, args: ca })) = (args.first(), args.get(1))
                    else {
                        continue;
                    };
                    if cc != COLOR_CLASS {
                        continue;
                    }
                    let col =
                        Rc::new(new_color(ca).ok_or_else(|| format!("Colors.{}: bad Color arguments", init.field))?);
                    match name.as_str() {
                        "addColor" => c.add_color(n, col),
                        "AddGameColor" => c.add_game_color(n, col),
                        "addColorCB" => c.add_color_cb(n, col),
                        _ => continue,
                    }
                }
                JValue::Static { class, name } if class == COLORS_CLASS => {
                    match c.fields.iter().find(|(f, _)| f == name) {
                        Some((_, col)) => col.clone(),
                        None => continue,
                    }
                }
                _ => continue,
            };
            c.fields.push((init.field, color));
        }
        if c.main.colors.is_empty() || c.cb.colors.is_empty() {
            return Err("Colors.<clinit> registered no colours".into());
        }
        Ok(c)
    }
}

/// `new Color(r, g, b)` (unclamped, alpha 1) or `new Color(r, g, b, a)`
/// (clamped to 0..1).
fn new_color(args: &[JValue]) -> Option<Color> {
    let f: Vec<f32> = args
        .iter()
        .map(|v| match v {
            JValue::Float(f) => Some(*f as f32),
            JValue::Int(i) => Some(*i as f32),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let c = |v: f32| v.clamp(0.0, 1.0);
    match f.as_slice() {
        [r, g, b] => Some(Color::rgba(*r, *g, *b, 1.0)),
        [r, g, b, a] => Some(Color::rgba(c(*r), c(*g), c(*b), c(*a))),
        _ => None,
    }
}

thread_local! {
    static STATE: RefCell<Option<Colors>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Colors) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

/// `Colors.ColorExists(name) ? Colors.GetColorByName(name) : null` for Rust
/// callers (`None` when the tables are not loaded).
pub(crate) fn named_color(name: &str) -> Option<Rc<Color>> {
    let name = lower(name);
    with(|s| s.main.set.contains(&name).then(|| s.main.color_map.get(&name).cloned()).flatten()).flatten()
}

fn color_value(c: &Rc<Color>) -> Value {
    Value::Object(c.clone())
}

fn opt_color(c: Option<Rc<Color>>) -> Value {
    c.as_ref().map_or(Value::Nil, color_value)
}

fn nfo_value(n: &Rc<ColNfo>) -> Value {
    Value::Object(n.clone())
}

/// A `Color` argument as the shared object (identity matters for
/// `getNameFromColor` and `AddGameColor`).
fn color_arg(c: &NativeCall<'_>, i: usize) -> Option<Rc<Color>> {
    match c.arg(i) {
        Value::Object(o) => {
            let any: Rc<dyn std::any::Any> = o;
            any.downcast::<Color>().ok()
        }
        _ => None,
    }
}

fn npe(what: &str) -> LuaError {
    LuaError::Runtime { message: format!("{what} is null"), class: "java.lang.NullPointerException" }
}

fn name_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<String> {
    str_arg(c, i).map(|s| s.to_string()).ok_or_else(|| npe("name"))
}

fn index_error(i: i32, len: usize) -> LuaError {
    LuaError::Runtime {
        message: format!("Index {i} out of bounds for length {len}"),
        class: "java.lang.IndexOutOfBoundsException",
    }
}

fn get_index<T: Clone>(v: &[T], i: i32) -> LuaResult<T> {
    usize::try_from(i).ok().and_then(|u| v.get(u).cloned()).ok_or_else(|| index_error(i, v.len()))
}

fn table(c: &mut Colors, cb: bool) -> &mut Table {
    if cb {
        &mut c.cb
    } else {
        &mut c.main
    }
}

fn register_table_fns(b: &crate::expose::ClassBuilder, prefix: &str, cb: bool) {
    let n = |s: &str| format!("{prefix}{s}");
    b.static_fn(&n("GetRandomColor"), move |c| {
        let col = with(|s| {
            let t = table(s, cb);
            let i = pz_core::random::next_range(0, t.colors.len() as i32 - 1);
            get_index(&t.colors, i)
        });
        Ok(c.push(match col {
            Some(r) => color_value(&r?),
            None => Value::Nil,
        }))
    });
    b.static_fn(&n("GetColorFromIndex"), move |c| {
        let i = int_arg(c, 0);
        let col = with(|s| get_index(&table(s, cb).colors, i)).transpose()?;
        Ok(c.push(opt_color(col)))
    });
    b.static_fn(&n("GetColorNameFromIndex"), move |c| {
        let i = int_arg(c, 0);
        let name = with(|s| {
            let names = table(s, cb).names();
            let items = names.items.borrow();
            get_index(&items, i)
        })
        .transpose()?;
        Ok(c.push(name.unwrap_or_default()))
    });
    b.static_fn(&n("GetColorsCount"), move |c| {
        let n = with(|s| table(s, cb).colors.len()).unwrap_or(0);
        Ok(c.push(n as f64))
    });
    b.static_fn(&n("GetColorByName"), move |c| {
        let name = lower(&name_arg(c, 0)?);
        let col = with(|s| table(s, cb).color_map.get(&name).cloned()).flatten();
        Ok(c.push(opt_color(col)))
    });
    b.static_fn(&n("GetColorNames"), move |c| {
        let names = with(|s| table(s, cb).names());
        Ok(c.push(names.map_or(Value::Nil, |l| Value::Object(l))))
    });
    b.static_fn(&n("ColorExists"), move |c| {
        let name = lower(&name_arg(c, 0)?);
        let r = with(|s| table(s, cb).set.contains(&name)).unwrap_or(false);
        Ok(c.push(r))
    });
}

/// Exposes `Colors` over the tables read from `jar` (nothing is exposed
/// without the jar).
pub fn register(thread: &mut Thread, jar: Option<&Jar>) {
    let Some(jar) = jar else { return };
    match Colors::load(jar) {
        Ok(c) => register_with(thread, c),
        Err(e) => pz_core::debug::with(|d| {
            d.log(pz_core::debug::DebugType::General, &format!("Colors not loaded: {e}"));
        }),
    }
}

/// Exposes `Colors` over `colors`.
pub fn register_with(thread: &mut Thread, colors: Colors) {
    let fields: Vec<(String, Rc<Color>)> = colors.fields.clone();
    STATE.with(|s| *s.borrow_mut() = Some(colors));

    let set = expose_enum(thread, COLOR_SET, &ColorSet::NAMES);
    set.method("getIndex", |c| {
        let i = with_this::<crate::expose::JavaEnum, _>(c, |e| e.ordinal)?;
        Ok(c.push(f64::from(i)))
    });

    let nfo = expose_class(thread, COL_NFO, None);
    nfo.ctor(|c| {
        let name = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_else(|| "null".into());
        let color = color_arg(c, 1).ok_or_else(|| npe("color"))?;
        let set = match c.arg(2) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<crate::expose::JavaEnum>()
                .filter(|e| e.class == COLOR_SET)
                .and_then(|e| ColorSet::from_name(e.name)),
            _ => None,
        };
        let set = set.ok_or_else(|| npe("colorSet"))?;
        Ok(c.push(nfo_value(&Rc::new(ColNfo::new(&name, color, set)))))
    });
    macro_rules! getter {
        ($name:literal, $f:expr) => {
            nfo.method($name, |c| {
                let v = with_this::<ColNfo, _>(c, $f)?;
                Ok(c.push(v))
            });
        };
    }
    getter!("getColorSet", |n: &ColNfo| enum_value(COLOR_SET, ColorSet::NAMES[n.color_set.index() as usize]));
    getter!("getColorSetIndex", |n: &ColNfo| f64::from(n.color_set.index()));
    getter!("getName", |n: &ColNfo| Value::str(&n.name));
    getter!("getHex", |n: &ColNfo| Value::str(&n.hex));
    getter!("getColor", |n: &ColNfo| color_value(&n.color));
    getter!("getR", |n: &ColNfo| f64::from(n.r));
    getter!("getG", |n: &ColNfo| f64::from(n.g));
    getter!("getB", |n: &ColNfo| f64::from(n.b));
    getter!("getRInt", |n: &ColNfo| f64::from(n.r_int));
    getter!("getGInt", |n: &ColNfo| f64::from(n.g_int));
    getter!("getBInt", |n: &ColNfo| f64::from(n.b_int));

    let b = expose_class(thread, COLORS, None);
    for (name, col) in &fields {
        b.field(name, color_value(col));
    }
    b.ctor(|c| Ok(c.push(Value::Object(Rc::new(ColorsObject)))));
    register_table_fns(&b, "", false);
    register_table_fns(&b, "CB_", true);
    b.static_fn("AddGameColor", |c| {
        let name = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_else(|| "null".into());
        let color = color_arg(c, 1).ok_or_else(|| npe("color"))?;
        let r = with(|s| s.add_game_color(&name, color));
        Ok(c.push(opt_color(r)))
    });
    b.static_fn("GetColorInfo", |c| {
        let name = str_arg(c, 0).map(|s| s.to_string());
        let r = name.and_then(|n| with(|s| s.main.info_map.get(&n).cloned()).flatten());
        Ok(c.push(r.as_ref().map_or(Value::Nil, nfo_value)))
    });
    b.static_fn("getNameFromColor", |c| {
        let color = color_arg(c, 0);
        let name = with(|s| {
            s.main
                .info_map
                .values()
                .find(|n| color.as_ref().is_some_and(|col| Rc::ptr_eq(&n.color, col)))
                .map(|n| n.name.clone())
        })
        .flatten();
        Ok(c.push(name.unwrap_or_default()))
    });
}

#[cfg(test)]
mod tests;
