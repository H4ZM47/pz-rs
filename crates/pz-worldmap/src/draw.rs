//! `WorldMapRenderer.Drawer`: one frame of the map, recorded as
//! [`Vbo`] runs in the map's scene space (see [`MapDraw`]).
//!
//! Ported: visible-cell rasterization, feature filtering per style layer,
//! polygon fills (plain, textured, per-zoom highway triangles), texture
//! layers (stretched and tiled), the cell grids, the visible-cells debug
//! view, the paint outside the bounds, the world bounds and drop shadow.
//! Image pyramids, symbols, streets, the visited overlay and the world
//! debug overlays (zones, zombie intensity, biome, water flow, player
//! models) are drawn by their own parts.

use crate::data::{GeometryType, WorldMapCell};
use crate::rasterize::scan_triangle;
use crate::renderer::{clamp, fast_floor, WorldMapRenderer};
use crate::style::{
    eval_color, eval_float, eval_texture_stop, LayerKind, LayerRef, RGBAf, TextureScaling,
    WorldMapStyle,
};
use crate::triangulate::{triangulate, DrawAbort, MapFilesInfo};
use crate::vbo::{Prim, RunTexture, Vbo};
use crate::world_map::WorldMap;
use pz_joml::Matrix4f;
use pz_render::Texture;
use std::collections::HashSet;
use std::rc::Rc;

/// A recorded frame: draw `vbo` with `projection * model_view` into the
/// viewport `x, y, width, height` (UI coordinates, top-left origin).
pub struct MapDraw {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub projection: Matrix4f,
    pub model_view: Matrix4f,
    pub model_view_projection: Matrix4f,
    /// `Wireframe`: `glPolygonMode(GL_LINE)` for the feature pass.
    pub wireframe: bool,
    pub vbo: Vbo,
    /// Set when the original's renderer threw part way (the rest of the
    /// frame is not drawn).
    pub aborted: Option<DrawAbort>,
}

/// Per-frame inputs besides the view.
pub struct DrawInputs<'a> {
    pub map: &'a WorldMap,
    pub style: &'a WorldMapStyle,
    /// `ui.color`: the background, painted outside the bounds.
    pub fill: [f32; 4],
    pub files: &'a dyn MapFilesInfo,
    /// `Core.debug`.
    pub debug: bool,
}

struct RenderLayer {
    cell_x: i32,
    cell_y: i32,
    /// (data index, cell, feature indices) in data order.
    features: Vec<(Rc<WorldMapCell>, usize)>,
}

/// `WorldMapRenderer.Drawer`.
pub struct Drawer<'a> {
    r: &'a WorldMapRenderer,
    inp: &'a DrawInputs<'a>,
    layers: Vec<LayerRef>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    center_world_x: f32,
    center_world_y: f32,
    pub zoom_f: f32,
    pub world_scale: f32,
    render_origin_x: f32,
    render_origin_y: f32,
    render_cell_x: f32,
    render_cell_y: f32,
    mvp: Matrix4f,
    triangulations_this_frame: i32,
    pub rasterize_xy: Vec<i32>,
    rasterize_mult: i32,
    render_layers_for_style_layer: Vec<Vec<RenderLayer>>,
    pub vbo: Vbo,
    files: &'a dyn MapFilesInfo,
}

const HIGHWAY_DELTAS: [f64; 6] = [1.0, 2.0, 4.0, 8.0, 12.0, 18.0];

impl<'a> Drawer<'a> {
    /// `init(renderer, ui)`.
    pub fn new(r: &'a WorldMapRenderer, inp: &'a DrawInputs<'a>) -> Self {
        let zoom_f = r.get_display_zoom_f();
        let world_scale = r.get_world_scale(zoom_f);
        let (cx, cy) = (r.get_center_world_x(), r.get_center_world_y());
        Drawer {
            r,
            inp,
            layers: inp.style.layers().to_vec(),
            x: r.get_absolute_x(),
            y: r.get_absolute_y(),
            width: r.get_width(),
            height: r.get_height(),
            center_world_x: cx,
            center_world_y: cy,
            zoom_f,
            world_scale,
            render_origin_x: (inp.map.get_min_x_in_squares() as f32 - cx) * world_scale,
            render_origin_y: (inp.map.get_min_y_in_squares() as f32 - cy) * world_scale,
            render_cell_x: 0.0,
            render_cell_y: 0.0,
            mvp: *r.get_model_view_projection_matrix(),
            triangulations_this_frame: 0,
            rasterize_xy: Vec::new(),
            rasterize_mult: 1,
            render_layers_for_style_layer: Vec::new(),
            vbo: Vbo::default(),
            files: inp.files,
        }
    }

    fn map(&self) -> &'a WorldMap {
        self.inp.map
    }
    fn opt(&self, name: &str) -> bool {
        self.r.get_boolean(name)
    }
    fn ui_to_world(&self, x: f32, y: f32) -> (f32, f32) {
        self.r.ui_to_world_mvp(
            x,
            y,
            self.zoom_f,
            self.center_world_x,
            self.center_world_y,
            &self.mvp,
        )
    }

    /// `render()` / `renderInternal()`.
    pub fn render(mut self) -> MapDraw {
        let aborted = self.render_internal().err();
        MapDraw {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            projection: *self.r.get_projection_matrix(),
            model_view: *self.r.get_model_view_matrix(),
            model_view_projection: self.mvp,
            wireframe: self.opt("Wireframe"),
            vbo: std::mem::take(&mut self.vbo),
            aborted,
        }
    }

    fn render_internal(&mut self) -> Result<(), DrawAbort> {
        let ws = self.world_scale;
        let m = self.map();
        let (min_x, min_y, max_x, max_y) = (
            m.get_min_x_in_squares(),
            m.get_min_y_in_squares(),
            m.get_max_x_in_squares(),
            m.get_max_y_in_squares(),
        );
        // TODO(worldmap pyramids): renderImagePyramids when ImagePyramid.
        self.calculate_visible_cells();
        if self.opt("Features") {
            self.filter_features();
            self.render_cell_features()?;
        } else {
            self.render_non_cell_features();
        }
        // TODO(worldmap): streets, symbols, and the world overlays
        // (OtherZones, ForagingZones, ZombieIntensity, ZombieVoronoi,
        // ZombieCutoff, BiomeOverlay, Animals, StoryZones, WaterFlow).
        if self.opt("VisibleCells") {
            self.render_visible_cells();
        }
        // TODO(worldmap visited): the fog of war; TODO(characters): player
        // models.
        if self.opt("CellGrid") {
            self.render_cell_grid(min_x / 256, min_y / 256, max_x / 256, max_y / 256, 256);
        }
        if self.opt("CellGrid300") {
            self.render_cell_grid(min_x / 300, min_y / 300, max_x / 300, max_y / 300, 300);
        }
        self.paint_areas_outside_bounds(min_x, min_y, max_x, max_y, ws);
        if self.opt("WorldBounds") {
            self.render_world_bounds();
        }
        Ok(())
    }

    /// `triangleArea` (Heron).
    fn triangle_area(x0: f32, y0: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
        let len = |x: f32, y: f32| ((x * x + y * y) as f64).sqrt() as f32;
        let a = len(x1 - x0, y1 - y0);
        let b = len(x2 - x1, y2 - y1);
        let c = len(x0 - x2, y0 - y2);
        let s = (a + b + c) / 2.0;
        ((s * (s - a) * (s - b) * (s - c)) as f64).sqrt() as f32
    }

    /// The view's corners in world units divided by `div` (the visible
    /// cells: 256), inset by `pad` pixels.
    fn corners(&self, pad: i32, div: f32, ox: f32, oy: f32) -> [f32; 8] {
        let (w, h) = (self.width, self.height);
        let p = pad as f32;
        let pts = [
            (p + 0.0, p + 0.0),
            ((w - pad) as f32, 0.0 + p),
            ((w - pad) as f32, (h - pad) as f32),
            (0.0 + p, (h - pad) as f32),
        ];
        let mut out = [0.0; 8];
        for (i, (x, y)) in pts.into_iter().enumerate() {
            let (wx, wy) = self.ui_to_world(x, y);
            out[i * 2] = (wx - ox) / div;
            out[i * 2 + 1] = (wy - oy) / div;
        }
        out
    }

    /// `calculateVisibleCells()`: the cells under the view (all of them
    /// when zoomed out past 100 squares per pixel).
    pub fn calculate_visible_cells(&mut self) {
        let pad = if self.inp.debug && self.opt("VisibleCells") {
            200
        } else {
            0
        };
        let m = self.map();
        self.rasterize_xy.clear();
        if 1.0 / self.world_scale > 100.0 {
            for y in m.get_min_y_in_cells()..=m.get_max_y_in_cells() {
                for x in m.get_min_x_in_cells()..=m.get_max_x_in_cells() {
                    self.rasterize_xy.push(x);
                    self.rasterize_xy.push(y);
                }
            }
            return;
        }
        let [x_tl, y_tl, x_tr, y_tr, x_br, y_br, x_bl, y_bl] = self.corners(pad, 256.0, 0.0, 0.0);
        let mut mult = 1;
        loop {
            let f = mult as f32;
            let a = Self::triangle_area(x_bl / f, y_bl / f, x_br / f, y_br / f, x_tr / f, y_tr / f)
                + Self::triangle_area(x_tr / f, y_tr / f, x_tl / f, y_tl / f, x_bl / f, y_bl / f);
            if a > 80.0 {
                mult += 1;
            } else {
                break;
            }
        }
        self.rasterize_mult = mult;
        let (wc, minx, maxx, miny, maxy) = (
            m.get_width_in_cells(),
            m.get_min_x_in_cells(),
            m.get_max_x_in_cells(),
            m.get_min_y_in_cells(),
            m.get_max_y_in_cells(),
        );
        let mut set = HashSet::new();
        let mut xy = Vec::new();
        let f = mult as f32;
        let mut cb = |sx: i32, sy: i32| {
            let id = sx.wrapping_add(sy.wrapping_mul(wc));
            if set.contains(&id) {
                return;
            }
            for y in sy * mult..sy * mult + mult {
                for x in sx * mult..sx * mult + mult {
                    if x < minx || x > maxx || y < miny || y > maxy {
                        continue;
                    }
                    set.insert(id);
                    xy.push(x);
                    xy.push(y);
                }
            }
        };
        scan_triangle(
            x_bl / f,
            y_bl / f,
            x_br / f,
            y_br / f,
            x_tr / f,
            y_tr / f,
            -1000,
            1000,
            &mut cb,
        );
        scan_triangle(
            x_tr / f,
            y_tr / f,
            x_tl / f,
            y_tl / f,
            x_bl / f,
            y_bl / f,
            -1000,
            1000,
            &mut cb,
        );
        self.rasterize_xy = xy;
    }

    /// `filterFeatures()`: per style layer, the features of each visible
    /// cell it draws.
    pub(crate) fn filter_features(&mut self) {
        self.render_layers_for_style_layer = (0..self.layers.len()).map(|_| Vec::new()).collect();
        let dirs = self.files.map_directory_names();
        for i in (0..self.rasterize_xy.len().saturating_sub(1)).step_by(2) {
            let (cx, cy) = (self.rasterize_xy[i], self.rasterize_xy[i + 1]);
            // TODO(worldmap visited): skip cells the visited overlay hides.
            let mut features: Vec<(Rc<WorldMapCell>, usize)> = Vec::new();
            for d in &self.map().data {
                if !d.is_ready() {
                    continue;
                }
                let Some(cell) = d.get_cell(cx, cy) else {
                    continue;
                };
                if cell.features.is_empty() {
                    continue;
                }
                if cell.priority.get() == -1 {
                    let rel = d.relative_file_name.borrow().clone();
                    if let Some(p) = dirs
                        .iter()
                        .position(|dir| rel.starts_with(&format!("media/maps/{dir}")))
                    {
                        cell.priority.set(p as i32);
                    }
                }
                for f in 0..cell.features.len() {
                    features.push((cell.clone(), f));
                }
            }
            if features.is_empty() {
                continue;
            }
            for (li, layer) in self.layers.iter().enumerate() {
                let l = layer.borrow();
                if l.ignore_features() || l.min_zoom > self.zoom_f {
                    continue;
                }
                let hits: Vec<_> = features
                    .iter()
                    .filter(|(c, f)| l.filter(&c.features[*f]))
                    .cloned()
                    .collect();
                if !hits.is_empty() {
                    self.render_layers_for_style_layer[li].push(RenderLayer {
                        cell_x: cx,
                        cell_y: cy,
                        features: hits,
                    });
                }
            }
        }
    }

    fn set_render_cell(&mut self, cx: i32, cy: i32) {
        let m = self.map();
        self.render_cell_x =
            self.render_origin_x + (cx * 256 - m.get_min_x_in_squares()) as f32 * self.world_scale;
        self.render_cell_y =
            self.render_origin_y + (cy * 256 - m.get_min_y_in_squares()) as f32 * self.world_scale;
    }

    /// `renderCellFeatures()`: each layer's `renderVisibleCells`.
    pub(crate) fn render_cell_features(&mut self) -> Result<(), DrawAbort> {
        for li in 0..self.layers.len() {
            let layer = self.layers[li].clone();
            if layer.borrow().min_zoom > self.zoom_f {
                continue;
            }
            let kind = layer.borrow().kind.clone();
            match kind {
                LayerKind::Line { .. } | LayerKind::Polygon { .. } => {
                    let rls = std::mem::take(&mut self.render_layers_for_style_layer[li]);
                    for rl in &rls {
                        self.set_render_cell(rl.cell_x, rl.cell_y);
                        for (cell, f) in &rl.features {
                            self.render_feature(&layer, rl.cell_x, rl.cell_y, cell, *f)?;
                        }
                    }
                    self.render_layers_for_style_layer[li] = rls;
                }
                LayerKind::Texture { .. } => self.render_texture_layer(&layer, None),
                // TODO(worldmap pyramids): the pyramid layer's
                // renderImagePyramid; text layers draw nothing here.
                LayerKind::Pyramid { .. } | LayerKind::Text { .. } => {}
            }
        }
        Ok(())
    }

    /// `renderNonCellFeatures()`: with `Features` off, only the texture
    /// (and pyramid) layers, cell by cell.
    fn render_non_cell_features(&mut self) {
        for i in (0..self.rasterize_xy.len().saturating_sub(1)).step_by(2) {
            let (cx, cy) = (self.rasterize_xy[i], self.rasterize_xy[i + 1]);
            self.set_render_cell(cx, cy);
            for layer in self.layers.clone() {
                let l = layer.borrow();
                if l.min_zoom > self.zoom_f {
                    continue;
                }
                let is_tex = matches!(l.kind, LayerKind::Texture { .. });
                drop(l);
                if is_tex {
                    self.render_texture_layer(&layer, Some((cx, cy)));
                }
            }
        }
    }

    /// `styleLayer.render(feature, args)`.
    fn render_feature(
        &mut self,
        layer: &LayerRef,
        cell_x: i32,
        cell_y: i32,
        cell: &WorldMapCell,
        f: usize,
    ) -> Result<(), DrawAbort> {
        let l = layer.borrow();
        match &l.kind {
            LayerKind::Polygon {
                fill,
                texture,
                scale,
            } => {
                let fill = eval_color(self.zoom_f, fill);
                if fill.a < 0.01 {
                    return Ok(());
                }
                let scale = eval_float(self.zoom_f, scale, 1.0);
                let stop = eval_texture_stop(self.zoom_f, texture);
                let tex = stop.and_then(|s| s.texture.clone());
                let scaling = stop.map_or(TextureScaling::IsoGridSquare, |s| s.scaling);
                drop(l);
                match tex.filter(|t| t.ready) {
                    None => self.fill_polygon(cell, f, fill),
                    Some(t) => self
                        .fill_polygon_textured(cell_x, cell_y, cell, f, fill, &t, scale, scaling),
                }
            }
            LayerKind::Line { fill, line_width } => {
                let fill = eval_color(self.zoom_f, fill);
                if fill.a < 0.01 {
                    return Ok(());
                }
                // drawLineString only flushes and sets GL state in the
                // original; nothing is drawn.
                let _ = line_width;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn ensure_triangles(
        &mut self,
        cell: &WorldMapCell,
        f: usize,
        delta: Option<&[f64]>,
    ) -> Result<bool, DrawAbort> {
        let g = cell.features[f].geometry.as_ref().unwrap();
        if g.triangles.borrow().as_ref().is_some_and(|t| t.failed) {
            return Ok(false);
        }
        if g.triangles.borrow().is_none() {
            if self.triangulations_this_frame > 500 {
                return Ok(false);
            }
            self.triangulations_this_frame += 1;
            triangulate(cell, g, delta, self.files)?;
            let mut t = g.triangles.borrow_mut();
            let t = t.as_mut().unwrap();
            if t.index_count <= 0 {
                t.failed = true;
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The triangle corners `first..first+count` of a cell's buffers.
    fn tri_corners(
        cell: &WorldMapCell,
        first: i32,
        count: i32,
    ) -> Result<Vec<[f32; 2]>, DrawAbort> {
        let ib = cell.index_buffer.borrow();
        let tb = cell.triangle_buffer.borrow();
        let mut out = Vec::new();
        let mut j = 0;
        while j < count {
            for k in 0..3 {
                let p = first + j + k;
                if p < 0 || p as usize >= ib.len() {
                    return Err(DrawAbort("IndexOutOfBoundsException in fillPolygon"));
                }
                let i = ib[p as usize] as u16 as usize;
                if i * 2 + 1 >= tb.len() {
                    return Err(DrawAbort("IndexOutOfBoundsException in fillPolygon"));
                }
                out.push([tb[i * 2], tb[i * 2 + 1]]);
            }
            j += 3;
        }
        Ok(out)
    }

    /// `fillPolygon(args, feature, color)`.
    fn fill_polygon(&mut self, cell: &WorldMapCell, f: usize, c: RGBAf) -> Result<(), DrawAbort> {
        let g = cell.features[f].geometry.as_ref().unwrap();
        if g.kind != GeometryType::Polygon {
            return Ok(());
        }
        let highway = cell.features[f].properties.contains_key("highway");
        if !self.ensure_triangles(cell, f, if highway { Some(&HIGHWAY_DELTAS) } else { None })? {
            return Ok(());
        }
        let (ox, oy, s) = (self.render_cell_x, self.render_cell_y, self.world_scale);
        let col = [c.r, c.g, c.b, c.a];
        let z = self.zoom_f as f64;
        let delta = if z <= 11.5 {
            18.0
        } else if z <= 12.0 {
            12.0
        } else if z <= 12.5 {
            8.0
        } else if z <= 13.0 {
            4.0
        } else if z <= 13.5 {
            2.0
        } else if z <= 14.0 {
            1.0
        } else {
            0.0
        };
        let (first, count) = {
            let t = g.triangles.borrow();
            let t = t.as_ref().unwrap();
            let tpz = if delta == 0.0 {
                None
            } else {
                t.per_zoom.iter().find(|p| p.0 == delta)
            };
            match tpz {
                Some(&(_, f, n)) => (f, n),
                None => (t.first_index, t.index_count),
            }
        };
        self.vbo.start_run(Prim::Triangles);
        let corners = Self::tri_corners(cell, first, count);
        let corners = match corners {
            Ok(c) => c,
            Err(e) => {
                self.vbo.end_run();
                return Err(e);
            }
        };
        for p in corners {
            self.vbo.element(ox + p[0] * s, oy + p[1] * s, 0.0, col);
        }
        self.vbo.end_run();
        Ok(())
    }

    /// `fillPolygon(args, feature, color, texture, scale, scaling)`.
    #[allow(clippy::too_many_arguments)]
    fn fill_polygon_textured(
        &mut self,
        cell_x: i32,
        cell_y: i32,
        cell: &WorldMapCell,
        f: usize,
        c: RGBAf,
        tex: &Texture,
        texture_scale: f32,
        scaling: TextureScaling,
    ) -> Result<(), DrawAbort> {
        let g = cell.features[f].geometry.as_ref().unwrap();
        if g.kind != GeometryType::Polygon {
            return Ok(());
        }
        if !self.ensure_triangles(cell, f, None)? {
            return Ok(());
        }
        let (ox, oy, ws) = (self.render_cell_x, self.render_cell_y, self.world_scale);
        let coord_scale = if scaling == TextureScaling::IsoGridSquare {
            1.0 / texture_scale
        } else {
            ws / texture_scale
        };
        let col = [c.r, c.g, c.b, c.a];
        let min_x = (cell_x * 256 + g.min_x) as f32;
        let min_y = (cell_y * 256 + g.min_y) as f32;
        let tex_w = tex.width as f32 * texture_scale;
        let tex_h = tex.height as f32 * texture_scale;
        let (whw, hhw) = (tex.width_hw as f32, tex.height_hw as f32);
        let origin_x = fast_floor(min_x / tex_w) as f32 * tex_w;
        let origin_y = fast_floor(min_y / tex_h) as f32 * tex_h;
        let (first, count) = {
            let t = g.triangles.borrow();
            let t = t.as_ref().unwrap();
            (t.first_index, t.index_count)
        };
        let corners = Self::tri_corners(cell, first, count)?;
        self.vbo.start_run(Prim::Triangles);
        self.vbo.set_texture(RunTexture::Texture(tex.clone()));
        self.vbo.set_nearest();
        for p in corners {
            let tx = (p[0] + (cell_x * 256) as f32 - origin_x) * coord_scale;
            let ty = (p[1] + (cell_y * 256) as f32 - origin_y) * coord_scale;
            self.vbo
                .element_uv(ox + p[0] * ws, oy + p[1] * ws, 0.0, tx / whw, ty / hhw, col);
        }
        self.vbo.end_run();
        Ok(())
    }

    /// A texture layer's `renderVisibleCells` (`cell` None) or
    /// `renderCell`.
    fn render_texture_layer(&mut self, layer: &LayerRef, cell: Option<(i32, i32)>) {
        let map = self.map();
        let mut l = layer.borrow_mut();
        let LayerKind::Texture {
            world_x1,
            world_y1,
            world_x2,
            world_y2,
            use_world_bounds,
            fill,
            texture,
            tile,
        } = &mut l.kind
        else {
            return;
        };
        if *use_world_bounds {
            *world_x1 = map.get_min_x_in_squares();
            *world_y1 = map.get_min_y_in_squares();
            *world_x2 = map.get_max_x_in_squares() + 1;
            *world_y2 = map.get_max_y_in_squares() + 1;
        }
        let c = eval_color(self.zoom_f, fill);
        if c.a < 0.01 {
            return;
        }
        let Some(tex) = eval_texture_stop(self.zoom_f, texture).and_then(|s| s.texture.clone())
        else {
            return;
        };
        let (x1, y1, x2, y2, tile) = (*world_x1, *world_y1, *world_x2, *world_y2, *tile);
        drop(l);
        match (tile, cell) {
            (true, Some((cx, cy))) => self.draw_texture_tiled_cell(&tex, c, x1, y1, x2, y2, cx, cy),
            (false, Some((cx, cy))) => self.draw_texture_cell(&tex, c, x1, y1, x2, y2, cx, cy),
            (true, None) => self.draw_texture_tiled(&tex, c, x1, y1, x2, y2),
            (false, None) => self.draw_texture(&tex, c, x1, y1, x2, y2),
        }
    }

    /// `drawTexture(texture, fill, x1, y1, x2, y2)`.
    pub fn draw_texture(&mut self, t: &Texture, c: RGBAf, wx1: i32, wy1: i32, wx2: i32, wy2: i32) {
        if !t.ready {
            return;
        }
        let ws = self.world_scale;
        let x1 = (wx1 as f32 - self.center_world_x) * ws;
        let y1 = (wy1 as f32 - self.center_world_y) * ws;
        let x2 = x1 + (wx2 - wx1) as f32 * ws;
        let y2 = y1 + (wy2 - wy1) as f32 * ws;
        self.vbo.start_run(Prim::Quads);
        self.vbo.set_texture(RunTexture::Texture(t.clone()));
        self.vbo.set_nearest();
        self.vbo.quad_uv(
            x1,
            y1,
            t.x_start,
            t.y_start,
            x2,
            y2,
            t.x_end,
            t.y_end,
            [c.r, c.g, c.b, c.a],
        );
        self.vbo.end_run();
    }

    /// `drawTexture(texture, fill, x1, y1, x2, y2, cellX, cellY)`. As in
    /// the original, the clamp compares UI offsets with the cell numbers.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_cell(
        &mut self,
        t: &Texture,
        c: RGBAf,
        wx1: i32,
        wy1: i32,
        wx2: i32,
        wy2: i32,
        cell_x: i32,
        cell_y: i32,
    ) {
        if !t.ready {
            return;
        }
        let ws = self.world_scale;
        let x1 = (wx1 as f32 - self.center_world_x) * ws;
        let y1 = (wy1 as f32 - self.center_world_y) * ws;
        let x2 = x1 + (wx2 - wx1) as f32 * ws;
        let y2 = y1 + (wy2 - wy1) as f32 * ws;
        let x1c = clamp(x1, cell_x as f32, cell_x as f32 + 256.0 * ws);
        let y1c = clamp(y1, cell_y as f32, cell_y as f32 + 256.0 * ws);
        let x2c = clamp(x2, cell_x as f32, cell_x as f32 + 256.0 * ws);
        let y2c = clamp(y2, cell_y as f32, cell_y as f32 + 256.0 * ws);
        if x1c >= x2c || y1c >= y2c {
            return;
        }
        let xs = t.width as f32 / (wx2 - wx1) as f32;
        let ys = t.height as f32 / (wy2 - wy1) as f32;
        let u0 = (x1c - x1) / (t.width_hw as f32 * ws) * xs;
        let v0 = (y1c - y1) / (t.height_hw as f32 * ws) * ys;
        let u1 = (x2c - x1) / (t.width_hw as f32 * ws) * xs;
        let v1 = (y2c - y1) / (t.height_hw as f32 * ws) * ys;
        self.vbo.start_run(Prim::Quads);
        self.vbo.set_texture(RunTexture::Texture(t.clone()));
        self.vbo.set_nearest();
        self.vbo
            .quad_uv(x1c, y1c, u0, v0, x2c, y2c, u1, v1, [c.r, c.g, c.b, c.a]);
        self.vbo.end_run();
    }

    #[allow(clippy::too_many_arguments)]
    fn tiled_quad(
        &mut self,
        t: &Texture,
        c: RGBAf,
        wx1: i32,
        wy1: i32,
        wx2: i32,
        wy2: i32,
        bounds: [f32; 4],
        xy: [i32; 4],
    ) {
        let (tw, th) = (t.width as i32, t.height as i32);
        let [x1, y1, x2, y2] = xy;
        let mut x1c = clamp(x1 as f32, bounds[0], bounds[2]);
        let mut y1c = clamp(y1 as f32, bounds[1], bounds[3]);
        let mut x2c = clamp(x2 as f32, bounds[0], bounds[2]);
        let mut y2c = clamp(y2 as f32, bounds[1], bounds[3]);
        x1c = clamp(x1c, wx1 as f32, wx2 as f32);
        y1c = clamp(y1c, wy1 as f32, wy2 as f32);
        x2c = clamp(x2c, wx1 as f32, wx2 as f32);
        y2c = clamp(y2c, wy1 as f32, wy2 as f32);
        let tx1 = (x1c - wx1 as f32) / tw as f32;
        let ty1 = (y1c - wy1 as f32) / th as f32;
        let tx2 = (x2c - wx1 as f32) / tw as f32;
        let ty2 = (y2c - wy1 as f32) / th as f32;
        let ws = self.world_scale;
        let (x1c, y1c) = (
            (x1c - self.center_world_x) * ws,
            (y1c - self.center_world_y) * ws,
        );
        let (x2c, y2c) = (
            (x2c - self.center_world_x) * ws,
            (y2c - self.center_world_y) * ws,
        );
        let u0 = tx1 * t.x_end;
        let v0 = ty1 * t.y_end;
        let u1 = (tx2 as i32) as f32 + (tx2 - (tx2 as i32) as f32) * t.x_end;
        let v1 = (ty2 as i32) as f32 + (ty2 - (ty2 as i32) as f32) * t.y_end;
        self.vbo.start_run(Prim::Quads);
        self.vbo.set_texture(RunTexture::Texture(t.clone()));
        self.vbo.set_nearest();
        self.vbo.set_repeat();
        self.vbo
            .quad_uv(x1c, y1c, u0, v0, x2c, y2c, u1, v1, [c.r, c.g, c.b, c.a]);
        self.vbo.end_run();
    }

    /// `drawTextureTiled(texture, fill, x1, y1, x2, y2, cellX, cellY)`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_texture_tiled_cell(
        &mut self,
        t: &Texture,
        c: RGBAf,
        wx1: i32,
        wy1: i32,
        wx2: i32,
        wy2: i32,
        cx: i32,
        cy: i32,
    ) {
        if !t.ready {
            return;
        }
        if cx * 256 >= wx2 || (cx + 1) * 256 <= wx1 || cy * 256 >= wy2 || (cy + 1) * 256 <= wy1 {
            return;
        }
        let (tw, th) = (t.width as i32, t.height as i32);
        let x1 = (fast_floor(cx as f32 * 256.0 / tw as f32) as f32 * tw as f32) as i32;
        let y1 = (fast_floor(cy as f32 * 256.0 / th as f32) as f32 * th as f32) as i32;
        let x2 =
            x1 + ((((cx + 1) as f32 * 256.0 - x1 as f32) / tw as f32) as f64).ceil() as i32 * tw;
        let y2 =
            y1 + ((((cy + 1) as f32 * 256.0 - y1 as f32) / th as f32) as f64).ceil() as i32 * th;
        let b = [
            (cx * 256) as f32,
            (cy * 256) as f32,
            ((cx + 1) * 256) as f32,
            ((cy + 1) * 256) as f32,
        ];
        self.tiled_quad(t, c, wx1, wy1, wx2, wy2, b, [x1, y1, x2, y2]);
    }

    /// `drawTextureTiled(texture, fill, x1, y1, x2, y2)`.
    pub fn draw_texture_tiled(
        &mut self,
        t: &Texture,
        c: RGBAf,
        wx1: i32,
        wy1: i32,
        wx2: i32,
        wy2: i32,
    ) {
        if !t.ready {
            return;
        }
        let (tw, th) = (t.width as i32, t.height as i32);
        let cx = fast_floor(wx1 as f32 / 256.0) as f32;
        let cy = fast_floor(wy1 as f32 / 256.0) as f32;
        let cx2 = fast_floor(wx2 as f32 / 256.0) as f32;
        let cy2 = fast_floor(wy2 as f32 / 256.0) as f32;
        let x1 = (fast_floor(cx * 256.0 / tw as f32) as f32 * tw as f32) as i32;
        let y1 = (fast_floor(cy * 256.0 / th as f32) as f32 * th as f32) as i32;
        let x2 = x1 + ((((cx2 + 1.0) * 256.0 - x1 as f32) / tw as f32) as f64).ceil() as i32 * tw;
        let y2 = y1 + ((((cy2 + 1.0) * 256.0 - y1 as f32) / th as f32) as f64).ceil() as i32 * th;
        let b = [
            cx * 256.0,
            cy * 256.0,
            (cx2 + 1.0) * 256.0,
            (cy2 + 1.0) * 256.0,
        ];
        self.tiled_quad(t, c, wx1, wy1, wx2, wy2, b, [x1, y1, x2, y2]);
    }

    /// `renderCellGrid(minX, minY, maxX, maxY, cellSizeInSquares)`.
    pub(crate) fn render_cell_grid(
        &mut self,
        min_x: i32,
        min_y: i32,
        max_x: i32,
        max_y: i32,
        size: i32,
    ) {
        let (ox, oy, ws) = (self.render_origin_x, self.render_origin_y, self.world_scale);
        let (mx, my) = (
            self.map().get_min_x_in_squares(),
            self.map().get_min_y_in_squares(),
        );
        let min_xui = ox + (min_x * size - mx) as f32 * ws;
        let min_yui = oy + (min_y * size - my) as f32 * ws;
        let max_xui = min_xui + ((max_x - min_x + 1) * size) as f32 * ws;
        let max_yui = min_yui + ((max_y - min_y + 1) * size) as f32 * ws;
        let c = [0.25, 0.25, 0.25, 1.0];
        self.vbo.start_run(Prim::Lines);
        self.vbo.set_line_width(1.0);
        for x in min_x..=max_x + 1 {
            let px = ox + (x * size - mx) as f32 * ws;
            self.vbo.line(px, min_yui, px, max_yui, c);
        }
        for y in min_y..=max_y + 1 {
            let py = oy + (y * size - my) as f32 * ws;
            self.vbo.line(min_xui, py, max_xui, py, c);
        }
        self.vbo.end_run();
    }

    /// `renderVisibleCells()`: the debug view of the visible cells.
    fn render_visible_cells(&mut self) {
        let pad = if self.inp.debug && self.opt("VisibleCells") {
            200
        } else {
            0
        };
        let ws = self.world_scale;
        if 1.0 / ws > 100.0 {
            return;
        }
        let (ox, oy) = (self.render_origin_x, self.render_origin_y);
        let (mx, my) = (
            self.map().get_min_x_in_squares() as f32,
            self.map().get_min_y_in_squares() as f32,
        );
        self.vbo.start_run(Prim::Triangles);
        for i in (0..self.rasterize_xy.len()).step_by(2) {
            let (x, y) = (self.rasterize_xy[i], self.rasterize_xy[i + 1]);
            let x0 = ox + ((x * 256) as f32 - mx) * ws;
            let y0 = oy + ((y * 256) as f32 - my) * ws;
            let x1 = ox + (((x + 1) * 256) as f32 - mx) * ws;
            let y1 = oy + (((y + 1) * 256) as f32 - my) * ws;
            let g = [0.0, 1.0, 0.0, 0.2];
            let b = [0.0, 0.0, 1.0, 0.2];
            self.vbo.triangle(x0, y0, x1, y0, x0, y1, g);
            self.vbo.triangle(x1, y0, x1, y1, x0, y1, b);
        }
        self.vbo.end_run();
        let [x_tl, y_tl, x_tr, y_tr, x_br, y_br, x_bl, y_bl] = self.corners(pad, 256.0, 0.0, 0.0);
        let p = |x: f32, y: f32| (ox + (x * 256.0 - mx) * ws, oy + (y * 256.0 - my) * ws);
        let (bl, br, tr, tl) = (p(x_bl, y_bl), p(x_br, y_br), p(x_tr, y_tr), p(x_tl, y_tl));
        self.vbo.start_run(Prim::Lines);
        self.vbo.set_line_width(4.0);
        self.vbo.line(bl.0, bl.1, br.0, br.1, [1.0, 0.0, 0.0, 1.0]);
        self.vbo.line(br.0, br.1, tr.0, tr.1, [1.0, 0.0, 0.0, 1.0]);
        self.vbo.line(tr.0, tr.1, bl.0, bl.1, [0.5, 0.5, 0.5, 1.0]);
        self.vbo.line(tr.0, tr.1, tl.0, tl.1, [0.0, 0.0, 1.0, 1.0]);
        self.vbo.line(tl.0, tl.1, bl.0, bl.1, [0.0, 0.0, 1.0, 1.0]);
        self.vbo.end_run();
    }

    /// `paintAreasOutsideBounds(...)`: the background over the parts of
    /// the edge cells outside the bounds.
    pub(crate) fn paint_areas_outside_bounds(
        &mut self,
        min_x: i32,
        min_y: i32,
        max_x: i32,
        max_y: i32,
        ws: f32,
    ) {
        let m = self.map();
        let (ox, oy) = (self.render_origin_x, self.render_origin_y);
        let x1cell = ox - (min_x % 256) as f32 * ws;
        let y1cell = oy - (min_y % 256) as f32 * ws;
        let x2cell = ox + ((m.get_max_x_in_cells() + 1) * 256 - min_x) as f32 * ws;
        let y2cell = oy + ((m.get_max_y_in_cells() + 1) * 256 - min_y) as f32 * ws;
        let (w, h) = (
            m.get_width_in_squares() as f32,
            m.get_height_in_squares() as f32,
        );
        let c = self.inp.fill;
        self.vbo.start_run(Prim::Quads);
        if min_x % 256 != 0 {
            self.vbo.quad(x1cell, y1cell, ox, y2cell, c);
        }
        if min_y % 256 != 0 {
            self.vbo.quad(ox, y1cell, ox + w * self.world_scale, oy, c);
        }
        if max_x + 1 != 0 {
            self.vbo.quad(
                ox + (max_x - min_x + 1) as f32 * ws,
                y1cell,
                x2cell,
                y2cell,
                c,
            );
        }
        if max_y + 1 != 0 {
            self.vbo.quad(ox, oy + h * ws, ox + w * ws, y2cell, c);
        }
        self.vbo.end_run();
    }

    /// `renderWorldBounds()`: the drop shadow and a grey outline.
    pub(crate) fn render_world_bounds(&mut self) {
        let (x1, y1) = (self.render_origin_x, self.render_origin_y);
        let x2 = x1 + self.map().get_width_in_squares() as f32 * self.world_scale;
        let y2 = y1 + self.map().get_height_in_squares() as f32 * self.world_scale;
        self.render_drop_shadow();
        let c = [0.5, 0.5, 0.5, 1.0];
        self.vbo.start_run(Prim::Lines);
        self.vbo.set_line_width(2.0);
        self.vbo.line(x1, y1, x2, y1, c);
        self.vbo.line(x2, y1, x2, y2, c);
        self.vbo.line(x2, y2, x1, y2, c);
        self.vbo.line(x1, y2, x1, y1, c);
        self.vbo.end_run();
    }

    /// `renderDropShadow()`.
    fn render_drop_shadow(&mut self) {
        let base = self.r.get_base_zoom(self.map());
        let th = self.r.drop_shadow_width as f32
            * (self.r.get_height() as f32 / 1080.0)
            * self.world_scale
            / self.r.get_world_scale(base);
        if th < 2.0 {
            return;
        }
        let (x1, y1) = (self.render_origin_x, self.render_origin_y);
        let x2 = x1 + self.map().get_width_in_squares() as f32 * self.world_scale;
        let y2 = y1 + self.map().get_height_in_squares() as f32 * self.world_scale;
        let (s, t) = ([0.5, 0.5, 0.5, 0.5], [0.5, 0.5, 0.5, 0.0]);
        self.vbo.start_run(Prim::Triangles);
        self.vbo.element(x1 + th, y2, 0.0, s);
        self.vbo.element(x2, y2, 0.0, s);
        self.vbo.element(x1 + th, y2 + th, 0.0, t);
        self.vbo.element(x2, y2, 0.0, s);
        self.vbo.element(x2 + th, y2 + th, 0.0, t);
        self.vbo.element(x1 + th, y2 + th, 0.0, t);
        self.vbo.element(x2, y1 + th, 0.0, s);
        self.vbo.element(x2 + th, y1 + th, 0.0, t);
        self.vbo.element(x2, y2, 0.0, s);
        self.vbo.element(x2 + th, y1 + th, 0.0, t);
        self.vbo.element(x2 + th, y2 + th, 0.0, t);
        self.vbo.element(x2, y2, 0.0, s);
        self.vbo.end_run();
    }
}
