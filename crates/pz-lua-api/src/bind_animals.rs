//! Lua bindings for Build 42.22 animal definition classes:
//! - `zombie.characters.animals.AnimalDefinitions`
//! - `zombie.characters.animals.datas.AnimalBreed`
//! - `zombie.characters.animals.datas.AnimalBreed$Sound`
//! - `zombie.characters.animals.datas.AnimalGrowStage`
//! - `zombie.characters.animals.AnimalGenomeDefinitions`
//! - `zombie.characters.animals.AnimalTracksDefinitions`
//! - `zombie.characters.animals.AnimalTracksDefinitions$AnimalTracksType`
//!
//! Vanilla `media/lua/shared/Definitions/animal/*Definitions.lua` scripts
//! populate `AnimalDefinitions.animals`, `AnimalGenomeDefinitions.genes`, and
//! `AnimalTracksDefinitions.animallist` on the global tables using
//! `X = X or {}`. Because [`register`] exposes these classes before Lua loads,
//! the static methods live on those same tables and lazily parse
//! [`AnimalDefinitionTables::load`] on first query (matching Java).

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_characters::pz_animals::{
    AnimalBreed, AnimalDefinitionTables, AnimalDefinitions, AnimalGrowStage,
    AnimalTracksDefinitions, AnimalTracksType, BreedSound,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};

use crate::bind_character_sim::CharacterObject;
use crate::bind_java_util::{list_value, JavaMap};
use crate::expose::{expose_class, str_arg, this};

pub const ANIMAL_DEFINITIONS_CLASS: &str = "zombie.characters.animals.AnimalDefinitions";
pub const ANIMAL_BREED_CLASS: &str = "zombie.characters.animals.datas.AnimalBreed";
pub const ANIMAL_BREED_SOUND_CLASS: &str = "zombie.characters.animals.datas.AnimalBreed$Sound";
pub const ANIMAL_GROW_STAGE_CLASS: &str = "zombie.characters.animals.datas.AnimalGrowStage";
pub const ANIMAL_GENOME_DEFINITIONS_CLASS: &str =
    "zombie.characters.animals.AnimalGenomeDefinitions";
pub const ANIMAL_TRACKS_DEFINITIONS_CLASS: &str =
    "zombie.characters.animals.AnimalTracksDefinitions";
pub const ANIMAL_TRACKS_TYPE_CLASS: &str =
    "zombie.characters.animals.AnimalTracksDefinitions$AnimalTracksType";

thread_local! {
    static TABLES: RefCell<Option<Rc<AnimalDefinitionTables>>> = const { RefCell::new(None) };
    static RNG_STATE: RefCell<u64> = const { RefCell::new(0x5eed_cafe_1234_5678) };
}

/// Replaces the cached animal definition tables (for example when `IsoWorld`
/// loads them during world init).
pub fn install_tables(tables: Option<Rc<AnimalDefinitionTables>>) {
    TABLES.with(|slot| *slot.borrow_mut() = tables);
}

/// Returns the cached animal definition tables, if already loaded.
pub fn cached_tables() -> Option<Rc<AnimalDefinitionTables>> {
    TABLES.with(|slot| slot.borrow().clone())
}

/// Clears cached animal definitions (`AnimalDefinitions.Reset()`).
pub fn reset() {
    TABLES.with(|slot| *slot.borrow_mut() = None);
}

fn has_subtable(env: &TableRef, global: &str, key: &str) -> bool {
    env.get_str(global)
        .as_table()
        .is_some_and(|t| t.rawget(&Value::str(key)).as_table().is_some())
}

fn load_tables_lenient(env: &TableRef) -> LuaResult<AnimalDefinitionTables> {
    let genome = if has_subtable(env, "AnimalGenomeDefinitions", "genes") {
        pz_characters::pz_animals::AnimalGenomeDefinitions::load(env)
            .map_err(|e| LuaError::runtime(format!("AnimalGenomeDefinitions load failed: {e}")))?
    } else {
        Default::default()
    };
    let animal_defs = if has_subtable(env, "AnimalDefinitions", "animals") {
        AnimalDefinitions::load_all(env)
            .map_err(|e| LuaError::runtime(format!("AnimalDefinitions load failed: {e}")))?
    } else {
        Default::default()
    };
    let tracks = if has_subtable(env, "AnimalTracksDefinitions", "animallist") {
        AnimalTracksDefinitions::load_all(env)
            .map_err(|e| LuaError::runtime(format!("AnimalTracksDefinitions load failed: {e}")))?
    } else {
        Default::default()
    };
    let migration = if env.get_str("MigrationGroupDefinitions").as_table().is_some() {
        pz_characters::pz_animals::MigrationGroupDefinitions::load_all(env)
            .map_err(|e| LuaError::runtime(format!("MigrationGroupDefinitions load failed: {e}")))?
    } else {
        Default::default()
    };
    Ok(AnimalDefinitionTables {
        genome,
        animal_defs,
        tracks,
        migration,
    })
}

fn reload_from_env(env: &TableRef) -> LuaResult<Rc<AnimalDefinitionTables>> {
    let tables = load_tables_lenient(env)?;
    let rc = Rc::new(tables);
    TABLES.with(|slot| *slot.borrow_mut() = Some(rc.clone()));
    Ok(rc)
}

fn ensure_loaded(env: &TableRef) -> LuaResult<Rc<AnimalDefinitionTables>> {
    if let Some(existing) = TABLES.with(|slot| slot.borrow().clone()) {
        return Ok(existing);
    }
    reload_from_env(env)
}

struct LuaRand;

impl pz_world::GameRand for LuaRand {
    fn next(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let n = RNG_STATE.with(|s| {
            let mut st = s.borrow_mut();
            let mut x = *st;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *st = x;
            x
        });
        (n % (bound as u64)) as i32
    }

    fn next_float(&mut self) -> f32 {
        (self.next(1_000_000) as f32) / 1_000_000.0
    }
}

fn opt_str_val(s: &Option<String>) -> Value {
    match s {
        Some(v) => Value::str(v.as_str()),
        None => Value::Nil,
    }
}

fn str_list_val(v: &Option<Vec<String>>) -> Value {
    match v {
        Some(items) => list_value(items.iter().map(|s| Value::str(s.as_str())).collect()),
        None => Value::Nil,
    }
}

// ---- userdata wrappers ------------------------------------------------------

#[derive(Clone)]
pub struct LuaAnimalDefinitions(pub Rc<AnimalDefinitions>);

impl LuaObject for LuaAnimalDefinitions {
    fn class_name(&self) -> &str {
        ANIMAL_DEFINITIONS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
pub struct LuaAnimalBreed(pub Rc<RefCell<AnimalBreed>>);

impl LuaObject for LuaAnimalBreed {
    fn class_name(&self) -> &str {
        ANIMAL_BREED_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
pub struct LuaBreedSound(pub Rc<BreedSound>);

impl LuaObject for LuaBreedSound {
    fn class_name(&self) -> &str {
        ANIMAL_BREED_SOUND_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
pub struct LuaAnimalGrowStage(pub Rc<AnimalGrowStage>);

impl LuaObject for LuaAnimalGrowStage {
    fn class_name(&self) -> &str {
        ANIMAL_GROW_STAGE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
pub struct LuaAnimalTracksDefinitions(pub Rc<AnimalTracksDefinitions>);

impl LuaObject for LuaAnimalTracksDefinitions {
    fn class_name(&self) -> &str {
        ANIMAL_TRACKS_DEFINITIONS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
pub struct LuaAnimalTracksType(pub Rc<AnimalTracksType>);

impl LuaObject for LuaAnimalTracksType {
    fn class_name(&self) -> &str {
        ANIMAL_TRACKS_TYPE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn def_value(def: AnimalDefinitions) -> Value {
    Value::Object(Rc::new(LuaAnimalDefinitions(Rc::new(def))))
}

fn breed_value(breed: AnimalBreed) -> Value {
    Value::Object(Rc::new(LuaAnimalBreed(Rc::new(RefCell::new(breed)))))
}

fn sound_value(sound: BreedSound) -> Value {
    Value::Object(Rc::new(LuaBreedSound(Rc::new(sound))))
}

fn stage_value(stage: AnimalGrowStage) -> Value {
    Value::Object(Rc::new(LuaAnimalGrowStage(Rc::new(stage))))
}

fn tracks_def_value(def: AnimalTracksDefinitions) -> Value {
    Value::Object(Rc::new(LuaAnimalTracksDefinitions(Rc::new(def))))
}

fn tracks_type_value(t: AnimalTracksType) -> Value {
    Value::Object(Rc::new(LuaAnimalTracksType(Rc::new(t))))
}

fn current_game_hour() -> i32 {
    let gt = crate::bind_climate::shared_game_time();
    let hour = gt.borrow().time_of_day.floor() as i32;
    hour.clamp(0, 23)
}

fn animal_type_from_arg(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<CharacterObject>()
            .and_then(|co| {
                co.0.try_borrow()
                    .ok()
                    .and_then(|c| c.animal.as_ref().and_then(|a| a.animal.animal_type.clone()))
            }),
        _ => None,
    }
}

fn age_to_grow_gene_from_arg(v: &Value) -> Option<f32> {
    let Value::Object(o) = v else { return None };
    let co = o.as_any().downcast_ref::<CharacterObject>()?;
    let c = co.0.try_borrow().ok()?;
    let a = c.animal.as_ref()?;
    let gene = a.animal.full_genome.get("agetogrow")?;
    Some(gene.used_gene().current_value)
}

/// Registers Build 42.22 animal definition classes on `thread`.
pub fn register(thread: &mut Thread) {
    register_animal_definitions(thread);
    register_animal_breed(thread);
    register_breed_sound(thread);
    register_grow_stage(thread);
    register_genome_definitions(thread);
    register_tracks_definitions(thread);
    register_tracks_type(thread);
}

/// Restores `meta.__index` to its method table before a repeat `expose_class`
/// call (e.g. when `register_all` runs more than once on the same thread).
fn prepare_field_class(thread: &Thread, fqn: &str) {
    if let Value::Table(meta) = thread.class_metatables.get_str(fqn) {
        if let Value::Table(methods) = meta.get_str("__pz_methods") {
            meta.set_str("__index", methods);
        }
    }
}

fn install_field_index(
    thread: &Thread,
    fqn: &str,
    methods: TableRef,
    lookup: impl Fn(&mut NativeCall<'_>, &str) -> LuaResult<Value> + 'static,
) {
    if let Value::Table(meta) = thread.class_metatables.get_str(fqn) {
        meta.set_str("__pz_methods", methods.clone());
        meta.set_str(
            "__index",
            Value::Native(pz_kahlua::NativeFunction::new(
                "__index",
                move |c: &mut NativeCall<'_>| {
                    let key = c.arg(1);
                    let found = methods.rawget(&key);
                    if !found.is_nil() {
                        return Ok(c.push(found));
                    }
                    let Some(name) = key.as_str() else {
                        return Ok(c.push(Value::Nil));
                    };
                    let val = lookup(c, name)?;
                    Ok(c.push(val))
                },
            )),
        );
    }
}

fn register_animal_definitions(thread: &mut Thread) {
    prepare_field_class(thread, ANIMAL_DEFINITIONS_CLASS);
    let b = expose_class(thread, ANIMAL_DEFINITIONS_CLASS, None);
    b.ctor(|c| Ok(c.push(def_value(AnimalDefinitions::default()))));
    b.static_fn("loadAnimalDefinitions", |c| {
        let env = c.thread.globals().clone();
        reload_from_env(&env)?;
        Ok(0)
    });
    b.static_fn("Reset", |_c| {
        reset();
        Ok(0)
    });
    b.static_fn("getDef", |c| {
        let Some(ty) = animal_type_from_arg(&c.arg(0)) else {
            return Ok(c.push(Value::Nil));
        };
        let env = c.thread.globals().clone();
        let tables = ensure_loaded(&env)?;
        let out = tables.def(&ty).cloned().map(def_value).unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
    b.static_fn("getAnimalDefs", |c| {
        let env = c.thread.globals().clone();
        let tables = ensure_loaded(&env)?;
        let map = JavaMap::hash_map();
        for (k, v) in tables.animal_defs.iter() {
            map.put(Value::str(k.as_str()), def_value(v.clone()));
        }
        Ok(c.push(Value::Object(Rc::new(map))))
    });
    b.static_fn("getAnimalDefsArray", |c| {
        let env = c.thread.globals().clone();
        let tables = ensure_loaded(&env)?;
        let items: Vec<Value> = tables
            .animal_defs
            .iter()
            .map(|(_, v)| def_value(v.clone()))
            .collect();
        Ok(c.push(list_value(items)))
    });

    b.method("getAnimalType", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(c.push(opt_str_val(&d.animal_type_str)))
    });
    b.method("getBodyModelStr", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(c.push(opt_str_val(&d.body_model_str)))
    });
    b.method("getGroup", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(c.push(opt_str_val(&d.group)))
    });
    b.method("getBabyType", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(c.push(opt_str_val(&d.baby_type)))
    });
    b.method("getMinBaby", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(c.push(Value::Num(d.min_baby as f64)))
    });
    b.method("getMaxBaby", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(c.push(Value::Num(d.max_baby as f64)))
    });
    b.method("getWildFleeTimeUntilDeadTimer", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(c.push(Value::Num(d.wild_flee_time_until_dead_timer as f64)))
    });
    b.method("canBeSkeleton", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(c.push(Value::Bool(d.can_be_skeleton(d.body_model_skel_str.is_some()))))
    });
    b.method("isBaby", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(c.push(Value::Bool(d.is_baby())))
    });
    b.method("isInsideHutchTime", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        let hour = match c.arg(1) {
            Value::Num(n) if (n as i32) >= 0 => n as i32,
            _ => current_game_hour(),
        };
        Ok(c.push(Value::Bool(d.is_inside_hutch_time(hour))))
    });
    b.method("isOutsideHutchTime", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(c.push(Value::Bool(!d.is_inside_hutch_time(current_game_hour()))))
    });
    b.method("getBreeds", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        let items: Vec<Value> = d.breeds.iter().cloned().map(breed_value).collect();
        Ok(c.push(list_value(items)))
    });
    b.method("getBreedByName", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        let out = str_arg(c, 1)
            .and_then(|name| d.breed_by_name(&name).cloned())
            .map(breed_value)
            .unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
    b.method("getRandomBreed", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        let mut rng = LuaRand;
        let out = d.random_breed(&mut rng).cloned().map(breed_value).unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
    b.method("getGrowStage", |c| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        let out = d.grow_stage().cloned().map(stage_value).unwrap_or(Value::Nil);
        Ok(c.push(out))
    });

    // Public Java field access via __index fallback after method lookup.
    let methods = b.methods.clone();
    install_field_index(thread, ANIMAL_DEFINITIONS_CLASS, methods, |c, name| {
        let o = this::<LuaAnimalDefinitions>(c)?;
        let d = &o.as_any().downcast_ref::<LuaAnimalDefinitions>().unwrap().0;
        Ok(animal_def_field(d, name))
    });
}

fn animal_def_field(d: &AnimalDefinitions, name: &str) -> Value {
    match name {
        "animalTypeStr" => opt_str_val(&d.animal_type_str),
        "bodyModelStr" => opt_str_val(&d.body_model_str),
        "bodyModelSkelStr" => opt_str_val(&d.body_model_skel_str),
        "bodyModelFleeceStr" => opt_str_val(&d.body_model_fleece_str),
        "bodyModelHeadlessStr" => opt_str_val(&d.body_model_headless_str),
        "bodyModelSkelNoHeadStr" => opt_str_val(&d.body_model_skel_no_head_str),
        "textureSkeleton" => opt_str_val(&d.texture_skeleton),
        "textureSkeletonBloody" => opt_str_val(&d.texture_skeleton_bloody),
        "textureRotten" => opt_str_val(&d.texture_rotten),
        "textureSkinned" => opt_str_val(&d.texture_skinned),
        "animset" => opt_str_val(&d.animset),
        "mate" => opt_str_val(&d.mate),
        "babyType" => opt_str_val(&d.baby_type),
        "eggType" => opt_str_val(&d.egg_type),
        "carcassItem" => opt_str_val(&d.carcass_item),
        "milkAnimPreset" => opt_str_val(&d.milk_anim_preset),
        "group" => opt_str_val(&d.group),
        "dung" => opt_str_val(&d.dung),
        "ropeBone" => opt_str_val(&d.rope_bone),
        "feedByHandAnim" => opt_str_val(&d.feed_by_hand_anim),
        "shadoww" => Value::Num(d.shadoww as f64),
        "shadowfm" => Value::Num(d.shadowfm as f64),
        "shadowbm" => Value::Num(d.shadowbm as f64),
        "turnDelta" => Value::Num(d.turn_delta as f64),
        "animalSize" => Value::Num(d.animal_size as f64),
        "minSize" => Value::Num(d.min_size as f64),
        "maxSize" => Value::Num(d.max_size as f64),
        "minAge" => Value::Num(d.min_age as f64),
        "minEnclosureSize" => Value::Num(d.min_enclosure_size as f64),
        "minAgeForBaby" => Value::Num(d.min_age_for_baby as f64),
        "maxAgeGeriatric" => Value::Num(d.max_age_geriatric as f64),
        "minBaby" => Value::Num(d.min_baby as f64),
        "maxBaby" => Value::Num(d.max_baby as f64),
        "pregnantPeriod" => Value::Num(d.pregnant_period as f64),
        "eggsPerDay" => Value::Num(d.eggs_per_day as f64),
        "fertilizedTimeMax" => Value::Num(d.fertilized_time_max as f64),
        "timeToHatch" => Value::Num(d.time_to_hatch as f64),
        "enterHutchTime" => Value::Num(d.enter_hutch_time as f64),
        "exitHutchTime" => Value::Num(d.exit_hutch_time as f64),
        "minMilk" => Value::Num(d.min_milk as f64),
        "maxMilk" => Value::Num(d.max_milk as f64),
        "maxWool" => Value::Num(d.max_wool as f64),
        "minWeight" => Value::Num(d.min_weight as f64),
        "maxWeight" => Value::Num(d.max_weight as f64),
        "baseDmg" => Value::Num(d.base_dmg as f64),
        "trailerBaseSize" => Value::Num(d.trailer_base_size as f64),
        "collisionSize" => Value::Num(d.collision_size as f64),
        "baseEncumbrance" => Value::Num(d.base_encumbrance as f64),
        "matingPeriodStart" => Value::Num(d.mating_period_start as f64),
        "matingPeriodEnd" => Value::Num(d.mating_period_end as f64),
        "spottingDist" => Value::Num(d.spotting_dist as f64),
        "udder" => Value::Bool(d.udder),
        "female" => Value::Bool(d.female),
        "male" => Value::Bool(d.male),
        "wild" => Value::Bool(d.wild),
        "canBeMilked" => Value::Bool(d.can_be_milked),
        "canBePicked" => Value::Bool(d.can_be_picked),
        "canBePet" => Value::Bool(d.can_be_pet),
        "canBeAttached" => Value::Bool(d.can_be_attached),
        "canBeTransported" => Value::Bool(d.can_be_transported),
        "canBeFeedByHand" => Value::Bool(d.can_be_feed_by_hand),
        "canBeDomesticated" => Value::Bool(d.can_be_domesticated),
        "canClimbFences" => Value::Bool(d.can_climb_fences),
        "canClimbStairs" => Value::Bool(d.can_climb_stairs),
        "eatGrass" => Value::Bool(d.eat_grass),
        "eatFromMother" => Value::Bool(d.eat_from_mother),
        "alwaysFleeHumans" => Value::Bool(d.always_flee_humans),
        "fleeZombies" => Value::Bool(d.flee_zombies),
        "attackBack" => Value::Bool(d.attack_back),
        "collidable" => Value::Bool(d.collidable),
        "canThump" => Value::Bool(d.can_thump),
        "needMom" => Value::Bool(d.need_mom),
        "breeds" => list_value(d.breeds.iter().cloned().map(breed_value).collect()),
        "stages" => list_value(d.stages.iter().cloned().map(stage_value).collect()),
        "hutches" => str_list_val(&d.hutches),
        "genes" => str_list_val(&d.genes),
        "eatTypeTrough" => str_list_val(&d.eat_type_trough),
        "feedByHandType" => str_list_val(&d.feed_by_hand_type),
        _ => Value::Nil,
    }
}

fn register_animal_breed(thread: &mut Thread) {
    prepare_field_class(thread, ANIMAL_BREED_CLASS);
    let b = expose_class(thread, ANIMAL_BREED_CLASS, None);
    b.ctor(|c| Ok(c.push(breed_value(AnimalBreed::default()))));
    b.method("getName", |c| {
        let o = this::<LuaAnimalBreed>(c)?;
        let br = o.as_any().downcast_ref::<LuaAnimalBreed>().unwrap().0.borrow();
        Ok(c.push(opt_str_val(&br.name)))
    });
    b.method("getMilkType", |c| {
        let o = this::<LuaAnimalBreed>(c)?;
        let br = o.as_any().downcast_ref::<LuaAnimalBreed>().unwrap().0.borrow();
        Ok(c.push(opt_str_val(&br.milk_type)))
    });
    b.method("getWoolType", |c| {
        let o = this::<LuaAnimalBreed>(c)?;
        let br = o.as_any().downcast_ref::<LuaAnimalBreed>().unwrap().0.borrow();
        Ok(c.push(opt_str_val(&br.wool_type)))
    });
    b.method("getFeatherItem", |c| {
        let o = this::<LuaAnimalBreed>(c)?;
        let br = o.as_any().downcast_ref::<LuaAnimalBreed>().unwrap().0.borrow();
        Ok(c.push(opt_str_val(&br.feather_item)))
    });
    b.method("getRottenTexture", |c| {
        let o = this::<LuaAnimalBreed>(c)?;
        let br = o.as_any().downcast_ref::<LuaAnimalBreed>().unwrap().0.borrow();
        Ok(c.push(opt_str_val(&br.rotten_texture)))
    });
    b.method("getSound", |c| {
        let o = this::<LuaAnimalBreed>(c)?;
        let br = o.as_any().downcast_ref::<LuaAnimalBreed>().unwrap().0.borrow();
        let out = str_arg(c, 1)
            .and_then(|id| br.sound(&id).cloned())
            .map(sound_value)
            .unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
    b.method("isSoundDefined", |c| {
        let o = this::<LuaAnimalBreed>(c)?;
        let br = o.as_any().downcast_ref::<LuaAnimalBreed>().unwrap().0.borrow();
        let defined = str_arg(c, 1).is_some_and(|id| br.is_sound_defined(&id));
        Ok(c.push(Value::Bool(defined)))
    });
    b.method("isSoundUndefined", |c| {
        let o = this::<LuaAnimalBreed>(c)?;
        let br = o.as_any().downcast_ref::<LuaAnimalBreed>().unwrap().0.borrow();
        let undefined = str_arg(c, 1).is_none_or(|id| !br.is_sound_defined(&id));
        Ok(c.push(Value::Bool(undefined)))
    });
    b.method("loadForcedGenes", |c| {
        let o = this::<LuaAnimalBreed>(c)?;
        if let Value::Table(t) = c.arg(1) {
            o.as_any()
                .downcast_ref::<LuaAnimalBreed>()
                .unwrap()
                .0
                .borrow_mut()
                .load_forced_genes(&t)
                .map_err(|e| LuaError::runtime(format!("{e}")))?;
        }
        Ok(0)
    });
    b.method("loadSounds", |c| {
        let o = this::<LuaAnimalBreed>(c)?;
        if let Value::Table(t) = c.arg(1) {
            o.as_any()
                .downcast_ref::<LuaAnimalBreed>()
                .unwrap()
                .0
                .borrow_mut()
                .load_sounds(&t)
                .map_err(|e| LuaError::runtime(format!("{e}")))?;
        }
        Ok(0)
    });

    let methods = b.methods.clone();
    install_field_index(thread, ANIMAL_BREED_CLASS, methods, |c, name| {
        let o = this::<LuaAnimalBreed>(c)?;
        let br = o.as_any().downcast_ref::<LuaAnimalBreed>().unwrap().0.borrow();
        let val = match name {
            "name" => opt_str_val(&br.name),
            "texture" => str_list_val(&br.texture),
            "textureMale" => opt_str_val(&br.texture_male),
            "textureBaby" => opt_str_val(&br.texture_baby),
            "minWeightBonus" => Value::Num(br.min_weight_bonus as f64),
            "maxWeightBonus" => Value::Num(br.max_weight_bonus as f64),
            "milkType" => opt_str_val(&br.milk_type),
            "woolType" => opt_str_val(&br.wool_type),
            "invIconMale" => opt_str_val(&br.inv_icon_male),
            "invIconFemale" => opt_str_val(&br.inv_icon_female),
            "invIconBaby" => opt_str_val(&br.inv_icon_baby),
            "invIconMaleDead" => opt_str_val(&br.inv_icon_male_dead),
            "invIconFemaleDead" => opt_str_val(&br.inv_icon_female_dead),
            "invIconBabyDead" => opt_str_val(&br.inv_icon_baby_dead),
            "invIconMaleSkel" => opt_str_val(&br.inv_icon_male_skel),
            "invIconFemaleSkel" => opt_str_val(&br.inv_icon_female_skel),
            "invIconBabySkel" => opt_str_val(&br.inv_icon_baby_skel),
            "leather" => opt_str_val(&br.leather),
            "headItem" => opt_str_val(&br.head_item),
            "featherItem" => opt_str_val(&br.feather_item),
            "maxFeather" => Value::Num(br.max_feather as f64),
            "rottenTexture" => opt_str_val(&br.rotten_texture),
            _ => Value::Nil,
        };
        Ok(val)
    });
}

fn register_breed_sound(thread: &mut Thread) {
    prepare_field_class(thread, ANIMAL_BREED_SOUND_CLASS);
    let b = expose_class(thread, ANIMAL_BREED_SOUND_CLASS, None);
    b.ctor(|c| Ok(c.push(sound_value(BreedSound::default()))));
    b.method("isIntervalValid", |c| {
        let o = this::<LuaBreedSound>(c)?;
        let s = &o.as_any().downcast_ref::<LuaBreedSound>().unwrap().0;
        Ok(c.push(Value::Bool(s.is_interval_valid())))
    });
    let methods = b.methods.clone();
    install_field_index(thread, ANIMAL_BREED_SOUND_CLASS, methods, |c, name| {
        let o = this::<LuaBreedSound>(c)?;
        let s = &o.as_any().downcast_ref::<LuaBreedSound>().unwrap().0;
        let val = match name {
            "id" => opt_str_val(&s.id),
            "soundName" => opt_str_val(&s.sound_name),
            "intervalMin" => Value::Num(s.interval_min as f64),
            "intervalMax" => Value::Num(s.interval_max as f64),
            "slot" => opt_str_val(&s.slot),
            "priority" => Value::Num(s.priority as f64),
            _ => Value::Nil,
        };
        Ok(val)
    });
}

fn register_grow_stage(thread: &mut Thread) {
    prepare_field_class(thread, ANIMAL_GROW_STAGE_CLASS);
    let b = expose_class(thread, ANIMAL_GROW_STAGE_CLASS, None);
    b.ctor(|c| Ok(c.push(stage_value(AnimalGrowStage::default()))));
    b.method("getAgeToGrow", |c| {
        let o = this::<LuaAnimalGrowStage>(c)?;
        let st = &o.as_any().downcast_ref::<LuaAnimalGrowStage>().unwrap().0;
        let gene = age_to_grow_gene_from_arg(&c.arg(1));
        Ok(c.push(Value::Num(st.age_to_grow_for(gene) as f64)))
    });
    let methods = b.methods.clone();
    install_field_index(thread, ANIMAL_GROW_STAGE_CLASS, methods, |c, name| {
        let o = this::<LuaAnimalGrowStage>(c)?;
        let st = &o.as_any().downcast_ref::<LuaAnimalGrowStage>().unwrap().0;
        let val = match name {
            "ageToGrow" => Value::Num(st.age_to_grow as f64),
            "nextStage" => opt_str_val(&st.next_stage),
            "nextStageMale" => opt_str_val(&st.next_stage_male),
            "stage" => opt_str_val(&st.stage),
            _ => Value::Nil,
        };
        Ok(val)
    });
}

fn register_genome_definitions(thread: &mut Thread) {
    let b = expose_class(thread, ANIMAL_GENOME_DEFINITIONS_CLASS, None);
    b.static_fn("loadGenomeDefinition", |c| {
        let env = c.thread.globals().clone();
        reload_from_env(&env)?;
        Ok(0)
    });
    b.static_fn("getGeneticDisorderList", |c| {
        let env = c.thread.globals().clone();
        let tables = ensure_loaded(&env)?;
        let items: Vec<Value> = tables
            .genome
            .genetic_disorder
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|s| Value::str(s.as_str()))
            .collect();
        Ok(c.push(list_value(items)))
    });
}

fn register_tracks_definitions(thread: &mut Thread) {
    let b = expose_class(thread, ANIMAL_TRACKS_DEFINITIONS_CLASS, None);
    b.ctor(|c| Ok(c.push(tracks_def_value(AnimalTracksDefinitions::default()))));
    b.static_fn("loadTracksDefinitions", |c| {
        let env = c.thread.globals().clone();
        reload_from_env(&env)?;
        Ok(0)
    });
    b.static_fn("getTracksDefinition", |c| {
        let env = c.thread.globals().clone();
        let tables = ensure_loaded(&env)?;
        let map = JavaMap::hash_map();
        for (k, v) in tables.tracks.iter() {
            map.put(Value::str(k.as_str()), tracks_def_value(v.clone()));
        }
        Ok(c.push(Value::Object(Rc::new(map))))
    });
    b.static_fn("getTrackType", |c| {
        let (Some(animal_type), Some(track_type)) = (str_arg(c, 0), str_arg(c, 1)) else {
            return Ok(c.push(Value::Nil));
        };
        let env = c.thread.globals().clone();
        let tables = ensure_loaded(&env)?;
        let out = tables
            .tracks
            .get(&animal_type)
            .and_then(|d| d.tracks.as_ref())
            .and_then(|m| m.get(&track_type))
            .cloned()
            .map(tracks_type_value)
            .unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
    b.static_fn("getRandomTrack", |c| {
        let (Some(animal_type), Some(action_type)) = (str_arg(c, 0), str_arg(c, 1)) else {
            return Ok(c.push(Value::Nil));
        };
        let env = c.thread.globals().clone();
        let tables = ensure_loaded(&env)?;
        let Some(def) = tables.tracks.get(&animal_type) else {
            return Ok(c.push(Value::Nil));
        };
        let sandbox_track_chance = crate::bind_sandbox::current_double("AnimalTrackChance")
            .map(|v| v as i32)
            .unwrap_or(4);
        let mut rng = LuaRand;
        let out = def
            .random_track(&action_type, sandbox_track_chance, &mut rng)
            .map_err(|e| LuaError::runtime(format!("{e}")))?
            .cloned()
            .map(tracks_type_value)
            .unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
}

fn register_tracks_type(thread: &mut Thread) {
    prepare_field_class(thread, ANIMAL_TRACKS_TYPE_CLASS);
    let b = expose_class(thread, ANIMAL_TRACKS_TYPE_CLASS, None);
    b.ctor(|c| Ok(c.push(tracks_type_value(AnimalTracksType::default()))));
    let methods = b.methods.clone();
    install_field_index(thread, ANIMAL_TRACKS_TYPE_CLASS, methods, |c, name| {
        let o = this::<LuaAnimalTracksType>(c)?;
        let t = &o.as_any().downcast_ref::<LuaAnimalTracksType>().unwrap().0;
        let val = match name {
            "type" => opt_str_val(&t.type_),
            "name" => opt_str_val(&t.name),
            "needDir" => Value::Bool(t.need_dir),
            "sprite" => opt_str_val(&t.sprite),
            "actionType" => opt_str_val(&t.action_type),
            "chanceToFindTrack" => Value::Num(t.chance_to_find_track as f64),
            "minSkill" => Value::Num(t.min_skill as f64),
            "chanceToSpawn" => Value::Num(t.chance_to_spawn as f64),
            "item" => opt_str_val(&t.item),
            _ => Value::Nil,
        };
        Ok(val)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animal_definitions_genome_and_tracks_round_trip_from_lua() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let src = r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            AnimalGenomeDefinitions = AnimalGenomeDefinitions or {}
            AnimalGenomeDefinitions.genes = {
                maxWeight = { minValue = 0.25, ratio = { meatRatio = 0.5 } },
                meatRatio = { forcedValues = true },
            }
            AnimalGenomeDefinitions.geneticDisorder = { dwarf = true, albino = true }

            AnimalDefinitions = AnimalDefinitions or {}
            AnimalDefinitions.animals = AnimalDefinitions.animals or {}
            AnimalDefinitions.animals["calf"] = {
                bodyModel = "CalfModel",
                bodyModelSkel = "CalfSkel",
                textureSkeleton = "CalfSkelTex",
                group = "cow",
                babyNbr = "1,2",
                babyType = "calf",
                enterHutchTime = 20,
                exitHutchTime = 6,
                stages = {
                    calf = { ageToGrow = 30, nextStage = "cow" }
                },
                breeds = {
                    angus = {
                        milkType = "CowMilk",
                        woolType = "None",
                        featherItem = "Base.Feather",
                        rottenTexture = "CalfRotten",
                        sounds = {
                            idle = { name = "CowIdle", intervalMin = 5, intervalMax = 10, priority = 2 }
                        }
                    }
                }
            }

            AnimalTracksDefinitions = AnimalTracksDefinitions or {}
            AnimalTracksDefinitions.animallist = {
                deer = {
                    skillToIdentify = 2,
                    tracks = {
                        footstep = { name = "footstep", actionType = "walk", sprite = "tr_01", chanceToSpawn = 100 }
                    }
                }
            }

            local def = AnimalDefinitions.getDef("calf")
            assert(def ~= nil, "def loaded lazily")
            assert(def:getAnimalType() == "calf")
            assert(def:getBodyModelStr() == "CalfModel")
            assert(def:getGroup() == "cow")
            assert(def:getMinBaby() == 1 and def:getMaxBaby() == 2)
            assert(def:isBaby() == true)
            assert(def:canBeSkeleton() == true)
            assert(def:isInsideHutchTime(22) == true)
            assert(def:isInsideHutchTime(12) == false)
            assert(def.group == "cow")

            local breed = def:getBreedByName("angus")
            assert(breed ~= nil, "breed found")
            assert(breed:getName() == "angus")
            assert(breed:getMilkType() == "CowMilk")
            assert(breed:getFeatherItem() == "Base.Feather")
            assert(breed:getRottenTexture() == "CalfRotten")
            assert(breed:isSoundDefined("idle") == true)
            assert(breed:isSoundUndefined("missing") == true)
            local snd = breed:getSound("idle")
            assert(snd ~= nil and snd:isIntervalValid() == true and snd.soundName == "CowIdle")
            assert(def:getBreeds():size() == 1)
            assert(def:getRandomBreed() ~= nil)

            local st = def:getGrowStage()
            assert(st ~= nil and st.nextStage == "cow")
            assert(st:getAgeToGrow(nil) == 30)

            assert(AnimalDefinitions.getAnimalDefsArray():size() == 1)
            assert(AnimalDefinitions.getAnimalDefs():size() == 1)

            local disorders = AnimalGenomeDefinitions.getGeneticDisorderList()
            assert(disorders:size() == 2)

            local tr = AnimalTracksDefinitions.getTrackType("deer", "footstep")
            assert(tr ~= nil and tr.sprite == "tr_01")
            assert(AnimalTracksDefinitions.getTracksDefinition():size() == 1)

            AnimalDefinitions.Reset()
            return true
        "#;
        let cl = pz_kahlua::stdlib::load_string(src, Some("bind_animals_test"), t.globals().clone()).unwrap();
        let res = t.pcall(&cl, &[]).unwrap_or_else(|e| panic!("lua error: {e:?}"));
        assert!(matches!(res.first(), Some(Value::Bool(true))));
        reset();
    }
}
