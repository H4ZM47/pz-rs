//! `IsoGameCharacter:getBodyDamage()`, `BodyDamage` and `BodyPart` exposed
//! to Lua under the original names, over
//! [`pz_characters::character::body_damage`] and
//! [`pz_characters::character::body_part`]. `BodyPartType` constants are the
//! enum objects of [`crate::bind_body_damage`].
//!
//! Methods taking a `BodyPartType` or an `int` (the original's overloads)
//! accept either. Owner-dependent methods run with the installed
//! `SimContext` ([`crate::bind_character_sim::install_sim_context`]);
//! `OnPlayerGetDamage` reaches the Lua event sink after the call returns.
//!
//! Not exposed yet: the methods taking items (`JustAteFood`,
//! `JustDrankBooze`, `JustTookPill`, `JustReadSomething`,
//! `DamageFromWeapon`, `damageFromSpikedArmor`) are ported over
//! `pz-inventory` items, which have no Lua objects yet; the ones taking a
//! zombie or animal (`AddRandomDamageFromZombie`, `DamageFromAnimal`);
//! `load`/`save`, `getBodyPartsLastState`,
//! `BodyPart.sync`/`syncWrite`/`getThermalNode`.
//! TODO(InventoryItem Lua objects, IsoZombie, IsoAnimal).

use crate::bind_body_damage::{enum_arg, BODY_PART_TYPE_CLASS};
use crate::bind_character_sim::{character_value, chr_of, with_ctx, LuaGameCharacterRef, ISO_GAME_CHARACTER_CLASS};
use crate::bind_characters::shared_characters;
use crate::bind_java_util::JavaList;
use crate::bind_registry::{java_error, shared_script_manager};
use crate::expose::{bool_arg, enum_value, expose_class, int_arg, num_arg, str_arg, with_this};
use pz_characters::character::{BodyDamage, BodyPart, GameCharacter, InjuryEnv};
use pz_characters::Characters;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::{ScriptError, ScriptManager};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const BODY_DAMAGE_CLASS: &str = "zombie.characters.BodyDamage.BodyDamage";
pub const BODY_PART_CLASS: &str = "zombie.characters.BodyDamage.BodyPart";

/// A character's `BodyDamage`.
pub struct BodyDamageObject(pub LuaGameCharacterRef);

/// One `BodyPart` of a character.
pub struct BodyPartObject {
    pub chr: LuaGameCharacterRef,
    pub index: usize,
}

impl LuaObject for BodyDamageObject {
    fn class_name(&self) -> &str {
        BODY_DAMAGE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl LuaObject for BodyPartObject {
    fn class_name(&self) -> &str {
        BODY_PART_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static HANDLES: RefCell<HashMap<(bool, usize, usize), Value>> = RefCell::new(HashMap::new());
}

/// The canonical `BodyDamage` object of `chr`.
pub fn body_damage_value(chr: &LuaGameCharacterRef) -> Value {
    HANDLES.with(|h| {
        h.borrow_mut()
            .entry((false, Rc::as_ptr(chr) as usize, 0))
            .or_insert_with(|| Value::Object(Rc::new(BodyDamageObject(chr.clone()))))
            .clone()
    })
}

/// The canonical object of part `index` of `chr`.
pub fn body_part_value(chr: &LuaGameCharacterRef, index: usize) -> Value {
    HANDLES.with(|h| {
        h.borrow_mut()
            .entry((true, Rc::as_ptr(chr) as usize, index))
            .or_insert_with(|| Value::Object(Rc::new(BodyPartObject { chr: chr.clone(), index })))
            .clone()
    })
}

fn float_arg(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

fn fv(f: f32) -> Value {
    Value::Num(f64::from(f))
}

fn opt_str(s: &Option<String>) -> Value {
    s.as_deref().map(Value::str).unwrap_or_default()
}

fn bd_chr(c: &NativeCall<'_>) -> LuaResult<LuaGameCharacterRef> {
    with_this::<BodyDamageObject, _>(c, |o| o.0.clone())
}

fn bp_this(c: &NativeCall<'_>) -> LuaResult<(LuaGameCharacterRef, usize)> {
    with_this::<BodyPartObject, _>(c, |o| (o.chr.clone(), o.index))
}

fn index_error(i: i64, len: usize) -> LuaError {
    java_error(ScriptError::new(format!(
        "java.lang.IndexOutOfBoundsException: Index {i} out of bounds for length {len}"
    )))
}

/// A `BodyPartType` (via `ToIndex`, `null` is 0) or `int` argument, as the
/// index into `bodyParts`.
fn part_arg(c: &NativeCall<'_>, i: usize, chr: &LuaGameCharacterRef) -> LuaResult<usize> {
    let len = chr.borrow().body_damage.parts.len();
    let idx = match c.arg(i) {
        Value::Num(n) => n as i32 as i64,
        _ => enum_arg(c, i, BODY_PART_TYPE_CLASS)?.unwrap_or(0) as i64,
    };
    usize::try_from(idx).ok().filter(|i| *i < len).ok_or_else(|| index_error(idx, len))
}

/// Runs `f` on the character with the installed context, then fires its
/// Lua events.
fn run<R>(
    c: &mut NativeCall<'_>,
    chr: &LuaGameCharacterRef,
    f: impl FnOnce(
        &mut GameCharacter<crate::bind_characters::LuaItem>,
        &dyn pz_characters::character::SimContext,
    ) -> Result<R, ScriptError>,
) -> LuaResult<R> {
    let v = character_value(chr);
    with_ctx(c, &v, |ctx| f(&mut chr.borrow_mut(), ctx))
}

/// Runs a `BodyPart` mutation that needs the owner's [`InjuryEnv`].
fn with_env(
    c: &mut NativeCall<'_>,
    sm: &ScriptManager,
    f: impl FnOnce(&mut BodyPart, &InjuryEnv<'_>),
) -> LuaResult<usize> {
    let (chr, i) = bp_this(c)?;
    run(c, &chr, |ch, ctx| {
        let env = ch.injury_env(sm, ctx)?;
        f(&mut ch.body_damage.parts[i], &env);
        Ok(())
    })?;
    Ok(0)
}

/// Register the classes and `IsoGameCharacter:getBodyDamage()`.
pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>, chars: Rc<RefCell<Characters>>) {
    let b = expose_class(thread, ISO_GAME_CHARACTER_CLASS, None);
    b.method("getBodyDamage", |c| {
        let chr = chr_of(c)?;
        Ok(c.push(body_damage_value(&chr)))
    });
    register_character_members(thread);
    register_body_damage(thread, &sm, &chars);
    register_body_part(thread, &sm, &chars);
}

/// `register` on the shared ScriptManager and Characters.
pub fn register_shared(thread: &mut Thread) {
    register(thread, shared_script_manager(), shared_characters());
}

type ChrGetF = fn(&GameCharacter<crate::bind_characters::LuaItem>) -> f32;
type ChrSetF = fn(&mut GameCharacter<crate::bind_characters::LuaItem>, f32);

/// The `IsoGameCharacter` members `BodyDamage` reads and writes.
fn register_character_members(thread: &mut Thread) {
    let b = expose_class(thread, ISO_GAME_CHARACTER_CLASS, None);
    let gets: [(&str, ChrGetF); 8] = [
        ("getBetaEffect", |c| c.beta_effect),
        ("getBetaDelta", |c| c.beta_delta),
        ("getPainEffect", |c| c.pain_effect),
        ("getPainDelta", |c| c.pain_delta),
        ("getSleepingTabletEffect", |c| c.sleeping_tablet_effect),
        ("getSleepingTabletDelta", |c| c.sleeping_tablet_delta),
        ("getReduceInfectionPower", |c| c.reduce_infection_power),
        ("getTimeSinceLastSmoke", |c| c.time_since_last_smoke),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let v = f(&chr_of(c)?.borrow());
            Ok(c.push(fv(v)))
        });
    }
    let sets: [(&str, ChrSetF); 12] = [
        ("setBetaEffect", |c, v| c.beta_effect = v),
        ("setBetaDelta", |c, v| c.beta_delta = v),
        ("setPainEffect", |c, v| c.pain_effect = v),
        ("setPainDelta", |c, v| c.pain_delta = v),
        ("setSleepingTabletEffect", |c, v| c.sleeping_tablet_effect = v),
        ("setSleepingTabletDelta", |c, v| c.sleeping_tablet_delta = v),
        ("setReduceInfectionPower", |c, v| c.reduce_infection_power = v),
        ("setTimeSinceLastSmoke", |c, v| c.time_since_last_smoke = v),
        ("SleepingTablet", |c, v| c.sleeping_tablet(v)),
        ("BetaBlockers", |c, v| c.beta_blockers(v)),
        ("BetaAntiDepress", |c, v| c.beta_anti_depress(v)),
        ("PainMeds", |c, v| c.pain_meds(v)),
    ];
    for (name, f) in sets {
        b.method(name, move |c| {
            let v = float_arg(c, 1);
            f(&mut chr_of(c)?.borrow_mut(), v);
            Ok(0)
        });
    }
    b.method("isReading", |c| {
        let v = chr_of(c)?.borrow().is_reading;
        Ok(c.push(v))
    });
    b.method("setReading", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().is_reading = v;
        Ok(0)
    });
    b.method("getHealth", |c| {
        let v = chr_of(c)?.borrow().health;
        Ok(c.push(fv(v)))
    });
    b.method("setHealth", |c| {
        let chr = chr_of(c)?;
        let v = float_arg(c, 1);
        run(c, &chr, |ch, ctx| {
            ch.set_health(ctx, v);
            Ok(())
        })?;
        Ok(0)
    });
}

type BdGet = fn(&BodyDamage) -> Value;
type BdSet = fn(&mut BodyDamage, &NativeCall<'_>);

fn register_body_damage(thread: &mut Thread, sm: &Rc<ScriptManager>, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, BODY_DAMAGE_CLASS, None);
    b.field("InfectionLevelToZombify", fv(pz_characters::character::body_damage::INFECTION_LEVEL_TO_ZOMBIFY));
    let gets: Vec<(&str, BdGet)> = vec![
        ("getHealth", |b| fv(b.health())),
        ("getOverallBodyHealth", |b| fv(b.overall_body_health)),
        ("getPainReduction", |b| fv(b.pain_reduction)),
        ("getColdReduction", |b| fv(b.cold_reduction)),
        ("getCatchACold", |b| fv(b.catch_a_cold)),
        ("getColdStrength", |b| fv(b.cold_strength())),
        ("getColdDamageStage", |b| fv(b.cold_damage_stage)),
        ("getColdProgressionRate", |b| fv(b.cold_progression_rate)),
        ("getInfectionGrowthRate", |b| fv(b.infection_growth_rate)),
        ("getInfectionTime", |b| fv(b.infection_time)),
        ("getInfectionMortalityDuration", |b| fv(b.infection_mortality_duration)),
        ("getHealthFromFood", |b| fv(b.health_from_food)),
        ("getHealthFromFoodTimer", |b| fv(b.health_from_food_timer)),
        ("getHealthReductionFromSevereBadMoodles", |b| fv(b.health_reduction_from_severe_bad_moodles)),
        ("getStandardHealthAddition", |b| fv(b.standard_health_addition)),
        ("getReducedHealthAddition", |b| fv(b.reduced_health_addition)),
        ("getSeverlyReducedHealthAddition", |b| fv(b.severly_reduced_health_addition)),
        ("getSleepingHealthAddition", |b| fv(b.sleeping_health_addition)),
        ("getBoredomDecreaseFromReading", |b| fv(b.boredom_decrease_from_reading)),
        ("getInitialThumpPain", |b| fv(b.initial_thump_pain)),
        ("getInitialScratchPain", |b| fv(b.initial_scratch_pain)),
        ("getInitialBitePain", |b| fv(b.initial_bite_pain)),
        ("getInitialWoundPain", |b| fv(b.initial_wound_pain)),
        ("getContinualPainIncrease", |b| fv(b.continual_pain_increase)),
        ("getPainReductionFromMeds", |b| fv(b.pain_reduction_from_meds)),
        ("getStandardPainReductionWhenWell", |b| fv(b.standard_pain_reduction_when_well)),
        ("getPanicIncreaseValue", |b| fv(b.panic_increase_value)),
        ("getPanicIncreaseValueFrame", |b| fv(b.panic_increase_value_frame())),
        ("getPanicReductionValue", |b| fv(b.panic_reduction_value)),
        ("getDrunkIncreaseValue", |b| fv(b.drunk_increase_value)),
        ("getDrunkReductionValue", |b| fv(b.drunk_reduction_value)),
        ("getTimeToSneezeOrCough", |b| fv(b.time_to_sneeze_or_cough)),
        ("getNumPartsBitten", |b| b.num_parts_bitten().into()),
        ("getNumPartsBleeding", |b| b.num_parts_bleeding().into()),
        ("getNumPartsScratched", |b| b.num_parts_scratched().into()),
        ("getDamageModCount", |b| b.damage_mod_count.into()),
        ("getOldNumZombiesVisible", |b| b.old_num_zombies_visible.into()),
        ("getCurrentNumZombiesVisible", |b| b.current_num_zombies_visible.into()),
        ("getRemotePainLevel", |b| b.remote_pain_level.into()),
        ("getSneezeCoughActive", |b| b.sneeze_cough_active.into()),
        ("IsSneezingCoughing", |b| b.is_sneezing_coughing().into()),
        ("getSneezeCoughTime", |b| b.sneeze_cough_time.into()),
        ("getSneezeCoughDelay", |b| b.sneeze_cough_delay.into()),
        ("getMildColdSneezeTimerMin", |b| b.mild_cold_sneeze_timer_min.into()),
        ("getMildColdSneezeTimerMax", |b| b.mild_cold_sneeze_timer_max.into()),
        ("getColdSneezeTimerMin", |b| b.cold_sneeze_timer_min.into()),
        ("getColdSneezeTimerMax", |b| b.cold_sneeze_timer_max.into()),
        ("getNastyColdSneezeTimerMin", |b| b.nasty_cold_sneeze_timer_min.into()),
        ("getNastyColdSneezeTimerMax", |b| b.nasty_cold_sneeze_timer_max.into()),
        ("getSmokerSneezeTimerMin", |b| b.smoker_sneeze_timer_min().into()),
        ("getSmokerSneezeTimerMax", |b| b.smoker_sneeze_timer_max().into()),
        ("getStandardHealthFromFoodTime", |b| b.standard_health_from_food_time.into()),
        ("HasInjury", |b| b.has_injury().into()),
        ("IsOnFire", |b| b.is_on_fire.into()),
        ("isIsOnFire", |b| b.is_on_fire.into()),
        ("WasBurntToDeath", |b| b.burnt_to_death.into()),
        ("isBurntToDeath", |b| b.burnt_to_death.into()),
        ("isHasACold", |b| b.has_a_cold.into()),
        ("isInfected", |b| b.is_infected.into()),
        ("isInf", |b| b.is_infected.into()),
        ("isIsFakeInfected", |b| b.is_fake_infected.into()),
        ("isReduceFakeInfection", |b| b.reduce_fake_infection.into()),
        ("isNeckBleeding", |b| b.is_neck_bleeding().into()),
        ("getWasDraggingCorpse", |b| b.was_dragging_corpse.into()),
        ("UseBandageOnMostNeededPart", |_| Value::Nil),
    ];
    for (name, f) in gets.into_iter().filter(|(n, _)| *n != "UseBandageOnMostNeededPart") {
        b.method(name, move |c| {
            let v = f(&bd_chr(c)?.borrow().body_damage);
            Ok(c.push(v))
        });
    }
    let sets: Vec<(&str, BdSet)> = vec![
        ("setPainReduction", |b, c| b.pain_reduction = float_arg(c, 1)),
        ("setColdReduction", |b, c| b.cold_reduction = float_arg(c, 1)),
        ("setCatchACold", |b, c| b.catch_a_cold = float_arg(c, 1)),
        ("setColdStrength", |b, c| b.cold_strength = float_arg(c, 1)),
        ("setColdDamageStage", |b, c| b.cold_damage_stage = float_arg(c, 1)),
        ("setColdProgressionRate", |b, c| b.cold_progression_rate = float_arg(c, 1)),
        ("setInfectionGrowthRate", |b, c| b.infection_growth_rate = float_arg(c, 1)),
        ("setInfectionTime", |b, c| b.infection_time = float_arg(c, 1)),
        ("setInfectionMortalityDuration", |b, c| b.infection_mortality_duration = float_arg(c, 1)),
        ("setHealthFromFood", |b, c| b.health_from_food = float_arg(c, 1)),
        ("setHealthFromFoodTimer", |b, c| b.health_from_food_timer = float_arg(c, 1)),
        ("setHealthReductionFromSevereBadMoodles", |b, c| b.health_reduction_from_severe_bad_moodles = float_arg(c, 1)),
        ("setStandardHealthAddition", |b, c| b.standard_health_addition = float_arg(c, 1)),
        ("setReducedHealthAddition", |b, c| b.reduced_health_addition = float_arg(c, 1)),
        ("setSeverlyReducedHealthAddition", |b, c| b.severly_reduced_health_addition = float_arg(c, 1)),
        ("setSleepingHealthAddition", |b, c| b.sleeping_health_addition = float_arg(c, 1)),
        ("setBoredomDecreaseFromReading", |b, c| b.boredom_decrease_from_reading = float_arg(c, 1)),
        ("setInitialThumpPain", |b, c| b.initial_thump_pain = float_arg(c, 1)),
        ("setInitialScratchPain", |b, c| b.initial_scratch_pain = float_arg(c, 1)),
        ("setInitialBitePain", |b, c| b.initial_bite_pain = float_arg(c, 1)),
        ("setInitialWoundPain", |b, c| b.initial_wound_pain = float_arg(c, 1)),
        ("setContinualPainIncrease", |b, c| b.continual_pain_increase = float_arg(c, 1)),
        ("setPainReductionFromMeds", |b, c| b.pain_reduction_from_meds = float_arg(c, 1)),
        ("setStandardPainReductionWhenWell", |b, c| b.standard_pain_reduction_when_well = float_arg(c, 1)),
        ("setPanicReductionValue", |b, c| b.panic_reduction_value = float_arg(c, 1)),
        ("setDrunkIncreaseValue", |b, c| b.drunk_increase_value = float_arg(c, 1)),
        ("setDrunkReductionValue", |b, c| b.drunk_reduction_value = float_arg(c, 1)),
        ("setTimeToSneezeOrCough", |b, c| b.time_to_sneeze_or_cough = float_arg(c, 1)),
        ("setOverallBodyHealth", |b, c| b.overall_body_health = float_arg(c, 1)),
        ("setDamageModCount", |b, c| b.damage_mod_count = int_arg(c, 1)),
        ("setOldNumZombiesVisible", |b, c| b.old_num_zombies_visible = int_arg(c, 1)),
        ("setCurrentNumZombiesVisible", |b, c| b.current_num_zombies_visible = int_arg(c, 1)),
        ("setRemotePainLevel", |b, c| b.remote_pain_level = int_arg(c, 1)),
        ("setSneezeCoughActive", |b, c| b.sneeze_cough_active = int_arg(c, 1)),
        ("setSneezeCoughTime", |b, c| b.sneeze_cough_time = int_arg(c, 1)),
        ("setSneezeCoughDelay", |b, c| b.sneeze_cough_delay = int_arg(c, 1)),
        ("setMildColdSneezeTimerMin", |b, c| b.mild_cold_sneeze_timer_min = int_arg(c, 1)),
        ("setMildColdSneezeTimerMax", |b, c| b.mild_cold_sneeze_timer_max = int_arg(c, 1)),
        ("setColdSneezeTimerMin", |b, c| b.cold_sneeze_timer_min = int_arg(c, 1)),
        ("setColdSneezeTimerMax", |b, c| b.cold_sneeze_timer_max = int_arg(c, 1)),
        ("setNastyColdSneezeTimerMin", |b, c| b.nasty_cold_sneeze_timer_min = int_arg(c, 1)),
        ("setNastyColdSneezeTimerMax", |b, c| b.nasty_cold_sneeze_timer_max = int_arg(c, 1)),
        ("setStandardHealthFromFoodTime", |b, c| b.standard_health_from_food_time = int_arg(c, 1)),
        ("OnFire", |b, c| b.is_on_fire = bool_arg(c, 1)),
        ("setIsOnFire", |b, c| b.is_on_fire = bool_arg(c, 1)),
        ("setBurntToDeath", |b, c| b.burnt_to_death = bool_arg(c, 1)),
        ("setHasACold", |b, c| b.has_a_cold = bool_arg(c, 1)),
        ("setInfected", |b, c| b.is_infected = bool_arg(c, 1)),
        ("setInf", |b, c| b.is_infected = bool_arg(c, 1)),
        ("setIsFakeInfected", |b, c| b.set_is_fake_infected(bool_arg(c, 1))),
        ("setReduceFakeInfection", |b, c| b.reduce_fake_infection = bool_arg(c, 1)),
        ("setWasDraggingCorpse", |b, c| b.was_dragging_corpse = bool_arg(c, 1)),
        ("AddGeneralHealth", |b, c| b.add_general_health(float_arg(c, 1))),
        ("ShowDebugInfo", |b, _| b.show_debug_info()),
        ("setBodyPartsLastState", |b, _| b.set_body_parts_last_state()),
    ];
    for (name, f) in sets {
        b.method(name, move |c| {
            let chr = bd_chr(c)?;
            f(&mut chr.borrow_mut().body_damage, c);
            Ok(0)
        });
    }
    b.method("UseBandageOnMostNeededPart", |c| {
        let v = bd_chr(c)?.borrow_mut().body_damage.use_bandage_on_most_needed_part();
        Ok(c.push(v))
    });
    b.method("setPanicIncreaseValue", {
        let sm = sm.clone();
        move |c| {
            let chr = bd_chr(c)?;
            let v = float_arg(c, 1);
            let des = chr.borrow().has_trait(&sm, "DESENSITIZED").map_err(java_error)?;
            chr.borrow_mut().body_damage.panic_increase_value = if des { 0.0 } else { v };
            Ok(0)
        }
    });
    b.method("getParentChar", |c| {
        let chr = bd_chr(c)?;
        Ok(c.push(character_value(&chr)))
    });
    b.method("getThermoregulator", |c| {
        let chr = bd_chr(c)?;
        Ok(c.push(crate::bind_nutrition::thermoregulator_value(&chr)))
    });
    b.method("getBodyParts", |c| {
        let chr = bd_chr(c)?;
        let ch = chr.clone();
        let read = move || (0..ch.borrow().body_damage.parts.len()).map(|i| body_part_value(&ch, i)).collect();
        Ok(c.push(Value::Object(Rc::new(JavaList::live(read, None)))))
    });
    b.method("getBodyPart", |c| {
        let chr = bd_chr(c)?;
        let i = part_arg(c, 1, &chr)?;
        Ok(c.push(body_part_value(&chr, i)))
    });
    type PartQuery = fn(&BodyPart) -> Value;
    let queries: [(&str, PartQuery); 14] = [
        ("getBodyPartHealth", |p| fv(p.health)),
        ("IsBandaged", |p| p.bandaged.into()),
        ("IsDeepWounded", |p| p.deep_wounded.into()),
        ("IsWounded", |p| p.deep_wounded.into()),
        ("IsBitten", |p| p.bitten.into()),
        ("IsBleeding", |p| p.bleeding.into()),
        ("IsBleedingStemmed", |p| p.is_bleeding_stemmed.into()),
        ("IsCauterized", |p| p.is_cauterized.into()),
        ("IsScratched", |p| p.scratched.into()),
        ("IsCut", |p| (p.cut_time > 0.0).into()),
        ("IsStitched", |p| p.stitched.into()),
        ("IsFakeInfected", |p| p.is_fake_infected.into()),
        ("doesBodyPartHaveInjury", |p| p.has_injury().into()),
        ("isBodyPartBleeding", |p| (p.bleeding_time > 0.0).into()),
    ];
    for (name, f) in queries {
        b.method(name, move |c| {
            let chr = bd_chr(c)?;
            // `IsFakeInfected()` without a part is the body's flag.
            if name == "IsFakeInfected" && c.arg(1).is_nil() {
                let v = chr.borrow().body_damage.is_fake_infected;
                return Ok(c.push(v));
            }
            let i = part_arg(c, 1, &chr)?;
            let v = f(&chr.borrow().body_damage.parts[i]);
            Ok(c.push(v))
        });
    }
    b.method("IsInfected", |c| {
        let chr = bd_chr(c)?;
        if c.arg(1).is_nil() {
            let v = chr.borrow().body_damage.is_infected;
            return Ok(c.push(v));
        }
        let i = part_arg(c, 1, &chr)?;
        let v = chr.borrow().body_damage.parts[i].is_infected;
        Ok(c.push(v))
    });
    type PairQuery = fn(&BodyPart) -> bool;
    let pairs: [(&str, PairQuery); 2] =
        [("doBodyPartsHaveInjuries", |p| p.has_injury()), ("areBodyPartsBleeding", |p| p.bleeding_time > 0.0)];
    for (name, f) in pairs {
        b.method(name, move |c| {
            let chr = bd_chr(c)?;
            let a = part_arg(c, 1, &chr)?;
            let r = f(&chr.borrow().body_damage.parts[a]) || {
                let b = part_arg(c, 2, &chr)?;
                f(&chr.borrow().body_damage.parts[b])
            };
            Ok(c.push(r))
        });
    }
    let ch = chars.clone();
    b.method("getBodyPartName", move |c| {
        let i = match c.arg(1) {
            Value::Num(n) => n as i32,
            _ => enum_arg(c, 1, BODY_PART_TYPE_CLASS)?.map_or(-1, |i| i as i32),
        };
        let name = ch.borrow().sim.body_part_types.name_of(i).to_string();
        Ok(c.push(name))
    });
    type PartSet = fn(&mut BodyPart, &NativeCall<'_>);
    let part_sets: [(&str, PartSet); 7] = [
        ("SetBleeding", |p, c| p.set_bleeding(bool_arg(c, 2))),
        ("SetBleedingStemmed", |p, c| p.set_bleeding_stemmed(bool_arg(c, 2))),
        ("SetCauterized", |p, c| p.set_cauterized(bool_arg(c, 2))),
        ("SetWounded", |p, c| p.set_deep_wounded(bool_arg(c, 2))),
        ("DisableFakeInfection", |p, _| p.disable_fake_infection()),
        ("AddDamage", |p, c| p.add_damage(float_arg(c, 2))),
        ("addStiffness", |p, c| p.add_stiffness(float_arg(c, 2))),
    ];
    for (name, f) in part_sets {
        b.method(name, move |c| {
            let chr = bd_chr(c)?;
            let i = match c.arg(1) {
                // `addStiffness(BodyPart, float)`.
                Value::Object(o) if o.as_any().is::<BodyPartObject>() => {
                    o.as_any().downcast_ref::<BodyPartObject>().unwrap().index
                }
                _ => part_arg(c, 1, &chr)?,
            };
            f(&mut chr.borrow_mut().body_damage.parts[i], c);
            Ok(0)
        });
    }
    b.method("SetBandaged", |c| {
        let chr = bd_chr(c)?;
        let i = part_arg(c, 1, &chr)?;
        let t = str_arg(c, 5).map(|s| s.to_string());
        chr.borrow_mut().body_damage.parts[i].set_bandaged(bool_arg(c, 2), float_arg(c, 3), bool_arg(c, 4), t);
        Ok(0)
    });
    let s = sm.clone();
    b.method("SetBitten", move |c| {
        let chr = bd_chr(c)?;
        let i = part_arg(c, 1, &chr)?;
        let bitten = bool_arg(c, 2);
        let infected = (c.nargs() > 3).then(|| bool_arg(c, 3));
        run(c, &chr, |ch, ctx| {
            let env = ch.injury_env(&s, ctx)?;
            let p = &mut ch.body_damage.parts[i];
            match infected {
                Some(inf) => p.set_bitten_infected(ctx, bitten, inf),
                None => p.set_bitten(&env, bitten),
            }
            Ok(())
        })?;
        Ok(0)
    });
    type EnvSet = fn(&mut BodyPart, &InjuryEnv<'_>, bool);
    let env_sets: [(&str, EnvSet); 3] = [
        ("SetScratched", |p, e, v| p.set_scratched(e, v, false)),
        ("SetScratchedFromWeapon", |p, e, v| p.set_scratched_weapon(e, v)),
        ("SetCut", |p, e, v| p.set_cut(e, v, false)),
    ];
    for (name, f) in env_sets {
        let s = sm.clone();
        b.method(name, move |c| {
            let chr = bd_chr(c)?;
            let i = part_arg(c, 1, &chr)?;
            let v = bool_arg(c, 2);
            run(c, &chr, |ch, ctx| {
                let env = ch.injury_env(&s, ctx)?;
                f(&mut ch.body_damage.parts[i], &env, v);
                Ok(())
            })?;
            Ok(0)
        });
    }

    // Owner-dependent methods.
    macro_rules! owner {
        ($name:expr, |$ch:ident, $c:ident, $sm:ident, $chars:ident, $ctx:ident, $args:ident| $body:expr) => {{
            let s = sm.clone();
            let k = chars.clone();
            b.method($name, move |$c| {
                let chr = bd_chr($c)?;
                let $sm: &ScriptManager = &s;
                let chars_ref = k.borrow();
                let $chars: &Characters = &chars_ref;
                let $args: Vec<Value> = (0..$c.nargs()).map(|i| $c.arg(i)).collect();
                let r = run($c, &chr, |$ch, $ctx| {
                    let _ = (&$sm, &$chars, &$ctx, &$args);
                    $body
                })?;
                Ok($c.push(r))
            });
        }};
    }
    owner!("Update", |ch, c, sm, chars, ctx, args| ch.body_damage_update(sm, chars, ctx).map(|_| Value::Nil));
    owner!("UpdateWetness", |ch, c, sm, chars, ctx, args| ch.update_wetness(sm, chars, ctx).map(|_| Value::Nil));
    owner!("UpdateCold", |ch, c, sm, chars, ctx, args| ch.update_cold(sm, chars, ctx).map(|_| Value::Nil));
    owner!("UpdateBoredom", |ch, c, sm, chars, ctx, args| ch.update_boredom(sm, chars, ctx).map(|_| Value::Nil));
    owner!("UpdateStrength", |ch, c, sm, chars, ctx, args| ch.update_strength(sm, chars).map(|_| Value::Nil));
    owner!("UpdatePanicState", |ch, c, sm, chars, ctx, args| ch.update_panic_state(sm, chars, ctx).map(|_| Value::Nil));
    owner!("UpdateDiscomfort", |ch, c, sm, chars, ctx, args| ch.update_discomfort(sm, chars, ctx).map(|_| Value::Nil));
    owner!("UpdateDraggingCorpse", |ch, c, sm, chars, ctx, args| {
        ch.update_dragging_corpse();
        Ok(Value::Nil)
    });
    owner!("TriggerSneezeCough", |ch, c, sm, chars, ctx, args| ch.trigger_sneeze_cough(sm, ctx).map(|_| Value::Nil));
    owner!("RestoreToFullHealth", |ch, c, sm, chars, ctx, args| {
        ch.restore_to_full_health(chars).map(|_| Value::Nil)
    });
    owner!("calculateOverallHealth", |ch, c, sm, chars, ctx, args| {
        ch.calculate_overall_health(chars);
        Ok(Value::Nil)
    });
    owner!("ReducePanic", |ch, c, sm, chars, ctx, args| ch.reduce_panic(chars, ctx).map(|_| Value::Nil));
    owner!("JustTookPainMeds", |ch, c, sm, chars, ctx, args| ch.just_took_pain_meds(chars).map(|_| Value::Nil));
    owner!("AddRandomDamage", |ch, c, sm, chars, ctx, args| ch.add_random_damage(sm, ctx).map(|_| Value::Nil));
    owner!("pickMortalityDuration", |ch, c, sm, chars, ctx, args| ch.pick_mortality_duration(sm, ctx).map(fv));
    owner!("GetBaseCorpseSickness", |ch, c, sm, chars, ctx, args| Ok(fv(ch.base_corpse_sickness(ctx))));
    owner!("getGeneralWoundInfectionLevel", |ch, c, sm, chars, ctx, args| {
        Ok(fv(ch.body_damage.general_wound_infection_level(ctx)))
    });
    owner!("getApparentInfectionLevel", |ch, c, sm, chars, ctx, args| {
        ch.body_damage.apparent_infection_level(&ch.stats, &chars.sim.character_stats).map(fv)
    });
    let num = |args: &[Value], i: usize| match args.get(i) {
        Some(Value::Num(n)) => *n,
        _ => 0.0,
    };
    owner!("ReduceGeneralHealth", |ch, c, sm, chars, ctx, args| {
        ch.reduce_general_health(chars, num(&args, 1) as f32);
        Ok(Value::Nil)
    });
    owner!("IncreasePanic", |ch, c, sm, chars, ctx, args| {
        ch.increase_panic(sm, chars, num(&args, 1) as i32).map(|_| Value::Nil)
    });
    owner!("IncreasePanicFloat", |ch, c, sm, chars, ctx, args| {
        ch.increase_panic_float(sm, chars, num(&args, 1) as f32).map(|_| Value::Nil)
    });
    owner!("JustDrankBoozeFluid", |ch, c, sm, chars, ctx, args| {
        ch.just_drank_booze_fluid(chars, num(&args, 1) as f32).map(|_| Value::Nil)
    });
    owner!("decreaseBodyWetness", |ch, c, sm, chars, ctx, args| {
        ch.decrease_body_wetness(chars, num(&args, 1) as f32).map(|_| Value::Nil)
    });
    owner!("increaseBodyWetness", |ch, c, sm, chars, ctx, args| {
        ch.increase_body_wetness(chars, num(&args, 1) as f32).map(|_| Value::Nil)
    });
    owner!("applyDamageFromWeapon", |ch, c, sm, chars, ctx, args| {
        ch.apply_damage_from_weapon(
            sm,
            chars,
            ctx,
            num(&args, 1) as i32,
            num(&args, 2) as f32,
            num(&args, 3) as i32,
            num(&args, 4) as f32,
        )
        .map(|_| Value::Nil)
    });
    let s = sm.clone();
    let k = chars.clone();
    b.method("setScratchedWindow", move |c| {
        let chr = bd_chr(c)?;
        let chars = k.borrow();
        let slot = run(c, &chr, |ch, ctx| ch.set_scratched_window(&s, &chars, ctx))?;
        Ok(c.push(slot.map(|i| body_part_value(&chr, i)).unwrap_or_default()))
    });
    let k = chars.clone();
    b.static_fn("getSicknessFromCorpsesRate", move |c| {
        let _ = &k;
        let n = int_arg(c, 0);
        let ctx = crate::bind_character_sim::current_sim_context();
        Ok(c.push(fv(GameCharacter::<crate::bind_characters::LuaItem>::sickness_from_corpses_rate(&*ctx, n))))
    });
}

type BpGet = fn(&BodyPart) -> Value;
type BpSet = fn(&mut BodyPart, &NativeCall<'_>);

fn register_body_part(thread: &mut Thread, sm: &Rc<ScriptManager>, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, BODY_PART_CLASS, None);
    let gets: Vec<(&str, BpGet)> = vec![
        ("getHealth", |p| fv(p.health)),
        ("getAlcoholLevel", |p| fv(p.alcohol_level)),
        ("getBandageLife", |p| fv(p.bandage_life)),
        ("getBandageNeededDamageLevel", |p| fv(p.bandage_needed_damage_level())),
        ("getBiteTime", |p| fv(p.bite_time)),
        ("getBleedingTime", |p| fv(p.bleeding_time)),
        ("getBurnSpeedModifier", |p| fv(p.burn_speed_modifier)),
        ("getBurnTime", |p| fv(p.burn_time)),
        ("getComfreyFactor", |p| fv(p.comfrey_factor)),
        ("getCutSpeedModifier", |p| fv(p.cut_speed_modifier)),
        ("getCutTime", |p| fv(p.cut_time)),
        ("getDamageScaler", |p| fv(p.damage_scaler)),
        ("getDeepWoundSpeedModifier", |p| fv(p.deep_wound_speed_modifier)),
        ("getDeepWoundTime", |p| fv(p.deep_wound_time)),
        ("getDistToCore", |p| fv(p.dist_to_core())),
        ("getFractureTime", |p| fv(p.fracture_time)),
        ("getGarlicFactor", |p| fv(p.garlic_factor)),
        ("getInnerTemperature", |p| fv(p.inner_temperature())),
        ("getLastTimeBurnWash", |p| fv(p.last_time_burn_wash)),
        ("getPlantainFactor", |p| fv(p.plantain_factor)),
        ("getScratchSpeedModifier", |p| fv(p.scratch_speed_modifier)),
        ("getScratchTime", |p| fv(p.scratch_time)),
        ("getSkinSurface", |p| fv(p.skin_surface())),
        ("getSkinTemperature", |p| fv(p.skin_temperature())),
        ("getSplintFactor", |p| fv(p.splint_factor)),
        ("getStiffness", |p| fv(p.stiffness)),
        ("getStitchTime", |p| fv(p.stitch_time)),
        ("getWetness", |p| fv(p.wetness)),
        ("getWoundInfectionLevel", |p| fv(p.wound_infection_level)),
        ("getIndex", |p| (p.index as i32).into()),
        ("getBandageType", |p| opt_str(&p.bandage_type)),
        ("getSplintItem", |p| opt_str(&p.splint_item)),
        ("manipulatingUsername", |p| opt_str(&p.manipulating_username)),
        ("getType", |p| enum_value(BODY_PART_TYPE_CLASS, &p.type_field)),
        ("HasInjury", |p| p.has_injury().into()),
        ("IsBleedingStemmed", |p| p.is_bleeding_stemmed.into()),
        ("IsCauterized", |p| p.is_cauterized.into()),
        ("IsFakeInfected", |p| p.is_fake_infected.into()),
        ("IsInfected", |p| p.is_infected.into()),
        ("bandaged", |p| p.bandaged.into()),
        ("bitten", |p| p.bitten.into()),
        ("bleeding", |p| p.bleeding.into()),
        ("deepWounded", |p| p.deep_wounded.into()),
        ("isDeepWounded", |p| p.deep_wounded.into()),
        ("haveBullet", |p| p.have_bullet.into()),
        ("haveGlass", |p| p.have_glass.into()),
        ("isBandageDirty", |p| p.is_bandage_dirty().into()),
        ("isBurnt", |p| p.is_burnt().into()),
        ("isCut", |p| p.cut.into()),
        ("isGetBandageXp", |p| p.get_bandage_xp.into()),
        ("isGetSplintXp", |p| p.get_splint_xp.into()),
        ("isGetStitchXp", |p| p.get_stitch_xp.into()),
        ("isInfectedWound", |p| p.infected_wound.into()),
        ("isNeedBurnWash", |p| p.need_burn_wash.into()),
        ("isSplint", |p| p.splint.into()),
        ("scratched", |p| p.scratched.into()),
        ("stitched", |p| p.stitched.into()),
    ];
    for (name, f) in gets {
        b.method(name, move |c| {
            let (chr, i) = bp_this(c)?;
            let v = f(&chr.borrow().body_damage.parts[i]);
            Ok(c.push(v))
        });
    }
    b.method("getAdditionalPain", |c| {
        let (chr, i) = bp_this(c)?;
        let p = &chr.borrow().body_damage.parts[i];
        let v = if c.nargs() > 1 { p.additional_pain_with(bool_arg(c, 1)) } else { p.additional_pain };
        Ok(c.push(fv(v)))
    });
    b.method("getParentChar", |c| {
        let (chr, _) = bp_this(c)?;
        Ok(c.push(character_value(&chr)))
    });
    let sets: Vec<(&str, BpSet)> = vec![
        ("AddDamage", |p, c| p.add_damage(float_arg(c, 1))),
        ("AddHealth", |p, c| p.add_health(float_arg(c, 1))),
        ("ReduceHealth", |p, c| p.reduce_health(float_arg(c, 1))),
        ("SetHealth", |p, c| p.set_health(float_arg(c, 1))),
        ("DisableFakeInfection", |p, _| p.disable_fake_infection()),
        ("RestoreToFullHealth", |p, _| p.restore_to_full_health()),
        ("SetBleedingStemmed", |p, c| p.set_bleeding_stemmed(bool_arg(c, 1))),
        ("SetCauterized", |p, c| p.set_cauterized(bool_arg(c, 1))),
        ("SetFakeInfected", |p, c| p.is_fake_infected = bool_arg(c, 1)),
        ("SetInfected", |p, c| p.is_infected = bool_arg(c, 1)),
        ("addStiffness", |p, c| p.add_stiffness(float_arg(c, 1))),
        ("setAdditionalPain", |p, c| p.additional_pain = float_arg(c, 1)),
        ("setAlcoholLevel", |p, c| p.alcohol_level = float_arg(c, 1)),
        ("setBandageLife", |p, c| p.set_bandage_life(float_arg(c, 1))),
        ("setBandageType", |p, c| p.bandage_type = str_arg(c, 1).map(|s| s.to_string())),
        ("setBiteTime", |p, c| p.bite_time = float_arg(c, 1)),
        ("setBleeding", |p, c| p.set_bleeding(bool_arg(c, 1))),
        ("setBleedingTime", |p, c| p.set_bleeding_time(float_arg(c, 1))),
        ("setBurnSpeedModifier", |p, c| p.burn_speed_modifier = float_arg(c, 1)),
        ("setBurnTime", |p, c| p.burn_time = float_arg(c, 1)),
        ("setComfreyFactor", |p, c| p.set_comfrey_factor(float_arg(c, 1))),
        ("setCutSpeedModifier", |p, c| p.cut_speed_modifier = float_arg(c, 1)),
        ("setCutTime", |p, c| p.set_cut_time(float_arg(c, 1))),
        ("setDeepWoundSpeedModifier", |p, c| p.deep_wound_speed_modifier = float_arg(c, 1)),
        ("setDeepWoundTime", |p, c| p.deep_wound_time = float_arg(c, 1)),
        ("setDeepWounded", |p, c| p.set_deep_wounded(bool_arg(c, 1))),
        ("setFractureTime", |p, c| p.fracture_time = float_arg(c, 1)),
        ("setGarlicFactor", |p, c| p.set_garlic_factor(float_arg(c, 1))),
        ("setGetBandageXp", |p, c| p.get_bandage_xp = bool_arg(c, 1)),
        ("setGetSplintXp", |p, c| p.get_splint_xp = bool_arg(c, 1)),
        ("setGetStitchXp", |p, c| p.get_stitch_xp = bool_arg(c, 1)),
        ("setHaveGlass", |p, c| p.have_glass = bool_arg(c, 1)),
        ("setInfectedWound", |p, c| p.infected_wound = bool_arg(c, 1)),
        ("setLastTimeBurnWash", |p, c| p.last_time_burn_wash = float_arg(c, 1)),
        ("setManipulatingUsername", |p, c| p.manipulating_username = str_arg(c, 1).map(|s| s.to_string())),
        ("setNeedBurnWash", |p, c| p.set_need_burn_wash(bool_arg(c, 1))),
        ("setPlantainFactor", |p, c| p.set_plantain_factor(float_arg(c, 1))),
        ("setScratchSpeedModifier", |p, c| p.scratch_speed_modifier = float_arg(c, 1)),
        ("setScratchTime", |p, c| p.set_scratch_time(float_arg(c, 1))),
        ("setSplint", |p, c| p.set_splint(bool_arg(c, 1), float_arg(c, 2))),
        ("setSplintFactor", |p, c| p.splint_factor = float_arg(c, 1)),
        ("setSplintItem", |p, c| p.splint_item = str_arg(c, 1).map(|s| s.to_string())),
        ("setStiffness", |p, c| p.set_stiffness(float_arg(c, 1))),
        ("setStitchTime", |p, c| p.stitch_time = float_arg(c, 1)),
        ("setWetness", |p, c| p.set_wetness(float_arg(c, 1))),
        ("setWoundInfectionLevel", |p, c| p.set_wound_infection_level(float_arg(c, 1))),
        ("setBandaged", |p, c| {
            let t = if c.nargs() > 3 { str_arg(c, 4).map(|s| s.to_string()) } else { None };
            p.set_bandaged(bool_arg(c, 1), float_arg(c, 2), c.nargs() > 3 && bool_arg(c, 3), t)
        }),
    ];
    for (name, f) in sets {
        b.method(name, move |c| {
            let (chr, i) = bp_this(c)?;
            f(&mut chr.borrow_mut().body_damage.parts[i], c);
            Ok(0)
        });
    }
    let s = sm.clone();
    b.method("getPain", move |c| {
        let (chr, i) = bp_this(c)?;
        let _ = &s;
        let v = run(c, &chr, |ch, ctx| Ok(ch.body_damage.parts[i].pain(ctx)))?;
        Ok(c.push(fv(v)))
    });
    type EnvFn = fn(&mut BodyPart, &InjuryEnv<'_>, &[Value]);
    fn b1(a: &[Value], i: usize) -> bool {
        matches!(a.get(i), Some(Value::Bool(true)))
    }
    fn n1(a: &[Value], i: usize) -> f64 {
        match a.get(i) {
            Some(Value::Num(n)) => *n,
            _ => 0.0,
        }
    }
    let env_fns: [(&str, EnvFn); 13] = [
        ("SetBitten", |p, e, a| {
            if a.len() > 2 {
                p.set_bitten_infected(e.ctx, b1(a, 1), b1(a, 2))
            } else {
                p.set_bitten(e, b1(a, 1))
            }
        }),
        ("SetScratchedWeapon", |p, e, a| p.set_scratched_weapon(e, b1(a, 1))),
        ("SetScratchedWindow", |p, e, a| p.set_scratched_window(e, b1(a, 1))),
        ("setCut", |p, e, a| p.set_cut(e, b1(a, 1), if a.len() > 2 { b1(a, 2) } else { true })),
        ("setScratched", |p, e, a| p.set_scratched(e, b1(a, 1), b1(a, 2))),
        ("setStitched", |p, e, a| p.set_stitched(e, b1(a, 1))),
        ("setHaveBullet", |p, e, a| p.set_have_bullet(e, b1(a, 1), n1(a, 2) as i32)),
        ("damageFromFirearm", |p, e, a| p.damage_from_firearm(e, n1(a, 1) as f32)),
        ("generateBleeding", |p, e, _| p.generate_bleeding(e)),
        ("generateDeepWound", |p, e, _| p.generate_deep_wound(e)),
        ("generateDeepShardWound", |p, e, _| p.generate_deep_shard_wound(e)),
        ("generateZombieInfection", |p, e, a| p.generate_zombie_infection(e, n1(a, 1) as i32)),
        ("setBurned", |p, e, _| p.set_burned(e)),
    ];
    for (name, f) in env_fns {
        let s = sm.clone();
        b.method(name, move |c| {
            let args: Vec<Value> = (0..c.nargs()).map(|i| c.arg(i)).collect();
            with_env(c, &s, |p, e| f(p, e, &args))
        });
    }
    let s = sm.clone();
    b.method("generateFracture", move |c| {
        let v = float_arg(c, 1);
        with_env(c, &s, |p, e| p.generate_fracture(e.ctx, v))
    });
    let s = sm.clone();
    b.method("generateFractureNew", move |c| {
        let v = float_arg(c, 1);
        with_env(c, &s, |p, e| p.generate_fracture_new(e, v))
    });
    let k = chars.clone();
    let s = sm.clone();
    b.method("DamageUpdate", move |c| {
        let (chr, i) = bp_this(c)?;
        let chars = k.borrow();
        run(c, &chr, |ch, ctx| ch.body_part_damage_update(&s, &chars, ctx, i))?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("hasDirtyClothing", move |c| {
        let (chr, i) = bp_this(c)?;
        let v = chr.borrow().items.has_dirty_clothing(&s, i as i32);
        Ok(c.push(v))
    });
    let s = sm.clone();
    b.method("hasBloodyClothing", move |c| {
        let (chr, i) = bp_this(c)?;
        let v = chr.borrow().items.has_bloody_clothing(&s, i as i32);
        Ok(c.push(v))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_character_sim::LuaGameCharacter;
    use pz_characters::character::CharacterKind;
    use pz_kahlua::TableRef;
    use pz_scripts::registry::builtin::{BodyPartDecl, BuiltinTables, CharacterStatDecl};

    fn exec(t: &mut Thread, src: &str) -> Result<(), String> {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map(|_| ()).map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e)))
    }

    /// Invented parts, stats and traits.
    fn setup() -> (Thread, LuaGameCharacterRef) {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        exec(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end")
            .unwrap();
        crate::bind_java_util::register(&mut t);
        let sm = Rc::new(ScriptManager::new());
        {
            let mut st = sm.statics.items.borrow_mut();
            let (reg, field, id) = ("character_trait", "FAST_HEALER", "t:quickheal");
            sm.registries.get(reg).unwrap().borrow_mut().register_id(id, false).unwrap();
            st.constants.insert(reg, field, id);
        }
        let mut tables = BuiltinTables::default();
        for f in ["PAIN", "WETNESS", "PANIC", "STRESS"] {
            tables.character_stats.push(CharacterStatDecl {
                field: f.into(),
                id: f.to_lowercase(),
                minimum: 0.0,
                maximum: 100.0,
                default: 0.0,
            });
        }
        let part = |f: &str, dm: f32| BodyPartDecl {
            field: f.into(),
            name: f.into(),
            pain_modifier: 0.5,
            damage_modifier: dm,
            bleeding_time_modifier: 0.25,
            ..Default::default()
        };
        tables.body_parts = vec![part("Hand_L", 0.5), part("Neck", 0.25), part("MAX", 1.0)];
        *sm.builtin_tables.borrow_mut() = tables.clone();
        let chars = Characters { sim: pz_characters::SimStatics::from_tables(&tables).unwrap(), ..Default::default() };
        let chars = Rc::new(RefCell::new(chars));
        crate::bind_body_damage::register(&mut t, &sm);
        crate::bind_character_sim::register(&mut t, sm.clone(), chars.clone());
        register(&mut t, sm.clone(), chars);
        let chr: LuaGameCharacter = GameCharacter::new(&sm, CharacterKind::Player);
        let chr = Rc::new(RefCell::new(chr));
        t.globals().set_str("player", character_value(&chr));
        (t, chr)
    }

    #[test]
    fn body_damage_from_lua() {
        let (mut t, chr) = setup();
        exec(
            &mut t,
            r#"
            local bd = player:getBodyDamage()
            assert(bd == player:getBodyDamage() and bd:getParentChar() == player)
            assert(bd:getBodyParts():size() == 2 and bd:getHealth() == 100)
            local neck = bd:getBodyPart(BodyPartType.Neck)
            assert(neck == bd:getBodyParts():get(1) and neck:getType() == BodyPartType.Neck)
            assert(neck:getIndex() == 1 and bd:getBodyPartName(1) == "Neck")
            neck:AddDamage(40)
            assert(neck:getHealth() == 60 and bd:getBodyPartHealth(BodyPartType.Neck) == 60)
            bd:calculateOverallHealth()
            assert(bd:getOverallBodyHealth() == 90, tostring(bd:getOverallBodyHealth()))
            bd:SetBandaged(1, true, 4, true, "Bandage")
            assert(bd:IsBandaged(BodyPartType.Neck) and neck:getBandageType() == "Bandage")
            neck:setBleedingTime(2)
            assert(not neck:bleeding(), "bandaged parts do not start bleeding")
            assert(not pcall(function() bd:getBodyPart(BodyPartType.MAX) end))
            neck:setScratchTime(500)
            assert(neck:getScratchTime() == 100)
            assert(BodyDamage.InfectionLevelToZombify > 0)
            bd:AddGeneralHealth(8)
            assert(neck:getHealth() == 68)
            "#,
        )
        .unwrap();
        assert_eq!(chr.borrow().body_damage.parts[1].bandage_life, 4.0);
    }
}
