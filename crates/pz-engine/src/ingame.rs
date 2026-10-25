//! `zombie.gameStates.IngameState`: the game itself. Only `enter` and the
//! shell of `update`/`render` are ported so far.

use crate::game_state::{GameState, StateAction, StateCx};
use pz_render::{DrawList, Rgba};

/// `zombie.gameStates.IngameState`.
#[derive(Default)]
pub struct IngameState {
    /// `loading` (static).
    pub loading: bool,
    /// Frames updated (`numberTicks`).
    pub number_ticks: u64,
}

impl GameState for IngameState {
    fn class_name(&self) -> &'static str {
        "zombie.gameStates.IngameState"
    }

    fn enter(&mut self, cx: &mut StateCx<'_>) {
        self.loading = true;
        pz_lua_api::bind_misc::set_ingame_state(true);
        // TODO(port): `UIManager.useUiFbo`, `SceneShaderStore.weatherShader`,
        // `GameSounds.fix3DListenerPosition(false)`; the player's
        // `updateUsername`, `setSceneCulled(false)` and
        // `getInventory().addItemsToProcessItems()`;
        // `ZombieSpawnRecorder.instance.init()`.
        if let Some(w) = cx.world.as_mut() {
            // `chunkMap[0].processAllLoadGridSquare()` then `chunkMap[0].update()`.
            w.with_stream(|s, cell, ctx| {
                s.process_all_load_grid_square(cell, ctx);
                s.chunk_map_update(0, cell, ctx);
            });
            w.hand_off_events();
            // TODO(port): `VehiclesDB2.instance.loadVehiclesInMeta()`
            // (`vehicles_db::load_vehicles_in_meta`), and
            // `LightingThread.instance.GameLoadingUpdate()`.
        }
        self.loading = false;
        // TODO(port): `MapCollisionData.instance.startGame()`
        // (`WorldPopulation::start_game` needs the engine's
        // `PopulationHost` over the cell's zombies);
        // `currentCell.putInVehicle(player)`.
        let tutorial = pz_lua_api::bind_core::shared_core().borrow().game_mode == "Tutorial";
        pz_lua_api::bind_audio::sound_system()
            .borrow_mut()
            .with(|m, _| m.set_music_state(if tutorial { "Tutorial" } else { "InGame" }));
        // TODO(port): `ClimateManager.getInstance().update()` on its own,
        // `AmbientStreamManager.instance.checkHaveElectricity()`.
        cx.lua.trigger_event("OnGameStart", &[]);
        cx.lua.trigger_event("OnLoad", &[]);
        // TODO(port): `PoolCaps.onLoadingFinished()`,
        // `FBORenderOcclusion.getInstance().init()`,
        // `UIManager.setbFadeBeforeUI(false)`, `UIManager.FadeIn(0.35)`,
        // the display focus listeners.
    }

    fn exit(&mut self, _cx: &mut StateCx<'_>) {
        // TODO(port): `IngameState.exit` (the fade, `WorldStreamer.stop`,
        // the population/pathfinding/lighting stops, the chunk unloads and
        // `world_save::close_stores`).
        pz_lua_api::bind_misc::set_ingame_state(false);
        pz_lua_api::bind_players::reset();
    }

    fn update(&mut self, _cx: &mut StateCx<'_>) -> StateAction {
        self.number_ticks += 1;
        // TODO(port): `IngameState.update` (`updateInternal`: game time
        // and climate, the world and characters, the streamer, popman,
        // `OnTick` and the rest of the frame).
        StateAction::Remain
    }

    fn render(&mut self, cx: &mut StateCx<'_>, out: &mut DrawList) {
        // TODO(port): `IngameState.render` (`renderframe` per player: the
        // world through pz-world-render / pz-iso-render, then
        // `renderframeui`).
        out.renderi(None, 0, 0, cx.screen_w, cx.screen_h, Rgba(0.0, 0.0, 0.0, 1.0));
        pz_ui::UIManager::render(&mut cx.lua.thread);
        pz_ui::with_ui(|m| m.sprite.lower_into(out));
        pz_ui::with_ui(|m| m.sprite.clear());
    }
}
