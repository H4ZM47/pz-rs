//! `FMOD_DSP_TYPE_MULTIBAND_EQ`: five bands of RBJ-cookbook biquads.
//!
//! Coefficients follow FMOD's: the frequency is clamped to 95 % of Nyquist
//! (and floored at 0.1 % of it), shelves use slope 1, and the 24/48 dB
//! responses cascade 2/4 identical sections whose Q is the square/fourth
//! root of the band's Q.

use super::Dsp;

const MAX_CHANNELS: usize = 8;

#[derive(Debug, Clone, Copy, Default)]
pub struct Band {
    pub filter: i32,
    pub frequency: f32,
    pub q: f32,
    pub gain_db: f32,
}

/// Normalised biquad: b0, b1, b2, a1, a2.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Coefs([f32; 5]);

const IDENTITY: Coefs = Coefs([1.0, 0.0, 0.0, 0.0, 0.0]);

fn coefs(filter: i32, rate: f32, frequency: f32, q: f32, gain_db: f32) -> Coefs {
    let rel = frequency / (rate * 0.5);
    let w0 = if rel < 0.001 {
        std::f32::consts::PI * 0.001
    } else {
        std::f32::consts::PI * rel.min(0.95)
    };
    let (sin, cos) = w0.sin_cos();
    let q = match filter {
        2 | 5 => q.sqrt(),
        3 | 6 => q.sqrt().sqrt(),
        _ => q,
    };
    let (b0, b1, b2, a0, a1, a2);
    match filter {
        1..=3 => {
            let alpha = sin / (2.0 * q);
            b0 = 0.5 * (1.0 - cos);
            b1 = 1.0 - cos;
            b2 = b0;
            a0 = 1.0 + alpha;
            a1 = -2.0 * cos;
            a2 = 1.0 - alpha;
        }
        4..=6 => {
            let alpha = sin / (2.0 * q);
            b0 = 0.5 * (1.0 + cos);
            b1 = -(1.0 + cos);
            b2 = b0;
            a0 = 1.0 + alpha;
            a1 = -2.0 * cos;
            a2 = 1.0 - alpha;
        }
        7 | 8 => {
            let a = 10f32.powf(gain_db / 40.0);
            let beta = sin * (2.0 * a).sqrt();
            let (ap1, am1) = (a + 1.0, a - 1.0);
            if filter == 7 {
                b0 = a * (ap1 - am1 * cos + beta);
                b1 = 2.0 * a * (am1 - ap1 * cos);
                b2 = a * (ap1 - am1 * cos - beta);
                a0 = ap1 + am1 * cos + beta;
                a1 = -2.0 * (am1 + ap1 * cos);
                a2 = ap1 + am1 * cos - beta;
            } else {
                b0 = a * (ap1 + am1 * cos + beta);
                b1 = -2.0 * a * (am1 + ap1 * cos);
                b2 = a * (ap1 + am1 * cos - beta);
                a0 = ap1 - am1 * cos + beta;
                a1 = 2.0 * (am1 - ap1 * cos);
                a2 = ap1 - am1 * cos - beta;
            }
        }
        9 => {
            let a = 10f32.powf(gain_db / 40.0);
            let alpha = sin / (2.0 * q);
            b0 = 1.0 + alpha * a;
            b1 = -2.0 * cos;
            b2 = 1.0 - alpha * a;
            a0 = 1.0 + alpha / a;
            a1 = b1;
            a2 = 1.0 - alpha / a;
        }
        // Band-pass, notch, all-pass and the 6 dB slopes are not used by
        // the game's banks.
        _ => return IDENTITY,
    }
    let n = 1.0 / a0;
    Coefs([b0 * n, b1 * n, b2 * n, a1 * n, a2 * n])
}

fn sections(filter: i32) -> usize {
    match filter {
        2 | 5 => 2,
        3 | 6 => 4,
        1 | 4 | 7 | 8 | 9 => 1,
        _ => 0,
    }
}

#[derive(Debug, Clone)]
pub struct MultibandEq {
    pub bands: [Band; 5],
    rate: f32,
    /// Transposed direct form II state: [band][section][channel] = (z1, z2).
    state: Vec<[(f32, f32); MAX_CHANNELS]>,
}

impl MultibandEq {
    pub fn new(rate: u32) -> MultibandEq {
        MultibandEq {
            bands: [Band {
                filter: 0,
                frequency: 8000.0,
                q: 0.707,
                gain_db: 0.0,
            }; 5],
            rate: rate as f32,
            state: vec![[(0.0, 0.0); MAX_CHANNELS]; 5 * 4],
        }
    }

    /// Parameters in FMOD order: per band A..E filter, frequency, Q, gain.
    pub fn set(&mut self, index: usize, v: f32) {
        let (b, p) = (index / 4, index % 4);
        let Some(band) = self.bands.get_mut(b) else {
            return;
        };
        match p {
            0 => band.filter = v as i32,
            1 => band.frequency = v,
            2 => band.q = v,
            _ => band.gain_db = v,
        }
    }
}

impl Dsp for MultibandEq {
    fn process(&mut self, buf: &mut [f32], channels: usize) {
        let channels = channels.min(MAX_CHANNELS);
        for (bi, band) in self.bands.iter().enumerate() {
            let n = sections(band.filter);
            if n == 0 {
                continue;
            }
            let Coefs([b0, b1, b2, a1, a2]) =
                coefs(band.filter, self.rate, band.frequency, band.q, band.gain_db);
            for s in 0..n {
                let st = &mut self.state[bi * 4 + s];
                for frame in buf.chunks_exact_mut(channels) {
                    for (ch, x) in frame.iter_mut().enumerate() {
                        let (z1, z2) = st[ch];
                        let y = b0 * *x + z1;
                        st[ch] = (b1 * *x - a1 * y + z2, b2 * *x - a2 * y);
                        *x = y;
                    }
                }
            }
        }
    }
}
