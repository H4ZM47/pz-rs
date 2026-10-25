//! The tile properties in `tileGeometry.txt` (`TileGeometryFile`), which
//! `TileGeometryManager.initSpriteProperties` copies onto sprites:
//! `ItemHeight`, `Surface`, `CurtainOffset` and the depth flags
//! (`OpaquePixelsOnly` = 4, `Translucent` = 2, `UseObjectDepthTexture` = 1).
//! The geometry itself is only used by the editor and is not read.

use std::collections::HashMap;
use std::path::Path;

use pz_fs::script_parser;

/// `IsoSprite.depthFlags` bits.
pub mod depth_flags {
    pub const USE_OBJECT_DEPTH_TEXTURE: i32 = 1;
    pub const TRANSLUCENT: i32 = 2;
    pub const OPAQUE_PIXELS_ONLY: i32 = 4;
}

/// `StringUtils.tryParseBoolean`.
pub fn try_parse_boolean(s: &str) -> bool {
    let s = s.trim();
    s.eq_ignore_ascii_case("true") || s == "1" || s == "1.0"
}

/// The tile properties of one `tileGeometry.txt`, by sprite name.
#[derive(Clone, Debug, Default)]
pub struct TileGeometryProperties {
    pub tiles: HashMap<String, HashMap<String, String>>,
}

impl TileGeometryProperties {
    /// `TileGeometryFile.parseFile`: every tile with a non-empty
    /// `properties` block.
    pub fn parse(text: &str) -> Self {
        let mut out = TileGeometryProperties::default();
        let root = script_parser::parse(&script_parser::strip_comments(text));
        let Some(file) = root.children.first() else { return out };
        for tileset in file.children.iter().filter(|b| b.kind == "tileset") {
            let Some(name) = tileset.value("name").map(|v| v.value().trim().to_string()) else { continue };
            for tile in tileset.children.iter().filter(|b| b.kind == "tile") {
                let Some((col, row)) = tile.value("xy").and_then(|v| {
                    let (c, r) = v.value().trim().split_once('x')?;
                    Some((c.trim().parse::<i32>().ok()?, r.trim().parse::<i32>().ok()?))
                }) else {
                    continue;
                };
                for props in tile.children.iter().filter(|b| b.kind == "properties") {
                    let map: HashMap<String, String> =
                        props.values.iter().filter(|v| !v.key().trim().is_empty()).map(|v| (v.key().trim().to_string(), v.value().trim().to_string())).collect();
                    if !map.is_empty() {
                        out.tiles.insert(format!("{name}_{}", col + row * 8), map);
                    }
                }
            }
        }
        out
    }

    /// Reads `<media>/tileGeometry.txt`.
    pub fn read(media: &Path) -> Option<Self> {
        std::fs::read_to_string(media.join("tileGeometry.txt")).ok().map(|t| Self::parse(&t))
    }

    /// The depth flags this file gives `sprite`.
    pub fn depth_flags(&self, sprite: &str) -> i32 {
        let Some(p) = self.tiles.get(sprite) else { return 0 };
        let flag = |k: &str, bit: i32| if p.get(k).is_some_and(|v| try_parse_boolean(v)) { bit } else { 0 };
        flag("OpaquePixelsOnly", depth_flags::OPAQUE_PIXELS_ONLY) | flag("Translucent", depth_flags::TRANSLUCENT) | flag("UseObjectDepthTexture", depth_flags::USE_OBJECT_DEPTH_TEXTURE)
    }
}

/// `TileGeometryManager`: the game's file, then each mod's, in that order.
#[derive(Clone, Debug, Default)]
pub struct TileGeometryManager {
    pub files: Vec<TileGeometryProperties>,
}

impl TileGeometryManager {
    pub fn from_media_dirs<'a>(dirs: impl IntoIterator<Item = &'a Path>) -> Self {
        TileGeometryManager { files: dirs.into_iter().filter_map(TileGeometryProperties::read).collect() }
    }

    /// `IsoSprite.depthFlags` after `initSpriteProperties`: the flags of
    /// every file, combined.
    pub fn depth_flags(&self, sprite: &str) -> i32 {
        self.files.iter().fold(0, |f, file| f | file.depth_flags(sprite))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tile_properties() {
        let text = "tileGeometry\n{\n VERSION = 2,\n tileset\n {\n  name = sheet_01,\n  /* sheet_01_9 */\n  tile\n  {\n   xy = 1x1,\n   box\n   {\n    min = 0x0x0,\n   }\n   properties\n   {\n    Surface = 29,\n    Translucent = true,\n    UseObjectDepthTexture = 1,\n   }\n  }\n  tile\n  {\n   xy = 2x0,\n  }\n }\n}\n";
        let p = TileGeometryProperties::parse(text);
        assert_eq!(p.tiles["sheet_01_9"]["Surface"], "29");
        assert_eq!(p.depth_flags("sheet_01_9"), 3);
        assert_eq!(p.depth_flags("sheet_01_2"), 0);
    }
}
