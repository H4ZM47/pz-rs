use super::*;
use pz_sandbox::ZombieConfig;
use pz_popman::native::Native;
use pz_popman::state_flags::{ZombieStateFlag, ZombieStateFlags};
use pz_popman::ZombiePopulationManager;
use pz_world::defs::room_id;
use pz_world::meta_grid::IsoMetaCell;
use pz_world::sprite::SpriteTables;
use pz_world::{BuildingDef, BuildingIdx, IsoChunk, RoomDef, RoomIdx};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Hand-authored enum tables: the names `SpriteTables` resolves plus the
/// wall flags, in an arbitrary order.
fn sprites() -> IsoSpriteManager {
    let s = |v: &str| v.split_whitespace().map(str::to_string).collect::<Vec<_>>();
    let flags = s(
        "WallNW solid solidfloor solidtrans collideW collideN exterior NoWallLighting \
         doorW doorN transparentW transparentN cutW cutN invisible HoppableN HoppableW canPathW \
         canPathN blocksight ForceAmbient WallSE FloorHeightOneThird FloorHeightTwoThirds \
         CantClimb open windowW windowN WindowW WindowN diamondFloor transparentFloor hidewalls \
         container WallOverlay attachedSE attachedW attachedN FloorOverlay vegitation \
         NeverCutaway HasLightOnSprite EntityScript water blueprint BlockRain trans noStart \
         forceRender burntOut WallN WallW",
    );
    let types = s(
        "normal stairsBW wall doorW doorN tree windowFN windowFW isMoveAbleObject \
         lightswitch curtainN curtainS curtainE curtainW jukebox radio doorFrN doorFrW stairsTN \
         stairsMN stairsBN stairsTW stairsMW",
    );
    IsoSpriteManager::new(SpriteTables::new(flags, types, s("North West")).unwrap())
}

const ROOM: i64 = 5;
const BUILDING: i64 = 77;
const OUTSIDE_ROOM: i64 = 6;

/// Room [`ROOM`] of building [`BUILDING`], and room [`OUTSIDE_ROOM`] with no
/// building, both in cell (0, 0).
fn meta() -> IsoMetaGrid {
    struct Zero;
    impl GameRand for Zero {
        fn next(&mut self, _max: i32) -> i32 {
            0
        }
    }
    let mut g = IsoMetaGrid::with_bounds(0, 0, 0, 0);
    let mut cell = IsoMetaCell::new(0, 0);
    let mut b = BuildingDef::new(&mut Zero);
    b.id = BUILDING;
    g.building_defs.push(b);
    for (i, (id, building)) in [(ROOM, Some(BuildingIdx(0))), (OUTSIDE_ROOM, None)]
        .into_iter()
        .enumerate()
    {
        let id = room_id::make_id(0, 0, id as i32);
        let mut r = RoomDef::new(id, "r".into());
        r.building = building;
        g.room_defs.push(r);
        cell.rooms.insert(id, RoomIdx(i as u32));
    }
    g.set_cell(0, 0, Some(cell));
    g
}

fn room(index: i64) -> i64 {
    room_id::make_id(0, 0, index as i32)
}

struct World {
    cell: IsoCell,
    meta: IsoMetaGrid,
    m: IsoSpriteManager,
}

impl World {
    /// Chunks (2, 2)..=(4, 4) (squares 16..40), no squares yet.
    fn new() -> World {
        let mut cell = IsoCell::default();
        for wx in 2..=4 {
            for wy in 2..=4 {
                cell.chunks.insert((wx, wy), IsoChunk::new(wx, wy));
            }
        }
        World {
            cell,
            meta: meta(),
            m: sprites(),
        }
    }

    /// A square at `(x, y, z)` with the named flags and object types.
    fn put(&mut self, x: i32, y: i32, z: i32, names: &[&str]) -> &mut IsoGridSquare {
        let mut sq = IsoGridSquare::new(x, y, z);
        for n in names {
            if let Some(f) = self.m.tables.flag_from_string(n) {
                sq.properties.set_flag(f);
            } else {
                let t = self.m.tables.object_type_from_string(n);
                assert_ne!(t, self.m.tables.object_type_max(), "{n}");
                sq.has_types |= 1u64 << (t.index() & 63);
            }
        }
        let ch = self
            .cell
            .chunks
            .get_mut(&(x.div_euclid(8), y.div_euclid(8)))
            .unwrap();
        ch.set_square(x.rem_euclid(8), y.rem_euclid(8), z, Some(sq));
        self.cell.get_grid_square_mut(x, y, z).unwrap()
    }

    fn sq(&self, x: i32, y: i32, z: i32) -> &IsoGridSquare {
        self.cell.get_grid_square(x, y, z).unwrap()
    }
}

/// `lineClearCollide` arguments: from, to, z, doors ignored, close to walls.
type LineCall = (f32, f32, f32, f32, i32, bool, bool);

/// [`SittingChecks`] with movers on listed squares, blocked lines to listed
/// targets, and a log of the `lineClearCollide` calls.
#[derive(Default)]
struct Checks {
    movers: Vec<(i32, i32, i32)>,
    blocked_to: Vec<(f32, f32)>,
    calls: RefCell<Vec<LineCall>>,
}

impl SittingChecks for Checks {
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
        self.calls
            .borrow_mut()
            .push((from_x, from_y, to_x, to_y, z, ignore_doors, close_to_walls));
        self.blocked_to.contains(&(to_x, to_y))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Create(i32, IsoDirections, i32, (i32, i32, i32)),
    Pos(u32, f32, f32),
    Sit(u32, f32, f32, Option<IsoDirections>),
    FakeDead(u32, f32),
    Crawler(u32, bool),
    CrawlUnder(u32, bool),
    Grapple(u32, bool),
    PathTo(u32, i32, i32),
    Die(u32),
    Remove(u32),
}

#[derive(Default)]
struct Zombies {
    ops: Vec<Op>,
    next: u32,
}

impl ZombieOps for Zombies {
    type Zombie = u32;
    fn create_real_zombie(
        &mut self,
        outfit: i32,
        dir: IsoDirections,
        persistent_id: i32,
        square: (i32, i32, i32),
    ) -> Result<Option<u32>, String> {
        self.ops
            .push(Op::Create(outfit, dir, persistent_id, square));
        self.next += 1;
        Ok(Some(self.next))
    }
    fn real_zombie_info(&self, _z: u32) -> Option<RealZombieInfo<u32>> {
        None
    }
    fn set_persistent_id(&mut self, _z: u32, _id: i32) {}
    fn set_position(&mut self, z: u32, x: f32, y: f32) {
        self.ops.push(Op::Pos(z, x, y));
    }
    fn sit_against_wall(&mut self, z: u32, x: f32, y: f32, dir: Option<IsoDirections>) {
        self.ops.push(Op::Sit(z, x, y, dir));
    }
    fn make_fake_dead(&mut self, z: u32, health: f32) {
        self.ops.push(Op::FakeDead(z, health));
    }
    fn make_crawler(&mut self, z: u32, can_walk: bool) {
        self.ops.push(Op::Crawler(z, can_walk));
    }
    fn set_can_crawl_under_vehicle(&mut self, z: u32, v: bool) {
        self.ops.push(Op::CrawlUnder(z, v));
    }
    fn set_reanimated_for_grapple_only(&mut self, z: u32, v: bool) {
        self.ops.push(Op::Grapple(z, v));
    }
    fn path_to(&mut self, z: u32, x: i32, y: i32) {
        self.ops.push(Op::PathTo(z, x, y));
    }
    fn die(&mut self, z: u32) {
        self.ops.push(Op::Die(z));
    }
    fn remove_zombie(&mut self, z: u32) {
        self.ops.push(Op::Remove(z));
    }
}

/// Draws from a fixed list, then 0.
struct ListRand(Vec<i32>, Vec<i32>);

impl GameRand for ListRand {
    fn next(&mut self, max: i32) -> i32 {
        self.1.push(max);
        if self.0.is_empty() {
            0
        } else {
            self.0.remove(0)
        }
    }
}

#[test]
fn square_reads_floor_room_and_building() {
    let mut w = World::new();
    w.put(20, 20, 0, &["solidfloor"]);
    w.put(21, 20, 0, &[]);
    w.put(22, 20, 0, &["stairsTN"]);
    w.put(23, 20, 0, &["solidfloor"]).room_id = room(ROOM);
    w.put(24, 20, 0, &["solidfloor"]).room_id = room(OUTSIDE_ROOM);
    // A room id the meta grid does not know: `getRoom()` finds no room.
    w.put(25, 20, 0, &["solidfloor"]).room_id = room(9);
    let (checks, mut rand, mut zs) = (
        NoSittingObstacles,
        ListRand(vec![], vec![]),
        Zombies::default(),
    );
    let h = WorldPopulationHost::new(&w.cell, &w.meta, &w.m, &checks, &mut rand, &mut zs);

    assert!(h.square(20, 21, 0).is_none());
    assert!(h.square(20, 20, 1).is_none());
    let sq = h.square(20, 20, 0).unwrap();
    assert!(sq.solid_floor);
    assert_eq!(sq.room_id, -1);
    assert_eq!(
        (sq.place.x, sq.place.y, sq.place.building, sq.place.room),
        (20, 20, None, None)
    );
    assert!(!h.square(21, 20, 0).unwrap().solid_floor);
    // Stairs count as a floor (`TreatAsSolidFloor`).
    assert!(h.square(22, 20, 0).unwrap().solid_floor);

    let sq = h.square(23, 20, 0).unwrap();
    assert_eq!(sq.room_id, room(ROOM));
    assert_eq!(sq.place.room, Some(room(ROOM)));
    assert_eq!(sq.place.building, Some(BUILDING));
    let sq = h.square(24, 20, 0).unwrap();
    assert_eq!(
        (sq.place.room, sq.place.building),
        (Some(room(OUTSIDE_ROOM)), None)
    );
    let sq = h.square(25, 20, 0).unwrap();
    assert_eq!(sq.room_id, room(9));
    assert_eq!((sq.place.room, sq.place.building), (None, None));
}

#[test]
fn wall_type_bits() {
    let mut w = World::new();
    w.put(20, 20, 0, &["WallN"]);
    w.put(21, 20, 0, &["WallW"]);
    w.put(22, 20, 0, &["WallNW"]);
    w.put(23, 20, 0, &[]);
    w.put(20, 21, 0, &["WallN"]);
    w.put(23, 21, 0, &["WallNW"]);
    w.put(24, 21, 0, &["WallNW"]);
    let f = WallFlags::new(&w.m);
    let t = |x, y| wall_type(w.sq(x, y, 0), &w.cell, &f);
    // North wall here; west wall on the square to the east; north wall on
    // the square to the south.
    assert_eq!(t(20, 20), 1 | 8 | 2);
    // West wall here, WallNW on the square to the east.
    assert_eq!(t(21, 20), 4 | 8);
    // WallNW is both; the square to its east has no wall.
    assert_eq!(t(22, 20), 5);
    // Nothing here; WallNW to the south counts as north.
    assert_eq!(t(23, 20), 2);
    // WallNW here and WallNW to the east.
    assert_eq!(t(23, 21), 5 | 8);
    // No square to the south or east (outside the loaded chunks).
    w.put(39, 39, 0, &["WallW"]);
    assert_eq!(wall_type(w.sq(39, 39, 0), &w.cell, &f), 4);
}

#[test]
fn is_free_rules() {
    let mut w = World::new();
    w.put(20, 20, 0, &["solidfloor"]);
    w.put(21, 20, 0, &[]);
    w.put(22, 20, 0, &["solidfloor", "solid"]);
    w.put(23, 20, 0, &["solidfloor", "solidtrans"]);
    w.put(24, 20, 0, &["solidfloor", "tree"]);
    // Stairs are free even when solid and without a floor.
    w.put(25, 20, 0, &["stairsBW", "solid"]);
    w.put(26, 20, 0, &["stairsMN"]);
    let checks = Checks {
        movers: vec![(26, 20, 0), (20, 21, 0)],
        ..Checks::default()
    };
    w.put(20, 21, 0, &["solidfloor"]);
    let (mut rand, mut zs) = (ListRand(vec![], vec![]), Zombies::default());
    let h = WorldPopulationHost::new(&w.cell, &w.meta, &w.m, &checks, &mut rand, &mut zs);
    let free = |x, y| h.is_free(w.sq(x, y, 0));
    assert!(free(20, 20));
    assert!(!free(21, 20));
    assert!(!free(22, 20));
    assert!(!free(23, 20));
    assert!(!free(24, 20));
    assert!(free(25, 20));
    // `isFree(true)` counts the characters on the square.
    assert!(!free(26, 20));
    assert!(!free(20, 21));
    assert!(is_free_ignoring_movers(w.sq(26, 20, 0), &w.m));
}

#[test]
fn sitting_wall_type_tests_in_order() {
    let mut w = World::new();
    // Passes: free, outside, a north wall.
    w.put(20, 20, 0, &["solidfloor", "WallN"]);
    // No wall.
    w.put(21, 20, 0, &["solidfloor"]);
    // Inside a building.
    w.put(22, 20, 0, &["solidfloor", "WallN"]).room_id = room(ROOM);
    // A room without a building is not a building.
    w.put(23, 20, 0, &["solidfloor", "WallW"]).room_id = room(OUTSIDE_ROOM);
    // Not free.
    w.put(24, 20, 0, &["solidfloor", "WallN", "solid"]);
    // A character stands there.
    w.put(25, 20, 0, &["solidfloor", "WallN"]);
    // The line to it is blocked.
    w.put(26, 20, 0, &["solidfloor", "WallN"]);
    // Upstairs.
    w.put(20, 20, 1, &["solidfloor", "WallW"]);
    let checks = Checks {
        movers: vec![(25, 20, 0)],
        blocked_to: vec![(26.5, 20.5)],
        ..Checks::default()
    };
    let (mut rand, mut zs) = (ListRand(vec![], vec![]), Zombies::default());
    let h = WorldPopulationHost::new(&w.cell, &w.meta, &w.m, &checks, &mut rand, &mut zs);
    let t = |x, z| h.sitting_wall_type(x, 20, z, 18.25, 21.75);
    assert_eq!(t(20, 0), Some(1));
    assert_eq!(t(21, 0), None);
    assert_eq!(t(22, 0), None);
    assert_eq!(t(23, 0), Some(4));
    assert_eq!(t(24, 0), None);
    assert_eq!(t(25, 0), None);
    assert_eq!(t(26, 0), None);
    assert_eq!(t(20, 1), Some(4));
    assert_eq!(t(27, 0), None);
    // `lineClearCollide` runs last, only for squares that passed the rest,
    // from the zombie to the square's centre, doors not ignored, close to
    // walls.
    assert_eq!(
        *checks.calls.borrow(),
        vec![
            (18.25, 21.75, 20.5, 20.5, 0, false, true),
            (18.25, 21.75, 23.5, 20.5, 0, false, true),
            (18.25, 21.75, 26.5, 20.5, 0, false, true),
            (18.25, 21.75, 20.5, 20.5, 1, false, true),
        ]
    );
}

#[test]
fn zombie_ops_and_rand_are_forwarded() {
    let w = World::new();
    let checks = NoSittingObstacles;
    let mut rand = ListRand(vec![2, 5], vec![]);
    let mut zs = Zombies::default();
    {
        let mut h = WorldPopulationHost::new(&w.cell, &w.meta, &w.m, &checks, &mut rand, &mut zs);
        assert_eq!(h.rand().next(3), 2);
        assert_eq!(h.rand().next(9), 5);
        let z = h
            .create_real_zombie(4, IsoDirections::SW, 99, (1, 2, 0))
            .unwrap()
            .unwrap();
        h.set_position(z, 1.5, 2.5);
        h.sit_against_wall(z, 1.5, 2.5, Some(IsoDirections::N));
        h.make_fake_dead(z, 0.75);
        h.make_crawler(z, true);
        h.set_can_crawl_under_vehicle(z, true);
        h.set_reanimated_for_grapple_only(z, false);
        h.path_to(z, 10, 11);
        h.die(z);
        h.remove_zombie(z);
    }
    assert_eq!(rand.1, vec![3, 9]);
    assert_eq!(
        zs.ops,
        vec![
            Op::Create(4, IsoDirections::SW, 99, (1, 2, 0)),
            Op::Pos(1, 1.5, 2.5),
            Op::Sit(1, 1.5, 2.5, Some(IsoDirections::N)),
            Op::FakeDead(1, 0.75),
            Op::Crawler(1, true),
            Op::CrawlUnder(1, true),
            Op::Grapple(1, false),
            Op::PathTo(1, 10, 11),
            Op::Die(1),
            Op::Remove(1),
        ]
    );
}

#[test]
fn player_chunk_map_origins() {
    let mut s = ChunkStreamer::new(3, 13);
    for (i, (wx, wy)) in [(100, 200), (50, 60), (-3, 7)].into_iter().enumerate() {
        let cm = s.chunk_map_mut(i);
        cm.world_x = wx;
        cm.world_y = wy;
    }
    s.chunk_map_mut(1).ignore = true;
    let maps = player_chunk_maps(&s);
    assert_eq!(maps.width, 13);
    assert_eq!(maps.origins, vec![(94, 194), (-9, 1)]);
}

/// A walker stored in an unloaded chunk becomes real when its chunk loads
/// and, on a `Rand.Next(3) == 0` draw, sits against the first wall
/// `getSquareForSittingZombie` finds.
#[test]
fn manager_sits_a_zombie_against_a_wall_of_the_cell() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp/world_popman_sit");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("w/zpop")).unwrap();
    std::fs::create_dir_all(dir.join("w/chunkdata")).unwrap();
    let clock = Rc::new(Cell::new(1_000_000i64));
    let mut native = Native::new(1234);
    let c = clock.clone();
    native.gs.clock = Box::new(move || c.get());
    let mut m: ZombiePopulationManager<u32> = ZombiePopulationManager::new(native);
    {
        let n = &mut m.native;
        n.mcd_set_game_state_string("Core.GameSaveWorld", "w")
            .unwrap();
        n.mcd_set_game_state_string(
            "GameWindow.GameModeCacheDir",
            &format!("{}/", dir.display()),
        )
        .unwrap();
        n.mcd_set_game_state_string("SavefileNaming.SUBDIR_CHUNKDATA", "chunkdata")
            .unwrap();
        n.mcd_set_game_state_string("SavefileNaming.SUBDIR_ZPOP", "zpop")
            .unwrap();
        n.mcd_set_game_state_bool("PAUSED", false).unwrap();
        n.mcd_init_meta_grid(0, 0, 2, 2);
        n.mcd_init(0, 0, 2, 2);
    }
    let cfg = ZombieConfig {
        population_multiplier: 0.0,
        ..Default::default()
    };
    m.init((0, 0, 2, 2), &cfg, &["Generic01".to_string()]);

    let mut w = World::new();
    for x in 24..32 {
        for y in 24..32 {
            w.put(x, y, 0, &["solidfloor"]);
        }
    }
    // Searched x-major from (-3, -3): the line to (25, 26) is blocked,
    // (26, 28) passes with a west wall on the square to its east.
    w.put(25, 26, 0, &["solidfloor", "WallN"]);
    w.put(27, 28, 0, &["solidfloor", "WallW"]);
    let checks = Checks {
        blocked_to: vec![(25.5, 26.5)],
        ..Checks::default()
    };
    let mut rand = ListRand(vec![], vec![]);
    let mut zs = Zombies::default();

    let state = ZombieStateFlags::from_flags(&[ZombieStateFlag::Initialized]).as_int();
    m.native
        .n_add_zombie(28.5, 29.5, 0.0, 4, 0, state, i32::MIN, i32::MIN, 0);
    {
        let mut host =
            WorldPopulationHost::new(&w.cell, &w.meta, &w.m, &checks, &mut rand, &mut zs);
        let mut tick = |m: &mut ZombiePopulationManager<u32>| {
            clock.set(clock.get() + 50);
            m.update_main(&mut host, 1.0, 10.0, &[(0, 0)], 10, &[]).unwrap();
            m.thread_step();
        };
        for _ in 0..3 {
            tick(&mut m);
        }
        m.add_chunk_to_world(3, 3, false);
        for _ in 0..3 {
            tick(&mut m);
        }
    }
    let (id, square) = match zs.ops.first() {
        Some(&Op::Create(0, IsoDirections::S, id, sq)) => (id, sq),
        other => panic!("{other:?} in {:?}", zs.ops),
    };
    assert_ne!(id, 0);
    assert_eq!(square, (26, 28, 0));
    // Wall type 8 (a wall on the east edge) leaves one direction, facing
    // away from it; `Rand.Next(1)` picks it.
    assert_eq!(
        zs.ops[1..],
        [
            Op::Sit(1, 26.5, 28.5, Some(IsoDirections::W)),
            Op::CrawlUnder(1, false),
            Op::Grapple(1, false),
        ]
    );
    assert_eq!(rand.1, vec![3, 1]);
    assert_eq!(
        *checks.calls.borrow(),
        vec![
            (28.5, 29.5, 25.5, 26.5, 0, false, true),
            (28.5, 29.5, 26.5, 28.5, 0, false, true),
        ]
    );
    m.stop();
}
