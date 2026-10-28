//! Prints the sandbox option definitions read from a local game jar
//! (output stays local; it is game data).
fn main() {
    let jar = std::env::args().nth(1).expect("path to projectzomboid.jar");
    let jar = pz_classfile::Jar::open(jar).expect("open jar");
    for d in pz_sandbox::defs::builtin_options(&jar).expect("read options") {
        println!("{d:?}");
    }
}
