//! Compare the map zones the shipped Lua registers on `OnLoadMapZones`
//! (`metazoneHandler.lua`'s `doMapZones`, `doSpawnOrigins`,
//! `initDefaultStreetData` and `basements.lua`'s
//! `loadMapBasementLuaFiles`) through the ported `IsoWorld` /
//! `IsoMetaGrid` bindings with the zones a save of the original game holds
//! (`map_zone.bin` is `IsoMetaGrid.zones` as the original registered them).
//!
//! Usage: `map_zones_parity <game-dir> <save-dir> <cache-dir>`. The save
//! (and the cache dir) should be scratch copies. Its `map_ver.bin` gives
//! `Core.gameMap`, its `map_worldgen.bin` the grid bounds.

use pz_world::zones::{ClipperOps, Zone};
use pz_world::{CreateParams, GameRand, IsoMetaGrid, WorldGenBounds};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

struct Zero;
impl GameRand for Zero {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

#[derive(Default)]
struct Clip(pz_clipper::ZoneClipper);
impl ClipperOps for Clip {
    fn offset_polyline(&mut self, points: &[f32], delta: f64) -> Option<Vec<f32>> {
        self.0.offset_polyline(points, delta)
    }
    fn clip_polygon_aabb(
        &mut self,
        points: &[f32],
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    ) -> Vec<Vec<f32>> {
        self.0.clip_polygon_aabb(points, x1, y1, x2, y2)
    }
}

/// The saved fields of a zone.
fn key(z: &Zone) -> String {
    format!(
        "{}|{}|{},{},{} {}x{} {:?} w{} {:?}",
        z.name, z.zone_type, z.x, z.y, z.z, z.w, z.h, z.geometry_type, z.polyline_width, z.points
    )
}

fn grid(
    fs: &pz_fs::GameFs,
    game_map: &str,
    bounds: WorldGenBounds,
    jar: &std::path::Path,
) -> IsoMetaGrid {
    let mut g = IsoMetaGrid::create(
        fs,
        CreateParams {
            game_map,
            world_gen: bounds,
            rand: &mut Zero,
            voronois: Vec::new(),
        },
    )
    .expect("IsoMetaGrid.Create");
    g.zone_tables = pz_world::zones::ZoneTables::load_from_game_jar(jar).expect("zone tables");
    g.clipper = Some(Box::new(Clip::default()));
    g
}

fn main() {
    let mut args = std::env::args().skip(1);
    let game = PathBuf::from(args.next().expect("game directory containing media/"));
    let save = PathBuf::from(args.next().expect("save directory"));
    let cache = PathBuf::from(args.next().expect("scratch cache directory"));
    let jar = game.join("projectzomboid.jar");
    let map = pz_lua_api::savefile::read_map_ver(&save.join("map_ver.bin").to_string_lossy())
        .and_then(|v| v.map_name)
        .expect("map_ver.bin map name");
    let mut params = pz_worldgen::WorldGenParams::new("");
    let _ = params.load_file(&save);
    let bounds = WorldGenBounds {
        min_x: params.min_x_cell,
        min_y: params.min_y_cell,
        max_x: params.max_x_cell,
        max_y: params.max_y_cell,
    };
    let mut gfs = pz_fs::GameFs::new(&game);
    gfs.set_cache_dir(&cache);
    let fs = Rc::new(RefCell::new(gfs));
    let mut lua = pz_lua_api::LuaManager::init(fs.clone()).expect("LuaManager.init");
    pz_lua_api::bind_core::shared_core().borrow_mut().game_map = map.clone();
    let g = grid(&fs.borrow(), &map, bounds, &jar);
    pz_lua_api::bind_world::set_meta_grid(Some(g));
    for f in [
        "media/lua/server/metazones/metazoneHandler.lua",
        "media/lua/shared/ProceduralBasements/basements.lua",
    ] {
        let abs = fs.borrow().resolve(f).expect(f);
        pz_lua_api::lua_manager::run_lua(&mut lua.thread, &lua.loader, &abs.to_string_lossy());
    }
    pz_lua_api::bind_basements::before_on_load_map_zones();
    lua.trigger_event("OnLoadMapZones", &[]);
    for e in &lua.loader.borrow().errors {
        println!("LUA ERROR {e}");
    }
    let ours: Vec<String> = pz_lua_api::bind_world::with_world(|w| {
        let g = w.meta_grid.as_ref().unwrap();
        g.zones.iter().map(|&id| key(g.zone(id))).collect()
    });
    let mut saved = grid(&fs.borrow(), &map, bounds, &jar);
    let mut spawned = pz_world::zone_save::SpawnedZombieZones::default();
    let mut ids = pz_world::zone_save::ClockUuids::default();
    let rep = saved
        .load_zones_file(&save, &mut spawned, &mut ids)
        .expect("map_zone.bin")
        .expect("map_zone.bin exists");
    let theirs: Vec<String> = saved.zones.iter().map(|&id| key(saved.zone(id))).collect();
    println!("map {map}");
    println!(
        "zone tables: preferred {:?}, directions {:?}",
        saved.zone_tables.preferred_zone_types, saved.zone_tables.iso_direction_names
    );
    println!(
        "saved zones {} (read {}, skipped WorldGen {}), registered zones {}",
        theirs.len(),
        rep.read,
        rep.skipped_world_gen,
        ours.len()
    );
    let mut by_type: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for k in &ours {
        by_type
            .entry(k.split('|').nth(1).unwrap().to_string())
            .or_default()
            .0 += 1;
    }
    for k in &theirs {
        by_type
            .entry(k.split('|').nth(1).unwrap().to_string())
            .or_default()
            .1 += 1;
    }
    for (t, (a, b)) in &by_type {
        println!(
            "type {t:?}: registered {a} saved {b}{}",
            if a == b { "" } else { "  <-- differs" }
        );
    }
    let same = ours.iter().zip(&theirs).take_while(|(a, b)| a == b).count();
    println!(
        "identical prefix {same} of {}",
        ours.len().max(theirs.len())
    );
    let mut shown = 0;
    for (i, (a, b)) in ours.iter().zip(&theirs).enumerate() {
        if a != b && shown < 10 {
            println!("#{i}\n  ours   {a}\n  theirs {b}");
            shown += 1;
        }
    }
    let (mut sa, mut sb) = (ours.clone(), theirs.clone());
    sa.sort();
    sb.sort();
    println!("same multiset: {}", sa == sb);
    let origins = pz_lua_api::bind_world::lua_world::take_spawn_origins();
    println!("spawn origins {}", origins.len());
    let water = pz_lua_api::bind_world::lua_world::shared_water_flow();
    println!(
        "water flow points {}, water zones {}",
        water.borrow().points().count(),
        water.borrow().zones.len()
    );
    let basements = pz_lua_api::bind_basements::per_map();
    for b in &basements {
        println!(
            "basements {:?}: {} definitions, {} access, {} spawn locations",
            b.map_id,
            b.definitions.len(),
            b.access_definitions.len(),
            b.spawn_locations.len()
        );
    }
}
