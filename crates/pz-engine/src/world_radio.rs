//! `zombie.radio.ZomboidRadio` and `IsoWaveSignal` (`IsoRadio` /
//! `IsoTelevision` / handheld `Radio`) engine integration.
//!
//! Owns the world's single [`ZomboidRadio`] instance, connects it to
//! `bind_radio` (Lua `getZomboidRadio()`, `getRadioAPI()`, `RecordedMedia`,
//! `RadioScriptManager`, `DeviceData`), drives `IsoWaveSignal.update` and
//! equipped radio items each frame, routes `GameTime`'s ten-minute
//! `RadioUpdateScripts` tick, supplies `ObjectLoadHooks` radio presets on
//! chunk load, persists `RADIO_SAVE.txt` / `recorded_media.bin`, and cleans
//! up emitters, lamppost lights and caches on `IngameState.exit`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use pz_audio::world_sound::SoundSource;
use pz_characters::character::items::has_tag;
use pz_inventory::device::{ChannelLists, DeviceData};
use pz_inventory::device_sim::{
    update_radio_item, ChatTarget, DeviceAudio, DeviceEnv, DeviceEvent, EmitterId, IsoObjectPos,
    Listener, MainsPower, MediaLine, NetMode, ParentKind, SquarePower, TvUpdate,
};
use pz_inventory::item::ItemKind;
use pz_inventory::shared::ItemRef;
use pz_inventory::template::ItemTemplate;
use pz_iso_render::chunk::dirty;
use pz_kahlua::Value;
use pz_lua_api::bind_character_sim::LuaGameCharacterRef;
use pz_lua_api::bind_radio::{
    self, DeviceOwner, LuaRadioText, RadioLuaHooks,
};
use pz_lua_api::bind_world::with_world;
use pz_lua_api::chat::characters::{attach_chat_element, chat_element, flush_chat};
use pz_lua_api::chat::element::ChatElement;
use pz_lua_api::chat::manager::with_chat_manager;
use pz_lua_api::chat::{ChatElementOwner, ChatOwnerKind, ChatSquare};
use pz_lua_api::LuaManager;
use pz_radio::{
    deliver_line, device_view, media_line, DeviceId, DeviceView, GameMode, ModRadioDirs,
    PlayerView, RadioSource, RadioText, RadioWorld, RecordedMedia, WithRadio, ZomboidRadio,
};
use pz_sound::emitter::ParentObject;
use pz_sound::params::FmodParameter;
use pz_sound::world_emitters::EmitterRef;
use pz_text::UIFont;
use pz_vehicles::env::VehicleDevices;
use pz_vehicles::{BaseVehicle, PartRef, VehiclePart};
use pz_world::objects::{ObjectLoadHooks, OverlaySprite, SpriteName};
use pz_world::sprite::SpriteIdx;
use pz_world::world_state::{PowerSettings, WorldState};
use pz_world::{GameRand, IsoObject, IsoObjectRef, IsoSpriteManager, ObjectKind};
use pz_world_render::lighting::CellLight;

use crate::iso_world::{opt_bool, opt_int, IsoWorld};
use crate::world_render::WorldRender;

const MAX_PLAYERS: usize = 4;

thread_local! {
    /// `ZomboidRadio.louisvilleObfuscation`: static in the original, so it
    /// survives `ZomboidRadio.Reset()` into the next loaded world.
    static LOUISVILLE_OBFUSCATION: Cell<bool> = const { Cell::new(false) };
}

/// Resets the static `louisvilleObfuscation` flag (for tests).
#[cfg(test)]
pub(crate) fn reset_louisville_obfuscation_for_test() {
    LOUISVILLE_OBFUSCATION.with(|c| c.set(false));
}

fn kind_device_data_mut(kind: &mut ObjectKind) -> Option<&mut DeviceData> {
    match kind {
        ObjectKind::Radio(d) | ObjectKind::Television(d) => Some(d),
        _ => None,
    }
}

/// `Translator.getText` / `getTextMediaEN` adapter for `pz-radio` and
/// `bind_radio`.
#[derive(Clone, Copy, Default)]
pub struct EngineRadioText;

impl RadioText for EngineRadioText {
    fn get_text(&self, key: &str) -> String {
        pz_lua_api::bind_text::with_translator(|t| t.get_text(key, &[]))
            .unwrap_or_else(|| key.to_string())
    }
}

impl LuaRadioText for EngineRadioText {
    fn get_text(&self, key: &str) -> String {
        RadioText::get_text(self, key)
    }

    fn get_text_media_en(&self, key: &str) -> String {
        pz_lua_api::bind_text::with_translator(|t| t.get_text_media_en(key))
            .unwrap_or_else(|| key.to_string())
    }
}

/// Builds the [`RadioSource`] from the active `GameFs`, `Core` and `Translator`.
pub fn radio_source(
    game_dir: &Path,
    fs: &pz_fs::GameFs,
    core: &pz_core::core::Core,
) -> RadioSource {
    let mods = fs
        .mod_ids()
        .iter()
        .filter_map(|id| fs.mod_dir(id).and_then(|d| fs.mod_info(d)))
        .map(|m| {
            let dir = |s: &str| (!s.is_empty()).then(|| PathBuf::from(s));
            ModRadioDirs {
                common_dir: dir(&m.common_dir),
                version_dir: dir(&m.version_dir),
            }
        })
        .collect();
    let (language, default_language, available_languages) =
        pz_lua_api::bind_text::with_translator(|t| {
            (
                t.get_language().name.clone(),
                t.default_language().name.clone(),
                t.languages
                    .languages()
                    .iter()
                    .map(|l| l.name.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_else(|| ("EN".to_string(), "EN".to_string(), vec!["EN".to_string()]));
    RadioSource {
        media_dir: game_dir.join("media"),
        mods,
        content_translations_enabled: core.get_content_translations_enabled(),
        language,
        default_language,
        available_languages,
    }
}

/// Cached radio emitter plus its live `DeviceVolume` parameter cell and
/// owning world device (if any) so dropped objects still release emitters.
struct OwnedRadioEmitter {
    emitter: EmitterRef,
    device_volume: Rc<Cell<f32>>,
    owner: Option<DeviceId>,
}

/// Pool of sound emitters currently held by active radio/TV devices.
#[derive(Default)]
struct RadioEmitterPool {
    next_id: EmitterId,
    emitters: HashMap<EmitterId, OwnedRadioEmitter>,
}

impl RadioEmitterPool {
    fn return_all(&mut self) {
        let drained: Vec<_> = self.emitters.drain().map(|(_, e)| e.emitter).collect();
        Self::release_emitters(drained);
    }

    /// Stops and returns every emitter owned by `owner`, even if the owning
    /// `IsoObject` was already dropped before cleanup ran.
    fn return_for_device(&mut self, owner: DeviceId) {
        let ids: Vec<EmitterId> = self
            .emitters
            .iter()
            .filter_map(|(&eid, e)| (e.owner == Some(owner)).then_some(eid))
            .collect();
        let drained: Vec<_> = ids
            .into_iter()
            .filter_map(|eid| self.emitters.remove(&eid).map(|e| e.emitter))
            .collect();
        Self::release_emitters(drained);
    }

    fn release_emitters(drained: Vec<EmitterRef>) {
        if drained.is_empty() {
            return;
        }
        let sound = pz_lua_api::bind_audio::sound_system();
        let mut sys = sound.borrow_mut();
        sys.with_world_emitters(|we, ctx| {
            for e in drained {
                e.borrow_mut().stop_all(ctx);
                e.borrow_mut().clear_parameters();
                we.return_ownership(e);
            }
        });
    }
}

/// [`DeviceAudio`] over the shared `SoundSystem::world_emitters`.
struct EngineRadioAudio {
    pool: Rc<RefCell<RadioEmitterPool>>,
    owner: Option<DeviceId>,
}

impl EngineRadioAudio {
    fn new(pool: Rc<RefCell<RadioEmitterPool>>, owner: Option<DeviceId>) -> Self {
        Self { pool, owner }
    }

    fn emitter_of(&self, id: EmitterId) -> Option<EmitterRef> {
        self.pool
            .borrow()
            .emitters
            .get(&id)
            .map(|e| e.emitter.clone())
    }
}

impl DeviceAudio for EngineRadioAudio {
    fn get_free_emitter(&mut self, x: f32, y: f32, z: i32) -> EmitterId {
        let device_volume = Rc::new(Cell::new(1.0f32));
        let vol_src = device_volume.clone();
        let sound = pz_lua_api::bind_audio::sound_system();
        let emitter = sound.borrow_mut().with_world_emitters(|we, ctx| {
            let e = we.get_free();
            we.take_ownership(&e);
            {
                let mut em = e.borrow_mut();
                em.set_pos(x, y, z as f32);
                let desc = ctx.backend.parameter_description("DeviceVolume");
                em.add_parameter(FmodParameter::local(
                    "DeviceVolume",
                    desc,
                    Box::new(move || vol_src.get()),
                ));
            }
            e
        });
        let mut pool = self.pool.borrow_mut();
        pool.next_id = pool.next_id.wrapping_add(1).max(1);
        let id = pool.next_id;
        pool.emitters.insert(
            id,
            OwnedRadioEmitter {
                emitter,
                device_volume,
                owner: self.owner,
            },
        );
        id
    }

    fn return_emitter(&mut self, id: EmitterId) {
        let removed = self.pool.borrow_mut().emitters.remove(&id);
        let Some(owned) = removed else { return };
        let sound = pz_lua_api::bind_audio::sound_system();
        sound.borrow_mut().with_world_emitters(|we, ctx| {
            owned.emitter.borrow_mut().stop_all(ctx);
            owned.emitter.borrow_mut().clear_parameters();
            we.return_ownership(owned.emitter);
        });
    }

    fn set_pos(&mut self, id: EmitterId, x: f32, y: f32, z: i32) {
        if let Some(e) = self.emitter_of(id) {
            e.borrow_mut().set_pos(x, y, z as f32);
        }
    }

    fn play_sound(&mut self, id: EmitterId, name: &str) -> i64 {
        let Some(e) = self.emitter_of(id) else {
            return 0;
        };
        let sound = pz_lua_api::bind_audio::sound_system();
        let res = sound
            .borrow_mut()
            .with_world_emitters(|_, ctx| e.borrow_mut().play_sound(name, ctx));
        res
    }

    fn play_sound_impl(&mut self, id: EmitterId, name: &str) -> i64 {
        let Some(e) = self.emitter_of(id) else {
            return 0;
        };
        let sound = pz_lua_api::bind_audio::sound_system();
        let res = sound
            .borrow_mut()
            .with_world_emitters(|_, ctx| e.borrow_mut().play_sound_impl(name, ParentObject::None, ctx));
        res
    }

    fn is_using_parameter(&mut self, id: EmitterId, handle: i64, name: &str) -> bool {
        let Some(e) = self.emitter_of(id) else {
            return false;
        };
        let sound = pz_lua_api::bind_audio::sound_system();
        let res = sound
            .borrow_mut()
            .with_world_emitters(|_, ctx| e.borrow().is_using_parameter(handle, name, ctx));
        res
    }

    fn set_volume(&mut self, id: EmitterId, handle: i64, volume: f32) {
        if let Some(e) = self.emitter_of(id) {
            e.borrow_mut().set_volume(handle, volume);
        }
    }

    fn stop_sound(&mut self, id: EmitterId, handle: i64) {
        let Some(e) = self.emitter_of(id) else {
            return;
        };
        let sound = pz_lua_api::bind_audio::sound_system();
        sound.borrow_mut().with_world_emitters(|_, ctx| {
            let _ = e.borrow_mut().stop_sound(handle, ctx);
        });
    }

    fn stop_or_trigger_sound(&mut self, id: EmitterId, handle: i64) {
        let Some(e) = self.emitter_of(id) else {
            return;
        };
        let sound = pz_lua_api::bind_audio::sound_system();
        sound
            .borrow_mut()
            .with_world_emitters(|_, ctx| e.borrow_mut().stop_or_trigger_sound(handle, ctx));
    }

    fn stop_or_trigger_sound_by_name(&mut self, id: EmitterId, name: &str) {
        let Some(e) = self.emitter_of(id) else {
            return;
        };
        let sound = pz_lua_api::bind_audio::sound_system();
        sound
            .borrow_mut()
            .with_world_emitters(|_, ctx| e.borrow_mut().stop_or_trigger_sound_by_name(name, ctx));
    }

    fn is_playing(&mut self, id: EmitterId, name: &str) -> bool {
        self.emitter_of(id)
            .is_some_and(|e| e.borrow().is_playing_name(name))
    }

    fn tick(&mut self, id: EmitterId) {
        let Some(e) = self.emitter_of(id) else {
            return;
        };
        let sound = pz_lua_api::bind_audio::sound_system();
        sound
            .borrow_mut()
            .with_world_emitters(|_, ctx| e.borrow_mut().tick(ctx));
    }

    fn update_device_volume(&mut self, id: Option<EmitterId>, volume: f32) {
        let Some(id) = id else { return };
        if let Some(owned) = self.pool.borrow().emitters.get(&id) {
            owned.device_volume.set(volume);
        }
    }
}

/// [`ChatElementOwner`] for a world `IsoRadio` / `IsoTelevision`.
struct WorldDeviceChatOwner {
    pos: (i32, i32, i32),
    is_tv: bool,
}

impl ChatElementOwner for WorldDeviceChatOwner {
    fn get_x(&self) -> f32 {
        self.pos.0 as f32
    }
    fn get_y(&self) -> f32 {
        self.pos.1 as f32
    }
    fn get_z(&self) -> f32 {
        self.pos.2 as f32
    }
    fn get_square(&self) -> Option<ChatSquare> {
        with_world(|w| {
            let cell = w.cell.as_ref()?;
            let sq = cell.get_grid_square(self.pos.0, self.pos.1, self.pos.2)?;
            let sq = sq.borrow();
            let room = (sq.room_id != -1).then_some(sq.room_id as u64);
            let building = sq
                .associated_building
                .or_else(|| {
                    let meta = w.meta_grid.as_ref()?;
                    sq.room_def(meta).and_then(|r| r.building)
                })
                .map(|b| b.0 as u64);
            Some(ChatSquare { building, room })
        })
    }
    fn kind(&self) -> ChatOwnerKind {
        if self.is_tv {
            ChatOwnerKind::Television
        } else {
            ChatOwnerKind::Radio
        }
    }
}

/// `ChatElementOwner` for a live vehicle radio/TV [`VehiclePart`].
struct VehiclePartChatOwner {
    part: Weak<VehiclePart>,
}

impl ChatElementOwner for VehiclePartChatOwner {
    fn get_x(&self) -> f32 {
        self.part.upgrade().map_or(0.0, |p| p.vehicle().x())
    }
    fn get_y(&self) -> f32 {
        self.part.upgrade().map_or(0.0, |p| p.vehicle().y())
    }
    fn get_z(&self) -> f32 {
        self.part.upgrade().map_or(0.0, |p| p.vehicle().z())
    }
    fn get_square(&self) -> Option<ChatSquare> {
        let part = self.part.upgrade()?;
        let v = part.vehicle();
        let (x, y, z) = (
            pz_world::java::fastfloor(v.x()),
            pz_world::java::fastfloor(v.y()),
            pz_world::java::fastfloor(v.z()),
        );
        with_world(|w| {
            let cell = w.cell.as_ref()?;
            let sq = cell.get_grid_square(x, y, z)?;
            let sq = sq.borrow();
            let room = (sq.room_id != -1).then_some(sq.room_id as u64);
            let building = sq
                .associated_building
                .or_else(|| {
                    let meta = w.meta_grid.as_ref()?;
                    sq.room_def(meta).and_then(|r| r.building)
                })
                .map(|b| b.0 as u64);
            Some(ChatSquare { building, room })
        })
    }
    fn kind(&self) -> ChatOwnerKind {
        ChatOwnerKind::VehiclePart
    }
}

/// Per-part runtime bookkeeping for a registered vehicle signal device
/// (`VehiclePart` with `DeviceData`).
struct VehicleDeviceEntry {
    part: Weak<VehiclePart>,
    two_way: bool,
    _chat_owner: Rc<VehiclePartChatOwner>,
    chat: Rc<RefCell<ChatElement>>,
}

/// Per-object runtime bookkeeping for a registered world `IsoWaveSignal`.
struct WorldDeviceEntry {
    object: Weak<RefCell<IsoObject>>,
    pos: (i32, i32, i32),
    is_tv: bool,
    two_way: bool,
    cached_sprite: Option<SpriteIdx>,
    screen_sprites: Vec<String>,
    lamppost_key: Option<u64>,
    _chat_owner: Rc<WorldDeviceChatOwner>,
    chat: Rc<RefCell<ChatElement>>,
}

/// Deferred Lua callback produced while updating radio devices or delivering
/// broadcast lines (fired only after every `RefCell` guard has been dropped).
enum PendingLuaEvent {
    OnDeviceText {
        guid: Option<String>,
        codes: String,
        pos: Option<[f32; 3]>,
        text: String,
        device: Value,
    },
    FlushPlayerChat(LuaGameCharacterRef),
}

/// Sandbox power settings captured at world init for `IsoGridSquare` mains
/// power checks (`canBePoweredHere`).
#[derive(Clone, Copy, Debug)]
struct RadioPowerConfig {
    time_since_apo: i32,
    elec_shut_modifier: i32,
    allow_exterior_generator: bool,
    generator_tile_range: i32,
    generator_vertical_power_range: i32,
    hearing: i32,
}

impl Default for RadioPowerConfig {
    fn default() -> Self {
        Self {
            time_since_apo: 1,
            elec_shut_modifier: 14,
            allow_exterior_generator: true,
            generator_tile_range: 20,
            generator_vertical_power_range: 3,
            hearing: 2,
        }
    }
}

/// Shared mutable state of the world's radio subsystem (separate from
/// `ZomboidRadio` and `RadioEmitterPool` to prevent `RefCell` aliasing).
struct WorldRadioState {
    save_dir: PathBuf,
    no_save: bool,
    time_since_apo: i32,
    power: RadioPowerConfig,
    next_device_id: u64,
    world_devices: HashMap<DeviceId, WorldDeviceEntry>,
    object_to_id: HashMap<usize, DeviceId>,
    vehicle_devices: HashMap<DeviceId, VehicleDeviceEntry>,
    part_to_id: HashMap<usize, DeviceId>,
    item_devices: HashMap<DeviceId, Weak<RefCell<pz_inventory::InventoryItem>>>,
    item_to_id: HashMap<usize, DeviceId>,
    prev_equipped_radios: [Option<ItemRef>; MAX_PLAYERS],
    pending_lua: Vec<PendingLuaEvent>,
    /// Queued `(DeviceId, DeviceEvent)` emitted inside `RadioWorld::add_device_text`
    /// while `ZomboidRadio` is mutably borrowed.
    deferred_device_events: Vec<(EventTarget, Vec<DeviceEvent>)>,
}

/// Identifies the owner of a batch of [`DeviceEvent`]s.
#[derive(Clone)]
enum EventTarget {
    World(DeviceId),
    Item {
        item: ItemRef,
        player: Option<LuaGameCharacterRef>,
        pos: [f32; 3],
    },
    Vehicle {
        id: DeviceId,
        pos: [f32; 3],
        device_lua: Value,
    },
}

impl WorldRadioState {
    fn new(save_dir: PathBuf, no_save: bool, time_since_apo: i32, power: RadioPowerConfig) -> Self {
        Self {
            save_dir,
            no_save,
            time_since_apo,
            power,
            next_device_id: 0,
            world_devices: HashMap::new(),
            object_to_id: HashMap::new(),
            vehicle_devices: HashMap::new(),
            part_to_id: HashMap::new(),
            item_devices: HashMap::new(),
            item_to_id: HashMap::new(),
            prev_equipped_radios: Default::default(),
            pending_lua: Vec::new(),
            deferred_device_events: Vec::new(),
        }
    }

    fn alloc_id(&mut self) -> DeviceId {
        self.next_device_id = self.next_device_id.wrapping_add(1).max(1);
        DeviceId(self.next_device_id)
    }

    fn id_for_item(&mut self, item: &ItemRef) -> DeviceId {
        let key = Rc::as_ptr(item) as usize;
        if let Some(&id) = self.item_to_id.get(&key) {
            if self
                .item_devices
                .get(&id)
                .and_then(Weak::upgrade)
                .is_some_and(|i| Rc::ptr_eq(&i, item))
            {
                return id;
            }
        }
        let id = self.alloc_id();
        self.item_to_id.insert(key, id);
        self.item_devices.insert(id, Rc::downgrade(item));
        id
    }
}

/// Queries `SquarePower` for `(x, y, z)` against the live world cell/meta/sprites.
fn query_square_power(
    pos: (i32, i32, i32),
    sprites: Option<&Rc<RefCell<IsoSpriteManager>>>,
    cfg: RadioPowerConfig,
) -> (MainsPower, bool) {
    let Some(sprites) = sprites else {
        return (MainsPower::Square(None), false);
    };
    let Ok(sprites) = sprites.try_borrow() else {
        return (MainsPower::Square(None), false);
    };
    let world_age_days = pz_lua_api::bind_climate::shared_game_time()
        .borrow()
        .world_age_days_since_begin(cfg.time_since_apo) as f32;
    with_world(|w| {
        let (Some(cell), Some(meta)) = (w.cell.as_ref(), w.meta_grid.as_ref()) else {
            return (MainsPower::Square(None), false);
        };
        let Some(sq_ref) = cell.get_grid_square(pos.0, pos.1, pos.2) else {
            return (MainsPower::Square(None), false);
        };
        let sq = sq_ref.borrow();
        let has_room = sq.room_id != -1;
        let power = PowerSettings {
            hydro_power_on: world_age_days <= cfg.elec_shut_modifier as f32,
            world_age_days,
            elec_shut_modifier: cfg.elec_shut_modifier,
            allow_exterior_generator: cfg.allow_exterior_generator,
            generator_tile_range: cfg.generator_tile_range,
            generator_vertical_power_range: cfg.generator_vertical_power_range,
        };
        let no_power = |x: i32, y: i32| {
            meta.get_zones_at(x, y, pos.2).iter().any(|&zid| {
                let zt = meta.zone(zid).zone_type.as_str();
                zt == "NoPower" || zt == "NoPowerOrWater"
            })
        };
        let ws = WorldState {
            cell,
            meta,
            sprites: &sprites,
            power,
            no_power_zone: &no_power,
            players: Vec::new(),
        };
        let sp = SquarePower {
            grid_power: ws.has_grid_power(&sq),
            have_electricity: ws.have_electricity(&sq),
            has_room,
        };
        (MainsPower::Square(Some(sp)), has_room)
    })
}

fn radio_local_players() -> [Option<LuaGameCharacterRef>; MAX_PLAYERS] {
    std::array::from_fn(pz_lua_api::bind_players::player)
}

/// Closest living local player's squared distance to `(x, y, z)`.
fn closest_listener_distance_sq(x: f32, y: f32, z: f32) -> f32 {
    let mut best = f32::MAX;
    for p in radio_local_players().into_iter().flatten() {
        let c = p.borrow();
        if c.is_dead() {
            continue;
        }
        let dx = c.vars.x - x;
        let dy = c.vars.y - y;
        let dz = (c.vars.z - z) * 3.0;
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 < best {
            best = d2;
        }
    }
    best
}

/// Finds a character's currently equipped radio item (`IsoGameCharacter.getEquipedRadio`):
/// primary hand, secondary hand, or back worn slot.
pub(crate) fn character_equipped_radio(chr: &LuaGameCharacterRef) -> Option<ItemRef> {
    let c = chr.borrow();
    let check_id = |id: Option<i32>| -> Option<ItemRef> {
        let item = c.items.item(id?)?;
        let is_radio = matches!(item.borrow().kind, ItemKind::Radio(_, _));
        is_radio.then_some(item)
    };
    if let Some(r) = check_id(c.items.primary_hand_item) {
        return Some(r);
    }
    if let Some(r) = check_id(c.items.secondary_hand_item) {
        return Some(r);
    }
    if let Some(worn) = &c.items.worn_items {
        for entry in worn.items() {
            if let Some(loc) = &entry.location {
                let p = loc.name.as_str();
                if p.eq_ignore_ascii_case("back")
                    || p.to_ascii_lowercase().ends_with(":back")
                {
                    if let Some(r) = check_id(Some(entry.item)) {
                        return Some(r);
                    }
                }
            }
        }
    }
    None
}

fn player_is_deaf(chr: &LuaGameCharacterRef) -> bool {
    let sm = pz_lua_api::bind_registry::shared_script_manager();
    chr.borrow().has_trait(&sm, "DEAF").unwrap_or(false)
}

/// Snapshot [`DeviceEnv`] for a single device call. Events are collected in
/// `events` so the caller can apply them after all device borrows are released.
struct EngineDeviceEnv {
    parent: ParentKind,
    mains: MainsPower,
    iso_object: Option<IsoObjectPos>,
    parent_pos: [f32; 3],
    has_sound_source: bool,
    in_room: bool,
    item_player_deaf: Option<bool>,
    audio: EngineRadioAudio,
    events: Vec<DeviceEvent>,
}

impl EngineDeviceEnv {
    fn for_world_object(
        pos: (i32, i32, i32),
        sprites: Option<&Rc<RefCell<IsoSpriteManager>>>,
        power_cfg: RadioPowerConfig,
        pool: Rc<RefCell<RadioEmitterPool>>,
        owner: Option<DeviceId>,
    ) -> Self {
        let (mains, in_room) = query_square_power(pos, sprites, power_cfg);
        Self {
            parent: ParentKind::Iso,
            mains,
            iso_object: Some(IsoObjectPos {
                x: pos.0 as f32,
                y: pos.1 as f32,
                z: pos.2 as f32,
                character: false,
            }),
            parent_pos: [pos.0 as f32, pos.1 as f32, pos.2 as f32],
            has_sound_source: true,
            in_room,
            item_player_deaf: None,
            audio: EngineRadioAudio::new(pool, owner),
            events: Vec::new(),
        }
    }

    fn for_item(
        player: Option<&LuaGameCharacterRef>,
        pool: Rc<RefCell<RadioEmitterPool>>,
    ) -> Self {
        let (pos, in_room, deaf, has_source) = match player {
            Some(p) => {
                let c = p.borrow();
                let pos = [c.vars.x, c.vars.y, c.vars.z];
                let in_room = c.environment.square_in_room;
                drop(c);
                (pos, in_room, Some(player_is_deaf(p)), true)
            }
            None => ([0.0; 3], false, None, false),
        };
        Self {
            parent: ParentKind::Item,
            mains: MainsPower::Square(None),
            iso_object: player.map(|_| IsoObjectPos {
                x: pos[0],
                y: pos[1],
                z: pos[2],
                character: true,
            }),
            parent_pos: pos,
            has_sound_source: has_source,
            in_room,
            item_player_deaf: deaf,
            audio: EngineRadioAudio::new(pool, None),
            events: Vec::new(),
        }
    }

    fn for_vehicle_part(
        vehicle: &BaseVehicle,
        part: &PartRef,
        pool: Rc<RefCell<RadioEmitterPool>>,
        owner: Option<DeviceId>,
    ) -> Self {
        let pos = [vehicle.x(), vehicle.y(), vehicle.z()];
        Self {
            parent: ParentKind::Vehicle,
            mains: MainsPower::Vehicle {
                uninstalled: part.is_inventory_item_uninstalled(),
                has_power: vehicle.battery_charge() > 0.0,
            },
            iso_object: Some(IsoObjectPos {
                x: pos[0],
                y: pos[1],
                z: pos[2],
                character: false,
            }),
            parent_pos: pos,
            has_sound_source: true,
            in_room: false,
            item_player_deaf: None,
            audio: EngineRadioAudio::new(pool, owner),
            events: Vec::new(),
        }
    }
}

/// Ensures `part` (if it currently owns `DeviceData`) is registered with both
/// [`WorldRadioState`] and [`ZomboidRadio`]. Idempotent by `PartRef` identity.
fn ensure_vehicle_part_registered(
    radio: &Rc<RefCell<ZomboidRadio>>,
    state: &Rc<RefCell<WorldRadioState>>,
    part: &PartRef,
) -> Option<DeviceId> {
    let two_way = part.st().device_data.as_ref()?.two_way;
    let key = Rc::as_ptr(part) as usize;
    let existing = {
        let st = state.borrow();
        st.part_to_id.get(&key).copied().and_then(|id| {
            let entry = st.vehicle_devices.get(&id)?;
            entry
                .part
                .upgrade()
                .is_some_and(|p| Rc::ptr_eq(&p, part))
                .then_some((id, entry.two_way))
        })
    };
    if let Some((id, prev_two_way)) = existing {
        if prev_two_way != two_way {
            if let Ok(mut r) = radio.try_borrow_mut() {
                r.unregister_device(id, prev_two_way);
                r.register_device(id, two_way);
            }
            if let Some(entry) = state.borrow_mut().vehicle_devices.get_mut(&id) {
                entry.two_way = two_way;
            }
        }
        return Some(id);
    }

    let chat_owner = Rc::new(VehiclePartChatOwner {
        part: Rc::downgrade(part),
    });
    let weak_owner: Weak<dyn ChatElementOwner> =
        Rc::downgrade(&chat_owner) as Weak<dyn ChatElementOwner>;
    let chat = Rc::new(RefCell::new(ChatElement::new(
        weak_owner,
        5,
        Some("device"),
    )));

    let id = {
        let mut st = state.borrow_mut();
        let id = st.alloc_id();
        st.part_to_id.insert(key, id);
        st.vehicle_devices.insert(
            id,
            VehicleDeviceEntry {
                part: Rc::downgrade(part),
                two_way,
                _chat_owner: chat_owner,
                chat,
            },
        );
        id
    };
    if let Ok(mut r) = radio.try_borrow_mut() {
        r.register_device(id, two_way);
    }
    Some(id)
}

impl DeviceEnv for EngineDeviceEnv {
    fn net(&self) -> NetMode {
        NetMode::SinglePlayer
    }

    fn minutes_stamp(&self) -> i64 {
        pz_lua_api::bind_climate::shared_game_time()
            .borrow()
            .minutes_stamp
    }

    fn multiplier(&self) -> f32 {
        let gt = pz_lua_api::bind_climate::shared_game_time();
        let ctx = pz_lua_api::bind_climate::LuaClimateContext::new();
        let res = gt.borrow().get_multiplier(&ctx);
        res
    }

    fn parent(&self) -> ParentKind {
        self.parent
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        pz_core::random::next(bound)
    }

    fn next_float(&mut self) -> f32 {
        pz_core::random::next_float(0.0, 1.0)
    }

    fn mains_power(&self) -> MainsPower {
        self.mains
    }

    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        bind_radio::with_radio(|r| r.channel_list(category)).flatten()
    }

    fn media_exists(&self, index: i16) -> bool {
        bind_radio::with_media(|m| m.media_data_from_index(index).is_some()).unwrap_or(false)
    }

    fn media_line(&self, index: i16, line: i32) -> Option<MediaLine> {
        bind_radio::with_media(|m| media_line(m, &EngineRadioText, index, line)).flatten()
    }

    fn iso_object(&self) -> Option<IsoObjectPos> {
        self.iso_object
    }

    fn closest_listener_distance_sq(&self, x: f32, y: f32, z: f32) -> f32 {
        closest_listener_distance_sq(x, y, z)
    }

    fn parent_pos(&self) -> [f32; 3] {
        self.parent_pos
    }

    fn has_sound_source(&self) -> bool {
        self.has_sound_source
    }

    fn in_room(&self) -> bool {
        self.in_room
    }

    fn item_player_deaf(&self) -> Option<bool> {
        self.item_player_deaf
    }

    fn event(&mut self, e: DeviceEvent) {
        self.events.push(e);
    }

    fn audio(&mut self) -> Option<&mut dyn DeviceAudio> {
        Some(&mut self.audio)
    }
}

/// Resolving `IsoTelevision.setupDefaultScreens()` screen sprites at offsets
/// 16, 32, 48, 64 from `sprite`.
fn resolve_tv_screen_sprites(sprites: &IsoSpriteManager, sprite: SpriteIdx) -> Vec<String> {
    let mut out = Vec::new();
    for offset in [16, 32, 48, 64] {
        if let Ok(Some(idx)) = sprites.sprite_with_offset(sprite, offset) {
            let name = sprites.sprites[idx].name();
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// Applies collected [`DeviceEvent`]s cleanly after `DeviceData` and
/// `ZomboidRadio` borrows have been released.
fn apply_device_events(
    target: EventTarget,
    events: Vec<DeviceEvent>,
    state: &Rc<RefCell<WorldRadioState>>,
    render: &Rc<RefCell<Option<WorldRender>>>,
) {
    if events.is_empty() {
        return;
    }
    let hearing = state.borrow().power.hearing;
    let world_sounds = pz_lua_api::bind_zombie::shared_world_sounds();

    for event in events {
        match event {
            DeviceEvent::WorldSound {
                repeating,
                radius,
                volume,
                stress,
            } => {
                let (x, y, z) = match &target {
                    EventTarget::World(id) => match state.borrow().world_devices.get(id) {
                        Some(e) => e.pos,
                        None => continue,
                    },
                    EventTarget::Item { pos, .. } | EventTarget::Vehicle { pos, .. } => (
                        pz_world::java::fastfloor(pos[0]),
                        pz_world::java::fastfloor(pos[1]),
                        pz_world::java::fastfloor(pos[2]),
                    ),
                };
                with_world(|w| {
                    let Some(cell) = w.cell.as_ref() else { return };
                    let mut fx = pz_world_sound::NoEffects;
                    let mut adapter = pz_world_sound::WorldSoundAdapter::new(cell, hearing, &mut fx);
                    let src = Some(SoundSource {
                        is_player_base: matches!(target, EventTarget::World(_)),
                        is_player: matches!(target, EventTarget::Item { .. }),
                        ..Default::default()
                    });
                    if repeating {
                        let _ = world_sounds.borrow_mut().add_sound_repeating(
                            &mut adapter,
                            src,
                            x,
                            y,
                            z,
                            radius,
                            volume,
                            stress,
                            0.0,
                            1.0,
                        );
                    } else {
                        let _ = world_sounds.borrow_mut().add_sound_stress(
                            &mut adapter,
                            src,
                            x,
                            y,
                            z,
                            radius,
                            volume,
                            stress,
                            0.0,
                            1.0,
                        );
                    }
                });
            }
            DeviceEvent::ChatLine {
                target: chat_target,
                text,
                rgb,
                range,
            } => match chat_target {
                ChatTarget::Device => {
                    let chat = match &target {
                        EventTarget::World(id) => {
                            state.borrow().world_devices.get(id).map(|e| e.chat.clone())
                        }
                        EventTarget::Vehicle { id, .. } => state
                            .borrow()
                            .vehicle_devices
                            .get(id)
                            .map(|e| e.chat.clone()),
                        EventTarget::Item { .. } => None,
                    };
                    if let Some(chat) = chat {
                        chat.borrow_mut().add_chat_line_full(
                            &text,
                            rgb[0],
                            rgb[1],
                            rgb[2],
                            UIFont::Medium,
                            range as f32,
                            "default",
                            true,
                            true,
                            true,
                            true,
                            true,
                            true,
                        );
                    }
                }
                ChatTarget::Player => {
                    if let EventTarget::Item {
                        player: Some(p), ..
                    } = &target
                    {
                        attach_chat_element(p);
                        let chat = chat_element(p);
                        chat.borrow_mut().add_chat_line_full(
                            &text,
                            rgb[0],
                            rgb[1],
                            rgb[2],
                            UIFont::Medium,
                            range as f32,
                            "radio",
                            true,
                            true,
                            true,
                            true,
                            true,
                            true,
                        );
                    }
                }
            },
            DeviceEvent::SayRadio {
                text,
                rgb,
                range,
                channel,
            } => {
                if let EventTarget::Item {
                    player: Some(p), ..
                } = &target
                {
                    p.borrow_mut().say_radio(
                        &text,
                        rgb[0],
                        rgb[1],
                        rgb[2],
                        range as f32,
                        channel,
                        "radio",
                    );
                    state
                        .borrow_mut()
                        .pending_lua
                        .push(PendingLuaEvent::FlushPlayerChat(p.clone()));
                }
            }
            DeviceEvent::ShowStaticRadioSound(text) => {
                let _ = with_chat_manager(|m| m.show_static_radio_sound(&text));
            }
            DeviceEvent::ShowRadioMessage { text, channel } => {
                let _ = with_chat_manager(|m| m.show_radio_message_text(&text, channel));
            }
            DeviceEvent::OnDeviceText {
                guid,
                codes,
                pos,
                text,
            } => {
                let device = match &target {
                    EventTarget::World(id) => state
                        .borrow()
                        .world_devices
                        .get(id)
                        .and_then(|e| e.object.upgrade())
                        .map(|o| bind_radio::world_object_value(&o))
                        .unwrap_or(Value::Nil),
                    EventTarget::Item { item, .. } => pz_lua_api::bind_items::item_value(item),
                    EventTarget::Vehicle { device_lua, .. } => device_lua.clone(),
                };
                state
                    .borrow_mut()
                    .pending_lua
                    .push(PendingLuaEvent::OnDeviceText {
                        guid,
                        codes,
                        pos,
                        text,
                        device,
                    });
            }
            DeviceEvent::AddLight { radius } => {
                if let EventTarget::World(id) = &target {
                    let mut st = state.borrow_mut();
                    if let Some(entry) = st.world_devices.get_mut(id) {
                        if let Ok(mut r) = render.try_borrow_mut() {
                            if let Some(wr) = r.as_mut() {
                                if let Some(old) = entry.lamppost_key.take() {
                                    wr.lighting.remove_lamppost(old);
                                }
                                let key = wr.lighting.add_lamppost(CellLight::new(
                                    entry.pos.0,
                                    entry.pos.1,
                                    entry.pos.2,
                                    0.0,
                                    0.0,
                                    1.0,
                                    radius,
                                ));
                                entry.lamppost_key = Some(key);
                                wr.invalidate_floor_chunk(
                                    entry.pos.0.div_euclid(8),
                                    entry.pos.1.div_euclid(8),
                                    entry.pos.2,
                                    dirty::LIGHTING,
                                );
                            }
                        }
                    }
                }
            }
            DeviceEvent::LightChanged { radius, rgb } => {
                if let EventTarget::World(id) = &target {
                    let st = state.borrow();
                    if let Some(entry) = st.world_devices.get(id) {
                        if let (Some(key), Ok(mut r)) = (entry.lamppost_key, render.try_borrow_mut())
                        {
                            if let Some(wr) = r.as_mut() {
                                if let Some(light) = wr.lighting.lamppost_mut(key) {
                                    light.radius = radius;
                                    light.r = rgb[0];
                                    light.g = rgb[1];
                                    light.b = rgb[2];
                                }
                                wr.invalidate_floor_chunk(
                                    entry.pos.0.div_euclid(8),
                                    entry.pos.1.div_euclid(8),
                                    entry.pos.2,
                                    dirty::LIGHTING,
                                );
                            }
                        }
                    }
                }
            }
            DeviceEvent::RemoveLight => {
                if let EventTarget::World(id) = &target {
                    let mut st = state.borrow_mut();
                    if let Some(entry) = st.world_devices.get_mut(id) {
                        if let Some(key) = entry.lamppost_key.take() {
                            if let Ok(mut r) = render.try_borrow_mut() {
                                if let Some(wr) = r.as_mut() {
                                    wr.lighting.remove_lamppost(key);
                                    wr.invalidate_floor_chunk(
                                        entry.pos.0.div_euclid(8),
                                        entry.pos.1.div_euclid(8),
                                        entry.pos.2,
                                        dirty::LIGHTING,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            DeviceEvent::ChatUpdate { history_range } => {
                let chat = match &target {
                    EventTarget::World(id) => {
                        state.borrow().world_devices.get(id).map(|e| e.chat.clone())
                    }
                    EventTarget::Vehicle { id, .. } => state
                        .borrow()
                        .vehicle_devices
                        .get(id)
                        .map(|e| e.chat.clone()),
                    EventTarget::Item { .. } => None,
                };
                if let Some(chat) = chat {
                    let mut c = chat.borrow_mut();
                    c.set_history_range(history_range);
                    c.update();
                }
            }
            DeviceEvent::TvOverlay(opt_idx) => {
                if let EventTarget::World(id) = &target {
                    let info = state.borrow().world_devices.get(id).and_then(|e| {
                        let obj = e.object.upgrade()?;
                        let name = opt_idx.and_then(|i| e.screen_sprites.get(i).cloned());
                        Some((obj, e.pos, name))
                    });
                    if let Some((obj, pos, sprite_name)) = info {
                        if let Ok(mut o) = obj.try_borrow_mut() {
                            o.overlay_sprite = sprite_name.map(|s| OverlaySprite {
                                name: SpriteName::Str(s),
                                color: None,
                            });
                        }
                        if let Ok(mut r) = render.try_borrow_mut() {
                            if let Some(wr) = r.as_mut() {
                                wr.invalidate_floor_chunk(
                                    pos.0.div_euclid(8),
                                    pos.1.div_euclid(8),
                                    pos.2,
                                    dirty::OBJECT_MODIFY,
                                );
                            }
                        }
                    }
                }
            }
            DeviceEvent::PlayerListensChannel { .. }
            | DeviceEvent::UpdateEquippedRadioFreq
            | DeviceEvent::UpdateGenerator
            | DeviceEvent::Transmit { .. } => {}
        }
    }
}

/// [`RadioWorld`] over the live engine singletons (`GameTime`, `Core`,
/// `IsoPlayer`, `WorldRadioState`).
struct EngineRadioWorld<'a> {
    state: Rc<RefCell<WorldRadioState>>,
    emitters: Rc<RefCell<RadioEmitterPool>>,
    sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
    lua: Option<&'a mut LuaManager>,
}

impl<'a> EngineRadioWorld<'a> {
    fn new(
        state: Rc<RefCell<WorldRadioState>>,
        emitters: Rc<RefCell<RadioEmitterPool>>,
        sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
        lua: Option<&'a mut LuaManager>,
    ) -> Self {
        Self {
            state,
            emitters,
            sprites,
            lua,
        }
    }
}

impl RadioWorld for EngineRadioWorld<'_> {
    fn hour(&self) -> i32 {
        pz_lua_api::bind_climate::shared_game_time().borrow().hour()
    }

    fn nights_survived(&self) -> i32 {
        pz_lua_api::bind_climate::shared_game_time()
            .borrow()
            .nights_survived
    }

    fn multiplier(&self) -> f32 {
        let gt = pz_lua_api::bind_climate::shared_game_time();
        let ctx = pz_lua_api::bind_climate::LuaClimateContext::new();
        let res = gt.borrow().get_multiplier(&ctx);
        res
    }

    fn time_since_apo(&self) -> i32 {
        self.state.borrow().time_since_apo
    }

    fn core_game_mode(&self) -> String {
        let core = pz_lua_api::bind_core::shared_core();
        let mode = core
            .borrow()
            .game_mode_name()
            .unwrap_or_default()
            .to_string();
        mode
    }

    fn weather_interference(&self) -> f32 {
        pz_lua_api::bind_climate::shared_game_time()
            .borrow()
            .climate
            .get_weather_interference()
    }

    fn current_save_dir(&self) -> Option<PathBuf> {
        let st = self.state.borrow();
        (!st.no_save).then(|| st.save_dir.clone())
    }

    fn num_players(&self) -> usize {
        pz_lua_api::bind_players::num_players().max(0) as usize
    }

    fn player(&self, index: usize) -> Option<PlayerView> {
        let p = pz_lua_api::bind_players::player(index)?;
        let (x, y, z, last_spoken_line, speak_colour) = {
            let c = p.borrow();
            (
                c.vars.x,
                c.vars.y,
                c.vars.z,
                c.chat.last_spoken_line.clone(),
                [c.speak_colour[0], c.speak_colour[1], c.speak_colour[2]],
            )
        };
        let equipped_radio = character_equipped_radio(&p).and_then(|item| {
            let id = self.state.borrow_mut().id_for_item(&item);
            let b = item.borrow();
            match &b.kind {
                ItemKind::Radio(_, d) => Some((id, device_view(d, [x, y, z], false))),
                _ => None,
            }
        });
        Some(PlayerView {
            x,
            y,
            last_spoken_line,
            speak_colour,
            equipped_radio,
        })
    }

    fn local_player_y(&self) -> Option<f32> {
        pz_lua_api::bind_players::player_instance().map(|p| p.borrow().vars.y)
    }

    fn device(&self, id: DeviceId) -> Option<DeviceView> {
        let st = self.state.borrow();
        if let Some(entry) = st.world_devices.get(&id) {
            let obj = entry.object.upgrade()?;
            let obj = obj.try_borrow().ok()?;
            let d = obj.kind.device_data()?;
            return Some(device_view(
                d,
                [entry.pos.0 as f32, entry.pos.1 as f32, entry.pos.2 as f32],
                d.runtime.wave.has_player_in_range,
            ));
        }
        if let Some(entry) = st.vehicle_devices.get(&id) {
            let part = entry.part.upgrade()?;
            let pst = part.st();
            let d = pst.device_data.as_ref()?;
            let v = part.vehicle();
            return Some(device_view(
                d,
                [v.x(), v.y(), v.z()],
                pst.has_player_in_range,
            ));
        }
        None
    }

    fn add_device_text(
        &mut self,
        device: DeviceId,
        listener_index: Option<usize>,
        text: &str,
        rgb: [f32; 3],
        guid: Option<&str>,
        codes: Option<&str>,
        distance: i32,
    ) {
        // World object device (`IsoRadio` / `IsoTelevision`).
        let world_entry = self.state.borrow().world_devices.get(&device).and_then(|e| {
            Some((e.object.upgrade()?, e.pos))
        });
        if let Some((obj, pos)) = world_entry {
            let power_cfg = self.state.borrow().power;
            let mut env = EngineDeviceEnv::for_world_object(
                pos,
                self.sprites.as_ref(),
                power_cfg,
                self.emitters.clone(),
                Some(device),
            );
            let listener = listener_index.map(|i| {
                pz_lua_api::bind_players::player(i).map(|p| Listener {
                    local: true,
                    deaf: player_is_deaf(&p),
                    equipped_parent: false,
                })
            });
            if let Ok(mut o) = obj.try_borrow_mut() {
                if let Some(d) = kind_device_data_mut(&mut o.kind) {
                    deliver_line(d, &mut env, listener, text, rgb, guid, codes, distance);
                }
            }
            if !env.events.is_empty() {
                self.state
                    .borrow_mut()
                    .deferred_device_events
                    .push((EventTarget::World(device), env.events));
            }
            return;
        }
        // Equipped radio inventory item.
        let item_ref = self
            .state
            .borrow()
            .item_devices
            .get(&device)
            .and_then(Weak::upgrade);
        if let Some(item) = item_ref {
            let player = listener_index
                .and_then(pz_lua_api::bind_players::player)
                .or_else(pz_lua_api::bind_players::player_instance);
            let mut env = EngineDeviceEnv::for_item(player.as_ref(), self.emitters.clone());
            let pos = env.parent_pos;
            let listener = listener_index.map(|i| {
                pz_lua_api::bind_players::player(i).map(|p| {
                    let equipped = character_equipped_radio(&p)
                        .is_some_and(|eq| Rc::ptr_eq(&eq, &item));
                    Listener {
                        local: true,
                        deaf: player_is_deaf(&p),
                        equipped_parent: equipped,
                    }
                })
            });
            if let Ok(mut it) = item.try_borrow_mut() {
                if let ItemKind::Radio(_, d) = &mut it.kind {
                    deliver_line(d, &mut env, listener, text, rgb, guid, codes, distance);
                }
            }
            if !env.events.is_empty() {
                self.state.borrow_mut().deferred_device_events.push((
                    EventTarget::Item {
                        item,
                        player,
                        pos,
                    },
                    env.events,
                ));
            }
            return;
        }
        // Vehicle signal part (`VehiclePart` with `DeviceData`).
        let part_ref = self
            .state
            .borrow()
            .vehicle_devices
            .get(&device)
            .and_then(|e| e.part.upgrade());
        if let Some(part) = part_ref {
            let v = part.vehicle();
            let pos = [v.x(), v.y(), v.z()];
            let mut env = EngineDeviceEnv::for_vehicle_part(
                &v,
                &part,
                self.emitters.clone(),
                Some(device),
            );
            let listener = listener_index.map(|i| {
                pz_lua_api::bind_players::player(i).map(|p| Listener {
                    local: true,
                    deaf: player_is_deaf(&p),
                    equipped_parent: false,
                })
            });
            let dd_opt = part.st_mut().device_data.take();
            if let Some(mut dd) = dd_opt {
                deliver_line(&mut dd, &mut env, listener, text, rgb, guid, codes, distance);
                part.st_mut().device_data = Some(dd);
            }
            if !env.events.is_empty() {
                self.state.borrow_mut().deferred_device_events.push((
                    EventTarget::Vehicle {
                        id: device,
                        pos,
                        device_lua: pz_vehicles::lua::part_value(&part),
                    },
                    env.events,
                ));
            }
        }
    }

    fn on_load_radio_scripts(&mut self, radio: &mut ZomboidRadio, new_game: bool) {
        let Some(lua) = self.lua.as_deref_mut() else {
            return;
        };
        bind_radio::with_scoped_radio(radio, || {
            lua.trigger_event(
                "OnLoadRadioScripts",
                &[bind_radio::script_manager_value(), Value::Bool(new_game)],
            );
        });
    }

    fn on_init_recorded_media(&mut self, media: &mut RecordedMedia) {
        let Some(lua) = self.lua.as_deref_mut() else {
            return;
        };
        bind_radio::with_scoped_media(media, || {
            lua.trigger_event("OnInitRecordedMedia", &[bind_radio::recorded_media_value()]);
        });
    }
}

/// [`DeviceEnv`] returned to Lua (`bind_radio`'s `RadioLuaHooks::device_env`).
/// Flushes collected events on drop (so every Lua `DeviceData` method call
/// applies its sound/light/overlay/chat side effects cleanly).
struct LuaCallbackDeviceEnv {
    inner: EngineDeviceEnv,
    target: EventTarget,
    state: Rc<RefCell<WorldRadioState>>,
    render: Rc<RefCell<Option<WorldRender>>>,
}

impl Drop for LuaCallbackDeviceEnv {
    fn drop(&mut self) {
        let events = std::mem::take(&mut self.inner.events);
        if !events.is_empty() {
            // Apply PlayerListensChannel straight to the installed radio if not
            // already borrowed, then forward the remaining side effects.
            let listens: Vec<(i32, bool)> = events
                .iter()
                .filter_map(|e| match *e {
                    DeviceEvent::PlayerListensChannel { channel, is_tv } => Some((channel, is_tv)),
                    _ => None,
                })
                .collect();
            if !listens.is_empty() {
                let mut w = EngineRadioWorld::new(
                    self.state.clone(),
                    self.inner.audio.pool.clone(),
                    None,
                    None,
                );
                bind_radio::with_radio(|r| {
                    WithRadio::apply_listens(&listens, r, &mut w);
                });
            }
            apply_device_events(self.target.clone(), events, &self.state, &self.render);
        }
    }
}

impl DeviceEnv for LuaCallbackDeviceEnv {
    fn net(&self) -> NetMode {
        self.inner.net()
    }
    fn minutes_stamp(&self) -> i64 {
        self.inner.minutes_stamp()
    }
    fn multiplier(&self) -> f32 {
        self.inner.multiplier()
    }
    fn parent(&self) -> ParentKind {
        self.inner.parent()
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        self.inner.next_int(bound)
    }
    fn next_float(&mut self) -> f32 {
        self.inner.next_float()
    }
    fn mains_power(&self) -> MainsPower {
        self.inner.mains_power()
    }
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.inner.channel_list(category)
    }
    fn media_exists(&self, index: i16) -> bool {
        self.inner.media_exists(index)
    }
    fn media_line(&self, index: i16, line: i32) -> Option<MediaLine> {
        self.inner.media_line(index, line)
    }
    fn iso_object(&self) -> Option<IsoObjectPos> {
        self.inner.iso_object()
    }
    fn closest_listener_distance_sq(&self, x: f32, y: f32, z: f32) -> f32 {
        self.inner.closest_listener_distance_sq(x, y, z)
    }
    fn parent_pos(&self) -> [f32; 3] {
        self.inner.parent_pos()
    }
    fn has_sound_source(&self) -> bool {
        self.inner.has_sound_source()
    }
    fn in_room(&self) -> bool {
        self.inner.in_room()
    }
    fn item_player_deaf(&self) -> Option<bool> {
        self.inner.item_player_deaf()
    }
    fn event(&mut self, e: DeviceEvent) {
        self.inner.event(e);
    }
    fn audio(&mut self) -> Option<&mut dyn DeviceAudio> {
        self.inner.audio()
    }
}

/// [`RadioLuaHooks`] connecting Lua `DeviceData` / `RecordedMedia` calls to
/// the live engine world.
struct EngineRadioLuaHooks {
    radio: Weak<RefCell<ZomboidRadio>>,
    state: Rc<RefCell<WorldRadioState>>,
    emitters: Rc<RefCell<RadioEmitterPool>>,
    sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
    render: Rc<RefCell<Option<WorldRender>>>,
}

impl RadioLuaHooks for EngineRadioLuaHooks {
    fn device_env(&self, owner: &DeviceOwner) -> Option<Box<dyn DeviceEnv>> {
        match owner {
            DeviceOwner::World(wd) => {
                let obj = wd.world_object()?;
                let key = Rc::as_ptr(&obj) as usize;
                let (id, pos, power_cfg) = {
                    let st = self.state.borrow();
                    let id = st.object_to_id.get(&key).copied();
                    let pos = id
                        .and_then(|id| st.world_devices.get(&id).map(|e| e.pos))
                        .unwrap_or((0, 0, 0));
                    (id, pos, st.power)
                };
                let inner = EngineDeviceEnv::for_world_object(
                    pos,
                    self.sprites.as_ref(),
                    power_cfg,
                    self.emitters.clone(),
                    id,
                );
                Some(Box::new(LuaCallbackDeviceEnv {
                    inner,
                    target: EventTarget::World(id.unwrap_or(DeviceId(0))),
                    state: self.state.clone(),
                    render: self.render.clone(),
                }))
            }
            DeviceOwner::Item(item) => {
                let player = radio_local_players()
                    .into_iter()
                    .flatten()
                    .find(|p| {
                        character_equipped_radio(p)
                            .is_some_and(|eq| Rc::ptr_eq(&eq, item))
                    })
                    .or_else(pz_lua_api::bind_players::player_instance);
                let inner = EngineDeviceEnv::for_item(player.as_ref(), self.emitters.clone());
                let pos = inner.parent_pos;
                Some(Box::new(LuaCallbackDeviceEnv {
                    inner,
                    target: EventTarget::Item {
                        item: item.clone(),
                        player,
                        pos,
                    },
                    state: self.state.clone(),
                    render: self.render.clone(),
                }))
            }
            DeviceOwner::Part(part) => {
                let v = part.vehicle();
                let pos = [v.x(), v.y(), v.z()];
                let id = self
                    .radio
                    .upgrade()
                    .and_then(|r| ensure_vehicle_part_registered(&r, &self.state, part));
                let inner = EngineDeviceEnv::for_vehicle_part(
                    &v,
                    part,
                    self.emitters.clone(),
                    id,
                );
                Some(Box::new(LuaCallbackDeviceEnv {
                    inner,
                    target: EventTarget::Vehicle {
                        id: id.unwrap_or(DeviceId(0)),
                        pos,
                        device_lua: bind_radio::device_owner_lua_value(owner),
                    },
                    state: self.state.clone(),
                    render: self.render.clone(),
                }))
            }
        }
    }

    fn can_player_remote_interact(&self, owner: &DeviceOwner, character: &Value) -> bool {
        let DeviceOwner::World(wd) = owner else {
            return false;
        };
        let Some(obj) = wd.world_object() else {
            return false;
        };
        let Some(chr) = character_from_value(character) else {
            return false;
        };
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let has_remote = {
            let c = chr.borrow();
            let check = |id: Option<i32>| -> bool {
                id.and_then(|id| c.items.item(id))
                    .is_some_and(|it| has_tag(&sm, &it.borrow(), "TV_REMOTE") || it.borrow().has_tag("TVRemote"))
            };
            check(c.items.primary_hand_item) || check(c.items.secondary_hand_item)
        };
        if !has_remote {
            return false;
        }
        let key = Rc::as_ptr(&obj) as usize;
        let tv_pos = {
            let st = self.state.borrow();
            st.object_to_id
                .get(&key)
                .and_then(|id| st.world_devices.get(id))
                .map(|e| e.pos)
        };
        let Some((tx, ty, tz)) = tv_pos else {
            return false;
        };
        let c = chr.borrow();
        let dx = c.vars.x - tx as f32;
        let dy = c.vars.y - ty as f32;
        let dz = c.vars.z as i32 - tz;
        dz == 0 && (dx * dx + dy * dy) <= 100.0
    }

    fn knows_media_line(&self, player: &Value, guid: &str) -> bool {
        let Some(chr) = character_from_value(player) else {
            return false;
        };
        let known = chr.borrow().known_media_lines.contains_key(guid);
        known
    }
}

fn character_from_value(v: &Value) -> Option<LuaGameCharacterRef> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<pz_lua_api::bind_character_sim::CharacterObject>()
            .map(|c| c.0.clone()),
        _ => None,
    }
}

/// [`VehicleDevices`] bridge connecting `pz-vehicles` (`VehiclePart.updateSignalDevice`)
/// to the live [`WorldRadio`] subsystem.
struct EngineVehicleDevices {
    radio: Weak<RefCell<ZomboidRadio>>,
    state: Rc<RefCell<WorldRadioState>>,
    emitters: Rc<RefCell<RadioEmitterPool>>,
    render: Rc<RefCell<Option<WorldRender>>>,
}

impl VehicleDevices for EngineVehicleDevices {
    fn device_env(
        &self,
        vehicle: &BaseVehicle,
        part: &PartRef,
    ) -> Option<Box<dyn DeviceEnv>> {
        let radio = self.radio.upgrade()?;
        let id = ensure_vehicle_part_registered(&radio, &self.state, part)?;
        let pos = [vehicle.x(), vehicle.y(), vehicle.z()];
        let inner = EngineDeviceEnv::for_vehicle_part(
            vehicle,
            part,
            self.emitters.clone(),
            Some(id),
        );
        Some(Box::new(LuaCallbackDeviceEnv {
            inner,
            target: EventTarget::Vehicle {
                id,
                pos,
                device_lua: pz_vehicles::lua::part_value(part),
            },
            state: self.state.clone(),
            render: self.render.clone(),
        }))
    }

    fn local_players(&self) -> Vec<Option<(f32, f32)>> {
        let n = (pz_lua_api::bind_players::num_players().max(0) as usize).min(MAX_PLAYERS);
        let count = if n == 0 && pz_lua_api::bind_players::player(0).is_some() {
            1
        } else {
            n
        };
        (0..count)
            .map(|i| {
                pz_lua_api::bind_players::player(i).and_then(|p| {
                    let c = p.borrow();
                    (!c.is_dead()).then_some((c.vars.x, c.vars.y))
                })
            })
            .collect()
    }
}

/// No-op [`VehicleDevices`] installed when the world radio is shut down.
struct NoEngineVehicleDevices;
impl VehicleDevices for NoEngineVehicleDevices {}

/// [`ObjectLoadHooks`] supplying `IsoWaveSignal.cloneDeviceDataFromItem` and
/// `ZomboidRadio.GetChannelList` during chunk loading (`with_stream`).
pub struct RadioObjectLoadHooks {
    radio: Option<Rc<RefCell<ZomboidRadio>>>,
    cache: Option<Rc<RefCell<HashMap<String, DeviceData>>>>,
    sm: Rc<pz_scripts::ScriptManager>,
}

impl RadioObjectLoadHooks {
    pub fn new(world_radio: Option<&WorldRadio>) -> Self {
        Self {
            radio: world_radio.map(|r| r.radio.clone()),
            cache: world_radio.map(|r| r.device_data_cache.clone()),
            sm: pz_lua_api::bind_registry::shared_script_manager(),
        }
    }
}

struct RadioChannelListsRef<'a>(&'a ZomboidRadio);
impl ChannelLists for RadioChannelListsRef<'_> {
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.0.channel_list(category)
    }
}

impl ObjectLoadHooks for RadioObjectLoadHooks {
    fn device_data_from_item(
        &mut self,
        item_full: &str,
        rand: &mut dyn GameRand,
    ) -> Option<DeviceData> {
        if let Some(cache) = &self.cache {
            if let Some(cached) = cache.borrow().get(item_full) {
                return Some(cached.clone());
            }
        }
        let script = self
            .sm
            .find_item(item_full, true)?
            .downcast::<pz_scripts::objects::items::Item>()?;
        let template = ItemTemplate::from_script_ref(&self.sm, &script)?;
        if template.class != pz_inventory::ItemClass::Radio {
            return None;
        }
        let radio_script = &template.radio;
        let mut data = DeviceData::default();
        let mut next = |b: i32| rand.next(b);
        if let Some(radio) = &self.radio {
            if let Ok(radio) = radio.try_borrow() {
                let lists = RadioChannelListsRef(&radio);
                data.instance_from_script(
                    Some(&template.display_name),
                    radio_script,
                    &lists,
                    &mut next,
                );
            } else {
                data.instance_from_script(
                    Some(&template.display_name),
                    radio_script,
                    &pz_inventory::device::NoChannels,
                    &mut next,
                );
            }
        } else {
            data.instance_from_script(
                Some(&template.display_name),
                radio_script,
                &pz_inventory::device::NoChannels,
                &mut next,
            );
        }
        if let Some(cache) = &self.cache {
            cache
                .borrow_mut()
                .insert(item_full.to_string(), data.clone());
        }
        Some(data)
    }

    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.radio
            .as_ref()?
            .try_borrow()
            .ok()?
            .channel_list(category)
    }
}

/// The engine's live radio subsystem owned by [`IsoWorld`].
pub struct WorldRadio {
    pub radio: Rc<RefCell<ZomboidRadio>>,
    state: Rc<RefCell<WorldRadioState>>,
    emitters: Rc<RefCell<RadioEmitterPool>>,
    /// `IsoWaveSignal.deviceDataCache` (cleared by `IsoWaveSignal.Reset()`).
    device_data_cache: Rc<RefCell<HashMap<String, DeviceData>>>,
}

impl WorldRadio {
    /// `ZomboidRadio.getInstance().Init(savedWorldVersion)`.
    #[allow(clippy::too_many_arguments)]
    pub fn init(
        game_dir: &Path,
        save_dir: PathBuf,
        saved_world_version: i32,
        no_save: bool,
        lua: &mut LuaManager,
        sb: &Rc<RefCell<pz_sandbox::SandboxOptions>>,
        sprites: Option<Rc<RefCell<IsoSpriteManager>>>,
        render: Rc<RefCell<Option<WorldRender>>>,
    ) -> Result<Self, String> {
        let hour = pz_lua_api::bind_climate::shared_game_time().borrow().hour();
        let (time_since_apo, power) = {
            let s = sb.borrow();
            let time_since_apo = opt_int(&s, "TimeSinceApo").max(1);
            (
                time_since_apo,
                RadioPowerConfig {
                    time_since_apo,
                    elec_shut_modifier: opt_int(&s, "ElecShutModifier"),
                    allow_exterior_generator: opt_bool(&s, "AllowExteriorGenerator"),
                    generator_tile_range: opt_int(&s, "GeneratorTileRange").max(1),
                    generator_vertical_power_range: opt_int(&s, "GeneratorVerticalPowerRange").max(1),
                    hearing: opt_int(&s, "ZombieLore.Hearing").max(1),
                },
            )
        };
        let state = Rc::new(RefCell::new(WorldRadioState::new(
            save_dir,
            no_save,
            time_since_apo,
            power,
        )));
        let emitters = Rc::new(RefCell::new(RadioEmitterPool::default()));
        let device_data_cache = Rc::new(RefCell::new(HashMap::new()));

        bind_radio::install_text(Rc::new(EngineRadioText));
        bind_radio::install_hooks(Rc::new(EngineRadioLuaHooks {
            radio: Weak::new(),
            state: state.clone(),
            emitters: emitters.clone(),
            sprites: sprites.clone(),
            render: render.clone(),
        }));
        bind_radio::radio_api_created();

        let source = {
            let fs = lua.fs.borrow();
            let core = pz_lua_api::bind_core::shared_core();
            let src = radio_source(game_dir, &fs, &core.borrow());
            src
        };

        let mut radio = ZomboidRadio::new(hour);
        radio.set_game_mode(GameMode::SinglePlayer);
        radio.louisville_obfuscation = LOUISVILLE_OBFUSCATION.with(|c| c.get());

        let init_result = {
            let mut world = EngineRadioWorld::new(
                state.clone(),
                emitters.clone(),
                sprites.clone(),
                Some(lua),
            );
            let mut rng = |bound: i32| pz_core::random::next(bound);
            radio.init(
                saved_world_version,
                &source,
                &EngineRadioText,
                &mut world,
                &mut rng,
            )
        };

        LOUISVILLE_OBFUSCATION.with(|c| c.set(radio.louisville_obfuscation));
        let radio = Rc::new(RefCell::new(radio));
        bind_radio::install_radio(Some(radio.clone()));
        bind_radio::install_hooks(Rc::new(EngineRadioLuaHooks {
            radio: Rc::downgrade(&radio),
            state: state.clone(),
            emitters: emitters.clone(),
            sprites: sprites.clone(),
            render: render.clone(),
        }));
        pz_vehicles::env::set_devices(Rc::new(EngineVehicleDevices {
            radio: Rc::downgrade(&radio),
            state: state.clone(),
            emitters: emitters.clone(),
            render,
        }));

        // Install the ten-minute `GameTime` hook (`ZomboidRadio.UpdateScripts`).
        let tick_radio = Rc::downgrade(&radio);
        let tick_state = Rc::downgrade(&state);
        let tick_emitters = emitters.clone();
        let tick_sprites = sprites;
        bind_radio::install_update_scripts_hook(Some(Rc::new(move |hour, minutes| {
            let (Some(radio), Some(state)) = (tick_radio.upgrade(), tick_state.upgrade()) else {
                return;
            };
            let Ok(mut r) = radio.try_borrow_mut() else {
                return;
            };
            let mut world = EngineRadioWorld::new(
                state,
                tick_emitters.clone(),
                tick_sprites.clone(),
                None,
            );
            let mut rng = |bound: i32| pz_core::random::next(bound);
            r.update_scripts(hour, minutes, &mut world, &mut rng);
            LOUISVILLE_OBFUSCATION.with(|c| c.set(r.louisville_obfuscation));
        })));

        let wr = Self {
            radio,
            state,
            emitters,
            device_data_cache,
        };
        if let Err(e) = init_result {
            bind_radio::reset_world_bindings();
            pz_vehicles::env::set_devices(Rc::new(NoEngineVehicleDevices));
            return Err(e.0);
        }
        Ok(wr)
    }

    /// Translated display names of all registered `RecordedMedia`, keyed by
    /// media index (for `WorldDictionary` / `ScriptItems.recorded_media`).
    pub fn recorded_media_names(&self) -> HashMap<i16, String> {
        let radio = self.radio.borrow();
        let rm = radio.recorded_media();
        let mut out = HashMap::new();
        for (i, id) in rm.indexes().iter().enumerate() {
            if let Some(m) = rm.media_data(id) {
                out.insert(i as i16, m.translated_item_display_name(&EngineRadioText));
            }
        }
        out
    }

    /// Snapshot of `ZomboidRadio`'s channel registry for `ScriptItems.channels`.
    pub fn channel_lists(&self) -> Box<dyn ChannelLists> {
        Box::new(self.radio.borrow().channel_names().clone())
    }

    /// `RecordedMedia.handleLegacyListenedLines(player)`: on a loaded game,
    /// hands legacy v1 listened lines to player 0; on a new game, clears them.
    pub fn handle_legacy_listened_lines(&self, player: Option<&LuaGameCharacterRef>) {
        let mut radio = self.radio.borrow_mut();
        match player {
            Some(p) => {
                let mut chr = p.borrow_mut();
                let mut learn = |guid: &str| {
                    chr.known_media_lines.insert(guid.to_string(), ());
                };
                radio
                    .recorded_media_mut()
                    .handle_legacy_listened_lines(Some(&mut learn));
            }
            None => radio.recorded_media_mut().handle_legacy_listened_lines(None),
        }
    }

    /// `IsoWaveSignal.addToWorld()` -> `ZomboidRadio.RegisterDevice(this)`.
    pub fn register_world_object(&self, object: &IsoObjectRef, pos: (i32, i32, i32)) {
        let key = Rc::as_ptr(object) as usize;
        if self.state.borrow().object_to_id.contains_key(&key) {
            return;
        }
        let Ok(obj) = object.try_borrow() else {
            return;
        };
        let (is_tv, two_way) = match &obj.kind {
            pz_world::ObjectKind::Radio(d) => (false, d.two_way),
            pz_world::ObjectKind::Television(d) => (true, d.two_way),
            _ => return,
        };
        drop(obj);

        let chat_owner = Rc::new(WorldDeviceChatOwner { pos, is_tv });
        let weak_owner: Weak<dyn ChatElementOwner> =
            Rc::downgrade(&chat_owner) as Weak<dyn ChatElementOwner>;
        let chat = Rc::new(RefCell::new(ChatElement::new(
            weak_owner,
            5,
            Some("device"),
        )));

        let id = {
            let mut st = self.state.borrow_mut();
            let id = st.alloc_id();
            st.object_to_id.insert(key, id);
            st.world_devices.insert(
                id,
                WorldDeviceEntry {
                    object: Rc::downgrade(object),
                    pos,
                    is_tv,
                    two_way,
                    cached_sprite: None,
                    screen_sprites: Vec::new(),
                    lamppost_key: None,
                    _chat_owner: chat_owner,
                    chat,
                },
            );
            id
        };
        self.radio.borrow_mut().register_device(id, two_way);
    }

    /// `IsoWaveSignal.removeFromWorld()` -> `ZomboidRadio.UnRegisterDevice`
    /// plus light and emitter cleanup.
    pub fn unregister_world_object(
        &self,
        object: &IsoObjectRef,
        render: &Rc<RefCell<Option<WorldRender>>>,
    ) {
        let key = Rc::as_ptr(object) as usize;
        let removed = {
            let mut st = self.state.borrow_mut();
            st.object_to_id
                .remove(&key)
                .and_then(|id| st.world_devices.remove(&id).map(|e| (id, e)))
        };
        let Some((id, entry)) = removed else {
            return;
        };
        self.cleanup_world_entry(id, entry, render);
    }

    /// Unregisters every world radio/TV residing in chunk `(wx, wy)` on unload.
    pub fn unregister_chunk(
        &self,
        wx: i32,
        wy: i32,
        render: &Rc<RefCell<Option<WorldRender>>>,
    ) {
        let to_remove: Vec<DeviceId> = {
            let st = self.state.borrow();
            st.world_devices
                .iter()
                .filter(|(_, e)| {
                    e.pos.0.div_euclid(8) == wx && e.pos.1.div_euclid(8) == wy
                        || e.object.strong_count() == 0
                })
                .map(|(&id, _)| id)
                .collect()
        };
        for id in to_remove {
            let entry = {
                let mut st = self.state.borrow_mut();
                let entry = st.world_devices.remove(&id);
                st.object_to_id.retain(|_, v| *v != id);
                entry
            };
            if let Some(entry) = entry {
                self.cleanup_world_entry(id, entry, render);
            }
        }
    }

    fn cleanup_world_entry(
        &self,
        id: DeviceId,
        entry: WorldDeviceEntry,
        render: &Rc<RefCell<Option<WorldRender>>>,
    ) {
        self.radio
            .borrow_mut()
            .unregister_device(id, entry.two_way);
        if let Some(key) = entry.lamppost_key {
            if let Ok(mut r) = render.try_borrow_mut() {
                if let Some(wr) = r.as_mut() {
                    wr.lighting.remove_lamppost(key);
                }
            }
        }
        if let Some(obj) = entry.object.upgrade() {
            let power_cfg = self.state.borrow().power;
            let mut env = EngineDeviceEnv::for_world_object(
                entry.pos,
                None,
                power_cfg,
                self.emitters.clone(),
                Some(id),
            );
            if let Ok(mut o) = obj.try_borrow_mut() {
                if let Some(d) = kind_device_data_mut(&mut o.kind) {
                    d.remove_light(&mut env);
                    d.clean_sounds_and_emitter(&mut env);
                }
            }
        }
        // Even if the `IsoObject` was already dropped before unregister/stale
        // cleanup ran, release any emitter owned by `id` so audio never leaks.
        self.emitters.borrow_mut().return_for_device(id);
    }

    /// Synchronizes registered vehicle signal parts against live
    /// `pz_vehicles::manager::cell_vehicles()`: registers any part with
    /// `DeviceData` and unregisters/cleans up any removed, unloaded, or
    /// uninstalled part.
    pub fn sync_vehicle_devices(&self) {
        let mut live_ids = HashSet::new();
        for vehicle in pz_vehicles::manager::cell_vehicles() {
            for part in vehicle.parts() {
                if part.st().device_data.is_some() {
                    if let Some(id) =
                        ensure_vehicle_part_registered(&self.radio, &self.state, &part)
                    {
                        live_ids.insert(id);
                    }
                }
            }
        }
        let stale: Vec<DeviceId> = {
            let st = self.state.borrow();
            st.vehicle_devices
                .keys()
                .copied()
                .filter(|id| !live_ids.contains(id))
                .collect()
        };
        for id in stale {
            let entry = {
                let mut st = self.state.borrow_mut();
                let entry = st.vehicle_devices.remove(&id);
                st.part_to_id.retain(|_, v| *v != id);
                entry
            };
            if let Some(entry) = entry {
                self.radio
                    .borrow_mut()
                    .unregister_device(id, entry.two_way);
                if let Some(part) = entry.part.upgrade() {
                    let v = part.vehicle();
                    let mut env = EngineDeviceEnv::for_vehicle_part(
                        &v,
                        &part,
                        self.emitters.clone(),
                        Some(id),
                    );
                    let dd_opt = part.st_mut().device_data.take();
                    if let Some(mut dd) = dd_opt {
                        dd.clean_sounds_and_emitter(&mut env);
                        part.st_mut().device_data = Some(dd);
                    }
                }
                self.emitters.borrow_mut().return_for_device(id);
            }
        }
    }

    /// `IsoWaveSignal.update()` for every live world radio/TV, plus
    /// `Radio.update()` for each local player's equipped radio item.
    pub fn update_devices(
        &self,
        lua: &mut LuaManager,
        sprites: Option<&Rc<RefCell<IsoSpriteManager>>>,
        render: &Rc<RefCell<Option<WorldRender>>>,
    ) {
        self.sync_vehicle_devices();
        {
            let st = self.state.borrow();
            for entry in st.vehicle_devices.values() {
                entry.chat.borrow_mut().update();
            }
        }
        let player_pos = pz_lua_api::bind_players::player_instance().map(|p| {
            let c = p.borrow();
            (c.vars.x, c.vars.y)
        });
        let power_cfg = self.state.borrow().power;

        // Snapshot live world devices so we never hold `state` across updates.
        let ids: Vec<DeviceId> = self.state.borrow().world_devices.keys().copied().collect();
        let mut stale = Vec::new();
        let mut all_listens: Vec<(i32, bool)> = Vec::new();

        for id in ids {
            let snapshot = {
                let st = self.state.borrow();
                st.world_devices.get(&id).map(|e| {
                    e.chat.borrow_mut().update();
                    (
                        e.object.upgrade(),
                        e.pos,
                        e.is_tv,
                        e.cached_sprite,
                        e.screen_sprites.len(),
                        e.chat.borrow().get_has_chat_to_display(),
                    )
                })
            };
            let Some((obj_opt, pos, is_tv, cached_sprite, mut screen_count, has_chat)) = snapshot
            else {
                continue;
            };
            let Some(obj) = obj_opt else {
                stale.push(id);
                continue;
            };

            let tv_update = if is_tv {
                let current_sprite = obj.try_borrow().ok().and_then(|o| o.sprite.tile_projection());
                let sprite_changed = cached_sprite.is_some() && cached_sprite != current_sprite;
                if cached_sprite != current_sprite {
                    if let (Some(sp_idx), Some(sp_mgr)) = (current_sprite, sprites) {
                        if let Ok(sp_mgr) = sp_mgr.try_borrow() {
                            let names = resolve_tv_screen_sprites(&sp_mgr, sp_idx);
                            screen_count = names.len();
                            if let Some(entry) = self.state.borrow_mut().world_devices.get_mut(&id)
                            {
                                entry.cached_sprite = Some(sp_idx);
                                entry.screen_sprites = names;
                            }
                        }
                    }
                }
                Some(TvUpdate {
                    screen_sprites: screen_count,
                    sprite_changed,
                })
            } else {
                None
            };

            let mut base_env = EngineDeviceEnv::for_world_object(
                pos,
                sprites,
                power_cfg,
                self.emitters.clone(),
                Some(id),
            );
            {
                let radio_guard = self.radio.borrow();
                let mut env = WithRadio::new(&mut base_env, &radio_guard, &EngineRadioText);
                if let Ok(mut o) = obj.try_borrow_mut() {
                    if let Some(d) = kind_device_data_mut(&mut o.kind) {
                        d.update_wave_signal(&mut env, player_pos, tv_update, has_chat);
                    }
                }
                all_listens.extend(env.listens);
            }
            apply_device_events(EventTarget::World(id), base_env.events, &self.state, render);
        }

        for id in stale {
            let entry = {
                let mut st = self.state.borrow_mut();
                let entry = st.world_devices.remove(&id);
                st.object_to_id.retain(|_, v| *v != id);
                entry
            };
            if let Some(entry) = entry {
                self.cleanup_world_entry(id, entry, render);
            }
        }

        // Equipped handheld radios (`IsoGameCharacter.radioEquipedCheck` + `Radio.update`).
        for slot in 0..MAX_PLAYERS {
            let player = pz_lua_api::bind_players::player(slot);
            let current = player.as_ref().and_then(character_equipped_radio);
            let prev = {
                let mut st = self.state.borrow_mut();
                let changed = match (&st.prev_equipped_radios[slot], &current) {
                    (Some(a), Some(b)) => !Rc::ptr_eq(a, b),
                    (None, None) => false,
                    _ => true,
                };
                if changed {
                    std::mem::replace(&mut st.prev_equipped_radios[slot], current.clone())
                } else {
                    None
                }
            };
            if let Some(old_item) = prev {
                let mut base_env =
                    EngineDeviceEnv::for_item(player.as_ref(), self.emitters.clone());
                let pos = base_env.parent_pos;
                {
                    let radio_guard = self.radio.borrow();
                    let mut env = WithRadio::new(&mut base_env, &radio_guard, &EngineRadioText);
                    if let Ok(mut it) = old_item.try_borrow_mut() {
                        if let ItemKind::Radio(_, d) = &mut it.kind {
                            update_radio_item(d, &mut env, false);
                        }
                    }
                }
                apply_device_events(
                    EventTarget::Item {
                        item: old_item,
                        player: player.clone(),
                        pos,
                    },
                    base_env.events,
                    &self.state,
                    render,
                );
            }
            if let Some(item) = current {
                let mut base_env =
                    EngineDeviceEnv::for_item(player.as_ref(), self.emitters.clone());
                let pos = base_env.parent_pos;
                {
                    let radio_guard = self.radio.borrow();
                    let mut env = WithRadio::new(&mut base_env, &radio_guard, &EngineRadioText);
                    if let Ok(mut it) = item.try_borrow_mut() {
                        if let ItemKind::Radio(_, d) = &mut it.kind {
                            update_radio_item(d, &mut env, true);
                        }
                    }
                    all_listens.extend(env.listens);
                }
                apply_device_events(
                    EventTarget::Item { item, player, pos },
                    base_env.events,
                    &self.state,
                    render,
                );
            }
        }

        if !all_listens.is_empty() {
            let mut radio = self.radio.borrow_mut();
            let mut world = EngineRadioWorld::new(
                self.state.clone(),
                self.emitters.clone(),
                sprites.cloned(),
                None,
            );
            WithRadio::apply_listens(&all_listens, &mut radio, &mut world);
        }

        self.flush_pending_lua(lua);
    }

    /// `ZomboidRadio.getInstance().update()` (called each frame from
    /// `IngameState.update`).
    pub fn update_radio(
        &self,
        lua: &mut LuaManager,
        sprites: Option<&Rc<RefCell<IsoSpriteManager>>>,
        render: &Rc<RefCell<Option<WorldRender>>>,
    ) {
        self.sync_vehicle_devices();
        {
            let mut radio = self.radio.borrow_mut();
            let mut world = EngineRadioWorld::new(
                self.state.clone(),
                self.emitters.clone(),
                sprites.cloned(),
                None,
            );
            let mut rng = |bound: i32| pz_core::random::next(bound);
            radio.update(&mut world, &mut rng);
            LOUISVILLE_OBFUSCATION.with(|c| c.set(radio.louisville_obfuscation));
        }

        let deferred = std::mem::take(&mut self.state.borrow_mut().deferred_device_events);
        for (target, events) in deferred {
            apply_device_events(target, events, &self.state, render);
        }
        self.flush_pending_lua(lua);
    }

    /// Fires all queued `OnDeviceText` and chat-flush events cleanly outside
    /// any `ZomboidRadio` / `IsoObject` / `WORLD` borrow.
    fn flush_pending_lua(&self, lua: &mut LuaManager) {
        let pending = std::mem::take(&mut self.state.borrow_mut().pending_lua);
        for ev in pending {
            match ev {
                PendingLuaEvent::OnDeviceText {
                    guid,
                    codes,
                    pos,
                    text,
                    device,
                } => {
                    let (x, y, z) = match pos {
                        Some([x, y, z]) => (x as f64, y as f64, z as f64),
                        None => (-1.0, -1.0, -1.0),
                    };
                    let guid_val = guid.as_deref().map_or(Value::Nil, Value::str);
                    lua.trigger_event(
                        "OnDeviceText",
                        &[
                            guid_val,
                            Value::str(&codes),
                            Value::Num(x),
                            Value::Num(y),
                            Value::Num(z),
                            Value::str(&text),
                            device,
                        ],
                    );
                }
                PendingLuaEvent::FlushPlayerChat(player) => {
                    let _ = flush_chat(&player);
                }
            }
        }
        let _ = pz_lua_api::chat::fire_pending_events(&mut lua.thread);
    }

    /// `ZomboidRadio.getInstance().Save()` (`GameWindow.save`).
    pub fn save(&self) {
        let mut radio = self.radio.borrow_mut();
        let world = EngineRadioWorld::new(self.state.clone(), self.emitters.clone(), None, None);
        radio.save(&world);
    }

    /// Projects each visible world radio/TV that has active dialogue lines and
    /// queues its `ChatElement` for `ChatElement::render_batch(player)`
    /// (`IsoWaveSignal.renderlast`).
    pub fn queue_device_chat(&self, player: usize) {
        let zoom = pz_lua_api::hud::host::with_host(|h| h.camera.zoom(player)).max(0.01);
        let tile_scale = pz_lua_api::bind_core::shared_core()
            .try_borrow()
            .map_or(2, |c| c.tile_scale)
            .max(1) as f32;
        let devices: Vec<([f32; 3], Rc<RefCell<ChatElement>>)> = {
            let st = self.state.borrow();
            let mut out: Vec<([f32; 3], Rc<RefCell<ChatElement>>)> = st
                .world_devices
                .values()
                .filter(|e| e.object.strong_count() > 0 && e.chat.borrow().get_has_chat_to_display())
                .map(|e| {
                    (
                        [e.pos.0 as f32 + 0.5, e.pos.1 as f32 + 0.5, e.pos.2 as f32],
                        e.chat.clone(),
                    )
                })
                .collect();
            for e in st.vehicle_devices.values() {
                if !e.chat.borrow().get_has_chat_to_display() {
                    continue;
                }
                if let Some(part) = e.part.upgrade() {
                    let v = part.vehicle();
                    out.push(([v.x(), v.y(), v.z()], e.chat.clone()));
                }
            }
            out
        };
        for (pos, chat) in devices {
            let (sx, sy) =
                pz_lua_api::hud::screen_iso::iso_to_screen(player, pos[0], pos[1], pos[2]);
            // Offset dialogue above the radio/TV tile (`IsoWaveSignal.renderlast`).
            let y = sy - (32.0 * tile_scale) / zoom;
            ChatElement::render_batched(&chat, player, sx as i32, y as i32, false);
        }
    }

    /// `ZomboidRadio.getInstance().Reset()` and `IsoWaveSignal.Reset()`
    /// (`IngameState.exit`). Preserves `louisvilleObfuscation` across resets.
    pub fn reset(&self, render: &Rc<RefCell<Option<WorldRender>>>) {
        LOUISVILLE_OBFUSCATION.with(|c| c.set(self.radio.borrow().louisville_obfuscation));
        bind_radio::reset_world_bindings();
        pz_vehicles::env::set_devices(Rc::new(NoEngineVehicleDevices));
        self.device_data_cache.borrow_mut().clear();

        // Remove all TV lamppost lights from the renderer.
        let lamppost_keys: Vec<u64> = {
            let mut st = self.state.borrow_mut();
            st.object_to_id.clear();
            st.part_to_id.clear();
            st.vehicle_devices.clear();
            st.item_to_id.clear();
            st.item_devices.clear();
            st.prev_equipped_radios = Default::default();
            st.pending_lua.clear();
            st.deferred_device_events.clear();
            st.world_devices
                .drain()
                .filter_map(|(_, e)| e.lamppost_key)
                .collect()
        };
        if !lamppost_keys.is_empty() {
            if let Ok(mut r) = render.try_borrow_mut() {
                if let Some(wr) = r.as_mut() {
                    for key in lamppost_keys {
                        wr.lighting.remove_lamppost(key);
                    }
                }
            }
        }
        self.emitters.borrow_mut().return_all();
    }
}

impl IsoWorld {
    /// `ZomboidRadio.getInstance().Init(savedWorldVersion)` during
    /// `IsoWorld.init`.
    pub(crate) fn init_radio(
        &mut self,
        lua: &mut LuaManager,
        sb: &Rc<RefCell<pz_sandbox::SandboxOptions>>,
        no_save: bool,
    ) {
        println!("ZomboidRadio.Init() start");
        match WorldRadio::init(
            &self.game_dir,
            self.save_dir.clone(),
            self.saved_world_version,
            no_save,
            lua,
            sb,
            self.sprites.clone(),
            self.render.clone(),
        ) {
            Ok(radio) => self.radio = Some(radio),
            Err(e) => {
                eprintln!("ERROR: ZomboidRadio.Init: {e}");
                self.errors.push(format!("ZomboidRadio.Init: {e}"));
            }
        }
        println!("ZomboidRadio.Init() end");
    }

    /// `RecordedMedia.handleLegacyListenedLines` after player load/creation.
    pub(crate) fn handle_legacy_listened_lines(&self, loaded_player: bool) {
        let Some(radio) = &self.radio else { return };
        if loaded_player {
            let player = pz_lua_api::bind_players::player_instance();
            radio.handle_legacy_listened_lines(player.as_ref());
        } else {
            radio.handle_legacy_listened_lines(None);
        }
    }

    /// `IsoWaveSignal.addToWorld()` handler for chunk object activation.
    pub(crate) fn register_radio_object(&self, object: &IsoObjectRef, pos: (i32, i32, i32)) {
        if let Some(radio) = &self.radio {
            radio.register_world_object(object, pos);
        }
    }

    /// `IsoWaveSignal.removeFromWorld()` for a chunk leaving the cell.
    pub(crate) fn unregister_radio_chunk(&self, wx: i32, wy: i32) {
        if let Some(radio) = &self.radio {
            radio.unregister_chunk(wx, wy, &self.render);
        }
    }

    /// `IsoWaveSignal.update` (all world radios/TVs) and `Radio.update`
    /// (equipped handheld radios), once per cell update.
    pub(crate) fn update_radio_devices(&self, lua: &mut LuaManager) {
        if let Some(radio) = &self.radio {
            radio.update_devices(lua, self.sprites.as_ref(), &self.render);
        }
    }

    /// `ZomboidRadio.getInstance().update()`, once per in-game frame.
    pub fn update_radio(&self, lua: &mut LuaManager) {
        if let Some(radio) = &self.radio {
            radio.update_radio(lua, self.sprites.as_ref(), &self.render);
        }
    }

    /// `ZomboidRadio.getInstance().Save()` during world save.
    pub(crate) fn save_radio(&self) {
        if let Some(radio) = &self.radio {
            radio.save();
        }
    }

    /// Queues visible world radio/TV `ChatElement`s for `ChatElement::render_batch(player)`.
    pub(crate) fn queue_radio_device_chat(&self, player: usize) {
        if let Some(radio) = &self.radio {
            radio.queue_device_chat(player);
        }
    }

    /// `ZomboidRadio.getInstance().Reset()` and `IsoWaveSignal.Reset()`.
    pub fn reset_radio(&mut self) {
        if let Some(radio) = self.radio.take() {
            radio.reset(&self.render);
        } else {
            bind_radio::reset_world_bindings();
            pz_vehicles::env::set_devices(Rc::new(NoEngineVehicleDevices));
        }
    }
}

#[cfg(test)]
mod tests;
