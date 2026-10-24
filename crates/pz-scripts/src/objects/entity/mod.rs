//! Script classes of the `entity` group: `zombie.scripting.entity` (game
//! entities, their templates and component scripts, craft recipes) and
//! `zombie.scripting.itemConfig`, plus the `zombie.entity` enums they store.

use std::rc::Rc;

use crate::base::{ScriptError, ScriptRef, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::script_type::ScriptType;

/// Declares a Java enum with `name()`, `values()` and `valueOf` (with the
/// original exception text). Variants are ordered by ordinal.
macro_rules! entity_enum {
    ($(#[$m:meta])* $name:ident, $class:literal, [$($v:ident),* $(,)?]) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($v),*
        }

        impl $name {
            /// `values()`, in ordinal order.
            pub const ALL: &'static [$name] = &[$($name::$v),*];

            /// `name()`.
            pub fn name(self) -> &'static str {
                match self {
                    $($name::$v => stringify!($v)),*
                }
            }

            /// `valueOf`: exact, case-sensitive constant name.
            pub fn value_of(s: &str) -> $crate::base::ScriptResult<$name> {
                Self::ALL.iter().copied().find(|v| v.name() == s).ok_or_else(|| {
                    $crate::objects::entity::illegal_argument(&format!("No enum constant {}.{}", $class, s))
                })
            }
        }
    };
}

#[macro_use]
pub mod component_script;
pub mod component_type;
pub mod components;
pub mod game_entity_script;
pub mod game_entity_template;
pub mod item_config;

pub use component_script::{ComponentBase, ComponentScript};
pub use component_type::ComponentType;
pub use components::crafting::CraftRecipe;
pub use game_entity_script::GameEntityScript;
pub use game_entity_template::GameEntityTemplate;
pub use item_config::ItemConfig;

/// `createInstance` for the script types this group ports; `None` for
/// types it does not (yet) handle.
pub fn create_instance(type_: ScriptType, module: &Rc<ScriptModule>, name: &str, token: &str) -> Option<ScriptRef> {
    Some(match type_ {
        ScriptType::Entity => ScriptRef::new(GameEntityScript::new()),
        ScriptType::EntityTemplate => ScriptRef::new(GameEntityTemplate::new(module, name, token)),
        ScriptType::CraftRecipe => ScriptRef::new(CraftRecipe::new()),
        ScriptType::ItemConfig => ScriptRef::new(ItemConfig::new()),
        _ => return None,
    })
}

/// Java static fields of this group's classes (reset by `ScriptManager.Reset`):
/// `ItemConfig`'s error context, used in `ItemConfigException` messages.
#[derive(Default)]
pub struct Statics {
    pub item_config_error_line: Option<String>,
    pub item_config_error_bucket: Option<String>,
    pub item_config_error_root: Option<String>,
    pub item_config_error_item_config: Option<String>,
}

/// Group steps run at the end of `ScriptManager.Load`.
pub fn post_load(_sm: &ScriptManager) {}

/// `new Exception(msg)`.
pub(crate) fn exception(msg: impl std::fmt::Display) -> ScriptError {
    ScriptError::new(format!("java.lang.Exception: {msg}"))
}

/// `new RuntimeException(msg)`.
pub(crate) fn runtime_exception(msg: impl std::fmt::Display) -> ScriptError {
    ScriptError::new(format!("java.lang.RuntimeException: {msg}"))
}

/// `new IOException(msg)`.
pub(crate) fn io_exception(msg: impl std::fmt::Display) -> ScriptError {
    ScriptError::new(format!("java.io.IOException: {msg}"))
}

/// `new IllegalArgumentException(msg)`.
pub(crate) fn illegal_argument(msg: &str) -> ScriptError {
    ScriptError::new(format!("java.lang.IllegalArgumentException: {msg}"))
}

/// A `NullPointerException` from dereferencing a missing value.
pub(crate) fn null_pointer() -> ScriptError {
    ScriptError::new("java.lang.NullPointerException")
}

/// `array[index]` with Java's bounds check.
pub(crate) fn array_get<'a>(arr: &[&'a str], index: usize) -> ScriptResult<&'a str> {
    arr.get(index).copied().ok_or_else(|| {
        ScriptError::new(format!(
            "java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {}",
            arr.len()
        ))
    })
}

/// `Float.parseFloat`, with its `NumberFormatException` text.
pub(crate) fn parse_float(s: &str) -> ScriptResult<f32> {
    java::parse_f32(s).ok_or_else(|| {
        let t = java::trim(s);
        if t.is_empty() {
            ScriptError::new("java.lang.NumberFormatException: empty String")
        } else {
            ScriptError::new(format!("java.lang.NumberFormatException: For input string: \"{t}\""))
        }
    })
}

/// `Integer.parseInt`, with its `NumberFormatException` text.
pub(crate) fn parse_int(s: &str) -> ScriptResult<i32> {
    java::parse_i32(s)
        .ok_or_else(|| ScriptError::new(format!("java.lang.NumberFormatException: For input string: \"{s}\"")))
}

/// `PZMath.tryParseInt`.
pub(crate) fn try_parse_int(s: &str, default: i32) -> i32 {
    if java::is_null_or_whitespace(Some(s)) {
        return default;
    }
    java::parse_i32(java::trim(s)).unwrap_or(default)
}

/// `String.indexOf(String)` (-1 when absent).
pub(crate) fn index_of(s: &str, pat: &str) -> i64 {
    java::index_of(s, pat, 0)
}

/// `String.substring(begin, end)` with Java's bounds check.
pub(crate) fn substring(s: &str, begin: i64, end: i64) -> ScriptResult<&str> {
    if begin < 0 || end > s.len() as i64 || begin > end {
        return Err(ScriptError::new(format!(
            "java.lang.StringIndexOutOfBoundsException: begin {begin}, end {end}, length {}",
            s.len()
        )));
    }
    Ok(&s[begin as usize..end as usize])
}

/// `String.substring(begin)`.
pub(crate) fn substring_from(s: &str, begin: i64) -> ScriptResult<&str> {
    substring(s, begin, s.len() as i64)
}

/// `s.substring(s.indexOf("[") + 1, s.indexOf("]"))`.
pub(crate) fn bracket_contents(s: &str) -> ScriptResult<&str> {
    substring(s, index_of(s, "[") + 1, index_of(s, "]"))
}

/// `StringUtils.containsWhitespace`: `s.matches("(.*?)\\s(.*?)")`, where
/// `.` stops at line terminators, so at most the matched `\s` may be one.
pub(crate) fn contains_whitespace(s: &str) -> bool {
    let is_terminator = |c: char| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}');
    match s.chars().filter(|&c| is_terminator(c)).count() {
        0 => s.chars().any(java::is_regex_space),
        1 => s.contains(['\n', '\r']),
        _ => false,
    }
}

/// `StringUtils.removeWhitespace`: `replaceAll("\\s", "")`.
pub(crate) fn remove_whitespace(s: &str) -> String {
    s.chars().filter(|&c| !java::is_regex_space(c)).collect()
}

/// `PZMath.normalize(float[])`: divides by the sum (summed back to front)
/// unless it is zero.
pub(crate) fn normalize(weights: &mut [f32]) {
    let mut total = 0.0f32;
    for w in weights.iter().rev() {
        total += *w;
    }
    if total != 0.0 {
        for w in weights.iter_mut().rev() {
            *w /= total;
        }
    }
}

/// `DebugType.General.error`.
pub(crate) fn log_error(sm: &ScriptManager, msg: &str) {
    sm.script_error(msg);
}

/// `CraftRecipeManager.FormatAndRegisterRecipeTagsQuery`, formatting part
/// (`TaggedObjectManager.formatQueryString`): whitelist and blacklist tags
/// trimmed, lower-cased and sorted, joined by `;` with a `-` before the
/// blacklist. Registering the query with the tag manager belongs to the
/// recipe manager. TODO(CraftRecipeManager)
pub fn format_recipe_tag_query(query: &str) -> ScriptResult<String> {
    if java::is_null_or_whitespace(Some(query)) {
        return Err(illegal_argument("Key is null or whitespace."));
    }
    let (white, black): (Option<Vec<&str>>, Option<Vec<&str>>) = if !query.contains('-') {
        (Some(java::split_char(query, ';')), None)
    } else if let Some(rest) = query.strip_prefix('-') {
        (None, Some(java::split_char(rest, ';')))
    } else {
        let sp = java::split_char(query, '-');
        (Some(java::split_char(array_get(&sp, 0)?, ';')), Some(java::split_char(array_get(&sp, 1)?, ';')))
    };
    let join = |tags: Vec<&str>| {
        let mut list: Vec<String> = tags.iter().map(|t| java::to_lower(java::trim(t))).collect();
        list.sort_by(|a, b| java::compare(a, b));
        list.join(";")
    };
    let mut sb = String::new();
    if let Some(w) = white {
        sb.push_str(&join(w));
    }
    if let Some(b) = black {
        sb.push('-');
        sb.push_str(&join(b));
    }
    Ok(sb)
}

/// Ids of the perks `PerkFactory.Perks` creates statically, which is what
/// `Perks.FromString` finds at script load. Perks added later from Lua are
/// not known here. TODO(PerkFactory)
const PERK_IDS: &[&str] = &[
    "None",
    "Agility",
    "Cooking",
    "Melee",
    "Crafting",
    "Fitness",
    "Strength",
    "Blunt",
    "Axe",
    "Lightfoot",
    "Nimble",
    "Sprinting",
    "Sneak",
    "Woodwork",
    "Aiming",
    "Reloading",
    "Farming",
    "Survivalist",
    "Fishing",
    "Trapping",
    "Passiv",
    "Firearm",
    "PlantScavenging",
    "Doctor",
    "Electricity",
    "Blacksmith",
    "MetalWelding",
    "Melting",
    "Mechanics",
    "Spear",
    "Maintenance",
    "SmallBlade",
    "LongBlade",
    "SmallBlunt",
    "Combat",
    "Tailoring",
    "Tracking",
    "Husbandry",
    "FlintKnapping",
    "Masonry",
    "Pottery",
    "Carving",
    "Butchering",
    "Glassmaking",
    "FarmingCategory",
    "PhysicalCategory",
    "MAX",
];

/// `PerkFactory.Perks.FromString`: the perk id, or `"MAX"` when unknown.
pub(crate) fn perk_from_string(id: &str) -> &'static str {
    PERK_IDS.iter().copied().find(|p| *p == id).unwrap_or("MAX")
}

#[cfg(test)]
mod tests;
