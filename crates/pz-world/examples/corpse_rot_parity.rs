//! Corpse rot ([`pz_world::dead_body::update_bodies`]) against the
//! original's `IsoDeadBody.updateBodies`, run by a local harness on the
//! same chunk files: the chunks around the save's corpses join the world
//! (`CorpseCount.chunkLoaded`, each corpse's `addToWorld`), then the world
//! age advances in seeded steps, `Rand` seeded `1000 + step` before each
//! `updateBodies`. Compares, per step, the next `Rand` draw, the registry's
//! iteration order, each corpse (rot stage, skin, skeleton, fake dead,
//! crawling, death and reanimate time, container items and poisoned
//! maggots, worn items) and the corpse squares' corpses and world items
//! (no game data is part of this repository).
//!
//! ```text
//! corpse_rot_parity <game dir> <sprite tables> <save tables> <save dir> <oracle corpses.txt> <out dir>
//! ```
//!
//! The sandbox comes from the harness's environment variables (`HOURS`,
//! `MAGGOT`, `FAKEDEAD`, `REMOVE_PLAYERS`, `SEASON`, `LASTID`).

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::{create_item_by_type, parse_item_names, ScriptItems};
use pz_inventory::item::ItemKind;
use pz_inventory::saved::SavedItem;
use pz_inventory::InventoryItem;
use pz_world::dead_body::{self, CorpseHooks, CorpseSettings};
use pz_world::isoregion::IsoRegions;
use pz_world::save::chunk_file::{apply_loaded_window_sprites, chunk_path, load_chunk};
use pz_world::save::visual::{CorpseVisual, IsoDeadBody};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::square_mutation::{
    DoorTables, MutationEnv, NoMutationHooks, RenderSquareHooks, SquareMutationHooks, SquarePos,
};
use pz_world::{GameRand, IsoCell, IsoMetaGrid, IsoSpriteManager, SpriteTables};

/// `java.util.Random`, shared by the world and the item factory.
#[derive(Clone)]
struct JavaRandom(Rc<RefCell<u64>>);

impl JavaRandom {
    fn seed(&self, seed: i64) {
        *self.0.borrow_mut() = (seed as u64 ^ 0x5DEECE66D) & ((1 << 48) - 1);
    }
    fn bits(&self, bits: u32) -> i32 {
        let mut s = self.0.borrow_mut();
        *s = (s.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (*s >> (48 - bits)) as i64 as i32
    }
    fn int(&self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let mut r = self.bits(31);
        let m = bound - 1;
        if bound & m == 0 {
            return ((bound as i64 * r as i64) >> 31) as i32;
        }
        let mut u = r;
        loop {
            r = u % bound;
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
            u = self.bits(31);
        }
    }
}

impl GameRand for JavaRandom {
    fn next(&mut self, bound: i32) -> i32 {
        self.int(bound)
    }
    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 / (1 << 24) as f32
    }
}

impl pz_inventory::component::FactoryRng for JavaRandom {
    fn next_int(&mut self, bound: i32) -> i32 {
        self.int(bound)
    }
    fn next_float(&mut self, min: f32, max: f32) -> f32 {
        self.next_float_range(min, max)
    }
}

/// `PerformanceSettings.fboRenderChunk` is on in the harness.
struct Fbo;
impl RenderSquareHooks for Fbo {
    fn fbo_render_chunk(&self) -> bool {
        true
    }
}

/// The world age `addToWorld` clamps death times to.
struct Age(f64);
impl SquareMutationHooks for Age {
    fn world_age_hours(&self) -> Option<f64> {
        Some(self.0)
    }
}

/// `InventoryItemFactory.CreateItem` over the save's scripts.
struct Items<'a> {
    registry: &'a ScriptItems,
    sm: &'a pz_scripts::ScriptManager,
    display: &'a dyn Fn(&str) -> Option<String>,
}

impl CorpseHooks for Items<'_> {
    fn create_item(&mut self, full_type: &str, _: &mut dyn GameRand) -> Option<InventoryItem> {
        create_item_by_type(self.registry, self.sm, full_type, 1.0, true, self.display)
    }
}

fn env_i(name: &str, def: i32) -> i32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(def)
}

fn body_line(p: SquarePos, b: &IsoDeadBody) -> String {
    let rot = match &b.visual {
        CorpseVisual::Human(v) => format!("h{} skin {}", v.zombie_rot_stage, v.skin_texture),
        CorpseVisual::Animal { rot_stage, .. } => format!("a{rot_stage}"),
    };
    let cont = match &b.container {
        None => "-1".to_string(),
        Some(c) => {
            let poison = c
                .items
                .iter()
                .filter(|i| matches!(i, SavedItem::Item(i) if matches!(&i.kind, ItemKind::Food(f) if f.poison_power == 5)))
                .count();
            format!("{}/{poison}", c.items.len())
        }
    };
    format!(
        "b {} {},{},{} {rot} skel {} fake {} crawl {} dt {:x} rt {:x} cont {cont} worn {}",
        b.object_id.0,
        p.0,
        p.1,
        p.2,
        b.is_skeleton(),
        b.fake_dead,
        b.crawling,
        b.death_time.to_bits(),
        b.reanimate_time.to_bits(),
        b.worn_items.len()
    )
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let save_tables = SaveTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let save_dir = Path::new(&a[4]);
    let oracle = std::fs::read_to_string(&a[5]).unwrap();
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
    let names = parse_item_names(
        &std::fs::read_to_string(game.join("media/lua/shared/Translate/EN/ItemName.json")).unwrap(),
    );
    let mut registry = ScriptItems::new(&sm, dict, &names);
    registry.blood_body_parts = save_tables.blood_body_parts;
    registry.world_sprites = pz_world::save::items::moveable_sprites(&sprites);
    let jar = pz_classfile::Jar::open(game.join("projectzomboid.jar")).unwrap();
    let doors = DoorTables::from_jar(&jar).unwrap();
    let ectx = pz_entity::BasicContext {
        attributes: pz_entity::AttributeTypes::from_jar(&jar).unwrap(),
        fluids: Some(pz_entity::FluidRegistry::from_scripts(&jar, &sm).unwrap()),
        ..Default::default()
    };
    let ectx: &'static pz_entity::BasicContext = Box::leak(Box::new(ectx));
    registry.components = Box::new(pz_inventory::component::EntityCodec { ctx: ectx });
    let mut all = Vec::new();
    for d in std::fs::read_dir(save_dir.join("map")).unwrap() {
        let d = d.unwrap();
        let Ok(wx) = d.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        for f in std::fs::read_dir(d.path()).unwrap() {
            let n = f.unwrap().file_name().to_string_lossy().to_string();
            if let Some(wy) = n.strip_suffix(".bin").and_then(|s| s.parse::<i32>().ok()) {
                all.push((wx, wy));
            }
        }
    }
    all.sort();
    let wtxt = std::fs::read_to_string(Path::new(&a[5]).with_file_name("window.txt")).unwrap();
    let w: Vec<i32> = wtxt
        .split_whitespace()
        .map(|v| v.parse().unwrap())
        .collect();
    let (min_wx, min_wy, win) = (w[0], w[1], w[2]);
    let mut cell = IsoCell::default();
    let mut order = Vec::new();
    {
        let ctx = SaveCtx {
            sprites: &sprites,
            tables: &save_tables,
            settings: &settings,
            items: &registry,
            hooks: &NoSaveHooks,
            meta: None,
        };
        for &(wx, wy) in &all {
            if wx < min_wx || wx >= min_wx + win || wy < min_wy || wy >= min_wy + win {
                continue;
            }
            let bytes = std::fs::read(chunk_path(save_dir, wx, wy)).unwrap();
            match load_chunk(&bytes, wx, wy, &ctx) {
                Ok(l) => {
                    cell.chunks.insert((wx, wy), l.chunk);
                    order.push((wx, wy));
                }
                Err(e) => eprintln!("load {wx},{wy}: {e}"),
            }
        }
    }
    for &(wx, wy) in &order {
        apply_loaded_window_sprites(&cell.chunks[&(wx, wy)], &mut sprites);
    }
    for &(wx, wy) in &order {
        let ch = cell.chunks.get_mut(&(wx, wy)).unwrap();
        for z in ch.min_level..=ch.max_level {
            for y in 0..8 {
                for x in 0..8 {
                    if let Some(sq) = ch.get_grid_square_mut(x, y, z) {
                        sq.recalc_properties(&sprites);
                    }
                }
            }
        }
    }
    let (cmin_x, cmin_y) = (min_wx.div_euclid(32) - 1, min_wy.div_euclid(32) - 1);
    let (cmax_x, cmax_y) = (
        (min_wx + win - 1).div_euclid(32) + 1,
        (min_wy + win - 1).div_euclid(32) + 1,
    );
    let mut meta = IsoMetaGrid::blank(cmin_x, cmin_y, cmax_x, cmax_y);
    for x in cmin_x..=cmax_x {
        for y in cmin_y..=cmax_y {
            meta.create_cell(x, y);
        }
    }
    let mut regions = IsoRegions::new(win);
    let rand = JavaRandom(Rc::new(RefCell::new(0)));
    *registry.rand.borrow_mut() = Some(Box::new(rand.clone()));
    let display = |full: &str| names.get(full).cloned();
    let settings = CorpseSettings {
        hours_for_corpse_removal: std::env::var("HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(216.0),
        remove_player_corpses_on_corpse_removal: env_i("REMOVE_PLAYERS", 0) == 1,
        disable_fake_dead: env_i("FAKEDEAD", 2),
        maggot_spawn: env_i("MAGGOT", 1),
        reanimate: 3,
        tutorial: false,
        season: [
            "Default",
            "Spring",
            "Early Summer",
            "Late Summer",
            "Autumn",
            "Winter",
        ][env_i("SEASON", 3) as usize]
            .to_string(),
    };
    let mut lines = oracle.lines();
    let base: f64 = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut hooks = Items {
        registry: &registry,
        sm: &sm,
        display: &display,
    };
    let (mut world_rand, mut path, mut fbo) = (rand.clone(), NoMutationHooks, Fbo);
    let mut age_hooks = Age(base);
    rand.seed(4242);
    cell.dead_bodies.last_id = env_i("LASTID", 1000) as i64;
    {
        let mut env = MutationEnv {
            cell: &mut cell,
            meta: &meta,
            sprites: &mut sprites,
            regions: &mut regions,
            rand: &mut world_rand,
            doors: &doors,
            world: &mut age_hooks,
            pathfind: &mut path,
            render: &mut fbo,
            num_players: 1,
        };
        for &(wx, wy) in &order {
            dead_body::chunk_loaded(env.cell, wx, wy);
            dead_body::add_loaded_corpses_to_world(&mut env, wx, wy);
        }
    }
    let mut out = format!("base {base} lastId {}\n", cell.dead_bodies.last_id);
    let mut csq: Vec<SquarePos> = Vec::new();
    for &(wx, wy) in &order {
        let ch = &cell.chunks[&(wx, wy)];
        for z in ch.min_level..=ch.max_level {
            for y in 0..8 {
                for x in 0..8 {
                    if ch
                        .get_grid_square(x, y, z)
                        .is_some_and(|s| !s.static_moving.is_empty())
                    {
                        csq.push((wx * 8 + x, wy * 8 + y, z));
                    }
                }
            }
        }
        let n = dead_body::corpse_count::get_corpse_count(&cell, &meta, wx, wy, 0, None, None);
        writeln!(out, "count {wx},{wy} {n}").unwrap();
    }
    let mut step = 0;
    for l in lines {
        let Some(rest) = l.strip_prefix("step ") else {
            continue;
        };
        let f: Vec<&str> = rest.split(' ').collect();
        let age = f64::from_bits(f[2].parse::<u64>().unwrap());
        rand.seed(1000 + step as i64);
        let mut err = "";
        if step > 0 {
            let mut env = MutationEnv {
                cell: &mut cell,
                meta: &meta,
                sprites: &mut sprites,
                regions: &mut regions,
                rand: &mut world_rand,
                doors: &doors,
                world: &mut age_hooks,
                pathfind: &mut path,
                render: &mut fbo,
                num_players: 1,
            };
            if dead_body::update_bodies(&mut env, &mut hooks, &settings, age)
                == dead_body::UpdateBodiesOutcome::NullContainer
            {
                err = " ERR java.lang.NullPointerException";
            }
        }
        writeln!(
            out,
            "step {step} age {} rand {}{err}",
            age.to_bits(),
            rand.bits(32)
        )
        .unwrap();
        let ids = cell.dead_bodies.ids();
        let mut s = String::from("order");
        for id in &ids {
            write!(s, " {id}").unwrap();
        }
        writeln!(out, "{s}").unwrap();
        for id in ids {
            if let Some((p, _, b)) = cell.dead_bodies.find(&cell, id) {
                writeln!(out, "{}", body_line(p, b)).unwrap();
            }
        }
        for p in &csq {
            let sq = cell.get_grid_square(p.0, p.1, p.2).unwrap();
            writeln!(
                out,
                "sq {},{},{} corpses {} items {}",
                p.0,
                p.1,
                p.2,
                sq.static_moving.len(),
                sq.world_objects.len()
            )
            .unwrap();
        }
        step += 1;
    }
    let out_dir = Path::new(&a[6]);
    std::fs::create_dir_all(out_dir).unwrap();
    std::fs::write(out_dir.join("corpses.txt"), &out).unwrap();
    let mut first = None;
    let (mut same, mut total) = (0, 0);
    for (i, (j, r)) in oracle.lines().zip(out.lines()).enumerate() {
        total += 1;
        if j == r {
            same += 1;
        } else if first.is_none() {
            first = Some((i + 1, j.to_string(), r.to_string()));
        }
    }
    println!(
        "{same}/{total} lines equal (oracle {} lines, rust {})",
        oracle.lines().count(),
        out.lines().count()
    );
    if let Some((n, j, r)) = first {
        println!("first difference at line {n}:\n  java {j}\n  rust {r}");
        std::process::exit(1);
    }
}
