//! The audio engine interface, shaped after the `fmod.javafmod` calls the
//! game makes. Handles are opaque numbers; 0 means "none", negative values
//! from creation calls are errors, as in javafmod.

use std::rc::Rc;

/// `FMOD_STUDIO_PLAYBACK_STATE` (same order and indices).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing = 0,
    Sustaining = 1,
    Stopped = 2,
    Starting = 3,
    Stopping = 4,
}

/// `FMOD_STUDIO_PARAMETER_DESCRIPTION`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterDescription {
    pub name: String,
    pub id: (u32, u32),
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
    /// `FMOD_STUDIO_PARAMETER_TYPE`.
    pub kind: i32,
    /// `FMOD_STUDIO_PARAMETER_FLAGS`.
    pub flags: u32,
}

/// `fmod.fmod.FMOD_STUDIO_EVENT_DESCRIPTION`.
#[derive(Debug, Clone, PartialEq)]
pub struct EventDescription {
    /// Engine handle (`address`).
    pub address: u64,
    pub path: String,
    pub length_ms: i32,
    pub is_3d: bool,
    pub is_oneshot: bool,
    pub has_sustain_points: bool,
    pub max_distance: f32,
    pub parameters: Vec<ParameterDescription>,
}

impl EventDescription {
    /// `hasParameter(description)`: by parameter ID.
    pub fn has_parameter(&self, p: &ParameterDescription) -> bool {
        self.parameters.iter().any(|q| q.id == p.id)
    }
}

pub type InstanceHandle = i64;
pub type SoundHandle = u64;
pub type ChannelHandle = u64;

/// `FMOD_LOOP_NORMAL` for `Channel_SetMode`.
pub const MODE_LOOP_NORMAL: u64 = 2;

pub trait AudioBackend {
    /// `FMODManager.getEventDescription(path)` (`event:/...`).
    fn event_description(&mut self, path: &str) -> Option<Rc<EventDescription>>;
    /// `FMODManager.getParameterDescription(name)` for global lookups.
    fn parameter_description(&mut self, _name: &str) -> Option<ParameterDescription> {
        None
    }
    /// `FMOD_Studio_System_CreateEventInstance`.
    fn create_event_instance(&mut self, desc: &EventDescription) -> InstanceHandle;
    fn instance_set_volume(&mut self, inst: InstanceHandle, volume: f32);
    fn instance_set_parameter_by_name(&mut self, inst: InstanceHandle, name: &str, value: f32);
    fn instance_start(&mut self, inst: InstanceHandle);
    fn instance_stop(&mut self, inst: InstanceHandle, immediate: bool);
    fn instance_release(&mut self, inst: InstanceHandle);
    fn instance_playback_state(&mut self, inst: InstanceHandle) -> PlaybackState;
    /// `FMOD_Studio_Listener3D(index, pos, vel, forward, up)`.
    fn listener_3d(
        &mut self,
        index: i32,
        pos: [f32; 3],
        vel: [f32; 3],
        forward: [f32; 3],
        up: [f32; 3],
    );
    /// `FMODManager.loadSound(path, looped)`; 0 on failure.
    fn load_sound(&mut self, path: &str, looped: bool) -> SoundHandle;
    /// `FMOD_System_PlaySound(sound, paused)`.
    fn play_sound(&mut self, sound: SoundHandle, paused: bool) -> ChannelHandle;
    fn channel_set_volume(&mut self, ch: ChannelHandle, volume: f32);
    fn channel_set_pitch(&mut self, ch: ChannelHandle, pitch: f32);
    fn channel_set_mode(&mut self, ch: ChannelHandle, mode: u64);
    fn channel_set_paused(&mut self, ch: ChannelHandle, paused: bool);
    fn channel_is_playing(&mut self, ch: ChannelHandle) -> bool;
    fn channel_stop(&mut self, ch: ChannelHandle);
    /// `FMOD_ChannelGroup_SetVolume(FMOD_System_GetMasterChannelGroup(), v)`.
    fn master_channel_group_set_volume(&mut self, volume: f32);
}

/// A backend with no sound: nothing exists and nothing plays.
#[derive(Default)]
pub struct SilentBackend;

impl AudioBackend for SilentBackend {
    fn event_description(&mut self, _path: &str) -> Option<Rc<EventDescription>> {
        None
    }
    fn create_event_instance(&mut self, _desc: &EventDescription) -> InstanceHandle {
        -1
    }
    fn instance_set_volume(&mut self, _inst: InstanceHandle, _volume: f32) {}
    fn instance_set_parameter_by_name(&mut self, _inst: InstanceHandle, _name: &str, _value: f32) {}
    fn instance_start(&mut self, _inst: InstanceHandle) {}
    fn instance_stop(&mut self, _inst: InstanceHandle, _immediate: bool) {}
    fn instance_release(&mut self, _inst: InstanceHandle) {}
    fn instance_playback_state(&mut self, _inst: InstanceHandle) -> PlaybackState {
        PlaybackState::Stopped
    }
    fn listener_3d(&mut self, _i: i32, _p: [f32; 3], _v: [f32; 3], _f: [f32; 3], _u: [f32; 3]) {}
    fn load_sound(&mut self, _path: &str, _looped: bool) -> SoundHandle {
        0
    }
    fn play_sound(&mut self, _sound: SoundHandle, _paused: bool) -> ChannelHandle {
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
}
