//! Loads every script of an installed game and prints per-type counts and
//! loader diagnostics: `cargo run -p pz-scripts --example load_scripts -- <game dir>`.

use pz_fs::GameFs;
use pz_scripts::manager::LogLevel;
use pz_scripts::{ScriptManager, ScriptType};

fn main() {
    let dir = std::env::args().nth(1).expect("usage: load_scripts <game dir containing media/>");
    let sm = ScriptManager::new();
    let jar = std::path::Path::new(&dir).join("projectzomboid.jar");
    if jar.exists() {
        sm.load_builtin_registries(&jar).expect("built-in registries");
    }
    let t = std::time::Instant::now();
    sm.load(&GameFs::new(dir), &[]).expect("load");
    eprintln!("loaded in {:?}", t.elapsed());
    if std::env::args().nth(2).as_deref() == Some("--dump") {
        // Same format as the local oracle harness, for diffing.
        for &ty in ScriptType::ALL {
            if ty == ScriptType::EntityComponent {
                continue;
            }
            let list = sm.get_scripts_for_type(ty);
            println!("TYPE {} {}", ty.name(), list.len());
            for s in list {
                let v = s.borrow().base().script_version();
                println!("  {} {}", s.full_type().unwrap_or_default(), v);
            }
        }
        println!("ERRORS {}", sm.has_load_errors(false));
        return;
    }
    for &ty in ScriptType::ALL {
        if ty == ScriptType::EntityComponent {
            continue;
        }
        println!("{:<32}{}", ty.name(), sm.get_scripts_for_type(ty).len());
    }
    println!("modules: {}", sm.module_list().len());
    println!("load errors: {}", sm.has_load_errors(false));
    for l in sm.take_log() {
        if matches!(l.level, LogLevel::Warn | LogLevel::Error) {
            println!("{:?}: {}", l.level, l.message);
        }
    }
}
