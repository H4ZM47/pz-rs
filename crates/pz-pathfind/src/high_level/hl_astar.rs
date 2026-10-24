//! `HLAStar`: the high-level search over chunk regions, level transitions
//! and visibility-graph nodes.

use super::{AdjustVisibilityGraphData, HlSearchNode, RegionId, TransId};
use crate::astar::{self, AStarHost};
use crate::request::NativeRequest;
use crate::square::bits::*;
use crate::util::{coordmodulo, dist2, fastfloor, Dir};
use crate::visibility::{
    NodeId, VGraphId, CXN_FLAG_THUMP, NODE_FLAG_CRAWL_INTERIOR, NODE_FLAG_PERIMETER,
    NODE_FLAG_STAIR,
};
use crate::world::{ChunkId, LevelRef, MoverSel, SquareId, World};

/// Result of the high-level search: chunk levels the low-level search may
/// use, level transitions in path order, and for each transition whether
/// the path continues from its bottom.
pub(crate) type HlResult = (Vec<LevelRef>, Vec<TransId>, Vec<bool>);

fn push_unique<T: PartialEq>(v: &mut Vec<T>, x: T) {
    if !v.contains(&x) {
        v.push(x);
    }
}

impl World {
    fn hl_new_node(&mut self, n: HlSearchNode) -> u32 {
        let i = self.hl.nodes.len() as u32;
        self.hl.nodes.push(n);
        let on_edge = self.hl.goal_in_unloaded_area && self.hln_calculate_on_edge_of_loaded_area(i);
        self.hl.nodes[i as usize].on_edge_of_loaded_area = on_edge;
        i
    }

    /// `HLAStar::getSearchNode(HLChunkRegion*)`.
    fn hl_node_for_region(&mut self, r: RegionId) -> u32 {
        if let Some(&i) = self.hl.node_map_region.get(&r) {
            return i;
        }
        let i = self.hl_new_node(HlSearchNode {
            chunk_region: Some(r),
            unloaded_x: -1,
            unloaded_y: -1,
            ..HlSearchNode::default()
        });
        self.hl.node_map_region.insert(r, i);
        i
    }

    /// `HLAStar::getSearchNode(HLLevelTransition*, bool)`.
    fn hl_node_for_transition(&mut self, t: TransId, bottom: bool) -> u32 {
        if let Some(&i) = self.hl.node_map_transition.get(&t) {
            return i;
        }
        let i = self.hl_new_node(HlSearchNode {
            level_transition: Some(t),
            bottom_of_staircase: bottom,
            unloaded_x: -1,
            unloaded_y: -1,
            ..HlSearchNode::default()
        });
        self.hl.node_map_transition.insert(t, i);
        i
    }

    /// `HLAStar::getSearchNode(Node*)`.
    fn hl_node_for_vg(&mut self, n: NodeId) -> u32 {
        if let Some(&i) = self.hl.node_map_vg.get(&n) {
            return i;
        }
        let i = self.hl_new_node(HlSearchNode {
            vg_node: Some(n),
            unloaded_x: -1,
            unloaded_y: -1,
            ..HlSearchNode::default()
        });
        self.hl.node_map_vg.insert(n, i);
        i
    }

    /// `HLAStar::getSearchNode(int, int)`.
    fn hl_node_unloaded(&mut self, x: i32, y: i32) -> u32 {
        if let Some(i) = self.hl.unloaded_node {
            return i;
        }
        let i = self.hl.nodes.len() as u32;
        self.hl.nodes.push(HlSearchNode {
            unloaded_x: x,
            unloaded_y: y,
            in_unloaded_area: true,
            ..HlSearchNode::default()
        });
        self.hl.unloaded_node = Some(i);
        i
    }

    /// `HLAStar::releaseSearchNodes`.
    fn hl_release_search_nodes(&mut self) {
        self.hl.nodes.clear();
        self.hl.node_map_region.clear();
        self.hl.node_map_transition.clear();
        self.hl.node_map_vg.clear();
        self.hl.unloaded_node = None;
    }

    /// `HLAStar::getRegionAt`.
    fn hl_region_at(&mut self, x: f32, y: f32, z: i32) -> Option<RegionId> {
        let c = self.get_chunk_from_chunk_pos(fastfloor(x * 0.125), fastfloor(y * 0.125))?;
        if !self.chunk(c).is_valid_level(z) {
            return None;
        }
        let r = LevelRef { chunk: c, z };
        self.get_high_level_data(r);
        self.hl_find_region_containing_square(r, fastfloor(x), fastfloor(y))
    }

    /// `HLAStar::getLevelTransitionAt`.
    pub(crate) fn hl_get_level_transition_at(&mut self, x: i32, y: i32, z: i32) -> Option<TransId> {
        let c = self.get_chunk_from_square_pos(x, y)?;
        if !self.chunk(c).is_valid_level(z) {
            return None;
        }
        let r = LevelRef { chunk: c, z };
        self.get_high_level_data(r);
        self.hl_init_stairs_if_needed(r);
        self.hl_level_transition_at_level(r, x, y)
    }

    /// The level data of a chunk level after `getHighLevelData`.
    fn hl_level_ref(&mut self, c: ChunkId, z: i32) -> LevelRef {
        let r = LevelRef { chunk: c, z };
        self.get_high_level_data(r);
        r
    }

    /// `HLAStar::getSquarePolygonalMapNode(Square*)`: the square's node if
    /// it has any connection. Creates the node as a side effect, as the
    /// native code does.
    fn hl_square_node(&mut self, s: Option<SquareId>) -> Option<NodeId> {
        let s = s?;
        let n = self.get_node_for_square(s);
        if self.node(n).visible.is_empty() {
            None
        } else {
            Some(n)
        }
    }

    /// `HLAStar::shouldIgnoreNode`.
    fn hl_should_ignore_node(&mut self, n: NodeId) -> bool {
        if !self.node(n).has_flag(NODE_FLAG_STAIR) {
            let s = match self.node(n).square {
                Some(s) => s,
                None => return false,
            };
            let (x, y, z) = (self.sq(s).x, self.sq(s).y, self.sq(s).z);
            return self
                .hl_region_at(x as f32 + 0.5, y as f32 + 0.5, z)
                .is_some();
        }
        for &c in &self.node(n).visible {
            let cx = self.vis.connections[c];
            if !self.node(cx.node1).has_flag(NODE_FLAG_STAIR)
                || !self.node(cx.node2).has_flag(NODE_FLAG_STAIR)
            {
                return false;
            }
        }
        true
    }

    /// `HLAStar::initVisibilityGraphNode`.
    fn hl_init_visibility_graph_node(
        &mut self,
        vg: VGraphId,
        x: f32,
        y: f32,
        z: f32,
        start: bool,
    ) -> Option<NodeId> {
        let mut data = if start {
            std::mem::take(&mut self.hl.adjust_start)
        } else {
            std::mem::take(&mut self.hl.adjust_goal)
        };
        data.graph = None;
        data.adjusted = false;
        data.vg_node = None;
        if !self.vis.graphs_pool[vg].created {
            self.graph_create(vg);
        }
        let adjusted = self.graph_get_point_outside_obstacles(vg, x, y, z, &mut data.adjust);
        let result = if adjusted == -1 {
            None
        } else {
            if adjusted == 1 {
                data.adjusted = true;
                data.vg_node = data.adjust.node;
                if data.adjust.is_node_new {
                    data.graph = Some(vg);
                }
            }
            if data.vg_node.is_none() {
                let n = self.node_alloc(x, y, fastfloor(z));
                data.vg_node = Some(n);
                self.graph_add_node(vg, n);
                data.graph = Some(vg);
            }
            data.vg_node
        };
        if start {
            self.hl.adjust_start = data;
        } else {
            self.hl.adjust_goal = data;
        }
        result
    }

    /// `HLAStar::cleanUpVisibilityGraphNode`.
    fn hl_clean_up_visibility_graph_node(&mut self, data: &AdjustVisibilityGraphData) {
        if let Some(g) = data.graph {
            let n = data.vg_node.unwrap();
            self.graph_remove_node(g, n);
            self.node_release(n);
        }
        if data.adjusted && data.adjust.is_node_new {
            self.undo_adjust_start_end(&data.adjust);
        }
    }

    /// Shared body of `initStartNode` / `initGoalNode`.
    fn hl_init_end_node(&mut self, x: f32, y: f32, z: i32, start: bool) -> Option<u32> {
        let s = self.get_square(fastfloor(x), fastfloor(y), z);
        if let Some(n) = self.hl_square_node(s) {
            if !self.hl_should_ignore_node(n) {
                return Some(self.hl_node_for_vg(n));
            }
        }
        if let Some(r) = self.hl_region_at(x, y, z) {
            return Some(self.hl_node_for_region(r));
        }
        let mut vg_node = None;
        match self.get_vis_graph_at(x, y, z, 0) {
            None => {
                if let Some(g) = self.get_vis_graph_at(x, y, z, 1) {
                    if !self.vis.graphs_pool[g].created {
                        self.graph_create(g);
                    }
                    let s = self.get_square(fastfloor(x), fastfloor(y), z);
                    if let Some(s) = s {
                        let n = self.get_point_outside_objects(s, x, y);
                        self.graph_add_node(g, n);
                        let data = if start {
                            &mut self.hl.adjust_start
                        } else {
                            &mut self.hl.adjust_goal
                        };
                        let (nx, ny) = (self.vis.nodes[n].x, self.vis.nodes[n].y);
                        if nx != x || ny != y {
                            data.adjusted = true;
                            data.adjust.is_node_new = false;
                        }
                        data.graph = Some(g);
                        data.vg_node = Some(n);
                        vg_node = Some(n);
                    }
                }
            }
            Some(g) => {
                vg_node = self.hl_init_visibility_graph_node(g, x, y, z as f32, start);
            }
        }
        if let Some(n) = vg_node {
            return Some(self.hl_node_for_vg(n));
        }
        let t = self.hl_get_level_transition_at(fastfloor(x), fastfloor(y), z)?;
        Some(self.hl_node_for_transition(t, true))
    }

    /// `HLAStar::initGoalNode`.
    fn hl_init_goal_node(&mut self, x: f32, y: f32, z: i32) -> Option<u32> {
        if self
            .get_chunk_from_square_pos(fastfloor(x), fastfloor(y))
            .is_none()
        {
            return Some(self.hl_node_unloaded(fastfloor(x), fastfloor(y)));
        }
        self.hl_init_end_node(x, y, z, false)
    }

    /// `HLAStar::addChunkLevelAndAdjacentToList(HLChunkLevel*)`.
    fn hl_add_level_and_adjacent(&mut self, r: LevelRef, list: &mut Vec<LevelRef>) {
        push_unique(list, r);
        let (wx, wy) = (self.chunk(r.chunk).wx, self.chunk(r.chunk).wy);
        for d in Dir::ALL {
            if let Some(c2) = self.get_chunk_from_chunk_pos(wx + d.dx(), wy + d.dy()) {
                if self.chunk(c2).is_valid_level(r.z) {
                    let r2 = self.hl_level_ref(c2, r.z);
                    push_unique(list, r2);
                }
            }
        }
    }

    /// `HLAStar::addChunkLevelAndAdjacentToList(HLChunkRegion*)`: the
    /// region's level, plus each adjacent level the region actually leads
    /// to.
    fn hl_add_region_level_and_adjacent(&mut self, region: RegionId, list: &mut Vec<LevelRef>) {
        let r = self.region(region).level;
        push_unique(list, r);
        let node = self.hl.node_map_region.get(&region).copied();
        let (wx, wy) = (self.chunk(r.chunk).wx, self.chunk(r.chunk).wy);
        for d in Dir::ALL {
            let c2 = match self.get_chunk_from_chunk_pos(wx + d.dx(), wy + d.dy()) {
                Some(c) if self.chunk(c).is_valid_level(r.z) => c,
                _ => continue,
            };
            let r2 = self.hl_level_ref(c2, r.z);
            let add = match node {
                None => true,
                Some(n) => {
                    if self.hln(n).successors.is_empty() {
                        if self.hl.goal == n {
                            let mut succ = Vec::new();
                            self.hl_get_successors(n, &mut succ);
                            succ.iter().any(|&s| {
                                self.hln(s)
                                    .chunk_region
                                    .is_some_and(|cr| self.region(cr).level == r2)
                            })
                        } else {
                            false
                        }
                    } else {
                        self.hln(n).successors.iter().any(|&(s, _)| {
                            self.hln(s)
                                .chunk_region
                                .is_some_and(|cr| self.region(cr).level == r2)
                        })
                    }
                }
            };
            if add {
                push_unique(list, r2);
            }
        }
    }

    /// `HLAStar::addLevelTransitionChunkLevelData`.
    fn hl_add_transition_levels(&mut self, t: TransId, list: &mut Vec<LevelRef>) {
        let tr = *self.trans(t);
        for (x, y, z) in [
            (
                tr.bottom_floor_x(),
                tr.bottom_floor_y(),
                tr.bottom_floor_z(),
            ),
            (tr.top_floor_x(), tr.top_floor_y(), tr.top_floor_z()),
        ] {
            if let Some(c) = self.get_chunk_from_square_pos(x, y) {
                let r = self.hl_level_ref(c, z);
                self.hl_add_level_and_adjacent(r, list);
            }
        }
    }

    /// `HLAStar::findPath`.
    pub(crate) fn hl_find_path(&mut self, req: &NativeRequest) -> HlResult {
        let mut chunk_levels = Vec::new();
        let mut transitions = Vec::new();
        let mut bottoms = Vec::new();
        self.hl_release_search_nodes();
        self.hl.adjust_start = AdjustVisibilityGraphData::default();
        self.hl.adjust_goal = AdjustVisibilityGraphData::default();
        self.hl.goal_in_unloaded_area = self
            .get_chunk_from_square_pos(fastfloor(req.target_x), fastfloor(req.target_y))
            .is_none();
        self.hl.unloaded_node = None;

        self.hl_find_path_inner(req, &mut chunk_levels, &mut transitions, &mut bottoms);

        let start = std::mem::take(&mut self.hl.adjust_start);
        self.hl_clean_up_visibility_graph_node(&start);
        let goal = std::mem::take(&mut self.hl.adjust_goal);
        self.hl_clean_up_visibility_graph_node(&goal);
        self.hl_release_search_nodes();
        (chunk_levels, transitions, bottoms)
    }

    fn hl_find_path_inner(
        &mut self,
        req: &NativeRequest,
        chunk_levels: &mut Vec<LevelRef>,
        transitions: &mut Vec<TransId>,
        bottoms: &mut Vec<bool>,
    ) {
        let start =
            match self.hl_init_end_node(req.start_x, req.start_y, fastfloor(req.start_z), true) {
                Some(n) => n,
                None => return,
            };
        let goal = match self.hl_init_goal_node(req.target_x, req.target_y, fastfloor(req.target_z))
        {
            Some(n) => n,
            None => return,
        };
        let (sr, gr) = (self.hln(start).chunk_region, self.hln(goal).chunk_region);
        if sr.is_some() && sr == gr {
            let r = self.region(sr.unwrap()).level;
            self.hl_add_level_and_adjacent(r, chunk_levels);
            return;
        }
        let (st, gt) = (
            self.hln(start).level_transition,
            self.hln(goal).level_transition,
        );
        if st.is_some() && st == gt {
            transitions.push(st.unwrap());
            bottoms.push(true);
            self.hl_add_transition_levels(st.unwrap(), chunk_levels);
            return;
        }
        self.hl.goal = goal;
        let path = {
            let mut host = HlHost { w: self };
            astar::shortest_path(&mut host, start)
        };
        let path = match path {
            Some(p) => p,
            None => return,
        };
        for (pi, &n) in path.iter().enumerate() {
            if let Some(r) = self.hln(n).chunk_region {
                self.hl_add_region_level_and_adjacent(r, chunk_levels);
            }
            if let Some(t) = self.hln(n).level_transition {
                transitions.push(t);
                self.hl_add_transition_levels(t, chunk_levels);
                let bottom = if pi == path.len() - 1 {
                    true
                } else {
                    fastfloor(self.hln_z(path[pi + 1])) <= self.trans(t).bottom_floor_z()
                };
                bottoms.push(bottom);
            }
            if let Some(v) = self.hln(n).vg_node {
                let (x, y, z) = (self.node(v).x, self.node(v).y, self.node(v).z);
                if let Some(c) = self.get_chunk_from_square_pos(fastfloor(x), fastfloor(y)) {
                    let r = self.hl_level_ref(c, z);
                    self.hl_add_level_and_adjacent(r, chunk_levels);
                }
            }
        }
    }

    // ---- successors -------------------------------------------------

    /// `HLAStar::getSuccessors`.
    pub(crate) fn hl_get_successors(&mut self, n: u32, out: &mut Vec<u32>) {
        self.hl.nodes[n as usize].successors.clear();
        let node = self.hln(n).clone();
        if node.in_unloaded_area {
            return;
        }
        if node.on_edge_of_loaded_area {
            out.push(self.hl.unloaded_node.expect("unloaded goal node"));
        }
        if node.level_transition.is_some() {
            self.hl_successor_regions_from_transition(n, out);
            self.hl_successor_graphs_from_transition(n, out);
            return;
        }
        if node.vg_node.is_some() {
            self.hl_successors_from_visibility_graph(n, out);
            return;
        }
        self.hl_successors_on_same_chunk(n, out);
        self.hl_successors_on_adjacent_chunks(n, out);
        self.hl_successors_going_up(n, out);
        self.hl_successors_going_down(n, out);
        self.hl_successor_visibility_graphs(n, out);
    }

    /// `HLAStar::setLowestCostSuccessor`.
    fn hl_set_lowest_cost(&mut self, node: u32, succ: u32, cost: f64) {
        let list = &mut self.hl.nodes[node as usize].successors;
        if let Some(e) = list.iter_mut().find(|e| e.0 == succ) {
            if e.1 <= cost {
                return;
            }
            e.1 = cost;
        } else {
            list.push((succ, cost));
        }
    }

    /// `HLAStar::addSuccessor(cost, node, HLChunkRegion*)`.
    fn hl_add_successor_region(&mut self, cost: f64, node: u32, r: RegionId, out: &mut Vec<u32>) {
        let s = self.hl_node_for_region(r);
        push_unique(out, s);
        self.hl_set_lowest_cost(node, s, cost);
    }

    /// `HLAStar::addSuccessor(cost, node, HLLevelTransition*, bottom)`.
    fn hl_add_successor_transition(
        &mut self,
        cost: f64,
        node: u32,
        t: TransId,
        bottom: bool,
        out: &mut Vec<u32>,
    ) {
        let s = self.hl_node_for_transition(t, bottom);
        push_unique(out, s);
        self.hl_set_lowest_cost(node, s, cost);
    }

    /// `HLAStar::addSuccessor(cost, node, Node*)`.
    fn hl_add_successor_vg(&mut self, cost: f64, node: u32, v: NodeId, out: &mut Vec<u32>) {
        let s = self.hl_node_for_vg(v);
        push_unique(out, s);
        if self.hln(node).vg_node.is_some() {
            return;
        }
        self.hl_set_lowest_cost(node, s, cost);
    }

    /// `HLAStar::calculateCost(node, square1, node2X, node2Y, square2)`.
    fn hl_calculate_cost(
        &self,
        node: u32,
        s1: SquareId,
        n2x: f32,
        n2y: f32,
        s2: Option<SquareId>,
    ) -> f64 {
        let sq1 = self.sq(s1);
        let x1 = sq1.x as f32 + 0.5;
        let y1 = sq1.y as f32 + 0.5;
        let z1 = sq1.z as f32;
        let (x2, y2, z2) = (x1, y1, z1);
        let d1 = dist2(x1 - self.hln_x(node), y1 - self.hln_y(node)) as f64;
        let d2 = crate::util::dist3(x1 - x2, y1 - y2, (z1 - z2) * 2.5) as f64;
        let d3 = dist2(n2x - x2, n2y - y2) as f64;
        let mut cost = d1 + d2 + d3;
        let m = self.mover();
        if m.is_zombie() && !m.crawling {
            return cost;
        }
        let not_crawling_zombie = !(m.is_zombie() && m.crawling);
        let s2 = match s2 {
            Some(s) => s,
            None => return cost,
        };
        let sq2 = self.sq(s2);
        let adjacent =
            |d: Dir| sq1.x + d.dx() == sq2.x && sq1.y + d.dy() == sq2.y && sq1.z == sq2.z;
        let (window_sq, n_or_w) = if adjacent(Dir::N) {
            (sq1, true)
        } else if adjacent(Dir::S) {
            (sq2, true)
        } else if adjacent(Dir::W) {
            (sq1, false)
        } else if adjacent(Dir::E) {
            (sq2, false)
        } else {
            return cost;
        };
        let unblocked = if n_or_w {
            window_sq.is_unblocked_window_n(self)
        } else {
            window_sq.is_unblocked_window_w(self)
        };
        if not_crawling_zombie && unblocked {
            cost += 20.0;
        } else if window_sq.has(if n_or_w { WINDOW_N } else { WINDOW_W }) {
            cost += 200.0;
        }
        cost
    }

    fn hl_calculate_cost_region(
        &self,
        node: u32,
        s1: SquareId,
        r: RegionId,
        s2: Option<SquareId>,
    ) -> f64 {
        let reg = self.region(r);
        self.hl_calculate_cost(node, s1, reg.center_x(), reg.center_y(), s2)
    }

    /// `HLAStar::calculateCost(node, square)`.
    fn hl_calculate_cost_to_square(&self, node: u32, s: SquareId) -> f64 {
        let sq = self.sq(s);
        dist2(
            sq.x as f32 + 0.5 - self.hln_x(node),
            sq.y as f32 + 0.5 - self.hln_y(node),
        ) as f64
    }

    fn hl_can_move(&self, x1: i32, y1: i32, z1: i32, x2: i32, y2: i32, z2: i32) -> bool {
        self.can_move_between_with(MoverSel::Request, x1, y1, z1, x2, y2, z2)
    }

    /// `HLAStar::getSuccessorChunkRegionsFromLevelTransition`.
    fn hl_successor_regions_from_transition(&mut self, n: u32, out: &mut Vec<u32>) {
        let t = self.hln(n).level_transition.unwrap();
        let top = self.trans_top_floor_square(t);
        let bottom = self.trans_bottom_floor_square(t);
        let (top, bottom) = match (top, bottom) {
            (Some(a), Some(b)) => (a, b),
            _ => return,
        };
        let tr = *self.trans(t);
        if let Some(c) = self.trans_bottom_floor_chunk(t) {
            let r = self.hl_level_ref(c, tr.bottom_floor_z());
            if let Some(reg) =
                self.hl_find_region_containing_square(r, tr.bottom_floor_x(), tr.bottom_floor_y())
            {
                let cost = self.hl_calculate_cost_region(n, top, reg, Some(bottom));
                self.hl_add_successor_region(cost, n, reg, out);
            }
        }
        if let Some(c) = self.trans_top_floor_chunk(t) {
            let r = self.hl_level_ref(c, tr.top_floor_z());
            if let Some(reg) =
                self.hl_find_region_containing_square(r, tr.top_floor_x(), tr.top_floor_y())
            {
                let cost = self.hl_calculate_cost_region(n, bottom, reg, Some(top));
                self.hl_add_successor_region(cost, n, reg, out);
            }
        }
    }

    /// `HLAStar::getSuccessorVisibilityGraphsFromLevelTransition`.
    fn hl_successor_graphs_from_transition(&mut self, n: u32, out: &mut Vec<u32>) {
        let t = self.hln(n).level_transition.unwrap();
        let top = self.trans_top_floor_square(t);
        let bottom = self.trans_bottom_floor_square(t);
        for s in [top, bottom].into_iter().flatten() {
            let (x, y, z) = (self.sq(s).x, self.sq(s).y, self.sq(s).z);
            if let Some(g) = self.get_vis_graph_at(x as f32 + 0.5, y as f32 + 0.5, z, 1) {
                if !self.vis.graphs_pool[g].created {
                    self.graph_create(g);
                }
                let v = self.get_node_for_square(s);
                self.hl_add_successor_vg(1.0, n, v, out);
            }
        }
    }

    /// `HLAStar::getSuccessorsFromVisibilityGraph`.
    fn hl_successors_from_visibility_graph(&mut self, n: u32, out: &mut Vec<u32>) {
        let v = self.hln(n).vg_node.unwrap();
        self.node_create_graphs_if_needed(v);
        let visible = self.node(v).visible.clone();
        for c in visible {
            let cxn = self.vis.connections[c];
            let other = cxn.other(v);
            if self.hl_should_ignore_node(other) {
                continue;
            }
            let m = self.mover();
            if !m.can_crawl && self.node(other).has_flag(NODE_FLAG_CRAWL_INTERIOR) {
                continue;
            }
            if !m.can_thump && cxn.has(CXN_FLAG_THUMP) {
                continue;
            }
            let (a, b) = (self.node(v), self.node(other));
            let cost = dist2(b.x - a.x, b.y - a.y) as f64;
            self.hl_add_successor_vg(cost, n, other, out);
        }
        if let Some(s) = self.node(v).square {
            if self.node(v).has_flag(NODE_FLAG_PERIMETER) || self.node(v).has_flag(NODE_FLAG_STAIR)
            {
                self.hl_add_successor_regions_adjacent_to_square(n, s, out);
            }
        }
    }

    /// `HLAStar::addSuccessorChunkRegionsAdjacentToSquare`.
    fn hl_add_successor_regions_adjacent_to_square(
        &mut self,
        n: u32,
        s1: SquareId,
        out: &mut Vec<u32>,
    ) {
        let (x, y, z) = (self.sq(s1).x, self.sq(s1).y, self.sq(s1).z);
        for d in Dir::ALL {
            let (x2, y2) = (x + d.dx(), y + d.dy());
            let c2 = match self.get_chunk_from_square_pos(x2, y2) {
                Some(c) if self.chunk(c).is_valid_level(z) => c,
                _ => continue,
            };
            let s2 = match self.squares_for_level(c2, z).unwrap()[coordmodulo(x2, 8) as usize]
                [coordmodulo(y2, 8) as usize]
            {
                Some(s) => s,
                None => continue,
            };
            let (sx2, sy2, sz2) = (self.sq(s2).x, self.sq(s2).y, self.sq(s2).z);
            if !self.hl_can_move(x, y, z, sx2, sy2, sz2) {
                continue;
            }
            let r = self.hl_level_ref(c2, z);
            let regions = self.chunk_level(r).hl.regions.clone();
            for reg in regions {
                if self.region_contains_square(reg, x2, y2) {
                    let cost = self.hl_calculate_cost_region(n, s1, reg, Some(s2));
                    self.hl_add_successor_region(cost, n, reg, out);
                }
            }
        }
    }

    /// `HLAStar::addSuccessorsOnEdge`.
    #[allow(clippy::too_many_arguments)]
    fn hl_add_successors_on_edge(
        &mut self,
        n: u32,
        r2: RegionId,
        xy: i32,
        bits: i32,
        chunk: ChunkId,
        dir: Dir,
        level: i32,
        out: &mut Vec<u32>,
    ) {
        let squares = *self.squares_for_level(chunk, level).unwrap();
        for i in 0..8 {
            if bits & (1 << i) == 0 {
                continue;
            }
            let s1 = if dir.dx() == 0 {
                squares[i][xy as usize]
            } else {
                squares[xy as usize][i]
            };
            let s1 = s1.expect("edge square exists");
            let (x, y, z) = (self.sq(s1).x, self.sq(s1).y, self.sq(s1).z);
            if !self.hl_can_move(x, y, z, x + dir.dx(), y + dir.dy(), z) {
                continue;
            }
            let s2 = self.adjacent_square(s1, dir);
            let cost = self.hl_calculate_cost_region(n, s1, r2, s2);
            self.hl_add_successor_region(cost, n, r2, out);
        }
    }

    /// `HLAStar::getSuccessorsOnSameChunk`.
    fn hl_successors_on_same_chunk(&mut self, n: u32, out: &mut Vec<u32>) {
        let r1 = self.hln(n).chunk_region.unwrap();
        let level_ref = self.region(r1).level;
        let chunk = level_ref.chunk;
        let level = level_ref.z;
        let regions = self.chunk_level(level_ref).hl.regions.clone();
        for y in 0..8usize {
            if y > 0 && self.region(r1).edge_n[y] != 0 {
                for &r2 in &regions {
                    let bits = self.region(r1).edge_n[y] & self.region(r2).edge_s[y - 1];
                    if r1 != r2 && bits != 0 {
                        self.hl_add_successors_on_edge(
                            n,
                            r2,
                            y as i32,
                            bits,
                            chunk,
                            Dir::N,
                            level,
                            out,
                        );
                    }
                }
            }
            if y < 7 && self.region(r1).edge_s[y] != 0 {
                for &r2 in &regions {
                    let bits = self.region(r1).edge_s[y] & self.region(r2).edge_n[y + 1];
                    if r1 != r2 && bits != 0 {
                        self.hl_add_successors_on_edge(
                            n,
                            r2,
                            y as i32,
                            bits,
                            chunk,
                            Dir::S,
                            level,
                            out,
                        );
                    }
                }
            }
        }
        for x in 0..8usize {
            if x > 0 && self.region(r1).edge_w[x] != 0 {
                for &r2 in &regions {
                    let bits = self.region(r1).edge_w[x] & self.region(r2).edge_e[x - 1];
                    if r1 != r2 && bits != 0 {
                        self.hl_add_successors_on_edge(
                            n,
                            r2,
                            x as i32,
                            bits,
                            chunk,
                            Dir::W,
                            level,
                            out,
                        );
                    }
                }
            }
            if x < 7 && self.region(r1).edge_e[x] != 0 {
                for &r2 in &regions {
                    let bits = self.region(r1).edge_e[x] & self.region(r2).edge_w[x + 1];
                    if r1 != r2 && bits != 0 {
                        self.hl_add_successors_on_edge(
                            n,
                            r2,
                            x as i32,
                            bits,
                            chunk,
                            Dir::E,
                            level,
                            out,
                        );
                    }
                }
            }
        }
    }

    /// `HLAStar::getSuccessorsOnAdjacentChunks`.
    fn hl_successors_on_adjacent_chunks(&mut self, n: u32, out: &mut Vec<u32>) {
        let r1 = self.hln(n).chunk_region.unwrap();
        let level_ref = self.region(r1).level;
        let chunk = level_ref.chunk;
        let level = level_ref.z;
        let (wx, wy) = (self.chunk(chunk).wx, self.chunk(chunk).wy);
        let squares1 = *self.squares_for_level(chunk, level).unwrap();
        // (edge of r1, neighbour chunk offset, edge of the neighbour region,
        // xy, direction, repetitions as in the native code)
        type EdgeSel = fn(&super::HlChunkRegion) -> i32;
        let sides: [(EdgeSel, i32, i32, EdgeSel, i32, Dir, usize); 4] = [
            (|r| r.edge_n[0], 0, -1, |r| r.edge_s[7], 0, Dir::N, 1),
            (|r| r.edge_s[7], 0, 1, |r| r.edge_n[0], 7, Dir::S, 1),
            (|r| r.edge_w[0], -1, 0, |r| r.edge_e[7], 0, Dir::W, 8),
            (|r| r.edge_e[7], 1, 0, |r| r.edge_w[0], 7, Dir::E, 8),
        ];
        for (mine, dx, dy, theirs, xy, dir, reps) in sides {
            if mine(self.region(r1)) == 0 {
                continue;
            }
            let c2 = match self.get_chunk_from_chunk_pos(wx + dx, wy + dy) {
                Some(c) if self.chunk(c).is_valid_level(level) => c,
                _ => continue,
            };
            let r2l = self.hl_level_ref(c2, level);
            let regions = self.chunk_level(r2l).hl.regions.clone();
            for r2 in regions {
                let bits = mine(self.region(r1)) & theirs(self.region(r2));
                if bits == 0 {
                    continue;
                }
                for _ in 0..reps {
                    let bits = mine(self.region(r1)) & theirs(self.region(r2));
                    self.hl_add_successors_on_edge(n, r2, xy, bits, chunk, dir, level, out);
                }
            }
        }
        let c = self.chunk(chunk);
        let corners = [
            (
                c.min_x(),
                c.min_y(),
                -1,
                -1,
                (0usize, 0usize),
                (7usize, 7usize),
            ),
            (c.max_x(), c.min_y(), 1, -1, (7, 0), (0, 7)),
            (c.max_x(), c.max_y(), 1, 1, (7, 7), (0, 0)),
            (c.min_x(), c.max_y(), -1, 1, (0, 7), (7, 0)),
        ];
        for (x1, y1, dx, dy, own, other) in corners {
            if !self.region_contains_square(r1, x1, y1) {
                continue;
            }
            let c2 = match self.get_chunk_from_chunk_pos(wx + dx, wy + dy) {
                Some(c) if self.chunk(c).is_valid_level(level) => c,
                _ => continue,
            };
            let r2l = self.hl_level_ref(c2, level);
            let r2 = match self.hl_find_region_containing_square(r2l, x1 + dx, y1 + dy) {
                Some(r) => r,
                None => continue,
            };
            if !self.hl_can_move(x1, y1, level, x1 + dx, y1 + dy, level) {
                continue;
            }
            let s1 = squares1[own.0][own.1].expect("corner square exists");
            let s2 = self.squares_for_level(c2, level).unwrap()[other.0][other.1];
            let cost = self.hl_calculate_cost_region(n, s1, r2, s2);
            self.hl_add_successor_region(cost, n, r2, out);
        }
    }

    /// Successor for the square at one end of a level transition: the
    /// square's node if it has connections, otherwise the transition.
    fn hl_add_transition_end(
        &mut self,
        n: u32,
        t: TransId,
        s: Option<SquareId>,
        bottom: bool,
        out: &mut Vec<u32>,
    ) {
        let s = s.expect("transition square exists");
        let cost = self.hl_calculate_cost_to_square(n, s);
        match self.hl_square_node(Some(s)) {
            Some(v) => self.hl_add_successor_vg(cost, n, v, out),
            None => self.hl_add_successor_transition(cost, n, t, bottom, out),
        }
    }

    /// `HLAStar::getSuccessorsGoingUp`.
    fn hl_successors_going_up(&mut self, n: u32, out: &mut Vec<u32>) {
        let r1 = self.hln(n).chunk_region.unwrap();
        let level_ref = self.region(r1).level;
        self.hl_init_stairs_if_needed(level_ref);
        let squares = self.chunk_level(level_ref).squares;
        let hl = &self.chunk_level(level_ref).hl;
        let list: Vec<TransId> = hl
            .stairs
            .iter()
            .chain(hl.sloped_surfaces.iter())
            .copied()
            .collect();
        for t in list {
            let tr = *self.trans(t);
            if !self.region_contains_square(r1, tr.bottom_floor_x(), tr.bottom_floor_y()) {
                continue;
            }
            let s = squares[coordmodulo(tr.bottom_floor_x(), 8) as usize]
                [coordmodulo(tr.bottom_floor_y(), 8) as usize];
            self.hl_add_transition_end(n, t, s, true, out);
        }
    }

    /// `HLAStar::getSuccessorsGoingDown`.
    fn hl_successors_going_down(&mut self, n: u32, out: &mut Vec<u32>) {
        let r1 = self.hln(n).chunk_region.unwrap();
        let level_ref = self.region(r1).level;
        let (wx, wy) = (
            self.chunk(level_ref.chunk).wx,
            self.chunk(level_ref.chunk).wy,
        );
        let level = level_ref.z;
        for (dx, dy) in [(0, 0), (0, 1), (1, 0)] {
            self.hl_successors_going_down_in(n, r1, wx + dx, wy + dy, level, false, out);
        }
        for (dx, dy) in [(0, 0), (0, 1), (0, -1), (1, 0), (-1, 0)] {
            self.hl_successors_going_down_in(n, r1, wx + dx, wy + dy, level, true, out);
        }
    }

    /// `getSuccessorsGoingDown_Staircases` / `_SlopedSurfaces`.
    #[allow(clippy::too_many_arguments)]
    fn hl_successors_going_down_in(
        &mut self,
        n: u32,
        r1: RegionId,
        wx: i32,
        wy: i32,
        level: i32,
        slopes: bool,
        out: &mut Vec<u32>,
    ) {
        let c2 = match self.get_chunk_from_chunk_pos(wx, wy) {
            Some(c) if self.chunk(c).is_valid_level(level - 1) => c,
            _ => return,
        };
        let below = self.hl_level_ref(c2, level - 1);
        self.hl_init_stairs_if_needed(below);
        let own = self.region(r1).level;
        let squares = self.chunk_level(own).squares;
        let hl = &self.chunk_level(below).hl;
        let list = if slopes {
            hl.sloped_surfaces.clone()
        } else {
            hl.stairs.clone()
        };
        for t in list {
            let tr = *self.trans(t);
            if !self.region_contains_square(r1, tr.top_floor_x(), tr.top_floor_y()) {
                continue;
            }
            let s = squares[coordmodulo(tr.top_floor_x(), 8) as usize]
                [coordmodulo(tr.top_floor_y(), 8) as usize];
            self.hl_add_transition_end(n, t, s, false, out);
        }
    }

    /// `HLAStar::getSuccessorVisibilityGraphs`.
    fn hl_successor_visibility_graphs(&mut self, n: u32, out: &mut Vec<u32>) {
        let r1 = self.hln(n).chunk_region.unwrap();
        let level_ref = self.region(r1).level;
        let chunk = level_ref.chunk;
        let level = level_ref.z;
        let c = self.chunk(chunk);
        let (min_x, min_y, max_x, max_y) = (c.min_x(), c.min_y(), c.max_x(), c.max_y());
        let squares1 = *self.squares_for_level(chunk, level).unwrap();
        let mut graphs = std::mem::take(&mut self.hl.visibility_graphs);
        graphs.clear();
        self.get_visibility_graphs_overlapping_chunk(chunk, level, &mut graphs);
        self.get_visibility_graphs_adjacent_to_chunk(chunk, level, &mut graphs);
        if !graphs.is_empty() {
            for &g in &graphs {
                let perimeter = self.vis.graphs_pool[g].perimeter_nodes.clone();
                for v in perimeter {
                    let s2 = self.node(v).square.expect("perimeter node has a square");
                    if !self
                        .sq(s2)
                        .is_inside(min_x - 1, min_y - 1, max_x + 2, max_y + 2)
                    {
                        continue;
                    }
                    let (sx, sy) = (self.sq(s2).x, self.sq(s2).y);
                    for d in Dir::ALL {
                        let (x2, y2) = (sx + d.dx(), sy + d.dy());
                        if !self.region_contains_square(r1, x2, y2)
                            || !self.hl_can_move(x2, y2, level, sx, sy, level)
                        {
                            continue;
                        }
                        let s1 = squares1[(x2 - min_x) as usize][(y2 - min_y) as usize]
                            .expect("region square");
                        let cost = self.hl_calculate_cost(
                            n,
                            s1,
                            sx as f32 + 0.5,
                            sy as f32 + 0.5,
                            Some(s2),
                        );
                        self.hl_add_successor_vg(cost, n, v, out);
                    }
                }
            }
            for y in 0..8 {
                for x in 0..8 {
                    let s = match squares1[x][y] {
                        Some(s) if self.region(r1).contains_square_local(x as i32, y as i32) => s,
                        _ => continue,
                    };
                    let bn = self.sq(s).has(STAIR_BN);
                    let bw = self.sq(s).has(STAIR_BW);
                    if !bn && !bw {
                        continue;
                    }
                    let dir = if bn { Dir::N } else { Dir::W };
                    let s2 = match self.adjacent_square(s, dir.rot180()) {
                        Some(s2) => s2,
                        None => continue,
                    };
                    let v = match self.get_existing_node_for_square(s2) {
                        Some(v) if self.node(v).has_flag(NODE_FLAG_STAIR) => v,
                        _ => continue,
                    };
                    let (sx, sy) = (self.sq(s).x, self.sq(s).y);
                    let (s2x, s2y) = (self.sq(s2).x, self.sq(s2).y);
                    if !self.hl_can_move(sx, sy, level, s2x, s2y, level) {
                        continue;
                    }
                    let cost =
                        self.hl_calculate_cost(n, s, s2x as f32 + 0.5, s2y as f32 + 0.5, Some(s2));
                    self.hl_add_successor_vg(cost, n, v, out);
                }
            }
        }
        self.hl.visibility_graphs = graphs;
    }
}

struct HlHost<'a> {
    w: &'a mut World,
}

impl AStarHost for HlHost<'_> {
    fn g(&self, n: u32) -> f64 {
        self.w.hl.nodes[n as usize].g
    }
    fn set_g(&mut self, n: u32, g: f64) {
        self.w.hl.nodes[n as usize].g = g;
    }
    fn h(&mut self, n: u32) -> f64 {
        self.w.hln_h(n)
    }
    fn c(&mut self, from: u32, to: u32) -> f64 {
        self.w.hln_c(from, to)
    }
    fn successors(&mut self, n: u32, out: &mut Vec<u32>) {
        self.w.hl_get_successors(n, out);
    }
    fn parent(&self, n: u32) -> Option<u32> {
        self.w.hl.nodes[n as usize].parent
    }
    fn set_parent(&mut self, n: u32, parent: Option<u32>) {
        self.w.hl.nodes[n as usize].parent = parent;
    }
    fn depth(&self, n: u32) -> i32 {
        self.w.hl.nodes[n as usize].depth
    }
    fn set_depth(&mut self, n: u32, depth: i32) {
        self.w.hl.nodes[n as usize].depth = depth;
    }
    fn total(&self, n: u32) -> f32 {
        self.w.hl.nodes[n as usize].total
    }
    fn set_total(&mut self, n: u32, total: f32) {
        self.w.hl.nodes[n as usize].total = total;
    }
    fn flags(&self, n: u32) -> u8 {
        self.w.hl.nodes[n as usize].flags
    }
    fn set_flags(&mut self, n: u32, flags: u8) {
        self.w.hl.nodes[n as usize].flags = flags;
    }
    fn in_goal(&self, n: u32) -> bool {
        n == self.w.hl.goal
    }
}
