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

- [ ] Ragdolls, boot: after ScriptManager loads `media/scripts/ragdolls`,
      `RagdollScripts::load_file` per file then `to_bullet(&mut bullet,
      false)` (`RagdollScript.toBullet`); once the world exists
      (`IsoChunk.addRagdollControllers`), `RagdollBuilder::initialize` with
      the `bob/bob_tpose` skeleton and the `Bob_TPose` clip; keep one
      `RagdollGlobals` (max active ragdolls from Core) (#102).
- [ ] Ragdolls, per character: keep an `Option<RagdollController>` beside
      its `AnimationPlayer` and update the player with
      `update_with_ragdoll(dt, vars, &mut RagdollFrame { env: RagdollEnv {
      bullet, globals, builder, character, bullet_frame_no:
      WorldSimulation's frame number, time_delta, debug }, controller,
      multiplier })` instead of `update`; implement `RagdollCharacter` on
      the character (`canRagdoll`, fall flags, `setOnFloor`,
      `doDeferredMovementFromRagdoll`, ballistics hits). Before `stopAll`
      or releasing the player call `release_ragdoll_controller(Some(host))`
      so the ragdoll leaves the world; `vehicle_collision` from
      `IsoZombie`'s vehicle contact (#102).
- [ ] Ragdoll corpses: when an `IsoDeadBody` is made from a character,
      `DeadBody.ragdoll_fall = isRagdollFall()` and
      `died_bone_transforms = pz_ragdoll::corpse::died_bone_transforms(
      player, ragdoll_fall, can_use_current_pose_for_corpse(player,
      culled, has_active_model))`; from saves,
      `corpse::died_bone_transforms_from_saved(n, &ragdoll bones)` with
      pz-world's `IsoDeadBody.ragdoll` (#102).

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
