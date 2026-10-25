//! Square properties, matrices and edge queries on hand-built squares.

use super::*;
use crate::los::{line_clear, TestResults};
use crate::objects::ObjectId;
use crate::objects::{IsoDoor, IsoWindow};
use crate::sprite::{test_tables, PropertyValueMap};
use crate::IsoChunk;

/// Hand-authored sprites: name, tile type name, flags/properties (an empty
/// value sets the flag of that name).
type Spec<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

const SPRITES: &[Spec] = &[
    ("floor", "normal", &[("solidfloor", "")]),
    (
        "glass_floor",
        "normal",
        &[("solidfloor", ""), ("transparentFloor", "")],
    ),
    ("wall_n", "wall", &[("collideN", ""), ("cutN", "")]),
    ("wall_w", "wall", &[("collideW", ""), ("cutW", "")]),
    (
        "glass_n",
        "wall",
        &[("collideN", ""), ("cutN", ""), ("transparentN", "")],
    ),
    (
        "fence_n",
        "normal",
        &[("collideN", ""), ("HoppableN", ""), ("canPathN", "")],
    ),
    (
        "window_n",
        "windowFN",
        &[("windowN", ""), ("transparentN", ""), ("cutN", "")],
    ),
    ("door_n", "doorN", &[("doorN", ""), ("collideN", "")]),
    ("crate", "normal", &[("solid", "")]),
    ("seat", "normal", &[("solidtrans", "")]),
    ("stairs_tn", "stairsTN", &[]),
    ("stairs_mn", "stairsMN", &[]),
    ("stairs_bn", "stairsBN", &[]),
    ("thump_window_n", "normal", &[("WindowN", "")]),
    ("thump", "normal", &[]),
    ("frame_w", "normal", &[("WindowW", "")]),
    ("barricade", "normal", &[]),
    ("plan", "normal", &[("solid", ""), ("blueprint", "")]),
    (
        "water",
        "normal",
        &[("water", ""), ("solidfloor", ""), ("solidtrans", "")],
    ),
    (
        "ramp_n",
        "normal",
        &[
            ("solidfloor", ""),
            ("SlopedSurfaceDirection", "N"),
            ("SlopedSurfaceHeightMin", "0"),
            ("SlopedSurfaceHeightMax", "100"),
        ],
    ),
    ("fuel", "normal", &[("fuelAmount", "40")]),
];

fn manager() -> IsoSpriteManager {
    let mut map = PropertyValueMap::default();
    // Keys `SetCustomPropertyValues` expects.
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
    for (_, _, props) in SPRITES {
        for (k, v) in *props {
            let e = map.entry(k);
            if !e.iter().any(|x| x == v) {
                e.push(v.to_string());
            }
        }
    }
    let mut m = IsoSpriteManager::new(test_tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (i, (name, ty, props)) in SPRITES.iter().enumerate() {
        let s = m.add_sprite(name, 100 + i as i32);
        let t = m.tables.object_type_from_string(ty);
        let (tables, alias) = (m.tables.clone(), m.alias.clone());
        let sp = &mut m.sprites[s];
        sp.name = Some(name.to_string());
        sp.tile_type = t;
        for (k, v) in *props {
            sp.properties.set(&tables, &alias, k, v, v.is_empty());
        }
    }
    m
}

fn add(sq: &mut IsoGridSquare, m: &IsoSpriteManager, name: &str, kind: ObjectKind) {
    let id = sq.new_object_id();
    let special = !matches!(kind, ObjectKind::Object);
    sq.objects
        .push(IsoObject::new(id, kind, Some(m.named_map[name])));
    if special {
        sq.special_objects.push(id);
    }
}

fn square(m: &IsoSpriteManager, x: i32, y: i32, z: i32, objs: &[&str]) -> IsoGridSquare {
    let mut sq = IsoGridSquare::new(x, y, z);
    for o in objs {
        add(&mut sq, m, o, ObjectKind::Object);
    }
    sq.recalc_properties(m);
    sq
}

/// A cell with one chunk at (1, 1) (squares 8..16).
struct World {
    cell: IsoCell,
    m: IsoSpriteManager,
    meta: IsoMetaGrid,
}

impl World {
    fn new() -> World {
        let mut cell = IsoCell::default();
        cell.chunks.insert((1, 1), IsoChunk::new(1, 1));
        World {
            cell,
            m: manager(),
            meta: IsoMetaGrid::empty(),
        }
    }

    fn put(&mut self, x: i32, y: i32, z: i32, objs: &[&str]) -> &mut IsoGridSquare {
        let sq = square(&self.m, x, y, z, objs);
        let ch = self
            .cell
            .chunks
            .get_mut(&(x.div_euclid(8), y.div_euclid(8)))
            .unwrap();
        ch.set_square(x.rem_euclid(8), y.rem_euclid(8), z, Some(sq));
        self.cell.get_grid_square_mut(x, y, z).unwrap()
    }

    fn add(&mut self, x: i32, y: i32, z: i32, name: &str, kind: ObjectKind) {
        let sq = self.cell.get_grid_square_mut(x, y, z).unwrap();
        add(sq, &self.m, name, kind);
    }

    fn sq(&self, x: i32, y: i32, z: i32) -> &IsoGridSquare {
        self.cell.get_grid_square(x, y, z).unwrap()
    }

    fn recalc(&mut self) {
        let mut keys: Vec<_> = self.cell.chunks.keys().copied().collect();
        keys.sort();
        for &(wx, wy) in &keys {
            self.cell.recalc_chunk_squares(wx, wy, &self.meta, &self.m);
        }
        for &(wx, wy) in &keys {
            self.cell.load_in_main_thread(wx, wy, &self.meta, &self.m);
        }
    }
}

#[test]
fn recalc_merges_and_derives_flags() {
    let m = manager();
    let f = &m.tables.flag;
    let sq = square(&m, 3, 3, 0, &["floor", "glass_n", "wall_n"]);
    assert!(sq.has(f.solidfloor) && sq.has(f.collideN) && sq.has(f.cutN));
    // An opaque cut wall makes the square's north side opaque.
    assert!(!sq.has(f.transparentN));
    assert!(sq.has_type(m.tables.ty.wall));
    // No room and no roof: exterior.
    assert!(sq.is_exterior(&m));

    assert!(square(&m, 3, 3, 0, &["glass_n"]).has(f.transparentN));

    // A non-hoppable north wall cancels the fence's hoppable/canPath.
    let fence = square(&m, 3, 3, 0, &["fence_n"]);
    assert!(fence.has(f.HoppableN) && fence.has(f.canPathN));
    let walled = square(&m, 3, 3, 0, &["fence_n", "wall_n"]);
    assert!(!walled.has(f.HoppableN) && !walled.has(f.canPathN));

    // An opaque floor cancels another object's transparentFloor.
    assert!(square(&m, 3, 3, 0, &["glass_floor"]).has(f.transparentFloor));
    assert!(!square(&m, 3, 3, 0, &["glass_floor", "floor"]).has(f.transparentFloor));

    // Blueprint objects are skipped.
    assert!(!square(&m, 3, 3, 0, &["plan"]).has(f.solid));

    // A dry floor after the water drops the water's solidtrans; on its own
    // (or with the floor first, which the water resets) it stays.
    let w = square(&m, 3, 3, 0, &["water", "floor"]);
    assert!(w.has(f.water) && !w.has(f.solidtrans));
    assert!(w.has_floor_over_water(&m));
    assert!(square(&m, 3, 3, 0, &["water"]).has(f.solidtrans));
    assert!(square(&m, 3, 3, 0, &["floor", "water"]).has(f.solidtrans));

    // A room clears exterior.
    let mut room = IsoGridSquare::new(3, 3, 0);
    room.room_id = 7;
    room.recalc_properties(&m);
    assert!(!room.is_exterior(&m));

    // Sloped surface values come from the merged container.
    let ramp = square(&m, 3, 3, 0, &["ramp_n"]);
    assert_eq!(ramp.sloped_surface_direction, Some(IsoDirections::N));
    assert!(ramp.has_sloped_surface_to_level_above(IsoDirections::N));
    assert!(ramp.treat_as_solid_floor(&m));
}

#[test]
fn recalc_keeps_fuel_amount() {
    let m = manager();
    let mut sq = square(&m, 3, 3, 0, &["fuel"]);
    assert_eq!(sq.properties.get(&m.alias, "fuelAmount"), Some("40"));
    sq.objects.clear();
    sq.recalc_properties(&m);
    assert_eq!(sq.properties.get(&m.alias, "fuelAmount"), Some("40"));
}

#[test]
fn roof_clears_exterior_below() {
    let mut w = World::new();
    w.put(10, 10, 0, &["floor"]);
    w.put(10, 10, 1, &["floor"]);
    w.put(11, 10, 0, &["floor"]);
    w.recalc();
    let m = &w.m;
    assert!(w.sq(10, 10, 0).have_roof);
    assert!(!w.sq(10, 10, 0).is_exterior(m));
    assert!(w.sq(10, 10, 1).is_exterior(m));
    assert!(w.sq(11, 10, 0).is_exterior(m));
    // Level 0 squares are created for the whole chunk, and the 3x3 around
    // non-empty squares on other levels.
    assert!(w.cell.get_grid_square(8, 8, 0).is_some());
    assert!(w.cell.get_grid_square(11, 11, 1).is_some());
    assert!(w.cell.get_grid_square(13, 13, 1).is_none());
}

#[test]
fn vision_blocked_by_walls_and_solids() {
    let mut w = World::new();
    w.put(10, 10, 0, &["floor"]);
    w.put(10, 11, 0, &["floor", "wall_n"]);
    w.put(11, 10, 0, &["floor", "crate"]);
    w.put(9, 10, 0, &["floor", "glass_n"]);
    w.recalc();
    let a = w.sq(10, 10, 0);
    // The south neighbour has a north wall.
    assert!(a.get_vision_matrix(0, 1, 0));
    assert!(w.sq(10, 11, 0).get_vision_matrix(0, -1, 0));
    // A solid object blocks vision into its square, not out of it.
    assert!(a.get_vision_matrix(1, 0, 0));
    assert!(!w.sq(11, 10, 0).get_vision_matrix(-1, 0, 0));
    // A transparent north wall does not block going west.
    assert!(!a.get_vision_matrix(-1, 0, 0));
    // The diagonal past the wall is blocked.
    assert!(a.get_vision_matrix(1, 1, 0));
    // No square above: not blocked.
    assert!(!a.get_vision_matrix(0, 0, 1));
    let b = w.sq(10, 11, 0);
    assert!(a.calculate_vision_blocked(Some(b), &w.m, &w.cell));
    assert!(!a.calculate_vision_blocked(None, &w.m, &w.cell));
}

#[test]
fn vision_between_levels() {
    let mut w = World::new();
    w.put(10, 10, 0, &["floor"]);
    w.put(10, 10, 1, &["floor"]);
    w.put(12, 12, 0, &["floor"]);
    w.put(12, 12, 1, &["glass_floor"]);
    w.recalc();
    assert!(w.sq(10, 10, 0).get_vision_matrix(0, 0, 1));
    assert!(w.sq(10, 10, 1).get_vision_matrix(0, 0, -1));
    assert!(!w.sq(12, 12, 0).get_vision_matrix(0, 0, 1));
}

#[test]
fn collide_matrix_and_walls() {
    let mut w = World::new();
    w.put(10, 10, 0, &["floor"]);
    w.put(10, 11, 0, &["floor", "wall_n"]);
    w.put(11, 10, 0, &["floor"]);
    w.put(12, 10, 0, &["floor", "wall_w"]);
    w.recalc();
    let a = w.sq(10, 10, 0);
    assert!(a.test_collide_adjacent(&w.meta, 0, 1, 0));
    assert!(!a.test_collide_adjacent(&w.meta, 1, 0, 0));
    assert!(a.test_collide_adjacent(&w.meta, 2, 0, 0));
    let (b, e, ee) = (w.sq(10, 11, 0), w.sq(11, 10, 0), w.sq(12, 10, 0));
    assert!(a.is_wall_to(Some(b), &w.m, &w.cell));
    assert!(b.is_wall_to(Some(a), &w.m, &w.cell));
    assert!(!a.is_wall_to(Some(e), &w.m, &w.cell));
    assert!(e.is_wall_to(Some(ee), &w.m, &w.cell));
    assert!(ee.is_wall_to(Some(e), &w.m, &w.cell));
    assert!(!a.is_wall_to(Some(a), &w.m, &w.cell));
}

#[test]
fn windows_and_doors_to() {
    let mut w = World::new();
    w.put(10, 10, 0, &["floor"]);
    w.put(10, 11, 0, &["floor"]);
    w.add(
        10,
        11,
        0,
        "window_n",
        ObjectKind::Window(IsoWindow {
            north: true,
            ..window()
        }),
    );
    w.put(12, 10, 0, &["floor"]);
    w.put(12, 11, 0, &["floor"]);
    w.add(
        12,
        11,
        0,
        "door_n",
        ObjectKind::Door(IsoDoor {
            north: true,
            ..door()
        }),
    );
    w.recalc();
    let g = &w.cell;
    let (a, b) = (w.sq(10, 10, 0), w.sq(10, 11, 0));
    assert!(a.is_window_to(Some(b), &w.m, g));
    assert!(!a.is_wall_to(Some(b), &w.m, g));
    assert!(a.is_window_blocked_to(Some(b), g, &NoSquareHooks));
    let found = a.get_window_to(Some(b), g).unwrap();
    assert!(matches!(found.kind, ObjectKind::Window(_)));
    assert!(b.get_window_to(Some(a), g).is_some());
    assert!(a.get_window_to(Some(w.sq(12, 10, 0)), g).is_none());
    let (c, d) = (w.sq(12, 10, 0), w.sq(12, 11, 0));
    assert!(c.is_door_to(Some(d), &w.m, g));
    assert!(c.is_door_blocked_to(Some(d), g, &NoSquareHooks));
    assert!(!a.is_door_to(Some(b), &w.m, g));
}

struct Barricaded;

impl SquareHooks for Barricaded {
    fn is_barricaded(&self, _sq: &IsoGridSquare, _obj: &IsoObject) -> bool {
        true
    }
}

#[test]
fn barricade_hook_blocks_open_window() {
    let mut w = World::new();
    w.put(10, 10, 0, &["floor"]);
    w.put(10, 11, 0, &["floor"]);
    let open = IsoWindow {
        north: true,
        open: true,
        ..window()
    };
    w.add(10, 11, 0, "window_n", ObjectKind::Window(open));
    let (a, b) = (w.sq(10, 10, 0), w.sq(10, 11, 0));
    assert!(!a.is_window_blocked_to(Some(b), &w.cell, &NoSquareHooks));
    assert!(a.is_window_blocked_to(Some(b), &w.cell, &Barricaded));
}

#[test]
fn chunk_edges_use_the_cell() {
    let mut w = World::new();
    w.cell.chunks.insert((2, 1), IsoChunk::new(2, 1));
    w.put(15, 10, 0, &["floor"]);
    w.put(16, 10, 0, &["floor", "wall_w"]);
    w.recalc();
    assert!(w.sq(15, 10, 0).get_vision_matrix(1, 0, 0));
    assert!(w.sq(16, 10, 0).get_vision_matrix(-1, 0, 0));
    let ch = &w.cell.chunks[&(1, 1)];
    assert!(ChunkGetter(ch).get_grid_square(16, 10, 0).is_none());
    assert!(ChunkGetter(ch).get_grid_square(15, 10, 0).is_some());
}

#[test]
fn directions_and_bits() {
    assert_eq!(IsoDirections::N.rot180(), IsoDirections::S);
    assert_eq!(IsoDirections::NE.rot180(), IsoDirections::SW);
    assert_eq!(IsoDirections::from_name(" w "), Some(IsoDirections::W));
    assert_eq!(IsoDirections::from_name("up"), None);
    assert_eq!(set_matrix_bit(0, 2, 1, 1, true), 1 << 14);
    assert!(get_matrix_bit(1 << 14, 2, 1, 1));
}

fn window() -> IsoWindow {
    IsoWindow {
        north: false,
        locked: false,
        perma_locked: false,
        open: false,
        destroyed: false,
        glass_removed: false,
        health: 0,
        max_health: 0,
        open_sprite: None,
        closed_sprite: None,
        smashed_sprite: None,
        glass_removed_sprite: None,
    }
}

fn door() -> IsoDoor {
    IsoDoor {
        north: false,
        open: false,
        locked: false,
        locked_by_key: false,
        health: 0,
        max_health: 0,
        pushed_strength: 0,
        pushed_max_strength: 0,
        open_sprite: None,
        closed_sprite: None,
        has_curtain: false,
        curtain_open: false,
        curtain_inside: false,
    }
}

#[test]
fn main_thread_load_fills_squares_across_edges() {
    let mut w = World::new();
    for (wx, wy) in [(2, 1), (0, 0), (1, 0), (0, 1)] {
        w.cell.chunks.insert((wx, wy), IsoChunk::new(wx, wy));
    }
    // A neighbour's object on level 1 next to this chunk's east edge.
    w.put(16, 10, 1, &["floor"]);
    // This chunk's object on level 2 in its north-west corner.
    w.put(8, 8, 2, &["floor"]);
    for k in [(1, 1), (2, 1), (0, 0), (1, 0), (0, 1)] {
        w.cell.recalc_chunk_squares(k.0, k.1, &w.meta, &w.m);
    }
    let levels = |w: &World, k| {
        let c: &IsoChunk = &w.cell.chunks[&k];
        (c.min_level, c.max_level)
    };
    assert_eq!(levels(&w, (1, 0)), (0, 0));
    w.cell.load_in_main_thread(1, 1, &w.meta, &w.m);
    let has = |w: &World, x, y, z| w.cell.get_grid_square(x, y, z).is_some();
    // EnsureSurroundNotNullY(7, 2, 1) here: (15, 9..=11, 1).
    assert_eq!(levels(&w, (1, 1)), (0, 2));
    for y in 9..=11 {
        assert!(has(&w, 15, y, 1));
    }
    assert!(!has(&w, 15, 8, 1) && !has(&w, 15, 12, 1) && !has(&w, 14, 10, 1));
    // Around (8, 8, 2) in the NW, N and W neighbours.
    assert_eq!(levels(&w, (0, 0)), (0, 2));
    assert!(has(&w, 7, 7, 2));
    assert_eq!(levels(&w, (1, 0)), (0, 2));
    assert!(has(&w, 8, 7, 2) && has(&w, 9, 7, 2) && !has(&w, 10, 7, 2));
    assert!(has(&w, 7, 8, 2) && has(&w, 7, 9, 2) && !has(&w, 7, 10, 2));
    // The east neighbour is not widened downwards or upwards beyond its own
    // objects' levels.
    assert_eq!(levels(&w, (2, 1)), (0, 1));
}

/// `loadInMainThread` sees only the chunks already added to the cell
/// (`setChunkDirect` comes just before it for each chunk), so a bit computed
/// there across a missing neighbour keeps that neighbour's absence until a
/// later chunk's pass recomputes the pair.
#[test]
fn main_thread_load_sees_only_chunks_added_before() {
    let build = |game_order: bool| {
        let mut w = World::new();
        w.cell.chunks.insert((1, 2), IsoChunk::new(1, 2));
        // Two seats on the south edge of chunk (1, 1), a window across the
        // edge in chunk (1, 2) south of the east one.
        w.put(8, 14, 0, &["floor"]);
        w.put(9, 14, 0, &["floor"]);
        w.put(8, 15, 0, &["floor", "seat"]);
        w.put(9, 15, 0, &["floor", "seat"]);
        w.put(8, 16, 0, &["floor"]);
        w.put(9, 16, 0, &["floor", "window_n"]);
        let south = w.cell.chunks.remove(&(1, 2)).unwrap();
        w.cell.recalc_chunk_squares(1, 1, &w.meta, &w.m);
        if game_order {
            w.cell.load_in_main_thread(1, 1, &w.meta, &w.m);
        }
        w.cell.chunks.insert((1, 2), south);
        w.cell.recalc_chunk_squares(1, 2, &w.meta, &w.m);
        if !game_order {
            w.cell.load_in_main_thread(1, 1, &w.meta, &w.m);
        }
        w.cell.load_in_main_thread(1, 2, &w.meta, &w.m);
        w
    };
    let bit = |w: &World, dx: i32, dy: i32| {
        get_matrix_bit(w.sq(8, 15, 0).collide_matrix, dx + 1, dy + 1, 1)
    };
    let w = build(true);
    // The window next to the east seat lets it be passed (solidtrans with a
    // window to the south): chunk (1, 2)'s corner pair recomputes the edge
    // through the cell.
    assert!(!bit(&w, 1, 0));
    // The diagonal past the east seat was last computed by chunk (1, 1)'s
    // pass, before chunk (1, 2) was in the cell.
    assert!(bit(&w, 1, -1));
    // Computed with both chunks present it is open.
    let w = build(false);
    assert!(!bit(&w, 1, 0));
    assert!(!bit(&w, 1, -1));
}

/// Special objects of classes this crate does not model, by square and
/// object identity.
#[derive(Default)]
struct Specials(Vec<((i32, i32, i32), ObjectId, SpecialObject)>);

impl SquareHooks for Specials {
    fn special_object(&self, sq: &IsoGridSquare, obj: &IsoObject) -> Option<SpecialObject> {
        self.0
            .iter()
            .find(|(at, id, _)| *at == (sq.x, sq.y, sq.z) && *id == obj.id)
            .map(|(_, _, s)| *s)
    }
}

impl World {
    /// A plain object in `specialObjects` (what a hook-modelled class looks
    /// like here).
    fn add_special(&mut self, x: i32, y: i32, z: i32, name: &str) -> ObjectId {
        self.add(x, y, z, name, ObjectKind::Object);
        let sq = self.cell.get_grid_square_mut(x, y, z).unwrap();
        let id = sq.objects.last().unwrap().id;
        sq.special_objects.push(id);
        id
    }
}

fn thumpable(north: bool) -> ThumpableInfo {
    ThumpableInfo {
        north,
        ..ThumpableInfo::default()
    }
}

#[test]
fn stairs_heights_and_edges() {
    let mut w = World::new();
    w.put(10, 9, 0, &["floor"]);
    w.put(10, 10, 0, &["stairs_tn"]);
    w.put(10, 11, 0, &["stairs_mn"]);
    w.put(10, 12, 0, &["stairs_bn"]);
    w.put(11, 10, 0, &["floor"]);
    let m = &w.m;
    let (top, mid, bottom) = (w.sq(10, 10, 0), w.sq(10, 11, 0), w.sq(10, 12, 0));
    assert_eq!(top.get_stairs_direction(m), Some(IsoDirections::N));
    assert_eq!(w.sq(10, 9, 0).get_stairs_direction(m), None);
    assert_eq!(top.get_stairs_height(IsoDirections::N, m), 1.0);
    assert_eq!(top.get_stairs_height(IsoDirections::S, m), 0.66);
    assert_eq!(top.get_stairs_height(IsoDirections::E, m), -1.0);
    assert_eq!(mid.get_stairs_height(IsoDirections::N, m), 0.66);
    // Bottom stairs report 0 for both ends, as in the original.
    assert_eq!(bottom.get_stairs_height_max(m), 0.0);
    assert_eq!(bottom.get_stairs_height_min(m), 0.0);
    let g = &w.cell;
    // The top step is 1 high to the north, the floor beyond is 0.
    assert!(top.is_stairs_edge_blocked(IsoDirections::N, m, g));
    assert!(!top.is_stairs_edge_blocked(IsoDirections::S, m, g));
    assert!(top.is_stairs_edge_blocked(IsoDirections::E, m, g));
    // No square to the west.
    assert!(top.is_stairs_edge_blocked(IsoDirections::W, m, g));
    assert!(!w
        .sq(10, 9, 0)
        .is_stairs_edge_blocked(IsoDirections::S, m, g));
}

#[test]
fn sloped_surface_heights() {
    let m = manager();
    let ramp = square(&m, 3, 3, 0, &["ramp_n"]);
    let floor = square(&m, 4, 3, 0, &["floor"]);
    assert_eq!(ramp.get_sloped_surface_height_at(0.5, 0.25), 0.75);
    assert_eq!(ramp.get_sloped_surface_height_at(0.5, -3.0), 1.0);
    assert_eq!(floor.get_sloped_surface_height_at(0.5, 0.5), 0.0);
    assert_eq!(ramp.get_sloped_surface_height_max(), 1.0);
    assert!(ramp.has_identical_sloped_surface(&square(&m, 9, 9, 0, &["ramp_n"])));
    assert!(!ramp.has_identical_sloped_surface(&floor));
}

#[test]
fn adjacent_windows_and_hoppables() {
    let mut w = World::new();
    for (x, y) in [
        (9, 10),
        (9, 11),
        (11, 10),
        (13, 10),
        (15, 11),
        (15, 12),
        (9, 14),
    ] {
        w.put(x, y, 0, &["floor"]);
    }
    // A window on the south edge of (9, 10).
    w.add(
        9,
        11,
        0,
        "window_n",
        ObjectKind::Window(IsoWindow {
            north: true,
            ..window()
        }),
    );
    // A window frame on the west edge of (11, 10).
    w.add(
        11,
        10,
        0,
        "frame_w",
        ObjectKind::WindowFrame { north: false },
    );
    let tw = w.add_special(13, 10, 0, "thump_window_n");
    // A fence on the north edge of (15, 12).
    w.add(15, 12, 0, "fence_n", ObjectKind::Object);
    let th = w.add_special(9, 14, 0, "thump");
    let hooks = Specials(vec![
        ((13, 10, 0), tw, SpecialObject::Thumpable(thumpable(true))),
        (
            (9, 14, 0),
            th,
            SpecialObject::Thumpable(ThumpableInfo {
                is_hoppable: true,
                ..thumpable(false)
            }),
        ),
    ]);
    let (m, g, none) = (&w.m, &w.cell, &NoSquareHooks);
    assert!(w.sq(9, 10, 0).is_adjacent_to_window(m, g, none));
    assert!(w.sq(9, 11, 0).is_adjacent_to_window(m, g, none));
    assert!(w.sq(11, 10, 0).is_adjacent_to_window(m, g, none));
    assert!(!w.sq(13, 10, 0).is_adjacent_to_window(m, g, none));
    assert!(w.sq(13, 10, 0).is_adjacent_to_window(m, g, &hooks));
    assert!(!w.sq(15, 12, 0).is_adjacent_to_window(m, g, &hooks));
    assert!(w.sq(15, 12, 0).is_adjacent_to_hoppable(m, g, none));
    assert!(w.sq(15, 11, 0).is_adjacent_to_hoppable(m, g, none));
    // The window frame's sprite counts as hoppable too.
    assert!(w.sq(11, 10, 0).is_adjacent_to_hoppable(m, g, none));
    assert!(!w.sq(9, 14, 0).is_adjacent_to_hoppable(m, g, none));
    assert!(w.sq(9, 14, 0).is_adjacent_to_hoppable(m, g, &hooks));
    // The thumpable window is on the north edge of (13, 10).
    w.put(13, 9, 0, &["floor"]);
    w.put(13, 11, 0, &["floor"]);
    let (m, g) = (&w.m, &w.cell);
    let a = w.sq(13, 10, 0);
    let (sq, o) = a
        .get_window_thumpable_to(g.get_grid_square(13, 9, 0), m, g, &hooks)
        .unwrap();
    assert_eq!(((sq.x, sq.y), o.id), ((13, 10), tw));
    assert!(a
        .get_window_thumpable_to(g.get_grid_square(13, 11, 0), m, g, &hooks)
        .is_none());
    assert!(w
        .sq(11, 10, 0)
        .get_window_frame_to(w.cell.get_grid_square(10, 10, 0), g)
        .is_none());
    w.put(10, 10, 0, &["floor"]);
    let g = &w.cell;
    let (sq, obj) = w
        .sq(10, 10, 0)
        .get_window_frame_to(g.get_grid_square(11, 10, 0), g)
        .unwrap();
    assert_eq!((sq.x, sq.y), (11, 10));
    assert!(matches!(obj.kind, ObjectKind::WindowFrame { north: false }));
}

#[test]
fn doors_to_and_reach() {
    let mut w = World::new();
    for (x, y) in [
        (10, 10),
        (10, 11),
        (12, 10),
        (12, 11),
        (14, 10),
        (14, 11),
        (8, 8),
        (9, 8),
    ] {
        w.put(x, y, 0, &["floor"]);
    }
    w.add(
        10,
        11,
        0,
        "door_n",
        ObjectKind::Door(IsoDoor {
            north: true,
            ..door()
        }),
    );
    let th = w.add_special(12, 11, 0, "thump");
    w.add(14, 11, 0, "wall_n", ObjectKind::Object);
    w.recalc();
    let door_th = ThumpableInfo {
        is_door: true,
        ..thumpable(true)
    };
    let hooks = Specials(vec![((12, 11, 0), th, SpecialObject::Thumpable(door_th))]);
    let (m, g, none) = (&w.m, &w.cell, &NoSquareHooks);
    let at = |x, y| g.get_grid_square(x, y, 0);
    let (_, d) = w.sq(10, 10, 0).get_door_to(at(10, 11), g, none).unwrap();
    assert!(matches!(d.kind, ObjectKind::Door(_)));
    assert!(w.sq(12, 10, 0).get_door_to(at(12, 11), g, none).is_none());
    let (sq, d) = w.sq(12, 10, 0).get_door_to(at(12, 11), g, &hooks).unwrap();
    assert_eq!(((sq.x, sq.y), d.id), ((12, 11), th));
    // Closed doors and walls stop reaching; open floor does not.
    assert!(!w.sq(10, 10, 0).can_reach_to(w.sq(10, 11, 0), m, g, none));
    assert!(!w.sq(14, 10, 0).can_reach_to(w.sq(14, 11, 0), m, g, none));
    assert!(w.sq(8, 8, 0).can_reach_to(w.sq(9, 8, 0), m, g, none));
    assert!(w.sq(8, 8, 0).can_reach_to(w.sq(8, 8, 0), m, g, none));
    assert!(!w.sq(8, 8, 0).can_reach_to(w.sq(10, 10, 0), m, g, none));
}

#[test]
fn vision_through_edges() {
    let mut w = World::new();
    for x in 8..16 {
        for y in 8..16 {
            w.put(x, y, 0, &["floor"]);
        }
    }
    w.add(12, 9, 0, "wall_w", ObjectKind::Object);
    w.add(
        9,
        13,
        0,
        "window_n",
        ObjectKind::Window(IsoWindow {
            north: true,
            ..window()
        }),
    );
    w.add(
        11,
        13,
        0,
        "door_n",
        ObjectKind::Door(IsoDoor {
            north: true,
            ..door()
        }),
    );
    w.add(
        14,
        11,
        0,
        "door_n",
        ObjectKind::Door(IsoDoor {
            north: true,
            open: true,
            ..door()
        }),
    );
    let bar = w.add_special(13, 13, 0, "barricade");
    let th = w.add_special(15, 13, 0, "thump");
    let thw = w.add_special(8, 13, 0, "thump_window_n");
    w.recalc();
    let door_th = ThumpableInfo {
        is_door: true,
        ..thumpable(true)
    };
    let barricade = |blocks_vision| SpecialObject::Barricade {
        dir: IsoDirections::N,
        blocks_vision,
    };
    let specials = |blocks| {
        Specials(vec![
            ((13, 13, 0), bar, barricade(blocks)),
            ((15, 13, 0), th, SpecialObject::Thumpable(door_th)),
            ((8, 13, 0), thw, SpecialObject::Thumpable(thumpable(true))),
        ])
    };
    let (hooks, weak) = (specials(true), specials(false));
    let env = VisionEnv::new(&w.cell, &w.m, &hooks);
    let tva = |x, y, dx, dy, ignore, env: &VisionEnv<'_, IsoCell>| {
        w.sq(x, y, 0)
            .test_vision_adjacent(dx, dy, 0, true, ignore, env)
    };
    use TestResults::*;
    // The wall on the west edge of (12, 9).
    assert_eq!(tva(11, 9, 1, 0, false, &env), Blocked);
    assert_eq!(tva(11, 10, 1, 0, false, &env), Clear);
    // Out of range.
    assert_eq!(tva(11, 10, 2, 0, false, &env), Blocked);
    assert_eq!(tva(9, 12, 0, 1, false, &env), ClearThroughWindow);
    assert_eq!(tva(9, 13, 0, -1, false, &env), ClearThroughWindow);
    // A closed door blocks unless doors are ignored.
    assert_eq!(tva(11, 12, 0, 1, false, &env), Blocked);
    assert_eq!(tva(11, 12, 0, 1, true, &env), Clear);
    // An open north door swings to the west edge.
    assert_eq!(tva(14, 11, -1, 0, false, &env), ClearThroughOpenDoor);
    assert_eq!(tva(14, 10, 0, 1, false, &env), Clear);
    // The barricade blocks from either side when it blocks vision.
    assert_eq!(tva(13, 13, 0, -1, false, &env), Blocked);
    assert_eq!(tva(13, 12, 0, 1, false, &env), Blocked);
    let weak_env = VisionEnv::new(&w.cell, &w.m, &weak);
    assert_eq!(tva(13, 12, 0, 1, false, &weak_env), Clear);
    let plain = VisionEnv::new(&w.cell, &w.m, &NoSquareHooks);
    // Without the hook the barricade is a plain object (no effect here).
    assert_eq!(tva(13, 12, 0, 1, false, &plain), Clear);
    // A closed thumpable door; a thumpable window is seen through.
    assert_eq!(tva(15, 12, 0, 1, false, &env), Blocked);
    assert_eq!(tva(15, 12, 0, 1, true, &env), Clear);
    assert_eq!(tva(8, 12, 0, 1, true, &env), Clear);
    // Diagonals check both sides: past the wall corner at (12, 9).
    assert_eq!(tva(11, 9, 1, 1, false, &env), Blocked);
    assert_eq!(tva(11, 11, 1, 1, false, &env), Clear);
    // Lines of sight.
    assert_eq!(line_clear(&env, 8, 9, 0, 15, 9, 0, false), Blocked);
    assert_eq!(line_clear(&env, 8, 10, 0, 15, 10, 0, false), Clear);
    assert_eq!(
        line_clear(&env, 9, 10, 0, 9, 15, 0, false),
        ClearThroughWindow
    );
    assert_eq!(line_clear(&env, 11, 10, 0, 11, 15, 0, false), Blocked);
    assert_eq!(line_clear(&env, 11, 10, 0, 11, 15, 0, true), Clear);
    assert_eq!(
        crate::los::line_clear_range(&env, 9, 8, 0, 9, 15, 0, false, 2),
        Blocked
    );
    assert_eq!(
        crate::los::line_clear_range(&env, 9, 8, 0, 9, 15, 0, false, 10),
        ClearThroughWindow
    );
}

/// `IsoChunkLevel.physicsCheck`: `RecalcProperties` sets it for the
/// square's level, `updatePhysicsForLevelIfNeeded` takes it, and
/// `setMinMaxLevel` keeps it for kept levels only.
#[test]
fn physics_check_flags() {
    let mut w = World::new();
    w.put(9, 9, 0, &["floor"]);
    w.put(9, 9, 1, &["floor"]);
    let ch = &w.cell.chunks[&(1, 1)];
    assert!(!ch.physics_check(0) && !ch.physics_check(1));
    w.recalc();
    let ch = w.cell.chunks.get_mut(&(1, 1)).unwrap();
    assert!(ch.physics_check(0) && ch.physics_check(1));
    assert!(ch.take_physics_check(1));
    assert!(!ch.take_physics_check(1));
    assert!(!ch.take_physics_check(5));
    ch.check_physics_later(5);
    assert!(!ch.physics_check(5));

    // A square changing objects marks its own level again.
    w.cell.recalc_all_with_neighbours(9, 9, 1, &w.meta, &w.m);
    let ch = w.cell.chunks.get_mut(&(1, 1)).unwrap();
    assert!(ch.physics_check(1));

    ch.set_min_max_level(-1, 0);
    assert!(ch.physics_check(0) && !ch.physics_check(-1) && !ch.physics_check(1));
    ch.set_min_max_level(-1, 1);
    assert!(!ch.physics_check(1));
}
