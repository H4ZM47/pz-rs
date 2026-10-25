//! `zombie.pathfind.CollideWithObstacles`: the main-thread collision of a
//! moving character against the vehicles around it, behind
//! `PolygonalMap2.resolveCollision`.
//!
//! Both entry points of the game go through this class (not
//! `CollideWithObstaclesPoly`, which nothing calls with the default debug
//! options):
//!
//! * `IsoMovingObject.postupdate` calls `resolveCollision(chr, nextX,
//!   nextY, out)` ([`CollideWithObstacles::resolve_collision`]) and sets
//!   `collidedWithVehicle` when the point it gets back differs from
//!   `(nextX, nextY)`;
//! * `IsoGameCharacter.slideAwayFromWalls` (with
//!   `DebugOptions.collideWithObstacles.debug.slideAwayFromWalls`, on by
//!   default) calls `resolveCollision(chr, radius, newPos)`
//!   ([`CollideWithObstacles::resolve_collision_radius`]).
//!
//! Obstacles are only built when a vehicle (`getPolyPlusRadius()`) is near
//! the move. Then the walls, windows, closed doors, stairs and solid squares
//! of the 4×4 squares around the character are traced into outlines 0.3
//! out from the square edges, the vehicle polygons and outlines are split
//! where they cross, and the move is clipped against the nearest edge it
//! runs into. Without a vehicle nearby the answer is always the move
//! itself.
//!
//! The Java object is a singleton (`PolygonalMap2.instance
//! .collideWithObstacles`) that keeps `move` and `nodeNormal` from one call
//! to the next, and the radius overload reads both without setting them
//! first; [`CollideWithObstacles`] keeps them the same way, so one instance
//! must serve the whole world in call order. The debug-render paths
//! (`Core.debug` and `collideWithObstacles.render.*`, off by default) only
//! draw, apart from `PolygonalMap2.render`, which is debug-only.
//!
//! The arithmetic is the Java code's: float expressions widened to double
//! where Java widens them, no fused multiply-adds, `Vector2.normalize` and
//! `Vector2.rotate`.

// The negated comparisons are Java's (false for NaN).
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::manual_range_contains)]

use pz_world::chunk::IsoGridSquare;
use pz_world::{EdgeFacing, GetSquare};

use super::{ceil_i, fastfloor, normalize, MainThreadMap};
use crate::mover::CharacterView;

/// `CollideWithObstacles.RADIUS`: how far outlines stand off square edges.
const RADIUS: f32 = 0.3;

/// `(float)Math.toRadians(90.0)`.
fn rad90() -> f32 {
    90.0f64.to_radians() as f32
}

/// `Vector2.rotate((float)Math.toRadians(90.0))`.
fn rotate90(x: f32, y: f32) -> (f32, f32) {
    let r = rad90() as f64;
    let (c, s) = (r.cos(), r.sin());
    let rx = x as f64 * c - y as f64 * s;
    let ry = x as f64 * s + y as f64 * c;
    (rx as f32, ry as f32)
}

/// `Math.min(float, float)`.
fn jmin(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        return b;
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// `Math.max(float, float)`.
fn jmax(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

#[inline]
fn dot(a: (f32, f32), b: (f32, f32)) -> f32 {
    a.0 * b.0 + a.1 * b.1
}

/// `ImmutableRectF`.
#[derive(Clone, Copy, Debug, Default)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn right(&self) -> f32 {
        self.x + self.w
    }
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
    fn contains_point(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && self.right() > o.x && self.y < o.bottom() && self.bottom() > o.y
    }
}

type NodeId = usize;
type EdgeId = usize;
type ObstacleId = usize;
type IntersectionId = usize;

/// `CCNode`.
#[derive(Debug)]
struct Node {
    x: f32,
    y: f32,
    z: i32,
    ignore: bool,
    edges: Vec<EdgeId>,
    visible: Vec<NodeId>,
}

/// `CCEdge`.
#[derive(Debug)]
struct Edge {
    node1: NodeId,
    node2: NodeId,
    obstacle: ObstacleId,
    intersections: Vec<IntersectionId>,
    normal: (f32, f32),
}

impl Edge {
    fn has_node(&self, n: NodeId) -> bool {
        n == self.node1 || n == self.node2
    }
}

/// `CCObstacle`.
#[derive(Debug, Default)]
struct Obstacle {
    edges: Vec<EdgeId>,
    bounds: Rect,
}

/// `CCIntersection`.
#[derive(Debug)]
struct Intersection {
    edge1: EdgeId,
    edge2: EdgeId,
    dist1: f32,
    dist2: f32,
    node_split: NodeId,
}

/// `CCObjectOutline`: the corner (north-west) and the west and north
/// edges of one square of the 5×5 grid around the character.
#[derive(Clone, Copy, Debug, Default)]
struct Outline {
    x: i32,
    y: i32,
    z: i32,
    nw: bool,
    nw_w: bool,
    nw_n: bool,
    nw_e: bool,
    nw_s: bool,
    w_w: bool,
    w_e: bool,
    w_cutoff: bool,
    n_n: bool,
    n_s: bool,
    n_cutoff: bool,
}

/// The side of the grid (`oo = new CCObjectOutline[5][5]`).
const OO: i32 = 5;

/// One call's obstacle graph: the pooled objects of the Java class as
/// arenas, and its `nodes`, `obstacles` and `intersections` lists.
#[derive(Debug, Default)]
struct Graph {
    node_arena: Vec<Node>,
    edges: Vec<Edge>,
    obstacle_arena: Vec<Obstacle>,
    intersection_arena: Vec<Intersection>,
    /// `this.nodes`.
    nodes: Vec<NodeId>,
    /// `this.obstacles`.
    obstacles: Vec<ObstacleId>,
    /// `this.intersections`.
    intersections: Vec<IntersectionId>,
    /// `oo`, indexed `[x * 5 + y]`.
    oo: [Outline; 25],
    /// `obstacleTraceNodes` (the `nodes` list every outline of a trace
    /// shares).
    trace_nodes: Vec<NodeId>,
}

impl Graph {
    fn clear(&mut self) {
        self.node_arena.clear();
        self.edges.clear();
        self.obstacle_arena.clear();
        self.intersection_arena.clear();
        self.nodes.clear();
        self.obstacles.clear();
        self.intersections.clear();
        self.trace_nodes.clear();
    }

    /// `CCNode.alloc().init(x, y, z)`.
    fn node(&mut self, x: f32, y: f32, z: i32) -> NodeId {
        self.node_arena.push(Node {
            x,
            y,
            z,
            ignore: false,
            edges: Vec::new(),
            visible: Vec::new(),
        });
        self.node_arena.len() - 1
    }

    /// `CCObstacle.alloc().init()`.
    fn obstacle(&mut self) -> ObstacleId {
        self.obstacle_arena.push(Obstacle::default());
        self.obstacle_arena.len() - 1
    }

    /// `CCEdge.alloc().init(node1, node2, obstacle)`.
    fn edge(&mut self, n1: NodeId, n2: NodeId, obstacle: ObstacleId) -> EdgeId {
        let id = self.edges.len();
        self.node_arena[n1].edges.push(id);
        self.node_arena[n2].edges.push(id);
        let (a, b) = (&self.node_arena[n1], &self.node_arena[n2]);
        let (nx, ny) = normalize(b.x - a.x, b.y - a.y);
        let normal = rotate90(nx, ny);
        self.edges.push(Edge {
            node1: n1,
            node2: n2,
            obstacle,
            intersections: Vec::new(),
            normal,
        });
        id
    }

    /// `CCIntersection.alloc().init(...)`.
    fn intersection(
        &mut self,
        edge1: EdgeId,
        edge2: EdgeId,
        dist1: f32,
        dist2: f32,
        node_split: NodeId,
    ) -> IntersectionId {
        self.intersection_arena.push(Intersection {
            edge1,
            edge2,
            dist1,
            dist2,
            node_split,
        });
        self.intersection_arena.len() - 1
    }

    /// `CCObstacle.calcBounds()`.
    fn calc_bounds(&mut self, o: ObstacleId) {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        // Float.MIN_VALUE, the smallest positive float.
        let mut max_x = f32::from_bits(1);
        let mut max_y = f32::from_bits(1);
        for &e in &self.obstacle_arena[o].edges {
            let n = &self.node_arena[self.edges[e].node1];
            min_x = jmin(min_x, n.x);
            min_y = jmin(min_y, n.y);
            max_x = jmax(max_x, n.x);
            max_y = jmax(max_y, n.y);
        }
        self.obstacle_arena[o].bounds = Rect {
            x: min_x - 0.01,
            y: min_y - 0.01,
            w: max_x - min_x + 0.02,
            h: max_y - min_y + 0.02,
        };
    }

    // ----- CCObjectOutline -----

    fn oo_get(&self, x: i32, y: i32) -> Option<usize> {
        if !(0..OO).contains(&x) || !(0..OO).contains(&y) {
            return None;
        }
        Some((x * OO + y) as usize)
    }

    /// `CCObjectOutline.setSolid`.
    fn set_solid(&mut self, x: i32, y: i32) {
        self.set_west(x, y);
        self.set_north(x, y);
        self.set_west(x + 1, y);
        self.set_north(x, y + 1);
    }

    /// `CCObjectOutline.setWest`.
    fn set_west(&mut self, x: i32, y: i32) {
        let f1 = self.oo_get(x, y);
        if let Some(i) = f1 {
            let f = &mut self.oo[i];
            if f.nw {
                f.nw_s = false;
            } else {
                f.nw = true;
                f.nw_w = true;
                f.nw_n = true;
                f.nw_e = true;
                f.nw_s = false;
            }
            f.w_w = true;
            f.w_e = true;
        }
        match self.oo_get(x, y + 1) {
            None => {
                if let Some(i) = f1 {
                    self.oo[i].w_cutoff = true;
                }
            }
            Some(j) => {
                let f = &mut self.oo[j];
                if f.nw {
                    f.nw_n = false;
                } else {
                    f.nw = true;
                    f.nw_n = false;
                    f.nw_w = true;
                    f.nw_e = true;
                    f.nw_s = true;
                }
            }
        }
    }

    /// `CCObjectOutline.setNorth`.
    fn set_north(&mut self, x: i32, y: i32) {
        let f1 = self.oo_get(x, y);
        if let Some(i) = f1 {
            let f = &mut self.oo[i];
            if f.nw {
                f.nw_e = false;
            } else {
                f.nw = true;
                f.nw_w = true;
                f.nw_n = true;
                f.nw_e = false;
                f.nw_s = true;
            }
            f.n_n = true;
            f.n_s = true;
        }
        match self.oo_get(x + 1, y) {
            None => {
                if let Some(i) = f1 {
                    self.oo[i].n_cutoff = true;
                }
            }
            Some(j) => {
                let f = &mut self.oo[j];
                if f.nw {
                    f.nw_w = false;
                } else {
                    f.nw = true;
                    f.nw_n = true;
                    f.nw_w = false;
                    f.nw_e = true;
                    f.nw_s = true;
                }
            }
        }
    }

    /// The `extend`-or-add step every `trace_*` starts with.
    fn trace_point(&mut self, z: i32, extend: bool, x: f32, y: f32) {
        if extend {
            let last = *self.trace_nodes.last().unwrap();
            let n = &mut self.node_arena[last];
            n.x = x;
            n.y = y;
        } else {
            let n = self.node(x, y, z);
            self.trace_nodes.push(n);
        }
    }

    /// Sets the last trace node's position (`node2.setXY`).
    fn set_last(&mut self, x: f32, y: f32) {
        let last = *self.trace_nodes.last().unwrap();
        let n = &mut self.node_arena[last];
        n.x = x;
        n.y = y;
    }

    fn trace_nw_n(&mut self, i: usize, extend: bool) {
        let Outline { x, y, z, .. } = self.oo[i];
        self.trace_point(z, extend, x as f32 + RADIUS, y as f32 - RADIUS);
        self.oo[i].nw_n = false;
        if self.oo[i].nw_e {
            self.trace_nw_e(i, false);
        } else if self.oo[i].n_n {
            self.trace_n_n(i, true);
        }
    }

    fn trace_nw_s(&mut self, i: usize, extend: bool) {
        let Outline { x, y, z, .. } = self.oo[i];
        self.trace_point(z, extend, x as f32 - RADIUS, y as f32 + RADIUS);
        self.oo[i].nw_s = false;
        if self.oo[i].nw_w {
            self.trace_nw_w(i, false);
        } else {
            let Some(j) = self.oo_get(x - 1, y) else {
                return;
            };
            if self.oo[j].n_s {
                self.trace_n_s(j, true);
            }
        }
    }

    fn trace_nw_w(&mut self, i: usize, extend: bool) {
        let Outline { x, y, z, .. } = self.oo[i];
        self.trace_point(z, extend, x as f32 - RADIUS, y as f32 - RADIUS);
        self.oo[i].nw_w = false;
        if self.oo[i].nw_n {
            self.trace_nw_n(i, false);
        } else {
            let Some(j) = self.oo_get(x, y - 1) else {
                return;
            };
            if self.oo[j].w_w {
                self.trace_w_w(j, true);
            }
        }
    }

    fn trace_nw_e(&mut self, i: usize, extend: bool) {
        let Outline { x, y, z, .. } = self.oo[i];
        self.trace_point(z, extend, x as f32 + RADIUS, y as f32 + RADIUS);
        self.oo[i].nw_e = false;
        if self.oo[i].nw_s {
            self.trace_nw_s(i, false);
        } else if self.oo[i].w_e {
            self.trace_w_e(i, true);
        }
    }

    fn trace_w_e(&mut self, i: usize, extend: bool) {
        let Outline { x, y, z, .. } = self.oo[i];
        self.trace_point(z, extend, x as f32 + RADIUS, (y + 1) as f32 - RADIUS);
        self.oo[i].w_e = false;
        if self.oo[i].w_cutoff {
            self.set_last(x as f32 + RADIUS, (y + 1) as f32 + RADIUS);
            let n = self.node(x as f32 - RADIUS, (y + 1) as f32 + RADIUS, z);
            self.trace_nodes.push(n);
            let n = self.node(x as f32 - RADIUS, (y + 1) as f32 - RADIUS, z);
            self.trace_nodes.push(n);
            self.trace_w_w(i, true);
            return;
        }
        let Some(j) = self.oo_get(x, y + 1) else {
            return;
        };
        if self.oo[j].nw && self.oo[j].nw_e {
            self.trace_nw_e(j, true);
        } else if self.oo[j].n_n {
            self.trace_n_n(j, false);
        }
    }

    fn trace_w_w(&mut self, i: usize, extend: bool) {
        let Outline { x, y, z, .. } = self.oo[i];
        self.trace_point(z, extend, x as f32 - RADIUS, y as f32 + RADIUS);
        self.oo[i].w_w = false;
        if self.oo[i].nw_w {
            self.trace_nw_w(i, true);
        } else {
            let Some(j) = self.oo_get(x - 1, y) else {
                return;
            };
            if self.oo[j].n_s {
                self.trace_n_s(j, false);
            }
        }
    }

    fn trace_n_n(&mut self, i: usize, extend: bool) {
        let Outline { x, y, z, .. } = self.oo[i];
        self.trace_point(z, extend, (x + 1) as f32 - RADIUS, y as f32 - RADIUS);
        self.oo[i].n_n = false;
        if self.oo[i].n_cutoff {
            self.set_last((x + 1) as f32 + RADIUS, y as f32 - RADIUS);
            let n = self.node((x + 1) as f32 + RADIUS, y as f32 + RADIUS, z);
            self.trace_nodes.push(n);
            let n = self.node((x + 1) as f32 - RADIUS, y as f32 + RADIUS, z);
            self.trace_nodes.push(n);
            self.trace_n_s(i, true);
            return;
        }
        let Some(j) = self.oo_get(x + 1, y) else {
            return;
        };
        if self.oo[j].nw_n {
            self.trace_nw_n(j, true);
        } else {
            let Some(k) = self.oo_get(x + 1, y - 1) else {
                return;
            };
            if self.oo[k].w_w {
                self.trace_w_w(k, false);
            }
        }
    }

    fn trace_n_s(&mut self, i: usize, extend: bool) {
        let Outline { x, y, z, .. } = self.oo[i];
        self.trace_point(z, extend, x as f32 + RADIUS, y as f32 + RADIUS);
        self.oo[i].n_s = false;
        if self.oo[i].nw_s {
            self.trace_nw_s(i, true);
        } else if self.oo[i].w_e {
            self.trace_w_e(i, false);
        }
    }

    /// `CCObjectOutline.trace(oo, obstacleTraceNodes)`: the closed outline
    /// starting at this corner in `trace_nodes`, or nothing.
    fn trace(&mut self, i: usize) {
        self.trace_nodes.clear();
        let Outline { x, y, z, .. } = self.oo[i];
        let node1 = self.node(x as f32 - RADIUS, y as f32 - RADIUS, z);
        self.trace_nodes.push(node1);
        self.trace_nw_n(i, false);
        let last = *self.trace_nodes.last().unwrap();
        let (n1, nl) = (&self.node_arena[node1], &self.node_arena[last]);
        if self.trace_nodes.len() == 2 || n1.x != nl.x || n1.y != nl.y {
            self.trace_nodes.clear();
        } else {
            *self.trace_nodes.last_mut().unwrap() = node1;
        }
    }

    // ----- obstacles -----

    /// `getObstaclesInRect(x1, y1, x2, y2, chrX, chrY, z)`.
    #[allow(clippy::too_many_arguments)]
    fn get_obstacles_in_rect<G: GetSquare>(
        &mut self,
        map: &MainThreadMap<'_, '_, G>,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        chr_x: i32,
        chr_y: i32,
        z: i32,
    ) {
        self.clear();
        let move_bounds = Rect {
            x: x1 - 1.0,
            y: y1 - 1.0,
            w: x2 - x1 + 2.0,
            h: y2 - y1 + 2.0,
        };
        // getVehiclesInRect(x1 - 1 - 4, y1 - 1 - 4, x2 + 2 + 8, y2 + 2 + 8, z)
        let (vx1, vy1) = (x1 - 1.0 - 4.0, y1 - 1.0 - 4.0);
        let (vx2, vy2) = (x2 + 2.0 + 8.0, y2 + 2.0 + 8.0);
        let (cx1, cy1) = (fastfloor(vx1 / 8.0), fastfloor(vy1 / 8.0));
        let (cx2, cy2) = (ceil_i(vx2 / 8.0), ceil_i(vy2 / 8.0));
        let mut polys = Vec::new();
        for cy in cy1..cy2 {
            for cx in cx1..cx2 {
                map.vehicles.for_each_vehicle_in_chunk(cx, cy, &mut |v| {
                    if v.has_script() && fastfloor(v.z()) == z {
                        polys.push(v.poly_plus_radius());
                    }
                    false
                });
            }
        }
        for poly in polys {
            let x_min = jmin(poly.x1, jmin(poly.x2, jmin(poly.x3, poly.x4)));
            let y_min = jmin(poly.y1, jmin(poly.y2, jmin(poly.y3, poly.y4)));
            let x_max = jmax(poly.x1, jmax(poly.x2, jmax(poly.x3, poly.x4)));
            let y_max = jmax(poly.y1, jmax(poly.y2, jmax(poly.y3, poly.y4)));
            let vehicle_bounds = Rect {
                x: x_min,
                y: y_min,
                w: x_max - x_min,
                h: y_max - y_min,
            };
            if !move_bounds.intersects(&vehicle_bounds) {
                continue;
            }
            let pz = fastfloor(poly.z);
            let n1 = self.node(poly.x1, poly.y1, pz);
            let n2 = self.node(poly.x2, poly.y2, pz);
            let n3 = self.node(poly.x3, poly.y3, pz);
            let n4 = self.node(poly.x4, poly.y4, pz);
            let o = self.obstacle();
            let e1 = self.edge(n1, n2, o);
            let e2 = self.edge(n2, n3, o);
            let e3 = self.edge(n3, n4, o);
            let e4 = self.edge(n4, n1, o);
            self.obstacle_arena[o].edges.extend([e1, e2, e3, e4]);
            self.calc_bounds(o);
            self.obstacles.push(o);
            self.nodes.extend([n1, n2, n3, n4]);
        }
        if self.obstacles.is_empty() {
            return;
        }
        let left = chr_x - 2;
        let top = chr_y - 2;
        let right = chr_x + 2 + 1;
        let bottom = chr_y + 2 + 1;
        for y in top..bottom {
            for x in left..right {
                let i = self.oo_get(x - left, y - top).unwrap();
                self.oo[i] = Outline {
                    x: x - left,
                    y: y - top,
                    z,
                    ..Outline::default()
                };
            }
        }
        let m = map.sprites();
        let t = &map.bits.t;
        let flag = &m.tables.flag;
        for y in top..bottom - 1 {
            for x in left..right - 1 {
                let Some(sq) = map.square(x, y, z) else {
                    continue;
                };
                if sq.is_solid(m)
                    || sq.is_solid_trans(m)
                        && !sq.is_adjacent_to_window(m, map.squares(), map.hooks())
                        && !sq.is_adjacent_to_hoppable(m, map.squares(), map.hooks())
                    || sq.has_type(t.stairsMN)
                    || sq.has_type(t.stairsTN)
                    || sq.has_type(t.stairsMW)
                    || sq.has_type(t.stairsTW)
                {
                    self.set_solid(x - left, y - top);
                }
                let mut collide_w = sq.has(flag.collideW);
                if sq.has(flag.windowW) || sq.has(flag.WindowW) {
                    collide_w = true;
                }
                if collide_w && sq.has(flag.doorW) {
                    collide_w = false;
                }
                let mut collide_n = sq.has(flag.collideN);
                if sq.has(flag.windowN) || sq.has(flag.WindowN) {
                    collide_n = true;
                }
                if collide_n && sq.has(flag.doorN) {
                    collide_n = false;
                }
                if collide_w
                    || sq.has_blocked_door(EdgeFacing::EastWest, map.hooks())
                    || sq.has_type(t.stairsBN)
                {
                    self.set_west(x - left, y - top);
                }
                if collide_n
                    || sq.has_blocked_door(EdgeFacing::NorthSouth, map.hooks())
                    || sq.has_type(t.stairsBW)
                {
                    self.set_north(x - left, y - top);
                }
                if sq.has_type(t.stairsBN) && x != right - 2 {
                    if map.square(x + 1, y, z).is_some() {
                        self.set_west(x + 1 - left, y - top);
                    }
                    continue;
                }
                if !sq.has_type(t.stairsBW) || y == bottom - 2 || map.square(x, y + 1, z).is_none()
                {
                    continue;
                }
                self.set_north(x - left, y + 1 - top);
            }
        }
        for y in 0..bottom - top {
            for x in 0..right - left {
                let Some(i) = self.oo_get(x, y) else {
                    continue;
                };
                let f = self.oo[i];
                if !f.nw || !f.nw_w || !f.nw_n {
                    continue;
                }
                self.trace(i);
                if self.trace_nodes.is_empty() {
                    continue;
                }
                let o = self.obstacle();
                let nodes = std::mem::take(&mut self.trace_nodes);
                let node0 = nodes[nodes.len() - 1];
                for i in (1..nodes.len()).rev() {
                    let n1 = nodes[i];
                    let n2 = nodes[i - 1];
                    self.node_arena[n1].x += left as f32;
                    self.node_arena[n1].y += top as f32;
                    let e = self.edge(n1, n2, o);
                    let (bl, bt) = if n2 != node0 {
                        (left as f32, top as f32)
                    } else {
                        (0.0, 0.0)
                    };
                    let (a, b) = (&self.node_arena[n1], &self.node_arena[n2]);
                    let n2x = b.x + bl;
                    let n2y = b.y + bt;
                    let (nx, ny) = normalize(n2x - a.x, n2y - a.y);
                    self.edges[e].normal = rotate90(nx, ny);
                    self.obstacle_arena[o].edges.push(e);
                    self.nodes.push(n1);
                }
                self.trace_nodes = nodes;
                self.calc_bounds(o);
                self.obstacles.push(o);
            }
        }
    }

    /// `getIntersection(edge1, edge2)`.
    fn get_intersection(&mut self, e1: EdgeId, e2: EdgeId) -> Option<IntersectionId> {
        let (a1, a2) = (self.edges[e1].node1, self.edges[e1].node2);
        let (b1, b2) = (self.edges[e2].node1, self.edges[e2].node2);
        let (x1, y1) = (self.node_arena[a1].x, self.node_arena[a1].y);
        let (x2, y2) = (self.node_arena[a2].x, self.node_arena[a2].y);
        let (x3, y3) = (self.node_arena[b1].x, self.node_arena[b1].y);
        let (x4, y4) = (self.node_arena[b2].x, self.node_arena[b2].y);
        let denom = ((y4 - y3) * (x2 - x1) - (x4 - x3) * (y2 - y1)) as f64;
        if denom > -0.01 && denom < 0.01 {
            return None;
        }
        let ua = ((x4 - x3) * (y1 - y3) - (y4 - y3) * (x1 - x3)) as f64 / denom;
        let ub = ((x2 - x1) * (y1 - y3) - (y2 - y1) * (x1 - x3)) as f64 / denom;
        if !(ua >= 0.0 && ua <= 1.0 && ub >= 0.0 && ub <= 1.0) {
            return None;
        }
        let ix = (x1 as f64 + ua * (x2 - x1) as f64) as f32;
        let iy = (y1 as f64 + ua * (y2 - y1) as f64) as f32;
        let lo = 0.01f32 as f64;
        let hi = 0.99f32 as f64;
        let node1 = if ua < lo {
            Some(a1)
        } else if ua > hi {
            Some(a2)
        } else {
            None
        };
        let node2 = if ub < lo {
            Some(b1)
        } else if ub > hi {
            Some(b2)
        } else {
            None
        };
        match (node1, node2) {
            (Some(n1), Some(n2)) => {
                let i = self.intersection(e1, e2, ua as f32, ub as f32, n1);
                self.edges[e1].intersections.push(i);
                self.intersections.push(i);
                let i = self.intersection(e1, e2, ua as f32, ub as f32, n2);
                self.edges[e2].intersections.push(i);
                self.intersections.push(i);
                None
            }
            (None, None) => {
                let z = self.node_arena[a1].z;
                let n = self.node(ix, iy, z);
                Some(self.intersection(e1, e2, ua as f32, ub as f32, n))
            }
            (n1, n2) => {
                let n = n1.or(n2).unwrap();
                Some(self.intersection(e1, e2, ua as f32, ub as f32, n))
            }
        }
    }

    /// `CCEdge.split(nodeSplit)`.
    fn split_edge(&mut self, e: EdgeId, split: NodeId) {
        let (n2, o) = (self.edges[e].node2, self.edges[e].obstacle);
        let new = self.edge(split, n2, o);
        let edges = &mut self.obstacle_arena[o].edges;
        let at = edges.iter().position(|&x| x == e).map_or(0, |p| p + 1);
        edges.insert(at, new);
        let n2_edges = &mut self.node_arena[n2].edges;
        if let Some(p) = n2_edges.iter().position(|&x| x == e) {
            n2_edges.remove(p);
        }
        self.edges[e].node2 = split;
        self.node_arena[split].edges.push(e);
    }

    /// `CCIntersection.split(edge)`.
    fn split_at(&mut self, i: IntersectionId, e: EdgeId) {
        let split = self.intersection_arena[i].node_split;
        let edge = &self.edges[e];
        if edge.has_node(split) {
            return;
        }
        let s = &self.node_arena[split];
        let (a, b) = (&self.node_arena[edge.node1], &self.node_arena[edge.node2]);
        if a.x == s.x && a.y == s.y || b.x == s.x && b.y == s.y {
            return;
        }
        self.split_edge(e, split);
    }

    /// `checkEdgeIntersection()`.
    fn check_edge_intersection(&mut self) {
        let n = self.obstacles.len();
        for i in 0..n {
            let o1 = self.obstacles[i];
            for j in i + 1..n {
                let o2 = self.obstacles[j];
                if !self.obstacle_arena[o1]
                    .bounds
                    .intersects(&self.obstacle_arena[o2].bounds)
                {
                    continue;
                }
                for k in 0..self.obstacle_arena[o1].edges.len() {
                    let e1 = self.obstacle_arena[o1].edges[k];
                    for m in 0..self.obstacle_arena[o2].edges.len() {
                        let e2 = self.obstacle_arena[o2].edges[m];
                        let Some(x) = self.get_intersection(e1, e2) else {
                            continue;
                        };
                        self.edges[e1].intersections.push(x);
                        self.edges[e2].intersections.push(x);
                        let split = self.intersection_arena[x].node_split;
                        if !self.edges[e1].has_node(split) && !self.edges[e2].has_node(split) {
                            self.nodes.push(split);
                        }
                        self.intersections.push(x);
                    }
                }
            }
        }
        for i in 0..n {
            let o1 = self.obstacles[i];
            for j in (0..self.obstacle_arena[o1].edges.len()).rev() {
                let e = self.obstacle_arena[o1].edges[j];
                if self.edges[e].intersections.is_empty() {
                    continue;
                }
                let mut list = std::mem::take(&mut self.edges[e].intersections);
                let dist = |x: &Intersection| if x.edge1 == e { x.dist1 } else { x.dist2 };
                let arena = &self.intersection_arena;
                list.sort_by(|&a, &b| {
                    let (d1, d2) = (dist(&arena[a]), dist(&arena[b]));
                    if d1 < d2 {
                        std::cmp::Ordering::Less
                    } else if d1 > d2 {
                        std::cmp::Ordering::Greater
                    } else {
                        std::cmp::Ordering::Equal
                    }
                });
                self.edges[e].intersections = list;
                for k in (0..self.edges[e].intersections.len()).rev() {
                    let x = self.edges[e].intersections[k];
                    self.split_at(x, e);
                }
            }
        }
    }

    fn obstacle_has_node(&self, o: ObstacleId, n: NodeId) -> bool {
        self.obstacle_arena[o]
            .edges
            .iter()
            .any(|&e| self.edges[e].has_node(n))
    }

    /// `CCObstacle.isPointInPolygon_WindingNumber(x, y, 0) == Inside`.
    fn is_point_inside(&self, o: ObstacleId, x: f32, y: f32) -> bool {
        let is_left = |x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32| {
            (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)
        };
        let mut wn = 0;
        for &e in &self.obstacle_arena[o].edges {
            let a = &self.node_arena[self.edges[e].node1];
            let b = &self.node_arena[self.edges[e].node2];
            if a.y <= y {
                if b.y > y && is_left(a.x, a.y, b.x, b.y, x, y) > 0.0 {
                    wn += 1;
                }
            } else if b.y <= y && is_left(a.x, a.y, b.x, b.y, x, y) < 0.0 {
                wn -= 1;
            }
        }
        wn != 0
    }

    /// `checkNodesInObstacles()`.
    fn check_nodes_in_obstacles(&mut self) {
        for i in 0..self.nodes.len() {
            let node = self.nodes[i];
            for &o in &self.obstacles {
                let mut is_intersect = false;
                for &x in &self.intersections {
                    let x = &self.intersection_arena[x];
                    if x.node_split != node {
                        continue;
                    }
                    if self.edges[x.edge1].obstacle == o || self.edges[x.edge2].obstacle == o {
                        is_intersect = true;
                    }
                    break;
                }
                if is_intersect {
                    continue;
                }
                // isNodeInsideOf
                if self.obstacle_has_node(o, node) {
                    continue;
                }
                let n = &self.node_arena[node];
                if !self.obstacle_arena[o].bounds.contains_point(n.x, n.y) {
                    continue;
                }
                if !self.is_point_inside(o, n.x, n.y) {
                    continue;
                }
                self.node_arena[node].ignore = true;
                break;
            }
        }
    }

    /// `CCNode.sharesEdge(other)`.
    fn shares_edge(&self, a: NodeId, b: NodeId) -> bool {
        self.node_arena[a]
            .edges
            .iter()
            .any(|&e| self.edges[e].has_node(b))
    }

    /// `CCNode.sharesShape(other)`.
    fn shares_shape(&self, a: NodeId, b: NodeId) -> bool {
        self.node_arena[a].edges.iter().any(|&e| {
            self.node_arena[b]
                .edges
                .iter()
                .any(|&f| self.edges[e].obstacle == self.edges[f].obstacle)
        })
    }

    /// `CCNode.onSameShapeButDoesNotShareAnEdge(other)`.
    fn on_same_shape_but_does_not_share_an_edge(&self, a: NodeId, b: NodeId) -> bool {
        let mut temp: Vec<ObstacleId> = Vec::new();
        for &e in &self.node_arena[a].edges {
            let o = self.edges[e].obstacle;
            if !temp.contains(&o) {
                temp.push(o);
            }
        }
        temp.into_iter().any(|o| {
            self.obstacle_has_node(o, b)
                && !self.obstacle_arena[o]
                    .edges
                    .iter()
                    .any(|&e| self.edges[e].has_node(a) && self.edges[e].has_node(b))
        })
    }

    /// `isVisible(node1, node2)`.
    fn is_visible(&self, a: NodeId, b: NodeId) -> bool {
        if self.shares_edge(a, b) {
            return !self.on_same_shape_but_does_not_share_an_edge(a, b);
        }
        !self.shares_shape(a, b)
    }

    /// `calculateNodeVisibility()`.
    fn calculate_node_visibility(&mut self) {
        for i in 0..self.obstacles.len() {
            let o = self.obstacles[i];
            for j in 0..self.obstacle_arena[o].edges.len() {
                let e = self.obstacle_arena[o].edges[j];
                let (a, b) = (self.edges[e].node1, self.edges[e].node2);
                if self.node_arena[a].ignore || self.node_arena[b].ignore || !self.is_visible(a, b)
                {
                    continue;
                }
                self.node_arena[a].visible.push(b);
                self.node_arena[b].visible.push(a);
            }
        }
    }

    /// `edge.node1.visible.contains(edge.node2)`.
    fn edge_visible(&self, e: EdgeId) -> bool {
        let e = &self.edges[e];
        self.node_arena[e.node1].visible.contains(&e.node2)
    }

    /// `CCEdge.closestPoint(x3, y3, closest)`: the point and, at either end,
    /// the node there.
    fn closest_point(&self, e: EdgeId, x3: f32, y3: f32) -> ((f32, f32), Option<NodeId>) {
        let e = &self.edges[e];
        let (a, b) = (&self.node_arena[e.node1], &self.node_arena[e.node2]);
        let (x1, y1, x2, y2) = (a.x, a.y, b.x, b.y);
        let dx = (x2 - x1) as f64;
        let dy = (y2 - y1) as f64;
        let u = ((x3 - x1) * (x2 - x1) + (y3 - y1) * (y2 - y1)) as f64 / (dx * dx + dy * dy);
        if u <= 0.001 {
            return ((x1, y1), Some(e.node1));
        }
        if u >= 0.999 {
            return ((x2, y2), Some(e.node2));
        }
        let xu = x1 as f64 + u * dx;
        let yu = y1 as f64 + u * dy;
        ((xu as f32, yu as f32), None)
    }

    /// `CCNode.getNormalAndEdgeVectors(normal, edgeVec)`: `None` (and both
    /// left as they were) unless two visible edges meet at the node.
    fn normal_and_edge_vectors(&self, n: NodeId) -> Option<((f32, f32), (f32, f32))> {
        let mut edge1: Option<EdgeId> = None;
        let mut edge2: Option<EdgeId> = None;
        for &e in &self.node_arena[n].edges {
            if !self.edge_visible(e) {
                continue;
            }
            match edge1 {
                None => edge1 = Some(e),
                Some(e1) => {
                    let (a, b) = (self.edges[e].node1, self.edges[e].node2);
                    if self.edges[e1].has_node(a) && self.edges[e1].has_node(b) {
                        continue;
                    }
                    edge2 = Some(e);
                }
            }
        }
        let (e1, e2) = (edge1?, edge2?);
        let (n1, n2) = (self.edges[e1].normal, self.edges[e2].normal);
        let normal = normalize(n1.0 + n2.0, n1.1 + n2.1);
        let e1 = &self.edges[e1];
        let (a, b) = (&self.node_arena[e1.node1], &self.node_arena[e1.node2]);
        let ev = if e1.node1 == n {
            (b.x - a.x, b.y - a.y)
        } else {
            (a.x - b.x, a.y - b.y)
        };
        Some((normal, normalize(ev.0, ev.1)))
    }

    /// The nearest visible edge of an obstacle `(x1, y1)` is inside, and the
    /// node at its closest point (the first loop of both overloads).
    fn closest_inside_edge(&self, x1: f32, y1: f32) -> (Option<EdgeId>, Option<NodeId>, f64) {
        let mut closest_edge = None;
        let mut node_at = None;
        let mut closest_dist = f64::MAX;
        for &o in &self.obstacles {
            if !self.is_point_inside(o, x1, y1) {
                continue;
            }
            for &e in &self.obstacle_arena[o].edges {
                if !self.edge_visible(e) {
                    continue;
                }
                let ((cx, cy), node) = self.closest_point(e, x1, y1);
                let dist_sq = ((x1 - cx) * (x1 - cx) + (y1 - cy) * (y1 - cy)) as f64;
                if !(dist_sq < closest_dist) {
                    continue;
                }
                closest_dist = dist_sq;
                closest_edge = Some(e);
                node_at = node;
            }
        }
        (closest_edge, node_at, closest_dist)
    }
}

/// `PolygonalMap2.instance.collideWithObstacles`: the obstacle graph of the
/// last call plus the vectors the Java object carries between calls.
#[derive(Debug, Default)]
pub struct CollideWithObstacles {
    graph: Graph,
    /// `move`: the normalized direction of the last `resolveCollision(chr,
    /// nx, ny)` that moved.
    move_: (f32, f32),
    /// `nodeNormal`.
    node_normal: (f32, f32),
    /// `edgeVec`.
    edge_vec: (f32, f32),
}

impl CollideWithObstacles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `chr.getCurrentSquare()` is a square with stairs.
    fn on_stairs<G: GetSquare>(map: &MainThreadMap<'_, '_, G>, chr: &dyn CharacterView) -> bool {
        chr.current_square()
            .and_then(|(x, y, z)| map.square(x, y, z))
            .is_some_and(|s: &IsoGridSquare| s.has_stairs(map.sprites()))
    }

    /// `CollideWithObstacles.resolveCollision(chr, nx, ny, finalPos)`
    /// (`PolygonalMap2.resolveCollision(chr, nx, ny, finalPos)`, called by
    /// `IsoMovingObject.postupdate`): where the character, moving from its
    /// position to `(nx, ny)` on its level, ends up against the vehicles and
    /// the walls next to them. `(nx, ny)` itself when nothing is in the way
    /// or the character stands on stairs.
    ///
    /// `PolygonalMap2`'s `GameClient.client && chr.isSkipResolveCollision()`
    /// early-out (answering `(nx, ny)`) is the caller's.
    pub fn resolve_collision<G: GetSquare>(
        &mut self,
        map: &MainThreadMap<'_, '_, G>,
        chr: &dyn CharacterView,
        nx: f32,
        ny: f32,
    ) -> (f32, f32) {
        if Self::on_stairs(map, chr) {
            return (nx, ny);
        }
        self.resolve_move(map, chr.x(), chr.y(), chr.z(), nx, ny)
    }

    /// `CollideWithObstacles.resolveCollision(chr, radius, finalPos)`
    /// (`PolygonalMap2.resolveCollision(chr, radius, finalPos)`, called by
    /// `IsoGameCharacter.slideAwayFromWalls`): whether a character of
    /// `radius` overlaps an obstacle edge it stands inside of, and
    /// `finalPos`: the point `radius` out from that edge when it does, the
    /// character's position otherwise.
    ///
    /// As in Java this reads the direction of the last
    /// [`Self::resolve_collision`] that moved, and may read a node normal
    /// left from an earlier call. `PolygonalMap2`'s `GameClient.client &&
    /// chr.isSkipResolveCollision()` early-out (answering `false` and
    /// leaving `finalPos` alone) is the caller's.
    pub fn resolve_collision_radius<G: GetSquare>(
        &mut self,
        map: &MainThreadMap<'_, '_, G>,
        chr: &dyn CharacterView,
        radius: f32,
    ) -> (bool, (f32, f32)) {
        if Self::on_stairs(map, chr) {
            return (false, (chr.x(), chr.y()));
        }
        self.resolve_radius(map, chr.x(), chr.y(), chr.z(), radius)
    }

    /// `resolveCollision(x1, y1, z1, x2, y2, finalPos)`.
    fn resolve_move<G: GetSquare>(
        &mut self,
        map: &MainThreadMap<'_, '_, G>,
        x1: f32,
        y1: f32,
        z1: f32,
        x2: f32,
        y2: f32,
    ) -> (f32, f32) {
        if x1 == x2 && y1 == y2 {
            return (x2, y2);
        }
        let mv = normalize(x2 - x1, y2 - y1);
        self.move_ = mv;
        let g = &mut self.graph;
        g.get_obstacles_in_rect(
            map,
            jmin(x1, x2),
            jmin(y1, y2),
            jmax(x1, x2),
            jmax(y1, y2),
            fastfloor(x1),
            fastfloor(y1),
            fastfloor(z1),
        );
        g.check_edge_intersection();
        g.check_nodes_in_obstacles();
        g.calculate_node_visibility();
        let (mut closest_edge, mut node_at, _) = g.closest_inside_edge(x1, y1);
        if let Some(e) = closest_edge {
            if dot(g.edges[e].normal, mv) >= 0.01 {
                closest_edge = None;
            }
        }
        if let Some(n) = node_at {
            if let Some((nn, ev)) = g.normal_and_edge_vectors(n) {
                self.node_normal = nn;
                self.edge_vec = ev;
                if dot(nn, mv) + 0.05 >= dot(nn, ev) {
                    node_at = None;
                    closest_edge = None;
                }
            }
        }
        if closest_edge.is_none() {
            let mut closest_dist_sq = f64::MAX;
            node_at = None;
            'obstacles: for &o in &g.obstacles {
                for &e in &g.obstacle_arena[o].edges {
                    if !g.edge_visible(e) {
                        continue;
                    }
                    let edge = &g.edges[e];
                    let (a, b) = (&g.node_arena[edge.node1], &g.node_arena[edge.node2]);
                    let (x3, y3, x4, y4) = (a.x, a.y, b.x, b.y);
                    let denom = ((y4 - y3) * (x2 - x1) - (x4 - x3) * (y2 - y1)) as f64;
                    if denom == 0.0 {
                        continue;
                    }
                    let ua = ((x4 - x3) * (y1 - y3) - (y4 - y3) * (x1 - x3)) as f64 / denom;
                    let ub = ((x2 - x1) * (y1 - y3) - (y2 - y1) * (x1 - x3)) as f64 / denom;
                    let dot2 = dot(edge.normal, mv);
                    if dot2 >= 0.0 || !(ua >= 0.0) || !(ua <= 1.0) || !(ub >= 0.0) || !(ub <= 1.0) {
                        continue;
                    }
                    if ub < 0.01 || ub > 0.99 {
                        let node = if ub < 0.01 { edge.node1 } else { edge.node2 };
                        if let Some((nn, ev)) = g.normal_and_edge_vectors(node) {
                            self.node_normal = nn;
                            self.edge_vec = ev;
                            if dot(nn, mv) + 0.05 >= dot(nn, ev) {
                                continue;
                            }
                            closest_edge = Some(e);
                            node_at = Some(node);
                            continue 'obstacles;
                        }
                    }
                    let ix = (x1 as f64 + ua * (x2 - x1) as f64) as f32;
                    let iy = (y1 as f64 + ua * (y2 - y1) as f64) as f32;
                    let ddx = (ix - x1) as f64;
                    let ddy = (iy - y1) as f64;
                    let dist_sq = ((ddx * ddx + ddy * ddy) as f32) as f64;
                    if !(dist_sq < closest_dist_sq) {
                        continue;
                    }
                    closest_dist_sq = dist_sq;
                    closest_edge = Some(e);
                }
            }
        }
        if let (Some(n), Some(edge1)) = (node_at, closest_edge) {
            let mut edge2 = None;
            for &e in &g.node_arena[n].edges {
                let ed = &g.edges[e];
                let e1 = &g.edges[edge1];
                let (p1, p2) = (&g.node_arena[e1.node1], &g.node_arena[e1.node2]);
                let (q1, q2) = (&g.node_arena[ed.node1], &g.node_arena[ed.node2]);
                if !g.edge_visible(e)
                    || e == edge1
                    || p1.x == q1.x && p1.y == q1.y && p2.x == q2.x && p2.y == q2.y
                    || p1.x == q2.x && p1.y == q2.y && p2.x == q1.x && p2.y == q1.y
                    || e1.has_node(ed.node1) && e1.has_node(ed.node2)
                {
                    continue;
                }
                edge2 = Some(e);
            }
            if let Some(edge2) = edge2 {
                // closestEdge == edge1 here.
                let e2 = &g.edges[edge2];
                let other = if n == e2.node1 { e2.node2 } else { e2.node1 };
                let (o, a) = (&g.node_arena[other], &g.node_arena[n]);
                let ev = normalize(o.x - a.x, o.y - a.y);
                self.edge_vec = ev;
                if dot(mv, ev) >= 0.0 {
                    closest_edge = Some(edge2);
                }
            }
        }
        match closest_edge {
            Some(e) => g.closest_point(e, x2, y2).0,
            None => (x2, y2),
        }
    }

    /// `resolveCollision(x1, y1, z1, radius, finalPos)`.
    fn resolve_radius<G: GetSquare>(
        &mut self,
        map: &MainThreadMap<'_, '_, G>,
        x1: f32,
        y1: f32,
        z1: f32,
        radius: f32,
    ) -> (bool, (f32, f32)) {
        if (radius as f64) <= 0.0 {
            return (false, (x1, y1));
        }
        let g = &mut self.graph;
        g.get_obstacles_in_rect(
            map,
            x1 - radius,
            y1 - radius,
            x1 + radius,
            y1 + radius,
            fastfloor(x1),
            fastfloor(y1),
            fastfloor(z1),
        );
        g.check_edge_intersection();
        g.check_nodes_in_obstacles();
        g.calculate_node_visibility();
        let (mut closest_edge, mut node_at, closest_dist) = g.closest_inside_edge(x1, y1);
        if let Some(n) = node_at {
            if let Some((nn, ev)) = g.normal_and_edge_vectors(n) {
                self.node_normal = nn;
                self.edge_vec = ev;
                if dot(nn, self.move_) + 0.05 >= dot(nn, ev) {
                    node_at = None;
                    closest_edge = None;
                }
            }
        }
        let Some(e) = closest_edge else {
            return (false, (x1, y1));
        };
        // Java compares the squared distance with the radius.
        if closest_dist > radius as f64 {
            return (false, (x1, y1));
        }
        let (pc, _) = g.closest_point(e, x1, y1);
        let vc = match node_at {
            None => g.edges[e].normal,
            Some(n) => {
                if let Some((nn, ev)) = g.normal_and_edge_vectors(n) {
                    self.node_normal = nn;
                    self.edge_vec = ev;
                }
                self.node_normal
            }
        };
        let vpc_x = pc.0 - x1;
        let vpc_y = pc.1 - y1;
        let vpc_dot_n = vpc_x * vc.0 + vpc_y * vc.1;
        if (vpc_dot_n as f64) < 0.0 {
            return (false, (x1, y1));
        }
        (true, (pc.0 + vc.0 * radius, pc.1 + vc.1 * radius))
    }
}
