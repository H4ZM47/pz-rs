//! `zombie.basements.Basements.getAPIv1()` (`BasementsV1`): what a map's
//! `basements.lua` (run by `loadMapBasementLuaFiles` on `OnLoadMapZones`)
//! and the `Basement` zones of its `objects.lua` (`metazoneHandler.lua`)
//! register: per map, basement and access definitions and spawn
//! locations. They are kept here ([`take_per_map`]) for
//! `Basements.beforeLoadMetaGrid`.

use crate::expose::{expose_class, int_arg, str_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_worldgen::basements::{
    get_or_create_per_map, spawn_location_from_properties, BasementsPerMap,
};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    /// `Basements.instance.basementsPerMap`, in creation order.
    static PER_MAP: RefCell<Vec<BasementsPerMap>> = RefCell::default();
}

/// `Basements.beforeOnLoadMapZones()`: forget what an earlier load
/// registered.
pub fn before_on_load_map_zones() {
    PER_MAP.with(|p| p.borrow_mut().clear());
}

/// The per-map definitions and spawn locations registered so far.
pub fn take_per_map() -> Vec<BasementsPerMap> {
    PER_MAP.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// A copy of the per-map data registered so far.
pub fn per_map() -> Vec<BasementsPerMap> {
    PER_MAP.with(|p| p.borrow().clone())
}

struct Api;

impl LuaObject for Api {
    fn class_name(&self) -> &str {
        "zombie.basements.BasementsV1"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct SpawnLocation;

impl LuaObject for SpawnLocation {
    fn class_name(&self) -> &str {
        "zombie.basements.BasementSpawnLocation"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn with_map<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut BasementsPerMap) -> Result<R, String>,
) -> LuaResult<R> {
    crate::expose::this::<Api>(c)?;
    let id = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
    PER_MAP.with(|p| {
        let mut p = p.borrow_mut();
        let Some(m) = get_or_create_per_map(&mut p, &id) else {
            return Err(LuaError::Runtime {
                message: format!("invalid mapID \"{id}\""),
                class: "java.lang.IllegalArgumentException",
            });
        };
        f(m).map_err(LuaError::runtime)
    })
}

fn table(c: &NativeCall<'_>, i: usize) -> LuaResult<TableRef> {
    c.arg(i)
        .as_table()
        .cloned()
        .ok_or_else(|| LuaError::Runtime {
            message: String::new(),
            class: "java.lang.NullPointerException",
        })
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(
        thread,
        "zombie.basements.Basements",
        Some("java.lang.Object"),
    );
    b.static_fn("getAPIv1", |c| Ok(c.push(Value::Object(Rc::new(Api)))));
    let api = expose_class(
        thread,
        "zombie.basements.BasementsV1",
        Some("java.lang.Object"),
    );
    api.method("addAccessDefinitions", |c| {
        let t = table(c, 2)?;
        with_map(c, |m| m.add_access_definitions(&t))?;
        Ok(0)
    });
    api.method("addBasementDefinitions", |c| {
        let t = table(c, 2)?;
        with_map(c, |m| m.add_basement_definitions(&t))?;
        Ok(0)
    });
    api.method("addSpawnLocations", |c| {
        let t = table(c, 2)?;
        with_map(c, |m| m.add_spawn_locations(&t))?;
        Ok(0)
    });
    // `registerBasementSpawnLocation(mapID, name, type, x, y, z, width,
    // height, properties)`.
    api.method("registerBasementSpawnLocation", |c| {
        let (x, y, z, w, h) = (
            int_arg(c, 4),
            int_arg(c, 5),
            int_arg(c, 6),
            int_arg(c, 7),
            int_arg(c, 8),
        );
        let props = c.arg(9).as_table().cloned();
        with_map(c, |m| {
            m.spawn_locations.push(spawn_location_from_properties(
                x,
                y,
                z,
                w,
                h,
                props.as_ref(),
            ));
            Ok(())
        })?;
        Ok(c.push(Value::Object(Rc::new(SpawnLocation))))
    });
    expose_class(
        thread,
        "zombie.basements.BasementSpawnLocation",
        Some("java.lang.Object"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_records_per_map() {
        before_on_load_map_zones();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        let src = r#"
            local api = Basements.getAPIv1()
            api:addBasementDefinitions(' Map A ', { b1 = { width = 10, height = 11, stairx = 2, stairy = 3, stairDir = "N" } })
            api:addAccessDefinitions('Map A', { ba_1 = { width = 4, height = 5, stairx = 0, stairy = 1, stairDir = "W" } })
            api:addSpawnLocations('Map A', { { x = 100, y = 200, stairDir = "N", choices = { " b1 ", "" }, access = "ba_1" } })
            api:registerBasementSpawnLocation('Map B', 'n', 'Basement', 1, 2, 0, 30, 40, { StairX = 5, StairDirection = "W" })
        "#;
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
        t.call(&f, &[]).unwrap();
        let p = take_per_map();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].map_id, "Map A");
        assert_eq!(p[0].definitions[0].width, 10);
        assert!(p[0].definitions[0].north);
        assert!(!p[0].access_definitions[0].north);
        let s = &p[0].spawn_locations[0];
        assert_eq!((s.x, s.y, s.w, s.h), (100, 200, 1, 1));
        assert_eq!(s.specific.as_deref(), Some(&["b1".to_string()][..]));
        assert_eq!(s.access.as_deref(), Some("ba_1"));
        let s = &p[1].spawn_locations[0];
        assert_eq!(
            (s.x, s.y, s.z, s.w, s.h, s.stair_x, s.stair_y, s.north),
            (1, 2, 0, 30, 40, 5, 0, false)
        );
        let bad = pz_kahlua::stdlib::load_string(
            "Basements.getAPIv1():addSpawnLocations(' ', {})",
            Some("t"),
            t.globals().clone(),
        )
        .unwrap();
        assert!(t
            .call(&bad, &[])
            .unwrap_err()
            .to_string()
            .contains("invalid mapID"));
    }
}
