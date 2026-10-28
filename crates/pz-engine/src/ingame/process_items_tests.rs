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
    assert!(cx.error.is_none());
}
