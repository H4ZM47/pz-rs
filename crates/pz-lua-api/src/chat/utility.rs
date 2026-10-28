//! The parts of `zombie.chat.ChatUtility` a single-player game uses (chat
//! streams, hearing range scrambling, the `*colour*`/`*icon*` markup), plus
//! `LuaManager.getHourMinuteJava` and `ZomboidRadio.scrambleString` with a
//! custom scramble text, which the chat calls.

use super::{now, with_icons, ChatPlayer, ChatSquare, ChatType};
use pz_core::config::float_to_string;

/// `ServerOptions.chatStreams`' default.
const DEFAULT_CHAT_STREAMS: &str = "s,r,a,w,y,sh,f,all";

/// `ServerOptions.getInstance()`'s `ChatStreams` and `GlobalChat`, or their
/// defaults when the options cannot be read.
fn stream_options() -> (String, bool) {
    let o = crate::bind_server::shared_instance();
    let o = o.as_ref().map(|o| o.borrow());
    let streams = o.as_ref().and_then(|o| o.get_option("ChatStreams")).unwrap_or_else(|| DEFAULT_CHAT_STREAMS.into());
    let global = o.as_ref().and_then(|o| o.get_boolean("GlobalChat")).unwrap_or(true);
    (streams, global)
}

/// `ChatUtility.getAllowedChatStreams()`: the server chat, then each
/// stream the `ChatStreams` option names ("all" only with `GlobalChat`).
pub fn get_allowed_chat_streams() -> Vec<ChatType> {
    let (streams, global) = stream_options();
    let mut v = vec![ChatType::Server];
    for s in streams.replace('"', "").split(',') {
        match s {
            "s" => v.push(ChatType::Say),
            "r" => v.push(ChatType::Radio),
            "a" => v.push(ChatType::Admin),
            "w" => v.push(ChatType::Whisper),
            "y" => v.push(ChatType::Shout),
            "sh" => v.push(ChatType::Safehouse),
            "f" => v.push(ChatType::Faction),
            "all" if global => v.push(ChatType::General),
            _ => {}
        }
    }
    v
}

/// `ChatUtility.chatStreamEnabled(type)`.
pub fn chat_stream_enabled(t: ChatType) -> bool {
    get_allowed_chat_streams().contains(&t)
}

/// `LuaManager.getHourMinuteJava()`: local "H:mm".
pub fn hour_minute() -> String {
    let t = now();
    format!("{}:{:02}", t.hour, t.minute)
}

/// `Character.isWhitespace`.
pub(crate) fn java_is_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | '\u{001C}'..='\u{001F}')
        || (c.is_whitespace() && !matches!(c, '\u{00A0}' | '\u{2007}' | '\u{202F}') && !c.is_control())
}

/// Where a speaker is, for [`scramble_value`].
#[derive(Clone, Copy, Debug)]
pub struct Speaker {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub square: Option<ChatSquare>,
}

/// `getDistance` to `dest` (Euclidean or Manhattan).
pub fn get_distance(x: f32, y: f32, dest: &dyn ChatPlayer, euclidean: bool) -> f32 {
    let (dx, dy) = (x - dest.get_x(), y - dest.get_y());
    if euclidean {
        ((dx as f64).powi(2) + (dy as f64).powi(2)).sqrt() as f32
    } else {
        dx.abs() + dy.abs()
    }
}

/// `playerWithinBounds`: `dest` inside the square of half-size `dist`.
pub fn player_within_bounds(x: f32, y: f32, dest: &dyn ChatPlayer, dist: f32) -> bool {
    let (px, py) = (dest.get_x(), dest.get_y());
    px > x - dist && px < x + dist && py > y - dist && py < y + dist
}

/// `getScrambleValue`: 0 heard clearly, 1 not heard, otherwise how much of
/// the text is scrambled (at least 0.2). Same building and room doubles
/// the range and always reaches; a wall between halves it; each floor
/// between takes a quarter.
pub fn scramble_value(src: Speaker, dest: &dyn ChatPlayer, base_range: f32, euclidean: bool) -> f32 {
    let mut m = 1.0f32;
    let mut force_scramble = false;
    let mut force_display = false;
    if let (Some(sq), Some(dsq)) = (src.square, dest.get_square()) {
        let db = dest.get_building();
        let dz = (dest.get_z() - src.z).abs();
        if db.is_some() && sq.building.is_some() && db == sq.building {
            if dsq.room == sq.room {
                m = (m as f64 * 2.0) as f32;
                force_display = true;
            } else if dz < 1.0 {
                m = (m as f64 * 2.0) as f32;
            }
        } else if db.is_some() || sq.building.is_some() {
            m = (m as f64 * 0.5) as f32;
            force_scramble = true;
        }
        if dz >= 1.0 {
            m = (m as f64 - m as f64 * (dz as f64 * 0.25)) as f32;
            force_scramble = true;
        }
    }
    let range = base_range * m;
    let mut scramble = 1.0;
    if m > 0.0 && player_within_bounds(src.x, src.y, dest, range) {
        let dist = get_distance(src.x, src.y, dest, euclidean);
        if dist >= 0.0 && dist < range {
            let full = range * 0.6;
            if force_display || (!force_scramble && dist < full) {
                scramble = 0.0;
            } else if range - full != 0.0 {
                scramble = (dist - full) / (range - full);
                if scramble < 0.2 {
                    scramble = 0.2;
                }
            }
        }
    }
    scramble
}

/// `ZomboidRadio.scrambleString(msg, intensity, true, custom)`: words are
/// replaced by `custom` with `intensity`% chance (one `custom` for a run),
/// `[...]` tags kept.
pub fn scramble_string(msg: &str, intensity: i32, custom: &str) -> String {
    if intensity <= 0 {
        return msg.to_string();
    }
    if intensity >= 100 {
        return custom.to_string();
    }
    let chars: Vec<char> = msg.chars().collect();
    let mut out = String::new();
    let mut scrambled = false;
    let mut opened = false;
    let mut word = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if opened {
            word.push(c);
            if c == ']' {
                out.push_str(&word);
                word.clear();
                opened = false;
            }
            continue;
        }
        if c != '[' && (!java_is_whitespace(c) || i == 0 || java_is_whitespace(chars[i - 1])) {
            word.push(c);
            continue;
        }
        if pz_core::random::next(100) > intensity {
            out.push_str(&word);
            out.push(' ');
            scrambled = false;
        } else if !scrambled {
            out.push_str(custom);
            out.push(' ');
            scrambled = true;
        }
        word.clear();
        if c == '[' {
            word.push('[');
            opened = true;
        }
    }
    out.push_str(&word);
    out
}

/// `Integer.parseInt` limited to 0..=255, else -1.
fn parse_color_int(s: &str) -> i32 {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return -1;
    }
    match s.parse::<i32>() {
        Ok(i) if (0..=255).contains(&i) => i,
        _ => -1,
    }
}

/// `getColorString(input, doFloat)`: a named colour or "r,g,b" (0-255) as
/// "r,g,b" ints or floats.
fn get_color_string(input: &str, float: bool) -> Option<String> {
    if let Some(c) = crate::bind_colors::named_color(input) {
        let (r, g, b) = (c.r.get(), c.g.get(), c.b.get());
        return Some(if float {
            format!("{},{},{}", float_to_string(r), float_to_string(g), float_to_string(b))
        } else {
            let i = |v: f32| (v * 255.0) as i32;
            format!("{},{},{}", i(r), i(g), i(b))
        });
    }
    if input.encode_utf16().count() <= 11 && input.contains(',') {
        let mut parts: Vec<&str> = input.split(',').collect();
        while parts.last() == Some(&"") {
            parts.pop();
        }
        if parts.len() == 3 {
            let (r, g, b) = (parse_color_int(parts[0]), parse_color_int(parts[1]), parse_color_int(parts[2]));
            if r != -1 && g != -1 && b != -1 {
                return Some(if float {
                    let f = |v: i32| float_to_string(v as f32 / 255.0);
                    format!("{},{},{}", f(r), f(g), f(b))
                } else {
                    format!("{r},{g},{b}")
                });
            }
        }
    }
    None
}

/// `ChatUtility.parseStringForChatBubble(str)`: `*colour*` becomes
/// `[col=r,g,b]` and `*icon*` (or `*music*`) `[img=name]` (at most ten);
/// `[br/]` and `[cdt=` are removed.
pub fn parse_string_for_chat_bubble(s: &str) -> String {
    let s = s.replace("[br/]", "").replace("[cdt=", "");
    let mut out = String::new();
    let mut test = String::new();
    let mut opened = false;
    let mut opened_color = false;
    let mut images = 0;
    for c in s.chars() {
        if c != '*' {
            if opened {
                test.push(c);
            } else {
                out.push(c);
            }
            continue;
        }
        if !opened {
            opened = true;
            continue;
        }
        let tag = std::mem::take(&mut test);
        if let Some(col) = get_color_string(&tag, false) {
            if opened_color {
                out.push_str("[/]");
            }
            out.push_str(&format!("[col={col}]"));
            opened = false;
            opened_color = true;
            continue;
        }
        let music = tag.eq_ignore_ascii_case("music");
        let icon = with_icons(|icons, _| icons.get(&tag.to_lowercase()).cloned());
        if images < 10 && (music || icon.is_some()) {
            if opened_color {
                out.push_str("[/]");
                opened_color = false;
            }
            let name = if music { "music".to_string() } else { icon.unwrap_or_default() };
            out.push_str(&format!("[img={name}]"));
            opened = false;
            images += 1;
            continue;
        }
        out.push('*');
        out.push_str(&tag);
    }
    if opened {
        out.push('*');
        out.push_str(&test);
        if opened_color {
            out.push_str("[/]");
        }
    }
    out
}

/// `ChatUtility.parseStringForChatLog(str)`: `*colour*` becomes
/// ` <RGB:r,g,b>` and `*icon*` an `<IMAGE:texture,w,h>` at half size
/// (`*music*` three quarters), at most ten.
pub fn parse_string_for_chat_log(s: &str) -> String {
    let mut out = String::new();
    let mut test = String::new();
    let mut opened = false;
    let mut opened_color = false;
    let mut images = 0;
    for c in s.chars() {
        if c != '*' {
            if opened {
                test.push(c);
            } else {
                out.push(c);
            }
            continue;
        }
        if !opened {
            opened = true;
            continue;
        }
        let tag = std::mem::take(&mut test);
        if let Some(col) = get_color_string(&tag, true) {
            out.push_str(&format!(" <RGB:{col}>"));
            opened = false;
            opened_color = true;
            continue;
        }
        let music = tag.eq_ignore_ascii_case("music");
        let icon = with_icons(|_, full| full.get(&tag.to_lowercase()).cloned());
        if images < 10 && (music || icon.is_some()) {
            if opened_color {
                out.push_str(" <RGB:1.0,1.0,1.0>");
                opened_color = false;
            }
            let id = if music { "Icon_music_notes".to_string() } else { icon.unwrap_or_default() };
            if let Some(tex) = pz_render::get_shared_texture(&id) {
                let scale = if music { 0.75 } else { 0.5 };
                let w = (tex.width as f32 * scale) as i32;
                let h = (tex.height as f32 * scale) as i32;
                out.push_str(&format!("<IMAGE:{id},{w},{h}>"));
                opened = false;
                images += 1;
                continue;
            }
        }
        out.push('*');
        out.push_str(&tag);
    }
    if opened {
        out.push('*');
        out.push_str(&test);
    }
    out
}
