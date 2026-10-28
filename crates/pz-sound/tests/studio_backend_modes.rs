//! Device-free regressions through the real core-channel backend and emitter.
#![cfg(feature = "studio")]

use pz_audio::studio::Studio;
use pz_audio::vorbis_setup::SetupHeaders;
use pz_scripts::objects::models::{GameSoundClip, GameSoundScript};
use pz_sound::backend::{AudioBackend, MODE_2D, MODE_3D};
use pz_sound::emitter::{Ctx, EmitterWorld, FmodSoundEmitter};
use pz_sound::game_sounds::{GameSounds, SoundEnv};
use pz_sound::studio_backend::StudioBackend;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

struct Files(PathBuf);

impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pz-rs-studio-mode-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn wave(&self, name: &str, frames: usize) -> String {
        let path = self.0.join(name);
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + frames as u32 * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((frames as u32 * 2).to_le_bytes());
        for _ in 0..frames {
            bytes.extend(16384i16.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn studio() -> Arc<Mutex<Studio>> {
    let studio = Arc::new(Mutex::new(Studio::new(SetupHeaders::default())));
    studio
        .lock()
        .unwrap()
        .set_listener([0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
    studio
}

fn mix(studio: &Arc<Mutex<Studio>>, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; frames * 2];
    studio.lock().unwrap().mix(&mut out);
    out
}

#[test]
fn backend_initial_3d_and_mode_changes_keep_attributes_independent() {
    let files = Files::new();
    let path = files.wave("constant.wav", 64);
    let studio = studio();
    let mut backend =
        StudioBackend::new(studio.clone()).with_file_resolver(move |_| Some(path.clone()));
    let sound = backend.load_sound("constant", false);
    assert_ne!(sound, 0);
    let channel = backend.play_sound(sound, false);
    backend.channel_set_3d_attributes(channel, [8.0, 0.0, 0.0], [0.0; 3]);
    let out = mix(&studio, 4);
    assert!(
        out[1] > out[0] && out[1] < 0.5,
        "backend-created sound starts 3D: {out:?}"
    );

    backend.channel_set_mode(channel, MODE_2D);
    backend.channel_set_3d_attributes(channel, [-8.0, 0.0, 0.0], [0.0; 3]);
    assert_eq!(mix(&studio, 4), vec![0.5; 8], "attributes cannot enable 3D");
    backend.channel_set_mode(channel, MODE_3D);
    let out = mix(&studio, 4);
    assert!(
        out[0] > out[1] && out[0] < 0.5,
        "3D uses the last submitted position: {out:?}"
    );
    backend.channel_set_mode(channel, MODE_2D);
    assert_eq!(mix(&studio, 4), vec![0.5; 8]);
    backend.channel_set_mode(channel, MODE_3D);
    let out = mix(&studio, 4);
    assert!(
        out[0] > out[1],
        "disabling mode must retain position: {out:?}"
    );
}

struct TwoPlayers;
impl EmitterWorld for TwoPlayers {
    fn num_players(&self) -> usize {
        2
    }
}

#[test]
fn ordinary_emitter_tick_after_set_3d_false_stays_2d() {
    let files = Files::new();
    let path = files.wave("emitter.wav", 64);
    let studio = studio();
    let mut backend =
        StudioBackend::new(studio.clone()).with_file_resolver(move |_| Some(path.clone()));
    let mut sounds = GameSounds::new(SoundEnv::default());
    let mut script = GameSoundScript::new();
    script.game_sound.name = Some("ModeFixture".into());
    script.game_sound.is3d = true;
    let mut clip = GameSoundClip::new(&script.game_sound);
    clip.file = Some("emitter.wav".into());
    clip.distance_min = 1.0;
    clip.distance_max = 100.0;
    script.game_sound.clips.push(clip);
    sounds.add_sound(Rc::new(RefCell::new(script)), &mut backend);
    let mut emitter = FmodSoundEmitter::new(0);
    emitter.set_pos(8.0, 0.0, 0.0);
    let mut rand = |_| 0;
    let exists = |_: &str| true;
    let reference;
    {
        let mut ctx = Ctx {
            backend: &mut backend,
            sounds: &mut sounds,
            world: &TwoPlayers,
            rand_next: &mut rand,
            media_file_exists: &exists,
        };
        reference = emitter.play_sound("ModeFixture", &mut ctx);
        assert_ne!(reference, 0);
        emitter.set_3d(reference, false, &mut ctx);
        emitter.tick(&mut ctx);
    }
    assert_eq!(
        mix(&studio, 4),
        vec![0.5; 8],
        "real tick submits attributes after MODE_2D"
    );
    emitter.set_pos(-8.0, 0.0, 0.0);
    {
        let mut ctx = Ctx {
            backend: &mut backend,
            sounds: &mut sounds,
            world: &TwoPlayers,
            rand_next: &mut rand,
            media_file_exists: &exists,
        };
        emitter.tick(&mut ctx);
    }
    assert_eq!(mix(&studio, 4), vec![0.5; 8], "moving emitter remains 2D");
    // Select mode directly, without submitting another position. This checks
    // that the actual tick stored its changed attributes while still in 2D.
    backend.channel_set_mode(reference as u64, MODE_3D);
    let out = mix(&studio, 4);
    assert!(
        out[0] > out[1] && out[0] < 0.5,
        "tick retained the new position: {out:?}"
    );
}

#[test]
fn resolved_aliases_and_loop_arguments_share_the_first_sound() {
    for first_looped in [false, true] {
        let files = Files::new();
        let path = files.wave("loop.wav", 4);
        let resolutions = Rc::new(RefCell::new(Vec::new()));
        let observed = resolutions.clone();
        let studio = studio();
        let mut backend = StudioBackend::new(studio.clone()).with_file_resolver(move |path_in| {
            observed.borrow_mut().push(path_in.to_owned());
            Some(path.clone())
        });
        let first = backend.load_sound("first-alias", first_looped);
        assert_ne!(first, 0);
        assert_eq!(backend.load_sound("second-alias", !first_looped), first);
        assert_eq!(*resolutions.borrow(), ["first-alias", "second-alias"]);
        let channel = backend.play_sound(first, false);
        backend.channel_set_mode(channel, MODE_2D);
        let out = mix(&studio, 8);
        assert_eq!(&out[..8], &[0.5; 8]);
        assert_eq!(&out[8..], if first_looped { &[0.5; 8] } else { &[0.0; 8] });
        assert_eq!(
            backend.channel_is_playing(channel),
            first_looped,
            "first cached loop argument wins"
        );
    }
}

#[test]
fn current_resolution_precedes_cache_and_changes_the_resolved_key() {
    let files = Files::new();
    let first_path = files.wave("first.wav", 64);
    let second_path = files.wave("second.wav", 64);
    let map = Rc::new(RefCell::new(HashMap::from([(
        "logical".to_owned(),
        first_path.clone(),
    )])));
    let current = map.clone();
    let studio = studio();
    let mut backend = StudioBackend::new(studio)
        .with_file_resolver(move |path| current.borrow().get(path).cloned());
    let first = backend.load_sound("logical", false);
    assert_ne!(first, 0);
    map.borrow_mut().remove("logical");
    assert_eq!(
        backend.load_sound("logical", false),
        0,
        "null resolution returns before a prior cache hit"
    );
    map.borrow_mut()
        .insert("logical".into(), second_path.clone());
    let second = backend.load_sound("logical", true);
    assert_ne!(second, 0);
    assert_ne!(second, first, "current resolved path owns the cache entry");
    map.borrow_mut().insert("alias".into(), second_path.clone());
    std::fs::remove_file(Path::new(&second_path)).unwrap();
    assert_eq!(
        backend.load_sound("alias", false),
        second,
        "resolved aliases hit the cache before another file read"
    );
    map.borrow_mut()
        .insert("logical".into(), first_path.clone());
    assert_eq!(backend.load_sound("logical", true), first);
    assert_eq!(
        backend.load_sound(&first_path, false),
        0,
        "an existing path is not an authority fallback"
    );
}

#[test]
fn no_resolver_does_not_fall_back_to_an_existing_absolute_file() {
    let files = Files::new();
    let path = files.wave("unmapped.wav", 4);
    let mut backend = StudioBackend::new(studio());
    assert_eq!(backend.load_sound(&path, false), 0);
}
