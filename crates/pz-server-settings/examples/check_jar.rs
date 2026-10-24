//! Reads the server option list and file-name rules from a local game jar
//! and prints counts (the values themselves are game data and stay local).
fn main() {
    let jar = std::env::args().nth(1).expect("path to projectzomboid.jar");
    let jar = pz_classfile::Jar::open(jar).expect("open jar");
    let defs = pz_server_settings::options::read_defs(&jar).expect("server options");
    let opts = pz_server_settings::ServerOptions::new(&defs);
    let reserved = pz_server_settings::settings::reserved_names(&jar).expect("reserved names");
    println!(
        "options {} public {} private {:?} reserved {}",
        opts.num_options(),
        opts.public_options().len(),
        defs.private_names,
        reserved.len()
    );
}
