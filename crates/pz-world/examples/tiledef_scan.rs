//! Read the installed game's `.tiles` files and print counts.
//!
//! `cargo run --release -p pz-world --example tiledef_scan -- <dir containing media/>`

use pz_world::tiledef::{TileDefFile, TILE_DEFINITION_FILES};

fn main() {
    let base = std::path::PathBuf::from(std::env::args().nth(1).expect("game dir"));
    for (name, number) in TILE_DEFINITION_FILES {
        match TileDefFile::read(&base.join("media").join(name), number) {
            Ok(f) => {
                let tiles: usize = f.tilesets.iter().map(|t| t.tiles.len()).sum();
                let props: usize = f
                    .tilesets
                    .iter()
                    .flat_map(|t| &t.tiles)
                    .map(|t| t.props.len())
                    .sum();
                println!(
                    "{name}: {} tilesets, {tiles} tiles, {props} props",
                    f.tilesets.len()
                );
            }
            Err(e) => println!("{name}: error {e}"),
        }
    }
}
