//! Replays the pure path-following helpers of `PathFindBehavior2` against
//! the original: `closestPointOnPath`, `advanceAlongPath` and
//! `getPathLength`.
//!
//! ```text
//! behavior_replay <oracle dir> <game dir> [region...]
//! ```
//!
//! `<oracle dir>` holds `regions/<name>/squares.tsv` (see
//! `square_bits_replay`) and `lcc/<name>/pfb.txt`, made by a local harness
//! on a machine with the game installed: the real map chunks loaded in a
//! headless game JVM (as for `line_clear_replay`), then 6000 seeded random
//! paths per region (2 to 7 nodes a few squares apart on a level that has
//! squares, some repeated nodes, some crawl flags and fractional levels) with
//! a query point near the path, passed to the original static
//! `closestPointOnPath` (null mover: the plain collide matrix between
//! adjacent squares), `advanceAlongPath` on a behaviour with a null
//! character, and `getPathLength` on a behaviour whose character is an
//! `IsoZombie` with only its position set. Nothing here is game data;
//! results are recorded as notes only.
//!
//! Chunks are loaded in the game's load order (each chunk loaded,
//! recalculated and passed through `loadInMainThread` before the next one
//! enters the cell). Cases whose path or query point is within 2 squares of
//! a square whose inputs differ from the game's (worldgen) are skipped for
//! the two square-reading helpers (`getPathLength` reads no squares). The
//! harness also writes every square's `collideMatrix`; the replay reports
//! how many differ (none in `westpoint` and `louisville`; elsewhere only
//! next to worldgen differences) but does not skip on them.
//!
//! Results on build 42 (30000 cases in 5 regions):
//!
//! | helper | compared | match |
//! |---|---|---|
//! | `closestPointOnPath` | 14939 | 14939 (index, `dist`, `x`, `y` bits) |
//! | `advanceAlongPath` | 14939 | 14939 |
//! | `getPathLength` | 30000 | 30000 |

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use pz_classfile::{Jar, Value};
use pz_fs::GameFs;
use pz_pathfind::behavior::{advance_along_path, closest_point_on_path, path_length, PointOnPath};
use pz_pathfind::game_vehicle::NoVehicles;
use pz_pathfind::main_thread::MainThreadMap;
use pz_pathfind::path::Path as PfPath;
use pz_pathfind::square_bits::{DefaultBitsHooks, SquareBits};
use pz_world::chunk::{ChunkLoadEnv, IsoCell};
use pz_world::fix2x::Fix2xTable;
use pz_world::{
    CreateParams, GameRand, IsoGridSquare, IsoMetaGrid, IsoSpriteManager, NoHooks,
    ObjectLoadSettings, ObjectTables, SpriteTables, WorldGenBounds,
};

/// `java.util.Random`.
struct JavaRandom(u64);

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom((seed as u64 ^ 0x5DEECE66D) & ((1 << 48) - 1))
    }
    fn bits(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - bits)) as i64 as i32
    }
}

impl GameRand for JavaRandom {
    fn next(&mut self, bound: i32) -> i32 {
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
    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 / (1 << 24) as f32
    }
}

/// The enum tables, read from the game's own classes.
fn sprite_tables(jar: &Jar) -> SpriteTables {
    let names = |class: &str, drop_max: bool| {
        let mut v = pz_classfile::enum_constants(jar, class).expect(class);
        if drop_max && v.last().map(String::as_str) == Some("MAX") {
            v.pop();
        }
        v
    };
    SpriteTables::new(
        names("zombie/iso/SpriteDetails/IsoFlagType", true),
        names("zombie/iso/SpriteDetails/IsoObjectType", true),
        names("zombie/core/properties/FasciaEdge", false),
    )
    .expect("sprite tables")
}

/// `IsoTree.LOGS_PER_SIZE` and `IsoWindow.LockedHouseFrequency`, read from
/// the game's own classes.
fn object_tables(jar: &Jar) -> ObjectTables {
    let int = |v: &Value| match v {
        Value::Int(i) => *i,
        other => panic!("expected an int, got {other:?}"),
    };
    let logs = match pz_classfile::static_value(jar, "zombie/iso/objects/IsoTree", "LOGS_PER_SIZE")
        .expect("IsoTree")
    {
        Some(Value::Array(a)) => a.iter().map(int).collect::<Vec<_>>(),
        other => panic!("LOGS_PER_SIZE: {other:?}"),
    };
    let freq = pz_classfile::self_constructed_constants(
        jar,
        "zombie/iso/objects/IsoWindow$LockedHouseFrequency",
    )
    .expect("LockedHouseFrequency");
    let mut text = logs
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",");
    text.push('\n');
    let pairs: Vec<String> = freq
        .iter()
        .map(|(_, args)| {
            // name, ordinal, value, lockChance
            format!("{}:{}", int(&args[2]), int(&args[3]))
        })
        .collect();
    text.push_str(&pairs.join(","));
    ObjectTables::from_text(&text).expect("object tables")
}

/// The oracle's input columns for one square.
#[derive(Clone, Debug, PartialEq)]
struct Inputs {
    null: bool,
    flags: String,
    solid_etc: String,
    room: String,
    tree_bush: String,
    objects: String,
}

fn oracle_inputs(path: &Path) -> HashMap<(i32, i32, i32), Inputs> {
    let text = std::fs::read_to_string(path).expect("squares.tsv");
    let mut out = HashMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let c: Vec<&str> = line.split('\t').collect();
        let key = (
            c[0].parse().unwrap(),
            c[1].parse().unwrap(),
            c[2].parse().unwrap(),
        );
        let get = |i: usize| c.get(i).copied().unwrap_or("").to_string();
        out.insert(
            key,
            Inputs {
                null: c[8] == "1",
                flags: get(9),
                solid_etc: format!("{} {} {} {}", get(10), get(11), get(12), get(13)),
                room: format!("{} {}", get(14), get(15)),
                tree_bush: format!("{} {}", get(16), get(17)),
                objects: get(20),
            },
        );
    }
    out
}

const FLAGS: [&str; 24] = [
    "solidfloor",
    "solid",
    "solidtrans",
    "collideW",
    "collideN",
    "HoppableN",
    "HoppableW",
    "TallHoppableN",
    "TallHoppableW",
    "WallN",
    "WallW",
    "WallNTrans",
    "WallWTrans",
    "windowW",
    "windowN",
    "WindowW",
    "WindowN",
    "canPathW",
    "canPathN",
    "DoorWallW",
    "DoorWallN",
    "water",
    "CantClimb",
    "exterior",
];
const OTYPES: [&str; 6] = [
    "stairsTW", "stairsMW", "stairsBW", "stairsTN", "stairsMN", "stairsBN",
];

fn rust_inputs(
    sq: Option<&IsoGridSquare>,
    sb: &SquareBits<'_, IsoCell>,
    m: &IsoSpriteManager,
) -> Inputs {
    let Some(sq) = sq else {
        return Inputs {
            null: true,
            flags: String::new(),
            solid_etc: "   ".into(),
            room: " ".into(),
            tree_bush: " ".into(),
            objects: String::new(),
        };
    };
    let t = &m.tables;
    let mut fl: Vec<&str> = FLAGS
        .iter()
        .filter(|n| sq.has(t.flag_from_string(n).unwrap()))
        .copied()
        .collect();
    fl.extend(
        OTYPES
            .iter()
            .filter(|n| sq.has_type(t.object_type_from_string(n)))
            .copied(),
    );
    let b = |v: bool| v as u8;
    Inputs {
        null: false,
        flags: fl.join(","),
        solid_etc: format!(
            "{} {} {} {}",
            b(sq.has(t.flag.solid)),
            b(sq.has(t.flag.solidtrans)),
            b(sq.treat_as_solid_floor(m)),
            b(sq.have_roof)
        ),
        room: format!("{} {}", sq.room_id, b(sb.has_building(sq))),
        tree_bush: format!("{} {}", b(sq.has_tree), b(sb.has_bush(sq))),
        objects: sq
            .objects
            .iter()
            .map(|o| {
                format!(
                    "{}:{}",
                    o.kind.class_name(),
                    o.sprite.map_or("null", |s| m.sprites[s].name())
                )
            })
            .collect::<Vec<_>>()
            .join(";"),
    }
}

fn h(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

/// Squares whose `getBits` inputs differ between the game and the port.
fn differing_squares(
    inputs: &HashMap<(i32, i32, i32), Inputs>,
    cell: &IsoCell,
    sb: &SquareBits<'_, IsoCell>,
    m: &IsoSpriteManager,
    bounds: (i32, i32, i32, i32),
) -> HashSet<(i32, i32, i32)> {
    let mut bad = HashSet::new();
    for (k, gi) in inputs {
        let ri = rust_inputs(cell.get_grid_square(k.0, k.1, k.2), sb, m);
        // Room ids do not matter here (the exterior flag is compared).
        let ri = Inputs {
            room: gi.room.clone(),
            ..ri
        };
        if *gi != ri {
            if std::env::var_os("REPLAY_DEBUG").is_some() {
                let what = if gi.null != ri.null {
                    format!("null game={} port={}", gi.null, ri.null)
                } else if gi.objects != ri.objects {
                    "objects".to_string()
                } else {
                    format!("other {gi:?} {ri:?}")
                };
                eprintln!("bad {k:?} {what}");
            }
            bad.insert(*k);
        }
    }
    // Squares only the port has (levels outside the game's chunk range).
    for x in bounds.0..bounds.2 {
        for y in bounds.1..bounds.3 {
            for z in -32..32 {
                if !inputs.contains_key(&(x, y, z)) && cell.get_grid_square(x, y, z).is_some() {
                    if std::env::var_os("REPLAY_DEBUG").is_some() {
                        eprintln!("bad {:?} port-only", (x, y, z));
                    }
                    bad.insert((x, y, z));
                }
            }
        }
    }
    bad
}

fn touches(
    bad: &HashSet<(i32, i32, i32)>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    z0: i32,
    z1: i32,
) -> bool {
    let (lx, hx) = (x0.min(x1).floor() as i32 - 2, x0.max(x1).floor() as i32 + 2);
    let (ly, hy) = (y0.min(y1).floor() as i32 - 2, y0.max(y1).floor() as i32 + 2);
    for z in z0.min(z1) - 1..=z0.max(z1) + 1 {
        for x in lx..=hx {
            for y in ly..=hy {
                if bad.contains(&(x, y, z)) {
                    return true;
                }
            }
        }
    }
    false
}

#[derive(Default)]
struct Tally {
    total: usize,
    skipped: usize,
    matched: usize,
    mismatched: usize,
    examples: Vec<String>,
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let oracle = PathBuf::from(&a[1]);
    let game = PathBuf::from(&a[2]);
    let only: Vec<&str> = a[3..].iter().map(String::as_str).collect();
    let jar = Jar::open(game.join("projectzomboid.jar")).expect("jar");
    let tables = sprite_tables(&jar);
    let objects = object_tables(&jar);
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| true).unwrap();
    let meta = IsoMetaGrid::create(
        &GameFs::new(&game),
        CreateParams {
            game_map: "Muldraugh, KY",
            world_gen: WorldGenBounds {
                min_x: -250,
                min_y: -250,
                max_x: 250,
                max_y: 250,
            },
            rand: &mut JavaRandom::new(42),
            voronois: Vec::new(),
        },
    )
    .expect("meta grid");
    let fix2x = Fix2xTable::default();
    let regions = [
        ("muldraugh", 1346, 1201, 12, 12),
        ("westpoint", 1485, 857, 14, 12),
        ("riverside", 777, 648, 17, 8),
        ("louisville", 1574, 245, 10, 10),
        ("forest", 1300, 1200, 8, 8),
    ];
    let mut grand: BTreeMap<&str, Tally> = BTreeMap::new();
    for (region, rx, ry, rw, rh) in regions {
        if !only.is_empty() && !only.contains(&region) {
            continue;
        }
        let inputs = oracle_inputs(&oracle.join("regions").join(region).join("squares.tsv"));
        // The game's load order: each chunk is loaded, recalculated and
        // passed through `loadInMainThread` before the next one enters the
        // cell (`IsoChunkMap.processAllLoadGridSquare`).
        let mut cell = IsoCell::default();
        for wy in ry - 1..ry + rh + 1 {
            for wx in rx - 1..rx + rw + 1 {
                let mut rand = JavaRandom::new(((wx as i64) << 32) ^ (wy as i64 & 0xffff_ffff));
                let mut hooks = NoHooks;
                let mut env = ChunkLoadEnv {
                    sprites: &mut sprites,
                    fix2x: &fix2x,
                    rand: &mut rand,
                    tables: &objects,
                    settings: ObjectLoadSettings::default(),
                    hooks: &mut hooks,
                };
                cell.load_chunk(&meta, wx, wy, &mut env);
                cell.recalc_chunk_squares(wx, wy, &meta, &sprites);
                cell.load_in_main_thread(wx, wy, &meta, &sprites);
            }
        }
        let sb = SquareBits::new(&sprites, &meta, &cell, &DefaultBitsHooks).unwrap();
        let bad = differing_squares(
            &inputs,
            &cell,
            &sb,
            &sprites,
            (rx * 8, ry * 8, (rx + rw) * 8, (ry + rh) * 8),
        );
        let map = MainThreadMap::new(&sb, &NoVehicles);
        // The collide matrices the game computed, compared for the record
        // (closestPointOnPath reads them; they are not used for skipping).
        let mut matrix_diffs = 0;
        let mut matrix_diffs_same_inputs = 0;
        let collide = std::fs::read_to_string(oracle.join("lcc").join(region).join("collide.txt"))
            .expect("collide.txt");
        for line in collide.lines() {
            let c: Vec<i32> = line.split(' ').map(|v| v.parse().unwrap()).collect();
            let ours = cell
                .get_grid_square(c[0], c[1], c[2])
                .map(|s| s.collide_matrix);
            if ours != Some(c[3]) {
                matrix_diffs += 1;
                if !bad.contains(&(c[0], c[1], c[2])) {
                    matrix_diffs_same_inputs += 1;
                }
            }
        }
        println!(
            "== {region}: {matrix_diffs} squares with a different collide matrix ({matrix_diffs_same_inputs} of them with the same inputs)"
        );
        let text = std::fs::read_to_string(oracle.join("lcc").join(region).join("pfb.txt"))
            .expect("pfb.txt");
        let mut tally: BTreeMap<&str, Tally> = BTreeMap::new();
        for line in text.lines() {
            let parts: Vec<Vec<&str>> = line.split(" | ").map(|p| p.split(' ').collect()).collect();
            let nn: usize = parts[0][1].parse().unwrap();
            let mut path = PfPath::default();
            for k in 0..nn {
                let o = 2 + k * 4;
                let c = &parts[0];
                path.add_node(h(c[o]), h(c[o + 1]), h(c[o + 2]), c[o + 3].parse().unwrap());
            }
            let q = &parts[1];
            let (qx, qy, qz, ad) = (h(q[0]), h(q[1]), h(q[2]), h(q[3]));
            let pi: i32 = q[4].parse().unwrap();
            let (tx, ty) = (h(q[5]), h(q[6]));
            let (mut lx, mut ly, mut hx, mut hy) = (qx, qy, qx, qy);
            for n in &path.nodes {
                lx = lx.min(n.x);
                ly = ly.min(n.y);
                hx = hx.max(n.x);
                hy = hy.max(n.y);
            }
            let skip = touches(&bad, lx, ly, hx, hy, qz.floor() as i32, qz.floor() as i32);
            let fmt = |p: &PointOnPath| {
                format!(
                    "{} {:08x} {:08x} {:08x}",
                    p.path_index,
                    p.dist.to_bits(),
                    p.x.to_bits(),
                    p.y.to_bits()
                )
            };
            let mut pop = PointOnPath::default();
            closest_point_on_path(&map, qx, qy, qz, None, &path, &mut pop);
            let mut pop2 = PointOnPath::default();
            advance_along_path(&map, qx, qy, qz, ad, None, &path, &mut pop2);
            let len = path_length(&path, pi, (tx, ty), qx, qy);
            let checks = [
                ("closestPointOnPath", fmt(&pop), parts[2].join(" "), skip),
                ("advanceAlongPath", fmt(&pop2), parts[3].join(" "), skip),
                (
                    "getPathLength",
                    format!("{:08x}", len.to_bits()),
                    parts[4].join(" "),
                    false,
                ),
            ];
            for (kind, got, want, skip) in checks {
                let t = tally.entry(kind).or_default();
                t.total += 1;
                if skip {
                    t.skipped += 1;
                } else if got == want {
                    t.matched += 1;
                } else {
                    t.mismatched += 1;
                    if t.examples.len() < 5 {
                        t.examples.push(format!("{line}  -> port {got}"));
                    }
                }
            }
        }
        for (k, t) in &tally {
            println!(
                "  {k}: {} cases, {} skipped, {} compared, {} match, {} differ",
                t.total,
                t.skipped,
                t.matched + t.mismatched,
                t.matched,
                t.mismatched
            );
            for e in &t.examples {
                println!("      {e}");
            }
            let g = grand.entry(k).or_default();
            g.total += t.total;
            g.skipped += t.skipped;
            g.matched += t.matched;
            g.mismatched += t.mismatched;
        }
    }
    println!("== total");
    for (k, t) in &grand {
        println!(
            "  {k}: {} cases, {} skipped, {} compared, {} match, {} differ",
            t.total,
            t.skipped,
            t.matched + t.mismatched,
            t.matched,
            t.mismatched
        );
    }
}
