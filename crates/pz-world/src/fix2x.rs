//! `IsoChunk.Fix2x(String)`: renames tiles in maps not marked `fixed2x`.
//!
//! The rename table (`IsoChunk.Fix2xMap`) is game data built in Java code, so
//! it is not part of this repository: it is supplied at runtime, read from
//! the installed `projectzomboid.jar`.

use std::collections::HashMap;

use crate::java::GameRand;

/// `IsoChunk.Fix2xMap`: old tile name to replacement. A replacement may be
/// empty, or the markers `randBush` / `randPlant`.
#[derive(Clone, Debug, Default)]
pub struct Fix2xTable {
    map: HashMap<String, String>,
}

impl Fix2xTable {
    pub fn from_entries(entries: impl IntoIterator<Item = (String, String)>) -> Self {
        // `HashMap.put`: a later entry replaces an earlier one.
        Fix2xTable {
            map: entries.into_iter().collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `IsoChunk.Fix2x(tileName)`. Returns the replacement name (possibly
    /// empty) or the input unchanged; `randBush`/`randPlant` draw from `rand`.
    pub fn fix2x(&self, tile_name: &str, rand: &mut dyn GameRand) -> String {
        let Some(s) = self.map.get(tile_name) else {
            return tile_name.to_string();
        };
        if s == "randBush" {
            let id = 64 + rand.next(16);
            return format!("f_bushes_1_{id}");
        }
        if s == "randPlant" {
            let id = rand.next(4) * 16 + rand.next(8);
            return format!("d_plants_1_{id}");
        }
        s.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Seq(Vec<i32>);
    impl GameRand for Seq {
        fn next(&mut self, _max: i32) -> i32 {
            self.0.remove(0)
        }
    }

    #[test]
    fn renames() {
        let t = Fix2xTable::from_entries(
            [
                ("old_a", ""),
                ("old_b", "new_b"),
                ("old_c", "randBush"),
                ("old_d", "randPlant"),
            ]
            .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        let mut r = Seq(vec![3, 2, 5]);
        assert_eq!(t.fix2x("old_a", &mut r), "");
        assert_eq!(t.fix2x("old_b", &mut r), "new_b");
        assert_eq!(t.fix2x("old_c", &mut r), "f_bushes_1_67");
        assert_eq!(t.fix2x("old_d", &mut r), "d_plants_1_37");
        assert_eq!(t.fix2x("other", &mut r), "other");
    }
}
