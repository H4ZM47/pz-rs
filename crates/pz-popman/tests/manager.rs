//! End-to-end: a zombie stored in an unloaded chunk becomes real when the
//! chunk loads, goes back to the library when it unloads, and the
//! population files are written. Invented bounds, no map data.

use pz_popman::manager::{ChunkZombie, RealZombieInfo, SquareInfo, SquarePlace, ZombieConfig};
use pz_popman::native::Native;
use pz_popman::state_flags::{ZombieStateFlag, ZombieStateFlags, ZombieStateSource};
use pz_popman::{PopulationHost, ZombiePopulationManager};
use pz_world::java::GameRand;
use pz_world::square::IsoDirections;
use std::cell::Cell;
use std::rc::Rc;

struct SeqRand(u32);

impl GameRand for SeqRand {
    fn next(&mut self, max: i32) -> i32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        if max <= 0 {
            0
        } else {
            ((self.0 >> 8) % max as u32) as i32
        }
    }
}

struct Host {
    rand: SeqRand,
    created: Vec<(i32, i32, (i32, i32, i32))>,
    positions: Vec<(u32, f32, f32)>,
    removed: Vec<u32>,
    crawlers: Vec<(u32, bool)>,
    next: u32,
}

impl PopulationHost for Host {
    type Zombie = u32;

    fn square(&self, x: i32, y: i32, _z: i32) -> Option<SquareInfo> {
        Some(SquareInfo { solid_floor: true, room_id: -1, place: SquarePlace { x, y, building: None, room: None } })
    }

    fn sitting_wall_type(&self, _x: i32, _y: i32, _z: i32, _fx: f32, _fy: f32) -> Option<i32> {
        None
    }

    fn rand(&mut self) -> &mut dyn GameRand {
        &mut self.rand
    }

    fn create_real_zombie(&mut self, outfit: i32, _dir: IsoDirections, persistent_id: i32, square: (i32, i32, i32)) -> Option<u32> {
        self.created.push((outfit, persistent_id, square));
        self.next += 1;
        Some(self.next)
    }

    fn set_position(&mut self, z: u32, x: f32, y: f32) {
        self.positions.push((z, x, y));
    }

    fn sit_against_wall(&mut self, _z: u32, _x: f32, _y: f32, _dir: Option<IsoDirections>) {}

    fn make_fake_dead(&mut self, _z: u32, _health: f32) {}

    fn make_crawler(&mut self, z: u32, can_walk: bool) {
        self.crawlers.push((z, can_walk));
    }

    fn set_can_crawl_under_vehicle(&mut self, _z: u32, _v: bool) {}

    fn set_reanimated_for_grapple_only(&mut self, _z: u32, _v: bool) {}

    fn path_to(&mut self, _z: u32, _x: i32, _y: i32) {}

    fn die(&mut self, _z: u32) {}

    fn remove_zombie(&mut self, z: u32) {
        self.removed.push(z);
    }
}

#[test]
fn chunk_load_makes_zombies_real_and_unload_stores_them() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("popman_manager");
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
    let mut host = Host {
        rand: SeqRand(7),
        created: Vec::new(),
        positions: Vec::new(),
        removed: Vec::new(),
        crawlers: Vec::new(),
        next: 0,
    };
    let reals: Vec<RealZombieInfo<u32>> = Vec::new();

    // A crawler waits in chunk (3, 3) while it is unloaded.
    let state = ZombieStateFlags::from_flags(&[ZombieStateFlag::Initialized, ZombieStateFlag::Crawling]).as_int();
    m.native.n_add_zombie(28.5, 29.5, 0.0, 4, 0, state, i32::MIN, i32::MIN, 0);
    let tick = |m: &mut ZombiePopulationManager<u32>, host: &mut Host| {
        clock.set(clock.get() + 50);
        m.update_main(host, 1.0, 10.0, &[(0, 0)], 10, &reals);
        m.thread_step();
    };
    for _ in 0..3 {
        tick(&mut m, &mut host);
    }
    assert!(host.created.is_empty());

    m.add_chunk_to_world(3, 3, false);
    for _ in 0..3 {
        tick(&mut m, &mut host);
    }
    assert_eq!(host.created.len(), 1, "{:?}", host.created);
    let (_, id, sq) = host.created[0];
    assert_ne!(id, 0);
    assert_eq!(sq, (28, 29, 0));
    assert_eq!(host.positions, vec![(1, 28.5, 29.5)]);
    assert_eq!(host.crawlers, vec![(1, false)]);

    // Register it as the game does, then unload its chunk.
    let info = RealZombieInfo {
        handle: 1u32,
        persistent_id: id,
        x: 28.5,
        y: 29.5,
        z: 0.0,
        dir: IsoDirections::S,
        outfit: 0,
        state: ZombieStateSource { crawling: true, ..Default::default() },
        path_target_x: i32::MIN,
        path_target_y: i32::MIN,
        dead: false,
        reanimated_player: false,
        walking: false,
    };
    assert_eq!(m.register_zombie(&info, id), id);
    assert!(m.real_zombies().is_live(id));
    m.remove_chunk_from_world(&mut host, 3, 3, &[ChunkZombie { info, level: 0, in_room: false }]);
    assert_eq!(host.removed, vec![1]);
    for _ in 0..3 {
        tick(&mut m, &mut host);
    }
    assert_eq!(host.created.len(), 1);

    m.save();
    assert!(dir.join("w/zpop/zpop_0_0.bin").exists());
    assert!(dir.join("w/zpop/zpop_ids.bin").exists());
    m.stop();
}
