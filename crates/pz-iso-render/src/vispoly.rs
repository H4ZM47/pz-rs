//! `VisibilityPolygon2`: the vision cone's darkening, drawn on the camera
//! character's level after its shadows (`performRenderTiles` calls
//! `renderMain` when `z == fastfloor(camCharacterZ)`).
//!
//! The game uses the new visibility (`DebugOptions.useNewVisibility`,
//! `Visibility.UseNew`, on by default): `calculateVisibilityPolygonNew`
//! collects shadow triangles (the area outside the view cone, and a
//! 70-square shadow behind each wall and solid square in the cone) and
//! `renderNew` draws them into the half-size blur target with
//! `visPolygon`, then darkens the world through `visibilityBlur`. The old
//! path (`calculateVisibilityPolygonOld` / `renderOld`, Clipper polygons
//! and five offset steps) is not used and is not ported.
//!
//! Gameplay inputs come in [`VisionFrame`]; the walls and solid squares of
//! each chunk level come in [`ChunkLevelData`], built from per-square
//! line-of-sight flags by [`ChunkLevelData::recreate`].

use pz_render::iso::{self, fastfloor};
use pz_render::{DrawCmd, VisibilityPolygonDraw};

use crate::cell::ChunkRenderer;
use crate::frame::FrameSize;
use crate::Camera;

/// `Drawer.downscale`: the blur target is the screen size over this.
pub const DOWNSCALE: u32 = 2;
/// `Drawer.shadowBlurRamp`.
pub const SHADOW_BLUR_RAMP: f32 = 0.015;
/// `Drawer.circleRadius` (only with `useCircle`).
pub const CIRCLE_RADIUS: f32 = 40.0;

/// `VisibilityPolygon2.VisibilityWall`: a run of north (horizontal) or
/// west (vertical) vision-blocking edges, in squares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibilityWall {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

impl VisibilityWall {
    pub fn is_horizontal(&self) -> bool {
        self.y1 == self.y2
    }
}

/// What `ChunkLevelData.recreate` reads from one `IsoGridSquare`.
/// Owner: the world (pz-world's square properties and line of sight).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisSquare {
    /// `isSolid()`.
    pub solid: bool,
    /// `has(IsoObjectType.tree)`.
    pub tree: bool,
    /// `getTree()` whose properties have `blocksight`.
    pub tree_blocks_sight: bool,
    /// `testVisionAdjacent(0, -1, 0, false, false) == Blocked`.
    pub vision_blocked_n: bool,
    /// `testVisionAdjacent(-1, 0, 0, false, false) == Blocked`.
    pub vision_blocked_w: bool,
}

/// `VisibilityPolygon2.ChunkLevelData` for one chunk level.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChunkLevelData {
    pub all_walls: Vec<VisibilityWall>,
    /// The solid squares (and squares with a sight-blocking tree).
    pub solid_squares: Vec<(i32, i32)>,
}

impl ChunkLevelData {
    fn has_wall(sq: Option<&VisSquare>, north: bool) -> bool {
        let Some(s) = sq else { return false };
        if s.solid || s.tree {
            return false;
        }
        if north {
            s.vision_blocked_n
        } else {
            s.vision_blocked_w
        }
    }

    /// `recreate()` for chunk `(wx, wy)` at `level` (empty unless the
    /// chunk has that level): the chunk's squares `[x + y * 8]`.
    pub fn recreate(
        wx: i32,
        wy: i32,
        level: i32,
        min_level: i32,
        max_level: i32,
        squares: &[Option<VisSquare>; 64],
    ) -> Self {
        let mut d = ChunkLevelData::default();
        if level < min_level || level > max_level {
            return d;
        }
        for y in 0..8 {
            let mut wall: Option<(i32, i32)> = None;
            for x in 0..8 {
                let sq = squares[(x + y * 8) as usize].as_ref();
                let (sx, sy) = (wx * 8 + x, wy * 8 + y);
                if Self::has_wall(sq, true) {
                    if wall.is_none() {
                        wall = Some((sx, sy));
                    }
                } else if let Some((x1, y1)) = wall.take() {
                    d.all_walls.push(VisibilityWall {
                        x1,
                        y1,
                        x2: wx * 8 + x,
                        y2: wy * 8 + y,
                    });
                }
                let Some(s) = sq else { continue };
                if s.tree {
                    if s.tree_blocks_sight {
                        d.solid_squares.push((sx, sy));
                    }
                    continue;
                }
                if s.solid {
                    d.solid_squares.push((sx, sy));
                }
            }
            if let Some((x1, y1)) = wall {
                d.all_walls.push(VisibilityWall {
                    x1,
                    y1,
                    x2: wx * 8 + 8,
                    y2: wy * 8 + y,
                });
            }
        }
        for x in 0..8 {
            let mut wall: Option<(i32, i32)> = None;
            for y in 0..8 {
                let sq = squares[(x + y * 8) as usize].as_ref();
                if Self::has_wall(sq, false) {
                    if wall.is_none() {
                        wall = Some((wx * 8 + x, wy * 8 + y));
                    }
                    continue;
                }
                if let Some((x1, y1)) = wall.take() {
                    d.all_walls.push(VisibilityWall {
                        x1,
                        y1,
                        x2: wx * 8 + x,
                        y2: wy * 8 + y,
                    });
                }
            }
            if let Some((x1, y1)) = wall {
                d.all_walls.push(VisibilityWall {
                    x1,
                    y1,
                    x2: wx * 8 + x,
                    y2: wy * 8 + 8,
                });
            }
        }
        d
    }
}

/// One frame's vision-cone inputs, for the player whose view this is.
#[derive(Clone, Debug, PartialEq)]
pub struct VisionFrame {
    /// `LightingJNI.calculateVisionCone(player)` (owner: pz-lighting /
    /// the player's stats and traits).
    pub vision_cone: f32,
    /// `player.getLookAngleRadians()` (owner: the player).
    pub look_angle: f32,
    /// `player.getLookVector()` (owner: the player).
    pub look_vector: [f32; 2],
    /// The driver of a vehicle reversing faster than 1 km/h, not aiming
    /// or looking around (`vehicle.isDriver(player) &&
    /// getCurrentSpeedKmHour() < -1`): the cone faces backwards. Owner:
    /// the player and pz-vehicles.
    pub reversing: bool,
    /// `player.getCurrentSquare() != null` (owner: the player).
    pub on_square: bool,
    /// The player's square holds a tree whose properties have
    /// `blocksight` (owner: the world).
    pub inside_tree: bool,
    /// The chunk map's `getWorldXMinTiles`, `getWorldYMinTiles`,
    /// `getWorldXMaxTiles`, `getWorldYMaxTiles` (owner: the cell's chunk
    /// map).
    pub chunk_map: [i32; 4],
    /// `ChunkLevelData` at the camera character's level of each chunk in
    /// `chunkMap.getChunks()` order that is loaded, lit for this player,
    /// and on screen at that level (`isOnScreen(pz)` and
    /// `IsOnScreen(false)`). Owner: the world (rebuilt with
    /// [`ChunkLevelData::recreate`] when a neighbour loads).
    pub chunks: Vec<ChunkLevelData>,
    /// `VisibilityPolygon2.useCircle` (false in the game).
    pub use_circle: bool,
    /// `PerformanceSettings.viewConeOpacity` (0..5, default 3; owner: the
    /// options).
    pub view_cone_opacity: i32,
    /// `IsoPlayer.numPlayers` and this player's index, for the split-screen
    /// rectangle (owner: the game state).
    pub num_players: i32,
    pub player_index: i32,
}

impl Default for VisionFrame {
    fn default() -> Self {
        VisionFrame {
            vision_cone: 0.0,
            look_angle: 0.0,
            look_vector: [1.0, 0.0],
            reversing: false,
            on_square: true,
            inside_tree: false,
            chunk_map: [0; 4],
            chunks: Vec::new(),
            use_circle: false,
            view_cone_opacity: 3,
            num_players: 1,
            player_index: 0,
        }
    }
}

/// `Vector2.getDirection`: `atan2` wrapped to `-PI..PI`.
pub fn direction(x: f32, y: f32) -> f32 {
    let angle = (y as f64).atan2(x as f64) as f32;
    wrap(
        angle,
        -std::f64::consts::PI as f32,
        std::f64::consts::PI as f32,
    )
}

/// `PZMath.wrap(val, min, max)`.
fn wrap(val: f32, min: f32, max: f32) -> f32 {
    let real_max = if max > min { max } else { min };
    let real_min = if max < min { max } else { min };
    let range = real_max - real_min;
    let rel = val - real_min;
    let frac = |v: f32| v - fastfloor(v) as f32;
    let w = if range <= 0.0 {
        0.0
    } else if rel < 0.0 {
        (1.0 - frac(-rel / range)) * range
    } else {
        frac(rel / range) * range
    };
    real_min + w
}

/// `IsoUtils.DistanceToSquared(x1, y1, x2, y2)`.
fn distance_sq(fx: f32, fy: f32, tx: f32, ty: f32) -> f32 {
    let (dx, dy) = ((tx - fx) as f64, (ty - fy) as f64);
    (dx * dx + dy * dy) as f32
}

/// `Drawer.intersectLineSegments`.
#[allow(clippy::too_many_arguments)]
fn intersect_line_segments(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    x4: f32,
    y4: f32,
) -> Option<[f32; 2]> {
    let d = (y4 - y3) * (x2 - x1) - (x4 - x3) * (y2 - y1);
    if d == 0.0 {
        return None;
    }
    let yd = y1 - y3;
    let xd = x1 - x3;
    let ua = ((x4 - x3) * yd - (y4 - y3) * xd) / d;
    if ua < 0.0 || ua > 1.0 {
        return None;
    }
    let ub = ((x2 - x1) * yd - (y2 - y1) * xd) / d;
    if ub < 0.0 || ub > 1.0 {
        return None;
    }
    Some([x1 + (x2 - x1) * ua, y1 + (y2 - y1) * ua])
}

/// `Drawer.intersectLineSegmentWithAABBEdge` into the static `edge`, which
/// keeps its last value when nothing intersects.
#[allow(clippy::too_many_arguments)]
fn intersect_aabb_edge(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    rx1: f32,
    ry1: f32,
    rx2: f32,
    ry2: f32,
    edge: &mut [f32; 2],
) {
    let hit = intersect_line_segments(x1, y1, x2, y2, rx1, ry1, rx2, ry1)
        .or_else(|| intersect_line_segments(x1, y1, x2, y2, rx2, ry1, rx2, ry2))
        .or_else(|| intersect_line_segments(x1, y1, x2, y2, rx2, ry2, rx1, ry2))
        .or_else(|| intersect_line_segments(x1, y1, x2, y2, rx1, ry2, rx1, ry1));
    if let Some(p) = hit {
        *edge = p;
    }
}

/// The drawer state `calculateVisibilityPolygonNew` works with.
struct Calc<'a> {
    v: &'a VisionFrame,
    px: f32,
    py: f32,
    vision_cone: f32,
    look_angle: f32,
    look: [f32; 2],
    /// `dirX1..dirY2`: only the old path sets them, so the new path tests
    /// against zero directions (which never intersect).
    dirs: [f32; 4],
    verts: Vec<f32>,
}

impl Calc<'_> {
    /// `getDotWithLookVector` (`Vector2.normalize` then `dot`).
    fn dot_look(&self, x: f32, y: f32) -> f32 {
        let (mut vx, mut vy) = (x - self.px, y - self.py);
        let len_sq = vx * vx + vy * vy;
        if (1.0 - len_sq).abs() < 1.0e-5 {
            // PZMath.equal(lengthSq, 1): left as it is.
        } else if len_sq == 0.0 {
            vx = 0.0;
            vy = 0.0;
        } else {
            let len = (len_sq as f64).sqrt() as f32;
            vx /= len;
            vy /= len;
        }
        vx * self.look[0] + vy * self.look[1]
    }

    /// `lineSegmentsIntersects`.
    #[allow(clippy::too_many_arguments)]
    fn line_segments_intersects(
        &self,
        sx: f32,
        sy: f32,
        dir_x: f32,
        dir_y: f32,
        ax: f32,
        ay: f32,
        bx: f32,
        by: f32,
    ) -> bool {
        let dba_x = bx - ax;
        let doa_y = sy - ay;
        let dba_y = by - ay;
        let doa_x = sx - ax;
        let inv = 1.0 / (dba_y * dir_x - dba_x * dir_y);
        let t = (dba_x * doa_y - dba_y * doa_x) * inv;
        if !(0.0..=1500.0).contains(&t) {
            return false;
        }
        let t2 = (doa_y * dir_x - doa_x * dir_y) * inv;
        (0.0..=1.0).contains(&t2)
    }

    /// `isInViewCone(x1, y1, x2, y2)`.
    fn in_view_cone(&self, x1: f32, y1: f32, x2: f32, y2: f32) -> bool {
        let r2 = CIRCLE_RADIUS * CIRCLE_RADIUS;
        if self.v.use_circle
            && (distance_sq(x1, y1, self.px, self.py) >= r2
                || distance_sq(x2, y2, self.px, self.py) >= r2)
        {
            return false;
        }
        let d1 = self.dot_look(x1, y1);
        let d2 = self.dot_look(x2, y2);
        let d3 = self.dot_look(x1 + (x2 - x1) * 0.5, y1 + (y2 - y1) * 0.5);
        // Float.compare(dot, -visionCone) >= 0: NaN sorts above all, -0
        // below 0.
        let limit = -self.vision_cone;
        let ge = |d: f32| d.is_nan() || d.total_cmp(&limit).is_ge();
        if ge(d1) || ge(d2) || ge(d3) {
            return true;
        }
        let [dx1, dy1, dx2, dy2] = self.dirs;
        self.line_segments_intersects(self.px, self.py, dx1, dy1, x1, y1, x2, y2)
            || self.line_segments_intersects(self.px, self.py, dx2, dy2, x1, y1, x2, y2)
    }

    /// `isInViewCone(square)`.
    fn square_in_view_cone(&self, x: i32, y: i32) -> bool {
        let (x0, y0, x1, y1) = (x as f32, y as f32, (x + 1) as f32, (y + 1) as f32);
        self.in_view_cone(x0, y0, x1, y0)
            || self.in_view_cone(x1, y0, x1, y1)
            || self.in_view_cone(x1, y1, x0, y1)
            || self.in_view_cone(x0, y1, x0, y0)
    }

    fn push(&mut self, x: f32, y: f32, b: f32) {
        self.verts.extend([x, y, b]);
    }

    /// `getLen(px, py, x, y)`.
    fn len(&self, x: f32, y: f32) -> f32 {
        let (dx, dy) = (x - self.px, y - self.py);
        ((dx * dx + dy * dy) as f64).sqrt() as f32
    }

    /// `addFullScreenShadow`.
    fn add_full_screen_shadow(&mut self) {
        let [x0, y0, x1, y1] = self.v.chunk_map.map(|v| v as f32);
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y0), (x1, y1), (x0, y1)] {
            self.push(x, y, 0.0);
        }
    }

    /// `addViewShadow`: the chunk map outside the view cone, as a fan from
    /// the player.
    fn add_view_shadow(&mut self, edge: &mut [f32; 2]) {
        let [x_min, y_min, x_max, y_max] = self.v.chunk_map;
        let (fx0, fy0, fx1, fy1) = (x_min as f32, y_min as f32, x_max as f32, y_max as f32);
        let offset = ((-self.vision_cone) as f64).acos() as f32;
        let dir_l = self.look_angle - offset;
        let dir_r = self.look_angle + offset;
        let (axl, ayl) = ((dir_l as f64).cos() as f32, (dir_l as f64).sin() as f32);
        let (axr, ayr) = ((dir_r as f64).cos() as f32, (dir_r as f64).sin() as f32);
        let (px, py) = (self.px, self.py);
        let (pxl, pyl) = (px + axl * 1500.0, py + ayl * 1500.0);
        let (pxr, pyr) = (px + axr * 1500.0, py + ayr * 1500.0);
        intersect_aabb_edge(px, py, pxr, pyr, fx0, fy0, fx1, fy1, edge);
        let [pxr, pyr] = *edge;
        intersect_aabb_edge(px, py, pxl, pyl, fx0, fy0, fx1, fy1, edge);
        let [pxl, pyl] = *edge;
        let angle_l = direction(pxl - px, pyl - py);
        let angle_r = direction(pxr - px, pyr - py);
        let corners = [
            (fx0, fy0, direction(fx0 - px, fy0 - py)),
            (fx1, fy0, direction(fx1 - px, fy0 - py)),
            (fx1, fy1, direction(fx1 - px, fy1 - py)),
            (fx0, fy1, direction(fx0 - px, fy1 - py)),
        ];
        self.push(px, py, 0.0);
        let l = self.len(pxr, pyr) * SHADOW_BLUR_RAMP;
        self.push(pxr, pyr, l);
        let corner = |c: &mut Self, (x, y, _): (f32, f32, f32)| {
            let len = c.len(x, y) * SHADOW_BLUR_RAMP;
            c.push(x, y, len);
            c.push(px, py, 0.0);
            c.push(x, y, len);
        };
        if angle_l > angle_r {
            for c in corners {
                if c.2 > angle_r && c.2 < angle_l {
                    corner(self, c);
                }
            }
        } else {
            for c in corners {
                if c.2 > angle_r {
                    corner(self, c);
                }
            }
            for c in corners {
                if c.2 < angle_l {
                    corner(self, c);
                }
            }
        }
        let l = self.len(pxl, pyl) * SHADOW_BLUR_RAMP;
        self.push(pxl, pyl, l);
    }

    /// `addLineShadow`: the segment and a quad 70 squares away from the
    /// player behind it.
    fn add_line_shadow(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        let (x1d, y1d, x2d, y2d) = (x1 - self.px, y1 - self.py, x2 - self.px, y2 - self.py);
        let sqr1 = x1d * x1d + y1d * y1d;
        let mag1 = (0.01f32.max(sqr1) as f64).sqrt() as f32;
        let sqr2 = x2d * x2d + y2d * y2d;
        let mag2 = (0.01f32.max(sqr2) as f64).sqrt() as f32;
        let (x1n, y1n, x2n, y2n) = (x1d / mag1, y1d / mag1, x2d / mag2, y2d / mag2);
        let len = 70.0 * SHADOW_BLUR_RAMP;
        self.push(x1, y1, 0.0);
        self.push(x2, y2, 0.0);
        self.push(x1 + x1n * 70.0, y1 + y1n * 70.0, len);
        self.push(x2, y2, 0.0);
        self.push(x1 + x1n * 70.0, y1 + y1n * 70.0, len);
        self.push(x2 + x2n * 70.0, y2 + y2n * 70.0, len);
    }

    /// `addSquareShadow`: the two faces of a solid square facing away from
    /// the player.
    fn add_square_shadow(&mut self, x: i32, y: i32) {
        let (fx, fy, fx1, fy1) = (x as f32, y as f32, (x + 1) as f32, (y + 1) as f32);
        if fx < self.px {
            self.add_line_shadow(fx1, fy, fx1, fy1);
        } else {
            self.add_line_shadow(fx, fy, fx, fy1);
        }
        if fy < self.py {
            self.add_line_shadow(fx, fy1, fx1, fy1);
        } else {
            self.add_line_shadow(fx, fy, fx1, fy);
        }
    }
}

/// `calculateVisibilityPolygonNew`: the shadow triangles (`shadowVerts`,
/// `x, y, aDist` per vertex) for the camera character at `cam`.
pub fn shadow_verts(cam: &Camera, v: &VisionFrame) -> Vec<f32> {
    let mut edge = [0.0f32; 2];
    shadow_verts_with_edge(cam, v, &mut edge)
}

/// [`shadow_verts`] with the drawer's static `edge` (the last ray end
/// found on the chunk map's border) carried in and out.
pub fn shadow_verts_with_edge(cam: &Camera, v: &VisionFrame, edge: &mut [f32; 2]) -> Vec<f32> {
    let mut look_angle = v.look_angle;
    let mut look = v.look_vector;
    if v.reversing {
        look_angle += std::f32::consts::PI;
        // Vector2.rotate(PI), in doubles.
        let pi = std::f32::consts::PI as f64;
        let (c, s) = (pi.cos(), pi.sin());
        let (x, y) = (look[0] as f64, look[1] as f64);
        look = [(x * c - y * s) as f32, (x * s + y * c) as f32];
    }
    let mut c = Calc {
        v,
        px: cam.x,
        py: cam.y,
        vision_cone: v.vision_cone,
        look_angle,
        look,
        dirs: [0.0; 4],
        verts: Vec::new(),
    };
    if !v.on_square {
        return c.verts;
    }
    if v.inside_tree {
        c.add_full_screen_shadow();
        return c.verts;
    }
    c.add_view_shadow(edge);
    let [x_min, y_min, x_max, y_max] = v.chunk_map;
    for chunk in &v.chunks {
        for w in &chunk.all_walls {
            let at_edge = if !w.is_horizontal() {
                w.x1 == x_min || w.x1 == x_max
            } else {
                w.y1 == y_min || w.y1 == y_max
            };
            if at_edge || !c.in_view_cone(w.x1 as f32, w.y1 as f32, w.x2 as f32, w.y2 as f32) {
                continue;
            }
            c.add_line_shadow(w.x1 as f32, w.y1 as f32, w.x2 as f32, w.y2 as f32);
        }
        for &(x, y) in &chunk.solid_squares {
            if c.square_in_view_cone(x, y) {
                c.add_square_shadow(x, y);
            }
        }
    }
    c.verts
}

/// `PolygonData.addVertex` over `shadowVerts`: each vertex in screen
/// pixels (`PlayerCamera.XToScreenExact` at the camera character's level)
/// with its blur ramp and its square depth less `1e-4`.
pub fn polygon_vertices(cam: &Camera, verts: &[f32]) -> Vec<[f32; 5]> {
    let (sx, sy) = (fastfloor(cam.x), fastfloor(cam.y));
    let z = fastfloor(cam.z) as f32;
    verts
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&[x, y, b]| {
            let cx = iso::x_to_screen(x, y) - cam.off_x as f32;
            let cy = iso::y_to_screen(x, y, z, 0) - cam.off_y as f32;
            let d = iso::square_depth(sx, sy, x, y, z);
            [cx, cy, 0.0, b, d + -1.0e-4]
        })
        .collect()
}

/// `VisibilityPolygon2.renderMain(playerIndex)`: the shadow triangles and
/// the blur pass for a screen of `size`.
pub fn render_main(
    r: &ChunkRenderer<'_>,
    v: &VisionFrame,
    size: FrameSize,
    out: &mut Vec<DrawCmd>,
) {
    out.push(draw(&r.cam, v, size));
}

/// The [`DrawCmd::VisibilityPolygon`] `renderNew` draws.
pub fn draw(cam: &Camera, v: &VisionFrame, size: FrameSize) -> DrawCmd {
    let verts = shadow_verts(cam, v);
    let (w, h) = (size.width as i32, size.height as i32);
    let p = v.player_index;
    let left = if p == 1 || p == 3 { w / 2 } else { 0 };
    let top = if p == 2 || p == 3 { h / 2 } else { 0 };
    let sw = if v.num_players > 1 { w / 2 } else { w };
    let sh = if v.num_players > 2 { h / 2 } else { h };
    let (ow, oh) = size.offscreen();
    let div_w = if v.num_players > 1 { 2 } else { 1 };
    let div_h = if v.num_players > 2 { 2 } else { 1 };
    DrawCmd::VisibilityPolygon(Box::new(VisibilityPolygonDraw {
        vertices: polygon_vertices(cam, &verts),
        blur_size: [size.width / DOWNSCALE, size.height / DOWNSCALE],
        screen: [left, top, sw, sh],
        display_size: [(ow as i32 / div_w) as f32, (oh as i32 / div_h) as f32],
        zoom: cam.zoom,
        opacity_scale: v.view_cone_opacity,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> VisionFrame {
        VisionFrame {
            vision_cone: -0.2,
            look_angle: 0.0,
            look_vector: [1.0, 0.0],
            chunk_map: [0, 0, 64, 64],
            ..Default::default()
        }
    }

    #[test]
    fn walls_run_along_north_and_west_edges() {
        let mut squares = [None; 64];
        for s in squares.iter_mut() {
            *s = Some(VisSquare::default());
        }
        // A north wall on squares (2..5, 3) and a west wall on (6, 0..2).
        for x in 2..5 {
            squares[x + 3 * 8].as_mut().unwrap().vision_blocked_n = true;
        }
        for y in 0..2 {
            squares[6 + y * 8].as_mut().unwrap().vision_blocked_w = true;
        }
        squares[7 + 7 * 8].as_mut().unwrap().solid = true;
        let d = ChunkLevelData::recreate(1, 2, 0, 0, 0, &squares);
        assert_eq!(
            d.all_walls,
            vec![
                VisibilityWall {
                    x1: 10,
                    y1: 19,
                    x2: 13,
                    y2: 19
                },
                VisibilityWall {
                    x1: 14,
                    y1: 16,
                    x2: 14,
                    y2: 18
                }
            ]
        );
        assert_eq!(d.solid_squares, vec![(15, 23)]);
        assert_eq!(
            ChunkLevelData::recreate(1, 2, 1, 0, 0, &squares),
            ChunkLevelData::default()
        );
    }

    #[test]
    fn the_view_shadow_fans_from_the_player_to_the_chunk_map() {
        let cam = Camera::player(32.5, 32.5, 0.0, 1280, 720, 1.0);
        let v = shadow_verts(&cam, &frame());
        // Player, right ray end, (corner, player, corner) for each corner
        // behind, left ray end; looking east two corners are behind.
        assert_eq!(v.len(), (2 + 2 * 3 + 1) * 3);
        assert_eq!(&v[0..3], &[32.5, 32.5, 0.0]);
        // Inside a tree the whole chunk map is shadowed.
        let t = shadow_verts(
            &cam,
            &VisionFrame {
                inside_tree: true,
                ..frame()
            },
        );
        assert_eq!(
            t,
            vec![
                0.0, 0.0, 0.0, 64.0, 0.0, 0.0, 64.0, 64.0, 0.0, 0.0, 0.0, 0.0, 64.0, 64.0, 0.0,
                0.0, 64.0, 0.0
            ]
        );
    }

    #[test]
    fn walls_in_the_cone_cast_shadows() {
        let cam = Camera::player(32.5, 32.5, 0.0, 1280, 720, 1.0);
        let wall = VisibilityWall {
            x1: 36,
            y1: 30,
            x2: 36,
            y2: 35,
        };
        let behind = VisibilityWall {
            x1: 28,
            y1: 31,
            x2: 28,
            y2: 34,
        };
        let at_edge = VisibilityWall {
            x1: 64,
            y1: 30,
            x2: 64,
            y2: 35,
        };
        let v = VisionFrame {
            chunks: vec![ChunkLevelData {
                all_walls: vec![wall, behind, at_edge],
                solid_squares: vec![(40, 32)],
            }],
            ..frame()
        };
        let base = shadow_verts(&cam, &frame()).len();
        let all = shadow_verts(&cam, &v);
        // One wall quad (6 vertices) and two square faces (12).
        assert_eq!(all.len() - base, 18 * 3);
        assert_eq!(&all[base..base + 6], &[36.0, 30.0, 0.0, 36.0, 35.0, 0.0]);
        let q = polygon_vertices(&cam, &all[base..base + 3]);
        assert_eq!(q[0][0], iso::x_to_screen(36.0, 30.0) - cam.off_x as f32);
        assert_eq!(q[0][4], iso::square_depth(32, 32, 36.0, 30.0, 0.0) - 1.0e-4);
    }

    #[test]
    fn draw_takes_the_screen_and_blur_sizes() {
        let cam = Camera::player(32.5, 32.5, 0.0, 1280, 720, 1.5);
        let DrawCmd::VisibilityPolygon(d) = draw(
            &cam,
            &frame(),
            FrameSize {
                width: 1280,
                height: 720,
            },
        ) else {
            panic!()
        };
        assert_eq!(d.blur_size, [640, 360]);
        assert_eq!(d.screen, [0, 0, 1280, 720]);
        assert_eq!(d.display_size, [2048.0, 1024.0]);
        assert_eq!(d.zoom, 1.5);
        assert_eq!(d.opacity_scale, 3);
    }
}

/// The per-chunk-level `ChunkLevelData` the game keeps on each `IsoChunk`
/// (`getVispolyDataForLevel`), with its rebuild rule: a level is recreated
/// when its `adjacentChunkLoadedCounter` differs from the chunk's, and
/// [`invalidate`](Self::invalidate) forces that (`ChunkLevelData.invalidate`
/// sets it to -1 and bumps `VisibilityPolygon2.dirtyObstacleCounter`).
#[derive(Clone, Debug, Default)]
pub struct VisPolyChunks {
    levels: std::collections::HashMap<(i32, i32, i32), (i64, ChunkLevelData)>,
    /// `VisibilityPolygon2.dirtyObstacleCounter`.
    pub dirty_obstacle_counter: i32,
}

impl VisPolyChunks {
    /// `IsoChunk.invalidateVispoly(level)` / `ChunkLevelData.invalidate()`
    /// for chunk `(wx, wy)`. Drain pz-world-render's `PendingRender.vispoly`
    /// into this.
    pub fn invalidate(&mut self, wx: i32, wy: i32, level: i32) {
        if let Some(e) = self.levels.get_mut(&(wx, wy, level)) {
            e.0 = -1;
        }
        self.dirty_obstacle_counter += 1;
    }

    /// `VisibilityPolygon2.addChunkToWorld`.
    pub fn add_chunk_to_world(&mut self) {
        self.dirty_obstacle_counter += 1;
    }

    /// Drops a chunk's levels when it unloads.
    pub fn remove_chunk(&mut self, wx: i32, wy: i32) {
        self.levels.retain(|k, _| (k.0, k.1) != (wx, wy));
    }

    /// The level's data, recreated with `build` (normally
    /// [`ChunkLevelData::recreate`]) when it is new, invalidated, or the
    /// chunk's `adjacentChunkLoadedCounter` has moved.
    pub fn level(
        &mut self,
        wx: i32,
        wy: i32,
        level: i32,
        adjacent_chunk_loaded_counter: i64,
        build: impl FnOnce() -> ChunkLevelData,
    ) -> &ChunkLevelData {
        let e = self
            .levels
            .entry((wx, wy, level))
            .or_insert_with(|| (i64::MIN, ChunkLevelData::default()));
        if e.0 != adjacent_chunk_loaded_counter {
            e.0 = adjacent_chunk_loaded_counter;
            e.1 = build();
        }
        &e.1
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn invalidate_forces_a_recreate() {
        let mut c = VisPolyChunks::default();
        let mut builds = 0;
        let get = |c: &mut VisPolyChunks, n: i64, builds: &mut i32| {
            c.level(1, 2, 0, n, || {
                *builds += 1;
                ChunkLevelData::default()
            });
        };
        get(&mut c, 3, &mut builds);
        get(&mut c, 3, &mut builds);
        c.invalidate(1, 2, 0);
        get(&mut c, 3, &mut builds);
        get(&mut c, 4, &mut builds);
        assert_eq!(builds, 3);
        assert_eq!(c.dirty_obstacle_counter, 1);
    }
}
