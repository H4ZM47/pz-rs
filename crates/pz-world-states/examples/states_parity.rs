//! `IsoGridSquare.testCollideSpecialObjects` towards the eight neighbours,
//! `getThumpableFor` / `getThumpCondition` for a zombie on each side of
//! every door, window, window frame, thumpable and barricade, and
//! `IsoBarricade.Damage` sequences of [`pz_world_states`] over every chunk
//! of a save, against the same calls evaluated by the original game on the
//! same files (made by a local harness on a machine with the game
//! installed; no game data is part of this repository).
//!
//! ```text
//! states_parity <game dir> <sprite tables> <save tables> <save dir> <oracle dir>
//! ```
//!
//! `<oracle dir>` holds the original's `states.txt`, `items.txt` and
//! `summary.txt`. Writes `rust_states.txt` there and prints line counts by
//! kind.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_pathfind::mover::ObjectRef;
use pz_world::java::GameRand;
use pz_world::objects::ObjectKind;
use pz_world::save::chunk_file::{chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::{IsoCell, IsoMetaGrid, IsoSpriteManager, SpriteTables};
use pz_world_states::{StatesEngine, StatesRuntime, StatesWorldAdapter, ThumpSettings, Thumper};

/// `java.util.Random`.
struct JRandom(i64);

impl JRandom {
    fn new(seed: i64) -> Self {
        JRandom((seed ^ 0x5DEECE66D) & ((1 << 48) - 1))
    }
    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - bits)) as i32
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        let mut r = self.next(31);
        let m = bound - 1;
        if bound & m == 0 {
            return ((bound as i64 * r as i64) >> 31) as i32;
        }
        let mut u = r;
        loop {
            r = u % bound;
            if u - r + m >= 0 {
                return r;
            }
            u = self.next(31);
        }
    }
    fn next_float(&mut self) -> f32 {
        self.next(24) as f32 / (1 << 24) as f32
    }
}

/// `String.hashCode()`.
fn java_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

struct NoRand;

impl GameRand for NoRand {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

struct Engine(NoRand);

impl StatesEngine for Engine {
    fn rand(&mut self) -> &mut dyn GameRand {
        &mut self.0
    }
}

fn hx(f: f32) -> String {
    format!("{:x}", f.to_bits())
}

fn at(r: ObjectRef, cell: &IsoCell) -> String {
    let sq_owner = cell.get_grid_square(r.x, r.y, r.z).unwrap();
    let sq = sq_owner.borrow();
    format!("{},{},{},{}", r.x, r.y, r.z, sq.object_index(r.id).unwrap())
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
    let world_age: f64 = std::fs::read_to_string(oracle.join("summary.txt"))
        .unwrap()
        .split_whitespace()
        .find_map(|w| w.strip_prefix("worldAgeHours="))
        .unwrap()
        .parse()
        .unwrap();
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
    let meta = IsoMetaGrid::empty();
    let mut rt = StatesRuntime::default();
    let mut engine = Engine(NoRand);
    let mut out = String::new();
    let mut bars = Vec::new();
    {
        let a = StatesWorldAdapter::new(
            &mut cell,
            &meta,
            &mut sprites,
            ThumpSettings::default(),
            &mut rt,
            &mut engine,
        );
        let cell = &*a.cell;
        for &(wx, wy) in &order {
            let ch = &cell.chunks[&(wx, wy)];
            for x in 0..8 {
                for y in 0..8 {
                    for z in ch.min_level..=ch.max_level {
                        let Some(sq_owner) = ch.get_grid_square(x, y, z) else {
                            continue;
                        };
                        let sq = sq_owner.borrow();
                        let near = (-1..=1).any(|dx| {
                            (-1..=1).any(|dy| {
                                cell.get_grid_square(sq.x + dx, sq.y + dy, sq.z)
                                    .is_some_and(|s| {
                                        let s = s.borrow();
                                        !s.special_objects.is_empty()
                                            || s.objects.iter().any(|o| {
                                                let o = o.borrow();
                                                o.moved_thumpable
                                                    || matches!(
                                                        o.kind,
                                                        ObjectKind::WindowFrame { .. }
                                                    )
                                            })
                                    })
                            })
                        });
                        if !near {
                            continue;
                        }
                        write!(out, "sc {},{},{} ", sq.x, sq.y, sq.z).unwrap();
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                if dx == 0 && dy == 0 {
                                    continue;
                                }
                                let to = (sq.x + dx, sq.y + dy, sq.z);
                                if cell.get_grid_square(to.0, to.1, to.2).is_none() {
                                    out.push_str("-/");
                                    continue;
                                }
                                match a.special_object_ref((sq.x, sq.y, sq.z), to) {
                                    None => out.push_str("n/"),
                                    Some(r) => write!(out, "{}/", at(r, cell)).unwrap(),
                                }
                            }
                        }
                        out.push('\n');
                    }
                }
            }
        }
        for &(wx, wy) in &order {
            let ch = &cell.chunks[&(wx, wy)];
            for x in 0..8 {
                for y in 0..8 {
                    for z in ch.min_level..=ch.max_level {
                        let Some(sq_owner) = ch.get_grid_square(x, y, z) else {
                            continue;
                        };
                        let sq = sq_owner.borrow();
                        for (oi, o) in sq.objects.iter().enumerate() {
                            let o = o.borrow();
                            let r = ObjectRef {
                                x: sq.x,
                                y: sq.y,
                                z: sq.z,
                                id: o.id,
                            };
                            match o.kind {
                                ObjectKind::Barricade(_) => {
                                    bars.push((r, format!("{},{},{},{oi}", sq.x, sq.y, sq.z)))
                                }
                                ObjectKind::Door(_)
                                | ObjectKind::Window(_)
                                | ObjectKind::Thumpable(_)
                                | ObjectKind::WindowFrame { .. } => {}
                                _ => continue,
                            }
                            write!(out, "tf {},{},{},{oi}", sq.x, sq.y, sq.z).unwrap();
                            let (fx, fy) = (sq.x as f32, sq.y as f32);
                            for (px, py) in [
                                (fx + 0.5, fy + 0.5),
                                (fx + 0.5, fy - 0.5),
                                (fx - 0.5, fy + 0.5),
                            ] {
                                for crawling in [false, true] {
                                    let t = Thumper {
                                        x: px,
                                        y: py,
                                        z: sq.z as f32,
                                        zombie: true,
                                        crawling,
                                        ..Default::default()
                                    };
                                    match a.thumpable_for(r, Some(&t)) {
                                        None => out.push_str(" n"),
                                        Some(tf) => write!(
                                            out,
                                            " {}={}",
                                            at(tf, cell),
                                            hx(a.thump_condition(tf))
                                        )
                                        .unwrap(),
                                    }
                                }
                            }
                            out.push('\n');
                        }
                    }
                }
            }
        }
    }
    let mut a = StatesWorldAdapter::new(
        &mut cell,
        &meta,
        &mut sprites,
        ThumpSettings::default(),
        &mut rt,
        &mut engine,
    );
    for (r, name) in bars {
        let mut rand = JRandom::new(java_hash(&name) as i64);
        write!(out, "bd {name}").unwrap();
        for _ in 0..40 {
            let destroyed = a.find(r).map_or(true, |(_, o)| match &o.borrow().kind {
                ObjectKind::Barricade(b) => b.is_destroyed(),
                _ => true,
            });
            if destroyed {
                break;
            }
            let amount = rand.next_int(1200) as f32 + rand.next_float();
            a.barricade_damage(r, amount);
            let Some((_, owner)) = a.find(r) else { break };
            let o = owner.borrow();
            let ObjectKind::Barricade(b) = &o.kind else {
                break;
            };
            let spr = o
                .sprite
                .and_then(|s| a.sprites.sprites[s].name.clone())
                .unwrap_or_else(|| "null".into());
            let p = b.plank_health;
            write!(
                out,
                " {}:{}/{}/{}.{}.{}.{}/{spr}",
                hx(amount),
                b.metal_health,
                b.metal_bar_health,
                p[0],
                p[1],
                p[2],
                p[3]
            )
            .unwrap();
        }
        out.push('\n');
    }
    std::fs::write(oracle.join("rust_states.txt"), &out).unwrap();
    let java = std::fs::read_to_string(oracle.join("states.txt")).unwrap();
    let mut stats: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let (jl, rl): (Vec<&str>, Vec<&str>) = (java.lines().collect(), out.lines().collect());
    let mut shown = 0;
    for i in 0..jl.len().max(rl.len()) {
        let (j, r) = (
            jl.get(i).copied().unwrap_or(""),
            rl.get(i).copied().unwrap_or(""),
        );
        let kind = j.split(' ').next().unwrap_or("?").to_string();
        let e = stats.entry(kind).or_default();
        if j == r {
            e.0 += 1;
        } else {
            e.1 += 1;
            if shown < 10 {
                shown += 1;
                eprintln!("java: {j}\nrust: {r}");
            }
        }
    }
    println!("chunks {} load errors {load_errors}", order.len());
    for (k, (eq, ne)) in stats {
        println!("{k}: {eq} equal, {ne} differ");
    }
}
