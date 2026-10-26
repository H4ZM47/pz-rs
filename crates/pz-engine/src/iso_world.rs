//! `zombie.iso.IsoWorld`: the loaded world and `IsoWorld.init`, the world
//! load `GameLoadingState`'s loader runs, in the original's order over the
//! subsystem crates.
//!
//! The meta grid and the current cell live in `pz_lua_api::bind_world`
//! (`IsoWorld.instance.metaGrid` / `currentCell`, which Lua reads); the
//! rest of the world state is here.

use pz_characters::character::save::SaveEnv;
use pz_characters::character::DefaultSimContext;
use pz_characters::CharacterKind;
use pz_core::save_trackers::{InstanceTracker, MetaTracker, Statistics};
use pz_kahlua::Value;
use pz_lua_api::bind_character_sim::{character_value, LuaGameCharacter, LuaGameCharacterRef};
use pz_lua_api::bind_characters::LuaSurvivorDesc;
use pz_lua_api::bind_world::{set_cell, set_meta_grid, with_world};
use pz_lua_api::player_db::{self, LocalPlayerLoad, PlayerDb, VehicleStore};
use pz_lua_api::LuaManager;
use pz_world::chunk::IsoCell;
use pz_world::fix2x::Fix2xTable;
use pz_world::isoregion::IsoRegions;
use pz_world::meta_save::{DefaultMetaEnv, MetaWorld};
use pz_world::save::items::AllItems;
use pz_world::save::{NoSaveHooks, SaveSettings, SaveTables};
use pz_world::streamer::{calc_chunk_width, ChunkStreamer, StreamCtx};
use pz_world::zone_save::{ClockUuids, SpawnedZombieZones};
use pz_world::{
    CreateParams, GameRand, IsoMetaGrid, IsoSpriteManager, NoHooks, ObjectLoadSettings,
    ObjectTables, SpriteTables, WorldGenBounds,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// `IsoWorld.WorldVersion` in this build (the save format the port reads).
pub const WORLD_VERSION: i32 = 249;

/// `Rand` (`RandStandard.INSTANCE`) as the world crates draw from it.
pub struct StandardRand;

impl GameRand for StandardRand {
    fn next(&mut self, max: i32) -> i32 {
        pz_core::random::next(max)
    }
    fn next_float(&mut self) -> f32 {
        pz_core::random::next_float(0.0, 1.0)
    }
}

/// Chunks that entered (`doLoadGridsquare`) or left (`removeFromWorld`)
/// the cell, in order, for the world renderer.
#[derive(Default)]
pub struct RenderChunkEvents {
    pub added: Vec<(i32, i32)>,
    pub removed: Vec<(i32, i32)>,
}

/// What the main menu's Lua leaves on `IsoWorld.instance` for a new game
/// (`CharacterCreationProfession.initWorld` on `OnInitWorld`: the player's
/// descriptor, traits and spawn square). The menu cannot run that yet
/// (`getWorld()` is not bound and the menu is blocked on the world map),
/// so the development entry supplies it.
#[derive(Clone, Debug, Default)]
pub struct NewGameSetup {
    /// `luaPosX/Y/Z`; `None` picks a point of the map's
    /// `spawnpoints.lua` as `initWorld` does (the `unemployed` list).
    pub spawn: Option<(i32, i32, i32)>,
    /// `WorldGenParams.INSTANCE.setSeedString` (`MapSpawnSelect.saveGenParams`).
    pub seed: String,
}

/// `IsoWorld.instance` and the world singletons `IsoWorld.init` creates.
pub struct IsoWorld {
    pub game_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// `ZomboidFileSystem.getCurrentSaveDir()`.
    pub save_dir: PathBuf,
    /// `IsoWorld.savedWorldVersion` (`-1` for a new game).
    pub saved_world_version: i32,
    /// `spawnRegionName`.
    pub spawn_region: String,
    /// `luaPosX/Y/Z`.
    pub lua_pos: (i32, i32, i32),
    /// `luaDesc`.
    pub lua_player_desc: Option<Rc<RefCell<LuaSurvivorDesc>>>,
    /// `luatraits`.
    pub lua_traits: Vec<Option<Rc<pz_scripts::registry::RegistryKey>>>,
    /// The development stand-in for the menu's `OnInitWorld` handler.
    pub new_game_setup: Option<NewGameSetup>,
    /// `hydroPowerOn`.
    pub hydro_power_on: bool,
    /// `IsoWorld.worldX/worldY`: the chunk player 0's map starts on.
    pub world_x: i32,
    pub world_y: i32,
    /// `IsoChunkMap.worldXa/Ya/Za`.
    pub world_xa: (i32, i32, i32),
    pub world_gen_params: pz_lua_api::bind_world_gen_params::WorldGenParamsRef,
    pub world_gen: Option<pz_worldgen::WorldGenHook>,
    /// `Basements.getInstance()` after `beforeLoadMetaGrid`, handed to the
    /// world generator (`onNewChunkLoaded`).
    pub basements: Option<pz_worldgen::basements::Basements>,
    /// `IsoSpriteManager.instance`.
    /// `IsoSpriteManager.instance`, shared with its Lua object.
    pub sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
    pub object_tables: Option<ObjectTables>,
    pub save_tables: Option<SaveTables>,
    pub save_settings: SaveSettings,
    pub fix2x: Fix2xTable,
    pub rand: StandardRand,
    /// `VehiclesDB2.instance`'s store.
    pub vehicles: VehicleStore,
    /// `PlayerDB.getInstance()` (`PlayerDB.setAllow(true)`).
    pub player_db: Option<PlayerDb>,
    /// `WorldDictionary.isNewGame` (`GameLoadingState.newGame`, set before
    /// the loader runs).
    pub dictionary_new_game: bool,
    /// The save's item registry: `WorldDictionary.data` over the scripts
    /// (`None` until `WorldDictionary.init()` read a `WorldDictionary.bin`).
    pub items: Option<pz_inventory::factory::ScriptItems>,
    /// `WorldStreamer.instance` with the chunk maps.
    pub streamer: Option<ChunkStreamer>,
    /// A failed native activation stops subsequent handoffs and retains the
    /// actual cursor/receiver for diagnostics and an explicit load abort.
    pub failed_chunk_activation: Option<(pz_world::save::chunk_file::ChunkActivationCursor, String)>,
    /// `IsoMetaGrid`'s part of `map_meta.bin` the grid does not keep.
    pub meta_world: MetaWorld,
    pub spawned_zones: SpawnedZombieZones,
    pub uuids: ClockUuids,
    /// `PersistentOutfits.getOutfitNames()`.
    pub outfit_names: Vec<String>,
    /// `ZombiePopulationManager.instance` and `MapCollisionData.instance`.
    pub population: Option<pz_world_popman::WorldPopulation<i32>>,
    /// `IsoRegions`.
    pub regions: Option<IsoRegions>,
    /// `PathfindNative`/`PolygonalMap2.instance`.
    pub pathfind: Option<pz_pathfind::host::PathHost>,
    /// `ErosionMain.getInstance()`.
    /// `ErosionMain.getInstance()`, shared with its Lua object.
    pub erosion: Option<Rc<RefCell<pz_erosion::erosion_main::ErosionMain>>>,
    pub instance_tracker: InstanceTracker,
    pub meta_tracker: MetaTracker,
    pub statistics: Statistics,
    /// The sandbox preset `map_sand.bin` named.
    pub preset: String,
    /// `IsoCell.getZombieList()`: the real zombies.
    pub zombies: Vec<LuaGameCharacter>,
    /// The last `IsoMovingObject` id handed out (`IDMax`).
    pub next_object_id: i32,
    /// Object state the character states keep outside the objects.
    pub states_runtime: pz_world_states::StatesRuntime,
    /// `ActionGroup.getActionGroup(name)`'s loaded groups.
    pub action_groups: pz_characters::character::state::ActionGroups,
    /// `IsoFireManager`.
    pub fire: pz_world::fire::IsoFireManager,
    pub fire_flags: Option<pz_world::fire::manager::FireFlags>,
    /// Each local player's `IsoPlayer.sqlId` (its `localPlayers` row; -1
    /// before its first save).
    pub player_sql_ids: [i32; 4],
    /// `IsoDoor`'s double-door offset tables, for the square mutations.
    pub door_tables: Option<pz_world::square_mutation::DoorTables>,
    /// `IsoWorld.frameNo`.
    pub frame_no: i64,
    /// The last `IsoChunk.loadID` handed out.
    pub chunk_load_id: i16,
    /// `VirtualZombieManager.getKeySpawnChanceD100()`.
    pub key_spawn_chance_d100: f32,
    /// Chunks that entered or left the cell since the renderer last looked.
    pub render_chunks: RenderChunkEvents,
    /// The world renderer (`IsoCell.render`'s state), made on the first
    /// rendered frame.
    pub render: Option<crate::world_render::WorldRender>,
    /// Live character models and animation players for this world.
    pub models: Option<crate::world_models::WorldModels>,
    /// Errors of steps that failed without stopping the load (the
    /// original's `ExceptionLogger.logException` calls).
    pub errors: Vec<String>,
}

/// A sandbox option's value as a number (0 when unknown).
pub(crate) fn opt_f64(sb: &pz_sandbox::SandboxOptions, name: &str) -> f64 {
    sb.option_by_name(name)
        .and_then(|o| o.config.get_value_as_string().parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// A boolean sandbox option (false when unknown).
pub(crate) fn opt_bool(sb: &pz_sandbox::SandboxOptions, name: &str) -> bool {
    sb.option_by_name(name)
        .is_some_and(|o| o.config.get_value_as_string() == "true")
}

pub(crate) fn opt_int(sb: &pz_sandbox::SandboxOptions, name: &str) -> i32 {
    sb.option_by_name(name)
        .and_then(|o| o.config.get_value_as_string().parse::<f64>().ok())
        .map_or(0, |v| v as i32)
}

/// `SandboxOptions.instance` (created on first use, as Lua does).
pub(crate) fn sandbox(lua: &mut LuaManager) -> Option<Rc<RefCell<pz_sandbox::SandboxOptions>>> {
    let v = pz_lua_api::bind_sandbox::instance(&mut lua.thread).ok()?;
    let Value::Object(o) = v else { return None };
    o.as_any()
        .downcast_ref::<pz_lua_api::bind_sandbox::SandboxObj>()
        .map(|s| s.0.clone())
}

fn sandbox_vars(lua: &LuaManager) -> Option<pz_kahlua::TableRef> {
    match lua.thread.globals().get_str("SandboxVars") {
        Value::Table(t) => Some(t),
        _ => None,
    }
}

/// Retire whole square and object owners after all callbacks and
/// world/sprite guards. A failed frame may leave these pinned until the
/// next successful boundary or the cell's borrow-free shutdown.
pub(crate) fn drain_retired_world_objects() {
    let retired = with_world(|world| world.cell.as_mut().map(IsoCell::take_retired_owners));
    drop(retired);
}

impl IsoWorld {
    /// `new IsoWorld()` for the save `<cache>/Saves/<Core.gameMode>/<Core.gameSaveWorld>`.
    pub fn new(game_dir: &Path, cache_dir: &Path, save_dir: PathBuf) -> Self {
        IsoWorld {
            game_dir: game_dir.to_path_buf(),
            cache_dir: cache_dir.to_path_buf(),
            save_dir,
            saved_world_version: -1,
            spawn_region: String::new(),
            lua_pos: (-1, -1, -1),
            lua_player_desc: None,
            lua_traits: Vec::new(),
            new_game_setup: None,
            hydro_power_on: false,
            world_x: 0,
            world_y: 0,
            world_xa: (0, 0, 0),
            world_gen_params: pz_lua_api::bind_world_gen_params::shared_world_gen_params(),
            world_gen: None,
            basements: None,
            sprites: None,
            object_tables: None,
            save_tables: None,
            save_settings: SaveSettings::default(),
            fix2x: Fix2xTable::default(),
            rand: StandardRand,
            vehicles: VehicleStore::default(),
            player_db: None,
            dictionary_new_game: false,
            items: None,
            streamer: None,
            failed_chunk_activation: None,
            meta_world: MetaWorld::default(),
            spawned_zones: SpawnedZombieZones::default(),
            uuids: ClockUuids::default(),
            outfit_names: Vec::new(),
            population: None,
            regions: None,
            pathfind: None,
            erosion: None,
            instance_tracker: InstanceTracker::default(),
            meta_tracker: MetaTracker::default(),
            statistics: Statistics::default(),
            preset: String::new(),
            zombies: Vec::new(),
            next_object_id: 0,
            states_runtime: Default::default(),
            action_groups: pz_characters::character::state::ActionGroups::from_media(
                game_dir.join("media"),
            ),
            fire: pz_world::fire::IsoFireManager::new(),
            fire_flags: None,
            door_tables: None,
            player_sql_ids: [-1; 4],
            frame_no: 0,
            chunk_load_id: 0,
            key_spawn_chance_d100: 0.0,
            render_chunks: RenderChunkEvents::default(),
            render: None,
            models: None,
            errors: Vec::new(),
        }
    }

    fn jar(&self) -> PathBuf {
        self.game_dir.join("projectzomboid.jar")
    }

    fn log_error(&mut self, e: impl Into<String>) {
        let e = e.into();
        eprintln!("ERROR: IsoWorld.init: {e}");
        self.errors.push(e);
    }

    /// `ZomboidFileSystem.getFileInCurrentSave(name)`.
    pub fn file_in_save(&self, name: &str) -> PathBuf {
        self.save_dir.join(name)
    }

    /// Runs `f` with the chunk streamer, the current cell and a
    /// [`StreamCtx`] over the world's systems.
    pub fn with_stream<R>(
        &mut self,
        f: impl FnOnce(&mut ChunkStreamer, &mut IsoCell, &mut StreamCtx<'_>) -> R,
    ) -> Option<R> {
        let no_save = pz_lua_api::bind_core::shared_core().borrow().is_no_save();
        let IsoWorld {
            save_dir,
            sprites,
            object_tables,
            save_tables,
            save_settings,
            fix2x,
            rand,
            vehicles,
            streamer,
            world_gen,
            items,
            ..
        } = self;
        // `GameTime.getWorldAgeHours()` as the item loads read it.
        if let Some(items) = items.as_mut() {
            items.world_age_hours = pz_lua_api::bind_climate::shared_game_time()
                .borrow()
                .world_age_hours();
        }
        let items: &dyn pz_world::save::items::ItemRegistry = match items.as_ref() {
            Some(i) => i,
            None => &AllItems,
        };
        let (Some(streamer), Some(sprites), Some(object_tables), Some(save_tables)) = (
            streamer.as_mut(),
            sprites.as_ref(),
            object_tables.as_ref(),
            save_tables.as_ref(),
        ) else {
            return None;
        };
        let (result, retired) = {
            let mut sprites = sprites.borrow_mut();
            let (result, retired_objects, retired_chunks) = with_world(|w| {
                let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_mut()) else {
                    return (None, pz_world::RetiredWorldOwners::default(), Vec::new());
                };
                let mut object_hooks = NoHooks;
                let mut ctx = StreamCtx {
                    save_dir,
                    meta,
                    sprites: &mut sprites,
                    fix2x,
                    rand,
                    object_tables,
                    object_settings: ObjectLoadSettings::default(),
                    object_hooks: &mut object_hooks,
                    save_tables,
                    save_settings,
                    // The save's `WorldDictionary`; without one (a new game, see
                    // `world_dictionary::init`) items load raw.
                    items,
                    save_hooks: &NoSaveHooks,
                    brand_new: world_gen
                        .as_mut()
                        .map(|h| h as &mut dyn pz_world::chunk::BrandNewHook),
                    // TODO(port): `ErosionChunkHook` wrapped in
                    // `ClearSoundsOnUnload` (erosion and world sounds on load).
                    chunk_hook: None,
                    no_save,
                    vehicles_db: vehicles.conn.as_mut(),
                    // TODO(port): the vehicle runtime's savers, meta and ids
                    // (`RuntimeVehicleSaver`, `VirtualVehicleMeta`, `VehicleIds`).
                    vehicle_saver: None,
                    vehicle_meta: None,
                    vehicle_ids: None,
                };
                let result = Some(f(streamer, cell, &mut ctx));
                let mut chunks = streamer.take_retired_chunks();
                // These terminal chunks have finished their last save. Keep
                // whole square owners in the cell's retry queue, including
                // never-installed squares still borrowed by a retained user.
                // Move slots only; actual square fields remain untouched.
                for chunk in &mut chunks {
                    for square in chunk.take_square_owners_for_retirement() {
                        cell.retire_square(square);
                    }
                }
                (result, cell.take_retired_owners(), chunks)
            });
            (result, (retired_chunks, retired_objects))
        };
        // A queued floor item may be the last owner of a bag's opaque Lua
        // parent. Its destructor can reenter WORLD and the sprite manager.
        // Both guards above must end before these payloads are destroyed.
        drop(retired);
        result
    }

    /// `readWorldVersion()` (single player: `map_ver.bin`'s first int,
    /// `-1` when missing).
    fn read_world_version(&self) -> i32 {
        match std::fs::read(self.file_in_save("map_ver.bin")) {
            Ok(b) if b.len() >= 4 => i32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            _ => -1,
        }
    }

    /// The menu and direct-game entry update the same static parameters;
    /// a saved world's file then overwrites them in place.
    fn load_world_gen_params(&mut self) {
        let mut params = self.world_gen_params.borrow_mut();
        if let Some(setup) = &self.new_game_setup {
            params.seed_string = setup.seed.clone();
            params.seed = pz_worldgen::java::string_hash(&setup.seed);
        }
        let _ = params.load_file(&self.save_dir);
    }

    /// `IsoWorld.init()` on the loader (`GameLoadingState`'s
    /// `runInner`, between `IsoWorld.instance = new IsoWorld()` and
    /// `OnGameTimeLoaded`). Returns `Err` where the original throws out of
    /// the loader (`unexpectedError`).
    pub fn init(&mut self, lua: &mut LuaManager) -> Result<(), String> {
        // Server-system registration loads gos_*.bin from this actual world.
        pz_lua_api::global_objects_save::set_save_dir(Some(self.save_dir.clone()));
        let core_rc = pz_lua_api::bind_core::shared_core();
        let (tutorial, no_save, last_stand, game_mode) = {
            let c = core_rc.borrow();
            (
                c.tutorial,
                c.is_no_save(),
                c.is_last_stand(),
                c.game_mode.clone(),
            )
        };
        if !tutorial {
            // TODO(port): the `RB*` randomized buildings, `RVS*` vehicle
            // stories and `RZS*` zone stories lists (`randomizedBuildingList`,
            // `randomizedVehicleStoryList`, `randomizedZoneList`).
        }
        // TODO(port): `RBBasic.getUniqueRDSSpawned().clear()`;
        // `BodyDamageSync` (network only).
        self.saved_world_version = self.read_world_version();
        // `map_ver.bin`: the map the save was made on.
        if let Some(v) =
            pz_lua_api::savefile::read_map_ver(&self.file_in_save("map_ver.bin").to_string_lossy())
        {
            if let Some(map) = v.map_name {
                core_rc.borrow_mut().game_map = map;
            }
        }
        // TODO(port): `ObjectIDManager.load` (`id_manager_data.bin`) and
        // `ImportantAreaManager.load` (`important_area_data.bin`).
        // `WorldGenParams.INSTANCE.load()`; a new single-player world keeps
        // the seed the menu set (`MapSpawnSelect.saveGenParams`).
        self.load_world_gen_params();
        // `metaGrid.CreateStep1()`.
        let game_map = core_rc.borrow().game_map.clone();
        let wg = {
            let params = self.world_gen_params.borrow();
            WorldGenBounds {
                min_x: params.min_x_cell,
                min_y: params.min_y_cell,
                max_x: params.max_x_cell,
                max_y: params.max_y_cell,
            }
        };
        let meta = {
            let fs = lua.fs.borrow();
            IsoMetaGrid::create(
                &fs,
                CreateParams {
                    game_map: &game_map,
                    world_gen: wg,
                    rand: &mut self.rand,
                    // TODO(port): the `ZombieVoronoi` instances.
                    voronois: Vec::new(),
                },
            )
            .map_err(|e| format!("IsoMetaGrid.Create: {e}"))?
        };
        let mut meta = meta;
        // `Zone.s_PreferredZoneTypes` / `IsoDirections` for zone
        // registration, and the native clipper (`PZClipper`).
        meta.zone_tables = pz_world::zones::ZoneTables::load_from_game_jar(&self.jar())?;
        meta.clipper = Some(Box::new(crate::zone_clipper::ZoneClipperOps::default()));
        set_meta_grid(Some(meta));
        lua.trigger_event("OnPreDistributionMerge", &[]);
        lua.trigger_event("OnDistributionMerge", &[]);
        lua.trigger_event("OnPostDistributionMerge", &[]);
        // `VehiclesDB2.instance.init()`.
        if !no_save {
            if let Err(e) = self.vehicles.create(&self.save_dir.to_string_lossy()) {
                self.log_error(format!("VehiclesDB2.init: {e}"));
            }
        }
        println!("triggerEvent OnInitWorld");
        lua.trigger_event("OnInitWorld", &[]);
        self.apply_new_game_setup(lua);
        // `SandboxOptions.instance.load()`.
        let sb = sandbox(lua).ok_or("SandboxOptions.instance")?;
        self.load_sandbox(lua, &sb);
        pz_lua_api::bind_world::power::install_sandbox(Some(sb.clone()));
        // TODO(port): `ItemPickerJava.Parse()`.
        self.hydro_power_on = {
            let gt = pz_lua_api::bind_climate::shared_game_time();
            let days = gt.borrow().world_age_hours() as f32 / 24.0;
            sb.borrow().does_power_grid_exist(days, 0)
        };
        // TODO(port): `ZomboidGlobals.toLua()`,
        // `ItemPickerJava.InitSandboxLootSettings()`, `survivorDescriptors.clear()`.
        // `LoadTileDefinitions*` (property strings, custom values, lookup
        // tables, the definitions), `PostTileDefinitions`, then
        // `loadedTileDefinitions`.
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let entity_names: Vec<String> = sm
            .get_all::<pz_scripts::objects::entity::GameEntityScript>(
                pz_scripts::ScriptType::Entity,
            )
            .iter()
            .filter_map(|e| e.borrow().full_name().ok())
            .collect();
        let tables = SpriteTables::from_game_jar(&self.jar())?;
        // TODO(port): mod tile definitions (`loadModTileDefPropertyStrings`,
        // `loadModTileDefs`) and `registerFakeJumboTree`.
        let mut sprites = IsoSpriteManager::load_tile_definitions_from(
            tables,
            &self.game_dir.join("media"),
            &entity_names,
        )?;
        sprites.post_tile_definitions(&pz_world::sprite::EntitySpriteConfig::from_scripts(&sm));
        println!("triggerEvent OnLoadedTileDefinitions");
        // TODO(port): the `IsoSpriteManager` Lua object (no binding yet).
        lua.trigger_event("OnLoadedTileDefinitions", &[Value::Nil]);
        // `!IsoSprite.hasNoTextures()`: a client has every tile's textures.
        // TODO(port): the world tile packs are not loaded at boot yet.
        sprites.loaded_tile_definitions(&|_| true)?;
        let sprites = Rc::new(RefCell::new(sprites));
        pz_lua_api::bind_sprites::install_sprite_manager(Some(sprites.clone()));
        self.sprites = Some(sprites);
        self.object_tables = Some(ObjectTables::from_game_jar(&self.jar())?);
        self.save_tables = Some(SaveTables::from_game_jar(&self.jar())?);
        self.door_tables = Some(pz_world::square_mutation::DoorTables::from_jar(
            &pz_classfile::Jar::open(self.jar()).map_err(|e| e.to_string())?,
        )?);
        // TODO(port): `AnimalDefinitions.getAnimalDefs()`
        // (`pz_animals::AnimalDefinitionTables::load`), `WeatherFxMask.init`,
        // `TemplateText.Initialize`.
        // `IsoRegions.init()`.
        let chunk_grid_width = {
            let c = core_rc.borrow();
            calc_chunk_width(c.get_screen_width(), c.get_screen_height())
        };
        self.regions = Some(IsoRegions::init_world(
            &self.save_dir,
            chunk_grid_width,
            no_save,
        ));
        // `BentFences.init()` (empty): the registry reads the sandbox's
        // `ZombieLore.FenceThumpersRequired` / `FenceDamageMultiplier`.
        {
            let s = sb.borrow();
            let multiplier = s
                .option_by_name("ZombieLore.FenceDamageMultiplier")
                .and_then(|o| o.config.get_value_as_string().parse::<f64>().ok())
                .unwrap_or(1.0) as f32;
            let bf = pz_lua_api::bind_bent_fences::shared_bent_fences();
            let mut bf = bf.borrow_mut();
            bf.thumpers_required = opt_int(&s, "ZombieLore.FenceThumpersRequired");
            bf.damage_multiplier = multiplier;
        }
        // TODO(port): `ObjectRenderEffects.init`,
        // `WorldConverter.instance.convert` (and its `worldVersionError`).
        // TODO(port): `SandboxOptions.handleOldZombiesFile2` (old `zombies.ini`).
        // `GameTime.init()` then `GameTime.load()`.
        {
            let gt = pz_lua_api::bind_climate::shared_game_time();
            let mut ctx = pz_lua_api::bind_climate::LuaClimateContext::new();
            let mut g = gt.borrow_mut();
            g.init(&mut ctx);
            let mut core = core_rc.borrow_mut();
            if let Err(e) = pz_core::game_time_save::load_map_t_file(
                &mut g,
                &mut core,
                &self.save_dir,
                self.saved_world_version,
            ) {
                drop((g, core));
                self.log_error(format!("GameTime.load: {e}"));
            }
        }
        // TODO(port): `ImprovedFog.init`; `ZomboidRadio.Init(savedWorldVersion)`
        // (`pz_radio::ZomboidRadio::init` needs the engine's `RadioWorld`);
        // `GlobalModData.init`.
        if let Err(e) = self.instance_tracker.load_file(&self.save_dir, no_save) {
            self.log_error(format!("InstanceTracker.load: {e}"));
        }
        // `ErosionGlobals.Boot` → `ErosionMain` (its `start()` reads
        // `erosion.ini`), whose season `OnInitSeasons` hands to Lua.
        self.boot_erosion(lua, &sb, no_save);
        // `WorldDictionary.init()`: the save's item registry.
        println!("WorldDictionary.init() start");
        let object_names = self
            .erosion
            .as_ref()
            .map(|e| e.borrow().world.get_object_names())
            .unwrap_or_default();
        let dictionary = crate::world_dictionary::init(
            &self.save_dir,
            no_save,
            self.dictionary_new_game,
            &sm,
            &object_names,
        )?;
        sm.post_world_dictionary_init();
        println!("WorldDictionary.init() end");
        self.items = match (dictionary, self.save_tables.as_ref()) {
            (Some(d), Some(tables)) => {
                let world_age_hours = pz_lua_api::bind_climate::shared_game_time()
                    .borrow()
                    .world_age_hours();
                let sprites = self.sprites.as_ref().map(|s| s.borrow());
                let inputs = crate::world_dictionary::RegistryInputs {
                    game_dir: &self.game_dir,
                    save_tables: tables,
                    sprites: sprites.as_deref(),
                    world_age_hours,
                };
                Some(crate::world_dictionary::script_items(d, &inputs))
            }
            _ => None,
        };
        // TODO(port): `FishSchoolManager.init`; `WorldMarkers.instance.init`;
        // `GameEntityManager.Init`.
        // `PersistentOutfits.instance.init()`.
        self.outfit_names = pz_lua_api::bind_zombie_outfits::init_persistent_outfits(
            &lua.thread,
            Some((&self.file_in_save("z_outfits.bin"), no_save)),
        );
        // TODO(port): `VehicleIDMap.Reset`, `new VehicleManager()`.
        // `metaGrid.CreateStep2()`: the building alarms (the adjacent-cell
        // rooms ran in `IsoMetaGrid::create`).
        {
            let s = sb.borrow();
            let alarm = opt_int(&s, "Alarm");
            let alarm_decay = opt_int(&s, "AlarmDecay");
            let power = self.hydro_power_on;
            let rand = &mut self.rand;
            with_world(|w| {
                if let Some(g) = w.meta_grid.as_mut() {
                    g.assign_alarms(rand, last_stand, power, alarm, || {
                        s.random_alarm_decay(alarm_decay)
                    });
                }
            });
        }
        let bounds = with_world(|w| {
            w.meta_grid
                .as_ref()
                .map(|g| (g.min_x, g.min_y, g.max_x, g.max_y))
        });
        pz_lua_api::bind_climate::init_climate_manager(&mut lua.thread, bounds);
        // TODO(port): `SafeHouse.init`, `VirtualZombieManager.instance.init`,
        // `StashSystem.init`.
        pz_lua_api::bind_basements::before_on_load_map_zones();
        lua.trigger_event("OnLoadMapZones", &[]);
        self.before_load_meta_grid();
        // TODO(port): `BuildingRoomsEditor.load`.
        self.load_meta_grid_files();
        // `Basements.afterLoadMetaGrid()` releases the spawn locations and
        // headers; the port keeps the headers for `onNewChunkLoaded`.
        println!("triggerEvent OnLoadedMapZones");
        lua.trigger_event("OnLoadedMapZones", &[]);
        // TODO(port): `ItemConfigurator.Preprocess()`.
        let save_dir = self.save_dir.to_string_lossy().into_owned();
        let is_player_alive = pz_lua_api::savefile::is_player_alive(no_save, &save_dir, 1);
        // `PlayerDB.setAllow(true)`.
        self.player_db = Some(PlayerDb::new(no_save, &save_dir));
        let load_character;
        if is_player_alive {
            load_character = true;
            // `LoadPlayerForInfo()`.
            let db = self.player_db.as_mut().unwrap();
            match player_db::load_player_for_info(db, &mut self.vehicles, &save_dir, (0, 0)) {
                LocalPlayerLoad::Db(Some(info)) => {
                    self.world_xa = (info.world_xa, info.world_ya, info.world_za);
                    self.world_x = info.s_world_x;
                    self.world_y = info.s_world_y;
                }
                LocalPlayerLoad::Db(None) => return Ok(()),
                // TODO(port): pre-database saves (`map_p.bin`).
                LocalPlayerLoad::MapPBin(p) => {
                    return Err(format!("{p}: map_p.bin saves are not supported"))
                }
            }
        } else {
            load_character = false;
            self.world_xa = self.lua_pos;
            self.world_x = pz_world::java::fastfloor(self.world_xa.0 as f32 / 8.0);
            self.world_y = pz_world::java::fastfloor(self.world_xa.1 as f32 / 8.0);
        }
        // TODO(port): `selectedDebugScenario` start location.
        // `MapCollisionData.init` and `ZombiePopulationManager.init`.
        self.init_population(&sb, &game_mode, last_stand, no_save);
        // TODO(port): `AnimalPopulationManager.init`.
        // Pathfinding: `PolygonalMap2.instance.init(metaGrid)`.
        let pf = with_world(|w| {
            w.meta_grid
                .as_ref()
                .map(|g| (g.min_x, g.min_y, g.width, g.height))
        });
        if let Some((x, y, wd, h)) = pf {
            self.pathfind = Some(pz_pathfind::host::PathHost::new(
                pz_pathfind::service::PathfindService::new(x, y, wd, h, false),
            ));
            // GlobalObjectLookup.init(metaGrid), before the cell is loaded.
            pz_lua_api::global_objects::init_lookup(pz_lua_api::global_objects::GridBounds {
                min_x: x,
                min_y: y,
                width: wd,
                height: h,
            });
        }
        // TODO(port): `SpawnPoints.initSinglePlayer`.
        // `WorldStreamer.instance.create()` and
        // `CellLoader.LoadCellBinaryChunk`: the world generator for
        // brand-new chunks, then player 0's chunk map around (worldX, worldY).
        self.world_gen = self.load_world_gen(lua, &sb, &game_map);
        if let Some(g) = self.world_gen.as_mut() {
            g.basements = self.basements.take();
        }
        self.streamer = Some(ChunkStreamer::new(1, chunk_grid_width));
        self.streamer.as_mut().unwrap().set_deferred_world_activation(true);
        set_cell(Some(IsoCell::default()));
        let (wx, wy) = (self.world_x, self.world_y);
        self.with_stream(|s, cell, ctx| s.load_world(wx, wy, cell, ctx));
        self.activate_streamed_chunks(lua)?;
        self.hand_off_events();
        // `ClimateManager.postCellLoadSetSnow()`.
        let snow = pz_lua_api::bind_climate::post_cell_load_set_snow(&mut lua.thread);
        // TODO(port): `IsoCell.setSnowTarget` and `ErosionIceQueen.setSnow`
        // from the returned effects.
        let _ = snow;
        // TODO(port): `MapCollisionData.instance.start()` (the worker
        // thread starts in `startGame`); `MapItem.LoadWorldMap()`.
        // The streamer ran until idle and every chunk was handed off
        // (`setChunkDirect`, `calculateZExtentsForChunkMap`) in `load_world`.
        if load_character {
            // `currentCell.LoadPlayer(savedWorldVersion)`.
            self.load_player(&save_dir)?;
            // TODO(port): `RecordedMedia.handleLegacyListenedLines`.
        } else {
            // TODO(port): `RecordedMedia.handleLegacyListenedLines(null)`.
            if pz_lua_api::bind_players::num_players() == 0 {
                pz_lua_api::bind_players::set_num_players(1);
            }
            self.create_new_player(lua, &sm)?;
        }
        // `PlayerDB.getInstance().canSavePlayers = true`.
        // TODO(port): `PlayerDB.canSavePlayers` (kept by the save path);
        // `TutorialManager.activeControlZombies`;
        // `ReanimatedPlayers.loadReanimatedPlayers`.
        if let Some(p) = pz_lua_api::bind_players::player_instance() {
            // TODO(port): `ScriptManager.checkAutoLearn`, `checkMetaRecipes`,
            // `VerifyAllCraftRecipesAreLearnable`.
            // `IsoPlayer.setCurrentSquareFromPosition()`.
            let mut c = p.borrow_mut();
            let (x, y, z) = (
                pz_world::java::fastfloor(c.vars.x),
                pz_world::java::fastfloor(c.vars.y),
                pz_world::java::fastfloor(c.vars.z),
            );
            let exists = with_world(|w| {
                w.cell
                    .as_ref()
                    .is_some_and(|cell| cell.get_grid_square(x, y, z).is_some())
            });
            c.path.current_square = exists.then_some((x, y, z));
        }
        // TODO(port): `PopulateCellWithSurvivors`, the cell's object list,
        // `LightingThread.instance.create()`.
        if let Err(e) = self.meta_tracker.load_file(
            &self.save_dir,
            no_save,
            pz_lua_api::bind_players::player(0).is_some(),
        ) {
            self.log_error(format!("MetaTracker.load: {e}"));
        }
        if let Err(e) = self.statistics.load_file(&self.save_dir, no_save) {
            self.log_error(format!("StatisticsManager.load: {e}"));
        }
        // TODO(port): `AnimationPlayerRecorder.initMessaging`,
        // `WorldDictionary.onWorldLoaded`, and the script / sprite-config
        // load-error check.
        pz_lua_api::bind_puddles::init_weather_fx(&lua.fs.borrow());
        let chunks = with_world(|w| w.cell.as_ref().map_or(0, |c| c.chunks.len()));
        let pos = pz_lua_api::bind_players::player(0).map(|p| {
            let p = p.borrow();
            (p.vars.x, p.vars.y, p.vars.z)
        });
        println!(
            "IsoWorld.init: chunk map at {},{}, {chunks} chunks in the cell, player 0 at {pos:?}",
            self.world_x, self.world_y
        );
        Ok(())
    }

    /// The development stand-in for `CharacterCreationProfession.initWorld`
    /// (`OnInitWorld`): a `SurvivorFactory.CreateSurvivor()` descriptor and
    /// a spawn point of the map's `spawnpoints.lua`.
    fn apply_new_game_setup(&mut self, lua: &mut LuaManager) {
        let Some(setup) = self.new_game_setup.clone() else {
            return;
        };
        if self.lua_player_desc.is_none() {
            let sm = pz_lua_api::bind_registry::shared_script_manager();
            let chars = pz_lua_api::bind_characters::shared_characters();
            let made = chars.borrow_mut().create_survivor(&sm);
            match made {
                Ok(d) => self.lua_player_desc = Some(Rc::new(RefCell::new(d))),
                Err(e) => self.log_error(format!("SurvivorFactory.CreateSurvivor: {e:?}")),
            }
        }
        self.lua_pos = match setup.spawn {
            Some(p) => p,
            None => match self.map_spawn_point(lua) {
                Some(p) => p,
                None => {
                    self.log_error(
                        "no spawn point was chosen, don't know where to spawn the player",
                    );
                    self.lua_pos
                }
            },
        };
        // What the menu's `getWorld():setLuaPosX/Y/Z` would have set.
        pz_lua_api::bind_world::lua_world::set_lua_pos(self.lua_pos);
    }

    /// The single-map spawn region `SpawnRegionMgr` builds
    /// (`media/maps/<map>/spawnpoints.lua`'s `SpawnPoints()`), its
    /// `unemployed` list, and one point at `ZombRand(#spawn) + 1`.
    fn map_spawn_point(&mut self, lua: &mut LuaManager) -> Option<(i32, i32, i32)> {
        let map = pz_lua_api::bind_core::shared_core()
            .borrow()
            .game_map
            .clone();
        let map = map.split(';').next().unwrap_or("").to_string();
        let path = lua
            .fs
            .borrow()
            .resolve(&format!("media/maps/{map}/spawnpoints.lua"))?;
        let src = std::fs::read_to_string(&path).ok()?;
        let g = lua.thread.globals().clone();
        let f =
            pz_kahlua::stdlib::load_string(&src, Some(&path.to_string_lossy()), g.clone()).ok()?;
        lua.thread.pcall(&f, &[]).ok()?;
        let Value::Table(points) = lua
            .thread
            .pcall(&g.get_str("SpawnPoints"), &[])
            .ok()?
            .into_iter()
            .next()?
        else {
            return None;
        };
        let Value::Table(list) = points.get_str("unemployed") else {
            return None;
        };
        let n = list.len() as i32;
        if n == 0 {
            return None;
        }
        let Value::Table(p) = list.get_int(pz_core::random::next(n) as i64 + 1) else {
            return None;
        };
        let num = |k: &str| p.get_str(k).as_num();
        let z = num("posZ").unwrap_or(0.0) as i32;
        match (num("worldX"), num("worldY")) {
            // The old format, in 300x300 cells.
            (Some(wx), Some(wy)) => Some((
                (wx * 300.0 + num("posX")?) as i32,
                (wy * 300.0 + num("posY")?) as i32,
                z,
            )),
            _ => Some((num("posX")? as i32, num("posY")? as i32, z)),
        }
    }

    /// `SandboxOptions.load()`: `map_sand.bin` (then
    /// `handleOldZombiesFile1`, `applySettings`, `toLua`), or with no file
    /// `resetToDefault` and `updateFromLua`.
    fn load_sandbox(&mut self, lua: &mut LuaManager, sb: &Rc<RefCell<pz_sandbox::SandboxOptions>>) {
        let vars = sandbox_vars(lua);
        let gt = pz_lua_api::bind_climate::shared_game_time();
        let mut s = sb.borrow_mut();
        match std::fs::read(self.file_in_save("map_sand.bin")) {
            Ok(bytes) => match s.load_bin(&bytes) {
                Ok(preset) => {
                    self.preset = preset;
                    // TODO(port): `handleOldZombiesFile1`.
                    // `applySettings()`.
                    let mut g = gt.borrow_mut();
                    g.start_year = s.first_year() + opt_int(&s, "StartYear") - 1;
                    g.start_month = opt_int(&s, "StartMonth") - 1;
                    g.start_day = opt_int(&s, "StartDay") - 1;
                    g.minutes_per_day = s.day_length_minutes() as f32;
                    g.start_time_of_day = match opt_int(&s, "StartTime") {
                        1 => 7.0,
                        3 => 12.0,
                        4 => 14.0,
                        5 => 17.0,
                        6 => 21.0,
                        7 => 0.0,
                        8 => 2.0,
                        9 => 5.0,
                        _ => 9.0,
                    };
                    if let Some(v) = &vars {
                        s.to_lua(v);
                    }
                    return;
                }
                Err(e) => {
                    drop(s);
                    self.log_error(format!("SandboxOptions.load: {e}"));
                    s = sb.borrow_mut();
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                drop(s);
                self.log_error(format!("SandboxOptions.load: {e}"));
                s = sb.borrow_mut();
            }
        }
        s.reset_to_default();
        // `updateFromLua()`.
        let mut g = gt.borrow_mut();
        if pz_lua_api::bind_core::shared_core().borrow().game_mode == "LastStand" {
            g.multiplier_bias = 1.2;
        }
        if let Some(v) = &vars {
            s.update_from_lua(v);
        }
        g.multiplier_bias = match opt_int(&s, "Speed") {
            1 => 0.8,
            2 => 0.9,
            4 => 1.1,
            5 => 1.2,
            _ => 1.0,
        };
        // TODO(port): `VirtualZombieManager.maxRealZombies` from `Zombies`.
    }

    /// `ErosionGlobals.Boot(spriteManager)`: `new ErosionMain` (whose
    /// `start()` reads the config), then the season it set up goes to the
    /// climate with `OnInitSeasons` (`bind_climate::start_erosion_season`).
    fn boot_erosion(
        &mut self,
        lua: &mut LuaManager,
        sb: &Rc<RefCell<pz_sandbox::SandboxOptions>>,
        no_save: bool,
    ) {
        let tables = match pz_erosion::tables::ErosionTables::from_jar_path(self.jar()) {
            Ok(t) => t,
            Err(e) => return self.log_error(format!("ErosionGlobals.Boot: {e}")),
        };
        let (day, month, year) = {
            let gt = pz_lua_api::bind_climate::shared_game_time();
            let g = gt.borrow();
            (g.day, g.month, g.year)
        };
        let s = sb.borrow();
        let settings = pz_erosion::erosion_main::ErosionSettings {
            erosion_speed: s.erosion_speed(),
            erosion_days: opt_int(&s, "ErosionDays"),
            time_since_apo: s.time_since_apo(),
            temperature_modifier: s.temperature_modifier(),
            day_night_cycle: opt_int(&s, "DayNightCycle"),
            day,
            month,
            year,
            core_debug: pz_lua_api::bind_core::shared_core().borrow().debug,
            no_save,
            world_seed: self.world_gen_params.borrow().seed,
            save_config: Some(self.file_in_save("erosion.ini")),
            cache_config: Some(self.cache_dir.join("erosion.ini")),
            media_config: Some(self.game_dir.join("media/data/erosion.ini")),
            ..Default::default()
        };
        let temperature = s.temperature_modifier();
        drop(s);
        let Some(sprites) = self.sprites.clone() else {
            return;
        };
        let main = pz_erosion::erosion_main::ErosionMain::new(
            tables,
            settings,
            &mut sprites.borrow_mut(),
            None,
            None,
        );
        let c = &main.get_config().season;
        let cfg = pz_core::weather::season::SeasonConfig {
            lat: c.lat,
            temp_max: c.temp_max,
            temp_min: c.temp_min,
            temp_diff: c.temp_diff,
            season_lag: c.season_lag,
            noon: c.noon,
            seed_a: c.seed_a,
            seed_b: c.seed_b,
            seed_c: c.seed_c,
            rain: c.rain,
        };
        pz_lua_api::bind_climate::start_erosion_season(&mut lua.thread, &cfg, temperature);
        // `ErosionMain.instance`; `mainTimer` reads the `GameTime` date.
        // TODO(port): the erosion tick and the square / chunk load hooks.
        let main = Rc::new(RefCell::new(main));
        let hooks = pz_lua_api::bind_erosion::ErosionLuaHooks {
            game_date: Some(Box::new(|| {
                let gt = pz_lua_api::bind_climate::shared_game_time();
                let g = gt.borrow();
                (g.day, g.month, g.year)
            })),
            ..Default::default()
        };
        pz_lua_api::bind_erosion::set_instance(Some((main.clone(), Rc::new(hooks))));
        self.erosion = Some(main);
    }

    /// `metaGrid.load()` (`map_meta.bin`), `map_zone.bin`, the meta cells
    /// and animal zones, then `processZones()`.
    fn load_meta_grid_files(&mut self) {
        let IsoWorld {
            save_dir,
            meta_world,
            spawned_zones,
            uuids,
            errors,
            ..
        } = self;
        with_world(|w| {
            let Some(g) = w.meta_grid.as_mut() else {
                return;
            };
            if let Err(e) = pz_world::meta_save::load_map_meta_file(
                g,
                meta_world,
                save_dir,
                &mut DefaultMetaEnv,
            ) {
                errors.push(format!("IsoMetaGrid.load: {e}"));
            }
            if let Err(e) = g.load_zones_file(save_dir, spawned_zones, uuids) {
                errors.push(format!("IsoMetaGrid.loadZone: {e}"));
            }
            errors.extend(pz_lua_api::world_save::load_meta_grid_extras(g, save_dir));
            g.process_zones();
        });
    }

    /// `MapCollisionData.instance.init(metaGrid)` then
    /// `ZombiePopulationManager.instance.init(metaGrid)`.
    fn init_population(
        &mut self,
        sb: &Rc<RefCell<pz_sandbox::SandboxOptions>>,
        game_mode: &str,
        last_stand: bool,
        no_save: bool,
    ) {
        let s = sb.borrow();
        let mut gs = pz_world_popman::world::save_game_state(&self.save_dir);
        gs.game_mode = game_mode.to_string();
        gs.last_stand = last_stand;
        gs.no_save = no_save;
        gs.distribution = opt_int(&s, "Distribution");
        gs.zombies = opt_int(&s, "Zombies");
        gs.zombies_disabled = gs.zombies == 6;
        let config = s.zombie_config();
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u32);
        let native = pz_popman::native::Native::new(seed);
        let outfits = &self.outfit_names;
        let origins = pz_lua_api::bind_world::lua_world::take_spawn_origins();
        self.population = with_world(|w| {
            w.meta_grid.as_ref().map(|m| {
                pz_world_popman::WorldPopulation::init_with_spawn_origins(
                    native, m, &gs, &config, outfits, &origins,
                )
            })
        });
    }

    /// `new WorldGenChunk(seed)`, the biome map and the map's `WorldGen`
    /// zones (`IsoChunk.LoadBrandNew`'s world generation).
    /// `Basements.beforeLoadMetaGrid()` for a world without
    /// `map_basements.bin`: the definitions `basements.lua` and the
    /// `Basement` zones registered on `OnLoadMapZones`, their `.pzby`
    /// headers, the placements (seeded by the world-generation seed) and
    /// their buildings in the lot headers and the meta grid.
    fn before_load_meta_grid(&mut self) {
        // TODO(port): `loadSavefile()` / `writeSavefile()`
        // (`map_basements.bin`): a loaded world recomputes the placements.
        let mut b =
            pz_worldgen::basements::Basements::new(Rc::new(pz_fs::GameFs::new(&self.game_dir)));
        b.per_map = pz_lua_api::bind_basements::take_per_map();
        b.load_headers(&mut self.rand);
        let seed = self.world_gen_params.borrow().seed();
        let rand = &mut self.rand;
        with_world(|w| {
            if let Some(meta) = w.meta_grid.as_mut() {
                b.calculate_placements(meta, seed);
                b.meta_buildings = b.add_buildings_to_meta_grid(meta, rand);
            }
        });
        for e in std::mem::take(&mut b.errors) {
            self.log_error(format!("Basements: {e}"));
        }
        self.basements = Some(b);
    }

    fn load_world_gen(
        &mut self,
        lua: &LuaManager,
        sb: &Rc<RefCell<pz_sandbox::SandboxOptions>>,
        game_map: &str,
    ) -> Option<pz_worldgen::WorldGenHook> {
        let jar = match pz_classfile::Jar::open(self.jar()) {
            Ok(j) => j,
            Err(e) => {
                self.log_error(format!("world generation: {e}"));
                return None;
            }
        };
        // TODO(port): mod lot directories (the world generator reads the
        // installed game's files).
        let _ = lua;
        let fs = Rc::new(pz_fs::GameFs::new(&self.game_dir));
        let maps: Vec<&str> = game_map.split(';').collect();
        let Some(bounds) = with_world(|w| {
            w.meta_grid
                .as_ref()
                .map(|g| (g.min_x, g.min_y, g.max_x, g.max_y))
        }) else {
            return None;
        };
        let s = sb.borrow();
        let lookup = |k: &str| {
            s.option_by_name(k)
                .and_then(|o| o.config.get_value_as_string().parse::<f64>().ok())
        };
        let seed = self.world_gen_params.borrow().seed_string.clone();
        let r = pz_worldgen::load(&pz_worldgen::WorldGenSetup {
            fs,
            jar: &jar,
            seed: &seed,
            maps: &maps,
            cell_bounds: bounds,
            sandbox: &lookup,
        });
        match r {
            Ok(g) => Some(pz_worldgen::WorldGenHook::new(g)),
            Err(e) => {
                drop(s);
                self.log_error(format!("world generation: {e}"));
                None
            }
        }
    }

    /// `IsoCell.LoadPlayer(savedWorldVersion)` in a local game:
    /// `importPlayersFromVehiclesDB`, then `PlayerDB.loadLocalPlayer(1)`.
    /// That makes a `new IsoPlayer(cell)` the instance and player 0 when
    /// there is none, then `loadPlayer` decodes the row into it with
    /// `IsoPlayer.load(bytes, worldVersion)` over the save's item registry
    /// (inventory, worn and hand items, body damage, stats, XP, nutrition,
    /// ...); a dead player gets 0 overall body health and health. On
    /// success player 0's chunk map moves to the player's chunk. A missing
    /// row or a failed decode is logged and the load goes on (the original
    /// ignores `LoadPlayer`'s result), leaving the player as far as it got.
    fn load_player(&mut self, save_dir: &str) -> Result<(), String> {
        let db = self.player_db.as_mut().ok_or("PlayerDB")?;
        let data = match player_db::load_local_player_1(db, &mut self.vehicles, save_dir) {
            LocalPlayerLoad::Db(d) => d,
            // TODO(port): pre-database saves (`map_p.bin`).
            LocalPlayerLoad::MapPBin(p) => {
                return Err(format!("{p}: map_p.bin saves are not supported"))
            }
        };
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let chars = pz_lua_api::bind_characters::shared_characters();
        let chr = match pz_lua_api::bind_players::player_instance() {
            Some(p) => p,
            None => {
                let chr = Rc::new(RefCell::new(
                    self.new_iso_player(&sm, &mut chars.borrow_mut())?,
                ));
                pz_lua_api::bind_players::set_player_instance(Some(chr.clone()));
                pz_lua_api::bind_players::set_player(0, Some(chr.clone()));
                chr
            }
        };
        let Some(data) = data else {
            self.log_error("PlayerDB.loadLocalPlayer(1): no player");
            return Ok(());
        };
        let ctx = DefaultSimContext;
        let loaded = {
            let Some(tables) = self.save_tables.as_ref() else {
                return Err("SaveTables".into());
            };
            let items: &dyn pz_inventory::env::ItemEnv = match self.items.as_ref() {
                Some(i) => i,
                None => &AllItems,
            };
            let env = SaveEnv { items, tables };
            let mut on_fire = false;
            let mut r = pz_inventory::io::Reader::new(&data.bytes);
            let mut c = chr.borrow_mut();
            let res = c.load_player(
                &sm,
                &mut chars.borrow_mut(),
                &ctx,
                &env,
                &mut r,
                data.world_version,
                &mut |_| on_fire = true,
            );
            if res.is_ok() && data.is_dead {
                c.body_damage.overall_body_health = 0.0;
                c.set_health(&ctx, 0.0);
            }
            // `player.sqlId = sqlId` (kept by the world: `GameCharacter`
            // has no field for it).
            if res.is_ok() {
                self.player_sql_ids[0] = data.sql_id;
            }
            drop(c);
            if on_fire {
                // TODO(port): `SetOnFire()` (`pz_world::fire::character::set_on_fire`
                // needs the world's `IsoFireManager`).
                self.log_error("IsoPlayer.load: the player is on fire (SetOnFire is not ported)");
            }
            res.map(|()| r.pos)
        };
        match loaded {
            Ok(consumed) => {
                let c = chr.borrow();
                let inv = c.items.inventory_items().len();
                let worn = c.items.worn_items.as_ref().map_or(0, |w| w.size());
                println!(
                    "IsoPlayer.load: {consumed} of {} bytes (world version {}), {inv} inventory items, {worn} worn, hours survived {}, health {}",
                    data.bytes.len(),
                    data.world_version,
                    c.hours_survived,
                    c.body_damage.overall_body_health()
                );
            }
            Err(e) => {
                self.log_error(format!("PlayerDB.loadPlayer: IsoPlayer.load: {e:?}"));
                return Ok(());
            }
        }
        let (x, y) = {
            let c = chr.borrow();
            (c.vars.x, c.vars.y)
        };
        let (wx, wy) = player_db::local_player_chunk_map_pos(x, y, (0, 0));
        if let Some(s) = self.streamer.as_mut() {
            let m = s.chunk_map_mut(0);
            m.world_x = wx;
            m.world_y = wy;
        }
        Ok(())
    }

    /// `new IsoPlayer(cell)` as far as the port models it: a player
    /// character with a `new SurvivorDesc()` and `initWornItems("Human")`.
    /// TODO(port): the rest of the constructor (sprite parts, the state
    /// machine and action context, the speak colour's draws).
    fn new_iso_player(
        &mut self,
        sm: &pz_scripts::ScriptManager,
        chars: &mut pz_characters::Characters,
    ) -> Result<LuaGameCharacter, String> {
        let mut chr = LuaGameCharacter::new(sm, CharacterKind::Player);
        self.next_object_id += 1;
        chr.id = self.next_object_id;
        chr.human_visual = Some(pz_world::save::visual::HumanVisual::new(
            pz_characters::character::items::blood_part::MAX,
        ));
        let desc =
            LuaSurvivorDesc::new(chars, sm).map_err(|e| format!("new SurvivorDesc(): {e:?}"))?;
        chr.descriptor = Some(Rc::new(RefCell::new(desc)));
        let group = chars
            .body_locations
            .get_group("Human")
            .map_err(|e| format!("initWornItems: {e:?}"))?;
        chr.init_worn_items(sm, group);
        Ok(chr)
    }

    /// `IsoPlayer.players[0] = player`, `IsoPlayer.setInstance(player)`.
    fn place_player(&mut self, chr: LuaGameCharacterRef) {
        // Share the world's moving-object identity allocation with real
        // zombies; model/state lookups must never alias another player.
        if chr.borrow().id == 0 {
            self.next_object_id += 1;
            chr.borrow_mut().id = self.next_object_id;
        }
        pz_lua_api::bind_players::set_player(0, Some(chr.clone()));
        pz_lua_api::bind_players::set_player_instance(Some(chr));
        // TODO(port): `IsoCamera.setCameraCharacter(player)`.
    }

    /// The new-game branch of `IsoWorld.init`: the spawn square (a free
    /// tile of its room), `new IsoPlayer(cell, luaDesc, x, y, z)`, traits
    /// and granted recipes, the spawn building explored, then
    /// `OnNewGame(player, square)`.
    fn create_new_player(
        &mut self,
        lua: &mut LuaManager,
        sm: &pz_scripts::ScriptManager,
    ) -> Result<(), String> {
        let (xa, ya, za) = self.world_xa;
        let sq = with_world(|w| {
            w.cell
                .as_ref()
                .and_then(|c| c.get_grid_square(xa, ya, za))
                .map(|s| {
                    let s = s.borrow();
                    (s.x, s.y, s.z)
                })
        });
        // TODO(port): `sq.isFree(false) && sq.getRoom() != null` →
        // `sq.getRoom().getFreeTile()` (IsoRoom is not ported).
        let mut player = None;
        if let Some(desc) = self.lua_player_desc.clone() {
            let Some((x, y, z)) = sq else {
                return Err(format!(
                    "can't create player at x,y,z={xa},{ya},{za} because the square is null"
                ));
            };
            // TODO(port): `WorldSimulation.instance.create()`.
            // `new IsoPlayer(cell, desc, x, y, z)`: the simulation state, the
            // descriptor and its worn items on the "Human" group. TODO(port):
            // the rest of the IsoPlayer constructor (sprite, state machine,
            // inventory setup, chat element).
            let mut chr = LuaGameCharacter::new(sm, CharacterKind::Player);
            chr.human_visual = Some(pz_world::save::visual::HumanVisual::new(
                pz_characters::character::items::blood_part::MAX,
            ));
            let group = desc.borrow().worn_items.body_location_group().clone();
            chr.descriptor = Some(desc);
            chr.init_worn_items(sm, group);
            chr.vars.x = x as f32 + 0.5;
            chr.vars.y = y as f32 + 0.5;
            chr.vars.z = z as f32;
            chr.vars.dir = pz_world::square::IsoDirections::SE;
            // `player.sqlId = 1`.
            self.player_sql_ids[0] = 1;
            let chr = Rc::new(RefCell::new(chr));
            self.place_player(chr.clone());
            player = Some(chr);
        }
        // `IsoPlayer.getInstance().applyTraits(..)`: without a player (no
        // descriptor from the menu) the original throws here.
        let Some(p) = pz_lua_api::bind_players::player_instance() else {
            return Err("java.lang.NullPointerException: IsoPlayer.getInstance() is null (no player descriptor)".into());
        };
        {
            let chars = pz_lua_api::bind_characters::shared_characters();
            let r = p.borrow_mut().apply_traits(
                sm,
                &chars.borrow(),
                Some(&self.lua_traits),
                &DefaultSimContext,
            );
            if let Err(e) = r {
                self.log_error(format!("IsoPlayer.applyTraits: {e:?}"));
            }
            // TODO(port): the profession's and traits' granted recipes.
        }
        // TODO(port): the spawn room explored (`setExplored`,
        // `setAllExplored`, `setBasementAllExplored`) and
        // `ZombiePopulationManager.playerSpawnedAt`; `initPoisonousBerry` /
        // `initPoisonousMushroom`.
        let pv = player.as_ref().map(character_value).unwrap_or_default();
        let sv = sq
            .map(|(x, y, z)| pz_lua_api::bind_world::square_value(x, y, z))
            .unwrap_or_default();
        println!("triggerEvent OnNewGame");
        lua.trigger_event("OnNewGame", &[pv, sv]);
        Ok(())
    }
}

#[cfg(test)]
mod world_gen_params_tests {
    use super::*;

    #[test]
    fn worlds_reuse_menu_parameters_and_load_saved_seed_and_bounds_in_place() {
        let shared = pz_lua_api::bind_world_gen_params::shared_world_gen_params();
        struct Restore(
            pz_lua_api::bind_world_gen_params::WorldGenParamsRef,
            pz_worldgen::WorldGenParams,
        );
        impl Drop for Restore {
            fn drop(&mut self) {
                *self.0.borrow_mut() = self.1.clone();
            }
        }
        let _restore = Restore(shared.clone(), shared.borrow().clone());
        *shared.borrow_mut() = pz_worldgen::WorldGenParams::new("from menu");
        shared.borrow_mut().min_x_cell = -21;
        shared.borrow_mut().max_y_cell = 33;
        let temp = std::env::temp_dir().join(format!(
            "pz-worldgen-owner-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir(&temp).unwrap();
        let mut world = IsoWorld::new(&temp, &temp, temp.clone());
        assert!(Rc::ptr_eq(&world.world_gen_params, &shared));
        world.load_world_gen_params();
        assert_eq!(shared.borrow().seed_string, "from menu");
        assert_eq!(shared.borrow().min_x_cell, -21);
        world.new_game_setup = Some(NewGameSetup {
            spawn: None,
            seed: "direct entry".into(),
        });
        world.load_world_gen_params();
        assert_eq!(shared.borrow().seed_string, "direct entry");
        assert_eq!(
            shared.borrow().seed,
            pz_worldgen::java::string_hash("direct entry")
        );
        assert_eq!(
            shared.borrow().min_x_cell,
            -21,
            "direct entry discarded menu bounds"
        );
        let mut saved = pz_worldgen::WorldGenParams::new("loaded world");
        saved.min_x_cell = -12;
        saved.max_y_cell = 24;
        saved.save_file(&temp, false).unwrap();
        world.load_world_gen_params();
        assert!(Rc::ptr_eq(&world.world_gen_params, &shared));
        assert_eq!(
            *shared.borrow(),
            saved,
            "loaded seed/bounds did not reach the menu owner"
        );
        drop(world);
        let next = IsoWorld::new(&temp, &temp, temp.clone());
        assert!(Rc::ptr_eq(&next.world_gen_params, &shared));
        assert_eq!(
            *next.world_gen_params.borrow(),
            saved,
            "new world reset the static parameters"
        );
        std::fs::remove_dir_all(temp).unwrap();
    }
}

#[cfg(test)]
mod stream_retirement_tests;
