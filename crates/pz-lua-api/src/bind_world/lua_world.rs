//! The `IsoWorld` side the map-loading Lua calls through the global
//! `getWorld()` (`LuaManager.GlobalObject.getWorld` returns
//! `IsoWorld.instance`, the same object as `IsoWorld.instance` in Lua):
//! zone registration (`registerZone`, `registerVehiclesZone`, ...,
//! `registerNavZones`), spawn origins, water flow, `Core` game settings
//! and the `luaPos*` fields; `IsoMetaGrid.registerGeometryZone` /
//! `registerWorldGenZone` / `getLotDirectories`; the `Zone` objects they
//! return; and the globals `getLotDirectories` and `fileExists`.
//!
//! What has no engine object yet is kept here for the engine to collect:
//! `ZombiePopulationManager.spawnOrigins` ([`take_spawn_origins`], read by
//! `ZombiePopulationManager.init`) and `IsoWaterFlow`'s static state
//! ([`shared_water_flow`]).

use super::{
    get, grid, java_err, key_of, npe, obj, opt, self_world, void, with_world_state_cell, wrong, Key,
    ZoneClass,
};
use crate::bind_java_util::list_value;
use crate::expose::{expose_class, global, int_arg, num_arg, str_arg, ClassBuilder};
use pz_fs::GameFs;
use pz_iso_render::water_flow::IsoWaterFlow;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_world::nav_streets::{read_streets_xml, Street};
use pz_world::zones::{PropValue, ZoneError, ZoneId, ZoneProps};
use pz_world::IsoMetaGrid;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// `IsoWorld` fields Lua sets before a game starts (`luaPosX/Y/Z`,
/// `weather`, `spawnRegionName`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LuaWorldFields {
    pub lua_pos: (i32, i32, i32),
    pub weather: Option<String>,
    pub spawn_region: String,
}

thread_local! {
    static FIELDS: RefCell<LuaWorldFields> = RefCell::default();
    /// `ZombiePopulationManager.instance.spawnOrigins`: `(x, y, w, h)`.
    static SPAWN_ORIGINS: RefCell<Vec<(i32, i32, i32, i32)>> = RefCell::default();
    static WATER_FLOW: Rc<RefCell<IsoWaterFlow>> = Rc::default();
    /// `WorldMapStreets.s_fileNameToData` (the streets part), by absolute
    /// file name.
    static STREETS: RefCell<HashMap<String, Rc<Vec<Street>>>> = RefCell::default();
    /// `WorldMapStreet.RAILROAD_STRINGS`.
    static RAILROAD: RefCell<Option<Rc<Vec<String>>>> = const { RefCell::new(None) };
}

/// The `IsoWorld` fields Lua set.
pub fn lua_world_fields() -> LuaWorldFields {
    FIELDS.with(|f| f.borrow().clone())
}

/// Set `IsoWorld.luaPosX/Y/Z` (as the main menu's Lua does).
pub fn set_lua_pos(pos: (i32, i32, i32)) {
    FIELDS.with(|f| f.borrow_mut().lua_pos = pos);
}

/// The spawn origins registered so far, cleared (`ZombiePopulationManager`
/// hands them to the native side in `init` and clears them in `stop`).
pub fn take_spawn_origins() -> Vec<(i32, i32, i32, i32)> {
    SPAWN_ORIGINS.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

/// `IsoWaterFlow`'s static points and zones.
pub fn shared_water_flow() -> Rc<RefCell<IsoWaterFlow>> {
    WATER_FLOW.with(Rc::clone)
}

/// A Lua properties table as zone constructors read it (`rawget`).
pub struct LuaZoneProps(pub TableRef);

impl ZoneProps for LuaZoneProps {
    fn rawget(&self, key: &str) -> PropValue {
        match self.0.get_str(key) {
            Value::Nil => PropValue::Nil,
            Value::Bool(v) => PropValue::Bool(v),
            Value::Num(d) => PropValue::Number(d),
            Value::Str(s) => PropValue::Str(s.to_string()),
            _ => PropValue::Other,
        }
    }
}

fn props_arg(c: &NativeCall<'_>, i: usize) -> Option<LuaZoneProps> {
    c.arg(i).as_table().cloned().map(LuaZoneProps)
}

fn as_props(p: &Option<LuaZoneProps>) -> Option<&dyn ZoneProps> {
    p.as_ref().map(|p| p as &dyn ZoneProps)
}

/// The exception the Java code throws for a zone error.
pub fn zone_error(e: ZoneError) -> LuaError {
    match e {
        ZoneError::UnknownGeometry(_) => java_err(
            "java.lang.IllegalArgumentException",
            "unknown zone geometry type",
        ),
        ZoneError::OddPointCount | ZoneError::MissingProperties | ZoneError::NoCell => npe(),
        ZoneError::InvalidDirection(n) => java_err(
            "java.lang.IllegalArgumentException",
            format!("No enum constant zombie.iso.IsoDirections.{n}"),
        ),
        ZoneError::WrongPropertyType(what) => java_err("java.lang.ClassCastException", what),
    }
}

/// The Lua object of a zone.
fn zone_value(g: &IsoMetaGrid, id: Option<ZoneId>) -> Value {
    opt(id.map(|id| Key::Zone(id.0, ZoneClass::of(&g.zone(id).kind))))
}

fn s(c: &NativeCall<'_>, i: usize) -> String {
    str_arg(c, i).map(|s| s.to_string()).unwrap_or_default()
}

/// `name, type, x, y, z, width, height` arguments from index 1.
fn zone_args(c: &NativeCall<'_>) -> (String, String, i32, i32, i32, i32, i32) {
    (
        s(c, 1),
        s(c, 2),
        int_arg(c, 3),
        int_arg(c, 4),
        int_arg(c, 5),
        int_arg(c, 6),
        int_arg(c, 7),
    )
}

/// `IsoMetaGrid.getLotDirectories()`.
pub fn lot_directories(fs: &GameFs) -> LuaResult<Vec<String>> {
    // `GameClient.client` / `GameServer.server` replace `Core.gameMap`;
    // neither runs here.
    let is_default = crate::bind_core::shared_core().borrow().game_map == "DEFAULT";
    if is_default {
        crate::bind_map_groups::set_default_world()?;
    }
    let map = crate::bind_core::shared_core().borrow().game_map.clone();
    Ok(pz_world::map_files::lot_directories(fs, &map))
}

/// `File(ZomboidFileSystem.instance.getString(filename)).exists()`, the
/// working directory being the install.
fn file_exists(fs: &GameFs, filename: &str) -> bool {
    let f = filename.replace('\\', "/");
    fs.resolve(&f).is_some_and(|p| p.exists())
}

#[cfg(test)]
pub(crate) fn set_railroad_strings(v: Vec<String>) {
    RAILROAD.with(|r| *r.borrow_mut() = Some(Rc::new(v)));
}

fn railroad_strings(fs: &GameFs) -> LuaResult<Rc<Vec<String>>> {
    if let Some(r) = RAILROAD.with(|r| r.borrow().clone()) {
        return Ok(r);
    }
    let jar = pz_classfile::Jar::open(fs.base.join("projectzomboid.jar"))
        .map_err(|e| LuaError::runtime(format!("projectzomboid.jar: {e}")))?;
    let v =
        Rc::new(pz_world::nav_streets::railroad_strings_from_jar(&jar).map_err(LuaError::runtime)?);
    RAILROAD.with(|r| *r.borrow_mut() = Some(v.clone()));
    Ok(v)
}

/// `IsoWorld.registerNavZones()`.
fn register_nav_zones(fs: &GameFs) -> LuaResult<()> {
    let railroad = railroad_strings(fs)?;
    for dir in lot_directories(fs)? {
        let file = format!("media/maps/{dir}/streets.xml");
        if !file_exists(fs, &file) {
            continue;
        }
        let abs = fs.get_string(&file);
        // `WorldMapStreets.getOrCreateData(file, abs)`.
        let streets = match STREETS.with(|s| s.borrow().get(&abs).cloned()) {
            Some(s) => s,
            None => {
                let streets = match std::fs::read_to_string(&abs) {
                    Ok(text) => read_streets_xml(&text)
                        .map_err(|e| java_err("java.lang.RuntimeException", e))?,
                    Err(_) => Vec::new(),
                };
                let streets = Rc::new(streets);
                STREETS.with(|s| s.borrow_mut().insert(abs, streets.clone()));
                streets
            }
        };
        grid(|g| {
            for street in streets.iter() {
                street
                    .register_nav_zones(g, &railroad)
                    .map_err(LuaError::runtime)?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn ret_zone(
    f: impl FnOnce(&mut IsoMetaGrid) -> Result<Option<ZoneId>, ZoneError>,
) -> LuaResult<Value> {
    grid(|g| {
        let id = f(g).map_err(zone_error)?;
        Ok(zone_value(g, id))
    })
}

/// `getWorld()`, `getLotDirectories()`, `fileExists()` and the `IsoWorld`
/// / `IsoMetaGrid` methods map loading calls.
pub fn register(thread: &mut Thread, fs: Rc<RefCell<GameFs>>) {
    global(thread, "getWorld", |c| Ok(c.push(obj(Key::World))));
    {
        let fs = fs.clone();
        global(thread, "getLotDirectories", move |c| {
            let dirs = lot_directories(&fs.borrow())?;
            Ok(c.push(list_value(
                dirs.into_iter().map(|d| Value::str(&d)).collect(),
            )))
        });
    }
    {
        let fs = fs.clone();
        global(thread, "fileExists", move |c| {
            let Some(f) = str_arg(c, 0) else {
                return Err(npe());
            };
            let e = file_exists(&fs.borrow(), &f);
            Ok(c.push(e))
        });
    }
    let b = expose_class(thread, "zombie.iso.IsoWorld", Some("java.lang.Object"));
    register_world_zones(&b, fs.clone());
    register_world_settings(&b);
    let m = expose_class(thread, "zombie.iso.IsoMetaGrid", Some("java.lang.Object"));
    register_meta_grid_zones(&m, fs);
}

fn register_world_zones(b: &ClassBuilder, fs: Rc<RefCell<GameFs>>) {
    let register_zone = |c: &mut NativeCall<'_>| {
        self_world(c)?;
        let (n, t, x, y, z, w, h) = zone_args(c);
        ret_zone(|g| Ok(Some(g.register_zone(&n, &t, x, y, z, w, h))))
    };
    get(b, "registerZone", register_zone);
    // Deprecated; calls `registerZone`.
    get(b, "registerZoneNoOverlap", register_zone);
    get(b, "registerVehiclesZone", |c| {
        self_world(c)?;
        let (n, t, x, y, z, w, h) = zone_args(c);
        let p = props_arg(c, 8);
        ret_zone(|g| g.register_vehicles_zone(&n, &t, x, y, z, w, h, as_props(&p)))
    });
    get(b, "registerMannequinZone", |c| {
        self_world(c)?;
        let (n, t, x, y, z, w, h) = zone_args(c);
        let p = props_arg(c, 8);
        ret_zone(|g| g.register_mannequin_zone(&n, &t, x, y, z, w, h, as_props(&p)))
    });
    void(b, "registerRoomTone", |c| {
        self_world(c)?;
        let (n, t, x, y, z, w, h) = zone_args(c);
        let p = props_arg(c, 8);
        grid(|g| {
            g.register_room_tone(&n, &t, x, y, z, w, h, as_props(&p))
                .map_err(zone_error)
        })
    });
    void(b, "registerSpawnOrigin", |c| {
        self_world(c)?;
        let (x, y, w, h) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        // `ZombiePopulationManager.registerSpawnOrigin`.
        if x < 0 || y < 0 || w < 0 || h < 0 {
            return Ok(());
        }
        SPAWN_ORIGINS.with(|s| s.borrow_mut().push((x, y, w, h)));
        Ok(())
    });
    void(b, "registerWaterFlow", |c| {
        self_world(c)?;
        let f = |i| num_arg(c, i) as f32;
        WATER_FLOW.with(|w| w.borrow_mut().add_flow(f(1), f(2), f(3), f(4)));
        Ok(())
    });
    void(b, "registerWaterZone", |c| {
        self_world(c)?;
        let f = |i| num_arg(c, i) as f32;
        WATER_FLOW.with(|w| w.borrow_mut().add_zone(f(1), f(2), f(3), f(4), f(5), f(6)));
        Ok(())
    });
    void(b, "removeZonesForLotDirectory", |c| {
        self_world(c)?;
        let Some(dir) = str_arg(c, 1) else {
            return Err(npe());
        };
        grid(|g| {
            g.remove_zones_for_lot_directory(&dir);
            Ok(())
        })
    });
    void(b, "checkVehiclesZones", |c| {
        self_world(c)?;
        grid(|g| {
            g.check_vehicles_zones();
            Ok(())
        })
    });
    void(b, "registerNavZones", move |c| {
        self_world(c)?;
        register_nav_zones(&fs.borrow())
    });
}

fn register_world_settings(b: &ClassBuilder) {
    let core = crate::bind_core::shared_core;
    let str_or_nil = |v: Option<String>| v.map_or(Value::Nil, |s| Value::str(&s));
    void(b, "setGameMode", move |c| {
        self_world(c)?;
        let Some(mode) = str_arg(c, 1) else {
            return Err(npe());
        };
        let core = core();
        let mut core = core.borrow_mut();
        core.set_game_mode(&mode);
        core.last_stand = &*mode == "LastStand";
        core.set_challenge(false);
        core.challenge_id = None;
        Ok(())
    });
    get(b, "getGameMode", move |c| {
        self_world(c)?;
        Ok(Value::str(&core().borrow().game_mode))
    });
    void(b, "setPreset", move |c| {
        self_world(c)?;
        core().borrow_mut().preset = s(c, 1);
        Ok(())
    });
    get(b, "getPreset", move |c| {
        self_world(c)?;
        Ok(Value::str(&core().borrow().preset))
    });
    void(b, "setWorld", move |c| {
        self_world(c)?;
        let Some(w) = str_arg(c, 1) else {
            return Err(npe());
        };
        core().borrow_mut().game_save_world = pz_world::java::java_trim(&w).to_string();
        Ok(())
    });
    get(b, "getWorld", move |c| {
        self_world(c)?;
        Ok(Value::str(&core().borrow().game_save_world))
    });
    void(b, "setMap", move |c| {
        self_world(c)?;
        core().borrow_mut().game_map = s(c, 1);
        Ok(())
    });
    get(b, "getMap", move |c| {
        self_world(c)?;
        Ok(Value::str(&core().borrow().game_map))
    });
    let pos_get = |b: &ClassBuilder, name: &str, f: fn(&(i32, i32, i32)) -> i32| {
        get(b, name, move |c| {
            self_world(c)?;
            Ok(FIELDS.with(|w| f(&w.borrow().lua_pos)).into())
        });
    };
    pos_get(b, "getLuaPosX", |p| p.0);
    pos_get(b, "getLuaPosY", |p| p.1);
    pos_get(b, "getLuaPosZ", |p| p.2);
    pos_get(b, "getLuaSpawnCellX", |p| {
        pz_world::java::coordmodulo(p.0, 256)
    });
    pos_get(b, "getLuaSpawnCellY", |p| {
        pz_world::java::coordmodulo(p.1, 256)
    });
    let pos_set = |b: &ClassBuilder, name: &str, f: fn(&mut (i32, i32, i32), i32)| {
        void(b, name, move |c| {
            self_world(c)?;
            let v = int_arg(c, 1);
            FIELDS.with(|w| f(&mut w.borrow_mut().lua_pos, v));
            Ok(())
        });
    };
    pos_set(b, "setLuaPosX", |p, v| p.0 = v);
    pos_set(b, "setLuaPosY", |p, v| p.1 = v);
    pos_set(b, "setLuaPosZ", |p, v| p.2 = v);
    // Deprecated no-ops.
    pos_set(b, "setLuaSpawnCellX", |_, _| {});
    pos_set(b, "setLuaSpawnCellY", |_, _| {});
    get(b, "getWeather", move |c| {
        self_world(c)?;
        Ok(str_or_nil(FIELDS.with(|w| w.borrow().weather.clone())))
    });
    void(b, "setWeather", |c| {
        self_world(c)?;
        let v = str_arg(c, 1).map(|s| s.to_string());
        FIELDS.with(|w| w.borrow_mut().weather = v);
        Ok(())
    });
    void(b, "setSpawnRegion", |c| {
        self_world(c)?;
        if let Some(v) = str_arg(c, 1) {
            FIELDS.with(|w| w.borrow_mut().spawn_region = v.to_string());
        }
        Ok(())
    });
    get(b, "getSpawnRegion", |c| {
        self_world(c)?;
        Ok(Value::str(
            &FIELDS.with(|w| w.borrow().spawn_region.clone()),
        ))
    });
}

fn self_meta_grid(c: &NativeCall<'_>) -> LuaResult<()> {
    match key_of(&c.arg(0))? {
        Key::MetaGrid => Ok(()),
        _ => Err(wrong(c, 0, "zombie.iso.IsoMetaGrid")),
    }
}

fn register_meta_grid_zones(b: &ClassBuilder, fs: Rc<RefCell<GameFs>>) {
    get(b, "getLotDirectories", move |c| {
        self_meta_grid(c)?;
        let dirs = lot_directories(&fs.borrow())?;
        Ok(list_value(
            dirs.into_iter().map(|d| Value::str(&d)).collect(),
        ))
    });
    get(b, "registerZone", |c| {
        self_meta_grid(c)?;
        let (n, t, x, y, z, w, h) = zone_args(c);
        ret_zone(|g| Ok(Some(g.register_zone(&n, &t, x, y, z, w, h))))
    });
    get(b, "registerWorldGenZone", |c| {
        self_meta_grid(c)?;
        let (n, t, x, y, z, w, h) = zone_args(c);
        let p = props_arg(c, 8);
        ret_zone(|g| Ok(g.register_world_gen_zone(&n, &t, x, y, z, w, h, as_props(&p))))
    });
    get(b, "registerVehiclesZone", |c| {
        self_meta_grid(c)?;
        let (n, t, x, y, z, w, h) = zone_args(c);
        let p = props_arg(c, 8);
        ret_zone(|g| g.register_vehicles_zone(&n, &t, x, y, z, w, h, as_props(&p)))
    });
    get(b, "registerMannequinZone", |c| {
        self_meta_grid(c)?;
        let (n, t, x, y, z, w, h) = zone_args(c);
        let p = props_arg(c, 8);
        ret_zone(|g| g.register_mannequin_zone(&n, &t, x, y, z, w, h, as_props(&p)))
    });
    get(b, "registerAnimalZone", |c| {
        self_meta_grid(c)?;
        let (n, t, x, y, z, w, h) = zone_args(c);
        let p = props_arg(c, 8);
        ret_zone(|g| Ok(g.register_animal_zone(&n, &t, x, y, z, w, h, as_props(&p))))
    });
    // `registerGeometryZone(name, type, z, geometry, pointsTable, properties)`.
    get(b, "registerGeometryZone", |c| {
        self_meta_grid(c)?;
        let (n, t, z) = (s(c, 1), s(c, 2), int_arg(c, 3));
        let Some(geometry) = str_arg(c, 4) else {
            return Err(npe());
        };
        let Some(points) = c.arg(5).as_table().cloned() else {
            return Err(npe());
        };
        let p = props_arg(c, 6);
        // `((Double) pointsTable.rawget(i + 1)).intValue()` pairwise.
        let len = points.len();
        let mut pts = Vec::with_capacity(len.max(0) as usize);
        let mut i = 0;
        while i < len {
            for k in [i + 1, i + 2] {
                match points.get_int(k) {
                    Value::Num(d) => pts.push(d),
                    Value::Nil => return Err(npe()),
                    _ => {
                        return Err(java_err(
                            "java.lang.ClassCastException",
                            "points value is not a Double",
                        ))
                    }
                }
            }
            i += 2;
        }
        ret_zone(|g| g.register_geometry_zone(&n, &t, z, &geometry, &pts, as_props(&p)))
    });
    void(b, "checkVehiclesZones", |c| {
        self_meta_grid(c)?;
        grid(|g| {
            g.check_vehicles_zones();
            Ok(())
        })
    });
}

fn zone_of(c: &NativeCall<'_>) -> LuaResult<ZoneId> {
    match key_of(&c.arg(0))? {
        Key::Zone(id, _) => Ok(ZoneId(id)),
        _ => Err(wrong(c, 0, "zombie.iso.zones.Zone")),
    }
}

/// `zombie.iso.zones.Zone` and the subclasses map loading creates, with
/// the getters backed by the zone data.
pub fn register_zone(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.iso.zones.Zone", Some("java.lang.Object"));
    let field = |b: &ClassBuilder, name: &str, f: fn(&pz_world::zones::Zone) -> Value| {
        get(b, name, move |c| {
            let id = zone_of(c)?;
            with_world_state_cell(|w| match w.borrow().meta_grid.as_ref() {
                Some(g) => Ok(f(g.zone(id))),
                None => Err(LuaError::runtime("IsoMetaGrid is not loaded")),
            })
        });
    };
    field(&b, "getName", |z| Value::str(&z.name));
    field(&b, "getType", |z| Value::str(&z.zone_type));
    field(&b, "getOriginalName", |z| {
        z.get_original_name().map_or(Value::Nil, Value::str)
    });
    field(&b, "getX", |z| z.x.into());
    field(&b, "getY", |z| z.y.into());
    field(&b, "getZ", |z| z.z.into());
    field(&b, "getWidth", |z| z.w.into());
    field(&b, "getHeight", |z| z.h.into());
    field(&b, "isPoint", |z| z.is_point().into());
    field(&b, "isPolygon", |z| z.is_polygon().into());
    field(&b, "isPolyline", |z| z.is_polyline().into());
    field(&b, "isRectangle", |z| z.is_rectangle().into());
    for sub in [
        "zombie.iso.zones.VehicleZone",
        "zombie.iso.objects.IsoMannequin$MannequinZone",
        "zombie.iso.worldgen.zones.WorldGenZone",
        "zombie.characters.animals.AnimalZone",
    ] {
        expose_class(thread, sub, Some("zombie.iso.zones.Zone"));
    }
}
