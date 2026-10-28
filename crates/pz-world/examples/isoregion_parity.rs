//! Region building ([`pz_world::isoregion`]) over every chunk of a save,
//! against the original's `IsoRegions` run by a local harness on the same
//! chunk files with the same calls (no game data is part of this
//! repository).
//!
//! ```text
//! isoregion_parity <game dir> <sprite tables> <save tables> <save dir> <oracle dir> <out dir>
//! ```
//!
//! The steps, as the harness does them: all chunks loaded (with the window
//! sprite changes of `IsoWindow.load`), `RecalcProperties` on every square;
//! the save's `isoregiondata` copied to `<out dir>/isoregiondata` and
//! loaded (`init`), one worker loop (dump `d0`); `readSurroundingChunks`
//! (7x7, apply) around each loaded chunk with a worker loop after each
//! (`d1`); 60 batches of 5 pseudo-random square changes
//! (`addSquareChangedJob`), a worker loop after each batch (`d2`); a new
//! session loading the files written (`d3`). Writes `rust_d*.txt` to
//! `<out dir>` and prints, per dump, whether it equals the oracle's.

use pz_world::ObjectSlot;
use std::path::Path;

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_world::isoregion::IsoRegions;
use pz_world::save::chunk_file::{apply_loaded_window_sprites, chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::{IsoCell, IsoSpriteManager, SpriteTables};

#[path = "common/region_dump.rs"]
mod region_dump;
use region_dump::dump;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let save_tables = SaveTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let save_dir = Path::new(&a[4]);
    let oracle = Path::new(&a[5]);
    let out_dir = Path::new(&a[6]);
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let settings = SaveSettings::default();
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game.join("projectzomboid.jar"))
        .expect("built-in registries");
    sm.load(&pz_fs::GameFs::new(game.to_str().unwrap()), &[])
        .expect("scripts");
    let dict = WorldDictionary::load(&std::fs::read(save_dir.join("WorldDictionary.bin")).unwrap())
        .unwrap();
    let names = pz_inventory::factory::parse_item_names(
        &std::fs::read_to_string(game.join("media/lua/shared/Translate/EN/ItemName.json")).unwrap(),
    );
    let mut registry = ScriptItems::new(&sm, dict, &names);
    registry.blood_body_parts = save_tables.blood_body_parts;
    registry.world_sprites = pz_world::save::items::moveable_sprites(&sprites);
    let jar = pz_classfile::Jar::open(game.join("projectzomboid.jar")).unwrap();
    let ectx = pz_entity::BasicContext {
        attributes: pz_entity::AttributeTypes::from_jar(&jar).unwrap(),
        fluids: Some(pz_entity::FluidRegistry::from_scripts(&jar, &sm).unwrap()),
        ..Default::default()
    };
    let ectx: &'static pz_entity::BasicContext = Box::leak(Box::new(ectx));
    registry.components = Box::new(pz_inventory::component::EntityCodec { ctx: ectx });
    let ctx = SaveCtx {
        sprites: &sprites,
        tables: &save_tables,
        settings: &settings,
        items: &registry,
        hooks: &NoSaveHooks,
        meta: None,
    };
    let mut files = Vec::new();
    for d in std::fs::read_dir(save_dir.join("map")).unwrap() {
        let d = d.unwrap();
        let Ok(wx) = d.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        for f in std::fs::read_dir(d.path()).unwrap() {
            let n = f.unwrap().file_name().to_string_lossy().to_string();
            if let Some(wy) = n.strip_suffix(".bin").and_then(|s| s.parse::<i32>().ok()) {
                files.push((wx, wy));
            }
        }
    }
    files.sort();
    let mut cell = IsoCell::default();
    let mut order = Vec::new();
    for &(wx, wy) in &files {
        let bytes = std::fs::read(chunk_path(save_dir, wx, wy)).unwrap();
        match load_chunk(&bytes, wx, wy, &ctx) {
            Ok(l) => {
                cell.chunks.insert((wx, wy), l.chunk);
                order.push((wx, wy));
            }
            Err(e) => eprintln!("load {wx},{wy}: {e}"),
        }
    }
    drop(registry);
    // `IsoWindow.load` changes the shared window sprites as each chunk loads.
    for &(wx, wy) in &order {
        apply_loaded_window_sprites(&cell.chunks[&(wx, wy)], &mut sprites);
    }
    for &(wx, wy) in &order {
        let ch = cell.chunks.get_mut(&(wx, wy)).unwrap();
        for z in ch.min_level..=ch.max_level {
            for y in 0..8 {
                for x in 0..8 {
                    if let Some(sq) = ch.get_grid_square_mut(x, y, z) {
                        sq.borrow_mut().recalc_properties(&sprites);
                    }
                }
            }
        }
    }
    // `SHOW_SQUARE=x,y,z`: print that square's objects and flags and stop.
    if let Ok(q) = std::env::var("SHOW_SQUARE") {
        let v: Vec<i32> = q.split(',').map(|x| x.parse().unwrap()).collect();
        if let Some(sq) = cell.get_grid_square(v[0], v[1], v[2]) {
            for o in sq.borrow().objects.iter().map(ObjectSlot::borrow) {
                let sp = o.sprite.tile_projection().map(|s| &sprites.sprites[s]);
                let f = &sprites.tables.flag;
                eprintln!(
                    "obj {:?} sprite={:?} kind={:?} cutW={:?} WindowW={:?} cut_n={:?}",
                    o.sprite.tile_projection(),
                    sp.and_then(|s| s.name.clone()),
                    std::mem::discriminant(&o.kind),
                    sp.map(|s| s.properties.has(f.cutW)),
                    sp.map(|s| s.properties.has(f.WindowW)),
                    sp.map(|s| s.cut_n)
                );
            }
            eprintln!(
                "sq cutW={} WindowW={} flags={}",
                sq.borrow().has(sprites.tables.flag.cutW),
                sq.borrow().has(sprites.tables.flag.WindowW),
                pz_world::isoregion::calculate_square_flags(Some(&sq.borrow()), &sprites)
            );
        }
        return;
    }
    let region_dir = out_dir.join("isoregiondata");
    let _ = std::fs::remove_dir_all(&region_dir);
    std::fs::create_dir_all(&region_dir).unwrap();
    let seed = save_dir.join("isoregiondata");
    if seed.is_dir() && std::env::var_os("NO_SEED").is_none() {
        for e in std::fs::read_dir(&seed).unwrap() {
            let e = e.unwrap();
            std::fs::copy(e.path(), region_dir.join(e.file_name())).unwrap();
        }
    }
    let mut dumps = Vec::new();
    let mut r = IsoRegions::init(Some(out_dir), 13);
    r.worker_tick();
    dumps.push(dump(&r, &order));
    for &(wx, wy) in &order {
        r.read_surrounding_chunks(&cell, &sprites, wx, wy, 7, true, false);
        r.worker_tick();
    }
    dumps.push(dump(&r, &order));
    let mut cand = Vec::new();
    for &(wx, wy) in &order {
        let max_l = cell.chunks[&(wx, wy)].max_level;
        for z in 0..=max_l + 1 {
            for y in 0..8 {
                for x in 0..8 {
                    cand.push((wx * 8 + x, wy * 8 + y, z));
                }
            }
        }
    }
    let masks: [i8; 6] = [1, 2, 16, 32, 4, 3];
    let mut st: u64 = 0x2545F4914F6CDD1D;
    let nb: usize = std::env::var("EDIT_BATCHES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    for _ in 0..nb {
        for _ in 0..5 {
            st = st
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let p = cand[((st >> 33) % cand.len() as u64) as usize];
            st = st
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let mask = masks[((st >> 33) % masks.len() as u64) as usize];
            let cur = r.get_square_flags(p.0, p.1, p.2);
            let nf = (if cur == -1 { 0 } else { cur }) ^ mask;
            r.add_square_changed_job(&cell, &sprites, p.0, p.1, p.2, false, nf);
        }
        r.worker_tick();
    }
    dumps.push(dump(&r, &order));
    let mut r = IsoRegions::init(Some(out_dir), 13);
    r.worker_tick();
    dumps.push(dump(&r, &order));
    for (i, d) in dumps.iter().enumerate() {
        std::fs::write(out_dir.join(format!("rust_d{i}.txt")), d).unwrap();
        let java = std::fs::read_to_string(oracle.join(format!("d{i}.txt"))).unwrap_or_default();
        let lines = d.lines().count();
        let same = d.lines().zip(java.lines()).filter(|(a, b)| a == b).count();
        println!(
            "d{i}: {} ({same}/{lines} lines equal, oracle {} lines)",
            if *d == java { "MATCH" } else { "DIFF" },
            java.lines().count()
        );
    }
    println!("warnings={}", r.warnings().len());
}
