//! The square mutations on hand-authored sprites and squares.

use crate::ObjectSlot;
use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use crate::objects::ObjectName;
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
    ("stairs_tn", "stairsTN", &[]),
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
    fn map_collision_square_changed(&mut self, sq: crate::SquareRef) {
        let sq = sq.borrow();
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
    // Explicit hand-authored data/trace fixture, not a live registry host.
    fn try_world_call(
        &mut self,
        obj: ObjectRef,
        object: IsoObjectRef,
        rand: &mut dyn GameRand,
        call: WorldCall,
    ) -> pz_entity::runtime_load::HostResult<()> {
        self.world_call(obj, object, rand, call);
        Ok(())
    }

    fn lua_event(&mut self, sq: crate::SquareRef, ev: LuaEvent) {
        let sq = sq.borrow();
        let s = match ev {
            LuaEvent::ObjectAboutToBeRemoved { index, .. } => {
                format!("OnObjectAboutToBeRemoved {}", sq.objects[index].borrow().id)
            }
            LuaEvent::TileRemoved { object } => {
                format!("OnTileRemoved {:?}", object.map(|o| o.borrow().id))
            }
            LuaEvent::ContainerUpdate => "OnContainerUpdate".into(),
        };
        self.push(s);
    }
    fn world_call(
        &mut self,
        r: ObjectRef,
        _: crate::IsoObjectRef,
        _: &mut dyn GameRand,
        c: WorldCall,
    ) {
        self.push(format!("world {} {c:?}", r.id));
    }
    fn fluid_amount(&mut self, _: ObjectRef, _: &IsoObject) -> f32 {
        5.0
    }
    fn ambient_walls_square_changed(&mut self, _: crate::SquareRef) {
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
                sq.objects.push(
                    (IsoObject::new(id, ObjectKind::Object, Some(m.named_map["floor"]))).into(),
                );
                sq.recalc_properties(&m);
                ch.set_square(i % 8, i / 8, 0, Some((sq).into()));
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

    fn sq(&self, p: SquarePos) -> crate::SquareRef {
        self.cell.get_grid_square(p.0, p.1, p.2).unwrap()
    }

    fn names(&self, p: SquarePos) -> Vec<&str> {
        self.sq(p)
            .borrow()
            .objects
            .iter()
            .map(ObjectSlot::borrow)
            .map(|o| self.m.sprites[o.sprite.tile_projection().unwrap()].name())
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
    let id = w.env().add_tile_object(p, (wall).into(), -1).unwrap().unwrap();
    assert!(w.sq(p).borrow().has(w.m.tables.flag.collideN));
    assert!(blocked(w.sq(p).borrow().collide_matrix, 0, -1, 0));
    assert!(blocked(w.sq((3, 2, 0)).borrow().collide_matrix, 0, 1, 0));
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
    w.env().add_tile_object(p, (door).into(), 0).unwrap().unwrap();
    assert_eq!(w.names(p), ["floor", "wall_n", "door_n"]);
    let wall = w.obj("wall_w");
    w.env().add_tile_object(p, (wall).into(), -1).unwrap().unwrap();
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
    w.env().add_tile_object((1, 1, 0), (tree).into(), -1).unwrap().unwrap();
    assert_eq!(w.rand.0, [40]);
    let sink = w.obj("sink");
    w.env().add_tile_object((2, 1, 0), (sink).into(), -1).unwrap().unwrap();
    assert_eq!(w.rand.0, [40, 15]);
    let log = w.take_log().join("\n");
    assert!(log.contains("AmbientEmitter(TreeAmbiance)"));
    assert!(log.contains("AmbientEmitter(WaterDrip)"));
    // A plain object with the tree sprite is not an `IsoTree`: no draw.
    let tree = w.obj("tree");
    w.env().add_tile_object((3, 1, 0), (tree).into(), -1).unwrap().unwrap();
    assert_eq!(w.rand.0, [40, 15]);
}

#[test]
fn remove_tile_object_events_and_table_tops() {
    let mut w = World::new();
    let p = (4, 4, 0);
    let table = w.obj("table");
    let t = w.env().add_tile_object(p, (table).into(), -1).unwrap().unwrap();
    let mut cup = w.obj("cup");
    cup.render_y_offset = 30.0;
    let c = w.env().add_tile_object(p, (cup).into(), -1).unwrap().unwrap();
    w.take_log();
    let (index, removed) = w.env().remove_tile_object(p, t).unwrap();
    assert_eq!(index, 1);
    assert_eq!(removed.unwrap().borrow().id, t);
    let cup_owner = w
        .sq(p)
        .borrow()
        .objects
        .iter()
        .find(|o| o.borrow().id == c)
        .unwrap()
        .ref_handle();
    let cup = cup_owner.borrow();
    assert_eq!(cup.render_y_offset, 6.0);
    drop(cup);
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
    let (index, removed) = w.env().remove_tile_object_with(p, 999, false).unwrap();
    assert_eq!((index, removed.is_none()), (-1, true));
    let (index, _) = w
        .env()
        .transmit_remove_item_from_square(p, 999, true)
        .unwrap();
    assert_eq!(index, -1);
}

#[test]
fn add_floor_replaces_floors_keeps_rugs_and_builds_below() {
    let mut w = World::new();
    let p = (2, 2, 0);
    let rug = w.obj("floors_rugs_x");
    w.env().add_tile_object(p, (rug).into(), -1).unwrap().unwrap();
    let grass = w.obj("blends_grassoverlays_x");
    w.env().add_tile_object(p, (grass).into(), -1).unwrap().unwrap();
    w.env().add_floor(p, "floor2").unwrap();
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
    w.env().add_floor((5, 5, 2), "floor").unwrap();
    assert!(w.cell.get_grid_square(4, 4, 2).is_some());
    assert!(w.cell.get_grid_square(6, 6, 1).is_some());
    assert!(!w.sq((5, 5, 1)).borrow().have_roof);
    assert!(w.sq((5, 5, 0)).borrow().have_roof);
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
    let id = w
        .env()
        .add_tile_object((1, 1, 3), (floor).into(), -1)
        .unwrap().unwrap();
    w.env().check_have_roof(1, 1);
    assert!(w.sq((1, 1, 0)).borrow().have_roof);
    assert!(!w.sq((1, 1, 3)).borrow().have_roof);
    w.env().remove_tile_object((1, 1, 3), id).unwrap();
    assert!(!w.sq((1, 1, 0)).borrow().have_roof);
}

#[test]
fn special_objects_and_sheet_ropes() {
    let mut w = World::new();
    let p = (6, 6, 0);
    let rope = w.obj("rope");
    let r = w.env().add_special_object(p, (rope).into(), -1).unwrap().unwrap();
    let crate_ = w.obj("crate");
    let c = w.env().add_special_object(p, (crate_).into(), -1).unwrap().unwrap();
    // The sheet rope moves back on top.
    assert_eq!(w.names(p), ["floor", "crate", "rope"]);
    assert_eq!(w.sq(p).borrow().special_objects, [r, c]);
    let (i, _) = w.env().remove_tile_object(p, c).unwrap();
    assert_eq!(i, 1);
    assert_eq!(w.sq(p).borrow().special_objects, [r]);
}

#[test]
fn erosion_removal_only_recalculates_properties() {
    let mut w = World::new();
    let p = (7, 1, 0);
    let wall = w.obj("wall_n");
    let id = w.env().add_tile_object(p, (wall).into(), -1).unwrap().unwrap();
    let before = w.sq(p).borrow().collide_matrix;
    w.take_log();
    let (i, removed) = w.env().remove_tile_object_erosion_no_recalc(p, id).unwrap();
    assert_eq!((i, removed.is_some()), (1, true));
    assert!(!w.sq(p).borrow().has(w.m.tables.flag.collideN));
    // The matrices are left as they were.
    assert_eq!(w.sq(p).borrow().collide_matrix, before);
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
            closed_sprite: o.sprite.tile_projection(),
            has_curtain: false,
            curtain_open: false,
            curtain_inside: false,
        });
        o
    };
    let d1 = door(&w, "dd_1");
    let a = w
        .env()
        .add_special_object((3, 6, 0), (d1).into(), -1)
        .unwrap().unwrap();
    let d2 = door(&w, "dd_2");
    let b = w
        .env()
        .add_special_object((4, 6, 0), (d2).into(), -1)
        .unwrap().unwrap();
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
    let (i, removed) = w.env().remove_tile_object((4, 6, 0), b).unwrap();
    assert_eq!((i, removed.map(|o| o.borrow().id)), (1, Some(b)));
    assert_eq!(w.names((3, 6, 0)), ["floor"]);
}

#[test]
fn regions_hear_about_wall_changes() {
    let mut w = World::new();
    let wall = w.obj("wall_n");
    w.env().add_tile_object((2, 2, 0), (wall).into(), -1).unwrap().unwrap();
    // The first change reads the chunks around it into the region data.
    assert!(w.regions.queued_jobs().count() > 0);
    w.regions.worker_tick();
    assert!(w.regions.get_data_chunk(0, 0).is_some());
    let flags = crate::isoregion::calculate_square_flags(Some(&(w.sq((2, 2, 0))).borrow()), &w.m);
    assert_eq!(w.regions.get_square_flags(2, 2, 0), flags);
    // A later change of a known chunk is a square update.
    let id = w.sq((2, 2, 0)).borrow().objects[1].borrow().id;
    w.env().remove_tile_object((2, 2, 0), id).unwrap();
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
        burn_timer: 0.0,
    }
}

#[test]
fn corpses() {
    let mut w = World::new();
    let p = (1, 2, 0);
    let burnt = w.m.tables.flag.burntOut;
    {
        let sq_owner = w.cell.get_grid_square_mut(1, 2, 0).unwrap();
        let mut sq = sq_owner.borrow_mut();
        sq.burnt_out = true;
        sq.properties.set_flag(burnt);
    }
    w.env().add_corpse(p, body());
    assert!(!w.sq(p).borrow().burnt_out && !w.sq(p).borrow().has(burnt));
    assert_eq!(w.sq(p).borrow().static_moving.len(), 1);
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
    w.env().add_tile_object((3, 3, 0), (wall).into(), -1).unwrap().unwrap();
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
    w.env().add_tile_object((8, 3, 0), (wall).into(), -1).unwrap().unwrap();
    assert!(w.cell.chunks[&(0, 0)].collision.is_clear_pending());
    assert!(w.cell.chunks[&(1, 0)].collision.is_clear_pending());
    // Unloading clears it as `IsoChunk.removeFromWorld` does.
    let mut ch = w.cell.unload_chunk(1, 0, None).unwrap();
    ch.collision.init(0, || 1u8);
    assert!(!w.cell.chunks.contains_key(&(1, 0)));
    assert_eq!(ch.collision.get::<u8>(0), Some(&1));
}

// ---- AddWorldInventoryItem and the floor drops ----

fn world_item(ty: &str, id: i32) -> pz_inventory::shared::ItemRef {
    let mut t = pz_inventory::ItemTemplate::new(pz_inventory::ItemClass::Normal, "Base", ty);
    t.display_name = format!("{ty} name");
    t.actual_weight = 20.0;
    let mut i = InventoryItem::create(Rc::new(t), 1);
    i.id = id;
    i.name = format!("{ty} name");
    i.key_id = 77;
    Rc::new(RefCell::new(i))
}

fn floor_owner(id: u32, item: crate::floor_item::FloorItem) -> IsoObjectRef {
    ObjectSlot::new(IsoObject::new(
        id,
        ObjectKind::WorldInventory(crate::floor_object::FloorObject::new(
            crate::objects::IsoWorldInventoryObject {
                xoff: 0.0,
                yoff: 0.0,
                zoff: 0.0,
                offset_x: 0.0,
                offset_y: 0.0,
                item,
                drop_time: 0.0,
                ignore_remove_sandbox: false,
                extended_placement: false,
            },
        )),
        None,
    ))
    .ref_handle()
}

#[test]
fn floor_weight_reads_independent_actual_members_duplicates_and_raw_items() {
    let sq = IsoGridSquare::new(0, 0, 0);
    let a = world_item("A", 1);
    let b = world_item("B", 2);
    a.borrow_mut().actual_weight = 3.0;
    b.borrow_mut().actual_weight = 4.0;
    // Equal local IDs do not merge distinct allocations. Neither belongs to main.
    let ao = floor_owner(7, crate::floor_item::FloorItem::retain(&a));
    let bo = floor_owner(7, crate::floor_item::FloorItem::retain(&b));
    let raw = floor_owner(
        8,
        crate::floor_item::FloorItem::from_saved(pz_inventory::saved::SavedItem::Raw(vec![
            0, 1, 0,
        ])),
    );
    {
        let mut list = sq.world_objects.borrow_mut();
        list.push_object(&ao);
        list.push_object(&raw);
        list.push_object(&bo);
        list.push_object(&ao);
    }
    let visits = RefCell::new(Vec::new());
    assert_eq!(
        sq.get_total_weight_of_items_on_floor(|_, item| {
            visits.borrow_mut().push(item.id);
            item.actual_weight
        }),
        Ok(10.0)
    );
    assert_eq!(*visits.borrow(), [1, 2, 1]);
    assert!(sq.objects.is_empty());
    a.borrow_mut().actual_weight = 5.0;
    assert_eq!(
        sq.get_total_weight_of_items_on_floor(|_, i| i.actual_weight),
        Ok(14.0)
    );
}

#[test]
fn floor_weight_reports_first_bad_entry_after_prior_calculations() {
    use crate::world_objects::{WorldObjectEntry as Entry, WorldObjectsReadError as Error};
    use pz_kahlua::Value;
    let sq = IsoGridSquare::new(0, 0, 0);
    let item = world_item("A", 1);
    let actual = floor_owner(7, crate::floor_item::FloorItem::retain(&item));
    let non_floor = ObjectSlot::new(IsoObject::new(7, ObjectKind::Object, None)).ref_handle();
    for (entry, expected) in [
        (Entry::Foreign(Value::Nil), Error::NullEntry { index: 1 }),
        (
            Entry::Foreign(Value::str("foreign")),
            Error::WrongType { index: 1 },
        ),
        (Entry::Object(non_floor), Error::WrongType { index: 1 }),
    ] {
        let old = sq.world_objects.borrow_mut().take_entries();
        drop(old);
        {
            let mut list = sq.world_objects.borrow_mut();
            list.push_object(&actual);
            list.push_entry(entry);
            list.push_object(&actual);
        }
        let visits = std::cell::Cell::new(0);
        assert_eq!(
            sq.get_total_weight_of_items_on_floor(|_, _| {
                visits.set(visits.get() + 1);
                1.0
            }),
            Err(expected)
        );
        assert_eq!(visits.get(), 1);
        assert_eq!(sq.world_objects.borrow().len(), 3);
    }
}

#[test]
fn floor_weight_releases_list_and_floor_guards_and_reads_live_size() {
    let sq = IsoGridSquare::new(0, 0, 0);
    let a = world_item("A", 1);
    let b = world_item("B", 2);
    let ao = floor_owner(7, crate::floor_item::FloorItem::retain(&a));
    let bo = floor_owner(8, crate::floor_item::FloorItem::retain(&b));
    let floor = match &ao.borrow().kind {
        ObjectKind::WorldInventory(floor) => floor.ref_handle(),
        _ => unreachable!(),
    };
    sq.world_objects.borrow_mut().push_object(&ao);
    let visits = RefCell::new(Vec::new());
    assert_eq!(
        sq.get_total_weight_of_items_on_floor(|_, item| {
            visits.borrow_mut().push(item.id);
            if item.id == 1 {
                // A pure Rust calculator can reach these independent owners. The
                // list/floor guards must already be gone; the next index is live.
                floor.borrow_mut().drop_time = 42.0;
                sq.world_objects.borrow_mut().push_object(&bo);
            }
            1.0
        }),
        Ok(2.0)
    );
    assert_eq!(*visits.borrow(), [1, 2]);
    assert_eq!(floor.borrow().drop_time, 42.0);
}

#[test]
fn floor_weight_host_borrows_are_distinct_from_native_null_and_cast_errors() {
    use crate::world_objects::WorldObjectsReadError as Error;
    let sq = IsoGridSquare::new(0, 0, 0);
    let item = world_item("A", 1);
    let actual = floor_owner(7, crate::floor_item::FloorItem::retain(&item));
    let floor = match &actual.borrow().kind {
        ObjectKind::WorldInventory(floor) => floor.ref_handle(),
        _ => unreachable!(),
    };
    sq.world_objects.borrow_mut().push_object(&actual);
    let calculate = || sq.get_total_weight_of_items_on_floor(|_, i| i.actual_weight);
    let list_guard = sq.world_objects.borrow_mut();
    assert_eq!(calculate(), Err(Error::BorrowedList { index: 0 }));
    drop(list_guard);
    let object_guard = actual.borrow_mut();
    assert_eq!(calculate(), Err(Error::BorrowedObject { index: 0 }));
    drop(object_guard);
    let floor_guard = floor.borrow_mut();
    assert_eq!(calculate(), Err(Error::BorrowedFloor { index: 0 }));
    drop(floor_guard);
    let item_guard = item.borrow_mut();
    assert_eq!(calculate(), Err(Error::BorrowedItem { index: 0 }));
    drop(item_guard);
    assert_eq!(calculate(), Ok(20.0));
    assert!(sq.world_objects.borrow().contains_object(&actual));
}

#[test]
fn floor_placement_callbacks_keep_original_receiver_after_same_id_replacement() {
    struct Calls {
        square: crate::SquareRef,
        actual: Option<IsoObjectRef>,
        replacement: Option<IsoObjectRef>,
        calls: usize,
        designated: bool,
    }
    impl SquareMutationHooks for Calls {
        // Explicit hand-authored data/trace fixture, not a live registry host.
        fn try_world_call(
            &mut self,
            obj: ObjectRef,
            object: IsoObjectRef,
            rand: &mut dyn GameRand,
            call: WorldCall,
        ) -> pz_entity::runtime_load::HostResult<()> {
            self.world_call(obj, object, rand, call);
            Ok(())
        }

        fn world_call(
            &mut self,
            _: ObjectRef,
            actual: IsoObjectRef,
            _: &mut dyn GameRand,
            _: WorldCall,
        ) {
            if let Some(first) = &self.actual {
                assert!(Rc::ptr_eq(first, &actual));
            } else {
                let copy = ObjectSlot::new(actual.borrow().clone());
                self.replacement = Some(copy.ref_handle());
                let mut sq = self.square.borrow_mut();
                let index = sq
                    .objects
                    .iter()
                    .position(|o| o.ptr_eq_ref(&actual))
                    .unwrap();
                sq.replace_object_slot(index, copy);
                self.actual = Some(actual.clone());
            }
            actual.borrow_mut().key_id += 1;
            self.calls += 1;
        }
        fn designation_zone_item_on_ground(&mut self, _: ObjectRef, actual: IsoObjectRef) {
            assert!(Rc::ptr_eq(self.actual.as_ref().unwrap(), &actual));
            actual.borrow_mut().name = Some(ObjectName::Str("original receiver".into()));
            self.designated = true;
        }
    }
    let mut w = World::new();
    let p = (2, 2, 0);
    let sq = w.sq(p);
    let mut calls = Calls {
        square: sq.clone(),
        actual: None,
        replacement: None,
        calls: 0,
        designated: false,
    };
    let item = world_item("A", 1);
    {
        let mut env = w.env();
        env.world = &mut calls;
        assert!(matches!(
            env.add_world_inventory_item_data_projection(p, &item, 0.5, 0.5, 0.0, 0.0),
            WorldItemPlaced::Object(_)
        ));
    }
    assert!(calls.calls > 1 && calls.designated);
    let original = calls.actual.as_ref().unwrap();
    let replacement = calls.replacement.as_ref().unwrap();
    assert_eq!(original.borrow().id, replacement.borrow().id);
    assert!(sq.borrow().world_objects.borrow().contains_object(original));
    assert!(!sq
        .borrow()
        .world_objects
        .borrow()
        .contains_object(replacement));
    assert!(sq
        .borrow()
        .objects
        .iter()
        .any(|o| o.ptr_eq_ref(replacement)));
    assert!(!sq.borrow().objects.iter().any(|o| o.ptr_eq_ref(original)));
    assert_eq!(
        original.borrow().name,
        Some(ObjectName::Str("original receiver".into()))
    );
}

fn live_world_component(
    actual: &IsoObjectRef,
    kind: pz_entity::ComponentType,
) -> pz_entity::ComponentRef {
    let component =
        pz_entity::RuntimeComponent::new_unowned(pz_entity::Component::create(kind).unwrap())
            .unwrap();
    let mut retired = pz_entity::RetiredComponents::new();
    assert!(
        crate::floor_object::runtime::add_before_world(actual, &component, &mut retired).unwrap()
    );
    drop(retired);
    component
}

#[test]
fn floor_removal_transfers_actual_fluid_and_removes_only_first_actual_membership() {
    use crate::world_objects::WorldObjectEntry as Entry;
    let mut w = World::new();
    let p = (2, 2, 0);
    let item = world_item("A", 1);
    let actual = floor_owner(700, crate::floor_item::FloorItem::retain(&item));
    let fluid = live_world_component(&actual, pz_entity::ComponentType::FluidContainer);
    let mut retired = pz_entity::RetiredComponents::new();
    crate::floor_object::runtime::publish_item_backlink(&actual, &item, &mut retired).unwrap();
    drop(retired);
    let replacement = ObjectSlot::new(actual.borrow().clone());
    let replacement_owner = replacement.ref_handle();
    let sq = w.sq(p);
    {
        let mut sq = sq.borrow_mut();
        sq.objects.push(ObjectSlot::retain(&actual));
        sq.objects.push(replacement);
        sq.objects.push(ObjectSlot::retain(&actual));
        sq.special_objects.push(700);
        let mut list = sq.world_objects.borrow_mut();
        list.push_object(&actual);
        list.push_entry(Entry::Foreign(pz_kahlua::Value::Nil));
        list.push_object(&replacement_owner);
        list.push_object(&actual);
    }
    let removed = super::lifecycle::remove_from_square_handle(
        &mut w.env(),
        ObjectRef { pos: p, id: 700 },
        &actual,
    )
    .unwrap()
    .unwrap();
    assert!(removed.ptr_eq_ref(&actual));
    assert!(actual
        .borrow()
        .components
        .get_ref(pz_entity::ComponentType::FluidContainer)
        .unwrap()
        .is_none());
    assert!(Rc::ptr_eq(
        &item
            .borrow()
            .components
            .get_ref(pz_entity::ComponentType::FluidContainer)
            .unwrap()
            .unwrap(),
        &fluid
    ));
    assert!(fluid
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&pz_entity::EntityOwner::from_ref(&item)));
    assert!(
        item.borrow()
            .world_item_slot()
            .unwrap()
            .ptr_eq(&pz_entity::EntityOwner::from_ref(&actual)),
        "native removal must not clear worldItem"
    );
    assert!(
        w.h.0.borrow().is_empty(),
        "fluid transfer is an actual storage phase, not a WorldCall notification"
    );
    let sq = sq.borrow();
    assert_eq!(
        sq.objects.iter().filter(|o| o.ptr_eq_ref(&actual)).count(),
        1
    );
    assert!(sq.objects.iter().any(|o| o.ptr_eq_ref(&replacement_owner)));
    assert!(sq.special_objects.is_empty());
    let list = sq.world_objects.borrow();
    assert_eq!(list.len(), 3);
    assert!(matches!(
        list.entry(0),
        Some(Entry::Foreign(pz_kahlua::Value::Nil))
    ));
    assert!(matches!(list.entry(1), Some(Entry::Object(o)) if Rc::ptr_eq(&o, &replacement_owner)));
    assert!(matches!(list.entry(2), Some(Entry::Object(o)) if Rc::ptr_eq(&o, &actual)));
}

#[test]
fn floor_removal_transfer_requires_real_fluid_component_and_floor_kind() {
    use pz_entity::ComponentType;
    for (floor, marker, component) in [
        (true, false, None),
        (true, true, None),
        (true, true, Some(ComponentType::Attributes)),
        (false, true, Some(ComponentType::FluidContainer)),
    ] {
        let mut w = World::new();
        let p = (2, 2, 0);
        let actual = if floor {
            floor_owner(
                700,
                crate::floor_item::FloorItem::retain(&world_item("A", 1)),
            )
        } else {
            ObjectSlot::new(IsoObject::new(700, ObjectKind::Object, None)).ref_handle()
        };
        {
            let mut object = actual.borrow_mut();
            object.has_components = marker;
        }
        if let Some(kind) = component {
            live_world_component(&actual, kind);
        }
        let square = w.sq(p);
        {
            let mut square = square.borrow_mut();
            square.objects.push(ObjectSlot::retain(&actual));
            square.special_objects.push(700);
            square.world_objects.borrow_mut().push_object(&actual);
        }
        let removed = super::lifecycle::remove_from_square_handle(
            &mut w.env(),
            ObjectRef { pos: p, id: 700 },
            &actual,
        )
        .unwrap()
        .unwrap();
        assert!(removed.ptr_eq_ref(&actual));
        assert!(
            w.h.0.borrow().is_empty(),
            "unexpected fluid notification: floor={floor}, marker={marker}, component={component:?}"
        );
        let square = square.borrow();
        assert!(!square.objects.iter().any(|o| o.ptr_eq_ref(&actual)));
        assert!(square.special_objects.is_empty());
        assert_eq!(square.world_objects.borrow().len(), usize::from(!floor));
    }
}

#[test]
fn floor_transfer_precedes_list_removal_and_preserves_completed_prefix_on_list_borrow_fault() {
    let mut w = World::new();
    let p = (2, 2, 0);
    let item = world_item("A", 1);
    let actual = floor_owner(700, crate::floor_item::FloorItem::retain(&item));
    let fluid = live_world_component(&actual, pz_entity::ComponentType::FluidContainer);
    let square = w.sq(p);
    let list = square.borrow().world_objects.clone();
    square
        .borrow_mut()
        .objects
        .push(ObjectSlot::retain(&actual));
    list.borrow_mut().push_object(&actual);
    let before_main: Vec<_> = square
        .borrow()
        .objects
        .iter()
        .map(ObjectSlot::ref_handle)
        .collect();
    let guard = list.borrow();
    let error = super::lifecycle::remove_from_square_handle(
        &mut w.env(),
        ObjectRef { pos: p, id: 700 },
        &actual,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        pz_entity::runtime_load::RuntimeHostError::Borrowed("removeFromSquare world list")
    ));
    assert!(actual
        .borrow()
        .components
        .get_ref(pz_entity::ComponentType::FluidContainer)
        .unwrap()
        .is_none());
    assert!(Rc::ptr_eq(
        &item
            .borrow()
            .components
            .get_ref(pz_entity::ComponentType::FluidContainer)
            .unwrap()
            .unwrap(),
        &fluid
    ));
    assert!(fluid
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&pz_entity::EntityOwner::from_ref(&item)));
    assert_eq!(guard.len(), 1);
    {
        let square = square.borrow();
        assert_eq!(square.objects.len(), before_main.len());
        for (after, before) in square.objects.iter().zip(&before_main) {
            assert!(
                after.ptr_eq_ref(before),
                "every preexisting and appended main-list owner keeps its position"
            );
        }
    }
    assert!(w.h.0.borrow().is_empty());
    drop(guard);
}

#[test]
fn floor_transfer_faults_are_visible_and_do_not_remove_memberships() {
    use pz_entity::runtime_load::RuntimeHostError;
    for fault in ["encoded", "raw", "borrowed-item", "borrowed-component"] {
        let mut w = World::new();
        let p = (2, 2, 0);
        let item = world_item("A", 1);
        let floor = if fault == "raw" {
            crate::floor_item::FloorItem::from_saved(pz_inventory::SavedItem::Raw(vec![0, 7, 0xff]))
        } else {
            crate::floor_item::FloorItem::retain(&item)
        };
        let actual = floor_owner(700, floor);
        let fluid = if fault == "encoded" {
            actual
                .borrow_mut()
                .components
                .push_encoded_blob(
                    pz_inventory::component::encode_component(
                        pz_entity::Component::FluidContainer(Default::default()),
                    ),
                    249,
                )
                .unwrap();
            None
        } else {
            Some(live_world_component(
                &actual,
                pz_entity::ComponentType::FluidContainer,
            ))
        };
        let square = w.sq(p);
        square
            .borrow_mut()
            .objects
            .push(ObjectSlot::retain(&actual));
        square
            .borrow()
            .world_objects
            .borrow_mut()
            .push_object(&actual);
        let before_main: Vec<_> = square
            .borrow()
            .objects
            .iter()
            .map(ObjectSlot::ref_handle)
            .collect();
        let item_guard = (fault == "borrowed-item").then(|| item.borrow());
        let component_guard =
            (fault == "borrowed-component").then(|| fluid.as_ref().unwrap().borrow());
        let error = super::lifecycle::remove_from_square_handle(
            &mut w.env(),
            ObjectRef { pos: p, id: 700 },
            &actual,
        )
        .unwrap_err();
        assert!(
            match fault {
                "encoded" => matches!(error, RuntimeHostError::Storage(_)),
                "raw" => matches!(error, RuntimeHostError::Unsupported { .. }),
                _ => matches!(error, RuntimeHostError::Borrowed(_)),
            },
            "{fault}: {error:?}"
        );
        {
            let square = square.borrow();
            assert_eq!(square.objects.len(), before_main.len(), "{fault}");
            for (after, before) in square.objects.iter().zip(&before_main) {
                assert!(
                    after.ptr_eq_ref(before),
                    "{fault}: every preexisting and appended main-list owner keeps its position"
                );
            }
        }
        assert_eq!(square.borrow().world_objects.borrow().len(), 1);
        assert!(w.h.0.borrow().is_empty());
        drop(component_guard);
        drop(item_guard);
        let weak = Rc::downgrade(&actual);
        // Failed receivers and detached component prefixes remain in the
        // cell's outer-boundary retirement queue, never dropped under guards.
        drop(actual);
        assert!(weak.upgrade().is_some());
        drop(w.cell.take_retired_owners());
    }
}

#[test]
fn detached_floor_list_removal_retires_final_owner_until_guards_end() {
    struct Probe {
        square: std::rc::Weak<RefCell<IsoGridSquare>>,
        list: std::rc::Weak<RefCell<crate::world_objects::WorldObjects>>,
        drops: Rc<std::cell::Cell<usize>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            let square = self.square.upgrade().unwrap();
            let list = self.list.upgrade().unwrap();
            assert!(square.try_borrow_mut().is_ok());
            assert!(list.try_borrow_mut().is_ok());
            self.drops.set(self.drops.get() + 1);
        }
    }
    let mut w = World::new();
    let p = (2, 2, 0);
    let square = w.sq(p);
    let list = square.borrow().world_objects.clone();
    let drops = Rc::new(std::cell::Cell::new(0));
    let template =
        pz_inventory::ItemTemplate::new(pz_inventory::ItemClass::Container, "Base", "Bag");
    let bag = Rc::new(RefCell::new(InventoryItem::create(Rc::new(template), 1)));
    let inventory = pz_inventory::shared::inventory_of(&bag.borrow()).unwrap();
    inventory.borrow_mut().parent = Some(Rc::new(Probe {
        square: Rc::downgrade(&square),
        list: Rc::downgrade(&list),
        drops: drops.clone(),
    }));
    let actual = floor_owner(700, crate::floor_item::FloorItem::retain(&bag));
    let weak = Rc::downgrade(&actual);
    list.borrow_mut().push_object(&actual);
    drop(inventory);
    drop(bag);
    // Deliberately absent from main: retiring only remove_object_slot's return
    // would miss this independent list's last native reference.
    assert!(super::lifecycle::remove_from_square_handle(
        &mut w.env(),
        ObjectRef { pos: p, id: 700 },
        &actual
    )
    .unwrap()
    .is_none());
    assert!(w.h.0.borrow().is_empty());
    drop(actual);
    assert!(list.borrow().is_empty());
    assert!(weak.upgrade().is_some());
    assert_eq!(drops.get(), 0);
    let retired = square.borrow_mut().take_retired_objects();
    assert_eq!(retired.len(), 1);
    assert_eq!(drops.get(), 0);
    drop(retired);
    drop(w.cell.take_retired_owners());
    assert!(weak.upgrade().is_none());
    assert_eq!(drops.get(), 1);
}

#[test]
fn floor_constructor_preserves_fluid_identity_without_backlink_then_supplied_add_publishes_after_lists(
) {
    struct OnAdd {
        square: crate::SquareRef,
        item: pz_inventory::shared::ItemRef,
        seen: bool,
    }
    impl SquareMutationHooks for OnAdd {
        // Explicit hand-authored data/trace fixture, not a live registry host.
        fn try_world_call(
            &mut self,
            obj: ObjectRef,
            object: IsoObjectRef,
            rand: &mut dyn GameRand,
            call: WorldCall,
        ) -> pz_entity::runtime_load::HostResult<()> {
            self.world_call(obj, object, rand, call);
            Ok(())
        }

        fn world_call(
            &mut self,
            _: ObjectRef,
            object: IsoObjectRef,
            _: &mut dyn GameRand,
            call: WorldCall,
        ) {
            if call != (WorldCall::ProcessWorldItems { add: true }) {
                return;
            }
            let square = self
                .square
                .try_borrow_mut()
                .expect("addToWorld retained square guard");
            let list = square
                .world_objects
                .try_borrow_mut()
                .expect("addToWorld retained world list guard");
            assert!(square.objects.iter().any(|slot| slot.ptr_eq_ref(&object)));
            assert!(list.contains_object(&object));
            assert!(self
                .item
                .try_borrow()
                .unwrap()
                .world_item_slot()
                .unwrap()
                .ptr_eq(&pz_entity::EntityOwner::from_ref(&object)));
            assert!(object.try_borrow_mut().is_ok());
            self.seen = true;
        }
    }
    for constructor_only in [true, false] {
        let mut w = World::new();
        let p = (2, 2, 0);
        let item = world_item("WaterBottle", 1);
        let fluid = pz_entity::RuntimeComponent::new_unowned(pz_entity::Component::FluidContainer(
            Default::default(),
        ))
        .unwrap();
        let color = fluid.borrow().fluid_color_slot().unwrap().clone();
        let mut retired = pz_entity::RetiredComponents::new();
        pz_inventory::component_host::ItemComponentHost::new(
            &item,
            &pz_entity::BasicContext::default(),
        )
        .add(&fluid, &mut retired)
        .unwrap();
        drop(retired);
        assert!(item.borrow().world_item_slot().is_none());
        let actual = if constructor_only {
            let slot = w
                .env()
                .construct_world_inventory_item_data_projection(&item, 0.2, 0.3, 0.0, 4.0)
                .unwrap();
            assert!(item.borrow().world_item_slot().is_none());
            assert!(w.sq(p).borrow().world_objects.borrow().is_empty());
            slot.ref_handle()
        } else {
            let mut hook = OnAdd {
                square: w.sq(p),
                item: item.clone(),
                seen: false,
            };
            let placed = {
                let mut env = w.env();
                env.world = &mut hook;
                env.try_add_world_inventory_item_data_projection(p, &item, 0.2, 0.3, 0.0, 4.0)
                    .unwrap()
            };
            assert!(matches!(placed, WorldItemPlaced::Object(_)));
            assert!(hook.seen);
            w.sq(p).borrow().objects.last().unwrap().ref_handle()
        };
        assert!(item
            .borrow()
            .components
            .get_ref(pz_entity::ComponentType::FluidContainer)
            .unwrap()
            .is_none());
        assert!(Rc::ptr_eq(
            &actual
                .borrow()
                .components
                .get_ref(pz_entity::ComponentType::FluidContainer)
                .unwrap()
                .unwrap(),
            &fluid
        ));
        assert!(Rc::ptr_eq(
            fluid.borrow().fluid_color_slot().unwrap(),
            &color
        ));
        assert!(fluid
            .borrow()
            .owner_slot()
            .unwrap()
            .ptr_eq(&pz_entity::EntityOwner::from_ref(&actual)));
        assert!(Rc::ptr_eq(
            &crate::floor_object::runtime::item_ref(&actual)
                .unwrap()
                .unwrap(),
            &item
        ));
    }
}

#[test]
fn created_and_supplied_floor_insertion_faults_preserve_distinct_native_backlink_prefixes() {
    for created in [true, false] {
        let mut w = World::new();
        let p = (2, 2, 0);
        let item = world_item("WaterBottle", 1);
        let square = w.sq(p);
        let list = square.borrow().world_objects.clone();
        let before = square.borrow().objects.len();
        let slot = created.then(|| {
            w.env()
                .construct_world_inventory_item_data_projection(&item, 0.2, 0.3, 0.0, 4.0)
                .unwrap()
        });
        assert!(item.borrow().world_item_slot().is_none());
        let guard = list.borrow_mut();
        let result = if let Some(slot) = slot {
            w.env()
                .finish_created_world_inventory_item_data_projection(p, slot)
        } else {
            w.env()
                .try_add_world_inventory_item_data_projection(p, &item, 0.2, 0.3, 0.0, 4.0)
        };
        assert!(matches!(
            result,
            Err(WorldItemPlacementError::Component(
                pz_entity::runtime_load::RuntimeHostError::Borrowed("floor placement worldObjects")
            ))
        ));
        assert_eq!(
            square.borrow().objects.len(),
            before + 1,
            "main insertion is the completed native prefix"
        );
        assert!(guard.is_empty());
        let actual = square.borrow().objects.last().unwrap().ref_handle();
        if created {
            assert!(item
                .borrow()
                .world_item_slot()
                .unwrap()
                .ptr_eq(&pz_entity::EntityOwner::from_ref(&actual)));
        } else {
            assert!(item.borrow().world_item_slot().is_none());
        }
        assert!(Rc::ptr_eq(
            &crate::floor_object::runtime::item_ref(&actual)
                .unwrap()
                .unwrap(),
            &item
        ));
        assert!(!w
            .h
            .0
            .borrow()
            .iter()
            .any(|line| line.contains("ProcessWorldItems { add: true }")));
        drop(guard);
    }
}

#[test]
fn world_item_joins_the_square() {
    let mut w = World::new();
    let p = (2, 7, 0);
    let item = world_item("Hammer", 9);
    let inv = pz_inventory::SharedContainer::default().into_ref();
    pz_inventory::shared::add_item(&inv, &item);
    pz_inventory::shared::remove_item(&inv, &item);
    let fluid = pz_entity::RuntimeComponent::new_unowned(pz_entity::Component::FluidContainer(
        Default::default(),
    ))
    .unwrap();
    let mut retired = pz_entity::RetiredComponents::new();
    pz_inventory::component_host::ItemComponentHost::new(
        &item,
        &pz_entity::BasicContext::default(),
    )
    .add(&fluid, &mut retired)
    .unwrap();
    drop(retired);
    let before = w.sq(p).borrow().objects.len();
    let placed = w
        .env()
        .add_world_inventory_item_data_projection(p, &item, 0.0, 0.5, 0.25, 123.5);
    let WorldItemPlaced::Object(r) = placed else {
        panic!("{placed:?}");
    };
    // z rotation, then the x offset (y is not 0).
    assert_eq!(w.rand.0, [360, 1000]);
    let sq = w.sq(p);
    assert_eq!(sq.borrow().objects.len(), before + 1);
    let o_owner = sq.borrow().objects.last().unwrap().ref_handle();
    assert_eq!(sq.borrow().world_objects.borrow().len(), 1);
    assert!(sq.borrow().world_objects.borrow().contains_object(&o_owner));
    let o = o_owner.borrow();
    assert_eq!(o.id, r.id);
    assert_eq!(o.name, Some(ObjectName::Str("Hammer name".into())));
    assert_eq!(o.key_id, 77);
    assert!(o.outline_on_mouseover && o.has_components);
    assert_eq!(o.components.len(), 1);
    assert!(Rc::ptr_eq(
        &o.components
            .get_ref(pz_entity::ComponentType::FluidContainer)
            .unwrap()
            .unwrap(),
        &fluid
    ));
    assert!(fluid
        .borrow()
        .owner_slot()
        .unwrap()
        .ptr_eq(&pz_entity::EntityOwner::from_ref(&o_owner)));
    let ObjectKind::WorldInventory(wi) = &o.kind else {
        panic!()
    };
    let wi = wi.borrow();
    assert_eq!(
        (wi.xoff, wi.yoff, wi.zoff, wi.drop_time),
        (0.0, 0.5, 0.25, 123.5)
    );
    assert!(Rc::ptr_eq(&wi.item.item_ref().unwrap(), &item));
    assert_eq!(wi.item.borrow_item().unwrap().id, 9);
    assert_eq!(wi.item.borrow_item().unwrap().world_z_rotation, 0.0);
    assert!(item.borrow().components.is_empty());
    assert_eq!(item.borrow().world_z_rotation, 0.0);
    drop(wi);
    drop(o);
    let log = w.take_log();
    // (2, 7) is chunk-local (2, 7): its own square renders it.
    assert_eq!(log.first().map(String::as_str), Some("render 0,0,0 68"));
    assert_eq!(log.last().map(String::as_str), Some("render 0,0,0 68"));
    assert!(log
        .iter()
        .any(|l| l.contains("ProcessWorldItems { add: true }")));
    assert!(!log
        .iter()
        .any(|l| l.starts_with("pathfind") || l.starts_with("collision")));
    // A second item: no draws for set offsets and rotation.
    w.rand.0.clear();
    let item2 = world_item("Nail", 10);
    item2.borrow_mut().world_z_rotation = 90.0;
    w.env()
        .add_world_inventory_item_data_projection(p, &item2, 0.3, 0.4, 0.0, 1.0);
    assert!(w.rand.0.is_empty());
    assert_eq!(w.sq(p).borrow().world_objects.borrow().len(), 2);
    let total = w
        .sq(p)
        .borrow()
        .get_total_weight_of_items_on_floor(|_, i| i.template.actual_weight)
        .unwrap();
    assert_eq!(total, 40.0);
    item.borrow_mut().actual_weight = 3.0;
    item2.borrow_mut().actual_weight = 7.0;
    assert_eq!(
        w.sq(p)
            .borrow()
            .get_total_weight_of_items_on_floor(|_, i| i.actual_weight)
            .unwrap(),
        10.0
    );
    // Cloning a square for an archive/pure hook keeps value-copy semantics;
    // the lifecycle hook snapshots did not replace the canonical item.
    let snapshot = w.sq(p).borrow().try_archive_copy().unwrap();
    let ObjectKind::WorldInventory(copied) = &snapshot.objects.last().unwrap().borrow().kind else {
        panic!()
    };
    let copied = copied.borrow();
    assert!(!Rc::ptr_eq(&copied.item.item_ref().unwrap(), &item2));
    let live_object = w.sq(p).borrow().objects.last().unwrap().ref_handle();
    let live_body = live_object.borrow();
    let ObjectKind::WorldInventory(live) = &live_body.kind else {
        panic!()
    };
    let live = live.borrow();
    assert!(Rc::ptr_eq(&live.item.item_ref().unwrap(), &item2));
    live.item.borrow_item_mut().unwrap().actual_weight = 11.0;
    assert_eq!(item2.borrow().actual_weight, 11.0);
    assert_eq!(copied.item.borrow_item().unwrap().actual_weight, 7.0);
}

#[test]
fn render_square_of_chunk_corners() {
    let w = World::new();
    assert_eq!(render_square(&w.cell, (0, 7, 0)), None, "no square south");
    assert_eq!(render_square(&w.cell, (7, 0, 0)), Some((8, 0, 0)));
    assert_eq!(render_square(&w.cell, (3, 3, 0)), Some((3, 3, 0)));
}

#[test]
fn corpses_generators_and_animals_go_to_hooks() {
    let mut w = World::new();
    let p = (3, 3, 0);
    let corpse = world_item("CorpseMale", 1);
    assert_eq!(
        w.env()
            .add_world_inventory_item_data_projection(p, &corpse, 0.5, 0.5, 0.0, 0.0),
        WorldItemPlaced::NotPlaced
    );
    let gen = world_item("Generator", 2);
    assert_eq!(
        w.env()
            .add_world_inventory_item_data_projection(p, &gen, 0.5, 0.5, 0.0, 0.0),
        WorldItemPlaced::NotPlaced
    );
    let mut t = (*gen.borrow().template).clone();
    t.class = pz_inventory::ItemClass::Animal;
    t.type_ = "Animal".into();
    let animal = Rc::new(RefCell::new(InventoryItem::create(Rc::new(t), 1)));
    assert_eq!(
        w.env()
            .add_world_inventory_item_data_projection(p, &animal, 0.5, 0.5, 0.0, 0.0),
        WorldItemPlaced::Animal
    );
    assert!(w.sq(p).borrow().world_objects.borrow().is_empty());
}

#[test]
fn solid_floor_and_apparent_z() {
    let mut w = World::new();
    w.env().create_new_grid_square((4, 4, 1), true);
    // Level 1 has no floor: level 0's.
    assert_eq!(w.cell.get_solid_floor_at(4, 4, 1, &w.m), Some((4, 4, 0)));
    assert_eq!(w.cell.get_solid_floor_at(4, 4, 3, &w.m), Some((4, 4, 0)));
    assert_eq!(w.cell.get_solid_floor_at(40, 4, 0, &w.m), None);
    let p = (5, 5, 0);
    let o = w.obj("stairs_tn");
    w.env().add_tile_object(p, (o).into(), -1).unwrap().unwrap();
    let sq = w.sq(p);
    assert!(sq.borrow().has_type(w.m.tables.ty.stairsTN));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    assert_eq!(
        sq.borrow().get_apparent_z(0.5, 0.25, &w.m, true),
        lerp(0.7666, 1.0, 0.75)
    );
    assert_eq!(sq.borrow().get_apparent_z(0.5, 2.0, &w.m, false), 0.6666);
    assert_eq!(
        w.sq((6, 6, 0))
            .borrow()
            .get_apparent_z(0.5, 0.5, &w.m, true),
        0.0
    );
}

/// A player holding `items` on (3, 3, 1) with a 50 capacity.
struct Carrier {
    inv: pz_inventory::ContainerRef,
}

impl pz_inventory::floor_drop::DropCharacter for Carrier {
    fn inventory(&self) -> pz_inventory::ContainerRef {
        self.inv.clone()
    }
    fn is_player(&self) -> bool {
        true
    }
    fn is_in_vehicle(&self) -> bool {
        false
    }
    fn current_square(&self) -> Option<SquarePos> {
        Some((3, 3, 1))
    }
    fn capacity_weight(&self) -> f32 {
        self.inv
            .borrow()
            .items
            .iter()
            .map(|i| i.borrow().template.actual_weight)
            .sum()
    }
    fn unequipped_weight(&self, item: &pz_inventory::shared::ItemRef) -> f32 {
        item.borrow().template.actual_weight
    }
    fn primary_hand_item(&self) -> Option<pz_inventory::shared::ItemRef> {
        None
    }
    fn secondary_hand_item(&self) -> Option<pz_inventory::shared::ItemRef> {
        None
    }
    fn set_primary_hand_item(&mut self, _: Option<&pz_inventory::shared::ItemRef>) {}
    fn set_secondary_hand_item(&mut self, _: Option<&pz_inventory::shared::ItemRef>) {}
}

#[test]
fn too_heavy_worn_item_drops_to_the_floor_below() {
    let mut w = World::new();
    w.env().create_new_grid_square((3, 3, 1), true);
    let coat = world_item("Coat", 1);
    let rock = world_item("Rock", 2);
    let anvil = world_item("Anvil", 3);
    let inv = pz_inventory::SharedContainer::default().into_ref();
    for i in [&coat, &rock, &anvil] {
        pz_inventory::shared::add_item(&inv, i);
    }
    let mut chr = Carrier { inv };
    let mut env = w.env();
    let mut floor = FloorDropEnv::new_data_projection(&mut env, 10.0);
    let d = pz_inventory::floor_drop::force_drop_too_heavy(&mut chr, &mut floor, true, Some(&coat))
        .unwrap()
        .unwrap();
    // `Rand.Next(0.1f, 0.9f)` with a generator answering 0.
    assert_eq!((d.square, d.x, d.y, d.z), ((3, 3, 0), 0.1, 0.1, 0.0));
    let placed = floor.placed;
    assert!(matches!(placed[..], [WorldItemPlaced::Object(r)] if r.pos == (3, 3, 0)));
    assert_eq!(chr.inv.borrow().items.len(), 2);
    let sq = w.sq((3, 3, 0));
    let wi_object = sq.borrow().objects.last().unwrap().ref_handle();
    let wi_body = wi_object.borrow();
    let ObjectKind::WorldInventory(wi) = &wi_body.kind else {
        panic!()
    };
    let wi = wi.borrow();
    assert_eq!((wi.xoff, wi.yoff, wi.drop_time), (0.1, 0.1, 10.0));
    assert!(Rc::ptr_eq(&wi.item.item_ref().unwrap(), &coat));
    assert_eq!(wi.item.borrow_item().unwrap().id, 1);
}

#[test]
fn removal_callback_pins_actual_receiver_recomputes_index_and_rejects_equal_replacement() {
    struct Callback {
        replacement: bool,
        target: IsoObjectRef,
        calls: usize,
    }
    impl SquareMutationHooks for Callback {
        // Explicit hand-authored data/trace fixture, not a live registry host.
        fn try_world_call(
            &mut self,
            obj: ObjectRef,
            object: IsoObjectRef,
            rand: &mut dyn GameRand,
            call: WorldCall,
        ) -> pz_entity::runtime_load::HostResult<()> {
            self.world_call(obj, object, rand, call);
            Ok(())
        }

        fn lua_event(&mut self, square: crate::SquareRef, event: LuaEvent) {
            let mut square = square.borrow_mut();
            if let LuaEvent::ObjectAboutToBeRemoved { index, ref object } = event {
                assert!(Rc::ptr_eq(&object, &self.target));
                assert!(square.objects[index].ptr_eq_ref(&object));
                // The original object can be borrowed by the callback itself.
                object.borrow_mut().key_id = 771;
                if self.replacement {
                    // Same value and local ID, genuinely different Java receiver.
                    let copy = ObjectSlot::new(object.borrow().clone());
                    square.replace_object_slot(index, copy);
                } else {
                    square.remove_object_slot(0);
                }
            }
            if let LuaEvent::TileRemoved {
                object: Some(object),
            } = event
            {
                assert!(Rc::ptr_eq(&object, &self.target));
                assert_eq!(object.borrow().key_id, 772);
            }
        }
        fn world_call(
            &mut self,
            _: ObjectRef,
            object: IsoObjectRef,
            _: &mut dyn GameRand,
            _: WorldCall,
        ) {
            assert!(Rc::ptr_eq(&object, &self.target));
            object.borrow_mut().key_id = 772;
            self.calls += 1;
        }
    }
    for replacement in [false, true] {
        let mut w = World::new();
        let p = (2, 2, 0);
        let slot = ObjectSlot::new(w.obj("crate"));
        let actual = slot.ref_handle();
        let id = w.env().add_tile_object(p, slot, -1).unwrap().unwrap();
        let mut callback = Callback {
            replacement,
            target: actual.clone(),
            calls: 0,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut env = w.env();
            env.world = &mut callback;
            env.remove_tile_object_with(p, id, false)
        }));
        if replacement {
            assert!(matches!(
                result.unwrap(),
                Err(pz_entity::runtime_load::RuntimeHostError::InvalidHostState(
                    _
                ))
            ));
            assert_eq!(callback.calls, 0);
            assert!(!w.sq(p).borrow().objects[1].ptr_eq_ref(&actual));
            assert_eq!(actual.borrow().key_id, 771);
        } else {
            let (index, removed) = result.unwrap().unwrap();
            assert_eq!(index, 0); // callback removed preceding floor
            assert!(removed.unwrap().ptr_eq_ref(&actual));
            assert!(callback.calls > 0);
            assert_eq!(actual.borrow().key_id, 772);
            assert!(w.sq(p).borrow().objects.is_empty());
        }
    }
}

#[test]
fn detached_remove_from_world_mutates_actual_receiver_without_reinsertion() {
    struct Calls {
        actual: IsoObjectRef,
        count: usize,
    }
    impl SquareMutationHooks for Calls {
        // Explicit hand-authored data/trace fixture, not a live registry host.
        fn try_world_call(
            &mut self,
            obj: ObjectRef,
            object: IsoObjectRef,
            rand: &mut dyn GameRand,
            call: WorldCall,
        ) -> pz_entity::runtime_load::HostResult<()> {
            self.world_call(obj, object, rand, call);
            Ok(())
        }

        fn world_call(
            &mut self,
            _: ObjectRef,
            object: IsoObjectRef,
            _: &mut dyn GameRand,
            _: WorldCall,
        ) {
            assert!(Rc::ptr_eq(&self.actual, &object));
            object.borrow_mut().key_id += 1;
            self.count += 1;
        }
    }
    let mut w = World::new();
    let p = (2, 2, 0);
    let slot = ObjectSlot::new(w.obj("crate"));
    let actual = slot.ref_handle();
    let id = w.env().add_tile_object(p, slot, -1).unwrap().unwrap();
    let removed = w.env().delete_tile_object(p, id).unwrap();
    assert!(removed.ptr_eq_ref(&actual));
    let count = w.sq(p).borrow().objects.len();
    let mut calls = Calls {
        actual: actual.clone(),
        count: 0,
    };
    {
        let mut env = w.env();
        env.world = &mut calls;
        env.object_remove_from_world_detached(ObjectRef { pos: p, id }, &actual)
            .unwrap();
    }
    assert!(calls.count > 0);
    assert_eq!(actual.borrow().key_id, calls.count as i32 - 1); // initial keyId is -1
    assert_eq!(w.sq(p).borrow().objects.len(), count);
    assert!(w.sq(p).borrow().object_index(id).is_none());
}

#[test]
fn detached_receiver_removal_event_keeps_original_argument_and_same_id_replacement() {
    struct Event {
        actual: IsoObjectRef,
        seen: bool,
    }
    impl SquareMutationHooks for Event {
        // Explicit hand-authored data/trace fixture, not a live registry host.
        fn try_world_call(
            &mut self,
            obj: ObjectRef,
            object: IsoObjectRef,
            rand: &mut dyn GameRand,
            call: WorldCall,
        ) -> pz_entity::runtime_load::HostResult<()> {
            self.world_call(obj, object, rand, call);
            Ok(())
        }

        fn lua_event(&mut self, _: crate::SquareRef, event: LuaEvent) {
            match event {
                LuaEvent::ObjectAboutToBeRemoved { .. } => panic!("detached receiver is absent"),
                LuaEvent::TileRemoved {
                    object: Some(object),
                } => {
                    assert!(Rc::ptr_eq(&self.actual, &object));
                    object.borrow_mut().key_id = 444;
                    self.seen = true;
                }
                _ => panic!("original non-null argument must reach OnTileRemoved"),
            }
        }
    }
    let mut w = World::new();
    let p = (2, 2, 0);
    let original = ObjectSlot::new(w.obj("crate"));
    let actual = original.ref_handle();
    let id = w.env().add_tile_object(p, original, -1).unwrap().unwrap();
    drop(w.env().delete_tile_object(p, id));
    let mut replacement = actual.borrow().clone();
    replacement.key_id = 555;
    let replacement = ObjectSlot::new(replacement);
    let replacement_handle = replacement.ref_handle();
    w.cell
        .get_grid_square_mut(p.0, p.1, p.2)
        .unwrap()
        .borrow_mut()
        .objects
        .push(replacement);
    let mut event = Event {
        actual: actual.clone(),
        seen: false,
    };
    let (index, removed) = {
        let mut env = w.env();
        env.world = &mut event;
        env.remove_tile_object_receiver(p, id, Some(actual.clone()))
    }
    .unwrap();
    assert_eq!(index, -1);
    assert!(removed.is_none());
    assert!(event.seen);
    assert_eq!(actual.borrow().key_id, 444);
    assert_eq!(replacement_handle.borrow().key_id, 555);
    assert!(w
        .sq(p)
        .borrow()
        .objects
        .last()
        .unwrap()
        .ptr_eq_ref(&replacement_handle));
}

#[test]
fn data_projection_floor_lifecycle_does_not_archive_clone_actual_item_graph() {
    let mut world = World::new();
    let item = world_item("Bandage", 5);
    pz_inventory::moddata::set_mod_data_value_ref(
        &item,
        "live",
        Some(pz_kahlua::Value::Bool(true)),
    )
    .unwrap();
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    let _held = table.try_borrow_body_mut().unwrap();
    let placed = world
        .env()
        .try_add_world_inventory_item_data_projection((2, 2, 0), &item, 0.2, 0.3, 0.0, 1.0)
        .unwrap();
    assert!(matches!(placed, WorldItemPlaced::Object(_)));
    let slot = item.borrow().world_item_slot().cloned().unwrap();
    let actual = slot.try_handle::<IsoObject>().unwrap();
    assert!(actual.borrow().added_to_world_or_equipped);
    assert!(Rc::ptr_eq(
        &crate::floor_object::runtime::item_ref(&actual)
            .unwrap()
            .unwrap(),
        &item
    ));
}

struct CheckedRemoval {
    fail_on: Option<WorldCall>,
    calls: Vec<(WorldCall, bool)>,
}
impl SquareMutationHooks for CheckedRemoval {
    fn try_world_call(
        &mut self,
        _: ObjectRef,
        object: IsoObjectRef,
        _: &mut dyn GameRand,
        call: WorldCall,
    ) -> pz_entity::runtime_load::HostResult<()> {
        self.calls
            .push((call, object.borrow().added_to_world_or_equipped));
        if self.fail_on == Some(call) {
            return Err(pz_entity::runtime_load::RuntimeHostError::Borrowed(
                "actual removal registry",
            ));
        }
        Ok(())
    }
}
fn removal_receiver(world: &mut World, floor: bool) -> IsoObjectRef {
    if floor {
        world
            .env()
            .construct_world_inventory_item_data_projection(
                &world_item("Bandage", 5),
                0.2,
                0.3,
                0.0,
                1.0,
            )
            .unwrap()
            .ref_handle()
    } else {
        ObjectSlot::new(world.obj("crate")).ref_handle()
    }
}

#[test]
fn checked_remove_clears_added_flag_after_unregister_for_both_initial_bits_and_classes() {
    for floor in [false, true] {
        for initially_added in [false, true] {
            let mut world = World::new();
            let object = removal_receiver(&mut world, floor);
            object.borrow_mut().added_to_world_or_equipped = initially_added;
            let mut calls = CheckedRemoval {
                fail_on: None,
                calls: Vec::new(),
            };
            let location = ObjectRef {
                pos: (2, 2, 0),
                id: object.borrow().id,
            };
            {
                let mut env = world.env();
                env.world = &mut calls;
                env.object_remove_from_world_detached(location, &object)
                    .unwrap();
            }
            let unreg = calls
                .calls
                .iter()
                .position(|(call, _)| *call == WorldCall::UnregisterEntity)
                .unwrap();
            assert_eq!(
                calls.calls[unreg].1, initially_added,
                "PC13 sees previous bit"
            );
            assert_eq!(
                calls.calls[unreg + 1],
                (WorldCall::ProcessIsoObjectRemove, false),
                "PC18 clears before next system"
            );
            assert!(calls.calls[unreg + 1..].iter().all(|(_, flag)| !flag));
            assert!(!object.borrow().added_to_world_or_equipped);
        }
    }
}

#[test]
fn failed_unregister_preserves_added_flag_and_stops_generic_and_floor_remove_prefix() {
    for floor in [false, true] {
        let mut world = World::new();
        let object = removal_receiver(&mut world, floor);
        object.borrow_mut().added_to_world_or_equipped = true;
        let mut calls = CheckedRemoval {
            fail_on: Some(WorldCall::UnregisterEntity),
            calls: Vec::new(),
        };
        let location = ObjectRef {
            pos: (2, 2, 0),
            id: object.borrow().id,
        };
        let result = {
            let mut env = world.env();
            env.world = &mut calls;
            env.object_remove_from_world_detached(location, &object)
        };
        assert!(matches!(
            result,
            Err(pz_entity::runtime_load::RuntimeHostError::Borrowed(
                "actual removal registry"
            ))
        ));
        let expected = if floor {
            vec![
                WorldCall::ProcessWorldItems { add: false },
                WorldCall::WorldItemRemoved,
                WorldCall::WorldItemContainer { add: false },
                WorldCall::UnregisterEntity,
            ]
        } else {
            vec![WorldCall::UnregisterEntity]
        };
        assert_eq!(
            calls
                .calls
                .iter()
                .map(|(call, _)| *call)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(calls.calls.iter().all(|(_, flag)| *flag));
        assert!(object.borrow().added_to_world_or_equipped);
    }
}

#[test]
fn post_unregister_host_fault_keeps_cleared_flag_and_stops_following_systems() {
    let mut world = World::new();
    let object = removal_receiver(&mut world, false);
    object.borrow_mut().added_to_world_or_equipped = true;
    let mut calls = CheckedRemoval {
        fail_on: Some(WorldCall::ProcessIsoObjectRemove),
        calls: Vec::new(),
    };
    let location = ObjectRef {
        pos: (2, 2, 0),
        id: object.borrow().id,
    };
    let result = {
        let mut env = world.env();
        env.world = &mut calls;
        env.object_remove_from_world_detached(location, &object)
    };
    assert!(result.is_err());
    assert_eq!(
        calls.calls,
        vec![
            (WorldCall::UnregisterEntity, true),
            (WorldCall::ProcessIsoObjectRemove, false)
        ]
    );
    assert!(!object.borrow().added_to_world_or_equipped);
}

#[test]
fn absent_live_lifecycle_capability_never_becomes_successful_projection() {
    let mut world = World::new();
    let object = removal_receiver(&mut world, false);
    object.borrow_mut().added_to_world_or_equipped = true;
    let location = ObjectRef {
        pos: (2, 2, 0),
        id: object.borrow().id,
    };
    let result = {
        let mut env = world.env();
        let mut missing = NoMutationHooks;
        env.world = &mut missing;
        env.object_remove_from_world_detached(location, &object)
    };
    assert!(matches!(
        result,
        Err(RuntimeHostError::Unsupported {
            operation: "native world lifecycle hook",
            ..
        })
    ));
    assert!(object.borrow().added_to_world_or_equipped);
    // The archive/geometry profile must be requested by name; it is no live
    // Native registry claim, even when its scalar/list projection completes.
    let mut projection = DataProjectionMutationHooks;
    {
        let mut env = world.env();
        env.world = &mut projection;
        env.object_remove_from_world_detached(location, &object)
            .unwrap();
    }
    assert!(!object.borrow().added_to_world_or_equipped);
}

#[test]
fn native_render_hook_requires_actual_capability_before_constructor_while_projection_is_explicit() {
    let mut world = World::new();
    let item = world_item("Bandage", 5);
    {
        let mut body = item.borrow_mut();
        body.world_x_rotation = 22.0;
        body.world_y_rotation = 33.0;
        body.world_z_rotation = 9.0;
    }
    let result = world
        .env()
        .try_add_world_inventory_item((2, 2, 0), &item, 0.2, 0.3, 0.0);
    assert!(matches!(
        result,
        Err(WorldItemPlacementError::Component(
            RuntimeHostError::Unsupported {
                operation: "native floor render invalidation",
                ..
            }
        ))
    ));
    assert_eq!(
        (
            item.borrow().world_x_rotation,
            item.borrow().world_y_rotation
        ),
        (22.0, 33.0)
    );
    assert!(
        world.take_log().is_empty(),
        "missing Native capability must not invoke legacy trace"
    );
    assert!(
        world.cell.take_retired_objects().is_empty(),
        "PC20 fails before actual constructor receiver"
    );
    assert!(item.borrow().world_item_slot().is_none());

    let result = world
        .env()
        .try_add_world_inventory_item_data_projection((2, 2, 0), &item, 0.2, 0.3, 0.0, 1.0)
        .unwrap();
    assert!(matches!(result, WorldItemPlaced::Object(_)));
    assert!(world
        .take_log()
        .iter()
        .any(|call| call == "render 0,0,0 68"));
    assert_eq!(
        (
            item.borrow().world_x_rotation,
            item.borrow().world_y_rotation
        ),
        (0.0, 0.0)
    );
    let owner = item.borrow().world_item_slot().cloned().unwrap();
    let actual = owner.try_handle::<IsoObject>().unwrap();
    assert_eq!(
        actual.borrow().pending_floor_visual_tail,
        Some(crate::objects::FloorVisualTail::ConstructorUpdateSprite),
        "explicit projection does not become Native visual completion"
    );
}


#[test]
fn add_object_host_failure_keeps_inserted_prefix_and_stops_restack_and_lighting() {
    use crate::sprite::runtime::{self, ObjectSprite};
    let mut world = World::new();
    let pos = (3, 3, 0);
    let rope = world.obj("rope");
    world
        .env()
        .add_special_object(pos, rope.into(), -1)
        .unwrap()
        .unwrap();
    world.take_log();
    world.sq(pos).borrow_mut().burnt_out = true;
    let before: Vec<_> = world
        .sq(pos)
        .borrow()
        .objects
        .iter()
        .map(ObjectSlot::ref_handle)
        .collect();
    let manager = Rc::new(RefCell::new(IsoSpriteManager::new(test_tables())));
    let mut retired = pz_entity::RetiredComponents::new();
    let sprite = runtime::create_runtime_sprite(&manager, &test_tables(), &mut retired).unwrap();
    let mut value = IsoObject::new(0, ObjectKind::Object, None);
    value.sprite = ObjectSprite::Native(sprite.clone());
    let slot = ObjectSlot::new(value);
    let actual = slot.ref_handle();
    let result = world.env().add_special_object(pos, slot, -1);
    assert!(matches!(
        result,
        Err(RuntimeHostError::Unsupported {
            operation: "actual native sprite subclass addToWorld",
            ..
        })
    ));
    let square = world.sq(pos);
    let square = square.borrow();
    assert!(!square.burnt_out, "the completed add prefix cleared burntOut");
    assert_eq!(square.objects.len(), before.len() + 1);
    for (left, right) in square.objects.iter().zip(&before) {
        assert!(left.ptr_eq_ref(right));
    }
    assert!(square.objects.last().unwrap().ptr_eq_ref(&actual));
    assert_eq!(square.special_objects.last(), Some(&actual.borrow().id));
    assert!(Rc::ptr_eq(
        actual.borrow().sprite.native_slot().unwrap(),
        &sprite
    ));
    assert!(!actual.borrow().added_to_world_or_equipped);
    assert!(
        world.take_log().is_empty(),
        "no later restack, lighting, region, path or render work after typed host failure"
    );
    drop(square);
    let absent_slot = ObjectSlot::new(world.obj("crate"));
    let absent = world
        .env()
        .transmit_add_object_to_square((99, 99, 0), absent_slot, -1);
    assert!(
        matches!(absent, Ok(None)),
        "genuine missing receiver remains separate from a host error"
    );
}

#[test]
fn successful_native_add_lifecycle_stops_before_unported_square_tail() {
    use crate::sprite::runtime::{self, ObjectSprite};
    for special in [false, true] {
        let mut world = World::new();
        let pos = (3, 3, 0);
        let rope = world.obj("rope");
        world
            .env()
            .add_special_object(pos, rope.into(), -1)
            .unwrap()
            .unwrap();
        world.take_log();
        let square = world.sq(pos);
        square.borrow_mut().burnt_out = true;
        let before: Vec<_> = square
            .borrow()
            .objects
            .iter()
            .map(ObjectSlot::ref_handle)
            .collect();
        let manager = Rc::new(RefCell::new(IsoSpriteManager::new(test_tables())));
        let mut retired = pz_entity::RetiredComponents::new();
        let sprite =
            runtime::create_runtime_sprite(&manager, &test_tables(), &mut retired).unwrap();
        let payload = crate::objects::IsoWorldInventoryObject {
            xoff: 0.5,
            yoff: 0.5,
            zoff: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            item: crate::floor_item::FloorItem::null(),
            drop_time: 1.0,
            ignore_remove_sandbox: false,
            extended_placement: false,
        };
        let mut value = IsoObject::new(
            0,
            ObjectKind::WorldInventory(crate::floor_object::FloorObject::new(payload)),
            None,
        );
        value.sprite = ObjectSprite::Native(sprite.clone());
        value.replace_square(Some(&square), &mut retired);
        let object = ObjectSlot::new(value);
        let actual = object.ref_handle();
        assert!(
            !world.cell.chunks[&(0, 0)]
                .hot_save
                .borrow()
                .requires_hot_save
        );
        let result = if special {
            world.env().add_special_object(pos, object, -1)
        } else {
            world.env().add_tile_object(pos, object, -1)
        };
        assert_eq!(
            result,
            Err(RuntimeHostError::Unsupported {
                operation: if special {
                    "native addObject restackSheetRope"
                } else {
                    "native addObject RecalcAllWithNeighbours"
                },
                kind: None,
            })
        );
        let body = square.borrow();
        assert!(!body.burnt_out);
        assert_eq!(body.objects.len(), before.len() + 1);
        for (left, right) in body.objects.iter().zip(&before) {
            assert!(left.ptr_eq_ref(right));
        }
        assert!(body.objects.last().unwrap().ptr_eq_ref(&actual));
        assert_eq!(body.special_objects.contains(&actual.borrow().id), special);
        assert!(actual.borrow().added_to_world_or_equipped);
        assert!(!actual.borrow().added_to_entity_manager);
        assert!(Rc::ptr_eq(
            actual.borrow().sprite.native_slot().unwrap(),
            &sprite
        ));
        assert!(
            world.cell.chunks[&(0, 0)]
                .hot_save
                .borrow()
                .requires_hot_save
        );
        assert!(pz_entity::runtime_event::entity_event_pool().len().unwrap() > 0);
        assert!(
            world.take_log().is_empty(),
            "completed native lifecycle is retained without legacy restack/recalc or later hooks"
        );
        drop(body);
        // Collector-style fixture teardown, without Native lifecycle/reset hooks.
        drop(actual.borrow_mut().take_square_for_gc());
        drop(sprite.borrow_mut().take_slots_for_gc());
    }
}
