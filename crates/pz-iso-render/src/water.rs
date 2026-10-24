//! `IsoWaterGeometry` (a square's water depth per corner, shore detection),
//! `IsoWater` (its quads and the water shader's uniforms) and
//! `FBORenderCell.renderWater` / `renderWaterShore`, which draw open water
//! after the chunk textures and shores in the per-level translucent pass.
//!
//! The floor rules that depend on water (`isObjectRenderLayer_Floor`,
//! `isObjectRenderLayer_TranslucentFloor`, the shore fade in `renderFloor`)
//! live in [`crate::cell`].

use pz_render::iso::{square_depth, x_to_screen, y_to_screen};
use pz_render::{gl, DrawCmd, SurfaceDraw, SurfaceProgram, SurfaceUniforms, SurfaceVertex};

use crate::alpha::CutawayView;
use crate::cell::{ChunkRenderer, Layer};
use crate::color::ColorInfo;
use crate::sprite::SpriteCtx;
use crate::weather::Surfaces;
use crate::world::RenderSquare;

/// `IsoWaterGeometry`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaterGeometry {
    /// `IsoWater.getShaderEnable()` (water quality below 2) when the square
    /// was filled; the floor rules read it as the original reads the global.
    pub shader: bool,
    pub has_water: bool,
    pub shore: bool,
    /// Corners 0-3: `(x, y)`, `(x, y+1)`, `(x+1, y+1)`, `(x+1, y)`.
    pub depth: [f32; 4],
    /// `IsoWaterFlow.getFlow` at the corners: flow angle and speed.
    pub flow: [f32; 4],
    pub speed: [f32; 4],
    pub is_external: f32,
    /// Whether the west and south neighbours have the `water` flag (a
    /// shore's quad is pushed 0.02 under them).
    pub w_water: bool,
    pub s_water: bool,
}

/// What `IsoWaterGeometry.init` reads from a square and its neighbours.
#[derive(Clone, Copy, Debug, Default)]
pub struct WaterInit<'a> {
    /// The square's properties have `water`.
    pub water: bool,
    pub exterior: bool,
    /// `IsoWaterFlow.getShore(x, y)` (1 outside every water zone;
    /// [`crate::water_flow::IsoWaterFlow::get_shore`]).
    pub shore_zone: i32,
    /// `getFloor().getSprite().getName()`.
    pub floor_sprite: Option<&'a str>,
    /// The neighbours' `water` flag, `None` for a missing square, in the
    /// order W, NW, N, SW, S, SE, E, NE.
    pub neighbours: [Option<bool>; 8],
    /// `IsoWaterFlow.getFlow(square, ax, ay)` at corners (0,0), (0,1),
    /// (1,1), (1,0): (flow, speed)
    /// ([`crate::water_flow::IsoWaterFlow::corner_flows`]).
    pub flow: [(f32, f32); 4],
    pub shader: bool,
}

impl WaterGeometry {
    /// `IsoWaterGeometry.init`; `None` where the original returns null (a
    /// shore candidate with a neighbour missing).
    pub fn init(w: &WaterInit<'_>) -> Option<Self> {
        let mut g = WaterGeometry { shader: w.shader, is_external: if w.exterior { 1.0 } else { 0.0 }, ..Default::default() };
        if w.water {
            g.has_water = true;
            g.depth = [1.0; 4];
        } else if w.shore_zone == 1 && w.floor_sprite.is_some_and(|n| n.starts_with("blends_natural")) {
            let n = w.neighbours;
            let [gs01, gs00, gs10, gs02, gs12, gs22, gs21, gs20] = [n[0]?, n[1]?, n[2]?, n[3]?, n[4]?, n[5]?, n[6]?, n[7]?];
            let corners = [gs01 || gs00 || gs10, gs01 || gs02 || gs12, gs12 || gs22 || gs21, gs21 || gs20 || gs10];
            for (i, c) in corners.into_iter().enumerate() {
                if c {
                    g.shore = true;
                    g.depth[i] = 1.0;
                }
            }
        }
        g.w_water = w.neighbours[0] == Some(true);
        g.s_water = w.neighbours[4] == Some(true);
        for i in 0..4 {
            g.flow[i] = w.flow[i].0;
            g.speed[i] = w.flow[i].1;
        }
        Some(g)
    }

    /// `isValid()`.
    pub fn is_valid(&self) -> bool {
        self.has_water || self.shore
    }

    /// `initRender`: the quad's world screen positions.
    pub fn positions(&self, x: i32, y: i32, z: i32) -> ([f32; 4], [f32; 4]) {
        let (mut dx0, mut dy1) = (0.0, 0.0);
        if self.shore {
            if self.w_water {
                dx0 = -0.02;
            }
            if self.s_water {
                dy1 = 0.02;
            }
        }
        let (x, y, z) = (x as f32, y as f32, z as f32);
        let c = [(x + dx0, y), (x + dx0, y + 1.0 + dy1), (x + 1.0, y + 1.0 + dy1), (x + 1.0, y)];
        (c.map(|(a, b)| x_to_screen(a, b)), c.map(|(a, b)| y_to_screen(a, b, z, 0)))
    }
}

/// `IsoWaterGeometry.hideWaterObjects`: the shore blends drawn by the water
/// shader instead (`blends_natural_02_0` … `_12`). The world crate sets
/// [`crate::world::SpriteInfo::hide_for_water_render`] on these while the
/// water shader is on.
pub fn hides_for_water_render(sprite_name: &str) -> bool {
    let Some(rest) = sprite_name.strip_prefix("blends_natural_02") else { return false };
    (0..=12).any(|i| rest.ends_with(&format!("_{i}")))
}

/// `square.z == 0 ? square.getWater() : null`, as the floor rules read it.
pub fn ground_water(sq: &RenderSquare) -> Option<&WaterGeometry> {
    if sq.z == 0 {
        sq.water.as_ref()
    } else {
        None
    }
}

/// The water program for the quality, `None` with the shader off.
fn program(s: &Surfaces<'_>) -> Option<SurfaceProgram> {
    match s.settings.water_quality {
        0 => Some(SurfaceProgram::WaterHq),
        1 => Some(SurfaceProgram::Water),
        _ => None,
    }
}

/// `IsoWater.RenderData.addSquare`.
fn add_square(r: &ChunkRenderer<'_>, sq: &RenderSquare, w: &WaterGeometry) -> [SurfaceVertex; 4] {
    let cam = &r.cam;
    let (jx, jy) = (cam.fix_jiggly_x * cam.zoom, cam.fix_jiggly_y * cam.zoom);
    let (xs, ys) = w.positions(sq.x, sq.y, sq.z);
    let (cx, cy) = (pz_render::iso::fastfloor(cam.x), pz_render::iso::fastfloor(cam.y));
    std::array::from_fn(|i| {
        let dx = if i == 1 || i == 2 { 1.0 } else { 0.0 } + cam.fix_jiggly_square_x;
        let dy = if i == 2 || i == 3 { 1.0 } else { 0.0 } + cam.fix_jiggly_square_y;
        SurfaceVertex {
            attr: [w.depth[i], w.flow[i], w.speed[i], w.is_external],
            // The water projection is the camera's: world positions less
            // the view offset under the zoomed projection.
            pos: [xs[i] + jx - cam.off_x as f32, ys[i] + jy - cam.off_y as f32],
            color: sq.vert_lights[(4 - i) % 4],
            depth: square_depth(cx, cy, sq.x as f32 + dx, sq.y as f32 + dy, sq.z as f32) + 0.001,
        }
    })
}

/// `WaterShader.updateWaterParams`' uniforms.
fn uniforms(r: &ChunkRenderer<'_>, s: &Surfaces<'_>) -> SurfaceUniforms {
    let cam = &r.cam;
    let (w, h) = (r.view_size.0 as f32 * cam.zoom, r.view_size.1 as f32 * cam.zoom);
    let (jx, jy) = (-cam.fix_jiggly_x * cam.zoom, -cam.fix_jiggly_y * cam.zoom);
    SurfaceUniforms {
        time: s.water.time,
        offset: [cam.off_x as f32 + jx - 90000.0, cam.off_y as f32 + jy - 640000.0, w, h],
        viewport: [0.0, 0.0, w / cam.zoom, h / cam.zoom],
        reflection_param: s.sky.shift,
        params: [[s.water.wind[0], s.water.wind[1], s.water.wind_speed(), s.puddles.rain_intensity.get()], [0.0; 4]],
    }
}

/// `SpriteRenderer.drawWater`: the quads with the water shader, depth
/// `GL_LESS` without writes, `(SRC_ALPHA, ONE_MINUS_SRC_ALPHA)`.
fn draw(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, quads: Vec<[SurfaceVertex; 4]>, out: &mut Vec<DrawCmd>) {
    let Some(program) = program(s) else { return };
    if quads.is_empty() {
        return;
    }
    out.push(DrawCmd::Surface(Box::new(SurfaceDraw {
        program,
        quads,
        uniforms: uniforms(r, s),
        ground: s.textures.texture("media/textures/river_bottom.png"),
        reflection: [s.sky.current.clone(), s.sky.prev.clone()],
        depth_func: gl::LESS,
        blend_func: [gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA],
    })));
}

/// Whether `renderOneLevel` caches `sq` at all (visible, with objects).
fn classified(cv: Option<&CutawayView<'_>>, sq: &RenderSquare) -> bool {
    !sq.objects.is_empty() && cv.is_none_or(|cv| cv.is_visible(sq))
}

/// `bHasAttachedSpritesOnWater`'s objects: no render layer, a `water`
/// sprite and attached sprites.
fn has_water_attached(cv: Option<&CutawayView<'_>>, sq: &RenderSquare) -> bool {
    sq.objects.iter().any(|o| crate::alpha::render_layer(cv, sq, o) == Layer::None && o.flags().water && !o.attached.is_empty())
}

/// The attached and overlay sprites of the squares' objects with no render
/// layer, drawn in the per-frame pass (`renderAttachedAndOverlaySprites`
/// with `renderTranslucentOnly`).
fn render_water_attached(r: &ChunkRenderer<'_>, squares: &[&RenderSquare], out: &mut Vec<DrawCmd>) {
    let cv = r.cutaways.as_ref();
    let mut ctx = SpriteCtx { cam: &r.cam, chunk: None, depth_maps: r.depth_maps, out };
    for sq in squares {
        for obj in sq.objects.iter().filter(|o| crate::alpha::render_layer(cv, sq, o) == Layer::None && !o.attached.is_empty()) {
            let col: ColorInfo = sq.light_info;
            crate::cell::render_attached_and_overlay(&mut ctx, sq, obj, col);
        }
    }
}

/// `FBORenderCell.renderWater`: per level from 0, the open water of every
/// on-screen square, then the attached sprites of water objects.
pub fn render_water(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, squares: &[&RenderSquare], out: &mut Vec<DrawCmd>) {
    if r.cam.z < 0.0 {
        return;
    }
    let cv = r.cutaways.as_ref();
    let shader = s.settings.water_shader();
    let max_z = squares.iter().map(|s| s.z).max().unwrap_or(0);
    for z in 0..=max_z {
        let at: Vec<&RenderSquare> = squares.iter().copied().filter(|sq| sq.z == z && classified(cv, sq)).collect();
        let mut quads = Vec::new();
        for sq in &at {
            let Some(w) = sq.water.as_ref() else { continue };
            // getCachedSquares_Water: squares with water.
            if !w.has_water {
                continue;
            }
            if let Some(f) = sq.floor() {
                if crate::alpha::render_layer(cv, sq, &sq.objects[f]) == Layer::TranslucentFloor {
                    continue;
                }
            }
            // IsoWater.render: open water only.
            if shader && w.is_valid() && !w.shore && w.has_water {
                quads.push(add_square(r, sq, w));
            }
        }
        draw(r, s, quads, out);
        if shader {
            let attach: Vec<&RenderSquare> = at.iter().copied().filter(|sq| sq.water.is_some_and(|w| w.is_valid()) && has_water_attached(cv, sq)).collect();
            render_water_attached(r, &attach, out);
        }
    }
}

/// `FBORenderCell.renderWaterShore` (called in the level-0 translucent
/// pass): the shore squares of every level.
pub fn render_water_shore(r: &ChunkRenderer<'_>, s: &Surfaces<'_>, squares: &[&RenderSquare], out: &mut Vec<DrawCmd>) {
    if !s.settings.water_shader() || r.cam.z < 0.0 {
        return;
    }
    let cv = r.cutaways.as_ref();
    let max_z = squares.iter().map(|s| s.z).max().unwrap_or(0);
    for z in 0..=max_z {
        let at: Vec<&RenderSquare> = squares.iter().copied().filter(|sq| sq.z == z && classified(cv, sq) && sq.water.is_some_and(|w| w.shore)).collect();
        let quads = at.iter().map(|sq| add_square(r, sq, sq.water.as_ref().expect("water"))).collect();
        draw(r, s, quads, out);
        render_water_attached(r, &at, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init(water: bool, neighbours: [Option<bool>; 8]) -> Option<WaterGeometry> {
        WaterGeometry::init(&WaterInit { water, exterior: true, shore_zone: 1, floor_sprite: Some("blends_natural_01_0"), neighbours, flow: [(0.0, 0.0); 4], shader: true })
    }

    #[test]
    fn shores_take_depth_from_the_water_around_each_corner() {
        let w = init(true, [None; 8]).unwrap();
        assert!(w.has_water && !w.shore && w.depth == [1.0; 4]);
        // Water to the west only: corners 0 (x, y) and 1 (x, y+1).
        let mut n = [Some(false); 8];
        n[0] = Some(true);
        let s = init(false, n).unwrap();
        assert!(s.shore && !s.has_water);
        assert_eq!(s.depth, [1.0, 1.0, 0.0, 0.0]);
        let (xs, _) = s.positions(10, 10, 0);
        assert_eq!(xs[0], x_to_screen(10.0 - 0.02, 10.0));
        // A missing neighbour: no geometry.
        assert!(init(false, [None; 8]).is_none());
        // Dry land away from water.
        let d = init(false, [Some(false); 8]).unwrap();
        assert!(!d.is_valid());
        assert!(hides_for_water_render("blends_natural_02_12"));
        assert!(!hides_for_water_render("blends_natural_02_13"));
    }
}
