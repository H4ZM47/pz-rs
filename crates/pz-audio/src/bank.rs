//! The Studio object model stored in a bank's `PROJ` list.
//!
//! Every object kind has a list (`LIST EVTS`) of per-object lists
//! (`LIST EVNT`), each holding the object's chunks (`EVTB`, `PRPS`, ...).
//! See `FORMAT.md` for the layouts; fields whose meaning is not yet known
//! keep a descriptive placeholder name.

use crate::reader::{Guid, Reader, Result, Truncated};
use crate::riff::Chunk;
use std::collections::HashMap;

/// The chunks of one object, by id.
#[derive(Default, Debug, Clone)]
pub struct RawObject<'a> {
    pub chunks: Vec<([u8; 4], &'a [u8])>,
}

impl<'a> RawObject<'a> {
    pub fn get(&self, id: &[u8; 4]) -> Option<&'a [u8]> {
        self.chunks.iter().find(|(i, _)| i == id).map(|(_, b)| *b)
    }
}

/// Group the objects of one kind: `LIST <group>` → `LIST <item>` → chunks.
pub fn raw_objects<'a>(file: &'a [u8], proj: &Chunk, group: &[u8; 4]) -> Vec<RawObject<'a>> {
    let Some(list) = proj
        .children
        .iter()
        .find(|c| c.list_type.as_ref() == Some(group))
    else {
        return Vec::new();
    };
    list.children
        .iter()
        .filter(|c| c.list_type.is_some())
        .map(|item| RawObject {
            chunks: item
                .children
                .iter()
                .map(|c| (c.list_type.unwrap_or(c.id), &file[c.body.clone()]))
                .collect(),
        })
        .collect()
}

/// Leaf chunks of a flat list (`LIST WAVS` → `WAV ` chunks).
pub fn raw_leaves<'a>(
    file: &'a [u8],
    proj: &Chunk,
    group: &[u8; 4],
    id: &[u8; 4],
) -> Vec<&'a [u8]> {
    proj.children
        .iter()
        .find(|c| c.list_type.as_ref() == Some(group))
        .map(|l| {
            l.children
                .iter()
                .filter(|c| &c.id == id)
                .map(|c| &file[c.body.clone()])
                .collect()
        })
        .unwrap_or_default()
}

/// Routing embedded in buses and instruments: where the signal goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Output {
    pub target: Guid,
    pub speaker_mode: u32,
    pub unknown: i32,
}

fn read_output(r: &mut Reader) -> Result<Output> {
    let size = r.u16()? as usize;
    let mut s = Reader::new(r.bytes(size)?);
    Ok(Output {
        target: s.guid()?,
        speaker_mode: s.u32()?,
        unknown: s.i32()?,
    })
}

#[derive(Debug, Clone)]
pub struct Event {
    pub id: Guid,
    /// The snapshot record, for snapshot events.
    pub parent: Guid,
    pub timeline: Guid,
    pub input_bus: Guid,
    pub master_bus: Guid,
    pub max_instances: u32,
    pub unknown_54: u32,
    pub unknown_58: u32,
    pub unknown_5c: u8,
    pub parameter_sheets: Vec<Guid>,
    pub unknown_words: [u32; 4],
    pub flags: u32,
    pub tracks: Vec<Guid>,
    pub parameter_ids: Vec<(u32, u32)>,
    pub instruments: Vec<Guid>,
    pub min_distance: f32,
    pub max_distance: f32,
}

impl Event {
    pub fn parse(b: &[u8]) -> Result<Event> {
        let mut r = Reader::new(b);
        Ok(Event {
            id: r.guid()?,
            parent: r.guid()?,
            timeline: r.guid()?,
            input_bus: r.guid()?,
            master_bus: r.guid()?,
            max_instances: r.u32()?,
            unknown_54: r.u32()?,
            unknown_58: r.u32()?,
            unknown_5c: r.u8()?,
            parameter_sheets: r.read_structs(|r| r.guid())?,
            unknown_words: [r.u32()?, r.u32()?, r.u32()?, r.u32()?],
            flags: r.u32()?,
            tracks: r.read_structs(|r| r.guid())?,
            parameter_ids: r.read_structs(|r| Ok((r.u32()?, r.u32()?)))?,
            instruments: r.read_structs(|r| r.guid())?,
            min_distance: r.f32()?,
            max_distance: r.f32()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub id: Guid,
    pub flags: u32,
    pub kind: u32,
    pub name: String,
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
    pub unknown: f32,
    pub labels: Vec<String>,
}

impl Parameter {
    pub const GLOBAL: u32 = 0x01;
    pub const DISCRETE: u32 = 0x08;
    pub const AUTOMATIC: u32 = 0x10;
    pub const LABELED: u32 = 0x20;

    pub fn parse(b: &[u8]) -> Result<Parameter> {
        let mut r = Reader::new(b);
        Ok(Parameter {
            id: r.guid()?,
            flags: r.u32()?,
            kind: r.u32()?,
            name: r.string()?,
            minimum: r.f32()?,
            maximum: r.f32()?,
            default: r.f32()?,
            unknown: r.f32()?,
            labels: r.read_structs(|r| r.string())?,
        })
    }

    /// FMOD_STUDIO_PARAMETER_FLAGS as reported by the Studio API.
    pub fn api_flags(&self) -> u32 {
        const READONLY: u32 = 1;
        const AUTOMATIC: u32 = 2;
        const GLOBAL: u32 = 4;
        const DISCRETE: u32 = 8;
        const LABELED: u32 = 16;
        let mut f = 0;
        if self.flags & Self::GLOBAL != 0 {
            f |= GLOBAL | READONLY;
        }
        if self.flags & Self::AUTOMATIC != 0 {
            f |= AUTOMATIC | READONLY;
        }
        if self.flags & Self::DISCRETE != 0 {
            f |= DISCRETE;
        }
        if self.flags & Self::LABELED != 0 {
            f |= LABELED;
        }
        f
    }
}

/// `PMLB`: an event's use of a parameter, with the instruments on its sheet.
#[derive(Debug, Clone)]
pub struct ParameterSheet {
    pub id: Guid,
    pub parameter: Guid,
    pub instruments: Vec<Guid>,
    pub unknown: u32,
}

impl ParameterSheet {
    pub fn parse(b: &[u8]) -> Result<ParameterSheet> {
        let mut r = Reader::new(b);
        Ok(ParameterSheet {
            id: r.guid()?,
            parameter: r.guid()?,
            instruments: r.read_structs(|r| r.guid())?,
            unknown: r.u32()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub instrument: Guid,
    pub start: u32,
    pub length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub id: Guid,
    pub position: u32,
    pub name: String,
    /// Non-zero for regions.
    pub length: u32,
}

#[derive(Debug, Clone)]
pub struct Timeline {
    pub id: Guid,
    pub instruments: Vec<Placement>,
    pub instruments_2: Vec<Placement>,
    pub unknown_c: Vec<Vec<u8>>,
    pub markers: Vec<Marker>,
    pub unknown_e: Vec<Vec<u8>>,
}

impl Timeline {
    pub fn parse(b: &[u8]) -> Result<Timeline> {
        let mut r = Reader::new(b);
        let placement = |r: &mut Reader| {
            Ok(Placement {
                instrument: r.guid()?,
                start: r.u32()?,
                length: r.u32()?,
            })
        };
        let raw = |r: &mut Reader| Ok(r.bytes(r.remaining())?.to_vec());
        Ok(Timeline {
            id: r.guid()?,
            instruments: r.read_structs(placement)?,
            instruments_2: r.read_structs(placement)?,
            unknown_c: r.read_structs(raw)?,
            markers: r.read_structs(|r| {
                Ok(Marker {
                    id: r.guid()?,
                    position: r.u32()?,
                    name: r.string()?,
                    length: r.u32()?,
                })
            })?,
            unknown_e: r.read_structs(raw)?,
        })
    }

    /// End of the last instrument or region, in 48 kHz samples.
    pub fn end(&self) -> u32 {
        let inst = self
            .instruments
            .iter()
            .chain(&self.instruments_2)
            .map(|p| p.start.saturating_add(p.length));
        let regions = self
            .markers
            .iter()
            .map(|m| m.position.saturating_add(m.length));
        // Entries of `unknown_c` start with a position that also counts.
        let c = self
            .unknown_c
            .iter()
            .filter_map(|b| Some(u32::from_le_bytes(b.get(0..4)?.try_into().ok()?)));
        inst.chain(regions).chain(c).max().unwrap_or(0)
    }
}

/// `WAV `: a waveform, i.e. one FSB5 sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Waveform {
    pub id: Guid,
    pub subsound: u32,
    /// 0x20000 on waveforms the bank streams (music).
    pub flags: u32,
}

impl Waveform {
    pub fn parse(b: &[u8]) -> Result<Waveform> {
        let mut r = Reader::new(b);
        let id = r.guid()?;
        let size = r.u16()? as usize;
        let mut s = Reader::new(r.bytes(size)?);
        let _unknown = s.u32()?;
        Ok(Waveform {
            id,
            subsound: s.u32()?,
            flags: s.u32()?,
        })
    }
}

/// Fields shared by every instrument (`INST`).
#[derive(Debug, Clone)]
pub struct InstrumentBase {
    pub owner: Guid,
    pub volume_db: f32,
    pub unknown_14: f32,
    pub loop_count: i32,
    pub unknown_1c: u32,
    pub unknown_20: f32,
    pub probability: f32,
    pub delay_min: f32,
    pub delay_max: f32,
    pub max_voices: u32,
    pub output: Option<Output>,
    pub raw: Vec<u8>,
}

impl InstrumentBase {
    pub fn parse(b: &[u8]) -> Result<InstrumentBase> {
        let mut r = Reader::new(b);
        let owner = r.guid()?;
        let volume_db = r.f32()?;
        let unknown_14 = r.f32()?;
        let loop_count = r.i32()?;
        let unknown_1c = r.u32()?;
        let unknown_20 = r.f32()?;
        let probability = r.f32()?;
        let delay_min = r.f32()?;
        let delay_max = r.f32()?;
        let mut tail = Reader::new(b);
        tail.pos = 0x50;
        let max_voices = tail.u32()?;
        let output = read_output(&mut tail).ok();
        Ok(InstrumentBase {
            owner,
            volume_db,
            unknown_14,
            loop_count,
            unknown_1c,
            unknown_20,
            probability,
            delay_min,
            delay_max,
            max_voices,
            output,
            raw: b.to_vec(),
        })
    }
}

#[derive(Debug, Clone)]
pub enum InstrumentKind {
    /// `WAIB`: plays one waveform.
    Single { waveform: Guid },
    /// `MUIB`: plays one entry of its playlist.
    Multi { playlist: Playlist },
    /// `SPIB`: scatterer over its playlist.
    Scatterer { playlist: Playlist, raw: Vec<u8> },
    /// `EVIB`: a nested or referenced event.
    Event { event: Guid, raw: Vec<u8> },
    /// `SLNB`: silence.
    Silence { raw: Vec<u8> },
    /// `CMDB`: command instrument.
    Command { raw: Vec<u8> },
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub mode: u32,
    pub unknown: u32,
    pub entries: Vec<(Guid, f32)>,
}

impl Playlist {
    pub fn parse(b: &[u8]) -> Result<Playlist> {
        let mut r = Reader::new(b);
        Ok(Playlist {
            mode: r.u32()?,
            unknown: r.u32()?,
            entries: r.read_structs(|r| Ok((r.guid()?, r.f32()?)))?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Instrument {
    pub id: Guid,
    pub base: InstrumentBase,
    pub kind: InstrumentKind,
}

#[derive(Debug, Clone)]
pub struct Bus {
    pub id: Guid,
    pub output: Output,
    pub body: BusBody,
}

/// `BUS `: fields every bus kind shares.
#[derive(Debug, Clone, Default)]
pub struct BusBody {
    pub unknown_0: u32,
    pub speaker_mode: u32,
    /// Effect chain, in order (`BEFF`, `SEFF`, ...).
    pub effects: Vec<Guid>,
    pub effects_2: Vec<Guid>,
    pub volume_db: f32,
    pub unknown_vol_2: f32,
    pub vcas: Vec<Guid>,
    pub unknown_max: u32,
    pub unknown_e: u32,
    pub effect_speaker_modes: Vec<u32>,
    pub effect_speaker_modes_2: Vec<u32>,
}

impl BusBody {
    pub fn parse(b: &[u8]) -> Result<BusBody> {
        let mut r = Reader::new(b);
        let unknown_0 = r.u32()?;
        let speaker_mode = r.u32()?;
        let effects = r.read_structs(|r| r.guid())?;
        let effects_2 = r.read_structs(|r| r.guid())?;
        let size = r.u16()? as usize;
        let mut v = Reader::new(r.bytes(size)?);
        Ok(BusBody {
            unknown_0,
            speaker_mode,
            effects,
            effects_2,
            volume_db: v.f32()?,
            unknown_vol_2: v.f32()?,
            vcas: r.read_structs(|r| r.guid())?,
            unknown_max: r.u32()?,
            unknown_e: r.u32()?,
            effect_speaker_modes: r.read_structs(|r| r.u32())?,
            effect_speaker_modes_2: r.read_structs(|r| r.u32())?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusKind {
    Group,
    Master,
    Input,
    Return,
}

/// A value from an effect's `PMEF` parameter list.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Data(Vec<u8>),
}

impl ParamValue {
    pub fn as_f32(&self) -> f32 {
        match self {
            ParamValue::Float(v) => *v,
            ParamValue::Int(v) => *v as f32,
            ParamValue::Bool(v) => *v as u8 as f32,
            ParamValue::Data(_) => 0.0,
        }
    }
}

fn read_params(b: &[u8]) -> Result<Vec<ParamValue>> {
    let mut r = Reader::new(b);
    let n = r.u32()? as usize;
    let mut out = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        out.push(match r.u32()? {
            0 => ParamValue::Float(r.f32()?),
            1 => ParamValue::Int(r.i32()?),
            2 => ParamValue::Bool(r.u8()? != 0),
            _ => {
                let size = r.u16()? as usize;
                ParamValue::Data(r.bytes(size)?.to_vec())
            }
        });
    }
    Ok(out)
}

/// Studio effect types (`BEFB`), named after the core DSP they run.
pub mod effect_type {
    pub const ECHO: u32 = 6;
    pub const FADER: u32 = 7;
    pub const LIMITER: u32 = 11;
    pub const COMPRESSOR: u32 = 18;
    pub const SFX_REVERB: u32 = 19;
    pub const PAN: u32 = 27;
    pub const THREE_EQ: u32 = 28;
    pub const LOUDNESS_METER: u32 = 30;
    pub const TRANSCEIVER: u32 = 34;
    pub const MULTIBAND_EQ: u32 = 36;
}

/// A built-in effect (`BEFF`).
#[derive(Debug, Clone)]
pub struct Effect {
    pub id: Guid,
    pub kind: u32,
    pub params: Vec<ParamValue>,
    pub bypass: bool,
}

/// A send to a return bus (`SEFF`).
#[derive(Debug, Clone)]
pub struct Send {
    pub id: Guid,
    pub return_bus: Guid,
    pub level_db: f32,
    pub bypass: bool,
}

fn read_bypass(b: Option<&[u8]>) -> bool {
    b.and_then(|b| b.get(0..4))
        .is_some_and(|w| u32::from_le_bytes(w.try_into().unwrap()) != 0)
}

/// Every object of one bank file.
#[derive(Default, Debug)]
pub struct BankModel {
    pub events: HashMap<Guid, Event>,
    pub parameters: HashMap<Guid, Parameter>,
    pub parameter_sheets: HashMap<Guid, ParameterSheet>,
    pub timelines: HashMap<Guid, Timeline>,
    pub waveforms: HashMap<Guid, Waveform>,
    pub instruments: HashMap<Guid, Instrument>,
    pub buses: HashMap<Guid, (BusKind, Bus)>,
    pub effects: HashMap<Guid, Effect>,
    pub sends: HashMap<Guid, Send>,
}

impl BankModel {
    pub fn parse(file: &[u8], root: &Chunk) -> Result<BankModel> {
        let proj = root.find(b"PROJ").ok_or(Truncated)?;
        let mut m = BankModel::default();
        let objects = |g: &[u8; 4]| raw_objects(file, proj, g);

        for o in objects(b"EVTS") {
            let e = Event::parse(o.get(b"EVTB").ok_or(Truncated)?)?;
            m.events.insert(e.id, e);
        }
        for o in objects(b"PRMS") {
            let p = Parameter::parse(o.get(b"PRMB").ok_or(Truncated)?)?;
            m.parameters.insert(p.id, p);
        }
        for o in objects(b"PMLS") {
            let p = ParameterSheet::parse(o.get(b"PMLB").ok_or(Truncated)?)?;
            m.parameter_sheets.insert(p.id, p);
        }
        for o in objects(b"TLNS") {
            let t = Timeline::parse(o.get(b"TLNB").ok_or(Truncated)?)?;
            m.timelines.insert(t.id, t);
        }
        for b in raw_leaves(file, proj, b"WAVS", b"WAV ") {
            let w = Waveform::parse(b)?;
            m.waveforms.insert(w.id, w);
        }

        let instrument_lists: [(&[u8; 4], &[u8; 4]); 6] = [
            (b"WAIS", b"WAIB"),
            (b"MUIS", b"MUIB"),
            (b"SPIS", b"SPIB"),
            (b"EVIS", b"EVIB"),
            (b"SLNS", b"SLNB"),
            (b"CMDS", b"CMDB"),
        ];
        for (group, body) in instrument_lists {
            for o in objects(group) {
                let b = o.get(body).ok_or(Truncated)?;
                let base = InstrumentBase::parse(o.get(b"INST").ok_or(Truncated)?)?;
                let playlist = || Playlist::parse(o.get(b"PLST").ok_or(Truncated)?);
                let mut r = Reader::new(b);
                let id = r.guid()?;
                let kind = match body {
                    b"WAIB" => InstrumentKind::Single {
                        waveform: r.guid()?,
                    },
                    b"MUIB" => InstrumentKind::Multi {
                        playlist: playlist()?,
                    },
                    b"SPIB" => InstrumentKind::Scatterer {
                        playlist: playlist()?,
                        raw: b.to_vec(),
                    },
                    b"EVIB" => InstrumentKind::Event {
                        event: r.guid()?,
                        raw: b.to_vec(),
                    },
                    b"SLNB" => InstrumentKind::Silence { raw: b.to_vec() },
                    _ => InstrumentKind::Command { raw: b.to_vec() },
                };
                m.instruments.insert(id, Instrument { id, base, kind });
            }
        }

        for o in objects(b"BEFX") {
            let b = o.get(b"BEFB").ok_or(Truncated)?;
            let mut r = Reader::new(b);
            let id = r.guid()?;
            let kind = r.u32()?;
            let params = o
                .get(b"PMEF")
                .map(read_params)
                .transpose()?
                .unwrap_or_default();
            m.effects.insert(
                id,
                Effect {
                    id,
                    kind,
                    params,
                    bypass: read_bypass(o.get(b"EFFB")),
                },
            );
        }
        for o in objects(b"SEFX") {
            let mut r = Reader::new(o.get(b"SEFB").ok_or(Truncated)?);
            let id = r.guid()?;
            let return_bus = r.guid()?;
            let level_db = r.f32()?;
            m.sends.insert(
                id,
                Send {
                    id,
                    return_bus,
                    level_db,
                    bypass: read_bypass(o.get(b"EFFB")),
                },
            );
        }

        let bus_lists: [(&[u8; 4], &[u8; 4], BusKind); 4] = [
            (b"GBSS", b"GBSB", BusKind::Group),
            (b"MBSS", b"MBSB", BusKind::Master),
            (b"IBSS", b"IBSB", BusKind::Input),
            (b"RBSS", b"RBSB", BusKind::Return),
        ];
        for (group, body, kind) in bus_lists {
            for o in objects(group) {
                let b = o.get(body).ok_or(Truncated)?;
                let mut r = Reader::new(b);
                let id = r.guid()?;
                let output = read_output(&mut r)?;
                let body = o
                    .get(b"BUS ")
                    .map(BusBody::parse)
                    .transpose()?
                    .unwrap_or_default();
                m.buses.insert(id, (kind, Bus { id, output, body }));
            }
        }
        Ok(m)
    }
}

/// `FMOD_STUDIO_PARAMETER_DESCRIPTION` for one of an event's parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterDescription {
    pub name: String,
    pub id: (u32, u32),
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
    pub kind: u32,
    pub flags: u32,
    pub guid: Guid,
}

impl BankModel {
    /// `EventDescription::getLength`, in milliseconds.
    pub fn event_length_ms(&self, e: &Event) -> u32 {
        let end = self.timelines.get(&e.timeline).map_or(0, Timeline::end);
        (end as u64 * 1000 / 48000) as u32
    }

    /// `EventDescription::getParameterDescriptionByIndex`, in index order.
    /// Parameters defined in another bank are looked up in `others`.
    pub fn event_parameters(&self, e: &Event, others: &[&BankModel]) -> Vec<ParameterDescription> {
        let find_sheet = |g: &Guid| {
            std::iter::once(self)
                .chain(others.iter().copied())
                .find_map(|m| m.parameter_sheets.get(g))
        };
        let find_param = |g: &Guid| {
            std::iter::once(self)
                .chain(others.iter().copied())
                .find_map(|m| m.parameters.get(g))
        };
        e.parameter_sheets
            .iter()
            .zip(&e.parameter_ids)
            .filter_map(|(sheet, id)| {
                let p = find_param(&find_sheet(sheet)?.parameter)?;
                Some(ParameterDescription {
                    name: p.name.clone(),
                    id: *id,
                    minimum: p.minimum,
                    maximum: p.maximum,
                    default: p.default,
                    kind: p.kind,
                    flags: p.api_flags(),
                    guid: p.id,
                })
            })
            .collect()
    }
}
