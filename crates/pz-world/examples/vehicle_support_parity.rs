//! Writes what the vehicle code reads from items and map zones, in the
//! format of a local harness that reads the same values from the original:
//!
//! - `created.txt`: every item script created with `Rand` seeded from the
//!   full type's `String.hashCode()` (vehicle-part values, capacities,
//!   weights, radio `DeviceData` with presets, map id, item id and the next
//!   draw);
//! - `vdb_parts.txt`: the part items and part containers of every
//!   `vehicles.db` row (`BaseVehicle.updateTotalMass` inputs);
//! - `vdb_draws.txt`: the `Rand` draws (`next(bits)` calls) each row's
//!   load makes (after the constructor's two), and `vdb_seq.txt` the
//!   bounds and values in order;
//! - `vzones.txt`: the vehicle zones of a map directory's `objects.lua` /
//!   `regions.lua` per cell, and `getVehicleZoneAt` at random points.
//!
//! ```text
//! vehicle_support_parity <game dir> <sprite tables> <save tables> <save dir> <vdb dir>
//!     <world age file> <oracle dir> <out dir> [<map dir>]
//! ```
//!
//! The radio channel lists, map definitions and `IsoDirections` names are
//! read from the oracle directory (runtime input).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

use pz_inventory::component::FactoryRng;
use pz_inventory::context::{ItemContext, NoOwner};
use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::{parse_item_names, ScriptItems};
use pz_inventory::item::{InventoryItem, ItemKind};
use pz_inventory::{ItemTemplate, SavedItem};
use pz_world::save::vehicle::VehicleRow;
use pz_world::save::SaveTables;
use pz_world::zones::{PropValue, ZoneGeometryType, ZoneProps};
use pz_world::{IsoMetaGrid, IsoSpriteManager, SpriteTables};

/// `java.util.Random`.
struct JavaRandom(u64);

/// `next(bits)` calls since the last reset (the harness counts the same).
static DRAWS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

thread_local! {
    /// The `nextInt(bound)` / `nextFloat()` calls of a row's load, as the
    /// harness lists them (`vdb_seq.txt`).
    static SEQ: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom((seed as u64 ^ 0x5DEECE66D) & ((1 << 48) - 1))
    }
    fn bits(&mut self, bits: u32) -> i32 {
        DRAWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - bits)) as i64 as i32
    }
    fn next_int(&mut self, bound: i32) -> i32 {
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

impl FactoryRng for JavaRandom {
    fn next_int(&mut self, bound: i32) -> i32 {
        let v = JavaRandom::next_int(self, bound);
        if bound > 0 {
            SEQ.with(|q| q.borrow_mut().push(format!("i{bound} {v}")));
        }
        v
    }
    fn next_float(&mut self, min: f32, max: f32) -> f32 {
        if max == min {
            return min;
        }
        let (min, max) = if min > max { (max, min) } else { (min, max) };
        let u = self.bits(24) as f32 / (1 << 24) as f32;
        SEQ.with(|q| q.borrow_mut().push(format!("f {u:?}")));
        min + u * (max - min)
    }
}

/// `String.hashCode()`.
fn java_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

fn f(v: f32) -> String {
    format!("f{:x}", v.to_bits())
}

fn b(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
    }
}

/// Script lookups for the weight rules; fluids through the default
/// `fluid_amount`.
struct Ctx {
    scripts: HashMap<String, Rc<ItemTemplate>>,
}

impl ItemContext for Ctx {
    fn world_age_hours(&self) -> f64 {
        0.0
    }
    fn multiplier(&self) -> f32 {
        1.0
    }
    fn rain_intensity(&self) -> f32 {
        0.0
    }
    fn is_raining(&self) -> bool {
        false
    }
    fn equipped_or_worn_encumbrance_multiplier(&self) -> f64 {
        1.0
    }
    fn basic_mask_defense(&self) -> f32 {
        0.0
    }
    fn improvised_gas_mask_defense(&self) -> f32 {
        0.0
    }
    fn standard_gas_mask_defense(&self) -> f32 {
        0.0
    }
    fn script(&self, name: &str) -> Option<Rc<ItemTemplate>> {
        let full = if name.contains('.') {
            name.to_string()
        } else {
            format!("Base.{name}")
        };
        self.scripts.get(&full).cloned()
    }
}

fn stats(i: &InventoryItem, ctx: &Ctx) -> String {
    let p = &i.vehicle_part;
    let mut o = format!(
        "class={} id={} ic={} mc={} cac={} bf={} dur={} ctsd={} cln={} clo={} wf={} sc={} el={} sd={} w={} uw={} aw={} cw={} cond={}/{}",
        i.class_name(),
        i.id,
        f(i.item_capacity),
        i.max_capacity,
        b(i.template.condition_affects_capacity),
        f(p.brake_force),
        f(p.durability),
        p.chance_to_spawn_damaged,
        f(p.condition_lower_normal),
        f(p.condition_lower_offroad),
        f(p.wheel_friction),
        f(p.suspension_compression),
        f(p.engine_loudness),
        f(p.suspension_damping),
        f(i.get_weight_with(ctx)),
        f(i.get_unequipped_weight(ctx, &NoOwner)),
        f(i.get_actual_weight_with(ctx)),
        f(i.get_contents_weight_with(ctx, &NoOwner)),
        i.condition,
        i.condition_max,
    );
    match &i.kind {
        ItemKind::Radio(_, d) => {
            write!(
                o,
                " dev={},{},{},{},{},{},{},{},{},{},{},{},{},{},{} presets=",
                d.device_name,
                b(d.two_way),
                d.transmit_range,
                d.mic_range,
                f(d.base_volume_range),
                b(d.is_portable),
                b(d.is_television),
                b(d.is_high_tier),
                d.channel,
                d.min_channel_range,
                d.max_channel_range,
                b(d.is_battery_powered),
                f(d.use_delta),
                d.media_type,
                b(d.no_transmit)
            )
            .unwrap();
            for p in &d.presets.presets {
                write!(o, "{}:{};", p.name, p.frequency).unwrap();
            }
        }
        ItemKind::Map { map_id, .. } => write!(o, " map={map_id}").unwrap(),
        _ => {}
    }
    o
}

/// A Kahlua properties table as zone properties.
struct LuaProps(pz_kahlua::TableRef);

impl ZoneProps for LuaProps {
    fn rawget(&self, key: &str) -> PropValue {
        use pz_kahlua::value::Value;
        match self.0.get_str(key) {
            Value::Nil => PropValue::Nil,
            Value::Bool(v) => PropValue::Bool(v),
            Value::Num(d) => PropValue::Number(d),
            Value::Str(s) => PropValue::Str(s.to_string()),
            _ => PropValue::Other,
        }
    }
}

fn zdig(z: &pz_world::VehicleZone) -> String {
    let geom = match z.geometry_type {
        ZoneGeometryType::Invalid => "INVALID",
        ZoneGeometryType::Point => "POINT",
        ZoneGeometryType::Polyline => "POLYLINE",
        ZoneGeometryType::Polygon => "POLYGON",
    };
    let pts = format!(
        "{{{}}}",
        z.points
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    format!(
        "{}|{}|{},{},{},{},{}|dir={}|flags={}|geom={geom}|pts={pts}|pw={}",
        z.name,
        z.zone_type,
        z.x,
        z.y,
        z.z,
        z.w,
        z.h,
        z.dir.map_or("null".to_string(), |d| format!("{d:?}")),
        i32::from(z.face_direction),
        z.polyline_width
    )
}

fn vehicle_zones(map_dir: &Path, oracle: &Path) -> String {
    use pz_kahlua::value::Value;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for e in std::fs::read_dir(map_dir).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        let Some(stem) = n.strip_suffix(".lotheader") else {
            continue;
        };
        let (x, y) = stem.split_once('_').unwrap();
        let (x, y): (i32, i32) = (x.parse().unwrap(), y.parse().unwrap());
        (min_x, min_y, max_x, max_y) = (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y));
    }
    let mut g = IsoMetaGrid::with_bounds(min_x, min_y, max_x, max_y);
    g.zone_tables.iso_direction_names = std::fs::read_to_string(oracle.join("directions.txt"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    let skip = [
        "Vegitation",
        "DeepForest",
        "Forest",
        "TownZone",
        "Farm",
        "FarmLand",
        "TrailerPark",
        "Nav",
    ];
    let special = [
        "Animal",
        "Basement",
        "Mannequin",
        "RoomTone",
        "SpawnOrigin",
        "WaterFlow",
        "WaterZone",
        "Region",
        "BuildingName",
        "WorldGen",
    ];
    let mut registered = 0;
    for (file, global) in [("objects.lua", "objects"), ("regions.lua", "regions")] {
        let Ok(src) = std::fs::read_to_string(map_dir.join(file)) else {
            continue;
        };
        let mut t = pz_kahlua::Thread::new(pz_kahlua::TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        let func = pz_kahlua::stdlib::load_string(&src, Some(file), t.globals().clone()).unwrap();
        t.pcall(&func, &[]).map_err(|_| "lua error").unwrap();
        let Value::Table(objs) = t.globals().get_str(global) else {
            continue;
        };
        let regions = global == "regions";
        for k in 1.. {
            let Value::Table(v) = objs.get_int(k) else {
                break;
            };
            let s = |key: &str| match v.get_str(key) {
                Value::Str(s) => s.to_string(),
                _ => String::new(),
            };
            let n = |key: &str| match v.get_str(key) {
                Value::Num(d) => d as i32,
                _ => 0,
            };
            let ty = s("type");
            if regions {
                if ty == "Mannequin" || ty == "Region" || ty == "BuildingName" {
                    continue;
                }
            } else if skip.contains(&ty.as_str())
                || special.contains(&ty.as_str())
                || !matches!(v.get_str("geometry"), Value::Nil)
            {
                continue;
            }
            let props = match v.get_str("properties") {
                Value::Table(p) => Some(LuaProps(p)),
                _ => None,
            };
            let z = g
                .register_vehicles_zone(
                    &s("name"),
                    &ty,
                    n("x"),
                    n("y"),
                    n("z"),
                    n("width"),
                    n("height"),
                    props.as_ref().map(|p| p as &dyn ZoneProps),
                )
                .unwrap();
            if z.is_some() {
                registered += 1;
            }
        }
    }
    g.check_vehicles_zones();
    let mut out = format!(
        "registered={registered} global={}\n",
        g.vehicles_zones.len()
    );
    let (w, h) = (max_x - min_x + 1, max_y - min_y + 1);
    for gx in 0..w {
        for gy in 0..h {
            let zones = g.cell_vehicle_zones(gx + min_x, gy + min_y);
            if zones.is_empty() {
                continue;
            }
            writeln!(out, "cell {},{} n={}", gx + min_x, gy + min_y, zones.len()).unwrap();
            for z in &zones {
                writeln!(out, "  {}", zdig(z)).unwrap();
            }
        }
    }
    let mut r = JavaRandom::new(77);
    for _ in 0..20000 {
        let px = min_x * 256 + r.next_int(w * 256);
        let py = min_y * 256 + r.next_int(h * 256);
        if let Some(z) = g.vehicle_zone_at(px, py, 0) {
            writeln!(out, "at {px},{py} {}", zdig(&z)).unwrap();
        }
    }
    out
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let save_tables = SaveTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let save_dir = Path::new(&a[4]);
    let vdb_dir = Path::new(&a[5]);
    let world_age: f64 = std::fs::read_to_string(&a[6])
        .unwrap()
        .split_whitespace()
        .find_map(|w| w.strip_prefix("worldAgeHours="))
        .unwrap()
        .parse()
        .unwrap();
    let oracle = Path::new(&a[7]);
    let out_dir = Path::new(&a[8]);
    std::fs::create_dir_all(out_dir).unwrap();
    if let Some(map_dir) = a.get(9) {
        std::fs::write(
            out_dir.join("vzones.txt"),
            vehicle_zones(Path::new(map_dir), oracle),
        )
        .unwrap();
    }

    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game.join("projectzomboid.jar"))
        .unwrap();
    sm.load(&pz_fs::GameFs::new(game.to_str().unwrap()), &[])
        .unwrap();
    let names = parse_item_names(
        &std::fs::read_to_string(game.join("media/lua/shared/Translate/EN/ItemName.json")).unwrap(),
    );
    let dict = WorldDictionary::load(&std::fs::read(save_dir.join("WorldDictionary.bin")).unwrap())
        .unwrap();
    let mut env = ScriptItems::new(&sm, dict, &names);
    env.world_age_hours = world_age;
    env.blood_body_parts = save_tables.blood_body_parts;
    env.world_sprites = pz_world::save::items::moveable_sprites(&sprites);
    let jar = pz_classfile::Jar::open(game.join("projectzomboid.jar")).unwrap();
    let entity = pz_entity::BasicContext {
        attributes: pz_entity::AttributeTypes::from_jar(&jar).unwrap(),
        fluids: Some(pz_entity::FluidRegistry::from_scripts(&jar, &sm).unwrap()),
        ..Default::default()
    };
    let entity: &'static pz_entity::BasicContext = Box::leak(Box::new(entity));
    env.components = Box::new(pz_inventory::component::EntityCodec { ctx: entity });
    env.assets = Some(pz_inventory::ItemAssets::load(game).unwrap());
    let mut channels: HashMap<String, Vec<(i32, String)>> = HashMap::new();
    for line in std::fs::read_to_string(oracle.join("channels.txt"))
        .unwrap()
        .lines()
    {
        let t: Vec<&str> = line.split('\t').collect();
        channels
            .entry(t[0].to_string())
            .or_default()
            .push((t[1].parse().unwrap(), t[2].to_string()));
    }
    env.channels = Some(Box::new(channels));
    env.map_definitions = std::fs::read_to_string(oracle.join("mapdefs.txt"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();

    let mut scripts: Vec<(String, Rc<ItemTemplate>)> = Vec::new();
    for it in pz_scripts::objects::items::get_all_items(&sm) {
        let full = it.borrow().full_name();
        let Some(t) = ItemTemplate::from_script_ref(&sm, &it) else {
            continue;
        };
        let t = match names.get(&full) {
            Some(n) if *n != t.display_name => {
                let mut t = (*t).clone();
                t.display_name = n.clone();
                Rc::new(t)
            }
            _ => t,
        };
        scripts.push((full, t));
    }
    scripts.sort_by(|a, b| a.0.cmp(&b.0));
    let ctx = Ctx {
        scripts: scripts.iter().cloned().collect(),
    };

    // Created items.
    let mut created = String::new();
    for (full, t) in &scripts {
        if full.starts_with("Moveables.") {
            continue;
        }
        *env.rand.borrow_mut() = Some(Box::new(JavaRandom::new(java_hash(full) as i64)));
        let i = pz_inventory::env::create_from_template(&env, t.clone(), 0);
        let next = pz_inventory::env::ItemEnv::rand_next(&env, 1_000_000);
        writeln!(created, "{full}\t{} next={next}", stats(&i, &ctx)).unwrap();
    }
    std::fs::write(out_dir.join("created.txt"), created).unwrap();

    // vehicles.db rows.
    let class = save_tables.class_id("Vehicle");
    let ts: f32 = std::fs::read_to_string(oracle.join("items.txt"))
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("#tileScale\t"))
        .map_or(2.0, |t| t.parse().unwrap());
    let mut parts = String::new();
    let mut draws = String::new();
    let mut draws_seq = String::new();
    let rows = std::fs::read_to_string(vdb_dir.join("rows.tsv")).unwrap_or_default();
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
            data: std::fs::read(vdb_dir.join(format!("rows/{}.bin", t[0]))).unwrap(),
        };
        *env.rand.borrow_mut() = Some(Box::new(JavaRandom::new(row.id as i64)));
        DRAWS.store(0, std::sync::atomic::Ordering::Relaxed);
        SEQ.with(|q| q.borrow_mut().clear());
        env.raw_counts.borrow_mut().clear();
        // `new BaseVehicle(cell)` before the load: `setKeyId(Rand.Next(100000000))`
        // and `rust = Rand.Next(0, 2)` (pz-vehicles' constructor makes the same).
        let _ = pz_inventory::env::ItemEnv::rand_next(&env, 100_000_000);
        let _ = pz_inventory::env::ItemEnv::rand_next(&env, 2);
        let loaded = row.load(class, (32.0 * ts, 96.0 * ts), &env);
        let drawn = DRAWS.load(std::sync::atomic::Ordering::Relaxed);
        let seq = SEQ.with(|q| std::mem::take(&mut *q.borrow_mut()));
        if !env.raw_counts.borrow().is_empty() {
            eprintln!("row {} raw items {:?}", t[0], env.raw_counts.borrow());
        }
        let v = match loaded {
            Ok(Some(v)) => v,
            Ok(None) => continue,
            Err(e) => {
                writeln!(parts, "vehicle {} ERROR {e}", row.id).unwrap();
                continue;
            }
        };
        let mut plus = 0.0f32;
        for p in &v.parts {
            let mut l = format!("part {} {}", row.id, p.id());
            if let Some(c) = &p.container {
                let items: Vec<&InventoryItem> =
                    c.items.iter().filter_map(SavedItem::as_item).collect();
                let cw = c.get_contents_weight_with(&ctx);
                plus += cw;
                write!(l, " cont={}/{}", f(cw), c.items.len()).unwrap();
                for ci in items {
                    write!(
                        l,
                        " [{} {} {}]",
                        ci.get_full_type(),
                        f(ci.get_unequipped_weight(&ctx, &NoOwner)),
                        f(ci.get_contents_weight_with(&ctx, &NoOwner))
                    )
                    .unwrap();
                }
            }
            if let Some(i) = p.item.as_ref().and_then(SavedItem::as_item) {
                plus += i.get_weight_with(&ctx);
                write!(l, " item={} {}", i.get_full_type(), stats(i, &ctx)).unwrap();
            }
            parts.push_str(&l);
            parts.push('\n');
        }
        writeln!(parts, "vehicle {} plus={}", row.id, f(plus)).unwrap();
        writeln!(draws, "draws {} n={drawn}", row.id).unwrap();
        for q in seq {
            writeln!(draws_seq, "seq {} {q}", row.id).unwrap();
        }
    }
    std::fs::write(out_dir.join("vdb_parts.txt"), parts).unwrap();
    std::fs::write(out_dir.join("vdb_draws.txt"), draws).unwrap();
    std::fs::write(out_dir.join("vdb_seq.txt"), draws_seq).unwrap();
}
