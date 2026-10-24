//! Prints the `ServerOptions` option constructors read from a local game
//! jar (output stays local; it is game data).
fn main() {
    let jar = std::env::args().nth(1).expect("path to projectzomboid.jar");
    let jar = pz_classfile::Jar::open(jar).expect("open jar");
    let calls = pz_sandbox::defs::ctor_calls(&jar, "zombie/network/ServerOptions", "zombie/network/ServerOptions$")
        .expect("read options");
    for c in calls {
        println!("{c:?}");
    }
}
