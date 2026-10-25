//! `VehicleType`: the vehicle spawn lists of the map zones, read from the
//! Lua table `VehicleZoneDistribution`.

use crate::rand;
use pz_kahlua::{TableRef, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// `VehicleType.VehicleTypeDefinition`.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleTypeDefinition {
    pub vehicle_type: String,
    pub index: i32,
    pub spawn_chance: f32,
}

/// `VehicleType`.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleType {
    pub vehicles_definition: Vec<VehicleTypeDefinition>,
    pub chance_to_spawn_normal: i32,
    pub chance_to_spawn_burnt: i32,
    pub spawn_rate: i32,
    pub chance_of_over_car: i32,
    pub random_angle: bool,
    pub base_vehicle_quality: f32,
    pub name: String,
    pub chance_to_spawn_key: i32,
    pub chance_to_part_damage: i32,
    pub is_special_car: bool,
    pub is_burnt_car: bool,
    pub chance_to_spawn_special: i32,
    pub force_spawn: bool,
}

impl VehicleType {
    pub fn new(name: &str) -> Self {
        VehicleType {
            vehicles_definition: Vec::new(),
            chance_to_spawn_normal: 80,
            chance_to_spawn_burnt: 0,
            spawn_rate: 16,
            chance_of_over_car: 0,
            random_angle: false,
            base_vehicle_quality: 1.0,
            name: name.to_string(),
            chance_to_spawn_key: 70,
            chance_to_part_damage: 0,
            is_special_car: false,
            is_burnt_car: false,
            chance_to_spawn_special: 5,
            force_spawn: false,
        }
    }

    /// `getRandomBaseVehicleQuality()`.
    pub fn random_base_vehicle_quality(&self) -> f32 {
        rand::next_float(
            self.base_vehicle_quality - 0.1,
            self.base_vehicle_quality + 0.1,
        )
    }
}

#[derive(Default)]
struct Registry {
    /// `vehicles` (a `HashMap`; iteration order is not used by the ported
    /// code).
    vehicles: HashMap<String, Rc<VehicleType>>,
    /// `specialVehicles` in `VehicleZoneDistribution` order.
    special: Vec<Rc<VehicleType>>,
}

thread_local! {
    static REG: RefCell<Registry> = RefCell::new(Registry::default());
}

/// `KahluaTableImpl.rawgetInt`.
fn raw_int(t: &TableRef, k: &str) -> i32 {
    match t.get_str(k) {
        Value::Num(d) => d as i32,
        _ => -1,
    }
}

/// `KahluaTableImpl.rawgetFloat`.
fn raw_float(t: &TableRef, k: &str) -> f32 {
    match t.get_str(k) {
        Value::Num(d) => d as f32,
        _ => -1.0,
    }
}

/// `KahluaTableImpl.rawgetBool`.
fn raw_bool(t: &TableRef, k: &str) -> bool {
    matches!(t.get_str(k), Value::Bool(true))
}

fn key_string(k: &Value) -> String {
    match k {
        Value::Str(s) => s.to_string(),
        Value::Num(n) => pz_kahlua::number::number_to_string(*n),
        other => format!("{other:?}"),
    }
}

/// `VehicleType.init()` from `VehicleZoneDistribution`.
pub fn init(distribution: &TableRef) {
    let mut reg = Registry::default();
    for (zk, zv) in distribution.entries() {
        let zone_type = key_string(&zk);
        let Value::Table(zone) = zv else { continue };
        let mut t = VehicleType::new(&zone_type);
        if let Value::Table(cars) = zone.get_str("vehicles") {
            for (ck, cv) in cars.entries() {
                let Value::Table(def) = cv else { continue };
                t.vehicles_definition.push(VehicleTypeDefinition {
                    vehicle_type: key_string(&ck),
                    index: raw_int(&def, "index"),
                    spawn_chance: raw_float(&def, "spawnChance"),
                });
            }
        }
        let mut summ = 0.0f32;
        for d in &t.vehicles_definition {
            summ += d.spawn_chance;
        }
        let summ = 100.0 / summ;
        for d in &mut t.vehicles_definition {
            d.spawn_chance *= summ;
        }
        let has = |k: &str| zone.contains_own(&Value::str(k));
        if has("chanceToPartDamage") {
            t.chance_to_part_damage = raw_int(&zone, "chanceToPartDamage");
        }
        if has("chanceToSpawnNormal") {
            t.chance_to_spawn_normal = raw_int(&zone, "chanceToSpawnNormal");
        }
        if has("chanceToSpawnSpecial") {
            t.chance_to_spawn_special = raw_int(&zone, "chanceToSpawnSpecial");
        }
        if has("specialCar") {
            t.is_special_car = raw_bool(&zone, "specialCar");
        }
        if has("burntCar") {
            t.is_burnt_car = raw_bool(&zone, "burntCar");
        }
        if has("baseVehicleQuality") {
            t.base_vehicle_quality = raw_float(&zone, "baseVehicleQuality");
        }
        if has("chanceOfOverCar") {
            t.chance_of_over_car = raw_int(&zone, "chanceOfOverCar");
        }
        if has("randomAngle") {
            t.random_angle = raw_bool(&zone, "randomAngle");
        }
        if has("spawnRate") {
            t.spawn_rate = raw_int(&zone, "spawnRate");
        }
        if has("chanceToSpawnKey") {
            t.chance_to_spawn_key = raw_int(&zone, "chanceToSpawnKey");
        }
        if has("chanceToSpawnBurnt") {
            t.chance_to_spawn_burnt = raw_int(&zone, "chanceToSpawnBurnt");
        }
        if has("forceSpawn") {
            t.force_spawn = raw_bool(&zone, "forceSpawn");
        }
        let t = Rc::new(t);
        reg.vehicles.insert(zone_type, t.clone());
        if t.is_special_car {
            reg.special.push(t);
        }
    }
    REG.with(|r| *r.borrow_mut() = reg);
}

/// `VehicleType.Reset()`.
pub fn reset() {
    REG.with(|r| *r.borrow_mut() = Registry::default());
}

fn ensure_init() {
    if REG.with(|r| r.borrow().vehicles.is_empty()) {
        if let Some(Value::Table(t)) = crate::lua::global("VehicleZoneDistribution") {
            init(&t);
        }
    }
}

/// `VehicleType.getTypeFromName(name)`.
pub fn type_from_name(name: &str) -> Option<Rc<VehicleType>> {
    ensure_init();
    REG.with(|r| r.borrow().vehicles.get(name).cloned())
}

/// `VehicleType.hasTypeForZone(zoneName)`.
pub fn has_type_for_zone(zone_name: &str) -> bool {
    ensure_init();
    REG.with(|r| r.borrow().vehicles.contains_key(&zone_name.to_lowercase()))
}

/// `VehicleType.getRandomVehicleType(zoneName, doNormalWhenSpecific)`.
pub fn random_vehicle_type(
    zone_name: &str,
    do_normal_when_specific: bool,
) -> Option<Rc<VehicleType>> {
    ensure_init();
    let zone_name = zone_name.to_lowercase();
    let get = |n: &str| REG.with(|r| r.borrow().vehicles.get(n).cloned());
    let mut t = get(&zone_name)?;
    if rand::next(100) < t.chance_to_spawn_burnt {
        return if rand::next(100) < 80 {
            get("normalburnt")
        } else {
            get("specialburnt")
        };
    }
    if do_normal_when_specific && t.is_special_car && rand::next(100) < t.chance_to_spawn_normal {
        t = get("parkingstall")?;
    }
    if !t.is_burnt_car && !t.is_special_car && rand::next(100) < t.chance_to_spawn_special {
        let special = REG.with(|r| r.borrow().special.clone());
        if special.is_empty() {
            return None;
        }
        t = special[rand::next(special.len() as i32) as usize].clone();
    }
    if t.is_burnt_car {
        return if rand::next(100) < 80 {
            get("normalburnt")
        } else {
            get("specialburnt")
        };
    }
    Some(t)
}
