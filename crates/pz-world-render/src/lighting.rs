//! pz-world squares, lamps and room lights as pz-lighting inputs, and the
//! per-frame lighting driver (`LightingJNI.update()` for a player, the
//! lighting thread's update, and decoding the results).

use std::collections::{BTreeSet, HashMap};

use pz_iso_render::color::ColorInfo as RenderColor;
use pz_lighting::jni::{
    detection_range, effective_fatigue, luma_inverted, update_player, ChunkInfo, ChunkLevelLight,
    Curtain, CurtainType, DecodePlayer, Door, FrameParams, GridSquare, IsoDirection, LightSource,
    LightSync, LightingWorld, PlayerFrame, PlayerInput, ResultLightInfo, RoomLightSource,
    SpecialObject, SquareLight, Thumpable, VisionCone, VisionConeInput, Window,
};
use pz_lighting::{LightingError, LightingSession};
use pz_world::{IsoGridSquare, ObjectKind};

use crate::hooks::{lighting_barricade, WorldHooks};
use crate::render_world::{RenderWorld, WorldView};
use crate::settings::RenderSettingsSource;
use crate::{HostResult, RuntimeHostError};

/// `MetaObject` type of a light switch (`RoomDef.objects`, used by
/// `IsoRoom.hasLightSwitches`).
const META_LIGHT_SWITCH: i32 = 7;

/// The lighting view of every loaded square ([`LightingWorld`]).
#[derive(Default)]
pub struct LightingSquares(pub HashMap<(i32, i32, i32), GridSquare>);

impl LightingWorld for LightingSquares {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&GridSquare> {
        self.0.get(&(x, y, z))
    }
}

/// `getBuildingDef().getID()` of the square's room.
/// The players that had seen `sq` when it was loaded, as a bit mask.
fn loaded_seen_by(sq: &IsoGridSquare) -> u8 {
    sq.seen
        .iter()
        .enumerate()
        .fold(0, |m, (i, &s)| if s { m | 1 << i } else { m })
}

fn building_def_id(w: &WorldView<'_>, sq: &IsoGridSquare) -> Option<i64> {
    let room = sq.room_def(w.meta)?;
    Some(w.meta.building_defs[room.building?.0 as usize].id)
}

/// `square` as `updateChunk` reads it. `light_level` and `seen_by` are the
/// world's running values (fed back from the decoded results).
pub fn grid_square(
    w: &WorldView<'_>,
    sq: &IsoGridSquare,
    hooks: &dyn WorldHooks,
    light_level: i32,
    seen_by: u8,
) -> HostResult<GridSquare> {
    let t = &w.sprites.tables;
    let mut special_objects = Vec::new();
    for &id in &sq.special_objects {
        let mut index = None;
        for (i, object) in sq.objects.iter().enumerate() {
            let object = object
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("lighting special-object lookup"))?;
            if object.id == id {
                index = Some(i);
                break;
            }
        }
        let Some(i) = index else { continue };
        let o = sq.objects[i]
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("lighting object"))?;
        match &o.kind {
            ObjectKind::Door(d) => {
                let (door_trans, garage_door) = {
                    let sp = o.sprite.try_read(w.sprites)?;
                    (
                        sp.as_deref().is_some_and(|sp| {
                            sp.properties.has_name(&w.sprites.alias, "doorTrans")
                        }),
                        sp.as_deref().is_some_and(|sp| {
                            sp.properties.has_name(&w.sprites.alias, "GarageDoor")
                        }),
                    )
                };
                let (same, opposite) = hooks.barricades(sq, &o);
                let curtain = hooks.door_curtain(sq, &o);
                special_objects.push(SpecialObject::Door(Door {
                    north: d.north,
                    open: d.open,
                    door_trans,
                    garage_door,
                    has_curtain: curtain.is_some(),
                    curtain_open: curtain.unwrap_or(false),
                    barricade_same: same,
                    barricade_opposite: opposite,
                }));
            }
            ObjectKind::Window(win) => {
                let (same, opposite) = hooks.barricades(sq, &o);
                special_objects.push(SpecialObject::Window(Window {
                    north: win.north,
                    open: win.open,
                    barricade_same: same,
                    barricade_opposite: opposite,
                }));
            }
            ObjectKind::Curtain(c) => {
                let sp = o.sprite.try_read(w.sprites)?;
                let prop = |name| {
                    sp.as_deref()
                        .and_then(|sp| sp.properties.get(&w.sprites.alias, name))
                        .map(str::to_string)
                };
                let kind = match sp.as_deref().map(|sp| sp.tile_type) {
                    Some(k) if k == t.ty.curtainW => CurtainType::W,
                    Some(k) if k == t.ty.curtainN => CurtainType::N,
                    Some(k) if k == t.ty.curtainE => CurtainType::E,
                    Some(k) if k == t.ty.curtainS => CurtainType::S,
                    _ => CurtainType::Other,
                };
                special_objects.push(SpecialObject::Curtain(Curtain {
                    kind,
                    north: c.north,
                    open: c.open,
                    light_filter_r: prop("LightFilterR"),
                    light_filter_g: prop("LightFilterG"),
                    light_filter_b: prop("LightFilterB"),
                    light_filter_intensity: prop("LightFilterIntensity"),
                    light_filter_mix: prop("LightFilterMix"),
                }));
            }
            ObjectKind::Thumpable(th) => {
                let (is_window, door_trans) = {
                    let sp = o.sprite.try_read(w.sprites)?;
                    (
                        sp.as_deref().is_some_and(|sp| {
                            sp.properties.has(t.flag.WindowN) || sp.properties.has(t.flag.WindowW)
                        }),
                        sp.as_deref().is_some_and(|sp| {
                            sp.properties.has_name(&w.sprites.alias, "doorTrans")
                        }),
                    )
                };
                let (same, opposite) = hooks.barricades(sq, &o);
                special_objects.push(SpecialObject::Thumpable(Thumpable {
                    north: th.north,
                    open: th.open,
                    is_door: th.is_door,
                    is_window,
                    can_pass_through: th.can_pass_through,
                    door_trans,
                    barricade_same: same,
                    barricade_opposite: opposite,
                }));
            }
            ObjectKind::Barricade(b) => {
                special_objects.push(SpecialObject::Barricade(lighting_barricade(b)));
            }
            _ => {}
        }
    }
    special_objects.extend(hooks.extra_special_objects(sq));
    Ok(GridSquare {
        x: sq.x,
        y: sq.y,
        z: sq.z,
        vision_matrix: sq.vision_matrix,
        exterior: sq.is_exterior(w.sprites),
        elevated_floor: sq.has_elevated_floor(w.sprites),
        building_id: building_def_id(w, sq),
        room_id: sq.room_def(w.meta).map(|_| sq.room_id),
        light_level,
        seen_by,
        special_objects,
    })
}

/// A lamp (`IsoLightSwitch.lights` of a switch that is not a room switch).
#[derive(Clone, Copy, Debug)]
struct Lamp {
    source: LightSource,
    /// The switch: `activated`, `streetLight`, position.
    activated: bool,
    street_light: bool,
    switch_pos: (i32, i32, i32),
}

#[derive(Default)]
struct ChunkLights {
    lamps: Vec<Lamp>,
    rooms: BTreeSet<i64>,
}

#[derive(Clone, Debug)]
struct ChunkState {
    min_level: i32,
    max_level: i32,
    /// `IsoChunk.lightCheck[player]` and the levels' `lightCheck[player]`.
    light_check: [bool; 4],
    level_light_check: Vec<[bool; 4]>,
}

/// One player's frame inputs.
#[derive(Clone, Copy, Debug)]
pub struct PlayerFrameInput {
    pub index: i32,
    /// Position, look vector and character values. `effective_fatigue` and
    /// `detection_range` as `updatePlayer` reads them (see
    /// [`default_player`]).
    pub player: PlayerInput,
    /// `calculateVisionCone` inputs; `day_light_strength` and
    /// `luma_inverted` are filled in by [`WorldLighting::frame`].
    pub cone: VisionConeInput,
    /// The player's chunk map: `getWorldXMin()`, `getWorldYMin()` (chunks)
    /// and `chunkGridWidth`.
    pub chunk_x_min: i32,
    pub chunk_y_min: i32,
    pub chunk_grid_width: i32,
}

/// A rested, unimpaired player at `(x, y, z)` looking along `forward`.
pub fn default_player(x: f32, y: f32, z: f32, forward: IsoDirection) -> PlayerInput {
    let (dx, dy) = forward.delta();
    let len = ((dx * dx + dy * dy) as f32).sqrt();
    PlayerInput {
        x,
        y,
        z,
        look_x: dx as f32 / len,
        look_y: dy as f32 / len,
        effective_fatigue: effective_fatigue(0.0),
        detection_range: detection_range(false, false, false, 1.0, 0.0, 0.0),
        forward: Some(forward),
        ..Default::default()
    }
}

/// A light the world adds to the cell itself (`IsoCell.addLamppost`):
/// fires, fireplaces and other burning objects. Its fields are the
/// `IsoLightSource` fields the owner writes directly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellLight {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub radius: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub active: bool,
    /// `localToBuilding.id`.
    pub local_to_building: Option<i32>,
}

impl CellLight {
    /// `new IsoLightSource(x, y, z, r, g, b, radius)`: active, permanent.
    pub fn new(x: i32, y: i32, z: i32, r: f32, g: f32, b: f32, radius: i32) -> Self {
        CellLight {
            x,
            y,
            z,
            radius,
            r,
            g,
            b,
            active: true,
            local_to_building: None,
        }
    }
}

/// A [`CellLight`] in `IsoCell.lamppostPositions`.
#[derive(Clone, Copy, Debug)]
struct Lamppost {
    key: u64,
    light: CellLight,
    /// `life == 0`: `removeLamppost` or a replaced light; `checkLights`
    /// drops it.
    dead: bool,
}

/// What a lighting frame changed.
#[derive(Clone, Debug, Default)]
pub struct LightingFrame {
    /// `LightingJNI.updateCounter[player]`.
    pub update_counter: i32,
    /// Chunk levels whose lighting changed: invalidate their textures.
    pub dirty_levels: Vec<(i32, i32, i32)>,
    /// Squares seen for the first time.
    pub first_seen: Vec<(i32, i32, i32)>,
}

/// The lighting side of the world: the pz-lighting session, its inputs
/// built from pz-world and the decoded results per player.
pub struct WorldLighting {
    pub session: LightingSession,
    pub squares: LightingSquares,
    sync: LightSync,
    cone: VisionCone,
    chunks: HashMap<(i32, i32), ChunkState>,
    lights: HashMap<(i32, i32), ChunkLights>,
    /// Lamp keys of removed chunks, sent once as not alive.
    removed_lamps: Vec<LightSource>,
    /// The cell's own lights, in `lamppostPositions` order.
    lampposts: Vec<Lamppost>,
    room_light_keys: HashMap<(i64, usize), u64>,
    lamp_keys: HashMap<(i32, i32, i32, u32, usize), u64>,
    next_key: u64,
    levels: HashMap<(i32, i32, i32, i32), ChunkLevelLight>,
    /// `GameTime.lightSourceUpdate`.
    light_source_update: f32,
    /// `LosUtil.cachecleared[player]`.
    cache_cleared: [bool; 4],
    /// `IsoCell.chunkMap[player]` in squares, `(min_x, min_y, max_x,
    /// max_y)`, for each local player (`None`: `ignore`).
    chunk_maps: [Option<(i32, i32, i32, i32)>; 4],
    /// `Core.dirtyGlobalLightsCount`.
    pub dirty_global_lights: i32,
    /// `PerformanceSettings.getLockFPS()`.
    pub lock_fps: i32,
    /// `IsoGridSquare.recalcLightTime` (only stored: build 42 reads it
    /// through its getter alone).
    pub recalc_light_time: f32,
    /// `LightingJNI.updateCounter[player]` after the player's last frame.
    update_counters: [i32; 4],
    /// `LightingJNI.buildingsChangedCounter[player]` (-1: none pending).
    buildings_changed_counter: [i32; 4],
}

impl Default for WorldLighting {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldLighting {
    pub fn new() -> Self {
        let mut session = LightingSession::new();
        // `LightingJNI.init`: `configure(0.005f)`.
        session.configure(0.005);
        WorldLighting {
            session,
            squares: LightingSquares::default(),
            sync: LightSync::new(),
            cone: VisionCone::default(),
            chunks: HashMap::new(),
            lights: HashMap::new(),
            removed_lamps: Vec::new(),
            lampposts: Vec::new(),
            room_light_keys: HashMap::new(),
            lamp_keys: HashMap::new(),
            next_key: 1,
            levels: HashMap::new(),
            light_source_update: 100.0,
            cache_cleared: [true; 4],
            chunk_maps: [None; 4],
            dirty_global_lights: 0,
            lock_fps: 60,
            recalc_light_time: 0.0,
            update_counters: [0; 4],
            buildings_changed_counter: [0; 4],
        }
    }

    fn rebuild_squares(
        &mut self,
        w: &WorldView<'_>,
        wx: i32,
        wy: i32,
        hooks: &dyn WorldHooks,
    ) -> HostResult<()> {
        let Some(ch) = w.cell.chunks.get(&(wx, wy)) else {
            return Ok(());
        };
        for sq_owner in ch.squares() {
            let sq = sq_owner
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("lighting chunk square"))?;
            let p = (sq.x, sq.y, sq.z);
            // A square new to lighting starts with the `seen` flags it
            // was loaded with (`IsoGridSquare.load` sets `lighting[i].bSeen`
            // from the saved `vis` byte, and `updateChunk` sends them).
            let (ll, seen) = self
                .squares
                .0
                .get(&p)
                .map_or((0, loaded_seen_by(&sq)), |g| (g.light_level, g.seen_by));
            self.squares
                .0
                .insert(p, grid_square(w, &sq, hooks, ll, seen)?);
        }
        let st = self.chunks.entry((wx, wy)).or_insert(ChunkState {
            min_level: 0,
            max_level: 0,
            light_check: [true; 4],
            level_light_check: Vec::new(),
        });
        st.min_level = ch.min_level;
        st.max_level = ch.max_level;
        st.light_check = [true; 4];
        st.level_light_check = vec![[true; 4]; (ch.max_level - ch.min_level + 1) as usize];
        Ok(())
    }

    /// A chunk was loaded or its squares changed (after
    /// `IsoCell::recalc_chunk_squares`): rebuilds its lighting squares, its
    /// lamps and room lights, refreshes the neighbouring chunks (their edge
    /// matrices changed) and flags them all for a lighting update.
    pub fn sync_chunk(
        &mut self,
        w: &WorldView<'_>,
        wx: i32,
        wy: i32,
        hooks: &dyn WorldHooks,
    ) -> HostResult<()> {
        self.rebuild_squares(w, wx, wy, hooks)?;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if (dx, dy) != (0, 0) && self.chunks.contains_key(&(wx + dx, wy + dy)) {
                    self.rebuild_squares(w, wx + dx, wy + dy, hooks)?;
                }
            }
        }
        self.sync_chunk_lights(w, wx, wy)?;
        Ok(())
    }

    /// Rebuilds the lighting squares of chunk `(wx, wy)` alone and flags
    /// all its levels for every player
    /// (`checkLightingLater_AllPlayers_AllLevels`).
    pub fn sync_chunk_squares(
        &mut self,
        w: &WorldView<'_>,
        wx: i32,
        wy: i32,
        hooks: &dyn WorldHooks,
    ) -> HostResult<()> {
        self.rebuild_squares(w, wx, wy, hooks)
    }

    /// Rebuilds the lighting square at `(x, y, z)` from the world (keeping
    /// its light level and `seen` flags) and flags its level for every
    /// player (`checkLightingLater_AllPlayers_OneLevel`). A square gone
    /// from the world leaves the lighting squares; a chunk whose levels
    /// changed is rebuilt whole.
    pub fn sync_square(
        &mut self,
        w: &WorldView<'_>,
        (x, y, z): (i32, i32, i32),
        hooks: &dyn WorldHooks,
    ) -> HostResult<()> {
        let (wx, wy) = (x.div_euclid(8), y.div_euclid(8));
        let Some(ch) = w.cell.chunks.get(&(wx, wy)) else {
            return Ok(());
        };
        let Some(st) = self.chunks.get(&(wx, wy)) else {
            // Not given to lighting yet: `sync_chunk` sends it whole.
            return Ok(());
        };
        if (st.min_level, st.max_level) != (ch.min_level, ch.max_level) {
            self.rebuild_squares(w, wx, wy, hooks)?;
            return Ok(());
        }
        let p = (x, y, z);
        match w.cell.get_grid_square(x, y, z) {
            Some(sq) => {
                let sq = sq
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("lighting square"))?;
                let (ll, seen) = self
                    .squares
                    .0
                    .get(&p)
                    .map_or((0, loaded_seen_by(&sq)), |g| (g.light_level, g.seen_by));
                self.squares
                    .0
                    .insert(p, grid_square(w, &sq, hooks, ll, seen)?);
            }
            None => {
                self.squares.0.remove(&p);
            }
        }
        self.mark_level(wx, wy, z);
        Ok(())
    }

    /// Rescans chunk `(wx, wy)`'s light switches and room switches into
    /// its lamps and room lights (removed lamps are sent once as not
    /// alive).
    pub fn sync_chunk_lights(&mut self, w: &WorldView<'_>, wx: i32, wy: i32) -> HostResult<()> {
        let Some(ch) = w.cell.chunks.get(&(wx, wy)) else {
            return Ok(());
        };
        let mut cl = ChunkLights::default();
        for sq_owner in ch.squares() {
            let sq = sq_owner
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("light-switch square"))?;
            for o in &sq.objects {
                let o = o
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("light-switch object"))?;
                let ObjectKind::LightSwitch(s) = &o.kind else {
                    continue;
                };
                if s.light_room {
                    // `room.lightSwitches.add(this)` for the switch's room.
                    if sq.room_def(w.meta).is_some() {
                        cl.rooms.insert(sq.room_id);
                    }
                    continue;
                }
                for (i, l) in s.lights.iter().enumerate() {
                    let k = (sq.x, sq.y, sq.z, o.id, i);
                    let key = *self.lamp_keys.entry(k).or_insert_with(|| {
                        self.next_key += 1;
                        self.next_key
                    });
                    cl.lamps.push(Lamp {
                        source: LightSource {
                            key,
                            x: l.x,
                            y: l.y,
                            z: l.z,
                            radius: l.radius,
                            r: l.r,
                            g: l.g,
                            b: l.b,
                            active: l.active,
                            life: -1,
                            local_to_building: None,
                            alive: true,
                        },
                        activated: s.activated,
                        street_light: s.street_light,
                        switch_pos: (sq.x, sq.y, sq.z),
                    });
                }
            }
            // `IsoLightSwitch.chunkLoaded`: rooms with switches
            // (`hasLightSwitches` also counts the def's switch objects).
            if let Some(def) = sq.room_def(w.meta) {
                if def.objects.iter().any(|m| m.kind == META_LIGHT_SWITCH) {
                    cl.rooms.insert(sq.room_id);
                }
            }
        }
        if let Some(old) = self.lights.insert((wx, wy), cl) {
            let new: BTreeSet<u64> = self.lights[&(wx, wy)]
                .lamps
                .iter()
                .map(|l| l.source.key)
                .collect();
            for l in old
                .lamps
                .into_iter()
                .filter(|l| !new.contains(&l.source.key))
            {
                self.removed_lamps.push(LightSource {
                    alive: false,
                    ..l.source
                });
            }
        }
        Ok(())
    }

    /// A chunk was unloaded.
    pub fn remove_chunk(&mut self, wx: i32, wy: i32) {
        self.chunks.remove(&(wx, wy));
        self.squares
            .0
            .retain(|&(x, y, _), _| (x.div_euclid(8), y.div_euclid(8)) != (wx, wy));
        self.levels
            .retain(|&(_, cx, cy, _), _| (cx, cy) != (wx, wy));
        if let Some(cl) = self.lights.remove(&(wx, wy)) {
            self.removed_lamps
                .extend(cl.lamps.into_iter().map(|l| LightSource {
                    alive: false,
                    ..l.source
                }));
        }
    }

    /// `IsoChunk.checkLightingLater_AllPlayers_AllLevels()`.
    pub fn mark_chunk(&mut self, wx: i32, wy: i32) {
        if let Some(st) = self.chunks.get_mut(&(wx, wy)) {
            st.light_check = [true; 4];
            st.level_light_check.fill([true; 4]);
        }
    }

    /// `IsoChunk.checkLightingLater_OnePlayer_AllLevels(player)`.
    pub fn mark_chunk_for_player(&mut self, wx: i32, wy: i32, player: i32) {
        let pi = player as usize & 3;
        if let Some(st) = self.chunks.get_mut(&(wx, wy)) {
            st.light_check[pi] = true;
            for l in &mut st.level_light_check {
                l[pi] = true;
            }
        }
    }

    /// `(IsoChunk.lightCheck[player], IsoChunkLevel.lightCheck[player])`
    /// of chunk `(wx, wy)` level `z`: whether the next frame sends them to
    /// the lighting library. `None` for a chunk or level lighting does not
    /// have.
    pub fn light_check(&self, wx: i32, wy: i32, z: i32, player: i32) -> Option<(bool, bool)> {
        let st = self.chunks.get(&(wx, wy))?;
        let pi = player as usize & 3;
        let l = st
            .level_light_check
            .get(usize::try_from(z - st.min_level).ok()?)?;
        Some((st.light_check[pi], l[pi]))
    }

    /// `LosUtil.cachecleared[player]`.
    pub fn los_cache_is_cleared(&self, player: i32) -> bool {
        self.cache_cleared[player as usize & 3]
    }

    /// `GameTime.lightSourceUpdate` as the next frame sends it.
    pub fn light_source_update_value(&self) -> f32 {
        self.light_source_update
    }

    /// `LosUtil.cachecleared[player] = true`.
    pub fn clear_los_cache(&mut self, player: i32) {
        self.cache_cleared[player as usize & 3] = true;
    }

    /// `GameTime.instance.lightSourceUpdate = 100.0f`.
    pub fn light_source_update(&mut self) {
        self.light_source_update = 100.0;
    }

    /// `LightingJNI.buildingsChanged()` for `num_players` local players:
    /// each player's buildings-changed counter is set two lighting updates
    /// ahead, then `lightSourceUpdate = 100`, every `LosUtil.cachecleared`
    /// and `++Core.dirtyGlobalLightsCount`.
    pub fn buildings_changed(&mut self, num_players: i32) {
        for p in 0..num_players.clamp(0, 4) as usize {
            self.buildings_changed_counter[p] = self.update_counters[p] + 2;
        }
        self.light_source_update = 100.0;
        self.cache_cleared = [true; 4];
        self.dirty_global_lights += 1;
    }

    /// The first half of `LightingJNI.checkChangedBuildings()` (run after
    /// every local player's lighting frame): for each of the `num_players`
    /// players whose counter the lighting updates have reached, the counter
    /// is cleared and, when the player has a chunk map, every loaded chunk
    /// in it is flagged for that player
    /// (`checkLightingLater_OnePlayer_AllLevels`). Returns those players
    /// and chunks, whose render levels and cutaways the caller invalidates.
    pub fn check_changed_buildings(&mut self, num_players: i32) -> Vec<(i32, Vec<(i32, i32)>)> {
        let mut out = Vec::new();
        for p in 0..num_players.clamp(0, 4) as usize {
            let c = self.buildings_changed_counter[p];
            if c == -1 || c > self.update_counters[p] {
                continue;
            }
            self.buildings_changed_counter[p] = -1;
            let Some((min_x, min_y, max_x, max_y)) = self.chunk_maps[p] else {
                continue;
            };
            let mut chunks = Vec::new();
            for wy in min_y / 8..max_y / 8 {
                for wx in min_x / 8..max_x / 8 {
                    if self.chunks.contains_key(&(wx, wy)) {
                        self.mark_chunk_for_player(wx, wy, p as i32);
                        chunks.push((wx, wy));
                    }
                }
            }
            out.push((p as i32, chunks));
        }
        out
    }

    /// Flags a square's level for a lighting update (`IsoGridSquare`
    /// changes call `checkLightingLater_AllPlayers_OneLevel`).
    pub fn mark_level(&mut self, wx: i32, wy: i32, z: i32) {
        if let Some(st) = self.chunks.get_mut(&(wx, wy)) {
            st.light_check = [true; 4];
            if let Some(l) = st.level_light_check.get_mut((z - st.min_level) as usize) {
                *l = [true; 4];
            }
        }
    }

    /// `IsoCell.addLamppost(light)`: appends a light to the cell's list
    /// and flags a lighting update (`recalcLightTime`,
    /// `dirtyGlobalLightsCount`, `lightSourceUpdate`). Returns its key.
    pub fn add_lamppost(&mut self, light: CellLight) -> u64 {
        let key = self.push_lamppost(light);
        self.mark_lights_dirty();
        key
    }

    /// `getLamppostPositions().add(light)`: appends without flagging.
    pub fn push_lamppost(&mut self, light: CellLight) -> u64 {
        self.next_key += 1;
        let key = self.next_key;
        self.lampposts.push(Lamppost {
            key,
            light,
            dead: false,
        });
        key
    }

    /// `IsoCell.removeLamppost(light)`: `life = 0` (the next frame's
    /// `checkLights` removes it) and the update flags.
    pub fn remove_lamppost(&mut self, key: u64) {
        self.kill_lamppost(key);
        self.mark_lights_dirty();
    }

    /// `light.life = 0` alone.
    pub fn kill_lamppost(&mut self, key: u64) {
        if let Some(l) = self.lampposts.iter_mut().find(|l| l.key == key) {
            l.dead = true;
        }
    }

    /// `IsoGridSquare.recalcLightTime = -1`, `++Core.dirtyGlobalLightsCount`
    /// and `GameTime.lightSourceUpdate = 100`.
    pub fn mark_lights_dirty(&mut self) {
        self.recalc_light_time = -1.0;
        self.dirty_global_lights += 1;
        self.light_source_update = 100.0;
    }

    /// The fields of a light still in the cell's list (its owner writes
    /// colour and `active` directly; `checkLights` sends the changes).
    pub fn lamppost_mut(&mut self, key: u64) -> Option<&mut CellLight> {
        self.lampposts
            .iter_mut()
            .find(|l| l.key == key)
            .map(|l| &mut l.light)
    }

    /// The cell's own lights still in its list, in order, and whether each
    /// is dead (`life == 0`, dropped by the next frame).
    pub fn lampposts(&self) -> impl Iterator<Item = (u64, &CellLight, bool)> {
        self.lampposts.iter().map(|l| (l.key, &l.light, l.dead))
    }

    /// The lighting library's id for a light (`IsoLightSource.id`), once
    /// `checkLights` has sent it.
    pub fn light_id(&self, key: u64) -> Option<i32> {
        self.sync.light_id(key)
    }

    /// `IsoCell.getLamppostPositions()` with each lamp's `active` after
    /// `checkLights`' power rules, plus removed lamps once, then the cell's
    /// own lights. A cell light that is dead or outside every local
    /// player's chunk map (`isInBounds`) is sent as not alive and leaves the
    /// list, as `checkLights` removes it.
    fn lamp_list(
        &mut self,
        settings: &dyn RenderSettingsSource,
        hooks: &dyn WorldHooks,
    ) -> Vec<LightSource> {
        let mut keys: Vec<_> = self.lights.keys().copied().collect();
        keys.sort();
        let mut out = std::mem::take(&mut self.removed_lamps);
        for k in keys {
            for l in &self.lights[&k].lamps {
                let (x, y, z) = l.switch_pos;
                let mut has_power = !hooks.light_switch_unpowered(x, y, z);
                if l.street_light && settings.night() < 0.5 {
                    has_power = false;
                }
                // Starts at `activated`; turns off without power, on with
                // power while activated.
                let active = has_power && l.activated;
                out.push(LightSource { active, ..l.source });
            }
        }
        let maps = self.chunk_maps;
        // `IsoLightSource.isInBounds()`: inside any local player's map.
        let in_bounds = |c: &CellLight| {
            maps.iter().flatten().any(|&(min_x, min_y, max_x, max_y)| {
                c.x >= min_x && c.x < max_x && c.y >= min_y && c.y < max_y
            })
        };
        for l in &self.lampposts {
            let c = &l.light;
            let in_bounds = in_bounds(c);
            out.push(LightSource {
                key: l.key,
                x: c.x,
                y: c.y,
                z: c.z,
                radius: c.radius,
                r: c.r,
                g: c.g,
                b: c.b,
                active: c.active,
                life: -1,
                local_to_building: c.local_to_building,
                alive: !l.dead && in_bounds,
            });
        }
        self.lampposts.retain(|l| !l.dead && in_bounds(&l.light));
        out
    }

    /// `IsoCell.roomLights` with `checkLights`' `active` and `isInBounds`.
    fn room_light_list(
        &mut self,
        w: &WorldView<'_>,
        hooks: &dyn WorldHooks,
    ) -> Vec<RoomLightSource> {
        let rooms: BTreeSet<i64> = self
            .lights
            .values()
            .flat_map(|c| c.rooms.iter().copied())
            .collect();
        let mut out = Vec::new();
        for rid in rooms {
            let Some(def) = w.meta.get_room_def_by_id(rid) else {
                continue;
            };
            let Some(b) = def.building else { continue };
            let building_id = w.meta.building_defs[b.0 as usize].id;
            let mut active = def.lights_active.get();
            if hooks.hydro_power_off() {
                let switch_has_power = self
                    .switch_positions(w, rid)
                    .iter()
                    .any(|&(x, y, z)| hooks.room_switch_has_power(x, y, z));
                if !switch_has_power {
                    active = false;
                }
            }
            for (i, r) in def.rects.iter().enumerate() {
                let key = *self.room_light_keys.entry((rid, i)).or_insert_with(|| {
                    self.next_key += 1;
                    self.next_key
                });
                // `IsoRoomLight.isInBounds()`: near any local player's map.
                let in_bounds =
                    self.chunk_maps
                        .iter()
                        .flatten()
                        .any(|&(min_x, min_y, max_x, max_y)| {
                            !(r.x - 5 >= max_x
                                || r.x + r.w + 5 <= min_x
                                || r.y - 5 >= max_y
                                || r.y + r.h + 5 <= min_y)
                        });
                out.push(RoomLightSource {
                    key,
                    building_id,
                    room_id: def.id,
                    x: r.x,
                    y: r.y,
                    z: def.level,
                    width: r.w,
                    height: r.h,
                    active,
                    in_bounds,
                });
            }
        }
        out
    }

    /// The loaded `lightRoom` switches of room `rid`.
    fn switch_positions(&self, w: &WorldView<'_>, rid: i64) -> Vec<(i32, i32, i32)> {
        let mut out = Vec::new();
        for ch in w.cell.chunks.values() {
            for sq_owner in ch.squares() {
                let sq = sq_owner.borrow();
                if sq.room_id != rid {
                    continue;
                }
                if sq
                    .objects
                    .iter()
                    .any(|o| matches!(&o.borrow().kind, ObjectKind::LightSwitch(s) if s.light_room))
                {
                    out.push((sq.x, sq.y, sq.z));
                }
            }
        }
        out
    }

    /// Sets local player `player`'s chunk map, `(getWorldXMin(),
    /// getWorldYMin(), chunkGridWidth)` in chunks, or `None` when the
    /// player has left (`chunkMap[player].ignore`). [`Self::frame`] sets
    /// its own player's; cell lights stay while inside any player's map.
    pub fn set_chunk_map(&mut self, player: i32, map: Option<(i32, i32, i32)>) {
        self.chunk_maps[player as usize & 3] =
            map.map(|(x, y, w)| (x * 8, y * 8, (x + w) * 8, (y + w) * 8));
    }

    /// The decoded light of a square for `player`.
    pub fn square_light(&self, player: i32, x: i32, y: i32, z: i32) -> Option<&SquareLight> {
        let lvl = self
            .levels
            .get(&(player, x.div_euclid(8), y.div_euclid(8), z))?;
        Some(&lvl.squares[(x.rem_euclid(8) + y.rem_euclid(8) * 8) as usize])
    }

    /// `square.lighting[player].getResultLight(0..resultLightCount())`:
    /// the square's result lights (at most five), as `JNILighting`
    /// decoded them from `getSquareLighting` the last time the square was
    /// dirty. Empty for a square without results.
    pub fn result_lights(&self, player: i32, x: i32, y: i32, z: i32) -> &[ResultLightInfo] {
        self.square_light(player, x, y, z)
            .map_or(&[], |l| &l.lights[..])
    }

    /// One frame for one player: `checkLights`, `calculateVisionCone`,
    /// `updatePlayer` (sending flagged chunks), decoding the results the
    /// last lighting update produced, then the lighting thread's update
    /// (`DoLightingUpdateNew`). Results lag one frame behind the inputs.
    /// `on_screen(wx, wy, z)` is `FBORenderLevels.isOnScreen`.
    pub fn frame(
        &mut self,
        w: &WorldView<'_>,
        p: &PlayerFrameInput,
        settings: &mut dyn RenderSettingsSource,
        hooks: &dyn WorldHooks,
        nanos: u64,
        on_screen: &dyn Fn(i32, i32, i32) -> bool,
    ) -> Result<LightingFrame, LightingError> {
        let pi = p.index;
        self.set_chunk_map(pi, Some((p.chunk_x_min, p.chunk_y_min, p.chunk_grid_width)));
        let lamps = self.lamp_list(settings, hooks);
        let rooms = self.room_light_list(w, hooks);
        let torches = hooks.torches();
        let r =
            self.sync
                .check_lights(&mut self.session, &lamps, &rooms, &torches, self.lock_fps)?;
        if r.light_source_update {
            self.light_source_update = 100.0;
        }
        // calculateVisionCone with the player's current square light.
        let light_at = |x: i32, y: i32, z: i32| {
            self.square_light(pi, x, y, z)
                .map(|l| pz_lighting::jni::ColorInfo {
                    r: l.light_info.r,
                    g: l.light_info.g,
                    b: l.light_info.b,
                    a: l.light_info.a,
                })
        };
        let (sx, sy, sz) = (
            p.player.x.floor() as i32,
            p.player.y.floor() as i32,
            p.player.z.floor() as i32,
        );
        let here = light_at(sx, sy, sz);
        let ahead = p.player.forward.and_then(|d| {
            let (dx, dy) = d.delta();
            light_at(sx + dx, sy + dy, sz)
        });
        let mut ci = p.cone;
        ci.day_light_strength = settings.day_light_strength();
        ci.luma_inverted = luma_inverted(here, ahead);
        let cone = self.cone.calculate(&ci);
        let render = settings.player_settings(pi);
        let mut infos = Vec::new();
        for cy in p.chunk_y_min..p.chunk_y_min + p.chunk_grid_width {
            for cx in p.chunk_x_min..p.chunk_x_min + p.chunk_grid_width {
                let Some(st) = self.chunks.get(&(cx, cy)) else {
                    continue;
                };
                let ui = pi as usize & 3;
                infos.push(ChunkInfo {
                    wx: cx,
                    wy: cy,
                    min_level: st.min_level,
                    max_level: st.max_level,
                    loaded: true,
                    light_check: st.light_check[ui],
                    level_light_check: st.level_light_check.iter().map(|l| l[ui]).collect(),
                });
            }
        }
        let frame = PlayerFrame {
            index: pi,
            chunk_x_min: p.chunk_x_min,
            chunk_y_min: p.chunk_y_min,
            chunk_grid_width: p.chunk_grid_width,
            player: Some(p.player),
            cone,
            render,
            cache_cleared: self.cache_cleared[pi as usize & 3],
            chunks: &infos,
        };
        let params = FrameParams {
            view_dist_max: settings.view_dist_max(),
            light_source_update: self.light_source_update,
            sky_light_level: render.sky_light_level(),
        };
        let res = update_player(&mut self.session, &self.squares, &frame, &params)?;
        self.cache_cleared[pi as usize & 3] = false;
        self.light_source_update = 0.0;
        self.update_counters[pi as usize & 3] = res.update_counter;
        for c in &res.chunks_sent {
            if let Some(st) = self.chunks.get_mut(c) {
                st.light_check[pi as usize & 3] = false;
            }
        }
        for &(cx, cy, z) in &res.levels_sent {
            if let Some(st) = self.chunks.get_mut(&(cx, cy)) {
                if let Some(l) = st.level_light_check.get_mut((z - st.min_level) as usize) {
                    l[pi as usize & 3] = false;
                }
            }
        }
        // Decode.
        let mut out = LightingFrame {
            update_counter: res.update_counter,
            ..Default::default()
        };
        let dp = DecodePlayer {
            x: p.player.x,
            y: p.player.y,
            z: p.player.z,
            forward: p.player.forward,
        };
        for info in &infos {
            for z in info.min_level..=info.max_level {
                let lvl = self
                    .levels
                    .entry((pi, info.wx, info.wy, z))
                    .or_insert_with(|| ChunkLevelLight::new(info.wx, info.wy, z));
                let squares = &self.squares.0;
                let pos = |i: usize| {
                    (
                        info.wx * 8 + (i % 8) as i32,
                        info.wy * 8 + (i / 8) as i32,
                        z,
                    )
                };
                let rr = lvl.refresh(
                    &mut self.session,
                    pi,
                    res.update_counter,
                    Some(&dp),
                    on_screen(info.wx, info.wy, z),
                    |i| squares.contains_key(&pos(i)),
                )?;
                if rr.invalidate {
                    out.dirty_levels.push((info.wx, info.wy, z));
                }
                out.first_seen.extend(rr.first_seen.iter().map(|&i| pos(i)));
                for (i, sl) in lvl.squares.iter().enumerate() {
                    if let Some(g) = self.squares.0.get_mut(&pos(i)) {
                        g.light_level = sl.light_level;
                        if sl.seen() {
                            g.seen_by |= 1 << (pi & 7);
                        }
                    }
                }
            }
        }
        self.session
            .do_lighting_update(nanos, self.dirty_global_lights > 0);
        if self.dirty_global_lights > 3 {
            self.dirty_global_lights = 2;
        }
        if self.dirty_global_lights > 0 {
            self.dirty_global_lights -= 1;
        }
        Ok(out)
    }

    /// Writes `player`'s decoded lighting into the render squares:
    /// `light_info`, `vert_lights`, `light_level` (`getLightLevel`: the
    /// brightest channel) and the cutaway visibility fields.
    pub fn apply(&self, player: i32, render: &mut RenderWorld) {
        for (&(x, y, z), rs) in render.squares.iter_mut() {
            let Some(l) = self.square_light(player, x, y, z) else {
                continue;
            };
            if !self.squares.0.contains_key(&(x, y, z)) {
                continue;
            }
            let c = l.light_info;
            rs.light_info = RenderColor {
                r: c.r,
                g: c.g,
                b: c.b,
                a: c.a,
            };
            rs.vert_lights = l.vert_light;
            rs.light_level = c.r.max(c.g).max(c.b);
            rs.cut.seen = l.seen();
            rs.cut.can_see = l.can_see();
            rs.cut.could_see = l.could_see();
            rs.cut.dark_multi = l.dark_multi;
        }
    }
}
