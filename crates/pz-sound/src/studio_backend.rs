//! [`AudioBackend`] over `pz-audio`'s studio. The studio is shared with the
//! output thread, so calls take its lock briefly.
//!
//! Parameter descriptions get FMODManager's global indices: in the order
//! names are first seen while walking every event (paths sorted), which is
//! only used to index per-parameter tables.

use crate::backend::{AudioBackend, ChannelHandle, EventDescription, InstanceHandle, ParameterDescription, PlaybackState};
use pz_audio::reader::Guid;
use pz_audio::studio::{self, Studio};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

pub struct StudioBackend {
    pub studio: Arc<Mutex<Studio>>,
    descriptions: HashMap<String, Option<Rc<EventDescription>>>,
    parameters: HashMap<String, ParameterDescription>,
    guids: HashMap<u64, Guid>,
}

fn state(s: studio::PlaybackState) -> PlaybackState {
    match s {
        studio::PlaybackState::Playing => PlaybackState::Playing,
        studio::PlaybackState::Sustaining => PlaybackState::Sustaining,
        studio::PlaybackState::Stopped => PlaybackState::Stopped,
        studio::PlaybackState::Starting => PlaybackState::Starting,
        studio::PlaybackState::Stopping => PlaybackState::Stopping,
    }
}

/// Instance handles are the studio's ids plus one (0 means none).
fn id(h: InstanceHandle) -> Option<usize> {
    (h > 0).then(|| h as usize - 1)
}

impl StudioBackend {
    pub fn new(studio: Arc<Mutex<Studio>>) -> Self {
        let mut b = StudioBackend { studio, descriptions: HashMap::new(), parameters: HashMap::new(), guids: HashMap::new() };
        b.index_parameters();
        b
    }

    /// FMODManager's `parameterDescriptionMap`.
    fn index_parameters(&mut self) {
        let s = self.studio.lock().unwrap();
        for path in s.event_paths() {
            let Some(g) = s.event_by_path(&path) else { continue };
            let Some(info) = s.event_info(&g) else { continue };
            for p in info.parameters {
                let n = self.parameters.len();
                self.parameters.entry(p.name.clone()).or_insert(ParameterDescription {
                    name: p.name,
                    id: p.id,
                    minimum: p.minimum,
                    maximum: p.maximum,
                    default: p.default,
                    kind: p.kind as i32,
                    flags: p.flags,
                    global_index: n,
                });
            }
        }
    }

    /// `FMODManager.getParameterCount()`.
    pub fn parameter_count(&self) -> usize {
        self.parameters.len()
    }

    /// Every event path (`FMODManager.getEventPathList()`).
    pub fn event_paths(&self) -> Vec<String> {
        self.studio.lock().unwrap().event_paths()
    }
}

impl AudioBackend for StudioBackend {
    fn event_description(&mut self, path: &str) -> Option<Rc<EventDescription>> {
        if let Some(d) = self.descriptions.get(&path.to_lowercase()) {
            return d.clone();
        }
        let s = self.studio.lock().unwrap();
        let d = s.event_by_path(path).and_then(|g| {
            let info = s.event_info(&g)?;
            let address = u64::from_le_bytes(g.0[0..8].try_into().unwrap()) | 1;
            self.guids.insert(address, g);
            Some(Rc::new(EventDescription {
                address,
                path: info.path,
                length_ms: info.length_ms as i32,
                is_3d: info.is_3d,
                is_oneshot: info.is_oneshot,
                has_sustain_points: false,
                max_distance: info.max_distance,
                parameters: info.parameters.iter().filter_map(|p| self.parameters.get(&p.name).cloned()).collect(),
            }))
        });
        drop(s);
        self.descriptions.insert(path.to_lowercase(), d.clone());
        d
    }

    fn parameter_description(&mut self, name: &str) -> Option<ParameterDescription> {
        self.parameters.get(name).cloned()
    }

    fn create_event_instance(&mut self, desc: &EventDescription) -> InstanceHandle {
        let Some(g) = self.guids.get(&desc.address).copied() else { return -1 };
        match self.studio.lock().unwrap().create_instance(g) {
            Some(i) => i as i64 + 1,
            None => -1,
        }
    }

    fn instance_set_volume(&mut self, inst: InstanceHandle, volume: f32) {
        if let Some(i) = id(inst) {
            self.studio.lock().unwrap().set_volume(i, volume);
        }
    }

    fn instance_set_parameter_by_name(&mut self, inst: InstanceHandle, name: &str, value: f32) {
        if let Some(i) = id(inst) {
            self.studio.lock().unwrap().set_parameter_by_name(i, name, value);
        }
    }

    fn instance_start(&mut self, inst: InstanceHandle) {
        if let Some(i) = id(inst) {
            self.studio.lock().unwrap().start(i);
        }
    }

    fn instance_stop(&mut self, inst: InstanceHandle, immediate: bool) {
        if let Some(i) = id(inst) {
            self.studio.lock().unwrap().stop(i, immediate);
        }
    }

    fn instance_release(&mut self, inst: InstanceHandle) {
        if let Some(i) = id(inst) {
            self.studio.lock().unwrap().release(i);
        }
    }

    fn instance_playback_state(&mut self, inst: InstanceHandle) -> PlaybackState {
        id(inst).map_or(PlaybackState::Stopped, |i| state(self.studio.lock().unwrap().state(i)))
    }

    fn listener_3d(&mut self, index: i32, pos: [f32; 3], _vel: [f32; 3], forward: [f32; 3], up: [f32; 3]) {
        if index == 0 {
            self.studio.lock().unwrap().set_listener(pos, forward, up);
        }
    }

    // File sounds (`.ogg`/`.wav` clips) are not played by the studio yet.
    fn load_sound(&mut self, _path: &str, _looped: bool) -> u64 {
        0
    }
    fn play_sound(&mut self, _sound: u64, _paused: bool) -> ChannelHandle {
        0
    }
    fn channel_set_volume(&mut self, _ch: ChannelHandle, _volume: f32) {}
    fn channel_set_pitch(&mut self, _ch: ChannelHandle, _pitch: f32) {}
    fn channel_set_mode(&mut self, _ch: ChannelHandle, _mode: u64) {}
    fn channel_set_paused(&mut self, _ch: ChannelHandle, _paused: bool) {}
    fn channel_is_playing(&mut self, _ch: ChannelHandle) -> bool {
        false
    }
    fn channel_stop(&mut self, _ch: ChannelHandle) {}
    fn master_channel_group_set_volume(&mut self, _volume: f32) {}

    fn instance_set_parameter_by_id(&mut self, inst: InstanceHandle, pid: (u32, u32), value: f32) {
        if let Some(i) = id(inst) {
            self.studio.lock().unwrap().set_parameter_by_id(i, pid, value);
        }
    }

    fn system_set_parameter_by_id(&mut self, pid: (u32, u32), value: f32) {
        self.studio.lock().unwrap().set_global_parameter_by_id(pid, value);
    }

    fn instance_set_property(&mut self, inst: InstanceHandle, property: i32, value: f32) {
        let Some(i) = id(inst) else { return };
        let mut s = self.studio.lock().unwrap();
        match property {
            crate::backend::EVENT_PROPERTY_MINIMUM_DISTANCE => s.set_distance_override(i, Some(value), None),
            crate::backend::EVENT_PROPERTY_MAXIMUM_DISTANCE => s.set_distance_override(i, None, Some(value)),
            _ => {}
        }
    }

    fn instance_timeline_position(&mut self, inst: InstanceHandle) -> i64 {
        id(inst).map_or(0, |i| self.studio.lock().unwrap().timeline_position_ms(i))
    }

    fn instance_set_timeline_position(&mut self, inst: InstanceHandle, ms: i32) {
        if let Some(i) = id(inst) {
            self.studio.lock().unwrap().set_timeline_position_ms(i, ms);
        }
    }

    fn instance_set_3d_position(&mut self, inst: InstanceHandle, x: f32, y: f32, z: f32) {
        if let Some(i) = id(inst) {
            self.studio.lock().unwrap().set_position(i, [x, y, z]);
        }
    }
}
