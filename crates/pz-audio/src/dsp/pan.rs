//! `FMOD_DSP_TYPE_PAN` in the configuration Studio's spatializer uses
//! (3D, surround output in 5.1, then FMOD's default downmix to stereo).
//!
//! Measured against the original DSP:
//! * point sources are panned by 2D VBAP over the 5.1 ring (C 0°,
//!   FR/FL ±30°, SR/SL ±110°), power-normalised;
//! * distance attenuation follows the rolloff modes exactly
//!   (linear-squared, linear, inverse clamped at max, inverse-tapered =
//!   min(inverse, linear-squared), custom = none);
//! * the 5.1 → stereo downmix is L = FL + C/√2 + SL/√2 (likewise R).
//!
//! Approximate until the DSP code is ported: extent (spread) integrates
//! VBAP amplitudes over the arc, which matches FMOD for extents up to about
//! 60° and drifts above; the sound-size to extent mapping is a fit; FMOD's
//! smoothing of gain changes is not modelled; stereo input is folded to mono.

use super::Dsp;

/// 5.1 channel order: FL, FR, C, LFE, SL, SR.
const RING: [(f32, usize); 5] = [(0.0, 2), (30.0, 1), (110.0, 5), (250.0, 4), (330.0, 0)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rolloff {
    LinearSquared,
    Linear,
    Inverse,
    InverseTapered,
    Custom,
}

impl Rolloff {
    pub fn from_index(i: i32) -> Rolloff {
        match i {
            1 => Rolloff::Linear,
            2 => Rolloff::Inverse,
            3 => Rolloff::InverseTapered,
            4 => Rolloff::Custom,
            _ => Rolloff::LinearSquared,
        }
    }

    pub fn gain(self, d: f32, min: f32, max: f32) -> f32 {
        if d <= min {
            return 1.0;
        }
        let lin_sq = || {
            let t = if max > min {
                ((d - min) / (max - min)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            (1.0 - t) * (1.0 - t)
        };
        let inverse = || {
            if min <= 0.0 {
                1.0
            } else {
                min / d.min(max.max(min))
            }
        };
        match self {
            Rolloff::LinearSquared => lin_sq(),
            Rolloff::Linear => {
                if max > min {
                    1.0 - ((d - min) / (max - min)).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            Rolloff::Inverse => inverse(),
            Rolloff::InverseTapered => inverse().min(lin_sq()),
            Rolloff::Custom => 1.0,
        }
    }
}

/// Power-normalised VBAP gains for a direction (degrees, clockwise from
/// front) over the 5.1 ring.
pub fn vbap(azimuth: f32) -> [f32; 6] {
    let th = azimuth.rem_euclid(360.0);
    let mut g = [0.0; 6];
    for k in 0..5 {
        let (a1, i1) = RING[k];
        let (a2, i2) = RING[(k + 1) % 5];
        let a2 = if a2 <= a1 { a2 + 360.0 } else { a2 };
        let t = if th < a1 { th + 360.0 } else { th };
        if t >= a1 && t <= a2 {
            let (s1, c1) = a1.to_radians().sin_cos();
            let (s2, c2) = a2.to_radians().sin_cos();
            let (sp, cp) = t.to_radians().sin_cos();
            let det = s1 * c2 - c1 * s2;
            let g1 = (sp * c2 - cp * s2) / det;
            let g2 = (s1 * cp - c1 * sp) / det;
            let n = (g1 * g1 + g2 * g2).sqrt();
            g[i1] += g1 / n;
            g[i2] += g2 / n;
            return g;
        }
    }
    g
}

/// Gains for a source of angular width `extent` (degrees).
pub fn spread(azimuth: f32, extent: f32) -> [f32; 6] {
    if extent <= 0.5 {
        return vbap(azimuth);
    }
    let n = (extent.min(360.0) as usize).max(8);
    let mut acc = [0.0f32; 6];
    for k in 0..n {
        let th = azimuth - extent / 2.0 + extent * (k as f32 + 0.5) / n as f32;
        for (a, g) in acc.iter_mut().zip(vbap(th)) {
            *a += g;
        }
    }
    let norm = acc.iter().map(|x| x * x).sum::<f32>().sqrt();
    acc.map(|x| if norm > 0.0 { x / norm } else { 0.0 })
}

#[derive(Debug, Clone)]
pub struct Pan {
    pub rolloff: Rolloff,
    pub min_distance: f32,
    pub max_distance: f32,
    /// 0 auto, 1 user, 2 off.
    pub extent_mode: i32,
    pub sound_size: f32,
    pub min_extent: f32,
    /// Listener-relative position: x right, y up, z forward.
    pub position: [f32; 3],
}

impl Pan {
    pub fn new() -> Pan {
        Pan {
            rolloff: Rolloff::LinearSquared,
            min_distance: 1.0,
            max_distance: 20.0,
            extent_mode: 0,
            sound_size: 0.0,
            min_extent: 0.0,
            position: [0.0, 0.0, 0.0],
        }
    }

    fn extent(&self, distance: f32) -> f32 {
        if self.extent_mode == 2 {
            return 0.0;
        }
        let r = self.sound_size * 0.5;
        let from_size = if distance <= 0.0 || r <= 0.0 {
            0.0
        } else {
            (230.4 * r / distance - 43.2).clamp(0.0, 360.0)
        };
        from_size.max(self.min_extent)
    }

    /// Stereo gains (left, right) for a mono source.
    pub fn stereo_gains(&self) -> (f32, f32) {
        let [x, _, z] = self.position;
        let d = (x * x + self.position[1] * self.position[1] + z * z).sqrt();
        let att = self.rolloff.gain(d, self.min_distance, self.max_distance);
        let g = if d < 1e-6 {
            spread(0.0, 360.0)
        } else {
            spread(x.atan2(z).to_degrees(), self.extent(d))
        };
        let h = std::f32::consts::FRAC_1_SQRT_2;
        (
            att * (g[0] + h * g[2] + h * g[4]),
            att * (g[1] + h * g[2] + h * g[5]),
        )
    }
}

impl Default for Pan {
    fn default() -> Self {
        Pan::new()
    }
}

impl Dsp for Pan {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            12 => self.min_distance = value,
            13 => self.max_distance = value,
            15 => self.sound_size = value,
            16 => self.min_extent = value,
            _ => {}
        }
    }

    fn set_position(&mut self, relative: [f32; 3]) {
        self.position = relative;
    }

    fn process(&mut self, buf: &mut [f32], channels: usize) {
        let (l, r) = self.stereo_gains();
        for frame in buf.chunks_exact_mut(channels) {
            let m = frame.iter().sum::<f32>() / channels as f32;
            if channels >= 2 {
                frame[0] = m * l;
                frame[1] = m * r;
                frame[2..].iter_mut().for_each(|s| *s = 0.0);
            } else {
                frame[0] = m * (l + r) * 0.5;
            }
        }
    }
}
