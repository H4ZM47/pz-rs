//! Square properties, matrices and edge queries on hand-built squares.

use super::*;
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
