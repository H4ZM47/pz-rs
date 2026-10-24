//! `HLChunkLevel`: the regions, staircases and sloped surfaces of one
//! chunk level, rebuilt lazily whenever `HLAStar::ModificationCount`
//! changes.

use super::{HlChunkRegion, HlLevelTransition, RegionId, TransId, TransitionKind};
use crate::square::bits::*;
use crate::square::Square;
use crate::util::Dir;
use crate::world::{LevelRef, SquareId, World};

/// `HLChunkLevel`.
#[derive(Clone, Debug)]
pub struct HlChunkLevel {
    pub(crate) modification_count: i32,
    pub(crate) modification_count_stairs: i32,
    pub(crate) regions: Vec<RegionId>,
    pub(crate) stairs: Vec<TransId>,
    pub(crate) sloped_surfaces: Vec<TransId>,
}

impl HlChunkLevel {
    pub(crate) fn new() -> Self {
        HlChunkLevel {
            modification_count: -1,
            modification_count_stairs: -1,
            regions: Vec::new(),
            stairs: Vec::new(),
            sloped_surfaces: Vec::new(),
        }
    }
}

/// `HLChunkLevel::canWalkOnSquare`.
pub(crate) fn can_walk_on_square(w: &World, s: Option<SquareId>) -> bool {
    match s {
        None => false,
        Some(s) => {
            let sq = w.sq(s);
            sq.treat_as_solid_floor() && !sq.is_really_solid(w)
        }
    }
}

/// `HLChunkLevel::isCanPathTransition`.
pub(crate) fn is_can_path_transition(w: &World, a: Option<SquareId>, b: Option<SquareId>) -> bool {
    let (a, b) = match (a, b) {
        (Some(a), Some(b)) if a != b => (a, b),
        _ => return false,
    };
    let (s1, s2): (&Square, &Square) = (w.sq(a), w.sq(b));
    if s1.z != s2.z {
        return false;
    }
    let to_w = s2.x < s1.x;
    if to_w && s1.has(CAN_PATH_W) {
        return true;
    }
    if !to_w && s2.x != s1.x && s2.has(CAN_PATH_W) {
        return true;
    }
    if s2.y < s1.y {
        s1.has(CAN_PATH_N)
    } else if s2.y != s1.y {
        s2.has(CAN_PATH_N)
    } else {
        false
    }
}

impl World {
    /// `ChunkLevel::getHighLevelData`: rebuild the regions if the world
    /// changed since they were computed.
    pub(crate) fn get_high_level_data(&mut self, r: LevelRef) {
        let mc = self.modification_count;
        let lvl = self.chunk_level_mut(r);
        if lvl.hl.modification_count != mc {
            lvl.hl.modification_count = mc;
            self.hl_init_regions(r);
        }
    }

    /// The level data of a chunk level, after `getHighLevelData`.
    pub(crate) fn hl_level(&mut self, r: LevelRef) -> &super::hl_chunk_level::HlChunkLevel {
        self.get_high_level_data(r);
        &self.chunk_level(r).hl
    }

    /// `HLChunkLevel::releaseRegions` + stairs + sloped surfaces
    /// (`removeFromWorld`).
    pub(crate) fn hl_remove_from_world(&mut self, hl: &mut HlChunkLevel) {
        for r in hl.regions.drain(..) {
            self.hl.regions.release(r);
        }
        for t in hl.stairs.drain(..) {
            self.hl.transitions.release(t);
        }
        for t in hl.sloped_surfaces.drain(..) {
            self.hl.transitions.release(t);
        }
        hl.modification_count = -1;
        hl.modification_count_stairs = -1;
    }

    /// `HLChunkLevel::initRegions`.
    fn hl_init_regions(&mut self, r: LevelRef) {
        let old = std::mem::take(&mut self.chunk_level_mut(r).hl.regions);
        for reg in old {
            self.hl.regions.release(reg);
        }
        let squares = self.chunk_level(r).squares;
        for y in 0..8 {
            for x in 0..8 {
                let s = match squares[x][y] {
                    Some(s) => s,
                    None => continue,
                };
                let sq = self.sq(s);
                let walkable = if sq.has(THUMPABLE) {
                    sq.treat_as_solid_floor()
                } else {
                    sq.treat_as_solid_floor() && !sq.is_really_solid(self)
                };
                if !walkable {
                    continue;
                }
                let (sx, sy, sz) = (sq.x, sq.y, sq.z);
                if self.hl_find_region_containing_square(r, sx, sy).is_some() {
                    continue;
                }
                let in_graph = self
                    .get_vis_graph_at(sx as f32 + 0.5, sy as f32 + 0.5, sz, 1)
                    .is_some();
                let sq = self.sq(s);
                if in_graph && !sq.has(ALL_STAIRS) && !sq.has_sloped_surface() {
                    continue;
                }
                let region = self.hl.regions.alloc(HlChunkRegion::new(r));
                self.flood_fill(region, s);
                self.hl.regions[region].init_edges();
                self.chunk_level_mut(r).hl.regions.push(region);
            }
        }
    }

    /// `HLChunkLevel::findRegionContainingSquare` (on already initialised
    /// level data).
    pub(crate) fn hl_find_region_containing_square(
        &self,
        r: LevelRef,
        x: i32,
        y: i32,
    ) -> Option<RegionId> {
        self.chunk_level(r)
            .hl
            .regions
            .iter()
            .copied()
            .find(|&reg| self.region_contains_square(reg, x, y))
    }

    /// `HLChunkLevel::initStairsIfNeeded`.
    pub(crate) fn hl_init_stairs_if_needed(&mut self, r: LevelRef) {
        let mc = self.modification_count;
        if self.chunk_level(r).hl.modification_count_stairs == mc {
            return;
        }
        self.chunk_level_mut(r).hl.modification_count_stairs = mc;
        self.hl_init_stairs(r);
    }

    /// `HLChunkLevel::initStairs` (and `initSlopedSurfaces`).
    fn hl_init_stairs(&mut self, r: LevelRef) {
        let old = std::mem::take(&mut self.chunk_level_mut(r).hl.stairs);
        for t in old {
            self.hl.transitions.release(t);
        }
        let squares = self.chunk_level(r).squares;
        let mut stairs = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let s = match squares[x][y] {
                    Some(s) => s,
                    None => continue,
                };
                let sq = self.sq(s).clone();
                if sq.has(STAIR_TN) {
                    stairs.push(self.hl.transitions.alloc(HlLevelTransition {
                        kind: TransitionKind::Staircase,
                        dir: Dir::N,
                        x: sq.x,
                        y: sq.y,
                        z: sq.z,
                    }));
                }
                if sq.has(STAIR_TW) {
                    stairs.push(self.hl.transitions.alloc(HlLevelTransition {
                        kind: TransitionKind::Staircase,
                        dir: Dir::W,
                        x: sq.x,
                        y: sq.y,
                        z: sq.z,
                    }));
                }
            }
        }
        self.chunk_level_mut(r).hl.stairs = stairs;

        let old = std::mem::take(&mut self.chunk_level_mut(r).hl.sloped_surfaces);
        for t in old {
            self.hl.transitions.release(t);
        }
        let mut slopes = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let s = match squares[x][y] {
                    Some(s) => s,
                    None => continue,
                };
                let sq = self.sq(s).clone();
                if let Some(dir) = sq.slope_dir {
                    if sq.slope_max >= 1.0 {
                        slopes.push(self.hl.transitions.alloc(HlLevelTransition {
                            kind: TransitionKind::SlopedSurface,
                            dir,
                            x: sq.x,
                            y: sq.y,
                            z: sq.z,
                        }));
                    }
                }
            }
        }
        self.chunk_level_mut(r).hl.sloped_surfaces = slopes;
    }

    /// `HLChunkLevel::getLevelTransitionAt`.
    pub(crate) fn hl_level_transition_at_level(
        &self,
        r: LevelRef,
        x: i32,
        y: i32,
    ) -> Option<TransId> {
        let hl = &self.chunk_level(r).hl;
        hl.stairs
            .iter()
            .chain(hl.sloped_surfaces.iter())
            .copied()
            .find(|&t| self.trans(t).is_bottom_floor_at(x, y))
    }
}
