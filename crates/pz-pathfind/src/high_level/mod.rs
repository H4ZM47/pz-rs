//! The high-level search (`zombie.pathfind.highLevel` / the native `HL*`
//! classes): each chunk level is split into connected regions of squares,
//! and an A* over regions, staircases, sloped surfaces and vehicle
//! visibility-graph nodes picks the chunk levels and level transitions the
//! low-level search may use.

pub mod flood_fill;
pub mod hl_astar;
pub mod hl_chunk_level;
pub mod hl_chunk_region;
pub mod hl_level_transition;
pub mod hl_search_node;

use std::collections::HashMap;

use crate::pool::{Id, Pool};
use crate::visibility::{NodeId, VGraphId};

pub use hl_chunk_region::HlChunkRegion;
pub use hl_level_transition::{HlLevelTransition, TransitionKind};
pub use hl_search_node::HlSearchNode;

pub type RegionId = Id<HlChunkRegion>;
pub type TransId = Id<HlLevelTransition>;

/// `HLAStar::AdjustVisibilityGraphData`.
#[derive(Clone, Debug, Default)]
pub(crate) struct AdjustVisibilityGraphData {
    pub adjust: crate::visibility::AdjustStartEndNodeData,
    pub adjusted: bool,
    pub graph: Option<VGraphId>,
    pub vg_node: Option<NodeId>,
}

/// Everything the high-level search keeps between calls (`HLGlobals`,
/// the `HLAStar` singleton and the shared `FloodFill`).
#[derive(Default)]
pub struct HighLevelState {
    pub(crate) regions: Pool<HlChunkRegion>,
    pub(crate) transitions: Pool<HlLevelTransition>,

    // HLAStar search state.
    pub(crate) nodes: Vec<HlSearchNode>,
    pub(crate) node_map_region: HashMap<RegionId, u32>,
    pub(crate) node_map_transition: HashMap<TransId, u32>,
    pub(crate) node_map_vg: HashMap<NodeId, u32>,
    pub(crate) unloaded_node: Option<u32>,
    pub(crate) goal: u32,
    pub(crate) goal_in_unloaded_area: bool,
    pub(crate) adjust_start: AdjustVisibilityGraphData,
    pub(crate) adjust_goal: AdjustVisibilityGraphData,
    pub(crate) visibility_graphs: Vec<VGraphId>,
}
