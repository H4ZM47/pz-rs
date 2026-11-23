use super::*;
use crate::bind_items::{install_item_env, GameItemEnv};
use crate::item_picker::PickerItem;
use pz_kahlua::TableRef;
use pz_scripts::ScriptManager;

const SCRIPTS: &str = "module Base {
    item Rock { ItemType = base:normal, Weight = 2, }
    item Apple { ItemType = base:food, Weight = 0.2, HungerChange = -10, DaysFresh = 5, }
    item Beans { ItemType = base:food, Weight = 0.5, HungerChange = -10, CannedFood = true, }
}";

fn scripts() -> Rc<ScriptManager> {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pz-bind-item-picker-{}-{n}", std::process::id()));
    let p = dir.join("media/scripts/test.txt");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, SCRIPTS).unwrap();
    let sm = ScriptManager::new();
    sm.enable_texture_capture();
    for id in ["base:normal", "base:food"] {
        sm.registries
            .get("item_type")
            .unwrap()
            .borrow_mut()
            .register_id(id, true)
            .unwrap();
    }
    let mut st = sm.statics.items.borrow_mut();
    st.constants.insert("item_type", "NORMAL", "base:normal");
    st.constants.insert("item_type", "FOOD", "base:food");
    drop(st);
    sm.load(&pz_fs::GameFs::new(dir.clone()), &[]).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    Rc::new(sm)
}

fn setup() -> Thread {
    let sm = scripts();
    crate::bind_registry::install_script_manager(sm.clone());
    let mut env = GameItemEnv::new(None);
    env.rand = Some(pz_core::random::Rand::new(7));
    install_item_env(Rc::new(env));
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    crate::bind_globals::register(&mut t);
    crate::bind_registry::register(&mut t, sm);
    crate::bind_items::register(&mut t);
    crate::bind_item_container::register(&mut t);
    register(&mut t);
    let mut l = LootSettings::default();
    l.modifiers.insert("Other", 1.0);
    l.modifiers.insert("Food", 1.0);
    item_picker::install_loot_settings(l);
    t
}

fn run(t: &mut Thread, src: &str) -> Result<Vec<Value>, String> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).expect("compile");
    t.pcall(&f, &[])
        .map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e)))
}

fn count(t: &mut Thread, kind: &str) -> String {
    count_in(t, kind, 50)
}

fn count_in(t: &mut Thread, kind: &str, capacity: i32) -> String {
    let src = format!(
        "local c = ItemContainer.new('{kind}', nil, nil)
         c:setCapacity({capacity})
         c:setSourceGrid({{}})
         ItemPicker = ItemPickerJava
         ItemPicker.fillContainer(c, nil)
         local s = ''
         for i = 0, c:getItems():size() - 1 do s = s .. c:getItems():get(i):getType() .. ' ' end
         return s"
    );
    match run(t, &src) {
        Ok(v) => pz_kahlua::exec::java_value_of(&v[0]),
        Err(e) => panic!("lua error: {e}"),
    }
}

fn dist(items: &[(&str, f32)], rolls: f32) -> PickerContainer {
    PickerContainer {
        items: items
            .iter()
            .map(|(n, c)| PickerItem { item_name: n.to_string(), chance: *c })
            .collect(),
        rolls,
        ..Default::default()
    }
}

fn install_all(containers: Vec<(&str, PickerContainer)>, procedural: Vec<(&str, PickerContainer)>) {
    let mut room = PickerRoom::default();
    for (k, v) in containers {
        room.containers.insert(k.to_string(), v);
    }
    let mut d = Distributions::default();
    d.rooms.insert("all".into(), room);
    for (k, v) in procedural {
        d.procedural.insert(k.to_string(), v);
    }
    item_picker::install(d);
}

#[test]
fn fill_container_rolls_the_all_distribution_of_the_container_type() {
    let mut t = setup();
    // chance 100: 100 * 100 * modifier 1 = 10000, every roll spawns.
    install_all(vec![("counter", dist(&[("Base.Rock", 100.0)], 3.0))], vec![]);
    assert_eq!(count(&mut t, "counter"), "Rock Rock Rock ");
    // A type without a distribution of its own falls back to `other`.
    assert_eq!(count(&mut t, "shelves"), "");
}

#[test]
fn fill_container_uses_other_unless_no_generic_loot() {
    let mut t = setup();
    install_all(vec![("other", dist(&[("Base.Rock", 100.0)], 1.0))], vec![]);
    assert_eq!(count(&mut t, "shelves"), "Rock ");
    assert_eq!(count(&mut t, "fridge"), "");
}

#[test]
fn only_one_stops_after_the_first_item() {
    let mut t = setup();
    let mut d = dist(&[("Base.Rock", 100.0), ("Base.Apple", 100.0)], 4.0);
    d.only_one = true;
    install_all(vec![("counter", d)], vec![]);
    assert_eq!(count(&mut t, "counter"), "Rock ");
}

#[test]
fn a_zero_loot_modifier_spawns_nothing() {
    let mut t = setup();
    let mut l = item_picker::loot_settings();
    l.modifiers.insert("Food", 0.0);
    item_picker::install_loot_settings(l);
    install_all(vec![("counter", dist(&[("Base.Apple", 100.0), ("Base.Rock", 100.0)], 1.0))], vec![]);
    assert_eq!(count(&mut t, "counter"), "Rock ");
}

#[test]
fn procedural_distribution_rolls_the_picked_list() {
    let mut t = setup();
    let proc = PickerContainer {
        procedural: true,
        procedural_items: vec![ProceduralItem {
            name: "Fruit".into(),
            min: 0,
            max: 2,
            weight_chance: 10,
            ..Default::default()
        }],
        ..Default::default()
    };
    install_all(vec![("counter", proc)], vec![("Fruit", dist(&[("Base.Apple", 100.0)], 2.0))]);
    assert_eq!(count(&mut t, "counter"), "Apple Apple ");
}

#[test]
fn a_full_container_stops_the_roll() {
    let mut t = setup();
    install_all(vec![("counter", dist(&[("Base.Apple", 100.0)], 30.0))], vec![]);
    // Capacity 1: five apples of 0.2, then `tryAddItemToContainer` fails
    // and the whole roll ends.
    let s = count_in(&mut t, "counter", 1);
    assert_eq!(s.matches("Apple").count(), 5, "{s}");
}

#[test]
fn loot_type_picks_the_modifier() {
    let mut t = setup();
    let mut l = item_picker::loot_settings();
    l.modifiers.insert("CannedFood", 0.0);
    item_picker::install_loot_settings(l);
    install_all(vec![("counter", dist(&[("Base.Beans", 100.0), ("Base.Apple", 100.0)], 1.0))], vec![]);
    assert_eq!(count(&mut t, "counter"), "Apple ");
}

#[test]
fn without_a_source_square_nothing_is_filled() {
    let mut t = setup();
    install_all(vec![("counter", dist(&[("Base.Rock", 100.0)], 1.0))], vec![]);
    let v = run(
        &mut t,
        "local c = ItemContainer.new('counter', nil, nil)
         ItemPickerJava.fillContainer(c, nil)
         return c:getItems():size()",
    )
    .unwrap();
    assert_eq!(pz_kahlua::exec::java_value_of(&v[0]), "0.0");
}


#[test]
fn load_gridsquare_fills_named_objects_once_and_marks_the_square() {
    let old_items = pz_inventory::shared::install_item_observer(Some(crate::component_gc::observe_item));
    let old_objects =
        pz_world::object_slot::install_object_observer(Some(crate::component_gc::observe_object));
    let mut t = setup();
    crate::bind_world_object::register_classes(&mut t);
    install_all(vec![("counter", dist(&[("Base.Rock", 100.0)], 1.0))], vec![]);
    let mut sprites = pz_world::sprite::IsoSpriteManager::new(pz_world::sprite::test_tables());
    let named = sprites.add_sprite_named("fixtures_counters_01_0");
    sprites.sprites[named].name = Some("fixtures_counters_01_0".into());
    let old_sprites = crate::bind_sprites::manager().ok();
    crate::bind_sprites::install_sprite_manager(Some(Rc::new(RefCell::new(sprites))));
    let mut square = pz_world::IsoGridSquare::new(1, 1, 0);
    for (id, sprite) in [(1, Some(named)), (2, None)] {
        let mut o = pz_world::IsoObject::new(id, pz_world::ObjectKind::Object, sprite);
        o.container = Some(pz_inventory::ItemContainer::new(Some("counter")));
        square.objects.push(pz_world::ObjectSlot::new(o).into());
    }
    let objects: Vec<_> = square.objects.iter().map(|s| s.ref_handle()).collect();
    let mut chunk = pz_world::IsoChunk::new(0, 0);
    chunk.set_square(1, 1, 0, Some(square.into()));
    let mut cell = pz_world::IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    let old_cell = crate::bind_world::with_world(|w| w.cell.take());
    crate::bind_world::set_cell(Some(cell));
    // A tile overlay for the counter (chance 1: always), with its texture.
    crate::bind_tile_overlays::shared_tile_overlays()
        .borrow_mut()
        .add_overlays(&[pz_world::tile_overlays::OverlayDef {
            tile: "fixtures_counters_01_0".into(),
            entries: vec![pz_world::tile_overlays::OverlayEntryDef {
                room: Some("other".into()),
                chance: 1,
                usage: Some(String::new()),
                tiles: vec!["overlay_crumbs_01_0".into()],
            }],
        }])
        .unwrap();
    crate::hud::test_util::install_textures(&[("overlay_crumbs_01_0", 64, 128)]);

    let main = |i: usize| {
        let v = crate::bind_world_object::object_value(&objects[i]).unwrap();
        crate::bind_world_object::adopt_square(&v, &objects[i], &Value::Nil).unwrap();
        objects[i].borrow().runtime_containers.as_ref().unwrap().main.clone().unwrap()
    };
    load_gridsquare(&mut t, 1, 1, 0).unwrap();
    let (filled, unnamed) = (main(0), main(1));
    assert!(filled.borrow().explored);
    assert_eq!(filled.borrow().items.len(), 1);
    // Without a sprite name `checkObject` returns before the container.
    assert!(!unnamed.borrow().explored);
    assert!(unnamed.borrow().items.is_empty());
    // `updateTileOverlaySprite`: the overlay goes first in the named
    // object's `attachedAnimSprite`; the unnamed object gets none.
    let overlay = |i: usize| {
        let o = objects[i].borrow();
        let sprites = crate::bind_sprites::manager().unwrap();
        let sprites = sprites.borrow();
        o.attached_anim_sprite.as_ref().map(|list| {
            list.iter()
                .map(|a| (sprites.sprites[a.sprite].name.clone(), a.copy_target_alpha, a.multiply_object_alpha))
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(overlay(0), Some(vec![(Some("overlay_crumbs_01_0".to_string()), false, true)]));
    assert_eq!(overlay(1), None);
    let done = crate::bind_world::with_world(|w| {
        w.cell.as_ref().unwrap().get_grid_square(1, 1, 0).unwrap().borrow().overlay_done
    });
    assert!(done);
    // `isOverlayDone`: a second load does not roll again.
    filled.borrow_mut().explored = false;
    load_gridsquare(&mut t, 1, 1, 0).unwrap();
    assert_eq!(filled.borrow().items.len(), 1);

    drop((filled, unnamed, objects));
    crate::bind_world::set_cell(old_cell);
    crate::bind_sprites::install_sprite_manager(old_sprites);
    pz_inventory::shared::install_item_observer(old_items);
    pz_world::object_slot::install_object_observer(old_objects);
}

#[test]
fn load_gridsquare_sets_container_overlays_by_item_count() {
    let old_items = pz_inventory::shared::install_item_observer(Some(crate::component_gc::observe_item));
    let old_objects =
        pz_world::object_slot::install_object_observer(Some(crate::component_gc::observe_object));
    let mut t = setup();
    crate::bind_world_object::register_classes(&mut t);
    install_all(vec![("counter", dist(&[("Base.Rock", 100.0)], 1.0))], vec![]);
    let mut sprites = pz_world::sprite::IsoSpriteManager::new(pz_world::sprite::test_tables());
    let named = sprites.add_sprite_named("fixtures_counters_01_0");
    sprites.sprites[named].name = Some("fixtures_counters_01_0".into());
    let old_sprites = crate::bind_sprites::manager().ok();
    crate::bind_sprites::install_sprite_manager(Some(Rc::new(RefCell::new(sprites))));
    let mut square = pz_world::IsoGridSquare::new(1, 1, 0);
    for (id, container) in [(1, Some("counter")), (2, Some("empty")), (3, None)] {
        let mut o = pz_world::IsoObject::new(id, pz_world::ObjectKind::Object, Some(named));
        o.container = container.map(|c| pz_inventory::ItemContainer::new(Some(c)));
        square.objects.push(pz_world::ObjectSlot::new(o).into());
    }
    let objects: Vec<_> = square.objects.iter().map(|s| s.ref_handle()).collect();
    let mut chunk = pz_world::IsoChunk::new(0, 0);
    chunk.set_square(1, 1, 0, Some(square.into()));
    let mut cell = pz_world::IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    let old_cell = crate::bind_world::with_world(|w| w.cell.take());
    crate::bind_world::set_cell(Some(cell));
    crate::bind_tile_overlays::shared_container_overlays().borrow_mut().add_overlays_v1(&[(
        "fixtures_counters_01_0".into(),
        vec![(Some("other".into()), Some("shelf_many".into()), Some("shelf_few".into()))],
    )]);
    crate::hud::test_util::install_textures(&[("shelf_many", 64, 128), ("shelf_few", 64, 128)]);

    load_gridsquare(&mut t, 1, 1, 0).unwrap();
    let overlay = |i: usize| {
        objects[i].borrow().overlay_sprite.as_ref().map(|o| o.name.clone())
    };
    // One item (fewer than 7): the "few items" sprite; the sprite exists
    // under that name for the renderer.
    assert_eq!(overlay(0), Some(pz_world::objects::SpriteName::Str("shelf_few".into())));
    let has_sprite = crate::bind_sprites::manager().unwrap().borrow().named_map.contains_key("shelf_few");
    assert!(has_sprite);
    // An empty container skips the update; no container clears it.
    assert_eq!(overlay(1), None);
    assert_eq!(overlay(2), None);
    // A sprite with container overlays gets no tile overlay.
    assert!(objects[0].borrow().attached_anim_sprite.is_none());
    // Seven items: the "many items" sprite, via Lua `ItemPickerJava.updateOverlaySprite`
    // and notifying the installed chunk-level invalidator.
    let o = objects[0].clone();
    let sq_ref = crate::bind_world::with_world(|w| {
        w.cell
            .as_ref()
            .and_then(|c| c.get_grid_square(1, 1, 0))
            .map(|s| s.clone())
    })
    .unwrap();
    let mut retired = pz_entity::RetiredComponents::new();
    o.borrow_mut().replace_square(Some(&sq_ref), &mut retired);
    drop(retired);
    let main = o.borrow().runtime_containers.as_ref().unwrap().main.clone().unwrap();
    let item = main.borrow().items[0].clone();
    main.borrow_mut().items.extend(std::iter::repeat_n(item, 6));
    let invalidated = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = invalidated.clone();
    crate::bind_tile_overlays::install_overlay_invalidator(Some(std::rc::Rc::new(
        move |x, y, z| log.borrow_mut().push((x, y, z)),
    )));
    let o_val = crate::bind_world_object::object_value(&o).unwrap();
    t.globals().set_str("_testOverlayObj", o_val);
    run(&mut t, "ItemPickerJava.updateOverlaySprite(_testOverlayObj)").unwrap();
    crate::bind_tile_overlays::install_overlay_invalidator(None);
    assert_eq!(overlay(0), Some(pz_world::objects::SpriteName::Str("shelf_many".into())));
    assert_eq!(*invalidated.borrow(), vec![(1, 1, 0)]);
    let _ = o.borrow_mut().take_square_for_gc();
    drop(sq_ref);

    drop((main, o, objects));
    crate::bind_world::set_cell(old_cell);
    crate::bind_sprites::install_sprite_manager(old_sprites);
    pz_inventory::shared::install_item_observer(old_items);
    pz_world::object_slot::install_object_observer(old_objects);
}
