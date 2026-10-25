use std::rc::Rc;

use pz_render::model::{MeshData, MeshKind, ModelDraw, ModelLight, ModelPass};
use pz_render::{DrawCmd, DrawList, TextureManager};

use crate::{FrameParams, Gpu};

const IDENT: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// A static square over NDC `-r..r`.
fn square(r: f32) -> Rc<MeshData> {
    let v = |x: f32, y: f32| [x, y, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let vertices = [v(-r, -r), v(r, -r), v(r, r), v(-r, r)].concat();
    Rc::new(MeshData {
        id: 0x0u64.wrapping_sub(7),
        kind: MeshKind::Static,
        vertices,
        indices: vec![0, 1, 2, 0, 2, 3],
    })
}

fn draw(mesh: Rc<MeshData>) -> ModelDraw {
    ModelDraw {
        mesh,
        texture: None,
        mvp: IDENT,
        transform: IDENT,
        palette: Rc::new(Vec::new()),
        target_depth: 0.5,
        final_scale: 1.0,
        uv_scale: [1.0, 1.0],
        lights: [ModelLight::default(); 5],
        ambient: [1.0; 3],
        tint: [1.0; 3],
        hue: 0.0,
        alpha: 1.0,
        pass: ModelPass::Character { cull: false },
    }
}

/// `GL_LINEAR` + clamp-to-edge sampling of one row/column.
fn lerp_tap(line: &[f32], pos: f32) -> f32 {
    let p = pos - 0.5;
    let i0 = p.floor();
    let t = p - i0;
    let at = |i: f32| line[(i as i32).clamp(0, line.len() as i32 - 1) as usize];
    at(i0) * (1.0 - t) + at(i0 + 1.0) * t
}

fn q8(v: f32) -> f32 {
    (v.clamp(0.0, 1.0) * 255.0).round() / 255.0
}

/// The GL result of `endFrame` over an opaque black buffer, in red: mask
/// `a`, thickened horizontally then vertically (each stored as RGBA8 after
/// SRC_ALPHA blending onto transparent), then blitted.
fn reference(a: &[Vec<f32>], n: usize, radius: f32, res: f32) -> Vec<Vec<f32>> {
    let step = radius / res * n as f32;
    let thicken = |src: &[Vec<f32>], horizontal: bool| -> Vec<Vec<f32>> {
        let mut out = vec![vec![0.0; n]; n];
        for y in 0..n {
            for x in 0..n {
                let (line, c): (Vec<f32>, f32) = if horizontal {
                    (src[y].clone(), x as f32 + 0.5)
                } else {
                    ((0..n).map(|yy| src[yy][x]).collect(), y as f32 + 0.5)
                };
                let mut sum = lerp_tap(&line, c);
                for k in 1..=4 {
                    sum +=
                        lerp_tap(&line, c - k as f32 * step) + lerp_tap(&line, c + k as f32 * step);
                }
                let s = sum.clamp(0.0, 1.0);
                out[y][x] = s;
            }
        }
        out
    };
    // B: alpha s*s; C from B's alpha.
    let b: Vec<Vec<f32>> = thicken(a, true)
        .iter()
        .map(|r| r.iter().map(|s| q8(s * s)).collect())
        .collect();
    let c = thicken(&b, false);
    (0..n)
        .map(|y| {
            (0..n)
                .map(|x| {
                    let (rgb, alpha) = (q8(c[y][x]), q8(c[y][x] * c[y][x]));
                    rgb * alpha * (1.0 - a[y][x])
                })
                .collect()
        })
        .collect()
}

/// A square outlined in red: the mask, the two thickening passes and the
/// masked blit match the GL arithmetic, with the inside left alone.
#[test]
fn outline_passes_match_gl() {
    let n = 64u32;
    let Ok(mut g) = Gpu::new_headless(n, n) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    let radius = pz_render_radius(1.0);
    dl.cmds.extend([
        DrawCmd::OutlineBegin {
            width: n,
            height: n,
            clear: true,
        },
        DrawCmd::OutlineModel {
            draw: Box::new(draw(square(0.25))),
            color: [1.0, 0.0, 0.0],
        },
        DrawCmd::OutlineEnd,
        DrawCmd::OutlineThicken {
            color: [1.0, 0.0, 0.0, 1.0],
            radius,
            resolution: [n as f32, n as f32],
        },
        DrawCmd::OutlineBlit,
    ]);
    g.render_frame(
        &[(
            FrameParams {
                clear: Some([0.0, 0.0, 0.0, 1.0]),
                ..FrameParams::ui(n, n)
            },
            &dl,
        )],
        &mut tm,
    );
    let px = g.read_pixels();
    let a: Vec<Vec<f32>> = (0..n as usize)
        .map(|y| {
            (0..n as usize)
                .map(|x| {
                    if (24..40).contains(&x) && (24..40).contains(&y) {
                        1.0
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect();
    let want = reference(&a, n as usize, radius, n as f32);
    let mut lit = 0;
    for y in 0..n as usize {
        for x in 0..n as usize {
            let got = px[(y * n as usize + x) * 4..][..4].to_vec();
            let w = (want[y][x] * 255.0).round() as i32;
            assert!(
                (i32::from(got[0]) - w).abs() <= 3,
                "({x}, {y}): {got:?} vs {w}"
            );
            assert!(got[1] <= 1 && got[2] <= 1, "({x}, {y}): {got:?}");
            if got[0] > 128 {
                lit += 1;
            }
        }
    }
    // A ring around the square, nothing inside it.
    assert!(lit > 4 * 16, "{lit} outline pixels");
    assert_eq!(px[(32 * n as usize + 32) * 4], 0);
}

/// `PZMath.lerp(0.5, 0.2, zoom / 2.5)`.
fn pz_render_radius(zoom: f32) -> f32 {
    0.5 + (0.2 - 0.5) * (zoom / 2.5)
}

/// `FBORenderObjectOutline.Drawer2`: a sprite drawn into the mask with
/// `vboRenderer_SpriteOutline` outlines like a mesh covering the same
/// pixels, and texels under half alpha leave no mask.
#[test]
fn sprite_outline_masks_like_a_mesh() {
    let n = 64u32;
    let Ok(mut g) = Gpu::new_headless(n, n) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut tm = TextureManager::new();
    let solid = tm.texture_from_pixels("solid", 4, 4, [9, 9, 9, 255].repeat(16), 0);
    let faint = tm.texture_from_pixels("faint", 4, 4, [9, 9, 9, 120].repeat(16), 0);
    let quad = |t: &pz_render::Texture, x: f32, y: f32, w: f32| DrawCmd::OutlineSprite {
        texture: Some(t.clone()),
        atlas: None,
        pos: [[x, y], [x + w, y], [x + w, y + w], [x, y + w]],
        uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        color: [1.0, 0.0, 0.0, 1.0],
    };
    let radius = pz_render_radius(1.0);
    let mut dl = DrawList::default();
    dl.cmds.extend([
        DrawCmd::OutlineBegin { width: n, height: n, clear: true },
        quad(&solid, 24.0, 24.0, 16.0),
        quad(&faint, 2.0, 2.0, 10.0),
        DrawCmd::OutlineEnd,
        DrawCmd::OutlineThicken { color: [1.0, 0.0, 0.0, 1.0], radius, resolution: [n as f32, n as f32] },
        DrawCmd::OutlineBlit,
    ]);
    g.render_frame(&[(FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..FrameParams::ui(n, n) }, &dl)], &mut tm);
    let px = g.read_pixels();
    let a: Vec<Vec<f32>> = (0..n as usize)
        .map(|y| (0..n as usize).map(|x| if (24..40).contains(&x) && (24..40).contains(&y) { 1.0 } else { 0.0 }).collect())
        .collect();
    let want = reference(&a, n as usize, radius, n as f32);
    for y in 0..n as usize {
        for x in 0..n as usize {
            let got = px[(y * n as usize + x) * 4..][..4].to_vec();
            let w = (want[y][x] * 255.0).round() as i32;
            assert!((i32::from(got[0]) - w).abs() <= 3, "({x}, {y}): {got:?} vs {w}");
        }
    }
}

/// Split screen: `glViewport` offsets place a player's drawing in its part
/// of the screen (GL counts y from the bottom), and the outline passes
/// stay inside the player's rectangle.
#[test]
fn viewport_offsets_place_a_players_quarter() {
    let n = 64u32;
    let Ok(mut g) = Gpu::new_headless(n, n) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut tm = TextureManager::new();
    let solid = tm.texture_from_pixels("solid", 4, 4, [9, 9, 9, 255].repeat(16), 0);
    let mut dl = DrawList::default();
    // The world buffer, then player 1 of 4 in GL coordinates: x = 32,
    // y = 0 (the bottom row).
    dl.cmds.push(DrawCmd::OffscreenBegin { width: n, height: n });
    dl.cmds.push(DrawCmd::Viewport { x: 32, y: 0, width: 32, height: 32 });
    dl.cmds.push(DrawCmd::Ortho { left: 0.0, right: 32.0, bottom: 32.0, top: 0.0, width: 32, height: 32 });
    dl.render(None, 0.0, 0.0, 8.0, 8.0, pz_render::Rgba(0.0, 0.0, 1.0, 1.0));
    dl.cmds.extend([
        DrawCmd::OutlineBegin { width: n, height: n, clear: true },
        DrawCmd::OutlineSprite {
            texture: Some(solid.clone()),
            atlas: None,
            pos: [[12.0, 12.0], [20.0, 12.0], [20.0, 20.0], [12.0, 20.0]],
            uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            color: [1.0, 0.0, 0.0, 1.0],
        },
        DrawCmd::OutlineEnd,
        DrawCmd::OutlineThicken { color: [1.0, 0.0, 0.0, 1.0], radius: pz_render_radius(1.0), resolution: [n as f32, n as f32] },
        DrawCmd::OutlineBlit,
        DrawCmd::OffscreenEnd,
        // Core.RenderOffScreenBuffer over the whole screen.
        DrawCmd::Viewport { x: 0, y: 0, width: n, height: n },
        DrawCmd::Ortho { left: 0.0, right: n as f32, bottom: n as f32, top: 0.0, width: n, height: n },
        DrawCmd::ScreenPass { x: 0.0, y: 0.0, w: n as f32, h: n as f32, tex: [0.0, 0.0, n as f32, n as f32], params: pz_render::ScreenParams { desaturation: 0.0, timer: 0.0 } },
    ]);
    g.render_frame(&[(FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..FrameParams::ui(n, n) }, &dl)], &mut tm);
    let px = g.read_pixels();
    let at = |x: usize, y: usize| px[(y * n as usize + x) * 4..][..4].to_vec();
    // The blue quad: the top-left 8x8 of the bottom-right quarter.
    assert!(at(36, 36)[2] > 200 && at(36, 36)[0] < 30, "{:?}", at(36, 36));
    assert!(at(4, 4)[2] < 30, "{:?}", at(4, 4));
    // The outline ring around (44..52, 44..52), nothing in other quarters.
    let red = |x: usize, y: usize| at(x, y)[0] > 100;
    assert!(red(43, 48) && red(52, 48), "{:?} {:?}", at(43, 48), at(52, 48));
    assert!(!red(48, 48), "inside stays clear");
    for y in 0..n as usize {
        for x in 0..n as usize {
            if x < 32 || y < 32 {
                assert!(at(x, y)[0] < 30, "({x}, {y}) outside the player's quarter: {:?}", at(x, y));
            }
        }
    }
}
