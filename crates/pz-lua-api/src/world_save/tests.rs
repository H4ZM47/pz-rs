use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use pz_world::zone_save::ClockUuids;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A fresh save folder for one test.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Dir {
        let d = std::env::temp_dir().join(format!(
            "pz-world-save-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }

    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Records the hook calls, and which files existed at each.
struct Log<'a> {
    dir: &'a Path,
    calls: Vec<String>,
}

/// A 2x2 thumbnail.
fn thumb() -> Thumbnail {
    Thumbnail {
        width: 2,
        height: 2,
        rgba: (0..16).collect(),
    }
}

impl Log<'_> {
    fn note(&mut self, what: &str) {
        let have: Vec<&str> = [
            "map_ver.bin",
            "map_sand.bin",
            "map_worldgen.bin",
            "statistics.bin",
            "thumb.png",
            "map.bin",
            "map_t.bin",
            "reanimated.bin",
        ]
        .into_iter()
        .filter(|f| self.dir.join(f).exists())
        .collect();
        self.calls.push(format!("{what} {have:?}"));
    }
}

impl SaveGameHooks for Log<'_> {
    fn on_save(&mut self) {
        self.note("on_save");
    }
    fn save_chunks(&mut self, _: &IsoMetaGrid) {
        self.note("chunks");
    }
    fn capture_thumbnail(&mut self, player: usize) -> Option<Thumbnail> {
        self.note(&format!("thumbnail {player}"));
        Some(thumb())
    }
    fn save_animals(&mut self) {
        self.note("animals");
    }
    fn save_population(&mut self) {
        self.note("population");
    }
    fn save_other_systems(&mut self) {
        self.note("others");
    }
    fn on_post_save(&mut self) {
        self.note("post_save");
    }
}

struct World {
    gt: GameTime,
    core: Core,
    meta: IsoMetaGrid,
    meta_world: MetaWorld,
    spawned: SpawnedZombieZones,
    uuids: ClockUuids,
    world_gen: WorldGenParams,
    instance_tracker: InstanceTracker,
    meta_tracker: MetaTracker,
    statistics: Statistics,
}

impl World {
    fn new() -> World {
        World {
            gt: GameTime::default(),
            core: Core::default(),
            meta: IsoMetaGrid::with_bounds(0, 0, 1, 1),
            meta_world: MetaWorld::default(),
            spawned: SpawnedZombieZones::default(),
            uuids: ClockUuids::default(),
            world_gen: WorldGenParams::new("seed"),
            instance_tracker: InstanceTracker::default(),
            meta_tracker: MetaTracker::default(),
            statistics: Statistics::default(),
        }
    }

    fn game<'a>(&'a mut self, dir: &'a Path, zombies: &'a [Vec<u8>]) -> SaveGame<'a> {
        SaveGame {
            save_dir: dir,
            no_save: false,
            has_cell: true,
            game_mode: "Sandbox",
            game_map: Some("Muldraugh, KY"),
            sandbox: b"SAND",
            game_time: &self.gt,
            core: &self.core,
            meta: &mut self.meta,
            meta_world: &self.meta_world,
            spawned_zones: &self.spawned,
            uuids: &mut self.uuids,
            players: None,
            can_save_players: true,
            reanimated: Some(zombies),
            world_gen: &self.world_gen,
            instance_tracker: &self.instance_tracker,
            cache_dir: None,
            meta_tracker: &self.meta_tracker,
            statistics: &self.statistics,
            local_players: &[false, true],
            on_game_thread: true,
        }
    }
}

#[test]
fn small_files_have_the_original_layouts() {
    assert_eq!(
        map_ver_bytes(Some("Ab")),
        [0, 0, 0, 249, 0, 0, 0, 2, 0, b'A', 0, b'b']
    );
    assert_eq!(map_ver_bytes(None), [0, 0, 0, 249, 0, 0, 0, 0]);
    assert_eq!(
        map_bin_bytes(CELL_SIZE),
        [0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 32]
    );
    assert_eq!(
        reanimated_bytes(&[vec![7, 8], vec![9]]),
        [0, 0, 0, 249, 0, 0, 0, 2, 7, 8, 9]
    );
}

#[test]
fn save_game_writes_in_the_original_order() {
    let d = Dir::new();
    let mut w = World::new();
    let zombies = [vec![1u8, 2, 3]];
    let mut g = w.game(&d.0, &zombies);
    let mut log = Log {
        dir: &d.0,
        calls: Vec::new(),
    };
    let r = save_game(&mut g, &mut log).unwrap();
    assert!(r.saved && r.errors.is_empty());
    // No player database: nothing to force.
    assert!(!r.force_save_players);
    assert_eq!(
        log.calls,
        [
            r#"on_save ["map_ver.bin", "map_sand.bin", "map_worldgen.bin", "statistics.bin"]"#,
            r#"thumbnail 1 ["map_ver.bin", "map_sand.bin", "map_worldgen.bin", "statistics.bin"]"#,
            r#"chunks ["map_ver.bin", "map_sand.bin", "map_worldgen.bin", "statistics.bin", "thumb.png"]"#,
            r#"animals ["map_ver.bin", "map_sand.bin", "map_worldgen.bin", "statistics.bin", "thumb.png", "map.bin", "map_t.bin", "reanimated.bin"]"#,
            r#"population ["map_ver.bin", "map_sand.bin", "map_worldgen.bin", "statistics.bin", "thumb.png", "map.bin", "map_t.bin", "reanimated.bin"]"#,
            r#"others ["map_ver.bin", "map_sand.bin", "map_worldgen.bin", "statistics.bin", "thumb.png", "map.bin", "map_t.bin", "reanimated.bin"]"#,
        ]
    );
    assert_eq!(
        d.names(),
        [
            "iTrack.bin",
            "map.bin",
            "map_animals.bin",
            "map_meta.bin",
            "map_sand.bin",
            "map_t.bin",
            "map_ver.bin",
            "map_worldgen.bin",
            "map_zone.bin",
            "metadata.bin",
            "reanimated.bin",
            "statistics.bin",
            "thumb.png"
        ]
    );
    assert_eq!(
        std::fs::read(d.0.join("map_worldgen.bin")).unwrap(),
        w.world_gen.save_bytes()
    );
    assert_eq!(
        std::fs::read(d.0.join("thumb.png")).unwrap(),
        pz_core::thumbnail::stbi_write_png(&thumb().rgba, 2, 2, 4, true)
    );
    assert_eq!(std::fs::read(d.0.join("map_sand.bin")).unwrap(), b"SAND");
    assert_eq!(
        std::fs::read(d.0.join("map_t.bin")).unwrap(),
        game_time_save::save_map_t(&w.gt, &w.core, false).unwrap()
    );
    assert_eq!(
        std::fs::read(d.0.join("reanimated.bin")).unwrap(),
        reanimated_bytes(&zombies)
    );
}

#[test]
fn a_failed_reanimated_save_writes_no_file() {
    let d = Dir::new();
    let mut w = World::new();
    let mut g = w.game(&d.0, &[]);
    g.reanimated = None;
    let mut log = Log {
        dir: &d.0,
        calls: Vec::new(),
    };
    let r = save_game(&mut g, &mut log).unwrap();
    assert!(r.saved && r.errors.is_empty());
    assert!(!d.names().contains(&"reanimated.bin".to_string()));
    assert!(d.names().contains(&"map_zone.bin".to_string()));
}

#[test]
fn games_that_are_not_saved_write_nothing() {
    for (no_save, has_cell, mode) in [
        (true, true, "Sandbox"),
        (false, false, "Sandbox"),
        (false, true, "LastStand"),
        (false, true, "Tutorial"),
    ] {
        let d = Dir::new();
        let mut w = World::new();
        let mut g = w.game(&d.0, &[]);
        (g.no_save, g.has_cell, g.game_mode) = (no_save, has_cell, mode);
        let mut log = Log {
            dir: &d.0,
            calls: Vec::new(),
        };
        let r = save_game(&mut g, &mut log).unwrap();
        assert!(!r.saved);
        assert!(log.calls.is_empty());
        assert!(d.names().is_empty());
    }
}

#[test]
fn quitting_stores_the_local_players_first_and_stops_further_player_saves() {
    let d = Dir::new();
    let dir = d.0.to_string_lossy().into_owned();
    let mut db = PlayerDb::new(false, &dir);
    let mut w = World::new();
    let mut g = w.game(&d.0, &[]);
    g.players = Some(&mut db);
    let mut log = Log {
        dir: &d.0,
        calls: Vec::new(),
    };
    let mut players = [PlayerData {
        sql_id: -1,
        x: 10.5,
        y: 20.5,
        name: Some("Kate".into()),
        world_version: WORLD_VERSION,
        bytes: vec![1, 2, 3],
        ..Default::default()
    }];
    let r = save_on_quit(&mut g, &mut players, &mut log);
    assert!(r.saved && r.errors.is_empty());
    // `canSavePlayers` went false before `IsoCell.save` asked.
    assert!(!r.force_save_players);
    assert_eq!(players[0].sql_id, 2);
    assert!(log.calls.last().unwrap().starts_with("post_save"));
    let row = db.load_player(2).unwrap();
    assert_eq!(row.bytes, [1, 2, 3]);
    assert_eq!(row.name.as_deref(), Some("Kate"));
    let mut vehicles = VehicleStore::default();
    close_stores(Some(&mut db), &mut vehicles, Default::default());
}

#[test]
fn a_regular_save_asks_for_the_players() {
    let d = Dir::new();
    let dir = d.0.to_string_lossy().into_owned();
    let mut db = PlayerDb::new(false, &dir);
    let mut w = World::new();
    let mut g = w.game(&d.0, &[]);
    g.players = Some(&mut db);
    let mut log = Log {
        dir: &d.0,
        calls: Vec::new(),
    };
    assert!(save_game(&mut g, &mut log).unwrap().force_save_players);
}

#[test]
fn hot_save_writes_meta_zones_and_time_unless_saving_is_off() {
    for no_save in [false, true] {
        let d = Dir::new();
        let mut w = World::new();
        let mut h = HotSave {
            save_dir: &d.0,
            no_save,
            meta: &mut w.meta,
            meta_world: &w.meta_world,
            spawned_zones: &w.spawned,
            uuids: &mut w.uuids,
            game_time: &w.gt,
            core: &w.core,
        };
        assert!(hot_save_ancillary(&mut h).is_empty());
        if no_save {
            assert!(d.names().is_empty());
        } else {
            assert_eq!(
                d.names(),
                [
                    "map_animals.bin",
                    "map_meta.bin",
                    "map_t.bin",
                    "map_zone.bin"
                ]
            );
            assert_eq!(
                std::fs::read(d.0.join("map_meta.bin")).unwrap(),
                save_map_meta(&w.meta, &w.meta_world, NetRole::SinglePlayer)
            );
        }
    }
}

#[test]
fn queued_meta_cells_are_saved_with_the_meta_grid() {
    let d = Dir::new();
    ensure_subdirectories_exist(&d.0).unwrap();
    let mut w = World::new();
    w.meta
        .set_cell(1, 0, Some(pz_world::meta_grid::IsoMetaCell::new(1, 0)));
    w.meta.add_cell_to_save(1, 0);
    let mut g = w.game(&d.0, &[]);
    // Not on the game thread: no thumbnail.
    g.on_game_thread = false;
    let mut log = Log {
        dir: &d.0,
        calls: Vec::new(),
    };
    let r = save_game(&mut g, &mut log).unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(!log.calls.iter().any(|c| c.starts_with("thumbnail")));
    assert_eq!(
        std::fs::read(d.0.join("metagrid/metacell_1_0.bin")).unwrap(),
        [0]
    );
    assert!(w.meta.cells_to_save.borrow().is_empty());
    let mut m = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    assert!(load_meta_grid_extras(&mut m, &d.0).is_empty());
    assert!(m.get_cell_data(1, 0).is_some());
}

#[test]
fn the_item_tracker_log_goes_to_the_cache_folder() {
    let d = Dir::new();
    let cache = Dir::new();
    let mut w = World::new();
    w.instance_tracker
        .inc(pz_core::save_trackers::ITEMS, "Base.Axe");
    let mut g = w.game(&d.0, &[]);
    g.cache_dir = Some(&cache.0);
    let mut log = Log {
        dir: &d.0,
        calls: Vec::new(),
    };
    save_game(&mut g, &mut log).unwrap();
    assert_eq!(
        std::fs::read_to_string(cache.0.join("ItemTracker.log")).unwrap(),
        "[Item Spawns] ; 1 entries\nBase.Axe = 1\n\n"
    );
    let mut t = InstanceTracker::default();
    t.load_file(&d.0, false).unwrap();
    assert_eq!(t.get(pz_core::save_trackers::ITEMS, "Base.Axe"), 1);
}
