//! Reads each `map_basements.bin` given on the command line with
//! [`Basements::load_savefile`] and writes it back with
//! [`Basements::savefile_bytes`] (with the world version the file holds),
//! reporting whether the bytes match.
//!
//! ```text
//! basements_roundtrip <map_basements.bin>...
//! ```
//!
//! Only counts and match results are printed.

use std::rc::Rc;

use pz_fs::GameFs;
use pz_worldgen::basements::Basements;

fn main() {
    let mut failed = false;
    for arg in std::env::args().skip(1) {
        let path = std::path::Path::new(&arg);
        let original = std::fs::read(path).expect("read");
        let mut b = Basements::new(Rc::new(GameFs::new("/nonexistent")));
        match b.load_savefile(path) {
            Ok(true) => {}
            Ok(false) => {
                println!("{arg}: missing");
                failed = true;
                continue;
            }
            Err(e) => {
                println!("{arg}: load failed: {e}");
                failed = true;
                continue;
            }
        }
        let rooms: usize = b.buildings.iter().map(|b| b.rooms.len()).sum();
        let version = i32::from_be_bytes(original[4..8].try_into().unwrap());
        let saved = b.savefile_bytes(version).expect("save");
        let same = saved == original;
        failed |= !same;
        println!(
            "{arg}: world version {version}, {} bytes, {} placements, {} buildings, {rooms} rooms, re-saved identical: {same}",
            original.len(),
            b.placements.len(),
            b.buildings.len(),
        );
    }
    std::process::exit(i32::from(failed));
}
