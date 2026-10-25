//! The adapter on hand-built squares with invented objects, tables and
//! random draws (no game data).

use std::collections::HashMap;

use pz_characters::character::state::{StateWorld, ThumpKind, ThumpTarget};
use pz_characters::{CharacterKind, GameCharacter};
use pz_pathfind::mover::ObjectRef;
use pz_scripts::ScriptManager;
use pz_world::java::GameRand;
use pz_world::objects::{IsoBarricade, IsoDoor, IsoObject, IsoThumpable, IsoWindow, ObjectKind};
use pz_world::sprite::PropertyValueMap;
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
    fn object_removed(&mut self, _sq: (i32, i32, i32), obj: &IsoObject, destroyed: bool) {
        self.removed.push((obj.kind.class_name().into(), destroyed));
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
                sq.objects.push(IsoObject::new(
                    id,
                    ObjectKind::Object,
                    Some(w.m.named_map["floor"]),
                ));
                sq.recalc_properties(&w.m);
                let ch = w
                    .cell
                    .chunks
                    .get_mut(&(x.div_euclid(8), y.div_euclid(8)))
                    .unwrap();
                ch.set_square(x.rem_euclid(8), y.rem_euclid(8), 0, Some(sq));
            }
        }
        w
    }

    fn add(&mut self, x: i32, y: i32, sprite: &str, kind: ObjectKind, special: bool) -> ObjectRef {
        let spr = self.m.named_map[sprite];
        let sq = self.cell.get_grid_square_mut(x, y, 0).unwrap();
        let id = sq.new_object_id();
        sq.objects.push(IsoObject::new(id, kind, Some(spr)));
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
            &self.m,
            self.settings,
            &mut self.rt,
            &mut self.engine,
        )
    }

    fn kind(&self, r: ObjectRef) -> Option<&ObjectKind> {
        let sq = self.cell.get_grid_square(r.x, r.y, r.z)?;
        sq.objects.iter().find(|o| o.id == r.id).map(|o| &o.kind)
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
    if let ObjectKind::Door(dd) =
        &mut w.cell.get_grid_square_mut(10, 10, 0).unwrap().objects[1].kind
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
    let planks = |w: &World| match w.kind(b) {
        Some(ObjectKind::Barricade(b)) => Some(b.plank_health),
        _ => None,
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
    let health = |w: &World| match w.kind(d) {
        Some(ObjectKind::Door(dd)) => Some(dd.health),
        _ => None,
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
    assert!(matches!(w.kind(win), Some(ObjectKind::Window(x)) if x.health == 20 && !x.destroyed));
    w.engine.rand = Seq(vec![2]);
    thump(&mut w, win, &mut z);
    let sq = w.cell.get_grid_square(12, 12, 0).unwrap();
    let o = sq.objects.iter().find(|o| o.id == win.id).unwrap();
    assert!(matches!(&o.kind, ObjectKind::Window(x) if x.destroyed && x.health == 0));
    assert_eq!(o.sprite, Some(w.m.named_map["window_w_smashed"]));
    // `damage` smashed it and dropped glass on the far side (the zombie
    // is east of the west edge); the thump's own break block is skipped
    // once destroyed, so the target stays.
    assert!(!sq
        .objects
        .iter()
        .any(|o| matches!(o.kind, ObjectKind::BrokenGlass)));
    let far = w.cell.get_grid_square(11, 12, 0).unwrap();
    let glass = far
        .objects
        .iter()
        .find(|o| matches!(o.kind, ObjectKind::BrokenGlass))
        .unwrap();
    assert_eq!(glass.sprite, Some(w.m.named_map["brokenglass_1_2"]));
    assert!(far.special_objects.contains(&glass.id));
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
    let hp = |w: &World| match w.kind(t) {
        Some(ObjectKind::Thumpable(x)) => Some(x.health),
        _ => None,
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
    assert!(w.kind(t2).is_some());
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
