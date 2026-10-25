//! Pure-Rust port of the game's `PZPopMan` native library
//! (`libPZPopMan.dylib` / `PZPopMan64.dll` / `libPZPopMan64.so`), the JNI
//! backend of `zombie.popman.ZombiePopulationManager` and
//! `zombie.MapCollisionData`.
//!
//! The library runs a "main" side (the JNI calls the game thread makes) and a
//! "worker" side (`n_updateThread` / `MapCollisionData.n_update`, called from
//! the `MapCollisionDataJNI` thread) that exchange batches through two
//! single-producer queues. This port keeps the same two halves and the same
//! batching, recycling and ordering, but runs them on whichever thread calls
//! in: [`Native`] is one value owning the whole library state, and each
//! `n_*` method is the JNI function of the same name.
//!
//! The library's globals that the game sets through
//! `MapCollisionData.n_setGameState` live in [`GameState`]; its clock
//! (`std::chrono::system_clock`) is [`GameState::clock`] and its random
//! generator (a `std::mt19937` seeded from `/dev/urandom`) is
//! [`Native::rand`].
//!
//! Reconstructed from the shipped arm64 macOS binary's behaviour; floating
//! point follows that build (fused multiply-adds where clang contracted
//! them). Known differences are listed in the crate README.

pub mod flood_fill;
pub mod mcd;
pub mod popman;
pub mod rand;
pub mod stream;
pub mod wall_follower;

use std::path::PathBuf;

pub use popman::{Native, Zombie, ZombieRecord};

/// The library's game-state globals (`n_setGameState`) and clock.
pub struct GameState {
    pub game_mode: String,
    pub game_save_world: String,
    pub last_stand: bool,
    pub no_save: bool,
    pub cache_dir: String,
    pub game_mode_cache_dir: String,
    pub save_dir: String,
    /// `SandboxOptions.Distribution` (2 = uniform).
    pub distribution: i32,
    /// `SandboxOptions.Zombies` (stored, unused).
    pub zombies: i32,
    pub subdir_chunkdata: String,
    pub subdir_zpop: String,
    pub zombies_disabled: bool,
    /// `GameServer::bServer`.
    pub server: bool,
    /// `GameTime::instance` as the worker last saw it: the multiplier and
    /// the world age in hours.
    pub multiplier: f32,
    pub world_age_hours: f64,
    /// `System::currentTimeMillis()`.
    pub clock: Box<dyn Fn() -> i64>,
}

impl Default for GameState {
    fn default() -> Self {
        GameState {
            game_mode: String::new(),
            game_save_world: String::new(),
            last_stand: false,
            no_save: false,
            cache_dir: String::new(),
            game_mode_cache_dir: String::new(),
            save_dir: String::new(),
            distribution: 0,
            zombies: 0,
            subdir_chunkdata: String::new(),
            subdir_zpop: String::new(),
            zombies_disabled: false,
            server: false,
            multiplier: 0.0,
            world_age_hours: 0.0,
            clock: Box::new(system_millis),
        }
    }
}

/// Wall-clock milliseconds, as the library reads them.
pub fn system_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64 / 1000)
        .unwrap_or(0)
}

impl GameState {
    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    /// `GameModeCacheDir + GameSaveWorld + '/' + subdir + '/' + name`.
    pub fn save_file(&self, subdir: &str, name: &str) -> PathBuf {
        PathBuf::from(format!("{}{}/{}/{}", self.game_mode_cache_dir, self.game_save_world, subdir, name))
    }

    /// `n_setGameState(String, boolean)`.
    pub fn set_bool(&mut self, key: &str, v: bool) -> Result<(), String> {
        match key {
            "Core.bLastStand" => self.last_stand = v,
            "Core.noSave" => self.no_save = v,
            "World.ZombiesDisabled" => self.zombies_disabled = v,
            // "PAUSED" is handled by `Native::set_game_state_bool`.
            _ => return Err(format!("unknown key {key}")),
        }
        Ok(())
    }

    /// `n_setGameState(String, int)`.
    pub fn set_int(&mut self, key: &str, v: i32) -> Result<(), String> {
        match key {
            "SandboxOptions.Distribution" => self.distribution = v,
            "SandboxOptions.Zombies" => self.zombies = v,
            _ => return Err(format!("unknown key {key}")),
        }
        Ok(())
    }

    /// `n_setGameState(String, String)`.
    pub fn set_string(&mut self, key: &str, v: &str) -> Result<(), String> {
        let slot = match key {
            "Core.GameMode" => &mut self.game_mode,
            "Core.GameSaveWorld" => &mut self.game_save_world,
            "GameWindow.CacheDir" => &mut self.cache_dir,
            "GameWindow.GameModeCacheDir" => &mut self.game_mode_cache_dir,
            "GameWindow.SaveDir" => &mut self.save_dir,
            "SavefileNaming.SUBDIR_CHUNKDATA" => &mut self.subdir_chunkdata,
            "SavefileNaming.SUBDIR_ZPOP" => &mut self.subdir_zpop,
            _ => return Err(format!("unknown key {key}")),
        };
        *slot = v.to_string();
        Ok(())
    }
}

/// `std::floor` as the library computes it for int conversions
/// (`(int)v - (v < (float)(int)v)`).
pub fn ffloor(v: f32) -> i32 {
    let t = v as i32;
    t - ((v < t as f32) as i32)
}
