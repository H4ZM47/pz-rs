//! The weather mask's world queries ([`FxMaskWorld`]) over pz-world: the
//! render squares of a [`RenderWorld`] and the world regions of
//! [`IsoRegions`] (player-built rooms count as "inside" when their region
//! is a fog mask).

use pz_iso_render::weather_fx_mask::{FxMaskWorld, RegionId};
use pz_iso_render::RenderSquare;
use pz_world::isoregion::{IsoRegions, WorldRegionRef};
use pz_world::IsoCell;

use crate::render_world::RenderWorld;

/// [`FxMaskWorld`] over a loaded cell, its render squares and regions.
pub struct WorldFxMask<'a> {
    pub render: &'a RenderWorld,
    pub cell: &'a IsoCell,
    pub regions: &'a IsoRegions,
}

/// `IsoMovingObject.getMasterRegion()` for an object on square
/// `(x, y, z)`: that square's `getIsoWorldRegion()` (`None` without a
/// square).
pub fn master_region(
    cell: &IsoCell,
    regions: &IsoRegions,
    x: i32,
    y: i32,
    z: i32,
) -> Option<RegionId> {
    let square = cell.get_grid_square(x, y, z)?;
    let region = square.borrow().get_iso_world_region(regions);
    region.map(|r| r.0)
}

impl FxMaskWorld for WorldFxMask<'_> {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare> {
        self.render.squares.get(&(x, y, z))
    }

    /// `gs.getIsoWorldRegion()` (cached on the square) where the cell has
    /// the square, else `IsoRegions.getIsoWorldRegion(x, y, z)`.
    fn world_region(&self, x: i32, y: i32, z: i32) -> Option<RegionId> {
        match self.cell.get_grid_square(x, y, z) {
            Some(sq) => {
                let region = sq.borrow().get_iso_world_region(self.regions);
                region
            }
            None => self.regions.get_iso_world_region(x, y, z),
        }
        .map(|r| r.0)
    }

    fn region_is_fog_mask(&self, r: RegionId) -> bool {
        self.regions.is_fog_mask(WorldRegionRef(r))
    }

    fn region_neighbors(&self, r: RegionId) -> Vec<RegionId> {
        self.regions
            .neighbors(WorldRegionRef(r))
            .iter()
            .map(|n| n.0)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::isoregion::{JobChunkUpdate, RegionJob, REGION_WORLD_VERSION};
    use pz_world::{IsoChunk, IsoGridSquare};

    /// A region-file block: chunk (0, 0) floored, with a walled room at
    /// (2..5, 2..6) roofed by level 1, and a walled unroofed shed at
    /// (6..8, 0..2) open to the chunk's east edge.
    fn block() -> Vec<u8> {
        let mut l0 = [16i8; 64];
        for x in 2..5 {
            l0[2 * 8 + x] |= 1;
            l0[6 * 8 + x] |= 1;
        }
        for y in 2..6 {
            l0[y * 8 + 2] |= 2;
            l0[y * 8 + 5] |= 2;
        }
        for x in 6..8 {
            l0[2 * 8 + x] |= 1;
        }
        for y in 0..2 {
            l0[y * 8 + 6] |= 2;
        }
        let mut l1 = [0i8; 64];
        for y in 2..6 {
            for x in 2..5 {
                l1[y * 8 + x] = 16;
            }
        }
        let mut b = Vec::new();
        for v in [0, REGION_WORLD_VERSION, 0, 0, 12 + 128, 1, 128] {
            b.extend_from_slice(&i32::to_be_bytes(v));
        }
        b.extend(l0.iter().chain(l1.iter()).map(|&f| f as u8));
        let n = b.len() as i32;
        b[..4].copy_from_slice(&n.to_be_bytes());
        b
    }

    fn regions() -> IsoRegions {
        let mut r = IsoRegions::new(13);
        let mut j = JobChunkUpdate::new();
        j.add_chunk_from_file(&block()).unwrap();
        r.enqueue_job(RegionJob::ChunkUpdate(j));
        r.apply_chunk_changes(false);
        r.worker_tick();
        r
    }

    #[test]
    fn regions_reach_the_mask() {
        let regions = regions();
        let mut cell = IsoCell::default();
        let mut ch = IsoChunk::new(0, 0);
        ch.set_square(3, 3, 0, Some((IsoGridSquare::new(3, 3, 0)).into()));
        cell.chunks.insert((0, 0), ch);
        let mut render = RenderWorld::default();
        render.squares.insert((3, 3, 0), RenderSquare::new(3, 3, 0));
        let w = WorldFxMask {
            render: &render,
            cell: &cell,
            regions: &regions,
        };
        assert!(w.square(3, 3, 0).is_some() && w.square(0, 0, 0).is_none());
        // From the square (cached) and without one (direct lookup).
        let room = w.world_region(3, 3, 0).unwrap();
        assert_eq!(w.world_region(4, 5, 0), Some(room));
        assert!(w.region_is_fog_mask(room));
        let outside = w.world_region(0, 0, 0).unwrap();
        assert!(!w.region_is_fog_mask(outside));
        assert_eq!(w.region_neighbors(room), vec![outside]);
        let shed = w.world_region(7, 0, 0).unwrap();
        assert_ne!(shed, outside);
        assert!(!w.region_is_fog_mask(shed));
        assert!(w.region_neighbors(outside).contains(&shed));
        assert_eq!(w.world_region(9, 0, 0), None);
        assert_eq!(master_region(&cell, &regions, 3, 3, 0), Some(room));
        assert_eq!(master_region(&cell, &regions, 4, 4, 0), None);
    }
}
