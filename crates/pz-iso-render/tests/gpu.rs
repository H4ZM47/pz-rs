//! A tiny synthetic scene recorded into a chunk texture, composited and
//! read back through pz-gpu.

use std::rc::Rc;

use pz_iso_render::{world::AttachedSprite, Camera, ChunkRenderer, ChunkTexture, DepthMaps, DepthPreset, ObjectKind, RenderObject, RenderSquare, SpriteFlags, SpriteInfo};
use pz_render::{flags, DrawCmd, Texture, TextureManager};

struct Maps(Texture);

impl DepthMaps for Maps {
    fn preset(&self, _: DepthPreset) -> Option<Texture> {
        Some(self.0.clone())
    }
    fn default_texture(&self) -> Option<Texture> {
        Some(self.0.clone())
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
    let maps = Maps(depth);
    let r = ChunkRenderer { cam, depth_maps: &maps };
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
