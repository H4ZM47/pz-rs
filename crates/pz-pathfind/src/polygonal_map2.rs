//! `PolygonalMap2`: request handling (`findPath`,
//! `findPathHighLevelThenLowLevel`), path post-processing (`cleanPath`,
//! `smoothPath`, `addPathSegmentNodes`) and the square-node helpers.

use crate::bytes::{ByteWriter, PathfindError};
use crate::path::{Path, PathNode, NODE_FLAG_CRAWL, NODE_FLAG_CRAWL_INTERIOR, NODE_FLAG_KEEP};
use crate::request::{NativeRequest, PathFindRequest, PmMover};
use crate::square::bits::*;
use crate::util::fastfloor;
use crate::visibility::{AdjustStartEndNodeData, NodeId, VGraphId};
use crate::world::{MoverSel, SquareId, World};

/// `LineClearCollide` flag: allow the line to touch obstacle edges.
pub const LCC_ALLOW_ON_EDGE: i32 = 16;
/// `LineClearCollide` flag: any square with a cost blocks the line.
pub const LCC_CHECK_COST: i32 = 4;
/// `LineClearCollide` flag (native only): farming plants block the line.
pub const LCC_AVOID_FARMING_PLANTS: i32 = 32;

impl World {
    /// `PolygonalMap2::getNodeForSquare`: the square's node, created (and
    /// remembered until the next rebuild) on first use.
    pub(crate) fn get_node_for_square(&mut self, s: SquareId) -> NodeId {
        if let Some(&n) = self.square_to_node.get(&s.raw()) {
            return n;
        }
        let n = self.node_alloc_square(s);
        self.square_to_node.insert(s.raw(), n);
        n
    }

    /// `PolygonalMap2::getExistingNodeForSquare`.
    pub(crate) fn get_existing_node_for_square(&self, s: SquareId) -> Option<NodeId> {
        self.square_to_node.get(&s.raw()).copied()
    }

    /// `PolygonalMap2::isBlockedInAllDirections`.
    pub(crate) fn is_blocked_in_all_directions(&self, x: i32, y: i32, z: i32) -> bool {
        let s = match self.get_square(x, y, z) {
            Some(s) => s,
            None => return false,
        };
        let blocked = |nx: i32, ny: i32| -> bool {
            match self.get_square(nx, ny, z) {
                Some(n) => self.can_not_move_between(s, n, false),
                None => false,
            }
        };
        let n = blocked(x, y - 1);
        let so = blocked(x, y + 1);
        let w = blocked(x - 1, y);
        let e = blocked(x + 1, y);
        n && so && w && e
    }

    /// `PolygonalMap2::canMoveBetween(mover, ...)` using a specific mover.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn can_move_between_with(
        &self,
        sel: MoverSel,
        x1: i32,
        y1: i32,
        z1: i32,
        x2: i32,
        y2: i32,
        z2: i32,
    ) -> bool {
        let a = self.get_square(x1, y1, z1);
        let c2 = match self.get_chunk_from_square_pos(x2, y2) {
            Some(c) => c,
            None => return false,
        };
        let b = self.chunk_get_square(c2, x2, y2, z2);
        let (a, b) = match (a, b) {
            (Some(a), Some(b)) => (a, b),
            _ => return false,
        };
        // The native code temporarily swaps the search's mover.
        let m = match sel {
            MoverSel::Request => &self.request_mover,
            MoverSel::FloodFill => &self.flood_mover,
        };
        !self.can_not_move_between_m(m, a, b, false)
    }

    /// `PolygonalMap2::getPointOutsideObjects`: nudge a goal inside a
    /// square away from walls and diagonal corners.
    pub(crate) fn get_point_outside_objects(
        &mut self,
        s: SquareId,
        target_x: f32,
        target_y: f32,
    ) -> NodeId {
        const R: f32 = 0.3;
        let sq = self.sq(s).clone();
        let get = |dx: i32, dy: i32| {
            self.get_square(sq.x + dx, sq.y + dy, sq.z)
                .map(|id| self.sq(id))
        };
        let w = get(-1, 0);
        let nw = get(-1, -1);
        let n = get(0, -1);
        let ne = get(1, -1);
        let e = get(1, 0);
        let se = get(1, 1);
        let so = get(0, 1);
        let sw = get(-1, 1);
        let mut x1 = sq.x as f32;
        let mut y1 = sq.y as f32;
        let mut x2 = sq.x as f32 + 1.0;
        let mut y2 = sq.y as f32 + 1.0;
        if sq.is_collide_w(self) {
            x1 += 0.35000002;
        }
        if sq.is_collide_n(self) {
            y1 += 0.35000002;
        }
        if let Some(e) = e {
            if e.has(COLLIDE_W) || e.has(ALL_STAIRS) || e.is_really_solid(self) {
                x2 -= 0.35000002;
            }
        }
        if let Some(so) = so {
            if so.has(COLLIDE_N) || so.has(ALL_STAIRS) || so.is_really_solid(self) {
                y2 -= 0.35000002;
            }
        }
        let mut tx = crate::util::clamp(target_x, x1, x2);
        let mut ty = crate::util::clamp(target_y, y1, y2);
        let fx = sq.x as f32;
        let fy = sq.y as f32;
        if tx <= fx + R && ty <= fy + R {
            let mut solid = nw
                .map(|q| q.has(ALL_STAIRS) || q.is_really_solid(self))
                .unwrap_or(false);
            solid |= n.map(|q| q.has(COLLIDE_W)).unwrap_or(false);
            solid |= w.map(|q| q.has(COLLIDE_N)).unwrap_or(false);
            if solid {
                let x3 = fx + R + 0.05;
                let y3 = fy + R + 0.05;
                if x3 - tx <= y3 - ty {
                    tx = x3;
                } else {
                    ty = y3;
                }
            }
        }
        if tx >= fx + 1.0 - R && ty <= fy + R {
            let mut solid = ne
                .map(|q| q.has(COLLIDE_W) || q.has(ALL_STAIRS) || q.is_really_solid(self))
                .unwrap_or(false);
            solid |= e.map(|q| q.has(COLLIDE_N)).unwrap_or(false);
            if solid {
                let x3 = fx + 1.0 - R - 0.05;
                let y3 = fy + R + 0.05;
                if tx - x3 <= y3 - ty {
                    tx = x3;
                } else {
                    ty = y3;
                }
            }
        }
        if tx <= fx + R && ty >= (sq.y + 1) as f32 - R {
            let mut solid = sw
                .map(|q| q.has(COLLIDE_N) || q.has(ALL_STAIRS) || q.is_really_solid(self))
                .unwrap_or(false);
            solid |= so.map(|q| q.has(COLLIDE_W)).unwrap_or(false);
            if solid {
                let x3 = fx + R + 0.05;
                let y3 = fy + 1.0 - R - 0.05;
                if x3 - tx <= ty - y3 {
                    tx = x3;
                } else {
                    ty = y3;
                }
            }
        }
        if tx >= (sq.x + 1) as f32 - R && ty >= fy + 1.0 - R {
            let solid = se
                .map(|q| {
                    q.has(COLLIDE_W)
                        || q.has(COLLIDE_N)
                        || q.has(ALL_STAIRS)
                        || q.is_really_solid(self)
                })
                .unwrap_or(false);
            if solid {
                let x3 = fx + 1.0 - R - 0.05;
                let y3 = fy + 1.0 - R - 0.05;
                if tx - x3 <= ty - y3 {
                    tx = x3;
                } else {
                    ty = y3;
                }
            }
        }
        self.node_alloc(tx, ty, sq.z)
    }

    /// The square at a request coordinate if a mover may stand on it
    /// (shared start/goal check of the native `findPath`).
    fn usable_end_square(&self, x: f32, y: f32, z: f32) -> Option<SquareId> {
        let s = self.get_square(fastfloor(x), fastfloor(y), fastfloor(z))?;
        let sq = self.sq(s);
        if sq.is_really_solid(self) && sq.is_non_thumpable_solid(self) {
            return None;
        }
        if !self.mover().can_thump && sq.has_square_thumpable(self) {
            return None;
        }
        Some(s)
    }

    /// `PolygonalMap2::findPath`: one low-level search on the request's
    /// current start/target.
    pub(crate) fn pm_find_path(&mut self, req: &mut NativeRequest) -> bool {
        let start_z = req.start_z;
        let target_z = req.target_z;
        if fastfloor(start_z) == fastfloor(target_z) {
            let m = self.mover();
            let mut flags = if m.is_zombie() { 0x10 } else { 0x14 };
            flags |= (m.avoid_farming_plants as i32) << 5;
            if !self.lcc_is_not_clear(
                req.start_x,
                req.start_y,
                req.target_x,
                req.target_y,
                fastfloor(start_z),
                flags,
            ) {
                req.path.add_node(req.start_x, req.start_y, req.start_z, 0);
                req.path
                    .add_node(req.target_x, req.target_y, req.target_z, 0);
                return true;
            }
        }
        self.vga_init();

        let mut st = FindPathState::default();
        let ok = self.pm_find_path_inner(req, &mut st);
        self.pm_find_path_cleanup(&st);
        ok
    }

    fn pm_find_path_inner(&mut self, req: &mut NativeRequest, st: &mut FindPathState) -> bool {
        let start_z = req.start_z;
        let target_z = req.target_z;
        let square = match self.usable_end_square(req.start_x, req.start_y, start_z) {
            Some(s) => s,
            None => return false,
        };
        let mut start_node: Option<u32> = None;
        if self.sq(square).has(ALL_STAIRS) {
            start_node = Some(self.vga_search_node_for_square(square));
        } else if let Some(vg) = self.get_vis_graph_for_square(square, 0) {
            if !self.vis.graphs_pool[vg].created {
                self.graph_create(vg);
            }
            let adjusted = self.graph_get_point_outside_obstacles(
                vg,
                req.start_x,
                req.start_y,
                start_z,
                &mut st.adjust_start,
            );
            if adjusted == -1 {
                return false;
            }
            let mut vg_node = None;
            if adjusted == 1 {
                st.adjust_start_flag = true;
                if st.adjust_start.is_node_new {
                    st.remove_start = Some(vg);
                }
                vg_node = st.adjust_start.node;
            }
            let vg_node = match vg_node {
                Some(n) => n,
                None => {
                    let n = self.node_alloc(req.start_x, req.start_y, fastfloor(start_z));
                    self.graph_add_node(vg, n);
                    st.remove_start = Some(vg);
                    n
                }
            };
            start_node = Some(self.vga_search_node_for_node(vg_node));
        }
        let start_node = match start_node {
            Some(n) => n,
            None => self.vga_search_node_for_square(square),
        };
        st.start_node = Some(start_node);

        let goal_node;
        if self
            .get_chunk_from_square_pos(fastfloor(req.target_x), fastfloor(req.target_y))
            .is_none()
        {
            goal_node =
                self.vga_search_node_unloaded(fastfloor(req.target_x), fastfloor(req.target_y));
        } else {
            let square = match self.usable_end_square(req.target_x, req.target_y, target_z) {
                Some(s) => s,
                None => return false,
            };
            let same_square = fastfloor(req.start_x) == fastfloor(req.target_x)
                && fastfloor(req.start_y) == fastfloor(req.target_y)
                && fastfloor(req.start_z) == fastfloor(req.target_z);
            if !same_square
                && self.is_blocked_in_all_directions(
                    fastfloor(req.target_x),
                    fastfloor(req.target_y),
                    fastfloor(target_z),
                )
            {
                return false;
            }
            let mut g: Option<u32> = None;
            if self.sq(square).has(ALL_STAIRS) {
                g = Some(self.vga_search_node_for_square(square));
            } else if let Some(vg) = self.get_vis_graph_for_square(square, 0) {
                if !self.vis.graphs_pool[vg].created {
                    self.graph_create(vg);
                }
                let adjusted = self.graph_get_point_outside_obstacles(
                    vg,
                    req.target_x,
                    req.target_y,
                    target_z,
                    &mut st.adjust_goal,
                );
                if adjusted == -1 {
                    return false;
                }
                let mut vg_node = None;
                if adjusted == 1 {
                    st.adjust_goal_flag = true;
                    if st.adjust_goal.is_node_new {
                        st.remove_goal = Some(vg);
                    }
                    vg_node = st.adjust_goal.node;
                }
                let vg_node = match vg_node {
                    Some(n) => n,
                    None => {
                        let n = self.node_alloc(req.target_x, req.target_y, fastfloor(target_z));
                        self.graph_add_node(vg, n);
                        st.remove_goal = Some(vg);
                        n
                    }
                };
                g = Some(self.vga_search_node_for_node(vg_node));
            } else if let Some(vg) = self.get_vis_graph_for_square(square, 1) {
                let n = self.get_point_outside_objects(square, req.target_x, req.target_y);
                self.graph_add_node(vg, n);
                if self.node(n).x != req.target_x || self.node(n).y != req.target_y {
                    st.adjust_goal_flag = true;
                    st.adjust_goal.is_node_new = false;
                }
                st.remove_goal = Some(vg);
                g = Some(self.vga_search_node_for_node(n));
            }
            goal_node = match g {
                Some(g) => g,
                None => self.vga_search_node_for_square(square),
            };
        }
        st.goal_node = Some(goal_node);

        let path = match self.vga_shortest_path(start_node, goal_node) {
            Some(p) => p,
            None => return false,
        };
        if path.len() == 1 {
            self.path_add_search_node(&mut req.path, start_node);
            let goal_square = self.vga.nodes[goal_node as usize].square;
            let keep_target = !st.adjust_goal_flag
                && goal_square.is_some_and(|s| {
                    let sq = self.sq(s);
                    sq.x as f32 + 0.5 != req.target_x && sq.y as f32 + 0.5 != req.target_y
                });
            if keep_target {
                req.path.add_node(req.target_x, req.target_y, target_z, 0);
            } else {
                self.path_add_search_node(&mut req.path, goal_node);
            }
        } else {
            self.clean_path(&path, req, st.adjust_goal_flag, goal_node);
            let m = self.mover();
            if m.is_player() && !m.is_npc {
                let flags = if m.avoid_farming_plants { 0x34 } else { 0x14 };
                self.smooth_path(&mut req.path, flags);
            }
        }
        true
    }

    /// The `final_action` at the end of the native `findPath`.
    fn pm_find_path_cleanup(&mut self, st: &FindPathState) {
        if let Some(vg) = st.remove_start {
            let n = self.vga.nodes[st.start_node.unwrap() as usize]
                .vg_node
                .unwrap();
            self.graph_remove_node(vg, n);
            self.node_release(n);
        }
        if let Some(vg) = st.remove_goal {
            let n = self.vga.nodes[st.goal_node.unwrap() as usize]
                .vg_node
                .unwrap();
            self.graph_remove_node(vg, n);
            self.node_release(n);
        }
        self.vga.nodes.clear();
        self.vga.node_map.clear();
        self.vga.square_map.clear();
        for (flag, data) in [
            (st.adjust_start_flag, &st.adjust_start),
            (st.adjust_goal_flag, &st.adjust_goal),
        ] {
            if flag && data.is_node_new {
                self.undo_adjust_start_end(data);
            }
        }
    }

    /// `Path::addNode(SearchNode*)`.
    fn path_add_search_node(&self, path: &mut Path, sn: u32) {
        let flags = match self.vga.nodes[sn as usize].vg_node {
            Some(v) => self.node(v).flags,
            None => 0,
        };
        path.add_node(self.sn_x(sn), self.sn_y(sn), self.sn_z(sn), flags);
    }

    /// `PolygonalMap2::cleanPath`: turn the search-node chain into path
    /// nodes, merging straight runs of squares.
    fn clean_path(
        &mut self,
        nodes: &[u32],
        req: &mut NativeRequest,
        adjust_goal: bool,
        goal_node: u32,
    ) {
        let is_npc = self.mover().is_npc;
        let mut square_prev: Option<SquareId> = None;
        let mut dx_old = -123;
        let mut dy_old = -123;
        for (i, &sn) in nodes.iter().enumerate() {
            let x = self.sn_x(sn);
            let y = self.sn_y(sn);
            let z = self.sn_z(sn);
            let mut flags = match self.vga.nodes[sn as usize].vg_node {
                Some(v) => self.node(v).flags,
                None => 0,
            };
            let square_cur = self.vga.nodes[sn as usize].square;
            let mut extend = false;
            let mut same_run = false;
            match (square_cur, square_prev) {
                (Some(cur), Some(prev)) if self.sq(cur).z == self.sq(prev).z => {
                    let (c, p) = (self.sq(cur), self.sq(prev));
                    let dx = c.x - p.x;
                    let dy = c.y - p.y;
                    if dx == dx_old && dy == dy_old {
                        same_run = true;
                        extend = req.path.len() >= 2
                            && !req.path.nodes.last().unwrap().has_flag(NODE_FLAG_KEEP);
                        if (dx == 0 && dy == -1 && p.has(CAN_PATH_N))
                            || (dx == 0 && dy == 1 && c.has(CAN_PATH_N))
                            || (dx == -1 && dy == 0 && p.has(CAN_PATH_W))
                            || (dx == 1 && dy == 0 && c.has(CAN_PATH_W))
                        {
                            extend = false;
                        }
                    } else {
                        dx_old = dx;
                        dy_old = dy;
                    }
                }
                _ => {
                    dx_old = -123;
                    dy_old = -123;
                }
            }
            let _ = same_run;
            square_prev = square_cur;
            if is_npc {
                extend = false;
            }
            if extend {
                let c = self.sq(square_cur.unwrap());
                let last = req.path.nodes.last_mut().unwrap();
                last.x = c.x as f32 + 0.5;
                last.y = c.y as f32 + 0.5;
                continue;
            }
            if req.path.len() > 1 {
                let last = req.path.nodes.last_mut().unwrap();
                if last.is_approximately_equal(x, y, z) {
                    last.x = x;
                    last.y = y;
                    last.z = z;
                    continue;
                }
            }
            if i > 0 {
                if let (Some(cur), Some(prev)) =
                    (square_cur, self.vga.nodes[nodes[i - 1] as usize].square)
                {
                    let (c, p) = (self.sq(cur), self.sq(prev));
                    let dx = c.x - p.x;
                    let dy = c.y - p.y;
                    if (dx == 0 && dy == -1 && p.has(CAN_PATH_N))
                        || (dx == 0 && dy == 1 && c.has(CAN_PATH_N))
                        || (dx == -1 && dy == 0 && p.has(CAN_PATH_W))
                        || (dx == 1 && dy == 0 && c.has(CAN_PATH_W))
                    {
                        flags |= NODE_FLAG_KEEP;
                    }
                }
            }
            req.path.add_node(x, y, z, flags);
        }
        let m = self.mover();
        if m.is_player() && !is_npc && !adjust_goal {
            if let Some(gs) = self.vga.nodes[goal_node as usize].square {
                let g = self.sq(gs);
                let dx = req.target_x - (g.x as f32 + 0.5);
                let dy = req.target_y - (g.y as f32 + 0.5);
                if dx.mul_add(dx, dy * dy) as f64 > 0.01 {
                    req.path
                        .add_node(req.target_x, req.target_y, req.target_z, 0);
                }
            }
        }
        // PATHS_UNDER_VEHICLES: drop crawl flags from isolated crawl nodes.
        let n = req.path.nodes.len();
        let mut prev: Option<PathNode> = None;
        for i in 0..n {
            let next = if i + 1 < n {
                Some(req.path.nodes[i + 1])
            } else {
                None
            };
            let node = &mut req.path.nodes[i];
            if node.has_flag(NODE_FLAG_CRAWL)
                && prev
                    .map(|p| !p.has_flag(NODE_FLAG_CRAWL_INTERIOR))
                    .unwrap_or(true)
                && next
                    .map(|p| !p.has_flag(NODE_FLAG_CRAWL_INTERIOR))
                    .unwrap_or(true)
            {
                node.flags &= !3;
            }
            prev = Some(*node);
        }
    }

    /// `PolygonalMap2::smoothPath`: drop path nodes that can be skipped by
    /// a clear straight line.
    fn smooth_path(&mut self, path: &mut Path, flags: i32) {
        let mut i = 0usize;
        while path.len() > 2 && i < path.len() - 2 {
            let n1 = path.nodes[i];
            let n2 = path.nodes[i + 1];
            let n3 = path.nodes[i + 2];
            if fastfloor(n1.z) != fastfloor(n2.z) || fastfloor(n1.z) != fastfloor(n3.z) {
                i += 1;
                continue;
            }
            if self.lcc_is_not_clear(n1.x, n1.y, n3.x, n3.y, fastfloor(n1.z), flags) {
                i += 1;
                continue;
            }
            path.nodes.remove(i + 1);
        }
    }

    /// `PolygonalMap2::addPathSegmentNodes`.
    fn add_path_segment_nodes(src: &Path, dest: &mut Path) {
        for &node in &src.nodes {
            if dest.len() >= 2 {
                let last = dest.nodes.last().unwrap();
                if (node.x - last.x).abs() < 0.01f32
                    && (node.y - last.y).abs() < 0.01f32
                    && (node.z - last.z).abs() < 0.01f32
                {
                    continue;
                }
            }
            dest.nodes.push(node);
        }
    }

    /// `PolygonalMap2::findPathHighLevelThenLowLevel`: plan the chunk
    /// levels and level transitions first, then run the low-level search
    /// for each level segment.
    pub(crate) fn find_path_high_level_then_low_level(&mut self, req: &mut NativeRequest) -> bool {
        self.vga_init();
        let (chunk_levels, transitions, bottoms) = self.hl_find_path(req);
        if chunk_levels.is_empty() {
            return false;
        }
        self.request_mover.allowed_chunk_levels = chunk_levels;
        self.request_mover.allowed_level_transitions = transitions.clone();
        if transitions.is_empty() {
            return self.pm_find_path(req);
        }
        let mut temp = Path::default();
        let (mut sx, mut sy, mut sz) = (req.start_x, req.start_y, req.start_z);
        let (tx, ty, tz) = (req.target_x, req.target_y, req.target_z);
        let _ = self.hl_get_level_transition_at(fastfloor(sx), fastfloor(sy), fastfloor(sz));
        let target_on_transition = self
            .hl_get_level_transition_at(fastfloor(tx), fastfloor(ty), fastfloor(tz))
            .is_some();
        let n = transitions.len();
        for (i, &lt) in transitions.iter().enumerate() {
            if target_on_transition && i == n - 1 {
                break;
            }
            req.start_x = sx;
            req.start_y = sy;
            req.start_z = sz;
            let t = self.trans(lt);
            if bottoms[i] {
                req.target_x = t.bottom_floor_x() as f32 + 0.5;
                req.target_y = t.bottom_floor_y() as f32 + 0.5;
                req.target_z = t.bottom_floor_z() as f32;
            } else {
                req.target_x = t.top_floor_x() as f32 + 0.5;
                req.target_y = t.top_floor_y() as f32 + 0.5;
                req.target_z = t.top_floor_z() as f32;
            }
            if !self.pm_find_path(req) {
                return false;
            }
            Self::add_path_segment_nodes(&req.path, &mut temp);
            req.path.clear();
            sx = req.target_x;
            sy = req.target_y;
            sz = req.target_z;
        }
        req.start_x = sx;
        req.start_y = sy;
        req.start_z = sz;
        req.target_x = tx;
        req.target_y = ty;
        req.target_z = tz;
        if !self.pm_find_path(req) {
            return false;
        }
        Self::add_path_segment_nodes(&req.path, &mut temp);
        req.path.clear();
        req.path.nodes.append(&mut temp.nodes);
        true
    }

    /// Consume the rebuild flag the way the JNI `findPath` does before
    /// every request.
    pub(crate) fn rebuild_if_needed(&mut self) {
        if !self.rebuild {
            return;
        }
        let graphs = std::mem::take(&mut self.vis.graphs);
        for g in graphs {
            self.graph_release(g);
        }
        let nodes: Vec<NodeId> = self.square_to_node.values().copied().collect();
        for n in nodes {
            self.node_release(n);
        }
        self.square_to_node.clear();
        self.vis.vehicles = self.vis.vehicle_map.values().copied().collect();
        self.create_visibility_graphs();
        self.rebuild = false;
        self.modification_count += 1;
    }

    /// `PathfindNative.findPath(requestBuffer, pathBuffer)`.
    ///
    /// Returns `Ok(None)` when no path was found (native result 0) and the
    /// path otherwise (native result 1). Path nodes keep the +32 level
    /// offset, exactly as written to the Java buffer.
    pub fn find_path_bytes(&mut self, request: &[u8]) -> Result<Option<Path>, PathfindError> {
        self.rebuild_if_needed();
        let (mut req, mover, _render) = NativeRequest::decode(request)?;
        self.request_mover = mover;
        let found = self.find_path_high_level_then_low_level(&mut req);
        self.request_mover = PmMover::default();
        if found {
            Ok(Some(std::mem::take(&mut req.path)))
        } else {
            Ok(None)
        }
    }

    /// Like [`World::find_path_bytes`] but writes the native result buffer
    /// (`i16` count, then `f32 x, f32 y, f32 z+32, i32 flags` per node,
    /// big-endian) into `out`, returning the native result code.
    pub fn find_path_into(
        &mut self,
        request: &[u8],
        out: &mut Vec<u8>,
        capacity: usize,
    ) -> Result<i32, PathfindError> {
        out.clear();
        match self.find_path_bytes(request)? {
            None => Ok(0),
            Some(path) => {
                let mut w = ByteWriter::with_limit(capacity);
                path.write_to(&mut w)?;
                *out = w.data;
                Ok(1)
            }
        }
    }

    /// Typed `findPath`.
    pub fn find_path(&mut self, request: &PathFindRequest) -> Option<Vec<PathNode>> {
        self.find_path_bytes(&request.to_bytes())
            .ok()
            .flatten()
            .map(|p| p.nodes)
    }
}

/// Book-keeping of the native `findPath` that its cleanup needs.
#[derive(Default)]
struct FindPathState {
    start_node: Option<u32>,
    goal_node: Option<u32>,
    remove_start: Option<VGraphId>,
    remove_goal: Option<VGraphId>,
    adjust_start_flag: bool,
    adjust_goal_flag: bool,
    adjust_start: AdjustStartEndNodeData,
    adjust_goal: AdjustStartEndNodeData,
}
