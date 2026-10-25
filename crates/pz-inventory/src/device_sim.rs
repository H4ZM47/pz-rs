//! The device simulation of `DeviceData` (`zombie.radio.devices`): power
//! drain, the on/off and channel logic, the static and talk sounds on the
//! device's emitter, received signals, recorded media playback, the world
//! sounds that attract zombies, and the parent-specific `AddDeviceText`
//! of `IsoWaveSignal`, `Radio` and `VehiclePart`. `IsoWaveSignal.update`
//! and the `IsoTelevision` screen and light are here too, since their
//! runtime state lives with the device.
//!
//! The world is reached through [`DeviceEnv`] (time, RNG, the parent,
//! power, the radio's channel lists and recorded media) and the emitter
//! through [`DeviceAudio`]; every side effect that does not return a value
//! goes out as a [`DeviceEvent`].

use crate::device::{DeviceData, DevicePresets};

/// `ZomboidRadio.staticSounds`.
pub const STATIC_SOUNDS: [&str; 4] = ["<bzzt>", "<fzzt>", "<wzzt>", "<szzt>"];

/// `ZomboidRadio.isStaticSound`.
pub fn is_static_sound(s: &str) -> bool {
    STATIC_SOUNDS.contains(&s)
}

const MINMOD: f32 = 1.5;
const MAXMOD: f32 = 5.0;
const HEADPHONES_FULL_TYPE: &str = "Base.Headphones";
const EARBUDS_FULL_TYPE: &str = "Base.Earbuds";

/// `GameClient.client` / `GameServer.server`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetMode {
    #[default]
    SinglePlayer,
    Client,
    Server,
}

/// What `DeviceData.parent` is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParentKind {
    #[default]
    None,
    /// `Radio` (an inventory item).
    Item,
    /// `IsoWaveSignal` (`IsoRadio` / `IsoTelevision`).
    Iso,
    /// `VehiclePart`.
    Vehicle,
}

/// The `RadioDeviceDataState` packet types of `transmitDeviceDataState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    TurnedOn = 0,
    Channel = 1,
    Battery = 2,
    Power = 3,
    Volume = 4,
    Presets = 5,
    Headphones = 6,
    MediaItem = 7,
    StartMedia = 8,
    StopMedia = 9,
    MediaLine = 10,
}

/// Where a chat line goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatTarget {
    /// The device's own `ChatElement` (`IsoWaveSignal`, `VehiclePart`).
    Device,
    /// The listening player's chat element (an equipped radio item).
    Player,
}

/// A side effect of the device simulation.
#[derive(Clone, Debug, PartialEq)]
pub enum DeviceEvent {
    /// `transmitDeviceDataState` (client) or
    /// `transmitDeviceDataStateServer` (server); only sent in that mode.
    Transmit { state: DeviceState, server: bool },
    /// `ZomboidRadio.PlayerListensChannel(channel, true, isTv)`.
    PlayerListensChannel { channel: i32, is_tv: bool },
    /// `IsoPlayer.updateEquippedRadioFreq()` of the player holding the
    /// radio item.
    UpdateEquippedRadioFreq,
    /// `IsoGenerator.updateGenerator(parent.getSquare())`.
    UpdateGenerator,
    /// `WorldSoundManager.addSound` / `addSoundRepeating` at the device's
    /// sound source (the object, the local player, or the vehicle).
    WorldSound {
        repeating: bool,
        radius: i32,
        volume: i32,
        stress: bool,
    },
    /// `ChatElement.addChatLine(line, r, g, b, Medium, range, ...)`.
    ChatLine {
        target: ChatTarget,
        text: String,
        rgb: [f32; 3],
        range: i32,
    },
    /// `IsoPlayer.SayRadio(line, r, g, b, Medium, range, channel, "radio")`.
    SayRadio {
        text: String,
        rgb: [f32; 3],
        range: i32,
        channel: i32,
    },
    /// `ChatManager.showStaticRadioSound`.
    ShowStaticRadioSound(String),
    /// `ChatManager.showRadioMessage(line, channel)`.
    ShowRadioMessage { text: String, channel: i32 },
    /// The `OnDeviceText` Lua event (`pos` is `None` for -1, -1, -1).
    OnDeviceText {
        guid: Option<String>,
        codes: String,
        pos: Option<[f32; 3]>,
        text: String,
    },
    /// `IsoWaveSignal`: the light source is added to the cell
    /// (`addLamppost`, light recalculation).
    AddLight { radius: i32 },
    /// `removeLightSourceFromWorld`.
    RemoveLight,
    /// `IsoTelevision.updateLightSource`: new radius and colour.
    LightChanged { radius: i32, rgb: [f32; 3] },
    /// `chatElement.setHistoryRange(range); chatElement.update()`.
    ChatUpdate { history_range: f32 },
    /// The television overlay sprite changed (`None` is no overlay, else
    /// an index into the screen sprites); `invalidateRenderChunkLevel`.
    TvOverlay(Option<usize>),
}

/// The power situation of a device that is not battery powered
/// (`canBePoweredHere`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainsPower {
    /// A vehicle part: installed and the vehicle battery is charged.
    Vehicle { uninstalled: bool, has_power: bool },
    /// The parent's square (`None` is a null square: an NPE in the
    /// original, answered as unpowered).
    Square(Option<SquarePower>),
}

/// `IsoGridSquare.hasGridPower()` / `haveElectricity()` / `getRoom()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SquarePower {
    pub grid_power: bool,
    pub have_electricity: bool,
    pub has_room: bool,
}

/// `DeviceData.getIsoObject()`'s position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IsoObjectPos {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `instanceof IsoGameCharacter` (no half-tile offset).
    pub character: bool,
}

/// A line of a recorded media (`MediaData.MediaLineData`).
#[derive(Clone, Debug, PartialEq)]
pub struct MediaLine {
    /// `getTranslatedText()`.
    pub text: String,
    pub rgb: [f32; 3],
    /// `getTextGuid()`.
    pub guid: Option<String>,
    pub codes: Option<String>,
}

/// The listening player of `WaveSignalDevice.AddDeviceText(player, ...)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Listener {
    pub local: bool,
    pub deaf: bool,
    /// `player.isEquipped(parent)` for a radio item.
    pub equipped_parent: bool,
}

/// An emitter handle owned by the device (`IsoWorld.getFreeEmitter`).
pub type EmitterId = u64;

/// The device's sound emitter (`BaseSoundEmitter` / `FMODSoundEmitter`).
pub trait DeviceAudio {
    /// `getFreeEmitter(x, y, z)`, `takeOwnershipOfEmitter`, and the
    /// device as its `parameterUpdater`.
    fn get_free_emitter(&mut self, x: f32, y: f32, z: i32) -> EmitterId;
    /// `stopAll()`, clear the `parameterUpdater`,
    /// `returnOwnershipOfEmitter`.
    fn return_emitter(&mut self, e: EmitterId);
    fn set_pos(&mut self, e: EmitterId, x: f32, y: f32, z: i32);
    /// `playSound(name)` (sent to other players).
    fn play_sound(&mut self, e: EmitterId, name: &str) -> i64;
    /// `playSoundImpl(name, null)` (local).
    fn play_sound_impl(&mut self, e: EmitterId, name: &str) -> i64;
    fn is_using_parameter(&mut self, e: EmitterId, sound: i64, name: &str) -> bool;
    fn set_volume(&mut self, e: EmitterId, sound: i64, volume: f32);
    fn stop_sound(&mut self, e: EmitterId, sound: i64);
    fn stop_or_trigger_sound(&mut self, e: EmitterId, sound: i64);
    fn stop_or_trigger_sound_by_name(&mut self, e: EmitterId, name: &str);
    fn is_playing(&mut self, e: EmitterId, name: &str) -> bool;
    fn tick(&mut self, e: EmitterId);
    /// `parameterList.update()`: the `DeviceVolume` parameter's value.
    fn update_device_volume(&mut self, _e: Option<EmitterId>, _volume: f32) {}
}

/// The world as one device sees it.
pub trait DeviceEnv {
    fn net(&self) -> NetMode {
        NetMode::SinglePlayer
    }
    /// `GameTime.getMinutesStamp()`.
    fn minutes_stamp(&self) -> i64;
    /// `GameTime.getMultiplier()`.
    fn multiplier(&self) -> f32;
    fn parent(&self) -> ParentKind;
    /// `Rand.Next(bound)` (`bound > 0`).
    fn next_int(&mut self, bound: i32) -> i32;
    /// The `Rand` generator's `nextFloat()`.
    fn next_float(&mut self) -> f32;
    /// `canBePoweredHere` for a device that is not battery powered.
    fn mains_power(&self) -> MainsPower;
    /// `ZomboidRadio.GetChannelList(category)`, in iteration order.
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>>;
    /// Whether `getMediaDataFromIndex(index)` is non-null.
    fn media_exists(&self, index: i16) -> bool;
    /// A line of the recorded media `index`.
    fn media_line(&self, index: i16, line: i32) -> Option<MediaLine>;
    /// `getIsoObject()` (the object, the item's outermost container
    /// parent, or the vehicle).
    fn iso_object(&self) -> Option<IsoObjectPos>;
    /// `FMODParameterUtils.getClosestListenerDistanceSquared(x, y, z)`.
    fn closest_listener_distance_sq(&self, x: f32, y: f32, z: f32) -> f32;
    /// The parent's `getX/Y/Z()`.
    fn parent_pos(&self) -> [f32; 3];
    /// Whether the world-sound source exists (an item uses
    /// `IsoPlayer.getInstance()`).
    fn has_sound_source(&self) -> bool {
        true
    }
    /// `getDeviceSoundVolumeRange`'s room test: the item's player's
    /// square, or the object's square, is in a room.
    fn in_room(&self) -> bool {
        false
    }
    /// The radio item's `getPlayer()`: `None` without one, else whether
    /// the player is deaf.
    fn item_player_deaf(&self) -> Option<bool> {
        None
    }
    fn event(&mut self, e: DeviceEvent);
    /// The emitter side; `None` where there is no sound (a server).
    fn audio(&mut self) -> Option<&mut dyn DeviceAudio> {
        None
    }
}

/// `Rand.Next(float min, float max)`.
pub fn rand_float(env: &mut dyn DeviceEnv, min: f32, max: f32) -> f32 {
    if max == min {
        return min;
    }
    let (min, max) = if min > max { (max, min) } else { (min, max) };
    min + env.next_float() * (max - min)
}

/// `Rand.Next(int min, int max)`.
fn rand_range(env: &mut dyn DeviceEnv, min: i32, max: i32) -> i32 {
    if max == min {
        return min;
    }
    let (min, max) = if min > max { (max, min) } else { (min, max) };
    env.next_int(max.wrapping_sub(min)).wrapping_add(min)
}

/// `ZomboidRadio.getRandomBzztFzzt()`.
pub fn random_bzzt(env: &mut dyn DeviceEnv) -> &'static str {
    STATIC_SOUNDS[env.next_int(STATIC_SOUNDS.len() as i32) as usize]
}

/// The `IsoTelevision.Screens` of the screen state machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TvScreen {
    #[default]
    Off,
    Test,
    Default,
    Alternate,
}

/// The unsaved state of an `IsoWaveSignal` / `IsoTelevision` (kept with
/// the device since the object owns exactly one).
#[derive(Clone, Debug, PartialEq)]
pub struct WaveSignalRuntime {
    pub has_player_in_range: bool,
    /// `lightSource != null`.
    pub has_light: bool,
    pub light_was_removed: bool,
    pub light_source_radius: i32,
    pub next_light_update: f32,
    pub light_update_cnt: f32,
    pub light_radius: i32,
    pub light_rgb: [f32; 3],
    pub current_screen: TvScreen,
    pub sprite_index: i32,
    pub has_setup_screens: bool,
    pub tick_is_light_update: bool,
    /// The number of screen sprites found by `setupDefaultScreens`.
    pub screen_sprites: usize,
    /// The overlay sprite (an index into the screen sprites).
    pub overlay: Option<usize>,
}

impl Default for WaveSignalRuntime {
    fn default() -> Self {
        WaveSignalRuntime {
            has_player_in_range: false,
            has_light: false,
            light_was_removed: false,
            light_source_radius: 4,
            next_light_update: 0.0,
            light_update_cnt: 0.0,
            light_radius: 4,
            light_rgb: [0.0, 0.0, 1.0],
            current_screen: TvScreen::Off,
            sprite_index: 0,
            has_setup_screens: false,
            tick_is_light_update: false,
            screen_sprites: 0,
            overlay: None,
        }
    }
}

/// The `DeviceData` fields that are not saved.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceRuntime {
    pub last_recorded_distance: i32,
    pub channel_changed_recently: bool,
    pub emitter: Option<EmitterId>,
    /// `playingMedia` (by its recorded media index).
    pub playing_media: Option<i16>,
    pub is_playing_media: bool,
    pub media_line_index: i32,
    pub line_counter: f32,
    pub current_media_line: Option<String>,
    pub current_media_color: Option<[f32; 3]>,
    pub is_stopping_media: bool,
    pub stop_media_counter: f32,
    pub radio_loop_sound: i64,
    pub do_trigger_world_sound: bool,
    pub listen_cnt: i32,
    pub next_static_sound: f32,
    pub voip_counter: f32,
    pub signal_counter: f32,
    pub sound_counter: f32,
    pub wave: WaveSignalRuntime,
}

impl Default for DeviceRuntime {
    fn default() -> Self {
        DeviceRuntime {
            last_recorded_distance: -1,
            channel_changed_recently: false,
            emitter: None,
            playing_media: None,
            is_playing_media: false,
            media_line_index: 0,
            line_counter: 0.0,
            current_media_line: None,
            current_media_color: None,
            is_stopping_media: false,
            stop_media_counter: 0.0,
            radio_loop_sound: 0,
            do_trigger_world_sound: false,
            listen_cnt: 0,
            next_static_sound: 0.0,
            voip_counter: 0.0,
            signal_counter: 0.0,
            sound_counter: 0.0,
            wave: WaveSignalRuntime::default(),
        }
    }
}

impl DeviceData {
    /// `getClone()`: a copy without the emitter (the presets are copied;
    /// everything else, including the loop sound id, is carried over).
    pub fn get_clone(&self) -> DeviceData {
        let mut c = self.clone();
        c.presets = DevicePresets {
            max_presets: self.presets.max_presets,
            presets: self.presets.presets.clone(),
        };
        c.runtime.emitter = None;
        c
    }

    /// `cloneDevicePresets(p)`.
    pub fn clone_device_presets(&mut self, p: Option<&DevicePresets>) {
        self.presets.clear_presets();
        if let Some(p) = p {
            for e in &p.presets {
                self.presets.add_preset(&e.name, e.frequency);
            }
        }
    }

    /// `setDeviceVolumeRaw`.
    pub fn set_device_volume_raw(&mut self, f: f32) {
        self.device_volume = clamp01(f);
    }

    /// `setDeviceVolume` (with the client transmit).
    pub fn set_device_volume_env(&mut self, env: &mut dyn DeviceEnv, f: f32) {
        self.device_volume = clamp01(f);
        transmit(env, DeviceState::Volume);
    }

    /// `setChannelRaw`.
    pub fn set_channel_raw(&mut self, c: i32) {
        self.channel = c;
    }

    /// `getDeviceVolumeRange()`.
    pub fn device_volume_range(&self) -> i32 {
        5 + (self.base_volume_range * self.device_volume) as i32
    }

    /// `getDeviceSoundVolumeRange()`.
    pub fn device_sound_volume_range(&self, env: &dyn DeviceEnv) -> i32 {
        match env.parent() {
            ParentKind::Item => {
                if env.in_room() {
                    3 + (self.base_volume_range * 0.4 * self.device_volume) as i32
                } else {
                    5 + (self.base_volume_range * self.device_volume) as i32
                }
            }
            ParentKind::Iso => {
                if env.in_room() {
                    3 + (self.base_volume_range * 0.5 * self.device_volume) as i32
                } else {
                    5 + (self.base_volume_range * 0.75 * self.device_volume) as i32
                }
            }
            _ => 5 + (self.base_volume_range / 2.0 * self.device_volume) as i32,
        }
    }

    /// `isReceivingSignal()`.
    pub fn is_receiving_signal(&self) -> bool {
        self.runtime.signal_counter > 0.0 || self.runtime.voip_counter > 0.0
    }

    /// `hasMedia()`.
    pub fn has_media(&self) -> bool {
        self.media_index >= 0
    }

    /// `isPlayingMedia()`.
    pub fn is_playing_media(&self) -> bool {
        self.runtime.is_playing_media
    }

    /// `isEmergencyBroadcast()`.
    pub fn is_emergency_broadcast(&self, env: &dyn DeviceEnv) -> bool {
        if self.is_television {
            return false;
        }
        env.channel_list("Emergency")
            .is_some_and(|l| l.iter().any(|(k, _)| *k == self.channel))
    }

    /// `addEmergencyChannel()`.
    pub fn add_emergency_channel(&mut self, lists: &dyn crate::device::ChannelLists) {
        if self.is_television {
            return;
        }
        if let Some(list) = lists.channel_list("Emergency") {
            for (k, v) in list {
                self.presets.add_preset(&v, k);
            }
        }
    }

    /// `canBePoweredHere()`.
    pub fn can_be_powered_here(&self, env: &dyn DeviceEnv) -> bool {
        if self.is_battery_powered {
            return true;
        }
        match env.mains_power() {
            MainsPower::Vehicle {
                uninstalled,
                has_power,
            } => !uninstalled && has_power,
            MainsPower::Square(None) => false,
            MainsPower::Square(Some(sq)) => {
                if sq.have_electricity {
                    true
                } else if !sq.has_room {
                    false
                } else {
                    sq.grid_power
                }
            }
        }
    }

    fn set_is_turned_on_internal(&mut self, b: bool) {
        self.is_turned_on = b;
        if !b {
            self.last_minute_stamp = -1;
        }
    }

    /// `setTurnedOnRaw` with the equipped-radio update of an item.
    pub fn set_turned_on_raw_env(&mut self, env: &mut dyn DeviceEnv, b: bool) {
        self.set_is_turned_on_internal(b);
        if env.parent() == ParentKind::Item {
            env.event(DeviceEvent::UpdateEquippedRadioFreq);
        }
    }

    /// `setIsTurnedOn(b)`.
    pub fn set_is_turned_on(&mut self, env: &mut dyn DeviceEnv, b: bool) {
        if self.can_be_powered_here(env) {
            if !self.is_battery_powered || self.power_delta > 0.0 {
                self.set_is_turned_on_internal(b);
            } else {
                self.set_is_turned_on_internal(false);
            }
            transmit(env, DeviceState::TurnedOn);
        } else if self.is_turned_on {
            self.set_is_turned_on_internal(false);
            transmit(env, DeviceState::TurnedOn);
        }
        if env.parent() == ParentKind::Item {
            env.event(DeviceEvent::UpdateEquippedRadioFreq);
        }
        env.event(DeviceEvent::UpdateGenerator);
    }

    /// `setMicIsMuted(b)`.
    pub fn set_mic_is_muted(&mut self, env: &mut dyn DeviceEnv, b: bool) {
        self.mic_is_muted = b;
        if env.parent() == ParentKind::Item {
            env.event(DeviceEvent::UpdateEquippedRadioFreq);
        }
    }

    /// `setChannel(chan, setIsListening)`.
    pub fn set_channel(&mut self, env: &mut dyn DeviceEnv, chan: i32, set_is_listening: bool) {
        if chan < self.min_channel_range || chan > self.max_channel_range {
            return;
        }
        self.channel = chan;
        let zap = if self.is_television {
            "TelevisionZap"
        } else if env.parent() == ParentKind::Vehicle {
            "VehicleRadioZap"
        } else {
            "RadioZap"
        };
        self.play_sound_send(env, zap, true);
        if self.runtime.radio_loop_sound > 0 {
            let (e, s) = (self.runtime.emitter, self.runtime.radio_loop_sound);
            if let (Some(e), Some(a)) = (e, env.audio()) {
                a.stop_sound(e, s);
            }
            self.runtime.radio_loop_sound = 0;
        }
        transmit(env, DeviceState::Channel);
        if set_is_listening {
            self.trigger_player_listening(env);
        }
    }

    /// `TriggerPlayerListening(true)`.
    pub fn trigger_player_listening(&mut self, env: &mut dyn DeviceEnv) {
        if self.is_turned_on {
            env.event(DeviceEvent::PlayerListensChannel {
                channel: self.channel,
                is_tv: self.is_television,
            });
        }
    }

    /// `playSoundSend(name, useDeviceVolume)`.
    pub fn play_sound_send(&mut self, env: &mut dyn DeviceEnv, name: &str, use_volume: bool) {
        let v = if use_volume { self.device_volume } else { 1.0 };
        self.play_sound(env, name, v, true);
    }

    /// `playSoundLocal(name, useDeviceVolume)`.
    pub fn play_sound_local(&mut self, env: &mut dyn DeviceEnv, name: &str, use_volume: bool) {
        let v = if use_volume { self.device_volume } else { 1.0 };
        self.play_sound(env, name, v, false);
    }

    /// `playSound(name, volume, transmit)`.
    pub fn play_sound(&mut self, env: &mut dyn DeviceEnv, name: &str, volume: f32, send: bool) {
        if env.net() == NetMode::Server {
            return;
        }
        self.set_emitter_and_pos(env);
        let Some(e) = self.runtime.emitter else {
            return;
        };
        if let Some(a) = env.audio() {
            let id = if send {
                a.play_sound(e, name)
            } else {
                a.play_sound_impl(e, name)
            };
            set_sound_volume(a, e, id, volume);
        }
    }

    /// `stopOrTriggerSoundByName(name)`.
    pub fn stop_or_trigger_sound_by_name(&mut self, env: &mut dyn DeviceEnv, name: &str) {
        if let (Some(e), Some(a)) = (self.runtime.emitter, env.audio()) {
            a.stop_or_trigger_sound_by_name(e, name);
        }
    }

    /// `cleanSoundsAndEmitter()`.
    pub fn clean_sounds_and_emitter(&mut self, env: &mut dyn DeviceEnv) {
        if let Some(e) = self.runtime.emitter.take() {
            if let Some(a) = env.audio() {
                a.return_emitter(e);
            }
            self.runtime.radio_loop_sound = 0;
        }
    }

    /// `setEmitterAndPos()`.
    fn set_emitter_and_pos(&mut self, env: &mut dyn DeviceEnv) {
        let Some(src) = env.iso_object() else {
            return;
        };
        let off = if env.parent() == ParentKind::Vehicle || src.character {
            0.0
        } else {
            0.5
        };
        let (x, y, z) = (src.x + off, src.y + off, fastfloor(src.z));
        let (vol, loop_sound) = (self.device_volume, self.runtime.radio_loop_sound);
        let Some(a) = env.audio() else {
            return;
        };
        let e = match self.runtime.emitter {
            None => {
                let e = a.get_free_emitter(x, y, z);
                self.runtime.emitter = Some(e);
                e
            }
            Some(e) => {
                a.set_pos(e, x, y, z);
                e
            }
        };
        if loop_sound != 0 {
            set_sound_volume(a, e, loop_sound, vol);
        }
    }

    /// `updateEmitter()`.
    fn update_emitter(&mut self, env: &mut dyn DeviceEnv) {
        if env.net() == NetMode::Server {
            return;
        }
        let (em, vol) = (self.runtime.emitter, self.device_volume);
        if let Some(a) = env.audio() {
            a.update_device_volume(em, vol);
        }
        let dist_sq = match env.iso_object() {
            Some(o) => env.closest_listener_distance_sq(o.x, o.y, o.z),
            None => f32::MAX,
        };
        if !self.is_turned_on || dist_sq > 256.0 {
            let button = match (self.runtime.emitter, env.audio()) {
                (Some(e), Some(a)) => {
                    a.is_playing(e, "RadioButton")
                        || a.is_playing(e, "TelevisionOff")
                        || a.is_playing(e, "VehicleRadioButton")
                }
                _ => false,
            };
            if button {
                if self.runtime.radio_loop_sound > 0 {
                    let (e, s) = (self.runtime.emitter, self.runtime.radio_loop_sound);
                    if let (Some(e), Some(a)) = (e, env.audio()) {
                        a.stop_sound(e, s);
                    }
                }
                self.set_emitter_and_pos(env);
                if let (Some(e), Some(a)) = (self.runtime.emitter, env.audio()) {
                    a.tick(e);
                }
                return;
            }
            self.clean_sounds_and_emitter(env);
            return;
        }
        self.set_emitter_and_pos(env);
        let Some(e) = self.runtime.emitter else {
            return;
        };
        let vehicle = env.parent() == ParentKind::Vehicle;
        let mut sound_name = if vehicle {
            "VehicleRadioProgram"
        } else {
            "RadioTalk"
        };
        if self.is_emergency_broadcast(env) {
            sound_name = "BroadcastEmergency";
        }
        let Some(a) = env.audio() else {
            return;
        };
        if self.runtime.signal_counter > 0.0 && !a.is_playing(e, sound_name) {
            if self.runtime.radio_loop_sound > 0 {
                a.stop_sound(e, self.runtime.radio_loop_sound);
            }
            self.runtime.radio_loop_sound = a.play_sound_impl(e, sound_name);
            set_sound_volume(a, e, self.runtime.radio_loop_sound, vol);
        }
        let mut loop_sound = if !self.is_television {
            "RadioStatic"
        } else {
            "TelevisionTestBeep"
        };
        if vehicle {
            loop_sound = "VehicleRadioStatic";
        }
        let restart = self.runtime.radio_loop_sound == 0
            || self.runtime.signal_counter <= 0.0 && !a.is_playing(e, loop_sound);
        if restart {
            if self.runtime.radio_loop_sound > 0 {
                a.stop_or_trigger_sound(e, self.runtime.radio_loop_sound);
                if !self.is_television {
                    if vehicle {
                        self.play_sound_send(env, "VehicleRadioZap", true);
                    } else {
                        self.play_sound_local(env, "RadioZap", true);
                    }
                }
            }
            let Some(a) = env.audio() else {
                return;
            };
            self.runtime.radio_loop_sound = a.play_sound_impl(e, loop_sound);
            set_sound_volume(a, e, self.runtime.radio_loop_sound, vol);
        }
        if let Some(a) = env.audio() {
            a.tick(e);
        }
    }

    /// `update(isIso, playerInRange)`.
    pub fn update(&mut self, env: &mut dyn DeviceEnv, is_iso: bool, player_in_range: bool) {
        let net = env.net();
        let now = env.minutes_stamp();
        if self.last_minute_stamp == -1 {
            self.last_minute_stamp = now;
        }
        if now > self.last_minute_stamp {
            let diff = now - self.last_minute_stamp;
            self.last_minute_stamp = now;
            self.runtime.listen_cnt = (self.runtime.listen_cnt as i64 + diff) as i32;
            if self.runtime.listen_cnt >= 10 {
                self.runtime.listen_cnt = 0;
            }
            let lc = self.runtime.listen_cnt;
            if net != NetMode::Server
                && self.is_turned_on
                && player_in_range
                && (lc == 0 || lc == 5)
            {
                self.trigger_player_listening(env);
            }
            if self.is_turned_on && self.is_battery_powered && self.power_delta > 0.0 {
                let pdmod = self.power_delta - self.power_delta % 0.01;
                self.set_power(self.power_delta - self.use_delta * diff as f32);
                if self.runtime.listen_cnt == 0
                    || self.power_delta == 0.0
                    || self.power_delta < pdmod
                {
                    if is_iso && net == NetMode::Server {
                        transmit_server(env, DeviceState::Power);
                    } else if !is_iso && net == NetMode::Client {
                        transmit(env, DeviceState::Power);
                    }
                }
            }
        }
        if self.is_turned_on
            && (self.is_battery_powered && self.power_delta <= 0.0
                || !self.can_be_powered_here(env))
        {
            self.set_is_turned_on_internal(false);
            if is_iso && net == NetMode::Server {
                transmit_server(env, DeviceState::TurnedOn);
            } else if !is_iso && net == NetMode::Client {
                transmit(env, DeviceState::TurnedOn);
            }
        }
        self.update_media_playing(env);
        self.update_emitter(env);
        self.update_simple(env);
    }

    /// `updateSimple()`.
    pub fn update_simple(&mut self, env: &mut dyn DeviceEnv) {
        let m = env.multiplier();
        let rt = &mut self.runtime;
        if rt.voip_counter >= 0.0 {
            rt.voip_counter -= 1.25 * m;
        }
        if rt.signal_counter >= 0.0 {
            rt.signal_counter -= 1.25 * m;
        }
        if rt.sound_counter >= 0.0 {
            rt.sound_counter -= 1.25 * m;
        }
        if rt.signal_counter <= 0.0 && rt.voip_counter <= 0.0 && rt.last_recorded_distance >= 0 {
            rt.last_recorded_distance = -1;
        }
        self.update_static_sounds(env);
        let net = env.net();
        if net == NetMode::Client {
            self.update_emitter(env);
        }
        if self.runtime.do_trigger_world_sound && self.runtime.sound_counter <= 0.0 {
            let inv = env.parent() == ParentKind::Item;
            let mode_ok = match net {
                NetMode::SinglePlayer => true,
                NetMode::Client => inv,
                NetMode::Server => !inv,
            };
            if self.is_turned_on
                && self.device_volume > 0.0
                && (!inv || self.headphone_type < 0)
                && mode_ok
                && env.parent() != ParentKind::None
                && env.has_sound_source()
            {
                let volume = (100.0 * self.device_volume) as i32;
                let radius = self.device_sound_volume_range(env);
                env.event(DeviceEvent::WorldSound {
                    repeating: true,
                    radius,
                    volume,
                    stress: volume > 50,
                });
            }
            self.runtime.do_trigger_world_sound = false;
            self.runtime.sound_counter = (300 + rand_range(env, 0, 300)) as f32;
        }
    }

    /// `updateStaticSounds()`.
    fn update_static_sounds(&mut self, env: &mut dyn DeviceEnv) {
        if !self.is_turned_on {
            return;
        }
        let delta = env.multiplier();
        self.runtime.next_static_sound -= delta;
        if self.runtime.next_static_sound <= 0.0 {
            if env.parent() != ParentKind::None
                && self.runtime.signal_counter <= 0.0
                && !self.no_transmit
                && !self.is_playing_media()
            {
                let s = random_bzzt(env);
                self.add_device_text(env, s, [1.0, 1.0, 1.0], None, None, -1);
                self.runtime.do_trigger_world_sound = true;
            }
            self.set_next_static_sound(env);
        }
    }

    fn set_next_static_sound(&mut self, env: &mut dyn DeviceEnv) {
        self.runtime.next_static_sound = rand_float(env, 250.0, 1500.0);
    }

    /// `doReceiveSignal(distance)`.
    pub fn do_receive_signal(&mut self, env: &mut dyn DeviceEnv, distance: i32) {
        if !self.is_turned_on {
            return;
        }
        self.runtime.last_recorded_distance = distance;
        let parent = env.parent();
        if self.device_volume > 0.0 && (parent == ParentKind::Iso || self.headphone_type < 0) {
            let source = parent != ParentKind::None && env.has_sound_source();
            if source && self.runtime.sound_counter <= 0.0 {
                let volume = (100.0 * self.device_volume) as i32;
                let radius = self.device_sound_volume_range(env);
                env.event(DeviceEvent::WorldSound {
                    repeating: false,
                    radius,
                    volume,
                    stress: volume > 50,
                });
                self.runtime.sound_counter = 120.0;
            }
        }
        self.runtime.signal_counter = 300.0;
        self.runtime.do_trigger_world_sound = true;
        self.set_next_static_sound(env);
    }

    /// `doReceiveMPSignal(distance)`.
    pub fn do_receive_mp_signal(&mut self, distance: f32) {
        self.runtime.last_recorded_distance = distance as i32;
        self.runtime.voip_counter = 10.0;
    }

    /// The parent's own `AddDeviceText(line, r, g, b, guid, codes,
    /// distance)` (`IsoWaveSignal`, `VehiclePart` or `Radio`).
    pub fn add_device_text(
        &mut self,
        env: &mut dyn DeviceEnv,
        line: &str,
        rgb: [f32; 3],
        guid: Option<&str>,
        codes: Option<&str>,
        distance: i32,
    ) {
        match env.parent() {
            ParentKind::Iso | ParentKind::Vehicle => {
                if !self.is_turned_on {
                    return;
                }
                if !is_static_sound(line) {
                    self.do_receive_signal(env, distance);
                }
                if self.device_volume > 0.0 {
                    env.event(DeviceEvent::ChatLine {
                        target: ChatTarget::Device,
                        text: line.to_string(),
                        rgb,
                        range: self.device_volume_range(),
                    });
                    if let Some(codes) = codes {
                        env.event(DeviceEvent::OnDeviceText {
                            guid: guid.map(str::to_string),
                            codes: codes.to_string(),
                            pos: Some(env.parent_pos()),
                            text: line.to_string(),
                        });
                    }
                }
            }
            ParentKind::Item => {
                if !is_static_sound(line) {
                    self.do_receive_signal(env, distance);
                }
                if env.item_player_deaf() == Some(false) && self.device_volume > 0.0 {
                    env.event(DeviceEvent::SayRadio {
                        text: line.to_string(),
                        rgb,
                        range: self.device_volume_range(),
                        channel: self.channel,
                    });
                    if let Some(codes) = codes {
                        env.event(DeviceEvent::OnDeviceText {
                            guid: guid.map(str::to_string),
                            codes: codes.to_string(),
                            pos: None,
                            text: line.to_string(),
                        });
                    }
                }
            }
            ParentKind::None => {}
        }
    }

    /// `WaveSignalDevice.AddDeviceText(player, line, ...)`: the default
    /// method the radio uses for a line a player hears.
    #[allow(clippy::too_many_arguments)]
    pub fn add_device_text_for_player(
        &mut self,
        env: &mut dyn DeviceEnv,
        player: Option<Listener>,
        line: &str,
        rgb: [f32; 3],
        guid: Option<&str>,
        codes: Option<&str>,
        distance: i32,
    ) {
        if self.device_volume <= 0.0 {
            return;
        }
        if !is_static_sound(line) {
            self.do_receive_signal(env, distance);
        }
        let Some(p) = player.filter(|p| p.local && !p.deaf) else {
            return;
        };
        let target = match env.parent() {
            ParentKind::Item if p.equipped_parent => Some(ChatTarget::Player),
            ParentKind::Iso | ParentKind::Vehicle => Some(ChatTarget::Device),
            _ => None,
        };
        if let Some(target) = target {
            env.event(DeviceEvent::ChatLine {
                target,
                text: line.to_string(),
                rgb,
                range: self.device_volume_range(),
            });
        }
        if is_static_sound(line) {
            env.event(DeviceEvent::ShowStaticRadioSound(line.to_string()));
        } else {
            env.event(DeviceEvent::ShowRadioMessage {
                text: line.to_string(),
                channel: self.channel,
            });
        }
        if let Some(codes) = codes {
            env.event(DeviceEvent::OnDeviceText {
                guid: guid.map(str::to_string),
                codes: codes.to_string(),
                pos: None,
                text: line.to_string(),
            });
        }
    }

    /// `StartPlayMedia()`.
    pub fn start_play_media(&mut self, env: &mut dyn DeviceEnv) {
        if env.net() == NetMode::Client {
            transmit(env, DeviceState::StartMedia);
        } else if !self.is_playing_media() && self.is_turned_on && self.has_media() {
            if env.media_exists(self.media_index) {
                self.runtime.playing_media = Some(self.media_index);
                self.runtime.is_playing_media = true;
                self.runtime.media_line_index = 0;
                self.pre_playing_media(env);
                transmit_server(env, DeviceState::StartMedia);
            } else {
                self.runtime.playing_media = None;
            }
        }
    }

    /// `StopPlayMedia()`.
    pub fn stop_play_media(&mut self, env: &mut dyn DeviceEnv) {
        if env.net() == NetMode::Client {
            transmit(env, DeviceState::StopMedia);
            return;
        }
        if env.net() == NetMode::Server {
            self.runtime.is_playing_media = false;
        }
        self.runtime.playing_media = None;
        self.runtime.is_stopping_media = true;
        self.runtime.stop_media_counter = 60.0 * MAXMOD * 0.5;
        self.television_media_switch(env);
        transmit_server(env, DeviceState::StopMedia);
    }

    /// `prePlayingMedia()`.
    fn pre_playing_media(&mut self, env: &mut dyn DeviceEnv) {
        self.runtime.line_counter = 60.0 * MAXMOD * 0.5;
        self.television_media_switch(env);
    }

    /// `televisionMediaSwitch()` (the first `getRandomBzztFzzt` draw is
    /// discarded, as in the original).
    fn television_media_switch(&mut self, env: &mut dyn DeviceEnv) {
        if self.media_type == 1 {
            let _ = random_bzzt(env);
            let s = random_bzzt(env);
            self.add_device_text(env, s, [0.5, 0.5, 0.5], None, None, 0);
            self.play_sound_local(env, "TelevisionZap", true);
        }
    }

    /// `updateMediaPlaying()`.
    pub fn update_media_playing(&mut self, env: &mut dyn DeviceEnv) {
        let net = env.net();
        if net == NetMode::Client
            && !(self.is_turned_on
                && self.device_volume > 0.0
                && env.parent() == ParentKind::Item
                && self.headphone_type >= 0)
        {
            return;
        }
        let m = env.multiplier();
        if self.runtime.is_stopping_media {
            self.runtime.stop_media_counter -= 1.25 * m;
            if self.runtime.stop_media_counter <= 0.0 {
                self.runtime.is_playing_media = false;
                self.runtime.is_stopping_media = false;
            }
            return;
        }
        if !(self.has_media() && self.is_playing_media()) {
            return;
        }
        if !self.is_turned_on {
            self.stop_play_media(env);
            return;
        }
        let Some(media) = self.runtime.playing_media else {
            return;
        };
        self.runtime.line_counter -= 1.25 * m;
        if self.runtime.line_counter > 0.0 {
            return;
        }
        let Some(line) = env.media_line(media, self.runtime.media_line_index) else {
            self.stop_play_media(env);
            return;
        };
        let lc = java_length(&line.text) as f32 / 10.0 * 60.0;
        self.runtime.line_counter = lc.clamp(60.0 * MINMOD, 60.0 * MAXMOD);
        if net == NetMode::Server {
            self.runtime.current_media_line = Some(line.text.clone());
            self.runtime.current_media_color = Some(line.rgb);
            transmit_server(env, DeviceState::MediaLine);
            if self.is_turned_on && self.device_volume > 0.0 {
                if let Some(codes) = &line.codes {
                    env.event(DeviceEvent::OnDeviceText {
                        guid: line.guid.clone(),
                        codes: codes.clone(),
                        pos: Some(env.parent_pos()),
                        text: line.text.clone(),
                    });
                }
            }
        } else {
            self.add_device_text(
                env,
                &line.text,
                line.rgb,
                line.guid.as_deref(),
                line.codes.as_deref(),
                0,
            );
        }
        self.runtime.media_line_index += 1;
    }

    /// `addBattery(bat)` for a `Base.Battery` with `current_uses`
    /// (`getCurrentUsesFloat()`): `true` when the battery goes into the
    /// device (the caller removes the item unless it is a client).
    pub fn add_battery(
        &mut self,
        env: &mut dyn DeviceEnv,
        full_type: &str,
        current_uses: f32,
    ) -> bool {
        if self.has_battery || full_type != "Base.Battery" {
            return false;
        }
        self.power_delta = current_uses;
        self.has_battery = true;
        transmit_server(env, DeviceState::Battery);
        true
    }

    /// `getBattery(inventory)`: `Some(power)` when a `Base.Battery` with
    /// `setCurrentUses((int)(maxUses * power))` is to be added to the
    /// inventory (not on a client, which still empties the device).
    pub fn take_battery(&mut self, env: &mut dyn DeviceEnv) -> Option<f32> {
        if !self.has_battery {
            return None;
        }
        let out = (env.net() != NetMode::Client).then_some(self.power_delta);
        self.power_delta = 0.0;
        self.has_battery = false;
        transmit_server(env, DeviceState::Battery);
        out
    }

    /// `addHeadphones(item, container)`: `true` when the headphones go
    /// into the device (the caller removes the item from `container`).
    pub fn add_headphones(
        &mut self,
        env: &mut dyn DeviceEnv,
        full_type: &str,
        has_container: bool,
    ) -> bool {
        if env.net() == NetMode::Client {
            return false;
        }
        if self.headphone_type < 0
            && (full_type == HEADPHONES_FULL_TYPE || full_type == EARBUDS_FULL_TYPE)
            && has_container
        {
            self.headphone_type = if full_type == HEADPHONES_FULL_TYPE {
                0
            } else {
                1
            };
            transmit_server(env, DeviceState::Headphones);
            return true;
        }
        false
    }

    /// `addHeadphonesToInventory(inventory)`: the item type to create and
    /// add (`None` when there is nothing to add).
    pub fn take_headphones(&mut self, env: &mut dyn DeviceEnv) -> Option<&'static str> {
        if env.net() == NetMode::Client || self.headphone_type < 0 {
            return None;
        }
        let item = match self.headphone_type {
            0 => Some(HEADPHONES_FULL_TYPE),
            1 => Some(EARBUDS_FULL_TYPE),
            _ => None,
        };
        self.headphone_type = -1;
        transmit_server(env, DeviceState::Headphones);
        item
    }

    /// `addMediaItem(media)` for a recorded media item of
    /// `media_type` / `recorded_media_index`: `true` when it goes into
    /// the device (the caller removes it from its container unless a
    /// client).
    pub fn add_media_item(
        &mut self,
        env: &mut dyn DeviceEnv,
        full_type: &str,
        is_recorded_media: bool,
        media_type: i8,
        recorded_media_index: i16,
    ) -> bool {
        if self.media_index >= 0 || !is_recorded_media || media_type != self.media_type {
            return false;
        }
        self.media_index = recorded_media_index;
        self.media_item = Some(full_type.to_string());
        transmit_server(env, DeviceState::MediaItem);
        true
    }

    /// `removeMediaItem(inventory)`: the item to create (its full type and
    /// `setRecordedMediaIndex`), `None` on a client or without media.
    pub fn remove_media_item(&mut self, env: &mut dyn DeviceEnv) -> Option<(Option<String>, i16)> {
        if !self.has_media() {
            return None;
        }
        let out =
            (env.net() != NetMode::Client).then(|| (self.media_item.clone(), self.media_index));
        if self.is_playing_media() {
            self.stop_play_media(env);
        }
        self.media_index = -1;
        self.media_item = None;
        transmit_server(env, DeviceState::MediaItem);
        out
    }

    /// `IsoWaveSignal.update()`; `player` is `IsoPlayer.getInstance()`'s
    /// position. With `tv` the `IsoTelevision` screen and light follow;
    /// `screen_sprites` is the number of screen sprites
    /// `setupDefaultScreens` finds and `sprite_changed` whether the
    /// object's sprite is not the cached one; `has_chat` is
    /// `chatElement.getHasChatToDisplay()`.
    pub fn update_wave_signal(
        &mut self,
        env: &mut dyn DeviceEnv,
        player: Option<(f32, f32)>,
        tv: Option<TvUpdate>,
        has_chat: bool,
    ) {
        let net = env.net();
        if net == NetMode::Client {
            self.update_simple(env);
        } else {
            let in_range = self.runtime.wave.has_player_in_range;
            self.update(env, true, in_range);
        }
        if net != NetMode::Server {
            self.runtime.wave.has_player_in_range = false;
            if self.is_turned_on {
                let dist = self.device_volume_range() as f32 * 0.6;
                let [x, y, _] = env.parent_pos();
                // The original tests with `||`, which holds for nearly
                // every position.
                if let Some((px, py)) = player {
                    if (px > x - dist || px < x + dist) && (py > y - dist || py < y + dist) {
                        self.runtime.wave.has_player_in_range = true;
                    }
                }
                if tv.is_some() {
                    self.update_tv_light(env, has_chat);
                }
            } else {
                self.remove_light(env);
            }
            env.event(DeviceEvent::ChatUpdate {
                history_range: self.device_volume_range() as f32 * 0.6,
            });
        } else {
            self.runtime.wave.has_player_in_range = false;
        }
        if let Some(tv) = tv {
            let w = &mut self.runtime.wave;
            if tv.sprite_changed {
                w.has_setup_screens = false;
                w.screen_sprites = 0;
                w.current_screen = TvScreen::Off;
                w.next_light_update = 0.0;
            }
            if !w.has_setup_screens {
                w.has_setup_screens = true;
                w.screen_sprites = tv.screen_sprites;
            }
            self.update_tv_screen(env);
        }
    }

    /// `IsoWaveSignal.removeLightSourceFromWorld()`.
    pub fn remove_light(&mut self, env: &mut dyn DeviceEnv) {
        if self.runtime.wave.has_light {
            self.runtime.wave.has_light = false;
            env.event(DeviceEvent::RemoveLight);
        }
    }

    /// `IsoTelevision.updateLightSource()`.
    fn update_tv_light(&mut self, env: &mut dyn DeviceEnv, has_chat: bool) {
        let m = env.multiplier();
        let w = &mut self.runtime.wave;
        w.tick_is_light_update = false;
        if !w.has_light {
            w.has_light = true;
            w.light_radius = w.light_source_radius;
            w.light_rgb = [0.0, 0.0, 1.0];
            w.light_was_removed = true;
        }
        if w.light_was_removed {
            w.light_was_removed = false;
            let radius = w.light_radius;
            env.event(DeviceEvent::AddLight { radius });
        }
        self.runtime.wave.light_update_cnt += m;
        if self.runtime.wave.light_update_cnt < self.runtime.wave.next_light_update {
            return;
        }
        let (intens_base, interval) = if !has_chat {
            (0.6, rand_float(env, 200.0, 400.0))
        } else {
            (0.0, rand_float(env, 15.0, 300.0))
        };
        let intensity = rand_float(env, intens_base, 1.0);
        let r = 0.58 + 0.25 * intensity;
        let gb = rand_float(env, 0.65, 0.85);
        let w = &mut self.runtime.wave;
        w.tick_is_light_update = true;
        let radius = 1 + ((w.light_source_radius - 1) as f32 * intensity) as i32;
        w.light_radius = radius;
        w.light_rgb = [r, gb, gb];
        w.light_update_cnt = 0.0;
        w.next_light_update = interval;
        env.event(DeviceEvent::LightChanged {
            radius,
            rgb: [r, gb, gb],
        });
    }

    /// `IsoTelevision.setScreen(screen)`.
    fn set_tv_screen(&mut self, screen: TvScreen) {
        let w = &mut self.runtime.wave;
        if screen == TvScreen::Off {
            w.current_screen = TvScreen::Off;
            w.overlay = None;
            return;
        }
        if w.current_screen == screen && screen != TvScreen::Alternate {
            return;
        }
        w.current_screen = screen;
        let n = w.screen_sprites;
        let sprite = match screen {
            TvScreen::Test => (n > 0).then_some(0),
            TvScreen::Default => (n > 1).then_some(1),
            TvScreen::Alternate => {
                if n < 2 {
                    None
                } else if n == 2 {
                    Some(1)
                } else {
                    w.sprite_index += 1;
                    if w.sprite_index < 1 {
                        w.sprite_index = 1;
                    }
                    if w.sprite_index > n as i32 - 1 {
                        w.sprite_index = 1;
                    }
                    Some(w.sprite_index as usize)
                }
            }
            TvScreen::Off => None,
        };
        w.overlay = sprite;
    }

    /// `IsoTelevision.updateTvScreen()`.
    fn update_tv_screen(&mut self, env: &mut dyn DeviceEnv) {
        let before = self.runtime.wave.overlay;
        if self.is_turned_on && self.runtime.wave.screen_sprites > 0 {
            if self.is_receiving_signal() || self.is_playing_media() {
                if self.runtime.wave.tick_is_light_update
                    || self.runtime.wave.current_screen != TvScreen::Alternate
                {
                    self.set_tv_screen(TvScreen::Alternate);
                }
            } else {
                self.set_tv_screen(TvScreen::Test);
            }
        } else if self.runtime.wave.current_screen != TvScreen::Off {
            self.set_tv_screen(TvScreen::Off);
        }
        if before != self.runtime.wave.overlay {
            env.event(DeviceEvent::TvOverlay(self.runtime.wave.overlay));
        }
    }

    /// `IsoWaveSignal.saveState(bb)`'s fields.
    pub fn save_state(&self, w: &mut crate::io::Writer) {
        w.i32(self.presets.presets.len() as i32);
        for p in &self.presets.presets {
            w.string(&p.name);
            w.i32(p.frequency);
        }
        w.bool(self.is_turned_on);
        w.i32(self.channel);
        w.f32(self.device_volume);
    }

    /// `IsoWaveSignal.loadState(bb)`.
    pub fn load_state(&mut self, r: &mut crate::io::Reader<'_>) -> crate::io::SaveResult<()> {
        let n = r.i32()?;
        for i in 0..n.max(0) {
            let name = r.string()?;
            let frequency = r.i32()?;
            if (i as usize) < self.presets.presets.len() {
                let p = &mut self.presets.presets[i as usize];
                p.name = name;
                p.frequency = frequency;
            } else {
                self.presets.add_preset(&name, frequency);
            }
        }
        while self.presets.presets.len() as i32 > n {
            self.presets.remove_preset(n);
        }
        self.set_turned_on_raw(r.bool()?);
        self.channel = r.i32()?;
        self.set_device_volume_raw(r.f32()?);
        Ok(())
    }

    /// `IsoRadio` / `IsoTelevision.getGeneratorPowerConsumption()`.
    pub fn generator_power_consumption(&self, tv: bool) -> f32 {
        if tv {
            if self.is_turned_on {
                0.03
            } else {
                0.0
            }
        } else if self.is_turned_on && !self.is_battery_powered {
            0.01
        } else {
            0.0
        }
    }
}

/// The `IsoTelevision` part of [`DeviceData::update_wave_signal`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TvUpdate {
    pub screen_sprites: usize,
    pub sprite_changed: bool,
}

/// `VehiclePart.updateSignalDevice()` for the device of a part:
/// `uninstalled` is `isInventoryItemUninstalled()`, `players` the local
/// players (`None` for a dead or missing one) and the returned value the
/// part's new `hasPlayerInRange`.
pub fn update_vehicle_part_device(
    d: &mut DeviceData,
    env: &mut dyn DeviceEnv,
    uninstalled: bool,
    has_player_in_range: bool,
    players: &[Option<(f32, f32)>],
) -> bool {
    if d.is_turned_on && uninstalled {
        d.set_is_turned_on(env, false);
    }
    let net = env.net();
    if net == NetMode::Client {
        d.update_simple(env);
    } else {
        d.update(env, true, has_player_in_range);
    }
    if net == NetMode::Server {
        return false;
    }
    let mut in_range = false;
    if d.is_turned_on {
        let dist = d.device_volume_range() as f32 * 0.6;
        let [x, y, _] = env.parent_pos();
        // The original compares the vehicle with itself in the second
        // half of each test, so any living player is in range.
        for (px, py) in players.iter().flatten() {
            if (*px > x - dist || x < x + dist) && (*py > y - dist || y < y + dist) {
                in_range = true;
                break;
            }
        }
    }
    env.event(DeviceEvent::ChatUpdate {
        history_range: d.device_volume_range() as f32 * 0.6,
    });
    in_range
}

/// `Radio.update()` for a radio item: `equipped` is whether a local
/// player has it as `getEquipedRadio()`.
pub fn update_radio_item(d: &mut DeviceData, env: &mut dyn DeviceEnv, equipped: bool) {
    if env.net() == NetMode::Server {
        return;
    }
    if equipped {
        d.update(env, false, true);
    } else {
        d.clean_sounds_and_emitter(env);
    }
}

fn clamp01(f: f32) -> f32 {
    f.clamp(0.0, 1.0)
}

fn fastfloor(f: f32) -> i32 {
    let i = f as i32;
    if f < i as f32 {
        i - 1
    } else {
        i
    }
}

/// `String.length()` (UTF-16 units).
fn java_length(s: &str) -> usize {
    s.encode_utf16().count()
}

fn set_sound_volume(a: &mut dyn DeviceAudio, e: EmitterId, sound: i64, volume: f32) {
    if a.is_using_parameter(e, sound, "DeviceVolume") {
        return;
    }
    a.set_volume(e, sound, volume);
}

/// `transmitDeviceDataState(type)` (clients only).
fn transmit(env: &mut dyn DeviceEnv, state: DeviceState) {
    if env.net() == NetMode::Client {
        env.event(DeviceEvent::Transmit {
            state,
            server: false,
        });
    }
}

/// `transmitDeviceDataStateServer(type, null)` (servers only).
fn transmit_server(env: &mut dyn DeviceEnv, state: DeviceState) {
    if env.net() == NetMode::Server {
        env.event(DeviceEvent::Transmit {
            state,
            server: true,
        });
    }
}

#[cfg(test)]
mod tests;
