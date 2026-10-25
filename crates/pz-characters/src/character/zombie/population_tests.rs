//! One population tick through `pz_world_popman::WorldPopulationHost`
//! with [`super::RealZombies`] on an invented cell.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pz_sandbox::ZombieConfig;
use pz_popman::native::Native;
use pz_popman::state_flags::{ZombieStateFlag, ZombieStateFlags};
use pz_popman::ZombiePopulationManager;
use pz_world::defs::room_id;
use pz_world::meta_grid::{IsoMetaCell, IsoMetaGrid};
use pz_world::sprite::SpriteTables;
use pz_world::{BuildingDef, BuildingIdx, GameRand, IsoCell, IsoChunk, IsoGridSquare, IsoSpriteManager, RoomDef, RoomIdx};
use pz_world_popman::{SittingChecks, WorldPopulationHost};

use super::RealZombies;
use crate::character::state::NullWorld;
use crate::character::tests::fixture;
use crate::character::{CharacterKind, GameCharacter, SimContext};
use crate::survivor::SurvivorDesc;

type LineCall = (f32, f32, f32, f32, i32, bool, bool);

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

}

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


struct Ctx(pz_core::random::Rand);
impl SimContext for Ctx {
    fn rand_next(&self, m: i32) -> i32 {
        self.0.next(m)
    }
    fn rand_next_range(&self, a: i32, b: i32) -> i32 {
        self.0.next_range(a, b)
    }
    fn rand_next_float(&self, a: f32, b: f32) -> f32 {
        self.0.next_float(a, b)
    }
}

#[test]
fn population_tick_creates_a_sitting_real_zombie() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/zombie_popman_sit");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("w/zpop")).unwrap();
    std::fs::create_dir_all(dir.join("w/chunkdata")).unwrap();
    let clock = Rc::new(Cell::new(1_000_000i64));
    let mut native = Native::new(1234);
    let c = clock.clone();
    native.gs.clock = Box::new(move || c.get());
    let mut m: ZombiePopulationManager<i32> = ZombiePopulationManager::new(native);
    {
        let n = &mut m.native;
        n.mcd_set_game_state_string("Core.GameSaveWorld", "w").unwrap();
        n.mcd_set_game_state_string("GameWindow.GameModeCacheDir", &format!("{}/", dir.display())).unwrap();
        n.mcd_set_game_state_string("SavefileNaming.SUBDIR_CHUNKDATA", "chunkdata").unwrap();
        n.mcd_set_game_state_string("SavefileNaming.SUBDIR_ZPOP", "zpop").unwrap();
        n.mcd_set_game_state_bool("PAUSED", false).unwrap();
        n.mcd_init_meta_grid(0, 0, 2, 2);
        n.mcd_init(0, 0, 2, 2);
    }
    let cfg = ZombieConfig { population_multiplier: 0.0, ..Default::default() };
    m.init((0, 0, 2, 2), &cfg, &["Generic01".to_string()]);
    let mut w = World::new();
    for x in 24..32 {
        for y in 24..32 {
            w.put(x, y, 0, &["solidfloor"]);
        }
    }
    w.put(25, 26, 0, &["solidfloor", "WallN"]);
    w.put(27, 28, 0, &["solidfloor", "WallW"]);
    let checks = Checks { blocked_to: vec![(25.5, 26.5)], ..Checks::default() };
    let mut rand = ListRand(vec![], vec![]);

    let (sm, mut chars) = fixture();
    let ctx = Ctx(pz_core::random::Rand::new(8));
    let desc = Rc::new(RefCell::new(SurvivorDesc::new(&mut chars, &sm).unwrap()));
    let chars = RefCell::new(chars);
    let mut zombies: Vec<GameCharacter<()>> = Vec::new();
    let mut next_id = 100;
    let mut nw = NullWorld;
    let mut survivor = || Ok(desc.clone());
    let water = |_: (i32, i32, i32)| false;
    let mut ops = RealZombies {
        sm: &sm,
        chars: &chars,
        ctx: &ctx,
        world: &mut nw,
        zombies: &mut zombies,
        next_id: &mut next_id,
        create_survivor: &mut survivor,
        water: &water,
        key_spawn_chance_d100: 0.0,
        building_keys: Vec::new(),
        removed: Vec::new(),
    };
    let state = ZombieStateFlags::from_flags(&[ZombieStateFlag::Initialized]).as_int();
    m.native.n_add_zombie(28.5, 29.5, 0.0, 4, 0, state, i32::MIN, i32::MIN, 0);
    {
        let mut host = WorldPopulationHost::new(&w.cell, &w.meta, &w.m, &checks, &mut rand, &mut ops);
        let mut tick = |m: &mut ZombiePopulationManager<i32>| {
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
    m.stop();
    assert_eq!(zombies.len(), 1);
    let z = &zombies[0];
    assert_eq!(z.kind, CharacterKind::Zombie);
    assert_eq!(z.id, 101);
    assert_ne!(z.zombie.persistent_id, 0);
    // Sat against the east wall of (26, 28), facing west.
    assert!(z.vars.sit_against_wall);
    assert_eq!((z.vars.x, z.vars.y), (26.5, 28.5));
    assert_eq!(z.vars.dir, pz_world::square::IsoDirections::W);
    assert!(!z.zombie.can_crawl_under_vehicle);
    assert!(!z.zombie.reanimated_for_grapple_only);
}

/// `zpop_X_Y.bin` version 7 (`DataOutputStream`, big-endian) for cell
/// `(0, 0)` with no outfit names and the given zombies `(x, y, id)` in
/// their chunks: standing walkers (`Initialized | CanWalk`), outfit 0,
/// facing south.
fn synthetic_zpop(zombies: &[(f32, f32, u32)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&7i16.to_be_bytes());
    b.extend_from_slice(&0i16.to_be_bytes());
    b.extend_from_slice(&0f32.to_be_bytes()); // lastRepop
    b.extend_from_slice(&0f32.to_be_bytes()); // lastRedistribute
    let state = ZombieStateFlags::from_flags(&[ZombieStateFlag::Initialized, ZombieStateFlag::CanWalk]).as_int();
    for i in 0..1024 {
        let (wx, wy) = (i % 32, i / 32);
        let here: Vec<_> = zombies.iter().filter(|z| (z.0 as i32) / 8 == wx && (z.1 as i32) / 8 == wy).collect();
        b.extend_from_slice(&0i16.to_be_bytes()); // desired
        b.extend_from_slice(&(here.len() as i16).to_be_bytes());
        for &&(x, y, id) in &here {
            b.extend_from_slice(&x.to_be_bytes());
            b.extend_from_slice(&y.to_be_bytes());
            b.push(0); // z
            b.push(4); // S
            b.extend_from_slice(&state.to_be_bytes());
            b.extend_from_slice(&0i32.to_be_bytes()); // outfit
            b.extend_from_slice(&id.to_be_bytes());
        }
        b.extend_from_slice(&0f32.to_be_bytes()); // lastSeen
        b.extend_from_slice(&0f32.to_be_bytes()); // lastRepop
    }
    b
}

/// A save whose `zpop_0_0.bin` holds three zombies in chunk (3, 3): the
/// world-load path (`init`, `addChunkToWorld`, `startGame`, per-frame
/// `updateMain`) makes them real zombies registered under their saved ids,
/// and unloading the chunk hands them back.
#[test]
fn loaded_world_zpop_zombies_become_real() {
    use pz_world_popman::world::save_game_state;
    use pz_world_popman::{PopulationFrame, WorldPopulation};

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/zombie_world_zpop");
    let _ = std::fs::remove_dir_all(&root);
    let save = root.join("Zomboid/Saves/Sandbox/w");
    std::fs::create_dir_all(save.join("zpop")).unwrap();
    let saved = [(25.5f32, 26.5f32, 501u32), (28.25, 29.75, 502), (30.5, 27.5, 503)];
    std::fs::write(save.join("zpop/zpop_0_0.bin"), synthetic_zpop(&saved)).unwrap();

    let clock = Rc::new(Cell::new(1_000_000i64));
    let mut native = Native::new(1234);
    let c = clock.clone();
    native.gs.clock = Box::new(move || c.get());
    let mut w = World::new();
    for x in 16..40 {
        for y in 16..40 {
            w.put(x, y, 0, &["solidfloor"]);
        }
    }
    let gs = save_game_state(&save);
    assert_eq!(gs.game_save_world, "w");
    let cfg = ZombieConfig { population_multiplier: 0.0, ..Default::default() };
    let mut pop: WorldPopulation<i32> = WorldPopulation::init(native, &w.meta, &gs, &cfg, &["Generic01".to_string()]);
    for wx in 2..=4 {
        for wy in 2..=4 {
            pop.add_chunk_to_world(&w.cell, &w.meta, &w.m, wx, wy);
        }
    }

    let (sm, mut chars) = fixture();
    let ctx = Ctx(pz_core::random::Rand::new(8));
    let desc = Rc::new(RefCell::new(SurvivorDesc::new(&mut chars, &sm).unwrap()));
    let chars = RefCell::new(chars);
    let mut zombies: Vec<GameCharacter<()>> = Vec::new();
    let mut next_id = 100;
    let mut nw = NullWorld;
    let mut survivor = || Ok(desc.clone());
    let water = |_: (i32, i32, i32)| false;
    let mut ops = RealZombies {
        sm: &sm,
        chars: &chars,
        ctx: &ctx,
        world: &mut nw,
        zombies: &mut zombies,
        next_id: &mut next_id,
        create_survivor: &mut survivor,
        water: &water,
        key_spawn_chance_d100: 0.0,
        building_keys: Vec::new(),
        removed: Vec::new(),
    };
    let maps = pz_world_popman::PlayerChunkMaps { origins: vec![(0, 0)], width: 10 };
    let checks = NoObstacles;
    let mut rand = ListRand(vec![1; 64], vec![]);
    let mut frame = |pop: &mut WorldPopulation<i32>, ops: &mut RealZombies<'_, ()>, start: bool| {
        let reals = ops.infos();
        let f = PopulationFrame { multiplier: 1.0, world_age_hours: 10.0, chunk_maps: &maps, real_zombies: &reals };
        let mut host = WorldPopulationHost::new(&w.cell, &w.meta, &w.m, &checks, &mut rand, ops);
        if start {
            pop.start_game(&mut host, &f).unwrap();
        } else {
            pop.update_game_state(false, false);
            pop.update(&mut host, &f).unwrap();
        }
        clock.set(clock.get() + 33);
    };
    frame(&mut pop, &mut ops, true);
    for _ in 0..30 {
        frame(&mut pop, &mut ops, false);
    }
    assert!(pop.worker.passes > 0);
    let mut got: Vec<(i32, (f32, f32))> = ops.zombies.iter().map(|z| (z.zombie.persistent_id, (z.vars.x, z.vars.y))).collect();
    got.sort_by_key(|g| g.0);
    assert_eq!(got, vec![(501, (25.5, 26.5)), (502, (28.25, 29.75)), (503, (30.5, 27.5))]);
    for z in ops.zombies.iter() {
        assert_eq!(z.kind, CharacterKind::Zombie);
        assert!(pop.manager.real_zombies().is_live(z.zombie.persistent_id));
        assert_eq!(z.forward_iso_direction(), pz_world::square::IsoDirections::S);
    }

    // The chunk unloads: its zombies go back to the library.
    let cz = ops.chunk_zombies(3, 3, &|_| false);
    assert_eq!(cz.len(), 3);
    {
        let mut host = WorldPopulationHost::new(&w.cell, &w.meta, &w.m, &checks, &mut rand, &mut ops);
        pop.remove_chunk_from_world(&mut host, 3, 3, &cz);
    }
    assert!(ops.zombies.is_empty());
    assert_eq!(ops.removed.len(), 3);
    pop.save(&[]);
    pop.stop();
}

struct NoObstacles;
impl SittingChecks for NoObstacles {
    fn has_moving_objects(&self, _x: i32, _y: i32, _z: i32) -> bool {
        false
    }
    fn line_clear_collide(&self, _: f32, _: f32, _: f32, _: f32, _: i32, _: bool, _: bool) -> bool {
        false
    }
}
