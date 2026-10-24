//! Vehicle obstacles and the visibility graphs built around them
//! (`Node`, `Connection`, `Edge`, `EdgeRing`, `Obstacle`,
//! `VisibilityGraph`, `Vehicle`, `VehicleRect`, `VehicleCluster`,
//! `VehiclePoly`, `ClusterOutlineGrid`).
//!
//! The native pathfinder only builds visibility graphs around clusters of
//! vehicles; everywhere else the low-level search walks squares. Square
//! nodes created by `PolygonalMap2::getNodeForSquare` live here too.

pub mod node;
pub mod vehicle;
pub mod visibility_graph;

use std::collections::BTreeMap;

use crate::pool::{Id, Pool};

pub use node::{Connection, Node};
pub use vehicle::{Vehicle, VehicleCluster, VehiclePoly, VehicleRect};
pub use visibility_graph::{AdjustStartEndNodeData, Edge, EdgeRing, Obstacle, VisibilityGraph};

pub type NodeId = Id<Node>;
pub type ConnId = Id<Connection>;
pub type EdgeId = Id<Edge>;
pub type EdgeRingId = Id<EdgeRing>;
pub type ObstacleId = Id<Obstacle>;
pub type VGraphId = Id<VisibilityGraph>;
pub type VehicleId = Id<Vehicle>;
pub type ClusterId = Id<VehicleCluster>;

/// Node flag: crawl node next to a vehicle.
pub const NODE_FLAG_CRAWL: i32 = 1;
/// Node flag: crawl node under a vehicle.
pub const NODE_FLAG_CRAWL_INTERIOR: i32 = 2;
/// Node flag: node is a vertex of an obstacle polygon.
pub const NODE_FLAG_IN_CHUNK_DATA: i32 = 4;
/// Node flag: node is on the perimeter of a visibility graph.
pub const NODE_FLAG_PERIMETER: i32 = 8;
/// Node flag: node is the top or bottom of a staircase.
pub const NODE_FLAG_STAIR: i32 = 16;
/// Connection flag: crossing a can-path edge (door, window, fence).
pub const CXN_FLAG_CAN_PATH: i32 = 1;
/// Connection flag: crossing requires thumping.
pub const CXN_FLAG_THUMP: i32 = 2;
/// Connection flag: crossing a target-only path edge.
pub const CXN_FLAG_TARGET_PATH: i32 = 4;
/// Connection flag: crossing a bendable fence.
pub const CXN_FLAG_BENDABLE: i32 = 8;

/// All vehicle / visibility-graph state.
#[derive(Default)]
pub struct VisState {
    pub(crate) nodes: Pool<Node>,
    pub(crate) connections: Pool<Connection>,
    pub(crate) edges: Pool<Edge>,
    pub(crate) edge_rings: Pool<EdgeRing>,
    pub(crate) obstacles: Pool<Obstacle>,
    pub(crate) graphs_pool: Pool<VisibilityGraph>,
    pub(crate) vehicles_pool: Pool<Vehicle>,
    pub(crate) clusters_pool: Pool<VehicleCluster>,
    /// `World` vehicle map, by vehicle id (iterated in id order).
    pub(crate) vehicle_map: BTreeMap<i32, VehicleId>,
    /// `PolygonalMap2::vehicles`, refreshed from the map on rebuild.
    pub(crate) vehicles: Vec<VehicleId>,
    /// `PolygonalMap2::clusters`.
    pub(crate) clusters: Vec<ClusterId>,
    /// `PolygonalMap2::graphs`.
    pub(crate) graphs: Vec<VGraphId>,
}
