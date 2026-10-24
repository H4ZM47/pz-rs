//! `zombie.iso.weather.SimplexNoise` (3D noise only, which is what world
//! generation samples) and `zombie.iso.worldgen.WorldGenSimplexGenerator`.
//!
//! The permutation and gradient tables are read from the installed game's
//! `SimplexNoise` class at runtime.

use pz_classfile::{Jar, Value};

use crate::java::JavaRandom;

const F3: f64 = 0.3333333333333333;
const G3: f64 = 0.16666666666666666;

/// The static tables of `SimplexNoise`: `perm`, `permMod12` and `grad3`.
#[derive(Clone, Debug)]
pub struct SimplexTables {
    perm: Vec<i32>,
    perm_mod12: Vec<i32>,
    grad3: Vec<[f64; 3]>,
}

impl SimplexTables {
    /// Builds the tables from a 256-entry permutation and the 3D gradients
    /// (the original's static initializer: `perm[i] = p[i & 255]`,
    /// `permMod12[i] = perm[i] % 12`).
    pub fn new(p: &[i32], grad3: Vec<[f64; 3]>) -> Result<Self, String> {
        if p.len() != 256 {
            return Err(format!("permutation has {} entries", p.len()));
        }
        if grad3.len() != 12 {
            return Err(format!("grad3 has {} entries", grad3.len()));
        }
        let perm: Vec<i32> = (0..512).map(|i| p[i & 255]).collect();
        let perm_mod12 = perm.iter().map(|v| v % 12).collect();
        Ok(SimplexTables {
            perm,
            perm_mod12,
            grad3,
        })
    }

    /// Reads `SimplexNoise.p` and `SimplexNoise.grad3` from the game jar.
    pub fn from_jar(jar: &Jar) -> Result<Self, String> {
        const CLASS: &str = "zombie/iso/weather/SimplexNoise";
        let get = |field: &str| -> Result<Vec<Value>, String> {
            match pz_classfile::static_value(jar, CLASS, field).map_err(|e| format!("{e:?}"))? {
                Some(Value::Array(a)) => Ok(a),
                other => Err(format!("SimplexNoise.{field}: {other:?}")),
            }
        };
        let num = |v: &Value| -> Result<f64, String> {
            match v {
                Value::Int(i) => Ok(*i as f64),
                Value::Float(f) => Ok(*f),
                other => Err(format!("not a number: {other:?}")),
            }
        };
        let p = get("p")?
            .iter()
            .map(|v| num(v).map(|f| f as i32))
            .collect::<Result<Vec<_>, _>>()?;
        let grad3 = get("grad3")?
            .iter()
            .map(|g| match g {
                Value::New { args, .. } if args.len() >= 3 => {
                    let n = args.len();
                    Ok([num(&args[n - 3])?, num(&args[n - 2])?, num(&args[n - 1])?])
                }
                other => Err(format!("grad3 entry: {other:?}")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(&p, grad3)
    }

    fn dot(&self, g: i32, x: f64, y: f64, z: f64) -> f64 {
        let g = &self.grad3[g as usize];
        g[0] * x + g[1] * y + g[2] * z
    }

    /// `SimplexNoise.noise(x, y, z)`.
    pub fn noise3(&self, xin: f64, yin: f64, zin: f64) -> f64 {
        let s = (xin + yin + zin) * F3;
        let i = fastfloor(xin + s);
        let j = fastfloor(yin + s);
        let k = fastfloor(zin + s);
        let t = (i.wrapping_add(j).wrapping_add(k)) as f64 * G3;
        let x0 = xin - (i as f64 - t);
        let y0 = yin - (j as f64 - t);
        let z0 = zin - (k as f64 - t);
        let (i1, j1, k1, i2, j2, k2) = if x0 >= y0 {
            if y0 >= z0 {
                (1, 0, 0, 1, 1, 0)
            } else if x0 >= z0 {
                (1, 0, 0, 1, 0, 1)
            } else {
                (0, 0, 1, 1, 0, 1)
            }
        } else if y0 < z0 {
            (0, 0, 1, 0, 1, 1)
        } else if x0 < z0 {
            (0, 1, 0, 0, 1, 1)
        } else {
            (0, 1, 0, 1, 1, 0)
        };
        let x1 = x0 - i1 as f64 + G3;
        let y1 = y0 - j1 as f64 + G3;
        let z1 = z0 - k1 as f64 + G3;
        let x2 = x0 - i2 as f64 + F3;
        let y2 = y0 - j2 as f64 + F3;
        let z2 = z0 - k2 as f64 + F3;
        let x3 = x0 - 1.0 + 0.5;
        let y3 = y0 - 1.0 + 0.5;
        let z3 = z0 - 1.0 + 0.5;
        let ii = (i & 0xFF) as usize;
        let jj = (j & 0xFF) as usize;
        let kk = (k & 0xFF) as usize;
        let p = &self.perm;
        let pm = &self.perm_mod12;
        let gi0 = pm[ii + p[jj + p[kk] as usize] as usize];
        let gi1 = pm[ii + i1 + p[jj + j1 + p[kk + k1] as usize] as usize];
        let gi2 = pm[ii + i2 + p[jj + j2 + p[kk + k2] as usize] as usize];
        let gi3 = pm[ii + 1 + p[jj + 1 + p[kk + 1] as usize] as usize];
        let corner = |t: f64, g: i32, x: f64, y: f64, z: f64| {
            if t < 0.0 {
                0.0
            } else {
                let t = t * t;
                t * t * self.dot(g, x, y, z)
            }
        };
        let n0 = corner(0.6 - x0 * x0 - y0 * y0 - z0 * z0, gi0, x0, y0, z0);
        let n1 = corner(0.6 - x1 * x1 - y1 * y1 - z1 * z1, gi1, x1, y1, z1);
        let n2 = corner(0.6 - x2 * x2 - y2 * y2 - z2 * z2, gi2, x2, y2, z2);
        let n3 = corner(0.6 - x3 * x3 - y3 * y3 - z3 * z3, gi3, x3, y3, z3);
        32.0 * (n0 + n1 + n2 + n3)
    }
}

/// `SimplexNoise.fastfloor(double)`.
fn fastfloor(x: f64) -> i32 {
    let xi = x as i32;
    if x < xi as f64 {
        xi - 1
    } else {
        xi
    }
}

/// `BiomeNoise`: one noise value per biome axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BiomeNoise {
    pub landscape: f64,
    pub plant: f64,
    pub bush: f64,
    pub temperature: f64,
    pub hygrometry: f64,
    pub ore: f64,
}

/// `WorldGenSimplexGenerator.WGSimplex`.
#[derive(Clone, Copy, Debug)]
struct WgSimplex {
    offset_x: f64,
    offset_y: f64,
    depth: f64,
    scale: f64,
}

impl WgSimplex {
    fn from_seed(seed: i64, scale: f64) -> Self {
        let mut rnd = JavaRandom::new(seed);
        let offset_x = rnd.next_double();
        let offset_y = rnd.next_double();
        let depth = rnd.next_double();
        WgSimplex {
            offset_x,
            offset_y,
            depth,
            scale,
        }
    }

    fn noise(&self, t: &SimplexTables, x: f64, y: f64) -> f64 {
        t.noise3(
            (x + self.offset_x) / self.scale,
            (y + self.offset_y) / self.scale,
            self.depth,
        )
    }
}

/// `WorldGenSimplexGenerator`: six biome-axis noises at scale 128 and a
/// selector noise at scale 16, each offset by a `Random(seed + k)`.
#[derive(Clone, Debug)]
pub struct WorldGenSimplexGenerator {
    tables: SimplexTables,
    axes: [WgSimplex; 6],
    selector: WgSimplex,
}

impl WorldGenSimplexGenerator {
    pub fn new(tables: SimplexTables, seed: i64) -> Self {
        let axis = |k: i64| WgSimplex::from_seed(seed.wrapping_add(k), 128.0);
        WorldGenSimplexGenerator {
            tables,
            axes: [
                axis(100),
                axis(200),
                axis(300),
                axis(400),
                axis(500),
                axis(600),
            ],
            selector: WgSimplex::from_seed(seed.wrapping_add(700), 16.0),
        }
    }

    /// `noise(x, y)`.
    pub fn noise(&self, x: f64, y: f64) -> BiomeNoise {
        let n = |i: usize| self.axes[i].noise(&self.tables, x, y);
        BiomeNoise {
            landscape: n(0),
            plant: n(1),
            bush: n(2),
            temperature: n(3),
            hygrometry: n(4),
            ore: n(5),
        }
    }

    /// `selector(x, y)`: the selector noise mapped to `[0, 1]`.
    pub fn selector(&self, x: f64, y: f64) -> f64 {
        (self.selector.noise(&self.tables, x, y) + 1.0) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An arbitrary permutation and the twelve cube-edge gradients.
    fn tables() -> SimplexTables {
        let p: Vec<i32> = (0..256).map(|i| (i * 167 + 13) % 256).collect();
        let mut g = Vec::new();
        for (a, b) in [(0, 1), (0, 2), (1, 2)] {
            for (sa, sb) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                let mut v = [0.0; 3];
                v[a] = sa;
                v[b] = sb;
                g.push(v);
            }
        }
        SimplexTables::new(&p, g).unwrap()
    }

    #[test]
    fn noise_is_bounded_and_zero_on_lattice_corners() {
        let t = tables();
        assert_eq!(t.noise3(0.0, 0.0, 0.0), 0.0);
        for i in 0..200 {
            let v = t.noise3(i as f64 * 0.37, i as f64 * -0.11, 0.5);
            assert!((-1.0..=1.0).contains(&v));
        }
    }

    #[test]
    fn selector_in_unit_range() {
        let g = WorldGenSimplexGenerator::new(tables(), 42);
        for x in 0..50 {
            let s = g.selector(x as f64 * 3.0, 7.0);
            assert!((0.0..=1.0).contains(&s));
        }
    }
}
