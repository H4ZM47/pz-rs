//! The FMOD core DSP effects the game's banks use. Effects without an
//! implementation here pass the signal through.
//!
//! Each effect processes interleaved blocks in place and takes its
//! parameters in the order of the matching `FMOD_DSP_<TYPE>_*` indices.

pub mod fader;
pub mod pan;

/// A processing effect on interleaved audio.
pub trait Dsp: Send {
    fn process(&mut self, buf: &mut [f32], channels: usize);

    /// Listener-relative source position (x right, y up, z forward), for
    /// effects that spatialise.
    fn set_position(&mut self, _relative: [f32; 3]) {}

    /// Set a float parameter by its `FMOD_DSP_<TYPE>_*` index (automation).
    fn set_param(&mut self, _index: usize, _value: f32) {}
}

use crate::bank::{effect_type, Effect, ParamValue};

/// Build the runtime DSP for a bank effect, or `None` for effects that do
/// not change the signal or are not ported yet. `event_range` is the owning
/// event's min/max distance, used by spatialisers that do not override it.
pub fn from_effect(e: &Effect, _rate: u32, event_range: (f32, f32)) -> Option<Box<dyn Dsp>> {
    let f = |i: usize| e.params.get(i).map_or(0.0, ParamValue::as_f32);
    let b = |i: usize| matches!(e.params.get(i), Some(ParamValue::Bool(true)));
    match e.kind {
        effect_type::FADER => Some(Box::new(fader::Fader { gain_db: f(0) })),
        effect_type::PAN => {
            let i = |k: usize| match e.params.get(k) {
                Some(ParamValue::Int(v)) => *v,
                _ => 0,
            };
            let mut p = pan::Pan::new();
            p.rolloff = pan::Rolloff::from_index(i(11));
            (p.min_distance, p.max_distance) = if b(23) { (f(12), f(13)) } else { event_range };
            p.extent_mode = i(14);
            p.sound_size = f(15);
            p.min_extent = f(16);
            Some(Box::new(p))
        }
        _ => None,
    }
}
