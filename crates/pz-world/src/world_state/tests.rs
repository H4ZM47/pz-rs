//! Barricades, thumpables, curtains, power and torches on hand-built
//! squares (made-up values).

use std::rc::Rc;

use super::*;
use crate::defs::BuildingIdx;
use crate::los::{line_clear, TestResults};
use crate::objects::{IsoDoor, IsoFire, IsoWindow};
use crate::sprite::{test_tables, PropertyValueMap};
use crate::square::{NoSquareHooks, VisionEnv};
use crate::IsoChunk;
use pz_inventory::item::ItemKind;
use pz_inventory::template::{ItemClass, ItemTemplate};

const SPRITES: &[(&str, &[&str])] = &[
    ("floor", &["solidfloor"]),
    ("door_n", &["doorN", "collideN"]),
    ("window_w", &["windowW"]),
    ("thump_wall", &["collideN"]),
    ("thump_window", &["WindowN"]),
    ("switch", &[]),
    ("hop_n", &["HoppableN"]),
    ("door_trans", &["doorTrans"]),
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
        "doorTrans",
    ] {
        map.entry(k);
    }
    for (_, props) in SPRITES {
        for k in *props {
            let e = map.entry(k);
            if e.is_empty() {
                e.push(String::new());
            }
        }
    }
    let mut m = IsoSpriteManager::new(test_tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (i, (name, props)) in SPRITES.iter().enumerate() {
        let s = m.add_sprite(name, 300 + i as i32);
        let (tables, alias) = (m.tables.clone(), m.alias.clone());
        let sp = &mut m.sprites[s];
        sp.name = Some(name.to_string());
        for k in *props {
            sp.properties.set(&tables, &alias, k, "", true);
        }
    }
    m
}

/// A cell with chunks (1, 1) and (2, 1) (squares x 8..24, y 8..16).
struct World {
    cell: IsoCell,
    m: IsoSpriteManager,
    meta: IsoMetaGrid,
}

fn power() -> PowerSettings {
    PowerSettings {
        hydro_power_on: true,
        world_age_days: 3.0,
        elec_shut_modifier: 9,
        allow_exterior_generator: true,
        generator_tile_range: 5,
        generator_vertical_power_range: 1,
    }
}

impl World {
    fn new() -> World {
        let mut cell = IsoCell::default();
        cell.chunks.insert((1, 1), IsoChunk::new(1, 1));
        cell.chunks.insert((2, 1), IsoChunk::new(2, 1));
        World {
            cell,
            m: manager(),
            meta: IsoMetaGrid::empty(),
        }
    }

    /// An indoor (roofed) floor square, or an exterior one.
    fn put(&mut self, x: i32, y: i32, z: i32, indoor: bool) {
        let mut sq = IsoGridSquare::new(x, y, z);
        let id = sq.new_object_id();
        sq.objects.push(IsoObject::new(
            id,
            ObjectKind::Object,
            Some(self.m.named_map["floor"]),
        ));
        sq.have_roof = indoor;
        sq.recalc_properties(&self.m);
        let ch = self
            .cell
            .chunks
            .get_mut(&(x.div_euclid(8), y.div_euclid(8)))
            .unwrap();
        ch.set_square(x.rem_euclid(8), y.rem_euclid(8), z, Some(sq));
    }

    fn add(&mut self, x: i32, y: i32, z: i32, sprite: &str, kind: ObjectKind) {
        let sq = self.cell.get_grid_square_mut(x, y, z).unwrap();
        let id = sq.new_object_id();
        sq.objects
            .push(IsoObject::new(id, kind, Some(self.m.named_map[sprite])));
        sq.special_objects.push(id);
    }

    fn barricade(&mut self, x: i32, y: i32, z: i32, dir: IsoDirections, planks: &[i32]) {
        let mut plank_health = [0; 4];
        plank_health[..planks.len()].copy_from_slice(planks);
        let b = IsoBarricade {
            dir: dir as i8,
            plank_health,
            metal_health: 0,
            metal_bar_health: 0,
        };
        self.add(x, y, z, "floor", ObjectKind::Barricade(b));
    }

    fn sq(&self, x: i32, y: i32, z: i32) -> &IsoGridSquare {
        self.cell.get_grid_square(x, y, z).unwrap()
    }

    fn state(&self) -> WorldState<'_> {
        WorldState::new(&self.cell, &self.meta, &self.m, power())
    }
}

fn door(north: bool, open: bool) -> ObjectKind {
    ObjectKind::Door(IsoDoor {
        north,
        open,
        ..Default::default()
    })
}

#[test]
fn barricade_values() {
    let mut b = IsoBarricade {
        dir: 6,
        plank_health: [700, 0, 30, 0],
        metal_health: 0,
        metal_bar_health: 0,
    };
    assert_eq!(b.forward_dir(), IsoDirections::E);
    assert_eq!(b.num_planks(), 2);
    assert!(!b.is_block_vision() && !b.is_destroyed() && b.can_add_plank());
    assert_eq!(b.light_transmission(), 1.0 - 2.0 * 0.15);
    b.plank_health = [5, 5, 5, 0];
    assert!(b.is_block_vision());
    b.plank_health = [0; 4];
    assert!(b.is_destroyed());
    assert_eq!(b.light_transmission(), 1.0);
    b.metal_bar_health = 12;
    assert!(b.is_metal_bar() && !b.is_block_vision() && !b.can_add_plank());
    assert_eq!(b.light_transmission(), 1.0);
    b.metal_health = 40;
    assert!(b.is_metal() && b.is_block_vision());
    assert_eq!(b.light_transmission(), 0.0);
}

#[test]
fn barricades_on_both_sides() {
    let mut w = World::new();
    for (x, y) in [(10, 10), (10, 11), (11, 11), (16, 12), (15, 12)] {
        w.put(x, y, 0, true);
    }
    // A north door on (10, 11): its own side faces N, the opposite square
    // (10, 10) holds the S side.
    w.add(10, 11, 0, "door_n", door(true, true));
    w.barricade(10, 10, 0, IsoDirections::S, &[300, 300]);
    // A barricade facing the wrong way on the door's own square.
    w.barricade(10, 11, 0, IsoDirections::W, &[300]);
    // A west window on (16, 12) across the chunk edge: the opposite square
    // is (15, 12) in chunk (1, 1).
    w.add(
        16,
        12,
        0,
        "window_w",
        ObjectKind::Window(IsoWindow::default()),
    );
    w.barricade(16, 12, 0, IsoDirections::W, &[1000, 1000, 1000]);
    w.barricade(15, 12, 0, IsoDirections::E, &[0, 0, 0, 900]);
    let ws = w.state();

    let sq = w.sq(10, 11, 0);
    let (same, opp) = ws.barricades_for(sq, true);
    assert!(same.is_none());
    assert_eq!(opp.unwrap().num_planks(), 2);
    assert!(ws.object_barricaded(sq, &sq.objects[1]));
    // The open, barricaded door blocks the way north.
    let north = w.sq(10, 10, 0);
    assert!(sq.is_door_blocked_to(Some(north), &w.cell, &ws));
    assert!(!sq.is_door_blocked_to(Some(north), &w.cell, &NoSquareHooks));

    let sq = w.sq(16, 12, 0);
    let (same, opp) = ws.barricades_for(sq, false);
    assert_eq!(same.unwrap().num_planks(), 3);
    assert_eq!(opp.unwrap().num_planks(), 1);
    // The floor object is not barricade-able.
    assert!(!ws.object_barricaded(sq, &sq.objects[0]));
    assert!(ws.object_barricaded(sq, &sq.objects[1]));
}

#[test]
fn thumpable_doors_and_windows() {
    let th = |is_door: bool, open: bool| {
        ObjectKind::Thumpable(Box::new(IsoThumpable {
            north: true,
            is_door,
            open,
            ..Default::default()
        }))
    };
    let mut w = World::new();
    w.put(12, 12, 0, true);
    w.put(12, 11, 0, true);
    w.put(13, 12, 0, true);
    w.add(12, 12, 0, "thump_wall", th(true, true));
    w.add(13, 12, 0, "thump_window", th(false, false));
    let ws = w.state();
    let sq = w.sq(12, 12, 0);
    // Open and not barricaded: not blocked.
    assert!(!ws.thumpable_door_blocked(sq, EdgeFacing::NorthSouth));
    assert!(!sq.is_door_blocked_to(Some(w.sq(12, 11, 0)), &w.cell, &ws));
    // A thumpable that is not a door is never a blocked door; only the
    // window sprite has the window flag.
    let sq2 = w.sq(13, 12, 0);
    assert!(!ws.thumpable_door_blocked(sq2, EdgeFacing::NorthSouth));
    assert!(ws.object_is_window(&sq2.objects[1]));
    assert!(!ws.object_is_window(&sq.objects[1]));

    // Closed: blocked on its own edge only.
    let mut w2 = World::new();
    w2.put(12, 12, 0, true);
    w2.put(12, 11, 0, true);
    w2.add(12, 12, 0, "thump_wall", th(true, false));
    let ws2 = w2.state();
    let sq = w2.sq(12, 12, 0);
    assert!(ws2.thumpable_door_blocked(sq, EdgeFacing::NorthSouth));
    assert!(!ws2.thumpable_door_blocked(sq, EdgeFacing::EastWest));
    assert!(sq.is_door_blocked_to(Some(w2.sq(12, 11, 0)), &w2.cell, &ws2));

    // Open but barricaded from the other side: blocked.
    let mut w3 = World::new();
    w3.put(12, 12, 0, true);
    w3.put(12, 11, 0, true);
    w3.add(12, 12, 0, "thump_wall", th(true, true));
    w3.barricade(12, 11, 0, IsoDirections::S, &[50]);
    let ws3 = w3.state();
    assert!(ws3.thumpable_door_blocked(w3.sq(12, 12, 0), EdgeFacing::NorthSouth));
}

#[test]
fn door_curtains() {
    let mut w = World::new();
    w.put(9, 9, 0, true);
    w.add(9, 9, 0, "door_n", door(true, false));
    w.add(
        9,
        9,
        0,
        "door_n",
        ObjectKind::Door(IsoDoor {
            has_curtain: true,
            curtain_open: true,
            ..Default::default()
        }),
    );
    let ws = w.state();
    let sq = w.sq(9, 9, 0);
    assert_eq!(ws.door_curtain(&sq.objects[0]), None);
    assert_eq!(ws.door_curtain(&sq.objects[1]), None);
    assert_eq!(ws.door_curtain(&sq.objects[2]), Some(true));
}

#[test]
fn generator_range_and_grid_power() {
    let mut w = World::new();
    w.put(12, 12, 0, true);
    w.put(16, 12, 0, true);
    w.put(17, 12, 0, true);
    w.put(12, 12, 1, true);
    w.put(12, 12, 2, true);
    w.put(13, 13, 0, false);
    // A generator at (12, 12, 0) is listed in both chunks it reaches.
    for c in [(1, 1), (2, 1)] {
        w.cell.chunks.get_mut(&c).unwrap().saved.generators = Some(vec![(12, 12, 0)]);
    }
    let ws = w.state();
    assert!(ws.have_electricity(w.sq(12, 12, 0)));
    // 4 tiles away: inside the 5-tile range; 5 tiles: on the edge.
    assert!(ws.have_electricity(w.sq(16, 12, 0)));
    assert!(ws.have_electricity(w.sq(17, 12, 0)));
    // One level up is in the vertical range, two is not.
    assert!(ws.have_electricity(w.sq(12, 12, 1)));
    assert!(!ws.have_electricity(w.sq(12, 12, 2)));
    // Exterior squares only when the sandbox allows it.
    assert!(ws.have_electricity(w.sq(13, 13, 0)));
    let mut p = power();
    p.allow_exterior_generator = false;
    p.generator_tile_range = 4;
    let ws = WorldState::new(&w.cell, &w.meta, &w.m, p);
    assert!(!ws.have_electricity(w.sq(13, 13, 0)));
    assert!(ws.have_electricity(w.sq(16, 12, 0)));
    assert!(!ws.have_electricity(w.sq(17, 12, 0)));

    // The grid: on until the shut-off day, never in a NoPower zone.
    let mut p = power();
    assert!(p.does_power_grid_exist(0));
    let ws = WorldState::new(&w.cell, &w.meta, &w.m, p);
    assert!(ws.has_grid_power(w.sq(12, 12, 0)));
    let zone = |x: i32, _y: i32| x == 12;
    let ws = WorldState {
        no_power_zone: &zone,
        ..WorldState::new(&w.cell, &w.meta, &w.m, p)
    };
    assert!(!ws.has_grid_power(w.sq(12, 12, 0)));
    assert!(ws.has_grid_power(w.sq(16, 12, 0)));
    p.world_age_days = 9.5;
    assert!(!p.does_power_grid_exist(0) && p.does_power_grid_exist(1));
}

#[test]
fn light_switch_power() {
    let mut w = World::new();
    w.put(10, 10, 0, true);
    w.put(11, 10, 0, true);
    w.put(14, 14, 0, true);
    w.put(13, 13, 0, true);
    // (10, 10) is part of a building; the others are not.
    w.cell
        .get_grid_square_mut(10, 10, 0)
        .unwrap()
        .roof_hide_building = Some(BuildingIdx(0));
    let lamp = IsoLightSwitch {
        bulb_item: Some("Base.TestBulb".into()),
        ..Default::default()
    };
    let street = IsoLightSwitch {
        street_light: true,
        ..lamp.clone()
    };
    w.add(14, 14, 0, "switch", ObjectKind::LightSwitch(lamp.clone()));
    w.add(11, 10, 0, "switch", ObjectKind::LightSwitch(lamp.clone()));
    w.add(13, 13, 0, "switch", ObjectKind::LightSwitch(street.clone()));
    let ws = w.state();
    // Grid on: a switch next to a building square has power, one away from
    // buildings does not, a street light does.
    assert!(ws.lamp_switch_has_power(11, 10, 0));
    assert!(!ws.lamp_switch_has_power(14, 14, 0));
    assert!(ws.lamp_switch_has_power(13, 13, 0));
    // No bulb: never.
    let no_bulb = IsoLightSwitch {
        bulb_item: None,
        ..lamp.clone()
    };
    assert!(!ws.can_switch_light(w.sq(11, 10, 0), &no_bulb));

    // Grid off: only a generator in reach of the switch or a neighbour.
    let mut p = power();
    p.world_age_days = 20.0;
    let ws = WorldState::new(&w.cell, &w.meta, &w.m, p);
    assert!(!ws.lamp_switch_has_power(11, 10, 0));
    assert!(!ws.room_switch_square_powered(14, 14, 0));
    w.cell.chunks.get_mut(&(1, 1)).unwrap().saved.generators = Some(vec![(13, 13, 0)]);
    let ws = WorldState::new(&w.cell, &w.meta, &w.m, p);
    assert!(ws.lamp_switch_has_power(14, 14, 0));
    assert!(ws.room_switch_square_powered(14, 14, 0));
    // A street light can switch on the generator but needs the grid.
    assert!(ws.can_switch_light(w.sq(13, 13, 0), &street));
    assert!(!ws.lamp_switch_has_power(13, 13, 0));
    // A battery-powered switch ignores both.
    let battery = IsoLightSwitch {
        use_battery: true,
        has_battery: true,
        can_be_modified: true,
        power: 0.4,
        ..lamp.clone()
    };
    let ws = WorldState::new(&w.cell, &w.meta, &w.m, power());
    assert!(ws.can_switch_light(w.sq(14, 14, 0), &battery));
    let flat = IsoLightSwitch {
        power: 0.0,
        ..battery
    };
    assert!(!ws.can_switch_light(w.sq(11, 10, 0), &flat));
}

fn item(class: ItemClass, f: impl FnOnce(&mut ItemTemplate)) -> InventoryItem {
    let mut t = ItemTemplate::new(class, "Test", "Thing");
    f(&mut t);
    InventoryItem::create(Rc::new(t), 1)
}

#[test]
fn player_torches() {
    let torch = item(ItemClass::Normal, |t| {
        t.activated_item = true;
        t.light_strength = 1.25;
        t.torch_cone = true;
        t.light_distance = 12;
        t.torch_dot = 0.5;
    });
    let mut on = torch.clone();
    on.activated = true;
    let candle = item(ItemClass::Normal, |t| {
        t.light_strength = 0.25;
        t.light_distance = 3;
    });
    let mut empty = item(ItemClass::Drainable, |t| t.light_strength = 0.5);
    empty.uses = 0;
    let rock = item(ItemClass::Normal, |_| {});
    let p = PlayerLights {
        index: 1,
        x: 10.5,
        y: 11.5,
        z: 0.0,
        look_x: 0.0,
        look_y: -1.0,
        // The same lit torch twice (in a hand and attached) counts once.
        items: vec![&torch, &on, &rock, &empty, &candle, &on],
    };
    let t = p.torches();
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].id, 5);
    assert!(t[0].cone);
    assert_eq!((t[0].dist, t[0].strength, t[0].dot), (12.0, 1.25, 0.5));
    assert_eq!((t[0].angle_x, t[0].angle_y), (0.0, -1.0));
    assert_eq!(t[1].id, 6);
    assert!(!t[1].cone);
    assert_eq!(t[1].dist, 3.0);
    // At most four per player.
    let lit: Vec<InventoryItem> = (0..6).map(|_| candle.clone()).collect();
    let p = PlayerLights {
        index: 0,
        items: lit.iter().collect(),
        ..p
    };
    let t = p.torches();
    assert_eq!(t.iter().map(|t| t.id).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
}

#[test]
fn weapon_light_part() {
    let part = item(ItemClass::WeaponPart, |t| {
        t.activated_item = true;
        t.light_strength = 0.75;
        t.torch_cone = true;
        t.light_distance = 9;
        t.torch_dot = 0.25;
    });
    let mut gun = item(ItemClass::Weapon, |_| {});
    assert!(!gun.can_emit_light() && !gun.is_torch_cone());
    if let ItemKind::Weapon(w) = &mut gun.kind {
        w.attachments.push(part);
    }
    assert!(gun.can_emit_light() && gun.is_torch_cone() && gun.can_be_activated());
    assert_eq!(gun.get_light_distance(), 9);
    assert_eq!(gun.get_torch_dot(), 0.25);
    assert!(!gun.is_emitting_light());
    gun.activated = true;
    assert!(gun.is_emitting_light());
}

fn fire() -> ObjectKind {
    ObjectKind::Fire(IsoFire {
        life: 100,
        spread_delay: 0,
        life_stage: 0,
        life_stage_timer: 0,
        life_stage_duration: 0,
        energy: 0,
        num_flame_particles: 0,
        spread_timer: 0,
        age: 0,
        perm: false,
        light_radius: 0,
        smoke: false,
        saved_update_time: 0,
    })
}

fn thump(f: impl FnOnce(&mut IsoThumpable)) -> ObjectKind {
    let mut t = IsoThumpable::default();
    f(&mut t);
    ObjectKind::Thumpable(Box::new(t))
}

#[test]
fn special_objects_from_the_real_objects() {
    let mut w = World::new();
    w.put(10, 10, 0, true);
    w.add(
        10,
        10,
        0,
        "thump_wall",
        thump(|t| {
            t.north = true;
            t.open = true;
            t.is_door = true;
            t.can_pass_through = true;
        }),
    );
    // `isHoppable()` from the field, from the sprite, and from the closed
    // sprite of a closed door (which wins over the current sprite).
    w.add(10, 10, 0, "thump_wall", thump(|t| t.is_hoppable = true));
    w.add(10, 10, 0, "hop_n", thump(|_| {}));
    let closed = w.m.named_map["thump_wall"];
    w.add(
        10,
        10,
        0,
        "hop_n",
        thump(|t| {
            t.is_door = true;
            t.closed_sprite = Some(closed);
        }),
    );
    let hop = w.m.named_map["hop_n"];
    w.add(
        10,
        10,
        0,
        "thump_wall",
        thump(|t| {
            t.is_door = true;
            t.closed_sprite = Some(hop);
        }),
    );
    w.barricade(10, 10, 0, IsoDirections::S, &[1, 1, 1]);
    w.barricade(10, 10, 0, IsoDirections::E, &[1, 1]);
    w.add(10, 10, 0, "floor", fire());
    w.add(10, 10, 0, "door_n", door(true, false));
    let ws = w.state();
    let sq = w.sq(10, 10, 0);
    let got: Vec<_> = sq
        .objects
        .iter()
        .map(|o| SquareHooks::special_object(&ws, sq, o))
        .collect();
    let th = |north, open, is_door, is_hoppable, can_pass_through| {
        Some(SpecialObject::Thumpable(ThumpableInfo {
            north,
            open,
            is_door,
            is_hoppable,
            can_pass_through,
        }))
    };
    assert_eq!(
        got,
        vec![
            None,
            th(true, true, true, false, true),
            th(false, false, false, true, false),
            th(false, false, false, true, false),
            th(false, false, true, false, false),
            th(false, false, true, true, false),
            Some(SpecialObject::Barricade {
                dir: IsoDirections::S,
                blocks_vision: true
            }),
            Some(SpecialObject::Barricade {
                dir: IsoDirections::E,
                blocks_vision: false
            }),
            Some(SpecialObject::Fire),
            None,
        ]
    );
    assert_eq!(SquareHooks::door_curtain(&ws, sq, &sq.objects[9]), None);
}

/// A row of squares y = 10, x 8..=14, with `kind` on the west edge of
/// (11, 10), as seen by `LosUtil.lineClear` from (9, 10) to (13, 10).
fn line_across(sprite: &str, kind: ObjectKind, ignore_doors: bool) -> TestResults {
    let mut w = World::new();
    for x in 8..=14 {
        w.put(x, 10, 0, true);
    }
    w.add(11, 10, 0, sprite, kind);
    let ws = w.state();
    let env = VisionEnv::new(&w.cell, &w.m, &ws);
    line_clear(&env, 9, 10, 0, 13, 10, 0, ignore_doors)
}

#[test]
fn line_of_sight_through_special_objects() {
    use TestResults::*;
    let door_w = |open| {
        thump(|t| {
            t.is_door = true;
            t.open = open;
        })
    };
    // A closed thumpable door blocks unless doors are ignored; an open one
    // swings onto the other edge.
    assert_eq!(line_across("thump_wall", door_w(false), false), Blocked);
    assert_eq!(line_across("thump_wall", door_w(false), true), Clear);
    assert_eq!(line_across("thump_wall", door_w(true), false), Clear);
    // `doorTrans` lets the closed thumpable door be seen through.
    assert_eq!(
        line_across("door_trans", door_w(false), false),
        ClearThroughOpenDoor
    );
    // `canPassThrough` never affects vision.
    let pass = thump(|t| {
        t.is_door = true;
        t.can_pass_through = true;
    });
    assert_eq!(line_across("thump_wall", pass, false), Clear);
    // A thumpable that is a window by its sprite is seen through.
    assert_eq!(line_across("thump_window", thump(|_| {}), false), Clear);

    // A barricade facing west blocks with more than two planks, even when
    // doors are ignored.
    let barricade = |planks: [i32; 4], metal| {
        ObjectKind::Barricade(IsoBarricade {
            dir: IsoDirections::W as i8,
            plank_health: planks,
            metal_health: metal,
            metal_bar_health: 0,
        })
    };
    assert_eq!(
        line_across("floor", barricade([5, 5, 5, 0], 0), true),
        Blocked
    );
    assert_eq!(
        line_across("floor", barricade([5, 5, 0, 0], 0), false),
        Clear
    );
    assert_eq!(line_across("floor", barricade([0; 4], 30), false), Blocked);
    // Fire has no effect.
    assert_eq!(line_across("floor", fire(), false), Clear);

    // A closed `doorTrans` door is seen through unless its curtain is
    // drawn.
    let curtain = |curtain_open| {
        ObjectKind::Door(IsoDoor {
            has_curtain: true,
            curtain_open,
            ..Default::default()
        })
    };
    assert_eq!(
        line_across("door_trans", door(false, false), false),
        ClearThroughClosedDoor
    );
    assert_eq!(
        line_across("door_trans", curtain(true), false),
        ClearThroughClosedDoor
    );
    assert_eq!(line_across("door_trans", curtain(false), false), Blocked);
    assert_eq!(line_across("door_trans", curtain(false), true), Clear);
}

#[test]
fn adjacent_queries_see_thumpables_and_barricades() {
    let mut w = World::new();
    for (x, y) in [(10, 10), (11, 10), (10, 9), (13, 10), (13, 9)] {
        w.put(x, y, 0, true);
    }
    // A closed west thumpable door on (11, 10), a hoppable north thumpable
    // on (10, 10) and a barricaded north thumpable window on (13, 10).
    w.add(11, 10, 0, "thump_wall", thump(|t| t.is_door = true));
    w.add(
        10,
        10,
        0,
        "thump_wall",
        thump(|t| {
            t.north = true;
            t.is_hoppable = true;
        }),
    );
    w.add(13, 10, 0, "thump_window", thump(|t| t.north = true));
    w.barricade(13, 10, 0, IsoDirections::N, &[5, 5, 5]);
    let ws = w.state();
    let (m, g) = (&w.m, &w.cell);
    let (a, b) = (w.sq(10, 10, 0), w.sq(11, 10, 0));
    assert!(b.has_blocked_door(EdgeFacing::EastWest, &ws));
    assert!(!a.can_reach_to(b, m, g, &ws));
    assert!(a.can_reach_to(b, m, g, &NoSquareHooks));
    assert!(b
        .get_door_facing(EdgeFacing::EastWest, &ws)
        .is_some_and(|o| matches!(o.kind, ObjectKind::Thumpable(_))));
    assert!(b
        .get_door_facing(EdgeFacing::EastWest, &NoSquareHooks)
        .is_none());
    assert!(a
        .get_hoppable_thumpable_facing(EdgeFacing::NorthSouth, &ws)
        .is_some());
    assert!(a
        .get_hoppable_thumpable_facing(EdgeFacing::EastWest, &ws)
        .is_none());
    let c = w.sq(13, 10, 0);
    assert!(c
        .get_thumpable_window_facing(EdgeFacing::NorthSouth, m, &ws)
        .is_some());
    // The barricade blocks vision north from (13, 10).
    let env = VisionEnv::new(g, m, &ws);
    assert_eq!(
        c.test_vision_adjacent(0, -1, 0, false, false, &env),
        TestResults::Blocked
    );
    assert_eq!(
        w.sq(13, 9, 0)
            .test_vision_adjacent(0, 1, 0, false, true, &env),
        TestResults::Blocked
    );
    let env = VisionEnv::new(g, m, &NoSquareHooks);
    assert_eq!(
        c.test_vision_adjacent(0, -1, 0, false, false, &env),
        TestResults::Clear
    );
}

#[test]
fn object_hooks_without_world_state() {
    let mut w = World::new();
    w.put(12, 12, 0, true);
    w.put(12, 11, 0, true);
    w.add(
        12,
        12,
        0,
        "thump_wall",
        thump(|t| {
            t.north = true;
            t.is_door = true;
            t.open = true;
        }),
    );
    w.barricade(12, 11, 0, IsoDirections::S, &[50, 50, 50]);
    let oh = ObjectHooks::new(&w.cell, &w.m);
    let (sq, north) = (w.sq(12, 12, 0), w.sq(12, 11, 0));
    // Open, but barricaded from the opposite square.
    assert!(oh.is_barricaded(sq, &sq.objects[1]));
    assert!(oh.has_blocked_thumpable_door(sq, EdgeFacing::NorthSouth));
    assert!(!sq.can_reach_to(north, &w.m, &w.cell, &oh));
    assert_eq!(
        oh.special_object(north, &north.objects[1]),
        Some(SpecialObject::Barricade {
            dir: IsoDirections::S,
            blocks_vision: true
        })
    );
    // Through one chunk only, the opposite square (same chunk here) is
    // still found.
    let ch = &w.cell.chunks[&(1, 1)];
    let getter = crate::square::ChunkGetter(ch);
    let oh = ObjectHooks::new(&getter, &w.m);
    assert!(oh.is_barricaded(sq, &sq.objects[1]));
}
