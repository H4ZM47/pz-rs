//! `IsoPuddlesGeometry` (which way a square's puddles run, and puddles
//! leaking into rooms) and `IsoPuddles`' drawing: per frame for puddle
//! quality 0 and 1 (`FBORenderCell.renderPuddles`), into the chunk
//! textures for quality 2 (`renderPuddlesToChunkTexture`), and over
//! translucent floors in the per-level pass
//! (`renderPuddlesTranslucentFloorsOnly`).

use pz_render::iso::{chunk_depth_data, coordmodulof, fastfloor, square_depth, x_to_screen, y_to_screen, LEVEL_DEPTH, PIXELS_PER_LEVEL};
use pz_render::{gl, DrawCmd, SurfaceDraw, SurfaceProgram, SurfaceUniforms, SurfaceVertex};

use crate::alpha::CutawayView;
use crate::cell::{ChunkRenderer, Layer};
use crate::chunk::{extra_height_for_jumbo_trees, ChunkTexture, FLOOR_HEIGHT};
use crate::weather::Surfaces;
use crate::world::RenderSquare;

/// `IsoPuddlesGeometry`'s per-corner direction weights (corners as
/// [`crate::water::WaterGeometry`]'s).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PuddlesGeometry {
    pub pdne: [f32; 4],
    pub pdnw: [f32; 4],
    pub pda: [f32; 4],
    /// `pnon`: 1 where no puddle may form (the shader's `1 - dirNone`).
    pub pnon: [f32; 4],
    /// `shouldRender()`.
    pub render: bool,
}

/// What `IsoPuddlesGeometry` reads from a square.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PuddleSquare {
    pub exterior: bool,
    pub water: bool,
    pub collide_w: bool,
    pub collide_n: bool,
    /// `hasSlopedSurface()`.
    pub sloped: bool,
    /// `getPuddlesDir()` ([`puddles_dir`]).
    pub puddles_dir: i32,
}

/// `IsoGridSquare.getPuddlesDir`: 1 in a room, 4 or 2 along traffic lines
/// (by the attached sprites' names), else 8.
pub fn puddles_dir<'a>(in_room: bool, attached_names: impl IntoIterator<Item = &'a str>) -> i32 {
    if in_room {
        return 1;
    }
    let mut dir = 8;
    for n in attached_names {
        if matches!(n, "street_trafficlines_01_2" | "street_trafficlines_01_6" | "street_trafficlines_01_22" | "street_trafficlines_01_32") {
            dir = 4;
        }
        if matches!(n, "street_trafficlines_01_4" | "street_trafficlines_01_0" | "street_trafficlines_01_16") {
            dir = 2;
        }
    }
    dir
}

impl PuddlesGeometry {
    /// `IsoPuddlesGeometry.init` followed by `shouldRender` (which fills
    /// in puddles leaking into rooms when `Core.getPerfPuddles()` is 0).
    /// `at(dx, dy)` is the square at that offset on the same level.
    pub fn init(sq: &PuddleSquare, perf_puddles: i32, at: impl Fn(i32, i32) -> Option<PuddleSquare>) -> Self {
        let mut g = PuddlesGeometry::default();
        if !(sq.water || !sq.exterior || sq.sloped) {
            g.pda = [1.0; 4];
            if perf_puddles <= 1 {
                let n = [at(-1, 0), at(-1, -1), at(0, -1), at(-1, 1), at(0, 1), at(1, 1), at(1, 0), at(1, -1)];
                if let [Some(gs01), Some(gs00), Some(gs10), Some(gs02), Some(gs12), Some(gs22), Some(gs21), Some(gs20)] = n {
                    g.set_flags(0, gs01.puddles_dir | gs00.puddles_dir | gs10.puddles_dir);
                    g.set_flags(1, gs01.puddles_dir | gs02.puddles_dir | gs12.puddles_dir);
                    g.set_flags(2, gs12.puddles_dir | gs22.puddles_dir | gs21.puddles_dir);
                    g.set_flags(3, gs21.puddles_dir | gs20.puddles_dir | gs10.puddles_dir);
                }
            }
        }
        g.render = g.should_render(sq, perf_puddles == 0, &at);
        g
    }

    fn set_flags(&mut self, n: usize, flags: i32) {
        self.pdne[n] = if flags & 2 != 0 { 1.0 } else { 0.0 };
        self.pdnw[n] = if flags & 4 != 0 { 1.0 } else { 0.0 };
        self.pda[n] = if flags & 8 != 0 { 1.0 } else { 0.0 };
        self.pnon[n] = 0.0;
    }

    fn any(&self) -> bool {
        (0..4).any(|i| self.pdne[i] + self.pdnw[i] + self.pda[i] + self.pnon[i] > 0.0)
    }

    /// `shouldRender`, with the interior calculation for puddles leaking
    /// through open edges into rooms.
    fn should_render(&mut self, sq: &PuddleSquare, leaking: bool, at: &impl Fn(i32, i32) -> Option<PuddleSquare>) -> bool {
        if self.any() {
            return true;
        }
        if sq.water || !leaking {
            return false;
        }
        self.pdne = [0.0; 4];
        self.pdnw = [0.0; 4];
        self.pda = [0.0; 4];
        self.pnon = [1.0; 4];
        let n = [at(-1, 0), at(-1, -1), at(0, -1), at(-1, 1), at(0, 1), at(1, 1), at(1, 0), at(1, -1)];
        let [Some(w), Some(nw), Some(nn), Some(sw), Some(s), Some(se), Some(e), Some(ne)] = n else { return false };
        if !w.exterior && !nn.exterior && !s.exterior && !e.exterior {
            return false;
        }
        let open = |g: &mut Self, corners: &[usize], v: f32| {
            for &c in corners {
                g.pnon[c] = v;
            }
            g.pda = [1.0; 4];
        };
        if !sq.collide_w && w.exterior {
            open(self, &[0, 1], 0.0);
        }
        if !s.collide_n && s.exterior {
            open(self, &[1, 2], 0.0);
        }
        if !e.collide_w && e.exterior {
            open(self, &[2, 3], 0.0);
        }
        if !sq.collide_n && nn.exterior {
            open(self, &[3, 0], 0.0);
        }
        if nn.collide_w || !nw.exterior {
            open(self, &[0], 1.0);
        }
        if s.collide_w || !sw.exterior {
            open(self, &[1], 1.0);
        }
        if sw.collide_n || !sw.exterior {
            open(self, &[1], 1.0);
        }
        if se.collide_n || !se.exterior {
            open(self, &[2], 1.0);
        }
        if se.collide_w || !se.exterior {
            open(self, &[2], 1.0);
        }
        if ne.collide_w || !ne.exterior {
            open(self, &[3], 1.0);
        }
        if e.collide_n || !ne.exterior {
            open(self, &[3], 1.0);
        }
        if w.collide_n || !nw.exterior {
            open(self, &[0], 1.0);
        }
        self.any()
    }

    /// The quad's positions (`init`): chunk-local for low-quality puddles
    /// drawn into a chunk texture, else world positions less a pixel.
    pub fn positions(sq: &RenderSquare, local: bool) -> ([f32; 4], [f32; 4]) {
        let z = sq.z as f32;
        let (bx, by) = if local { (coordmodulof(sq.x as f32, 8), coordmodulof(sq.y as f32, 8)) } else { (sq.x as f32, sq.y as f32) };
        let (x0, y0) = (bx - z * 3.0, by - z * 3.0);
        let c = [(x0, y0), (x0, y0 + 1.0), (x0 + 1.0, y0 + 1.0), (x0 + 1.0, y0)];
        let mut xs = c.map(|(a, b)| x_to_screen(a, b));
        let mut ys = [0.0; 4];
        ys[0] = y_to_screen(c[0].0, c[0].1, z, sq.z);
        for i in 1..4 {
            ys[i] = y_to_screen(c[i].0, c[i].1, 0.0, 0);
        }
        if !local {
            xs = xs.map(|x| x - 1.0);
        }
        (xs, ys)
    }
}

/// Whether low-quality puddles on `sq` are drawn chunk-local
/// (`IsoPuddlesGeometry.init`'s first branch).
fn local_positions(s: &Surfaces<'_>, sq: &RenderSquare) -> bool {
    let shore = s.settings.water_shader() && sq.water.is_some_and(|w| w.shore);
    let transparent = sq.floor().is_some_and(|f| sq.objects[f].flags().transparent_floor);
    s.settings.puddles_quality == 2 && !shore && !transparent
}

fn program(s: &Surfaces<'_>) -> SurfaceProgram {
    match s.settings.puddles_quality {
        0 => SurfaceProgram::PuddlesHq,
        1 => SurfaceProgram::PuddlesMq,
        _ => SurfaceProgram::PuddlesLq,
    }
}

/// The seam strip `renderToChunkTexture` adds along chunk edges.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seam {
    None,
    East,
    South,
}

/// `IsoPuddles.RenderData.addSquare`. `chunk` is the chunk texture being
/// recorded (`isCaching`).
fn add_square(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, sq: &RenderSquare, g: &PuddlesGeometry, chunk: Option<ChunkTexture>, seam: Seam) -> [SurfaceVertex; 4] {
    let cam = &r.cam;
    let (jx, jy) = (cam.fix_jiggly_x * cam.zoom, cam.fix_jiggly_y * cam.zoom);
    let (xs, ys) = PuddlesGeometry::positions(sq, local_positions(s, sq));
    let (cx, cy) = (fastfloor(cam.x), fastfloor(cam.y));
    // updateLighting.
    let v = &sq.vert_lights;
    let color = [v[0], v[3], v[2], v[1]];
    std::array::from_fn(|i| {
        let dx = if i == 2 || i == 3 { 1.0 } else { 0.0 } + cam.fix_jiggly_square_x;
        let dy = if i == 1 || i == 2 { 1.0 } else { 0.0 } + cam.fix_jiggly_square_y;
        let mut x = xs[i] + jx;
        let mut y = ys[i] + jy;
        let mut depth = square_depth(cx, cy, sq.x as f32 + dx, sq.y as f32 + dy, sq.z as f32) - 1.0e-4;
        match chunk {
            Some(t) => {
                depth -= chunk_depth_data(cam.chunk_x(), cam.chunk_y(), fastfloor(sq.x as f32 / 8.0), fastfloor(sq.y as f32 / 8.0), sq.z).depth_start;
                if sq.x.rem_euclid(8) == 0 && dx == 0.0 {
                    x -= 2.0;
                    y -= 1.0;
                }
                match seam {
                    Seam::South => {
                        depth -= LEVEL_DEPTH;
                        x -= if dy == 1.0 { 6.0 } else { 64.0 };
                        y += if dy == 1.0 { 3.0 } else { 32.0 };
                    }
                    Seam::East => {
                        depth -= LEVEL_DEPTH;
                        x += if dx == 1.0 { 6.0 } else { 64.0 };
                        y += if dx == 1.0 { 3.0 } else { 32.0 };
                    }
                    Seam::None => {}
                }
                // RenderToChunkTexture's projection and viewport put the
                // chunk-local positions where the chunk's sprites go.
                let (xoff, yoff) = t.record_offset();
                x += xoff;
                y += yoff;
            }
            None => {
                x -= cam.off_x as f32;
                y -= cam.off_y as f32;
            }
        }
        SurfaceVertex { attr: [g.pdne[i], g.pdnw[i], g.pda[i], g.pnon[i]], pos: [x, y], color: color[i], depth }
    })
}

/// `IsoPuddles.render`'s and `renderToChunkTexture`'s common checks.
fn can_render(s: &Surfaces<'_>, z: i32) -> bool {
    s.should_render_puddles() && !(z > 0 && s.settings.perf_puddles > 0)
}

/// The puddle squares' quads; `None` when nothing is drawn.
fn quads(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, squares: &[&RenderSquare], chunk: Option<ChunkTexture>) -> Vec<[SurfaceVertex; 4]> {
    let mut out = Vec::new();
    for sq in squares {
        let Some(g) = sq.puddles.as_ref().filter(|g| g.render) else { continue };
        out.push(add_square(r, s, sq, g, chunk, Seam::None));
        if chunk.is_some() {
            if sq.x.rem_euclid(8) == 7 {
                out.push(add_square(r, s, sq, g, chunk, Seam::East));
            }
            if sq.y.rem_euclid(8) == 7 {
                out.push(add_square(r, s, sq, g, chunk, Seam::South));
            }
        }
    }
    out
}

/// `IsoPuddles.render(squares, z)`: drawn per frame with the camera's
/// uniforms, depth `GL_LEQUAL`, `(SRC_ALPHA, ONE_MINUS_SRC_ALPHA)`.
fn render(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, squares: &[&RenderSquare], z: i32, out: &mut Vec<DrawCmd>) {
    if squares.is_empty() || !can_render(s, z) {
        return;
    }
    let q = quads(r, s, squares, None);
    if q.is_empty() {
        return;
    }
    let cam = &r.cam;
    let (w, h) = (r.view_size.0 as f32 * cam.zoom, r.view_size.1 as f32 * cam.zoom);
    let (jx, jy) = (-cam.fix_jiggly_x * cam.zoom, -cam.fix_jiggly_y * cam.zoom);
    let uniforms = SurfaceUniforms {
        time: s.puddles.time,
        offset: [cam.off_x as f32 + jx - 90000.0, cam.off_y as f32 + jy - 640000.0, w, h],
        viewport: [0.0, 0.0, w / cam.zoom, h / cam.zoom],
        reflection_param: s.sky.shift,
        params: s.puddles.params(z),
    };
    push(s, q, uniforms, [gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA], out);
}

fn push(s: &Surfaces<'_>, quads: Vec<[SurfaceVertex; 4]>, uniforms: SurfaceUniforms, blend_func: [u32; 4], out: &mut Vec<DrawCmd>) {
    out.push(DrawCmd::Surface(Box::new(SurfaceDraw {
        program: program(s),
        quads,
        uniforms,
        ground: s.textures.texture("media/textures/puddles_hm.png"),
        reflection: [s.sky.current.clone(), s.sky.prev.clone()],
        depth_func: gl::LEQUAL,
        blend_func,
    })));
}

fn visible(cv: Option<&CutawayView<'_>>, sq: &RenderSquare) -> bool {
    cv.is_none_or(|cv| cv.is_visible(sq))
}

fn floor_layer(cv: Option<&CutawayView<'_>>, sq: &RenderSquare) -> Option<Layer> {
    sq.floor().map(|f| crate::alpha::render_layer(cv, sq, &sq.objects[f]))
}

fn has_puddles(sq: &RenderSquare) -> bool {
    sq.puddles.is_some_and(|g| g.render)
}

/// `FBORenderCell.renderPuddles` (puddle quality 0 and 1): every
/// on-screen level's puddles after the chunk textures.
pub fn render_puddles(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, squares: &[&RenderSquare], out: &mut Vec<DrawCmd>) {
    if !s.should_render_puddles() || s.snow_anywhere() {
        return;
    }
    let cv = r.cutaways.as_ref();
    let mut max_z = squares.iter().map(|s| s.z).max().unwrap_or(0);
    if s.settings.perf_puddles > 0 {
        max_z = 0;
    }
    for z in 0..=max_z {
        let at: Vec<&RenderSquare> = squares
            .iter()
            .copied()
            .filter(|sq| sq.z == z && !sq.objects.is_empty() && visible(cv, sq) && has_puddles(sq))
            .filter(|sq| match floor_layer(cv, sq) {
                None => false,
                Some(l) => !(s.settings.puddles_quality < 2 && l == Layer::TranslucentFloor),
            })
            .collect();
        render(r, s, &at, z, out);
    }
}

/// `FBORenderCell.renderPuddlesTranslucentFloorsOnly`: puddles on level
/// `z`'s translucent floors (shores included), in the per-level pass.
pub fn render_puddles_translucent_floors(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, squares: &[&RenderSquare], z: i32, out: &mut Vec<DrawCmd>) {
    if !s.should_render_puddles() || (s.settings.perf_puddles > 0 && z != 0) {
        return;
    }
    let cv = r.cutaways.as_ref();
    let at: Vec<&RenderSquare> = squares.iter().copied().filter(|sq| sq.z == z && !sq.objects.is_empty() && visible(cv, sq) && has_puddles(sq) && floor_layer(cv, sq) == Some(Layer::TranslucentFloor)).collect();
    render(r, s, &at, z, out);
}

/// `FBORenderCell.renderPuddlesToChunkTexture` (puddle quality 2): level
/// `z`'s puddles recorded into chunk texture `tex` with
/// `RenderToChunkTexture`'s uniforms and `(SRC_ALPHA, ONE_MINUS_SRC_ALPHA,
/// ONE, ONE)`.
pub fn render_puddles_to_chunk(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, tex: ChunkTexture, squares: &[&RenderSquare], z: i32, out: &mut Vec<DrawCmd>) {
    if !s.should_render_puddles() || s.snow_anywhere() || z < tex.min_level || z > tex.top_level {
        return;
    }
    let cv = r.cutaways.as_ref();
    let at: Vec<&RenderSquare> = squares.iter().copied().filter(|sq| visible(cv, sq) && has_puddles(sq) && floor_layer(cv, sq).is_some_and(|l| l != Layer::TranslucentFloor)).collect();
    if at.is_empty() || !can_render(s, z) {
        return;
    }
    let q = quads(r, s, &at, Some(tex));
    if q.is_empty() {
        return;
    }
    let span = (FLOOR_HEIGHT * 8) as f32;
    let (bx, by) = ((tex.wx * 8) as f32, (tex.wy * 8) as f32);
    let (rcx, rcy) = (x_to_screen(bx, by), y_to_screen(bx, by, z as f32, 0));
    let num_levels = tex.top_level - tex.min_level + 1;
    let bottom = span + (num_levels * PIXELS_PER_LEVEL) as f32 + extra_height_for_jumbo_trees(tex.min_level, tex.top_level) as f32;
    let w = tex.width() as f32;
    let uniforms = SurfaceUniforms {
        time: s.puddles.time,
        offset: [rcx - 90000.0, -rcy - 640000.0, w, span],
        viewport: [0.0, bottom - span - ((z - tex.min_level) * PIXELS_PER_LEVEL) as f32, w, span],
        reflection_param: s.sky.shift,
        params: s.puddles.params(z),
    };
    push(s, q, uniforms, [gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::ONE, gl::ONE], out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext() -> PuddleSquare {
        PuddleSquare { exterior: true, puddles_dir: 8, ..Default::default() }
    }

    #[test]
    fn outdoor_puddles_take_the_neighbours_directions() {
        let g = PuddlesGeometry::init(&ext(), 0, |dx, dy| Some(PuddleSquare { puddles_dir: if (dx, dy) == (1, 0) { 2 } else { 8 }, ..ext() }));
        assert!(g.render);
        assert_eq!(g.pda, [1.0; 4]);
        // Corners 2 and 3 touch the east square's traffic line.
        assert_eq!(g.pdne, [0.0, 0.0, 1.0, 1.0]);
        // Water squares never get puddles.
        let w = PuddlesGeometry::init(&PuddleSquare { water: true, ..ext() }, 0, |_, _| Some(ext()));
        assert!(!w.render);
    }

    #[test]
    fn puddles_leak_into_rooms_through_open_edges() {
        let room = PuddleSquare { exterior: false, puddles_dir: 1, ..Default::default() };
        // Open to the west only (outdoors), everything else indoors.
        let g = PuddlesGeometry::init(&room, 0, |dx, dy| Some(if (dx, dy) == (-1, 0) { ext() } else { room }));
        assert!(g.render);
        assert_eq!(g.pda, [1.0; 4]);
        assert_eq!(g.pnon, [1.0, 1.0, 1.0, 1.0]);
        // Without leaking (Core.getPerfPuddles() > 0), no interior puddles.
        assert!(!PuddlesGeometry::init(&room, 1, |_, _| Some(ext())).render);
        assert_eq!(puddles_dir(false, ["street_trafficlines_01_0"]), 2);
    }
}
