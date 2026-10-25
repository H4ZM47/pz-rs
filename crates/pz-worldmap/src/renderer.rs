//! `zombie.worldMap.WorldMapRenderer`: the view (zoom, centre, the
//! orthographic or isometric camera), the UI <-> world transforms and the
//! renderer's boolean options.

// The original's literal angles (1.0471976, 0.7853982), not the consts.
#![allow(clippy::approx_constant)]

use crate::projection;
use crate::world_map::WorldMap;
use pz_core::config::{ConfigOption, OptionKind};
use pz_joml::{Matrix4f, Quaternionf};

/// `VIEW_CHANGE_TIME`: the isometric toggle's blend, in ms.
pub const VIEW_CHANGE_TIME: i64 = 175;
/// `transitionTo`'s duration, in ms.
pub const TRANSITION_TIME: i64 = 600;

/// The `WorldMapBooleanOption`s in declaration order (the order of
/// `getOptionByIndex`) with their defaults.
pub const BOOLEAN_OPTIONS: &[(&str, bool)] = &[
    ("AllPrintMedia", false),
    ("AllStashMaps", false),
    ("Animals", false),
    ("AnimalTracks", false),
    ("Basements", false),
    ("BlurUnvisited", true),
    ("BuildingsWithoutFeatures", false),
    ("CellGrid", false),
    ("CellGrid300", false),
    ("ClampBaseZoomToPoint5", true),
    ("ColorblindPatterns", false),
    ("DebugInfo", false),
    ("PlaceNames", true),
    ("TileGrid", false),
    ("UnvisitedGrid", true),
    ("Features", true),
    ("OtherZones", false),
    ("ForagingZones", false),
    ("ZombieIntensity", false),
    ("ZombieVoronoi", false),
    ("ZombieCutoff", false),
    ("BiomeOverlay", false),
    ("HideUnvisited", false),
    ("HitTest", false),
    ("HighlightStreet", true),
    ("LargeStreetLabel", true),
    ("OutlineStreets", false),
    ("ShowStreetNames", true),
    ("ImagePyramid", true),
    ("TerrainImage", false),
    ("Isometric", true),
    ("LineString", true),
    ("Players", false),
    ("PlayerModel", false),
    ("RemotePlayers", false),
    ("PlayerNames", false),
    ("Symbols", true),
    ("RemoteSymbols", true),
    ("Wireframe", false),
    ("WorldBounds", true),
    ("MiniMapSymbols", false),
    ("VisibleCells", false),
    ("VisibleTiles", false),
    ("DimUnsharedSymbols", false),
    ("StoryZones", false),
    ("WGRoads", false),
    ("InfiniteZoom", false),
    ("VirtualVehicles", false),
    ("WaterFlow", false),
];

/// The Java class of every renderer option.
pub const BOOLEAN_OPTION_CLASS: &str = "zombie.worldMap.WorldMapRenderer$WorldMapBooleanOption";

/// `zombie.worldMap.WorldMapRenderer` (view state; drawing lives in
/// [`crate::draw`]).
pub struct WorldMapRenderer {
    /// `worldMap != null`: `setMap` was called.
    has_map: bool,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    zoom: i32,
    zoom_f: f32,
    display_zoom_f: f32,
    center_world_x: f32,
    center_world_y: f32,
    zoom_ui_x: f32,
    zoom_ui_y: f32,
    zoom_world_x: f32,
    zoom_world_y: f32,
    transition_to: bool,
    transition_from_world_x: f32,
    transition_from_world_y: f32,
    transition_from_zoom_f: f32,
    transition_to_world_x: f32,
    transition_to_world_y: f32,
    transition_to_zoom_f: f32,
    transition_to_start_ms: i64,
    projection: Matrix4f,
    model_view: Matrix4f,
    model_view_projection: Matrix4f,
    model_view_change: Quaternionf,
    view_change_time: i64,
    is_isometric: bool,
    first_update: bool,
    pub drop_shadow_width: i32,
    max_zoom: f32,
    options: Vec<ConfigOption>,
}

impl Default for WorldMapRenderer {
    fn default() -> Self {
        Self::new()
    }
}

fn opt_index(name: &str) -> Option<usize> {
    BOOLEAN_OPTIONS.iter().position(|(n, _)| *n == name)
}

impl WorldMapRenderer {
    pub fn new() -> Self {
        WorldMapRenderer {
            has_map: false,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            zoom: 0,
            zoom_f: 0.0,
            display_zoom_f: 0.0,
            center_world_x: 0.0,
            center_world_y: 0.0,
            zoom_ui_x: 0.0,
            zoom_ui_y: 0.0,
            zoom_world_x: 0.0,
            zoom_world_y: 0.0,
            transition_to: false,
            transition_from_world_x: 0.0,
            transition_from_world_y: 0.0,
            transition_from_zoom_f: 0.0,
            transition_to_world_x: 0.0,
            transition_to_world_y: 0.0,
            transition_to_zoom_f: 0.0,
            transition_to_start_ms: 0,
            projection: Matrix4f::identity(),
            model_view: Matrix4f::identity(),
            model_view_projection: Matrix4f::identity(),
            model_view_change: Quaternionf {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            view_change_time: 0,
            is_isometric: false,
            first_update: false,
            drop_shadow_width: 12,
            max_zoom: 18.0,
            options: BOOLEAN_OPTIONS
                .iter()
                .map(|(n, d)| ConfigOption::boolean(n, *d))
                .collect(),
        }
    }

    pub fn get_absolute_x(&self) -> i32 {
        self.x
    }
    pub fn get_absolute_y(&self) -> i32 {
        self.y
    }
    pub fn get_width(&self) -> i32 {
        self.width
    }
    pub fn get_height(&self) -> i32 {
        self.height
    }

    // ---- options ----

    pub fn get_option_count(&self) -> usize {
        self.options.len()
    }
    pub fn get_option_by_index(&self, index: usize) -> Option<&ConfigOption> {
        self.options.get(index)
    }
    pub fn get_option_by_index_mut(&mut self, index: usize) -> Option<&mut ConfigOption> {
        self.options.get_mut(index)
    }
    pub fn get_option_index(&self, name: &str) -> Option<usize> {
        opt_index(name)
    }
    /// `setBoolean(name, value)`: unknown names are ignored.
    pub fn set_boolean(&mut self, name: &str, value: bool) {
        if let Some(i) = opt_index(name) {
            self.options[i].set_bool(value);
        }
    }
    /// `getBoolean(name)`: false for unknown names.
    pub fn get_boolean(&self, name: &str) -> bool {
        opt_index(name).is_some_and(|i| self.options[i].get_bool())
    }
    /// `setDouble(name, value)`: the renderer has no double options.
    pub fn set_double(&mut self, name: &str, value: f64) {
        if let Some(o) = opt_index(name).map(|i| &mut self.options[i]) {
            if let OptionKind::Double(_) = o.kind {
                o.set_double(value);
            }
        }
    }
    /// `getDouble(name, defaultValue)`.
    pub fn get_double(&self, name: &str, default_value: f64) -> f64 {
        match opt_index(name).map(|i| &self.options[i].kind) {
            Some(OptionKind::Double(d)) => d.value,
            _ => default_value,
        }
    }
    fn opt(&self, name: &str) -> bool {
        self.get_boolean(name)
    }
    /// `isDimUnsharedSymbols()`.
    pub fn is_dim_unshared_symbols(&self) -> bool {
        self.opt("DimUnsharedSymbols")
    }

    // ---- camera ----

    /// `calcMatrices(centerWorldX, centerWorldY, zoomF, projection,
    /// modelView)`: the projection and the model-view (identity, or the
    /// isometric tilt).
    pub fn calc_matrices(&self) -> (Matrix4f, Matrix4f) {
        let (w, h) = (self.width as f32, self.height as f32);
        let projection = Matrix4f::set_ortho(-w / 2.0, w / 2.0, h / 2.0, -h / 2.0, -2000.0, 2000.0);
        let mut model_view = Matrix4f::identity();
        if self.opt("Isometric") {
            model_view = Matrix4f::rotation_xyz(1.0471976, 0.0, 0.7853982);
        }
        (projection, model_view)
    }

    fn viewport(&self) -> [i32; 4] {
        [0, 0, self.width, self.height]
    }

    /// `getCameraRay(uiX, uiY, modelViewProjection)`: origin and direction.
    fn camera_ray(&self, ui_x: f32, ui_y: f32, mvp: &Matrix4f) -> ([f32; 3], [f32; 3]) {
        let inv = mvp.invert();
        let vp = self.viewport();
        let start = inv.unproject_inv(ui_x, ui_y, 0.0, vp);
        let end = inv.unproject_inv(ui_x, ui_y, 1.0, vp);
        let d = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
        (start, pz_joml::normalize(d))
    }

    /// `intersect_ray_plane` against the z = 0 plane; `None` when the ray
    /// misses (the caller then uses the origin).
    fn intersect_ground(origin: [f32; 3], dir: [f32; 3]) -> Option<[f32; 3]> {
        // u = direction * 10000, w = origin - point (the plane point is 0).
        let u = [dir[0] * 10000.0, dir[1] * 10000.0, dir[2] * 10000.0];
        let w = origin;
        let normal = [0.0f32, 0.0, 1.0];
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let d = dot(normal, u);
        let n = -dot(normal, w);
        if d.abs() < 1.0e-8 {
            return None;
        }
        let s = n / d;
        if !(0.0..=1.0).contains(&s) {
            return None;
        }
        Some([
            origin[0] + u[0] * s,
            origin[1] + u[1] * s,
            origin[2] + u[2] * s,
        ])
    }

    /// `uiToScene(uiX, uiY, modelViewProjection)`.
    pub fn ui_to_scene(&self, ui_x: f32, ui_y: f32, mvp: &Matrix4f) -> [f32; 3] {
        let (o, d) = self.camera_ray(ui_x, self.height as f32 - ui_y, mvp);
        Self::intersect_ground(o, d).unwrap_or([0.0; 3])
    }

    /// `sceneToUI(sceneX, sceneY, sceneZ, modelViewProjection)`.
    pub fn scene_to_ui(&self, x: f32, y: f32, z: f32, mvp: &Matrix4f) -> [f32; 3] {
        mvp.project(x, y, z, self.viewport())
    }

    /// The view's model-view-projection for `calcMatrices`' camera.
    fn calc_mvp(&self) -> Matrix4f {
        let (p, mv) = self.calc_matrices();
        mul(&p, &mv)
    }

    /// `uiToWorldX(uiX, uiY, zoomF, centerWorldX, centerWorldY)` and Y
    /// (`calcMatrices`' camera).
    pub fn ui_to_world_calc(
        &self,
        ui_x: f32,
        ui_y: f32,
        zoom_f: f32,
        cx: f32,
        cy: f32,
    ) -> (f32, f32) {
        self.ui_to_world_mvp(ui_x, ui_y, zoom_f, cx, cy, &self.calc_mvp())
    }

    /// `uiToWorldX/Y(uiX, uiY, zoomF, centerWorldX, centerWorldY,
    /// modelViewProjection)`.
    pub fn ui_to_world_mvp(
        &self,
        ui_x: f32,
        ui_y: f32,
        zoom_f: f32,
        cx: f32,
        cy: f32,
        mvp: &Matrix4f,
    ) -> (f32, f32) {
        let p = self.ui_to_scene(ui_x, ui_y, mvp);
        let s = 1.0 / self.get_world_scale(zoom_f);
        (p[0] * s + cx, p[1] * s + cy)
    }

    /// `worldToUIX/Y(worldX, worldY, zoomF, centerWorldX, centerWorldY,
    /// modelViewProjection)`.
    pub fn world_to_ui_mvp(
        &self,
        wx: f32,
        wy: f32,
        zoom_f: f32,
        cx: f32,
        cy: f32,
        mvp: &Matrix4f,
    ) -> (f32, f32) {
        let s = self.get_world_scale(zoom_f);
        let p = self.scene_to_ui((wx - cx) * s, (wy - cy) * s, 0.0, mvp);
        (p[0], self.height as f32 - p[1])
    }

    /// `worldToUIX/Y(..., projection, modelView)`.
    #[allow(clippy::too_many_arguments)]
    pub fn world_to_ui_pm(
        &self,
        wx: f32,
        wy: f32,
        zoom_f: f32,
        cx: f32,
        cy: f32,
        p: &Matrix4f,
        mv: &Matrix4f,
    ) -> (f32, f32) {
        self.world_to_ui_mvp(wx, wy, zoom_f, cx, cy, &mul(p, mv))
    }

    /// `worldOriginUIX(zoomF, centerWorldX)`.
    pub fn world_origin_ui_x(&self, zoom_f: f32, cx: f32) -> f32 {
        self.world_to_ui_mvp(
            0.0,
            0.0,
            zoom_f,
            cx,
            self.center_world_y,
            &self.model_view_projection,
        )
        .0
    }

    /// `worldOriginUIY(zoomF, centerWorldY)`.
    pub fn world_origin_ui_y(&self, zoom_f: f32, cy: f32) -> f32 {
        self.world_to_ui_mvp(
            0.0,
            0.0,
            zoom_f,
            self.center_world_x,
            cy,
            &self.model_view_projection,
        )
        .1
    }

    pub fn get_zoom(&self) -> i32 {
        self.zoom
    }
    pub fn get_zoom_f(&self) -> f32 {
        self.zoom_f
    }
    pub fn get_display_zoom_f(&self) -> f32 {
        self.display_zoom_f
    }
    /// `zoomMult()`.
    pub fn zoom_mult(&self) -> f32 {
        Self::zoom_mult_at(self.zoom_f)
    }
    /// `zoomMult(zoomF)`.
    pub fn zoom_mult_at(zoom_f: f32) -> f32 {
        2f64.powf(zoom_f as f64) as f32
    }

    /// `getWorldScale(zoomF)`: UI pixels per square.
    pub fn get_world_scale(&self, zoom_f: f32) -> f32 {
        let mpp = projection::meters_per_pixel_at_zoom(zoom_f as f64, self.height as f64);
        (1.0 / mpp) as f32
    }

    /// `zoomAt(mouseX, mouseY, delta)`.
    pub fn zoom_at(&mut self, map: &WorldMap, mouse_x: i32, mouse_y: i32, delta: i32) {
        let (wx, wy) = self.ui_to_world_calc(
            mouse_x as f32,
            mouse_y as f32,
            self.display_zoom_f,
            self.center_world_x,
            self.center_world_y,
        );
        self.zoom_f = clamp(
            self.zoom_f + delta as f32 / 2.0,
            self.get_base_zoom(map),
            self.get_max_zoom(),
        );
        self.zoom = self.zoom_f as i32;
        self.zoom_world_x = wx;
        self.zoom_world_y = wy;
        self.zoom_ui_x = mouse_x as f32;
        self.zoom_ui_y = mouse_y as f32;
    }

    /// `transitionTo(worldX, worldY, zoomF)` started at `now_ms`.
    pub fn transition_to(&mut self, map: &WorldMap, wx: f32, wy: f32, zoom_f: f32, now_ms: i64) {
        self.transition_from_world_x = self.center_world_x;
        self.transition_from_world_y = self.center_world_y;
        self.transition_from_zoom_f = self.display_zoom_f;
        self.transition_to_world_x = wx;
        self.transition_to_world_y = wy;
        self.transition_to_zoom_f = clamp(zoom_f, self.get_base_zoom(map), self.get_max_zoom());
        self.transition_to = true;
        self.transition_to_start_ms = now_ms;
    }

    pub fn get_center_world_x(&self) -> f32 {
        self.center_world_x
    }
    pub fn get_center_world_y(&self) -> f32 {
        self.center_world_y
    }

    /// `centerOn(worldX, worldY)`.
    pub fn center_on(&mut self, wx: f32, wy: f32) {
        self.center_world_x = wx;
        self.center_world_y = wy;
        if self.display_zoom_f != self.zoom_f {
            self.zoom_world_x = wx;
            self.zoom_world_y = wy;
            self.zoom_ui_x = self.width as f32 / 2.0;
            self.zoom_ui_y = self.height as f32 / 2.0;
        }
    }

    /// `moveView(dx, dy)`.
    pub fn move_view(&mut self, dx: i32, dy: i32) {
        self.center_on(
            self.center_world_x + dx as f32,
            self.center_world_y + dy as f32,
        );
    }

    /// `getBaseZoom()`: the zoom that fits the map's bounds.
    pub fn get_base_zoom(&self, map: &WorldMap) -> f32 {
        let h = self.height as f64;
        let mut zoom =
            projection::zoom_at_meters_per_pixel(map.get_height_in_squares() as f64 / h, h);
        if map.get_width_in_squares() as f32 * self.get_world_scale(zoom as f32) > self.width as f32
        {
            zoom = projection::zoom_at_meters_per_pixel(
                map.get_width_in_squares() as f64 / self.width as f64,
                h,
            );
        }
        if self.opt("ClampBaseZoomToPoint5") {
            zoom = ((zoom * 2.0) as i32) as f64 / 2.0;
        }
        let floor = if self.opt("InfiniteZoom") {
            zoom as f32
        } else {
            12.0
        };
        max_f(zoom as f32, floor)
    }

    /// `setZoom(zoom)`.
    pub fn set_zoom(&mut self, map: &WorldMap, zoom: f32) {
        self.zoom_f = clamp(zoom, self.get_base_zoom(map), self.get_max_zoom());
        self.zoom = self.zoom_f as i32;
        self.display_zoom_f = self.zoom_f;
    }

    /// `setMaxZoom(maxZoom)`.
    pub fn set_max_zoom(&mut self, max_zoom: f32) {
        self.max_zoom = clamp(max_zoom, 0.0, 24.0);
    }

    /// `getMaxZoom()`.
    pub fn get_max_zoom(&self) -> f32 {
        if self.opt("InfiniteZoom") {
            24.0
        } else {
            self.max_zoom
        }
    }

    /// `resetView()`: the whole map, centred.
    pub fn reset_view(&mut self, map: &WorldMap) {
        self.zoom_f = if self.width.wrapping_mul(self.height) < 1 {
            12.0
        } else {
            self.get_base_zoom(map)
        };
        self.zoom = self.zoom_f as i32;
        self.center_world_x =
            map.get_min_x_in_squares() as f32 + map.get_width_in_squares() as f32 / 2.0;
        self.center_world_y =
            map.get_min_y_in_squares() as f32 + map.get_height_in_squares() as f32 / 2.0;
        self.zoom_world_x = self.center_world_x;
        self.zoom_world_y = self.center_world_y;
        self.zoom_ui_x = self.width as f32 / 2.0;
        self.zoom_ui_y = self.height as f32 / 2.0;
    }

    pub fn get_projection_matrix(&self) -> &Matrix4f {
        &self.projection
    }
    pub fn get_model_view_matrix(&self) -> &Matrix4f {
        &self.model_view
    }
    pub fn get_model_view_projection_matrix(&self) -> &Matrix4f {
        &self.model_view_projection
    }

    /// `setMap(worldMap, x, y, width, height)`.
    pub fn set_map(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.has_map = true;
        self.x = x;
        self.y = y;
        self.width = width;
        self.height = height;
    }

    /// `getWorldMap() != null`.
    pub fn has_map(&self) -> bool {
        self.has_map
    }

    /// `updateView()` at `now_ms`, `millis_since_last_render` being
    /// `UIManager.getMillisSinceLastRender()`.
    pub fn update_view(&mut self, map: &WorldMap, now_ms: i64, millis_since_last_render: f64) {
        if !self.has_map
            || map.get_width_in_squares() <= 1
            || map.get_height_in_squares() <= 1
            || self.width < 1
            || self.height < 1
        {
            return;
        }
        if self.display_zoom_f == 0.0 {
            self.display_zoom_f = self.zoom_f;
        }
        if self.display_zoom_f != self.zoom_f {
            let dt = (millis_since_last_render / 750.0) as f32;
            let diff = (self.zoom_f - self.display_zoom_f).abs();
            let mult = if diff > 0.25 { diff / 0.25 } else { 1.0 };
            if self.display_zoom_f < self.zoom_f {
                self.display_zoom_f = min_f(self.display_zoom_f + dt * mult, self.zoom_f);
            } else if self.display_zoom_f > self.zoom_f {
                self.display_zoom_f = max_f(self.display_zoom_f - dt * mult, self.zoom_f);
            }
            let (wx2, wy2) = self.ui_to_world_calc(
                self.zoom_ui_x,
                self.zoom_ui_y,
                self.display_zoom_f,
                0.0,
                0.0,
            );
            self.center_world_x = self.zoom_world_x - wx2;
            self.center_world_y = self.zoom_world_y - wy2;
        }
        if !self.first_update {
            self.first_update = true;
            self.is_isometric = self.opt("Isometric");
        }
        if self.is_isometric != self.opt("Isometric") {
            self.is_isometric = self.opt("Isometric");
            if self.view_change_time + VIEW_CHANGE_TIME < now_ms {
                self.model_view_change = Quaternionf::set_from_unnormalized(&self.model_view);
            }
            self.view_change_time = now_ms;
        }
        if self.transition_to {
            let elapsed = now_ms - self.transition_to_start_ms;
            if elapsed >= TRANSITION_TIME {
                self.transition_to = false;
                self.center_world_x = self.transition_to_world_x;
                self.center_world_y = self.transition_to_world_y;
                self.zoom_f = self.transition_to_zoom_f;
                self.zoom = fast_floor(self.zoom_f);
                self.display_zoom_f = self.zoom_f;
            } else {
                let f = ease_out_in_quad(elapsed as f32 / 600.0);
                self.center_world_x =
                    lerp(self.transition_from_world_x, self.transition_to_world_x, f);
                self.center_world_y =
                    lerp(self.transition_from_world_y, self.transition_to_world_y, f);
                self.zoom_f = lerp(self.transition_from_zoom_f, self.transition_to_zoom_f, f);
                self.zoom = fast_floor(self.zoom_f);
                self.display_zoom_f = self.zoom_f;
            }
        }
        let (p, mv) = self.calc_matrices();
        self.projection = p;
        self.model_view = mv;
        if self.view_change_time + VIEW_CHANGE_TIME > now_ms {
            let f = (self.view_change_time + VIEW_CHANGE_TIME - now_ms) as f32 / 175.0;
            let q1 = self.model_view_change;
            let q2 = Quaternionf::set_from_unnormalized(&self.model_view);
            self.model_view = Matrix4f::rotation_quat(&q1.slerp(&q2, 1.0 - f));
        }
        self.model_view_projection = mul(&self.projection, &self.model_view);
    }

    /// `setDropShadowWidth(width)`.
    pub fn set_drop_shadow_width(&mut self, width: i32) {
        self.drop_shadow_width = width;
    }
}

/// `Matrix4f.mul(right)`: JOML copies `this` when `right` is flagged
/// identity (keeping negative zeros the general product would lose).
pub fn mul(a: &Matrix4f, b: &Matrix4f) -> Matrix4f {
    if *b == Matrix4f::identity() {
        *a
    } else {
        a.mul(b)
    }
}

/// `PZMath.clamp(float, float, float)`.
pub fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// `PZMath.max(float, float)`.
pub fn max_f(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `PZMath.min(float, float)`.
pub fn min_f(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `PZMath.lerp(float, float, float)`.
pub fn lerp(src: f32, dest: f32, alpha: f32) -> f32 {
    src + (dest - src) * alpha
}

/// `PZMath.fastfloor(float)`.
pub fn fast_floor(v: f32) -> i32 {
    let xi = v as i32;
    if v < xi as f32 {
        xi - 1
    } else {
        xi
    }
}

/// `PZMath.lerpFunc_EaseOutInQuad`.
pub fn ease_out_in_quad(x: f32) -> f32 {
    if x < 0.5 {
        x * x * 2.0
    } else {
        let r = 1.0 - (2.0 * x - 1.0);
        0.5 + (1.0 - r * r) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> WorldMap {
        let mut m = WorldMap::new();
        m.set_bounds_in_cells(0, 0, 9, 9);
        m
    }

    #[test]
    fn options() {
        let mut r = WorldMapRenderer::new();
        assert_eq!(r.get_option_count(), 49);
        assert!(r.get_boolean("Isometric"));
        r.set_boolean("Isometric", false);
        assert!(!r.get_boolean("Isometric"));
        r.set_boolean("Nope", true);
        assert!(!r.get_boolean("Nope"));
        assert_eq!(r.get_double("Isometric", 3.5), 3.5);
    }

    #[test]
    fn ortho_round_trip() {
        let m = map();
        let mut r = WorldMapRenderer::new();
        r.set_boolean("Isometric", false);
        r.set_map(0, 0, 800, 600);
        r.reset_view(&m);
        r.update_view(&m, 1000, 16.0);
        let mvp = *r.get_model_view_projection_matrix();
        let z = r.get_display_zoom_f();
        let (ux, uy) = r.world_to_ui_mvp(
            1000.0,
            700.0,
            z,
            r.get_center_world_x(),
            r.get_center_world_y(),
            &mvp,
        );
        let (wx, wy) = r.ui_to_world_mvp(
            ux,
            uy,
            z,
            r.get_center_world_x(),
            r.get_center_world_y(),
            &mvp,
        );
        assert!(
            (wx - 1000.0).abs() < 0.01 && (wy - 700.0).abs() < 0.01,
            "{wx} {wy}"
        );
    }

    fn b(f: f32) -> String {
        format!("{:x}", f.to_bits())
    }

    fn dump(r: &WorldMapRenderer, m: &WorldMap, sz: [i32; 2], out: &mut Vec<String>) {
        let z = r.get_display_zoom_f();
        let (cx, cy) = (r.get_center_world_x(), r.get_center_world_y());
        let mut s = format!(
            "view {} {} {} {} {} {} {}",
            b(r.get_base_zoom(m)),
            b(r.get_zoom_f()),
            b(z),
            r.get_zoom(),
            b(cx),
            b(cy),
            b(r.get_world_scale(z))
        );
        let mvp = *r.get_model_view_projection_matrix();
        for f in mvp.to_array() {
            s.push_str(&format!(" {}", b(f)));
        }
        out.push(s);
        let ui = [
            [0.0, 0.0],
            [sz[0] as f32 / 2.0, sz[1] as f32 / 2.0],
            [123.5, 77.25],
            [sz[0] as f32, sz[1] as f32],
            [-50.0, 1000.0],
        ];
        for p in ui {
            let a = r.ui_to_world_calc(p[0], p[1], z, cx, cy);
            let c = r.ui_to_world_mvp(p[0], p[1], z, cx, cy, &mvp);
            out.push(format!("ui2w {} {} {} {}", b(a.0), b(a.1), b(c.0), b(c.1)));
        }
        let w = [
            [0.0, 0.0],
            [cx, cy],
            [1234.5, 987.25],
            [cx + 100.0, cy - 300.0],
        ];
        for p in w {
            let a = r.world_to_ui_mvp(p[0], p[1], z, cx, cy, &mvp);
            out.push(format!(
                "w2ui {} {} {} {}",
                b(a.0),
                b(a.1),
                b(r.world_origin_ui_x(z, cx)),
                b(r.world_origin_ui_y(z, cy))
            ));
        }
    }

    /// Parity with the original `WorldMapRenderer` view (oracle
    /// `view.txt` from `PZ_WORLDMAP_ORACLE_DIR`): base zoom, reset, zoom
    /// steps, the camera matrices and both transforms, flat and
    /// isometric, over sizes, bounds and the zoom options.
    #[test]
    fn matches_view_oracle() {
        let Some(dir) = std::env::var_os("PZ_WORLDMAP_ORACLE_DIR") else {
            eprintln!("PZ_WORLDMAP_ORACLE_DIR not set; skipping");
            return;
        };
        let want = std::fs::read_to_string(std::path::Path::new(&dir).join("view.txt")).unwrap();
        let mut out = Vec::new();
        for iso in 0..2 {
            for sz in [[800, 600], [1280, 720], [300, 900]] {
                for bd in [[0, 0, 9, 9], [20, 30, 40, 45]] {
                    for cl in 0..2 {
                        for inf in 0..2 {
                            let mut m = WorldMap::new();
                            let mut r = WorldMapRenderer::new();
                            r.set_boolean("Isometric", iso == 1);
                            r.set_boolean("ClampBaseZoomToPoint5", cl == 1);
                            r.set_boolean("InfiniteZoom", inf == 1);
                            m.set_bounds_in_cells(bd[0], bd[1], bd[2], bd[3]);
                            r.set_map(10, 20, sz[0], sz[1]);
                            r.reset_view(&m);
                            r.update_view(&m, 1_000_000, 0.0);
                            out.push(format!(
                                "case {iso} {}x{} {},{},{},{} {cl} {inf}",
                                sz[0], sz[1], bd[0], bd[1], bd[2], bd[3]
                            ));
                            dump(&r, &m, sz, &mut out);
                            r.zoom_at(&m, 100, 50, -3);
                            r.update_view(&m, 1_000_000, 16.0);
                            out.push("zoomAt".into());
                            dump(&r, &m, sz, &mut out);
                            for _ in 0..5 {
                                r.update_view(&m, 1_000_000, 16.0);
                            }
                            dump(&r, &m, sz, &mut out);
                            r.set_zoom(&m, 15.25);
                            r.center_on(3000.5, 2100.25);
                            r.update_view(&m, 1_000_000, 16.0);
                            out.push("setZoom".into());
                            dump(&r, &m, sz, &mut out);
                            r.move_view(-37, 12);
                            r.zoom_at(&m, 0, 0, 5);
                            r.center_on(1000.0, 900.0);
                            r.update_view(&m, 1_000_000, 16.0);
                            out.push("zoomAt2".into());
                            dump(&r, &m, sz, &mut out);
                            r.set_max_zoom(30.0);
                            out.push(format!("max {}", b(r.get_max_zoom())));
                            r.set_max_zoom(-1.0);
                            out.push(format!(
                                "max {} {}",
                                b(r.get_max_zoom()),
                                b(r.get_base_zoom(&m))
                            ));
                            let _ = &mut m;
                        }
                    }
                }
            }
        }
        let want: Vec<&str> = want.lines().collect();
        let mut bad = 0;
        for (i, (g, w)) in out.iter().zip(&want).enumerate() {
            if g != w {
                if bad < 10 {
                    eprintln!("line {i}\n got {g}\nwant {w}");
                }
                bad += 1;
            }
        }
        assert_eq!(out.len(), want.len());
        assert_eq!(bad, 0, "{bad} lines differ");
    }

    /// The isometric toggle's blend (`setFromUnnormalized`, `slerp`,
    /// `Matrix4f.set(Quaternionf)`) against JOML (oracle `quat.txt`).
    #[test]
    fn matches_quat_oracle() {
        let Some(dir) = std::env::var_os("PZ_WORLDMAP_ORACLE_DIR") else {
            eprintln!("PZ_WORLDMAP_ORACLE_DIR not set; skipping");
            return;
        };
        let want = std::fs::read_to_string(std::path::Path::new(&dir).join("quat.txt")).unwrap();
        let ms = [
            Matrix4f::identity(),
            Matrix4f::rotation_xyz(1.0471976, 0.0, 0.7853982),
        ];
        let mut out = Vec::new();
        for m1 in &ms {
            for m2 in &ms {
                for f in [0.0f32, 0.1, 0.25, 0.5, 0.77, 1.0] {
                    let q1 = Quaternionf::set_from_unnormalized(m1);
                    let q2 = Quaternionf::set_from_unnormalized(m2);
                    let q = q1.slerp(&q2, f);
                    let r = Matrix4f::rotation_quat(&q);
                    let mut s = String::from("q");
                    for qq in [q2, q] {
                        for v in [qq.x, qq.y, qq.z, qq.w] {
                            s.push_str(&format!(" {}", b(v)));
                        }
                    }
                    for v in r.to_array() {
                        s.push_str(&format!(" {}", b(v)));
                    }
                    out.push(s);
                }
            }
        }
        assert_eq!(out, want.lines().collect::<Vec<_>>());
    }
}
