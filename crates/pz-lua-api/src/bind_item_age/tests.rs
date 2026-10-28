use super::*;
use pz_inventory::shared::SharedContainer;
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use pz_kahlua::{NativeFunction, TableRef};
use std::cell::Cell;

struct Restore {
    world: Option<crate::bind_world::WorldState>,
    core: Rc<RefCell<pz_core::core::Core>>,
    time: Rc<RefCell<pz_core::game_time::GameTime>>,
}
impl Drop for Restore {
    fn drop(&mut self) {
        let old = self.world.take().unwrap();
        let retired = crate::bind_world::with_world(|w| std::mem::replace(w, old));
        drop(retired);
        crate::bind_core::install_core(self.core.clone());
        crate::bind_climate::install_game_time(self.time.clone());
    }
}

fn setup() -> (Restore, Thread, Runtime) {
    let restore = Restore {
        world: Some(crate::bind_world::with_world(std::mem::take)),
        core: crate::bind_core::shared_core(),
        time: crate::bind_climate::shared_game_time(),
    };
    crate::bind_world::with_world(|w| w.cell = Some(Default::default()));
    crate::bind_core::install_core(Rc::new(RefCell::new(Default::default())));
    crate::bind_climate::install_game_time(Rc::new(RefCell::new(Default::default())));
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    crate::bind_items::register(&mut thread);
    let events = EventManager::register(thread.globals(), "Events", &["OnContainerUpdate"]);
    let defs =
        ["FridgeFactor", "FoodRotSpeed", "ElecShutModifier"].map(|name| pz_sandbox::OptionDef {
            name: name.into(),
            kind: pz_sandbox::OptionKind::Integer {
                min: -1,
                max: 100000,
                default: if name == "ElecShutModifier" { -1 } else { 3 },
            },
            translation: None,
            value_translation: None,
        });
    let runtime = Runtime {
        options: Some(Rc::new(RefCell::new(pz_sandbox::SandboxOptions::new(
            &defs,
        )))),
        events: Some(events),
    };
    (restore, thread, runtime)
}

fn now(hours: f32) {
    let time = crate::bind_climate::shared_game_time();
    let mut time = time.borrow_mut();
    time.nights_survived = (hours / 24.0) as i32;
    time.time_of_day = 7.0 + hours % 24.0;
}

fn item(class: ItemClass, name: &str) -> ItemRef {
    let mut template = ItemTemplate::new(class, "Base", name);
    template.days_fresh = 1;
    template.days_totally_rotten = 2;
    let item = shared::new_item(InventoryItem::create(Rc::new(template), 1));
    if class == ItemClass::Food {
        food_mut(&item, |f| {
            f.last_aged = 0.0;
            f.last_frozen_update = 0.0;
        })
        .unwrap();
    }
    item
}

fn run(thread: &mut Thread, source: &str) {
    let f = pz_kahlua::stdlib::load_string(
        source,
        Some("food-age-host-test"),
        thread.globals().clone(),
    )
    .unwrap();
    thread
        .pcall(&f, &[])
        .unwrap_or_else(|e| panic!("Lua fixture failed: {e:?}"));
}

fn observer(
    thread: &mut Thread,
    f: impl Fn(&mut pz_kahlua::NativeCall<'_>) -> LuaResult<usize> + 'static,
) {
    thread.globals().set_str(
        "ageObserver",
        Value::Native(NativeFunction::new("ageObserver", f)),
    );
    run(
        thread,
        "Events.OnContainerUpdate.Add(function(food) ageObserver(food) end)",
    );
}

fn power_parent(
    container: &ContainerRef,
    f: impl Fn(&mut pz_kahlua::NativeCall<'_>) -> LuaResult<usize> + 'static,
) {
    let table = TableRef::new();
    table.set_str(
        "getObjectIndex",
        Value::Native(NativeFunction::new("getObjectIndex", |c| Ok(c.push(0)))),
    );
    table.set_str(
        "checkObjectPowered",
        Value::Native(NativeFunction::new("checkObjectPowered", f)),
    );
    crate::bind_item_container::set_parent(container, Value::Table(table));
}

#[test]
fn base_is_empty_and_bag_walks_actual_duplicate_and_later_appended_children() {
    let (_restore, mut thread, runtime) = setup();
    let base = item(ItemClass::Normal, "Ordinary");
    base.borrow_mut().condition = 7;
    thread
        .globals()
        .set_str("ordinary", crate::bind_items::item_value(&base));
    run(&mut thread, "ordinary:updateAge()");
    assert_eq!(base.borrow().condition, 7);
    assert!(update_age_with(&mut thread, &base, Some(false), &runtime).is_err());
    let bag = item(ItemClass::Container, "Bag");
    let inventory = shared::inventory_of(&bag.borrow()).unwrap();
    let first = item(ItemClass::Food, "First");
    let second = item(ItemClass::Food, "Second");
    first.borrow_mut().id = 71;
    second.borrow_mut().id = 71;
    shared::add_item(&inventory, &first);
    inventory.borrow_mut().items.push(first.clone());
    let count = Rc::new(Cell::new(0));
    let calls = count.clone();
    let items = inventory.clone();
    let appended = second.clone();
    observer(&mut thread, move |_| {
        let n = calls.get();
        calls.set(n + 1);
        if n == 0 {
            shared::add_item(&items, &appended);
            now(48.0);
        }
        Ok(0)
    });
    now(24.0);
    update_age_with(&mut thread, &bag, None, &runtime).unwrap();
    assert_eq!(count.get(), 3);
    assert_eq!(food(&first, |f| f.age).unwrap(), 2.0);
    assert_eq!(food(&second, |f| f.age).unwrap(), 2.0);
    assert!(Rc::ptr_eq(&inventory.borrow().items[2], &second));
}

#[test]
fn power_callback_can_move_food_and_later_predicate_uses_new_outer_container() {
    let (_restore, mut thread, runtime) = setup();
    let food_item = item(ItemClass::Food, "MovingFood");
    let freezer = SharedContainer::new(1, "freezer").into_ref();
    freezer.borrow_mut().custom_temperature = 0.2;
    let next = SharedContainer::new(2, "fridge").into_ref();
    shared::add_item(&freezer, &food_item);
    let actual = food_item.clone();
    let moved = next.clone();
    let calls = Rc::new(Cell::new(0));
    let callback_calls = calls.clone();
    power_parent(&freezer, move |c| {
        callback_calls.set(callback_calls.get() + 1);
        assert!(actual.try_borrow_mut().is_ok());
        crate::bind_world::with_world(|w| assert!(w.cell.is_some()));
        shared::add_item(&moved, &actual);
        Ok(c.push(true))
    });
    now(4.0);
    update_age_with(&mut thread, &food_item, Some(false), &runtime).unwrap();
    assert_eq!(calls.get(), 1);
    assert!(Rc::ptr_eq(
        &shared::container_of(&food_item).unwrap(),
        &next
    ));
    assert_eq!(
        food(&food_item, |f| (f.freezing_time, f.frozen, f.heat)).unwrap(),
        (0.0, false, 0.2)
    );
    assert_eq!(food(&food_item, |f| f.age).unwrap(), (4.0f64 / 24.0) as f32);
}

#[test]
fn later_power_failure_retains_completed_freezing_prefix() {
    let (_restore, mut thread, runtime) = setup();
    let actual = item(ItemClass::Food, "Prefix");
    food_mut(&actual, |f| f.fertilized = true).unwrap();
    let freezer = SharedContainer::new(1, "freezer").into_ref();
    shared::add_item(&freezer, &actual);
    let calls = Rc::new(Cell::new(0));
    let seen = calls.clone();
    power_parent(&freezer, move |c| {
        let n = seen.get();
        seen.set(n + 1);
        if n == 1 {
            Err(LuaError::runtime("second power failure"))
        } else {
            Ok(c.push(true))
        }
    });
    now(1.0);
    assert!(update_age_with(&mut thread, &actual, Some(false), &runtime).is_err());
    assert_eq!(calls.get(), 2);
    assert_eq!(
        food(&actual, |f| (
            f.freezing_time,
            f.fertilized,
            f.last_frozen_update,
            f.last_aged
        ))
        .unwrap(),
        (25.0, false, 0.0, 0.0)
    );
}

#[test]
fn heat_queues_actual_identity_before_event_and_event_mutation_survives() {
    let (_restore, mut thread, runtime) = setup();
    let actual = item(ItemClass::Food, "Queued");
    let other = item(ItemClass::Food, "SameId");
    actual.borrow_mut().id = 8;
    other.borrow_mut().id = 8;
    food_mut(&actual, |f| f.age = 0.999).unwrap();
    let container = SharedContainer::new(1, "none").into_ref();
    container.borrow_mut().custom_temperature = 0.2;
    shared::add_item(&container, &actual);
    crate::bind_world::with_world(|w| {
        let queues = &w.cell.as_ref().unwrap().process_items;
        queues.add(Some(other.clone()), false);
        queues.queue_remove(Some(actual.clone()));
    });
    let expected = actual.clone();
    let owner = container.clone();
    let calls = Rc::new(Cell::new(0));
    let seen = calls.clone();
    observer(&mut thread, move |c| {
        assert!(Rc::ptr_eq(
            &crate::bind_items::item_arg_value(&c.arg(0)).unwrap(),
            &expected
        ));
        assert!(process_contains(&expected).unwrap());
        assert!(owner.try_borrow_mut().is_ok());
        assert!(food(&expected, |f| f.age).unwrap() > 1.0);
        food_mut(&expected, |f| f.age = 9.0)?;
        seen.set(seen.get() + 1);
        Ok(0)
    });
    now(0.125);
    update_age_with(&mut thread, &actual, Some(false), &runtime).unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(food(&actual, |f| f.age).unwrap(), 9.0);
    assert_eq!(
        food(&actual, |f| f.heat).unwrap(),
        food_age::lerp_heat(1.0, 0.2, 0.125 / 0.33333334)
    );
    crate::bind_world::with_world(|w| {
        let queues = &w.cell.as_ref().unwrap().process_items;
        assert_eq!(queues.items.borrow().len(), 2);
        assert!(queues.remove.borrow().is_empty());
    });
}

#[test]
fn noarg_timer_does_not_suppress_aging_or_local_events() {
    let (_restore, mut thread, runtime) = setup();
    let count = Rc::new(Cell::new(0));
    let calls = count.clone();
    observer(&mut thread, move |_| {
        calls.set(calls.get() + 1);
        Ok(0)
    });
    now(24.0);
    for name in ["A", "B"] {
        let item = item(ItemClass::Food, name);
        update_age_with(&mut thread, &item, None, &runtime).unwrap();
        assert_eq!(food(&item, |f| f.age).unwrap(), 1.0);
    }
    assert_eq!(count.get(), 2);
}

#[test]
fn server_transport_gap_errors_after_age_and_never_fakes_delivery() {
    let (_restore, mut thread, runtime) = setup();
    crate::bind_core::shared_core().borrow_mut().is_server = true;
    let item = item(ItemClass::Food, "Server");
    now(24.0);
    update_age_with(&mut thread, &item, Some(false), &runtime).unwrap();
    now(48.0);
    assert!(update_age_with(&mut thread, &item, Some(true), &runtime).is_err());
    assert_eq!(food(&item, |f| (f.age, f.last_aged)).unwrap(), (2.0, 48.0));
}

#[test]
fn source_power_callback_precedes_live_fridge_factor_read() {
    let (_restore, mut thread, runtime) = setup();
    let actual = item(ItemClass::Food, "Cold");
    let fridge = SharedContainer::new(1, "fridge").into_ref();
    fridge.borrow_mut().custom_temperature = 0.2;
    shared::add_item(&fridge, &actual);
    let source = TableRef::new();
    let options = runtime.options.as_ref().unwrap().clone();
    let receiver = actual.clone();
    let owner = fridge.clone();
    let calls = Rc::new(Cell::new(0));
    let seen = calls.clone();
    source.set_str(
        "haveElectricity",
        Value::Native(NativeFunction::new("haveElectricity", move |c| {
            assert!(receiver.try_borrow_mut().is_ok());
            assert!(owner.try_borrow_mut().is_ok());
            crate::bind_world::with_world(|w| assert!(w.cell.is_some()));
            options
                .borrow_mut()
                .option_by_name_mut("FridgeFactor")
                .unwrap()
                .config
                .set_int(4);
            seen.set(seen.get() + 1);
            Ok(c.push(true))
        })),
    );
    fridge.borrow_mut().source_grid = Some(Rc::new(crate::bind_item_container::LuaRef(
        Value::Table(source),
    )));
    now(24.0);
    update_age_with(&mut thread, &actual, Some(false), &runtime).unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(
        food(&actual, |f| (f.age, f.last_aged)).unwrap(),
        (0.1, 24.0)
    );
}

#[test]
fn missing_source_power_method_errors_after_freezing_and_heat_prefixes() {
    let (_restore, mut thread, runtime) = setup();
    let actual = item(ItemClass::Food, "UnsupportedSource");
    let fridge = SharedContainer::new(1, "fridge").into_ref();
    fridge.borrow_mut().custom_temperature = 0.2;
    fridge.borrow_mut().source_grid = Some(Rc::new(crate::bind_item_container::LuaRef(
        Value::Table(TableRef::new()),
    )));
    shared::add_item(&fridge, &actual);
    now(24.0);
    let error = update_age_with(&mut thread, &actual, Some(false), &runtime).unwrap_err();
    assert!(error.to_string().contains("haveElectricity"));
    assert_eq!(
        food(&actual, |f| (
            f.last_frozen_update,
            f.heat,
            f.age,
            f.last_aged
        ))
        .unwrap(),
        (24.0, 0.2, 0.0, 0.0)
    );
}
