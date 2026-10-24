//! `FBORenderSnow`: snow on a chunk level's outdoor floors, recorded into
//! the chunk texture right after the floors with the depth test off.
//!
//! Which squares get snow comes from a 10x10 `SnowGrid` over the chunk and
//! its one-square border: `Noise2D` against the snow fraction picks full
//! squares, and the empty squares beside them get edge, strip, cove or
//! enclosed tiles. Squares on the water's edge get the strips facing land.
//! The grid is a pure function of the world and the snow target, so it is
//! rebuilt whenever the chunk texture is recorded (the original caches it
//! per chunk level and rebuilds it on the same changes).
//!
//! Tiles are `e_newsnow_ground_1_<n>`, looked up through
//! [`crate::weather::SurfaceTextures`].

use pz_render::iso::{calculate_depth, coordmodulof, x_to_screen, y_to_screen, TILE_SCALE};
use pz_render::{DrawCmd, Rgba, Texture, WorldShader};

use crate::chunk::ChunkTexture;
use crate::color;
use crate::texd::{FloorShaper, TexDraw};
use crate::weather::Surfaces;
use crate::world::RenderSquare;

/// `erosion.utils.Noise2D`.
#[derive(Clone, Debug, Default)]
pub struct Noise2D {
    layers: Vec<NoiseLayer>,
}

#[derive(Clone, Debug)]
struct NoiseLayer {
    freq: f32,
    amp: f32,
    p: Vec<i32>,
}

const PERM: [i32; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175, 74, 165, 71, 134, 139, 48, 27, 166,
    77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212,
    207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239,
    107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141, 128, 195, 78, 66, 215, 61, 156, 180,
];

impl Noise2D {
    fn fade(t: f32) -> f32 {
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    fn lerp(t: f32, a: f32, b: f32) -> f32 {
        a + t * (b - a)
    }

    fn noise(orig_x: f32, orig_y: f32, p: &[i32]) -> f32 {
        let x = (orig_x as f64 - (orig_x / 255.0).floor() as f64 * 255.0).floor() as i32;
        let y = (orig_y as f64 - (orig_y / 255.0).floor() as f64 * 255.0).floor() as i32;
        let fx = Self::fade(orig_x - orig_x.floor());
        let fy = Self::fade(orig_y - orig_y.floor());
        let (x, y) = (x as usize, y as usize);
        let aa = p[x] as usize + y;
        let ab = p[x] as usize + y + 1;
        let ba = p[x + 1] as usize + y;
        let bb = p[x + 1] as usize + y + 1;
        let pf = |i: usize| PERM[p[i] as usize] as f32;
        Self::lerp(fy, Self::lerp(fx, pf(aa), pf(ba)), Self::lerp(fx, pf(ab), pf(bb)))
    }

    /// `layeredNoise`: 0..1.
    pub fn layered_noise(&self, x: f32, y: f32) -> f32 {
        let mut f = 0.0;
        let mut max_sum = 0.0;
        for l in &self.layers {
            max_sum += l.amp;
            f += Self::noise(x * l.freq, y * l.freq, &l.p) * l.amp;
        }
        f / max_sum / 255.0
    }

    pub fn add_layer(&mut self, seed: i32, freq: f32, amp: f32) {
        let wrap = |v: i32| (v as f64 - (v as f32 / 256.0).floor() as f64 * 256.0).floor() as i32;
        let seed = wrap(seed);
        let mut p = vec![0; 512];
        for (i, &v) in PERM.iter().enumerate() {
            let ii = wrap(seed + i as i32) as usize;
            p[ii] = v;
            p[256 + ii] = v;
        }
        self.layers.push(NoiseLayer { freq, amp, p });
    }

    /// The snow noise: `initSnowGrid`'s three layers.
    pub fn snow() -> Self {
        let mut n = Noise2D::default();
        n.add_layer(16, 0.5, 3.0);
        n.add_layer(32, 2.0, 5.0);
        n.add_layer(64, 5.0, 8.0);
        n
    }
}

/// A `SnowGridTiles` set: its id and its tiles' numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tiles {
    id: u8,
    tiles: [u8; 4],
    len: usize,
}

impl Tiles {
    const fn new(id: u8, base: u8, step: u8, len: usize) -> Self {
        let mut tiles = [0; 4];
        let mut i = 0;
        while i < len {
            tiles[i] = base + step * i as u8;
            i += 1;
        }
        Tiles { id, tiles, len }
    }

    /// `getAt(worldX, worldY)` (the original offsets both by the meta
    /// grid's minimum x).
    fn get_at(&self, world_x: i32, world_y: i32, min_x: i32) -> u8 {
        let i = ((world_x - min_x * 256) + (world_y - min_x * 256)).rem_euclid(self.len as i32);
        self.tiles[i as usize]
    }

    fn contains(&self, tile: u8) -> bool {
        self.tiles[..self.len].contains(&tile)
    }
}

const SQUARE: Tiles = Tiles::new(0, 40, 1, 4);
const ENCLOSED: Tiles = Tiles::new(1, 0, 1, 4);
const COVE: [Tiles; 4] = [Tiles::new(2, 7, 4, 3), Tiles::new(3, 5, 4, 3), Tiles::new(4, 4, 4, 3), Tiles::new(5, 6, 4, 3)];
/// `snowFirstNonSquare`: square, enclosed and cove tiles are drawn as
/// diamonds.
const FIRST_NON_SQUARE: u8 = 6;
const EDGE: [Tiles; 4] = [Tiles::new(6, 16, 4, 3), Tiles::new(7, 17, 4, 3), Tiles::new(8, 18, 4, 3), Tiles::new(9, 19, 4, 3)];
const STRIP: [Tiles; 4] = [Tiles::new(10, 28, 4, 3), Tiles::new(11, 31, 4, 3), Tiles::new(12, 29, 4, 3), Tiles::new(13, 30, 4, 3)];

const SEAM_E: [u8; 30] = [7, 6, 5, 3, 2, 1, 0, 15, 14, 13, 11, 10, 9, 23, 21, 19, 17, 30, 27, 25, 38, 34, 47, 46, 45, 44, 43, 42, 41, 40];
const SEAM_S: [u8; 30] = [6, 5, 4, 3, 2, 1, 0, 14, 13, 12, 10, 9, 8, 22, 21, 18, 17, 31, 26, 25, 39, 35, 47, 46, 45, 44, 43, 42, 41, 40];

/// A grid cell's two layers: the tile number and its set's id.
type Cell = [Option<(u8, u8)>; 2];

/// `SnowGrid`: two tile layers over a 10x10 window.
#[derive(Clone, Debug, PartialEq)]
pub struct SnowGrid {
    pub world_x: i32,
    pub world_y: i32,
    pub frac: i32,
    /// `[x][y][layer]`: the tile number and its set's id.
    grid: [[Cell; 10]; 10],
}

const W: i32 = 10;
const H: i32 = 10;

fn wrap_index(mut x: i32, mut y: i32) -> Option<(usize, usize)> {
    if x == W {
        x = 0;
    }
    if x == -1 {
        x = W - 1;
    }
    if y == H {
        y = 0;
    }
    if y == -1 {
        y = H - 1;
    }
    ((0..W).contains(&x) && (0..H).contains(&y)).then_some((x as usize, y as usize))
}

/// The eight `IsoDirections` neighbour offsets.
const DIRECTIONS: [(i32, i32); 8] = [(0, -1), (-1, -1), (-1, 0), (-1, 1), (0, 1), (1, 1), (1, 0), (1, -1)];

impl SnowGrid {
    /// `SnowGrid.init`. `square(x, y, z)` is `getGridSquare`.
    #[allow(clippy::too_many_arguments)]
    pub fn new<'s>(world_x: i32, world_y: i32, level: i32, frac: i32, winter: bool, min_x: i32, noise: &Noise2D, square: impl Fn(i32, i32, i32) -> Option<&'s RenderSquare>) -> Self {
        let mut g = SnowGrid { world_x, world_y, frac, grid: [[[None; 2]; 10]; 10] };
        if !winter {
            return g;
        }
        for y in 0..H {
            for x in 0..W {
                if level == 0 {
                    let Some(sq) = square(world_x + x, world_y + y, level) else { continue };
                    let near_water = DIRECTIONS.iter().any(|(dx, dy)| square(sq.x + dx, sq.y + dy, sq.z).and_then(|s| s.water.as_ref()).is_some_and(|w| w.is_valid()));
                    if near_water {
                        continue;
                    }
                }
                let nx = (world_x + x).rem_euclid(256);
                let ny = (world_y + y).rem_euclid(256);
                if noise.layered_noise(nx as f32 / 10.0, ny as f32 / 10.0) <= frac as f32 / 100.0 {
                    g.grid[x as usize][y as usize][0] = Some((SQUARE.get_at(world_x + x, world_y + y, min_x), SQUARE.id));
                }
            }
        }
        for y in 0..H {
            for x in 0..W {
                if g.grid[x as usize][y as usize][0].is_some() {
                    continue;
                }
                let (n, s, w, e) = (g.check(x, y - 1), g.check(x, y + 1), g.check(x - 1, y), g.check(x + 1, y));
                let count = [n, s, e, w].iter().filter(|b| **b).count();
                match count {
                    1 => {
                        let t = if n {
                            STRIP[0]
                        } else if s {
                            STRIP[1]
                        } else if e {
                            STRIP[3]
                        } else {
                            STRIP[2]
                        };
                        g.set(x, y, 0, t, min_x);
                    }
                    2 => {
                        if n && s {
                            g.set(x, y, 0, STRIP[0], min_x);
                            g.set(x, y, 1, STRIP[1], min_x);
                        } else if e && w {
                            g.set(x, y, 0, STRIP[2], min_x);
                            g.set(x, y, 1, STRIP[3], min_x);
                        } else if n {
                            g.set(x, y, 0, EDGE[if w { 0 } else { 3 }], min_x);
                        } else if s {
                            g.set(x, y, 0, EDGE[if w { 2 } else { 1 }], min_x);
                        } else if w {
                            g.set(x, y, 0, EDGE[if n { 0 } else { 2 }], min_x);
                        } else if e {
                            g.set(x, y, 0, EDGE[if n { 3 } else { 1 }], min_x);
                        }
                    }
                    3 => {
                        if !n {
                            g.set(x, y, 0, COVE[1], min_x);
                        } else if !s {
                            g.set(x, y, 0, COVE[0], min_x);
                        } else if !e {
                            g.set(x, y, 0, COVE[2], min_x);
                        } else if !w {
                            g.set(x, y, 0, COVE[3], min_x);
                        }
                    }
                    4 => g.set(x, y, 0, ENCLOSED, min_x),
                    _ => {}
                }
            }
        }
        g
    }

    /// `check`: a full snow square (wrapping one step past the edges).
    pub fn check(&self, x: i32, y: i32) -> bool {
        wrap_index(x, y).and_then(|(x, y)| self.grid[x][y][0]).is_some_and(|(t, _)| SQUARE.contains(t))
    }

    fn set(&mut self, x: i32, y: i32, layer: usize, tiles: Tiles, min_x: i32) {
        if let Some((ix, iy)) = wrap_index(x, y) {
            self.grid[ix][iy][layer] = Some((tiles.get_at(self.world_x + ix as i32, self.world_y + iy as i32, min_x), tiles.id));
        }
    }

    /// The tile and set id at a grid position.
    pub fn tile(&self, x: i32, y: i32, layer: usize) -> Option<(u8, u8)> {
        if !(0..W).contains(&x) || !(0..H).contains(&y) {
            return None;
        }
        self.grid[x as usize][y as usize][layer]
    }
}

fn has(sq: &RenderSquare, f: impl Fn(&crate::world::SpriteFlags) -> bool) -> bool {
    sq.objects.iter().any(|o| o.sprite.as_ref().is_some_and(|s| f(&s.flags)))
}

fn water_valid(sq: &RenderSquare) -> bool {
    sq.water.as_ref().is_some_and(|w| w.is_valid())
}

/// `getShoreInt`: which sides (N 1, E 2, S 4, W 8) face land, looked up on
/// level 0.
fn shore_int(s: &Surfaces<'_>, sq: &RenderSquare) -> i32 {
    let land = |dx: i32, dy: i32| s.world.square(sq.x + dx, sq.y + dy, 0).is_some_and(|n| !has(n, |f| f.water) && !water_valid(n));
    let mut shore = 0;
    if land(0, -1) {
        shore |= 1;
    }
    if land(1, 0) {
        shore |= 2;
    }
    if land(0, 1) {
        shore |= 4;
    }
    if land(-1, 0) {
        shore |= 8;
    }
    shore
}

/// One snow tile draw (`renderSnowTileBase`): the tile with the square's
/// corner lights, as a floor diamond for square tiles.
struct SnowDraw<'a, 'b> {
    s: &'a Surfaces<'b>,
    out: &'a mut Vec<DrawCmd>,
    z: f32,
}

impl SnowDraw<'_, '_> {
    fn tile(&mut self, tile: u8, sx: i32, sy: i32, cols: [u32; 4], square: bool) {
        let Some(tex) = self.s.textures.texture(&format!("e_newsnow_ground_1_{tile}")) else { return };
        self.texture(&tex, sx, sy, cols, square);
    }

    fn texture(&mut self, tex: &Texture, sx: i32, sy: i32, cols: [u32; 4], square: bool) {
        // setAlpha4(1).
        let col = cols.map(|c| color::set_alpha(c, 1.0));
        let shaper = FloorShaper { col, tint: 0, shore: false, water_depth: [0.0; 4] };
        let Some(mut d) = TexDraw::new(tex, sx as f32 + tex.offset_x, sy as f32 + tex.offset_y, tex.width as f32, tex.height as f32, Rgba(1.0, 1.0, 1.0, 1.0)) else { return };
        if square {
            shaper.accept_diamond(&mut d, 1);
        } else {
            shaper.accept(&mut d);
        }
        self.out.push(d.into_cmd(WorldShader::Default { z: self.z }));
    }
}

/// `FBORenderSnow.RenderSnow(chunk, level)` into chunk texture `tex`:
/// `floors` are the level's `IsoCell.SolidFloor` squares.
pub fn render_snow(s: &Surfaces<'_>, tex: ChunkTexture, level: i32, floors: &[&RenderSquare], out: &mut Vec<DrawCmd>) {
    let frac = s.climate.snow_target;
    if frac <= 0 {
        return;
    }
    let noise = Noise2D::snow();
    let grid = SnowGrid::new(tex.wx * 8 - 1, tex.wy * 8 - 1, level, frac, s.climate.winter, s.meta_grid_min_x, &noise, |x, y, z| s.world.square(x, y, z));
    let (xoff, yoff) = tex.record_offset();
    let mut d = SnowDraw { s, out, z: 0.0 };
    for sq in floors {
        if sq.cut.room.is_some() || !has(sq, |f| f.exterior) || !has(sq, |f| f.solidfloor) {
            continue;
        }
        let shore = if has(sq, |f| f.water) || water_valid(sq) {
            match shore_int(s, sq) {
                0 => continue,
                v => v,
            }
        } else {
            0
        };
        let (gx, gy) = (sq.x - grid.world_x, sq.y - grid.world_y);
        let (lx, ly) = (coordmodulof(sq.x as f32, 8), coordmodulof(sq.y as f32, 8));
        let mut sx = x_to_screen(lx, ly) + xoff;
        let mut sy = y_to_screen(lx, ly, level as f32, 0) + yoff;
        sx -= (32 * TILE_SCALE) as f32;
        sy -= (96 * TILE_SCALE) as f32;
        if has(sq, |f| f.floor_height_one_third) {
            sy -= (32 * TILE_SCALE) as f32;
        }
        if has(sq, |f| f.floor_height_two_thirds) {
            sy -= (64 * TILE_SCALE) as f32;
        }
        d.z = calculate_depth(lx, ly, level as f32) * 2.0 - 1.0;
        let v = sq.vert_lights;
        let mut cols = [v[0], v[1], v[2], v[3]];
        for layer in 0..2 {
            let Some((tile, id)) = grid.tile(gx, gy, layer) else { continue };
            if layer == 0 {
                render_snow_tile(s, &mut d, &grid, sq, shore, (gx, gy), (tile, id), (sx as i32, sy as i32), &mut cols);
            } else if shore == 0 {
                d.tile(tile, sx as i32, sy as i32, cols, id < FIRST_NON_SQUARE);
            }
        }
    }
}

/// `renderSnowTile`.
#[allow(clippy::too_many_arguments)]
fn render_snow_tile(s: &Surfaces<'_>, d: &mut SnowDraw<'_, '_>, grid: &SnowGrid, sq: &RenderSquare, shore: i32, (gx, gy): (i32, i32), (tile, id): (u8, u8), (sx, sy): (i32, i32), cols: &mut [u32; 4]) {
    let v = sq.vert_lights;
    if shore == 0 {
        d.tile(tile, sx, sy, *cols, id < FIRST_NON_SQUARE);
        // seamFix2: the strips over the chunk's south and east edges.
        let has_floor = |dx: i32, dy: i32| s.world.square(sq.x + dx, sq.y + dy, sq.z).is_some_and(|n| n.floor().is_some());
        if sq.y.rem_euclid(8) == 7 && SEAM_S.contains(&tile) && has_floor(0, 1) {
            *cols = [v[3], v[2], v[2], v[3]];
            d.tile(28, sx - 64, sy + 32, *cols, false);
        }
        if sq.x.rem_euclid(8) == 7 && SEAM_E.contains(&tile) && has_floor(1, 0) {
            *cols = [v[1], v[1], v[2], v[2]];
            d.tile(29, sx + 64, sy + 32, *cols, false);
        }
        return;
    }
    let full = grid.check(gx, gy);
    let n = shore & 1 == 1 && (full || grid.check(gx, gy - 1));
    let e = shore & 2 == 2 && (full || grid.check(gx + 1, gy));
    let so = shore & 4 == 4 && (full || grid.check(gx, gy + 1));
    let w = shore & 8 == 8 && (full || grid.check(gx - 1, gy));
    let count = [n, so, e, w].iter().filter(|b| **b).count();
    let (mut t1, mut t2, mut square) = (None, None, false);
    match count {
        1 => {
            t1 = Some(if n {
                STRIP[0]
            } else if so {
                STRIP[1]
            } else if e {
                STRIP[3]
            } else {
                STRIP[2]
            })
        }
        2 => {
            if n && so {
                t1 = Some(STRIP[0]);
                t2 = Some(STRIP[1]);
            } else if e && w {
                t1 = Some(STRIP[2]);
                t2 = Some(STRIP[3]);
            } else if n {
                t1 = Some(EDGE[if w { 0 } else { 3 }]);
            } else if so {
                t1 = Some(EDGE[if w { 2 } else { 1 }]);
            } else if w {
                t1 = Some(EDGE[if n { 0 } else { 2 }]);
            } else if e {
                t1 = Some(EDGE[if n { 3 } else { 1 }]);
            }
        }
        3 => {
            t1 = if !n {
                Some(COVE[1])
            } else if !so {
                Some(COVE[0])
            } else if !e {
                Some(COVE[2])
            } else if !w {
                Some(COVE[3])
            } else {
                None
            };
            square = true;
        }
        4 => {
            t1 = Some(ENCLOSED);
            square = true;
        }
        _ => return,
    }
    if let Some(t) = t1 {
        let var = ((sq.x + sq.y) % t.len as i32) as usize;
        d.tile(t.tiles[var], sx, sy, *cols, square);
        if let Some(t2) = t2 {
            d.tile(t2.tiles[var], sx, sy, *cols, false);
        }
    }
}

/// `FBORenderSnow.gridSquareIsSnow` for a square on a chunk level.
pub fn square_is_snow(s: &Surfaces<'_>, sq: &RenderSquare) -> bool {
    if s.climate.snow_target <= 0 {
        return false;
    }
    if !has(sq, |f| f.solidfloor) || has(sq, |f| f.water) || water_valid(sq) || !has(sq, |f| f.exterior) || sq.cut.room.is_some() {
        return false;
    }
    let (wx, wy) = (sq.x.div_euclid(8), sq.y.div_euclid(8));
    let noise = Noise2D::snow();
    let grid = SnowGrid::new(wx * 8 - 1, wy * 8 - 1, sq.z, s.climate.snow_target, s.climate.winter, s.meta_grid_min_x, &noise, |x, y, z| s.world.square(x, y, z));
    grid.check(sq.x - grid.world_x, sq.y - grid.world_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_in_range_and_deterministic() {
        let n = Noise2D::snow();
        let a = n.layered_noise(1.3, 4.7);
        assert!((0.0..=1.0).contains(&a));
        assert_eq!(a, Noise2D::snow().layered_noise(1.3, 4.7));
        // Integer lattice points take the permutation values directly.
        let mut one = Noise2D::default();
        one.add_layer(0, 1.0, 1.0);
        assert_eq!(one.layered_noise(0.0, 0.0), PERM[PERM[PERM[0] as usize] as usize] as f32 / 255.0);
    }

    #[test]
    fn full_snow_covers_everything_and_none_covers_nothing() {
        let n = Noise2D::snow();
        let full = SnowGrid::new(0, 0, 1, 100, true, 0, &n, |_, _, _| None);
        assert!((0..10).all(|x| (0..10).all(|y| full.check(x, y))));
        assert_eq!(full.tile(0, 0, 0).map(|t| t.0), Some(SQUARE.get_at(0, 0, 0)));
        let none = SnowGrid::new(0, 0, 1, 0, true, 0, &n, |_, _, _| None);
        // Noise at or below 0 is rare; only exact zeros would be snow.
        assert!((0..10).all(|x| (0..10).all(|y| !none.check(x, y) || n.layered_noise(x as f32 / 10.0, y as f32 / 10.0) <= 0.0)));
        let summer = SnowGrid::new(0, 0, 1, 100, false, 0, &n, |_, _, _| None);
        assert!(!summer.check(3, 3));
    }

    #[test]
    fn empty_squares_beside_snow_get_edges() {
        let mut g = SnowGrid { world_x: 0, world_y: 0, frac: 50, grid: [[[None; 2]; 10]; 10] };
        g.grid[4][4][0] = Some((40, SQUARE.id));
        assert!(g.check(4, 4));
        // (4, 5) has snow to its north only: the north strip.
        let (nn, s, w, e) = (g.check(4, 4), g.check(4, 6), g.check(3, 5), g.check(5, 5));
        assert!(nn && !s && !w && !e);
        assert_eq!(STRIP[0].tiles[..3], [28, 32, 36]);
        assert_eq!(COVE[0].tiles[..3], [7, 11, 15]);
    }
}
