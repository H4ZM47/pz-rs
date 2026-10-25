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
    /// `IsoPlayer.players[i]`'s position and draw offsets, for HUD
    /// widgets that follow the player on screen; `None` without a player.
    fn player_screen(&self, _i: usize) -> Option<PlayerScreen> {
        None
    }
    /// `IsoPlayer.players[i].isDead()`.
    fn player_dead(&self, _i: usize) -> bool {
        false
    }
}

/// What `ActionProgressBar.updateScreenPos` reads from a player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerScreen {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `IsoGameCharacter.offsetX` / `offsetY`.
    pub offset_x: f32,
    pub offset_y: f32,
    /// `getUserNameHeight()`.
    pub user_name_height: i32,
}

/// `IsoCamera` / `Core` zoom as the HUD reads them. Owner: world-render
/// thread.
pub trait HudCamera {
    /// `IsoCamera.getOffX()` / `getOffY()` (of the player being rendered).
    fn off_x(&self) -> f32 {
        0.0
    }
    fn off_y(&self) -> f32 {
        0.0
    }
    /// `Core.getZoom(player)`.
    fn zoom(&self, _player: usize) -> f32 {
        1.0
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
    /// `getRealworldSecondsSinceLastUpdate()`.
    fn realworld_seconds_since_last_update(&self) -> f32 {
        1.0 / 60.0
    }
    /// `getTimeOfDay()` (or `serverTimeOfDay` for an MP client while the
    /// server fast-forwards: `GameClient.fastForward`).
    fn time_of_day(&self) -> f32 {
        9.0
    }
    /// `getDay()` (0-based).
    fn day(&self) -> i32 {
        0
    }
    /// `getMonth()` (0-based).
    fn month(&self) -> i32 {
        0
    }
}

/// `ClimateManager` as the HUD reads it. Owner: characters thread.
pub trait HudClimate {
    /// `getAirTemperatureForCharacter(players[player], false)` (Celsius).
    fn air_temperature_for_character(&self, _player: usize) -> f32 {
        0.0
    }
}

/// An `AlarmClock` / `AlarmClockClothing` item as `Clock` reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AlarmClockItem {
    /// `AlarmClockClothing` (else `AlarmClock`).
    pub clothing: bool,
    /// `hasTag(ItemTag.DIGITAL)`.
    pub digital_tag: bool,
    /// `isDigital()`.
    pub is_digital: bool,
    pub alarm_set: bool,
    pub ringing: bool,
    /// `isWorn()` / `isEquipped()` (inventory items).
    pub worn: bool,
    pub equipped: bool,
}

/// Where an alarm clock is: in `getWornItems()` or `getInventory().getItems()`,
/// by its index among that list's alarm clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlarmClockRef {
    Worn(usize),
    Inventory(usize),
}

/// A player's alarm clocks. Owner: world thread (items).
pub trait HudAlarmClocks {
    /// The alarm clocks among `players[player].getWornItems()`, in order.
    fn worn_clocks(&self, _player: usize) -> Vec<AlarmClockItem> {
        Vec::new()
    }
    /// The alarm clocks among `players[player].getInventory().getItems()`,
    /// in order.
    fn inventory_clocks(&self, _player: usize) -> Vec<AlarmClockItem> {
        Vec::new()
    }
    /// `stopRinging()`.
    fn stop_ringing(&mut self, _player: usize, _item: AlarmClockRef) {}
    /// `setAlarmSet(set)` then `syncAlarmClock()`.
    fn set_alarm_set(&mut self, _player: usize, _item: AlarmClockRef, _set: bool) {}
}

/// `DebugOptions` the HUD reads. Owner: us (debug options).
pub trait HudDebug {
    /// `DebugOptions.instance.cheat.clock.visible`.
    fn cheat_clock_visible(&self) -> bool {
        false
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
impl HudCamera for NoHud {}
impl HudClimate for NoHud {}
impl HudAlarmClocks for NoHud {}
impl HudDebug for NoHud {}
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
    pub camera: Box<dyn HudCamera>,
    pub climate: Box<dyn HudClimate>,
    pub alarm_clocks: Box<dyn HudAlarmClocks>,
    pub debug: Box<dyn HudDebug>,
}

impl Default for HudHost {
    fn default() -> Self {
        HudHost {
            players: Box::new(NoHud),
            game_time: Box::new(SharedGameTime),
            sound: Box::new(NoHud),
            network: Box::new(NoHud),
            camera: Box::new(NoHud),
            climate: Box::new(NoHud),
            alarm_clocks: Box::new(NoHud),
            debug: Box::new(NoHud),
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
