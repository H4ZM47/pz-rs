//! The Lua-defined tables a zombie's outfit comes from:
//! `zombie.characters.ZombiesZoneDefinition` (`ZombiesZoneDefinition`),
//! `UnderwearDefinition` (`UnderwearDefinition`),
//! `AttachedItems.AttachedWeaponDefinitions` (`AttachedWeaponDefinitions`)
//! and `ZombiesStageDefinitions` (built in). The Lua binding reads the
//! tables (`KahluaTableImpl` iteration order is insertion order) into these
//! structs on `checkDirty`; the draws that only need the tables are here,
//! the ones that dress a zombie are `GameCharacter`'s
//! (`character::zombie::dress`).

use pz_core::random::{self as rand, outfit_rng};
use pz_fs::java::is_null_or_whitespace;
use pz_scripts::java::{compare, eq_ignore_case, parse_f32, split_char};
use std::collections::HashMap;

/// `ZombiesZoneDefinition.StringChance` / `UnderwearDefinition.StringChance`.
#[derive(Clone, Debug, PartialEq)]
pub struct StringChance {
    pub str: String,
    pub chance: f32,
}

// ---- ZombiesStageDefinitions ------------------------------------------------

/// `ZombiesStageDefinitions.daysEarly` / `daysMid` / `daysLate`.
pub const DAYS_EARLY: i32 = 10;
pub const DAYS_MID: i32 = 30;
pub const DAYS_LATE: i32 = 90;

/// `ZombiesStageDefinitions.init()`'s table: outfit, later outfits, mixed.
const STAGES: [(&str, [&str; 3], bool); 6] = [
    ("Bandit", ["Bandit_Early", "Bandit_Mid", "Bandit_Late"], true),
    ("Survivalist", ["Survivalist", "Survivalist_Mid", "Survivalist_Late"], false),
    ("Survivalist02", ["Survivalist02", "Survivalist02_Mid", "Survivalist02_Late"], false),
    ("Survivalist03", ["Survivalist03", "Survivalist03_Mid", "Survivalist03_Late"], false),
    ("Survivalist04", ["Survivalist04", "Survivalist04_Mid", "Survivalist04_Late"], false),
    ("Survivalist05", ["Survivalist05", "Survivalist05_Mid", "Survivalist05_Late"], false),
];

/// `ZombiesStageDefinitions.instance.getAdvancedOutfitName(outfitName)`:
/// a later stage of the outfit by `GameTime.getWorldAgeDaysSinceBegin()`
/// (a mixed one picks a stage with `Rand`); `None` keeps the name.
pub fn advanced_outfit_name(outfit_name: &str, world_age_days_since_begin: f64) -> Option<String> {
    if outfit_name.is_empty() {
        return None;
    }
    let (_, later, mixed) = STAGES.iter().find(|(o, _, _)| *o == outfit_name)?;
    let days = world_age_days_since_begin as i32;
    let possible = if days >= DAYS_LATE {
        3
    } else if days >= DAYS_MID {
        2
    } else if days >= DAYS_EARLY {
        1
    } else {
        0
    };
    let possible = possible.min(later.len() as i32);
    if possible < 1 {
        return None;
    }
    let roll = if *mixed { rand::next(possible) + 1 } else { possible };
    Some(later[roll as usize - 1].to_string())
}

// ---- UnderwearDefinition -------------------------------------------------------

/// `UnderwearDefinition.OutfitUnderwearDefinition`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutfitUnderwearDefinition {
    pub top: Option<Vec<StringChance>>,
    pub top_total_chance: i32,
    pub bottom: Option<String>,
    pub chance_to_spawn: i32,
    pub female: bool,
}

impl OutfitUnderwearDefinition {
    /// The constructor: `topTotalChance` sums the tops' chances as ints.
    pub fn new(top: Option<Vec<StringChance>>, bottom: Option<String>, chance_to_spawn: i32, gender: Option<&str>) -> Self {
        let mut total = 0i32;
        for t in top.iter().flatten() {
            total = (total as f32 + t.chance) as i32;
        }
        OutfitUnderwearDefinition { top, top_total_chance: total, bottom, chance_to_spawn, female: gender == Some("female") }
    }
}

/// `UnderwearDefinition`'s statics.
#[derive(Clone, Debug, PartialEq)]
pub struct UnderwearDefinition {
    pub dirty: bool,
    pub base_chance: i32,
    pub outfit_definition: Vec<OutfitUnderwearDefinition>,
}

impl Default for UnderwearDefinition {
    fn default() -> Self {
        UnderwearDefinition { dirty: true, base_chance: 50, outfit_definition: Vec::new() }
    }
}

/// What `addRandomUnderwear` picked: the bottom's item type and the
/// definition's tops.
#[derive(Clone, Debug, PartialEq)]
pub struct UnderwearPick {
    pub bottom: Option<String>,
    pub tops: Option<Vec<StringChance>>,
    pub top_total_chance: i32,
}

impl UnderwearDefinition {
    /// `addRandomUnderwear`'s draws up to the definition (after the
    /// skeleton check and `checkDirty`): `None` when the base chance roll
    /// fails or no definition is picked.
    pub fn pick(&self, female: bool) -> Option<UnderwearPick> {
        if outfit_rng::next(100) > self.base_chance {
            return None;
        }
        let valid: Vec<&OutfitUnderwearDefinition> = self.outfit_definition.iter().filter(|d| d.female == female).collect();
        let total: i32 = valid.iter().fold(0i32, |a, d| a.wrapping_add(d.chance_to_spawn));
        let choice = outfit_rng::next(total);
        let mut subtotal = 0i32;
        for d in valid {
            subtotal = subtotal.wrapping_add(d.chance_to_spawn);
            if choice < subtotal {
                return Some(UnderwearPick { bottom: d.bottom.clone(), tops: d.top.clone(), top_total_chance: d.top_total_chance });
            }
        }
        None
    }

    /// The top roll: the top's item type, if any.
    pub fn pick_top(tops: &[StringChance], total: i32) -> Option<String> {
        let choice = outfit_rng::next(total);
        let mut subtotal = 0i32;
        for t in tops {
            subtotal = (subtotal as f32 + t.chance) as i32;
            if choice < subtotal {
                return Some(t.str.clone());
            }
        }
        None
    }
}

// ---- AttachedWeaponDefinitions ----------------------------------------------------

/// `AttachedWeaponDefinition`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttachedWeaponDefinition {
    pub id: String,
    pub chance: i32,
    pub outfit: Vec<String>,
    pub weapon_location: Vec<String>,
    /// `BloodBodyPartType` indices, sorted.
    pub blood_locations: Vec<usize>,
    pub add_holes: bool,
    pub day_survived: i32,
    pub ensure_item: Option<String>,
    pub weapons: Vec<String>,
}

impl AttachedWeaponDefinition {
    /// `init(key, table)`'s sorts: locations, blood parts and weapons.
    pub fn sorted(mut self) -> Self {
        self.weapon_location.sort_by(|a, b| compare(a, b));
        self.blood_locations.sort();
        self.weapons.sort_by(|a, b| compare(a, b));
        self
    }
}

/// `AttachedWeaponCustomOutfit`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttachedWeaponCustomOutfit {
    pub outfit: String,
    pub chance: i32,
    pub maxitem: i32,
    pub weapons: Vec<AttachedWeaponDefinition>,
}

/// `AttachedWeaponDefinitions.instance`.
#[derive(Clone, Debug, PartialEq)]
pub struct AttachedWeaponDefinitions {
    pub dirty: bool,
    pub chance_of_attached_weapon: i32,
    /// Sorted by id.
    pub definitions: Vec<AttachedWeaponDefinition>,
    pub outfit_definitions: Vec<AttachedWeaponCustomOutfit>,
}

impl Default for AttachedWeaponDefinitions {
    fn default() -> Self {
        AttachedWeaponDefinitions { dirty: true, chance_of_attached_weapon: 0, definitions: Vec::new(), outfit_definitions: Vec::new() }
    }
}

/// `BloodBodyPartType.FromString(name)` (`MAX` for an unknown name).
pub fn blood_part_from_string(name: &str) -> usize {
    const NAMES: [&str; 18] = [
        "Hand_L", "Hand_R", "ForeArm_L", "ForeArm_R", "UpperArm_L", "UpperArm_R", "Torso_Upper", "Torso_Lower", "Head", "Neck", "Groin",
        "UpperLeg_L", "UpperLeg_R", "LowerLeg_L", "LowerLeg_R", "Foot_L", "Foot_R", "Back",
    ];
    NAMES.iter().position(|n| *n == name).unwrap_or(NAMES.len())
}

/// The weapons an `addRandomAttachedWeapon` call picks, one per round.
#[derive(Clone, Debug, Default)]
pub struct AttachedWeaponRounds {
    /// The definitions still in the pool.
    pub pool: Vec<AttachedWeaponDefinition>,
    pub repeat: i32,
    /// The custom outfit's chance (`customOutfit.chance`) when one applies.
    pub custom_chance: Option<i32>,
}

impl AttachedWeaponDefinitions {
    /// `addRandomAttachedWeapon`'s pool (after the tutorial check and
    /// `checkDirty`): the outfit's custom weapons (rolled against their
    /// chance), else every definition when the attached-weapon chance roll
    /// passes. `None` when nothing is to be added.
    pub fn rounds(&self, outfit: Option<&str>) -> Option<AttachedWeaponRounds> {
        if self.definitions.is_empty() {
            return None;
        }
        let mut r = AttachedWeaponRounds { pool: Vec::new(), repeat: 1, custom_chance: None };
        if let Some(name) = outfit {
            for c in &self.outfit_definitions {
                if c.outfit == name && outfit_rng::next(100) < c.chance {
                    r.pool.extend(c.weapons.iter().cloned());
                    r.repeat = if c.maxitem > -1 { c.maxitem } else { 1 };
                    r.custom_chance = Some(c.chance);
                    break;
                }
            }
        }
        if r.pool.is_empty() {
            if outfit_rng::next(100) + 1 > self.chance_of_attached_weapon {
                return None;
            }
            r.pool.extend(self.definitions.iter().cloned());
        }
        Some(r)
    }

    /// `pickRandomInList(definitions, zed)`: the definitions the zombie
    /// qualifies for (world age, outfit), weighted by chance.
    pub fn pick_random_in_list(pool: &[AttachedWeaponDefinition], outfit: Option<&str>, world_age_days: f32) -> Option<usize> {
        let mut total = 0i32;
        let mut possible: Vec<usize> = Vec::new();
        for (i, d) in pool.iter().enumerate() {
            if d.day_survived > 0 {
                // `!(age > daySurvived)`: a NaN age fails too.
                if world_age_days.partial_cmp(&(d.day_survived as f32)) != Some(std::cmp::Ordering::Greater) {
                    continue;
                }
            } else if !d.outfit.is_empty() && !outfit.is_some_and(|o| d.outfit.iter().any(|x| x == o)) {
                continue;
            }
            total = total.wrapping_add(d.chance);
            possible.push(i);
        }
        let choice = outfit_rng::next(total);
        let mut subtotal = 0i32;
        for i in possible {
            subtotal = subtotal.wrapping_add(pool[i].chance);
            if choice < subtotal {
                return Some(i);
            }
        }
        None
    }
}

// ---- ZombiesZoneDefinition ----------------------------------------------------

/// `ZZDOutfit`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZzdOutfit {
    pub name: Option<String>,
    pub custom_name: String,
    pub chance: f32,
    pub to_spawn: i32,
    pub gender: Option<String>,
    pub mandatory: Option<String>,
    pub room: Option<String>,
    pub female_hair_styles: Option<Vec<StringChance>>,
    pub male_hair_styles: Option<Vec<StringChance>>,
    pub beard_styles: Option<Vec<StringChance>>,
}

impl ZzdOutfit {
    /// `initOutfit(table)` from the table's `name`, `chance`, `gender`,
    /// `toSpawn`, `mandatory`, `room` and the hair/beard style strings.
    /// `Err` is the exception parsing a style string throws.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: Option<String>,
        chance: f32,
        gender: Option<String>,
        to_spawn: i32,
        mandatory: Option<String>,
        room: Option<String>,
        female_hair_styles: Option<&str>,
        male_hair_styles: Option<&str>,
        beard_styles: Option<&str>,
    ) -> Result<Self, String> {
        Ok(ZzdOutfit {
            name,
            custom_name: String::new(),
            chance,
            to_spawn,
            gender,
            mandatory,
            room,
            female_hair_styles: init_string_chance(female_hair_styles)?,
            male_hair_styles: init_string_chance(male_hair_styles)?,
            beard_styles: init_string_chance(beard_styles)?,
        })
    }
}

/// `ZZDZone`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZzdZone {
    pub name: String,
    pub female_chance: i32,
    pub male_chance: i32,
    pub chance_to_spawn: i32,
    pub to_spawn: i32,
    pub outfits: Vec<ZzdOutfit>,
}

impl ZzdZone {
    /// `initZone(name, table)`: each outfit's custom name is
    /// `ZZD.<zone>.<outfit>`.
    pub fn new(name: String, female_chance: i32, male_chance: i32, chance_to_spawn: i32, to_spawn: i32, outfits: Vec<ZzdOutfit>) -> Self {
        let outfits = outfits
            .into_iter()
            .map(|mut o| {
                o.custom_name = format!("ZZD.{name}.{}", o.name.as_deref().unwrap_or("null"));
                o
            })
            .collect();
        ZzdZone { name, female_chance, male_chance, chance_to_spawn, to_spawn, outfits }
    }
}

/// `initStringChance(styles)`: `"a:60;b:10"` (`null` for a blank string).
pub fn init_string_chance(styles: Option<&str>) -> Result<Option<Vec<StringChance>>, String> {
    let Some(styles) = styles.filter(|s| !is_null_or_whitespace(s)) else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for style in split_char(styles, ';') {
        let parts = split_char(style, ':');
        let str = parts.first().copied().ok_or("java.lang.ArrayIndexOutOfBoundsException: Index 0 out of bounds for length 0")?;
        let chance = parts
            .get(1)
            .ok_or_else(|| format!("java.lang.ArrayIndexOutOfBoundsException: Index 1 out of bounds for length {}", parts.len()))?;
        let chance = parse_f32(chance).ok_or_else(|| format!("java.lang.NumberFormatException: For input string: \"{chance}\""))?;
        out.push(StringChance { str: str.to_string(), chance });
    }
    Ok(Some(out))
}

/// A `Zone` as `ZombiesZoneDefinition` reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneView {
    /// Identifies the zone object (its [`ZoneSpawnState`] is kept by it).
    pub key: u64,
    pub name: Option<String>,
    pub zone_type: Option<String>,
    /// `zone.id` (a `UUID`).
    pub id: Option<u128>,
}

/// The transient `Zone` fields `pickDefinition` and `applyDefinition`
/// write: `spawnSpecialZombies` (a `Boolean`) and `spawnedZombies`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneSpawnState {
    pub spawn_special_zombies: Option<bool>,
    pub spawned_zombies: Option<HashMap<String, i32>>,
}

/// What `pickDefinition` returned (`PickDefinition`).
#[derive(Clone, Debug, PartialEq)]
pub struct PickDefinition {
    pub zone_key: u64,
    /// `(zone index, outfit index)` of the `ZZDOutfit`.
    pub table: (usize, usize),
    pub female: bool,
}

/// The world `ZombiesZoneDefinition` reads.
pub trait ZoneWorld {
    /// `IsoWorld.instance.currentCell.getGridSquare(x, y, z)`: `None`
    /// without a square, else the room's name (`None` outside rooms).
    fn square_room(&self, x: i32, y: i32, z: i32) -> Option<Option<String>>;
    /// `IsoWorld.instance.metaGrid.getZonesAt(x, y, z)`, in order.
    fn zones_at(&self, x: i32, y: i32, z: i32) -> Vec<ZoneView>;
}

/// `ZombiesZoneDefinition`'s statics, with the zone fields and
/// `IsoWorld.spawnedZombieZone` it updates.
#[derive(Clone, Debug)]
pub struct ZombiesZoneDefinition {
    pub dirty: bool,
    zone_list: Vec<ZzdZone>,
    zone_map: HashMap<String, usize>,
    custom_outfit_map: HashMap<String, (usize, usize)>,
    /// `Zone.spawnSpecialZombies` / `spawnedZombies` by zone key.
    pub zone_spawns: HashMap<u64, ZoneSpawnState>,
    /// `IsoWorld.spawnedZombieZone`: zone name to the zone ids that spawned
    /// special zombies.
    pub spawned_zombie_zone: HashMap<Option<String>, Vec<Option<u128>>>,
}

impl Default for ZombiesZoneDefinition {
    fn default() -> Self {
        ZombiesZoneDefinition {
            dirty: true,
            zone_list: Vec::new(),
            zone_map: HashMap::new(),
            custom_outfit_map: HashMap::new(),
            zone_spawns: HashMap::new(),
            spawned_zombie_zone: HashMap::new(),
        }
    }
}

impl ZombiesZoneDefinition {
    /// `init()` with the zones read from Lua's `ZombiesZoneDefinition`
    /// table (in its order; an empty list when the global is not a table).
    pub fn set_zones(&mut self, zones: Vec<ZzdZone>) {
        self.dirty = false;
        self.zone_list.clear();
        self.zone_map.clear();
        for z in zones {
            self.zone_map.insert(z.name.clone(), self.zone_list.len());
            self.zone_list.push(z);
        }
    }

    pub fn zones(&self) -> &[ZzdZone] {
        &self.zone_list
    }

    pub fn outfit(&self, table: (usize, usize)) -> &ZzdOutfit {
        &self.zone_list[table.0].outfits[table.1]
    }

    /// `registerCustomOutfits()` (after `checkDirty`): the custom outfit
    /// names `PersistentOutfits` registers, in order.
    pub fn register_custom_outfits(&mut self) -> Vec<String> {
        self.custom_outfit_map.clear();
        let mut names = Vec::new();
        for (zi, z) in self.zone_list.iter().enumerate() {
            for (oi, o) in z.outfits.iter().enumerate() {
                names.push(o.custom_name.clone());
                self.custom_outfit_map.insert(o.custom_name.clone(), (zi, oi));
            }
        }
        names
    }

    /// `s_customOutfitMap.get(outfitName)`.
    pub fn custom_outfit(&self, custom_name: &str) -> Option<&ZzdOutfit> {
        self.custom_outfit_map.get(custom_name).map(|&t| self.outfit(t))
    }

    fn spawn_state(&mut self, key: u64) -> &mut ZoneSpawnState {
        self.zone_spawns.entry(key).or_default()
    }

    /// `getDefinitionZoneAt(x, y, z)`.
    pub fn get_definition_zone_at(&self, world: &dyn ZoneWorld, x: i32, y: i32, z: i32) -> Option<ZoneView> {
        let zones = world.zones_at(x, y, z);
        let mut zones2 = Vec::new();
        for zone in zones.iter().rev() {
            if zone.zone_type.as_deref().is_some_and(|t| eq_ignore_case("ZombiesType", t))
                && zone.name.as_ref().is_some_and(|n| self.zone_map.contains_key(n))
            {
                return Some(zone.clone());
            }
            if zone.zone_type.as_ref().is_some_and(|t| self.zone_map.contains_key(t)) {
                zones2.push(zone);
            }
        }
        zones2.into_iter().rev().find(|z| z.zone_type.as_ref().is_some_and(|t| self.zone_map.contains_key(t))).cloned()
    }

    /// `pickDefinition(x, y, z, bFemale, isOnSpawn)` (after `checkDirty`).
    pub fn pick_definition(&mut self, world: &dyn ZoneWorld, x: i32, y: i32, z: i32, female: bool, on_spawn: bool) -> Option<PickDefinition> {
        let room_name = world.square_room(x, y, z)?;
        let zone = self.get_definition_zone_at(world, x, y, z)?;
        if self.spawn_state(zone.key).spawn_special_zombies == Some(false) {
            return None;
        }
        let name = match &zone.name {
            Some(n) if !n.is_empty() => n.clone(),
            _ => zone.zone_type.clone().unwrap_or_default(),
        };
        let zi = *self.zone_map.get(&name)?;
        let table = &self.zone_list[zi];
        let (chance, max_to_spawn, male_chance, female_chance) = (table.chance_to_spawn, table.to_spawn, table.male_chance, table.female_chance);
        if chance != -1 {
            let already = self.spawned_zombie_zone.entry(zone.name.clone()).or_default();
            let contains = already.contains(&zone.id);
            let count = already.len() as i32;
            if contains {
                self.spawn_state(zone.key).spawn_special_zombies = Some(true);
            }
            let special = self.spawn_state(zone.key).spawn_special_zombies;
            if max_to_spawn == -1 || special.is_none() && count < max_to_spawn {
                if rand::next(100) < chance {
                    self.spawn_state(zone.key).spawn_special_zombies = Some(true);
                    self.spawned_zombie_zone.entry(zone.name.clone()).or_default().push(zone.id);
                } else {
                    self.spawn_state(zone.key).spawn_special_zombies = Some(false);
                    return None;
                }
            }
        }
        let mut female = female;
        if male_chance > 0 && rand::next(100) < male_chance {
            female = false;
        }
        if female_chance > 0 && rand::next(100) < female_chance {
            female = true;
        }
        let mut mandatory = Vec::new();
        let mut normal = Vec::new();
        for oi in 0..self.zone_list[zi].outfits.len() {
            let t = &self.zone_list[zi].outfits[oi];
            if let Some(room) = &t.room {
                if !room_name.as_ref().is_some_and(|r| room.contains(r.as_str())) {
                    continue;
                }
            }
            let g = t.gender.as_deref();
            if g.is_some_and(|g| eq_ignore_case("male", g)) && female || g.is_some_and(|g| eq_ignore_case("female", g)) && !female {
                continue;
            }
            let outfit_name = t.name.clone().unwrap_or_default();
            let to_spawn = t.to_spawn;
            if t.mandatory.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("true")) {
                let st = self.spawn_state(zone.key);
                let already = st.spawned_zombies.as_ref().map_or(0, |m| m.get(&outfit_name).copied().unwrap_or(0));
                if already >= to_spawn {
                    continue;
                }
                mandatory.push(oi);
                if !on_spawn {
                    continue;
                }
                match &mut st.spawned_zombies {
                    None => {
                        st.spawned_zombies = Some(HashMap::from([(outfit_name, 1)]));
                    }
                    Some(m) => {
                        m.insert(outfit_name, already + 1);
                    }
                }
                continue;
            }
            normal.push(oi);
        }
        let pick = if !mandatory.is_empty() {
            Some(mandatory[rand::next(mandatory.len() as i32) as usize])
        } else {
            let list: Vec<&ZzdOutfit> = normal.iter().map(|&i| &self.zone_list[zi].outfits[i]).collect();
            get_random_outfit_in_set_list(&list, true).map(|i| normal[i])
        }?;
        Some(PickDefinition { zone_key: zone.key, table: (zi, pick), female })
    }

    /// `applyDefinition`'s count of the outfit for the zone (when the
    /// outfit was found): `spawnedZombies[outfit] += 1`.
    pub fn count_spawned(&mut self, zone_key: u64, outfit_name: &str) {
        let st = self.spawn_state(zone_key);
        match &mut st.spawned_zombies {
            None => st.spawned_zombies = Some(HashMap::from([(outfit_name.to_string(), 1)])),
            Some(m) => {
                let n = m.get(outfit_name).copied().unwrap_or(0);
                m.insert(outfit_name.to_string(), n + 1);
            }
        }
    }

    /// `getRandomDefaultOutfit`'s pick from the `Default` zone: the
    /// `ZZDOutfit` name, if any. `Err` is the `NullPointerException`
    /// without a `Default` zone.
    pub fn random_default_outfit_name(&self, female: bool, room_name: Option<&str>) -> Result<Option<String>, String> {
        let zi = *self.zone_map.get("Default").ok_or("java.lang.NullPointerException")?;
        let mut list = Vec::new();
        for t in &self.zone_list[zi].outfits {
            if let Some(room) = &t.room {
                if !room_name.is_some_and(|r| room.contains(r)) {
                    continue;
                }
            }
            if let Some(g) = t.gender.as_deref() {
                let male_ok = eq_ignore_case("male", g) && !female;
                let female_ok = eq_ignore_case("female", g) && female;
                if !male_ok && !female_ok {
                    continue;
                }
            }
            list.push(t);
        }
        Ok(get_random_outfit_in_set_list(&list, false).and_then(|i| list[i].name.clone()))
    }
}

/// `getRandomOutfitInSetList(list, doTotalChance100)`: the index picked.
/// Both `Rand` rolls are made when the total is over 100 (or not
/// `doTotalChance100`), as the original does.
pub fn get_random_outfit_in_set_list(list: &[&ZzdOutfit], total_chance_100: bool) -> Option<usize> {
    let total: f32 = list.iter().fold(0.0f32, |a, o| a + o.chance);
    let mut choice = rand::next_float(0.0, 100.0);
    if !total_chance_100 || total > 100.0 {
        choice = rand::next_float(0.0, total);
    }
    let mut subtotal = 0.0f32;
    for (i, o) in list.iter().enumerate() {
        subtotal += o.chance;
        if choice < subtotal {
            return Some(i);
        }
    }
    None
}

/// `getRandomHairOrBeard(styles)` (`OutfitRNG`): `""` for `null`.
pub fn get_random_hair_or_beard(styles: &[StringChance]) -> Option<String> {
    let choice = outfit_rng::next_float(0.0, 100.0);
    let mut subtotal = 0.0f32;
    for s in styles {
        subtotal += s.chance;
        if choice < subtotal {
            if eq_ignore_case("null", &s.str) {
                return Some(String::new());
            }
            return Some(s.str.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct World(Vec<ZoneView>, Option<Option<String>>);
    impl ZoneWorld for World {
        fn square_room(&self, _x: i32, _y: i32, _z: i32) -> Option<Option<String>> {
            self.1.clone()
        }
        fn zones_at(&self, _x: i32, _y: i32, _z: i32) -> Vec<ZoneView> {
            self.0.clone()
        }
    }

    fn outfit(name: &str, chance: f32, gender: Option<&str>, mandatory: bool, to_spawn: i32) -> ZzdOutfit {
        ZzdOutfit::new(Some(name.into()), chance, gender.map(Into::into), to_spawn, mandatory.then(|| "true".into()), None, None, Some("Bald:50;null:50"), None)
            .unwrap()
    }

    #[test]
    fn zones_and_picks() {
        assert_eq!(init_string_chance(Some("a:1;b")).unwrap_err(), "java.lang.ArrayIndexOutOfBoundsException: Index 1 out of bounds for length 1");
        assert_eq!(blood_part_from_string("Back"), 17);
        assert_eq!(blood_part_from_string("x"), 18);
        let mut d = ZombiesZoneDefinition::default();
        d.set_zones(vec![
            ZzdZone::new("Army".into(), -1, -1, -1, -1, vec![outfit("ArmyCamoGreen", 100.0, None, false, -1)]),
            ZzdZone::new("Prison".into(), 0, 100, 50, 1, vec![outfit("Guard", 10.0, Some("male"), true, 1), outfit("Inmate", 90.0, None, false, -1)]),
            ZzdZone::new("Default".into(), -1, -1, -1, -1, vec![outfit("Generic01", 50.0, Some("female"), false, -1), outfit("Generic02", 50.0, None, false, -1)]),
        ]);
        assert_eq!(d.register_custom_outfits(), ["ZZD.Army.ArmyCamoGreen", "ZZD.Prison.Guard", "ZZD.Prison.Inmate", "ZZD.Default.Generic01", "ZZD.Default.Generic02"]);
        assert_eq!(d.custom_outfit("ZZD.Prison.Guard").unwrap().name.as_deref(), Some("Guard"));
        let army = ZoneView { key: 1, name: Some("".into()), zone_type: Some("Army".into()), id: Some(7) };
        let w = World(vec![army.clone()], Some(None));
        rand::set_seed(3);
        let p = d.pick_definition(&w, 0, 0, 0, true, true).unwrap();
        assert_eq!((p.table, p.female), ((0, 0), true));
        let prison = ZoneView { key: 2, name: Some("Prison".into()), zone_type: Some("ZombiesType".into()), id: Some(9) };
        let w = World(vec![army, prison], Some(None));
        let mut picks = Vec::new();
        for s in 0..20 {
            rand::set_seed(s);
            let mut d2 = d.clone();
            picks.push(d2.pick_definition(&w, 0, 0, 0, true, true).map(|p| (p.table, p.female, d2.zone_spawns[&2].clone())));
        }
        // Either the zone rolled its 50% and spawned the mandatory (male)
        // guard, or it turned special zombies off.
        assert!(picks.iter().any(Option::is_none));
        let (t, f, st) = picks.into_iter().flatten().next().unwrap();
        assert_eq!((t, f), ((1, 0), false));
        assert_eq!(st.spawned_zombies.unwrap()["Guard"], 1);
        assert!(World(vec![], None).square_room(0, 0, 0).is_none());
        rand::set_seed(1);
        assert!(d.random_default_outfit_name(false, None).unwrap().is_some_and(|n| n == "Generic02"));
        outfit_rng::set_seed(4);
        let hair = get_random_hair_or_beard(d.custom_outfit("ZZD.Army.ArmyCamoGreen").unwrap().male_hair_styles.as_ref().unwrap());
        assert!(matches!(hair.as_deref(), Some("Bald") | Some("")));
    }

    #[test]
    fn stages_underwear_weapons() {
        assert_eq!(advanced_outfit_name("Survivalist02", 31.0).as_deref(), Some("Survivalist02_Mid"));
        assert_eq!(advanced_outfit_name("Survivalist02", 3.0), None);
        assert_eq!(advanced_outfit_name("Police", 300.0), None);
        rand::set_seed(2);
        assert!(advanced_outfit_name("Bandit", 100.0).is_some_and(|n| n.starts_with("Bandit_")));
        let u = UnderwearDefinition {
            dirty: false,
            base_chance: 100,
            outfit_definition: vec![OutfitUnderwearDefinition::new(
                Some(vec![StringChance { str: "Base.Bra".into(), chance: 30.5 }]),
                Some("Base.Briefs".into()),
                10,
                Some("female"),
            )],
        };
        assert_eq!(u.outfit_definition[0].top_total_chance, 30);
        outfit_rng::set_seed(9);
        let p = u.pick(true).unwrap();
        assert_eq!(p.bottom.as_deref(), Some("Base.Briefs"));
        assert!(u.pick(false).is_none());
        let a = AttachedWeaponDefinitions {
            dirty: false,
            chance_of_attached_weapon: 100,
            definitions: vec![AttachedWeaponDefinition { id: "a".into(), chance: 5, outfit: vec!["Police".into()], ..Default::default() }],
            outfit_definitions: vec![],
        };
        let r = a.rounds(Some("Police")).unwrap();
        assert_eq!(AttachedWeaponDefinitions::pick_random_in_list(&r.pool, Some("Police"), 0.0), Some(0));
        assert_eq!(AttachedWeaponDefinitions::pick_random_in_list(&r.pool, Some("Cop"), 0.0), None);
    }
}
