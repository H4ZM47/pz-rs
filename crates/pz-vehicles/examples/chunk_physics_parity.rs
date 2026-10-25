//! Compares [`pz_vehicles::chunk_physics::update_physics_for_level`] with
//! the `ToBullet` buffers the original's `IsoChunk.updatePhysicsForLevel`
//! produced for a region of real map chunks (a local harness; its data is
//! not part of this repository).
//!
//! ```text
//! chunk_physics_parity <game dir> <sprite tables> <object tables> <region dir>
//! ```
//!
//! The region dir holds `chunkinfo.txt` (`wx wy minLevel maxLevel` per
//! loaded chunk, in load order) and `tobullet.bin` (`wx, wy, level, n` as
//! little-endian ints, then the `n` buffer bytes, per record). Chunks load
//! in that order with `Rand` reseeded per chunk as the harness did.

use pz_fs::GameFs;
use pz_vehicles::chunk_physics::update_physics_for_level_in;
use pz_vehicles::rand::JavaRandom;
use pz_world::chunk::{ChunkLoadEnv, IsoCell};
use pz_world::fix2x::Fix2xTable;
use pz_world::{
    CreateParams, GameRand, IsoMetaGrid, IsoSpriteManager, NoHooks, ObjectLoadSettings,
    ObjectTables, SpriteTables, WorldGenBounds,
};

/// A `ToBullet` record: `(wx, wy, level)` and the buffer.
type Record = ((i32, i32, i32), Vec<u8>);

struct Rand(JavaRandom);
impl GameRand for Rand {
    fn next(&mut self, max: i32) -> i32 {
        if max <= 0 {
            0
        } else {
            self.0.next_int(max)
        }
    }
    fn next_float(&mut self) -> f32 {
        self.0.next_float()
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = std::path::Path::new(&a[1]);
    let region = std::path::Path::new(&a[4]);
    let grid = IsoMetaGrid::create(
        &GameFs::new(game.to_str().unwrap()),
        CreateParams {
            game_map: "Muldraugh, KY",
            world_gen: WorldGenBounds::default(),
            rand: &mut Rand(JavaRandom::new(42)),
            voronois: Vec::new(),
        },
    )
    .expect("meta grid");
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[2]).unwrap()).unwrap();
    let objects = ObjectTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let fix2x = Fix2xTable::default();

    let info = std::fs::read_to_string(region.join("chunkinfo.txt")).unwrap();
    let levels: Vec<(i32, i32, i32, i32)> = info
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let v: Vec<i32> = l.split_whitespace().map(|s| s.parse().unwrap()).collect();
            (v[0], v[1], v[2], v[3])
        })
        .collect();
    let chunks: Vec<(i32, i32)> = levels.iter().map(|l| (l.0, l.1)).collect();
    let mut cell = IsoCell::default();
    for &(wx, wy) in &chunks {
        let mut rand = Rand(JavaRandom::new(
            ((wx as i64) << 32) ^ (wy as i64 & 0xffff_ffff),
        ));
        let mut env = ChunkLoadEnv {
            sprites: &mut sprites,
            fix2x: &fix2x,
            rand: &mut rand,
            tables: &objects,
            settings: ObjectLoadSettings::default(),
            hooks: &mut NoHooks,
        };
        cell.load_chunk(&grid, wx, wy, &mut env);
    }
    for &(wx, wy) in &chunks {
        cell.recalc_chunk_squares(wx, wy, &grid, &sprites);
    }
    // The harness's level range per chunk (loading on this side does not
    // add the levels the original's erosion and stitching create).
    let mut level_diffs = 0;
    for &(wx, wy, mn, mx) in &levels {
        let ch = &cell.chunks[&(wx, wy)];
        if (ch.min_level, ch.max_level) != (mn, mx) {
            level_diffs += 1;
        }
    }
    println!("chunks whose level range differs={level_diffs}");

    let data = std::fs::read(region.join("tobullet.bin")).unwrap();
    let mut expect = Vec::new();
    let mut p = 0;
    let int = |p: &mut usize| {
        let v = i32::from_le_bytes(data[*p..*p + 4].try_into().unwrap());
        *p += 4;
        v
    };
    while p < data.len() {
        let (wx, wy, z, n) = (int(&mut p), int(&mut p), int(&mut p), int(&mut p) as usize);
        expect.push(((wx, wy, z), data[p..p + n].to_vec()));
        p += n;
    }
    let mut got = Vec::new();
    for &(wx, wy, mn, mx) in &levels {
        let ch = &cell.chunks[&(wx, wy)];
        for z in mn..=mx {
            if let Some(b) = update_physics_for_level_in(ch, z, mn, mx, &sprites, &|_| None) {
                got.push(((wx, wy, z), b));
            }
        }
    }
    let (mut same, mut diff) = (0, 0);
    for (i, (e, g)) in expect.iter().zip(&got).enumerate() {
        if e == g {
            same += 1;
        } else {
            diff += 1;
            if diff <= 5 {
                println!(
                    "record {i}: J {:?} R {:?} len {} {}",
                    e.0,
                    g.0,
                    e.1.len(),
                    g.1.len()
                );
                println!(
                    "  J head {:?}\n  R head {:?}",
                    &e.1[..e.1.len().min(24)],
                    &g.1[..g.1.len().min(24)]
                );
                let squares = |b: &[u8]| {
                    let mut out = Vec::new();
                    let mut q = 11;
                    while q + 2 < b.len() && b[q] != 0xff {
                        let n = b[q + 2] as usize;
                        out.push((b[q], b[q + 1], b[q + 3..q + 3 + n].to_vec()));
                        q += 3 + n;
                    }
                    out
                };
                let (je, ge) = (squares(&e.1), squares(&g.1));
                for (a, b) in je.iter().zip(&ge) {
                    if a != b {
                        println!("  square {},{}: J {:?} R {:?}", a.0, a.1, a.2, b.2);
                    }
                }
            }
        }
    }
    println!(
        "records java={} rust={} same={same} diff={diff}",
        expect.len(),
        got.len()
    );
    // Per square, where the harness also wrote the squares' objects: the
    // shapes of squares that loaded the same objects on both sides.
    let Ok(objs) = std::fs::read_to_string(region.join("objects.txt")) else {
        return;
    };
    let shapes_of = |recs: &[Record]| {
        let mut m = std::collections::HashMap::new();
        for ((wx, wy, z), b) in recs {
            let mut q = 11;
            while q + 2 < b.len() && b[q] != 0xff {
                let n = b[q + 2] as usize;
                m.insert(
                    (*wx, *wy, *z, b[q] as i32, b[q + 1] as i32),
                    b[q + 3..q + 3 + n].to_vec(),
                );
                q += 3 + n;
            }
        }
        m
    };
    let (js, rs) = (shapes_of(&expect), shapes_of(&got));
    let (mut eq_objs, mut eq_shapes, mut diff_objs, mut shown) = (0, 0, 0, 0);
    for l in objs.lines() {
        let t: Vec<&str> = l.splitn(6, ' ').collect();
        let k: Vec<i32> = t[..5].iter().map(|v| v.parse().unwrap()).collect();
        let (wx, wy, z, x, y) = (k[0], k[1], k[2], k[3], k[4]);
        let rust_names = cell
            .chunks
            .get(&(wx, wy))
            .and_then(|c| c.get_grid_square(x, y, z))
            .map(|sq| {
                let names: Vec<String> = sq
                    .objects
                    .iter()
                    .map(|o| {
                        let o = o.borrow();
                        o.sprite
                            .and_then(|i| sprites.sprites[i].name.clone())
                            .unwrap_or_else(|| "null".into())
                    })
                    .collect();
                format!("[{}]", names.join("|"))
            });
        let java_names = (t[5] != "-").then(|| t[5].to_string());
        if rust_names != java_names {
            diff_objs += 1;
            continue;
        }
        eq_objs += 1;
        let key = (wx, wy, z, x, y);
        if js.get(&key) == rs.get(&key) {
            eq_shapes += 1;
        } else if shown < 10 {
            shown += 1;
            println!(
                "  {key:?} {:?}: J {:?} R {:?}",
                java_names,
                js.get(&key),
                rs.get(&key)
            );
        }
    }
    println!(
        "squares with the same objects={eq_objs} same shapes={eq_shapes}; squares whose objects differ={diff_objs}"
    );
}
