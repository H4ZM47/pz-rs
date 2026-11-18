//! `GameTime.getDeathString(player)`, `getZombieKilledText(player)` and
//! `getGameModeText()`, which `ISPostDeathUI.OnPlayerDeath` shows when the
//! player dies.

use pz_kahlua::{LuaError, LuaResult, NativeCall, Value};
use pz_text::jfmt::JavaArg;

use crate::bind_character_sim::{CharacterObject, LuaGameCharacterRef};

/// `Translator.getText(key, args)` (the key itself without a translator).
fn text(key: &str, args: &[JavaArg]) -> String {
    crate::bind_text::with_translator(|t| t.get_text(key, args)).unwrap_or_else(|| key.to_string())
}

/// `GameTime.getTimeSurvived(player)` for `getHoursSurvived()` hours:
/// years, months, days and hours (30-day months, 12-month years), each
/// with its singular or plural word; under one hour, minutes and seconds.
pub(crate) fn time_survived(hours: f64, text: impl Fn(&str) -> String) -> String {
    let h = hours as f32;
    let hour = (h as i32) % 24;
    let mut days = (h as i32) / 24;
    let mut months = days / 30;
    days %= 30;
    let years = months / 12;
    months %= 12;
    let mut s = String::new();
    for (n, one, many) in [
        (years, "IGUI_Gametime_year", "IGUI_Gametime_years"),
        (months, "IGUI_Gametime_month", "IGUI_Gametime_months"),
        (days, "IGUI_Gametime_day", "IGUI_Gametime_days"),
        (hour, "IGUI_Gametime_hour", "IGUI_Gametime_hours"),
    ] {
        if n == 0 {
            continue;
        }
        let word = text(if n > 1 { many } else { one });
        if !s.is_empty() {
            s.push_str(", ");
        }
        s = format!("{s}{n} {word}");
    }
    if s.trim().is_empty() {
        let minutes = (h * 60.0) as i32;
        let seconds = (h * 60.0 * 60.0) as i32 - minutes * 60;
        s = format!(
            "{minutes} {}, {seconds} {}",
            text("IGUI_Gametime_minutes"),
            text("IGUI_Gametime_secondes")
        );
    }
    s
}

/// The key of `getZombieKilledText` for `kills`: the plural for 0 or more
/// than one, the singular for one, none (null) for a negative count.
pub(crate) fn zombie_killed_key(kills: i32) -> Option<&'static str> {
    match kills {
        1 => Some("IGUI_Gametime_zombieCount"),
        k if k == 0 || k > 1 => Some("IGUI_Gametime_zombiesCount"),
        _ => None,
    }
}

/// `Translator.getTextOrNull(key, args)`.
fn text_or_null(key: &str, args: &[JavaArg]) -> Option<String> {
    crate::bind_text::with_translator(|t| t.get_text_or_null(key, args)).flatten()
}

/// `getGameModeText()`: the translated game mode (or challenge) name in
/// `IGUI_Gametime_GameMode`, with " (DEBUG)" when `Core.debug` is set.
pub(crate) fn game_mode_text(
    game_mode: &str,
    challenge_id: Option<&str>,
    debug: bool,
    text_or_null: impl Fn(&str, &[JavaArg]) -> Option<String>,
) -> String {
    let mode = match challenge_id.filter(|id| !id.is_empty()) {
        None => text_or_null(&format!("UI_NewGame_{game_mode}"), &[]),
        Some(id) => text_or_null(&format!("Challenge_{id}_name"), &[]),
    }
    .unwrap_or_else(|| game_mode.to_string());
    let mut s = text_or_null("IGUI_Gametime_GameMode", &[JavaArg::Str(mode.clone())])
        .unwrap_or_else(|| format!("Game mode: {mode}"));
    if debug {
        s.push_str(" (DEBUG)");
    }
    s
}

pub(super) fn get_game_mode_text(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let core = crate::bind_core::shared_core();
    let s = {
        let core = core.borrow();
        game_mode_text(&core.game_mode, core.challenge_id.as_deref(), core.debug, text_or_null)
    };
    Ok(c.push(s))
}

/// The `IsoPlayer` argument at `i`; a null argument throws like the
/// original's first call on it.
fn player_arg(c: &NativeCall<'_>, i: usize, method: &str) -> LuaResult<LuaGameCharacterRef> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<CharacterObject>()
            .map(|p| p.0.clone())
            .ok_or_else(|| LuaError::runtime(format!("GameTime: expected an IsoPlayer, got {}", pz_kahlua::stdlib::java_class_name(&c.arg(i))))),
        _ => Err(LuaError::Runtime {
            message: format!("Cannot invoke \"zombie.characters.IsoPlayer.{method}()\" because \"player\" is null"),
            class: "java.lang.NullPointerException",
        }),
    }
}

pub(super) fn get_death_string(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let player = player_arg(c, 1, "getHoursSurvived")?;
    let ctx = crate::bind_character_sim::current_sim_context();
    let hours = player.borrow().hours_survived(&*ctx);
    let survived = time_survived(hours, |k| text(k, &[]));
    Ok(c.push(text("IGUI_Gametime_SurvivedFor", &[JavaArg::Str(survived)])))
}

pub(super) fn get_zombie_killed_text(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let player = player_arg(c, 1, "getZombieKills")?;
    let kills = player.borrow().zombie_kills;
    let v = zombie_killed_key(kills).map_or(Value::Nil, |k| Value::str(&text(k, &[JavaArg::Int(kills)])));
    Ok(c.push(v))
}

#[cfg(test)]
mod tests {
    use super::{time_survived, zombie_killed_key};

    fn words(k: &str) -> String {
        k.trim_start_matches("IGUI_Gametime_").to_string()
    }

    #[test]
    fn time_survived_counts_down_from_years_to_hours_then_minutes() {
        assert_eq!(time_survived(1.0, words), "1 hour");
        assert_eq!(time_survived(26.0, words), "1 day, 2 hours");
        // 400 days: 1 year, 1 month, 10 days.
        assert_eq!(time_survived(400.0 * 24.0 + 5.0, words), "1 year, 1 month, 10 days, 5 hours");
        assert_eq!(time_survived(2.0 * 360.0 * 24.0, words), "2 years");
        assert_eq!(time_survived(0.5, words), "30 minutes, 0 secondes");
        assert_eq!(time_survived(0.0, words), "0 minutes, 0 secondes");
    }

    #[test]
    fn game_mode_text_names_the_mode_or_challenge() {
        use pz_text::jfmt::JavaArg;
        let known = |k: &str, a: &[JavaArg]| match (k, a) {
            ("UI_NewGame_Apocalypse", _) => Some("Apocalypse".to_string()),
            ("IGUI_Gametime_GameMode", [JavaArg::Str(m)]) => Some(format!("Mode: {m}")),
            _ => None,
        };
        assert_eq!(super::game_mode_text("Apocalypse", None, false, known), "Mode: Apocalypse");
        assert_eq!(super::game_mode_text("Apocalypse", Some(""), true, known), "Mode: Apocalypse (DEBUG)");
        // An untranslated challenge falls back to the game mode name.
        assert_eq!(super::game_mode_text("Sandbox", Some("Kingsmouth"), false, known), "Mode: Sandbox");
        assert_eq!(super::game_mode_text("Sandbox", None, false, |_, _| None), "Game mode: Sandbox");
    }

    #[test]
    fn zombie_killed_text_key_by_count() {
        assert_eq!(zombie_killed_key(0), Some("IGUI_Gametime_zombiesCount"));
        assert_eq!(zombie_killed_key(1), Some("IGUI_Gametime_zombieCount"));
        assert_eq!(zombie_killed_key(7), Some("IGUI_Gametime_zombiesCount"));
        assert_eq!(zombie_killed_key(-1), None);
    }
}
