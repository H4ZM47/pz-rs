//! Build 42.22 Lua bindings for `zombie.iso.SpawnPoints`.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{TableRef, Thread, Value};
use pz_world::meta_grid::IsoMetaGrid;
use pz_world::spawn_points::{SpawnPoint, SpawnPoints};

use crate::bind_java_util::JavaList;
use crate::bind_world::with_world;
use crate::expose::expose_class;

const SPAWN_POINTS_CLASS: &str = "zombie.iso.SpawnPoints";

thread_local! {
    static INSTANCE: RefCell<SpawnPoints> = RefCell::new(SpawnPoints::new());
    static SPAWN_REGIONS: RefCell<Option<TableRef>> = const { RefCell::new(None) };
}

/// Reset session state on world teardown.
pub fn reset() {
    INSTANCE.with(|s| s.borrow_mut().clear());
    SPAWN_REGIONS.with(|r| *r.borrow_mut() = None);
}

/// Query whether a building ID is marked as a starter spawn building.
pub fn is_spawn_building_id(id: i64) -> bool {
    INSTANCE.with(|s| s.borrow().is_spawn_building_id(id))
}

/// Parse spawn regions from Lua's `SpawnRegionMgr.getSpawnRegions()` and
/// resolve their starter buildings on the installed `IsoMetaGrid`.
/// Fetches the Lua table BEFORE borrowing `WORLD` so Lua callbacks can safely
/// access world bindings without re-entering `WORLD`.
pub fn init_single_player(thread: &mut Thread) -> usize {
    INSTANCE.with(|s| s.borrow_mut().clear());
    let regions_tbl = fetch_spawn_regions(thread);
    SPAWN_REGIONS.with(|slot| *slot.borrow_mut() = regions_tbl.clone());
    with_world(|w| {
        let Some(meta) = w.meta_grid.as_ref() else {
            return 0;
        };
        if let Some(regions) = regions_tbl.as_ref() {
            parse_regions_table(regions, meta);
        }
        INSTANCE.with(|s| s.borrow_mut().init_spawn_buildings(meta))
    })
}

fn fetch_spawn_regions(thread: &mut Thread) -> Option<TableRef> {
    let mgr = thread.globals().get_str("SpawnRegionMgr");
    let Value::Table(_) = &mgr else {
        return None;
    };
    let func = thread.table_get(&mgr, &Value::str("getSpawnRegions")).ok()?;
    if func.is_nil() {
        return None;
    }
    let res = thread.pcall(&func, &[]).ok()?;
    match res.into_iter().next() {
        Some(Value::Table(t)) => Some(t),
        _ => None,
    }
}

fn parse_regions_table(regions: &TableRef, meta: &IsoMetaGrid) {
    for (_, region_val) in regions.try_entries().unwrap_or_default() {
        let Value::Table(region) = region_val else {
            continue;
        };
        if region
            .get_str("position")
            .as_str()
            .is_some_and(|p| p.eq_ignore_ascii_case("center"))
        {
            INSTANCE.with(|s| s.borrow_mut().add_center_point(meta));
        }
        let Value::Table(points_by_prof) = region.get_str("points") else {
            continue;
        };
        for (_, prof_val) in points_by_prof.try_entries().unwrap_or_default() {
            let Value::Table(prof_list) = prof_val else {
                continue;
            };
            for (_, pt_val) in prof_list.try_entries().unwrap_or_default() {
                let Value::Table(pt_tbl) = pt_val else {
                    continue;
                };
                let world_x = pt_tbl.get_str("worldX").as_num().map(|n| n as i32);
                let world_y = pt_tbl.get_str("worldY").as_num().map(|n| n as i32);
                let pos_x = pt_tbl.get_str("posX").as_num().unwrap_or(0.0) as i32;
                let pos_y = pt_tbl.get_str("posY").as_num().unwrap_or(0.0) as i32;
                let pos_z = pt_tbl.get_str("posZ").as_num().unwrap_or(0.0) as i32;
                let pt = SpawnPoint::from_table_fields(world_x, world_y, pos_x, pos_y, pos_z);
                INSTANCE.with(|s| s.borrow_mut().add_point(pt));
            }
        }
    }
}

#[derive(Debug)]
struct SpawnPointsHandle;

impl LuaObject for SpawnPointsHandle {
    fn class_name(&self) -> &'static str {
        SPAWN_POINTS_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn call_zero(thread: &mut Thread, obj: &Value, method: &str) -> Option<Value> {
    let f = thread.table_get(obj, &Value::str(method)).ok()?;
    if f.is_nil() {
        return None;
    }
    thread
        .pcall(&f, std::slice::from_ref(obj))
        .ok()?
        .into_iter()
        .next()
}

/// Register `zombie.iso.SpawnPoints`.
pub fn register(thread: &mut Thread) {
    let sp = expose_class(thread, SPAWN_POINTS_CLASS, Some("java.lang.Object"));
    sp.field("instance", Value::Object(Rc::new(SpawnPointsHandle)));
    sp.method("init", |_| {
        reset();
        Ok(0)
    });
    sp.method("initSinglePlayer", |c| {
        init_single_player(c.thread);
        Ok(0)
    });
    sp.method("initServer1", |_| Ok(0));
    sp.method("initServer2", |_| Ok(0));
    sp.method("getSpawnRegions", |c| {
        let out = SPAWN_REGIONS.with(|r| r.borrow().clone().map(Value::Table));
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    sp.method("getSpawnPoints", |c| {
        let items = INSTANCE.with(|s| {
            s.borrow()
                .points
                .iter()
                .map(|p| {
                    let t = TableRef::new();
                    t.set_str("x", Value::Num(p.x as f64));
                    t.set_str("y", Value::Num(p.y as f64));
                    t.set_str("z", Value::Num(p.z as f64));
                    Value::Table(t)
                })
                .collect()
        });
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    sp.method("isSpawnBuilding", |c| {
        let arg1 = c.arg(1);
        let ok = if arg1.is_nil() {
            false
        } else {
            call_zero(c.thread, &arg1, "getID")
                .and_then(|v| v.as_num())
                .is_some_and(|id| is_spawn_building_id(id as i64))
        };
        Ok(c.push(ok))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::defs::{BuildingDef, RoomDef, RoomRect};
    use pz_world::GameRand;

    struct ZeroRand;
    impl GameRand for ZeroRand {
        fn next(&mut self, _: i32) -> i32 {
            0
        }
    }

    #[test]
    fn spawn_points_lua_single_player_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);

        let mut g = IsoMetaGrid::blank(0, 0, 0, 0);
        g.create_cell(0, 0).unwrap();
        let mut r = RoomDef::new(0, "bedroom".to_string());
        r.level = 0;
        r.rects.push(RoomRect::new(40, 50, 8, 8));
        r.CalculateBounds();
        let b = g
            .add_building(0, 0, BuildingDef::new(&mut ZeroRand), vec![r], vec![])
            .unwrap();
        let bid = g.building_defs[b.0 as usize].id;
        crate::bind_world::set_meta_grid(Some(g));

        let src = format!(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            SpawnRegionMgr = {{
                getSpawnRegions = function()
                    return {{
                        {{
                            name = "TestTown",
                            points = {{
                                unemployed = {{
                                    {{ posX = 42, posY = 52, posZ = 0 }}
                                }}
                            }}
                        }}
                    }}
                end
            }}
            SpawnPoints.instance:initSinglePlayer()
            assert(SpawnPoints.instance:getSpawnPoints():size() == 1)
            assert(SpawnPoints.instance:getSpawnRegions() ~= nil)
            local fake_b = {{ getID = function() return {bid} end }}
            assert(SpawnPoints.instance:isSpawnBuilding(fake_b) == true)
            local other_b = {{ getID = function() return 999999 end }}
            assert(SpawnPoints.instance:isSpawnBuilding(other_b) == false)
            "#
        );
        let f = pz_kahlua::stdlib::load_string(&src, Some("spawn_points_test"), t.globals().clone())
            .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));

        crate::bind_world::set_meta_grid(None);
        reset();
    }
}
