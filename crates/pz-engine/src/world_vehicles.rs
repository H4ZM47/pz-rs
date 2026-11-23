//! Bullet physics (`pz-bullet`), runtime vehicles (`pz-vehicles`), and
//! vehicle rendering/lighting (`pz-vehicle-render`) integration for `pz-engine`.
//!
//! Owns the world's single [`PzBullet`] physics instance (installed into
//! [`pz_vehicles::physics::Bullet`]), wires [`VehicleWorld`], [`VehicleEvents`],
//! [`ItemFactory`], `VehiclesDB2` (`VehicleIds`, [`RuntimeVehicleSaver`],
//! [`VirtualVehicleMeta`], [`VirtualVehicleManager`]), chunk vehicle spawning
//! (`IsoChunk.AddVehicles` via [`ChunkSpawner`]) and loaded-vehicle adoption,
//! chunk static physics (`ChunkPhysics`), pathfinding integration
//! ([`PathfindVehicles`]), per-vehicle [`LightbarWorldLights`] and headlight
//! torches ([`cell_torches`]), and per-vehicle [`VehicleModelSet`] rendering
//! into [`FrameObjects::moving`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use pz_bullet::PzBullet;
use pz_fs::GameFs;
use pz_inventory::template::ItemTemplate;
use pz_inventory::InventoryItem;
use pz_iso_render::models::{interpolate_light, ModelCamera};
use pz_iso_render::objects::FrameObjects;
use pz_kahlua::Value;
use pz_lua_api::bind_world::with_world;
use pz_lua_api::LuaManager;
use pz_model::assets::MediaIndex;
use pz_render::{Texture, TextureManager};
use pz_scripts::objects::items::Item as ScriptItem;
use pz_scripts::objects::vehicles::VehicleScript;
use pz_scripts::script_type::ScriptType;
use pz_scripts::ScriptManager;
use pz_sound::emitter::ParentObject;
use pz_vehicle_render::lights::{cell_torches as render_cell_torches, LightbarWorldLights};
use pz_vehicle_render::models::{MediaVehicleModels, VehicleModelSet};
use pz_vehicle_render::render::{render_vehicle_object, SquareLight, VehicleFrame};
use pz_vehicles::cell_link;
use pz_vehicles::chunk_physics::ChunkPhysics;
use pz_vehicles::env::{
    self, GameTimeState, ItemFactory, LightbarState, SandboxVehicle, SquareInfo, VehicleEvents,
    VehicleWorld,
};
use pz_vehicles::physics::{Bullet, NoPhysics};
use pz_vehicles::save::from_save;
use pz_vehicles::spawn::{CellSpawnWorld, ChunkSpawner, SpawnOptions};
use pz_vehicles::vehicles_db::RuntimeVehicleSaver;
use pz_vehicles::virtual_vehicle::{self, VirtualVehicleMeta};
use pz_vehicles::{manager, vehicle_type, world_sim, BaseVehicle, PartRef, VehicleRef};
use pz_world::save::vehicles_db::{self, VehicleIds};
use pz_world::save::SaveCtx;
use pz_world::streamer::{ChunkSource, ChunkStreamer};
use pz_world::IsoSpriteManager;
use pz_world_render::{WorldLighting, WorldView};

use crate::iso_world::{opt_bool, opt_f64, opt_int, IsoWorld};
use crate::sim::FrameSim;
use crate::world_render::WorldRender;

pz_vehicles::pathfind_adapter!(pz_pathfind);

/// Default vehicle shadow texture path (`BaseVehicle.getShadowTexture`).
const DEFAULT_SHADOW_TEXTURE: &str = "media/vehicleShadow.png";

/// Shared mutable state accessed by [`EngineVehicleEvents`] and drained on the
/// main loop outside vehicle borrows.
struct VehicleEventsState {
    pending_db_removes: Vec<i32>,
    lightbars: HashMap<usize, (Weak<BaseVehicle>, LightbarWorldLights)>,
    render: Rc<RefCell<Option<WorldRender>>>,
}

impl VehicleEventsState {
    fn new(render: Rc<RefCell<Option<WorldRender>>>) -> Self {
        Self {
            pending_db_removes: Vec::new(),
            lightbars: HashMap::new(),
            render,
        }
    }

    fn clear_all_lights(&mut self) {
        if let Ok(mut r) = self.render.try_borrow_mut() {
            if let Some(wr) = r.as_mut() {
                for (_, (_, mut lb)) in self.lightbars.drain() {
                    lb.remove(&mut wr.lighting);
                }
                return;
            }
        }
        self.lightbars.clear();
    }

    fn prune_stale_lights(&mut self, live: &HashSet<usize>) {
        let mut stale = Vec::new();
        for (&k, (weak, _)) in &self.lightbars {
            if !live.contains(&k) || weak.strong_count() == 0 {
                stale.push(k);
            }
        }
        if stale.is_empty() {
            return;
        }
        if let Ok(mut r) = self.render.try_borrow_mut() {
            if let Some(wr) = r.as_mut() {
                for k in stale {
                    if let Some((_, mut lb)) = self.lightbars.remove(&k) {
                        lb.remove(&mut wr.lighting);
                    }
                }
                return;
            }
        }
        for k in stale {
            self.lightbars.remove(&k);
        }
    }
}

/// [`VehicleWorld`] over the engine's live `IsoCell`, `IsoMetaGrid`,
/// `IsoSpriteManager` and [`ChunkPhysics`].
struct EngineVehicleWorld {
    sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
    chunk_physics: Rc<RefCell<ChunkPhysics>>,
    chunk_grid_width: i32,
}

impl VehicleWorld for EngineVehicleWorld {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
        let sprites = self.sprites.as_ref();
        with_world(|w| {
            let cell = w.cell.as_ref()?;
            let sq_ref = cell.get_grid_square(x, y, z)?;
            let sq = sq_ref.borrow();
            let floor_sprite = sprites.and_then(|sp| {
                let sp = sp.try_borrow().ok()?;
                sq.objects.iter().find_map(|o| {
                    let o = o.borrow();
                    let s = o.sprite.try_read(&sp).ok()??;
                    if s.properties.has(sp.tables.flag.solidfloor) {
                        s.name.clone()
                    } else {
                        None
                    }
                })
            });
            let has_floor = floor_sprite.is_some() || !sq.objects.is_empty();
            let meta = w.meta_grid.as_ref();
            let room_def = meta.and_then(|m| sq.room_def(m).map(|r| r.name.clone()));
            let in_room = sq.room_id != -1;
            let zone_type = meta.and_then(|m| {
                m.get_zones_at(x, y, z)
                    .first()
                    .map(|&zid| m.zone(zid).zone_type.clone())
            });
            let chunk_has_refs = cell
                .chunks
                .contains_key(&(x.div_euclid(8), y.div_euclid(8)));
            Some(SquareInfo {
                has_floor,
                floor_sprite,
                room_def,
                in_room,
                zone_type,
                chunk_has_refs,
            })
        })
    }

    fn is_valid_chunk(&self, wx: i32, wy: i32) -> bool {
        with_world(|w| {
            w.meta_grid
                .as_ref()
                .is_some_and(|m| m.is_valid_chunk(wx, wy))
        })
    }

    fn chunk_loaded(&self, wx: i32, wy: i32) -> bool {
        with_world(|w| {
            w.cell
                .as_ref()
                .is_some_and(|c| c.chunks.contains_key(&(wx, wy)))
        })
    }

    fn chunk_grid_width(&self) -> i32 {
        self.chunk_grid_width
    }

    fn vehicle_zone_name(&self, x: i32, y: i32, z: i32) -> Option<String> {
        with_world(|w| {
            w.meta_grid
                .as_ref()?
                .vehicle_zone_at(x, y, z)
                .map(|z| z.name.clone())
        })
    }

    fn is_raining(&self) -> bool {
        let gt = pz_lua_api::bind_climate::shared_game_time();
        let r = gt.try_borrow().is_ok_and(|g| g.climate.is_raining());
        r
    }

    fn chunk_vehicles(&self, wx: i32, wy: i32) -> Option<Vec<VehicleRef>> {
        with_world(|w| cell_link::chunk_vehicles(w.cell.as_ref()?, wx, wy))
    }

    fn update_physics_for_level_if_needed(
        &self,
        wx: i32,
        wy: i32,
        level: i32,
        to_bullet: &mut Vec<Vec<u8>>,
    ) -> bool {
        let Some(sprites) = self.sprites.as_ref() else {
            return false;
        };
        let Ok(sp) = sprites.try_borrow() else {
            return false;
        };
        let Ok(cp) = self.chunk_physics.try_borrow() else {
            return false;
        };
        with_world(|w| {
            let valid = w
                .meta_grid
                .as_ref()
                .is_some_and(|m| m.is_valid_chunk(wx, wy));
            let Some(cell) = w.cell.as_mut() else {
                return false;
            };
            cp.update_if_needed(cell, valid, &sp, wx, wy, level, to_bullet)
        })
    }

    fn add_vehicle_to_chunk(&self, v: &VehicleRef, chunk: (i32, i32)) {
        with_world(|w| {
            if let Some(cell) = w.cell.as_mut() {
                cell_link::add_to_chunk(cell, v, chunk);
            }
        });
    }

    fn vehicle_chunk_changed(&self, v: &VehicleRef, from: (i32, i32), to: (i32, i32)) {
        with_world(|w| {
            if let Some(cell) = w.cell.as_mut() {
                cell_link::update_chunk(cell, v, from, to);
            }
        });
    }

    fn remove_vehicle_from_chunk(&self, v: &VehicleRef, chunk: (i32, i32)) {
        with_world(|w| {
            if let Some(cell) = w.cell.as_mut() {
                cell_link::remove_from_chunk(cell, v, chunk);
            }
        });
    }
}

/// [`VehicleEvents`] over the engine's lighting, sound and vehicle DB.
struct EngineVehicleEvents {
    state: Rc<RefCell<VehicleEventsState>>,
}

impl VehicleEvents for EngineVehicleEvents {
    fn vehicle_removed_from_database(&self, vehicle: &BaseVehicle) {
        let sql_id = vehicle.st().sql_id;
        if sql_id > 0 {
            self.state.borrow_mut().pending_db_removes.push(sql_id);
        }
    }

    fn lightbar_world_lights(&self, vehicle: &BaseVehicle, state: LightbarState) {
        let key = vehicle as *const BaseVehicle as usize;
        let Ok(mut st) = self.state.try_borrow_mut() else {
            return;
        };
        let render = st.render.clone();
        let Ok(mut r) = render.try_borrow_mut() else {
            return;
        };
        let Some(wr) = r.as_mut() else {
            return;
        };
        let entry = st.lightbars.entry(key).or_insert_with(|| {
            let weak = manager::cell_vehicles()
                .into_iter()
                .find(|v| std::ptr::eq(v.as_ref(), vehicle))
                .map(|v| Rc::downgrade(&v))
                .unwrap_or_default();
            (weak, LightbarWorldLights::new(vehicle))
        });
        entry.1.apply(vehicle, state, &mut wr.lighting);
    }

    fn play_sound(&self, name: &str) -> i64 {
        let sound = pz_lua_api::bind_audio::sound_system();
        let Ok(mut s) = sound.try_borrow_mut() else {
            return 0;
        };
        s.with_world_emitters(|pool, ctx| {
            let e = pool.get_free();
            let h = e.borrow_mut().play_sound_impl(name, ParentObject::None, ctx);
            h
        })
    }
}

/// [`ItemFactory`] creating script items for vehicle part creation and keys.
struct EngineVehicleItems {
    sm: Rc<ScriptManager>,
}

impl ItemFactory for EngineVehicleItems {
    fn create_item(&self, full_type: &str) -> Option<InventoryItem> {
        let script = self.sm.find_item(full_type, true)?.downcast::<ScriptItem>()?;
        let template = ItemTemplate::from_script_ref(&self.sm, &script)?;
        let mut item = InventoryItem::create(template, 0);
        item.id = pz_vehicles::rand::next(i32::MAX);
        Some(item)
    }
}

/// Per-vehicle model entry cached in [`VehicleRenderStore`].
struct VehicleModelEntry {
    weak: Weak<BaseVehicle>,
    set: VehicleModelSet,
    model_count: usize,
}

/// Owns loaded vehicle model sets and the media index used to load meshes and
/// textures on demand.
struct VehicleRenderStore {
    game_dir: PathBuf,
    media: Option<MediaIndex>,
    media_scanned: bool,
    default_shadow: Option<Texture>,
    shadow_loaded: bool,
    models: HashMap<usize, VehicleModelEntry>,
    last_blood_decals: Option<bool>,
}

impl VehicleRenderStore {
    fn new(game_dir: PathBuf) -> Self {
        Self {
            game_dir,
            media: None,
            media_scanned: false,
            default_shadow: None,
            shadow_loaded: false,
            models: HashMap::new(),
            last_blood_decals: None,
        }
    }

    fn ensure_media(&mut self) -> Option<&MediaIndex> {
        if !self.media_scanned {
            self.media_scanned = true;
            self.media = MediaIndex::scan(&self.game_dir).ok();
        }
        self.media.as_ref()
    }

    fn update_animations(&mut self, unmodded_multiplier: f32) {
        self.models.retain(|_, e| e.weak.strong_count() > 0);
        for entry in self.models.values_mut() {
            entry.set.update_animations(unmodded_multiplier);
        }
    }

    fn prune_stale(&mut self, live: &HashSet<usize>) {
        self.models
            .retain(|k, e| live.contains(k) && e.weak.strong_count() > 0);
    }

    fn clear(&mut self) {
        self.models.clear();
        self.default_shadow = None;
        self.shadow_loaded = false;
        self.last_blood_decals = None;
    }
}

/// Active vehicle headlights for the lighting pass (`LightingJNI.checkLights`).
pub fn cell_torches() -> Vec<pz_lighting::jni::TorchInfo> {
    let vehicles = manager::cell_vehicles();
    render_cell_torches(vehicles.iter().map(|v| v.as_ref()))
}

/// Captures `SandboxVehicle` and `SpawnOptions` from `SandboxOptions`.
fn read_sandbox_options(
    sb: &pz_sandbox::SandboxOptions,
) -> (SandboxVehicle, SpawnOptions) {
    let vehicle = SandboxVehicle {
        car_general_condition: opt_int(sb, "CarGeneralCondition").max(1),
        recently_survivor_vehicles: opt_int(sb, "RecentlySurvivorVehicles").max(1),
        vehicle_easy_use: opt_bool(sb, "VehicleEasyUse"),
        zombie_attraction_multiplier: {
            let v = opt_f64(sb, "ZombieAttractionMultiplier");
            if v > 0.0 { v } else { 1.0 }
        },
        car_damage_on_impact: opt_int(sb, "CarDamageOnImpact").max(1),
        player_damage_from_crash: opt_bool(sb, "PlayerDamageFromCrash"),
        key_loot_new: opt_f64(sb, "KeyLootNew"),
        siren_effects_zombies: opt_bool(sb, "SirenEffectsZombies"),
        siren_shutoff_hours: opt_f64(sb, "SirenShutoffHours"),
        injury_severity: opt_int(sb, "InjurySeverity").max(1),
        bone_fracture: opt_bool(sb, "BoneFracture"),
    };
    let spawn = SpawnOptions {
        car_spawn_rate: opt_int(sb, "CarSpawnRate").max(1),
        car_alarm: opt_int(sb, "CarAlarm").max(1),
        traffic_jam: opt_bool(sb, "TrafficJam"),
        enable_vehicles: opt_bool(sb, "EnableVehicles"),
        vehicles_everywhere: false,
    };
    (vehicle, spawn)
}

/// Uploads every loaded `VehicleScript` into Bullet (`VehicleScript.toBullet` ->
/// `Bullet.defineVehicleScript`).
pub fn define_all_vehicle_scripts(sm: &ScriptManager) -> usize {
    let mut defined = 0usize;
    for script_ref in sm.collection(ScriptType::Vehicle).get_all_scripts() {
        let Some(vs_rc) = script_ref.downcast::<VehicleScript>() else {
            continue;
        };
        let mut vs = vs_rc.borrow_mut();
        if vs.bullet_params().is_none() {
            let _ = vs.to_bullet();
        }
        let full_name = vs.full_name();
        if let Some(params) = vs.bullet_params() {
            let _ = Bullet::call(|b| b.define_vehicle_script(&full_name, params));
            defined += 1;
        }
    }
    defined
}

/// Engine-owned Bullet physics and vehicle runtime attached to [`IsoWorld`].
pub struct WorldVehicles {
    pub ids: Rc<RefCell<VehicleIds>>,
    chunk_physics: Rc<RefCell<ChunkPhysics>>,
    events_state: Rc<RefCell<VehicleEventsState>>,
    render_store: RefCell<VehicleRenderStore>,
    brand_new_chunks: HashSet<(i32, i32)>,
    pending_chunks: Vec<(i32, i32, bool)>,
    spawn_options: SpawnOptions,
    sandbox_cfg: SandboxVehicle,
    chunk_grid_width: i32,
}

impl WorldVehicles {
    /// Initializes Bullet (`PzBullet`), uploads `VehicleScript` definitions,
    /// loads `VehicleZoneDistribution`, initializes `VehicleIds` from
    /// `vehicles.db`, installs `VehicleWorld`/`VehicleEvents`/`ItemFactory`,
    /// and creates the Bullet world (`WorldSimulation.create()`).
    #[allow(clippy::too_many_arguments)]
    pub fn init(
        game_dir: &Path,
        lua: &LuaManager,
        sb: &Rc<RefCell<pz_sandbox::SandboxOptions>>,
        vehicles_conn: Option<&mut pz_sqlite::Connection>,
        sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
        render: Rc<RefCell<Option<WorldRender>>>,
        meta_bounds: (i32, i32, i32, i32),
        chunk_map: (i32, i32, i32),
    ) -> Self {
        let (sandbox_cfg, spawn_options) = read_sandbox_options(&sb.borrow());
        let ids = match vehicles_conn {
            Some(conn) => VehicleIds::init(conn).unwrap_or_default(),
            None => VehicleIds::default(),
        };
        let ids = Rc::new(RefCell::new(ids));

        // Reset any leftover thread-local state from a previous session.
        world_sim::reset();
        manager::reset();
        virtual_vehicle::with_manager(|m| m.reset());
        spawn_reset();

        // Install live Bullet physics backend.
        let _ = Bullet::install(Box::new(PzBullet::new()));

        let sm = pz_lua_api::bind_registry::shared_script_manager();
        pz_vehicles::vehicle::set_script_manager(sm.clone());
        let _ = define_all_vehicle_scripts(&sm);

        // Read Lua's `VehicleZoneDistribution` table into `VehicleType`.
        vehicle_type::reset();
        if let Value::Table(dist) = lua
            .thread
            .globals()
            .get_own(&Value::str("VehicleZoneDistribution"))
        {
            vehicle_type::init(&dist);
        }

        let chunk_physics = Rc::new(RefCell::new(ChunkPhysics::default()));
        let events_state = Rc::new(RefCell::new(VehicleEventsState::new(render)));

        env::set_sandbox(sandbox_cfg.clone());
        env::set_vehicle_ids(Some(ids.clone()));
        env::set_world(Rc::new(EngineVehicleWorld {
            sprites,
            chunk_physics: chunk_physics.clone(),
            chunk_grid_width: chunk_map.2,
        }));
        env::set_events(Rc::new(EngineVehicleEvents {
            state: events_state.clone(),
        }));
        env::set_items(Rc::new(EngineVehicleItems { sm }));

        let (min_x, min_y, max_x, max_y) = meta_bounds;
        world_sim::set_world_bounds(min_x, min_y, max_x, max_y, chunk_map);
        world_sim::create();

        Self {
            ids,
            chunk_physics,
            events_state,
            render_store: RefCell::new(VehicleRenderStore::new(game_dir.to_path_buf())),
            brand_new_chunks: HashSet::new(),
            pending_chunks: Vec::new(),
            spawn_options,
            sandbox_cfg,
            chunk_grid_width: chunk_map.2,
        }
    }

    /// `VehiclesDB2.loadVehiclesInMeta()` (`IngameState.enter`, single player).
    pub fn load_vehicles_in_meta(
        &mut self,
        lua: &mut LuaManager,
        conn: Option<&mut pz_sqlite::Connection>,
        ctx: &SaveCtx<'_>,
    ) {
        let Some(conn) = conn else { return };
        let res = vehicles_db::load_vehicles_in_meta(Some(conn), ctx);
        if res.loaded.is_empty() {
            return;
        }
        pz_vehicles::lua::with_thread(&mut lua.thread, || {
            let _ = virtual_vehicle::add_loaded_vehicles_to_meta(&res.loaded, |lv| {
                Some(from_save(&lv.vehicle))
            });
        });
    }

    /// Records whether a served chunk came from `LoadBrandNew` so `HandedOff`
    /// knows whether to run `IsoChunk.AddVehicles` or adopt loaded rows.
    pub fn note_chunk_served(&mut self, wx: i32, wy: i32, source: &ChunkSource) {
        match source {
            ChunkSource::BrandNew | ChunkSource::BrandNewAfterError(_) => {
                self.brand_new_chunks.insert((wx, wy));
            }
            ChunkSource::Disk => {
                self.brand_new_chunks.remove(&(wx, wy));
            }
        }
    }

    /// Queues a newly handed-off chunk for Bullet level bounds, static chunk
    /// physics, and either `AddVehicles` (brand-new) or `adopt_loaded` (saved).
    pub fn note_chunk_handed_off(&mut self, wx: i32, wy: i32) {
        let brand_new = self.brand_new_chunks.remove(&(wx, wy));
        // Mark the chunk's levels dirty for Bullet immediately and upload its
        // min/max level bounds.
        let level_bounds = with_world(|w| {
            let cell = w.cell.as_mut()?;
            let chunk = cell.chunks.get_mut(&(wx, wy))?;
            let (min_l, max_l) = (chunk.min_level, chunk.max_level);
            for z in min_l..=max_l {
                chunk.check_physics_later(z);
            }
            Some((min_l, max_l))
        });
        if let Some((min_l, max_l)) = level_bounds {
            Bullet::call(|b| b.set_chunk_min_max_level(wx, wy, min_l, max_l));
        }
        self.pending_chunks.push((wx, wy, brand_new));
    }

    /// Marks specific chunk levels dirty for `Bullet.updatePhysicsForLevelIfNeeded`
    /// after square mutations.
    pub fn mark_physics_dirty(&self, levels: impl IntoIterator<Item = (i32, i32, i32)>) {
        with_world(|w| {
            if let Some(cell) = w.cell.as_mut() {
                ChunkPhysics::mark_physics_dirty(cell, levels);
            }
        });
    }

    /// Processes all chunks handed off since the last tick: spawns vehicles on
    /// brand-new chunks (`IsoChunk.AddVehicles`) or adopts `LoadedVehicle`s
    /// from `vehicles.db`, then adds each to the world/cell manager.
    pub fn process_pending_chunks(
        &mut self,
        lua: &mut LuaManager,
        sprites: Option<&Rc<RefCell<IsoSpriteManager>>>,
    ) {
        if self.pending_chunks.is_empty() {
            return;
        }
        let chunks = std::mem::take(&mut self.pending_chunks);
        let opts = self.spawn_options.clone();
        pz_vehicles::lua::with_thread(&mut lua.thread, || {
            for (wx, wy, brand_new) in chunks {
                let to_add: Vec<VehicleRef> = if brand_new {
                    let Some(sp_rc) = sprites else { continue };
                    let Ok(sp) = sp_rc.try_borrow() else { continue };
                    let spawned = with_world(|w| {
                        let (Some(cell), Some(meta)) = (w.cell.as_ref(), w.meta_grid.as_ref())
                        else {
                            return Vec::new();
                        };
                        let sw = CellSpawnWorld {
                            cell,
                            sprites: &sp,
                            meta,
                        };
                        let mut spawner = ChunkSpawner::new(wx, wy, &sw, opts.clone());
                        spawner.add_vehicles();
                        spawner.vehicles
                    });
                    if !spawned.is_empty() {
                        with_world(|w| {
                            if let Some(cell) = w.cell.as_mut() {
                                for v in &spawned {
                                    cell_link::add_to_chunk(cell, v, (wx, wy));
                                }
                            }
                        });
                    }
                    spawned
                } else {
                    with_world(|w| {
                        let Some(cell) = w.cell.as_mut() else {
                            return Vec::new();
                        };
                        cell_link::adopt_loaded(cell, wx, wy, |lv| Some(from_save(&lv.vehicle)))
                    })
                };
                for v in to_add {
                    manager::add_to_world(&v);
                }
            }
        });
    }

    /// Syncs `GameTime`, steps `WorldSimulation` + all live cell vehicles,
    /// advances part animations, updates `VirtualVehicleManager`, and applies
    /// queued DB updates/removals.
    #[allow(clippy::too_many_arguments)]
    pub fn update_frame(
        &mut self,
        lua: &mut LuaManager,
        sim: &FrameSim,
        sprites: Option<&Rc<RefCell<IsoSpriteManager>>>,
        streamer: Option<&mut ChunkStreamer>,
        vehicles_conn: Option<&mut pz_sqlite::Connection>,
        save_ctx: Option<&SaveCtx<'_>>,
    ) {
        self.process_pending_chunks(lua, sprites);

        let (gt_state, unmodded_mult) = {
            let gt = sim.game_time.borrow();
            let ctx = pz_lua_api::bind_climate::LuaClimateContext::new();
            let unmodded = gt.get_unmodded_multiplier(&ctx);
            let time_delta = gt.get_time_delta(&ctx);
            let now_ms = pz_lua_api::player_db::current_time_millis();
            (
                GameTimeState {
                    multiplier: gt.get_multiplier(&ctx),
                    time_delta,
                    thirty_fps_multiplier: gt.get_thirty_fps_multiplier(&ctx),
                    inv_multiplier: gt.get_inv_multiplier(&ctx),
                    unmodded_multiplier: unmodded,
                    physics_seconds_since_last_update: time_delta,
                    world_age_hours: gt.world_age_hours(),
                    delta_minutes_per_day: gt.get_delta_minutes_per_day(),
                    server_time_mills: now_ms,
                },
                unmodded,
            )
        };
        env::set_now_ms(gt_state.server_time_mills);
        env::set_time(gt_state);
        env::set_sandbox(self.sandbox_cfg.clone());
        if let Ok(gt) = sim.game_time.try_borrow() {
            world_sim::set_air_temperature(gt.climate.get_temperature());
        }

        pz_vehicles::lua::with_thread(&mut lua.thread, || {
            manager::process_add_vehicles();
            manager::update_frame();
        });

        self.render_store
            .borrow_mut()
            .update_animations(unmodded_mult);

        if let Some(ctx) = save_ctx {
            let queued = {
                let mut ids = self.ids.borrow_mut();
                virtual_vehicle::with_manager(|m| {
                    m.update(Some(&mut ids), ctx);
                    m.take_queued()
                })
            };
            if let Some(streamer) = streamer {
                let rows = queued.into_iter().filter_map(Result::ok);
                streamer.queue_vehicle_updates(rows);
            }
        }

        let removes = std::mem::take(&mut self.events_state.borrow_mut().pending_db_removes);
        if let Some(conn) = vehicles_conn {
            for sql_id in removes {
                vehicles_db::remove_vehicle(conn, sql_id);
            }
        }
    }

    /// `VirtualVehicleManager.save()` during quit / hot save: queues every
    /// virtual vehicle's current row onto the chunk streamer.
    pub fn save_virtual_vehicles(
        &mut self,
        streamer: Option<&mut ChunkStreamer>,
        ctx: &SaveCtx<'_>,
    ) {
        let queued = {
            let mut ids = self.ids.borrow_mut();
            virtual_vehicle::with_manager(|m| {
                m.save(Some(&mut ids), ctx);
                m.take_queued()
            })
        };
        if let Some(streamer) = streamer {
            let rows = queued.into_iter().filter_map(Result::ok);
            streamer.queue_vehicle_updates(rows);
        }
    }

    /// Cleans up render models and lightbar lampposts for vehicles that left
    /// the cell when a chunk unloaded.
    pub fn chunk_removed(&mut self, wx: i32, wy: i32) {
        self.brand_new_chunks.remove(&(wx, wy));
        self.pending_chunks.retain(|&(x, y, _)| (x, y) != (wx, wy));
        let live: HashSet<usize> = manager::cell_vehicles()
            .iter()
            .map(|v| Rc::as_ptr(v) as usize)
            .collect();
        self.events_state.borrow_mut().prune_stale_lights(&live);
        self.render_store.borrow_mut().prune_stale(&live);
    }

    /// Builds [`FrameObjects::moving`] render entries for every vehicle in
    /// `IsoCell.getVehicles()`, updating transforms, lights and damage
    /// overlays.
    #[allow(clippy::too_many_arguments)]
    pub fn render_vehicles(
        &self,
        view: &WorldView<'_>,
        player: usize,
        default_ambient: f32,
        camera: &ModelCamera,
        lighting: &WorldLighting,
        fs: &GameFs,
        tm: &mut TextureManager,
        objects: &mut FrameObjects,
    ) {
        let vehicles = manager::cell_vehicles();
        if vehicles.is_empty() {
            return;
        }
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let blood_decals = pz_lua_api::bind_core::shared_core()
            .try_borrow()
            .map_or(true, |c| c.get_option_blood_decals() != 0);

        let mut store = self.render_store.borrow_mut();
        let blood_changed = store.last_blood_decals != Some(blood_decals);
        store.last_blood_decals = Some(blood_decals);

        if !store.shadow_loaded {
            store.default_shadow = tm.shared_texture(fs, DEFAULT_SHADOW_TEXTURE);
            store.shadow_loaded = true;
        }
        let default_shadow = store.default_shadow.clone();
        let Some(media) = store.ensure_media().cloned() else {
            return;
        };

        let tm_cell = RefCell::new(tm);
        let load_tex = |path: &str, _flags: i32| -> Option<Texture> {
            tm_cell.borrow_mut().shared_texture(fs, path)
        };
        // Load with `reflections: false` (`_noreflect`) until SkyBox textures are wired.
        let model_source = MediaVehicleModels::new(&media, sm.clone(), false, &load_tex);
        let item_part_model = |_part: &PartRef, _model_id: &str| -> Option<String> { None };

        for v in &vehicles {
            let key = Rc::as_ptr(v) as usize;
            let model_count = v.st().models.len();
            let entry = store.models.entry(key).or_insert_with(|| {
                let mut load_tex_mut = |path: &str, _flags: i32| -> Option<Texture> {
                    tm_cell.borrow_mut().shared_texture(fs, path)
                };
                VehicleModelEntry {
                    weak: Rc::downgrade(v),
                    set: VehicleModelSet::create(
                        v,
                        &model_source,
                        &mut load_tex_mut,
                        &item_part_model,
                    ),
                    model_count,
                }
            });
            if entry.model_count != model_count {
                entry.set.sync(v, &model_source, &item_part_model);
                entry.model_count = model_count;
            }
            entry.set.update_transform(v);
            entry.set.instance.update_lights(v);
            let needs_damage = blood_changed || v.st().do_damage_overlay;
            if needs_damage {
                v.st_mut().do_damage_overlay = false;
                entry.set.instance.do_damage_overlay(v, blood_decals);
            }

            let (vx, vy, vz) = (v.x(), v.y(), v.z());
            let (sx, sy, sz) = (vx.floor() as i32, vy.floor() as i32, vz.floor() as i32);
            let sq_light = lighting.square_light(player as i32, sx, sy, sz);
            let ambient = sq_light
                .map(|l| {
                    let c = interpolate_light(&l.vert_light, vx - sx as f32, vy - sy as f32);
                    [c.r, c.g, c.b]
                })
                .unwrap_or([default_ambient; 3]);
            let (square_light, seen) = sq_light.map_or(([default_ambient; 3], true), |l| {
                let c = l.light_info;
                ([c.r, c.g, c.b], l.vis & 2 != 0)
            });
            let lights = SquareLight::from_result_lights(
                lighting.result_lights(player as i32, sx, sy, sz),
            );
            let _ = view;
            let frame = VehicleFrame {
                alpha: 1.0,
                ambient,
                lights,
                sky_texture_shift: 0.0,
                sky_textures: [None, None],
                square_light,
                seen,
                slope: None,
                default_shadow: default_shadow.clone(),
            };
            objects
                .moving
                .push(render_vehicle_object(v, &entry.set, &frame, camera));
        }
    }

    /// `WorldSimulation.destroy()`, `VehicleIDMap.Reset()`, `VirtualVehicleManager.Reset()`,
    /// `VehicleType.Reset()`, and Bullet/render teardown on `IngameState.exit`.
    pub fn reset(&mut self) {
        self.events_state.borrow_mut().clear_all_lights();
        self.render_store.borrow_mut().clear();
        self.brand_new_chunks.clear();
        self.pending_chunks.clear();
        world_sim::destroy();
        world_sim::reset();
        manager::reset();
        virtual_vehicle::with_manager(|m| m.reset());
        vehicle_type::reset();
        spawn_reset();
        env::set_vehicle_ids(None);
        let _ = Bullet::install(Box::new(NoPhysics));
    }

    /// Access to the runtime's [`VehicleIds`] for `StreamCtx::vehicle_ids`.
    pub fn ids(&self) -> &Rc<RefCell<VehicleIds>> {
        &self.ids
    }

    /// Helper constructing a fresh [`RuntimeVehicleSaver`] for `StreamCtx`.
    pub fn saver() -> RuntimeVehicleSaver<'static> {
        RuntimeVehicleSaver::default()
    }

    /// Helper constructing a fresh [`VirtualVehicleMeta`] for `StreamCtx`.
    pub fn meta_hook() -> VirtualVehicleMeta {
        VirtualVehicleMeta::default()
    }

    /// Current chunk physics helper (for tests and callers).
    pub fn chunk_physics(&self) -> &Rc<RefCell<ChunkPhysics>> {
        &self.chunk_physics
    }

    /// Current chunk grid width.
    pub fn chunk_grid_width(&self) -> i32 {
        self.chunk_grid_width
    }
}

fn spawn_reset() {
    pz_vehicles::spawn::reset_checked_vehicles();
}

impl IsoWorld {
    /// Initializes `WorldVehicles` (Bullet physics, scripts, `VehicleIds`,
    /// `VehicleZoneDistribution`) once meta bounds and `VehiclesDB2` are ready.
    pub fn init_vehicles(
        &mut self,
        lua: &LuaManager,
        sb: &Rc<RefCell<pz_sandbox::SandboxOptions>>,
        chunk_map: (i32, i32, i32),
    ) {
        let meta_bounds = with_world(|w| {
            w.meta_grid
                .as_ref()
                .map_or((0, 0, 0, 0), |m| (m.min_x, m.min_y, m.max_x, m.max_y))
        });
        let wv = WorldVehicles::init(
            &self.game_dir,
            lua,
            sb,
            self.vehicles.conn.as_mut(),
            self.sprites.clone(),
            self.render.clone(),
            meta_bounds,
            chunk_map,
        );
        self.vehicles_runtime = Some(wv);
    }

    /// Loads `inMeta` rows from `vehicles.db` into `VirtualVehicleManager` on
    /// `IngameState.enter`.
    pub fn load_vehicles_in_meta(&mut self, lua: &mut LuaManager) {
        let Some(mut wv) = self.vehicles_runtime.take() else {
            return;
        };
        let Some(sp_rc) = self.sprites.clone() else {
            self.vehicles_runtime = Some(wv);
            return;
        };
        let Some(save_tables) = self.save_tables.clone() else {
            self.vehicles_runtime = Some(wv);
            return;
        };
        let items: &dyn pz_world::save::items::ItemRegistry = self
            .items
            .as_ref()
            .map(|i| i as &dyn pz_world::save::items::ItemRegistry)
            .unwrap_or(&pz_world::save::items::AllItems);
        let sprites = sp_rc.borrow();
        let save_settings = pz_world::save::SaveSettings::default();
        let ctx = SaveCtx {
            sprites: &sprites,
            tables: &save_tables,
            settings: &save_settings,
            items,
            hooks: &pz_world::save::NoSaveHooks,
            meta: None,
        };
        wv.load_vehicles_in_meta(lua, self.vehicles.conn.as_mut(), &ctx);
        self.vehicles_runtime = Some(wv);
    }

    /// Runs pending chunk vehicle spawning / adoption (call with `&mut LuaManager`).
    pub fn process_pending_vehicle_chunks(&mut self, lua: &mut LuaManager) {
        let sprites = self.sprites.clone();
        if let Some(wv) = self.vehicles_runtime.as_mut() {
            wv.process_pending_chunks(lua, sprites.as_ref());
        }
    }

    /// Per-frame vehicle physics, vehicle/driver updates, animations and
    /// `VirtualVehicleManager` tick.
    pub fn update_vehicles(&mut self, lua: &mut LuaManager, sim: &FrameSim) {
        let Some(mut wv) = self.vehicles_runtime.take() else {
            return;
        };
        let sp_rc = self.sprites.clone();
        wv.process_pending_chunks(lua, sp_rc.as_ref());
        let save_tables = self.save_tables.clone();
        let items: &dyn pz_world::save::items::ItemRegistry = self
            .items
            .as_ref()
            .map(|i| i as &dyn pz_world::save::items::ItemRegistry)
            .unwrap_or(&pz_world::save::items::AllItems);
        let save_settings = pz_world::save::SaveSettings::default();
        if let (Some(sp_rc), Some(save_tables)) = (sp_rc.as_ref(), save_tables.as_ref()) {
            let sprites = sp_rc.borrow();
            let ctx = SaveCtx {
                sprites: &sprites,
                tables: save_tables,
                settings: &save_settings,
                items,
                hooks: &pz_world::save::NoSaveHooks,
                meta: None,
            };
            wv.update_frame(
                lua,
                sim,
                None,
                self.streamer.as_mut(),
                self.vehicles.conn.as_mut(),
                Some(&ctx),
            );
        } else {
            wv.update_frame(
                lua,
                sim,
                sp_rc.as_ref(),
                self.streamer.as_mut(),
                self.vehicles.conn.as_mut(),
                None,
            );
        }
        self.vehicles_runtime = Some(wv);
    }

    /// Queues all active `VirtualVehicleManager` rows onto the streamer during
    /// quit save.
    pub fn save_virtual_vehicles(&mut self) {
        let Some(mut wv) = self.vehicles_runtime.take() else {
            return;
        };
        if let (Some(sp_rc), Some(save_tables)) = (self.sprites.as_ref(), self.save_tables.as_ref())
        {
            let items: &dyn pz_world::save::items::ItemRegistry = self
                .items
                .as_ref()
                .map(|i| i as &dyn pz_world::save::items::ItemRegistry)
                .unwrap_or(&pz_world::save::items::AllItems);
            let sprites = sp_rc.borrow();
            let save_settings = pz_world::save::SaveSettings::default();
            let ctx = SaveCtx {
                sprites: &sprites,
                tables: save_tables,
                settings: &save_settings,
                items,
                hooks: &pz_world::save::NoSaveHooks,
                meta: None,
            };
            wv.save_virtual_vehicles(self.streamer.as_mut(), &ctx);
        }
        self.vehicles_runtime = Some(wv);
    }

    /// Tears down Bullet physics and vehicle runtime on `IngameState.exit`.
    pub fn reset_vehicles(&mut self) {
        if let Some(mut wv) = self.vehicles_runtime.take() {
            wv.reset();
        }
    }
}

#[cfg(test)]
mod tests;
