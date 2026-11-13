use super::*;
use pz_render::Texture;

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

/// A GL-style perspective matrix looking down -z.
fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / (fov_y / 2.0).tan();
    [[f / aspect, 0.0, 0.0, 0.0], [0.0, f, 0.0, 0.0], [0.0, 0.0, (far + near) / (near - far), -1.0], [0.0, 0.0, 2.0 * far * near / (near - far), 0.0]]
}

fn quad(z: f32, half: f32, tex: &Texture, color: u32) -> DrawCmd {
    DrawCmd::Quad3D {
        texture: tex.clone(),
        pos: [[-half, -half, z], [half, -half, z], [half, half, z], [-half, half, z]],
        uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        color: [color; 4],
        translucent: false,
    }
}

fn glass(z: f32, half: f32, tex: &Texture, color: u32) -> DrawCmd {
    let DrawCmd::Quad3D { texture, pos, uv, color, .. } = quad(z, half, tex, color) else { unreachable!() };
    DrawCmd::Quad3D { texture, pos, uv, color, translucent: true }
}

fn params() -> FrameParams {
    FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..FrameParams::ui(16, 16) }
}

fn begin() -> DrawCmd {
    DrawCmd::Scene3DBegin { view_proj: perspective(1.0, 1.0, 0.1, 100.0), clear: Some([0.0, 0.0, 1.0, 1.0]) }
}

const RED: u32 = 0xFF00_00FF;
const GREEN: u32 = 0xFF00_FF00;

#[test]
fn quad_renders_in_the_centre_over_the_clear_colour() {
    let Some(mut g) = gpu(16, 16) else { return };
    let mut tm = TextureManager::new();
    let white = tm.white();
    let mut dl = DrawList::default();
    dl.cmds.extend([begin(), quad(-2.0, 0.5, &white, RED), DrawCmd::Scene3DEnd]);
    g.render_frame(&[(params(), &dl)], &mut tm);
    let p = g.read_pixels();
    assert_eq!(px(&p, 16, 8, 8), [255, 0, 0, 255]);
    assert_eq!(px(&p, 16, 0, 0), [0, 0, 255, 255]);
    assert_eq!(px(&p, 16, 15, 15), [0, 0, 255, 255]);
}

#[test]
fn nearer_quad_wins_regardless_of_order() {
    for near_first in [true, false] {
        let Some(mut g) = gpu(16, 16) else { return };
        let mut tm = TextureManager::new();
        let white = tm.white();
        let near = quad(-1.5, 0.3, &white, RED);
        let far = quad(-3.0, 1.2, &white, GREEN);
        let mut dl = DrawList::default();
        dl.cmds.push(begin());
        if near_first {
            dl.cmds.extend([near, far]);
        } else {
            dl.cmds.extend([far, near]);
        }
        dl.cmds.push(DrawCmd::Scene3DEnd);
        g.render_frame(&[(params(), &dl)], &mut tm);
        let p = g.read_pixels();
        assert_eq!(px(&p, 16, 8, 8), [255, 0, 0, 255], "near_first={near_first}");
        assert_eq!(px(&p, 16, 8, 3), [0, 255, 0, 255], "near_first={near_first}");
    }
}

#[test]
fn transparent_texels_are_discarded() {
    let Some(mut g) = gpu(16, 16) else { return };
    let mut tm = TextureManager::new();
    let white = tm.white();
    let clear = tm.texture_from_pixels("clear3d", 1, 1, vec![255, 0, 0, 0], 0);
    let mut dl = DrawList::default();
    dl.cmds.extend([begin(), quad(-3.0, 0.9, &white, GREEN), quad(-1.5, 0.5, &clear, RED), DrawCmd::Scene3DEnd]);
    g.render_frame(&[(params(), &dl)], &mut tm);
    let p = g.read_pixels();
    assert_eq!(px(&p, 16, 8, 8), [0, 255, 0, 255]);
}

#[test]
fn texel_below_half_alpha_is_discarded() {
    let Some(mut g) = gpu(16, 16) else { return };
    let mut tm = TextureManager::new();
    let white = tm.white();
    // Alpha 76/255 = 0.30.
    let faint = tm.texture_from_pixels("faint3d", 1, 1, vec![255, 255, 255, 76], 0);
    let mut dl = DrawList::default();
    dl.cmds.extend([begin(), quad(-3.0, 0.9, &white, GREEN), quad(-1.5, 0.5, &faint, RED), DrawCmd::Scene3DEnd]);
    g.render_frame(&[(params(), &dl)], &mut tm);
    let p = g.read_pixels();
    assert_eq!(px(&p, 16, 8, 8), [0, 255, 0, 255]);
}

#[test]
fn texel_above_half_alpha_draws_opaque() {
    let Some(mut g) = gpu(16, 16) else { return };
    let mut tm = TextureManager::new();
    let white = tm.white();
    // Alpha 179/255 = 0.70; colour comes out opaque, not blended with the far quad.
    let solid = tm.texture_from_pixels("solid3d", 1, 1, vec![255, 255, 255, 179], 0);
    let mut dl = DrawList::default();
    dl.cmds.extend([begin(), quad(-3.0, 0.9, &white, GREEN), quad(-1.5, 0.5, &solid, RED), DrawCmd::Scene3DEnd]);
    g.render_frame(&[(params(), &dl)], &mut tm);
    let p = g.read_pixels();
    assert_eq!(px(&p, 16, 8, 8), [255, 0, 0, 255]);
}

#[test]
fn translucent_quad_blends_over_opaque() {
    let Some(mut g) = gpu(16, 16) else { return };
    let mut tm = TextureManager::new();
    let white = tm.white();
    let half = tm.texture_from_pixels("half3d", 1, 1, vec![255, 255, 255, 128], 0);
    let mut dl = DrawList::default();
    dl.cmds.extend([begin(), quad(-3.0, 0.9, &white, GREEN), glass(-1.5, 0.5, &half, RED), DrawCmd::Scene3DEnd]);
    g.render_frame(&[(params(), &dl)], &mut tm);
    let p = g.read_pixels();
    let c = px(&p, 16, 8, 8);
    assert!((i32::from(c[0]) - 128).abs() <= 6 && (i32::from(c[1]) - 127).abs() <= 6 && c[2] <= 6, "{c:?}");
}

#[test]
fn translucent_quad_does_not_write_depth() {
    let Some(mut g) = gpu(16, 16) else { return };
    let mut tm = TextureManager::new();
    let white = tm.white();
    let half = tm.texture_from_pixels("half3d", 1, 1, vec![255, 255, 255, 128], 0);
    let mut dl = DrawList::default();
    // Glass first, then a farther opaque quad that must still show through it.
    dl.cmds.extend([begin(), glass(-1.5, 0.5, &half, RED), quad(-3.0, 0.9, &white, GREEN), DrawCmd::Scene3DEnd]);
    g.render_frame(&[(params(), &dl)], &mut tm);
    let p = g.read_pixels();
    let c = px(&p, 16, 8, 8);
    assert!(c[1] > 100, "{c:?}");
}
