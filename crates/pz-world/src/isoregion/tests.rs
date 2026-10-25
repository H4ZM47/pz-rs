//! Region building on hand-built chunk flags and squares.

use super::data::{BIT_HAS_FLOOR, BIT_PATH_WALL_N, BIT_STAIRCASE, BIT_WALL_N, BIT_WALL_W};
use super::*;
use crate::objects::{IsoObject, ObjectKind};
use crate::sprite::{test_tables, PropertyValueMap};

const F: i8 = BIT_HAS_FLOOR;

/// One level of flags, indexed `y * 8 + x`.
type Level = [i8; 64];

/// A region-file block for chunk `(cx, cy)` with `levels` (level 0 first).
fn block(cx: i32, cy: i32, levels: &[Level]) -> Vec<u8> {
    let mut b = Vec::new();
    put_i32(&mut b, 0);
    put_i32(&mut b, REGION_WORLD_VERSION);
    put_i32(&mut b, cx);
    put_i32(&mut b, cy);
    put_i32(&mut b, 12 + 64 * levels.len() as i32);
    put_i32(&mut b, levels.len() as i32 - 1);
    put_i32(&mut b, 64 * levels.len() as i32);
    for l in levels {
        b.extend(l.iter().map(|&f| f as u8));
    }
    let n = b.len() as i32;
    set_i32(&mut b, 0, n);
    b
}

fn load(r: &mut IsoRegions, chunks: &[(i32, i32, Vec<Level>)]) {
    let mut j = JobChunkUpdate::new();
    for (cx, cy, levels) in chunks {
        assert_eq!(j.add_chunk_from_file(&block(*cx, *cy, levels)), Some(true));
    }
    r.job_queue.push_back(RegionJob::ChunkUpdate(j));
    r.apply_chunk_changes(false);
    r.worker_tick();
}

fn open_level(f: i8) -> Level {
    [f; 64]
}

/// Floor everywhere, with a room of squares `x0..x1`, `y0..y1` walled in
/// (north walls on its top row and below its bottom row, west walls on
/// its left column and right of its right column).
fn room_level(x0: i32, y0: i32, x1: i32, y1: i32) -> Level {
    let mut l = open_level(F);
    for x in x0..x1 {
        l[(y0 * 8 + x) as usize] |= BIT_WALL_N;
        if y1 < 8 {
            l[(y1 * 8 + x) as usize] |= BIT_WALL_N;
        }
    }
    for y in y0..y1 {
        l[(y * 8 + x0) as usize] |= BIT_WALL_W;
        if x1 < 8 {
            l[(y * 8 + x1) as usize] |= BIT_WALL_W;
        }
    }
    l
}

fn roof_over(x0: i32, y0: i32, x1: i32, y1: i32) -> Level {
    let mut l = [0i8; 64];
    for y in y0..y1 {
        for x in x0..x1 {
            l[(y * 8 + x) as usize] = F;
        }
    }
    l
}

#[test]
fn hash_and_dirs() {
    assert_eq!(hash(3, 2), (2 << 16) ^ 3);
    assert_eq!(hash(-1, 0), -1);
    for d in 0..6 {
        assert_eq!(get_opposite_dir(get_opposite_dir(d)), d);
    }
    assert_eq!(get_opposite_dir(7), -1);
    assert_eq!(chunk_file_name(12, -3), "datachunk_12_-3.bin");
}

#[test]
fn open_chunk_is_one_open_region() {
    let mut r = IsoRegions::new(13);
    load(&mut r, &[(10, 10, vec![open_level(F)])]);
    let cr = r.get_chunk_region(80, 80, 0).unwrap();
    let c = r.chunk_region(cr);
    assert_eq!(c.square_size(), 64);
    // 7 edge squares per side plus corners: 32 square sides on the border.
    assert_eq!(c.chunk_border_squares_cnt(), 32);
    // No neighbour chunks: every side is open.
    assert!(!r.chunk_region_is_enclosed(cr));
    let w = r.get_iso_world_region(87, 87, 0).unwrap();
    assert_eq!(Some(w), c.iso_world_region());
    assert_eq!(r.world_region(w).square_size(), 64);
    assert!(!r.is_enclosed(w) && !r.is_fog_mask(w));
    assert!(r.neighbors(w).is_empty());
    assert_eq!(r.get_square_flags(81, 82, 0), F);
    assert_eq!(r.get_square_flags(81, 82, 1), -1);
    assert_eq!(r.get_iso_world_region(88, 80, 0), None);
    assert_eq!(r.manager().chunk_region_count(), 1);
    assert_eq!(r.manager().world_region_count(), 1);
    // Ids: chunk region 0, world region 1, from one counter.
    assert_eq!(c.id(), 0);
    assert_eq!(r.world_region(w).id(), 1);
}

#[test]
fn walled_roofed_room_is_a_fog_mask() {
    let mut r = IsoRegions::new(13);
    load(
        &mut r,
        &[(0, 0, vec![room_level(2, 2, 5, 6), roof_over(2, 2, 5, 6)])],
    );
    let room = r.get_iso_world_region(3, 3, 0).unwrap();
    let outside = r.get_iso_world_region(0, 0, 0).unwrap();
    assert_ne!(room, outside);
    assert_eq!(r.world_region(room).square_size(), 12);
    assert_eq!(r.world_region(outside).square_size(), 52);
    assert!(r.is_enclosed(room));
    assert_eq!(r.world_region(room).roof_cnt(), 12);
    assert!(r.world_region(room).is_fully_roofed());
    assert!(r.is_fog_mask(room) && r.is_player_room(room));
    assert!(!r.is_fog_mask(outside));
    assert_eq!(r.neighbors(room), &[outside]);
    assert_eq!(r.neighbors(outside), &[room]);
    // The roof level's squares are their own regions.
    let up = r.get_iso_world_region(3, 3, 1).unwrap();
    assert!(!r.is_enclosed(up));
}

#[test]
fn room_without_full_roof_is_not_a_fog_mask() {
    let mut r = IsoRegions::new(13);
    load(
        &mut r,
        &[(0, 0, vec![room_level(2, 2, 5, 6), roof_over(2, 2, 5, 5)])],
    );
    let room = r.get_iso_world_region(3, 3, 0).unwrap();
    assert!(r.is_enclosed(room));
    assert_eq!(r.world_region(room).roof_cnt(), 9);
    assert!((r.world_region(room).roofed_percentage() - 0.75).abs() < 1e-6);
    assert!(!r.is_fog_mask(room));
}

#[test]
fn stairs_count_as_roofed() {
    // (3, 3) has nothing above it: unroofed, unless it is a staircase.
    let l0 = room_level(2, 2, 5, 6);
    let mut l1 = roof_over(2, 2, 5, 6);
    l1[3 * 8 + 3] = 0;
    let mut r = IsoRegions::new(13);
    load(&mut r, &[(0, 0, vec![l0, l1])]);
    let room = r.get_iso_world_region(3, 3, 0).unwrap();
    assert_eq!(r.world_region(room).roof_cnt(), 11);
    assert!(!r.is_fog_mask(room));
    let mut l0s = l0;
    l0s[3 * 8 + 3] |= BIT_STAIRCASE;
    let mut r = IsoRegions::new(13);
    load(&mut r, &[(0, 0, vec![l0s, l1])]);
    let room = r.get_iso_world_region(3, 3, 0).unwrap();
    assert_eq!(r.world_region(room).roof_cnt(), 12);
    assert!(r.is_fog_mask(room));
}

#[test]
fn rooms_join_across_chunk_edges() {
    // A room from x = 5 in chunk 0 to x = 3 in chunk 1 (rows 2..6).
    let mut a = open_level(F);
    for x in 5..8 {
        a[2 * 8 + x] |= BIT_WALL_N;
        a[6 * 8 + x] |= BIT_WALL_N;
    }
    for y in 2..6 {
        a[y * 8 + 5] |= BIT_WALL_W;
    }
    let mut b = open_level(F);
    for x in 0..3 {
        b[2 * 8 + x] |= BIT_WALL_N;
        b[6 * 8 + x] |= BIT_WALL_N;
    }
    for y in 2..6 {
        b[y * 8 + 3] |= BIT_WALL_W;
    }
    let mut r = IsoRegions::new(13);
    load(&mut r, &[(0, 0, vec![a]), (1, 0, vec![b])]);
    let w0 = r.get_iso_world_region(6, 3, 0).unwrap();
    let w1 = r.get_iso_world_region(9, 3, 0).unwrap();
    assert_eq!(w0, w1);
    assert_eq!(r.world_region(w0).square_size(), 24);
    assert_eq!(r.world_region(w0).size(), 2);
    assert!(r.is_enclosed(w0));
    let c0 = r.get_chunk_region(6, 3, 0).unwrap();
    let c1 = r.get_chunk_region(9, 3, 0).unwrap();
    assert_eq!(r.chunk_region(c0).connected_neighbors(), &[c1]);
    // A wall on the chunk edge splits it: west walls on chunk 1's x = 0.
    let mut b2 = b;
    for y in 2..6 {
        b2[y * 8] |= BIT_WALL_W;
    }
    let mut r = IsoRegions::new(13);
    load(&mut r, &[(0, 0, vec![a]), (1, 0, vec![b2])]);
    let w0 = r.get_iso_world_region(6, 3, 0).unwrap();
    let w1 = r.get_iso_world_region(9, 3, 0).unwrap();
    assert_ne!(w0, w1);
    assert!(r.neighbors(w0).contains(&w1));
    assert!(r.is_enclosed(w0) && r.is_enclosed(w1));
}

#[test]
fn chunks_loaded_later_link_and_reenclose() {
    let mut r = IsoRegions::new(13);
    load(&mut r, &[(0, 0, vec![open_level(F)])]);
    let w = r.get_iso_world_region(0, 0, 0).unwrap();
    assert!(!r.is_enclosed(w));
    // A second chunk east of it: the open regions merge into one world
    // region spanning both.
    load(&mut r, &[(1, 0, vec![open_level(F)])]);
    let w0 = r.get_iso_world_region(0, 0, 0).unwrap();
    let w1 = r.get_iso_world_region(8, 0, 0).unwrap();
    assert_eq!(w0, w1);
    assert_eq!(r.world_region(w0).square_size(), 128);
    assert_eq!(r.manager().world_region_count(), 1);
}

#[test]
fn square_updates_rebuild_the_chunk() {
    let mut r = IsoRegions::new(13);
    load(
        &mut r,
        &[(0, 0, vec![room_level(2, 2, 5, 6), roof_over(2, 2, 5, 6)])],
    );
    r.discovered_chunks.push(hash(0, 0));
    let room = r.get_iso_world_region(3, 3, 0).unwrap();
    assert!(r.is_fog_mask(room));
    // Knock out the north wall of (3, 2): the room opens to the outside.
    let cell = IsoCell::default();
    let m = manager();
    r.add_square_changed_job(&cell, &m, 3, 2, 0, true, F);
    assert!(matches!(
        r.job_queue.front(),
        Some(RegionJob::SquareUpdate {
            x: 3,
            y: 2,
            z: 0,
            flags: F
        })
    ));
    assert!(r.update(&cell, &m, None));
    let a = r.get_iso_world_region(3, 3, 0).unwrap();
    let b = r.get_iso_world_region(0, 0, 0).unwrap();
    assert_eq!(a, b);
    assert!(!r.is_enclosed(a));
    assert_eq!(r.get_square_flags(3, 2, 0), F);
    // Unchanged flags make no change.
    r.add_square_changed_job(&cell, &m, 3, 2, 0, true, F);
    let before = r.manager().chunk_region_count();
    r.update(&cell, &m, None);
    assert_eq!(r.manager().chunk_region_count(), before);
}

#[test]
fn ids_are_reused_last_released_first() {
    let mut r = IsoRegions::new(13);
    load(&mut r, &[(0, 0, vec![open_level(F)])]);
    // Reloading the chunk releases chunk region 0 and world region 1 and
    // takes the ids back from the stack.
    load(&mut r, &[(0, 0, vec![open_level(F)])]);
    let cr = r.get_chunk_region(0, 0, 0).unwrap();
    let w = r.get_iso_world_region(0, 0, 0).unwrap();
    let ids = (r.chunk_region(cr).id(), r.world_region(w).id());
    assert!(ids == (0, 1) || ids == (1, 0), "{ids:?}");
    assert_eq!(r.manager().chunk_region_count(), 1);
    assert_eq!(r.manager().world_region_count(), 1);
}

#[test]
fn data_chunk_save_matches_its_block() {
    let levels = vec![room_level(2, 2, 5, 6), roof_over(2, 2, 5, 6)];
    let mut r = IsoRegions::new(13);
    load(&mut r, &[(4, 5, levels.clone())]);
    let mut j = JobChunkUpdate::new();
    j.add_chunk_from_data_chunk(r.get_data_chunk(4, 5).unwrap());
    let files = j.chunk_files();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, 4);
    assert_eq!(files[0].1, 5);
    assert_eq!(files[0].2, &block(4, 5, &levels)[..]);
    // The job header: total length and count.
    assert_eq!(get_i32(j.bytes(), 0), Some(j.bytes().len() as i32));
    assert_eq!(get_i32(j.bytes(), 4), Some(1));
}

#[test]
fn job_buffers_fill_up() {
    let mut j = JobChunkUpdate::new();
    let b = block(0, 0, &[open_level(F); 32]);
    assert_eq!(b.len(), SINGLE_CHUNK_PACKET_SIZE);
    let mut n = 0;
    while j.can_add_chunk() {
        assert_eq!(j.add_chunk_from_file(&b), Some(true));
        n += 1;
    }
    assert_eq!(n, 31);
    assert_eq!(j.chunk_count(), 31);
    assert_eq!(j.add_chunk_from_file(&b[..10]), None);
}

#[test]
fn load_keeps_higher_levels_already_there() {
    let mut r = IsoRegions::new(13);
    load(
        &mut r,
        &[(0, 0, vec![open_level(F), roof_over(0, 0, 8, 8)])],
    );
    load(&mut r, &[(0, 0, vec![open_level(F)])]);
    let c = r.get_data_chunk(0, 0).unwrap();
    // `highestZ` is the loaded value; level 1 stays active with its old
    // flags.
    assert_eq!(c.highest_z(), 0);
    assert!(c.is_active(1));
    assert_eq!(c.get_square_checked(0, 0, 1, true), F);
}

#[test]
fn path_walls_still_block() {
    let mut l = open_level(F);
    for x in 0..8 {
        l[4 * 8 + x] |= BIT_WALL_N | BIT_PATH_WALL_N;
    }
    let mut r = IsoRegions::new(13);
    load(&mut r, &[(0, 0, vec![l])]);
    assert_ne!(r.get_chunk_region(0, 0, 0), r.get_chunk_region(0, 4, 0));
}

fn manager() -> IsoSpriteManager {
    let specs: &[(&str, &str, &[&str])] = &[
        ("floor", "normal", &["solidfloor"]),
        ("wall_n", "wall", &["cutN"]),
        ("wall_w", "wall", &["cutW"]),
        ("wall_nw_se", "wall", &["cutW", "WallSE"]),
        ("window_n", "windowFN", &["cutN", "windowN"]),
        ("frame_w", "wall", &["cutW", "WindowW"]),
        ("doorframe_n", "doorFrN", &[]),
        ("stairs", "stairsMW", &[]),
    ];
    let mut map = PropertyValueMap::default();
    for k in [
        "WindowN",
        "WindowW",
        "DoorWallN",
        "DoorWallW",
        "WallSE",
        "tree",
        "lightR",
        "lightG",
        "lightB",
        "ItemHeight",
        "Surface",
    ] {
        map.entry(k);
    }
    for (_, _, props) in specs {
        for k in *props {
            let e = map.entry(k);
            if !e.iter().any(|x| x.is_empty()) {
                e.push(String::new());
            }
        }
    }
    let mut m = IsoSpriteManager::new(test_tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (i, (name, ty, props)) in specs.iter().enumerate() {
        let s = m.add_sprite(name, 100 + i as i32);
        let t = m.tables.object_type_from_string(ty);
        let (tables, alias) = (m.tables.clone(), m.alias.clone());
        let sp = &mut m.sprites[s];
        sp.name = Some(name.to_string());
        sp.tile_type = t;
        for k in *props {
            sp.properties.set(&tables, &alias, k, "", true);
        }
    }
    m
}

fn square(m: &IsoSpriteManager, x: i32, y: i32, z: i32, objs: &[&str]) -> IsoGridSquare {
    let mut sq = IsoGridSquare::new(x, y, z);
    for o in objs {
        let id = sq.new_object_id();
        sq.objects.push(IsoObject::new(
            id,
            ObjectKind::Object,
            Some(m.named_map[*o]),
        ));
    }
    sq.recalc_properties(m);
    sq
}

#[test]
fn square_flags_from_properties() {
    let m = manager();
    let fl = |objs: &[&str]| calculate_square_flags(Some(&square(&m, 0, 0, 0, objs)), &m);
    assert_eq!(calculate_square_flags(None, &m), 0);
    assert_eq!(fl(&[]), 0);
    assert_eq!(fl(&["floor"]), F);
    assert_eq!(fl(&["floor", "wall_n"]), F | BIT_WALL_N);
    assert_eq!(fl(&["wall_w"]), BIT_WALL_W);
    // `WallSE` cancels the west wall.
    assert_eq!(fl(&["wall_nw_se"]), 0);
    assert_eq!(fl(&["window_n"]), BIT_WALL_N | BIT_PATH_WALL_N);
    assert_eq!(fl(&["frame_w"]), BIT_WALL_W | data::BIT_PATH_WALL_W);
    assert_eq!(fl(&["doorframe_n"]), BIT_WALL_N);
    assert_eq!(fl(&["floor", "stairs"]), F | BIT_STAIRCASE);
}

/// A cell with chunks `(0, 0)` and `(1, 0)`: floor everywhere and, in
/// chunk 0, a walled room `(2..5, 2..6)` with a floor above it.
fn cell(m: &IsoSpriteManager) -> IsoCell {
    let mut cell = IsoCell::default();
    for wx in 0..2 {
        let mut ch = IsoChunk::new(wx, 0);
        for y in 0..8 {
            for x in 0..8 {
                let (gx, gy) = (wx * 8 + x, y);
                let mut objs = vec!["floor"];
                if wx == 0 && (2..5).contains(&x) && (y == 2 || y == 6) {
                    objs.push("wall_n");
                }
                if wx == 0 && (2..6).contains(&y) && (x == 2 || x == 5) {
                    objs.push("wall_w");
                }
                ch.set_square(x, y, 0, Some(square(m, gx, gy, 0, &objs)));
                if wx == 0 && (2..5).contains(&x) && (2..6).contains(&y) {
                    ch.set_square(x, y, 1, Some(square(m, gx, gy, 1, &["floor"])));
                }
            }
        }
        cell.chunks.insert((wx, 0), ch);
    }
    cell
}

#[test]
fn square_changes_read_the_surrounding_chunks() {
    let m = manager();
    let mut cell = cell(&m);
    let mut r = IsoRegions::new(13);
    let sq = cell.get_grid_square(3, 3, 0).unwrap();
    assert_eq!(sq.get_iso_world_region(&r), None);
    // First change in an unseen chunk: both loaded chunks are read.
    r.set_previous_flags(None, &m);
    r.square_changed(&cell, &m, 3, 3, 0, false);
    assert_eq!(r.discovered_chunks(), &[hash(0, 0), hash(1, 0)]);
    // Still cached as "none" until the update drops the cache.
    assert_eq!(sq.get_iso_world_region(&r), None);
    assert!(r.update(&cell, &m, None));
    let sq = cell.get_grid_square(3, 3, 0).unwrap();
    let room = sq.get_iso_world_region(&r).unwrap();
    assert!(r.is_fog_mask(room));
    assert!(sq.is_in_a_room(&crate::IsoMetaGrid::empty(), &r));
    assert_eq!(sq.player_room_size(&r), Some(12));
    let out = cell.get_grid_square(12, 3, 0).unwrap();
    assert!(!r.is_fog_mask(out.get_iso_world_region(&r).unwrap()));

    // Remove the wall of (3, 2): a square update of a seen chunk.
    let g = cell.get_grid_square(3, 2, 0).unwrap();
    r.set_previous_flags(Some(g), &m);
    let g = square(&m, 3, 2, 0, &["floor"]);
    cell.chunks
        .get_mut(&(0, 0))
        .unwrap()
        .set_square(3, 2, 0, Some(g));
    r.square_changed(&cell, &m, 3, 2, 0, true);
    assert!(matches!(
        r.job_queue.front(),
        Some(RegionJob::SquareUpdate {
            x: 3,
            y: 2,
            z: 0,
            ..
        })
    ));
    r.update(&cell, &m, None);
    let a = cell
        .get_grid_square(3, 3, 0)
        .unwrap()
        .get_iso_world_region(&r);
    let b = cell
        .get_grid_square(12, 3, 0)
        .unwrap()
        .get_iso_world_region(&r);
    assert_eq!(a, b);
}

#[test]
fn saves_and_loads_region_files() {
    let m = manager();
    let cell = cell(&m);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/isoregion-test")
        .join(format!("save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = IsoRegions::init(Some(&dir), 13);
    let rd = dir.join(FILE_DIR);
    assert!(rd.is_dir());
    r.read_surrounding_chunks(&cell, &m, 0, 0, 7, true, false);
    r.update(&cell, &m, None);
    let header = std::fs::read(rd.join(HEADER_FILE)).unwrap();
    let mut want = Vec::new();
    for v in [REGION_WORLD_VERSION, 2, hash(0, 0), hash(1, 0)] {
        put_i32(&mut want, v);
    }
    assert_eq!(header, want);
    let f0 = std::fs::read(rd.join(chunk_file_name(0, 0))).unwrap();
    let mut j = JobChunkUpdate::new();
    j.add_chunk_from_iso_chunk(&cell.chunks[&(0, 0)], &m);
    assert_eq!(f0, j.chunk_files()[0].2);
    assert_eq!(get_i32(&f0, 20), Some(1));

    // A new session loads them back into the same regions.
    let mut r2 = IsoRegions::init(Some(&dir), 13);
    assert_eq!(r2.discovered_chunks().len(), 2);
    r2.update(&IsoCell::default(), &m, None);
    for (x, y, z) in [(3, 3, 0), (0, 0, 0), (12, 7, 0), (3, 3, 1)] {
        assert_eq!(r.get_square_flags(x, y, z), r2.get_square_flags(x, y, z));
        let (a, b) = (
            r.get_iso_world_region(x, y, z).unwrap(),
            r2.get_iso_world_region(x, y, z).unwrap(),
        );
        assert_eq!(
            r.world_region(a).square_size(),
            r2.world_region(b).square_size()
        );
        assert_eq!(r.is_fog_mask(a), r2.is_fog_mask(b));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn debug_load_follows_the_player() {
    let m = manager();
    let cell = cell(&m);
    let mut r = IsoRegions::new(13);
    r.debug_load_all_chunks = true;
    r.update(&cell, &m, Some((3.5, 3.5)));
    assert_eq!(r.discovered_chunks().len(), 2);
    r.update(&cell, &m, Some((3.5, 3.5)));
    assert!(r.get_iso_world_region(3, 3, 0).is_some());
}

#[test]
fn loaded_windows_make_their_walls_cut() {
    // A saved window whose closed sprite has no cut flag of its own: the
    // `IsoWindow.load` changes make its square a west wall with a window.
    let mut m = manager();
    let spr = m.named_map["floor"];
    let open = m.named_map["stairs"];
    let mut sq = IsoGridSquare::new(1, 1, 0);
    let id = sq.new_object_id();
    sq.objects.push(IsoObject::new(
        id,
        ObjectKind::Window(crate::objects::IsoWindow {
            closed_sprite: Some(spr),
            open_sprite: Some(open),
            ..Default::default()
        }),
        Some(spr),
    ));
    sq.recalc_properties(&m);
    assert_eq!(calculate_square_flags(Some(&sq), &m), F);
    let mut ch = IsoChunk::new(0, 0);
    ch.set_square(1, 1, 0, Some(sq));
    crate::save::chunk_file::apply_loaded_window_sprites(&ch, &mut m);
    let mut sq = ch.get_grid_square(1, 1, 0).unwrap().clone();
    sq.recalc_properties(&m);
    // The open sprite now has the closed sprite's flags and tile type.
    assert_eq!(
        calculate_square_flags(Some(&sq), &m),
        F | BIT_WALL_W | data::BIT_PATH_WALL_W
    );
    assert!(m.sprites[open].properties.has(m.tables.flag.cutW));
    assert_eq!(m.sprites[open].tile_type, m.sprites[spr].tile_type);
}
