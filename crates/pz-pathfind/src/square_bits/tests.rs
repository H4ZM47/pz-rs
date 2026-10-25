//! `getBits`, `getCost` and the chunk buffer on hand-built squares.

use super::bits::*;
use super::*;
use pz_world::objects::{IsoDoor, IsoWindow};
use pz_world::sprite::PropertyValueMap;
use pz_world::{IsoCell, ObjectKind};

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Hand-authored enum tables: every constant `pz-world` and this module
/// look up, in an arbitrary order.
fn tables() -> SpriteTables {
    SpriteTables::new(
        names(&[
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
            "attachedE",
            "attachedS",
            "FloorOverlay",
            "vegitation",
            "NeverCutaway",
            "HasLightOnSprite",
            "EntityScript",
            "water",
            "solidtrans",
            "blueprint",
            "BlockRain",
            "trans",
            "noStart",
            "forceRender",
            "burntOut",
            "TallHoppableN",
            "TallHoppableW",
            "WallN",
            "WallW",
            "WallNTrans",
            "WallWTrans",
            "DoorWallN",
            "DoorWallW",
            "isEave",
            "makeWindowInvincible",
            "climbSheetTopW",
            "climbSheetTopN",
            "climbSheetTopE",
            "climbSheetTopS",
            "climbSheetW",
            "climbSheetN",
            "climbSheetE",
            "climbSheetS",
        ]),
        names(&[
            "normal",
            "tree",
            "wall",
            "doorN",
            "doorW",
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
            "stairsTN",
            "stairsMN",
            "stairsBN",
            "stairsTW",
            "stairsMW",
            "stairsBW",
            "WestRoofB",
            "WestRoofM",
            "WestRoofT",
        ]),
        names(&["North", "West"]),
    )
    .unwrap()
}

/// Sprite name, tile type, tileset, properties (an empty value sets the
/// flag of that name).
type Spec<'a> = (&'a str, &'a str, &'a str, &'a [(&'a str, &'a str)]);

const SPRITES: &[Spec] = &[
    ("floor", "normal", "floors", &[("solidfloor", "")]),
    (
        "wall_n",
        "wall",
        "walls",
        &[("collideN", ""), ("cutN", ""), ("WallN", "")],
    ),
    (
        "wall_w",
        "wall",
        "walls",
        &[("collideW", ""), ("cutW", ""), ("WallW", "")],
    ),
    (
        "fence_n",
        "normal",
        "fences",
        &[("collideN", ""), ("HoppableN", ""), ("canPathN", "")],
    ),
    (
        "window_n",
        "windowFN",
        "windows",
        &[("windowN", ""), ("cutN", "")],
    ),
    (
        "frame_n",
        "normal",
        "frames",
        &[("WindowN", ""), ("cutN", "")],
    ),
    (
        "door_n",
        "doorN",
        "doors",
        &[("doorN", ""), ("collideN", "")],
    ),
    ("doorway_w", "normal", "frames", &[("DoorWallW", "")]),
    ("crate", "normal", "furniture", &[("solid", "")]),
    ("tree", "tree", "trees", &[]),
    ("bush", "normal", "f_bushes_1", &[]),
    ("roof", "WestRoofB", "roofs", &[]),
    ("eave_roof", "WestRoofB", "roofs", &[("isEave", "")]),
    ("rope", "normal", "ropes", &[("climbSheetN", "")]),
    (
        "ramp_n",
        "normal",
        "ramps",
        &[
            ("solidfloor", ""),
            ("SlopedSurfaceDirection", "N"),
            ("SlopedSurfaceHeightMin", "25"),
            ("SlopedSurfaceHeightMax", "50"),
        ],
    ),
    ("stairs", "stairsMN", "stairs", &[("solidfloor", "")]),
    ("barrel", "normal", "furniture", &[]),
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
    for (_, _, _, props) in SPRITES {
        for (k, v) in *props {
            let e = map.entry(k);
            if !e.iter().any(|x| x == v) {
                e.push(v.to_string());
            }
        }
    }
    let mut m = IsoSpriteManager::new(tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (i, (name, ty, tileset, props)) in SPRITES.iter().enumerate() {
        let s = m.add_sprite(name, 100 + i as i32);
        let t = m.tables.object_type_from_string(ty);
        let (tables, alias) = (m.tables.clone(), m.alias.clone());
        let sp = &mut m.sprites[s];
        sp.name = Some(name.to_string());
        sp.tileset_name = Some(tileset.to_string());
        sp.tile_type = t;
        for (k, v) in *props {
            sp.properties.set(&tables, &alias, k, v, v.is_empty());
        }
    }
    m
}

/// A cell with chunk (1, 1) (squares 8..16) and its west neighbour
/// (0, 1), and the hooks the next `bits` call uses.
struct World {
    cell: IsoCell,
    m: IsoSpriteManager,
    meta: IsoMetaGrid,
}

impl World {
    fn new() -> World {
        let mut cell = IsoCell::default();
        cell.chunks.insert((1, 1), IsoChunk::new(1, 1));
        cell.chunks.insert((0, 1), IsoChunk::new(0, 1));
        World {
            cell,
            m: manager(),
            meta: IsoMetaGrid::empty(),
        }
    }

    /// Puts a new square with plain objects of these sprites.
    fn put(&mut self, x: i32, y: i32, z: i32, objs: &[&str]) {
        let mut sq = IsoGridSquare::new(x, y, z);
        for o in objs {
            self.add_to(&mut sq, o, ObjectKind::Object);
        }
        let ch = self
            .cell
            .chunks
            .get_mut(&(x.div_euclid(8), y.div_euclid(8)))
            .unwrap();
        ch.set_square(x.rem_euclid(8), y.rem_euclid(8), z, Some(sq));
        self.recalc(x, y, z);
    }

    fn add_to(
        &self,
        sq: &mut IsoGridSquare,
        name: &str,
        kind: ObjectKind,
    ) -> pz_world::objects::ObjectId {
        let id = sq.new_object_id();
        let special = !matches!(kind, ObjectKind::Object);
        sq.objects
            .push(pz_world::ObjectSlot::new(IsoObject::new(id, kind, Some(self.m.named_map[name]))));
        if special {
            sq.special_objects.push(id);
        }
        id
    }

    fn add(&mut self, x: i32, y: i32, z: i32, name: &str, kind: ObjectKind) {
        let mut sq = self.cell.get_grid_square_mut(x, y, z).unwrap().clone();
        self.add_to(&mut sq, name, kind);
        *self.cell.get_grid_square_mut(x, y, z).unwrap() = sq;
        self.recalc(x, y, z);
    }

    fn recalc(&mut self, x: i32, y: i32, z: i32) {
        let m = &self.m;
        self.cell
            .get_grid_square_mut(x, y, z)
            .unwrap()
            .recalc_properties(m);
    }

    fn sq(&self, x: i32, y: i32, z: i32) -> &IsoGridSquare {
        self.cell.get_grid_square(x, y, z).unwrap()
    }

    fn bits_with(&self, x: i32, y: i32, z: i32, hooks: &dyn SquareBitsHooks) -> u64 {
        let sb = SquareBits::new(&self.m, &self.meta, &self.cell, hooks).unwrap();
        sb.get_bits(self.sq(x, y, z))
    }

    fn bits(&self, x: i32, y: i32, z: i32) -> u64 {
        self.bits_with(x, y, z, &DefaultBitsHooks)
    }

    fn cost(&self, x: i32, y: i32, z: i32) -> i16 {
        let sb = SquareBits::new(&self.m, &self.meta, &self.cell, &DefaultBitsHooks).unwrap();
        sb.get_cost(self.sq(x, y, z))
    }
}

#[test]
fn floor_solid_and_stairs() {
    let mut w = World::new();
    w.put(9, 9, 0, &["floor"]);
    w.put(10, 9, 0, &["floor", "crate"]);
    w.put(11, 9, 0, &["stairs"]);
    assert_eq!(w.bits(9, 9, 0), SOLID_FLOOR);
    assert_eq!(w.bits(10, 9, 0), SOLID_FLOOR | SOLID);
    assert_eq!(w.bits(11, 9, 0), SOLID_FLOOR | STAIR_MN);
}

#[test]
fn hoppable_and_tall_hoppable_walls() {
    let mut w = World::new();
    // A low fence is hoppable whatever is around it.
    w.put(9, 9, 0, &["floor", "fence_n"]);
    assert_eq!(
        w.bits(9, 9, 0),
        SOLID_FLOOR | COLLIDE_N | CAN_PATH_N | HOPPABLE_N
    );
    // An outdoor wall with open floor on both sides (north and south: the
    // original tests the two neighbours of the square) can be climbed over.
    w.put(12, 10, 0, &["floor", "wall_n"]);
    w.put(12, 9, 0, &["floor"]);
    assert_eq!(w.bits(12, 10, 0), SOLID_FLOOR | COLLIDE_N);
    w.put(12, 11, 0, &["floor"]);
    assert_eq!(w.bits(12, 10, 0), SOLID_FLOOR | COLLIDE_N | TALL_HOPPABLE_N);
    // Not under a roof.
    w.cell.get_grid_square_mut(12, 9, 0).unwrap().have_roof = true;
    assert_eq!(w.bits(12, 10, 0), SOLID_FLOOR | COLLIDE_N);
    w.cell.get_grid_square_mut(12, 9, 0).unwrap().have_roof = false;
    // Not when a sloped roof without eave is above either side.
    w.put(12, 9, 1, &["roof"]);
    assert_eq!(w.bits(12, 10, 0), SOLID_FLOOR | COLLIDE_N);
    w.put(12, 9, 1, &["eave_roof"]);
    assert_eq!(w.bits(12, 10, 0), SOLID_FLOOR | COLLIDE_N | TALL_HOPPABLE_N);
    // Not next to a sheet rope.
    w.add(12, 9, 0, "rope", ObjectKind::Object);
    assert_eq!(w.bits(12, 10, 0), SOLID_FLOOR | COLLIDE_N);
    // Across a chunk edge: the west side is in chunk (0, 1).
    w.put(8, 12, 0, &["floor", "wall_w"]);
    w.put(9, 12, 0, &["floor"]);
    assert_eq!(w.bits(8, 12, 0), SOLID_FLOOR | COLLIDE_W);
    w.put(7, 12, 0, &["floor"]);
    assert_eq!(w.bits(8, 12, 0), SOLID_FLOOR | COLLIDE_W | TALL_HOPPABLE_W);
    // A safehouse only matters to a client.
    struct Client;
    impl SquareBitsHooks for Client {
        fn is_client(&self) -> bool {
            true
        }
        fn in_safehouse(&self, sq: &IsoGridSquare) -> bool {
            sq.x == 7
        }
    }
    assert_eq!(w.bits_with(8, 12, 0, &Client), SOLID_FLOOR | COLLIDE_W);
}

#[test]
fn windows() {
    let mut w = World::new();
    w.put(9, 9, 0, &["floor"]);
    let window = |open, destroyed, glass_removed| {
        ObjectKind::Window(IsoWindow {
            north: true,
            open,
            destroyed,
            glass_removed,
            ..Default::default()
        })
    };
    w.add(9, 9, 0, "window_n", window(false, false, false));
    let closed = SOLID_FLOOR | WINDOW_N | COLLIDE_N;
    assert_eq!(w.bits(9, 9, 0), closed);
    w.put(9, 9, 0, &["floor"]);
    w.add(9, 9, 0, "window_n", window(true, false, false));
    assert_eq!(w.bits(9, 9, 0), closed | WINDOW_N_UNBLOCKED);
    struct Barricades;
    impl SquareBitsHooks for Barricades {
        fn is_barricaded(&self, _: &IsoGridSquare, _: &IsoObject) -> bool {
            true
        }
    }
    assert_eq!(w.bits_with(9, 9, 0, &Barricades), closed);
    w.put(9, 9, 0, &["floor"]);
    w.add(9, 9, 0, "window_n", window(false, true, false));
    assert_eq!(w.bits(9, 9, 0), closed);
    w.put(9, 9, 0, &["floor"]);
    w.add(9, 9, 0, "window_n", window(false, true, true));
    assert_eq!(w.bits(9, 9, 0), closed | WINDOW_N_UNBLOCKED);
    // An empty window frame can be climbed through.
    w.put(10, 9, 0, &["floor"]);
    w.add(10, 9, 0, "frame_n", ObjectKind::WindowFrame { north: true });
    assert_eq!(w.bits(10, 9, 0), closed | WINDOW_N_UNBLOCKED);
    assert_eq!(w.bits_with(10, 9, 0, &Barricades), closed);
}

#[test]
fn doors_and_doorways() {
    let mut w = World::new();
    w.put(9, 9, 0, &["floor"]);
    let door = |open| {
        ObjectKind::Door(IsoDoor {
            north: true,
            open,
            ..Default::default()
        })
    };
    w.add(9, 9, 0, "door_n", door(false));
    assert_eq!(w.bits(9, 9, 0), SOLID_FLOOR | COLLIDE_N | CAN_PATH_N);
    w.put(9, 9, 0, &["floor"]);
    w.add(9, 9, 0, "door_n", door(true));
    assert_eq!(w.bits(9, 9, 0), SOLID_FLOOR | COLLIDE_N | DOOR_N_UNBLOCKED);
    // A doorway without a door is an open door.
    w.put(10, 9, 0, &["floor", "doorway_w"]);
    assert_eq!(
        w.bits(10, 9, 0),
        SOLID_FLOOR | CAN_PATH_W | COLLIDE_W | DOOR_W_UNBLOCKED
    );
}

#[test]
fn thumpables() {
    let mut w = World::new();
    w.put(9, 9, 0, &["floor"]);
    let mut th = IsoThumpable {
        block_all_the_square: true,
        ..Default::default()
    };
    w.add(
        9,
        9,
        0,
        "barrel",
        ObjectKind::Thumpable(Box::new(th.clone())),
    );
    assert_eq!(
        w.bits(9, 9, 0),
        SOLID_FLOOR | CAN_PATH_W | CAN_PATH_N | THUMPABLE | SOLID_TRANS
    );
    // A thumpable wall.
    th.block_all_the_square = false;
    w.put(10, 9, 0, &["floor"]);
    w.add(
        10,
        9,
        0,
        "wall_w",
        ObjectKind::Thumpable(Box::new(th.clone())),
    );
    assert_eq!(
        w.bits(10, 9, 0),
        SOLID_FLOOR | COLLIDE_W | CAN_PATH_W | THUMP_W
    );
    // One that lets characters through is not.
    th.can_pass_through = true;
    w.put(11, 9, 0, &["floor"]);
    w.add(11, 9, 0, "wall_w", ObjectKind::Thumpable(Box::new(th)));
    assert_eq!(w.bits(11, 9, 0), SOLID_FLOOR | COLLIDE_W);
    // A moved furniture item that is thumpable.
    w.put(12, 9, 0, &["floor", "barrel"]);
    w.cell.get_grid_square_mut(12, 9, 0).unwrap().objects[1].borrow_mut().moved_thumpable = true;
    assert_eq!(
        w.bits(12, 9, 0),
        SOLID_FLOOR | CAN_PATH_W | CAN_PATH_N | THUMPABLE | SOLID_TRANS
    );
}

#[test]
fn hooks_for_global_objects_and_bent_fences() {
    let mut w = World::new();
    w.put(9, 9, 0, &["floor", "fence_n"]);
    struct All;
    impl SquareBitsHooks for All {
        fn has_lit_campfire(&self, _: &IsoGridSquare) -> bool {
            true
        }
        fn has_farming_plant(&self, _: &IsoGridSquare) -> bool {
            true
        }
        fn bent_fences_enabled(&self) -> bool {
            true
        }
        fn is_unbent_object(&self, _: &IsoGridSquare, _: &IsoObject) -> bool {
            true
        }
    }
    let plain = w.bits(9, 9, 0);
    assert_eq!(plain, SOLID_FLOOR | COLLIDE_N | CAN_PATH_N | HOPPABLE_N);
    // The fence sprite is no `WallN`, so nothing bends.
    assert_eq!(
        w.bits_with(9, 9, 0, &All),
        plain | CAN_PATH_W | THUMPABLE | SOLID_TRANS | FARMING_PLANT
    );
    w.put(10, 9, 0, &["floor", "wall_n"]);
    assert_eq!(
        w.bits_with(10, 9, 0, &All) & (BENDABLE_N | TARGET_PATH_N | THUMP_N),
        BENDABLE_N | TARGET_PATH_N | THUMP_N
    );
}

#[test]
fn cost_of_trees_and_bushes() {
    let mut w = World::new();
    w.put(9, 9, 0, &["floor"]);
    w.put(10, 9, 0, &["floor", "tree"]);
    w.put(11, 9, 0, &["floor", "bush"]);
    w.put(12, 9, 0, &["floor", "tree", "bush"]);
    assert_eq!(w.cost(9, 9, 0), 0);
    assert_eq!(w.cost(10, 9, 0), 5);
    assert_eq!(w.cost(11, 9, 0), 5);
    assert_eq!(w.cost(12, 9, 0), 5);
}

#[test]
fn chunk_buffer_layout_and_square_update() {
    let mut w = World::new();
    w.put(9, 10, 0, &["floor", "tree"]);
    w.put(10, 11, 1, &["ramp_n"]);
    let ch = &w.cell.chunks[&(1, 1)];
    assert_eq!((ch.min_level, ch.max_level), (0, 1));
    let sb = SquareBits::new(&w.m, &w.meta, &w.cell, &DefaultBitsHooks).unwrap();
    let buf = sb.chunk_update_buffer(ch);
    assert_eq!(buf.len(), 8 + 2 * 64 * 10 + 2 + 12);
    assert_eq!(&buf[0..8], &[0, 0, 0, 32, 0, 0, 0, 33]);
    let rec = |z: usize, x: usize, y: usize| {
        let o = 8 + (z * 64 + y * 8 + x) * 10;
        (
            u64::from_be_bytes(buf[o..o + 8].try_into().unwrap()),
            i16::from_be_bytes(buf[o + 8..o + 10].try_into().unwrap()),
        )
    };
    assert_eq!(rec(0, 1, 2), (SOLID_FLOOR, 5));
    assert_eq!(rec(0, 0, 0), (0, 0));
    assert_eq!(rec(1, 2, 3), (SOLID_FLOOR, 0));
    let s = 8 + 2 * 64 * 10;
    assert_eq!(&buf[s..s + 2], &[0, 1]);
    assert_eq!(&buf[s + 2..s + 6], &[2, 3, 33, 0]);
    assert_eq!(
        f32::from_be_bytes(buf[s + 6..s + 10].try_into().unwrap()),
        0.25
    );
    assert_eq!(
        f32::from_be_bytes(buf[s + 10..s + 14].try_into().unwrap()),
        0.5
    );

    let u = sb.square_update(w.sq(10, 11, 1));
    assert_eq!(
        (u.x, u.y, u.z, u.bits, u.cost),
        (10, 11, 33, SOLID_FLOOR, 0)
    );
    assert_eq!(
        (u.slope_direction, u.slope_min, u.slope_max),
        (0, 0.25, 0.5)
    );
    let u = sb.square_update(w.sq(9, 10, 0));
    assert_eq!(u.slope_direction, NO_SLOPE_DIRECTION);
    assert_eq!(
        sb.sloped_surface(w.sq(10, 11, 1)),
        Some(SlopedSurface {
            direction: IsoDirections::N,
            min: 0.25,
            max: 0.5
        })
    );
}
