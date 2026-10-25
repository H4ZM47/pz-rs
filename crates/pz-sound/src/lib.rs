//! The game's sound logic above the audio engine: `zombie.GameSounds`,
//! `zombie.audio.GameSound`/`GameSoundClip` behaviour, and (next)
//! `SoundManager` and the sound emitters.
//!
//! Everything reaches the audio engine through [`backend::AudioBackend`],
//! which mirrors the `fmod.javafmod` calls the original makes, so the engine
//! (`pz-audio`'s studio, or a silent backend) stays swappable.

pub mod backend;
pub mod emitter;
pub mod game_sounds;
pub mod params;
pub mod sound_manager;
#[cfg(feature = "studio")]
pub mod studio_backend;
pub mod system;
pub mod world_emitters;
