//! Barricade, thumpable, door, power and light-item queries of
//! [`pz_world::WorldState`] over every chunk of a save, against the same
//! queries evaluated by the original game on the same files (made by a
//! local harness on a machine with the game installed; no game data is
//! part of this repository).
//!
//! ```text
//! world_hooks_parity <game dir> <sprite tables> <save tables> <save dir> <oracle dir>
//! ```
//!
//! `<oracle dir>` holds the original's `hooks.txt` (a `power` line with the
//! sandbox values it ran with, then per square and object lines),
//! `items.txt` and `summary.txt`. Writes `rust_hooks.txt` there and prints
//! line counts by kind.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_inventory::item::{InventoryItem, ItemKind};
use pz_world::objects::{IsoBarricade, ItemContainer, ObjectKind};
use pz_world::save::chunk_file::{chunk_path, load_chunk};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::{
    EdgeFacing, IsoCell, IsoMetaGrid, IsoSpriteManager, PowerSettings, SpriteTables, WorldState,
};

fn fx(f: f32) -> String {
    format!("{:x}", f.to_bits())
}

fn bar(b: Option<&IsoBarricade>) -> String {
    match b {
        None => "null".into(),
        Some(b) => format!(
            "{}/{}/{}/{}",
            b.forward_dir() as usize,
            b.num_planks(),
            b.is_metal() as u8,
            b.is_metal_bar() as u8
        ),
    }
}

fn light_items(out: &mut String, p: &str, items: &ItemContainer, depth: usize) {
    for (i, it) in items.items.iter().enumerate() {
        let Some(it) = it.as_item() else { continue };
        light_item(out, p, it, depth, i);
    }
}

fn light_item(out: &mut String, p: &str, it: &InventoryItem, depth: usize, i: usize) {
    let ls = it.get_light_strength();
    let (can, em) = (it.can_emit_light(), it.is_emitting_light());
    if ls != 0.0 || can || em || it.can_be_activated() {
        writeln!(
            out,
            "{p}/{depth}.{i} {} can={} emit={} ls={} dist={} cone={} dot={} act={}/{} uses={}",
            it.get_full_type(),
            can as u8,
            em as u8,
            fx(ls),
            it.get_light_distance(),
            it.is_torch_cone() as u8,
            fx(it.get_torch_dot()),
            it.can_be_activated() as u8,
            it.is_activated() as u8,
            it.get_current_uses()
        )
        .unwrap();
    }
    if depth < 8 {
        if let ItemKind::Container(c) = &it.kind {
            light_items(out, &format!("{p}/{depth}.{i}"), &c.container, depth + 1);
        }
    }
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
    let java = std::fs::read_to_string(oracle.join("hooks.txt")).unwrap();
    // The sandbox and world values the original ran with.
    let pl: HashMap<&str, &str> = java
        .lines()
        .next()
        .unwrap()
        .split(' ')
        .skip(1)
        .filter_map(|kv| kv.split_once('='))
        .collect();
    let power = PowerSettings {
        hydro_power_on: true,
        world_age_days: f32::from_bits(u32::from_str_radix(pl["worldAgeDays"], 16).unwrap()),
        elec_shut_modifier: pl["elecShut"].parse().unwrap(),
        allow_exterior_generator: pl["ext"] == "true",
        generator_tile_range: pl["range"].parse().unwrap(),
        generator_vertical_power_range: pl["vrange"].parse().unwrap(),
    };
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
    let ws = WorldState::new(&cell, &meta, &sprites, power);
    let mut out = java.lines().next().unwrap().to_string();
    out.push('\n');
    for &(wx, wy) in &order {
        let ch = &cell.chunks[&(wx, wy)];
        for x in 0..8 {
            for y in 0..8 {
                for z in ch.min_level..=ch.max_level {
                    let Some(sq) = ch.get_grid_square(x, y, z) else {
                        continue;
                    };
                    let at = format!("{},{},{}", sq.x, sq.y, sq.z);
                    let elec = ws.have_electricity(sq);
                    let mut bl = String::new();
                    for (dx, dy) in [(0, -1), (-1, 0), (0, 1), (1, 0)] {
                        let Some(o2) = cell.get_grid_square(sq.x + dx, sq.y + dy, sq.z) else {
                            bl.push('-');
                            continue;
                        };
                        bl.push(if sq.is_door_blocked_to(Some(o2), &cell, &ws) {
                            'D'
                        } else {
                            '.'
                        });
                        bl.push(if sq.is_window_blocked_to(Some(o2), &cell, &ws) {
                            'W'
                        } else {
                            '.'
                        });
                    }
                    if elec || bl.contains('D') || bl.contains('W') {
                        writeln!(out, "sq {at} elec={} blocked={bl}", elec as u8).unwrap();
                    }
                    for (oi, o) in sq.objects.iter().enumerate() {
                        let p = format!(" {at} {oi}");
                        match &o.kind {
                            ObjectKind::Barricade(b) => writeln!(
                                out,
                                "barricade{p} dir={} planks={} metal={} bar={} bv={} lt={} destroyed={}",
                                b.forward_dir() as usize,
                                b.num_planks(),
                                b.is_metal() as u8,
                                b.is_metal_bar() as u8,
                                b.is_block_vision() as u8,
                                fx(b.light_transmission()),
                                b.is_destroyed() as u8
                            )
                            .unwrap(),
                            ObjectKind::Door(_) | ObjectKind::Window(_) | ObjectKind::Thumpable(_) => {
                                let (kind, north, open) = match &o.kind {
                                    ObjectKind::Door(d) => ("door", d.north, d.open),
                                    ObjectKind::Window(w) => ("window", w.north, w.open),
                                    ObjectKind::Thumpable(t) => ("thump", t.north, t.open),
                                    _ => unreachable!(),
                                };
                                let (same, opp) = ws.barricades_for(sq, north);
                                write!(
                                    out,
                                    "{kind}{p} north={} open={} barricaded={} same={} opp={}",
                                    north as u8,
                                    open as u8,
                                    ws.object_barricaded(sq, o) as u8,
                                    bar(same),
                                    bar(opp)
                                )
                                .unwrap();
                                if let ObjectKind::Door(d) = &o.kind {
                                    let c = ws.door_curtain(o);
                                    write!(
                                        out,
                                        " curtain={} copen={}",
                                        c.is_some() as u8,
                                        (d.has_curtain && d.curtain_open) as u8
                                    )
                                    .unwrap();
                                }
                                if let ObjectKind::Thumpable(t) = &o.kind {
                                    let barr = ws.object_barricaded(sq, o);
                                    let blocked = |f: EdgeFacing| {
                                        (EdgeFacing::of_north(t.north) == f && t.is_blocked_door(barr))
                                            as u8
                                    };
                                    write!(
                                        out,
                                        " isDoor={} isWindow={} pass={} blockedNS={} blockedEW={} hp={}/{} frame={} canBar={} light={}",
                                        t.is_door as u8,
                                        ws.object_is_window(o) as u8,
                                        t.can_pass_through as u8,
                                        blocked(EdgeFacing::NorthSouth),
                                        blocked(EdgeFacing::EastWest),
                                        t.health,
                                        t.max_health,
                                        t.is_door_frame as u8,
                                        t.can_barricade as u8,
                                        t.light_source_on as u8
                                    )
                                    .unwrap();
                                }
                                out.push('\n');
                            }
                            ObjectKind::LightSwitch(s) => writeln!(
                                out,
                                "switch{p} can={} street={} room={} bulb={}",
                                ws.can_switch_light(sq, s) as u8,
                                s.street_light as u8,
                                s.light_room as u8,
                                s.bulb_item.is_some() as u8
                            )
                            .unwrap(),
                            ObjectKind::Generator(g) => writeln!(
                                out,
                                "gen{p} act={} conn={}",
                                g.activated as u8, g.connected as u8
                            )
                            .unwrap(),
                            _ => {}
                        }
                        let containers: Vec<&ItemContainer> = match &o.kind {
                            ObjectKind::Mannequin(Some(m)) => m.container.iter().collect(),
                            _ => o.containers().collect(),
                        };
                        for (ci, c) in containers.into_iter().enumerate() {
                            light_items(&mut out, &format!("item{p} c{ci}"), c, 0);
                        }
                        if let ObjectKind::WorldInventory(w) = &o.kind {
                            if let Some(it) = w.item.as_item() {
                                light_item(&mut out, &format!("item{p} w"), it, 0, 0);
                            }
                        }
                    }
                }
            }
        }
    }
    std::fs::write(oracle.join("rust_hooks.txt"), &out).unwrap();
    // Compare line by line, by kind.
    let mut counts = BTreeMap::<String, (usize, usize)>::new();
    let jl: Vec<&str> = java.lines().collect();
    let rl: Vec<&str> = out.lines().collect();
    let kind = |l: &str| l.split(' ').next().unwrap_or("").to_string();
    let mut shown = 0;
    let n = jl.len().max(rl.len());
    for i in 0..n {
        let (j, r) = (jl.get(i), rl.get(i));
        let k = kind(j.or(r).unwrap());
        let e = counts.entry(k).or_default();
        if j == r {
            e.0 += 1;
        } else {
            e.1 += 1;
            if shown < 10 {
                shown += 1;
                eprintln!(
                    "line {i}\n  java: {}\n  rust: {}",
                    j.unwrap_or(&""),
                    r.unwrap_or(&"")
                );
            }
        }
    }
    println!("chunks {} load errors {load_errors}", order.len());
    for (k, (same, diff)) in counts {
        println!("{k}: {same} equal, {diff} differ");
    }
}
