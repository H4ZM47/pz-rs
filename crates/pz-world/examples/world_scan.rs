//! Load the installed game's meta grid and print a summary.
//!
//! `cargo run --release -p pz-world --example world_scan -- <dir containing media/> [map] [x y z]`

use pz_fs::GameFs;
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
    if args.len() >= 6 {
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
        let sprites = pz_world::TileDefSprites::load(&std::path::Path::new(&args[1]).join("media"))
            .expect("tiles");
        let mut cell = pz_world::IsoCell::default();
        let t = std::time::Instant::now();
        let ch = cell.load_chunk(&grid, x.div_euclid(8), y.div_euclid(8), &sprites, &mut rand);
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
                sq.room_id,
                sq.objects
                    .iter()
                    .map(|o| &o.sprite_name)
                    .collect::<Vec<_>>()
            );
        }
    }
}
