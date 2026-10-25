//! `HLSearchNode`: a node of the high-level search (a chunk region, one end
//! of a level transition, a visibility-graph node, or the single node
//! standing for an unloaded goal).

use super::{RegionId, TransId};
use crate::util::dist3;
use crate::visibility::NodeId;
use crate::world::World;

/// `HLSearchNode`.
#[derive(Clone, Debug, Default)]
pub struct HlSearchNode {
    pub chunk_region: Option<RegionId>,
    pub level_transition: Option<TransId>,
    pub bottom_of_staircase: bool,
    pub vg_node: Option<NodeId>,
    pub unloaded_x: i32,
    pub unloaded_y: i32,
    pub in_unloaded_area: bool,
    pub on_edge_of_loaded_area: bool,
    /// `HLSuccessor`s: successor node and the cheapest known step cost.
    pub successors: Vec<(u32, f64)>,
    pub parent: Option<u32>,
    pub g: f64,
    pub depth: i32,
    pub total: f32,
    pub flags: u8,
}

impl World {
    #[inline]
    pub(crate) fn hln(&self, i: u32) -> &HlSearchNode {
        &self.hl.nodes[i as usize]
    }

    /// `HLSearchNode::getX`.
    pub(crate) fn hln_x(&self, i: u32) -> f32 {
        let n = self.hln(i);
        if let Some(r) = n.chunk_region {
            self.region(r).center_x()
        } else if let Some(t) = n.level_transition {
            self.trans(t).search_node_x(n.bottom_of_staircase)
        } else if let Some(v) = n.vg_node {
            self.node(v).x
        } else {
            n.unloaded_x as f32
        }
    }

    /// `HLSearchNode::getY`.
    pub(crate) fn hln_y(&self, i: u32) -> f32 {
        let n = self.hln(i);
        if let Some(r) = n.chunk_region {
            self.region(r).center_y()
        } else if let Some(t) = n.level_transition {
            self.trans(t).search_node_y(n.bottom_of_staircase)
        } else if let Some(v) = n.vg_node {
            self.node(v).y
        } else {
            n.unloaded_y as f32
        }
    }

    /// `HLSearchNode::getZ`.
    pub(crate) fn hln_z(&self, i: u32) -> f32 {
        let n = self.hln(i);
        if let Some(r) = n.chunk_region {
            self.region(r).level.z as f32
        } else if let Some(t) = n.level_transition {
            let tr = self.trans(t);
            (if n.bottom_of_staircase {
                tr.bottom_floor_z()
            } else {
                tr.top_floor_z()
            }) as f32
        } else if let Some(v) = n.vg_node {
            self.node(v).z as f32
        } else {
            32.0
        }
    }

    fn hln_dist(&self, a: u32, b: u32) -> f64 {
        let dx = self.hln_x(a) - self.hln_x(b);
        let dy = self.hln_y(a) - self.hln_y(b);
        let dz = (self.hln_z(a) - self.hln_z(b)) * 2.5;
        dist3(dx, dy, dz) as f64
    }

    /// `HLSearchNode::h`.
    pub(crate) fn hln_h(&self, i: u32) -> f64 {
        self.hln_dist(i, self.hl.goal)
    }

    /// `HLSearchNode::c`.
    pub(crate) fn hln_c(&self, from: u32, to: u32) -> f64 {
        let other = self.hln(to);
        if other.in_unloaded_area {
            return self.hln_dist(to, from);
        }
        let this = self.hln(from);
        if this.vg_node.is_none() || other.vg_node.is_none() {
            return this
                .successors
                .iter()
                .find(|(n, _)| *n == to)
                .map(|&(_, c)| c)
                .unwrap_or(0.0);
        }
        self.hln_dist(to, from)
    }

    /// `HLSearchNode::calculateOnEdgeOfLoadedArea`.
    pub(crate) fn hln_calculate_on_edge_of_loaded_area(&self, i: u32) -> bool {
        let n = self.hln(i);
        if let Some(r) = n.chunk_region {
            self.region_is_on_edge_of_loaded_area(r)
        } else if n.level_transition.is_some() {
            false
        } else if let Some(v) = n.vg_node {
            self.node_is_on_edge_of_loaded_area(v)
        } else {
            false
        }
    }
}
