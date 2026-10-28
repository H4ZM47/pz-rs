//! FMOD core sounds and channels: loose files played outside the Studio
//! banks (`System::createSound` / `playSound`). They mix straight into the
//! master output, after the Studio buses.

use crate::dsp::pan::{Pan, Rolloff};
use crate::sound_file::SoundFile;
use crate::studio::Pcm;
use std::sync::Arc;

/// Output sample rate the mixer runs at.
const OUTPUT_RATE: f64 = 48000.0;

struct Sound {
    pcm: Arc<Pcm>,
    rate: u32,
    looped: bool,
    is_3d: bool,
}

struct Channel {
    sound: Arc<Pcm>,
    /// Source frames per output frame at pitch 1.
    step: f64,
    pos: f64,
    volume: f32,
    pitch: f32,
    paused: bool,
    looped: bool,
    /// Mode and the last submitted position are independent.
    is_3d: bool,
    position: [f32; 3],
    level: f32,
    min_distance: f32,
    max_distance: f32,
    /// In the in-game non-bank group, which pauses as a whole.
    non_bank: bool,
    done: bool,
}

#[derive(Default)]
pub struct Channels {
    sounds: Vec<Option<Sound>>,
    /// Slots with a generation, so a stale id never reaches a reused slot.
    channels: Vec<(u32, Option<Channel>)>,
    /// Master channel group volume.
    pub master_volume: Option<f32>,
    pub non_bank_paused: bool,
}

pub type SoundId = usize;
/// Slot in the low 32 bits, generation above.
pub type ChannelId = u64;

impl Channels {
    /// New channels inherit the sound's initial spatial mode.
    pub fn add_sound(&mut self, file: SoundFile, looped: bool, is_3d: bool) -> SoundId {
        self.sounds.push(Some(Sound {
            pcm: Arc::new(file.pcm),
            rate: file.rate,
            looped,
            is_3d,
        }));
        self.sounds.len() - 1
    }

    pub fn release_sound(&mut self, id: SoundId) {
        if let Some(s) = self.sounds.get_mut(id) {
            *s = None;
        }
    }

    /// `System::playSound`: a new channel at full volume, inheriting its sound's mode.
    pub fn play(&mut self, id: SoundId, paused: bool) -> Option<ChannelId> {
        let s = self.sounds.get(id)?.as_ref()?;
        let ch = Channel {
            sound: s.pcm.clone(),
            step: s.rate as f64 / OUTPUT_RATE,
            pos: 0.0,
            volume: 1.0,
            pitch: 1.0,
            paused,
            looped: s.looped,
            is_3d: s.is_3d,
            position: [0.0; 3],
            level: 1.0,
            min_distance: 1.0,
            max_distance: 10000.0,
            non_bank: false,
            done: false,
        };
        let i = match self.channels.iter().position(|c| c.1.is_none()) {
            Some(i) => i,
            None => {
                self.channels.push((0, None));
                self.channels.len() - 1
            }
        };
        let slot = &mut self.channels[i];
        slot.0 = slot.0.wrapping_add(1);
        slot.1 = Some(ch);
        Some((slot.0 as u64) << 32 | i as u64)
    }

    fn slot(&self, id: ChannelId) -> Option<&Channel> {
        let (g, c) = self.channels.get(id as u32 as usize)?;
        (*g == (id >> 32) as u32).then_some(c.as_ref()?)
    }

    fn get(&mut self, id: ChannelId) -> Option<&mut Channel> {
        let (g, c) = self.channels.get_mut(id as u32 as usize)?;
        if *g != (id >> 32) as u32 {
            return None;
        }
        c.as_mut().filter(|c| !c.done)
    }

    pub fn set_volume(&mut self, id: ChannelId, v: f32) {
        if let Some(c) = self.get(id) {
            c.volume = v;
        }
    }
    pub fn set_pitch(&mut self, id: ChannelId, p: f32) {
        if let Some(c) = self.get(id) {
            c.pitch = p.max(0.0);
        }
    }
    pub fn set_paused(&mut self, id: ChannelId, p: bool) {
        if let Some(c) = self.get(id) {
            c.paused = p;
        }
    }
    pub fn set_looped(&mut self, id: ChannelId, l: bool) {
        if let Some(c) = self.get(id) {
            c.looped = l;
        }
    }
    /// `FMOD_2D` or `FMOD_3D`, retaining the last submitted position.
    pub fn set_3d(&mut self, id: ChannelId, on: bool) {
        if let Some(c) = self.get(id) {
            c.is_3d = on;
        }
    }
    pub fn set_position(&mut self, id: ChannelId, p: [f32; 3]) {
        if let Some(c) = self.get(id) {
            c.position = p;
        }
    }
    pub fn set_3d_level(&mut self, id: ChannelId, level: f32) {
        if let Some(c) = self.get(id) {
            c.level = level.clamp(0.0, 1.0);
        }
    }
    pub fn set_min_max_distance(&mut self, id: ChannelId, min: f32, max: f32) {
        if let Some(c) = self.get(id) {
            c.min_distance = min;
            c.max_distance = max;
        }
    }
    pub fn set_non_bank(&mut self, id: ChannelId) {
        if let Some(c) = self.get(id) {
            c.non_bank = true;
        }
    }
    pub fn stop(&mut self, id: ChannelId) {
        if self.slot(id).is_some() {
            self.channels[id as u32 as usize].1 = None;
        }
    }
    pub fn stop_all(&mut self) {
        for c in &mut self.channels {
            c.1 = None;
        }
    }
    pub fn is_playing(&self, id: ChannelId) -> bool {
        self.slot(id).is_some_and(|c| !c.done)
    }
    pub fn position_ms(&self, id: ChannelId) -> u32 {
        self.slot(id)
            .map_or(0, |c| (c.pos * 1000.0 / (c.step * OUTPUT_RATE)) as u32)
    }

    /// Mix every playing channel into `out` (stereo, added). `relative`
    /// maps a world position to listener space.
    pub fn mix(&mut self, out: &mut [f32], relative: impl Fn([f32; 3]) -> [f32; 3]) {
        let master = self.master_volume.unwrap_or(1.0);
        let frames = out.len() / 2;
        for (_, slot) in &mut self.channels {
            let Some(c) = slot else { continue };
            if c.done || c.paused || (c.non_bank && self.non_bank_paused) {
                continue;
            }
            let (mut gl, mut gr) = (1.0, 1.0);
            if c.is_3d {
                let pan = Pan {
                    rolloff: Rolloff::Inverse,
                    min_distance: c.min_distance,
                    max_distance: c.max_distance,
                    extent_mode: 2,
                    position: relative(c.position),
                    ..Pan::new()
                };
                let (l, r) = pan.stereo_gains();
                gl = (1.0 - c.level) + c.level * l;
                gr = (1.0 - c.level) + c.level * r;
            }
            let v = c.volume * master;
            let ch = c.sound.channels as usize;
            let n = c.sound.frames();
            let step = c.step * c.pitch as f64;
            for f in 0..frames {
                if c.pos >= n as f64 {
                    if c.looped && n > 0 {
                        c.pos %= n as f64;
                    } else {
                        c.done = true;
                        break;
                    }
                }
                let i = c.pos as usize;
                let t = (c.pos - i as f64) as f32;
                let j = if i + 1 < n {
                    i + 1
                } else if c.looped {
                    0
                } else {
                    i
                };
                let at = |k: usize, s: usize| c.sound.data[k * ch + s.min(ch - 1)];
                let l = at(i, 0) + (at(j, 0) - at(i, 0)) * t;
                let r = at(i, 1) + (at(j, 1) - at(i, 1)) * t;
                out[f * 2] += l * v * gl;
                out[f * 2 + 1] += r * v * gr;
                c.pos += step;
            }
        }
    }

    /// Free the slots of finished channels (`is_playing` is then false).
    pub fn reap(&mut self) {
        for (_, c) in &mut self.channels {
            if c.as_ref().is_some_and(|c| c.done) {
                *c = None;
            }
        }
    }
}
