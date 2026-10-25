//! Prints `IsoZombie`'s close-sneak-bonus table read from the installed
//! jar: `cargo run -p pz-scripts --example zombie_tables -- <projectzomboid.jar>`.

fn main() {
    let path = std::env::args().nth(1).expect("jar path");
    let jar = pz_classfile::Jar::open(path).expect("jar");
    let t = pz_scripts::registry::builtin::BuiltinTables::load(&jar).expect("tables");
    for (k, d, v) in &t.zombie_close_sneak_bonus {
        println!("{k} {d} {v}");
    }
}
