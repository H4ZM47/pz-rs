//! `zombie.characters.animals.AnimalDefinitions`, built from the Lua global
//! `AnimalDefinitions.animals` (`media/lua/shared/Definitions/animal/*Definitions.lua`).

use crate::breed::{AnimalBreed, AnimalGrowStage};
use crate::genome::AnimalGenomeDefinitions;
use crate::java::{self, as_table, entries, lower, JHashMap, JResult};
use pz_kahlua::TableRef;

/// One animal type (`AnimalDefinitions`). The `Model` fields the original
/// resolves through `ModelManager` after loading are left to the renderer;
/// their names are kept (`body_model_str`, ...).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalDefinitions {
    pub animal_type_str: Option<String>,
    pub body_model_str: Option<String>,
    pub body_model_skel_no_head_str: Option<String>,
    pub body_model_fleece_str: Option<String>,
    pub body_model_skel_str: Option<String>,
    pub body_model_headless_str: Option<String>,
    pub texture_skeleton: Option<String>,
    pub texture_skeleton_bloody: Option<String>,
    pub texture_rotten: Option<String>,
    pub texture_skinned: Option<String>,
    pub animset: Option<String>,
    pub mate: Option<String>,
    pub shadoww: f32,
    pub shadowfm: f32,
    pub shadowbm: f32,
    pub turn_delta: f32,
    pub animal_size: f32,
    pub min_size: f32,
    pub max_size: f32,
    pub min_age: i32,
    pub min_enclosure_size: i32,
    pub baby_type: Option<String>,
    pub min_age_for_baby: i32,
    pub max_age_geriatric: i32,
    pub udder: bool,
    pub female: bool,
    pub male: bool,
    pub stages: Vec<AnimalGrowStage>,
    pub breeds: Vec<AnimalBreed>,
    /// Never filled by the loader (`genome`).
    pub genome: Vec<crate::genome::AnimalAllele>,
    pub always_flee_humans: bool,
    pub flee_zombies: bool,
    pub can_be_attached: bool,
    pub can_be_transported: bool,
    pub hunger_multiplier: f32,
    pub thirst_multiplier: f32,
    pub health_loss_multiplier: f32,
    pub wander_mul: f32,
    pub idle_type_nbr: i32,
    pub eating_type_nbr: i32,
    pub sitting_type_nbr: i32,
    pub eat_from_mother: bool,
    pub periodic_run: bool,
    pub pregnant_period: i32,
    pub eat_grass: bool,
    pub sit_randomly: bool,
    pub eat_type_trough: Option<Vec<String>>,
    pub can_be_milked: bool,
    pub min_baby: i32,
    pub max_baby: i32,
    pub idle_emote_chance: i32,
    pub eggs_per_day: i32,
    pub egg_type: Option<String>,
    pub fertilized_time_max: i32,
    pub time_to_hatch: i32,
    pub can_be_picked: bool,
    pub hutches: Option<Vec<String>>,
    pub enter_hutch_time: i32,
    pub exit_hutch_time: i32,
    pub genes: Option<Vec<String>>,
    pub min_milk: f32,
    pub max_milk: f32,
    pub max_wool: f32,
    pub min_weight: f32,
    pub max_weight: f32,
    pub carcass_item: Option<String>,
    pub attack_dist: i32,
    pub attack_timer: i32,
    pub dont_attack_other_male: bool,
    pub can_be_feed_by_hand: bool,
    pub base_dmg: f32,
    pub milk_anim_preset: Option<String>,
    pub feed_by_hand_type: Option<Vec<String>>,
    pub trailer_base_size: f32,
    pub can_be_pet: bool,
    pub attack_back: bool,
    pub collision_size: f32,
    pub base_encumbrance: f32,
    pub mating_period_start: i32,
    pub mating_period_end: i32,
    pub time_before_next_pregnancy: i32,
    pub thirst_hunger_trigger: f32,
    pub collidable: bool,
    pub can_thump: bool,
    pub wild: bool,
    pub spotting_dist: i32,
    pub group: Option<String>,
    pub can_be_alerted: bool,
    pub dung: Option<String>,
    pub attack_if_stressed: bool,
    pub happy_anim: i32,
    pub rope_bone: Option<String>,
    pub min_clutch_size: i32,
    pub max_clutch_size: i32,
    pub lay_egg_period_start: i32,
    pub stress_above_ground: bool,
    pub can_climb_stairs: bool,
    pub stress_under_rain: bool,
    pub can_climb_fences: bool,
    pub need_mom: bool,
    pub can_be_domesticated: bool,
    pub dung_chance_per_day: i32,
    pub hunger_boost: f32,
    pub thirst_boost: f32,
    pub dist_to_eat: f32,
    pub knockdown_attack: bool,
    pub min_body_part: i32,
    pub can_do_laceration: bool,
    pub max_blood: f32,
    pub min_blood: f32,
    pub litter_eat_together: bool,
    pub add_tracking_xp: bool,
    pub corpse_size: f32,
    pub corpse_length: f32,
    pub idle_sound_radius: f32,
    pub idle_sound_volume: f32,
    /// Private in the original (`getWildFleeTimeUntilDeadTimer`).
    pub wild_flee_time_until_dead_timer: f32,
    pub can_be_killed_without_weapon: bool,
    pub feed_by_hand_anim: Option<String>,
}

impl Default for AnimalDefinitions {
    fn default() -> Self {
        AnimalDefinitions {
            animal_type_str: None,
            body_model_str: None,
            body_model_skel_no_head_str: None,
            body_model_fleece_str: None,
            body_model_skel_str: None,
            body_model_headless_str: None,
            texture_skeleton: None,
            texture_skeleton_bloody: None,
            texture_rotten: None,
            texture_skinned: None,
            animset: None,
            mate: None,
            shadoww: 0.0,
            shadowfm: 0.0,
            shadowbm: 0.0,
            turn_delta: 0.8,
            animal_size: 0.0,
            min_size: 0.0,
            max_size: 0.0,
            min_age: 0,
            min_enclosure_size: 0,
            baby_type: None,
            min_age_for_baby: 0,
            max_age_geriatric: 0,
            udder: false,
            female: false,
            male: false,
            stages: Vec::new(),
            breeds: Vec::new(),
            genome: Vec::new(),
            always_flee_humans: true,
            flee_zombies: true,
            can_be_attached: false,
            can_be_transported: false,
            hunger_multiplier: 0.0,
            thirst_multiplier: 0.0,
            health_loss_multiplier: 0.05,
            wander_mul: 400.0,
            idle_type_nbr: 0,
            eating_type_nbr: 0,
            sitting_type_nbr: 0,
            eat_from_mother: false,
            periodic_run: false,
            pregnant_period: 0,
            eat_grass: false,
            sit_randomly: false,
            eat_type_trough: None,
            can_be_milked: false,
            min_baby: 1,
            max_baby: 1,
            idle_emote_chance: 1000,
            eggs_per_day: 0,
            egg_type: None,
            fertilized_time_max: 0,
            time_to_hatch: 0,
            can_be_picked: true,
            hutches: None,
            enter_hutch_time: 0,
            exit_hutch_time: 0,
            genes: None,
            min_milk: 0.0,
            max_milk: 0.0,
            max_wool: 0.0,
            min_weight: 10.0,
            max_weight: 100.0,
            carcass_item: None,
            attack_dist: 1,
            attack_timer: 1000,
            dont_attack_other_male: false,
            can_be_feed_by_hand: false,
            base_dmg: 0.5,
            milk_anim_preset: None,
            feed_by_hand_type: None,
            trailer_base_size: 0.0,
            can_be_pet: false,
            attack_back: false,
            collision_size: 0.0,
            base_encumbrance: 1.0,
            mating_period_start: 0,
            mating_period_end: 0,
            time_before_next_pregnancy: 0,
            thirst_hunger_trigger: 0.1,
            collidable: true,
            can_thump: true,
            wild: false,
            spotting_dist: 10,
            group: None,
            can_be_alerted: false,
            dung: None,
            attack_if_stressed: false,
            happy_anim: 0,
            rope_bone: Some("Bip01_Head".into()),
            min_clutch_size: -1,
            max_clutch_size: -1,
            lay_egg_period_start: -1,
            stress_above_ground: false,
            can_climb_stairs: false,
            stress_under_rain: false,
            can_climb_fences: false,
            need_mom: true,
            can_be_domesticated: true,
            dung_chance_per_day: 50,
            hunger_boost: 1.0,
            thirst_boost: 1.0,
            dist_to_eat: 2.0,
            knockdown_attack: false,
            min_body_part: 0,
            can_do_laceration: false,
            max_blood: 0.0,
            min_blood: 0.0,
            litter_eat_together: false,
            add_tracking_xp: true,
            corpse_size: 1.0,
            corpse_length: 1.0,
            idle_sound_radius: 0.0,
            idle_sound_volume: 0.0,
            wild_flee_time_until_dead_timer: 0.0,
            can_be_killed_without_weapon: false,
            feed_by_hand_anim: Some("AnimalLureHigh".into()),
        }
    }
}

impl AnimalDefinitions {
    /// `loadAnimalDefinitions()` over a Lua environment (`LuaManager.env`):
    /// `animalDefs` in its `HashMap` iteration order. The genome
    /// definitions it loads first when missing are a separate
    /// [`AnimalGenomeDefinitions::load`]. A missing `AnimalDefinitions`
    /// global gives an empty map, as in the original.
    pub fn load_all(env: &TableRef) -> JResult<JHashMap<AnimalDefinitions>> {
        let mut defs = JHashMap::new();
        let Some(definitions) = env.get_str("AnimalDefinitions").as_table().cloned() else {
            return Ok(defs);
        };
        let animals = as_table(&definitions.rawget(&pz_kahlua::Value::str("animals")))?;
        for (k, v) in entries(&animals) {
            let def = Self::load_one(java::to_string(&k), &as_table(&v)?)?;
            defs.insert(def.animal_type_str.clone().unwrap_or_default(), def);
        }
        Ok(defs)
    }

    fn load_one(animal_type: String, table: &TableRef) -> JResult<AnimalDefinitions> {
        let mut d = AnimalDefinitions {
            animal_type_str: Some(animal_type),
            ..Default::default()
        };
        for (k, v) in entries(table) {
            let key = lower(&java::to_string(&k));
            let raw = java::to_string(&v);
            let s = java::java_trim(&raw).to_string();
            let f = || java::parse_float(&s);
            let i = || java::float_int(&s);
            let b = || java::parse_bool(&s);
            match key.as_str() {
                "bodymodel" => d.body_model_str = Some(raw),
                "bodymodelfleece" => d.body_model_fleece_str = Some(raw),
                "bodymodelheadless" => d.body_model_headless_str = Some(raw),
                "bodymodelskelnohead" => d.body_model_skel_no_head_str = Some(raw),
                "textureskeleton" => d.texture_skeleton = Some(raw),
                "textureskeletonbloody" => d.texture_skeleton_bloody = Some(raw),
                "texturerotten" => d.texture_rotten = Some(raw),
                "textureskinned" => d.texture_skinned = Some(raw),
                "bodymodelskel" => d.body_model_skel_str = Some(raw),
                "carcassitem" => d.carcass_item = Some(s),
                "corpsesize" => d.corpse_size = f()?,
                "corpselength" => d.corpse_length = f()?,
                "idlesoundradius" => d.idle_sound_radius = f()?,
                "idlesoundvolume" => d.idle_sound_volume = f()?,
                "wildfleetimeuntildeadtimer" => d.wild_flee_time_until_dead_timer = f()?,
                "maxblood" => d.max_blood = f()?,
                "minblood" => d.min_blood = f()?,
                "milkanimpreset" => d.milk_anim_preset = Some(s),
                "animset" => d.animset = Some(s),
                "ropebone" => d.rope_bone = Some(s),
                "mate" => d.mate = Some(s),
                "dung" => d.dung = Some(s),
                "shadoww" => d.shadoww = f()?,
                "shadowfm" => d.shadowfm = f()?,
                "shadowbm" => d.shadowbm = f()?,
                "turndelta" => d.turn_delta = f()?,
                "animalsize" => d.animal_size = f()?,
                "minsize" => d.min_size = f()?,
                "maxsize" => d.max_size = f()?,
                "hungermultiplier" => d.hunger_multiplier = f()?,
                "thirstmultiplier" => d.thirst_multiplier = f()?,
                "healthlossmultiplier" => d.health_loss_multiplier = f()?,
                "wandermul" => d.wander_mul = f()?,
                "minenclosuresize" => d.min_enclosure_size = i()?,
                "happyanim" => d.happy_anim = i()?,
                "minage" => d.min_age = i()?,
                "minageforbaby" => d.min_age_for_baby = i()?,
                "attackdist" => d.attack_dist = i()?,
                "attacktimer" => d.attack_timer = i()?,
                "timebeforenextpregnancy" => d.time_before_next_pregnancy = i()?,
                "spottingdist" => d.spotting_dist = i()?,
                "dungchanceperday" => d.dung_chance_per_day = i()?,
                "minbodypart" => d.min_body_part = i()?,
                "minclutchsize" => d.min_clutch_size = i()?,
                "maxclutchsize" => d.max_clutch_size = i()?,
                "layeggperiodstart" => d.lay_egg_period_start = i()?,
                "matingperiodstart" => d.mating_period_start = i()?,
                "matingperiodend" => d.mating_period_end = i()?,
                "babytype" => d.baby_type = Some(s),
                "group" => d.group = Some(s),
                "eggtype" => d.egg_type = Some(s),
                "maxagegeriatric" => d.max_age_geriatric = i()?,
                "idleemotechance" => d.idle_emote_chance = i()?,
                "eggsperday" => d.eggs_per_day = i()?,
                "fertilizedtimemax" => d.fertilized_time_max = i()?,
                "timetohatch" => d.time_to_hatch = i()?,
                "enterhutchtime" => d.enter_hutch_time = i()?,
                "exithutchtime" => d.exit_hutch_time = i()?,
                "idletypenbr" => d.idle_type_nbr = i()?,
                "eatingtypenbr" => d.eating_type_nbr = i()?,
                "sittingtypenbr" => d.sitting_type_nbr = i()?,
                "pregnantperiod" => d.pregnant_period = i()?,
                "minweight" => d.min_weight = f()?,
                "maxweight" => d.max_weight = f()?,
                "littereattogether" => d.litter_eat_together = b(),
                "udder" => d.udder = b(),
                "female" => d.female = b(),
                "male" => d.male = b(),
                "addtrackingxp" => d.add_tracking_xp = b(),
                "fleezombies" => d.flee_zombies = b(),
                "stressaboveground" => d.stress_above_ground = b(),
                "stressunderrain" => d.stress_under_rain = b(),
                "canclimbfences" => d.can_climb_fences = b(),
                "needmom" => d.need_mom = b(),
                "canbedomesticated" => d.can_be_domesticated = b(),
                "knockdownattack" => d.knockdown_attack = b(),
                "candolaceration" => d.can_do_laceration = b(),
                "canclimbstairs" => d.can_climb_stairs = b(),
                "canbealerted" => d.can_be_alerted = b(),
                "attackifstressed" => d.attack_if_stressed = b(),
                "alwaysfleehumans" => d.always_flee_humans = b(),
                "canbeattached" => d.can_be_attached = b(),
                "canbetransported" => d.can_be_transported = b(),
                "eatfrommother" => d.eat_from_mother = b(),
                "periodicrun" => d.periodic_run = b(),
                "eatgrass" => d.eat_grass = b(),
                "sitrandomly" => d.sit_randomly = b(),
                "canbemilked" => d.can_be_milked = b(),
                "canbepicked" => d.can_be_picked = b(),
                "collidable" => d.collidable = b(),
                "canthump" => d.can_thump = b(),
                "wild" => d.wild = b(),
                "dontattackothermale" => d.dont_attack_other_male = b(),
                "canbepet" => d.can_be_pet = b(),
                "attackback" => d.attack_back = b(),
                "canbefeedbyhand" => d.can_be_feed_by_hand = b(),
                "eattypetrough" => d.eat_type_trough = Some(java::split_comma(&s)),
                "hutches" => d.hutches = Some(java::split_comma(&s)),
                "feedbyhandtype" => d.feed_by_hand_type = Some(java::split_comma(&s)),
                "collisionsize" => d.collision_size = f()?,
                "thirsthungertrigger" => d.thirst_hunger_trigger = f()?,
                "hungerboost" => d.hunger_boost = f()?,
                "thirstboost" => d.thirst_boost = f()?,
                "disttoeat" => d.dist_to_eat = f()?,
                "babynbr" => {
                    let split = java::split_comma(&s);
                    let at = |i: usize| {
                        split.get(i).ok_or_else(|| {
                            java::JavaException(format!(
                                "ArrayIndexOutOfBoundsException: Index {i} out of bounds for length {}",
                                split.len()
                            ))
                        })
                    };
                    d.min_baby = java::parse_int(at(0)?)?;
                    d.max_baby = java::parse_int(at(1)?)?;
                }
                "maxmilk" => d.max_milk = f()?,
                "basedmg" => d.base_dmg = f()?,
                "minmilk" => d.min_milk = f()?,
                "trailerbasesize" => d.trailer_base_size = f()?,
                "maxwool" => d.max_wool = f()?,
                "baseencumbrance" => d.base_encumbrance = f()?,
                "stages" => {
                    for (sk, sv) in entries(&as_table(&v)?) {
                        let stage = AnimalGrowStage::load(java::to_string(&sk), &as_table(&sv)?)?;
                        d.stages.push(stage);
                    }
                }
                "breeds" => {
                    for (bk, bv) in entries(&as_table(&v)?) {
                        let breed = AnimalBreed::load(java::to_string(&bk), &as_table(&bv)?)?;
                        d.breeds.push(breed);
                    }
                }
                "genes" => {
                    let mut genes: Vec<String> = Vec::new();
                    for (_, gv) in entries(&as_table(&v)?) {
                        let g = java::to_string_trim(&gv);
                        if !genes.contains(&g) {
                            genes.push(g);
                        }
                    }
                    d.genes = Some(genes);
                }
                "canbekilledwithoutweapon" => d.can_be_killed_without_weapon = b(),
                "feedbyhandanim" => d.feed_by_hand_anim = Some(s),
                _ => {}
            }
        }
        Ok(d)
    }

    /// `getBreedByName(name)`.
    pub fn breed_by_name(&self, name: &str) -> Option<&AnimalBreed> {
        self.breeds.iter().find(|b| b.name.as_deref() == Some(name))
    }

    /// `getRandomBreed()`.
    pub fn random_breed(&self, rand: &mut dyn pz_world::GameRand) -> Option<&AnimalBreed> {
        let i = rand.next_range(0, self.breeds.len() as i32);
        self.breeds.get(i as usize)
    }

    /// `isInsideHutchTime(hour)`; `hour` is the caller's
    /// `GameTime.getHour()` when the original is passed `null` or a
    /// negative hour.
    pub fn is_inside_hutch_time(&self, hour: i32) -> bool {
        let (enter, exit) = (self.enter_hutch_time, self.exit_hutch_time);
        if enter < exit {
            hour >= enter && hour < exit
        } else {
            hour < exit || hour >= enter
        }
    }

    /// `getGrowStage()`: the stage named after this animal type.
    pub fn grow_stage(&self) -> Option<&AnimalGrowStage> {
        let t = self.animal_type_str.as_deref()?;
        self.stages.iter().find(|s| s.stage.as_deref() == Some(t))
    }

    /// `isBaby()`.
    pub fn is_baby(&self) -> bool {
        self.grow_stage().is_some_and(|s| s.next_stage.is_some())
    }

    /// `canBeSkeleton()`, given whether `bodyModelSkel` resolved to a
    /// loaded model.
    pub fn can_be_skeleton(&self, has_skel_model: bool) -> bool {
        self.texture_skeleton.is_some() && has_skel_model
    }

    /// The subset of this definition that loading a saved animal reads
    /// (`pz_world::save::animal::AnimalDefinition`).
    pub fn to_save_definition(
        &self,
        genome: &AnimalGenomeDefinitions,
    ) -> pz_world::save::animal::AnimalDefinition {
        let known = |g: &String| genome.full_genome_def.contains_key(&lower(g));
        pz_world::save::animal::AnimalDefinition {
            eggs_per_day: self.eggs_per_day,
            max_wool: self.max_wool,
            min_size: self.min_size,
            max_size: self.max_size,
            can_be_domesticated: self.can_be_domesticated,
            wild: self.wild,
            baby: self.is_baby(),
            genes: self
                .genes
                .as_ref()
                .map(|g| g.iter().filter(|g| known(g)).cloned().collect()),
            breeds: self
                .breeds
                .iter()
                .map(|b| pz_world::save::animal::AnimalBreedDef {
                    name: b.name.clone().unwrap_or_default(),
                    wool_type: b.wool_type.clone(),
                    forced_genes: b
                        .forced_genes
                        .as_ref()
                        .map(|m| m.keys().into_iter().filter(|g| known(g)).collect())
                        .unwrap_or_default(),
                })
                .collect(),
        }
    }
}
