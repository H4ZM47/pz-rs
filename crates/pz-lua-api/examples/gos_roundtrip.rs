//! Loads each `gos_<name>.bin` given into a server global-object system and
//! saves it again, reporting whether the bytes are identical. The save
//! keeps only keys the Lua side registered, so the example registers every
//! string key found in the file. With `--mutate <out-dir>`, also writes a
//! copy with an added object and changed mod data.
//!
//! `gos_roundtrip [--mutate <out-dir>] <gos_x.bin>...`

use pz_kahlua::{TableRef, Value};
use pz_lua_api::global_objects::{init_lookup, reset, GlobalObjectSystem, GridBounds, Side};
use pz_lua_api::global_objects_save::{load_system_bytes, save_system_bytes};
use std::collections::HashSet;
use std::path::Path;

fn string_keys(t: &TableRef, into: &mut HashSet<String>) {
    for (k, _) in t.entries() {
        if let Value::Str(s) = k {
            into.insert(s.to_string());
        }
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut mutate = None;
    if args.first().map(String::as_str) == Some("--mutate") {
        mutate = Some(args[1].clone());
        args.drain(..2);
    }
    let mut failed = false;
    for f in &args {
        reset(Side::Server);
        init_lookup(GridBounds {
            min_x: 0,
            min_y: 0,
            width: 200,
            height: 200,
        });
        let path = Path::new(f);
        let stem = path.file_stem().unwrap().to_string_lossy();
        let name = stem.strip_prefix("gos_").unwrap_or(&stem).to_string();
        let data = std::fs::read(path).expect("read");
        let system = GlobalObjectSystem::new(Side::Server, &name).unwrap();
        if let Err(e) = load_system_bytes(&system, &data, path) {
            println!("{f}: load error {e}");
            failed = true;
            continue;
        }
        let mut keys = HashSet::new();
        string_keys(&system.mod_data, &mut keys);
        *system.mod_data_keys.borrow_mut() = keys;
        let mut keys = HashSet::new();
        for i in 0..system.object_count() {
            string_keys(
                &system.object_by_index(i as i32).unwrap().mod_data,
                &mut keys,
            );
        }
        *system.object_mod_data_keys.borrow_mut() = keys;
        let out = save_system_bytes(&system).expect("save");
        let same = out == data;
        failed |= !same;
        println!(
            "{f}: {} ({} bytes, {} objects, {} system keys, {} object keys)",
            if same { "identical" } else { "DIFFERENT" },
            data.len(),
            system.object_count(),
            system.mod_data_keys.borrow().len(),
            system.object_mod_data_keys.borrow().len()
        );
        if let Some(dir) = &mutate {
            system.mod_data.set_str("rustKey", 42.5);
            system.mod_data_keys.borrow_mut().insert("rustKey".into());
            let o = system.new_object(1234, 5678, 1).unwrap();
            o.mod_data.set_str("state", Value::str("rust"));
            o.mod_data.set_str("rustOnly", true);
            {
                let mut keys = system.object_mod_data_keys.borrow_mut();
                keys.insert("state".into());
                keys.insert("rustOnly".into());
            }
            let save = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "out".into());
            let out = Path::new(dir).join(format!("{save}.gos_{name}.bin"));
            std::fs::write(out, save_system_bytes(&system).expect("save")).expect("write");
        }
    }
    reset(Side::Server);
    std::process::exit(failed as i32);
}
