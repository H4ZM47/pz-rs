//! `FMOD_DSP_TYPE_COMPRESSOR`.
//!
//! A feed-forward compressor on signal power: the squared input (summed over
//! channels when linked) runs through two cascaded one-pole smoothers that
//! use the attack coefficient while rising and the release coefficient while
//! falling. Above the threshold the gain is
//! `(env / threshold) ^ ((1 / ratio - 1) / 2)`, times the make-up gain.

use super::Dsp;

/// Time constant scale FMOD uses for attack and release coefficients.
const TIME_SCALE: f32 = -3.111_27;

#[derive(Debug, Clone)]
pub struct Compressor {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_db: f32,
    pub linked: bool,
    rate: f32,
    env1: [f32; 32],
    env2: [f32; 32],
}

impl Compressor {
    pub fn new(rate: u32) -> Compressor {
        Compressor {
            threshold_db: 0.0,
            ratio: 2.5,
            attack_ms: 20.0,
            release_ms: 100.0,
            makeup_db: 0.0,
            linked: true,
            rate: rate as f32,
            env1: [0.0; 32],
            env2: [0.0; 32],
        }
    }

    /// Set from the effect's parameter list (threshold, ratio, attack,
    /// release, make-up, side-chain, linked).
    pub fn set_float(&mut self, index: usize, v: f32) {
        match index {
            0 => self.threshold_db = v,
            1 => self.ratio = v,
            2 => self.attack_ms = v,
            3 => self.release_ms = v,
            4 => self.makeup_db = v,
            _ => {}
        }
    }

    fn coef(&self, ms: f32) -> f32 {
        1.0 - (TIME_SCALE / (ms / 1000.0 * self.rate)).exp()
    }
}

impl Dsp for Compressor {
    fn process(&mut self, buf: &mut [f32], channels: usize) {
        let channels = channels.min(32);
        let threshold = 10f32.powf(self.threshold_db / 10.0);
        let inv_threshold = 1.0 / threshold;
        let exponent = (1.0 / self.ratio - 1.0) * 0.5;
        let makeup = if self.makeup_db <= -80.0 {
            0.0
        } else {
            10f32.powf(self.makeup_db / 20.0)
        };
        let attack = self.coef(self.attack_ms);
        let release = self.coef(self.release_ms);
        let gain_for = |env: f32| {
            if env > threshold {
                (env * inv_threshold).powf(exponent) * makeup
            } else {
                makeup
            }
        };
        for frame in buf.chunks_exact_mut(channels) {
            if self.linked {
                let x: f32 = frame.iter().map(|s| s * s).sum();
                let c = if x > self.env2[0] { attack } else { release };
                self.env1[0] += c * (x - self.env1[0]);
                self.env2[0] += c * (self.env1[0] - self.env2[0]);
                let g = gain_for(self.env2[0]);
                frame.iter_mut().for_each(|s| *s *= g);
            } else {
                for (ch, s) in frame.iter_mut().enumerate() {
                    let x = *s * *s;
                    let c = if x > self.env2[ch] { attack } else { release };
                    self.env1[ch] += c * (x - self.env1[ch]);
                    self.env2[ch] += c * (self.env1[ch] - self.env2[ch]);
                    *s *= gain_for(self.env2[ch]);
                }
            }
        }
    }
}
