//! `zombie.gameStates.IngameState`: the game itself. `update` is
//! `updateInternal` (the quit save, `IsoWorld.update`, `UpdateStuff`,
//! `onTick`), `render` is `renderInternal` (`renderframe` per player, then
//! `renderframetext` and `renderframeui`).

use crate::game_state::{state, GameState, StateAction, StateCx, StateRef};
use crate::iso_world::{opt_bool, opt_f64, opt_int, sandbox};
use crate::sim::FrameSim;
use crate::world_render::RenderView;
use pz_characters::character::player::PlayerRenderInputs;
use pz_iso_render::frame::PlayerView;
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
    pz_lua_api::bind_audio::sound_system()
        .borrow_mut()
        .with(|m, _| m.set_music_state(name));
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

/// Per-frame split-screen slot geometry (`MultiTextureFBO2.render` and
/// `IsoCamera.getScreenLeft/Top/Width/Height`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SplitLayout {
    pub num_players: u32,
    pub highest_active: Option<usize>,
    pub screen: (u32, u32),
}

impl SplitLayout {
    pub(crate) fn compute(
        active: [bool; MAX_PLAYERS],
        num_players_hint: usize,
        screen_w: u32,
        screen_h: u32,
    ) -> Self {
        let highest_active = active.iter().rposition(|&a| a);
        let num_players = num_players_hint
            .max(highest_active.map_or(1, |h| h + 1))
            .clamp(1, MAX_PLAYERS) as u32;
        Self {
            num_players,
            highest_active,
            screen: (screen_w.max(1), screen_h.max(1)),
        }
    }

    /// `PlayerView` for slot `n` when in split screen (`num_players > 1`);
    /// `None` for single player.
    pub(crate) fn player_view(&self, n: usize) -> Option<PlayerView> {
        (self.num_players > 1).then_some(PlayerView {
            player: n as u32,
            num_players: self.num_players,
            screen: self.screen,
        })
    }

    /// `IsoCamera.getScreenLeft/Top/Width/Height(n)` for slot `n`.
    pub(crate) fn screen_rect(&self, n: usize) -> [u32; 4] {
        PlayerView {
            player: n as u32,
            num_players: self.num_players,
            screen: self.screen,
        }
        .screen_rect()
    }

    /// `MultiTextureFBO2.render`: empty slots up to the highest active player
    /// index are covered with a black rectangle.
    pub(crate) fn is_empty_slot_to_fill(&self, n: usize, active: bool) -> bool {
        !active && self.highest_active.is_some_and(|highest| n <= highest)
    }
}

/// Populate `WeatherShader` (`screen.wgsl`) viewport parameters from the
/// effective split-screen layout rectangle.
///
/// `screen_rect` is top-down (`[left, top, view_w, view_h]`), whereas
/// `screen.wgsl` evaluates `frag.y` bottom-up (`g.flip - in.pos.y`), so
/// `offscreen_top` must be the bottom-up GL origin `screen_h - top - view_h`.
/// Dimensions are derived directly from `view_w * zoom` / `view_h * zoom`
/// so split views stay scaled to their actual viewport regardless of
/// `Core.num_players` timing.
pub(crate) fn apply_split_screen_params(
    params: &mut ScreenParams,
    screen_rect: [u32; 4],
    screen_h: u32,
    zoom: f32,
    tile_scale: i32,
) {
    let [left, top, view_w, view_h] = screen_rect;
    params.offscreen_left = left as f32;
    params.offscreen_top = screen_h.saturating_sub(top.saturating_add(view_h)) as f32;
    params.offscreen_width = view_w as f32 * zoom;
    params.offscreen_height = view_h as f32 * zoom;
    params.zoom = zoom;
    params.tile_width = if tile_scale == 2 { 64.0 } else { 32.0 };
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
        crate::failure::record_first(cx.error, (format!("Unable to rebuild main menu: {error}")).into());
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
        let snow = pz_lua_api::bind_climate::take_snow_effects();
        if let Some(w) = &cx.world { w.apply_snow_effects(snow); }
        let sim = FrameSim::capture();
        let (hearing, fire_spread) = match sandbox(cx.lua) {
            Some(sb) => {
                let sb = sb.borrow();
                (
                    opt_int(&sb, "ZombieLore.Hearing"),
                    opt_bool(&sb, "FireSpread"),
                )
            }
            None => (2, true),
        };
        let tutorial = pz_lua_api::bind_core::shared_core().borrow().tutorial;
        if !self.paused {
            // TODO(port): `ScriptManager.instance.update()`.
            if let Some(w) = cx.world.as_mut() {
                w.update_world_sounds(hearing);
                let raining = pz_lua_api::bind_climate::shared_game_time()
                    .borrow()
                    .climate
                    .is_raining();
                let _ = w.update_fire(
                    &sim,
                    pz_world::fire::FireSettings {
                        fire_spread,
                        raining,
                        tutorial,
                    },
                    hearing,
                    Some(cx.lua),
                );
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
            // `LoopedRangedWeaponSounds`, `FliesSound`, `CorpseFlies`.
            pz_lua_api::item_sound::update();
            // `WorldMapVisited.update()`: its getInstance NPE (no meta grid)
            // propagates out of the update like the original's.
            if let Err(fault) = pz_lua_api::worldmap::visited::update() {
                crate::failure::record_first(cx.error, format!("WorldMapVisited.update: {fault}").into());
            }
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
    fn render_settings(
        &mut self,
        lua: &mut LuaManager,
        n: usize,
        screen_rect: [u32; 4],
        screen_h: u32,
        zoom: f32,
    ) -> (ClimateSettings, ScreenParams) {
        let night_darkness = sandbox(lua).map_or(3, |sb| opt_int(&sb.borrow(), "NightDarkness"));
        let (last_stand, tile_scale) = {
            let c = pz_lua_api::bind_core::shared_core();
            let c = c.borrow();
            (c.is_last_stand(), c.tile_scale)
        };
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
        if let Err(e) =
            pz_lua_api::bind_search_mode::update_render_inputs(n, &mut self.render_inputs[n])
        {
            eprintln!("RenderSettings.updateRenderSettings: {e}");
        }
        let mut players = [PlayerRenderInput::default(); MAX_PLAYERS];
        players[n].night_vision_goggles = self.render_inputs[n].night_vision_goggles;
        let settings = ClimateSettings { climate, players };
        let params = settings.screen_params(n, ScreenParams::default());
        let mut params =
            pz_lua_api::bind_search_mode::apply_screen_inputs(params, n, &self.render_inputs[n]);
        apply_split_screen_params(&mut params, screen_rect, screen_h, zoom, tile_scale);
        let lock_fps = pz_lua_api::bind_climate::climate_settings().lock_fps;
        (settings, self.screen_timer.apply(params, lock_fps))
    }

    /// `renderframe(nPlayer)`.
    fn renderframe(
        &mut self,
        cx: &mut StateCx<'_>,
        n: usize,
        layout: &SplitLayout,
        out: &mut DrawList,
    ) {
        let Some(p) = player(n) else { return };
        let screen_rect = layout.screen_rect(n);
        let [_, _, view_w, view_h] = screen_rect;
        let zoom = pz_lua_api::bind_core::shared_core()
            .borrow()
            .get_zoom(n as i32);
        let (climate, screen) = self.render_settings(cx.lua, n, screen_rect, layout.screen.1, zoom);
        pz_lua_api::hud::update_progress_bar(n);
        let Some(w) = cx.world.as_mut() else { return };
        // TODO(port): skip the world while the player is asleep and the
        // fade is complete (`isAsleep`, `UIManager.getFadeAlpha`).
        let (x, y, z, dir) = {
            let c = p.borrow();
            (c.vars.x, c.vars.y, c.vars.z, c.vars.dir)
        };
        let Some(chunk_map) = w
            .streamer
            .as_ref()
            .filter(|s| n < s.num_players())
            .map(|s| {
                let m = s.chunk_map(n);
                (m.world_x - s.width / 2, m.world_y - s.width / 2, s.width)
            })
        else {
            return;
        };
        let mut view = RenderView {
            player: n,
            x,
            y,
            z,
            dir,
            chunk_map,
            width: view_w.max(1),
            height: view_h.max(1),
            split_view: layout.player_view(n),
            zoom,
            climate,
            screen,
            tick: self.tick_count as i64,
            multiplier: FrameSim::capture().multiplier,
        };
        // `IsoCamera`: the player's camera, for the HUD and the player's
        // input. TODO(port): `PlayerCamera` (deferred movement, the
        // right-click pan).
        let cam =
            pz_iso_render::Camera::player(x, y, z, view.width as i32, view.height as i32, zoom);
        {
            let mut f = self.cameras.borrow_mut();
            f.cameras[n] = Some(cam);
            f.player_index = pz_lua_api::bind_players::player_index().max(0) as usize;
            f.frame_player_index = n;
        }
        pz_player_input::set_camera_info(
            n,
            pz_player_input::CameraInfo {
                off_x: cam.off_x as f32,
                off_y: cam.off_y as f32,
                right_click_x: 0.0,
                right_click_y: 0.0,
            },
        );
        // TODO(port): `ModelOutlines.startFrameMain`/`endFrameMain`,
        // `RenderSettings.legacyPostRender`.
        let fs = cx.lua.fs.clone();
        let tm = pz_lua_api::bind_render::textures();
        let result = {
            let fs = fs.try_borrow_mut().map_err(|_| {
                pz_entity::runtime_load::RuntimeHostError::Borrowed("world render file system")
            });
            match fs {
                Ok(mut fs) => w.render(&mut view, &mut fs, &tm, &mut out.cmds),
                Err(error) => Err(error),
            }
        };
        if let Err(error) = result {
            if cx.error.is_none() {
                crate::failure::record_first(cx.error, (format!("IsoWorld.render: {error:?}")).into());
            }
            return;
        }
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
        if let Some(w) = cx.world.as_ref() {
            w.queue_radio_device_chat(n);
        }
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
            if let Err(error) = w.activate_streamed_chunks(cx.lua) {
                w.errors.push(error);
            }
            w.hand_off_events();
            w.process_pending_vehicle_chunks(cx.lua);
            w.load_vehicles_in_meta(cx.lua);
            // TODO(port): `LightingThread.instance.GameLoadingUpdate()`.
        }
        self.loading = false;
        if let Some(w) = cx.world.as_mut() {
            w.key_spawn_chance_d100 = sandbox(cx.lua).map_or(0.0, |sb| {
                (opt_f64(&sb.borrow(), "KeyLootNew") * 50.0).min(90.0) as f32
            });
            // `MapCollisionData.instance.startGame()`.
            w.start_population(&FrameSim::capture());
        }
        // TODO(port): `currentCell.putInVehicle(player)`.
        let tutorial = match pz_lua_api::bind_core::shared_core().borrow().is_tutorial() {
            Ok(value) => value,
            Err(error) => {
                crate::failure::record_first(cx.error, format!("IngameState tutorial mode: {error}").into());
                return;
            }
        };
        set_music_state(if tutorial { "Tutorial" } else { "InGame" });
        // TODO(port): `ClimateManager.getInstance().update()` on its own,
        // `AmbientStreamManager.instance.checkHaveElectricity()`.
        // `WorldMarkers.instance.init()` with the game.
        pz_lua_api::hud::world_markers::with_markers(|m| m.init());
        // The HUD reads the iso cameras (`IsoCamera.cameras`).
        crate::world_hud::install_hud_camera(self.cameras.clone());
        // The object picker behind the mouse: `UIManager.update`'s context
        // pick and `IsoObjectPicker.Instance`.
        if let Some(world) = cx.world.as_ref() {
            if let Some(sprites) = world.sprites.clone() {
                crate::world_pick::install(world.render.clone(), sprites, self.cameras.clone());
            }
        }
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
        crate::world_pick::uninstall();
        // TODO(port): the `ScreenFader` fade to black, rendering the world
        // until it ends; `RenderThread.setWaitForRenderState(false)`.
        if let Some(w) = cx.world.as_mut() {
            w.reset_ragdolls();
            if let Some(models) = w.models.take() {
                cx.retired_models.push(models);
            }
            w.reset_vehicles();
            w.stop();
        }
        // TODO(port): `ModelManager.Reset`, `ZombieSpawnRecorder.quit`,
        // `Core.offscreenBuffer.destroy`, `WeatherFxMask.destroy`.
        pz_lua_api::bind_players::reset();
        pz_lua_api::bind_core::shared_core()
            .borrow_mut()
            .num_players = 1;
        // TODO(port): `Temperature.reset`.
        if let Err(e) = pz_lua_api::hud::world_markers::with_markers(|m| m.reset()) {
            eprintln!("WorldMarkers.reset: {e:?}");
        }
        // TODO(port): `IsoMarkers.instance.reset()`.
        pz_lua_api::bind_search_mode::reset();
        // `ZomboidRadio.getInstance().Reset()`, `IsoWaveSignal.Reset()`, and
        // `IsoFireManager.Reset()`.
        if let Some(w) = cx.world.as_mut() {
            w.reset_radio();
            w.reset_fire();
        }
        // TODO(port): `GameEntityManager.Reset`, `SpriteConfigManager.Reset`,
        // `Fluid`/`Energy`/`CraftRecipeManager`/`ErosionGlobals.Reset`,
        // `IsoGenerator`, `StashSystem`, `LootRespawn`, `VehicleCache`,
        // `VehicleIDMap`, `IsoChunk`, `ChunkChecksum`,
        // the atlases and `FBORenderChunkManager.Reset`.
        // `IsoWorld.instance.KillCell()`: the cell and the world's state go.
        pz_lua_api::bind_zombie::shared_world_sounds().borrow_mut().dispose_cell();
        pz_lua_api::bind_world::set_cell(None);
        pz_lua_api::bind_sprites::install_sprite_manager(None);
        pz_lua_api::bind_world::square_queries::install_square_light(None);
        // Native KillCell precedes DesignationZone.Reset/Animal.Reset. The
        // permanent class lists retain identity; reset only their contents.
        if let Some(world) = cx.world.as_mut() {
            if world.meta_world.designation_zones.live_slot().is_some() {
                if let Err(error) = world.reset_designation_registry() {
                    crate::failure::record_first(cx.error, (format!("designation exit reset host failure: {error:?}")).into());
                    return;
                }
            }
        }
        *cx.world = None;
        let retired_emitters = pz_lua_api::bind_audio::sound_system()
            .borrow_mut()
            .with_world_emitters(|pool, ctx| pool.stop_and_reset(ctx));
        drop(retired_emitters);
        pz_lua_api::item_sound::reset();
        pz_lua_api::bind_eat::reset_emitters();
        // `WorldMap.Reset()`: the map-files cache and the visited instance
        // do not carry into the next world.
        pz_lua_api::worldmap::reset();
        // `BaseSoundManager.stop`, before the new menu environment starts.
        pz_lua_api::bind_audio::sound_system()
            .borrow_mut()
            .with(|manager, ctx| manager.stop(ctx));
        pz_lua_api::bind_misc::set_ingame_state(false);
        // The original rebuilds scripts and Lua before redirecting to the
        // menu. Registry ids cannot carry across worlds whose dictionary
        // assigns a different id to the same type. Native shutdown skips
        // the menu rebuild, retaining the just-completed save events.
        // Exact RegistryReset.resetAll phase before rebuilding scripts.
        // The older fresh-manager reconstruction is still a separate
        // ScriptManager.Reset/retained-manager parity boundary.
        let manager = pz_lua_api::bind_registry::shared_script_manager();
        if manager.native_item_key_statics().is_some() {
            if let Err(error) = manager.try_reset_native_item_key_registries() {
                crate::failure::record_first(cx.error, (format!("native item key reset: {error:?}")).into());
                return;
            }
        }
        if !cx.shutdown {
            rebuild_main_menu_with(cx, crate::boot::reload_main_menu_environment);
        }
    }

    fn redirect_state(&mut self, _cx: &mut StateCx<'_>) -> Option<StateRef> {
        Some(
            self.redirect
                .take()
                .unwrap_or_else(|| state(crate::main_screen::MainScreenState::default())),
        )
    }

    fn reenter(&mut self, _cx: &mut StateCx<'_>) {
        set_music_state("InGame");
    }

    fn update(&mut self, cx: &mut StateCx<'_>) -> StateAction {
        self.tick_count += 1;
        // TODO(port): the first 60 ticks set each player's
        // `dirtyRecalcGridStackTime = 20`.
        cx.lua
            .trigger_event("OnTickEvenPaused", &[Value::Num(self.number_ticks as f64)]);
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
                if let Err(error) = w.save_on_quit(cx.lua) {
                    crate::failure::record_first(cx.error, (format!("PlayerDB save host failure: {error}")).into());
                    return StateAction::Remain;
                }
            }
            println!("IngameState Exiting done.");
            return StateAction::Continue;
        }
        // TODO(network): `GameWindow.serverDisconnected`.
        // TODO(port): the debug states (`DebugChunkState`, the editors).
        let all_dead = pz_lua_api::bind_players::player_instance().is_some()
            && pz_lua_api::bind_players::all_players_dead();
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
                pz_lua_api::bind_character_sim::install_sim_context(std::rc::Rc::new(
                    FrameSim::capture(),
                ));
                if let Err(error) = w.update(cx.lua, &sim) {
                    // A typed character host fault is not an ordinary script
                    // update error. Stop this frame before later callbacks.
                    crate::failure::record_first(cx.error, crate::failure::EngineError::state("IsoWorld.update", error));
                    return StateAction::Remain;
                }
                // `ZomboidRadio.getInstance().update()`.
                w.update_radio(cx.lua);
            }
            // TODO(port): `ObjectAmbientEmitters` (threaded),
            // `GameEntityManager.Update`, `AnimalController.update`.
            self.update_stuff(cx);
            self.on_tick(cx.lua);
            // TODO(port): `FMODAmbientWalls.update`.
            // `TickMusicDirector`.
            call_global(cx.lua, "SadisticMusicDirectorTick");
            // TODO(port): `ObjectAmbientEmitters.update`.
            self.number_ticks += 1;
            pz_lua_api::bind_misc::set_ingame_tick(self.number_ticks);
        }
        // `GameWindow.save` that Lua (`save`) or `SleepingEvent.wakeUp`
        // asked for during this update.
        if pz_lua_api::bind_game_window::take_save_request() {
            if let Some(w) = cx.world.as_mut() {
                if let Err(error) = w.save_during_play(cx.lua) {
                    crate::failure::record_first(cx.error, (format!("GameWindow.save host failure: {error}")).into());
                    return StateAction::Remain;
                }
            }
        }
        // Live character animation advances from postupdate. TODO(port):
        // the remaining `ModelManager.instance.update()` work, `FPSGraph`.
        StateAction::Remain
    }

    fn render(&mut self, cx: &mut StateCx<'_>, out: &mut DrawList) {
        // `renderInternal`: each local player's world frame (the first
        // clears), then their text, then the UI.
        out.renderi(
            None,
            0,
            0,
            cx.screen_w,
            cx.screen_h,
            Rgba(0.0, 0.0, 0.0, 1.0),
        );
        let active: [bool; MAX_PLAYERS] = std::array::from_fn(|n| player(n).is_some());
        let core_players = pz_lua_api::bind_core::shared_core()
            .borrow()
            .num_players
            .max(1) as usize;
        let num_players_hint = (pz_lua_api::bind_players::num_players().max(1) as usize).max(core_players);
        let layout = SplitLayout::compute(
            active,
            num_players_hint,
            cx.screen_w.max(1) as u32,
            cx.screen_h.max(1) as u32,
        );
        // Keep `Core.num_players` aligned with the active split layout so
        // `Core:getOffscreenWidth/Height(n)` and Lua queries match the rendered
        // viewport divisions.
        if let Ok(mut core) = pz_lua_api::bind_core::shared_core().try_borrow_mut() {
            core.num_players = layout.num_players as i32;
        }
        for n in 0..MAX_PLAYERS {
            let Some(p) = player(n) else {
                // `MultiTextureFBO2.render`: fill empty slots up to the highest
                // active player index with black.
                if layout.is_empty_slot_to_fill(n, false) {
                    let [sx, sy, sw, sh] = layout.screen_rect(n);
                    out.renderi(
                        None,
                        sx as i32,
                        sy as i32,
                        sw as i32,
                        sh as i32,
                        Rgba(0.0, 0.0, 0.0, 1.0),
                    );
                }
                // TODO(port): `SpriteRenderer.prePopulating` for slot 0.
                continue;
            };
            // `IsoPlayer.setInstance`, `IsoCamera.setCameraCharacter`,
            // `Core.StartFrame(n, clear)`, `IsoCamera.frameState.set(n)`.
            pz_lua_api::bind_players::set_player_instance(Some(p));
            self.renderframe(cx, n, &layout, out);
            if cx.error.is_some() {
                return;
            }
        }
        // TODO(port): `FBORenderObjectHighlight.clearHighlightOnceFlag`.
        for n in 0..MAX_PLAYERS {
            if player(n).is_some() {
                self.renderframetext(cx, n);
            }
        }
        // The clock's visibility, digits and alarm come from the players'
        // alarm clocks here, then the moodles and speed controls follow it.
        pz_ui::UIManager::resize();
        self.renderframeui(cx, out);
    }
}

#[cfg(test)]
mod process_items_tests;

#[cfg(test)]
mod split_screen_tests {
    use super::*;

    #[test]
    fn split_layout_computes_single_dual_and_quad_rects_and_empty_slot_fills() {
        // Single player: full screen, `player_view` is None, no empty fills.
        let single = SplitLayout::compute([true, false, false, false], 1, 1920, 1080);
        assert_eq!(single.num_players, 1);
        assert_eq!(single.player_view(0), None);
        assert_eq!(single.screen_rect(0), [0, 0, 1920, 1080]);
        assert!(!single.is_empty_slot_to_fill(1, false));

        // Two players: left/right halves.
        let dual = SplitLayout::compute([true, true, false, false], 2, 1920, 1080);
        assert_eq!(dual.num_players, 2);
        assert_eq!(dual.screen_rect(0), [0, 0, 960, 1080]);
        assert_eq!(dual.screen_rect(1), [960, 0, 960, 1080]);
        assert_eq!(
            dual.player_view(1),
            Some(PlayerView { player: 1, num_players: 2, screen: (1920, 1080) })
        );

        // Players 0 and 2 active (slot 1 empty): 3-player quarter layout, slot 1
        // filled black, slot 3 beyond highest active so not filled.
        let sparse = SplitLayout::compute([true, false, true, false], 2, 1920, 1080);
        assert_eq!(sparse.num_players, 3);
        assert_eq!(sparse.highest_active, Some(2));
        assert_eq!(sparse.screen_rect(0), [0, 0, 960, 540]);
        assert_eq!(sparse.screen_rect(1), [960, 0, 960, 540]);
        assert_eq!(sparse.screen_rect(2), [0, 540, 960, 540]);
        assert!(sparse.is_empty_slot_to_fill(1, false));
        assert!(!sparse.is_empty_slot_to_fill(0, true));
        assert!(!sparse.is_empty_slot_to_fill(3, false));
    }

    #[test]
    fn split_screen_params_use_viewport_dimensions_and_bottom_up_origin() {
        // Two-player split (1920x1080) at zoom 1.5: each half is 960x1080.
        let dual = SplitLayout::compute([true, true, false, false], 1, 1920, 1080);
        let mut p1 = ScreenParams::default();
        apply_split_screen_params(&mut p1, dual.screen_rect(1), dual.screen.1, 1.5, 2);
        assert_eq!(p1.offscreen_left, 960.0);
        assert_eq!(p1.offscreen_top, 0.0);
        assert_eq!(p1.offscreen_width, 960.0 * 1.5);
        assert_eq!(p1.offscreen_height, 1080.0 * 1.5);
        assert_eq!(p1.zoom, 1.5);
        assert_eq!(p1.tile_width, 64.0);

        // Four-player split (1920x1080): top row occupies bottom-up y=[540, 1080],
        // bottom row occupies bottom-up y=[0, 540].
        let quad = SplitLayout::compute([true, true, true, true], 1, 1920, 1080);
        let mut top_right = ScreenParams::default();
        apply_split_screen_params(&mut top_right, quad.screen_rect(1), quad.screen.1, 2.0, 2);
        assert_eq!(top_right.offscreen_left, 960.0);
        assert_eq!(top_right.offscreen_top, 540.0);
        assert_eq!(top_right.offscreen_width, 960.0 * 2.0);
        assert_eq!(top_right.offscreen_height, 540.0 * 2.0);

        let mut bottom_left = ScreenParams::default();
        apply_split_screen_params(&mut bottom_left, quad.screen_rect(2), quad.screen.1, 1.0, 1);
        assert_eq!(bottom_left.offscreen_left, 0.0);
        assert_eq!(bottom_left.offscreen_top, 0.0);
        assert_eq!(bottom_left.offscreen_width, 960.0);
        assert_eq!(bottom_left.offscreen_height, 540.0);
        assert_eq!(bottom_left.tile_width, 32.0);
    }
}
