//! The tile-definition side of sprites: `IsoSprite`, `IsoSpriteManager`,
//! `PropertyContainer`, `TilePropertyAliasMap` and `IsoWorld`'s tile
//! definition loading.
//!
//! Enum constant tables (`IsoFlagType`, `IsoObjectType`, `FasciaEdge`) are
//! runtime data ([`SpriteTables`]) read from the installed game. Texture,
//! animation and rendering state are not ported.

mod loader;
mod manager;
mod properties;
mod types;

pub use loader::{load_property_strings, set_custom_property_values};
pub use manager::{
    GridIdx, IsoSprite, IsoSpriteGrid, IsoSpriteManager, SpriteIdx, DEFAULT_SPRITE_ID,
};
pub use properties::{
    PropertyContainer, PropertyValueMap, SurfaceInfo, TileProperty, TilePropertyAliasMap,
};
#[cfg(all(test, not(feature = "test-util")))]
pub(crate) use types::test_tables;
pub use types::{FasciaEdge, Flags, IsoFlagType, IsoObjectType, ObjectTypes, SpriteTables};
#[cfg(feature = "test-util")]
pub use types::test_tables;

/// `Character.isWhitespace` for Latin-1 chars.
fn is_java_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{1C}'..='\u{1F}' | ' '
    )
}

/// `StringUtils.isNullOrWhitespace`.
pub(crate) fn is_null_or_whitespace(s: Option<&str>) -> bool {
    match s {
        None => true,
        Some(s) => s.is_empty() || s.chars().all(is_java_whitespace),
    }
}

/// `Integer.parseInt`; failure is a `NumberFormatException`.
pub(crate) fn parse_int(s: &str) -> std::io::Result<i32> {
    s.parse::<i32>().map_err(|_| {
        std::io::Error::other(format!("NumberFormatException: For input string: \"{s}\""))
    })
}

/// `PZMath.tryParseInt`.
pub(crate) fn try_parse_int(s: &str, default: i32) -> i32 {
    if is_null_or_whitespace(Some(s)) {
        return default;
    }
    crate::java::java_trim(s).parse().unwrap_or(default)
}

/// `StringUtils.equalsIgnoreCase` (`String.equalsIgnoreCase`) for Latin-1.
pub(crate) fn equals_ignore_case(a: &str, b: &str) -> bool {
    a.chars().count() == b.chars().count()
        && a.chars().zip(b.chars()).all(|(x, y)| {
            x == y || x.to_uppercase().eq(y.to_uppercase()) || x.to_lowercase().eq(y.to_lowercase())
        })
}

/// `String.split(String)` with a single-char separator: trailing empty
/// strings are removed.
pub(crate) fn java_split(s: &str, sep: char) -> Vec<&str> {
    let mut v: Vec<&str> = s.split(sep).collect();
    if v.len() == 1 {
        return v;
    }
    while v.last() == Some(&"") {
        v.pop();
    }
    v
}

#[cfg(test)]
mod tests;
