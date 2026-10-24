//! Runs saved vehicles through the vehicle runtime (pz-vehicles with the
//! game's own vehicle Lua on pz-kahlua) the way a local harness runs the
//! original classes, replaying the physics answers that harness recorded,
//! and compares the per-frame state dumps and the `Bullet` calls made.
//! No game data or harness output is part of this repository.
//!
//! ```text
//! vehicle_runtime_parity <game dir> <sprite tables> <save tables> <save dir>
//!     <rows dir> <item list dir> <oracle dir> [vehicle id]
//! ```
//!
//! `<rows dir>` holds `rows.tsv` and `rows/<id>.bin` (a `vehicles.db`);
//! `<item list dir>` the harness's `items.txt`; `<oracle dir>` the
//! harness's `sandbox.txt` and `v<id>/dump.txt` + `v<id>/bullet.log`.

use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_inventory::InventoryItem;
use pz_kahlua::{NativeFunction, TableRef, Value};
use pz_lua_api::lua_manager::run_lua;
use pz_lua_api::LuaManager;
use pz_scripts::objects::items::Item;
use pz_scripts::script_type::ScriptType;
use pz_vehicles::env::{self, GameTimeState, SandboxVehicle, SquareInfo, VehicleWorld};
use pz_vehicles::items::ItemStats;
use pz_vehicles::part::PartItem;
use pz_vehicles::physics::replay::ReplayPhysics;
use pz_vehicles::physics::Bullet;
use pz_vehicles::vehicle::{BaseVehicle, VehicleCharacter};
use pz_vehicles::{manager, rand, world_sim};
use pz_world::save::vehicle::VehicleRow;
use pz_world::save::SaveTables;
use pz_world::{IsoSpriteManager, SpriteTables};
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

fn fh(v: f32) -> String {
    format!("{:x}", v.to_bits())
}
fn dh(v: f64) -> String {
    format!("{:x}", v.to_bits())
}

/// The harness's world: no squares, the meta grid bounds it dumped.
struct OracleWorld {
    meta: (i32, i32, i32, i32),
    grid_width: i32,
}

impl VehicleWorld for OracleWorld {
    fn square(&self, _x: i32, _y: i32, _z: i32) -> Option<SquareInfo> {
        None
    }
    fn is_valid_chunk(&self, wx: i32, wy: i32) -> bool {
        let (min_x, min_y, max_x, max_y) = self.meta;
        let (wx, wy) = (wx.wrapping_mul(8), wy.wrapping_mul(8));
        !(wx < min_x.wrapping_mul(256)
            || wx >= (max_x + 1).wrapping_mul(256)
            || wy < min_y.wrapping_mul(256))
            && wy < (max_y + 1).wrapping_mul(256)
    }
    fn chunk_loaded(&self, _wx: i32, _wy: i32) -> bool {
        false
    }
    fn chunk_grid_width(&self) -> i32 {
        self.grid_width
    }
}

struct Items {
    sm: Rc<pz_scripts::ScriptManager>,
    registry: &'static ScriptItems,
}

/// The `Rand` draws `Item.InstanceItem` makes before the id: a radio's
/// `setRandomChannel` (no presets headless) and the `OutfitRNG` seed.
fn instance_draw_bounds(sm: &pz_scripts::ScriptManager, t: &pz_inventory::ItemTemplate) -> Vec<i32> {
    let mut b = Vec::new();
    if t.class == pz_inventory::ItemClass::Radio {
        if let Some(i) = sm.find_item(&t.full_type(), true).and_then(|s| s.downcast::<Item>()) {
            let i = i.borrow();
            let min = if (200..=1_000_000).contains(&i.min_channel) { i.min_channel } else { 200 };
            let max = if (200..=1_000_000).contains(&i.max_channel) { i.max_channel } else { 1_000_000 };
            b.push(-(max - min));
        }
    }
    b.push(i32::MAX);
    b
}

impl env::ItemFactory for Items {
    /// `InventoryItemFactory.CreateItem(fullType)` with the original's
    /// `Rand` draws.
    fn create_item(&self, full_type: &str) -> Option<InventoryItem> {
        use pz_inventory::ItemEnv as _;
        let id = self.registry.dictionary.item_id(full_type)?;
        let t = self.registry.template(id)?;
        for b in instance_draw_bounds(&self.sm, &t) {
            if b < 0 {
                rand::next_range(0, -b);
            } else {
                rand::next(b);
            }
        }
        let mut item = pz_inventory::env::create_item(self.registry, id)?;
        item.id = rand::next(2_146_250_223) + 1_233_423;
        Some(item)
    }
    fn stats(&self, full_type: &str) -> ItemStats {
        self.sm
            .find_item(full_type, true)
            .and_then(|s| s.downcast::<Item>())
            .map(|i| ItemStats::from_script(&i.borrow()))
            .unwrap_or_default()
    }
}

/// The item registry, noting the `Rand` draws the original's
/// `InventoryItemFactory.CreateItem` makes for every loaded item (its id).
struct DrawingItems<'a> {
    inner: &'a ScriptItems,
    sm: &'a pz_scripts::ScriptManager,
    bounds: std::cell::RefCell<Vec<i32>>,
}

impl pz_inventory::ItemEnv for DrawingItems<'_> {
    fn template(&self, id: i16) -> Option<Rc<pz_inventory::ItemTemplate>> {
        self.inner.template(id)
    }
    fn exists(&self, id: i16) -> bool {
        let e = self.inner.exists(id);
        if e {
            let t = self.inner.template(id);
            let mut b = self.bounds.borrow_mut();
            match &t {
                Some(t) => b.extend(instance_draw_bounds(self.sm, t)),
                None => b.push(i32::MAX),
            }
            b.push(2_146_250_223);
        }
        e
    }
    fn full_type(&self, id: i16) -> Option<String> {
        self.inner.full_type(id)
    }
    fn obsolete(&self, t: &str) -> bool {
        self.inner.obsolete(t)
    }
    fn world_age_hours(&self) -> f64 {
        self.inner.world_age_hours()
    }
    fn blood_body_parts(&self) -> usize {
        self.inner.blood_body_parts()
    }
    fn components(&self) -> &dyn pz_inventory::ComponentCodec {
        self.inner.components()
    }
    fn decodes(&self, t: &pz_inventory::ItemTemplate) -> bool {
        self.inner.decodes(t)
    }
    fn extra_item_round_trip(&self, id: i16) -> i16 {
        self.inner.extra_item_round_trip(id)
    }
    fn texture_round_trip(&self, t: &pz_inventory::ItemTemplate, name: &str) -> Option<String> {
        self.inner.texture_round_trip(t, name)
    }
    fn recorded_media_name(&self, i: i16) -> Option<Option<String>> {
        self.inner.recorded_media_name(i)
    }
    fn world_sprite(&self, name: &str) -> Option<pz_inventory::env::WorldSpriteInfo> {
        self.inner.world_sprite(name)
    }
    fn script_actual_weight(&self, name: &str) -> Option<f32> {
        self.inner.script_actual_weight(name)
    }
    fn rand_next(&self, bound: i32) -> i32 {
        self.bounds.borrow_mut().push(bound);
        0
    }
}

/// `Core.soundDisabled`: sounds play on a `DummySoundEmitter`.
struct NoSound;
impl env::VehicleEvents for NoSound {
    fn play_sound(&self, _name: &str) -> i64 {
        0
    }
}

struct Driver;
impl VehicleCharacter for Driver {}

/// The harness's `vdump` line.
fn vdump(v: &BaseVehicle, frame: i32) -> String {
    let mut s = String::new();
    write!(
        s,
        "V {} id={} pos={},{},{}",
        frame,
        v.id(),
        fh(v.x()),
        fh(v.y()),
        fh(v.z())
    )
    .unwrap();
    write!(s, " sc={} skin={}", v.script_name(), v.st().skin_index).unwrap();
    write!(
        s,
        " tr={} es={} rpm={}",
        v.transmission().letter(),
        v.engine_state().name(),
        dh(v.engine_speed())
    )
    .unwrap();
    write!(s, " thr={}", fh(v.st().throttle)).unwrap();
    if let Some(c) = v.st().ctrl {
        write!(
            s,
            " ctl={},{},{},{},{}{}{}",
            fh(c.engine_force),
            fh(c.braking_force),
            fh(c.vehicle_steering),
            c.is_enable as i32,
            c.is_gas as i32,
            c.is_gas_r as i32,
            c.is_break as i32
        )
        .unwrap();
    }
    write!(
        s,
        " spd={} bat={}",
        fh(v.current_speed_kmh()),
        fh(v.battery_charge())
    )
    .unwrap();
    let gas = v
        .gas_tank()
        .map_or("-".to_string(), |g| fh(g.container_content_amount()));
    write!(
        s,
        " gas={} mass={} brk={}",
        gas,
        fh(v.mass()),
        fh(v.braking_force())
    )
    .unwrap();
    write!(
        s,
        " eq={},{},{}",
        v.engine_quality(),
        v.engine_loudness(),
        v.engine_power()
    )
    .unwrap();
    write!(
        s,
        " lights={}{}",
        v.headlights_on() as i32,
        v.stoplights_on() as i32
    )
    .unwrap();
    {
        let st = v.st();
        write!(
            s,
            " good={} mech={} key={} rust={}",
            st.is_good_car as i32,
            st.mechanical_id,
            st.key_id,
            fh(st.rust)
        )
        .unwrap();
        write!(
            s,
            " col={},{},{}",
            fh(st.color_hue),
            fh(st.color_saturation),
            fh(st.color_value)
        )
        .unwrap();
        write!(
            s,
            " dur={},{},{},{}",
            st.front_end_durability,
            st.rear_end_durability,
            st.current_front_end_durability,
            st.current_rear_end_durability
        )
        .unwrap();
    }
    if v.script().is_some() {
        let p = v.poly();
        write!(
            s,
            " poly={},{},{},{},{},{},{},{}",
            fh(p.x1),
            fh(p.y1),
            fh(p.x2),
            fh(p.y2),
            fh(p.x3),
            fh(p.y3),
            fh(p.x4),
            fh(p.y4)
        )
        .unwrap();
        let q = v.poly_plus_radius();
        write!(
            s,
            " polyR={},{},{},{}",
            fh(q.x1),
            fh(q.y1),
            fh(q.x3),
            fh(q.y3)
        )
        .unwrap();
    }
    s.push_str(" parts=");
    for p in v.parts() {
        let id = p.id().unwrap_or_else(|| "null".into());
        write!(s, "{}:{}", id, p.condition()).unwrap();
        if let Some(PartItem::Item(i)) = p.st().item.clone() {
            let i = i.borrow();
            write!(s, ":i{}", i.condition).unwrap();
            if pz_vehicles::items::is_drainable(&i) {
                write!(s, "u{}", fh(pz_vehicles::items::current_uses_float(&i))).unwrap();
            }
        } else if p.has_item() {
            s.push_str(":i?");
        }
        if p.is_container() {
            write!(s, ":c{}", fh(p.container_content_amount())).unwrap();
        }
        let st = p.st();
        if let Some(l) = &st.light {
            write!(
                s,
                ":l{}{}{}f{}",
                l.active as i32,
                fh(l.offset.x),
                fh(l.dist),
                l.focusing
            )
            .unwrap();
        }
        if let Some(d) = &st.door {
            write!(
                s,
                ":d{}{}{}",
                d.open as i32, d.locked as i32, d.lock_broken as i32
            )
            .unwrap();
        }
        let md = st.mod_data.clone();
        drop(st);
        if let Some(t) = md.filter(|t| !t.is_empty()) {
            if let Value::Num(d) = t.get_str("temperature") {
                write!(s, ":t{}", dh(d)).unwrap();
            }
        }
        let st = p.st();
        write!(
            s,
            ":w{}{}{}",
            fh(st.wheel_friction),
            fh(st.suspension_compression),
            fh(st.suspension_damping)
        )
        .unwrap();
        write!(s, ":{};", fh(st.last_updated)).unwrap();
    }
    s
}

fn parse_hex_f(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}
fn parse_hex_d(s: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(s, 16).unwrap())
}

/// Applies a harness `F` line (clock and `GameTime` values).
fn apply_time(line: &str) {
    let t: Vec<&str> = line.split(' ').collect();
    env::set_now_ms(t[2].parse().unwrap());
    let mut g = GameTimeState::default();
    g.multiplier = parse_hex_f(t[3]);
    g.time_delta = parse_hex_f(t[4]);
    g.thirty_fps_multiplier = parse_hex_f(t[5]);
    g.inv_multiplier = parse_hex_f(t[6]);
    g.unmodded_multiplier = parse_hex_f(t[7]);
    g.physics_seconds_since_last_update = parse_hex_f(t[8]);
    g.world_age_hours = parse_hex_d(t[9]);
    g.delta_minutes_per_day = parse_hex_f(t[10]);
    env::set_time(g);
}

fn native(
    t: &TableRef,
    name: &str,
    f: impl Fn(&mut pz_kahlua::NativeCall<'_>) -> pz_kahlua::LuaResult<usize> + 'static,
) {
    t.set_str(name, Value::Native(NativeFunction::new(name, f)));
}

/// The globals the vehicle Lua reads that are not vehicle bindings
/// (`ZombRand`, `getGameTime`, `SandboxVars`, ...), backed by the runtime.
fn install_globals(lua: &mut LuaManager, sandbox_lines: &[String]) {
    let g = lua.thread.globals().clone();
    native(&g, "ZombRand", |c| {
        let n = c.nargs();
        let a = pz_kahlua::exec::raw_tonumber(&c.arg(0)).unwrap_or(0.0);
        let r = if n >= 2 {
            rand::zomb_rand_range(a, pz_kahlua::exec::raw_tonumber(&c.arg(1)).unwrap_or(0.0))
        } else {
            rand::zomb_rand(a)
        };
        Ok(c.push(r))
    });
    native(&g, "ZombRandFloat", |c| {
        let a = pz_kahlua::exec::raw_tonumber(&c.arg(0)).unwrap_or(0.0) as f32;
        let b = pz_kahlua::exec::raw_tonumber(&c.arg(1)).unwrap_or(0.0) as f32;
        Ok(c.push(rand::zomb_rand_float(a, b) as f64))
    });
    native(&g, "isServer", |c| Ok(c.push(false)));
    native(&g, "instanceItem", |c| {
        let t = c.arg(0).as_str().map(str::to_string);
        let item = t.and_then(|t| env::items().create_item(&t));
        let v = item.map_or(Value::Nil, |i| pz_lua_api::bind_vehicles::item_value(&Rc::new(std::cell::RefCell::new(i))));
        Ok(c.push(v))
    });
    native(&g, "isClient", |c| Ok(c.push(false)));
    let gt = TableRef::new();
    native(&gt, "getWorldAgeHours", |c| {
        Ok(c.push(env::time().world_age_hours))
    });
    let gt2 = gt.clone();
    native(&g, "getGameTime", move |c| Ok(c.push(gt2.clone())));
    let so = TableRef::new();
    native(&so, "getTimeSinceApo", |c| Ok(c.push(1.0)));
    let so2 = so.clone();
    native(&g, "getSandboxOptions", move |c| Ok(c.push(so2.clone())));
    let dt = TableRef::new();
    let general = TableRef::new();
    native(&general, "debugln", |_| Ok(0));
    dt.set_str("General", general);
    g.set_str("DebugType", dt);
    let lem = TableRef::new();
    let events = g.get_str("Events");
    native(&lem, "AddEvent", move |c| {
        if let (Value::Table(ev), Some(name)) = (&events, c.arg(0).as_str().map(str::to_string)) {
            if ev.get_str(&name).is_nil() {
                let e = TableRef::new();
                native(&e, "Add", |_| Ok(0));
                ev.set_str(&name, e);
            }
        }
        Ok(0)
    });
    g.set_str("LuaEventManager", lem);
    let sv = TableRef::new();
    for l in sandbox_lines {
        if l.starts_with('@') {
            continue;
        }
        let Some((key, val)) = l.split_once(' ') else {
            continue;
        };
        let v = match &val[..1] {
            "n" => Value::Num(parse_hex_d(&val[1..])),
            "b" => Value::Bool(&val[1..] == "1"),
            _ => Value::str(&val[1..]),
        };
        let mut t = sv.clone();
        let parts: Vec<&str> = key.split('.').collect();
        for p in &parts[..parts.len() - 1] {
            t = match t.get_str(p) {
                Value::Table(n) => n,
                _ => {
                    let n = TableRef::new();
                    t.set_str(p, n.clone());
                    n
                }
            };
        }
        t.set_str(parts[parts.len() - 1], v);
    }
    g.set_str("SandboxVars", sv);
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let save_tables = SaveTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let save_dir = Path::new(&a[4]);
    let rows_dir = Path::new(&a[5]);
    let items_dir = Path::new(&a[6]);
    let oracle = Path::new(&a[7]);
    let only: Option<i32> = a.get(8).map(|s| s.parse().unwrap());
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let mut tile_scale = 2;
    for line in std::fs::read_to_string(items_dir.join("items.txt"))
        .unwrap()
        .lines()
    {
        if let Some(t) = line.strip_prefix("#tileScale\t") {
            tile_scale = t.parse().unwrap();
        }
    }
    let sm = Rc::new(pz_scripts::ScriptManager::new());
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
    let class = save_tables.class_id("Vehicle");
    let offsets = (32.0 * tile_scale as f32, 96.0 * tile_scale as f32);
    let _ = ScriptType::Vehicle;

    // Runtime environment.
    pz_vehicles::vehicle::set_script_manager(sm.clone());
    let registry: &'static ScriptItems = Box::leak(Box::new(registry));
    env::set_items(Rc::new(Items { sm: sm.clone(), registry }));
    env::set_events(Rc::new(NoSound));
    let sandbox_lines: Vec<String> = std::fs::read_to_string(oracle.join("sandbox.txt"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    let at = |k: &str| {
        sandbox_lines
            .iter()
            .find_map(|l| l.strip_prefix(&format!("@{k} ")))
            .unwrap()
            .to_string()
    };
    let sandbox = SandboxVehicle {
        car_general_condition: at("carGeneralCondition").parse().unwrap(),
        recently_survivor_vehicles: at("recentlySurvivorVehicles").parse().unwrap(),
        vehicle_easy_use: at("vehicleEasyUse") == "true",
        zombie_attraction_multiplier: at("zombieAttractionMultiplier").parse().unwrap(),
        car_damage_on_impact: at("carDamageOnImpact").parse().unwrap(),
        player_damage_from_crash: at("playerDamageFromCrash") == "true",
        key_loot_new: at("keyLootNew").parse().unwrap(),
        siren_shutoff_hours: at("sirenShutoffHours").parse().unwrap(),
        ..Default::default()
    };
    env::set_sandbox(sandbox);
    rand::set_lock_fps(at("lockFPS").parse().unwrap());
    let meta: Vec<i32> = at("metaGrid")
        .split(' ')
        .map(|s| s.parse().unwrap())
        .collect();
    env::set_world(Rc::new(OracleWorld {
        meta: (meta[0], meta[1], meta[2], meta[3]),
        grid_width: at("chunkGridWidth").parse().unwrap(),
    }));
    world_sim::set_air_temperature(parse_hex_f(&at("airTemperature")));

    // Lua: the same files as the harness, on pz-kahlua.
    let fs = Rc::new(pz_fs::GameFs::new(game.to_str().unwrap()));
    let mut lua = LuaManager::init(fs).expect("lua");
    install_globals(&mut lua, &sandbox_lines);
    for f in [
        "media/lua/shared/luautils.lua",
        "media/lua/shared/VehicleZoneDefinition.lua",
        "media/lua/server/Vehicles/Vehicles.lua",
    ] {
        let p = game.join(f);
        run_lua(&mut lua.thread, &lua.loader, p.to_str().unwrap());
    }
    for e in lua.loader.borrow().errors.iter() {
        println!("lua load: {}", e.lines().next().unwrap_or(""));
    }

    let rows = std::fs::read_to_string(rows_dir.join("rows.tsv")).unwrap();
    let mut totals = (0usize, 0usize, 0usize, 0usize, 0usize);
    for line in rows.lines().filter(|l| !l.is_empty()) {
        let t: Vec<&str> = line.split('\t').collect();
        let id: i32 = t[0].parse().unwrap();
        if only.is_some_and(|o| o != id) {
            continue;
        }
        let vdir = oracle.join(format!("v{id}"));
        let Ok(java_dump) = std::fs::read_to_string(vdir.join("dump.txt")) else {
            continue;
        };
        let blog = std::fs::read_to_string(vdir.join("bullet.log")).unwrap();
        let row = VehicleRow {
            id,
            wx: t[1].parse().unwrap(),
            wy: t[2].parse().unwrap(),
            x: t[3].parse().unwrap(),
            y: t[4].parse().unwrap(),
            world_version: t[5].parse().unwrap(),
            in_meta: t[6] == "1",
            data: std::fs::read(rows_dir.join(format!("rows/{id}.bin"))).unwrap(),
        };
        let drawing = DrawingItems {
            inner: registry,
            sm: &sm,
            bounds: Default::default(),
        };
        let saved = match row.load(class, offsets, &drawing) {
            Ok(Some(v)) => v,
            other => {
                println!("v{id}: not loaded ({:?})", other.err());
                continue;
            }
        };
        let java_lines: Vec<&str> = java_dump
            .lines()
            .filter(|l| l.starts_with('V') || l.starts_with('E') || l.starts_with("D loadDraws"))
            .collect();
        let java_draws = java_lines
            .iter()
            .find(|l| l.starts_with("D loadDraws"))
            .map(|l| l.to_string());
        let java_lines: Vec<&str> = java_lines
            .into_iter()
            .filter(|l| !l.starts_with('D'))
            .collect();
        let times: Vec<&str> = java_dump.lines().filter(|l| l.starts_with("F ")).collect();
        let mut rust_lines: Vec<String> = Vec::new();
        let replay = ReplayPhysics::from_log(&blog);
        Bullet::install(Box::new(replay.clone()));
        let frames = times.len() as i32 - 1;
        pz_vehicles::lua::with_thread(&mut lua.thread, || {
            manager::reset();
            world_sim::reset();
            world_sim::set_created(0.0, 0.0);
            rand::seed_standard(id as i64);
            rand::seed_lua(id as i64 * 31 + 7);
            apply_time(times[0]);
            let bounds = drawing.bounds.borrow().clone();
            let v = pz_vehicles::save::from_save_with(&saved, || {
                for b in &bounds {
                    if *b < 0 {
                        rand::next_range(0, -*b);
                    } else {
                        rand::next(*b);
                    }
                }
            });
            rust_lines.push(format!("D loadDraws={}", bounds.len()));
            rand::seed_standard(id as i64 + 1_000_003);
            v.st_mut().chunk = Some((row.wx, row.wy));
            v.add_to_world();
            manager::process_add_vehicles();
            rust_lines.push(vdump(&v, -1));
            let driven = v.script().is_some_and(|s| s.borrow().wheel_count() > 0)
                && v.engine_part().is_some();
            let driver: Rc<dyn VehicleCharacter> = Rc::new(Driver);
            for frame in 0..frames {
                apply_time(times[frame as usize + 1]);
                if driven && frame == 2 {
                    let off = v
                        .passenger_position(0, "inside")
                        .map(|p| p.offset)
                        .unwrap_or_default();
                    v.set_passenger(0, Some(driver.clone()), off);
                    rust_lines.push("E enter".into());
                }
                if driven && frame == 3 {
                    v.set_hotwired(true);
                    rust_lines.push("E hotwire".into());
                }
                if driven && frame == 5 {
                    v.try_start_engine(true);
                    rust_lines.push("E tryStartEngine".into());
                }
                if driven && frame == 360 {
                    v.shut_off(pz_vehicles::engine::EngineReason::TurnedOff);
                    rust_lines.push("E shutOff".into());
                }
                if driven && frame == 380 {
                    v.clear_passenger(0);
                    rust_lines.push("E exit".into());
                }
                world_sim::update();
                if v.driver().is_some() && v.has_controller() {
                    let mut cc = pz_vehicles::controller::ClientControls::default();
                    if (60..200).contains(&frame) {
                        cc.forward = true;
                    }
                    if (120..160).contains(&frame) {
                        cc.steering = -1.0;
                    }
                    if (170..185).contains(&frame) {
                        cc.steering = 1.0;
                    }
                    if (200..230).contains(&frame) {
                        cc.brake = true;
                    }
                    if (240..290).contains(&frame) {
                        cc.backward = true;
                    }
                    if (300..340).contains(&frame) {
                        cc.forward = true;
                        cc.steering = 0.5;
                    }
                    v.set_client_controls(cc);
                    v.update_physics();
                }
                v.update();
                rust_lines.push(vdump(&v, frame));
            }
            v.remove_from_world();
        });
        Bullet::install(Box::new(pz_vehicles::physics::NoPhysics));
        let (mism, remaining) = (replay.mismatches(), replay.remaining());
        let rust_draws = rust_lines
            .iter()
            .find(|l| l.starts_with("D loadDraws"))
            .cloned();
        rust_lines.retain(|l| !l.starts_with('D'));
        if java_draws != rust_draws {
            println!(
                "v{id}: item load draws differ (item port): J {:?} R {:?}",
                java_draws, rust_draws
            );
        }
        let mut first_diff = None;
        let mut ndiff = 0;
        let mut field_counts: std::collections::BTreeMap<String, usize> = Default::default();
        for (i, (j, r)) in java_lines.iter().zip(rust_lines.iter()).enumerate() {
            if *j != r.as_str() {
                ndiff += 1;
                let mut fields: Vec<String> = Vec::new();
                for (a, b) in j.split(' ').zip(r.split(' ')) {
                    if a != b {
                        let key = a.split('=').next().unwrap_or("").to_string();
                        if key == "parts" {
                            for (pa, pb) in a.split(';').zip(b.split(';')) {
                                if pa != pb {
                                    let pid = pa.split(':').next().unwrap_or("");
                                    let fa: Vec<&str> = pa.split(':').collect();
                                    let fb: Vec<&str> = pb.split(':').collect();
                                    let k = fa.iter().zip(fb.iter()).position(|(x, y)| x != y).unwrap_or(0);
                                    fields.push(format!("parts.{pid}.{}", fa.get(k).map_or("", |x| &x[..1.min(x.len())])));
                                }
                            }
                        } else {
                            fields.push(key);
                        }
                    }
                }
                for f in fields {
                    *field_counts.entry(f).or_insert(0usize) += 1;
                }
                if first_diff.is_none() {
                    first_diff = Some((i, j.to_string(), r.clone()));
                }
            }
        }
        if java_lines.len() != rust_lines.len() {
            ndiff += 1;
        }
        totals.0 += 1;
        totals.1 += java_lines.len();
        totals.2 += ndiff;
        totals.3 += mism.len();
        totals.4 += remaining;
        println!(
            "v{id}: lines={} diff={} bullet_mismatch={} bullet_left={} lua_errors={}",
            java_lines.len(),
            ndiff,
            mism.len(),
            remaining,
            pz_vehicles::lua::take_errors().len()
        );
        if !field_counts.is_empty() {
            println!("  differing fields: {:?}", field_counts);
        }
        if let Some((i, j, r)) = first_diff {
            println!("  first diff at {i}:\n  J {j}\n  R {r}");
            for (k, (jw, rw)) in j.split(' ').zip(r.split(' ')).enumerate() {
                if jw != rw {
                    println!("  field {k}: J {jw}\n           R {rw}");
                    break;
                }
            }
        }
        if let Some((i, e, m)) = mism.first() {
            println!(
                "  first bullet mismatch at call {i}:\n  J {}\n  R {}",
                &e[..e.len().min(300)],
                &m[..m.len().min(300)]
            );
        }
    }
    println!(
        "vehicles={} lines={} line_diffs={} bullet_mismatches={} bullet_left={}",
        totals.0, totals.1, totals.2, totals.3, totals.4
    );
}
