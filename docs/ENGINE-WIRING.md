# Engine wiring checklist

Hooks that subsystem crates expose and the game loop (`pz-engine`,
`pz-client`) must call. Tick each item when it is wired on main.

## Boot (`GameWindow.init`)
- [ ] Mods (only when Core's mods option is on): `bind_mods::read_default`,
      `GameFs::load_mods`, mod texture packs via `mod_pack_files` →
      `load_texture_pack` + `set_texture_pack_lookup` (#24).
- [x] ScriptManager: `load_builtin_registries` + `load`, then
      `bind_registry::install_script_manager` (#22/#27).
- [x] Item assets for `instanceItem`: after the ScriptManager,
      `bind_items::install_item_assets(pz_inventory::ItemAssets::load(game_dir)?)`
      (`ClothingItem` XMLs + `BloodClothingType`; without it items made from
      Lua have no `ItemVisual`). Wired in `pz_engine::boot::load_scripts`,
      used by every client mode. Decals
- [ ] Item assets for `instanceItem`: after the ScriptManager,
      Lua have no `ItemVisual`). Wired in `pz_engine::boot::load_scripts`
      (both client modes). Decals
      (`ItemVisual.getDecal`, `getAllDecalNamesForItem`) also need the meta
      population line below (`ClothingDecals`). A character simulation
      that installs its own `ItemLuaEnv` must return these assets from
      its context (#130).
- [x] Item entity registries: `bind_items::install_item_runtime(assets,
      item_entity_context(jar, sm))`, so items made from Lua get their
      factory components (`FluidContainer`); wired in
      `pz_engine::boot::load_scripts`. A character simulation's own
      `ItemLuaEnv` must return them from `entity()` (#143).
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
      `PersistentOutfits.getOutfitNames()`, what
      `bind_zombie_outfits::init_persistent_outfits` returns (see the
      world-load list). The outfit ids in `zpop` records reach `RealZombies`
      as `persistent_outfit_id`; dressing resolves them (#147).
- [ ] Meta population: `bind_appearance::install_population` of a
      `Population::init(fs)` (`ClothingDecals`, `BeardStyles`, `HairStyles`,
      `OutfitManager`, `VoiceStyles`, `PopTemplateManager`) right after
      `ScriptManager.Load`, in `GameWindow.initShared`'s order (#67; wired in
      `pz_engine::boot::load_scripts`). HumanVisual's lazy hair/beard/skin
      getters, `UI3DModel.setSurvivorDesc`/`setOutfitName` and
      `SurvivorDesc:dressInNamedOutfit` read it. Still open: popman's
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

- [x] `UIManager.init()` is `pz_lua_api::hud::init_ui(&mut thread)` (builds
      `SpeedControls`; the in-game HUD when a player exists) — boot uses it
      (#144).
- [ ] HUD inputs: `pz_lua_api::hud::host::install(HudHost { players,
      game_time, sound, network })` — `HudPlayers` (IsoPlayer statics,
      characters thread), `HudGameTime` (characters thread; defaults to the
      shared Lua `GameTime`), `HudSound` (`SoundManager`, audio thread),
      `HudNetwork` (`GameClient`/`GameServer`) (#144).
- [x] HUD players, game time, climate and moodles:
      `hud::characters::install_character_hud_hosts()` (run by
      `register_all`) reads `bind_players` (`IsoPlayer.players`,
      `numPlayers`, `getInstance`), the shared `GameTime`/`ClimateManager`
      and the players' `Moodles`.
- [ ] Local players: when a player is created / loaded (`IsoPlayer`
      constructor, `GameLoadingState`) call `bind_players::set_player(i,
      Some(chr))`, `set_player_instance`, `set_num_players`; each frame for
      the local player (`IsoPlayer.updateInternal1`)
      `hud::set_moodles_character(i, Some(i))`; `bind_players::reset()` when
      leaving the game. Then `UIManager.init` (`hud::init_ui`) builds the
      in-game HUD.

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
- [ ] Player input: after the camera update call
      `pz_player_input::set_camera_info(i, CameraInfo { off_x, off_y,
      right_click_x, right_click_y })` per local player (the reticle
      position is `chr.player.aiming_position`, set by
      `chr.update_aiming_reticle(&input, multiplier)`); after
      `bind_input::update`/`after_ui_update` the game loop builds one
      `EnginePlayerInput::capture()` per frame, passes `&input` to each local
      player's `IsoPlayer.update`, then calls `input.write_back()` (#137).
- [ ] `ZombiePopulationManager::update_main` with PopulationHost and
      `ZombieOps` (`RealZombies`) (#73/#77; `RealZombies::create_survivor`
      returns `CreateSurvivor`'s `ScriptResult`, and `WorldPopulation::update`
      / `start_game` return `Err(java exception text)` when creating a real
      zombie threw (#147): `IngameState.update` has try/finally with no
      catch (IngameState.java:640–650), so an `Err` aborts the rest of that
      frame's `IngameState.update` and is logged via `ExceptionLogger`; the
      popman worker pass has already run, as its own thread would): `GameWindow.logic` calls
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

- [x] Title video: `MainScreenState.render_background` plays
      `title_screen_background.bik` through `bind_video::VideoTexture`
      (`render_frame` uploads each due frame with
      `TextureManager::tex_image`; `pz-gpu` re-uploads on
      `TextureData::version`). Still to wire: `MainScreenState.exit` →
      `exit_video()`, and the `enter()` check that turns video effects off
      below 8 GB of RAM. Headless runs that want the video to advance
      should step `pz_video::with_manager(|m| m.set_clock(..))` with the
      frame time (#159).

- [x] Text entry: `bind_uitextbox2::update_keyboard(&mut mgr.thread)` once
      per frame (`Core.DoFrameReady` → `Core.updateKeyboard`), after the
      window's key/char events are queued (#119).

## World (`GameLoadingState` / `IngameState`)
- [ ] `GameLoadingState.enter`: `bind_video::release_all_video_textures()`
      (`LuaManager.releaseAllVideoTextures`, after `WorldMapImages.Reset` /
      `WorldMapVisited.Reset`) closes and destroys every texture `getVideo`
      returned (#159).
- [ ] Character inventories: hold the inventory as a
      `pz_inventory::ContainerRef` (`SharedContainer::from_container` on
      load, `to_container` on save), return
      `bind_item_container::container_value(&inv)` from `getInventory()` and
      call `bind_item_container::set_parent(&inv, character_lua_value)`. The
      container then calls `getInventoryWeight`, `getMaxWeight`,
      `isUnlimitedCarry` (`isGhostMode` in debug), `hasTrait` and
      `removeFromHands` on that Lua object; world containers need their
      `IsoObject` parent set the same way (#143).
- [ ] Item owners: a character's inventory parent goes in with
      `bind_item_container::set_parent_with_owner(&inv, character_lua_value,
      owner)`, `owner` an `ItemOwner` answering `chr.isEquipped(item)`,
      `getWornItems().contains(item)`, the hand items and `place` for the
      character's items (`CharacterItemOwner` over the `GameCharacter` impl
      in `pz-characters/src/character/item_owner.rs`; done for characters
      through `character_value`, #179). An `ItemLuaEnv` returns
      `bind_item_container::container_owner(item)` from `owner` (as
      `GameItemEnv` does), so `isEquipped`/`isWorn`/`isInPlayerInventory`
      follow `item.container.parent` (bags).
- [ ] Inventories loaded into an existing container must make items with
      `pz_inventory::shared::new_item` (or `SharedContainer::fill_from`) so a
      bag's inventory gets its `containingItem` (bags).
- [ ] End of `GameLoadingState`: `hud::init_ui(&mut thread)` (the in-game
      HUD, then `OnCreateUI`) before `OnCreatePlayer` (#144).
- [ ] `IngameState` per player: `hud::update_progress_bar(n)` in the update
      and `hud::render_progress_bar(&mut thread, n)` after the player's
      game-world UI; `HudPlayers::player_screen` (characters thread) and
      `HudCamera` (`IsoCamera.getOffX/Y`, `Core.getZoom`; world-render
      thread) feed its position (#145).
- [ ] Clock inputs: `HudAlarmClocks` (players' worn and inventory
      `AlarmClock`/`AlarmClockClothing`, stop/set alarm; world thread),
      `HudClimate::air_temperature_for_character` and
      `HudGameTime::time_of_day/day/month` and `digits_time_of_day` (the
      digits' `serverTimeOfDay` swap while an MP client fast-forwards)
      (characters thread),
      `HudPlayers::player_dead`, `HudDebug::cheat_clock_visible`. Call
      `UIManager::resize()` when a player's clocks change (the original
      re-runs `resize` from its callers) (#146; digits split #164).
- [ ] Moodles: `hud::set_moodles_character(i, Some(player_index))` when a
      player is created or removed (`IsoPlayer` sets
      `UIManager.getMoodleUI(i).setCharacter`); `HudMoodles` (a player's
      `Moodles`: refresh flag, levels, good/bad, display and description
      strings; characters thread) (#148).
- [x] Character inventories: `CharacterItems::inventory` is a
      `pz_inventory::ContainerRef` (loaded in place with
      `SharedContainer::fill_from`, saved via `to_container`);
      `bind_character_sim::character_value` sets the character's Lua object
      as its parent once and `getInventory()` returns
      `bind_item_container::container_value(&inv)`, so Lua and the
      simulation share the container and items. Characters handed to Lua
      must go through `character_value` (#143, char-inventory PR).
- [ ] Worn items: characters made outside `load_player`,
      `new_survivor` and `new_zombie` (an `IsoPlayer` constructor) must call
      `GameCharacter::init_worn_items(sm, BodyLocations.getGroup("Human"))`
      or `getWornItems()` is `nil` (#179).
- [x] Item-side `isEquipped()`/`isWorn()` for character items:
      `character_value` (and `setWornItem`) set the inventory's parent with
      `set_parent_with_owner(&inv, character_lua_value,
      Rc::new(CharacterItemOwner::new(&chr)))`, so `GameItemEnv::owner`
      (`container_owner`) reaches the character's hands and `WornItems`
      (#179).
- [ ] World containers need their `IsoObject` parent set with
      `bind_item_container::set_parent` the same way (#143).
- [ ] Shared tooltip: `HudPicking::picked_tile` (`UIManager.getPicked().tile`
      as a `pz_ui::tooltip::TooltipObject`: identity, `haveSpecialTooltip`,
      its square's Lua object; world thread); the tooltip font option and
      highlight colours come from `Core` (#153).
- [ ] Health panel: `HudBodyDamage` (`isFemale`, `getBodyDamage().getHealth()`,
      each `BodyPart`'s wound state as a `BodyPartView`; the remote
      `getBodyDamageRemote()` for another MP player; characters thread) and
      `HudDebug::ui_render_outline` (`Core.debug` plus the debug option)
      feed `NewHealthPanel`/`UI_BodyPart`, built by `ISNewHealthPanel` (#155).
      `HudBodyDamage` for local characters is `CharacterBodyDamage`
      (installed by `install_character_hud_hosts`); the remote path is
      TODO(network).
- [ ] Debug console (built in `hud::init_ui`): with `Core.debug`, the
      `TOGGLE_LUA_CONSOLE` key toggles `UIManager.debug_console`'s visibility
      in `GameWindow.logic`; `DebugLog`'s stream calls
      `hud::debug_console::debug_log_write(bytes)`; `HudDebug`'s
      `ui_debug_console_start_visible/echo_command/debug_log` options;
      `HudNetwork::may_process_console_commands` (role capability or coop
      host; network thread); `HudConsoleMethods` (GlobalObject's declared
      methods and each return type's public methods, for suggestions;
      ours, from the exposure tables) (#160).
- [ ] Zombies: pass the real `StatesWorldAdapter` and sitting-obstacle
      provider to the zombie state machines each frame, not
      `NullWorld`/`NoSittingObstacles` (found in the e2e run).
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
- [ ] `IsoWorld.init`, after `GameEntityManager.Init` and before
      `metaGrid.CreateStep2` (between the erosion boot and
      `init_climate_manager` above): `PersistentOutfits.instance.init()` =
      `bind_zombie_outfits::init_persistent_outfits(thread,
      Some((&save_dir.join("z_outfits.bin"), core.no_save)))` (Lua's
      `ZombiesZoneDefinition` must have run); keep the returned outfit names
      for `WorldPopulation::init` (#147).
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
- [ ] After the cell load: `ClimateManager.postCellLoadSetSnow()` =
      `bind_climate::post_cell_load_set_snow(thread)`; apply the returned
      `SnowEffects`: `IsoCell.setSnowTarget(target)` (0 unless sandbox
      `EnableSnowOnGround`) and `ErosionIceQueen::set_snow(ice_queen)`. The
      climate update's own snow effects collect in the same slot: apply
      `bind_climate::take_snow_effects()` after `update_game_time` each frame (#147).
- [ ] Zombie dressing (`ModelManager.Add`/`Reset` call
      `ModelManager.dressInRandomOutfit`): when a zombie gets its model,
      `GameCharacter::model_manager_dress_in_random_outfit(&mut
      ZombieDressCx { chars, dress (bind_appearance::with_dress_context),
      outfits: &mut shared_zombie_outfits(), world: &mut LuaDressWorld {
      globals, zones (cell squares' room names, meta grid `getZonesAt`),
      create_item (`bind_items::create_item`), loot_modifier,
      chance_of_attached_weapon (sandbox) }, settings (sandbox
      `ClothingDegradation`, `RemoveZombieLoot`, game mode, world age) })`;
      it fills `human_visual`, `zombie.item_visuals` and attached weapons (#147).
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
- [ ] Chunk vehicles: pass the `VehicleStore` connection to the streamer,
      `StreamCtx { vehicles_db: vehicle_store.conn.as_mut(), .. }`; its
      `DoChunk` then calls `save::vehicles_db::load_chunk` for every chunk it
      serves, from disk or brand new (`WorldStreamer.DoChunkAlways`), and
      reports `StreamEvent::VehiclesLoaded`. `None` (no store,
      `Core.isNoSave`) loads none. The streamer passes `meta: Some(grid)`
      in `SaveCtx`, so squares get `associatedBuilding` (#100).
- [ ] Chunk vehicles on save: every `IsoChunk.Save` the streamer does
      writes the chunk's vehicles (`VehiclesDB2.unloadChunk`,
      `StreamEvent::VehiclesUnloaded`). Set `StreamCtx::vehicle_saver` to
      a saver that knows the attached scripts (`LoadedVehicleSaver {
      scripts }`), later to the vehicle runtime's own saver for vehicles
      in the world; the default writes none (null script, as the original
      for a vehicle never added to the world) (#150).
- [ ] Runtime vehicles on save: `StreamCtx::vehicle_saver =
      Some(&pz_vehicles::vehicles_db::RuntimeVehicleSaver::default())`
      writes the runtime `BaseVehicle`s of a saved chunk's list
      (`VehicleBuffer.set` + `BaseVehicle.save`: `vehicles_db::vehicle_buffer`),
      and hands entries the runtime has not taken over to
      `LoadedVehicleSaver { NoVehicleScripts }`. A vehicle without a script
      or chunk is skipped with the original's NPE; the rest still save.
      Needs: loaded rows taken over with `cell_link::adopt_loaded(..,
      save::from_save)` (sets `sqlId`, `square`/`current` and the saved
      `IsoObject` fields); `sqlId` for new vehicles from
      `VehiclesDB2.setVehicleLoaded` (`allocateID` over `init_used_ids`) in
      `addToWorld` (not ported; an id < 1 is written as is, like the
      shipped game with asserts off); and `IsoChunk.removeFromWorld` before
      the chunk save is queued: per vehicle on its squares
      `remove_from_world`, clear `square`/`current` (the save then does not
      clamp the position), and when `vehicles_db::should_update_in_meta` is
      `Ok(true)` take it off the list and write
      `update_vehicle(vehicle_buffer(v))` (`VehiclesDB2.updateVehicle`, then
      `VirtualVehicleManager.addToMeta`). The quit save's
      `updateVehicleAndTrailer` uses the same `vehicle_buffer`
      (ws/vehicles-db-save).
- [ ] Vehicles leaving with their chunk (`IsoChunk.removeFromWorld`,
      unload and quit) are done by the streamer: with
      `RuntimeVehicleSaver` as `StreamCtx::vehicle_saver`, each vehicle
      `added_to_world` on one of the chunk's squares (`current`) loses its
      loaded id, and when `should_update_in_meta` is true leaves the list,
      gets its row queued (off its square, `square` cleared) and goes to
      `StreamCtx::vehicle_meta` (`VirtualVehicleManager.addToMeta`: the
      vehicle runtime implements `vehicles_db::VehicleMetaHook`). The
      queued rows are written by the streamer's idle loop
      (`StreamEvent::VehiclesUpdated`) and, after `streamer.stop`, by
      `world_save::close_stores(.., streamer.take_vehicle_queue())`.
      Create `vehicles_db::VehicleIds::init(conn)` at `VehiclesDB2.init`,
      pass it as `StreamCtx::vehicle_ids`, and call
      `ids.set_vehicle_loaded(sql_id)` from the runtime's `addToWorld`
      (store the returned id) (#167).
- [ ] Animal paths: on `StreamEvent::GenAnimalsPath { wx, wy }` run
      `ZoneGenerator.genAnimalsPath` (not ported); it calls
      `meta.add_cell_to_save(cell_x, cell_y)` unless the sandbox's
      animal path chance is "none" and registers its zones with
      `register_animal_zone_def` (#167).
- [ ] Hot save: on `StreamEvent::HotSave` (main thread, after the
      streamer step) call `world_save::hot_save_ancillary(&mut HotSave {
      .. })` and set `PlayerDB.forceSavePlayers` when `canSavePlayers`
      (#150).
- [ ] Game save (`GameWindow.save(true)`: sleep, menu save, quit):
      `world_save::save_game(&mut SaveGame { .. }, &mut hooks)`; the
      engine's `SaveGameHooks::save_chunks(meta)` builds its `StreamCtx`
      and calls `streamer.wait_for_saves(ctx)` then
      `streamer.save_chunk_maps(&cell, ctx)`; `save_population` calls
      `WorldPopulation::save`; `on_save` fires Lua `OnSave`. Supply
      `sandbox: &sandbox.save_bin(world_version, preset)`, and from the
      characters side the local players' `IsoPlayer.save` bytes and the
      reanimated zombies' `IsoZombie.save` bytes (#150).
      `SaveGame` also takes `world_gen` (`map_worldgen.bin`),
      `instance_tracker` (`iTrack.bin`; `cache_dir` for
      `ItemTracker.log`), `meta_tracker` (helicopter + ambient alarms,
      `metadata.bin`), `statistics`, `local_players` and
      `on_game_thread`; the renderer implements
      `SaveGameHooks::capture_thumbnail(player)` (zoom 1, centre, render,
      read back the 256x256 middle as bottom-up RGBA) and `save_game`
      writes `thumb.png`, then the renderer reloads that texture.
      `map_animals.bin` and the queued `metagrid/metacell_*.bin` are
      written with `map_zone.bin` (#167).
- [ ] Game load (`IsoWorld.init`): `world_gen.load_file(save_dir)`
      where `WorldGenParams.INSTANCE.load()` runs; after
      `GlobalModData.init`, `instance_tracker.load_file(save_dir,
      no_save)`; after `map_zone.bin`,
      `world_save::load_meta_grid_extras(&mut meta, save_dir)`; after
      player 0 is placed, `meta_tracker.load_file(save_dir, no_save,
      players[0].is_some())` then `statistics.load_file(save_dir,
      no_save)` (achievement checks are the caller's). In
      `GameLoadingState`, unless saving is off,
      `world_save::ensure_subdirectories_exist(save_dir)` (#167).
- [ ] Quit (`Core.exiting`): `world_save::save_on_quit(&mut g,
      &mut local_players, &mut hooks)` (`on_post_save` fires `OnPostSave`),
      then `IngameState.exit`: `streamer.stop(&mut cell, ctx)`, the other
      stops, `world_save::close_stores(Some(&mut player_db), &mut
      vehicle_store)` (#150).
- [ ] `IsoWorld.init`, after `VehiclesDB2.init` and before
      `WorldStreamer.create` / the cell load:
      `let regions = IsoRegions::init_world(save_dir, chunk_grid_width,
      core.no_save)` (`IsoRegions.init`: reads and queues every
      `isoregiondata` file). Per frame, in `IsoWorld.updateWorld` right after
      `IsoCell.update`: `regions.update_with_buildings(&mut BuildingCtx { .. },
      player)`; the first call makes the saved regions visible, as the
      original's first `IsoRegions.update` swaps them in. Give the same
      `IsoRegions` to the region readers (puddles, weather mask, square
      edits through `set_previous_flags` / `square_changed`). Quit:
      `IngameState` calls `IsoRegions.reset()`; drop it.
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
- [ ] Climbs, stairs and fences (#139): `StatesEngine::zombies_on_square`
      and `moving_objects_on_square` (the squares' moving-object lists, in
      list order), `dict_object_name` (WorldDictionary object names),
      `bent_fences()` (a `pz_world_states::BentFences::new(lore
      fenceThumpersRequired, fenceDamageMultiplier)` filled from
      `BentFences.lua`'s `addFenceTiles` as `FenceTileDef`s),
      `broken_fence_items` (`BrokenFences.addItems`) and `fence_collapsed`
      (the trip/crush reactions of characters by id); `StatesEngine::rand`
      must be the game's `Rand` (bent-fence damage draws). `SimContext::
      is_listener_in_range` from `SoundManager`; `SimEvent::ObstacleSound`
      → `playSoundImpl` + `setParameterValueByName` (and the player's
      movement-speed parameter). Owners that drive player paths call
      `GameCharacter::run_path_requests` after `path_update` and give
      players a `PathMoverState::window_hook` (`IsoWindow.canClimbThrough`).
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
- [ ] The world's square mutations (`MutationEnv.render`) and
      `WorldRegionToMetaGrid` (`BuildingCtx.render`) are given
      `pz_world_render::RenderInvalidation::new(&mut render_world,
      &mut world_lighting, &mut per_player_chunk_caches, &mut pending,
      sprites.tables.flag.water, num_players)`; each frame, before the
      lighting frames and chunk recording, `pending.apply(view,
      render_world, lighting, world_hooks, sprite_cache, textures,
      PuddleInputs { regions, perf_puddles: Core.getPerfPuddles() })`
      (which also re-inits `pending.puddles`' geometry), and after every
      player's lighting frame `check_changed_buildings(lighting, caches,
      pending, num_players)`; `pending.vispoly` is drained by
      `VisibilityPolygon2` (chunk physics reads `IsoChunk`'s own
      `physicsCheck`) (#127). Squares built on chunk load (`build_chunk`, new squares in
      `rebuild_square`) queue in `render_world.puddles_recalc` and get
      their puddle geometry in the next `apply` (`recalc = true` on a new
      `getPuddles()`).
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
      and shot nodes test); pass
      `firearm::CharacterRagdoll::new(&mut chr, StateCx)` (pz-characters'
      `bullet` feature) as `RagdollEnv::character` (it reads
      `can_ragdoll_uncapped`, writes the fall flags, `onFloor` and the
      deferred movement, slides away from walls on start and takes the
      pending shot with the attacker's position from
      `StateWorld::character_mut`); after the update call
      `CharacterRagdoll::track(&mut chr, &player, controller)` so
      `chr.ragdoll` (`RagdollTracking`) and `isRagdollSimulationActive`
      follow the controller (#114, char-ragdoll PR).
- [ ] Per frame, PlayerInput adapter: once per frame the rendering side
      implements `pz_characters::character::player::PlayerInput` over
      `pz_input` (`InputState`, `Core::get_key`/`get_key_binding`, the
      controllers' `CharacterJoypad*Binding`s, `UIManager::is_mouse_over_inventory`,
      `Core` toggle options, camera `XToIso`/`YToIso` and `XToScreenExact`,
      zoom, screen size, wall-clock ms); for a gamepad player call
      `chr.update_aiming_reticle(input, multiplier)` first
      (`AimingReticle.update`) and feed the returned position back to the
      renderer (#135).
- [ ] Per frame, local player update order (`MovingObjectUpdateScheduler`):
      `preupdate_player()`, `frame_step(input)` (the input component's
      toggles), `update_player(input, registry, groups, cx)` (IsoPlayer
      `updateInternal2` then the player part of `IsoGameCharacter.updateInternal`:
      stats, body damage, `calculate_stats`, state machine, fire, momentum,
      dirt, ballistics), and after all objects updated `postupdate_player(cx)`
      (move/collisions, action context, hit reaction, body part last state).
      Before the update set `chr.player.action_blocks_movement`/
      `action_overrides_animation`/`has_actions` from the Lua timed action
      queue and `climate_wind_speed_movement`/`climate_wind_force_movement`.
      Handle `SimEvent::SetGameSpeed` at once (speed controls; `GameTime`
      multiplier 1) and the returned `PlayerUpdate::events` (Lua
      `OnPlayerMove`/`OnPlayerUpdate`/`OnContextKey`/`OnPressWalkTo`/reload,
      contextual actions to the world/Lua, reticle mode, cursor, stop action
      queue, dirt). `StateWorld` player hooks: `game_speed`,
      `deferred_movement_length_sq`, `characters_on_square`, `separate`,
      `path_should_be_moving`, `contains_any_idle_nodes`,
      `animation_player_ready`, `rendered_angle`, `primary_hand_model_ready`,
      `square_trees`, `collapsed_fence`, `tree_slow_factor`, `ground_info`,
      `foot_injury_floor`, `vehicle_useable_part_near`,
      `context_actions_nswe`/`context_actions_corners`/`context_door_or_window`;
      `SimContext::wet_ground_final_value`, `clothing_degradation`
      (#135).

- [ ] Ragdolls, boot: after ScriptManager loads `media/scripts/ragdolls`,
      `RagdollScripts::load_file` per file then `to_bullet(&mut bullet,
      false)` (`RagdollScript.toBullet`); once the world exists
      (`IsoChunk.addRagdollControllers`), `RagdollBuilder::initialize` with
      the `bob/bob_tpose` skeleton and the `Bob_TPose` clip; keep one
      `RagdollGlobals` (max active ragdolls from Core) (#142).
- [ ] Ragdolls, per character: keep an `Option<RagdollController>` beside
      its `AnimationPlayer` and update the player with
      `update_with_ragdoll(dt, vars, &mut RagdollFrame { env: RagdollEnv {
      bullet, globals, builder, character, bullet_frame_no:
      WorldSimulation's frame number, time_delta, debug }, controller,
      multiplier })` instead of `update`, with `CharacterRagdoll` as the
      character (see the per-frame ragdoll line above). Before `stopAll`
      or releasing the player call `release_ragdoll_controller(Some(host))`
      so the ragdoll leaves the world; `vehicle_collision` from
      `IsoZombie`'s vehicle contact (#142).
- [ ] Ragdoll corpses: when an `IsoDeadBody` is made from a character,
      `DeadBody.ragdoll_fall = isRagdollFall()` and
      `died_bone_transforms = pz_ragdoll::corpse::died_bone_transforms(
      player, ragdoll_fall, can_use_current_pose_for_corpse(player,
      culled, has_active_model))`; from saves,
      `corpse::died_bone_transforms_from_saved(n, &ragdoll bones)` with
      pz-world's `IsoDeadBody.ragdoll` (#142).

## World render pass
- [ ] Cull chunk textures to the screen (`plan_chunks` / `level_on_screen`);
      allocating every loaded chunk exhausts the GPU device.
- [ ] Water/sky: `SkyBox::render`, `Surfaces.sky` from `reflection()`;
      `SurfaceChecks::run` + `apply_surface_checks` before `plan_chunks`;
      water flow from lot directories; splash state (#53).
- [ ] One frame call: `frame::world_frame_full(r, size, chunks, objects,
      drop_shadow, screen, out, late)` with the frame's `FrameObjects` and
      the weather as `late`. It draws `performRenderTiles`' order itself
      (composite; players, corpse/mannequin shadows, corpses, items;
      puddles; moving objects; water; animated attachments; per-level
      translucent floors/puddles/shore/splashes/shadows/objects; outlines
      end; then `late`) (#124).
- [x] `render_animated_attachments` per chunk between water and
      translucent passes (#76) — done by the frame itself (#124).
- [ ] `render_weather_fx` as `world_frame_full`'s `late`, driven by
      `ClimateRenderValues` (#81); pass `weather_fx_mask::FxMaskCharacters
      { world, objects: &frame_objects }` as its `FxMaskWorld` so the mask's
      `renderCharacters` replays #79's `ObjectRender` draws without shadows.
      Every character's `ObjectRender.pos` must be set
      (`render_character_object` does; set it on hand-built zombies and
      animals) (#124).
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
      (pz-iso-render `skybox.rs`, now on the same stack (#124)).
      Until that is wired, load vehicle models with `reflections: false` (the
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
- [ ] Object outlines: per highlighted non-character object
      (`IsoObject.isOutlineHighlight(player)`, colour from
      `getOutlineHighlightCol`, alpha times `Core.blinkAlpha` when
      blinking) push an `ObjectOutline` into `FrameObjects.object_outlines`:
      `corpse(entry)` with the corpse's `BodyTexture` entry,
      `mannequin(entry, ..)` with the mannequin's atlas entry
      (`DeadBodyAtlas::body_texture` of `Mannequin::atlas_body()` keyed by
      `posed_body_key(.., standing)`), `world_item_atlas` /
      `world_item_sprite` for placed items, `sprites(..)` for other sprite
      objects (sprite, then overlay and attached sprites when
      `isOutlineHlAttached`). Objects with a sprite model are skipped, as
      are objects whose render layer is none or alpha 0 (#125).
- [ ] Mannequins: from each `IsoMannequin` (script anim set, state,
      validated pose, direction, surfaces below/on its square, outside /
      room, interpolated light, highlight tint) build a `Mannequin`, pose an
      `AnimationPlayer` on the body skeleton with `mannequin::pose`, build
      its `ModelSlot` (skin texture `Body/<script texture>` plus the
      outfit's clothing), and the frame its chunk texture is recorded push
      `mannequin::chunk_models(.., chunk texture, camera square)` into
      `FrameObjects.chunk_models`; every frame `mannequins` gets an
      `ObjectRender` with `Mannequin::shadow(lightInfo)` (#125).
- [ ] Animal corpses: `DeadBody { animal: true, animal_anim_set:
      animalAnimSet, animal_size }`, a slot from
      `ModelSlot::single_body(model script mesh, Body/<skin texture>,
      animalEffect)` for the breed's `AnimalVisual`, the anim set's
      `deadbody` state, keyed with `animal_body_key` (#125).
- [ ] Split screen: one `world_frame_for_player(.., Some(PlayerView {
      player, num_players, screen }), ..)` per player into the shared
      offscreen buffer (each draws in its `glViewport` quarter, outline
      passes in `IsoCamera.getScreenLeft/Top/Width/Height`), the camera
      sized to the player's view; the last call's screen pass shows all
      of them (#125).
- [ ] GPU faults: check `Gpu::error()` after `render_frame` (a lost
      device stops drawing; out-of-memory chunk/atlas textures are left
      out) and stop or rebuild the `Gpu` with a clear message (#120).

- [ ] Frame extras (#141): build the world frame with
      `frame::world_frame_extras(.., &mut FrameExtras { .. }, ..)`; a field
      left `None` draws nothing. `ModelOutlines.endFrameMain` now runs after
      the weather, area highlights and LineDrawer, as in
      `IngameState.renderframe`.
- [ ] Corpse flies (#141): `RenderSquare.flies` from
      `IsoGridSquare.hasFlies` (set by the corpse rot simulation); keep one
      `flies::CorpseFlies` (texture `media/textures/CorpseFlies.png`), call
      `update(realworldSecondsSinceLastUpdate)` from `IsoWorld.update` (not
      on a server) and pass it as `FrameExtras.flies`.
- [ ] Object highlight (#141): `RenderObject.highlight =
      Some(ObjectHighlight { color: getHighlightColor(playerIndex),
      alpha_scale: isBlink() ? Core.blinkAlpha : 1 })` while
      `isHighlighted(playerIndex)` (the render-once bookkeeping of
      `objectHighlightRendered` stays with the owner); `FrameExtras.highlight
      = Some(HighlightFrame { creating_thumbnail })`.
- [ ] Opaque-objects event (#141): `FrameExtras.opaque_event`
      with the joypad flag, `UIManager.getPickedTile()`,
      `IsoWorld.isValidSquare` and a handler raising Lua's
      `RenderOpaqueObjectsInWorld(playerIndex, x, y, z, square)` whose
      handlers (ISBuildingObject ghosts, fishing Bobber) draw into the list.
- [ ] Markers (#141): `FrameExtras.markers` with
      `WorldMarkers.gridSquareMarkers` after `update()`'s fade/blink
      (`GridSquareMarker`), `IsoMarkers.markers` (`IsoMarker`, with the item
      model's draws when it has one), `circle_center.png` and
      `circle_only_highlight.png` from `media/textures/worldMap`, a white
      texture, the chunk map's world tile bounds and the player's z. Homing
      points and direction arrows draw in `renderframetext` (screen space)
      with `markers::render_frame_text`; their placement
      (`WorldMarkers.update`/`PlayerScreen`) is not ported.
- [ ] Area highlights (#141): `FrameExtras.area_highlights` with
      this UI frame's `FBORenderAreaHighlights.addHighlight*` calls, the
      client's non-PvP zones and safehouses (when shown), the animal
      designation zones (when `isSeeDesignationZone`), the chunk map bounds
      and a white texture; outlines go through `LineDrawer`
      (`area_highlights::render_lines`).
- [ ] Vision cone (#141): `FrameExtras.vision` = a
      `vispoly::VisionFrame` each frame: `vision_cone` from
      `LightingJNI.calculateVisionCone` (pz-lighting), look angle/vector and
      the reversing-driver flag from the player/pz-vehicles,
      `inside_tree`/`on_square` from the player's square, `chunk_map` from
      the cell's chunk map, `view_cone_opacity` from the options, split
      screen index/count from the game state. Keep a
      `vispoly::VisPolyChunks`; take each on-screen chunk level at
      `floor(camZ)` (in `getChunks` order) from `level(wx, wy, z,
      adjacentChunkLoadedCounter, || ChunkLevelData::recreate(..))` with
      `VisSquare` flags from pz-world's line of sight, and drain
      pz-world-render's `PendingRender.vispoly` into
      `VisPolyChunks::invalidate(wx, wy, level)` each frame.
- [ ] Cell fog (#141): keep a `fog::ImprovedFog`; the weather
      owner calls `init(fogIntensity)` at world start and
      `update(&FogClimate)` once per tick (ClimateManager fog intensity,
      new-fog exterior colour, wind angle and intensity, `GameTime`
      multiplier, `PerformanceSettings.fogQuality`). Each frame
      `FrameExtras.fog = FogFrame { noise:
      media/textures/weather/fognew/fog_noise.png, player (x, y, z,
      isInARoom, in a vehicle), storm (tropical storm or thunderstorm
      running), right_click_off, offscreen_left/top, .. }`.
- [ ] New draw commands (#141): `DrawCmd::ImprovedFog` and
      `DrawCmd::VisibilityPolygon` are executed by pz-gpu (`fog.wgsl`,
      `vispoly.wgsl`); any other backend must handle them.

## Cleanups
- [ ] Fold bind_vehicles' item subset into the full item bindings (#86).
- [ ] Convert remaining bare prints to DebugLog (#66 meta_grid.rs, #67
      meta_zones.rs/zone_save.rs) (#69).
