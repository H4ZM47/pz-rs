//! Replays the game's native-pathfinder chunk buffers against this port.
//!
//! ```text
//! square_bits_replay <oracle dir> <game dir> [region...]
//! ```
//!
//! `<game dir>` holds `projectzomboid.jar` and `media/`; `<oracle dir>` holds
//! `regions/<name>/chunks.bin` and `squares.tsv`, made by a local harness on
//! a machine with the game installed (the exact `ChunkUpdateTask.init`
//! buffers and per-square inputs of real map chunks, loaded fresh from the
//! `Muldraugh, KY` lotpacks with a one-chunk margin, `Rand` reseeded per
//! chunk). Nothing here is game data; results are recorded as notes only.
//!
//! The replay loads the same chunks (region plus margin, row-major) with
//! `pz-world`'s lotpack loader in the game's load order: each chunk is
//! loaded, recalculated (`recalc_chunk_squares`) and passed through
//! `loadInMainThread` before the next one enters the cell, as
//! `IsoChunkMap.processAllLoadGridSquare` adds chunks one at a time. It then
//! builds every
//! chunk buffer with [`SquareBits::chunk_update_buffer`] and compares byte
//! for byte, then square by square with per-bit counts. Each mismatching
//! square is classified by comparing the inputs `getBits` reads (flags,
//! roof, room, building, tree/bush, objects) with the oracle's.
//!
//! Results on build 42 (612 chunks, 101056 squares; unchanged by the
//! load order, which only moves collide matrices):
//!
//! | region | squares | bit-exact | buffers identical |
//! |---|---|---|---|
//! | muldraugh | 10752 | 9997 | 90 / 144 |
//! | westpoint | 30016 | 30016 | 168 / 168 |
//! | riverside | 17536 | 17463 | 112 / 136 |
//! | louisville | 38592 | 38592 | 100 / 100 |
//! | forest | 4160 | 3858 | 42 / 64 |
//!
//! No square whose inputs (and neighbours' inputs) match the game's differs.
//! Every remaining difference comes from world generation
//! (`WorldGenChunk.generateChunks` in `IsoChunk.LoadBrandNew`), which
//! `pz-world` does not run yet: the game replaces the lotpacks'
//! `vegetation_trees_01` placeholders and grass overlays with biome
//! vegetation (jumbo trees, bushes, grass, boulders). All 1130 differing
//! squares (750 + 5 through a neighbour in muldraugh, 73 in riverside, 302 in
//! forest) have different objects there; the bits are `SOLID` (a tree or
//! boulder), `SOLID_TRANS` (boulders), the tall-hoppable bits (a solid square
//! beside a wall) and the cost (trees and bushes).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use pz_classfile::Jar;
use pz_fs::GameFs;
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

struct Record {
    wx: i32,
    wy: i32,
    data: Vec<u8>,
}

fn read_chunks(path: &Path) -> Vec<Record> {
    let b = std::fs::read(path).expect("chunks.bin");
    let mut out = Vec::new();
    let mut i = 0;
    let le = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    while i + 16 <= b.len() {
        let (wx, wy, len) = (le(i + 4), le(i + 8), le(i + 12) as usize);
        out.push(Record {
            wx,
            wy,
            data: b[i + 16..i + 16 + len].to_vec(),
        });
        i += 16 + len;
    }
    out
}

/// A chunk buffer split into squares `(x, y, z) -> (bits, cost)` and slope
/// records.
struct Parsed {
    levels: (i32, i32),
    squares: HashMap<(i32, i32, i32), (u64, i16)>,
    slopes: Vec<[u8; 12]>,
}

fn parse(b: &[u8]) -> Parsed {
    let be32 = |o: usize| i32::from_be_bytes(b[o..o + 4].try_into().unwrap());
    let (min, max) = (be32(0) - 32, be32(4) - 32);
    let mut squares = HashMap::new();
    let mut o = 8;
    for z in min..=max {
        for y in 0..8 {
            for x in 0..8 {
                let bits = u64::from_be_bytes(b[o..o + 8].try_into().unwrap());
                let cost = i16::from_be_bytes(b[o + 8..o + 10].try_into().unwrap());
                squares.insert((x, y, z), (bits, cost));
                o += 10;
            }
        }
    }
    let n = i16::from_be_bytes(b[o..o + 2].try_into().unwrap());
    o += 2;
    let slopes = (0..n)
        .map(|i| {
            b[o + 12 * i as usize..o + 12 * (i as usize + 1)]
                .try_into()
                .unwrap()
        })
        .collect();
    Parsed {
        levels: (min, max),
        squares,
        slopes,
    }
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

/// The oracle squares' sprite names that the port's square lacks.
fn missing_sprites(game: &str, rust: &str) -> Vec<String> {
    let mut have: Vec<&str> = rust.split(';').filter(|s| !s.is_empty()).collect();
    let mut out = Vec::new();
    for o in game.split(';').filter(|s| !s.is_empty()) {
        if let Some(i) = have.iter().position(|h| *h == o) {
            have.remove(i);
        } else {
            out.push(o.to_string());
        }
    }
    out
}

/// The tileset of a `Class:sprite` entry (the sprite name without its
/// index).
fn tileset(s: &str) -> String {
    let name = s.split_once(':').map_or(s, |x| x.1);
    name.rsplit_once('_').map_or(name, |x| x.0).to_string()
}

/// A cause label for a square whose result differs, and the tilesets of
/// the objects only the game (`+`) or only the port (`-`) has.
fn cause(game: &Inputs, rust: &Inputs) -> (String, Vec<String>) {
    if game.null != rust.null {
        return ("square exists on one side only".into(), Vec::new());
    }
    if game.objects != rust.objects {
        let mut sets: Vec<String> = missing_sprites(&game.objects, &rust.objects)
            .iter()
            .map(|s| format!("+{}", tileset(s)))
            .collect();
        sets.extend(
            missing_sprites(&rust.objects, &game.objects)
                .iter()
                .map(|s| format!("-{}", tileset(s))),
        );
        sets.sort();
        sets.dedup();
        return ("objects differ".into(), sets);
    }
    let mut d = Vec::new();
    if game.flags != rust.flags {
        d.push("flags");
    }
    if game.solid_etc != rust.solid_etc {
        d.push("solid/floor/roof");
    }
    if game.room != rust.room {
        d.push("room/building");
    }
    if game.tree_bush != rust.tree_bush {
        d.push("tree/bush");
    }
    if d.is_empty() {
        ("inputs equal".into(), Vec::new())
    } else {
        (
            format!("derived state differs: {}", d.join(",")),
            Vec::new(),
        )
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let oracle = PathBuf::from(&a[1]);
    let game = PathBuf::from(&a[2]);
    let only: Vec<&str> = a[3..].iter().map(String::as_str).collect();
    let jar = Jar::open(game.join("projectzomboid.jar")).expect("jar");
    let tables = SpriteTables::from_jar(&jar).expect("sprite tables");
    let objects = ObjectTables::from_jar(&jar).expect("object tables");
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
    eprintln!(
        "meta grid {}..{} x {}..{}",
        meta.min_x, meta.max_x, meta.min_y, meta.max_y
    );
    let fix2x = Fix2xTable::default();
    let regions = [
        "muldraugh",
        "westpoint",
        "riverside",
        "louisville",
        "forest",
    ];
    let mut totals = (0usize, 0usize, 0usize, 0usize);
    for region in regions {
        if !only.is_empty() && !only.contains(&region) {
            continue;
        }
        let dir = oracle.join("regions").join(region);
        let records = read_chunks(&dir.join("chunks.bin"));
        let inputs = oracle_inputs(&dir.join("squares.tsv"));
        let (rx0, rx1) = (
            records.iter().map(|r| r.wx).min().unwrap(),
            records.iter().map(|r| r.wx).max().unwrap(),
        );
        let (ry0, ry1) = (
            records.iter().map(|r| r.wy).min().unwrap(),
            records.iter().map(|r| r.wy).max().unwrap(),
        );
        let mut cell = IsoCell::default();
        // Load order (`IsoChunkMap.processAllLoadGridSquare`): each chunk is
        // loaded, recalculated and passed through `loadInMainThread` before
        // the next one enters the cell, so the cross-chunk passes only see
        // the chunks loaded before it.
        for wy in ry0 - 1..=ry1 + 1 {
            for wx in rx0 - 1..=rx1 + 1 {
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
        let (mut n_sq, mut n_exact, mut n_buf, mut n_content) = (0, 0, 0, 0);
        let mut tilesets: BTreeMap<String, usize> = BTreeMap::new();
        let mut bit_diff: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
        let mut cost_diff = 0;
        let mut causes: BTreeMap<String, usize> = BTreeMap::new();
        let mut examples: BTreeMap<String, String> = BTreeMap::new();
        let mut level_diffs = Vec::new();
        let mut slope_diffs = 0;
        for r in &records {
            let ch = &cell.chunks[&(r.wx, r.wy)];
            let ours = sb.chunk_update_buffer(ch);
            if ours == r.data {
                n_buf += 1;
            }
            let (g, o) = (parse(&r.data), parse(&ours));
            if g.levels != o.levels {
                level_diffs.push(format!(
                    "{},{} game {:?} port {:?}",
                    r.wx, r.wy, g.levels, o.levels
                ));
            }
            if g.slopes != o.slopes {
                slope_diffs += 1;
            }
            // Same squares (levels only one side has are all zeros) and
            // slopes: the buffers differ at most in the level range.
            let same = |a: &Parsed, b: &Parsed| {
                a.squares
                    .iter()
                    .all(|(k, v)| b.squares.get(k).copied().unwrap_or((0, 0)) == *v)
            };
            if g.slopes == o.slopes && same(&g, &o) && same(&o, &g) {
                n_content += 1;
            }
            let mut keys: Vec<_> = g.squares.keys().copied().collect();
            keys.sort();
            for k in keys {
                n_sq += 1;
                let (gb, gc) = g.squares[&k];
                let (ob, oc) = o.squares.get(&k).copied().unwrap_or((0, 0));
                if (gb, gc) == (ob, oc) {
                    n_exact += 1;
                    continue;
                }
                let diff = gb ^ ob;
                for bit in 0..64 {
                    if diff >> bit & 1 != 0 {
                        let e = bit_diff.entry(bit).or_default();
                        if gb >> bit & 1 != 0 {
                            e.0 += 1;
                        } else {
                            e.1 += 1;
                        }
                    }
                }
                if gc != oc {
                    cost_diff += 1;
                }
                let (x, y, z) = (r.wx * 8 + k.0, r.wy * 8 + k.1, k.2);
                let gi = inputs.get(&(x, y, z)).cloned().expect("square in tsv");
                let ri = rust_inputs(cell.get_grid_square(x, y, z), &sb, &sprites);
                let (mut c, mut sets) = cause(&gi, &ri);
                if z < 0 {
                    c = format!("{c} (basement level)");
                }
                if c == "inputs equal" {
                    // Neighbour inputs (tall hoppable walls read both sides
                    // and the squares above).
                    let near = [(0, -1), (0, 1), (-1, 0), (1, 0)]
                        .iter()
                        .flat_map(|&(dx, dy)| [(x + dx, y + dy, z), (x + dx, y + dy, z + 1)])
                        .chain([(x, y, z + 1)])
                        .find_map(|(nx, ny, nz)| {
                            let gn = inputs.get(&(nx, ny, nz))?;
                            let rn = rust_inputs(cell.get_grid_square(nx, ny, nz), &sb, &sprites);
                            (*gn != rn).then(|| cause(gn, &rn))
                        });
                    if let Some((nc, ns)) = near {
                        c = format!("neighbour {nc}");
                        sets = ns;
                    }
                }
                for t in &sets {
                    *tilesets.entry(t.clone()).or_default() += 1;
                }
                let c = format!("{c} (bits {diff:#x})");
                *causes.entry(c.clone()).or_default() += 1;
                examples.entry(c).or_insert_with(|| {
                    format!(
                        "{x},{y},{z} game {gb:#x}/{gc} port {ob:#x}/{oc}\n      game {gi:?}\n      port {ri:?}"
                    )
                });
            }
        }
        println!(
            "== {region}: squares {n_sq}, bit-exact {n_exact}, buffers identical {n_buf}/{n}, identical but for the level range {}, cost diffs {cost_diff}, slope-record diffs {slope_diffs}",
            n_content - n_buf,
            n = records.len()
        );
        for l in &level_diffs {
            println!("  levels differ: {l}");
        }
        for (bit, (g, o)) in &bit_diff {
            println!(
                "  bit {bit:2} ({:#x}): game-only {g}, port-only {o}",
                1u64 << bit
            );
        }
        for (c, n) in &causes {
            println!("  {n:6}  {c}");
            if std::env::var_os("REPLAY_EXAMPLES").is_some() {
                println!("      e.g. {}", examples[c]);
            }
        }
        if !tilesets.is_empty() {
            let t: Vec<String> = tilesets.iter().map(|(k, v)| format!("{k} {v}")).collect();
            println!(
                "  tilesets of differing objects (squares): {}",
                t.join(", ")
            );
        }
        totals.0 += n_sq;
        totals.1 += n_exact;
        totals.2 += n_buf;
        totals.3 += records.len();
    }
    println!(
        "== total: squares {}, bit-exact {}, buffers identical {}/{}",
        totals.0, totals.1, totals.2, totals.3
    );
}
