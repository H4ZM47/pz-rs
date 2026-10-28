//! `DoTileObjectCreation` / `FixStackableObjects` on hand-authored sprites.

use crate::ObjectSlot;
use std::collections::VecDeque;

use super::*;
use crate::objects::{NoHooks, ObjectTables};
use crate::sprite::test_tables;
use crate::sprite::PropertyValueMap;

/// A hand-authored sprite: name, id, tile type name, properties (an empty
/// value sets the flag of that name).
type Spec<'a> = (&'a str, i32, &'a str, &'a [(&'a str, &'a str)]);

fn manager(specs: &[Spec]) -> IsoSpriteManager {
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
    for (_, _, _, props) in specs {
        for (k, v) in *props {
            let e = map.entry(k);
            if !e.iter().any(|x| x == v) {
                e.push(v.to_string());
            }
        }
    }
    let mut m = IsoSpriteManager::new(test_tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (name, id, ty, props) in specs {
        let s = m.add_sprite(name, *id);
        let t = m.tables.object_type_from_string(ty);
        let (tables, alias) = (m.tables.clone(), m.alias.clone());
        let sp = &mut m.sprites[s];
        sp.name = Some(name.to_string());
        sp.tile_type = t;
        for (k, v) in *props {
            // An empty value is a flag when the name is one.
            sp.properties.set(&tables, &alias, k, v, v.is_empty());
        }
        sp.cut_n = sp.properties.has(tables.flag.cutN);
        sp.cut_w = sp.properties.has(tables.flag.cutW);
        sp.solidfloor = sp.properties.has(tables.flag.solidfloor);
    }
    m.loaded_tile_definitions(&|_| false).unwrap();
    m
}

/// `Rand` that hands out scripted values and logs every call.
#[derive(Default)]
struct Script {
    ints: VecDeque<i32>,
    floats: VecDeque<f32>,
    log: Vec<String>,
}

impl GameRand for Script {
    fn next(&mut self, max: i32) -> i32 {
        self.log.push(format!("next({max})"));
        if max <= 0 {
            return 0;
        }
        self.ints.pop_front().unwrap_or(0)
    }
    fn next_float(&mut self) -> f32 {
        self.log.push("nextFloat".into());
        self.floats.pop_front().unwrap_or(0.0)
    }
}

fn tables() -> ObjectTables {
    ObjectTables::new(vec![2, 2, 3, 3, 4, 4, 5, 5], vec![(0, 0), (1, 9), (2, 7)]).unwrap()
}

struct World {
    m: IsoSpriteManager,
    rand: Script,
    tables: ObjectTables,
    settings: ObjectLoadSettings,
    sq: crate::SquareRef,
    meta: IsoMetaGrid,
}

impl World {
    fn new(specs: &[Spec]) -> Self {
        World {
            m: manager(specs),
            rand: Script::default(),
            tables: tables(),
            settings: ObjectLoadSettings::default(),
            sq: crate::SquareSlot::new(IsoGridSquare::new(10, 20, 0)).ref_handle(),
            meta: IsoMetaGrid::empty(),
        }
    }

    fn place_with(&mut self, name: &str, hooks: &mut dyn ObjectLoadHooks) -> JavaResult<()> {
        let spr = self.m.named_map[name];
        let ty = self.m.sprites[spr].tile_type;
        let mut env = ObjectEnv {
            blood_body_parts: 7,
            sprites: &mut self.m,
            rand: &mut self.rand,
            tables: &self.tables,
            settings: self.settings,
            hooks,
        };
        do_tile_object_creation(&mut env, &self.meta, &self.sq, spr, ty, name)
    }

    fn place(&mut self, name: &str) -> JavaResult<()> {
        self.place_with(name, &mut NoHooks)
    }

    fn classes(&self) -> Vec<&'static str> {
        self.sq
            .borrow()
            .objects
            .iter()
            .map(ObjectSlot::borrow)
            .map(|o| o.kind.class_name())
            .collect()
    }

    fn idx(&self, name: &str) -> SpriteIdx {
        self.m.named_map[name]
    }
}

#[test]
fn doors_lock_and_order_against_walls() {
    let mut w = World::new(&[
        ("door_0", 1, "doorN", &[]),
        ("door_2", 3, "doorN", &[("open", "")]),
        ("wall_0", 10, "wall", &[("cutN", "")]),
        ("gdoor_0", 20, "doorW", &[("GarageDoor", "2")]),
        ("gdoor_8", 28, "doorW", &[("GarageDoor", "5"), ("open", "")]),
    ]);
    w.rand.ints = [10, 0].into();
    w.place("door_0").unwrap();
    let ObjectKind::Door(d) = &w.sq.borrow().objects[0].borrow().kind.clone() else {
        panic!()
    };
    assert!(d.north && !d.open && d.locked && d.locked_by_key);
    assert_eq!(d.open_sprite, Some(w.idx("door_2")));
    assert_eq!(w.rand.log, ["next(100)", "next(3)"]);
    assert_eq!(
        w.sq.borrow().special_objects,
        [w.sq.borrow().objects[0].borrow().id]
    );
    assert!(w.sq.borrow().objects[0].borrow().outline_on_mouseover);

    // A wall goes in front of the door and is not a special object.
    w.place("wall_0").unwrap();
    assert_eq!(w.classes(), ["IsoObject", "IsoDoor"]);
    assert_eq!(
        w.sq.borrow().objects[0].borrow().tile.as_deref(),
        Some("wall_0")
    );

    // An open garage door: never locked, and its open sprite is itself.
    w.rand = Script {
        ints: [99].into(),
        ..Default::default()
    };
    w.place("gdoor_8").unwrap();
    let ObjectKind::Door(d) = &w.sq.borrow().objects[2].borrow().kind.clone() else {
        panic!()
    };
    assert!(d.open && !d.locked);
    assert_eq!(d.closed_sprite, Some(w.idx("gdoor_0")));
    assert_eq!(w.rand.log, ["next(100)"]);

    // Locked houses "never": no draw.
    w.settings.locked_houses = 1;
    w.rand = Script::default();
    w.place("gdoor_0").unwrap();
    assert!(w.rand.log.is_empty());
}

#[test]
fn windows_smashed_and_frames() {
    let mut w = World::new(&[
        (
            "win_0",
            1,
            "windowFN",
            &[("windowN", ""), ("SmashedTileOffset", "2")],
        ),
        ("win_2", 3, "normal", &[("windowN", "")]),
        ("wall_0", 10, "wall", &[("cutN", "")]),
        ("frame_0", 11, "normal", &[("WindowN", "")]),
        ("frame_1", 12, "normal", &[("WindowW", "")]),
    ]);
    assert_eq!(
        w.m.smashed_window_sprites.get(&w.idx("win_2")),
        Some(&w.idx("win_0"))
    );
    // Lock chance for "VeryOften" (the fallback) is 7 here.
    w.rand.ints = [3].into();
    w.place("win_2").unwrap();
    let o = w.sq.borrow().objects[0].borrow().clone();
    let ObjectKind::Window(win) = &o.kind else {
        panic!()
    };
    assert!(win.north && win.destroyed && !win.glass_removed && win.locked);
    assert_eq!(o.sprite, Some(w.idx("win_2")));
    assert_eq!(o.tile.as_deref(), Some("win_2"));
    // The smashed sprite took the closed sprite's type and flags.
    let smashed = &w.m.sprites[w.idx("win_2")];
    assert_eq!(smashed.tile_type, w.m.tables.ty.windowFN);
    assert!(smashed.properties.has(w.m.tables.flag.cutN));

    // A frame overlays the wall it matches; otherwise it is an object.
    w.place("wall_0").unwrap();
    w.place("frame_0").unwrap();
    let wall =
        w.sq.borrow()
            .objects
            .iter()
            .map(ObjectSlot::borrow)
            .find(|o| o.tile.as_deref() == Some("wall_0"))
            .map(|o| o.clone())
            .unwrap();
    assert_eq!(wall.attached_sprites(), Some(vec![w.idx("frame_0")]));
    w.place("frame_1").unwrap();
    assert_eq!(w.classes().last(), Some(&"IsoWindowFrame"));
}

#[test]
fn curtains_and_missing_sprites() {
    let mut w = World::new(&[
        ("curt_1", 1, "curtainW", &[]),
        ("curt_5", 5, "curtainW", &[]),
        ("curt_2", 2, "curtainN", &[]),
    ]);
    w.place("curt_5").unwrap();
    let ObjectKind::Curtain(c) = &w.sq.borrow().objects[0].borrow().kind.clone() else {
        panic!()
    };
    assert!(!c.north && c.open);
    assert_eq!(c.closed_sprite, Some(w.idx("curt_1")));
    // "Closed" curtain whose open sprite (+4) does not exist: the object
    // has no sprite and finishing it throws.
    assert_eq!(w.place("curt_2"), Err("NullPointerException".into()));
}

#[test]
fn trees_need_natural_floor_and_tint() {
    let mut w = World::new(&[
        ("blends_street_01_0", 1, "normal", &[("solidfloor", "")]),
        ("blends_natural_01_0", 2, "normal", &[("solidfloor", "")]),
        (
            "vegetation_trees_01_0",
            3,
            "tree",
            &[("tree", "2"), ("vegitation", "")],
        ),
    ]);
    w.place("blends_street_01_0").unwrap();
    w.place("vegetation_trees_01_0").unwrap();
    assert_eq!(w.classes(), ["IsoObject"]);
    w.sq.borrow_mut().clear_objects();
    w.place("blends_natural_01_0").unwrap();
    w.rand.ints = [10, 20, 29].into();
    w.place("vegetation_trees_01_0").unwrap();
    let o = w.sq.borrow().objects[1].borrow().clone();
    assert_eq!(
        o.kind,
        ObjectKind::Tree(IsoTree {
            size: 2,
            log_yield: 2,
            damage: 80
        })
    );
    assert_eq!((o.tint_r, o.tint_g, o.tint_b), (0.8, 0.9, 0.99));
    // A second tree replaces the first.
    w.place("vegetation_trees_01_0").unwrap();
    assert_eq!(w.classes(), ["IsoObject", "IsoTree"]);
}

#[test]
fn light_switches() {
    let mut w = World::new(&[
        (
            "light_0",
            1,
            "lightswitch",
            &[
                ("lightR", "255"),
                ("lightG", "0"),
                ("lightB", "51"),
                ("LightRadius", "0"),
            ],
        ),
        ("switch_0", 2, "lightswitch", &[]),
        (
            "bad_0",
            3,
            "lightswitch",
            &[("lightR", "x"), ("lightG", "0"), ("lightB", "0")],
        ),
    ]);
    w.place("light_0").unwrap();
    let ObjectKind::LightSwitch(s) = &w.sq.borrow().objects[0].borrow().kind.clone() else {
        panic!()
    };
    assert!(!s.light_room && s.activated);
    assert_eq!((s.primary_r, s.primary_b), (1.0, 0.2));
    assert_eq!(s.lights.len(), 1);
    assert_eq!(
        (s.lights[0].x, s.lights[0].y, s.lights[0].radius),
        (10, 20, 10)
    );
    w.place("switch_0").unwrap();
    let ObjectKind::LightSwitch(s) = &w.sq.borrow().objects[1].borrow().kind.clone() else {
        panic!()
    };
    assert!(s.light_room && s.activated && s.lights.is_empty());
    assert!(w
        .place("bad_0")
        .unwrap_err()
        .starts_with("NumberFormatException"));
}

#[test]
fn overlays_attach() {
    let mut w = World::new(&[
        ("floor_0", 1, "normal", &[("solidfloor", "")]),
        ("wall_0", 2, "wall", &[("cutN", "")]),
        (
            "over_0",
            3,
            "normal",
            &[("WallOverlay", ""), ("attachedN", "")],
        ),
        (
            "over_1",
            4,
            "normal",
            &[("WallOverlay", ""), ("attachedW", "")],
        ),
        ("rug_0", 5, "normal", &[("FloorOverlay", "")]),
    ]);
    w.place("rug_0").unwrap();
    assert!(w.sq.borrow().objects.is_empty());
    w.place("floor_0").unwrap();
    w.place("wall_0").unwrap();
    w.place("over_0").unwrap();
    w.place("rug_0").unwrap();
    // No west wall: a plain object without a tile.
    w.place("over_1").unwrap();
    assert_eq!(w.sq.borrow().objects.len(), 3);
    assert_eq!(
        w.sq.borrow().objects[0].borrow().attached_sprites(),
        Some(vec![w.idx("rug_0")])
    );
    assert_eq!(
        w.sq.borrow().objects[1].borrow().attached_sprites(),
        Some(vec![w.idx("over_0")])
    );
    assert_eq!(w.sq.borrow().objects[2].borrow().tile, None);
}

#[test]
fn containers_and_classes_by_property() {
    let mut w = World::new(&[
        (
            "crate_0",
            1,
            "normal",
            &[
                ("container", ""),
                ("container", "crate"),
                ("ContainerCapacity", "20"),
                ("Freezer", ""),
                ("FreezerCapacity", "7"),
            ],
        ),
        (
            "bbq_0",
            2,
            "normal",
            &[
                ("container", ""),
                ("container", "barbecue"),
                ("propaneTank", ""),
            ],
        ),
        ("bbq_8", 10, "normal", &[("propaneTank", "")]),
        (
            "stove_0",
            3,
            "normal",
            &[("container", ""), ("container", "toaster")],
        ),
        ("fire_0", 4, "normal", &[("container", "brazier")]),
        (
            "wash_0",
            5,
            "normal",
            &[("IsoType", "IsoCombinationWasherDryer")],
        ),
        ("broken_0", 6, "normal", &[("container", "")]),
        ("man_0", 7, "normal", &[("CustomName", "Mannequin")]),
        ("juke_0", 8, "jukebox", &[]),
    ]);
    w.place("crate_0").unwrap();
    let o = w.sq.borrow().objects[0].borrow().clone();
    let c = o.container.as_ref().unwrap();
    assert_eq!((c.kind.as_deref(), c.capacity), (Some("crate"), 20));
    assert_eq!(o.secondary_containers[0].capacity, 7);
    assert!(o.outline_on_mouseover);
    w.rand.ints = [600].into();
    for t in ["bbq_0", "stove_0", "fire_0", "wash_0", "man_0", "juke_0"] {
        w.place(t).unwrap();
    }
    assert_eq!(
        w.classes(),
        [
            "IsoObject",
            "IsoBarbecue",
            "IsoStove",
            "IsoFireplace",
            "IsoCombinationWasherDryer",
            "IsoMannequin",
            "IsoJukebox"
        ]
    );
    let ObjectKind::Barbecue(b) = &w.sq.borrow().objects[1].borrow().kind.clone() else {
        panic!()
    };
    assert!(b.has_propane_tank);
    assert_eq!(b.fuel_amount_minutes, 600);
    assert_eq!(b.no_tank_sprite, Some(w.idx("bbq_8")));
    // A container flag without a container type throws.
    assert_eq!(w.place("broken_0"), Err("NullPointerException".into()));
}

struct Radios;
impl ObjectLoadHooks for Radios {
    fn device_data_from_item(
        &mut self,
        item: &str,
        _rand: &mut dyn GameRand,
    ) -> Option<DeviceData> {
        (item == "Base.Radio").then(|| DeviceData {
            min_channel_range: 100,
            max_channel_range: 2000,
            ..Default::default()
        })
    }
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        match category {
            "Emergency" => Some(vec![(150, "E".into()), (5000, "far".into())]),
            "Radio" => Some(vec![(400, "R1".into()), (600, "R2".into())]),
            _ => None,
        }
    }
}

#[test]
fn radios_draw_in_order() {
    let mut w = World::new(&[
        ("radio_0", 1, "radio", &[("CustomItem", "Base.Radio")]),
        ("tv_0", 2, "normal", &[("signal", "tv")]),
        ("odd_0", 3, "normal", &[("signal", "other")]),
    ]);
    w.settings.minutes_stamp = 100;
    // Emergency 150 (kept: 0 < 300), Radio 400 (dropped: 900 >= 800),
    // Radio 600 (kept), preset 1 of 2, turned on (35 <= 35).
    w.rand.ints = [0, 900, 5, 1, 35].into();
    w.rand.floats = [0.5].into();
    w.place_with("radio_0", &mut Radios).unwrap();
    assert_eq!(
        w.rand.log,
        [
            "next(1000)",
            "next(1000)",
            "next(1000)",
            "nextFloat",
            "next(2)",
            "next(100)"
        ]
    );
    let d = w.sq.borrow().objects[0]
        .borrow()
        .kind
        .device_data()
        .unwrap()
        .clone();
    assert_eq!(d.presets.len(), 2);
    assert_eq!(d.channel, 600);
    assert!((d.device_volume - 0.55).abs() < 1e-6);
    assert!(d.is_turned_on);
    assert!((d.power_delta - 0.9).abs() < 1e-6);

    // No device data from the item: a default device, no draws.
    w.rand = Script::default();
    w.place("tv_0").unwrap();
    assert_eq!(w.classes()[1], "IsoTelevision");
    assert!(w.rand.log.is_empty());
    assert_eq!(w.place("odd_0"), Err("NullPointerException".into()));
}

#[test]
fn stackable_objects() {
    let mut w = World::new(&[
        (
            "table_0",
            1,
            "normal",
            &[("IsTable", ""), ("Surface", "10")],
        ),
        (
            "top_0",
            2,
            "normal",
            &[
                ("IsTableTop", ""),
                ("IsSurfaceOffset", ""),
                ("Surface", "4"),
            ],
        ),
        ("stack_0", 5, "normal", &[("StackReplaceTileOffset", "1")]),
        ("stack_1", 6, "normal", &[("IsTableTop", "")]),
    ]);
    for t in ["table_0", "top_0", "stack_0"] {
        w.place(t).unwrap();
    }
    w.sq.borrow_mut().fix_stackable_objects(&w.m);
    let y: Vec<f32> =
        w.sq.borrow()
            .objects
            .iter()
            .map(ObjectSlot::borrow)
            .map(|o| o.render_y_offset)
            .collect();
    assert_eq!(y, [0.0, 6.0, 10.0]);
    assert_eq!(
        w.sq.borrow().objects[2].borrow().sprite,
        Some(w.idx("stack_1"))
    );
}

#[test]
fn entity_loading_callback_can_borrow_actual_square_and_object_before_insertion() {
    struct Entity {
        square: crate::SquareRef,
        called: bool,
    }
    impl ObjectLoadHooks for Entity {
        fn create_iso_entity(&mut self, object: crate::IsoObjectRef, _: &mut dyn GameRand) -> bool {
            let mut square = self
                .square
                .try_borrow_mut()
                .expect("square guard must end before host callback");
            assert!(square.objects.is_empty());
            square.have_roof = true;
            object
                .try_borrow_mut()
                .expect("object guard must end before host callback")
                .key_id = 731;
            self.called = true;
            true
        }
    }
    let mut w = World::new(&[("entity", 1, "normal", &[("EntityScript", "")])]);
    let mut hook = Entity {
        square: w.sq.clone(),
        called: false,
    };
    w.place_with("entity", &mut hook).unwrap();
    assert!(hook.called);
    assert!(w.sq.borrow().have_roof);
    assert_eq!(w.sq.borrow().objects[0].borrow().key_id, 731);
    assert!(w.sq.borrow().objects[0].borrow().has_components);
}
