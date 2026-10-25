# Engine wiring checklist

Hooks that subsystem crates expose and the game loop (`pz-engine`,
`pz-client`) must call. Checkboxes record implemented connections; runtime
parity requires separate verification. Current continuation is on
`codex/gameplay-continuation`, based on Claude's `ws/integration` and
`ws/ingame-loop`; the older `main` checkout is not the current engine.

The October 25 continuation connects live player/zombie models and the same
animation rigs to state-world root motion, thump timing and turn targets.
It also restores the pending world-Lua registries, fixes population chunk
lookup overflow, and saves active sessions on native window close. Quit-save
Lua events run between borrow-scoped phases, so their clock/world mutations
reach the saved files. These changes still need complete gameplay and
platform parity checks.

New worlds now build and save `WorldDictionary.bin` before loading players
and chunks. Item/entity IDs share the original counter, saved and removed
entries retain their IDs, erosion object names are registered, and the
script manager runs `PostWorldDictionaryInit`. New string/script register
population and version hashing remain incomplete. The installed session
regression now reboots the saved world and checks the reconstructed player;
its result must be recorded separately from ordinary unit tests.

The platform workflow builds the native client and runs workspace tests on
Windows, Linux, Apple Silicon and Intel Mac runners. Installed game assets
are excluded, so a green source check alone cannot establish gameplay parity.

Returning to the menu rebuilds script/Lua/UI owners before menu entry;
native shutdown skips that rebuild. A failed rebuild stops the client
instead of entering a menu with mixed owners. Character `getModData`
exposes a live table and saves retained nested mutations. The conservative
collector handles registered tables, Lua closures and shared upvalues,
including environment and recursive closure cycles. Reviewed Event callbacks
and Loader caches participate through explicit strong native-capture owners.
Other native captures, coroutines, prototypes and engine-owned references
remain opaque roots; character/action cycles and some old LuaManager
environments can remain retained. Heap
statistics and a complete VM/host collector remain unported.
The SQLite writer rejects auto-vacuum files before changing them; writing
that format needs pointer-map/root relocation support and remains pending.

`WorldGenParams.INSTANCE` now shares the engine's seed and cell bounds.
Menu changes, saved-file loading and world initialization use that same
owner. The binding covers the shipped menu's seed/bounds operations; the
remaining API and post-initialization generator seed updates need parity
work. Installed compiler string-literal quirks are preserved and tested.

Starting-item scripts can create a named key ring through the normal item
factory and inventory, including the original chance of a house key from
the actual meta-room's building. String-based item creation retains its
script dictionary ID, so nested items can reload with their correct types.
The player-screen Lua globals read the current Core size and player count;
this supplies inventory layout geometry but does not implement split-screen
world rendering. Inventory category/type queries use the real per-item
reference and original subclass constructor behavior, including null types
and the WeaponPart constructor's WEAPON value. `getTex` delegates to the
same live texture lookup as `getTexture`. `isHidden` reads the retained
script item at use time, so the shipped inventory pane can refresh rows.

The window reads the real single-player speed-control pause state after UI
input. Paused frames skip game-state simulation while UI/audio/rendering
continue; only in-game frames emit `OnTickEvenPaused(0)`. The survival guide
resumes through its shipped close handler. Paused camera updates and full
network pause ownership remain pending.

The running timed-action stack now belongs to the character, separately
from Lua's pending-action queue. Protected callbacks run between released
character borrows, before body damage/stat processing, and completed actions
are removed by identity. Movement modifiers, animation events, progress and
hand models use the live action. Aiming and walking/running cancellation
callbacks now run inline between released host borrows, preserving already
computed locals and reading live player clocks after resume. Other player
events retain wider phase gaps; normal vanilla queue entrance still needs
genuine grapple ownership for `isDraggingCorpse`. Remaining action APIs, network and
voice audio also need work. Sneaking uses the existing input/save field;
farming exposes its actual transient player flag.

Puddles now have a shared live singleton and indexed float controls. A real
cell-owned WeatherFX object initializes at world init; the climate updates
its targets and puddles after Lua climate callbacks, respecting the live
enable flags. Movement and ground-dirt calculations read the live wet-ground
control, including Lua admin changes made after frame capture. Per-cell
particle stepping, sky/water updates, masks and rendering remain unwired.

Known integration gaps include action-context/turning phase timing, timed
actions, character visibility fading, split-screen viewports, weather and
sky/water effects, animals/ragdoll rendering, basement save persistence,
overlay and broken-fence mutations, key spawning/naming, and missing Lua/UI
bindings. The installed-asset session regression is opt-in and writes only a
fresh isolated cache. A successful ordinary workspace suite does not exercise
all installed assets, render paths or foreign platforms.

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
- [x] Gamepads: `Gamepads::new(game_dir, cache_dir, c)` (#32).
- [x] Sound: build `pz_audio::studio::Studio` from
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
- [ ] Current player: wherever the original calls `IsoPlayer.setInstance`
      around a local player's update/render, call
      `bind_players::set_player_instance`; `getPlayerIndex()`
      (`bind_players::player_index`, the HUD's `player_index`) reads it, or
      `assumedPlayer` (`set_assumed_player`) without an instance (#177).

- [x] `IsoWorld.init` → `ScriptManager.PostTileDefinitions`: after the
      tile definitions and the scripts, call
      `sprites.post_tile_definitions(&pz_world::sprite::EntitySpriteConfig::from_scripts(&sm))`
      (sets `EntityScriptName` and builds the `SpriteConfigManager` entries
      chunk loads use for `SpriteConfig`'s master flag; the save's
      `ScriptItems` answers the `SpriteConfigs` register) (window byte).

## Per frame (`GameWindow.logic` / `render`)
- [x] Input: every winit event → `pz_input::winit::handle`; before
      `UIManager::update` call `bind_input::update(&mut mgr, secs)`, after it
      `bind_input::after_ui_update(&mut mgr)`; apply
      `bind_input::take_window_requests()`; `gp.poll(c)` first (#30/#32).
- [x] Sound: `SoundManager` update and emitter ticks through
      `sound_system().with(..)`; listener from the active camera (#78/#83).
- [x] `UITransition.UpdateAll` → `bind_misc::ui_transition_update_all()` (#20).
- [ ] Vsync from Core options; `FrameParams` size from Core screen size.
- [x] `GameTime::update` / `update_game_time(thread)` once per frame
      before the world (#49).
- [ ] Pathfinding: PathHost `update_thread` then `update_main` before the
      characters update; `host.remove(id)` when a character leaves (#74).
- [x] Characters, in game order: `preupdate_zombie` (players: their
      preupdate), `update_zombie` (the state machine, then `update_fire`),
      `postupdate_zombie` / `moving_postupdate` (with
      `slide_head_away_from_walls`) (#41/#74/#75).
- [x] Player input: after the camera update call
      `pz_player_input::set_camera_info(i, CameraInfo { off_x, off_y,
      right_click_x, right_click_y })` per local player (the reticle
      position is `chr.player.aiming_position`, set by
      `chr.update_aiming_reticle(&input, multiplier)`); after
      `bind_input::update`/`after_ui_update` the game loop builds one
      `EnginePlayerInput::capture()` per frame, passes `&input` to each local
      player's `IsoPlayer.update`, then calls `input.write_back()` (#137).
- [x] `ZombiePopulationManager::update_main` with PopulationHost and
      `ZombieOps` (`RealZombies`) (#73/#77; `RealZombies::create_survivor`
      returns `CreateSurvivor`'s `ScriptResult`, and `WorldPopulation::update`
      / `start_game` return `Err(java exception text)` when creating a real
      zombie threw (#147): `IngameState.update` has try/finally with no
      catch (IngameState.java:640–650), so an `Err` aborts the rest of that
      frame's `IngameState.update` and is logged via `ExceptionLogger`; the
      popman worker pass has already run, as its own thread would): `GameWindow.logic` calls
      `WorldPopulation::update_game_state(no_save, paused)`; `IngameState.update`
      (after `VirtualZombieManager.update`) calls `WorldPopulation::update(&mut
      pz_world_states::population_host(rand, &mut RealZombies { world: &mut
      states_adapter, .. }),
      &PopulationFrame { multiplier, world_age_hours,
      chunk_maps: &player_chunk_maps(&streamer), real_zombies:
      &real_zombies.infos() })`; it also runs the in-line worker
      (`MCDThread`) pass when due.
- [x] `WorldSoundManager::update` once per frame (#75).
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
- [x] `IsoWorld.init`, after `IsoMetaGrid::create`: `meta.zone_tables =
      ZoneTables::load_from_game_jar(jar)?` and `meta.clipper =
      Some(Box::new(zone_clipper::ZoneClipperOps::default()))` (polyline
      outlines, polygon clipping) (#224).
- [x] `IsoWorld.init`, before `OnLoadMapZones`:
      `bind_basements::before_on_load_map_zones()`; the bent-fence sandbox
      values on `bind_bent_fences::shared_bent_fences()`
      (`ZombieLore.FenceThumpersRequired` / `FenceDamageMultiplier`);
      `ZombiePopulationManager.init` through
      `WorldPopulation::init_with_spawn_origins(.., &lua_world::take_spawn_origins())`
      (#224).
- [x] The sprite manager is `Rc<RefCell<IsoSpriteManager>>`, installed with
      `bind_sprites::install_sprite_manager` (`IsoSpriteManager.instance`);
      `ErosionMain` is shared through `bind_erosion::set_instance` (#224).
- [x] `Basements.beforeLoadMetaGrid`: build `pz_worldgen::basements::Basements`
      from `bind_basements::take_per_map()` (what `basements.lua` and the
      `Basement` zones registered), then load headers, placements and the
      meta-grid buildings (#224); `IsoWorld::before_load_meta_grid`, handed
      to `WorldGenHook.basements` for `onNewChunkLoaded` (Claude handoff).
- [x] Lua new-world registries: tile/container overlays, `BrokenFences`,
      `KeyNamer`, `WorldGenUtils.INSTANCE` and the randomized-world clutter
      lists are registered before server Lua loads (Claude handoff).
- [x] Character Lua world handles: `getCurrentSquare` / `getSquare` read
      the character's live current square and return the world's canonical
      handle; `getCell` returns the installed cell. `getJoypadBind` reads
      the player's input component. Unloaded squares/cells return `nil`.
- [ ] `Basements.loadSavefile` / `writeSavefile` (`map_basements.bin`): a
      loaded world recomputes its placements.
- [ ] `TileOverlays.updateTileOverlaySprite(obj)` when a square's objects
      are created or loaded: `bind_tile_overlays::shared_tile_overlays()`
      `.pick_overlay(..)` puts the sprite first in `attachedAnimSprite`;
      `ContainerOverlays.updateContainerOverlaySprite` from
      `shared_container_overlays()` sets the overlay sprite;
      `fixTableTopOverlays(square)` after moving objects; both `Reset()` on
      world unload.
- [ ] `BrokenFences.destroyFence(obj, dir)` (thumping, vehicles): the engine's
      `StatesEngine::destroy_fence` / `is_breakable_fence` /
      `broken_fence_items` read `bind_broken_fences::shared_broken_fences()`
      (`update_sprite_name`, `debris_sprite`, `adjacent_damage`,
      `destroy_adjacent_dirs`, `add_items` drawing `Rand`), then do the
      square changes, sound and recalculation; `Reset()` on unload.
- [ ] `ItemPickerJava.KeyNamer.nameKey(item, square)` when keys spawn:
      `bind_key_namer::key_namer().key_name_translation(&square)` with a
      `pz_inventory::key_namer::KeySquare` over the square.
- [ ] Story clutter: `RandomizedWorldBase.get*ClutterItem()` reads
      `bind_randomized_world::clutter(field)`.
- [ ] `MapObjects.newGridSquare` / `loadGridSquare` on brand-new / loaded
      squares: call `bind_map_objects::on_new_callbacks(sprite)` /
      `on_load_callbacks(sprite)` with each object (needs an `IsoObject`
      Lua binding); `MapObjects.Reset` on world unload (#224).
- [ ] Fence bending (`pz-world-states`) reads its `BentFences` from
      `bind_bent_fences::shared_bent_fences()` (#224).
- [ ] Water rendering reads `IsoWaterFlow` from
      `lua_world::shared_water_flow()`; `IsoWaterFlow.Reset` on world
      unload (#224).
- [ ] Animal definitions: `pz_animals::AnimalDefinitionTables::load(&env)`
      over `LuaManager.env` once shared Lua is loaded, as `IsoWorld.init`
      calls `AnimalDefinitions.getAnimalDefs()` (which loads the genome
      definitions); tracks reload where `IsoWorld` calls
      `loadTracksDefinitions` (#191).
- [x] `GameLoadingState.enter`: `bind_video::release_all_video_textures()`
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
      character's items (the `GameCharacter` impl in
      `pz-characters/src/character/item_owner.rs`). An `ItemLuaEnv` returns
      character's items (`CharacterItemOwner` over the `GameCharacter` impl
      in `pz-characters/src/character/item_owner.rs`; done for characters
      through `character_value`, #187). An `ItemLuaEnv` returns
      `bind_item_container::container_owner(item)` from `owner` (as
      `GameItemEnv` does), so `isEquipped`/`isWorn`/`isInPlayerInventory`
      follow `item.container.parent` (#185).
- [ ] Inventories loaded into an existing container must make items with
      `pz_inventory::shared::new_item` (or `SharedContainer::fill_from`, which
      `shared::load_into` now calls) so a bag's inventory gets its
      `containingItem` (#185).
- [x] End of `GameLoadingState`: `hud::init_ui(&mut thread)` (the in-game
      HUD, then `OnCreateUI`) before `OnCreatePlayer` (#144).
- [x] `IngameState` per player: `hud::update_progress_bar(n)` in the update
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
      `shared::load_into`, saved via `to_container`);
      `bind_character_sim::character_value` sets the character's Lua object
      as its parent once and `getInventory()` returns
      `bind_item_container::container_value(&inv)`, so Lua and the
      simulation share the container and items. Characters handed to Lua
      must go through `character_value` (#143, char-inventory PR).
- [ ] Worn items: characters made outside `load_player`,
      `new_survivor` and `new_zombie` (an `IsoPlayer` constructor) must call
      `GameCharacter::init_worn_items(sm, BodyLocations.getGroup("Human"))`
      or `getWornItems()` is `nil` (#187).
- [x] Item-side `isEquipped()`/`isWorn()` for character items:
      `character_value` (and `setWornItem`) set the inventory's parent with
      `set_parent_with_owner(&inv, character_lua_value,
      Rc::new(CharacterItemOwner::new(&chr)))`, so `GameItemEnv::owner`
      (`container_owner`) reaches the character's hands and `WornItems`
      (#187).
- [ ] World containers need their `IsoObject` parent set with
      `bind_item_container::set_parent` the same way (#143).
- [ ] Shared tooltip: `HudPicking::picked_tile` (`UIManager.getPicked().tile`
      as a `pz_ui::tooltip::TooltipObject`: identity, `haveSpecialTooltip`,
      its square's Lua object; world thread); the tooltip font option and
      highlight colours come from `Core` (#153).
- [x] Item tooltips: `item:DoTooltip(tooltip)` / `DoTooltipEmbedded` are
      bound with the item classes (`bind_items::register` →
      `item_tooltip::register`); the tooltip's `setCharacter` gives the
      rows their character (queried through its Lua binding) (#192).
- [ ] Item tooltip debug rows: `item_tooltip::set_debug_options` from
      `DebugOptions.tooltipInfo` / `tooltipAttributes` when the debug UI
      changes them (#192).
- [x] HUD alarm clocks: `hud::alarm_clocks::install_alarm_clock_host()`
      (in `register_all`) puts `PlayerAlarmClocks` in `HudAlarmClocks`
      (#192).
- [ ] Alarm clock updates: call
      `hud::alarm_clocks::update_player_alarm_clocks(GameTime.getWorldAgeHours())`
      each tick (the cell's `processItems`) and apply each
      `AlarmEffects`: `WorldSoundManager.addSoundRepeating` at the player's
      square (`soundRadius`, 3), `ItemSoundManager` add/remove (the ring
      loop, "AlarmClockRingingStop" for `AlarmClock`), MP syncs (world,
      sound, network threads). Clocks on the ground run
      `AlarmData::update` from the cell's process list with their square
      (world thread) (#192).
- [ ] Object picking: implement `hud::picking::PickWorld` over the chunk
      renderer's per-object `ObjectRenderInfo` for player 0 (layer, target
      alpha, cutaway, `renderX/Y/Width/Height/ScaleX/ScaleY`, the sprite
      mask) and the chunk textures' `renderX/Y`, then install
      `with_host(|h| h.picking = Box::new(WorldPicking { world }))`; use
      `picking::context_pick` for `UIManager.setPicked` on mouse events
      (world-render thread) (#192).
- [ ] Object picking render side: keep a `pz_iso_render::RenderInfos`
      next to the chunk caches, build each player's `ChunkRenderer` with
      `.with_render_infos(&infos, player)` (chunk recording, composite and
      translucent passes record `ObjectRenderInfo`), call
      `infos.forget_chunk(wx, wy)` when a chunk unloads, and give the HUD a
      `PickWorld` built per frame as `pz_engine::world_hud::EnginePick {
      world: pz_world_render::WorldPick { view, render, infos, player: 0,
      camera: player 0's camera }, hooks }`; `world_hud::PickHooks`
      supplies `IsoPlayer.players[0]` (characters thread), the square Lua
      objects, door adjacency, entity UIs and corpses (`isMouseOver`).
      Sprite models (`renderModel` → `updateRenderInfoForObjectPicker`) are
      not recorded yet (world-render thread) (#198).
- [ ] HUD camera: create one `pz_world_render::SharedCameraFrame`, call
      `pz_engine::world_hud::install_hud_camera(frame.clone())` after the
      HUD host is installed,
      and each frame set `cameras[i]` to each player's `Camera`,
      `player_index` (`IsoPlayer.getPlayerIndex()`) and
      `frame_player_index` (`IsoCamera.frameState.playerIndex`) before the
      HUD updates and renders (world-render thread) (#198).
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
- [x] TextDrawObject: `IngameState.renderframeui` calls
      `hud::text_draw::render_batch(player)` after `WorldMarkers`/`IsoMarkers`
      (`DebugChunkState` calls `no_render`); `ChatElement`'s constructor calls
      `ensure_chat_backdrop()`; `HudPlayers::player_index`
      (`IsoPlayer.getPlayerIndex()`; characters thread) tags batched draws
      (#174).
- [ ] Gizmos: `IsoWorld.render` calls `gizmo::render(frame player)` after
      the cell and draws the returned `GizmoFrame` (its projection, then each
      `GizmoPrimitive`: debug axis lines, `addCylinder_Fill`, `addTorus` with
      depth testing as noted) on the render thread; `HudGizmoCamera`
      (`IsoCamera.frameState` camCharacter/zoom and `cameras[0]`'s offsets;
      world-render thread). `UIManager`'s left button already goes through
      `gizmo_hit_test_hook` (#199).
- [ ] Chat (SP): characters thread implements `chat::ChatPlayer` and
      `chat::ChatElementOwner` for IsoPlayer (and installs `chat::ChatPlayers`
      for `IsoPlayer.players`/`getInstance`), gives each `IsoGameCharacter` a
      `ChatElement::new(weak_self, 1, Some("character"))` (5 lines in
      `initTextObjects` for players), calls its `update()` in
      `IsoGameCharacter.update` and `ChatElement::render_batched` from
      `renderlast`, and routes `Say`/`SayShout`/`SayRadio`/`addLineChatElement`/
      `SayDebug` to it and `with_chat_manager`; `GameLoadingState` calls
      `chat::set_allowed_chat_icons(..)` (`Texture.collectAllIcons`) then
      `with_chat_manager(|m| m.init(true, player))`; `IngameState.renderframeui`
      calls `ChatElement::render_batch(player)` right after
      `text_draw::render_batch`; Rust callers of the chat fire its queued Lua
      events with `chat::fire_pending_events(thread)` (#190).
- [ ] WorldMarkers: `with_markers(WorldMarkers::init)` when the game starts
      (`IngameState.enter`) and `reset()` on exit; `hud::world_markers::render`
      per player in `IngameState.renderframeui` (before `render_batch`); the
      FBO chunk renderer draws `fbo_markers(z)` per level
      (`FBORenderWorldMarkers`: chunk-map culling, depth, VBO). Inputs:
      `HudCamera::camera_offset` / `frame_player_index` (world-render thread),
      `HudPlayers::player_square` / `player_index_of` and
      `HudClimate::fog_intensity` (characters thread),
      `HudPicking::square_coords` (world thread) (#193; `WorldPicking`
      implements it from the square handle, #197).
      events with `chat::fire_pending_events(thread)` (#TBD-chat).
- [ ] Chat, characters side (`pz_lua_api::chat::characters`): create each
      character's chat element with `attach_chat_element(&chr)` when the
      character is made (the constructor's `chatElement`;
      `character_value` and `bind_players::set_player` also attach). After
      every character update (`update_player`, the zombie update) call
      `flush_chat(&chr)` and then `chat::fire_pending_events(thread)`: it
      runs the `Say`s, `setMaxChatLines(5)` and the chat element update
      that `updateTextObjects` queued (every tick, or the queue grows).
      Fill `CharacterEnvironment::square_can_see` (`getCurrentSquare()
      .getCanSee(0)`), `square_building` and `square_room` (opaque ids).
      `IsoGameCharacter.renderlast`: `render_last(&chr, x, y, isInvisible,
      camCharacter == chr)` at `getNameCoords` (above the name tag,
      progress bar and halo note). `GameLoadingState`'s
      `ChatManager.init(true, player)` takes `bind_players::player_instance()`
      as the `Rc<dyn ChatPlayer>` (#195).
- [x] Zombies: pass the real `StatesWorldAdapter` and sitting-obstacle
      provider to the zombie state machines each frame, not
      `NullWorld`/`NoSittingObstacles` (found in the e2e run).
- [x] Popman over the real world (#210): build the population host with
      `pz_world_states::population_host(rand, &mut RealZombies { world:
      &mut StatesWorldAdapter::new(..), .. })` (the host reads the cell,
      meta grid, sprites and sitting checks through the adapter between
      zombie calls; the adapter is the `SittingChecks`: `has_moving_objects`
      is `StatesEngine::has_moving_objects` plus the lent zombie list,
      `line_clear_collide` is `LineClearCollideMain` with pz-vehicles'
      `PathfindVehicles` and `ignore_vehicle = None`). The engine's
      `has_moving_objects` must answer for players and animals on the
      square (the real zombie list is counted by the host).
World load, in the original's order (`GameLoadingState.enter` loads the
server Lua first, so `ClimateMain.lua`'s `OnClimateManagerInit` handler is
registered):
- [x] `GameTime.setInstance(new GameTime())`: `bind_climate::install_game_time`.
- [x] `ClimateManager.setInstance(new ClimateManager())`:
      `bind_climate::new_climate_manager(thread)` (fires
      `OnClimateManagerInit`; `ClimateMain.lua` sets the season colours, without
      which the global light stays black).
- [x] `IsoWorld.init`: `GameTime.init` then `GameTime.load`
      (`game_time_save::load_map_t_file`; the climate part keeps the colours);
      `ErosionGlobals.Boot` → `bind_climate::start_erosion_season(thread,
      &SeasonConfig, sandbox Temperature)` (`OnInitSeasons`); after
      `metaGrid.CreateStep2`, `bind_climate::init_climate_manager(thread,
      Some((meta.min_x, meta.min_y, meta.max_x, meta.max_y)))`.
- [x] `IsoWorld.init`, after `GameEntityManager.Init` and before
      `metaGrid.CreateStep2` (between the erosion boot and
      `init_climate_manager` above): `PersistentOutfits.instance.init()` =
      `bind_zombie_outfits::init_persistent_outfits(thread,
      Some((&save_dir.join("z_outfits.bin"), core.no_save)))` (Lua's
      `ZombiesZoneDefinition` must have run); keep the returned outfit names
      for `WorldPopulation::init` (#147).
- [x] `IsoWorld.init`, after `OnLoadedMapZones` and before the cell load:
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
- [x] `IngameState.enter`, world loaded: `WorldPopulation::start_game(&mut
      host, &frame)` (`MapCollisionData.startGame`).
- [x] Quit: `WorldPopulation::save(&reals)` with the world save, then
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
      `VehiclesDB2.setVehicleLoaded` in `addToWorld` (with the ids
      installed, see below; without them an id < 1 is written as is, like
      the shipped game with asserts off); and `IsoChunk.removeFromWorld`
      before the chunk save is queued, which the streamer does (next
      entry). The quit save's `updateVehicleAndTrailer` uses the same
      `vehicle_buffer` (ws/vehicles-db-save).
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
- [ ] Vehicle ids and the meta, filled from pz-vehicles (ws/vehicle-meta):
      at `VehiclesDB2.init` make `let ids = Rc::new(RefCell::new(
      VehicleIds::init(conn)?))` (`VehicleIds::default()` without a store)
      and `pz_vehicles::env::set_vehicle_ids(Some(ids.clone()))`;
      `BaseVehicle::add_to_world` then calls `set_vehicle_loaded` (a new
      vehicle's `sqlId` is allocated there) and
      `VirtualVehicleManager.removeFromMeta`, `remove_from_world` calls
      `set_vehicle_unloaded`. Each streamer call fills `StreamCtx` with
      `vehicle_saver: Some(&pz_vehicles::vehicles_db::RuntimeVehicleSaver::default())`,
      `vehicle_meta: Some(&mut pz_vehicles::virtual_vehicle::VirtualVehicleMeta::default())`
      (stateless; its `skipped` counts entries that were not runtime
      vehicles) and `vehicle_ids: Some(&mut ids.borrow_mut())`. Keep that
      borrow only for the streamer call: after it, for each vehicle that
      was on an unloaded chunk (`StreamEvent::VehiclesToMeta` ids and the
      chunk's remaining runtime vehicles) call the runtime's
      `remove_from_world()` (physics, id map, cell list, sounds, lights;
      it borrows the ids again and unloads the id once more, which is a
      no-op), so it must not run while the streamer holds `&mut ids`. A
      vehicle sent to the meta already left its chunk list and its
      square/current (`VirtualVehicle.set`). The vehicle runtime no longer
      has `VehicleEvents::vehicle_unloaded`; `vehicle_removed_from_database`
      (`permanentlyRemove`) stays an event: forward it to
      `vehicles_db::remove_vehicle(conn, sql_id)` on the streamer's store.
- [ ] `VirtualVehicleManager` (single player only): per frame in
      `IngameState.update`, after the vehicle manager, `let rows =
      pz_vehicles::virtual_vehicle::with_manager(|m| { m.update(Some(&mut
      ids.borrow_mut()), &save_ctx); m.take_queued() })` (records whose
      alarm, siren and engine stopped give the vehicle its state back and
      queue their row at a chunk of their position); on the game save
      (`GameWindow.save`, after `GameEntityManager.Save`) `m.save(..)`
      then `m.take_queued()`. Pass the `Ok` rows to
      `streamer.queue_vehicle_updates(rows)` (`VehiclesDB2.updateVehicle`;
      the streamer's idle loop writes them) and log the `Err`s as the
      original's `ExceptionLogger`. `save_ctx` is the streamer's `SaveCtx`
      (sprites, save tables/settings, items). `m.reset()` on
      `IngameState.exit`.
- [ ] `VehiclesDB2.loadVehiclesInMeta` (`IngameState.enter`, single
      player, after `chunkMap[0].update()`): `let r =
      vehicles_db::load_vehicles_in_meta(vehicle_store.conn.as_mut(),
      &save_ctx)` (rows saved with `inMeta`), then
      `pz_vehicles::virtual_vehicle::add_loaded_vehicles_to_meta(&r.loaded,
      |lv| Some(pz_vehicles::save::from_save(&lv.vehicle)))`: each vehicle
      not already in the world goes through `addToWorld`,
      `removeFromWorld` and `addToMeta`.
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
- [x] Game load (`IsoWorld.init`): `world_gen.load_file(save_dir)`
      where `WorldGenParams.INSTANCE.load()` runs; after
      `GlobalModData.init`, `instance_tracker.load_file(save_dir,
      no_save)`; after `map_zone.bin`,
      `world_save::load_meta_grid_extras(&mut meta, save_dir)`; after
      player 0 is placed, `meta_tracker.load_file(save_dir, no_save,
      players[0].is_some())` then `statistics.load_file(save_dir,
      no_save)` (achievement checks are the caller's). In
      `GameLoadingState`, unless saving is off,
      `world_save::ensure_subdirectories_exist(save_dir)` (#167).
- [x] Quit (`Core.exiting`): `world_save::save_on_quit(&mut g,
      &mut local_players, &mut hooks)` (`on_post_save` fires `OnPostSave`),
      then `IngameState.exit`: `streamer.stop(&mut cell, ctx)`, the other
      stops, `world_save::close_stores(Some(&mut player_db), &mut
      vehicle_store)` (#150).
- [ ] Session save, characters side: for each local player build its row
      with `world_save::local_player_data(&mut player, sql_id, sm, chars,
      ctx, &SaveEnv { .. })` (`PlayerData.set`: `IsoPlayer.save` bytes from
      `GameCharacter::save_player`) and pass them to `save_on_quit`; keep the
      `sql_id` `PlayerDb::save_player` gives a new player. For
      `SaveGame::reanimated` call
      `character::save::reanimated_players_to_save(sm, chars, ctx, &env,
      &mut tracked, &mut cell_zombies, &|z| stairs_at(z))` (the manager's
      list, then the cell's live reanimated players); on `Err` log it and
      pass `None` (no `reanimated.bin`) (#178).
- [ ] `IsoWorld.init` (`ReanimatedPlayers.loadReanimatedPlayers`): read
      `reanimated.bin` (missing: none) into the tracked list with
      `character::save::load_reanimated_players(.., new_zombie, set_on_fire,
      &mut tracked)`; `new_zombie` is `new IsoZombie(cell)`
      (`GameCharacter::new_zombie` with no descriptor). Call each zombie's
      `set_default_state(cx)` once the world exists. Not ported yet:
      `addReanimatedPlayersToChunk` (chunk load moves a tracked zombie into
      the cell's zombie list) and `removeReanimatedPlayerFromWorld` (chunk
      unload moves it back) (#178).
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
- [x] Local player at world init: `VehicleStore::create` (`VehiclesDB2.init`),
      `savefile::is_player_alive`, `PlayerDb::new`; `load_player_for_info`
      gives `SWorldX/SWorldY` for the chunk map; later `load_local_player_1`
      (decode with `load_player`, health 0 when `is_dead`, set `sqlId`) and
      `chunkMap.worldX/Y = local_player_chunk_map_pos(x, y, offset)` (#100;
      wired in `IsoWorld::load_player`, #209/#212). Left: `player.sqlId`
      (no field on `GameCharacter` yet) and `map_p.bin` saves (#212).
- [ ] CGlobalObjects `initSystems` and update pumps (#31).
- [x] Popman: `pz_world_states::population_host` (#210; was
      `WorldPopulationHost::from_state`), `update_main` with
      `player_chunk_maps(&streamer)`; SittingChecks is the
      `StatesWorldAdapter` (#77, #60, #210). Driven through
      `WorldPopulation` (see the world-load list above).
- [ ] World sound: `WorldSoundAdapter::from_state`; engine `SoundEffects`
      (OnWorldSound, fish noise, popman) (#71).
- [x] Lighting: `WorldLighting::set_chunk_map` for every local player (#76).
- [ ] Fire: `IsoFireManager::update(FireWorld, rand, FireFrame)` once per
      frame; FireHooks; fire_updated/removed via FireLightLinks (#89).
      (#FIRE-REST) `update` returns `Err(java exception text)` when a
      burn threw (`IsoWindow.removeSheet` on a window outside rooms whose
      inside square does not exist): like the population `Err` above,
      nothing up to `IngameState.update` catches it, so the rest of that
      frame's `IngameState.update` is skipped and the text is logged via
      `ExceptionLogger`. (Engine, #225: the decompiled `UpdateStuff` wraps
      `IsoFireManager.Update` in its own `catch (Exception)`, and
      `MapCollisionData.updateMain` through `PolygonalMap2.updateMain` in
      one `catch (Throwable)`; so a fire `Err` is logged and the frame goes
      on, a population `Err` is logged and skips the pathfinder's pass.)
- [ ] Fire world parts (#FIRE-REST): `FireWorld.mutation` (`FireMutation`:
      the regions, door tables and the `SquareMutationHooks` /
      `PathfindSquareHooks` / `RenderSquareHooks` the square mutations
      use), also in `StatesFire.mutation`. `WorldCall::FireManager` /
      `FireStopSound` from a fire removed by other code (Lua, mutations)
      go to `IsoFireManager::remove` / `sounds.remove_fire`.
- [ ] Fire sound slots (#FIRE-REST): `FireSoundHooks::fire_slot_play` /
      `fire_slot_stop` drive 20 emitters (`FMODSoundEmitter` +
      `ParameterFireSize`); `FireHooks::listener_distance_sq` is
      `FMODParameterUtils.getClosestListenerDistanceSquared`.
- [ ] Heat sources (#FIRE-REST): add/resize/remove the cell's
      `IsoHeatSource`s from `IsoFire::heat_source` (permanent fires, in
      `fire_updated` / `fire_removed`), `IsoFireplace::heat_source()` and
      `IsoBarbecue::heat_source()` after each hearth update; their
      `running_sound()` loops on the object's emitter.
- [ ] Hutch burning (#FIRE-REST): `FireHooks::hutch_release_contents`
      runs `releaseAllAnimals` + `dropAllEggs` of the hutch object.
- [ ] Fire fighting (#FIRE-REST): the put-out action uses
      `fire::fighting::{get_square_to_extinguish, get_extinguisher}` with
      an `ExtinguisherItem` view of the inventory items.
- [x] States: `StatesWorldAdapter` + engine `StatesEngine` (#88).
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
- [x] Player load (`PlayerDB.loadLocalPlayer`/`loadPlayer`): the bytes of
      `PlayerDb::load_local_player(sql_id)` (#100) go to
      `GameCharacter::load_player(sm, chars, ctx, SaveEnv { items:
      ScriptItems over the save's WorldDictionary, tables }, reader,
      world_version, set_on_fire)` on a fresh `GameCharacter::new(.., Player)`
      with a descriptor; `set_on_fire` runs `pz_world::fire::character::set_on_fire`
      with the world's fire manager. Then, for an `isDead` row,
      `setOverallBodyHealth(0)`/`setHealth(0)`; `saved_vehicle` seats the
      player; save with `save_player` (#101). Wired in
      `IsoWorld::load_player` over the save's `ScriptItems` (#212). Left:
      `set_on_fire` is only logged (no `IsoFireManager` on `IsoWorld`),
      `saved_vehicle` seating, and saving through `save_player` (#212).
- [ ] `WorldDictionary.init()` (#212): `world_dictionary::init` ports
      `DictionaryData.load()` only. Still to port: `collectObjectNames`,
      `collectStrings`, `parseInfoLoadList`/`parseCurrentInfoSet` (ids for
      scripts new since the save, removed or mod-changed items, new games),
      `parseObjectNameLoadList`, the `StringDictionary`/`ScriptsDictionary`
      registers, `getDictionaryMods`, `backupCurrentDataSet`, `save()`,
      `saveAsText("WorldDictionaryReadable.lua")`, `WorldDictionaryLogger`
      and `createErrorBackups`; then `ScriptManager.PostWorldDictionaryInit`
      and `WorldDictionary.onWorldLoaded`. A new game has no registry until
      then (chunk items load raw). `world_dictionary::script_items` still
      needs `recorded_media` (`ZomboidRadio.getRecordedMedia()`),
      `channels` and `map_definitions` (#212).
- [ ] Square mutations: every object change after load goes through
      `pz_world::square_mutation::MutationEnv` (`add_tile_object`,
      `add_special_object`, `remove_tile_object`,
      `transmit_remove_item_from_square`, `add_floor`,
      `remove_tile_object_erosion_no_recalc`, `add_corpse` / `remove_corpse`,
      `create_new_grid_square`) with `num_players` and
      `DoorTables::from_jar` (#97).
- [ ] Items on the floor: every `AddWorldInventoryItem` goes through
      `MutationEnv::add_world_inventory_item(pos, item, x, y, height,
      world_age_hours)` (`WorldItemPlaced` says what it became). The world
      implements the new `SquareMutationHooks` methods: `corpse_from_item`
      (`loadCorpseFromByteData`), `generator_from_item` (`IsoGenerator`),
      `animal_dropped` (`IsoAnimal`), `process_items_remove`,
      `designation_zone_item_on_ground` (#186).
- [ ] Character item drops (#186, characters side #189): the characters
      implement `floor_drop::DropCharacter` through
      `pz_characters::character::CharacterDrop` and take the world as a
      `FloorCx { ctx, floor }`. Wired: `set_worn_item` / `remove_worn_item`
      (`forceDropTooHeavy` right after `wornItems.setItem`),
      `climbThroughWindow` / `climbThroughWindowFrame`, `climbSheetRope`,
      `climbDownSheetRope` and `IsoPlayer.climbOverWall` (`dropHeavyItems`),
      `ClimbOverFenceState`'s `VaultSprintFallLanded`, `fallenOnKnees`,
      `handleLandingImpact` and `IsoPlayer.OnDeath` (`dropHandItems`)
      (#214). The engine must:
      - return its `square_mutation::FloorDropEnv::new(&mut env,
        world_age_hours)` from `StateWorld::floor()` for the states;
      - `bind_character_sim::install_floor_world(Some(..))` with a
        `FloorWorld` that builds a `FloorDropEnv` per call, for Lua's
        `setWornItem` / `removeWornItem` / `setClothingItem_*` and the new
        `dropHandItems` / `dropHeavyItems` (without one nothing drops);
      - `Use`'s `Unwear` (`apply_use_outcome`) passes no world yet, so a
        used-up worn item never drops (#189).
- [ ] Wall and sheet-rope climbs, landings and player death (#214, on
      #208): the climb states read the world through #208's hooks
      (`sheet_rope_square`, `chunk_levels`, `sheet_rope_exit`,
      `can_climb_over_wall`, `climb_wall_fence_type`); `pz-world-states`
      also implements the new `floor_sprite_name` (the landing's natural
      floor). The engine must:
      - implement `StateWorld::path_reset` (`PathFindBehavior2.reset()`, a
        failed wall climb; `pz-world-states` leaves the no-op default, as
        for `path_cancel`);
      - start the climbs: `GameCharacter::climb_over_wall(dir, cx)` from
        the player's wall-climb input (the `PathMoverRequest::ClimbOverWall`
        handler in `run_path_requests` already calls it),
        `climb_sheet_rope` / `climb_down_sheet_rope` from the rope input,
        and `fall_from_rope` where the original lets go;
      - call `GameCharacter::do_land(impact_iso_speed, cx)` when a falling
        character lands (`IsoGameCharacter.updateFalling`, not yet ported)
        and `player_on_death(cx)` from `IsoPlayer.OnDeath`;
      - answer `SimContext::debug_always_trip_over_fence` (`DebugOptions`;
        `easy_climbing` is #208's line);
      - handle `SimEvent::MusicIntensityEvent`, `FallingItem`
        (`createFallingItem`), `FallenHat`
        (`PersistentOutfits.setFallenHat`), `StopPlayerVoiceSound`,
        `CharacterDeath` (`OnCharacterDeath`), `FadeIn` and `PlayerDeath`
        (the rest of `IsoPlayer.OnDeath`: death music, `OnPlayerDeath`,
        leaving the vehicle, `removeSaveFile`, `setDrag`);
      - bind the new calls to Lua (`climbSheetRope`, `canClimbSheetRope`,
        `canClimbOverWall`, `climbOverWall`, `isClimbingRope`,
        `fallFromRope`, `getClimbingFailChanceInt`, `fallenOnKnees`, ...)
        where the Lua API needs them; none is bound yet.
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
- [ ] Radio: keep one `pz_radio::ZomboidRadio` per world
      (`ZomboidRadio::new(GameTime hour)`, with the game mode set) and
      implement `RadioWorld` (GameTime hour / nightsSurvived / multiplier,
      `SandboxOptions.timeSinceApo`, `Core.gameMode`, weather
      interference, the current save dir or `None` for no-save, local
      players and registered devices, `AddDeviceText`, the
      `OnLoadRadioScripts` / `OnInitRecordedMedia` Lua events). In
      `IsoWorld.init` call `init(savedWorldVersion or -1, &RadioSource
      { media dir, mods, language }, translator getText, world, Rand)`;
      call `update_scripts(hour, minute)` from `GameTime` on every
      ten-minute change, `update` once per frame in `IngameState`, `save`
      from `GameWindow` world saving, and drop the radio on
      `IngameState` exit (`Reset`), carrying `louisville_obfuscation`
      over (a static in the original). Devices call `register_device` /
      `unregister_device`; DeviceData preset generation takes the radio
      as its `ChannelLists` (#200).

- [ ] Windows (#208): `StatesEngine::player_input` returns the local
      player's frame `PlayerInput` (Open/Close/Smash window states read
      `pressedMovement` / `pressedCancelAction` from it); `toggle_window`
      must run `IsoWindow.ToggleWindow` (also used by `OpenWindowState`
      and `CloseWindowState`); `weapon_hit_thumpable` fires Lua
      `OnWeaponHitThumpable`; `vehicle_window_destroyed` /
      `vehicle_window_hit` from pz-vehicles (`smashCarWindow`).
      `window_closing_success` should answer from
      `pz_characters::character::states::window::closing_window(chr)` of
      the characters on the window's squares. `SimContext::easy_climbing`
      = `SandboxOptions.easyClimbing`. Lua `ISOpenCloseWindow` and
      `ISSmashWindow` call `GameCharacter::open_window`, `close_window`,
      `smash_window`, `smash_car_window`. Not ported: the debug
      window-unlock cheat, the `Hit_Door` render effect on a bare-handed
      barricade hit.

## World render pass
- [x] Cull chunk textures to the screen (`plan_chunks` / `level_on_screen`);
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
      sized to the player's view; each call's screen pass draws that
      player's screen rectangle with its own `ScreenParams` (#125, #181).
      `MultiTextureFBO2.render` also covers slots up to the highest player
      index without a player with a black `renderi(null, sx, sy, sw, sh)`;
      draw that rectangle black for such slots (#181).
- [ ] Screen shader (`WeatherShader`) inputs (#181): per player and frame
      build `pz_render::ScreenParams` and run it through one
      `ScreenTimer` kept for the session (`timer.apply(params,
      PerformanceSettings.lockFps)`, once per player screen pass and once
      per save thumbnail, in that order):
      `light`, `light_intensity`, `desaturation`, `night_value`,
      `exterior` and `night_vision_goggles` from
      `ClimateSettings::screen_params(player, params)` (pz-world-render,
      `updateRenderSettings`' screen half over the same `ClimateInput` and
      `PlayerRenderInput.night_vision_goggles` =
      `isWearingNightVisionGoggles()`) (#183); `drunk_factor` = intoxication / 100,
      `blur_factor` = `IsoPlayer.getBlurFactor()` (not updated for a dead
      player); `search` from the `SearchMode` port (characters/gameplay,
      not ported yet: `isShaderEnabled`, `getShaderBlur/Radius/
      GradientWidth/Desat/Darkness` by the player's square being outside a
      room); `offscreen_left/top` = `IsoCamera.getOffscreenLeft/Top`,
      `light`/`light_intensity` (`getBlendColor`/`getBlendIntensity`),
      `desaturation` (after `*= 1 - darkness`), `night_value`
      (`getDarkness` = 1 - day light strength; 0.25 in Last Stand),
      `exterior` from `RenderSettings.PlayerRenderSettings`
      (`updateRenderSettings`; pz-world-render's `ClimateSettings`
      computes the lighting half of it); `night_vision_goggles` =
      `isWearingNightVisionGoggles()`, `drunk_factor` = intoxication / 100,
      player); `search` from the `SearchMode` port (see the next item:
      `isShaderEnabled`, `getShaderBlur/Radius/GradientWidth/Desat/
      Darkness` by the player's square being outside a room); `offscreen_left/top` = `IsoCamera.getOffscreenLeft/Top`,
      `offscreen_width/height` = pz-core `Core` offscreen width/height,
      `right_click` = the player camera's `rightClickX/Y`, `zoom` =
      `Core.getZoom(player)`, `tile_width` = 64 at `Core.tileScale` 2 else
      32. Save thumbnail (#173, not in integration yet): pass
      `params.thumbnail(256, 256, 1.0)` (`startThumbnail`) to
      `capture_thumbnail`.
- [x] Screen shader player inputs and `SearchMode` (#188): in
      `IngameState.UpdateStuff` (after `WorldMapVisited.update`) call
      `pz_lua_api::bind_search_mode::update(GameTime.getTimeDelta())`; in
      `IngameState.exit` call `bind_search_mode::reset()`. Keep one
      `PlayerRenderInputs` per player slot for the session; each frame,
      where `RenderSettings.updateRenderSettings` runs for player `i`, call
      `bind_search_mode::update_render_inputs(i, &mut inputs[i])`, then
      build the screen pass's params with
      `bind_search_mode::apply_screen_inputs(params, i, &inputs[i])`
      (fills `night_vision_goggles`, `drunk_factor`, `blur_factor` and the
      `search` group; the latter picks exterior/interior values from
      `IsoPlayer.players[i]`'s `environment.has_current_square` /
      `square_in_room`, which the world must keep filled). On
      `OnClothingUpdated` for a player call `chr.update_vision_effects(sm)`
      (the blur target from glasses and `SHORT_SIGHTED`); the per-frame
      `updateVisionEffectTargets` already runs in `update_player`.
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

## World map
- [ ] Map inputs: `pz_lua_api::worldmap::host::install(WorldMapHost {
      players, meta_grid })` with the live local players (position, or
      the vehicle's; username) for the map's player dots, and
      `IsoMetaGrid`'s cell bounds for `setBoundsFromWorld` (#213). The
      default `meta_grid` (`host::WorldMetaGrid`) already reads the grid
      installed with `bind_world::set_meta_grid`; pass it when installing
      players (#223).
