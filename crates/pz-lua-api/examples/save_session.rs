//! A played session on a copy of a save: load the world around a chunk,
//! walk with the chunk streamer (unloaded chunks and their vehicles are
//! saved), then quit (`world_save::save_on_quit`, `ChunkStreamer::stop`,
//! `world_save::close_stores`), for comparing every written file against
//! the original game driven by a local harness on a machine with the game
//! installed. Results are recorded as notes only.
//!
//! ```text
//! save_session <game dir> <save dir> <work dir> <script> <out file> [--scripts <file>]
//! ```
//!
//! `<work dir>` must not exist. The save's `map/`, `vehicles.db`,
//! `map_t.bin`, `map_meta.bin`, `map_zone.bin`, `map_sand.bin`,
//! `isoregiondata/` and `gos_*.bin` are copied there first; the session
//! reads and writes only the copies. The map has no lot files, so
//! brand-new chunks are empty (as in the harness). The meta grid of the
//! save step holds exactly the rooms and buildings `map_meta.bin` names;
//! the streamer's grid is the harness's empty one (cells `0..=99`).
//!
//! `--scripts` gives the vehicle scripts the harness attached to every
//! handed-off vehicle (`name \t alarmEnable \t lightbarEnable \t alarm,..
//! \t alarmLoop,..`); without it no vehicle has a script, and unloading
//! cannot write them (as in the original).
//!
//! The script is `streamer_walk`'s (`width`, `load`, `frame`) plus `quit`.
//! The out file gets one line per streamer event (`R S C H D U W X`,
//! `VU` for vehicles written on unload, `HOT` for the hot save).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use pz_core::core::Core;
use pz_core::game_time::GameTime;
use pz_core::game_time_save;
use pz_kahlua::{TableRef, Value};
use pz_lua_api::global_objects::{init_lookup, reset, GlobalObjectSystem, GridBounds, Side};
use pz_lua_api::global_objects_save::load_system_bytes;
use pz_lua_api::player_db::VehicleStore;
use pz_lua_api::world_save::{self, HotSave, SaveGame, SaveGameHooks};
use pz_world::chunk::IsoCell;
use pz_world::fix2x::Fix2xTable;
use pz_world::meta_save::{load_map_meta, DefaultMetaEnv, MetaWorld, NetRole};
use pz_world::save::items::AllItems;
use pz_world::save::vehicle::AlarmSounds;
use pz_world::save::vehicles_db::{LoadedVehicleSaver, VehicleScriptFacts, VehicleScripts};
use pz_world::save::{NoSaveHooks, SaveSettings, SaveTables};
use pz_world::streamer::{ChunkSource, ChunkStreamer, PlayerPos, Serve, StreamCtx, StreamEvent};
use pz_world::zone_save::{ClockUuids, SpawnedZombieZones};
use pz_world::{
    BuildingDef, BuildingIdx, GameRand, IsoMetaCell, IsoMetaGrid, IsoSpriteManager, NoHooks,
    ObjectLoadSettings, ObjectTables, RoomDef, RoomIdx, SpriteTables,
};

struct NoRand;
impl GameRand for NoRand {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

/// The harness's script table.
struct Scripts(HashMap<String, (bool, bool, Vec<String>, Vec<String>)>);

impl VehicleScripts for Scripts {
    fn script(&self, name: &str) -> Option<VehicleScriptFacts<'_>> {
        let (alarm_enable, lightbar, alarm, alarm_loop) = self.0.get(name)?;
        Some(VehicleScriptFacts {
            sounds: AlarmSounds {
                alarm_enable: *alarm_enable,
                alarm,
                alarm_loop,
            },
            lightbar_enable: *lightbar,
        })
    }
}

fn read_scripts(path: &Path) -> Scripts {
    let list = |s: Option<&str>| -> Vec<String> {
        s.unwrap_or("")
            .split(',')
            .filter(|x| !x.is_empty())
            .map(str::to_string)
            .collect()
    };
    let mut m = HashMap::new();
    for l in std::fs::read_to_string(path).unwrap().lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 3 {
            continue;
        }
        m.insert(
            f[0].to_string(),
            (
                f[1] == "true",
                f[2] == "true",
                list(f.get(3).copied()),
                list(f.get(4).copied()),
            ),
        );
    }
    Scripts(m)
}

fn line(e: &StreamEvent) -> Option<String> {
    Some(match e {
        StreamEvent::Requested { wx, wy, .. } => format!("R {wx},{wy}"),
        StreamEvent::Served { wx, wy, source } => format!(
            "S {wx},{wy} {}",
            match source {
                ChunkSource::Disk => "d".to_string(),
                ChunkSource::BrandNew => "n".into(),
                ChunkSource::BrandNewAfterError(e) => format!("e {e}"),
            }
        ),
        StreamEvent::Cancelled { wx, wy } => format!("C {wx},{wy}"),
        StreamEvent::HandedOff { wx, wy } => format!("H {wx},{wy}"),
        StreamEvent::Discarded { wx, wy } => format!("D {wx},{wy}"),
        StreamEvent::Unloaded { wx, wy } => format!("U {wx},{wy}"),
        StreamEvent::Saved { wx, wy, .. } => format!("W {wx},{wy}"),
        StreamEvent::SaveFailed { wx, wy, error } => format!("W {wx},{wy} failed: {error}"),
        StreamEvent::Removed { wx, wy } => format!("X {wx},{wy}"),
        StreamEvent::VehiclesUnloaded {
            wx,
            wy,
            saved,
            errors,
        } => {
            if saved.is_empty() && errors.is_empty() {
                return None;
            }
            let errs: Vec<&str> = errors.iter().map(|e| e.1.as_str()).collect();
            format!("# VU {wx},{wy} saved={saved:?} errors={errs:?}")
        }
        StreamEvent::HotSave => "# HOT".into(),
        StreamEvent::VehiclesLoaded { .. } => return None,
    })
}

/// A grid holding exactly the rooms and buildings `map_meta.bin` names, in
/// file order (`world_save_roundtrip --standin-out`), with the file
/// applied. `map_zone.bin` is loaded before the cells exist (the stand-in
/// cells have no map files to seed meta chunks from), as
/// `world_save_roundtrip` loads it into an empty grid.
fn meta_grid(
    data: &[u8],
    work: &Path,
    spawned: &mut SpawnedZombieZones,
    uuids: &mut ClockUuids,
) -> (IsoMetaGrid, MetaWorld) {
    let file =
        pz_world::meta_save::MapMetaFile::parse(data, NetRole::SinglePlayer, &mut DefaultMetaEnv)
            .expect("map_meta.bin");
    let c = &file.cells;
    let mut g = IsoMetaGrid::with_bounds(c.min_x, c.min_y, c.max_x, c.max_y);
    g.load_zones_file(work, spawned, uuids)
        .expect("map_zone.bin");
    let h = c.max_y - c.min_y + 1;
    for (i, (rooms, buildings)) in c.cells.iter().enumerate() {
        if rooms.is_empty() && buildings.is_empty() {
            continue;
        }
        let (gx, gy) = (i as i32 / h, i as i32 % h);
        let mut cell = IsoMetaCell::new(c.min_x + gx, c.min_y + gy);
        cell.info = Some((0, format!("{}_{}.lotheader", c.min_x + gx, c.min_y + gy)));
        for r in rooms {
            let mut def = RoomDef::new(r.meta_id, "r".into());
            def.meta_id = r.meta_id;
            let idx = RoomIdx(g.room_defs.len() as u32);
            g.room_defs.push(def);
            cell.room_list.push(idx);
            cell.room_by_meta_id.insert(r.meta_id, idx);
        }
        for b in buildings {
            let mut def = BuildingDef::new(&mut NoRand);
            def.meta_id = b.meta_id;
            let idx = BuildingIdx(g.building_defs.len() as u32);
            g.building_defs.push(def);
            cell.buildings.push(idx);
            cell.building_by_meta_id.insert(b.meta_id, idx);
        }
        g.set_cell(gx, gy, Some(cell));
    }
    let mut world = MetaWorld::default();
    load_map_meta(
        &mut g,
        &mut world,
        data,
        NetRole::SinglePlayer,
        &mut DefaultMetaEnv,
    )
    .expect("load map_meta.bin");
    (g, world)
}

fn string_keys(t: &TableRef, into: &mut HashSet<String>) {
    for (k, _) in t.entries() {
        if let Value::Str(s) = k {
            into.insert(s.to_string());
        }
    }
}

/// Every `gos_*.bin` loaded into a server system that saves every key it
/// holds (the Lua side registers the key lists in the game).
fn load_global_objects(work: &Path) -> Vec<std::rc::Rc<GlobalObjectSystem>> {
    reset(Side::Server);
    init_lookup(GridBounds {
        min_x: 0,
        min_y: 0,
        width: 200,
        height: 200,
    });
    let mut names: Vec<String> = std::fs::read_dir(work)
        .unwrap()
        .filter_map(|e| {
            let n = e.unwrap().file_name().to_string_lossy().into_owned();
            let s = n.strip_prefix("gos_")?.strip_suffix(".bin")?.to_string();
            Some(s)
        })
        .collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let path = work.join(format!("gos_{name}.bin"));
        let system = GlobalObjectSystem::new(Side::Server, &name).unwrap();
        load_system_bytes(&system, &std::fs::read(&path).unwrap(), &path).unwrap();
        let mut keys = HashSet::new();
        string_keys(&system.mod_data, &mut keys);
        *system.mod_data_keys.borrow_mut() = keys;
        let mut keys = HashSet::new();
        for i in 0..system.object_count() {
            string_keys(
                &system.object_by_index(i as i32).unwrap().mod_data,
                &mut keys,
            );
        }
        *system.object_mod_data_keys.borrow_mut() = keys;
        out.push(system);
    }
    out
}

/// The item registry over the game's scripts and the save's
/// `WorldDictionary.bin` (as `load_gaps_parity` builds it).
fn script_items(
    game: &Path,
    save: &Path,
    sprites: &IsoSpriteManager,
    save_tables: &SaveTables,
    world_age: f64,
) -> pz_inventory::factory::ScriptItems {
    let sm: &'static pz_scripts::ScriptManager =
        Box::leak(Box::new(pz_scripts::ScriptManager::new()));
    sm.load_builtin_registries(&game.join("projectzomboid.jar"))
        .expect("built-in registries");
    sm.load(&pz_fs::GameFs::new(game.to_str().unwrap()), &[])
        .expect("scripts");
    let dict = pz_inventory::dictionary::WorldDictionary::load(
        &std::fs::read(save.join("WorldDictionary.bin")).unwrap(),
    )
    .unwrap();
    let names = pz_inventory::factory::parse_item_names(
        &std::fs::read_to_string(game.join("media/lua/shared/Translate/EN/ItemName.json")).unwrap(),
    );
    let mut registry = pz_inventory::factory::ScriptItems::new(sm, dict, &names);
    registry.world_age_hours = world_age;
    registry.blood_body_parts = save_tables.blood_body_parts;
    registry.world_sprites = pz_world::save::items::moveable_sprites(sprites);
    let jar = pz_classfile::Jar::open(game.join("projectzomboid.jar")).unwrap();
    let ectx = pz_entity::BasicContext {
        attributes: pz_entity::AttributeTypes::from_jar(&jar).unwrap(),
        fluids: Some(pz_entity::FluidRegistry::from_scripts(&jar, sm).unwrap()),
        ..Default::default()
    };
    let ectx: &'static pz_entity::BasicContext = Box::leak(Box::new(ectx));
    registry.components = Box::new(pz_inventory::component::EntityCodec { ctx: ectx });
    registry.assets = Some(pz_inventory::ItemAssets::load(game).unwrap());
    registry
}

/// Everything a [`StreamCtx`] borrows.
struct Env {
    stream_meta: IsoMetaGrid,
    sprites: IsoSpriteManager,
    fix2x: Fix2xTable,
    rand: NoRand,
    object_tables: ObjectTables,
    object_hooks: NoHooks,
    save_tables: SaveTables,
    settings: SaveSettings,
    vehicles: VehicleStore,
    scripts: Option<Scripts>,
    items: Box<dyn pz_world::save::items::ItemRegistry>,
}

impl Env {
    fn run(&mut self, work: &Path, f: impl FnOnce(&mut StreamCtx<'_>)) {
        let saver = self
            .scripts
            .as_ref()
            .map(|s| LoadedVehicleSaver { scripts: s });
        let mut ctx = StreamCtx {
            save_dir: work,
            meta: &self.stream_meta,
            sprites: &mut self.sprites,
            fix2x: &self.fix2x,
            rand: &mut self.rand,
            object_tables: &self.object_tables,
            object_settings: ObjectLoadSettings::default(),
            object_hooks: &mut self.object_hooks,
            save_tables: &self.save_tables,
            save_settings: &self.settings,
            items: &*self.items,
            save_hooks: &NoSaveHooks,
            brand_new: None,
            chunk_hook: None,
            no_save: false,
            vehicles_db: self.vehicles.conn.as_mut(),
            vehicle_saver: saver
                .as_ref()
                .map(|s| s as &dyn pz_world::save::vehicles_db::ChunkVehicleSaver),
        };
        f(&mut ctx);
    }
}

/// The chunk part of `IsoCell.save`.
struct Hooks<'a> {
    env: &'a mut Env,
    streamer: &'a mut ChunkStreamer,
    cell: &'a IsoCell,
    work: &'a Path,
}

impl SaveGameHooks for Hooks<'_> {
    fn save_chunks(&mut self, _meta: &IsoMetaGrid) {
        let (s, cell) = (&mut *self.streamer, self.cell);
        self.env.run(self.work, |ctx| {
            s.wait_for_saves(ctx);
            s.save_chunk_maps(cell, ctx);
        });
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 6 && a.len() != 8 {
        eprintln!("usage: save_session <game dir> <save dir> <work dir> <script> <out file> [--scripts <file>]");
        std::process::exit(2);
    }
    let game = Path::new(&a[1]);
    let (save, work) = (Path::new(&a[2]), Path::new(&a[3]));
    assert!(!work.exists(), "work dir must not exist");
    copy_dir(&save.join("map"), &work.join("map"));
    for f in std::fs::read_dir(save).unwrap() {
        let n = f.unwrap().file_name().to_string_lossy().into_owned();
        let wanted = [
            "vehicles.db",
            "map_t.bin",
            "map_meta.bin",
            "map_zone.bin",
            "map_sand.bin",
        ];
        if wanted.contains(&n.as_str()) || (n.starts_with("gos_") && n.ends_with(".bin")) {
            std::fs::copy(save.join(&n), work.join(&n)).unwrap();
        }
    }
    if save.join("isoregiondata").is_dir() {
        copy_dir(&save.join("isoregiondata"), &work.join("isoregiondata"));
    }
    let jar = game.join("projectzomboid.jar");
    let mut sprites = IsoSpriteManager::load_tile_definitions_from(
        SpriteTables::from_game_jar(&jar).unwrap(),
        &game.join("media"),
        &[],
    )
    .unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let mut stream_meta = IsoMetaGrid::empty();
    (stream_meta.max_x, stream_meta.max_y) = (99, 99);
    // `VehiclesDB2.init` → `SQLStore.create` on the copy.
    let mut vehicles = VehicleStore::default();
    vehicles
        .create(&work.to_string_lossy())
        .expect("vehicles.db");
    let mut env = Env {
        stream_meta,
        sprites,
        fix2x: Fix2xTable::default(),
        rand: NoRand,
        object_tables: ObjectTables::from_game_jar(&jar).unwrap(),
        object_hooks: NoHooks,
        save_tables: SaveTables::from_game_jar(&jar).unwrap(),
        settings: SaveSettings::default(),
        vehicles,
        items: Box::new(AllItems),
        scripts: (a.len() == 8 && a[6] == "--scripts").then(|| read_scripts(Path::new(&a[7]))),
    };

    // The save-side world state: game time, meta grid and zones, sandbox,
    // global objects.
    let (mut gt, mut core) = (GameTime::default(), Core::default());
    game_time_save::load_map_t_file(&mut gt, &mut core, work, -1).expect("map_t.bin");
    let mut spawned = SpawnedZombieZones::default();
    let mut uuids = ClockUuids::default();
    let (mut meta, meta_world) = meta_grid(
        &std::fs::read(work.join("map_meta.bin")).unwrap(),
        work,
        &mut spawned,
        &mut uuids,
    );
    let jarf = pz_classfile::Jar::open(&jar).unwrap();
    let defs = pz_sandbox::defs::builtin_options(&jarf).unwrap();
    let mut sandbox = pz_sandbox::SandboxOptions::new(&defs);
    let preset = sandbox
        .load_bin(&std::fs::read(work.join("map_sand.bin")).unwrap())
        .unwrap();
    let sandbox_bin =
        sandbox.save_bin(pz_sandbox::save_bin::world_version(&jarf).unwrap(), &preset);
    // `SaveSettings` from the loaded game time and sandbox.
    let opt = |n: &str| {
        sandbox
            .option_by_name(n)
            .map(|o| o.config.get_value_as_string())
            .unwrap_or_else(|| panic!("sandbox option {n}"))
    };
    env.settings.world_age_hours = gt.world_age_hours();
    env.settings.calendar_millis = gt.calendar_time_in_millis();
    env.settings.blood_splat_lifespan_days = opt("BloodSplatLifespanDays").parse().unwrap();
    env.settings.hours_for_world_item_removal = opt("HoursForWorldItemRemoval").parse().unwrap();
    env.settings
        .set_world_item_removal_list(&opt("WorldItemRemovalList"));
    env.settings.item_removal_list_blacklist = opt("ItemRemovalListBlacklistToggle") == "true";
    env.items = Box::new(script_items(
        game,
        save,
        &env.sprites,
        &env.save_tables,
        gt.world_age_hours(),
    ));
    let _systems = load_global_objects(work);
    let game_map = pz_lua_api::savefile::read_map_ver(&save.join("map_ver.bin").to_string_lossy())
        .and_then(|v| v.map_name);

    let script = std::fs::read_to_string(&a[4]).unwrap();
    let mut lines = script.lines();
    let width: i32 = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut s = ChunkStreamer::new(1, width);
    let mut cell = IsoCell::default();
    let mut out = String::new();
    let mut last = (0.0f32, 0.0f32);
    for l in lines {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        out.push_str(&format!("## {l}\n"));
        let t: Vec<&str> = l.split_whitespace().collect();
        match t[0] {
            "load" => {
                let (wx, wy) = (t[1].parse().unwrap(), t[2].parse().unwrap());
                env.run(work, |ctx| s.load_world(wx, wy, &mut cell, ctx));
                last = ((wx * 8 + 4) as f32, (wy * 8 + 4) as f32);
            }
            "frame" => {
                let (x, y): (f32, f32) = (t[1].parse().unwrap(), t[2].parse().unwrap());
                let serve = match t[3] {
                    "all" => Serve::UntilIdle,
                    n => Serve::Jobs(n.parse().unwrap()),
                };
                let p = PlayerPos::moved(last.0, last.1, x, y);
                env.run(work, |ctx| s.update(&[Some(p)], serve, &mut cell, ctx));
                last = (x, y);
            }
            "quit" => {
                let mut g = SaveGame {
                    save_dir: work,
                    no_save: false,
                    has_cell: true,
                    game_mode: "Sandbox",
                    game_map: game_map.as_deref(),
                    sandbox: &sandbox_bin,
                    game_time: &gt,
                    core: &core,
                    meta: &mut meta,
                    meta_world: &meta_world,
                    spawned_zones: &spawned,
                    uuids: &mut uuids,
                    players: None,
                    can_save_players: true,
                    reanimated: Some(&[]),
                };
                let mut hooks = Hooks {
                    env: &mut env,
                    streamer: &mut s,
                    cell: &cell,
                    work,
                };
                let r = world_save::save_on_quit(&mut g, &mut [], &mut hooks);
                out.push_str(&format!("# save {r:?}\n"));
                env.run(work, |ctx| s.stop(&mut cell, ctx));
                world_save::close_stores(None, &mut env.vehicles);
            }
            _ => panic!("bad script line {l}"),
        }
        for e in s.take_events() {
            if e == StreamEvent::HotSave {
                let errors = world_save::hot_save_ancillary(&mut HotSave {
                    save_dir: work,
                    no_save: false,
                    meta: &mut meta,
                    meta_world: &meta_world,
                    spawned_zones: &spawned,
                    uuids: &mut uuids,
                    game_time: &gt,
                    core: &core,
                });
                assert!(errors.is_empty(), "{errors:?}");
            }
            if let Some(t) = line(&e) {
                out.push_str(&t);
                out.push('\n');
            }
        }
    }
    std::fs::write(&a[5], out).unwrap();
}
