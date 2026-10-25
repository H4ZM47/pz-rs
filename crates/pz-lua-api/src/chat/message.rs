//! `zombie.chat.ChatMessage` and `zombie.chat.ServerChatMessage`.

use super::base::ChatRef;
use super::ChatError;
use crate::bind_misc_classes::Color;
use pz_core::debug::LocalTime;
use pz_kahlua::value::LuaObject;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const CHAT_MESSAGE: &str = "zombie.chat.ChatMessage";
pub const SERVER_CHAT_MESSAGE: &str = "zombie.chat.ServerChatMessage";

/// One message. Shared (`Rc`) between its chat and Lua; a
/// `ServerChatMessage` is a message with `server` set.
pub struct ChatMessage {
    chat: RefCell<ChatRef>,
    datetime: Cell<LocalTime>,
    author: RefCell<Option<String>>,
    text: RefCell<Option<String>>,
    scramble: Cell<bool>,
    custom_tag: RefCell<Option<String>>,
    text_color: RefCell<Rc<Color>>,
    custom_color: Cell<bool>,
    over_head_speech: Cell<bool>,
    show_in_chat: Cell<bool>,
    from_discord: Cell<bool>,
    server_alert: Cell<bool>,
    radio_channel: Cell<i32>,
    local: Cell<bool>,
    should_attract_zombies: Cell<bool>,
    server_author: Cell<bool>,
    server: bool,
}

impl ChatMessage {
    /// `new ChatMessage(chat, text)`.
    pub fn new(chat: &ChatRef, text: Option<&str>) -> Rc<ChatMessage> {
        Self::with_datetime(chat, super::now(), text)
    }

    /// `new ChatMessage(chat, datetime, text)`.
    pub fn with_datetime(chat: &ChatRef, datetime: LocalTime, text: Option<&str>) -> Rc<ChatMessage> {
        Rc::new(Self::build(chat, datetime, text, false))
    }

    /// `new ServerChatMessage(chat, text)`: author "Server", server author.
    pub fn new_server(chat: &ChatRef, text: Option<&str>) -> Rc<ChatMessage> {
        let m = Self::build(chat, super::now(), text, true);
        *m.author.borrow_mut() = Some("Server".into());
        m.server_author.set(true);
        Rc::new(m)
    }

    fn build(chat: &ChatRef, datetime: LocalTime, text: Option<&str>, server: bool) -> ChatMessage {
        ChatMessage {
            chat: RefCell::new(chat.clone()),
            datetime: Cell::new(datetime),
            author: RefCell::new(None),
            text: RefCell::new(text.map(str::to_string)),
            scramble: Cell::new(false),
            custom_tag: RefCell::new(None),
            text_color: RefCell::new(chat.get_color()),
            custom_color: Cell::new(false),
            over_head_speech: Cell::new(true),
            show_in_chat: Cell::new(true),
            from_discord: Cell::new(false),
            server_alert: Cell::new(false),
            radio_channel: Cell::new(-1),
            local: Cell::new(false),
            should_attract_zombies: Cell::new(false),
            server_author: Cell::new(false),
            server,
        }
    }

    /// Whether this is a `ServerChatMessage`.
    pub fn is_server_message(&self) -> bool {
        self.server
    }

    pub fn is_should_attract_zombies(&self) -> bool {
        self.should_attract_zombies.get()
    }
    pub fn set_should_attract_zombies(&self, v: bool) {
        self.should_attract_zombies.set(v);
    }
    pub fn is_local(&self) -> bool {
        self.local.get()
    }
    pub fn set_local(&self, v: bool) {
        self.local.set(v);
    }

    /// `getTextWithReplacedParentheses()`: `<` and `>` as `&lt;`/`&gt;`.
    pub fn get_text_with_replaced_parentheses(&self) -> Option<String> {
        self.text.borrow().as_ref().map(|t| t.replace('<', "&lt;").replace('>', "&gt;"))
    }

    pub fn set_scrambled_text(&self, text: Option<&str>) {
        self.scramble.set(true);
        *self.text.borrow_mut() = text.map(str::to_string);
    }
    pub fn get_radio_channel(&self) -> i32 {
        self.radio_channel.get()
    }
    pub fn set_radio_channel(&self, v: i32) {
        self.radio_channel.set(v);
    }
    pub fn is_server_author(&self) -> bool {
        self.server_author.get()
    }
    pub fn set_server_author(&self, v: bool) {
        self.server_author.set(v);
    }
    pub fn is_from_discord(&self) -> bool {
        self.from_discord.get()
    }
    pub fn make_from_discord(&self) {
        self.from_discord.set(true);
    }
    pub fn is_over_head_speech(&self) -> bool {
        self.over_head_speech.get()
    }
    pub fn set_over_head_speech(&self, v: bool) {
        self.over_head_speech.set(v);
    }
    pub fn is_show_in_chat(&self) -> bool {
        self.show_in_chat.get()
    }
    pub fn set_show_in_chat(&self, v: bool) {
        self.show_in_chat.set(v);
    }
    pub fn get_datetime(&self) -> LocalTime {
        self.datetime.get()
    }
    pub fn set_datetime(&self, t: LocalTime) {
        self.datetime.set(t);
    }

    /// `getDatetimeStr()`: `DateTimeFormatter.ofPattern("h:m")`.
    pub fn get_datetime_str(&self) -> String {
        let t = self.datetime.get();
        let h = match t.hour % 12 {
            0 => 12,
            h => h,
        };
        format!("{h}:{}", t.minute)
    }

    pub fn is_show_author(&self) -> bool {
        self.get_chat().is_show_author()
    }
    pub fn get_author(&self) -> Option<String> {
        self.author.borrow().clone()
    }

    /// `setAuthor(author)`; a `ServerChatMessage` refuses.
    pub fn set_author(&self, author: Option<&str>) -> Result<(), ChatError> {
        if self.server {
            return Err(ChatError::new("java.lang.UnsupportedOperationException", ""));
        }
        *self.author.borrow_mut() = author.map(str::to_string);
        Ok(())
    }

    pub fn get_chat(&self) -> ChatRef {
        self.chat.borrow().clone()
    }
    pub fn get_chat_id(&self) -> i32 {
        self.get_chat().get_id()
    }
    pub fn get_text(&self) -> Option<String> {
        self.text.borrow().clone()
    }
    pub fn set_text(&self, text: Option<&str>) {
        *self.text.borrow_mut() = text.map(str::to_string);
    }

    /// `getTextWithPrefix()`: the chat's `getMessageTextWithPrefix`.
    pub fn get_text_with_prefix(&self) -> String {
        self.get_chat().get_message_text_with_prefix(self)
    }

    pub fn is_scramble(&self) -> bool {
        self.scramble.get()
    }
    pub fn get_custom_tag(&self) -> Option<String> {
        self.custom_tag.borrow().clone()
    }
    pub fn set_custom_tag(&self, t: Option<&str>) {
        *self.custom_tag.borrow_mut() = t.map(str::to_string);
    }
    pub fn get_text_color(&self) -> Rc<Color> {
        self.text_color.borrow().clone()
    }
    pub fn set_text_color(&self, c: Rc<Color>) {
        self.custom_color.set(true);
        *self.text_color.borrow_mut() = c;
    }
    pub fn is_custom_color(&self) -> bool {
        self.custom_color.get()
    }
    pub fn is_server_alert(&self) -> bool {
        self.server_alert.get()
    }
    pub fn set_server_alert(&self, v: bool) {
        self.server_alert.set(v);
    }

    /// `clone()`: a field-for-field copy (same chat, same colour object).
    pub fn clone_message(&self) -> Rc<ChatMessage> {
        Rc::new(ChatMessage {
            chat: RefCell::new(self.get_chat()),
            datetime: Cell::new(self.datetime.get()),
            author: RefCell::new(self.get_author()),
            text: RefCell::new(self.get_text()),
            scramble: Cell::new(self.scramble.get()),
            custom_tag: RefCell::new(self.get_custom_tag()),
            text_color: RefCell::new(self.get_text_color()),
            custom_color: Cell::new(self.custom_color.get()),
            over_head_speech: Cell::new(self.over_head_speech.get()),
            show_in_chat: Cell::new(self.show_in_chat.get()),
            from_discord: Cell::new(self.from_discord.get()),
            server_alert: Cell::new(self.server_alert.get()),
            radio_channel: Cell::new(self.radio_channel.get()),
            local: Cell::new(self.local.get()),
            should_attract_zombies: Cell::new(self.should_attract_zombies.get()),
            server_author: Cell::new(self.server_author.get()),
            server: self.server,
        })
    }
}

/// Java's `"" + string` for a nullable string.
pub(crate) fn or_null(s: &Option<String>) -> &str {
    s.as_deref().unwrap_or("null")
}

impl LuaObject for ChatMessage {
    fn class_name(&self) -> &str {
        if self.server {
            SERVER_CHAT_MESSAGE
        } else {
            CHAT_MESSAGE
        }
    }
    /// `toString()`.
    fn to_lua_string(&self) -> String {
        format!(
            "ChatMessage{{chat={}, author='{}', text='{}'}}",
            self.get_chat().get_title(),
            or_null(&self.get_author()),
            or_null(&self.get_text())
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        if self.server {
            &[CHAT_MESSAGE]
        } else {
            &[]
        }
    }
}
