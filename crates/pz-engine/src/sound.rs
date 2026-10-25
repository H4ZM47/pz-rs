//! `SoundManager` / `FMODManager.init` for the windowed client: the FMOD
//! Studio banks through the pz-audio studio, played on the system's
//! audio device.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// The banks `FMODManager.init` loads, in order (`media/sound/banks/Desktop`).
const BANKS: [&str; 4] = ["ZomboidSound.strings", "ZomboidSound", "ZomboidSoundMP", "ZomboidMusic"];

/// Build the studio, start output and install the shared sound system.
/// Returns the output stream, which plays for as long as it is kept.
/// On failure the sound system stays silent and the error is returned.
pub fn install(game_dir: &Path, cache_dir: &Path, fs: Rc<RefCell<pz_fs::GameFs>>, core: &pz_core::core::Core) -> Result<pz_audio::output::Output, String> {
    if core.sound_disabled {
        return Err("sound disabled".into());
    }
    let setups = pz_audio::vorbis_setup::SetupHeaders::from_game_dir(game_dir).map_err(|e| format!("vorbis setup headers: {e}"))?;
    let mut studio = pz_audio::studio::Studio::new(setups);
    let banks = game_dir.join("media").join("sound").join("banks").join("Desktop");
    for b in BANKS {
        studio.load_bank(&banks.join(format!("{b}.bank"))).map_err(|e| format!("bank {b}: {e}"))?;
    }
    let studio = Arc::new(Mutex::new(studio));
    let output = pz_audio::output::Output::start(studio.clone())?;
    let backend = pz_sound::studio_backend::StudioBackend::new(studio);
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
