//! `zombie.worldMap.UIWorldMap`: the map element (on the pz-ui extension
//! slot) and the state its `UIWorldMapV1/V2/V3` API objects drive.

use super::host::with_host;
use crate::bind_ui::UIELEMENT;
use pz_kahlua::Thread;
use pz_text::UIFont;
use pz_ui::{with_ui, UIElement, UIElementExt};
use pz_worldmap::style::WorldMapStyle;
use pz_worldmap::{WorldMap, WorldMapRenderer};
use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
use std::rc::{Rc, Weak};

pub const UIWORLDMAP: &str = "zombie.worldMap.UIWorldMap";
const SUPERS: &[&str] = &[UIELEMENT];

/// The fields `UIWorldMap` and its API objects share.
pub struct MapState {
    pub world_map: WorldMap,
    pub renderer: WorldMapRenderer,
    pub style: WorldMapStyle,
    /// `color`: the background, `RGBAf(0.85882354, 0.84313726, 0.7529412, 1)`.
    pub color: [f32; 4],
    pub do_stencil: bool,
    pub map_editor: bool,
}

/// `zombie.worldMap.UIWorldMap`.
pub struct UIWorldMap {
    st: RefCell<MapState>,
    /// `apiV1`, `apiV2`, `apiV3`, made on first use (they point back at
    /// the element weakly).
    pub(super) apis: RefCell<[Option<Rc<dyn pz_kahlua::value::LuaObject>>; 3]>,
    /// `getStyleAPI()` of V1 (`WorldMapStyleV1`) and of V3
    /// (`WorldMapStyleV2`).
    pub(super) style_apis: RefCell<[Option<Rc<dyn pz_kahlua::value::LuaObject>>; 2]>,
}

impl UIWorldMap {
    /// `new UIWorldMap(table)`.
    pub fn create(table: Option<pz_kahlua::TableRef>) -> Rc<UIElement> {
        UIElement::new_ext(
            table,
            Box::new(UIWorldMap {
                st: RefCell::new(MapState {
                    world_map: WorldMap::new(),
                    renderer: WorldMapRenderer::new(),
                    style: WorldMapStyle::new(),
                    color: [0.858_823_54, 0.843_137_26, 0.752_941_2, 1.0],
                    do_stencil: true,
                    map_editor: false,
                }),
                apis: RefCell::new([None, None, None]),
                style_apis: RefCell::new([None, None]),
            }),
        )
    }

    pub fn st(&self) -> Ref<'_, MapState> {
        self.st.borrow()
    }
    pub fn st_mut(&self) -> RefMut<'_, MapState> {
        self.st.borrow_mut()
    }

    /// `renderer.setMap(worldMap, absX, absY, width, height)` with the
    /// element's current geometry (`intValue()` of each).
    pub fn set_map_from(&self, el: &UIElement) {
        let (x, y, w, h) = geometry(el);
        self.st.borrow_mut().renderer.set_map(x, y, w, h);
    }

    /// `scaleWidthToHeight()`.
    pub fn scale_width_to_height(&self, el: &UIElement) {
        let width = {
            let s = self.st.borrow();
            let h = el.get_height();
            let zoom = pz_worldmap::projection::zoom_at_meters_per_pixel(
                s.world_map.get_height_in_squares() as f64 / h,
                h,
            );
            let scale = s.renderer.get_world_scale(zoom as f32);
            s.world_map.get_width_in_squares() as f32 * scale
        };
        el.set_width(width as f64);
        if let Some(t) = el.get_table() {
            t.set_str("width", el.get_width());
        }
        self.set_map_from(el);
        let mut s = self.st.borrow_mut();
        let MapState {
            world_map,
            renderer,
            ..
        } = &mut *s;
        renderer.reset_view(world_map);
    }

    /// `renderPlayer(x, y)`: a 6x6 red square.
    fn render_player(&self, el: &UIElement, px: f32, py: f32) {
        let (ux, uy) = {
            let s = self.st.borrow();
            let r = &s.renderer;
            r.world_to_ui_mvp(
                px,
                py,
                r.get_display_zoom_f(),
                r.get_center_world_x(),
                r.get_center_world_y(),
                r.get_model_view_projection_matrix(),
            )
        };
        let (ux, uy) = (
            pz_worldmap::renderer::fast_floor(ux) as f32,
            pz_worldmap::renderer::fast_floor(uy) as f32,
        );
        el.draw_texture_scaled_col(
            None,
            ux as f64 - 3.0,
            uy as f64 - 3.0,
            6.0,
            6.0,
            1.0,
            0.0,
            0.0,
            1.0,
        );
    }

    /// `renderPlayerName(x, y, userName)`.
    fn render_player_name(&self, el: &UIElement, px: f32, py: f32, name: &str) {
        let (show, ux, uy) = {
            let s = self.st.borrow();
            let r = &s.renderer;
            let (ux, uy) = r.world_to_ui_mvp(
                px,
                py,
                r.get_display_zoom_f(),
                r.get_center_world_x(),
                r.get_center_world_y(),
                r.get_model_view_projection_matrix(),
            );
            (r.get_boolean("PlayerNames"), ux, uy)
        };
        if !show || name.trim().is_empty() {
            return;
        }
        let ux = pz_worldmap::renderer::fast_floor(ux) as f32;
        let uy = pz_worldmap::renderer::fast_floor(uy) as f32;
        let text_width = pz_ui::draw::measure_string_x(Some(UIFont::Small), name) + 16;
        // `TextManager.instance.font.getLineHeight()`: the default font.
        let font_height = pz_ui::draw::font_height(None);
        let text_height = (font_height as f64 * 1.25).ceil() as i32;
        el.draw_texture_scaled_col(
            None,
            ux as f64 - text_width as f64 / 2.0,
            uy as f64 + 4.0,
            text_width as f64,
            text_height as f64,
            0.5,
            0.5,
            0.5,
            0.5,
        );
        el.draw_text_centre(
            None,
            name,
            ux as f64,
            (uy + 4.0) as f64 + (text_height - font_height) as f64 / 2.0,
            0.0,
            0.0,
            0.0,
            1.0,
        );
    }

    /// `renderLocalPlayers()`.
    fn render_local_players(&self, el: &UIElement) {
        {
            let s = self.st.borrow();
            if !s.renderer.get_boolean("Players") || s.renderer.get_display_zoom_f() >= 20.0 {
                return;
            }
        }
        let client = crate::hud::host::with_host(|h| h.network.is_client());
        for p in with_host(|h| h.players.live_players()) {
            self.render_player(el, p.x, p.y);
            if client {
                self.render_player_name(el, p.x, p.y, &p.username);
            }
        }
    }
}

/// `(getAbsoluteX().intValue(), getAbsoluteY().intValue(),
/// getWidth().intValue(), getHeight().intValue())`.
pub fn geometry(el: &UIElement) -> (i32, i32, i32, i32) {
    (
        el.get_absolute_x() as i32,
        el.get_absolute_y() as i32,
        el.get_width() as i32,
        el.get_height() as i32,
    )
}

/// `Core.debug`.
fn core_debug() -> bool {
    crate::bind_core::shared_core().borrow().debug
}

impl UIElementExt for UIWorldMap {
    fn class_name(&self) -> &'static str {
        UIWORLDMAP
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `render()`.
    fn render(&self, el: &UIElement, t: &mut Thread) {
        if !el.is_visible() {
            return;
        }
        if let Some(p) = el.get_parent() {
            let mdh = p.get_max_draw_height();
            if mdh != -1.0 && mdh <= el.get_y() {
                return;
            }
        }
        let (ax, ay, w, h) = geometry(el);
        let sx1 = el.clamp_to_parent_x(ax as f64);
        let sx2 = el.clamp_to_parent_x(ax as f64 + el.get_width());
        let sy1 = el.clamp_to_parent_y(ay as f64);
        let sy2 = el.clamp_to_parent_y(ay as f64 + el.get_height());
        let do_stencil = self.st.borrow().do_stencil;
        if do_stencil {
            el.set_stencil_rect(
                (sx1 as i32 - ax) as f64,
                (sy1 as i32 - ay) as f64,
                (sx2 as i32 - sx1 as i32) as f64,
                (sy2 as i32 - sy1 as i32) as f64,
            );
        }
        let color = self.st.borrow().color;
        el.draw_texture_scaled_col(
            None,
            0.0,
            0.0,
            el.get_width(),
            el.get_height(),
            color[0] as f64,
            color[1] as f64,
            color[2] as f64,
            color[3] as f64,
        );
        let now = pz_ui::manager::now_ms();
        let since = with_ui(|m| m.ui_render_interval_ms) as f64;
        {
            let mut s = self.st.borrow_mut();
            let MapState {
                world_map,
                renderer,
                ..
            } = &mut *s;
            renderer.set_map(ax, ay, w, h);
            renderer.update_view(world_map, now, since);
        }
        self.draw_map();
        // TODO(worldmap): markers, visited, streets, WorldGenDebug roads,
        // AnimalZones and virtual vehicles.
        self.render_local_players(el);
        let (players, zoom_f, player_model) = {
            let s = self.st.borrow();
            (
                s.renderer.get_boolean("Players"),
                s.renderer.get_display_zoom_f(),
                s.renderer.get_boolean("PlayerModel"),
            )
        };
        if players && !(zoom_f >= 20.0 && player_model) {
            for p in with_host(|h| h.players.live_players()) {
                self.render_player(el, p.x, p.y);
            }
        }
        let debug = core_debug();
        if debug && self.st.borrow().renderer.get_boolean("DebugInfo") {
            self.render_debug_info(el);
        }
        if do_stencil {
            el.clear_stencil_rect();
            el.repaint_stencil_rect(0.0, 0.0, el.get_width(), el.get_height());
        }
        if debug && crate::hud::host::with_host(|h| h.debug.ui_render_outline()) {
            let x = -el.get_x_scroll();
            let y = -el.get_y_scroll();
            let r1 = if el.is_mouse_over() { 0.0 } else { 1.0 };
            let (w, h) = (el.get_width(), el.get_height());
            el.draw_texture_scaled_col(None, x, y, 1.0, h, r1, 1.0, 1.0, 0.5);
            el.draw_texture_scaled_col(None, x + 1.0, y, w - 2.0, 1.0, r1, 1.0, 1.0, 0.5);
            el.draw_texture_scaled_col(None, x + w - 1.0, y, 1.0, h, r1, 1.0, 1.0, 0.5);
            el.draw_texture_scaled_col(None, x + 1.0, y + h - 1.0, w - 2.0, 1.0, r1, 1.0, 1.0, 0.5);
        }
        if debug && self.st.borrow().renderer.get_boolean("HitTest") {
            self.render_hit_test(el);
        }
        // TODO(world): BuildingsWithoutFeatures and Basements need
        // IsoMetaGrid's BuildingDefs.
        el.render_base(t);
    }
}

impl UIWorldMap {
    /// `renderer.render(this)`: the drawer's frame, queued like
    /// `SpriteRenderer.drawGeneric`.
    fn draw_map(&self) {
        // WorldMapImages.startFrame/endFrame bracket the frame in
        // SpriteRenderer; here they bracket each map's draw.
        pz_worldmap::pyramid::start_frame();
        let draw = {
            let s = self.st.borrow();
            if !s.renderer.has_map() {
                return;
            }
            with_host(|h| {
                let inp = pz_worldmap::draw::DrawInputs {
                    map: &s.world_map,
                    style: &s.style,
                    fill: s.color,
                    files: &*h.map_files,
                    debug: core_debug(),
                };
                pz_worldmap::draw::Drawer::new(&s.renderer, &inp).render()
            })
        };
        super::present::submit(&draw);
        pz_worldmap::pyramid::end_frame();
    }

    /// `HitTest`: the outline of the last data feature under the mouse.
    fn render_hit_test(&self, el: &UIElement) {
        let (wx, wy) = (self.mouse_to_world_x(el), self.mouse_to_world_y(el));
        let mut last = None;
        for d in self.st.borrow().world_map.data.iter() {
            if !d.is_ready() {
                continue;
            }
            if let Some((cell, hits)) = d.hit_test(wx, wy) {
                if let Some(&i) = hits.last() {
                    last = Some((cell, i));
                }
            }
        }
        let Some((cell, i)) = last else { return };
        let (cx, cy) = (cell.x * 256, cell.y * 256);
        let (ax, ay) = (el.get_absolute_x() as i32, el.get_absolute_y() as i32);
        let Some(g) = cell.features[i].geometry.as_ref() else {
            return;
        };
        for ring in &g.points {
            let n = ring.num_points();
            for j in 0..n {
                let (p1x, p1y) = (ring.get_x(&cell.points, j), ring.get_y(&cell.points, j));
                let (p2x, p2y) = (
                    ring.get_x(&cell.points, (j + 1) % n),
                    ring.get_y(&cell.points, (j + 1) % n),
                );
                let (x1, y1) = self.world_to_ui((cx + p1x) as f32, (cy + p1y) as f32);
                let (x2, y2) = self.world_to_ui((cx + p2x) as f32, (cy + p2y) as f32);
                with_ui(|m| {
                    m.sprite.renderline(
                        None,
                        ax + x1 as i32,
                        ay + y1 as i32,
                        ax + x2 as i32,
                        ay + y2 as i32,
                        pz_render::Rgba(1.0, 0.0, 0.0, 1.0),
                        1.0,
                    )
                });
            }
        }
    }

    /// The `DebugInfo` panel: the square and cell under the mouse, the
    /// centre, the zoom and the scale.
    fn render_debug_info(&self, el: &UIElement) {
        use pz_core::config::float_to_string as jf;
        let font_hgt = pz_ui::draw::font_height(Some(UIFont::Small));
        el.draw_texture_scaled_col(
            None,
            0.0,
            0.0,
            200.0,
            font_hgt as f64 * 6.0,
            1.0,
            1.0,
            1.0,
            1.0,
        );
        let (wx, wy) = (self.mouse_to_world_x(el), self.mouse_to_world_y(el));
        let lines = {
            let s = self.st.borrow();
            let r = &s.renderer;
            [
                format!("SQUARE = {},{}", wx as i32, wy as i32),
                format!("CELL = {},{}", (wx / 256.0) as i32, (wy / 256.0) as i32),
                format!(
                    "CELL (300) = {},{}",
                    (wx / 300.0) as i32,
                    (wy / 300.0) as i32
                ),
                format!(
                    "CENTER = {},{}",
                    pz_worldmap::renderer::fast_floor(r.get_center_world_x()),
                    pz_worldmap::renderer::fast_floor(r.get_center_world_y())
                ),
                format!("ZOOM = {}", jf(r.get_display_zoom_f())),
                format!("SCALE = {}", jf(r.get_world_scale(r.get_zoom_f()))),
            ]
        };
        let mut y = 0;
        for l in lines {
            el.draw_text_default(&l, 0.0, y as f64, 0.0, 0.0, 0.0, 1.0);
            y += font_hgt;
        }
    }

    /// `apiV1.mouseToWorldX()`.
    pub fn mouse_to_world_x(&self, el: &UIElement) -> f32 {
        let (mx, my) = mouse_local(el);
        self.ui_to_world_x(mx, my)
    }

    /// `apiV1.mouseToWorldY()`.
    pub fn mouse_to_world_y(&self, el: &UIElement) -> f32 {
        let (mx, my) = mouse_local(el);
        self.ui_to_world_y(mx, my)
    }

    fn has_content(&self) -> bool {
        let s = self.st.borrow();
        s.world_map.has_data() || s.world_map.has_images()
    }

    /// `apiV1.uiToWorldX(uiX, uiY)`.
    pub fn ui_to_world_x(&self, ui_x: f32, ui_y: f32) -> f32 {
        if !self.has_content() {
            return 0.0;
        }
        let s = self.st.borrow();
        let r = &s.renderer;
        r.ui_to_world_calc(
            ui_x,
            ui_y,
            r.get_display_zoom_f(),
            r.get_center_world_x(),
            r.get_center_world_y(),
        )
        .0
    }

    /// `apiV1.uiToWorldY(uiX, uiY)`.
    pub fn ui_to_world_y(&self, ui_x: f32, ui_y: f32) -> f32 {
        if !self.has_content() {
            return 0.0;
        }
        let s = self.st.borrow();
        let r = &s.renderer;
        r.ui_to_world_calc(
            ui_x,
            ui_y,
            r.get_display_zoom_f(),
            r.get_center_world_y(),
            r.get_center_world_y(),
        )
        .1
    }

    /// `apiV1.worldToUIX/Y(worldX, worldY)` with the last view's matrix.
    pub fn world_to_ui(&self, wx: f32, wy: f32) -> (f32, f32) {
        if !self.has_content() {
            return (0.0, 0.0);
        }
        let s = self.st.borrow();
        let r = &s.renderer;
        r.world_to_ui_mvp(
            wx,
            wy,
            r.get_display_zoom_f(),
            r.get_center_world_x(),
            r.get_center_world_y(),
            r.get_model_view_projection_matrix(),
        )
    }
}

/// `Mouse.getXA() - getAbsoluteX().intValue()` and Y.
fn mouse_local(el: &UIElement) -> (f32, f32) {
    let (mx, my) = with_ui(|m| (m.mouse.x, m.mouse.y));
    (
        (mx - el.get_absolute_x() as i32) as f32,
        (my - el.get_absolute_y() as i32) as f32,
    )
}

/// The element an API object or option belongs to.
pub fn upgrade(w: &Weak<UIElement>) -> Option<Rc<UIElement>> {
    w.upgrade()
}
