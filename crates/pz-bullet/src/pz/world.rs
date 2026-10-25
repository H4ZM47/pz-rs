#![allow(clippy::needless_range_loop)] // 8x8 square loops follow the C++
//! `WorldSimulation`: the game's physics world (PZBullet.cpp /
//! WorldSimulation.cpp), reconstructed from the shipped library.

use std::collections::{BTreeMap, HashMap};

use super::chunk::{Chunk, ChunkId, ChunkLevel, ChunkMap};
use super::script::VehicleScript;
use super::vehicle::PzVehicle;
use crate::api::JavaCallbacks;
use crate::bt::collision_object::*;
use crate::bt::dispatch::ContactAddedHook;
use crate::bt::dynamics::rigid_body::RigidBodyConstructionInfo;
use crate::bt::dynamics::world::{DiscreteDynamicsWorld, WorldHooks};
use crate::bt::linear_math::*;
use crate::bt::manifold::ManifoldPoint;
use crate::bt::shapes::Shape;

/// `BulletObjectType`, kept in the low byte of `CollisionObject::user_index`
/// (the vehicle id is in the bits above): the `BulletObject` the game
/// attaches as user pointer.
pub mod bullet_object {
    pub const VEHICLE: i64 = 1;
    pub const WALL_N: i64 = 3;
    pub const WALL_W: i64 = 4;
    pub const WALL_S: i64 = 5;
    pub const WALL_E: i64 = 6;
    pub const SOLID: i64 = 7;
    pub const FLOOR: i64 = 8;
    pub const TREE: i64 = 9;
    pub const MESH: i64 = 10;
    pub const PHYSICS_OBJECT: i64 = 11;
}

/// One level is `3 * 0.8164967f` physics units high.
pub const LEVEL_SCALE: f64 = 0.816_496_670_246_124_3;
/// `2.44949` as the binary uses it (`level = (int)(y / 2.449490010738373)`).
pub const LEVEL_HEIGHT: f64 = 2.449_490_010_738_373;

/// Event counters (diagnostics for coverage checks; not part of the original).
pub mod stats {
    use std::sync::atomic::{AtomicUsize, Ordering};
    pub static STAIRS: AtomicUsize = AtomicUsize::new(0);
    pub static STAIRS_CONTACTS: AtomicUsize = AtomicUsize::new(0);
    pub static MESH_BODIES: AtomicUsize = AtomicUsize::new(0);
    pub static VEHICLE_CONTACTS: AtomicUsize = AtomicUsize::new(0);
    pub static STATIC_CONTACTS: AtomicUsize = AtomicUsize::new(0);
    pub static SCROLLS: AtomicUsize = AtomicUsize::new(0);
    pub static LEVEL_LOADS: AtomicUsize = AtomicUsize::new(0);
    pub fn bump(c: &AtomicUsize) {
        c.fetch_add(1, Ordering::Relaxed);
    }
    pub fn report() -> String {
        let g = |c: &AtomicUsize| c.load(Ordering::Relaxed);
        format!(
            "stairs={} stairsContacts={} meshBodies={} vehicleVehicleContacts={} vehicleStaticContacts={} scrolls={} levelLoads={}",
            g(&STAIRS), g(&STAIRS_CONTACTS), g(&MESH_BODIES), g(&VEHICLE_CONTACTS), g(&STATIC_CONTACTS), g(&SCROLLS), g(&LEVEL_LOADS)
        )
    }
}

/// `WorldSimulation::Constraint` (an entry of the constraint map).
#[derive(Clone, Debug)]
pub struct ConstraintEntry {
    pub vehicle_a: i32,
    pub vehicle_b: i32,
    pub constraint: ConstraintId,
}

/// `ServerCell`: a 5×5-chunk (40×40 tile) area kept by a server.
#[derive(Clone, Debug)]
pub struct ServerCell {
    pub cx: i32,
    pub cy: i32,
    /// `chunks[x * 5 + y]` at chunk `(cx*5 + x, cy*5 + y)`.
    pub chunks: Vec<ChunkId>,
    /// The cell's ground box.
    pub ground: ObjId,
}

/// `WorldSimulation`.
pub struct WorldSimulation {
    pub is_server: bool,
    pub min_cell_x: i32,
    pub min_cell_y: i32,
    pub max_cell_x: i32,
    pub max_cell_y: i32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub dw: DiscreteDynamicsWorld,
    /// `+0x48`: chunk maps of players 0..3.
    pub chunk_maps: [Option<ChunkMap>; 4],
    pub chunks: Vec<Option<Chunk>>,
    free_chunks: Vec<ChunkId>,
    /// `+0x98`: vehicles by id.
    pub vehicles: BTreeMap<i32, PzVehicle>,
    /// `+0x198`: physics objects by id.
    pub physics_objects: BTreeMap<i32, ObjId>,
    /// `+0x190`: last physics object id.
    pub physics_object_counter: i32,
    /// `+0x1b8`: constraints by id, `+0x1b0` the last id.
    pub constraints: BTreeMap<i32, ConstraintEntry>,
    pub constraint_counter: i32,
    /// `+0x1d0`..: pooled wall bodies per direction (N, W, S, E) and length.
    pub wall_pools: [[Vec<ObjId>; 8]; 4],
    /// `+0x7d0`: pooled solid bodies (also used for solid stairs).
    pub solid_pool: Vec<ObjId>,
    /// `+0x800`: pooled tree bodies.
    pub tree_pool: Vec<ObjId>,
    /// `+0x830`: pooled floor bodies by size key `w | h << 16`.
    pub floor_pools: BTreeMap<u32, Vec<ObjId>>,
    /// `+0x848`: pooled stairs collision objects.
    pub stairs_pool: Vec<ObjId>,
    /// `+0x878`: floor shapes by size key.
    pub floor_shapes: BTreeMap<u32, ShapeId>,
    /// `+0x890`: floor body -> size key.
    pub floor_body_key: HashMap<ObjId, u32>,
    pub ground_shape: Option<ShapeId>,
    pub solid_shape: Option<ShapeId>,
    pub tree_shape: Option<ShapeId>,
    /// `+0x8c0`..`+0x9c0`: wall shapes per direction and length.
    pub wall_shapes: [[Option<ShapeId>; 8]; 4],
    pub physics_object_shape: Option<ShapeId>,
    /// `+0x9d8`: physics meshes by index.
    pub meshes: Vec<Option<ShapeId>>,
    /// `+0x9f0`.
    pub stairs_shape: Option<ShapeId>,
    /// `+0x9f8`: server cells.
    pub server_cells: Vec<ServerCell>,
    /// `+0xa10`: stair boxes per part.
    pub stairs_count: [i32; 2],
    /// `+0xa18`: full extents of the two stair box kinds.
    pub stairs_extents: [Vec3; 2],
    /// Value of the stairs child transforms' origin x, which the original
    /// leaves uninitialised (a stack slot); see `get_stairs_shape`.
    pub stairs_origin_x: f64,
}

/// The world's callbacks into the glue for one step: `tickCallback`
/// (installed as the pre-tick callback), `collisionCallback`
/// (`gContactAddedCallback`) and `PZDiscreteDynamicsWorld::saveKinematicState`.
pub struct GlueHooks<'a> {
    pub vehicles: &'a mut BTreeMap<i32, PzVehicle>,
}

impl ContactAddedHook for GlueHooks<'_> {
    fn contact_added_enabled(&self) -> bool {
        true
    }
    /// `collisionCallback`.
    fn contact_added(
        &mut self,
        cp: &mut ManifoldPoint,
        _obj0: ObjId,
        co0: &CollisionObject,
        _part_id0: i32,
        _index0: i32,
        _obj1: ObjId,
        co1: &CollisionObject,
        _part_id1: i32,
        _index1: i32,
    ) -> bool {
        let u0 = co0.user_index;
        let u1 = co1.user_index;
        if !co0.is_rigid_body() || !co1.is_rigid_body() {
            stats::bump(&stats::STAIRS_CONTACTS);
        } else if (u0 & 0xff) == bullet_object::VEHICLE && (u1 & 0xff) == bullet_object::VEHICLE {
            stats::bump(&stats::VEHICLE_CONTACTS);
        } else {
            stats::bump(&stats::STATIC_CONTACTS);
        }
        if u0 == 0 || u1 == 0 || (u0 & 0xff) != bullet_object::VEHICLE {
            return false;
        }
        let id0 = (u0 >> 8) as i32;
        if (u1 & 0xff) == bullet_object::VEHICLE {
            if let Some(v) = self.vehicles.get_mut(&id0) {
                v.is_collide = true;
            }
            let id1 = (u1 >> 8) as i32;
            if let Some(v) = self.vehicles.get_mut(&id1) {
                v.is_collide = true;
            }
        } else if let Some(v) = self.vehicles.get_mut(&id0) {
            // (the other BulletObject records the vehicle id; not observable)
            v.is_collide = true;
        }
        let f = co0.friction * flt(0.9);
        let mut d5 = -10.0;
        if -10.0 <= f {
            d5 = f;
        }
        let mut d4 = 10.0;
        if d5 <= 10.0 {
            d4 = d5;
        }
        cp.combined_friction = d4;
        cp.combined_restitution = co0.restitution * 0.1;
        true
    }
}

impl WorldHooks for GlueHooks<'_> {
    /// `tickCallback`: limits each vehicle's change of linear (3.5) and
    /// angular (3.0) velocity per substep and its top speed.
    fn internal_pre_tick(&mut self, world: &mut DiscreteDynamicsWorld, _time_step: f64) {
        for v in self.vehicles.values_mut() {
            let o = world.cw.obj_mut(v.body);
            let rb = o.body.as_deref_mut().expect("vehicle body");
            let mut lin = rb.linear_velocity;
            let mut ang = rb.angular_velocity;
            let mut lin_w = lin.w;
            let mut rev = 0;
            if v.is_static {
                v.last_linear_velocity = lin;
            } else {
                let l = lin.length();
                let d = l - v.last_linear_velocity.length();
                if l > flt(0.1) && d > 3.5 {
                    let last = v.last_linear_velocity;
                    let dv = lin - last;
                    let k = 3.5 / dv.length();
                    let s = dv * k;
                    lin = Vec3::new(last.x + s.x, last.y + s.y, last.z + s.z);
                    rb.linear_velocity = lin;
                    rev += 1;
                    v.last_linear_velocity = lin;
                    lin_w = 0.0;
                } else {
                    v.last_linear_velocity = lin;
                }
                let a = ang.length();
                let d = a - v.last_angular_velocity.length();
                if a > flt(0.1) && d > 3.0 {
                    let last = v.last_angular_velocity;
                    let da = ang - last;
                    let k = 3.0 / da.length();
                    let s = da * k;
                    ang = Vec3::new(last.x + s.x, last.y + s.y, last.z + s.z);
                    rb.angular_velocity = ang;
                    rev += 1;
                }
            }
            v.last_angular_velocity = ang;
            if v.max_speed < lin.length() {
                let m = v.velocity_multiplier;
                rb.linear_velocity = Vec3::new4(lin.x * m, lin.y * m, lin.z * m, lin_w);
                rev += 1;
            }
            o.update_revision += rev;
        }
    }
    /// `PZDiscreteDynamicsWorld::saveKinematicState`: only static vehicles.
    fn save_kinematic_state(&mut self, world: &mut DiscreteDynamicsWorld, time_step: f64) -> bool {
        for v in self.vehicles.values() {
            if v.is_static {
                world.cw.obj_mut(v.body).save_kinematic_state(time_step);
            }
        }
        true
    }
}

fn static_body_info(shape: ShapeId, origin: Vec3) -> RigidBodyConstructionInfo {
    let mut info = RigidBodyConstructionInfo::new(0.0, None, Some(shape), Vec3::ZERO);
    info.start_world_transform = Transform::new(Mat3::IDENTITY, origin);
    info
}

/// `(double)((float)level * 3.0f) * 0.8164966702461243`.
#[inline]
fn level_y(level: i32) -> f64 {
    ((level as f32) * 3.0f32) as f64 * LEVEL_SCALE
}

impl WorldSimulation {
    /// `WorldSimulation(minCellX, minCellY, maxCellX, maxCellY, offsetX,
    /// offsetY, isServer)`.
    pub fn new(
        min_cell_x: i32,
        min_cell_y: i32,
        max_cell_x: i32,
        max_cell_y: i32,
        offset_x: i32,
        offset_y: i32,
        is_server: bool,
    ) -> WorldSimulation {
        let mut dw = DiscreteDynamicsWorld::new();
        dw.set_gravity(&Vec3::new(0.0, -10.0, 0.0));
        dw.cw.dispatch_info.allowed_ccd_penetration = 0.0;
        dw.debug_mode = Some(0x801);
        WorldSimulation {
            is_server,
            min_cell_x,
            min_cell_y,
            max_cell_x,
            max_cell_y,
            offset_x,
            offset_y,
            dw,
            chunk_maps: [None, None, None, None],
            chunks: Vec::new(),
            free_chunks: Vec::new(),
            vehicles: BTreeMap::new(),
            physics_objects: BTreeMap::new(),
            physics_object_counter: 0,
            constraints: BTreeMap::new(),
            constraint_counter: 0,
            wall_pools: Default::default(),
            solid_pool: Vec::new(),
            tree_pool: Vec::new(),
            floor_pools: BTreeMap::new(),
            stairs_pool: Vec::new(),
            floor_shapes: BTreeMap::new(),
            floor_body_key: HashMap::new(),
            ground_shape: None,
            solid_shape: None,
            tree_shape: None,
            wall_shapes: [[None; 8]; 4],
            physics_object_shape: None,
            meshes: Vec::new(),
            stairs_shape: None,
            server_cells: Vec::new(),
            stairs_count: [12, 12],
            stairs_extents: [
                Vec3::new(1.0, flt(0.05), flt(0.2)),
                Vec3::new(1.0, flt(0.2), flt(0.05)),
            ],
            stairs_origin_x: 0.0,
        }
    }

    // ------------------------------------------------------------------
    // chunks
    // ------------------------------------------------------------------

    pub fn chunk(&self, id: ChunkId) -> &Chunk {
        self.chunks[id].as_ref().expect("dangling chunk")
    }
    pub fn chunk_mut(&mut self, id: ChunkId) -> &mut Chunk {
        self.chunks[id].as_mut().expect("dangling chunk")
    }
    fn new_chunk(&mut self, wx: i32, wy: i32) -> ChunkId {
        let c = Chunk::new(wx, wy);
        if let Some(id) = self.free_chunks.pop() {
            self.chunks[id] = Some(c);
            id
        } else {
            self.chunks.push(Some(c));
            self.chunks.len() - 1
        }
    }
    /// `delete chunk`: removes the bodies of every level.
    fn delete_chunk(&mut self, id: ChunkId) {
        let (min, max) = {
            let c = self.chunk(id);
            (c.min_level, c.max_level)
        };
        if min <= max {
            for l in min..=max {
                self.level_remove_bodies_and_clear_shapes(id, l);
            }
        }
        self.chunks[id] = None;
        self.free_chunks.push(id);
    }

    /// `isValidChunk(wx, wy)`.
    pub fn is_valid_chunk(&self, wx: i32, wy: i32) -> bool {
        self.min_cell_x * 32 <= wx
            && self.min_cell_y * 32 <= wy
            && wx < self.max_cell_x * 32 + 32
            && wy < self.max_cell_y * 32 + 32
    }
    pub fn is_valid_chunk_x(&self, wx: i32) -> bool {
        self.min_cell_x * 32 <= wx && wx < self.max_cell_x * 32 + 32
    }
    pub fn is_valid_chunk_y(&self, wy: i32) -> bool {
        self.min_cell_y * 32 <= wy && wy < self.max_cell_y * 32 + 32
    }
    fn adjacent_n(&self, c: &Chunk) -> bool {
        c.wy + 1 == self.min_cell_y * 32
    }
    fn adjacent_s(&self, c: &Chunk) -> bool {
        c.wy == self.max_cell_y * 32 + 32
    }
    fn adjacent_w(&self, c: &Chunk) -> bool {
        c.wx + 1 == self.min_cell_x * 32
    }
    fn adjacent_e(&self, c: &Chunk) -> bool {
        c.wx == self.max_cell_x * 32 + 32
    }

    /// `getChunk(playerIndex, wx, wy)`.
    pub fn get_chunk(&self, player_index: i32, wx: i32, wy: i32) -> Option<ChunkId> {
        if self.is_server {
            for sc in &self.server_cells {
                if let Some(c) = Self::server_cell_chunk(sc, wx, wy) {
                    return Some(c);
                }
            }
            return None;
        }
        self.chunk_maps[player_index as usize]
            .as_ref()
            .and_then(|m| m.chunk_at(wx, wy))
    }

    fn server_cell_chunk(sc: &ServerCell, wx: i32, wy: i32) -> Option<ChunkId> {
        let x = wx - sc.cx * 5;
        let y = wy - sc.cy * 5;
        if (0..5).contains(&x) && (0..5).contains(&y) {
            Some(sc.chunks[(x * 5 + y) as usize])
        } else {
            None
        }
    }

    /// `getChunkForAnyPlayer(wx, wy)`.
    pub fn get_chunk_for_any_player(&self, wx: i32, wy: i32) -> Option<ChunkId> {
        if self.is_server {
            return self.get_chunk(0, wx, wy);
        }
        for m in self.chunk_maps.iter().flatten() {
            if m.contains(wx, wy) {
                return m.chunk_at(wx, wy);
            }
        }
        None
    }

    /// `Chunk::setMinMaxLevel(min, max)`.
    pub fn chunk_set_min_max_level(&mut self, id: ChunkId, min: i32, max: i32) {
        let (old_min, old_max) = {
            let c = self.chunk(id);
            (c.min_level, c.max_level)
        };
        if old_min == min && old_max == max {
            return;
        }
        if old_min <= old_max {
            for l in old_min..=old_max {
                if l < min || max < l {
                    self.level_remove_bodies_and_clear_shapes(id, l);
                }
            }
        }
        let c = self.chunk_mut(id);
        let mut old: Vec<Option<ChunkLevel>> = std::mem::take(&mut c.levels)
            .into_iter()
            .map(Some)
            .collect();
        let mut levels = Vec::new();
        if min <= max {
            for l in min..=max {
                if old_min <= l && l <= old_max {
                    levels.push(old[(l - old_min) as usize].take().expect("level"));
                } else {
                    levels.push(ChunkLevel::new(l));
                }
            }
        }
        c.levels = levels;
        c.min_level = min;
        c.max_level = max;
    }

    /// `ChunkLevel::removePhysicsBodiesAndClearShapes`.
    fn level_remove_bodies_and_clear_shapes(&mut self, id: ChunkId, level: i32) {
        let has = self
            .chunk(id)
            .level(level)
            .map(|l| l.has_bodies)
            .unwrap_or(false);
        if has {
            self.chunk_mut(id).level_mut(level).unwrap().has_bodies = false;
            self.level_remove_bodies(id, level);
        }
        if let Some(l) = self.chunk_mut(id).level_mut(level) {
            l.dirty = false;
            l.shapes = [[[0; 4]; 8]; 8];
        }
    }

    /// `ChunkLevel::removePhysicsBodies()`.
    pub fn level_remove_bodies(&mut self, id: ChunkId, level: i32) {
        for y in 0..8 {
            for x in 0..8 {
                self.level_remove_square_bodies(id, level, x, y);
            }
        }
    }

    /// `ChunkLevel::removePhysicsBodies(x, y)`.
    fn level_remove_square_bodies(&mut self, id: ChunkId, level: i32, x: usize, y: usize) {
        for s in 0..4 {
            let (body, code) = {
                let l = self.chunk(id).level(level).unwrap();
                (l.bodies[x][y][s], l.shapes[x][y][s])
            };
            if let Some(b) = body {
                if code == 1 || code == 10 {
                    self.remove_solid(b);
                } else if code == 6 {
                    self.remove_tree(b);
                } else if code > 10 {
                    self.remove_mesh(b);
                }
                self.chunk_mut(id).level_mut(level).unwrap().bodies[x][y][s] = None;
            }
        }
        let (floor, walls, stairs) = {
            let l = self.chunk(id).level(level).unwrap();
            (
                l.floor[x][y],
                [
                    l.walls[0][x][y],
                    l.walls[1][x][y],
                    l.walls[2][x][y],
                    l.walls[3][x][y],
                ],
                l.stairs[x][y],
            )
        };
        if let Some(b) = floor {
            self.remove_floor(b);
            self.chunk_mut(id).level_mut(level).unwrap().floor[x][y] = None;
        }
        // The original removes east walls through removeWallS.
        for (d, w) in walls.iter().enumerate() {
            if let Some(b) = *w {
                let pool_dir = if d == 3 { 2 } else { d };
                self.remove_wall(pool_dir, b);
                self.chunk_mut(id).level_mut(level).unwrap().walls[d][x][y] = None;
            }
        }
        if let Some(b) = stairs {
            self.remove_stairs(b);
            self.chunk_mut(id).level_mut(level).unwrap().stairs[x][y] = None;
        }
    }

    /// `Chunk::load`.
    fn chunk_load(&mut self, id: ChunkId, cb: &mut dyn JavaCallbacks) {
        let edge = {
            let c = self.chunk(id);
            self.adjacent_n(c) || self.adjacent_s(c) || self.adjacent_w(c) || self.adjacent_e(c)
        };
        if edge {
            self.chunk_set_min_max_level(id, 0, 63);
        }
        let (min, max) = {
            let c = self.chunk(id);
            (c.min_level, c.max_level)
        };
        if min <= max {
            for l in min..=max {
                self.level_load(id, l, cb);
            }
        }
    }

    /// `ChunkLevel::load`.
    fn level_load(&mut self, id: ChunkId, level: i32, cb: &mut dyn JavaCallbacks) {
        let need = self.chunk(id).level(level).unwrap().need;
        if need {
            let (wx, wy) = {
                let c = self.chunk(id);
                (c.wx, c.wy)
            };
            let mut bufs = Vec::new();
            cb.update_physics_for_level_if_needed(wx, wy, level, &mut bufs);
            for b in bufs {
                self.to_bullet(&b);
            }
            // The callback may have changed this chunk's levels.
            let l = match self.chunk(id).level(level) {
                Some(l) => l,
                None => return,
            };
            if l.has_bodies {
                self.chunk_mut(id).level_mut(level).unwrap().need = false;
                return;
            }
            self.chunk_mut(id).level_mut(level).unwrap().has_bodies = true;
            stats::bump(&stats::LEVEL_LOADS);
            self.level_add_bodies(id, level);
            self.chunk_mut(id).level_mut(level).unwrap().need = false;
            return;
        }
        if self.chunk(id).level(level).unwrap().has_bodies {
            self.chunk_mut(id).level_mut(level).unwrap().has_bodies = false;
            self.level_remove_bodies(id, level);
        }
        self.chunk_mut(id).level_mut(level).unwrap().need = false;
    }

    /// `ChunkLevel::addPhysicsBodies`.
    fn level_add_bodies(&mut self, id: ChunkId, level: i32) {
        let (wx, wy) = {
            let c = self.chunk(id);
            (c.wx, c.wy)
        };
        for y in 0..8 {
            for x in 0..8 {
                let gx = x as i32 + wx * 8;
                let gy = y as i32 + wy * 8;
                for s in 0..4 {
                    let (has, code) = {
                        let l = self.chunk(id).level(level).unwrap();
                        (l.bodies[x][y][s].is_some(), l.shapes[x][y][s])
                    };
                    if has {
                        continue;
                    }
                    let body = match code {
                        10 => Some(self.add_solid(gx, gy, level, 6)),
                        6 => Some(self.add_tree(gx, gy, level)),
                        1 => Some(self.add_solid(gx, gy, level, 14)),
                        c if c >= 11 => Some(self.add_mesh(gx, gy, level, (c - 11) as i32)),
                        _ => None,
                    };
                    if body.is_some() {
                        self.chunk_mut(id).level_mut(level).unwrap().bodies[x][y][s] = body;
                    }
                }
            }
        }
        self.update_floors(id, level);
        self.update_walls(id, level, 0);
        self.update_walls(id, level, 1);
        self.update_walls(id, level, 2);
        self.update_walls(id, level, 3);
        self.update_stairs(id, level);
    }

    /// Run-length merge of one line of 8 cells: the first cell of each run
    /// gets the run length, the rest of the run is cleared.
    fn merge_line(c: &mut [u8; 8]) {
        let mut i = 0usize;
        loop {
            if c[i] != 0 {
                let len = if i < 7 {
                    let mut n = 1usize;
                    while i + n < 8 && c[i + n] != 0 {
                        c[i + n] = 0;
                        n += 1;
                    }
                    n
                } else {
                    1
                };
                c[i] = len as u8;
                i = i + len - 1;
            }
            if i >= 7 {
                break;
            }
            i += 1;
        }
    }

    /// `ChunkLevel::updateFloors`: merges floor squares into rectangles
    /// (runs along x, then equal runs along y) and keeps one box per
    /// rectangle.
    fn update_floors(&mut self, id: ChunkId, level: i32) {
        let (wx, wy) = {
            let c = self.chunk(id);
            (c.wx, c.wy)
        };
        // g[x][y] = (width, height)
        let mut g = [[[0u8; 2]; 8]; 8];
        {
            let l = self.chunk(id).level(level).unwrap();
            for x in 0..8 {
                for y in 0..8 {
                    if l.shapes[x][y].contains(&7) {
                        g[x][y] = [1, 1];
                    }
                }
            }
        }
        if !self.is_valid_chunk(wx, wy) {
            g = [[[1, 1]; 8]; 8];
        }
        for y in 0..8 {
            let mut line = [0u8; 8];
            for x in 0..8 {
                line[x] = g[x][y][0];
            }
            Self::merge_line(&mut line);
            for x in 0..8 {
                if line[x] == 0 {
                    g[x][y] = [0, 0];
                } else {
                    g[x][y][0] = line[x];
                }
            }
        }
        // equal runs along y merge into taller rectangles (from y = 7 down)
        for y in (1..8).rev() {
            for x in 0..8 {
                let a = g[x][y][0];
                if a != 0 && a == g[x][y - 1][0] {
                    g[x][y - 1][1] = g[x][y - 1][1].wrapping_add(g[x][y][1]);
                    g[x][y] = [0, 0];
                }
            }
        }
        for y in 0..8 {
            for x in 0..8 {
                let cur = self.chunk(id).level(level).unwrap().floor[x][y];
                let cur_shape = cur.map(|b| self.dw.cw.obj(b).shape());
                let (w, h) = (g[x][y][0] as i32, g[x][y][1] as i32);
                let want = self.get_floor_shape(w, h);
                if cur_shape != want {
                    if let Some(b) = cur {
                        self.remove_floor(b);
                        self.chunk_mut(id).level_mut(level).unwrap().floor[x][y] = None;
                    }
                    if want.is_some() {
                        let b = self.add_floor(wx * 8 + x as i32, wy * 8 + y as i32, level, w, h);
                        self.chunk_mut(id).level_mut(level).unwrap().floor[x][y] = Some(b);
                    }
                }
            }
        }
    }

    /// `ChunkLevel::updateWallsN/W/S/E` (`dir` 0..3).
    fn update_walls(&mut self, id: ChunkId, level: i32, dir: usize) {
        let (wx, wy) = {
            let c = self.chunk(id);
            (c.wx, c.wy)
        };
        let code = 2 + dir as u32;
        let mut g = [[0u8; 8]; 8];
        {
            let l = self.chunk(id).level(level).unwrap();
            for x in 0..8 {
                for y in 0..8 {
                    g[x][y] = l.shapes[x][y].contains(&code) as u8;
                }
            }
        }
        let c = self.chunk(id).clone_header();
        match dir {
            0 => {
                if self.is_valid_chunk_x(wx) && self.adjacent_s(&c) {
                    for col in g.iter_mut() {
                        col[0] = 1;
                    }
                }
            }
            1 => {
                if self.adjacent_e(&c) && self.is_valid_chunk_y(wy) {
                    g[0] = [1; 8];
                }
            }
            2 => {
                if self.is_valid_chunk_x(wx) && self.adjacent_n(&c) {
                    for col in g.iter_mut() {
                        col[7] = 1;
                    }
                }
            }
            _ => {
                if self.adjacent_w(&c) && self.is_valid_chunk_y(wy) {
                    g[7] = [1; 8];
                }
            }
        }
        if dir == 0 || dir == 2 {
            // runs along x for each y
            for y in 0..8 {
                let mut line = [0u8; 8];
                for x in 0..8 {
                    line[x] = g[x][y];
                }
                Self::merge_line(&mut line);
                for x in 0..8 {
                    g[x][y] = line[x];
                }
            }
        } else {
            for col in g.iter_mut() {
                Self::merge_line(col);
            }
        }
        for y in 0..8 {
            for x in 0..8 {
                let cur = self.chunk(id).level(level).unwrap().walls[dir][x][y];
                let cur_shape = cur.map(|b| self.dw.cw.obj(b).shape());
                let len = g[x][y] as i32;
                let want = self.get_wall_shape(dir, len);
                if cur_shape != want {
                    if let Some(b) = cur {
                        self.remove_wall(dir, b);
                        self.chunk_mut(id).level_mut(level).unwrap().walls[dir][x][y] = None;
                    }
                    if want.is_some() {
                        let b =
                            self.add_wall(dir, wx * 8 + x as i32, wy * 8 + y as i32, level, len);
                        self.chunk_mut(id).level_mut(level).unwrap().walls[dir][x][y] = Some(b);
                    }
                }
            }
        }
    }

    /// `ChunkLevel::updateStairs`. A square with several stair codes gets
    /// one object per code; only the last is remembered (the others stay
    /// in the world for good, as in the original).
    fn update_stairs(&mut self, id: ChunkId, level: i32) {
        let (wx, wy) = {
            let c = self.chunk(id);
            (c.wx, c.wy)
        };
        for y in 0..8 {
            for x in 0..8 {
                let (cur, s) = {
                    let l = self.chunk(id).level(level).unwrap();
                    (l.stairs[x][y], l.shapes[x][y])
                };
                let gx = wx * 8 + x as i32;
                let gy = wy * 8 + y as i32;
                if let Some(cur) = cur {
                    let keep = s.iter().any(|&c| c == 8 || c == 9);
                    if !keep {
                        self.remove_stairs(cur);
                        self.chunk_mut(id).level_mut(level).unwrap().stairs[x][y] = None;
                    }
                } else {
                    let mut last = None;
                    for &code in &s {
                        if code & !1 == 8 {
                            last = Some(self.add_stairs(gx, level, gy, code));
                        }
                    }
                    if last.is_some() {
                        self.chunk_mut(id).level_mut(level).unwrap().stairs[x][y] = last;
                    }
                }
            }
        }
    }

    /// `ToBullet(buffer)`: chunk shape updates (`TO_UPDATE_CHUNK` records,
    /// little-endian).
    pub fn to_bullet(&mut self, buf: &[u8]) {
        let rd = |p: usize| -> u8 { buf.get(p).copied().unwrap_or(0xff) };
        let rs = |p: usize| -> i32 { (((rd(p + 1) as u16) << 8) | rd(p) as u16) as i16 as i32 };
        let mut p = 0usize;
        loop {
            let op = rd(p);
            if op != 8 {
                p += 1;
                if op == 0xff {
                    return;
                }
                continue;
            }
            let (cwx, cwy, min, max, level) =
                (rs(p + 1), rs(p + 3), rs(p + 5), rs(p + 7), rs(p + 9));
            let chunk = self.get_chunk_for_any_player(cwx, cwy);
            p += 11;
            if let Some(c) = chunk {
                self.chunk_set_min_max_level(c, min, max);
                let has = self
                    .chunk(c)
                    .level(level)
                    .map(|l| l.has_bodies)
                    .unwrap_or(false);
                if has {
                    self.chunk_mut(c).level_mut(level).unwrap().has_bodies = false;
                    self.level_remove_bodies(c, level);
                }
            }
            loop {
                let x = rd(p);
                p += 1;
                if x == 0xff {
                    break;
                }
                let y = rd(p);
                let n = rd(p + 1) as usize;
                p += 2;
                let shapes: Vec<u32> = (0..n).map(|i| rd(p + i) as u32).collect();
                p += n;
                if let Some(c) = chunk {
                    // Out-of-range levels write through a null level in the
                    // original (a crash); ignored here.
                    if let Some(l) = self.chunk_mut(c).level_mut(level) {
                        if (x as usize) < 8 && (y as usize) < 8 {
                            l.set_shapes(x as usize, y as usize, &shapes);
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // chunk maps
    // ------------------------------------------------------------------

    /// `activateChunkMap(playerIndex, wx, wy, width)`.
    pub fn activate_chunk_map(&mut self, i: i32, wx: i32, wy: i32, width: i32) {
        let mut map = ChunkMap::new(i, wx, wy, width);
        let others: Vec<usize> = (0..4).filter(|&j| j != i as usize).collect();
        for j in others {
            let Some(other) = self.chunk_maps[j].as_ref() else {
                continue;
            };
            let ow = other.width;
            let mut adopt = Vec::new();
            for y in 0..ow {
                for x in 0..ow {
                    if let Some(c) = other.chunk_at(wx + x, wy + y) {
                        adopt.push(c);
                    }
                }
            }
            for c in adopt {
                let (cx, cy) = {
                    let ch = self.chunk(c);
                    (ch.wx - wx, ch.wy - wy)
                };
                map.set(cx, cy, Some(c));
                self.chunk_mut(c).referenced[i as usize] = true;
            }
        }
        for y in 0..width {
            for x in 0..width {
                if map.get(x, y).is_none() {
                    let c = self.new_chunk(wx + x, wy + y);
                    self.chunk_mut(c).referenced[i as usize] = true;
                    map.set(x, y, Some(c));
                }
            }
        }
        // An older map in this slot is leaked by the original: its chunks
        // stay referenced.
        self.chunk_maps[i as usize] = Some(map);
    }

    /// `deactivateChunkMap(playerIndex)`: deletes the map; chunks no other
    /// map references are deleted with their bodies.
    pub fn deactivate_chunk_map(&mut self, i: i32) {
        if let Some(map) = self.chunk_maps[i as usize].take() {
            let w = map.width;
            for y in 0..w {
                for x in 0..w {
                    let c = map.get(x, y);
                    self.unref_chunk(c, map.player_index as usize);
                }
            }
        }
    }

    fn unref_chunk(&mut self, c: Option<ChunkId>, i: usize) {
        if let Some(c) = c {
            self.chunk_mut(c).referenced[i] = false;
            if !self.chunk(c).is_referenced() {
                self.delete_chunk(c);
            }
        }
    }

    /// `ChunkMap::scrollLeft/Right/Up/Down` (`dir` 0..3).
    pub fn scroll_chunk_map(&mut self, i: i32, dir: i32) {
        stats::bump(&stats::SCROLLS);
        let iu = i as usize;
        let mut map = self.chunk_maps[iu].take().expect("chunk map");
        let w = map.width;
        for k in 0..w {
            let (old, nx, ny) = match dir {
                0 => {
                    let old = map.get(w - 1, k);
                    for x in (1..w).rev() {
                        let v = map.get(x - 1, k);
                        map.set(x, k, v);
                    }
                    map.set(0, k, None);
                    (old, map.wx - 1, map.wy + k)
                }
                1 => {
                    let old = map.get(0, k);
                    for x in 0..w - 1 {
                        let v = map.get(x + 1, k);
                        map.set(x, k, v);
                    }
                    map.set(w - 1, k, None);
                    (old, map.wx + w, map.wy + k)
                }
                2 => {
                    let old = map.get(k, w - 1);
                    for y in (1..w).rev() {
                        let v = map.get(k, y - 1);
                        map.set(k, y, v);
                    }
                    map.set(k, 0, None);
                    (old, map.wx + k, map.wy - 1)
                }
                _ => {
                    let old = map.get(k, 0);
                    for y in 0..w - 1 {
                        let v = map.get(k, y + 1);
                        map.set(k, y, v);
                    }
                    map.set(k, w - 1, None);
                    (old, map.wx + k, map.wy + w)
                }
            };
            self.chunk_maps[iu] = Some(map);
            self.unref_chunk(old, iu);
            let found = self.get_chunk_for_any_player(nx, ny);
            map = self.chunk_maps[iu].take().unwrap();
            let c = match found {
                Some(c) => c,
                None => self.new_chunk(nx, ny),
            };
            self.chunk_mut(c).referenced[iu] = true;
            match dir {
                0 => map.set(0, k, Some(c)),
                1 => map.set(w - 1, k, Some(c)),
                2 => map.set(k, 0, Some(c)),
                _ => map.set(k, w - 1, Some(c)),
            }
        }
        match dir {
            0 => map.wx -= 1,
            1 => map.wx += 1,
            2 => map.wy -= 1,
            _ => map.wy += 1,
        }
        self.chunk_maps[iu] = Some(map);
    }

    /// `ChunkMap::setNeed(x, y, level)` (tile coordinates).
    fn chunk_map_set_need(&mut self, i: usize, x: i32, y: i32, level: i32) {
        let (mwx, mwy, w) = {
            let m = self.chunk_maps[i].as_ref().unwrap();
            (m.wx, m.wy, m.width)
        };
        let cx = ((x as f32) / 8.0).floor() as i32 - mwx;
        let cy = ((y as f32) / 8.0).floor() as i32 - mwy;
        for gx in cx - 1..=cx + 1 {
            if gx < 0 {
                continue;
            }
            for gy in cy - 1..=cy + 1 {
                if gy < w && gy >= 0 && gx < w {
                    let c = self.chunk_maps[i]
                        .as_ref()
                        .unwrap()
                        .get(gx, gy)
                        .expect("chunk");
                    for z in level - 1..=level + 1 {
                        self.chunk_mut(c).set_need(z);
                    }
                }
            }
        }
    }

    /// `WorldSimulation::stepSimulation`.
    pub fn step_simulation(
        &mut self,
        time_step: f64,
        max_sub_steps: i32,
        fixed_time_step: f64,
        cb: &mut dyn JavaCallbacks,
    ) {
        for i in 0..4 {
            if self.chunk_maps[i].is_none() {
                continue;
            }
            let ids: Vec<i32> = self.vehicles.keys().copied().collect();
            for id in ids {
                let body = self.vehicles[&id].body;
                let o = self.dw.cw.obj(body).world_transform.origin;
                let x = (o.x + self.offset_x as f64).floor() as i32;
                let y = (o.z + self.offset_y as f64).floor() as i32;
                let z = (o.y / LEVEL_HEIGHT).floor() as i32;
                self.chunk_map_set_need(i, x, y, z);
            }
            // ChunkMap::load
            let w = self.chunk_maps[i].as_ref().unwrap().width;
            for y in 0..w {
                for x in 0..w {
                    let c = self.chunk_maps[i]
                        .as_ref()
                        .unwrap()
                        .get(x, y)
                        .expect("chunk");
                    self.chunk_load(c, cb);
                }
            }
        }
        let mut hooks = GlueHooks {
            vehicles: &mut self.vehicles,
        };
        self.dw
            .step_simulation(time_step, max_sub_steps, fixed_time_step, &mut hooks);
        self.check_vehicle_constraints(cb);
    }

    /// `checkVehicleConstraints`.
    fn check_vehicle_constraints(&mut self, cb: &mut dyn JavaCallbacks) {
        let keys: Vec<i32> = self.constraints.keys().copied().collect();
        for k in keys {
            let Some(e) = self.constraints.get(&k) else {
                continue;
            };
            let c = self.dw.constraints.get(e.constraint);
            if c.needs_feedback && c.applied_impulse > 500.0 {
                let imp = c.applied_impulse as f32;
                cb.on_vehicle_constraint_impulse(k, e.vehicle_a, e.vehicle_b, imp);
            }
        }
    }

    // ------------------------------------------------------------------
    // static bodies
    // ------------------------------------------------------------------

    fn add_static(&mut self, body: ObjId, kind: i64, group: i16, mask: i16) -> ObjId {
        self.dw.cw.obj_mut(body).user_index = kind;
        self.dw.add_rigid_body_filtered(body, group, mask);
        body
    }

    /// Takes a pooled body (resetting its transform) or makes a new one.
    fn pooled_body(&mut self, pool: Option<ObjId>, shape: Option<ShapeId>, origin: Vec3) -> ObjId {
        match pool {
            Some(b) => {
                let o = self.dw.cw.obj_mut(b);
                o.update_revision += 1;
                o.world_transform = Transform::new(Mat3::IDENTITY, origin);
                b
            }
            None => {
                let o = CollisionObject::new_rigid_body(&static_body_info(
                    shape.expect("shape"),
                    origin,
                ));
                self.dw.cw.insert_object(o)
            }
        }
    }

    /// `getWallNShape/W/S/E(length)`.
    pub fn get_wall_shape(&mut self, dir: usize, len: i32) -> Option<ShapeId> {
        if !(1..=8).contains(&len) {
            return None;
        }
        if let Some(s) = self.wall_shapes[dir][(len - 1) as usize] {
            return Some(s);
        }
        let half = len as u32 as f64 / 2.0;
        let he = if dir == 0 || dir == 2 {
            Vec3::new(half, 1.224_745_005_369_186_4, flt(0.05))
        } else {
            Vec3::new(flt(0.05), 1.224_745_005_369_186_4, half)
        };
        let s = self.dw.cw.shapes.add(Shape::new_box(&he));
        self.wall_shapes[dir][(len - 1) as usize] = Some(s);
        Some(s)
    }

    /// `createWallBody(x, y, level, length, dir)`.
    fn create_wall_body(&mut self, x: i32, y: i32, level: i32, len: i32, dir: usize) -> ObjId {
        let ix = x - self.offset_x;
        let iy = y - self.offset_y;
        let half = len as f64 / 2.0;
        let (ex, ez) = if dir == 1 || dir == 3 {
            (flt(0.05), half)
        } else {
            (half, flt(0.05))
        };
        let yv = level_y(level);
        let origin = match dir {
            0 | 1 => Vec3::new(ex + ix as f64, yv + 1.224_745_005_369_186_4, ez + iy as f64),
            2 => Vec3::new(
                ex + ix as f64,
                fma(
                    ((level as f32) * 3.0f32) as f64,
                    LEVEL_SCALE,
                    1.224_745_005_369_186_4,
                ),
                (iy + 1) as f64 - ez,
            ),
            _ => Vec3::new(
                (ix + 1) as f64 - ex,
                fma(
                    ((level as f32) * 3.0f32) as f64,
                    LEVEL_SCALE,
                    1.224_745_005_369_186_4,
                ),
                ez + iy as f64,
            ),
        };
        let pool = self.wall_pools[dir][(len - 1) as usize].pop();
        let shape = if pool.is_none() {
            self.get_wall_shape(dir, len)
        } else {
            None
        };
        self.pooled_body(pool, shape, origin)
    }

    /// `addWallN/W/S/E`.
    fn add_wall(&mut self, dir: usize, x: i32, y: i32, level: i32, len: i32) -> ObjId {
        let b = self.create_wall_body(x, y, level, len, dir);
        self.add_static(b, bullet_object::WALL_N + dir as i64, 1, 14)
    }

    /// `removeWallN/W/S/E`: back into the pool of the length whose shape
    /// the body has (the last pool when none matches).
    fn remove_wall(&mut self, dir: usize, b: ObjId) {
        self.dw.cw.obj_mut(b).user_index = 0;
        self.dw.remove_rigid_body(b);
        let shape = self.dw.cw.obj(b).collision_shape;
        let mut idx = 7;
        for (i, s) in self.wall_shapes[dir].iter().enumerate() {
            if *s == shape {
                idx = i;
                break;
            }
        }
        self.wall_pools[dir][idx].push(b);
    }

    /// `createSolid(x, y, level)`.
    fn create_solid(&mut self, x: i32, y: i32, level: i32) -> ObjId {
        if self.solid_shape.is_none() {
            self.solid_shape = Some(
                self.dw
                    .cw
                    .shapes
                    .add(Shape::new_box(&Vec3::new(0.5, 1.0, 0.5))),
            );
        }
        let origin = Vec3::new(
            (x - self.offset_x) as f64 + 0.5,
            level_y(level) + 1.0,
            (y - self.offset_y) as f64 + 0.5,
        );
        let pool = self.solid_pool.pop();
        let shape = self.solid_shape;
        self.pooled_body(pool, shape, origin)
    }

    /// `addSolid` (mask 14) and `addSolidStairs` (mask 6).
    fn add_solid(&mut self, x: i32, y: i32, level: i32, mask: i16) -> ObjId {
        let b = self.create_solid(x, y, level);
        self.add_static(b, bullet_object::SOLID, 1, mask)
    }

    fn remove_solid(&mut self, b: ObjId) {
        self.dw.cw.obj_mut(b).user_index = 0;
        self.dw.remove_rigid_body(b);
        self.solid_pool.push(b);
    }

    /// `createTreeBody` / `addTree`.
    fn add_tree(&mut self, x: i32, y: i32, level: i32) -> ObjId {
        if self.tree_shape.is_none() {
            self.tree_shape = Some(self.dw.cw.shapes.add(Shape::new_box(&Vec3::new(
                flt(0.1),
                1.0,
                flt(0.1),
            ))));
        }
        let ox = ((x - self.offset_x) as f32 + 0.5f32) as f64 + flt(0.1);
        let oz = ((y - self.offset_y) as f32 + 0.5f32) as f64 + flt(0.1);
        let origin = Vec3::new(ox, level_y(level) + 1.0, oz);
        let pool = self.tree_pool.pop();
        let shape = self.tree_shape;
        let b = self.pooled_body(pool, shape, origin);
        self.add_static(b, bullet_object::TREE, 1, 14)
    }

    fn remove_tree(&mut self, b: ObjId) {
        self.dw.cw.obj_mut(b).user_index = 0;
        self.dw.remove_rigid_body(b);
        self.tree_pool.push(b);
    }

    /// `createMeshBody` / `addMesh` (a new body every time).
    fn add_mesh(&mut self, x: i32, y: i32, level: i32, mesh: i32) -> ObjId {
        stats::bump(&stats::MESH_BODIES);
        let shape = self.meshes.get(mesh as usize).copied().flatten();
        let ox = ((x as f32 - self.offset_x as f32) + 0.5f32) as f64;
        let oz = ((y as f32 - self.offset_y as f32) + 0.5f32) as f64;
        let origin = Vec3::new(ox, (level * 3) as f64 * LEVEL_SCALE, oz);
        let mut info = RigidBodyConstructionInfo::new(0.0, None, shape, Vec3::ZERO);
        info.start_world_transform = Transform::new(Mat3::IDENTITY, origin);
        let o = CollisionObject::new_rigid_body(&info);
        let b = self.dw.cw.insert_object(o);
        self.add_static(b, bullet_object::MESH, 1, 14)
    }

    /// `removeMesh`: the body is deleted.
    fn remove_mesh(&mut self, b: ObjId) {
        self.dw.cw.obj_mut(b).user_index = 0;
        self.dw.remove_rigid_body(b);
        self.dw.cw.delete_object(b);
    }

    /// `getFloorShape(w, h)`: one cached box per size (created on first
    /// use, even when only compared against).
    pub fn get_floor_shape(&mut self, w: i32, h: i32) -> Option<ShapeId> {
        if !(1..=8).contains(&w) || !(1..=8).contains(&h) {
            return None;
        }
        let key = (w as u32) | ((h as u32) << 16);
        if let Some(&s) = self.floor_shapes.get(&key) {
            return Some(s);
        }
        let he = Vec3::new(
            (w as u32 as f64 + 0.02) * 0.5,
            f64::from_bits(0x3fa9_9999_a000_0000),
            (h as u32 as f64 + 0.02) * 0.5,
        );
        let s = self.dw.cw.shapes.add(Shape::new_box(&he));
        self.floor_shapes.insert(key, s);
        Some(s)
    }

    /// `createFloorBody` / `addFloor`.
    fn add_floor(&mut self, x: i32, y: i32, level: i32, w: i32, h: i32) -> ObjId {
        let key = (w as u32) | ((h as u32) << 16);
        let origin = Vec3::new(
            w as f64 / 2.0 + (x - self.offset_x) as f64,
            (level * 3) as f64 * LEVEL_SCALE + -flt(0.05),
            h as f64 / 2.0 + (y - self.offset_y) as f64,
        );
        let pool = self.floor_pools.entry(key).or_default().pop();
        let b = match pool {
            Some(_) => self.pooled_body(pool, None, origin),
            None => {
                let shape = self.get_floor_shape(w, h);
                let b = self.pooled_body(None, shape, origin);
                self.floor_body_key.insert(b, key);
                b
            }
        };
        self.add_static(b, bullet_object::FLOOR, 1, 14)
    }

    fn remove_floor(&mut self, b: ObjId) {
        self.dw.cw.obj_mut(b).user_index = 0;
        self.dw.remove_rigid_body(b);
        match self.floor_body_key.get(&b) {
            Some(&key) => self.floor_pools.entry(key).or_default().push(b),
            None => {
                self.dw.cw.delete_object(b);
            }
        }
    }

    /// `getStairsShape()`: a compound of thin boxes, `stairs_count[0]` of
    /// the first kind and `stairs_count[1]` of the second, climbing one
    /// level over 12 steps.
    fn get_stairs_shape(&mut self) -> ShapeId {
        if let Some(s) = self.stairs_shape {
            return s;
        }
        let n = self.stairs_count[0];
        let compound = self.dw.cw.shapes.add(Shape::new_compound(true));
        self.stairs_shape = Some(compound);
        let e0 = self.stairs_extents[0];
        let e1 = self.stairs_extents[1];
        let b0 = self.dw.cw.shapes.add(Shape::new_box(&Vec3::new(
            e0.x * 0.5,
            e0.y * 0.5,
            e0.z * 0.5,
        )));
        let b1 = self.dw.cw.shapes.add(Shape::new_box(&Vec3::new(
            e1.x * 0.5,
            e1.y * 0.5,
            e1.z * 0.5,
        )));
        let step = LEVEL_HEIGHT / n as f64;
        let mut t = Transform::IDENTITY;
        t.origin = Vec3::new(self.stairs_origin_x, step * 0.5, 1.75);
        for _ in 0..self.stairs_count[0] {
            self.dw.cw.shapes.compound_add_child_shape(compound, &t, b0);
            t.origin = Vec3::new4(t.origin.x, t.origin.y + step, t.origin.z + -0.25, 0.0);
        }
        // The second run starts from the same height the first ended at.
        t.basis = Mat3::IDENTITY;
        t.origin = Vec3::new4(t.origin.x, 0.0, 1.875, 0.0);
        for _ in 0..self.stairs_count[1] {
            self.dw.cw.shapes.compound_add_child_shape(compound, &t, b1);
            t.origin = Vec3::new4(t.origin.x, t.origin.y + step, t.origin.z + -0.25, 0.0);
        }
        compound
    }

    /// `createStairsCollisionObject` / `addStairs(x, level, y, shape)`.
    fn add_stairs(&mut self, x: i32, level: i32, y: i32, code: u32) -> ObjId {
        stats::bump(&stats::STAIRS);
        let mut e = self.stairs_extents[0];
        let b = match self.stairs_pool.pop() {
            Some(b) => b,
            None => {
                let shape = self.get_stairs_shape();
                let mut o = CollisionObject::new();
                o.set_collision_shape(shape);
                o.activate(true);
                self.dw.cw.insert_object(o)
            }
        };
        let mut a = 0.0;
        if code == 9 {
            e = Vec3::new(
                self.stairs_extents[0].z,
                self.stairs_extents[0].y,
                self.stairs_extents[0].x,
            );
            a = f64::from_bits(0x3fe9_21fb_5444_2d18);
        }
        let (s, c) = bt_sincos(a);
        let z3 = 0.0f64.copysign(s);
        let q = Quat::new(
            fma(c, 0.0, z3),
            s + 0.0f64.copysign(-c),
            fma(c, 0.0, 0.0f64.copysign(-s)),
            c + z3,
        );
        let mut basis = Mat3::IDENTITY;
        basis.set_rotation(&q);
        let origin = Vec3::new(
            e.x * 0.5 + (x - self.offset_x) as f64,
            level_y(level) + e.y * 0.5,
            e.z * 0.5 + (y - self.offset_y) as f64,
        );
        let o = self.dw.cw.obj_mut(b);
        o.update_revision += 1;
        o.world_transform = Transform::new(basis, origin);
        self.dw.add_collision_object(b, 1, 13);
        b
    }

    fn remove_stairs(&mut self, b: ObjId) {
        self.dw.remove_collision_object(b);
        self.stairs_pool.push(b);
    }

    // ------------------------------------------------------------------
    // meshes
    // ------------------------------------------------------------------

    /// `clearPhysicsMeshes()`.
    pub fn clear_physics_meshes(&mut self) {
        for s in self.meshes.drain(..).flatten() {
            self.dw.cw.shapes.remove(s);
        }
    }

    /// `definePhysicsMesh(index, compound, numPoints, points)`.
    pub fn define_physics_mesh(
        &mut self,
        index: i32,
        compound: bool,
        num_points: i32,
        points: &[f32],
    ) {
        let i = index as usize;
        if self.meshes.len() <= i {
            self.meshes.resize(i + 1, None);
        }
        let mut hull = Shape::new_convex_hull(&[], 0, 3);
        if num_points > 0 {
            let n = num_points as usize;
            for k in 0..n {
                let p = Vec3::new(
                    points[k * 3] as f64,
                    points[k * 3 + 1] as f64,
                    points[k * 3 + 2] as f64,
                );
                if let crate::bt::shapes::ShapeKind::ConvexHull(h) = &mut hull.kind {
                    h.add_point(&p, k == n - 1);
                }
            }
        }
        let hull_id = self.dw.cw.shapes.add(hull);
        if compound {
            let c = match self.meshes[i] {
                Some(c) => c,
                None => {
                    let c = self.dw.cw.shapes.add(Shape::new_compound(true));
                    self.meshes[i] = Some(c);
                    c
                }
            };
            self.dw
                .cw
                .shapes
                .compound_add_child_shape(c, &Transform::IDENTITY, hull_id);
        } else {
            self.meshes[i] = Some(hull_id);
        }
    }

    // ------------------------------------------------------------------
    // vehicles
    // ------------------------------------------------------------------

    /// `WorldSimulation::addVehicle(id, transform, script)`.
    pub fn add_vehicle(
        &mut self,
        id: i32,
        t: &Transform,
        script_index: usize,
        script: &VehicleScript,
    ) {
        let burnt = script.num_wheels == 0;
        let v = PzVehicle::new(self, id, burnt, script_index, script);
        let v = v.add_to_world(self, t);
        self.vehicles.insert(id, v);
    }

    /// `WorldSimulation::removeVehicle(id)`.
    pub fn remove_vehicle(&mut self, id: i32) {
        if !self.vehicles.contains_key(&id) {
            return;
        }
        self.remove_constraints_on_vehicle(id);
        let v = self.vehicles.remove(&id).unwrap();
        v.remove_from_world(self);
    }

    /// `removeConstraintsOnVehicle`. (The original keeps using the erased
    /// map node and crashes when a removed constraint is not the map's
    /// last; here the loop is safe.)
    fn remove_constraints_on_vehicle(&mut self, id: i32) {
        let keys: Vec<i32> = self
            .constraints
            .iter()
            .filter(|(_, e)| e.vehicle_a == id || e.vehicle_b == id)
            .map(|(k, _)| *k)
            .collect();
        for k in keys {
            let e = self.constraints.remove(&k).unwrap();
            self.dw.remove_constraint(e.constraint);
            self.dw.constraints.remove(e.constraint);
        }
    }

    /// `WorldSimulation::addConstraint`.
    pub fn add_constraint(
        &mut self,
        vehicle_a: i32,
        vehicle_b: i32,
        c: crate::bt::dynamics::typed_constraint::TypedConstraint,
    ) -> i32 {
        let cid = self.dw.add_constraint(c, false);
        self.constraint_counter += 1;
        let id = self.constraint_counter;
        self.constraints.insert(
            id,
            ConstraintEntry {
                vehicle_a,
                vehicle_b,
                constraint: cid,
            },
        );
        id
    }

    /// `WorldSimulation::removeConstraint(id)`.
    pub fn remove_constraint(&mut self, id: i32) {
        if let Some(e) = self.constraints.remove(&id) {
            self.dw.remove_constraint(e.constraint);
            self.dw.constraints.remove(e.constraint);
        }
    }

    /// `WorldSimulation::setConstraintERP(id, erp, axis)`:
    /// `setParam(BT_CONSTRAINT_STOP_ERP, erp, axis)`.
    pub fn set_constraint_erp(&mut self, id: i32, erp: f32, axis: i32) {
        if let Some(e) = self.constraints.get(&id) {
            self.dw
                .constraints
                .get_mut(e.constraint)
                .set_param(2, erp as f64, axis);
        }
    }
}

trait CloneHeader {
    fn clone_header(&self) -> Chunk;
}
impl CloneHeader for Chunk {
    fn clone_header(&self) -> Chunk {
        Chunk {
            wx: self.wx,
            wy: self.wy,
            min_level: self.min_level,
            max_level: self.max_level,
            levels: Vec::new(),
            referenced: self.referenced,
        }
    }
}

impl WorldSimulation {
    /// `createServerCell(cx, cy)`: 5×5 chunks plus a ground box.
    pub fn create_server_cell(&mut self, cx: i32, cy: i32) {
        if self.server_cells.iter().any(|c| c.cx == cx && c.cy == cy) {
            return;
        }
        if self.ground_shape.is_none() {
            self.ground_shape = Some(
                self.dw
                    .cw
                    .shapes
                    .add(Shape::new_box(&Vec3::new(30.0, 1.0, 30.0))),
            );
        }
        let origin = Vec3::new(
            ((cx * 40 - self.offset_x) + 20) as f64,
            -1.0,
            ((cy * 40 - self.offset_y) + 20) as f64,
        );
        let body =
            CollisionObject::new_rigid_body(&static_body_info(self.ground_shape.unwrap(), origin));
        let body = self.dw.cw.insert_object(body);
        let mut chunks = vec![0; 25];
        for y in 0..5 {
            for x in 0..5 {
                chunks[x * 5 + y] = self.new_chunk(cx * 5 + x as i32, cy * 5 + y as i32);
            }
        }
        self.server_cells.push(ServerCell {
            cx,
            cy,
            chunks,
            ground: body,
        });
        self.dw.add_rigid_body_filtered(body, 1, 14);
    }

    /// `removeServerCell(cx, cy)`.
    pub fn remove_server_cell(&mut self, cx: i32, cy: i32) {
        let Some(i) = self
            .server_cells
            .iter()
            .position(|c| c.cx == cx && c.cy == cy)
        else {
            return;
        };
        let sc = self.server_cells.remove(i);
        self.dw.remove_rigid_body(sc.ground);
        for y in 0..5 {
            for x in 0..5 {
                self.delete_chunk(sc.chunks[x * 5 + y]);
            }
        }
    }

    /// `addPhysicsObject(worldX, worldY)`: a 1×1×1 box of mass 50 dropped
    /// from height 10.5, rotation locked.
    pub fn add_physics_object(&mut self, x: f32, y: f32) -> i32 {
        if self.physics_object_shape.is_none() {
            self.physics_object_shape = Some(
                self.dw
                    .cw
                    .shapes
                    .add(Shape::new_box(&Vec3::new(0.5, 0.5, 0.5))),
            );
        }
        let shape = self.physics_object_shape.unwrap();
        let origin = Vec3::new(
            (x - self.offset_x as f32) as f64 + 0.5,
            10.5,
            (y - self.offset_y as f32) as f64 + 0.5,
        );
        let ms = DefaultMotionState::new(Transform::new(Mat3::IDENTITY, origin));
        let inertia = self.dw.cw.shapes.get(shape).calculate_local_inertia(50.0);
        let info = RigidBodyConstructionInfo::new(50.0, Some(ms), Some(shape), inertia);
        let mut o = CollisionObject::new_rigid_body(&info);
        o.update_revision += 1;
        o.rb_mut().angular_factor = Vec3::ZERO;
        let b = self.dw.cw.insert_object(o);
        self.dw.add_rigid_body_filtered(b, 4, 15);
        self.physics_object_counter += 1;
        self.physics_objects.insert(self.physics_object_counter, b);
        self.physics_object_counter
    }
}
