//! Ports of the FMOD core DSP effects that the game's banks use.
//!
//! Each effect processes interleaved blocks in place and takes its
//! parameters in the order of the matching `FMOD_DSP_<TYPE>_*` indices.

pub mod compressor;
pub mod fader;
pub mod limiter;
pub mod multiband_eq;

/// A processing effect on interleaved audio.
pub trait Dsp: Send {
    fn process(&mut self, buf: &mut [f32], channels: usize);
}

use crate::bank::{effect_type, Effect, ParamValue};

/// Build the runtime DSP for a bank effect, or `None` for effects that do
/// not change the signal or are not ported yet.
pub fn from_effect(e: &Effect, rate: u32) -> Option<Box<dyn Dsp>> {
    let f = |i: usize| e.params.get(i).map_or(0.0, ParamValue::as_f32);
    let b = |i: usize| matches!(e.params.get(i), Some(ParamValue::Bool(true)));
    match e.kind {
        effect_type::FADER => Some(Box::new(fader::Fader { gain_db: f(0) })),
        effect_type::COMPRESSOR => {
            let mut c = compressor::Compressor::new(rate);
            for i in 0..5 {
                c.set_float(i, f(i));
            }
            c.linked = b(6);
            Some(Box::new(c))
        }
        effect_type::LIMITER => {
            let mut l = limiter::Limiter::new(rate);
            for i in 0..3 {
                l.set_float(i, f(i));
            }
            l.linked = b(3);
            Some(Box::new(l))
        }
        effect_type::MULTIBAND_EQ => {
            let mut m = multiband_eq::MultibandEq::new(rate);
            for i in 0..20 {
                m.set(i, f(i));
            }
            Some(Box::new(m))
        }
        _ => None,
    }
}
