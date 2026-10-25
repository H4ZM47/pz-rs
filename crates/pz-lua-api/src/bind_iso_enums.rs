//! `zombie.iso.IsoDirections`, `zombie.iso.SpriteDetails.IsoFlagType` and
//! `zombie.iso.SpriteDetails.IsoObjectType`, plus the `IsoGridSquare`
//! static fields that shared Lua touches while loading.
//!
//! The constants (names, order and per-constant constructor values such as
//! a direction's `dx`/`dy` or a flag's `index`) are read at start-up from
//! the installed `projectzomboid.jar` with `pz-classfile`; nothing about
//! them is kept here.

use crate::bind_java_util::{list_value, JavaList};
use crate::expose::{
    enum_value, expose_class, expose_enum, int_arg, num_arg, str_arg, with_this, JavaEnum,
};
use crate::LuaManager;
use pz_classfile::{Jar, Value as ClassValue};
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::cell::RefCell;
use std::rc::Rc;

pub const ISO_DIRECTIONS: &str = "zombie.iso.IsoDirections";
pub const ISO_FLAG_TYPE: &str = "zombie.iso.SpriteDetails.IsoFlagType";
pub const ISO_OBJECT_TYPE: &str = "zombie.iso.SpriteDetails.IsoObjectType";
pub const ISO_GRID_SQUARE: &str = "zombie.iso.IsoGridSquare";

/// An enum constant: its name and the integer constructor arguments that
/// follow the implicit name and ordinal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumConstant {
    pub name: String,
    pub args: Vec<i64>,
}

/// Constants in `values()` order from the declared names and the
/// initializer's `new C(name, ordinal, args...)` calls.
pub fn enum_constants_from(
    class: &str,
    names: &[String],
    ctors: &[(String, Vec<ClassValue>)],
) -> Result<Vec<EnumConstant>, String> {
    names
        .iter()
        .enumerate()
        .map(|(ordinal, name)| {
            let (_, args) = ctors
                .iter()
                .find(|(f, _)| f == name)
                .ok_or_else(|| format!("{class}.{name}: no constructor call"))?;
            match args.get(..2) {
                Some([ClassValue::Str(n), ClassValue::Int(o)])
                    if n == name && *o == ordinal as i64 => {}
                _ => {
                    return Err(format!(
                        "{class}.{name}: unexpected enum constructor arguments {args:?}"
                    ))
                }
            }
            let rest = args[2..]
                .iter()
                .map(|a| match a {
                    ClassValue::Int(i) => Ok(*i),
                    other => Err(format!("{class}.{name}: non-constant argument {other:?}")),
                })
                .collect::<Result<_, _>>()?;
            Ok(EnumConstant {
                name: name.clone(),
                args: rest,
            })
        })
        .collect()
}

/// Read enum `class` (`a/b/C`) from the jar.
pub fn read_enum(jar: &Jar, class: &str) -> Result<Vec<EnumConstant>, String> {
    let names = pz_classfile::enum_constants(jar, class).map_err(|e| format!("{class}: {e}"))?;
    let ctors = pz_classfile::self_constructed_constants(jar, class)
        .map_err(|e| format!("{class}: {e}"))?;
    enum_constants_from(class, &names, &ctors)
}

/// Everything this module reads from the jar.
#[derive(Clone, Debug, Default)]
pub struct IsoClassData {
    pub directions: Vec<EnumConstant>,
    pub flag_types: Vec<EnumConstant>,
    pub object_types: Vec<EnumConstant>,
    /// What `IsoGridSquare.<clinit>` stores into `ignoreBlockingSprites`.
    pub ignore_blocking_sprites: Option<ClassValue>,
}

impl IsoClassData {
    pub fn read(jar: &Jar) -> (IsoClassData, Vec<String>) {
        let mut errors = Vec::new();
        let mut get = |class: &str| {
            read_enum(jar, class).unwrap_or_else(|e| {
                errors.push(e);
                Vec::new()
            })
        };
        let directions = get("zombie/iso/IsoDirections");
        let flag_types = get("zombie/iso/SpriteDetails/IsoFlagType");
        let object_types = get("zombie/iso/SpriteDetails/IsoObjectType");
        let ignore_blocking_sprites =
            pz_classfile::static_value(jar, "zombie/iso/IsoGridSquare", "ignoreBlockingSprites")
                .unwrap_or_else(|e| {
                    errors.push(format!("IsoGridSquare.ignoreBlockingSprites: {e}"));
                    None
                });
        (
            IsoClassData {
                directions,
                flag_types,
                object_types,
                ignore_blocking_sprites,
            },
            errors,
        )
    }
}

thread_local! {
    static IGNORE_BLOCKING_SPRITES: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// The contents of `IsoGridSquare.ignoreBlockingSprites` (sprite names Lua
/// adds; `isBlockedTo`-style wall checks skip them).
pub fn ignore_blocking_sprites() -> Vec<String> {
    IGNORE_BLOCKING_SPRITES.with(|l| match &*l.borrow() {
        Some(Value::Object(o)) => o
            .as_any()
            .downcast_ref::<JavaList>()
            .map(|l| l.items.borrow().iter().filter_map(str_value).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    })
}

fn str_value(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

/// Read the jar next to `media/` and register everything.
pub fn register(mgr: &mut LuaManager) {
    let jar_path = mgr.fs.borrow().base.join("projectzomboid.jar");
    let data = match Jar::open(&jar_path) {
        Ok(jar) => {
            let (data, errors) = IsoClassData::read(&jar);
            for e in errors {
                (mgr.thread.log)(&format!("bind_iso_enums: {e}"));
            }
            data
        }
        Err(e) => {
            (mgr.thread.log)(&format!("bind_iso_enums: {}: {e}", jar_path.display()));
            IsoClassData::default()
        }
    };
    register_with(&mut mgr.thread, &data);
}

/// Register the classes from already-read data.
pub fn register_with(thread: &mut Thread, data: &IsoClassData) {
    register_directions(thread, &data.directions);
    register_indexed(thread, ISO_FLAG_TYPE, &data.flag_types);
    register_indexed(thread, ISO_OBJECT_TYPE, &data.object_types);
    register_grid_square_statics(thread, data.ignore_blocking_sprites.as_ref());
}

fn leak_names(constants: &[EnumConstant]) -> Vec<&'static str> {
    constants
        .iter()
        .map(|c| &*Box::leak(c.name.clone().into_boxed_str()))
        .collect()
}

fn ordinal(c: &NativeCall<'_>) -> LuaResult<i32> {
    with_this::<JavaEnum, _>(c, |e| e.ordinal)
}

fn values_of(fqn: &str, constants: &[EnumConstant]) -> Rc<Vec<Value>> {
    Rc::new(constants.iter().map(|k| enum_value(fqn, &k.name)).collect())
}

fn index_out_of_bounds(i: i32, len: usize) -> LuaError {
    LuaError::Runtime {
        message: format!("Index {i} out of bounds for length {len}"),
        class: "java.lang.ArrayIndexOutOfBoundsException",
    }
}

// ---- IsoDirections ----------------------------------------------------------------

/// `PZMath.fastfloor`.
fn fastfloor(v: f32) -> i32 {
    let xi = v as i32;
    if v < xi as f32 {
        xi - 1
    } else {
        xi
    }
}

/// `PZMath.wrap(val, min, max)`.
pub fn pz_wrap(val: f32, min: f32, max: f32) -> f32 {
    let real_max = if max > min { max } else { min };
    let real_min = if max > min { min } else { max };
    let range = real_max - real_min;
    let rel = val - real_min;
    let frac = |v: f32| v - fastfloor(v) as f32;
    let wrapped = if range <= 0.0 {
        0.0
    } else if rel < 0.0 {
        (1.0 - frac(-rel / range)) * range
    } else {
        frac(rel / range) * range
    };
    real_min + wrapped
}

/// `IsoDirections.safeFromAngle`: index into `values()`.
pub fn from_angle_index(radians: f32) -> usize {
    ((6 - (8.5f32 + 1.273_239_5f32 * radians) as i32) & 7) as usize
}

/// `IsoDirections.safeCardinalFromAngle`.
pub fn cardinal_from_angle_index(radians: f32) -> usize {
    (2 * ((3 - (4.5f32 + 0.636_619_75f32 * radians) as i32) & 3)) as usize
}

/// `IsoDirections` constructor: `angle = ordinal * (2 * (float) PI) / 8`.
pub fn direction_angle(ordinal: i32) -> f32 {
    ordinal as f32 * (std::f32::consts::PI * 2.0) / 8.0
}

/// The angle argument(s) of `fromAngle` / `cardinalFromAngle`: one float
/// (wrapped to [-pi, pi]) or `dx, dy` (`atan2`).
fn angle_arg(c: &NativeCall<'_>) -> f32 {
    let pi = std::f64::consts::PI as f32;
    if c.nargs() >= 2 {
        let (dx, dy) = (num_arg(c, 0) as f32, num_arg(c, 1) as f32);
        (dy as f64).atan2(dx as f64) as f32
    } else {
        pz_wrap(num_arg(c, 0) as f32, -pi, pi)
    }
}

fn register_directions(thread: &mut Thread, constants: &[EnumConstant]) {
    let b = expose_enum(thread, ISO_DIRECTIONS, &leak_names(constants));
    let values = values_of(ISO_DIRECTIONS, constants);
    let deltas: Rc<Vec<(i32, i32)>> = Rc::new(
        constants
            .iter()
            .map(|k| {
                (
                    k.args.first().copied().unwrap_or(0) as i32,
                    k.args.get(1).copied().unwrap_or(0) as i32,
                )
            })
            .collect(),
    );
    let at = {
        let values = values.clone();
        move |i: i32| values.get((i & 7) as usize).cloned().unwrap_or_default()
    };
    {
        let at = at.clone();
        b.static_fn("fromIndex", move |c| Ok(c.push(at(int_arg(c, 0)))));
    }
    {
        let values = values.clone();
        b.static_fn("fromString", move |c| {
            let s = str_arg(c, 0).ok_or_else(|| LuaError::Runtime { message: "str is null".into(), class: "java.lang.NullPointerException" })?;
            let name = s.trim_matches(|ch: char| ch <= ' ').to_uppercase();
            values
                .iter()
                .find(|v| matches!(v, Value::Object(o) if o.as_any().downcast_ref::<JavaEnum>().is_some_and(|e| e.name == name)))
                .cloned()
                .map(|v| c.push(v))
                .ok_or_else(|| LuaError::Runtime {
                    message: format!("No enum constant {ISO_DIRECTIONS}.{name}"),
                    class: "java.lang.IllegalArgumentException",
                })
        });
    }
    {
        let at = at.clone();
        b.static_fn("fromAngle", move |c| {
            Ok(c.push(at(from_angle_index(angle_arg(c)) as i32)))
        });
    }
    {
        let at = at.clone();
        b.static_fn("cardinalFromAngle", move |c| {
            Ok(c.push(at(cardinal_from_angle_index(angle_arg(c)) as i32)))
        });
    }
    for (name, step, times_sign) in [("RotLeft", 1, 1), ("RotRight", -1, -1)] {
        let at = at.clone();
        b.method(name, move |c| {
            let o = ordinal(c)?;
            let d = if c.nargs() >= 2 {
                times_sign * int_arg(c, 1)
            } else {
                step
            };
            Ok(c.push(at(o + d)))
        });
    }
    {
        let at = at.clone();
        b.method("Rot180", move |c| {
            let o = ordinal(c)?;
            Ok(c.push(at(o + 4)))
        });
    }
    type Delta = fn((i32, i32)) -> Value;
    let delta_methods: [(&str, Delta); 4] = [
        ("dx", |(dx, _)| Value::from(dx)),
        ("dy", |(_, dy)| Value::from(dy)),
        ("isCardinal", |(dx, dy)| Value::from(dx == 0 || dy == 0)),
        ("isDiagonal", |(dx, dy)| Value::from(dx != 0 && dy != 0)),
    ];
    for (name, f) in delta_methods {
        let deltas = deltas.clone();
        b.method(name, move |c| {
            let o = ordinal(c)?;
            let d = deltas.get(o as usize).copied().unwrap_or((0, 0));
            Ok(c.push(f(d)))
        });
    }
    b.method("toAngle", |c| {
        let o = ordinal(c)?;
        Ok(c.push(direction_angle(o) as f64))
    });
    b.method("toAngleDegrees", |c| {
        let o = ordinal(c)?;
        Ok(c.push((direction_angle(o) * 57.295_776f32) as f64))
    });
}

// ---- IsoFlagType / IsoObjectType -----------------------------------------------------

/// `FromString(str)`: constants before `MAX` by name, otherwise `MAX`.
pub fn from_string_index(constants: &[EnumConstant], s: Option<&str>) -> Option<usize> {
    let max = constants.iter().position(|k| k.name == "MAX");
    let named = s.and_then(|s| {
        constants[..max.unwrap_or(constants.len())]
            .iter()
            .position(|k| k.name == s)
    });
    named.or(max)
}

fn register_indexed(thread: &mut Thread, fqn: &'static str, constants: &[EnumConstant]) {
    let b = expose_enum(thread, fqn, &leak_names(constants));
    let values = values_of(fqn, constants);
    let indices: Rc<Vec<i32>> = Rc::new(
        constants
            .iter()
            .map(|k| k.args.first().copied().unwrap_or(0) as i32)
            .collect(),
    );
    b.method("index", move |c| {
        let o = ordinal(c)?;
        Ok(c.push(indices.get(o as usize).copied().unwrap_or(0)))
    });
    {
        let values = values.clone();
        b.static_fn("fromIndex", move |c| {
            let i = int_arg(c, 0);
            let v = usize::try_from(i).ok().and_then(|i| values.get(i)).cloned();
            v.map(|v| c.push(v))
                .ok_or_else(|| index_out_of_bounds(i, values.len()))
        });
    }
    let constants = Rc::new(constants.to_vec());
    b.static_fn("FromString", move |c| {
        let s = str_arg(c, 0);
        let v = from_string_index(&constants, s.as_deref())
            .and_then(|i| values.get(i))
            .cloned()
            .unwrap_or_default();
        Ok(c.push(v))
    });
}

// ---- IsoGridSquare statics -------------------------------------------------------------

/// Static fields of `IsoGridSquare` that load-time Lua reads. The class
/// table is shared with the instance bindings: `expose_class` fetches it if
/// it already exists and `field` never overwrites a value set there.
fn register_grid_square_statics(thread: &mut Thread, ignore_blocking: Option<&ClassValue>) {
    let b = expose_class(thread, ISO_GRID_SQUARE, Some("java.lang.Object"));
    // `ignoreBlockingSprites = new ArrayList()`.
    if let Some(ClassValue::New { class, args }) = ignore_blocking {
        if class == "java/util/ArrayList" && args.is_empty() {
            let list = list_value(Vec::new());
            b.field("ignoreBlockingSprites", list);
            let stored = b.statics.get_str("ignoreBlockingSprites");
            IGNORE_BLOCKING_SPRITES.with(|l| *l.borrow_mut() = Some(stored));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    fn ctor(name: &str, ordinal: i64, rest: &[i64]) -> (String, Vec<ClassValue>) {
        let mut args = vec![ClassValue::Str(name.into()), ClassValue::Int(ordinal)];
        args.extend(rest.iter().map(|&i| ClassValue::Int(i)));
        (name.to_string(), args)
    }

    fn k(name: &str, args: &[i64]) -> EnumConstant {
        EnumConstant {
            name: name.into(),
            args: args.to_vec(),
        }
    }

    /// A made-up four-way compass, not the game's table.
    fn compass() -> Vec<EnumConstant> {
        vec![
            k("UP", &[0, -1]),
            k("UL", &[-1, -1]),
            k("LEFT", &[-1, 0]),
            k("DL", &[-1, 1]),
            k("DOWN", &[0, 1]),
            k("DR", &[1, 1]),
            k("RIGHT", &[1, 0]),
            k("UR", &[1, -1]),
        ]
    }

    fn data() -> IsoClassData {
        IsoClassData {
            directions: compass(),
            flag_types: vec![
                k("alpha", &[0]),
                k("beta", &[1]),
                k("MAX", &[2]),
                k("after", &[3]),
            ],
            object_types: vec![k("one", &[0]), k("MAX", &[1])],
            ignore_blocking_sprites: Some(ClassValue::New {
                class: "java/util/ArrayList".into(),
                args: vec![],
            }),
        }
    }

    fn run(thread: &mut Thread, src: &str) -> Vec<Value> {
        let proto = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(proto),
            thread.globals().clone(),
        ));
        thread.call(&f, &[]).unwrap_or_else(|e| panic!("{e}"))
    }

    fn thread() -> Thread {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register_with(&mut t, &data());
        t
    }

    #[test]
    fn constants_from_constructor_calls() {
        let names = vec!["A".to_string(), "B".to_string()];
        let ctors = vec![ctor("B", 1, &[7]), ctor("A", 0, &[5, -2])];
        let got = enum_constants_from("x/E", &names, &ctors).unwrap();
        assert_eq!(got, vec![k("A", &[5, -2]), k("B", &[7])]);
        let bad = vec![ctor("A", 1, &[]), ctor("B", 1, &[])];
        assert!(enum_constants_from("x/E", &names, &bad).is_err());
        assert!(enum_constants_from("x/E", &names, &ctors[..1]).is_err());
    }

    #[test]
    fn angles_match_java_float_math() {
        assert_eq!(from_angle_index(0.0), 6);
        assert_eq!(from_angle_index(std::f32::consts::FRAC_PI_2), 4);
        assert_eq!(from_angle_index(-std::f32::consts::FRAC_PI_2), 0);
        assert_eq!(cardinal_from_angle_index(0.0), 6);
        assert_eq!(cardinal_from_angle_index(std::f32::consts::FRAC_PI_2), 4);
        assert_eq!(cardinal_from_angle_index(std::f32::consts::PI), 2);
        let pi = std::f32::consts::PI;
        assert!(
            (pz_wrap(3.0 * pi, -pi, pi) - pi).abs() < 1e-5
                || (pz_wrap(3.0 * pi, -pi, pi) + pi).abs() < 1e-5
        );
        assert!((pz_wrap(0.5, -pi, pi) - 0.5).abs() < 1e-5);
        assert_eq!(direction_angle(2), std::f32::consts::FRAC_PI_2);
    }

    #[test]
    fn from_string_stops_at_max() {
        let d = data();
        assert_eq!(from_string_index(&d.flag_types, Some("beta")), Some(1));
        assert_eq!(from_string_index(&d.flag_types, Some("after")), Some(2));
        assert_eq!(from_string_index(&d.flag_types, None), Some(2));
    }

    #[test]
    fn lua_sees_directions() {
        let mut t = thread();
        let r = run(
            &mut t,
            "local d = IsoDirections.DR
             return d:dx(), d:dy(), d:RotLeft() == IsoDirections.RIGHT, d:RotRight(2) == IsoDirections.DL,
                    d:Rot180() == IsoDirections.UL, IsoDirections.fromIndex(9) == IsoDirections.UL,
                    d:ordinal(), tostring(IsoDirections.fromString(' dl ')), d:isDiagonal(),
                    IsoDirections.fromAngle(1, 0) == IsoDirections.RIGHT",
        );
        let nums: Vec<String> = r.iter().map(pz_kahlua::exec::java_value_of).collect();
        assert_eq!(
            nums,
            ["1.0", "1.0", "true", "true", "true", "true", "5.0", "DL", "true", "true"]
        );
    }

    #[test]
    fn lua_sees_flags_and_square_statics() {
        let mut t = thread();
        let r = run(
            &mut t,
            "IsoGridSquare.ignoreBlockingSprites:add('sign_1')
             return IsoFlagType.beta:index(), IsoFlagType.FromString('nope') == IsoFlagType.MAX,
                    IsoObjectType.fromIndex(1) == IsoObjectType.MAX, pcall(IsoFlagType.fromIndex, 9)",
        );
        assert_eq!(pz_kahlua::exec::java_value_of(&r[0]), "1.0");
        assert!(r[1].truthy() && r[2].truthy());
        assert!(!r[3].truthy());
        assert_eq!(ignore_blocking_sprites(), ["sign_1"]);
    }

    #[test]
    fn grid_square_field_composes_with_existing_class() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        let b = expose_class(&mut t, ISO_GRID_SQUARE, Some("java.lang.Object"));
        b.static_fn("getNew", |c| Ok(c.push(Value::Nil)));
        register_with(&mut t, &data());
        let g = t.globals().get_str("IsoGridSquare");
        let g = g.as_table().unwrap();
        assert!(!g.get_str("getNew").is_nil());
        assert!(!g.get_str("ignoreBlockingSprites").is_nil());
    }
}
