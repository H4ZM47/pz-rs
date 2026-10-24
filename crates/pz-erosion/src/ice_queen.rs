//! `zombie.erosion.season.ErosionIceQueen`: the winter (snow) variant of
//! sprites, from the `SnowTile` tile property and the categories'
//! registrations.

use std::collections::HashMap;

use pz_world::sprite::SpriteIdx;
use pz_world::IsoSpriteManager;

/// `IsoSpriteManager.getSprite(name)`: the named sprite, created (as
/// `AddSprite(name)` does) when missing.
pub fn get_sprite(m: &mut IsoSpriteManager, name: &str) -> SpriteIdx {
    match m.named_map.get(name) {
        Some(&s) => s,
        None => {
            let s = m.add_sprite_named(name);
            m.sprites[s].name = Some(name.to_string());
            s
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    sprite: SpriteIdx,
    winter: String,
}

/// `ErosionIceQueen`.
#[derive(Clone, Debug, Default)]
pub struct ErosionIceQueen {
    sprites: Vec<Entry>,
    snow_state: bool,
    /// `IsoSprite.snowSprite` of the registered sprites, as `setSnow` sets it.
    pub snow_sprite: HashMap<SpriteIdx, Option<SpriteIdx>>,
}

fn is_java_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{1C}'..='\u{1F}' | ' ')
}

/// `fixTileName`: normalizes the number after the last `_`.
fn fix_tile_name(name: &str) -> String {
    let Some(p) = name.rfind('_') else {
        return name.to_string();
    };
    let t = crate::java::trim(&name[p + 1..]);
    let index = if t.is_empty() { -1 } else { t.parse::<i32>().unwrap_or(-1) };
    if index == -1 {
        return name.to_string();
    }
    format!("{}_{index}", &name[..p])
}

impl ErosionIceQueen {
    /// `new ErosionIceQueen(manager)`: registers every sprite with a
    /// `SnowTile` property (`readTileProperties`).
    pub fn new(m: &mut IsoSpriteManager) -> Self {
        let mut q = ErosionIceQueen::default();
        let list: Vec<(String, String)> = m
            .named_map_java_order()
            .into_iter()
            .filter_map(|(_, s)| {
                let spr = &m.sprites[s];
                let snow = spr.properties.get(&m.alias, "SnowTile")?;
                if snow.chars().all(is_java_whitespace) {
                    return None;
                }
                Some((spr.name.clone().unwrap_or_default(), fix_tile_name(snow)))
            })
            .collect();
        for (name, snow) in list {
            q.add_sprite(m, &name, &snow);
        }
        q
    }

    /// `addSprite(sprite, winterSprite)`.
    pub fn add_sprite(&mut self, m: &mut IsoSpriteManager, sprite: &str, winter: &str) {
        let spr = get_sprite(m, sprite);
        get_sprite(m, winter);
        m.sprites[spr].name = Some(sprite.to_string());
        self.sprites.push(Entry { sprite: spr, winter: winter.to_string() });
    }

    /// `setSnow(isSnow)`. Like the original, a change in either direction
    /// points every registered sprite at its winter sprite.
    pub fn set_snow(&mut self, m: &IsoSpriteManager, is_snow: bool) {
        if self.snow_state != is_snow {
            self.snow_state = is_snow;
            for e in &self.sprites {
                self.snow_sprite.insert(e.sprite, m.named_map.get(&e.winter).copied());
            }
        }
    }

    pub fn is_snow(&self) -> bool {
        self.snow_state
    }

    /// Number of registered sprites.
    pub fn len(&self) -> usize {
        self.sprites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sprites.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_names() {
        assert_eq!(fix_tile_name("a_b_ 07"), "a_b_7");
        assert_eq!(fix_tile_name("plain"), "plain");
        assert_eq!(fix_tile_name("a_x"), "a_x");
    }
}
