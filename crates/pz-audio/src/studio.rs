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
use std::collections::{BTreeMap, HashMap, HashSet};
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
    instances: BTreeMap<InstanceId, EventInstance>,
    next_instance: InstanceId,
    timeline_markers: Vec<(InstanceId, String, i32)>,
    rng: u64,
    bus_buffers: HashMap<Guid, Vec<f32>>,
    /// Paths of the loaded strings banks.
    strings: crate::strings::StringTable,
    /// Listener position, forward and up vectors.
    listener: [[f32; 3]; 3],
    /// Global parameter values by parameter GUID.
    global_params: HashMap<Guid, f32>,
    /// Runtime state of each bus effect (`None`: passes the signal through).
    dsps: HashMap<Guid, Option<Box<dyn crate::dsp::Dsp>>>,
    /// Loose sound files on FMOD core channels.
    pub channels: crate::channels::Channels,
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
        parameters: Vec<Guid>,
        track: Guid,
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
    /// Track in the owning event that receives this nested event.
    parent_track: Guid,
    start_delay: usize,
    /// `EventInstance::setProperty` min/max distance overrides.
    distance_override: (Option<f32>, Option<f32>),
    /// `EventInstance::release`: free once stopped.
    released: bool,
    timeline_finished: bool,
    watch_markers: bool,
}

/// What `EventDescription` queries report.
#[derive(Debug, Clone)]
pub struct EventInfo {
    pub path: String,
    pub length_ms: u32,
    pub is_3d: bool,
    pub is_oneshot: bool,
    pub min_distance: f32,
    pub max_distance: f32,
    pub parameters: Vec<crate::bank::ParameterDescription>,
}

pub type InstanceId = usize;

impl Studio {
    pub fn new(setups: SetupHeaders) -> Studio {
        Studio {
            banks: Vec::new(),
            setups,
            pcm_cache: HashMap::new(),
            instances: BTreeMap::new(),
            next_instance: 0,
            timeline_markers: Vec::new(),
            rng: 0x9e37_79b9_7f4a_7c15,
            bus_buffers: HashMap::new(),
            listener: [[0.0; 3], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
            global_params: HashMap::new(),
            dsps: HashMap::new(),
            channels: Default::default(),
            strings: crate::strings::StringTable::default(),
        }
    }

    /// `Studio::System::getEvent(path)` (case-insensitive).
    pub fn event_by_path(&self, path: &str) -> Option<Guid> {
        let g = self.strings.lookup(path)?;
        self.find_event(&g).map(|_| g)
    }

    /// Every loaded event's path.
    pub fn event_paths(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .banks
            .iter()
            .flat_map(|b| b.model.events.keys())
            .filter_map(|g| self.strings.path(g).map(str::to_string))
            .filter(|p| p.starts_with("event:/"))
            .collect();
        v.sort();
        v
    }

    /// `EventDescription` queries. 3D means the event's own buses run a
    /// spatializer; one-shot means no looping instrument or loop region.
    pub fn event_info(&self, event: &Guid) -> Option<EventInfo> {
        let (bank, e) = self.find_event(event)?;
        let m = &self.banks[bank].model;
        let others: Vec<&BankModel> = self.banks.iter().map(|b| &b.model).collect();
        let parameters = m.event_parameters(e, &others);
        let buses = std::iter::once(e.master_bus).chain(e.tracks.iter().copied());
        let is_3d = buses.filter_map(|b| self.find_bus(&b)).any(|(_, bus)| {
            bus.body.effects.iter().chain(&bus.body.effects_2).any(|fx| {
                self.banks.iter().find_map(|b| b.model.effects.get(fx)).is_some_and(|x| x.kind == crate::bank::effect_type::PAN)
            })
        });
        let timeline = m.timelines.get(&e.timeline);
        let loop_region = timeline.is_some_and(|t| t.markers.iter().any(|mk| mk.length > 0));
        let looping = timeline
            .map(|t| t.instruments.iter().chain(&t.instruments_2).map(|p| p.instrument).collect::<Vec<_>>())
            .unwrap_or_default()
            .iter()
            .chain(&e.instruments)
            .filter_map(|g| self.find_instrument(g))
            .any(|(_, i)| i.base.loop_count < 0);
        Some(EventInfo {
            path: self.strings.path(event).unwrap_or_default().to_string(),
            length_ms: m.event_length_ms(e),
            is_3d,
            is_oneshot: !loop_region && !looping,
            min_distance: e.min_distance,
            max_distance: e.max_distance,
            parameters,
        })
    }

    /// `EventInstance::stop`. Without modelled release envelopes both
    /// modes stop at once.
    pub fn stop(&mut self, id: InstanceId, _immediate: bool) {
        let mut stopped = HashSet::new();
        // Children are allocated after their parents.
        for (&key, instance) in self.instances.range_mut(id..) {
            if key == id || instance.parent.is_some_and(|p| stopped.contains(&p)) {
                instance.state = PlaybackState::Stopped;
                instance.voices.clear();
                stopped.insert(key);
            }
        }
    }

    /// The core master channel group owns Studio events and loose channels.
    pub fn stop_all(&mut self) {
        for i in self.instances.values_mut() {
            i.state = PlaybackState::Stopped;
            i.voices.clear();
        }
        self.channels.stop_all();
    }

    /// `EventInstance::release`.
    pub fn release(&mut self, id: InstanceId) {
        if let Some(i) = self.instances.get_mut(&id) {
            i.released = true;
        }
    }

    /// `EventInstance::setProperty(MIN/MAX_DISTANCE)`.
    pub fn set_distance_override(&mut self, id: InstanceId, min: Option<f32>, max: Option<f32>) {
        if let Some(i) = self.instances.get_mut(&id) {
            if min.is_some() {
                i.distance_override.0 = min;
            }
            if max.is_some() {
                i.distance_override.1 = max;
            }
        }
    }

    /// Find the parameter with this ID among the events' parameter lists.
    fn parameter_by_id(&self, id: (u32, u32)) -> Option<Guid> {
        for b in &self.banks {
            for e in b.model.events.values() {
                if let Some(k) = e.parameter_ids.iter().position(|x| *x == id) {
                    let sheet = e.parameter_sheets.get(k)?;
                    return self.banks.iter().find_map(|b| b.model.parameter_sheets.get(sheet)).map(|s| s.parameter);
                }
            }
        }
        None
    }

    /// `EventInstance::setParameterByID`.
    pub fn set_parameter_by_id(&mut self, id: InstanceId, pid: (u32, u32), value: f32) {
        if !self.instances.contains_key(&id) { return; }
        let Some(g) = self.parameter_by_id(pid) else { return };
        let Some(p) = self.find_parameter(&g).cloned() else { return };
        let v = value.clamp(p.minimum, p.maximum);
        if p.flags & crate::bank::Parameter::GLOBAL != 0 {
            self.global_params.insert(g, v);
        } else if let Some(i) = self.instances.get_mut(&id) {
            i.params.insert(g, v);
        }
    }

    /// `Studio::System::setParameterByID`.
    pub fn set_global_parameter_by_id(&mut self, pid: (u32, u32), value: f32) {
        let Some(g) = self.parameter_by_id(pid) else { return };
        let Some(p) = self.find_parameter(&g).cloned() else { return };
        self.global_params.insert(g, value.clamp(p.minimum, p.maximum));
    }

    /// Subscribe the instance to FMOD timeline marker callbacks.
    pub fn watch_timeline_markers(&mut self, id: InstanceId) {
        if let Some(instance) = self.instances.get_mut(&id) {
            instance.watch_markers = true;
        }
    }

    pub fn take_timeline_markers(&mut self) -> Vec<(InstanceId, String, i32)> {
        std::mem::take(&mut self.timeline_markers)
    }

    /// `EventInstance::getTimelinePosition` (ms).
    pub fn timeline_position_ms(&self, id: InstanceId) -> i64 {
        self.instances.get(&id).map_or(0, |i| (i.position * 1000 / SAMPLE_RATE as u64) as i64)
    }

    /// `EventInstance::setTimelinePosition` (ms).
    pub fn set_timeline_position_ms(&mut self, id: InstanceId, ms: i32) {
        if let Some(i) = self.instances.get_mut(&id) {
            i.position = (ms.max(0) as u64) * SAMPLE_RATE as u64 / 1000;
        }
    }

    /// `Studio::System::setListenerAttributes` (position, forward, up).
    pub fn set_listener(&mut self, position: [f32; 3], forward: [f32; 3], up: [f32; 3]) {
        self.listener = [position, forward, up];
    }

    /// `EventInstance::set3DAttributes` (position only for now).
    pub fn set_position(&mut self, id: InstanceId, position: [f32; 3]) {
        if let Some(i) = self.instances.get_mut(&id) { i.world_position = position; }
    }

    fn find_parameter(&self, id: &Guid) -> Option<&crate::bank::Parameter> {
        self.banks.iter().find_map(|b| b.model.parameters.get(id))
    }

    /// `EventInstance::setParameterByName` (case-insensitive).
    pub fn set_parameter_by_name(&mut self, id: InstanceId, name: &str, value: f32) -> bool {
        let Some(i) = self.instances.get(&id) else { return false };
        let e = self.banks[i.bank].model.events[&i.event].clone();
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
                    self.instances.get_mut(&id).unwrap().params.insert(pid, v);
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
            _ => *self.instances[&id].params.get(&p.id).unwrap_or(&p.default),
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
        if let Some(strings) = file.strings() {
            self.strings.merge(strings);
            return Ok(());
        }
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
        let id = self.next_instance;
        self.next_instance = id.checked_add(1)?;
        self.instances.insert(id, EventInstance {
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
            parent_track: Guid::NULL,
            start_delay: 0,
            distance_override: (None, None),
            released: false,
            timeline_finished: false,
            watch_markers: false,
        });
        Some(id)
    }

    pub fn start(&mut self, id: InstanceId) {
        if !self.instances.contains_key(&id) { return; }
        self.stop(id, true);
        let i = self.instances.get_mut(&id).unwrap();
        i.triggered.fill(false);
        i.dsps.clear();
        i.timeline_finished = false;
        i.start_delay = 0;
        i.state = PlaybackState::Playing;
        i.started = false;
        i.position = 0;
    }

    pub fn set_volume(&mut self, id: InstanceId, v: f32) {
        if let Some(i) = self.instances.get_mut(&id) { i.gain = v; }
    }

    pub fn state(&self, id: InstanceId) -> PlaybackState {
        self.instances.get(&id).map_or(PlaybackState::Stopped, |i| i.state)
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
        parent_delay_ms: f32,
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
            parent_delay_ms + inst.base.delay_min + (inst.base.delay_max - inst.base.delay_min) * self.random();
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
                        self.instrument_voices(g, gain, track, depth + 1, delay_ms, out);
                        break;
                    }
                    pick -= w;
                }
            }
            InstrumentKind::Event { event, raw } => {
                // EVIB lists the parameters passed down to a referenced event.
                let parameters = raw.get(36..).and_then(|bytes| {
                    crate::reader::Reader::new(bytes).read_structs(|r| {
                        r.u32()?;
                        r.guid()
                    }).ok()
                }).unwrap_or_default();
                out.push(Start::Event { event: *event, parameters, track, gain, delay_ms });
            }
            _ => {}
        }
    }

    /// Start a placement at its exact frame in the current device callback.
    fn start_instrument(&mut self, inst: InstanceId, id: Guid, remaining: Option<usize>, block_offset: usize) {
        let mut v = Vec::new();
        self.instrument_voices(&id, 1.0, Guid::NULL, 0, 0.0, &mut v);
        let delay_frames = |ms: f32| block_offset.saturating_add((ms.max(0.0) * SAMPLE_RATE as f32 / 1000.0) as usize);
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
                        let delay = delay_frames(delay_ms);
                        self.instances.get_mut(&inst).unwrap().voices.push(Voice {
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
                    parameters,
                    track,
                    gain,
                    delay_ms,
                } => {
                    let mut ancestor = Some(inst);
                    while let Some(i) = ancestor {
                        if self.instances[&i].event == event { break; }
                        ancestor = self.instances[&i].parent;
                    }
                    if ancestor.is_some() { continue; }
                    let Some(child) = self.create_instance(event) else {
                        continue;
                    };
                    let inherited = parameters.iter().filter_map(|p|
                        self.instances[&inst].params.get(p).map(|&v| (*p, v))).collect();
                    let c = self.instances.get_mut(&child).unwrap();
                    c.params = inherited;
                    c.parent = Some(inst);
                    c.parent_track = track;
                    c.released = true;
                    c.gain = gain;
                    c.start_delay = delay_frames(delay_ms);
                    c.state = PlaybackState::Playing;
                }
            }
        }
    }

    /// One-shot instruments selected on the event's initial parameter values.
    fn start_parameter_sheets(&mut self, id: InstanceId, event: &Event, offset: usize) {
        let mut root = id;
        while let Some(parent) = self.instances[&root].parent { root = parent; }
        let relative = self.relative(self.instances[&root].world_position);
        for sheet_id in &event.parameter_sheets {
            let Some(sheet) = self.banks.iter().find_map(|b| b.model.parameter_sheets.get(sheet_id)) else { continue };
            if sheet.instruments.is_empty() { continue; }
            let Some(parameter) = self.find_parameter(&sheet.parameter) else { continue };
            let value = self.parameter_value(id, parameter, event, relative);
            let selected: Vec<_> = sheet.instruments.iter().copied().filter(|key|
                self.find_instrument(key).and_then(|(_, i)| i.base.parameter_range)
                    .is_some_and(|range| range.parameter == parameter.id && range.contains(value))).collect();
            for instrument in selected {
                self.start_instrument(id, instrument, None, offset);
            }
        }
    }

    fn find_bus(&self, id: &Guid) -> Option<&(BusKind, crate::bank::Bus)> {
        self.banks.iter().find_map(|b| b.model.buses.get(id))
    }

    /// Unautomated event master volume for the diagnostic description.
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
            if !self.instances[&id].dsps.contains_key(fx) {
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
                self.instances.get_mut(&id).unwrap().dsps.insert(*fx, made);
            }
            if let Some(Some(d)) = self.instances.get_mut(&id).unwrap().dsps.get_mut(fx) {
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
                    *o += s * v * self.channels.master_volume.unwrap_or(1.0);
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
        let mut event_tracks: BTreeMap<InstanceId, HashMap<Guid, Vec<f32>>> = BTreeMap::new();
        let mut cursor = 0;
        while let Some((&id, instance)) = self.instances.range(cursor..).next() {
            cursor = id + 1;
            if instance.state != PlaybackState::Playing || instance.timeline_finished {
                continue;
            }
            let leading_silence = instance.start_delay;
            if leading_silence >= frames {
                self.instances.get_mut(&id).unwrap().start_delay -= frames;
                continue;
            }
            let active_frames = frames - leading_silence;
            self.instances.get_mut(&id).unwrap().start_delay = 0;
            let (bank, event) = {
                let i = &self.instances[&id];
                (i.bank, self.banks[i.bank].model.events[&i.event].clone())
            };
            let first_block = !self.instances[&id].started;
            if first_block {
                self.instances.get_mut(&id).unwrap().started = true;
                for g in event.instruments.clone() {
                    self.start_instrument(id, g, None, leading_silence);
                }
                self.start_parameter_sheets(id, &event, leading_silence);
            }
            // Timeline triggers within this block.
            if let Some(t) = self.banks[bank]
                .model
                .timelines
                .get(&event.timeline)
                .cloned()
            {
                let pos = self.instances[&id].position;
                if self.instances[&id].watch_markers {
                    let end = pos + active_frames as u64;
                    for marker in &t.markers {
                        let at = u64::from(marker.position);
                        if marker.length == 0 && !marker.name.is_empty() && at <= end
                            && (at > pos || (first_block && pos == 0 && at == 0))
                        {
                            let ms = (at * 1000 / u64::from(SAMPLE_RATE)) as i32;
                            self.timeline_markers.push((id, marker.name.clone(), ms));
                        }
                    }
                }
                for (k, p) in t.instruments.iter().chain(&t.instruments_2).enumerate() {
                    if !self.instances[&id].triggered[k] && (p.start as u64) < pos + active_frames as u64 {
                        self.instances.get_mut(&id).unwrap().triggered[k] = true;
                        let offset = (p.start as u64).saturating_sub(pos) as usize;
                        self.start_instrument(id, p.instrument, Some(p.length as usize), leading_silence + offset);
                    }
                }
            }
            let inst = self.instances.get_mut(&id).unwrap();
            for v in &mut inst.voices {
                let buf = event_tracks.entry(id).or_default()
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
                    buf[f * 2] += l * v.gain;
                    buf[f * 2 + 1] += r * v.gain;
                    v.pos += 1;
                }
            }
            inst.voices
                .retain(|v| v.pos < v.pcm.frames() && v.remaining != Some(0));
            inst.position += active_frames as u64;
            let end = self.banks[bank]
                .model
                .timelines
                .get(&event.timeline)
                .map_or(0, |t| t.end() as u64);
            if inst.voices.is_empty() && inst.position >= end {
                inst.timeline_finished = true;
            }
        }

        // Child instances have higher IDs. Mix them first, into the track
        // that owns the event instrument. This lets the parent's spatializer,
        // automation and volume act on its own voices and its children together.
        while let Some((id, tracks)) = event_tracks.pop_last() {
            let instance = &self.instances[&id];
            let event = self.banks[instance.bank].model.events[&instance.event].clone();
            let parent = instance.parent;
            let target = instance.parent_track;
            let gain = instance.gain;
            let mut root = id;
            while let Some(owner) = self.instances[&root].parent { root = owner; }
            let rel = self.relative(self.instances[&root].world_position);
            self.run_event_buses(id, &event, tracks, &mut scratch, rel);
            let dst = if let Some(owner) = parent {
                event_tracks.entry(owner).or_default().entry(target).or_insert_with(|| vec![0.0; out.len()])
            } else {
                self.bus_buffers.entry(event.input_bus).or_insert_with(|| vec![0.0; out.len()])
            };
            for (o, s) in dst.iter_mut().zip(&scratch) { *o += s * gain; }
        }
        // A parent remains playing until its last nested event finishes.
        let mut playing_parents = HashSet::new();
        for (&id, i) in self.instances.iter_mut().rev() {
            if i.timeline_finished && !playing_parents.contains(&id) { i.state = PlaybackState::Stopped; }
            if i.state != PlaybackState::Stopped {
                if let Some(parent) = i.parent { playing_parents.insert(parent); }
            }
        }
        self.instances.retain(|_, i| !(i.released && i.state == PlaybackState::Stopped));
        self.process_buses(out);
        let mut channels = std::mem::take(&mut self.channels);
        channels.mix(out, |p| self.relative(p));
        channels.reap();
        self.channels = channels;
    }

    pub fn lookup_event(&self, strings: &crate::strings::StringTable, path: &str) -> Option<Guid> {
        strings.lookup(path)
    }
}

impl Studio {
    #[doc(hidden)]
    pub fn debug_instance(&self, id: InstanceId) -> String {
        let Some(i) = self.instances.get(&id) else { return "released".into() };
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

#[cfg(test)]
mod lifetime_tests {
    use super::*;

    #[test]
    #[ignore = "requires the installed Footsteps event and FSB samples"]
    fn footstep_parameter_sheets_select_the_native_sample_families() {
        let game = std::path::PathBuf::from(std::env::var_os("PZ_GAME_DIR").unwrap());
        let mut studio = Studio::new(SetupHeaders::from_game_dir(&game).unwrap());
        for bank in crate::startup_bank_paths(&game) { studio.load_bank(&bank).unwrap(); }
        let footsteps = studio.event_by_path("event:/Character/Foley/Footsteps").unwrap();
        let concrete = "Player_Fs_Concrete_Sneakers_Walk_";
        for (material, shoe, gait, overlay, base_family, layer_family) in [
            (2, 3, 3, 0, concrete, None),
            (0, 0, 0, 0, "Player_Fs_Wood_Loose_Barefoot_Sneak_", None),
            (7, 3, 3, 0, "Player_Fs_Wood_Loose_Sneakers_Walk_", None),
            (2, 3, 5, 0, "Player_Fs_Concrete_Sneakers_Sprint_", None),
            (12, 1, 1, 0, "Player_Fs_Metal_Boots_Sneak_", None),
            (2, 3, 3, 1, concrete, Some("Player_Fs_Glass_Walk_")),
            (2, 3, 3, 2, concrete, Some("Player_Fs_Water_Walk_")),
            (2, 3, 3, 3, concrete, Some("Player_Fs_Water_Walk_")),
            (2, 3, 3, 4, concrete, Some("Player_Fs_Garbage_Walk_")),
            (2, 3, 3, 5, concrete, Some("Player_Fs_Leaves_Walk_")),
        ] {
            let id = studio.create_instance(footsteps).unwrap();
            for (name, value) in [("FootstepMaterial", material), ("FootstepMaterial2", overlay),
                ("ShoeType", shoe), ("CharacterMovementSpeed", gait)] {
                assert!(studio.set_parameter_by_name(id, name, value as f32));
            }
            studio.start(id);
            let mut block = [0.0; 1024 * 2];
            studio.mix(&mut block);
            let playing: Vec<_> = studio.instances.values().flat_map(|i| &i.voices)
                .map(|voice| {
                    let (&(bank, sample), _) = studio.pcm_cache.iter()
                        .find(|(_, pcm)| Arc::ptr_eq(pcm, &voice.pcm)).unwrap();
                    studio.banks[bank].file.fsb.as_ref().unwrap().samples[sample as usize].name.as_deref().unwrap()
                }).collect();
            assert_eq!(playing.len(), 1 + usize::from(layer_family.is_some()), "{material}/{shoe}/{gait}/{overlay}: {playing:?}");
            for expected in [Some(base_family), layer_family].into_iter().flatten() {
                assert!(playing.iter().any(|s| s.starts_with(expected)), "{playing:?} expected {expected}");
            }
            let mut samples = vec![0.0; 12000 * 2];
            studio.mix(&mut samples);
            assert!(samples.iter().all(|s| s.is_finite()));
            let peak = samples.iter().chain(&block).fold(0.0_f32, |p, x| p.max(x.abs()));
            // The installed barefoot sneak sample peaks below -60 dBFS after routing.
            assert!(peak > 1e-6, "{material}/{shoe}/{gait} peak={peak}");
            studio.stop(id, true);
            studio.release(id);
            studio.mix(&mut block);
        }
        // The parent Footsteps event owns the spatializer. Reuse the same
        // random sequence so both placements select the very same waveform.
        let seed = studio.rng;
        let mut energy = Vec::new();
        for distance in [0.0, 64.0] {
            studio.rng = seed;
            let id = studio.create_instance(footsteps).unwrap();
            for (name, value) in [("FootstepMaterial", 2.0), ("ShoeType", 3.0), ("CharacterMovementSpeed", 3.0)] {
                studio.set_parameter_by_name(id, name, value);
            }
            studio.set_position(id, [0.0, 0.0, distance]);
            studio.start(id);
            let mut samples = vec![0.0; 24000 * 2];
            studio.mix(&mut samples);
            energy.push(samples.iter().map(|s| s * s).sum::<f32>());
            studio.stop(id, true);
            studio.release(id);
        }
        assert!(energy[0] > 0.0 && energy[1] < energy[0] * 0.01, "{energy:?}");

    }

    #[test]
    #[ignore = "requires installed FMOD banks in PZ_GAME_DIR"]
    fn nested_placement_starts_at_the_same_sample_across_callback_boundaries() {
        use crate::bank::{Placement, Timeline};
        let game = std::path::PathBuf::from(std::env::var_os("PZ_GAME_DIR").unwrap());
        let mut studio = Studio::new(SetupHeaders::from_game_dir(&game).unwrap());
        for bank in crate::startup_bank_paths(&game) {
            studio.load_bank(&bank).unwrap();
        }
        let smash = studio.event_by_path("event:/Object/Window/Smash").unwrap();
        let (bank, event) = studio.find_event(&smash).unwrap();
        let mut parent = event.clone();
        let parent_id = Guid([0xa1; 16]);
        let timeline = Guid([0xa2; 16]);
        let placement = Guid([0xa3; 16]);
        parent.id = parent_id;
        parent.timeline = timeline;
        parent.instruments.clear();
        let model = &mut studio.banks[bank].model;
        let mut instrument = model.instruments.values().next().unwrap().clone();
        instrument.id = placement;
        instrument.kind = InstrumentKind::Event { event: smash, raw: Vec::new() };
        instrument.base.volume_db = 0.0;
        instrument.base.probability = 100.0;
        instrument.base.delay_min = 0.5;
        instrument.base.delay_max = 0.5;
        assert!(model.instruments.insert(placement, instrument).is_none());
        assert!(model.events.insert(parent_id, parent).is_none());
        assert!(model.timelines.insert(timeline, Timeline {
            id: timeline,
            instruments: vec![Placement { instrument: placement, start: 77, length: 24000 }],
            instruments_2: vec![], unknown_c: vec![], markers: vec![], unknown_e: vec![],
        }).is_none());

        // 13 frames before the parent, 77 to the placement and 24 of
        // instrument delay put the child's first frame at sample frame 114.
        for sizes in [[64, 64, 256], [114, 1, 269], [121, 128, 135]] {
            let owner = studio.create_instance(parent_id).unwrap();
            studio.start(owner);
            studio.instances.get_mut(&owner).unwrap().start_delay = 13;
            let mut elapsed: usize = 0;
            for frames in sizes {
                let mut out = vec![0.0; frames * 2];
                studio.mix(&mut out);
                let silent = (114usize.saturating_sub(elapsed)).min(frames);
                assert!(out[..silent * 2].iter().all(|s| *s == 0.0));
                elapsed += frames;
                assert_eq!(studio.instances[&owner].position, (elapsed - 13) as u64);
                if elapsed > 90 {
                    let child = studio.instances.values().find(|i| i.parent == Some(owner)).unwrap();
                    assert_eq!(child.position, elapsed.saturating_sub(114) as u64);
                }
            }
            let mut samples = vec![0.0; 24000 * 2];
            studio.mix(&mut samples);
            assert!(samples.iter().all(|s| s.is_finite()));
            assert!(samples.iter().any(|s| s.abs() > 0.001));
            studio.stop(owner, true);
            studio.release(owner);
            studio.mix(&mut [0.0; 128]);
            // Separate the mixer bus effect histories of the three playbacks.
            studio.dsps.clear();
        }
    }

    #[test]
    #[ignore = "requires installed FMOD banks in PZ_GAME_DIR"]
    fn released_instances_and_nested_events_end_with_their_owner() {
        let game = std::path::PathBuf::from(std::env::var_os("PZ_GAME_DIR").expect("PZ_GAME_DIR"));
        let mut studio = Studio::new(SetupHeaders::from_game_dir(&game).unwrap());
        for bank in crate::startup_bank_paths(&game) { studio.load_bank(&bank).unwrap(); }
        let smash = studio.event_by_path("event:/Object/Window/Smash").unwrap();
        let nested = studio.banks.iter().flat_map(|bank| bank.model.instruments.values())
            .find_map(|instrument| match &instrument.kind {
                InstrumentKind::Event { event, .. } if *event != smash && instrument.base.probability >= 100.0
                    && studio.find_event(event).is_some() => Some(instrument.id),
                _ => None,
            }).expect("installed nested event instrument");
        let mut block = [0.0; 128];
        let parent = studio.create_instance(smash).unwrap();
        studio.start(parent);
        studio.start_instrument(parent, nested, None, 1024);
        let child = studio.instances.iter().find(|(_, i)| i.parent == Some(parent)).unwrap().0.to_owned();
        assert!(studio.instances[&child].start_delay >= 1024);
        studio.instances.get_mut(&parent).unwrap().timeline_finished = true;
        studio.release(parent);
        studio.mix(&mut block);
        assert_eq!(studio.state(parent), PlaybackState::Playing);
        assert_eq!(studio.state(child), PlaybackState::Playing);
        studio.stop(parent, true);
        studio.mix(&mut block);
        assert!(studio.instances.is_empty());
        let next = studio.create_instance(smash).unwrap();
        studio.start(next);
        studio.stop(parent, true);
        studio.set_volume(child, 0.0);
        assert_eq!(studio.state(next), PlaybackState::Playing);
        studio.stop_all();
        assert_eq!(studio.state(next), PlaybackState::Stopped);
        studio.release(next);
        studio.mix(&mut block);
        assert!(studio.instances.is_empty());
    }
}
