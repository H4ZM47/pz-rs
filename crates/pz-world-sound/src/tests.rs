use super::*;
use pz_world::IsoGridSquare;

fn square(x: i32, y: i32, z: i32, room_id: i64) -> IsoGridSquare {
    let mut sq = IsoGridSquare::new(x, y, z);
    sq.room_id = room_id;
    sq
}

/// Chunks (0,0), (1,0) and (-1,-1); squares (2,3,0) outside, (4,4,0) in
/// room 42, (-1,-1,0) in room 7.
fn cell() -> IsoCell {
    let mut cell = IsoCell::default();
    let mut c = IsoChunk::new(0, 0);
    c.set_square(2, 3, 0, Some((square(2, 3, 0, -1)).into()));
    c.set_square(4, 4, 0, Some((square(4, 4, 0, 42)).into()));
    cell.chunks.insert((0, 0), c);
    cell.chunks.insert((1, 0), IsoChunk::new(1, 0));
    let mut c = IsoChunk::new(-1, -1);
    c.set_square(7, 7, 0, Some((square(-1, -1, 0, 7)).into()));
    cell.chunks.insert((-1, -1), c);
    cell
}

#[derive(Default)]
struct Log {
    events: Vec<(i32, i32, i32, i32, i32, Option<SoundSource>)>,
    fish: Vec<(i32, i32, i32)>,
    population: Vec<(i32, i32, bool)>,
    sent: usize,
    unloaded: Vec<(i32, i32)>,
}

impl SoundEffects for Log {
    fn on_world_sound(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
        volume: i32,
        source: Option<SoundSource>,
    ) {
        self.events.push((x, y, z, radius, volume, source));
    }
    fn add_fish_noise(&mut self, x: i32, y: i32, radius: i32) {
        self.fish.push((x, y, radius));
    }
    fn population_add(&mut self, sound: &WorldSound, do_send: bool) {
        self.population.push((sound.x, sound.y, do_send));
    }
    fn send(&mut self, _sound: &WorldSound) {
        self.sent += 1;
    }
}

impl ChunkLoadHook for Log {
    fn load_grid_square(
        &mut self,
        _c: &mut IsoChunk,
        _x: i32,
        _y: i32,
        _z: i32,
        _ctx: &mut HookCtx<'_>,
    ) {
    }
    fn chunk_loaded(&mut self, _c: &mut IsoChunk, _ctx: &mut HookCtx<'_>) {}
    fn chunk_unloaded(&mut self, wx: i32, wy: i32) {
        self.unloaded.push((wx, wy));
    }
}

#[test]
fn chunk_loaded_and_room_at() {
    let cell = cell();
    let mut fx = NoEffects;
    let w = WorldSoundAdapter::new(&cell, 3, &mut fx);
    assert_eq!(w.hearing_option(), 3);
    assert!(!w.is_server() && !w.is_client());
    assert!(w.chunk_loaded(0, 0));
    assert!(w.chunk_loaded(1, 0));
    assert!(w.chunk_loaded(-1, -1));
    assert!(!w.chunk_loaded(0, 1));
    assert_eq!(w.room_at(2, 3, 0), RoomAt::Outside);
    assert_eq!(w.room_at(4, 4, 0), RoomAt::Room(42));
    assert_eq!(w.room_at(-1, -1, 0), RoomAt::Room(7));
    // Missing square in a loaded chunk, other level, unloaded chunk.
    assert_eq!(w.room_at(5, 5, 0), RoomAt::NoSquare);
    assert_eq!(w.room_at(4, 4, 1), RoomAt::NoSquare);
    assert_eq!(w.room_at(4, 12, 0), RoomAt::NoSquare);
}

#[test]
fn flags_are_inputs() {
    let cell = cell();
    let mut fx = NoEffects;
    let mut w = WorldSoundAdapter::new(&cell, 2, &mut fx);
    w.server = true;
    assert!(w.is_server() && !w.is_client());
    w.server = false;
    w.client = true;
    assert!(w.is_client());
}

#[test]
fn add_sound_uses_cell_and_forwards_effects() {
    let cell = cell();
    let mut log = Log::default();
    let mut sounds = WorldSoundManager::new();
    let src = SoundSource {
        id: 9,
        is_player: true,
        ..Default::default()
    };
    let id = {
        let mut w = WorldSoundAdapter::new(&cell, 2, &mut log);
        sounds
            .add_sound(&mut w, Some(src), 4, 4, 0, 10, 20)
            .unwrap()
    };
    assert_eq!(log.events, vec![(4, 4, 0, 10, 20, Some(src))]);
    assert_eq!(log.fish, vec![(4, 4, 1)]);
    assert_eq!(log.population, vec![(4, 4, true)]);
    // Offline: nothing is sent.
    assert_eq!(log.sent, 0);
    // Chunks 0..2 x 0..2 (integer division truncates toward zero, so
    // (-1,-1) is not reached); only loaded ones get the sound.
    assert_eq!(sounds.chunk_sounds(0, 0), &[id]);
    assert_eq!(sounds.chunk_sounds(1, 0), &[id]);
    assert!(sounds.chunk_sounds(0, 1).is_empty());
    assert!(sounds.chunk_sounds(-1, -1).is_empty());

    // A client sends but adds no fish noise.
    let mut log = Log::default();
    let mut w = WorldSoundAdapter::new(&cell, 2, &mut log);
    w.client = true;
    sounds.add_sound(&mut w, None, 4, 4, 0, 10, 20).unwrap();
    assert!(log.fish.is_empty());
    assert_eq!(log.sent, 1);
}

#[test]
fn unload_clears_chunk_sounds_and_forwards() {
    let mut cell = cell();
    let mut sounds = WorldSoundManager::new();
    let id = {
        let mut fx = NoEffects;
        let mut w = WorldSoundAdapter::new(&cell, 2, &mut fx);
        sounds.add_sound(&mut w, None, 4, 4, 0, 10, 20).unwrap()
    };
    let mut inner = Log::default();
    {
        let mut hook = ClearSoundsOnUnload::new(&mut sounds, Some(&mut inner));
        assert!(cell.unload_chunk(1, 0, Some(&mut hook)).is_some());
        // Not loaded: no hook call.
        assert!(cell.unload_chunk(5, 5, Some(&mut hook)).is_none());
    }
    assert_eq!(inner.unloaded, vec![(1, 0)]);
    assert!(sounds.chunk_sounds(1, 0).is_empty());
    assert_eq!(sounds.chunk_sounds(0, 0), &[id]);

    // Without an inner hook.
    let mut hook = ClearSoundsOnUnload::new(&mut sounds, None);
    cell.unload_chunk(0, 0, Some(&mut hook));
    assert!(sounds.chunk_sounds(0, 0).is_empty());
    // The sound itself stays in the manager's global list.
    let mut fx = NoEffects;
    let w = WorldSoundAdapter::new(&cell, 2, &mut fx);
    assert!(!w.chunk_loaded(0, 0));
    assert_eq!(sounds.sound(id).x, 4);
}
