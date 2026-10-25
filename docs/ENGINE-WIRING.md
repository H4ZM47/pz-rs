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
- [ ] Event and hook names read from the jar via pz-classfile instead of
      `events.rs` constants (#7).
- [ ] Character simulation context: `bind_character_sim::install_sim_context`
      with the engine's `SimContext` (lore, climate, `Rand`, events) (#41).
- [ ] Game time and climate: `bind_climate::install_game_time` and
      `set_climate_settings` from the sandbox options (#49).
- [ ] Meta population: `install_population` with
      `shared_population().init(fs)` (#67); popman's
      `bind_popman::install_population` (#73).
- [ ] World sounds: `bind_zombie::install_world_sounds` (#75).
- [ ] States world: `bind_states::install_state_world` (#74).
- [ ] Perks: `PerkFactory::init` before Lua runs (#26).

## Per frame (`GameWindow.logic` / `render`)
- [ ] Input: every winit event → `pz_input::winit::handle`; before
      `UIManager::update` call `bind_input::update(&mut mgr, secs)`, after it
      `bind_input::after_ui_update(&mut mgr)`; apply
      `bind_input::take_window_requests()`; `gp.poll(c)` first (#30/#32).
- [ ] `UITransition.UpdateAll` → `bind_misc::ui_transition_update_all()` (#20).
- [ ] Vsync from Core options; `FrameParams` size from Core screen size.
- [ ] `GameTime::update` / `update_game_time(thread)` once per frame
      before the world (#49).
- [ ] Pathfinding: PathHost `update_thread` then `update_main` before the
      characters update; `host.remove(id)` when a character leaves (#74).
- [ ] Characters, in game order: `preupdate_zombie` (players: their
      preupdate), `update_zombie` (the state machine, then `update_fire`),
      `postupdate_zombie` / `moving_postupdate` (with
      `slide_head_away_from_walls`) (#41/#74/#75).
- [ ] `ZombiePopulationManager::update_main` with PopulationHost and
      `ZombieOps` (`RealZombies`) (#73/#77).
- [ ] `WorldSoundManager::update` once per frame (#75).

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
- [ ] Pathfinding: one `PathfindService::new(meta min_x, min_y, width, height, is_server)` in a
      `pz_pathfind::host::PathHost` per world. On chunk load/unload
      `add_chunk_to_world(chunk, load_id, &SquareBits)` /
      `remove_chunk_from_world`, `square_changed` on square edits (SquareBits
      over the cell with `pz_world::ObjectHooks`); per frame, before characters
      update, `update_thread()` (game runs it every ~50 ms while there is work)
      then `update_main(vehicles, seating, movers)`; `remove(id)` when a
      character leaves; postupdate calls `PathHost::resolve_collision(_radius)`
      on the same host (#60).
- [ ] `StateWorld` impl owners: characters by id, squares, PathHost
      (`path_*`, `path_cancel_request` → `PathfindService::cancel_request`,
      `resolve_collision`, `resolve_collision_radius`,
      `test_collide_adjacent` with the special objects), LOS, world sounds,
      corpses, thumpables, `rustle_vegetation` (render effects, draws
      `Rand`), hoppables (`collided_with_hoppables`: fence climbing)
      (#74/#75).
- [ ] Character fire: `StateWorld::character_fire` runs
      `pz_characters::character::fire::update_character_fire` with the
      world's `IsoFireManager`/`FireWorld` (#89).
- [ ] States collision (#95): `StatesEngine::collide_mover` (`CollideMoverInfo::of(chr, path)`
      plus the sit-on-furniture square and `adef.canClimbFences`),
      `hoppable_actions` → `setThumpTarget` / `climbOverFence`; after a
      `test_collide_adjacent` hit the character runs `IsoZombie.collideWith`
      on `special_object_between(from, to)`; `bent_fences_enabled` /
      `is_unbent_fence` from BentFences.
- [ ] Saved windows: `IsoChunk::apply_window_load_sprites(&mut sprites)` for
      every chunk the streamer loads, before its squares' properties and
      matrices are recalculated (#95).
- [ ] States path host hooks (#95): `StatesEngine::resolve_collision`,
      `resolve_collision_radius` (PathHost over the `WorldView`) and
      `path_cancel_request` (`PathfindService::cancel_request`).
- [ ] States fire and render effects (#95): `StatesEngine::fire()` (the
      world's `IsoFireManager`, FireHooks, flags, settings);
      `has_render_effect` / `set_render_effect` (ObjectRenderEffects) and
      `do_tree_noises` (tree sound manager); `ThumpSettings::server`.
- [ ] `SimContext` climate values from `ClimateManager` (#49).
- [ ] Square edits → pathfinding: `pz_pathfind::pathfind_square_hooks!(pz_world)`
      in the engine crate, and pass `PathfindSquareHooksImpl(PathfindMutationHooks { service:
      host.service_mut(), sprites, meta, bits_hooks: &ObjectHooks })` as
      `MutationEnv`'s `PathfindSquareHooks`; compile-checked against #97
      d844ab0 (#60, #97).
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
- [ ] Lighting pass reads `ClimateManager::render_values` (#49).

## Cleanups
- [ ] Fold bind_vehicles' item subset into the full item bindings (#86).
- [ ] Convert remaining bare prints to DebugLog (#66 meta_grid.rs, #67
      meta_zones.rs/zone_save.rs) (#69).
