//! Everything the sound code needs in one place, for callers (the game
//! loop, the Lua bindings) that hold a single shared sound system.

use crate::backend::{AudioBackend, SilentBackend};
use crate::emitter::{Ctx, EmitterWorld};
use crate::game_sounds::{GameSounds, SoundEnv};
use crate::sound_manager::SoundManager;

/// A world with no players or squares (the main menu).
pub struct EmptyWorld;

impl EmitterWorld for EmptyWorld {}

pub struct SoundSystem {
    pub backend: Box<dyn AudioBackend>,
    pub sounds: GameSounds,
    pub manager: SoundManager,
    pub world_emitters: crate::world_emitters::WorldEmitters,
    pub world: Box<dyn EmitterWorld>,
    /// `Rand.Next(n)`.
    pub rand_next: Box<dyn FnMut(i32) -> i32>,
    /// `ZomboidFileSystem.getAbsolutePath(path) != null`.
    pub media_file_exists: Box<dyn Fn(&str) -> bool>,
}

impl SoundSystem {
    pub fn new(backend: Box<dyn AudioBackend>, env: SoundEnv, parameter_count: usize) -> Self {
        SoundSystem {
            backend,
            sounds: GameSounds::new(env),
            manager: SoundManager::new(parameter_count),
            world_emitters: crate::world_emitters::WorldEmitters::new(parameter_count),
            world: Box::new(EmptyWorld),
            // Install the game's Rand; until then the first clip is always chosen.
            rand_next: Box::new(|_| 0),
            media_file_exists: Box::new(|_| false),
        }
    }

    /// A system that plays nothing (sound disabled or headless).
    pub fn silent() -> Self {
        Self::new(
            Box::new(SilentBackend),
            SoundEnv {
                sound_disabled: true,
                ..Default::default()
            },
            0,
        )
    }

    /// Run `f` with the manager and a context over the rest.
    pub fn with<R>(&mut self, f: impl FnOnce(&mut SoundManager, &mut Ctx) -> R) -> R {
        let SoundSystem {
            backend,
            sounds,
            manager,
            world,
            rand_next,
            media_file_exists,
            ..
        } = self;
        let mut ctx = Ctx {
            backend: backend.as_mut(),
            sounds,
            world: world.as_ref(),
            rand_next: rand_next.as_mut(),
            media_file_exists: media_file_exists.as_ref(),
        };
        f(manager, &mut ctx)
    }

    /// Run world/item emitter work against the same real backend and sound
    /// catalog as SoundManager, without borrowing or reusing its UI emitter.
    pub fn with_world_emitters<R>(
        &mut self,
        f: impl FnOnce(&mut crate::world_emitters::WorldEmitters, &mut Ctx) -> R,
    ) -> R {
        let SoundSystem {
            backend,
            sounds,
            world_emitters,
            world,
            rand_next,
            media_file_exists,
            ..
        } = self;
        let mut ctx = Ctx {
            backend: backend.as_mut(),
            sounds,
            world: world.as_ref(),
            rand_next: rand_next.as_mut(),
            media_file_exists: media_file_exists.as_ref(),
        };
        f(world_emitters, &mut ctx)
    }
}
