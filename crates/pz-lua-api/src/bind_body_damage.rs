//! `zombie.characters.BodyDamage.Metabolics` and `BodyPartType` exposed to
//! Lua. Both are enums whose constants and constructor data (`met`, the
//! body part names, models and modifiers) are read from the installed jar
//! ([`pz_scripts::registry::builtin::BuiltinTables`]), so this file names
//! no game constants.
//!
//! `BodyPartType.GetSkinSurface` and the other `Get*` statics warn through
//! `DebugType.General` for `null`/`MAX` in the original; the warning is not
//! logged here. `getRandom` uses `Rand`'s generator for `OutfitRNG`'s.
//! TODO(DebugLog, OutfitRNG).

use crate::bind_registry::shared_script_manager;
use crate::expose::{expose_enum, int_arg, num_arg, str_arg, this, JavaEnum};
use pz_characters::Characters;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::registry::builtin::BodyPartDecl;
use pz_scripts::ScriptManager;
use std::rc::Rc;

pub const METABOLICS_CLASS: &str = "zombie.characters.BodyDamage.Metabolics";
pub const BODY_PART_TYPE_CLASS: &str = "zombie.characters.BodyDamage.BodyPartType";

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// The ordinal of `self` if it is a constant of `class`.
fn this_ordinal(c: &NativeCall<'_>, class: &str) -> LuaResult<usize> {
    let o = this::<JavaEnum>(c)?;
    let e = o.as_any().downcast_ref::<JavaEnum>().unwrap();
    if e.class != class {
        return Err(LuaError::runtime(format!("Expected {class} but got {}", e.class)));
    }
    Ok(e.ordinal as usize)
}

/// The ordinal of an enum argument of `class` (`None` for nil).
fn enum_arg(c: &NativeCall<'_>, i: usize, class: &str) -> LuaResult<Option<usize>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<JavaEnum>().filter(|e| e.class == class) {
            Some(e) => Ok(Some(e.ordinal as usize)),
            None => Err(LuaError::runtime(format!("Expected {class} but got {}", o.class_name()))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected {class} but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

/// A `float` parameter (Kahlua's `Double.floatValue()`).
fn float_arg(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

/// A `float` return value.
fn float_value(f: f32) -> Value {
    Value::Num(f64::from(f))
}

/// Register both enums from `sm`'s built-in tables.
pub fn register(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let (mets, parts) = {
        let t = sm.builtin_tables.borrow();
        (t.metabolics.clone(), t.body_parts.clone())
    };
    register_metabolics(thread, mets);
    register_body_part_type(thread, parts);
}

/// `register` on the shared ScriptManager.
pub fn register_shared(thread: &mut Thread) {
    register(thread, &shared_script_manager());
}

fn met_to_wm2(met: f32) -> f32 {
    58.0f32 * met
}

fn met_to_w(met: f32) -> f32 {
    met_to_wm2(met) * 1.8f32
}

fn met_to_btu_hr(met: f32) -> f32 {
    356.0f32 * met
}

fn register_metabolics(thread: &mut Thread, mets: Vec<(String, f32)>) {
    let names: Vec<&'static str> = mets.iter().map(|(n, _)| leak(n)).collect();
    let b = expose_enum(thread, METABOLICS_CLASS, &names);
    let mets: Rc<Vec<f32>> = Rc::new(mets.into_iter().map(|(_, m)| m).collect());
    type Conv = fn(f32) -> f32;
    let getters: [(&str, Conv); 4] =
        [("getMet", |m| m), ("getWm2", met_to_wm2), ("getW", met_to_w), ("getBtuHr", met_to_btu_hr)];
    for (name, f) in getters {
        let m = mets.clone();
        b.method(name, move |c| {
            let i = this_ordinal(c, METABOLICS_CLASS)?;
            Ok(c.push(float_value(f(m[i]))))
        });
    }
    let statics: [(&str, Conv); 3] = [("MetToWm2", met_to_wm2), ("MetToW", met_to_w), ("MetToBtuHr", met_to_btu_hr)];
    for (name, f) in statics {
        b.static_fn(name, move |c| Ok(c.push(float_value(f(float_arg(c, 0))))));
    }
}

fn register_body_part_type(thread: &mut Thread, parts: Vec<BodyPartDecl>) {
    let names: Vec<&'static str> = parts.iter().map(|p| leak(&p.field)).collect();
    let b = expose_enum(thread, BODY_PART_TYPE_CLASS, &names);
    let parts = Rc::new(parts);
    // `MAX` is the last constant.
    let max = parts.len().saturating_sub(1);
    let consts: Rc<Vec<Value>> =
        Rc::new(names.iter().map(|n| crate::expose::enum_value(BODY_PART_TYPE_CLASS, n)).collect());

    b.method("index", |c| {
        let i = this_ordinal(c, BODY_PART_TYPE_CLASS)?;
        Ok(c.push(i as f64))
    });
    let k = consts.clone();
    b.static_fn("FromIndex", move |c| {
        let i = int_arg(c, 0);
        let v = usize::try_from(i).ok().filter(|i| *i < k.len()).unwrap_or(max);
        Ok(c.push(k[v].clone()))
    });
    let (k, p) = (consts.clone(), parts.clone());
    b.static_fn("FromString", move |c| {
        let s = str_arg(c, 0);
        let i = s
            .and_then(|s| p.iter().position(|d| pz_scripts::java::eq_ignore_case(&d.name, &s)))
            .unwrap_or(max);
        Ok(c.push(k[i].clone()))
    });
    b.static_fn("ToIndex", |c| {
        let i = enum_arg(c, 0, BODY_PART_TYPE_CLASS)?.unwrap_or(0);
        Ok(c.push(i as f64))
    });
    let p = parts.clone();
    b.static_fn("ToString", move |c| {
        let i = enum_arg(c, 0, BODY_PART_TYPE_CLASS)?.unwrap_or(max);
        Ok(c.push(p[i].name.as_str()))
    });
    let p = parts.clone();
    b.static_fn("getDisplayName", move |c| {
        let i = enum_arg(c, 0, BODY_PART_TYPE_CLASS)?.unwrap_or(max);
        // `Translator.getText(null)` for a part without a key.
        let text = p[i].display_name_key.as_deref().map(Characters::translate);
        Ok(c.push(text.map(Value::from).unwrap_or_default()))
    });

    type Field = fn(&BodyPartDecl) -> f32;
    // Statics taking an index: `FromIndex(index).field`.
    let by_index: [(&str, Field); 3] = [
        ("getPainModifyer", |d| d.pain_modifier),
        ("getDamageModifyer", |d| d.damage_modifier),
        ("getBleedingTimeModifyer", |d| d.bleeding_time_modifier),
    ];
    for (name, f) in by_index {
        let p = parts.clone();
        b.static_fn(name, move |c| {
            let i = usize::try_from(int_arg(c, 0)).ok().filter(|i| *i < p.len()).unwrap_or(max);
            Ok(c.push(float_value(f(&p[i]))))
        });
    }
    // Statics taking a part: `null` and `MAX` read `MAX`'s value.
    let by_part: [(&str, Field); 5] = [
        ("GetSkinSurface", |d| d.skin_surface),
        ("GetDistToCore", |d| d.dist_to_core),
        ("GetUmbrellaMod", |d| d.umbrella_mod),
        ("GetMaxActionPenalty", |d| d.max_action_penalty),
        ("GetMaxMovementPenalty", |d| d.max_movement_penalty),
    ];
    for (name, f) in by_part {
        let p = parts.clone();
        b.static_fn(name, move |c| {
            let i = enum_arg(c, 0, BODY_PART_TYPE_CLASS)?.unwrap_or(max);
            Ok(c.push(float_value(f(&p[i]))))
        });
    }
    let p = parts.clone();
    b.method("getBandageModel", move |c| {
        let i = this_ordinal(c, BODY_PART_TYPE_CLASS)?;
        Ok(c.push(p[i].bandage_model.as_deref().map(Value::from).unwrap_or_default()))
    });
    type Model = fn(&BodyPartDecl) -> Option<&str>;
    let models: [(&str, Model); 3] = [
        ("getBiteWoundModel", |d| d.bite_wound_model.as_deref()),
        ("getScratchWoundModel", |d| d.scratch_wound_model.as_deref()),
        ("getCutWoundModel", |d| d.cut_wound_model.as_deref()),
    ];
    for (name, f) in models {
        let p = parts.clone();
        b.method(name, move |c| {
            let i = this_ordinal(c, BODY_PART_TYPE_CLASS)?;
            // `model + String.valueOf(gender)`.
            let v = f(&p[i]).map(|m| {
                let g = match c.arg(1) {
                    Value::Nil => "null".to_string(),
                    other => pz_kahlua::exec::java_value_of(&other),
                };
                Value::from(format!("{m}{g}"))
            });
            Ok(c.push(v.unwrap_or_default()))
        });
    }
    let k = consts;
    b.static_fn("getRandom", move |c| {
        let i = pz_core::random::next_range(0, max as i32) as usize;
        Ok(c.push(k[i].clone()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    fn part(field: &str, name: &str, key: Option<&str>, bite: Option<&str>, pain: f32) -> BodyPartDecl {
        BodyPartDecl {
            field: field.into(),
            name: name.into(),
            display_name_key: key.map(Into::into),
            bandage_model: bite.map(|_| format!("T.Wrap_{field}")),
            bite_wound_model: bite.map(Into::into),
            pain_modifier: pain,
            skin_surface: pain / 10.0,
            ..Default::default()
        }
    }

    #[test]
    fn enums_from_tables() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        let sm = Rc::new(ScriptManager::new());
        {
            let mut tables = sm.builtin_tables.borrow_mut();
            tables.metabolics = vec![("Napping".into(), 0.7), ("Juggling".into(), 2.5), ("MAX".into(), 9.9)];
            tables.body_parts = vec![
                part("Elbow_L", "Elbow_L", Some("UI_elbow"), Some("T.Bite_Elbow_"), 0.5),
                part("Knee_R", "Knee_R", Some("UI_knee"), None, 0.25),
                part("MAX", "Odd Part", Some("UI_odd"), None, 1.0),
            ];
        }
        register(&mut t, &sm);
        let src = r#"
            local function assert(v) if not v then error("assertion failed!", 2) end end
            local j = Metabolics.Juggling
            assert(j == Metabolics.valueOf("Juggling") and tostring(j) == "Juggling" and j:ordinal() == 1)
            assert(j:getMet() == 2.5 and j:getWm2() == 145 and j:getBtuHr() == 890)
            -- A float widened to double, as Kahlua returns it.
            local nap = Metabolics.Napping:getMet()
            assert(nap ~= 0.7 and math.abs(nap - 0.7) < 1e-6)
            assert(Metabolics.MetToWm2(1) == 58 and #Metabolics.values() == 3)

            local e = BodyPartType.Elbow_L
            assert(e:index() == 0 and BodyPartType.FromIndex(1) == BodyPartType.Knee_R)
            assert(BodyPartType.FromIndex(7) == BodyPartType.MAX and BodyPartType.FromIndex(-1) == BodyPartType.MAX)
            assert(BodyPartType.FromString("knee_r") == BodyPartType.Knee_R and BodyPartType.FromString(nil) == BodyPartType.MAX)
            assert(BodyPartType.ToIndex(nil) == 0 and BodyPartType.ToIndex(BodyPartType.MAX) == 2)
            assert(BodyPartType.ToString(nil) == "Odd Part" and tostring(BodyPartType.MAX) == "MAX")
            assert(BodyPartType.getDisplayName(e) == "UI_elbow" and BodyPartType.getDisplayName(nil) == "UI_odd")
            assert(BodyPartType.getPainModifyer(1) == 0.25 and BodyPartType.getPainModifyer(9) == 1)
            assert(BodyPartType.GetSkinSurface(nil) == BodyPartType.GetSkinSurface(BodyPartType.MAX))
            assert(e:getBandageModel() == "T.Wrap_Elbow_L" and BodyPartType.Knee_R:getBandageModel() == nil)
            assert(e:getBiteWoundModel("Female") == "T.Bite_Elbow_Female" and BodyPartType.Knee_R:getBiteWoundModel("Male") == nil)
            local r = BodyPartType.getRandom()
            assert(r == BodyPartType.Elbow_L or r == BodyPartType.Knee_R)
        "#;
        let p = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e))).unwrap();
    }
}
