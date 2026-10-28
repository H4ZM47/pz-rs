//! The characters' side of the chat: [`ChatElementOwner`] and
//! [`ChatPlayer`] for the bound characters (`RefCell<LuaGameCharacter>`,
//! so a `Weak` of the character itself is its chat element's owner),
//! [`ChatPlayers`] over [`crate::bind_players`], each character's
//! `chatElement` ([`chat_element`]), `IsoGameCharacter.ProcessSay`
//! ([`process_say`]), the chat calls the simulation queued
//! ([`flush_chat`]), `renderlast`'s chat lines ([`render_last`]) and the
//! Lua members of `IsoGameCharacter` / `IsoPlayer` that speak or read
//! speech ([`register`]).
//!
//! A character is read through `try_borrow`: while the character is
//! mutably borrowed (inside its own update) the chat sees it at the origin,
//! not on a square, and not deaf, so callers run the chat with no borrow
//! held ([`flush_chat`] does).
//!
//! Single player: `getUsername` is `username` (disguises and the
//! first/last-name decorations are multiplayer), `getUserNameHeight` is 0,
//! `isAccessLevel` is false (roles are not ported), the faction and
//! safehouse checks are false. The equipped radio (`getEquipedRadio`,
//! `radioEquipedCheck`) is TODO(radio); `TutorialManager.profanityFilter`
//! is false outside the tutorial, which is not ported, so `ProcessSay`
//! never filters.

use super::element::ChatElement;
use super::{
    fire_pending_events, with_chat_manager, ChatElementOwner, ChatError, ChatPlayer, ChatPlayers, ChatResult,
    ChatSquare, ChatType,
};
use crate::bind_character_sim::{chr_of, LuaGameCharacter, LuaGameCharacterRef};
use crate::bind_core::LuaColorInfo;
use crate::bind_misc_classes::Color;
use crate::bind_players;
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, JavaEnum};
use pz_characters::character::chat::{ChatRequest, SayArgs};
use pz_characters::CharacterKind;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_text::UIFont;
use std::any::Any;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

const ISO_GAME_CHARACTER: &str = crate::bind_character_sim::ISO_GAME_CHARACTER_CLASS;
const ISO_PLAYER: &str = crate::bind_character_sim::ISO_PLAYER_CLASS;
const CHAT_ELEMENT_CLASS: &str = "zombie.chat.ChatElement";
const RADIO_CLASS: &str = "zombie.radio.ZomboidRadio";

thread_local! {
    /// `ZomboidRadio.staticSounds` (from the jar's static initializer).
    static STATIC_SOUNDS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

// ---- ChatElementOwner / ChatPlayer ------------------------------------------------

fn read<R>(cell: &RefCell<LuaGameCharacter>, default: R, f: impl FnOnce(&LuaGameCharacter) -> R) -> R {
    cell.try_borrow().map_or(default, |c| f(&c))
}

fn square_of(c: &LuaGameCharacter) -> Option<ChatSquare> {
    let e = &c.environment;
    e.has_current_square.then_some(ChatSquare { building: e.square_building, room: e.square_room })
}

/// `IsoGameCharacter implements ChatElementOwner`.
impl ChatElementOwner for RefCell<LuaGameCharacter> {
    fn get_x(&self) -> f32 {
        read(self, 0.0, |c| c.vars.x)
    }
    fn get_y(&self) -> f32 {
        read(self, 0.0, |c| c.vars.y)
    }
    fn get_z(&self) -> f32 {
        read(self, 0.0, |c| c.vars.z)
    }
    fn get_square(&self) -> Option<ChatSquare> {
        read(self, None, square_of)
    }
    /// `this == IsoPlayer.players[player_num]` (by identity).
    fn is_player(&self, player_num: usize) -> bool {
        bind_players::player(player_num).is_some_and(|p| std::ptr::eq(Rc::as_ptr(&p), self))
    }
}

/// `IsoPlayer` as the chat reads it.
impl ChatPlayer for RefCell<LuaGameCharacter> {
    /// `playerIndex`.
    fn player_num(&self) -> usize {
        read(self, 0, |c| c.player.player_index.max(0) as usize)
    }
    /// `getUsername()` (single player: `username`).
    fn username(&self) -> String {
        read(self, String::new(), |c| c.username().unwrap_or_default().to_string())
    }
    fn chat_element(&self) -> Rc<RefCell<ChatElement>> {
        if let Some(e) = read(self, None, element_slot) {
            return e;
        }
        // The players' slots hold the character's `Rc`: attach through it.
        for i in 0..bind_players::MAX_PLAYERS {
            if let Some(p) = bind_players::player(i).filter(|p| std::ptr::eq(Rc::as_ptr(p), self)) {
                return chat_element(&p);
            }
        }
        if let Some(p) = bind_players::player_instance().filter(|p| std::ptr::eq(Rc::as_ptr(p), self)) {
            return chat_element(&p);
        }
        // Not reachable from a bound character: an element without owner.
        let owner: Weak<dyn ChatElementOwner> = Weak::<RefCell<LuaGameCharacter>>::new();
        Rc::new(RefCell::new(ChatElement::new(owner, 1, Some("character"))))
    }
    /// `getSpeakColour()`.
    fn speak_colour(&self) -> (f32, f32, f32) {
        read(self, (1.0, 1.0, 1.0), |c| (c.speak_colour[0], c.speak_colour[1], c.speak_colour[2]))
    }
    fn is_dead(&self) -> bool {
        read(self, false, |c| c.is_dead())
    }
    fn get_x(&self) -> f32 {
        ChatElementOwner::get_x(self)
    }
    fn get_y(&self) -> f32 {
        ChatElementOwner::get_y(self)
    }
    fn get_z(&self) -> f32 {
        ChatElementOwner::get_z(self)
    }
    fn get_square(&self) -> Option<ChatSquare> {
        ChatElementOwner::get_square(self)
    }
    /// `hasTrait(CharacterTrait.DEAF)`.
    fn is_deaf(&self) -> bool {
        let sm = crate::bind_registry::shared_script_manager();
        read(self, false, |c| c.has_trait(&sm, "DEAF").unwrap_or(false))
    }
}

/// `IsoPlayer.players` / `numPlayers` / `getInstance()` from
/// [`crate::bind_players`].
pub struct CharacterChatPlayers;

impl ChatPlayers for CharacterChatPlayers {
    fn num_players(&self) -> usize {
        bind_players::num_players().max(0) as usize
    }
    fn player(&self, i: usize) -> Option<Rc<dyn ChatPlayer>> {
        bind_players::player(i).map(|p| p as Rc<dyn ChatPlayer>)
    }
    fn instance(&self) -> Option<Rc<dyn ChatPlayer>> {
        bind_players::player_instance().map(|p| p as Rc<dyn ChatPlayer>)
    }
}

/// Makes [`CharacterChatPlayers`] the players the chat reads.
pub fn install_character_chat_players() {
    super::install_players(Rc::new(CharacterChatPlayers));
}

// ---- the chat element -------------------------------------------------------------

fn element_slot(c: &LuaGameCharacter) -> Option<Rc<RefCell<ChatElement>>> {
    c.chat.element.clone().and_then(|e| e.downcast::<RefCell<ChatElement>>().ok())
}

/// The constructor's `chatElement = new ChatElement(this, 1, "character")`:
/// attaches `chr`'s chat element (owned by a `Weak` of `chr`) if it has
/// none yet. The engine calls it when it creates a character;
/// [`crate::bind_character_sim::character_value`] and
/// [`crate::bind_players::set_player`] also do.
/// Skipped while `chr` is borrowed.
pub fn attach_chat_element(chr: &LuaGameCharacterRef) {
    if chr.try_borrow_mut().is_ok_and(|c| c.chat.element.is_none()) {
        let _ = chat_element(chr);
    }
}

/// `getChatElement()` (attached first if needed).
pub fn chat_element(chr: &LuaGameCharacterRef) -> Rc<RefCell<ChatElement>> {
    if let Some(e) = element_slot(&chr.borrow()) {
        return e;
    }
    let weak: Weak<RefCell<LuaGameCharacter>> = Rc::downgrade(chr);
    let owner: Weak<dyn ChatElementOwner> = weak;
    let e = Rc::new(RefCell::new(ChatElement::new(owner, 1, Some("character"))));
    let any: Rc<dyn Any> = e.clone();
    chr.borrow_mut().chat.element = Some(any);
    e
}

// ---- speech -----------------------------------------------------------------------

/// `ZomboidRadio.staticSounds` from the jar's `<clinit>`.
pub fn static_sounds_from_jar(jar: &pz_classfile::Jar) -> Vec<String> {
    let Ok(cf) = pz_classfile::load_class(jar, RADIO_CLASS) else { return Vec::new() };
    let Ok(inits) = pz_classfile::static_init::static_inits(&cf) else { return Vec::new() };
    inits
        .into_iter()
        .find(|i| i.class == cf.this_class && i.field == "staticSounds")
        .and_then(|i| i.value.as_str_array().map(|a| a.into_iter().map(str::to_string).collect()))
        .unwrap_or_default()
}

/// Sets `ZomboidRadio.staticSounds`.
pub fn set_static_sounds(sounds: Vec<String>) {
    STATIC_SOUNDS.with(|s| *s.borrow_mut() = sounds);
}

/// `ZomboidRadio.isStaticSound(text)`.
pub fn is_static_sound(text: &str) -> bool {
    STATIC_SOUNDS.with(|s| s.borrow().iter().any(|x| x == text))
}

fn class_cast(chr: &LuaGameCharacter) -> ChatError {
    let class = match chr.kind {
        CharacterKind::Survivor => crate::bind_character_sim::ISO_SURVIVOR_CLASS,
        CharacterKind::Zombie => crate::bind_character_sim::ISO_ZOMBIE_CLASS,
        _ => ISO_PLAYER,
    };
    ChatError::new("java.lang.ClassCastException", format!("class {class} cannot be cast to class {ISO_PLAYER}"))
}

/// `IsoGameCharacter.ProcessSay(...)`: nothing unless `allowConversation`;
/// `default` is the player's info message in the say chat, `whisper` only
/// remembers the line, `shout` goes to the shout chat, `radio` is a line
/// on the chat element and a radio message (or the static sound). The
/// `default` and `shout` paths cast to `IsoPlayer` (a survivor or zombie
/// gets the original's `ClassCastException`).
pub fn process_say(chr: &LuaGameCharacterRef, a: &SayArgs) -> ChatResult<()> {
    let (allow, is_player, username) = {
        let c = chr.borrow();
        (c.chat.allow_conversation, matches!(c.kind, CharacterKind::Player | CharacterKind::Animal), c.username().map(str::to_string))
    };
    if !allow {
        return Ok(());
    }
    let username = || -> ChatResult<String> {
        if !is_player {
            return Err(class_cast(&chr.borrow()));
        }
        Ok(username.clone().unwrap_or_default())
    };
    let set_last = |line: &str| chr.borrow_mut().chat.last_spoken_line = Some(line.to_string());
    match a.tag.as_str() {
        "default" => {
            let author = username()?;
            with_chat_manager(|m| m.show_info_message_from(&author, &a.line))?;
            set_last(&a.line);
        }
        "whisper" => set_last(&a.line),
        "shout" => {
            let author = username()?;
            with_chat_manager(|m| m.send_message_to_chat_as(&author, ChatType::Shout, &a.line))?;
            set_last(&a.line);
        }
        "radio" => {
            let el = chat_element(chr);
            el.borrow_mut().add_chat_line_full(
                &a.line,
                a.r,
                a.g,
                a.b,
                UIFont::Medium,
                a.base_range,
                &a.tag,
                true,
                true,
                true,
                false,
                false,
                true,
            );
            if is_static_sound(&a.line) {
                with_chat_manager(|m| m.show_static_radio_sound(&a.line))?;
            } else {
                with_chat_manager(|m| m.show_radio_message_text(&a.line, a.channel))?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Runs the chat calls `chr`'s simulation queued, in order: the engine
/// calls it after each character update (and the Lua members below after
/// each call). An exception stops the rest, as it stops the original's
/// `updateTextObjects`.
pub fn flush_chat(chr: &LuaGameCharacterRef) -> ChatResult<()> {
    let requests = chr.borrow_mut().take_chat_requests();
    for r in requests {
        match r {
            ChatRequest::Say(a) => process_say(chr, &a)?,
            ChatRequest::AddNoLogText(t) => ChatElement::add_no_log_text(&t),
            ChatRequest::SetMaxChatLines(n) => chat_element(chr).borrow_mut().set_max_chat_lines(n),
            ChatRequest::UpdateElement => {
                let el = chat_element(chr);
                el.borrow_mut().update();
                let speaking = el.borrow().is_speaking();
                chr.borrow_mut().after_chat_element_update(speaking);
            }
        }
    }
    Ok(())
}

/// The chat part of `renderlast()`: `chatElement.renderBatched(
/// IsoPlayer.getPlayerIndex(), x, y, ignoreRadioLines)` at the name
/// coordinates (`x`, `y`: `getNameCoords`, above the name tag, progress
/// bar and halo note, which are the renderer's), when the character has
/// text objects or chat to show and is not invisible to the camera
/// (`camera_is_self`: `IsoCamera.frameState.camCharacter == this`).
/// `ignoreRadioLines` needs the equipped radio, TODO(radio): false.
pub fn render_last(chr: &LuaGameCharacterRef, x: i32, y: i32, invisible: bool, camera_is_self: bool) {
    let el = chat_element(chr);
    let has_text = {
        let c = chr.borrow();
        c.chat.has_init_text_objects && c.chat.iso_player
    };
    if !has_text && !el.borrow().get_has_chat_to_display() {
        return;
    }
    // `isInvisible() && camCharacter != this && !canSeePlayerStats`
    // (the role capability is a client's).
    if invisible && !camera_is_self {
        return;
    }
    let player = bind_players::player_index().max(0) as usize;
    ChatElement::render_batched(&el, player, x, y, false);
}

// ---- Lua --------------------------------------------------------------------------

/// `getChatElement()`'s value: `ChatElement` is not exposed to Lua.
pub struct ChatElementObject(pub Rc<RefCell<ChatElement>>);

impl LuaObject for ChatElementObject {
    fn class_name(&self) -> &str {
        CHAT_ELEMENT_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static ELEMENT_VALUES: RefCell<Vec<(Weak<RefCell<ChatElement>>, Value)>> = const { RefCell::new(Vec::new()) };
}

/// One Lua object per chat element.
fn element_value(e: &Rc<RefCell<ChatElement>>) -> Value {
    ELEMENT_VALUES.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|(w, _)| w.strong_count() > 0);
        if let Some((_, val)) = v.iter().find(|(w, _)| std::ptr::eq(w.as_ptr(), Rc::as_ptr(e))) {
            return val.clone();
        }
        let val = Value::Object(Rc::new(ChatElementObject(e.clone())));
        v.push((Rc::downgrade(e), val.clone()));
        val
    })
}

fn npe() -> LuaError {
    ChatError::npe("").into()
}

fn line_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<String> {
    str_arg(c, i).map(|s| s.to_string()).ok_or_else(npe)
}

fn f32_arg(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

/// A `UIFont` constant (`null` otherwise).
fn font_arg(c: &NativeCall<'_>, i: usize) -> Option<UIFont> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == "zombie.ui.UIFont")
            .and_then(|e| UIFont::ALL.get(e.ordinal as usize).copied()),
        _ => None,
    }
}

fn opt_str_value(s: Option<String>) -> Value {
    s.map_or(Value::Nil, |s| Value::from(s.as_str()))
}

/// Runs `f` on the character, flushes its queued chat calls, then fires
/// the chat's Lua events.
fn speak(c: &mut NativeCall<'_>, f: impl FnOnce(&mut LuaGameCharacter)) -> LuaResult<usize> {
    let chr = chr_of(c)?;
    f(&mut chr.borrow_mut());
    let r = flush_chat(&chr);
    fire_pending_events(c.thread);
    r?;
    Ok(0)
}

/// Runs a chat call on the character's element, then fires the chat's Lua
/// events.
fn on_element<R>(c: &mut NativeCall<'_>, f: impl FnOnce(&mut ChatElement) -> R) -> LuaResult<R> {
    let el = chat_element(&chr_of(c)?);
    let r = f(&mut el.borrow_mut());
    fire_pending_events(c.thread);
    Ok(r)
}

/// The `IsoGameCharacter` and `IsoPlayer` members that speak or read
/// speech.
pub fn register(thread: &mut Thread) {
    register_character(thread);
    register_player(thread);
}

fn register_character(thread: &mut Thread) {
    let b = expose_class(thread, ISO_GAME_CHARACTER, None);
    // Say(line) / Say(line, r, g, b, font, baseRange, customTag).
    b.method("Say", |c| {
        let line = line_arg(c, 1)?;
        if c.nargs() >= 8 {
            let (r, g, bl, range) = (f32_arg(c, 2), f32_arg(c, 3), f32_arg(c, 4), f32_arg(c, 6));
            let tag = str_arg(c, 7).map(|s| s.to_string()).ok_or_else(npe)?;
            return speak(c, |chr| chr.say_with(&line, r, g, bl, range, &tag));
        }
        speak(c, |chr| chr.say(&line))
    });
    b.method("setSayLine", |c| {
        let line = line_arg(c, 1)?;
        speak(c, |chr| chr.say(&line))
    });
    b.method("SayWhisper", |c| {
        let line = line_arg(c, 1)?;
        speak(c, |chr| chr.say_whisper(&line))
    });
    b.method("SayShout", |c| {
        let line = line_arg(c, 1)?;
        speak(c, |chr| chr.say_shout(&line))
    });
    // SayRadio(line, r, g, b, font, baseRange, channel, customTag).
    b.method("SayRadio", |c| {
        let line = line_arg(c, 1)?;
        let (r, g, bl, range, channel) = (f32_arg(c, 2), f32_arg(c, 3), f32_arg(c, 4), f32_arg(c, 6), int_arg(c, 7));
        let tag = str_arg(c, 8).map(|s| s.to_string()).ok_or_else(npe)?;
        speak(c, |chr| chr.say_radio(&line, r, g, bl, range, channel, &tag))
    });
    b.method("triggerCough", |c| {
        let ctx = crate::bind_character_sim::current_sim_context();
        speak(c, |chr| chr.trigger_cough(&*ctx))
    });
    // SayDebug(text) / SayDebug(n, text).
    b.method("SayDebug", |c| {
        let (n, text) = if c.nargs() >= 3 { (int_arg(c, 1), str_arg(c, 2)) } else { (0, str_arg(c, 1)) };
        let text = text.map(|s| s.to_string()).ok_or_else(npe)?;
        on_element(c, |e| e.say_debug(n, &text))?;
        Ok(0)
    });
    // addLineChatElement(text[, r, g, b[, font, baseRange, customTag[,
    // bbcode, img, icons, colors, fonts, equalizeHeights]]]).
    b.method("addLineChatElement", |c| {
        let text = line_arg(c, 1)?;
        let n = c.nargs();
        let (r, g, bl) = if n >= 5 { (f32_arg(c, 2), f32_arg(c, 3), f32_arg(c, 4)) } else { (1.0, 1.0, 1.0) };
        let (font, range, tag) = if n >= 8 {
            (font_arg(c, 5).unwrap_or(UIFont::Dialogue), f32_arg(c, 6), str_arg(c, 7).map(|s| s.to_string()).ok_or_else(npe)?)
        } else {
            (UIFont::Dialogue, 30.0, "default".to_string())
        };
        let flags: [bool; 6] =
            if n >= 14 { std::array::from_fn(|i| bool_arg(c, 8 + i)) } else { [false, false, false, false, false, true] };
        on_element(c, |e| {
            e.add_chat_line_full(&text, r, g, bl, font, range, &tag, flags[0], flags[1], flags[2], flags[3], flags[4], flags[5])
        })?;
        Ok(0)
    });
    b.method("getChatElement", |c| {
        let el = chat_element(&chr_of(c)?);
        Ok(c.push(element_value(&el)))
    });
    b.method("getMaxChatLines", |c| {
        let v = chat_element(&chr_of(c)?).borrow().get_max_chat_lines();
        Ok(c.push(v))
    });
    b.method("getTalkerType", |c| {
        let v = chat_element(&chr_of(c)?).borrow().get_talker_type().to_string();
        Ok(c.push(Value::from(v.as_str())))
    });
    b.method("getSayLine", |c| {
        let v = chat_element(&chr_of(c)?).borrow().get_say_line().map(str::to_string);
        Ok(c.push(opt_str_value(v)))
    });
    for name in ["IsSpeaking", "isSpeaking"] {
        b.method(name, |c| {
            let v = chat_element(&chr_of(c)?).borrow().is_speaking();
            Ok(c.push(v))
        });
    }
    b.method("IsSpeakingNPC", |c| {
        let v = chat_element(&chr_of(c)?).borrow().is_speaking_npc();
        Ok(c.push(v))
    });
    b.method("setSpeaking", |c| {
        chr_of(c)?.borrow_mut().chat.speaking = bool_arg(c, 1);
        Ok(0)
    });
    b.method("getSpeakTime", |c| {
        let v = chr_of(c)?.borrow().chat.speak_time;
        Ok(c.push(Value::Num(f64::from(v))))
    });
    b.method("setSpeakTime", |c| {
        chr_of(c)?.borrow_mut().chat.speak_time = int_arg(c, 1) as f32;
        Ok(0)
    });
    b.method("getLastSpokenLine", |c| {
        let v = chr_of(c)?.borrow().chat.last_spoken_line.clone();
        Ok(c.push(opt_str_value(v)))
    });
    b.method("setLastSpokenLine", |c| {
        chr_of(c)?.borrow_mut().chat.last_spoken_line = str_arg(c, 1).map(|s| s.to_string());
        Ok(0)
    });
    b.method("isAllowConversation", |c| {
        let v = chr_of(c)?.borrow().chat.allow_conversation;
        Ok(c.push(v))
    });
    b.method("setAllowConversation", |c| {
        chr_of(c)?.borrow_mut().chat.allow_conversation = bool_arg(c, 1);
        Ok(0)
    });
    // `speakColour` is held by value: `getSpeakColour()` is a copy.
    b.method("getSpeakColour", |c| {
        let [r, g, bl, a] = chr_of(c)?.borrow().speak_colour;
        let col = Color::rgba(r, g, bl, a);
        Ok(c.push(Value::Object(Rc::new(col))))
    });
    b.method("setSpeakColour", |c| {
        let v = c.arg(1);
        let rgba = match &v {
            Value::Object(o) => o.as_any().downcast_ref::<Color>().map(|k| [k.r.get(), k.g.get(), k.b.get(), k.a.get()]),
            _ => None,
        };
        let Some(rgba) = rgba else {
            return Err(LuaError::runtime(format!(
                "Expected zombie.core.Color but got {}",
                pz_kahlua::stdlib::java_class_name(&v)
            )));
        };
        chr_of(c)?.borrow_mut().speak_colour = rgba;
        Ok(0)
    });
    // setSpeakColourInfo(info): `new Color(info.r, info.g, info.b, 1)`.
    b.method("setSpeakColourInfo", |c| {
        let info = match c.arg(1) {
            Value::Object(o) => o.as_any().downcast_ref::<LuaColorInfo>().map(|i| i.c.get()),
            _ => return Err(npe()),
        };
        let info = info.ok_or_else(|| LuaError::runtime("Expected ColorInfo"))?;
        chr_of(c)?.borrow_mut().speak_colour = [info.r, info.g, info.b, 1.0];
        Ok(0)
    });
    // `GameClient.client` is false: no name tag.
    b.method("getUserNameHeight", |c| Ok(c.push(0)));
}

fn register_player(thread: &mut Thread) {
    let b = expose_class(thread, ISO_PLAYER, Some(ISO_GAME_CHARACTER));
    // getUsername() / getUsername(Boolean) / getUsername(Boolean, Boolean).
    b.method("getUsername", |c| {
        let v = chr_of(c)?.borrow().username().map(str::to_string);
        Ok(c.push(opt_str_value(v)))
    });
    b.method("setUsername", |c| {
        chr_of(c)?.borrow_mut().set_username(str_arg(c, 1).as_deref());
        Ok(0)
    });
    b.method("updateUsername", |c| {
        let ctx = crate::bind_character_sim::current_sim_context();
        chr_of(c)?.borrow_mut().update_username(&*ctx).ok_or_else(npe)?;
        Ok(0)
    });
    b.method("getDisplayName", |c| {
        let v = chr_of(c)?.borrow_mut().display_name();
        Ok(c.push(opt_str_value(v)))
    });
    b.method("setDisplayName", |c| {
        chr_of(c)?.borrow_mut().set_display_name(str_arg(c, 1).as_deref());
        Ok(0)
    });
    b.method("resetDisplayName", |c| {
        chr_of(c)?.borrow_mut().reset_display_name();
        Ok(0)
    });
    b.method("isAllChatMuted", |c| {
        let v = chr_of(c)?.borrow().player_fields.all_chat_muted;
        Ok(c.push(v))
    });
    b.method("setAllChatMuted", |c| {
        chr_of(c)?.borrow_mut().player_fields.all_chat_muted = bool_arg(c, 1);
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::host::{install, HudGameTime, HudHost};
    use pz_anim::AnimationVariableSource;
    use pz_characters::character::DefaultSimContext;
    use pz_kahlua::TableRef;
    use pz_scripts::ScriptManager;

    struct Multiplier(f32);
    impl HudGameTime for Multiplier {
        fn get_multiplier(&self) -> f32 {
            self.0
        }
    }

    fn new_chr(kind: CharacterKind, x: f32, y: f32) -> LuaGameCharacterRef {
        let sm = ScriptManager::new();
        let mut c = LuaGameCharacter::new(&sm, kind);
        c.vars.x = x;
        c.vars.y = y;
        Rc::new(RefCell::new(c))
    }

    /// Player 0 (the instance) at (10, 10), the chat initialised on it.
    fn setup() -> LuaGameCharacterRef {
        install(HudHost { game_time: Box::new(Multiplier(1.0)), ..HudHost::default() });
        bind_players::reset();
        install_character_chat_players();
        crate::bind_core::shared_core().borrow_mut().debug = false;
        set_static_sounds(vec!["<invented-static>".into()]);
        let p = new_chr(CharacterKind::Player, 10.0, 10.0);
        bind_players::set_player(0, Some(p.clone()));
        bind_players::set_player_instance(Some(p.clone()));
        with_chat_manager(|m| m.init(true, p.clone() as Rc<dyn ChatPlayer>)).unwrap();
        p
    }

    fn line(chr: &LuaGameCharacterRef, i: usize) -> String {
        chat_element(chr).borrow().line(0, i).map(|l| l.get_original().to_string()).unwrap_or_default()
    }

    #[test]
    fn element_is_owned_by_the_character() {
        let p = setup();
        let el = chat_element(&p);
        assert!(Rc::ptr_eq(&el, &chat_element(&p)));
        assert_eq!(el.borrow().get_talker_type(), "character");
        assert_eq!(el.borrow().get_max_chat_lines(), 1);
        // The owner is the character itself.
        let owner: &dyn ChatPlayer = &*p;
        assert!(Rc::ptr_eq(&owner.chat_element(), &el));
        assert!(ChatElementOwner::is_player(&*p, 0) && !ChatElementOwner::is_player(&*p, 1));
        assert_eq!(owner.username(), "Bob");
        assert_eq!(ChatPlayer::get_x(&*p), 10.0);
        assert_eq!(ChatPlayer::get_square(&*p), None);
        p.borrow_mut().environment.has_current_square = true;
        p.borrow_mut().environment.square_building = Some(7);
        assert_eq!(ChatPlayer::get_square(&*p), Some(ChatSquare { building: Some(7), room: None }));
        // Read while borrowed: the defaults.
        let b = p.borrow_mut();
        assert_eq!(ChatPlayer::get_x(&*p), 0.0);
        drop(b);
        // Players' slots and character values attach an element.
        let z = new_chr(CharacterKind::Zombie, 0.0, 0.0);
        assert!(z.borrow().chat.element.is_none());
        crate::bind_character_sim::character_value(&z);
        assert!(z.borrow().chat.element.is_some());
        let players = CharacterChatPlayers;
        assert_eq!(players.num_players(), 1);
        assert!(players.player(0).is_some() && players.player(1).is_none() && players.instance().is_some());
    }

    #[test]
    fn say_whisper_shout_and_update_speaking() {
        let p = setup();
        // initTextObjects: five lines for a player.
        p.borrow_mut().update_text_objects(&DefaultSimContext);
        flush_chat(&p).unwrap();
        assert_eq!(chat_element(&p).borrow().get_max_chat_lines(), 5);
        assert!(!p.borrow().chat.speaking);

        p.borrow_mut().say("invented line");
        flush_chat(&p).unwrap();
        assert_eq!(line(&p, 0), "invented line");
        assert_eq!(p.borrow().chat.last_spoken_line.as_deref(), Some("invented line"));
        let say = with_chat_manager(|m| m.chat(ChatType::Say)).unwrap();
        assert_eq!(say.message_count(), 1);

        // Whispering only remembers the line.
        p.borrow_mut().say_whisper("quiet");
        flush_chat(&p).unwrap();
        assert_eq!(line(&p, 0), "invented line");
        assert_eq!(p.borrow().chat.last_spoken_line.as_deref(), Some("quiet"));

        p.borrow_mut().say_shout("LOUD");
        flush_chat(&p).unwrap();
        assert_eq!((line(&p, 0), line(&p, 1)), ("LOUD".into(), "invented line".into()));
        let shout = with_chat_manager(|m| m.chat(ChatType::Shout)).unwrap();
        assert_eq!(shout.message_count(), 1);

        // Radio: a line on the element, then the radio chat (static sound
        // or message; neither is shown in single player).
        p.borrow_mut().say_radio("<invented-static>", 0.5, 0.5, 0.5, 20.0, 0, "radio");
        flush_chat(&p).unwrap();
        assert_eq!(line(&p, 0), "<invented-static>");
        assert_eq!(p.borrow().chat.last_spoken_line.as_deref(), Some("LOUD"));

        // updateTextObjects: speaking follows the newest non-radio line.
        p.borrow_mut().say("again");
        p.borrow_mut().environment.call_out = true;
        let mut ticks = 0;
        loop {
            p.borrow_mut().update_text_objects(&DefaultSimContext);
            flush_chat(&p).unwrap();
            ticks += 1;
            let c = p.borrow();
            assert_eq!(c.environment.speaking, c.chat.speaking);
            if !c.chat.speaking {
                break;
            }
            assert!(c.environment.call_out);
        }
        assert_eq!(ticks, 126);
        assert!(!p.borrow().environment.call_out);

        // No conversation: nothing.
        p.borrow_mut().chat.allow_conversation = false;
        p.borrow_mut().say("muted");
        flush_chat(&p).unwrap();
        assert_eq!(p.borrow().chat.last_spoken_line.as_deref(), Some("again"));
    }

    #[test]
    fn zombies_and_survivors() {
        let _p = setup();
        let z = new_chr(CharacterKind::Zombie, 1.0, 1.0);
        z.borrow_mut().say("brains");
        assert!(z.borrow().chat.requests.is_empty());
        // A zombie still whispers (no line), and has one chat line.
        z.borrow_mut().say_whisper("hiss");
        flush_chat(&z).unwrap();
        assert_eq!(z.borrow().chat.last_spoken_line.as_deref(), Some("hiss"));
        z.borrow_mut().update_text_objects(&DefaultSimContext);
        flush_chat(&z).unwrap();
        assert_eq!(chat_element(&z).borrow().get_max_chat_lines(), 1);
        assert!(z.borrow().username().is_none());

        // `(IsoPlayer) this` fails for a survivor.
        let s = new_chr(CharacterKind::Survivor, 1.0, 1.0);
        s.borrow_mut().say("hello");
        let e = flush_chat(&s).unwrap_err();
        assert_eq!(e.class, "java.lang.ClassCastException");
        assert!(s.borrow().chat.last_spoken_line.is_none());
    }

    #[test]
    fn sneeze_and_sleep_speech() {
        let p = setup();
        flush_chat(&p).unwrap();
        p.borrow_mut().body_damage.sneeze_cough_active = 2;
        p.borrow_mut().update_text_objects(&DefaultSimContext);
        flush_chat(&p).unwrap();
        assert_eq!(line(&p, 0), "IGUI_PlayerText_Cough");
        let ext = p.borrow().get_variable_string("Ext");
        assert_eq!(ext, "Cough");
        // Speaking: no more sneeze text.
        assert!(p.borrow().chat.speaking);
        p.borrow_mut().update_text_objects(&DefaultSimContext);
        assert!(p.borrow().chat.requests.iter().all(|r| !matches!(r, ChatRequest::Say(_))));
        flush_chat(&p).unwrap();

        // Asleep on a square the player sees: a whisper once the counter
        // passes 250 * 60 / 30.
        let p2 = new_chr(CharacterKind::Player, 0.0, 0.0);
        {
            let mut c = p2.borrow_mut();
            c.asleep = true;
            c.environment.has_current_square = true;
            c.environment.square_can_see = true;
        }
        for _ in 0..500 {
            p2.borrow_mut().update_text_objects(&DefaultSimContext);
        }
        flush_chat(&p2).unwrap();
        assert!(p2.borrow().chat.last_spoken_line.is_none());
        p2.borrow_mut().update_text_objects(&DefaultSimContext);
        flush_chat(&p2).unwrap();
        assert_eq!(p2.borrow().chat.last_spoken_line.as_deref(), Some("ZzzZZZzzzz"));
        assert!(ChatElement::is_no_log_text("ZzzZZZzzzz"));
        assert_eq!(p2.borrow().chat.sleep_speech_cnt, 0);
    }

    /// Ticks of `updateTextObjects` until the speaker stops speaking and
    /// until the line is gone, per game speed (the original gives the same
    /// counts).
    #[test]
    fn speaking_and_display_time_by_multiplier() {
        for (m, speaking, display) in [(1.0, 126, 252), (3.0, 42, 84), (0.5, 252, 503), (4.0, 32, 63)] {
            let p = setup();
            install(HudHost { game_time: Box::new(Multiplier(m)), ..HudHost::default() });
            // The first update sets up the text objects (five lines; a
            // line said before it would be dropped, as in the original).
            p.borrow_mut().update_text_objects(&DefaultSimContext);
            flush_chat(&p).unwrap();
            p.borrow_mut().say("invented line");
            let el = chat_element(&p);
            let mut ticks = 0;
            let mut speaking_ticks = 0;
            loop {
                p.borrow_mut().update_text_objects(&DefaultSimContext);
                flush_chat(&p).unwrap();
                ticks += 1;
                if speaking_ticks == 0 && !p.borrow().chat.speaking {
                    speaking_ticks = ticks;
                }
                if !el.borrow().get_has_chat_to_display() {
                    break;
                }
            }
            assert_eq!((speaking_ticks, ticks), (speaking, display), "multiplier {m}");
        }
    }

    fn run(t: &mut Thread, src: &str) {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        if let Err((e, tr)) = t.pcall(&f, &[]) {
            panic!("lua error: {e:?}\n{tr}");
        }
    }

    #[test]
    fn lua_members() {
        let p = setup();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_text::register(&mut t);
        register(&mut t);
        t.globals().set_str("player", crate::bind_character_sim::character_value(&p));
        run(
            &mut t,
            r#"
            local function assert(c) if not c then error("check failed") end end
            assert(player:getUsername() == "Bob")
            assert(player:getUsername(true, false) == "Bob")
            assert(player:getDisplayName() == "Bob")
            player:setDisplayName("Invented Name")
            assert(player:getDisplayName() == "Invented Name")
            player:resetDisplayName()
            player:setUsername("Tester")
            assert(player:getDisplayName() == "Tester")
            assert(player:getMaxChatLines() == 1)
            assert(player:getTalkerType() == "character")
            assert(player:getChatElement() == player:getChatElement())
            assert(not player:isSpeaking())
            player:Say("first words")
            assert(player:isSpeaking() == false)
            assert(player:getLastSpokenLine() == "first words")
            assert(player:getSayLine() == "first words")
            player:SayWhisper("psst")
            assert(player:getLastSpokenLine() == "psst")
            player:addLineChatElement("plain", 1, 0, 0)
            assert(player:getSayLine() == "plain")
            player:SayDebug(0, "debug text")
            assert(player:getSayLine() == "debug text")
            assert(player:isAllowConversation())
            player:setAllowConversation(false)
            player:SayShout("ignored")
            assert(player:getLastSpokenLine() == "psst")
            player:setSpeakTime(3)
            assert(player:getSpeakTime() == 3)
            assert(not player:isAllChatMuted())
            player:setAllChatMuted(true)
            assert(player:isAllChatMuted())
            assert(player:getUserNameHeight() == 0)
            player:setSpeakColour(player:getSpeakColour())
            "#,
        );
        assert_eq!(p.borrow().speak_colour, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(line(&p, 0), "debug text");
    }
}
