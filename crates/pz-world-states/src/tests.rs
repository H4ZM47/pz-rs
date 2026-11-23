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
    ("door_n_open", "normal", &[]),
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
    ("protected_window", "normal", &[("WindowLocked", "true")]),
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
    ("door_n_forcelocked", "doorN", &[("doorN", ""), ("collideN", ""), ("forceLocked", "")]),
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
        "forceLocked",
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
    disable_door_effects: bool,
    noises: Vec<(i32, (i32, i32, i32))>,
    cancelled: Vec<MoverId>,
    toggled: Vec<ObjectRef>,
    closing_windows: Vec<ClosingWindow>,
    replace_toggled_square: bool,
    moving: Vec<(i32, i32, i32)>,
}

impl StatesEngine for Engine {
    fn closing_windows(&self) -> &[ClosingWindow] { &self.closing_windows }
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
    fn door_sprite_effects_enabled(&self) -> bool { !self.disable_door_effects }
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
    fn toggle_window(&mut self, cell: &mut IsoCell, _meta: &IsoMetaGrid, _sprites: &IsoSpriteManager, window: ObjectRef, _chr: &Thumper) -> pz_inventory::floor_drop::HostResult<()> {
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
        Ok(())
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
fn surviving_barricade_hit_reuses_its_phase_and_expires() {
    let mut world = World::new();
    let win = world.add(12, 12, "window_w", window(&world, 50), true);
    let board = world.barricade(12, 12, IsoDirections::W, &[1000]);
    let actor = climber(12.5, 12.5);
    let hit = |w: &mut World| w.adapter().window_weapon_hit_by(win, &actor, true);
    world.engine.disable_door_effects = true;
    hit(&mut world);
    assert!(world.rt.visible_door_hits(&world.cell).unwrap().is_empty());
    world.engine.disable_door_effects = false;
    hit(&mut world);
    for _ in 0..2 { world.rt.update_door_hits(1.0, &mut world.engine.rand); }
    let visible = world.rt.visible_door_hits(&world.cell).unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].0, board);
    assert!(visible[0].1[0][0] < 0.0 && visible[0].1[0][1] < 0.0);
    hit(&mut world);
    assert_eq!(visible, world.rt.visible_door_hits(&world.cell).unwrap());
    for _ in 0..30 { world.rt.update_door_hits(1.0, &mut world.engine.rand); }
    assert!(world.rt.visible_door_hits(&world.cell).unwrap().is_empty());

    hit(&mut world);
    let square = world.cell.get_grid_square(board.x, board.y, board.z).unwrap();
    let old = world.object(board).unwrap();
    let replacement = IsoObject::new(board.id, ObjectKind::Object, None);
    let idx = square.borrow().object_index(board.id).unwrap();
    square.borrow_mut().objects[idx] = pz_world::ObjectSlot::new(replacement);
    assert!(world.rt.visible_door_hits(&world.cell).unwrap().is_empty());
    drop(old);
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
        sq.objects[0].borrow_mut().sprite = Some(spr).into();
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
fn first_window_closer_uses_near_then_far_moving_object_order() {
    let mut world = World::new();
    let window = world.add(12, 12, "window_w", window(&world, 0), true);
    let near = world.cell.get_grid_square(12, 12, 0).unwrap();
    let far = world.cell.get_grid_square(11, 12, 0).unwrap();
    near.borrow_mut().moving_objects.extend([99, 2, 1]);
    far.borrow_mut().moving_objects.push(3);
    for (actor, success) in [(1, true), (2, false), (3, true)] {
        world.engine.closing_windows.push(ClosingWindow { actor, target: window, success });
    }
    let can_climb = |world: &mut World| {
        let adapter = world.adapter();
        let (sq, obj) = adapter.find(window).unwrap();
        let result = adapter.portal_can_climb_through(&sq.borrow(), &obj.borrow(), None);
        result
    };
    assert!(can_climb(&mut world));
    near.borrow_mut().moving_objects.swap(1, 2);
    assert!(!can_climb(&mut world));
    near.borrow_mut().moving_objects.clear();
    assert!(!can_climb(&mut world));
    far.borrow_mut().moving_objects.clear();
    assert!(can_climb(&mut world));
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
fn debug_unlock_preserves_protected_or_missing_sprites_only_while_opening() {
    let mut world = World::new();
    let win = world.add(12, 12, "protected_window", window(&world, 50), true);
    let object = world.object(win).unwrap();
    for missing in [false, true] {
        {
            let mut body = object.borrow_mut();
            if missing { body.sprite = pz_world::objects::ObjectSprite::None; }
            let ObjectKind::Window(pane) = &mut body.kind else { unreachable!() };
            pane.locked = true;
            pane.perma_locked = true;
        }
        let adapter = world.adapter();
        adapter.debug_unlock_window_at(win, true).unwrap();
        let info = adapter.window_info_of(win).unwrap();
        assert!(info.locked && info.perma_locked);
        adapter.debug_unlock_window_at(win, false).unwrap();
        let info = adapter.window_info_of(win).unwrap();
        assert!(!info.locked && !info.perma_locked);
    }
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

#[test]
fn actual_sprite_query_values_are_owned_and_host_faults_are_not_absence() {
    use pz_entity::runtime_load::RuntimeHostError;
    use pz_world::sprite::runtime::{create_runtime_sprite, ObjectSprite};
    use std::{cell::RefCell, rc::Rc};

    let w = World::new();
    let arena_before = w.m.sprites.len();
    let native_manager = Rc::new(RefCell::new(manager()));
    let mut retired = pz_entity::RetiredComponents::new();
    let sprite = create_runtime_sprite(&native_manager, &w.m.tables, &mut retired).unwrap();
    let mut object = IsoObject::new(99, ObjectKind::Object, None);
    object.sprite = ObjectSprite::Native(sprite.clone());
    {
        let mut body = sprite.borrow_mut();
        body.data.name = Some("actual_before".into());
        body.data.properties.set_raw(&w.m.alias, "Material", "Wood");
        body.data.properties.set_flag(w.m.tables.flag.collideN);
    }
    let env = Env::new(&w.cell, &w.m, &w.meta);
    let name = env.try_sprite_name(&object).unwrap().unwrap();
    let material = env.try_obj_prop(&object, "Material").unwrap().unwrap();
    let properties = env.try_props(&object).unwrap().unwrap();
    {
        let mut body = sprite.borrow_mut();
        body.data.name = Some("actual_after".into());
        body.data.properties.unset(&w.m.alias, "Material");
        body.data.properties.unset_flag(w.m.tables.flag.collideN);
    }
    assert_eq!(name, "actual_before");
    assert_eq!(material, "Wood");
    assert!(properties.has(w.m.tables.flag.collideN));
    assert_eq!(env.sprite_name(&object).as_deref(), Some("actual_after"));
    assert_eq!(env.obj_prop(&object, "Material"), None);
    assert!(!env.obj_has_flag(&object, "collideN"));
    assert_eq!(w.m.sprites.len(), arena_before);

    let held = sprite.borrow_mut();
    assert!(matches!(
        env.try_sprite_name(&object),
        Err(RuntimeHostError::Borrowed(_))
    ));
    assert!(matches!(
        env.try_props(&object),
        Err(RuntimeHostError::Borrowed(_))
    ));
    assert!(matches!(
        env.try_metadata(&object),
        Err(RuntimeHostError::Borrowed(_))
    ));
    let absent = IsoObject::new(100, ObjectKind::Object, None);
    assert_eq!(env.try_sprite_name(&absent).unwrap(), None);
    drop(held);
    object.sprite = ObjectSprite::ArchiveUnmanaged(-1);
    assert!(matches!(
        env.try_sprite_name(&object),
        Err(RuntimeHostError::Unsupported { .. })
    ));
    let detached = { sprite.borrow_mut().take_slots_for_gc() };
    drop(detached);
    drop(retired);
}

#[test]
fn state_fence_floor_and_bush_reads_follow_actual_sprite_mutation_and_retire_replacements() {
    use pz_world::sprite::runtime::{create_runtime_sprite, ObjectSprite};
    use std::{cell::RefCell, rc::Rc};

    let mut w = World::new();
    let r = w.add(10, 10, "crate", ObjectKind::Object, false);
    let native_manager = Rc::new(RefCell::new(manager()));
    let mut retired = pz_entity::RetiredComponents::new();
    let arena_before = w.m.sprites.len();
    let sprite = create_runtime_sprite(&native_manager, &w.m.tables, &mut retired).unwrap();
    let (floor, object) = {
        let sq = w.cell.get_grid_square(10, 10, 0).unwrap();
        let sq = sq.borrow();
        (sq.objects[0].ref_handle(), sq.objects[1].ref_handle())
    };
    floor.borrow_mut().sprite = ObjectSprite::Native(sprite.clone());
    object.borrow_mut().sprite = ObjectSprite::Native(sprite.clone());
    {
        let mut body = sprite.borrow_mut();
        body.data.name = Some("actual_floor".into());
        body.data.tileset_name = Some("f_bushes_1".into());
        body.data.tile_type = w.m.tables.ty.wall;
        body.data.cut_n = true;
        body.data.properties.set_flag(w.m.tables.flag.solidfloor);
    }
    {
        let adapter = w.adapter();
        assert_eq!(adapter.rustle_targets((10, 10, 0)), vec![(r, false, false)]);
        assert_eq!(
            adapter.floor_sprite_name_at((10, 10, 0)).as_deref(),
            Some("actual_floor")
        );
        let sq = adapter.cell.get_grid_square(10, 10, 0).unwrap();
        assert!(adapter.has_non_hoppable_wall(&sq.borrow(), true));
    }
    {
        let mut body = sprite.borrow_mut();
        body.data.name = Some("actual_changed".into());
        body.data.tileset_name = None;
        body.data.cut_n = false;
        body.data.properties.unset_flag(w.m.tables.flag.solidfloor);
    }
    {
        let adapter = w.adapter();
        assert!(adapter.rustle_targets((10, 10, 0)).is_empty());
        assert_eq!(adapter.floor_sprite_name_at((10, 10, 0)), None);
        let sq = adapter.cell.get_grid_square(10, 10, 0).unwrap();
        assert!(!adapter.has_non_hoppable_wall(&sq.borrow(), true));
    }
    assert!(Rc::ptr_eq(
        object.borrow().sprite.native_slot().unwrap(),
        &sprite
    ));
    assert_eq!(w.m.sprites.len(), arena_before);
    let pins_before = Rc::strong_count(&sprite);
    let tile = w.m.named_map["crate"];
    w.adapter().replace_object_sprite_tile(&object, Some(tile));
    assert_eq!(object.borrow().sprite.tile_projection(), Some(tile));
    assert!(Rc::ptr_eq(
        floor.borrow().sprite.native_slot().unwrap(),
        &sprite
    ));
    assert_eq!(
        Rc::strong_count(&sprite),
        pins_before,
        "old actual slot must be queued"
    );
    let retired_world = w.cell.take_retired_owners();
    assert_eq!(Rc::strong_count(&sprite), pins_before);
    drop(retired_world);
    assert_eq!(Rc::strong_count(&sprite), pins_before - 1);
    let detached = { sprite.borrow_mut().take_slots_for_gc() };
    drop(detached);
    drop(retired);
}

#[test]
fn sprite_replacement_observes_full_native_class_after_object_guards() {
    use pz_world::object_slot::{install_object_observer, ObjectObserver};
    use std::cell::RefCell;
    thread_local! {
        static CLASSES: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    }
    fn observed(object: &pz_world::IsoObjectRef, native_class: &'static str) {
        let body = object
            .try_borrow_mut()
            .expect("object guard survived observation");
        assert_eq!(native_class, body.kind.native_class_name());
        CLASSES.with(|classes| classes.borrow_mut().push(native_class));
    }
    struct Restore(Option<ObjectObserver>);
    impl Drop for Restore {
        fn drop(&mut self) {
            install_object_observer(self.0);
        }
    }

    let mut w = World::new();
    let plain = w.add(10, 10, "crate", ObjectKind::Object, false);
    let win = w.add(11, 10, "window_w", window(&w, 50), true);
    let owner = w.object(plain).unwrap();
    CLASSES.with(|classes| classes.borrow_mut().clear());
    let _restore = Restore(install_object_observer(Some(observed)));
    let tile = w.m.named_map["floor"];
    w.adapter().replace_object_sprite_tile(&owner, Some(tile));
    w.adapter().smash_window(win, false);
    CLASSES.with(|classes| {
        assert_eq!(
            *classes.borrow(),
            ["zombie.iso.IsoObject", "zombie.iso.objects.IsoWindow"]
        );
    });
}

// ---- the interact key (`IsoPlayer.doContext`) ----

mod context {
    use pz_characters::character::player::context::{ContextQuery, ContextSpecial};
    use pz_characters::character::player::ContextualActionKind as Kind;

    use super::*;

    /// A player standing in the middle of `(x, y, 0)`, tapping the key.
    fn query(x: i32, y: i32) -> ContextQuery {
        ContextQuery {
            chr: 1,
            square: (x, y, 0),
            x: x as f32 + 0.5,
            y: y as f32 + 0.5,
            shift: false,
            ticks_since_pressed_movement: 0.0,
            grappling: false,
            time_pressed_context: 0.0,
            ignore_auto_vault: false,
        }
    }

    /// A door with a closed and an open sprite on the north edge of
    /// `(x, y)`.
    fn north_door(w: &mut World, x: i32, y: i32, open: bool) -> ObjectRef {
        let mut k = door(true, 500);
        if let ObjectKind::Door(d) = &mut k {
            d.open = open;
            d.closed_sprite = Some(w.m.named_map["door_n"]);
            d.open_sprite = Some(w.m.named_map["door_n_open"]);
        }
        w.add(x, y, if open { "door_n_open" } else { "door_n" }, k, true)
    }

    /// A player at `square` holding the keys with the ids `keys` toggles
    /// the door.
    fn toggle_at(
        w: &mut World,
        d: ObjectRef,
        square: (i32, i32, i32),
        keys: &[i32],
    ) -> pz_inventory::floor_drop::HostResult<()> {
        let have_key = |id: i32| keys.contains(&id);
        let actor = pz_characters::character::player::context::DoorActor { square, have_key: &have_key };
        w.adapter().toggle_player_door(d, 1, &actor)
    }

    /// The same, from the square south of the door's edge.
    fn toggle(w: &mut World, d: ObjectRef, keys: &[i32]) -> pz_inventory::floor_drop::HostResult<()> {
        toggle_at(w, d, (d.x, d.y, d.z), keys)
    }

    fn lock(w: &World, r: ObjectRef, key_id: i32) {
        let object = w.object(r).unwrap();
        let mut o = object.borrow_mut();
        o.key_id = key_id;
        if let ObjectKind::Door(d) = &mut o.kind {
            d.locked = true;
        }
    }

    fn found(w: &mut World, q: &ContextQuery, dir: IsoDirections) -> Option<ObjectRef> {
        w.adapter().context_door_or_window_object_at(q, dir)
    }

    fn toggles(w: &mut World, q: &ContextQuery, dir: IsoDirections, obj: ObjectRef) -> Vec<Kind> {
        let actions = w.adapter().context_do_door_or_window_at(q, dir, obj).unwrap();
        actions.iter().for_each(|a| assert_eq!((a.dir, a.object), (Some(dir), Some(obj.id as i64))));
        actions.iter().map(|a| a.action).collect()
    }

    fn is_open(w: &World, r: ObjectRef) -> bool {
        matches!(&w.object(r).unwrap().borrow().kind, ObjectKind::Door(d) if d.open)
    }

    #[test]
    fn a_closed_door_in_front_opens_and_closes() {
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, false);
        let q = query(10, 10);
        assert_eq!(found(&mut w, &q, IsoDirections::N), Some(d));
        assert_eq!(toggles(&mut w, &q, IsoDirections::N, d), vec![Kind::ToggleDoor]);
        toggle(&mut w, d, &[]).unwrap();
        assert!(is_open(&w, d));
        assert_eq!(w.object(d).unwrap().borrow().sprite, Some(w.m.named_map["door_n_open"]));
        assert!(w.engine.played.last().unwrap().ends_with("Open"));
        // The open door is still found from the square, and closes again.
        assert_eq!(found(&mut w, &q, IsoDirections::N), Some(d));
        assert_eq!(toggles(&mut w, &q, IsoDirections::N, d), vec![Kind::ToggleDoor]);
        toggle(&mut w, d, &[]).unwrap();
        assert!(!is_open(&w, d));
        assert_eq!(w.object(d).unwrap().borrow().sprite, Some(w.m.named_map["door_n"]));
        assert!(w.engine.played.last().unwrap().ends_with("Close"));
    }

    #[test]
    fn a_door_is_found_from_the_other_side_of_its_edge() {
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, false);
        // Standing north of it, facing south.
        assert_eq!(found(&mut w, &query(10, 9), IsoDirections::S), Some(d));
        // Not through the sides, and not where there is no door.
        assert_eq!(found(&mut w, &query(10, 10), IsoDirections::E), None);
        assert_eq!(found(&mut w, &query(10, 10), IsoDirections::W), None);
        assert_eq!(found(&mut w, &query(12, 12), IsoDirections::N), None);
        // An open door swings onto the west edge of its square.
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, true);
        assert_eq!(found(&mut w, &query(10, 10), IsoDirections::W), Some(d));
    }

    #[test]
    fn an_open_door_is_still_found_by_the_square_across_its_edge() {
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, true);
        // The open door does not block the edge, so `getDoor(NORTH_SOUTH)`
        // of the neighbour finds it after `getOpenDoor` and the closed-state
        // lookup have not.
        assert_eq!(found(&mut w, &query(10, 9), IsoDirections::S), Some(d));
    }

    #[test]
    fn a_door_in_the_way_is_not_toggled() {
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, false);
        w.add(10, 10, "solid_block", ObjectKind::Object, false);
        toggle(&mut w, d, &[]).unwrap();
        assert!(!is_open(&w, d));
        assert!(w.engine.played.is_empty());
    }

    /// Puts `(x, y)` in room `room`; a square without a room is outside.
    fn set_room(w: &mut World, x: i32, y: i32, room: i64) {
        let sq = w.cell.get_grid_square_mut(x, y, 0).unwrap();
        sq.borrow_mut().room_id = room;
        sq.borrow_mut().recalc_properties(&w.m);
    }

    #[test]
    fn a_locked_door_stays_shut_without_its_key() {
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, false);
        lock(&w, d, 77);
        toggle(&mut w, d, &[]).unwrap();
        assert!(!is_open(&w, d));
        assert!(w.engine.played.last().unwrap().ends_with("Locked"));
        // Another key does not fit.
        toggle(&mut w, d, &[78]).unwrap();
        assert!(!is_open(&w, d));
        assert!(matches!(&w.object(d).unwrap().borrow().kind, ObjectKind::Door(k) if k.locked && k.locked_by_key));
    }

    #[test]
    fn a_locked_door_opens_with_its_key_and_stays_unlocked() {
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, false);
        lock(&w, d, 77);
        toggle(&mut w, d, &[3, 77]).unwrap();
        assert!(is_open(&w, d));
        assert!(w.engine.played.iter().any(|s| s.ends_with("Unlock")));
        assert!(w.engine.played.last().unwrap().ends_with("Open"));
        assert!(matches!(&w.object(d).unwrap().borrow().kind, ObjectKind::Door(k) if !k.locked && !k.locked_by_key));
        // Closed again, it opens without the key.
        toggle(&mut w, d, &[]).unwrap();
        assert!(!is_open(&w, d));
        toggle(&mut w, d, &[]).unwrap();
        assert!(is_open(&w, d));
    }

    #[test]
    fn a_door_locked_without_a_key_id_cannot_be_opened() {
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, false);
        lock(&w, d, -1);
        toggle(&mut w, d, &[77]).unwrap();
        assert!(!is_open(&w, d));
        assert!(w.engine.played.last().unwrap().ends_with("Locked"));
    }

    #[test]
    fn a_door_is_opened_from_inside_without_a_key() {
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, false);
        lock(&w, d, 77);
        for y in [9, 10] {
            set_room(&mut w, 10, y, 5);
        }
        // Standing in the room, on either side of the door.
        toggle_at(&mut w, d, (10, 9, 0), &[]).unwrap();
        assert!(is_open(&w, d));
        // From another room, or from outside, the door stays locked.
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, false);
        lock(&w, d, 77);
        set_room(&mut w, 10, 10, 5);
        set_room(&mut w, 11, 10, 6);
        toggle_at(&mut w, d, (11, 10, 0), &[]).unwrap();
        assert!(!is_open(&w, d));
    }

    #[test]
    fn a_force_locked_door_is_not_opened_from_inside() {
        let mut w = World::new();
        let mut k = door(true, 500);
        if let ObjectKind::Door(d) = &mut k {
            d.closed_sprite = Some(w.m.named_map["door_n"]);
            d.open_sprite = Some(w.m.named_map["door_n_open"]);
        }
        let d = w.add(10, 10, "door_n_forcelocked", k, true);
        lock(&w, d, 77);
        for y in [9, 10] {
            set_room(&mut w, 10, y, 5);
        }
        toggle_at(&mut w, d, (10, 10, 0), &[]).unwrap();
        assert!(!is_open(&w, d));
    }

    #[test]
    fn a_custom_lock_asks_for_the_key() {
        use pz_inventory::lua_table::{LuaTable, LuaValue};
        use pz_world::save::lua_table::LuaKey;
        let mut w = World::new();
        let d = north_door(&mut w, 10, 10, false);
        w.object(d).unwrap().borrow_mut().key_id = 31;
        w.object(d).unwrap().borrow_mut().mod_data = Some(LuaTable {
            entries: vec![(LuaKey::Str("CustomLock".into()), LuaValue::Bool(true))],
        });
        toggle(&mut w, d, &[]).unwrap();
        assert!(!is_open(&w, d));
        toggle(&mut w, d, &[31]).unwrap();
        assert!(is_open(&w, d));
    }

    #[test]
    fn doors_the_port_does_not_cover_fail_loudly() {
        let mut w = World::new();
        let k = ObjectKind::Object;
        let o = w.add(10, 10, "door_n", k, true);
        assert!(matches!(
            toggle(&mut w, o, &[]).unwrap_err(),
            pz_inventory::floor_drop::RuntimeHostError::Unsupported { .. }
        ));
    }

    #[test]
    fn a_closed_window_is_toggled_with_a_short_or_long_press() {
        let mut w = World::new();
        let kind = window(&w, 50);
        let win = w.add(10, 10, "window_w", kind, true);
        let mut q = query(10, 10);
        assert_eq!(found(&mut w, &q, IsoDirections::W), Some(win));
        assert_eq!(toggles(&mut w, &q, IsoDirections::W, win), vec![Kind::ToggleWindow]);
        q.time_pressed_context = 0.6;
        assert_eq!(toggles(&mut w, &q, IsoDirections::W, win), vec![Kind::ToggleWindow]);
    }

    #[test]
    fn a_curtain_key_press_is_not_guessed() {
        let mut w = World::new();
        let kind = window(&w, 50);
        let win = w.add(10, 10, "window_w", kind, true);
        let mut q = query(10, 10);
        q.shift = true;
        assert!(w.adapter().context_do_door_or_window_at(&q, IsoDirections::W, win).is_err());
    }

    #[test]
    fn a_fence_is_hopped_from_the_square_that_has_it() {
        let mut w = World::new();
        w.add(10, 10, "fence_n", ObjectKind::Object, false);
        let a = w.adapter();
        let (sq, object) = a.context_hoppable_at(&query(10, 10), IsoDirections::N).unwrap();
        assert_eq!(sq, (10, 10, 0));
        assert!(object.is_some());
        // Seen from the north it is the south neighbour's edge.
        assert_eq!(a.context_hoppable_at(&query(10, 9), IsoDirections::S).unwrap().0, (10, 10, 0));
        assert!(a.context_hoppable_at(&query(10, 10), IsoDirections::W).is_none());
    }

    #[test]
    fn a_sheet_rope_is_found_on_its_side_of_the_square() {
        let mut w = World::new();
        w.add(10, 10, "sheet_w", ObjectKind::Object, false);
        let a = w.adapter();
        assert!(a.context_sheet_rope_side_at(&query(10, 10), IsoDirections::W));
        assert!(!a.context_sheet_rope_side_at(&query(10, 10), IsoDirections::N));
    }

    #[test]
    fn nothing_special_on_an_empty_square() {
        let mut w = World::new();
        let a = w.adapter();
        for which in [ContextSpecial::Hutch, ContextSpecial::ButcherHook, ContextSpecial::RestOnFurniture] {
            assert_eq!(a.context_special_at(&query(10, 10), IsoDirections::N, which).unwrap(), None);
        }
    }
}
#[test]
fn characters_by_id() {
    let mut w = World::new();
    let mut list = vec![zombie(10.5, 10.5, 1), zombie(12.5, 10.5, 1), zombie(20.5, 10.5, 1)];
    for (i, z) in list.iter_mut().enumerate() {
        z.id = 10 + i as i32;
    }
    {
        let a = w.adapter();
        assert!(StateWorld::<()>::characters_near(&a, 10.5, 10.5, 0.0, 3.0).is_empty());
    }
    let mut a = w.adapter().with_characters(&mut list);
    assert_eq!(StateWorld::<()>::characters_near(&a, 10.5, 10.5, 0.0, 3.0), vec![10, 11]);
    StateWorld::<()>::character_mut(&mut a, 11).unwrap().health = 0.5;
    assert!(StateWorld::<()>::character_mut(&mut a, 99).is_none());
    // A taken character goes back to its place in the list.
    let z = StateWorld::<()>::take_character(&mut a, 11).unwrap();
    assert!(StateWorld::<()>::character_mut(&mut a, 11).is_none());
    StateWorld::<()>::put_character(&mut a, z);
    // Nested takes, put back in reverse order, keep the order too.
    let z10 = StateWorld::<()>::take_character(&mut a, 10).unwrap();
    let z12 = StateWorld::<()>::take_character(&mut a, 12).unwrap();
    assert_eq!(StateWorld::<()>::characters_near(&a, 10.5, 10.5, 0.0, 100.0), vec![11]);
    StateWorld::<()>::put_character(&mut a, z12);
    StateWorld::<()>::put_character(&mut a, z10);
    drop(a);
    assert_eq!(list.iter().map(|z| z.id).collect::<Vec<_>>(), vec![10, 11, 12]);
    assert_eq!(list[1].health, 0.5);
}

#[test]
fn moving_squares_list_the_characters_on_them() {
    let mut w = World::new();
    let mut z = zombie(9.5, 9.5, 1);
    let mut a = w.adapter();
    let on = |a: &StatesWorldAdapter<'_>, x, y| StateWorld::<()>::characters_on_square(a, (x, y, 0));
    z.set_moving_square(Some((9, 9, 0)), &mut a);
    z.set_moving_square(Some((9, 9, 0)), &mut a);
    assert_eq!(on(&a, 9, 9), vec![3]);
    z.set_moving_square(Some((10, 9, 0)), &mut a);
    assert!(on(&a, 9, 9).is_empty());
    assert_eq!(on(&a, 10, 9), vec![3]);
    z.path.current_square = Some((10, 9, 0));
    z.remove_from_square(&mut a);
    assert!(on(&a, 10, 9).is_empty());
    assert_eq!((z.path.current_square, z.path.moving_square), (None, None));
}

/// The game settings the path behaviours read: the defaults.
struct PathGame;

impl pz_pathfind::mover::PathWorldHooks for PathGame {
    fn lore_speed(&self) -> i32 {
        2
    }
    fn easy_climbing(&self) -> bool {
        false
    }
    fn bent_fences_enabled(&self) -> bool {
        false
    }
    fn is_client(&self) -> bool {
        false
    }
    fn is_server(&self) -> bool {
        false
    }
    fn game_time_multiplier(&self) -> f32 {
        1.0
    }
    fn thirty_fps_multiplier(&self) -> f32 {
        1.0
    }
    fn current_game_speed(&self) -> i32 {
        1
    }
}

#[test]
fn the_states_find_paths_through_the_host() {
    use pz_pathfind::host::PathHost;
    use pz_pathfind::service::PathfindService;
    let mut w = World::new();
    let mut list = vec![zombie(10.5, 10.5, 1), zombie(14.5, 12.5, 1)];
    list[0].id = 10;
    list[1].id = 20;
    let target = list[1].mover_id_of();
    // Without a host the path calls do nothing.
    {
        let mut a = w.adapter().with_characters(&mut list);
        let mut z = StateWorld::<()>::take_character(&mut a, 10).unwrap();
        StateWorld::<()>::path_to_character(&mut a, &mut z, target);
        assert!(StateWorld::<()>::path_behavior(&a, 10).is_none());
        StateWorld::<()>::put_character(&mut a, z);
    }
    let mut host = PathHost::new(PathfindService::new(0, 0, 1, 1, false));
    let game = PathGame;
    let mut a = w.adapter().with_characters(&mut list).with_path(&mut host, &game);
    // The zombie is out of the list while it updates; its target is found
    // in the list by mover id.
    let mut z = StateWorld::<()>::take_character(&mut a, 10).unwrap();
    StateWorld::<()>::path_to_character(&mut a, &mut z, target);
    let b = StateWorld::<()>::path_behavior(&a, 10).expect("no PathFindBehavior2 for the zombie");
    assert!(b.is_goal_character());
    assert_eq!(b.target_char(), Some(target));
    assert_eq!((b.target_x(), b.target_y()), (14.5, 12.5));
    // A location goal replaces it.
    StateWorld::<()>::path_to_location(&mut a, &mut z, 3, 4, 0);
    let b = StateWorld::<()>::path_behavior(&a, 10).unwrap();
    assert!(b.is_goal_location());
    assert_eq!((b.target_x(), b.target_y()), (3.5, 4.5));
    StateWorld::<()>::put_character(&mut a, z);
    // A target that is not in the list is not followed.
    let mut other = zombie(1.5, 1.5, 1);
    other.id = 30;
    let gone = zombie(2.5, 2.5, 1).mover_id_of();
    StateWorld::<()>::path_to_character(&mut a, &mut other, gone);
    assert!(StateWorld::<()>::path_behavior(&a, 30).map_or(true, |b| !b.is_goal_character()));
}

#[test]
fn facing_uses_frame_sprite_and_resolves_the_closest_live_grid_member() {
    use pz_entity::runtime_load::RuntimeHostError;
    use pz_world::sprite::{runtime::{create_runtime_sprite, ObjectSprite}, IsoSpriteGrid};
    use std::{cell::RefCell, rc::Rc};

    let mut w = World::new();
    let first = w.add(10, 10, "crate", ObjectKind::WindowFrame { north: true }, false);
    let second = w.add(11, 10, "wall_w", ObjectKind::Object, false);
    let frame = w.object(first).unwrap();
    let second = w.object(second).unwrap();
    let square = w.cell.get_grid_square(10, 10, 0).unwrap();
    let square = square.borrow();
    assert_eq!(pz_world::world_state::barricade_able_north(&frame.borrow()), Some(true));
    assert_eq!(w.adapter().try_closest_facing_position(&square, &frame.borrow(), 14.0, 10.0).unwrap(),
        Some((10.5, 10.5)));

    let tile = w.m.named_map["crate"];
    let mut grid = IsoSpriteGrid::new(2, 1, 1);
    grid.set_sprite(0, 0, 0, tile);
    let group = w.m.grids.len();
    w.m.grids.push(grid);
    w.m.sprites[tile].sprite_grid = Some(group);
    let manager = Rc::new(RefCell::new(manager()));
    let actual = create_runtime_sprite(&manager, &w.m.tables, &mut pz_entity::RetiredComponents::new()).unwrap();
    actual.borrow_mut().data = w.m.sprites[w.m.named_map["wall_w"]].clone();
    actual.borrow_mut().data.sprite_grid = Some(group);
    second.borrow_mut().sprite = ObjectSprite::Native(actual.clone());

    let query = |w: &mut World, x| w.adapter().try_closest_facing_position(&square, &frame.borrow(), x, 10.5);
    assert_eq!(query(&mut w, 11.0).unwrap(), Some((10.5, 10.5)), "first member wins the tie");
    assert_eq!(query(&mut w, 11.75).unwrap(), Some((11.0, 10.5)), "native sprite supplies the west wall edge");
    let held = actual.borrow_mut();
    assert!(matches!(query(&mut w, 11.75), Err(RuntimeHostError::Borrowed(_))));
    drop(held);
    let second_square = w.cell.get_grid_square(11, 10, 0).unwrap();
    assert_eq!(w.adapter().try_closest_facing_position(&second_square.borrow(), &second.borrow(), 14.0, 10.0).unwrap(),
        None, "an unindexed sprite has offsets (-1,-1,-1) and its search finds no upstairs member");
}
