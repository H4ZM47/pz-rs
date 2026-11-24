//! Build 42.22 `zombie.iso.worldgen.zombie.ZombieVoronoi` — Voronoi noise
//! zombie density modifiers read from the Lua global `zombie_voronoi` table
//! and passed into `IsoMetaGrid::create`.

use std::collections::HashMap;

use pz_kahlua::{TableRef, Value};
use pz_world::ZombieVoronoi as MetaGridZombieVoronoi;

use crate::java::JavaRandom;

/// `zombie.iso.worldgen.zombie.ClosestSelectionType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClosestSelectionType {
    First,
    Second,
    SecondMinusFirst,
}

impl ClosestSelectionType {
    /// Parses a Lua `closest` string (`"FIRST"`, `"SECOND"`, `"SECOND_MINUS_FIRST"`).
    pub fn from_name(name: &str) -> Self {
        match name {
            "FIRST" => Self::First,
            "SECOND" => Self::Second,
            _ => Self::SecondMinusFirst,
        }
    }

    /// `ClosestSelection.getClosest(x, y, points)` over squared Euclidean distances.
    pub fn get_closest(self, x: f64, y: f64, points: &[(f64, f64)]) -> f64 {
        if points.is_empty() {
            return 0.0;
        }
        let mut d0 = f64::INFINITY;
        let mut d1 = f64::INFINITY;
        for &(px, py) in points {
            let dx = x - px;
            let dy = y - py;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < d0 {
                d1 = d0;
                d0 = dist_sq;
            } else if dist_sq < d1 {
                d1 = dist_sq;
            }
        }
        match self {
            Self::First => d0,
            Self::Second => {
                if d1.is_finite() {
                    d1
                } else {
                    d0
                }
            }
            Self::SecondMinusFirst => {
                if d1.is_finite() {
                    d1 - d0
                } else {
                    0.0
                }
            }
        }
    }
}

/// `zombie.iso.worldgen.zombie.ZombieVoronoiEntry`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZombieVoronoiEntry {
    pub number_points: i32,
    pub closest_point: ClosestSelectionType,
    pub scale: f64,
    pub cutoff: f64,
}

impl Default for ZombieVoronoiEntry {
    fn default() -> Self {
        Self {
            number_points: 1,
            closest_point: ClosestSelectionType::SecondMinusFirst,
            scale: 16.0,
            cutoff: 0.2,
        }
    }
}

/// `HashUtil.finalizeHash(long)`.
#[inline]
fn finalize_hash(mut h: i64) -> i64 {
    h = h.wrapping_mul(h).wrapping_mul(h).wrapping_mul(60493);
    (h >> 13) ^ h
}

/// `HashUtil.hash2D(seed, x, y)`.
#[inline]
pub fn hash_2d(seed: i64, x: i64, y: i64) -> i32 {
    let mut h = seed ^ 1619i64.wrapping_mul(x);
    h ^= 31337i64.wrapping_mul(y);
    finalize_hash(h) as i32
}

/// `ZombieVoronoiReader.getEntries(KahluaTable)` reading the Lua global
/// `zombie_voronoi` list of entry tables.
pub fn read_entries(table: &TableRef) -> Vec<ZombieVoronoiEntry> {
    let mut out = Vec::new();
    for (_k, v) in table.entries() {
        let Some(entry_tbl) = v.as_table() else {
            continue;
        };
        let number_points = match entry_tbl.get_str("points") {
            Value::Num(n) => n as i32,
            _ => 1,
        };
        let closest_point = match entry_tbl.get_str("closest") {
            Value::Str(s) => ClosestSelectionType::from_name(&s),
            _ => ClosestSelectionType::SecondMinusFirst,
        };
        let scale = match entry_tbl.get_str("scale") {
            Value::Num(n) => n,
            _ => 16.0,
        };
        let cutoff = match entry_tbl.get_str("cutoff") {
            Value::Num(n) => n,
            _ => 0.2,
        };
        out.push(ZombieVoronoiEntry {
            number_points,
            closest_point,
            scale,
            cutoff,
        });
    }
    out
}

/// `zombie.iso.worldgen.zombie.ZombieVoronoi`.
pub struct ZombieVoronoi {
    pub seed: i64,
    pub number_points: i32,
    pub closest_point: ClosestSelectionType,
    pub scale: f64,
    pub cutoff: f64,
    /// `SandboxOptions.instance.zombieVoronoiNoise.getValue()`.
    pub noise_enabled: bool,
    cell_cache: HashMap<(i32, i32), Vec<f64>>,
}

impl ZombieVoronoi {
    pub fn new(seed: i64, entry: ZombieVoronoiEntry, noise_enabled: bool) -> Self {
        Self {
            seed,
            number_points: entry.number_points.max(0),
            closest_point: entry.closest_point,
            scale: if entry.scale == 0.0 { 16.0 } else { entry.scale },
            cutoff: entry.cutoff,
            noise_enabled,
            cell_cache: HashMap::new(),
        }
    }

    /// `ZombieVoronoi.evaluateNoise(x, y, rnd, points)`.
    pub fn evaluate_noise(
        &self,
        x: f64,
        y: f64,
        rnd: &mut JavaRandom,
        points: &mut Vec<(f64, f64)>,
    ) -> f64 {
        points.clear();
        let gx = x.floor() as i32;
        let gy = y.floor() as i32;
        for dx in -1..=1 {
            let cx = gx.wrapping_add(dx);
            for dy in -1..=1 {
                let cy = gy.wrapping_add(dy);
                let h = hash_2d(self.seed, cx as i64, cy as i64);
                rnd.set_seed(h as i64);
                for _ in 0..self.number_points {
                    let px = rnd.next_double() + cx as f64;
                    let py = rnd.next_double() + cy as f64;
                    points.push((px, py));
                }
            }
        }
        self.closest_point.get_closest(x, y, points)
    }

    /// `ZombieVoronoi.evaluateCellNoise(cellX, cellY)`: 1024 values indexed
    /// `wx + wy * 32` for `wx, wy in 0..32`, cached per cell until released.
    pub fn evaluate_cell_noise(&mut self, cell_x: i32, cell_y: i32) -> &[f64] {
        if !self.cell_cache.contains_key(&(cell_x, cell_y)) {
            let mut out = vec![0.0; 1024];
            let mut points = Vec::with_capacity((self.number_points as usize) * 9);
            let mut rnd = JavaRandom::new(0);
            for wx in 0..32 {
                let sx = (wx + cell_x.wrapping_mul(32)) as f64 / self.scale;
                for wy in 0..32 {
                    let sy = (wy + cell_y.wrapping_mul(32)) as f64 / self.scale;
                    out[(wx + wy * 32) as usize] =
                        self.evaluate_noise(sx, sy, &mut rnd, &mut points);
                }
            }
            self.cell_cache.insert((cell_x, cell_y), out);
        }
        self.cell_cache.get(&(cell_x, cell_y)).unwrap()
    }

    /// `ZombieVoronoi.evaluateCellCutoff(cellX, cellY, cutoff)`.
    pub fn evaluate_cell_cutoff_with(
        &mut self,
        cell_x: i32,
        cell_y: i32,
        cutoff: f64,
    ) -> Vec<f64> {
        self.evaluate_cell_noise(cell_x, cell_y)
            .iter()
            .map(|&v| if v < cutoff { 0.0 } else { 1.0 })
            .collect()
    }
}

impl MetaGridZombieVoronoi for ZombieVoronoi {
    fn evaluate_cell_cutoff(&mut self, cell_x: i32, cell_y: i32) -> Vec<f64> {
        let cutoff = if self.noise_enabled { self.cutoff } else { 0.0 };
        self.evaluate_cell_cutoff_with(cell_x, cell_y, cutoff)
    }

    fn release_cell(&mut self, cell_x: i32, cell_y: i32) {
        self.cell_cache.remove(&(cell_x, cell_y));
    }
}

/// `ZombieVoronoi.getVoronois(seed)`: reads the Lua global `zombie_voronoi`
/// table and builds one boxed [`MetaGridZombieVoronoi`] per entry with seed
/// `seed + i`.
pub fn from_lua_globals(
    globals: &TableRef,
    seed: i64,
    noise_enabled: bool,
) -> Vec<Box<dyn MetaGridZombieVoronoi>> {
    let Some(tbl) = globals.get_str("zombie_voronoi").as_table().cloned() else {
        return Vec::new();
    };
    read_entries(&tbl)
        .into_iter()
        .enumerate()
        .map(|(i, entry)| {
            Box::new(ZombieVoronoi::new(
                seed.wrapping_add(i as i64),
                entry,
                noise_enabled,
            )) as Box<dyn MetaGridZombieVoronoi>
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_2d_and_closest_selection_match_build_42_22() {
        let h1 = hash_2d(12345, 7, -3);
        let h2 = hash_2d(12345, 7, -3);
        let h3 = hash_2d(12345, 8, -3);
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);

        let pts = vec![(1.0, 0.0), (3.0, 0.0), (5.0, 0.0)];
        let d1 = ClosestSelectionType::First.get_closest(0.0, 0.0, &pts);
        let d2 = ClosestSelectionType::Second.get_closest(0.0, 0.0, &pts);
        let diff = ClosestSelectionType::SecondMinusFirst.get_closest(0.0, 0.0, &pts);
        assert!((d1 - 1.0).abs() < 1e-12);
        assert!((d2 - 9.0).abs() < 1e-12);
        assert!((diff - 8.0).abs() < 1e-12);
    }

    #[test]
    fn reader_evaluates_cell_cutoff_and_cache_lifecycle() {
        let globals = TableRef::new();
        let list = TableRef::new();
        let e1 = TableRef::new();
        e1.set_str("points", 2.0);
        e1.set_str("closest", "FIRST");
        e1.set_str("scale", 8.0);
        e1.set_str("cutoff", 0.25);
        list.set_int(1, e1);
        // Second entry exercises all defaults (points=1, SECOND_MINUS_FIRST, scale=16, cutoff=0.2).
        list.set_int(2, TableRef::new());
        globals.set_str("zombie_voronoi", list);

        let entries = read_entries(globals.get_str("zombie_voronoi").as_table().unwrap());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].number_points, 2);
        assert_eq!(entries[0].closest_point, ClosestSelectionType::First);
        assert_eq!(entries[1], ZombieVoronoiEntry::default());

        let mut enabled = from_lua_globals(&globals, 99, true);
        assert_eq!(enabled.len(), 2);
        let c0 = enabled[0].evaluate_cell_cutoff(10, 20);
        assert_eq!(c0.len(), 1024);
        assert!(c0.iter().all(|&v| v == 0.0 || v == 1.0));
        assert!(c0.iter().any(|&v| v == 0.0) && c0.iter().any(|&v| v == 1.0));
        enabled[0].release_cell(10, 20);

        let mut disabled = from_lua_globals(&globals, 99, false);
        let c_off = disabled[0].evaluate_cell_cutoff(10, 20);
        assert_eq!(c_off.len(), 1024);
        assert!(c_off.iter().all(|&v| v == 1.0));
    }
}
