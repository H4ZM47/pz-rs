//! `zombie.gameStates.IngameState`: the game itself. `update` is
//! `updateInternal` (the quit save, `IsoWorld.update`, `UpdateStuff`,
//! `onTick`), `render` is `renderInternal` (`renderframe` per player, then
//! `renderframetext` and `renderframeui`).

use crate::game_state::{state, GameState, StateAction, StateCx, StateRef};
use crate::iso_world::{opt_bool, opt_f64, opt_int, sandbox};
use crate::sim::FrameSim;
use crate::world_render::RenderView;
use pz_characters::character::player::PlayerRenderInputs;
use pz_kahlua::Value;
use pz_lua_api::LuaManager;
use pz_render::{DrawList, Rgba, ScreenParams, ScreenTimer};
use pz_world_render::{ClimateInput, ClimateSettings, PlayerRenderInput, SharedCameraFrame};

/// `IsoPlayer.MAX`.
const MAX_PLAYERS: usize = 4;

/// `zombie.gameStates.IngameState`.
#[derive(Default)]
pub struct IngameState {
    /// `loading` (static).
    pub loading: bool,
    /// `numberTicks`: frames updated, the `OnTick` argument.
    pub number_ticks: u64,
    /// `tickCount`.
    pub tick_count: u64,
    /// `saveDelay`.
    pub save_delay: f32,
    /// `paused` (a server with nobody on it).
    pub paused: bool,
    /// `redirectState`.
    redirect: Option<StateRef>,
    /// `RenderSettings.PlayerRenderSettings`' player inputs, per slot.
    render_inputs: [PlayerRenderInputs; MAX_PLAYERS],
    /// `WeatherShader`'s timer uniforms (one per session).
    screen_timer: ScreenTimer,
    /// `IsoCamera.cameras` as the HUD reads them.
    cameras: SharedCameraFrame,
}

/// `SoundManager.instance.setMusicState(name)`.
fn set_music_state(name: &str) {
    pz_lua_api::bind_audio::sound_system().borrow_mut().with(|m, _| m.set_music_state(name));
}

/// `LuaManager.call(name, null)`: the global function `name`, if there is
/// one, with its error logged.
fn call_global(lua: &mut LuaManager, name: &str) {
    let f = lua.thread.globals().get_str(name);
    if matches!(f, Value::Nil) {
        return;
    }
    if let Err((e, tr)) = lua.thread.pcall(&f, &[]) {
        eprintln!("{name}: {e:?}\n{tr}");
    }
}

/// `IsoPlayer.players[n]`.
fn player(n: usize) -> Option<pz_lua_api::bind_character_sim::LuaGameCharacterRef> {
    pz_lua_api::bind_players::player(n)
}

/// Rebuilding Lua can retire shared hosts before a later step fails.
/// Record that failure so the state machine cannot enter the old menu.
pub(crate) fn rebuild_main_menu_with(
    cx: &mut StateCx<'_>,
    rebuild: impl FnOnce(&mut LuaManager, &crate::game_window::Launch) -> Result<(), String>,
) {
    if let Err(error) = rebuild(cx.lua, cx.launch) {
        *cx.error = Some(format!("Unable to rebuild main menu: {error}"));
    }
}

impl IngameState {
    /// `UpdateStuff()`.
    fn update_stuff(&mut self, cx: &mut StateCx<'_>) {
        let gt = pz_lua_api::bind_climate::shared_game_time();
        {
            let ctx = pz_lua_api::bind_climate::LuaClimateContext::new();
            let mut g = gt.borrow_mut();
            self.save_delay += g.get_multiplier(&ctx);
            if self.save_delay / 60.0 > 30.0 {
                self.save_delay = 0.0;
            }
            g.last_last_time_of_day = g.last_time_of_day;
            g.last_time_of_day = g.time_of_day;
        }
        // `GameTime.getInstance().update(IsoPlayer.allPlayersAsleep() &&
        // UIManager.getFadeAlpha() == 1)`, then (`IsoWorld.update`'s
        // "Update Climate" in the original, one step earlier)
        // `ClimateManager.update`. TODO(port): `allPlayersAsleep` (the
        // climate settings' `all_players_asleep`).
        pz_lua_api::bind_climate::update_game_time(&mut cx.lua.thread);
        // TODO(port): apply `bind_climate::take_snow_effects()`
        // (`IsoCell.setSnowTarget`, `ErosionIceQueen.setSnow`).
        let _ = pz_lua_api::bind_climate::take_snow_effects();
        let sim = FrameSim::capture();
        let (hearing, fire_spread) = match sandbox(cx.lua) {
            Some(sb) => {
                let sb = sb.borrow();
                (opt_int(&sb, "ZombieLore.Hearing"), opt_bool(&sb, "FireSpread"))
            }
            None => (2, true),
        };
        let tutorial = pz_lua_api::bind_core::shared_core().borrow().tutorial;
        if !self.paused {
            // TODO(port): `ScriptManager.instance.update()`.
            if let Some(w) = cx.world.as_mut() {
                w.update_world_sounds(hearing);
                // TODO(port): `RainManager.isRaining()`. `UpdateStuff`
                // wraps `IsoFireManager.Update` in its own
                // `catch (Exception)`: an `Err` is logged (`update_fire`
                // does) and the frame goes on.
                let _ = w.update_fire(&sim, pz_world::fire::FireSettings { fire_spread, raining: false, tutorial });
                // TODO(port): `RainManager.Update`, `Meta.instance.update`,
                // `VirtualZombieManager.instance.update`.
                // `MapCollisionData.updateMain` +
                // `ZombiePopulationManager.updateMain`, then
                // `PathfindNative.checkUseNativeCode` (the Java pathfinder:
                // `PolygonalMap2.instance.updateMain()`), in one
                // `try { .. } catch (Throwable)`: a population `Err` (logged)
                // skips the pathfinder's pass, `LootRespawn` and the rest go on.
                if w.update_population(&sim).is_ok() {
                    w.update_pathfinding();
                }
            }
            // TODO(port): `LootRespawn.update`, the vehicle sound
            // `Manager`, `AnimEventEmulator`, `BodyDamageSync`,
            // `VirtualVehicleManager.update`,
            // `LoopedRangedWeaponSounds`, `FliesSound`, `CorpseFlies`,
            // `WorldMapVisited.update`.
            pz_lua_api::item_sound::update();
        }
        pz_lua_api::bind_search_mode::update(pz_lua_api::bind_climate::game_time_delta());
        // `RenderSettings.getInstance().update()`: the settings are
        // computed where the frame renders (`renderframe`).
    }

    /// `onTick()`.
    fn on_tick(&mut self, lua: &mut LuaManager) {
        lua.trigger_event("OnTick", &[Value::Num(self.number_ticks as f64)]);
    }

    /// `RenderSettings.applyRenderSettings(n)`'s inputs: the climate's
    /// render values, the sandbox night darkness and the player's
    /// `WeatherShader` inputs.
    fn render_settings(&mut self, lua: &mut LuaManager, n: usize) -> (ClimateSettings, ScreenParams) {
        let night_darkness = sandbox(lua).map_or(3, |sb| opt_int(&sb.borrow(), "NightDarkness"));
        let last_stand = pz_lua_api::bind_core::shared_core().borrow().is_last_stand();
        let gt = pz_lua_api::bind_climate::shared_game_time();
        let g = gt.borrow();
        // TODO(port): `SceneShaderStore.weatherShader` (the screen pass
        // runs, so it counts as present).
        let weather_shader = true;
        let v = g.climate.render_values(n, weather_shader);
        let climate = ClimateInput {
            global_light_exterior: v.cm_global_light.exterior,
            global_light_interior: v.cm_global_light.interior,
            ambient: v.cm_ambient,
            day_light_strength: v.cm_day_light_strength,
            night_strength: v.cm_night_strength,
            desaturation: v.cm_desaturation,
            view_distance: v.cm_view_distance,
            fog_intensity: v.cm_fog_intensity,
            moon_float: v.moon_float,
            game_night: g.get_night(),
            night_darkness,
            weather_shader,
            last_stand,
        };
        drop(g);
        if let Err(e) = pz_lua_api::bind_search_mode::update_render_inputs(n, &mut self.render_inputs[n]) {
            eprintln!("RenderSettings.updateRenderSettings: {e}");
        }
        let mut players = [PlayerRenderInput::default(); MAX_PLAYERS];
        players[n].night_vision_goggles = self.render_inputs[n].night_vision_goggles;
        let settings = ClimateSettings { climate, players };
        let params = settings.screen_params(n, ScreenParams::default());
        let params = pz_lua_api::bind_search_mode::apply_screen_inputs(params, n, &self.render_inputs[n]);
        let lock_fps = pz_lua_api::bind_climate::climate_settings().lock_fps;
        (settings, self.screen_timer.apply(params, lock_fps))
    }

    /// `renderframe(nPlayer)`.
    fn renderframe(&mut self, cx: &mut StateCx<'_>, n: usize, out: &mut DrawList) {
        let Some(p) = player(n) else { return };
        let (climate, screen) = self.render_settings(cx.lua, n);
        pz_lua_api::hud::update_progress_bar(n);
        let Some(w) = cx.world.as_mut() else { return };
        // TODO(port): skip the world while the player is asleep and the
        // fade is complete (`isAsleep`, `UIManager.getFadeAlpha`).
        let (x, y, z, dir) = {
            let c = p.borrow();
            (c.vars.x, c.vars.y, c.vars.z, c.vars.dir)
        };
        let Some(chunk_map) = w.streamer.as_ref().filter(|s| n < s.num_players()).map(|s| {
            let m = s.chunk_map(n);
            (m.world_x - s.width / 2, m.world_y - s.width / 2, s.width)
        }) else {
            return;
        };
        let zoom = pz_lua_api::bind_core::shared_core().borrow().get_zoom(n as i32);
        // TODO(port): split screen (`IsoCamera.getScreenWidth/Height`).
        let mut view = RenderView {
            player: n,
            x,
            y,
            z,
            dir,
            chunk_map,
            width: cx.screen_w.max(1) as u32,
            height: cx.screen_h.max(1) as u32,
            zoom,
            climate,
            screen,
            tick: self.tick_count as i64,
            multiplier: FrameSim::capture().multiplier,
        };
        // `IsoCamera`: the player's camera, for the HUD and the player's
        // input. TODO(port): `PlayerCamera` (deferred movement, the
        // right-click pan).
        let cam = pz_iso_render::Camera::player(x, y, z, view.width as i32, view.height as i32, zoom);
        {
            let mut f = self.cameras.borrow_mut();
            f.cameras[n] = Some(cam);
            f.player_index = pz_lua_api::bind_players::player_index().max(0) as usize;
            f.frame_player_index = n;
        }
        pz_player_input::set_camera_info(
            n,
            pz_player_input::CameraInfo { off_x: cam.off_x as f32, off_y: cam.off_y as f32, right_click_x: 0.0, right_click_y: 0.0 },
        );
        // TODO(port): `ModelOutlines.startFrameMain`/`endFrameMain`,
        // `RenderSettings.legacyPostRender`.
        let fs = cx.lua.fs.clone();
        let tm = pz_lua_api::bind_render::textures();
        w.render(&mut view, &mut fs.borrow_mut(), &tm, &mut out.cmds);
        cx.lua.trigger_event("OnPostRender", &[]);
        // TODO(port): `LineDrawer.clear`, the animation debug key.
    }

    /// `renderframetext(nPlayer)`.
    fn renderframetext(&mut self, cx: &mut StateCx<'_>, n: usize) {
        // TODO(port): the visible UI elements that follow the game world
        // (`isFollowGameWorld`, `getRenderThisPlayerOnly`).
        pz_lua_api::hud::render_progress_bar(&mut cx.lua.thread, n);
        pz_lua_api::hud::world_markers::render(&mut cx.lua.thread);
        // TODO(port): `IsoMarkers.instance.render()`.
        pz_lua_api::hud::text_draw::render_batch(n as i32);
        pz_lua_api::chat::element::ChatElement::render_batch(n);
        // TODO(port): `ParameterRoomTypeEx.renderRoomTones`,
        // `ThumpDamageRender`, `Core.EndFrameText`.
    }

    /// `renderframeui()`.
    fn renderframeui(&mut self, cx: &mut StateCx<'_>, out: &mut DrawList) {
        // TODO(port): `TextManager.DrawTextFromGameWorld`, `SkyBox.draw`.
        pz_ui::UIManager::render(&mut cx.lua.thread);
        // TODO(port): `ZomboidRadio.render`, `drawDebugChunkMap`,
        // `DeadBodyAtlas`/`WorldItemAtlas.renderUI`, the FPS graph,
        // `OnSleepingTick`, `ActiveMods.renderUI`, `JoypadManager.renderUI`.
        // `Core.EndFrameUI`.
        pz_ui::with_ui(|m| m.sprite.lower_into(out));
        pz_ui::with_ui(|m| m.sprite.clear());
    }
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
        // `updateUsername`, `setSceneCulled(false)`;
        // `ZombieSpawnRecorder.instance.init()`.
        if let Some(player) = pz_lua_api::bind_players::player_instance() {
            let inventory = player.borrow().items.inventory.clone();
            if let Err(e) = pz_lua_api::bind_process_items::add_container_items(&inventory) {
                eprintln!("[IngameState.enter] addItemsToProcessItems: {e}");
            }
        }
        if let Some(w) = cx.world.as_mut() {
            // `chunkMap[0].processAllLoadGridSquare()` then `chunkMap[0].update()`.
            w.with_stream(|s, cell, ctx| {
                s.process_all_load_grid_square(cell, ctx);
                s.chunk_map_update(0, cell, ctx);
            });
            if let Err(error) = w.activate_streamed_chunks(cx.lua) { w.errors.push(error); }
            w.hand_off_events();
            // TODO(port): `VehiclesDB2.instance.loadVehiclesInMeta()`
            // (`vehicles_db::load_vehicles_in_meta`), and
            // `LightingThread.instance.GameLoadingUpdate()`.
        }
        self.loading = false;
        if let Some(w) = cx.world.as_mut() {
            w.key_spawn_chance_d100 = sandbox(cx.lua).map_or(0.0, |sb| (opt_f64(&sb.borrow(), "KeyLootNew") * 50.0).min(90.0) as f32);
            // `MapCollisionData.instance.startGame()`.
            w.start_population(&FrameSim::capture());
        }
        // TODO(port): `currentCell.putInVehicle(player)`.
        let tutorial = pz_lua_api::bind_core::shared_core().borrow().game_mode == "Tutorial";
        set_music_state(if tutorial { "Tutorial" } else { "InGame" });
        // TODO(port): `ClimateManager.getInstance().update()` on its own,
        // `AmbientStreamManager.instance.checkHaveElectricity()`.
        // `WorldMarkers.instance.init()` with the game.
        pz_lua_api::hud::world_markers::with_markers(|m| m.init());
        // The HUD reads the iso cameras (`IsoCamera.cameras`).
        crate::world_hud::install_hud_camera(self.cameras.clone());
        cx.lua.trigger_event("OnGameStart", &[]);
        cx.lua.trigger_event("OnLoad", &[]);
        // TODO(port): `PoolCaps.onLoadingFinished()`,
        // `FBORenderOcclusion.getInstance().init()`,
        // `UIManager.setbFadeBeforeUI(false)`, `UIManager.FadeIn(0.35)`,
        // the display focus listeners.
    }

    fn exit(&mut self, cx: &mut StateCx<'_>) {
        // TODO(port): the display focus listeners, Steam rich presence,
        // `UIManager.useUiFbo`, the FPS graph, `UIManager.updateBeforeFadeOut`.
        set_music_state("MainMenu");
        // TODO(port): the `ScreenFader` fade to black, rendering the world
        // until it ends; `RenderThread.setWaitForRenderState(false)`.
        if let Some(w) = cx.world.as_mut() {
            if let Some(models) = w.models.take() {
                cx.retired_models.push(models);
            }
            w.stop();
        }
        // TODO(port): `ModelManager.Reset`, `ZombieSpawnRecorder.quit`,
        // `Core.offscreenBuffer.destroy`, `WeatherFxMask.destroy`.
        pz_lua_api::bind_players::reset();
        pz_lua_api::bind_core::shared_core().borrow_mut().num_players = 1;
        // TODO(port): `Temperature.reset`.
        if let Err(e) = pz_lua_api::hud::world_markers::with_markers(|m| m.reset()) {
            eprintln!("WorldMarkers.reset: {e:?}");
        }
        // TODO(port): `IsoMarkers.instance.reset()`.
        pz_lua_api::bind_search_mode::reset();
        // TODO(port): `ZomboidRadio.Reset`, `IsoWaveSignal.Reset`,
        // `GameEntityManager.Reset`, `SpriteConfigManager.Reset`,
        // `Fluid`/`Energy`/`CraftRecipeManager`/`ErosionGlobals.Reset`,
        // `IsoGenerator`, `StashSystem`, `LootRespawn`, `VehicleCache`,
        // `VehicleIDMap`, `IsoChunk`, `ChunkChecksum`,
        // the atlases and `FBORenderChunkManager.Reset`.
        // `IsoWorld.instance.KillCell()`: the cell and the world's state go.
        pz_lua_api::bind_world::set_cell(None);
        *cx.world = None;
        let retired_emitters = pz_lua_api::bind_audio::sound_system().borrow_mut().world_emitters.reset();
        drop(retired_emitters);
        pz_lua_api::item_sound::reset();
        // `BaseSoundManager.stop`, before the new menu environment starts.
        pz_lua_api::bind_audio::sound_system().borrow_mut().with(|manager, ctx| manager.stop(ctx));
        pz_lua_api::bind_misc::set_ingame_state(false);
        // The original rebuilds scripts and Lua before redirecting to the
        // menu. Registry ids cannot carry across worlds whose dictionary
        // assigns a different id to the same type. Native shutdown skips
        // the menu rebuild, retaining the just-completed save events.
        if !cx.shutdown {
            rebuild_main_menu_with(cx, crate::boot::reload_main_menu_environment);
        }
    }

    fn redirect_state(&mut self, _cx: &mut StateCx<'_>) -> Option<StateRef> {
        Some(self.redirect.take().unwrap_or_else(|| state(crate::main_screen::MainScreenState::default())))
    }

    fn reenter(&mut self, _cx: &mut StateCx<'_>) {
        set_music_state("InGame");
    }

    fn update(&mut self, cx: &mut StateCx<'_>) -> StateAction {
        self.tick_count += 1;
        // TODO(port): the first 60 ticks set each player's
        // `dirtyRecalcGridStackTime = 20`.
        cx.lua.trigger_event("OnTickEvenPaused", &[Value::Num(self.number_ticks as f64)]);
        // TODO(port): `DebugFileWatcher.instance.update()`,
        // `AdvancedAnimator.checkModifiedFiles()`.
        let exiting = {
            let core = pz_lua_api::bind_core::shared_core();
            let mut c = core.borrow_mut();
            std::mem::replace(&mut c.exiting, false)
        };
        if exiting {
            println!("IngameState Exiting...");
            // `PlayerDB.saveLocalPlayersForce()`, `GameWindow.save(true)`,
            // `OnPostSave`.
            if let Some(w) = cx.world.as_mut() {
                w.save_on_quit(cx.lua);
            }
            println!("IngameState Exiting done.");
            return StateAction::Continue;
        }
        // TODO(network): `GameWindow.serverDisconnected`.
        // TODO(port): the debug states (`DebugChunkState`, the editors).
        let all_dead = pz_lua_api::bind_players::player_instance().is_some() && pz_lua_api::bind_players::all_players_dead();
        if all_dead {
            // TODO(port): `UIManager.getSpeedControls().SetCurrentGameSpeed(1)`,
            // `IsoCamera.update()`.
        }
        // TODO(port): `waitMul` from the speed controls.
        if !self.paused {
            if let Some(w) = cx.world.as_mut() {
                // `chunkMap[n].ProcessChunkPos(player)`
                // (`IsoWorld.doChunkMapUpdate`).
                w.process_chunk_pos(cx.lua);
                let sim = FrameSim::capture();
                pz_lua_api::bind_character_sim::install_sim_context(std::rc::Rc::new(FrameSim::capture()));
                w.update(cx.lua, &sim);
            }
            // TODO(port): `ObjectAmbientEmitters` (threaded),
            // `GameEntityManager.Update`, `AnimalController.update`,
            // `ZomboidRadio.update`.
            self.update_stuff(cx);
            self.on_tick(cx.lua);
            // TODO(port): `FMODAmbientWalls.update`.
            // `TickMusicDirector`.
            call_global(cx.lua, "SadisticMusicDirectorTick");
            // TODO(port): `ObjectAmbientEmitters.update`.
            self.number_ticks += 1;
        }
        // Live character animation advances from postupdate. TODO(port):
        // the remaining `ModelManager.instance.update()` work, `FPSGraph`.
        StateAction::Remain
    }

    fn render(&mut self, cx: &mut StateCx<'_>, out: &mut DrawList) {
        // `renderInternal`: each local player's world frame (the first
        // clears), then their text, then the UI.
        out.renderi(None, 0, 0, cx.screen_w, cx.screen_h, Rgba(0.0, 0.0, 0.0, 1.0));
        for n in 0..MAX_PLAYERS {
            let Some(p) = player(n) else {
                // TODO(port): `SpriteRenderer.prePopulating` for slot 0.
                continue;
            };
            // `IsoPlayer.setInstance`, `IsoCamera.setCameraCharacter`,
            // `Core.StartFrame(n, clear)`, `IsoCamera.frameState.set(n)`.
            pz_lua_api::bind_players::set_player_instance(Some(p));
            self.renderframe(cx, n, out);
        }
        // TODO(port): `FBORenderObjectHighlight.clearHighlightOnceFlag`.
        for n in 0..MAX_PLAYERS {
            if player(n).is_some() {
                self.renderframetext(cx, n);
            }
        }
        // TODO(port): `UIManager.resize()`.
        self.renderframeui(cx, out);
    }
}

#[cfg(test)]
mod process_items_tests;
