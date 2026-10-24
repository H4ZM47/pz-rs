//! `fmod.fmod.FMODSoundEmitter`: plays game sounds at a position, either as
//! FMOD Studio events (bank sounds) or as channels playing sound files
//! (`.ogg`/`.wav` clips, mostly from mods), and keeps their volume,
//! position, parameters and occlusion up to date each tick.
//!
//! The emitter holds no game objects. What it reads from the world (players,
//! squares, rooms, line of sight, the emitter's parent object) comes through
//! [`EmitterWorld`]; FMOD calls go through [`AudioBackend`]; sound lookups
//! through [`GameSounds`].

use crate::backend::{
    AudioBackend, EventDescription, InstanceHandle, ParameterDescription, PlaybackState,
    EVENT_PROPERTY_MAXIMUM_DISTANCE, EVENT_PROPERTY_MINIMUM_DISTANCE, MODE_2D, MODE_3D,
    MODE_LOOP_NORMAL,
};
use crate::game_sounds::{effective_volume, GameSounds, SoundRef};
use crate::params::FmodParameter;
use std::rc::Rc;

/// `fmod.fmod.EmitterType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmitterType {
    #[default]
    Footstep,
    Voice,
    Extra,
}

/// A room as the emitter sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomInfo {
    pub id: u64,
    pub building: u64,
    /// `room.squares.size()`.
    pub num_squares: usize,
    /// `room.def.level`.
    pub level: i32,
}

/// A square as the emitter sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct SquareInfo {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub room: Option<RoomInfo>,
    /// `square.getBuilding()`.
    pub building: Option<u64>,
    /// `square.getZone().getType()`.
    pub zone_type: Option<String>,
    /// `square.isCouldSee(playerIndex)` for players 0..4.
    pub could_see: [bool; 4],
}

/// A player as the emitter sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerInfo {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub deaf: bool,
    pub square: Option<SquareInfo>,
    /// `getZi()`.
    pub zi: i32,
    /// `numNearbyBuildingsRooms`.
    pub num_nearby_buildings_rooms: f32,
    /// `getBuilding()`.
    pub building: Option<u64>,
}

/// The emitter's parent object, for remote events and file-sound occlusion.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ParentObject {
    #[default]
    None,
    /// An `IsoPlayer`.
    Player { local: bool },
    /// Another `IsoMovingObject`.
    MovingObject,
    /// An `IsoDoor` or `IsoWindow` at (x, y, z) facing north or west, on a
    /// square in `room`/`building`.
    DoorOrWindow {
        x: f32,
        y: f32,
        z: f32,
        north: bool,
        room: Option<u64>,
        building: Option<u64>,
    },
    /// Any other `IsoObject`.
    Object,
}

/// World queries the emitter makes.
pub trait EmitterWorld {
    /// `IsoPlayer.numPlayers`.
    fn num_players(&self) -> usize {
        1
    }
    /// `IsoPlayer.players[i]`.
    fn player(&self, _i: usize) -> Option<PlayerInfo> {
        None
    }
    /// `IsoPlayer.getInstance()` and its index.
    fn main_player(&self) -> Option<(usize, PlayerInfo)> {
        self.player(0).map(|p| (0, p))
    }
    /// `currentCell.getGridSquare(x, y, z)` (coordinates floored).
    fn square(&self, _x: f32, _y: f32, _z: f32) -> Option<SquareInfo> {
        None
    }
    /// `metaGrid.getRoomAt(x, y, z) != null`.
    fn meta_room_at(&self, _x: i32, _y: i32, _z: i32) -> bool {
        false
    }
    /// `System.currentTimeMillis()` captured by `FMODSoundEmitter.update()`.
    fn current_time_ms(&self) -> i64 {
        0
    }
    /// `GameServer.server`.
    fn is_server(&self) -> bool {
        false
    }
    /// `ScriptManager.getSoundTimeline(event).getPosition(name)`, -1 if none.
    fn sound_timeline_position(&self, _event: &str, _name: &str) -> i32 {
        -1
    }
}

/// The original's `IFMODParameterUpdater`.
pub trait ParameterUpdater {
    fn start_event(
        &mut self,
        inst: InstanceHandle,
        sound: &SoundRef,
        clip: usize,
        remote: bool,
        set: &[bool],
        backend: &mut dyn AudioBackend,
    );
    fn update_event(
        &mut self,
        inst: InstanceHandle,
        sound: &SoundRef,
        clip: usize,
        backend: &mut dyn AudioBackend,
    );
    fn stop_event(
        &mut self,
        inst: InstanceHandle,
        sound: &SoundRef,
        clip: usize,
        remote: bool,
        set: &[bool],
        backend: &mut dyn AudioBackend,
    );
}

/// Everything an emitter call may touch.
pub struct Ctx<'a> {
    pub backend: &'a mut dyn AudioBackend,
    pub sounds: &'a mut GameSounds,
    pub world: &'a dyn EmitterWorld,
    /// `Rand.Next(n)`.
    pub rand_next: &'a mut dyn FnMut(i32) -> i32,
    /// `ZomboidFileSystem.getAbsolutePath(path) != null`.
    pub media_file_exists: &'a dyn Fn(&str) -> bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Event {
        instance: InstanceHandle,
        stopped: InstanceHandle,
        triggered_cue: bool,
        check_time_ms: i64,
    },
    File {
        sound: u64,
        channel: u64,
        is3d: i8,
        ambient: bool,
        lx: f32,
        ly: f32,
        lz: f32,
    },
}

#[derive(Clone)]
struct Sound {
    sound: SoundRef,
    clip: usize,
    remote: bool,
    name: Option<String>,
    volume: f32,
    pitch: f32,
    /// Kept as in the original (it only feeds network messages there).
    #[allow(dead_code)]
    parent: ParentObject,
    set_volume: f32,
    set_x: f32,
    set_y: f32,
    set_z: f32,
    kind: Kind,
}

impl Sound {
    fn reference(&self) -> i64 {
        match self.kind {
            Kind::Event { instance, .. } => instance,
            Kind::File { channel, .. } => channel as i64,
        }
    }

    fn stop_immediate(&self) -> bool {
        self.sound
            .borrow()
            .game_sound
            .clips
            .get(self.clip)
            .is_some_and(|c| c.is_stop_immediate())
    }

    fn event_description(&self, sounds: &GameSounds) -> Option<Rc<EventDescription>> {
        let ev = sounds.clip_events(&self.sound).get(self.clip)?;
        if self.remote {
            ev.description_mp.clone()
        } else {
            ev.description.clone()
        }
    }

    fn has_sustain_points(&self, sounds: &GameSounds) -> bool {
        self.event_description(sounds)
            .is_some_and(|d| d.has_sustain_points)
    }

    fn is_clip_of(&self, sound: &SoundRef) -> bool {
        Rc::ptr_eq(&self.sound, sound) && self.clip < sound.borrow().game_sound.clips.len()
    }

    /// `GameSoundClip.checkReloaded()`: after a script reload, find the clip
    /// with the same event or file.
    fn check_reloaded(&mut self) {
        let s = self.sound.borrow();
        let gs = &s.game_sound;
        let Some(me) = gs.clips.get(self.clip) else {
            return;
        };
        if me.reload_epoch == gs.reload_epoch {
            return;
        }
        let mut best = None;
        for (i, other) in gs.clips.iter().enumerate() {
            if other.event.is_some() && other.event == me.event {
                best = Some(i);
            }
            if other.file.is_some() && other.file == me.file {
                best = Some(i);
            }
        }
        drop(s);
        if let Some(b) = best {
            self.clip = b;
        }
    }

    /// `getVolume()`.
    fn volume(&mut self, sounds: &GameSounds) -> f32 {
        self.check_reloaded();
        let s = self.sound.borrow();
        let clip = &s.game_sound.clips[self.clip.min(s.game_sound.clips.len().saturating_sub(1))];
        self.volume * effective_volume(&sounds.env, &s.game_sound, clip)
    }
}

struct ParameterValue {
    instance: InstanceHandle,
    global_index: usize,
    value: f32,
}

pub struct FmodSoundEmitter {
    to_start: Vec<Sound>,
    instances: Vec<Sound>,
    stopped: Vec<Sound>,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub emitter_type: EmitterType,
    pub parent: ParentObject,
    play_remote_events: bool,
    /// `ParameterOcclusion`: last pushed value and the instances using it.
    occlusion_current: f32,
    occlusion_instances: Vec<InstanceHandle>,
    pub parameters: Vec<FmodParameter>,
    pub parameter_updater: Option<Box<dyn ParameterUpdater>>,
    parameter_values: Vec<ParameterValue>,
    /// `FMODManager.getParameterCount()`, for the parameter bit set.
    parameter_count: usize,
}

const ACTION_PROGRESS: &str = "ActionProgressPercent";

impl FmodSoundEmitter {
    pub fn new(parameter_count: usize) -> Self {
        FmodSoundEmitter {
            to_start: Vec::new(),
            instances: Vec::new(),
            stopped: Vec::new(),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            emitter_type: EmitterType::Footstep,
            parent: ParentObject::None,
            play_remote_events: false,
            occlusion_current: f32::NAN,
            occlusion_instances: Vec::new(),
            parameters: Vec::new(),
            parameter_updater: None,
            parameter_values: Vec::new(),
            parameter_count,
        }
    }

    pub fn set_pos(&mut self, x: f32, y: f32, z: f32) {
        self.x = x;
        self.y = y;
        self.z = z;
    }

    pub fn set_play_remote_events(&mut self, remote: bool) {
        self.play_remote_events = remote;
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.to_start.is_empty() && self.instances.is_empty() && self.stopped.is_empty()
    }

    /// `hasSoundsToStart()`.
    pub fn has_sounds_to_start(&self) -> bool {
        !self.to_start.is_empty()
    }

    // --- stopping -------------------------------------------------------

    fn stop_sound_obj(
        &mut self,
        s: &mut Sound,
        release_event: bool,
        immediate: bool,
        ctx: &mut Ctx,
    ) {
        match s.kind {
            Kind::Event { instance, .. } => {
                if instance == 0 {
                    return;
                }
                self.stop_event(instance, s, ctx);
                ctx.backend.instance_stop(instance, immediate);
                if release_event {
                    ctx.backend.instance_release(instance);
                }
                if let Kind::Event {
                    instance: i,
                    stopped,
                    ..
                } = &mut s.kind
                {
                    *stopped = instance;
                    *i = 0;
                }
            }
            Kind::File { channel, .. } => {
                if channel == 0 {
                    return;
                }
                ctx.backend.channel_stop(channel);
                if let Kind::File { sound, channel, .. } = &mut s.kind {
                    *sound = 0;
                    *channel = 0;
                }
            }
        }
    }

    fn release(&mut self, mut s: Sound, ctx: &mut Ctx) {
        let immediate = s.stop_immediate();
        self.stop_sound_obj(&mut s, true, immediate, ctx);
    }

    /// `stopSound(ref)` (releases the event).
    pub fn stop_sound(&mut self, reference: i64, ctx: &mut Ctx) -> i32 {
        self.stop_sound_inner(reference, true, ctx)
    }

    /// `stopSoundDelayRelease(ref)`: the event finishes before release.
    pub fn stop_sound_delay_release(&mut self, reference: i64, ctx: &mut Ctx) -> i32 {
        self.stop_sound_inner(reference, false, ctx)
    }

    fn stop_sound_inner(&mut self, reference: i64, release_event: bool, ctx: &mut Ctx) -> i32 {
        let (hit, keep): (Vec<_>, Vec<_>) = self
            .to_start
            .drain(..)
            .partition(|s| s.reference() == reference);
        self.to_start = keep;
        for s in hit {
            self.release(s, ctx);
        }
        let mut i = 0;
        while i < self.instances.len() {
            if self.instances[i].reference() != reference {
                i += 1;
                continue;
            }
            let mut s = self.instances.remove(i);
            let immediate = s.stop_immediate();
            self.stop_sound_obj(&mut s, release_event, immediate, ctx);
            if release_event {
                self.release(s, ctx);
            } else {
                self.stopped.push(s);
            }
        }
        0
    }

    /// `stopSoundLocal(ref)`.
    pub fn stop_sound_local(&mut self, reference: i64, ctx: &mut Ctx) {
        self.stop_sound_inner(reference, true, ctx);
    }

    /// `stopOrTriggerSoundLocal(ref)`.
    pub fn stop_or_trigger_sound_local(&mut self, reference: i64, ctx: &mut Ctx) {
        let (hit, keep): (Vec<_>, Vec<_>) = self
            .to_start
            .drain(..)
            .partition(|s| s.reference() == reference);
        self.to_start = keep;
        for s in hit {
            self.release(s, ctx);
        }
        let mut i = 0;
        while i < self.instances.len() {
            if self.instances[i].reference() != reference {
                i += 1;
                continue;
            }
            if self.instances[i].has_sustain_points(ctx.sounds) {
                self.trigger_cue_at(i, ctx);
                i += 1;
                continue;
            }
            let s = self.instances.remove(i);
            self.release(s, ctx);
        }
    }

    fn sound_by_name(name: &str, ctx: &mut Ctx) -> Option<SoundRef> {
        let exists = ctx.media_file_exists;
        ctx.sounds.get_or_create_sound(name, ctx.backend, exists)
    }

    /// `stopSoundByName(name)`.
    pub fn stop_sound_by_name(&mut self, name: &str, ctx: &mut Ctx) -> i32 {
        let Some(gs) = Self::sound_by_name(name, ctx) else {
            return 0;
        };
        let (hit, keep): (Vec<_>, Vec<_>) =
            self.to_start.drain(..).partition(|s| s.is_clip_of(&gs));
        self.to_start = keep;
        for s in hit {
            self.release(s, ctx);
        }
        let (hit, keep): (Vec<_>, Vec<_>) =
            self.instances.drain(..).partition(|s| s.is_clip_of(&gs));
        self.instances = keep;
        for s in hit {
            self.release(s, ctx);
        }
        0
    }

    /// `stopOrTriggerSound(handle)`.
    pub fn stop_or_trigger_sound(&mut self, handle: i64, ctx: &mut Ctx) {
        if let Some(i) = self.to_start.iter().position(|s| s.reference() == handle) {
            let s = self.to_start.remove(i);
            self.release(s, ctx);
            return;
        }
        let Some(i) = self.instances.iter().position(|s| s.reference() == handle) else {
            return;
        };
        if let Kind::Event { instance, .. } = self.instances[i].kind {
            let desc = self.instances[i].event_description(ctx.sounds);
            let progress = ctx.backend.parameter_description(ACTION_PROGRESS);
            if let (Some(desc), Some(p)) = (desc, progress) {
                if desc.has_parameter(&p) {
                    if instance != 0 {
                        ctx.backend
                            .instance_set_parameter_by_id(instance, p.id, 100.0);
                    }
                    let now = ctx.world.current_time_ms();
                    if let Kind::Event {
                        triggered_cue,
                        check_time_ms,
                        ..
                    } = &mut self.instances[i].kind
                    {
                        *triggered_cue = true;
                        *check_time_ms = now;
                    }
                    return;
                }
            }
        }
        if self.instances[i].has_sustain_points(ctx.sounds) {
            self.trigger_cue_at(i, ctx);
        } else {
            let s = self.instances.remove(i);
            self.release(s, ctx);
        }
    }

    /// `stopOrTriggerSoundByName(name)`.
    pub fn stop_or_trigger_sound_by_name(&mut self, name: &str, ctx: &mut Ctx) {
        let Some(gs) = Self::sound_by_name(name, ctx) else {
            return;
        };
        let (hit, keep): (Vec<_>, Vec<_>) =
            self.to_start.drain(..).partition(|s| s.is_clip_of(&gs));
        self.to_start = keep;
        for s in hit {
            self.release(s, ctx);
        }
        let mut i = 0;
        while i < self.instances.len() {
            if !self.instances[i].is_clip_of(&gs) {
                i += 1;
                continue;
            }
            if self.instances[i].has_sustain_points(ctx.sounds) {
                self.trigger_cue_at(i, ctx);
                i += 1;
                continue;
            }
            let s = self.instances.remove(i);
            self.release(s, ctx);
        }
    }

    /// `limitSound(gameSound, maxInstances)`.
    fn limit_sound(&mut self, gs: &SoundRef, max: usize, ctx: &mut Ctx) {
        let mut total = self.to_start.iter().filter(|s| s.is_clip_of(gs)).count()
            + self.instances.iter().filter(|s| s.is_clip_of(gs)).count();
        if total <= max {
            return;
        }
        let mut i = 0;
        while i < self.to_start.len() {
            if !self.to_start[i].is_clip_of(gs) {
                i += 1;
                continue;
            }
            let s = self.to_start.remove(i);
            self.release(s, ctx);
            total -= 1;
            if total <= max {
                return;
            }
        }
        let mut i = 0;
        while i < self.instances.len() {
            if !self.instances[i].is_clip_of(gs) {
                i += 1;
                continue;
            }
            if self.instances[i].has_sustain_points(ctx.sounds) {
                if !matches!(
                    self.instances[i].kind,
                    Kind::Event {
                        triggered_cue: true,
                        ..
                    }
                ) {
                    self.trigger_cue_at(i, ctx);
                }
                i += 1;
                continue;
            }
            let s = self.instances.remove(i);
            self.release(s, ctx);
            total -= 1;
            if total <= max {
                return;
            }
        }
    }

    /// `stopAll()`.
    pub fn stop_all(&mut self, ctx: &mut Ctx) {
        for s in std::mem::take(&mut self.to_start) {
            self.release(s, ctx);
        }
        for s in std::mem::take(&mut self.instances) {
            self.release(s, ctx);
        }
    }

    // --- per-sound settings ---------------------------------------------

    /// `setVolume(ref, volume)`.
    pub fn set_volume(&mut self, reference: i64, volume: f32) {
        for s in self.to_start.iter_mut().chain(self.instances.iter_mut()) {
            if s.reference() == reference {
                s.volume = volume;
            }
        }
    }

    /// `setPitch(ref, pitch)`: the original sets every sound's pitch (only
    /// the log message checks the reference).
    pub fn set_pitch(&mut self, _reference: i64, pitch: f32) {
        for s in self.to_start.iter_mut().chain(self.instances.iter_mut()) {
            s.pitch = pitch;
        }
    }

    /// `setVolumeAll(volume)`.
    pub fn set_volume_all(&mut self, volume: f32) {
        for s in self.to_start.iter_mut().chain(self.instances.iter_mut()) {
            s.volume = volume;
        }
    }

    /// `hasSustainPoints(ref)`.
    pub fn has_sustain_points(&self, reference: i64, sounds: &GameSounds) -> bool {
        self.to_start
            .iter()
            .chain(self.instances.iter())
            .find(|s| s.reference() == reference)
            .and_then(|s| s.event_description(sounds))
            .is_some_and(|d| d.has_sustain_points)
    }

    fn trigger_cue_at(&mut self, i: usize, ctx: &mut Ctx) {
        let sustain = self.instances[i].has_sustain_points(ctx.sounds);
        let now = ctx.world.current_time_ms();
        if let Kind::Event {
            instance,
            triggered_cue,
            check_time_ms,
            ..
        } = &mut self.instances[i].kind
        {
            if *instance == 0 || !sustain {
                return;
            }
            ctx.backend.instance_key_off(*instance);
            *triggered_cue = true;
            *check_time_ms = now;
        }
    }

    /// `triggerCue(ref)`.
    pub fn trigger_cue(&mut self, reference: i64, ctx: &mut Ctx) {
        for i in 0..self.instances.len() {
            if self.instances[i].reference() == reference {
                self.trigger_cue_at(i, ctx);
            }
        }
    }

    /// `set3D(ref, is3D)` (file sounds only).
    pub fn set_3d(&mut self, reference: i64, is3d: bool, ctx: &mut Ctx) {
        let (x, y, z) = (self.x, self.y, self.z);
        for s in self.to_start.iter_mut().chain(self.instances.iter_mut()) {
            if s.reference() != reference {
                continue;
            }
            if let Kind::File {
                channel, is3d: cur, ..
            } = &mut s.kind
            {
                let want = is3d as i8;
                if *cur != want {
                    ctx.backend
                        .channel_set_mode(*channel, if is3d { MODE_3D } else { MODE_2D });
                    if is3d {
                        ctx.backend
                            .channel_set_3d_attributes(*channel, [x, y, z * 3.0], [0.0; 3]);
                    }
                    *cur = want;
                }
            }
        }
    }

    /// `isPlaying(ref)`.
    pub fn is_playing(&self, reference: i64) -> bool {
        self.to_start
            .iter()
            .chain(self.instances.iter())
            .any(|s| s.reference() == reference)
    }

    /// `isPlaying(alias)`.
    pub fn is_playing_name(&self, alias: &str) -> bool {
        self.to_start
            .iter()
            .chain(self.instances.iter())
            .any(|s| s.name.as_deref() == Some(alias))
    }

    /// `restart(handle)`.
    pub fn restart(&mut self, handle: i64, ctx: &mut Ctx) -> bool {
        if self.to_start.iter().any(|s| s.reference() == handle) {
            return true;
        }
        match self
            .instances
            .iter()
            .find(|s| s.reference() == handle)
            .map(|s| s.kind)
        {
            Some(Kind::Event { instance, .. }) if instance != 0 => {
                ctx.backend.instance_start(instance);
                true
            }
            _ => false,
        }
    }

    // --- parameters -----------------------------------------------------

    pub fn add_parameter(&mut self, p: FmodParameter) {
        self.parameters.push(p);
    }

    /// `setParameterValue(ref, description, value)`.
    pub fn set_parameter_value(
        &mut self,
        reference: i64,
        d: Option<&ParameterDescription>,
        value: f32,
        ctx: &mut Ctx,
    ) {
        let Some(d) = d else { return };
        if reference == 0 {
            return;
        }
        if let Some(s) = self.instances.iter().find(|s| s.reference() == reference) {
            if let Kind::Event { instance, .. } = s.kind {
                if instance != 0 {
                    ctx.backend
                        .instance_set_parameter_by_id(instance, d.id, value);
                }
            }
            return;
        }
        if let Some(pv) = self
            .parameter_values
            .iter_mut()
            .find(|p| p.instance == reference && p.global_index == d.global_index)
        {
            pv.value = value;
            return;
        }
        if self
            .stopped
            .iter()
            .any(|s| matches!(s.kind, Kind::Event { stopped, .. } if stopped == reference))
        {
            ctx.backend
                .instance_set_parameter_by_id(reference, d.id, value);
            return;
        }
        if !self.to_start.iter().any(|s| s.reference() == reference) {
            return;
        }
        self.parameter_values.push(ParameterValue {
            instance: reference,
            global_index: d.global_index,
            value,
        });
    }

    /// `setParameterValueByName(ref, name, value)`.
    pub fn set_parameter_value_by_name(
        &mut self,
        reference: i64,
        name: &str,
        value: f32,
        ctx: &mut Ctx,
    ) {
        let d = ctx.backend.parameter_description(name);
        self.set_parameter_value(reference, d.as_ref(), value, ctx);
    }

    /// `isUsingParameter(handle, name)`.
    pub fn is_using_parameter(&self, handle: i64, name: &str, ctx: &mut Ctx) -> bool {
        let Some(p) = ctx.backend.parameter_description(name) else {
            return false;
        };
        let s = self
            .to_start
            .iter()
            .find(|s| s.reference() == handle)
            .or_else(|| self.instances.iter().find(|s| s.reference() == handle));
        s.and_then(|s| s.event_description(ctx.sounds))
            .is_some_and(|d| d.has_parameter(&p))
    }

    /// `setTimelinePosition(ref, name)` (sounds not started yet only).
    pub fn set_timeline_position(&mut self, reference: i64, position_name: &str, ctx: &mut Ctx) {
        if reference == 0 {
            return;
        }
        let Some(s) = self.to_start.iter().find(|s| s.reference() == reference) else {
            return;
        };
        let Kind::Event { instance, .. } = s.kind else {
            return;
        };
        let event = s
            .sound
            .borrow()
            .game_sound
            .clips
            .get(s.clip)
            .and_then(|c| c.event.clone());
        let Some(event) = event else { return };
        let pos = ctx.world.sound_timeline_position(&event, position_name);
        if instance != 0 && pos != -1 {
            ctx.backend.instance_set_timeline_position(instance, pos);
        }
    }

    /// `clearParameters()`.
    pub fn clear_parameters(&mut self) {
        // ParameterOcclusion.resetToDefault only clears its own field; the
        // value last pushed to FMOD is kept.
        self.parameters.clear();
        self.parameter_values.clear();
    }

    fn param_set(&self) -> Vec<bool> {
        vec![false; self.parameter_count.max(128)]
    }

    fn occlusion_description(ctx: &mut Ctx) -> Option<ParameterDescription> {
        ctx.backend.parameter_description("Occlusion")
    }

    /// `startEvent(inst, clip, remote)`: apply pending values and attach
    /// the emitter's parameters to the new instance.
    fn start_event(&mut self, inst: InstanceHandle, s: &Sound, ctx: &mut Ctx) {
        let mut set = self.param_set();
        let occlusion = Self::occlusion_description(ctx);
        let params = s
            .event_description(ctx.sounds)
            .map(|d| d.parameters.clone())
            .unwrap_or_default();
        for ep in &params {
            if let Some(pv) = self
                .parameter_values
                .iter()
                .find(|p| p.instance == inst && p.global_index == ep.global_index)
            {
                ctx.backend
                    .instance_set_parameter_by_id(inst, ep.id, pv.value);
                set[ep.global_index] = true;
                continue;
            }
            if occlusion
                .as_ref()
                .is_some_and(|o| o.global_index == ep.global_index)
            {
                self.occlusion_instances.push(inst);
                ctx.backend
                    .instance_set_parameter_by_id(inst, ep.id, self.occlusion_current);
                set[ep.global_index] = true;
                continue;
            }
            if let Some(p) = self.parameters.iter_mut().find(|p| {
                p.description()
                    .is_some_and(|d| d.global_index == ep.global_index)
            }) {
                p.start_event_instance(inst, ctx.backend);
                set[ep.global_index] = true;
            }
        }
        if let Some(u) = &mut self.parameter_updater {
            u.start_event(inst, &s.sound, s.clip, s.remote, &set, ctx.backend);
        }
    }

    fn update_event(&mut self, inst: InstanceHandle, s: &Sound, ctx: &mut Ctx) {
        if let Some(u) = &mut self.parameter_updater {
            u.update_event(inst, &s.sound, s.clip, ctx.backend);
        }
    }

    fn stop_event(&mut self, inst: InstanceHandle, s: &Sound, ctx: &mut Ctx) {
        let mut set = self.param_set();
        let occlusion = Self::occlusion_description(ctx);
        let params = s
            .event_description(ctx.sounds)
            .map(|d| d.parameters.clone())
            .unwrap_or_default();
        for ep in &params {
            if let Some(i) = self
                .parameter_values
                .iter()
                .position(|p| p.instance == inst && p.global_index == ep.global_index)
            {
                self.parameter_values.remove(i);
                set[ep.global_index] = true;
                continue;
            }
            if occlusion
                .as_ref()
                .is_some_and(|o| o.global_index == ep.global_index)
            {
                if let Some(i) = self.occlusion_instances.iter().position(|&x| x == inst) {
                    self.occlusion_instances.remove(i);
                }
                set[ep.global_index] = true;
                continue;
            }
            if let Some(p) = self.parameters.iter_mut().find(|p| {
                p.description()
                    .is_some_and(|d| d.global_index == ep.global_index)
            }) {
                p.stop_event_instance(inst);
                set[ep.global_index] = true;
            }
        }
        if let Some(u) = &mut self.parameter_updater {
            u.stop_event(inst, &s.sound, s.clip, s.remote, &set, ctx.backend);
        }
    }

    /// `ParameterOcclusion.calculateCurrentValue()`.
    fn calculate_occlusion(&self, world: &dyn EmitterWorld) -> f32 {
        let mut occ = 1.0f32;
        for i in 0..4 {
            let v = match world.player(i) {
                None => 1.0,
                Some(p) => {
                    let sq = world.square(self.x, self.y, self.z);
                    match (&p.square, sq) {
                        (Some(_), Some(sq)) if !sq.could_see[i] => 1.0,
                        _ => 0.0,
                    }
                }
            };
            occ = occ.min(v);
        }
        ((occ * 1000.0) as i32) as f32 / 1000.0
    }

    fn update_occlusion(&mut self, ctx: &mut Ctx) {
        let v = self.calculate_occlusion(ctx.world);
        if v == self.occlusion_current {
            return;
        }
        self.occlusion_current = v;
        if let Some(d) = Self::occlusion_description(ctx) {
            for &inst in &self.occlusion_instances {
                ctx.backend.instance_set_parameter_by_id(inst, d.id, v);
            }
        }
    }

    // --- playing --------------------------------------------------------

    /// `playSoundImpl(name, parent)`: pick a random clip of the named sound.
    pub fn play_sound_impl(&mut self, name: &str, parent: ParentObject, ctx: &mut Ctx) -> i64 {
        let Some(gs) = Self::sound_by_name(name, ctx) else {
            return 0;
        };
        let n = gs.borrow().game_sound.clips.len();
        let clip = (ctx.rand_next)(n as i32) as usize;
        self.play_clip(gs, clip, parent, ctx)
    }

    /// `playSound(name)` on a single-player client (no network packets).
    pub fn play_sound(&mut self, name: &str, ctx: &mut Ctx) -> i64 {
        if ctx.world.is_server() {
            return 0;
        }
        self.play_sound_impl(name, ParentObject::None, ctx)
    }

    /// `playSound(name, x, y, z)`.
    pub fn play_sound_at(&mut self, name: &str, x: i32, y: i32, z: i32, ctx: &mut Ctx) -> i64 {
        self.set_pos(x as f32, y as f32, z as f32);
        self.play_sound(name, ctx)
    }

    /// `playSoundLoopedImpl(name)`.
    pub fn play_sound_looped_impl(&mut self, name: &str, ctx: &mut Ctx) -> i64 {
        self.play_sound_impl(name, ParentObject::None, ctx)
    }

    /// `playClip(clip, parent)`.
    pub fn play_clip(
        &mut self,
        sound: SoundRef,
        clip: usize,
        parent: ParentObject,
        ctx: &mut Ctx,
    ) -> i64 {
        self.add_sound(sound, clip, 1.0, parent, ctx).unwrap_or(0)
    }

    /// `playAmbientSound(name)`: file sounds skip the room reverb boost.
    pub fn play_ambient_sound(&mut self, name: &str, ctx: &mut Ctx) -> i64 {
        if ctx.world.is_server() {
            return 0;
        }
        let Some(gs) = Self::sound_by_name(name, ctx) else {
            return 0;
        };
        let n = gs.borrow().game_sound.clips.len();
        let clip = (ctx.rand_next)(n as i32) as usize;
        let r = self.add_sound(gs, clip, 1.0, ParentObject::None, ctx);
        if let Some(r) = r {
            if let Some(s) = self.to_start.iter_mut().find(|s| s.reference() == r) {
                if let Kind::File { ambient, .. } = &mut s.kind {
                    *ambient = true;
                }
            }
        }
        r.unwrap_or(0)
    }

    /// `playAmbientLoopedImpl(name)`.
    pub fn play_ambient_looped_impl(&mut self, name: &str, ctx: &mut Ctx) -> i64 {
        if ctx.world.is_server() {
            return 0;
        }
        let Some(gs) = Self::sound_by_name(name, ctx) else {
            return 0;
        };
        let n = gs.borrow().game_sound.clips.len();
        let clip = (ctx.rand_next)(n as i32) as usize;
        self.add_sound(gs, clip, 1.0, ParentObject::None, ctx)
            .unwrap_or(0)
    }

    fn should_play_remote_events(&self) -> bool {
        matches!(self.parent, ParentObject::Player { local: false }) || self.play_remote_events
    }

    /// `addSound(clip, volume, parent)`.
    fn add_sound(
        &mut self,
        sound: SoundRef,
        clip: usize,
        volume: f32,
        parent: ParentObject,
        ctx: &mut Ctx,
    ) -> Option<i64> {
        let (max_instances, event, file, name, distances, priority, pitch, distance_max) = {
            let s = sound.borrow();
            let gs = &s.game_sound;
            let c = gs.clips.get(clip)?;
            (
                gs.max_instances_per_emitter,
                c.event.clone(),
                c.file.clone(),
                gs.name.clone(),
                (
                    c.has_min_distance().then_some(c.distance_min),
                    c.has_max_distance().then_some(c.distance_max),
                ),
                c.priority,
                c.pitch,
                c.distance_max,
            )
        };
        if max_instances > 0 {
            self.limit_sound(&sound, (max_instances - 1) as usize, ctx);
        }
        let blank = |s: &Option<String>| s.as_deref().is_none_or(str::is_empty);
        if !blank(&event) {
            let ev = ctx
                .sounds
                .clip_events(&sound)
                .get(clip)
                .cloned()
                .unwrap_or_default();
            let mut desc = ev.description.clone()?;
            let mut remote = false;
            if let Some(mp) = &ev.description_mp {
                if self.should_play_remote_events() {
                    desc = mp.clone();
                    remote = true;
                }
            }
            let inst = ctx.backend.create_event_instance(&desc);
            if inst < 0 {
                return None;
            }
            if let Some(d) = distances.0 {
                ctx.backend
                    .instance_set_property(inst, EVENT_PROPERTY_MINIMUM_DISTANCE, d);
            }
            if let Some(d) = distances.1 {
                ctx.backend
                    .instance_set_property(inst, EVENT_PROPERTY_MAXIMUM_DISTANCE, d);
            }
            self.to_start.push(Sound {
                sound,
                clip,
                remote,
                name,
                volume,
                pitch: 1.0,
                parent,
                set_volume: 1.0,
                set_x: 0.0,
                set_y: 0.0,
                set_z: 0.0,
                kind: Kind::Event {
                    instance: inst,
                    stopped: 0,
                    triggered_cue: false,
                    check_time_ms: 0,
                },
            });
            return Some(inst);
        }
        if !blank(&file) {
            let file = file.unwrap();
            let snd = ctx.backend.load_sound(&file, false);
            if snd == 0 {
                return None;
            }
            let ch = ctx.backend.play_sound(snd, true);
            ctx.backend.channel_set_volume(ch, 0.0);
            ctx.backend.channel_set_priority(ch, 9 - priority);
            ctx.backend.channel_set_non_bank_group(ch);
            if distance_max == 0.0 || (self.x == 0.0 && self.y == 0.0) {
                ctx.backend.channel_set_mode(ch, MODE_2D);
            }
            self.to_start.push(Sound {
                sound,
                clip,
                remote: false,
                name,
                volume,
                pitch,
                parent,
                set_volume: 1.0,
                set_x: 0.0,
                set_y: 0.0,
                set_z: 0.0,
                kind: Kind::File {
                    sound: snd,
                    channel: ch,
                    is3d: -1,
                    ambient: false,
                    lx: 0.0,
                    ly: 0.0,
                    lz: 0.0,
                },
            });
            return Some(ch as i64);
        }
        None
    }

    // --- tick -----------------------------------------------------------

    /// `tick()`.
    pub fn tick(&mut self, ctx: &mut Ctx) {
        if !self.is_empty() {
            self.update_occlusion(ctx);
            for p in &mut self.parameters {
                p.update(ctx.backend);
            }
        }
        let starting = self.to_start.len();
        let first_new = self.instances.len();
        self.instances.append(&mut self.to_start);
        let mut i = 0;
        while i < self.instances.len() {
            let is_starting = i >= first_new && i < first_new + starting;
            let mut s = self.instances[i].clone();
            let done = self.tick_sound(&mut s, is_starting, ctx);
            if done {
                self.instances.remove(i);
                self.release(s, ctx);
            } else {
                self.instances[i] = s;
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.stopped.len() {
            let done = match self.stopped[i].kind {
                Kind::Event { stopped, .. } => {
                    if ctx.backend.instance_playback_state(stopped) == PlaybackState::Stopped {
                        ctx.backend.instance_release(stopped);
                        if let Kind::Event { stopped, .. } = &mut self.stopped[i].kind {
                            *stopped = 0;
                        }
                        true
                    } else {
                        false
                    }
                }
                Kind::File { .. } => true,
            };
            if done {
                let s = self.stopped.remove(i);
                self.release(s, ctx);
            } else {
                i += 1;
            }
        }
    }

    fn tick_sound(&mut self, s: &mut Sound, is_starting: bool, ctx: &mut Ctx) -> bool {
        match s.kind {
            Kind::Event { .. } => self.tick_event(s, is_starting, ctx),
            Kind::File { .. } => self.tick_file(s, is_starting, ctx),
        }
    }

    /// `EventSound.tick(isStarting)`.
    fn tick_event(&mut self, s: &mut Sound, is_starting: bool, ctx: &mut Ctx) -> bool {
        let Kind::Event {
            instance,
            triggered_cue,
            check_time_ms,
            ..
        } = s.kind
        else {
            return false;
        };
        let now = ctx.world.current_time_ms();
        if !is_starting {
            let state = ctx.backend.instance_playback_state(instance);
            if state == PlaybackState::Stopping {
                return false;
            }
            if state == PlaybackState::Stopped {
                ctx.backend.instance_release(instance);
                self.stop_event(instance, s, ctx);
                if let Kind::Event { instance, .. } = &mut s.kind {
                    *instance = 0;
                }
                return true;
            }
            if triggered_cue && now - check_time_ms > 250 && state == PlaybackState::Sustaining {
                ctx.backend.instance_key_off(instance);
            }
            let length = s
                .event_description(ctx.sounds)
                .map_or(0, |d| d.length_ms as i64);
            if triggered_cue && length > 0 && now - check_time_ms > 1500 {
                let position = ctx.backend.instance_timeline_position(instance);
                if position > length + 1000 {
                    ctx.backend.instance_stop(instance, false);
                }
                if let Kind::Event { check_time_ms, .. } = &mut s.kind {
                    *check_time_ms = now;
                }
            }
        }
        // Float.compare: NaN equals NaN and -0.0 differs from 0.0.
        let differs = |a: f32, b: f32| a.to_bits() != b.to_bits() && !(a.is_nan() && b.is_nan());
        if differs(self.x, s.set_x) || differs(self.y, s.set_y) || differs(self.z, s.set_z) {
            s.set_x = self.x;
            s.set_y = self.y;
            s.set_z = self.z;
            ctx.backend
                .instance_set_3d_position(instance, self.x, self.y, self.z * 3.0);
        }
        let v = s.volume(ctx.sounds);
        if differs(v, s.set_volume) {
            s.set_volume = v;
            ctx.backend.instance_set_volume(instance, v);
        }
        if is_starting {
            self.start_event(instance, s, ctx);
            ctx.backend.instance_start(instance);
        } else {
            self.update_event(instance, s, ctx);
        }
        false
    }

    /// `FileSound.tick(isStarting)`.
    fn tick_file(&mut self, s: &mut Sound, is_starting: bool, ctx: &mut Ctx) -> bool {
        let Kind::File {
            channel,
            is3d,
            ambient,
            lx,
            ly,
            ..
        } = s.kind
        else {
            return false;
        };
        let (looped, is3d_sound, distance_min, distance_max, reverb_max_range, reverb_factor) = {
            let g = s.sound.borrow();
            let c = &g.game_sound.clips[s.clip.min(g.game_sound.clips.len() - 1)];
            (
                g.game_sound.loop_,
                g.game_sound.is3d,
                c.distance_min,
                c.distance_max,
                c.reverb_max_range,
                c.reverb_factor,
            )
        };
        let b = &mut *ctx.backend;
        if is_starting && looped {
            b.channel_set_mode(channel, MODE_LOOP_NORMAL);
        }
        let range = distance_min;
        if !is_starting && !b.channel_is_playing(channel) {
            return true;
        }
        let (x, y, z) = (self.x, self.y, self.z);
        if !is3d_sound || (x == 0.0 && y == 0.0) {
            let at_origin = x == 0.0 && y == 0.0;
            if !(at_origin || (!is_starting && x == lx && y == ly) || is3d != 1) {
                b.channel_set_3d_attributes(channel, [x, y, z * 3.0], [0.0; 3]);
            }
            let v = s.volume(ctx.sounds);
            let b = &mut *ctx.backend;
            b.channel_set_volume(channel, v);
            b.channel_set_pitch(channel, s.pitch);
            if is_starting {
                b.channel_set_paused(channel, false);
            }
            return false;
        }
        // The original sets lx/ly/lz before computing the velocity, so it is 0.
        b.channel_set_3d_attributes(channel, [x, y, z * 3.0], [0.0, 0.0, 0.0]);
        let mut min_dist = f32::MAX;
        for i in 0..ctx.world.num_players() {
            let Some(p) = ctx.world.player(i) else {
                continue;
            };
            if p.deaf {
                continue;
            }
            let d = ((x - p.x).powi(2) as f64
                + (y - p.y).powi(2) as f64
                + (z * 3.0 - p.z * 3.0).powi(2) as f64)
                .sqrt() as f32;
            min_dist = min_dist.min(d);
        }
        let level = if min_dist >= 2.0 {
            1.0
        } else {
            1.0 - (2.0 - min_dist) / 2.0
        };
        b.channel_set_3d_level(channel, level);
        let v = s.volume(ctx.sounds);
        let b = &mut *ctx.backend;
        if ctx.world.num_players() > 1 {
            if is_starting {
                b.system_set_reverb_default(0, 0);
                b.channel_set_3d_min_max_distance(channel, distance_min, distance_max);
                b.channel_set_3d_occlusion(channel, 0.0, 0.0);
            }
            b.channel_set_volume(channel, v);
            if is_starting {
                b.channel_set_paused(channel, false);
            }
            b.channel_set_reverb_properties(channel, 0, 0.0);
            b.channel_set_reverb_properties(channel, 1, 0.0);
            b.system_set_reverb_default(1, 0);
            b.channel_set_3d_occlusion(channel, 0.0, 0.0);
            return false;
        }
        let Some((pi, me)) = ctx.world.main_player() else {
            return false;
        };
        let mut reverb =
            ((x - me.x).abs() + (y - me.y).abs() + (z - me.z).abs() * 2.0) / reverb_max_range;
        let Some(current) = me.square.clone() else {
            b.channel_set_3d_min_max_distance(channel, range, distance_max);
            b.channel_set_volume(channel, v);
            if is_starting {
                b.channel_set_paused(channel, false);
            }
            return false;
        };
        match current.room {
            None => {
                if !ambient {
                    reverb += me.num_nearby_buildings_rooms / 32.0;
                    reverb += 0.08;
                }
            }
            Some(r) => {
                if !ambient {
                    reverb += r.num_squares as f32 / 500.0;
                }
            }
        }
        reverb = reverb.min(1.0);
        reverb *= reverb;
        reverb *= reverb;
        reverb *= reverb_factor;
        reverb *= 10.0;
        if current.room.is_none() && reverb < 0.1 {
            reverb = 0.1;
        }
        let _ = reverb; // the original computes it but never applies it
        let (mut preset, mut off_a, mut off_b) = if !ambient && current.room.is_some() {
            (0, 1, 2)
        } else {
            (2, 0, 1)
        };
        let sq = ctx.world.square(x, y, z);
        if sq
            .as_ref()
            .and_then(|s| s.zone_type.as_deref())
            .is_some_and(|t| t == "Forest" || t == "DeepForest")
        {
            preset = 1;
            off_a = 0;
            off_b = 2;
        }
        b.channel_set_reverb_properties(channel, preset, 0.0);
        b.channel_set_reverb_properties(channel, off_a, 0.0);
        b.channel_set_reverb_properties(channel, off_b, 0.0);
        b.channel_set_3d_min_max_distance(channel, range, distance_max);
        let mut direct = 0.0f32;
        let mut reverb_occ = 0.0f32;
        let player_room = current.room;
        match &sq {
            Some(sq2) => {
                if let ParentObject::DoorOrWindow {
                    x: dx,
                    y: dy,
                    z: dz,
                    north,
                    room: parent_room,
                    building: parent_building,
                } = &self.parent
                {
                    let r = player_room;
                    if r.map(|r| r.id) != *parent_room {
                        if r.is_some_and(|r| Some(r.building) == *parent_building) {
                            direct = 0.33;
                            reverb_occ = 0.33;
                        } else {
                            let other = if *north {
                                ctx.world.square(*dx, dy - 1.0, *dz)
                            } else {
                                ctx.world.square(dx - 1.0, *dy, *dz)
                            };
                            if let Some(other) = other {
                                if r.is_some() || other.room.is_none() {
                                    if let (Some(r), Some(oroom)) = (r, other.room) {
                                        if Some(r.building) == other.building {
                                            if r.id != oroom.id {
                                                if r.level == other.z {
                                                    direct = 0.33;
                                                    reverb_occ = 0.33;
                                                } else {
                                                    direct = 0.6;
                                                    reverb_occ = 0.6;
                                                }
                                            }
                                        } else {
                                            direct = 0.33;
                                            reverb_occ = 0.33;
                                        }
                                    } else {
                                        direct = 0.33;
                                        reverb_occ = 0.33;
                                    }
                                }
                            }
                        }
                    }
                } else if let Some(sroom) = sq2.room {
                    match player_room {
                        None => {
                            direct = 0.33;
                            reverb_occ = 0.23;
                        }
                        Some(r) => {
                            if r.id != sroom.id {
                                direct = 0.24;
                                reverb_occ = 0.24;
                            }
                            if sroom.building != r.building {
                                direct = 1.0;
                                reverb_occ = 0.8;
                            }
                            if sroom.level != me.zi {
                                direct = 0.6;
                                reverb_occ = 0.6;
                            }
                        }
                    }
                } else if player_room.is_some() {
                    direct = 0.79;
                    reverb_occ = 0.59;
                }
                if !sq2.could_see[pi.min(3)] && *sq2 != current {
                    direct += 0.4;
                }
            }
            None => {
                if ctx
                    .world
                    .meta_room_at(x.floor() as i32, y.floor() as i32, z.floor() as i32)
                {
                    direct = 1.0;
                    reverb_occ = 1.0;
                }
                direct += if player_room.is_some() { 0.94 } else { 0.6 };
            }
        }
        if let Some(sq2) = &sq {
            if me.zi != sq2.z {
                direct *= 1.3;
            }
        }
        direct = direct.min(0.9);
        reverb_occ = reverb_occ.min(0.9);
        if self.emitter_type == EmitterType::Footstep
            && z > me.z
            && sq.as_ref().is_some_and(|s| s.building == me.building)
        {
            direct = 0.0;
            reverb_occ = 0.0;
        }
        if s.name.as_deref() == Some("HouseAlarm") {
            direct = 0.0;
            reverb_occ = 0.0;
        }
        b.channel_set_3d_occlusion(channel, direct, reverb_occ);
        b.channel_set_volume(channel, v);
        b.channel_set_pitch(channel, s.pitch);
        if is_starting {
            b.channel_set_paused(channel, false);
        }
        if let Kind::File { lx, ly, lz, .. } = &mut s.kind {
            *lx = x;
            *ly = y;
            *lz = z;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{ChannelHandle, SoundHandle};
    use crate::game_sounds::SoundEnv;
    use pz_scripts::objects::models::game_sound::{GameSoundClip, GameSoundScript};
    use std::cell::RefCell;

    #[derive(Default)]
    struct Mock {
        calls: Vec<String>,
        state: PlaybackState,
        next: i64,
        sustain: bool,
    }

    impl Default for PlaybackState {
        fn default() -> Self {
            PlaybackState::Playing
        }
    }

    fn pdesc(name: &str, idx: usize) -> ParameterDescription {
        ParameterDescription {
            name: name.into(),
            id: (idx as u32, 0),
            minimum: 0.0,
            maximum: 100.0,
            default: 0.0,
            kind: 0,
            flags: 0,
            global_index: idx,
        }
    }

    impl AudioBackend for Mock {
        fn event_description(&mut self, path: &str) -> Option<Rc<EventDescription>> {
            (!path.contains("Remote")).then(|| {
                Rc::new(EventDescription {
                    address: 1,
                    path: path.into(),
                    length_ms: 1000,
                    is_3d: true,
                    is_oneshot: false,
                    has_sustain_points: self.sustain,
                    max_distance: 20.0,
                    parameters: vec![
                        pdesc("Occlusion", 0),
                        pdesc("ActionProgressPercent", 1),
                        pdesc("Speed", 2),
                    ],
                })
            })
        }
        fn parameter_description(&mut self, name: &str) -> Option<ParameterDescription> {
            match name {
                "Occlusion" => Some(pdesc(name, 0)),
                "ActionProgressPercent" => Some(pdesc(name, 1)),
                "Speed" => Some(pdesc(name, 2)),
                _ => None,
            }
        }
        fn create_event_instance(&mut self, _d: &EventDescription) -> InstanceHandle {
            self.next += 1;
            self.calls.push(format!("create {}", self.next));
            self.next
        }
        fn instance_set_volume(&mut self, i: InstanceHandle, v: f32) {
            self.calls.push(format!("volume {i} {v}"));
        }
        fn instance_set_parameter_by_name(&mut self, _i: InstanceHandle, _n: &str, _v: f32) {}
        fn instance_start(&mut self, i: InstanceHandle) {
            self.calls.push(format!("start {i}"));
        }
        fn instance_stop(&mut self, i: InstanceHandle, imm: bool) {
            self.calls.push(format!("stop {i} {imm}"));
        }
        fn instance_release(&mut self, i: InstanceHandle) {
            self.calls.push(format!("release {i}"));
        }
        fn instance_playback_state(&mut self, _i: InstanceHandle) -> PlaybackState {
            self.state
        }
        fn listener_3d(&mut self, _i: i32, _p: [f32; 3], _v: [f32; 3], _f: [f32; 3], _u: [f32; 3]) {
        }
        fn load_sound(&mut self, path: &str, _l: bool) -> SoundHandle {
            self.calls.push(format!("load {path}"));
            5
        }
        fn play_sound(&mut self, _s: SoundHandle, paused: bool) -> ChannelHandle {
            self.calls.push(format!("play paused={paused}"));
            77
        }
        fn channel_set_volume(&mut self, c: ChannelHandle, v: f32) {
            self.calls.push(format!("chvol {c} {v}"));
        }
        fn channel_set_pitch(&mut self, _c: ChannelHandle, _p: f32) {}
        fn channel_set_mode(&mut self, c: ChannelHandle, m: u64) {
            self.calls.push(format!("chmode {c} {m}"));
        }
        fn channel_set_paused(&mut self, c: ChannelHandle, p: bool) {
            self.calls.push(format!("chpause {c} {p}"));
        }
        fn channel_is_playing(&mut self, _c: ChannelHandle) -> bool {
            true
        }
        fn channel_stop(&mut self, c: ChannelHandle) {
            self.calls.push(format!("chstop {c}"));
        }
        fn master_channel_group_set_volume(&mut self, _v: f32) {}
        fn instance_set_parameter_by_id(&mut self, i: InstanceHandle, id: (u32, u32), v: f32) {
            self.calls.push(format!("param {i} {} {v}", id.0));
        }
        fn instance_set_property(&mut self, i: InstanceHandle, p: i32, v: f32) {
            self.calls.push(format!("prop {i} {p} {v}"));
        }
        fn instance_key_off(&mut self, i: InstanceHandle) {
            self.calls.push(format!("keyoff {i}"));
        }
        fn instance_set_3d_position(&mut self, i: InstanceHandle, x: f32, y: f32, z: f32) {
            self.calls.push(format!("pos {i} {x} {y} {z}"));
        }
    }

    struct World;
    impl EmitterWorld for World {}

    fn sounds(names: &[(&str, Option<&str>, Option<&str>)], b: &mut Mock) -> GameSounds {
        let mut g = GameSounds::new(SoundEnv::default());
        for (name, event, file) in names {
            let mut s = GameSoundScript::new();
            s.game_sound.name = Some(name.to_string());
            let mut c = GameSoundClip::new(&s.game_sound);
            c.event = event.map(str::to_string);
            c.file = file.map(str::to_string);
            c.volume = 0.5;
            c.distance_max = 30.0;
            c.init_flags = 2;
            s.game_sound.clips.push(c);
            g.add_sound(Rc::new(RefCell::new(s)), b);
        }
        g
    }

    macro_rules! ctx {
        ($b:expr, $g:expr) => {
            Ctx {
                backend: &mut $b,
                sounds: &mut $g,
                world: &World,
                rand_next: &mut |_| 0,
                media_file_exists: &|_| false,
            }
        };
    }

    #[test]
    fn event_sound_lifecycle() {
        let mut b = Mock::default();
        let mut g = sounds(&[("Step", Some("Foot/Step"), None)], &mut b);
        let mut e = FmodSoundEmitter::new(8);
        e.set_pos(10.0, 20.0, 1.0);
        let r = e.play_sound("Step", &mut ctx!(b, g));
        assert_eq!(r, 1);
        assert_eq!(b.calls, ["create 1", "prop 1 4 30"]);
        // A value set before the sound starts is applied when it starts.
        e.set_parameter_value_by_name(r, "Speed", 3.0, &mut ctx!(b, g));
        b.calls.clear();
        e.tick(&mut ctx!(b, g));
        // With no players every square counts as occluded (1.0).
        assert_eq!(
            b.calls,
            [
                "pos 1 10 20 3",
                "volume 1 0.5",
                "param 1 0 1",
                "param 1 2 3",
                "start 1"
            ]
        );
        // Later values go straight to the instance.
        b.calls.clear();
        e.set_parameter_value_by_name(r, "Speed", 4.0, &mut ctx!(b, g));
        assert_eq!(b.calls, ["param 1 2 4"]);
        // Nothing changed: the tick sends nothing.
        b.calls.clear();
        e.tick(&mut ctx!(b, g));
        assert!(b.calls.is_empty(), "{:?}", b.calls);
        assert!(e.is_playing(r) && e.is_playing_name("Step"));
        b.state = PlaybackState::Stopped;
        b.calls.clear();
        e.tick(&mut ctx!(b, g));
        assert_eq!(b.calls, ["release 1"]);
        assert!(e.is_empty());
    }

    #[test]
    fn stop_or_trigger_uses_action_progress() {
        let mut b = Mock::default();
        let mut g = sounds(&[("Saw", Some("Tools/Saw"), None)], &mut b);
        let mut e = FmodSoundEmitter::new(8);
        let r = e.play_sound("Saw", &mut ctx!(b, g));
        e.tick(&mut ctx!(b, g));
        b.calls.clear();
        e.stop_or_trigger_sound(r, &mut ctx!(b, g));
        // The event has ActionProgressPercent: it is set to 100 and kept.
        assert_eq!(b.calls, ["param 1 1 100"]);
        assert!(e.is_playing(r));
        e.stop_sound(r, &mut ctx!(b, g));
        assert!(!e.is_playing(r));
        assert_eq!(
            b.calls[1..],
            ["stop 1 false".to_string(), "release 1".to_string()]
        );
    }

    #[test]
    fn max_instances_per_emitter() {
        let mut b = Mock::default();
        let mut g = sounds(&[("Bang", Some("Bang"), None)], &mut b);
        g.sounds()[0]
            .borrow_mut()
            .game_sound
            .max_instances_per_emitter = 2;
        let mut e = FmodSoundEmitter::new(8);
        let a = e.play_sound("Bang", &mut ctx!(b, g));
        let c = e.play_sound("Bang", &mut ctx!(b, g));
        let d = e.play_sound("Bang", &mut ctx!(b, g));
        assert!(!e.is_playing(a) && e.is_playing(c) && e.is_playing(d));
        // Same call stream as the original (checked through the JVM oracle).
        assert_eq!(
            b.calls,
            [
                "create 1",
                "prop 1 4 30",
                "create 2",
                "prop 2 4 30",
                "stop 1 false",
                "release 1",
                "create 3",
                "prop 3 4 30"
            ]
        );
    }

    #[test]
    fn file_sound_at_origin_plays_2d() {
        let mut b = Mock::default();
        let mut g = sounds(&[("Mod", None, Some("media/sound/mod.ogg"))], &mut b);
        let mut e = FmodSoundEmitter::new(8);
        let r = e.play_sound("Mod", &mut ctx!(b, g));
        assert_eq!(r, 77);
        assert_eq!(
            b.calls,
            [
                "load media/sound/mod.ogg",
                "play paused=true",
                "chvol 77 0",
                "chmode 77 8"
            ]
        );
        b.calls.clear();
        e.tick(&mut ctx!(b, g));
        assert_eq!(b.calls, ["chvol 77 0.5", "chpause 77 false"]);
        e.stop_all(&mut ctx!(b, g));
        assert!(e.is_empty());
    }
}
