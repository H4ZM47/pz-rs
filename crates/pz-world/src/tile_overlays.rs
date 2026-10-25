//! `zombie.iso.TileOverlays` and `zombie.iso.ContainerOverlays`: the
//! overlay sprites `media/lua/server/Items/*Overlays.lua` (tiles) and
//! `WorldFiller.lua` (container contents) register with `addOverlays`, and
//! the per-square choice of an overlay (`LocationRNG` seeded by the square's
//! position).
//!
//! The registries hold what the Lua tables say; reading the tables is the
//! Lua binding's job, which hands each overlay over as an
//! [`OverlayDef`] in table-iteration order. Choosing an overlay takes the
//! square's position and room name and the texture lookup as inputs, so
//! the object that gets the sprite stays with the caller.

use pz_core::random::location_rng;
use pz_kahlua::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// The actual mutable `ArrayList` allocation returned to Lua. Its
/// elements are erased Java values: Lua may add a number or null as well
/// as a sprite name. Clearing the registry drops its reference to a list;
/// callers retaining that list continue to own its original contents.
pub type UnderlyingSpriteNames = Rc<RefCell<Vec<Value>>>;

fn add_underlying_name(list: &UnderlyingSpriteNames, name: &str) {
    let name = Value::str(name);
    let mut list = list.borrow_mut();
    if !list.iter().any(|v| v.raw_equals(&name)) {
        list.push(name);
    }
}

fn string_names(list: &UnderlyingSpriteNames) -> Vec<String> {
    list.borrow()
        .iter()
        .filter_map(|v| match v {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        })
        .collect()
}

/// One room entry of an overlay table: `{ name = .., chance = ..,
/// usage = .., tiles = {..} }` (tiles) or `{ name = .., tiles = {many, few} }`
/// (containers).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayEntryDef {
    /// `rawgetStr("name")`.
    pub room: Option<String>,
    /// `rawgetInt("chance")` (-1 when missing).
    pub chance: i32,
    /// `rawgetStr("usage")`.
    pub usage: Option<String>,
    /// The `tiles` values' `toString()`, in iteration order.
    pub tiles: Vec<String>,
}

/// One key of an overlay table (the underlying sprite) and its entries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayDef {
    pub tile: String,
    pub entries: Vec<OverlayEntryDef>,
}

/// Why `addOverlays` threw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlayError {
    /// A `null` dereference (a missing `usage` string).
    NullPointer,
    /// `Integer.parseInt` / `Float.parseFloat` on a usage value.
    NumberFormat(String),
    /// `ContainerOverlays.addOverlays` with a version other than -1 or 1.
    UnknownVersion(i32),
}

impl std::fmt::Display for OverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OverlayError::NullPointer => f.write_str("java.lang.NullPointerException"),
            OverlayError::NumberFormat(s) => write!(f, "For input string: \"{s}\""),
            OverlayError::UnknownVersion(v) => write!(f, "unknown overlayMap.VERSION {v}"),
        }
    }
}

impl std::error::Error for OverlayError {}

/// `TileOverlayUsage`.
#[derive(Clone, Debug, PartialEq)]
pub struct TileOverlayUsage {
    pub usage: String,
    pub z_only: i32,
    pub z_greater_than: i32,
    pub alpha: f32,
    pub table_top: bool,
}

impl Default for TileOverlayUsage {
    fn default() -> Self {
        TileOverlayUsage {
            usage: String::new(),
            z_only: i32::MIN,
            z_greater_than: i32::MIN,
            alpha: -1.0,
            table_top: false,
        }
    }
}

/// `StringUtils.isNullOrWhitespace`.
fn null_or_whitespace(s: Option<&str>) -> bool {
    s.is_none_or(|s| s.chars().all(char::is_whitespace))
}

/// Java's `String.trim()`: strips code points <= ' ' at both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `Integer.parseInt`.
fn parse_int(s: &str) -> Result<i32, OverlayError> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(OverlayError::NumberFormat(s.to_string()));
    }
    s.parse::<i32>()
        .map_err(|_| OverlayError::NumberFormat(s.to_string()))
}

/// `Float.parseFloat` (leading/trailing whitespace allowed, as in Java).
fn parse_float(s: &str) -> Result<f32, OverlayError> {
    let t = java_trim(s);
    let t = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    t.parse::<f32>()
        .map_err(|_| OverlayError::NumberFormat(s.to_string()))
}

impl TileOverlayUsage {
    /// `parse(usage)`: `false` on an unknown token (the fields parsed
    /// before it are kept).
    pub fn parse(&mut self, usage: &str) -> Result<bool, OverlayError> {
        self.usage = java_trim(usage).to_string();
        if null_or_whitespace(Some(&self.usage)) {
            return Ok(true);
        }
        // `usage.split(";")`: trailing empty strings are dropped.
        let mut parts: Vec<&str> = usage.split(';').collect();
        while parts.len() > 1 && parts.last() == Some(&"") {
            parts.pop();
        }
        for s in parts {
            if let Some(v) = s.strip_prefix("z=") {
                self.z_only = parse_int(v)?;
            } else if let Some(v) = s.strip_prefix("z>") {
                self.z_greater_than = parse_int(v)?;
            } else if let Some(v) = s.strip_prefix("alpha=") {
                self.alpha = parse_float(v)?.clamp(0.0, 1.0);
            } else if s.starts_with("tabletop") {
                self.table_top = true;
            } else {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `match(square)`.
    pub fn matches(&self, z: i32) -> bool {
        if self.z_only != i32::MIN && z != self.z_only {
            return false;
        }
        self.z_greater_than == i32::MIN || z > self.z_greater_than
    }
}

/// `TileOverlayEntry`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileOverlayEntry {
    pub room: String,
    pub chance: i32,
    pub tiles: Vec<String>,
    pub usage: TileOverlayUsage,
}

impl TileOverlayEntry {
    /// `pickRandom(x, y, z)`: one of the tiles with chance `1/chance`.
    pub fn pick_random(&self, x: i32, y: i32, z: i32) -> Option<&str> {
        location_rng::with_instance(|r| {
            let n = r.next_int_at(self.chance, x, y, z);
            if n == 0 && !self.tiles.is_empty() {
                let n = r.next_int(self.tiles.len() as i32);
                return Some(self.tiles[n as usize].as_str());
            }
            None
        })
    }
}

/// `TileOverlay`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileOverlay {
    pub tile: String,
    pub entries: Vec<TileOverlayEntry>,
}

impl TileOverlay {
    /// `pickRandom(room, square)`: one of the entries for `room` whose usage
    /// matches the square's level.
    pub fn pick_random(&self, room: &str, x: i32, y: i32, z: i32) -> Option<&TileOverlayEntry> {
        let entries: Vec<&TileOverlayEntry> = self
            .entries
            .iter()
            .filter(|e| e.room.eq_ignore_ascii_case(room) && e.usage.matches(z))
            .collect();
        if entries.is_empty() {
            return None;
        }
        let n = location_rng::with_instance(|r| r.next_int_at(entries.len() as i32, x, y, z));
        Some(entries[n as usize])
    }
}

/// The overlay `updateTileOverlaySprite` puts first in the object's
/// `attachedAnimSprite`.
#[derive(Clone, Debug, PartialEq)]
pub struct TileOverlayPick {
    pub sprite: String,
    /// `(r, g, b, alpha)` set on the sprite instance when the usage has an
    /// `alpha=` above 0.
    pub tint: Option<[f32; 4]>,
}

/// The square an overlay is chosen for.
#[derive(Clone, Copy, Debug)]
pub struct OverlaySquare<'a> {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// `square.getRoom().getName()`.
    pub room: Option<&'a str>,
}

/// `TileOverlays.instance`.
#[derive(Clone, Debug, Default)]
pub struct TileOverlays {
    pub overlay_map: HashMap<String, TileOverlay>,
    /// `overlayNameToUnderlyingName`: each overlay sprite's underlying
    /// sprites, in registration order.
    pub overlay_name_to_underlying_name: HashMap<String, UnderlyingSpriteNames>,
}

impl TileOverlays {
    /// `addOverlays(overlayMap)` for the table's non-`VERSION` keys.
    pub fn add_overlays(&mut self, defs: &[OverlayDef]) -> Result<(), OverlayError> {
        for d in defs {
            if d.tile.eq_ignore_ascii_case("VERSION") {
                continue;
            }
            let mut o = TileOverlay {
                tile: d.tile.clone(),
                entries: Vec::new(),
            };
            for ed in &d.entries {
                let mut e = TileOverlayEntry {
                    room: ed.room.clone().unwrap_or_default(),
                    chance: ed.chance,
                    ..Default::default()
                };
                let usage = ed.usage.as_deref().ok_or(OverlayError::NullPointer)?;
                e.usage.parse(usage)?;
                for t in &ed.tiles {
                    // TODO(port): under `Core.debug` the original prints
                    // "BLANK OVERLAY TEXTURE" for a missing texture.
                    if null_or_whitespace(Some(t)) || t.eq_ignore_ascii_case("none") {
                        e.tiles.push(String::new());
                    } else {
                        e.tiles.push(t.clone());
                    }
                }
                o.entries.push(e);
            }
            let tile = o.tile.clone();
            let names: Vec<String> = o
                .entries
                .iter()
                .flat_map(|e| e.tiles.iter().cloned())
                .collect();
            self.overlay_map.insert(tile.clone(), o);
            for name in names {
                let list = self
                    .overlay_name_to_underlying_name
                    .entry(name)
                    .or_default();
                add_underlying_name(list, &tile);
            }
        }
        Ok(())
    }

    /// `hasOverlays(obj)` for the object's sprite name.
    pub fn has_overlays(&self, sprite_name: Option<&str>) -> bool {
        sprite_name.is_some_and(|n| self.overlay_map.contains_key(n))
    }

    /// `getUnderlyingSpriteNames(overlayName)`.
    pub fn get_underlying_sprite_names(&self, overlay_name: &str) -> Option<Vec<String>> {
        self.overlay_name_to_underlying_name
            .get(overlay_name)
            .map(string_names)
    }

    /// The backing collection, preserving its identity and all element
    /// types for the Java-facing getter. The string getter above is a
    /// snapshot for Rust consumers that need sprite names.
    pub fn underlying_sprite_names(&self, overlay_name: &str) -> Option<UnderlyingSpriteNames> {
        self.overlay_name_to_underlying_name
            .get(overlay_name)
            .cloned()
    }

    /// `updateTileOverlaySprite(obj)`'s choice: the overlay sprite for an
    /// object with sprite `sprite_name` on `sq`. `has_object_on_top` is
    /// `hasObjectOnTop(obj)` (asked only for a `tabletop` usage);
    /// `texture_exists` is `Texture.getSharedTexture(name) != null` (not
    /// asked on a server).
    pub fn pick_overlay(
        &self,
        sprite_name: Option<&str>,
        sq: OverlaySquare<'_>,
        server: bool,
        has_object_on_top: impl FnOnce() -> bool,
        texture_exists: impl FnOnce(&str) -> bool,
    ) -> Option<TileOverlayPick> {
        let overlay = self.overlay_map.get(sprite_name?)?;
        let room = sq.room.unwrap_or("other");
        let entry = overlay
            .pick_random(room, sq.x, sq.y, sq.z)
            .or_else(|| overlay.pick_random("other", sq.x, sq.y, sq.z))?;
        if entry.usage.table_top && has_object_on_top() {
            return None;
        }
        let name = entry.pick_random(
            sq.x.wrapping_abs(),
            sq.y.wrapping_abs(),
            sq.z.wrapping_abs(),
        );
        let tint = (entry.usage.alpha >= 0.0).then_some([1.0, 1.0, 1.0, entry.usage.alpha]);
        let name = name.filter(|n| !null_or_whitespace(Some(n)))?;
        if !server && !texture_exists(name) {
            return None;
        }
        Some(TileOverlayPick {
            sprite: name.to_string(),
            tint: tint.filter(|t| t[3] > 0.0),
        })
    }

    /// `removeTableTopOverlays(obj)`: the sprite names of the `tabletop`
    /// entries of the object's overlay, whose instances are removed from
    /// `attachedAnimSprite`.
    pub fn table_top_overlay_names(&self, sprite_name: &str) -> Vec<&str> {
        let Some(o) = self.overlay_map.get(sprite_name) else {
            return Vec::new();
        };
        o.entries
            .iter()
            .filter(|e| e.usage.table_top)
            .flat_map(|e| e.tiles.iter().map(String::as_str))
            .collect()
    }

    /// `Reset()`.
    pub fn reset(&mut self) {
        self.overlay_map.clear();
        self.overlay_name_to_underlying_name.clear();
    }
}

/// `ContainerOverlayEntry`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainerOverlayEntry {
    pub room: String,
    pub many_items: Option<String>,
    pub few_items: Option<String>,
}

/// `ContainerOverlay`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainerOverlay {
    pub name: String,
    pub entries: Vec<ContainerOverlayEntry>,
}

impl ContainerOverlay {
    /// `pickRandom(room, x, y, z)`.
    pub fn pick_random(
        &self,
        room: &str,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<&ContainerOverlayEntry> {
        let entries: Vec<&ContainerOverlayEntry> = self
            .entries
            .iter()
            .filter(|e| e.room.eq_ignore_ascii_case(room))
            .collect();
        if entries.is_empty() {
            return None;
        }
        let n = location_rng::with_instance(|r| r.next_int_at(entries.len() as i32, x, y, z));
        Some(entries[n as usize])
    }
}

/// A version-0 room entry: the room key and `tiles[1]` / `tiles[2]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainerOverlayV0Entry {
    pub room: String,
    pub many_items: Option<String>,
    pub few_items: Option<String>,
}

/// `ContainerOverlays.instance`.
#[derive(Clone, Debug, Default)]
pub struct ContainerOverlays {
    pub overlay_map: HashMap<String, ContainerOverlay>,
    pub overlay_name_to_underlying_name: HashMap<String, UnderlyingSpriteNames>,
}

impl ContainerOverlays {
    /// `parseContainerOverlayMapV0`: keys and rooms in the table's hash
    /// order; the underlying-name map is not filled.
    pub fn add_overlays_v0(&mut self, defs: &[(String, Vec<ContainerOverlayV0Entry>)]) {
        for (key, entries) in defs {
            let o = ContainerOverlay {
                name: key.clone(),
                entries: entries
                    .iter()
                    .map(|e| ContainerOverlayEntry {
                        room: e.room.clone(),
                        many_items: e.many_items.clone(),
                        few_items: e.few_items.clone(),
                    })
                    .collect(),
            };
            self.overlay_map.insert(key.clone(), o);
        }
    }

    /// `parseContainerOverlayMapV1`: each entry's `tiles` are
    /// `{manyItems, fewItems}` (`OverlayEntryDef::tiles[0..2]`, `None` for a
    /// missing index; "none" or blank is no sprite).
    pub fn add_overlays_v1(
        &mut self,
        defs: &[(
            String,
            Vec<(Option<String>, Option<String>, Option<String>)>,
        )],
    ) {
        for (key, entries) in defs {
            if key.eq_ignore_ascii_case("VERSION") {
                continue;
            }
            let fix = |s: &Option<String>| {
                s.clone()
                    .filter(|s| !null_or_whitespace(Some(s)) && !s.eq_ignore_ascii_case("none"))
            };
            let o = ContainerOverlay {
                name: key.clone(),
                entries: entries
                    .iter()
                    .map(|(room, many, few)| ContainerOverlayEntry {
                        room: room.clone().unwrap_or_default(),
                        many_items: fix(many),
                        few_items: fix(few),
                    })
                    .collect(),
            };
            let names: Vec<String> = o
                .entries
                .iter()
                .flat_map(|e| [e.few_items.clone(), e.many_items.clone()])
                .flatten()
                .collect();
            self.overlay_map.insert(key.clone(), o);
            for name in names {
                let list = self
                    .overlay_name_to_underlying_name
                    .entry(name)
                    .or_default();
                add_underlying_name(list, key);
            }
        }
    }

    /// `hasOverlays(obj)`.
    pub fn has_overlays(&self, sprite_name: Option<&str>) -> bool {
        sprite_name.is_some_and(|n| self.overlay_map.contains_key(n))
    }

    /// `getUnderlyingSpriteNames(overlayName)`.
    pub fn get_underlying_sprite_names(&self, overlay_name: &str) -> Option<Vec<String>> {
        self.overlay_name_to_underlying_name
            .get(overlay_name)
            .map(string_names)
    }

    pub fn underlying_sprite_names(&self, overlay_name: &str) -> Option<UnderlyingSpriteNames> {
        self.overlay_name_to_underlying_name
            .get(overlay_name)
            .cloned()
    }

    /// `updateContainerOverlaySprite(obj)`'s choice for an object (not a
    /// stove or feeding trough) with sprite `sprite_name` and a container
    /// holding `item_count` items (`None`: no container or no item list):
    /// the name given to `setOverlaySprite` (`None` clears it).
    pub fn pick_overlay(
        &self,
        sprite_name: Option<&str>,
        item_count: Option<usize>,
        sq: OverlaySquare<'_>,
        server: bool,
        texture_exists: impl FnOnce(&str) -> bool,
    ) -> Option<String> {
        let count = item_count.filter(|&n| n > 0)?;
        let overlay = self.overlay_map.get(sprite_name?)?;
        let room = sq.room.unwrap_or("other");
        let entry = overlay
            .pick_random(room, sq.x, sq.y, sq.z)
            .or_else(|| overlay.pick_random("other", sq.x, sq.y, sq.z))?;
        let mut name = entry.many_items.clone();
        if entry.few_items.is_some() && count < 7 {
            name = entry.few_items.clone();
        }
        let name = name?;
        if !null_or_whitespace(Some(&name)) && !server && !texture_exists(&name) {
            return None;
        }
        Some(name)
    }

    /// `Reset()`.
    pub fn reset(&mut self) {
        self.overlay_map.clear();
        self.overlay_name_to_underlying_name.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(room: &str, chance: i32, usage: &str, tiles: &[&str]) -> OverlayEntryDef {
        OverlayEntryDef {
            room: Some(room.into()),
            chance,
            usage: Some(usage.into()),
            tiles: tiles.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn usage_parse() {
        let mut u = TileOverlayUsage::default();
        assert_eq!(u.parse("z=1;alpha=1.5;tabletop"), Ok(true));
        assert_eq!((u.z_only, u.alpha, u.table_top), (1, 1.0, true));
        assert!(u.matches(1) && !u.matches(0));
        let mut u = TileOverlayUsage::default();
        assert_eq!(u.parse("z>0;bogus"), Ok(false));
        assert_eq!(u.z_greater_than, 0);
        assert!(!u.matches(0) && u.matches(2));
        let mut u = TileOverlayUsage::default();
        assert!(matches!(u.parse("z=x"), Err(OverlayError::NumberFormat(_))));
        assert_eq!(TileOverlayUsage::default().parse("   "), Ok(true));
    }

    #[test]
    fn add_and_pick() {
        let mut t = TileOverlays::default();
        t.add_overlays(&[
            OverlayDef {
                tile: "VERSION".into(),
                entries: vec![],
            },
            OverlayDef {
                tile: "base_0".into(),
                entries: vec![entry("other", 1, "", &["ov_0", "none", "ov_1"])],
            },
            OverlayDef {
                tile: "base_1".into(),
                entries: vec![entry("kitchen", 1, "alpha=0.5", &["ov_0"])],
            },
        ])
        .unwrap();
        assert!(!t.overlay_map.contains_key("VERSION"));
        assert_eq!(
            t.overlay_map["base_0"].entries[0].tiles,
            vec!["ov_0", "", "ov_1"]
        );
        assert_eq!(
            t.get_underlying_sprite_names("ov_0").unwrap(),
            ["base_0", "base_1"]
        );
        assert_eq!(t.get_underlying_sprite_names("").unwrap(), ["base_0"]);
        // chance 1: nextInt(1) is always 0, so a tile is always picked.
        let sq = OverlaySquare {
            x: 10,
            y: 20,
            z: 0,
            room: Some("Kitchen"),
        };
        let p = t
            .pick_overlay(Some("base_1"), sq, false, || false, |_| true)
            .unwrap();
        assert_eq!(p.sprite, "ov_0");
        assert_eq!(p.tint, Some([1.0, 1.0, 1.0, 0.5]));
        // No texture: nothing.
        assert!(t
            .pick_overlay(Some("base_1"), sq, false, || false, |_| false)
            .is_none());
        // The room has no entry: falls back to "other".
        assert!(t
            .pick_overlay(
                Some("base_1"),
                OverlaySquare { room: None, ..sq },
                true,
                || false,
                |_| true
            )
            .is_none());
        let missing = OverlayDef {
            tile: "x".into(),
            entries: vec![OverlayEntryDef {
                usage: None,
                ..entry("other", 1, "", &[])
            }],
        };
        assert_eq!(t.add_overlays(&[missing]), Err(OverlayError::NullPointer));
        t.reset();
        assert!(t.overlay_map.is_empty());
    }

    #[test]
    fn container_overlays() {
        let mut c = ContainerOverlays::default();
        c.add_overlays_v1(&[(
            "fridge_0".into(),
            vec![
                (
                    Some("other".into()),
                    Some("food_0".into()),
                    Some("food_1".into()),
                ),
                (Some("other".into()), Some("none".into()), None),
            ],
        )]);
        let o = &c.overlay_map["fridge_0"];
        assert_eq!(o.entries[1].many_items, None);
        assert_eq!(
            c.get_underlying_sprite_names("food_1").unwrap(),
            ["fridge_0"]
        );
        let sq = OverlaySquare {
            x: 3,
            y: 4,
            z: 0,
            room: None,
        };
        assert_eq!(
            c.pick_overlay(Some("fridge_0"), Some(0), sq, true, |_| true),
            None
        );
        let picked = c.pick_overlay(Some("fridge_0"), Some(10), sq, true, |_| true);
        let n = location_rng::with_instance(|r| r.next_int_at(2, 3, 4, 0));
        assert_eq!(picked, if n == 0 { Some("food_0".into()) } else { None });
    }
}
