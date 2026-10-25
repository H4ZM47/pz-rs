//! `IsoGridSquare.testCollideAdjacent(mover, x, y, z)` for no mover, a
//! zombie (standing, crawling, on the floor), a player, a no-clip player
//! and an animal towards the 26 neighbours (with the special object hit),
//! `isHoppableTo` / `getHoppableTo` towards the 8 same-level neighbours,
//! `haveDoor`, and `IsoObject.isHoppable` / `isTallHoppable`, over every
//! square near a door, window, window frame, thumpable, barricade or
//! hoppable, against the same calls evaluated by the original game on the
//! same files (made by a local harness on a machine with the game
//! installed; no game data is part of this repository). The collide
//! matrices of both come from `RecalcAllWithNeighbours(true)` on every
//! square, with the meta grid spanning the loaded chunks.
//!
//! ```text
//! mover_parity <game dir> <sprite tables> <save tables> <save dir> <oracle dir>
//! ```
//!
//! `<oracle dir>` holds the original's `mover.txt`, `items.txt` and
//! `summary.txt`. Writes `rust_mover.txt` there and prints the counts.

use std::fmt::Write as _;
use std::path::Path;

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_world::objects::ObjectKind;
use pz_world::save::chunk_file::{chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::square::{CollideEnv, CollideMover, MoverKind};
use pz_world::{IsoCell, IsoMetaGrid, IsoSpriteManager, SpriteTables};
use pz_world_states::CollideMoverInfo;

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
    let mut meta = IsoMetaGrid::empty();
    let (mut min_wx, mut min_wy, mut max_wx, mut max_wy) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(wx, wy) in &order {
        min_wx = min_wx.min(wx);
        min_wy = min_wy.min(wy);
        max_wx = max_wx.max(wx);
        max_wy = max_wy.max(wy);
    }
    meta.min_x = (min_wx * 8).div_euclid(256);
    meta.max_x = (max_wx * 8).div_euclid(256);
    meta.min_y = (min_wy * 8).div_euclid(256);
    meta.max_y = (max_wy * 8).div_euclid(256);
    // `IsoWindow.load` changes the shared window sprites as it reads them.
    for &(wx, wy) in &order {
        pz_world::save::chunk_file::apply_loaded_window_sprites(&cell.chunks[&(wx, wy)], &mut sprites);
    }
    for &(wx, wy) in &order {
        let (min, max) = {
            let ch = &cell.chunks[&(wx, wy)];
            (ch.min_level, ch.max_level)
        };
        for z in min..=max {
            for y in 0..8 {
                for x in 0..8 {
                    let (sx, sy) = (wx * 8 + x, wy * 8 + y);
                    if cell.get_grid_square(sx, sy, z).is_some() {
                        cell.recalc_all_with_neighbours(sx, sy, z, &meta, &sprites);
                    }
                }
            }
        }
    }
    let movers: Vec<Option<CollideMoverInfo>> = vec![
        None,
        Some(mover(MoverKind::Zombie, false, false)),
        Some(mover(MoverKind::Zombie, true, false)),
        Some(mover(MoverKind::Zombie, false, false)),
        Some(mover(MoverKind::Player, false, false)),
        Some(mover(MoverKind::Player, false, true)),
        Some(mover(MoverKind::Animal, false, false)),
    ];
    let mut out = String::new();
    let (mut n_sq, mut n_calls, mut n_blocked, mut n_special) = (0, 0, 0, 0);
    for &(wx, wy) in &order {
        let (min, max) = {
            let ch = &cell.chunks[&(wx, wy)];
            (ch.min_level, ch.max_level)
        };
        for x in 0..8 {
            for y in 0..8 {
                for z in min..=max {
                    let (sx, sy) = (wx * 8 + x, wy * 8 + y);
                    let Some(sq) = cell.get_grid_square(sx, sy, z) else {
                        continue;
                    };
                    let near = (-1..=1).any(|dx| {
                        (-1..=1).any(|dy| {
                            cell.get_grid_square(sx + dx, sy + dy, z).is_some_and(|s| {
                                !s.special_objects.is_empty()
                                    || s.objects.iter().any(|o| {
                                        let o = o.borrow();
                                        o.moved_thumpable
                                            || matches!(o.kind, ObjectKind::WindowFrame { .. })
                                            || o.is_hoppable(&sprites)
                                    })
                            })
                        })
                    });
                    if !near {
                        continue;
                    }
                    n_sq += 1;
                    write!(out, "mv {sx},{sy},{z} d={} ", sq.have_door() as i32).unwrap();
                    for dz in -1..=1 {
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                if (dx, dy, dz) == (0, 0, 0) {
                                    continue;
                                }
                                for m in &movers {
                                    n_calls += 1;
                                    let r = cell
                                        .test_collide_adjacent_mover_mut(
                                            (sx, sy, z),
                                            m.as_ref().map(|m| m as &dyn CollideMover),
                                            (dx, dy, dz),
                                            &meta,
                                            &sprites,
                                        )
                                        .unwrap();
                                    if r.blocked {
                                        n_blocked += 1;
                                    }
                                    out.push(if r.blocked { '1' } else { '0' });
                                    if let Some(sp) = r.special {
                                        n_special += 1;
                                        let o = sp.object;
                                        let osq = cell.get_grid_square(o.x, o.y, o.z).unwrap();
                                        write!(
                                            out,
                                            "@{},{},{},{}{}",
                                            o.x,
                                            o.y,
                                            o.z,
                                            osq.object_index(o.id).unwrap(),
                                            if sp.door { "D" } else { "" }
                                        )
                                        .unwrap();
                                    }
                                    out.push(',');
                                }
                                out.push('/');
                            }
                        }
                    }
                    out.push_str(" h ");
                    let env = CollideEnv::new(&cell, &sprites, &meta);
                    let sq = cell.get_grid_square(sx, sy, z).unwrap();
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            if dx == 0 && dy == 0 {
                                continue;
                            }
                            let Some(nb) = cell.get_grid_square(sx + dx, sy + dy, z) else {
                                out.push_str("-/");
                                continue;
                            };
                            out.push(if env.is_hoppable_to(sq, Some(nb)) {
                                '1'
                            } else {
                                '0'
                            });
                            if let Some((osq, id)) = env.get_hoppable_to(sq, Some(nb)) {
                                write!(
                                    out,
                                    "@{},{},{},{}",
                                    osq.x,
                                    osq.y,
                                    osq.z,
                                    osq.object_index(id).unwrap()
                                )
                                .unwrap();
                            }
                            out.push('/');
                        }
                    }
                    out.push_str(" o ");
                    for o in &sq.objects {
                        let o = o.borrow();
                        write!(
                            out,
                            "{}{}.",
                            o.is_hoppable(&sprites) as i32,
                            o.is_tall_hoppable(&sprites) as i32
                        )
                        .unwrap();
                    }
                    out.push('\n');
                }
            }
        }
    }
    std::fs::write(oracle.join("rust_mover.txt"), &out).unwrap();
    let java = std::fs::read_to_string(oracle.join("mover.txt")).unwrap();
    let (jl, rl): (Vec<&str>, Vec<&str>) = (java.lines().collect(), out.lines().collect());
    let (mut eq, mut ne, mut shown) = (0, 0, 0);
    for i in 0..jl.len().max(rl.len()) {
        let (j, r) = (
            jl.get(i).copied().unwrap_or(""),
            rl.get(i).copied().unwrap_or(""),
        );
        if j == r {
            eq += 1;
        } else {
            ne += 1;
            if shown < 6 {
                shown += 1;
                eprintln!("java: {j}\nrust: {r}");
            }
        }
    }
    println!("chunks {} load errors {load_errors}", order.len());
    println!(
        "squares {n_sq} calls {n_calls} blocked {n_blocked} special {n_special}; lines {eq} equal, {ne} differ"
    );
}

fn mover(kind: MoverKind, crawling: bool, no_clip: bool) -> CollideMoverInfo {
    CollideMoverInfo {
        kind,
        crawling,
        no_clip,
        ..Default::default()
    }
}
