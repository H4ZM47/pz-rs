//! `FMOD_DSP_TYPE_LIMITER`.
//!
//! A peak follower that jumps up instantly and decays by
//! `release / (release + 1 / rate)` per frame; the output gain is
//! `min(maximizer, ceiling / envelope)`. Linked mode shares one envelope
//! across channels, updated sample by sample within each frame.

use super::Dsp;

#[derive(Debug, Clone)]
pub struct Limiter {
    pub release_ms: f32,
    pub ceiling_db: f32,
    pub maximizer_db: f32,
    pub linked: bool,
    rate: f32,
    env: [f32; 32],
}

fn db_gain(db: f32) -> f32 {
    if db <= -80.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

impl Limiter {
    pub fn new(rate: u32) -> Limiter {
        Limiter {
            release_ms: 10.0,
            ceiling_db: 0.0,
            maximizer_db: 0.0,
            linked: false,
            rate: rate as f32,
            env: [0.0; 32],
        }
    }

    pub fn set_float(&mut self, index: usize, v: f32) {
        match index {
            0 => self.release_ms = v,
            1 => self.ceiling_db = v,
            2 => self.maximizer_db = v,
            _ => {}
        }
    }

    pub fn set_bool(&mut self, index: usize, v: bool) {
        if index == 3 {
            self.linked = v;
        }
    }
}

impl Dsp for Limiter {
    fn process(&mut self, buf: &mut [f32], channels: usize) {
        let channels = channels.min(32);
        let ceiling = db_gain(self.ceiling_db);
        let max_gain = db_gain(self.maximizer_db);
        let t = self.release_ms / 1000.0;
        let decay = t / (t + 1.0 / self.rate);
        for frame in buf.chunks_exact_mut(channels) {
            if self.linked {
                self.env[0] *= decay;
            }
            for (ch, s) in frame.iter_mut().enumerate() {
                let e = if self.linked { 0 } else { ch };
                if !self.linked {
                    self.env[e] *= decay;
                }
                let a = s.abs();
                if a > self.env[e] {
                    self.env[e] = a;
                }
                *s *= max_gain.min(ceiling / self.env[e]);
            }
        }
    }
}
