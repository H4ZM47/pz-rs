//! `zombie.pathfind.PathFindBehavior2`: a character following a path
//! from the pathfinder, frame by frame.
//!
//! A [`PathFindBehavior2`] belongs to one character. The character, the
//! world and the pathfinder are handed in on every call through a
//! [`PathContext`]; nothing here holds on to them. Requests go to a
//! [`PathfindService`] under the behaviour's [`FinderId`]; when the
//! service reports a result for that finder (see [`Delivered`]), the owner
//! calls [`PathFindBehavior2::succeeded`] or [`PathFindBehavior2::failed`],
//! which is `IPathfinder.Succeeded` / `Failed`.
//!
//! The Java float and double arithmetic is kept as written: `IsoUtils`
//! distances go through `Math.pow` in double, `Vector2.normalize` skips
//! vectors whose squared length is within `1e-5` of one, and the
//! closest-point maths mixes float and double exactly as the original.
//!
//! Left out: the debug rendering (`render`, the `actualPos` trail) and the
//! `PolygonalMap2` (non-native) request path. `pathToNearestTable` takes a
//! plain slice (the Lua table conversion belongs to the Lua bindings).

// `!(a < b)` keeps Java's NaN behaviour.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod hooks;

use pz_world::chunk::IsoGridSquare;
use pz_world::objects::ObjectKind;
use pz_world::{GetSquare, IsoDirections};

use crate::main_thread::{line_clear, TestResults};
use crate::main_thread::{normalize, LineClearCollideMain, MainThreadMap};
use crate::mover::{
    CharacterState, CharacterView, FaceTarget, MoverId, MoverSnapshot, ObjectRef, PathFindProgress,
    PathMover, PathWorldHooks,
};
use crate::path::Path;
use crate::service::{FinderId, PathfindCallback, PathfindService};

pub use hooks::*;

/// `PathFindBehavior2.Goal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Goal {
    #[default]
    None,
    Character,
    Location,
    Sound,
    VehicleAdjacent,
    VehicleArea,
    VehicleSeat,
    SitOnFurniture,
    GrabCorpse,
}

/// `PathFindBehavior2.BehaviorResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BehaviorResult {
    Working,
    Failed,
    Succeeded,
}

/// `PathFindBehavior2.PointOnPath`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointOnPath {
    pub path_index: i32,
    pub dist: f32,
    pub x: f32,
    pub y: f32,
}

/// `pathToNearest` argument errors (Java's `IllegalArgumentException`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathArgumentError {
    /// "locations is null or empty".
    Empty,
    /// "locations should be multiples of x,y,z".
    NotTriples,
}

/// `zombie.ai.WalkingOnTheSpot`: fails a path when the character has not
/// moved for a while.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WalkingOnTheSpot {
    x: f32,
    y: f32,
    time: f32,
    seconds: f32,
}

impl WalkingOnTheSpot {
    /// `check(chr)`: true once the character has stayed within 0.1 of one
    /// spot for longer than 400 multiplier units (animals 30, or 150 at the
    /// fastest game speed outside a server).
    pub fn check(&mut self, chr: &dyn PathMover, world: &dyn PathWorldHooks) -> bool {
        let mut time_check = 400.0f32;
        if chr.is_animal() {
            time_check = 30.0;
            if !world.is_server() && world.current_game_speed() == 4 {
                time_check = 150.0;
            }
        }
        self.step(chr.x(), chr.y(), world);
        self.time > time_check
    }

    /// `check(x, y)`.
    pub fn check_xy(&mut self, x: f32, y: f32, world: &dyn PathWorldHooks) -> bool {
        self.step(x, y, world);
        self.time > 400.0
    }

    fn step(&mut self, x: f32, y: f32, world: &dyn PathWorldHooks) {
        if distance_to_squared(self.x, self.y, x, y) < 0.010_000_001f32 {
            self.time += world.game_time_multiplier();
            self.seconds += world.thirty_fps_multiplier() / 30.0;
        } else {
            self.x = x;
            self.y = y;
            self.time = 0.0;
            self.seconds = 0.0;
        }
    }

    /// `seconds`: how long the character has stood still, in seconds.
    pub fn seconds(&self) -> f32 {
        self.seconds
    }

    /// `reset(x, y)`.
    pub fn reset(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
        self.time = 0.0;
        self.seconds = 0.0;
    }
}

/// What `PathFindBehavior2` works with on each call.
pub struct PathContext<'c, 'w, 'a, G: GetSquare> {
    /// The native pathfinder's Java side (`PathfindNative.instance`).
    pub service: &'c mut PathfindService,
    /// Squares and vehicles (`PolygonalMap2.instance` main-thread queries,
    /// `IsoWorld.instance.currentCell`).
    pub map: &'c MainThreadMap<'w, 'a, G>,
    pub world: &'c dyn PathWorldHooks,
    pub vehicles: &'c dyn VehicleScripts,
    pub seating: &'c dyn SeatingHooks,
    pub characters: &'c dyn CharacterLookup,
}

impl<'c, 'w, 'a: 'w, G: GetSquare> PathContext<'c, 'w, 'a, G> {
    fn lcc(&self) -> LineClearCollideMain<'_, 'w, 'a, G> {
        LineClearCollideMain::new(self.map)
    }
}

/// Collects the service's results for dispatch to the behaviours
/// (`PathfindNative.updateMain` calls `IPathfinder` directly; here the
/// owner routes each entry to [`PathFindBehavior2::succeeded`] /
/// [`PathFindBehavior2::failed`] by its [`FinderId`]).
#[derive(Clone, Debug, Default)]
pub struct Delivered {
    /// `(finder, mover, path)`; `None` is `Failed`.
    pub results: Vec<(FinderId, MoverId, Option<Path>)>,
}

impl PathfindCallback for Delivered {
    fn succeeded(&mut self, finder: FinderId, mover: MoverId, path: &Path) {
        self.results.push((finder, mover, Some(path.clone())));
    }
    fn failed(&mut self, finder: FinderId, mover: MoverId) {
        self.results.push((finder, mover, None));
    }
}

// ----- Java maths -----

/// `PZMath.fastfloor(float)`.
fn ff(v: f32) -> i32 {
    crate::main_thread::fastfloor(v)
}

/// `PZMath.fastfloor(double)`.
fn ffd(v: f64) -> i32 {
    let i = v as i32;
    if v < i as f64 {
        i - 1
    } else {
        i
    }
}

/// `IsoUtils.DistanceTo(x1, y1, x2, y2)` (also `DistanceTo2D`).
pub fn distance_to(fx: f32, fy: f32, tx: f32, ty: f32) -> f32 {
    let dx = (tx - fx) as f64;
    let dy = (ty - fy) as f64;
    (dx * dx + dy * dy).sqrt() as f32
}

/// `IsoUtils.DistanceToSquared(x1, y1, x2, y2)`.
pub fn distance_to_squared(fx: f32, fy: f32, tx: f32, ty: f32) -> f32 {
    let dx = (tx - fx) as f64;
    let dy = (ty - fy) as f64;
    (dx * dx + dy * dy) as f32
}

/// `IsoUtils.DistanceToSquared(x1, y1, z1, x2, y2, z2)`.
pub fn distance_to_squared3(fx: f32, fy: f32, fz: f32, tx: f32, ty: f32, tz: f32) -> f32 {
    let dx = (tx - fx) as f64;
    let dy = (ty - fy) as f64;
    let dz = (tz - fz) as f64;
    (dx * dx + dy * dy + dz * dz) as f32
}

/// `Vector2.getLength()`.
fn v_len(x: f32, y: f32) -> f32 {
    ((x * x + y * y) as f64).sqrt() as f32
}

/// `Vector2.setLength(len)`.
fn v_set_length(x: f32, y: f32, len: f32) -> (f32, f32) {
    let (x, y) = normalize(x, y);
    (x * len, y * len)
}

/// `Vector3f.length()` (JOML, no FMA).
fn v3_len(x: f32, y: f32, z: f32) -> f32 {
    ((x * x + y * y + z * z) as f64).sqrt() as f32
}

/// `PathFindBehavior2.closestPointOnPath(x3, y3, z, mover, path, pop)`:
/// the point of the path closest to `(x3, y3, z)` among segments on that
/// level whose closest point is reachable from the mover's square (adjacent
/// squares are tested with `testCollideAdjacent(mover, ...)`, a zombie with
/// `ghost` forced on). `mover` `None` is a `null` mover (the plain collide
/// matrix).
#[allow(clippy::too_many_arguments)]
pub fn closest_point_on_path<G: GetSquare>(
    map: &MainThreadMap<'_, '_, G>,
    x3: f32,
    y3: f32,
    z: f32,
    mut mover: Option<&mut dyn PathMover>,
    path: &Path,
    pop: &mut PointOnPath,
) {
    let lcc = LineClearCollideMain::new(map);
    pop.path_index = 0;
    let mut closest_dist = f32::MAX;
    let n = path.nodes.len();
    for i in 0..n.saturating_sub(1) {
        let node1 = path.nodes[i];
        let node2 = path.nodes[i + 1];
        if ff(node1.z) != ff(z) && ff(node2.z) != ff(z) {
            continue;
        }
        let (x1, y1, x2, y2) = (node1.x, node1.y, node2.x, node2.y);
        let num = ((x3 - x1) * (x2 - x1) + (y3 - y1) * (y2 - y1)) as f64;
        let ddx = (x2 - x1) as f64;
        let ddy = (y2 - y1) as f64;
        let mut u = num / (ddx * ddx + ddy * ddy);
        let mut xu = x1 as f64 + u * (x2 - x1) as f64;
        let mut yu = y1 as f64 + u * (y2 - y1) as f64;
        if u <= 0.0 {
            xu = x1 as f64;
            yu = y1 as f64;
            u = 0.0;
        } else if u >= 1.0 {
            xu = x2 as f64;
            yu = y2 as f64;
            u = 1.0;
        }
        let dx = ffd(xu) - ff(x3);
        let dy = ffd(yu) - ff(y3);
        if (dx != 0 || dy != 0) && dx.abs() <= 1 && dy.abs() <= 1 {
            let square1 = map.square(ff(x3), ff(y3), ff(z));
            let square2 = map.square(ffd(xu), ffd(yu), ff(z));
            if let (Some(s1), Some(_)) = (square1, square2) {
                let from = (s1.x, s1.y, s1.z);
                let blocked = match mover.as_deref_mut() {
                    Some(m) => {
                        if let Some(zm) = m.as_zombie_mut() {
                            let ghost = zm.is_ghost();
                            zm.set_ghost(true);
                            let b = m.test_collide_adjacent(from, dx, dy, 0);
                            if let Some(zm) = m.as_zombie_mut() {
                                zm.set_ghost(ghost);
                            }
                            b
                        } else {
                            m.test_collide_adjacent(from, dx, dy, 0)
                        }
                    }
                    None => s1.test_collide_adjacent(map.bits.meta, dx, dy, 0),
                };
                if blocked {
                    continue;
                }
            }
        }
        let mut closest_z = z;
        if dx.abs() <= 1 && dy.abs() <= 1 {
            let s1 = map.square(ff(node1.x), ff(node1.y), ff(node1.z));
            let s2 = map.square(ff(node2.x), ff(node2.y), ff(node2.z));
            let z1 = s1.map_or(node1.z, |s| lcc.get_apparent_z(s));
            let z2 = s2.map_or(node2.z, |s| lcc.get_apparent_z(s));
            closest_z = z1 + (z2 - z1) * u as f32;
        }
        let dist = distance_to_squared3(x3, y3, z, xu as f32, yu as f32, closest_z);
        if dist < closest_dist {
            closest_dist = dist;
            pop.path_index = i as i32;
            pop.dist = if u == 1.0 { 1.0 } else { u as f32 };
            pop.x = xu as f32;
            pop.y = yu as f32;
        }
    }
}

/// `PathFindBehavior2.advanceAlongPath(x, y, z, dist, pop)`: the point
/// `dist` further along the path than the closest point to `(x, y, z)`.
#[allow(clippy::too_many_arguments)]
pub fn advance_along_path<G: GetSquare>(
    map: &MainThreadMap<'_, '_, G>,
    mut x: f32,
    mut y: f32,
    z: f32,
    mut dist: f32,
    mover: Option<&mut dyn PathMover>,
    path: &Path,
    pop: &mut PointOnPath,
) {
    closest_point_on_path(map, x, y, z, mover, path, pop);
    let n = path.nodes.len() as i32;
    let mut i = pop.path_index;
    while i < n - 1 {
        let node1 = path.nodes[i as usize];
        let node2 = path.nodes[i as usize + 1];
        let dist2 = distance_to(x, y, node2.x, node2.y) as f64;
        if !(dist as f64 > dist2) {
            pop.path_index = i;
            pop.dist += dist / distance_to(node1.x, node1.y, node2.x, node2.y);
            pop.x = node1.x + pop.dist * (node2.x - node1.x);
            pop.y = node1.y + pop.dist * (node2.y - node1.y);
            return;
        }
        x = node2.x;
        y = node2.y;
        dist = (dist as f64 - dist2) as f32;
        pop.dist = 0.0;
        i += 1;
    }
    pop.path_index = n - 1;
    pop.dist = 1.0;
    let last = path.nodes[pop.path_index as usize];
    pop.x = last.x;
    pop.y = last.y;
}

/// The body of `PathFindBehavior2.getPathLength()`: from `(cx, cy)` to
/// node `path_index + 1` and on along the path, or straight to the target
/// when the path is empty or used up.
pub fn path_length(path: &Path, path_index: i32, target: (f32, f32), cx: f32, cy: f32) -> f32 {
    let straight = |tx: f32, ty: f32| {
        let dx = cx - tx;
        let dy = cy - ty;
        ((dx * dx + dy * dy) as f64).sqrt() as f32
    };
    let n = path.nodes.len() as i32;
    if n == 0 || path_index + 1 >= n {
        return straight(target.0, target.1);
    }
    let next = path.nodes[(path_index + 1) as usize];
    let mut length = straight(next.x, next.y);
    for i in (path_index + 2)..n {
        let a = path.nodes[(i - 1) as usize];
        let b = path.nodes[i as usize];
        let dx = a.x - b.x;
        let dy = a.y - b.y;
        length += ((dx * dx + dy * dy) as f64).sqrt() as f32;
    }
    length
}

/// `PathFindBehavior2`.
#[derive(Clone, Debug)]
pub struct PathFindBehavior2 {
    /// The `IPathfinder` identity under which requests are made.
    pub finder: FinderId,
    pub path_next_is_set: bool,
    pub path_next_x: f32,
    pub path_next_y: f32,
    start_x: f32,
    start_y: f32,
    start_z: f32,
    target_x: f32,
    target_y: f32,
    target_z: f32,
    target_xyz: Vec<f32>,
    path: Path,
    path_index: i32,
    is_cancel: bool,
    started_moving: bool,
    pub stopping: bool,
    turning_to_obstacle: bool,
    pub walking_on_the_spot: WalkingOnTheSpot,
    goal: Goal,
    goal_character: Option<MoverId>,
    goal_sit_on_furniture_object: Option<ObjectRef>,
    goal_sit_on_furniture_any_sprite_grid_object: bool,
    goal_vehicle: Option<i16>,
    goal_vehicle_area: Option<String>,
    goal_vehicle_seat: i32,
    point_on_path: PointOnPath,
}

const DIRECTIONS: [&str; 4] = ["N", "S", "W", "E"];
const SIDES: [&str; 3] = ["Front", "Left", "Right"];

impl PathFindBehavior2 {
    /// `new PathFindBehavior2(chr)`.
    pub fn new(finder: FinderId) -> Self {
        PathFindBehavior2 {
            finder,
            path_next_is_set: false,
            path_next_x: 0.0,
            path_next_y: 0.0,
            start_x: 0.0,
            start_y: 0.0,
            start_z: 0.0,
            target_x: 0.0,
            target_y: 0.0,
            target_z: 0.0,
            target_xyz: Vec::new(),
            path: Path::default(),
            path_index: 0,
            is_cancel: true,
            started_moving: false,
            stopping: false,
            turning_to_obstacle: false,
            walking_on_the_spot: WalkingOnTheSpot::default(),
            goal: Goal::None,
            goal_character: None,
            goal_sit_on_furniture_object: None,
            goal_sit_on_furniture_any_sprite_grid_object: false,
            goal_vehicle: None,
            goal_vehicle_area: None,
            goal_vehicle_seat: 0,
            point_on_path: PointOnPath::default(),
        }
    }

    // ----- state accessors -----

    pub fn goal(&self) -> Goal {
        self.goal
    }
    pub fn is_goal_none(&self) -> bool {
        self.goal == Goal::None
    }
    pub fn is_goal_character(&self) -> bool {
        self.goal == Goal::Character
    }
    pub fn is_goal_location(&self) -> bool {
        self.goal == Goal::Location
    }
    pub fn is_goal_sound(&self) -> bool {
        self.goal == Goal::Sound
    }
    pub fn is_goal_sit_on_furniture(&self) -> bool {
        self.goal == Goal::SitOnFurniture
    }
    /// `getGoalSitOnFurnitureObject()`.
    pub fn goal_sit_on_furniture_object(&self) -> Option<ObjectRef> {
        self.goal_sit_on_furniture_object
    }
    pub fn is_goal_vehicle_adjacent(&self) -> bool {
        self.goal == Goal::VehicleAdjacent
    }
    pub fn is_goal_vehicle_area(&self) -> bool {
        self.goal == Goal::VehicleArea
    }
    pub fn is_goal_vehicle_seat(&self) -> bool {
        self.goal == Goal::VehicleSeat
    }
    /// The goal vehicle, its area and seat (`goalVehicle`,
    /// `goalVehicleArea`, `goalVehicleSeat`).
    pub fn goal_vehicle(&self) -> (Option<i16>, Option<&str>, i32) {
        (
            self.goal_vehicle,
            self.goal_vehicle_area.as_deref(),
            self.goal_vehicle_seat,
        )
    }
    pub fn target_x(&self) -> f32 {
        self.target_x
    }
    pub fn target_y(&self) -> f32 {
        self.target_y
    }
    pub fn target_z(&self) -> f32 {
        self.target_z
    }
    /// `targetXyz`: the other candidate targets.
    pub fn target_xyz(&self) -> &[f32] {
        &self.target_xyz
    }
    /// The start the last request uses (`startX/Y/Z`).
    pub fn start(&self) -> (f32, f32, f32) {
        (self.start_x, self.start_y, self.start_z)
    }
    /// The behaviour's path (what `chr.getPath2()` is).
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn path_index(&self) -> i32 {
        self.path_index
    }
    /// `cancel()`.
    pub fn cancel(&mut self) {
        self.is_cancel = true;
    }
    /// `getIsCancelled()`.
    pub fn is_cancelled(&self) -> bool {
        self.is_cancel
    }
    /// `getTargetChar()`.
    pub fn target_char(&self) -> Option<MoverId> {
        if self.goal == Goal::Character {
            self.goal_character
        } else {
            None
        }
    }
    /// `isTargetLocation(x, y, z)`.
    pub fn is_target_location(&self, x: f32, y: f32, z: f32) -> bool {
        self.goal == Goal::Location
            && x == self.target_x
            && y == self.target_y
            && ff(z) == ff(self.target_z)
    }
    /// `shouldGetUpFromCrawl()`.
    pub fn should_get_up_from_crawl(&self, chr: &dyn PathMover) -> bool {
        chr.get_variable_boolean("ShouldStandUp")
    }
    /// `hasStartedMoving()`.
    pub fn has_started_moving(&self) -> bool {
        self.started_moving
    }
    /// `isTurningToObstacle()`.
    pub fn is_turning_to_obstacle(&self) -> bool {
        self.turning_to_obstacle
    }
    /// `allowTurnAnimation()`.
    pub fn allow_turn_animation(&self) -> bool {
        !self.has_started_moving() || self.is_turning_to_obstacle()
    }
    /// `shouldBeMoving()`.
    pub fn should_be_moving(&self, chr: &dyn PathMover) -> bool {
        if self.stopping {
            return false;
        }
        !self.allow_turn_animation() || !chr.should_be_turning()
    }
    /// `isMovingUsingPathFind()`.
    pub fn is_moving_using_path_find(&self) -> bool {
        !self.stopping && !self.is_goal_none() && !self.is_cancel
    }
    /// `shouldIgnoreCollisionWithSquare(square)`: true for the square of
    /// the furniture being sat on.
    pub fn should_ignore_collision_with_square(&self, square: (i32, i32, i32)) -> bool {
        self.goal == Goal::SitOnFurniture
            && self
                .goal_sit_on_furniture_object
                .is_some_and(|o| (o.x, o.y, o.z) == square)
    }
    /// `isStrafing()`.
    pub fn is_strafing(&self, chr: &dyn PathMover) -> bool {
        if chr.is_zombie() || self.stopping {
            return false;
        }
        if chr.as_player().is_some_and(|p| !p.is_local()) {
            return false;
        }
        self.path.nodes.len() == 2
            && distance_to_squared3(
                self.start_x,
                self.start_y,
                self.start_z * 3.0,
                self.target_x,
                self.target_y,
                self.target_z * 3.0,
            ) < 0.25
    }

    /// `getPathLength()`: what is left of the path from the character.
    pub fn get_path_length(&self, chr: &dyn CharacterView) -> f32 {
        path_length(
            &self.path,
            self.path_index,
            (self.target_x, self.target_y),
            chr.x(),
            chr.y(),
        )
    }

    // ----- goals -----

    /// `reset()`.
    pub fn reset(&mut self, chr: &mut dyn PathMover) {
        self.start_x = chr.x();
        self.start_y = chr.y();
        self.start_z = chr.z();
        self.target_x = self.start_x;
        self.target_y = self.start_y;
        self.target_z = self.start_z;
        self.target_xyz.clear();
        self.path_index = 0;
        chr.set_path_find_progress(PathFindProgress::NotRunning);
        self.walking_on_the_spot.reset(self.start_x, self.start_y);
    }

    /// `setData(targetX, targetY, targetZ)`: start from the character, set
    /// the target, cancel the running request.
    pub fn set_data(
        &mut self,
        chr: &mut dyn PathMover,
        service: &mut PathfindService,
        x: f32,
        y: f32,
        z: f32,
    ) {
        self.start_x = chr.x();
        self.start_y = chr.y();
        self.start_z = chr.z();
        self.target_x = x;
        self.target_y = y;
        self.target_z = z;
        self.target_xyz.clear();
        self.path_index = 0;
        service.cancel_request(chr.mover_id());
        chr.set_path_find_progress(PathFindProgress::NotRunning);
        self.stopping = false;
    }

    fn begin(&mut self, goal: Goal) {
        self.is_cancel = false;
        self.started_moving = false;
        self.goal = goal;
    }

    /// `pathToLocation(x, y, z)`: the centre of a square.
    pub fn path_to_location(
        &mut self,
        chr: &mut dyn PathMover,
        service: &mut PathfindService,
        x: i32,
        y: i32,
        z: i32,
    ) {
        self.begin(Goal::Location);
        self.set_data(chr, service, x as f32 + 0.5, y as f32 + 0.5, z as f32);
    }

    /// `pathToLocationF(x, y, z)`.
    pub fn path_to_location_f(
        &mut self,
        chr: &mut dyn PathMover,
        service: &mut PathfindService,
        x: f32,
        y: f32,
        z: f32,
    ) {
        self.begin(Goal::Location);
        self.set_data(chr, service, x, y, z);
    }

    /// `pathToSound(x, y, z)`.
    pub fn path_to_sound(
        &mut self,
        chr: &mut dyn PathMover,
        service: &mut PathfindService,
        x: i32,
        y: i32,
        z: i32,
    ) {
        self.begin(Goal::Sound);
        self.set_data(chr, service, x as f32 + 0.5, y as f32 + 0.5, z as f32);
    }

    /// `pathToNearest(locations)`: the first `x, y, z` triple is the
    /// target, the rest are alternatives (the pathfinder keeps the
    /// shortest).
    pub fn path_to_nearest(
        &mut self,
        chr: &mut dyn PathMover,
        service: &mut PathfindService,
        locations: &[f32],
    ) -> Result<(), PathArgumentError> {
        if locations.is_empty() {
            return Err(PathArgumentError::Empty);
        }
        if !locations.len().is_multiple_of(3) {
            return Err(PathArgumentError::NotTriples);
        }
        self.begin(Goal::Location);
        self.set_data(chr, service, locations[0], locations[1], locations[2]);
        self.target_xyz.extend_from_slice(&locations[3..]);
        Ok(())
    }

    /// `pathToCharacter(target)`.
    pub fn path_to_character<G: GetSquare>(
        &mut self,
        chr: &mut dyn PathMover,
        ctx: &mut PathContext<'_, '_, '_, G>,
        target_id: MoverId,
        target: &dyn CharacterView,
    ) {
        self.begin(Goal::Character);
        self.goal_character = Some(target_id);
        if let Some(v) = target.vehicle() {
            let vz = ff(ctx.vehicles.position(v).2) as f32;
            if let Some(p) = ctx.vehicles.choose_best_attack_position(v, target, chr) {
                self.set_data(chr, ctx.service, p.0, p.1, vz);
                return;
            }
            let (vx, vy, _) = ctx.vehicles.position(v);
            self.set_data(chr, ctx.service, vx, vy, vz);
            if ctx
                .vehicles
                .closest_point_on_poly_dist_sq(v, chr.x(), chr.y())
                < 100.0
            {
                if let Some(z) = chr.as_zombie_mut() {
                    z.set_allow_repath_delay(100.0);
                }
                chr.set_path_find_progress(PathFindProgress::Failed);
            }
        }
        let map = ctx.map;
        let target_sq = target
            .current_square()
            .and_then(|(x, y, z)| map.square(x, y, z));
        if let Some(tsq) = target_sq.filter(|s| {
            target.is_sitting_on_furniture()
                && (s.is_solid(map.sprites()) || s.is_solid_trans(map.sprites()))
        }) {
            let dir = target.sit_on_furniture_direction();
            let x = ff(target.x());
            let y = ff(target.y());
            let z = ff(target.z());
            let cand = match dir {
                Some(IsoDirections::N) => Some((x as f32 + 0.5, y as f32 - 0.3)),
                Some(IsoDirections::S) => Some((x as f32 + 0.5, (y + 1) as f32 + 0.3)),
                Some(IsoDirections::W) => Some((x as f32 - 0.3, y as f32 + 0.5)),
                Some(IsoDirections::E) => Some(((x + 1) as f32 + 0.3, y as f32 + 0.5)),
                // DebugType.General.warn("unhandled sitting direction")
                _ => None,
            };
            if let Some((cx, cy)) = cand {
                let sq = map.square(ff(cx), ff(cy), z);
                if self.is_good_chair_adjacent_square(map, tsq, sq) {
                    self.set_data(chr, ctx.service, cx, cy, z as f32);
                    return;
                }
            }
            let mut test_squares = Vec::new();
            for i in 0..8usize {
                let d = IsoDirections::ALL[i];
                if Some(d) == dir {
                    continue;
                }
                let ts = tsq.get_adjacent_square(d, map.bits.squares);
                if self.is_good_chair_adjacent_square(map, tsq, ts) {
                    test_squares.push(ts.unwrap());
                }
            }
            let mut closest_distance = 0.0f32;
            let mut closest: Option<&IsoGridSquare> = None;
            for ts in test_squares {
                // IsoMovingObject.DistTo(int, int): Manhattan.
                let d = (chr.x() - ts.x as f32).abs() + (chr.y() - ts.y as f32).abs();
                if closest_distance != 0.0
                    && (!(d < closest_distance)
                        || !self.is_good_chair_adjacent_square(map, tsq, Some(ts)))
                {
                    continue;
                }
                closest_distance = d;
                closest = Some(ts);
            }
            if let Some(c) = closest {
                self.set_data(
                    chr,
                    ctx.service,
                    c.x as f32 + 0.5,
                    c.y as f32 + 0.5,
                    c.z as f32,
                );
                return;
            }
        }
        self.set_data(chr, ctx.service, target.x(), target.y(), target.z());
    }

    /// `isGoodChairAdjacentSquare(target, adjacent)`.
    pub fn is_good_chair_adjacent_square<G: GetSquare>(
        &self,
        map: &MainThreadMap<'_, '_, G>,
        target: &IsoGridSquare,
        adjacent: Option<&IsoGridSquare>,
    ) -> bool {
        let Some(a) = adjacent else { return false };
        let water = map.bits.t.water;
        !a.is_solid(map.sprites())
            && !a.is_solid_trans(map.sprites())
            && (!a.has(water) || a.has_floor_over_water(map.bits.sprites))
            && a.can_reach_to(target, map.sprites(), map.squares(), map.hooks())
    }

    /// `pathToSitOnFurniture(furniture, bAnySpriteGridObject)`.
    pub fn path_to_sit_on_furniture<G: GetSquare>(
        &mut self,
        chr: &mut dyn PathMover,
        ctx: &mut PathContext<'_, '_, '_, G>,
        furniture: ObjectRef,
        any_sprite_grid_object: bool,
    ) {
        let mut locations = Vec::new();
        let mut objects = Vec::new();
        if any_sprite_grid_object {
            objects = ctx.seating.sprite_grid_objects_excluding_self(furniture);
        }
        objects.push(furniture);
        for o in &objects {
            self.path_to_sit_on_furniture_no_sprite_grid(chr, ctx, *o, &mut locations);
        }
        self.begin(Goal::SitOnFurniture);
        self.goal_sit_on_furniture_object = Some(furniture);
        self.goal_sit_on_furniture_any_sprite_grid_object = any_sprite_grid_object;
        if locations.is_empty() {
            let (x, y, z) = (chr.x(), chr.y(), chr.z());
            self.set_data(chr, ctx.service, x, y, z);
            chr.set_path_find_progress(PathFindProgress::Failed);
            return;
        }
        self.set_data(chr, ctx.service, locations[0], locations[1], locations[2]);
        self.target_xyz.extend_from_slice(&locations[3..]);
    }

    fn path_to_sit_on_furniture_no_sprite_grid<G: GetSquare>(
        &self,
        chr: &mut dyn PathMover,
        ctx: &PathContext<'_, '_, '_, G>,
        furniture: ObjectRef,
        locations: &mut Vec<f32>,
    ) {
        let map = ctx.map;
        let Some(square) = map.square(furniture.x, furniture.y, furniture.z) else {
            return;
        };
        let blocked = |r: TestResults| {
            r == TestResults::Blocked
                || r == TestResults::ClearThroughClosedDoor
                || r == TestResults::ClearThroughWindow
        };
        for (di, direction) in DIRECTIONS.iter().enumerate() {
            for (si, side) in SIDES.iter().enumerate() {
                let Some(mut wp) = ctx
                    .seating
                    .adjacent_position(chr, furniture, direction, side)
                else {
                    continue;
                };
                let solid = square.is_solid(map.sprites()) || square.is_solid_trans(map.sprites());
                let in_square = is_point_in_square(wp.0, wp.1, square.x, square.y);
                if solid && in_square {
                    let (ox, oy) = (wp.0, wp.1);
                    let (sx, sy) = (square.x, square.y);
                    match (di, si) {
                        (0, 0) => wp.1 = sy as f32 - 0.3,
                        (0, 1) => wp.0 = sx as f32 - 0.3,
                        (0, 2) => wp.0 = (sx + 1) as f32 + 0.3,
                        (1, 0) => wp.1 = (sy + 1) as f32 + 0.3,
                        (1, 1) => wp.0 = (sx + 1) as f32 + 0.3,
                        (1, 2) => wp.0 = sx as f32 - 0.3,
                        (2, 0) => wp.0 = sx as f32 - 0.3,
                        (2, 1) => wp.1 = (sy + 1) as f32 + 0.3,
                        (2, 2) => wp.1 = sy as f32 - 0.3,
                        (3, 0) => wp.0 = (sx + 1) as f32 + 0.3,
                        (3, 1) => wp.1 = sy as f32 - 0.3,
                        _ => wp.1 = (sy + 1) as f32 + 0.3,
                    }
                    let r = line_clear(
                        &map.vision_env(),
                        ff(wp.0),
                        ff(wp.1),
                        ff(wp.2),
                        ff(ox),
                        ff(oy),
                        ff(wp.2),
                        false,
                    );
                    if blocked(r) {
                        continue;
                    }
                } else if !in_square {
                    let r = line_clear(
                        &map.vision_env(),
                        ff(wp.0),
                        ff(wp.1),
                        ff(wp.2),
                        square.x,
                        square.y,
                        square.z,
                        false,
                    );
                    if blocked(r) {
                        continue;
                    }
                }
                // IsoGameCharacter.canStandAt(x, y, z): flags 17.
                if !(solid
                    || ctx
                        .lcc()
                        .can_stand_at_old(wp.0, wp.1, ff(wp.2) as f32, None, 17))
                {
                    continue;
                }
                locations.extend_from_slice(&[wp.0, wp.1, wp.2]);
            }
        }
    }

    /// `fixSitOnFurniturePath(targetX, targetY)`: end the path at the
    /// closest seat position when it is within one square of the target.
    fn fix_sit_on_furniture_path(
        &mut self,
        chr: &dyn PathMover,
        seating: &dyn SeatingHooks,
        tx: f32,
        ty: f32,
    ) {
        let Some(goal) = self.goal_sit_on_furniture_object else {
            return;
        };
        if !seating.object_exists(goal) {
            return;
        }
        let mut closest = (0.0f32, 0.0f32, 0.0f32);
        let mut closest_dist_sq = f32::MAX;
        let mut objects = Vec::new();
        if self.goal_sit_on_furniture_any_sprite_grid_object {
            objects = seating.sprite_grid_objects_excluding_self(goal);
        }
        objects.push(goal);
        let mut closest_object = goal;
        for o in &objects {
            for d in DIRECTIONS {
                for s in SIDES {
                    let Some(wp) = seating.adjacent_position(chr, *o, d, s) else {
                        continue;
                    };
                    let dist_sq = distance_to_squared(tx, ty, wp.0, wp.1);
                    if dist_sq < closest_dist_sq {
                        closest = wp;
                        closest_dist_sq = dist_sq;
                        closest_object = *o;
                    }
                }
            }
        }
        if closest_dist_sq > 1.0 {
            return;
        }
        self.goal_sit_on_furniture_object = Some(closest_object);
        if distance_to_squared(closest.0, closest.1, tx, ty) > 0.002_500_000_2f32 {
            self.path.add_node(closest.0, closest.1, closest.2, 0);
            self.target_x = closest.0;
            self.target_y = closest.1;
        }
    }

    /// `pathToVehicleAdjacent(vehicle)`: the four sides of the vehicle, the
    /// shortest reachable one wins. As in Java, a vehicle with no free side
    /// throws (`TFloatArrayList.get(0)` on an empty list) after the goal is
    /// set; the port stops there and returns `false`.
    pub fn path_to_vehicle_adjacent<G: GetSquare>(
        &mut self,
        chr: &mut dyn PathMover,
        ctx: &mut PathContext<'_, '_, '_, G>,
        vehicle: i16,
    ) -> bool {
        self.begin(Goal::VehicleAdjacent);
        self.goal_vehicle = Some(vehicle);
        let ext = ctx.vehicles.extents(vehicle);
        let com = ctx.vehicles.center_of_mass_offset(vehicle);
        let width = ext.0;
        let length = ext.2;
        let min_x = com.0 - width / 2.0 - 0.3;
        let min_y = com.2 - length / 2.0 - 0.3;
        let max_x = com.0 + width / 2.0 + 0.3;
        let max_y = com.2 + length / 2.0 + 0.3;
        let mut locations = Vec::new();
        let tz = ff(self.target_z);
        for (lx, ly, lz) in [
            (min_x, com.1, com.2),
            (max_x, com.1, com.2),
            (com.0, com.1, min_y),
            (com.0, com.1, max_y),
        ] {
            let v = ctx.vehicles.world_pos(vehicle, lx, ly, lz);
            if ctx
                .lcc()
                .can_stand_at(v.0, v.1, tz, Some(vehicle), false, true)
            {
                locations.extend_from_slice(&[v.0, v.1, self.target_z]);
            }
        }
        if locations.is_empty() {
            return false;
        }
        self.set_data(chr, ctx.service, locations[0], locations[1], locations[2]);
        self.target_xyz.extend_from_slice(&locations[3..]);
        true
    }

    /// The direct two-node path players get when the target is in sight on
    /// their level.
    fn direct_path(&mut self, chr: &mut dyn PathMover) {
        self.path.clear();
        self.path.add_node(chr.x(), chr.y(), chr.z(), 0);
        self.path
            .add_node(self.target_x, self.target_y, self.target_z, 0);
        chr.set_path_find_progress(PathFindProgress::Found);
    }

    /// `pathToVehicleArea(vehicle, areaId)`.
    pub fn path_to_vehicle_area<G: GetSquare>(
        &mut self,
        chr: &mut dyn PathMover,
        ctx: &mut PathContext<'_, '_, '_, G>,
        vehicle: i16,
        area: &str,
    ) {
        let Some(center) = ctx.vehicles.area_center(vehicle, area) else {
            self.target_x = chr.x();
            self.target_y = chr.y();
            self.target_z = chr.z();
            chr.set_path_find_progress(PathFindProgress::Failed);
            return;
        };
        self.begin(Goal::VehicleArea);
        self.goal_vehicle = Some(vehicle);
        self.goal_vehicle_area = Some(area.to_string());
        let vz = ff(ctx.vehicles.position(vehicle).2) as f32;
        self.set_data(chr, ctx.service, center.0, center.1, vz);
        if chr.as_player().is_some()
            && ff(chr.z()) == ff(self.target_z)
            && !ctx.lcc().line_clear_collide(
                chr.x(),
                chr.y(),
                self.target_x,
                self.target_y,
                ff(self.target_z),
                None,
                true,
                true,
            )
        {
            self.direct_path(chr);
        }
    }

    /// `pathToVehicleSeat(vehicle, seat)`.
    pub fn path_to_vehicle_seat<G: GetSquare>(
        &mut self,
        chr: &mut dyn PathMover,
        ctx: &mut PathContext<'_, '_, '_, G>,
        vehicle: i16,
        seat: i32,
    ) {
        let vs = ctx.vehicles;
        if let Some(posn) = vs.passenger_position(vehicle, seat, "outside2") {
            let mut wp = match &posn.area {
                None => vs.passenger_position_world_pos(vehicle, seat, "outside2"),
                Some(a) => {
                    let p = vs.area_position_world_4_player_interact(vehicle, a);
                    (p.0, p.1, 0.0)
                }
            };
            wp = (wp.0 - chr.x(), wp.1 - chr.y(), wp.2 - chr.z());
            if v3_len(wp.0, wp.1, wp.2) < 2.0 {
                let wp = vs.passenger_position_world_pos(vehicle, seat, "outside2");
                self.set_data(chr, ctx.service, wp.0, wp.1, ff(wp.2) as f32);
                if chr.as_player().is_some() && ff(chr.z()) == ff(self.target_z) {
                    self.direct_path(chr);
                    return;
                }
            }
        }
        let Some(posn) = vs.passenger_position(vehicle, seat, "outside") else {
            match vs.passenger_door_area(vehicle, seat) {
                None => {
                    self.target_x = chr.x();
                    self.target_y = chr.y();
                    self.target_z = chr.z();
                    chr.set_path_find_progress(PathFindProgress::Failed);
                }
                Some(area) => self.path_to_vehicle_area(chr, ctx, vehicle, &area),
            }
            return;
        };
        self.begin(Goal::VehicleSeat);
        self.goal_vehicle = Some(vehicle);
        self.goal_vehicle_seat = seat;
        let wp = match &posn.area {
            None => vs.passenger_position_world_pos(vehicle, seat, "outside"),
            Some(a) => {
                let p = vs.area_position_world_4_player_interact(vehicle, a);
                (
                    p.0,
                    p.1,
                    ff(vs.transform_origin_y(vehicle) / 2.449_49) as f32,
                )
            }
        };
        self.set_data(chr, ctx.service, wp.0, wp.1, ff(wp.2) as f32);
        if chr.as_player().is_some()
            && ff(chr.z()) == ff(self.target_z)
            && !ctx.lcc().line_clear_collide(
                chr.x(),
                chr.y(),
                self.target_x,
                self.target_y,
                ff(self.target_z),
                None,
                true,
                true,
            )
        {
            self.direct_path(chr);
        }
    }

    /// `getGrabCorpseLocations(body, out)`.
    pub fn grab_corpse_locations(body: &dyn DeadBodyView) -> Vec<(f32, f32)> {
        let mut out = Vec::new();
        let (bx, by, _) = body.position();
        if body.is_animal() {
            let collision_size =
                body.animal_collision_size().map_or(1.0, |c| c.max(0.2)) * body.animal_size();
            let radius = collision_size as f64 * 2.0;
            for i in 0..12 {
                let angle = (i as f64 * 360.0 / 12.0).to_radians();
                let cx = bx as f64 + radius * angle.cos();
                let cy = by as f64 + radius * angle.sin();
                out.push((cx as f32, cy as f32));
            }
            return out;
        }
        let head = body.grab_head_position();
        if body.can_be_grabbed_from(head.0, head.1) {
            out.push(head);
        }
        let legs = body.grab_legs_position();
        if body.can_be_grabbed_from(legs.0, legs.1) {
            out.push(legs);
        }
        if out.is_empty() && body.can_be_grabbed_from(bx, by) {
            out.push((bx, by));
        }
        out
    }

    /// `pathToGrabCorpse(body)`. Returns `false` when there is no point to
    /// grab from (Java logs an error and does nothing).
    pub fn path_to_grab_corpse(
        &mut self,
        chr: &mut dyn PathMover,
        service: &mut PathfindService,
        body: &dyn DeadBodyView,
    ) -> bool {
        let pts = Self::grab_corpse_locations(body);
        let Some(first) = pts.first().copied() else {
            return false;
        };
        let z = body.position().2;
        self.begin(Goal::GrabCorpse);
        self.set_data(chr, service, first.0, first.1, z);
        for p in &pts[1..] {
            self.target_xyz.extend_from_slice(&[p.0, p.1, z]);
        }
        true
    }

    // ----- results -----

    /// `IPathfinder.Succeeded(path, mover)`.
    pub fn succeeded(&mut self, chr: &mut dyn PathMover, seating: &dyn SeatingHooks, path: &Path) {
        self.path = path.clone();
        if !self.is_cancel {
            chr.set_path2(Some(&self.path));
        }
        if let Some(node) = path.nodes.last().copied() {
            self.target_x = node.x;
            self.target_y = node.y;
            self.target_z = node.z;
            if self.is_goal_sit_on_furniture() {
                self.fix_sit_on_furniture_path(chr, seating, self.target_x, self.target_y);
            }
        }
        chr.set_path_find_progress(PathFindProgress::Found);
    }

    /// `IPathfinder.Failed(mover)`.
    pub fn failed(&mut self, chr: &mut dyn PathMover) {
        chr.set_path_find_progress(PathFindProgress::Failed);
    }

    // ----- per frame -----

    /// `update()`.
    pub fn update<G: GetSquare>(
        &mut self,
        chr: &mut dyn PathMover,
        ctx: &mut PathContext<'_, '_, '_, G>,
    ) -> BehaviorResult {
        self.update_speed(chr, ctx, 1.0)
    }

    /// `update(speedMul)`: request a path if none is running, otherwise
    /// walk along it. `Succeeded` once at the end.
    pub fn update_speed<G: GetSquare>(
        &mut self,
        chr: &mut dyn PathMover,
        ctx: &mut PathContext<'_, '_, '_, G>,
        speed_mul: f32,
    ) -> BehaviorResult {
        match chr.path_find_progress() {
            PathFindProgress::NotRunning => {
                let snap = MoverSnapshot::capture(chr, ctx.world);
                ctx.service.add_request(
                    self.finder,
                    snap,
                    (self.start_x, self.start_y, self.start_z),
                    (self.target_x, self.target_y, self.target_z),
                    &self.target_xyz,
                );
                chr.set_path_find_progress(PathFindProgress::NotYetFound);
                self.walking_on_the_spot.reset(chr.x(), chr.y());
                self.update_while_running_pathfind(chr, ctx.world);
                return BehaviorResult::Working;
            }
            PathFindProgress::NotYetFound => {
                self.update_while_running_pathfind(chr, ctx.world);
                return BehaviorResult::Working;
            }
            PathFindProgress::Failed => return BehaviorResult::Failed,
            PathFindProgress::Found => {}
        }
        let state = chr.current_state();
        if matches!(
            state,
            CharacterState::ClimbOverFence
                | CharacterState::ClimbThroughWindow
                | CharacterState::PlayerSitOnFurniture
        ) {
            if ctx.world.is_client() && chr.as_player().is_some_and(|p| !p.is_local_player()) {
                let (dx, dy) = chr.deferred_movement();
                chr.move_unmodded(dx, dy);
            }
            return BehaviorResult::Working;
        }
        if chr.vehicle().is_some() {
            return BehaviorResult::Failed;
        }
        if self.walking_on_the_spot.check(chr, ctx.world) {
            return BehaviorResult::Failed;
        }
        chr.set_moving(true);
        chr.set_path2(Some(&self.path));
        let is_zombie = chr.is_zombie();
        if self.goal == Goal::Character && is_zombie {
            let goal_vehicle = self
                .goal_character
                .and_then(|id| ctx.characters.character(id))
                .and_then(|t| t.vehicle().map(|v| (t, v)));
            if let Some((target, v)) = goal_vehicle {
                if distance_to_squared(self.target_x, self.target_y, chr.x(), chr.y()) < 16.0 {
                    let Some(p) = ctx.vehicles.choose_best_attack_position(v, target, chr) else {
                        return BehaviorResult::Failed;
                    };
                    if (p.0 - self.target_x).abs() > 0.1 || (p.1 - self.target_y).abs() > 0.1 {
                        if ctx.vehicles.current_speed_km_hour(v).abs() > 0.8 {
                            // lineClearCollide(..., goalCharacter): the
                            // goal character's vehicle is ignored.
                            if !ctx.lcc().line_clear_collide(
                                chr.x(),
                                chr.y(),
                                p.0,
                                p.1,
                                ff(self.target_z),
                                Some(v),
                                true,
                                true,
                            ) {
                                self.path.clear();
                                self.path.add_node(chr.x(), chr.y(), chr.z(), 0);
                                self.path.add_node(p.0, p.1, p.2, 0);
                            } else if distance_to_squared(p.0, p.1, self.target_x, self.target_y)
                                > distance_to_squared(chr.x(), chr.y(), p.0, p.1)
                            {
                                return BehaviorResult::Working;
                            }
                        } else {
                            let z = chr.as_zombie_mut().unwrap();
                            if z.allow_repath_delay() <= 0.0 {
                                z.set_allow_repath_delay(6.25);
                                if ctx.lcc().line_clear_collide(
                                    chr.x(),
                                    chr.y(),
                                    p.0,
                                    p.1,
                                    ff(self.target_z),
                                    None,
                                    true,
                                    true,
                                ) {
                                    let tz = self.target_z;
                                    self.set_data(chr, ctx.service, p.0, p.1, tz);
                                    return BehaviorResult::Working;
                                }
                                self.path.clear();
                                self.path.add_node(chr.x(), chr.y(), chr.z(), 0);
                                self.path.add_node(p.0, p.1, p.2, 0);
                            }
                        }
                    }
                }
            }
        }
        if self.path.nodes.len() < 2 {
            // Java indexes the empty path and throws.
            return BehaviorResult::Failed;
        }
        let mut pop = PointOnPath::default();
        closest_point_on_path(
            ctx.map,
            chr.x(),
            chr.y(),
            chr.z(),
            Some(&mut *chr),
            &self.path,
            &mut pop,
        );
        self.point_on_path = pop;
        self.path_index = pop.path_index;
        let n = self.path.nodes.len() as i32;
        if self.path_index == n - 2 {
            let node = self.path.nodes[(n - 1) as usize];
            let dist_to_end = distance_to(chr.x(), chr.y(), node.x, node.y);
            if dist_to_end <= 0.05 {
                let (mx, my) = chr.deferred_movement();
                let length_test = chr.as_animal().map_or(0.0, |a| a.animal_size());
                if v_len(mx, my) > length_test {
                    if is_zombie || chr.as_player().is_some() {
                        chr.set_moving(false);
                    }
                    let (vx, vy) = v_set_length(
                        node.x - chr.x(),
                        node.y - chr.y(),
                        dist_to_end.min(0.005 * ctx.world.game_time_multiplier()),
                    );
                    chr.move_unmodded(vx, vy);
                    self.stopping = true;
                    return BehaviorResult::Working;
                }
                self.path_next_is_set = false;
                return BehaviorResult::Succeeded;
            }
            self.stopping = false;
        } else if self.path_index < n - 2 && pop.dist > 0.999 {
            self.path_index += 1;
        }
        let v1 = self.path.nodes[self.path_index as usize];
        let v2 = self.path.nodes[self.path_index as usize + 1];
        self.path_next_x = v2.x;
        self.path_next_y = v2.y;
        self.path_next_is_set = true;
        let (dir_x, dir_y) = normalize(self.path_next_x - chr.x(), self.path_next_y - chr.y());
        if chr.is_performing_no_aim_short_strafe() {
            chr.set_strafe_state_speed(1.0);
            chr.set_variable_float("StrafeSpeed", 1.0);
        }
        let (mut mx, mut my) = chr.deferred_movement();
        if !ctx.world.is_server() && !chr.is_animation_updating_this_frame() {
            mx = 0.0;
            my = 0.0;
        }
        let mut speed = v_len(mx, my);
        speed *= speed_mul;
        if let Some(z) = chr.as_zombie_mut() {
            z.set_running(false);
            if ctx.world.lore_speed() == 1 {
                z.set_running(true);
            }
        }
        let dist = speed * 1.0;
        let dist_to = distance_to(self.path_next_x, self.path_next_y, chr.x(), chr.y());
        if dist >= dist_to {
            speed *= dist_to / dist;
            self.path_index += 1;
        }
        if is_zombie {
            self.check_crawling_transition(chr, ctx, v1, v2, dist_to);
        }
        if !is_zombie && dist_to >= 0.5 {
            let step = 0.5f32.max(speed);
            let (nx, ny, nz) = (chr.x() + dir_x * step, chr.y() + dir_y * step, chr.z());
            if self.check_door_hoppable_window(chr, ctx, nx, ny, nz) {
                return BehaviorResult::Failed;
            }
            if state != chr.current_state() {
                return BehaviorResult::Working;
            }
        }
        if speed <= 0.0 {
            self.walking_on_the_spot.reset(chr.x(), chr.y());
            return BehaviorResult::Working;
        }
        if self.should_be_moving(chr) {
            let (vx, vy) = v_set_length(dir_x, dir_y, speed);
            chr.move_unmodded(vx, vy);
            self.started_moving = true;
        }
        if self.is_strafing(chr) {
            if matches!(
                self.goal,
                Goal::VehicleAdjacent | Goal::VehicleArea | Goal::VehicleSeat
            ) {
                if let Some(v) = self.goal_vehicle {
                    chr.face_this_object(FaceTarget::Vehicle(v));
                }
            }
        } else if !chr.is_aiming() {
            if self.is_turning_to_obstacle() && chr.should_be_turning() {
                // keep turning
            } else if chr.is_animating_backwards() {
                let bx = chr.x() - self.path_next_x;
                let by = chr.y() - self.path_next_y;
                if bx * bx + by * by > 0.0 {
                    chr.direction_from_vector(bx, by);
                    let (nx, ny) = normalize(bx, by);
                    chr.set_forward_direction(nx, ny);
                    chr.update_animation_forward_direction();
                }
            } else {
                chr.face_location_f(self.path_next_x, self.path_next_y);
            }
        }
        BehaviorResult::Working
    }

    /// `updateWhileRunningPathfind()`: keep walking to the last path point
    /// while a new path is being searched.
    fn update_while_running_pathfind(
        &mut self,
        chr: &mut dyn PathMover,
        world: &dyn PathWorldHooks,
    ) {
        if !self.path_next_is_set {
            return;
        }
        let (x, y) = (self.path_next_x, self.path_next_y);
        self.move_to_point(chr, world, x, y, 1.0);
    }

    /// `moveToPoint(x, y, speedMul)`.
    pub fn move_to_point(
        &mut self,
        chr: &mut dyn PathMover,
        world: &dyn PathWorldHooks,
        x: f32,
        y: f32,
        speed_mul: f32,
    ) {
        if chr.as_player().is_some() && chr.current_state() == CharacterState::CollideWithWall {
            return;
        }
        let (dx, dy) = (x - chr.x(), y - chr.y());
        if ff(x) == ff(chr.x()) && ff(y) == ff(chr.y()) && v_len(dx, dy) <= 0.1 {
            return;
        }
        let (dir_x, dir_y) = normalize(dx, dy);
        let (mx, my) = chr.deferred_movement();
        let mut speed = v_len(mx, my);
        speed *= speed_mul;
        let mut remote_with_target = false;
        let is_client = world.is_client();
        let lunging = chr.is_current_state(CharacterState::LungeNetwork);
        if let Some(z) = chr.as_zombie_mut() {
            z.set_running(world.lore_speed() == 1);
            remote_with_target = is_client && z.is_remote_zombie() && z.has_target() && lunging;
        }
        if speed <= 0.0 {
            return;
        }
        let (vx, vy) = v_set_length(dir_x, dir_y, speed);
        chr.move_unmodded(vx, vy);
        if remote_with_target {
            return;
        }
        chr.face_location(x - 0.5, y - 0.5);
        chr.set_forward_direction(x - chr.x(), y - chr.y());
        chr.normalize_forward_direction();
    }

    /// `moveToDir(target, speedMul)`: walk straight at a moving object at
    /// `(tx, ty)`.
    pub fn move_to_dir(
        &mut self,
        chr: &mut dyn PathMover,
        world: &dyn PathWorldHooks,
        tx: f32,
        ty: f32,
        speed_mul: f32,
    ) {
        let (dx, dy) = (tx - chr.x(), ty - chr.y());
        if v_len(dx, dy) <= 0.1 {
            return;
        }
        let (dir_x, dir_y) = normalize(dx, dy);
        let (mx, my) = chr.deferred_movement();
        let mut speed = v_len(mx, my);
        speed *= speed_mul;
        if let Some(z) = chr.as_zombie_mut() {
            z.set_running(false);
            if world.lore_speed() == 1 {
                z.set_running(true);
            }
        }
        if speed <= 0.0 {
            return;
        }
        let (vx, vy) = v_set_length(dir_x, dir_y, speed);
        chr.move_unmodded(vx, vy);
        chr.face_location(tx - 0.5, ty - 0.5);
        chr.set_forward_direction(tx - chr.x(), ty - chr.y());
        chr.normalize_forward_direction();
    }

    /// `checkDoorHoppableWindow(nx, ny, z)`: open doors, climb windows and
    /// fences on the way into the next square. `true` fails the path.
    fn check_door_hoppable_window<G: GetSquare>(
        &mut self,
        chr: &mut dyn PathMover,
        ctx: &mut PathContext<'_, '_, '_, G>,
        nx: f32,
        ny: f32,
        z: f32,
    ) -> bool {
        self.turning_to_obstacle = false;
        let map = ctx.map;
        let Some(current) = chr
            .current_square()
            .and_then(|(x, y, z)| map.square(x, y, z))
        else {
            return false;
        };
        let Some(square) = map.square(ff(nx), ff(ny), ff(z)) else {
            return false;
        };
        if (square.x, square.y, square.z) == (current.x, current.y, current.z) {
            return false;
        }
        let dx = square.x - current.x;
        let dy = square.y - current.y;
        if dx != 0 && dy != 0 {
            return false;
        }
        if let Some(door) = map.door_to(current, square) {
            let open = match map.object(door).map(|o| &o.kind) {
                Some(ObjectKind::Door(d)) => d.open,
                Some(ObjectKind::Thumpable(t)) => t.open,
                _ => true,
            };
            if !open {
                let player_closed_recently = chr.as_animal().is_none()
                    && chr
                        .as_player()
                        .is_some_and(|p| p.time_since_close_door() < 50.0);
                if player_closed_recently {
                    chr.set_collidable(false);
                } else {
                    if !chr.could_open_door(door) {
                        chr.toggle_door(door);
                        return true;
                    }
                    chr.set_collidable(true);
                    if !chr.toggle_door(door) {
                        return true;
                    }
                }
            }
        }
        if let Some(window) = map.window_to(current, square) {
            let (smashed, glass_removed) = match map.object(window).map(|o| &o.kind) {
                Some(ObjectKind::Window(w)) => (w.destroyed, w.glass_removed),
                _ => (false, false),
            };
            if !chr.can_climb_through_window(window) || smashed && !glass_removed {
                return true;
            }
            if chr.is_aiming() {
                return false;
            }
            chr.face_this_object(FaceTarget::Object(window));
            if chr.should_be_turning() {
                self.turning_to_obstacle = true;
                return false;
            }
            chr.climb_through_window(window);
            return false;
        }
        if let Some(window) = map.window_thumpable_to(current, square) {
            if map.is_barricaded(window) {
                return true;
            }
            if chr.is_aiming() {
                return false;
            }
            chr.face_this_object(FaceTarget::Object(window));
            if chr.should_be_turning() {
                self.turning_to_obstacle = true;
                return false;
            }
            chr.climb_through_window(window);
            return false;
        }
        if let Some(frame) = map.window_frame_to(current, square) {
            chr.climb_through_window_frame(frame);
            return false;
        }
        let t = &map.bits.t;
        let climb_dir = if dx > 0 && square.has(t.HoppableW) {
            Some(IsoDirections::E)
        } else if dx < 0 && current.has(t.HoppableW) {
            Some(IsoDirections::W)
        } else if dy < 0 && current.has(t.HoppableN) {
            Some(IsoDirections::N)
        } else if dy > 0 && square.has(t.HoppableN) {
            Some(IsoDirections::S)
        } else {
            None
        };
        if let Some(d) = climb_dir {
            if chr.is_aiming() {
                return false;
            }
            chr.face_direction(d);
            if chr.should_be_turning() {
                self.turning_to_obstacle = true;
                return false;
            }
            chr.climb_over_fence(d);
        }
        let tall_w =
            |s: &IsoGridSquare| s.has(t.TallHoppableW) || s.has(t.WallW) || s.has(t.WallWTrans);
        let tall_n =
            |s: &IsoGridSquare| s.has(t.TallHoppableN) || s.has(t.WallN) || s.has(t.WallNTrans);
        let climb_dir = if dx > 0 && tall_w(square) {
            Some(IsoDirections::E)
        } else if dx < 0 && tall_w(current) {
            Some(IsoDirections::W)
        } else if dy < 0 && tall_n(current) {
            Some(IsoDirections::N)
        } else if dy > 0 && tall_n(square) {
            Some(IsoDirections::S)
        } else {
            None
        };
        if let Some(d) = climb_dir {
            if let Some(p) = chr.as_player_mut() {
                p.climb_over_wall(d);
                return false;
            }
        }
        false
    }

    /// `checkCrawlingTransition(v1, v2, distTo)`: tell a zombie to get
    /// down before a crawl-only stretch, or to stand up after one.
    fn check_crawling_transition<G: GetSquare>(
        &mut self,
        chr: &mut dyn PathMover,
        ctx: &mut PathContext<'_, '_, '_, G>,
        mut v1: crate::path::PathNode,
        mut v2: crate::path::PathNode,
        mut dist_to: f32,
    ) {
        let n = self.path.nodes.len() as i32;
        if self.path_index < n - 2 {
            v1 = self.path.nodes[self.path_index as usize];
            v2 = self.path.nodes[self.path_index as usize + 1];
            dist_to = distance_to(v2.x, v2.y, chr.x(), chr.y());
        }
        let Some(z) = chr.as_zombie() else { return };
        if z.is_crawling() {
            if !z.is_can_walk() {
                return;
            }
            if chr.previous_state() == CharacterState::ZombieGetDown
                && z.get_down_is_near_start_xy()
            {
                return;
            }
            let mut pop = PointOnPath::default();
            let (x, y, cz) = (chr.x(), chr.y(), chr.z());
            advance_along_path(
                ctx.map,
                x,
                y,
                cz,
                0.5,
                Some(&mut *chr),
                &self.path,
                &mut pop,
            );
            self.point_on_path = pop;
            let lcc = ctx.lcc();
            if !lcc.can_stand_at(pop.x, pop.y, ff(chr.z()), None, false, true) {
                return;
            }
            if !v2.has_flag(1) && lcc.can_stand_at(chr.x(), chr.y(), ff(chr.z()), None, false, true)
            {
                chr.set_variable_bool("ShouldStandUp", true);
            }
        } else {
            if v1.has_flag(1) && v2.has_flag(1) {
                chr.set_variable_bool("ShouldBeCrawling", true);
                if let Some(z) = chr.as_zombie_mut() {
                    z.get_down_set_params();
                }
                return;
            }
            if dist_to < 0.4 && !v1.has_flag(1) && v2.has_flag(1) {
                chr.set_variable_bool("ShouldBeCrawling", true);
                if let Some(z) = chr.as_zombie_mut() {
                    z.get_down_set_params();
                }
            }
        }
    }
}

/// `isPointInSquare(x, y, squareX, squareY)`.
fn is_point_in_square(x: f32, y: f32, sx: i32, sy: i32) -> bool {
    x >= sx as f32 && x < sx as f32 + 1.0 && y >= sy as f32 && y < (sy + 1) as f32
}
