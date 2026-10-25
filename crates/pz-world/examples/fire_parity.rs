//! `IsoFireManager` and `IsoFire` against the original on the chunks of a
//! save: fires started at the squares the original's run lists, stepped
//! with the same frame values and a `java.util.Random` with the same seed,
//! then the fires per step and the burnt squares compared line by line
//! (made by a local harness on a machine with the game installed; no game
//! data is part of this repository).
//!
//! ```text
//! fire_parity <game dir> <sprite tables> <save tables> <save dir> <oracle dir>
//! ```
//!
//! `<oracle dir>` holds the original's `fire.txt` (a header with the seed,
//! the start squares, per-step frame values and fires, then the burnt and
//! touched squares' objects) and `items.txt`. Writes `rust_fire.txt` there
//! and prints the first difference.

use pz_world::ObjectSlot;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_world::fire::manager::FireFlags;
use pz_world::fire::{FireFrame, FireHooks, FireMutation, FireSettings, FireWorld, IsoFireManager};
use pz_world::isoregion::IsoRegions;
use pz_world::save::chunk_file::{chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::square_mutation::{DoorTables, NoMutationHooks};
use pz_world::{GameRand, IsoCell, IsoMetaGrid, IsoSpriteManager, SpriteTables};

/// `java.util.Random`, counting generator steps.
struct JavaRandom {
    seed: i64,
    steps: u64,
}

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom {
            seed: (seed ^ 0x5DEECE66D) & ((1 << 48) - 1),
            steps: 0,
        }
    }

    fn bits(&mut self, bits: u32) -> i32 {
        self.steps += 1;
        self.seed = (self.seed.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.seed >> (48 - bits)) as i32
    }
}

impl GameRand for JavaRandom {
    fn next(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        if bound.count_ones() == 1 {
            return ((bound as i64 * self.bits(31) as i64) >> 31) as i32;
        }
        loop {
            let b = self.bits(31);
            let v = b % bound;
            if b.wrapping_sub(v).wrapping_add(bound - 1) >= 0 {
                return v;
            }
        }
    }

    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 / (1 << 24) as f32
    }
}

struct Names<'a>(&'a WorldDictionary);

impl pz_world::fire::sound::FireSoundHooks for Names<'_> {}
impl FireHooks for Names<'_> {
    fn dict_object_name(&self, id: i8) -> Option<String> {
        self.0.object_name(id).map(str::to_string)
    }
}

fn hex(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

/// `hearth.txt`: two fireplaces, a charcoal barbecue and an unlit fireplace
/// (fuel 45, 20, 30, 100; the second put out at step 3, the barbecue at 5;
/// the last lit at step 2 and put out at 30) burning fuel as the world age
/// in each step line passes. Returns (same lines, lines).
fn hearth_parity(java: &str) -> (usize, usize, String) {
    use pz_world::objects::{IsoBarbecue, IsoFireplace};
    let mut fp: Vec<IsoFireplace> = [45, 20, 100]
        .iter()
        .map(|&f| IsoFireplace {
            fuel_amount_minutes: f,
            ..Default::default()
        })
        .collect();
    fp[0].lit = true;
    fp[1].lit = true;
    let mut bbq = IsoBarbecue {
        fuel_amount_minutes: 30,
        lit: true,
        ..Default::default()
    };
    let mut out = String::new();
    for l in java.lines().filter(|l| l.starts_with("step ")) {
        let s: usize = l.split(' ').nth(1).unwrap().parse().unwrap();
        let h = f64::from_bits(u64::from_str_radix(l.split("hours=").nth(1).unwrap(), 16).unwrap());
        match s {
            2 => {
                fp[2].turn_on();
            }
            3 => fp[1].extinguish(),
            5 => bbq.extinguish(false),
            30 => fp[2].extinguish(),
            _ => {}
        }
        writeln!(out, "{l}").unwrap();
        let line = |i: usize, fuel: i32, lit: bool, sm: bool, since: i32, last: f32, acc: f32| {
            format!(
                "hearth {i} fuel={fuel} lit={} smoulder={} since={since} last={:x} acc={:x}\n",
                lit as u8,
                sm as u8,
                last.to_bits(),
                acc.to_bits()
            )
        };
        for i in 0..4 {
            if i == 2 {
                bbq.update_fuel(h);
                let b = &bbq;
                out += &line(
                    2,
                    b.fuel_amount(),
                    b.lit,
                    b.smouldering,
                    b.minutes_since_extinguished,
                    b.last_update_time,
                    b.minute_accumulator,
                );
            } else {
                let f = &mut fp[if i == 3 { 2 } else { i }];
                f.update_fuel(h);
                out += &line(
                    i,
                    f.fuel_amount(),
                    f.lit,
                    f.smouldering,
                    f.minutes_since_extinguished,
                    f.last_update_time,
                    f.minute_accumulator,
                );
            }
        }
    }
    let same = java
        .lines()
        .zip(out.lines())
        .filter(|(a, b)| a == b)
        .count();
    (same, java.lines().count().max(out.lines().count()), out)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let save_tables = SaveTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let save_dir = Path::new(&a[4]);
    let oracle = Path::new(&a[5]);
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let mut tile_scale = 2;
    for line in std::fs::read_to_string(oracle.join("items.txt"))
        .unwrap()
        .lines()
    {
        if let Some(t) = line.strip_prefix("#tileScale\t") {
            tile_scale = t.parse().unwrap();
        }
    }
    if let Ok(h) = std::fs::read_to_string(oracle.join("hearth.txt")) {
        let (same, n, out) = hearth_parity(&h);
        std::fs::write(oracle.join("rust_hearth.txt"), out).unwrap();
        println!("hearth lines {same}/{n} same");
        if !oracle.join("fire.txt").exists() {
            return;
        }
    }
    let java = std::fs::read_to_string(oracle.join("fire.txt")).unwrap();
    let head: HashMap<&str, &str> = java
        .lines()
        .next()
        .unwrap()
        .split(' ')
        .filter_map(|kv| kv.split_once('='))
        .collect();
    let world_age: f64 = head["worldAgeHours"].parse().unwrap();
    let settings = SaveSettings {
        tile_scale,
        world_age_hours: world_age,
        ..Default::default()
    };
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
    registry.world_age_hours = world_age;
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
    let mut load_errors = 0;
    for &(wx, wy) in &files {
        let bytes = std::fs::read(chunk_path(save_dir, wx, wy)).unwrap();
        match load_chunk(&bytes, wx, wy, &ctx) {
            Ok(l) => {
                cell.chunks.insert((wx, wy), l.chunk);
                order.push((wx, wy));
            }
            Err(e) => {
                load_errors += 1;
                eprintln!("load {wx},{wy}: {e}");
            }
        }
    }
    let flags = FireFlags::resolve(&sprites.tables).unwrap();
    // The harness's `new IsoMetaGrid()`: no cells, so every square is
    // valid (`isValidSquare` overflows) and RecalcAllWithNeighbours reaches
    // all neighbours.
    let mut meta = IsoMetaGrid::empty();
    (meta.min_x, meta.min_y, meta.max_x, meta.max_y) = (10000000, 10000000, -10000000, -10000000);
    let seed: i64 = java
        .lines()
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let settings_fire = FireSettings {
        fire_spread: head["spread"] == "true",
        ..Default::default()
    };
    let dict2 =
        WorldDictionary::load(&std::fs::read(save_dir.join("WorldDictionary.bin")).unwrap())
            .unwrap();
    let mut hooks = Names(&dict2);
    let mut rand = JavaRandom::new(seed);
    let mut mgr = IsoFireManager::new();
    let mut out = String::new();
    writeln!(out, "{}", java.lines().next().unwrap()).unwrap();
    let doors = DoorTables::from_jar(&jar).unwrap();
    let mut regions = IsoRegions::new(13);
    let (mut mw, mut mp, mut mr) = (NoMutationHooks, NoMutationHooks, NoMutationHooks);
    let mut w = FireWorld {
        cell: &mut cell,
        meta: &meta,
        sprites: &mut sprites,
        mutation: FireMutation {
            regions: &mut regions,
            doors: &doors,
            world: &mut mw,
            pathfind: &mut mp,
            render: &mut mr,
            num_players: 1,
        },
        flags,
        settings: settings_fire,
        hooks: &mut hooks,
    };
    for l in java.lines().filter_map(|l| l.strip_prefix("start ")) {
        let p: Vec<i32> = l.split(',').map(|v| v.parse().unwrap()).collect();
        writeln!(out, "start {l}").unwrap();
        mgr.start_fire4(&mut w, &mut rand, (p[0], p[1], p[2]), false, 100);
    }
    let mut touched = BTreeSet::new();
    // FireRestOracle's runs also list the sound slots after every update
    // and the special-object count of every square.
    let sounds = java.lines().any(|l| l.starts_with("sounds"));
    let frames: Vec<(f32, f32)> = java
        .lines()
        .filter(|l| l.starts_with("step "))
        .map(|l| {
            let kv: HashMap<&str, &str> =
                l.split(' ').filter_map(|kv| kv.split_once('=')).collect();
            (hex(kv["t30"]), hex(kv["ms"]))
        })
        .collect();
    for (s, &(t30, ms)) in frames.iter().enumerate() {
        writeln!(
            out,
            "step {s} t30={:x} ms={:x} rand={} fires={}",
            t30.to_bits(),
            ms.to_bits(),
            rand.steps,
            mgr.fire_stack.len()
        )
        .unwrap();
        for &r in &mgr.fire_stack {
            touched.insert((r.x, r.y, r.z));
            let sq = w.cell.get_grid_square(r.x, r.y, r.z).unwrap();
            let idx = sq.object_index(r.id).map_or(-1, |i| i as i32);
            let f = mgr.fire(w.cell, r).unwrap();
            writeln!(
                out,
                "fire {},{},{} age={} life={} energy={} stage={} timer={} spread={}/{} idx={idx}",
                r.x,
                r.y,
                r.z,
                f.age,
                f.life,
                f.energy,
                f.life_stage,
                f.life_stage_timer,
                f.spread_timer,
                f.spread_delay
            )
            .unwrap();
        }
        if s + 1 == frames.len() {
            break;
        }
        let frame = FireFrame {
            multiplied_seconds: ms,
            thirty_fps: t30,
            now_millis: 0,
        };
        // The harness stops IsoFireManager.Update's loop at an exception,
        // as the original does, and skips fireSounds.update.
        if let Err(e) = mgr.update(&mut w, &mut rand, frame) {
            writeln!(out, "error {e}").unwrap();
        }
        if sounds {
            let mut l = String::from("sounds");
            for (i, slot) in mgr.sounds.slots.iter().enumerate() {
                if let Some(r) = slot.fire {
                    write!(l, " {i}:{},{},{}:{}", r.x, r.y, r.z, slot.playing as u8).unwrap();
                }
            }
            writeln!(out, "{l}").unwrap();
        }
    }
    let mut squares = BTreeSet::new();
    for &(wx, wy) in &order {
        let ch = &w.cell.chunks[&(wx, wy)];
        for x in 0..8 {
            for y in 0..8 {
                for z in ch.min_level..=ch.max_level {
                    let Some(sq) = ch.get_grid_square(x, y, z) else {
                        continue;
                    };
                    if !sq.burnt_out && !touched.contains(&(sq.x, sq.y, sq.z)) {
                        continue;
                    }
                    let objs: Vec<String> = sq
                        .objects
                        .iter()
                        .map(ObjectSlot::borrow)
                        .map(|o| {
                            let n = o
                                .sprite
                                .and_then(|s| w.sprites.sprites[s].name.clone())
                                .unwrap_or_else(|| "null".into());
                            let class = o.kind.class_name();
                            let class = match class {
                                "IsoWaveSignal" => "IsoRadio",
                                c => c,
                            };
                            format!("{class}:{n}")
                        })
                        .collect();
                    let sp = if sounds {
                        format!(" sp={}", sq.special_objects.len())
                    } else {
                        String::new()
                    };
                    squares.insert(format!(
                        "square {},{},{} burnt={}{sp} objs={}",
                        sq.x,
                        sq.y,
                        sq.z,
                        sq.burnt_out as u8,
                        objs.join("|")
                    ));
                }
            }
        }
    }
    for s in squares {
        writeln!(out, "{s}").unwrap();
    }
    std::fs::write(oracle.join("rust_fire.txt"), &out).unwrap();
    let jl: Vec<&str> = java.lines().filter(|l| !l.starts_with("lamps ")).collect();
    let rl: Vec<&str> = out.lines().collect();
    let (mut same, mut first) = (0, None);
    for i in 0..jl.len().max(rl.len()) {
        if jl.get(i) == rl.get(i) {
            same += 1;
        } else if first.is_none() {
            first = Some(i);
        }
    }
    let steps = frames.len();
    let fires = jl.iter().filter(|l| l.starts_with("fire ")).count();
    let burnt = jl.iter().filter(|l| l.contains(" burnt=1 ")).count();
    println!(
        "lines {same}/{} same; {steps} steps, {fires} fire lines, {burnt} burnt squares; load errors {load_errors}",
        jl.len().max(rl.len())
    );
    if let Some(i) = first {
        println!(
            "first difference at line {}:\n  java {:?}\n  rust {:?}",
            i + 1,
            jl.get(i),
            rl.get(i)
        );
    }
}
