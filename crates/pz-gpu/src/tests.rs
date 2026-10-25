use super::*;
use pz_render::{Rgba, WorldShader};

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

/// A chunk texture holds two overlapping depth tiles; the nearer wins
/// whatever the draw order, the composite carries depth into the main
/// target, and texture rows keep GL's orientation.
#[test]
fn chunk_texture_depth_and_composite() {
    let Some(mut g) = gpu(8, 8) else { return };
    let mut tm = TextureManager::new();
    let red = tm.texture_from_pixels("red", 4, 4, [255, 0, 0, 255].repeat(16), 0);
    let green = tm.texture_from_pixels("green", 4, 4, [0, 255, 0, 255].repeat(16), 0);
    let blue = tm.texture_from_pixels("blue", 4, 4, [0, 0, 255, 255].repeat(16), 0);
    // Depth 0.5 everywhere except a transparent (depth 0) left column.
    let mut dpx = [128, 128, 128, 255].repeat(16);
    for y in 0..4 {
        dpx[y * 16..y * 16 + 4].copy_from_slice(&[0, 0, 0, 0]);
    }
    // Depth textures are sampled GL_NEAREST (`TileDepthTexture.updateGPUTexture`).
    let nearest = pz_render::flags::FILTER_MIN_NEAREST | pz_render::flags::FILTER_MAG_NEAREST;
    let depth = tm.texture_from_pixels("depth", 4, 4, dpx, nearest);
    let uv1 = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let tile = |tex: &pz_render::Texture, y: f32, h: f32, front: f32, far: f32| {
        let mut dl = DrawList::default();
        dl.render(Some(tex), 0.0, y, 8.0, h, Rgba(1.0, 1.0, 1.0, 1.0));
        let DrawCmd::Sprite(s) = dl.cmds.pop().unwrap() else { unreachable!() };
        DrawCmd::WorldSprite(s, WorldShader::TileDepth { depth: depth.clone(), uv1, front, far, opaque: false })
    };
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::ChunkBegin { id: 7, width: 8, height: 8, clear: true });
    // `StartFrameFlipY`: ortho (0, w, 0, h).
    dl.cmds.push(DrawCmd::Ortho { left: 0.0, right: 8.0, bottom: 0.0, top: 8.0, width: 8, height: 8 });
    dl.cmds.push(DrawCmd::BlendFuncSeparate(gl::ONE, gl::ONE_MINUS_SRC_ALPHA, gl::ONE_MINUS_DST_ALPHA, gl::ONE));
    dl.cmds.push(DrawCmd::GlEnable(gl::DEPTH_TEST));
    dl.cmds.push(DrawCmd::DepthFunc(gl::LEQUAL));
    dl.cmds.push(DrawCmd::DepthMask(true));
    // Green (further) over the whole chunk, red (nearer) on the top half
    // drawn first: red must stay on top where both are.
    dl.cmds.push(tile(&red, 0.0, 4.0, 0.1, 0.2));
    dl.cmds.push(tile(&green, 0.0, 8.0, 0.3, 0.4));
    dl.cmds.push(DrawCmd::ChunkEnd);
    dl.cmds.push(DrawCmd::Ortho { left: 0.0, right: 8.0, bottom: 8.0, top: 0.0, width: 8, height: 8 });
    dl.cmds.push(DrawCmd::Clear(gl::DEPTH_BUFFER_BIT));
    dl.cmds.push(DrawCmd::ChunkComposite { id: 7, x: 0.0, y: 0.0, w: 8.0, h: 8.0, color: 0xFFFF_FFFF, chunk_depth: 0.0, mag_linear: false });
    // A `default` sprite at window depth 0.25 hides green (0.35) but not
    // red (0.15).
    dl.cmds.push(DrawCmd::BlendFuncSeparate(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    let mut b = DrawList::default();
    b.render(Some(&blue), 0.0, 0.0, 8.0, 8.0, Rgba(1.0, 1.0, 1.0, 1.0));
    let DrawCmd::Sprite(s) = b.cmds.pop().unwrap() else { unreachable!() };
    dl.cmds.push(DrawCmd::WorldSprite(s, WorldShader::Default { z: -0.5 }));
    dl.cmds.push(DrawCmd::GlDisable(gl::DEPTH_TEST));
    g.render_frame(&[(opaque_black(8, 8), &dl)], &mut tm);
    let p = g.read_pixels();
    assert_eq!(px(&p, 8, 4, 1), [255, 0, 0, 255], "red stays in front, top rows");
    assert_eq!(px(&p, 8, 4, 6), [0, 0, 255, 255], "blue hides green");
    // The depth-0 column was discarded in the chunk; the composite there
    // carries the cleared depth, which clamps to 1, so blue covers it.
    assert_eq!(px(&p, 8, 0, 1), [0, 0, 255, 255]);
}

/// The offscreen world buffer drawn with the `screen` shader: a flat
/// colour comes back graded (desaturate 0.1, contrast 1.2 about 0.4).
#[test]
fn screen_pass_grades_the_offscreen_buffer() {
    let Some(mut g) = gpu(16, 8) else { return };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::OffscreenBegin { width: 16, height: 8 });
    dl.render_rect(0, 0, 16, 8, Rgba(0.6, 0.4, 0.2, 1.0));
    dl.cmds.push(DrawCmd::OffscreenEnd);
    dl.cmds.push(DrawCmd::ScreenPass { x: 0.0, y: 0.0, w: 16.0, h: 8.0, tex: [0.0, 0.0, 16.0, 8.0], params: pz_render::ScreenParams { desaturation: 0.0, timer: 0.0 } });
    g.render_frame(&[(opaque_black(16, 8), &dl)], &mut tm);
    let p = g.read_pixels();
    let got = px(&p, 16, 8, 4);
    assert!(near_rgb(got, [158, 103, 48]), "{got:?}");
}

#[test]
fn screen_pass_keeps_orientation() {
    let Some(mut g) = gpu(16, 16) else { return };
    let mut tm = TextureManager::new();
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::OffscreenBegin { width: 16, height: 16 });
    dl.render_rect(0, 0, 16, 8, Rgba(1.0, 0.0, 0.0, 1.0));
    dl.render_rect(0, 8, 16, 8, Rgba(0.0, 0.0, 1.0, 1.0));
    dl.cmds.push(DrawCmd::OffscreenEnd);
    dl.cmds.push(DrawCmd::ScreenPass { x: 0.0, y: 0.0, w: 16.0, h: 16.0, tex: [0.0, 0.0, 16.0, 16.0], params: pz_render::ScreenParams { desaturation: 0.0, timer: 0.0 } });
    g.render_frame(&[(opaque_black(16, 16), &dl)], &mut tm);
    let p = g.read_pixels();
    let (top, bottom) = (px(&p, 16, 8, 2), px(&p, 16, 8, 13));
    assert!(top[0] > 200 && top[2] < 50, "{top:?}");
    assert!(bottom[2] > 200 && bottom[0] < 50, "{bottom:?}");
}

fn surface_quad(x0: f32, y0: f32, x1: f32, y1: f32, attr: [f32; 4], depth: f32) -> [pz_render::SurfaceVertex; 4] {
    let v = |x, y| pz_render::SurfaceVertex { attr, pos: [x, y], color: 0xFFFF_FFFF, depth };
    [v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y1)]
}

fn surface(program: pz_render::SurfaceProgram, quads: Vec<[pz_render::SurfaceVertex; 4]>, ground: Option<pz_render::Texture>, params: [[f32; 4]; 2]) -> DrawCmd {
    DrawCmd::Surface(Box::new(pz_render::SurfaceDraw {
        program,
        quads,
        uniforms: pz_render::SurfaceUniforms { time: 1.0, offset: [-90000.0, -640000.0, 16.0, 8.0], viewport: [0.0, 0.0, 16.0, 8.0], reflection_param: 0.5, params },
        ground,
        reflection: [None, None],
        depth_func: gl::LEQUAL,
        blend_func: [gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA],
    }))
}

/// `puddles_lq` with a white height map: full level, so the puddle colour
/// is `mix(dark, reflection, 0.8)` at alpha 0.5.
#[test]
fn low_quality_puddles_follow_the_height_map() {
    let Some(mut g) = gpu(16, 8) else { return };
    let mut tm = TextureManager::new();
    let hm = tm.texture_from_pixels("hm", 4, 4, [255u8; 4].repeat(16), 0);
    let mut dl = DrawList::default();
    dl.cmds.push(DrawCmd::DepthMask(true));
    dl.cmds.push(DrawCmd::Clear(gl::DEPTH_BUFFER_BIT));
    dl.cmds.push(surface(pz_render::SurfaceProgram::PuddlesLq, vec![surface_quad(0.0, 0.0, 8.0, 8.0, [0.0; 4], 0.5)], Some(hm), [[0.0; 4], [0.0, 0.0, 0.5, 0.0]]));
    g.render_frame(&[(opaque_black(16, 8), &dl)], &mut tm);
    let p = g.read_pixels();
    assert!(near_rgb(px(&p, 16, 3, 3), [54, 54, 61]), "{:?}", px(&p, 16, 3, 3));
    assert_eq!(px(&p, 16, 12, 3), [0, 0, 0, 255]);
}

/// Every surface program builds and draws; water is depth-tested with
/// its fragment depth.
#[test]
fn surface_programs_draw_with_their_fragment_depth() {
    use pz_render::SurfaceProgram as P;
    let Some(mut g) = gpu(16, 8) else { return };
    let mut tm = TextureManager::new();
    let ground = tm.texture_from_pixels("ground", 4, 4, [200u8, 150, 100, 255].repeat(16), 0);
    for program in [P::Water, P::WaterHq, P::PuddlesHq, P::PuddlesMq, P::PuddlesLq] {
        let mut dl = DrawList::default();
        dl.cmds.push(DrawCmd::DepthMask(true));
        dl.cmds.push(DrawCmd::Clear(gl::DEPTH_BUFFER_BIT));
        // A nearer quad on the right half: depth 0.2 written.
        dl.cmds.push(DrawCmd::GlEnable(gl::DEPTH_TEST));
        dl.cmds.push(DrawCmd::DepthFunc(gl::ALWAYS));
        dl.cmds.push(DrawCmd::WorldSprite(
            pz_render::Sprite { texture: None, x: [8.0, 16.0, 16.0, 8.0], y: [0.0, 0.0, 8.0, 8.0], u: [0.0; 4], v: [0.0; 4], color: [0xFF00_00FF; 4], flipped: false },
            WorldShader::Default { z: 0.2 * 2.0 - 1.0 },
        ));
        let attr = if program.is_water() { [1.0, 0.0, 0.0, 1.0] } else { [0.0; 4] };
        dl.cmds.push(surface(program, vec![surface_quad(0.0, 0.0, 16.0, 8.0, attr, 0.5)], Some(ground.clone()), [[0.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.7, 0.0]]));
        g.render_frame(&[(opaque_black(16, 8), &dl)], &mut tm);
        let p = g.read_pixels();
        // Behind the nearer quad: untouched red.
        assert_eq!(px(&p, 16, 12, 4), [255, 0, 0, 255], "{program:?}");
        // In front of the cleared depth: the surface blended over black.
        assert_ne!(px(&p, 16, 3, 4), [0, 0, 0, 255], "{program:?}");
    }
}

/// The sky renders into its reflection texture: sky inside the sphere
/// map, black outside it; sampled like any texture afterwards.
#[test]
fn skybox_renders_into_its_reflection_texture() {
    let Some(mut g) = gpu(64, 64) else { return };
    let mut tm = TextureManager::new();
    let l = [0.35f32, 0.22, 0.3];
    let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
    let uniforms = pz_render::SkyBoxUniforms {
        time: 1.0,
        cloud_count: 0.3,
        cloud_size: 0.2,
        sun_light: l.map(|v| v / n),
        sun_colour: [1.0, 0.86, 0.7],
        sky_h_colour: [0.1, 0.1, 0.4],
        sky_l_colour: [0.1, 0.45, 0.7],
        cloud_light: 0.99,
        stars: 0.0,
        fog: 0.0,
        wind: [0.0; 3],
    };
    for hires in [false, true] {
        let mut dl = DrawList::default();
        dl.cmds.push(DrawCmd::SkyBox(Box::new(pz_render::SkyBoxDraw { target: 0, hires, uniforms })));
        let sky = pz_render::skybox_texture(0);
        dl.render(Some(&sky), 0.0, 0.0, 64.0, 64.0, Rgba(1.0, 1.0, 1.0, 1.0));
        g.render_frame(&[(opaque_black(64, 64), &dl)], &mut tm);
        let p = g.read_pixels();
        // Clouds overhead (the centre), blue sky lower down.
        let c = px(&p, 64, 32, 32);
        assert!(c[0] > 60 && c[3] == 255, "{hires}: {c:?}");
        let b = px(&p, 64, 50, 50);
        assert!(b[2] > b[0] + 40, "{hires}: {b:?}");
        assert_eq!(px(&p, 64, 1, 1), [0, 0, 0, 255], "{hires}");
    }
}

fn particles(program: pz_render::ParticleProgram, time: f32, texture: Option<pz_render::Texture>, particles: Vec<[f32; 4]>, blend: u32) -> DrawCmd {
    // Particles.orthogonal(0, 256, 256, 0, -1, 1), column major.
    let mvp = [[2.0 / 256.0, 0.0, 0.0, 0.0], [0.0, -2.0 / 256.0, 0.0, 0.0], [0.0, 0.0, -1.0, 0.0], [-1.0, 1.0, 0.0, 1.0]];
    DrawCmd::Particles(Box::new(pz_render::ParticlesDraw { program, mvp, time, param: [[0.0; 3]; 3], texture, particles, blend_func: [gl::SRC_ALPHA, blend], viewport: (256, 256) }))
}

/// One flame at the middle of its life: `fire.vert` with `hashA` = -1
/// (id 0, first cycle) gives size 0.5 (a 90-pixel quad turned by
/// -0.13 * 3.1415), lifted 75 pixels, alpha 0.5, atlas cell 16 (column 0,
/// row 2); `fire.frag` takes alpha from the texel's red, added to black.
#[test]
fn a_fire_particle_lands_where_fire_vert_puts_it() {
    let Some(mut g) = gpu(256, 256) else { return };
    let mut tm = TextureManager::new();
    let mut px8x4 = vec![0u8; 8 * 4 * 4];
    for t in px8x4.chunks_mut(4) {
        t.copy_from_slice(&[0, 255, 0, 255]);
    }
    px8x4[(2 * 8) * 4..(2 * 8) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
    let atlas = tm.texture_from_pixels("FireFlame", 8, 4, px8x4, 0);
    let mut dl = DrawList::default();
    dl.cmds.push(particles(pz_render::ParticleProgram::Fire, 0.3, Some(atlas), vec![[128.0, 203.0, 0.0, 0.0]], gl::ONE));
    g.render_frame(&[(opaque_black(256, 256), &dl)], &mut tm);
    let p = g.read_pixels();
    // Centre: the cell's red at alpha 0.5 (bilinear filtering pulls in a
    // little of the neighbouring cells).
    let c = px(&p, 256, 128, 128);
    assert!((118..=130).contains(&c[0]) && c[1] < 8 && c[2] == 0, "{c:?}");
    // Inside the turned square, and outside it.
    assert!(px(&p, 256, 128, 165)[0] > 0);
    assert_eq!(px(&p, 256, 128 + 60, 128), [0, 0, 0, 255]);
    assert_eq!(px(&p, 256, 128, 128 - 60), [0, 0, 0, 255]);
}

/// `smoke.vert`/`smoke.frag` draw alpha-blended puffs above the particle.
#[test]
fn a_smoke_particle_draws_above_its_origin() {
    let Some(mut g) = gpu(256, 256) else { return };
    let mut tm = TextureManager::new();
    let white = tm.texture_from_pixels("FireSmokes", 2, 2, [255u8; 4].repeat(4), 0);
    let mut dl = DrawList::default();
    dl.cmds.push(particles(pz_render::ParticleProgram::Smoke, 0.5, Some(white), vec![[128.0, 250.0, 0.0, 0.0]], gl::ONE_MINUS_SRC_ALPHA));
    g.render_frame(&[(opaque_black(256, 256), &dl)], &mut tm);
    let p = g.read_pixels();
    // Life 0.28..0.36: 110..145 pixels up, at least 105 pixels across.
    let c = px(&p, 256, 128, 120);
    assert!(c[0] > 0 && c[0] == c[1] && c[1] == c[2], "{c:?}");
    assert_eq!(px(&p, 256, 2, 254), [0, 0, 0, 255]);
}
