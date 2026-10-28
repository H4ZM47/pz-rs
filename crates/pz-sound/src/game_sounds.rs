//! `zombie.GameSounds`: the registry of named game sounds (from `sound`
//! scripts), their FMOD event lookups, the per-sound user volumes saved in
//! `sounds.ini`, and the sound preview used by the debug sound browser.
//!
//! The original keeps static state; here it lives in a [`GameSounds`] value.
//! Script sounds are shared with the script manager (`Rc`), so a reloaded
//! script is recognised by identity as in Java.

use crate::backend::{
    AudioBackend, ChannelHandle, EventDescription, InstanceHandle, PlaybackState, MODE_LOOP_NORMAL,
};
use pz_core::config::{ConfigFile, ConfigOption};
use pz_scripts::objects::models::game_sound::{
    GameSound, GameSoundClip, GameSoundScript, MasterVolume,
};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;

pub const VERSION: i32 = 1;

pub type SoundRef = Rc<RefCell<GameSoundScript>>;

/// Engine-side facts the original reads from statics.
#[derive(Debug, Clone, Default)]
pub struct SoundEnv {
    /// `GameServer.server`.
    pub server: bool,
    /// `GameClient.client`.
    pub client: bool,
    /// `Core.debug`.
    pub debug: bool,
    /// `Core.soundDisabled`.
    pub sound_disabled: bool,
    /// `SystemDisabler.getEnableAdvancedSoundOptions()`.
    pub advanced_sound_options: bool,
    /// `ZomboidFileSystem.getCacheDir()` (never the user's real game folder
    /// in tests).
    pub cache_dir: PathBuf,
}

/// Event descriptions resolved for one clip.
#[derive(Debug, Clone, Default)]
pub struct ClipEvents {
    pub description: Option<Rc<EventDescription>>,
    pub description_mp: Option<Rc<EventDescription>>,
}

/// A player the 3D listener follows (`fix3DListenerPosition`).
#[derive(Debug, Clone, Copy)]
pub struct ListenerPlayer {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub deaf: bool,
}

enum Preview {
    Bank {
        instance: InstanceHandle,
        sound: SoundRef,
        clip: usize,
        gain: f32,
    },
    File {
        channel: ChannelHandle,
        sound: SoundRef,
        clip: usize,
        gain: f32,
    },
}

/// Logging sink for `DebugType.Sound` messages.
pub type Log = Box<dyn Fn(&str)>;

pub struct GameSounds {
    sounds: Vec<SoundRef>,
    by_name: HashMap<Option<String>, SoundRef>,
    /// Per sound (by pointer), the events of each clip.
    clip_events: HashMap<usize, Vec<ClipEvents>>,
    pub sound_is_paused: bool,
    preview: Option<Preview>,
    missing_event_count: i32,
    pub env: SoundEnv,
    log: Log,
}

fn key(s: &SoundRef) -> usize {
    Rc::as_ptr(s) as *const () as usize
}

/// `GameSound.getUserVolume()`.
pub fn user_volume(env: &SoundEnv, gs: &GameSound) -> f32 {
    if env.advanced_sound_options {
        gs.user_volume
    } else {
        1.0
    }
}

/// `GameSound.setUserVolume(gain)`.
pub fn set_user_volume(gs: &mut GameSound, gain: f32) {
    // Math.max(0, Math.min(2, gain)): NaN propagates through min/max.
    gs.user_volume = if gain.is_nan() {
        f32::NAN
    } else {
        gain.clamp(0.0, 2.0)
    };
}

/// `GameSoundClip.getEffectiveVolume()` (and `...InMenu`, identical).
pub fn effective_volume(env: &SoundEnv, gs: &GameSound, clip: &GameSoundClip) -> f32 {
    clip.volume * user_volume(env, gs)
}

/// `PZMath.tryParseFloat(s, default)` (`Float.parseFloat` also accepts a
/// trailing type suffix).
fn try_parse_float(s: &str, default: f32) -> f32 {
    let t = pz_core::config::java_trim(s);
    if t.is_empty() {
        return default;
    }
    let t = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    t.parse::<f32>().unwrap_or(default)
}

impl GameSounds {
    pub fn new(env: SoundEnv) -> GameSounds {
        GameSounds {
            sounds: Vec::new(),
            by_name: HashMap::new(),
            clip_events: HashMap::new(),
            sound_is_paused: false,
            preview: None,
            missing_event_count: 0,
            env,
            log: Box::new(|_| {}),
        }
    }

    pub fn set_log(&mut self, log: Log) {
        self.log = log;
    }

    pub fn sounds(&self) -> &[SoundRef] {
        &self.sounds
    }

    /// Event descriptions of a sound's clips (index-aligned with `clips`).
    pub fn clip_events(&self, sound: &SoundRef) -> &[ClipEvents] {
        self.clip_events.get(&key(sound)).map_or(&[], Vec::as_slice)
    }

    /// `addSound(sound)`.
    pub fn add_sound(&mut self, sound: SoundRef, backend: &mut dyn AudioBackend) {
        self.init_clip_events(&sound, backend);
        let name = sound.borrow().game_sound.name.clone();
        let mut index = self.sounds.len();
        if self.by_name.contains_key(&name) {
            index = self
                .sounds
                .iter()
                .position(|s| s.borrow().game_sound.name == name)
                .unwrap_or(self.sounds.len());
            if index < self.sounds.len() {
                self.sounds.remove(index);
            }
        }
        self.sounds
            .insert(index.min(self.sounds.len()), sound.clone());
        self.by_name.insert(name, sound);
    }

    /// `initClipEvents(sound)`: look up `event:/<event>` and
    /// `event:/Remote/<event>` for clips not resolved yet.
    fn init_clip_events(&mut self, sound: &SoundRef, backend: &mut dyn AudioBackend) {
        if self.env.server {
            return;
        }
        let s = sound.borrow();
        let events = self.clip_events.entry(key(sound)).or_default();
        events.resize_with(s.game_sound.clips.len(), ClipEvents::default);
        for (clip, ev) in s.game_sound.clips.iter().zip(events.iter_mut()) {
            let Some(event) = &clip.event else { continue };
            if ev.description.is_some() {
                continue;
            }
            ev.description = backend.event_description(&format!("event:/{event}"));
            if ev.description.is_none() {
                (self.log)(&format!(
                    "No such FMOD event \"{event}\" for GameSound \"{}\"",
                    s.game_sound.name.as_deref().unwrap_or("null")
                ));
                self.missing_event_count += 1;
            }
            ev.description_mp = backend.event_description(&format!("event:/Remote/{event}"));
            if let Some(mp) = &ev.description_mp {
                (self.log)(&format!("MP event {}", mp.path));
            }
        }
    }

    /// `isKnownSound(name)`.
    pub fn is_known_sound(&self, name: &str) -> bool {
        self.by_name.contains_key(&Some(name.to_string()))
    }

    /// `getSound(name)` = `getOrCreateSound(name)`. `media_file_exists`
    /// answers `ZomboidFileSystem.getAbsolutePath(path) != null`.
    pub fn get_or_create_sound(
        &mut self,
        name: &str,
        backend: &mut dyn AudioBackend,
        media_file_exists: &dyn Fn(&str) -> bool,
    ) -> Option<SoundRef> {
        if name.is_empty() {
            return None;
        }
        if let Some(s) = self.by_name.get(&Some(name.to_string())) {
            return Some(s.clone());
        }
        (self.log)(&format!("no GameSound called \"{name}\", adding a new one"));
        let mut script = GameSoundScript::new();
        script.game_sound.name = Some(name.to_string());
        script.game_sound.category = "AUTO".to_string();
        let mut clip = GameSoundClip::new(&script.game_sound);
        let mut events = ClipEvents::default();
        if let Some(desc) = backend.event_description(&format!("event:/{name}")) {
            clip.event = Some(name.to_string());
            events.description = Some(desc);
            events.description_mp = backend.event_description(&format!("event:/Remote/{name}"));
        } else {
            let ogg = format!("media/sound/{name}.ogg");
            let wav = format!("media/sound/{name}.wav");
            let path = if media_file_exists(&ogg) {
                Some(ogg)
            } else if media_file_exists(&wav) {
                Some(wav)
            } else {
                None
            };
            if let Some(path) = path {
                if backend.load_sound(&path, false) != 0 {
                    clip.file = Some(path);
                }
            }
        }
        if clip.event.is_none() && clip.file.is_none() {
            (self.log)(&format!(
                "couldn't find an FMOD event or .ogg or .wav file for sound \"{name}\""
            ));
        }
        script.game_sound.clips.push(clip);
        let sound = Rc::new(RefCell::new(script));
        self.clip_events.insert(key(&sound), vec![events]);
        self.sounds.push(sound.clone());
        self.by_name.insert(
            Some(name.replace(".wav", "").replace(".ogg", "")),
            sound.clone(),
        );
        Some(sound)
    }

    /// `ScriptsLoaded()`: register every script sound that has clips, then
    /// apply `sounds.ini`. In debug builds the original also reports events
    /// no sound uses; `all_event_paths` supplies the banks' event paths.
    pub fn scripts_loaded(
        &mut self,
        scripts: Vec<SoundRef>,
        backend: &mut dyn AudioBackend,
        all_event_paths: &dyn Fn() -> Vec<String>,
    ) {
        for s in scripts {
            if s.borrow().game_sound.clips.is_empty() {
                continue;
            }
            self.add_sound(s, backend);
        }
        self.load_ini();
        if self.env.debug {
            let used: std::collections::HashSet<String> = self
                .sounds
                .iter()
                .flat_map(|s| {
                    s.borrow()
                        .game_sound
                        .clips
                        .iter()
                        .filter_map(|c| c.event.clone())
                        .collect::<Vec<_>>()
                })
                .filter(|e| !e.is_empty())
                .collect();
            let mut unused: Vec<String> = all_event_paths()
                .into_iter()
                .map(|p| p.replace("event:/", ""))
                .filter(|p| !used.contains(p))
                .collect();
            unused.sort();
            (self.log)(&format!("FMOD {} missing events", self.missing_event_count));
            (self.log)(&format!(
                "FMOD {} events not used by any GameSound",
                unused.len()
            ));
        }
    }

    /// `OnReloadSound(scriptSound)`.
    pub fn on_reload_sound(&mut self, script: &SoundRef, backend: &mut dyn AudioBackend) {
        if self.sounds.iter().any(|s| Rc::ptr_eq(s, script)) {
            // The clips may have been rebuilt; resolve the new ones.
            let n = script.borrow().game_sound.clips.len();
            self.clip_events
                .insert(key(script), vec![ClipEvents::default(); n]);
            self.init_clip_events(script, backend);
            return;
        }
        if script.borrow().game_sound.clips.is_empty() {
            return;
        }
        self.add_sound(script.clone(), backend);
    }

    /// `getCategories()`: sorted, distinct.
    pub fn categories(&self) -> Vec<String> {
        let set: BTreeSet<String> = self
            .sounds
            .iter()
            .map(|s| s.borrow().game_sound.category.clone())
            .collect();
        set.into_iter().collect()
    }

    /// `getSoundsInCategory(category)`.
    pub fn sounds_in_category(&self, category: &str) -> Vec<SoundRef> {
        self.sounds
            .iter()
            .filter(|s| s.borrow().game_sound.category == category)
            .cloned()
            .collect()
    }

    fn ini_path(&self) -> PathBuf {
        self.env.cache_dir.join("sounds.ini")
    }

    /// `loadINI()`.
    pub fn load_ini(&mut self) {
        let mut cf = ConfigFile::new();
        if !cf.read(&self.ini_path()) {
            return;
        }
        if cf.get_version() > VERSION {
            return;
        }
        for option in cf.get_options() {
            let Some(sound) = self.by_name.get(&Some(option.name().to_string())) else {
                continue;
            };
            let v = try_parse_float(&option.get_value_as_string(), 1.0);
            set_user_volume(&mut sound.borrow_mut().game_sound, v);
        }
    }

    /// `saveINI()`.
    pub fn save_ini(&self) {
        let options: Vec<ConfigOption> = self
            .sounds
            .iter()
            .map(|s| {
                let s = s.borrow();
                let mut o = ConfigOption::double(
                    s.game_sound.name.as_deref().unwrap_or("null"),
                    0.0,
                    2.0,
                    0.0,
                );
                o.set_double(user_volume(&self.env, &s.game_sound) as f64);
                o
            })
            .collect();
        let refs: Vec<&ConfigOption> = options.iter().collect();
        ConfigFile::new().write(&self.ini_path(), VERSION, &refs);
    }

    /// `previewSound(name)`. `rand_next(n)` is `Rand.Next(n)`.
    pub fn preview_sound(
        &mut self,
        name: &str,
        backend: &mut dyn AudioBackend,
        rand_next: &mut dyn FnMut(i32) -> i32,
    ) {
        if self.env.sound_disabled {
            (self.log)(&format!("sound is disabled, not playing {name}"));
            return;
        }
        if !self.is_known_sound(name) {
            (self.log)(&format!("sound is not known, not playing {name}"));
            return;
        }
        let sound = self.by_name[&Some(name.to_string())].clone();
        let n = sound.borrow().game_sound.clips.len();
        if n == 0 {
            (self.log)("GameSound.clips is empty");
            return;
        }
        let clip = rand_next(n as i32) as usize;
        if self.sound_is_paused {
            if !self.env.client {
                backend.master_channel_group_set_volume(1.0);
            }
            self.sound_is_paused = false;
        }
        self.stop_preview_inner(backend);
        let (event, file, gain, pitch, looped, music) = {
            let s = sound.borrow();
            let c = &s.game_sound.clips[clip];
            (
                c.event.clone(),
                c.file.clone(),
                effective_volume(&self.env, &s.game_sound, c),
                c.pitch,
                s.game_sound.loop_,
                s.game_sound.master == MasterVolume::Music,
            )
        };
        if event.is_some() {
            let desc = self
                .clip_events(&sound)
                .get(clip)
                .and_then(|e| e.description.clone());
            let Some(desc) = desc else {
                (self.log)(&format!(
                    "failed to get event {}",
                    event.unwrap_or_default()
                ));
                return;
            };
            let instance = backend.create_event_instance(&desc);
            if instance < 0 {
                (self.log)(&format!("failed to create EventInstance: error={instance}"));
                return;
            }
            backend.instance_set_volume(instance, gain);
            backend.instance_set_parameter_by_name(instance, "Occlusion", 0.0);
            backend.instance_start(instance);
            if music {
                backend.instance_set_parameter_by_name(instance, "Volume", 10.0);
            }
            self.preview = Some(Preview::Bank {
                instance,
                sound,
                clip,
                gain,
            });
        } else if let Some(file) = file {
            let snd = backend.load_sound(&file, looped);
            if snd == 0 {
                return;
            }
            let channel = backend.play_sound(snd, true);
            backend.channel_set_volume(channel, gain);
            backend.channel_set_pitch(channel, pitch);
            if looped {
                backend.channel_set_mode(channel, MODE_LOOP_NORMAL);
            }
            backend.channel_set_paused(channel, false);
            self.preview = Some(Preview::File {
                channel,
                sound,
                clip,
                gain,
            });
        }
    }

    fn stop_preview_inner(&mut self, backend: &mut dyn AudioBackend) {
        match self.preview.take() {
            Some(Preview::Bank { instance, .. }) if instance != 0 => {
                backend.instance_stop(instance, false);
                backend.instance_release(instance);
            }
            Some(Preview::File { channel, .. }) if channel != 0 => backend.channel_stop(channel),
            _ => {}
        }
    }

    /// `stopPreview()`.
    pub fn stop_preview(&mut self, backend: &mut dyn AudioBackend) {
        self.stop_preview_inner(backend);
    }

    /// `isPreviewPlaying()`: updates the preview (volume changes, end).
    pub fn is_preview_playing(&mut self, backend: &mut dyn AudioBackend) -> bool {
        let env = self.env.clone();
        let target = |sound: &SoundRef, clip: usize| {
            let s = sound.borrow();
            s.game_sound
                .clips
                .get(clip)
                .map_or(1.0, |c| effective_volume(&env, &s.game_sound, c))
        };
        let finished = match &mut self.preview {
            None => return false,
            Some(Preview::Bank {
                instance,
                sound,
                clip,
                gain,
            }) => {
                let state = backend.instance_playback_state(*instance);
                if state == PlaybackState::Stopping {
                    false
                } else if state == PlaybackState::Stopped {
                    backend.instance_release(*instance);
                    true
                } else {
                    let t = target(sound, *clip);
                    if *gain != t {
                        *gain = t;
                        backend.instance_set_volume(*instance, t);
                    }
                    false
                }
            }
            Some(Preview::File {
                channel,
                sound,
                clip,
                gain,
            }) => {
                if !backend.channel_is_playing(*channel) {
                    true
                } else {
                    let t = target(sound, *clip);
                    if *gain != t {
                        *gain = t;
                        backend.channel_set_volume(*channel, t);
                    }
                    false
                }
            }
        };
        if finished {
            self.preview = None;
            return false;
        }
        match &self.preview {
            Some(Preview::Bank { instance, .. }) => {
                // isPlaying: STOPPING counts as playing; STOPPED does not.
                backend.instance_playback_state(*instance) != PlaybackState::Stopped
            }
            Some(Preview::File { channel, .. }) => backend.channel_is_playing(*channel),
            None => false,
        }
    }

    /// `fix3DListenerPosition(inMenu)`.
    pub fn fix_3d_listener_position(
        &self,
        in_menu: bool,
        players: &[Option<ListenerPlayer>],
        backend: &mut dyn AudioBackend,
    ) {
        if self.env.sound_disabled {
            return;
        }
        if in_menu {
            backend.listener_3d(0, [0.0; 3], [0.0; 3], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]);
            return;
        }
        let d = -1.0 / 2.0f64.sqrt() as f32;
        for (i, p) in players.iter().enumerate() {
            let Some(p) = p else { continue };
            if p.deaf {
                continue;
            }
            backend.listener_3d(
                i as i32,
                [p.x, p.y, p.z * 3.0],
                [0.0; 3],
                [d, d, 0.0],
                [0.0, 0.0, 1.0],
            );
        }
    }

    /// `Reset()`.
    pub fn reset(&mut self, backend: &mut dyn AudioBackend) {
        self.sounds.clear();
        self.by_name.clear();
        self.clip_events.clear();
        self.stop_preview_inner(backend);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records calls; knows the events in `events`.
    #[derive(Default)]
    struct Mock {
        events: Vec<&'static str>,
        calls: Vec<String>,
        state: Option<PlaybackState>,
    }

    impl AudioBackend for Mock {
        fn event_description(&mut self, path: &str) -> Option<Rc<EventDescription>> {
            self.events.iter().position(|e| *e == path).map(|i| {
                Rc::new(EventDescription {
                    address: i as u64 + 1,
                    path: path.to_string(),
                    length_ms: 0,
                    is_3d: false,
                    is_oneshot: true,
                    has_sustain_points: false,
                    max_distance: 20.0,
                    parameters: vec![],
                })
            })
        }
        fn create_event_instance(&mut self, d: &EventDescription) -> InstanceHandle {
            self.calls.push(format!("create {}", d.path));
            100
        }
        fn instance_set_volume(&mut self, i: InstanceHandle, v: f32) {
            self.calls.push(format!("volume {i} {v}"));
        }
        fn instance_set_parameter_by_name(&mut self, i: InstanceHandle, n: &str, v: f32) {
            self.calls.push(format!("param {i} {n} {v}"));
        }
        fn instance_start(&mut self, i: InstanceHandle) {
            self.calls.push(format!("start {i}"));
        }
        fn instance_stop(&mut self, i: InstanceHandle, imm: bool) {
            self.calls.push(format!("stop {i} {imm}"));
        }
        fn instance_release(&mut self, i: InstanceHandle) {
            self.calls.push(format!("release {i}"));
        }
        fn instance_playback_state(&mut self, _i: InstanceHandle) -> PlaybackState {
            self.state.unwrap_or(PlaybackState::Playing)
        }
        fn listener_3d(&mut self, i: i32, p: [f32; 3], _v: [f32; 3], f: [f32; 3], u: [f32; 3]) {
            self.calls.push(format!("listener {i} {p:?} {f:?} {u:?}"));
        }
        fn load_sound(&mut self, path: &str, looped: bool) -> u64 {
            self.calls.push(format!("load {path} {looped}"));
            7
        }
        fn play_sound(&mut self, s: u64, paused: bool) -> u64 {
            self.calls.push(format!("play {s} {paused}"));
            9
        }
        fn channel_set_volume(&mut self, c: u64, v: f32) {
            self.calls.push(format!("chvol {c} {v}"));
        }
        fn channel_set_pitch(&mut self, c: u64, p: f32) {
            self.calls.push(format!("chpitch {c} {p}"));
        }
        fn channel_set_mode(&mut self, c: u64, m: u64) {
            self.calls.push(format!("chmode {c} {m}"));
        }
        fn channel_set_paused(&mut self, c: u64, p: bool) {
            self.calls.push(format!("chpause {c} {p}"));
        }
        fn channel_is_playing(&mut self, _c: u64) -> bool {
            true
        }
        fn channel_stop(&mut self, c: u64) {
            self.calls.push(format!("chstop {c}"));
        }
        fn master_channel_group_set_volume(&mut self, v: f32) {
            self.calls.push(format!("master {v}"));
        }
    }

    fn script(name: &str, category: &str, events: &[Option<&str>]) -> SoundRef {
        let mut s = GameSoundScript::new();
        s.game_sound.name = Some(name.to_string());
        s.game_sound.category = category.to_string();
        for e in events {
            let mut c = GameSoundClip::new(&s.game_sound);
            c.event = e.map(str::to_string);
            s.game_sound.clips.push(c);
        }
        Rc::new(RefCell::new(s))
    }

    fn env(dir: &std::path::Path) -> SoundEnv {
        SoundEnv {
            cache_dir: dir.to_path_buf(),
            ..Default::default()
        }
    }

    /// A fresh directory per call, so parallel tests never share one.
    fn scratch(tag: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pz-sound-test-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn add_sound_replaces_in_place_and_resolves_events() {
        let mut b = Mock {
            events: vec!["event:/UI/Click", "event:/Remote/UI/Click"],
            ..Default::default()
        };
        let mut g = GameSounds::new(env(&scratch("add")));
        g.add_sound(script("A", "UI", &[Some("UI/Click")]), &mut b);
        g.add_sound(script("B", "World", &[Some("Missing")]), &mut b);
        let a2 = script("A", "Zed", &[Some("UI/Click"), None]);
        g.add_sound(a2.clone(), &mut b);
        let names: Vec<_> = g
            .sounds()
            .iter()
            .map(|s| s.borrow().game_sound.name.clone().unwrap())
            .collect();
        assert_eq!(names, ["A", "B"]);
        assert!(Rc::ptr_eq(&g.sounds()[0], &a2));
        let ev = g.clip_events(&a2);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].description.as_ref().unwrap().path, "event:/UI/Click");
        assert_eq!(
            ev[0].description_mp.as_ref().unwrap().path,
            "event:/Remote/UI/Click"
        );
        assert!(ev[1].description.is_none());
        assert_eq!(g.missing_event_count, 1);
        assert_eq!(g.categories(), ["World", "Zed"]);
        assert_eq!(g.sounds_in_category("World").len(), 1);
    }

    #[test]
    fn get_or_create_falls_back_to_files() {
        let mut b = Mock {
            events: vec!["event:/Thing"],
            ..Default::default()
        };
        let mut g = GameSounds::new(env(&scratch("create")));
        let s = g.get_or_create_sound("Thing", &mut b, &|_| false).unwrap();
        assert_eq!(s.borrow().game_sound.category, "AUTO");
        assert_eq!(
            s.borrow().game_sound.clips[0].event.as_deref(),
            Some("Thing")
        );
        let f = g
            .get_or_create_sound("Boom.wav", &mut b, &|p| p == "media/sound/Boom.wav.wav")
            .unwrap();
        assert_eq!(
            f.borrow().game_sound.clips[0].file.as_deref(),
            Some("media/sound/Boom.wav.wav")
        );
        // Stored without the extension, as the original does.
        assert!(g.is_known_sound("Boom"));
        assert!(!g.is_known_sound("Boom.wav"));
        assert!(g.get_or_create_sound("", &mut b, &|_| true).is_none());
    }

    #[test]
    fn ini_round_trip_needs_advanced_options() {
        let dir = scratch("ini");
        let mut b = Mock::default();
        let mut g = GameSounds::new(SoundEnv {
            advanced_sound_options: true,
            ..env(&dir)
        });
        g.add_sound(script("A", "UI", &[None]), &mut b);
        set_user_volume(&mut g.sounds()[0].borrow_mut().game_sound, 3.0);
        assert_eq!(user_volume(&g.env, &g.sounds()[0].borrow().game_sound), 2.0);
        set_user_volume(&mut g.sounds()[0].borrow_mut().game_sound, 0.25);
        g.save_ini();
        let text = std::fs::read_to_string(dir.join("sounds.ini")).unwrap();
        assert!(text.starts_with("Version=1"), "{text}");
        let mut g2 = GameSounds::new(SoundEnv {
            advanced_sound_options: true,
            ..env(&dir)
        });
        g2.scripts_loaded(
            vec![script("A", "UI", &[None]), script("Empty", "UI", &[])],
            &mut b,
            &Vec::new,
        );
        assert_eq!(g2.sounds().len(), 1);
        assert_eq!(
            user_volume(&g2.env, &g2.sounds()[0].borrow().game_sound),
            0.25
        );
        g2.env.advanced_sound_options = false;
        assert_eq!(
            user_volume(&g2.env, &g2.sounds()[0].borrow().game_sound),
            1.0
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn preview_plays_bank_event_then_stops() {
        let mut b = Mock {
            events: vec!["event:/Music/Main"],
            ..Default::default()
        };
        let mut g = GameSounds::new(env(&scratch("preview")));
        let s = script("Theme", "Music", &[Some("Music/Main")]);
        s.borrow_mut().game_sound.master = MasterVolume::Music;
        s.borrow_mut().game_sound.clips[0].volume = 0.5;
        g.add_sound(s, &mut b);
        g.sound_is_paused = true;
        g.preview_sound("Theme", &mut b, &mut |_| 0);
        assert_eq!(
            b.calls,
            [
                "master 1",
                "create event:/Music/Main",
                "volume 100 0.5",
                "param 100 Occlusion 0",
                "start 100",
                "param 100 Volume 10"
            ]
        );
        assert!(g.is_preview_playing(&mut b));
        b.state = Some(PlaybackState::Stopped);
        assert!(!g.is_preview_playing(&mut b));
        assert_eq!(b.calls.last().unwrap(), "release 100");
    }

    #[test]
    fn listener_positions() {
        let mut b = Mock::default();
        let g = GameSounds::new(env(&scratch("listener")));
        g.fix_3d_listener_position(true, &[], &mut b);
        let p = ListenerPlayer {
            x: 10.0,
            y: 20.0,
            z: 1.0,
            deaf: false,
        };
        let deaf = ListenerPlayer { deaf: true, ..p };
        g.fix_3d_listener_position(false, &[Some(p), None, Some(deaf)], &mut b);
        assert_eq!(b.calls.len(), 2);
        assert_eq!(
            b.calls[0],
            "listener 0 [0.0, 0.0, 0.0] [0.0, -1.0, 0.0] [0.0, 0.0, 1.0]"
        );
        assert!(
            b.calls[1].starts_with("listener 0 [10.0, 20.0, 3.0] [-0.70710677, -0.70710677, 0.0]"),
            "{}",
            b.calls[1]
        );
    }
}
