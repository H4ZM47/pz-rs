//! Glue between the world sound manager (`pz-audio`) and the loaded world
//! (`pz-world`), so neither crate depends on the other.
//!
//! - [`WorldSoundAdapter`] answers `WorldSoundManager`'s world queries from
//!   an [`IsoCell`] and forwards its side effects (Lua event, fishing,
//!   zombie population, network) to a [`SoundEffects`] the caller supplies.
//! - [`ClearSoundsOnUnload`] is a [`ChunkLoadHook`] that empties a chunk's
//!   sound list when the chunk leaves the cell, then forwards to an inner
//!   hook.

use pz_audio::world_sound::{RoomAt, SoundSource, SoundWorld, WorldSound, WorldSoundManager};
use pz_world::save::chunk_file::{ChunkLoadHook, HookCtx};
use pz_world::{IsoCell, IsoChunk, WorldState};

/// Side effects of `WorldSoundManager.addSound` that live outside the world
/// (Lua events, `FishSchoolManager`, `ZombiePopulationManager`, networking).
/// Every method defaults to doing nothing.
pub trait SoundEffects {
    /// `LuaEventManager.triggerEvent("OnWorldSound", x, y, z, radius, volume, source)`.
    fn on_world_sound(
        &mut self,
        _x: i32,
        _y: i32,
        _z: i32,
        _radius: i32,
        _volume: i32,
        _source: Option<SoundSource>,
    ) {
    }
    /// `FishSchoolManager.addSoundNoise(x, y, radius / 6)`.
    fn add_fish_noise(&mut self, _x: i32, _y: i32, _radius: i32) {}
    /// `ZombiePopulationManager.addWorldSound(sound, doSend)`.
    fn population_add(&mut self, _sound: &WorldSound, _do_send: bool) {}
    /// `GameClient.sendWorldSound` / `GameServer.sendWorldSound`.
    fn send(&mut self, _sound: &WorldSound) {}
}

/// Effects that do nothing.
pub struct NoEffects;

impl SoundEffects for NoEffects {}

/// [`SoundWorld`] over the loaded cell.
pub struct WorldSoundAdapter<'a> {
    pub cell: &'a IsoCell,
    /// `SandboxOptions.instance.lore.hearing`.
    pub hearing: i32,
    /// `GameServer.server`.
    pub server: bool,
    /// `GameClient.client`.
    pub client: bool,
    pub effects: &'a mut dyn SoundEffects,
}

impl<'a> WorldSoundAdapter<'a> {
    /// An offline (neither server nor client) adapter.
    pub fn new(cell: &'a IsoCell, hearing: i32, effects: &'a mut dyn SoundEffects) -> Self {
        WorldSoundAdapter {
            cell,
            hearing,
            server: false,
            client: false,
            effects,
        }
    }

    /// [`new`](Self::new) over `state.cell`.
    pub fn from_state(
        state: &WorldState<'a>,
        hearing: i32,
        effects: &'a mut dyn SoundEffects,
    ) -> Self {
        Self::new(state.cell, hearing, effects)
    }
}

impl SoundWorld for WorldSoundAdapter<'_> {
    fn hearing_option(&self) -> i32 {
        self.hearing
    }
    fn is_server(&self) -> bool {
        self.server
    }
    fn is_client(&self) -> bool {
        self.client
    }
    /// `currentCell.getChunk(cx, cy) != null`.
    fn chunk_loaded(&self, cx: i32, cy: i32) -> bool {
        self.cell.chunks.contains_key(&(cx, cy))
    }
    /// `getGridSquare(x, y, z).getRoom()`: `getRoom` is null when
    /// `roomId == -1`.
    fn room_at(&self, x: i32, y: i32, z: i32) -> RoomAt {
        match self.cell.get_grid_square(x, y, z) {
            None => RoomAt::NoSquare,
            Some(sq) if sq.borrow().room_id == -1 => RoomAt::Outside,
            Some(sq) => RoomAt::Room(sq.borrow().room_id as u64),
        }
    }
    fn on_world_sound(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
        volume: i32,
        source: Option<SoundSource>,
    ) {
        self.effects.on_world_sound(x, y, z, radius, volume, source);
    }
    fn add_fish_noise(&mut self, x: i32, y: i32, radius: i32) {
        self.effects.add_fish_noise(x, y, radius);
    }
    fn population_add(&mut self, sound: &WorldSound, do_send: bool) {
        self.effects.population_add(sound, do_send);
    }
    fn send(&mut self, sound: &WorldSound) {
        self.effects.send(sound);
    }
}

/// A [`ChunkLoadHook`] that clears `IsoChunk.soundList` of chunks leaving
/// the cell and otherwise forwards to `inner`.
///
/// The original clears the list in `IsoChunk.resetForStore()`, which runs
/// after `removeFromWorld()` once the chunk is saved and recycled
/// (`WorldReuserThread`, `IsoChunk.reuseGridsquares`). Between the two the
/// chunk is out of the cell, so `getChunk` cannot reach its list and no
/// sound is added to it; clearing at unload is indistinguishable.
pub struct ClearSoundsOnUnload<'a> {
    pub sounds: &'a mut WorldSoundManager,
    pub inner: Option<&'a mut dyn ChunkLoadHook>,
}

impl<'a> ClearSoundsOnUnload<'a> {
    pub fn new(
        sounds: &'a mut WorldSoundManager,
        inner: Option<&'a mut dyn ChunkLoadHook>,
    ) -> Self {
        ClearSoundsOnUnload { sounds, inner }
    }
}

impl ChunkLoadHook for ClearSoundsOnUnload<'_> {
    fn load_grid_square(
        &mut self,
        chunk: &mut IsoChunk,
        x: i32,
        y: i32,
        z: i32,
        ctx: &mut HookCtx<'_>,
    ) {
        if let Some(h) = self.inner.as_deref_mut() {
            h.load_grid_square(chunk, x, y, z, ctx);
        }
    }
    fn chunk_loaded(&mut self, chunk: &mut IsoChunk, ctx: &mut HookCtx<'_>) {
        if let Some(h) = self.inner.as_deref_mut() {
            h.chunk_loaded(chunk, ctx);
        }
    }
    fn chunk_unloaded(&mut self, wx: i32, wy: i32) {
        if let Some(h) = self.inner.as_deref_mut() {
            h.chunk_unloaded(wx, wy);
        }
        self.sounds.clear_chunk(wx, wy);
    }
}

#[cfg(test)]
mod tests;
