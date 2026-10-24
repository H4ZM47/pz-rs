//! Compares this port's brand-new chunk loading with world generation
//! against the game's.
//!
//! ```text
//! worldgen_replay <oracle dir> <game dir> [region...]
//! ```
//!
//! `<game dir>` holds `projectzomboid.jar` and `media/`. `<oracle dir>` holds
//! `regions/<name>/chunks.bin` and `squares.tsv`, written by a local harness
//! that runs the game's own `IsoChunk.LoadChunk` headless on an empty save
//! (lotpacks, world generation with the seed `pzrs-pathfind-oracle`,
//! basements), `Rand` reseeded per chunk with `wx << 32 ^ wy`, row-major
//! over each region plus a one-chunk margin, followed by
//! `loadInMainThread`. Nothing here is game data; results are recorded as
//! notes only.
//!
//! The replay loads the same chunks in the same order through
//! `pz_world::chunk::load_brand_new_cached` with [`WorldGenHook`], runs
//! `recalc_chunk_squares` and `load_in_main_thread`, and compares each
//! square's object list (class and sprite name, in order) with the
//! oracle's `objects` column.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_classfile::Jar;
use pz_fs::GameFs;
use pz_world::chunk::{load_brand_new_cached, ChunkLoadEnv, IsoCell, LotCache};
use pz_world::fix2x::Fix2xTable;
use pz_world::{
    CreateParams, IsoMetaGrid, IsoSpriteManager, NoHooks, ObjectLoadSettings, ObjectTables,
    SpriteTables, WorldGenBounds,
};
use pz_worldgen::java::JavaRandom;
use pz_worldgen::{WorldGenHook, WorldGenSetup};

const SEED: &str = "pzrs-pathfind-oracle";

fn region_chunks(path: &Path) -> Vec<(i32, i32)> {
    let b = std::fs::read(path).expect("chunks.bin");
    let mut out = Vec::new();
    let mut i = 0;
    let le = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    while i + 16 <= b.len() {
        out.push((le(i + 4), le(i + 8)));
        i += 16 + le(i + 12) as usize;
    }
    out
}

/// `(x, y, z) -> (null, objects)` from `squares.tsv`.
fn oracle_objects(path: &Path) -> HashMap<(i32, i32, i32), (bool, String)> {
    let text = std::fs::read_to_string(path).expect("squares.tsv");
    let mut out = HashMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let c: Vec<&str> = line.split('\t').collect();
        let key = (
            c[0].parse().unwrap(),
            c[1].parse().unwrap(),
            c[2].parse().unwrap(),
        );
        out.insert(
            key,
            (c[8] == "1", c.get(20).copied().unwrap_or("").to_string()),
        );
    }
    out
}

/// The sprite tilesets of entries only one side has, `+` for the game's.
fn diff_sets(game: &str, port: &str) -> Vec<String> {
    let only = |a: &str, b: &str| -> Vec<String> {
        let mut have: Vec<&str> = b.split(';').filter(|s| !s.is_empty()).collect();
        let mut out = Vec::new();
        for o in a.split(';').filter(|s| !s.is_empty()) {
            if let Some(i) = have.iter().position(|h| *h == o) {
                have.remove(i);
            } else {
                let name = o.split_once(':').map_or(o, |x| x.1);
                out.push(name.rsplit_once('_').map_or(name, |x| x.0).to_string());
            }
        }
        out
    };
    let mut v: Vec<String> = only(game, port)
        .into_iter()
        .map(|s| format!("+{s}"))
        .collect();
    v.extend(only(port, game).into_iter().map(|s| format!("-{s}")));
    v.sort();
    v.dedup();
    v
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let oracle = PathBuf::from(&a[1]);
    let game = PathBuf::from(&a[2]);
    let only: Vec<&str> = a[3..].iter().map(String::as_str).collect();
    let jar_path = game.join("projectzomboid.jar");
    let jar = Jar::open(&jar_path).expect("jar");
    let tables = SpriteTables::from_game_jar(&jar_path).expect("sprite tables");
    let objects = ObjectTables::from_game_jar(&jar_path).expect("object tables");
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| true).unwrap();
    let fs = Rc::new(GameFs::new(&game));
    let meta = IsoMetaGrid::create(
        &fs,
        CreateParams {
            game_map: "Muldraugh, KY",
            world_gen: WorldGenBounds {
                min_x: -250,
                min_y: -250,
                max_x: 250,
                max_y: 250,
            },
            rand: &mut JavaRandom::new(42),
            voronois: Vec::new(),
        },
    )
    .expect("meta grid");
    let sandbox = pz_worldgen::jar_consts::sandbox_double_defaults(&jar).expect("sandbox defaults");
    let lookup = |k: &str| sandbox.get(k).copied();
    let t0 = std::time::Instant::now();
    let gen = pz_worldgen::load(&WorldGenSetup {
        fs: fs.clone(),
        jar: &jar,
        seed: SEED,
        maps: &["Muldraugh, KY"],
        cell_bounds: (meta.min_x, meta.min_y, meta.max_x, meta.max_y),
        sandbox: &lookup,
    })
    .expect("world generation definitions");
    eprintln!(
        "worldgen loaded in {:?}: {} biomes, {} map biomes, {} zones, {} static modules, {} vein configs, {} road generators",
        t0.elapsed(),
        gen.biomes.len(),
        gen.biomes_map.len(),
        gen.zones.len(),
        gen.static_modules.len(),
        gen.veins.configs.len(),
        gen.road_generators.len()
    );
    let mut hook = WorldGenHook::new(gen);
    // `REPLAY_NO_WORLDGEN=1`: the lotpacks alone, to see what generation changes.
    hook.do_worldgen = std::env::var_os("REPLAY_NO_WORLDGEN").is_none();
    let fix2x = Fix2xTable::default();
    // Every region of the oracle with an objects column.
    let mut regions: Vec<String> = std::fs::read_dir(oracle.join("regions"))
        .expect("regions")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|r| r != "synthetic_slopes")
        .collect();
    regions.sort();
    let mut total = (0usize, 0usize);
    let verbose = std::env::var_os("REPLAY_EXAMPLES").is_some();
    for region in &regions {
        let region = region.as_str();
        if !only.is_empty() && !only.contains(&region) {
            continue;
        }
        let dir = oracle.join("regions").join(region);
        let chunks = region_chunks(&dir.join("chunks.bin"));
        let game_sq = oracle_objects(&dir.join("squares.tsv"));
        let (x0, x1) = (
            chunks.iter().map(|c| c.0).min().unwrap(),
            chunks.iter().map(|c| c.0).max().unwrap(),
        );
        let (y0, y1) = (
            chunks.iter().map(|c| c.1).min().unwrap(),
            chunks.iter().map(|c| c.1).max().unwrap(),
        );
        let mut cell = IsoCell::default();
        let mut cache = LotCache::default();
        let mut order = Vec::new();
        for wy in y0 - 1..=y1 + 1 {
            for wx in x0 - 1..=x1 + 1 {
                let mut rand = JavaRandom::new(((wx as i64) << 32) ^ (wy as i64 & 0xffff_ffff));
                let mut hooks = NoHooks;
                let mut env = ChunkLoadEnv {
                    sprites: &mut sprites,
                    fix2x: &fix2x,
                    rand: &mut rand,
                    tables: &objects,
                    settings: ObjectLoadSettings::default(),
                    hooks: &mut hooks,
                };
                let (ch, _) =
                    load_brand_new_cached(&meta, wx, wy, &mut env, Some(&mut hook), &mut cache);
                cell.chunks.insert((wx, wy), ch);
                order.push((wx, wy));
            }
        }
        for &(wx, wy) in &order {
            cell.recalc_chunk_squares(wx, wy, &meta, &sprites);
        }
        for &(wx, wy) in &order {
            cell.load_in_main_thread(wx, wy, &meta, &sprites);
        }
        let (mut n, mut same, mut null_vs_empty) = (0, 0, 0);
        let mut causes: BTreeMap<String, usize> = BTreeMap::new();
        let mut examples: Vec<String> = Vec::new();
        let mut keys: Vec<_> = game_sq.keys().copied().collect();
        keys.sort_by_key(|k| (k.2, k.1, k.0));
        for k in keys {
            let (g_null, g_obj) = &game_sq[&k];
            n += 1;
            let ours = cell.get_grid_square(k.0, k.1, k.2);
            let p_obj = ours.map(|sq| {
                sq.objects
                    .iter()
                    .map(|o| {
                        format!(
                            "{}:{}",
                            o.kind.class_name(),
                            o.sprite.map_or("null", |s| sprites.sprites[s].name())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(";")
            });
            let p_null = ours.is_none();
            let p_obj = p_obj.unwrap_or_default();
            if *g_obj == p_obj {
                same += 1;
                if *g_null != p_null {
                    null_vs_empty += 1;
                }
                continue;
            }
            let mut sets = diff_sets(g_obj, &p_obj);
            if sets.is_empty() {
                sets.push("order".into());
            }
            let c = format!(
                "{}{}",
                if k.2 < 0 { "(basement level) " } else { "" },
                sets.join(" ")
            );
            *causes.entry(c).or_default() += 1;
            if verbose && examples.len() < 40 {
                examples.push(format!(
                    "{},{},{}\n    game {g_obj}\n    port {p_obj}",
                    k.0, k.1, k.2
                ));
            }
        }
        println!(
            "== {region}: squares {n}, object lists identical {same} ({null_vs_empty} null on one side, empty on the other), differ {}",
            n - same
        );
        for (c, k) in &causes {
            println!("  {k:6}  {c}");
        }
        for e in &examples {
            println!("  e.g. {e}");
        }
        total.0 += n;
        total.1 += same;
    }
    for e in hook.gen.errors.iter().take(20) {
        println!("generation error: {e}");
    }
    println!(
        "== total: squares {}, identical {}, differ {}",
        total.0,
        total.1,
        total.0 - total.1
    );
}
