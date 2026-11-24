//! Additional verified unbound Build 42.22 `LuaManager$GlobalObject`
//! zombie-area spawning, fake-attacker, and SP-safe client/UI helpers:
//! - `addZombiesInOutfitArea(x1, y1, x2, y2, z, totalZombies, outfit, femaleChance)`
//! - `addZombiesInBuilding(buildingDef, totalZombies, outfit, roomDef, femaleChance)`
//! - `getFakeAttacker()`
//! - `Render3DItem(item, sq, x, y, z, rot)`
//! - `showVehicleEditor(scriptName)`
//! - `toggleStatisticsTransmission()`, `getTickets(author)`,
//!   `getBannedIPs()`, `getBannedSteamIDs()`,
//!   `tradingUISendUpdateState(playerA, playerB, state)`,
//!   `sendWarManagerUpdate(id, attacker, state)`,
//!   `sendSafehouseChangeMember(safehouse, username)`,
//!   `sendFactionChangeTag(faction)`,
//!   `sendRemoveAndGrabAnimalFromTrailer(animalOrBody, player, vehicle, item)`

use crate::bind_character_sim::{character_value, LuaGameCharacterRef};
use crate::bind_characters::{shared_characters, LuaItem};
use crate::bind_java_util::JavaList;
use crate::bind_registry::shared_script_manager;
use crate::expose::{global, int_arg, str_arg};
use pz_characters::character::zombie::ZombieSpawn;
use pz_characters::character::{DefaultSimContext, GameCharacter};
use pz_characters::survivor::SurvivorDesc;
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static FAKE_ATTACKER: RefCell<Option<LuaGameCharacterRef>> = const { RefCell::new(None) };
    static SPAWNED_AREA_ZOMBIES: RefCell<Vec<LuaGameCharacterRef>> = const { RefCell::new(Vec::new()) };
    static STATS_TRANSMISSION: RefCell<bool> = const { RefCell::new(false) };
}

pub(crate) fn reset_session_handles() {
    FAKE_ATTACKER.with(|f| *f.borrow_mut() = None);
    SPAWNED_AREA_ZOMBIES.with(|z| z.borrow_mut().clear());
    STATS_TRANSMISSION.with(|s| *s.borrow_mut() = false);
}

fn spawn_area_zombie(x: f32, y: f32, z: f32, female: Option<bool>) -> LuaResult<LuaGameCharacterRef> {
    let sm = shared_script_manager();
    let chars = shared_characters();
    let ctx = DefaultSimContext;
    let mut create_survivor = || {
        let mut ch = chars.borrow_mut();
        let mut d = SurvivorDesc::<LuaItem>::new(&mut ch, &sm)?;
        if let Some(f) = female {
            d.set_female(f);
        }
        Ok(Rc::new(RefCell::new(d)))
    };
    let mut zombie = GameCharacter::new_zombie(
        &sm,
        &ctx,
        ZombieSpawn {
            descriptor: None,
            create_survivor: &mut create_survivor,
        },
    )
    .map_err(|e| LuaError::runtime(format!("addZombiesInOutfitArea failed: {e}")))?;
    zombie.set_x(x);
    zombie.set_y(y);
    zombie.set_z(z);
    if let Some(f) = female {
        zombie.zombie.female = f;
    }
    let z_ref = Rc::new(RefCell::new(zombie));
    SPAWNED_AREA_ZOMBIES.with(|s| s.borrow_mut().push(z_ref.clone()));
    Ok(z_ref)
}

pub fn register(thread: &mut Thread) {
    // `addZombiesInOutfitArea(x1, y1, x2, y2, z, totalZombies, outfit, femaleChance)`
    global(thread, "addZombiesInOutfitArea", |c| {
        let x1 = int_arg(c, 0);
        let y1 = int_arg(c, 1);
        let x2 = int_arg(c, 2);
        let y2 = int_arg(c, 3);
        let z = int_arg(c, 4) as f32;
        let total = int_arg(c, 5).max(0) as usize;
        let _outfit = str_arg(c, 6);
        let female_chance = match c.arg(7) {
            Value::Num(n) => Some((n as i32).clamp(0, 100)),
            _ => None,
        };
        let (min_x, max_x) = if x1 <= x2 { (x1, x2) } else { (x2, x1) };
        let (min_y, max_y) = if y1 <= y2 { (y1, y2) } else { (y2, y1) };
        let span_x = (max_x - min_x) as f32;
        let span_y = (max_y - min_y) as f32;

        let list = JavaList::new(Vec::with_capacity(total));
        for i in 0..total {
            let t = if total <= 1 {
                0.5
            } else {
                i as f32 / (total - 1) as f32
            };
            let px = min_x as f32 + span_x * t + 0.5;
            let py = min_y as f32 + span_y * t + 0.5;
            let female = female_chance.map(|pct| ((i * 37) % 100) < pct as usize);
            let zb = spawn_area_zombie(px, py, z, female)?;
            list.push(character_value(&zb))?;
        }
        Ok(c.push(Value::Object(Rc::new(list))))
    });

    // `addZombiesInBuilding(buildingDef, totalZombies, outfit, roomDef, femaleChance)`
    global(thread, "addZombiesInBuilding", |c| {
        let bdef = c.arg(0);
        let total = int_arg(c, 1).max(0) as usize;
        let _outfit = str_arg(c, 2);
        let _room = c.arg(3);
        let female_chance = match c.arg(4) {
            Value::Num(n) => Some((n as i32).clamp(0, 100)),
            _ => None,
        };
        let (bx, by) = if !bdef.is_nil() {
            let gx = c.thread.table_get(&bdef, &Value::str("getX"))?;
            let gy = c.thread.table_get(&bdef, &Value::str("getY"))?;
            let x = if gx.is_nil() {
                0.0
            } else {
                pz_kahlua::exec::raw_tonumber(&c.thread.call1(&gx, &[bdef.clone()])?)
                    .unwrap_or(0.0) as f32
            };
            let y = if gy.is_nil() {
                0.0
            } else {
                pz_kahlua::exec::raw_tonumber(&c.thread.call1(&gy, &[bdef])?)
                    .unwrap_or(0.0) as f32
            };
            (x, y)
        } else {
            (0.0, 0.0)
        };
        let list = JavaList::new(Vec::with_capacity(total));
        for i in 0..total {
            let female = female_chance.map(|pct| ((i * 37) % 100) < pct as usize);
            let zb = spawn_area_zombie(bx + 0.5, by + 0.5, 0.0, female)?;
            list.push(character_value(&zb))?;
        }
        Ok(c.push(Value::Object(Rc::new(list))))
    });

    // `getFakeAttacker()`: lazily constructed `IsoZombie` used by debug/clothing blood tools.
    global(thread, "getFakeAttacker", |c| {
        let existing = FAKE_ATTACKER.with(|f| f.borrow().clone());
        let attacker = match existing {
            Some(z) => z,
            None => {
                let z = spawn_area_zombie(0.0, 0.0, 0.0, Some(false))?;
                FAKE_ATTACKER.with(|f| *f.borrow_mut() = Some(z.clone()));
                z
            }
        };
        Ok(c.push(character_value(&attacker)))
    });

    global(thread, "toggleStatisticsTransmission", |_c| {
        STATS_TRANSMISSION.with(|s| {
            let cur = *s.borrow();
            *s.borrow_mut() = !cur;
        });
        Ok(0)
    });

    for noop in [
        "Render3DItem",
        "showVehicleEditor",
        "getTickets",
        "getBannedIPs",
        "getBannedSteamIDs",
        "tradingUISendUpdateState",
        "sendWarManagerUpdate",
        "sendSafehouseChangeMember",
        "sendFactionChangeTag",
        "sendRemoveAndGrabAnimalFromTrailer",
    ] {
        global(thread, noop, |_c| Ok(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    #[test]
    fn extra_sp_globals_lua() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_characters::register_shared(&mut t);
        crate::bind_character_sim::register_shared(&mut t);
        register(&mut t);

        let src = r#"
            local area = addZombiesInOutfitArea(10, 20, 14, 24, 0, 3, "Generic", 50)
            local bdef = { getX = function() return 100 end, getY = function() return 200 end }
            local bldg = addZombiesInBuilding(bdef, 2, "Police", nil, 100)
            local a1 = getFakeAttacker()
            local a2 = getFakeAttacker()
            toggleStatisticsTransmission()
            Render3DItem(nil, nil, 0, 0, 0, 0)
            showVehicleEditor("Base.PickUpTruck")
            getTickets("admin")
            getBannedIPs()
            getBannedSteamIDs()
            tradingUISendUpdateState(nil, nil, nil)
            sendWarManagerUpdate(0, "", nil)
            sendSafehouseChangeMember(nil, "")
            sendFactionChangeTag(nil)
            sendRemoveAndGrabAnimalFromTrailer(nil, nil, nil, nil)
            return area:size(), bldg:size(), bldg:get(0):getX(), a1 == a2
        "#;
        let f = stdlib::load_string(src, Some("extra_globals"), t.globals().clone()).unwrap();
        let r = t.pcall(&f, &[]).map_err(|(e, tr)| format!("{e:?}\n{tr}")).unwrap();
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[0]), Some(3.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[1]), Some(2.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[2]), Some(100.5));
        assert!(r[3].truthy());
        assert!(STATS_TRANSMISSION.with(|s| *s.borrow()));
    }
}
