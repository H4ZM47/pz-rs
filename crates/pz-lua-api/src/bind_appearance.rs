//! Lua bindings for the population tables (`pz_characters::population`):
//! the exposed `HairStyles`/`HairStyle`, `BeardStyles`/`BeardStyle` and
//! `VoiceStyles`/`VoiceStyle`, and the `LuaManager.GlobalObject` globals
//! `getHairStylesInstance`, `getBeardStylesInstance`,
//! `getVoiceStylesInstance`, `getAllHairStyles`, `getAllBeardStyles`,
//! `getAllVoiceStyles` and `getAllOutfits`.
//!
//! The state is a shared [`Population`] ([`shared_population`],
//! [`install_population`]) the engine fills with `Population::init`.
//! `HairOutfitDefinitions` and `DefaultClothing` are read from Lua's tables
//! of the same names when marked dirty (every `RunLua`, as in the
//! original) and next used ([`check_dirty`]).
//!
//! Style objects are canonical (one Lua object per list entry), so Lua `==`
//! matches the original's identity. The lists they return are read-only
//! live views (`getAllMaleStyles`, `getTrimChoices`; the original's are its
//! mutable fields). The classes' statics (`init`, `Reset`, `Parse`,
//! `parse`) are not exposed.
//!
//! `IsoWorld.getWorldAgeDays()` comes from the shared `GameTime` with
//! `getTimeSinceApo()` taken as 1. TODO(SandboxOptions).

use crate::bind_climate::shared_game_time;
use crate::bind_java_util::{list_value, JavaList};
use crate::bind_text::translate;
use crate::expose::{bool_arg, expose_class, global, str_arg, with_this};
use pz_characters::population::{
    BeardStyle, HairOutfitDefinitions, HairStyle, HaircutDefinition, HaircutOutfitDefinition,
    Population, VoiceStyle,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_scripts::java::compare;
use std::any::Any;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;

pub const HAIR_STYLES_CLASS: &str = "zombie.core.skinnedmodel.population.HairStyles";
pub const HAIR_STYLE_CLASS: &str = "zombie.core.skinnedmodel.population.HairStyle";
pub const BEARD_STYLES_CLASS: &str = "zombie.core.skinnedmodel.population.BeardStyles";
pub const BEARD_STYLE_CLASS: &str = "zombie.core.skinnedmodel.population.BeardStyle";
pub const VOICE_STYLES_CLASS: &str = "zombie.core.skinnedmodel.population.VoiceStyles";
pub const VOICE_STYLE_CLASS: &str = "zombie.core.skinnedmodel.population.VoiceStyle";

thread_local! {
    static SHARED: RefCell<Option<Rc<RefCell<Population>>>> = const { RefCell::new(None) };
    static HANDLES: RefCell<HashMap<Key, Value>> = RefCell::new(HashMap::new());
}

/// Make `pop` the state the bindings use.
pub fn install_population(pop: Rc<RefCell<Population>>) {
    SHARED.with(|s| *s.borrow_mut() = Some(pop));
    HANDLES.with(|h| h.borrow_mut().clear());
}

/// The shared population tables, created empty on first use.
pub fn shared_population() -> Rc<RefCell<Population>> {
    SHARED.with(|s| s.borrow_mut().get_or_insert_with(Default::default).clone())
}

/// `LuaManager.RunLua`'s `DefaultClothing.instance.dirty = true;
/// HairOutfitDefinitions.instance.dirty = true`.
pub fn mark_lua_definitions_dirty() {
    let pop = shared_population();
    let Ok(mut p) = pop.try_borrow_mut() else {
        return;
    };
    p.hair_outfit_definitions.dirty = true;
    p.default_clothing.dirty = true;
}

/// `checkDirty()` of `HairOutfitDefinitions` and `DefaultClothing`: reload
/// them from Lua's globals if a Lua file ran since the last load.
pub fn check_dirty(thread: &Thread, pop: &mut Population) {
    let globals = thread.globals().clone();
    if pop.hair_outfit_definitions.dirty {
        pop.hair_outfit_definitions = hair_outfit_definitions_from_lua(&globals);
    }
    if pop.default_clothing.dirty {
        pop.default_clothing.dirty = false;
        let d = &mut pop.default_clothing;
        for l in [
            &mut d.pants,
            &mut d.tshirt,
            &mut d.tshirt_decal,
            &mut d.vest,
        ] {
            l.hue.clear();
            l.texture.clear();
            l.tint.clear();
        }
        if let Value::Table(defaults) = globals.get_str("DefaultClothing") {
            for (key, lists) in [
                ("Pants", &mut d.pants),
                ("TShirt", &mut d.tshirt),
                ("TShirtDecal", &mut d.tshirt_decal),
                ("Vest", &mut d.vest),
            ] {
                let Value::Table(t) = defaults.get_str(key) else {
                    continue;
                };
                for (k, list) in [
                    ("hue", &mut lists.hue),
                    ("texture", &mut lists.texture),
                    ("tint", &mut lists.tint),
                ] {
                    let Value::Table(items) = t.rawget(&Value::str(k)) else {
                        continue;
                    };
                    for i in 1..=items.len() {
                        let v = items.rawget(&Value::Num(i as f64));
                        if !v.is_nil() {
                            list.push(java_to_string(&v));
                        }
                    }
                }
            }
        }
    }
}

fn java_to_string(v: &Value) -> String {
    match v {
        Value::Str(s) => s.to_string(),
        other => pz_kahlua::exec::java_value_of(other),
    }
}

fn str_field(t: &TableRef, k: &str) -> Option<String> {
    match t.rawget(&Value::str(k)) {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

/// `HairOutfitDefinitions.init()` over Lua's `HairOutfitDefinitions`.
/// Entries the original would fail on later (missing name, outfit or
/// `onlyFor`, bad chances) are logged and skipped.
pub fn hair_outfit_definitions_from_lua(globals: &TableRef) -> HairOutfitDefinitions {
    let mut defs = HairOutfitDefinitions {
        dirty: false,
        ..Default::default()
    };
    let Value::Table(def) = globals.get_str("HairOutfitDefinitions") else {
        return defs;
    };
    let Value::Table(cuts) = def.rawget(&Value::str("haircutDefinition")) else {
        return defs;
    };
    for (_, v) in cuts.entries() {
        let Value::Table(t) = v else { continue };
        let min_age = match t.rawget(&Value::str("minWorldAge")) {
            Value::Num(n) => n as i32,
            _ => -1,
        };
        match (str_field(&t, "name"), str_field(&t, "onlyFor")) {
            (Some(name), Some(only)) => defs
                .haircut_definition
                .push(HaircutDefinition::new(name, min_age, &only)),
            _ => {
                eprintln!("HairOutfitDefinitions.haircutDefinition: entry without name or onlyFor")
            }
        }
    }
    let Value::Table(outfits) = def.rawget(&Value::str("haircutOutfitDefinition")) else {
        return defs;
    };
    for (_, v) in outfits.entries() {
        let Value::Table(t) = v else { continue };
        let Some(outfit) = str_field(&t, "outfit") else {
            eprintln!("HairOutfitDefinitions.haircutOutfitDefinition: entry without outfit");
            continue;
        };
        let f = |k| str_field(&t, k);
        match HaircutOutfitDefinition::new(
            outfit,
            f("haircut").as_deref(),
            f("femaleHaircut").as_deref(),
            f("maleHaircut").as_deref(),
            f("beard").as_deref(),
            f("haircutColor").as_deref(),
        ) {
            Ok(d) => defs.outfit_definition.push(d),
            Err(e) => eprintln!("HairOutfitDefinitions.haircutOutfitDefinition: {e}"),
        }
    }
    defs
}

fn world_age_days() -> f32 {
    shared_game_time().borrow().world_age_days_since_begin(1) as f32
}

// ---- objects -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    HairStyles,
    BeardStyles,
    VoiceStyles,
    Hair(bool, usize),
    Beard(usize),
    Voice(usize),
}

/// A population object: a table instance or one entry of its lists.
pub struct AppearanceObject {
    key: Key,
    pop: Rc<RefCell<Population>>,
}

impl LuaObject for AppearanceObject {
    fn class_name(&self) -> &str {
        match self.key {
            Key::HairStyles => HAIR_STYLES_CLASS,
            Key::BeardStyles => BEARD_STYLES_CLASS,
            Key::VoiceStyles => VOICE_STYLES_CLASS,
            Key::Hair(..) => HAIR_STYLE_CLASS,
            Key::Beard(_) => BEARD_STYLE_CLASS,
            Key::Voice(_) => VOICE_STYLE_CLASS,
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn object(key: Key) -> Value {
    HANDLES.with(|h| {
        h.borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                Value::Object(Rc::new(AppearanceObject {
                    key,
                    pop: shared_population(),
                }))
            })
            .clone()
    })
}

fn opt_object(key: Option<Key>) -> Value {
    key.map_or(Value::Nil, object)
}

fn this_key(c: &NativeCall<'_>) -> LuaResult<(Key, Rc<RefCell<Population>>)> {
    with_this::<AppearanceObject, _>(c, |o| (o.key, o.pop.clone()))
}

fn npe() -> LuaError {
    LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    }
}

fn opt_str(s: Option<&str>) -> Value {
    s.map_or(Value::Nil, Value::str)
}

fn hair<R>(c: &NativeCall<'_>, f: impl FnOnce(&HairStyle) -> R) -> LuaResult<R> {
    let (key, pop) = this_key(c)?;
    let Key::Hair(female, i) = key else {
        return Err(npe());
    };
    let p = pop.borrow();
    let s = p
        .hair_styles
        .as_ref()
        .and_then(|h| h.get(female).get(i))
        .ok_or_else(npe)?;
    Ok(f(s))
}

fn beard<R>(c: &NativeCall<'_>, f: impl FnOnce(&BeardStyle) -> R) -> LuaResult<R> {
    let (key, pop) = this_key(c)?;
    let Key::Beard(i) = key else {
        return Err(npe());
    };
    let p = pop.borrow();
    let s = p
        .beard_styles
        .as_ref()
        .and_then(|b| b.styles.get(i))
        .ok_or_else(npe)?;
    Ok(f(s))
}

fn voice<R>(c: &NativeCall<'_>, f: impl FnOnce(&VoiceStyle) -> R) -> LuaResult<R> {
    let (key, pop) = this_key(c)?;
    let Key::Voice(i) = key else {
        return Err(npe());
    };
    let p = pop.borrow();
    let s = p
        .voice_styles
        .as_ref()
        .and_then(|v| v.styles.get(i))
        .ok_or_else(npe)?;
    Ok(f(s))
}

/// A read-only live `ArrayList` over the current values of `read`.
fn live_list(read: impl Fn() -> Vec<Value> + 'static) -> Value {
    Value::Object(Rc::new(JavaList::live(read, None)))
}

fn trim_choices(key: Key) -> Value {
    let pop = shared_population();
    live_list(move || {
        let p = pop.borrow();
        let list = match key {
            Key::Hair(f, i) => p
                .hair_styles
                .as_ref()
                .and_then(|h| h.get(f).get(i))
                .map(|s| s.trim_choices.clone()),
            Key::Beard(i) => p
                .beard_styles
                .as_ref()
                .and_then(|b| b.styles.get(i))
                .map(|s| s.trim_choices.clone()),
            _ => None,
        };
        list.unwrap_or_default()
            .iter()
            .map(|s| Value::str(s))
            .collect()
    })
}

fn style_list(female: Option<bool>, voice: bool) -> Value {
    let pop = shared_population();
    live_list(move || {
        let p = pop.borrow();
        let n = match (female, voice) {
            (Some(f), _) => p.hair_styles.as_ref().map_or(0, |h| h.get(f).len()),
            (None, false) => p.beard_styles.as_ref().map_or(0, |b| b.styles.len()),
            (None, true) => p.voice_styles.as_ref().map_or(0, |v| v.styles.len()),
        };
        (0..n)
            .map(|i| match (female, voice) {
                (Some(f), _) => object(Key::Hair(f, i)),
                (None, false) => object(Key::Beard(i)),
                (None, true) => object(Key::Voice(i)),
            })
            .collect()
    })
}

/// `List.sort` with the original's comparators, as a stable binary
/// insertion sort (it tolerates their inconsistency for two empty names).
fn java_sort<T>(v: &mut Vec<T>, cmp: impl Fn(&T, &T) -> Ordering) {
    for i in 1..v.len() {
        let (mut lo, mut hi) = (0, i);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if cmp(&v[i], &v[mid]) == Ordering::Less {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let x = v.remove(i);
        v.insert(lo, x);
    }
}

/// The `getAll*Styles` comparator over `(name, translated name, ..)`: an
/// empty name first, else by the translation.
fn by_translated_name(a: &(String, String, usize), b: &(String, String, usize)) -> Ordering {
    if a.0.is_empty() {
        return Ordering::Less;
    }
    if b.0.is_empty() {
        return Ordering::Greater;
    }
    compare(&a.1, &b.1)
}

/// `names` with their indices, sorted by the translated `<prefix><name>`.
fn sorted_names(names: Vec<String>, prefix: &str) -> Vec<(String, String, usize)> {
    let mut v: Vec<(String, String, usize)> = names
        .into_iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), translate(&format!("{prefix}{n}")), i))
        .collect();
    java_sort(&mut v, by_translated_name);
    v
}

// ---- registration --------------------------------------------------------

/// Register the classes and globals.
pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, HAIR_STYLE_CLASS, None);
    b.method("isValid", |c| {
        let v = hair(c, HairStyle::is_valid)?;
        Ok(c.push(v))
    });
    b.method("getAlternate", |c| {
        let cat = str_arg(c, 1).ok_or_else(npe)?;
        let v = hair(c, |s| opt_str(s.get_alternate(&cat)))?;
        Ok(c.push(v))
    });
    b.method("getLevel", |c| {
        let v = hair(c, |s| s.level as f64)?;
        Ok(c.push(v))
    });
    b.method("getName", |c| {
        let v = hair(c, |s| Value::str(&s.name))?;
        Ok(c.push(v))
    });
    b.method("getTrimChoices", |c| {
        let (key, _) = this_key(c)?;
        Ok(c.push(trim_choices(key)))
    });
    b.method("isAttachedHair", |c| {
        let v = hair(c, |s| s.attached_hair)?;
        Ok(c.push(v))
    });
    b.method("isGrowReference", |c| {
        let v = hair(c, |s| s.grow_reference)?;
        Ok(c.push(v))
    });
    b.method("isNoChoose", |c| {
        let v = hair(c, |s| s.no_choose)?;
        Ok(c.push(v))
    });

    let b = expose_class(thread, HAIR_STYLES_CLASS, None);
    for (name, female) in [("FindMaleStyle", false), ("FindFemaleStyle", true)] {
        b.method(name, move |c| {
            let n = str_arg(c, 1);
            let (_, pop) = this_key(c)?;
            let p = pop.borrow();
            let h = p.hair_styles.as_ref().ok_or_else(npe)?;
            let i = h
                .find_style(female, n.as_deref())
                .and_then(|s| h.get(female).iter().position(|x| std::ptr::eq(x, s)));
            drop(p);
            Ok(c.push(opt_object(i.map(|i| Key::Hair(female, i)))))
        });
    }
    for (name, female) in [
        ("getRandomMaleStyle", false),
        ("getRandomFemaleStyle", true),
    ] {
        b.method(name, move |c| {
            let outfit = str_arg(c, 1);
            let (_, pop) = this_key(c)?;
            let mut p = pop.borrow_mut();
            if p.hair_styles
                .as_ref()
                .is_some_and(|h| !h.get(female).is_empty())
            {
                check_dirty(c.thread, &mut p);
            }
            let h = p.hair_styles.as_ref().ok_or_else(npe)?;
            let defs = &p.hair_outfit_definitions;
            let age = world_age_days();
            let v = if female {
                h.get_random_female_style(defs, outfit.as_deref(), age)
            } else {
                h.get_random_male_style(defs, outfit.as_deref(), age)
            };
            drop(p);
            Ok(c.push(v))
        });
    }
    b.method("getAlternateForHat", |c| {
        let style = match c.arg(1) {
            Value::Object(o) => o.as_any().downcast_ref::<AppearanceObject>().map(|o| o.key),
            _ => None,
        };
        let cat = str_arg(c, 2);
        let (_, pop) = this_key(c)?;
        let p = pop.borrow();
        let h = p.hair_styles.as_ref().ok_or_else(npe)?;
        let Some(Key::Hair(female, i)) = style else {
            return Err(npe());
        };
        let s = h.get(female).get(i).ok_or_else(npe)?;
        let cat = cat.ok_or_else(npe)?;
        let r = h.get_alternate_for_hat(s, &cat).map(|a| {
            let idx = |f: bool| h.get(f).iter().position(|x| std::ptr::eq(x, a));
            idx(true)
                .map(|i| Key::Hair(true, i))
                .or_else(|| idx(false).map(|i| Key::Hair(false, i)))
                .unwrap_or(Key::Hair(female, i))
        });
        drop(p);
        Ok(c.push(opt_object(r)))
    });
    b.method("getAllMaleStyles", |c| {
        Ok(c.push(style_list(Some(false), false)))
    });
    b.method("getAllFemaleStyles", |c| {
        Ok(c.push(style_list(Some(true), false)))
    });

    let b = expose_class(thread, BEARD_STYLE_CLASS, None);
    b.method("isValid", |c| {
        let v = beard(c, BeardStyle::is_valid)?;
        Ok(c.push(v))
    });
    b.method("getLevel", |c| {
        let v = beard(c, |s| s.level as f64)?;
        Ok(c.push(v))
    });
    b.method("getName", |c| {
        let v = beard(c, |s| Value::str(&s.name))?;
        Ok(c.push(v))
    });
    b.method("getTrimChoices", |c| {
        let (key, _) = this_key(c)?;
        Ok(c.push(trim_choices(key)))
    });

    let b = expose_class(thread, BEARD_STYLES_CLASS, None);
    b.method("FindStyle", |c| {
        let n = str_arg(c, 1);
        let (_, pop) = this_key(c)?;
        let p = pop.borrow();
        let bs = p.beard_styles.as_ref().ok_or_else(npe)?;
        let i = bs
            .find_style(n.as_deref())
            .and_then(|s| bs.styles.iter().position(|x| std::ptr::eq(x, s)));
        drop(p);
        Ok(c.push(opt_object(i.map(Key::Beard))))
    });
    b.method("getRandomStyle", |c| {
        let outfit = str_arg(c, 1);
        let (_, pop) = this_key(c)?;
        let p = pop.borrow();
        let bs = p.beard_styles.as_ref().ok_or_else(npe)?;
        let v = bs
            .get_random_style(&p.hair_outfit_definitions, outfit.as_deref())
            .ok_or_else(npe)?;
        drop(p);
        Ok(c.push(v))
    });
    b.method("getInstance", |c| {
        let has = shared_population().borrow().beard_styles.is_some();
        Ok(c.push(if has {
            object(Key::BeardStyles)
        } else {
            Value::Nil
        }))
    });
    b.method("getAllStyles", |c| Ok(c.push(style_list(None, false))));

    let b = expose_class(thread, VOICE_STYLE_CLASS, None);
    b.method("isValid", |c| {
        let v = voice(c, VoiceStyle::is_valid)?;
        Ok(c.push(v))
    });
    b.method("getName", |c| {
        let v = voice(c, |s| Value::str(&s.name))?;
        Ok(c.push(v))
    });
    b.method("getPrefix", |c| {
        let v = voice(c, |s| Value::str(&s.prefix))?;
        Ok(c.push(v))
    });
    b.method("getBodyTypeDefault", |c| {
        let v = voice(c, |s| s.body_type_default as f64)?;
        Ok(c.push(v))
    });
    b.method("getVoiceType", |c| {
        let v = voice(c, |s| s.voice_type as f64)?;
        Ok(c.push(v))
    });

    let b = expose_class(thread, VOICE_STYLES_CLASS, None);
    b.method("FindStyle", |c| {
        let n = str_arg(c, 1);
        let (_, pop) = this_key(c)?;
        let p = pop.borrow();
        let vs = p.voice_styles.as_ref().ok_or_else(npe)?;
        let i = vs
            .find_style(n.as_deref())
            .and_then(|s| vs.styles.iter().position(|x| std::ptr::eq(x, s)));
        drop(p);
        Ok(c.push(opt_object(i.map(Key::Voice))))
    });
    b.method("getInstance", |c| {
        let has = shared_population().borrow().voice_styles.is_some();
        Ok(c.push(if has {
            object(Key::VoiceStyles)
        } else {
            Value::Nil
        }))
    });
    b.method("getAllStyles", |c| Ok(c.push(style_list(None, true))));

    global(thread, "getHairStylesInstance", |c| {
        let has = shared_population().borrow().hair_styles.is_some();
        Ok(c.push(if has {
            object(Key::HairStyles)
        } else {
            Value::Nil
        }))
    });
    global(thread, "getBeardStylesInstance", |c| {
        let has = shared_population().borrow().beard_styles.is_some();
        Ok(c.push(if has {
            object(Key::BeardStyles)
        } else {
            Value::Nil
        }))
    });
    global(thread, "getVoiceStylesInstance", |c| {
        let has = shared_population().borrow().voice_styles.is_some();
        Ok(c.push(if has {
            object(Key::VoiceStyles)
        } else {
            Value::Nil
        }))
    });
    global(thread, "getAllHairStyles", |c| {
        let female = bool_arg(c, 0);
        let names: Vec<String> = shared_population()
            .borrow()
            .hair_styles
            .as_ref()
            .map(|h| h.get(female).iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        let sorted = sorted_names(names, "IGUI_Hair_");
        Ok(c.push(list_value(
            sorted.iter().map(|s| Value::str(&s.0)).collect(),
        )))
    });
    global(thread, "getAllBeardStyles", |c| {
        let names: Vec<String> = shared_population()
            .borrow()
            .beard_styles
            .as_ref()
            .map(|b| b.styles.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        let sorted = sorted_names(names, "IGUI_Beard_");
        Ok(c.push(list_value(
            sorted.iter().map(|s| Value::str(&s.0)).collect(),
        )))
    });
    global(thread, "getAllVoiceStyles", |c| {
        let names: Vec<String> = shared_population()
            .borrow()
            .voice_styles
            .as_ref()
            .map(|v| v.styles.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        let out = sorted_names(names, "IGUI_Voice_")
            .into_iter()
            .filter(|s| !s.0.is_empty())
            .map(|s| object(Key::Voice(s.2)))
            .collect();
        Ok(c.push(list_value(out)))
    });
    global(thread, "getAllOutfits", |c| {
        let female = bool_arg(c, 0);
        let mut names: Vec<String> = shared_population()
            .borrow()
            .outfit_manager
            .as_ref()
            .map(|m| m.get(female).iter().map(|o| o.name.clone()).collect())
            .unwrap_or_default();
        names.sort_by(|a, b| compare(a, b));
        Ok(c.push(list_value(names.iter().map(|s| Value::str(s)).collect())))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_characters::population::{BeardStyles, HairStyles, VoiceStyles};

    fn run(thread: &mut Thread, src: &str) -> Vec<Value> {
        let proto = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(proto),
            thread.globals().clone(),
        ));
        thread
            .pcall(&f, &[])
            .map_err(|(m, t)| format!("{m:?}\n{t}"))
            .unwrap()
    }

    #[test]
    fn lua_reads_styles() {
        let pop = Rc::new(RefCell::new(Population::default()));
        {
            let mut p = pop.borrow_mut();
            p.hair_styles = Some(
                HairStyles::from_xml(
                    b"<hairStyles><male><name>Bald</name><model/></male><male><name>Short</name><model>m</model><level>1</level>\
                      <trimChoices>Bald</trimChoices><alternate category=\"hat\" style=\"Bald\"/></male>\
                      <female><name>Long</name><model>f</model></female></hairStyles>",
                )
                .unwrap(),
            );
            let mut b = BeardStyles::from_xml(
                b"<beardStyles><style><name>Full</name></style></beardStyles>",
            )
            .unwrap();
            b.styles.insert(0, BeardStyle::default());
            p.beard_styles = Some(b);
            let mut v = VoiceStyles::from_xml(b"<voiceStyles><style><name>Kate</name><prefix>VoiceFemale</prefix></style></voiceStyles>").unwrap();
            v.styles.insert(0, VoiceStyle::default());
            p.voice_styles = Some(v);
        }
        install_population(pop);
        let mut thread = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        crate::bind_java_util::register(&mut thread);
        register(&mut thread);
        let r = run(
            &mut thread,
            r#"
            local h = getHairStylesInstance()
            local s = h:FindMaleStyle("short")
            local alt = h:getAlternateForHat(s, "HAT")
            local all = h:getAllMaleStyles()
            return s:getName(), s:getLevel(), s:getTrimChoices():get(0), alt:getName(), alt == h:FindMaleStyle(""),
                all:size(), all:get(1) == s, getAllHairStyles(false):get(0), getAllBeardStyles():get(0),
                getAllVoiceStyles():size(), getBeardStylesInstance():FindStyle("full"):getName(), h:FindFemaleStyle("x"),
                s:isValid(), alt:isValid()
            "#,
        );
        let s = |v: &Value| pz_kahlua::exec::java_value_of(v);
        let got: Vec<String> = r.iter().map(s).collect();
        assert_eq!(
            got,
            [
                "Short", "1.0", "Bald", "Bald", "true", "2.0", "true", "Bald", "", "1.0", "Full",
                "null", "true", "false"
            ]
        );
    }
}
