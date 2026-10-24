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
    /// `globalIndex`: FMODManager's index of this parameter across all events.
    pub global_index: usize,
}

impl ParameterDescription {
    /// `isGlobal()`.
    pub fn is_global(&self) -> bool {
        self.flags & 4 != 0
    }
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
/// `FMOD_2D`.
pub const MODE_2D: u64 = 8;
/// `FMOD_3D`.
pub const MODE_3D: u64 = 16;

/// `FMOD_STUDIO_EVENT_PROPERTY` indices used by the game.
pub const EVENT_PROPERTY_MINIMUM_DISTANCE: i32 = 3;
pub const EVENT_PROPERTY_MAXIMUM_DISTANCE: i32 = 4;

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

    // Calls the sound emitters make; defaults do nothing.

    /// `FMOD_Studio_EventInstance_SetParameterByID(inst, id, value, false)`.
    fn instance_set_parameter_by_id(
        &mut self,
        _inst: InstanceHandle,
        _id: (u32, u32),
        _value: f32,
    ) {
    }
    /// `FMOD_Studio_System_SetParameterByID(id, value, false)`.
    fn system_set_parameter_by_id(&mut self, _id: (u32, u32), _value: f32) {}
    /// `FMOD_Studio_EventInstance_SetProperty(inst, property, value)`.
    fn instance_set_property(&mut self, _inst: InstanceHandle, _property: i32, _value: f32) {}
    /// `FMOD_Studio_EventInstance_KeyOff`.
    fn instance_key_off(&mut self, _inst: InstanceHandle) {}
    /// `FMOD_Studio_GetTimelinePosition` (ms).
    fn instance_timeline_position(&mut self, _inst: InstanceHandle) -> i64 {
        0
    }
    /// `FMOD_Studio_EventInstance_SetTimelinePosition` (ms).
    fn instance_set_timeline_position(&mut self, _inst: InstanceHandle, _ms: i32) {}
    /// `FMOD_Studio_EventInstance3D(inst, x, y, z)`.
    fn instance_set_3d_position(&mut self, _inst: InstanceHandle, _x: f32, _y: f32, _z: f32) {}
    /// `FMOD_Channel_SetPriority`.
    fn channel_set_priority(&mut self, _ch: ChannelHandle, _priority: i32) {}
    /// `FMOD_Channel_SetChannelGroup(ch, FMODManager.channelGroupInGameNonBankSounds)`.
    fn channel_set_non_bank_group(&mut self, _ch: ChannelHandle) {}
    /// `FMOD_Channel_Set3DAttributes(ch, pos, vel)`.
    fn channel_set_3d_attributes(&mut self, _ch: ChannelHandle, _pos: [f32; 3], _vel: [f32; 3]) {}
    /// `FMOD_Channel_Set3DLevel`.
    fn channel_set_3d_level(&mut self, _ch: ChannelHandle, _level: f32) {}
    /// `FMOD_Channel_Set3DMinMaxDistance`.
    fn channel_set_3d_min_max_distance(&mut self, _ch: ChannelHandle, _min: f32, _max: f32) {}
    /// `FMOD_Channel_Set3DOcclusion(direct, reverb)`.
    fn channel_set_3d_occlusion(&mut self, _ch: ChannelHandle, _direct: f32, _reverb: f32) {}
    /// `FMOD_Channel_SetReverbProperties(ch, instance, wet)`.
    fn channel_set_reverb_properties(&mut self, _ch: ChannelHandle, _instance: i32, _wet: f32) {}
    /// `FMOD_System_SetReverbDefault(instance, preset)`.
    fn system_set_reverb_default(&mut self, _instance: i32, _preset: i32) {}

    // Calls SoundManager makes; defaults do nothing.

    /// `FMOD_Studio_System_GetBus(path)`; 0 if missing.
    fn get_bus(&mut self, _path: &str) -> u64 {
        0
    }
    fn bus_set_mute(&mut self, _bus: u64, _mute: bool) {}
    fn bus_set_paused(&mut self, _bus: u64, _paused: bool) {}
    /// `FMOD_Studio_System_GetVCA(path)`; 0 if missing.
    fn get_vca(&mut self, _path: &str) -> u64 {
        0
    }
    fn vca_set_volume(&mut self, _vca: u64, _volume: f32) {}
    /// `FMOD_Studio_LoadEventSampleData(desc)`.
    fn load_event_sample_data(&mut self, _desc: &EventDescription) {}
    /// Pause/unpause FMODManager's `channelGroupInGameNonBankSounds`.
    fn non_bank_group_set_paused(&mut self, _paused: bool) {}
    /// `FMOD_ChannelGroup_Stop(FMOD_System_GetMasterChannelGroup())`.
    fn master_channel_group_stop(&mut self) {}
    /// `FMOD_System_Update()`.
    fn system_update(&mut self) {}
    /// `FMOD_Studio_EventInstance_SetCallback(inst, cb, TIMELINE_MARKER)`:
    /// markers are then reported by [`AudioBackend::take_timeline_markers`].
    fn instance_watch_timeline_markers(&mut self, _inst: InstanceHandle) {}
    /// Timeline markers passed since the last call: (instance, name, ms).
    fn take_timeline_markers(&mut self) -> Vec<(InstanceHandle, String, i32)> {
        Vec::new()
    }
    /// `FMOD_Sound_Release`.
    fn sound_release(&mut self, _sound: SoundHandle) {}
    /// `FMOD_Channel_GetPosition(ch, FMOD_TIMEUNIT_MS)`.
    fn channel_position_ms(&mut self, _ch: ChannelHandle) -> u32 {
        0
    }
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
