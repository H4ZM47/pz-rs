//! A Studio runtime in the shape of `FMOD::Studio::System`: load banks,
//! create event instances, and mix them to interleaved stereo at 48 kHz.
//!
//! This first version plays timelines, action sheets and nested multi
//! instruments with instrument and bus volumes. Effects, automation,
//! modulation, parameters sheets, 3D and snapshots come next.

use crate::bank::{BankModel, BusKind, Event, Instrument, InstrumentKind};
use crate::reader::Guid;
use crate::vorbis_setup::SetupHeaders;
use crate::BankFile;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub const SAMPLE_RATE: u32 = 48000;

pub fn db_to_gain(db: f32) -> f32 {
    if db <= -80.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

struct LoadedBank {
    file: BankFile,
    model: BankModel,
}

/// Decoded waveform, interleaved.
pub struct Pcm {
    pub channels: u16,
    pub data: Vec<f32>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.max(1) as usize
    }
}

pub struct Studio {
    banks: Vec<LoadedBank>,
    setups: SetupHeaders,
    pcm_cache: HashMap<(usize, u32), Arc<Pcm>>,
    instances: Vec<EventInstance>,
    rng: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Sustaining,
    Stopped,
    Starting,
    Stopping,
}

enum Start {
    Wave {
        bank: usize,
        subsound: u32,
        gain: f32,
        delay_ms: f32,
    },
    Event {
        event: Guid,
        gain: f32,
        delay_ms: f32,
    },
}

struct Voice {
    pcm: Arc<Pcm>,
    pos: usize,
    gain: f32,
    /// Frames until the voice starts.
    delay: usize,
    /// Timeline frames left before the placement ends (async instruments ignore it).
    remaining: Option<usize>,
}

pub struct EventInstance {
    pub event: Guid,
    bank: usize,
    position: u64,
    started: bool,
    pub state: PlaybackState,
    voices: Vec<Voice>,
    gain: f32,
    /// Timeline placements already triggered.
    triggered: Vec<bool>,
    /// Set for nested events: their output joins the parent's track.
    parent: Option<InstanceId>,
    /// Gain of the routing below the event (cached per block).
    route: f32,
    start_delay: usize,
}

pub type InstanceId = usize;

impl Studio {
    pub fn new(setups: SetupHeaders) -> Studio {
        Studio {
            banks: Vec::new(),
            setups,
            pcm_cache: HashMap::new(),
            instances: Vec::new(),
            rng: 0x9e37_79b9_7f4a_7c15,
        }
    }

    pub fn load_bank(&mut self, path: &Path) -> std::io::Result<()> {
        let file = BankFile::open(path)?;
        let model = file
            .model()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.banks.push(LoadedBank { file, model });
        Ok(())
    }

    fn find_event(&self, id: &Guid) -> Option<(usize, &Event)> {
        self.banks
            .iter()
            .enumerate()
            .find_map(|(i, b)| b.model.events.get(id).map(|e| (i, e)))
    }

    fn find_instrument(&self, id: &Guid) -> Option<(usize, &Instrument)> {
        self.banks
            .iter()
            .enumerate()
            .find_map(|(i, b)| b.model.instruments.get(id).map(|x| (i, x)))
    }

    pub fn create_instance(&mut self, event: Guid) -> Option<InstanceId> {
        let (bank, e) = self.find_event(&event)?;
        let n = self.banks[bank]
            .model
            .timelines
            .get(&e.timeline)
            .map_or(0, |t| t.instruments.len() + t.instruments_2.len());
        self.instances.push(EventInstance {
            event,
            bank,
            position: 0,
            started: false,
            state: PlaybackState::Stopped,
            voices: Vec::new(),
            gain: 1.0,
            triggered: vec![false; n],
            parent: None,
            route: 1.0,
            start_delay: 0,
        });
        Some(self.instances.len() - 1)
    }

    pub fn start(&mut self, id: InstanceId) {
        let i = &mut self.instances[id];
        i.state = PlaybackState::Playing;
        i.started = false;
        i.position = 0;
    }

    pub fn set_volume(&mut self, id: InstanceId, v: f32) {
        self.instances[id].gain = v;
    }

    pub fn state(&self, id: InstanceId) -> PlaybackState {
        self.instances[id].state
    }

    fn random(&mut self) -> f32 {
        // xorshift64*: deterministic, good enough for playlist choices.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        ((self.rng.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u64 << 24) as f32
    }

    fn pcm(&mut self, bank: usize, subsound: u32) -> Option<Arc<Pcm>> {
        if let Some(p) = self.pcm_cache.get(&(bank, subsound)) {
            return Some(p.clone());
        }
        let b = &self.banks[bank].file;
        let dec = b.decoder(subsound as usize, &self.setups).ok()?;
        let channels = dec.channels;
        let data = dec.decode_all().ok()?;
        let p = Arc::new(Pcm { channels, data });
        self.pcm_cache.insert((bank, subsound), p.clone());
        Some(p)
    }

    /// Resolve an instrument to the voices it starts, with its gain applied.
    fn instrument_voices(&mut self, id: &Guid, gain: f32, depth: u32, out: &mut Vec<Start>) {
        if depth > 8 {
            return;
        }
        let Some((bank, inst)) = self.find_instrument(id) else {
            return;
        };
        let inst = inst.clone();
        if inst.base.probability < 100.0 && self.random() * 100.0 >= inst.base.probability {
            return;
        }
        let gain = gain * db_to_gain(inst.base.volume_db);
        let delay_ms =
            inst.base.delay_min + (inst.base.delay_max - inst.base.delay_min) * self.random();
        match &inst.kind {
            InstrumentKind::Single { waveform } => {
                if let Some(w) = self.banks[bank].model.waveforms.get(waveform) {
                    out.push(Start::Wave {
                        bank,
                        subsound: w.subsound,
                        gain,
                        delay_ms,
                    });
                }
            }
            InstrumentKind::Multi { playlist } | InstrumentKind::Scatterer { playlist, .. } => {
                let total: f32 = playlist.entries.iter().map(|e| e.1).sum();
                let mut pick = self.random() * total;
                for (g, w) in &playlist.entries {
                    if pick < *w {
                        self.instrument_voices(g, gain, depth + 1, out);
                        break;
                    }
                    pick -= w;
                }
            }
            InstrumentKind::Event { event, .. } => out.push(Start::Event {
                event: *event,
                gain,
                delay_ms,
            }),
            _ => {}
        }
    }

    fn start_instrument(&mut self, inst: InstanceId, id: Guid, remaining: Option<usize>) {
        let mut v = Vec::new();
        self.instrument_voices(&id, 1.0, 0, &mut v);
        for start in v {
            match start {
                Start::Wave {
                    bank,
                    subsound,
                    gain,
                    delay_ms,
                } => {
                    if let Some(pcm) = self.pcm(bank, subsound) {
                        let delay = (delay_ms.max(0.0) * SAMPLE_RATE as f32 / 1000.0) as usize;
                        self.instances[inst].voices.push(Voice {
                            pcm,
                            pos: 0,
                            gain,
                            delay,
                            remaining,
                        });
                    }
                }
                Start::Event {
                    event,
                    gain,
                    delay_ms,
                } => {
                    let Some(child) = self.create_instance(event) else {
                        continue;
                    };
                    let parent_gain = self.instances[inst].gain * self.instances[inst].route;
                    let c = &mut self.instances[child];
                    c.parent = Some(inst);
                    c.gain = gain * parent_gain;
                    c.start_delay = (delay_ms.max(0.0) * SAMPLE_RATE as f32 / 1000.0) as usize;
                    c.state = PlaybackState::Playing;
                }
            }
        }
    }

    /// Gain of the path from an event to the output: the event's master
    /// bus and input bus, then the mixer group chain.
    fn routing_gain(&self, bank: usize, e: &Event, track: Option<Guid>) -> f32 {
        let mut g = 1.0;
        let mut seen = 0;
        let mut cur = track.unwrap_or(e.master_bus);
        let mut visited_master = track.is_none();
        loop {
            seen += 1;
            if seen > 32 {
                break;
            }
            let found = self.banks.iter().find_map(|b| b.model.buses.get(&cur));
            let Some((kind, bus)) = found else { break };
            g *= db_to_gain(bus.body.volume_db);
            if *kind == BusKind::Master && cur == e.master_bus {
                visited_master = true;
                cur = e.input_bus;
                continue;
            }
            if bus.output.target == Guid::NULL {
                if !visited_master {
                    cur = e.master_bus;
                    continue;
                }
                break;
            }
            cur = bus.output.target;
        }
        let _ = bank;
        g
    }

    /// Mix `frames` frames of stereo into `out` (interleaved, added).
    pub fn mix(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut id = 0;
        while id < self.instances.len() {
            let this = id;
            id += 1;
            let id = this;
            if self.instances[id].state != PlaybackState::Playing {
                continue;
            }
            if self.instances[id].start_delay >= frames {
                self.instances[id].start_delay -= frames;
                continue;
            }
            let (bank, event) = {
                let i = &self.instances[id];
                (i.bank, self.banks[i.bank].model.events[&i.event].clone())
            };
            let route = if self.instances[id].parent.is_some() {
                // Only the nested event's own master bus applies.
                self.banks
                    .iter()
                    .find_map(|b| b.model.buses.get(&event.master_bus))
                    .map_or(1.0, |b| db_to_gain(b.1.body.volume_db))
            } else {
                self.routing_gain(bank, &event, None)
            };
            self.instances[id].route = route;
            if !self.instances[id].started {
                self.instances[id].started = true;
                for g in event.instruments.clone() {
                    self.start_instrument(id, g, None);
                }
            }
            // Timeline triggers within this block.
            if let Some(t) = self.banks[bank]
                .model
                .timelines
                .get(&event.timeline)
                .cloned()
            {
                let pos = self.instances[id].position;
                for (k, p) in t.instruments.iter().chain(&t.instruments_2).enumerate() {
                    if !self.instances[id].triggered[k] && (p.start as u64) < pos + frames as u64 {
                        self.instances[id].triggered[k] = true;
                        let offset = (p.start as u64).saturating_sub(pos) as usize;
                        let before = self.instances[id].voices.len();
                        self.start_instrument(id, p.instrument, Some(p.length as usize));
                        for v in &mut self.instances[id].voices[before..] {
                            v.delay += offset;
                        }
                    }
                }
            }
            let inst = &mut self.instances[id];
            let g = inst.gain * route;
            for v in &mut inst.voices {
                let ch = v.pcm.channels as usize;
                for f in 0..frames {
                    if v.delay > 0 {
                        v.delay -= 1;
                        continue;
                    }
                    if v.pos >= v.pcm.frames() || v.remaining == Some(0) {
                        break;
                    }
                    if let Some(r) = &mut v.remaining {
                        *r -= 1;
                    }
                    let (l, r) = if ch == 1 {
                        let s = v.pcm.data[v.pos];
                        (s, s)
                    } else {
                        (v.pcm.data[v.pos * ch], v.pcm.data[v.pos * ch + 1])
                    };
                    out[f * 2] += l * v.gain * g;
                    out[f * 2 + 1] += r * v.gain * g;
                    v.pos += 1;
                }
            }
            inst.voices
                .retain(|v| v.pos < v.pcm.frames() && v.remaining != Some(0));
            inst.position += frames as u64;
            let end = self.banks[bank]
                .model
                .timelines
                .get(&event.timeline)
                .map_or(0, |t| t.end() as u64);
            if inst.voices.is_empty() && inst.position >= end {
                inst.state = PlaybackState::Stopped;
            }
        }
    }

    pub fn lookup_event(&self, strings: &crate::strings::StringTable, path: &str) -> Option<Guid> {
        strings.lookup(path)
    }
}

impl Studio {
    #[doc(hidden)]
    pub fn debug_instance(&self, id: InstanceId) -> String {
        let i = &self.instances[id];
        let e = &self.banks[i.bank].model.events[&i.event];
        format!(
            "voices={} state={:?} pos={} route={} inst={} tl={:?}",
            i.voices.len(),
            i.state,
            i.position,
            self.routing_gain(i.bank, e, None),
            e.instruments.len(),
            self.banks[i.bank]
                .model
                .timelines
                .get(&e.timeline)
                .map(|t| t.instruments.len())
        )
    }
}
