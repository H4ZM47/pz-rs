//! Exercise the exact backend factory used by install, without opening a device.

use super::backend_with_game_fs;
use pz_audio::studio::Studio;
use pz_audio::vorbis_setup::SetupHeaders;
use pz_sound::backend::{AudioBackend, MODE_2D};
use pz_sound::studio_backend::StudioBackend;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

struct Files(PathBuf);

impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pz-rs-engine-sound-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn wave(&self, relative: &str, sample: i16) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + 128u32).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(128u32.to_le_bytes());
        for _ in 0..64 {
            bytes.extend(sample.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mix_sample(backend: &mut StudioBackend, studio: &Arc<Mutex<Studio>>, sound: u64) -> f32 {
    let channel = backend.play_sound(sound, false);
    backend.channel_set_mode(channel, MODE_2D);
    let mut out = [0.0; 8];
    studio.lock().unwrap().mix(&mut out);
    backend.channel_stop(channel);
    assert_eq!(out[0], out[1]);
    out[0]
}

#[test]
fn production_backend_factory_uses_live_install_mod_and_current_mapping() {
    let files = Files::new();
    let installed = files.wave("install/media/sound/live.wav", 8192);
    let modded = files.wave("mod/common/media/sound/live.wav", 24576);
    let install = files.0.join("install");
    let current_dir = std::env::current_dir().unwrap();
    assert_ne!(
        current_dir.canonicalize().unwrap(),
        install.canonicalize().unwrap()
    );
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&install)));
    let first_resolved = fs
        .borrow()
        .absolute_path("media/sound/live.wav")
        .unwrap()
        .to_owned();
    let studio = Arc::new(Mutex::new(Studio::new(SetupHeaders::default())));
    let mut backend = backend_with_game_fs(studio.clone(), fs.clone());
    let first = backend.load_sound("MEDIA/SOUND/LIVE.WAV", false);
    assert_ne!(
        first, 0,
        "resolve through the installed GameFs, regardless of cwd"
    );
    assert_eq!(mix_sample(&mut backend, &studio, first), 0.25);
    {
        let mut active = fs.borrow_mut();
        active.set_mod_id_to_dir("audiofixture", files.0.join("mod").to_str().unwrap());
        active.load_mod("audiofixture");
        let resolved = active.absolute_path("media/sound/live.wav").unwrap();
        assert_eq!(
            Path::new(resolved).canonicalize().unwrap(),
            modded.canonicalize().unwrap()
        );
    }
    let overridden = backend.load_sound("media/sound/live.wav", false);
    assert_ne!(overridden, 0);
    assert_ne!(
        overridden, first,
        "factory retained the live map, not a snapshot"
    );
    assert_eq!(mix_sample(&mut backend, &studio, overridden), 0.75);
    fs.borrow_mut()
        .active_file_map
        .remove("media/sound/live.wav");
    assert_eq!(
        backend.load_sound("media/sound/live.wav", false),
        0,
        "current null wins over the old cache"
    );
    assert_eq!(
        backend.load_sound(installed.to_str().unwrap(), false),
        0,
        "no filesystem or cwd fallback"
    );
    fs.borrow_mut()
        .active_file_map
        .insert("media/sound/live.wav".into(), first_resolved);
    assert_eq!(
        backend.load_sound("media/sound/live.wav", true),
        first,
        "resolved cache entry survives map changes"
    );
}

#[test]
fn held_game_fs_borrow_is_a_failure_and_does_not_become_a_negative_cache_entry() {
    let files = Files::new();
    files.wave("install/media/sound/borrow.wav", 16384);
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(files.0.join("install"))));
    let studio = Arc::new(Mutex::new(Studio::new(SetupHeaders::default())));
    let mut backend = backend_with_game_fs(studio.clone(), fs.clone());
    let held = fs.borrow_mut();
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        backend.load_sound("media/sound/borrow.wav", false)
    }));
    assert!(
        failed.is_err(),
        "host borrow failure must not return missing-file 0"
    );
    drop(held);
    let loaded = backend.load_sound("media/sound/borrow.wav", false);
    assert_ne!(
        loaded, 0,
        "the failed resolution did not create a negative cache entry"
    );
    assert_eq!(mix_sample(&mut backend, &studio, loaded), 0.5);
    let held = fs.borrow_mut();
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        backend.load_sound("media/sound/borrow.wav", true)
    }));
    assert!(
        failed.is_err(),
        "current authority is required even for a cached sound"
    );
    drop(held);
    assert_eq!(backend.load_sound("media/sound/borrow.wav", true), loaded);
}

pub(super) fn installed_sound(path: &str, name: &str) -> (pz_sound::system::SoundSystem, Arc<Mutex<Studio>>, usize) {
    use pz_scripts::objects::models::game_sound::GameSoundScript;
    use pz_scripts::{ScriptManager, ScriptObject};
    use pz_sound::system::SoundSystem;

    let game = PathBuf::from(std::env::var_os("PZ_GAME_DIR").expect("PZ_GAME_DIR"));
    let script = std::fs::read_to_string(game.join(path)).unwrap();
    let manager = ScriptManager::new();
    let root = manager.parse(&script);
    let sound = root.get_block("module", Some("Base")).unwrap().get_block("sound", Some(name)).unwrap();
    let mut body = String::new();
    sound.pretty_print(0, &mut body, "\n", "    ");
    let mut clip = GameSoundScript::new();
    clip.load(&manager, name, &body).unwrap();
    let mut studio = Studio::new(SetupHeaders::from_game_dir(&game).unwrap());
    for bank in pz_audio::startup_bank_paths(&game) {
        studio.load_bank(&bank).unwrap();
    }
    let studio = Arc::new(Mutex::new(studio));
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&game)));
    let backend = backend_with_game_fs(studio.clone(), fs);
    let count = backend.parameter_count();
    let mut system = SoundSystem::new(Box::new(backend), Default::default(), count);
    system.sounds.add_sound(Rc::new(RefCell::new(clip)), system.backend.as_mut());
    (system, studio, count)
}

/// Run the installed sound script through the world emitter and Studio mixer.
#[test]
#[ignore = "requires PZ_GAME_DIR and installed sound banks"]
fn installed_window_smash_renders_and_releases_world_audio() {
    let (mut system, studio, _) = installed_sound("media/scripts/generated/sounds/objects/sounds_object_window.txt", "SmashWindow");
    system.update_player_listener(0, [10695.5, 9383.5, 2.0], false);
    let (emitter, handle) = system.play_world_clip("SmashWindow", [10696.5, 9383.5, 2.0], true);
    assert_ne!(handle, 0);
    system.with_world_emitters(|pool, ctx| pool.update(1, ctx));
    let mut block = [0.0; 2048];
    let mut peak = 0.0_f32;
    for _ in 0..24 {
        block.fill(0.0);
        studio.lock().unwrap().mix(&mut block);
        assert!(block.iter().all(|sample| sample.is_finite()));
        peak = block.iter().fold(peak, |peak, sample| peak.max(sample.abs()));
    }
    assert!(peak > 0.001, "SmashWindow PCM peak {peak}");
    system.with_world_emitters(|pool, ctx| drop(pool.stop_and_reset(ctx)));
    assert!(emitter.borrow().is_empty());
    block.fill(0.0);
    studio.lock().unwrap().mix(&mut block);
    assert!(block.iter().all(|sample| *sample == 0.0), "audio persisted after world exit");
}
