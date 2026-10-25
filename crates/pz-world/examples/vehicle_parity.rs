//! Vehicles, pushables and animal tracks against the original game's own
//! load and save (made by a local harness on a machine with the game
//! installed; no game data is part of this repository).
//!
//! ```text
//! vehicle_parity <game dir> <sprite tables> <save tables> <save dir> <rows dir> <oracle dir>
//! ```
//!
//! `<rows dir>` holds `rows.tsv` (`id wx wy x y worldversion inMeta`) and
//! `rows/<id>.bin`, the `vehicles` table of a `vehicles.db`. `<oracle dir>`
//! holds the original's `vdb/<id>.bin` re-saves and `vdb_digest.txt`, and
//! for synthetic chunk files (`synth/map/...`, chunks with pushables,
//! wheelie bins, animal tracks and vehicle lists added by the original's
//! classes) `resave/map/...`, `resave/vehicles/<wx>_<wy>_<i>.bin` and
//! `synth_digest.txt`. Prints counts per kind.

use pz_world::ObjectSlot;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_inventory::java::sort_string_hash_map;
use pz_world::objects::{ItemContainer, ObjectKind};
use pz_world::save::chunk_file::{chunk_path, load_chunk, save_chunk};
use pz_world::save::io::Writer;
use pz_world::save::items::ItemBlob;
use pz_world::save::vehicle::{rotation_round_trip, BaseVehicle, VehicleRow};
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::{IsoSpriteManager, SpriteTables};

fn f(v: f32) -> String {
    format!("f{:x}", v.to_bits())
}

fn d(v: f64) -> String {
    format!("d{:x}", v.to_bits())
}

fn b(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
    }
}

fn item(i: Option<&ItemBlob>) -> String {
    match i {
        None => "null".into(),
        Some(ItemBlob::Item(i)) => format!("{}#{}c{}", i.get_full_type(), i.id, i.condition),
        Some(ItemBlob::Raw(r)) => format!("raw{}", r.len()),
    }
}

fn cont(c: Option<&ItemContainer>) -> String {
    match c {
        None => "null".into(),
        Some(c) => format!(
            "{}/{}/{}/{}{}",
            c.kind.as_deref().unwrap_or("null"),
            c.items.len(),
            c.capacity,
            b(c.explored),
            b(c.has_been_looted)
        ),
    }
}

/// The oracle's `vdig` line for a vehicle after `save`.
fn vdig(v: &BaseVehicle) -> String {
    let mut s = String::new();
    let rot = rotation_round_trip(v.saved_rot);
    let e = v.engine.clone().unwrap_or_default();
    let md = match &v.base.mod_data {
        None => "null".to_string(),
        Some(t) => (if t.is_empty() { "true" } else { "false" }).to_string(),
    };
    write!(
        s,
        "pos={},{},{} dir={} md={md} pz={} rot={},{},{},{} script={} skin={} eng={},{},{},{} dur={},{},{},{} key={},{} fl={}{}{}{} lb={},{} k={}{}{}{} rust={} col={},{},{} vid={} mech={} alarmed={} alarm={},null siren={} ckey={} blood=",
        f(v.base.x), f(v.base.y), f(v.base.z), v.base.dir, f(v.saved_physics_z),
        f(rot[0]), f(rot[1]), f(rot[2]), f(rot[3]), v.script_name, v.skin_index,
        b(e.running), e.loudness, e.quality, e.power,
        v.front_end_durability, v.rear_end_durability, v.current_front_end_durability, v.current_rear_end_durability,
        v.key_id, v.key_spawned, b(v.headlights_on), b(v.created), b(v.sound_horn_on), b(v.sound_back_move_on),
        v.lightbar_lights_mode, v.lightbar_siren_mode,
        b(v.key_is_on_door), b(v.hotwired), b(v.hotwired_broken), b(v.keys_in_ignition),
        f(v.rust), f(v.color_hue), f(v.color_saturation), f(v.color_value), v.vehicle_id, v.mechanical_id, b(v.alarmed),
        d(v.alarm_start_time), d(v.siren_start_time),
        item(v.current_key.as_ref()),
    )
    .unwrap();
    let mut blood = v.blood_intensity.clone();
    sort_string_hash_map(&mut blood, |e| e.0.as_str());
    for (k, x) in &blood {
        write!(s, "{k}:{x};").unwrap();
    }
    match &v.towing {
        Some(t) => write!(
            s,
            " tow={},{},{},{}",
            t.vehicle_id,
            t.attachment_self,
            t.attachment_other,
            f(t.z_offset)
        ),
        None => write!(s, " tow=-1,null,null,f0"),
    }
    .unwrap();
    let animals = if v.has_animals() {
        i32::from_be_bytes(v.animals[1..5].try_into().unwrap())
    } else {
        0
    };
    write!(
        s,
        " reg={} prev={}{} animals={animals} parts={}",
        f(v.regulator_speed),
        b(v.previously_entered),
        b(v.previously_moved),
        v.parts.len()
    )
    .unwrap();
    for p in &v.parts {
        let md = match &p.mod_data {
            None => "null".to_string(),
            Some(t) => (if t.is_empty() { "true" } else { "false" }).to_string(),
        };
        let dev = p.device_data.as_ref().map_or("null".into(), |x| {
            format!("{},{},{}", x.device_name, b(x.is_turned_on), x.channel)
        });
        let light = p.light.as_ref().map_or("null".into(), |l| {
            format!(
                "{},{},{},{},{},{}",
                b(l.active),
                f(l.offset_x),
                f(l.offset_y),
                f(l.intensity),
                f(l.dist),
                l.focusing
            )
        });
        let door = p.door.as_ref().map_or("null".into(), |x| {
            format!("{}{}{}", b(x.open), b(x.locked), b(x.lock_broken))
        });
        let win = p.window.as_ref().map_or("null".into(), |x| {
            format!("{},{}", b(x.open), f(x.open_delta))
        });
        write!(
            s,
            " | {} {} {} item={} cont={} md={md} dev={dev} light={light} door={door} win={win} cond={} wf={} msi={} sc={} sd={} ent={}",
            p.part_id, b(p.created), f(p.last_updated), item(p.item.as_ref()), cont(p.container.as_ref()),
            p.condition, f(p.wheel_friction), p.mechanic_skill_installer, f(p.suspension_compression),
            f(p.suspension_damping), b(!p.components.is_empty())
        )
        .unwrap();
    }
    s
}

/// Lines of the oracle digest keyed by their first two words.
fn digest_map(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, ' ');
            let (a, k, rest) = (it.next()?, it.next()?, it.next()?);
            Some((format!("{a} {k}"), rest.to_string()))
        })
        .collect()
}

fn first_diff(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()))
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let save_tables = SaveTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let save_dir = Path::new(&a[4]);
    let rows_dir = Path::new(&a[5]);
    let oracle = Path::new(&a[6]);
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
    let class = save_tables.class_id("Vehicle");
    let ts = tile_scale as f32;
    let offsets = (32.0 * ts, 96.0 * ts);
    let mut counts = BTreeMap::<String, usize>::new();
    macro_rules! c {
        ($k:expr) => {
            *counts.entry($k.to_string()).or_default() += 1
        };
    }
    let mut diffs = String::new();

    // vehicles.db rows.
    let jd =
        digest_map(&std::fs::read_to_string(oracle.join("vdb_digest.txt")).unwrap_or_default());
    let rows = std::fs::read_to_string(rows_dir.join("rows.tsv")).unwrap_or_default();
    for line in rows.lines().filter(|l| !l.is_empty()) {
        let t: Vec<&str> = line.split('\t').collect();
        let row = VehicleRow {
            id: t[0].parse().unwrap(),
            wx: t[1].parse().unwrap(),
            wy: t[2].parse().unwrap(),
            x: t[3].parse().unwrap(),
            y: t[4].parse().unwrap(),
            world_version: t[5].parse().unwrap(),
            in_meta: t[6] == "1",
            data: std::fs::read(rows_dir.join(format!("rows/{}.bin", t[0]))).unwrap(),
        };
        c!("vdb rows");
        let v = match row.load(class, offsets, &registry) {
            Ok(Some(v)) => v,
            Ok(None) => {
                c!("vdb skipped");
                continue;
            }
            Err(e) => {
                c!("vdb load errors");
                writeln!(diffs, "VDB LOADERR {} {e}", row.id).unwrap();
                continue;
            }
        };
        if v.has_animals() {
            c!("vdb with trailer animals (kept as bytes)");
        }
        let key = format!("vehicle {}", row.id);
        if jd.get(&key).map(String::as_str) == Some(vdig(&v).as_str()) {
            c!("vdb decode == java");
        } else {
            c!("vdb decode differs");
            writeln!(
                diffs,
                "VDB DIGEST {}\n  rust: {}\n  java: {}",
                row.id,
                vdig(&v),
                jd.get(&key).cloned().unwrap_or_default()
            )
            .unwrap();
        }
        let mut w = Writer::new();
        if let Err(e) = v.save(&mut w, class, row.square(), None, &registry) {
            c!("vdb save errors");
            writeln!(diffs, "VDB SAVEERR {} {e}", row.id).unwrap();
            continue;
        }
        if w.buf == row.data {
            c!("vdb resave == input");
        }
        let _ = std::fs::create_dir_all(oracle.join("rust_vdb"));
        std::fs::write(oracle.join(format!("rust_vdb/{}.bin", row.id)), &w.buf).unwrap();
        let java = std::fs::read(oracle.join(format!("vdb/{}.bin", row.id))).unwrap_or_default();
        if w.buf == java {
            c!("vdb resave == java resave");
        } else {
            c!("vdb resave differs");
            let at = first_diff(&w.buf, &java);
            writeln!(
                diffs,
                "VDB BYTES {} rust={} java={} first={at} rust:{:02x?} java:{:02x?}",
                row.id,
                w.buf.len(),
                java.len(),
                &w.buf[at.saturating_sub(8)..(at + 16).min(w.buf.len())],
                &java[at.saturating_sub(8).min(java.len())..(at + 16).min(java.len())]
            )
            .unwrap();
        }
    }

    // Synthetic chunk files.
    let ctx = SaveCtx {
        sprites: &sprites,
        tables: &save_tables,
        settings: &settings,
        items: &registry,
        hooks: &NoSaveHooks,
        meta: None,
    };
    let sd = std::fs::read_to_string(oracle.join("synth_digest.txt")).unwrap_or_default();
    let mut java_lines: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for l in sd.lines() {
        let mut it = l.splitn(3, ' ');
        let (kind, at) = (it.next().unwrap(), it.next().unwrap());
        let chunk = at.splitn(3, ',').take(2).collect::<Vec<_>>().join(",");
        java_lines
            .entry(format!("{kind} {chunk}"))
            .or_default()
            .push(l.to_string());
    }
    let synth = oracle.join("synth");
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(synth.join("map")) {
        for dx in rd {
            let dx = dx.unwrap();
            let Ok(wx) = dx.file_name().to_string_lossy().parse::<i32>() else {
                continue;
            };
            for fy in std::fs::read_dir(dx.path()).unwrap() {
                let n = fy.unwrap().file_name().to_string_lossy().to_string();
                if let Some(wy) = n.strip_suffix(".bin").and_then(|s| s.parse::<i32>().ok()) {
                    files.push((wx, wy));
                }
            }
        }
    }
    files.sort();
    for &(wx, wy) in &files {
        c!("synth chunks");
        let bytes = std::fs::read(chunk_path(&synth, wx, wy)).unwrap();
        let loaded = match load_chunk(&bytes, wx, wy, &ctx) {
            Ok(l) => l,
            Err(e) => {
                c!("synth load errors");
                writeln!(diffs, "SYNTH LOADERR {wx},{wy} {e}").unwrap();
                continue;
            }
        };
        let ch = &loaded.chunk;
        // Vehicles of the chunk's vehicle list.
        let jv = java_lines
            .get(&format!("vehicle {wx},{wy}"))
            .cloned()
            .unwrap_or_default();
        let loaded_vehicles: Vec<_> = ch.vehicles.loaded().collect();
        if jv.len() != loaded_vehicles.len() {
            writeln!(
                diffs,
                "SYNTH VEHICLE COUNT {wx},{wy} rust={} java={}",
                loaded_vehicles.len(),
                jv.len()
            )
            .unwrap();
        }
        for (i, cv) in loaded_vehicles.iter().enumerate() {
            c!("chunk vehicles");
            let line = format!("vehicle {wx},{wy},{i} {}", vdig(&cv.vehicle));
            if jv.get(i) == Some(&line) {
                c!("chunk vehicle decode == java");
            } else {
                c!("chunk vehicle decode differs");
                writeln!(
                    diffs,
                    "SYNTH VDIGEST {wx},{wy},{i}\n  rust: {line}\n  java: {}",
                    jv.get(i).cloned().unwrap_or_default()
                )
                .unwrap();
            }
            let square = cv.square.map(|(x, y, _)| (x, y));
            let mut w = Writer::new();
            cv.vehicle
                .save(&mut w, class, square, None, &registry)
                .unwrap();
            let java = std::fs::read(oracle.join(format!("resave/vehicles/{wx}_{wy}_{i}.bin")))
                .unwrap_or_default();
            if w.buf == java {
                c!("chunk vehicle resave == java");
            } else {
                c!("chunk vehicle resave differs");
                writeln!(
                    diffs,
                    "SYNTH VBYTES {wx},{wy},{i} first={}",
                    first_diff(&w.buf, &java)
                )
                .unwrap();
            }
        }
        // Pushables, wheelie bins and animal tracks.
        let mut rt = Vec::new();
        let mut rp = Vec::new();
        for x in 0..8 {
            for y in 0..8 {
                for z in ch.min_level..=ch.max_level {
                    let Some(sq) = ch.get_grid_square(x, y, z) else {
                        continue;
                    };
                    for o in sq.borrow().objects.iter().map(ObjectSlot::borrow) {
                        match &o.kind {
                            ObjectKind::AnimalTrack(t) => rt.push(format!(
                                "track {wx},{wy} {} {} {} {} {} {} {} sprite={}",
                                t.animal_type,
                                t.track_type,
                                t.x,
                                t.y,
                                t.dir.map_or("null".into(), |d| {
                                    ["N", "NW", "W", "SW", "S", "SE", "E", "NE"][d as usize]
                                        .to_string()
                                }),
                                t.added_time,
                                t.added_to_world,
                                o.sprite
                                    .and_then(|s| sprites.sprites[s].name.clone())
                                    .unwrap_or_else(|| "null".into())
                            )),
                            ObjectKind::Pushable(p) => rp.push(format!(
                                "pushable {wx},{wy} {} {} {} {} dir={} off={},{} sprite={} cont={}",
                                if p.wheelie_bin {
                                    "IsoWheelieBin"
                                } else {
                                    "IsoPushableObject"
                                },
                                f(p.base.x),
                                f(p.base.y),
                                f(p.base.z),
                                ["N", "NW", "W", "SW", "S", "SE", "E", "NE"][p.base.dir as usize],
                                f(p.base.offset_x),
                                f(p.base.offset_y),
                                o.sprite.map_or_else(
                                    || o.sprite_id_override
                                        .map_or("null".into(), |i| i.to_string()),
                                    |s| ctx.sprite_id(s).to_string()
                                ),
                                o.container.as_ref().map_or("null".into(), |c| format!(
                                    "{}/{}/{}",
                                    c.kind.as_deref().unwrap_or("null"),
                                    c.items.len(),
                                    c.capacity
                                ))
                            )),
                            _ => {}
                        }
                    }
                }
            }
        }
        for (kind, mine) in [("track", rt), ("pushable", rp)] {
            let java = java_lines
                .get(&format!("{kind} {wx},{wy}"))
                .cloned()
                .unwrap_or_default();
            *counts.entry(format!("{kind}s")).or_default() += java.len();
            let same = mine.iter().filter(|l| java.contains(l)).count();
            *counts.entry(format!("{kind} decode == java")).or_default() += same;
            if same != java.len() || mine.len() != java.len() {
                writeln!(
                    diffs,
                    "SYNTH {kind} {wx},{wy}\n  rust: {mine:?}\n  java: {java:?}"
                )
                .unwrap();
            }
        }
        let out = match save_chunk(ch, &ctx) {
            Ok(o) => o,
            Err(e) => {
                c!("synth save errors");
                writeln!(diffs, "SYNTH SAVEERR {wx},{wy} {e}").unwrap();
                continue;
            }
        };
        let java = std::fs::read(chunk_path(&oracle.join("resave"), wx, wy)).unwrap_or_default();
        if out == java {
            c!("synth chunk resave == java");
        } else {
            c!("synth chunk resave differs");
            writeln!(
                diffs,
                "SYNTH BYTES {wx},{wy} first={}",
                first_diff(&out, &java)
            )
            .unwrap();
        }
    }
    std::fs::write(oracle.join("rust_diffs.txt"), diffs).unwrap();
    for (k, v) in registry.raw_counts.borrow().iter() {
        println!("RAW {v} {k}");
    }
    println!("{counts:?}");
}
