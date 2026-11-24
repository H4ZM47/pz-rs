//! Lua bindings for Build 42.22 live animal classes:
//! - `zombie.characters.animals.IsoAnimal` (subclass of `IsoPlayer`)
//! - `zombie.characters.animals.datas.AnimalData`
//! - `zombie.characters.animals.AnimalGene`
//! - `zombie.characters.animals.AnimalAllele`
//!
//! Unlocks Build 42.22 animal husbandry UIs and context menus (`ISAnimalUI`,
//! `ISAnimalContextMenu`, `ISAnimalZoneUI`, `ISButcherHookUI`, `ISHutchUI`).

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_characters::pz_animals::{AnimalAllele, AnimalGene};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};

use crate::bind_character_sim::{
    character_value, CharacterObject, LuaGameCharacter, LuaGameCharacterRef, ISO_ANIMAL_CLASS,
    ISO_PLAYER_CLASS,
};
use crate::bind_characters::shared_characters;
use crate::bind_java_util::{list_value, JavaMap};
use crate::bind_registry::shared_script_manager;
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, this};

pub const ANIMAL_DATA_CLASS: &str = "zombie.characters.animals.datas.AnimalData";
pub const ANIMAL_GENE_CLASS: &str = "zombie.characters.animals.AnimalGene";
pub const ANIMAL_ALLELE_CLASS: &str = "zombie.characters.animals.AnimalAllele";

fn opt_str(s: &Option<String>) -> Value {
    match s {
        Some(v) => Value::str(v.as_str()),
        None => Value::Nil,
    }
}

struct SimpleRand(u64);

impl pz_world::GameRand for SimpleRand {
    fn next(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x % (bound as u64)) as i32
    }

    fn next_float(&mut self) -> f32 {
        (self.next(1_000_000) as f32) / 1_000_000.0
    }
}

// ---- userdata wrappers ------------------------------------------------------

#[derive(Clone)]
pub struct LuaAnimalAllele(pub Rc<RefCell<AnimalAllele>>);

impl LuaObject for LuaAnimalAllele {
    fn class_name(&self) -> &str {
        ANIMAL_ALLELE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
pub struct LuaAnimalGene(pub Rc<RefCell<AnimalGene>>);

impl LuaObject for LuaAnimalGene {
    fn class_name(&self) -> &str {
        ANIMAL_GENE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Live view onto an owning `IsoAnimal`'s `AnimalData`.
#[derive(Clone)]
pub struct LuaAnimalData(pub LuaGameCharacterRef);

impl LuaObject for LuaAnimalData {
    fn class_name(&self) -> &str {
        ANIMAL_DATA_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn allele_value(a: AnimalAllele) -> Value {
    Value::Object(Rc::new(LuaAnimalAllele(Rc::new(RefCell::new(a)))))
}

pub fn gene_value(g: AnimalGene) -> Value {
    Value::Object(Rc::new(LuaAnimalGene(Rc::new(RefCell::new(g)))))
}

pub fn animal_data_value(chr: &LuaGameCharacterRef) -> Value {
    Value::Object(Rc::new(LuaAnimalData(chr.clone())))
}

fn animal_char(c: &NativeCall<'_>) -> LuaResult<LuaGameCharacterRef> {
    let o = this::<CharacterObject>(c)?;
    let co = o.as_any().downcast_ref::<CharacterObject>().unwrap();
    Ok(co.0.clone())
}

fn player_online_id(v: &Value) -> i16 {
    let Value::Object(o) = v else { return 0 };
    let Some(co) = o.as_any().downcast_ref::<CharacterObject>() else {
        return 0;
    };
    co.0.try_borrow().map(|c| c.id as i16).unwrap_or(0)
}

/// Registers `AnimalAllele`, `AnimalGene`, `AnimalData`, and `IsoAnimal`
/// methods on `thread`. Call after `bind_character_sim::register_shared`.
pub fn register(thread: &mut Thread) {
    register_animal_allele(thread);
    register_animal_gene(thread);
    register_animal_data(thread);
    register_iso_animal(thread);
}

fn register_animal_allele(thread: &mut Thread) {
    let b = expose_class(thread, ANIMAL_ALLELE_CLASS, None);
    b.ctor(|c| {
        if let Value::Object(o) = c.arg(0) {
            if let Some(src) = o.as_any().downcast_ref::<LuaAnimalAllele>() {
                return Ok(c.push(allele_value(src.0.borrow().clone())));
            }
        }
        Ok(c.push(allele_value(AnimalAllele::default())))
    });
    b.method("getName", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        let a = o.as_any().downcast_ref::<LuaAnimalAllele>().unwrap().0.borrow();
        Ok(c.push(opt_str(&a.name)))
    });
    b.method("getCurrentValue", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        let a = o.as_any().downcast_ref::<LuaAnimalAllele>().unwrap().0.borrow();
        Ok(c.push(Value::Num(a.current_value as f64)))
    });
    b.method("setCurrentValue", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        o.as_any()
            .downcast_ref::<LuaAnimalAllele>()
            .unwrap()
            .0
            .borrow_mut()
            .current_value = num_arg(c, 1) as f32;
        Ok(0)
    });
    b.method("getTrueRatioValue", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        let a = o.as_any().downcast_ref::<LuaAnimalAllele>().unwrap().0.borrow();
        Ok(c.push(Value::Num(a.true_ratio_value as f64)))
    });
    b.method("setTrueRatioValue", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        o.as_any()
            .downcast_ref::<LuaAnimalAllele>()
            .unwrap()
            .0
            .borrow_mut()
            .true_ratio_value = num_arg(c, 1) as f32;
        Ok(0)
    });
    b.method("isDominant", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        let a = o.as_any().downcast_ref::<LuaAnimalAllele>().unwrap().0.borrow();
        Ok(c.push(Value::Bool(a.dominant)))
    });
    b.method("setDominant", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        o.as_any()
            .downcast_ref::<LuaAnimalAllele>()
            .unwrap()
            .0
            .borrow_mut()
            .dominant = bool_arg(c, 1);
        Ok(0)
    });
    b.method("isUsed", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        let a = o.as_any().downcast_ref::<LuaAnimalAllele>().unwrap().0.borrow();
        Ok(c.push(Value::Bool(a.used)))
    });
    b.method("setUsed", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        o.as_any()
            .downcast_ref::<LuaAnimalAllele>()
            .unwrap()
            .0
            .borrow_mut()
            .used = bool_arg(c, 1);
        Ok(0)
    });
    b.method("getGeneticDisorder", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        let a = o.as_any().downcast_ref::<LuaAnimalAllele>().unwrap().0.borrow();
        Ok(c.push(opt_str(&a.genetic_disorder)))
    });
    b.method("setGeneticDisorder", |c| {
        let o = this::<LuaAnimalAllele>(c)?;
        o.as_any()
            .downcast_ref::<LuaAnimalAllele>()
            .unwrap()
            .0
            .borrow_mut()
            .genetic_disorder = str_arg(c, 1).map(|s| s.to_string());
        Ok(0)
    });
}

fn register_animal_gene(thread: &mut Thread) {
    let b = expose_class(thread, ANIMAL_GENE_CLASS, None);
    b.ctor(|c| {
        if let Value::Object(o) = c.arg(0) {
            if let Some(src) = o.as_any().downcast_ref::<LuaAnimalGene>() {
                return Ok(c.push(gene_value(src.0.borrow().clone())));
            }
        }
        let mut rng = SimpleRand(0x6e65_7767_656e_6501);
        Ok(c.push(gene_value(AnimalGene::new(&mut rng))))
    });
    b.method("getName", |c| {
        let o = this::<LuaAnimalGene>(c)?;
        let g = o.as_any().downcast_ref::<LuaAnimalGene>().unwrap().0.borrow();
        Ok(c.push(opt_str(&g.name)))
    });
    b.method("getAllele1", |c| {
        let o = this::<LuaAnimalGene>(c)?;
        let g = o.as_any().downcast_ref::<LuaAnimalGene>().unwrap().0.borrow();
        Ok(c.push(allele_value(g.allele1.clone())))
    });
    b.method("getAllele2", |c| {
        let o = this::<LuaAnimalGene>(c)?;
        let g = o.as_any().downcast_ref::<LuaAnimalGene>().unwrap().0.borrow();
        Ok(c.push(allele_value(g.allele2.clone())))
    });
    b.method("getUsedGene", |c| {
        let o = this::<LuaAnimalGene>(c)?;
        let g = o.as_any().downcast_ref::<LuaAnimalGene>().unwrap().0.borrow();
        Ok(c.push(allele_value(g.used_gene().clone())))
    });
    b.method("initUsedGene", |c| {
        let o = this::<LuaAnimalGene>(c)?;
        let mut rng = SimpleRand(0x696e_6974_7573_6564);
        o.as_any()
            .downcast_ref::<LuaAnimalGene>()
            .unwrap()
            .0
            .borrow_mut()
            .init_used_gene(&mut rng);
        Ok(0)
    });
}

fn register_animal_data(thread: &mut Thread) {
    let b = expose_class(thread, ANIMAL_DATA_CLASS, None);
    macro_rules! data_get {
        ($name:literal, |$c:ident, $a:ident| $body:expr) => {
            b.method($name, |c| {
                let o = this::<LuaAnimalData>(c)?;
                let chr = o.as_any().downcast_ref::<LuaAnimalData>().unwrap().0.borrow();
                let out = match chr.animal_fields() {
                    Some(f) => {
                        let $c = &*chr;
                        let $a = &f.animal;
                        $body
                    }
                    None => Value::Nil,
                };
                Ok(c.push(out))
            });
        };
    }
    macro_rules! data_set {
        ($name:literal, |$call:ident, $a:ident| $body:expr) => {
            b.method($name, |c| {
                let o = this::<LuaAnimalData>(c)?;
                let mut chr = o.as_any().downcast_ref::<LuaAnimalData>().unwrap().0.borrow_mut();
                if let Some(f) = chr.animal_fields_mut() {
                    let $call = c;
                    let $a = &mut f.animal;
                    $body;
                }
                Ok(0)
            });
        };
    }

    data_get!("getAge", |_c, a| Value::Num(a.data.age as f64));
    data_set!("setAge", |c, a| a.data.age = int_arg(c, 1));
    data_get!("getSize", |_c, a| Value::Num(a.data.size as f64));
    data_set!("setSize", |c, a| a.data.size = num_arg(c, 1) as f32);
    data_set!("setSizeForced", |c, a| a.data.size = num_arg(c, 1) as f32);
    data_get!("getOriginalSize", |_c, a| Value::Num(a.data.original_size as f64));
    data_get!("getWeight", |_c, a| Value::Num(a.data.weight as f64));
    data_set!("setWeight", |c, a| a.data.weight = num_arg(c, 1) as f32);
    data_get!("getMilkQuantity", |_c, a| Value::Num(a.data.milk_qty as f64));
    data_set!("setMilkQuantity", |c, a| a.data.milk_qty = num_arg(c, 1) as f32);
    data_get!("getMaxMilkActual", |_c, a| Value::Num(a.data.max_milk_actual as f64));
    data_set!("setMaxMilkActual", |c, a| a.data.max_milk_actual = num_arg(c, 1) as f32);
    data_get!("canHaveMilk", |_c, a| Value::Bool(a.data.can_have_milk));
    data_set!("setCanHaveMilk", |c, a| a.data.can_have_milk = bool_arg(c, 1));
    data_get!("getWoolQuantity", |_c, a| Value::Num(a.data.wool_qty as f64));
    data_set!("setWoolQuantity", |c, a| a.data.wool_qty = num_arg(c, 1) as f32);
    data_get!("isPregnant", |_c, a| Value::Bool(a.data.pregnant));
    data_set!("setPregnant", |c, a| a.data.pregnant = bool_arg(c, 1));
    data_get!("getPregnancyTime", |_c, a| Value::Num(a.data.pregnant_time as f64));
    data_set!("setPregnancyTime", |c, a| a.data.pregnant_time = int_arg(c, 1));
    data_get!("isFertilized", |_c, a| Value::Bool(a.data.fertilized));
    data_set!("setFertilized", |c, a| a.data.fertilized = bool_arg(c, 1));
    data_get!("getFertilizedTime", |_c, a| Value::Num(a.data.fertilized_time as f64));
    b.method("setFertilizedTime", |c| {
        let o = this::<LuaAnimalData>(c)?;
        let mut chr = o.as_any().downcast_ref::<LuaAnimalData>().unwrap().0.borrow_mut();
        let v = int_arg(c, 1);
        if let Some(f) = chr.animal_fields_mut() {
            f.animal.data.fertilized_time = v;
        }
        Ok(c.push(Value::Num(v as f64)))
    });
    data_get!("getHutchPosition", |_c, a| Value::Num(a.data.hutch_position as f64));
    data_set!("setHutchPosition", |c, a| a.data.hutch_position = int_arg(c, 1));
    data_get!("getPreferredHutchPosition", |_c, a| Value::Num(a.data.preferred_hutch_position as f64));
    data_set!("setPreferredHutchPosition", |c, a| a.data.preferred_hutch_position = int_arg(c, 1));
    data_get!("getAttachedTreeX", |_c, a| Value::Num(a.data.attached_tree_x as f64));
    data_get!("getAttachedTreeY", |_c, a| Value::Num(a.data.attached_tree_y as f64));
    data_get!("getDaysSurvived", |c, _a| Value::Num((c.hours_survived / 24.0) as i32 as f64));
    data_get!("isFemale", |c, _a| {
        let fem = c.descriptor.as_ref().is_some_and(|d| d.borrow().is_female());
        Value::Bool(fem)
    });
    data_get!("getAgeString", |_c, a| Value::str(a.data_age_string()));
}

fn register_iso_animal(thread: &mut Thread) {
    let b = expose_class(thread, ISO_ANIMAL_CLASS, Some(ISO_PLAYER_CLASS));

    // Constructors:
    //   IsoAnimal.new(cell)
    //   IsoAnimal.new(cell, x, y, z, type, breedName [, skeleton])
    b.ctor(|c| {
        let sm = shared_script_manager();
        let chars_rc = shared_characters();
        let mut chr = {
            let mut chars = chars_rc.borrow_mut();
            LuaGameCharacter::new_animal(&sm, &mut chars)
                .map_err(|e| LuaError::runtime(format!("IsoAnimal.new: {e}")))?
        };
        if c.nargs() >= 5 {
            let x = num_arg(c, 1) as f32;
            let y = num_arg(c, 2) as f32;
            let z = num_arg(c, 3) as f32;
            chr.vars.x = x;
            chr.vars.y = y;
            chr.vars.z = z;
            chr.vars.last_x = x;
            chr.vars.last_y = y;
            chr.vars.last_z = z;
            chr.vars.next_x = x;
            chr.vars.next_y = y;
            chr.path.current_square = Some((x.floor() as i32, y.floor() as i32, z.floor() as i32));
            let animal_type = str_arg(c, 4).map(|s| s.to_string());
            let breed_name = str_arg(c, 5).map(|s| s.to_string());
            let skeleton = c.nargs() >= 7 && bool_arg(c, 6);
            if let Some(f) = chr.animal_fields_mut() {
                f.animal.animal_type = animal_type;
                f.animal.should_be_skeleton = skeleton;
                if let Some(bn) = breed_name {
                    f.animal.data.breed = Some(pz_characters::pz_animals::AnimalBreed {
                        name: Some(bn),
                        ..Default::default()
                    });
                }
            }
        }
        let chr_ref = Rc::new(RefCell::new(chr));
        Ok(c.push(character_value(&chr_ref)))
    });

    macro_rules! a_get {
        ($name:literal, |$c:ident, $f:ident| $body:expr) => {
            b.method($name, |c| {
                let chr_ref = animal_char(c)?;
                let chr = chr_ref.borrow();
                let out = match chr.animal_fields() {
                    Some($f) => {
                        let $c = &*chr;
                        $body
                    }
                    None => Value::Nil,
                };
                Ok(c.push(out))
            });
        };
    }

    macro_rules! a_set {
        ($name:literal, |$call:ident, $c:ident, $f:ident| $body:expr) => {
            b.method($name, |c| {
                let chr_ref = animal_char(c)?;
                let mut chr = chr_ref.borrow_mut();
                if let Some($f) = chr.animal_fields_mut() {
                    let $call = c;
                    let _ = &$f;
                    $body;
                }
                let _ = |_: &LuaGameCharacter| {};
                Ok(0)
            });
        };
    }

    b.method("getObjectName", |c| Ok(c.push(Value::str("Animal"))));
    b.method("getData", |c| {
        let chr_ref = animal_char(c)?;
        Ok(c.push(animal_data_value(&chr_ref)))
    });

    a_get!("getAnimalType", |_c, f| opt_str(&f.animal.animal_type));
    a_get!("getTypeAndBreed", |_c, f| {
        let ty = f.animal.animal_type.as_deref().unwrap_or("");
        let br = f
            .animal
            .breed()
            .and_then(|b| b.name.as_deref())
            .unwrap_or("");
        Value::str(&format!("{ty}{br}"))
    });
    a_get!("getAnimalID", |_c, f| Value::Num(f.animal.animal_id as f64));
    a_set!("setAnimalID", |c, _chr, f| f.animal.animal_id = int_arg(c, 1));
    a_get!("getItemID", |_c, f| Value::Num(f.animal.item_id as f64));
    a_set!("setItemID", |c, _chr, f| f.animal.item_id = int_arg(c, 1));
    a_get!("getCustomName", |_c, f| opt_str(&f.animal.custom_name));
    a_set!("setCustomName", |c, _chr, f| {
        f.animal.custom_name = str_arg(c, 1).map(|s| s.to_string())
    });
    a_get!("getFullName", |_c, f| {
        if let Some(custom) = f.animal.custom_name.as_deref().filter(|s| !s.is_empty()) {
            Value::str(custom)
        } else {
            Value::str(f.animal.animal_type.as_deref().unwrap_or("Animal"))
        }
    });
    a_get!("isExistInTheWorld", |_c, f| Value::Bool(f.exists_in_world));
    a_get!("isInvincible", |_c, f| Value::Bool(f.animal.invincible));
    a_set!("setIsInvincible", |c, _chr, f| f.animal.invincible = bool_arg(c, 1));
    a_get!("isWild", |_c, f| Value::Bool(f.animal.is_wild()));
    a_set!("setWild", |c, _chr, f| f.animal.wild = bool_arg(c, 1));
    a_get!("isOnHook", |_c, f| Value::Bool(f.animal.on_hook));
    a_set!("setOnHook", |c, _chr, f| f.animal.on_hook = bool_arg(c, 1));
    a_get!("isRoadKill", |_c, f| Value::Bool(f.animal.road_kill));
    a_set!("setIsRoadKill", |c, _chr, f| f.animal.road_kill = bool_arg(c, 1));
    a_get!("isAlerted", |_c, f| Value::Bool(f.alerted_chr.is_some()));
    a_set!("setIsAlerted", |c, _chr, f| {
        if !bool_arg(c, 1) {
            f.alerted_chr = None;
        }
    });
    a_set!("setShouldBeSkeleton", |c, _chr, f| {
        f.animal.should_be_skeleton = bool_arg(c, 1)
    });
    a_get!("isMoveForwardOnZone", |_c, f| Value::Bool(f.animal.move_forward_on_zone));
    a_set!("setMoveForwardOnZone", |c, _chr, f| {
        f.animal.move_forward_on_zone = bool_arg(c, 1)
    });
    a_get!("hasAnimalZone", |_c, f| Value::Bool(f.animal.animal_zone.is_some()));
    a_get!("getNestBoxIndex", |_c, f| Value::Num(f.animal.nest_box as f64));
    a_get!("getAge", |_c, f| Value::Num(f.animal.age() as f64));
    a_set!("setAgeDebug", |c, _chr, f| f.animal.data.age = int_arg(c, 1));
    a_get!("getAnimalSize", |_c, f| Value::Num(f.animal.data.size as f64));
    a_get!("getAnimalOriginalSize", |_c, f| Value::Num(f.animal.data.original_size as f64));
    a_get!("getStress", |_c, f| Value::Num(f.animal.stress_level as f64));
    a_set!("changeStress", |c, _chr, f| {
        f.animal.change_stress(num_arg(c, 1) as f32)
    });
    a_set!("setDebugStress", |c, _chr, f| {
        f.animal.set_debug_stress(num_arg(c, 1) as f32)
    });
    a_get!("getPetTimer", |_c, f| Value::Num(f.animal.pet_timer as f64));
    a_get!("petTimerDone", |_c, f| Value::Bool(f.animal.pet_timer_done()));
    a_get!("isBaby", |_c, f| Value::Bool(f.animal.is_baby().unwrap_or(false)));
    a_get!("haveEnoughMilkToFeedFrom", |_c, f| {
        Value::Bool(f.animal.have_enough_milk_to_feed_from())
    });
    a_get!("getMeatRatio", |_c, f| Value::Num(f.animal.meat_ratio() as f64));
    a_get!("getEggGeneMod", |_c, f| Value::Num(f.animal.egg_gene_mod() as f64));
    a_get!("getCurrentClutchSize", |_c, f| Value::Num(f.animal.data.clutch_size as f64));
    a_get!("getZoneAcceptance", |_c, f| Value::Num(f.animal.zone_acceptance as f64));
    a_get!("isAnimalMoving", |c, _f| Value::Bool(c.animal_is_moving()));
    a_get!("isAnimalEating", |c, _f| Value::Bool(c.animal_is_eating()));
    a_get!("isAnimalSitting", |c, _f| Value::Bool(c.animal_is_sitting()));
    a_get!("getHunger", |c, f| Value::Num(c.stats.get(&f.hunger) as f64));
    a_get!("getThirst", |c, f| Value::Num(c.stats.get(&f.thirst) as f64));

    b.method("getAcceptanceLevel", |c| {
        let chr_ref = animal_char(c)?;
        let pid = player_online_id(&c.arg(1));
        let mut rng = SimpleRand(0x6163_6365_7074_0001);
        let mut chr = chr_ref.borrow_mut();
        let v = chr
            .animal_fields_mut()
            .map(|f| f.animal.acceptance_level(pid, &mut rng))
            .unwrap_or(0.0);
        Ok(c.push(Value::Num(v as f64)))
    });
    b.method("getPlayerAcceptance", |c| {
        let chr_ref = animal_char(c)?;
        let pid = player_online_id(&c.arg(1));
        let mut rng = SimpleRand(0x6163_6365_7074_0002);
        let mut chr = chr_ref.borrow_mut();
        let v = chr
            .animal_fields_mut()
            .map(|f| f.animal.player_acceptance_of(pid, &mut rng))
            .unwrap_or(0.0);
        Ok(c.push(Value::Num(v as f64)))
    });
    b.method("addAcceptance", |c| {
        let chr_ref = animal_char(c)?;
        let pid = player_online_id(&c.arg(1));
        let amount = num_arg(c, 2) as f32;
        let mut rng = SimpleRand(0x6163_6365_7074_0003);
        if let Some(f) = chr_ref.borrow_mut().animal_fields_mut() {
            f.animal.add_acceptance(pid, 0, amount, &mut rng);
        }
        Ok(0)
    });
    b.method("setDebugAcceptance", |c| {
        let chr_ref = animal_char(c)?;
        let pid = player_online_id(&c.arg(1));
        let val = num_arg(c, 2) as f32;
        if let Some(f) = chr_ref.borrow_mut().animal_fields_mut() {
            f.animal.set_debug_acceptance(pid, val);
        }
        Ok(0)
    });

    b.method("getFullGenome", |c| {
        let chr_ref = animal_char(c)?;
        let chr = chr_ref.borrow();
        let map = JavaMap::hash_map();
        if let Some(f) = chr.animal_fields() {
            for (k, g) in f.animal.full_genome.iter() {
                map.put(Value::str(k.as_str()), gene_value(g.clone()));
            }
        }
        Ok(c.push(Value::Object(Rc::new(map))))
    });
    b.method("getFullGenomeList", |c| {
        let chr_ref = animal_char(c)?;
        let chr = chr_ref.borrow();
        let items: Vec<Value> = chr
            .animal_fields()
            .map(|f| {
                f.animal
                    .full_genome_list()
                    .into_iter()
                    .cloned()
                    .map(gene_value)
                    .collect()
            })
            .unwrap_or_default();
        Ok(c.push(list_value(items)))
    });
    b.method("getUsedGene", |c| {
        let chr_ref = animal_char(c)?;
        let chr = chr_ref.borrow();
        let out = str_arg(c, 1)
            .and_then(|name| {
                chr.animal_fields()
                    .and_then(|f| f.animal.used_gene(&name).cloned())
            })
            .map(allele_value)
            .unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
    b.method("hasGeneticDisorder", |c| {
        let chr_ref = animal_char(c)?;
        let chr = chr_ref.borrow();
        let has = str_arg(c, 1).is_some_and(|d| {
            chr.animal_fields()
                .is_some_and(|f| f.animal.has_genetic_disorder(&d))
        });
        Ok(c.push(Value::Bool(has)))
    });
    b.method("getGeneticDisorder", |c| {
        let chr_ref = animal_char(c)?;
        let chr = chr_ref.borrow();
        let items: Vec<Value> = chr
            .animal_fields()
            .map(|f| {
                f.animal
                    .genetic_disorder
                    .iter()
                    .map(|s| Value::str(s.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        Ok(c.push(list_value(items)))
    });

    b.method("addToWorld", |c| {
        let chr_ref = animal_char(c)?;
        if let Some(f) = chr_ref.borrow_mut().animal_fields_mut() {
            f.exists_in_world = true;
        }
        Ok(0)
    });
    b.method("removeFromWorld", |c| {
        let chr_ref = animal_char(c)?;
        if let Some(f) = chr_ref.borrow_mut().animal_fields_mut() {
            f.exists_in_world = false;
        }
        Ok(0)
    });
    b.method("remove", |c| {
        let chr_ref = animal_char(c)?;
        if let Some(f) = chr_ref.borrow_mut().animal_fields_mut() {
            f.exists_in_world = false;
        }
        Ok(0)
    });
    b.method("setHealth", |c| {
        let chr_ref = animal_char(c)?;
        chr_ref.borrow_mut().health = num_arg(c, 1) as f32;
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn iso_animal_data_and_genome_round_trip_from_lua() {
        {
            let mut bt = pz_scripts::registry::builtin::BuiltinTables::default();
            for f in ["HUNGER", "THIRST"] {
                bt.character_stats.push(pz_scripts::registry::builtin::CharacterStatDecl {
                    field: f.to_string(),
                    id: f.to_lowercase(),
                    minimum: 0.0,
                    maximum: 1.0,
                    default: 0.0,
                });
                bt.ordered_stats.push(f.to_string());
            }
            shared_characters().borrow_mut().sim =
                pz_characters::SimStatics::from_tables(&bt).unwrap();
        }
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_character_sim::register_shared(&mut t);
        register(&mut t);

        let src = r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            local a = IsoAnimal.new(nil, 10, 20, 0, "cow", "angus")
            assert(a ~= nil, "IsoAnimal constructed")
            assert(a:getObjectName() == "Animal")
            assert(a:getAnimalType() == "cow")
            assert(a:getTypeAndBreed() == "cowangus")
            a:setAnimalID(777)
            assert(a:getAnimalID() == 777)
            a:setCustomName("Bessie")
            assert(a:getCustomName() == "Bessie")
            assert(a:getFullName() == "Bessie")

            a:addToWorld()
            assert(a:isExistInTheWorld() == true)
            a:setDebugStress(25)
            a:changeStress(10)
            assert(a:getStress() == 35)
            a:setDebugAcceptance(nil, 42)
            assert(a:getPlayerAcceptance(nil) == 42)

            local data = a:getData()
            assert(data ~= nil, "AnimalData present")
            data:setAge(45)
            assert(data:getAge() == 45 and a:getAge() == 45)
            data:setMilkQuantity(3.5)
            assert(data:getMilkQuantity() == 3.5)
            data:setWoolQuantity(1.25)
            assert(data:getWoolQuantity() == 1.25)
            data:setPregnant(true)
            data:setPregnancyTime(12)
            assert(data:isPregnant() == true and data:getPregnancyTime() == 12)
            data:setWeight(180)
            assert(data:getWeight() == 180)

            local al = AnimalAllele.new()
            al:setCurrentValue(0.75)
            al:setDominant(true)
            al:setUsed(true)
            al:setGeneticDisorder("dwarf")
            assert(al:getCurrentValue() == 0.75)
            assert(al:isDominant() == true)
            assert(al:isUsed() == true)
            assert(al:getGeneticDisorder() == "dwarf")

            local g = AnimalGene.new()
            g:initUsedGene()
            assert(g:getUsedGene() ~= nil)
            assert(a:getFullGenome():size() == 0)
            assert(a:getGeneticDisorder():size() == 0)

            a:removeFromWorld()
            assert(a:isExistInTheWorld() == false)
            return true
        "#;
        let cl = pz_kahlua::stdlib::load_string(src, Some("bind_iso_animal_test"), t.globals().clone())
            .unwrap();
        let res = t
            .pcall(&cl, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        assert!(matches!(res.first(), Some(Value::Bool(true))));
    }
}
