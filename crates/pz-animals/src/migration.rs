//! `MigrationGroupDefinitions`, built from the Lua global of the same name
//! (`media/lua/shared/Definitions/animal/MigrationGroupDefinitions.lua`).

use crate::java::{
    self, as_table, entries, lower, rawget_int, rawget_str, JHashMap, JResult, JavaException,
};
use pz_kahlua::{TableRef, Value};
use pz_world::GameRand;

/// `MigrationGroupDefinitions.MigrationGroupDefinitionsGrouped`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MigrationGroupDefinitionsGrouped {
    pub animal: Option<String>,
    pub chance: i32,
}

/// One migration group.
#[derive(Clone, Debug, PartialEq)]
pub struct MigrationGroupDefinitions {
    pub type_: Option<String>,
    pub male: Option<String>,
    pub female: Option<String>,
    pub baby: Option<String>,
    pub min_animal: i32,
    pub max_animal: i32,
    pub max_male: i32,
    pub baby_chance: i32,
    pub min_time_before_eat: i32,
    pub max_time_before_eat: i32,
    pub time_to_eat: i32,
    pub min_time_before_sleep: i32,
    pub max_time_before_sleep: i32,
    pub sleep_period_start: Option<Vec<String>>,
    pub sleep_period_end: Option<Vec<String>>,
    pub eat_period_start: Option<Vec<String>>,
    pub eat_period_end: Option<Vec<String>>,
    pub time_to_sleep: i32,
    pub speed: f32,
    pub track_chance: i32,
    pub poop_chance: i32,
    pub broken_twigs_chance: i32,
    pub herb_graze_chance: i32,
    pub fur_chance: i32,
    pub flat_herb_chance: i32,
    pub possible_breed: Option<Vec<String>>,
    pub group: Option<Vec<MigrationGroupDefinitionsGrouped>>,
}

impl Default for MigrationGroupDefinitions {
    fn default() -> Self {
        MigrationGroupDefinitions {
            type_: None,
            male: None,
            female: None,
            baby: None,
            min_animal: 0,
            max_animal: 0,
            max_male: 0,
            baby_chance: 0,
            min_time_before_eat: 10,
            max_time_before_eat: 60,
            time_to_eat: 30,
            min_time_before_sleep: 180,
            max_time_before_sleep: 600,
            sleep_period_start: None,
            sleep_period_end: None,
            eat_period_start: None,
            eat_period_end: None,
            time_to_sleep: 30,
            speed: 1.0,
            track_chance: 200,
            poop_chance: 200,
            broken_twigs_chance: 200,
            herb_graze_chance: 200,
            fur_chance: 200,
            flat_herb_chance: 200,
            possible_breed: None,
            group: None,
        }
    }
}

/// The values `initValueFromDef(VirtualAnimal)` copies onto a virtual
/// animal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VirtualAnimalValues {
    pub speed: f32,
    pub time_to_eat: i32,
    pub time_to_sleep: i32,
    pub track_chance: i32,
    pub poop_chance: i32,
    pub broken_twigs_chance: i32,
    pub herb_graze_chance: i32,
    pub fur_chance: i32,
    pub flat_herb_chance: i32,
    pub sleep_period_start: Vec<i32>,
    pub sleep_period_end: Vec<i32>,
    pub eat_period_start: Vec<i32>,
    pub eat_period_end: Vec<i32>,
}

impl MigrationGroupDefinitions {
    /// `loadMigrationsDefinitions()` over a Lua environment: `migrationDef`
    /// in its iteration order.
    pub fn load_all(env: &TableRef) -> JResult<JHashMap<MigrationGroupDefinitions>> {
        let mut out = JHashMap::new();
        let Some(definitions) = env.get_str("MigrationGroupDefinitions").as_table().cloned() else {
            return Ok(out);
        };
        for (k, v) in entries(&definitions) {
            let mut def = MigrationGroupDefinitions {
                type_: Some(java::to_string(&k)),
                ..Default::default()
            };
            let value2 = as_table(&v)?;
            let groups = value2.rawget(&Value::str("groups"));
            if !groups.is_nil() {
                let mut list = Vec::new();
                for (_, gv) in entries(&as_table(&groups)?) {
                    let g = as_table(&gv)?;
                    list.push(MigrationGroupDefinitionsGrouped {
                        animal: rawget_str(&g, "animal")?,
                        chance: rawget_int(&g, "chance"),
                    });
                }
                def.group = Some(list);
                out.insert(def.type_.clone().unwrap_or_default(), def);
                continue;
            }
            for (k2, v2) in entries(&value2) {
                let key = lower(&java::to_string(&k2));
                let s = java::to_string_trim(&v2);
                let i = || java::float_int(&s);
                match key.as_str() {
                    "male" => def.male = Some(s),
                    "female" => def.female = Some(s),
                    "baby" => def.baby = Some(s),
                    "maxanimal" => def.max_animal = i()?,
                    "minanimal" => def.min_animal = i()?,
                    "maxmale" => def.max_male = i()?,
                    "babychance" => def.baby_chance = i()?,
                    "mintimebeforeeat" => def.min_time_before_eat = i()?,
                    "maxtimebeforeeat" => def.max_time_before_eat = i()?,
                    "timetoeat" => def.time_to_eat = i()?,
                    "trackchance" => def.track_chance = i()?,
                    "poopchance" => def.poop_chance = i()?,
                    "brokentwigschance" => def.broken_twigs_chance = i()?,
                    "herbgrazechance" => def.herb_graze_chance = i()?,
                    "flatherbchance" => def.flat_herb_chance = i()?,
                    "furchance" => def.fur_chance = i()?,
                    "mintimebeforesleep" => def.min_time_before_sleep = i()?,
                    "maxtimebeforesleep" => def.max_time_before_sleep = i()?,
                    "timetosleep" => def.time_to_sleep = i()?,
                    "sleepperiodstart" => def.sleep_period_start = Some(java::split_comma(&s)),
                    "sleepperiodend" => def.sleep_period_end = Some(java::split_comma(&s)),
                    "eatperiodend" => def.eat_period_end = Some(java::split_comma(&s)),
                    "eatperiodstart" => def.eat_period_start = Some(java::split_comma(&s)),
                    "speed" => def.speed = java::parse_float(&s)?,
                    "possiblebreed" => def.possible_breed = Some(java::split_comma(&s)),
                    _ => {}
                }
            }
            out.insert(def.type_.clone().unwrap_or_default(), def);
        }
        Ok(out)
    }

    /// `getRandBreed()`.
    pub fn rand_breed(&self, rand: &mut dyn GameRand) -> JResult<String> {
        let list = self
            .possible_breed
            .as_ref()
            .ok_or_else(|| JavaException("NullPointerException: possibleBreed".into()))?;
        let i = rand.next_range(0, list.len() as i32);
        list.get(i as usize).cloned().ok_or_else(|| {
            JavaException(format!(
                "IndexOutOfBoundsException: Index {i} out of bounds for length {}",
                list.len()
            ))
        })
    }

    /// `getNextEatTime(animalType)` for this definition, given
    /// `GameTime.getWorldAgeHours()`.
    pub fn next_eat_time(&self, world_age_hours: f64, rand: &mut dyn GameRand) -> f64 {
        world_age_hours
            + rand.next_range(self.min_time_before_eat, self.max_time_before_eat + 1) as f64 / 60.0
    }

    /// `getNextSleepTime(animalType)` for this definition.
    pub fn next_sleep_time(&self, world_age_hours: f64, rand: &mut dyn GameRand) -> f64 {
        world_age_hours
            + rand.next_range(self.min_time_before_sleep, self.max_time_before_sleep + 1) as f64
                / 60.0
    }

    /// The values `initValueFromDef(VirtualAnimal)` sets. The period lists
    /// are appended to the virtual animal's (`Integer.parseInt` of each
    /// entry).
    pub fn virtual_animal_values(&self) -> JResult<VirtualAnimalValues> {
        let ints = |l: &Option<Vec<String>>| -> JResult<Vec<i32>> {
            l.iter().flatten().map(|s| java::parse_int(s)).collect()
        };
        Ok(VirtualAnimalValues {
            speed: self.speed,
            time_to_eat: self.time_to_eat,
            time_to_sleep: self.time_to_sleep,
            track_chance: self.track_chance,
            poop_chance: self.poop_chance,
            broken_twigs_chance: self.broken_twigs_chance,
            herb_graze_chance: self.herb_graze_chance,
            fur_chance: self.fur_chance,
            flat_herb_chance: self.flat_herb_chance,
            sleep_period_start: ints(&self.sleep_period_start)?,
            sleep_period_end: ints(&self.sleep_period_end)?,
            eat_period_start: ints(&self.eat_period_start)?,
            eat_period_end: ints(&self.eat_period_end)?,
        })
    }

    /// The group entry `generatePossibleAnimalsFromGroup` picks: the first
    /// whose cumulative chance reaches a roll of `Rand.Next(total)`.
    pub fn pick_group<'a>(
        &'a self,
        rand: &mut dyn GameRand,
    ) -> Option<&'a MigrationGroupDefinitionsGrouped> {
        let group = self.group.as_ref()?;
        let total: i32 = group.iter().map(|g| g.chance).sum();
        let r = rand.next(total);
        let mut index = 0;
        for g in group {
            if g.chance + index >= r {
                return Some(g);
            }
            index += g.chance;
        }
        None
    }
}
