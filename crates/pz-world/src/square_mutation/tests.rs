//! The square mutations on hand-authored sprites and squares.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use crate::objects::{IsoDoor, IsoTree};
use crate::sprite::{test_tables, PropertyValueMap};
use crate::IsoChunk;

/// Hand-authored sprites: name, tile type name, flags/properties (an empty
/// value sets the flag of that name).
type Spec<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

const SPRITES: &[Spec] = &[
    ("floor", "normal", &[("solidfloor", "")]),
    ("floor2", "normal", &[]),
    ("floors_rugs_x", "normal", &[("solidfloor", "")]),
    ("blends_grassoverlays_x", "normal", &[]),
    ("wall_n", "wall", &[("collideN", ""), ("cutN", "")]),
    ("wall_w", "wall", &[("collideW", ""), ("cutW", "")]),
    ("door_n", "doorN", &[("doorN", ""), ("collideN", "")]),
    ("dd_1", "doorN", &[("doorN", ""), ("DoubleDoor", "1")]),
    ("dd_2", "doorN", &[("doorN", ""), ("DoubleDoor", "2")]),
    ("crate", "normal", &[("solid", "")]),
    ("table", "normal", &[("IsTable", ""), ("Surface", "24")]),
    ("cup", "normal", &[("IsTableTop", "")]),
    ("tree", "tree", &[]),
    ("sink", "normal", &[("waterPiped", "")]),
    ("rope", "normal", &[("climbSheetN", "")]),
];

fn manager() -> IsoSpriteManager {
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
        sp.cut_n = props.iter().any(|(k, _)| *k == "cutN");
        sp.cut_w = props.iter().any(|(k, _)| *k == "cutW");
        for (k, v) in *props {
            sp.properties.set(&tables, &alias, k, v, v.is_empty());
        }
    }
    m
}

/// Every hook call, in order, shared by the three hook views.
#[derive(Clone, Default)]
struct H(Rc<RefCell<Vec<String>>>);

impl H {
    fn push(&self, s: String) {
        self.0.borrow_mut().push(s);
    }
}

impl PathfindSquareHooks for H {
    fn square_changed(&mut self, cell: &IsoCell, sq: SquarePos) {
        // What the pathfinder reads at the call: the square and its
        // neighbours.
        assert!(cell.get_grid_square(sq.0, sq.1, sq.2).is_some());
        let n = adjacent_squares(cell, sq).iter().flatten().count();
        self.push(format!("pathfind {},{},{} +{n}", sq.0, sq.1, sq.2));
    }
    fn map_collision_square_changed(&mut self, sq: &IsoGridSquare) {
        self.push(format!("collision {},{}", sq.x, sq.y));
    }
    fn add_chunk_to_world(&mut self, cell: &IsoCell, wx: i32, wy: i32) {
        assert!(cell.chunks.contains_key(&(wx, wy)));
        self.push(format!("addChunk {wx},{wy}"));
    }
}

impl RenderSquareHooks for H {
    fn invalidate_render_chunk_level(&mut self, wx: i32, wy: i32, level: i32, flags: u64) {
        self.push(format!("render {wx},{wy},{level} {flags}"));
    }
    fn invalidate_vispoly_chunk_level(&mut self, _: i32, _: i32, _: i32) {
        self.push("vispoly".into());
    }
    fn cutaways_square_changed(&mut self, _: Option<SquarePos>) {
        self.push("cutaways".into());
    }
    fn los_cache_cleared(&mut self, p: i32) {
        self.push(format!("los {p}"));
    }
    fn set_recalc_light_time(&mut self, t: f32) {
        self.push(format!("recalcLight {t}"));
    }
    fn light_source_update(&mut self) {
        self.push("lightSourceUpdate".into());
    }
    fn grid_occlusion_square_changed(&mut self) {
        self.push("occlusion".into());
    }
}

impl SquareMutationHooks for H {
    fn lua_event(&mut self, sq: &IsoGridSquare, ev: LuaEvent<'_>) {
        let s = match ev {
            LuaEvent::ObjectAboutToBeRemoved { index } => {
                format!("OnObjectAboutToBeRemoved {}", sq.objects[index].id)
            }
            LuaEvent::TileRemoved { object } => {
                format!("OnTileRemoved {:?}", object.map(|o| o.id))
            }
            LuaEvent::ContainerUpdate => "OnContainerUpdate".into(),
        };
        self.push(s);
    }
    fn world_call(&mut self, r: ObjectRef, _: &mut IsoObject, _: &mut dyn GameRand, c: WorldCall) {
        self.push(format!("world {} {c:?}", r.id));
    }
    fn fluid_amount(&mut self, _: ObjectRef, _: &IsoObject) -> f32 {
        5.0
    }
    fn ambient_walls_square_changed(&mut self, _: &IsoGridSquare) {
        self.push("ambientWalls".into());
    }
    fn designation_zone_add_new_roof(&mut self, x: i32, y: i32, z: i32) {
        self.push(format!("newRoof {x},{y},{z}"));
    }
}

/// `Rand` that answers 0 and records the bounds it was asked for.
#[derive(Default)]
struct Zero(Vec<i32>);
impl GameRand for Zero {
    fn next(&mut self, max: i32) -> i32 {
        self.0.push(max);
        0
    }
}

struct World {
    cell: IsoCell,
    meta: IsoMetaGrid,
    m: IsoSpriteManager,
    regions: IsoRegions,
    rand: Zero,
    doors: DoorTables,
    h: H,
    hp: H,
    hr: H,
}

impl World {
    /// Chunks (0, 0) and (1, 0) with a floor on every level-0 square.
    fn new() -> World {
        let m = manager();
        let mut cell = IsoCell::default();
        for wx in 0..2 {
            let mut ch = IsoChunk::new(wx, 0);
            for i in 0..64 {
                let mut sq = IsoGridSquare::new(wx * 8 + i % 8, i / 8, 0);
                let id = sq.new_object_id();
                sq.objects.push(IsoObject::new(
                    id,
                    ObjectKind::Object,
                    Some(m.named_map["floor"]),
                ));
                sq.recalc_properties(&m);
                ch.set_square(i % 8, i / 8, 0, Some(sq));
            }
            cell.chunks.insert((wx, 0), ch);
        }
        let mut meta = IsoMetaGrid::blank(0, 0, 0, 0);
        meta.create_cell(0, 0);
        let h = H::default();
        World {
            cell,
            meta,
            m,
            regions: IsoRegions::new(13),
            rand: Zero::default(),
            // Made-up offsets: closed north doors' first two parts side by
            // side along x.
            doors: DoorTables {
                x_offset: [[[0; 4]; 2], [[0, 1, 5, 6], [0; 4]]],
                y_offset: [[[0; 4]; 2]; 2],
            },
            hp: h.clone(),
            hr: h.clone(),
            h,
        }
    }

    fn env(&mut self) -> MutationEnv<'_> {
        MutationEnv {
            cell: &mut self.cell,
            meta: &self.meta,
            sprites: &mut self.m,
            regions: &mut self.regions,
            rand: &mut self.rand,
            doors: &self.doors,
            world: &mut self.h,
            pathfind: &mut self.hp,
            render: &mut self.hr,
            num_players: 2,
        }
    }

    fn obj(&self, name: &str) -> IsoObject {
        IsoObject::new(0, ObjectKind::Object, Some(self.m.named_map[name]))
    }

    fn sq(&self, p: SquarePos) -> &IsoGridSquare {
        self.cell.get_grid_square(p.0, p.1, p.2).unwrap()
    }

    fn names(&self, p: SquarePos) -> Vec<&str> {
        self.sq(p)
            .objects
            .iter()
            .map(|o| self.m.sprites[o.sprite.unwrap()].name())
            .collect()
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.h.0.borrow_mut())
    }
}

fn blocked(m: i32, dx: i32, dy: i32, dz: i32) -> bool {
    crate::square::get_matrix_bit(m, dx + 1, dy + 1, dz + 1)
}

#[test]
fn add_tile_object_order_and_hooks() {
    let mut w = World::new();
    let p = (3, 3, 0);
    let wall = w.obj("wall_n");
    let id = w.env().add_tile_object(p, wall, -1).unwrap();
    assert!(w.sq(p).has(w.m.tables.flag.collideN));
    assert!(blocked(w.sq(p).collide_matrix, 0, -1, 0));
    assert!(blocked(w.sq((3, 2, 0)).collide_matrix, 0, 1, 0));
    let log = w.take_log();
    let expected = [
        format!("world {id} RegisterEntity"),
        format!("world {id} CheckMoveWithWind"),
        format!("world {id} LightSource {{ add: true }}"),
        format!("world {id} FlagForHotSave"),
        "los 0".into(),
        "los 1".into(),
        "recalcLight -1".into(),
        "lightSourceUpdate".into(),
        "collision 3,3".into(),
        "vispoly".into(),
        "cutaways".into(),
        "ambientWalls".into(),
        "pathfind 3,3,0 +8".into(),
        "render 0,0,0 64".into(),
    ];
    assert_eq!(log, expected);
    // A door goes after the walls, a wall before an existing door.
    let door = w.obj("door_n");
    w.env().add_tile_object(p, door, 0);
    assert_eq!(w.names(p), ["floor", "wall_n", "door_n"]);
    let wall = w.obj("wall_w");
    w.env().add_tile_object(p, wall, -1);
    assert_eq!(w.names(p), ["floor", "wall_n", "wall_w", "door_n"]);
}

#[test]
fn add_to_world_draws_rand_for_trees_and_dripping_water() {
    let mut w = World::new();
    let mut tree = w.obj("tree");
    tree.kind = ObjectKind::Tree(IsoTree {
        size: 1,
        log_yield: 1,
        damage: 0,
    });
    w.env().add_tile_object((1, 1, 0), tree, -1);
    assert_eq!(w.rand.0, [40]);
    let sink = w.obj("sink");
    w.env().add_tile_object((2, 1, 0), sink, -1);
    assert_eq!(w.rand.0, [40, 15]);
    let log = w.take_log().join("\n");
    assert!(log.contains("AmbientEmitter(TreeAmbiance)"));
    assert!(log.contains("AmbientEmitter(WaterDrip)"));
    // A plain object with the tree sprite is not an `IsoTree`: no draw.
    let tree = w.obj("tree");
    w.env().add_tile_object((3, 1, 0), tree, -1);
    assert_eq!(w.rand.0, [40, 15]);
}

#[test]
fn remove_tile_object_events_and_table_tops() {
    let mut w = World::new();
    let p = (4, 4, 0);
    let table = w.obj("table");
    let t = w.env().add_tile_object(p, table, -1).unwrap();
    let mut cup = w.obj("cup");
    cup.render_y_offset = 30.0;
    let c = w.env().add_tile_object(p, cup, -1).unwrap();
    w.take_log();
    let (index, removed) = w.env().remove_tile_object(p, t);
    assert_eq!(index, 1);
    assert_eq!(removed.unwrap().id, t);
    let cup = w.sq(p).objects.iter().find(|o| o.id == c).unwrap();
    assert_eq!(cup.render_y_offset, 6.0);
    let log = w.take_log();
    let at = |s: String| log.iter().position(|l| *l == s).unwrap();
    let about = at(format!("OnObjectAboutToBeRemoved {t}"));
    let render = at("render 0,0,0 128".into());
    let unreg = at(format!("world {t} UnregisterEntity"));
    let collision = at("collision 4,4".into());
    let gone = at(format!("OnTileRemoved {:?}", Some(t)));
    let pathfind = at("pathfind 4,4,0 +8".into());
    assert!(about < render && render < unreg && unreg < collision);
    assert!(collision < gone && gone < pathfind);
    // Not on the square: -1, and only the notifications.
    let (index, removed) = w.env().remove_tile_object_with(p, 999, false);
    assert_eq!((index, removed.is_none()), (-1, true));
    let (index, _) = w.env().transmit_remove_item_from_square(p, 999, true);
    assert_eq!(index, -1);
}

#[test]
fn add_floor_replaces_floors_keeps_rugs_and_builds_below() {
    let mut w = World::new();
    let p = (2, 2, 0);
    let rug = w.obj("floors_rugs_x");
    w.env().add_tile_object(p, rug, -1);
    let grass = w.obj("blends_grassoverlays_x");
    w.env().add_tile_object(p, grass, -1);
    w.env().add_floor(p, "floor2");
    // The old floor and the grass overlay are gone; the new floor goes
    // under the rug and its sprite becomes a solid floor.
    assert_eq!(w.names(p), ["floor2", "floors_rugs_x"]);
    let f = w.m.tables.flag.solidfloor;
    assert!(w.m.by_name("floor2").unwrap().properties.has(f));
    // A floor on level 2: the missing neighbours there and the column
    // below down to level 1 are created. The roof check runs before the
    // column is filled, so only the existing square below gets a roof.
    w.env().create_new_grid_square((5, 5, 2), false);
    w.take_log();
    w.env().add_floor((5, 5, 2), "floor");
    assert!(w.cell.get_grid_square(4, 4, 2).is_some());
    assert!(w.cell.get_grid_square(6, 6, 1).is_some());
    assert!(!w.sq((5, 5, 1)).have_roof);
    assert!(w.sq((5, 5, 0)).have_roof);
    let log = w.take_log();
    assert!(log.contains(&"newRoof 5,5,2".to_string()));
    assert!(log.contains(&"occlusion".to_string()));
    // The floor object did not join the world.
    assert!(!log.iter().any(|l| l.contains("RegisterEntity")));
}

#[test]
fn check_have_roof_follows_rain_blocking_squares() {
    let mut w = World::new();
    w.env().create_new_grid_square((1, 1, 3), true);
    let floor = w.obj("floor");
    let id = w.env().add_tile_object((1, 1, 3), floor, -1).unwrap();
    w.env().check_have_roof(1, 1);
    assert!(w.sq((1, 1, 0)).have_roof);
    assert!(!w.sq((1, 1, 3)).have_roof);
    w.env().remove_tile_object((1, 1, 3), id);
    assert!(!w.sq((1, 1, 0)).have_roof);
}

#[test]
fn special_objects_and_sheet_ropes() {
    let mut w = World::new();
    let p = (6, 6, 0);
    let rope = w.obj("rope");
    let r = w.env().add_special_object(p, rope, -1).unwrap();
    let crate_ = w.obj("crate");
    let c = w.env().add_special_object(p, crate_, -1).unwrap();
    // The sheet rope moves back on top.
    assert_eq!(w.names(p), ["floor", "crate", "rope"]);
    assert_eq!(w.sq(p).special_objects, [r, c]);
    let (i, _) = w.env().remove_tile_object(p, c);
    assert_eq!(i, 1);
    assert_eq!(w.sq(p).special_objects, [r]);
}

#[test]
fn erosion_removal_only_recalculates_properties() {
    let mut w = World::new();
    let p = (7, 1, 0);
    let wall = w.obj("wall_n");
    let id = w.env().add_tile_object(p, wall, -1).unwrap();
    let before = w.sq(p).collide_matrix;
    w.take_log();
    let (i, removed) = w.env().remove_tile_object_erosion_no_recalc(p, id);
    assert_eq!((i, removed.is_some()), (1, true));
    assert!(!w.sq(p).has(w.m.tables.flag.collideN));
    // The matrices are left as they were.
    assert_eq!(w.sq(p).collide_matrix, before);
    let log = w.take_log();
    assert!(log.iter().all(|l| l.starts_with("world")), "{log:?}");
}

#[test]
fn double_doors_are_removed_together() {
    let mut w = World::new();
    let door = |w: &World, name: &str| {
        let mut o = w.obj(name);
        o.kind = ObjectKind::Door(IsoDoor {
            north: true,
            open: false,
            locked: false,
            locked_by_key: false,
            health: 1,
            max_health: 1,
            pushed_strength: 0,
            pushed_max_strength: 0,
            open_sprite: None,
            closed_sprite: o.sprite,
            has_curtain: false,
            curtain_open: false,
            curtain_inside: false,
        });
        o
    };
    let d1 = door(&w, "dd_1");
    let a = w.env().add_special_object((3, 6, 0), d1, -1).unwrap();
    let d2 = door(&w, "dd_2");
    let b = w.env().add_special_object((4, 6, 0), d2, -1).unwrap();
    let r = ObjectRef {
        pos: (4, 6, 0),
        id: b,
    };
    assert_eq!(w.env().double_door_index(r), 2);
    assert_eq!(
        w.env().double_door_object(r, 1),
        Some(ObjectRef {
            pos: (3, 6, 0),
            id: a
        })
    );
    let (i, removed) = w.env().remove_tile_object((4, 6, 0), b);
    assert_eq!((i, removed.map(|o| o.id)), (1, Some(b)));
    assert_eq!(w.names((3, 6, 0)), ["floor"]);
}

#[test]
fn regions_hear_about_wall_changes() {
    let mut w = World::new();
    let wall = w.obj("wall_n");
    w.env().add_tile_object((2, 2, 0), wall, -1);
    // The first change reads the chunks around it into the region data.
    assert!(w.regions.queued_jobs().count() > 0);
    w.regions.worker_tick();
    assert!(w.regions.get_data_chunk(0, 0).is_some());
    let flags = crate::isoregion::calculate_square_flags(Some(w.sq((2, 2, 0))), &w.m);
    assert_eq!(w.regions.get_square_flags(2, 2, 0), flags);
    // A later change of a known chunk is a square update.
    let id = w.sq((2, 2, 0)).objects[1].id;
    w.env().remove_tile_object((2, 2, 0), id);
    assert!(w.regions.queued_jobs().any(|j| matches!(
        j,
        crate::isoregion::RegionJob::SquareUpdate { x: 2, y: 2, .. }
    )));
}

fn body() -> IsoDeadBody {
    use crate::save::visual::{CorpseVisual, HumanVisual, MovingBase};
    IsoDeadBody {
        base: MovingBase {
            offset_x: 0.0,
            offset_y: 0.0,
            x: 1.5,
            y: 2.5,
            z: 0.0,
            dir: 0,
            mod_data: None,
        },
        female: false,
        was_zombie: true,
        animal: None,
        object_id: (1, 0),
        persistent_outfit_id: 0,
        desc: None,
        visual: CorpseVisual::Human(Box::new(HumanVisual::new(0))),
        container: None,
        worn_items: vec![],
        attached_items: vec![],
        death_time: 0.0,
        reanimate_time: 0.0,
        fall_on_front: false,
        killed_by_fall: false,
        was_skeleton: false,
        angle: 0.0,
        zombie_rot_stage_at_death: 0,
        animal_rot_stage_at_death: 0,
        rotten_texture: String::new(),
        skel_inv_icon: String::new(),
        crawling: false,
        fake_dead: false,
        ragdoll: None,
    }
}

#[test]
fn corpses() {
    let mut w = World::new();
    let p = (1, 2, 0);
    let burnt = w.m.tables.flag.burntOut;
    {
        let sq = w.cell.get_grid_square_mut(1, 2, 0).unwrap();
        sq.burnt_out = true;
        sq.properties.set_flag(burnt);
    }
    w.env().add_corpse(p, body());
    assert!(!w.sq(p).burnt_out && !w.sq(p).has(burnt));
    assert_eq!(w.sq(p).static_moving.len(), 1);
    w.take_log();
    assert!(w.env().remove_corpse(p, 0).is_some());
    assert_eq!(w.take_log(), ["render 0,0,0 130", "OnContainerUpdate"]);
}

#[test]
fn recalculating_properties_clears_the_chunk_collision_data() {
    let mut w = World::new();
    let ch = w.cell.chunks.get_mut(&(0, 0)).unwrap();
    assert_eq!(*ch.collision.init(0, || 7u32).unwrap(), 7);
    assert_eq!(*ch.collision.init(1, || 8u32).unwrap(), 8);
    // Built data is kept until something changes.
    assert_eq!(*ch.collision.init(0, || 9u32).unwrap(), 7);
    assert!(ch.collision.init(8, || 0u32).is_none());
    let wall = w.obj("wall_n");
    w.env().add_tile_object((3, 3, 0), wall, -1);
    let ch = w.cell.chunks.get_mut(&(0, 0)).unwrap();
    // `clear()` only flags; the levels are dropped at the next `init`.
    assert!(ch.collision.is_clear_pending());
    assert_eq!(ch.collision.get::<u32>(1), Some(&8));
    assert_eq!(*ch.collision.init(0, || 9u32).unwrap(), 9);
    assert!(!ch.collision.is_clear_pending());
    assert_eq!(ch.collision.get::<u32>(1), None);
    // The neighbouring chunk's squares are recalculated too (the wall's
    // neighbourhood does not reach it here, so it is untouched).
    assert!(!w.cell.chunks[&(1, 0)].collision.is_clear_pending());
    let wall = w.obj("wall_w");
    w.env().add_tile_object((8, 3, 0), wall, -1);
    assert!(w.cell.chunks[&(0, 0)].collision.is_clear_pending());
    assert!(w.cell.chunks[&(1, 0)].collision.is_clear_pending());
    // Unloading clears it as `IsoChunk.removeFromWorld` does.
    let mut ch = w.cell.unload_chunk(1, 0, None).unwrap();
    ch.collision.init(0, || 1u8);
    assert!(!w.cell.chunks.contains_key(&(1, 0)));
    assert_eq!(ch.collision.get::<u8>(0), Some(&1));
}
