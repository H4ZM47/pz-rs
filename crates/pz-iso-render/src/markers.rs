//! The markers drawn in the per-level loop of `performRenderTiles`, after
//! the level's shadows (and the vision cone) and before its translucent
//! objects: `WorldMarkers.renderGridSquareMarkers(z)`
//! (`FBORenderWorldMarkers`: circles laid on the floor under the model
//! camera) and `IsoMarkers.renderIsoMarkers` (textures billboarded on a
//! square, and circles queued for `LineDrawer`); and
//! `FBORenderCell.renderOpaqueObjectsEvent`, Lua's
//! `RenderOpaqueObjectsInWorld` before the moving objects.
//!
//! `WorldMarkers`' homing points and direction arrows draw in
//! `IngameState.renderframetext`, after the world's offscreen pass, in
//! screen space: [`DirectionArrow`] / [`draw_texture_angle`].

use pz_render::iso::{self, fastfloor};
use pz_render::{gl, DrawCmd, DrawList, Rgba, Texture, WorldShader};

use crate::area_highlights::{floor_quad, iso_circle, Line};
use crate::cell::ChunkRenderer;
use crate::models::ModelCamera;
use crate::Camera;

/// A `RenderOpaqueObjectsInWorld` handler: `(playerIndex is the frame's)
/// x, y, z` of the square and the frame's draw list.
pub type OpaqueObjectsHandler<'a> = &'a mut dyn FnMut(i32, i32, i32, &mut Vec<DrawCmd>);

/// `renderOpaqueObjectsEvent`'s inputs: where the event is raised and the
/// Lua handlers it reaches (owner: pz-lua-api's event dispatch; in the
/// game `ISBuildingObject` ghosts and fishing `Bobber`s draw from it).
pub struct OpaqueObjectsEvent<'a> {
    /// `JoypadManager.getFromPlayer(playerIndex) != null`: the event is at
    /// the player's square instead of the picked tile.
    pub joypad: bool,
    /// `UIManager.getPickedTile()` (owner: the UI's mouse picking).
    pub picked_tile: Option<[f32; 2]>,
    /// `IsoWorld.isValidSquare(x, y, z)` (owner: the world bounds).
    pub valid_square: &'a dyn Fn(i32, i32, i32) -> bool,
    pub handler: OpaqueObjectsHandler<'a>,
}

impl OpaqueObjectsEvent<'_> {
    /// The square the event is raised for, if any.
    pub fn square(&self, cam: &Camera) -> Option<(i32, i32, i32)> {
        let z = fastfloor(cam.z);
        let (x, y) = if self.joypad {
            (fastfloor(cam.x), fastfloor(cam.y))
        } else {
            let [px, py] = self.picked_tile?;
            (fastfloor(px), fastfloor(py))
        };
        (self.valid_square)(x, y, z).then_some((x, y, z))
    }

    /// `renderOpaqueObjectsEvent(playerIndex)`.
    pub fn trigger(&mut self, cam: &Camera, out: &mut Vec<DrawCmd>) {
        if let Some((x, y, z)) = self.square(cam) {
            (self.handler)(x, y, z, out);
        }
    }
}

/// A `WorldMarkers.GridSquareMarker` (owner: the Lua callers of
/// `getWorldMarkers():addGridSquareMarker`, with `WorldMarkers.update`'s
/// fade and blink).
#[derive(Clone, Debug, PartialEq)]
pub struct GridSquareMarker {
    /// `getOriginalX/Y/Z`: the square it was placed on.
    pub orig: [f32; 3],
    /// `getZ()`, the level it draws on.
    pub z: f32,
    /// `getSize()`.
    pub size: f32,
    /// `getR/G/B`.
    pub rgb: [f32; 3],
    /// `getAlpha()`: the fading alpha.
    pub alpha: f32,
    pub active: bool,
    /// `getTextureName()` / `getOverlayTextureName()`.
    pub texture_name: String,
    pub overlay_texture_name: Option<String>,
}

/// An `IsoMarkers.IsoMarker` (owner: the Lua callers of
/// `getIsoMarkers():addIsoMarker`).
#[derive(Clone, Debug, Default)]
pub struct IsoMarker {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `zLayer`.
    pub z_layer: i32,
    pub rgba: [f32; 4],
    pub active: bool,
    /// `circleSize`: a `LineDrawer.DrawIsoCircle` of 32 segments when > 0.
    pub circle_size: f32,
    pub textures: Vec<Texture>,
    /// With an item: `WorldItemModelDrawer.renderMain(item, ...)`'s draws
    /// when the item has a model (the textures are then skipped); `None`
    /// when it has none (owner: the item model renderer).
    pub item_model: Option<Vec<DrawCmd>>,
}

/// One frame's markers for the viewing player.
#[derive(Clone, Debug, Default)]
pub struct MarkersFrame {
    pub grid_square_markers: Vec<GridSquareMarker>,
    /// `media/textures/worldMap/circle_center.png` and
    /// `circle_only_highlight.png`.
    pub circle_center: Option<Texture>,
    pub circle_only_highlight: Option<Texture>,
    /// What an untextured `VBORenderer` run samples when no texture was
    /// bound before it (`Texture.getWhite()`).
    pub white: Option<Texture>,
    /// The chunk map's world tile bounds (min x, min y, max x, max y);
    /// `None` when the chunk map is ignored. Owner: the cell's chunk map.
    pub chunk_map: Option<[i32; 4]>,
    pub iso_markers: Vec<IsoMarker>,
    /// `IsoPlayer.players[playerIndex]`'s z, `None` without a player.
    pub player_z: Option<f32>,
    /// `LineDrawer` lines the markers queue this frame (for
    /// [`crate::area_highlights::render_lines`]).
    pub lines: Vec<Line>,
}

/// One `FBORenderWorldMarkers.Marker`.
#[derive(Clone, Debug, PartialEq)]
struct Marker {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    z: f32,
    rgba: [f32; 4],
    texture1: Option<Texture>,
    texture2: Option<Texture>,
}

impl MarkersFrame {
    /// The levels holding markers (the frame's level loop covers them).
    pub fn levels(&self) -> Vec<i32> {
        let grid = self
            .grid_square_markers
            .iter()
            .filter(|m| m.active && m.z == m.z.trunc())
            .map(|m| m.z as i32);
        grid.chain(self.iso_markers.iter().map(|m| m.z_layer))
            .collect()
    }

    /// `FBORenderWorldMarkers.render(z, markers)`'s markers on screen.
    fn grid_markers(&self, z: i32) -> Vec<Marker> {
        let Some(m) = self.chunk_map else {
            return Vec::new();
        };
        let mut v = Vec::new();
        for g in &self.grid_square_markers {
            if !g.active || g.z != z as f32 {
                continue;
            }
            let cx = g.orig[0] + 0.5;
            let cy = g.orig[1] + 0.5;
            let radius = g.size * 0.69;
            let mut mk = Marker {
                x1: cx - radius,
                y1: cy - radius,
                x2: cx + radius,
                y2: cy + radius,
                z: g.orig[2],
                rgba: [g.rgb[0], g.rgb[1], g.rgb[2], g.alpha],
                texture1: None,
                texture2: None,
            };
            match g.texture_name.as_str() {
                "circle_center" | "circle_highlight_2" => mk.texture1 = self.circle_center.clone(),
                "circle_highlight" => {
                    mk.texture1 = self.circle_center.clone();
                    mk.texture2 = self.circle_only_highlight.clone();
                }
                _ => {}
            }
            if matches!(
                g.overlay_texture_name.as_deref(),
                Some("circle_only_highlight" | "circle_only_highlight_2")
            ) {
                mk.texture2 = self.circle_only_highlight.clone();
            }
            if mk.x1 < m[2] as f32
                && mk.x2 > m[0] as f32
                && mk.y1 < m[3] as f32
                && mk.y2 > m[1] as f32
            {
                v.push(mk);
            }
        }
        v
    }

    /// `WorldMarkers.renderGridSquareMarkers(z)` then
    /// `IsoMarkers.renderIsoMarkers(z)`.
    pub fn render_level(
        &mut self,
        r: &ChunkRenderer<'_>,
        mc: &ModelCamera,
        z: i32,
        out: &mut Vec<DrawCmd>,
    ) {
        self.render_grid_square_markers(mc, z, out);
        self.render_iso_markers(r, z, out);
    }

    /// `FBORenderWorldMarkers.Drawer.render`: each marker's two quads
    /// (texture 1 then texture 2) on the floor, depth-tested `LEQUAL`
    /// without writes, at the corners' square depths less `1.5e-4`. A quad
    /// without its texture samples the texture bound before it (the first
    /// quad's, else white).
    pub fn render_grid_square_markers(&self, mc: &ModelCamera, z: i32, out: &mut Vec<DrawCmd>) {
        let markers = self.grid_markers(z);
        if markers.is_empty() {
            return;
        }
        out.extend([
            DrawCmd::GlEnable(gl::DEPTH_TEST),
            DrawCmd::DepthFunc(gl::LEQUAL),
            DrawCmd::DepthMask(false),
            DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA),
        ]);
        let mut bound = self.white.clone();
        let (px, py) = (fastfloor(mc.cam.x), fastfloor(mc.cam.y));
        for m in &markers {
            let adjust: f32 = -1.0e-4f32 - 5.0e-5f32;
            let corners = [[m.x1, m.y1], [m.x2, m.y1], [m.x2, m.y2], [m.x1, m.y2]];
            let depths = corners.map(|[x, y]| iso::square_depth(px, py, x, y, m.z) + adjust);
            let o = [(m.x1 + m.x2) / 2.0, (m.y1 + m.y2) / 2.0, m.z];
            for t in [&m.texture1, &m.texture2] {
                if t.is_some() {
                    bound = t.clone();
                }
                if let Some(tex) = bound.clone() {
                    out.push(floor_quad(mc, o, corners, depths, m.rgba, tex));
                }
            }
        }
        out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    }

    /// `IsoMarkers.renderIsoMarkers(z)` (with `renderTranslucentOnly`):
    /// every marker's circle is queued for `LineDrawer` (on every level,
    /// as the original does), then the markers of this level on the
    /// player's level draw their item model or textures.
    pub fn render_iso_markers(&mut self, r: &ChunkRenderer<'_>, z: i32, out: &mut Vec<DrawCmd>) {
        let Some(player_z) = self.player_z else {
            return;
        };
        if self.iso_markers.is_empty() {
            return;
        }
        out.push(DrawCmd::GlEnable(gl::DEPTH_TEST));
        out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
        for m in &self.iso_markers {
            if m.circle_size > 0.0 {
                iso_circle(m.x, m.y, m.z, m.circle_size, 32, m.rgba, &mut self.lines);
            }
            if m.z_layer != z || fastfloor(m.z) != fastfloor(player_z) || !m.active {
                continue;
            }
            for t in &m.textures {
                if let Some(draws) = &m.item_model {
                    out.extend(draws.iter().cloned());
                    continue;
                }
                let [cr, cg, cb, ca] = m.rgba;
                render_texture_with_depth(
                    &r.cam,
                    t,
                    t.width as f32,
                    t.height as f32,
                    [cr, cg, cb, ca],
                    m.x,
                    m.y,
                    m.z,
                    out,
                );
            }
        }
    }
}

/// `IsoSprite.renderTextureWithDepth(texture, w, h, r, g, b, a, x, y, z)`
/// in the per-frame passes: the texture centred above `(x, y, z)` with the
/// default shader at that point's depth plus its chunk's.
#[allow(clippy::too_many_arguments)]
pub fn render_texture_with_depth(
    cam: &Camera,
    tex: &Texture,
    w: f32,
    h: f32,
    rgba: [f32; 4],
    x: f32,
    y: f32,
    z: f32,
    out: &mut Vec<DrawCmd>,
) {
    let x = x + cam.fix_jiggly_square_x;
    let y = y + cam.fix_jiggly_square_y;
    let chunk = iso::chunk_depth_data(
        cam.chunk_x(),
        cam.chunk_y(),
        fastfloor(x / 8.0),
        fastfloor(y / 8.0),
        fastfloor(z),
    )
    .depth_start;
    let chunk = (chunk + 0.5) * 2.0 - 1.0;
    let next_z = iso::calculate_depth(x, y, z) * 2.0 - 1.0;
    let sx = iso::x_to_screen(x, y) - cam.off_x as f32 - w / 2.0;
    let sy = iso::y_to_screen(x, y, z, 0) - cam.off_y as f32 - h;
    let mut dl = DrawList::default();
    let [r, g, b, a] = rgba;
    dl.render(Some(tex), sx, sy, w, h, Rgba(r, g, b, a));
    for c in dl.cmds {
        if let DrawCmd::Sprite(s) = c {
            out.push(DrawCmd::WorldSprite(
                s,
                WorldShader::Default { z: chunk + next_z },
            ));
        }
    }
}

/// A `WorldMarkers` homing point or direction arrow as `update` leaves it
/// for `renderHomingPoint` / `renderDirectionArrow` (owner: the Lua
/// callers and `WorldMarkers.update`'s screen-edge placement).
#[derive(Clone, Debug)]
pub struct DirectionArrow {
    pub texture: Texture,
    pub render_width: f32,
    pub render_height: f32,
    /// `renderScreenX/Y` (homing points: `renderX/Y`).
    pub screen: [f32; 2],
    /// The angle `DrawTextureAngle` turns it by (`180 - angle`, wrapped
    /// into 0..360; 0 for an arrow without `renderWithAngle`).
    pub angle: f64,
    pub rgba: [f32; 4],
    pub render_size_mod: f32,
}

/// `WorldMarkers.DrawTextureAngle`: the texture centred on `centre`,
/// turned by `angle` degrees, as the four corners `SpriteRenderer.render`
/// takes (in `double`, as the original).
pub fn draw_texture_angle(a: &DirectionArrow, out: &mut DrawList) {
    let dx = a.render_width * a.render_size_mod / 2.0;
    let dy = a.render_height * a.render_size_mod / 2.0;
    let radian = (180.0 + a.angle).to_radians();
    let (x_cos, x_sin) = (radian.cos() * dx as f64, radian.sin() * dx as f64);
    let (y_cos, y_sin) = (radian.cos() * dy as f64, radian.sin() * dy as f64);
    let (cx, cy) = (a.screen[0] as f64, a.screen[1] as f64);
    let xs = [
        x_cos - y_sin + cx,
        -x_cos - y_sin + cx,
        -x_cos + y_sin + cx,
        x_cos + y_sin + cx,
    ];
    let ys = [
        y_cos + x_sin + cy,
        y_cos - x_sin + cy,
        -y_cos - x_sin + cy,
        -y_cos + x_sin + cy,
    ];
    let [r, g, b, al] = a.rgba;
    out.render4(
        Some(&a.texture),
        xs.map(|v| v as f32),
        ys.map(|v| v as f32),
        Rgba(r, g, b, al),
    );
}

/// `WorldMarkers.render`'s homing points then direction arrows (not drawn
/// on the world) in `renderframetext`, screen space; a homing point's
/// alpha is raised towards 1 by the fog (`ClimateManager.getFogIntensity`).
pub fn render_frame_text(
    homing_points: &[DirectionArrow],
    fog_intensity: f32,
    arrows: &[DirectionArrow],
    out: &mut DrawList,
) {
    for p in homing_points {
        let mut p = p.clone();
        let mut alpha = p.rgba[3];
        if fog_intensity > 0.0 && alpha < 1.0 {
            alpha += (1.0 - alpha) * fog_intensity * 2.0;
            alpha = alpha.clamp(0.0, 1.0);
        }
        p.rgba[3] = alpha;
        draw_texture_angle(&p, out);
    }
    for a in arrows {
        draw_texture_angle(a, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{DepthMaps, DepthPreset};

    struct NoMaps;
    impl DepthMaps for NoMaps {
        fn preset(&self, _: DepthPreset) -> Option<Texture> {
            None
        }
        fn default_texture(&self) -> Option<Texture> {
            None
        }
    }

    fn tex(name: &str, id: u32) -> Texture {
        let mut t = Texture::empty(name);
        t.id = Some(id);
        t.width = 64;
        t.height = 64;
        t.ready = true;
        t
    }

    fn marker(name: &str, z: f32) -> GridSquareMarker {
        GridSquareMarker {
            orig: [10.0, 12.0, z],
            z,
            size: 1.0,
            rgb: [1.0, 0.5, 0.0],
            alpha: 0.8,
            active: true,
            texture_name: name.into(),
            overlay_texture_name: None,
        }
    }

    #[test]
    fn grid_square_markers_two_quads_each() {
        let cam = Camera::centred(10.0, 10.0, 0.0, 800, 600, 1.0);
        let mc = ModelCamera::new(&cam, 800.0, 600.0);
        let f = MarkersFrame {
            grid_square_markers: vec![
                marker("circle_highlight", 0.0),
                marker("circle_center", 1.0),
                marker("circle_center", 0.0),
            ],
            circle_center: Some(tex("cc", 1)),
            circle_only_highlight: Some(tex("coh", 2)),
            white: Some(tex("white", 3)),
            chunk_map: Some([0, 0, 100, 100]),
            ..Default::default()
        };
        let mut out = Vec::new();
        f.render_grid_square_markers(&mc, 0, &mut out);
        let ids: Vec<_> = out
            .iter()
            .filter_map(|c| match c {
                DrawCmd::ProjectedDepthQuad { texture, pos, .. } => Some((texture.id, pos[0][2])),
                _ => None,
            })
            .collect();
        // circle_highlight: centre then ring; circle_center: the centre
        // twice (its second run samples the still-bound centre).
        assert_eq!(
            ids.iter().map(|i| i.0).collect::<Vec<_>>(),
            [Some(1), Some(2), Some(1), Some(1)]
        );
        let d = iso::square_depth(10, 10, 10.5 - 0.69, 12.5 - 0.69, 0.0) + (-1.0e-4f32 - 5.0e-5f32);
        assert_eq!(ids[0].1, d);
        assert!(f.levels().contains(&1));
    }

    #[test]
    fn iso_markers_queue_circles_every_level_and_draw_on_theirs() {
        let cam = Camera::centred(10.0, 10.0, 0.0, 800, 600, 1.0);
        let r = ChunkRenderer::new(cam, &NoMaps, (800, 600));
        let mut f = MarkersFrame {
            iso_markers: vec![IsoMarker {
                x: 10.5,
                y: 10.5,
                z: 0.0,
                z_layer: 0,
                rgba: [1.0; 4],
                active: true,
                circle_size: 1.0,
                textures: vec![tex("m", 4)],
                item_model: None,
            }],
            player_z: Some(0.0),
            ..Default::default()
        };
        let mut out = Vec::new();
        f.render_iso_markers(&r, 0, &mut out);
        f.render_iso_markers(&r, 1, &mut out);
        assert_eq!(f.lines.len(), 64);
        let s: Vec<_> = out
            .iter()
            .filter(|c| matches!(c, DrawCmd::WorldSprite(..)))
            .collect();
        assert_eq!(s.len(), 1);
        let DrawCmd::WorldSprite(sp, WorldShader::Default { z }) = s[0] else {
            panic!()
        };
        assert_eq!(
            sp.x[0],
            iso::x_to_screen(10.5, 10.5) - cam.off_x as f32 - 32.0
        );
        let chunk = (iso::chunk_depth_data(1, 1, 1, 1, 0).depth_start + 0.5) * 2.0 - 1.0;
        assert_eq!(
            *z,
            chunk + iso::calculate_depth(10.5, 10.5, 0.0) * 2.0 - 1.0
        );
    }

    #[test]
    fn opaque_event_at_picked_tile_or_player() {
        let cam = Camera::centred(10.7, 11.2, 0.0, 800, 600, 1.0);
        let mut got = Vec::new();
        let mut h = |x: i32, y: i32, z: i32, _: &mut Vec<DrawCmd>| got.push((x, y, z));
        let valid = |_: i32, _: i32, _: i32| true;
        let mut e = OpaqueObjectsEvent {
            joypad: false,
            picked_tile: Some([3.5, 4.2]),
            valid_square: &valid,
            handler: &mut h,
        };
        e.trigger(&cam, &mut Vec::new());
        e.joypad = true;
        e.trigger(&cam, &mut Vec::new());
        e.joypad = false;
        e.picked_tile = None;
        e.trigger(&cam, &mut Vec::new());
        assert_eq!(got, [(3, 4, 0), (10, 11, 0)]);
    }

    #[test]
    fn texture_angle_corners() {
        let a = DirectionArrow {
            texture: tex("arrow", 5),
            render_width: 20.0,
            render_height: 10.0,
            screen: [100.0, 50.0],
            angle: 0.0,
            rgba: [1.0; 4],
            render_size_mod: 1.0,
        };
        let mut dl = DrawList::default();
        draw_texture_angle(&a, &mut dl);
        let DrawCmd::Sprite(s) = &dl.cmds[0] else {
            panic!()
        };
        // 180 degrees: the corners swap sides.
        assert!((s.x[0] - 90.0).abs() < 1e-4 && (s.y[0] - 45.0).abs() < 1e-4);
        assert!((s.x[2] - 110.0).abs() < 1e-4 && (s.y[2] - 55.0).abs() < 1e-4);
    }
}
