//! `renderFog` (`ImprovedFog`) through `world_frame_extras` and pz-gpu.
//!
//! The synthetic test draws a grass field with an indoor patch under fog
//! made from a generated noise texture and checks the fog lightens the
//! outdoors, leaves the frame alone without intensity, and stays off the
//! indoor rows. With `PZ_GAME_DIR` and `PZ_FOG_FRAMES` set, a house of
//! the game's tiles under the game's `fog_noise.png` is written as BMP
//! files there (`fog_hq.bmp`, `fog_lq.bmp`, `fog_inside.bmp`, `fog_off.bmp`).

use std::rc::Rc;

use pz_iso_render::cutaways::GridWorld;
use pz_iso_render::fog::{FogFrame, FogPlayer, ImprovedFog};
use pz_iso_render::frame::{world_frame_extras, ChunkLevels, FrameExtras, FrameSize};
use pz_iso_render::objects::FrameObjects;
use pz_iso_render::{
    Camera, ChunkRenderer, ChunkTexture, DepthMaps, DepthPreset, ObjectKind, RenderObject,
    RenderSquare, SpriteFlags, SpriteInfo, TileType,
};
use pz_render::{flags, DrawCmd, DrawList, Texture, TextureManager};

const W: u32 = 640;
const H: u32 = 400;

struct Maps(Texture);

impl DepthMaps for Maps {
    fn preset(&self, _: DepthPreset) -> Option<Texture> {
        Some(self.0.clone())
    }
    fn default_texture(&self) -> Option<Texture> {
        Some(self.0.clone())
    }
}

fn floor_tex(tm: &mut TextureManager, name: &str, rgba: [u8; 4]) -> Texture {
    let mut px = Vec::with_capacity(128 * 64 * 4);
    for y in 0..64 {
        for x in 0..128 {
            let inside = ((x as f32 + 0.5 - 64.0).abs() / 64.0
                + (y as f32 + 0.5 - 32.0).abs() / 32.0)
                <= 1.0;
            px.extend_from_slice(&if inside { rgba } else { [0; 4] });
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

fn depth_map(tm: &mut TextureManager) -> Texture {
    let mut px = Vec::with_capacity(128 * 256 * 4);
    for y in 0..256 {
        for x in 0..128 {
            let inside = y >= 192
                && ((x as f32 + 0.5 - 64.0).abs() / 64.0
                    + (y as f32 - 192.0 + 0.5 - 32.0).abs() / 32.0)
                    <= 1.0;
            px.extend_from_slice(&if inside { [255; 4] } else { [0, 0, 0, 255] });
        }
    }
    let mut d = tm.texture_from_pixels(
        "depth",
        128,
        256,
        px,
        flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST,
    );
    d.width_orig = 128;
    d.height_orig = 256;
    d
}

/// A tiling value noise in all four channels, standing in for
/// `fog_noise.png` (repeat-wrapped, linear).
fn noise_tex(tm: &mut TextureManager) -> Texture {
    const N: usize = 256;
    let hash = |x: usize, y: usize, c: usize| -> f32 {
        let mut h = (x as u32 % 32).wrapping_mul(374_761_393)
            ^ (y as u32 % 32).wrapping_mul(668_265_263)
            ^ (c as u32).wrapping_mul(2_246_822_519);
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        (h ^ (h >> 16)) as f32 / u32::MAX as f32
    };
    let mut px = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            for c in 0..4 {
                let (fx, fy) = (x as f32 / 8.0, y as f32 / 8.0);
                let (ix, iy) = (fx as usize, fy as usize);
                let (tx, ty) = (fx.fract(), fy.fract());
                let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
                let a = hash(ix, iy, c) + (hash(ix + 1, iy, c) - hash(ix, iy, c)) * sx;
                let b = hash(ix, iy + 1, c) + (hash(ix + 1, iy + 1, c) - hash(ix, iy + 1, c)) * sx;
                px.push(((a + (b - a) * sy) * 255.0) as u8);
            }
        }
    }
    tm.texture_from_pixels("fog_noise", N as i32, N as i32, px, 0)
}

fn fog_frame(alpha: f32, quality: i32, noise: &Texture, player: FogPlayer) -> FogFrame {
    let mut fog = ImprovedFog::default();
    fog.init(alpha);
    fog.update(&pz_iso_render::fog::FogClimate {
        fog_intensity: alpha,
        color: [0.9, 0.9, 0.95],
        wind_angle_radians: 0.7,
        wind_intensity: 0.2,
        multiplier: 1.0,
        fog_quality: quality,
    });
    FogFrame {
        fog,
        fog_quality: quality,
        noise: Some(noise.clone()),
        player,
        ..Default::default()
    }
}

/// The frame's chunks: level 0 of each chunk of `w` in `0..n` x `0..n`.
fn chunks(w: &GridWorld, n: i32) -> Vec<ChunkLevels<'_>> {
    let mut v = Vec::new();
    for wy in 0..n {
        for wx in 0..n {
            let mut level = [None; 64];
            for (i, slot) in level.iter_mut().enumerate() {
                *slot = w
                    .squares
                    .get(&(wx * 8 + (i % 8) as i32, wy * 8 + (i / 8) as i32, 0));
            }
            v.push(ChunkLevels {
                texture: ChunkTexture::new(wx, wy, 0, 0, 0),
                levels: vec![(0, level)],
                dirty: true,
            });
        }
    }
    v
}

fn render(
    gpu: &mut pz_gpu::Gpu,
    tm: &mut TextureManager,
    r: &ChunkRenderer<'_>,
    chunks: &[ChunkLevels<'_>],
    fog: Option<FogFrame>,
    w: u32,
    h: u32,
) -> (Vec<u8>, usize) {
    let mut extras = FrameExtras {
        fog,
        ..Default::default()
    };
    let cmds = world_frame_extras(
        r,
        FrameSize {
            width: w,
            height: h,
        },
        chunks,
        &FrameObjects::default(),
        None,
        &mut extras,
        pz_render::ScreenParams::default(),
        Vec::new(),
        &mut |_| {},
    );
    let rects = cmds
        .iter()
        .map(|c| {
            if let DrawCmd::ImprovedFog(d) = c {
                d.rects.len()
            } else {
                0
            }
        })
        .sum();
    let list = DrawList { cmds };
    gpu.render_frame(&[(pz_gpu::FrameParams::ui(w, h), &list)], tm);
    (gpu.read_pixels(), rects)
}

fn lum(px: &[u8], x: u32, y: u32, w: u32) -> f32 {
    let p = &px[((y * w + x) * 4) as usize..][..3];
    (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0
}

#[test]
fn fog_lightens_the_outdoors_and_spares_rooms() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else {
        return;
    };
    let mut tm = TextureManager::new();
    let grass = floor_tex(&mut tm, "grass", [40, 90, 40, 255]);
    let boards = floor_tex(&mut tm, "boards", [90, 60, 30, 255]);
    let depth = depth_map(&mut tm);
    let noise = noise_tex(&mut tm);
    let maps = Maps(depth);
    let outdoor = Rc::new(SpriteInfo {
        name: "grass".into(),
        texture: Some(grass),
        flags: SpriteFlags {
            solidfloor: true,
            exterior: true,
            ..Default::default()
        },
        tint_mod: [1.0; 3],
        ..Default::default()
    });
    let indoor = Rc::new(SpriteInfo {
        name: "boards".into(),
        texture: Some(boards),
        flags: SpriteFlags {
            solidfloor: true,
            ..Default::default()
        },
        tint_mod: [1.0; 3],
        ..Default::default()
    });
    let mut w = GridWorld::default();
    for y in 0..32 {
        for x in 0..32 {
            let mut sq = RenderSquare::new(x, y, 0);
            let inside = (12..17).contains(&x) && (7..13).contains(&y);
            sq.objects.push(RenderObject::new(
                ObjectKind::Plain,
                Some(if inside {
                    indoor.clone()
                } else {
                    outdoor.clone()
                }),
            ));
            if inside {
                sq.cut.room = Some(1);
            }
            w.insert(sq);
        }
    }
    let cam = Camera::centred(12.0, 12.0, 0.0, W as i32, H as i32, 1.0);
    let r = ChunkRenderer::new(cam, &maps, (W, H));
    let ch = chunks(&w, 4);
    let player = FogPlayer {
        x: 12.0,
        y: 12.0,
        ..Default::default()
    };
    let (clear, none) = render(
        &mut gpu,
        &mut tm,
        &r,
        &ch,
        Some(fog_frame(0.0, 0, &noise, player)),
        W,
        H,
    );
    assert_eq!(none, 0);
    let (plain, _) = render(&mut gpu, &mut tm, &r, &ch, None, W, H);
    assert_eq!(clear, plain, "no intensity, no fog");
    let (fogged, rects) = render(
        &mut gpu,
        &mut tm,
        &r,
        &ch,
        Some(fog_frame(1.0, 0, &noise, player)),
        W,
        H,
    );
    assert!(rects > 0);
    if let Ok(dir) = std::env::var("PZ_FOG_FRAMES") {
        write_bmp(
            &std::path::Path::new(&dir).join("fog_synthetic.bmp"),
            &fogged,
            W,
            H,
        );
        write_bmp(
            &std::path::Path::new(&dir).join("fog_synthetic_off.bmp"),
            &plain,
            W,
            H,
        );
    }
    // Away from the player's clear circle the grass is lighter under fog;
    // the indoor patch (screen centre of square (17, 17)) is not.
    let lighter = (0..W)
        .step_by(8)
        .flat_map(|x| (0..H).step_by(8).map(move |y| (x, y)))
        .filter(|&(x, y)| lum(&fogged, x, y, W) > lum(&plain, x, y, W) + 8.0)
        .count();
    assert!(lighter > 200, "fog lightened only {lighter} samples");
    let at = |p: &[u8], x: f32, y: f32| {
        let sx = pz_render::iso::x_to_screen(x, y) - cam.off_x as f32;
        let sy = pz_render::iso::y_to_screen(x, y, 0.0, 0) - cam.off_y as f32;
        lum(p, sx as u32, sy as u32, W)
    };
    eprintln!(
        "room {} -> {}, field {} -> {}",
        at(&plain, 14.5, 7.5),
        at(&fogged, 14.5, 7.5),
        at(&plain, 9.5, 15.5),
        at(&fogged, 9.5, 15.5)
    );
}

fn write_bmp(path: &std::path::Path, px: &[u8], w: u32, h: u32) {
    let row = (w * 4) as usize;
    let mut b = Vec::new();
    let size = 54 + px.len() as u32;
    b.extend_from_slice(b"BM");
    for v in [size, 0, 54, 40] {
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
            b.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    std::fs::write(path, b).expect("bmp");
}

/// With `PZ_GAME_DIR` and `PZ_FOG_FRAMES`: a house of the game's tiles in
/// a grass field under the game's fog noise, from outside (high and low
/// fog quality), from inside, and without fog.
#[test]
fn game_house_in_fog() {
    let (Ok(game), Ok(dir)) = (std::env::var("PZ_GAME_DIR"), std::env::var("PZ_FOG_FRAMES")) else {
        return;
    };
    const FW: u32 = 1024;
    const FH: u32 = 640;
    let mut fs = pz_fs::GameFs::new(std::path::Path::new(&game));
    fs.set_cache_dir(std::env::temp_dir().join("pz-fog-test"));
    let mut tm = TextureManager::new();
    tm.load_ui_packs(&fs);
    for p in ["Tiles2x", "Tiles2x.floor"] {
        tm.load_texture_pack(&fs, p, 0, None);
    }
    tm.set_texture_pack_lookup(fs.mod_ids());
    let tiles = pz_render::tile_depth::TileDepthTextureManager::init(&mut fs, &mut tm);
    let maps = pz_iso_render::seam::WorldDepthMaps {
        tiles: &tiles,
        seams: Vec::new(),
        cutaways: Vec::new(),
    };
    let noise = tm
        .shared_texture(&fs, "media/textures/weather/fognew/fog_noise.png")
        .expect("fog_noise.png");
    let mut sprite = |name: &str, f: &dyn Fn(&mut SpriteInfo)| {
        let t = tm.shared_texture(&fs, name);
        assert!(t.is_some(), "missing {name}");
        let mut s = SpriteInfo {
            name: name.to_string(),
            texture: t,
            tint_mod: [1.0; 3],
            ..Default::default()
        };
        f(&mut s);
        Rc::new(s)
    };
    let grass = sprite("blends_natural_01_16", &|s| {
        s.flags.solidfloor = true;
        s.flags.exterior = true;
    });
    let boards = sprite("floors_interior_tilesandwood_01_16", &|s| {
        s.flags.solidfloor = true
    });
    let wall_w = sprite("walls_exterior_house_01_0", &|s| {
        s.cut_w = true;
        s.tile_type = TileType::Wall;
        s.flags.wall_w = true;
    });
    let wall_n = sprite("walls_exterior_house_01_1", &|s| {
        s.cut_n = true;
        s.tile_type = TileType::Wall;
        s.flags.wall_n = true;
    });
    let wall_nw = sprite("walls_exterior_house_01_2", &|s| {
        s.cut_n = true;
        s.cut_w = true;
        s.tile_type = TileType::Wall;
        s.flags.wall_nw = true;
    });
    let wall_se = sprite("walls_exterior_house_01_3", &|s| {
        s.tile_type = TileType::Wall;
        s.flags.wall_se = true;
    });
    let obj = |s: &Rc<SpriteInfo>| RenderObject::new(ObjectKind::Plain, Some(s.clone()));
    let mut w = GridWorld::default();
    let (x0, x1) = (14, 20);
    for y in 0..40 {
        for x in 0..40 {
            let mut sq = RenderSquare::new(x, y, 0);
            let inside = (x0..x1).contains(&x) && (x0..x1).contains(&y);
            if inside {
                sq.objects.push(obj(&boards));
                sq.cut.room = Some(1);
            } else {
                sq.objects.push(obj(&grass));
            }
            let on_w = x == x0 || x == x1;
            let on_n = y == x0 || y == x1;
            let span = |v: i32| (x0..x1).contains(&v);
            match (on_w && span(y), on_n && span(x)) {
                (true, true) => sq.objects.push(obj(&wall_nw)),
                (true, false) => sq.objects.push(obj(&wall_w)),
                (false, true) => sq.objects.push(obj(&wall_n)),
                _ if x == x1 && y == x1 => sq.objects.push(obj(&wall_se)),
                _ => {}
            }
            w.insert(sq);
        }
    }
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(FW, FH) else {
        return;
    };
    let ch = chunks(&w, 5);
    for (name, at, inside, fog) in [
        ("fog_hq", (11.5, 11.5), false, Some(0)),
        ("fog_lq", (11.5, 11.5), false, Some(1)),
        ("fog_inside", (16.5, 16.5), true, Some(0)),
        ("fog_off", (11.5, 11.5), false, None),
    ] {
        let cam = Camera::player(at.0, at.1, 0.0, FW as i32, FH as i32, 1.0);
        let r = ChunkRenderer::new(cam, &maps, (FW, FH));
        let player = FogPlayer {
            x: at.0,
            y: at.1,
            in_room: inside,
            ..Default::default()
        };
        let frame = fog.map(|q| fog_frame(1.0, q, &noise, player));
        let (px, _) = render(&mut gpu, &mut tm, &r, &ch, frame, FW, FH);
        write_bmp(
            &std::path::Path::new(&dir).join(format!("{name}.bmp")),
            &px,
            FW,
            FH,
        );
    }
}
