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
    bus_buffers: HashMap<Guid, Vec<f32>>,
    /// Listener position, forward and up vectors.
    listener: [[f32; 3]; 3],
    /// Global parameter values by parameter GUID.
    global_params: HashMap<Guid, f32>,
    /// Runtime state of each bus effect (`None`: passes the signal through).
    dsps: HashMap<Guid, Option<Box<dyn crate::dsp::Dsp>>>,
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
        track: Guid,
    },
    Event {
        event: Guid,
        gain: f32,
        delay_ms: f32,
    },
}

struct Voice {
    pcm: Arc<Pcm>,
    /// Event bus the voice plays into (an audio track).
    track: Guid,
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
    /// World position of the event (FMOD's left-handed axes).
    pub world_position: [f32; 3],
    /// Runtime state of the event's own bus effects.
    dsps: HashMap<Guid, Option<Box<dyn crate::dsp::Dsp>>>,
    /// Local parameter values by parameter GUID.
    params: HashMap<Guid, f32>,
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
            bus_buffers: HashMap::new(),
            listener: [[0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
            global_params: HashMap::new(),
            dsps: HashMap::new(),
        }
    }

    /// `Studio::System::setListenerAttributes` (position, forward, up).
    pub fn set_listener(&mut self, position: [f32; 3], forward: [f32; 3], up: [f32; 3]) {
        self.listener = [position, forward, up];
    }

    /// `EventInstance::set3DAttributes` (position only for now).
    pub fn set_position(&mut self, id: InstanceId, position: [f32; 3]) {
        self.instances[id].world_position = position;
    }

    fn find_parameter(&self, id: &Guid) -> Option<&crate::bank::Parameter> {
        self.banks.iter().find_map(|b| b.model.parameters.get(id))
    }

    /// `EventInstance::setParameterByName` (case-insensitive).
    pub fn set_parameter_by_name(&mut self, id: InstanceId, name: &str, value: f32) -> bool {
        let e = self.banks[self.instances[id].bank].model.events[&self.instances[id].event].clone();
        for sheet in &e.parameter_sheets {
            let Some(s) = self
                .banks
                .iter()
                .find_map(|b| b.model.parameter_sheets.get(sheet))
            else {
                continue;
            };
            let Some(p) = self.find_parameter(&s.parameter) else {
                continue;
            };
            if p.name.eq_ignore_ascii_case(name) {
                let v = value.clamp(p.minimum, p.maximum);
                let pid = p.id;
                if p.flags & crate::bank::Parameter::GLOBAL != 0 {
                    self.global_params.insert(pid, v);
                } else {
                    self.instances[id].params.insert(pid, v);
                }
                return true;
            }
        }
        false
    }

    /// `Studio::System::setParameterByName` for global parameters.
    pub fn set_global_parameter(&mut self, name: &str, value: f32) -> bool {
        let found = self
            .banks
            .iter()
            .flat_map(|b| b.model.parameters.values())
            .find(|p| {
                p.flags & crate::bank::Parameter::GLOBAL != 0 && p.name.eq_ignore_ascii_case(name)
            })
            .map(|p| (p.id, value.clamp(p.minimum, p.maximum)));
        if let Some((id, v)) = found {
            self.global_params.insert(id, v);
        }
        found.is_some()
    }

    /// Current value of a parameter for an instance, computing the built-in
    /// automatic parameters from the 3D position.
    fn parameter_value(
        &self,
        id: InstanceId,
        p: &crate::bank::Parameter,
        e: &Event,
        rel: [f32; 3],
    ) -> f32 {
        let [x, y, z] = rel;
        let horiz = (x * x + z * z).sqrt();
        let dist = (horiz * horiz + y * y).sqrt();
        let v = match p.kind {
            1 => dist,
            4 => x.atan2(z).to_degrees(),
            5 => y.atan2(horiz).to_degrees(),
            9 => {
                if e.max_distance > e.min_distance {
                    ((dist - e.min_distance) / (e.max_distance - e.min_distance)).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            _ if p.flags & crate::bank::Parameter::GLOBAL != 0 => {
                *self.global_params.get(&p.id).unwrap_or(&p.default)
            }
            _ => *self.instances[id].params.get(&p.id).unwrap_or(&p.default),
        };
        v.clamp(p.minimum, p.maximum)
    }

    /// Values of every controller active for an instance: controller GUID →
    /// value of the parameter driving it.
    fn controller_inputs(&self, id: InstanceId, e: &Event, rel: [f32; 3]) -> HashMap<Guid, f32> {
        let mut out = HashMap::new();
        for sheet in &e.parameter_sheets {
            let Some((s, ctrls)) = self.banks.iter().find_map(|b| {
                Some((
                    b.model.parameter_sheets.get(sheet)?,
                    b.model.sheet_controllers.get(sheet)?,
                ))
            }) else {
                continue;
            };
            let Some(p) = self.find_parameter(&s.parameter) else {
                continue;
            };
            let v = self.parameter_value(id, p, e, rel);
            for c in ctrls {
                out.insert(*c, v);
            }
        }
        out
    }

    /// Automated value of `property` on `target`, if any controller drives it.
    fn automated(&self, target: &Guid, property: u32, inputs: &HashMap<Guid, f32>) -> Option<f32> {
        let props = self
            .banks
            .iter()
            .find_map(|b| b.model.properties.get(target))?;
        let prop = props.iter().find(|p| p.property == property)?;
        let mut total = None;
        for cg in &prop.curves {
            let Some(curve) = self.banks.iter().find_map(|b| b.model.curves.get(cg)) else {
                continue;
            };
            let Some(v) = inputs.get(&curve.controller) else {
                continue;
            };
            *total.get_or_insert(0.0) += curve.eval(*v);
        }
        let ui = total?;
        Some(
            match self
                .banks
                .iter()
                .find_map(|b| b.model.mappings.get(&prop.mapping))
            {
                Some(m) => m.apply(ui),
                None => ui,
            },
        )
    }

    fn relative(&self, p: [f32; 3]) -> [f32; 3] {
        let [lp, f, u] = self.listener;
        let d = [p[0] - lp[0], p[1] - lp[1], p[2] - lp[2]];
        // Left-handed: right = up x forward.
        let r = [
            u[1] * f[2] - u[2] * f[1],
            u[2] * f[0] - u[0] * f[2],
            u[0] * f[1] - u[1] * f[0],
        ];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        [dot(d, r), dot(d, u), dot(d, f)]
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
            world_position: [0.0; 3],
            dsps: HashMap::new(),
            params: HashMap::new(),
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
    fn instrument_voices(
        &mut self,
        id: &Guid,
        gain: f32,
        track: Guid,
        depth: u32,
        out: &mut Vec<Start>,
    ) {
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
        let track = match inst.base.output {
            Some(o) if track == Guid::NULL => o.target,
            _ => track,
        };
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
                        track,
                    });
                }
            }
            InstrumentKind::Multi { playlist } | InstrumentKind::Scatterer { playlist, .. } => {
                let total: f32 = playlist.entries.iter().map(|e| e.1).sum();
                let mut pick = self.random() * total;
                for (g, w) in &playlist.entries {
                    if pick < *w {
                        self.instrument_voices(g, gain, track, depth + 1, out);
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
        self.instrument_voices(&id, 1.0, Guid::NULL, 0, &mut v);
        for start in v {
            match start {
                Start::Wave {
                    bank,
                    subsound,
                    gain,
                    delay_ms,
                    track,
                } => {
                    if let Some(pcm) = self.pcm(bank, subsound) {
                        let delay = (delay_ms.max(0.0) * SAMPLE_RATE as f32 / 1000.0) as usize;
                        self.instances[inst].voices.push(Voice {
                            pcm,
                            track,
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

    fn find_bus(&self, id: &Guid) -> Option<&(BusKind, crate::bank::Bus)> {
        self.banks.iter().find_map(|b| b.model.buses.get(id))
    }

    /// Gain inside the event: its master bus volume (tracks and event
    /// effects are not modelled yet).
    fn event_gain(&self, e: &Event) -> f32 {
        self.find_bus(&e.master_bus)
            .map_or(1.0, |b| db_to_gain(b.1.body.volume_db))
    }

    /// Run an event's own mixer: tracks into the event's master bus, each
    /// bus applying its effects and volume. Writes the master's output to `out`.
    fn run_event_buses(
        &mut self,
        id: InstanceId,
        e: &Event,
        mut bufs: HashMap<Guid, Vec<f32>>,
        out: &mut [f32],
        rel: [f32; 3],
    ) {
        out.iter_mut().for_each(|s| *s = 0.0);
        for _ in 0..16 {
            let Some(g) = bufs.keys().copied().find(|g| *g != e.master_bus) else {
                break;
            };
            let mut buf = bufs.remove(&g).unwrap();
            let target = match self.find_bus(&g) {
                Some((_, bus)) if bus.output.target != Guid::NULL => {
                    let bus = bus.clone();
                    self.event_bus_effects(id, e, &bus, &mut buf, rel);
                    bus.output.target
                }
                _ => e.master_bus,
            };
            let dst = bufs.entry(target).or_insert_with(|| vec![0.0; buf.len()]);
            dst.iter_mut().zip(&buf).for_each(|(o, s)| *o += s);
        }
        if let Some(mut buf) = bufs.remove(&e.master_bus) {
            if let Some((_, bus)) = self.find_bus(&e.master_bus) {
                let bus = bus.clone();
                self.event_bus_effects(id, e, &bus, &mut buf, rel);
            }
            out.copy_from_slice(&buf);
        }
    }

    fn event_bus_effects(
        &mut self,
        id: InstanceId,
        e: &Event,
        bus: &crate::bank::Bus,
        buf: &mut [f32],
        rel: [f32; 3],
    ) {
        let inputs = self.controller_inputs(id, e, rel);
        for fx in bus.body.effects.iter().chain(&bus.body.effects_2) {
            let n = self
                .banks
                .iter()
                .find_map(|b| b.model.effects.get(fx))
                .map_or(0, |x| x.params.len());
            let auto: Vec<(usize, f32)> = (0..n)
                .filter_map(|i| {
                    Some((
                        i,
                        self.automated(
                            fx,
                            crate::bank::property::EFFECT_PARAMETER + i as u32,
                            &inputs,
                        )?,
                    ))
                })
                .collect();
            if !self.instances[id].dsps.contains_key(fx) {
                let made = self
                    .banks
                    .iter()
                    .find_map(|b| b.model.effects.get(fx))
                    .and_then(|x| {
                        if x.bypass {
                            None
                        } else {
                            crate::dsp::from_effect(
                                x,
                                SAMPLE_RATE,
                                (e.min_distance, e.max_distance),
                            )
                        }
                    });
                self.instances[id].dsps.insert(*fx, made);
            }
            if let Some(Some(d)) = self.instances[id].dsps.get_mut(fx) {
                for (i, v) in &auto {
                    d.set_param(*i, *v);
                }
                d.set_position(rel);
                d.process(buf, 2);
            }
        }
        let vol = self
            .automated(&bus.id, crate::bank::property::VOLUME, &inputs)
            .unwrap_or(bus.body.volume_db);
        let v = db_to_gain(vol);
        buf.iter_mut().for_each(|s| *s *= v);
    }

    /// Number of buses between `id` and the root master bus.
    fn depth(&self, id: &Guid) -> usize {
        let mut d = 0;
        let mut cur = *id;
        while let Some((_, b)) = self.find_bus(&cur) {
            if b.output.target == Guid::NULL || d > 64 {
                break;
            }
            cur = b.output.target;
            d += 1;
        }
        d
    }

    /// Run the mixer's buses: effects then volume on each, deepest first,
    /// each adding into its output. The root's result goes to `out`.
    fn process_buses(&mut self, out: &mut [f32]) {
        let mut pending: Vec<Guid> = self.bus_buffers.keys().copied().collect();
        // Make sure every bus on the way to the root has a buffer.
        let mut k = 0;
        while k < pending.len() {
            if let Some((_, b)) = self.find_bus(&pending[k]) {
                let t = b.output.target;
                if t != Guid::NULL && !self.bus_buffers.contains_key(&t) {
                    self.bus_buffers.insert(t, vec![0.0; out.len()]);
                    pending.push(t);
                }
            }
            k += 1;
        }
        pending.sort_by_key(|g| std::cmp::Reverse(self.depth(g)));
        for g in pending {
            let Some(mut buf) = self.bus_buffers.remove(&g) else {
                continue;
            };
            let Some((_, bus)) = self.find_bus(&g).cloned() else {
                continue;
            };
            for e in bus.body.effects.iter().chain(&bus.body.effects_2) {
                if !self.dsps.contains_key(e) {
                    let made = self
                        .banks
                        .iter()
                        .find_map(|b| b.model.effects.get(e))
                        .and_then(|fx| {
                            if fx.bypass {
                                None
                            } else {
                                crate::dsp::from_effect(fx, SAMPLE_RATE, (1.0, 20.0))
                            }
                        });
                    self.dsps.insert(*e, made);
                }
                if let Some(Some(d)) = self.dsps.get_mut(e) {
                    d.process(&mut buf, 2);
                }
            }
            let v = db_to_gain(bus.body.volume_db);
            if bus.output.target == Guid::NULL {
                for (o, s) in out.iter_mut().zip(&buf) {
                    *o += s * v;
                }
            } else {
                let dst = self
                    .bus_buffers
                    .entry(bus.output.target)
                    .or_insert_with(|| vec![0.0; buf.len()]);
                for (o, s) in dst.iter_mut().zip(&buf) {
                    *o += s * v;
                }
            }
        }
        self.bus_buffers.clear();
    }

    /// Mix `frames` frames of stereo into `out` (interleaved, added).
    pub fn mix(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut scratch = vec![0f32; out.len()];
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
            self.instances[id].route = self.event_gain(&event);
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
            let mut tracks: HashMap<Guid, Vec<f32>> = HashMap::new();
            let inst = &mut self.instances[id];
            let g = inst.gain;
            for v in &mut inst.voices {
                let buf = tracks
                    .entry(v.track)
                    .or_insert_with(|| vec![0.0; out.len()]);
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
                        // Studio tracks place mono at full level on both sides.
                        let s = v.pcm.data[v.pos];
                        (s, s)
                    } else {
                        (v.pcm.data[v.pos * ch], v.pcm.data[v.pos * ch + 1])
                    };
                    buf[f * 2] += l * v.gain * g;
                    buf[f * 2 + 1] += r * v.gain * g;
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
            let rel = self.relative(self.instances[id].world_position);
            self.run_event_buses(id, &event, tracks, &mut scratch, rel);
            // Nested events join their parent's routing.
            let mut top = id;
            while let Some(p) = self.instances[top].parent {
                top = p;
            }
            let input = self.banks[self.instances[top].bank].model.events
                [&self.instances[top].event]
                .input_bus;
            let dst = self
                .bus_buffers
                .entry(input)
                .or_insert_with(|| vec![0.0; out.len()]);
            for (o, s) in dst.iter_mut().zip(&scratch) {
                *o += s;
            }
        }
        self.process_buses(out);
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
            self.event_gain(e),
            e.instruments.len(),
            self.banks[i.bank]
                .model
                .timelines
                .get(&e.timeline)
                .map(|t| t.instruments.len())
        )
    }
}
