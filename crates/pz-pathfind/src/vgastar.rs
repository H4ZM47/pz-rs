//! `VGAStar`, `SearchNode` and `GoalNode`: the low-level A* over squares
//! and visibility-graph nodes.

use std::collections::HashMap;

use crate::astar::{self, AStarHost};
use crate::request::PmMover;
use crate::square::bits::*;
use crate::util::{coordmodulo, dist3, fastfloor, Dir};
use crate::visibility::{NodeId, CXN_FLAG_BENDABLE, CXN_FLAG_TARGET_PATH, CXN_FLAG_THUMP};
use crate::visibility::{NODE_FLAG_CRAWL_INTERIOR, NODE_FLAG_PERIMETER, NODE_FLAG_STAIR};
use crate::world::{SquareId, World};

/// `SearchNode`.
#[derive(Clone, Debug, Default)]
pub struct SearchNode {
    pub vg_node: Option<NodeId>,
    pub square: Option<SquareId>,
    pub unloaded_x: i32,
    pub unloaded_y: i32,
    pub in_unloaded_area: bool,
    pub parent: Option<u32>,
    pub g: f64,
    pub depth: i32,
    pub total: f32,
    pub flags: u8,
}

/// `VGAStar` search state.
#[derive(Default)]
pub struct VgAStar {
    pub(crate) nodes: Vec<SearchNode>,
    /// Search nodes by visibility-graph node.
    pub(crate) node_map: HashMap<NodeId, u32>,
    /// Search nodes by square id.
    pub(crate) square_map: HashMap<u32, u32>,
    /// `GoalNode::searchNode`.
    pub(crate) goal: u32,
}

impl World {
    /// `VGAStar::init` (the native `init2`): reset the search and create
    /// a search node for every node in `PolygonalMap2::squareToNode`.
    pub(crate) fn vga_init(&mut self) {
        self.vga.nodes.clear();
        self.vga.node_map.clear();
        self.vga.square_map.clear();
        let entries: Vec<(u32, NodeId)> =
            self.square_to_node.iter().map(|(&k, &v)| (k, v)).collect();
        for (key, n) in entries {
            let square = self.node(n).square;
            let idx = self.vga.nodes.len() as u32;
            self.vga.nodes.push(SearchNode {
                vg_node: Some(n),
                square,
                unloaded_x: -1,
                unloaded_y: -1,
                ..SearchNode::default()
            });
            self.vga.square_map.insert(key, idx);
            self.vga.node_map.insert(n, idx);
        }
    }

    /// `VGAStar::getSearchNode(Node*)`.
    pub(crate) fn vga_search_node_for_node(&mut self, n: NodeId) -> u32 {
        if let Some(s) = self.node(n).square {
            return self.vga_search_node_for_square(s);
        }
        if let Some(&i) = self.vga.node_map.get(&n) {
            return i;
        }
        let idx = self.vga.nodes.len() as u32;
        self.vga.nodes.push(SearchNode {
            vg_node: Some(n),
            unloaded_x: -1,
            unloaded_y: -1,
            ..SearchNode::default()
        });
        self.vga.node_map.insert(n, idx);
        idx
    }

    /// `VGAStar::getSearchNode(Square*)`.
    pub(crate) fn vga_search_node_for_square(&mut self, s: SquareId) -> u32 {
        if let Some(&i) = self.vga.square_map.get(&s.raw()) {
            return i;
        }
        let idx = self.vga.nodes.len() as u32;
        self.vga.nodes.push(SearchNode {
            square: Some(s),
            unloaded_x: -1,
            unloaded_y: -1,
            ..SearchNode::default()
        });
        self.vga.square_map.insert(s.raw(), idx);
        idx
    }

    /// `VGAStar::getSearchNode(int, int)`: a node in an unloaded area.
    pub(crate) fn vga_search_node_unloaded(&mut self, x: i32, y: i32) -> u32 {
        let idx = self.vga.nodes.len() as u32;
        self.vga.nodes.push(SearchNode {
            unloaded_x: x,
            unloaded_y: y,
            in_unloaded_area: true,
            ..SearchNode::default()
        });
        idx
    }

    /// `VGAStar::isSquareInCluster`.
    pub(crate) fn is_square_in_cluster(&self, s: SquareId) -> bool {
        self.get_vis_graph_for_square(s, 0).is_some()
    }

    /// `SearchNode::getX`.
    pub(crate) fn sn_x(&self, i: u32) -> f32 {
        let n = &self.vga.nodes[i as usize];
        if let Some(s) = n.square {
            self.sq(s).x as f32 + 0.5
        } else if let Some(v) = n.vg_node {
            self.node(v).x
        } else {
            n.unloaded_x as f32
        }
    }

    /// `SearchNode::getY`.
    pub(crate) fn sn_y(&self, i: u32) -> f32 {
        let n = &self.vga.nodes[i as usize];
        if let Some(s) = n.square {
            self.sq(s).y as f32 + 0.5
        } else if let Some(v) = n.vg_node {
            self.node(v).y
        } else {
            n.unloaded_y as f32
        }
    }

    /// `SearchNode::getZ`.
    pub(crate) fn sn_z(&self, i: u32) -> f32 {
        let n = &self.vga.nodes[i as usize];
        if let Some(s) = n.square {
            self.sq(s).z as f32
        } else if let Some(v) = n.vg_node {
            self.node(v).z as f32
        } else {
            32.0
        }
    }

    /// `SearchNode::dist`.
    pub(crate) fn sn_dist(&self, a: u32, b: u32) -> f64 {
        let (na, nb) = (&self.vga.nodes[a as usize], &self.vga.nodes[b as usize]);
        if let (Some(sa), Some(sb)) = (na.square, nb.square) {
            let (s1, s2) = (self.sq(sa), self.sq(sb));
            if (s1.x - s2.x).abs() <= 1 && (s1.y - s2.y).abs() <= 1 {
                return if s1.y != s2.y && s1.x != s2.x {
                    std::f64::consts::SQRT_2
                } else {
                    1.0
                };
            }
        }
        let dx = self.sn_x(a) - self.sn_x(b);
        let dy = self.sn_y(a) - self.sn_y(b);
        let dz = (self.sn_z(a) - self.sn_z(b)) * 2.5;
        dist3(dx, dy, dz) as f64
    }

    /// `SearchNode::isOnEdgeOfLoadedArea`.
    pub(crate) fn sn_is_on_edge_of_loaded_area(&self, i: u32) -> bool {
        let x = fastfloor(self.sn_x(i));
        let y = fastfloor(self.sn_y(i));
        let mut on_edge = false;
        if coordmodulo(x, 8) == 0 && self.get_chunk_from_square_pos(x - 1, y).is_none() {
            on_edge = true;
        }
        if coordmodulo(x, 8) == 7 && self.get_chunk_from_square_pos(x + 1, y).is_none() {
            on_edge = true;
        }
        if coordmodulo(y, 8) == 0 && self.get_chunk_from_square_pos(x, y - 1).is_none() {
            on_edge = true;
        }
        if coordmodulo(y, 8) == 7 && self.get_chunk_from_square_pos(x, y + 1).is_none() {
            on_edge = true;
        }
        on_edge
    }

    /// `VGAStar::isKnownBlocked`. The JNI request carries no known blocked
    /// edges, so the native map is always empty.
    pub(crate) fn is_known_blocked(&self, _a: SquareId, _b: SquareId) -> bool {
        false
    }

    /// `VGAStar::canMoveBetween`.
    pub(crate) fn can_move_between(&self, a: SquareId, b: SquareId, is_between: bool) -> bool {
        !self.can_not_move_between(a, b, is_between)
    }

    /// `VGAStar::canAnimalBreakObstacleOrClimbFences`.
    #[allow(clippy::too_many_arguments)]
    fn can_animal_break_obstacle(
        &self,
        m: &PmMover,
        s1: SquareId,
        s2: SquareId,
        col_w: bool,
        col_n: bool,
        col_e: bool,
        col_s: bool,
    ) -> bool {
        if !m.can_thump && !m.can_climb_fences {
            return false;
        }
        let (s1, s2) = (self.sq(s1), self.sq(s2));
        if col_w {
            return s1.has(COLLIDE_W) && s1.has(CAN_PATH_W);
        }
        if col_n {
            return s1.has(COLLIDE_N) && s1.has(CAN_PATH_N);
        }
        if col_e {
            return s2.has(COLLIDE_W) && s2.has(CAN_PATH_W);
        }
        if col_s {
            return s2.has(COLLIDE_N) && s2.has(CAN_PATH_N);
        }
        false
    }

    /// `VGAStar::canNotMoveBetween`: whether a mover cannot step from
    /// `a` to the adjacent square `b` (same level).
    pub(crate) fn can_not_move_between(&self, a: SquareId, b: SquareId, is_between: bool) -> bool {
        self.can_not_move_between_m(self.mover(), a, b, is_between)
    }

    /// `VGAStar::canNotMoveBetween` with an explicit mover.
    pub(crate) fn can_not_move_between_m(
        &self,
        m: &PmMover,
        a: SquareId,
        b: SquareId,
        is_between: bool,
    ) -> bool {
        let w = self;
        let s1 = w.sq(a);
        let s2 = w.sq(b);
        let (x1, y1, x2, y2) = (s1.x, s1.y, s2.x, s2.y);
        if s2.is_non_thumpable_solid(w) {
            return true;
        }
        if !m.can_thump && (s2.is_really_solid(w) || s2.has_square_thumpable(w)) {
            return true;
        }
        if y2 < y1 && s1.has(STAIR_TN) {
            return true;
        }
        if x2 < x1 && s1.has(STAIR_TW) {
            return true;
        }
        if y1 < y2 && x2 == x1 && s2.has(STAIR_TN) {
            return true;
        }
        if x1 < x2 && y2 == y1 && s2.has(STAIR_TW) {
            return true;
        }
        if x2 != x1 && s2.has(STAIR_TN | STAIR_MN | STAIR_BN) {
            return true;
        }
        if y2 != y1 && s2.has(STAIR_TW | STAIR_MW | STAIR_BW) {
            return true;
        }
        if x2 != x1 && s1.has(STAIR_TN | STAIR_MN | STAIR_BN) {
            return true;
        }
        if y2 != y1 && s1.has(STAIR_TW | STAIR_MW | STAIR_BW) {
            return true;
        }
        if s2.z == s1.z {
            if x2 == x1
                && y2 == y1 - 1
                && (s1.is_sloped_surface_edge_blocked(w, Dir::N)
                    || s2.is_sloped_surface_edge_blocked(w, Dir::S))
            {
                return true;
            }
            if x2 == x1
                && y2 == y1 + 1
                && (s1.is_sloped_surface_edge_blocked(w, Dir::S)
                    || s2.is_sloped_surface_edge_blocked(w, Dir::N))
            {
                return true;
            }
            if x2 == x1 - 1
                && y2 == y1
                && (s1.is_sloped_surface_edge_blocked(w, Dir::W)
                    || s2.is_sloped_surface_edge_blocked(w, Dir::E))
            {
                return true;
            }
            if x2 == x1 + 1
                && y2 == y1
                && (s1.is_sloped_surface_edge_blocked(w, Dir::E)
                    || s2.is_sloped_surface_edge_blocked(w, Dir::W))
            {
                return true;
            }
        }
        if !s2.has(SOLID_FLOOR) && !s2.has(ALL_STAIRS) {
            return true;
        }
        if w.is_known_blocked(a, b) {
            return true;
        }
        let test_n = y2 < y1;
        let test_s = y1 < y2;
        let test_w = x2 < x1;
        let test_e = x1 < x2;
        if !m.is_animal() {
            // Per-edge "this obstacle does not stop the mover" terms.
            let ok_target_n = m.has_target || !s1.has(TARGET_PATH_N);
            let ok_target_w = m.has_target || !s1.has(TARGET_PATH_W);
            let ok_bend_n = m.can_bend || !s1.has(BENDABLE_N);
            let ok_bend_w = m.can_bend || !s1.has(BENDABLE_W);
            let ok_thump_n = m.can_thump || !s1.has(THUMP_N);
            let ok_thump_w = m.can_thump || !s1.has(THUMP_W);
            let no_tall_n = !m.can_climb_tall_fences || !s1.has(TALL_HOPPABLE_N);
            let no_tall_w = !m.can_climb_tall_fences || !s1.has(TALL_HOPPABLE_W);
            let can_path_n = s1.is_can_path_n(w);
            let can_path_w = s1.is_can_path_w(w);
            let col_n = test_n
                && s1.is_collide_n(w)
                && ((x1 != x2 || is_between)
                    || (!(can_path_n && ok_target_n && ok_bend_n && ok_thump_n) && no_tall_n));
            let col_w = test_w
                && s1.is_collide_w(w)
                && ((y1 != y2 || is_between)
                    || (!(can_path_w && ok_target_w && ok_bend_w && ok_thump_w) && no_tall_w));

            let ok_target_n = m.has_target || !s2.has(TARGET_PATH_N);
            let ok_target_w = m.has_target || !s2.has(TARGET_PATH_W);
            let ok_bend_n = m.can_bend || !s2.has(BENDABLE_N);
            let ok_bend_w = m.can_bend || !s2.has(BENDABLE_W);
            let ok_thump_n = m.can_thump || !s2.has(THUMP_N);
            let ok_thump_w = m.can_thump || !s2.has(THUMP_W);
            let no_tall_n = !m.can_climb_tall_fences || !s2.has(TALL_HOPPABLE_N);
            let no_tall_w = !m.can_climb_tall_fences || !s2.has(TALL_HOPPABLE_W);
            let can_path_n = s2.is_can_path_n(w);
            let can_path_w = s2.is_can_path_w(w);
            let col_s = test_s
                && s2.is_collide_n(w)
                && ((x1 != x2 || is_between)
                    || (!(can_path_n && ok_target_n && ok_bend_n && ok_thump_n) && no_tall_n));
            let col_e = test_e
                && s2.is_collide_w(w)
                && ((y1 != y2 || is_between)
                    || (!(can_path_w && ok_target_w && ok_bend_w && ok_thump_w) && no_tall_w));
            if col_n || col_w || col_s || col_e {
                return true;
            }
        } else {
            let open_door_n = test_n && s1.is_unblocked_door_n(w);
            let open_door_w = test_w && s1.is_unblocked_door_w(w);
            let col_n = test_n && (s1.is_collide_n(w) || s1.is_thump_n(w)) && !open_door_n;
            let col_w = test_w && (s1.is_collide_w(w) || s1.is_thump_w(w)) && !open_door_w;
            if test_n && s1.is_can_path_n(w) && (is_between || x1 != x2) {
                return true;
            }
            if test_w && s1.is_can_path_w(w) && (is_between || y1 != y2) {
                return true;
            }
            if (col_n || col_w) && !w.can_animal_break_obstacle(m, a, b, col_w, col_n, false, false)
            {
                return true;
            }
            let open_door_s = test_s && s2.is_unblocked_door_n(w);
            let open_door_e = test_e && s2.is_unblocked_door_w(w);
            let col_s = test_s && (s2.is_collide_n(w) || s2.is_thump_n(w)) && !open_door_s;
            let col_e = test_e && (s2.is_collide_w(w) || s2.is_thump_w(w)) && !open_door_e;
            if test_s && s2.is_can_path_n(w) && (is_between || x1 != x2) {
                return true;
            }
            if test_e && s2.is_can_path_w(w) && (is_between || y1 != y2) {
                return true;
            }
            if (col_s || col_e) && !w.can_animal_break_obstacle(m, a, b, false, false, col_e, col_s)
            {
                return true;
            }
        }
        if x2 == x1 || y2 == y1 {
            return false;
        }
        // Diagonal move: both orthogonal neighbours must be passable.
        let between_a = w.get_square(x1, y2, s1.z);
        let between_b = w.get_square(x2, y1, s1.z);
        if x2 == x1 + 1 && y2 == y1 + 1 {
            if let (Some(ba), Some(bb)) = (between_a, between_b) {
                let (sa, sb) = (w.sq(ba), w.sq(bb));
                if (sa.has(WINDOW_N) && sb.has(WINDOW_W)) || (sa.is_thump_n(w) && sb.is_thump_w(w))
                {
                    return true;
                }
            } else if between_a.is_none() {
                return true;
            }
        } else if x2 == x1 - 1 && y2 == y1 - 1 && between_a.is_some() && between_b.is_some() {
            let (sa, sb) = (w.sq(between_a.unwrap()), w.sq(between_b.unwrap()));
            if sa.has(WINDOW_W) && sb.has(WINDOW_N) {
                return true;
            }
            if sa.is_thump_w(w) && sb.is_thump_n(w) {
                return true;
            }
        } else if between_a.is_none() {
            return true;
        }
        let ba = between_a.unwrap();
        if w.can_not_move_between_m(m, a, ba, true) {
            return true;
        }
        let bb = match between_b {
            Some(bb) => bb,
            None => return true,
        };
        if w.can_not_move_between_m(m, a, bb, true) {
            return true;
        }
        if w.can_not_move_between_m(m, b, ba, true) {
            return true;
        }
        w.can_not_move_between_m(m, b, bb, true)
    }

    /// `SearchNode::c`: cost of stepping from `a` to its successor `b`.
    pub(crate) fn sn_cost(&self, a: u32, b: u32) -> f64 {
        let na = &self.vga.nodes[a as usize];
        let nb = &self.vga.nodes[b as usize];
        if nb.in_unloaded_area {
            return self.sn_dist(a, b);
        }
        let m = self.mover();
        let is_zombie = m.is_zombie();
        let crawling_zombie = is_zombie && m.crawling;
        let mut avoid_windows = !is_zombie || m.crawling;
        let animal_cant_climb = m.is_animal() && !m.can_climb_fences;
        avoid_windows |= animal_cant_climb;
        let mut add = 0.0f64;
        if avoid_windows {
            if let (Some(sa), Some(sb)) = (na.square, nb.square) {
                let (s1, s2) = (self.sq(sa), self.sq(sb));
                let pick =
                    |sq: &crate::square::Square, window: u64, unblocked: u64| -> Option<f64> {
                        if sq.has(window) {
                            Some(if !crawling_zombie && sq.has(unblocked) {
                                20.0
                            } else {
                                200.0
                            })
                        } else {
                            Some(0.0)
                        }
                    };
                let r = if s1.x == s2.x - 1 && s1.y == s2.y {
                    pick(s2, WINDOW_W, WINDOW_W_UNBLOCKED)
                } else if s1.x == s2.x + 1 && s1.y == s2.y {
                    pick(s1, WINDOW_W, WINDOW_W_UNBLOCKED)
                } else if s1.x == s2.x && s1.y == s2.y - 1 {
                    pick(s2, WINDOW_N, WINDOW_N_UNBLOCKED)
                } else if s1.x == s2.x && s1.y == s2.y + 1 {
                    pick(s1, WINDOW_N, WINDOW_N_UNBLOCKED)
                } else {
                    None
                };
                if let Some(v) = r {
                    add = v;
                }
            }
        }
        if let Some(sb) = nb.square {
            if self.sq(sb).has(THUMPABLE) {
                add = add.max_native(20.0);
            }
        }
        if m.avoid_farming_plants {
            if let Some(sb) = nb.square {
                if self.sq(sb).has(FARMING_PLANT) {
                    add = add.max_native(20.0);
                }
            }
        }
        if let (Some(va), Some(vb)) = (na.vg_node, nb.vg_node) {
            for &c in &self.node(va).visible {
                let cxn = self.vis.connections[c];
                if cxn.other(va) == vb {
                    let thumpable_square = self
                        .node(va)
                        .square
                        .map(|s| self.sq(s).has(THUMPABLE))
                        .unwrap_or(false);
                    if !thumpable_square && cxn.has(CXN_FLAG_THUMP) {
                        add = add.max_native(20.0);
                    }
                    break;
                }
            }
        }
        let sq_of = |n: &SearchNode| -> Option<SquareId> {
            match n.square {
                Some(s) => Some(s),
                None => {
                    let v = self.node(n.vg_node.unwrap());
                    self.get_square(fastfloor(v.x), fastfloor(v.y), v.z)
                }
            }
        };
        let square1 = sq_of(na);
        let square2 = sq_of(nb);
        if let (Some(q1), Some(q2)) = (square1, square2) {
            let (s1, s2) = (self.sq(q1), self.sq(q2));
            let east = s1.x == s2.x - 1 && s1.y == s2.y;
            let west = s1.x == s2.x + 1 && s1.y == s2.y;
            let south = s1.x == s2.x && s1.y == s2.y - 1;
            let north = s1.x == s2.x && s1.y == s2.y + 1;
            if east {
                if s2.has(THUMP_W) || s2.has(TALL_HOPPABLE_W) {
                    add = add.max_native(20.0);
                }
            } else if west {
                if s1.has(THUMP_W) || s1.has(TALL_HOPPABLE_W) {
                    add = add.max_native(20.0);
                }
            } else if south {
                if s2.has(THUMP_N) || s2.has(TALL_HOPPABLE_N) {
                    add = add.max_native(20.0);
                }
            } else if north && (s1.has(THUMP_N) || s1.has(TALL_HOPPABLE_N)) {
                add = add.max_native(20.0);
            }
            if crawling_zombie || animal_cant_climb {
                let animal = m.is_animal();
                if east {
                    if s2.has(COLLIDE_W)
                        && s2.has(CAN_PATH_W)
                        && (!animal || !s2.is_unblocked_door_w(self))
                    {
                        add = add.max_native(20.0);
                    }
                } else if west {
                    if s1.has(COLLIDE_W)
                        && s1.has(CAN_PATH_W)
                        && (!animal || !s1.is_unblocked_door_w(self))
                    {
                        add = add.max_native(20.0);
                    }
                } else if south {
                    if s2.has(COLLIDE_N)
                        && s2.has(CAN_PATH_N)
                        && (!animal || !s2.is_unblocked_door_n(self))
                    {
                        add = add.max_native(20.0);
                    }
                } else if north
                    && s1.has(COLLIDE_N)
                    && s1.has(CAN_PATH_N)
                    && (!animal || !s1.is_unblocked_door_n(self))
                {
                    add = add.max_native(20.0);
                }
            }
            if is_zombie {
                // The native code tests the north target-path bit of the
                // first square for both vertical directions.
                if east {
                    if s2.has(TARGET_PATH_W) {
                        add = add.max_native(20.0);
                    }
                } else if west {
                    if s1.has(TARGET_PATH_W) {
                        add = add.max_native(20.0);
                    }
                } else if (south || north) && s1.has(TARGET_PATH_N) {
                    add = add.max_native(20.0);
                }
            }
        }
        // PolygonalMap2 PATHS_UNDER_VEHICLES is always on.
        let self_not_under = match na.vg_node {
            None => true,
            Some(v) => !self.node(v).has_flag(NODE_FLAG_CRAWL_INTERIOR),
        };
        if let Some(vb) = nb.vg_node {
            if self.node(vb).has_flag(NODE_FLAG_CRAWL_INTERIOR)
                && self_not_under
                && !m.ignore_crawl_cost
            {
                add += 10.0;
            }
        }
        if let Some(sb) = nb.square {
            add += self.sq(sb).cost as f64;
        }
        add + self.sn_dist(a, b)
    }

    fn push_unique(out: &mut Vec<u32>, n: u32) {
        if !out.contains(&n) {
            out.push(n);
        }
    }

    /// `SearchNode::getSuccessors`.
    pub(crate) fn sn_successors(&mut self, i: u32, out: &mut Vec<u32>) {
        let goal = self.vga.goal;
        if self.vga.nodes[goal as usize].in_unloaded_area && self.sn_is_on_edge_of_loaded_area(i) {
            out.push(goal);
        }
        let this_square = self.vga.nodes[i as usize].square;
        if let Some(vg) = self.vga.nodes[i as usize].vg_node {
            self.node_create_graphs_if_needed(vg);
            let visible = self.node(vg).visible.clone();
            for c in visible {
                let cxn = self.vis.connections[c];
                let other = cxn.other(vg);
                let sn = self.vga_search_node_for_node(other);
                if let (Some(a), Some(b)) =
                    (self.node(vg).square, self.vga.nodes[sn as usize].square)
                {
                    if self.is_known_blocked(a, b) {
                        continue;
                    }
                }
                let m = self.mover();
                if !m.can_crawl && self.node(other).has_flag(NODE_FLAG_CRAWL_INTERIOR) {
                    continue;
                }
                if !m.can_thump && cxn.has(CXN_FLAG_THUMP) {
                    continue;
                }
                if !m.has_target && cxn.has(CXN_FLAG_TARGET_PATH) {
                    continue;
                }
                if !m.can_bend && cxn.has(CXN_FLAG_BENDABLE) {
                    continue;
                }
                out.push(sn);
            }
            let has_graphs = !self.node(vg).graphs.is_empty();
            if has_graphs {
                if self.node(vg).has_flag(NODE_FLAG_STAIR) {
                    let s = this_square.expect("stair node has a square");
                    let (x, y, z) = (self.sq(s).x, self.sq(s).y, self.sq(s).z);
                    if let Some(s1) = self.get_square(x - 1, y, z) {
                        if self.sq(s1).has(STAIR_BW) {
                            if !self.is_allowed_chunk_level(s1) {
                                return;
                            }
                            if self.can_move_between(s, s1, false) {
                                let sn = self.vga_search_node_for_square(s1);
                                Self::push_unique(out, sn);
                            }
                        }
                    }
                    let s1 = match self.get_square(x, y - 1, z) {
                        Some(s1) => s1,
                        None => return,
                    };
                    if !self.sq(s1).has(STAIR_BN)
                        || !self.is_allowed_chunk_level(s1)
                        || !self.can_move_between(s, s1, false)
                    {
                        return;
                    }
                    let sn = self.vga_search_node_for_square(s1);
                    Self::push_unique(out, sn);
                    return;
                }
                if !self.node(vg).has_flag(NODE_FLAG_PERIMETER) {
                    return;
                }
            }
        }
        let s = match this_square {
            Some(s) => s,
            None => return,
        };
        let (x, y, z) = (self.sq(s).x, self.sq(s).y, self.sq(s).z);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                if let Some(s1) = self.get_square(x + dx, y + dy, z) {
                    if self.is_allowed_chunk_level(s1)
                        && (!self.is_square_in_cluster(s1) || self.sq(s1).has(ALL_STAIRS))
                        && self.can_move_between(s, s1, false)
                    {
                        let sn = self.vga_search_node_for_square(s1);
                        Self::push_unique(out, sn);
                    }
                }
            }
        }
        if self.sq(s).has(STAIR_BW | STAIR_BN) {
            let dir = if self.sq(s).has(STAIR_BN) {
                Dir::N
            } else {
                Dir::W
            };
            if let Some(s1) = self.adjacent_square(s, dir.rot180()) {
                if let Some(n) = self.get_existing_node_for_square(s1) {
                    if self.node(n).has_flag(NODE_FLAG_STAIR)
                        && self.is_allowed_chunk_level(s1)
                        && self.can_move_between(s, s1, false)
                    {
                        let sn = self.vga_search_node_for_square(s1);
                        Self::push_unique(out, sn);
                    }
                }
            }
        }
        let (min_level, max_level) = (self.mover().min_level, self.mover().max_level);
        if z > min_level {
            for (dx, dy, dir) in [
                (0, 1, Dir::N),
                (0, -1, Dir::S),
                (1, 0, Dir::W),
                (-1, 0, Dir::E),
            ] {
                if let Some(s1) = self.get_square(x + dx, y + dy, z - 1) {
                    if self.sq(s1).has_transition_to_level_above(dir)
                        && !self.is_square_in_cluster(s1)
                        && self.is_allowed_level_transition(s, true)
                    {
                        let sn = self.vga_search_node_for_square(s1);
                        Self::push_unique(out, sn);
                    }
                }
            }
        }
        if z >= max_level {
            return;
        }
        for (dx, dy, dir) in [
            (0, -1, Dir::N),
            (0, 1, Dir::S),
            (-1, 0, Dir::W),
            (1, 0, Dir::E),
        ] {
            if self.sq(s).has_transition_to_level_above(dir) {
                if let Some(s1) = self.get_square(x + dx, y + dy, z + 1) {
                    if !self.is_square_in_cluster(s1) && self.is_allowed_level_transition(s1, true)
                    {
                        let sn = self.vga_search_node_for_square(s1);
                        Self::push_unique(out, sn);
                    }
                }
            }
        }
    }

    /// `PMMover::isAllowedChunkLevel`.
    pub(crate) fn is_allowed_chunk_level(&mut self, s: SquareId) -> bool {
        if self.mover().allowed_chunk_levels.is_empty() {
            return true;
        }
        let (x, y, z) = (self.sq(s).x, self.sq(s).y, self.sq(s).z);
        let c = match self.get_chunk_from_square_pos(x, y) {
            Some(c) => c,
            None => return false,
        };
        let r = crate::world::LevelRef { chunk: c, z };
        self.get_high_level_data(r);
        self.mover().allowed_chunk_levels.contains(&r)
    }

    /// `PMMover::isAllowedLevelTransition(dir, square, bTopFloorSquare)`.
    pub(crate) fn is_allowed_level_transition(&self, s: SquareId, top_floor: bool) -> bool {
        let m = self.mover();
        if m.allowed_level_transitions.is_empty() {
            return true;
        }
        m.allowed_level_transitions.iter().any(|&t| {
            let sq = if top_floor {
                self.trans_top_floor_square(t)
            } else {
                self.trans_bottom_floor_square(t)
            };
            sq == Some(s)
        })
    }

    /// `VGAStar::shortestPath`: run the search from `start` to `goal`.
    pub(crate) fn vga_shortest_path(&mut self, start: u32, goal: u32) -> Option<Vec<u32>> {
        self.vga.goal = goal;
        let mut host = VgHost { w: self };
        astar::shortest_path(&mut host, start)
    }
}

/// `std::max(double, double)` as the native code evaluates it
/// (`a >= b ? a : b` with the constant on the right).
trait MaxNative {
    fn max_native(self, b: f64) -> f64;
}

impl MaxNative for f64 {
    #[inline]
    fn max_native(self, b: f64) -> f64 {
        if b <= self {
            self
        } else {
            b
        }
    }
}

struct VgHost<'a> {
    w: &'a mut World,
}

impl AStarHost for VgHost<'_> {
    fn g(&self, n: u32) -> f64 {
        self.w.vga.nodes[n as usize].g
    }
    fn set_g(&mut self, n: u32, g: f64) {
        self.w.vga.nodes[n as usize].g = g;
    }
    fn h(&mut self, n: u32) -> f64 {
        let goal = self.w.vga.goal;
        self.w.sn_dist(n, goal)
    }
    fn c(&mut self, from: u32, to: u32) -> f64 {
        self.w.sn_cost(from, to)
    }
    fn successors(&mut self, n: u32, out: &mut Vec<u32>) {
        self.w.sn_successors(n, out);
    }
    fn parent(&self, n: u32) -> Option<u32> {
        self.w.vga.nodes[n as usize].parent
    }
    fn set_parent(&mut self, n: u32, parent: Option<u32>) {
        self.w.vga.nodes[n as usize].parent = parent;
    }
    fn depth(&self, n: u32) -> i32 {
        self.w.vga.nodes[n as usize].depth
    }
    fn set_depth(&mut self, n: u32, depth: i32) {
        self.w.vga.nodes[n as usize].depth = depth;
    }
    fn total(&self, n: u32) -> f32 {
        self.w.vga.nodes[n as usize].total
    }
    fn set_total(&mut self, n: u32, total: f32) {
        self.w.vga.nodes[n as usize].total = total;
    }
    fn flags(&self, n: u32) -> u8 {
        self.w.vga.nodes[n as usize].flags
    }
    fn set_flags(&mut self, n: u32, flags: u8) {
        self.w.vga.nodes[n as usize].flags = flags;
    }
    fn in_goal(&self, n: u32) -> bool {
        n == self.w.vga.goal
    }
}
