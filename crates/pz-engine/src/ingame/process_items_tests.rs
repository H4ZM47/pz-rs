use super::*;
use crate::game_window::Launch;
use crate::iso_world::IsoWorld;
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn paused_ingame_frames_preserve_real_cell_and_item_sound_queues() {
    let old_core = pz_lua_api::bind_core::shared_core();
    let old_time = pz_lua_api::bind_climate::shared_game_time();
    let old_world = pz_lua_api::bind_world::with_world(std::mem::take);
    let old_sound = pz_lua_api::bind_audio::sound_system();
    struct Restore {
        core: Rc<RefCell<pz_core::core::Core>>,
        time: Rc<RefCell<pz_core::game_time::GameTime>>,
        world: pz_lua_api::bind_world::WorldState,
        sound: Rc<RefCell<pz_sound::system::SoundSystem>>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            pz_lua_api::bind_world::with_world(|w| *w = std::mem::take(&mut self.world));
            pz_lua_api::bind_core::install_core(self.core.clone());
            pz_lua_api::bind_climate::install_game_time(self.time.clone());
            pz_lua_api::bind_audio::install_sound_system(self.sound.clone());
            pz_lua_api::item_sound::reset();
        }
    }
    let _restore = Restore {
        core: old_core,
        time: old_time,
        world: old_world,
        sound: old_sound,
    };
    pz_lua_api::bind_players::reset();
    pz_lua_api::bind_core::install_core(Rc::new(RefCell::new(pz_core::core::Core::default())));
    pz_lua_api::bind_climate::install_game_time(Rc::new(RefCell::new(
        pz_core::game_time::GameTime::default(),
    )));
    pz_lua_api::bind_audio::install_sound_system(Rc::new(RefCell::new(
        pz_sound::system::SoundSystem::silent(),
    )));
    pz_lua_api::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    // An unpaused frame runs `WorldMapVisited.update()` (#265), which like
    // the original needs `IsoWorld.instance.MetaGrid`.
    pz_lua_api::bind_world::set_meta_grid(Some(pz_world::IsoMetaGrid::with_bounds(0, 0, 1, 1)));
    pz_lua_api::item_sound::reset();
    let item = pz_inventory::shared::new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Rock")),
        1,
    ));
    pz_lua_api::bind_world::with_world(|w| {
        let queue = &w.cell.as_ref().unwrap().process_items;
        queue.add(Some(item.clone()), false);
        queue.queue_remove(Some(item.clone()));
    });
    pz_lua_api::item_sound::add_item(Some(item.clone()));
    let root = std::env::temp_dir();
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&root)));
    let mut lua = LuaManager::init(fs).unwrap();
    let source = r#"
        processTickCount = 0
        Events.OnTick.Add(function()
            if not IsoCell.getInstance():getProcessItems():isEmpty() then error('OnTick observed undrained items') end
            if not IsoCell.getInstance():getProcessItemsRemove():isEmpty() then error('OnTick observed pending removal') end
            processTickCount = processTickCount + 1
        end)
    "#;
    let f =
        pz_kahlua::stdlib::load_string(source, Some("queue-phase"), lua.thread.globals().clone())
            .unwrap();
    lua.thread.call(&f, &[]).unwrap();
    let mut world = Some(IsoWorld::new(&root, &root, root.clone()));
    let launch = Launch {
        game_dir: root.clone(),
        cache_dir: root,
        start: Default::default(),
    };
    let mut error = None;
    let mut retired_models = Vec::new();
    let mut cx = StateCx {
        lua: &mut lua,
        launch: &launch,
        screen_w: 1,
        screen_h: 1,
        multiplier: 1.0,
        time_delta: 1.0 / 30.0,
        world: &mut world,
        shutdown: false,
        error: &mut error,
        retired_models: &mut retired_models,
    };
    let mut state = IngameState {
        paused: true,
        ..Default::default()
    };
    for _ in 0..3 {
        assert!(matches!(state.update(&mut cx), StateAction::Remain));
    }
    pz_lua_api::bind_world::with_world(|w| {
        let queue = &w.cell.as_ref().unwrap().process_items;
        assert_eq!(queue.items.borrow().len(), 1);
        assert_eq!(queue.remove.borrow().len(), 1);
    });
    assert!(pz_lua_api::item_sound::shared_manager()
        .borrow()
        .emitter_for(&item)
        .is_none());
    assert_eq!(
        cx.lua.thread.globals().get_str("processTickCount").as_num(),
        Some(0.0)
    );
    state.paused = false;
    assert!(matches!(state.update(&mut cx), StateAction::Remain));
    pz_lua_api::bind_world::with_world(|w| {
        let queue = &w.cell.as_ref().unwrap().process_items;
        assert!(queue.items.borrow().is_empty());
        assert!(queue.remove.borrow().is_empty());
    });
    assert!(pz_lua_api::item_sound::shared_manager()
        .borrow()
        .emitter_for(&item)
        .is_some());
    assert_eq!(
        cx.lua.thread.globals().get_str("processTickCount").as_num(),
        Some(1.0)
    );
    assert!(cx.error.is_none(), "{:?}", cx.error.as_ref().map(|e| format!("{e:?}")));
}

#[test]
fn process_items_ages_cooks_food_and_retires_cooled_base_items() {
    let old_core = pz_lua_api::bind_core::shared_core();
    let old_time = pz_lua_api::bind_climate::shared_game_time();
    let old_world = pz_lua_api::bind_world::with_world(std::mem::take);
    struct Restore {
        core: Rc<RefCell<pz_core::core::Core>>,
        time: Rc<RefCell<pz_core::game_time::GameTime>>,
        world: pz_lua_api::bind_world::WorldState,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            pz_lua_api::bind_world::with_world(|w| *w = std::mem::take(&mut self.world));
            pz_lua_api::bind_core::install_core(self.core.clone());
            pz_lua_api::bind_climate::install_game_time(self.time.clone());
        }
    }
    let _restore = Restore {
        core: old_core,
        time: old_time,
        world: old_world,
    };

    pz_lua_api::bind_core::install_core(Rc::new(RefCell::new(pz_core::core::Core::default())));
    let gt = Rc::new(RefCell::new(pz_core::game_time::GameTime::default()));
    gt.borrow_mut().nights_survived = 0;
    gt.borrow_mut().time_of_day = 10.0;
    pz_lua_api::bind_climate::install_game_time(gt.clone());
    pz_lua_api::bind_world::set_cell(Some(pz_world::IsoCell::default()));

    let root = std::env::temp_dir();
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&root)));
    let mut lua = LuaManager::init(fs).unwrap();

    // Heated normal item that should cool to 1.0 and retire from process_items.
    let mut rock = InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Rock")),
        1,
    );
    rock.item_heat = 1.0005;
    let rock_ref = pz_inventory::shared::new_item(rock);

    // Perishable cookable food item that should cook when hot and stay in process_items.
    let mut tpl = ItemTemplate::new(ItemClass::Food, "Base", "Steak");
    tpl.days_fresh = 2;
    tpl.days_totally_rotten = 4;
    let mut steak = InventoryItem::create(Rc::new(tpl), 2);
    if let pz_inventory::ItemKind::Food(food) = &mut steak.kind {
        food.is_cookable = true;
        food.minutes_to_cook = 10.0;
        food.heat = 2.0;
        food.last_cook_minute = 180; // (time_of_day 10.0 - 7.0) * 60
        food.last_aged = 3.0;
    }
    let steak_ref = pz_inventory::shared::new_item(steak);

    pz_lua_api::bind_world::with_world(|w| {
        let q = &w.cell.as_ref().unwrap().process_items;
        q.add(Some(rock_ref.clone()), false);
        q.add(Some(steak_ref.clone()), false);
    });

    // Advance game clock by 15 minutes (0.25 hours -> minute 615).
    gt.borrow_mut().time_of_day = 10.25;
    let errors = pz_lua_api::bind_process_items::process_items(&mut lua.thread, 1.0);
    assert!(errors.is_empty(), "{errors:?}");
    pz_lua_api::bind_world::with_world(|w| {
        w.cell.as_ref().unwrap().process_items.drain_remove();
    });

    assert!((rock_ref.borrow().item_heat - 1.0).abs() < 1e-5);
    if let pz_inventory::ItemKind::Food(food) = &steak_ref.borrow().kind {
        assert!(food.cooked, "steak should cook after 15 mins above heat 1.6");
        assert!(food.cooking_time >= 15.0);
        assert!(food.heat < 2.0, "steak heat should move toward ambient");
    } else {
        panic!("expected Food item");
    }
    pz_lua_api::bind_world::with_world(|w| {
        let items = w.cell.as_ref().unwrap().process_items.items.borrow();
        assert_eq!(items.len(), 1, "cooled rock retired, perishable steak kept");
        assert!(Rc::ptr_eq(items[0].as_ref().unwrap(), &steak_ref));
    });
}
