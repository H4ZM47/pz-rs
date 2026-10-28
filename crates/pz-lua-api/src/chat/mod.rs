//! `zombie.chat`: the chat system as a single-player game runs it.
//!
//! [`ChatManager`] is `ChatManager.getInstance()`; [`ChatManager::init`] with
//! `single_player = true` creates the say, general, shout, radio and admin
//! chats the way the original does (no tabs, no server chat, no faction,
//! safehouse or whisper chats). [`ChatElement`] is the per-character speech
//! line stack (`IsoGameCharacter.chatElement`), drawn through
//! [`crate::hud::text_draw::TextDrawObject`].
//!
//! Single player only: the multiplayer paths (join/leave/tab packets,
//! sending to the server or to chat members, member lists, whisper chat
//! creation round trips, `VoiceManagerData` radios, the "client chat"
//! log file) are not ported. Where an SP call reaches one of them the port
//! does what the original does there (usually a `NullPointerException`
//! on the missing server chat or connection, raised as a [`ChatError`]).
//!
//! The character side comes through [`ChatPlayer`], [`ChatElementOwner`]
//! and [`ChatPlayers`] ([`install_players`]); the characters thread
//! implements them. Lua events the chat raises are queued and fired by
//! [`fire_pending_events`] (the Lua bindings call it after each call).

pub mod base;
pub mod characters;
pub mod element;
pub mod lua;
pub mod manager;
pub mod message;
pub mod settings;
pub mod utility;

#[cfg(test)]
mod tests;

pub use base::{ChatBase, ChatKind, ChatRef, ChatTab};
pub use element::ChatElement;
pub use manager::{with_chat_manager, ChatManager};
pub use message::ChatMessage;
pub use settings::{ChatSettings, FontSize};

use pz_core::debug::{LocalTime, LogClock, SystemClock};
use pz_kahlua::{LuaError, Thread, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// `zombie.network.chat.ChatType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatType {
    NotDefined,
    General,
    Whisper,
    Say,
    Shout,
    Faction,
    Safehouse,
    Radio,
    Admin,
    Server,
}

impl ChatType {
    pub const ALL: [ChatType; 10] = [
        ChatType::NotDefined,
        ChatType::General,
        ChatType::Whisper,
        ChatType::Say,
        ChatType::Shout,
        ChatType::Faction,
        ChatType::Safehouse,
        ChatType::Radio,
        ChatType::Admin,
        ChatType::Server,
    ];

    /// The Java constant name (`toString()`).
    pub fn name(self) -> &'static str {
        match self {
            ChatType::NotDefined => "notDefined",
            ChatType::General => "general",
            ChatType::Whisper => "whisper",
            ChatType::Say => "say",
            ChatType::Shout => "shout",
            ChatType::Faction => "faction",
            ChatType::Safehouse => "safehouse",
            ChatType::Radio => "radio",
            ChatType::Admin => "admin",
            ChatType::Server => "server",
        }
    }

    /// `getValue()`.
    pub fn value(self) -> i32 {
        self as i32 - 1
    }

    /// `ChatType.valueOf(Integer)`: `notDefined` for unknown values.
    pub fn from_value(v: i32) -> ChatType {
        ChatType::ALL.into_iter().skip(1).find(|t| t.value() == v).unwrap_or(ChatType::NotDefined)
    }

    /// `getTitleID()`.
    pub fn title_id(self) -> &'static str {
        match self {
            ChatType::NotDefined => "",
            ChatType::General => "UI_chat_general_chat_title_id",
            ChatType::Whisper => "UI_chat_private_chat_title_id",
            ChatType::Say | ChatType::Shout => "UI_chat_local_chat_title_id",
            ChatType::Faction => "UI_chat_faction_chat_title_id",
            ChatType::Safehouse => "UI_chat_safehouse_chat_title_id",
            ChatType::Radio => "UI_chat_radio_chat_title_id",
            ChatType::Admin => "UI_chat_admin_chat_title_id",
            ChatType::Server => "UI_chat_server_chat_title_id",
        }
    }
}

/// `zombie.chat.ChatMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatMode {
    ServerMultiPlayer,
    ClientMultiPlayer,
    SinglePlayer,
}

impl ChatMode {
    pub fn name(self) -> &'static str {
        match self {
            ChatMode::ServerMultiPlayer => "ServerMultiPlayer",
            ChatMode::ClientMultiPlayer => "ClientMultiPlayer",
            ChatMode::SinglePlayer => "SinglePlayer",
        }
    }
}

/// A Java exception the original throws on a chat path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatError {
    pub class: &'static str,
    pub message: String,
}

impl ChatError {
    pub fn new(class: &'static str, message: impl Into<String>) -> Self {
        ChatError { class, message: message.into() }
    }
    pub fn npe(message: impl Into<String>) -> Self {
        Self::new("java.lang.NullPointerException", message)
    }
    pub fn illegal_argument(message: impl Into<String>) -> Self {
        Self::new("java.lang.IllegalArgumentException", message)
    }
}

impl From<ChatError> for LuaError {
    fn from(e: ChatError) -> LuaError {
        LuaError::Runtime { message: e.message, class: e.class }
    }
}

pub type ChatResult<T> = Result<T, ChatError>;

// ---- Character side ---------------------------------------------------------------

/// The square a speaker or listener stands on, as far as the chat compares
/// them: opaque building and room identities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChatSquare {
    pub building: Option<u64>,
    pub room: Option<u64>,
}

/// What a [`ChatElement`]'s owner is (the original's `instanceof` checks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChatOwnerKind {
    #[default]
    Character,
    Television,
    Radio,
    VehiclePart,
    Other,
}

/// `zombie.chat.ChatElementOwner` plus what `ChatElement` asks of its
/// owner's concrete class.
pub trait ChatElementOwner {
    fn get_x(&self) -> f32;
    fn get_y(&self) -> f32;
    fn get_z(&self) -> f32;
    fn get_square(&self) -> Option<ChatSquare>;
    fn kind(&self) -> ChatOwnerKind {
        ChatOwnerKind::Character
    }
    /// `owner == IsoPlayer.players[player_num]`.
    fn is_player(&self, _player_num: usize) -> bool {
        false
    }
    /// `IsoTelevision.isFacing(player)` (televisions only).
    fn is_facing(&self, _player: &dyn ChatPlayer) -> bool {
        false
    }
}

/// `Radio.getDeviceData()` as the chat reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RadioDeviceState {
    pub turned_on: bool,
    pub two_way: bool,
    pub portable: bool,
    pub mic_muted: bool,
}

/// An `IsoPlayer` as the chat reads it.
pub trait ChatPlayer {
    /// The index in `IsoPlayer.players`.
    fn player_num(&self) -> usize;
    fn username(&self) -> String;
    /// `getChatElement()`.
    fn chat_element(&self) -> Rc<RefCell<ChatElement>>;
    /// `getSpeakColour()` (r, g, b).
    fn speak_colour(&self) -> (f32, f32, f32) {
        (1.0, 1.0, 1.0)
    }
    fn is_dead(&self) -> bool {
        false
    }
    fn get_x(&self) -> f32;
    fn get_y(&self) -> f32;
    fn get_z(&self) -> f32;
    /// `getSquare()` (`getSquare().getRoom()` is its `room`).
    fn get_square(&self) -> Option<ChatSquare>;
    /// `getBuilding()`.
    fn get_building(&self) -> Option<u64> {
        self.get_square().and_then(|s| s.building)
    }
    /// `hasTrait(CharacterTrait.DEAF)`.
    fn is_deaf(&self) -> bool {
        false
    }
    /// `isAccessLevel(level)`.
    fn is_access_level(&self, _level: &str) -> bool {
        false
    }
    /// `getEquipedRadio()` with device data.
    fn equipped_radio(&self) -> Option<RadioDeviceState> {
        None
    }
    /// `Faction.isAlreadyInFaction(player)`.
    fn is_in_faction(&self) -> bool {
        false
    }
    /// `SafeHouse.hasSafehouse(player) != null`.
    fn has_safehouse(&self) -> bool {
        false
    }
}

/// The `IsoPlayer` statics the chat reads.
pub trait ChatPlayers {
    /// `IsoPlayer.numPlayers`.
    fn num_players(&self) -> usize {
        0
    }
    /// `IsoPlayer.players[i]`.
    fn player(&self, _i: usize) -> Option<Rc<dyn ChatPlayer>> {
        None
    }
    /// `IsoPlayer.getInstance()`.
    fn instance(&self) -> Option<Rc<dyn ChatPlayer>> {
        None
    }
}

struct NoPlayers;
impl ChatPlayers for NoPlayers {}

/// `bind_climate::EventSink`'s shape: `LuaEventManager.triggerEvent`.
pub type EventSink = Rc<dyn Fn(&mut Thread, &str, &[Value])>;

thread_local! {
    static PLAYERS: RefCell<Rc<dyn ChatPlayers>> = RefCell::new(Rc::new(NoPlayers));
    static SINK: RefCell<Option<EventSink>> = const { RefCell::new(None) };
    static PENDING: RefCell<Vec<(&'static str, Vec<Value>)>> = const { RefCell::new(Vec::new()) };
    static CLOCK: RefCell<Option<Rc<dyn LogClock>>> = const { RefCell::new(None) };
    /// `ChatUtility.allowedChatIcons` / `allowedChatIconsFull`.
    static ICONS: RefCell<(HashMap<String, String>, HashMap<String, String>)> = RefCell::default();
}

/// Makes `p` the players the chat reads (characters thread).
pub fn install_players(p: Rc<dyn ChatPlayers>) {
    PLAYERS.with(|s| *s.borrow_mut() = p);
}

pub(crate) fn players() -> Rc<dyn ChatPlayers> {
    PLAYERS.with(|s| s.borrow().clone())
}

/// Where chat Lua events go.
pub fn install_event_sink(sink: EventSink) {
    SINK.with(|s| *s.borrow_mut() = Some(sink));
}

/// Pending Lua values and their callback sink belong to the environment
/// that produced them; a fresh environment must not receive that queue.
pub(crate) fn reset_session_callbacks() {
    PENDING.with(|pending| pending.borrow_mut().clear());
    SINK.with(|sink| *sink.borrow_mut() = None);
}

/// `LuaEventManager.triggerEvent(name, args)`, deferred to
/// [`fire_pending_events`].
pub(crate) fn trigger_event(name: &'static str, args: Vec<Value>) {
    PENDING.with(|p| p.borrow_mut().push((name, args)));
}

/// Fires the queued chat events in order (dropped without a sink).
pub fn fire_pending_events(thread: &mut Thread) {
    let pending = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    let Some(sink) = SINK.with(|s| s.borrow().clone()) else { return };
    for (name, args) in pending {
        sink(thread, name, &args);
    }
}

/// Replaces the wall clock used for message times and `[hh:mm]` stamps.
pub fn set_clock(clock: Rc<dyn LogClock>) {
    CLOCK.with(|c| *c.borrow_mut() = Some(clock));
}

/// `LocalDateTime.now()` / `Calendar.getInstance()`.
pub(crate) fn now() -> LocalTime {
    let clock = CLOCK.with(|c| c.borrow_mut().get_or_insert_with(|| Rc::new(SystemClock::new())).clone());
    let ms = clock.now_millis();
    LocalTime::from_epoch(ms, clock.local_offset_millis(ms))
}

/// `ChatUtility.InitAllowedChatIcons()`: `Texture.collectAllIcons` fills
/// `icons` (lower-case short name -> texture name, for bubbles) and `full`
/// (for the chat log); the texture scan is the engine's.
pub fn set_allowed_chat_icons(icons: HashMap<String, String>, full: HashMap<String, String>) {
    ICONS.with(|i| *i.borrow_mut() = (icons, full));
}

pub(crate) fn with_icons<R>(f: impl FnOnce(&HashMap<String, String>, &HashMap<String, String>) -> R) -> R {
    ICONS.with(|i| {
        let i = i.borrow();
        f(&i.0, &i.1)
    })
}

/// `Core.debug`.
pub(crate) fn core_debug() -> bool {
    crate::bind_core::shared_core().borrow().debug
}

/// `DebugLog.log(text)`.
pub(crate) fn debug_log(text: &str) {
    pz_core::debug::with(|d| d.log(pz_core::debug::DebugType::General, text));
}

/// `Translator.getText(key, args...)` with string arguments.
pub(crate) fn get_text(key: &str, args: &[&str]) -> String {
    let args: Vec<pz_text::JavaArg> = args.iter().map(|a| pz_text::JavaArg::Str(a.to_string())).collect();
    crate::bind_text::with_translator(|t| t.get_text(key, &args)).unwrap_or_else(|| key.to_string())
}

/// Java `String.trim()`.
pub(crate) fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}
