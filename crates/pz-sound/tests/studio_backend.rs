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
