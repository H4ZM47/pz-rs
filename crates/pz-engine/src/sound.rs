//! `SoundManager` / `FMODManager.init` for the windowed client: the FMOD
//! Studio banks through the pz-audio studio, played on the system's
//! audio device.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// The same backend construction used by install, without starting a device.
/// Resolve each load against the live GameFs, including subsequent mod overrides.
fn backend_with_game_fs(
    studio: Arc<Mutex<pz_audio::studio::Studio>>,
    fs: Rc<RefCell<pz_fs::GameFs>>,
) -> pz_sound::studio_backend::StudioBackend {
    pz_sound::studio_backend::StudioBackend::new(studio)
        .with_file_resolver(move |path| fs.borrow().absolute_path(path).map(str::to_owned))
}

/// Build the studio, start output and install the shared sound system.
/// Returns the output stream, which plays for as long as it is kept.
/// On failure the sound system stays silent and the error is returned.
pub fn install(game_dir: &Path, cache_dir: &Path, fs: Rc<RefCell<pz_fs::GameFs>>, core: &pz_core::core::Core) -> Result<pz_audio::output::Output, String> {
    if core.sound_disabled {
        return Err("sound disabled".into());
    }
    let setups = pz_audio::vorbis_setup::SetupHeaders::from_game_dir(game_dir).map_err(|e| format!("vorbis setup headers: {e}"))?;
    let mut studio = pz_audio::studio::Studio::new(setups);
    for path in pz_audio::startup_bank_paths(game_dir) {
        // A single bank failure does not disable the other native banks.
        if let Err(error) = studio.load_bank(&path) {
            eprintln!("sound bank {}: {error}", path.display());
        }
    }
    let studio = Arc::new(Mutex::new(studio));
    let output = pz_audio::output::Output::start(studio.clone())?;
    let backend = backend_with_game_fs(studio, fs.clone());
    let count = backend.parameter_count();
    let env = pz_sound::game_sounds::SoundEnv {
        server: false,
        client: false,
        debug: core.get_debug(),
        sound_disabled: false,
        advanced_sound_options: false,
        cache_dir: cache_dir.to_path_buf(),
    };
    let mut system = pz_sound::system::SoundSystem::new(Box::new(backend), env, count);
    system.rand_next = Box::new(pz_core::random::next);
    system.media_file_exists = Box::new(move |p| fs.borrow().absolute_path(p).is_some());
    pz_lua_api::bind_audio::install_sound_system(Rc::new(RefCell::new(system)));
    Ok(output)
}

/// Inputs sampled by the global music parameters on SoundManager.Update.
pub(super) fn music_inputs(core: &pz_core::core::Core) -> pz_sound::sound_manager::MusicInputs {
    use pz_lua_api::bind_players::{player, MAX_PLAYERS};
    use pz_sound::sound_manager::{MusicInputs, MusicPlayer};
    let players = (0..MAX_PLAYERS).map(|i| player(i).map(|chr| {
        let chr = chr.borrow();
        let stats = &chr.stats;
        MusicPlayer {
            dead: chr.is_dead(),
            asleep: chr.asleep,
            intensity: chr.player.music_intensity_events.intensity(),
            zombies_targeting: stats.music_zombies_targeting_nearby_moving
                + stats.music_zombies_targeting_distant_moving
                + stats.music_zombies_targeting_nearby_not_moving
                + stats.music_zombies_targeting_distant_not_moving,
            zombies_visible: stats.music_zombies_visible,
            threat: chr.player.music_threat_levels.intensity(),
        }
    })).collect();
    MusicInputs {
        music_action_style: core.get_option_music_action_style(),
        music_library: core.get_option_music_library(),
        music_volume_option: core.get_option_music_volume(),
        players,
    }
}

#[cfg(test)]
mod tests;
