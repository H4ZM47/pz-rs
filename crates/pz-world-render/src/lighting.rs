//! pz-world squares, lamps and room lights as pz-lighting inputs, and the
//! per-frame lighting driver (`LightingJNI.update()` for a player, the
//! lighting thread's update, and decoding the results).

use std::collections::{BTreeSet, HashMap};

use pz_iso_render::color::ColorInfo as RenderColor;
use pz_lighting::jni::{
    detection_range, effective_fatigue, luma_inverted, update_player, ChunkInfo, ChunkLevelLight,
    Curtain, CurtainType, DecodePlayer, Door, FrameParams, GridSquare, IsoDirection, LightSource,
    LightSync, LightingWorld, PlayerFrame, PlayerInput, RoomLightSource, SpecialObject,
    SquareLight, VisionCone, VisionConeInput, Window,
};
use pz_lighting::{LightingError, LightingSession};
use pz_world::{IsoGridSquare, ObjectKind};

use crate::hooks::WorldHooks;
use crate::render_world::{RenderWorld, WorldView};
use crate::settings::RenderSettingsSource;

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
) -> GridSquare {
    let t = &w.sprites.tables;
    let mut special_objects = Vec::new();
    for &id in &sq.special_objects {
        let Some(i) = sq.object_index(id) else {
            continue;
        };
        let o = &sq.objects[i];
        let sp = o.sprite.map(|s| &w.sprites.sprites[s]);
        let prop = |n: &str| {
            sp.and_then(|sp| sp.properties.get(&w.sprites.alias, n))
                .map(str::to_string)
        };
        let has_name = |n: &str| sp.is_some_and(|sp| sp.properties.has_name(&w.sprites.alias, n));
        match &o.kind {
            ObjectKind::Door(d) => {
                let (same, opposite) = hooks.barricades(sq, o);
                let curtain = hooks.door_curtain(sq, o);
                special_objects.push(SpecialObject::Door(Door {
                    north: d.north,
                    open: d.open,
                    door_trans: has_name("doorTrans"),
                    garage_door: has_name("GarageDoor"),
                    has_curtain: curtain.is_some(),
                    curtain_open: curtain.unwrap_or(false),
                    barricade_same: same,
                    barricade_opposite: opposite,
                }));
            }
            ObjectKind::Window(win) => {
                let (same, opposite) = hooks.barricades(sq, o);
                special_objects.push(SpecialObject::Window(Window {
                    north: win.north,
                    open: win.open,
                    barricade_same: same,
                    barricade_opposite: opposite,
                }));
            }
            ObjectKind::Curtain(c) => {
                let kind = match sp.map(|sp| sp.tile_type) {
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
            _ => {}
        }
    }
    special_objects.extend(hooks.extra_special_objects(sq));
    GridSquare {
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
    }
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
    room_light_keys: HashMap<(i64, usize), u64>,
    lamp_keys: HashMap<(i32, i32, i32, u32, usize), u64>,
    next_key: u64,
    levels: HashMap<(i32, i32, i32, i32), ChunkLevelLight>,
    /// `GameTime.lightSourceUpdate`.
    light_source_update: f32,
    /// `LosUtil.cachecleared[player]`.
    cache_cleared: [bool; 4],
    /// `Core.dirtyGlobalLightsCount`.
    pub dirty_global_lights: i32,
    /// `PerformanceSettings.getLockFPS()`.
    pub lock_fps: i32,
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
            room_light_keys: HashMap::new(),
            lamp_keys: HashMap::new(),
            next_key: 1,
            levels: HashMap::new(),
            light_source_update: 100.0,
            cache_cleared: [true; 4],
            dirty_global_lights: 0,
            lock_fps: 60,
        }
    }

    fn rebuild_squares(&mut self, w: &WorldView<'_>, wx: i32, wy: i32, hooks: &dyn WorldHooks) {
        let Some(ch) = w.cell.chunks.get(&(wx, wy)) else {
            return;
        };
        for sq in ch.squares() {
            let p = (sq.x, sq.y, sq.z);
            let (ll, seen) = self
                .squares
                .0
                .get(&p)
                .map_or((0, 0), |g| (g.light_level, g.seen_by));
            self.squares
                .0
                .insert(p, grid_square(w, sq, hooks, ll, seen));
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
    }

    /// A chunk was loaded or its squares changed (after
    /// `IsoCell::recalc_chunk_squares`): rebuilds its lighting squares, its
    /// lamps and room lights, refreshes the neighbouring chunks (their edge
    /// matrices changed) and flags them all for a lighting update.
    pub fn sync_chunk(&mut self, w: &WorldView<'_>, wx: i32, wy: i32, hooks: &dyn WorldHooks) {
        self.rebuild_squares(w, wx, wy, hooks);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if (dx, dy) != (0, 0) && self.chunks.contains_key(&(wx + dx, wy + dy)) {
                    self.rebuild_squares(w, wx + dx, wy + dy, hooks);
                }
            }
        }
        let Some(ch) = w.cell.chunks.get(&(wx, wy)) else {
            return;
        };
        let mut cl = ChunkLights::default();
        for sq in ch.squares() {
            for o in &sq.objects {
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

    /// `IsoCell.getLamppostPositions()` with each lamp's `active` after
    /// `checkLights`' power rules, plus removed lamps once.
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
        out
    }

    /// `IsoCell.roomLights` with `checkLights`' `active` and `isInBounds`.
    fn room_light_list(
        &mut self,
        w: &WorldView<'_>,
        hooks: &dyn WorldHooks,
        bounds: (i32, i32, i32, i32),
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
                let (min_x, min_y, max_x, max_y) = bounds;
                let in_bounds = !(r.x - 5 >= max_x
                    || r.x + r.w + 5 <= min_x
                    || r.y - 5 >= max_y
                    || r.y + r.h + 5 <= min_y);
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
            for sq in ch.squares().filter(|s| s.room_id == rid) {
                if sq
                    .objects
                    .iter()
                    .any(|o| matches!(&o.kind, ObjectKind::LightSwitch(s) if s.light_room))
                {
                    out.push((sq.x, sq.y, sq.z));
                }
            }
        }
        out
    }

    /// The decoded light of a square for `player`.
    pub fn square_light(&self, player: i32, x: i32, y: i32, z: i32) -> Option<&SquareLight> {
        let lvl = self
            .levels
            .get(&(player, x.div_euclid(8), y.div_euclid(8), z))?;
        Some(&lvl.squares[(x.rem_euclid(8) + y.rem_euclid(8) * 8) as usize])
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
        let bounds = (
            p.chunk_x_min * 8,
            p.chunk_y_min * 8,
            (p.chunk_x_min + p.chunk_grid_width) * 8,
            (p.chunk_y_min + p.chunk_grid_width) * 8,
        );
        let lamps = self.lamp_list(settings, hooks);
        let rooms = self.room_light_list(w, hooks, bounds);
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
