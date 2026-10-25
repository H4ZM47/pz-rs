//! A tiny synthetic scene recorded into a chunk texture, composited and
//! read back through pz-gpu.

use std::rc::Rc;

use pz_iso_render::{world::AttachedSprite, Camera, ChunkRenderer, ChunkTexture, DepthMaps, DepthPreset, ObjectKind, RenderObject, RenderSquare, SpriteFlags, SpriteInfo};
use pz_render::{flags, DrawCmd, Texture, TextureManager};

/// Depth maps: one for the presets, one as the default texture.
struct Maps(Texture, Texture);

impl DepthMaps for Maps {
    fn preset(&self, _: DepthPreset) -> Option<Texture> {
        Some(self.0.clone())
    }
    fn default_texture(&self) -> Option<Texture> {
        Some(self.1.clone())
    }
}

fn solid(tm: &mut TextureManager, name: &str, w: i32, h: i32, rgba: [u8; 4]) -> Texture {
    tm.texture_from_pixels(name, w, h, rgba.repeat((w * h) as usize), flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST)
}

#[test]
fn floor_tiles_land_where_the_original_draws_them() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(256, 128) else { return };
    let mut tm = TextureManager::new();
    // A floor tile: the bottom 64 rows of a 128x256 frame.
    let mut floor_tex = solid(&mut tm, "floor", 128, 64, [200, 100, 50, 255]);
    floor_tex.offset_y = 192.0;
    floor_tex.width_orig = 128;
    floor_tex.height_orig = 256;
    let mut depth = solid(&mut tm, "depth", 128, 256, [128, 128, 128, 255]);
    depth.width_orig = 128;
    depth.height_orig = 256;
    let floor = Rc::new(SpriteInfo { texture: Some(floor_tex), flags: SpriteFlags { solidfloor: true, ..Default::default() }, tint_mod: [1.0; 3], ..Default::default() });
    let mut squares = Vec::new();
    for (x, y) in [(4, 4), (5, 4)] {
        let mut sq = RenderSquare::new(x, y, 0);
        sq.objects.push(RenderObject::new(ObjectKind::Plain, Some(floor.clone())));
        squares.push(sq);
    }
    let _ = AttachedSprite { sprite: floor.clone(), alpha: 1.0 };
    let cam = Camera::centred(4.0, 4.0, 0.0, 256, 128, 1.0);
    assert_eq!((cam.off_x, cam.off_y), (-128, 192));
    let maps = Maps(depth.clone(), depth);
    let r = ChunkRenderer::new(cam, &maps, (256, 128));
    let tex = ChunkTexture::new(0, 0, 0, 0, 0);
    let mut level = [None; 64];
    for sq in &squares {
        level[(sq.x + sq.y * 8) as usize] = Some(sq);
    }
    let mut cmds = Vec::new();
    r.record(tex, &[(0, level)], &mut cmds);
    // Core.StartFrame's clear (glClear honours the depth write mask).
    cmds.push(DrawCmd::DepthMask(true));
    cmds.push(DrawCmd::Clear(pz_render::gl::DEPTH_BUFFER_BIT));
    r.composite(&[tex], 256, 128, &mut cmds);
    let list = pz_render::DrawList { cmds };
    let params = pz_gpu::FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..pz_gpu::FrameParams::ui(256, 128) };
    gpu.render_frame(&[(params, &list)], &mut tm);
    let p = gpu.read_pixels();
    let px = |x: usize, y: usize| p[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4].to_vec();
    // Square (4, 4) spans x 64..192, y 64..128 on screen; (5, 4) is 64 to
    // the right and 32 lower.
    assert_eq!(px(70, 100), vec![200, 100, 50, 255]);
    assert_eq!(px(250, 120), vec![200, 100, 50, 255]);
    assert_eq!(px(30, 30), vec![0, 0, 0, 255]);
}

/// An open door is drawn in the translucent pass at alpha 0.6 over the
/// composited floor, depth-tested against it.
#[test]
fn open_door_is_drawn_translucent_over_the_floor() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(256, 128) else { return };
    let mut tm = TextureManager::new();
    let mut floor_tex = solid(&mut tm, "floor", 128, 64, [200, 100, 50, 255]);
    floor_tex.offset_y = 192.0;
    floor_tex.width_orig = 128;
    floor_tex.height_orig = 256;
    let mut door_tex = solid(&mut tm, "door", 128, 256, [0, 0, 255, 255]);
    door_tex.width_orig = 128;
    door_tex.height_orig = 256;
    let mut depth = solid(&mut tm, "depth", 128, 256, [128, 128, 128, 255]);
    depth.width_orig = 128;
    depth.height_orig = 256;
    let floor = Rc::new(SpriteInfo { texture: Some(floor_tex), flags: SpriteFlags { solidfloor: true, ..Default::default() }, tint_mod: [1.0; 3], ..Default::default() });
    // The door's own depth texture puts it nearer than the floor.
    let mut near = solid(&mut tm, "near", 128, 256, [64, 64, 64, 255]);
    near.width_orig = 128;
    near.height_orig = 256;
    let door = Rc::new(SpriteInfo { texture: Some(door_tex), tile_type: pz_iso_render::TileType::DoorN, depth_texture: Some(near), tint_mod: [1.0; 3], ..Default::default() });
    let mut sq = RenderSquare::new(4, 4, 0);
    sq.objects.push(RenderObject::new(ObjectKind::Plain, Some(floor)));
    sq.objects.push(RenderObject::new(ObjectKind::Door { open: true }, Some(door)));
    let cam = Camera::centred(4.0, 4.0, 0.0, 256, 128, 1.0);
    let maps = Maps(depth.clone(), depth);
    let r = ChunkRenderer::new(cam, &maps, (256, 128));
    let tex = ChunkTexture::new(0, 0, 0, 0, 0);
    let mut level = [None; 64];
    level[36] = Some(&sq);
    let mut cmds = vec![DrawCmd::DepthMask(true), DrawCmd::Clear(pz_render::gl::DEPTH_BUFFER_BIT)];
    r.record(tex, &[(0, level)], &mut cmds);
    r.composite(&[tex], 256, 128, &mut cmds);
    r.render_translucent(&[&sq], 0, &mut cmds);
    let list = pz_render::DrawList { cmds };
    let params = pz_gpu::FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..pz_gpu::FrameParams::ui(256, 128) };
    gpu.render_frame(&[(params, &list)], &mut tm);
    let p = gpu.read_pixels();
    let px = |x: usize, y: usize| p[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4].to_vec();
    // Floor (200, 100, 50) * 0.4 + premultiplied blue * 0.6.
    let got = px(128, 100);
    let want = [80u8, 40, 173];
    assert!(got.iter().zip(want).all(|(a, b)| (i32::from(*a) - i32::from(b)).abs() <= 1), "{got:?}");
    // Above the floor only the door shows: blue at 0.6 over black.
    let got = px(128, 20);
    assert!((i32::from(got[2]) - 153).abs() <= 1 && got[0] == 0, "{got:?}");
}

/// The whole frame: offscreen buffer, chunk composite and screen pass.
#[test]
fn world_frame_reaches_the_screen() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(256, 128) else { return };
    let mut tm = TextureManager::new();
    let mut floor_tex = solid(&mut tm, "floor", 128, 64, [200, 100, 50, 255]);
    floor_tex.offset_y = 192.0;
    floor_tex.width_orig = 128;
    floor_tex.height_orig = 256;
    let mut depth = solid(&mut tm, "depth", 128, 256, [128, 128, 128, 255]);
    depth.width_orig = 128;
    depth.height_orig = 256;
    let floor = Rc::new(SpriteInfo { texture: Some(floor_tex), flags: SpriteFlags { solidfloor: true, ..Default::default() }, tint_mod: [1.0; 3], ..Default::default() });
    let mut sq = RenderSquare::new(4, 4, 0);
    sq.objects.push(RenderObject::new(ObjectKind::Plain, Some(floor)));
    let cam = Camera::centred(4.0, 4.0, 0.0, 256, 128, 1.0);
    let maps = Maps(depth.clone(), depth);
    let r = ChunkRenderer::new(cam, &maps, (256, 128));
    let mut level = [None; 64];
    level[36] = Some(&sq);
    let chunks = [pz_iso_render::frame::ChunkLevels { texture: ChunkTexture::new(0, 0, 0, 0, 0), levels: vec![(0, level)], dirty: true }];
    let cmds = pz_iso_render::frame::world_frame(&r, pz_iso_render::frame::FrameSize { width: 256, height: 128 }, &chunks, pz_render::ScreenParams { desaturation: 0.0, timer: 0.0 });
    let list = pz_render::DrawList { cmds };
    gpu.render_frame(&[(pz_gpu::FrameParams::ui(256, 128), &list)], &mut tm);
    let p = gpu.read_pixels();
    let px = |x: usize, y: usize| p[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4].to_vec();
    let got = px(128, 96);
    assert!(got[0] > 150 && got[2] < 100, "{got:?}");
}

/// A cube mesh as `ImportedStaticMesh` lays it out: position, normal, uv.
fn cube(id: u64, h: f32) -> pz_render::model::MeshData {
    let mut v = Vec::new();
    let mut idx = Vec::new();
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    ];
    for (n, a, b) in faces {
        let base = (v.len() / 8) as u32;
        for (s, t) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p: [f32; 3] = std::array::from_fn(|i| (n[i] + a[i] * s + b[i] * t) * h);
            v.extend_from_slice(&[p[0], p[1], p[2], n[0], n[1], n[2], 0.5, 0.5]);
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    pz_render::model::MeshData { id, kind: pz_render::model::MeshKind::Static, vertices: v, indices: idx }
}

/// A model drawn through the model camera lands where its square is and
/// is hidden by world sprites nearer than its square.
#[test]
fn model_lands_on_its_square_and_depth_tests_against_tiles() {
    use pz_iso_render::models::{fill_lights, ModelBasis, ModelCamera};
    use pz_render::model::{ModelDraw, ModelPass};
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(256, 128) else { return };
    let mut tm = TextureManager::new();
    let white = tm.white();
    let cam = Camera::centred(4.0, 4.0, 0.0, 256, 128, 1.0);
    let mc = ModelCamera::new(&cam, 256.0, 128.0);
    let identity = pz_iso_render::trees::Mat4::identity().0;
    let draw = |x: f32, y: f32| {
        let mv = mc.model_view(x, y, 0.0, ModelBasis::Character { angle: 0.0 });
        let mvp = mc.mvp(&mv);
        let (lights, ambient) = fill_lights(pz_iso_render::color::ColorInfo::WHITE, [x, y, 0.0], 0.0);
        DrawCmd::Model(Box::new(ModelDraw {
            mesh: Rc::new(cube(1, 0.4)),
            texture: Some(white.clone()),
            mvp: mvp.0,
            transform: identity,
            palette: Rc::new(Vec::new()),
            target_depth: mc.target_depth(&mvp, x, y, 0.0, 1e-4),
            final_scale: 1.0,
            uv_scale: [1.0, 1.0],
            lights,
            ambient,
            tint: [1.0, 0.0, 0.0],
            hue: 0.0,
            alpha: 1.0,
            pass: ModelPass::Character { cull: true },
        }))
    };
    // A blue sprite over the whole view, at the depth of the square two to
    // the south-east: it hides models behind it but not models in front.
    let mut bl = pz_render::DrawList::default();
    let blue = solid(&mut tm, "blue", 4, 4, [0, 0, 255, 255]);
    bl.render(Some(&blue), 0.0, 0.0, 256.0, 128.0, pz_render::Rgba(1.0, 1.0, 1.0, 1.0));
    let DrawCmd::Sprite(s) = bl.cmds.pop().unwrap() else { unreachable!() };
    let wall_depth = pz_render::iso::square_depth(4, 4, 5.0, 5.0, 0.0);
    let mut cmds = vec![
        DrawCmd::Ortho { left: 0.0, right: 256.0, bottom: 128.0, top: 0.0, width: 256, height: 128 },
        DrawCmd::DepthMask(true),
        DrawCmd::Clear(pz_render::gl::DEPTH_BUFFER_BIT),
        DrawCmd::GlEnable(pz_render::gl::DEPTH_TEST),
        DrawCmd::DepthFunc(pz_render::gl::LEQUAL),
        DrawCmd::WorldSprite(s, pz_render::WorldShader::Default { z: wall_depth * 2.0 - 1.0 }),
    ];
    // Behind the wall (square 4,4) and in front of it (square 6,6).
    cmds.push(draw(4.5, 4.5));
    cmds.push(draw(6.5, 6.5));
    let list = pz_render::DrawList { cmds };
    gpu.render_frame(&[(pz_gpu::FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..pz_gpu::FrameParams::ui(256, 128) }, &list)], &mut tm);
    let p = gpu.read_pixels();
    let px = |x: usize, y: usize| p[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4].to_vec();
    // Square (4.5, 4.5) projects to the view centre; (6.5, 6.5) is 128
    // pixels lower, off screen at y 128; check its top instead.
    assert_eq!(px(128, 64)[2], 255, "the hidden model stays behind the wall: {:?}", px(128, 64));
    let red_rows: Vec<usize> = (0..128).filter(|&y| px(128, y)[0] > 0).collect();
    assert!(!red_rows.is_empty(), "the near model is drawn");
    assert!(*red_rows.first().unwrap() > 64, "{red_rows:?}");
}

/// Depth maps with a billboard depth texture, as fire sprites use.
struct BillboardMaps(Texture);

impl DepthMaps for BillboardMaps {
    fn preset(&self, _: DepthPreset) -> Option<Texture> {
        Some(self.0.clone())
    }
    fn default_texture(&self) -> Option<Texture> {
        Some(self.0.clone())
    }
    fn billboard(&self) -> Option<Texture> {
        Some(self.0.clone())
    }
}

/// A fire's flame frame stands on the middle of its square, drawn
/// unlit in the translucent pass with the billboard depth texture; its
/// smoke is drawn in the square's light at half tint.
#[test]
fn a_fire_stands_on_its_square() {
    use pz_iso_render::fire::{FireSquare, FireState};
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(256, 128) else { return };
    let mut tm = TextureManager::new();
    let mut flame = solid(&mut tm, "flame", 64, 64, [255, 0, 0, 255]);
    flame.offset_x = 32.0;
    flame.offset_y = 192.0;
    flame.width_orig = 128;
    flame.height_orig = 256;
    let mut smoke = solid(&mut tm, "smoke", 32, 32, [200, 200, 200, 255]);
    smoke.offset_x = 48.0;
    smoke.offset_y = 140.0;
    smoke.width_orig = 128;
    smoke.height_orig = 256;
    let mut depth = solid(&mut tm, "depth", 128, 256, [128, 128, 128, 255]);
    depth.width_orig = 128;
    depth.height_orig = 256;
    let mut fire = FireState::new_fire(FireSquare::default(), 100, 1000, &mut pz_iso_render::fire::rand(&pz_core::random::Rand::new(1)));
    // Stage 2: smoke, then flames.
    fire.life_stage = 1;
    fire.life_stage_timer = 1;
    fire.update_from_timer(1.5, &mut pz_iso_render::fire::rand(&pz_core::random::Rand::new(2)));
    assert_eq!(fire.life_stage, 2);
    let obj = pz_iso_render::fire::render_object(&fire, false, &|n: &str| if n.starts_with("Fire") { Some(flame.clone()) } else { Some(smoke.clone()) });
    let mut sq = RenderSquare::new(4, 4, 0);
    sq.light_info = pz_iso_render::color::ColorInfo { r: 1.0, g: 0.5, b: 1.0, a: 1.0 };
    sq.objects.push(obj);
    let cam = Camera::centred(4.0, 4.0, 0.0, 256, 128, 1.0);
    let maps = BillboardMaps(depth);
    let r = ChunkRenderer::new(cam, &maps, (256, 128));
    let mut cmds = vec![DrawCmd::DepthMask(true), DrawCmd::Clear(pz_render::gl::DEPTH_BUFFER_BIT)];
    r.render_translucent(&[&sq], 0, &mut cmds);
    let list = pz_render::DrawList { cmds };
    let params = pz_gpu::FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..pz_gpu::FrameParams::ui(256, 128) };
    gpu.render_frame(&[(params, &list)], &mut tm);
    let p = gpu.read_pixels();
    let px = |x: usize, y: usize| p[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4].to_vec();
    // The square's top corner is at (128, 64), its centre 32 lower: the
    // flame's 64x64 covers x 96..160, y 32..96.
    assert_eq!(px(128, 60), vec![255, 0, 0, 255]);
    assert_eq!(px(100, 34), vec![255, 0, 0, 255]);
    assert_eq!(px(90, 60), vec![0, 0, 0, 255]);
    assert_eq!(px(128, 100), vec![0, 0, 0, 255]);
    // The smoke (x 112..144, rows -20..12) in the light times 0.5.
    let s = px(128, 5);
    let want = [100, 50, 100];
    assert!(s.iter().zip(want).all(|(a, b)| (i32::from(*a) - b).abs() <= 2), "{s:?}");
    assert_eq!(px(108, 5), vec![0, 0, 0, 255]);
}
