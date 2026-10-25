//! `DeadBodyAtlas`: corpses (and prone bodies) posed once, rendered into
//! `ModelManager.bitmap` (1024x1024) with `AnimatedModel`'s UI camera,
//! copied into a 1024x1024 atlas with the `DeadBodyAtlas` shader (colour
//! and depth), then drawn every frame from the atlas
//! (`BodyTextureDepthDrawer`). This module has the render job's camera
//! and placement math (`calcMatrices`, `calcModelBounds`,
//! `calcModelOffset`, `calcEntrySize`), the copy (`toBodyAtlas`) and the
//! in-world draw (`IsoDeadBody.render`); the pose and model draws come
//! from the character code.

// The original writes the angles as float literals.
#![allow(clippy::approx_constant)]

use pz_render::iso::{x_to_screen, y_to_screen, TILE_SCALE};
use pz_render::{gl, DrawCmd};

use crate::atlas::{body_depth_draw, AtlasEntry};
use crate::joml::{intersect_ray_plane, normalize, Matrix4f};
use crate::Camera;

/// `DeadBodyAtlas.ATLAS_SIZE`.
pub const ATLAS_SIZE: i32 = 1024;
/// `ModelManager.bitmap`'s size.
pub const BITMAP_SIZE: i32 = 1024;
/// `RenderJob.SIZE_V`.
pub const SIZE_V: f32 = 42.75;

const VIEWPORT: [i32; 4] = [0, 0, BITMAP_SIZE, BITMAP_SIZE];

/// `RenderJob.calcMatrices(projection, modelView, offsetX, offsetY,
/// offsetZ)`: the flipped-y projection the bounds are measured with.
pub fn matrices(angle: f32, scale: f32, offset: [f32; 3]) -> (Matrix4f, Matrix4f) {
    // The bitmap's width over its height.
    let x_scale = 1.0f32;
    let f = 2048.0f32.sqrt();
    let p = Matrix4f::identity()
        .ortho(
            -SIZE_V * x_scale,
            SIZE_V * x_scale,
            SIZE_V,
            -SIZE_V,
            -100.0,
            100.0,
        )
        .scale(-f, f, f);
    let mv = Matrix4f::identity()
        .rotate(0.523_598_8, 1.0, 0.0, 0.0)
        .rotate(angle + 0.785_398_2, 0.0, 1.0, 0.0)
        .scale1(scale)
        .translate(offset[0], offset[1], offset[2]);
    (p, mv)
}

/// `AnimatedModel.UIModelCamera.Begin` for `DoRender(0, 0, 1024, 1024,
/// SIZE_V, angle)` (`flipY` off): the projection and model-view the
/// corpse's meshes are drawn into the bitmap with.
pub fn bitmap_camera(angle: f32, offset: [f32; 3]) -> (Matrix4f, Matrix4f) {
    // The bitmap's width over its height.
    let x_scale = 1.0f32;
    let f = 2048.0f32.sqrt();
    let p = Matrix4f::set_ortho(
        -SIZE_V * x_scale,
        SIZE_V * x_scale,
        -SIZE_V,
        SIZE_V,
        -100.0,
        100.0,
    )
    .scale(-f, f, f);
    let mv = Matrix4f::identity()
        .rotate(0.523_598_8, 1.0, 0.0, 0.0)
        .rotate(angle + 0.785_398_2, 0.0, 1.0, 0.0)
        .translate(offset[0], offset[1], offset[2]);
    (p, mv)
}

/// `sceneToUI`.
fn scene_to_ui(p: &Matrix4f, mv: &Matrix4f, v: [f32; 3]) -> [f32; 3] {
    p.mul(mv).project(v[0], v[1], v[2], VIEWPORT)
}

/// `uiToScene`: where the camera ray through window `(x, y)` meets the
/// model's floor plane (`y = 0`), or the origin.
fn ui_to_scene(p: &Matrix4f, mv: &Matrix4f, x: f32, y: f32) -> [f32; 3] {
    let inv = p.mul(mv).invert_affine();
    let start = inv.unproject_inv(x, y, 0.0, VIEWPORT);
    let end = inv.unproject_inv(x, y, 1.0, VIEWPORT);
    let dir = normalize([end[0] - start[0], end[1] - start[1], end[2] - start[2]]);
    intersect_ray_plane([0.0; 3], [0.0, 1.0, 0.0], start, dir).unwrap_or([0.0; 3])
}

/// `calcModelBounds` without a ragdoll pose: the window extent of the
/// bones' model-space positions (`Translation_Data` left out), at most
/// 1024 pixels each way.
pub fn model_bounds(p: &Matrix4f, mv: &Matrix4f, bones: &[[f32; 3]]) -> [f32; 4] {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, -f32::MAX, -f32::MAX);
    for b in bones {
        let s = scene_to_ui(p, mv, *b);
        min_x = min_x.min(s[0]);
        max_x = max_x.max(s[0]);
        min_y = min_y.min(s[1]);
        max_y = max_y.max(s[1]);
    }
    if max_x - min_x > 1024.0 {
        let excess = max_x - min_x - 1024.0;
        min_x += excess / 2.0;
        max_x -= excess / 2.0;
    }
    if max_y - min_y > 1024.0 {
        let excess = max_y - min_y - 1024.0;
        min_y += excess / 2.0;
        max_y -= excess / 2.0;
    }
    [min_x, min_y, max_x, max_y]
}

/// The placement `calcModelOffset` and `calcEntrySize` work out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyPlacement {
    /// The model offset the body is drawn with (`setOffsetWhileRendering`).
    pub offset: [f32; 3],
    /// `entry.offsetX/offsetY`.
    pub entry_offset: [f32; 2],
    pub entry_w: i32,
    pub entry_h: i32,
}

/// `calcModelOffset` then (after rendering) `calcEntrySize`, for a body
/// turned to `angle` (`getRenderedAngle`) with `scale` (`finalScale`) and
/// the bones' model-space positions.
pub fn placement(angle: f32, scale: f32, bones: &[[f32; 3]]) -> BodyPlacement {
    let w = BITMAP_SIZE as f32;
    let (p, mv) = matrices(angle, scale, [0.0; 3]);
    let [min_x, min_y, max_x, max_y] = model_bounds(&p, &mv, bones);
    let s = ui_to_scene(&p, &mv, min_x, min_y);
    let (scene_min_x, scene_min_y) = (s[0], s[2]);
    let ui_x = (w - (max_x - min_x)) / 2.0;
    let ui_y = (w - (max_y - min_y)) / 2.0;
    let s = ui_to_scene(&p, &mv, ui_x, ui_y);
    let offset = [
        (s[0] - scene_min_x) * scale,
        0.0,
        (s[2] - scene_min_y) * scale,
    ];
    let div = 8.0 / TILE_SCALE as f32;
    let entry_offset = [(ui_x - min_x) / div, (ui_y - min_y) / div];
    // calcEntrySize.
    let (p, mv) = matrices(angle, scale, offset);
    let [min_x, min_y, max_x, max_y] = model_bounds(&p, &mv, bones);
    let lo = |v: f32| ((v - 128.0) / 128.0).floor() * 128.0;
    let hi = |v: f32| ((v + 128.0) / 128.0).ceil() * 128.0;
    let idiv = 8 / TILE_SCALE;
    let entry_w = (((hi(max_x) - lo(min_x)) as i32) / idiv).min(1024);
    let entry_h = (((hi(max_y) - lo(min_y)) as i32) / idiv).min(1024);
    BodyPlacement {
        offset,
        entry_offset,
        entry_w,
        entry_h,
    }
}

/// `toBodyAtlas`: the bitmap's centre (a quarter of it at tile scale 2)
/// copied into the entry's cell with its depth.
pub fn to_body_atlas(
    e: &AtlasEntry,
    atlas_clear: bool,
    clear_slot: bool,
    bitmap: u64,
) -> Vec<DrawCmd> {
    let size = ATLAS_SIZE as f32;
    let w = BITMAP_SIZE / 8 * TILE_SCALE;
    let h = BITMAP_SIZE / 8 * TILE_SCALE;
    let x = (e.x - (w - e.w) / 2) as f32;
    let y = (e.y - (h - e.h) / 2) as f32;
    let (w, h) = (w as f32, h as f32);
    let mut out = vec![
        DrawCmd::AtlasBegin {
            id: e.atlas,
            width: ATLAS_SIZE as u32,
            height: ATLAS_SIZE as u32,
            clear: atlas_clear,
        },
        DrawCmd::Ortho {
            left: 0.0,
            right: size,
            bottom: size,
            top: 0.0,
            width: ATLAS_SIZE as u32,
            height: ATLAS_SIZE as u32,
        },
        DrawCmd::GlEnable(gl::DEPTH_TEST),
        DrawCmd::DepthFunc(gl::ALWAYS),
        DrawCmd::DepthMask(true),
        // toBodyAtlas sets no blend function: it runs from
        // IsoDeadBody.render inside renderCorpsesInWorld, after
        // FBORenderChunk.renderInWorldMainThread left blending on with
        // glBlendFuncSeparate(ONE, ONE_MINUS_SRC_ALPHA, ONE_MINUS_DST_ALPHA,
        // ONE). Into the cleared cell that copies the filtered bitmap texel
        // unchanged (premultiplied colour, coverage alpha); SRC_ALPHA
        // blending would square the alpha of every edge texel.
        DrawCmd::GlEnable(gl::BLEND),
        DrawCmd::BlendFuncSeparate(gl::ONE, gl::ONE_MINUS_SRC_ALPHA, gl::ONE_MINUS_DST_ALPHA, gl::ONE),
    ];
    let [sx, sy, sw, sh] = e.scissor();
    out.push(DrawCmd::Clip(Some([sx, sy, sw, sh])));
    if clear_slot {
        out.push(DrawCmd::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT));
    }
    out.push(DrawCmd::AtlasQuad {
        id: bitmap,
        pos: [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
        uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        color: 0xFFFF_FFFF,
        depth: [0.0, 1.0],
    });
    out.push(DrawCmd::AtlasEnd);
    out
}

/// `IsoDeadBody.render` with its atlas entry: the body's screen position
/// (camera offset and jiggle fix) and `BodyTexture.render`. `rgba` is the
/// square's light interpolated at the body (or the highlight colour).
pub fn body_world_draw(
    e: &AtlasEntry,
    cam: &Camera,
    pos: [f32; 3],
    rgba: [f32; 4],
) -> Vec<DrawCmd> {
    let [x, y, z] = pos;
    let sx = x_to_screen(x, y) - cam.off_x as f32 + cam.fix_jiggly_x * cam.zoom;
    let sy = y_to_screen(x, y, z, 0) - cam.off_y as f32 + cam.fix_jiggly_y * cam.zoom;
    body_depth_draw(e, cam, pos, sx, sy, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body lying along model z (1.8 long), 0.4 wide.
    fn bones() -> Vec<[f32; 3]> {
        vec![
            [0.0, 0.1, -0.9],
            [0.0, 0.1, 0.9],
            [0.2, 0.05, 0.0],
            [-0.2, 0.05, 0.0],
        ]
    }

    #[test]
    fn body_is_centred_in_its_entry() {
        for angle in [0.0f32, 0.7, 2.0, -1.3] {
            let pl = placement(angle, 1.0, &bones());
            assert!(
                pl.entry_w >= 64 && pl.entry_w % 32 == 0 && pl.entry_h % 32 == 0,
                "{pl:?}"
            );
            // With the offset applied the bounds are centred in the bitmap.
            let (p, mv) = matrices(angle, 1.0, pl.offset);
            let b = model_bounds(&p, &mv, &bones());
            let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            assert!(
                (cx - 512.0).abs() < 0.5 && (cy - 512.0).abs() < 0.5,
                "{angle}: {b:?}"
            );
        }
    }

    #[test]
    fn copy_targets_the_cell() {
        let e = AtlasEntry {
            atlas: 7,
            atlas_size: 1024,
            x: 256,
            y: 0,
            w: 256,
            h: 128,
            offset_x: 0.0,
            offset_y: 0.0,
        };
        let cmds = to_body_atlas(&e, true, false, 3);
        let DrawCmd::AtlasQuad { id, pos, .. } = &cmds[8] else {
            panic!("{:?}", cmds[8])
        };
        assert_eq!(*id, 3);
        assert_eq!(pos[0], [256.0, -64.0]);
        assert_eq!(pos[2], [512.0, 192.0]);
    }
}
