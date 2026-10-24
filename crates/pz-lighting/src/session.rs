//! [`LightingSession`]: the library's global state and its native method
//! surface (`Java_zombie_iso_LightingJNI_*`), one method per native.

use std::collections::BTreeMap;

use crate::model::{
    Character, Light, LightingObject, ObjectKind, ResultLights, RoomLight, Torch, Transmission,
    NUM_LEVELS,
};
use crate::state::{EndFrame, LightingState, MapChange};
use crate::world::{edge, Chunk, SqId};

/// The Java exceptions the native code throws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightingError {
    /// `java.lang.IllegalArgumentException` with its message.
    IllegalArgument(&'static str),
    /// `java.lang.IllegalStateException` with its message.
    IllegalState(&'static str),
}

impl std::fmt::Display for LightingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LightingError::IllegalArgument(m) => write!(f, "IllegalArgumentException: {m}"),
            LightingError::IllegalState(m) => write!(f, "IllegalStateException: {m}"),
        }
    }
}

impl std::error::Error for LightingError {}

use LightingError::{IllegalArgument, IllegalState};

const INVALID_PLAYER: LightingError = IllegalArgument("invalid playerIndex");
const NO_BEGIN_UPDATE: LightingError = IllegalState("missing stateBeginUpdate?");
const NO_CHUNK: LightingError = IllegalState("missing chunkBeginUpdate?");
const NO_SQUARE: LightingError = IllegalState("missing squareBeginUpdate?");
const STATE_NULL: LightingError = IllegalState("state is null");

/// Number of `i32`s [`LightingSession::get_square_lighting`] produces.
pub const SQUARE_LIGHTING_LEN: usize = 44;

/// Lights, room lights and torches, shared by the four player states.
#[derive(Default)]
pub(crate) struct Globals {
    pub lights: Vec<Light>,
    pub light_ids: BTreeMap<i32, usize>,
    pub active_lights: Vec<usize>,
    pub removed_lights: Vec<usize>,
    pub free_lights: Vec<usize>,
    pub room_lights: Vec<RoomLight>,
    pub room_light_ids: BTreeMap<i32, usize>,
    pub active_room_lights: Vec<usize>,
    pub removed_room_lights: Vec<usize>,
    pub free_room_lights: Vec<usize>,
    pub torches: Vec<Torch>,
    pub torch_list: Vec<usize>,
    pub free_torches: Vec<usize>,
    /// `LightingSquare::rmod/gmod/bmod` (stored, never used).
    pub rgb_mod: [f32; 3],
}

/// The whole library: up to four player states plus the shared lights.
///
/// Calls mirror `zombie.iso.LightingJNI`'s natives. The native threading is
/// modelled synchronously: [`LightingSession::state_end_update`] hands the
/// queued changes over, [`LightingSession::do_lighting_update`] computes,
/// and the next `state_end_update` swaps the results in. Results therefore
/// lag the inputs by one hand-off, as in the game.
#[derive(Default)]
pub struct LightingSession {
    states: [Option<Box<LightingState>>; 4],
    current: Option<usize>,
    cur_chunk: Option<usize>,
    cur_level: Option<i32>,
    cur_square: Option<SqId>,
    globals: Globals,
    dark_step: f32,
}

fn check_player(p: i32) -> Result<usize, LightingError> {
    if (p as u32) < 4 {
        Ok(p as usize)
    } else {
        Err(INVALID_PLAYER)
    }
}

fn lights_change(queue: &mut Vec<MapChange>) -> (&mut Vec<usize>, &mut Vec<usize>) {
    let pos = match queue.iter().position(|c| matches!(c, MapChange::Lights { .. })) {
        Some(p) => p,
        None => {
            queue.push(MapChange::Lights { add: Vec::new(), remove: Vec::new() });
            queue.len() - 1
        }
    };
    match &mut queue[pos] {
        MapChange::Lights { add, remove } => (add, remove),
        _ => unreachable!(),
    }
}

fn erase_first(v: &mut Vec<usize>, x: usize) {
    if let Some(i) = v.iter().position(|&e| e == x) {
        v.remove(i);
    }
}

impl LightingSession {
    pub fn new() -> LightingSession {
        LightingSession::default()
    }

    fn cur(&mut self) -> Result<&mut LightingState, LightingError> {
        let p = self.current.ok_or(NO_BEGIN_UPDATE)?;
        Ok(self.states[p].as_deref_mut().expect("current state exists"))
    }

    /// `configure(darkStep)`. Stored, never read.
    pub fn configure(&mut self, dark_step: f32) {
        self.dark_step = dark_step;
    }

    fn scroll(&mut self, p: i32, change: MapChange) -> Result<(), LightingError> {
        let p = check_player(p)?;
        if let Some(st) = self.states[p].as_deref_mut() {
            st.queue.push(change);
        }
        Ok(())
    }

    pub fn scroll_left(&mut self, p: i32) -> Result<(), LightingError> {
        self.scroll(p, MapChange::Left)
    }

    pub fn scroll_right(&mut self, p: i32) -> Result<(), LightingError> {
        self.scroll(p, MapChange::Right)
    }

    pub fn scroll_up(&mut self, p: i32) -> Result<(), LightingError> {
        self.scroll(p, MapChange::Up)
    }

    pub fn scroll_down(&mut self, p: i32) -> Result<(), LightingError> {
        self.scroll(p, MapChange::Down)
    }

    /// `stateBeginUpdate(player, worldXMin, worldYMin, chunkGridW,
    /// chunkGridH)`. A new state is created when there is none or the grid
    /// size changed. Coordinates are in chunks.
    pub fn state_begin_update(&mut self, p: i32, x: i32, y: i32, w: i32, h: i32) -> Result<(), LightingError> {
        let pi = check_player(p)?;
        if self.current.is_some() {
            return Err(IllegalState("missing stateEndUpdate?"));
        }
        let keep = matches!(&self.states[pi], Some(st) if st.map.w == w && st.map.h == h);
        if !keep {
            self.states[pi] = Some(Box::new(LightingState::new(p, x, y, w, h)));
        }
        self.current = Some(pi);
        Ok(())
    }

    /// `stateEndFrame`. Repeated calls before a hand-off are merged.
    #[allow(clippy::too_many_arguments)]
    pub fn state_end_frame(
        &mut self,
        rmod: f32,
        gmod: f32,
        bmod: f32,
        ambient: f32,
        night: f32,
        view_dist: f32,
        view_dist_max: f32,
        cache_cleared: bool,
        light_source_update: f32,
        sky_light_level: i32,
    ) -> Result<(), LightingError> {
        let max_player = if self.states[3].is_some() {
            3
        } else if self.states[2].is_some() {
            2
        } else if self.states[1].is_some() {
            1
        } else if self.states[0].is_some() {
            0
        } else {
            -1
        };
        let st = self.cur()?;
        for c in &mut st.queue {
            if let MapChange::EndFrame(ef) = c {
                ef.rmod = rmod;
                ef.gmod = gmod;
                ef.bmod = bmod;
                ef.ambient = ambient;
                ef.night = night;
                ef.sky = sky_light_level as u32;
                ef.view_dist = view_dist;
                ef.view_dist_max = view_dist_max;
                ef.cache_cleared |= cache_cleared;
                if ef.light_source_update < light_source_update {
                    ef.light_source_update = light_source_update;
                }
                ef.frames += 1;
                return Ok(());
            }
        }
        st.queue.push(MapChange::EndFrame(EndFrame {
            rmod,
            gmod,
            bmod,
            ambient,
            night,
            view_dist,
            view_dist_max,
            cache_cleared,
            light_source_update,
            frames: 1,
            sky: sky_light_level as u32,
            max_player,
        }));
        Ok(())
    }

    /// `stateEndUpdate`: hands the queued changes to the update, or, when
    /// an update has finished, swaps its results in and syncs the lights.
    pub fn state_end_update(&mut self) -> Result<(), LightingError> {
        let p = self.current.ok_or(NO_BEGIN_UPDATE)?;
        let st = self.states[p].as_deref_mut().expect("current state exists");
        if !st.work {
            if st.queue.iter().any(|c| matches!(c, MapChange::Player(_))) {
                std::mem::swap(&mut st.queue, &mut st.pending);
                st.work = true;
            }
        } else if st.done {
            std::mem::swap(&mut st.queue, &mut st.pending);
            Self::sync_lights(&mut self.globals, st, p);
            st.swap_results();
        }
        self.current = None;
        Ok(())
    }

    /// The light, room-light and torch bookkeeping of `stateEndUpdate`.
    fn sync_lights(g: &mut Globals, st: &mut LightingState, p: usize) {
        let actives = g.active_lights.clone();
        for l in actives {
            if !g.lights[l].added[p] {
                let (add, remove) = lights_change(&mut st.pending);
                erase_first(remove, l);
                add.push(l);
                g.lights[l].added[p] = true;
            }
            let light = &mut g.lights[l];
            light.active_snap[p] = light.active;
            if light.life_max > 0 && light.life < 1 && light.active {
                let id = light.id;
                light.active = false;
                g.light_ids.remove(&id);
                erase_first(&mut g.active_lights, l);
                g.removed_lights.push(l);
            }
        }
        let removed = g.removed_lights.clone();
        for l in removed {
            if g.lights[l].added[p] {
                let (add, remove) = lights_change(&mut st.pending);
                erase_first(add, l);
                remove.push(l);
            }
            if g.lights[l].added.iter().all(|a| !a) {
                g.free_lights.push(l);
                erase_first(&mut g.removed_lights, l);
            }
        }
        for &rl in &g.active_room_lights {
            let r = &mut g.room_lights[rl];
            if !r.added[p] {
                st.room_lights.push(rl);
                r.added[p] = true;
            }
            r.active_snap[p] = r.active;
        }
        let removed = g.removed_room_lights.clone();
        for rl in removed {
            st.room_lights.retain(|&x| x != rl);
            g.room_lights[rl].added[p] = false;
            if g.room_lights[rl].added.iter().all(|a| !a) {
                g.free_room_lights.push(rl);
                erase_first(&mut g.removed_room_lights, rl);
            }
        }
        let mut i = 0;
        while i < g.torch_list.len() {
            let t = g.torch_list[i];
            if g.torches[t].id == 0 {
                if g.torches[t].added[p] {
                    erase_first(&mut st.torches, t);
                    g.torches[t].added[p] = false;
                }
                if g.torches[t].added.iter().all(|a| !a) {
                    g.free_torches.push(t);
                    g.torch_list.remove(i);
                    continue;
                }
            } else if !g.torches[t].added[p] {
                st.torches.push(t);
                g.torches[t].added[p] = true;
            }
            i += 1;
        }
    }

    /// `stateUpdateCounter(player)`: -1 before the first results.
    pub fn state_update_counter(&self, p: i32) -> Result<i32, LightingError> {
        let p = check_player(p)?;
        Ok(self.states[p].as_ref().map(|s| s.update_counter).unwrap_or(-1))
    }

    /// `teleport(player, chunkX, chunkY)`.
    pub fn teleport(&mut self, p: i32, x: i32, y: i32) -> Result<(), LightingError> {
        let p = check_player(p)?;
        let st = self.states[p].as_deref_mut().ok_or(STATE_NULL)?;
        for c in &mut st.queue {
            if let MapChange::Teleport { x: tx, y: ty } = c {
                *tx = x;
                *ty = y;
                return Ok(());
            }
        }
        st.queue.push(MapChange::Teleport { x, y });
        Ok(())
    }

    /// `DoLightingUpdateNew(nanos, dirtyGlobal)`: runs the pending update
    /// of every state that has one.
    pub fn do_lighting_update(&mut self, nanos: u64, dirty_global: bool) {
        for i in 0..4 {
            let Some(st) = self.states[i].as_deref_mut() else { continue };
            if !st.done && st.work {
                st.apply_map_changes(&mut self.globals);
                st.dirty_global = dirty_global;
                st.done = st.do_lighting_update(&mut self.globals, nanos);
            }
        }
    }

    /// `WaitingForMain()`.
    pub fn waiting_for_main(&self) -> bool {
        self.states.iter().flatten().any(|s| s.done)
    }

    /// `playerSet`. `z` is the native level (`z + 32`).
    pub fn player_set(&mut self, c: Character) -> Result<(), LightingError> {
        let st = self.cur()?;
        for ch in &mut st.queue {
            if let MapChange::Player(old) = ch {
                **old = c;
                return Ok(());
            }
        }
        st.queue.push(MapChange::Player(Box::new(c)));
        Ok(())
    }

    /// `chunkLightingDone(wx, wy)`.
    pub fn chunk_lighting_done(&mut self, wx: i32, wy: i32) -> Result<bool, LightingError> {
        let st = self.cur()?;
        let f = &st.front[32];
        let cx = wx.wrapping_sub(f.ox / 8);
        let cy = wy.wrapping_sub(f.oy / 8);
        if cx < 0 || cx >= st.map.w || cy < 0 || cy >= st.map.h {
            return Ok(false);
        }
        Ok(st.map.done[(st.map.w * cy + cx) as usize] != 0)
    }

    /// `getChunkDirty(player, wx, wy, z)`: reads and clears.
    pub fn get_chunk_dirty(&mut self, p: i32, wx: i32, wy: i32, z: i32) -> Result<bool, LightingError> {
        let p = check_player(p)?;
        if self.current.is_some() {
            return Err(IllegalState("missing stateEndUpdate?"));
        }
        let Some(st) = self.states[p].as_deref_mut() else { return Ok(false) };
        if !(0..NUM_LEVELS as i32).contains(&z) {
            return Ok(false);
        }
        let (w, h) = (st.w, st.h);
        let rl = &mut st.front[z as usize];
        let cx = wx.wrapping_sub(rl.ox / 8);
        let cy = wy.wrapping_sub(rl.oy / 8);
        if cx < 0 || cx >= st.map.w || cy < 0 || cy >= st.map.h {
            return Ok(false);
        }
        let i = (st.map.w * cy + cx) as usize;
        let d = rl.data_mut(w, h);
        let v = d.chunk_dirty[i];
        d.chunk_dirty[i] = 0;
        Ok(v != 0)
    }

    /// `chunkBeginUpdate(wx, wy, minLevel, maxLevel)` (native levels).
    pub fn chunk_begin_update(&mut self, wx: i32, wy: i32, min: i32, max: i32) -> Result<(), LightingError> {
        if self.cur_chunk.is_some() {
            self.cur()?;
            return Err(IllegalState("missing chunkEndUpdate?"));
        }
        let st = self.cur()?;
        let found = st.queue.iter().position(|c| matches!(c, MapChange::Chunk { chunk, .. } if chunk.wx == wx && chunk.wy == wy));
        let pos = match found {
            Some(pos) => pos,
            None => {
                st.queue.push(MapChange::Chunk { chunk: Box::new(Chunk::new(wx, wy)), updated: [false; NUM_LEVELS] });
                st.queue.len() - 1
            }
        };
        let mut freed = Vec::new();
        if let MapChange::Chunk { chunk, updated } = &mut st.queue[pos] {
            if min <= max {
                for z in min..=max {
                    if let Some(u) = updated.get_mut(z as usize) {
                        *u = false;
                    }
                }
            }
            set_min_max_level(chunk, min, max, &mut freed);
        }
        for id in freed {
            st.free_square(id);
        }
        self.cur_chunk = Some(pos);
        Ok(())
    }

    /// `chunkEndUpdate()`.
    pub fn chunk_end_update(&mut self) -> Result<(), LightingError> {
        if self.cur_chunk.take().is_none() {
            return Err(NO_CHUNK);
        }
        Ok(())
    }

    fn cur_chunk(&mut self) -> Result<(&mut LightingState, usize), LightingError> {
        let pos = self.cur_chunk.ok_or(NO_CHUNK)?;
        let p = self.current.ok_or(NO_CHUNK)?;
        Ok((self.states[p].as_deref_mut().expect("current state exists"), pos))
    }

    /// `chunkLevelBeginUpdate(z)`: marks the level as updated.
    pub fn chunk_level_begin_update(&mut self, z: i32) -> Result<(), LightingError> {
        let has_level = self.cur_level.is_some();
        let (st, pos) = self.cur_chunk()?;
        if has_level {
            return Err(IllegalState("missing chunkLevelEndUpdate?"));
        }
        let MapChange::Chunk { chunk, updated } = &mut st.queue[pos] else { return Err(NO_CHUNK) };
        let in_range = chunk.level(z).is_some();
        if let Some(u) = updated.get_mut(z as usize) {
            *u = true;
        }
        self.cur_level = in_range.then_some(z);
        Ok(())
    }

    /// `chunkLevelEndUpdate()`.
    pub fn chunk_level_end_update(&mut self) -> Result<(), LightingError> {
        if self.cur_level.take().is_none() {
            return Err(IllegalState("missing chunkLevelBeginUpdate?"));
        }
        Ok(())
    }

    /// `squareSetNull(x, y, z)` (chunk-local x/y, native level).
    pub fn square_set_null(&mut self, x: i32, y: i32, z: i32) -> Result<(), LightingError> {
        let (st, pos) = self.cur_chunk()?;
        let MapChange::Chunk { chunk, .. } = &mut st.queue[pos] else { return Err(NO_CHUNK) };
        if z < chunk.min || ((x | y) as u32) >= 8 || z > chunk.max {
            return Err(IllegalArgument("invalid x,y,z"));
        }
        let slot = &mut chunk.level_mut(z).unwrap()[(x * 8 + y) as usize];
        if let Some(id) = slot.take() {
            st.free_square(id);
        }
        Ok(())
    }

    /// `squareBeginUpdate(x, y, z)` (chunk-local x/y, native level).
    pub fn square_begin_update(&mut self, x: i32, y: i32, z: i32) -> Result<(), LightingError> {
        let has_square = self.cur_square.is_some();
        let (st, pos) = self.cur_chunk()?;
        if has_square {
            return Err(IllegalState("missing squareEndUpdate?"));
        }
        let MapChange::Chunk { chunk, .. } = &st.queue[pos] else { return Err(NO_CHUNK) };
        if z < chunk.min || ((x | y) as u32) >= 8 || z > chunk.max {
            return Err(IllegalArgument("invalid x,y,z"));
        }
        let (wx, wy) = (chunk.wx, chunk.wy);
        let existing = chunk.level(z).unwrap()[(x * 8 + y) as usize];
        let id = match existing {
            Some(id) => id,
            None => {
                let id = st.alloc_square();
                st.sq_mut(id).begin(x + wx * 8, y + wy * 8, z);
                if let MapChange::Chunk { chunk, .. } = &mut st.queue[pos] {
                    chunk.level_mut(z).unwrap()[(x * 8 + y) as usize] = Some(id);
                }
                id
            }
        };
        self.cur_square = Some(id);
        Ok(())
    }

    fn cur_square(&mut self) -> Result<(&mut LightingState, SqId), LightingError> {
        let id = self.cur_square.ok_or(NO_SQUARE)?;
        let p = self.current.ok_or(NO_SQUARE)?;
        Ok((self.states[p].as_deref_mut().expect("current state exists"), id))
    }

    /// `squareSet`. `vision_unblocked` has one bit per `IsoDirections`
    /// ordinal (N, NW, W, SW, S, SE, E, NE); the bits are remapped to the
    /// native layout (see [`crate::flags`]).
    #[allow(clippy::too_many_arguments)]
    pub fn square_set(
        &mut self,
        vision_unblocked: i32,
        up_clear: bool,
        down_clear: bool,
        elevated_floor: bool,
        vision_matrix: i32,
        building_id: i64,
        room_id: i64,
        light_level: i32,
        open_air: bool,
    ) -> Result<(), LightingError> {
        let (st, id) = self.cur_square()?;
        let player = st.player;
        let v = vision_unblocked as u32;
        let bit = |b: u32| ((v >> b) & 1) as u16;
        let s = st.sq_mut(id);
        let mut f = bit(2)
            | (bit(0) << 1)
            | (bit(6) << 2)
            | (bit(4) << 3)
            | (bit(1) << 4)
            | (bit(7) << 5)
            | (bit(5) << 6)
            | (bit(3) << 7)
            | (u16::from(up_clear) << 8)
            | (u16::from(down_clear) << 9)
            | (u16::from(open_air) << 10)
            | (u16::from(elevated_floor) << 11)
            | (s.flags & 0xf000);
        s.set_level(light_level as u32);
        s.vision_matrix = vision_matrix as u32;
        s.building_id = building_id;
        s.room_id = room_id;
        for t in &mut s.trans {
            t.r = 0.0;
            t.g = 0.0;
            t.b = 0.0;
        }
        s.objects.clear();
        let seen = ((vision_matrix as u32) >> ((player as u32 + 27) & 31)) & 1;
        f = (f & 0xefff) | ((seen as u16) << 12);
        s.flags = f;
        Ok(())
    }

    /// `squareSetLightTransmission`: W, N, E, S.
    pub fn square_set_light_transmission(&mut self, t: [Transmission; 4]) -> Result<(), LightingError> {
        let (st, id) = self.cur_square()?;
        let s = st.sq_mut(id);
        s.trans[edge::W] = t[0];
        s.trans[edge::N] = t[1];
        s.trans[edge::E] = t[2];
        s.trans[edge::S] = t[3];
        Ok(())
    }

    fn add_object(&mut self, kind: ObjectKind, flags: u16) -> Result<(), LightingError> {
        let (st, id) = self.cur_square()?;
        st.sq_mut(id).objects.push(LightingObject { owner: id, kind, flags });
        Ok(())
    }

    /// `squareAddCurtain(wnes, open)`: `wnes` 4 W, 8 N, 0x10 E, 0x20 S.
    pub fn square_add_curtain(&mut self, wnes: i32, open: bool) -> Result<(), LightingError> {
        let wnes = wnes as u16;
        let f = u16::from(open) | (u16::from(wnes & 0x28 != 0) << 1) | wnes;
        self.add_object(ObjectKind::Curtain, f)
    }

    /// `squareAddDoor(north, open, transparent)`.
    pub fn square_add_door(&mut self, north: bool, open: bool, trans: bool) -> Result<(), LightingError> {
        let f = u16::from(open) | (u16::from(north) << 1) | (u16::from(trans) << 2);
        self.add_object(ObjectKind::Door, f)
    }

    /// `squareAddThumpable(north, open, isDoor, doorTrans)`.
    pub fn square_add_thumpable(&mut self, north: bool, open: bool, is_door: bool, door_trans: bool) -> Result<(), LightingError> {
        let f = u16::from(open) | (u16::from(north) << 1) | (u16::from(is_door) << 2) | (u16::from(door_trans) << 3);
        self.add_object(ObjectKind::Thumpable, f)
    }

    /// `squareAddWindow(north, open, opaque)`.
    pub fn square_add_window(&mut self, north: bool, open: bool, opaque: bool) -> Result<(), LightingError> {
        let f = u16::from(open) | (u16::from(north) << 1) | (u16::from(opaque) << 2);
        self.add_object(ObjectKind::Window, f)
    }

    /// `squareEndUpdate()`.
    pub fn square_end_update(&mut self) -> Result<(), LightingError> {
        if self.cur_square.take().is_none() {
            return Err(NO_SQUARE);
        }
        Ok(())
    }

    // ---------------------------------------------------------------
    // Result getters

    fn front_index(&self, p: i32, x: i32, y: i32, z: i32) -> Result<Option<(&LightingState, usize)>, LightingError> {
        let pi = check_player(p)?;
        let st = self.states[pi].as_deref().ok_or(STATE_NULL)?;
        Ok(Self::index_in(st, x, y, z).map(|i| (st, i)))
    }

    fn index_in(st: &LightingState, x: i32, y: i32, z: i32) -> Option<usize> {
        if !(0..NUM_LEVELS as i32).contains(&z) {
            return None;
        }
        let rl = &st.front[z as usize];
        let lx = x.wrapping_sub(rl.ox);
        let ly = y.wrapping_sub(rl.oy);
        if lx < 0 || ly < 0 || lx >= st.w || ly >= st.h {
            return None;
        }
        Some((lx + st.w * ly) as usize)
    }

    /// `getVertLight(player, x, y, z, i)`: vertex colour `0xFFbbggrr`.
    pub fn get_vert_light(&self, p: i32, x: i32, y: i32, z: i32, i: i32) -> Result<i32, LightingError> {
        let Some((st, idx)) = self.front_index(p, x, y, z)? else { return Ok(0) };
        if !(0..8).contains(&i) {
            return Ok(0);
        }
        Ok(st.front[z as usize].data.as_ref().map(|d| d.vert[i as usize][idx] as i32).unwrap_or(0))
    }

    /// `getLightInfo(player, x, y, z, channel)`: 0 red, 1 green, else blue.
    pub fn get_light_info(&self, p: i32, x: i32, y: i32, z: i32, ch: i32) -> Result<f32, LightingError> {
        let Some((st, idx)) = self.front_index(p, x, y, z)? else { return Ok(0.0) };
        let v = st.front[z as usize].data.as_ref().map_or(0, |d| match ch {
            1 => d.g[idx],
            0 => d.r[idx],
            _ => d.b[idx],
        });
        Ok(f32::from(v) / 255.0)
    }

    /// `getDarkMulti(player, x, y, z)`.
    pub fn get_dark_multi(&self, p: i32, x: i32, y: i32, z: i32) -> Result<f32, LightingError> {
        let Some((st, idx)) = self.front_index(p, x, y, z)? else { return Ok(0.0) };
        Ok(st.front[z as usize].data.as_ref().map_or(0.0, |d| d.dark[idx]))
    }

    /// `getTargetDarkMulti(player, x, y, z)`.
    pub fn get_target_dark_multi(&self, p: i32, x: i32, y: i32, z: i32) -> Result<f32, LightingError> {
        let Some((st, idx)) = self.front_index(p, x, y, z)? else { return Ok(0.0) };
        Ok(st.front[z as usize].data.as_ref().map_or(0.0, |d| d.target[idx]))
    }

    fn vis_bit(&self, p: i32, x: i32, y: i32, z: i32, bit: u8) -> Result<bool, LightingError> {
        let pi = check_player(p)?;
        let Some(st) = self.states[pi].as_deref() else { return Ok(false) };
        let Some(idx) = Self::index_in(st, x, y, z) else { return Ok(false) };
        Ok(st.front[z as usize].data.as_ref().is_some_and(|d| d.vis[idx] & bit != 0))
    }

    /// `getSeen(player, x, y, z)`.
    pub fn get_seen(&self, p: i32, x: i32, y: i32, z: i32) -> Result<bool, LightingError> {
        self.vis_bit(p, x, y, z, 1)
    }

    /// `getCanSee(player, x, y, z)`.
    pub fn get_can_see(&self, p: i32, x: i32, y: i32, z: i32) -> Result<bool, LightingError> {
        self.vis_bit(p, x, y, z, 2)
    }

    /// `getCouldSee(player, x, y, z)`.
    pub fn get_could_see(&self, p: i32, x: i32, y: i32, z: i32) -> Result<bool, LightingError> {
        self.vis_bit(p, x, y, z, 4)
    }

    /// `getSquareLighting(player, x, y, z, int[])`. Writes the layout the
    /// Java side decodes (see [`crate::SquareLighting`]); light slots past
    /// the light count are left untouched. Clears the square's dirty flag.
    /// Returns `Ok(false)` when there is no state or the square is outside
    /// the grid.
    pub fn get_square_lighting_into(&mut self, p: i32, x: i32, y: i32, z: i32, out: &mut [i32]) -> Result<bool, LightingError> {
        let pi = check_player(p)?;
        let Some(st) = self.states[pi].as_deref_mut() else { return Ok(false) };
        let Some(idx) = Self::index_in(st, x, y, z) else { return Ok(false) };
        if out.len() < 43 {
            return Err(IllegalArgument("output array is too small"));
        }
        let (w, h) = (st.w, st.h);
        let d = st.front[z as usize].data_mut(w, h);
        out[0] = i32::from(d.vis[idx] & 7);
        out[1] = i32::from(d.r[idx]) | (i32::from(d.g[idx]) << 8) | (i32::from(d.b[idx]) << 16);
        out[2] = (f64::from(d.dark[idx]) * 100000.0) as i32;
        out[3] = (f64::from(d.target[idx]) * 100000.0) as i32;
        out[4] = d.level[idx] as i32;
        for k in 0..8 {
            out[5 + k] = d.vert[k][idx] as i32;
        }
        let mut lights: ResultLights = d.statics[idx];
        let torches = d.torches[idx];
        if torches.count > 0 {
            for e in &torches.lights[..(torches.count as usize).min(5)] {
                let pk = e.packed;
                lights.add_light(
                    e.id,
                    e.x,
                    e.y,
                    e.z,
                    i32::from(e.radius),
                    (pk & 0xff) as f32 / 255.0,
                    ((pk >> 8) & 0xff) as f32 / 255.0,
                    ((pk >> 16) & 0xff) as f32 / 255.0,
                    (pk >> 24) as u8,
                    e.dist,
                );
            }
        }
        out[13] = lights.count.min(5);
        for (k, e) in lights.lights.iter().take(lights.count.clamp(0, 5) as usize).enumerate() {
            let base = 14 + 6 * k;
            let vals = [e.id, e.x, e.y, e.z, i32::from(e.radius), e.packed as i32];
            for (j, v) in vals.into_iter().enumerate() {
                if let Some(o) = out.get_mut(base + j) {
                    *o = v;
                }
            }
        }
        d.dirty[idx] = 0;
        Ok(true)
    }

    /// [`Self::get_square_lighting_into`] into a zeroed array.
    pub fn get_square_lighting(&mut self, p: i32, x: i32, y: i32, z: i32) -> Result<Option<[i32; SQUARE_LIGHTING_LEN]>, LightingError> {
        let mut out = [0i32; SQUARE_LIGHTING_LEN];
        Ok(self.get_square_lighting_into(p, x, y, z, &mut out)?.then_some(out))
    }

    /// `getSquareDirty(player, x, y, z)`.
    pub fn get_square_dirty(&self, p: i32, x: i32, y: i32, z: i32) -> Result<bool, LightingError> {
        let pi = check_player(p)?;
        let Some(st) = self.states[pi].as_deref() else { return Ok(false) };
        let Some(idx) = Self::index_in(st, x, y, z) else { return Ok(false) };
        let rl = &st.front[z as usize];
        let lx = x - rl.ox;
        let ly = y - rl.oy;
        let dirty = rl.data.as_ref().map_or(1, |d| d.dirty[idx]);
        let v = if dirty & 1 != 0 { 1 } else { u8::from(st.map.full_dirty) };
        let ci = (st.map.w * (ly >> 3) + (lx >> 3)) as usize;
        Ok(st.map.done.get(ci).copied().unwrap_or(0) & v != 0)
    }

    // ---------------------------------------------------------------
    // Lights

    fn take_light_slot(g: &mut Globals) -> usize {
        match g.free_lights.pop() {
            Some(i) => i,
            None => {
                g.lights.push(Light::default());
                g.lights.len() - 1
            }
        }
    }

    /// `addLight(id, x, y, z, radius, r, g, b, buildingId, active)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_light(&mut self, id: i32, x: i32, y: i32, z: i32, radius: i32, r: f32, g: f32, b: f32, _building_id: i32, active: bool) -> Result<(), LightingError> {
        let gl = &mut self.globals;
        if gl.light_ids.contains_key(&id) {
            return Err(IllegalState("duplicate light id"));
        }
        let slot = Self::take_light_slot(gl);
        let snap = gl.lights[slot].active_snap;
        gl.lights[slot] = Light { id, x, y, z, radius, r, g, b, active, life: -1, life_max: -1, added: [false; 4], active_snap: snap };
        gl.active_lights.push(slot);
        gl.light_ids.insert(id, slot);
        Ok(())
    }

    /// `addTempLight(id, x, y, z, radius, r, g, b, lifeFrames)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_temp_light(&mut self, id: i32, x: i32, y: i32, z: i32, radius: i32, r: f32, g: f32, b: f32, life: i32) {
        let gl = &mut self.globals;
        let slot = Self::take_light_slot(gl);
        let snap = gl.lights[slot].active_snap;
        gl.lights[slot] = Light { id, x, y, z, radius, r, g, b, active: true, life, life_max: life, added: [false; 4], active_snap: snap };
        gl.active_lights.push(slot);
        gl.light_ids.insert(id, slot);
    }

    /// `removeLight(id)`.
    pub fn remove_light(&mut self, id: i32) -> Result<(), LightingError> {
        let gl = &mut self.globals;
        let slot = gl.light_ids.remove(&id).ok_or(IllegalState("unknown light id"))?;
        gl.active_lights.retain(|&l| l != slot);
        if gl.lights[slot].added.iter().all(|a| !a) {
            gl.free_lights.push(slot);
        } else {
            gl.removed_lights.push(slot);
        }
        Ok(())
    }

    /// `setLightActive(id, active)`.
    pub fn set_light_active(&mut self, id: i32, active: bool) -> Result<(), LightingError> {
        let gl = &mut self.globals;
        let slot = *gl.light_ids.get(&id).ok_or(IllegalState("unknown light id"))?;
        gl.lights[slot].active = active;
        Ok(())
    }

    /// `setLightColor(id, r, g, b)`.
    pub fn set_light_color(&mut self, id: i32, r: f32, g: f32, b: f32) -> Result<(), LightingError> {
        let gl = &mut self.globals;
        let slot = *gl.light_ids.get(&id).ok_or(IllegalState("unknown light id"))?;
        let l = &mut gl.lights[slot];
        l.r = r;
        l.g = g;
        l.b = b;
        Ok(())
    }

    /// `addRoomLight(id, buildingId, roomId, x, y, z, w, h, active)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_room_light(&mut self, id: i32, building_id: i64, room_id: i64, x: i32, y: i32, z: i32, w: i32, h: i32, active: bool) -> Result<(), LightingError> {
        let gl = &mut self.globals;
        if gl.room_light_ids.contains_key(&id) {
            return Err(IllegalState("duplicate room-light id"));
        }
        let slot = match gl.free_room_lights.pop() {
            Some(i) => i,
            None => {
                gl.room_lights.push(RoomLight::default());
                gl.room_lights.len() - 1
            }
        };
        let snap = gl.room_lights[slot].active_snap;
        gl.room_lights[slot] = RoomLight { id, building_id, room_id, x, y, z, w, h, active, added: [false; 4], active_snap: snap };
        gl.active_room_lights.push(slot);
        gl.room_light_ids.insert(id, slot);
        Ok(())
    }

    /// `removeRoomLight(id)`.
    pub fn remove_room_light(&mut self, id: i32) -> Result<(), LightingError> {
        let gl = &mut self.globals;
        let slot = gl.room_light_ids.remove(&id).ok_or(IllegalState("unknown room-light id"))?;
        gl.active_room_lights.retain(|&l| l != slot);
        if gl.room_lights[slot].added.iter().all(|a| !a) {
            gl.free_room_lights.push(slot);
        } else {
            gl.removed_room_lights.push(slot);
        }
        Ok(())
    }

    /// `setRoomLightActive(id, active)`.
    pub fn set_room_light_active(&mut self, id: i32, active: bool) -> Result<(), LightingError> {
        let gl = &mut self.globals;
        let slot = *gl.room_light_ids.get(&id).ok_or(IllegalState("unknown room-light id"))?;
        gl.room_lights[slot].active = active;
        Ok(())
    }

    /// `updateTorch(id, x, y, z, r, g, b, angleX, angleY, dist, strength,
    /// cone, dot, focusing)`. `z` is the native level.
    #[allow(clippy::too_many_arguments)]
    pub fn update_torch(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        z: f32,
        r: f32,
        g: f32,
        b: f32,
        angle_x: f32,
        angle_y: f32,
        dist: f32,
        strength: f32,
        cone: bool,
        dot: f32,
        focusing: i32,
    ) {
        let gl = &mut self.globals;
        let slot = match gl.torch_list.iter().copied().find(|&t| gl.torches[t].id == id) {
            Some(t) => t,
            None => {
                let t = match gl.free_torches.pop() {
                    Some(t) => t,
                    None => {
                        gl.torches.push(Torch::default());
                        gl.torches.len() - 1
                    }
                };
                gl.torches[t] = Torch { id, ..Torch::default() };
                gl.torch_list.push(t);
                t
            }
        };
        let added = gl.torches[slot].added;
        gl.torches[slot] = Torch { id: gl.torches[slot].id, x, y, z, r, g, b, angle_x, angle_y, dist, strength, cone, dot, focusing, added };
    }

    /// `removeTorch(id)`.
    pub fn remove_torch(&mut self, id: i32) {
        let gl = &mut self.globals;
        if let Some(t) = gl.torch_list.iter().copied().find(|&t| gl.torches[t].id == id) {
            gl.torches[t].id = 0;
        }
    }

    /// `getVisibleRoomCount(player)`.
    pub fn get_visible_room_count(&self, p: i32) -> Result<i32, LightingError> {
        let p = check_player(p)?;
        Ok(self.states[p].as_ref().map_or(0, |s| s.visible_rooms.len() as i32))
    }

    /// `getVisibleRooms(player, long[])`: room ids in ascending unsigned
    /// order.
    pub fn get_visible_rooms(&self, p: i32, out: &mut [i64]) -> Result<i32, LightingError> {
        let p = check_player(p)?;
        let Some(st) = self.states[p].as_deref() else { return Ok(0) };
        if out.len() < st.visible_rooms.len() {
            return Err(IllegalArgument("output array is too small"));
        }
        for (o, &r) in out.iter_mut().zip(st.visible_rooms.iter()) {
            *o = r as i64;
        }
        Ok(st.visible_rooms.len() as i32)
    }

    /// Parity-testing hook: floods the sky seeds of `player`'s next full
    /// relight in the given order (world x, y and native level per seed)
    /// instead of insertion order. Seeds missing from the list go last.
    #[doc(hidden)]
    pub fn debug_set_sky_seed_order(&mut self, player: i32, order: Vec<(i32, i32, i32)>) {
        if let Some(st) = check_player(player).ok().and_then(|p| self.states[p].as_deref_mut()) {
            st.seed_order = Some(order);
        }
    }

    /// `destroy()`: drops every state and light.
    pub fn destroy(&mut self) {
        let dark_step = self.dark_step;
        *self = LightingSession::default();
        self.dark_step = dark_step;
    }
}

/// `LightingChunk::setMinMaxLevel`: keeps the levels inside the new range
/// and adds empty ones; squares of dropped levels are returned in `freed`.
fn set_min_max_level(chunk: &mut Chunk, min: i32, max: i32, freed: &mut Vec<SqId>) {
    if chunk.min == min && chunk.max == max {
        return;
    }
    let mut levels = Vec::new();
    for z in min..=max {
        match chunk.level(z) {
            Some(l) => levels.push(*l),
            None => levels.push([None; 64]),
        }
    }
    for z in chunk.min..=chunk.max {
        if z < min || z > max {
            freed.extend(chunk.level(z).unwrap().iter().flatten().copied());
        }
    }
    chunk.levels = levels;
    chunk.min = min;
    chunk.max = max;
}
