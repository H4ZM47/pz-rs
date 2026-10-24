//! `LoadBrandNew` order and the world-generation hook, on an empty map.

use super::*;
use crate::fix2x::Fix2xTable;
use crate::java::GameRand;
use crate::objects::{NoHooks, ObjectLoadSettings, ObjectTables};
use crate::sprite::{test_tables, IsoSpriteManager};

#[test]
fn corner_of_generation() {
    let got: Vec<i32> = (-4..=4).map(get_corner_of_generation).collect();
    assert_eq!(got, [-4, -4, -2, -2, 0, 0, 2, 2, 4]);
    assert_eq!(get_corner_of_generation(i32::MIN), i32::MIN);
    assert_eq!(get_corner_of_generation(i32::MAX), i32::MAX - 1);
}

#[test]
fn chunks_cache_bounds_and_local_squares() {
    let mut c = ChunksCache::default();
    assert_eq!((c.min_chunk_x(), c.max_chunk_x()), (i32::MAX, i32::MIN));
    for (x, y) in [(-2, 4), (-2, 5), (-1, 4), (-1, 5)] {
        let mut ch = IsoChunk::new(x, y);
        ch.set_square(1, 2, 0, Some(IsoGridSquare::new(x * 8 + 1, y * 8 + 2, 0)));
        c.put(x, y, ch);
    }
    assert_eq!(c.coords(), [(-2, 4), (-2, 5), (-1, 4), (-1, 5)]);
    assert_eq!((c.min_chunk_x(), c.max_chunk_x()), (-2, -1));
    assert_eq!((c.min_chunk_y(), c.max_chunk_y()), (4, 5));
    assert_eq!((c.min_tile_x(), c.max_tile_x()), (-16, 0));
    assert_eq!((c.min_tile_y(), c.max_tile_y()), (32, 48));
    let sq = c.get_grid_square(9, 10, 0).unwrap();
    assert_eq!((sq.x, sq.y), (-7, 42));
    assert!(c.get_grid_square(9, 11, 0).is_none());
    assert!(c.get_grid_square(16, 2, 0).is_none());
}

struct NoRand;
impl GameRand for NoRand {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

/// Records the calls and marks the chunks it touches.
#[derive(Default)]
struct Gen {
    worldgen: bool,
    zombies: bool,
    log: Vec<String>,
}

impl BrandNewHook for Gen {
    fn generate_chunks(
        &mut self,
        chunks: &mut ChunksCache,
        _: &IsoMetaGrid,
        _: &mut ChunkLoadEnv<'_>,
    ) {
        self.log.push(format!("generate {:?}", chunks.coords()));
        for x in chunks.min_chunk_x()..chunks.min_chunk_x() + 2 {
            for y in chunks.min_chunk_y()..chunks.min_chunk_y() + 2 {
                let ch = chunks.get_mut(x, y).unwrap();
                ch.set_min_max_level(0, 1);
            }
        }
    }

    fn on_new_chunk_loaded(
        &mut self,
        chunk: &mut IsoChunk,
        _: &IsoMetaGrid,
        _: &mut ChunkLoadEnv<'_>,
    ) {
        self.log
            .push(format!("basements {},{}", chunk.wx, chunk.wy));
        chunk.set_min_max_level(-1, chunk.max_level);
    }

    fn do_worldgen(&self) -> bool {
        self.worldgen
    }

    fn add_zombie_on_cell_load(&self) -> bool {
        self.zombies
    }
}

fn run(wx: i32, wy: i32, gen: Option<&mut dyn BrandNewHook>) -> (IsoChunk, Vec<IsoChunk>) {
    let mut sprites = IsoSpriteManager::new(test_tables());
    let fix2x = Fix2xTable::default();
    let tables = ObjectTables::new(vec![1; 8], vec![(0, 0)]).unwrap();
    let mut env = ChunkLoadEnv {
        sprites: &mut sprites,
        fix2x: &fix2x,
        rand: &mut NoRand,
        tables: &tables,
        settings: ObjectLoadSettings::default(),
        hooks: &mut NoHooks,
    };
    load_brand_new(&IsoMetaGrid::empty(), wx, wy, &mut env, gen)
}

#[test]
fn load_brand_new_order_and_hand_off() {
    let mut g = Gen {
        worldgen: true,
        ..Gen::default()
    };
    let (ch, others) = run(3, -5, Some(&mut g));
    assert_eq!(
        g.log,
        [
            "generate [(2, -6), (2, -5), (3, -6), (3, -5)]",
            "basements 3,-5"
        ]
    );
    assert_eq!((ch.wx, ch.wy), (3, -5));
    assert_eq!((ch.min_level, ch.max_level), (-1, 1));
    assert!(!ch.add_zombies);
    let coords: Vec<_> = others
        .iter()
        .map(|c| (c.wx, c.wy, c.min_level, c.max_level))
        .collect();
    // World generation's changes to the other chunks are handed back.
    assert_eq!(coords, [(2, -6, 0, 1), (2, -5, 0, 1), (3, -6, 0, 1)]);
}

#[test]
fn load_brand_new_without_worldgen() {
    let mut g = Gen {
        zombies: true,
        ..Gen::default()
    };
    let (ch, others) = run(0, 1, Some(&mut g));
    assert_eq!(g.log, ["basements 0,1"]);
    assert!(ch.add_zombies);
    assert_eq!(others.len(), 3);
    let (ch, others) = run(-1, -1, None);
    assert_eq!((ch.wx, ch.wy, ch.min_level, ch.max_level), (-1, -1, 0, 0));
    assert!(ch.add_zombies);
    let coords: Vec<_> = others.iter().map(|c| (c.wx, c.wy)).collect();
    assert_eq!(coords, [(-2, -2), (-2, -1), (-1, -2)]);
}
