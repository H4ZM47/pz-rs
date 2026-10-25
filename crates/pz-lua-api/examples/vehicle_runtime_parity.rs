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
use pz_scripts::script_type::ScriptType;
use pz_vehicles::env::{self, GameTimeState, SandboxVehicle, SquareInfo, VehicleWorld};
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
    /// No map data: `PolygonalMap2.canStandAt` finds no square.
    fn can_stand_at(&self, _x: f32, _y: f32, _z: i32, _v: &BaseVehicle) -> bool {
        false
    }
}

struct Items {
    registry: &'static ScriptItems,
    ctx: ItemCtx,
}

/// `Rand` for the item factory (`InventoryItemFactory.CreateItem`).
struct VehicleRand;
impl pz_inventory::component::FactoryRng for VehicleRand {
    fn next_int(&mut self, max: i32) -> i32 {
        rand::next(max)
    }
    fn next_float(&mut self, min: f32, max: f32) -> f32 {
        rand::next_float(min, max)
    }
}

impl env::ItemFactory for Items {
    /// `InventoryItemFactory.CreateItem(fullType)` (the factory makes the
    /// original's `Rand` draws).
    fn create_item(&self, full_type: &str) -> Option<InventoryItem> {
        let id = self.registry.dictionary.item_id(full_type)?;
        pz_inventory::env::create_item(self.registry, id)
    }
    fn context(&self) -> &dyn pz_inventory::ItemContext {
        &self.ctx
    }
}

/// Weights read item scripts (ammo) and decode fluid containers.
struct ItemCtx {
    registry: &'static ScriptItems,
}
impl pz_inventory::ItemContext for ItemCtx {
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
    fn script(&self, name: &str) -> Option<Rc<pz_inventory::ItemTemplate>> {
        use pz_inventory::ItemEnv as _;
        let full = if name.contains('.') {
            name.to_string()
        } else {
            format!("Base.{name}")
        };
        let id = self.registry.dictionary.item_id(&full)?;
        self.registry.template(id)
    }
}

/// The item registry, noting the `Rand` draws the original's
/// `InventoryItemFactory.CreateItem` makes for every loaded item (its id).
struct DrawingItems<'a> {
    inner: &'a ScriptItems,
    /// Draw from the vehicle `Rand` stream (else record and answer 0/min).
    live: bool,
    bounds: std::cell::RefCell<Vec<i32>>,
}

impl pz_inventory::ItemEnv for DrawingItems<'_> {
    fn template(&self, id: i16) -> Option<Rc<pz_inventory::ItemTemplate>> {
        self.inner.template(id)
    }
    fn exists(&self, id: i16) -> bool {
        self.inner.exists(id)
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
        if self.live {
            rand::next(bound)
        } else {
            0
        }
    }
    fn rand_float(&self, min: f32, max: f32) -> f32 {
        self.bounds.borrow_mut().push(i32::MIN);
        if self.live {
            rand::next_float(min, max)
        } else {
            min
        }
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
    let g = GameTimeState {
        multiplier: parse_hex_f(t[3]),
        time_delta: parse_hex_f(t[4]),
        thirty_fps_multiplier: parse_hex_f(t[5]),
        inv_multiplier: parse_hex_f(t[6]),
        unmodded_multiplier: parse_hex_f(t[7]),
        physics_seconds_since_last_update: parse_hex_f(t[8]),
        world_age_hours: parse_hex_d(t[9]),
        delta_minutes_per_day: parse_hex_f(t[10]),
        ..Default::default()
    };
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
        let v = item.map_or(Value::Nil, |i| {
            pz_lua_api::bind_vehicles::item_value(&Rc::new(std::cell::RefCell::new(i)))
        });
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
    registry.rand = std::cell::RefCell::new(Some(Box::new(VehicleRand)));
    // The harness runs without tile definitions (`IsoSpriteManager.namedMap`
    // is empty), so `Moveable.ReadFromWorldSprite` finds no sprite there.
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
    env::set_items(Rc::new(Items {
        registry,
        ctx: ItemCtx { registry },
    }));
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
    let fs = Rc::new(std::cell::RefCell::new(pz_fs::GameFs::new(
        game.to_str().unwrap(),
    )));
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

    if a[5] == "--surround" {
        pz_vehicles::lua::with_thread(&mut lua.thread, || run_surround(oracle));
        return;
    }
    if a[5] == "--dispatch" {
        pz_vehicles::lua::with_thread(&mut lua.thread, || run_dispatch(oracle, &sandbox_lines));
        return;
    }
    if a[5] == "--spawn" {
        pz_vehicles::lua::with_thread(&mut lua.thread, || run_spawn(oracle, &sandbox_lines));
        return;
    }

    // Cases: the rows of a vehicles.db, or (rows dir `--scripts`) a new
    // vehicle of every script the harness spawned.
    struct Case {
        id: i32,
        dir: std::path::PathBuf,
        chunk: (i32, i32),
        saved: Option<pz_world::save::vehicle::BaseVehicle>,
        bounds: Vec<i32>,
        script: Option<String>,
        row: Option<VehicleRow>,
    }
    let mut cases = Vec::new();
    let e2e = a[5] == "--e2e";
    if a[5] == "--scripts" || e2e {
        let mut i = 1;
        while oracle.join(format!("s{i}")).exists() {
            let dir = oracle.join(format!("s{i}"));
            let dump = std::fs::read_to_string(dir.join("dump.txt")).unwrap_or_default();
            let script = dump
                .lines()
                .find_map(|l| l.strip_prefix("S "))
                .map(str::to_string);
            if only.is_none_or(|o| o == i) {
                cases.push(Case {
                    id: i,
                    dir,
                    chunk: if e2e {
                        (1348 + i % 8, 1203 + (i / 8) % 8)
                    } else {
                        (1250 + i % 7, 1100 + i / 7)
                    },
                    saved: None,
                    bounds: Vec::new(),
                    script,
                    row: None,
                });
            }
            i += 1;
        }
    } else {
        let rows = std::fs::read_to_string(rows_dir.join("rows.tsv")).unwrap();
        for line in rows.lines().filter(|l| !l.is_empty()) {
            let t: Vec<&str> = line.split('\t').collect();
            let id: i32 = t[0].parse().unwrap();
            if only.is_some_and(|o| o != id) {
                continue;
            }
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
                live: false,
                bounds: Default::default(),
            };
            let saved = match row.load(class, offsets, &drawing) {
                Ok(Some(v)) => v,
                other => {
                    println!("v{id}: not loaded ({:?})", other.err());
                    continue;
                }
            };
            let bounds = drawing.bounds.borrow().clone();
            if std::env::var("LOAD_TRACE")
                .ok()
                .and_then(|v| v.parse::<i32>().ok())
                == Some(id)
            {
                let mut m = std::collections::BTreeMap::new();
                for b in &bounds {
                    *m.entry(*b).or_insert(0) += 1;
                }
                println!("load draws v{id}: {m:?}");
                for b in &bounds {
                    println!("load draw {b}");
                }
            }
            cases.push(Case {
                id,
                dir: oracle.join(format!("v{id}")),
                chunk: (row.wx, row.wy),
                saved: Some(saved),
                bounds,
                script: None,
                row: Some(row),
            });
        }
    }
    // `--e2e`: pz-bullet with the region's chunk shapes instead of the
    // recorded calls (the harness ran the original's libPZBullet).
    let region_bufs: Vec<Vec<u8>> = if e2e {
        let region = std::env::var("E2E_REGION").expect("E2E_REGION");
        let data = std::fs::read(Path::new(&region).join("tobullet.bin")).unwrap();
        let mut out = Vec::new();
        let mut p = 0;
        while p < data.len() {
            let n = i32::from_le_bytes(data[p + 12..p + 16].try_into().unwrap()) as usize;
            out.push(data[p + 16..p + 16 + n].to_vec());
            p += 16 + n;
        }
        out
    } else {
        Vec::new()
    };
    let mut totals = (0usize, 0usize, 0usize, 0usize, 0usize);
    for case in cases {
        let id = case.id;
        let Ok(java_dump) = std::fs::read_to_string(case.dir.join("dump.txt")) else {
            continue;
        };
        let blog = std::fs::read_to_string(case.dir.join("bullet.log")).unwrap_or_default();
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
        if e2e {
            let script = case.script.clone().unwrap_or_default();
            let params = pz_vehicles::vehicle::find_vehicle_script(&script)
                .and_then(|s| s.borrow().bullet_params().map(|p| p.to_vec()));
            // Each case ends by installing `NoPhysics`: a new PzBullet per
            // case (the scripts the case uses are defined below).
            Bullet::install(Box::new(pz_bullet::api::PzBullet::new()));
            Bullet::call(|b| {
                b.destroy_world();
                b.init_world(-250, -250, 250, 250, -64000, -64000, false)?;
                b.activate_chunk_map(0, 1345, 1200, 15)?;
                for buf in &region_bufs {
                    b.to_bullet(buf);
                }
                if let Some(p) = &params {
                    b.define_vehicle_script(&script, p)?;
                }
                Ok(())
            });
        } else {
            Bullet::install(Box::new(replay.clone()));
        }
        let frames = times.len() as i32 - 1;
        pz_vehicles::lua::with_thread(&mut lua.thread, || {
            manager::reset();
            world_sim::reset();
            if e2e {
                world_sim::set_created(-64000.0, -64000.0);
            } else {
                world_sim::set_created(0.0, 0.0);
            }
            rand::seed_standard(id as i64);
            rand::seed_lua(id as i64 * 31 + 7);
            apply_time(times[0]);
            let bounds = case.bounds.clone();
            let live_draws = std::cell::RefCell::new(0usize);
            let v = match &case.saved {
                Some(saved) => pz_vehicles::save::from_save_with(saved, || {
                    // The load's item draws, live on the vehicle stream
                    // (the loaded fields come from the first load).
                    let live = DrawingItems {
                        inner: registry,
                        live: true,
                        bounds: Default::default(),
                    };
                    if let Some(row) = &case.row {
                        let _ = row.load(class, offsets, &live);
                    }
                    *live_draws.borrow_mut() = live.bounds.borrow().len();
                }),
                None => {
                    let v = BaseVehicle::new();
                    v.set_script_name(case.script.as_deref().unwrap_or(""));
                    let (wx, wy) = case.chunk;
                    v.set_position((wx * 8) as f32 + 4.5, (wy * 8) as f32 + 3.25, 0.0);
                    let q = pz_vehicles::jmath::Quaternionf::angle_axis(
                        id as f32 * 0.37,
                        0.0,
                        1.0,
                        0.0,
                    );
                    v.st_mut().saved_rot = q;
                    v.st_mut().jni_transform.set_rotation(&q);
                    v
                }
            };
            let draws = if case.row.is_some() {
                *live_draws.borrow()
            } else {
                bounds.len()
            };
            rust_lines.push(format!("D loadDraws={}", draws));
            if std::env::var_os("NO_RESEED").is_none() {
                rand::seed_standard(id as i64 + 1_000_003);
            }
            v.st_mut().chunk = Some(case.chunk);
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
        let (mism, remaining) = if e2e {
            (Vec::new(), 0)
        } else {
            (replay.mismatches(), replay.remaining())
        };
        let rust_draws = rust_lines
            .iter()
            .find(|l| l.starts_with("D loadDraws"))
            .cloned();
        rust_lines.retain(|l| !l.starts_with('D'));
        // `RandAbstract.id` wraps at 10000.
        let count = |l: &Option<String>| {
            l.as_deref()
                .and_then(|l| l.strip_prefix("D loadDraws="))
                .and_then(|n| n.parse::<i32>().ok())
                .map(|n| n.rem_euclid(10_000))
        };
        if count(&java_draws) != count(&rust_draws) {
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
                                    let k = fa
                                        .iter()
                                        .zip(fb.iter())
                                        .position(|(x, y)| x != y)
                                        .unwrap_or(0);
                                    fields.push(format!(
                                        "parts.{pid}.{}",
                                        fa.get(k).map_or("", |x| &x[..1.min(x.len())])
                                    ));
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
            "{}{id}: lines={} diff={} bullet_mismatch={} bullet_left={} lua_errors={}",
            if case.script.is_some() { "s" } else { "v" },
            java_lines.len(),
            ndiff,
            mism.len(),
            remaining,
            pz_vehicles::lua::take_errors().len()
        );
        for e in pz_vehicles::physics::take_exceptions().iter().take(3) {
            println!("  physics exception: {e}");
        }
        for e in pz_vehicles::lua::take_errors().iter().take(2) {
            println!("  lua: {}", e.lines().next().unwrap_or(""));
        }
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

/// The harness's synthetic chunk map: every chunk loaded, a square missing
/// where `(7x + 3y) mod 23 == 0`, indoors where `(x + 2y) mod 9 == 0`.
struct SpawnTestWorld;

impl pz_vehicles::spawn::SpawnWorld for SpawnTestWorld {
    fn square_exists(&self, x: i32, y: i32, z: i32) -> bool {
        z == 0 && (x * 7 + y * 3).rem_euclid(23) != 0
    }
    fn is_outside(&self, x: i32, y: i32, _z: i32) -> bool {
        (x + 2 * y).rem_euclid(9) != 0
    }
    fn vehicle_zone_at(&self, _x: i32, _y: i32, _z: i32) -> bool {
        false
    }
    fn vehicle_zones(&self, _cx: i32, _cy: i32) -> Vec<pz_vehicles::spawn::VehicleZone> {
        Vec::new()
    }
    fn chunk_vehicles(&self, _wx: i32, _wy: i32) -> Option<Vec<pz_vehicles::VehicleRef>> {
        Some(Vec::new())
    }
}

/// `--spawn`: the `IsoChunk.AddVehicles` helpers on the harness's zones,
/// compared with `spawn.txt`.
fn spawn_options(sandbox_lines: &[String]) -> pz_vehicles::spawn::SpawnOptions {
    let opt = |k: &str| {
        sandbox_lines.iter().find_map(|l| {
            let (a, b) = l.split_once(' ')?;
            if a != k {
                return None;
            }
            match b.as_bytes().first()? {
                b'n' => Some(f64::from_bits(u64::from_str_radix(&b[1..], 16).ok()?)),
                b'b' => Some(if &b[1..] == "1" { 1.0 } else { 0.0 }),
                _ => None,
            }
        })
    };
    let mut options = pz_vehicles::spawn::SpawnOptions::default();
    if let Some(v) = opt("CarSpawnRate") {
        options.car_spawn_rate = v as i32;
    }
    if let Some(v) = opt("CarAlarm") {
        options.car_alarm = v as i32;
    }
    if let Some(v) = opt("TrafficJam") {
        options.traffic_jam = v != 0.0;
    }
    if let Some(v) = opt("EnableVehicles") {
        options.enable_vehicles = v != 0.0;
    }
    options
}

fn run_spawn(oracle: &std::path::Path, sandbox_lines: &[String]) {
    use pz_vehicles::spawn::{self, ChunkSpawner, VehicleZone};
    let options = spawn_options(sandbox_lines);
    world_sim::set_created(0.0, 0.0);
    if std::env::var_os("SPAWN_DEBUG").is_some() {
        let t = pz_vehicles::vehicle_type::type_from_name("parkingstall");
        println!("parkingstall: {:?}", t.map(|t| t.vehicles_definition.len()));
    }
    let text = std::fs::read_to_string(oracle.join("spawn.txt")).unwrap();
    let mut lines = text.lines();
    let head: Vec<i32> = lines
        .next()
        .unwrap()
        .split(' ')
        .skip(1)
        .map(|v| v.parse().unwrap())
        .collect();
    let (wx, wy) = (head[0], head[1]);
    let mut java: Vec<(String, Vec<String>)> = Vec::new();
    for l in lines {
        if l.starts_with("C ") {
            java.push((l.to_string(), Vec::new()));
        } else if let Some(c) = java.last_mut() {
            c.1.push(l.to_string());
        }
    }
    let world = SpawnTestWorld;
    let (mut ok, mut bad, mut vehicles) = (0, 0, 0);
    for (header, expect) in &java {
        let t: Vec<&str> = header.split(' ').collect();
        let i: i64 = t[1].parse().unwrap();
        let kind: i32 = t[2].parse().unwrap();
        let name = t[3];
        let n = |k: usize| t[k].parse::<i32>().unwrap();
        let mut zone = VehicleZone {
            name: name.to_string(),
            zone_type: "ParkingStall".to_string(),
            x: n(4),
            y: n(5),
            z: 0,
            w: n(6),
            h: n(7),
            dir: pz_world::IsoDirections::from_name(t[8]),
            face_direction: t[9] == "true",
            geometry_type: pz_world::ZoneGeometryType::Invalid,
            points: Vec::new(),
            polyline_width: 0,
        };
        if kind == 1 || kind == 3 {
            zone.geometry_type = pz_world::ZoneGeometryType::Polyline;
            zone.points = t[11].split(',').map(|v| v.parse().unwrap()).collect();
            zone.polyline_width = n(10);
        }
        spawn::reset_checked_vehicles();
        rand::seed_standard(7000 + i);
        let mut sp = ChunkSpawner::new(wx, wy, &world, options.clone());
        match kind {
            0 => sp.on_zone(&zone, name),
            1 => {
                sp.on_zone_polyline(&zone, name);
            }
            3 => sp.traffic_jam_polyline(&zone, name),
            _ => {
                let d = match name {
                    "TrafficJamW" => pz_world::IsoDirections::W,
                    "TrafficJamE" => pz_world::IsoDirections::E,
                    "TrafficJamS" => pz_world::IsoDirections::S,
                    _ => pz_world::IsoDirections::N,
                };
                sp.traffic_jam(&zone, name, d)
            }
        }
        let mut got = Vec::new();
        for v in &sp.vehicles {
            let st = v.st();
            let q = st.saved_rot;
            let o = st.jni_transform.origin;
            got.push(format!(
                "V {} skin={} type={} zone={} pos={},{},{} rot={},{},{},{} o={},{},{} rust={} alarm={} key={} color={}",
                st.script_name,
                st.skin_index,
                st.vehicle_type,
                st.respawn_zone,
                fh(st.x),
                fh(st.y),
                fh(st.z),
                fh(q.x),
                fh(q.y),
                fh(q.z),
                fh(q.w),
                fh(o.x),
                fh(o.y),
                fh(o.z),
                fh(st.rust),
                st.alarmed,
                st.key_id,
                st.do_color
            ));
        }
        vehicles += got.len();
        got.push(format!("R {}", rand::next(1_000_000_000)));
        let exp: Vec<&String> = expect.iter().filter(|l| !l.starts_with("E ")).collect();
        if exp.len() == got.len() && exp.iter().zip(&got).all(|(a, b)| *a == b) {
            ok += 1;
        } else {
            bad += 1;
            if bad <= 5 {
                println!("{header}");
                for l in &exp {
                    println!("  J {l}");
                }
                for l in &got {
                    println!("  R {l}");
                }
            }
        }
    }
    println!(
        "spawn cases={} ok={ok} bad={bad} vehicles={vehicles}",
        java.len()
    );
}

/// The harness's spawned-vehicle line.
fn spawn_line(v: &pz_vehicles::VehicleRef) -> String {
    let st = v.st();
    let q = st.saved_rot;
    let o = st.jni_transform.origin;
    format!(
        "V {} skin={} type={} zone={} pos={},{},{} rot={},{},{},{} o={},{},{} rust={} alarm={} key={} color={}",
        st.script_name,
        st.skin_index,
        st.vehicle_type,
        st.respawn_zone,
        fh(st.x),
        fh(st.y),
        fh(st.z),
        fh(q.x),
        fh(q.y),
        fh(q.z),
        fh(q.w),
        fh(o.x),
        fh(o.y),
        fh(o.z),
        fh(st.rust),
        st.alarmed,
        st.key_id,
        st.do_color
    )
}

/// The synthetic chunk map with one meta cell holding the case's zones.
struct DispatchWorld {
    zones: Vec<pz_vehicles::spawn::VehicleZone>,
}

impl pz_vehicles::spawn::SpawnWorld for DispatchWorld {
    fn square_exists(&self, x: i32, y: i32, z: i32) -> bool {
        SpawnTestWorld.square_exists(x, y, z)
    }
    fn is_outside(&self, x: i32, y: i32, z: i32) -> bool {
        SpawnTestWorld.is_outside(x, y, z)
    }
    /// `Zone.contains` for rectangles; the harness's polylines have no
    /// width, so they contain nothing.
    fn vehicle_zone_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.zones.iter().any(|zn| {
            z == zn.z
                && x >= zn.x
                && x < zn.x + zn.w
                && y >= zn.y
                && y < zn.y + zn.h
                && !zn.is_polyline()
        })
    }
    fn vehicle_zones(&self, _cx: i32, _cy: i32) -> Vec<pz_vehicles::spawn::VehicleZone> {
        self.zones.clone()
    }
    fn chunk_vehicles(&self, _wx: i32, _wy: i32) -> Option<Vec<pz_vehicles::VehicleRef>> {
        Some(Vec::new())
    }
}

/// `--dispatch`: `IsoChunk.AddVehicles` end to end on `dispatch.txt`'s
/// zone lists.
fn run_dispatch(oracle: &std::path::Path, sandbox_lines: &[String]) {
    use pz_vehicles::spawn::{self, ChunkSpawner, VehicleZone};
    world_sim::set_created(0.0, 0.0);
    let options = spawn_options(sandbox_lines);
    let text = std::fs::read_to_string(oracle.join("dispatch.txt")).unwrap();
    let mut lines = text.lines();
    let head: Vec<i32> = lines
        .next()
        .unwrap()
        .split(' ')
        .skip(1)
        .map(|v| v.parse().unwrap())
        .collect();
    let (wx, wy) = (head[0], head[1]);
    struct Case {
        id: i64,
        zones: Vec<VehicleZone>,
        expect: Vec<String>,
    }
    let mut cases: Vec<Case> = Vec::new();
    for l in lines {
        let t: Vec<&str> = l.split(' ').collect();
        if t[0] == "C" {
            cases.push(Case {
                id: t[1].parse().unwrap(),
                zones: Vec::new(),
                expect: Vec::new(),
            });
        } else if t[0] == "Z" {
            let n = |k: usize| t[k].parse::<i32>().unwrap();
            let poly = t[8] != "-";
            cases.last_mut().unwrap().zones.push(VehicleZone {
                name: if t[1] == "-" {
                    String::new()
                } else {
                    t[1].to_string()
                },
                zone_type: "parkingstall".to_string(),
                x: n(2),
                y: n(3),
                z: 0,
                w: n(4),
                h: n(5),
                dir: pz_world::IsoDirections::from_name(t[6]),
                face_direction: t[7] == "true",
                geometry_type: if poly {
                    pz_world::ZoneGeometryType::Polyline
                } else {
                    pz_world::ZoneGeometryType::Invalid
                },
                points: if poly {
                    t[8].split(',').map(|v| v.parse().unwrap()).collect()
                } else {
                    Vec::new()
                },
                polyline_width: 0,
            });
        } else if t[0] != "E" {
            cases.last_mut().unwrap().expect.push(l.to_string());
        }
    }
    let (mut ok, mut bad, mut vehicles) = (0, 0, 0);
    for c in &cases {
        let world = DispatchWorld {
            zones: c.zones.clone(),
        };
        spawn::reset_checked_vehicles();
        rand::seed_standard(9000 + c.id);
        let mut sp = ChunkSpawner::new(wx, wy, &world, options.clone());
        sp.add_vehicles();
        let mut got: Vec<String> = sp.vehicles.iter().map(spawn_line).collect();
        vehicles += got.len();
        got.push(format!("R {}", rand::next(1_000_000_000)));
        if got == c.expect {
            ok += 1;
        } else {
            bad += 1;
            if bad <= 4 {
                println!(
                    "case {}: {:?}",
                    c.id,
                    c.zones.iter().map(|z| &z.name).collect::<Vec<_>>()
                );
                for l in &c.expect {
                    println!("  J {l}");
                }
                for l in &got {
                    println!("  R {l}");
                }
            }
        }
    }
    println!(
        "dispatch cases={} ok={ok} bad={bad} vehicles={vehicles}",
        cases.len()
    );
}

/// `--surround`: `SurroundVehicle` over the harness's call sequences
/// (`surround.txt`): zombie, target and vehicle moves, clock steps,
/// `update()` and `getPositionForZombie` in the recorded order.
fn run_surround(oracle: &std::path::Path) {
    use pz_vehicles::surround::{SurroundTarget, SurroundZombie};
    let text = std::fs::read_to_string(oracle.join("surround.txt")).unwrap();
    let hx = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
    #[derive(Clone, Copy)]
    struct Z {
        x: f32,
        y: f32,
        z: f32,
        on_floor: bool,
        can_walk: bool,
        dead: bool,
        has_target: bool,
    }
    let mut v: Option<pz_vehicles::VehicleRef> = None;
    let mut zs: std::collections::HashMap<u64, Z> = Default::default();
    let mut target = (0.0f32, 0.0f32, false);
    let (mut ok, mut bad, mut cases) = (0, 0, 0);
    let mut case_bad = false;
    let place = |v: &BaseVehicle, x: f32, y: f32, a: f32| {
        let (ox, oy) = env::offset();
        v.set_position(x, y, 0.0);
        let q = pz_vehicles::jmath::Quaternionf::angle_axis(a, 0.0, 1.0, 0.0);
        let mut st = v.st_mut();
        st.saved_rot = q;
        st.jni_transform.set_rotation(&q);
        st.jni_transform.origin = pz_vehicles::jmath::Vector3f::new(x - ox, 0.0, y - oy);
        st.poly_dirty = true;
    };
    world_sim::set_created(0.0, 0.0);
    for line in text.lines() {
        let t: Vec<&str> = line.split(' ').collect();
        let snapshot = |key: u64, z: &Z, target: (f32, f32, bool)| SurroundZombie {
            key,
            x: z.x,
            y: z.y,
            z: z.z,
            on_floor: z.on_floor,
            can_walk: z.can_walk,
            dead: z.dead,
            reused: false,
            target: z.has_target.then_some(SurroundTarget {
                x: target.0,
                y: target.1,
                seat: if target.2 { 0 } else { -1 },
            }),
        };
        match t[0] {
            "C" => {
                if case_bad {
                    bad += 1;
                } else if cases > 0 {
                    ok += 1;
                }
                case_bad = false;
                cases += 1;
                manager::reset();
                let nv = BaseVehicle::new();
                nv.set_script_name(t[2]);
                nv.set_script(t[2]);
                place(&nv, hx(t[3]), hx(t[4]), hx(t[5]));
                zs.clear();
                v = Some(nv);
            }
            "T" => {
                target = (hx(t[1]), hx(t[2]), t[3] == "true");
                let vv = v.as_ref().unwrap();
                if target.2 && vv.character(0).is_none() {
                    vv.set_passenger(0, Some(Rc::new(Driver)), Default::default());
                }
            }
            "Z" => {
                zs.insert(
                    t[1].parse().unwrap(),
                    Z {
                        x: hx(t[2]),
                        y: hx(t[3]),
                        z: hx(t[4]),
                        on_floor: t[5] == "true",
                        can_walk: t[6] == "true",
                        dead: t[7] == "true",
                        has_target: t[8] == "true",
                    },
                );
            }
            "M" => place(v.as_ref().unwrap(), hx(t[1]), hx(t[2]), hx(t[3])),
            "K" => env::set_now_ms(t[1].parse().unwrap()),
            "U" => {
                let vv = v.as_ref().unwrap();
                let lookup = |k: u64| zs.get(&k).map(|z| snapshot(k, z, target));
                let mut s = std::mem::take(&mut vv.st_mut().surround);
                s.update(vv, &lookup);
                vv.st_mut().surround = s;
            }
            "P" => {
                let vv = v.as_ref().unwrap();
                let k: u64 = t[1].parse().unwrap();
                let lookup = |k: u64| zs.get(&k).map(|z| snapshot(k, z, target));
                let me = snapshot(k, &zs[&k], target);
                let r = vv.surround_position_for_zombie(&me, &lookup);
                let got = match r {
                    Some((x, y)) => format!("P {k} {},{}", fh(x), fh(y)),
                    None => format!("P {k} null"),
                };
                if got != line {
                    if !case_bad && bad < 5 {
                        println!("case {}: J {line} R {got}", cases - 1);
                    }
                    case_bad = true;
                }
            }
            "S" => {
                let vv = v.as_ref().unwrap();
                let st = vv.st();
                let mut out = format!("S n={}", st.surround.positions.len());
                for p in &st.surround.positions {
                    write!(
                        out,
                        " {},{}{}{}",
                        fh(p.pos_world.x),
                        fh(p.pos_world.y),
                        if p.blocked { "b" } else { "" },
                        p.zombie.map(|z| format!(":{z}")).unwrap_or_default()
                    )
                    .unwrap();
                }
                if out != line {
                    if !case_bad && bad < 5 {
                        println!("case {}: J {line}\n         R {out}", cases - 1);
                    }
                    case_bad = true;
                }
            }
            _ => {}
        }
    }
    if case_bad {
        bad += 1;
    } else if cases > 0 {
        ok += 1;
    }
    println!("surround cases={cases} ok={ok} bad={bad}");
}
