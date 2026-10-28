//! The vision cone (`VisibilityPolygon2`) over a synthetic floor with a
//! few walls, through the whole world frame and pz-gpu: outside the cone
//! and behind the walls the floor is darkened by `visibilityBlur`'s
//! `0.022 * viewConeOpacity`, inside the cone it is not, and the wall
//! sprites (nearer than the shadow polygon's floor depth) are not.
//! `PZ_VISPOLY_BMP=<dir>` writes the frames as BMP files there.

use std::rc::Rc;

use pz_iso_render::frame::{world_frame_extras, ChunkLevels, FrameExtras, FrameSize};
use pz_iso_render::objects::FrameObjects;
use pz_iso_render::vispoly::{ChunkLevelData, VisSquare, VisionFrame};
use pz_iso_render::{
    Camera, ChunkRenderer, ChunkTexture, DepthMaps, DepthPreset, ObjectKind, RenderObject,
    RenderSquare, SpriteFlags, SpriteInfo,
};
use pz_render::{flags, DrawList, ScreenParams, Texture, TextureManager};

const W: u32 = 640;
const H: u32 = 360;

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
    let mut t = tm.texture_from_pixels(
        name,
        w,
        h,
        rgba.repeat((w * h) as usize),
        flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST,
    );
    t.width_orig = 128;
    t.height_orig = 256;
    t
}

/// A floor diamond in the bottom 64 rows of a 128x256 frame.
fn diamond(tm: &mut TextureManager, name: &str, rgb: [u8; 3]) -> Texture {
    let mut px = vec![0u8; 128 * 64 * 4];
    for y in 0..64 {
        for x in 0..128 {
            let (dx, dy) = (
                (x as f32 - 63.5).abs() / 64.0,
                (y as f32 - 31.5).abs() / 32.0,
            );
            if dx + dy <= 1.0 {
                px[(y * 128 + x) * 4..][..4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
    }
    let mut t = tm.texture_from_pixels(
        name,
        128,
        64,
        px,
        flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST,
    );
    t.offset_y = 192.0;
    t.width_orig = 128;
    t.height_orig = 256;
    t
}

fn write_bmp(path: &std::path::Path, px: &[u8], w: u32, h: u32) {
    let row = (w * 4) as usize;
    let mut b = Vec::new();
    b.extend_from_slice(b"BM");
    for v in [54 + px.len() as u32, 0, 54, 40] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    for v in [0u32, px.len() as u32, 2835, 2835, 0, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for y in (0..h as usize).rev() {
        for p in px[y * row..(y + 1) * row].chunks(4) {
            b.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    std::fs::write(path, b).expect("bmp");
}

#[test]
fn the_vision_cone_darkens_outside_the_cone_and_behind_walls() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else {
        return;
    };
    let mut tm = TextureManager::new();
    // Floor depth texture white: each floor pixel at its square's far
    // depth, behind the shadow polygon's floor-plane depth.
    let far = solid(&mut tm, "far", 128, 256, [255, 255, 255, 255]);
    let near = solid(&mut tm, "near", 128, 256, [40, 40, 40, 255]);
    let light = diamond(&mut tm, "light", [200, 200, 190]);
    let dark = diamond(&mut tm, "dark", [150, 160, 150]);
    let floor = |t: &Texture| {
        Rc::new(SpriteInfo {
            texture: Some(t.clone()),
            flags: SpriteFlags {
                solidfloor: true,
                ..Default::default()
            },
            tint_mod: [1.0; 3],
            ..Default::default()
        })
    };
    let (floor_a, floor_b) = (floor(&light), floor(&dark));
    // A low block: the bottom 128 rows of the frame.
    let mut wall_tex = solid(&mut tm, "wall", 128, 128, [170, 60, 50, 255]);
    wall_tex.offset_y = 128.0;
    let wall = Rc::new(SpriteInfo {
        texture: Some(wall_tex),
        depth_texture: Some(near),
        tint_mod: [1.0; 3],
        ..Default::default()
    });
    // Walls: west walls at x = 16 (y 10..15), north walls at y = 16
    // (x 14..18), and a solid square (a crate) at (20, 9).
    let west = |x: i32, y: i32| x == 16 && (10..15).contains(&y);
    let north = |x: i32, y: i32| y == 16 && (14..18).contains(&x);
    let solid_sq = |x: i32, y: i32| (x, y) == (20, 9);
    let mut squares = Vec::new();
    for y in 0..32 {
        for x in 0..32 {
            let mut sq = RenderSquare::new(x, y, 0);
            sq.objects.push(RenderObject::new(
                ObjectKind::Plain,
                Some(if (x + y) % 2 == 0 {
                    floor_a.clone()
                } else {
                    floor_b.clone()
                }),
            ));
            if west(x, y) || north(x, y) || solid_sq(x, y) {
                sq.objects
                    .push(RenderObject::new(ObjectKind::Plain, Some(wall.clone())));
            }
            squares.push(sq);
        }
    }
    let at = |x: i32, y: i32| &squares[(y * 32 + x) as usize];
    let mut chunks = Vec::new();
    let mut vis_chunks = Vec::new();
    for wy in 0..4 {
        for wx in 0..4 {
            let mut level = [None; 64];
            let mut vis = [None; 64];
            for i in 0..64 {
                let (x, y) = (wx * 8 + (i % 8) as i32, wy * 8 + (i / 8) as i32);
                level[i] = Some(at(x, y));
                vis[i] = Some(VisSquare {
                    solid: solid_sq(x, y),
                    vision_blocked_w: west(x, y),
                    vision_blocked_n: north(x, y),
                    ..Default::default()
                });
            }
            chunks.push(ChunkLevels {
                texture: ChunkTexture::new(wx, wy, 0, 0, 0),
                levels: vec![(0, level)],
                dirty: true,
            });
            vis_chunks.push(ChunkLevelData::recreate(wx, wy, 0, 0, 0, &vis));
        }
    }
    let cam = Camera::centred(12.5, 12.5, 0.0, W as i32, H as i32, 2.0);
    let maps = Maps(far);
    let size = FrameSize {
        width: W,
        height: H,
    };
    // Looking east (screen right and down), a cone of about 145 degrees.
    let look = 0.0f32;
    let vision = VisionFrame {
        vision_cone: -0.3,
        look_angle: look,
        look_vector: [look.cos(), look.sin()],
        chunk_map: [0, 0, 32, 32],
        chunks: vis_chunks,
        ..Default::default()
    };
    let mut render = |vision: Option<VisionFrame>| {
        let r = ChunkRenderer::new(cam, &maps, (W, H));
        let mut extras = FrameExtras {
            vision,
            ..Default::default()
        };
        let cmds = world_frame_extras(
            &r,
            size,
            &chunks,
            &FrameObjects::default(),
            None,
            &mut extras,
            ScreenParams::default(),
            Vec::new(),
            &mut |_| {},
        );
        let list = DrawList { cmds };
        gpu.render_frame(
            &[(
                pz_gpu::FrameParams {
                    clear: Some([0.0, 0.0, 0.0, 1.0]),
                    ..pz_gpu::FrameParams::ui(W, H)
                },
                &list,
            )],
            &mut tm,
        );
        gpu.read_pixels()
    };
    let plain = render(None);
    let cone = render(Some(vision.clone()));
    let strong = render(Some(VisionFrame {
        view_cone_opacity: 5,
        ..vision.clone()
    }));
    let screen = |x: f32, y: f32| {
        let sx = pz_render::iso::x_to_screen(x, y) - cam.off_x as f32;
        let sy = pz_render::iso::y_to_screen(x, y, 0.0, 0) - cam.off_y as f32;
        ((sx / cam.zoom) as usize, (sy / cam.zoom) as usize)
    };
    let px = |p: &[u8], (x, y): (usize, usize)| p[(y * W as usize + x) * 4..][..3].to_vec();
    let darkened = |(x, y)| {
        let (a, b) = (px(&plain, (x, y)), px(&cone, (x, y)));
        // 0.066 of black over the colour, then the screen pass.
        a.iter()
            .zip(&b)
            .all(|(a, b)| (0.89..0.96).contains(&(f32::from(*b) / f32::from(*a))))
    };
    let untouched = |(x, y)| px(&plain, (x, y)) == px(&cone, (x, y));
    if let Ok(dir) = std::env::var("PZ_VISPOLY_BMP") {
        let dir = std::path::Path::new(&dir);
        write_bmp(&dir.join("vispoly_off.bmp"), &plain, W, H);
        write_bmp(&dir.join("vispoly_on.bmp"), &cone, W, H);
        write_bmp(&dir.join("vispoly_opacity5.bmp"), &strong, W, H);
        // The darkening, amplified.
        let diff: Vec<u8> = plain
            .chunks(4)
            .zip(cone.chunks(4))
            .flat_map(|(a, b)| {
                let d = (i32::from(a[0]) - i32::from(b[0])).clamp(0, 255) as u8;
                let v = d.saturating_mul(20);
                [v, v, v, 255]
            })
            .collect();
        write_bmp(&dir.join("vispoly_diff_x20.bmp"), &diff, W, H);
    }
    // In the cone, before the walls.
    assert!(untouched(screen(14.5, 12.5)), "in the cone");
    assert!(
        untouched(screen(16.5, 7.5)),
        "in the cone, past the wall's end"
    );
    // Behind the player.
    assert!(darkened(screen(9.5, 12.5)), "behind the player");
    // Behind the west wall at x = 16.
    assert!(darkened(screen(19.5, 12.5)), "behind the west wall");
    // Behind the north wall at y = 16.
    assert!(darkened(screen(16.5, 19.5)), "behind the north wall");
}
