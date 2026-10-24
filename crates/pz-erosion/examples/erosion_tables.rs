//! Prints a summary of the erosion tables read from a game jar:
//! `cargo run -p pz-erosion --example erosion_tables -- <projectzomboid.jar>`.
fn main() {
    let jar = std::env::args()
        .nth(1)
        .expect("usage: erosion_tables <projectzomboid.jar>");
    let t = pz_erosion::tables::ErosionTables::from_jar_path(jar).unwrap();
    println!("perm {}", t.noise_perm.len());
    println!(
        "soil_table {}x{}",
        t.soil_table.len(),
        t.soil_table.first().map_or(0, |r| r.len())
    );
    println!("square_rands {}", t.square_rands.len());
    println!("trees {} soilRef {}", t.trees.len(), t.trees_soil_ref.len());
    println!(
        "plants {} soilRef {}",
        t.plants.len(),
        t.plants_soil_ref.len()
    );
    println!("bush {} soilRef {}", t.bush.len(), t.bush_soil_ref.len());
    println!("flowerbed tiles {}", t.flowerbed_tile_ids.len());
    println!("season names {}", t.season_names.len());
    println!("config defaults {}", t.config_defaults.len());
}
