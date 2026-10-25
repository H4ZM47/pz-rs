//! The radio and television broadcasts (`zombie.radio`): the radio script
//! files (`RadioData`), channels, scripts, broadcasts and lines, the
//! broadcast schedule of `ZomboidRadio` and `RadioScriptManager`, the
//! distribution of lines and player speech to devices, the radio save
//! (`radio/data/RADIO_SAVE.txt`) and the recorded media list
//! (`recorded_media.bin`).
//!
//! All game data (the XML scripts, the `RD_` line texts, the media the Lua
//! side registers) is read at runtime; the world side (time, players,
//! devices, Lua events) is reached through [`RadioWorld`] and the game RNG
//! is passed in as `FnMut(bound) -> Random.nextInt(bound)`.

pub mod channel;
pub mod data;
pub mod java;
pub mod manager;
pub mod media;
pub mod radio;
pub mod script;
pub mod xml;

#[cfg(test)]
mod tests;

pub use channel::{AiredLine, RadioChannel, RadioDataRef};
pub use data::{ModRadioDirs, RadioData, RadioSource, RadioText, RadioTranslationData};
pub use manager::{ChannelNames, LoadError, RadioScriptManager};
pub use media::{MediaData, MediaLineData, RecordedMedia};
pub use radio::{
    is_static_sound, Airwaves, DeviceId, DeviceView, PlayerView, RadioNet, RadioWorld,
    Transmission, ZomboidRadio, RADIO_SAVE_FILE,
};
pub use script::{BroadcastId, Broadcasts, ExitOption, RadioBroadCast, RadioLine, RadioScript};

/// An exception the original catches around the radio loading (a missing
/// attribute, a number that does not parse, an unknown category).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadioError(pub String);

impl std::fmt::Display for RadioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RadioError {}

/// `zombie.radio.ChannelCategory`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChannelCategory {
    #[default]
    Undefined,
    Radio,
    Television,
    Military,
    Amateur,
    Bandit,
    Emergency,
    Other,
}

impl ChannelCategory {
    pub const ALL: [ChannelCategory; 8] = [
        ChannelCategory::Undefined,
        ChannelCategory::Radio,
        ChannelCategory::Television,
        ChannelCategory::Military,
        ChannelCategory::Amateur,
        ChannelCategory::Bandit,
        ChannelCategory::Emergency,
        ChannelCategory::Other,
    ];

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            ChannelCategory::Undefined => "Undefined",
            ChannelCategory::Radio => "Radio",
            ChannelCategory::Television => "Television",
            ChannelCategory::Military => "Military",
            ChannelCategory::Amateur => "Amateur",
            ChannelCategory::Bandit => "Bandit",
            ChannelCategory::Emergency => "Emergency",
            ChannelCategory::Other => "Other",
        }
    }

    /// `valueOf(name)` (exact, case-sensitive).
    pub fn value_of(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// `zombie.radio.GameMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GameMode {
    #[default]
    SinglePlayer,
    Server,
    Client,
}
