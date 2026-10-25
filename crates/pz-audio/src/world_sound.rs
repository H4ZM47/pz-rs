//! `zombie.WorldSoundManager`: the gameplay sounds zombies, animals and
//! players react to (not audible sounds; those go through the studio).
//!
//! The manager keeps no reference to the world. Callers pass what the
//! original reads from it:
//! * [`SoundWorld`]: the sandbox hearing option, whether a chunk is loaded
//!   (sounds register in each loaded chunk they reach, like
//!   `IsoChunk.soundList`), the room of a square, and the side effects of
//!   adding a sound (the `OnWorldSound` Lua event, fish noise, the
//!   population manager, network sends);
//! * [`Listener`]: a zombie's or animal's position, square, hearing and
//!   current sound target.
//!
//! Sounds live in a pool like the original's `ObjectPool`: released sounds
//! are reused first-in first-out, so a [`SoundId`] can come back as a new
//! sound after the old one expires, exactly as a Java reference would.

use std::collections::HashMap;

/// Identity of the object that made a sound (the original's `Object
/// source`), with the type tests `WorldSound.init` does on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SoundSource {
    /// Identity of the source object, compared by zombies' sound targets.
    pub id: u64,
    pub is_player: bool,
    pub is_zombie: bool,
    /// Generator, jukebox, TV, radio, stove, washer or dryer.
    pub is_player_base: bool,
    pub is_vehicle: bool,
    pub is_animal: bool,
}

/// `WorldSound` flags (`addSound(..., short flags)`).
pub mod flags {
    pub const STRESS_ANIMALS: i16 = 1;
    pub const STRESS_HUMANS: i16 = 2;
    pub const STRESS_ZOMBIES: i16 = 4;
}

/// `WorldSoundManager.WorldSound`.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldSound {
    pub source: Option<SoundSource>,
    pub life: i32,
    pub radius: i32,
    pub stress_humans: bool,
    pub stress_zombies: bool,
    pub stress_animals: bool,
    pub volume: i32,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub zombie_ignore_dist: f32,
    pub source_is_zombie: bool,
    pub source_is_player: bool,
    pub source_is_player_base: bool,
    pub stress_mod: f32,
    pub repeating: bool,
}

impl Default for WorldSound {
    fn default() -> Self {
        WorldSound {
            source: None,
            life: 1,
            radius: 0,
            stress_humans: false,
            stress_zombies: false,
            stress_animals: false,
            volume: 0,
            x: 0,
            y: 0,
            z: 0,
            zombie_ignore_dist: 0.0,
            source_is_zombie: false,
            source_is_player: false,
            source_is_player_base: false,
            stress_mod: 1.0,
            repeating: false,
        }
    }
}

impl WorldSound {
    /// `sourceIsVehicle()`.
    pub fn source_is_vehicle(&self) -> bool {
        self.source.is_some_and(|s| s.is_vehicle)
    }
}

/// Index of a sound in the manager's pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SoundId(pub usize);

/// Room of a square, as `IsoGridSquare.getRoom()` sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomAt {
    /// The square does not exist (not loaded).
    NoSquare,
    Outside,
    Room(u64),
}

/// What the manager needs from the world, and its side effects.
pub trait SoundWorld {
    /// `SandboxOptions.instance.lore.hearing` (1 pinpoint, 2 normal, 3 poor,
    /// 4 random, 5 random normal/poor).
    fn hearing_option(&self) -> i32 {
        2
    }
    /// `GameServer.server`.
    fn is_server(&self) -> bool {
        false
    }
    /// `GameClient.client`.
    fn is_client(&self) -> bool {
        false
    }
    /// `currentCell.getChunk(cx, cy) != null`.
    fn chunk_loaded(&self, _cx: i32, _cy: i32) -> bool {
        false
    }
    /// `currentCell.getGridSquare(x, y, z).getRoom()`.
    fn room_at(&self, _x: i32, _y: i32, _z: i32) -> RoomAt {
        RoomAt::NoSquare
    }
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
    /// `FishSchoolManager.addSoundNoise(x, y, radius / 6)` (not on clients).
    fn add_fish_noise(&mut self, _x: i32, _y: i32, _radius: i32) {}
    /// `ZombiePopulationManager.addWorldSound(sound, doSend)`.
    fn population_add(&mut self, _sound: &WorldSound, _do_send: bool) {}
    /// `GameClient.sendWorldSound` / `GameServer.sendWorldSound`.
    fn send(&mut self, _sound: &WorldSound) {}
}

/// A world with nothing loaded and no side effects.
pub struct NoWorld;

impl SoundWorld for NoWorld {}

/// The zombie or animal asking about sounds.
#[derive(Debug, Clone, Copy)]
pub struct Listener {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub square: ListenerSquare,
    /// `IsoZombie.hearing`.
    pub hearing: i32,
    /// `getWornItemsHearingMultiplier() * getWeatherHearingMultiplier()`.
    pub hearing_scale: f32,
    /// `IsoZombie.soundSourceTarget`.
    pub sound_source_target: Option<u64>,
    /// `IsoAnimal.isWild()`.
    pub wild: bool,
}

/// The listener's current square.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenerSquare {
    /// `getCurrentSquare() == null`.
    None,
    /// A square whose `chunk` is null.
    NoChunk,
    /// A square in chunk (cx, cy).
    Chunk(i32, i32),
}

/// `WorldSoundManager.ResultBiggestSound`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BiggestSound {
    pub sound: Option<SoundId>,
    pub attract: f32,
}

const MUFFLE_DIFFERENT_ROOMS: f32 = 1.2;
const MUFFLE_INSIDE_OUTSIDE: f32 = 1.4;
/// `ObjectPool` default maximum size.
const POOL_MAX: usize = 1024;

/// `IsoUtils.DistanceToSquared` (float inputs, computed in double).
fn dist_sq(fx: f32, fy: f32, fz: f32, tx: f32, ty: f32, tz: f32) -> f32 {
    let d = |a: f32, b: f32| ((b - a) as f64).powi(2);
    (d(fx, tx) + d(fy, ty) + d(fz, tz)) as f32
}

/// `PZMath.ceil`.
fn pz_ceil(v: f32) -> f32 {
    if v >= 0.0 {
        (v + 0.999_999_9) as i32 as f32
    } else {
        (v - 1.0e-7) as i32 as f32
    }
}

#[derive(Default)]
pub struct WorldSoundManager {
    pool: Vec<WorldSound>,
    /// Released sounds, oldest first (the pool's free list).
    free: std::collections::VecDeque<usize>,
    /// `soundList`.
    pub sound_list: Vec<SoundId>,
    /// `IsoChunk.soundList` of each loaded chunk the manager has touched.
    chunk_lists: HashMap<(i32, i32), Vec<SoundId>>,
}

impl WorldSoundManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sound(&self, id: SoundId) -> &WorldSound {
        &self.pool[id.0]
    }

    pub fn sound_mut(&mut self, id: SoundId) -> &mut WorldSound {
        &mut self.pool[id.0]
    }

    /// `IsoChunk.soundList` of a chunk.
    pub fn chunk_sounds(&self, cx: i32, cy: i32) -> &[SoundId] {
        self.chunk_lists.get(&(cx, cy)).map_or(&[], Vec::as_slice)
    }

    /// `IsoChunk` reset (the chunk was unloaded or reused).
    pub fn clear_chunk(&mut self, cx: i32, cy: i32) {
        self.chunk_lists.remove(&(cx, cy));
    }

    /// `getNew()`.
    fn alloc(&mut self) -> usize {
        match self.free.pop_front() {
            Some(i) => i,
            None => {
                self.pool.push(WorldSound::default());
                self.pool.len() - 1
            }
        }
    }

    /// `release(sound)`.
    fn release(&mut self, i: usize) {
        if self.free.len() < POOL_MAX {
            self.free.push_back(i);
        }
    }

    /// `KillCell()`.
    pub fn kill_cell(&mut self) {
        for id in std::mem::take(&mut self.sound_list) {
            self.pool[id.0].source = None;
            self.release(id.0);
        }
    }

    /// `getHearingMultiplier(int)`.
    pub fn hearing_multiplier(hearing: i32) -> f32 {
        match hearing {
            1 => 3.0,
            3 => 0.45,
            _ => 1.0,
        }
    }

    /// `getHearingMultiplier(IsoZombie)`; `None` is a null zombie.
    pub fn zombie_hearing_multiplier(zombie: Option<&Listener>) -> f32 {
        match zombie {
            None => Self::hearing_multiplier(2),
            Some(z) => Self::hearing_multiplier(z.hearing) * z.hearing_scale,
        }
    }

    /// `WorldSound.init(source, x, y, z, radius, volume, zombieIgnoreDist,
    /// stressMod, flags)`, including its Lua event and fish noise.
    #[allow(clippy::too_many_arguments)]
    fn init_sound(
        &mut self,
        world: &mut dyn SoundWorld,
        i: usize,
        source: Option<SoundSource>,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
        volume: i32,
        zombie_ignore_dist: f32,
        stress_mod: f32,
        f: i16,
    ) {
        self.pool[i] = WorldSound {
            source,
            life: 16,
            radius,
            stress_humans: f & flags::STRESS_HUMANS != 0,
            stress_zombies: f & flags::STRESS_ZOMBIES != 0,
            stress_animals: f & flags::STRESS_ANIMALS != 0,
            volume,
            x,
            y,
            z,
            zombie_ignore_dist,
            source_is_zombie: source.is_some_and(|s| s.is_zombie),
            source_is_player: source.is_some_and(|s| s.is_player),
            source_is_player_base: source.is_some_and(|s| s.is_player_base),
            stress_mod,
            repeating: false,
        };
        world.on_world_sound(x, y, z, radius, volume, source);
        if !world.is_client() {
            world.add_fish_noise(x, y, radius / 6);
        }
    }

    /// `addSound(source, x, y, z, radius, volume, zombieIgnoreDist, stressMod,
    /// sourceIsZombie, doSend, remote, repeating, flags)`: the overload all
    /// others call. Returns `None` (Java `null`) for a non-positive radius.
    #[allow(clippy::too_many_arguments)]
    pub fn add_sound_flags(
        &mut self,
        world: &mut dyn SoundWorld,
        source: Option<SoundSource>,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
        volume: i32,
        zombie_ignore_dist: f32,
        stress_mod: f32,
        source_is_zombie: bool,
        do_send: bool,
        _remote: bool,
        repeating: bool,
        f: i16,
    ) -> Option<SoundId> {
        if radius <= 0 {
            return None;
        }
        let i = self.alloc();
        self.init_sound(
            world,
            i,
            source,
            x,
            y,
            z,
            radius,
            volume,
            zombie_ignore_dist,
            stress_mod,
            f,
        );
        let id = SoundId(i);
        {
            let s = &mut self.pool[i];
            s.repeating = repeating;
            if source.is_none() {
                s.source_is_zombie = source_is_zombie;
            }
        }
        if !world.is_server() {
            let mut hearing = world.hearing_option();
            if hearing == 4 {
                hearing = 1;
            }
            if hearing == 5 {
                hearing = 2;
            }
            let s = &self.pool[i];
            let animal = if s.stress_humans || s.stress_animals {
                3.0
            } else {
                1.0
            };
            let zombie = Self::hearing_multiplier(hearing);
            let mult = if animal > zombie { animal } else { zombie };
            let radius_max = pz_ceil(radius as f32 * mult) as i32;
            let cmin_x = (x - radius_max) / 8;
            let cmin_y = (y - radius_max) / 8;
            let cmax_x = ((x as f32 + radius_max as f32) / 8.0).ceil() as i32;
            let cmax_y = ((y as f32 + radius_max as f32) / 8.0).ceil() as i32;
            for cx in cmin_x..cmax_x {
                for cy in cmin_y..cmax_y {
                    if world.chunk_loaded(cx, cy) {
                        self.chunk_lists.entry((cx, cy)).or_default().push(id);
                    }
                }
            }
        }
        self.sound_list.push(id);
        world.population_add(&self.pool[i], do_send);
        if do_send && (world.is_client() || world.is_server()) {
            world.send(&self.pool[i]);
        }
        Some(id)
    }

    /// `addSound(source, x, y, z, radius, volume, stressHumans,
    /// zombieIgnoreDist, stressMod, sourceIsZombie, doSend, remote,
    /// repeating, stressAnimals)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_sound_full(
        &mut self,
        world: &mut dyn SoundWorld,
        source: Option<SoundSource>,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
        volume: i32,
        stress_humans: bool,
        zombie_ignore_dist: f32,
        stress_mod: f32,
        source_is_zombie: bool,
        do_send: bool,
        remote: bool,
        repeating: bool,
        stress_animals: bool,
    ) -> Option<SoundId> {
        let mut f = flags::STRESS_ZOMBIES;
        if stress_animals {
            f |= flags::STRESS_ANIMALS;
        }
        if stress_humans {
            f |= flags::STRESS_HUMANS;
        }
        self.add_sound_flags(
            world,
            source,
            x,
            y,
            z,
            radius,
            volume,
            zombie_ignore_dist,
            stress_mod,
            source_is_zombie,
            do_send,
            remote,
            repeating,
            f,
        )
    }

    /// `addSound(source, x, y, z, radius, volume)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_sound(
        &mut self,
        world: &mut dyn SoundWorld,
        source: Option<SoundSource>,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
        volume: i32,
    ) -> Option<SoundId> {
        self.add_sound_stress(world, source, x, y, z, radius, volume, false, 0.0, 1.0)
    }

    /// `addSound(source, x, y, z, radius, volume, stressHumans,
    /// zombieIgnoreDist, stressMod)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_sound_stress(
        &mut self,
        world: &mut dyn SoundWorld,
        source: Option<SoundSource>,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
        volume: i32,
        stress_humans: bool,
        zombie_ignore_dist: f32,
        stress_mod: f32,
    ) -> Option<SoundId> {
        self.add_sound_full(
            world,
            source,
            x,
            y,
            z,
            radius,
            volume,
            stress_humans,
            zombie_ignore_dist,
            stress_mod,
            false,
            true,
            false,
            false,
            false,
        )
    }

    /// `addSoundRepeating(source, x, y, z, radius, volume, stressHumans,
    /// zombieIgnoreDist, stressMod)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_sound_repeating(
        &mut self,
        world: &mut dyn SoundWorld,
        source: Option<SoundSource>,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
        volume: i32,
        stress_humans: bool,
        zombie_ignore_dist: f32,
        stress_mod: f32,
    ) -> Option<SoundId> {
        self.add_sound_full(
            world,
            source,
            x,
            y,
            z,
            radius,
            volume,
            stress_humans,
            zombie_ignore_dist,
            stress_mod,
            false,
            true,
            false,
            true,
            false,
        )
    }

    /// The list a listener searches: its chunk's, or the global one on a
    /// server or when the square has no chunk.
    fn list_for(&self, world: &dyn SoundWorld, l: Option<&Listener>) -> Vec<SoundId> {
        match l.map(|l| l.square) {
            Some(ListenerSquare::Chunk(cx, cy)) if !world.is_server() => {
                self.chunk_sounds(cx, cy).to_vec()
            }
            _ => self.sound_list.clone(),
        }
    }

    /// `getSoundZomb(zombie)`.
    pub fn get_sound_zomb(&self, world: &dyn SoundWorld, zombie: &Listener) -> Option<SoundId> {
        let target = zombie.sound_source_target?;
        if zombie.square == ListenerSquare::None {
            return None;
        }
        self.list_for(world, Some(zombie)).into_iter().find(|id| {
            let s = &self.pool[id.0];
            s.source.map(|src| src.id) == Some(target) && s.stress_zombies
        })
    }

    /// `getSoundAnimal(animal)`.
    pub fn get_sound_animal(&self, world: &dyn SoundWorld, animal: &Listener) -> Option<SoundId> {
        if animal.square == ListenerSquare::None {
            return None;
        }
        let mut loudest = None;
        let mut loudest_volume = 0.0f32;
        for id in self.list_for(world, Some(animal)) {
            let s = &self.pool[id.0];
            if !s.stress_humans && !s.stress_animals {
                continue;
            }
            let d = dist_sq(
                animal.x,
                animal.y,
                animal.z * 3.0,
                s.x as f32,
                s.y as f32,
                s.z as f32 * 3.0,
            );
            let radius = s.radius as f32 * if animal.wild { 3.0 } else { 1.0 };
            if d > radius * radius {
                continue;
            }
            let volume = s.volume as f32 * (1.0 - d / (radius * radius));
            if volume > loudest_volume {
                loudest_volume = volume;
                loudest = Some(id);
            }
        }
        loudest
    }

    /// The room muffling shared by the attraction functions.
    fn muffle(
        world: &dyn SoundWorld,
        s: &WorldSound,
        x: i32,
        y: i32,
        z: i32,
        mut delta: f32,
    ) -> f32 {
        let a = world.room_at(s.x, s.y, s.z);
        let b = world.room_at(x, y, z);
        if a != RoomAt::NoSquare && b != RoomAt::NoSquare && a != b {
            delta *= MUFFLE_DIFFERENT_ROOMS;
            if a == RoomAt::Outside || b == RoomAt::Outside {
                delta *= MUFFLE_INSIDE_OUTSIDE;
            }
        }
        delta
    }

    /// `getBiggestSoundZomb(x, y, z, ignoreBySameType, zombie)`.
    pub fn get_biggest_sound_zomb(
        &self,
        world: &dyn SoundWorld,
        x: i32,
        y: i32,
        z: i32,
        ignore_by_same_type: bool,
        zombie: Option<&Listener>,
    ) -> BiggestSound {
        if zombie.is_some_and(|z| z.square == ListenerSquare::None) {
            return BiggestSound {
                sound: None,
                attract: 0.0,
            };
        }
        let mut largest_sound = -1_000_000.0f32;
        let mut largest = None;
        let mult = Self::zombie_hearing_multiplier(zombie);
        for id in self.list_for(world, zombie) {
            let s = &self.pool[id.0];
            if !s.stress_zombies || s.radius == 0 {
                continue;
            }
            let d = dist_sq(
                x as f32,
                y as f32,
                (z * 3) as f32,
                s.x as f32,
                s.y as f32,
                (s.z * 3) as f32,
            );
            let radius = s.radius as f32 * mult;
            if d > radius * radius {
                continue;
            }
            if d < s.zombie_ignore_dist * s.zombie_ignore_dist && z == s.z {
                continue;
            }
            if ignore_by_same_type && s.source_is_zombie {
                continue;
            }
            let delta = Self::muffle(world, s, x, y, z, d / (radius * radius));
            let delta = 1.0 - delta;
            if delta <= 0.0 {
                continue;
            }
            let delta = delta.min(1.0);
            let tot = s.volume as f32 * delta;
            if tot > largest_sound {
                largest_sound = tot;
                largest = Some(id);
            }
        }
        BiggestSound {
            sound: largest,
            attract: largest_sound,
        }
    }

    /// `getSoundAttract(sound, zombie)`.
    pub fn get_sound_attract(
        &self,
        world: &dyn SoundWorld,
        sound: Option<SoundId>,
        zombie: &Listener,
    ) -> f32 {
        let Some(id) = sound else { return 0.0 };
        let s = &self.pool[id.0];
        if s.radius == 0 || s.source_is_zombie {
            return 0.0;
        }
        let d = dist_sq(
            zombie.x,
            zombie.y,
            zombie.z * 3.0,
            s.x as f32,
            s.y as f32,
            (s.z * 3) as f32,
        );
        let radius = s.radius as f32 * Self::zombie_hearing_multiplier(Some(zombie));
        self.attract(world, s, d, radius, zombie)
    }

    /// `getSoundAttractAnimal(sound, animal)`.
    pub fn get_sound_attract_animal(
        &self,
        world: &dyn SoundWorld,
        sound: Option<SoundId>,
        animal: &Listener,
    ) -> f32 {
        let Some(id) = sound else { return 0.0 };
        let s = &self.pool[id.0];
        if s.radius == 0 {
            return 0.0;
        }
        let d = dist_sq(
            animal.x,
            animal.y,
            animal.z * 3.0,
            s.x as f32,
            s.y as f32,
            (s.z * 3) as f32,
        );
        let radius = s.radius as f32 * if animal.wild { 3.0 } else { 1.0 };
        self.attract(world, s, d, radius, animal)
    }

    fn attract(
        &self,
        world: &dyn SoundWorld,
        s: &WorldSound,
        d: f32,
        radius: f32,
        l: &Listener,
    ) -> f32 {
        if d > radius * radius {
            return 0.0;
        }
        if d < s.zombie_ignore_dist * s.zombie_ignore_dist && l.z == s.z as f32 {
            return 0.0;
        }
        // getGridSquare(double, double, double) floors the coordinates.
        let (fx, fy, fz) = (l.x.floor() as i32, l.y.floor() as i32, l.z.floor() as i32);
        let delta = Self::muffle(world, s, fx, fy, fz, d / (radius * radius));
        s.volume as f32 * (1.0 - delta).clamp(0.0, 1.0)
    }

    /// `getStressFromSounds(x, y, z)`.
    pub fn get_stress_from_sounds(&self, x: i32, y: i32, _z: i32) -> f32 {
        let mut ret = 0.0f32;
        for id in &self.sound_list {
            let s = &self.pool[id.0];
            if !s.stress_humans || s.radius == 0 {
                continue;
            }
            let dist = (s.x as f32 - x as f32).abs() + (s.y as f32 - y as f32).abs();
            let delta = 1.0 - dist / s.radius as f32;
            if delta <= 0.0 {
                continue;
            }
            ret += delta.min(1.0) * s.stress_mod;
        }
        ret
    }

    /// `update()`: `active_chunks` are the chunks in the players' chunk maps
    /// (each runs `IsoChunk.updateSounds`), ignored on a server.
    pub fn update(
        &mut self,
        world: &dyn SoundWorld,
        active_chunks: impl IntoIterator<Item = (i32, i32)>,
    ) {
        if !world.is_server() {
            for c in active_chunks {
                if let Some(list) = self.chunk_lists.get_mut(&c) {
                    let pool = &self.pool;
                    list.retain(|id| pool[id.0].life > 0);
                }
            }
        }
        let mut n = 0;
        while n < self.sound_list.len() {
            let id = self.sound_list[n];
            if self.pool[id.0].life <= 0 {
                self.sound_list.remove(n);
                self.release(id.0);
                continue;
            }
            self.pool[id.0].life -= 1;
            n += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world where chunks (0..4, 0..4) are loaded and squares x < 10 are
    /// room 1, x >= 20 outside.
    #[derive(Default)]
    struct TestWorld {
        events: Vec<(i32, i32, i32, i32, i32)>,
        fish: Vec<(i32, i32, i32)>,
        added: usize,
        hearing: i32,
        server: bool,
    }

    impl SoundWorld for TestWorld {
        fn hearing_option(&self) -> i32 {
            if self.hearing == 0 {
                2
            } else {
                self.hearing
            }
        }
        fn is_server(&self) -> bool {
            self.server
        }
        fn chunk_loaded(&self, cx: i32, cy: i32) -> bool {
            (0..4).contains(&cx) && (0..4).contains(&cy)
        }
        fn room_at(&self, x: i32, _y: i32, _z: i32) -> RoomAt {
            match x {
                ..=9 => RoomAt::Room(1),
                10..=19 => RoomAt::Room(2),
                _ => RoomAt::Outside,
            }
        }
        fn on_world_sound(
            &mut self,
            x: i32,
            y: i32,
            z: i32,
            r: i32,
            v: i32,
            _s: Option<SoundSource>,
        ) {
            self.events.push((x, y, z, r, v));
        }
        fn add_fish_noise(&mut self, x: i32, y: i32, r: i32) {
            self.fish.push((x, y, r));
        }
        fn population_add(&mut self, _s: &WorldSound, _d: bool) {
            self.added += 1;
        }
    }

    fn zombie(x: f32, y: f32) -> Listener {
        Listener {
            x,
            y,
            z: 0.0,
            square: ListenerSquare::Chunk((x as i32) / 8, (y as i32) / 8),
            hearing: 2,
            hearing_scale: 1.0,
            sound_source_target: None,
            wild: false,
        }
    }

    #[test]
    fn add_sound_registers_in_reached_loaded_chunks() {
        let mut w = TestWorld::default();
        let mut m = WorldSoundManager::new();
        assert_eq!(m.add_sound(&mut w, None, 5, 5, 0, 0, 10), None);
        let id = m.add_sound(&mut w, None, 12, 12, 0, 6, 50).unwrap();
        // radius 6 with normal hearing: chunks (12-6)/8=0 .. ceil(18/8)=3 (exclusive).
        for cx in 0..4 {
            for cy in 0..4 {
                assert_eq!(
                    m.chunk_sounds(cx, cy).contains(&id),
                    cx < 3 && cy < 3,
                    "{cx},{cy}"
                );
            }
        }
        assert_eq!(w.events, vec![(12, 12, 0, 6, 50)]);
        assert_eq!(w.fish, vec![(12, 12, 1)]);
        assert_eq!(w.added, 1);
        let s = m.sound(id);
        assert_eq!(
            (s.life, s.stress_zombies, s.stress_humans),
            (16, true, false)
        );
    }

    #[test]
    fn stress_sounds_reach_three_times_further_into_chunks() {
        let mut w = TestWorld::default();
        let mut m = WorldSoundManager::new();
        let id = m
            .add_sound_stress(&mut w, None, 12, 12, 0, 4, 50, true, 0.0, 1.0)
            .unwrap();
        // radiusMax = PZMath.ceil(12.0), which is 13 because 12 + 0.9999999f
        // rounds to 13.0f: chunks 0 .. ceil(25/8) = 4.
        assert!(m.chunk_sounds(3, 3).contains(&id));
        let id2 = m
            .add_sound_stress(&mut w, None, 20, 20, 0, 1, 50, false, 0.0, 1.0)
            .unwrap();
        // Not stressful: radiusMax = PZMath.ceil(1.0) = 2: chunks 2 .. ceil(22/8) = 3.
        assert!(m.chunk_sounds(2, 2).contains(&id2));
        assert!(!m.chunk_sounds(3, 3).contains(&id2));
    }

    #[test]
    fn biggest_sound_uses_falloff_and_room_muffling() {
        let mut w = TestWorld::default();
        let mut m = WorldSoundManager::new();
        let a = m.add_sound(&mut w, None, 5, 5, 0, 20, 100).unwrap();
        let b = m.add_sound(&mut w, None, 15, 5, 0, 20, 100).unwrap();
        let z = zombie(6.0, 5.0);
        let r = m.get_biggest_sound_zomb(&w, 6, 5, 0, false, Some(&z));
        assert_eq!(r.sound, Some(a));
        // Same room: 1 - 1/400.
        assert!((r.attract - 100.0 * (1.0 - 1.0 / 400.0)).abs() < 1e-4);
        // Sound b is in another room: delta (81/400) * 1.2.
        let attract_b = m.get_sound_attract(&w, Some(b), &z);
        assert!(
            (attract_b - 100.0 * (1.0 - 81.0 / 400.0 * 1.2)).abs() < 1e-3,
            "{attract_b}"
        );
        // From outside, a sound in a room is muffled twice.
        let attract_out = m.get_sound_attract(&w, Some(b), &zombie(21.0, 5.0));
        assert!(
            (attract_out - 100.0 * (1.0 - 36.0 / 400.0 * 1.2 * 1.4)).abs() < 1e-3,
            "{attract_out}"
        );
        // Too muffled to hear at all.
        assert_eq!(m.get_sound_attract(&w, Some(a), &zombie(21.0, 5.0)), 0.0);
    }

    #[test]
    fn ignore_distance_and_same_type() {
        let mut w = TestWorld::default();
        let mut m = WorldSoundManager::new();
        let zsrc = SoundSource {
            id: 7,
            is_zombie: true,
            ..Default::default()
        };
        m.add_sound_stress(&mut w, Some(zsrc), 5, 5, 0, 20, 100, false, 3.0, 1.0);
        let z = zombie(6.0, 5.0);
        // Within the ignore distance on the same level.
        assert_eq!(
            m.get_biggest_sound_zomb(&w, 6, 5, 0, false, Some(&z)).sound,
            None
        );
        let far = zombie(10.0, 5.0);
        assert!(m
            .get_biggest_sound_zomb(&w, 10, 5, 0, false, Some(&far))
            .sound
            .is_some());
        assert_eq!(
            m.get_biggest_sound_zomb(&w, 10, 5, 0, true, Some(&far))
                .sound,
            None
        );
        // Zombie sources never attract.
        let id = m.sound_list[0];
        assert_eq!(m.get_sound_attract(&w, Some(id), &far), 0.0);
    }

    #[test]
    fn update_expires_after_sixteen_ticks_and_reuses_pool() {
        let mut w = TestWorld::default();
        let mut m = WorldSoundManager::new();
        let id = m.add_sound(&mut w, None, 5, 5, 0, 5, 10).unwrap();
        for _ in 0..16 {
            m.update(&w, [(0, 0)]);
            assert_eq!(m.sound_list, vec![id]);
        }
        // Chunk lists are cleaned first in the update that drops the sound.
        m.update(&w, [(1, 1)]);
        assert!(m.sound_list.is_empty());
        assert!(m.chunk_sounds(0, 0).contains(&id));
        assert!(m.chunk_sounds(1, 1).is_empty());
        // Released sounds are reused.
        assert_eq!(m.add_sound(&mut w, None, 5, 5, 0, 5, 10), Some(id));
    }

    #[test]
    fn stress_from_sounds_is_manhattan_falloff() {
        let mut w = TestWorld::default();
        let mut m = WorldSoundManager::new();
        m.add_sound_stress(&mut w, None, 0, 0, 0, 10, 50, true, 0.0, 2.0);
        m.add_sound(&mut w, None, 0, 0, 0, 10, 50);
        assert!((m.get_stress_from_sounds(3, 4, 0) - (1.0 - 7.0 / 10.0) * 2.0).abs() < 1e-6);
        assert_eq!(m.get_stress_from_sounds(10, 4, 0), 0.0);
    }

    #[test]
    fn sound_target_lookup_and_server_list() {
        let mut w = TestWorld {
            server: true,
            ..Default::default()
        };
        let mut m = WorldSoundManager::new();
        let src = SoundSource {
            id: 42,
            is_player: true,
            ..Default::default()
        };
        let id = m.add_sound(&mut w, Some(src), 5, 5, 0, 5, 10).unwrap();
        // On a server sounds are not put in chunk lists; lookups use the global list.
        assert!(m.chunk_sounds(0, 0).is_empty());
        let mut z = zombie(5.0, 5.0);
        assert_eq!(m.get_sound_zomb(&w, &z), None);
        z.sound_source_target = Some(42);
        assert_eq!(m.get_sound_zomb(&w, &z), Some(id));
        assert!(m.sound(id).source_is_player);
    }

    #[test]
    fn hearing_options() {
        assert_eq!(WorldSoundManager::hearing_multiplier(1), 3.0);
        assert_eq!(WorldSoundManager::hearing_multiplier(3), 0.45);
        assert_eq!(WorldSoundManager::hearing_multiplier(2), 1.0);
        let mut w = TestWorld {
            hearing: 4,
            ..Default::default()
        };
        let mut m = WorldSoundManager::new();
        // Option 4 (random) counts as pinpoint when registering in chunks.
        let id = m.add_sound(&mut w, None, 12, 12, 0, 4, 10).unwrap();
        assert!(m.chunk_sounds(0, 0).contains(&id));
        // PZMath.ceil adds 0.9999999f, which rounds whole numbers up.
        assert_eq!(pz_ceil(2.0), 3.0);
        assert_eq!(pz_ceil(2.1), 3.0);
        assert_eq!(pz_ceil(0.5), 1.0);
    }
}
