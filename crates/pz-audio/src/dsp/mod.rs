//! Ports of the FMOD core DSP effects that the game's banks use.
//!
//! Each effect processes interleaved blocks in place and takes its
//! parameters in the order of the matching `FMOD_DSP_<TYPE>_*` indices.

pub mod compressor;
pub mod limiter;

/// A processing effect on interleaved audio.
pub trait Dsp: Send {
    fn process(&mut self, buf: &mut [f32], channels: usize);
}
