//! `zombie.iso.BrokenFences`: the breakable fence sprites
//! `media/lua/server/Items/BrokenFences.lua` registers
//! (`addBrokenTiles`: each unbroken sprite's fully broken, left-broken and
//! right-broken replacements; `addDebrisTiles`: the debris sprite for each
//! side), and the decisions `destroyFence` / `damageAdjacent` /
//! `updateSprite` / `addDebrisObject` / `addItems` make with them.
//!
//! The square and object mutations (replacing the sprite, adding the debris
//! object and items, the recalculations, the sound) are the engine's; these
//! functions return what to do, drawing `Rand` (`RandStandard`) where the
//! original does.

use pz_world::IsoDirections;
use std::collections::HashMap;

/// `RandStandard.INSTANCE` as the fence code draws it.
pub trait FenceRand {
    /// `Rand.Next(max)`.
    fn next(&mut self, max: i32) -> i32;
    /// `Rand.Next(min, max)` for floats.
    fn next_float(&mut self, min: f32, max: f32) -> f32;
}

/// One `BrokenFences.Tile`. A list is `None` when its table is missing or
/// empty (`tableToTiles`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrokenTile {
    pub self_: Option<Vec<String>>,
    pub left: Option<Vec<String>>,
    pub right: Option<Vec<String>>,
    pub debris_n: Option<Vec<String>>,
    pub debris_s: Option<Vec<String>>,
    pub debris_w: Option<Vec<String>>,
    pub debris_e: Option<Vec<String>>,
}

/// `Tile.pickRandom(choices)`: `PZArrayUtil.pickRandom` (`Rand.Next(size)`).
fn pick_random(choices: &Option<Vec<String>>, rand: &mut dyn FenceRand) -> Option<String> {
    let c = choices.as_ref()?;
    if c.is_empty() {
        return None;
    }
    Some(c[rand.next(c.len() as i32) as usize].clone())
}

/// `addDebrisTiles()` with a key `addBrokenTiles` did not register.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownTile;

impl std::fmt::Display for UnknownTile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("addDebrisTiles() with unknown tile")
    }
}

impl std::error::Error for UnknownTile {}

/// What `damageAdjacent` does to the breakable fence next to a destroyed
/// one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdjacentDamage {
    /// Nothing (a north-west corner seen from the south or east).
    None,
    /// `destroyFence(obj, dirBreak)`: it was already broken on the other
    /// side.
    Destroy,
    /// `updateSprite(obj, breakLeft, breakRight)`.
    Damage { left: bool, right: bool },
}

/// One debris item `addItems` drops: the script type and the square
/// offsets.
#[derive(Clone, Debug, PartialEq)]
pub struct DebrisItem {
    pub item_type: &'static str,
    pub x: f32,
    pub y: f32,
}

/// `BrokenFences.getInstance()`.
#[derive(Clone, Debug, Default)]
pub struct BrokenFences {
    /// The `Tile` objects, shared by the maps as in Java.
    pub tiles: Vec<BrokenTile>,
    pub unbroken_map: HashMap<String, usize>,
    pub broken_left_map: HashMap<String, usize>,
    pub broken_right_map: HashMap<String, usize>,
    pub all_map: HashMap<String, usize>,
}

impl BrokenFences {
    /// `addBrokenTiles(tiles)`: one `(key, self, left, right)` per
    /// non-`VERSION` key, in table order.
    pub fn add_broken_tiles(&mut self, defs: Vec<(String, BrokenTile)>) {
        for (key, tile) in defs {
            if key.eq_ignore_ascii_case("VERSION") {
                continue;
            }
            let i = self.tiles.len();
            for s in tile.left.iter().flatten() {
                self.broken_left_map.insert(s.clone(), i);
            }
            for s in tile.right.iter().flatten() {
                self.broken_right_map.insert(s.clone(), i);
            }
            self.tiles.push(BrokenTile {
                debris_n: None,
                debris_s: None,
                debris_w: None,
                debris_e: None,
                ..tile
            });
            self.unbroken_map.insert(key, i);
        }
        for m in [
            &self.unbroken_map,
            &self.broken_left_map,
            &self.broken_right_map,
        ] {
            for (k, &v) in m {
                self.all_map.insert(k.clone(), v);
            }
        }
    }

    /// `addDebrisTiles(tiles)`: `(key, north, south, west, east)`; the
    /// tiles before an unknown key keep their debris.
    #[allow(clippy::type_complexity)]
    pub fn add_debris_tiles(
        &mut self,
        defs: Vec<(String, [Option<Vec<String>>; 4])>,
    ) -> Result<(), UnknownTile> {
        for (key, [n, s, w, e]) in defs {
            if key.eq_ignore_ascii_case("VERSION") {
                continue;
            }
            let &i = self.unbroken_map.get(&key).ok_or(UnknownTile)?;
            let t = &mut self.tiles[i];
            t.debris_n = n;
            t.debris_s = s;
            t.debris_w = w;
            t.debris_e = e;
        }
        Ok(())
    }

    fn tile(&self, sprite_name: &str) -> Option<&BrokenTile> {
        self.all_map.get(sprite_name).map(|&i| &self.tiles[i])
    }

    /// `isUnbroken(obj)` for the object's sprite name.
    pub fn is_unbroken(&self, sprite_name: Option<&str>) -> bool {
        sprite_name.is_some_and(|n| self.unbroken_map.contains_key(n))
    }

    /// `isBrokenLeft(obj)`.
    pub fn is_broken_left(&self, sprite_name: Option<&str>) -> bool {
        sprite_name.is_some_and(|n| self.broken_left_map.contains_key(n))
    }

    /// `isBrokenRight(obj)`.
    pub fn is_broken_right(&self, sprite_name: Option<&str>) -> bool {
        sprite_name.is_some_and(|n| self.broken_right_map.contains_key(n))
    }

    /// `isBreakableObject(obj)` / `isBreakableSprite(name)`.
    pub fn is_breakable_sprite(&self, sprite_name: Option<&str>) -> bool {
        sprite_name.is_some_and(|n| self.all_map.contains_key(n))
    }

    /// `updateSprite(obj, brokenLeft, brokenRight)`'s new sprite name
    /// (`None`: the object keeps its sprite).
    pub fn update_sprite_name(
        &self,
        sprite_name: Option<&str>,
        broken_left: bool,
        broken_right: bool,
        rand: &mut dyn FenceRand,
    ) -> Option<String> {
        let tile = self.tile(sprite_name?)?;
        if broken_left && broken_right {
            pick_random(&tile.self_, rand)
        } else if broken_left {
            pick_random(&tile.left, rand)
        } else if broken_right {
            pick_random(&tile.right, rand)
        } else {
            None
        }
    }

    /// `getBreakableObject(square, north)`: the index of the first object
    /// (`(sprite name, collideN, collideW)` in square order) that is
    /// breakable and collides on that side.
    pub fn get_breakable_object(
        &self,
        objects: &[(Option<&str>, bool, bool)],
        north: bool,
    ) -> Option<usize> {
        objects.iter().position(|&(name, n, w)| {
            self.is_breakable_sprite(name) && ((north && n) || (!north && w))
        })
    }

    /// `damageAdjacent(square, dirAdjacent, dirBreak)` for the breakable
    /// object found on the adjacent square (`is_nw`: it collides north and
    /// west).
    pub fn adjacent_damage(
        &self,
        sprite_name: Option<&str>,
        is_nw: bool,
        dir_adjacent: IsoDirections,
    ) -> AdjacentDamage {
        use IsoDirections as D;
        let left = matches!(dir_adjacent, D::N | D::E);
        let right = matches!(dir_adjacent, D::S | D::W);
        if is_nw && matches!(dir_adjacent, D::S | D::E) {
            return AdjacentDamage::None;
        }
        if left && self.is_broken_right(sprite_name) {
            return AdjacentDamage::Destroy;
        }
        if right && self.is_broken_left(sprite_name) {
            return AdjacentDamage::Destroy;
        }
        AdjacentDamage::Damage { left, right }
    }

    /// The `damageAdjacent` calls `destroyFence` makes after destroying a
    /// fence that collides north / west: `(dirAdjacent, north)` where
    /// `north` is the side `getBreakableObject` looks for on that square.
    pub fn destroy_adjacent_dirs(north: bool, west: bool) -> Vec<(IsoDirections, bool)> {
        use IsoDirections as D;
        let dirs: &[D] = if north && west {
            &[D::S, D::E]
        } else if north {
            &[D::W, D::E]
        } else if west {
            &[D::N, D::S]
        } else {
            &[]
        };
        dirs.iter()
            .map(|&d| (d, matches!(d, D::W | D::E)))
            .collect()
    }

    /// `addDebrisObject(obj, dir)`'s debris sprite and whether it goes on
    /// the adjacent square in `dir` (north and west) instead of the
    /// fence's. `Err` for a direction other than N/S/W/E
    /// (`IllegalArgumentException("invalid direction")`).
    pub fn debris_sprite(
        &self,
        sprite_name: Option<&str>,
        dir: IsoDirections,
        rand: &mut dyn FenceRand,
    ) -> Result<Option<(String, bool)>, &'static str> {
        let Some(tile) = sprite_name.and_then(|n| self.tile(n)) else {
            return Ok(None);
        };
        let (list, adjacent) = match dir {
            IsoDirections::N => (&tile.debris_n, true),
            IsoDirections::S => (&tile.debris_s, false),
            IsoDirections::W => (&tile.debris_w, true),
            IsoDirections::E => (&tile.debris_e, false),
            _ => return Err("invalid direction"),
        };
        Ok(pick_random(list, rand).map(|s| (s, adjacent)))
    }

    /// `addItems(obj, square)`: the items dropped for the object's
    /// `Material` / `Material2` / `Material3` properties.
    pub fn add_items(materials: [Option<&str>; 3], rand: &mut dyn FenceRand) -> Vec<DebrisItem> {
        let has = |m: &str| materials.iter().any(|x| *x == Some(m));
        let mut out = Vec::new();
        let mut add = |t: &'static str, rand: &mut dyn FenceRand| {
            let x = rand.next_float(0.0, 0.5);
            let y = rand.next_float(0.0, 0.5);
            out.push(DebrisItem { item_type: t, x, y });
        };
        if has("Wood") {
            add("Base.UnusableWood", rand);
            if rand.next(5) == 0 {
                add("Base.UnusableWood", rand);
            }
        }
        if has("MetalBars") && rand.next(2) == 0 {
            add("Base.MetalBar", rand);
        }
        if has("MetalWire") && rand.next(3) == 0 {
            add("Base.Wire", rand);
        }
        if has("MetalPlates") && rand.next(2) == 0 {
            add("Base.SheetMetal", rand);
            add("Base.NutsBolts", rand);
        }
        if has("MetalScrap") && rand.next(2) == 0 {
            add("Base.SteelChunk", rand);
            add("Base.SteelPiece", rand);
        }
        if has("Nails") && rand.next(2) == 0 {
            add("Base.Nails", rand);
        }
        if has("Screws") && rand.next(2) == 0 {
            add("Base.Screws", rand);
        }
        out
    }

    /// `Reset()`. The `Tile` objects stay reachable in Java only through the
    /// maps, so they go too.
    pub fn reset(&mut self) {
        self.unbroken_map.clear();
        self.broken_left_map.clear();
        self.broken_right_map.clear();
        self.all_map.clear();
        self.tiles.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Seq(Vec<i32>);

    impl FenceRand for Seq {
        fn next(&mut self, max: i32) -> i32 {
            self.0.remove(0).min(max - 1)
        }
        fn next_float(&mut self, min: f32, _max: f32) -> f32 {
            min
        }
    }

    fn v(s: &[&str]) -> Option<Vec<String>> {
        (!s.is_empty()).then(|| s.iter().map(|s| s.to_string()).collect())
    }

    fn fences() -> BrokenFences {
        let mut b = BrokenFences::default();
        b.add_broken_tiles(vec![
            ("VERSION".into(), BrokenTile::default()),
            (
                "f_1".into(),
                BrokenTile {
                    self_: v(&["d_1"]),
                    left: v(&["d_2"]),
                    right: v(&["d_0", "d_9"]),
                    ..Default::default()
                },
            ),
        ]);
        b
    }

    #[test]
    fn registry_and_sprites() {
        let mut b = fences();
        assert!(b.is_unbroken(Some("f_1")) && !b.is_unbroken(Some("d_2")));
        assert!(b.is_broken_left(Some("d_2")) && b.is_broken_right(Some("d_9")));
        assert!(b.is_breakable_sprite(Some("d_0")) && !b.is_breakable_sprite(Some("d_1")));
        assert!(!b.is_breakable_sprite(None));
        // From a right-broken piece, breaking the left side picks `left` of
        // the same tile.
        let mut r = Seq(vec![0]);
        assert_eq!(
            b.update_sprite_name(Some("d_9"), true, false, &mut r)
                .as_deref(),
            Some("d_2")
        );
        let mut r = Seq(vec![1]);
        assert_eq!(
            b.update_sprite_name(Some("f_1"), false, true, &mut r)
                .as_deref(),
            Some("d_9")
        );
        assert_eq!(
            b.update_sprite_name(Some("f_1"), false, false, &mut r),
            None
        );
        assert_eq!(
            b.add_debris_tiles(vec![("nope".into(), Default::default())]),
            Err(UnknownTile)
        );
        b.add_debris_tiles(vec![(
            "f_1".into(),
            [v(&["n_0"]), v(&["s_0"]), None, v(&["e_0"])],
        )])
        .unwrap();
        let mut r = Seq(vec![0, 0]);
        assert_eq!(
            b.debris_sprite(Some("f_1"), IsoDirections::N, &mut r),
            Ok(Some(("n_0".into(), true)))
        );
        assert_eq!(
            b.debris_sprite(Some("f_1"), IsoDirections::W, &mut r),
            Ok(None)
        );
        assert!(b
            .debris_sprite(Some("f_1"), IsoDirections::NW, &mut r)
            .is_err());
        b.reset();
        assert!(b.all_map.is_empty());
    }

    #[test]
    fn adjacent_rules() {
        let b = fences();
        assert_eq!(
            b.adjacent_damage(Some("f_1"), true, IsoDirections::S),
            AdjacentDamage::None
        );
        assert_eq!(
            b.adjacent_damage(Some("d_9"), false, IsoDirections::N),
            AdjacentDamage::Destroy
        );
        assert_eq!(
            b.adjacent_damage(Some("d_2"), false, IsoDirections::W),
            AdjacentDamage::Destroy
        );
        assert_eq!(
            b.adjacent_damage(Some("f_1"), false, IsoDirections::E),
            AdjacentDamage::Damage {
                left: true,
                right: false
            }
        );
        assert_eq!(
            BrokenFences::destroy_adjacent_dirs(true, false),
            vec![(IsoDirections::W, true), (IsoDirections::E, true)]
        );
        assert_eq!(
            BrokenFences::destroy_adjacent_dirs(false, true),
            vec![(IsoDirections::N, false), (IsoDirections::S, false)]
        );
        let objs = [
            (Some("x"), true, false),
            (Some("f_1"), false, true),
            (Some("d_2"), true, false),
        ];
        assert_eq!(b.get_breakable_object(&objs, true), Some(2));
        assert_eq!(b.get_breakable_object(&objs, false), Some(1));
    }

    #[test]
    fn items() {
        // Wood: one plank, a second on Next(5) == 0; MetalWire on Next(3) == 0.
        let mut r = Seq(vec![0, 1]);
        let it = BrokenFences::add_items([Some("Wood"), None, Some("MetalWire")], &mut r);
        let types: Vec<_> = it.iter().map(|i| i.item_type).collect();
        assert_eq!(types, ["Base.UnusableWood", "Base.UnusableWood"]);
    }
}
