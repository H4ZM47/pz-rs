//! The character's side of `zombie.chat`: `IsoGameCharacter`'s speech
//! fields (`allowConversation`, `speaking`, `speakTime`, `lastSpokenLine`,
//! `chatElement`), `IsoPlayer.username` / `displayName`, and the speech the
//! simulation itself raises (`DoSneezeText`, `doSleepSpeech`,
//! `triggerCough`) and the character part of `updateTextObjects`.
//!
//! The chat (`ChatElement`, `ChatManager`) lives in `pz-lua-api`, which
//! depends on this crate, so the character cannot call it. What the
//! original does on the chat from inside the simulation is queued as a
//! [`ChatRequest`] in [`CharacterChat::requests`], in the original's
//! order; `pz-lua-api`'s `chat::characters::flush_chat` runs them on the
//! character's chat element (held type-erased in
//! [`CharacterChat::element`]) once the update returns.

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use pz_anim::{AnimationVariableSource, VariableValue};

use super::context::{SimContext, SimEvent};
use super::GameCharacter;

/// `IsoPlayer`'s constructor: `username = "Bob"`.
pub const DEFAULT_USERNAME: &str = "Bob";

/// `IsoGameCharacter.sleepText` once `doSleepSpeech` has set it.
pub const SLEEP_TEXT: &str = "ZzzZZZzzzz";

thread_local! {
    /// `IsoGameCharacter.sleepText != null` (the static is set on the first
    /// sleep speech).
    static SLEEP_TEXT_SET: Cell<bool> = const { Cell::new(false) };
}

/// The arguments of `IsoGameCharacter.ProcessSay(line, r, g, b,
/// baseRange, channel, customTag)`.
#[derive(Clone, Debug, PartialEq)]
pub struct SayArgs {
    pub line: String,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub base_range: f32,
    pub channel: i32,
    pub tag: String,
}

/// A chat call the simulation made, run on the chat by `pz-lua-api`.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatRequest {
    /// `ProcessSay(...)` (from `Say`, `SayWhisper`, `SayShout`, `SayRadio`).
    Say(SayArgs),
    /// `ChatElement.addNoLogText(text)`.
    AddNoLogText(String),
    /// `chatElement.setMaxChatLines(n)` (`initTextObjects`).
    SetMaxChatLines(i32),
    /// The end of `updateTextObjects`: `chatElement.update()`, then
    /// `speaking = chatElement.IsSpeaking()`, cleared with `callOut` when
    /// not speaking or dead.
    UpdateElement,
}

/// `IsoGameCharacter`'s (and `IsoPlayer`'s) chat state.
pub struct CharacterChat {
    /// `allowConversation` (true from the field initializer).
    pub allow_conversation: bool,
    /// `speaking` (`setSpeaking`; `updateTextObjects` derives it).
    pub speaking: bool,
    /// `speakTime`.
    pub speak_time: f32,
    /// `lastSpokenLine`.
    pub last_spoken_line: Option<String>,
    /// `IsoPlayer.username` ("Bob" from the constructor).
    pub username: Option<String>,
    /// `hasInitTextObjects`.
    pub has_init_text_objects: bool,
    /// `isoPlayer` (set by `initTextObjects` for players).
    pub iso_player: bool,
    /// `sleepSpeechCnt`.
    pub sleep_speech_cnt: i32,
    /// `drawUserName`, `canSeeCurrent` (`updateTextObjects` clears them;
    /// `renderlast` sets them for the multiplayer name tag).
    pub draw_user_name: bool,
    pub can_see_current: bool,
    /// `chatElement`: `pz-lua-api`'s `Rc<RefCell<ChatElement>>`, attached
    /// when the character is created (its owner is a `Weak` of the
    /// character).
    pub element: Option<Rc<dyn Any>>,
    /// Chat calls made inside the simulation, oldest first.
    pub requests: Vec<ChatRequest>,
}

impl Default for CharacterChat {
    fn default() -> Self {
        CharacterChat {
            allow_conversation: true,
            speaking: false,
            speak_time: 0.0,
            last_spoken_line: None,
            username: None,
            has_init_text_objects: false,
            iso_player: false,
            sleep_speech_cnt: 0,
            draw_user_name: false,
            can_see_current: false,
            element: None,
            requests: Vec::new(),
        }
    }
}

impl std::fmt::Debug for CharacterChat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CharacterChat")
            .field("allow_conversation", &self.allow_conversation)
            .field("speaking", &self.speaking)
            .field("speak_time", &self.speak_time)
            .field("last_spoken_line", &self.last_spoken_line)
            .field("username", &self.username)
            .field("has_element", &self.element.is_some())
            .field("requests", &self.requests)
            .finish()
    }
}

impl<I: 'static> GameCharacter<I> {
    /// `speakColour`'s r, g, b.
    fn speak_rgb(&self) -> (f32, f32, f32) {
        let [r, g, b, _] = self.speak_colour;
        (r, g, b)
    }

    fn process_say(&mut self, line: &str, (r, g, b): (f32, f32, f32), base_range: f32, channel: i32, tag: &str) {
        self.chat.requests.push(ChatRequest::Say(SayArgs {
            line: line.to_string(),
            r,
            g,
            b,
            base_range,
            channel,
            tag: tag.to_string(),
        }));
    }

    /// `Say(line)`: zombies say nothing; others speak in `speakColour`,
    /// 30 tiles, tag `default`.
    pub fn say(&mut self, line: &str) {
        if self.is_zombie() {
            return;
        }
        self.process_say(line, self.speak_rgb(), 30.0, 0, "default");
    }

    /// `Say(line, r, g, b, font, baseRange, customTag)` (the font is not
    /// used).
    pub fn say_with(&mut self, line: &str, r: f32, g: f32, b: f32, base_range: f32, tag: &str) {
        self.process_say(line, (r, g, b), base_range, 0, tag);
    }

    /// `SayWhisper(line)`: 10 tiles, tag `whisper`.
    pub fn say_whisper(&mut self, line: &str) {
        self.process_say(line, self.speak_rgb(), 10.0, 0, "whisper");
    }

    /// `SayShout(line)`: 60 tiles, tag `shout`.
    pub fn say_shout(&mut self, line: &str) {
        self.process_say(line, self.speak_rgb(), 60.0, 0, "shout");
    }

    /// `SayRadio(line, r, g, b, font, baseRange, channel, customTag)` (the
    /// font is not used).
    #[allow(clippy::too_many_arguments)]
    pub fn say_radio(&mut self, line: &str, r: f32, g: f32, b: f32, base_range: f32, channel: i32, tag: &str) {
        self.process_say(line, (r, g, b), base_range, channel, tag);
    }

    /// `IsoPlayer.getUsername()` in single player (`usernameDisguised`
    /// only changes on a client or server, and the name decorations are
    /// client-only): `username`.
    pub fn username(&self) -> Option<&str> {
        self.chat.username.as_deref()
    }

    /// `IsoPlayer.setUsername(name)`.
    pub fn set_username(&mut self, name: Option<&str>) {
        self.chat.username = name.map(str::to_string);
    }

    /// `IsoPlayer.updateUsername()`: forename and surname run together
    /// (nothing on a client or server; a `NullPointerException` without a
    /// descriptor is `None`).
    pub fn update_username(&mut self, ctx: &dyn SimContext) -> Option<()> {
        if ctx.is_client() || ctx.is_server() {
            return Some(());
        }
        let d = self.descriptor.as_ref()?.clone();
        let d = d.borrow();
        let or_null = |s: &Option<String>| s.clone().unwrap_or_else(|| "null".into());
        self.chat.username = Some(format!("{}{}", or_null(&d.forename), or_null(&d.surname)));
        Some(())
    }

    /// `IsoPlayer.getDisplayName()` in single player: `displayName`, set to
    /// `getUsername(..)` (= `username`) when null or empty.
    pub fn display_name(&mut self) -> Option<String> {
        let empty = self.player_fields.display_name.as_deref().is_none_or(str::is_empty);
        if empty {
            self.player_fields.display_name = self.chat.username.clone();
        }
        self.player_fields.display_name.clone()
    }

    /// `IsoPlayer.setDisplayName(name)`.
    pub fn set_display_name(&mut self, name: Option<&str>) {
        self.player_fields.display_name = name.map(str::to_string);
    }

    /// `IsoPlayer.resetDisplayName()` (the client's disguise refresh is not
    /// reached in single player).
    pub fn reset_display_name(&mut self) {
        self.set_display_name(None);
    }

    /// `DoSneezeText()`: the sneeze or cough line for the body damage's
    /// `IsSneezingCoughing()` (1 sneeze, 2 cough, 3 muffled sneeze, 4
    /// muffled cough), with its `Ext` animation variable, the player's voice
    /// sound, `Say`, and the `EventDoExt` animation event.
    pub fn do_sneeze_text(&mut self, ctx: &dyn SimContext) {
        let player = self.is_player() || self.is_animal();
        let mut text = None;
        let kind = self.body_damage.is_sneezing_coughing();
        if kind <= 0 {
            return;
        }
        if kind == 1 {
            text = Some(ctx.get_text("IGUI_PlayerText_Sneeze"));
            let n = ctx.rand_next(2) + 1;
            self.set_variable("Ext", VariableValue::String(n.to_string()));
            if player {
                ctx.event(SimEvent::PlayerVoiceSound("SneezeHeavy"));
            }
        }
        if self.body_damage.is_sneezing_coughing() == 2 {
            text = Some(ctx.get_text("IGUI_PlayerText_Cough"));
            self.set_variable("Ext", VariableValue::String("Cough".into()));
            if player {
                ctx.event(SimEvent::PlayerVoiceSound("Cough"));
            }
        }
        if self.body_damage.is_sneezing_coughing() == 3 {
            text = Some(ctx.get_text("IGUI_PlayerText_SneezeMuffled"));
            let n = ctx.rand_next(2) + 1;
            self.set_variable("Ext", VariableValue::String(n.to_string()));
            if player {
                ctx.event(SimEvent::PlayerVoiceSound("SneezeLight"));
            }
        }
        if self.body_damage.is_sneezing_coughing() == 4 {
            text = Some(ctx.get_text("IGUI_PlayerText_CoughMuffled"));
            self.set_variable("Ext", VariableValue::String("Cough".into()));
            if player {
                ctx.event(SimEvent::PlayerVoiceSound("MuffledCough"));
            }
        }
        if let Some(text) = text {
            self.say(&text);
            self.report_event("EventDoExt");
        }
    }

    /// `triggerCough()`.
    pub fn trigger_cough(&mut self, ctx: &dyn SimContext) {
        self.set_variable("Ext", VariableValue::String("Cough".into()));
        self.say(&ctx.get_text("IGUI_PlayerText_Cough"));
        self.report_event("EventDoExt");
        if self.is_player() || self.is_animal() {
            ctx.event(SimEvent::PlayerVoiceSound("Cough"));
        }
    }

    /// `doSleepSpeech()`: "ZzzZZZzzzz" whispered once the counter passes
    /// `250 * lockFPS / 30` (the first time also kept out of the chat log).
    pub fn do_sleep_speech(&mut self, ctx: &dyn SimContext) {
        self.chat.sleep_speech_cnt += 1;
        if self.chat.sleep_speech_cnt as f32 > (250 * ctx.lock_fps()) as f32 / 30.0 {
            self.chat.sleep_speech_cnt = 0;
            if !SLEEP_TEXT_SET.with(|s| s.replace(true)) {
                self.chat.requests.push(ChatRequest::AddNoLogText(SLEEP_TEXT.into()));
            }
            self.say_whisper(SLEEP_TEXT);
        }
    }

    /// `initTextObjects()`: players (not animals) get five chat lines. The
    /// multiplayer name tag (`userName`) and the halo note are the
    /// renderer's.
    pub fn init_text_objects(&mut self) {
        self.chat.has_init_text_objects = true;
        if self.is_player() {
            self.chat.requests.push(ChatRequest::SetMaxChatLines(5));
            self.chat.iso_player = true;
        }
    }

    /// `updateTextObjects()` up to the chat element's update, which is
    /// queued ([`ChatRequest::UpdateElement`]). `radioEquipedCheck` and the halo note's
    /// clock are TODO(radio, halo).
    pub fn update_text_objects(&mut self, ctx: &dyn SimContext) {
        if ctx.is_server() {
            return;
        }
        if !self.chat.has_init_text_objects {
            self.init_text_objects();
        }
        if !self.chat.speaking {
            self.do_sneeze_text(ctx);
            if self.is_asleep() && self.environment.has_current_square && self.environment.square_can_see {
                self.do_sleep_speech(ctx);
            }
        }
        self.chat.speaking = false;
        self.chat.draw_user_name = false;
        self.chat.can_see_current = false;
        self.chat.requests.push(ChatRequest::UpdateElement);
    }

    /// The tail of `updateTextObjects` once the chat element has updated:
    /// `speaking = chatElement.IsSpeaking()`, and with it the
    /// [`IsSpeaking()`](super::context::CharacterEnvironment::speaking)
    /// the simulation reads; not speaking or dead clears `speaking` and
    /// `callOut`.
    pub fn after_chat_element_update(&mut self, element_speaking: bool) {
        self.environment.speaking = element_speaking;
        self.chat.speaking = element_speaking;
        if !self.chat.speaking || self.is_dead() {
            self.chat.speaking = false;
            self.environment.call_out = false;
        }
    }

    /// Takes the queued chat calls.
    pub fn take_chat_requests(&mut self) -> Vec<ChatRequest> {
        std::mem::take(&mut self.chat.requests)
    }
}
