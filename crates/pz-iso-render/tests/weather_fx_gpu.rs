//! Rain over a synthetic scene through pz-gpu: outdoors it covers the view;
//! with the player in a building, the building's floor (drawn into the
//! mask) stays dry. `PZ_WEATHERFX_BMP=<dir>` also writes both frames as
//! BMP files there.

use std::collections::HashMap;
use std::rc::Rc;

use pz_core::random::Rand;
use pz_iso_render::weather_fx::{
    IsoWeatherFx, WeatherFxClimate, WeatherFxFrame, WeatherFxTextures, WeatherFxView,
};
use pz_iso_render::weather_fx_mask::{
    render_weather_fx, FxMaskFrame, FxMaskPlayer, FxMaskSprites, FxMaskWorld, WeatherFxMask,
};
use pz_iso_render::world::BuildingId;
use pz_iso_render::{Camera, ObjectKind, RenderObject, RenderSquare, SpriteFlags, SpriteInfo};
use pz_render::{flags, DrawCmd, DrawList, Rgba, Texture, TextureManager};

const W: u32 = 512;
const H: u32 = 256;

struct Room(HashMap<(i32, i32, i32), RenderSquare>);

impl FxMaskWorld for Room {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare> {
        self.0.get(&(x, y, z))
    }
}

/// A 6x6 room at (10..16, 10..16) with outside floors around it.
fn room() -> Room {
    let mut m = HashMap::new();
    for x in 0..30 {
        for y in 0..30 {
            let mut sq = RenderSquare::new(x, y, 0);
            let inside = (10..16).contains(&x) && (10..16).contains(&y);
            let s = Rc::new(SpriteInfo {
                flags: SpriteFlags {
                    solidfloor: true,
                    exterior: !inside,
                    ..Default::default()
                },
                ..Default::default()
            });
            sq.objects
                .push(RenderObject::new(ObjectKind::Plain, Some(s)));
            if inside {
                sq.cut.building = Some(BuildingId(1));
            }
            m.insert((x, y, 0), sq);
        }
    }
    Room(m)
}

fn solid(tm: &mut TextureManager, name: &str, w: i32, h: i32, rgba: [u8; 4]) -> Texture {
    tm.texture_from_pixels(
        name,
        w,
        h,
        rgba.repeat((w * h) as usize),
        flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST,
    )
}

/// A floor stand-in: a solid diamond in the bottom 64 rows of a 128x256
/// frame, as floor tiles are trimmed.
fn diamond(tm: &mut TextureManager) -> Texture {
    let mut px = vec![0u8; 128 * 64 * 4];
    for y in 0..64 {
        for x in 0..128 {
            let (dx, dy) = (
                (x as f32 - 63.5).abs() / 64.0,
                (y as f32 - 31.5).abs() / 32.0,
            );
            if dx + dy <= 1.0 {
                px[(y * 128 + x) * 4..][..4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    let mut t = tm.texture_from_pixels(
        "mask_floor",
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

fn write_bmp(path: &std::path::Path, px: &[u8]) {
    let row = (W * 4) as usize;
    let mut b = Vec::new();
    let size = 54 + px.len() as u32;
    b.extend_from_slice(b"BM");
    for v in [size, 0, 54, 40] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(W as i32).to_le_bytes());
    b.extend_from_slice(&(H as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    for v in [0u32, px.len() as u32, 2835, 2835, 0, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for y in (0..H as usize).rev() {
        for p in px[y * row..(y + 1) * row].chunks(4) {
            b.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    std::fs::write(path, b).expect("bmp");
}

/// Renders the scene with the player at `at` and returns the pixels.
fn frame(
    gpu: &mut pz_gpu::Gpu,
    tm: &mut TextureManager,
    fx: &mut IsoWeatherFx,
    world: &Room,
    sprites: &FxMaskSprites,
    at: [i32; 3],
    building: Option<BuildingId>,
) -> Vec<u8> {
    let cam = Camera::centred(12.5, 12.5, 0.0, W as i32, H as i32, 1.0);
    let view = WeatherFxView::new(cam.off_x as f32, cam.off_y as f32, W as i32, H as i32, 1.0);
    let mut mask = WeatherFxMask::default();
    mask.init_mask(
        world,
        FxMaskPlayer {
            square: Some(at),
            building,
            z: 0.0,
            master_region: None,
        },
        true,
    );
    let mut dl = DrawList::default();
    // The world: a dark ground.
    dl.cmds.push(DrawCmd::Ortho {
        left: 0.0,
        right: W as f32,
        bottom: H as f32,
        top: 0.0,
        width: W,
        height: H,
    });
    dl.render_rect(0, 0, W as i32, H as i32, Rgba(0.1, 0.15, 0.1, 1.0));
    render_weather_fx(
        fx,
        &mut mask,
        world,
        sprites,
        &view,
        &FxMaskFrame::default(),
        &mut dl.cmds,
    );
    let params = pz_gpu::FrameParams {
        clear: Some([0.0, 0.0, 0.0, 1.0]),
        ..pz_gpu::FrameParams::ui(W, H)
    };
    gpu.render_frame(&[(params, &dl)], tm);
    gpu.read_pixels()
}

#[test]
fn rain_stays_out_of_the_players_building() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else {
        return;
    };
    let mut tm = TextureManager::new();
    let streak = solid(&mut tm, "fogwhite_tex", 8, 8, [255, 255, 255, 255]);
    let textures = WeatherFxTextures {
        fog_white: Some(streak),
        ..Default::default()
    };
    let floor = Rc::new(SpriteInfo {
        texture: Some(diamond(&mut tm)),
        ..Default::default()
    });
    let sprites = FxMaskSprites {
        floor: Some(floor),
        ..Default::default()
    };
    let world = room();
    let rand = Rand::new(11);
    let mut fx = IsoWeatherFx::new(&rand, &textures);
    fx.update_fx(
        &WeatherFxClimate {
            precipitation_intensity: 1.0,
            precipitation_is_snow: false,
            wind_precip_intensity: 0.3,
            wind_angle_intensity: 0.5,
            ..Default::default()
        },
        &Default::default(),
    );
    for _ in 0..400 {
        fx.update(&rand, &WeatherFxFrame::default());
    }
    let outdoors = frame(
        &mut gpu,
        &mut tm,
        &mut fx,
        &world,
        &sprites,
        [5, 5, 0],
        None,
    );
    let indoors = frame(
        &mut gpu,
        &mut tm,
        &mut fx,
        &world,
        &sprites,
        [12, 12, 0],
        Some(BuildingId(1)),
    );
    if let Ok(dir) = std::env::var("PZ_WEATHERFX_BMP") {
        write_bmp(
            &std::path::Path::new(&dir).join("rain_outdoors.bmp"),
            &outdoors,
        );
        write_bmp(
            &std::path::Path::new(&dir).join("rain_indoors.bmp"),
            &indoors,
        );
    }
    let ground = |p: &[u8], i: usize| p[i * 4 + 1] <= 40;
    // The room's centre square sits at the view's centre; its floor
    // diamond spans 128x64 pixels about (256, 128) (shifted up a tile by
    // the floor's place in its frame).
    let cam = Camera::centred(12.5, 12.5, 0.0, W as i32, H as i32, 1.0);
    let cx = pz_render::iso::x_to_screen(12.5, 12.5) - cam.off_x as f32;
    let cy = pz_render::iso::y_to_screen(12.5, 12.5, 0.0, 0) - cam.off_y as f32;
    let mut wet_in = (0, 0);
    let mut wet_out = 0;
    for y in 0..H as usize {
        for x in 0..W as usize {
            let i = y * W as usize + x;
            let in_room = ((x as f32 - cx).abs() / 160.0 + (y as f32 - cy).abs() / 80.0) < 1.0;
            if in_room {
                wet_in.0 += usize::from(!ground(&indoors, i));
                wet_in.1 += usize::from(!ground(&outdoors, i));
            } else {
                wet_out += usize::from(!ground(&indoors, i));
            }
        }
    }
    assert!(wet_in.1 > 200, "rain over the room outdoors: {}", wet_in.1);
    assert_eq!(wet_in.0, 0, "rain inside the room with the player in it");
    assert!(wet_out > 150, "rain outside the room: {wet_out}");
}

/// With `PZ_GAME_DIR` set: every texture the effects and the mask load
/// exists in the game's packs, and a frame of each layer renders with
/// them (written out with `PZ_WEATHERFX_BMP`).
#[test]
fn game_textures_load_and_render() {
    let Ok(game) = std::env::var("PZ_GAME_DIR") else {
        return;
    };
    let mut fs = pz_fs::GameFs::new(std::path::Path::new(&game));
    fs.set_cache_dir(std::env::temp_dir().join("pz-weatherfx-test"));
    let mut tm = TextureManager::new();
    tm.load_ui_packs(&fs);
    for p in ["Tiles2x", "Tiles2x.floor"] {
        tm.load_texture_pack(&fs, p, 0, None);
    }
    tm.set_texture_pack_lookup(fs.mod_ids());
    let textures = WeatherFxTextures::load(|path, flags| {
        if flags == 0 {
            tm.shared_texture(&fs, path)
        } else {
            tm.shared_texture_flags(&fs, path, flags)
        }
    });
    let all: Vec<_> = textures
        .clouds
        .iter()
        .chain(&textures.fog)
        .chain(&textures.snow)
        .chain([&textures.fog_circle, &textures.fog_white])
        .collect();
    for (t, (path, _)) in all.iter().zip(WeatherFxTextures::paths()) {
        assert!(
            t.as_ref().is_some_and(|t| t.ready && t.width > 0),
            "missing {path}"
        );
    }
    let sprite = |tm: &mut TextureManager, name: &str| {
        let t = tm.shared_texture(&fs, name);
        assert!(t.is_some(), "missing {name}");
        Rc::new(SpriteInfo {
            texture: t,
            ..Default::default()
        })
    };
    let names = FxMaskSprites::SPRITES;
    let sprites = FxMaskSprites {
        floor: Some(sprite(&mut tm, names[0])),
        wall_n: Some(sprite(&mut tm, names[1])),
        wall_w: Some(sprite(&mut tm, names[2])),
        wall_nw: Some(sprite(&mut tm, names[3])),
        wall_se: Some(sprite(&mut tm, names[4])),
        white: tm.shared_texture(&fs, FxMaskSprites::WHITE),
    };
    assert!(sprites.white.is_some(), "missing {}", FxMaskSprites::WHITE);
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else {
        return;
    };
    let world = room();
    let rand = Rand::new(5);
    let mut fx = IsoWeatherFx::new(&rand, &textures);
    for (name, snow) in [("rain", false), ("snow", true)] {
        let c = WeatherFxClimate {
            precipitation_intensity: 0.8,
            precipitation_is_snow: snow,
            wind_intensity: 0.4,
            wind_precip_intensity: 0.2,
            wind_angle_intensity: 0.3,
            fog_intensity: 0.4,
            cloud_intensity: 0.8,
        };
        fx.update_fx(&c, &Default::default());
        for _ in 0..400 {
            fx.update(&rand, &WeatherFxFrame::default());
        }
        let outdoors = frame(
            &mut gpu,
            &mut tm,
            &mut fx,
            &world,
            &sprites,
            [5, 5, 0],
            None,
        );
        let indoors = frame(
            &mut gpu,
            &mut tm,
            &mut fx,
            &world,
            &sprites,
            [12, 12, 0],
            Some(BuildingId(1)),
        );
        assert_ne!(outdoors, indoors);
        if let Ok(dir) = std::env::var("PZ_WEATHERFX_BMP") {
            write_bmp(
                &std::path::Path::new(&dir).join(format!("game_{name}_outdoors.bmp")),
                &outdoors,
            );
            write_bmp(
                &std::path::Path::new(&dir).join(format!("game_{name}_indoors.bmp")),
                &indoors,
            );
        }
    }
}
