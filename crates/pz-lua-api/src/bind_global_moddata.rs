//! Build 42.22 Lua bindings and persistence for:
//! - `zombie.ZomboidGlobals` (`Load`, `toLua`)
//! - `zombie.world.moddata.GlobalModData` (`global_mod_data.bin` persistence + `OnInitGlobalModData`)
//! - `zombie.world.moddata.ModData` (static Lua API over `GlobalModData`)

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_characters::character::ZomboidGlobals;
use pz_kahlua::java_io::{JavaReader, JavaWrite};
use pz_kahlua::table_save::{load_table, save_table, Flavor};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, TableRef, Thread, Value};

use crate::bind_java_util::JavaList;
use crate::events::EventManager;
use crate::expose::expose_class;
use crate::global_objects_save::WORLD_VERSION;

const ZOMBOID_GLOBALS_CLASS: &str = "zombie.ZomboidGlobals";
const GLOBAL_MOD_DATA_CLASS: &str = "zombie.world.moddata.GlobalModData";
const MOD_DATA_CLASS: &str = "zombie.world.moddata.ModData";

/// Build 42.22 `GlobalModData.SAVE_FILE` + `SAVE_EXT`.
pub const GLOBAL_MOD_DATA_FILE: &str = "global_mod_data.bin";
const GLOBAL_MOD_DATA_TMP: &str = "global_mod_data.tmp";

/// Extra scalar fields on `zombie.ZomboidGlobals` that are not stored in
/// `pz_characters::character::ZomboidGlobals`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExtraZomboidGlobals {
    pub clean_blood_bleach_amount: f64,
    pub refill_blowtorch_propane_amount: f64,
    pub clean_stain_cleaning_fluid_amount: f64,
}

#[derive(Default)]
struct GlobalModDataStore {
    tables: BTreeMap<String, TableRef>,
    save_dir: Option<PathBuf>,
    no_save: bool,
    uuid_counter: u64,
}

impl GlobalModDataStore {
    fn next_uuid(&mut self) -> String {
        self.uuid_counter = self.uuid_counter.wrapping_add(1);
        let n = self.uuid_counter;
        format!("00000000-0000-4000-8000-{n:012x}")
    }
}

thread_local! {
    static ZOMBOID_GLOBALS: RefCell<ZomboidGlobals> = RefCell::new(ZomboidGlobals::INITIAL);
    static EXTRA_GLOBALS: RefCell<ExtraZomboidGlobals> = RefCell::new(ExtraZomboidGlobals::default());
    static GLOBAL_MOD_DATA: RefCell<GlobalModDataStore> = RefCell::new(GlobalModDataStore::default());
    static EVENTS: RefCell<Option<EventManager>> = const { RefCell::new(None) };
    static IS_CLIENT: Cell<bool> = const { Cell::new(false) };
}

/// Snapshot of the current loaded `ZomboidGlobals`.
pub fn current_zomboid_globals() -> ZomboidGlobals {
    ZOMBOID_GLOBALS.with(|g| g.borrow().clone())
}

/// Reset session state on world teardown.
pub fn reset() {
    ZOMBOID_GLOBALS.with(|g| *g.borrow_mut() = ZomboidGlobals::INITIAL);
    EXTRA_GLOBALS.with(|g| *g.borrow_mut() = ExtraZomboidGlobals::default());
    GLOBAL_MOD_DATA.with(|s| {
        let mut s = s.borrow_mut();
        s.tables.clear();
        s.save_dir = None;
        s.no_save = false;
    });
    EVENTS.with(|e| *e.borrow_mut() = None);
}

fn read_num_field(tbl: &TableRef, key: &str) -> Option<f64> {
    match tbl.get_str(key) {
        Value::Num(n) if n.is_finite() => Some(n),
        _ => None,
    }
}

/// `ZomboidGlobals.Load()`: reads numeric fields from the Lua global
/// `ZomboidGlobals` table into the native `ZomboidGlobals` struct.
pub fn load_zomboid_globals(thread: &Thread) -> bool {
    let Value::Table(tbl) = thread.globals().get_str("ZomboidGlobals") else {
        return false;
    };
    ZOMBOID_GLOBALS.with(|slot| {
        let mut g = slot.borrow_mut();
        macro_rules! load_f64 {
            ($field:ident, $key:expr) => {
                if let Some(v) = read_num_field(&tbl, $key) {
                    g.$field = v;
                }
            };
        }
        load_f64!(sprinting_endurance_reduce, "SprintingEnduranceReduce");
        load_f64!(running_endurance_reduce, "RunningEnduranceReduce");
        load_f64!(imobile_endurance_reduce, "ImobileEnduranceIncrease");
        load_f64!(thirst_increase, "ThirstIncrease");
        load_f64!(thirst_sleeping_increase, "ThirstSleepingIncrease");
        load_f64!(thirst_level_to_auto_drink, "ThirstLevelToAutoDrink");
        load_f64!(
            thirst_level_reduction_on_auto_drink,
            "ThirstLevelReductionOnAutoDrink"
        );
        load_f64!(hunger_increase, "HungerIncrease");
        load_f64!(hunger_increase_when_well_fed, "HungerIncreaseWhenWellFed");
        load_f64!(hunger_increase_while_asleep, "HungerIncreaseWhileAsleep");
        load_f64!(hunger_increase_when_exercise, "HungerIncreaseWhenExercise");
        load_f64!(fatigue_increase, "FatigueIncrease");
        load_f64!(stress_reduction, "StressDecrease");
        load_f64!(boredom_increase_rate, "BoredomIncrease");
        load_f64!(boredom_decrease_rate, "BoredomDecrease");
        load_f64!(unhappiness_increase, "UnhappinessIncrease");
        load_f64!(stress_from_sounds_multiplier, "StressFromSoundsMultiplier");
        load_f64!(stress_from_bite_or_scratch, "StressFromBiteOrScratch");
        load_f64!(stress_from_hemophobic, "StressFromHemophobic");
        load_f64!(stress_from_discomfort, "StressFromDiscomfort");
        load_f64!(anger_decrease, "AngerDecrease");
        load_f64!(sleep_fatigue_reduction, "SleepFatigueReduction");
        load_f64!(wetness_increase, "WetnessIncrease");
        load_f64!(wetness_decrease, "WetnessDecrease");
        load_f64!(catch_a_cold_increase_rate, "CatchAColdIncreaseRate");
        load_f64!(catch_a_cold_decrease_rate, "CatchAColdDecreaseRate");
        load_f64!(poison_level_decrease, "PoisonLevelDecrease");
        load_f64!(poison_health_reduction, "PoisonHealthReduction");
        load_f64!(food_sickness_decrease, "FoodSicknessDecrease");
        load_f64!(idle_increase_rate, "IdleIncrease");
        load_f64!(idle_decrease_rate, "IdleDecrease");
        load_f64!(
            vehicle_discomfort_when_over_encumbered,
            "VehicleDiscomfortWhenOverEncumbered"
        );
        load_f64!(
            equipped_or_worn_encumbrance_multiplier,
            "EquippedOrWornEncumbranceMultiplier"
        );
        load_f64!(
            corpse_sickness_filter_drain_multiplier,
            "CorpseSicknessFilterDrainMultiplier"
        );
    });
    EXTRA_GLOBALS.with(|slot| {
        let mut e = slot.borrow_mut();
        if let Some(v) = read_num_field(&tbl, "CleanBloodBleachAmount") {
            e.clean_blood_bleach_amount = v;
        }
        if let Some(v) = read_num_field(&tbl, "RefillBlowtorchPropaneAmount") {
            e.refill_blowtorch_propane_amount = v;
        }
        if let Some(v) = read_num_field(&tbl, "CleanStainCleaningFluidAmount") {
            e.clean_stain_cleaning_fluid_amount = v;
        }
    });
    true
}

/// `ZomboidGlobals.toLua()`: pushes native `ZomboidGlobals` values back onto
/// the Lua global `ZomboidGlobals` table (creating it if absent).
pub fn zomboid_globals_to_lua(thread: &Thread) {
    let tbl = match thread.globals().get_str("ZomboidGlobals") {
        Value::Table(t) => t,
        _ => {
            let t = TableRef::new();
            thread.globals().set_str("ZomboidGlobals", Value::Table(t.clone()));
            t
        }
    };
    ZOMBOID_GLOBALS.with(|slot| {
        let g = slot.borrow();
        let set = |k: &str, v: f64| {
            tbl.set_str(k, Value::Num(v));
        };
        set("SprintingEnduranceReduce", g.sprinting_endurance_reduce);
        set("RunningEnduranceReduce", g.running_endurance_reduce);
        set("ImobileEnduranceIncrease", g.imobile_endurance_reduce);
        set("ThirstIncrease", g.thirst_increase);
        set("ThirstSleepingIncrease", g.thirst_sleeping_increase);
        set("ThirstLevelToAutoDrink", g.thirst_level_to_auto_drink);
        set(
            "ThirstLevelReductionOnAutoDrink",
            g.thirst_level_reduction_on_auto_drink,
        );
        set("HungerIncrease", g.hunger_increase);
        set("HungerIncreaseWhenWellFed", g.hunger_increase_when_well_fed);
        set("HungerIncreaseWhileAsleep", g.hunger_increase_while_asleep);
        set("HungerIncreaseWhenExercise", g.hunger_increase_when_exercise);
        set("FatigueIncrease", g.fatigue_increase);
        set("StressDecrease", g.stress_reduction);
        set("BoredomIncrease", g.boredom_increase_rate);
        set("BoredomDecrease", g.boredom_decrease_rate);
        set("UnhappinessIncrease", g.unhappiness_increase);
        set("StressFromSoundsMultiplier", g.stress_from_sounds_multiplier);
        set("StressFromBiteOrScratch", g.stress_from_bite_or_scratch);
        set("StressFromHemophobic", g.stress_from_hemophobic);
        set("StressFromDiscomfort", g.stress_from_discomfort);
        set("AngerDecrease", g.anger_decrease);
        set("SleepFatigueReduction", g.sleep_fatigue_reduction);
        set("WetnessIncrease", g.wetness_increase);
        set("WetnessDecrease", g.wetness_decrease);
        set("CatchAColdIncreaseRate", g.catch_a_cold_increase_rate);
        set("CatchAColdDecreaseRate", g.catch_a_cold_decrease_rate);
        set("PoisonLevelDecrease", g.poison_level_decrease);
        set("PoisonHealthReduction", g.poison_health_reduction);
        set("FoodSicknessDecrease", g.food_sickness_decrease);
        set("IdleIncrease", g.idle_increase_rate);
        set("IdleDecrease", g.idle_decrease_rate);
        set(
            "VehicleDiscomfortWhenOverEncumbered",
            g.vehicle_discomfort_when_over_encumbered,
        );
        set(
            "EquippedOrWornEncumbranceMultiplier",
            g.equipped_or_worn_encumbrance_multiplier,
        );
        set(
            "CorpseSicknessFilterDrainMultiplier",
            g.corpse_sickness_filter_drain_multiplier,
        );
    });
    EXTRA_GLOBALS.with(|slot| {
        let e = slot.borrow();
        tbl.set_str(
            "CleanBloodBleachAmount",
            Value::Num(e.clean_blood_bleach_amount),
        );
        tbl.set_str(
            "RefillBlowtorchPropaneAmount",
            Value::Num(e.refill_blowtorch_propane_amount),
        );
        tbl.set_str(
            "CleanStainCleaningFluidAmount",
            Value::Num(e.clean_stain_cleaning_fluid_amount),
        );
    });
}

/// Serialize all `GlobalModData` tables to Build 42.22 `global_mod_data.bin` bytes.
pub fn save_bytes() -> io::Result<Vec<u8>> {
    GLOBAL_MOD_DATA.with(|store| {
        let store = store.borrow();
        let mut out = Vec::new();
        out.put_i32(WORLD_VERSION);
        out.put_i32(store.tables.len() as i32);
        for (tag, tbl) in &store.tables {
            out.write_string_utf(Some(tag));
            save_table(tbl, &mut out, Flavor::ByteBuffer)?;
        }
        Ok(out)
    })
}

/// Load `GlobalModData` tables from Build 42.22 `global_mod_data.bin` bytes.
pub fn load_bytes(data: &[u8]) -> io::Result<()> {
    let mut r = JavaReader::new(data);
    let world_version = r.get_i32()?;
    let count = r.get_i32()?;
    let mut loaded = BTreeMap::new();
    for _ in 0..count.max(0) {
        let tag = r.read_string_utf()?;
        let tbl = TableRef::new();
        load_table(&tbl, &mut r, world_version, Flavor::ByteBuffer)?;
        loaded.insert(tag, tbl);
    }
    GLOBAL_MOD_DATA.with(|store| {
        store.borrow_mut().tables = loaded;
    });
    Ok(())
}

/// Save `global_mod_data.bin` atomically into `save_dir` unless `no_save`.
pub fn save_world(save_dir: &Path, no_save: bool) -> io::Result<()> {
    if no_save {
        return Ok(());
    }
    let bytes = save_bytes()?;
    std::fs::create_dir_all(save_dir)?;
    let tmp = save_dir.join(GLOBAL_MOD_DATA_TMP);
    let dst = save_dir.join(GLOBAL_MOD_DATA_FILE);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &dst)?;
    Ok(())
}

/// Load `global_mod_data.bin` from `save_dir` if present; returns `Ok(true)` if
/// a file was loaded, `Ok(false)` if this is a fresh save or `no_save`.
pub fn load_world(save_dir: &Path, no_save: bool) -> io::Result<bool> {
    if no_save {
        return Ok(false);
    }
    let path = save_dir.join(GLOBAL_MOD_DATA_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    load_bytes(&bytes)?;
    Ok(true)
}

/// `GlobalModData.instance.init()` called from `IsoWorld::init`: clears old
/// tables, loads `global_mod_data.bin` if present, and fires
/// `Events.OnInitGlobalModData(isNewGame)`.
pub fn init_world(thread: &mut Thread, save_dir: &Path, no_save: bool) -> io::Result<()> {
    GLOBAL_MOD_DATA.with(|store| {
        let mut s = store.borrow_mut();
        s.tables.clear();
        s.save_dir = Some(save_dir.to_path_buf());
        s.no_save = no_save;
    });
    let loaded = load_world(save_dir, no_save)?;
    let is_new_game = !loaded;
    let sink = EVENTS.with(|e| e.borrow().clone());
    if let Some(events) = sink {
        events.trigger(thread, "OnInitGlobalModData", &[Value::Bool(is_new_game)]);
    }
    Ok(())
}

#[derive(Debug)]
struct GlobalModDataHandle;

impl LuaObject for GlobalModDataHandle {
    fn class_name(&self) -> &'static str {
        GLOBAL_MOD_DATA_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn str_arg(c: &NativeCall<'_>, idx: usize) -> Option<String> {
    c.arg(idx).as_str().map(str::to_string)
}

fn moddata_table_names() -> Value {
    let items = GLOBAL_MOD_DATA.with(|store| {
        store
            .borrow()
            .tables
            .keys()
            .map(|k| Value::str(k))
            .collect::<Vec<_>>()
    });
    Value::Object(Rc::new(JavaList::new(items)))
}

fn moddata_exists(tag: Option<String>) -> Value {
    let ok = tag.is_some_and(|t| GLOBAL_MOD_DATA.with(|s| s.borrow().tables.contains_key(&t)));
    Value::Bool(ok)
}

fn moddata_get_or_create(tag: Option<String>) -> Value {
    let Some(tag) = tag else {
        return Value::Nil;
    };
    let tbl = GLOBAL_MOD_DATA.with(|s| {
        s.borrow_mut()
            .tables
            .entry(tag)
            .or_insert_with(TableRef::new)
            .clone()
    });
    Value::Table(tbl)
}

fn moddata_get(tag: Option<String>) -> Value {
    let Some(tag) = tag else {
        return Value::Nil;
    };
    GLOBAL_MOD_DATA.with(|s| {
        s.borrow()
            .tables
            .get(&tag)
            .cloned()
            .map(Value::Table)
            .unwrap_or(Value::Nil)
    })
}

fn moddata_create(tag: Option<String>) -> Value {
    GLOBAL_MOD_DATA.with(|s| {
        let mut store = s.borrow_mut();
        match tag {
            None => {
                let uuid = store.next_uuid();
                store.tables.insert(uuid.clone(), TableRef::new());
                Value::str(&uuid)
            }
            Some(t) => {
                if store.tables.contains_key(&t) {
                    Value::Nil
                } else {
                    let tbl = TableRef::new();
                    store.tables.insert(t, tbl.clone());
                    Value::Table(tbl)
                }
            }
        }
    })
}

fn moddata_remove(tag: Option<String>) -> Value {
    let Some(tag) = tag else {
        return Value::Nil;
    };
    GLOBAL_MOD_DATA.with(|s| {
        s.borrow_mut()
            .tables
            .remove(&tag)
            .map(Value::Table)
            .unwrap_or(Value::Nil)
    })
}

fn moddata_add(tag: Option<String>, val: Value) {
    if let (Some(tag), Value::Table(tbl)) = (tag, val) {
        GLOBAL_MOD_DATA.with(|s| {
            s.borrow_mut().tables.insert(tag, tbl);
        });
    }
}

fn moddata_transmit(thread: &mut Thread, tag: Option<String>) {
    let Some(tag) = tag else {
        return;
    };
    if IS_CLIENT.with(Cell::get) {
        return;
    }
    let tbl = GLOBAL_MOD_DATA.with(|s| s.borrow().tables.get(&tag).cloned());
    if let Some(tbl) = tbl {
        let sink = EVENTS.with(|e| e.borrow().clone());
        if let Some(events) = sink {
            events.trigger(
                thread,
                "OnReceiveGlobalModData",
                &[Value::str(&tag), Value::Table(tbl)],
            );
        }
    }
}

/// Register `ZomboidGlobals`, `GlobalModData`, and `ModData`.
pub fn register(thread: &mut Thread, events: Option<EventManager>) {
    EVENTS.with(|e| *e.borrow_mut() = events);

    let zg = expose_class(thread, ZOMBOID_GLOBALS_CLASS, Some("java.lang.Object"));
    zg.static_fn("Load", |c| {
        load_zomboid_globals(c.thread);
        Ok(0)
    });
    zg.static_fn("toLua", |c| {
        zomboid_globals_to_lua(c.thread);
        Ok(0)
    });

    let gmd = expose_class(thread, GLOBAL_MOD_DATA_CLASS, Some("java.lang.Object"));
    gmd.field("instance", Value::Object(Rc::new(GlobalModDataHandle)));
    gmd.method("init", |c| {
        let (dir, no_save) = GLOBAL_MOD_DATA.with(|s| {
            let s = s.borrow();
            (s.save_dir.clone(), s.no_save)
        });
        if let Some(dir) = dir {
            let _ = init_world(c.thread, &dir, no_save);
        } else {
            GLOBAL_MOD_DATA.with(|s| s.borrow_mut().tables.clear());
            let sink = EVENTS.with(|e| e.borrow().clone());
            if let Some(events) = sink {
                events.trigger(c.thread, "OnInitGlobalModData", &[Value::Bool(true)]);
            }
        }
        Ok(0)
    });
    gmd.method("reset", |_| {
        GLOBAL_MOD_DATA.with(|s| s.borrow_mut().tables.clear());
        Ok(0)
    });
    gmd.method("collectTableNames", |c| {
        if let Value::Object(obj) = c.arg(1) {
            if let Some(list) = obj.as_any().downcast_ref::<JavaList>() {
                let mut items = list.items.borrow_mut();
                GLOBAL_MOD_DATA.with(|s| {
                    for k in s.borrow().tables.keys() {
                        items.push(Value::str(k));
                    }
                });
            }
        }
        Ok(0)
    });
    gmd.method("exists", |c| Ok(c.push(moddata_exists(str_arg(c, 1)))));
    gmd.method("getOrCreate", |c| {
        Ok(c.push(moddata_get_or_create(str_arg(c, 1))))
    });
    gmd.method("get", |c| Ok(c.push(moddata_get(str_arg(c, 1)))));
    gmd.method("create", |c| Ok(c.push(moddata_create(str_arg(c, 1)))));
    gmd.method("remove", |c| Ok(c.push(moddata_remove(str_arg(c, 1)))));
    gmd.method("add", |c| {
        moddata_add(str_arg(c, 1), c.arg(2));
        Ok(0)
    });
    gmd.method("transmit", |c| {
        let tag = str_arg(c, 1);
        moddata_transmit(c.thread, tag);
        Ok(0)
    });
    gmd.method("request", |_| Ok(0));
    gmd.method("save", |_| {
        let (dir, no_save) = GLOBAL_MOD_DATA.with(|s| {
            let s = s.borrow();
            (s.save_dir.clone(), s.no_save)
        });
        if let Some(dir) = dir {
            let _ = save_world(&dir, no_save);
        }
        Ok(0)
    });
    gmd.method("load", |_| {
        let (dir, no_save) = GLOBAL_MOD_DATA.with(|s| {
            let s = s.borrow();
            (s.save_dir.clone(), s.no_save)
        });
        if let Some(dir) = dir {
            let _ = load_world(&dir, no_save);
        }
        Ok(0)
    });

    let md = expose_class(thread, MOD_DATA_CLASS, Some("java.lang.Object"));
    md.static_fn("getTableNames", |c| Ok(c.push(moddata_table_names())));
    md.static_fn("exists", |c| Ok(c.push(moddata_exists(str_arg(c, 0)))));
    md.static_fn("getOrCreate", |c| {
        Ok(c.push(moddata_get_or_create(str_arg(c, 0))))
    });
    md.static_fn("get", |c| Ok(c.push(moddata_get(str_arg(c, 0)))));
    md.static_fn("create", |c| Ok(c.push(moddata_create(str_arg(c, 0)))));
    md.static_fn("remove", |c| Ok(c.push(moddata_remove(str_arg(c, 0)))));
    md.static_fn("add", |c| {
        moddata_add(str_arg(c, 0), c.arg(1));
        Ok(0)
    });
    md.static_fn("transmit", |c| {
        let tag = str_arg(c, 0);
        moddata_transmit(c.thread, tag);
        Ok(0)
    });
    md.static_fn("request", |_| Ok(0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zomboid_globals_and_mod_data_persistence_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        let events = EventManager::register(t.globals(), "Events", crate::events::EVENT_NAMES);
        register(&mut t, Some(events));

        let dir = std::env::temp_dir().join(format!("pz_global_mod_data_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            ZomboidGlobals.RunningEnduranceReduce = 0.0052
            ZomboidGlobals.HungerIncrease = 0.0032
            ZomboidGlobals.Load()

            local init_new = nil
            Events.OnInitGlobalModData.Add(function(isNew) init_new = isNew end)
            local recv_tag = nil
            local recv_val = nil
            Events.OnReceiveGlobalModData.Add(function(tag, tbl)
                recv_tag = tag
                recv_val = tbl.score
            end)

            assert(not ModData.exists("faction_stats"))
            local tbl = ModData.getOrCreate("faction_stats")
            tbl.score = 42
            tbl.label = "west_point"
            assert(ModData.exists("faction_stats"))
            assert(ModData.get("faction_stats").score == 42)
            assert(ModData.create("faction_stats") == nil)
            local auto_id = ModData.create()
            assert(type(auto_id) == "string" and ModData.exists(auto_id))
            assert(ModData.getTableNames():size() == 2)
            ModData.transmit("faction_stats")
            assert(recv_tag == "faction_stats" and recv_val == 42)
            local removed = ModData.remove(auto_id)
            assert(removed ~= nil and not ModData.exists(auto_id))
            "#,
            Some("global_moddata_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));

        let g = current_zomboid_globals();
        assert!((g.running_endurance_reduce - 0.0052).abs() < 1e-9);
        assert!((g.hunger_increase - 0.0032).abs() < 1e-9);

        save_world(&dir, false).unwrap();
        assert!(dir.join(GLOBAL_MOD_DATA_FILE).exists());

        init_world(&mut t, &dir, false).unwrap();
        let verify = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            assert(ModData.exists("faction_stats"))
            local tbl = ModData.get("faction_stats")
            assert(tbl.score == 42 and tbl.label == "west_point")
            "#,
            Some("global_moddata_reload_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&verify, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));

        let _ = std::fs::remove_dir_all(&dir);
        reset();
    }
}
