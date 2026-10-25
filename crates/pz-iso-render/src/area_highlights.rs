//! `FBORenderAreaHighlights` (drawn by `IsoWorld.renderInternal` after the
//! weather) and `LineDrawer.render`, which follows it: translucent
//! rectangles laid on the floor under the model camera, depth-tested
//! against the world, and the frame's queued debug/outline lines.

use pz_render::iso::{self, fastfloor, TILE_SCALE};
use pz_render::{gl, DrawCmd, DrawList, Rgba, Texture};

use crate::models::ModelCamera;
use crate::shadows::Shadow;
use crate::Camera;

/// A `LineDrawer.DrawableLine` queued this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// `addLine(x, y, z, x2, y2, z2, r, g, b, a)`: drawn by `DrawIsoLine`.
    Line {
        from: [f32; 3],
        to: [f32; 3],
        rgba: [f32; 4],
    },
    /// `addRect(x, y, z, width, height, r, g, b)` /
    /// `addRectYOffset`: an iso rectangle outline (`DrawIsoRect`), opaque.
    Rect {
        x: f32,
        y: f32,
        z: f32,
        width: f32,
        height: f32,
        y_pixel_offset: i32,
        rgb: [f32; 3],
    },
}

/// `LineDrawer.DrawIsoCircle(x, y, z, radius, segments, r, g, b, a)`: the
/// circle's chords queued as lines (angles in `double`, as the original).
pub fn iso_circle(
    x: f32,
    y: f32,
    z: f32,
    radius: f32,
    segments: i32,
    rgba: [f32; 4],
    lines: &mut Vec<Line>,
) {
    let (xd, yd, rd) = (x as f64, y as f64, radius as f64);
    let mut lx = xd + rd * (0.0f64 / segments as f64).to_radians().cos();
    let mut ly = yd + rd * (0.0f64 / segments as f64).to_radians().sin();
    for i in 1..=segments {
        let a = (i as f64 * 360.0 / segments as f64).to_radians();
        let cx = xd + rd * a.cos();
        let cy = yd + rd * a.sin();
        lines.push(Line::Line {
            from: [lx as f32, ly as f32, z],
            to: [cx as f32, cy as f32, z],
            rgba,
        });
        lx = cx;
        ly = cy;
    }
}

/// `LineDrawer.render()`: every queued line, plain sprite shader, no depth
/// test, blended (SRC_ALPHA, ONE_MINUS_SRC_ALPHA), one pixel thick.
pub fn render_lines(cam: &Camera, lines: &[Line], out: &mut Vec<DrawCmd>) {
    if lines.is_empty() {
        return;
    }
    out.push(DrawCmd::GlDisable(gl::DEPTH_TEST));
    out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    let mut dl = DrawList::default();
    let (jx, jy) = (cam.fix_jiggly_x * cam.zoom, cam.fix_jiggly_y * cam.zoom);
    let sx = |x: f32, y: f32| iso::x_to_screen(x, y) - cam.off_x as f32;
    let sy = |x: f32, y: f32, z: f32| iso::y_to_screen(x, y, z, 0) - cam.off_y as f32;
    for l in lines {
        match *l {
            Line::Line {
                from: [x, y, z],
                to: [x2, y2, z2],
                rgba: [r, g, b, a],
            } => {
                // DrawIsoLine(x, y, z, x2, y2, z2, r, g, b, a, 1).
                dl.render_line(
                    None,
                    sx(x, y) + jx,
                    sy(x, y, z) + jy,
                    sx(x2, y2) + jx,
                    sy(x2, y2, z2) + jy,
                    Rgba(r, g, b, a),
                    1.0,
                );
            }
            Line::Rect {
                x,
                y,
                z,
                width,
                height,
                y_pixel_offset,
                rgb: [r, g, b],
            } => {
                // DrawIsoRect(x, y, width, height, (int) z, yPixelOffset, r, g, b).
                let (mut x, mut y, mut w, mut h) = (x, y, width, height);
                if w < 0.0 {
                    w = -w;
                    x -= w;
                }
                if h < 0.0 {
                    h = -h;
                    y -= h;
                }
                let z = z as i32 as f32;
                let f = (-y_pixel_offset * TILE_SCALE) as f32;
                let p = |px: f32, py: f32| (sx(px, py) + jx, sy(px, py, z) + f + jy);
                let (a, b2, c, d) = (p(x, y), p(x + w, y), p(x + w, y + h), p(x, y + h));
                let col = Rgba(r, g, b, 1.0);
                for (s, e) in [(a, b2), (b2, c), (c, d), (d, a)] {
                    dl.render_line(None, s.0, s.1, e.0, e.1, col, 1.0);
                }
            }
        }
    }
    out.extend(dl.cmds);
}

/// One `FBORenderAreaHighlights.AreaHighlight`: tiles `x1..x2`, `y1..y2`
/// on level `z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaHighlight {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    pub z: i32,
    pub rgba: [f32; 4],
}

impl AreaHighlight {
    fn is_on_screen(&self, m: [i32; 4]) -> bool {
        self.x1 < m[2] && self.x2 > m[0] && self.y1 < m[3] && self.y2 > m[1]
    }

    fn clamp_to_chunk_map(&mut self, m: [i32; 4]) {
        self.x1 = self.x1.max(m[0]);
        self.y1 = self.y1.max(m[1]);
        self.x2 = self.x2.min(m[2]);
        self.y2 = self.y2.min(m[3]);
    }

    /// `AreaHighlight.render`: the rectangle as one floor quad about its
    /// centre, each corner at its square depth less `1e-4` (`1.4e-4`
    /// behind the camera character).
    pub fn quad(&self, mc: &ModelCamera, white: &Texture) -> DrawCmd {
        let (px, py) = (mc.cam.x, mc.cam.y);
        let ox = (self.x1 + self.x2) as f32 / 2.0;
        let oy = (self.y1 + self.y2) as f32 / 2.0;
        let adjust = if ox + oy < px + py { -1.4e-4 } else { -1.0e-4 };
        let (x1, y1, x2, y2, z) = (
            self.x1 as f32,
            self.y1 as f32,
            self.x2 as f32,
            self.y2 as f32,
            self.z as f32,
        );
        let corners = [[x1, y1], [x2, y1], [x2, y2], [x1, y2]];
        let depths =
            corners.map(|[x, y]| iso::square_depth(fastfloor(px), fastfloor(py), x, y, z) + adjust);
        floor_quad(mc, [ox, oy, z], corners, depths, self.rgba, white.clone())
    }
}

/// A quad in the floor plane about `o` under the model camera
/// (`scaling(Core.scale) .. translate(0, -0.72, 0)`, the same model-view as
/// `FBORenderShadows`), with per-corner window depths.
pub fn floor_quad(
    mc: &ModelCamera,
    o: [f32; 3],
    corners: [[f32; 2]; 4],
    depths: [f32; 4],
    rgba: [f32; 4],
    texture: Texture,
) -> DrawCmd {
    let s = Shadow {
        ox: o[0],
        oy: o[1],
        oz: o[2],
        corners,
        slope_angle_x: 0.0,
        slope_angle_y: 0.0,
        depths,
        rgba,
        texture,
        vehicle: false,
    };
    s.quad(mc)
}

/// `animals.DesignationZone`: an animal designation zone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesignationZone {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub z: i32,
    /// Its id is in `player.getSelectedZonesForHighlight()`.
    pub selected: bool,
}

/// One frame's area highlights for the viewing player.
#[derive(Clone, Debug, Default)]
pub struct AreaHighlightsFrame {
    /// `Texture.getWhite()`; without it no rectangles are drawn (the
    /// outlines still are).
    pub white: Option<Texture>,
    pub player_index: i32,
    /// The chunk map's world tile bounds (`getWorldXMinTiles`,
    /// `getWorldYMinTiles`, `getWorldXMaxTiles`, `getWorldYMaxTiles`);
    /// `None` when the chunk map is ignored (nothing drawn). Owner: the
    /// cell's chunk map.
    pub chunk_map: Option<[i32; 4]>,
    /// `addHighlight` / `addHighlightForPlayer` calls kept for this UI
    /// frame (`renderTimeMs == UIManager.uiRenderTimeMS`): the player they
    /// are for (`None`: every player) and the area. Owner: the Lua/UI code
    /// calling `getAreaHighlights():addHighlight...`.
    pub user_defined: Vec<(Option<i32>, AreaHighlight)>,
    /// `NonPvpZone.getAllZones()` bounds (x, y, x2, y2), filled only on a
    /// client whose player `isSeeNonPvpZone()`. Owner: the network client.
    pub non_pvp_zones: Vec<[i32; 4]>,
    /// `SafeHouse.getSafehouseList()` bounds (x, y, x2, y2), filled only
    /// on a debug client. Owner: the network client.
    pub safehouses: Vec<[i32; 4]>,
    /// `DesignationZone.allZones`, filled only while the player
    /// `isSeeDesignationZone()`. Owner: the animal zones.
    pub designation_zones: Vec<DesignationZone>,
}

impl AreaHighlightsFrame {
    /// The frame's highlights in `render`'s order (user areas, non-PvP
    /// zones, safehouses, designation zones), each queuing its outline
    /// (`LineDrawer.addRect`, before clamping) and clamped to the chunk
    /// map.
    pub fn highlights(&self, lines: &mut Vec<Line>) -> Vec<AreaHighlight> {
        let Some(m) = self.chunk_map else {
            return Vec::new();
        };
        let mut all: Vec<AreaHighlight> = self
            .user_defined
            .iter()
            .filter(|(p, _)| p.is_none_or(|p| p == self.player_index))
            .map(|(_, a)| *a)
            .collect();
        let zone = |b: &[i32; 4], z: i32, rgba: [f32; 4]| AreaHighlight {
            x1: b[0],
            y1: b[1],
            x2: b[2],
            y2: b[3],
            z,
            rgba,
        };
        all.extend(
            self.non_pvp_zones
                .iter()
                .map(|b| zone(b, 0, [0.0, 0.0, 1.0, 0.25])),
        );
        all.extend(
            self.safehouses
                .iter()
                .map(|b| zone(b, 0, [1.0, 0.0, 0.0, 0.25])),
        );
        all.extend(self.designation_zones.iter().map(|d| {
            let rgba = if d.selected {
                [0.2, 0.8, 0.9, 0.4]
            } else {
                [0.2, 0.2, 0.9, 0.4]
            };
            zone(&[d.x, d.y, d.x + d.w, d.y + d.h], d.z, rgba)
        }));
        let mut keep = Vec::new();
        for mut a in all {
            if !a.is_on_screen(m) {
                continue;
            }
            let [r, g, b, _] = a.rgba;
            lines.push(Line::Rect {
                x: a.x1 as f32,
                y: a.y1 as f32,
                z: a.z as f32,
                width: (a.x2 - a.x1) as f32,
                height: (a.y2 - a.y1) as f32,
                y_pixel_offset: 0,
                rgb: [r, g, b],
            });
            a.clamp_to_chunk_map(m);
            keep.push(a);
        }
        keep
    }

    /// `FBORenderAreaHighlights.render()`: the rectangles drawn with the
    /// depth test on (`LEQUAL`), no depth writes, blended (SRC_ALPHA,
    /// ONE_MINUS_SRC_ALPHA); the outlines are added to `lines` for
    /// `LineDrawer.render`.
    pub fn render(
        &self,
        _cam: &Camera,
        mc: &ModelCamera,
        lines: &mut Vec<Line>,
        out: &mut Vec<DrawCmd>,
    ) {
        let highlights = self.highlights(lines);
        let Some(white) = self.white.as_ref() else {
            return;
        };
        if highlights.is_empty() {
            return;
        }
        out.extend([
            DrawCmd::GlEnable(gl::DEPTH_TEST),
            DrawCmd::DepthFunc(gl::LEQUAL),
            DrawCmd::DepthMask(false),
            DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA),
        ]);
        out.extend(highlights.iter().map(|a| a.quad(mc, white)));
        // GLStateRenderThread.restore.
        out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_closes_and_rect_outline_is_four_lines() {
        let mut lines = Vec::new();
        iso_circle(10.0, 10.0, 0.0, 2.0, 32, [1.0; 4], &mut lines);
        assert_eq!(lines.len(), 32);
        let Line::Line { from, .. } = &lines[0] else {
            panic!()
        };
        let Line::Line { to, .. } = &lines[31] else {
            panic!()
        };
        assert!((from[0] - to[0]).abs() < 1e-5 && (from[1] - to[1]).abs() < 1e-5);
        let cam = Camera::centred(10.0, 10.0, 0.0, 800, 600, 1.0);
        let mut out = Vec::new();
        render_lines(
            &cam,
            &[Line::Rect {
                x: 9.0,
                y: 9.0,
                z: 0.0,
                width: 2.0,
                height: 2.0,
                y_pixel_offset: 0,
                rgb: [1.0, 0.0, 0.0],
            }],
            &mut out,
        );
        assert_eq!(
            out.iter()
                .filter(|c| matches!(c, DrawCmd::Sprite(_)))
                .count(),
            4
        );
    }

    #[test]
    fn highlights_filter_outline_then_clamp() {
        let a = AreaHighlight {
            x1: -5,
            y1: 2,
            x2: 4,
            y2: 6,
            z: 0,
            rgba: [1.0, 1.0, 0.0, 0.5],
        };
        let f = AreaHighlightsFrame {
            chunk_map: Some([0, 0, 100, 100]),
            player_index: 0,
            user_defined: vec![(Some(1), a), (None, a)],
            designation_zones: vec![DesignationZone {
                x: 200,
                y: 0,
                w: 2,
                h: 2,
                z: 0,
                selected: false,
            }],
            ..Default::default()
        };
        let mut lines = Vec::new();
        let h = f.highlights(&mut lines);
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].x1, h[0].x2), (0, 4));
        assert_eq!(
            lines,
            vec![Line::Rect {
                x: -5.0,
                y: 2.0,
                z: 0.0,
                width: 9.0,
                height: 4.0,
                y_pixel_offset: 0,
                rgb: [1.0, 1.0, 0.0]
            }]
        );
    }
}
