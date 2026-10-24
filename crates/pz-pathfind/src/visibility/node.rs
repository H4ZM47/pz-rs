//! `Node` and `Connection`: vertices of the visibility graphs and the
//! square nodes created on demand by `PolygonalMap2::getNodeForSquare`.

use super::{ConnId, EdgeId, NodeId, ObstacleId, VGraphId};
use crate::util::{coordmodulo, fastfloor};
use crate::world::{SquareId, World};

/// `Node`.
#[derive(Clone, Debug, Default)]
pub struct Node {
    pub x: f32,
    pub y: f32,
    pub z: i32,
    pub ignore: bool,
    pub square: Option<SquareId>,
    pub graphs: Vec<VGraphId>,
    pub edges: Vec<EdgeId>,
    pub visible: Vec<ConnId>,
    pub flags: i32,
}

impl Node {
    #[inline]
    pub fn has_flag(&self, flag: i32) -> bool {
        self.flags & flag != 0
    }
}

/// `Connection`: an edge of a visibility graph.
#[derive(Clone, Copy, Debug)]
pub struct Connection {
    pub node1: NodeId,
    pub node2: NodeId,
    pub flags: i32,
}

impl Connection {
    /// `otherNode`.
    #[inline]
    pub fn other(&self, n: NodeId) -> NodeId {
        if n != self.node1 {
            self.node1
        } else {
            self.node2
        }
    }

    #[inline]
    pub fn has(&self, flag: i32) -> bool {
        self.flags & flag != 0
    }
}

impl World {
    #[inline]
    pub(crate) fn node(&self, n: NodeId) -> &Node {
        &self.vis.nodes[n]
    }

    #[inline]
    pub(crate) fn node_mut(&mut self, n: NodeId) -> &mut Node {
        &mut self.vis.nodes[n]
    }

    /// `Node::alloc()->init(x, y, z)`.
    pub(crate) fn node_alloc(&mut self, x: f32, y: f32, z: i32) -> NodeId {
        self.vis.nodes.alloc(Node {
            x,
            y,
            z,
            ..Node::default()
        })
    }

    /// `Node::alloc()->init(square)`.
    pub(crate) fn node_alloc_square(&mut self, s: SquareId) -> NodeId {
        let sq = self.sq(s);
        let (x, y, z) = (sq.x as f32 + 0.5, sq.y as f32 + 0.5, sq.z);
        self.vis.nodes.alloc(Node {
            x,
            y,
            z,
            square: Some(s),
            ..Node::default()
        })
    }

    /// `Node::release`: drop every connection, then return to the pool.
    pub(crate) fn node_release(&mut self, n: NodeId) {
        while let Some(&c) = self.vis.nodes[n].visible.last() {
            self.break_connection(c);
        }
        self.vis.nodes.release(n);
    }

    /// `PolygonalMap2::connectTwoNodes`.
    pub(crate) fn connect_two_nodes(&mut self, a: NodeId, b: NodeId, flags: i32) -> ConnId {
        let c = self.vis.connections.alloc(Connection {
            node1: a,
            node2: b,
            flags,
        });
        self.vis.nodes[a].visible.push(c);
        self.vis.nodes[b].visible.push(c);
        c
    }

    /// `PolygonalMap2::breakConnection(Connection*)`.
    pub(crate) fn break_connection(&mut self, c: ConnId) {
        let cxn = self.vis.connections[c];
        for n in [cxn.node1, cxn.node2] {
            let v = &mut self.vis.nodes[n].visible;
            if let Some(i) = v.iter().position(|&x| x == c) {
                v.remove(i);
            }
        }
        self.vis.connections.release(c);
    }

    /// `PolygonalMap2::breakConnection(Node*, Node*)`.
    pub(crate) fn break_connection_between(&mut self, a: NodeId, b: NodeId) {
        let found = self.vis.nodes[a]
            .visible
            .iter()
            .copied()
            .find(|&c| self.vis.connections[c].other(a) == b);
        if let Some(c) = found {
            self.break_connection(c);
        }
    }

    /// `Node::isConnectedTo`.
    pub(crate) fn node_is_connected_to(&self, a: NodeId, b: NodeId) -> bool {
        let n = &self.vis.nodes[a];
        if n.has_flag(super::NODE_FLAG_IN_CHUNK_DATA) {
            return true;
        }
        n.visible.iter().any(|&c| {
            let cx = &self.vis.connections[c];
            cx.node1 == b || cx.node2 == b
        })
    }

    /// `Node::isOnEdgeOfLoadedArea`.
    pub(crate) fn node_is_on_edge_of_loaded_area(&self, n: NodeId) -> bool {
        let node = &self.vis.nodes[n];
        let x = fastfloor(node.x);
        let y = fastfloor(node.y);
        (coordmodulo(x, 8) == 0 && self.get_chunk_from_square_pos(x - 1, y).is_none())
            || (coordmodulo(x, 8) == 7 && self.get_chunk_from_square_pos(x + 1, y).is_none())
            || (coordmodulo(y, 8) == 0 && self.get_chunk_from_square_pos(x, y - 1).is_none())
            || (coordmodulo(y, 8) == 7 && self.get_chunk_from_square_pos(x, y + 1).is_none())
    }

    /// `Node::sharesEdge`.
    pub(crate) fn node_shares_edge(&self, a: NodeId, b: NodeId) -> bool {
        self.vis.nodes[a]
            .edges
            .iter()
            .any(|&e| self.edge_has_node(e, b))
    }

    /// `Node::sharesShape`.
    pub(crate) fn node_shares_shape(&self, a: NodeId, b: NodeId) -> bool {
        let other = &self.vis.nodes[b].edges;
        for &e in &self.vis.nodes[a].edges {
            let o = self.vis.edges[e].obstacle;
            if o.is_none() {
                continue;
            }
            if other.iter().any(|&e2| self.vis.edges[e2].obstacle == o) {
                return true;
            }
        }
        false
    }

    /// `Node::getObstacles`.
    pub(crate) fn node_obstacles(&self, n: NodeId, out: &mut Vec<ObstacleId>) {
        for &e in &self.vis.nodes[n].edges {
            if let Some(o) = self.vis.edges[e].obstacle {
                if !out.contains(&o) {
                    out.push(o);
                }
            }
        }
    }

    /// `Node::onSameShapeButDoesNotShareAnEdge`.
    pub(crate) fn node_on_same_shape_but_does_not_share_an_edge(
        &self,
        a: NodeId,
        b: NodeId,
    ) -> bool {
        let mut obstacles = Vec::new();
        self.node_obstacles(a, &mut obstacles);
        obstacles
            .iter()
            .any(|&o| self.obstacle_has_node(o, b) && !self.obstacle_has_adjacent_nodes(o, a, b))
    }

    /// `Node::createGraphsIfNeeded`.
    pub(crate) fn node_create_graphs_if_needed(&mut self, n: NodeId) {
        let graphs = self.vis.nodes[n].graphs.clone();
        for g in graphs {
            if !self.vis.graphs_pool[g].created {
                self.graph_create(g);
            }
        }
    }
}
