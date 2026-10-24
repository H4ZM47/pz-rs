//! The Java side of the native pathfinder: `zombie.pathfind.nativeCode
//! .PathfindNative` (main thread) and `PathfindNativeThread` (the worker),
//! with `PathFindRequest`, `PathRequestTask`, `RequestQueue`, the chunk,
//! square and vehicle tasks and `VehicleState`.
//!
//! # Threads and timing
//!
//! In the game the native calls run on `PathfindNativeThread`, a daemon
//! thread that runs `updateThread()` at most 20 times a second (its `Sync`
//! sleeps out the rest of a 50 ms frame) and otherwise waits until the main
//! thread queues something. The main thread queues tasks and requests at any
//! time and collects finished requests in `updateMain()` once per frame, so
//! how many frames a request takes depends on wall-clock timing.
//!
//! [`PathfindService`] runs both halves on the caller's thread:
//! [`PathfindService::update_thread`] is one pass of `updateThread()` and
//! [`PathfindService::update_main`] is `updateMain()`. The order of work
//! inside a pass (at most 10 chunk updates, then all square, vehicle and
//! request tasks, then at most 2 requests by priority), the cancel handling
//! and the delivery order are the Java ones; only *when* a pass runs is up
//! to the caller. A schedule close to the game's is one `update_thread()`
//! per 50 ms of game time (every third frame at 60 fps) while there is work
//! ([`PathfindService::has_thread_work`], the inverse of `shouldWait()`),
//! and `update_main()` every frame. Running `update_thread()` right before
//! `update_main()` every frame gives the shortest latency the game can
//! produce. No real-thread wrapper is provided: the world, the queues and
//! the request map would all have to be shared, and nothing in the port
//! needs it yet.
//!
//! # Differences from the Java side
//!
//! * The mover is captured as a [`MoverSnapshot`] when the request is
//!   added; `isNpc()`, which Java reads on the worker thread, is read then
//!   too.
//! * The `IPathfinder` that receives the result is named by a
//!   [`FinderId`]; [`PathfindService::update_main`] calls a
//!   [`PathfindCallback`] with it.
//! * `IsoChunk.loadedBits & 2` (set by `addChunkToWorld`) and
//!   `IsoChunk.getLoadID()` are kept by the service per chunk position, as
//!   `pz-world` chunks carry neither. `squareChanged` sends a neighbour in a
//!   chunk the service has not seen with load id -1, which the native side
//!   ignores like any stale load id.
//! * Object pools, the debug path-to-mouse request (`doNotRelease`,
//!   `bRender`), `checkUseNativeCode` (switching to the Java
//!   `PolygonalMap2`, which is not ported) and rendering are left out.

use std::collections::{HashMap, HashSet, VecDeque};

use pz_world::chunk::{IsoChunk, IsoGridSquare};
use pz_world::{GetSquare, IsoDirections};

use crate::bytes::PathfindError;
use crate::game_vehicle::{poly_is_equal, GameVehicle, VehicleSource};
use crate::mover::{MoverClass, MoverId, MoverSnapshot};
use crate::path::Path;
use crate::request::{MoverKind, PathFindRequest as WireRequest};
use crate::square_bits::{SquareBits, SquareUpdate};
use crate::visibility::{Vehicle, VehiclePoly};
use crate::world::World;

/// Capacity of `PathfindNativeThread.pathBb` (at most 191 nodes fit).
pub const PATH_BUFFER_CAPACITY: usize = 3072;
/// `updateThread`: chunk updates per pass.
pub const CHUNK_UPDATES_PER_PASS: usize = 10;
/// `updateThread`: requests per pass.
pub const REQUESTS_PER_PASS: usize = 2;

/// Names the `IPathfinder` (usually a `PathFindBehavior2`) a request reports
/// to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FinderId(pub u64);

/// A request handed out by [`PathfindService::add_request`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// `zombie.pathfind.IPathfinder`.
pub trait PathfindCallback {
    /// `IPathfinder.Succeeded(path, mover)`. Levels in `path` are game
    /// levels (the native +32 is removed) and it has at least two nodes.
    fn succeeded(&mut self, finder: FinderId, mover: MoverId, path: &Path);
    /// `IPathfinder.Failed(mover)`.
    fn failed(&mut self, finder: FinderId, mover: MoverId);
}

/// `zombie.pathfind.nativeCode.PathFindRequest`.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingRequest {
    pub finder: FinderId,
    pub mover: MoverSnapshot,
    pub start_x: f32,
    pub start_y: f32,
    pub start_z: f32,
    pub target_x: f32,
    pub target_y: f32,
    pub target_z: f32,
    /// `targetXyz`: further candidate targets, `x, y, z` triples.
    pub target_xyz: Vec<f32>,
    /// `minLevel` (always 0).
    pub min_level: i32,
    /// `maxLevel` (always 63).
    pub max_level: i32,
    /// The result, levels already converted (`z - 32`).
    pub path: Path,
    pub cancel: bool,
}

impl PendingRequest {
    /// The 47-byte request `PathfindNative.findPath` writes.
    pub fn wire(&self) -> WireRequest {
        let m = &self.mover;
        WireRequest {
            start_x: self.start_x,
            start_y: self.start_y,
            start_z: self.start_z,
            target_x: self.target_x,
            target_y: self.target_y,
            target_z: self.target_z,
            mover: match m.class {
                MoverClass::Player => MoverKind::Player,
                MoverClass::Zombie => MoverKind::Zombie,
                // moverType 0.
                MoverClass::Animal | MoverClass::Other => MoverKind::Animal,
            },
            is_npc: m.class == MoverClass::Player && m.is_npc,
            can_crawl: m.can_crawl,
            crawling: m.crawling,
            ignore_crawl_cost: m.ignore_crawl_cost,
            can_thump: m.can_thump,
            can_climb_fences: m.can_climb_fences,
            has_target: m.has_target,
            can_bend: m.can_bend,
            min_level: self.min_level,
            max_level: self.max_level,
            render: false,
            can_climb_tall_fences: m.can_climb_tall_fences,
            avoid_farming_plants: m.avoid_farming_plants,
        }
    }
}

/// A queued chunk task (`ChunkUpdateTask` / `ChunkRemoveTask`).
#[derive(Clone, Debug, PartialEq)]
pub enum ChunkTask {
    /// `PathfindNative.updateChunk(loadId, wx, wy, bb)`.
    Update {
        load_id: i16,
        wx: i32,
        wy: i32,
        buffer: Vec<u8>,
    },
    /// `PathfindNative.removeChunk(wx, wy)`.
    Remove { wx: i32, wy: i32 },
}

/// A queued `SquareUpdateTask`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SquareTask {
    pub load_id: i16,
    pub update: SquareUpdate,
}

/// A queued vehicle task.
#[derive(Clone, Debug, PartialEq)]
pub enum VehicleTask {
    /// `VehicleAddTask`: `PathfindNative.addVehicle`.
    Add(Vehicle),
    /// `VehicleUpdateTask`: `PathfindNative.teleportVehicle`.
    Update(Vehicle),
    /// `VehicleRemoveTask`: `PathfindNative.removeVehicle`.
    Remove(i16),
}

/// What one [`PathfindService::update_thread`] pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadPass {
    pub chunk_tasks: usize,
    pub square_tasks: usize,
    pub vehicle_tasks: usize,
    pub request_tasks: usize,
    /// Requests taken off the priority queues, in order (cancelled ones
    /// included).
    pub requests: Vec<RequestId>,
    /// The first exception a native call threw outside the request loop
    /// (`run()` logs it and the rest of the pass is skipped).
    pub error: Option<PathfindError>,
}

/// `RequestQueue`: player, aggressive-zombie and other requests.
#[derive(Default)]
struct RequestQueue {
    player: VecDeque<RequestId>,
    aggro_zombie: VecDeque<RequestId>,
    other: VecDeque<RequestId>,
}

impl RequestQueue {
    fn is_empty(&self) -> bool {
        self.player.is_empty() && self.aggro_zombie.is_empty() && self.other.is_empty()
    }

    fn remove_first(&mut self) -> Option<RequestId> {
        self.player
            .pop_front()
            .or_else(|| self.aggro_zombie.pop_front())
            .or_else(|| self.other.pop_front())
    }

    fn add(&mut self, id: RequestId, queue: u8) {
        match queue {
            1 => self.player.push_back(id),
            2 => self.aggro_zombie.push_back(id),
            _ => self.other.push_back(id),
        }
    }
}

/// `PathfindNative` plus `PathfindNativeThread`, single-threaded (see the
/// module doc).
pub struct PathfindService {
    world: World,
    chunk_tasks: VecDeque<ChunkTask>,
    square_tasks: VecDeque<SquareTask>,
    vehicle_tasks: VecDeque<VehicleTask>,
    /// `requestTaskQueue`: `PathRequestTask`s (request, queue number).
    request_tasks: VecDeque<(RequestId, u8)>,
    requests: RequestQueue,
    request_map: HashMap<MoverId, RequestId>,
    request_to_main: VecDeque<RequestId>,
    live: HashMap<RequestId, PendingRequest>,
    next_request: u64,
    /// `PathfindNative.vehicleState`: the last `polyPlusRadius` sent, by
    /// `vehicleId`.
    vehicle_state: HashMap<i16, VehiclePoly>,
    /// Chunks added with `addChunkToWorld` (`loadedBits & 2`) and their
    /// load ids.
    loaded_chunks: HashMap<(i32, i32), i16>,
    path_buffer: Vec<u8>,
}

impl PathfindService {
    /// `PathfindNative.init(metaGrid)`: `initWorld(minX, minY, width,
    /// height, GameServer.server)` and a fresh worker.
    pub fn new(min_x: i32, min_y: i32, width: i32, height: i32, is_server: bool) -> Self {
        PathfindService {
            world: World::new(min_x, min_y, width, height, is_server),
            chunk_tasks: VecDeque::new(),
            square_tasks: VecDeque::new(),
            vehicle_tasks: VecDeque::new(),
            request_tasks: VecDeque::new(),
            requests: RequestQueue::default(),
            request_map: HashMap::new(),
            request_to_main: VecDeque::new(),
            live: HashMap::new(),
            next_request: 0,
            vehicle_state: HashMap::new(),
            loaded_chunks: HashMap::new(),
            path_buffer: Vec::new(),
        }
    }

    /// The native world.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// The native world, mutably (for tests and tools; the game only
    /// touches it from the worker).
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// `PathfindNative.stop()` without destroying the world:
    /// `PathfindNativeThread.cleanup()` (drops every queued task and
    /// request) and the vehicle states.
    pub fn stop(&mut self) {
        self.chunk_tasks.clear();
        self.square_tasks.clear();
        self.vehicle_tasks.clear();
        self.request_tasks.clear();
        self.requests = RequestQueue::default();
        self.request_to_main.clear();
        self.request_map.clear();
        self.live.clear();
        self.vehicle_state.clear();
    }

    // ----- main thread: world changes -----

    /// `PathfindNative.addChunkToWorld(chunk)`: queue a `ChunkUpdateTask`
    /// built now from `chunk` (`ChunkUpdateTask.init`) and mark the chunk as
    /// known to the pathfinder. `load_id` is `chunk.getLoadID()`.
    pub fn add_chunk_to_world<G: GetSquare>(
        &mut self,
        chunk: &IsoChunk,
        load_id: i16,
        bits: &SquareBits<'_, G>,
    ) {
        let buffer = bits.chunk_update_buffer(chunk);
        self.queue_chunk_update(load_id, chunk.wx, chunk.wy, buffer);
    }

    /// [`PathfindService::add_chunk_to_world`] with a prepared
    /// `ChunkUpdateTask` buffer.
    pub fn queue_chunk_update(&mut self, load_id: i16, wx: i32, wy: i32, buffer: Vec<u8>) {
        self.chunk_tasks.push_back(ChunkTask::Update {
            load_id,
            wx,
            wy,
            buffer,
        });
        self.loaded_chunks.insert((wx, wy), load_id);
    }

    /// `PathfindNative.removeChunkFromWorld(chunk)`.
    pub fn remove_chunk_from_world(&mut self, wx: i32, wy: i32) {
        self.chunk_tasks.push_back(ChunkTask::Remove { wx, wy });
        self.loaded_chunks.remove(&(wx, wy));
    }

    /// Whether `addChunkToWorld` was called for the chunk at `(wx, wy)`
    /// since it was last removed (`chunk.loadedBits & 2`).
    pub fn is_chunk_added(&self, wx: i32, wy: i32) -> bool {
        self.loaded_chunks.contains_key(&(wx, wy))
    }

    /// `PathfindNative.squareChanged(square)`: when the square's chunk was
    /// added, queue a `SquareUpdateTask` for each of its 8 neighbours
    /// (`IsoDirections` order) and then for the square.
    pub fn square_changed<G: GetSquare>(
        &mut self,
        square: &IsoGridSquare,
        bits: &SquareBits<'_, G>,
    ) {
        let chunk = (square.x.div_euclid(8), square.y.div_euclid(8));
        if !self.loaded_chunks.contains_key(&chunk) {
            return;
        }
        for dir in IsoDirections::ALL {
            if let Some(sq2) = square.get_adjacent_square(dir, bits.squares) {
                self.queue_square_update(sq2, bits);
            }
        }
        self.queue_square_update(square, bits);
    }

    fn queue_square_update<G: GetSquare>(&mut self, sq: &IsoGridSquare, bits: &SquareBits<'_, G>) {
        let chunk = (sq.x.div_euclid(8), sq.y.div_euclid(8));
        let load_id = self.loaded_chunks.get(&chunk).copied().unwrap_or(-1);
        self.square_tasks.push_back(SquareTask {
            load_id,
            update: bits.square_update(sq),
        });
    }

    /// `PathfindNative.addVehicle(vehicle)`.
    pub fn add_vehicle(&mut self, vehicle: &dyn GameVehicle) {
        self.vehicle_state
            .insert(vehicle.vehicle_id(), vehicle.poly_plus_radius());
        self.vehicle_tasks
            .push_back(VehicleTask::Add(vehicle.native_vehicle()));
    }

    /// `PathfindNative.removeVehicle(vehicle)`.
    pub fn remove_vehicle(&mut self, vehicle_id: i16) {
        self.vehicle_state.remove(&vehicle_id);
        self.vehicle_tasks
            .push_back(VehicleTask::Remove(vehicle_id));
    }

    /// `PathfindNative.updateVehicle(vehicle)`.
    pub fn update_vehicle(&mut self, vehicle: &dyn GameVehicle) {
        self.vehicle_tasks
            .push_back(VehicleTask::Update(vehicle.native_vehicle()));
    }

    // ----- main thread: requests -----

    /// `PathfindNative.addRequest(finder, mover, start, target)`, then
    /// `request.targetXyz.addAll(extra_targets)` as `PathFindBehavior2`
    /// does. Cancels the mover's previous request.
    #[allow(clippy::too_many_arguments)]
    pub fn add_request(
        &mut self,
        finder: FinderId,
        mover: MoverSnapshot,
        start: (f32, f32, f32),
        target: (f32, f32, f32),
        extra_targets: &[f32],
    ) -> RequestId {
        self.cancel_request(mover.id);
        let id = RequestId(self.next_request);
        self.next_request += 1;
        let queue = mover.queue;
        let mover_id = mover.id;
        self.live.insert(
            id,
            PendingRequest {
                finder,
                mover,
                start_x: start.0,
                start_y: start.1,
                start_z: start.2,
                target_x: target.0,
                target_y: target.1,
                target_z: target.2,
                target_xyz: extra_targets.to_vec(),
                min_level: 0,
                max_level: 63,
                path: Path::default(),
                cancel: false,
            },
        );
        self.request_map.insert(mover_id, id);
        self.request_tasks.push_back((id, queue));
        id
    }

    /// `PathfindNative.cancelRequest(mover)`.
    pub fn cancel_request(&mut self, mover: MoverId) {
        if let Some(id) = self.request_map.remove(&mover) {
            if let Some(r) = self.live.get_mut(&id) {
                r.cancel = true;
            }
        }
    }

    /// The live request (queued, being processed or waiting for
    /// `updateMain`).
    pub fn request(&self, id: RequestId) -> Option<&PendingRequest> {
        self.live.get(&id)
    }

    /// The mover's current request (`requestMap.get(mover)`).
    pub fn request_for(&self, mover: MoverId) -> Option<RequestId> {
        self.request_map.get(&mover).copied()
    }

    /// `PathfindNative.updateMain()`: send changed vehicles to the worker
    /// (`VehicleState.check`), then deliver every finished request in the
    /// order the worker finished them: not cancelled, an empty path calls
    /// `Failed`, any other `Succeeded`.
    pub fn update_main(
        &mut self,
        vehicles: &dyn VehicleSource,
        callback: &mut dyn PathfindCallback,
    ) {
        let mut changed = Vec::new();
        vehicles.for_each_vehicle(&mut |v| {
            if let Some(state) = self.vehicle_state.get_mut(&v.vehicle_id()) {
                let now = v.poly_plus_radius();
                if !poly_is_equal(state, &now) {
                    *state = now;
                    changed.push(v.native_vehicle());
                }
            }
        });
        for v in changed {
            self.vehicle_tasks.push_back(VehicleTask::Update(v));
        }
        while let Some(id) = self.request_to_main.pop_front() {
            let Some(r) = self.live.remove(&id) else {
                continue;
            };
            if self.request_map.get(&r.mover.id) == Some(&id) {
                self.request_map.remove(&r.mover.id);
            }
            if !r.cancel {
                if r.path.is_empty() {
                    callback.failed(r.finder, r.mover.id);
                } else {
                    callback.succeeded(r.finder, r.mover.id, &r.path);
                }
            }
        }
    }

    // ----- worker -----

    /// The inverse of `PathfindNativeThread.shouldWait()`: whether a pass
    /// has anything to do.
    pub fn has_thread_work(&self) -> bool {
        !self.chunk_tasks.is_empty()
            || !self.square_tasks.is_empty()
            || !self.vehicle_tasks.is_empty()
            || !self.request_tasks.is_empty()
            || !self.requests.is_empty()
    }

    /// One `PathfindNativeThread.updateThread()` pass: up to
    /// [`CHUNK_UPDATES_PER_PASS`] chunk updates (removals do not count),
    /// every queued square task, vehicle task and request task, the native
    /// `update()`, then up to [`REQUESTS_PER_PASS`] requests by priority.
    /// A cancelled request is passed straight back and does not count
    /// against the limit.
    pub fn update_thread(&mut self) -> ThreadPass {
        let mut pass = ThreadPass::default();
        if let Err(e) = self.run_tasks(&mut pass) {
            pass.error = Some(e);
            return pass;
        }
        self.world.update();
        let mut left = REQUESTS_PER_PASS;
        while let Some(id) = self.requests.remove_first() {
            pass.requests.push(id);
            let Some(mut r) = self.live.remove(&id) else {
                continue;
            };
            if r.cancel {
                self.live.insert(id, r);
                self.request_to_main.push_back(id);
                continue;
            }
            let found = self.find_path(&mut r).and_then(|()| {
                if !r.target_xyz.is_empty() {
                    self.find_shortest_path_of_multiple(&mut r)
                } else {
                    Ok(())
                }
            });
            if found.is_err() {
                // ExceptionLogger.logException(t); request.path.clear();
                r.path.clear();
            }
            self.live.insert(id, r);
            self.request_to_main.push_back(id);
            left -= 1;
            if left == 0 {
                break;
            }
        }
        pass
    }

    fn run_tasks(&mut self, pass: &mut ThreadPass) -> Result<(), PathfindError> {
        let mut update = CHUNK_UPDATES_PER_PASS;
        while let Some(task) = self.chunk_tasks.pop_front() {
            pass.chunk_tasks += 1;
            match task {
                ChunkTask::Update {
                    load_id,
                    wx,
                    wy,
                    buffer,
                } => {
                    self.world.update_chunk(load_id as i32, wx, wy, &buffer)?;
                    update -= 1;
                    if update == 0 {
                        break;
                    }
                }
                ChunkTask::Remove { wx, wy } => self.world.remove_chunk(wx, wy)?,
            }
        }
        while let Some(t) = self.square_tasks.pop_front() {
            pass.square_tasks += 1;
            let u = t.update;
            self.world.update_square(
                t.load_id as i32,
                u.x,
                u.y,
                u.z,
                u.bits as i64,
                u.cost,
                u.slope_direction,
                u.slope_min,
                u.slope_max,
            );
        }
        while let Some(t) = self.vehicle_tasks.pop_front() {
            pass.vehicle_tasks += 1;
            match t {
                VehicleTask::Add(v) => self.world.add_vehicle_typed(v),
                VehicleTask::Update(v) => self.world.teleport_vehicle_typed(v),
                VehicleTask::Remove(id) => self.world.remove_vehicle(id as i32),
            }
        }
        while let Some((id, queue)) = self.request_tasks.pop_front() {
            pass.request_tasks += 1;
            self.requests.add(id, queue);
        }
        Ok(())
    }

    /// `PathfindNativeThread.findPath(request)`: run the native search and
    /// turn its buffer into `request.path` (levels `z - 32`; a single node
    /// is duplicated; fewer than two nodes is no path).
    fn find_path(&mut self, r: &mut PendingRequest) -> Result<(), PathfindError> {
        let wire = r.wire().to_bytes();
        let mut out = std::mem::take(&mut self.path_buffer);
        let result = self
            .world
            .find_path_into(&wire, &mut out, PATH_BUFFER_CAPACITY);
        let parsed = result.map(|ret| {
            r.path.clear();
            if ret == 1 {
                let count = i16::from_be_bytes([out[0], out[1]]).max(0) as usize;
                for i in 0..count {
                    let o = 2 + i * 16;
                    let f =
                        |k: usize| f32::from_be_bytes(out[o + k..o + k + 4].try_into().unwrap());
                    let flags = i32::from_be_bytes(out[o + 12..o + 16].try_into().unwrap());
                    r.path.add_node(f(0), f(4), f(8) - 32.0, flags);
                }
            }
            if r.path.len() == 1 {
                let n = r.path.nodes[0];
                r.path.add_node(n.x, n.y, n.z, 0);
            }
            if r.path.len() < 2 {
                r.path.clear();
            }
        });
        self.path_buffer = out;
        parsed
    }

    /// `PathfindNativeThread.findShortestPathOfMultiple(request)`: also
    /// search every `targetXyz` and keep the shortest path
    /// (`Path.length()`, see [`Path::java_length`]) and its target. Ties
    /// keep the earlier target.
    fn find_shortest_path_of_multiple(
        &mut self,
        r: &mut PendingRequest,
    ) -> Result<(), PathfindError> {
        let mut shortest = r.path.clone();
        let (mut tx, mut ty, mut tz) = (r.target_x, r.target_y, r.target_z);
        let mut min_length = if shortest.is_empty() {
            f32::MAX
        } else {
            shortest.java_length()
        };
        let targets = r.target_xyz.clone();
        for t in targets.as_chunks::<3>().0 {
            r.target_x = t[0];
            r.target_y = t[1];
            r.target_z = t[2];
            r.path.clear();
            self.find_path(r)?;
            if r.path.is_empty() {
                continue;
            }
            let length = r.path.java_length();
            if length < min_length {
                min_length = length;
                shortest = r.path.clone();
                (tx, ty, tz) = (r.target_x, r.target_y, r.target_z);
            }
        }
        r.path = shortest;
        r.target_x = tx;
        r.target_y = ty;
        r.target_z = tz;
        Ok(())
    }

    /// The queued chunk tasks (for tests and diagnostics).
    pub fn queued_chunk_tasks(&self) -> &VecDeque<ChunkTask> {
        &self.chunk_tasks
    }

    /// The queued square tasks.
    pub fn queued_square_tasks(&self) -> &VecDeque<SquareTask> {
        &self.square_tasks
    }

    /// The queued vehicle tasks.
    pub fn queued_vehicle_tasks(&self) -> &VecDeque<VehicleTask> {
        &self.vehicle_tasks
    }

    /// Movers with a request in flight (`requestMap` keys).
    pub fn movers_with_requests(&self) -> HashSet<MoverId> {
        self.request_map.keys().copied().collect()
    }
}

impl Path {
    /// `zombie.pathfind.Path.length()` on the main-thread path (levels
    /// already converted): `IsoUtils.DistanceTo(x1, y1, z1, x2, y2, z2)` per
    /// segment, computed in double from float differences, without the
    /// native side's level weighting.
    pub fn java_length(&self) -> f32 {
        let mut dist = 0.0f32;
        for w in self.nodes.windows(2) {
            let (a, b) = (w[0], w[1]);
            let dx = (b.x - a.x) as f64;
            let dy = (b.y - a.y) as f64;
            let dz = (b.z - a.z) as f64;
            dist += (dx * dx + dy * dy + dz * dz).sqrt() as f32;
        }
        dist
    }
}
