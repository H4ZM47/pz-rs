//! Live output: pulls stereo blocks from a [`Studio`] and plays them on the
//! default device through cpal, resampling when the device does not run at
//! 48 kHz.

use crate::studio::{Studio, SAMPLE_RATE};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

/// Frames the studio mixes per call (FMOD's default DSP block is 1024).
const BLOCK: usize = 1024;

/// Keeps the device stream alive; dropping it stops playback.
pub struct Output {
    _stream: cpal::Stream,
    pub device_rate: u32,
    pub device_channels: u16,
}

/// Pulls 48 kHz stereo from the studio and converts it to the device rate.
struct Feeder {
    studio: Arc<Mutex<Studio>>,
    block: Vec<f32>,
    /// Read position in `block`, in frames, fractional for resampling.
    pos: f64,
    step: f64,
    prev: [f32; 2],
}

impl Feeder {
    fn refill(&mut self) {
        self.block.iter_mut().for_each(|s| *s = 0.0);
        if let Ok(mut s) = self.studio.lock() {
            s.mix(&mut self.block);
        }
    }

    fn frame(&mut self) -> [f32; 2] {
        let frames = self.block.len() / 2;
        while self.pos >= frames as f64 {
            self.prev = [
                self.block[(frames - 1) * 2],
                self.block[(frames - 1) * 2 + 1],
            ];
            self.pos -= frames as f64;
            self.refill();
        }
        let i = self.pos.floor() as usize;
        let t = (self.pos - i as f64) as f32;
        let a = if i == 0 {
            self.prev
        } else {
            [self.block[(i - 1) * 2], self.block[(i - 1) * 2 + 1]]
        };
        let b = [self.block[i * 2], self.block[i * 2 + 1]];
        self.pos += self.step;
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }
}

impl Output {
    pub fn start(studio: Arc<Mutex<Studio>>) -> Result<Output, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no audio output device")?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        let device_rate = config.sample_rate().0;
        let device_channels = config.channels();
        let mut feeder = Feeder {
            studio,
            block: vec![0.0; BLOCK * 2],
            pos: BLOCK as f64,
            step: SAMPLE_RATE as f64 / device_rate as f64,
            prev: [0.0; 2],
        };
        let ch = device_channels as usize;
        let err = |e| eprintln!("pz-audio: output stream error: {e}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config.into(),
                move |data: &mut [f32], _| {
                    for frame in data.chunks_mut(ch) {
                        let [l, r] = feeder.frame();
                        for (k, s) in frame.iter_mut().enumerate() {
                            *s = match k {
                                0 => l,
                                1 => r,
                                _ => 0.0,
                            };
                        }
                    }
                },
                err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &config.into(),
                move |data: &mut [i16], _| {
                    for frame in data.chunks_mut(ch) {
                        let [l, r] = feeder.frame();
                        for (k, s) in frame.iter_mut().enumerate() {
                            let v = match k {
                                0 => l,
                                1 => r,
                                _ => 0.0,
                            };
                            *s = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
                        }
                    }
                },
                err,
                None,
            ),
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Output {
            _stream: stream,
            device_rate,
            device_channels,
        })
    }
}
