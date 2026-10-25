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
- [ ] Character simulation context: `bind_character_sim::install_sim_context`
      with the engine's `SimContext` (lore, climate, `Rand`, events) (#41).
- [ ] Game time and climate: `bind_climate::install_game_time` and
      `set_climate_settings` from the sandbox options (#49);
      `bind_climate::install_event_sink` routing to
      `LuaEventManager.triggerEvent` (climate, `OnInitSeasons` and clock
      events reach Lua only through it).
- [ ] Zombie population: a `pz_popman::native::Native` per world (seeded,
      wall clock) for `WorldPopulation::init`; the outfit names are
      `PersistentOutfits.getOutfitNames()` (not ported yet: the outfit ids in
      `zpop` records reach `RealZombies` unmapped, i.e. without
      `PersistentOutfits.getOutfit`'s variant re-roll).
- [ ] Meta population: `install_population` with
      `shared_population().init(fs)` (#67); popman's
      `bind_popman::install_population` (#73).
- [ ] World sounds: `bind_zombie::install_world_sounds` (#75).
- [ ] States world: `bind_states::install_state_world` (#74).
- [ ] Perks: `PerkFactory::init` before Lua runs (#26).

- [ ] Perks initialised before Lua loads (#26).
- [ ] BodyLocations.lua runs at boot so the "Human" body-location group and
      clothing hide/alt rules exist (#26).
- [x] UI avatars: `pz_engine::ui_models::install(game_dir, fs)` before Lua
      loads (installs the `bind_ui3d::UIModelRenderer`); `TextManager` init
      (`bind_text::boot`) in the engine boot (#115).
- [ ] ScriptManager loading in the windowed engine boot (only the client's
      `--png`/`--headless-frames` paths load scripts) (#115).

## Per frame (`GameWindow.logic` / `render`)
- [ ] Input: every winit event → `pz_input::winit::handle`; before
      `UIManager::update` call `bind_input::update(&mut mgr, secs)`, after it
      `bind_input::after_ui_update(&mut mgr)`; apply
      `bind_input::take_window_requests()`; `gp.poll(c)` first (#30/#32).
- [ ] Sound: `SoundManager` update and emitter ticks through
      `sound_system().with(..)`; listener from the active camera (#78/#83).
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
      `ZombieOps` (`RealZombies`) (#73/#77): `GameWindow.logic` calls
      `WorldPopulation::update_game_state(no_save, paused)`; `IngameState.update`
      (after `VirtualZombieManager.update`) calls `WorldPopulation::update(&mut
      WorldPopulationHost::from_state(.., &mut RealZombies { .. }),
      &PopulationFrame { multiplier, world_age_hours,
      chunk_maps: &player_chunk_maps(&streamer), real_zombies:
      &real_zombies.infos() })`; it also runs the in-line worker
      (`MCDThread`) pass when due.
- [ ] `WorldSoundManager::update` once per frame (#75).
- [x] UI avatars: `UiModels::set_time_delta` (from `GameTime` once it is
      wired; the frame multiplier / 30 now) and `UiModels::prepare(gpu)`
      (texture compositing) before the UI renders (#115).

- [ ] Text entry: `bind_uitextbox2::update_keyboard(&mut mgr.thread)` once
      per frame (`Core.DoFrameReady` → `Core.updateKeyboard`), after the
      window's key/char events are queued (#119).

## World (`GameLoadingState` / `IngameState`)
World load, in the original's order (`GameLoadingState.enter` loads the
server Lua first, so `ClimateMain.lua`'s `OnClimateManagerInit` handler is
registered):
- [ ] `GameTime.setInstance(new GameTime())`: `bind_climate::install_game_time`.
- [ ] `ClimateManager.setInstance(new ClimateManager())`:
      `bind_climate::new_climate_manager(thread)` (fires
      `OnClimateManagerInit`; `ClimateMain.lua` sets the season colours, without
      which the global light stays black).
- [ ] `IsoWorld.init`: `GameTime.init` then `GameTime.load`
      (`game_time_save::load_map_t_file`; the climate part keeps the colours);
      `ErosionGlobals.Boot` → `bind_climate::start_erosion_season(thread,
      &SeasonConfig, sandbox Temperature)` (`OnInitSeasons`); after
      `metaGrid.CreateStep2`, `bind_climate::init_climate_manager(thread,
      Some((meta.min_x, meta.min_y, meta.max_x, meta.max_y)))`.
- [ ] `IsoWorld.init`, after `OnLoadedMapZones` and before the cell load:
      `pz_world_popman::WorldPopulation::init(native, &meta,
      &save_game_state(save_dir) + sandbox/mode fields, &ZombieConfig,
      outfit names)` (`MapCollisionData.init` then
      `ZombiePopulationManager.init`; reads `zpop_ids.bin`/`zpop_virtual.bin`,
      each `zpop_X_Y.bin` when a loaded area first touches its cell).
- [ ] Every chunk the streamer hands off (`StreamEvent::HandedOff`,
      `doLoadGridsquare`): `WorldPopulation::add_chunk_to_world(&cell, &meta,
      &sprites, wx, wy)`; every `StreamEvent::Unloaded`:
      `WorldPopulation::remove_chunk_from_world(&mut host, wx, wy,
      &real_zombies.chunk_zombies(wx, wy, in_room))` (needs the chunk's rooms:
      decide `in_room` before the chunk leaves the cell).
- [ ] After the cell load: `ClimateManager.postCellLoadSetSnow()`.
- [ ] `IngameState.enter`, world loaded: `WorldPopulation::start_game(&mut
      host, &frame)` (`MapCollisionData.startGame`).
- [ ] Quit: `WorldPopulation::save(&reals)` with the world save, then
      `WorldPopulation::stop()`.

- [ ] Sandbox: on loading a save, `SandboxOptions::load_bin(map_sand.bin)` on
      the constructed instance (preset name goes to the world), then
      `handleOldZombiesFile1`, `applySettings` and `toLua` as `load()` does;
      on save/new game write `save_bin(save_bin::world_version(&jar), preset)`
      to `map_sand.bin` (#21).
- [ ] `ChunkStreamer::new(players, calc_chunk_width(w, h))`, `load_world`,
      per-frame `update(..)`, `take_events()`, `save_now()` on quit (#64);
      wrap its ChunkLoadHook in `ClearSoundsOnUnload` (#71) and use
      `pz_worldgen::WorldGenHook` as BrandNewHook (#63).
- [ ] Chunk vehicles: in the streamer's `DoChunk`, right after `load_chunk`
      / `load_brand_new` succeeds and before `finish_chunk_load`, call
      `save::vehicles_db::load_chunk(vehicle_store.conn.as_mut(), &mut chunk,
      &save_ctx)` (`WorldStreamer.DoChunkAlways`); pass `meta: Some(grid)` in
      `SaveCtx` for every save chunk so squares get `associatedBuilding`
      (#100).
- [ ] Local player at world init: `VehicleStore::create` (`VehiclesDB2.init`),
      `savefile::is_player_alive`, `PlayerDb::new`; `load_player_for_info`
      gives `SWorldX/SWorldY` for the chunk map; later `load_local_player_1`
      (decode with `load_player`, health 0 when `is_dead`, set `sqlId`) and
      `chunkMap.worldX/Y = local_player_chunk_map_pos(x, y, offset)` (#100).
- [ ] CGlobalObjects `initSystems` and update pumps (#31).
- [ ] Popman: `WorldPopulationHost::from_state`, `update_main` with
      `player_chunk_maps(&streamer)`; supply SittingChecks,
      `has_moving_objects`, `line_clear_collide` (#77, #60). Driven through
      `WorldPopulation` (see the world-load list above).
- [ ] World sound: `WorldSoundAdapter::from_state`; engine `SoundEffects`
      (OnWorldSound, fish noise, popman) (#71).
- [ ] Lighting: `WorldLighting::set_chunk_map` for every local player (#76).
- [ ] Fire: `IsoFireManager::update(FireWorld, rand, FireFrame)` once per
      frame; FireHooks; fire_updated/removed via FireLightLinks (#89).
- [ ] States: `StatesWorldAdapter` + engine `StatesEngine` (#88).
- [ ] Zombie vehicle checks (`isVehicleBetween`, `DistToSquared(vehicle)`)
      read `IsoChunk.vehicles.loaded()` (#88/#91); once the vehicle runtime
      replaces those entries with its own objects, route these through
      the cell's vehicle list instead.
- [ ] Erosion: `ErosionMain::new(tables from jar, ErosionSettings from
      sandbox/GameTime, sprites, None = host time zone, OnInitSeasons hook)` at
      world load; pass `ErosionChunkHook { main, hooks: &ObjectHooks,
      dictionary: Some(&WorldDictionary), rand, logs_per_size }` to
      `finish_chunk_load` (it also gets `chunk_unloaded`); `main_timer(day,
      month, year)` on GameTimes ten-minute tick; bind `bind_erosion::
      set_instance` for Lua (#42).
- [ ] Pathfinding: one `PathfindService::new(meta min_x, min_y, width, height, is_server)` in a
      `pz_pathfind::host::PathHost` per world. On chunk load/unload
      `add_chunk_to_world(chunk, load_id, &SquareBits)` /
      `remove_chunk_from_world`, `square_changed` on square edits (SquareBits
      over the cell with `pz_world::ObjectHooks`); per frame, before characters
      update, `update_thread()` (game runs it every ~50 ms while there is work)
      then `update_main(vehicles, seating, movers)`; `remove(id)` when a
      character leaves; postupdate calls `PathHost::resolve_collision(_radius)`
      on the same host (#60).
- [ ] Vehicles: `world_sim::set_world_bounds` + `create()` at world load
      (`destroy()` on exit) with a `VehiclePhysics` impl (pz-bullet once it
      lands; `NoPhysics` until then); `manager::process_add_vehicles()` and
      `update_frame()` per frame; `spawn` (`IsoChunk.AddVehicles`) from the
      chunk loader through a `SpawnWorld` impl fed by the meta cell vehicle
      zones; `bind_vehicles::register` (#86).
- [ ] `StateWorld` impl owners: characters by id, squares, PathHost
      (`path_*`, `path_cancel_request` → `PathfindService::cancel_request`,
      `resolve_collision`, `resolve_collision_radius`,
      `test_collide_adjacent` with the special objects), LOS, world sounds,
      corpses, thumpables, `test_collide_adjacent_mover`, `hoppable_actions`, `fence_climb` (pz-world-states implements all three), `rustle_vegetation` (render effects, draws
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
- [ ] Chunk physics after square edits: once per frame, drain the world
      renderer's `PendingRender::physics` into
      `pz_vehicles::chunk_physics::ChunkPhysics::mark_physics_dirty(cell, ..)`
      (`IsoChunk.checkPhysicsLater`); `ChunkPhysics::update_if_needed` then
      rebuilds them from Bullet's step callback (#86, #99).
- [ ] Vehicles ↔ pathfinding: `pz_vehicles::pathfind_adapter!(pz_pathfind)`
      (#86).
- [ ] Vehicle models: `VehicleModelSet::create` when a vehicle is added to
      the world (`ModelManager.addVehicle`), `sync` after part model
      visibility changes, `update_animations(unmoddedMultiplier)` in
      `postupdate` (#90).
- [ ] Vehicle lights → lighting: `WorldHooks::torches` returns the
      players' light-item torches, then
      `pz_vehicle_render::lights::cell_torches(cell vehicles)`
      (`LightingJNI.checkLights`; pz-lighting's `check_lights` sends
      z + 32 and removes stale torches). Per vehicle a
      `lights::LightbarWorldLights::new` (`createParts`); the engine's
      `pz_vehicles::env::VehicleEvents` impl forwards
      `lightbar_world_lights(vehicle, state)` (`updateWorldLights`, from
      `BaseVehicle.update`) to that vehicle's `LightbarWorldLights::apply(
      vehicle, state, &mut cell's WorldLighting)`, and calls `remove` on
      `removeFromWorld`; it adds/removes lampposts through
      `WorldLighting::add_lamppost`/`remove_lamppost` (#90).
- [ ] Vehicles: `world_sim::set_world_bounds` + `create()` at world load
      (`destroy()` on exit) with a `VehiclePhysics` impl (pz-bullet once it
      lands; `NoPhysics` until then); `manager::process_add_vehicles()` and
      `update_frame()` per frame; `spawn` (`IsoChunk.AddVehicles`) from the
      chunk loader through a `SpawnWorld` impl fed by the meta cell vehicle
      zones; `bind_vehicles::register` (#86).
- [ ] Player load (`PlayerDB.loadLocalPlayer`/`loadPlayer`): the bytes of
      `PlayerDb::load_local_player(sql_id)` (#100) go to
      `GameCharacter::load_player(sm, chars, ctx, SaveEnv { items:
      ScriptItems over the save's WorldDictionary, tables }, reader,
      world_version, set_on_fire)` on a fresh `GameCharacter::new(.., Player)`
      with a descriptor; `set_on_fire` runs `pz_world::fire::character::set_on_fire`
      with the world's fire manager. Then, for an `isDead` row,
      `setOverallBodyHealth(0)`/`setHealth(0)`; `saved_vehicle` seats the
      player; save with `save_player` (#101).
- [ ] Square mutations: every object change after load goes through
      `pz_world::square_mutation::MutationEnv` (`add_tile_object`,
      `add_special_object`, `remove_tile_object`,
      `transmit_remove_item_from_square`, `add_floor`,
      `remove_tile_object_erosion_no_recalc`, `add_corpse` / `remove_corpse`,
      `create_new_grid_square`) with `num_players` and
      `DoorTables::from_jar` (#97).
- [ ] Collision code fills `IsoChunk::collision` through
      `ChunkCollision::init(z, build)` (`CollideWithObstaclesPoly` /
      `LineClearCollide`) and drops levels with `take` on vehicle moves; the
      mutations clear it (#97).
- [ ] Pathfinding thread implements `PathfindSquareHooks`:
      `square_changed(cell, pos)` (PathfindNative square tasks for the
      square and its neighbours, bits read from `cell` at the call),
      `add_chunk_to_world(cell, wx, wy)` (ChunkUpdateTask buffer built from
      `cell`), `map_collision_square_changed` (popman
      `MapCollisionData` bits), `uses_native_code` from the debug option
      (#97).
- [ ] Render side implements `RenderSquareHooks`: render-chunk / vispoly
      invalidation, `FBORenderCutaways.squareChanged`,
      `LosUtil.cachecleared`, `recalcLightTime`, `Core.dirtyGlobalLightsCount`,
      `lightSourceUpdate`, `IsoGridOcclusionData`, `fbo_render_chunk`,
      `properties_recalculated` (puddles, `checkLightingLater`, water;
      `IsoChunk` keeps `physicsCheck` itself), `clear_water`,
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
- [ ] Bullet world ownership: the world/engine owns the one `PzBullet`
      (`WorldSimulation`) and counts its steps (`getBulletFrameNo`); the
      characters reach it through `StateWorld::physics()` (`PzBullet`
      implements `pz_characters::character::firearm::PhysicsWorld` with
      the `bullet` feature) (#114).
- [ ] Firearm hooks on `StateWorld`: `muzzle_attachment(chr)` (the
      primary hand model's MUZZLE attachment world position/direction),
      `aim_reticle_iso(chr)` (`AimingReticle` x/y through `XToIso/YToIso`),
      `ballistics_skeleton(chr)` (bone count, `getAngle()`, packed bone
      transforms), `first_blocking_square` (`LosUtil` for tracers),
      `square_is_outside`, `vehicle_aim`, `ragdoll_pelvis(chr)` (the
      pz-ragdoll controller's pelvis); `SimContext::wind_intensity`, the
      firearm sandbox options and `active_ragdoll_simulations`
      (`RagdollGlobals.number_of_active_simulations`) (#114).
- [ ] Per frame, ballistics (`IsoGameCharacter.updateInternal` order):
      each non-animal character `update_ballistics_target(ctx, world)`;
      a character aiming an aimed firearm (`getUseHandWeapon`) runs
      `setAngleFromAim` then `update_ballistics(sm, chars, ctx, world)`;
      `CombatManager.update` for player 0 with a ready ranged weapon and
      the reticle mode: `calculate_attack_vars` + `calculate_ranged_hit_info_list`
      (`updateReticle`; the reticle chance comes from
      `calculate_hit_chance_data(cx, w, None, vehicle)`); in the
      character's postupdate `ballistics_controller.post_update()`; on
      release `release_ballistics_controller` / `release_ballistics_target`.
      Physics steps after the character updates (#114).
- [ ] Shots: the anim event `AttackCollisionCheck` runs the ranged path
      of `attack_collision_check`; Lua's `ISReloadWeaponAction.onShoot`
      (`OnWeaponSwingHitPoint`) spends the round through the bound
      HandWeapon methods; `SimEvent::BulletTracer`, `MuzzleFlash`,
      `BulletHitSound`, `CharacterAddBlood`, `CombatStatistic`,
      `ReticleMode` go to the effects, sound, visuals and statistics
      (#114).
- [ ] Per frame, ragdolls: before the animation update call
      `update_ragdoll_variables(ctx)` (the `canRagdoll` variable the death
      and shot nodes test); once pz-ragdoll (#102) lands, its
      `RagdollCharacter` reads `can_ragdoll_uncapped`, the fall flags and
      `take_ragdoll_hit(sm, attacker position)` (the shot impulse from the
      ballistics target's combat data and `PhysicsHitReactionScript`), and
      after the update the controller state goes into `chr.ragdoll`
      (`RagdollTracking`) (#114).

## World render pass
- [ ] Cull chunk textures to the screen (`plan_chunks` / `level_on_screen`);
      allocating every loaded chunk exhausts the GPU device.
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
- [ ] Vehicles in `FrameObjects.moving` (cell object-list order): per
      vehicle `update_transform`, `instance.update_lights`,
      `instance.do_damage_overlay` (when `doDamageOverlay` is flagged or the
      blood-decal option changes), then `render_vehicle_object` with a
      `VehicleFrame` (alpha, `interpolateLight` ambient, the square's
      result lights as `SquareLight::from_result_lights(
      WorldLighting::result_lights(player, x, y, z))`, square light/seen,
      sky box textures + shift, `media/vehicleShadow.png`) (#90).
- [ ] Character model lights: per character and player
      `PlayerLightData::update(&LightContext { result_lights:
      WorldLighting::result_lights(player, square).to_vec(), .. })`
      (`ModelInstance.updateLights`), then `effect_lights_main` /
      `current_ambient` into `CharacterFrame` (#90).
- [ ] Vehicle reflections: the reflective vehicle shaders need
      `VehicleFrame`'s sky-box textures and shift from #53's `SkyBox`
      (pz-iso-render `skybox.rs`, on a separate stack). Until that is
      wired, load vehicle models with `reflections: false` (the
      `_noreflect` shader variants, as with `bPerfReflections` off) (#90).
- [ ] Character outlines: per character, `ObjectRender.outline =
      OutlineHighlight::character_request(playerIndex, x, y, isProne,
      camCharacterX, camCharacterY)` from the object's highlight state
      (`IsoObject.setOutlineHighlight`/`Col`: CombatManager's aim targets in
      `Core.getBadHighlitedColor`, the debug melee outline cyan at 0.65,
      Lua); `local_player = true` on the viewing player's object;
      `renderlast` clears the highlight. `world_frame_with_objects` runs
      `startFrameMain`/`endFrameMain` around the object and translucent
      passes (#94).
- [ ] Held/attached item textures: once the slot is built (and each frame
      before drawing it) `item_texture::render_equipped_textures_with(slot,
      exists, file, gpu, tm)` renders the dirty initializers (tint/TINT
      mask, weapon blood, fluid); on `postUpdateEquippedTextures` (blood or
      fluid changed) call `inst.refresh_texture_init(held.texture_init())`
      on the hand models; build `HeldItem`s `with_fluid(ctx)` (#94).
- [ ] Placed items: `world_item::item_params_with_fluid` with
      `item.get_fluid_container(ctx)` (else the world item's) for the
      `_Fluid` model and the fluid overlay; draw the atlas jobs with
      `WorldItemAtlas::render_with(out, |p| world_item::atlas_smart_texture(..))`
      and hand the returned textures to `Combiner::release` after
      `render_frame` (#94).

## Cleanups
- [ ] Fold bind_vehicles' item subset into the full item bindings (#86).
- [ ] Convert remaining bare prints to DebugLog (#66 meta_grid.rs, #67
      meta_zones.rs/zone_save.rs) (#69).
