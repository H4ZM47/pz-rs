//! Hand-written checks of what the vehicle code reads from items: the
//! vehicle-part values and capacities set at creation, the `Rand` draws of
//! new radios and maps, and fluid contents in item weights.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::api::Offline;
use crate::context::{ItemContext, NoOwner};
use crate::device::{ChannelLists, DeviceData};
use crate::env::{create_from_template, ItemEnv};
use crate::io::Writer;
use crate::item::ItemKind;
use crate::saved::SavedItem;
use crate::template::{ItemClass, ItemTemplate, RadioScript, VehiclePartStats};

/// Records every `Rand.Next(bound)` and answers from a script of values.
struct DrawEnv {
    answers: RefCell<Vec<i32>>,
    bounds: RefCell<Vec<i32>>,
    channels: HashMap<String, Vec<(i32, String)>>,
    maps: Vec<String>,
    templates: HashMap<i16, Rc<ItemTemplate>>,
}

impl DrawEnv {
    fn new(answers: &[i32]) -> Self {
        DrawEnv {
            answers: RefCell::new(answers.iter().rev().copied().collect()),
            bounds: RefCell::new(Vec::new()),
            channels: HashMap::new(),
            maps: Vec::new(),
            templates: HashMap::new(),
        }
    }
}

impl ItemEnv for DrawEnv {
    fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
        self.templates.get(&id).cloned()
    }
    fn exists(&self, id: i16) -> bool {
        self.templates.contains_key(&id)
    }
    fn rand_next(&self, bound: i32) -> i32 {
        self.bounds.borrow_mut().push(bound);
        self.answers.borrow_mut().pop().unwrap_or(0)
    }
    fn channel_lists(&self) -> &dyn ChannelLists {
        &self.channels
    }
    fn map_definitions(&self) -> Vec<String> {
        self.maps.clone()
    }
}

fn part_template() -> ItemTemplate {
    let mut t = ItemTemplate::new(ItemClass::Normal, "Base", "NormalTire1");
    t.display_name = "Tire".into();
    t.actual_weight = 10.0;
    t.item_capacity = 35;
    t.max_capacity = 40;
    t.condition_affects_capacity = true;
    t.vehicle_part = VehiclePartStats {
        brake_force: 2.5,
        durability: 0.75,
        chance_to_spawn_damaged: 70,
        condition_lower_normal: 0.04,
        condition_lower_offroad: 0.5,
        wheel_friction: 1.6,
        suspension_compression: 3.2,
        engine_loudness: 90.0,
        suspension_damping: 2.6,
    };
    t
}

#[test]
fn vehicle_part_values_are_set_at_creation() {
    let e = DrawEnv::new(&[]);
    let i = create_from_template(&e, Rc::new(part_template()), 0);
    assert_eq!(i.item_capacity, 35.0);
    assert_eq!(i.max_capacity, 40);
    assert_eq!(i.vehicle_part.brake_force, 2.5);
    assert_eq!(i.vehicle_part.wheel_friction, 1.6);
    assert_eq!(i.vehicle_part.chance_to_spawn_damaged, 70);
    assert_eq!(i.vehicle_part.engine_loudness, 90.0);
    assert!(i.template.condition_affects_capacity);
    // Without script values the fields keep the `InventoryItem` defaults.
    let plain = create_from_template(
        &e,
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Rock")),
        0,
    );
    assert_eq!(plain.item_capacity, -1.0);
    assert_eq!(plain.max_capacity, -1);
    assert_eq!(plain.vehicle_part, VehiclePartStats::default());
}

#[test]
fn capacities_are_saved_and_load_keeps_the_part_values() {
    let mut e = DrawEnv::new(&[]);
    let t = Rc::new(part_template());
    e.templates.insert(5, t.clone());
    let mut i = create_from_template(&e, t, 5);
    i.item_capacity = 20.0;
    let mut w = Writer::new();
    i.save(&mut w, &e).unwrap();
    let back = match SavedItem::decode(&w.buf, 249, &e) {
        SavedItem::Item(b) => *b,
        SavedItem::Raw(_) => panic!("raw"),
    };
    assert_eq!(back.item_capacity, 20.0);
    assert_eq!(back.max_capacity, 40);
    assert_eq!(back.vehicle_part.suspension_damping, 2.6);
    // A record without the capacities loads them as unset (-1), as the
    // original's `load` resets them before reading.
    let mut i2 = create_from_template(&e, e.templates[&5].clone(), 5);
    i2.item_capacity = -1.0;
    i2.max_capacity = -1;
    let mut w = Writer::new();
    i2.save(&mut w, &e).unwrap();
    let back = match SavedItem::decode(&w.buf, 249, &e) {
        SavedItem::Item(b) => *b,
        SavedItem::Raw(_) => panic!("raw"),
    };
    assert_eq!((back.item_capacity, back.max_capacity), (-1.0, -1));
    assert_eq!(back.vehicle_part.brake_force, 2.5);
}

fn lists() -> HashMap<String, Vec<(i32, String)>> {
    let mut m = HashMap::new();
    m.insert(
        "Emergency".to_string(),
        vec![(90000, "E1".to_string()), (200, "E2".to_string())],
    );
    m.insert(
        "Radio".to_string(),
        vec![
            (98000, "R1".to_string()),
            (101000, "R2".to_string()),
            (98000, "R3".to_string()),
        ],
    );
    m.insert("Amateur".to_string(), vec![(95000, "A1".to_string())]);
    m.insert("Military".to_string(), vec![(99000, "M1".to_string())]);
    m.insert(
        "Television".to_string(),
        vec![(150, "T0".to_string()), (300, "T1".to_string())],
    );
    m
}

#[test]
fn presets_draw_per_channel_in_range() {
    let l = lists();
    let mut d = DeviceData {
        min_channel_range: 88000,
        max_channel_range: 108000,
        ..Default::default()
    };
    // Emergency (chance 300): E1 in range draws (299 passes), E2 is out of
    // range and draws nothing. Radio (chance 800): R1 passes, R2 fails,
    // R3 passes but repeats R1's frequency.
    let answers = [299, 799, 800, 0];
    let mut it = answers.iter();
    let mut bounds = Vec::new();
    d.generate_presets(&l, &mut |b| {
        bounds.push(b);
        *it.next().unwrap()
    });
    assert_eq!(bounds, vec![1000, 1000, 1000, 1000]);
    let names: Vec<_> = d.presets.presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["E1", "R1"]);
    // Two-way and high tier: Amateur and Military are drawn too.
    let mut d2 = DeviceData {
        min_channel_range: 88000,
        max_channel_range: 108000,
        two_way: true,
        is_high_tier: true,
        ..Default::default()
    };
    let mut n = 0;
    d2.generate_presets(&l, &mut |_| {
        n += 1;
        0
    });
    assert_eq!(n, 6);
    assert_eq!(d2.presets.presets.len(), 5);
    // A television takes every in-range channel without drawing.
    let mut tv = DeviceData {
        is_television: true,
        ..Default::default()
    };
    tv.generate_presets(&l, &mut |_| panic!("no draw"));
    assert_eq!(tv.presets.presets.len(), 1);
}

#[test]
fn random_channel_without_presets_rounds_to_200() {
    let mut d = DeviceData {
        min_channel_range: 88000,
        max_channel_range: 108000,
        ..Default::default()
    };
    let mut bounds = Vec::new();
    d.set_random_channel(&mut |b| {
        bounds.push(b);
        1234
    });
    assert_eq!(bounds, vec![20000]);
    assert_eq!(d.channel, 89200);
    d.presets.add_preset("A", 91000);
    d.presets.add_preset("B", 92000);
    d.set_random_channel(&mut |b| {
        assert_eq!(b, 2);
        1
    });
    assert_eq!(d.channel, 92000);
}

#[test]
fn new_radio_draws_presets_then_channel_then_the_item_draws() {
    let mut t = ItemTemplate::new(ItemClass::Radio, "Base", "RadioRed");
    t.display_name = "Radio".into();
    t.radio = RadioScript {
        transmit_range: -5,
        min_channel: 88000,
        max_channel: 108000,
        uses_battery: true,
        use_delta: 0.6,
        ..Default::default()
    };
    let mut e = DrawEnv::new(&[0, 999, 999, 999, 0]);
    e.channels = lists();
    let i = create_from_template(&e, Rc::new(t), 0);
    // Emergency E1, Radio R1..R3, the channel, OutfitRNG, the item id.
    assert_eq!(
        *e.bounds.borrow(),
        vec![1000, 1000, 1000, 1000, 1, i32::MAX, 2146250223]
    );
    let ItemKind::Radio(_, d) = &i.kind else {
        panic!("not a radio");
    };
    assert_eq!(d.device_name, "Radio");
    assert_eq!(d.transmit_range, 0);
    assert_eq!(d.use_delta, 0.6 / 60.0);
    assert!(d.is_battery_powered);
    assert_eq!(d.presets.presets.len(), 1);
    assert_eq!(d.channel, 90000);
}

#[test]
fn new_map_picks_a_definition_when_blank() {
    let t = ItemTemplate::new(ItemClass::Map, "Base", "Map");
    let mut e = DrawEnv::new(&[1]);
    e.maps = vec!["A".into(), "B".into(), "C".into()];
    let i = create_from_template(&e, Rc::new(t.clone()), 0);
    assert!(matches!(&i.kind, ItemKind::Map { map_id, .. } if map_id == "B"));
    assert_eq!(e.bounds.borrow()[0], 3);
    // Without definitions: "Default", no draw.
    let e = DrawEnv::new(&[]);
    let i = create_from_template(&e, Rc::new(t.clone()), 0);
    assert!(matches!(&i.kind, ItemKind::Map { map_id, .. } if map_id == "Default"));
    assert_eq!(*e.bounds.borrow(), vec![i32::MAX, 2146250223]);
    // A script map: kept, no draw.
    let mut t2 = t;
    t2.map = Some("Muldraugh".into());
    let e = DrawEnv::new(&[]);
    let i = create_from_template(&e, Rc::new(t2), 0);
    assert!(matches!(&i.kind, ItemKind::Map { map_id, .. } if map_id == "Muldraugh"));
}

#[test]
fn fluid_contents_count_in_weights() {
    let e = DrawEnv::new(&[]);
    let mut t = ItemTemplate::new(ItemClass::Normal, "Base", "WaterBottle");
    t.display_name = "Water Bottle".into();
    t.actual_weight = 0.2;
    let mut i = create_from_template(&e, Rc::new(t), 0);
    let mut fc = pz_entity::FluidContainer {
        capacity: 1.0,
        ..Default::default()
    };
    fc.fluids.push(pz_entity::FluidInstance::new(
        pz_entity::FluidRef::Type(1),
        0.75,
    ));
    fc.cache_invalidated = true;
    i.set_fluid_container(fc);
    // Decoded from the component bytes, as after a load.
    i.component_cache = None;
    let ctx: &dyn ItemContext = &Offline;
    assert_eq!(ctx.fluid_amount(&i), Some(0.75));
    assert_eq!(i.get_contents_weight_with(ctx, &NoOwner), 0.75);
    assert_eq!(i.get_unequipped_weight(ctx, &NoOwner), 0.2 + 0.75);
    let mut c = crate::ItemContainer::new(Some("TruckBed"));
    c.items.push(SavedItem::Item(Box::new(i)));
    assert_eq!(c.get_contents_weight(), 0.2 + 0.75);
}
