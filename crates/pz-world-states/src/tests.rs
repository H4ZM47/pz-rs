//! The adapter on hand-built squares with invented objects, tables and
//! random draws (no game data).

use std::collections::HashMap;

use pz_characters::character::state::{SheetRopeExit, StateWorld, ThumpKind, ThumpTarget};
use pz_characters::{CharacterKind, GameCharacter};
use pz_pathfind::mover::{CharacterView, MoverId, ObjectRef, PathMover};
use pz_scripts::ScriptManager;
use pz_world::java::GameRand;
use pz_world::objects::{IsoBarricade, IsoDoor, IsoObject, IsoThumpable, IsoWindow, ObjectKind};
use pz_world::sprite::PropertyValueMap;
use pz_world::square::{CollidedFlags, HoppableAction, MoverKind, SquareObject};
use pz_world::{
    IsoCell, IsoChunk, IsoDirections, IsoGridSquare, IsoMetaGrid, IsoSpriteManager, SpriteTables,
};

use super::*;

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

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
            "makeWindowInvincible",
            "smoke",
            "climbSheetW",
            "climbSheetN",
            "climbSheetE",
            "climbSheetS",
            "climbSheetTopW",
            "climbSheetTopN",
            "climbSheetTopE",
            "climbSheetTopS",
            "WallN",
            "WallW",
            "WallNTrans",
            "WallWTrans",
            "DoorWallW",
            "DoorWallN",
            "isEave",
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

type Spec<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

const SPRITES: &[Spec] = &[
    ("floor", "normal", &[("solidfloor", "")]),
    ("door_n", "doorN", &[("doorN", ""), ("collideN", "")]),
    (
        "door_w_mat",
        "doorW",
        &[("doorW", ""), ("collideW", ""), ("Material", "Wood")],
    ),
    (
        "window_w",
        "normal",
        &[("windowW", ""), ("WindowW", ""), ("collideW", "")],
    ),
    (
        "window_w_smashed",
        "normal",
        &[("windowW", ""), ("WindowW", "")],
    ),
    ("crate", "normal", &[("solidtrans", "")]),
    ("brokenglass_1_2", "normal", &[]),
    ("carpentry_01_9", "normal", &[]),
    ("sneaky", "normal", &[("CloseSneakBonus", "250")]),
    ("stairs_tn", "stairsTN", &[]),
    ("stairs_mn", "stairsMN", &[]),
    ("stairs_bn", "stairsBN", &[]),
    ("fence_n", "normal", &[("collideN", ""), ("HoppableN", "")]),
    (
        "tall_fence_w",
        "normal",
        &[("collideW", ""), ("HoppableW", ""), ("TallHoppableW", "")],
    ),
    ("tree_a", "tree", &[]),
    ("shrub", "normal", &[("Bush", "")]),
    (
        "wall_n_metal",
        "wall",
        &[("collideN", ""), ("FenceTypeHigh", "Metal")],
    ),
    ("wall_w", "wall", &[("collideW", "")]),
    ("sheet_w", "normal", &[("climbSheetW", "")]),
    ("sheet_top_w", "normal", &[("climbSheetTopW", "")]),
    ("solid_block", "normal", &[("solid", "")]),
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
        "Bush",
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
    let mut m = IsoSpriteManager::new(tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (i, (name, ty, props)) in SPRITES.iter().enumerate() {
        let s = m.add_sprite(name, 500 + i as i32);
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

/// A draw sequence (`Rand.Next`), then zeros.
#[derive(Default)]
struct Seq(Vec<i32>);

impl GameRand for Seq {
    fn next(&mut self, max: i32) -> i32 {
        if self.0.is_empty() {
            0
        } else {
            self.0.remove(0).min(max - 1)
        }
    }
}

#[derive(Default)]
struct Engine {
    rand: Seq,
    thumpers: i32,
    chars: HashMap<i32, Thumper>,
    sounds: Vec<WorldSoundRequest>,
    played: Vec<String>,
    items: Vec<String>,
    removed: Vec<(String, bool)>,
    retain_removed: bool,
    removed_owners: Vec<pz_world::IsoObjectRef>,
    movers: HashMap<i32, CollideMoverInfo>,
    hop: Vec<(i32, Vec<HoppableAction>)>,
    effects: Vec<(ObjectRef, NewRenderEffect)>,
    noises: Vec<(i32, (i32, i32, i32))>,
    cancelled: Vec<MoverId>,
    toggled: Vec<ObjectRef>,
    replace_toggled_square: bool,
    moving: Vec<(i32, i32, i32)>,
}

impl StatesEngine for Engine {
    fn rand(&mut self) -> &mut dyn GameRand {
        &mut self.rand
    }
    fn character(&self, id: i32) -> Option<Thumper> {
        self.chars.get(&id).copied()
    }
    fn surrounding_thumpers(&self, _chr: &Thumper) -> i32 {
        self.thumpers
    }
    fn add_world_sound(&mut self, s: WorldSoundRequest) {
        self.sounds.push(s);
    }
    fn play_sound(&mut self, _chr: i32, name: &str, _o: Option<ObjectRef>) {
        self.played.push(name.into());
    }
    fn add_world_item(
        &mut self,
        _sq: (i32, i32, i32),
        item: &str,
        _x: f32,
        _y: f32,
        _key: Option<i32>,
    ) {
        self.items.push(item.into());
    }
    fn object_removed(
        &mut self,
        _sq: (i32, i32, i32),
        obj: &pz_world::IsoObjectRef,
        destroyed: bool,
    ) {
        self.removed
            .push((obj.borrow().kind.class_name().into(), destroyed));
        if self.retain_removed {
            // The adapter must release every object guard before callbacks.
            obj.borrow_mut().key_id = 902;
            self.removed_owners.push(obj.clone());
        }
    }
    fn collide_mover(&self, id: i32) -> Option<CollideMoverInfo> {
        self.movers.get(&id).copied()
    }
    fn hoppable_actions(&mut self, chr: i32, actions: &[HoppableAction]) {
        self.hop.push((chr, actions.to_vec()));
    }
    fn has_render_effect(&self, obj: ObjectRef) -> bool {
        self.effects.iter().any(|(r, _)| *r == obj)
    }
    fn set_render_effect(&mut self, obj: ObjectRef, effect: NewRenderEffect) {
        self.effects.push((obj, effect));
    }
    fn do_tree_noises(&mut self, chr: i32, sq: (i32, i32, i32)) {
        self.noises.push((chr, sq));
    }
    fn path_cancel_request(&mut self, mover: MoverId) {
        self.cancelled.push(mover);
    }
    fn toggle_window(&mut self, cell: &mut IsoCell, window: ObjectRef, _chr: i32) {
        self.toggled.push(window);
        let sq = cell
            .get_grid_square_mut(window.x, window.y, window.z)
            .unwrap();
        let object = sq
            .borrow()
            .objects
            .iter()
            .find(|o| o.borrow().id == window.id)
            .map(pz_world::ObjectSlot::ref_handle);
        if let Some(object) = object {
            if let ObjectKind::Window(w) = &mut object.borrow_mut().kind {
                w.open = !w.open;
            };
        }
        if self.replace_toggled_square {
            let mut replacement = sq
                .try_borrow_mut()
                .expect("square guard survived into ToggleWindow")
                .try_archive_copy()
                .unwrap();
            for object in &replacement.objects {
                let mut object = object.borrow_mut();
                if object.id == window.id {
                    if let ObjectKind::Window(w) = &mut object.kind {
                        w.open = false;
                    }
                }
            }
            replacement.overlay_done = true;
            cell.chunks
                .get_mut(&(window.x.div_euclid(8), window.y.div_euclid(8)))
                .unwrap()
                .set_square(
                    window.x.rem_euclid(8),
                    window.y.rem_euclid(8),
                    window.z,
                    Some(replacement.into()),
                );
        }
    }
    fn has_moving_objects(&self, sq: (i32, i32, i32)) -> bool {
        self.moving.contains(&sq)
    }
    fn resolve_collision_radius(
        &mut self,
        world: WorldView<'_>,
        _chr: &dyn CharacterView,
        radius: f32,
    ) -> (bool, (f32, f32)) {
        (
            world.cell.get_grid_square(8, 8, 0).is_some(),
            (radius, radius),
        )
    }
}

/// Floor squares x 8..24, y 8..16 on level 0 (chunks (1, 1) and (2, 1)).
struct World {
    cell: IsoCell,
    m: IsoSpriteManager,
    meta: IsoMetaGrid,
    rt: StatesRuntime,
    engine: Engine,
    settings: ThumpSettings,
}

impl World {
    fn new() -> World {
        let mut w = World {
            cell: IsoCell::default(),
            m: manager(),
            meta: IsoMetaGrid::empty(),
            rt: StatesRuntime::default(),
            engine: Engine {
                thumpers: 1,
                ..Default::default()
            },
            settings: ThumpSettings::default(),
        };
        w.cell.chunks.insert((1, 1), IsoChunk::new(1, 1));
        w.cell.chunks.insert((2, 1), IsoChunk::new(2, 1));
        for x in 8..24 {
            for y in 8..16 {
                let mut sq = IsoGridSquare::new(x, y, 0);
                let id = sq.new_object_id();
                sq.objects.push(pz_world::ObjectSlot::new(IsoObject::new(
                    id,
                    ObjectKind::Object,
                    Some(w.m.named_map["floor"]),
                )));
                sq.recalc_properties(&w.m);
                let ch = w
                    .cell
                    .chunks
                    .get_mut(&(x.div_euclid(8), y.div_euclid(8)))
                    .unwrap();
                ch.set_square(x.rem_euclid(8), y.rem_euclid(8), 0, Some((sq).into()));
            }
        }
        w
    }

    fn add(&mut self, x: i32, y: i32, sprite: &str, kind: ObjectKind, special: bool) -> ObjectRef {
        let spr = self.m.named_map[sprite];
        let sq_owner = self.cell.get_grid_square_mut(x, y, 0).unwrap();
        let mut sq = sq_owner.borrow_mut();
        let id = sq.new_object_id();
        sq.objects.push(pz_world::ObjectSlot::new(IsoObject::new(
            id,
            kind,
            Some(spr),
        )));
        if special {
            sq.special_objects.push(id);
        }
        sq.recalc_properties(&self.m);
        ObjectRef { x, y, z: 0, id }
    }

    fn barricade(&mut self, x: i32, y: i32, dir: IsoDirections, planks: &[i32]) -> ObjectRef {
        let mut plank_health = [0; 4];
        plank_health[..planks.len()].copy_from_slice(planks);
        let b = IsoBarricade {
            dir: dir as i8,
            plank_health,
            metal_health: 0,
            metal_bar_health: 0,
        };
        self.add(x, y, "carpentry_01_9", ObjectKind::Barricade(b), true)
    }

    fn adapter(&mut self) -> StatesWorldAdapter<'_> {
        StatesWorldAdapter::new(
            &mut self.cell,
            &self.meta,
            &mut self.m,
            self.settings,
            &mut self.rt,
            &mut self.engine,
        )
    }

    fn object(&self, r: ObjectRef) -> Option<pz_world::IsoObjectRef> {
        let sq_owner = self.cell.get_grid_square(r.x, r.y, r.z)?;
        let sq = sq_owner.borrow();
        sq.objects
            .iter()
            .find(|o| o.borrow().id == r.id)
            .map(pz_world::ObjectSlot::ref_handle)
    }
}

fn door(north: bool, health: i32) -> ObjectKind {
    ObjectKind::Door(IsoDoor {
        north,
        health,
        max_health: 500,
        ..Default::default()
    })
}

fn window(w: &World, health: i32) -> ObjectKind {
    ObjectKind::Window(IsoWindow {
        north: false,
        health,
        max_health: 50,
        smashed_sprite: Some(w.m.named_map["window_w_smashed"]),
        ..Default::default()
    })
}

fn zombie(x: f32, y: f32, strength: i32) -> GameCharacter<()> {
    let sm = ScriptManager::new();
    let mut z = GameCharacter::<()>::new(&sm, CharacterKind::Zombie);
    z.id = 3;
    z.vars.x = x;
    z.vars.y = y;
    z.zombie.strength = strength;
    z.zombie.cognition = 3;
    z.path.current_square = Some((x as i32, y as i32, 0));
    z
}

fn special(w: &mut World, from: (i32, i32, i32), to: (i32, i32, i32)) -> Option<ObjectRef> {
    w.adapter().special_object_ref(from, to)
}

fn info(w: &mut World, r: ObjectRef, chr: i32) -> ThumpInfo {
    StateWorld::<()>::thump_info(&w.adapter(), ThumpTarget::Object(r), chr).unwrap()
}

fn thump(w: &mut World, r: ObjectRef, z: &mut GameCharacter<()>) {
    StateWorld::thump(&mut w.adapter(), ThumpTarget::Object(r), z);
}

#[test]
fn special_objects_between_neighbours() {
    let mut w = World::new();
    let d = w.add(10, 10, "door_n", door(true, 500), true);
    // Across the north edge, both ways, and diagonally through it.
    assert_eq!(special(&mut w, (10, 10, 0), (10, 9, 0)), Some(d));
    assert_eq!(special(&mut w, (10, 9, 0), (10, 10, 0)), Some(d));
    assert_eq!(special(&mut w, (10, 10, 0), (9, 9, 0)), Some(d));
    assert_eq!(special(&mut w, (10, 10, 0), (11, 9, 0)), Some(d));
    assert_eq!(special(&mut w, (10, 10, 0), (11, 10, 0)), None);
    // An open door is not in the way.
    if let ObjectKind::Door(dd) = &mut w
        .cell
        .get_grid_square_mut(10, 10, 0)
        .unwrap()
        .borrow()
        .objects[1]
        .borrow_mut()
        .kind
    {
        dd.open = true;
    }
    assert_eq!(special(&mut w, (10, 10, 0), (10, 9, 0)), None);
    // A thumpable blocking its square is the next square's solid.
    let th = IsoThumpable {
        block_all_the_square: true,
        is_thumpable: true,
        ..Default::default()
    };
    let t = w.add(14, 12, "crate", ObjectKind::Thumpable(Box::new(th)), true);
    assert_eq!(special(&mut w, (13, 12, 0), (14, 12, 0)), Some(t));
    assert_eq!(special(&mut w, (13, 11, 0), (14, 12, 0)), Some(t));
    // Not next to a window when it is solidtrans.
    let win = w.add(15, 12, "window_w", window(&w, 50), true);
    assert_eq!(special(&mut w, (13, 12, 0), (14, 12, 0)), None);
    // The window on the west edge of (15, 12), from the east.
    assert_eq!(special(&mut w, (15, 12, 0), (14, 12, 0)), Some(win));
    assert_eq!(special(&mut w, (14, 12, 0), (15, 12, 0)), Some(win));
    assert_eq!(special(&mut w, (16, 12, 0), (15, 12, 0)), None);
}

#[test]
fn barricade_is_what_the_zombie_thumps() {
    let mut w = World::new();
    let d = w.add(10, 10, "door_n", door(true, 500), true);
    let b = w.barricade(10, 9, IsoDirections::S, &[1000, 300]);
    w.engine.chars.insert(
        3,
        Thumper {
            id: 3,
            x: 10.5,
            y: 10.5,
            zombie: true,
            ..Default::default()
        },
    );
    w.engine.chars.insert(
        4,
        Thumper {
            id: 4,
            x: 10.5,
            y: 9.5,
            zombie: true,
            ..Default::default()
        },
    );
    // From the south the barricade on the far side is the opposite one.
    let inside = info(&mut w, d, 3);
    assert_eq!(inside.kind, ThumpKind::Door);
    assert_eq!(inside.thumpable_for, Some(ThumpTarget::Object(b)));
    assert!(!inside.barricade_for_character);
    let outside = info(&mut w, d, 4);
    assert!(outside.barricade_for_character);
    assert_eq!(outside.thumpable_for_kind, ThumpKind::Barricade);
    assert_eq!(outside.barricade_planks, 2);
    assert_eq!(outside.thump_condition, 0.3);
    assert_eq!(outside.facing_position, (10.5, 10.0));
    assert_eq!(outside.opposite_square, Some((10, 9, 0)));
}

#[test]
fn thumping_a_barricade_breaks_planks_then_removes_it() {
    let mut w = World::new();
    let d = w.add(10, 10, "door_n", door(true, 500), true);
    let b = w.barricade(10, 9, IsoDirections::S, &[1000, 300]);
    let mut z = zombie(10.5, 9.5, 200);
    z.zombie.thump_target = Some(ThumpTarget::Object(d));
    let planks = |w: &World| {
        let owner = w.object(b)?;
        let object = owner.borrow();
        match &object.kind {
            ObjectKind::Barricade(b) => Some(b.plank_health),
            _ => None,
        }
    };
    // The door hands over to the barricade; the top plank takes the damage.
    thump(&mut w, d, &mut z);
    assert_eq!(planks(&w), Some([1000, 100, 0, 0]));
    thump(&mut w, d, &mut z);
    assert_eq!(planks(&w), Some([1000, 0, 0, 0]));
    assert_eq!(w.engine.played, ["BreakBarricadePlank"]);
    for _ in 0..5 {
        thump(&mut w, b, &mut z);
    }
    assert_eq!(planks(&w), None);
    assert_eq!(w.engine.played.len(), 2);
    assert_eq!(w.engine.removed, [("IsoBarricade".to_string(), false)]);
    assert_eq!(w.engine.sounds.len(), 7);
    assert!(w
        .engine
        .sounds
        .iter()
        .all(|s| s.radius == 20 && s.source == Some(3)));
    // Gone: the door is thumped itself now.
    w.engine.chars.insert(3, Thumper::of(&z));
    assert_eq!(
        info(&mut w, d, 3).thumpable_for,
        Some(ThumpTarget::Object(d))
    );
}

#[test]
fn door_needs_two_thumpers_and_breaks_into_items() {
    let mut w = World::new();
    let d = w.add(12, 12, "door_w_mat", door(false, 30), true);
    let mut z = zombie(12.5, 12.5, 20);
    z.zombie.thump_target = Some(ThumpTarget::Object(d));
    let health = |w: &World| {
        let owner = w.object(d)?;
        let object = owner.borrow();
        match &object.kind {
            ObjectKind::Door(dd) => Some(dd.health),
            _ => None,
        }
    };
    thump(&mut w, d, &mut z);
    assert_eq!(health(&w), Some(30));
    w.engine.thumpers = 2;
    w.settings.lore_strength = 1;
    thump(&mut w, d, &mut z);
    // Strength, then 2 per thumper for superhuman zombies.
    assert_eq!(health(&w), Some(6));
    // NextBool(5) draws 0 (a second piece of wood), then 1 hinge.
    w.engine.rand = Seq(vec![0, 1]);
    thump(&mut w, d, &mut z);
    assert_eq!(health(&w), None);
    assert_eq!(z.zombie.thump_target, None);
    assert_eq!(w.engine.played, ["WoodDoorBreak"]);
    assert_eq!(
        w.engine.items,
        [
            "Base.UnusableWood",
            "Base.UnusableWood",
            "Base.Doorknob",
            "Base.Hinge"
        ]
    );
    assert_eq!(w.engine.removed, [("IsoDoor".to_string(), true)]);
    // The door's flags left the square.
    assert!(!w
        .cell
        .get_grid_square(12, 12, 0)
        .unwrap()
        .borrow()
        .has(w.m.tables.flag.doorW));
}

#[test]
fn smashing_a_window_leaves_glass_and_hands_over_to_the_barricade() {
    let mut w = World::new();
    let win = w.add(12, 12, "window_w", window(&w, 50), true);
    let b = w.barricade(11, 12, IsoDirections::E, &[500]);
    let mut z = zombie(12.5, 12.5, 30);
    z.zombie.thump_target = Some(ThumpTarget::Object(win));
    thump(&mut w, win, &mut z);
    assert!(
        matches!(&w.object(win).unwrap().borrow().kind, ObjectKind::Window(x) if x.health == 20 && !x.destroyed)
    );
    w.engine.rand = Seq(vec![2]);
    thump(&mut w, win, &mut z);
    let sq_owner = w.cell.get_grid_square(12, 12, 0).unwrap();
    let sq = sq_owner.borrow();
    let o = sq
        .objects
        .iter()
        .find(|o| o.borrow().id == win.id)
        .unwrap()
        .borrow();
    assert!(matches!(&o.kind, ObjectKind::Window(x) if x.destroyed && x.health == 0));
    assert_eq!(o.sprite, Some(w.m.named_map["window_w_smashed"]));
    drop(o);
    // `damage` smashed it and dropped glass on the far side (the zombie
    // is east of the west edge); the thump's own break block is skipped
    // once destroyed, so the target stays.
    assert!(!sq
        .objects
        .iter()
        .any(|o| matches!(o.borrow().kind, ObjectKind::BrokenGlass)));
    let far_owner = w.cell.get_grid_square(11, 12, 0).unwrap();
    let far = far_owner.borrow();
    let glass = far
        .objects
        .iter()
        .find(|o| matches!(o.borrow().kind, ObjectKind::BrokenGlass))
        .unwrap()
        .borrow();
    assert_eq!(glass.sprite, Some(w.m.named_map["brokenglass_1_2"]));
    assert!(far.special_objects.contains(&glass.id));
    drop(glass);
    drop(far);
    drop(sq);
    assert_eq!(z.zombie.thump_target, Some(ThumpTarget::Object(win)));
    assert_eq!(
        w.engine
            .sounds
            .iter()
            .filter(|s| s.source.is_none())
            .count(),
        1
    );
    // The smashed window hands over to the barricade behind it.
    w.engine.chars.insert(3, Thumper::of(&z));
    let i = info(&mut w, win, 3);
    assert_eq!(i.thumpable_for, Some(ThumpTarget::Object(b)));
    assert!(i.destroyed);
}

#[test]
fn thumpable_takes_partial_damage() {
    let mut w = World::new();
    let th = IsoThumpable {
        north: true,
        is_thumpable: true,
        health: 2,
        max_health: 4,
        thump_dmg: 8,
        thump_sound: "ZombieThumpWood".into(),
        ..Default::default()
    };
    let t = w.add(12, 12, "crate", ObjectKind::Thumpable(Box::new(th)), true);
    w.engine.thumpers = 2;
    let mut z = zombie(12.5, 12.5, 5);
    let hp = |w: &World| {
        let owner = w.object(t)?;
        let object = owner.borrow();
        match &object.kind {
            ObjectKind::Thumpable(x) => Some(x.health),
            _ => None,
        }
    };
    for _ in 0..3 {
        thump(&mut w, t, &mut z);
    }
    assert_eq!(hp(&w), Some(2));
    thump(&mut w, t, &mut z);
    assert_eq!(hp(&w), Some(1));
    let i = info(&mut w, t, 3);
    assert_eq!(
        (i.thump_sound.as_str(), i.thump_condition),
        ("ZombieThumpWood", 0.25)
    );
    for _ in 0..4 {
        thump(&mut w, t, &mut z);
    }
    assert_eq!(hp(&w), None);
    assert_eq!(w.engine.played, ["BreakFurniture"]);
    // Without construction thumping nothing happens.
    let th2 = IsoThumpable {
        is_thumpable: true,
        health: 1,
        thump_dmg: 1,
        ..Default::default()
    };
    let t2 = w.add(13, 12, "crate", ObjectKind::Thumpable(Box::new(th2)), true);
    w.settings.thump_on_construction = false;
    thump(&mut w, t2, &mut z);
    assert!(w.object(t2).is_some());
}

#[test]
fn square_queries() {
    let mut w = World::new();
    w.add(9, 9, "sneaky", ObjectKind::Object, false);
    w.add(10, 9, "crate", ObjectKind::Object, false);
    w.add(11, 9, "stairs_tn", ObjectKind::Object, false);
    w.add(11, 10, "stairs_mn", ObjectKind::Object, false);
    w.add(11, 11, "stairs_bn", ObjectKind::Object, false);
    w.add(12, 12, "door_n", door(true, 1), true);
    let a = w.adapter();
    let s = |x, y, solid| StateWorld::<()>::grid_sneak_modifier(&a, (x, y, 0), solid);
    assert_eq!(s(9, 9, false), 2.5);
    assert_eq!(s(12, 12, false), 8.0);
    assert_eq!(s(8, 8, false), 1.0);
    assert_eq!(s(10, 9, true), 4.0);
    assert!(!StateWorld::<()>::square_is_free(&a, (10, 9, 0)));
    assert!(StateWorld::<()>::square_is_free(&a, (8, 8, 0)));
    assert!(StateWorld::<()>::same_staircase(&a, (11, 9, 0), 11, 11, 0));
    assert!(!StateWorld::<()>::same_staircase(&a, (11, 9, 0), 11, 12, 0));
    assert!(StateWorld::<()>::is_something_to(
        &a,
        (12, 12, 0),
        (12, 11, 0)
    ));
    assert!(!StateWorld::<()>::is_something_to(
        &a,
        (12, 12, 0),
        (13, 12, 0)
    ));
    let i = StateWorld::<()>::square(&a, 11, 10, 0).unwrap();
    assert!(i.has_stairs && i.has_stairs_north && !i.has_stairs_west && i.solid_floor);
    assert_eq!(
        StateWorld::<()>::square_sprite_names(&a, (9, 9, 0)),
        ["floor", "sneaky"]
    );
    assert!(StateWorld::<()>::chunk_loaded(&a, 2, 1) && !StateWorld::<()>::chunk_loaded(&a, 0, 0));
    // The closed door blocks the collide line through its edge.
    assert!(StateWorld::<()>::los_line_clear_collide(
        &a,
        (12, 14, 0),
        (12, 10, 0),
        false
    ));
    assert!(!StateWorld::<()>::los_line_clear_collide(
        &a,
        (14, 14, 0),
        (14, 10, 0),
        false
    ));
    assert_ne!(
        StateWorld::<()>::los_line_clear(&a, (12, 14, 0), (12, 10, 0), false),
        TestResults::Clear
    );
    assert_eq!(
        StateWorld::<()>::los_line_clear(&a, (14, 14, 0), (14, 10, 0), false),
        TestResults::Clear
    );
}

#[test]
fn corpses_and_eating() {
    let mut w = World::new();
    assert_eq!(corpse_pos(corpse_id(-5, 70000, -3, 2)), (-5, 70000, -3, 2));
    let mut a = w.adapter();
    assert!(StateWorld::<()>::dead_body_at(&a, (9, 9, 0)).is_none());
    StateWorld::<()>::set_eating(&mut a, EatTarget::Body(7), 3, true);
    StateWorld::<()>::set_eating(&mut a, EatTarget::Body(7), 3, true);
    StateWorld::<()>::set_eating(&mut a, EatTarget::Body(7), 4, true);
    assert_eq!(StateWorld::<()>::eating_count(&a, EatTarget::Body(7)), 2);
    StateWorld::<()>::set_eating(&mut a, EatTarget::Body(7), 3, false);
    assert_eq!(StateWorld::<()>::eating_count(&a, EatTarget::Body(7)), 1);
}

#[test]
fn vehicle_box_and_polygon() {
    let script = VehicleBox {
        extents: [2.0, 1.0, 4.0],
        center_of_mass_offset: [0.0; 3],
    };
    // A quarter turn about the vertical axis: the length runs along x.
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let g = VehicleGeometry::from_parts((20.0, 20.0, 0.0), 0.0, [0.0, h, 0.0, h], script);
    assert!(g.intersects((17.5, 20.0, 0.1), (22.5, 20.0, 0.1)));
    assert!(g.intersects((21.8, 15.0, 0.1), (21.8, 25.0, 0.1)));
    assert!(!g.intersects((23.0, 15.0, 0.1), (23.0, 25.0, 0.1)));
    let d = g.closest_dist_sq(20.0, 25.0);
    assert!((d - 16.0).abs() < 1e-3, "{d}");
}

/// Recalculates every square's matrices (`RecalcAllWithNeighbours(true)`).
fn recalc(w: &mut World) {
    for x in 8..24 {
        for y in 8..16 {
            w.cell.recalc_all_with_neighbours(x, y, 0, &w.meta, &w.m);
        }
    }
}

fn mover_info(id: i32, kind: MoverKind) -> CollideMoverInfo {
    CollideMoverInfo {
        id,
        kind,
        ..Default::default()
    }
}

fn zombie_mover(id: i32) -> MoverId {
    MoverId((3 << 32) | id as u64)
}

#[test]
fn collide_adjacent_with_the_mover() {
    let mut w = World::new();
    // Squares are valid only inside the meta grid's cells.
    w.meta.max_x = 1;
    w.meta.max_y = 1;
    let d = w.add(12, 12, "door_n", door(true, 500), true);
    recalc(&mut w);
    w.engine.movers.insert(3, mover_info(3, MoverKind::Zombie));
    let mut noclip = mover_info(4, MoverKind::Player);
    noclip.no_clip = true;
    let a = w.adapter();
    // A zombie walking north into the closed door hits it.
    let z = mover_info(3, MoverKind::Zombie);
    let r = a
        .test_collide_adjacent_mover(&z, (12, 12, 0), (0, -1, 0))
        .unwrap();
    assert!(r.blocked);
    let sp = r.special.unwrap();
    assert!(sp.door);
    assert_eq!(
        sp.object,
        SquareObject {
            x: 12,
            y: 12,
            z: 0,
            id: d.id
        }
    );
    // From the north side it is the same door, through the collide matrix
    // path too.
    let r = a
        .test_collide_adjacent_mover(&z, (12, 11, 0), (0, 1, 0))
        .unwrap();
    assert_eq!(r.special.map(|s| s.object.id), Some(d.id));
    // No mover: only the matrix bit (the door blocks it).
    let env = a.collide_env();
    let sq_owner = a.cell.get_grid_square(12, 12, 0).unwrap();
    let r = env.test_collide_adjacent(&sq_owner, None, 0, -1, 0);
    assert!(r.blocked && r.special.is_none() && !r.recalc);
    // Open floor: nothing; a player gets no recalculation for a clear bit.
    let p = mover_info(4, MoverKind::Player);
    let r = a
        .test_collide_adjacent_mover(&p, (14, 12, 0), (1, 0, 0))
        .unwrap();
    assert_eq!(r, Default::default());
    // No clip walks through the door.
    let r = a
        .test_collide_adjacent_mover(&noclip, (12, 12, 0), (0, -1, 0))
        .unwrap();
    assert!(!r.blocked);
    // Out of range and other levels are blocked.
    assert!(
        a.test_collide_adjacent_mover(&z, (12, 12, 0), (2, 0, 0))
            .unwrap()
            .blocked
    );
    // Ignoring the target square (sitting on furniture there) lets it pass.
    let mut sit = z;
    sit.sit_on_square = Some((12, 11, 0));
    assert!(
        !a.test_collide_adjacent_mover(&sit, (12, 12, 0), (0, -1, 0))
            .unwrap()
            .blocked
    );
    // Through the StateWorld hook, by MoverId.
    assert!(StateWorld::<()>::test_collide_adjacent(
        &a,
        zombie_mover(3),
        (12, 12, 0),
        0,
        -1,
        0
    ));
    assert!(!StateWorld::<()>::test_collide_adjacent(
        &a,
        zombie_mover(3),
        (14, 12, 0),
        0,
        -1,
        0
    ));
    // Outside the meta grid every move is blocked.
    assert!(StateWorld::<()>::test_collide_adjacent(
        &a,
        zombie_mover(3),
        (15, 15, 0),
        1,
        1,
        0
    ));
}

#[test]
fn hoppable_edges() {
    let mut w = World::new();
    let f = w.add(10, 10, "fence_n", ObjectKind::Object, false);
    let t = w.add(14, 10, "tall_fence_w", ObjectKind::Object, false);
    let a = w.adapter();
    let env = a.collide_env();
    let sq = |x, y| a.cell.get_grid_square(x, y, 0);
    let (s10, s9, s11) = (sq(10, 10).unwrap(), sq(10, 9).unwrap(), sq(10, 11).unwrap());
    assert!(env.is_hoppable_to(&s10.borrow(), Some(&s9.borrow())));
    assert!(env.is_hoppable_to(&s9.borrow(), Some(&s10.borrow())));
    assert!(!env.is_hoppable_to(&s10.borrow(), Some(&s11.borrow())));
    // Diagonal is never hoppable, but getHoppableTo still finds the edge.
    assert!(!env.is_hoppable_to(&s11.borrow(), Some(&sq(9, 9).unwrap().borrow())));
    assert_eq!(
        env.get_hoppable_to(&s9, Some(&s10)).map(|(_, id)| id),
        Some(f.id)
    );
    assert_eq!(env.get_hoppable(&s10.borrow(), true), Some(f.id));
    assert_eq!(env.get_hoppable(&s10.borrow(), false), None);
    let s10_body = s10.borrow();
    let o = s10_body.objects[s10_body.object_index(f.id).unwrap()].borrow();
    assert!(o.is_hoppable(a.sprites) && !o.is_tall_hoppable(a.sprites));
    assert!(o.is_north_hoppable(a.sprites));
    let s14 = sq(14, 10).unwrap();
    let s14_body = s14.borrow();
    let o = s14_body.objects[s14_body.object_index(t.id).unwrap()].borrow();
    assert!(o.is_hoppable(a.sprites) && o.is_tall_hoppable(a.sprites));
    assert!(env.is_hoppable_to(&s14_body, Some(&sq(13, 10).unwrap().borrow())));
    assert!(!s10_body.have_door());
}

#[test]
fn doors_hop_only_when_closed_with_a_hoppable_closed_sprite() {
    let mut w = World::new();
    let fence = w.m.named_map["fence_n"];
    let mut k = door(true, 500);
    if let ObjectKind::Door(d) = &mut k {
        d.closed_sprite = Some(fence);
    }
    let d = w.add(12, 12, "door_n", k, true);
    let a = w.adapter();
    let sq_owner = a.cell.get_grid_square(12, 12, 0).unwrap();
    let sq = sq_owner.borrow();
    assert!(sq.have_door());
    let o = sq.objects[sq.object_index(d.id).unwrap()].borrow();
    assert!(o.is_hoppable(a.sprites));
    let mut open = o.clone();
    if let ObjectKind::Door(d) = &mut open.kind {
        d.open = true;
    }
    assert!(!open.is_hoppable(a.sprites));
}

#[test]
fn zombies_climb_or_break_fences_they_collide_with() {
    let mut w = World::new();
    w.add(10, 10, "fence_n", ObjectKind::Object, false);
    let ht = w.add(
        12,
        10,
        "fence_n",
        ObjectKind::Thumpable(Box::new(IsoThumpable {
            north: true,
            is_hoppable: true,
            ..Default::default()
        })),
        true,
    );
    let north = CollidedFlags {
        n: true,
        ..Default::default()
    };
    let mut z = mover_info(3, MoverKind::Zombie);
    z.collided = north;
    w.engine.movers.insert(3, z);
    let mut crawler = z;
    crawler.id = 5;
    crawler.crawling = true;
    w.engine.movers.insert(5, crawler);
    let mut a = w.adapter();
    // A standing zombie that hit the fence from the south climbs it.
    assert_eq!(
        a.hoppable_actions_for(&z, (10, 10, 0)),
        vec![HoppableAction::ClimbOverFence(IsoDirections::N)]
    );
    // Not while attacking, nor going east (no fence there).
    let mut busy = z;
    busy.states.attack = true;
    assert!(a.hoppable_actions_for(&busy, (10, 10, 0)).is_empty());
    let mut east = z;
    east.collided = CollidedFlags {
        e: true,
        ..Default::default()
    };
    assert!(a.hoppable_actions_for(&east, (10, 10, 0)).is_empty());
    // Two edges at once: no single direction, nothing.
    let mut corner = z;
    corner.collided.w = true;
    assert!(a.hoppable_actions_for(&corner, (10, 10, 0)).is_empty());
    // A crawler thumps a hoppable thumpable that is not itself thumpable.
    assert_eq!(
        a.hoppable_actions_for(&crawler, (12, 10, 0)),
        vec![HoppableAction::ThumpTarget(SquareObject {
            x: 12,
            y: 10,
            z: 0,
            id: ht.id
        })]
    );
    // Through the StateWorld hook the engine gets the actions.
    StateWorld::<()>::collided_with_hoppables(&mut a, 3, Some((10, 10, 0)));
    StateWorld::<()>::collided_with_hoppables(&mut a, 9, Some((10, 10, 0)));
    assert_eq!(
        w.engine.hop,
        vec![(3, vec![HoppableAction::ClimbOverFence(IsoDirections::N)])]
    );
}

#[test]
fn animals_climb_fences_they_can() {
    let mut w = World::new();
    w.add(10, 11, "fence_n", ObjectKind::Object, false);
    let a = w.adapter();
    let mut an = mover_info(7, MoverKind::Animal);
    an.collided = CollidedFlags {
        s: true,
        ..Default::default()
    };
    assert!(a.hoppable_actions_for(&an, (10, 10, 0)).is_empty());
    an.can_climb_fences = true;
    assert_eq!(
        a.hoppable_actions_for(&an, (10, 10, 0)),
        vec![HoppableAction::ClimbOverFence(IsoDirections::S)]
    );
    an.states.animal_climb_over_fence = true;
    assert!(a.hoppable_actions_for(&an, (10, 10, 0)).is_empty());
}

/// A context whose `Rand.Next(float, float)` returns `min + 1` and counts.
#[derive(Default)]
struct DrawCtx(std::cell::Cell<i32>);

impl pz_characters::character::SimContext for DrawCtx {
    fn rand_next_float(&self, min: f32, _max: f32) -> f32 {
        self.0.set(self.0.get() + 1);
        min + 1.0
    }
}

#[test]
fn vegetation_rustles_past_the_first_object() {
    let mut w = World::new();
    let tree = w.add(
        10,
        10,
        "tree_a",
        ObjectKind::Tree(pz_world::objects::IsoTree {
            size: 5,
            log_yield: 1,
            damage: 0,
        }),
        false,
    );
    let bush = w.add(10, 10, "shrub", ObjectKind::Object, false);
    // A bush as the first object makes noise but rustles nothing.
    {
        let spr = w.m.named_map["shrub"];
        let sq_owner = w.cell.get_grid_square_mut(11, 10, 0).unwrap();
        let mut sq = sq_owner.borrow_mut();
        sq.objects[0].borrow_mut().sprite = Some(spr);
        sq.recalc_properties(&w.m);
    }
    let ctx = DrawCtx::default();
    let mut a = w.adapter();
    StateWorld::<()>::rustle_vegetation(&mut a, 3, (10, 10, 0), &ctx);
    // An effect already running draws nothing.
    StateWorld::<()>::rustle_vegetation(&mut a, 3, (10, 10, 0), &ctx);
    StateWorld::<()>::rustle_vegetation(&mut a, 3, (11, 10, 0), &ctx);
    StateWorld::<()>::rustle_vegetation(&mut a, 3, (12, 12, 0), &ctx);
    assert_eq!(ctx.0.get(), 2);
    assert_eq!(
        w.engine.effects,
        vec![
            (
                tree,
                NewRenderEffect {
                    kind: RenderEffectType::VegetationRustle,
                    total_max_time: 46.0,
                    is_tree: true,
                    is_big: true,
                }
            ),
            (
                bush,
                NewRenderEffect {
                    kind: RenderEffectType::VegetationRustle,
                    total_max_time: 46.0,
                    is_tree: false,
                    is_big: false,
                }
            ),
        ]
    );
    assert_eq!(
        w.engine.noises,
        vec![(3, (10, 10, 0)), (3, (10, 10, 0)), (3, (11, 10, 0))]
    );
    // A server has no render effects or tree noises.
    w.settings.server = true;
    let mut a = w.adapter();
    StateWorld::<()>::rustle_vegetation(&mut a, 3, (10, 10, 0), &ctx);
    assert_eq!(ctx.0.get(), 2);
    assert_eq!(w.engine.noises.len(), 3);
}

#[test]
fn path_host_hooks_go_to_the_engine() {
    let mut w = World::new();
    let mut z = zombie(10.5, 10.5, 2);
    let mut a = w.adapter();
    StateWorld::<()>::path_cancel_request(&mut a, &mut z);
    let r = StateWorld::<()>::resolve_collision_radius(&mut a, &z, 0.3);
    assert_eq!(r, (true, (0.3, 0.3)));
    // The default keeps the position.
    assert_eq!(
        StateWorld::<()>::resolve_collision(&mut a, &z, 1.0, 2.0),
        (1.0, 2.0)
    );
    assert_eq!(w.engine.cancelled, vec![z.mover_id()]);
    // Without an engine fire simulation, character fire does nothing.
}

#[test]
fn loaded_windows_share_their_closed_sprite_properties() {
    let mut w = World::new();
    let (closed, smashed) = (w.m.named_map["window_w"], w.m.named_map["window_w_smashed"]);
    let win = IsoWindow {
        north: false,
        closed_sprite: Some(closed),
        smashed_sprite: Some(smashed),
        ..Default::default()
    };
    let r = w.add(12, 12, "window_w_smashed", ObjectKind::Window(win), true);
    let collide_w = w.m.tables.flag.collideW;
    assert!(!w.m.sprites[smashed].properties.has(collide_w));
    pz_world::save::chunk_file::apply_loaded_window_sprites(&w.cell.chunks[&(1, 1)], &mut w.m);
    assert!(w.m.sprites[smashed].properties.has(collide_w));
    assert!(w.m.sprites[closed].properties.has(w.m.tables.flag.cutW));
    let sq_owner = w.cell.get_grid_square(12, 12, 0).unwrap();
    let sq = sq_owner.borrow();
    assert_eq!(sq.object_index(r.id), Some(1));
}

fn climber(x: f32, y: f32) -> Thumper {
    Thumper {
        id: 7,
        x,
        y,
        current_square: Some((x as i32, y as i32, 0)),
        ..Default::default()
    }
}

#[test]
fn window_info_and_perma_lock() {
    let mut w = World::new();
    let win = w.add(12, 12, "window_w", window(&w, 50), true);
    let i = w.adapter().window_info_of(win).unwrap();
    assert_eq!(i.square, (12, 12, 0));
    assert!(!i.north && i.in_world && !i.open && !i.destroyed && !i.perma_locked && !i.invincible);
    assert_eq!(i.opposite(), (11, 12, 0));
    w.adapter().set_window_perma_locked_at(win, true);
    assert!(w.adapter().window_info_of(win).unwrap().perma_locked);
    // Not a window.
    let d = w.add(10, 10, "door_n", door(true, 500), true);
    assert_eq!(w.adapter().window_info_of(d), None);
}

#[test]
fn sheet_rope_squares_and_chunk_levels() {
    let mut w = World::new();
    w.add(12, 12, "sheet_w", ObjectKind::Object, false);
    w.add(13, 12, "sheet_top_w", ObjectKind::Object, false);
    let a = w.adapter();
    let r = a.sheet_rope_square_at((12, 12, 0)).unwrap();
    assert!(r.sheet_rope && r.w && !r.top && !r.n && !r.s && !r.e);
    assert!(r.can_climb_here && r.solid_floor && !r.solidtrans);
    let t = a.sheet_rope_square_at((13, 12, 0)).unwrap();
    assert!(t.sheet_rope && t.top && t.w);
    let none = a.sheet_rope_square_at((14, 12, 0)).unwrap();
    assert!(!none.sheet_rope && !none.w);
    assert_eq!(a.sheet_rope_square_at((40, 40, 0)), None);
    let ch = w.cell.chunks.get_mut(&(1, 1)).unwrap();
    ch.min_level = -1;
    ch.max_level = 3;
    let a = w.adapter();
    assert_eq!(a.chunk_levels_at((12, 12, 0)), (-1, 3));
    assert_eq!(a.chunk_levels_at((17, 12, 0)), (0, 0));
    assert_eq!(a.chunk_levels_at((100, 100, 0)), (0, 0));
}

#[test]
fn sheet_rope_exits() {
    let mut w = World::new();
    let t = climber(12.5, 12.5);
    // Nothing on the edge.
    assert_eq!(
        w.adapter()
            .sheet_rope_exit_for(&t, (12, 12, 0), IsoDirections::W),
        SheetRopeExit::Undefined
    );
    // Off the map beyond.
    assert_eq!(
        w.adapter()
            .sheet_rope_exit_for(&t, (8, 12, 0), IsoDirections::W),
        SheetRopeExit::Blocked
    );
    // A shut window is opened (ToggleWindow) and climbed through.
    let win = w.add(12, 12, "window_w", window(&w, 50), true);
    assert_eq!(
        w.adapter()
            .sheet_rope_exit_for(&t, (12, 12, 0), IsoDirections::W),
        SheetRopeExit::OpenWindow(win)
    );
    assert_eq!(w.engine.toggled, [win]);
    // Already open: no second toggle.
    assert_eq!(
        w.adapter()
            .sheet_rope_exit_for(&t, (12, 12, 0), IsoDirections::W),
        SheetRopeExit::OpenWindow(win)
    );
    assert_eq!(w.engine.toggled.len(), 1);
    // A fence on the north edge.
    w.add(14, 12, "fence_n", ObjectKind::Object, false);
    let t2 = climber(14.5, 12.5);
    assert_eq!(
        w.adapter()
            .sheet_rope_exit_for(&t2, (14, 12, 0), IsoDirections::N),
        SheetRopeExit::Fence
    );
    // A solid square beyond it blocks the climb.
    w.add(14, 11, "solid_block", ObjectKind::Object, false);
    assert_eq!(
        w.adapter()
            .sheet_rope_exit_for(&t2, (14, 12, 0), IsoDirections::N),
        SheetRopeExit::Blocked
    );
}

#[test]
fn sheet_rope_toggle_uses_original_window_when_callback_replaces_its_square() {
    let mut w = World::new();
    let win = w.add(12, 12, "window_w", window(&w, 50), true);
    let original = w.object(win).unwrap();
    let square = w.cell.get_grid_square(12, 12, 0).unwrap();
    w.engine.replace_toggled_square = true;
    assert_eq!(
        w.adapter()
            .sheet_rope_exit_for(&climber(12.5, 12.5), (12, 12, 0), IsoDirections::W),
        SheetRopeExit::OpenWindow(win)
    );
    assert_eq!(w.engine.toggled, [win]);
    let replacement_square = w.cell.get_grid_square(12, 12, 0).unwrap();
    let replacement = w.object(win).unwrap();
    assert!(!std::rc::Rc::ptr_eq(&square, &replacement_square));
    assert!(!std::rc::Rc::ptr_eq(&original, &replacement));
    assert!(replacement_square.borrow().overlay_done);
    assert!(matches!(&original.borrow().kind, ObjectKind::Window(w) if w.open));
    assert!(matches!(&replacement.borrow().kind, ObjectKind::Window(w) if !w.open));
}

#[test]
fn climbing_over_walls() {
    let mut w = World::new();
    w.add(12, 12, "wall_n_metal", ObjectKind::Object, false);
    w.add(14, 12, "wall_w", ObjectKind::Object, false);
    let a = w.adapter();
    // North over the metal wall on (12, 12), from inside the square.
    assert!(a.can_climb_over_wall_for(&climber(12.5, 12.5), IsoDirections::N));
    // No wall on that edge.
    assert!(!a.can_climb_over_wall_for(&climber(12.5, 12.5), IsoDirections::W));
    assert!(!a.can_climb_over_wall_for(&climber(10.5, 12.5), IsoDirections::N));
    // South onto (12, 12) from (12, 11): the wall is on the square beyond.
    assert!(a.can_climb_over_wall_for(&climber(12.5, 11.5), IsoDirections::S));
    // East over the west wall of (14, 12).
    assert!(a.can_climb_over_wall_for(&climber(13.5, 12.5), IsoDirections::E));
    assert_eq!(
        a.climb_wall_fence_type_at((12, 12, 0), (12, 11), IsoDirections::N),
        Some(1)
    );
    assert_eq!(
        a.climb_wall_fence_type_at((12, 11, 0), (12, 12), IsoDirections::S),
        Some(1)
    );
    assert_eq!(
        a.climb_wall_fence_type_at((13, 12, 0), (14, 12), IsoDirections::E),
        Some(0)
    );
    assert_eq!(
        a.climb_wall_fence_type_at((12, 12, 0), (11, 12), IsoDirections::W),
        None
    );
    // A hoppable fence is not a wall.
    w.add(16, 12, "fence_n", ObjectKind::Object, false);
    let a = w.adapter();
    assert!(!a.can_climb_over_wall_for(&climber(16.5, 12.5), IsoDirections::N));
    assert_eq!(
        a.climb_wall_fence_type_at((16, 12, 0), (16, 11), IsoDirections::N),
        None
    );
}

#[test]
fn sitting_checks_on_the_adapter() {
    use pz_world_popman::SittingChecks;
    let mut w = World::new();
    w.add(12, 12, "door_n", door(true, 500), true);
    w.engine.moving.push((9, 9, 0));
    let a = w.adapter();
    let line =
        |fx, fy, tx, ty| SittingChecks::line_clear_collide(&a, fx, fy, tx, ty, 0, false, true);
    assert!(SittingChecks::has_moving_objects(&a, 9, 9, 0));
    assert!(!SittingChecks::has_moving_objects(&a, 9, 10, 0));
    // Open floor; then through the closed door's north edge.
    assert!(!line(14.5, 13.5, 14.5, 10.5));
    assert!(line(12.5, 13.5, 12.5, 10.5));
}

#[test]
fn population_host_reads_the_adapter_real_zombies_hold() {
    use pz_characters::character::zombie::population::RealZombies;
    use pz_characters::character::DefaultSimContext;
    use pz_characters::survivor::SurvivorDescRef;
    use pz_popman::manager::PopulationHost;

    let mut w = World::new();
    w.add(10, 9, "crate", ObjectKind::Object, false);
    w.engine.moving.push((9, 9, 0));
    let sm = ScriptManager::new();
    let chars = std::cell::RefCell::new(pz_characters::Characters::default());
    let ctx = DefaultSimContext;
    let mut zombies = vec![zombie(11.5, 9.5, 1)];
    let mut next_id = 10;
    let mut survivor = || -> pz_scripts::ScriptResult<SurvivorDescRef<()>> {
        unreachable!("no zombie is created")
    };
    let water = |_: (i32, i32, i32)| false;
    let mut adapter = w.adapter();
    let mut ops = RealZombies {
        sm: &sm,
        chars: &chars,
        ctx: &ctx,
        world: &mut adapter,
        zombies: &mut zombies,
        next_id: &mut next_id,
        create_survivor: &mut survivor,
        water: &water,
        key_spawn_chance_d100: 0.0,
        building_keys: Vec::new(),
        removed: Vec::new(),
    };
    let mut rand = Seq(vec![4]);
    let mut host = population_host(&mut rand, &mut ops);
    assert!(host.square(8, 8, 0).unwrap().solid_floor);
    assert!(host.square(30, 8, 0).is_none());
    let free = |h: &pz_world_popman::WorldPopulationHost<'_, _>, x, y| {
        let v = h.view();
        h.is_free(&v.cell.get_grid_square(x, y, 0).unwrap().borrow())
    };
    assert!(free(&host, 8, 8));
    // The crate; a character the engine knows; a real zombie of the list.
    assert!(!free(&host, 10, 9));
    assert!(!free(&host, 9, 9));
    assert!(!free(&host, 11, 9));
    assert_eq!(host.rand().next(9), 4);
    host.set_position(3, 12.5, 12.5);
    assert_eq!((zombies[0].vars.x, zombies[0].vars.y), (12.5, 12.5));
}

#[test]
fn the_floor_sprite_name_is_the_solid_floor_object() {
    let mut w = World::new();
    let a = w.adapter();
    assert_eq!(
        a.floor_sprite_name_at((10, 10, 0)).as_deref(),
        Some("floor")
    );
    assert_eq!(a.floor_sprite_name_at((40, 40, 0)), None);
}

#[test]
fn removal_callback_can_mutate_and_retain_the_actual_whole_object() {
    let mut w = World::new();
    let r = w.add(10, 10, "crate", ObjectKind::Object, false);
    let owner = {
        let sq_owner = w.cell.get_grid_square(10, 10, 0).unwrap();
        let sq = sq_owner.borrow();
        sq.objects[sq.object_index(r.id).unwrap()].ref_handle()
    };
    owner.borrow_mut().tint_r = 0.375;
    w.engine.retain_removed = true;
    let removed = w.adapter().remove_object(r, true).unwrap();
    assert!(removed.ptr_eq_ref(&owner));
    assert!(std::rc::Rc::ptr_eq(&w.engine.removed_owners[0], &owner));
    assert_eq!(owner.borrow().key_id, 902, "callback mutation hit a copy");
    assert_eq!(owner.borrow().tint_r, 0.375);
    assert!(w
        .cell
        .get_grid_square(10, 10, 0)
        .unwrap()
        .borrow()
        .object_index(r.id)
        .is_none());
    drop(removed);
    assert_eq!(w.engine.removed_owners[0].borrow().tint_r, 0.375);
}
