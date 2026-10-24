//! End-to-end erosion over a `pz-world` chunk with synthetic tables (no
//! game data): chunk/square initialisation, replacing a map tree, spawning
//! and growing objects, and the saved records.

use std::sync::Arc;

use pz_erosion::regions::ErosionRegions;
use pz_erosion::tables::{BushInit, Ev, PlantInit, TreeInit};
use pz_erosion::world_host::NoEdges;
use pz_erosion::{ErosionMain, ErosionSettings, ErosionTables};
use pz_world::objects::{IsoObject, ObjectKind, ObjectName};
use pz_world::{GameRand, IsoChunk, IsoGridSquare, IsoSpriteManager, SpriteTables};

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn sprite_tables() -> SpriteTables {
    let flags = names(&[
        "collideW",
        "collideN",
        "exterior",
        "NoWallLighting",
        "doorW",
        "doorN",
        "transparentW",
        "transparentN",
        "cutW",
        "cutN",
        "solid",
        "invisible",
        "HoppableN",
        "HoppableW",
        "canPathW",
        "canPathN",
        "blocksight",
        "ForceAmbient",
        "WallSE",
        "FloorHeightOneThird",
        "FloorHeightTwoThirds",
        "CantClimb",
        "open",
        "windowW",
        "windowN",
        "WindowW",
        "WindowN",
        "diamondFloor",
        "transparentFloor",
        "solidfloor",
        "hidewalls",
        "container",
        "WallOverlay",
        "attachedSE",
        "attachedW",
        "attachedN",
        "FloorOverlay",
        "vegitation",
        "NeverCutaway",
        "HasLightOnSprite",
        "EntityScript",
    ]);
    let types = names(&[
        "normal",
        "wall",
        "doorW",
        "doorN",
        "tree",
        "windowFN",
        "windowFW",
        "isMoveAbleObject",
        "lightswitch",
        "curtainN",
        "curtainS",
        "curtainE",
        "curtainW",
        "jukebox",
        "radio",
        "doorFrN",
        "doorFrW",
    ]);
    SpriteTables::new(flags, types, names(&["North", "West"])).unwrap()
}

/// Made-up tables with the shapes the game's have.
fn synthetic_tables() -> ErosionTables {
    let perm: Arc<[i32]> = (0..256)
        .map(|i| (i * 37 + 11) % 256)
        .collect::<Vec<i32>>()
        .into();
    let soil_ref = |n: usize, k: i32| {
        (0..12)
            .map(|r| (0..n).map(|c| ((r + c) as i32 % k) + 1).collect())
            .collect()
    };
    ErosionTables {
        noise_perm: perm,
        soil_table: (0..10)
            .map(|r| (0..10).map(|c| (r + c) % 12 + 1).collect())
            .collect(),
        square_rands: (0..8100).map(|i| (i * 7) % 100).collect(),
        trees: vec![
            TreeInit::new("Tree A", "t_a_1", false),
            TreeInit::new("Tree B", "t_b_1", true),
        ],
        trees_soil_ref: soil_ref(4, 2),
        plants: (0..24)
            .map(|i| PlantInit {
                name: format!("Plant {i}"),
                has_flower: i % 2 == 0,
                bloomstart: 0.1,
                bloomend: 0.4,
            })
            .collect(),
        plants_soil_ref: soil_ref(5, 24),
        bush: (0..16)
            .map(|i| BushInit {
                name: format!("Bush {i}"),
                bloomstart: 0.2,
                bloomend: 0.5,
                has_flower: i % 3 == 0,
            })
            .collect(),
        bush_soil_ref: soil_ref(4, 16),
        flowerbed_tile_ids: (16..28).collect(),
        season_names: names(&["s0", "s1", "s2", "s3", "s4", "s5"]),
        config_defaults: vec![
            ("time.tickunit".into(), Ev::Int(144)),
            ("season.lat".into(), Ev::Int(38)),
            ("season.tempMax".into(), Ev::Int(25)),
            ("season.tempDiff".into(), Ev::Int(7)),
            ("season.seasonLag".into(), Ev::Int(31)),
            ("season.noon".into(), Ev::Float(12.5)),
            ("seeds.seedMain0".into(), Ev::Int(16)),
        ],
    }
}

struct Lcg(u64);
impl GameRand for Lcg {
    fn next(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) % max as u64) as i32
    }
}

fn sprite(m: &mut IsoSpriteManager, name: &str, id: i32) -> usize {
    let s = m.add_sprite(name, id);
    m.sprites[s].name = Some(name.to_string());
    s
}

fn setup() -> (IsoSpriteManager, IsoChunk) {
    let mut m = IsoSpriteManager::new(sprite_tables());
    let floor = sprite(&mut m, "blends_natural_01_5", 100);
    let solid = m.tables.flag.solidfloor;
    m.sprites[floor].properties.set_flag(solid);
    let tree = sprite(&mut m, "vegetation_trees_01_8", 101);
    let mut chunk = IsoChunk::new(2, 3);
    for x in 0..8 {
        for y in 0..8 {
            let mut sq = IsoGridSquare::new(16 + x, 24 + y, 0);
            let id = sq.new_object_id();
            sq.objects
                .push(IsoObject::new(id, ObjectKind::Object, Some(floor)));
            if (x, y) == (3, 4) {
                let id = sq.new_object_id();
                sq.objects
                    .push(IsoObject::new(id, ObjectKind::Object, Some(tree)));
            }
            chunk.set_square(x, y, 0, Some(sq));
        }
    }
    (m, chunk)
}

#[test]
fn regions_and_categories() {
    let tables = synthetic_tables();
    let (regions, ice) = ErosionRegions::init(&tables);
    assert_eq!(regions.regions.len(), 4);
    assert_eq!(regions.regions[0].categories.len(), 4);
    assert_eq!(regions.regions[2].categories.len(), 2);
    // Trees: one winter sprite per stage; bushes: two per bush; ferns: two.
    assert_eq!(ice.len(), 2 * 8 + 16 * 2 + 8 * 2);
    let names = regions.get_object_names();
    assert!(names.contains(&"Tree A".to_string()) && names.contains(&"WallCracks".to_string()));
}

#[test]
fn chunk_load_replaces_and_grows() {
    let tables = synthetic_tables();
    let (mut sprites, mut chunk) = setup();
    let settings = ErosionSettings {
        time_since_apo: 3,
        ..Default::default()
    };
    let mut main = ErosionMain::new(tables, settings, &mut sprites, None, None);
    // timeSinceApo 3: 60 days of a 100-day (tickunit 144) cycle.
    assert_eq!(main.get_etick(), 60);
    assert!(!main.ice_queen.is_empty());
    let mut rand = Lcg(7);
    for x in 0..8 {
        for y in 0..8 {
            main.load_grid_square(&mut chunk, x, y, 0, &mut sprites, &NoEdges, &mut rand, None);
        }
    }
    main.chunk_loaded(&mut chunk);

    let cd = chunk.erosion.clone().unwrap();
    assert!(cd.init);
    assert_eq!((cd.e_tick_stamp, cd.epoch), (60, 0));

    // The map tree was adopted by NatureTrees and renamed.
    let sq = chunk.get_grid_square(3, 4, 0).unwrap();
    let saved = sq.erosion.as_ref().unwrap();
    assert!(saved.init);
    let tree_data = saved
        .regions
        .iter()
        .find(|r| (r.region_id, r.category_id) == (0, 0))
        .unwrap();
    assert!(tree_data.has_spawned);
    let tree = sq
        .objects
        .iter()
        .find(|o| matches!(&o.name, Some(ObjectName::Str(n)) if n.starts_with("Tree")))
        .unwrap();
    // update() then staged it with the erosion sprite set (`t_*_1_<n>`).
    let name = sprites.sprites[tree.sprite.unwrap()].name.clone().unwrap();
    assert!(
        name.starts_with("t_a_1_") || name.starts_with("t_b_1_"),
        "{name}"
    );

    // Every square was initialised; in-memory data mirrors the saved record.
    for x in 0..8 {
        for y in 0..8 {
            let s = chunk.get_grid_square(x, y, 0).unwrap();
            assert!(s.erosion.as_ref().unwrap().init);
            let mem = main.square_data(16 + x, 24 + y, 0).unwrap();
            assert_eq!(&mem.to_saved(), s.erosion.as_ref().unwrap());
            assert!(mem.rand.is_some());
        }
    }

    // A second pass with unchanged stamps leaves everything alone.
    let before: Vec<_> = chunk
        .squares()
        .map(|s| (s.objects.len(), s.erosion.clone()))
        .collect();
    main.load_grid_square(&mut chunk, 0, 0, 0, &mut sprites, &NoEdges, &mut rand, None);
    let after: Vec<_> = chunk
        .squares()
        .map(|s| (s.objects.len(), s.erosion.clone()))
        .collect();
    assert_eq!(before, after);

    // Levels other than 0 are ignored, like the original.
    main.load_grid_square(&mut chunk, 0, 0, 1, &mut sprites, &NoEdges, &mut rand, None);

    // After a reload (no in-memory state) squares come back from the saved
    // records without their RandLocation.
    main.chunk_unloaded(2, 3);
    assert!(main.square_data(16, 24, 0).is_none());
    main.e_ticks += 50;
    main.update_chunk_now(&mut chunk, &mut sprites, &NoEdges, &mut rand, None);
    let mem = main.square_data(16 + 3, 24 + 4, 0).unwrap();
    assert!(mem.rand.is_none());
    assert_eq!(chunk.erosion.as_ref().unwrap().e_tick_stamp, 110);
}

#[test]
fn erosion_square_controls() {
    let tables = synthetic_tables();
    let (mut sprites, mut chunk) = setup();
    let mut main = ErosionMain::new(
        tables,
        ErosionSettings {
            static_rand: true,
            ..Default::default()
        },
        &mut sprites,
        None,
        None,
    );
    let mut rand = Lcg(1);
    main.load_grid_square(&mut chunk, 3, 4, 0, &mut sprites, &NoEdges, &mut rand, None);
    main.disable_erosion(&mut chunk, 3, 4, 0);
    assert!(
        chunk
            .get_grid_square(3, 4, 0)
            .unwrap()
            .erosion
            .as_ref()
            .unwrap()
            .do_nothing
    );
    main.reset_square(&mut chunk, 3, 4, 0);
    let s = chunk
        .get_grid_square(3, 4, 0)
        .unwrap()
        .erosion
        .clone()
        .unwrap();
    assert!(!s.init && s.regions.is_empty());
}

#[test]
fn main_timer_advances() {
    let tables = synthetic_tables();
    let (mut sprites, _) = setup();
    let dir = std::env::temp_dir().join(format!("pz-erosion-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("erosion.ini");
    let settings = ErosionSettings {
        save_config: Some(cfg.clone()),
        ..Default::default()
    };
    let mut main = ErosionMain::new(tables.clone(), settings.clone(), &mut sprites, None, None);
    for _ in 0..144 {
        main.main_timer(8, 6, 1993);
    }
    assert_eq!(main.get_etick(), 1);
    main.main_timer(9, 6, 1993);
    assert_eq!(main.epoch, 1);
    // The config written to the save is read back on the next start.
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("time.eticks = 1\n"));
    let again = ErosionMain::new(tables, settings, &mut sprites, None, None);
    assert_eq!((again.get_etick(), again.cfg.time.epoch), (1, 1));
    std::fs::remove_dir_all(&dir).ok();
}
