//! `IsoFireManager.FireSounds`: the fires that asked for a sound this
//! frame (`updateSound`) share 20 emitter slots, nearest to a listener
//! first. A fire keeps the slot it had; the others take free ones, and
//! slots nobody plays this frame stop.
//!
//! The emitters are the engine's ([`FireSoundHooks`]); this module decides
//! which fire each slot plays, its `FireSize` parameter and which event it
//! starts.

use crate::fire::manager::FireRef;

/// `new FireSounds(20)`.
pub const FIRE_SOUND_SLOTS: usize = 20;

/// The sound event a slot's emitter plays when it is not playing it yet
/// (`playSoundImpl` after `isPlaying`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireSoundEvent {
    /// `"CampfireRunning"` (`isCampfire()`).
    CampfireRunning,
    /// `"Fire"` (not smoke).
    Fire,
}

impl FireSoundEvent {
    pub fn name(self) -> &'static str {
        match self {
            FireSoundEvent::CampfireRunning => "CampfireRunning",
            FireSoundEvent::Fire => "Fire",
        }
    }
}

/// What a fire's sound depends on, read when the slots are assigned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireSoundInfo {
    /// `getObjectIndex() != -1`: the fire is still on its square.
    pub on_square: bool,
    pub smoke: bool,
    pub life_stage: i32,
    /// `isCampfire()`.
    pub campfire: bool,
    /// `FMODParameterUtils.getClosestListenerDistanceSquared(x + 0.5,
    /// y + 0.5, z)`.
    pub listener_distance_sq: f32,
}

impl FireSoundInfo {
    /// `shouldPlay(fire)`.
    pub fn should_play(&self) -> bool {
        self.on_square && (self.smoke || self.life_stage <= 4)
    }

    /// `ParameterFireSize` for the stage (`playSound`).
    pub fn fire_size(&self) -> i32 {
        match self.life_stage {
            2 | 4 => 1,
            3 => 2,
            _ => 0,
        }
    }

    /// The event `playSound` starts when its emitter is not playing it.
    pub fn event(&self) -> Option<FireSoundEvent> {
        if self.campfire {
            Some(FireSoundEvent::CampfireRunning)
        } else if !self.smoke {
            Some(FireSoundEvent::Fire)
        } else {
            None
        }
    }
}

/// The emitters of the slots (`FireSounds.Slot.emitter`, an
/// `FMODSoundEmitter` with a `ParameterFireSize`, or a `DummySoundEmitter`
/// with sound disabled). Every method defaults to doing nothing.
#[allow(unused_variables)]
pub trait FireSoundHooks {
    /// `Slot.playSound(fire)`: `emitter.setPos(x + 0.5, y + 0.5, z)`,
    /// `parameterFireSize.setSize(fire_size)`, then `event` is started with
    /// `playSoundImpl` unless the emitter is playing it already (keeping
    /// its instance), and `emitter.tick()`.
    fn fire_slot_play(
        &mut self,
        slot: usize,
        pos: (f32, f32, i32),
        fire_size: i32,
        event: Option<FireSoundEvent>,
    ) {
    }
    /// `Slot.stopPlaying()`: with no instance the emitter only ticks (when
    /// it is not empty); otherwise a sustained instance gets
    /// `triggerCue`, any other `stopAll`, and the instance is forgotten.
    fn fire_slot_stop(&mut self, slot: usize) {}
}

/// One `FireSounds.Slot`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FireSoundSlot {
    pub fire: Option<FireRef>,
    pub playing: bool,
}

/// `IsoFireManager.fireSounds`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FireSounds {
    /// `fires`: the fires that called `updateSound` since the last update.
    pub fires: Vec<FireRef>,
    pub slots: [FireSoundSlot; FIRE_SOUND_SLOTS],
}

impl Default for FireSounds {
    fn default() -> Self {
        FireSounds {
            fires: Vec::new(),
            slots: [FireSoundSlot::default(); FIRE_SOUND_SLOTS],
        }
    }
}

impl FireSounds {
    /// `addFire(fire)` (`IsoFireManager.updateSound`).
    pub fn add_fire(&mut self, r: FireRef) {
        if !self.fires.contains(&r) {
            self.fires.push(r);
        }
    }

    /// `removeFire(fire)` (`IsoFireManager.stopSound`).
    pub fn remove_fire(&mut self, r: FireRef) {
        if let Some(i) = self.fires.iter().position(|&f| f == r) {
            self.fires.remove(i);
        }
    }

    fn existing_slot(&self, r: FireRef) -> Option<usize> {
        self.slots.iter().position(|s| s.fire == Some(r))
    }

    fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| !s.playing)
    }

    fn stop_not_playing(&mut self, hooks: &mut dyn FireSoundHooks) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.playing {
                continue;
            }
            hooks.fire_slot_stop(i);
            slot.fire = None;
        }
    }

    fn play(
        &mut self,
        hooks: &mut dyn FireSoundHooks,
        slot: usize,
        r: FireRef,
        info: &FireSoundInfo,
    ) {
        let pos = (r.x as f32 + 0.5, r.y as f32 + 0.5, r.z);
        hooks.fire_slot_play(slot, pos, info.fire_size(), info.event());
        self.slots[slot].fire = Some(r);
        self.slots[slot].playing = true;
    }

    /// `update()` (single player): `info` describes each fire in `fires`.
    pub fn update(
        &mut self,
        hooks: &mut dyn FireSoundHooks,
        mut info: impl FnMut(FireRef) -> FireSoundInfo,
    ) {
        for s in &mut self.slots {
            s.playing = false;
        }
        if self.fires.is_empty() {
            self.stop_not_playing(hooks);
            return;
        }
        // Collections.sort is stable; the comparator reads the distances.
        let mut keyed: Vec<(FireRef, FireSoundInfo)> =
            self.fires.iter().map(|&r| (r, info(r))).collect();
        keyed.sort_by(|a, b| {
            let (da, db) = (a.1.listener_distance_sq, b.1.listener_distance_sq);
            if da > db {
                std::cmp::Ordering::Greater
            } else if da < db {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        self.fires = keyed.iter().map(|k| k.0).collect();
        let count = keyed.len().min(self.slots.len());
        for (r, fi) in &keyed[..count] {
            if !fi.should_play() {
                continue;
            }
            if let Some(j) = self.existing_slot(*r) {
                self.play(hooks, j, *r, fi);
            }
        }
        for (r, fi) in &keyed[..count] {
            if !fi.should_play() || self.existing_slot(*r).is_some() {
                continue;
            }
            // count <= the slot count, so a slot is always free.
            let j = self.free_slot().expect("free fire sound slot");
            self.play(hooks, j, *r, fi);
        }
        self.stop_not_playing(hooks);
        self.fires.clear();
    }

    /// `Reset()`.
    pub fn reset(&mut self, hooks: &mut dyn FireSoundHooks) {
        for (i, s) in self.slots.iter_mut().enumerate() {
            hooks.fire_slot_stop(i);
            s.fire = None;
            s.playing = false;
        }
    }
}
