//! Isometric projection and world depth: `IsoUtils`' screen transforms,
//! `IsoDepthHelper` and the level pairing `FBORenderLevels` uses.
//!
//! All arithmetic is `f32` in the original's operation order, so positions
//! and depths match bit for bit.

/// `Core.tileScale`.
pub const TILE_SCALE: i32 = 2;
/// `IsoDepthHelper.CHUNK_DEPTH`.
pub const CHUNK_DEPTH: f32 = 0.023_093_667;
/// `IsoDepthHelper.SQUARE_DEPTH` and `LEVEL_DEPTH`.
pub const LEVEL_DEPTH: f32 = 0.002_886_708_4;
/// `IsoDepthHelper.CHUNK_WIDTH_OF_DEPTH_BUFFER`.
pub const CHUNK_WIDTH_OF_DEPTH_BUFFER: i32 = 20;
/// Squares per chunk side.
pub const CHUNK_SIZE: i32 = 8;
/// `FBORenderChunk.PIXELS_PER_LEVEL` at tile scale 2.
pub const PIXELS_PER_LEVEL: i32 = 96 * TILE_SCALE;

/// `PZMath.fastfloor(float)`.
pub fn fastfloor(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 {
        i - 1
    } else {
        i
    }
}

/// `PZMath.coordmodulof`.
pub fn coordmodulof(v: f32, divisor: i32) -> f32 {
    v - (fastfloor(v / divisor as f32) * divisor) as f32
}

/// `IsoUtils.XToScreen`.
pub fn x_to_screen(x: f32, y: f32) -> f32 {
    let mut sx = 0.0f32;
    sx += x * (32 * TILE_SCALE) as f32;
    sx - y * (32 * TILE_SCALE) as f32
}

/// `IsoUtils.YToScreen`.
pub fn y_to_screen(x: f32, y: f32, z: f32, screen_z: i32) -> f32 {
    let mut sy = 0.0f32;
    sy += y * (16 * TILE_SCALE) as f32;
    sy += x * (16 * TILE_SCALE) as f32;
    sy + (screen_z as f32 - z) * (96 * TILE_SCALE) as f32
}

/// `IsoUtils.XToIso` with the camera offset already added to the screen
/// position.
pub fn x_to_iso(px: f32, py: f32, floor: f32) -> f32 {
    (px + 2.0 * py) / (64.0 * TILE_SCALE as f32) + 3.0 * floor
}

/// `IsoUtils.YToIso`, as [`x_to_iso`].
pub fn y_to_iso(px: f32, py: f32, floor: f32) -> f32 {
    (px - 2.0 * py) / (-64.0 * TILE_SCALE as f32) + 3.0 * floor
}

/// `FBORenderLevels.calculateMinLevel`: the even level starting the pair
/// that one chunk texture holds.
pub fn calculate_min_level(level: i32) -> i32 {
    if level < 0 {
        -((level + 1).abs() / 2 + 1) * 2
    } else {
        level / 2 * 2
    }
}

/// `IsoDepthHelper.Results`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DepthResults {
    pub index_x: i32,
    pub index_y: i32,
    pub index_x2: i32,
    pub index_y2: i32,
    pub size_x: i32,
    pub size_y: i32,
    pub depth_start: f32,
    pub depth_end: f32,
    pub max_depth: f32,
}

/// `IsoDepthHelper.getChunkDepthData`: depth of chunk `(wx, wy)` at `level`
/// relative to the camera's chunk. Chunks further south-east are nearer.
pub fn chunk_depth_data(centre_wx: i32, centre_wy: i32, wx: i32, wy: i32, level: i32) -> DepthResults {
    let (xscr_min, yscr_min) = (centre_wx + 10, centre_wy + 10);
    let (xscr_max, yscr_max) = (centre_wx - 10, centre_wy - 10);
    let dif_x = (xscr_max - xscr_min).abs() * 8;
    let dif_y = (yscr_max - yscr_min).abs() * 8;
    let index_x = (dif_x / 8 - (wx - xscr_max)) * 8;
    let index_y = (dif_y / 8 - (wy - yscr_max)) * 8;
    let index_x2 = (dif_x / 8 - (wx - 1 - xscr_max)) * 8;
    let index_y2 = (dif_y / 8 - (wy - 1 - yscr_max)) * 8;
    let max = (dif_x + dif_y) as f32;
    let mut depth_start = (index_x + index_y) as f32 / 8.0 / 40.0;
    depth_start *= 0.461_873_35;
    depth_start -= calculate_min_level(level) as f32 * LEVEL_DEPTH;
    DepthResults {
        index_x,
        index_y,
        index_x2,
        index_y2,
        size_x: dif_x,
        size_y: dif_y,
        depth_start,
        depth_end: (index_x2 + index_y2) as f32 / max,
        max_depth: max,
    }
}

/// `IsoDepthHelper.calculateDepth`: depth of a point within its chunk level
/// pair.
pub fn calculate_depth(x: f32, y: f32, z: f32) -> f32 {
    let x_mod = 8.0 - coordmodulof(x, 8);
    let y_mod = 8.0 - coordmodulof(y, 8);
    let min_level = calculate_min_level(fastfloor(z));
    let z_offset = (2.0 - (z - min_level as f32)) * LEVEL_DEPTH;
    (x_mod + y_mod) / 16.0 * CHUNK_DEPTH + z_offset
}

/// `IsoDepthHelper.getSquareDepthData(...).depthStart`, with the camera
/// given as a square position.
pub fn square_depth(centre_x: i32, centre_y: i32, x: f32, y: f32, z: f32) -> f32 {
    let cx = fastfloor(centre_x as f32 / 8.0);
    let cy = fastfloor(centre_y as f32 / 8.0);
    let r = chunk_depth_data(cx, cy, fastfloor(x / 8.0), fastfloor(y / 8.0), fastfloor(z));
    r.depth_start + calculate_depth(x, y, z)
}

/// The far (`zDepthBlendToZ`) and front (`zDepthBlendZ`) depths
/// `IsoSprite.startTileDepthShader` gives a tile at `(x, y, z)`. When the
/// tile is recorded into a chunk texture, `chunk` is that chunk and level
/// and both depths are made relative to its depth.
pub fn tile_depths(cam_x: f32, cam_y: f32, x: f32, y: f32, z: f32, chunk: Option<(i32, i32, i32)>) -> (f32, f32) {
    let (cx, cy) = (fastfloor(cam_x), fastfloor(cam_y));
    let mut far = square_depth(cx, cy, x, y, z);
    let mut front = square_depth(cx, cy, x + 1.0, y + 1.0, z + 1.0);
    if let Some((wx, wy, level)) = chunk {
        let d = chunk_depth_data(fastfloor(cam_x / 8.0), fastfloor(cam_y / 8.0), wx, wy, level).depth_start;
        far -= d;
        front -= d;
    }
    (far, front)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_level_pairs() {
        let got: Vec<i32> = (-4..4).map(calculate_min_level).collect();
        assert_eq!(got, [-4, -4, -2, -2, 0, 0, 2, 2]);
    }

    #[test]
    fn screen_transforms() {
        assert_eq!(x_to_screen(10.0, 4.0), 384.0);
        assert_eq!(y_to_screen(10.0, 4.0, 1.0, 0), 448.0 - 192.0);
        let (sx, sy) = (x_to_screen(13.0, 7.0), y_to_screen(13.0, 7.0, 0.0, 0));
        assert_eq!(x_to_iso(sx, sy, 0.0), 13.0);
        assert_eq!(y_to_iso(sx, sy, 0.0), 7.0);
    }

    #[test]
    fn depth_orders_south_east_nearer() {
        let near = square_depth(100, 100, 101.0, 101.0, 0.0);
        let far = square_depth(100, 100, 100.0, 100.0, 0.0);
        assert!(near < far);
        // The camera chunk sits in the middle of the 20-chunk range.
        let r = chunk_depth_data(12, 12, 12, 12, 0);
        assert_eq!((r.index_x, r.index_y, r.size_x), (80, 80, 160));
        assert_eq!(r.depth_start, (160.0f32 / 8.0 / 40.0) * 0.461_873_35);
    }
}
