//! Load → save of the files `GameWindow.save` and `IsoMetaGrid.save` write
//! besides the chunks, for comparing against the original's own load →
//! save: `map_worldgen.bin`, `iTrack.bin` (and `ItemTracker.log`),
//! `metadata.bin`, `statistics.bin`, `map_animals.bin` and
//! `metagrid/metacell_*.bin` (every loaded cell queued again).
//!
//! `cargo run -p pz-lua-api --example save_extras_roundtrip -- <save dir> <out dir>`
//! prints the meta grid bounds (from `map_meta.bin`) the original's grid
//! needs.

use std::path::Path;

use pz_core::save_trackers::{InstanceTracker, MetaTracker, Statistics};
use pz_lua_api::world_save;
use pz_world::meta_save::{DefaultMetaEnv, MapMetaFile, NetRole};
use pz_world::zone_save::ClockUuids;
use pz_world::IsoMetaGrid;
use pz_worldgen::params::WorldGenParams;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (save, out) = (Path::new(&a[1]), Path::new(&a[2]));
    std::fs::create_dir_all(out.join("metagrid")).unwrap();
    let meta_bytes = std::fs::read(save.join("map_meta.bin")).expect("map_meta.bin");
    let file = MapMetaFile::parse(&meta_bytes, NetRole::SinglePlayer, &mut DefaultMetaEnv)
        .expect("map_meta.bin");
    let c = &file.cells;
    println!("bounds {} {} {} {}", c.min_x, c.min_y, c.max_x, c.max_y);

    let mut wg = WorldGenParams::new("");
    println!("WorldGenParams.load={:?}", wg.load_file(save));
    wg.save_file(out, false).unwrap();
    let mut it = InstanceTracker::default();
    it.load_file(save, false).unwrap();
    it.save_file(out, false).unwrap();
    std::fs::write(out.join("ItemTracker.log"), it.item_tracker_log()).unwrap();
    let mut mt = MetaTracker::default();
    mt.load_file(save, false, false).unwrap();
    mt.save_file(out, false).unwrap();
    let mut st = Statistics::default();
    st.load_file(save, false).unwrap();
    st.save_file(out, false).unwrap();
    println!("statistics={}", st.values().len());

    let mut g = IsoMetaGrid::with_bounds(c.min_x, c.min_y, c.max_x, c.max_y);
    let errors = world_save::load_meta_grid_extras(&mut g, save);
    assert!(errors.is_empty(), "{errors:?}");
    println!(
        "animal zones={} cellsToSave after load={}",
        g.animal_zones.len(),
        g.cells_to_save.borrow().len()
    );
    let mut uuids = ClockUuids::default();
    g.save_animal_zones_file(out, &mut uuids).unwrap();
    let mut cells = g.load_cells_files(save).unwrap();
    cells.sort();
    for (x, y) in &cells {
        g.add_cell_to_save(*x, *y);
    }
    g.save_cells_files(out).unwrap();
    println!("metacells queued={}", cells.len());
}
