//! Loads each `map_sand.bin` given on the command line into
//! `SandboxOptions` (definitions read from the game jar), saves it again
//! and reports whether the bytes are identical, printing every value with
//! `--dump`.
//!
//! With `--compare <dir>`, compares each result with `<dir>/<file name>`
//! (for example the original game's own load+save of the same input)
//! instead of with the input.
//!
//! With `--base <file>`, each load starts from the values in that file
//! made the defaults (a `map_sand.bin` of a freshly constructed original
//! `SandboxOptions`, i.e. after `defines.lua` and the Apocalypse preset),
//! so values the input fails to parse keep the same starting values.
//!
//! `sand_bin_roundtrip <projectzomboid.jar> [--dump] [--base <file>] [--compare <dir>] <map_sand.bin>...`

use pz_classfile::Jar;
use pz_sandbox::{defs::builtin_options, save_bin::world_version, SandboxOptions};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let jar = Jar::open(&args[0]).expect("jar");
    let defs = builtin_options(&jar).expect("defs");
    let wv = world_version(&jar).expect("world version");
    let dump = args.iter().any(|a| a == "--dump");
    let compare = args
        .iter()
        .position(|a| a == "--compare")
        .map(|i| std::path::PathBuf::from(&args[i + 1]));
    let base = args
        .iter()
        .position(|a| a == "--base")
        .map(|i| std::fs::read(&args[i + 1]).expect("base file"));
    let mut files = Vec::new();
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dump" => {}
            "--compare" | "--base" => {
                it.next();
            }
            _ => files.push(a),
        }
    }
    let (mut same, mut n) = (0, 0);
    for f in files {
        let bytes = std::fs::read(f).expect("read");
        let expected = match &compare {
            Some(d) => std::fs::read(d.join(std::path::Path::new(f).file_name().unwrap()))
                .expect("expected file"),
            None => bytes.clone(),
        };
        let mut s = SandboxOptions::new(&defs);
        if let Some(b) = &base {
            s.load_bin(b).expect("base");
            s.set_defaults_to_current_values();
        }
        let preset = s.load_bin(&bytes).expect("load");
        let out = s.save_bin(wv, &preset);
        n += 1;
        if out == expected {
            same += 1;
        } else {
            println!("DIFF {f} ({} vs {} bytes)", out.len(), expected.len());
        }
        if dump {
            for o in &s.options {
                println!("{f}\t{}\t{}", o.name(), o.config.get_value_as_string());
            }
            println!("{f}\tpreset\t{preset}");
        }
    }
    println!("{same}/{n} byte-identical after load+save");
    if same != n {
        std::process::exit(1);
    }
}
