//! `zombie.SoundManager` (the FMOD one): UI sounds, the combined music
//! event and its global parameters, the music played by name, pause and
//! resume, and the volume VCAs.
//!
//! World sounds (`PlaySound`, `PlayWorldSound*`, night ambients, impact
//! sounds) take emitters from `IsoWorld` and arrive with the world port.

use crate::backend::{AudioBackend, ChannelHandle, InstanceHandle, PlaybackState, SoundHandle};
use crate::emitter::{Ctx, FmodSoundEmitter, ParameterUpdater, ParentObject};
use crate::game_sounds::{effective_volume, SoundRef};
use crate::params::{FmodParameter, FmodParameterList};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// What the music parameters read from the game.
#[derive(Debug, Clone, Default)]
pub struct MusicInputs {
    /// `Core.getOptionMusicActionStyle()`.
    pub music_action_style: i32,
    /// `Core.getOptionMusicLibrary()`.
    pub music_library: i32,
    /// `Core.getOptionMusicVolume()`.
    pub music_volume_option: i32,
    /// `IsoPlayer.players` (only what the music parameters read).
    pub players: Vec<Option<MusicPlayer>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MusicPlayer {
    pub dead: bool,
    pub asleep: bool,
    /// `getMusicIntensityEvents().getIntensity()`.
    pub intensity: f32,
    /// `getMusicThreatStatuses().getIntensity()`.
    pub threat: f32,
    /// Sum of the four `Stats.musicZombiesTargeting*` counters.
    pub zombies_targeting: i32,
    /// `Stats.musicZombiesVisible`.
    pub zombies_visible: i32,
}

/// `ParameterMusicState.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicState {
    MainMenu = 0,
    Loading = 1,
    InGame = 2,
    PauseMenu = 3,
    Tutorial = 4,
}

/// `ParameterMusicWakeState.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeState {
    Awake = 0,
    Sleeping = 1,
    WakeNormal = 2,
    WakeNightmare = 3,
    WakeZombies = 4,
}

/// The `choosePlayer()` logic the player-based music parameters share.
#[derive(Default)]
struct PlayerChooser {
    index: Cell<i32>,
}

impl PlayerChooser {
    /// Returns the chosen player and whether a new player was just chosen.
    fn choose(&self, players: &[Option<MusicPlayer>]) -> (Option<MusicPlayer>, bool) {
        let alive = |i: i32| {
            players
                .get(i as usize)
                .and_then(|p| p.clone())
                .filter(|p| !p.dead)
        };
        if self.index.get() != -1 && alive(self.index.get()).is_none() {
            self.index.set(-1);
        }
        if self.index.get() != -1 {
            return (alive(self.index.get()), false);
        }
        for i in 0..players.len() as i32 {
            if let Some(p) = alive(i) {
                self.index.set(i);
                return (Some(p), true);
            }
        }
        (None, false)
    }
}

/// Shared state of the music parameters.
pub struct MusicParamState {
    pub inputs: RefCell<MusicInputs>,
    pub allow_music: Cell<bool>,
    pub state: Cell<MusicState>,
    wake: Cell<WakeState>,
    wake_chooser: PlayerChooser,
}

fn level(v: f32) -> f32 {
    if v < 34.0 {
        0.0
    } else if v < 67.0 {
        1.0
    } else {
        2.0
    }
}

fn music_parameters(st: &Rc<MusicParamState>, backend: &mut dyn AudioBackend) -> FmodParameterList {
    let mut list = FmodParameterList::default();
    let mut add = |name: &str, f: Box<dyn FnMut() -> f32>| {
        let d = backend.parameter_description(name);
        list.add(FmodParameter::global(name, d, Box::new(f)));
    };
    let s = st.clone();
    add(
        "MusicActionStyle",
        Box::new(move || {
            if s.inputs.borrow().music_action_style == 2 {
                1.0
            } else {
                0.0
            }
        }),
    );
    let (s, c) = (
        st.clone(),
        PlayerChooser {
            index: Cell::new(-1),
        },
    );
    add(
        "MusicIntensity",
        Box::new(move || {
            c.choose(&s.inputs.borrow().players)
                .0
                .map_or(0.0, |p| level(p.intensity))
        }),
    );
    let (s, c) = (
        st.clone(),
        PlayerChooser {
            index: Cell::new(-1),
        },
    );
    add(
        "MusicThreat",
        Box::new(move || {
            c.choose(&s.inputs.borrow().players)
                .0
                .map_or(0.0, |p| level(p.threat))
        }),
    );
    let s = st.clone();
    add(
        "MusicLibrary",
        Box::new(move || match s.inputs.borrow().music_library {
            2 => 1.0,
            3 => 2.0,
            _ => 0.0,
        }),
    );
    let s = st.clone();
    add("MusicState", Box::new(move || s.state.get() as i32 as f32));
    let s = st.clone();
    add(
        "MusicToggleMute",
        Box::new(move || if s.allow_music.get() { 0.0 } else { 1.0 }),
    );
    let s = st.clone();
    add(
        "MusicWakeState",
        Box::new(move || {
            let (p, fresh) = s.wake_chooser.choose(&s.inputs.borrow().players);
            if let Some(p) = &p {
                if fresh {
                    s.wake.set(if p.asleep {
                        WakeState::Sleeping
                    } else {
                        WakeState::Awake
                    });
                }
                if s.wake.get() == WakeState::Awake && p.asleep {
                    s.wake.set(WakeState::Sleeping);
                }
            }
            s.wake.get() as i32 as f32
        }),
    );
    let (s, c) = (
        st.clone(),
        PlayerChooser {
            index: Cell::new(-1),
        },
    );
    add(
        "MusicZombiesTargeting",
        Box::new(move || {
            c.choose(&s.inputs.borrow().players)
                .0
                .map_or(0.0, |p| p.zombies_targeting.clamp(0, 50) as f32)
        }),
    );
    let (s, c) = (
        st.clone(),
        PlayerChooser {
            index: Cell::new(-1),
        },
    );
    add(
        "MusicZombiesVisible",
        Box::new(move || {
            c.choose(&s.inputs.borrow().players)
                .0
                .map_or(0.0, |p| p.zombies_visible.clamp(0, 50) as f32)
        }),
    );
    list
}

/// SoundManager as the music emitter's parameter updater: attaches the
/// music parameters the emitter did not handle itself.
struct MusicUpdater {
    list: Rc<RefCell<FmodParameterList>>,
}

impl ParameterUpdater for MusicUpdater {
    fn start_event(
        &mut self,
        inst: InstanceHandle,
        _sound: &SoundRef,
        _clip: usize,
        event_params: &[usize],
        set: &[bool],
        backend: &mut dyn AudioBackend,
    ) {
        for &gi in event_params {
            if set.get(gi).copied().unwrap_or(false) {
                continue;
            }
            let mut list = self.list.borrow_mut();
            if let Some(p) = list
                .parameters
                .iter_mut()
                .rev()
                .find(|p| p.description().is_some_and(|d| d.global_index == gi))
            {
                p.start_event_instance(inst, backend);
            }
        }
    }

    fn update_event(
        &mut self,
        _inst: InstanceHandle,
        _sound: &SoundRef,
        _clip: usize,
        _backend: &mut dyn AudioBackend,
    ) {
    }

    fn stop_event(
        &mut self,
        inst: InstanceHandle,
        _sound: &SoundRef,
        _clip: usize,
        event_params: &[usize],
        set: &[bool],
        _backend: &mut dyn AudioBackend,
    ) {
        for &gi in event_params {
            if set.get(gi).copied().unwrap_or(false) {
                continue;
            }
            let mut list = self.list.borrow_mut();
            if let Some(p) = list
                .parameters
                .iter_mut()
                .rev()
                .find(|p| p.description().is_some_and(|d| d.global_index == gi))
            {
                p.stop_event_instance(inst);
            }
        }
    }
}

/// `SoundManager.Music`: music played by `DoMusic`.
#[derive(Default)]
struct Music {
    sound: Option<(SoundRef, usize)>,
    instance: InstanceHandle,
    channel: ChannelHandle,
    file_sound: SoundHandle,
    effective_volume: f32,
}

pub struct SoundManager {
    pub sound_volume: f32,
    pub music_volume: f32,
    pub ambient_volume: f32,
    pub vehicle_engine_volume: f32,
    params: Rc<MusicParamState>,
    music_params: Option<Rc<RefCell<FmodParameterList>>>,
    initialized: bool,
    in_game_group_bus: u64,
    music_group_bus: u64,
    music_emitter: Option<FmodSoundEmitter>,
    music_combined_event: i64,
    ui_emitter: Option<FmodSoundEmitter>,
    ui_sound_muted: bool,
    muted: bool,
    music: Music,
    current_music_name: Option<String>,
    current_music_library: Option<String>,
    /// `MainScreenState.lightningTimelineMarker`, set by the music event's
    /// "Lightning" marker.
    pub lightning_timeline_marker: bool,
    /// `FMODManager.getParameterCount()`.
    parameter_count: usize,
}

impl SoundManager {
    pub fn new(parameter_count: usize) -> Self {
        SoundManager {
            sound_volume: 0.8,
            music_volume: 0.36,
            ambient_volume: 0.8,
            vehicle_engine_volume: 0.5,
            params: Rc::new(MusicParamState {
                inputs: RefCell::new(MusicInputs::default()),
                allow_music: Cell::new(true),
                state: Cell::new(MusicState::MainMenu),
                wake: Cell::new(WakeState::Awake),
                wake_chooser: PlayerChooser {
                    index: Cell::new(-1),
                },
            }),
            music_params: None,
            initialized: false,
            in_game_group_bus: 0,
            music_group_bus: 0,
            music_emitter: None,
            music_combined_event: 0,
            ui_emitter: None,
            ui_sound_muted: false,
            muted: false,
            music: Music::default(),
            current_music_name: None,
            current_music_library: None,
            lightning_timeline_marker: false,
            parameter_count,
        }
    }

    /// Inputs the music parameters read (Core options, players); set each
    /// frame before [`SoundManager::update`].
    pub fn set_music_inputs(&self, inputs: MusicInputs) {
        *self.params.inputs.borrow_mut() = inputs;
    }

    pub fn allow_music(&self) -> bool {
        self.params.allow_music.get()
    }

    pub fn set_allow_music(&self, allow: bool) {
        self.params.allow_music.set(allow);
    }

    pub fn set_ui_sound_muted(&mut self, muted: bool) {
        self.ui_sound_muted = muted;
    }

    pub fn is_ui_sound_muted(&self) -> bool {
        self.ui_sound_muted
    }

    /// `isRemastered()`.
    pub fn is_remastered(&self, rand_next: &mut dyn FnMut(i32) -> i32) -> bool {
        let library = self.params.inputs.borrow().music_library;
        library == 1 || (library == 3 && rand_next(2) == 0)
    }

    /// `getUIEmitter()`.
    pub fn ui_emitter(&mut self) -> &mut FmodSoundEmitter {
        let n = self.parameter_count;
        self.ui_emitter
            .get_or_insert_with(|| FmodSoundEmitter::new(n))
    }

    /// `playUISound(name)`. `player_pos` is the position of
    /// `IsoPlayer.getInstance()` (or its reanimated corpse), if any.
    pub fn play_ui_sound(
        &mut self,
        name: &str,
        player_pos: Option<[f32; 3]>,
        ctx: &mut Ctx,
    ) -> i64 {
        if self.ui_sound_muted {
            return 0;
        }
        let exists = ctx.media_file_exists;
        let Some(gs) = ctx.sounds.get_or_create_sound(name, ctx.backend, exists) else {
            return 0;
        };
        let n = gs.borrow().game_sound.clips.len();
        if n == 0 {
            return 0;
        }
        let clip = (ctx.rand_next)(n as i32) as usize;
        let emitter = self.ui_emitter();
        let inst = emitter.play_clip(gs, clip, ParentObject::None, ctx);
        if inst != 0 {
            if let Some([x, y, z]) = player_pos {
                emitter.set_pos(x, y, z);
                for (p, v) in [
                    ("FootstepMaterial", 2.0),
                    ("FootstepMaterial2", 0.0),
                    ("Inside", 0.0),
                    ("RainIntensity", 0.0),
                ] {
                    emitter.set_parameter_value_by_name(inst, p, v, ctx);
                }
            }
        }
        emitter.tick(ctx);
        ctx.backend.system_update();
        inst
    }

    /// `isPlayingUISound(name)`.
    pub fn is_playing_ui_sound(&self, name: &str) -> bool {
        self.ui_emitter
            .as_ref()
            .is_some_and(|e| e.is_playing_name(name))
    }

    /// `isPlayingUISound(eventInstance)`.
    pub fn is_playing_ui_sound_instance(&self, inst: i64) -> bool {
        self.ui_emitter.as_ref().is_some_and(|e| e.is_playing(inst))
    }

    /// `stopUISound(eventInstance)`.
    pub fn stop_ui_sound(&mut self, inst: i64, ctx: &mut Ctx) {
        self.ui_emitter().stop_sound(inst, ctx);
    }

    /// `pauseSoundAndMusic(keepMusic)` on a single-player game.
    pub fn pause_sound_and_music(&mut self, keep_music: bool, client: bool, ctx: &mut Ctx) {
        if client {
            self.muted = true;
            ctx.backend.bus_set_mute(self.in_game_group_bus, true);
            ctx.backend.bus_set_mute(self.music_group_bus, !keep_music);
            ctx.sounds.sound_is_paused = true;
            return;
        }
        ctx.backend.bus_set_paused(self.in_game_group_bus, true);
        ctx.backend.non_bank_group_set_paused(true);
        ctx.sounds.sound_is_paused = true;
    }

    /// `resumeSoundAndMusic()`.
    pub fn resume_sound_and_music(&mut self, ctx: &mut Ctx) {
        if self.muted {
            self.muted = false;
            ctx.backend.bus_set_mute(self.in_game_group_bus, false);
            ctx.backend.bus_set_mute(self.music_group_bus, false);
            ctx.backend.non_bank_group_set_paused(false);
            ctx.sounds.sound_is_paused = false;
            return;
        }
        ctx.backend.bus_set_paused(self.in_game_group_bus, false);
        ctx.backend.non_bank_group_set_paused(false);
        ctx.sounds.sound_is_paused = false;
    }

    /// `setMusicState(name)`; unknown names are ignored with a warning.
    pub fn set_music_state(&self, name: &str) -> bool {
        let s = match name {
            "MainMenu" => MusicState::MainMenu,
            "Loading" => MusicState::Loading,
            "InGame" => MusicState::InGame,
            "PauseMenu" => MusicState::PauseMenu,
            "Tutorial" => MusicState::Tutorial,
            _ => return false,
        };
        self.params.state.set(s);
        true
    }

    /// `setMusicWakeState(player, name)` for the chosen player.
    pub fn set_music_wake_state(&self, player_index: i32, name: &str) -> bool {
        let s = match name {
            "Awake" => WakeState::Awake,
            "Sleeping" => WakeState::Sleeping,
            "WakeNormal" => WakeState::WakeNormal,
            "WakeNightmare" => WakeState::WakeNightmare,
            "WakeZombies" => WakeState::WakeZombies,
            _ => return false,
        };
        let players = self.params.inputs.borrow().players.clone();
        let (p, _) = self.params.wake_chooser.choose(&players);
        if p.is_some() && self.params.wake_chooser.index.get() == player_index {
            self.params.wake.set(s);
        }
        true
    }

    // --- music by name --------------------------------------------------

    fn music_is_playing(&mut self, backend: &mut dyn AudioBackend) -> bool {
        if self.music.instance != 0 {
            let s = backend.instance_playback_state(self.music.instance);
            return s != PlaybackState::Stopped && s != PlaybackState::Stopping;
        }
        self.music.channel != 0 && backend.channel_is_playing(self.music.channel)
    }

    /// `isPlayingMusic()`.
    pub fn is_playing_music(&mut self, backend: &mut dyn AudioBackend) -> bool {
        self.music_is_playing(backend)
    }

    /// `StopMusic()`.
    pub fn stop_music_all(&mut self, backend: &mut dyn AudioBackend) {
        if self.music.instance != 0 {
            backend.instance_stop(self.music.instance, false);
            backend.instance_release(self.music.instance);
            self.music.instance = 0;
        }
        if self.music.channel != 0 {
            backend.channel_stop(self.music.channel);
            self.music.channel = 0;
            backend.sound_release(self.music.file_sound);
            self.music.file_sound = 0;
        }
    }

    /// `stopMusic(name)`.
    pub fn stop_music(&mut self, name: Option<&str>, backend: &mut dyn AudioBackend) {
        if !self.music_is_playing(backend) {
            return;
        }
        let current = self.current_music_name(backend);
        let blank = name.is_none_or(|n| n.trim_matches(|c: char| c <= ' ').is_empty());
        if blank
            || current
                .as_deref()
                .is_some_and(|c| name.unwrap().eq_ignore_ascii_case(c))
        {
            self.stop_music_all(backend);
        }
    }

    /// `getCurrentMusicName()`.
    pub fn current_music_name(&mut self, backend: &mut dyn AudioBackend) -> Option<String> {
        if self.music_is_playing(backend) {
            self.current_music_name.clone()
        } else {
            None
        }
    }

    /// `getCurrentMusicLibrary()`.
    pub fn current_music_library(&mut self, backend: &mut dyn AudioBackend) -> Option<String> {
        if self.music_is_playing(backend) {
            self.current_music_library.clone()
        } else {
            None
        }
    }

    /// `getMusicPosition()` in ms.
    pub fn music_position(&mut self, backend: &mut dyn AudioBackend) -> f32 {
        if !self.music_is_playing(backend) {
            return 0.0;
        }
        if self.music.instance != 0 {
            return backend.instance_timeline_position(self.music.instance) as f32;
        }
        if self.music.channel != 0 {
            return backend.channel_position_ms(self.music.channel) as f32;
        }
        0.0
    }

    /// `playMusic(name)` = `DoMusic(name, false)`.
    pub fn play_music(&mut self, name: &str, ctx: &mut Ctx) {
        if !self.allow_music() || self.params.inputs.borrow().music_volume_option == 0 {
            return;
        }
        if self.music_is_playing(ctx.backend) {
            self.stop_music_all(ctx.backend);
        }
        let remastered = self.params.inputs.borrow().music_library == 1;
        let exists = ctx.media_file_exists;
        let gs = ctx.sounds.get_or_create_sound(name, ctx.backend, exists);
        let clip = gs.as_ref().and_then(|g| {
            let n = g.borrow().game_sound.clips.len();
            (n > 0).then(|| (ctx.rand_next)(n as i32) as usize)
        });
        if let (Some(gs), Some(clip)) = (gs, clip) {
            let (event, file, volume, pitch) = {
                let s = gs.borrow();
                let c = &s.game_sound.clips[clip];
                (
                    c.event.clone(),
                    c.file.clone(),
                    effective_volume(&ctx.sounds.env, &s.game_sound, c),
                    c.pitch,
                )
            };
            if event.is_some() {
                let desc = ctx
                    .sounds
                    .clip_events(&gs)
                    .get(clip)
                    .and_then(|e| e.description.clone());
                if let Some(desc) = desc {
                    ctx.backend.load_event_sample_data(&desc);
                    self.music.instance = ctx.backend.create_event_instance(&desc);
                    self.music.sound = Some((gs, clip));
                    self.music.effective_volume = volume;
                    ctx.backend
                        .instance_set_parameter_by_name(self.music.instance, "Volume", 10.0);
                    ctx.backend.instance_set_volume(self.music.instance, volume);
                    ctx.backend.instance_start(self.music.instance);
                }
            } else if let Some(file) = file {
                let snd = ctx.backend.load_sound(&file, false);
                if snd > 0 {
                    self.music.channel = ctx.backend.play_sound(snd, true);
                    self.music.sound = Some((gs, clip));
                    self.music.effective_volume = volume;
                    ctx.backend.channel_set_volume(self.music.channel, volume);
                    ctx.backend.channel_set_pitch(self.music.channel, pitch);
                    ctx.backend.channel_set_paused(self.music.channel, false);
                }
            }
        }
        self.current_music_name = Some(name.to_string());
        self.current_music_library = Some(
            if remastered {
                "official"
            } else {
                "earlyaccess"
            }
            .to_string(),
        );
    }

    fn update_music_volume(&mut self, ctx: &mut Ctx) {
        let Some((gs, clip)) = &self.music.sound else {
            return;
        };
        let target = {
            let s = gs.borrow();
            let Some(c) = s.game_sound.clips.get(*clip) else {
                return;
            };
            effective_volume(&ctx.sounds.env, &s.game_sound, c)
        };
        if self.music.effective_volume == target {
            return;
        }
        self.music.effective_volume = target;
        if self.music.instance != 0 {
            ctx.backend.instance_set_volume(self.music.instance, target);
        }
        if self.music.channel != 0 {
            ctx.backend.channel_set_volume(self.music.channel, target);
        }
    }

    // --- volumes ------------------------------------------------------

    fn set_vca(backend: &mut dyn AudioBackend, path: &str, v: f32) {
        let vca = backend.get_vca(path);
        if vca != 0 {
            backend.vca_set_volume(vca, v);
        }
    }

    pub fn set_sound_volume(&mut self, v: f32, backend: &mut dyn AudioBackend) {
        self.sound_volume = v;
        Self::set_vca(backend, "vca:/Settings_Sfx", v);
    }

    /// `setAmbientVolume`: the original forces 1.0.
    pub fn set_ambient_volume(&mut self, _v: f32, backend: &mut dyn AudioBackend) {
        self.ambient_volume = 1.0;
        Self::set_vca(backend, "vca:/Settings_Ambience", 1.0);
    }

    pub fn get_ambient_volume(&self) -> f32 {
        1.0
    }

    pub fn set_music_volume(&mut self, v: f32, backend: &mut dyn AudioBackend) {
        self.music_volume = v;
        Self::set_vca(backend, "vca:/Settings_Music", v);
    }

    pub fn set_vehicle_engine_volume(&mut self, v: f32, backend: &mut dyn AudioBackend) {
        self.vehicle_engine_volume = v;
        Self::set_vca(backend, "vca:/Settings_VehicleEngines", v);
    }

    // --- frame update -------------------------------------------------

    /// `Update()`. `toggle_music_pressed` is
    /// `GameKeyboard.isKeyPressed(KeybindId.TOGGLE_MUSIC)`.
    pub fn update(&mut self, toggle_music_pressed: bool, ctx: &mut Ctx) {
        if !self.initialized {
            self.initialized = true;
            self.in_game_group_bus = ctx.backend.get_bus("bus:/InGame");
            self.music_group_bus = ctx.backend.get_bus("bus:/Music");
            let list = Rc::new(RefCell::new(music_parameters(&self.params, ctx.backend)));
            let mut emitter = FmodSoundEmitter::new(self.parameter_count);
            emitter.parameter_updater = Some(Box::new(MusicUpdater { list: list.clone() }));
            self.music_emitter = Some(emitter);
            self.music_params = Some(list);
            let n = self.parameter_count;
            self.ui_emitter
                .get_or_insert_with(|| FmodSoundEmitter::new(n));
        }
        self.update_music(toggle_music_pressed, ctx);
        let ui = self.ui_emitter.as_mut().unwrap();
        ui.tick(ctx);
        if ui.is_empty() {
            ui.clear_parameters();
        }
        if !self.allow_music() {
            self.stop_music_all(ctx.backend);
        }
        if self.music_is_playing(ctx.backend) {
            self.update_music_volume(ctx);
        }
        for (_, name, _) in ctx.backend.take_timeline_markers() {
            if name == "Lightning" {
                self.lightning_timeline_marker = true;
            }
        }
    }

    fn update_music(&mut self, toggle_music_pressed: bool, ctx: &mut Ctx) {
        if let Some(list) = &self.music_params {
            list.borrow_mut().update(ctx.backend);
        }
        if toggle_music_pressed {
            let allow = !self.allow_music();
            self.set_allow_music(allow);
            if !allow {
                self.stop_music_all(ctx.backend);
            }
        }
        let allow = self.allow_music();
        let emitter = self.music_emitter.as_mut().unwrap();
        if !emitter.is_playing(self.music_combined_event) {
            self.music_combined_event =
                emitter.play_sound_impl("MusicCombined", ParentObject::None, ctx);
            if self.music_combined_event != 0 {
                ctx.backend
                    .instance_watch_timeline_markers(self.music_combined_event);
            }
        }
        if emitter.is_playing(self.music_combined_event) {
            emitter.set_volume(self.music_combined_event, if allow { 1.0 } else { 0.0 });
        }
        emitter.tick(ctx);
    }

    /// `stop()`: stops the manager's own emitters and the master group.
    pub fn stop(&mut self, ctx: &mut Ctx) {
        if let Some(e) = &mut self.ui_emitter {
            e.stop_all(ctx);
        }
        if let Some(e) = &mut self.music_emitter {
            e.stop_all(ctx);
        }
        ctx.backend.master_channel_group_stop();
    }

    /// The music state parameter's current value, for tests and bindings.
    pub fn music_state(&self) -> MusicState {
        self.params.state.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emitter::tests::{sounds, Mock, World};

    macro_rules! ctx {
        ($b:expr, $g:expr) => {
            Ctx {
                backend: &mut $b,
                sounds: &mut $g,
                world: &World,
                rand_next: &mut |_| 0,
                media_file_exists: &|_| false,
            }
        };
    }

    #[test]
    fn ui_sound_in_main_menu() {
        let mut b = Mock::default();
        let mut g = sounds(&[("UIClick", Some("UI/Click"), None)], &mut b);
        let mut m = SoundManager::new(8);
        b.calls.clear();
        let i = m.play_ui_sound("UIClick", None, &mut ctx!(b, g));
        assert_eq!(i, 1);
        assert_eq!(
            b.calls,
            // At (0, 0, 0) the position equals the initial one: no 3D call.
            [
                "create 1",
                "prop 1 4 30",
                "volume 1 0.5",
                "param 1 0 1",
                "start 1",
                "update"
            ]
        );
        assert!(m.is_playing_ui_sound("UIClick"));
        m.set_ui_sound_muted(true);
        assert_eq!(m.play_ui_sound("UIClick", None, &mut ctx!(b, g)), 0);
    }

    #[test]
    fn ui_sound_in_game_sets_ground_parameters() {
        let mut b = Mock::default();
        let mut g = sounds(&[("UIClick", Some("UI/Click"), None)], &mut b);
        let mut m = SoundManager::new(8);
        b.calls.clear();
        m.play_ui_sound("UIClick", Some([5.0, 6.0, 0.0]), &mut ctx!(b, g));
        // Values set before the start are applied by the start.
        assert_eq!(
            b.calls,
            [
                "create 1",
                "prop 1 4 30",
                "pos 1 5 6 0",
                "volume 1 0.5",
                "param 1 0 1",
                "param 1 4 2",
                "param 1 5 0",
                "param 1 6 0",
                "param 1 7 0",
                "start 1",
                "update"
            ]
        );
    }

    #[test]
    fn music_combined_starts_with_music_state() {
        let mut b = Mock::default();
        let mut g = sounds(&[("MusicCombined", Some("Music/Main"), None)], &mut b);
        let mut m = SoundManager::new(8);
        b.calls.clear();
        m.update(false, &mut ctx!(b, g));
        assert_eq!(
            b.calls[..3],
            ["bus bus:/InGame", "bus bus:/Music", "gparam 3 0"]
        );
        assert!(b.calls.contains(&"start 1".to_string()));
        assert!(m.set_music_state("PauseMenu"));
        assert!(!m.set_music_state("Nope"));
        b.calls.clear();
        m.update(false, &mut ctx!(b, g));
        assert!(b.calls.contains(&"gparam 3 3".to_string()), "{:?}", b.calls);
        // The toggle-music key mutes the combined event.
        b.calls.clear();
        m.update(true, &mut ctx!(b, g));
        assert!(!m.allow_music());
        assert!(b.calls.contains(&"volume 1 0".to_string()), "{:?}", b.calls);
    }
}
