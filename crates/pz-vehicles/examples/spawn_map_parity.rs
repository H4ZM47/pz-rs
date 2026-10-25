//! `IsoChunk.AddVehicles` on the real map's vehicle zones, compared with a
//! local harness that ran the original's `AddVehicles` on real chunks (its
//! data is not part of this repository).
//!
//! ```text
//! spawn_map_parity <game dir> <oracle dir>
//! ```
//!
//! The vehicle zones come from pz-world: the `Muldraugh, KY` `objects.lua`
//! registered into an `IsoMetaGrid`, read back with `cell_vehicle_zones` /
//! `vehicle_zone_at`, and compared first with the original's per-cell zone
//! lists (`vzones.txt`). The oracle dir also holds `sandbox.txt` and, per
//! region, `squares.txt` (level-0 squares of every loaded chunk: missing,
//! outside or inside) and `spawn.txt` (per chunk: the spawned vehicles and
//! the next `Rand` draw, `Rand` seeded per chunk). Chunks are populated in
//! the harness's order; vehicles of earlier chunks count for
//! `isGoodVehiclePos` and the checked-vehicles list lives across regions,
//! as in one game session. Vehicle stories are off on both sides.

use pz_kahlua::value::Value;
use pz_vehicles::rand;
use pz_vehicles::spawn::{ChunkSpawner, SpawnOptions, SpawnWorld, VehicleZone};
use pz_vehicles::VehicleRef;
use pz_world::zones::{PropValue, ZoneGeometryType, ZoneProps};
use pz_world::IsoMetaGrid;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

struct LuaProps(pz_kahlua::TableRef);

impl ZoneProps for LuaProps {
    fn rawget(&self, key: &str) -> PropValue {
        match self.0.get_str(key) {
            Value::Nil => PropValue::Nil,
            Value::Bool(v) => PropValue::Bool(v),
            Value::Num(d) => PropValue::Number(d),
            Value::Str(s) => PropValue::Str(s.to_string()),
            _ => PropValue::Other,
        }
    }
}

fn run_lua_file(path: &Path) -> pz_kahlua::Thread {
    let src = std::fs::read_to_string(path).unwrap();
    let mut t = pz_kahlua::Thread::new(pz_kahlua::TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let func = pz_kahlua::stdlib::load_string(&src, Some(&name), t.globals().clone()).unwrap();
    t.pcall(&func, &[]).map_err(|_| "lua error").unwrap();
    t
}

/// The map's vehicle zones as `IsoMetaGrid` registers them from
/// `objects.lua` (`Vehicle` / `ParkingStall` entries).
fn vehicle_grid(map_dir: &Path) -> IsoMetaGrid {
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
    g.zone_tables.iso_direction_names = ["N", "NW", "W", "SW", "S", "SE", "E", "NE"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let t = run_lua_file(&map_dir.join("objects.lua"));
    let Value::Table(objs) = t.globals().get_str("objects") else {
        panic!("no objects table");
    };
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
        if !matches!(v.get_str("geometry"), Value::Nil) {
            continue;
        }
        let props = match v.get_str("properties") {
            Value::Table(p) => Some(LuaProps(p)),
            _ => None,
        };
        let _ = g.register_vehicles_zone(
            &s("name"),
            &s("type"),
            n("x"),
            n("y"),
            n("z"),
            n("width"),
            n("height"),
            props.as_ref().map(|p| p as &dyn ZoneProps),
        );
    }
    g.check_vehicles_zones();
    g
}

/// The harness's zone line.
fn zline(z: &VehicleZone) -> String {
    let geom = match z.geometry_type {
        ZoneGeometryType::Invalid => "INVALID",
        ZoneGeometryType::Point => "Point",
        ZoneGeometryType::Polyline => "Polyline",
        ZoneGeometryType::Polygon => "Polygon",
    };
    let pts = if z.points.is_empty() {
        "-".to_string()
    } else {
        z.points
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "Z {}|{}|{}|{}|{}|{}|{}|{}|{}|{geom}|{pts}|{}",
        z.name,
        z.zone_type,
        z.x,
        z.y,
        z.z,
        z.w,
        z.h,
        z.dir.map_or("null".to_string(), |d| format!("{d:?}")),
        z.face_direction,
        z.polyline_width
    )
}

fn fh(v: f32) -> String {
    format!("{:x}", v.to_bits())
}

/// The harness's vehicle line.
fn vline(v: &VehicleRef) -> String {
    let st = v.st();
    let q = st.saved_rot;
    format!(
        "V {} skin={} type={} zone={} pos={},{},{} rot={},{},{},{} rust={} alarm={} key={} color={}",
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
        fh(st.rust),
        st.alarmed,
        st.key_id,
        st.do_color
    )
}

struct MapWorld<'a> {
    grid: &'a IsoMetaGrid,
    /// Level-0 squares of the loaded chunks: `o` outside, `i` inside.
    squares: HashMap<(i32, i32), u8>,
    loaded: std::collections::HashSet<(i32, i32)>,
    spawned: std::cell::RefCell<HashMap<(i32, i32), Vec<VehicleRef>>>,
}

impl SpawnWorld for MapWorld<'_> {
    fn square_exists(&self, x: i32, y: i32, z: i32) -> bool {
        z == 0 && self.squares.contains_key(&(x, y))
    }
    fn is_outside(&self, x: i32, y: i32, z: i32) -> bool {
        z == 0 && self.squares.get(&(x, y)) == Some(&b'o')
    }
    fn vehicle_zone_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.grid.vehicle_zone_at(x, y, z).is_some()
    }
    fn vehicle_zones(&self, cell_x: i32, cell_y: i32) -> Vec<VehicleZone> {
        self.grid.cell_vehicle_zones(cell_x, cell_y)
    }
    fn chunk_vehicles(&self, wx: i32, wy: i32) -> Option<Vec<VehicleRef>> {
        self.loaded.contains(&(wx, wy)).then(|| {
            self.spawned
                .borrow()
                .get(&(wx, wy))
                .cloned()
                .unwrap_or_default()
        })
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = Path::new(&a[1]);
    let oracle = Path::new(&a[2]);

    // Zones.
    let grid = vehicle_grid(&game.join("media/maps/Muldraugh, KY"));
    let java = std::fs::read_to_string(oracle.join("vzones.txt")).unwrap();
    let (mut cells, mut cells_same, mut zones_j, mut zones_r) = (0, 0, 0, 0);
    let mut shown = 0;
    let mut cur: Option<((i32, i32), Vec<String>)> = None;
    let mut check = |c: Option<((i32, i32), Vec<String>)>| {
        let Some(((cx, cy), jz)) = c else { return };
        let rz: Vec<String> = grid.cell_vehicle_zones(cx, cy).iter().map(zline).collect();
        cells += 1;
        zones_j += jz.len();
        zones_r += rz.len();
        if rz == jz {
            cells_same += 1;
        } else if shown < 3 {
            shown += 1;
            println!("cell {cx},{cy}: java {} zones, rust {}", jz.len(), rz.len());
            for (j, r) in jz.iter().zip(&rz) {
                if j != r {
                    println!("  J {j}\n  R {r}");
                    break;
                }
            }
        }
    };
    for l in java.lines() {
        if let Some(rest) = l.strip_prefix("cell ") {
            check(cur.take());
            let c = rest.split(' ').next().unwrap();
            let (x, y) = c.split_once(',').unwrap();
            cur = Some(((x.parse().unwrap(), y.parse().unwrap()), Vec::new()));
        } else if l.starts_with("Z ") {
            cur.as_mut().unwrap().1.push(l.to_string());
        }
    }
    check(cur.take());
    println!(
        "zones: cells={cells} identical={cells_same} java={zones_j} rust={zones_r} (rust total {})",
        grid.vehicles_zones.len()
    );

    // Scripts and the zone distribution.
    let sm = pz_scripts::ScriptManager::new();
    sm.load(&pz_fs::GameFs::new(game.to_str().unwrap()), &[])
        .unwrap();
    pz_vehicles::vehicle::set_script_manager(Rc::new(sm));
    let t = run_lua_file(&game.join("media/lua/shared/VehicleZoneDefinition.lua"));
    let Value::Table(dist) = t.globals().get_str("VehicleZoneDistribution") else {
        panic!("no VehicleZoneDistribution");
    };
    pz_vehicles::vehicle_type::init(&dist);
    pz_vehicles::world_sim::set_created(0.0, 0.0);

    let mut options = SpawnOptions::default();
    for l in std::fs::read_to_string(oracle.join("sandbox.txt"))
        .unwrap()
        .lines()
    {
        let (k, v) = l.split_once(' ').unwrap();
        match k {
            "CarSpawnRate" => options.car_spawn_rate = v.parse().unwrap(),
            "CarAlarm" => options.car_alarm = v.parse().unwrap(),
            "TrafficJam" => options.traffic_jam = v == "true",
            "EnableVehicles" => options.enable_vehicles = v == "true",
            _ => {}
        }
    }

    // Regions, in the harness's order.
    let mut regions: Vec<_> = std::fs::read_dir(oracle.join("regions"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    regions.sort_by_key(|p| {
        let log = std::fs::read_to_string(oracle.join("order.txt")).unwrap_or_default();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        log.lines().position(|l| l == n).unwrap_or(usize::MAX)
    });
    let (mut chunks, mut ok, mut vehicles) = (0, 0, 0);
    for region in &regions {
        let mut squares = HashMap::new();
        let mut loaded = std::collections::HashSet::new();
        for l in std::fs::read_to_string(region.join("squares.txt"))
            .unwrap()
            .lines()
        {
            let t: Vec<&str> = l.split(' ').collect();
            let (wx, wy): (i32, i32) = (t[0].parse().unwrap(), t[1].parse().unwrap());
            loaded.insert((wx, wy));
            for (i, c) in t[2].bytes().enumerate() {
                if c != b'-' {
                    let (x, y) = ((i % 8) as i32, (i / 8) as i32);
                    squares.insert((wx * 8 + x, wy * 8 + y), c);
                }
            }
        }
        let world = MapWorld {
            grid: &grid,
            squares,
            loaded,
            spawned: Default::default(),
        };
        let text = std::fs::read_to_string(region.join("spawn.txt")).unwrap();
        let mut blocks: Vec<((i32, i32), Vec<String>)> = Vec::new();
        for l in text.lines() {
            if let Some(rest) = l.strip_prefix("C ") {
                let (x, y) = rest.split_once(' ').unwrap();
                blocks.push(((x.parse().unwrap(), y.parse().unwrap()), Vec::new()));
            } else if let Some(b) = blocks.last_mut() {
                b.1.push(l.to_string());
            }
        }
        let (mut rok, mut rbad, mut rv) = (0, 0, 0);
        for ((wx, wy), expect) in &blocks {
            rand::seed_standard(0x5EED_i64 * 1_000_003 + ((*wx as i64) << 20) + *wy as i64);
            let mut sp = ChunkSpawner::new(*wx, *wy, &world, options.clone());
            sp.add_vehicles();
            let mut got: Vec<String> = sp.vehicles.iter().map(vline).collect();
            rv += got.len();
            world
                .spawned
                .borrow_mut()
                .insert((*wx, *wy), sp.vehicles.clone());
            got.push(format!("R {}", rand::next(1_000_000_000)));
            if &got == expect {
                rok += 1;
            } else {
                rbad += 1;
                if rbad <= 2 {
                    let mut s = format!("{} chunk {wx},{wy}:\n", region.display());
                    for l in expect {
                        writeln!(s, "  J {l}").unwrap();
                    }
                    for l in &got {
                        writeln!(s, "  R {l}").unwrap();
                    }
                    print!("{s}");
                }
            }
        }
        println!(
            "{}: chunks={} ok={rok} bad={rbad} vehicles={rv}",
            region.file_name().unwrap().to_string_lossy(),
            blocks.len()
        );
        chunks += blocks.len();
        ok += rok;
        vehicles += rv;
    }
    println!("spawn: chunks={chunks} ok={ok} vehicles={vehicles}");
}
