//! `zombie.chat.ChatManager`, single-player path.

use super::base::{ChatBase, ChatKind, ChatRef, ChatTab};
use super::message::ChatMessage;
use super::utility::{chat_stream_enabled, get_allowed_chat_streams};
use super::{core_debug, debug_log, get_text, java_trim, trigger_event, ChatError, ChatPlayer, ChatResult, ChatType};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    NotStarted,
    Starting,
    Working,
}

/// `zombie.chat.ChatManager`.
pub struct ChatManager {
    /// Chats joined through the server's packets (none in single player).
    mp_chats: HashMap<i32, ChatRef>,
    /// Pending whisper chats by recipient (the server never answers here).
    whisper_chat_creation: HashMap<String, Vec<String>>,
    tabs: HashMap<i16, Rc<ChatTab>>,
    focus_tab: Option<Rc<ChatTab>>,
    player: Option<Rc<dyn ChatPlayer>>,
    my_nickname: Option<String>,
    single_player_mode: bool,
    general_chat: Option<ChatRef>,
    say_chat: Option<ChatRef>,
    shout_chat: Option<ChatRef>,
    radio_chat: Option<ChatRef>,
    admin_chat: Option<ChatRef>,
    server_chat: Option<ChatRef>,
    stage: Stage,
}

thread_local! {
    static INSTANCE: RefCell<Option<Rc<RefCell<ChatManager>>>> = const { RefCell::new(None) };
}

/// `ChatManager.getInstance()` (created on first use).
pub fn instance() -> Rc<RefCell<ChatManager>> {
    INSTANCE.with(|i| i.borrow_mut().get_or_insert_with(|| Rc::new(RefCell::new(ChatManager::new()))).clone())
}

/// Runs `f` on `ChatManager.getInstance()`.
pub fn with_chat_manager<R>(f: impl FnOnce(&mut ChatManager) -> R) -> R {
    f(&mut instance().borrow_mut())
}

fn npe(what: &str) -> ChatError {
    ChatError::npe(format!("{what} is null"))
}

impl ChatManager {
    fn new() -> Self {
        ChatManager {
            mp_chats: HashMap::new(),
            whisper_chat_creation: HashMap::new(),
            tabs: HashMap::new(),
            focus_tab: None,
            player: None,
            my_nickname: None,
            single_player_mode: false,
            general_chat: None,
            say_chat: None,
            shout_chat: None,
            radio_chat: None,
            admin_chat: None,
            server_chat: None,
            stage: Stage::NotStarted,
        }
    }

    pub fn is_single_player_mode(&self) -> bool {
        self.single_player_mode
    }

    /// `isWorking()`: only the server's "fully connected" packet sets it.
    pub fn is_working(&self) -> bool {
        self.stage == Stage::Working
    }

    fn chats(&self) -> impl Iterator<Item = &ChatRef> {
        [&self.general_chat, &self.say_chat, &self.shout_chat, &self.radio_chat, &self.admin_chat, &self.server_chat]
            .into_iter()
            .flatten()
            .chain(self.mp_chats.values())
    }

    /// `init(isSinglePlayer, owner)`. In single player: the say, general,
    /// shout, radio and admin chats, the range-based ones speaking through
    /// the player's chat element. Multiplayer only fires
    /// `OnChatWindowInit` here (the server's packets do the rest).
    pub fn init(&mut self, single_player: bool, owner: Rc<dyn ChatPlayer>) -> ChatResult<()> {
        self.stage = Stage::Starting;
        self.single_player_mode = single_player;
        for c in self.chats() {
            c.clear_messages();
        }
        self.general_chat = None;
        self.say_chat = None;
        self.shout_chat = None;
        self.radio_chat = None;
        self.admin_chat = None;
        self.server_chat = None;
        self.mp_chats.clear();
        self.tabs.clear();
        self.focus_tab = None;
        self.my_nickname = Some(owner.username());
        self.player = Some(owner);
        if single_player {
            let say = ChatBase::new_single_player(ChatKind::Say);
            self.say_chat = Some(say.clone());
            say.init_range_based()?;
            self.general_chat = Some(ChatBase::new_single_player(ChatKind::General));
            let shout = ChatBase::new_single_player(ChatKind::Shout);
            self.shout_chat = Some(shout.clone());
            shout.init_range_based()?;
            let radio = ChatBase::new_single_player(ChatKind::Radio);
            self.radio_chat = Some(radio.clone());
            radio.init_range_based()?;
            self.admin_chat = Some(ChatBase::new_single_player(ChatKind::Admin));
        } else {
            trigger_event("OnChatWindowInit", Vec::new());
        }
        Ok(())
    }

    /// The chat of `t` this manager holds (`getChat`'s fields).
    pub fn chat(&self, t: ChatType) -> Option<ChatRef> {
        match t {
            ChatType::General => self.general_chat.clone(),
            ChatType::Say => self.say_chat.clone(),
            ChatType::Shout => self.shout_chat.clone(),
            ChatType::Radio => self.radio_chat.clone(),
            ChatType::Admin => self.admin_chat.clone(),
            ChatType::Server => self.server_chat.clone(),
            _ => None,
        }
    }

    /// `getChat(type)`: whisper chats are per recipient; the faction and
    /// safehouse chats only come from the server.
    fn get_chat(&self, t: ChatType) -> ChatResult<Option<ChatRef>> {
        match t {
            ChatType::Whisper => Err(ChatError::illegal_argument("Whisper not unique chat")),
            ChatType::NotDefined => Err(ChatError::illegal_argument("Chat type is undefined")),
            _ => Ok(self.chat(t)),
        }
    }

    /// `updateChatSettings(fontSize, showTimestamp, showTitle)`: the
    /// `Core` options, and the server's chats (the single-player chats keep
    /// their settings).
    pub fn update_chat_settings(&mut self, font_size: Option<&str>, show_timestamp: bool, show_title: bool) {
        {
            let core = crate::bind_core::shared_core();
            let mut core = core.borrow_mut();
            core.set_option_chat_font_size(font_size);
            core.set_option_show_chat_timestamp(show_timestamp);
            core.set_option_show_chat_title(show_title);
        }
        for c in self.mp_chats.values() {
            c.set_font_size(font_size.unwrap_or_default());
            c.set_show_timestamp(show_timestamp);
            c.set_show_title(show_title);
        }
    }

    /// `showInfoMessage(msg)`: a local bubble from the player.
    pub fn show_info_message(&mut self, msg: &str) -> ChatResult<()> {
        let say = self.say_chat.clone().ok_or_else(|| npe("sayChat"))?;
        let m = say.create_info_message(Some(msg))?;
        say.show_message(&m)
    }

    /// `showInfoMessage(author, msg)`: ignored before init.
    pub fn show_info_message_from(&mut self, author: &str, msg: &str) -> ChatResult<()> {
        let Some(say) = self.say_chat.clone() else { return Ok(()) };
        let m = say.create_info_message(Some(msg))?;
        m.set_author(Some(author))?;
        say.show_message(&m)
    }

    /// `sendMessageToChat(author, type, msg)`.
    pub fn send_message_to_chat_as(&mut self, author: &str, t: ChatType, msg: &str) -> ChatResult<()> {
        let msg = java_trim(msg);
        if msg.is_empty() {
            return Ok(());
        }
        let Some(chat) = self.get_chat(t)? else {
            if core_debug() {
                return Err(ChatError::illegal_argument(format!(
                    "Chat '{}' is null. Chat should be init before use!",
                    t.name()
                )));
            }
            return self.show_chat_disabled_message(t);
        };
        let m = chat.create_message(Some(msg))?;
        m.set_author(Some(author))?;
        self.send(&chat, &m)
    }

    /// `sendMessageToChat(type, msg)`: as the player.
    pub fn send_message_to_chat(&mut self, t: ChatType, msg: &str) -> ChatResult<()> {
        let author = self.player.as_ref().ok_or_else(|| npe("player"))?.username();
        self.send_message_to_chat_as(&author, t, msg)
    }

    /// `sendWhisperMessage(destPlayerName, msg)`. Without a server the
    /// first message to a name fails on the missing connection after
    /// queuing the chat creation; later ones are queued silently.
    pub fn send_whisper_message(&mut self, dest: Option<&str>, msg: &str) -> ChatResult<()> {
        if !chat_stream_enabled(ChatType::Whisper) {
            return self.show_chat_disabled_message(ChatType::Whisper);
        }
        let me = self.my_nickname.clone().unwrap_or_default();
        let dest = match dest {
            Some(d) if d.to_lowercase() != me.to_lowercase() => d,
            _ => {
                let text = get_text("UI_chat_whisper_message_to_yourself_error", &[]);
                return self.show_server_chat_message(&text);
            }
        };
        if let Some(q) = self.whisper_chat_creation.get_mut(dest) {
            q.push(msg.to_string());
            return Ok(());
        }
        self.whisper_chat_creation.insert(dest.to_string(), Vec::new());
        Err(npe("serverConnection"))
    }

    /// `isPlayerCanUseChat(chat)`.
    pub fn is_player_can_use_chat(&self, t: ChatType) -> ChatResult<bool> {
        if !chat_stream_enabled(t) {
            return Ok(false);
        }
        let player = || self.player.as_ref().ok_or_else(|| npe("player"));
        Ok(match t {
            ChatType::Radio => self.is_player_can_use_radio_chat()?,
            ChatType::Admin => player()?.is_access_level("admin"),
            ChatType::Faction => player()?.is_in_faction(),
            ChatType::Safehouse => player()?.has_safehouse(),
            _ => true,
        })
    }

    /// `focusOnTab(id)`.
    pub fn focus_on_tab(&mut self, id: i16) -> ChatResult<()> {
        match self.tabs.values().find(|t| t.get_id() == id) {
            Some(t) => {
                self.focus_tab = Some(t.clone());
                Ok(())
            }
            None => Err(ChatError::new("java.lang.RuntimeException", format!("Tab with id = '{id}' not found"))),
        }
    }

    /// `getTabName(tabID)`: the title, or the id as text.
    pub fn get_tab_name(&self, id: i16) -> String {
        self.tabs.get(&id).map_or_else(|| id.to_string(), |t| t.get_title().to_string())
    }

    pub fn get_focus_tab(&self) -> Option<Rc<ChatTab>> {
        self.focus_tab.clone()
    }

    fn radio(&self) -> ChatResult<ChatRef> {
        self.radio_chat.clone().ok_or_else(|| npe("radioChat"))
    }

    /// `showRadioMessage(msg)`.
    pub fn show_radio_message(&mut self, msg: &Rc<ChatMessage>) -> ChatResult<()> {
        self.radio()?.show_message(msg)
    }

    /// `showRadioMessage(text, channel)`: channel 0 keeps -1.
    pub fn show_radio_message_text(&mut self, text: &str, channel: i32) -> ChatResult<()> {
        let radio = self.radio()?;
        let m = radio.create_message(Some(text))?;
        if channel != 0 {
            m.set_radio_channel(channel);
        }
        radio.show_message(&m)
    }

    /// `showStaticRadioSound(text)`.
    pub fn show_static_radio_sound(&mut self, text: &str) -> ChatResult<()> {
        let radio = self.radio()?;
        let m = radio.create_static_sound_message(Some(text))?;
        radio.show_message(&m)
    }

    /// `createRadiostationMessage(text, channel)`.
    pub fn create_radiostation_message(&self, text: &str, channel: i32) -> ChatResult<Rc<ChatMessage>> {
        self.radio()?.create_broadcasting_message(Some(text), channel)
    }

    /// `showServerChatMessage(msg)`: the server chat only exists in
    /// multiplayer, so single player gets the original's
    /// `NullPointerException`.
    pub fn show_server_chat_message(&mut self, msg: &str) -> ChatResult<()> {
        let server = self.server_chat.clone().ok_or_else(|| npe("serverChat"))?;
        let m = server.create_server_message(Some(msg));
        server.show_message(&m)
    }

    /// `addMessage(author, msg)`: into the general chat by id, which only
    /// the server's chats are registered under.
    pub fn add_message(&mut self, author: &str, msg: &str) -> ChatResult<()> {
        let general = self.general_chat.clone().ok_or_else(|| ChatError::new("java.lang.RuntimeException", ""))?;
        let chat = self.mp_chats.get(&general.get_id()).cloned().ok_or_else(|| npe("chat"))?;
        chat.show_message_text(Some(msg), Some(author))
    }

    /// `sendMessageToChat(chat, msg)`: shows it (single player sends
    /// nothing on); radio is only reached through the say chat.
    fn send(&mut self, chat: &ChatRef, msg: &Rc<ChatMessage>) -> ChatResult<()> {
        if chat.get_type() == ChatType::Radio {
            if core_debug() {
                return Err(ChatError::illegal_argument(
                    "You can't send message to radio directly. Use radio and send say message",
                ));
            }
            debug_log("You try to use radio chat directly. It's restricted. Try to use say chat");
            return Ok(());
        }
        chat.show_message(msg)?;
        if !chat.is_enabled() {
            return self.show_chat_disabled_message(chat.get_type());
        }
        Ok(())
    }

    /// `ChatManager.UpdateClient()`: once per frame. It only advances
    /// pending whisper chats once the server has connected the chat
    /// (`isWorking`), which never happens in single player.
    pub fn update_client() {}

    /// `showChatDisabledMessage(expectedChat)`: lists the usable streams
    /// in the server chat.
    fn show_chat_disabled_message(&mut self, expected: ChatType) -> ChatResult<()> {
        let title = get_text(expected.title_id(), &[]);
        let mut text = get_text("UI_chat_chat_disabled_msg", &[&title]);
        for t in get_allowed_chat_streams() {
            if self.is_player_can_use_chat(t)? {
                text.push_str(&format!("    * {} <LINE> ", get_text(t.title_id(), &[])));
            }
        }
        self.show_server_chat_message(&text)
    }

    /// `isPlayerCanUseRadioChat()`: an equipped two-way portable radio,
    /// on, mic not muted.
    fn is_player_can_use_radio_chat(&self) -> ChatResult<bool> {
        let player = self.player.as_ref().ok_or_else(|| npe("player"))?;
        Ok(player.equipped_radio().is_some_and(|d| d.turned_on && d.two_way && d.portable && !d.mic_muted))
    }
}
