//! `ZomboidRadio`: the radio core. It owns the script manager and the
//! channel names, schedules the broadcasts (`UpdateScripts` every ten game
//! minutes, `update` every frame), carries lines and player speech to the
//! devices that can hear them (scrambled by distance and weather), and
//! saves its state to `radio/data/RADIO_SAVE.txt` in the world save.
//!
//! The world side (game time, sandbox options, players, devices) is behind
//! [`RadioWorld`]; devices are opaque [`DeviceId`]s the world resolves.

use std::path::PathBuf;

use crate::channel::AiredLine;
use crate::data::{RadioData, RadioSource, RadioText};
use crate::java::{
    fastfloor, is_whitespace, line_separator, parse_int, rand_bool, rand_next, read_lines,
    simple_lower, simple_upper, split_char, split_whitespace, trim, JavaHashMap, Rng,
};
use crate::manager::{ChannelNames, RadioScriptManager};
use crate::media::RecordedMedia;
use crate::{GameMode, RadioChannel, RadioError};

/// `ZomboidRadio.SAVE_FILE`, under `<save>/radio/data/`.
pub const RADIO_SAVE_FILE: &str = "RADIO_SAVE.txt";

/// `ZomboidRadio.staticSounds`.
pub const STATIC_SOUNDS: [&str; 4] = ["<bzzt>", "<fzzt>", "<wzzt>", "<szzt>"];

/// The vanilla channels scrambled once the player has been near
/// Louisville (`ZomboidRadio.obfuscateChannels`).
pub const OBFUSCATE_CHANNELS: [i32; 6] = [200, 201, 204, 93200, 98000, 101200];

/// `ZomboidRadio.isStaticSound`.
pub fn is_static_sound(s: &str) -> bool {
    STATIC_SOUNDS.contains(&s)
}

/// A registered `WaveSignalDevice` (a radio or television item, world
/// object or vehicle part), resolved by the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeviceId(pub u64);

/// What the radio reads from a device and its `DeviceData`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeviceView {
    pub turned_on: bool,
    pub two_way: bool,
    pub portable: bool,
    pub television: bool,
    pub mic_muted: bool,
    pub mic_range: i32,
    pub transmit_range: i32,
    pub channel: i32,
    pub volume: f32,
    /// `isPlayingMedia()`.
    pub playing_media: bool,
    /// `isNoTransmit()`.
    pub no_transmit: bool,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `WaveSignalDevice.HasPlayerInRange()`.
    pub has_player_in_range: bool,
}

/// What the radio reads from a local player (`IsoPlayer.players[i]`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerView {
    pub x: f32,
    pub y: f32,
    /// `getLastSpokenLine()`.
    pub last_spoken_line: Option<String>,
    /// `getSpeakColour()`.
    pub speak_colour: [f32; 3],
    /// `getEquipedRadio()` (only radios with device data).
    pub equipped_radio: Option<(DeviceId, DeviceView)>,
}

/// A message for the network layer (the multiplayer paths; none of them
/// are sent in single player).
#[derive(Clone, Debug, PartialEq)]
pub enum RadioNet {
    /// `GameClient.sendRadioServerDataRequest`.
    ServerDataRequest,
    /// `GameServer.sendRadioPostSilence`.
    PostSilence,
    /// `GameServer.sendIsoWaveSignal` / `GameClient.sendIsoWaveSignal`.
    IsoWaveSignal(Transmission),
    /// `GameClient.sendPlayerListensChannel`.
    PlayerListensChannel {
        channel: i32,
        listening: bool,
        is_tv: bool,
    },
}

/// A line sent on a channel (`SendTransmission`'s arguments).
#[derive(Clone, Debug, PartialEq)]
pub struct Transmission {
    /// The sending connection, -1 for none.
    pub source: i64,
    pub x: i32,
    pub y: i32,
    pub channel: i32,
    pub msg: String,
    pub guid: Option<String>,
    pub codes: Option<String>,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// The transmit range, -1 for a broadcast heard everywhere.
    pub signal_strength: i32,
    pub is_tv: bool,
}

/// The world as the radio sees it. Defaults describe an empty world.
pub trait RadioWorld {
    /// `GameTime.getHour()`.
    fn hour(&self) -> i32;
    /// `GameTime.getNightsSurvived()`.
    fn nights_survived(&self) -> i32;
    /// `GameTime.getMultiplier()`.
    fn multiplier(&self) -> f32;
    /// `SandboxOptions.timeSinceApo` (1 is the first month).
    fn time_since_apo(&self) -> i32;
    /// `Core.gameMode`.
    fn core_game_mode(&self) -> String;
    /// `ClimateManager.getWeatherInterference()`.
    fn weather_interference(&self) -> f32 {
        0.0
    }
    /// `ZomboidFileSystem.getCurrentSaveDir()`, `None` with
    /// `Core.isNoSave()`.
    fn current_save_dir(&self) -> Option<PathBuf>;
    /// `IsoPlayer.numPlayers`.
    fn num_players(&self) -> usize {
        0
    }
    /// `IsoPlayer.players[index]`.
    fn player(&self, _index: usize) -> Option<PlayerView> {
        None
    }
    /// `IsoPlayer.getInstance().getY()`.
    fn local_player_y(&self) -> Option<f32> {
        None
    }
    /// A registered device, `None` when it (or its device data) is gone.
    fn device(&self, _id: DeviceId) -> Option<DeviceView> {
        None
    }
    /// `WaveSignalDevice.AddDeviceText`: with `listener` the player's
    /// variant (the device's chat line, `doReceiveSignal`, the radio
    /// message HUD and `OnDeviceText`), without it the device's own.
    #[allow(clippy::too_many_arguments)]
    fn add_device_text(
        &mut self,
        _device: DeviceId,
        _listener: Option<usize>,
        _text: &str,
        _rgb: [f32; 3],
        _guid: Option<&str>,
        _codes: Option<&str>,
        _distance: i32,
    ) {
    }
    /// The server's `OnDeviceText` Lua event for a device that hears coded
    /// text.
    fn server_device_text(
        &mut self,
        _device: DeviceId,
        _guid: Option<&str>,
        _codes: &str,
        _text: &str,
    ) {
    }
    /// The `OnLoadRadioScripts` Lua event (mods add channels here).
    fn on_load_radio_scripts(&mut self, _radio: &mut ZomboidRadio, _new_game: bool) {}
    /// The `OnInitRecordedMedia` Lua event (registers the CDs and tapes).
    fn on_init_recorded_media(&mut self, _media: &mut RecordedMedia) {}
    /// Multiplayer messages.
    fn net(&mut self, _msg: RadioNet) {}
}

/// `ZomboidRadio.FreqListEntry`.
#[derive(Clone, Copy, Debug)]
struct FreqListEntry {
    transmit_range: i32,
    source_x: i32,
    source_y: i32,
}

/// The transmission side of `ZomboidRadio` (split out so a channel can air
/// a line while the script manager is borrowed).
#[derive(Clone, Debug, Default)]
pub struct Airwaves {
    mode: GameMode,
    devices: Vec<DeviceId>,
    broadcast_devices: Vec<DeviceId>,
    has_applied_range_distortion: bool,
    has_applied_interference: bool,
}

/// `GetDistance`: the truncated Euclidean distance.
fn distance(dx: i32, dy: i32, sx: i32, sy: i32) -> i32 {
    let (a, b) = (
        (dx as i64 - sx as i64) as f64,
        (dy as i64 - sy as i64) as f64,
    );
    (a * a + b * b).sqrt() as i32
}

impl Airwaves {
    /// `getRandomBzztFzzt`.
    pub fn random_bzzt(rng: &mut Rng<'_>) -> &'static str {
        STATIC_SOUNDS[rand_next(rng, STATIC_SOUNDS.len() as i32) as usize]
    }

    /// `scrambleString(msg, intensity, ignoreBBcode, customScramble)`:
    /// each word is replaced by static with `intensity` percent chance (a
    /// run of replaced words gives one static). With `ignore_bbcode`
    /// bracketed tags are kept whole.
    pub fn scramble_string(
        &mut self,
        msg: &str,
        intensity: i32,
        ignore_bbcode: bool,
        custom: Option<&str>,
        rng: &mut Rng<'_>,
    ) -> String {
        self.has_applied_interference = false;
        if intensity <= 0 {
            return msg.to_string();
        }
        if intensity >= 100 {
            return custom.unwrap_or_else(|| Self::random_bzzt(rng)).to_string();
        }
        self.has_applied_interference = true;
        let mut out: Vec<u16> = Vec::new();
        let push = |out: &mut Vec<u16>, s: &str| out.extend(s.encode_utf16());
        if ignore_bbcode {
            let chars: Vec<u16> = msg.encode_utf16().collect();
            let is_ws = |c: u16| char::from_u32(c as u32).is_some_and(is_whitespace);
            let mut scrambling = false;
            let mut opened = false;
            let mut word: Vec<u16> = Vec::new();
            for i in 0..chars.len() {
                let c = chars[i];
                if opened {
                    word.push(c);
                    if c == b']' as u16 {
                        out.append(&mut word);
                        opened = false;
                    }
                    continue;
                }
                if c != b'[' as u16 && (!is_ws(c) || i == 0 || is_ws(chars[i - 1])) {
                    word.push(c);
                    continue;
                }
                if rand_next(rng, 100) > intensity {
                    out.append(&mut word);
                    push(&mut out, " ");
                    scrambling = false;
                } else if !scrambling {
                    push(&mut out, custom.unwrap_or_else(|| Self::random_bzzt(rng)));
                    push(&mut out, " ");
                    scrambling = true;
                }
                word.clear();
                if c == b'[' as u16 {
                    word.push(c);
                    opened = true;
                }
            }
            out.append(&mut word);
        } else {
            let mut scrambling = false;
            for word in split_whitespace(msg) {
                if rand_next(rng, 100) > intensity {
                    push(&mut out, word);
                    push(&mut out, " ");
                    scrambling = false;
                } else if !scrambling {
                    push(&mut out, custom.unwrap_or_else(|| Self::random_bzzt(rng)));
                    push(&mut out, " ");
                    scrambling = true;
                }
            }
        }
        String::from_utf16_lossy(&out)
    }

    fn apply_weather_interference(
        &mut self,
        msg: &str,
        signal_strength: i32,
        world: &dyn RadioWorld,
        rng: &mut Rng<'_>,
    ) -> String {
        let wi = world.weather_interference();
        if wi <= 0.0 {
            return msg.to_string();
        }
        let intensity = (wi * 100.0) as i32;
        self.scramble_string(msg, intensity, signal_strength == -1, None, rng)
    }

    fn device_range_distortion(
        &mut self,
        msg: String,
        signal_strength: i32,
        dist: i32,
        rng: &mut Rng<'_>,
    ) -> String {
        let distort_range = signal_strength as f32 * 0.9;
        if distort_range < signal_strength as f32 && dist as f32 > distort_range {
            let intensity =
                100.0 * ((dist as f32 - distort_range) / (signal_strength as f32 - distort_range));
            let s = self.scramble_string(&msg, intensity as i32, false, None, rng);
            self.has_applied_range_distortion = true;
            return s;
        }
        msg
    }

    /// `SendTransmission`: weather interference greys and scrambles
    /// non-television lines, then the line is distributed (and sent over
    /// the network in multiplayer).
    pub fn send_transmission(
        &mut self,
        mut tx: Transmission,
        world: &mut dyn RadioWorld,
        rng: &mut Rng<'_>,
    ) {
        let mode = self.mode;
        if !tx.is_tv && matches!(mode, GameMode::Server | GameMode::SinglePlayer) {
            self.has_applied_interference = false;
            tx.msg = self.apply_weather_interference(&tx.msg, tx.signal_strength, world, rng);
            if self.has_applied_interference {
                tx.r = 0.5;
                tx.g = 0.5;
                tx.b = 0.5;
                tx.codes = Some(String::new());
            }
        }
        match mode {
            GameMode::SinglePlayer => self.distribute_transmission(&tx, world, rng),
            GameMode::Server => {
                self.distribute_transmission(&tx, world, rng);
                world.net(RadioNet::IsoWaveSignal(tx));
            }
            GameMode::Client => world.net(RadioNet::IsoWaveSignal(tx)),
        }
    }

    /// `DistributeTransmission`: to the local players' equipped radios,
    /// then to the registered devices on the channel.
    pub fn distribute_transmission(
        &mut self,
        tx: &Transmission,
        world: &mut dyn RadioWorld,
        rng: &mut Rng<'_>,
    ) {
        if !tx.is_tv {
            match self.mode {
                GameMode::SinglePlayer => {
                    for i in 0..world.num_players() {
                        if let Some(p) = world.player(i) {
                            self.distribute_to_player(i, &p, tx, world, rng);
                        }
                    }
                }
                // The client hands the line to the radios of the voice
                // manager data (multiplayer, not ported yet).
                GameMode::Client => return,
                GameMode::Server => {}
            }
        }
        let mut msg = tx.msg.clone();
        for i in 0..self.devices.len() {
            let id = self.devices[i];
            let Some(d) = world.device(id) else {
                continue;
            };
            if !d.turned_on || tx.is_tv != d.television {
                continue;
            }
            // As in the original, a device playing media ends the whole
            // distribution, not just its own turn.
            if d.playing_media || d.no_transmit {
                return;
            }
            if tx.channel != d.channel {
                continue;
            }
            let pass =
                tx.signal_strength == -1 || (tx.x != fastfloor(d.x) && tx.y != fastfloor(d.y));
            if !pass {
                continue;
            }
            let mut dist = -1;
            if tx.signal_strength > 0 {
                self.has_applied_range_distortion = false;
                dist = distance(fastfloor(d.x), fastfloor(d.y), tx.x, tx.y);
                // The scrambled text carries on to the next devices.
                msg = self.device_range_distortion(msg, tx.signal_strength, dist, rng);
            }
            let rgb = if self.has_applied_range_distortion {
                [0.5, 0.5, 0.5]
            } else {
                [tx.r, tx.g, tx.b]
            };
            if self.mode == GameMode::Server {
                if d.volume > 0.0 {
                    if let Some(codes) = &tx.codes {
                        world.server_device_text(id, tx.guid.as_deref(), codes, &msg);
                    }
                }
                continue;
            }
            world.add_device_text(
                id,
                None,
                &msg,
                rgb,
                tx.guid.as_deref(),
                tx.codes.as_deref(),
                dist,
            );
        }
    }

    fn distribute_to_player(
        &mut self,
        index: usize,
        player: &PlayerView,
        tx: &Transmission,
        world: &mut dyn RadioWorld,
        rng: &mut Rng<'_>,
    ) {
        let Some((id, radio)) = player.equipped_radio else {
            return;
        };
        if !(radio.portable && radio.turned_on && radio.channel == tx.channel) {
            return;
        }
        if radio.volume <= 0.0 || radio.playing_media || radio.no_transmit {
            return;
        }
        let mut pass = false;
        let mut dist = -1;
        if tx.signal_strength < 0 {
            pass = true;
        } else {
            dist = distance(player.x as i32, player.y as i32, tx.x, tx.y);
            if dist > 3 && dist < tx.signal_strength {
                pass = true;
            }
        }
        if !pass {
            return;
        }
        let mut msg = tx.msg.clone();
        if tx.signal_strength > 0 {
            self.has_applied_range_distortion = false;
            msg = self.device_range_distortion(msg, tx.signal_strength, dist, rng);
        }
        if !self.has_applied_range_distortion {
            world.add_device_text(
                id,
                Some(index),
                &msg,
                [tx.r, tx.g, tx.b],
                tx.guid.as_deref(),
                tx.codes.as_deref(),
                dist,
            );
        } else {
            world.add_device_text(
                id,
                None,
                &msg,
                [0.5, 0.5, 0.5],
                tx.guid.as_deref(),
                tx.codes.as_deref(),
                dist,
            );
        }
    }
}

/// `ZomboidRadio`.
#[derive(Clone, Debug)]
pub struct ZomboidRadio {
    air: Airwaves,
    script_manager: Option<RadioScriptManager>,
    days_since_start: i32,
    last_recorded_hour: i32,
    player_last_line: Vec<Option<String>>,
    names: ChannelNames,
    has_recieved_server_data: bool,
    recorded_media: RecordedMedia,
    last_save_file: Option<PathBuf>,
    last_save_content: Option<String>,
    freqlist: JavaHashMap<i32, FreqListEntry>,
    /// `ZomboidRadio.postRadioSilence` (static in the original).
    pub post_radio_silence: bool,
    /// `ZomboidRadio.disableBroadcasting` (static in the original).
    pub disable_broadcasting: bool,
    /// `ZomboidRadio.louisvilleObfuscation` (static in the original: it
    /// outlives `Reset`, so the owner carries it over to a new instance to
    /// be exact).
    pub louisville_obfuscation: bool,
}

impl ZomboidRadio {
    /// `new ZomboidRadio()` (`getInstance`): `hour` is `GameTime.getHour()`.
    pub fn new(hour: i32) -> Self {
        ZomboidRadio {
            air: Airwaves::default(),
            script_manager: None,
            days_since_start: 0,
            last_recorded_hour: hour,
            player_last_line: vec![None; 4],
            names: ChannelNames::default(),
            has_recieved_server_data: false,
            recorded_media: RecordedMedia::new(),
            last_save_file: None,
            last_save_content: None,
            freqlist: JavaHashMap::new(),
            post_radio_silence: false,
            disable_broadcasting: false,
            louisville_obfuscation: false,
        }
    }

    /// `getGameMode` (from `GameClient.client` / `GameServer.server`).
    pub fn game_mode(&self) -> GameMode {
        self.air.mode
    }

    pub fn set_game_mode(&mut self, mode: GameMode) {
        self.air.mode = mode;
    }

    pub fn script_manager(&self) -> Option<&RadioScriptManager> {
        self.script_manager.as_ref()
    }

    pub fn script_manager_mut(&mut self) -> Option<&mut RadioScriptManager> {
        self.script_manager.as_mut()
    }

    pub fn days_since_start(&self) -> i32 {
        self.days_since_start
    }

    pub fn last_recorded_hour(&self) -> i32 {
        self.last_recorded_hour
    }

    pub fn devices(&self) -> &[DeviceId] {
        &self.air.devices
    }

    pub fn broadcast_devices(&self) -> &[DeviceId] {
        &self.air.broadcast_devices
    }

    pub fn airwaves(&mut self) -> &mut Airwaves {
        &mut self.air
    }

    pub fn channel_names(&self) -> &ChannelNames {
        &self.names
    }

    pub fn recorded_media(&self) -> &RecordedMedia {
        &self.recorded_media
    }

    pub fn recorded_media_mut(&mut self) -> &mut RecordedMedia {
        &mut self.recorded_media
    }

    pub fn set_has_recieved_server_data(&mut self, state: bool) {
        self.has_recieved_server_data = state;
    }

    /// `addChannelName(name, frequency, category, overwrite)`.
    pub fn add_channel_name(
        &mut self,
        name: &str,
        frequency: i32,
        category: &str,
        overwrite: bool,
    ) {
        self.names.add(name, frequency, category, overwrite);
    }

    /// `removeChannelName(frequency)`.
    pub fn remove_channel_name(&mut self, frequency: i32) {
        self.names.remove(frequency);
    }

    /// `GetChannelList(category)`.
    pub fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.names.channel_list(category)
    }

    /// `getChannelName(frequency)`.
    pub fn channel_name(&self, frequency: i32) -> Option<&str> {
        self.names.channel_name(frequency)
    }

    /// `getFullChannelList`.
    pub fn full_channel_list(&self) -> Vec<(String, Vec<(i32, String)>)> {
        self.names.full_channel_list()
    }

    /// `getRandomFrequency()`.
    pub fn random_frequency(&self, rng: &mut Rng<'_>) -> i32 {
        self.random_frequency_in(88000, 108000, rng)
    }

    /// `getRandomFrequency(min, max)`.
    pub fn random_frequency_in(&self, min: i32, max: i32, rng: &mut Rng<'_>) -> i32 {
        self.names.random_frequency(min, max, rng)
    }

    /// `RadioScriptManager.AddChannel` through this radio's name registry.
    /// `false` without a script manager or when refused.
    pub fn add_channel(&mut self, channel: RadioChannel, overwrite: bool) -> bool {
        match self.script_manager.as_mut() {
            Some(m) => m.add_channel(channel, overwrite, &mut self.names),
            None => false,
        }
    }

    /// `RadioScriptManager.RemoveChannel`.
    pub fn remove_channel(&mut self, frequency: i32) {
        if let Some(m) = self.script_manager.as_mut() {
            m.remove_channel(frequency, &mut self.names);
        }
    }

    /// `ObfuscateChannelCheck`.
    pub fn obfuscate_channel_check(channel: &mut RadioChannel) {
        if channel.is_vanilla() && OBFUSCATE_CHANNELS.contains(&channel.frequency()) {
            channel.set_louisville_obfuscate(true);
        }
    }

    fn radio_data_dir(world: &dyn RadioWorld) -> Option<PathBuf> {
        world
            .current_save_dir()
            .map(|d| d.join("radio").join("data"))
    }

    /// `Init(savedWorldVersion)` (`-1` for a new game): loads the recorded
    /// media, the radio files and the mods' channels, then either starts
    /// the scripts `timeSinceApo` months in or restores the save. `Err` is
    /// the original's caught exception: the radio is left as far as it got.
    pub fn init(
        &mut self,
        saved_world_version: i32,
        source: &RadioSource,
        text: &dyn RadioText,
        world: &mut dyn RadioWorld,
        rng: &mut Rng<'_>,
    ) -> Result<(), RadioError> {
        self.post_radio_silence = false;
        let save_dir = world.current_save_dir();
        self.recorded_media.init(save_dir.as_deref(), &mut |m| {
            world.on_init_recorded_media(m)
        });
        self.last_recorded_hour = world.hour();
        if self.air.mode == GameMode::Client {
            world.net(RadioNet::ServerDataRequest);
            self.script_manager = None;
            return Ok(());
        }
        let manager = self
            .script_manager
            .get_or_insert_with(RadioScriptManager::new);
        if let Some(d) = Self::radio_data_dir(world) {
            let _ = std::fs::create_dir_all(d);
        }
        let list = RadioData::fetch_all_radio_data(source, text, &mut manager.broadcasts, rng)?;
        for mut rd in list {
            for mut channel in rd.take_channels() {
                Self::obfuscate_channel_check(&mut channel);
                let add = match manager.channels().get(&channel.frequency()) {
                    None => true,
                    Some(found) => {
                        let found_vanilla =
                            found.radio_data().map(|r| r.vanilla).ok_or_else(|| {
                                RadioError(format!(
                                    "channel {} has no radio data",
                                    found.frequency()
                                ))
                            })?;
                        found_vanilla && !channel.is_vanilla()
                    }
                };
                if add {
                    manager.add_channel(channel, true, &mut self.names);
                }
            }
        }
        world.on_load_radio_scripts(self, saved_world_version == -1);
        let manager = self
            .script_manager
            .get_or_insert_with(RadioScriptManager::new);
        let months = (world.time_since_apo() - 1).max(0);
        if saved_world_version == -1 {
            if months > 0 {
                self.days_since_start = (months as f32 * 30.5) as i32;
                manager.simulate_scripts_until(self.days_since_start, true, rng);
            }
            self.check_game_mode_specific_start(&world.core_game_mode(), rng)?;
        } else {
            if !self.load(world, rng) {
                self.days_since_start = (months as f32 * 30.5) as i32 + world.nights_survived();
            }
            if self.days_since_start > 0 {
                let days = self.days_since_start;
                if let Some(m) = self.script_manager.as_mut() {
                    m.simulate_scripts_until(days, false, rng);
                }
            }
        }
        Ok(())
    }

    /// `checkGameModeSpecificStart`: "Initial Infection" opens with each
    /// channel's `init_infection` script (then its normal one), "Six Months
    /// Later" with the numbers station and the pastor.
    fn check_game_mode_specific_start(
        &mut self,
        game_mode: &str,
        rng: &mut Rng<'_>,
    ) -> Result<(), RadioError> {
        let days = self.days_since_start;
        let Some(manager) = self.script_manager.as_mut() else {
            return Ok(());
        };
        let no_script = |c: &RadioChannel| {
            RadioError(format!("channel {} has no current script", c.frequency()))
        };
        let (channels, _) = manager.parts_mut();
        if game_mode == "Initial Infection" {
            for c in channels.values_mut() {
                if c.radio_script("init_infection").is_some() {
                    let current = c
                        .current_script_name()
                        .ok_or_else(|| no_script(c))?
                        .to_string();
                    let s = c.radio_script_mut("init_infection").unwrap();
                    s.clear_exit_options();
                    s.add_exit_option(&current, 100, 0);
                    c.set_active_script("init_infection", days, 1, -1, rng);
                } else {
                    let err = no_script(c);
                    c.current_script_mut()
                        .ok_or(err)?
                        .set_start_day_stamp(days + 1);
                }
            }
        } else if game_mode == "Six Months Later" {
            for c in channels.values_mut() {
                if c.name() == "Classified M1A1" {
                    c.set_active_script("numbers", days, 1, -1, rng);
                } else if c.name() == "NNR Radio" {
                    c.set_active_script("pastor", days, 1, -1, rng);
                }
            }
        }
        Ok(())
    }

    /// The text `Save` writes to `RADIO_SAVE.txt`.
    pub fn save_content(&self) -> Option<String> {
        let manager = self.script_manager.as_ref()?;
        let nl = line_separator();
        let mut s = format!(
            "DaysSinceStart = {}{nl}LvObfuscation = {}{nl}",
            self.days_since_start, self.louisville_obfuscation
        );
        manager.save(&mut s);
        Some(s)
    }

    /// `Save()`: writes `RADIO_SAVE.txt` when it changed (an unchanged
    /// radio skips the recorded media too, as in the original), then
    /// `recorded_media.bin`.
    pub fn save(&mut self, world: &dyn RadioWorld) {
        let Some(save_dir) = world.current_save_dir() else {
            return;
        };
        let dir = save_dir.join("radio").join("data");
        if matches!(self.air.mode, GameMode::Server | GameMode::SinglePlayer) && dir.is_dir() {
            if let Some(content) = self.save_content() {
                let file = dir.join(RADIO_SAVE_FILE);
                if self.last_save_file.as_ref() == Some(&file)
                    && self.last_save_content.as_ref() == Some(&content)
                {
                    return;
                }
                let _ = std::fs::write(&file, &content);
                self.last_save_file = Some(file);
                self.last_save_content = Some(content);
            }
        }
        self.recorded_media.save(Some(&save_dir));
    }

    /// `Load()`: `false` without a save file or when it is malformed (the
    /// channels then keep what was restored before the bad line). Every
    /// channel's script is cleared first, even without a file.
    pub fn load(&mut self, world: &dyn RadioWorld, rng: &mut Rng<'_>) -> bool {
        if !matches!(self.air.mode, GameMode::Server | GameMode::SinglePlayer) {
            return false;
        }
        let Some(manager) = self.script_manager.as_mut() else {
            return false;
        };
        let (channels, _) = manager.parts_mut();
        for c in channels.values_mut() {
            c.set_active_script_null();
        }
        let Some(dir) = Self::radio_data_dir(world) else {
            return false;
        };
        let Ok(bytes) = std::fs::read(dir.join(RADIO_SAVE_FILE)) else {
            return false;
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut channel_lines = Vec::new();
        for line in read_lines(&text) {
            let line = trim(line);
            if line.starts_with("DaysSinceStart") {
                let Some(v) = split_char(line, '=')
                    .get(1)
                    .and_then(|v| parse_int(trim(v)))
                else {
                    return false;
                };
                self.days_since_start = v;
            } else if line.starts_with("LvObfuscation") {
                let Some(v) = split_char(line, '=')
                    .get(1)
                    .map(|v| trim(v).eq_ignore_ascii_case("true"))
                else {
                    return false;
                };
                self.louisville_obfuscation = v;
            } else {
                channel_lines.push(line.to_string());
            }
        }
        manager.load(&channel_lines, rng).is_ok()
    }

    /// `UpdateScripts(hour, mins)`, called by `GameTime` every ten game
    /// minutes: counts the days, advances the scripts, saves, and lets the
    /// devices near a player start a due broadcast.
    pub fn update_scripts(
        &mut self,
        hour: i32,
        mins: i32,
        world: &mut dyn RadioWorld,
        rng: &mut Rng<'_>,
    ) {
        let mode = self.air.mode;
        if matches!(mode, GameMode::Server | GameMode::SinglePlayer) {
            if hour == 0 && self.last_recorded_hour != 0 {
                self.days_since_start += 1;
            }
            self.last_recorded_hour = hour;
            let day = self.days_since_start;
            if let Some(m) = self.script_manager.as_mut() {
                m.update_scripts(day, hour, mins, rng);
            }
            self.save(world);
        }
        if matches!(mode, GameMode::Client | GameMode::SinglePlayer) {
            for i in 0..self.air.devices.len() {
                let Some(d) = world.device(self.air.devices[i]) else {
                    continue;
                };
                if d.turned_on && d.has_player_in_range {
                    // `DeviceData.TriggerPlayerListening`.
                    self.player_listens_channel(d.channel, true, d.television, world);
                }
            }
        }
        if mode == GameMode::Client && !self.has_recieved_server_data {
            world.net(RadioNet::ServerDataRequest);
        }
    }

    /// `update()`, every frame: the Louisville and post-silence flags, the
    /// channels' airing, and what the players say into two-way radios.
    pub fn update(&mut self, world: &mut dyn RadioWorld, rng: &mut Rng<'_>) {
        let mode = self.air.mode;
        if mode == GameMode::SinglePlayer && world.local_player_y().is_some_and(|y| y < 3550.0) {
            self.louisville_obfuscation = true;
        }
        if matches!(mode, GameMode::Server | GameMode::SinglePlayer)
            && self.days_since_start > 14
            && !self.post_radio_silence
        {
            self.post_radio_silence = true;
            if mode == GameMode::Server {
                world.net(RadioNet::PostSilence);
            }
        }
        if matches!(mode, GameMode::Server | GameMode::SinglePlayer) {
            if let Some(m) = self.script_manager.as_mut() {
                let multiplier = world.multiplier();
                let (disabled, lv) = (self.disable_broadcasting, self.louisville_obfuscation);
                let air = &mut self.air;
                m.update(multiplier, &mut |line: AiredLine| {
                    if disabled {
                        return;
                    }
                    let tx = if line.louisville_obfuscate && lv {
                        let text = air.scramble_string(&line.text, 85, true, None, rng);
                        Transmission {
                            source: -1,
                            x: 0,
                            y: 0,
                            channel: line.frequency,
                            msg: text,
                            guid: None,
                            codes: Some(String::new()),
                            r: 0.7,
                            g: 0.5,
                            b: 0.5,
                            signal_strength: -1,
                            is_tv: line.is_tv,
                        }
                    } else {
                        Transmission {
                            source: -1,
                            x: 0,
                            y: 0,
                            channel: line.frequency,
                            msg: line.text,
                            guid: None,
                            codes: Some(line.codes),
                            r: line.r,
                            g: line.g,
                            b: line.b,
                            signal_strength: -1,
                            is_tv: line.is_tv,
                        }
                    };
                    air.send_transmission(tx, world, rng);
                });
            }
        }
        if matches!(mode, GameMode::SinglePlayer | GameMode::Client) {
            self.relay_player_speech(world, rng);
        }
    }

    /// The speech part of `update`: a new line a player says goes out on
    /// every two-way radio in mic range (the strongest transmitter per
    /// channel). Only the single-player path is ported; the client's staff
    /// and invisibility checks belong with multiplayer.
    fn relay_player_speech(&mut self, world: &mut dyn RadioWorld, rng: &mut Rng<'_>) {
        if self.air.mode != GameMode::SinglePlayer {
            return;
        }
        let n = world.num_players();
        if self.player_last_line.len() < n {
            self.player_last_line.resize(n, None);
        }
        for i in 0..n {
            let Some(player) = world.player(i) else {
                continue;
            };
            let Some(line) = player.last_spoken_line.clone() else {
                continue;
            };
            if self.player_last_line[i].as_ref() == Some(&line) {
                continue;
            }
            self.player_last_line[i] = Some(line.clone());
            self.freqlist.clear();
            for j in 0..n {
                if let Some(other) = world.player(j) {
                    self.check_player_for_device(&other, &player, j == i);
                }
            }
            for k in 0..self.air.broadcast_devices.len() {
                let Some(d) = world.device(self.air.broadcast_devices[k]) else {
                    continue;
                };
                if !d.turned_on || !d.two_way || !d.has_player_in_range || d.mic_muted {
                    continue;
                }
                let dist = distance(
                    fastfloor(player.x),
                    fastfloor(player.y),
                    fastfloor(d.x),
                    fastfloor(d.y),
                );
                if dist >= d.mic_range {
                    continue;
                }
                self.add_frequency_list_entry(&d, fastfloor(d.x), fastfloor(d.y));
            }
            if self.freqlist.is_empty() {
                continue;
            }
            let [r, g, b] = player.speak_colour;
            let entries: Vec<(i32, FreqListEntry)> = self
                .freqlist
                .iter()
                .into_iter()
                .map(|(k, v)| (*k, *v))
                .collect();
            for (channel, e) in entries {
                let tx = Transmission {
                    source: -1,
                    x: e.source_x,
                    y: e.source_y,
                    channel,
                    msg: line.clone(),
                    guid: None,
                    codes: None,
                    r,
                    g,
                    b,
                    signal_strength: e.transmit_range,
                    is_tv: false,
                };
                self.air.send_transmission(tx, world, rng);
            }
        }
    }

    fn add_frequency_list_entry(&mut self, d: &DeviceView, x: i32, y: i32) {
        let entry = FreqListEntry {
            transmit_range: d.transmit_range,
            source_x: x,
            source_y: y,
        };
        match self.freqlist.get_mut(&d.channel) {
            None => self.freqlist.insert(d.channel, entry),
            Some(e) if e.transmit_range < d.transmit_range => *e = entry,
            Some(_) => {}
        }
    }

    fn check_player_for_device(&mut self, plr: &PlayerView, speaker: &PlayerView, is_self: bool) {
        let Some((_, d)) = plr.equipped_radio else {
            return;
        };
        if !(d.portable && d.two_way && d.turned_on && !d.mic_muted) {
            return;
        }
        let near = is_self
            || distance(
                fastfloor(speaker.x),
                fastfloor(speaker.y),
                fastfloor(plr.x),
                fastfloor(plr.y),
            ) < d.mic_range;
        if near {
            self.add_frequency_list_entry(&d, fastfloor(plr.x), fastfloor(plr.y));
        }
    }

    /// `SendTransmission(source, x, y, channel, msg, guid, codes, r, g, b,
    /// signalStrength, isTV)`.
    pub fn send_transmission(
        &mut self,
        tx: Transmission,
        world: &mut dyn RadioWorld,
        rng: &mut Rng<'_>,
    ) {
        self.air.send_transmission(tx, world, rng);
    }

    /// `scrambleString(msg, intensity, ignoreBBcode, customScramble)`.
    pub fn scramble_string(
        &mut self,
        msg: &str,
        intensity: i32,
        ignore_bbcode: bool,
        custom: Option<&str>,
        rng: &mut Rng<'_>,
    ) -> String {
        self.air
            .scramble_string(msg, intensity, ignore_bbcode, custom, rng)
    }

    /// `PlayerListensChannel(channel, listening, isTV)`.
    pub fn player_listens_channel(
        &mut self,
        channel: i32,
        listening: bool,
        is_tv: bool,
        world: &mut dyn RadioWorld,
    ) {
        match self.air.mode {
            GameMode::SinglePlayer | GameMode::Server => {
                if let Some(m) = self.script_manager.as_mut() {
                    m.player_listens_channel(channel, listening, is_tv);
                }
            }
            GameMode::Client => world.net(RadioNet::PlayerListensChannel {
                channel,
                listening,
                is_tv,
            }),
        }
    }

    /// `RegisterDevice`: two-way devices also relay speech (not on a
    /// server).
    pub fn register_device(&mut self, id: DeviceId, two_way: bool) {
        if !self.air.devices.contains(&id) {
            self.air.devices.push(id);
        }
        if self.air.mode != GameMode::Server && two_way && !self.air.broadcast_devices.contains(&id)
        {
            self.air.broadcast_devices.push(id);
        }
    }

    /// `UnRegisterDevice`.
    pub fn unregister_device(&mut self, id: DeviceId, two_way: bool) {
        self.air.devices.retain(|d| *d != id);
        if self.air.mode != GameMode::Server && two_way {
            self.air.broadcast_devices.retain(|d| *d != id);
        }
    }

    /// `computerize(str)`: every letter in random case.
    pub fn computerize(s: &str, rng: &mut Rng<'_>) -> String {
        s.chars()
            .map(|c| {
                if (c as u32) < 0x10000 && c.is_alphabetic() {
                    if rand_bool(rng, 2) {
                        simple_lower(c)
                    } else {
                        simple_upper(c)
                    }
                } else {
                    c
                }
            })
            .collect()
    }
}

impl pz_inventory::device::ChannelLists for ZomboidRadio {
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.names.channel_list(category)
    }
}

impl pz_inventory::device::ChannelLists for ChannelNames {
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        ChannelNames::channel_list(self, category)
    }
}
