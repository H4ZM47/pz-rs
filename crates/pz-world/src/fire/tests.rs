//! Fires on hand-built squares (made-up sprites and values), with a
//! scripted `Rand` that records the calls in order.

use super::character::{
    fire_check, reduce_health_when_burning, spread_fire, CharacterFireFrame, CharacterKind,
    FireCharacter,
};
use super::manager::{FireFlags, NoFireHooks};
use super::*;
use crate::fire::manager::{fire_body, fire_body_mut};
use crate::objects::{IsoDoor, IsoFireplace, IsoGenerator, IsoObject, ObjectKind};
use crate::sprite::{test_tables, IsoFlagType, PropertyValueMap, SpriteTables};
use crate::ObjectSlot;
use crate::{IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoSpriteManager};

/// Returns scripted values (0 once they run out) and records each call.
#[derive(Default)]
struct Script {
    ints: Vec<i32>,
    calls: Vec<String>,
}

impl Script {
    fn new(ints: &[i32]) -> Self {
        Script {
            ints: ints.to_vec(),
            calls: Vec::new(),
        }
    }
}

impl GameRand for Script {
    fn next(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        self.calls.push(format!("next({max})"));
        if self.ints.is_empty() {
            0
        } else {
            self.ints.remove(0).min(max - 1)
        }
    }

    fn next_float_range(&mut self, min: f32, max: f32) -> f32 {
        self.calls.push(format!("float({min},{max})"));
        min
    }
}

pub(super) fn tables() -> SpriteTables {
    let t = test_tables();
    let mut flags: Vec<String> = (0..t.flag_count())
        .map(|i| t.flag_name(IsoFlagType(i as u8)).to_string())
        .collect();
    for f in [
        "burning",
        "smoke",
        "unflamable",
        "DoorWallW",
        "DoorWallN",
        "bed",
        "waterPiped",
        "attachedS",
        "attachedE",
        "WallN",
        "WallW",
        "WallNW",
        "climbSheetTopN",
        "climbSheetTopS",
        "climbSheetTopW",
        "climbSheetTopE",
        "climbSheetS",
        "climbSheetW",
        "climbSheetE",
    ] {
        flags.push(f.into());
    }
    let types: Vec<String> = (0..t.object_type_max().index())
        .map(|i| {
            t.object_type_name(crate::sprite::IsoObjectType(i as u8))
                .to_string()
        })
        .collect();
    SpriteTables::new(flags, types, vec!["North".into(), "West".into()]).unwrap()
}

/// `(name, flags, tile type, firerequirement, burntTile)`.
type Spr = (
    &'static str,
    &'static [&'static str],
    &'static str,
    i32,
    Option<&'static str>,
);

const SPRITES: &[Spr] = &[
    ("grass", &["solidfloor", "vegitation"], "normal", 0, None),
    ("carpet", &["solidfloor"], "normal", 30, None),
    ("stone", &["solidfloor"], "normal", 8, None),
    ("immune", &["solidfloor"], "normal", 800_000, None),
    ("pond", &["solidfloor", "water"], "normal", 0, None),
    ("wall_w", &["collideW"], "wall", 0, None),
    ("door", &["doorN"], "doorN", 0, None),
    ("bench", &["solidtrans"], "normal", 0, None),
    ("table", &["solidtrans"], "normal", 0, Some("table_burnt")),
    ("gen", &[], "normal", 0, None),
    ("tree", &[], "tree", 0, None),
    ("walls_burnt_01_0", &["collideW"], "wall", 0, None),
    ("table_burnt", &[], "normal", 0, None),
    ("frame", &[], "normal", 0, None),
    ("rope_top", &["climbSheetTopN"], "normal", 0, None),
    ("hutch", &[], "normal", 0, None),
    // A two-part player-built garage door (north-facing, parts at x and x+1).
    (
        "gdoor1",
        &["GarageDoor=1"],
        "normal",
        0,
        Some("walls_burnt_01_0"),
    ),
    ("gdoor2", &["GarageDoor=2", "attachedN"], "normal", 0, None),
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
        "doorTrans",
    ] {
        map.entry(k);
    }
    map.entry("GarageDoor")
        .extend(["1".to_string(), "2".to_string()]);
    let mut m = IsoSpriteManager::new(tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (i, (name, props, ty, req, burnt)) in SPRITES.iter().enumerate() {
        let s = m.add_sprite(name, 500 + i as i32);
        let (t, alias) = (m.tables.clone(), m.alias.clone());
        let sp = &mut m.sprites[s];
        sp.name = Some(name.to_string());
        sp.tile_type = t.object_type_from_string(ty);
        sp.firerequirement = *req;
        sp.burnt_tile = burnt.map(str::to_string);
        for k in *props {
            let (k, v) = k.split_once('=').unwrap_or((k, ""));
            sp.properties.set(&t, &alias, k, v, true);
        }
    }
    m
}

/// Chunks (0, 0) and (1, 0): squares x 0..16, y 0..8 on level 0.
struct World {
    cell: IsoCell,
    m: IsoSpriteManager,
    meta: IsoMetaGrid,
    flags: FireFlags,
    settings: FireSettings,
    regions: crate::isoregion::IsoRegions,
    doors: crate::square_mutation::DoorTables,
    mw: crate::square_mutation::NoMutationHooks,
    mp: crate::square_mutation::NoMutationHooks,
    mr: crate::square_mutation::NoMutationHooks,
}

impl World {
    fn new() -> World {
        let mut cell = IsoCell::default();
        cell.chunks.insert((0, 0), IsoChunk::new(0, 0));
        cell.chunks.insert((1, 0), IsoChunk::new(1, 0));
        let m = manager();
        let flags = FireFlags::resolve(&m.tables).unwrap();
        World {
            cell,
            m,
            meta: IsoMetaGrid::empty(),
            flags,
            settings: FireSettings::default(),
            regions: crate::isoregion::IsoRegions::new(13),
            doors: crate::square_mutation::DoorTables {
                x_offset: [[[0; 4]; 2]; 2],
                y_offset: [[[0; 4]; 2]; 2],
            },
            mw: crate::square_mutation::NoMutationHooks,
            mp: crate::square_mutation::NoMutationHooks,
            mr: crate::square_mutation::NoMutationHooks,
        }
    }

    /// A square with `objects` (sprite names), exterior unless `indoor`.
    fn put(&mut self, x: i32, y: i32, indoor: bool, objects: &[&str]) {
        let mut sq = IsoGridSquare::new(x, y, 0);
        for n in objects {
            let id = sq.new_object_id();
            sq.objects
                .push((IsoObject::new(id, ObjectKind::Object, Some(self.m.named_map[*n]))).into());
        }
        sq.have_roof = indoor;
        sq.recalc_properties(&self.m);
        let ch = self.cell.chunks.get_mut(&(x.div_euclid(8), 0)).unwrap();
        ch.set_square(x.rem_euclid(8), y, 0, Some((sq).into()));
    }

    fn add(&mut self, x: i32, y: i32, sprite: &str, kind: ObjectKind) {
        let s = self.m.named_map[sprite];
        let sq_owner = self.cell.get_grid_square_mut(x, y, 0).unwrap();
        let mut sq = sq_owner.borrow_mut();
        let id = sq.new_object_id();
        sq.objects.push((IsoObject::new(id, kind, Some(s))).into());
        sq.special_objects.push(id);
    }

    fn fw<'a>(&'a mut self, hooks: &'a mut dyn FireHooks) -> FireWorld<'a> {
        FireWorld {
            cell: &mut self.cell,
            meta: &self.meta,
            sprites: &mut self.m,
            mutation: FireMutation {
                regions: &mut self.regions,
                doors: &self.doors,
                world: &mut self.mw,
                pathfind: &mut self.mp,
                render: &mut self.mr,
                num_players: 1,
            },
            flags: self.flags,
            settings: self.settings,
            hooks,
        }
    }

    fn sq(&self, x: i32, y: i32) -> crate::SquareRef {
        self.cell.get_grid_square(x, y, 0).unwrap()
    }

    fn sprite_names(&self, x: i32, y: i32) -> Vec<String> {
        self.sq(x, y)
            .borrow()
            .objects
            .iter()
            .map(ObjectSlot::borrow)
            .map(|o| match o.sprite {
                Some(s) => self.m.sprites[s].name().to_string(),
                None => o.kind.class_name().to_string(),
            })
            .collect()
    }
}

/// Records the hooks the fire calls.
#[derive(Default)]
struct Log(Vec<String>);

impl sound::FireSoundHooks for Log {}
impl FireHooks for Log {
    fn world_sound(&mut self, x: i32, y: i32, _: i32, radius: i32, volume: i32) {
        self.0.push(format!("sound {x},{y} {radius}/{volume}"));
    }
    fn play_world_sound(&mut self, name: &str, x: i32, y: i32, _: i32, _: f32, _: f32) {
        self.0.push(format!("{name} {x},{y}"));
    }
    fn lua_event(&mut self, name: &str, x: i32, y: i32, _: i32) {
        self.0.push(format!("{name} {x},{y}"));
    }
    fn fire_updated(&mut self, r: FireRef, fire: &IsoFire) {
        self.0.push(format!(
            "updated {},{} light {:?}",
            r.x,
            r.y,
            fire.light.map(|l| l.radius)
        ));
    }
    fn fire_removed(&mut self, r: FireRef) {
        self.0.push(format!("removed {},{}", r.x, r.y));
    }
}

#[test]
fn starting_a_fire_draws_rand_like_the_constructor() {
    let mut w = World::new();
    w.put(2, 2, false, &["grass"]);
    let mut mgr = IsoFireManager::new();
    let mut log = Log::default();
    // Life 800 + 1000, spread delay 400, then the sound roll hits.
    let mut r = Script::new(&[1000, 400, 0]);
    let at = mgr
        .start_fire(&mut w.fw(&mut log), &mut r, 2, 2, 0, false, 20, 0)
        .unwrap();
    assert_eq!(
        r.calls,
        ["float(0,30)", "next(2200)", "next(900)", "next(5)"]
    );
    assert_eq!(log.0, ["OnNewFire 2,2", "sound 2,2 20/20"]);
    let f = fire_body(&mgr.fire(&w.cell, at).unwrap()).clone();
    // Vegetation adds 50 energy; the grass floor needs nothing.
    assert_eq!((f.life, f.spread_delay, f.energy), (1800, 400, 70));
    assert_eq!(f.life_stage_duration, 360);
    assert!(w.sq(2, 2).borrow().has(w.flags.burning));
    assert_eq!(mgr.fire_stack, vec![at]);
    w.put(3, 2, false, &["pond"]);
    w.put(4, 2, false, &[]);
    w.put(5, 2, false, &["carpet"]);
    let burnt = w.m.tables.flag.burntOut;
    w.cell
        .get_grid_square_mut(5, 2, 0)
        .unwrap()
        .borrow_mut()
        .properties
        .set_flag(burnt);
    // A burning square takes no second fire; nor does water, an empty
    // square, a missing one, or burnt ground (unless anywhere goes).
    let fw = w.fw(&mut log);
    assert!(!fw.can_add_fire(Some((2, 2, 0)), true));
    assert!(!fw.can_add_fire(Some((3, 2, 0)), true));
    assert!(!fw.can_add_fire(Some((4, 2, 0)), true));
    assert!(!fw.can_add_fire(Some((9, 9, 0)), true));
    assert!(!fw.can_add_fire(Some((5, 2, 0)), false));
    assert!(fw.can_add_fire(Some((5, 2, 0)), true));
}

#[test]
fn a_poor_floor_shortens_the_fire_and_takes_energy() {
    let mut w = World::new();
    w.put(1, 1, true, &["stone"]);
    let mut mgr = IsoFireManager::new();
    let mut r = Script::new(&[]);
    let at = mgr
        .start_fire(&mut w.fw(&mut NoFireHooks), &mut r, 1, 1, 0, true, 10, 0)
        .unwrap();
    let f = fire_body(&mgr.fire(&w.cell, at).unwrap()).clone();
    // 10 less the floor's 8 is under the minimum of 5.
    assert_eq!(f.energy, 5);
    // No life given: 800 less a hundred per point of the requirement,
    // under 600, so Rand.Next(300, 600).
    assert_eq!(f.life, 300);
    assert_eq!(r.calls[..3], ["float(0,30)", "next(2200)", "next(300)"]);
}

#[test]
fn burning_turns_walls_and_furniture_to_burnt_tiles() {
    let mut w = World::new();
    w.put(
        2,
        2,
        true,
        &["carpet", "wall_w", "bench", "table", "tree", "immune"],
    );
    w.add(2, 2, "door", ObjectKind::Door(IsoDoor::default()));
    let mut mgr = IsoFireManager::new();
    let mut log = Log::default();
    // Wall: Rand.Next(2) == 0 picks "0"; bench: floors_burnt_01_ +
    // Next(1, 2); tree: fencing_burnt_01_ + Next(15, 19) + 1.
    let mut r = Script::new(&[0, 0, 2]);
    burn::burn(&mut mgr, &mut w.fw(&mut log), &mut r, (2, 2, 0)).unwrap();
    assert_eq!(r.calls, ["next(2)", "next(1)", "next(4)"]);
    // The indoor carpet burns to floors_burnt_01_0; the new burnt sprites
    // have no name (AddSprite); the fireproof floor stays; the door is
    // gone.
    assert_eq!(
        w.sprite_names(2, 2),
        ["", "walls_burnt_01_0", "", "table_burnt", "", "immune"]
    );
    for n in [
        "floors_burnt_01_0",
        "floors_burnt_01_1",
        "fencing_burnt_01_18",
    ] {
        assert!(w.m.named_map.contains_key(n), "{n}");
    }
    let sq = w.sq(2, 2);
    assert!(sq.borrow().burnt_out && sq.borrow().has(w.m.tables.flag.burntOut));
    assert!(sq.borrow().special_objects.is_empty());
    assert!(log.0.contains(&"OnGridBurnt 2,2".to_string()));
    // Burning again leaves the burnt tiles alone.
    let mut r = Script::new(&[]);
    burn::burn(&mut mgr, &mut w.fw(&mut log), &mut r, (2, 2, 0)).unwrap();
    assert!(r.calls.is_empty());
}

#[test]
fn a_fuelled_generator_explodes_into_fires_around() {
    let mut w = World::new();
    for x in 0..6 {
        for y in 0..6 {
            w.put(x, y, false, &["grass"]);
        }
    }
    let g = IsoGenerator {
        connected: false,
        activated: true,
        fuel: 5.0,
        condition: 100,
        last_hour: 0,
        total_power_using: 0.0,
    };
    w.add(2, 2, "gen", ObjectKind::Generator(g));
    let mut mgr = IsoFireManager::new();
    let mut log = Log::default();
    // Power 20: Rand.Next(100) < 20 lights a square. (0, 0) rolls 0 and
    // takes energy Next(100, 270) = 150; the rest of its fire's rolls are
    // 0; every later square rolls 99.
    let mut ints = vec![0, 50];
    ints.extend([0; 3]);
    ints.extend([99; 40]);
    let mut r = Script::new(&ints);
    burn::burn(&mut mgr, &mut w.fw(&mut log), &mut r, (2, 2, 0)).unwrap();
    assert!(log.0.contains(&"BurnedObjectExploded 2,2".to_string()));
    // The grass sprite has no `exterior` flag of its own: a burnt floor.
    assert_eq!(w.sprite_names(2, 2), [""]);
    assert_eq!(mgr.fire_stack.len(), 1);
    assert_eq!((mgr.fire_stack[0].x, mgr.fire_stack[0].y), (0, 0));
    let f_owner = mgr.fire(&w.cell, mgr.fire_stack[0]).unwrap();
    let f = fire_body(&f_owner);
    // Plus the vegetation's 50.
    assert_eq!(f.energy, 200);
    assert_eq!(&r.calls[..3], ["next(100)", "next(170)", "float(0,30)"]);
    assert!(w.sq(0, 0).borrow().burnt_out);
    assert_eq!(mgr.fire_recalc, 1);
}

#[test]
fn a_fire_spreads_spending_its_energy_then_goes_out() {
    let mut w = World::new();
    w.put(2, 2, false, &["carpet"]);
    w.put(2, 1, false, &["carpet"]);
    let mut mgr = IsoFireManager::new();
    let mut fire = IsoFire::new_fire(FireSquare::default(), 35, 100, &mut Script::new(&[]));
    fire.life_stage = 1;
    fire.life_stage_timer = 50;
    fire.spread_timer = 1;
    fire.spread_delay = 9;
    let at = IsoFireManager::place(&mut w.fw(&mut NoFireHooks), 2, 2, 0, fire).unwrap();
    mgr.add_loaded(&mut w.fw(&mut NoFireHooks), &mut Script::new(&[]), at);
    assert!(w.sq(2, 2).borrow().has(w.flags.burning));
    let mut log = Log::default();
    // One spread (Next(3) = 0, no bonus), northwards (Next(13) = 0): (2, 1)
    // needs its floor's 30 of the 35, leaving 5; exterior, so the new fire
    // gets the 5 (the minimum, after its floor's 30).
    let mut r = Script::new(&[0, 1, 0]);
    let frame = FireFrame {
        multiplied_seconds: 0.0,
        thirty_fps: 1.5,
        now_millis: 0,
    };
    mgr.update(&mut w.fw(&mut log), &mut r, frame).unwrap();
    assert_eq!(
        &r.calls[..4],
        ["next(3)", "next(50)", "next(13)", "float(0,30)"]
    );
    assert_eq!(mgr.fire_stack.len(), 2);
    let f = fire_body(&mgr.fire(&w.cell, at).unwrap()).clone();
    assert_eq!((f.energy, f.spread_timer, f.age), (5, 9, 1));
    let new = fire_body(&mgr.fire(&w.cell, mgr.fire_stack[1]).unwrap()).clone();
    assert_eq!(new.energy, 5);
    assert_eq!(log.0.last().unwrap(), "updated 2,2 light Some(5)");
    // At no energy the first fire goes out.
    fire_body_mut(&IsoFireManager::fire_mut(&mut w.cell, at).unwrap()).energy = 0;
    let mut log = Log::default();
    mgr.update(&mut w.fw(&mut log), &mut Script::new(&[]), frame)
        .unwrap();
    assert!(log.0.contains(&"removed 2,2".to_string()));
    assert_eq!(mgr.fire_stack.len(), 1);
    assert!(!w.sq(2, 2).borrow().has(w.flags.burning));
    assert_eq!(w.sprite_names(2, 2), ["carpet"]);
}

#[test]
fn rain_spends_the_energy_without_spreading() {
    let mut w = World::new();
    w.put(2, 2, false, &["carpet"]);
    w.put(2, 1, false, &["carpet"]);
    w.settings.raining = true;
    let mut mgr = IsoFireManager::new();
    let mut fire = IsoFire::new_fire(FireSquare::default(), 35, 100, &mut Script::new(&[]));
    fire.life_stage = 1;
    fire.life_stage_timer = 50;
    fire.spread_timer = 1;
    let at = IsoFireManager::place(&mut w.fw(&mut NoFireHooks), 2, 2, 0, fire).unwrap();
    mgr.add_loaded(&mut w.fw(&mut NoFireHooks), &mut Script::new(&[]), at);
    let frame = FireFrame {
        multiplied_seconds: 0.0,
        thirty_fps: 1.5,
        now_millis: 0,
    };
    mgr.update(
        &mut w.fw(&mut NoFireHooks),
        &mut Script::new(&[0, 1, 0]),
        frame,
    )
    .unwrap();
    assert_eq!(mgr.fire_stack.len(), 1);
    assert_eq!(fire_body(&mgr.fire(&w.cell, at).unwrap()).energy, 5);
    // With FireSpread off nothing is spent.
    w.settings = FireSettings {
        fire_spread: false,
        ..Default::default()
    };
    let f_owner = IsoFireManager::fire_mut(&mut w.cell, at).unwrap();
    let mut f = fire_body_mut(&f_owner);
    f.energy = 35;
    f.spread_timer = 1;
    drop(f);
    let mut r = Script::new(&[]);
    mgr.update(&mut w.fw(&mut NoFireHooks), &mut r, frame)
        .unwrap();
    assert!(r.calls.is_empty());
    assert_eq!(fire_body(&mgr.fire(&w.cell, at).unwrap()).energy, 35);
}

#[test]
fn the_oldest_fire_makes_room_for_a_new_one() {
    let mut w = World::new();
    for x in 0..16 {
        for y in 0..8 {
            w.put(x, y, false, &["carpet"]);
        }
    }
    let mut mgr = IsoFireManager::new();
    let mut r = Script::new(&[]);
    for i in 0..MAX_FIRE_OBJECTS as i32 {
        let at = mgr
            .start_fire(
                &mut w.fw(&mut NoFireHooks),
                &mut r,
                i % 16,
                i / 16,
                0,
                false,
                50,
                0,
            )
            .unwrap();
        fire_body_mut(&IsoFireManager::fire_mut(&mut w.cell, at).unwrap()).age =
            if i == 7 { 30 } else { i % 5 };
    }
    let oldest = mgr.fire_stack[7];
    mgr.start_fire(&mut w.fw(&mut NoFireHooks), &mut r, 15, 7, 0, false, 50, 0)
        .unwrap();
    assert_eq!(mgr.fire_stack.len(), MAX_FIRE_OBJECTS);
    assert!(!mgr.fire_stack.contains(&oldest));
    assert!(mgr.fire(&w.cell, oldest).is_none());
    assert!(!w.sq(7, 0).borrow().has(w.flags.burning));
}

struct Zombie {
    on_fire: bool,
    at: (i32, i32, i32),
    damage: f32,
    attached: Option<(i32, i32, Option<i32>)>,
}

impl FireCharacter for Zombie {
    fn id(&self) -> u64 {
        7
    }
    fn kind(&self) -> CharacterKind {
        CharacterKind::Zombie
    }
    fn square(&self) -> Option<(i32, i32, i32)> {
        Some(self.at)
    }
    fn on_fire(&self) -> bool {
        self.on_fire
    }
    fn set_on_fire_flag(&mut self, on: bool) {
        self.on_fire = on;
    }
    fn is_dead(&self) -> bool {
        self.damage > 1.0
    }
    fn attach_fire(&mut self, dx: i32, dy: i32, part: Option<i32>) {
        self.attached = Some((dx, dy, part));
    }
    fn burn_part_range(&self) -> (i32, i32) {
        (0, 17)
    }
    fn apply_damage(&mut self, d: f32) {
        self.damage += d;
    }
    fn burn_player(&mut self, _: f32) {}
}

#[test]
fn zombies_catch_fire_take_damage_and_spread_it() {
    let mut w = World::new();
    w.put(2, 2, false, &["carpet"]);
    w.put(3, 2, false, &["carpet"]);
    let mut mgr = IsoFireManager::new();
    let mut r = Script::new(&[]);
    mgr.start_fire(&mut w.fw(&mut NoFireHooks), &mut r, 2, 2, 0, false, 50, 0);
    let mut z = Zombie {
        on_fire: false,
        at: (2, 2, 0),
        damage: 0.0,
        attached: None,
    };
    let frame = CharacterFireFrame {
        multiplier: 2.0,
        thirty_fps: 1.0,
        lock_fps: 60,
    };
    let mut r = Script::new(&[3, 4, 9]);
    fire_check(&mut mgr, &w.fw(&mut NoFireHooks), &mut z, frame, &mut r);
    assert!(z.on_fire);
    assert_eq!(r.calls, ["next(16)", "next(20)", "next(17)"]);
    assert_eq!(z.attached, Some((5, 14, None)));
    assert_eq!(mgr.burning_characters, [7]);
    reduce_health_when_burning(&mut mgr, &mut z, frame, &mut r);
    assert_eq!(z.damage, 0.0038 / 20.0 * 2.0);
    // On a square that is not burning, a roll in AdjustForFramerate(3000)
    // (6000 at 60 fps) under the spread probability lights it.
    z.at = (3, 2, 0);
    let mut r = Script::new(&[5]);
    spread_fire(&mut mgr, &mut w.fw(&mut NoFireHooks), &z, frame, &mut r);
    assert_eq!(r.calls[0], "next(6000)");
    assert!(w.sq(3, 2).borrow().has(w.flags.burning));
}

#[test]
fn a_fireplace_burns_a_minute_of_fuel_per_minute_then_smoulders() {
    let mut fp = IsoFireplace {
        fuel_amount_minutes: 30,
        ..Default::default()
    };
    assert!(fp.turn_on());
    fp.update_fuel(10.0);
    assert_eq!(fp.last_update_time, 10.0);
    // 20.5 minutes: 20 burnt, half a minute carried.
    fp.update_fuel(10.0 + 20.5 / 60.0);
    assert_eq!(fp.fuel_amount(), 10);
    assert!((fp.minute_accumulator - 0.5).abs() < 1e-3);
    assert_eq!(fp.calc_light_radius(), 2);
    fp.extinguish();
    assert_eq!(fp.minutes_since_extinguished, 0);
    // Smouldering: four minutes later, four used.
    fp.update_fuel(10.0 + 24.5 / 60.0);
    assert!(fp.smouldering);
    assert_eq!((fp.fuel_amount(), fp.minutes_since_extinguished), (6, 4));
    // It stops after ten minutes, having used at most ten.
    fp.update_fuel(10.0 + 40.5 / 60.0);
    assert_eq!((fp.fuel_amount(), fp.minutes_since_extinguished), (0, -1));
    assert!(!fp.smouldering && !fp.lit);
    // Lit with fuel running out: put out without smouldering.
    fp.add_fuel(3);
    fp.turn_on();
    fp.update_fuel(11.0);
    assert!(!fp.lit);
    assert_eq!(fp.minutes_since_extinguished, -1);
}

#[test]
fn burning_takes_down_the_sheet_rope_of_a_window_frame() {
    let mut w = World::new();
    w.put(2, 2, true, &["carpet", "rope_top"]);
    w.add(2, 2, "frame", ObjectKind::WindowFrame { north: true });
    assert!(w.sq(2, 2).borrow().has(w.flags.climbSheetTop[0].unwrap()));
    let mut mgr = IsoFireManager::new();
    burn::burn(
        &mut mgr,
        &mut w.fw(&mut NoFireHooks),
        &mut Script::new(&[]),
        (2, 2, 0),
    )
    .unwrap();
    // The rope tile is gone (the chunk's lowest level stops the descent),
    // the frame stays and the square no longer has the rope flag.
    assert_eq!(w.sprite_names(2, 2), ["", "frame"]);
    assert!(!w.sq(2, 2).borrow().has(w.flags.climbSheetTop[0].unwrap()));
    assert!(!w.sq(2, 2).borrow().have_sheet_rope);
}

#[test]
fn a_burning_hutch_leaves_ashes_on_its_squares() {
    use crate::objects::{HutchMaster, IsoHutch};
    let mut w = World::new();
    w.put(2, 2, false, &["grass"]);
    w.put(3, 2, false, &["grass"]);
    let master = IsoHutch {
        linked_x: 0,
        linked_y: 0,
        linked_z: 0,
        master: Some(HutchMaster {
            sprite_name: "hutch".into(),
            open: false,
            open_egg_hatch: false,
            saved_x: 2,
            saved_y: 2,
            saved_z: 0,
            animals: Vec::new(),
            hutch_dirt: 0.0,
            nest_box_dirt: 0.0,
            nest_boxes: Vec::new(),
        }),
    };
    w.add(2, 2, "hutch", ObjectKind::Hutch(Box::new(master)));
    let slave = IsoHutch {
        linked_x: 2,
        linked_y: 2,
        linked_z: 0,
        master: None,
    };
    w.add(3, 2, "hutch", ObjectKind::Hutch(Box::new(slave)));
    #[derive(Default)]
    struct H(Vec<(i32, i32)>);
    impl sound::FireSoundHooks for H {}
    impl FireHooks for H {
        fn hutch_release_contents(&mut self, r: crate::square_mutation::ObjectRef) {
            self.0.push((r.pos.0, r.pos.1));
        }
    }
    let mut h = H::default();
    let mut mgr = IsoFireManager::new();
    let mut r = Script::new(&[]);
    burn::burn(&mut mgr, &mut w.fw(&mut h), &mut r, (2, 2, 0)).unwrap();
    // The slave square's hutch is released and removed; both squares get
    // ashes (Rand.Next(1, 2) each) and burn.
    assert_eq!(h.0, [(3, 2)]);
    assert_eq!(&r.calls[..2], ["next(1)", "next(1)"]);
    let ash = |x| {
        w.sq(x, 2).borrow()
            .objects
            .iter().map(ObjectSlot::borrow)
            .any(|o| matches!(&o.sprite_name, Some(crate::objects::SpriteName::Str(n)) if n == "floors_burnt_01_1"))
    };
    assert!(ash(2) && ash(3));
    assert!(!w
        .sq(3, 2)
        .borrow()
        .objects
        .iter()
        .map(ObjectSlot::borrow)
        .any(|o| matches!(o.kind, ObjectKind::Hutch(_))));
    assert!(w.sq(3, 2).borrow().burnt_out);
}

#[test]
fn fire_sound_slots_go_to_the_nearest_fires_and_stay() {
    use sound::{FireSoundEvent, FireSoundHooks, FireSoundInfo, FireSounds, FIRE_SOUND_SLOTS};
    #[derive(Default)]
    struct Em(Vec<String>);
    impl FireSoundHooks for Em {
        fn fire_slot_play(
            &mut self,
            slot: usize,
            pos: (f32, f32, i32),
            size: i32,
            ev: Option<FireSoundEvent>,
        ) {
            self.0.push(format!(
                "play {slot} {},{} {size} {:?}",
                pos.0,
                pos.1,
                ev.map(|e| e.name())
            ));
        }
    }
    let r = |x: i32| FireRef {
        x,
        y: 0,
        z: 0,
        id: x as u32,
    };
    let info = |r: FireRef| FireSoundInfo {
        on_square: true,
        smoke: r.x == 2,
        life_stage: if r.x == 3 { 5 } else { 3 },
        campfire: r.x == 4,
        // Nearer for larger x.
        listener_distance_sq: 100.0 - r.x as f32,
    };
    let mut s = FireSounds::default();
    let mut em = Em::default();
    for x in 0..30 {
        s.add_fire(r(x));
    }
    s.update(&mut em, info);
    // The 20 nearest (29 down to 10) get slots in that order.
    assert_eq!(s.slots[0].fire, Some(r(29)));
    assert_eq!(s.slots[FIRE_SOUND_SLOTS - 1].fire, Some(r(10)));
    assert_eq!(em.0[0], "play 0 29.5,0.5 2 Some(\"Fire\")");
    assert!(s.fires.is_empty());
    // Next frame only three fires ask; 10 keeps its slot, smoke plays no
    // event, a stage-5 fire is not played, a campfire runs its own sound.
    em.0.clear();
    for x in [2, 3, 4, 10] {
        s.add_fire(r(x));
    }
    s.update(&mut em, info);
    assert_eq!(
        em.0,
        [
            "play 19 10.5,0.5 2 Some(\"Fire\")",
            "play 0 4.5,0.5 2 Some(\"CampfireRunning\")",
            "play 1 2.5,0.5 2 None",
        ]
    );
    assert_eq!(s.slots.iter().filter(|s| s.fire.is_some()).count(), 3);
}

#[test]
fn an_animal_rolls_like_a_player_before_catching_fire() {
    struct Animal(Zombie);
    impl FireCharacter for Animal {
        fn id(&self) -> u64 {
            8
        }
        fn kind(&self) -> CharacterKind {
            CharacterKind::Animal
        }
        fn square(&self) -> Option<(i32, i32, i32)> {
            Some(self.0.at)
        }
        fn on_fire(&self) -> bool {
            self.0.on_fire
        }
        fn set_on_fire_flag(&mut self, on: bool) {
            self.0.on_fire = on;
        }
        fn is_dead(&self) -> bool {
            false
        }
        fn attach_fire(&mut self, dx: i32, dy: i32, part: Option<i32>) {
            self.0.attached = Some((dx, dy, part));
        }
        fn burn_part_range(&self) -> (i32, i32) {
            (0, 17)
        }
        fn apply_damage(&mut self, _: f32) {}
        fn burn_player(&mut self, _: f32) {}
    }
    let mut w = World::new();
    w.put(2, 2, false, &["carpet"]);
    let mut mgr = IsoFireManager::new();
    mgr.start_fire(
        &mut w.fw(&mut NoFireHooks),
        &mut Script::new(&[]),
        2,
        2,
        0,
        false,
        50,
        0,
    );
    let mut a = Animal(Zombie {
        on_fire: false,
        at: (2, 2, 0),
        damage: 0.0,
        attached: None,
    });
    let frame = CharacterFireFrame {
        multiplier: 1.0,
        thirty_fps: 1.0,
        lock_fps: 30,
    };
    let mut r = Script::new(&[5, 3, 4, 9]);
    fire_check(&mut mgr, &w.fw(&mut NoFireHooks), &mut a, frame, &mut r);
    assert!(a.0.on_fire);
    assert_eq!(r.calls, ["next(70)", "next(16)", "next(20)", "next(17)"]);
    // A burnt body part like a player's.
    assert_eq!(a.0.attached, Some((5, 14, Some(9))));
    assert!(mgr.characters_on_fire.is_empty());
}

#[test]
fn a_permanent_fire_keeps_a_heat_source_at_its_light_radius() {
    let mut f = IsoFire::permanent();
    f.light_radius = 6;
    assert_eq!(f.heat_source, None);
    f.update(1.0, 1.0, 0, 1.0, &mut Script::new(&[]));
    assert_eq!(f.heat_source, Some(6));
    f.light_radius = 3;
    f.update(1.0, 1.0, 0, 1.0, &mut Script::new(&[]));
    assert_eq!(f.heat_source, Some(3));
    let mut fp = IsoFireplace {
        fuel_amount_minutes: 30,
        ..Default::default()
    };
    assert_eq!(fp.heat_source(), None);
    assert_eq!(fp.running_sound(None), None);
    fp.turn_on();
    assert_eq!(fp.heat_source(), Some((4, 35)));
    assert_eq!(fp.running_sound(None), Some("FireplaceRunning"));
    assert_eq!(fp.running_sound(Some("StoveRunning")), Some("StoveRunning"));
}

#[test]
fn fire_fighting_finds_the_fire_and_the_extinguisher() {
    use fighting::*;
    let mut w = World::new();
    w.put(2, 2, false, &["carpet"]);
    w.put(3, 3, false, &["carpet"]);
    let mut mgr = IsoFireManager::new();
    mgr.start_fire(
        &mut w.fw(&mut NoFireHooks),
        &mut Script::new(&[]),
        3,
        3,
        0,
        false,
        50,
        0,
    );
    let none = |_: (i32, i32, i32)| false;
    let b = w.flags.burning;
    assert_eq!(
        get_square_to_extinguish(&w.cell, b, (2, 2, 0), &none),
        Some((3, 3, 0))
    );
    let on_fire = |p: (i32, i32, i32)| p == (2, 2, 0);
    assert_eq!(
        get_square_to_extinguish(&w.cell, b, (2, 2, 0), &on_fire),
        Some((2, 2, 0))
    );
    struct It(&'static str, i32, bool, Option<Option<(bool, f32)>>);
    impl ExtinguisherItem for It {
        fn item_type(&self) -> &str {
            self.0
        }
        fn current_uses(&self) -> i32 {
            self.1
        }
        fn is_water_source(&self) -> bool {
            self.2
        }
        fn is_drainable(&self) -> bool {
            false
        }
        fn fluid(&self) -> Option<Option<(bool, f32)>> {
            self.3
        }
    }
    let bottle = It("WaterBottle", 0, true, Some(Some((true, 1.05))));
    let small = It("WaterBottle", 0, true, Some(Some((true, 0.5))));
    let bag = It("Sandbag", 1, false, None);
    let ext = It("Extinguisher", 3, false, None);
    assert_eq!(water_uses_integer(Some(&bottle)), 10);
    assert!(is_extinguisher(Some(&bottle)) && !is_extinguisher(Some(&small)));
    let inv: [&dyn ExtinguisherItem; 3] = [&bottle, &bag, &ext];
    assert_eq!(get_extinguisher(None, Some(&small), &inv), Some(4));
    assert_eq!(get_extinguisher(None, None, &inv[..2]), Some(3));
    assert_eq!(get_extinguisher(Some(&bottle), None, &[]), Some(0));
}

#[test]
fn a_window_outside_rooms_with_no_square_inside_throws_like_the_original() {
    use crate::objects::IsoWindow;
    let mut w = World::new();
    w.put(2, 2, false, &["carpet"]);
    let win = IsoWindow {
        north: true,
        locked: false,
        perma_locked: false,
        open: false,
        destroyed: false,
        glass_removed: false,
        health: 75,
        max_health: 75,
        open_sprite: None,
        closed_sprite: None,
        smashed_sprite: None,
        glass_removed_sprite: None,
    };
    w.add(2, 2, "frame", ObjectKind::Window(win));
    let mut mgr = IsoFireManager::new();
    // removeSheet reads the square north of it (2,1), which is null: the
    // NPE leaves BurnWalls before anything burns.
    let e = burn::burn(
        &mut mgr,
        &mut w.fw(&mut NoFireHooks),
        &mut Script::new(&[]),
        (2, 2, 0),
    );
    assert_eq!(e.unwrap_err(), burn::REMOVE_SHEET_NPE);
    assert!(!w.sq(2, 2).borrow().burnt_out);
    assert_eq!(w.sprite_names(2, 2), ["carpet", "frame"]);
    // With the square there (no curtain), the window burns.
    w.put(2, 1, false, &["carpet"]);
    burn::burn(
        &mut mgr,
        &mut w.fw(&mut NoFireHooks),
        &mut Script::new(&[]),
        (2, 2, 0),
    )
    .unwrap();
    assert!(w.sq(2, 2).borrow().burnt_out);
}

/// Square (2,2) with a carpet, part 1 of a player-built garage door and
/// `extra`; part 2 on (3,2).
fn garage_door_world(extra: &[&str]) -> World {
    use crate::objects::IsoThumpable;
    let mut w = World::new();
    let mut objs = vec!["carpet"];
    objs.extend_from_slice(extra);
    w.put(2, 2, false, &["carpet"]);
    w.put(3, 2, false, &["carpet"]);
    let part = |w: &World, s: &str| {
        ObjectKind::Thumpable(Box::new(IsoThumpable {
            north: true,
            is_door: true,
            closed_sprite: Some(w.m.named_map[s]),
            ..Default::default()
        }))
    };
    let k = part(&w, "gdoor1");
    w.add(2, 2, "gdoor1", k);
    let k = part(&w, "gdoor2");
    w.add(3, 2, "gdoor2", k);
    for e in extra {
        let sq_owner = w.cell.get_grid_square_mut(2, 2, 0).unwrap();
        let mut sq = sq_owner.borrow_mut();
        let id = sq.new_object_id();
        sq.objects
            .push((IsoObject::new(id, ObjectKind::Object, Some(w.m.named_map[*e]))).into());
    }
    w
}

#[test]
fn burnt_objects_are_replaced_at_the_loop_index_after_a_recursive_burn_shifts_the_list() {
    // BurnWalls takes part 1 (index 1), burns part 2 first
    // (safelyBurnTileObjectFromSquare); part 2 has no burntTile and is
    // attached, so RemoveTileObject safely removes the whole door, part 1
    // too. Part 1 is an IsoThumpable with a burntTile: removeFromWorld and
    // objects.set(1, new object) -- index 1 now holds the table, which is
    // replaced instead and never burns.
    let mut w = garage_door_world(&["table"]);
    assert_eq!(w.sprite_names(2, 2), ["carpet", "gdoor1", "table"]);
    let mut mgr = IsoFireManager::new();
    burn::burn(
        &mut mgr,
        &mut w.fw(&mut NoFireHooks),
        &mut Script::new(&[]),
        (2, 2, 0),
    )
    .unwrap();
    // ("" is floors_burnt_01_0, a sprite these tables do not have.)
    assert_eq!(w.sprite_names(2, 2), ["", "walls_burnt_01_0"]);
    assert_eq!(w.sprite_names(3, 2), [""]);
    assert!(w.sq(2, 2).borrow().special_objects.is_empty());
    assert!(w.sq(2, 2).borrow().burnt_out && w.sq(3, 2).borrow().burnt_out);
    // Without the table, index 1 is past the end: PZArrayList.set throws
    // and BurnWalls stops before the square is marked burnt out.
    let mut w = garage_door_world(&[]);
    let e = burn::burn(
        &mut mgr,
        &mut w.fw(&mut NoFireHooks),
        &mut Script::new(&[]),
        (2, 2, 0),
    );
    assert_eq!(
        e.unwrap_err(),
        "java.lang.IndexOutOfBoundsException: Index: 1 Size: 1"
    );
    assert_eq!(w.sprite_names(2, 2), [""]);
    assert!(!w.sq(2, 2).borrow().burnt_out && w.sq(3, 2).borrow().burnt_out);
}
