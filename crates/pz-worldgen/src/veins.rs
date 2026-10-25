//! `zombie.iso.worldgen.veins`: ore veins (a centre disc plus straight
//! arms) placed per meta cell from the world seed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::defs::{OreVeinConfig, TileGroup};
use crate::java::JavaRandom;
use crate::params::WorldGenParams;

/// `zombie.iso.Vector2.normalize()` on a float pair.
fn normalize(x: f32, y: f32) -> (f32, f32) {
    let len_sq = x * x + y * y;
    if (1.0f32 - len_sq).abs() < 1.0e-5 {
        return (x, y);
    }
    if len_sq == 0.0 {
        return (0.0, 0.0);
    }
    let len = (len_sq as f64).sqrt() as f32;
    (x / len, y / len)
}

/// `OreVein`.
#[derive(Clone, Debug)]
pub struct OreVein {
    pub start: (f32, f32),
    pub arms_distance: Vec<f32>,
    pub end_points: Vec<(f32, f32)>,
    pub config: usize,
    pub arms_prob: f32,
    pub center_prob: f32,
    pub center_radius: i32,
}

impl OreVein {
    /// `new OreVein(cellX, cellY, config, rnd)`.
    pub fn new(
        cell_x: i32,
        cell_y: i32,
        config: usize,
        c: &OreVeinConfig,
        rnd: &mut JavaRandom,
    ) -> Self {
        let cx = rnd.next_int_bound(256);
        let cy = rnd.next_int_bound(256);
        let start = ((cell_x * 256 + cx) as f32, (cell_y * 256 + cy) as f32);
        let amount =
            rnd.next_int_bound(c.arms_amount_max - c.arms_amount_min + 1) + c.arms_amount_min;
        let offset_angle = rnd.next_float() * 360.0;
        let delta_angle = 360.0f32 / amount as f32;
        let mut arms_distance = Vec::new();
        let mut end_points = Vec::new();
        for i in 0..amount {
            let rao = (rnd.next_float() * 2.0 - 1.0) * c.arms_delta_angle as f32;
            let orientation = delta_angle * i as f32 + offset_angle + rao;
            let dist = rnd.next_float() * c.arms_dist_max as f32 + c.arms_dist_min as f32;
            let rad = (orientation as f64).to_radians();
            end_points.push((
                start.0 + (rad.sin() * dist as f64) as f32,
                start.1 + (-rad.cos() * dist as f64) as f32,
            ));
            arms_distance.push(dist);
        }
        OreVein {
            start,
            arms_distance,
            end_points,
            config,
            arms_prob: c.arms_prob,
            center_prob: c.center_prob,
            center_radius: c.center_radius,
        }
    }

    /// `isValid(x, y, rnd)`.
    pub fn is_valid(&self, x: i32, y: i32, rnd: &mut JavaRandom) -> bool {
        let dx = self.start.0 - x as f32;
        let dy = self.start.1 - y as f32;
        let len_sq = dx * dx + dy * dy;
        if len_sq < self.center_radius.wrapping_mul(self.center_radius) as f32
            && rnd.next_float() < self.center_prob
        {
            return true;
        }
        for (i, &dist) in self.arms_distance.iter().enumerate() {
            if len_sq > dist * dist {
                continue;
            }
            let e = self.end_points[i];
            let d1 = normalize(e.0 - self.start.0, e.1 - self.start.1);
            let d3 = normalize(e.0 - x as f32, e.1 - y as f32);
            if (d1.0 - d3.0).abs() < 0.001
                && (d1.1 - d3.1).abs() < 0.001
                && rnd.next_float() < self.arms_prob
            {
                return true;
            }
        }
        false
    }
}

type VeinCache = HashMap<(i32, i32), Rc<Vec<OreVein>>>;

/// `Veins`: the veins of each cell, generated on first use.
pub struct Veins {
    pub configs: Vec<OreVeinConfig>,
    cache: RefCell<VeinCache>,
}

impl Veins {
    pub fn new(configs: Vec<OreVeinConfig>) -> Self {
        Veins {
            configs,
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// `get(cellX, cellY)`.
    pub fn get(&self, params: &WorldGenParams, cell_x: i32, cell_y: i32) -> Rc<Vec<OreVein>> {
        if let Some(v) = self.cache.borrow().get(&(cell_x, cell_y)) {
            return v.clone();
        }
        let mut rnd = params.random(cell_x, cell_y, 0);
        let mut out = Vec::new();
        for (i, c) in self.configs.iter().enumerate() {
            if rnd.next_float() > c.probability {
                continue;
            }
            out.push(OreVein::new(cell_x, cell_y, i, c, &mut rnd));
        }
        let out = Rc::new(out);
        self.cache
            .borrow_mut()
            .insert((cell_x, cell_y), out.clone());
        out
    }

    /// The vein's single-tile features (`getSingleFeatures`).
    pub fn tiles(&self, vein: &OreVein) -> &[TileGroup] {
        &self.configs[vein.config].tiles
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_square_is_in_the_vein_when_probabilities_are_one() {
        let c = OreVeinConfig {
            tiles: vec![],
            center_radius: 5,
            center_prob: 1.0,
            arms_amount_min: 3,
            arms_amount_max: 3,
            arms_dist_min: 10,
            arms_dist_max: 10,
            arms_delta_angle: 0,
            arms_prob: 1.0,
            probability: 1.0,
        };
        let mut rnd = JavaRandom::new(1);
        let v = OreVein::new(0, 0, 0, &c, &mut rnd);
        assert_eq!(v.end_points.len(), 3);
        let (sx, sy) = (v.start.0 as i32, v.start.1 as i32);
        assert!(v.is_valid(sx, sy, &mut rnd));
        assert!(!v.is_valid(sx + 100, sy + 100, &mut rnd));
        assert_eq!(normalize(3.0, 4.0), (0.6, 0.8));
    }
}
