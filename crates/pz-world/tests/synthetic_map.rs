//! Loads a small hand-built map directory (authored test data, not game data).

use std::path::Path;

use pz_fs::GameFs;
use pz_world::chunk::{ChunkLoadEnv, IsoCell, SpriteLookup};
use pz_world::fix2x::Fix2xTable;
use pz_world::{CreateParams, GameRand, IsoMetaGrid, WorldGenBounds};

struct Zero;
impl GameRand for Zero {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

struct Names;
impl SpriteLookup for Names {
    fn sprite_id(&self, name: &str) -> Option<i32> {
        match name {
            "floor_a" => Some(1),
            "wall_b" => Some(2),
            _ => None,
        }
    }
}

fn ints(v: &mut Vec<u8>, xs: &[i32]) {
    for x in xs {
        v.extend(x.to_le_bytes());
    }
}

fn write_map(root: &Path) {
    let dir = root.join("media/maps/Test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("map.info"), "title=Test\nfixed2x=true\n").unwrap();

    // Header for cell 1,2: three tiles, levels 0..1, one room in one building.
    let mut h = b"LOTH".to_vec();
    ints(&mut h, &[1, 3]);
    h.extend(b"floor_a\nwall_b\nmissing_c\n");
    ints(&mut h, &[8, 8, 0, 1]);
    ints(&mut h, &[1]);
    h.extend(b"kitchen\n");
    // level 0, one rect at cell-local (3,4) 2x2, one meta object.
    ints(&mut h, &[0, 1, 3, 4, 2, 2, 1, 7, 4, 5]);
    ints(&mut h, &[1, 1, 0]);
    h.extend((0..1024).map(|i| (i % 7) as u8));
    std::fs::write(dir.join("1_2.lotheader"), h).unwrap();

    // Lotpack: every chunk table entry points at the same chunk body.
    let mut p = b"LOTP".to_vec();
    ints(&mut p, &[1, 0]);
    let body = 8 + 4 + 1024 * 8;
    for _ in 0..1024 {
        ints(&mut p, &[body, 0]);
    }
    // Level 0 squares in x-major order (x outer, y inner):
    // (0,0) floor+wall, (0,1) only a room id (count 1), then skip 61,
    // (7,7) missing tile. Level 1: skip all 64.
    ints(&mut p, &[3, 0, 0, 1]);
    ints(&mut p, &[1]);
    ints(&mut p, &[-1, 61]);
    ints(&mut p, &[2, 0, 2]);
    ints(&mut p, &[-1, 64]);
    std::fs::write(dir.join("world_1_2.lotpack"), p).unwrap();
}

#[test]
fn loads_header_rooms_and_chunk() {
    let root = std::env::temp_dir().join(format!("pz-world-synthetic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write_map(&root);
    let fs = GameFs::new(&root);
    let grid = IsoMetaGrid::create(
        &fs,
        CreateParams {
            game_map: "Test",
            world_gen: WorldGenBounds::default(),
            rand: &mut Zero,
            voronois: Vec::new(),
        },
    )
    .unwrap();

    let header = grid.map_files[0].get_lot_header(1, 2).unwrap();
    assert_eq!(header.tiles_used, vec!["floor_a", "wall_b", "missing_c"]);
    assert_eq!((header.min_level, header.max_level), (0, 1));
    // Stored column-major in the file, row-major in the header.
    assert_eq!(header.get_zombie_intensity(1), (32 % 7) as u8);

    let (rx, ry) = (256 + 3, 512 + 4);
    let room = grid.get_room_at(rx, ry, 0).expect("room");
    assert_eq!(room.name, "kitchen");
    assert_eq!(room.objects[0].x, 4 + 256 - rx);
    assert!(grid.get_room_at(rx, ry, 1).is_none());
    assert_eq!(grid.buildings.len(), 1);

    let mut cell = IsoCell::default();
    let (wx, wy) = (32, 64);
    let ch = cell.load_chunk(
        &grid,
        wx,
        wy,
        &mut ChunkLoadEnv {
            sprites: &Names,
            fix2x: &Fix2xTable::default(),
            rand: &mut Zero,
        },
    );
    let sq = ch.get_grid_square(0, 0, 0).unwrap();
    let names: Vec<_> = sq.objects.iter().map(|o| o.sprite_name.as_str()).collect();
    assert_eq!(names, vec!["floor_a", "wall_b"]);
    // Neighbours of a used square are created empty; count-1 squares are not.
    assert!(ch.get_grid_square(1, 1, 0).unwrap().objects.is_empty());
    assert!(ch.get_grid_square(0, 1, 0).unwrap().objects.is_empty());
    // A square with only a missing tile exists but has no objects.
    assert!(ch.get_grid_square(7, 7, 0).unwrap().objects.is_empty());
    assert!(ch.get_grid_square(4, 4, 0).is_none());
    let _ = std::fs::remove_dir_all(&root);
}
