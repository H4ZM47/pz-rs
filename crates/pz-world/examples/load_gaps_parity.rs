//! Loads every chunk of a save against the map's meta grid and attaches
//! the chunk's `vehicles.db` rows, writing what a local harness writes for
//! the original (`IsoChunk.LoadFromDiskOrBufferInternal` and
//! `VehiclesDB2.loadChunk`), for comparison. No game data is part of this
//! repository.
//!
//! ```text
//! load_gaps_parity <game dir> <sprite tables> <save tables> <save dir> <oracle dir> <out dir> <game map>
//! ```
//!
//! `<save dir>` is a scratch copy (rows that fail to load are deleted from
//! its `vehicles.db`, as in the original). `<oracle dir>` holds the
//! harness's `items.txt` (registry id, full type, creatable, obsolete) and
//! `summary.txt` (world age). Writes `assoc.txt` (every loaded square's
//! `associatedBuilding` id), `vehicles.txt` (each chunk's vehicle list in
//! order) and `remaining.txt` (the row ids left) into `<out dir>`.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use pz_fs::GameFs;
use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_world::chunk::IsoChunk;
use pz_world::chunk_vehicles::{LoadedVehicle, VehicleOrigin};
use pz_world::save::chunk_file::{chunk_path, load_chunk};
use pz_world::save::vehicles_db;
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::{
    CreateParams, GameRand, IsoMetaGrid, IsoSpriteManager, SpriteTables, WorldGenBounds,
};

struct Zero;
impl GameRand for Zero {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let save_tables = SaveTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let save_dir = Path::new(&a[4]);
    let oracle = Path::new(&a[5]);
    let out = Path::new(&a[6]);
    std::fs::create_dir_all(out).unwrap();
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let mut tile_scale = 2;
    for line in std::fs::read_to_string(oracle.join("items.txt"))
        .unwrap()
        .lines()
    {
        if let Some(t) = line.strip_prefix("#tileScale\t") {
            tile_scale = t.parse().unwrap();
        }
    }
    let summary = std::fs::read_to_string(oracle.join("summary.txt")).unwrap();
    let world_age: f64 = summary
        .split_whitespace()
        .find_map(|w| w.strip_prefix("worldAgeHours="))
        .unwrap()
        .parse()
        .unwrap();
    let settings = SaveSettings {
        world_age_hours: world_age,
        world_item_removal_list: "Base.Hat, Base.Glasses, Base.Maggots, Base.Slug, Base.Slug2, Base.Snail, Base.Worm, Base.Dung_Mouse, Base.Dung_Rat"
            .split(',')
            .map(|s| s.trim().to_string())
            .collect(),
        tile_scale,
        ..Default::default()
    };
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game.join("projectzomboid.jar"))
        .expect("built-in registries");
    sm.load(&GameFs::new(game.to_str().unwrap()), &[])
        .expect("scripts");
    let dict = WorldDictionary::load(&std::fs::read(save_dir.join("WorldDictionary.bin")).unwrap())
        .unwrap();
    let names = pz_inventory::factory::parse_item_names(
        &std::fs::read_to_string(game.join("media/lua/shared/Translate/EN/ItemName.json")).unwrap(),
    );
    let mut registry = ScriptItems::new(&sm, dict, &names);
    registry.world_age_hours = world_age;
    registry.blood_body_parts = save_tables.blood_body_parts;
    registry.world_sprites = pz_world::save::items::moveable_sprites(&sprites);
    let jar = pz_classfile::Jar::open(game.join("projectzomboid.jar")).unwrap();
    let ectx = pz_entity::BasicContext {
        attributes: pz_entity::AttributeTypes::from_jar(&jar).unwrap(),
        fluids: Some(pz_entity::FluidRegistry::from_scripts(&jar, &sm).unwrap()),
        ..Default::default()
    };
    let ectx: &'static pz_entity::BasicContext = Box::leak(Box::new(ectx));
    registry.components = Box::new(pz_inventory::component::EntityCodec { ctx: ectx });
    registry.assets = Some(pz_inventory::ItemAssets::load(game).unwrap());

    let t0 = std::time::Instant::now();
    let grid = IsoMetaGrid::create(
        &GameFs::new(game.to_str().unwrap()),
        CreateParams {
            game_map: &a[7],
            world_gen: WorldGenBounds::default(),
            rand: &mut Zero,
            voronois: Vec::new(),
        },
    )
    .expect("meta grid");
    let mut grid = grid;
    // The rest of `CreateStep2`: rooms added to adjacent cells (alarms do
    // not matter here).
    grid.assign_alarms(&mut Zero, false, true, 1, || 0);
    eprintln!(
        "meta grid {:?} buildings={}",
        t0.elapsed(),
        grid.buildings.len()
    );
    let ctx = SaveCtx {
        sprites: &sprites,
        tables: &save_tables,
        settings: &settings,
        items: &registry,
        hooks: &NoSaveHooks,
        meta: Some(&grid),
    };

    // `SQLStore.create` on an existing file: auto-commit off.
    let mut conn = pz_sqlite::Connection::open(save_dir.join("vehicles.db")).unwrap();
    conn.set_autocommit(false).unwrap();
    let (used, seen) = vehicles_db::init_used_ids(&mut conn).unwrap();
    eprintln!("vehicle usedIds={} seenChunks={}", used.len(), seen.len());

    let mut files = BTreeSet::new();
    for d in std::fs::read_dir(save_dir.join("map")).unwrap() {
        let d = d.unwrap();
        let Ok(wx) = d.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        for f in std::fs::read_dir(d.path()).unwrap() {
            let n = f.unwrap().file_name().to_string_lossy().to_string();
            if let Some(wy) = n.strip_suffix(".bin").and_then(|s| s.parse::<i32>().ok()) {
                files.insert((wx, wy));
            }
        }
    }
    let row_chunks: BTreeSet<(i32, i32)> = {
        let t = conn.table("vehicles").unwrap();
        let (cx, cy) = (t.column("wx").unwrap(), t.column("wy").unwrap());
        conn.rows(&t)
            .unwrap()
            .iter()
            .map(|r| (r.get(&t, cx).as_i32(), r.get(&t, cy).as_i32()))
            .collect()
    };
    let all: BTreeSet<(i32, i32)> = files.union(&row_chunks).copied().collect();
    let mut assoc = String::new();
    let mut veh = String::new();
    let mut counts = HashMap::<&str, usize>::new();
    for &(wx, wy) in &all {
        let has_file = files.contains(&(wx, wy));
        let mut ch = if has_file {
            let bytes = std::fs::read(chunk_path(save_dir, wx, wy)).unwrap();
            match load_chunk(&bytes, wx, wy, &ctx) {
                Ok(l) => l.chunk,
                Err(e) => {
                    *counts.entry("errors").or_default() += 1;
                    writeln!(assoc, "chunk {wx},{wy} ERROR {e}").unwrap();
                    continue;
                }
            }
        } else {
            *counts.entry("noFile").or_default() += 1;
            IsoChunk::new(wx, wy)
        };
        if has_file {
            writeln!(assoc, "chunk {wx},{wy}").unwrap();
            for x in 0..8 {
                for y in 0..8 {
                    for z in ch.min_level..=ch.max_level {
                        let Some(sq) = ch.get_grid_square(x, y, z) else {
                            continue;
                        };
                        *counts.entry("squares").or_default() += 1;
                        let id = sq.associated_building.map_or(-1, |b| {
                            *counts.entry("assoc").or_default() += 1;
                            grid.building_defs[b.0 as usize].id
                        });
                        writeln!(assoc, " {},{},{} {id}", sq.x, sq.y, sq.z).unwrap();
                    }
                }
            }
        }
        if !row_chunks.contains(&(wx, wy)) {
            continue;
        }
        let r = vehicles_db::load_chunk(Some(&mut conn), &mut ch, &ctx);
        if let Some(e) = r.error {
            eprintln!("chunk {wx},{wy}: {e}");
        }
        let vs: Vec<std::rc::Rc<LoadedVehicle>> = ch.vehicles.loaded().collect();
        writeln!(
            veh,
            "chunk {wx},{wy}{} n={}",
            if has_file { "" } else { " nofile" },
            vs.len()
        )
        .unwrap();
        for v in vs {
            *counts.entry("vehicles").or_default() += 1;
            let VehicleOrigin::Database { sql_id } = v.origin else {
                panic!("chunk file vehicle");
            };
            let sq = v
                .square
                .map_or("null".to_string(), |(x, y, z)| format!("{x},{y},{z}"));
            let b = &v.vehicle.base;
            writeln!(
                veh,
                " v {sql_id} sq={sq} script={} x={} y={} z={} parts={}",
                v.vehicle.script_name,
                b.x.to_bits() as i32,
                b.y.to_bits() as i32,
                b.z.to_bits() as i32,
                v.vehicle.parts.len()
            )
            .unwrap();
        }
    }
    let (left, _) = vehicles_db::init_used_ids(&mut conn).unwrap();
    let mut rem = String::new();
    for id in left {
        writeln!(rem, "{id}").unwrap();
    }
    std::fs::write(out.join("assoc.txt"), assoc).unwrap();
    std::fs::write(out.join("vehicles.txt"), veh).unwrap();
    std::fs::write(out.join("remaining.txt"), rem).unwrap();
    let c = |k| counts.get(k).copied().unwrap_or(0);
    println!(
        "files={} rowChunks={} noFile={} errors={} squares={} assoc={} vehicles={}",
        files.len(),
        row_chunks.len(),
        c("noFile"),
        c("errors"),
        c("squares"),
        c("assoc"),
        c("vehicles")
    );
}
