//! Small engine classes on the menu path that no subsystem owns:
//! `zombie.SystemDisabler`, `zombie.core.Color`, `zombie.iso.Vector2`,
//! `org.joml.Vector3f` and the `zombie.characters.Capability` enum.

use crate::bind_item_visual::{ImmutableColor, IMMUTABLE_COLOR_CLASS};
use crate::expose::{bool_arg, expose_class, expose_enum, num_arg, str_arg, with_this};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

fn f32_arg(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

fn is_num(v: &Value) -> bool {
    matches!(v, Value::Num(_))
}

fn npe() -> LuaError {
    LuaError::Runtime {
        message: "null".into(),
        class: "java.lang.NullPointerException",
    }
}

fn no_method(name: &str) -> LuaError {
    LuaError::Runtime {
        message: format!("No implementation found for {name}"),
        class: "java.lang.RuntimeException",
    }
}

// ---- zombie.SystemDisabler ----------------------------------------------------------

thread_local! {
    static DISABLER: Cell<[bool; 9]> = const { Cell::new(SYSTEM_DISABLER_DEFAULTS) };
}

/// doCharacterStats, doZombieCreation, doSurvivorCreation, doPlayerCreation,
/// doOverridePOVCharacters, doVehiclesEverywhere, doWorldSyncEnable,
/// doMainLoopDealWithNetData, enableAdvancedSoundOptions (static init).
const SYSTEM_DISABLER_DEFAULTS: [bool; 9] =
    [true, true, false, true, true, false, false, true, false];

thread_local! {
    static UNCAPPED_FPS: Cell<bool> = const { Cell::new(false) };
}

fn set_flag(i: usize, v: bool) {
    DISABLER.with(|d| {
        let mut a = d.get();
        a[i] = v;
        d.set(a);
    });
}

fn flag(i: usize) -> bool {
    DISABLER.with(|d| d.get()[i])
}

fn register_system_disabler(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.SystemDisabler", None);
    // Public static fields are copied into the class table once, as Kahlua does.
    for (i, name) in [
        "doCharacterStats",
        "doZombieCreation",
        "doSurvivorCreation",
        "doPlayerCreation",
        "doOverridePOVCharacters",
        "doVehiclesEverywhere",
        "doWorldSyncEnable",
    ]
    .into_iter()
    .enumerate()
    {
        b.field(name, flag(i));
    }
    for name in [
        "doEnableDetectOpenGLErrors",
        "doEnableDetectOpenGLErrorsInTexture",
        "doVehiclesWithoutTextures",
        "zombiesDontAttack",
    ] {
        b.field(name, false);
    }
    for (i, name) in [
        "setDoCharacterStats",
        "setDoZombieCreation",
        "setDoSurvivorCreation",
        "setDoPlayerCreation",
        "setOverridePOVCharacters",
        "setVehiclesEverywhere",
        "setWorldSyncEnable",
    ]
    .into_iter()
    .enumerate()
    {
        b.static_fn(name, move |c| {
            set_flag(i, bool_arg(c, 0));
            Ok(0)
        });
    }
    b.static_fn("getdoHighFriction", |c| Ok(c.push(false)));
    b.static_fn("getdoVehicleLowRider", |c| Ok(c.push(false)));
    b.static_fn("printDetailedInfo", |c| Ok(c.push(false)));
    b.static_fn("getDoMainLoopDealWithNetData", |c| Ok(c.push(flag(7))));
    b.static_fn("setEnableAdvancedSoundOptions", |c| {
        set_flag(8, bool_arg(c, 0));
        Ok(0)
    });
    b.static_fn("getEnableAdvancedSoundOptions", |c| Ok(c.push(flag(8))));
    b.static_fn("setUncappedFPS", |c| {
        UNCAPPED_FPS.with(|u| u.set(bool_arg(c, 0)));
        Ok(0)
    });
    b.static_fn("getUncappedFPS", |c| {
        Ok(c.push(UNCAPPED_FPS.with(Cell::get)))
    });
    b.static_fn("Reset", |_| {
        DISABLER.with(|d| d.set(SYSTEM_DISABLER_DEFAULTS));
        Ok(0)
    });
}

/// `SystemDisabler.getUncappedFPS()`.
pub fn uncapped_fps() -> bool {
    UNCAPPED_FPS.with(Cell::get)
}

// ---- zombie.core.Color ---------------------------------------------------------------------

/// `zombie.core.Color`.
#[derive(Default)]
pub struct Color {
    pub r: Cell<f32>,
    pub g: Cell<f32>,
    pub b: Cell<f32>,
    pub a: Cell<f32>,
}

impl LuaObject for Color {
    fn class_name(&self) -> &str {
        "zombie.core.Color"
    }
    fn to_lua_string(&self) -> String {
        let f = pz_core::config::float_to_string;
        format!(
            "Color ({},{},{},{})",
            f(self.r.get()),
            f(self.g.get()),
            f(self.b.get()),
            f(self.a.get())
        )
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `PZMath.clamp(v, 0, 1)` (NaN passes through).
#[allow(clippy::manual_clamp)]
pub(crate) fn clamp01(v: f32) -> f32 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

impl Color {
    pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color {
            r: Cell::new(r),
            g: Cell::new(g),
            b: Cell::new(b),
            a: Cell::new(a),
        }
    }

    fn set(&self, r: f32, g: f32, b: f32, a: f32) {
        self.r.set(r);
        self.g.set(g);
        self.b.set(b);
        self.a.set(a);
    }

    fn get(&self) -> [f32; 4] {
        [self.r.get(), self.g.get(), self.b.get(), self.a.get()]
    }

    /// `new Color(int value)` / `fromColor`: ABGR with alpha 0 read as 255.
    fn set_from_int(&self, v: i32) {
        let b = (v & 0xFF0000) >> 16;
        let g = (v & 0xFF00) >> 8;
        let r = v & 0xFF;
        let mut a = (v as u32 & 0xFF00_0000) as i32 >> 24;
        if a < 0 {
            a += 256;
        }
        if a == 0 {
            a = 255;
        }
        self.set(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        );
    }
}

/// `Color.colorToABGR(r, g, b, a)`.
pub fn color_to_abgr(r: f32, g: f32, b: f32, a: f32) -> i32 {
    let byte = |v: f32| (clamp01(v) * 255.0) as i32 & 0xFF;
    byte(a) << 24 | byte(b) << 16 | byte(g) << 8 | byte(r)
}

fn channel(v: i32, shift: u32) -> f32 {
    0.003921569f32 * ((v >> shift) & 0xFF) as f32
}

fn abgr_channels(v: i32) -> [f32; 4] {
    [channel(v, 0), channel(v, 8), channel(v, 16), channel(v, 24)]
}

/// A `zombie.core.Color` Lua object.
pub fn color_value_of(r: f32, g: f32, b: f32, a: f32) -> Value {
    color_value(Color::rgba(r, g, b, a))
}

/// The components of a `zombie.core.Color` value.
pub fn color_of(v: &Value) -> Option<[f32; 4]> {
    match v {
        Value::Object(o) => o.as_any().downcast_ref::<Color>().map(Color::get),
        _ => None,
    }
}

fn color_value(c: Color) -> Value {
    Value::Object(Rc::new(c))
}

fn color_arg(c: &NativeCall<'_>, i: usize) -> Option<[f32; 4]> {
    match c.arg(i) {
        Value::Object(o) => o.as_any().downcast_ref::<Color>().map(Color::get),
        _ => None,
    }
}

fn this_color<R>(c: &NativeCall<'_>, f: impl FnOnce(&Color) -> R) -> LuaResult<R> {
    with_this::<Color, _>(c, f)
}

/// The integer argument Kahlua passes for an `int` parameter.
fn int_of(c: &NativeCall<'_>, i: usize) -> i32 {
    num_arg(c, i) as i32
}

/// `Integer.decode(nm)`.
fn integer_decode(s: &str) -> Option<i32> {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (radix, digits) = if let Some(d) = rest
        .strip_prefix("0x")
        .or_else(|| rest.strip_prefix("0X"))
        .or_else(|| rest.strip_prefix('#'))
    {
        (16, d)
    } else if rest.len() > 1 && rest.starts_with('0') {
        (8, &rest[1..])
    } else {
        (10, rest)
    };
    if digits.is_empty() || digits.starts_with(['-', '+']) {
        return None;
    }
    let v = i64::from_str_radix(digits, radix).ok()?;
    let v = if neg { -v } else { v };
    i32::try_from(v).ok()
}

fn register_color(thread: &mut Thread, jar: Option<&pz_classfile::Jar>) {
    let b = expose_class(thread, "zombie.core.Color", None);
    // Named colours: read from Color.<clinit> in the installed jar.
    if let Some(jar) = jar {
        if let Ok(ctors) = pz_classfile::self_constructed_constants(jar, "zombie/core/Color") {
            for (name, args) in ctors {
                let nums: Vec<f64> = args
                    .iter()
                    .filter_map(|a| match a {
                        pz_classfile::Value::Int(i) => Some(*i as f64),
                        pz_classfile::Value::Float(f) => Some(*f),
                        _ => None,
                    })
                    .collect();
                let ints = args
                    .iter()
                    .all(|a| matches!(a, pz_classfile::Value::Int(_)));
                let col = Color::default();
                match (nums.len(), ints) {
                    (4, true) => col.set(
                        nums[0] as f32 / 255.0,
                        nums[1] as f32 / 255.0,
                        nums[2] as f32 / 255.0,
                        nums[3] as f32 / 255.0,
                    ),
                    (3, true) => col.set(
                        nums[0] as f32 / 255.0,
                        nums[1] as f32 / 255.0,
                        nums[2] as f32 / 255.0,
                        1.0,
                    ),
                    (4, false) => col.set(
                        clamp01(nums[0] as f32),
                        clamp01(nums[1] as f32),
                        clamp01(nums[2] as f32),
                        clamp01(nums[3] as f32),
                    ),
                    (3, false) => col.set(nums[0] as f32, nums[1] as f32, nums[2] as f32, 1.0),
                    _ => continue,
                }
                b.field(&name, color_value(col));
            }
        }
    }
    // Kahlua picks the overload with the most parameters, preferring float
    // over all-int signatures, so plain numbers reach the float constructors.
    b.ctor(|c| {
        let n = c.nargs();
        let col = Color::rgba(0.0, 0.0, 0.0, 1.0);
        match n {
            0 => {}
            1 => match c.arg(0) {
                Value::Object(o) => match o.as_any().downcast_ref::<Color>() {
                    Some(src) => col.set(src.r.get(), src.g.get(), src.b.get(), src.a.get()),
                    None => return Err(no_method("Color.new")),
                },
                Value::Nil => {}
                Value::Num(_) => col.set_from_int(int_of(c, 0)),
                _ => return Err(no_method("Color.new")),
            },
            3 if is_num(&c.arg(0)) => col.set(f32_arg(c, 0), f32_arg(c, 1), f32_arg(c, 2), 1.0),
            3 => {
                let (a, b2) = (
                    color_arg(c, 0).ok_or_else(npe)?,
                    color_arg(c, 1).ok_or_else(npe)?,
                );
                let d = f32_arg(c, 2);
                col.set(
                    a[0] + (b2[0] - a[0]) * d,
                    a[1] + (b2[1] - a[1]) * d,
                    a[2] + (b2[2] - a[2]) * d,
                    a[3] + (b2[3] - a[3]) * d,
                );
            }
            _ => col.set(
                clamp01(f32_arg(c, 0)),
                clamp01(f32_arg(c, 1)),
                clamp01(f32_arg(c, 2)),
                clamp01(f32_arg(c, 3)),
            ),
        }
        Ok(c.push(color_value(col)))
    });

    b.static_fn("colorToABGR", |c| {
        let v = match color_arg(c, 0) {
            Some([r, g, b, a]) => color_to_abgr(r, g, b, a),
            None => color_to_abgr(f32_arg(c, 0), f32_arg(c, 1), f32_arg(c, 2), f32_arg(c, 3)),
        };
        Ok(c.push(v as f64))
    });
    b.static_fn("abgrToColor", |c| {
        let [r, g, b, a] = abgr_channels(int_of(c, 0));
        let Value::Object(o) = c.arg(1) else {
            return Err(npe());
        };
        let col = o.as_any().downcast_ref::<Color>().ok_or_else(npe)?;
        col.set(r, g, b, a);
        Ok(c.push(c.arg(1)))
    });
    let blend = |name: &'static str,
                 f: fn([f32; 4], [f32; 4]) -> [f32; 4],
                 b: &crate::expose::ClassBuilder| {
        b.static_fn(name, move |c| {
            let [r, g, bl, a] = f(abgr_channels(int_of(c, 0)), abgr_channels(int_of(c, 1)));
            Ok(c.push(color_to_abgr(r, g, bl, a) as f64))
        });
    };
    blend(
        "multiplyABGR",
        |v, m| [v[0] * m[0], v[1] * m[1], v[2] * m[2], v[3] * m[3]],
        &b,
    );
    blend(
        "multiplyBGR",
        |v, m| [v[0] * m[0], v[1] * m[1], v[2] * m[2], v[3]],
        &b,
    );
    blend(
        "blendBGR",
        |v, t| {
            let ta = t[3];
            [
                v[0] * (1.0 - ta) + t[0] * ta,
                v[1] * (1.0 - ta) + t[1] * ta,
                v[2] * (1.0 - ta) + t[2] * ta,
                v[3],
            ]
        },
        &b,
    );
    blend(
        "blendABGR",
        |v, t| {
            let ta = t[3];
            [
                v[0] * (1.0 - ta) + t[0] * ta,
                v[1] * (1.0 - ta) + t[1] * ta,
                v[2] * (1.0 - ta) + t[2] * ta,
                v[3] * (1.0 - ta) + ta * ta,
            ]
        },
        &b,
    );
    // tintABGR(target, tint)
    blend(
        "tintABGR",
        |t, tint| {
            let a = tint[3];
            [
                tint[0] * a + t[0] * (1.0 - a),
                tint[1] * a + t[1] * (1.0 - a),
                tint[2] * a + t[2] * (1.0 - a),
                t[3],
            ]
        },
        &b,
    );
    b.static_fn("lerpABGR", |c| {
        let (x, y, t) = (
            abgr_channels(int_of(c, 0)),
            abgr_channels(int_of(c, 1)),
            f32_arg(c, 2),
        );
        let l = |i: usize| x[i] * (1.0 - t) + y[i] * t;
        Ok(c.push(color_to_abgr(l(0), l(1), l(2), l(3)) as f64))
    });
    for (name, shift) in [
        ("getRedChannelFromABGR", 0),
        ("getGreenChannelFromABGR", 8),
        ("getBlueChannelFromABGR", 16),
        ("getAlphaChannelFromABGR", 24),
    ] {
        b.static_fn(name, move |c| {
            Ok(c.push(channel(int_of(c, 0), shift) as f64))
        });
    }
    for (name, shift) in [
        ("setRedChannelToABGR", 0u32),
        ("setGreenChannelToABGR", 8),
        ("setBlueChannelToABGR", 16),
        ("setAlphaChannelToABGR", 24),
    ] {
        b.static_fn(name, move |c| {
            let v = int_of(c, 0);
            let byte = (clamp01(f32_arg(c, 1)) * 255.0) as i32 & 0xFF;
            let mask = !(0xFFi32 << shift);
            Ok(c.push((byte << shift | (v & mask)) as f64))
        });
    }
    b.static_fn("decode", |c| {
        let s = str_arg(c, 0).ok_or_else(npe)?;
        let v = integer_decode(&s).ok_or_else(|| LuaError::Runtime {
            message: format!("For input string: \"{s}\""),
            class: "java.lang.NumberFormatException",
        })?;
        let col = Color::default();
        col.set_from_int(v);
        Ok(c.push(color_value(col)))
    });
    b.static_fn("HSBtoRGB", |c| {
        let col = match c.arg(3) {
            Value::Object(o) if o.as_any().is::<Color>() => o,
            _ => Rc::new(Color::default()) as Rc<dyn LuaObject>,
        };
        let [r, g, bl] = hsb_to_rgb(f32_arg(c, 0), f32_arg(c, 1), f32_arg(c, 2));
        let target = col.as_any().downcast_ref::<Color>().unwrap();
        target.set(r as f32 / 255.0, g as f32 / 255.0, bl as f32 / 255.0, 1.0);
        Ok(c.push(Value::Object(col)))
    });

    macro_rules! getter {
        ($name:literal, $e:expr) => {
            b.method($name, |c| {
                let v = this_color(c, $e)?;
                Ok(c.push(v as f64))
            });
        };
    }
    getter!("getR", |x: &Color| x.r.get());
    getter!("getG", |x: &Color| x.g.get());
    getter!("getB", |x: &Color| x.b.get());
    getter!("getRedFloat", |x: &Color| x.r.get());
    getter!("getGreenFloat", |x: &Color| x.g.get());
    getter!("getBlueFloat", |x: &Color| x.b.get());
    getter!("getAlphaFloat", |x: &Color| x.a.get());
    getter!("getRed", |x: &Color| (x.r.get() * 255.0) as i32);
    getter!("getGreen", |x: &Color| (x.g.get() * 255.0) as i32);
    getter!("getBlue", |x: &Color| (x.b.get() * 255.0) as i32);
    getter!("getAlpha", |x: &Color| (x.a.get() * 255.0) as i32);
    getter!("getRedByte", |x: &Color| (x.r.get() * 255.0) as i32);
    getter!("getGreenByte", |x: &Color| (x.g.get() * 255.0) as i32);
    getter!("getBlueByte", |x: &Color| (x.b.get() * 255.0) as i32);
    getter!("getAlphaByte", |x: &Color| (x.a.get() * 255.0) as i32);
    b.method("hashCode", |c| {
        let v = this_color(c, |x| {
            ((x.r.get() + x.g.get() + x.b.get() + x.a.get()) as i32).wrapping_mul(255)
        })?;
        Ok(c.push(v as f64))
    });
    b.method("toString", |c| {
        let v = this_color(c, |x| x.to_lua_string())?;
        Ok(c.push(v))
    });
    b.method("equals", |c| {
        let other = color_arg(c, 1);
        let v = this_color(c, |x| other.is_some_and(|o| o == x.get()))?;
        Ok(c.push(v))
    });
    b.method("equalBytes", |c| {
        let other = color_arg(c, 1);
        let v = this_color(c, |x| {
            other.is_some_and(|o| {
                (0..4).all(|i| (o[i] * 255.0) as i32 == (x.get()[i] * 255.0) as i32)
            })
        })?;
        Ok(c.push(v))
    });
    b.method("set", |c| {
        let n = c.nargs() - 1;
        if n == 1 {
            let o = color_arg(c, 1).ok_or_else(npe)?;
            this_color(c, |x| x.set(o[0], o[1], o[2], o[3]))?;
        } else {
            let a = if n >= 4 { f32_arg(c, 4) } else { 1.0 };
            let (r, g, bl) = (f32_arg(c, 1), f32_arg(c, 2), f32_arg(c, 3));
            this_color(c, |x| x.set(r, g, bl, a))?;
        }
        Ok(c.push(c.arg(0)))
    });
    b.method("setColor", |c| {
        let (a, b2) = (
            color_arg(c, 1).ok_or_else(npe)?,
            color_arg(c, 2).ok_or_else(npe)?,
        );
        let d = f32_arg(c, 3);
        this_color(c, |x| {
            x.set(
                a[0] + (b2[0] - a[0]) * d,
                a[1] + (b2[1] - a[1]) * d,
                a[2] + (b2[2] - a[2]) * d,
                a[3] + (b2[3] - a[3]) * d,
            )
        })?;
        Ok(0)
    });
    b.method("setABGR", |c| {
        let [r, g, bl, a] = abgr_channels(int_of(c, 1));
        this_color(c, |x| x.set(r, g, bl, a))?;
        Ok(0)
    });
    b.method("fromColor", |c| {
        let v = int_of(c, 1);
        this_color(c, |x| x.set_from_int(v))?;
        Ok(0)
    });
    b.method("add", |c| {
        let o = color_arg(c, 1).ok_or_else(npe)?;
        this_color(c, |x| {
            let s = x.get();
            x.set(s[0] + o[0], s[1] + o[1], s[2] + o[2], s[3] + o[3]);
        })?;
        Ok(0)
    });
    b.method("addToCopy", |c| {
        let o = color_arg(c, 1).ok_or_else(npe)?;
        let s = this_color(c, Color::get)?;
        let copy = Color::rgba(
            clamp01(s[0]) + o[0],
            clamp01(s[1]) + o[1],
            clamp01(s[2]) + o[2],
            clamp01(s[3]) + o[3],
        );
        Ok(c.push(color_value(copy)))
    });
    b.method("brighter", |c| {
        let d = if c.nargs() > 1 { f32_arg(c, 1) } else { 0.2 };
        this_color(c, |x| {
            x.set(x.r.get() + d, x.g.get() + d, x.b.get() + d, x.a.get())
        })?;
        Ok(c.push(c.arg(0)))
    });
    b.method("darker", |c| {
        let d = if c.nargs() > 1 { f32_arg(c, 1) } else { 0.5 };
        this_color(c, |x| {
            x.set(x.r.get() - d, x.g.get() - d, x.b.get() - d, x.a.get())
        })?;
        Ok(c.push(c.arg(0)))
    });
    b.method("multiply", |c| {
        let o = color_arg(c, 1).ok_or_else(npe)?;
        let s = this_color(c, Color::get)?;
        let m = Color::rgba(
            clamp01(s[0] * o[0]),
            clamp01(s[1] * o[1]),
            clamp01(s[2] * o[2]),
            clamp01(s[3] * o[3]),
        );
        Ok(c.push(color_value(m)))
    });
    b.method("scale", |c| {
        let k = f32_arg(c, 1);
        this_color(c, |x| {
            x.set(x.r.get() * k, x.g.get() * k, x.b.get() * k, x.a.get() * k)
        })?;
        Ok(c.push(c.arg(0)))
    });
    b.method("scaleCopy", |c| {
        let k = f32_arg(c, 1);
        let s = this_color(c, Color::get)?;
        let m = Color::rgba(
            clamp01(s[0]) * k,
            clamp01(s[1]) * k,
            clamp01(s[2]) * k,
            clamp01(s[3]) * k,
        );
        Ok(c.push(color_value(m)))
    });
    b.method("interp", |c| {
        let to = color_arg(c, 1).ok_or_else(npe)?;
        let d = f32_arg(c, 2);
        let s = this_color(c, Color::get)?;
        let Value::Object(o) = c.arg(3) else {
            return Err(npe());
        };
        let dest = o.as_any().downcast_ref::<Color>().ok_or_else(npe)?;
        dest.set(
            s[0] + (to[0] - s[0]) * d,
            s[1] + (to[1] - s[1]) * d,
            s[2] + (to[2] - s[2]) * d,
            s[3] + (to[3] - s[3]) * d,
        );
        Ok(0)
    });
    b.method("changeHSBValue", |c| {
        let (hf, sf, bf) = (f32_arg(c, 1), f32_arg(c, 2), f32_arg(c, 3));
        this_color(c, |x| {
            let [h, s, v] = rgb_to_hsb(
                (x.r.get() * 255.0) as i32,
                (x.g.get() * 255.0) as i32,
                (x.b.get() * 255.0) as i32,
            );
            let [r, g, bl] = hsb_to_rgb(h * hf, s * sf, v * bf);
            x.r.set(r as f32 / 255.0);
            x.g.set(g as f32 / 255.0);
            x.b.set(bl as f32 / 255.0);
        })?;
        Ok(0)
    });
}

// ---- zombie.core.ImmutableColor --------------------------------------------------------------

fn imm(r: f32, g: f32, b: f32, a: f32) -> ImmutableColor {
    ImmutableColor { r, g, b, a }
}

/// `new ImmutableColor(float r, float g, float b)`: clamped, opaque.
fn imm_rgb(r: f32, g: f32, b: f32) -> ImmutableColor {
    imm(clamp01(r), clamp01(g), clamp01(b), 1.0)
}

/// `new ImmutableColor(int r, int g, int b, int a)`.
fn imm_ints(r: i32, g: i32, b: i32, a: i32) -> ImmutableColor {
    imm(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0)
}

/// `new ImmutableColor(int value)`: ABGR with alpha 0 read as 255.
fn imm_from_int(v: i32) -> ImmutableColor {
    let col = Color::default();
    col.set_from_int(v);
    let [r, g, b, a] = col.get();
    imm(r, g, b, a)
}

/// An `ImmutableColor` Lua object.
pub fn immutable_color_value(c: ImmutableColor) -> Value {
    Value::Object(Rc::new(c))
}

/// The `ImmutableColor` argument `i`; `None` for `null` or another class.
pub fn immutable_color_arg(c: &NativeCall<'_>, i: usize) -> Option<ImmutableColor> {
    match c.arg(i) {
        Value::Object(o) => o.as_any().downcast_ref::<ImmutableColor>().copied(),
        _ => None,
    }
}

fn this_imm(c: &NativeCall<'_>) -> LuaResult<ImmutableColor> {
    with_this::<ImmutableColor, _>(c, |x| *x)
}

fn register_immutable_color(thread: &mut Thread, jar: Option<&pz_classfile::Jar>) {
    let b = expose_class(thread, IMMUTABLE_COLOR_CLASS, None);
    // Named colours: read from ImmutableColor.<clinit> in the installed jar.
    if let Some(jar) = jar {
        if let Ok(ctors) = pz_classfile::self_constructed_constants(jar, "zombie/core/ImmutableColor") {
            for (name, args) in ctors {
                let ints: Option<Vec<i32>> = args
                    .iter()
                    .map(|a| match a {
                        pz_classfile::Value::Int(i) => Some(*i as i32),
                        _ => None,
                    })
                    .collect();
                let floats: Option<Vec<f32>> = args
                    .iter()
                    .map(|a| match a {
                        pz_classfile::Value::Float(f) => Some(*f as f32),
                        _ => None,
                    })
                    .collect();
                let col = match (ints.as_deref(), floats.as_deref()) {
                    (Some(&[r, g, bl, a]), _) => imm_ints(r, g, bl, a),
                    (Some(&[r, g, bl]), _) => imm_ints(r, g, bl, 255),
                    (Some(&[v]), _) => imm_from_int(v),
                    (_, Some(&[r, g, bl, a])) => imm(r.min(1.0), g.min(1.0), bl.min(1.0), a.min(1.0)),
                    (_, Some(&[r, g, bl])) => imm_rgb(r, g, bl),
                    _ => continue,
                };
                b.field(&name, immutable_color_value(col));
            }
        }
    }
    // As for `Color`: plain numbers reach the float constructors (one
    // number, the `int` one).
    b.ctor(|c| {
        let col = match c.nargs() {
            1 => match c.arg(0) {
                Value::Num(_) => imm_from_int(int_of(c, 0)),
                // `new ImmutableColor((ImmutableColor) null)`: opaque black.
                Value::Nil => imm(0.0, 0.0, 0.0, 1.0),
                Value::Object(o) => match (o.as_any().downcast_ref::<ImmutableColor>(), o.as_any().downcast_ref::<Color>()) {
                    (Some(x), _) => *x,
                    (_, Some(x)) => {
                        let [r, g, bl, a] = x.get();
                        imm(r, g, bl, a)
                    }
                    _ => return Err(no_method("ImmutableColor.new")),
                },
                _ => return Err(no_method("ImmutableColor.new")),
            },
            3 if is_num(&c.arg(0)) => imm_rgb(f32_arg(c, 0), f32_arg(c, 1), f32_arg(c, 2)),
            3 => {
                let (a, b2) = (color_arg(c, 0).ok_or_else(npe)?, color_arg(c, 1).ok_or_else(npe)?);
                let d = f32_arg(c, 2);
                let l = |i: usize| a[i] + (b2[i] - a[i]) * d;
                imm(l(0), l(1), l(2), l(3))
            }
            4 => imm(
                f32_arg(c, 0).min(1.0),
                f32_arg(c, 1).min(1.0),
                f32_arg(c, 2).min(1.0),
                f32_arg(c, 3).min(1.0),
            ),
            _ => return Err(no_method("ImmutableColor.new")),
        };
        Ok(c.push(immutable_color_value(col)))
    });
    b.static_fn("random", |_c| {
        let h = pz_core::random::next_float(0.0, 1.0);
        let s = pz_core::random::next_float(0.0, 0.6);
        let v = pz_core::random::next_float(0.0, 0.9);
        let [r, g, bl] = hsb_to_rgb(h, s, v);
        // `new ImmutableColor(Color.HSBtoRGB(...))` (`new Color(int, int, int)`).
        Ok(_c.push(immutable_color_value(imm_ints(r, g, bl, 255))))
    });
    b.static_fn("decode", |c| {
        let s = str_arg(c, 0).ok_or_else(npe)?;
        let v = integer_decode(&s).ok_or_else(|| LuaError::Runtime {
            message: format!("For input string: \"{s}\""),
            class: "java.lang.NumberFormatException",
        })?;
        Ok(c.push(immutable_color_value(imm_from_int(v))))
    });
    macro_rules! getter {
        ($name:literal, $e:expr) => {
            b.method($name, |c| {
                let x = this_imm(c)?;
                let f: fn(ImmutableColor) -> f64 = $e;
                Ok(c.push(f(x)))
            });
        };
    }
    getter!("getRedFloat", |x| x.r as f64);
    getter!("getGreenFloat", |x| x.g as f64);
    getter!("getBlueFloat", |x| x.b as f64);
    getter!("getAlphaFloat", |x| x.a as f64);
    getter!("getRedInt", |x| (x.r * 255.0) as i32 as f64);
    getter!("getGreenInt", |x| (x.g * 255.0) as i32 as f64);
    getter!("getBlueInt", |x| (x.b * 255.0) as i32 as f64);
    getter!("getAlphaInt", |x| (x.a * 255.0) as i32 as f64);
    getter!("getRedByte", |x| ((x.r * 255.0) as i32 & 0xFF) as i8 as f64);
    getter!("getGreenByte", |x| ((x.g * 255.0) as i32 & 0xFF) as i8 as f64);
    getter!("getBlueByte", |x| ((x.b * 255.0) as i32 & 0xFF) as i8 as f64);
    getter!("getAlphaByte", |x| ((x.a * 255.0) as i32 & 0xFF) as i8 as f64);
    getter!("hashCode", |x| ((x.r + x.g + x.b + x.a) as i32).wrapping_mul(255) as f64);
    b.method("toString", |c| {
        let v = with_this::<ImmutableColor, _>(c, |x| x.to_lua_string())?;
        Ok(c.push(v))
    });
    b.method("equals", |c| {
        let (x, o) = (this_imm(c)?, immutable_color_arg(c, 1));
        Ok(c.push(o.is_some_and(|o| o == x)))
    });
    b.method("toMutableColor", |c| {
        let x = this_imm(c)?;
        Ok(c.push(color_value(Color::rgba(x.r, x.g, x.b, x.a))))
    });
    b.method("add", |c| {
        let (x, o) = (this_imm(c)?, immutable_color_arg(c, 1).ok_or_else(npe)?);
        let v = imm((x.r + o.r).min(1.0), (x.g + o.g).min(1.0), (x.b + o.b).min(1.0), (x.a + o.a).min(1.0));
        Ok(c.push(immutable_color_value(v)))
    });
    for (name, default, sign) in [("brighter", 0.2f32, 1.0f32), ("darker", 0.5, -1.0)] {
        b.method(name, move |c| {
            let x = this_imm(c)?;
            let d = if c.nargs() > 1 { f32_arg(c, 1) } else { default } * sign;
            Ok(c.push(immutable_color_value(imm_rgb(x.r + d, x.g + d, x.b + d))))
        });
    }
    b.method("multiply", |c| {
        let (x, o) = (this_imm(c)?, color_arg(c, 1).ok_or_else(npe)?);
        let v = imm((x.r * o[0]).min(1.0), (x.g * o[1]).min(1.0), (x.b * o[2]).min(1.0), (x.a * o[3]).min(1.0));
        Ok(c.push(immutable_color_value(v)))
    });
    b.method("scale", |c| {
        let (x, k) = (this_imm(c)?, f32_arg(c, 1));
        let v = imm((x.r * k).min(1.0), (x.g * k).min(1.0), (x.b * k).min(1.0), (x.a * k).min(1.0));
        Ok(c.push(immutable_color_value(v)))
    });
    b.method("interp", |c| {
        let (x, to, d) = (this_imm(c)?, immutable_color_arg(c, 1).ok_or_else(npe)?, f32_arg(c, 2));
        let l = |a: f32, b: f32| (a + (b - a) * d).min(1.0);
        let v = imm(l(x.r, to.r), l(x.g, to.g), l(x.b, to.b), l(x.a, to.a));
        Ok(c.push(immutable_color_value(v)))
    });
}

/// `java.awt.Color.HSBtoRGB` (also `Color.HSBtoRGB`, which rounds the same way).
fn hsb_to_rgb(hue: f32, saturation: f32, brightness: f32) -> [i32; 3] {
    let c = |v: f32| (v * 255.0 + 0.5) as i32;
    if saturation == 0.0 {
        let v = c(brightness);
        return [v, v, v];
    }
    let h = (hue - hue.floor()) * 6.0;
    let f = h - h.floor();
    let p = brightness * (1.0 - saturation);
    let q = brightness * (1.0 - saturation * f);
    let t = brightness * (1.0 - saturation * (1.0 - f));
    match h as i32 {
        0 => [c(brightness), c(t), c(p)],
        1 => [c(q), c(brightness), c(p)],
        2 => [c(p), c(brightness), c(t)],
        3 => [c(p), c(q), c(brightness)],
        4 => [c(t), c(p), c(brightness)],
        5 => [c(brightness), c(p), c(q)],
        _ => [0, 0, 0],
    }
}

/// `java.awt.Color.RGBtoHSB`.
fn rgb_to_hsb(r: i32, g: i32, b: i32) -> [f32; 3] {
    let cmax = r.max(g).max(b);
    let cmin = r.min(g).min(b);
    let brightness = cmax as f32 / 255.0;
    let saturation = if cmax != 0 {
        (cmax - cmin) as f32 / cmax as f32
    } else {
        0.0
    };
    let hue = if saturation == 0.0 {
        0.0
    } else {
        let d = (cmax - cmin) as f32;
        let rc = (cmax - r) as f32 / d;
        let gc = (cmax - g) as f32 / d;
        let bc = (cmax - b) as f32 / d;
        let mut h = if r == cmax {
            bc - gc
        } else if g == cmax {
            2.0 + rc - bc
        } else {
            4.0 + gc - rc
        };
        h /= 6.0;
        if h < 0.0 {
            h += 1.0;
        }
        h
    };
    [hue, saturation, brightness]
}

// ---- zombie.iso.Vector2 ------------------------------------------------------------------------

/// `zombie.iso.Vector2`.
#[derive(Default)]
pub struct Vector2 {
    pub x: Cell<f32>,
    pub y: Cell<f32>,
}

impl LuaObject for Vector2 {
    fn class_name(&self) -> &str {
        "zombie.iso.Vector2"
    }
    fn to_lua_string(&self) -> String {
        let f = |v: f32| {
            pz_text::jfmt::format("%f", &[pz_text::JavaArg::Double(v as f64)]).unwrap_or_default()
        };
        format!(
            "Vector2 (X: {}, Y: {}) (L: {}, D:{})",
            f(self.x.get()),
            f(self.y.get()),
            f(self.length()),
            f(self.direction())
        )
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `PZMath.wrap(val, range)`.
fn wrap_range(val: f32, range: f32) -> f32 {
    if range <= 0.0 {
        return 0.0;
    }
    let frac = |v: f32| v - v.floor();
    if val < 0.0 {
        (1.0 - frac(-val / range)) * range
    } else {
        frac(val / range) * range
    }
}

/// `PZMath.wrap(val, min, max)`.
fn wrap(val: f32, min: f32, max: f32) -> f32 {
    let (hi, lo) = (
        if max > min { max } else { min },
        if max < min { max } else { min },
    );
    lo + wrap_range(val - lo, hi - lo)
}

impl Vector2 {
    pub fn new(x: f32, y: f32) -> Self {
        Vector2 {
            x: Cell::new(x),
            y: Cell::new(y),
        }
    }

    fn set(&self, x: f32, y: f32) {
        self.x.set(x);
        self.y.set(y);
    }

    fn length_sq(&self) -> f32 {
        self.x.get() * self.x.get() + self.y.get() * self.y.get()
    }

    fn length(&self) -> f32 {
        (self.length_sq() as f64).sqrt() as f32
    }

    fn direction(&self) -> f32 {
        let angle = (self.y.get() as f64).atan2(self.x.get() as f64) as f32;
        wrap(
            angle,
            -std::f64::consts::PI as f32,
            std::f64::consts::PI as f32,
        )
    }

    fn set_length_and_direction(&self, direction: f32, length: f32) {
        self.set(
            ((direction as f64).cos() * length as f64) as f32,
            ((direction as f64).sin() * length as f64) as f32,
        );
    }

    fn normalize(&self) -> f32 {
        let l2 = self.length_sq();
        if (1.0 - l2).abs() < 1.0e-5 {
            return 1.0;
        }
        if l2 == 0.0 {
            self.set(0.0, 0.0);
            return 0.0;
        }
        let l = (l2 as f64).sqrt() as f32;
        self.set(self.x.get() / l, self.y.get() / l);
        l
    }

    fn rotate_rad(&self, r: f64) {
        let (x, y) = (self.x.get() as f64, self.y.get() as f64);
        self.set(
            (x * r.cos() - y * r.sin()) as f32,
            (x * r.sin() + y * r.cos()) as f32,
        );
    }
}

fn vec2_arg(c: &NativeCall<'_>, i: usize) -> Option<(f32, f32)> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<Vector2>()
            .map(|v| (v.x.get(), v.y.get())),
        _ => None,
    }
}

fn vec2_obj(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<dyn LuaObject>> {
    match c.arg(i) {
        Value::Object(o) if o.as_any().is::<Vector2>() => Ok(o),
        _ => Err(npe()),
    }
}

fn vec2(o: &Rc<dyn LuaObject>) -> &Vector2 {
    o.as_any().downcast_ref::<Vector2>().unwrap()
}

fn this_vec2<R>(c: &NativeCall<'_>, f: impl FnOnce(&Vector2) -> R) -> LuaResult<R> {
    with_this::<Vector2, _>(c, f)
}

fn register_vector2(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.Vector2", None);
    b.ctor(|c| {
        let v = match c.nargs() {
            0 => Vector2::default(),
            1 => {
                let (x, y) = vec2_arg(c, 0).ok_or_else(npe)?;
                Vector2::new(x, y)
            }
            _ => Vector2::new(f32_arg(c, 0), f32_arg(c, 1)),
        };
        Ok(c.push(Value::Object(Rc::new(v))))
    });
    b.static_fn("fromLengthDirection", |c| {
        let v = Vector2::default();
        v.set_length_and_direction(f32_arg(c, 1), f32_arg(c, 0));
        Ok(c.push(Value::Object(Rc::new(v))))
    });
    b.static_fn("dot", |c| {
        Ok(c.push((f32_arg(c, 0) * f32_arg(c, 2) + f32_arg(c, 1) * f32_arg(c, 3)) as f64))
    });
    b.static_fn("getDirection", |c| {
        let v = Vector2::new(f32_arg(c, 0), f32_arg(c, 1));
        Ok(c.push(v.direction() as f64))
    });
    b.static_fn("addScaled", |c| {
        let (a, bb, s) = (
            vec2_arg(c, 0).ok_or_else(npe)?,
            vec2_arg(c, 1).ok_or_else(npe)?,
            f32_arg(c, 2),
        );
        let r = vec2_obj(c, 3)?;
        vec2(&r).set(a.0 + bb.0 * s, a.1 + bb.1 * s);
        Ok(c.push(Value::Object(r)))
    });
    b.static_fn("scale", |c| {
        let v = vec2_obj(c, 0)?;
        let s = f32_arg(c, 1);
        vec2(&v).set(vec2(&v).x.get() * s, vec2(&v).y.get() * s);
        Ok(c.push(Value::Object(v)))
    });
    b.static_fn("moveTowards", |c| {
        let cur = vec2_obj(c, 0)?;
        let target = vec2_obj(c, 1)?;
        let max_delta = f32_arg(c, 2);
        let (cx, cy) = (vec2(&cur).x.get(), vec2(&cur).y.get());
        let (tx, ty) = (vec2(&target).x.get() - cx, vec2(&target).y.get() - cy);
        let sq = tx * tx + ty * ty;
        if sq == 0.0 || (max_delta >= 0.0 && sq <= max_delta * max_delta) {
            return Ok(c.push(Value::Object(target)));
        }
        let d = (sq as f64).sqrt() as f32;
        Ok(c.push(Value::Object(Rc::new(Vector2::new(
            cx + tx / d * max_delta,
            cy + ty / d * max_delta,
        )))))
    });

    b.method("getX", |c| Ok(c.push(this_vec2(c, |v| v.x.get())? as f64)));
    b.method("getY", |c| Ok(c.push(this_vec2(c, |v| v.y.get())? as f64)));
    b.method("setX", |c| {
        let x = f32_arg(c, 1);
        this_vec2(c, |v| v.x.set(x))?;
        Ok(0)
    });
    b.method("setY", |c| {
        let y = f32_arg(c, 1);
        this_vec2(c, |v| v.y.set(y))?;
        Ok(0)
    });
    b.method("set", |c| {
        let (x, y) = match vec2_arg(c, 1) {
            Some(p) => p,
            None => (f32_arg(c, 1), f32_arg(c, 2)),
        };
        this_vec2(c, |v| v.set(x, y))?;
        Ok(c.push(c.arg(0)))
    });
    b.method("add", |c| {
        let (x, y) = vec2_arg(c, 1).ok_or_else(npe)?;
        this_vec2(c, |v| v.set(v.x.get() + x, v.y.get() + y))?;
        Ok(c.push(c.arg(0)))
    });
    b.method("mul", |c| {
        let m = f32_arg(c, 1);
        this_vec2(c, |v| v.set(v.x.get() * m, v.y.get() * m))?;
        Ok(c.push(c.arg(0)))
    });
    b.method("scale", |c| {
        let s = f32_arg(c, 1);
        this_vec2(c, |v| v.set(v.x.get() * s, v.y.get() * s))?;
        Ok(0)
    });
    b.method("dot", |c| {
        let (x, y) = match vec2_arg(c, 1) {
            Some(p) => p,
            None => (f32_arg(c, 1), f32_arg(c, 2)),
        };
        Ok(c.push(this_vec2(c, |v| v.x.get() * x + v.y.get() * y)? as f64))
    });
    b.method("getLength", |c| {
        Ok(c.push(this_vec2(c, Vector2::length)? as f64))
    });
    b.method("getLengthSquared", |c| {
        Ok(c.push(this_vec2(c, Vector2::length_sq)? as f64))
    });
    b.method("getDirection", |c| {
        Ok(c.push(this_vec2(c, Vector2::direction)? as f64))
    });
    b.method("getDirectionNeg", |c| {
        Ok(c.push(this_vec2(c, |v| (v.x.get() as f64).atan2(v.y.get() as f64) as f32)? as f64))
    });
    b.method("setDirection", |c| {
        let d = f32_arg(c, 1);
        this_vec2(c, |v| v.set_length_and_direction(d, v.length()))?;
        Ok(c.push(c.arg(0)))
    });
    b.method("setLength", |c| {
        let l = f32_arg(c, 1);
        this_vec2(c, |v| {
            v.normalize();
            v.set(v.x.get() * l, v.y.get() * l);
        })?;
        Ok(c.push(c.arg(0)))
    });
    b.method("setMaxLength", |c| {
        let m = f32_arg(c, 1);
        let r = this_vec2(c, |v| {
            let l = v.length();
            if l > m {
                v.normalize();
                v.set(v.x.get() * m, v.y.get() * m);
                m
            } else {
                l
            }
        })?;
        Ok(c.push(r as f64))
    });
    b.method("setLengthAndDirection", |c| {
        let (d, l) = (f32_arg(c, 1), f32_arg(c, 2));
        this_vec2(c, |v| v.set_length_and_direction(d, l))?;
        Ok(c.push(c.arg(0)))
    });
    b.method("normalize", |c| {
        Ok(c.push(this_vec2(c, Vector2::normalize)? as f64))
    });
    b.method("rotate", |c| {
        let r = f32_arg(c, 1) as f64;
        this_vec2(c, |v| v.rotate_rad(r))?;
        Ok(0)
    });
    b.method("tangent", |c| {
        this_vec2(c, |v| v.rotate_rad(90f64.to_radians()))?;
        Ok(0)
    });
    b.method("angleTo", |c| {
        let (x, y) = vec2_arg(c, 1).ok_or_else(npe)?;
        Ok(c.push(this_vec2(c, |v| {
            ((y - v.y.get()) as f64).atan2((x - v.x.get()) as f64) as f32
        })? as f64))
    });
    b.method("aimAt", |c| {
        let (x, y) = vec2_arg(c, 1).ok_or_else(npe)?;
        this_vec2(c, |v| {
            let a = ((y - v.y.get()) as f64).atan2((x - v.x.get()) as f64) as f32;
            v.set_length_and_direction(a, v.length());
        })?;
        Ok(c.push(c.arg(0)))
    });
    b.method("angleBetween", |c| {
        let (x, y) = vec2_arg(c, 1).ok_or_else(npe)?;
        let other_len = ((x * x + y * y) as f64).sqrt() as f32;
        let r = this_vec2(c, |v| {
            let dot = ((v.x.get() * x + v.y.get() * y) / (v.length() * other_len)).clamp(-1.0, 1.0);
            (dot as f64).acos() as f32
        })?;
        Ok(c.push(r as f64))
    });
    b.method("distanceTo", |c| {
        let (x, y) = vec2_arg(c, 1).ok_or_else(npe)?;
        let r = this_vec2(c, |v| {
            (((x - v.x.get()) as f64).powi(2) + ((y - v.y.get()) as f64).powi(2)).sqrt() as f32
        })?;
        Ok(c.push(r as f64))
    });
    b.method("floorX", |c| {
        Ok(c.push(this_vec2(c, |v| fastfloor(v.x.get()))? as f64))
    });
    b.method("floorY", |c| {
        Ok(c.push(this_vec2(c, |v| fastfloor(v.y.get()))? as f64))
    });
    b.method("clone", |c| {
        let (x, y) = this_vec2(c, |v| (v.x.get(), v.y.get()))?;
        Ok(c.push(Value::Object(Rc::new(Vector2::new(x, y)))))
    });
    b.method("equals", |c| {
        let o = vec2_arg(c, 1);
        Ok(c.push(this_vec2(c, |v| o == Some((v.x.get(), v.y.get())))?))
    });
    b.method("toString", |c| {
        Ok(c.push(this_vec2(c, |v| v.to_lua_string())?))
    });
}

/// `PZMath.fastfloor`.
fn fastfloor(v: f32) -> i32 {
    let xi = v as i32;
    if v < xi as f32 {
        xi - 1
    } else {
        xi
    }
}

// ---- org.joml.Vector3f ----------------------------------------------------------------------------

/// `org.joml.Vector3f` (the methods Lua uses on the menu path and in the
/// vehicle and rest actions).
#[derive(Default)]
pub struct Vector3f {
    pub x: Cell<f32>,
    pub y: Cell<f32>,
    pub z: Cell<f32>,
}

impl LuaObject for Vector3f {
    fn class_name(&self) -> &str {
        "org.joml.Vector3f"
    }
    fn to_lua_string(&self) -> String {
        // JOML: Runtime.formatNumbers(toString(Options.NUMBER_FORMAT)) — "(x y z)"
        // with 3-decimal scientific notation.
        let f = |v: f32| joml_format(v as f64);
        format!(
            "({} {} {})",
            f(self.x.get()),
            f(self.y.get()),
            f(self.z.get())
        )
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// JOML's default number format (`DecimalFormat(" 0.000E0;-")`).
fn joml_format(v: f64) -> String {
    if v == 0.0 {
        return " 0.000E0".into();
    }
    let s = format!("{:.3e}", v.abs());
    let (m, e) = s.split_once('e').unwrap();
    format!("{}{}E{}", if v < 0.0 { "-" } else { " " }, m, e)
}

impl Vector3f {
    fn get(&self) -> [f32; 3] {
        [self.x.get(), self.y.get(), self.z.get()]
    }

    fn set(&self, v: [f32; 3]) {
        self.x.set(v[0]);
        self.y.set(v[1]);
        self.z.set(v[2]);
    }

    fn length_sq(v: [f32; 3]) -> f32 {
        v[0].mul_add(v[0], v[1].mul_add(v[1], v[2] * v[2]))
    }
}

fn vec3_arg(c: &NativeCall<'_>, i: usize) -> Option<[f32; 3]> {
    match c.arg(i) {
        Value::Object(o) => o.as_any().downcast_ref::<Vector3f>().map(Vector3f::get),
        _ => None,
    }
}

fn this_vec3<R>(c: &NativeCall<'_>, f: impl FnOnce(&Vector3f) -> R) -> LuaResult<R> {
    with_this::<Vector3f, _>(c, f)
}

/// The `(x, y, z)` given either as a vector or as three numbers starting at `i`.
fn xyz(c: &NativeCall<'_>, i: usize) -> LuaResult<[f32; 3]> {
    match vec3_arg(c, i) {
        Some(v) => Ok(v),
        None if c.nargs() > i + 2 => Ok([f32_arg(c, i), f32_arg(c, i + 1), f32_arg(c, i + 2)]),
        None if c.nargs() == i + 1 && is_num(&c.arg(i)) => Ok([f32_arg(c, i); 3]),
        None => Err(npe()),
    }
}

fn register_vector3f(thread: &mut Thread) {
    let b = expose_class(thread, "org.joml.Vector3f", None);
    b.ctor(|c| {
        let v = Vector3f::default();
        if c.nargs() > 0 {
            v.set(xyz(c, 0)?);
        }
        Ok(c.push(Value::Object(Rc::new(v))))
    });
    b.static_fn("length", |c| {
        Ok(c.push(
            (Vector3f::length_sq([f32_arg(c, 0), f32_arg(c, 1), f32_arg(c, 2)]) as f64).sqrt()
                as f32 as f64,
        ))
    });
    b.static_fn("lengthSquared", |c| {
        Ok(c.push(Vector3f::length_sq([f32_arg(c, 0), f32_arg(c, 1), f32_arg(c, 2)]) as f64))
    });
    let dist_sq = |c: &NativeCall<'_>| {
        let d = [
            f32_arg(c, 0) - f32_arg(c, 3),
            f32_arg(c, 1) - f32_arg(c, 4),
            f32_arg(c, 2) - f32_arg(c, 5),
        ];
        Vector3f::length_sq(d)
    };
    b.static_fn("distanceSquared", move |c| Ok(c.push(dist_sq(c) as f64)));
    b.static_fn("distance", move |c| {
        Ok(c.push((dist_sq(c) as f64).sqrt() as f32 as f64))
    });

    b.method("x", |c| Ok(c.push(this_vec3(c, |v| v.x.get())? as f64)));
    b.method("y", |c| Ok(c.push(this_vec3(c, |v| v.y.get())? as f64)));
    b.method("z", |c| Ok(c.push(this_vec3(c, |v| v.z.get())? as f64)));
    b.method("set", |c| {
        let v = xyz(c, 1)?;
        this_vec3(c, |s| s.set(v))?;
        Ok(c.push(c.arg(0)))
    });
    b.method("zero", |c| {
        this_vec3(c, |s| s.set([0.0; 3]))?;
        Ok(c.push(c.arg(0)))
    });
    let op = |name: &'static str, f: fn(f32, f32) -> f32, b: &crate::expose::ClassBuilder| {
        b.method(name, move |c| {
            let o = xyz(c, 1)?;
            this_vec3(c, |s| {
                let v = s.get();
                s.set([f(v[0], o[0]), f(v[1], o[1]), f(v[2], o[2])]);
            })?;
            Ok(c.push(c.arg(0)))
        });
    };
    op("add", |a, b| a + b, &b);
    op("sub", |a, b| a - b, &b);
    op("mul", |a, b| a * b, &b);
    op("div", |a, b| a / b, &b);
    b.method("negate", |c| {
        this_vec3(c, |s| {
            let v = s.get();
            s.set([-v[0], -v[1], -v[2]]);
        })?;
        Ok(c.push(c.arg(0)))
    });
    b.method("length", |c| {
        Ok(c.push(this_vec3(c, |s| (Vector3f::length_sq(s.get()) as f64).sqrt() as f32)? as f64))
    });
    b.method("lengthSquared", |c| {
        Ok(c.push(this_vec3(c, |s| Vector3f::length_sq(s.get()))? as f64))
    });
    b.method("dot", |c| {
        let o = xyz(c, 1)?;
        Ok(c.push(this_vec3(c, |s| {
            let v = s.get();
            v[0].mul_add(o[0], v[1].mul_add(o[1], v[2] * o[2]))
        })? as f64))
    });
    b.method("distanceSquared", |c| {
        let o = xyz(c, 1)?;
        Ok(c.push(this_vec3(c, |s| {
            let v = s.get();
            Vector3f::length_sq([v[0] - o[0], v[1] - o[1], v[2] - o[2]])
        })? as f64))
    });
    b.method("distance", |c| {
        let o = xyz(c, 1)?;
        Ok(c.push(this_vec3(c, |s| {
            let v = s.get();
            (Vector3f::length_sq([v[0] - o[0], v[1] - o[1], v[2] - o[2]]) as f64).sqrt() as f32
        })? as f64))
    });
    b.method("normalize", |c| {
        let len = if c.nargs() > 1 && is_num(&c.arg(1)) {
            Some(f32_arg(c, 1))
        } else {
            None
        };
        this_vec3(c, |s| {
            let v = s.get();
            let inv = (1.0 / (Vector3f::length_sq(v) as f64).sqrt()) as f32 * len.unwrap_or(1.0);
            s.set([v[0] * inv, v[1] * inv, v[2] * inv]);
        })?;
        Ok(c.push(c.arg(0)))
    });
    b.method("cross", |c| {
        let o = xyz(c, 1)?;
        this_vec3(c, |s| {
            let v = s.get();
            s.set([
                v[1].mul_add(o[2], -v[2] * o[1]),
                v[2].mul_add(o[0], -v[0] * o[2]),
                v[0].mul_add(o[1], -v[1] * o[0]),
            ]);
        })?;
        Ok(c.push(c.arg(0)))
    });
    b.method("equals", |c| {
        let o = vec3_arg(c, 1);
        Ok(c.push(this_vec3(c, |s| o == Some(s.get()))?))
    });
    b.method("toString", |c| {
        Ok(c.push(this_vec3(c, |s| s.to_lua_string())?))
    });
}

// ---- zombie.characters.Capability --------------------------------------------------------------

fn register_capability(thread: &mut Thread, jar: Option<&pz_classfile::Jar>) {
    let Some(jar) = jar else { return };
    let Ok(names) = pz_classfile::enum_constants(jar, "zombie/characters/Capability") else {
        return;
    };
    let names: Vec<&'static str> = names
        .into_iter()
        .map(|n| &*Box::leak(n.into_boxed_str()))
        .collect();
    expose_enum(thread, "zombie.characters.Capability", &names);
}

pub fn register(thread: &mut Thread, jar: Option<&pz_classfile::Jar>) {
    register_system_disabler(thread);
    register_color(thread, jar);
    register_immutable_color(thread, jar);
    register_vector2(thread);
    register_vector3f(thread);
    register_capability(thread, jar);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abgr_round_trip() {
        let v = color_to_abgr(1.0, 0.5, 0.0, 1.0);
        assert_eq!(v as u32, 0xFF00_7FFF);
        assert!((channel(v, 8) - 0.49803925).abs() < 1e-6);
        assert_eq!(integer_decode("0xFF"), Some(255));
        assert_eq!(integer_decode("#10"), Some(16));
        assert_eq!(integer_decode("010"), Some(8));
        assert_eq!(hsb_to_rgb(0.0, 1.0, 1.0), [255, 0, 0]);
    }

    #[test]
    fn vector2_direction_wraps() {
        let v = Vector2::new(-1.0, 0.0);
        assert!(
            (v.direction() - std::f32::consts::PI).abs() < 1e-5
                || (v.direction() + std::f32::consts::PI).abs() < 1e-5
        );
        assert_eq!(fastfloor(-0.5), -1);
        assert_eq!(joml_format(1.5), " 1.500E0");
    }

    #[test]
    fn lua_smoke() {
        let env = pz_kahlua::TableRef::new();
        let mut thread = Thread::new(env.clone());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        register(&mut thread, None);
        let src = "local c = Color.new(1, 0.5, 2, 1) \
                   local v = Vector2.new(3, 4) \
                   local w = Vector3f.new(1, 2, 2) \
                   SystemDisabler.setUncappedFPS(true) \
                   return c:getR(), c:getB(), c:getRed(), v:getLength(), w:length(), SystemDisabler.getUncappedFPS(), \
                          Color.new(255):getAlpha(), SystemDisabler.doCharacterStats";
        let proto = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(proto), env));
        let r = thread.call(&f, &[]).unwrap();
        let n = |i: usize| match r[i] {
            Value::Num(x) => x,
            _ => f64::NAN,
        };
        assert_eq!((n(0), n(1), n(2), n(3), n(4)), (1.0, 1.0, 255.0, 5.0, 3.0));
        assert!(r[5].raw_equals(&Value::Bool(true)));
        assert_eq!(n(6), 255.0);
        assert!(r[7].raw_equals(&Value::Bool(true)));
    }
}
