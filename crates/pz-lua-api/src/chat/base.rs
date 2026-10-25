//! `zombie.chat.ChatBase`, `zombie.chat.ChatTab` and the default chats a
//! single-player game uses (`zombie.chat.defaultChats`: `SayChat`,
//! `ShoutChat`, `GeneralChat`, `RadioChat`, `AdminChat`, their
//! `RangeBasedChat` base, and `ServerChat`'s formatting).
//!
//! The subclasses are one [`ChatBase`] with a [`ChatKind`]; their overrides
//! dispatch on it.

use super::element::ChatElement;
use super::message::{or_null, ChatMessage};
use super::settings::{color_f, color_i, white, ChatSettings};
use super::utility::{chat_stream_enabled, hour_minute, parse_string_for_chat_bubble, parse_string_for_chat_log};
use super::{
    core_debug, debug_log, get_text, players, trigger_event, ChatError, ChatMode, ChatPlayer, ChatResult, ChatType,
};
use crate::bind_misc_classes::Color;
use pz_core::config::float_to_string;
use pz_kahlua::value::LuaObject;
use pz_kahlua::Value;
use pz_text::UIFont;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub const CHAT_BASE: &str = "zombie.chat.ChatBase";
const RANGE_BASED_CHAT: &str = "zombie.chat.defaultChats.RangeBasedChat";
const ID_NOT_SET: i32 = -29048394;

pub type ChatRef = Rc<ChatBase>;

/// Which `defaultChats` class a chat is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatKind {
    General,
    Say,
    Shout,
    Radio,
    Admin,
    Server,
}

impl ChatKind {
    pub fn chat_type(self) -> ChatType {
        match self {
            ChatKind::General => ChatType::General,
            ChatKind::Say => ChatType::Say,
            ChatKind::Shout => ChatType::Shout,
            ChatKind::Radio => ChatType::Radio,
            ChatKind::Admin => ChatType::Admin,
            ChatKind::Server => ChatType::Server,
        }
    }

    pub fn class_name(self) -> &'static str {
        match self {
            ChatKind::General => "zombie.chat.defaultChats.GeneralChat",
            ChatKind::Say => "zombie.chat.defaultChats.SayChat",
            ChatKind::Shout => "zombie.chat.defaultChats.ShoutChat",
            ChatKind::Radio => "zombie.chat.defaultChats.RadioChat",
            ChatKind::Admin => "zombie.chat.defaultChats.AdminChat",
            ChatKind::Server => "zombie.chat.defaultChats.ServerChat",
        }
    }

    /// A `RangeBasedChat`.
    pub fn is_range_based(self) -> bool {
        matches!(self, ChatKind::Say | ChatKind::Shout | ChatKind::Radio)
    }

    /// The class's `getDefaultSettings()` (`GeneralChat` and `AdminChat`
    /// have one too, but their single-player constructors do not use it).
    pub fn default_settings(self) -> ChatSettings {
        let mut s = ChatSettings::new();
        s.bold = true;
        s.show_chat_title = true;
        match self {
            ChatKind::Say => {
                s.font_color = white();
                s.show_author = true;
                s.show_timestamp = true;
                s.unique = true;
                s.allow_colors = true;
                s.allow_chat_icons = true;
                s.allow_images = true;
                s.allow_fonts = false;
                s.allow_bbcode = true;
                s.equalize_line_heights = true;
                s.range = 30.0;
                s.set_zombie_attraction_range(15.0);
            }
            ChatKind::Shout => {
                s.font_color = color_i(255, 51, 51, 255);
                s.show_author = true;
                s.show_timestamp = true;
                s.unique = true;
                s.allow_colors = false;
                s.allow_fonts = false;
                s.allow_bbcode = false;
                s.equalize_line_heights = true;
                s.range = 60.0;
            }
            ChatKind::Radio => {
                // Color.lightGray.
                s.font_color = color_f(0.7, 0.7, 0.7, 1.0);
                s.show_author = false;
                s.show_timestamp = true;
                s.unique = true;
                s.allow_colors = true;
                s.allow_fonts = false;
                s.allow_bbcode = true;
                s.allow_images = false;
                s.allow_chat_icons = true;
            }
            ChatKind::General => {
                s.font_color = color_i(255, 165, 0, 255);
                s.show_author = true;
                s.show_timestamp = true;
                s.unique = true;
                s.allow_colors = true;
                s.allow_fonts = true;
                s.allow_bbcode = true;
            }
            ChatKind::Admin => {
                s.font_color = white();
                s.show_author = true;
                s.show_timestamp = true;
                s.unique = true;
                s.allow_colors = true;
                s.allow_fonts = true;
                s.allow_bbcode = true;
            }
            ChatKind::Server => {
                s.font_color = color_i(0, 128, 255, 255);
                s.show_author = false;
                s.show_timestamp = false;
                s.allow_colors = true;
                s.allow_fonts = false;
                s.allow_bbcode = false;
            }
        }
        s
    }
}

/// `zombie.chat.ChatTab`.
pub struct ChatTab {
    id: i16,
    title_id: String,
    translated_title: String,
    contained_chats: RefCell<HashSet<i32>>,
    enabled: Cell<bool>,
}

impl ChatTab {
    /// `new ChatTab(tabID, titleId)`: the title is translated now.
    pub fn new(id: i16, title_id: &str) -> Self {
        ChatTab {
            id,
            title_id: title_id.to_string(),
            translated_title: get_text(title_id, &[]),
            contained_chats: RefCell::default(),
            enabled: Cell::new(false),
        }
    }

    /// `new ChatTab(tabID, titleId, chatID)`.
    pub fn with_chat(id: i16, title_id: &str, chat_id: i32) -> Self {
        let t = Self::new(id, title_id);
        t.contained_chats.borrow_mut().insert(chat_id);
        t
    }

    /// `RemoveChat(chatID)`.
    pub fn remove_chat(&self, chat_id: i32) -> ChatResult<()> {
        if !self.contained_chats.borrow_mut().remove(&chat_id) {
            return Err(ChatError::new(
                "java.lang.RuntimeException",
                format!("Tab '{}' doesn't contains a chat id: {chat_id}", self.id),
            ));
        }
        Ok(())
    }

    pub fn get_title_id(&self) -> &str {
        &self.title_id
    }
    pub fn get_title(&self) -> &str {
        &self.translated_title
    }
    pub fn get_id(&self) -> i16 {
        self.id
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled.get()
    }
    pub fn set_enabled(&self, v: bool) {
        self.enabled.set(v);
    }
}

thread_local! {
    /// `RangeBasedChat.overHeadChat`.
    static OVERHEAD_CHAT: RefCell<Option<Rc<RefCell<ChatElement>>>> = const { RefCell::new(None) };
    /// `RangeBasedChat.players`: speakers by username.
    static SPEAKERS: RefCell<HashMap<String, Option<Rc<dyn ChatPlayer>>>> = RefCell::default();
    /// `RangeBasedChat.currentPlayerName`.
    static CURRENT_PLAYER_NAME: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// `RangeBasedChat.overHeadChat` (the element range-based chats speak
/// through).
pub fn overhead_chat() -> Option<Rc<RefCell<ChatElement>>> {
    OVERHEAD_CHAT.with(|o| o.borrow().clone())
}

/// `zombie.chat.ChatBase` and its single-player subclasses.
pub struct ChatBase {
    kind: ChatKind,
    id: Cell<i32>,
    title_id: &'static str,
    chat_type: ChatType,
    settings: RefCell<ChatSettings>,
    custom_settings: Cell<bool>,
    chat_tab: RefCell<Option<Rc<ChatTab>>>,
    translated_title: RefCell<Option<String>>,
    messages: RefCell<Vec<Rc<ChatMessage>>>,
    mode: Cell<ChatMode>,
    chat_owner: RefCell<Option<Rc<dyn ChatPlayer>>>,
    /// `RangeBasedChat.customTag`.
    custom_tag: &'static str,
    /// `GeneralChat.discordMessageColor`.
    discord_color: Rc<Color>,
}

impl ChatBase {
    /// The single-player constructor (`new SayChat()`, ...): no id, no tab,
    /// owned by `IsoPlayer.getInstance()`. `SayChat`, `ShoutChat` and
    /// `RadioChat` install their default settings; `GeneralChat` and
    /// `AdminChat` keep plain `ChatSettings`.
    pub fn new_single_player(kind: ChatKind) -> ChatRef {
        let chat_type = kind.chat_type();
        let c = ChatBase {
            kind,
            id: Cell::new(ID_NOT_SET),
            title_id: chat_type.title_id(),
            chat_type,
            settings: RefCell::new(ChatSettings::new()),
            custom_settings: Cell::new(false),
            chat_tab: RefCell::new(None),
            translated_title: RefCell::new(None),
            messages: RefCell::default(),
            mode: Cell::new(ChatMode::SinglePlayer),
            chat_owner: RefCell::new(players().instance()),
            custom_tag: if kind == ChatKind::Radio { "radio" } else { "default" },
            discord_color: color_i(114, 137, 218, 255),
        };
        if matches!(kind, ChatKind::Say | ChatKind::Shout | ChatKind::Radio) {
            c.set_settings(kind.default_settings());
        }
        Rc::new(c)
    }

    pub fn kind(&self) -> ChatKind {
        self.kind
    }

    /// Sets the tab (the multiplayer constructors take one).
    pub fn set_chat_tab(&self, tab: Option<Rc<ChatTab>>) {
        *self.chat_tab.borrow_mut() = tab;
    }

    /// Drops the message list (breaks the chat/message reference cycle
    /// when a chat is replaced).
    pub(crate) fn clear_messages(&self) {
        self.messages.borrow_mut().clear();
    }

    pub fn message_count(&self) -> usize {
        self.messages.borrow().len()
    }

    /// `isEnabled()`.
    pub fn is_enabled(&self) -> bool {
        chat_stream_enabled(self.chat_type)
    }

    /// `getChatOwnerName()`.
    pub fn get_chat_owner_name(&self) -> ChatResult<String> {
        match &*self.chat_owner.borrow() {
            Some(o) => Ok(o.username()),
            None => {
                if self.mode.get() != ChatMode::ServerMultiPlayer {
                    if core_debug() {
                        return Err(ChatError::npe("chat owner is null but name quired"));
                    }
                    debug_log(&format!("chat owner is null but name quired. Chat: {}", self.chat_type.name()));
                }
                Ok(String::new())
            }
        }
    }

    /// `getChatOwner()`.
    pub fn get_chat_owner(&self) -> ChatResult<Option<Rc<dyn ChatPlayer>>> {
        let o = self.chat_owner.borrow().clone();
        if o.is_none() && self.mode.get() != ChatMode::ServerMultiPlayer {
            if core_debug() {
                return Err(ChatError::npe("chat owner is null"));
            }
            debug_log(&format!("chat owner is null. Chat: {}", self.chat_type.name()));
        }
        Ok(o)
    }

    pub fn get_mode(&self) -> ChatMode {
        self.mode.get()
    }
    pub fn get_type(&self) -> ChatType {
        self.chat_type
    }
    pub fn get_id(&self) -> i32 {
        self.id.get()
    }
    pub fn get_title_id(&self) -> &'static str {
        self.title_id
    }
    pub fn get_color(&self) -> Rc<Color> {
        self.settings.borrow().font_color.clone()
    }

    /// `getTabID()`: a `NullPointerException` without a tab (every
    /// single-player chat). `ServerChat` on a client answers the focused
    /// tab instead; there is no client here.
    pub fn get_tab_id(&self) -> ChatResult<i16> {
        self.chat_tab.borrow().as_ref().map(|t| t.get_id()).ok_or_else(|| ChatError::npe("chatTab is null"))
    }

    pub fn get_range(&self) -> f32 {
        self.settings.borrow().range
    }

    /// `isSendingToRadio()`: range-based chats are.
    pub fn is_sending_to_radio(&self) -> bool {
        self.kind.is_range_based()
    }

    pub fn get_zombie_attraction_range(&self) -> f32 {
        self.settings.borrow().get_zombie_attraction_range()
    }

    /// `setSettings(settings)`: marks them custom.
    pub fn set_settings(&self, s: ChatSettings) {
        *self.settings.borrow_mut() = s;
        self.custom_settings.set(true);
    }

    pub fn settings(&self) -> std::cell::Ref<'_, ChatSettings> {
        self.settings.borrow()
    }

    /// `setFontSize(fontSize)`: lower-cased first.
    pub fn set_font_size(&self, s: &str) {
        self.settings.borrow_mut().set_font_size(&s.to_lowercase());
    }
    pub fn set_show_timestamp(&self, v: bool) {
        self.settings.borrow_mut().show_timestamp = v;
    }
    pub fn set_show_title(&self, v: bool) {
        self.settings.borrow_mut().show_chat_title = v;
    }
    pub fn is_custom_settings(&self) -> bool {
        self.custom_settings.get()
    }
    pub fn is_show_author(&self) -> bool {
        self.settings.borrow().show_author
    }
    pub fn is_show_timestamp(&self) -> bool {
        self.settings.borrow().show_timestamp
    }
    pub fn is_show_title(&self) -> bool {
        self.settings.borrow().show_chat_title
    }
    pub fn get_font_size(&self) -> String {
        self.settings.borrow().get_font_size()
    }

    /// `getTitle()`: the translated title id, cached.
    pub fn get_title(&self) -> String {
        self.translated_title.borrow_mut().get_or_insert_with(|| get_text(self.title_id, &[])).clone()
    }

    /// `createMessage(text)`, with the subclass overrides.
    pub fn create_message(self: &Rc<Self>, text: Option<&str>) -> ChatResult<Rc<ChatMessage>> {
        let msg = self.create_base_message(text)?;
        if self.kind.is_range_based() {
            if self.mode.get() == ChatMode::SinglePlayer {
                msg.set_show_in_chat(false);
            }
            msg.set_over_head_speech(true);
            msg.set_should_attract_zombies(true);
        }
        if self.kind == ChatKind::Radio {
            if self.mode.get() == ChatMode::SinglePlayer {
                msg.set_over_head_speech(true);
                msg.set_show_in_chat(false);
            }
            msg.set_should_attract_zombies(true);
        }
        Ok(msg)
    }

    /// `ChatBase.createMessage(text)`: by the chat owner, not the server.
    fn create_base_message(self: &Rc<Self>, text: Option<&str>) -> ChatResult<Rc<ChatMessage>> {
        let author = self.get_chat_owner_name()?;
        let msg = ChatMessage::new(self, text);
        msg.set_author(Some(&author))?;
        msg.set_server_author(false);
        Ok(msg)
    }

    /// `createServerMessage(text)`.
    pub fn create_server_message(self: &Rc<Self>, text: Option<&str>) -> Rc<ChatMessage> {
        let msg = ChatMessage::new_server(self, text);
        msg.set_server_author(true);
        msg
    }

    /// `RangeBasedChat.createBubbleMessage(text)`.
    pub fn create_bubble_message(self: &Rc<Self>, text: Option<&str>) -> ChatResult<Rc<ChatMessage>> {
        let msg = self.create_base_message(text)?;
        msg.set_over_head_speech(true);
        msg.set_show_in_chat(false);
        Ok(msg)
    }

    /// `SayChat.createInfoMessage(text)`.
    pub fn create_info_message(self: &Rc<Self>, text: Option<&str>) -> ChatResult<Rc<ChatMessage>> {
        let msg = self.create_bubble_message(text)?;
        msg.set_local(true);
        msg.set_show_in_chat(false);
        Ok(msg)
    }

    /// `SayChat.createCalloutMessage(text)`.
    pub fn create_callout_message(self: &Rc<Self>, text: Option<&str>) -> ChatResult<Rc<ChatMessage>> {
        let msg = self.create_bubble_message(text)?;
        msg.set_local(false);
        msg.set_should_attract_zombies(true);
        Ok(msg)
    }

    /// `RadioChat.createBroadcastingMessage(text, channel)`.
    pub fn create_broadcasting_message(
        self: &Rc<Self>,
        text: Option<&str>,
        channel: i32,
    ) -> ChatResult<Rc<ChatMessage>> {
        let msg = self.create_bubble_message(text)?;
        msg.set_author(Some(""))?;
        msg.set_should_attract_zombies(false);
        msg.set_radio_channel(channel);
        Ok(msg)
    }

    /// `RadioChat.createStaticSoundMessage(text)`.
    pub fn create_static_sound_message(self: &Rc<Self>, text: Option<&str>) -> ChatResult<Rc<ChatMessage>> {
        let msg = self.create_bubble_message(text)?;
        msg.set_author(Some(""))?;
        msg.set_should_attract_zombies(false);
        Ok(msg)
    }

    /// `showMessage(text, author)`.
    pub fn show_message_text(self: &Rc<Self>, text: Option<&str>, author: Option<&str>) -> ChatResult<()> {
        let msg = ChatMessage::with_datetime(self, super::now(), text);
        msg.set_author(author)?;
        self.show_message(&msg)
    }

    /// `showMessage(msg)`, with the subclass overrides.
    pub fn show_message(self: &Rc<Self>, msg: &Rc<ChatMessage>) -> ChatResult<()> {
        match self.kind {
            ChatKind::Radio => {
                // RadioChat neither keeps the message nor shows a bubble.
                if self.is_enabled() && msg.is_show_in_chat() && self.has_chat_tab() {
                    self.add_message_event(msg)?;
                }
            }
            ChatKind::Server => {
                self.messages.borrow_mut().push(msg.clone());
                if self.is_enabled() {
                    let tab = self.get_tab_id()?;
                    trigger_event("OnAddMessage", vec![message_value(msg), Value::Num(tab as f64)]);
                    if msg.is_server_alert() {
                        trigger_event("OnAlertMessage", vec![message_value(msg), Value::Num(tab as f64)]);
                    }
                }
            }
            _ => {
                self.messages.borrow_mut().push(msg.clone());
                if self.is_enabled() && msg.is_show_in_chat() && self.has_chat_tab() {
                    self.add_message_event(msg)?;
                }
                if self.kind.is_range_based() && msg.is_over_head_speech() {
                    self.show_in_speech_bubble(msg)?;
                }
            }
        }
        Ok(())
    }

    fn add_message_event(&self, msg: &Rc<ChatMessage>) -> ChatResult<()> {
        let tab = self.get_tab_id()?;
        trigger_event("OnAddMessage", vec![message_value(msg), Value::Num(tab as f64)]);
        Ok(())
    }

    /// `RangeBasedChat.Init()`: speaks through the owner's chat element.
    pub fn init_range_based(&self) -> ChatResult<()> {
        let name = self.get_chat_owner_name()?;
        CURRENT_PLAYER_NAME.with(|c| *c.borrow_mut() = Some(name));
        SPEAKERS.with(|s| s.borrow_mut().clear());
        let owner = self.get_chat_owner()?.ok_or_else(|| ChatError::npe("chat owner is null"))?;
        let el = owner.chat_element();
        OVERHEAD_CHAT.with(|o| *o.borrow_mut() = Some(el));
        Ok(())
    }

    /// `RangeBasedChat.selectFont(text)`: `Medium` when a character is
    /// past the end of the `Dialogue` font's table.
    fn select_font(text: &str) -> UIFont {
        let Some(n) = crate::bind_text::font_char_count(UIFont::Dialogue) else {
            return UIFont::Dialogue;
        };
        if text.encode_utf16().any(|c| c as usize > n) {
            UIFont::Medium
        } else {
            UIFont::Dialogue
        }
    }

    /// `RangeBasedChat.getPlayer(name)`: a local player by username.
    fn get_player(name: Option<&str>) -> Option<Rc<dyn ChatPlayer>> {
        let p = players();
        let name = name?;
        (0..p.num_players()).filter_map(|i| p.player(i)).find(|pl| pl.username() == name)
    }

    /// `RangeBasedChat.showInSpeechBubble(msg)`.
    fn show_in_speech_bubble(&self, msg: &ChatMessage) -> ChatResult<()> {
        let color = self.get_color();
        let author = msg.get_author();
        let (r, g, b) = match Self::get_player(author.as_deref()) {
            Some(p) => p.speak_colour(),
            None => (color.r.get(), color.g.get(), color.b.get()),
        };
        let text = parse_string_for_chat_bubble(&msg.get_text().unwrap_or_default());
        let font = Self::select_font(&text);
        let (range, s) = (self.get_range(), self.settings.borrow().clone());
        let add = |el: &Rc<RefCell<ChatElement>>| {
            el.borrow_mut().add_chat_line_full(
                &text,
                r,
                g,
                b,
                font,
                range,
                self.custom_tag,
                s.allow_bbcode,
                s.allow_images,
                s.allow_chat_icons,
                s.allow_colors,
                s.allow_fonts,
                s.equalize_line_heights,
            );
        };
        let current = CURRENT_PLAYER_NAME.with(|c| c.borrow().clone());
        let own = match author.as_deref() {
            None | Some("") => true,
            Some(a) => current.as_deref().is_some_and(|c| a.to_lowercase() == c.to_lowercase()),
        };
        if own {
            let el = overhead_chat().ok_or_else(|| ChatError::npe("overHeadChat is null"))?;
            add(&el);
            return Ok(());
        }
        let author = author.unwrap_or_default();
        let known = SPEAKERS.with(|s| s.borrow().get(&author).cloned());
        let mut speaker = match known {
            Some(p) => p,
            None => {
                let p = Self::get_player(Some(&author));
                SPEAKERS.with(|s| s.borrow_mut().insert(author.clone(), p.clone()));
                p
            }
        };
        if let Some(p) = speaker.clone() {
            if p.is_dead() {
                speaker = Self::get_player(Some(&author));
                SPEAKERS.with(|s| {
                    if let Some(e) = s.borrow_mut().get_mut(&author) {
                        *e = speaker.clone();
                    }
                });
            }
            match speaker {
                Some(p) => add(&p.chat_element()),
                None => return Err(ChatError::npe("player is null")),
            }
        }
        Ok(())
    }

    /// `getMessageTextWithPrefix(msg)`, with the subclass overrides.
    pub fn get_message_text_with_prefix(&self, msg: &ChatMessage) -> String {
        let prefix = self.get_message_prefix(msg);
        match self.kind {
            ChatKind::Say => {
                let text = msg.get_text_with_replaced_parentheses().map(|t| parse_string_for_chat_log(&t));
                format!("{prefix} {}", or_null(&text))
            }
            ChatKind::Server => format!("{prefix} {}", or_null(&msg.get_text())),
            _ => format!("{prefix} {}", or_null(&msg.get_text_with_replaced_parentheses())),
        }
    }

    /// `getMessagePrefix(msg)`, with the subclass overrides.
    pub fn get_message_prefix(&self, msg: &ChatMessage) -> String {
        let mut s = String::new();
        let author = msg.get_author();
        match self.kind {
            ChatKind::Server => {
                s.push_str(&self.get_chat_settings_tags());
                let mut added = false;
                if self.is_show_title() {
                    s.push_str(&format!("[{}]", self.get_title()));
                    added = true;
                }
                if !msg.is_server_author() && self.is_show_author() {
                    s.push_str(&format!("[{}]", or_null(&author)));
                    added = true;
                }
                if added {
                    s.push_str(": ");
                }
                return s;
            }
            ChatKind::General => {
                let c = if msg.is_from_discord() { self.discord_color.clone() } else { self.get_color() };
                s.push_str(&Self::color_tag(&c));
                s.push(' ');
                s.push_str(&self.get_font_size_tag());
                s.push(' ');
            }
            _ => s.push_str(&self.get_chat_settings_tags()),
        }
        if self.is_show_timestamp() {
            s.push_str(&format!("[{}]", hour_minute()));
        }
        if self.is_show_title() {
            s.push_str(&format!("[{}]", self.get_title()));
        }
        if self.kind == ChatKind::Radio {
            match author.as_deref() {
                Some(a) if self.is_show_author() && !a.is_empty() => s.push_str(&format!(" {a} ")),
                _ => s.push_str(" Radio "),
            }
            s.push_str(&format!(" ({}): ", radio_channel_str(msg.get_radio_channel())));
            return s;
        }
        if self.is_show_author() {
            s.push_str(&format!("[{}]", or_null(&author)));
        }
        s.push_str(": ");
        s
    }

    /// `getColorTag(color)`.
    fn color_tag(c: &Color) -> String {
        format!("<RGB:{},{},{}>", float_to_string(c.r.get()), float_to_string(c.g.get()), float_to_string(c.b.get()))
    }

    /// `getFontSizeTag()`.
    fn get_font_size_tag(&self) -> String {
        format!("<SIZE:{}>", self.get_font_size())
    }

    /// `getChatSettingsTags()`.
    fn get_chat_settings_tags(&self) -> String {
        format!("{} {} ", Self::color_tag(&self.get_color()), self.get_font_size_tag())
    }

    /// `hasChatTab()`.
    pub fn has_chat_tab(&self) -> bool {
        self.chat_tab.borrow().is_some()
    }
}

/// `RadioChat.getRadioChannelStr`: kHz as "<MHz>.<trimmed kHz> MHz".
fn radio_channel_str(channel: i32) -> String {
    let mut hz = channel % 1000;
    while hz % 10 == 0 && hz != 0 {
        hz /= 10;
    }
    format!("{}.{hz} MHz", channel / 1000)
}

pub(crate) fn message_value(msg: &Rc<ChatMessage>) -> Value {
    Value::Object(msg.clone())
}

impl LuaObject for ChatBase {
    fn class_name(&self) -> &str {
        self.kind.class_name()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        if self.kind.is_range_based() {
            &[RANGE_BASED_CHAT, CHAT_BASE]
        } else {
            &[CHAT_BASE]
        }
    }
}
