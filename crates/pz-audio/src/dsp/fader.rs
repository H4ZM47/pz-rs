//! `FMOD_DSP_TYPE_FADER`: a gain in dB.

use super::Dsp;

#[derive(Debug, Clone)]
pub struct Fader {
    pub gain_db: f32,
}

impl Dsp for Fader {
    fn set_param(&mut self, index: usize, value: f32) {
        if index == 0 {
            self.gain_db = value;
        }
    }

    fn process(&mut self, buf: &mut [f32], _channels: usize) {
        let g = if self.gain_db <= -80.0 {
            0.0
        } else {
            10f32.powf(self.gain_db / 20.0)
        };
        buf.iter_mut().for_each(|s| *s *= g);
    }
}
