use super::*;
use pz_render::Rgba;

fn gpu(w: u32, h: u32) -> Option<Gpu> {
    match Gpu::new_headless(w, h) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn px(p: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    p[i..i + 4].try_into().unwrap()
}

fn near_rgb(a: [u8; 4], b: [u8; 3]) -> bool {
    a[..3].iter().zip(b).all(|(x, y)| (i32::from(*x) - i32::from(y)).abs() <= 1)
}

fn opaque_black(w: u32, h: u32) -> FrameParams {
    FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..FrameParams::ui(w, h) }
}

#[test]
fn solid_quads_blend_like_gl() {
    let Some(mut g) = gpu(16, 8) else { return };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    dl.render_rect(0, 0, 8, 8, Rgba(1.0, 0.0, 0.0, 1.0));
    // Half-transparent blue over red with SRC_ALPHA, ONE_MINUS_SRC_ALPHA;
    // 0.5 becomes 127/255 in the vertex colour.
    dl.render_rect(4, 0, 8, 8, Rgba(0.0, 0.0, 1.0, 0.5));
    g.render_frame(&[(opaque_black(16, 8), &dl)], &mut tm);
    let p = g.read_pixels();
    assert_eq!(px(&p, 16, 1, 1), [255, 0, 0, 255]);
    assert!(near_rgb(px(&p, 16, 5, 1), [128, 0, 127]), "{:?}", px(&p, 16, 5, 1));
    assert!(near_rgb(px(&p, 16, 10, 7), [0, 0, 127]), "{:?}", px(&p, 16, 10, 7));
    assert_eq!(px(&p, 16, 14, 7), [0, 0, 0, 255]);
}

#[test]
fn quads_cover_exact_pixels() {
    let Some(mut g) = gpu(8, 8) else { return };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    dl.render_rect(2, 3, 3, 2, Rgba(1.0, 1.0, 1.0, 1.0));
    g.render_frame(&[(opaque_black(8, 8), &dl)], &mut tm);
    let p = g.read_pixels();
    for y in 0..8 {
        for x in 0..8 {
            let inside = (2..5).contains(&x) && (3..5).contains(&y);
            assert_eq!(px(&p, 8, x, y)[0], if inside { 255 } else { 0 }, "{x},{y}");
        }
    }
}

#[test]
fn stencil_clips_like_set_stencil_rect() {
    // UIElement.setStencilRect(2, 2, 4, 4), a full-screen quad, then
    // clearStencilRect.
    let Some(mut g) = gpu(8, 8) else { return };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::StencilMask(255));
    dl.cmds.push(DrawCmd::GlEnable(gl::STENCIL_TEST));
    dl.cmds.push(DrawCmd::StencilFunc(gl::EQUAL, 0, 255));
    dl.cmds.push(DrawCmd::StencilOp(gl::KEEP, gl::KEEP, gl::INCR));
    dl.cmds.push(DrawCmd::ColorMask(false, false, false, false));
    dl.renderi(None, 2, 2, 4, 4, Rgba(1.0, 0.0, 0.0, 1.0));
    dl.cmds.push(DrawCmd::ColorMask(true, true, true, true));
    dl.cmds.push(DrawCmd::StencilOp(gl::KEEP, gl::KEEP, gl::KEEP));
    dl.cmds.push(DrawCmd::StencilFunc(gl::EQUAL, 1, 255));
    dl.render_rect(0, 0, 8, 8, Rgba(0.0, 1.0, 0.0, 1.0));
    dl.cmds.push(DrawCmd::GlDisable(gl::STENCIL_TEST));
    dl.cmds.push(DrawCmd::StencilFunc(gl::ALWAYS, 255, 255));
    dl.cmds.push(DrawCmd::Clear(gl::STENCIL_BUFFER_BIT | gl::DEPTH_BUFFER_BIT));
    g.render_frame(&[(opaque_black(8, 8), &dl)], &mut tm);
    let p = g.read_pixels();
    assert_eq!(px(&p, 8, 3, 3), [0, 255, 0, 255]);
    assert_eq!(px(&p, 8, 1, 1), [0, 0, 0, 255]);
    assert_eq!(px(&p, 8, 6, 6), [0, 0, 0, 255]);
    // The stencil was cleared, so the next frame draws everywhere.
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::GlEnable(gl::STENCIL_TEST));
    dl.cmds.push(DrawCmd::StencilFunc(gl::EQUAL, 0, 255));
    dl.render_rect(0, 0, 8, 8, Rgba(0.0, 0.0, 1.0, 1.0));
    g.render_frame(&[(opaque_black(8, 8), &dl)], &mut tm);
    assert_eq!(px(&g.read_pixels(), 8, 1, 1), [0, 0, 255, 255]);
}

#[test]
fn textured_quad_is_tinted_by_vertex_colour() {
    let Some(mut g) = gpu(4, 2) else { return };
    let mut tm = TextureManager::new();
    let white = tm.white();
    let mut dl = DrawList::default();
    dl.render(Some(&white), 0.0, 0.0, 4.0, 2.0, Rgba(1.0, 0.5, 0.0, 1.0));
    g.render_frame(&[(opaque_black(4, 2), &dl)], &mut tm);
    assert_eq!(px(&g.read_pixels(), 4, 3, 1), [255, 127, 0, 255]);
}

#[test]
fn scissor_limits_drawing() {
    let Some(mut g) = gpu(8, 8) else { return };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::Clip(Some([4, 0, 4, 8])));
    dl.render_rect(0, 0, 8, 8, Rgba(1.0, 1.0, 1.0, 1.0));
    dl.cmds.push(DrawCmd::Clip(None));
    g.render_frame(&[(opaque_black(8, 8), &dl)], &mut tm);
    let p = g.read_pixels();
    assert_eq!(px(&p, 8, 3, 3)[0], 0);
    assert_eq!(px(&p, 8, 4, 3)[0], 255);
}

#[test]
fn triangle_split_and_projection() {
    assert_eq!(quad_indices(4, 1, 1), [4, 5, 6, 4, 6, 7]);
    assert_eq!(quad_indices(4, 1, 2), [5, 6, 7, 5, 7, 4]);
    let m = gl::ortho2d(0.0, 100.0, 50.0, 0.0);
    // Screen (0, 0) is the top-left of clip space.
    assert_eq!((m[3][0], m[3][1]), (-1.0, 1.0));
}
