//! The frame extras over a synthetic floor through the whole world frame
//! and pz-gpu: corpse flies over a square, a highlighted crate redrawn in
//! its highlight colour (the plain one unchanged), a grid-square marker
//! circle, an iso marker's circle outline and an area highlight with its
//! outline. `PZ_EXTRAS_BMP=<dir>` writes the frames as BMP files there.

use std::rc::Rc;

use pz_iso_render::area_highlights::{AreaHighlight, AreaHighlightsFrame};
use pz_iso_render::color::ColorInfo;
use pz_iso_render::flies::CorpseFlies;
use pz_iso_render::frame::{world_frame_extras, ChunkLevels, FrameExtras, FrameSize};
use pz_iso_render::markers::{GridSquareMarker, IsoMarker, MarkersFrame};
use pz_iso_render::objects::FrameObjects;
use pz_iso_render::world::ObjectHighlight;
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

/// A 1024x1024 sheet of 8x7 frames of 128 pixels, each a cloud of dark
/// dots whose spread depends on the frame.
fn fly_sheet(tm: &mut TextureManager) -> Texture {
    let mut px = vec![0u8; 1024 * 1024 * 4];
    for f in 0..56usize {
        let (cx, cy) = ((f % 8) * 128 + 64, (f / 8) * 128 + 64);
        for k in 0..40usize {
            let a = (k * 37 + f * 11) as f32 * 0.7;
            let r = 10.0 + ((k * 13 + f * 7) % 40) as f32;
            let (x, y) = (cx as f32 + a.cos() * r, cy as f32 + a.sin() * r * 0.6);
            for dy in 0..3 {
                for dx in 0..3 {
                    let i = ((y as usize + dy) * 1024 + x as usize + dx) * 4;
                    px[i..i + 4].copy_from_slice(&[10, 10, 10, 255]);
                }
            }
        }
    }
    tm.texture_from_pixels(
        "media/textures/CorpseFlies.png",
        1024,
        1024,
        px,
        flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST,
    )
}

/// A filled disc (`circle_center`).
fn disc(tm: &mut TextureManager) -> Texture {
    let mut px = vec![0u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let d = ((x as f32 - 31.5).powi(2) + (y as f32 - 31.5).powi(2)).sqrt();
            if d < 31.0 {
                px[(y * 64 + x) * 4..][..4].copy_from_slice(&[
                    255,
                    255,
                    255,
                    if d > 26.0 { 255 } else { 120 },
                ]);
            }
        }
    }
    tm.texture_from_pixels("circle_center", 64, 64, px, 0)
}

#[test]
fn extras_draw_in_the_world_frame() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else {
        return;
    };
    let mut tm = TextureManager::new();
    let far = solid(&mut tm, "far", 128, 256, [255, 255, 255, 255]);
    let near = solid(&mut tm, "near", 128, 256, [40, 40, 40, 255]);
    let light = diamond(&mut tm, "light", [200, 200, 190]);
    let dark = diamond(&mut tm, "dark", [150, 160, 150]);
    let white = tm.texture_from_pixels("white", 4, 4, vec![255; 64], 0);
    let flies_tex = fly_sheet(&mut tm);
    let circle = disc(&mut tm);
    let floor = |t: &Texture| {
        Rc::new(SpriteInfo {
            texture: Some(t.clone()),
            flags: SpriteFlags {
                solidfloor: true,
                exterior: true,
                ..Default::default()
            },
            tint_mod: [1.0; 3],
            ..Default::default()
        })
    };
    let (floor_a, floor_b) = (floor(&light), floor(&dark));
    let mut box_tex = solid(&mut tm, "crate", 64, 64, [150, 110, 60, 255]);
    box_tex.offset_x = 32.0;
    box_tex.offset_y = 176.0;
    let crate_ = Rc::new(SpriteInfo {
        texture: Some(box_tex),
        depth_texture: Some(near),
        tint_mod: [1.0; 3],
        ..Default::default()
    });
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
            if (x, y) == (14, 11) || (x, y) == (10, 14) {
                let mut o = RenderObject::new(ObjectKind::Plain, Some(crate_.clone()));
                if (x, y) == (14, 11) {
                    o.highlight = Some(ObjectHighlight {
                        color: ColorInfo::new(1.0, 0.1, 0.1, 0.6),
                        alpha_scale: 1.0,
                    });
                }
                sq.objects.push(o);
            }
            sq.flies = (x, y) == (11, 11);
            squares.push(sq);
        }
    }
    let at = |x: i32, y: i32| &squares[(y * 32 + x) as usize];
    let mut chunks = Vec::new();
    for wy in 0..4 {
        for wx in 0..4 {
            let level =
                std::array::from_fn(|i| Some(at(wx * 8 + (i % 8) as i32, wy * 8 + (i / 8) as i32)));
            chunks.push(ChunkLevels {
                texture: ChunkTexture::new(wx, wy, 0, 0, 0),
                levels: vec![(0, level)],
                dirty: true,
            });
        }
    }
    let cam = Camera::player(12.5, 12.5, 0.0, W as i32, H as i32, 2.0);
    let maps = Maps(far);
    let size = FrameSize {
        width: W,
        height: H,
    };
    let mut render = |on: bool| {
        let r = ChunkRenderer::new(cam, &maps, (W, H));
        let mut extras = FrameExtras::default();
        if on {
            extras.flies = Some(CorpseFlies {
                texture: Some(flies_tex.clone()),
                counter: 0.0,
                frame: 3,
            });
            extras.highlight = Some(Default::default());
            extras.markers = Some(MarkersFrame {
                grid_square_markers: vec![GridSquareMarker {
                    orig: [12.0, 14.0, 0.0],
                    z: 0.0,
                    size: 1.5,
                    rgb: [0.2, 0.6, 1.0],
                    alpha: 0.9,
                    active: true,
                    texture_name: "circle_center".into(),
                    overlay_texture_name: None,
                }],
                circle_center: Some(circle.clone()),
                white: Some(white.clone()),
                chunk_map: Some([0, 0, 32, 32]),
                iso_markers: vec![IsoMarker {
                    x: 15.5,
                    y: 14.5,
                    z: 0.0,
                    z_layer: 0,
                    rgba: [1.0, 1.0, 0.0, 1.0],
                    active: true,
                    circle_size: 1.2,
                    ..Default::default()
                }],
                player_z: Some(0.0),
                ..Default::default()
            });
            extras.area_highlights = Some(AreaHighlightsFrame {
                white: Some(white.clone()),
                chunk_map: Some([0, 0, 32, 32]),
                user_defined: vec![(
                    None,
                    AreaHighlight {
                        x1: 8,
                        y1: 8,
                        x2: 11,
                        y2: 10,
                        z: 0,
                        rgba: [0.1, 0.9, 0.2, 0.35],
                    },
                )],
                ..Default::default()
            });
        }
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
    let plain = render(false);
    let on = render(true);
    if let Ok(dir) = std::env::var("PZ_EXTRAS_BMP") {
        let dir = std::path::Path::new(&dir);
        write_bmp(&dir.join("extras_off.bmp"), &plain, W, H);
        write_bmp(&dir.join("extras_on.bmp"), &on, W, H);
    }
    let screen = |x: f32, y: f32| {
        let sx = pz_render::iso::x_to_screen(x, y) - cam.off_x as f32;
        let sy = pz_render::iso::y_to_screen(x, y, 0.0, 0) - cam.off_y as f32;
        ((sx / cam.zoom) as usize, (sy / cam.zoom) as usize)
    };
    let px = |p: &[u8], (x, y): (usize, usize)| {
        [
            p[(y * W as usize + x) * 4],
            p[(y * W as usize + x) * 4 + 1],
            p[(y * W as usize + x) * 4 + 2],
        ]
    };
    // The highlighted crate's face turns red; the other crate does not.
    let face = |x: f32, y: f32| {
        let (sx, sy) = screen(x + 0.5, y + 0.5);
        (sx, sy - 22)
    };
    let (a, b) = (px(&plain, face(14.0, 11.0)), px(&on, face(14.0, 11.0)));
    assert!(b[1] < a[1] && b[2] < a[2] && b[0] > 2 * b[1], "{a:?} {b:?}");
    assert_eq!(px(&plain, face(10.0, 14.0)), px(&on, face(10.0, 14.0)));
    // The area highlight greens its floor.
    let (a, b) = (px(&plain, screen(9.5, 9.0)), px(&on, screen(9.5, 9.0)));
    assert!(b[1] > b[0] && b[0] < a[0], "{a:?} {b:?}");
    // The grid marker tints the floor under its centre blue.
    let (a, b) = (px(&plain, screen(12.5, 14.5)), px(&on, screen(12.5, 14.5)));
    assert!(b[2] > b[0] && a != b, "{a:?} {b:?}");
}
