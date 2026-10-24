//! `zombie.erosion.utils.Noise2D`: layered 2D value noise over a
//! permutation table. The table (`Noise2D.perm`) is game code data and is
//! read from the installed game ([`crate::tables::ErosionTables`]).

use std::sync::Arc;

#[derive(Clone, Debug)]
struct Layer {
    freq: f32,
    amp: f32,
    p: Box<[i32; 512]>,
}

/// `Noise2D`.
#[derive(Clone, Debug)]
pub struct Noise2D {
    perm: Arc<[i32]>,
    layers: Vec<Layer>,
}

fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// `(int)Math.floor((double)v - Math.floor(v / 255.0f) * 255.0)`: a floored
/// modulo computed in mixed float/double.
fn wrap(v: f32, m: f32) -> i32 {
    ((v as f64) - ((v / m) as f64).floor() * m as f64).floor() as i32
}

impl Noise2D {
    /// `new Noise2D()` over the permutation table.
    pub fn new(perm: Arc<[i32]>) -> Self {
        Noise2D { perm, layers: Vec::with_capacity(3) }
    }

    fn noise(&self, orig_x: f32, orig_y: f32, p: &[i32; 512]) -> f32 {
        let x = wrap(orig_x, 255.0) as usize;
        let y = wrap(orig_y, 255.0);
        let fx = fade(orig_x - (orig_x as f64).floor() as f32);
        let fy = fade(orig_y - (orig_y as f64).floor() as f32);
        let aa = (p[x] + y) as usize;
        let ab = (p[x] + y + 1) as usize;
        let ba = (p[x + 1] + y) as usize;
        let bb = (p[x + 1] + y + 1) as usize;
        let perm = &self.perm;
        lerp(
            fy,
            lerp(fx, perm[p[aa] as usize] as f32, perm[p[ba] as usize] as f32),
            lerp(fx, perm[p[ab] as usize] as f32, perm[p[bb] as usize] as f32),
        )
    }

    /// `layeredNoise(x, y)`.
    pub fn layered_noise(&self, x: f32, y: f32) -> f32 {
        let mut f = 0.0f32;
        let mut max_sum = 0.0f32;
        for l in &self.layers {
            max_sum += l.amp;
            f += self.noise(x * l.freq, y * l.freq, &l.p) * l.amp;
        }
        f / max_sum / 255.0
    }

    /// `addLayer(seed, freq, amp)`.
    pub fn add_layer(&mut self, seed: i32, freq: f32, amp: f32) {
        let seed = ((seed as f64) - ((seed as f32 / 256.0) as f64).floor() * 256.0).floor() as i32;
        let mut p = Box::new([0i32; 512]);
        for i in 0..256 {
            let s = seed.wrapping_add(i);
            let ii = ((s as f64) - ((s as f32 / 256.0) as f64).floor() * 256.0).floor() as usize;
            p[ii] = self.perm[i as usize];
            p[256 + ii] = p[ii];
        }
        self.layers.push(Layer { freq, amp, p });
    }

    /// The permutation table.
    pub fn perm(&self) -> Arc<[i32]> {
        self.perm.clone()
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.layers.clear();
    }
}
