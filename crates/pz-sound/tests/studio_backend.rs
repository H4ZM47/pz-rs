//! The studio backend over an installed game's banks:
//! `PZ_GAME_DIR=... cargo test -p pz-sound --features studio -- --ignored`.
#![cfg(feature = "studio")]

use pz_audio::studio::Studio;
use pz_audio::vorbis_setup::SetupHeaders;
use pz_sound::backend::{AudioBackend, PlaybackState};
use pz_sound::studio_backend::StudioBackend;
use std::sync::{Arc, Mutex};

#[test]
#[ignore]
fn plays_game_events_through_the_backend() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let mut studio = Studio::new(SetupHeaders::from_game_dir(&dir).unwrap());
    let banks = dir.join("media/sound/banks/Desktop");
    for name in ["ZomboidSound.strings", "ZomboidSound", "ZomboidMusic"] {
        studio.load_bank(&banks.join(format!("{name}.bank"))).unwrap();
    }
    let mut b = StudioBackend::new(Arc::new(Mutex::new(studio)));
    let paths = b.event_paths();
    assert!(paths.len() > 1000, "{}", paths.len());

    // Global indices are dense and unique.
    let n = b.parameter_count();
    assert!(n > 0);
    let mut seen = vec![false; n];
    for p in &paths {
        let d = b.event_description(p).unwrap_or_else(|| panic!("{p}"));
        assert_eq!(d.path.to_lowercase(), p.to_lowercase());
        for q in &d.parameters {
            seen[q.global_index] = true;
            assert_eq!(b.parameter_description(&q.name).unwrap().global_index, q.global_index);
        }
    }
    assert!(seen.iter().all(|&x| x));
    assert!(b.event_description("event:/NoSuchEvent").is_none());

    // An instance starts, takes parameters and stops.
    let d = b.event_description(&paths[0]).unwrap();
    let inst = b.create_event_instance(&d);
    assert!(inst > 0);
    b.instance_set_volume(inst, 0.5);
    for q in &d.parameters {
        b.instance_set_parameter_by_id(inst, q.id, q.maximum);
    }
    b.instance_start(inst);
    assert_ne!(b.instance_playback_state(inst), PlaybackState::Stopped);
    b.instance_stop(inst, true);
    b.instance_release(inst);
    assert_eq!(b.instance_playback_state(inst), PlaybackState::Stopped);
}

#[test]
#[ignore]
fn plays_game_sound_files_through_the_backend() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let studio = Arc::new(Mutex::new(Studio::new(SetupHeaders::default())));
    let root = dir.clone();
    let mut b = StudioBackend::new(studio.clone()).with_file_resolver(move |p| Some(root.join(p)));

    // Every loose file decodes.
    let mut n = 0;
    for e in std::fs::read_dir(dir.join("media/sound")).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".ogg") || name.ends_with(".wav") {
            assert_ne!(b.load_sound(&format!("media/sound/{name}"), false), 0, "{name}");
            n += 1;
        }
    }
    assert!(n > 100);
    assert_eq!(b.load_sound("media/sound/NoSuchFile.ogg", false), 0);

    let snd = b.load_sound("media/sound/9mmShot.ogg", false);
    assert_eq!(b.load_sound("media/sound/9mmShot.ogg", false), snd, "cached");
    let ch = b.play_sound(snd, true);
    b.channel_set_volume(ch, 1.0);
    b.channel_set_paused(ch, false);
    assert!(b.channel_is_playing(ch));
    let mut out = vec![0.0f32; 2 * 4800];
    studio.lock().unwrap().mix(&mut out);
    assert!(out.iter().any(|&x| x.abs() > 0.01));
    assert!(b.channel_position_ms(ch) >= 99);
    b.channel_stop(ch);
    assert!(!b.channel_is_playing(ch));
}
