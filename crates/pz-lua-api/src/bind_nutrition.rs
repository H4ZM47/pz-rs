//! `zombie.characters.BodyDamage.Nutrition`, `Thermoregulator`,
//! `Thermoregulator$ThermalNode`, `Fitness` and `Fitness$FitnessExercise`
//! exposed to Lua under the original names, over the characters of
//! [`crate::bind_character_sim`], with `IsoPlayer.getNutrition`/`getFitness`
//! and the `IsoGameCharacter` members that forward to the thermoregulator.
//!
//! The thermoregulator belongs to the character's `BodyDamage`, which is not
//! exposed yet: [`thermoregulator_value`] is the object
//! `BodyDamage.getThermoregulator()` returns. TODO(BodyDamage).
//!
//! `Fitness.init` reads `FitnessExercises.exercisesType` from the Lua
//! globals, as the original reads `LuaManager.env`.
//!
//! Not exposed: `save`/`load` (TODO(saves)), `Fitness.getRegularityMap`/
//! `setRegularityMap` (TODO(java.util.HashMap view)).

use crate::bind_body_damage::{BODY_PART_TYPE_CLASS, METABOLICS_CLASS};
use crate::bind_character_sim::{
    character_value, chr_of, with_ctx, LuaGameCharacterRef, ISO_GAME_CHARACTER_CLASS,
    ISO_PLAYER_CLASS,
};
use crate::bind_characters::shared_characters;
use crate::bind_java_util::class_cast;
use crate::bind_registry::{java_error, shared_script_manager};
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, with_this, JavaEnum};
use pz_characters::character::{FitnessExercise, Thermoregulator};
use pz_characters::Characters;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::{ScriptError, ScriptManager};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const NUTRITION_CLASS: &str = "zombie.characters.BodyDamage.Nutrition";
pub const THERMOREGULATOR_CLASS: &str = "zombie.characters.BodyDamage.Thermoregulator";
pub const THERMAL_NODE_CLASS: &str = "zombie.characters.BodyDamage.Thermoregulator$ThermalNode";
pub const FITNESS_CLASS: &str = "zombie.characters.BodyDamage.Fitness";
pub const FITNESS_EXERCISE_CLASS: &str = "zombie.characters.BodyDamage.Fitness$FitnessExercise";

thread_local! {
    static HANDLES: RefCell<HashMap<(u8, usize, usize), Value>> = RefCell::new(HashMap::new());
}

/// One Lua object per (kind, character, member), so Lua `==` is identity.
fn canonical(id: (u8, usize, usize), make: impl FnOnce() -> Value) -> Value {
    HANDLES.with(|h| h.borrow_mut().entry(id).or_insert_with(make).clone())
}

/// A player's `Nutrition`.
pub struct NutritionObject(pub LuaGameCharacterRef);
/// A player's `Thermoregulator`.
pub struct ThermoregulatorObject(pub LuaGameCharacterRef);
/// One `ThermalNode` of a player's thermoregulator.
pub struct ThermalNodeObject {
    pub chr: LuaGameCharacterRef,
    pub index: usize,
}
/// A player's `Fitness`.
pub struct FitnessObject(pub LuaGameCharacterRef);
/// A `FitnessExercise` (a snapshot; it has package-private fields only).
pub struct FitnessExerciseObject(pub FitnessExercise);

macro_rules! lua_object {
    ($t:ty, $class:expr) => {
        impl LuaObject for $t {
            fn class_name(&self) -> &str {
                $class
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
    };
}

lua_object!(NutritionObject, NUTRITION_CLASS);
lua_object!(ThermoregulatorObject, THERMOREGULATOR_CLASS);
lua_object!(ThermalNodeObject, THERMAL_NODE_CLASS);
lua_object!(FitnessObject, FITNESS_CLASS);
lua_object!(FitnessExerciseObject, FITNESS_EXERCISE_CLASS);

/// The canonical `Nutrition` of `chr` (nil for a character without one).
pub fn nutrition_value(chr: &LuaGameCharacterRef) -> Value {
    if chr.borrow().nutrition.is_none() {
        return Value::Nil;
    }
    canonical((0, Rc::as_ptr(chr) as usize, 0), || {
        Value::Object(Rc::new(NutritionObject(chr.clone())))
    })
}

/// The canonical `Thermoregulator` of `chr` (`BodyDamage.getThermoregulator()`;
/// nil for a character without one).
pub fn thermoregulator_value(chr: &LuaGameCharacterRef) -> Value {
    if chr.borrow().body_damage.thermoregulator().is_none() {
        return Value::Nil;
    }
    canonical((1, Rc::as_ptr(chr) as usize, 0), || {
        Value::Object(Rc::new(ThermoregulatorObject(chr.clone())))
    })
}

fn thermal_node_value(chr: &LuaGameCharacterRef, index: usize) -> Value {
    canonical((2, Rc::as_ptr(chr) as usize, index), || {
        Value::Object(Rc::new(ThermalNodeObject {
            chr: chr.clone(),
            index,
        }))
    })
}

/// The canonical `Fitness` of `chr` (nil for a character without one).
pub fn fitness_value(chr: &LuaGameCharacterRef) -> Value {
    if chr.borrow().fitness.is_none() {
        return Value::Nil;
    }
    canonical((3, Rc::as_ptr(chr) as usize, 0), || {
        Value::Object(Rc::new(FitnessObject(chr.clone())))
    })
}

fn float_arg(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

fn float_value(f: f32) -> Value {
    Value::Num(f64::from(f))
}

fn npe() -> LuaError {
    LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    }
}

/// The ordinal and name of an enum argument of `class` (`None` for nil).
fn enum_arg(c: &NativeCall<'_>, i: usize, class: &str) -> LuaResult<Option<(usize, &'static str)>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == class)
        {
            Some(e) => Ok(Some((e.ordinal as usize, e.name))),
            None => Err(class_cast(&c.arg(i), class)),
        },
        other => Err(class_cast(&other, class)),
    }
}

/// Register the classes, bound to `sm` and `chars`.
pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>, chars: Rc<RefCell<Characters>>) {
    register_character_members(thread, &sm, &chars);
    register_nutrition(thread, &sm, &chars);
    register_thermoregulator(thread, &sm, &chars);
    register_fitness(thread, &sm, &chars);
}

/// `register` on the shared ScriptManager and Characters.
pub fn register_shared(thread: &mut Thread) {
    register(thread, shared_script_manager(), shared_characters());
}

fn register_character_members(
    thread: &mut Thread,
    sm: &Rc<ScriptManager>,
    _chars: &Rc<RefCell<Characters>>,
) {
    let b = expose_class(thread, ISO_GAME_CHARACTER_CLASS, None);
    let s = sm.clone();
    b.method("setMetabolicTarget", move |c| {
        let chr = chr_of(c)?;
        let target = match c.arg(1) {
            Value::Object(_) => match enum_arg(c, 1, METABOLICS_CLASS)? {
                Some((_, name)) => Some(
                    s.builtin_tables
                        .borrow()
                        .metabolics
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, m)| *m)
                        .ok_or_else(npe)?,
                ),
                None => None,
            },
            Value::Nil => None,
            _ => Some(float_arg(c, 1)),
        };
        if let Some(t) = target {
            chr.borrow_mut()
                .set_metabolic_target(t)
                .map_err(java_error)?;
        }
        Ok(0)
    });
    b.method("getThirstMultiplier", |c| {
        let v = chr_of(c)?.borrow().thirst_multiplier();
        Ok(c.push(v))
    });
    b.method("getHungerMultiplier", |c| {
        let v = chr_of(c)?.borrow().hunger_multiplier();
        Ok(c.push(v))
    });
    b.method("getFatiqueMultiplier", |c| {
        let v = chr_of(c)?.borrow().fatique_multiplier();
        Ok(c.push(v))
    });
    b.method("getTimedActionTimeModifier", |c| {
        let v = chr_of(c)?.borrow().timed_action_time_modifier();
        Ok(c.push(float_value(v)))
    });
    let p = expose_class(thread, ISO_PLAYER_CLASS, Some(ISO_GAME_CHARACTER_CLASS));
    p.method("getNutrition", |c| {
        let chr = chr_of(c)?;
        Ok(c.push(nutrition_value(&chr)))
    });
    p.method("getFitness", |c| {
        let chr = chr_of(c)?;
        Ok(c.push(fitness_value(&chr)))
    });
}

fn nutrition_chr(c: &NativeCall<'_>) -> LuaResult<LuaGameCharacterRef> {
    with_this::<NutritionObject, _>(c, |o| o.0.clone())
}

fn with_nutrition<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut pz_characters::character::Nutrition) -> R,
) -> LuaResult<R> {
    let chr = nutrition_chr(c)?;
    let mut chr = chr.borrow_mut();
    Ok(f(chr.nutrition.as_mut().ok_or_else(npe)?))
}

fn register_nutrition(
    thread: &mut Thread,
    sm: &Rc<ScriptManager>,
    chars: &Rc<RefCell<Characters>>,
) {
    use pz_characters::character::Nutrition;
    let b = expose_class(thread, NUTRITION_CLASS, None);
    type FloatGet = fn(&Nutrition) -> f32;
    let floats: [(&str, FloatGet); 4] = [
        ("getCalories", Nutrition::calories),
        ("getCarbohydrates", Nutrition::carbohydrates),
        ("getProteins", Nutrition::proteins),
        ("getLipids", Nutrition::lipids),
    ];
    for (name, f) in floats {
        b.method(name, move |c| {
            let v = with_nutrition(c, |n| f(n))?;
            Ok(c.push(float_value(v)))
        });
    }
    type FloatSet = fn(&mut Nutrition, f32);
    let setters: [(&str, FloatSet); 4] = [
        ("setCalories", Nutrition::set_calories),
        ("setCarbohydrates", Nutrition::set_carbohydrates),
        ("setProteins", Nutrition::set_proteins),
        ("setLipids", Nutrition::set_lipids),
    ];
    for (name, f) in setters {
        b.method(name, move |c| {
            let v = float_arg(c, 1);
            with_nutrition(c, |n| f(n, v))?;
            Ok(0)
        });
    }
    b.method("getWeight", |c| {
        let v = with_nutrition(c, |n| n.weight())?;
        Ok(c.push(v))
    });
    b.method("setWeight", |c| {
        let v = num_arg(c, 1);
        with_nutrition(c, |n| n.set_weight(v))?;
        Ok(0)
    });
    type BoolGet = fn(&Nutrition) -> bool;
    let bools: [(&str, BoolGet); 3] = [
        ("isIncWeight", Nutrition::is_inc_weight),
        ("isIncWeightLot", Nutrition::is_inc_weight_lot),
        ("isDecWeight", Nutrition::is_dec_weight),
    ];
    for (name, f) in bools {
        b.method(name, move |c| {
            let v = with_nutrition(c, |n| f(n))?;
            Ok(c.push(v))
        });
    }
    type BoolSet = fn(&mut Nutrition, bool);
    let bool_setters: [(&str, BoolSet); 3] = [
        ("setIncWeight", Nutrition::set_inc_weight),
        ("setIncWeightLot", Nutrition::set_inc_weight_lot),
        ("setDecWeight", Nutrition::set_dec_weight),
    ];
    for (name, f) in bool_setters {
        b.method(name, move |c| {
            let v = bool_arg(c, 1);
            with_nutrition(c, |n| f(n, v))?;
            Ok(0)
        });
    }
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("update", move |c| {
        let chr = nutrition_chr(c)?;
        let v = character_value(&chr);
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| {
            chr.borrow_mut().nutrition_update(&s, &chars, ctx)
        })?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("applyWeightFromTraits", move |c| {
        let chr = nutrition_chr(c)?;
        let mut chr = chr.borrow_mut();
        let mut n = chr.nutrition.take().ok_or_else(npe)?;
        let r = n.apply_weight_from_traits(&|f| chr.has_trait(&s, f));
        chr.nutrition = Some(n);
        r.map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("applyTraitFromWeight", move |c| {
        nutrition_chr(c)?
            .borrow_mut()
            .apply_trait_from_weight(&s)
            .map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("characterHaveWeightTrouble", move |c| {
        let chr = nutrition_chr(c)?;
        let chr = chr.borrow();
        let v = Nutrition::character_have_weight_trouble(&|f| chr.has_trait(&s, f))
            .map_err(java_error)?;
        Ok(c.push(v))
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("canAddFitnessXp", move |c| {
        let chr = nutrition_chr(c)?;
        let chr = chr.borrow();
        let level = chr.perk_level(ch.borrow().perks.constant("Fitness").as_ref());
        let n = chr.nutrition.as_ref().ok_or_else(npe)?;
        let v = n
            .can_add_fitness_xp(level, &|f| chr.has_trait(&s, f))
            .map_err(java_error)?;
        Ok(c.push(v))
    });
}

fn thermo_chr(c: &NativeCall<'_>) -> LuaResult<LuaGameCharacterRef> {
    with_this::<ThermoregulatorObject, _>(c, |o| o.0.clone())
}

fn with_thermo<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut Thermoregulator) -> R) -> LuaResult<R> {
    let chr = thermo_chr(c)?;
    let mut chr = chr.borrow_mut();
    Ok(f(chr.body_damage.thermoregulator_mut().ok_or_else(npe)?))
}

fn register_thermoregulator(
    thread: &mut Thread,
    sm: &Rc<ScriptManager>,
    chars: &Rc<RefCell<Characters>>,
) {
    let b = expose_class(thread, THERMOREGULATOR_CLASS, None);
    b.field(
        "THERMAL_COLD_DAMAGE_MOD",
        float_value(Thermoregulator::THERMAL_COLD_DAMAGE_MOD),
    );
    b.static_fn("setSimulationMultiplier", |c| {
        Thermoregulator::set_simulation_multiplier(float_arg(c, 0));
        Ok(0)
    });
    type Const = fn() -> f32;
    let consts: [(&str, Const); 3] = [
        ("getSkinCelciusMin", || Thermoregulator::SKIN_CELCIUS_MIN),
        ("getSkinCelciusFavorable", || {
            Thermoregulator::SKIN_CELCIUS_FAVORABLE
        }),
        ("getSkinCelciusMax", || Thermoregulator::SKIN_CELCIUS_MAX),
    ];
    for (name, f) in consts {
        b.static_fn(name, move |c| Ok(c.push(float_value(f()))));
    }
    type FloatGet = fn(&Thermoregulator) -> f32;
    let floats: [(&str, FloatGet); 17] = [
        ("getBodyHeatDelta", Thermoregulator::body_heat_delta),
        ("getHeatGeneration", Thermoregulator::heat_generation),
        ("getMetabolicRate", Thermoregulator::metabolic_rate),
        ("getMetabolicTarget", Thermoregulator::metabolic_target),
        ("getMetabolicRateReal", Thermoregulator::metabolic_rate_real),
        ("getSetPoint", Thermoregulator::set_point),
        ("getCoreHeatDelta", Thermoregulator::core_heat_delta),
        ("getCoreRateOfChange", Thermoregulator::core_rate_of_change),
        (
            "getExternalAirTemperature",
            Thermoregulator::external_air_temperature,
        ),
        (
            "getTimedActionTimeModifier",
            Thermoregulator::timed_action_time_modifier,
        ),
        ("getThermalDamage", Thermoregulator::thermal_damage),
        ("getDbg_totalHeatRaw", Thermoregulator::dbg_total_heat_raw),
        ("getDbg_totalHeat", Thermoregulator::dbg_total_heat),
        ("getCoreCelcius", Thermoregulator::core_celcius),
        ("getDbg_primTotal", Thermoregulator::dbg_prim_total),
        ("getDbg_secTotal", Thermoregulator::dbg_sec_total),
        ("getSimulationMultiplier", |_| {
            Thermoregulator::simulation_multiplier()
        }),
    ];
    for (name, f) in floats {
        b.method(name, move |c| {
            let v = with_thermo(c, |t| f(t))?;
            Ok(c.push(float_value(v)))
        });
    }
    type Fallible = fn(&Thermoregulator) -> Result<f32, ScriptError>;
    let fallible: [(&str, Fallible); 3] = [
        ("getCoreTemperature", Thermoregulator::core_temperature),
        ("getCoreTemperatureUI", Thermoregulator::core_temperature_ui),
        ("getHeatGenerationUI", Thermoregulator::heat_generation_ui),
    ];
    for (name, f) in fallible {
        b.method(name, move |c| {
            let v = with_thermo(c, |t| f(t))?.map_err(java_error)?;
            Ok(c.push(float_value(v)))
        });
    }
    type DoubleGet = fn(&Thermoregulator) -> f64;
    let doubles: [(&str, DoubleGet); 3] = [
        ("getFluidsMultiplier", Thermoregulator::fluids_multiplier),
        ("getEnergyMultiplier", Thermoregulator::energy_multiplier),
        ("getFatigueMultiplier", Thermoregulator::fatigue_multiplier),
    ];
    for (name, f) in doubles {
        b.method(name, move |c| {
            let v = with_thermo(c, |t| f(t))?;
            Ok(c.push(v))
        });
    }
    // `Multiplier`-specific simulation multipliers.
    use pz_characters::character::thermoregulator::Multiplier as M;
    let mults: [(&str, M); 7] = [
        ("getDefaultMultiplier", M::Default),
        ("getMetabolicRateIncMultiplier", M::MetabolicRateInc),
        ("getMetabolicRateDecMultiplier", M::MetabolicRateDec),
        ("getBodyHeatMultiplier", M::BodyHeat),
        ("getCoreHeatExpandMultiplier", M::CoreHeatExpand),
        ("getCoreHeatContractMultiplier", M::CoreHeatContract),
        ("getSkinCelciusMultiplier", M::SkinCelcius),
    ];
    for (name, m) in mults {
        b.method(name, move |c| {
            let chr = thermo_chr(c)?;
            let v = character_value(&chr);
            let r = with_ctx(c, &v, |ctx| {
                let chr = chr.borrow();
                let t = chr
                    .body_damage
                    .thermoregulator()
                    .ok_or_else(|| ScriptError::new("java.lang.NullPointerException"))?;
                Ok(t.simulation_multiplier_for(m, ctx))
            })?;
            Ok(c.push(float_value(r)))
        });
    }
    b.method("getTemperatureAir", |c| {
        let v = thermo_chr(c)?.borrow().environment.air_temperature;
        Ok(c.push(float_value(v)))
    });
    b.method("getTemperatureAirAndWind", |c| {
        let v = thermo_chr(c)?.borrow().environment.air_and_wind_temperature;
        Ok(c.push(float_value(v)))
    });
    b.method("thermalChevronUp", |c| {
        let v = with_thermo(c, |t| t.thermal_chevron_up())?;
        Ok(c.push(v))
    });
    b.method("thermalChevronCount", |c| {
        let v = with_thermo(c, |t| t.thermal_chevron_count())?;
        Ok(c.push(v))
    });
    let s = sm.clone();
    let levels = move |chr: &LuaGameCharacterRef| -> LuaResult<(i32, i32)> {
        let chr = chr.borrow();
        let hypo = chr
            .moodles
            .moodle_level_of(&s, "HYPOTHERMIA")
            .map_err(java_error)?;
        let hyper = chr
            .moodles
            .moodle_level_of(&s, "HYPERTHERMIA")
            .map_err(java_error)?;
        Ok((hypo, hyper))
    };
    let lv = levels.clone();
    b.method("getMovementModifier", move |c| {
        let (hypo, hyper) = lv(&thermo_chr(c)?)?;
        Ok(c.push(float_value(Thermoregulator::movement_modifier(hypo, hyper))))
    });
    let lv = levels.clone();
    b.method("getCombatModifier", move |c| {
        let (hypo, hyper) = lv(&thermo_chr(c)?)?;
        Ok(c.push(float_value(Thermoregulator::combat_modifier(hypo, hyper))))
    });
    let lv = levels;
    b.method("getCatchAColdDelta", move |c| {
        let (hypo, _) = lv(&thermo_chr(c)?)?;
        let v = with_thermo(c, |t| t.catch_a_cold_delta(hypo))?;
        Ok(c.push(float_value(v)))
    });
    let ch = chars.clone();
    b.method("getEnergy", move |c| {
        let chr = thermo_chr(c)?;
        let v = Thermoregulator::energy(&chr.borrow().stats, &ch.borrow().sim.character_stats)
            .map_err(java_error)?;
        Ok(c.push(float_value(v)))
    });
    let ch = chars.clone();
    b.method("getBodyFluids", move |c| {
        let chr = thermo_chr(c)?;
        let v = Thermoregulator::body_fluids(&chr.borrow().stats, &ch.borrow().sim.character_stats)
            .map_err(java_error)?;
        Ok(c.push(float_value(v)))
    });
    let s = sm.clone();
    b.method("setMetabolicTarget", move |c| {
        let target = match c.arg(1) {
            Value::Object(_) => {
                let (_, name) = enum_arg(c, 1, METABOLICS_CLASS)?.ok_or_else(npe)?;
                s.builtin_tables
                    .borrow()
                    .metabolics
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, m)| *m)
                    .ok_or_else(npe)?
            }
            Value::Nil => return Err(npe()),
            _ => float_arg(c, 1),
        };
        with_thermo(c, |t| t.set_metabolic_target(target))?.map_err(java_error)?;
        Ok(0)
    });
    b.method("reset", |c| {
        with_thermo(c, |t| t.reset())?.map_err(java_error)?;
        Ok(0)
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("update", move |c| {
        let chr = thermo_chr(c)?;
        let v = character_value(&chr);
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| {
            chr.borrow_mut().thermoregulator_update(&s, &chars, ctx)
        })?;
        Ok(0)
    });
    b.method("getNodeSize", |c| {
        let v = with_thermo(c, |t| t.node_size())?;
        Ok(c.push(v as f64))
    });
    b.method("getNode", |c| {
        let chr = thermo_chr(c)?;
        let i = int_arg(c, 1);
        let n = with_thermo(c, |t| t.node_size())?;
        let i = usize::try_from(i)
            .ok()
            .filter(|i| *i < n)
            .ok_or_else(|| LuaError::Runtime {
                message: format!("Index {i} out of bounds for length {n}"),
                class: "java.lang.ArrayIndexOutOfBoundsException",
            })?;
        Ok(c.push(thermal_node_value(&chr, i)))
    });
    b.method("getNodeForType", |c| {
        let chr = thermo_chr(c)?;
        let ty = enum_arg(c, 1, BODY_PART_TYPE_CLASS)?;
        let i = ty.and_then(|(o, _)| with_thermo(c, |t| t.node_index_for_type(o)).ok().flatten());
        Ok(c.push(i.map_or(Value::Nil, |i| thermal_node_value(&chr, i))))
    });
    b.method("getNodeForBloodType", |c| {
        let chr = thermo_chr(c)?;
        let i = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .map(|e| e.ordinal as usize),
            _ => None,
        };
        let i = i.and_then(|o| {
            with_thermo(c, |t| t.node_index_for_blood_type(o))
                .ok()
                .flatten()
        });
        Ok(c.push(i.map_or(Value::Nil, |i| thermal_node_value(&chr, i))))
    });
    register_thermal_node(thread, sm);
}

fn register_thermal_node(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    use pz_characters::character::ThermalNode;
    let b = expose_class(thread, THERMAL_NODE_CLASS, None);
    fn with_node<R>(c: &NativeCall<'_>, f: impl FnOnce(&ThermalNode) -> R) -> LuaResult<R> {
        let (chr, i) = with_this::<ThermalNodeObject, _>(c, |o| (o.chr.clone(), o.index))?;
        let chr = chr.borrow();
        let t = chr.body_damage.thermoregulator().ok_or_else(npe)?;
        Ok(f(t.node(i).ok_or_else(npe)?))
    }
    type FloatGet = fn(&ThermalNode) -> f32;
    let floats: [(&str, FloatGet); 21] = [
        ("getDistToCore", ThermalNode::dist_to_core),
        ("getSkinSurface", ThermalNode::skin_surface),
        ("getInsulation", ThermalNode::insulation),
        ("getWindresist", ThermalNode::windresist),
        ("getCelcius", ThermalNode::celcius),
        ("getSkinCelcius", ThermalNode::skin_celcius),
        ("getHeatDelta", ThermalNode::heat_delta),
        ("getPrimaryDelta", ThermalNode::primary_delta),
        ("getSecondaryDelta", ThermalNode::secondary_delta),
        ("getClothingWetness", ThermalNode::clothing_wetness),
        ("getBodyWetness", ThermalNode::body_wetness),
        ("getBodyResponse", ThermalNode::body_response),
        ("getSkinCelciusUI", ThermalNode::skin_celcius_ui),
        ("getHeatDeltaUI", ThermalNode::heat_delta_ui),
        ("getPrimaryDeltaUI", ThermalNode::primary_delta_ui),
        ("getSecondaryDeltaUI", ThermalNode::secondary_delta_ui),
        ("getInsulationUI", ThermalNode::insulation_ui),
        ("getWindresistUI", ThermalNode::windresist_ui),
        ("getClothingWetnessUI", ThermalNode::clothing_wetness_ui),
        ("getBodyWetnessUI", ThermalNode::body_wetness_ui),
        ("getBodyResponseUI", ThermalNode::body_response_ui),
    ];
    for (name, f) in floats {
        b.method(name, move |c| {
            let v = with_node(c, f)?;
            Ok(c.push(float_value(v)))
        });
    }
    type BoolGet = fn(&ThermalNode) -> bool;
    let bools: [(&str, BoolGet); 3] = [
        ("hasUpstream", ThermalNode::has_upstream),
        ("hasDownstream", ThermalNode::has_downstream),
        ("isCore", ThermalNode::is_core),
    ];
    for (name, f) in bools {
        b.method(name, move |c| {
            let v = with_node(c, f)?;
            Ok(c.push(v))
        });
    }
    let s = sm.clone();
    b.method("getName", move |c| {
        let i = with_node(c, |n| n.body_part_index())?;
        // `BodyPartType.getDisplayName`: `Translator.getText(displayNameKey)`.
        let key = s
            .builtin_tables
            .borrow()
            .body_parts
            .get(i)
            .and_then(|p| p.display_name_key.clone());
        Ok(c.push(
            key.as_deref()
                .map(Characters::translate)
                .map(Value::from)
                .unwrap_or_default(),
        ))
    });
}

fn fitness_chr(c: &NativeCall<'_>) -> LuaResult<LuaGameCharacterRef> {
    with_this::<FitnessObject, _>(c, |o| o.0.clone())
}

fn with_fitness<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut pz_characters::character::Fitness) -> R,
) -> LuaResult<R> {
    let chr = fitness_chr(c)?;
    let mut chr = chr.borrow_mut();
    Ok(f(chr.fitness.as_mut().ok_or_else(npe)?))
}

fn table_of(v: Value) -> LuaResult<Option<pz_kahlua::TableRef>> {
    match v {
        Value::Nil => Ok(None),
        Value::Table(t) => Ok(Some(t)),
        other => Err(class_cast(&other, "se.krka.kahlua.j2se.KahluaTableImpl")),
    }
}

/// `new FitnessExercise(table)`.
fn fitness_exercise(v: &Value) -> LuaResult<FitnessExercise> {
    let t = table_of(v.clone())?.ok_or_else(npe)?;
    let get = |k: &str| t.rawget(&Value::str(k));
    let str_of = |v: Value| -> LuaResult<Option<String>> {
        match v {
            Value::Nil => Ok(None),
            Value::Str(s) => Ok(Some(s.to_string())),
            other => Err(class_cast(&other, "java.lang.String")),
        }
    };
    let ty = str_of(get("type"))?;
    let metabolics = match get("metabolics") {
        Value::Nil => None,
        Value::Object(o) => match o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == METABOLICS_CLASS)
        {
            Some(e) => Some(e.name.to_string()),
            None => return Err(class_cast(&Value::Object(o), METABOLICS_CLASS)),
        },
        other => return Err(class_cast(&other, METABOLICS_CLASS)),
    };
    let stiffness = str_of(get("stiffness"))?.ok_or_else(npe)?;
    let xp_mod = match get("xpMod") {
        Value::Num(n) => n as f32,
        _ => -1.0,
    };
    Ok(FitnessExercise::new(ty, metabolics, &stiffness, xp_mod))
}

fn register_fitness(thread: &mut Thread, sm: &Rc<ScriptManager>, chars: &Rc<RefCell<Characters>>) {
    expose_class(thread, FITNESS_EXERCISE_CLASS, None);
    let b = expose_class(thread, FITNESS_CLASS, None);
    let s = sm.clone();
    b.method("init", move |c| {
        let chr = fitness_chr(c)?;
        if !chr
            .borrow()
            .fitness
            .as_ref()
            .ok_or_else(npe)?
            .exercises()
            .is_empty()
        {
            return Ok(0);
        }
        let globals = c.thread.globals().clone();
        let Some(ex) = table_of(globals.rawget(&Value::str("FitnessExercises")))? else {
            return Ok(0);
        };
        let Some(list) = table_of(ex.rawget(&Value::str("exercisesType")))? else {
            return Ok(0);
        };
        let mut entries = Vec::new();
        for (k, v) in list.entries() {
            let made = match &k {
                Value::Str(key) => fitness_exercise(&v).map(|e| (key.to_string(), e)),
                other => Err(class_cast(other, "java.lang.String")),
            };
            match made {
                Ok(e) => entries.push(e),
                Err(e) => {
                    // The exercises made before the failure stay.
                    let mut chr = chr.borrow_mut();
                    let f = chr.fitness.as_mut().ok_or_else(npe)?;
                    for (k, x) in entries {
                        f.put_exercise(k, x);
                    }
                    return Err(e);
                }
            }
        }
        let v = character_value(&chr);
        with_ctx(c, &v, |ctx| {
            chr.borrow_mut().fitness_init(&s, Some(entries), ctx)
        })?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("initRegularityMapProfession", move |c| {
        let chr = fitness_chr(c)?;
        let v = character_value(&chr);
        with_ctx(c, &v, |ctx| {
            chr.borrow_mut().init_regularity_map_profession(&s, ctx)
        })?;
        Ok(0)
    });
    b.method("setCurrentExercise", |c| {
        let ty = str_arg(c, 1);
        with_fitness(c, |f| f.set_current_exercise(ty.as_deref()))?;
        Ok(0)
    });
    b.method("getCurrentExe", |c| {
        let e = with_fitness(c, |f| f.current_exe().cloned())?;
        Ok(c.push(e.map_or(Value::Nil, |e| {
            Value::Object(Rc::new(FitnessExerciseObject(e)))
        })))
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("exerciseRepeat", move |c| {
        let chr = fitness_chr(c)?;
        let v = character_value(&chr);
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| {
            chr.borrow_mut().fitness_exercise_repeat(&s, &chars, ctx)
        })?;
        Ok(0)
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("reduceEndurance", move |c| {
        let chr = fitness_chr(c)?;
        let v = character_value(&chr);
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| {
            chr.borrow_mut().fitness_reduce_endurance(&s, &chars, ctx)
        })?;
        Ok(0)
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("incStats", move |c| {
        let chr = fitness_chr(c)?;
        let v = character_value(&chr);
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| {
            chr.borrow_mut().fitness_inc_stats(&s, &chars, ctx)
        })?;
        Ok(0)
    });
    b.method("incRegularity", |c| {
        with_fitness(c, |f| f.inc_regularity())?.map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("incFutureStiffness", move |c| {
        let chr = fitness_chr(c)?;
        let mut chr = chr.borrow_mut();
        let tired = chr
            .moodles
            .moodle_level_of(&s, "TIRED")
            .map_err(java_error)?;
        chr.fitness
            .as_mut()
            .ok_or_else(npe)?
            .inc_future_stiffness(tired)
            .map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("update", move |c| {
        let chr = fitness_chr(c)?;
        let v = character_value(&chr);
        with_ctx(c, &v, |ctx| chr.borrow_mut().fitness_update(&s, ctx))?;
        Ok(0)
    });
    b.method("resetValues", |c| {
        with_fitness(c, |f| f.reset_values())?;
        Ok(0)
    });
    b.method("removeStiffnessValue", |c| {
        let ty = str_arg(c, 1);
        with_fitness(c, |f| {
            if let Some(t) = ty {
                f.remove_stiffness_value(&t)
            }
        })?;
        Ok(0)
    });
    b.method("onGoingStiffness", |c| {
        let v = with_fitness(c, |f| f.on_going_stiffness())?;
        Ok(c.push(v))
    });
    b.method("getCurrentExeStiffnessTimer", |c| {
        let ty = str_arg(c, 1).ok_or_else(npe)?;
        let v = with_fitness(c, |f| f.current_exe_stiffness_timer(&ty))?.map_err(java_error)?;
        Ok(c.push(v))
    });
    b.method("getCurrentExeStiffnessInc", |c| {
        let ty = str_arg(c, 1).ok_or_else(npe)?;
        let v = with_fitness(c, |f| f.current_exe_stiffness_inc(&ty))?.map_err(java_error)?;
        Ok(c.push(float_value(v)))
    });
    b.method("getRegularity", |c| {
        let ty = str_arg(c, 1);
        let v = with_fitness(c, |f| ty.map_or(0.0, |t| f.regularity(&t)))?;
        Ok(c.push(float_value(v)))
    });
    b.method("getParent", |c| {
        let chr = fitness_chr(c)?;
        Ok(c.push(character_value(&chr)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_character_sim::LuaGameCharacter;
    use pz_characters::{CharacterKind, SurvivorDesc};
    use pz_kahlua::TableRef;
    use pz_scripts::registry::builtin::{
        BodyPartDecl, BuiltinTables, CharacterStatDecl, PerkDecl, PerkInit,
    };

    fn run(t: &mut Thread, src: &str) -> Result<(), String> {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(p),
            t.globals().clone(),
        ));
        t.pcall(&f, &[])
            .map(|_| ())
            .map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e)))
    }

    /// Invented ids, stats, body parts, metabolics and perks.
    fn setup() -> (Thread, LuaGameCharacterRef, Rc<ScriptManager>) {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end")
            .unwrap();
        crate::bind_java_util::register(&mut t);
        let sm = Rc::new(ScriptManager::new());
        {
            let mut st = sm.statics.items.borrow_mut();
            let mut keys = vec![];
            for (i, f) in [
                "EMACIATED",
                "VERY_UNDERWEIGHT",
                "UNDERWEIGHT",
                "OVERWEIGHT",
                "OBESE",
                "WEIGHT_GAIN",
                "FAST_LEARNER",
                "SLOW_LEARNER",
                "PACIFIST",
                "CRAFTY",
            ]
            .iter()
            .enumerate()
            {
                keys.push(("character_trait", *f, format!("t:w{i}")));
            }
            for (i, f) in [
                "TIRED",
                "HEAVY_LOAD",
                "HYPOTHERMIA",
                "HYPERTHERMIA",
                "ENDURANCE",
            ]
            .iter()
            .enumerate()
            {
                keys.push(("moodle_type", *f, format!("t:m{i}")));
            }
            for (i, f) in [
                "UNEMPLOYED",
                "FIRE_OFFICER",
                "FITNESS_INSTRUCTOR",
                "SECURITY_GUARD",
            ]
            .iter()
            .enumerate()
            {
                keys.push(("character_profession", *f, format!("t:p{i}")));
            }
            for (reg, field, id) in keys {
                sm.registries
                    .get(reg)
                    .unwrap()
                    .borrow_mut()
                    .register_id(&id, false)
                    .unwrap();
                st.constants.insert(reg, field, &id);
            }
        }
        let mut tables = BuiltinTables::default();
        for f in [
            "ENDURANCE",
            "FITNESS",
            "HUNGER",
            "FATIGUE",
            "THIRST",
            "SICKNESS",
            "TEMPERATURE",
        ] {
            let (min, max, def) = match f {
                "ENDURANCE" => (0.0, 1.0, 1.0),
                "FITNESS" => (-1.0, 1.0, 0.0),
                "TEMPERATURE" => (20.0, 40.0, 37.0),
                _ => (0.0, 1.0, 0.0),
            };
            tables.character_stats.push(CharacterStatDecl {
                field: f.into(),
                id: f.to_lowercase(),
                minimum: min,
                maximum: max,
                default: def,
            });
        }
        tables.body_parts = ["Hand_L", "Torso_Upper", "Head", "MAX"]
            .iter()
            .map(|f| BodyPartDecl {
                field: f.to_string(),
                name: f.to_lowercase(),
                skin_surface: 0.25,
                ..Default::default()
            })
            .collect();
        tables.metabolics = [
            ("Default", 1.5),
            ("DefaultExercise", 2.5),
            ("Fitness", 4.5),
            ("FitnessHeavy", 6.0),
            ("MAX", 9.5),
        ]
        .iter()
        .map(|(n, m)| (n.to_string(), *m))
        .collect();
        tables.level_up_levels = vec![5, 10];
        let decl = |f: &str| PerkDecl {
            field: f.into(),
            id: f.into(),
            parent: None,
        };
        tables.perks = ["None", "Fitness", "Strength", "MAX"].map(decl).to_vec();
        tables.perk_ids = tables.perks.iter().map(|p| p.id.clone()).collect();
        let xp = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        tables.perk_init = vec![PerkInit::SetParent {
            perk: "None".into(),
            parent: "None".into(),
        }];
        for p in ["Fitness", "Strength"] {
            tables.perk_init.push(PerkInit::AddPerk {
                perk: p.into(),
                translation: p.into(),
                parent: None,
                xp,
                passiv: true,
            });
        }
        *sm.builtin_tables.borrow_mut() = tables.clone();
        let mut chars = Characters {
            perks: pz_characters::PerkFactory::from_tables(&tables).unwrap(),
            ..Default::default()
        };
        chars.perks.init(&tables, &|k| k.to_string()).unwrap();
        chars.sim = pz_characters::SimStatics::from_tables(&tables).unwrap();
        let desc =
            SurvivorDesc::<crate::bind_characters::LuaItem>::new_unregistered(&mut chars, &sm)
                .unwrap();
        let chars = Rc::new(RefCell::new(chars));
        crate::bind_registry::register(&mut t, sm.clone());
        crate::bind_body_damage::register(&mut t, &sm);
        crate::bind_character_sim::register(&mut t, sm.clone(), chars.clone());
        register(&mut t, sm.clone(), chars);
        let mut chr: LuaGameCharacter =
            pz_characters::GameCharacter::new(&sm, CharacterKind::Player);
        chr.in_players = true;
        chr.player_instance = true;
        chr.descriptor = Some(Rc::new(RefCell::new(desc)));
        let chr = Rc::new(RefCell::new(chr));
        t.globals().set_str("player", character_value(&chr));
        t.globals().set_str("thermo", thermoregulator_value(&chr));
        (t, chr, sm)
    }

    #[test]
    fn nutrition_from_lua() {
        let (mut t, chr, sm) = setup();
        run(
            &mut t,
            r#"
            local n = player:getNutrition()
            assert(n == player:getNutrition())
            assert(n:getCalories() == 800 and n:getWeight() == 80)
            n:setCalories(5000)
            assert(n:getCalories() == 3700)
            n:setLipids(-12.5)
            assert(n:getLipids() == -12.5)
            n:setWeight(101)
            n:applyTraitFromWeight()
            assert(not n:isIncWeight() and n:characterHaveWeightTrouble())
            assert(n:canAddFitnessXp())
            "#,
        )
        .unwrap();
        assert!(chr.borrow().has_trait(&sm, "OBESE").unwrap());
    }

    #[test]
    fn thermoregulator_from_lua() {
        let (mut t, chr, _) = setup();
        run(
            &mut t,
            r#"
            assert(thermo:getNodeSize() == 3)
            local head = thermo:getNodeForType(BodyPartType.Head)
            assert(head == thermo:getNode(2) and head:getSkinCelcius() == 33 and not head:isCore())
            assert(head:hasUpstream() == false)
            assert(thermo:getNodeForType(BodyPartType.Torso_Upper):isCore())
            assert(thermo:getCoreTemperature() == 37 and thermo:getMetabolicRate() == 1.5)
            assert(thermo:thermalChevronCount() == 0 and thermo:thermalChevronUp())
            player:setMetabolicTarget(Metabolics.Fitness)
            assert(thermo:getMetabolicTarget() == 4.5)
            thermo:setMetabolicTarget(99)
            assert(thermo:getMetabolicTarget() == 9.5)
            assert(Thermoregulator.getSkinCelciusMax() == 42)
            assert(thermo:getEnergy() == 1 and thermo:getBodyFluids() == 1)
            assert(not pcall(function() return thermo:getNode(3) end))
            "#,
        )
        .unwrap();
        assert_eq!(
            chr.borrow()
                .body_damage
                .thermoregulator()
                .unwrap()
                .metabolic_target(),
            9.5
        );
    }

    #[test]
    fn fitness_from_lua() {
        let (mut t, chr, _) = setup();
        run(
            &mut t,
            r#"
            FitnessExercises = { exercisesType = {
                hop = { type = "hop", stiffness = "legs,abs", metabolics = Metabolics.FitnessHeavy, xpMod = 2 },
            } }
            local f = player:getFitness()
            assert(f == player:getFitness() and f:getParent() == player)
            f:init()
            f:setCurrentExercise("hop")
            assert(f:getCurrentExe() ~= nil)
            f:exerciseRepeat()
            assert(f:getRegularity("hop") > 0)
            assert(f:getCurrentExeStiffnessTimer("legs,arms") == 72)
            assert(f:getCurrentExeStiffnessInc("abs") > 0)
            f:removeStiffnessValue("legs")
            assert(f:getCurrentExeStiffnessTimer("legs") == 0)
            f:setCurrentExercise(nil)
            assert(f:getCurrentExe() == nil)
            assert(not pcall(function() f:exerciseRepeat() end))
            "#,
        )
        .unwrap();
        let c = chr.borrow();
        let fit = c.fitness.as_ref().unwrap();
        assert_eq!(fit.exercises().get("hop").unwrap().xp_modifier, 2.0);
        assert_eq!(
            fit.exercises().get("hop").unwrap().metabolics.as_deref(),
            Some("FitnessHeavy")
        );
    }

    #[test]
    fn fitness_init_keeps_exercises_before_a_bad_entry() {
        let (mut t, chr, _) = setup();
        let r = run(
            &mut t,
            r#"
            FitnessExercises = { exercisesType = {} }
            FitnessExercises.exercisesType.hop = { type = "hop", stiffness = "legs" }
            FitnessExercises.exercisesType.bad = { type = "bad" }
            player:getFitness():init()
            "#,
        );
        assert!(r.unwrap_err().contains("NullPointerException"));
        let c = chr.borrow();
        let ex = c.fitness.as_ref().unwrap().exercises();
        assert!(ex.contains_key("hop") && !ex.contains_key("bad"));
    }
}
