//! Round-trips `map_meta.bin` and `map_zone.bin` of save folders.
//!
//! For each folder: the raw `map_meta.bin` parse/write; `map_zone.bin`
//! loaded into an empty grid with the bounds `map_meta.bin` records; and,
//! with `--game <dir containing media/>`, `map_meta.bin` applied to the
//! meta grid built from the installed maps listed in `map_ver.bin` and
//! saved from it. With `--standin-out <dir>`, `map_meta.bin` is also loaded
//! into a grid holding exactly the rooms and buildings the file names and
//! the result written there. With `--mutate <out-dir>`, also writes changed
//! copies of both files (named after the folder) for loading in the
//! original game.
//!
//! `world_save_roundtrip [--game <dir>] [--standin-out <dir>] [--mutate <out-dir>] <save-dir>...`

use pz_fs::GameFs;
use pz_kahlua::java_io::{JavaReader, JavaUuid};
use pz_world::meta_save::{
    load_map_meta, save_map_meta, DefaultMetaEnv, DesignationZone, MapMetaFile, MetaWorld, NetRole,
    SafeHouse,
};
use pz_world::zone_save::{ClockUuids, SpawnedZombieZones};
use pz_world::{
    BuildingDef, BuildingIdx, CreateParams, GameRand, IsoMetaCell, IsoMetaGrid, RoomDef, RoomIdx,
    WorldGenBounds, ZoneGeometryType,
};
use std::path::Path;

struct Zero;
impl GameRand for Zero {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

fn first_diff(a: &[u8], b: &[u8]) -> String {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => format!("first difference at byte {i}"),
        None => format!("lengths {} vs {}", a.len(), b.len()),
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let (mut game, mut mutate, mut standin) = (None, None, None);
    while args.len() >= 2 && args[0].starts_with("--") {
        match args[0].as_str() {
            "--game" => game = Some(args[1].clone()),
            "--mutate" => mutate = Some(args[1].clone()),
            "--standin-out" => standin = Some(args[1].clone()),
            other => panic!("unknown option {other}"),
        }
        args.drain(..2);
    }
    let mut failed = false;
    for dir in &args {
        let dir = Path::new(dir);
        let label = dir.file_name().unwrap().to_string_lossy().into_owned();
        let meta = std::fs::read(dir.join("map_meta.bin")).expect("map_meta.bin");
        let file =
            MapMetaFile::parse(&meta, NetRole::SinglePlayer, &mut DefaultMetaEnv).expect("parse");
        let raw = file.to_bytes(NetRole::SinglePlayer);
        let rooms: usize = file.cells.cells.iter().map(|c| c.0.len()).sum();
        let buildings: usize = file.cells.cells.iter().map(|c| c.1.len()).sum();
        let w = &file.world;
        println!(
            "{label}: map_meta raw {} ({} bytes, cells {}..{} x {}..{}, {rooms} rooms, {buildings} buildings, {} safehouses, {} factions, {} designation zones, stash {:?}, {} unique stories)",
            if raw == meta { "identical".to_string() } else { format!("DIFFERENT, {}", first_diff(&raw, &meta)) },
            meta.len(),
            file.cells.min_x,
            file.cells.max_x,
            file.cells.min_y,
            file.cells.max_y,
            w.safehouses.len(),
            w.factions.len(),
            w.designation_zones.len(),
            w.stash.as_ref().map(|s| (s.possible_stashes.len(), s.buildings_to_do.len(), s.already_read_map.len())),
            w.unique_rds_spawned.len()
        );
        failed |= raw != meta;

        // map_zone.bin through an empty grid of the same bounds.
        let c = &file.cells;
        let mut zgrid = IsoMetaGrid::with_bounds(c.min_x, c.min_y, c.max_x, c.max_y);
        let zone_data = std::fs::read(dir.join("map_zone.bin")).expect("map_zone.bin");
        let mut spawned = SpawnedZombieZones::default();
        let rep = zgrid
            .load_zones(&zone_data, -1, &mut spawned, &mut ClockUuids::default())
            .expect("load zones");
        let zone_out = zgrid
            .save_zones(&spawned, &mut ClockUuids::default())
            .expect("save zones");
        println!(
            "{label}: map_zone {} ({} bytes, {} zones read, {} world-gen dropped, {} suspicious, {} spawned-zombie entries)",
            if zone_out == zone_data { "identical".to_string() } else { format!("DIFFERENT, {}", first_diff(&zone_out, &zone_data)) },
            zone_data.len(),
            rep.read,
            rep.skipped_world_gen,
            rep.suspicious,
            spawned.entries.len()
        );
        failed |= zone_out != zone_data;

        if let Some(game) = &game {
            let ver = std::fs::read(dir.join("map_ver.bin")).expect("map_ver.bin");
            let mut r = JavaReader::new(&ver);
            r.get_i32().unwrap();
            let map = r.read_string_chars().unwrap();
            let fs = GameFs::new(game);
            let mut grid = IsoMetaGrid::create(
                &fs,
                CreateParams {
                    game_map: &map,
                    world_gen: WorldGenBounds {
                        min_x: c.min_x,
                        min_y: c.min_y,
                        max_x: c.max_x,
                        max_y: c.max_y,
                    },
                    rand: &mut Zero,
                    voronois: Vec::new(),
                },
            )
            .expect("create grid");
            let mut world = MetaWorld::default();
            let rep = load_map_meta(
                &mut grid,
                &mut world,
                &meta,
                NetRole::SinglePlayer,
                &mut DefaultMetaEnv,
            )
            .expect("load");
            let out = save_map_meta(&grid, &world, NetRole::SinglePlayer);
            println!(
                "{label}: map_meta via map grid {} (size mismatch {}, {} unknown rooms, {} unknown buildings, {} alarmed)",
                if out == meta { "identical".to_string() } else { format!("DIFFERENT, {}", first_diff(&out, &meta)) },
                rep.size_mismatch,
                rep.unknown_rooms.len(),
                rep.unknown_buildings.len(),
                rep.total_alarmed
            );
            failed |= out != meta;
            // The differences should all come from map content this port
            // does not build (B42 basements): records the map lacks, and
            // buildings whose meta id the basement merge changed.
            let mine = grid.meta_cells();
            let mut matching = 0;
            for (i, (fr, fb)) in file.cells.cells.iter().enumerate() {
                let (gr, gb) = &mine.cells[i];
                let known_r: Vec<_> = fr
                    .iter()
                    .filter(|r| gr.iter().any(|g| g.meta_id == r.meta_id))
                    .collect();
                let known_b: Vec<_> = fb
                    .iter()
                    .filter(|b| gb.iter().any(|g| g.meta_id == b.meta_id))
                    .collect();
                if known_r.len() == gr.len() && known_b.len() == gb.len() {
                    matching += 1;
                }
            }
            println!(
                "{label}:   cells whose map rooms and buildings all appear in the file: {matching} of {}",
                mine.cells.len()
            );
        }

        if let Some(out_dir) = &standin {
            // Load and save through a grid holding exactly the rooms and
            // buildings the file names, in file order (as the Java oracle
            // builds it), for comparison with the original's output.
            let mut g = IsoMetaGrid::with_bounds(c.min_x, c.min_y, c.max_x, c.max_y);
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
                    let mut def = BuildingDef::new(&mut Zero);
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
                &meta,
                NetRole::SinglePlayer,
                &mut DefaultMetaEnv,
            )
            .expect("load");
            let out = save_map_meta(&g, &world, NetRole::SinglePlayer);
            println!(
                "{label}: map_meta via stand-in grid {}",
                if out == meta {
                    "identical".to_string()
                } else {
                    format!("changed, {}", first_diff(&out, &meta))
                }
            );
            std::fs::write(
                Path::new(out_dir).join(format!("{label}.map_meta.bin")),
                out,
            )
            .expect("write");
        }

        if let Some(out_dir) = &mutate {
            let mut m = file.clone();
            for (rooms, buildings) in m.cells.cells.iter_mut().take(40) {
                for r in rooms.iter_mut() {
                    r.explored = !r.explored;
                    r.roof_fixed = true;
                }
                for b in buildings.iter_mut() {
                    b.alarmed = !b.alarmed;
                    b.loot_respawn_hour += 3;
                    b.alarm_decay = 1234;
                }
            }
            let mut sh = SafeHouse::new(100, 200, 10, 12, "rustOwner");
            sh.add_player("guest");
            sh.datetime_created = 1_700_000_000_000;
            sh.location = Some("Muldraugh, KY".into());
            m.world.safehouses.push(sh);
            m.world.designation_zones.push(DesignationZone {
                id: 1234567.0,
                x: 10600,
                y: 9400,
                z: 0,
                w: 6,
                h: 7,
                zone_type: "AnimalZone".into(),
                name: "rust pen".into(),
                hour_last_seen: 99,
            });
            m.world.unique_rds_spawned.push("RustStory".into());
            std::fs::write(
                Path::new(out_dir).join(format!("{label}.map_meta.bin")),
                m.to_bytes(NetRole::SinglePlayer),
            )
            .expect("write");

            let x = c.min_x * 256 + 300;
            let y = c.min_y * 256 + 300;
            zgrid.register_zone("RustForest", "Forest", x, y, 0, 40, 30);
            let pts = [x, y, x + 50, y + 10, x + 20, y + 60];
            zgrid.register_zone_with_geometry(
                "RustRoad",
                "Nav",
                x,
                y,
                0,
                51,
                61,
                ZoneGeometryType::Polyline,
                Some(&pts),
                5,
            );
            spawned.put("RustForest".into(), vec![JavaUuid { msb: 1, lsb: 2 }]);
            let bytes = zgrid
                .save_zones(&spawned, &mut ClockUuids::default())
                .expect("save");
            std::fs::write(
                Path::new(out_dir).join(format!("{label}.map_zone.bin")),
                bytes,
            )
            .expect("write");
        }
    }
    std::process::exit(failed as i32);
}
