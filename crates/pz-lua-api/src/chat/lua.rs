//! Lua exposure of `zombie.chat.ChatMessage`, `zombie.chat.ChatBase` and
//! `zombie.chat.ServerChatMessage`, and the chat globals of
//! `LuaManager.GlobalObject` (`processSayMessage`, `proceedPM`,
//! `checkPlayerCanUseChat`, ...).
//!
//! The network-only members (`pack`, `unpackMessage`, `packMessage`,
//! member lists, `send*` packets) and the `LocalDateTime` accessors
//! (`getDatetime`, `setDatetime`) are not exposed. `ChatType` and
//! `ChatMode` values reach Lua as enum constants of classes that are not
//! exposed, as in the original.

use super::base::{ChatBase, CHAT_BASE};
use super::message::{ChatMessage, CHAT_MESSAGE, SERVER_CHAT_MESSAGE};
use super::{fire_pending_events, get_text, java_trim, with_chat_manager, ChatError, ChatMode, ChatResult, ChatType};
use crate::bind_misc_classes::Color;
use crate::expose::{bool_arg, expose_class, global, int_arg, str_arg, JavaEnum};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const CHAT_TYPE: &str = "zombie.network.chat.ChatType";
const CHAT_MODE: &str = "zombie.chat.ChatMode";

thread_local! {
    /// The enum constants handed to Lua, one object per constant.
    static ENUMS: RefCell<HashMap<(&'static str, &'static str), Value>> = RefCell::default();
}

fn enum_constant(class: &'static str, name: &'static str, ordinal: i32) -> Value {
    ENUMS.with(|e| {
        e.borrow_mut()
            .entry((class, name))
            .or_insert_with(|| Value::Object(Rc::new(JavaEnum { class, name, ordinal })))
            .clone()
    })
}

fn chat_type_value(t: ChatType) -> Value {
    enum_constant(CHAT_TYPE, t.name(), ChatType::ALL.iter().position(|x| *x == t).unwrap_or(0) as i32)
}

fn chat_mode_value(m: ChatMode) -> Value {
    enum_constant(CHAT_MODE, m.name(), m as i32)
}

fn downcast<T: Any>(v: Value) -> Option<Rc<T>> {
    match v {
        Value::Object(o) => {
            let any: Rc<dyn Any> = o;
            any.downcast::<T>().ok()
        }
        _ => None,
    }
}

fn expected(what: &str, v: &Value) -> LuaError {
    LuaError::runtime(format!("Expected {what} but got {}", pz_kahlua::stdlib::java_class_name(v)))
}

fn this_msg(c: &NativeCall<'_>) -> LuaResult<Rc<ChatMessage>> {
    let v = c.arg(0);
    downcast::<ChatMessage>(v.clone()).ok_or_else(|| expected(CHAT_MESSAGE, &v))
}

fn chat_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<ChatBase>> {
    let v = c.arg(i);
    downcast::<ChatBase>(v.clone()).ok_or_else(|| expected(CHAT_BASE, &v))
}

fn opt_str(c: &NativeCall<'_>, i: usize) -> Option<String> {
    str_arg(c, i).map(|s| s.to_string())
}

fn push_str(c: &mut NativeCall<'_>, s: Option<String>) -> usize {
    c.push(s.map_or(Value::Nil, Value::from))
}

fn msg_value(m: Rc<ChatMessage>) -> Value {
    Value::Object(m)
}

/// Runs a chat call, then fires the Lua events it raised.
fn run<R>(c: &mut NativeCall<'_>, f: impl FnOnce() -> ChatResult<R>) -> LuaResult<R> {
    let r = f();
    fire_pending_events(c.thread);
    r.map_err(LuaError::from)
}

macro_rules! msg_bool {
    ($b:ident, $get:literal, $getf:ident, $set:literal, $setf:ident) => {
        $b.method($get, |c| {
            let v = this_msg(c)?.$getf();
            Ok(c.push(v))
        });
        $b.method($set, |c| {
            this_msg(c)?.$setf(bool_arg(c, 1));
            Ok(0)
        });
    };
}

fn register_chat_message(thread: &mut Thread) {
    let b = expose_class(thread, CHAT_MESSAGE, None);
    b.ctor(|c| {
        let chat = chat_arg(c, 0)?;
        let m = ChatMessage::new(&chat, str_arg(c, 1).as_deref());
        Ok(c.push(msg_value(m)))
    });
    msg_bool!(
        b,
        "isShouldAttractZombies",
        is_should_attract_zombies,
        "setShouldAttractZombies",
        set_should_attract_zombies
    );
    msg_bool!(b, "isLocal", is_local, "setLocal", set_local);
    msg_bool!(b, "isServerAuthor", is_server_author, "setServerAuthor", set_server_author);
    msg_bool!(b, "isOverHeadSpeech", is_over_head_speech, "setOverHeadSpeech", set_over_head_speech);
    msg_bool!(b, "isShowInChat", is_show_in_chat, "setShowInChat", set_show_in_chat);
    msg_bool!(b, "isServerAlert", is_server_alert, "setServerAlert", set_server_alert);
    b.method("getTextWithReplacedParentheses", |c| {
        let s = this_msg(c)?.get_text_with_replaced_parentheses();
        Ok(push_str(c, s))
    });
    b.method("setScrambledText", |c| {
        this_msg(c)?.set_scrambled_text(str_arg(c, 1).as_deref());
        Ok(0)
    });
    b.method("getRadioChannel", |c| {
        let v = this_msg(c)?.get_radio_channel();
        Ok(c.push(v as f64))
    });
    b.method("setRadioChannel", |c| {
        this_msg(c)?.set_radio_channel(int_arg(c, 1));
        Ok(0)
    });
    b.method("isFromDiscord", |c| {
        let v = this_msg(c)?.is_from_discord();
        Ok(c.push(v))
    });
    b.method("makeFromDiscord", |c| {
        this_msg(c)?.make_from_discord();
        Ok(0)
    });
    b.method("getDatetimeStr", |c| {
        let s = this_msg(c)?.get_datetime_str();
        Ok(c.push(s))
    });
    b.method("isShowAuthor", |c| {
        let v = this_msg(c)?.is_show_author();
        Ok(c.push(v))
    });
    b.method("getAuthor", |c| {
        let s = this_msg(c)?.get_author();
        Ok(push_str(c, s))
    });
    b.method("setAuthor", |c| {
        let m = this_msg(c)?;
        m.set_author(str_arg(c, 1).as_deref())?;
        Ok(0)
    });
    b.method("getChat", |c| {
        let chat = this_msg(c)?.get_chat();
        Ok(c.push(Value::Object(chat)))
    });
    b.method("getChatID", |c| {
        let v = this_msg(c)?.get_chat_id();
        Ok(c.push(v as f64))
    });
    b.method("getText", |c| {
        let s = this_msg(c)?.get_text();
        Ok(push_str(c, s))
    });
    b.method("setText", |c| {
        this_msg(c)?.set_text(str_arg(c, 1).as_deref());
        Ok(0)
    });
    b.method("getTextWithPrefix", |c| {
        let s = this_msg(c)?.get_text_with_prefix();
        Ok(c.push(s))
    });
    b.method("isScramble", |c| {
        let v = this_msg(c)?.is_scramble();
        Ok(c.push(v))
    });
    b.method("getCustomTag", |c| {
        let s = this_msg(c)?.get_custom_tag();
        Ok(push_str(c, s))
    });
    b.method("setCustomTag", |c| {
        this_msg(c)?.set_custom_tag(str_arg(c, 1).as_deref());
        Ok(0)
    });
    b.method("getTextColor", |c| {
        let col = this_msg(c)?.get_text_color();
        Ok(c.push(Value::Object(col)))
    });
    b.method("setTextColor", |c| {
        let m = this_msg(c)?;
        let v = c.arg(1);
        let col = downcast::<Color>(v.clone()).ok_or_else(|| expected("zombie.core.Color", &v))?;
        m.set_text_color(col);
        Ok(0)
    });
    b.method("isCustomColor", |c| {
        let v = this_msg(c)?.is_custom_color();
        Ok(c.push(v))
    });
    b.method("clone", |c| {
        let m = this_msg(c)?.clone_message();
        Ok(c.push(msg_value(m)))
    });
    b.method("toString", |c| {
        let s = this_msg(c)?.to_lua_string();
        Ok(c.push(s))
    });

    let s = expose_class(thread, SERVER_CHAT_MESSAGE, Some(CHAT_MESSAGE));
    s.ctor(|c| {
        let chat = chat_arg(c, 0)?;
        let m = ChatMessage::new_server(&chat, str_arg(c, 1).as_deref());
        Ok(c.push(msg_value(m)))
    });
    s.method("setAuthor", |c| {
        let m = this_msg(c)?;
        m.set_author(str_arg(c, 1).as_deref())?;
        Ok(0)
    });
}

fn this_chat(c: &NativeCall<'_>) -> LuaResult<Rc<ChatBase>> {
    chat_arg(c, 0)
}

fn register_chat_base(thread: &mut Thread) {
    let b = expose_class(thread, CHAT_BASE, None);
    b.method("isEnabled", |c| {
        let v = this_chat(c)?.is_enabled();
        Ok(c.push(v))
    });
    b.method("getMode", |c| {
        let v = chat_mode_value(this_chat(c)?.get_mode());
        Ok(c.push(v))
    });
    b.method("getType", |c| {
        let v = chat_type_value(this_chat(c)?.get_type());
        Ok(c.push(v))
    });
    b.method("getID", |c| {
        let v = this_chat(c)?.get_id();
        Ok(c.push(v as f64))
    });
    b.method("getTitleID", |c| {
        let v = this_chat(c)?.get_title_id();
        Ok(c.push(v))
    });
    b.method("getColor", |c| {
        let v = this_chat(c)?.get_color();
        Ok(c.push(Value::Object(v)))
    });
    b.method("getTabID", |c| {
        let v = this_chat(c)?.get_tab_id()?;
        Ok(c.push(v as f64))
    });
    b.method("getRange", |c| {
        let v = this_chat(c)?.get_range();
        Ok(c.push(v as f64))
    });
    b.method("isSendingToRadio", |c| {
        let v = this_chat(c)?.is_sending_to_radio();
        Ok(c.push(v))
    });
    b.method("getZombieAttractionRange", |c| {
        let v = this_chat(c)?.get_zombie_attraction_range();
        Ok(c.push(v as f64))
    });
    b.method("setFontSize", |c| {
        let chat = this_chat(c)?;
        let s = str_arg(c, 1).ok_or_else(|| LuaError::from(ChatError::npe("fontSize is null")))?;
        chat.set_font_size(&s);
        Ok(0)
    });
    b.method("setShowTimestamp", |c| {
        this_chat(c)?.set_show_timestamp(bool_arg(c, 1));
        Ok(0)
    });
    b.method("setShowTitle", |c| {
        this_chat(c)?.set_show_title(bool_arg(c, 1));
        Ok(0)
    });
    // close(): leaves every member; single-player chats have none.
    b.method("close", |c| {
        this_chat(c)?;
        Ok(0)
    });
    b.method("createMessage", |c| {
        let chat = this_chat(c)?;
        let text = opt_str(c, 1);
        let m = run(c, || chat.create_message(text.as_deref()))?;
        Ok(c.push(msg_value(m)))
    });
    b.method("createServerMessage", |c| {
        let m = this_chat(c)?.create_server_message(str_arg(c, 1).as_deref());
        Ok(c.push(msg_value(m)))
    });
    b.method("showMessage", |c| {
        let chat = this_chat(c)?;
        if c.nargs() >= 3 {
            let (text, author) = (opt_str(c, 1), opt_str(c, 2));
            run(c, || chat.show_message_text(text.as_deref(), author.as_deref()))?;
        } else {
            let v = c.arg(1);
            let m = downcast::<ChatMessage>(v.clone()).ok_or_else(|| expected(CHAT_MESSAGE, &v))?;
            run(c, || chat.show_message(&m))?;
        }
        Ok(0)
    });
    b.method("getMessageTextWithPrefix", |c| {
        let chat = this_chat(c)?;
        let v = c.arg(1);
        let m = downcast::<ChatMessage>(v.clone()).ok_or_else(|| expected(CHAT_MESSAGE, &v))?;
        Ok(c.push(chat.get_message_text_with_prefix(&m)))
    });
    b.method("getMessagePrefix", |c| {
        let chat = this_chat(c)?;
        let v = c.arg(1);
        let m = downcast::<ChatMessage>(v.clone()).ok_or_else(|| expected(CHAT_MESSAGE, &v))?;
        Ok(c.push(chat.get_message_prefix(&m)))
    });
}

/// `proceedPM`'s `(\"[^\"]*\s+[^\"]*\"|[^\"]\S*)\s(.+)`, matched whole, with
/// Java's `\s` and `.`.
fn pm_regex() -> regex::Regex {
    const WS: &str = r"[ \t\n\x0B\x0C\r]";
    const NWS: &str = r"[^ \t\n\x0B\x0C\r]";
    const DOT: &str = r"[^\n\r\x{85}\x{2028}\x{2029}]";
    regex::Regex::new(&format!(r#"\A(?:("[^"]*{WS}+[^"]*"|[^"]{NWS}*){WS}({DOT}+))\z"#)).unwrap()
}

/// `ChatManager.getInstance().sendMessageToChat(type, message.trim())` for
/// a non-empty message.
fn send_global(c: &mut NativeCall<'_>, t: ChatType) -> LuaResult<usize> {
    let Some(m) = str_arg(c, 0).filter(|m| !m.is_empty()) else { return Ok(0) };
    run(c, || with_chat_manager(|cm| cm.send_message_to_chat(t, java_trim(&m))))?;
    Ok(0)
}

/// `checkPlayerCanUseChat`'s command table.
fn chat_for_command(cmd: &str) -> ChatType {
    match cmd {
        "/all" => ChatType::General,
        "/a" | "/admin" => ChatType::Admin,
        "/s" | "/say" => ChatType::Say,
        "/y" | "/yell" => ChatType::Shout,
        "/f" | "/faction" => ChatType::Faction,
        "/sh" | "/safehouse" => ChatType::Safehouse,
        "/w" | "/whisper" => ChatType::Whisper,
        "/radio" | "/r" => ChatType::Radio,
        _ => {
            pz_core::debug::DebugType::Lua.warn(None, "Chat command not found");
            ChatType::NotDefined
        }
    }
}

fn register_globals(thread: &mut Thread) {
    global(thread, "processSayMessage", |c| send_global(c, ChatType::Say));
    global(thread, "processGeneralMessage", |c| send_global(c, ChatType::General));
    global(thread, "processShoutMessage", |c| send_global(c, ChatType::Shout));
    global(thread, "proceedFactionMessage", |c| send_global(c, ChatType::Faction));
    global(thread, "processSafehouseMessage", |c| send_global(c, ChatType::Safehouse));
    global(thread, "processAdminChatMessage", |c| send_global(c, ChatType::Admin));
    global(thread, "proceedPM", |c| {
        let cmd = str_arg(c, 0).ok_or_else(|| LuaError::from(ChatError::npe("command is null")))?;
        let cmd = java_trim(&cmd).to_string();
        let caps = pm_regex().captures(&cmd).map(|m| (m[1].replace('"', ""), m[2].to_string()));
        let Some((user, msg)) = caps else {
            let text = get_text("IGUI_Commands_Whisper", &[]);
            run(c, || with_chat_manager(|cm| cm.add_message("Error", &text)))?;
            return Ok(c.push(""));
        };
        run(c, || with_chat_manager(|cm| cm.send_whisper_message(Some(&user), &msg)))?;
        Ok(c.push(user))
    });
    global(thread, "showWrongChatTabMessage", |c| {
        let (actual, right, cmd) = (int_arg(c, 0) as i16, int_arg(c, 1) as i16, opt_str(c, 2));
        run(c, || {
            with_chat_manager(|cm| {
                let (a, r) = (cm.get_tab_name(actual), cm.get_tab_name(right));
                let msg = get_text("UI_chat_wrong_tab", &[&a, &r, cmd.as_deref().unwrap_or("null")]);
                cm.show_server_chat_message(&msg)
            })
        })?;
        Ok(0)
    });
    global(thread, "focusOnTab", |c| {
        let id = int_arg(c, 0) as i16;
        run(c, || with_chat_manager(|cm| cm.focus_on_tab(id)))?;
        Ok(0)
    });
    global(thread, "updateChatSettings", |c| {
        let (size, ts, title) = (opt_str(c, 0), bool_arg(c, 1), bool_arg(c, 2));
        with_chat_manager(|cm| cm.update_chat_settings(size.as_deref(), ts, title));
        Ok(0)
    });
    global(thread, "checkPlayerCanUseChat", |c| {
        let cmd = str_arg(c, 0).ok_or_else(|| LuaError::from(ChatError::npe("chatCommand is null")))?;
        let t = chat_for_command(java_trim(&cmd));
        let v = run(c, || with_chat_manager(|cm| cm.is_player_can_use_chat(t)))?;
        Ok(c.push(v))
    });
}

/// Exposes the chat classes and globals.
pub fn register(thread: &mut Thread) {
    register_chat_base(thread);
    register_chat_message(thread);
    register_globals(thread);
}
