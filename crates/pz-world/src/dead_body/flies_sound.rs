//! `FliesSound`: the looping `CorpseFlies` sound near rotting corpses and
//! the squares' flies (`IsoGridSquare.setHasFlies`), which the renderer
//! draws (`FBORenderCell.renderFlies` over `hasFlies()`).
//!
//! Per local player, the nine chunks around the player's chunk on the
//! player's level are checked when the player changes chunk, level or
//! building (or something forces it: a chunk loaded, a corpse added to or
//! removed from a checked chunk level, the `Deaf` trait changing). A chunk
//! level whose corpse count (`CorpseCount.getCorpseCount` for the player's
//! building) makes `BodyDamage.getSicknessFromCorpsesRate` exceed
//! `ZomboidGlobals.FoodSicknessDecrease` gets an emitter on the middle one
//! of its squares holding corpses in that building, which fades in and
//! gets the flies; otherwise its emitter fades out and loses them.
//!
//! The emitters themselves (`IsoWorld.getFreeEmitter`, `BaseSoundEmitter`)
//! are the engine's, through [`FliesSoundHooks`].

use std::collections::HashMap;

use pz_core::debug::DebugType;
use pz_entity::runtime_load::{HostResult, RuntimeHostError};

use crate::defs::BuildingIdx;
use crate::square_mutation::{RenderSquareHooks, SquarePos};
use crate::SquareRef;

use super::corpse_count::coorddivision;

/// Initial value of native mutable `FliesSound.maxCorpseCount` (unused by
/// this selected update path); not a substitute for a live static read.
pub const INITIAL_MAX_CORPSE_COUNT: i32 = 25;

/// The sound `ChunkLevelData.update` loops.
pub const CORPSE_FLIES_SOUND: &str = "CorpseFlies";

/// An engine sound emitter (`BaseSoundEmitter`).
pub type EmitterId = u64;

/// The sound side and the character-side queries of `FliesSound`.
pub trait FliesSoundHooks {
    /// `BodyDamage.getSicknessFromCorpsesRate(corpseCount)`.
    fn sickness_from_corpses_rate(&self, corpse_count: i32) -> f32;
    /// `getZombieCount()` of the square at `(x, y, z)`, for
    /// `CorpseCount.getCorpseCount` with the sandbox's
    /// `ZombieHealthImpact` on.
    fn zombie_count(&self, _x: i32, _y: i32, _z: i32) -> i32 {
        0
    }
    /// `IsoWorld.instance.getFreeEmitter(x, y, z)`.
    fn get_free_emitter(&mut self, x: f32, y: f32, z: f32) -> EmitterId;
    /// `emitter.playSoundLoopedImpl(name)`.
    fn play_sound_looped_impl(&mut self, emitter: EmitterId, name: &str);
    /// `emitter.setVolumeAll(volume)`.
    fn set_volume_all(&mut self, emitter: EmitterId, volume: f32);
    /// `emitter.setPos(x, y, z)`.
    fn set_pos(&mut self, emitter: EmitterId, x: f32, y: f32, z: f32);
    /// `emitter.stopAll()`.
    fn stop_all(&mut self, emitter: EmitterId);
}

/// What `FliesSound.update` reads about a local player
/// (`IsoPlayer.players[i]`).
#[derive(Clone)]
pub struct FliesPlayer {
    /// `playerIndex`.
    pub index: usize,
    /// `getXi()`, `getYi()`, `getZi()`.
    pub xi: i32,
    pub yi: i32,
    pub zi: i32,
    /// `getCurrentSquare()` (`None` skips the player).
    pub square: Option<SquareRef>,
    /// `hasTrait(CharacterTrait.DEAF)`.
    pub deaf: bool,
}

/// The game state `FliesSound.update` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FliesEnv {
    /// `SandboxOptions.decayingCorpseHealthImpact` (1 turns the flies off).
    pub decaying_corpse_health_impact: i32,
    /// `SandboxOptions.zombieHealthImpact`.
    pub zombie_health_impact: bool,
    /// `ZomboidGlobals.foodSicknessDecrease`.
    pub food_sickness_decrease: f64,
    /// `GameTime.getThirtyFPSMultiplier()`.
    pub thirty_fps_multiplier: f32,
    /// `IsoPlayer.numPlayers`.
    pub num_players: usize,
}

/// `IsoGridSquare.setHasFlies(hasFlies)`: a change invalidates the
/// square's chunk level (`OBJECT_ADD` on, `OBJECT_REMOVE` off) and is
/// passed to the render snapshot ([`RenderSquareHooks::flies_changed`]).
pub fn set_has_flies(
    square: &SquareRef,
    render: &mut dyn RenderSquareHooks,
    flies: bool,
) -> HostResult<()> {
    let changed = {
        let sq = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("setHasFlies square read"))?;
        if sq.has_flies == flies {
            None
        } else {
            Some((sq.x, sq.y, sq.z))
        }
    };
    if let Some(pos) = changed {
        render.invalidate_render_chunk_level(
            pos.0.div_euclid(8),
            pos.1.div_euclid(8),
            pos.2,
            if flies { 64 } else { 128 },
        );
    }
    // Actual native setHasFlies invalidates before its putfield, including
    // any observation through the invalidation service. No guard crosses it.
    square
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("setHasFlies square write"))?
        .has_flies = flies;
    if changed.is_some() {
        render.flies_changed(square.clone(), flies);
    }
    Ok(())
}

// A displaced actual square can own VM-backed children. Its final owner
// retires at the existing outer host boundary, after enclosing guards end.
fn retire_square(square: Option<SquareRef>) {
    if let Some(square) = square {
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_owner(pz_entity::EntityOwner::from_ref(&square));
        pz_inventory::component_host::retire_components(retired);
    }
}

/// `FliesSound.FadeEmitter`.
struct FadeEmitter {
    emitter: Option<EmitterId>,
    volume: f32,
    target_volume: f32,
    sq: Option<SquareRef>,
    /// The chunk owning it was unloaded: freed once out of the fade list.
    orphan: bool,
}

impl FadeEmitter {
    fn new() -> Self {
        FadeEmitter {
            emitter: None,
            volume: 1.0,
            target_volume: 1.0,
            sq: None,
            orphan: false,
        }
    }

    /// `Reset()`.
    fn reset(&mut self) {
        self.emitter = None;
        self.volume = 1.0;
        self.target_volume = 1.0;
        retire_square(self.sq.take());
    }
}

impl Drop for FadeEmitter {
    fn drop(&mut self) {
        retire_square(self.sq.take());
    }
}

/// A chunk level's `ChunkLevelData`: `(wx, wy, z)` of the owning chunk's
/// `ChunkData`.
type LevelKey = (i32, i32, i32);

/// `FliesSound.PlayerData`.
#[derive(Clone, Debug, PartialEq)]
struct PlayerData {
    wx: i32,
    wy: i32,
    z: i32,
    building: Option<BuildingIdx>,
    force_update: bool,
    deaf: bool,
    refs: Vec<LevelKey>,
    refs_prev: Vec<LevelKey>,
}

impl Default for PlayerData {
    fn default() -> Self {
        PlayerData {
            wx: i32::MIN,
            wy: i32::MIN,
            z: i32::MIN,
            building: None,
            force_update: false,
            deaf: false,
            refs: Vec::new(),
            refs_prev: Vec::new(),
        }
    }
}

/// `FliesSound.instance`.
#[derive(Default)]
pub struct FliesSound {
    player_data: [PlayerData; 4],
    /// `fadeEmitters`, indices into `emitters` (duplicates possible, as
    /// in the original's list).
    fade_emitters: Vec<usize>,
    emitters: Vec<FadeEmitter>,
    free: Vec<usize>,
    /// `IsoChunk.corpseData` of the loaded chunks: per level (`z + 32`)
    /// the `ChunkLevelData.emitters` of each player.
    chunks: HashMap<(i32, i32), Vec<[Option<usize>; 4]>>,
}

/// Current-world services for the algorithm. Each query is callback-free and
/// ends its WORLD/square guards before returning. Sound/render services run
/// after those queries, and may reenter the actual retained square.
///
/// An engine adapter must use the current actual chunk/count/building fields,
/// return the same retained SquareRefs, and enumerate corpse squares in native
/// row order. It must not hold a parent WORLD guard throughout `update`.
/// Live engine activation and native building/chunk identity remain follow-up.
pub trait FliesWorld {
    fn building(&self, square: &SquareRef) -> HostResult<Option<BuildingIdx>>;
    fn has_chunk(&self, wx: i32, wy: i32) -> HostResult<bool>;
    fn corpse_count(
        &self,
        wx: i32,
        wy: i32,
        z: i32,
        building: Option<BuildingIdx>,
        zombie_health_impact: bool,
    ) -> HostResult<i32>;
    fn corpse_squares(
        &self,
        wx: i32,
        wy: i32,
        z: i32,
        building: Option<BuildingIdx>,
    ) -> HostResult<Vec<SquareRef>>;
    fn set_has_flies(&mut self, square: &SquareRef, flies: bool) -> HostResult<()>;
    fn hooks(&mut self) -> &mut dyn FliesSoundHooks;
}

impl FliesSound {
    pub fn new() -> Self {
        Self::default()
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        for p in &mut self.player_data {
            p.z = i32::MIN;
            p.wy = i32::MIN;
            p.wx = i32::MIN;
            p.building = None;
            p.force_update = false;
            p.refs.clear();
            p.refs_prev.clear();
        }
    }

    /// Whether a player's checked chunk levels currently hold the square
    /// `pos`'s level (the flies emitter of player `pn` there plays).
    pub fn has_emitter(&self, pn: usize, pos: SquarePos) -> bool {
        self.level_slot(pos.0.div_euclid(8), pos.1.div_euclid(8), pos.2, pn)
            .is_some_and(|i| self.emitters[i].emitter.is_some())
    }

    /// The emitters playing now, with their square, volume and target
    /// volume, in creation order of their slots.
    pub fn playing(&self) -> HostResult<Vec<(EmitterId, Option<SquarePos>, f32, f32)>> {
        let mut playing = Vec::new();
        for emitter in &self.emitters {
            let Some(id) = emitter.emitter else { continue };
            let pos = match emitter.sq.as_ref() {
                Some(square) => {
                    let sq = square
                        .try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("FliesSound playing square"))?;
                    Some((sq.x, sq.y, sq.z))
                }
                None => None,
            };
            playing.push((id, pos, emitter.volume, emitter.target_volume));
        }
        Ok(playing)
    }

    /// Trace every physical actual-square field when the engine owns this
    /// manager. Coordinate metadata from `playing` is never a replacement.
    pub fn visit_square_refs(&self, mut visit: impl FnMut(&SquareRef)) {
        for emitter in &self.emitters {
            if let Some(square) = emitter.sq.as_ref() {
                visit(square);
            }
        }
    }

    fn level_slot(&self, wx: i32, wy: i32, z: i32, pn: usize) -> Option<usize> {
        let l = self
            .chunks
            .get(&(wx, wy))?
            .get(usize::try_from(z + 32).ok()?)?;
        l[pn]
    }

    /// `chunkLoaded(chunk)`: its corpse data created (or reset), and every
    /// player rechecks.
    pub fn chunk_loaded(&mut self, wx: i32, wy: i32) {
        if let Some(levels) = self.chunks.get(&(wx, wy)) {
            for i in levels
                .iter()
                .flatten()
                .flatten()
                .copied()
                .collect::<Vec<_>>()
            {
                self.emitters[i].reset();
            }
        } else {
            self.chunks.insert((wx, wy), vec![[None; 4]; 64]);
        }
        for p in &mut self.player_data {
            p.force_update = true;
        }
    }

    /// `IsoChunk.removeFromWorld`'s `corpseData.removeFromWorld()`: its
    /// emitters stopped and reset; the chunk's data is gone with the chunk
    /// (`getChunkData` finds no chunk any more).
    pub fn chunk_removed_from_world(&mut self, wx: i32, wy: i32, hooks: &mut dyn FliesSoundHooks) {
        let Some(levels) = self.chunks.remove(&(wx, wy)) else {
            return;
        };
        for i in levels.iter().flatten().flatten().copied() {
            if let Some(e) = self.emitters[i].emitter {
                hooks.stop_all(e);
            }
            self.emitters[i].reset();
            if self.fade_emitters.contains(&i) {
                self.emitters[i].orphan = true;
            } else {
                self.free.push(i);
            }
        }
    }

    fn corpse_changed(&mut self, x: i32, y: i32, z: i32) {
        if !(-32..=31).contains(&z) {
            DebugType::General.error(None, &format!("invalid z-coordinate {x},{y},{z}"));
            return;
        }
        let key = (x.div_euclid(8), y.div_euclid(8));
        if !self.chunks.contains_key(&key) {
            return;
        }
        let level = (key.0, key.1, z);
        for p in &mut self.player_data {
            if p.refs.contains(&level) {
                p.force_update = true;
            }
        }
    }

    /// `corpseAdded(x, y, z)` (from `IsoDeadBody.addToWorld`).
    pub fn corpse_added(&mut self, x: i32, y: i32, z: i32) {
        self.corpse_changed(x, y, z);
    }

    /// `corpseRemoved(x, y, z)` (from `IsoDeadBody.removeFromWorld`).
    pub fn corpse_removed(&mut self, x: i32, y: i32, z: i32) {
        self.corpse_changed(x, y, z);
    }

    /// `update()`: each local player with a current square
    /// (`players[i]` for `i < numPlayers`), then the fading emitters. Off
    /// when the sandbox's `DecayingCorpseHealthImpact` is 1.
    pub fn update(
        &mut self,
        w: &mut dyn FliesWorld,
        players: &[Option<FliesPlayer>],
        env: &FliesEnv,
    ) -> HostResult<()> {
        if env.decaying_corpse_health_impact == 1 {
            return Ok(());
        }
        for i in 0..env.num_players.min(4) {
            let Some(p) = players.get(i).and_then(Option::as_ref) else {
                continue;
            };
            if p.square.is_none() {
                continue;
            }
            self.update_player(i, w, p, env)?;
        }
        let mut i = 0;
        while i < self.fade_emitters.len() {
            let e = self.fade_emitters[i];
            if self.update_fade(e, w.hooks(), env.thirty_fps_multiplier) {
                self.fade_emitters.remove(i);
                if self.emitters[e].orphan && !self.fade_emitters.contains(&e) {
                    self.emitters[e].orphan = false;
                    self.free.push(e);
                }
                continue;
            }
            i += 1;
        }
        Ok(())
    }

    /// `FadeEmitter.update()`: whether it leaves the fade list.
    fn update_fade(&mut self, i: usize, hooks: &mut dyn FliesSoundHooks, m: f32) -> bool {
        let e = &mut self.emitters[i];
        let Some(id) = e.emitter else {
            return true;
        };
        if e.volume < e.target_volume {
            e.volume += 0.01 * m;
            if e.volume >= e.target_volume {
                e.volume = e.target_volume;
                return true;
            }
        } else {
            e.volume += -0.01 * m;
            if e.volume <= 0.0 {
                e.volume = 0.0;
                hooks.stop_all(id);
                e.emitter = None;
                return true;
            }
        }
        hooks.set_volume_all(id, e.volume);
        false
    }

    /// `PlayerData.update(player)`.
    fn update_player(
        &mut self,
        pn: usize,
        w: &mut dyn FliesWorld,
        p: &FliesPlayer,
        env: &FliesEnv,
    ) -> HostResult<()> {
        {
            let d = &mut self.player_data[pn];
            if d.deaf != p.deaf {
                d.force_update = true;
                d.deaf = p.deaf;
            }
        }
        let square = p.square.as_ref().unwrap();
        // Native skips isSameLocation entirely after a force/deaf change.
        // Its building query precedes the player-coordinate comparisons.
        if !self.player_data[pn].force_update {
            let building = w.building(square)?;
            let d = &self.player_data[pn];
            let same = building == d.building
                && coorddivision(p.xi, 8) == d.wx
                && coorddivision(p.yi, 8) == d.wy
                && p.zi == d.z;
            if same {
                return Ok(());
            }
        }
        self.player_data[pn].force_update = false;
        let sq = {
            let sq = square
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("FliesSound player square"))?;
            (sq.x, sq.y, sq.z)
        };
        {
            let d = &mut self.player_data[pn];
            d.wx = coorddivision(sq.0, 8);
            d.wy = coorddivision(sq.1, 8);
            d.z = sq.2;
        }
        // This is a distinct native read after wx/wy/z are published,
        // including when isSameLocation already read the building above.
        let building = w.building(square)?;
        let (wx, wy, z) = {
            let d = &mut self.player_data[pn];
            d.building = building;
            d.refs.clear();
            (d.wx, d.wy, d.z)
        };
        for dy in -1..=1 {
            for dx in -1..=1 {
                // `getChunkData((wx + dx) * 8, (wy + dy) * 8)`.
                let key = ((wx + dx) * 8).div_euclid(8);
                let key = (key, ((wy + dy) * 8).div_euclid(8));
                if !self.chunks.contains_key(&key) || !w.has_chunk(key.0, key.1)? {
                    continue;
                }
                let level = (key.0, key.1, z);
                self.update_level(level, wx + dx, wy + dy, z, pn, building, w, p, env)?;
                // Native refs.add follows each successful level update.
                // A later failure preserves this completed field prefix.
                self.player_data[pn].refs.push(level);
            }
        }
        for i in 0..self.player_data[pn].refs_prev.len() {
            let previous = self.player_data[pn].refs_prev[i];
            if !self.player_data[pn].refs.contains(&previous) {
                self.deref(previous, pn, w)?;
            }
        }
        // Only after every deref succeeds does the native method clear
        // refsPrev and copy the completed current refs into it.
        let d = &mut self.player_data[pn];
        d.refs_prev.clear();
        d.refs_prev.extend_from_slice(&d.refs);
        Ok(())
    }

    fn slot(&mut self, level: LevelKey, pn: usize) -> &mut Option<usize> {
        &mut self.chunks.get_mut(&(level.0, level.1)).unwrap()[(level.2 + 32) as usize][pn]
    }

    fn alloc(&mut self) -> usize {
        if let Some(i) = self.free.pop() {
            self.emitters[i] = FadeEmitter::new();
            return i;
        }
        self.emitters.push(FadeEmitter::new());
        self.emitters.len() - 1
    }

    /// `ChunkLevelData.update(wx, wy, z, player)`.
    #[allow(clippy::too_many_arguments)]
    fn update_level(
        &mut self,
        level: LevelKey,
        wx: i32,
        wy: i32,
        z: i32,
        pn: usize,
        building: Option<BuildingIdx>,
        w: &mut dyn FliesWorld,
        p: &FliesPlayer,
        env: &FliesEnv,
    ) -> HostResult<()> {
        let mut corpse_count = w.corpse_count(wx, wy, z, building, env.zombie_health_impact)?;
        if p.deaf {
            corpse_count = 0;
        }
        if w.hooks().sickness_from_corpses_rate(corpse_count) as f64 > env.food_sickness_decrease {
            let Some(sound_sq) = self.calc_sound_pos(w, wx, wy, z, building)? else {
                return Ok(());
            };
            let sound_pos = {
                let sq = sound_sq
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("FliesSound selected square"))?;
                (sq.x, sq.y, sq.z)
            };
            if self.slot(level, pn).is_none() {
                let i = self.alloc();
                *self.slot(level, pn) = Some(i);
            }
            let fe = self.slot(level, pn).unwrap();
            match self.emitters[fe].emitter {
                None => {
                    let id = w.hooks().get_free_emitter(
                        sound_pos.0 as f32,
                        sound_pos.1 as f32,
                        z as f32,
                    );
                    w.hooks().play_sound_looped_impl(id, CORPSE_FLIES_SOUND);
                    w.hooks().set_volume_all(id, 0.0);
                    self.emitters[fe].emitter = Some(id);
                    self.emitters[fe].volume = 0.0;
                    self.fade_emitters.push(fe);
                }
                Some(id) => {
                    if let Some(old) = self.emitters[fe].sq.as_ref() {
                        w.set_has_flies(old, false)?;
                    }
                    w.hooks()
                        .set_pos(id, sound_pos.0 as f32, sound_pos.1 as f32, z as f32);
                    if self.emitters[fe].target_volume != 1.0 && !self.fade_emitters.contains(&fe) {
                        self.fade_emitters.push(fe);
                    }
                }
            }
            self.emitters[fe].target_volume = 1.0;
            retire_square(self.emitters[fe].sq.replace(sound_sq.clone()));
            w.set_has_flies(&sound_sq, true)?;
        } else {
            let Some(fe) = *self.slot(level, pn) else {
                return Ok(());
            };
            if self.emitters[fe].emitter.is_none() {
                return Ok(());
            }
            if !self.fade_emitters.contains(&fe) {
                self.fade_emitters.push(fe);
            }
            if p.deaf {
                self.emitters[fe].volume = 0.0;
            }
            self.emitters[fe].target_volume = 0.0;
            if let Some(sq) = self.emitters[fe].sq.as_ref() {
                w.set_has_flies(sq, false)?;
            }
        }
        Ok(())
    }

    /// `ChunkLevelData.calcSoundPos(wx, wy, z, building)`: the middle one
    /// (`count / 2`) of the chunk's squares on level `z`, row by row, that
    /// hold corpses and are in `building`.
    fn calc_sound_pos(
        &self,
        w: &dyn FliesWorld,
        wx: i32,
        wy: i32,
        z: i32,
        building: Option<BuildingIdx>,
    ) -> HostResult<Option<SquareRef>> {
        let found = w.corpse_squares(wx, wy, z, building)?;
        Ok(found.get(found.len() / 2).cloned())
    }

    /// `ChunkLevelData.deref(player)`.
    fn deref(&mut self, level: LevelKey, pn: usize, w: &mut dyn FliesWorld) -> HostResult<()> {
        let Some(fe) = self.level_slot(level.0, level.1, level.2, pn) else {
            return Ok(());
        };
        if self.emitters[fe].emitter.is_none() {
            return Ok(());
        }
        if !self.fade_emitters.contains(&fe) {
            self.fade_emitters.push(fe);
        }
        self.emitters[fe].target_volume = 0.0;
        if let Some(sq) = self.emitters[fe].sq.as_ref() {
            w.set_has_flies(sq, false)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod phase_tests;
