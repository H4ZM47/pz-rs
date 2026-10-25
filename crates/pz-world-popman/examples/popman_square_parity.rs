//! Compares [`WorldPopulationHost`]'s square queries with the original
//! game's on real map chunks:
//! - `square()`: `solidFloorCached ? solidFloor : TreatAsSolidFloor()`,
//!   `roomId`, the room and building def ids;
//! - `isFree(true)` and `getWallType()`;
//! - `sitting_wall_type()` from three fixed offsets per square;
//! - the whole `getSquareForSittingZombie` search from seeded points.
//!
//! ```text
//! popman_square_parity <oracle dir> <game dir> [region...]
//! ```
//!
//! `<oracle dir>/<region>/popman.txt` comes from a local harness, run on a
//! machine with the game installed. The harness boots the original classes
//! headless and loads each region's chunks brand new from the lotpacks:
//! `IsoChunk.LoadChunk`, `loadInWorldStreamerThread`, then `setChunkDirect`
//! and `loadInMainThread` in row-major order, with a one-chunk margin. It
//! then writes, per square:
//! - `Q x y z popFloor treatAsSolidFloor cached roomId roomDef buildingDef
//!   isFree(true) movingObjects wallType objects`;
//! - `L fromX fromY toX toY z blocked`: `PolygonalMap2.lineClearCollide(...,
//!   null, false, true)`, its float arguments as raw bits;
//! - `S fromX fromY z (x y wallType | none)`: `getSquareForSittingZombie`,
//!   with `L` lines for every candidate square.
//!
//! None of this is game data, and the results are recorded as notes only.
//!
//! The port loads the same chunks the same way: `IsoCell::load_chunk`, then
//! `finish_chunk_load` per chunk in the same order. `lineClearCollide` is
//! `pz-pathfind`'s job and is compared there (#60). Here the original's
//! answers are fed back through [`SittingChecks`], and so are the
//! characters standing on squares (none on fresh chunks), so what is
//! compared is the rest of the logic.
//!
//! `pz-world` does not run world generation, so worldgen vegetation is
//! missing from the port's chunks. A square whose object list (class and
//! sprite) differs from the game's is skipped, as is any square next to
//! one that differs (for `getWallType`). A search is skipped when any
//! square it can reach differs.
//!
//! Results on build 42 (five regions; every compared value matched):
//!
//! | region | squares compared / dumped | sitting tests | searches compared (found a wall) |
//! |---|---|---|---|
//! | muldraugh | 4285 / 9817 | 12855 | 149 (29) |
//! | westpoint | 22229 / 22813 | 66687 | 1716 (438) |
//! | riverside | 10407 / 12429 | 31221 | 894 (89) |
//! | louisville | 27933 / 28351 | 83799 | 1881 (85) |
//! | forest | 1098 / 4105 | 3294 | 33 (0) |
//!
//! Each compared square checks six values: floor, `roomId`, room def,
//! building def, `isFree(true)` and `getWallType()`. Most of the skipped
//! squares are in `muldraugh` and `forest`, where worldgen vegetation is
//! missing from the port.
//!
//! The meta grid needs `CreateStep2`'s building loop
//! ([`IsoMetaGrid::assign_alarms`]) as well as [`IsoMetaGrid::create`].
//! Without it, a building that crosses a cell edge has no rooms in the
//! cell it extends into: 2288 `westpoint` squares then got room id -1.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use pz_fs::GameFs;
use pz_popman::manager::PopulationHost;
use pz_world::chunk::{ChunkLoadEnv, IsoCell};
use pz_world::fix2x::Fix2xTable;
use pz_world::save::chunk_file::finish_chunk_load;
use pz_world::{
    CreateParams, GameRand, IsoDirections, IsoGridSquare, IsoMetaGrid, IsoSpriteManager, NoHooks,
    ObjectLoadSettings, ObjectTables, SpriteTables, WorldGenBounds,
};
use pz_world_popman::{square_info, SittingChecks, WorldPopulationHost, ZombieOps};

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

/// `lineClearCollide` arguments as raw bits: from, to, z.
type LineKey = (u32, u32, u32, u32, i32);

/// The original's `lineClearCollide` answers and moving-object counts.
#[derive(Default)]
struct Recorded {
    lines: HashMap<LineKey, bool>,
    movers: HashSet<(i32, i32, i32)>,
    misses: Cell<usize>,
}

impl SittingChecks for Recorded {
    fn has_moving_objects(&self, x: i32, y: i32, z: i32) -> bool {
        self.movers.contains(&(x, y, z))
    }
    fn line_clear_collide(
        &self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        z: i32,
        ignore_doors: bool,
        close_to_walls: bool,
    ) -> bool {
        assert!(!ignore_doors && close_to_walls);
        let k = (
            from_x.to_bits(),
            from_y.to_bits(),
            to_x.to_bits(),
            to_y.to_bits(),
            z,
        );
        match self.lines.get(&k) {
            Some(&b) => b,
            None => {
                self.misses.set(self.misses.get() + 1);
                false
            }
        }
    }
}

/// No real zombies are made here.
struct NoZombies;

impl ZombieOps for NoZombies {
    type Zombie = u32;
    fn create_real_zombie(
        &mut self,
        _: i32,
        _: IsoDirections,
        _: i32,
        _: (i32, i32, i32),
    ) -> Result<Option<u32>, String> {
        Ok(None)
    }
    fn real_zombie_info(&self, _: u32) -> Option<pz_world_popman::RealZombieInfo<u32>> {
        None
    }
    fn set_persistent_id(&mut self, _: u32, _: i32) {}
    fn set_position(&mut self, _: u32, _: f32, _: f32) {}
    fn sit_against_wall(&mut self, _: u32, _: f32, _: f32, _: Option<IsoDirections>) {}
    fn make_fake_dead(&mut self, _: u32, _: f32) {}
    fn make_crawler(&mut self, _: u32, _: bool) {}
    fn set_can_crawl_under_vehicle(&mut self, _: u32, _: bool) {}
    fn set_reanimated_for_grapple_only(&mut self, _: u32, _: bool) {}
    fn path_to(&mut self, _: u32, _: i32, _: i32) {}
    fn die(&mut self, _: u32) {}
    fn remove_zombie(&mut self, _: u32) {}
}

/// One `Q` line.
struct Square {
    at: (i32, i32, i32),
    pop_floor: bool,
    room_id: i64,
    room: Option<i64>,
    building: Option<i64>,
    free: bool,
    wall: i32,
    objects: String,
}

fn objects(sq: &IsoGridSquare, m: &IsoSpriteManager) -> String {
    sq.objects
        .iter()
        .map(|o| {
            let o = o.borrow();
            format!(
                "{}:{};",
                o.kind.class_name(),
                o.sprite.map_or("null", |s| m.sprites[s].name())
            )
        })
        .collect()
}

fn h(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

fn opt(s: &str) -> Option<i64> {
    (s != "-").then(|| s.parse().unwrap())
}

#[derive(Default)]
struct Tally {
    compared: usize,
    matched: usize,
    examples: Vec<String>,
}

impl Tally {
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        self.compared += 1;
        if ok {
            self.matched += 1;
        } else if self.examples.len() < 8 {
            self.examples.push(what());
        }
    }
}

/// `getSquareForSittingZombie`'s search, over the host's square test.
fn search(host: &impl PopulationHost, x: f32, y: f32, z: i32) -> Option<(i32, i32, i32)> {
    let (bx, by) = (x.floor() as i32, y.floor() as i32);
    for dx in -3..3 {
        for dy in -3..3 {
            if let Some(w) = host.sitting_wall_type(bx + dx, by + dy, z, x, y) {
                return Some((bx + dx, by + dy, w));
            }
        }
    }
    None
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let oracle = PathBuf::from(&a[1]);
    let game = PathBuf::from(&a[2]);
    let only: Vec<&str> = a[3..].iter().map(String::as_str).collect();
    let jar = game.join("projectzomboid.jar");
    let tables = SpriteTables::from_game_jar(&jar).expect("sprite tables");
    let object_tables = ObjectTables::from_game_jar(&jar).expect("object tables");
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| true).unwrap();
    let mut meta = IsoMetaGrid::create(
        &GameFs::new(game.to_str().unwrap()),
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
    // The building loop of `CreateStep2`: rooms of buildings that cross a
    // cell edge are added to the adjacent cells (alarm option 1 draws
    // nothing; alarms do not matter here).
    meta.assign_alarms(&mut JavaRandom::new(0), false, true, 1, || 0);
    let fix2x = Fix2xTable::default();
    let regions = [
        ("muldraugh", 1346, 1201, 12, 12),
        ("westpoint", 1485, 857, 14, 12),
        ("riverside", 777, 648, 17, 8),
        ("louisville", 1574, 245, 10, 10),
        ("forest", 1300, 1200, 8, 8),
    ];
    let fields = [
        "floor", "roomId", "room", "building", "isFree", "wallType", "sit", "search",
    ];
    for (region, rx, ry, rw, rh) in regions {
        if !only.is_empty() && !only.contains(&region) {
            continue;
        }
        let path = oracle.join(region).join("popman.txt");
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("{region}: no {}", path.display());
            continue;
        };
        let mut cell = IsoCell::default();
        for wy in ry - 1..ry + rh + 1 {
            for wx in rx - 1..rx + rw + 1 {
                let mut rand = JavaRandom::new(((wx as i64) << 32) ^ (wy as i64 & 0xffff_ffff));
                let mut hooks = NoHooks;
                let mut env = ChunkLoadEnv {
                    sprites: &mut sprites,
                    fix2x: &fix2x,
                    rand: &mut rand,
                    tables: &object_tables,
                    settings: ObjectLoadSettings::default(),
                    hooks: &mut hooks,
                };
                cell.load_chunk(&meta, wx, wy, &mut env);
                finish_chunk_load(&mut cell, wx, wy, &meta, &mut sprites, None);
            }
        }

        let mut squares = Vec::new();
        let mut rec = Recorded::default();
        let mut searches = Vec::new();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let c: Vec<&str> = line.split(' ').collect();
            match c[0] {
                "Q" => {
                    let at = (
                        c[1].parse().unwrap(),
                        c[2].parse().unwrap(),
                        c[3].parse().unwrap(),
                    );
                    if c[11] != "0" {
                        rec.movers.insert(at);
                    }
                    squares.push(Square {
                        at,
                        pop_floor: c[4] == "1",
                        room_id: c[7].parse().unwrap(),
                        room: opt(c[8]),
                        building: opt(c[9]),
                        free: c[10] == "1",
                        wall: c[12].parse().unwrap(),
                        objects: c.get(13).copied().unwrap_or("").to_string(),
                    });
                }
                "L" => {
                    let k = (
                        h(c[1]).to_bits(),
                        h(c[2]).to_bits(),
                        h(c[3]).to_bits(),
                        h(c[4]).to_bits(),
                        c[5].parse().unwrap(),
                    );
                    rec.lines.insert(k, c[6] == "1");
                }
                "S" => {
                    let res = (c[4] != "none").then(|| {
                        (
                            c[4].parse::<i32>().unwrap(),
                            c[5].parse::<i32>().unwrap(),
                            c[6].parse::<i32>().unwrap(),
                        )
                    });
                    searches.push((h(c[1]), h(c[2]), c[3].parse::<i32>().unwrap(), res));
                }
                _ => {}
            }
        }

        // Squares whose objects differ from the game's (or that only one
        // side has).
        let mut bad: HashSet<(i32, i32, i32)> = HashSet::new();
        let mut game_has = HashSet::new();
        for s in &squares {
            game_has.insert(s.at);
            match cell.get_grid_square(s.at.0, s.at.1, s.at.2) {
                Some(sq) if objects(&sq.borrow(), &sprites) == s.objects => {}
                _ => {
                    bad.insert(s.at);
                }
            }
        }
        let (x0, y0, x1, y1) = (rx * 8, ry * 8, (rx + rw) * 8, (ry + rh) * 8);
        let mut port_only = 0;
        for (&(wx, wy), ch) in &cell.chunks {
            for z in ch.min_level..=ch.max_level {
                for x in 0..8 {
                    for y in 0..8 {
                        let at = (wx * 8 + x, wy * 8 + y, z);
                        if at.0 < x0 || at.0 >= x1 || at.1 < y0 || at.1 >= y1 {
                            continue;
                        }
                        if ch.get_grid_square(x, y, z).is_some() && !game_has.contains(&at) {
                            port_only += 1;
                            bad.insert(at);
                        }
                    }
                }
            }
        }
        // The margin chunks were not dumped: treat their squares as unknown.
        let unknown = |x: i32, y: i32| x < x0 || x >= x1 || y < y0 || y >= y1;
        let near_bad = |x: i32, y: i32, z: i32| {
            bad.contains(&(x, y, z))
                || bad.contains(&(x + 1, y, z))
                || bad.contains(&(x, y + 1, z))
                || unknown(x, y)
                || unknown(x + 1, y)
                || unknown(x, y + 1)
        };

        let mut zs = NoZombies;
        let mut rand = JavaRandom::new(0);
        let host = WorldPopulationHost::new(&cell, &meta, &sprites, &rec, &mut rand, &mut zs);
        let mut t: BTreeMap<&str, Tally> = fields.iter().map(|f| (*f, Tally::default())).collect();
        let mut skipped = 0;
        let offs: [(f32, f32); 3] = [(1.5, 2.25), (-1.75, 0.5), (2.9, -1.1)];
        for s in &squares {
            let (x, y, z) = s.at;
            if near_bad(x, y, z) {
                skipped += 1;
                continue;
            }
            let sq_owner = cell.get_grid_square(x, y, z).unwrap();
            let sq = sq_owner.borrow();
            let info = square_info(&sq, &meta, &sprites);
            let at = format!("{x},{y},{z}");
            t.get_mut("floor")
                .unwrap()
                .check(info.solid_floor == s.pop_floor, || {
                    format!("{at} game={} port={}", s.pop_floor, info.solid_floor)
                });
            t.get_mut("roomId")
                .unwrap()
                .check(info.room_id == s.room_id, || {
                    format!("{at} game={} port={}", s.room_id, info.room_id)
                });
            t.get_mut("room")
                .unwrap()
                .check(info.place.room == s.room, || {
                    format!("{at} game={:?} port={:?}", s.room, info.place.room)
                });
            t.get_mut("building")
                .unwrap()
                .check(info.place.building == s.building, || {
                    format!("{at} game={:?} port={:?}", s.building, info.place.building)
                });
            let free = host.is_free(&sq);
            t.get_mut("isFree").unwrap().check(free == s.free, || {
                format!("{at} game={} port={free}", s.free)
            });
            let wall = host.wall_type(&sq);
            t.get_mut("wallType").unwrap().check(wall == s.wall, || {
                format!("{at} game={} port={wall}", s.wall)
            });
            for (ox, oy) in offs {
                let (fx, fy) = (x as f32 + ox, y as f32 + oy);
                let blocked = rec.lines[&(
                    fx.to_bits(),
                    fy.to_bits(),
                    (x as f32 + 0.5).to_bits(),
                    (y as f32 + 0.5).to_bits(),
                    z,
                )];
                let game =
                    (s.free && s.building.is_none() && s.wall != 0 && !blocked).then_some(s.wall);
                let port = host.sitting_wall_type(x, y, z, fx, fy);
                t.get_mut("sit").unwrap().check(port == game, || {
                    format!("{at} from {fx},{fy} game={game:?} port={port:?}")
                });
            }
        }
        let mut search_skipped = 0;
        let (mut found, mut none) = (0, 0);
        for &(fx, fy, z, game) in &searches {
            let (bx, by) = (fx.floor() as i32, fy.floor() as i32);
            if (bx - 3..bx + 3).any(|x| (by - 3..by + 3).any(|y| near_bad(x, y, z))) {
                search_skipped += 1;
                continue;
            }
            let port = search(&host, fx, fy, z);
            if game.is_some() {
                found += 1;
            } else {
                none += 1;
            }
            t.get_mut("search").unwrap().check(port == game, || {
                format!("from {fx},{fy},{z} game={game:?} port={port:?}")
            });
        }
        println!(
            "== {region}: {} game squares, {} skipped (objects differ: {}, port-only {port_only}), {} searches ({search_skipped} skipped; compared {found} sit, {none} none), lineClearCollide misses {}",
            squares.len(),
            skipped,
            bad.len(),
            searches.len(),
            rec.misses.get()
        );
        for (f, tally) in &t {
            println!("  {f:9} {}/{}", tally.matched, tally.compared);
            for e in &tally.examples {
                println!("      {e}");
            }
        }
    }
}
