//! Load the installed game's meta grid and print a summary.
//!
//! `cargo run --release -p pz-world --example world_scan -- <dir containing media/> [map] [x y z <sprite tables> <object tables>]`
//!
//! The two tables files hold enum constants and lookup tables read from the
//! installed game (see `SpriteTables::from_text`, `ObjectTables::from_text`);
//! they are not part of this repository.

use pz_fs::GameFs;
use pz_world::ObjectSlot;
use pz_world::{CreateParams, GameRand, IsoMetaGrid, WorldGenBounds};

/// Placeholder generator for inspection (not the game's `Rand`).
struct Lcg(u64);
impl GameRand for Lcg {
    fn next(&mut self, max: i32) -> i32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        if max <= 0 {
            0
        } else {
            ((self.0 >> 33) % max as u64) as i32
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let fs = GameFs::new(args.get(1).expect("game dir"));
    let map = args.get(2).map(String::as_str).unwrap_or("Muldraugh, KY");
    let t = std::time::Instant::now();
    let mut rand = Lcg(42);
    let grid = IsoMetaGrid::create(
        &fs,
        CreateParams {
            game_map: map,
            world_gen: WorldGenBounds::default(),
            rand: &mut rand,
            voronois: Vec::new(),
        },
    )
    .expect("create");
    println!(
        "cells X [{} {}] Y [{} {}] in {:?}",
        grid.min_x,
        grid.max_x,
        grid.min_y,
        grid.max_y,
        t.elapsed()
    );
    println!(
        "map dirs: {:?}",
        grid.map_files
            .iter()
            .map(|m| (&m.map_directory_name, m.priority, m.info_headers.len()))
            .collect::<Vec<_>>()
    );
    println!(
        "rooms {} building defs {} registered buildings {}",
        grid.room_defs.len(),
        grid.building_defs.len(),
        grid.buildings.len()
    );
    if args.len() >= 8 {
        let (x, y, z): (i32, i32, i32) = (
            args[3].parse().unwrap(),
            args[4].parse().unwrap(),
            args[5].parse().unwrap(),
        );
        match grid.get_room_at(x, y, z) {
            Some(r) => println!(
                "room at {x},{y},{z}: {} id={} metaId={} bounds=({},{})-({},{})",
                r.name, r.id, r.meta_id, r.x, r.y, r.x2, r.y2
            ),
            None => println!("no room at {x},{y},{z}"),
        }
        let tables = pz_world::SpriteTables::from_text(&std::fs::read_to_string(&args[6]).unwrap())
            .expect("sprite tables");
        let objects =
            pz_world::ObjectTables::from_text(&std::fs::read_to_string(&args[7]).unwrap())
                .expect("object tables");
        let media = std::path::Path::new(&args[1]).join("media");
        let mut sprites =
            pz_world::IsoSpriteManager::load_tile_definitions_from(tables, &media, &[])
                .expect("tiles");
        sprites.loaded_tile_definitions(&|_| false).expect("loaded");
        let blood_body_parts = pz_world::save::SaveTables::from_game_jar(&std::path::Path::new(&args[1]).join("projectzomboid.jar")).expect("visual body tables").blood_body_parts;
        let mut cell = pz_world::IsoCell::default();
        let t = std::time::Instant::now();
        let ch = cell.load_chunk(
            &grid,
            x.div_euclid(8),
            y.div_euclid(8),
            &mut pz_world::chunk::ChunkLoadEnv {
                blood_body_parts: blood_body_parts,
                sprites: &mut sprites,
                // Muldraugh is fixed2x; other maps need the table from the jar.
                fix2x: &pz_world::fix2x::Fix2xTable::default(),
                rand: &mut rand,
                tables: &objects,
                settings: pz_world::ObjectLoadSettings::default(),
                hooks: &mut pz_world::NoHooks,
            },
        );
        println!(
            "chunk {},{} levels {}..{}: {} squares in {:?}",
            ch.wx,
            ch.wy,
            ch.min_level,
            ch.max_level,
            ch.squares().count(),
            t.elapsed()
        );
        if let Some(sq) = cell.get_grid_square(x, y, z) {
            println!(
                "square {x},{y},{z} room={} objects={:?}",
                sq.borrow().room_id,
                sq.borrow()
                    .objects
                    .iter()
                    .map(ObjectSlot::borrow)
                    .map(|o| (o.kind.class_name(), o.tile.clone().unwrap_or_default()))
                    .collect::<Vec<_>>()
            );
        }
    }
}
