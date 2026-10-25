# Engine wiring checklist

Hooks that subsystem crates expose and the game loop (`pz-engine`,
`pz-client`) must call. Tick each item when it is wired on main.

## Boot (`GameWindow.init`)
- [ ] Mods (only when Core's mods option is on): `bind_mods::read_default`,
      `GameFs::load_mods`, mod texture packs via `mod_pack_files` →
      `load_texture_pack` + `set_texture_pack_lookup` (#24).
- [ ] ScriptManager: `load_builtin_registries` + `load`, then
      `bind_registry::install_script_manager` (#22/#27).
- [ ] Real `MapGroupsHost` via `bind_map_groups::install_host` (#17/#24).
- [ ] Gamepads: `Gamepads::new(game_dir, cache_dir, c)` (#32).
- [ ] Sound: build `pz_audio::studio::Studio` from
      `SetupHeaders::from_game_dir`, load the strings, Sound, SoundMP and
      Music banks, start `pz_audio::output::Output` on it, then
      `bind_audio::install_sound_system(SoundSystem::new(Box::new(backend), env, backend.parameter_count()))`
      with `backend = StudioBackend::new(studio)` (pz-sound feature
      `studio`); set its `rand_next` and `media_file_exists` (#80/#82/#83).
- [ ] Event and hook names read from the jar via pz-classfile instead of
      `events.rs` constants (#7).

## Per frame (`GameWindow.logic` / `render`)
- [ ] Input: every winit event → `pz_input::winit::handle`; before
      `UIManager::update` call `bind_input::update(&mut mgr, secs)`, after it
      `bind_input::after_ui_update(&mut mgr)`; apply
      `bind_input::take_window_requests()`; `gp.poll(c)` first (#30/#32).
- [ ] Sound: `SoundManager` update and emitter ticks through
      `sound_system().with(..)`; listener from the active camera (#78/#83).
- [ ] `UITransition.UpdateAll` → `bind_misc::ui_transition_update_all()` (#20).
- [ ] Vsync from Core options; `FrameParams` size from Core screen size.

## World (`GameLoadingState` / `IngameState`)
- [ ] `ChunkStreamer::new(players, calc_chunk_width(w, h))`, `load_world`,
      per-frame `update(..)`, `take_events()`, `save_now()` on quit (#64);
      wrap its ChunkLoadHook in `ClearSoundsOnUnload` (#71) and use
      `pz_worldgen::WorldGenHook` as BrandNewHook (#63).
- [ ] CGlobalObjects `initSystems` and update pumps (#31).
- [ ] Popman: `WorldPopulationHost::from_state`, `update_main` with
      `player_chunk_maps(&streamer)`; supply SittingChecks,
      `has_moving_objects`, `line_clear_collide` (#77, #60).
- [ ] World sound: `WorldSoundAdapter::from_state`; engine `SoundEffects`
      (OnWorldSound, fish noise, popman) (#71).
- [ ] Lighting: `WorldLighting::set_chunk_map` for every local player (#76).
- [ ] Fire: `IsoFireManager::update(FireWorld, rand, FireFrame)` once per
      frame; FireHooks; fire_updated/removed via FireLightLinks (#89).
- [ ] States: `StatesWorldAdapter` + engine `StatesEngine` (#88).
- [ ] Vehicles ↔ pathfinding: `pz_vehicles::pathfind_adapter!(pz_pathfind)`
      (#86).

## World render pass
- [ ] Water/sky: `SkyBox::render`, `Surfaces.sky` from `reflection()`;
      `SurfaceChecks::run` + `apply_surface_checks` before `plan_chunks`;
      water flow from lot directories; splash state (#53).
- [ ] `render_animated_attachments` per chunk between water and
      translucent passes (#76).
- [ ] `render_weather_fx` through `world_frame_into_with`, driven by
      `ClimateRenderValues` (#81); FxMask character hook replays #79
      ObjectRender draws without shadows.
- [ ] Feed corpses, world items and fireplace state from pz-world (#87, #76).

## Cleanups
- [ ] Fold bind_vehicles' item subset into the full item bindings (#86).
- [ ] Convert remaining bare prints to DebugLog (#66 meta_grid.rs, #67
      meta_zones.rs/zone_save.rs) (#69).
