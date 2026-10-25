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

## Per frame (`GameWindow.logic` / `render`)
- [ ] Input: every winit event → `pz_input::winit::handle`; before
      `UIManager::update` call `bind_input::update(&mut mgr, secs)`, after it
      `bind_input::after_ui_update(&mut mgr)`; apply
      `bind_input::take_window_requests()`; `gp.poll(c)` first (#30/#32).
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
- [ ] Square mutations: every object change after load goes through
      `pz_world::square_mutation::MutationEnv` (`add_tile_object`,
      `add_special_object`, `remove_tile_object`,
      `transmit_remove_item_from_square`, `add_floor`,
      `remove_tile_object_erosion_no_recalc`, `add_corpse` / `remove_corpse`,
      `create_new_grid_square`) with `num_players` and
      `DoorTables::from_jar` (#97).
- [ ] Pathfinding thread implements `PathfindSquareHooks`: `square_changed`
      (PathfindNative square tasks for the square and `adjacent`),
      `add_chunk_to_world`, `map_collision_square_changed` (popman
      `MapCollisionData` bits), `uses_native_code` from the debug option
      (#97).
- [ ] Render side implements `RenderSquareHooks`: render-chunk / vispoly
      invalidation, `FBORenderCutaways.squareChanged`,
      `LosUtil.cachecleared`, `recalcLightTime`, `Core.dirtyGlobalLightsCount`,
      `lightSourceUpdate`, `IsoGridOcclusionData`, `fbo_render_chunk`,
      `properties_recalculated` (puddles, `checkLightingLater`,
      `checkPhysicsLater`, collision cache, water), `clear_water`,
      `buildings_changed` (`LightingJNI.buildingsChanged`),
      `chunk_buildings_changed` (#97).
- [ ] Engine implements `SquareMutationHooks`: Lua `OnObjectAboutToBeRemoved`
      / `OnTileRemoved` / `OnContainerUpdate`, the `WorldCall` registries of
      `addToWorld` / `removeFromWorld` (entities, process lists, lampposts,
      emitters, fire manager, generators, radio, hutches, mannequins, world
      items), the power / fluid / piped-fuel / sprite-config queries,
      `FMODAmbientWalls`, `DesignationZoneAnimal.addNewRoof`, corpse
      registries, room lights, `IsoRoom.clear` (#97).
- [ ] Regions: call `IsoRegions::update_with_buildings(BuildingCtx { .. })`
      once per frame instead of `update`, so player rooms become meta-grid
      buildings (`WorldRegionToMetaGrid`) (#97).

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
