use pz_scripts::objects::definitions::{FluidCategory, FluidType};
use pz_scripts::objects::entity::components::combat::MaterialType;
use pz_scripts::objects::entity::components::crafting::StartMode;

use super::*;
use crate::components::*;
use crate::context::{DictScript, ScriptDictionary};
use crate::io::{pack_unit, unpack_unit, Reader, Writer};
use crate::tables::{AttributeTypeInfo, InitialValue};

/// A context with a few attribute types and fluids, nothing else.
fn ctx() -> BasicContext {
    let mut attributes = AttributeTypes::default();
    for (id, name, kind) in [
        (0, "Sharpness", AttributeKind::Float),
        (1, "HeadCondition", AttributeKind::Int),
        (4, "TimesHeadRepaired", AttributeKind::Int),
        (70, "Label", AttributeKind::String),
        (130, "Flag", AttributeKind::Bool),
    ] {
        attributes.insert(AttributeTypeInfo {
            id,
            name: name.into(),
            kind,
            initial: Some(InitialValue::Int(0)),
            hidden_ui: true,
        });
    }
    let mut fluids = FluidRegistry::default();
    fluids.insert(
        Some(FluidType::Water.id()),
        FluidInfo {
            name: "Water".into(),
            color: [0.5, 0.5, 1.0, 1.0],
            categories: vec![FluidCategory::Water],
            ..Default::default()
        },
    );
    fluids.insert(
        Some(FluidType::TaintedWater.id()),
        FluidInfo {
            name: "TaintedWater".into(),
            color: [0.5, 0.5, 1.0, 1.0],
            categories: vec![],
            ..Default::default()
        },
    );
    fluids.insert(
        None,
        FluidInfo {
            name: "Cola".into(),
            color: [0.2, 0.1, 0.1, 1.0],
            categories: vec![],
            ..Default::default()
        },
    );
    BasicContext {
        attributes,
        fluids: Some(fluids),
        dictionary: None,
        recipes: None,
    }
}

fn roundtrip(c: &Components) -> Components {
    let bytes = save_components(c);
    let (back, info) = load_components(&bytes, 249, &ctx()).unwrap();
    assert!(info.issues.is_empty(), "{:?}", info.issues);
    assert_eq!(info.consumed, bytes.len());
    assert_eq!(save_components(&back), bytes);
    back
}

#[test]
fn unit_bytes_survive_unpack_pack() {
    for b in i8::MIN..=i8::MAX {
        assert_eq!(pack_unit(unpack_unit(b)), b);
    }
    assert_eq!(pack_unit(2.0), 127);
    assert_eq!(pack_unit(-1.0), -128);
}

#[test]
fn entity_framing_is_count_then_length_prefixed_blocks() {
    let mut c = Components::new();
    c.add(Component::Script(EntityScriptInfo {
        original_is_item: true,
        original_script: Some("Base.Mug".into()),
    }));
    c.add(Component::Empty(ComponentType::CraftRecipe));
    let bytes = save_components(&c);
    let mut want = vec![2u8];
    // Script: id 9, isItem, has script, "Base.Mug".
    let body = [&[0u8, 9, 1, 1, 0, 8][..], b"Base.Mug"].concat();
    want.extend_from_slice(&(body.len() as i32).to_be_bytes());
    want.extend_from_slice(&body);
    // CraftRecipe: id 20, no data.
    want.extend_from_slice(&[0, 0, 0, 2, 0, 20]);
    assert_eq!(bytes, want);
    assert_eq!(
        entity_len(&[&bytes[..], &[9, 9]].concat()).unwrap(),
        bytes.len()
    );
    assert_eq!(roundtrip(&c), c);
}

#[test]
fn unknown_and_failing_blocks_are_skipped() {
    let mut w = Writer::new();
    w.i8(3);
    // An id no component type has.
    let s = w.block_start();
    w.i16(99);
    w.i32(7);
    w.block_end(s);
    // A durability with an unknown material name.
    let s = w.block_start();
    w.i16(ComponentType::Durability.id());
    w.i16(4);
    w.string("Cheese");
    w.block_end(s);
    // A meta tag with a byte left over.
    let s = w.block_start();
    w.i16(ComponentType::MetaTag.id());
    w.i64(42);
    w.i8(1);
    w.block_end(s);
    let (c, info) = load_components(&w.buf, 249, &ctx()).unwrap();
    assert_eq!(info.consumed, w.buf.len());
    assert_eq!(info.issues.len(), 3);
    assert_eq!(info.loaded, vec![ComponentType::MetaTag]);
    assert_eq!(c.list, vec![Component::MetaTag(MetaTag { stored_id: 42 })]);
}

#[test]
fn loading_in_place_keeps_what_load_does_not_reset() {
    let mut c = Components::new();
    c.add(Component::FluidContainer(
        factory::feeding_trough_fluid_container(0.0),
    ));
    let logic = CraftLogic {
        action_anim_override: Some("Untie_Low".into()),
        ..Default::default()
    };
    c.add(Component::CraftLogic(logic));
    // A save holding a plain container and a plain craft logic.
    let mut saved = Components::new();
    saved.add(Component::CraftLogic(CraftLogic::default()));
    saved.add(Component::FluidContainer(FluidContainer::default()));
    let bytes = save_components(&saved);
    let info = load_components_into(&mut c, &bytes, 249, &ctx()).unwrap();
    assert_eq!(
        info.loaded,
        vec![ComponentType::CraftLogic, ComponentType::FluidContainer]
    );
    // Order stays the constructor's; the filter and capacity were reset,
    // the rain catcher and animation override were not.
    let Component::FluidContainer(fc) = &c.list[0] else {
        panic!()
    };
    assert_eq!(
        (fc.capacity, fc.whitelist.is_none(), fc.rain_catcher),
        (1.0, true, 0.55)
    );
    let Component::CraftLogic(l) = &c.list[1] else {
        panic!()
    };
    assert_eq!(l.action_anim_override.as_deref(), Some("Untie_Low"));
    // Items drop components the save did not have.
    c.add(Component::Durability(Durability::default()));
    c.retain_loaded(&info.loaded);
    assert_eq!(c.len(), 2);
}

#[test]
fn feeding_trough_container_layout() {
    let fc = factory::feeding_trough_fluid_container(0.0);
    let mut w = Writer::new();
    fc.save(&mut w);
    let mut want = Writer::new();
    // capacity | whitelist | canPlayerEmpty | rainCatcher.
    want.i16(1 | 8 | 64 | 512);
    want.f32(0.05);
    want.bool(true);
    want.i8(2);
    want.i8(FluidType::Water.id());
    want.i8(FluidType::TaintedWater.id());
    want.i8(2);
    for s in java::hash_set_order(["Water".to_string(), "TaintedWater".to_string()]) {
        want.string(&s);
    }
    want.i8(0);
    want.f32(0.55);
    assert_eq!(w.buf, want.buf);
    let mut back = FluidContainer::default();
    back.load(&mut Reader::new(&w.buf), &ctx()).unwrap();
    assert_eq!(back, fc);
}

#[test]
fn fluid_container_forms_and_registry_checks() {
    let mut fc = FluidContainer {
        capacity: 2.0,
        container_name: "Bottle".into(),
        ..Default::default()
    };
    fc.fluids.push(FluidInstance::new(
        FluidRef::Type(FluidType::Water.id()),
        0.5,
    ));
    let one = Component::FluidContainer(fc.clone());
    let mut c = Components::new();
    c.add(one.clone());
    assert_eq!(roundtrip(&c).list[0], one);
    // Two instances use the counted form; a modded one goes by name and a
    // custom colour is kept.
    let mut cola = FluidInstance::new(FluidRef::Modded("Cola".into()), 0.25);
    cola.custom_color = Some([unpack_unit(0), unpack_unit(10), unpack_unit(-128)]);
    fc.fluids.push(cola);
    fc.blacklist = Some(FluidFilter {
        whitelist: false,
        fluids: vec![FluidType::Water],
        fluid_strings: vec!["Cola".into()],
        categories: vec![FluidCategory::Beverage],
    });
    let mut c = Components::new();
    c.add(Component::FluidContainer(fc.clone()));
    assert_eq!(roundtrip(&c).list[0], Component::FluidContainer(fc.clone()));
    // A fluid the registry lacks is dropped; so is a filter name.
    fc.fluids
        .push(FluidInstance::new(FluidRef::Modded("Gone".into()), 0.1));
    fc.blacklist
        .as_mut()
        .unwrap()
        .fluid_strings
        .push("Gone".into());
    let mut c = Components::new();
    c.add(Component::FluidContainer(fc));
    let (back, _) = load_components(&save_components(&c), 249, &ctx()).unwrap();
    let Component::FluidContainer(b) = &back.list[0] else {
        panic!()
    };
    assert_eq!(b.fluids.len(), 2);
    assert_eq!(b.blacklist.as_ref().unwrap().fluid_strings, ["Cola"]);
    // A saved colour equal to the fluid's own folds back to "no colour".
    let mut w = Writer::new();
    w.i8(1 | 4);
    w.i8(FluidType::Water.id());
    for v in [0.5f32, 0.5, 1.0] {
        w.i8(pack_unit(v));
    }
    w.f32(0.3);
    let fi = FluidInstance::load(&mut Reader::new(&w.buf), &ctx()).unwrap();
    assert_eq!(
        fi.custom_color,
        Some([
            unpack_unit(pack_unit(0.5)),
            unpack_unit(pack_unit(0.5)),
            1.0
        ])
    );
    let mut w = Writer::new();
    w.i8(1 | 4);
    w.i8(FluidType::Water.id());
    for b in [pack_unit(1.0), pack_unit(1.0), pack_unit(1.0)] {
        w.i8(b);
    }
    w.f32(0.3);
    let mut white = ctx();
    white.fluids.as_mut().unwrap().insert(
        Some(FluidType::Water.id()),
        FluidInfo {
            name: "Water".into(),
            color: [1.0, 1.0, 1.0, 1.0],
            categories: vec![],
            ..Default::default()
        },
    );
    let fi = FluidInstance::load(&mut Reader::new(&w.buf), &white).unwrap();
    assert_eq!(fi.custom_color, None);
}

#[test]
fn filter_sets_follow_java_ordering() {
    let mut f = FluidFilter::new(true);
    f.add_fluid(FluidType::TaintedWater);
    f.add_fluid(FluidType::Water);
    f.add_fluid(FluidType::Water);
    f.add_fluid(FluidType::Modded);
    assert_eq!(f.fluids, [FluidType::Water, FluidType::TaintedWater]);
    assert_eq!(
        f.fluid_strings,
        java::hash_set_order(["TaintedWater".to_string(), "Water".to_string()])
    );
    f.add_category(FluidCategory::Water);
    f.add_category(FluidCategory::Beverage);
    assert_eq!(
        f.categories,
        [FluidCategory::Beverage, FluidCategory::Water]
    );
}

#[test]
fn attributes_use_id_list_or_bit_mask() {
    let mut a = AttributeContainer::default();
    a.set(1, AttributeValue::Int(10));
    a.set(0, AttributeValue::Float(0.5));
    let mut w = Writer::new();
    a.save(&mut w);
    // Two 8-bit ids: 2 * 8 + 16 <= 64, so an id list in insertion order.
    assert_eq!(&w.buf[..4], &[8, 0, 2, 1]);
    let mut c = Components::new();
    c.add(Component::Attributes(a));
    roundtrip(&c);

    // An id past 127 makes ids 16 bits wide; four of them (80 bits) do not
    // fit three 64-bit masks' worth, so the mask form is used.
    let mut a = AttributeContainer::default();
    a.set(130, AttributeValue::Bool(true));
    a.set(70, AttributeValue::String("x".into()));
    a.set(4, AttributeValue::Int(3));
    a.set(1, AttributeValue::Int(2));
    let mut w = Writer::new();
    a.save(&mut w);
    assert_eq!(&w.buf[..4], &[16, 0, 4, 0]);
    // Mask form: one 8-bit id over 64 needs two masks (128 bits) but
    // ten such ids in 8 bits each plus 16 exceed it.
    let mut m = AttributeContainer::default();
    for id in [4, 1, 0] {
        m.set(id, AttributeValue::Int(id as i32));
    }
    for id in 5..12 {
        m.set(id, AttributeValue::Int(id as i32));
    }
    let mut cx = ctx();
    for id in [0].into_iter().chain(5..12) {
        cx.attributes.insert(AttributeTypeInfo {
            id,
            name: format!("A{id}"),
            kind: AttributeKind::Int,
            initial: None,
            hidden_ui: true,
        });
    }
    let mut w = Writer::new();
    m.save(&mut w);
    assert_eq!(&w.buf[..2], &[1, 1]);
    // Values are written in insertion order but read back in id order.
    let mut back = AttributeContainer::default();
    back.load(&mut Reader::new(&w.buf), &cx).unwrap();
    assert_eq!(
        back.attributes.iter().map(|x| x.id).collect::<Vec<_>>(),
        (0..12).filter(|i| ![2, 3].contains(i)).collect::<Vec<_>>()
    );
    assert_eq!(back.attributes[0].value, AttributeValue::Int(4));
    // An unregistered id fails the load.
    let mut w = Writer::new();
    w.i8(8);
    w.i16(1);
    w.i8(99);
    assert!(AttributeContainer::default()
        .load(&mut Reader::new(&w.buf), &ctx())
        .is_err());
}

#[test]
fn sprite_configs_resolve_through_the_dictionary() {
    let sc = SpriteConfig {
        config_script: Some(DictScriptRef {
            id: Some(3),
            name: None,
            script_version: 77,
        }),
        multi_square_master: true,
    };
    let mut c = Components::new();
    c.add(Component::SpriteConfig(sc));
    c.add(Component::SpriteOverlayConfig(SpriteOverlayConfig {
        config_script: Some(DictScriptRef {
            id: Some(1),
            name: None,
            script_version: -1,
        }),
        applied_style: Some("Rusty".into()),
    }));
    assert_eq!(roundtrip(&c), c);
    let bytes = save_components(&c);

    let mut dict = ScriptDictionary::default();
    dict.insert(
        ScriptRegister::SpriteConfigs,
        3,
        DictScript {
            name: "Base.Barrel".into(),
            script_version: -1,
        },
    );
    dict.insert(
        ScriptRegister::SpriteOverlayConfigs,
        1,
        DictScript {
            name: "Base.Rack".into(),
            script_version: -1,
        },
    );
    let mut cx = ctx();
    cx.dictionary = Some(dict);
    let (back, _) = load_components(&bytes, 249, &cx).unwrap();
    // Saving writes the script's own version.
    let Component::SpriteConfig(s) = &back.list[0] else {
        panic!()
    };
    let got = s
        .config_script
        .as_ref()
        .map(|d| (d.name.as_deref(), d.script_version));
    assert_eq!(got, Some((Some("Base.Barrel"), -1)));
    // An id the dictionary lacks leaves the previous script in place.
    cx.dictionary = Some(ScriptDictionary::default());
    let mut prior = Components::new();
    prior.add(Component::SpriteConfig(SpriteConfig::default()));
    load_components_into(&mut prior, &bytes, 249, &cx).unwrap();
    assert_eq!(
        prior.list[0],
        Component::SpriteConfig(SpriteConfig {
            config_script: None,
            multi_square_master: true
        })
    );
}

#[test]
fn craft_logics_and_in_progress_crafts() {
    // inputs: 0; no mod data; consumed; ratio 1.0.
    let data = CraftRecipeData {
        recipe: Some(("Base.DryLeather".into(), 5)),
        elapsed_time: 12.5,
        rest: vec![0, 0, 0, 0, 0, 1, 0x3f, 0x80, 0, 0],
    };
    let logic = CraftLogic {
        recipe_tag_query: Some("dryleather".into()),
        start_mode: StartMode::Automatic,
        inputs_group_name: Some("in".into()),
        outputs_group_name: None,
        in_progress: vec![data],
        action_anim_override: None,
    };
    let mut c = Components::new();
    c.add(Component::DryingCraftLogic(DryingCraftLogic {
        logic: logic.clone(),
        wetness: vec![0.25],
    }));
    assert_eq!(roundtrip(&c), c);
    // A recipe whose version changed is cancelled, and the saved wetness
    // count then no longer matches.
    let mut cx = ctx();
    cx.recipes = Some(
        [(
            "Base.DryLeather".to_string(),
            context::CraftRecipeInfo {
                script_version: 6,
                input_count: 0,
            },
        )]
        .into(),
    );
    let (back, _) = load_components(&save_components(&c), 249, &cx).unwrap();
    let Component::DryingCraftLogic(d) = &back.list[0] else {
        panic!()
    };
    assert!(d.logic.in_progress.is_empty());
    assert!(d.wetness.is_empty());

    // Wetness saved for a different number of crafts is zeroed.
    let mut w = Writer::new();
    logic.save(&mut w);
    w.i32(2);
    w.f64(1.0);
    w.f64(2.0);
    let mut d = DryingCraftLogic::default();
    d.load(&mut Reader::new(&w.buf), 249, &ctx()).unwrap();
    assert_eq!(d.wetness, [0.0]);
}

#[test]
fn resources_merge_groups_and_drop_duplicate_ids() {
    let item = |id: &str| Resource {
        id: id.into(),
        resource_type: 1,
        resource_io: 1,
        is_locked: false,
        progress: 0.0,
        channel: 0,
        flags: 0,
        body: ResourceBody::Item {
            capacity: 1.0,
            stack_any_item: true,
            stored_items: ItemList {
                entries: vec![ItemRecord {
                    identical: 2,
                    data: vec![1, 2, 3],
                    extra_ids: vec![77],
                }],
            },
        },
    };
    let fluid = Resource {
        id: "water".into(),
        resource_type: 2,
        resource_io: 2,
        is_locked: true,
        progress: 0.5,
        channel: 3,
        flags: 4,
        body: ResourceBody::Fluid(Box::new(FluidContainer {
            capacity: 10.0,
            ..Default::default()
        })),
    };
    let energy = Resource {
        id: "power".into(),
        resource_type: 3,
        resource_io: 1,
        is_locked: false,
        progress: 0.0,
        channel: 0,
        flags: 0,
        body: ResourceBody::Energy {
            energy: Some(EnergyRef::Modded("Steam".into())),
            capacity: 4.0,
            stored_energy: 1.0,
        },
    };
    let res = Resources {
        groups: vec![
            ResourceGroup {
                name: "a".into(),
                resources: vec![item("i0"), fluid],
            },
            ResourceGroup {
                name: "b".into(),
                resources: vec![energy],
            },
        ],
    };
    let mut c = Components::new();
    c.add(Component::Resources(res.clone()));
    assert_eq!(roundtrip(&c).list[0], Component::Resources(res.clone()));

    let mut dup = res;
    dup.groups.push(ResourceGroup {
        name: "a".into(),
        resources: vec![item("i1"), item("i0")],
    });
    let mut w = Writer::new();
    dup.save(&mut w);
    let mut back = Resources::default();
    back.load(&mut Reader::new(&w.buf), 249, &ctx()).unwrap();
    assert_eq!(back.groups.len(), 2);
    let ids: Vec<&str> = back.groups[0]
        .resources
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(ids, ["i0", "water", "i1"]);
}

#[test]
fn small_components_round_trip() {
    let mut c = Components::new();
    c.add(Component::UiConfig(UiConfig {
        xui_skin_name: Some("S".into()),
        entity_style_name: None,
        ui_enabled: false,
    }));
    c.add(Component::CraftBench(CraftBench {
        recipe_tag_query: "q".into(),
        fluid_input_channels: 5,
        energy_input_channels: 0,
    }));
    c.add(Component::Durability(Durability {
        current_hit_points: 0.5,
        max_hit_points: 1.0,
        material: MaterialType::Wood,
    }));
    c.add(Component::MetaTag(MetaTag { stored_id: -3 }));
    c.add(Component::Empty(ComponentType::ContextMenuConfig));
    c.add(Component::Opaque {
        kind: ComponentType::FurnaceLogic,
        data: vec![1, 2, 3],
    });
    assert_eq!(roundtrip(&c), c);
    // A second component of a type is refused unless replacing.
    assert!(!c.add(Component::MetaTag(MetaTag { stored_id: 1 })));
    c.add_replace(Component::MetaTag(MetaTag { stored_id: 1 }), true);
    assert_eq!(
        c.list.last(),
        Some(&Component::MetaTag(MetaTag { stored_id: 1 }))
    );
}

#[test]
fn meta_entities_round_trip() {
    let mut comps = Components::new();
    comps.add(Component::FluidContainer(FluidContainer {
        rain_catcher: 0.4,
        ..Default::default()
    }));
    let m = MetaEntity {
        entity_net_id: 1 << 40,
        original_entity_type: 1,
        x: 10.5,
        y: -2.0,
        z: 0.0,
        is_outside_cached: true,
        components: comps,
    };
    let bytes = save_meta_entities(&[m.clone(), MetaEntity::default()]);
    let back = load_meta_entities(&bytes, 249, &ctx()).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(back[0].0, m);
    let list: Vec<_> = back.into_iter().map(|(m, _)| m).collect();
    assert_eq!(save_meta_entities(&list), bytes);
}

#[test]
fn attribute_types_by_name_ignore_case() {
    let cx = ctx();
    assert_eq!(
        cx.attribute_type_by_name("sharpness").map(|t| t.id),
        Some(0)
    );
    assert!(cx.attribute_type_by_name("Nope").is_none());
}

/// Reads the registries from an installed game when `PZ_JAR` points at
/// `projectzomboid.jar`.
#[test]
fn jar_tables_when_available() {
    let Ok(path) = std::env::var("PZ_JAR") else {
        return;
    };
    let jar = pz_classfile::Jar::open(path).unwrap();
    let attrs = AttributeTypes::from_jar(&jar).unwrap();
    assert!(attrs.iter().count() > 0);
    assert!(!tables::enum_fluid_names(&jar).unwrap().is_empty());
    assert!(!tables::named_colors(&jar).unwrap().is_empty());
}

#[test]
fn fluid_amount_cache_and_ratios() {
    let c = ctx();
    let mut fc = FluidContainer::default();
    fc.set_capacity(2.0);
    assert!(fc.is_empty());
    assert_eq!(fc.get_filled_ratio(), 0.0);
    fc.add_fluid(FluidRef::Type(FluidType::Water.id()), None, 0.5, &c);
    assert_eq!(fc.get_amount(), 0.5);
    assert_eq!(fc.get_filled_ratio(), 0.25);
    assert_eq!(fc.get_free_capacity(), 1.5);
    // Adding more than fits clamps to the free capacity.
    fc.add_fluid(FluidRef::Type(FluidType::Water.id()), None, 5.0, &c);
    assert_eq!(fc.fluids.len(), 1);
    assert_eq!(fc.fluids[0].amount, 2.0);
    assert!(fc.is_full());
    assert_eq!(fc.get_filled_ratio(), 1.0);
    // Within 0.001 of the capacity the amount reads as the capacity.
    fc.fluids[0].amount = 1.9995;
    fc.cache_invalidated = true;
    assert_eq!(fc.get_amount(), 2.0);
    fc.empty();
    assert!(fc.is_empty() && fc.fluids.is_empty());
}

#[test]
fn fluid_color_is_the_amount_weighted_mix() {
    let c = ctx();
    let mut fc = FluidContainer::default();
    fc.set_capacity(1.0);
    assert_eq!(fc.get_color(&c), [1.0, 1.0, 1.0]);
    fc.add_fluid(FluidRef::Type(FluidType::Water.id()), None, 0.25, &c);
    fc.add_fluid(FluidRef::Modded("Cola".into()), None, 0.75, &c);
    // Water (0.5, 0.5, 1.0) a quarter, Cola (0.2, 0.1, 0.1) three quarters.
    let col = fc.get_color(&c);
    let want = [
        0.0f32 + 0.5 * (0.25f32 / 1.0) + 0.2 * (0.75f32 / 1.0),
        0.0f32 + 0.5 * 0.25 + 0.1 * 0.75,
        0.0f32 + 1.0 * 0.25 + 0.1 * 0.75,
    ];
    assert_eq!(col, want);
    // A custom colour replaces the fluid's own.
    fc.fluids[1].custom_color = Some([0.0, 0.0, 0.0]);
    assert_eq!(fc.get_color(&c), [0.125, 0.125, 0.25]);
}

#[test]
fn fluids_that_do_not_blend_are_refused() {
    let mut c = ctx();
    let mut no_cola = FluidFilter::new(false);
    no_cola.add_fluid_string("Cola");
    c.fluids
        .as_mut()
        .unwrap()
        .set_blend_filters("Water", None, Some(no_cola));
    let mut fc = FluidContainer::default();
    fc.add_fluid(FluidRef::Type(FluidType::Water.id()), None, 0.25, &c);
    assert!(!fc.can_add_fluid(&FluidRef::Modded("Cola".into()), &c));
    fc.add_fluid(FluidRef::Modded("Cola".into()), None, 0.25, &c);
    assert_eq!(fc.fluids.len(), 1);
    // Tainted water blends with water.
    fc.add_fluid(FluidRef::Type(FluidType::TaintedWater.id()), None, 0.25, &c);
    assert_eq!(fc.fluids.len(), 2);
    // A locked container takes nothing.
    fc.input_locked = true;
    assert!(!fc.can_add_fluid(&FluidRef::Type(FluidType::Water.id()), &c));
}

#[test]
fn colors_fluids_mix_their_colour() {
    let mut c = ctx();
    c.fluids.as_mut().unwrap().insert(
        Some(FluidType::Dye.id()),
        FluidInfo {
            name: "Dye".into(),
            color: [1.0, 1.0, 1.0, 1.0],
            categories: vec![FluidCategory::Colors],
            ..Default::default()
        },
    );
    let dye = FluidRef::Type(FluidType::Dye.id());
    let mut fc = FluidContainer::default();
    fc.add_fluid(dye.clone(), Some([1.0, 0.0, 0.0]), 0.25, &c);
    assert_eq!(fc.fluids[0].custom_color, Some([1.0, 0.0, 0.0]));
    // Grey on one side: an RGB lerp by the added share (0.25 of 0.5).
    fc.add_fluid(dye.clone(), Some([0.5, 0.5, 0.5]), 0.25, &c);
    assert_eq!(fc.fluids.len(), 1);
    assert_eq!(fc.fluids[0].amount, 0.5);
    assert_eq!(fc.fluids[0].custom_color, Some([0.75, 0.25, 0.25]));
    // Water is not a colour fluid: its colour is not mixed.
    let mut w = FluidContainer::default();
    w.add_fluid(
        FluidRef::Type(FluidType::Water.id()),
        Some([0.0, 0.0, 0.0]),
        0.25,
        &c,
    );
    w.add_fluid(
        FluidRef::Type(FluidType::Water.id()),
        Some([1.0, 0.0, 0.0]),
        0.25,
        &c,
    );
    assert_eq!(w.fluids[0].custom_color, Some([0.0, 0.0, 0.0]));
}

#[test]
fn loaded_container_caches_its_amount() {
    let c = ctx();
    let mut fc = FluidContainer::default();
    fc.set_capacity(3.0);
    fc.add_fluid(FluidRef::Type(FluidType::Water.id()), None, 1.0, &c);
    let mut w = Writer::new();
    fc.save(&mut w);
    let mut back = FluidContainer::default();
    back.load(&mut Reader::new(&w.buf), &c).unwrap();
    assert!(!back.cache_invalidated);
    assert_eq!(back.amount_cache, 1.0);
    assert_eq!(back, fc);
}
