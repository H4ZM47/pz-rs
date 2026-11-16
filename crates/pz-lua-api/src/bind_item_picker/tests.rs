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

