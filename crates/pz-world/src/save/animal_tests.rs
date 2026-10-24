//! Feeding troughs and hutch animals: hand-made objects saved, loaded and
//! saved again. Definitions here are made-up test values.

use super::animal::*;
use super::io::{Reader, Writer};
use super::items::{AllItems, ItemRegistry};
use super::lua_table::{LuaKey, LuaTable, LuaValue};
use super::object_io::{load_object, save_object};
use super::*;
use crate::objects::*;
use pz_inventory::component::{encode_component, ComponentCodec, EntityCodec};
use pz_inventory::item::{AnimalAllele, AnimalGene};

struct TestHooks {
    def: AnimalDefinition,
}

impl SaveHooks for TestHooks {
    fn animal_definition(&self, t: &str) -> Option<&AnimalDefinition> {
        (t == "testhen").then_some(&self.def)
    }
    fn hutch_max_animals(&self, _sprite: &str) -> Option<i32> {
        Some(20)
    }
}

/// An item environment whose components go through pz-entity.
struct EntityEnv(EntityCodec<'static>);
impl ItemRegistry for EntityEnv {
    fn components(&self) -> &dyn ComponentCodec {
        &self.0
    }
}

fn entity_env() -> EntityEnv {
    let ctx: &'static pz_entity::BasicContext = Box::leak(Box::default());
    EntityEnv(EntityCodec { ctx })
}

fn ctx<'a>(
    m: &'a crate::sprite::IsoSpriteManager,
    t: &'a SaveTables,
    s: &'a SaveSettings,
    items: &'a dyn ItemRegistry,
    hooks: &'a dyn SaveHooks,
) -> SaveCtx<'a> {
    SaveCtx {
        sprites: m,
        tables: t,
        settings: s,
        items,
        hooks,
        meta: None,
    }
}

fn round(o: &IsoObject, c: &SaveCtx<'_>) -> IsoObject {
    let mut w = Writer::new();
    save_object(&mut w, c, o).unwrap();
    load_object(&mut Reader::new(&w.buf), c, WORLD_VERSION, o.id)
        .unwrap()
        .unwrap()
}

fn trough(linked: (i32, i32)) -> IsoObject {
    IsoObject::new(
        1,
        ObjectKind::FeedingTrough(IsoFeedingTrough {
            feeding_types: vec![("Grass".into(), 2.5), ("Hay".into(), 1.0)],
            water: 3.0,
            linked_x: linked.0,
            linked_y: linked.1,
            north: true,
            max_water: 0.0,
            max_feed: 0,
        }),
        None,
    )
}

/// Made-up stats: a 0..1 stat and a 20..40 stat defaulting to 30.
fn stat_defs() -> Vec<CharacterStatDecl> {
    let d = |id: &str, minimum, maximum, default| CharacterStatDecl {
        field: id.to_uppercase(),
        id: id.into(),
        minimum,
        maximum,
        default,
    };
    // Declared in one order, saved in another.
    let decls = vec![d("Warmth", 20.0, 40.0, 30.0), d("Calm", 0.0, 1.0, 0.0)];
    let order = ["CALM".to_string(), "WARMTH".to_string()];
    assert!(ordered_character_stats(&decls, &["NONE".to_string()]).is_none());
    ordered_character_stats(&decls, &order).unwrap()
}

fn tables() -> SaveTables {
    let mut t = SaveTables::for_tests();
    t.character_stats = stat_defs();
    t
}

fn fluid_id() -> i16 {
    pz_entity::ComponentType::FluidContainer.id()
}

#[test]
fn feeding_trough_constructor_adds_a_fluid_container() {
    let m = crate::sprite::IsoSpriteManager::new(crate::sprite::test_tables());
    let (t, s) = (tables(), SaveSettings::default());
    let c = ctx(&m, &t, &s, &AllItems, &NoSaveHooks);
    let want = encode_component(pz_entity::Component::FluidContainer(
        pz_entity::factory::feeding_trough_fluid_container(0.0),
    ));
    for linked in [(0, 0), (12, 34)] {
        let back = round(&trough(linked), &c);
        // Masters and slaves alike: the constructor runs before load.
        assert_eq!(back.components, vec![want.clone()]);
        assert!(back.has_components);
        let ObjectKind::FeedingTrough(tr) = &back.kind else {
            panic!()
        };
        assert_eq!(
            (tr.linked_x, tr.linked_y, tr.water),
            (linked.0, linked.1, 3.0)
        );
        // Saving again keeps the container: a stable re-save.
        let again = round(&back, &c);
        assert_eq!(again.components, back.components);
    }
    // Another saved component is appended after the constructor's.
    let mut o = trough((0, 0));
    o.components.push(pz_inventory::ComponentBlob {
        type_id: 99,
        data: vec![1, 2],
    });
    let back = round(&o, &c);
    assert_eq!(
        back.components
            .iter()
            .map(|c| c.type_id)
            .collect::<Vec<_>>(),
        vec![fluid_id(), 99]
    );
}

#[test]
fn saved_trough_container_loads_in_place() {
    let m = crate::sprite::IsoSpriteManager::new(crate::sprite::test_tables());
    let (t, s) = (tables(), SaveSettings::default());
    let env = entity_env();
    let c = ctx(&m, &t, &s, &env, &NoSaveHooks);
    // A saved plain container: capacity 2, no rain catcher, no filter.
    let fc = pz_entity::FluidContainer {
        capacity: 2.0,
        ..Default::default()
    };
    let mut o = trough((0, 0));
    o.components
        .push(encode_component(pz_entity::Component::FluidContainer(fc)));
    let back = round(&o, &c);
    assert_eq!(back.components.len(), 1);
    let mut loaded = pz_entity::FluidContainer::default();
    loaded
        .load(
            &mut pz_entity::io::Reader::new(&back.components[0].data),
            &pz_entity::BasicContext::default(),
        )
        .unwrap();
    // The saved capacity, and the constructor's rain catcher, which a
    // load without that field leaves alone.
    assert_eq!((loaded.capacity, loaded.rain_catcher), (2.0, 0.55));
}

fn gene(name: &str, v1: f32, v2: f32, disorder: &str) -> AnimalGene {
    let allele = |v: f32| AnimalAllele {
        name: name.into(),
        current_value: v,
        true_ratio_value: 0.0,
        dominant: true,
        genetic_disorder: disorder.into(),
    };
    AnimalGene {
        id: 7,
        name: name.into(),
        allele1: allele(v1),
        allele2: allele(v2),
    }
}

fn test_def() -> AnimalDefinition {
    AnimalDefinition {
        eggs_per_day: 1,
        max_wool: 0.0,
        min_size: 0.8,
        max_size: 1.1,
        can_be_domesticated: true,
        wild: false,
        baby: false,
        genes: Some(vec!["maxSize".into(), "stress".into()]),
        breeds: vec![AnimalBreedDef {
            name: "plain".into(),
            wool_type: None,
            forced_genes: Vec::new(),
        }],
    }
}

fn animal(pos: i32) -> IsoAnimal {
    let mut md = LuaTable::default();
    md.rawset(LuaKey::Str("k".into()), Some(LuaValue::Num(1.0)));
    let defs = stat_defs();
    let mut stats = Stats::new(&defs);
    stats.0[1] = 50.0;
    IsoAnimal {
        animal_zone: None,
        x: 10.5,
        y: 20.5,
        z: 0.0,
        dir: 3,
        stats,
        animal_type: "testhen".into(),
        breed: "plain".into(),
        custom_name: Some("Hen".into()),
        mod_data: md,
        item_id: 1234,
        female: true,
        animal_id: 40 + pos,
        genome: vec![
            ("maxsize".into(), gene("maxsize", 0.5, 1.0, "")),
            ("stress".into(), gene("stress", 0.3, 0.4, "fidget")),
        ],
        attached_tree: Some((3, 4)),
        attach_back_to_tree_x: 0,
        attach_back_to_tree_y: 0,
        age: 100,
        hours_survived: 2400.0,
        time_since_last_update: 5,
        size: 1.0,
        attach_back_to_mother: 0,
        mother: Some(77),
        pregnant: true,
        pregnant_time: 9,
        can_have_milk: false,
        milk_qty: 0.0,
        max_milk_actual: 0.0,
        milk_removed: 0,
        preferred_hutch_position: pos,
        hutch_position: pos,
        wool_qty: 3.0,
        fertilized_time: 1,
        fertilized: true,
        eggs_today: 2,
        stress_level: 0.25,
        player_acceptance: vec![(1, 4.0), (300, 5.0), (-20, 6.0)],
        weight: 2.0,
        last_pregnancy_time: 11,
        last_milk_timer: 12,
        last_impregnate_time: 13,
        health: 0.75,
        virtual_id: 4.5,
        migration_group: Some("flock".into()),
        clutch_size: 3,
        on_hook: true,
        hook: Some((1, 2, 0)),
        attach_back_to_hook_x: 0,
        attach_back_to_hook_y: 0,
        attach_back_to_hook_z: 0,
        pet_timer: 1.5,
        wild: false,
        online_id: 1,
        genetic_disorder: Vec::new(),
        def: test_def(),
        wool_type: None,
    }
}

fn hutch(animals: Vec<(i32, IsoAnimal)>, nest: Option<IsoAnimal>) -> IsoObject {
    IsoObject::new(
        2,
        ObjectKind::Hutch(Box::new(IsoHutch {
            linked_x: 0,
            linked_y: 0,
            linked_z: 0,
            master: Some(HutchMaster {
                sprite_name: "hutch_0".into(),
                open: true,
                open_egg_hatch: false,
                saved_x: 1,
                saved_y: 2,
                saved_z: 0,
                animals,
                hutch_dirt: 0.5,
                nest_box_dirt: 0.25,
                nest_boxes: vec![
                    NestBox {
                        eggs: Vec::new(),
                        animal: nest,
                    },
                    NestBox {
                        eggs: Vec::new(),
                        animal: None,
                    },
                ],
            }),
        })),
        None,
    )
}

fn master(o: &IsoObject) -> &HutchMaster {
    let ObjectKind::Hutch(h) = &o.kind else {
        panic!()
    };
    h.master.as_ref().unwrap()
}

#[test]
fn hutch_animals_round_trip_with_load_changes() {
    let m = crate::sprite::IsoSpriteManager::new(crate::sprite::test_tables());
    let (t, s) = (tables(), SaveSettings::default());
    let hooks = TestHooks { def: test_def() };
    let c = ctx(&m, &t, &s, &AllItems, &hooks);
    let mut big = animal(17);
    big.size = 2.0;
    let o = hutch(
        vec![(1, animal(1)), (16, animal(16)), (4, animal(4)), (17, big)],
        Some(animal(9)),
    );
    let back = round(&o, &c);
    let mb = master(&back);
    // Saved in HashMap<Integer, _> order, and added back in that order.
    assert_eq!(
        mb.animals.iter().map(|e| e.0).collect::<Vec<_>>(),
        vec![16, 1, 17, 4]
    );
    let (_, a) = &mb.animals[0];
    assert_eq!(a.hutch_position, 16);
    // addAnimalInside clears the item id; the nest box animal keeps it.
    assert_eq!(a.item_id, 0);
    let nest = mb.nest_boxes[0].animal.as_ref().unwrap();
    assert_eq!((nest.item_id, nest.hutch_position), (1234, -1));
    // Runtime links are dropped; what the save held stays.
    assert_eq!(a.attached_tree, None);
    assert_eq!((a.attach_back_to_tree_x, a.attach_back_to_tree_y), (3, 4));
    assert_eq!((a.mother, a.attach_back_to_mother), (None, 77));
    assert!(a.on_hook && a.hook.is_none());
    assert_eq!((a.attach_back_to_hook_x, a.attach_back_to_hook_y), (1, 2));
    // setWild(false) clears the migration group and virtual id.
    assert_eq!((a.migration_group.as_deref(), a.virtual_id), (None, 0.0));
    // Stats are clamped; the genome is in HashMap<String, _> order.
    assert_eq!(a.stats.0, vec![0.0, 40.0]);
    let mut names = vec![("maxsize", 0), ("stress", 0)];
    pz_inventory::java::sort_string_hash_map(&mut names, |e| e.0);
    assert_eq!(
        a.genome.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(),
        names.iter().map(|e| e.0).collect::<Vec<_>>()
    );
    assert_eq!(a.genetic_disorder, vec!["fidget".to_string()]);
    // The size is clamped to the maxSize gene's range (second allele).
    let (_, big) = &mb.animals[2];
    assert_eq!(big.size, 1.1);
    // HashMap<Short, Float> order.
    let mut acc = vec![(1i16, 4.0f32), (300, 5.0), (-20, 6.0)];
    pz_inventory::java::sort_int_hash_map(&mut acc, |e| e.0 as i32);
    assert_eq!(a.player_acceptance, acc);
    assert_eq!(a.custom_name.as_deref(), Some("Hen"));
    assert_eq!((a.eggs_today, a.wool_qty), (2, 0.0));
    // The next load drops the hook and tree positions (not saved again);
    // from there re-saves are stable.
    let again = round(&back, &c);
    let (_, a2) = &master(&again).animals[0];
    assert!(!a2.on_hook);
    assert_eq!(a2.attach_back_to_tree_x, i32::MAX);
    let third = round(&again, &c);
    assert_eq!(master(&third).animals, master(&again).animals);
    let mut w1 = Writer::new();
    let mut w2 = Writer::new();
    save_object(&mut w1, &c, &again).unwrap();
    save_object(&mut w2, &c, &third).unwrap();
    assert_eq!(w1.buf, w2.buf);
}

#[test]
fn hutch_animal_needs_its_definition_and_free_places() {
    let m = crate::sprite::IsoSpriteManager::new(crate::sprite::test_tables());
    let (t, s) = (tables(), SaveSettings::default());
    let hooks = TestHooks { def: test_def() };
    let c = ctx(&m, &t, &s, &AllItems, &hooks);
    let mut w = Writer::new();
    save_object(&mut w, &c, &hutch(vec![(2, animal(2))], None)).unwrap();
    let none = ctx(&m, &t, &s, &AllItems, &NoSaveHooks);
    assert!(load_object(&mut Reader::new(&w.buf), &none, WORLD_VERSION, 1).is_err());
    // ... and the CharacterStat table.
    let bare = SaveTables::for_tests();
    let no_stats = ctx(&m, &bare, &s, &AllItems, &hooks);
    assert!(load_object(&mut Reader::new(&w.buf), &no_stats, WORLD_VERSION, 1).is_err());
    // Two animals preferring one place: the second draws a new one.
    let o = hutch(vec![(5, animal(5)), (6, animal(5))], None);
    let back = round(&o, &c);
    assert_eq!(
        master(&back)
            .animals
            .iter()
            .map(|e| e.0)
            .collect::<Vec<_>>(),
        vec![5, 0]
    );
    // Wool is saved only when the breed has a wool type and the definition
    // some wool.
    let mut a = animal(3);
    a.def.max_wool = 2.0;
    a.wool_type = Some("Wool".into());
    assert_eq!(a.max_wool(), 2.0);
    a.genome
        .push(("maxwool".into(), gene("maxwool", 0.1, 0.5, "")));
    assert_eq!(a.max_wool(), 1.0);
    let mut no_type = a.clone();
    no_type.wool_type = None;
    let mut plain = Writer::new();
    no_type.save(&mut plain, &c, false).unwrap();
    let mut wool = Writer::new();
    a.save(&mut wool, &c, false).unwrap();
    assert_eq!(wool.buf.len(), plain.buf.len() + 4);
}
