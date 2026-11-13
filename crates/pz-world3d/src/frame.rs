//! One player's 3D world frame, in the same frame structure as the
//! isometric one (`world_frame_full`): the scene is drawn into the
//! offscreen buffer, then the buffer goes to the screen through the
//! weather `screen` pass, so night, fog tint and the other screen effects
//! still apply.

use pz_iso_render::frame::FrameSize;
use pz_render::{gl, DrawCmd, Rgba, ScreenParams};

use crate::mesh::Face;

/// Port extension: what one 3D frame is drawn from.
pub struct Scene<'a> {
    /// The player's view (the screen size, not times zoom).
    pub size: FrameSize,
    /// `projection * view`, column-major GL clip space, model space around
    /// the camera character.
    pub view_proj: [[f32; 4]; 4],
    /// The sky colour behind everything.
    pub sky: [f32; 4],
    pub faces: &'a [Face],
    /// Character and vehicle draws (only [`DrawCmd::Model`] and
    /// [`DrawCmd::VehicleModel`] are kept) made with a perspective
    /// `ModelCamera`.
    pub models: &'a [DrawCmd],
    /// The player's `WeatherShader` inputs.
    pub screen: ScreenParams,
    /// Draw a crosshair at the centre (mouse-look on: what the mouse acts
    /// on).
    pub crosshair: bool,
}

/// A small plus at the centre of a `w` x `h` view: a dark outline under a
/// light cross.
pub fn crosshair(w: f32, h: f32) -> Vec<DrawCmd> {
    let (cx, cy) = ((w / 2.0).floor(), (h / 2.0).floor());
    let bar = |x: f32, y: f32, bw: f32, bh: f32, c: Rgba| DrawCmd::Quad {
        texture: None,
        x,
        y,
        w: bw,
        h: bh,
        color: c,
    };
    let dark = Rgba(0.0, 0.0, 0.0, 0.6);
    let light = Rgba(1.0, 1.0, 1.0, 0.85);
    let (arm, t) = (7.0, 2.0);
    vec![
        bar(cx - arm - 1.0, cy - t / 2.0 - 1.0, 2.0 * arm + 2.0, t + 2.0, dark),
        bar(cx - t / 2.0 - 1.0, cy - arm - 1.0, t + 2.0, 2.0 * arm + 2.0, dark),
        bar(cx - arm, cy - t / 2.0, 2.0 * arm, t, light),
        bar(cx - t / 2.0, cy - arm, t, 2.0 * arm, light),
    ]
}

/// The sky colour for an ambient light level (0 = night, 1 = full day).
pub fn sky_colour(ambient: f32) -> [f32; 4] {
    let a = ambient.clamp(0.0, 1.0);
    [0.08 + 0.42 * a, 0.10 + 0.52 * a, 0.14 + 0.66 * a, 1.0]
}

/// A face as a [`DrawCmd::Quad3D`].
pub fn face_cmd(f: &Face) -> DrawCmd {
    DrawCmd::Quad3D {
        texture: f.texture.clone(),
        pos: f.pos,
        uv: f.uv,
        color: f.color,
    }
}

/// The draw list of a 3D frame.
pub fn scene_frame(s: &Scene<'_>) -> Vec<DrawCmd> {
    let (w, h) = (s.size.width, s.size.height);
    let (ow, oh) = s.size.offscreen();
    let mut out = vec![
        DrawCmd::OffscreenBegin {
            width: ow,
            height: oh,
        },
        DrawCmd::GlEnable(gl::BLEND),
        DrawCmd::Ortho {
            left: 0.0,
            right: w as f32,
            bottom: h as f32,
            top: 0.0,
            width: w,
            height: h,
        },
        DrawCmd::ColorMask(true, true, true, true),
        DrawCmd::DepthMask(true),
        DrawCmd::StencilMask(0xFF),
        DrawCmd::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT | gl::STENCIL_BUFFER_BIT),
        DrawCmd::Scene3DBegin {
            view_proj: s.view_proj,
            clear: Some(s.sky),
        },
    ];
    out.extend(s.faces.iter().map(face_cmd));
    out.extend(
        s.models
            .iter()
            .filter(|c| matches!(c, DrawCmd::Model(_) | DrawCmd::VehicleModel(_)))
            .cloned(),
    );
    out.extend([
        DrawCmd::Scene3DEnd,
        DrawCmd::GlDisable(gl::DEPTH_TEST),
        DrawCmd::OffscreenEnd,
        DrawCmd::Ortho {
            left: 0.0,
            right: w as f32,
            bottom: h as f32,
            top: 0.0,
            width: w,
            height: h,
        },
        DrawCmd::ScreenPass {
            x: 0.0,
            y: 0.0,
            w: w as f32,
            h: h as f32,
            tex: [0.0, 0.0, w as f32, h as f32],
            params: s.screen,
        },
        DrawCmd::GlEnable(gl::BLEND),
    ]);
    if s.crosshair {
        out.extend(crosshair(w as f32, h as f32));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_shape() {
        let models = [DrawCmd::GlEnable(gl::BLEND)];
        let s = Scene {
            size: FrameSize {
                width: 800,
                height: 600,
            },
            view_proj: crate::Mat4::identity().0,
            sky: sky_colour(1.0),
            faces: &[],
            models: &models,
            screen: ScreenParams::default(),
            crosshair: false,
        };
        let cmds = scene_frame(&s);
        assert!(matches!(
            cmds[0],
            DrawCmd::OffscreenBegin {
                width: 1024,
                height: 1024
            }
        ));
        assert!(cmds
            .iter()
            .any(|c| matches!(c, DrawCmd::Scene3DBegin { .. })));
        // State commands among the model draws are dropped.
        assert_eq!(
            cmds.iter()
                .filter(|c| matches!(c, DrawCmd::GlEnable(_)))
                .count(),
            2
        );
        assert!(matches!(cmds[cmds.len() - 2], DrawCmd::ScreenPass { .. }));
    }

    #[test]
    fn crosshair_is_centred() {
        let c = crosshair(800.0, 600.0);
        assert_eq!(c.len(), 4);
        for cmd in &c {
            let DrawCmd::Quad { x, y, w, h, .. } = cmd else {
                panic!("{cmd:?}");
            };
            assert!((x + w / 2.0 - 400.0).abs() < 1e-4);
            assert!((y + h / 2.0 - 300.0).abs() < 1e-4);
        }
    }

    #[test]
    fn sky_is_darker_at_night() {
        assert!(sky_colour(0.0)[2] < sky_colour(1.0)[2]);
    }
}
