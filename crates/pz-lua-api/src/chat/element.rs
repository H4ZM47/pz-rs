//! `zombie.chat.ChatElement`: the speech lines over a speaker (character,
//! radio, television, vehicle part), one stack of
//! [`TextDrawObject`]s per local player, each heard (or scrambled) by
//! that player's distance and building.
//!
//! [`ChatElement::render_batched`] queues a player's lines for
//! [`ChatElement::render_batch`], as `renderBatched`/`RenderBatch` do.

use super::utility::{player_within_bounds, scramble_string, scramble_value, Speaker};
use super::{players, ChatElementOwner, ChatOwnerKind, ChatPlayer};
use crate::hud::host::with_host;
use crate::hud::text_draw::{chat_backdrop, ensure_chat_backdrop, TextDrawObject};
use pz_render::{DrawCmd, Rgba};
use pz_text::UIFont;
use pz_ui::{with_ui, UiDrawCmd};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};

/// `IsoPlayer.MAX` local players.
const MAX_PLAYERS: usize = 4;
/// `PlayerLines.lineDisplayTime`.
const LINE_DISPLAY_TIME: i32 = 314;

thread_local! {
    /// `ChatElement.doBackDrop`.
    static DO_BACKDROP: Cell<bool> = const { Cell::new(true) };
    /// `ChatElement.renderBatch`: per player, the elements queued this frame.
    static RENDER_BATCH: RefCell<[Vec<Rc<RefCell<ChatElement>>>; MAX_PLAYERS]> = RefCell::default();
    /// `ChatElement.noLogText`.
    static NO_LOG_TEXT: RefCell<HashSet<String>> = RefCell::default();
}

/// `ChatElement.PlayerLines`: one player's view of the lines, newest first.
struct PlayerLines {
    line_display_time: i32,
    render_x: i32,
    render_y: i32,
    ignore_radio_lines: bool,
    chat_lines: Vec<TextDrawObject>,
}

impl PlayerLines {
    fn new(lines: usize) -> Self {
        let chat_lines = (0..lines)
            .map(|_| {
                let mut t = TextDrawObject::new(0, 0, 0, true, true, true, true, true, true);
                t.set_default_font(UIFont::Medium);
                t
            })
            .collect();
        PlayerLines {
            line_display_time: LINE_DISPLAY_TIME,
            render_x: 0,
            render_y: 0,
            ignore_radio_lines: false,
            chat_lines,
        }
    }

    fn set_max_chars_per_line(&mut self, n: i32) {
        for l in &mut self.chat_lines {
            l.set_max_chars_per_line(n);
        }
    }

    /// `getNewLineObject()`: the oldest line, cleared and moved to the front.
    fn new_line_object(&mut self) -> Option<&mut TextDrawObject> {
        let mut last = self.chat_lines.pop()?;
        last.clear();
        self.chat_lines.insert(0, last);
        self.chat_lines.first_mut()
    }
}

/// `zombie.chat.ChatElement`.
pub struct ChatElement {
    player_lines: Vec<PlayerLines>,
    owner: Weak<dyn ChatElementOwner>,
    history_val: f32,
    history_in_range: bool,
    history_range: f32,
    use_euclidean: bool,
    has_chat_to_display: bool,
    max_chat_lines: i32,
    max_chars_per_line: i32,
    say_line: Option<String>,
    say_line_tag: Option<String>,
    speaking: bool,
    speaking_npc: bool,
    talker_type: String,
}

impl ChatElement {
    /// `new ChatElement(owner, numberOfLines, talkerType)` (a character's
    /// is `(this, 1, "character")`). Creates the shared backdrop texture.
    pub fn new(owner: Weak<dyn ChatElementOwner>, lines: i32, talker_type: Option<&str>) -> Self {
        let mut e = ChatElement {
            player_lines: Vec::new(),
            owner,
            history_val: 1.0,
            history_in_range: false,
            history_range: 15.0,
            use_euclidean: true,
            has_chat_to_display: false,
            max_chat_lines: -1,
            max_chars_per_line: -1,
            say_line: None,
            say_line_tag: None,
            speaking: false,
            speaking_npc: false,
            talker_type: talker_type.unwrap_or("unknown").to_string(),
        };
        e.set_max_chat_lines(lines);
        e.set_max_chars_per_line(75);
        ensure_chat_backdrop();
        e
    }

    /// `setMaxChatLines(n)`: capped at 10; a change rebuilds every
    /// player's lines (without re-applying the characters per line). As in
    /// the original, a value below 1 is not raised (0 gives no lines).
    pub fn set_max_chat_lines(&mut self, n: i32) {
        let n = n.min(10);
        if n != self.max_chat_lines {
            self.max_chat_lines = n;
            self.player_lines = (0..MAX_PLAYERS).map(|_| PlayerLines::new(n.max(0) as usize)).collect();
        }
    }

    pub fn get_max_chat_lines(&self) -> i32 {
        self.max_chat_lines
    }

    pub fn set_max_chars_per_line(&mut self, n: i32) {
        for p in &mut self.player_lines {
            p.set_max_chars_per_line(n);
        }
        self.max_chars_per_line = n;
    }

    /// `IsSpeaking()`: the newest non-radio line is in its first half.
    pub fn is_speaking(&self) -> bool {
        self.speaking
    }
    /// `IsSpeakingNPC()`.
    pub fn is_speaking_npc(&self) -> bool {
        self.speaking_npc
    }
    pub fn get_talker_type(&self) -> &str {
        &self.talker_type
    }
    pub fn set_talker_type(&mut self, t: Option<&str>) {
        self.talker_type = t.unwrap_or("").to_string();
    }
    /// `getSayLine()`.
    pub fn get_say_line(&self) -> Option<&str> {
        self.say_line.as_deref()
    }
    /// `getSayLineTag()`: "" unless speaking.
    pub fn get_say_line_tag(&self) -> &str {
        match (&self.say_line_tag, self.speaking) {
            (Some(t), true) => t,
            _ => "",
        }
    }
    pub fn set_history_range(&mut self, r: f32) {
        self.history_range = r;
    }
    pub fn set_use_euclidean(&mut self, b: bool) {
        self.use_euclidean = b;
    }
    pub fn get_has_chat_to_display(&self) -> bool {
        self.has_chat_to_display
    }
    pub fn history_val(&self) -> f32 {
        self.history_val
    }

    /// One player's line `i` (newest first), for inspection.
    pub fn line(&self, player: usize, i: usize) -> Option<&TextDrawObject> {
        self.player_lines.get(player)?.chat_lines.get(i)
    }

    /// `IsoPlayer.players[i]` for `i < numPlayers`, with `i`.
    fn local_players() -> Vec<(usize, Rc<dyn ChatPlayer>)> {
        let p = players();
        (0..p.num_players().min(MAX_PLAYERS)).filter_map(|i| p.player(i).map(|pl| (i, pl))).collect()
    }

    /// `SayDebug(n, text)`: sets line `n` directly for every local player
    /// (re-arming its clock when the text is unchanged).
    pub fn say_debug(&mut self, n: i32, text: &str) {
        if with_host(|h| h.network.is_server()) || n < 0 || n >= self.max_chat_lines {
            return;
        }
        let n = n as usize;
        let max_chars = self.max_chars_per_line;
        for (i, _) in Self::local_players() {
            let Some(pl) = self.player_lines.get_mut(i) else { continue };
            let time = pl.line_display_time as f32;
            let Some(line) = pl.chat_lines.get_mut(n) else { continue };
            if line.get_original() == text && !text.is_empty() {
                line.set_internal_tick_clock(time);
                continue;
            }
            line.set_settings(true, true, true, true, true, true);
            line.set_internal_tick_clock(time);
            line.set_custom_tag("default");
            line.set_default_colors(Rgba(1.0, 1.0, 1.0, 1.0));
            line.read_string_with(UIFont::Medium, text, max_chars);
        }
        self.say_line = Some(text.to_string());
        self.say_line_tag = Some("default".into());
        self.has_chat_to_display = true;
    }

    /// `Say(line)` (`Talker`): white, 25 tiles.
    pub fn say(&mut self, line: &str) {
        self.add_chat_line_full(
            line,
            1.0,
            1.0,
            1.0,
            UIFont::Dialogue,
            25.0,
            "default",
            false,
            false,
            false,
            false,
            false,
            true,
        );
    }

    /// `addChatLine(msg, r, g, b, baseRange)`.
    pub fn add_chat_line_range(&mut self, msg: &str, r: f32, g: f32, b: f32, base_range: f32) {
        self.add_chat_line_full(
            msg,
            r,
            g,
            b,
            UIFont::Dialogue,
            base_range,
            "default",
            false,
            false,
            false,
            false,
            false,
            true,
        );
    }

    /// `addChatLine(msg, r, g, b)`: 25 tiles.
    pub fn add_chat_line(&mut self, msg: &str, r: f32, g: f32, b: f32) {
        self.add_chat_line_range(msg, r, g, b, 25.0);
    }

    /// `addChatLine(msg, r, g, b, font, baseRange, customTag, bbcode, img,
    /// icons, colors, fonts, equalizeHeights)`: a new line for each local
    /// player who hears it (grey and scrambled when partly heard). Deaf
    /// players miss radios, vehicle parts and televisions not facing them.
    #[allow(clippy::too_many_arguments)]
    pub fn add_chat_line_full(
        &mut self,
        msg: &str,
        r: f32,
        g: f32,
        b: f32,
        font: UIFont,
        base_range: f32,
        custom_tag: &str,
        bbcode: bool,
        img: bool,
        icons: bool,
        colors: bool,
        fonts: bool,
        equalize: bool,
    ) {
        if with_host(|h| h.network.is_server()) {
            return;
        }
        let owner = self.owner.upgrade();
        let max_chars = self.max_chars_per_line;
        for (i, player) in Self::local_players() {
            if player.is_deaf() {
                let misses = match owner.as_ref().map(|o| o.kind()) {
                    Some(ChatOwnerKind::Television) => !owner.as_ref().is_some_and(|o| o.is_facing(&*player)),
                    Some(ChatOwnerKind::Radio | ChatOwnerKind::VehiclePart) => true,
                    _ => false,
                };
                if misses {
                    continue;
                }
            }
            let scramble = self.get_scramble_value(owner.as_deref(), i, &*player, base_range);
            if scramble >= 1.0 {
                continue;
            }
            let Some(pl) = self.player_lines.get_mut(i) else { continue };
            let time = pl.line_display_time as f32;
            let Some(line) = pl.new_line_object() else { continue };
            line.set_settings(bbcode, img, icons, colors, fonts, equalize);
            line.set_internal_tick_clock(time);
            line.set_custom_tag(custom_tag);
            let text = if scramble > 0.0 {
                line.set_default_colors(Rgba(0.5, 0.5, 0.5, 1.0));
                scramble_string(msg, (100.0 * scramble) as i32, "...")
            } else {
                line.set_default_colors(Rgba(r, g, b, 1.0));
                msg.to_string()
            };
            line.read_string_with(font, &text, max_chars);
            self.say_line = Some(msg.to_string());
            self.say_line_tag = Some(custom_tag.to_string());
            self.has_chat_to_display = true;
        }
    }

    /// `getScrambleValue(player, baseRange)`: 0 for the owner itself.
    fn get_scramble_value(
        &self,
        owner: Option<&dyn ChatElementOwner>,
        i: usize,
        player: &dyn ChatPlayer,
        range: f32,
    ) -> f32 {
        let Some(o) = owner else { return 1.0 };
        if o.is_player(i) {
            return 0.0;
        }
        let src = Speaker { x: o.get_x(), y: o.get_y(), z: o.get_z(), square: o.get_square() };
        scramble_value(src, player, range, self.use_euclidean)
    }

    /// `updateChatLines()`: runs the line clocks (each older line 1.2x
    /// faster) and works out whether the speaker is speaking.
    fn update_chat_lines(&mut self) {
        self.speaking = false;
        self.speaking_npc = false;
        let mut found = false;
        if self.has_chat_to_display {
            self.has_chat_to_display = false;
            let n = players().num_players().min(self.player_lines.len());
            let multiplier = with_host(|h| h.game_time.get_multiplier());
            for pl in &mut self.player_lines[..n] {
                let mut delta = 1.25 * multiplier;
                let display = pl.line_display_time;
                for line in &mut pl.chat_lines {
                    let t = line.update_internal_tick_clock_by(delta);
                    if t <= 0.0 {
                        continue;
                    }
                    self.has_chat_to_display = true;
                    if !found && line.get_custom_tag() != "radio" {
                        let alpha = t / (display as f32 / 2.0);
                        if alpha >= 1.0 {
                            self.speaking = true;
                        }
                        if alpha >= 0.0 {
                            self.speaking_npc = true;
                        }
                        found = true;
                    }
                    delta *= 1.2;
                }
            }
        }
        if !self.speaking {
            self.say_line = None;
            self.say_line_tag = None;
        }
    }

    /// `updateHistory()`: older lines fade in and out as the first player
    /// enters and leaves `historyRange`.
    fn update_history(&mut self) {
        if !self.has_chat_to_display {
            self.history_val = 0.0;
            return;
        }
        self.history_in_range = false;
        let Some(player) = players().instance() else { return };
        let owner = self.owner.upgrade();
        if owner.as_ref().is_some_and(|o| o.is_player(player.player_num())) {
            self.history_val = 1.0;
            return;
        }
        self.history_in_range = match &owner {
            Some(o) => player_within_bounds(o.get_x(), o.get_y(), &*player, self.history_range),
            None => false,
        };
        if self.history_in_range && self.history_val != 1.0 {
            self.history_val = (self.history_val + 0.04).min(1.0);
        }
        if !self.history_in_range && self.history_val != 0.0 {
            self.history_val = (self.history_val - 0.04).max(0.0);
        }
    }

    /// `update()`: once per tick (not on a server).
    pub fn update(&mut self) {
        if with_host(|h| h.network.is_server()) {
            return;
        }
        self.update_chat_lines();
        self.update_history();
    }

    /// `renderBatched(playerNum, x, y, ignoreRadioLines)`: queues this
    /// player's lines above (x, y) for [`ChatElement::render_batch`].
    pub fn render_batched(this: &Rc<RefCell<ChatElement>>, player: usize, x: i32, y: i32, ignore_radio_lines: bool) {
        {
            let mut e = this.borrow_mut();
            if player >= e.player_lines.len() || !e.has_chat_to_display || with_host(|h| h.network.is_server()) {
                return;
            }
            let pl = &mut e.player_lines[player];
            pl.render_x = x;
            pl.render_y = y;
            pl.ignore_radio_lines = ignore_radio_lines;
        }
        RENDER_BATCH.with(|b| b.borrow_mut()[player].push(this.clone()));
    }

    /// `clear(playerIndex)`: drops that player's live lines.
    pub fn clear(&mut self, player: usize) {
        if !self.has_chat_to_display {
            return;
        }
        self.has_chat_to_display = false;
        if let Some(pl) = self.player_lines.get_mut(player) {
            for l in &mut pl.chat_lines {
                if l.get_internal_clock() > 0.0 {
                    l.clear();
                    let t = l.get_internal_clock();
                    l.update_internal_tick_clock_by(t);
                }
            }
        }
        self.history_in_range = false;
        self.history_val = 0.0;
    }

    /// `ChatElement.RenderBatch(playerNum)`: draws and drops the queued
    /// elements.
    pub fn render_batch(player: usize) {
        let queued = RENDER_BATCH.with(|b| b.borrow_mut().get_mut(player).map(std::mem::take).unwrap_or_default());
        for e in queued {
            e.borrow_mut().render_lines(player);
        }
    }

    /// `ChatElement.NoRender(playerNum)`.
    pub fn no_render(player: usize) {
        RENDER_BATCH.with(|b| {
            if let Some(v) = b.borrow_mut().get_mut(player) {
                v.clear();
            }
        });
    }

    /// `ChatElement.addNoLogText(text)`.
    pub fn add_no_log_text(text: &str) {
        if !text.is_empty() {
            NO_LOG_TEXT.with(|s| s.borrow_mut().insert(text.to_string()));
        }
    }

    pub fn is_no_log_text(text: &str) -> bool {
        NO_LOG_TEXT.with(|s| s.borrow().contains(text))
    }

    /// `ChatElement.doBackDrop`.
    pub fn set_do_backdrop(v: bool) {
        DO_BACKDROP.with(|d| d.set(v));
    }

    /// `PlayerLines.render()`: newest at the bottom, stacked upwards, each
    /// fading over its last quarter; older lines also by `historyVal`.
    fn render_lines(&mut self, player: usize) {
        if with_host(|h| h.network.is_server()) || !self.has_chat_to_display {
            return;
        }
        let history = self.history_val;
        let backdrop = if DO_BACKDROP.with(|d| d.get()) { chat_backdrop() } else { None };
        let Some(pl) = self.player_lines.get_mut(player) else { return };
        let mut index = 0;
        for line in &mut pl.chat_lines {
            if !line.get_enabled() {
                continue;
            }
            let (w, h) = (line.get_width(), line.get_height());
            if w <= 0 || h <= 0 {
                index += 1;
                continue;
            }
            let t = line.get_internal_clock();
            if t <= 0.0 || (line.get_custom_tag() == "radio" && pl.ignore_radio_lines) {
                continue;
            }
            let alpha = (t / (pl.line_display_time as f32 / 4.0)).min(1.0);
            pl.render_y -= h + 1;
            let outlines = line.get_default_font_enum() != UIFont::Dialogue;
            if let Some(b) = &backdrop {
                with_ui(|m| m.sprite.push(UiDrawCmd::Render(DrawCmd::BlendFunc(770, 771))));
                b.render_inner_based(pl.render_x - w / 2, pl.render_y, w, h, Rgba(0.0, 0.0, 0.0, 0.4));
            }
            let (x, y) = (pl.render_x as f64, pl.render_y as f64);
            if index == 0 {
                line.draw(x, y, outlines, Some(alpha));
            } else if history > 0.0 {
                line.draw(x, y, outlines, Some(alpha * history));
            }
            index += 1;
        }
    }
}
