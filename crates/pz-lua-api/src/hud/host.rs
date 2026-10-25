//! What the HUD widgets read from other workstreams, as small input traits
//! with no-op defaults. Each trait names the owner that implements it;
//! the engine installs the real ones with [`install`].

use std::cell::RefCell;

/// `IsoPlayer` statics the HUD reads. Owner: characters thread.
pub trait HudPlayers {
    /// `IsoPlayer.getInstance() != null`.
    fn has_player_instance(&self) -> bool {
        false
    }
    /// `IsoPlayer.numPlayers`.
    fn num_players(&self) -> i32 {
        1
    }
    /// `IsoPlayer.players[i] != null`.
    fn player_exists(&self, _i: usize) -> bool {
        false
    }
    /// `IsoPlayer.allPlayersDead()`.
    fn all_players_dead(&self) -> bool {
        false
    }
}

/// `GameTime.instance` as the HUD uses it. Owner: characters thread.
pub trait HudGameTime {
    /// `setMultiplier(m)`.
    fn set_multiplier(&mut self, _m: f32) {}
    /// `getMultiplier()` (all factors).
    fn get_multiplier(&self) -> f32 {
        0.8
    }
    /// `getTrueMultiplier()`.
    fn get_true_multiplier(&self) -> f32 {
        1.0
    }
}

/// `SoundManager.instance` calls the HUD makes. Owner: audio thread.
pub trait HudSound {
    fn pause_sound_and_music(&mut self, _keep_music: bool) {}
    fn resume_sound_and_music(&mut self) {}
    fn set_music_state(&mut self, _state: &str) {}
    fn play_ui_sound(&mut self, _name: &str) {}
}

/// `GameClient.client` / `GameServer.server`. Owner: network thread.
pub trait HudNetwork {
    fn is_client(&self) -> bool {
        false
    }
    fn is_server(&self) -> bool {
        false
    }
}

/// The no-op defaults.
pub struct NoHud;
impl HudPlayers for NoHud {}
impl HudSound for NoHud {}
impl HudNetwork for NoHud {}

/// [`HudGameTime`] over the `GameTime` the Lua bindings share
/// (`bind_climate::shared_game_time`), the default until the engine
/// installs its own.
pub struct SharedGameTime;

impl HudGameTime for SharedGameTime {
    fn set_multiplier(&mut self, m: f32) {
        crate::bind_climate::shared_game_time()
            .borrow_mut()
            .multiplier = m;
    }
    fn get_multiplier(&self) -> f32 {
        let ctx = crate::bind_climate::LuaClimateContext::new();
        crate::bind_climate::shared_game_time()
            .borrow()
            .get_multiplier(&ctx)
    }
    fn get_true_multiplier(&self) -> f32 {
        crate::bind_climate::shared_game_time()
            .borrow()
            .get_true_multiplier()
    }
}

/// The installed inputs.
pub struct HudHost {
    pub players: Box<dyn HudPlayers>,
    pub game_time: Box<dyn HudGameTime>,
    pub sound: Box<dyn HudSound>,
    pub network: Box<dyn HudNetwork>,
}

impl Default for HudHost {
    fn default() -> Self {
        HudHost {
            players: Box::new(NoHud),
            game_time: Box::new(SharedGameTime),
            sound: Box::new(NoHud),
            network: Box::new(NoHud),
        }
    }
}

thread_local! {
    static HOST: RefCell<HudHost> = RefCell::new(HudHost::default());
}

/// Replace the HUD inputs.
pub fn install(h: HudHost) {
    HOST.with(|s| *s.borrow_mut() = h);
}

/// Run `f` on the installed inputs. Implementations must not call Lua.
pub fn with_host<R>(f: impl FnOnce(&mut HudHost) -> R) -> R {
    HOST.with(|s| f(&mut s.borrow_mut()))
}
