//! Rain over a synthetic scene through pz-gpu: outdoors it covers the view;
//! with the player in a building, the building's floor (drawn into the
//! mask) stays dry; the south and east walls the mask cuts back out let
//! the rain over them unless the player's cutaway opens them; characters
//! drawn into the mask stay dry. `PZ_WEATHERFX_BMP=<dir>` also writes the
//! frames as BMP files there.

use std::collections::HashMap;
use std::rc::Rc;

use pz_core::random::Rand;
use pz_iso_render::weather_fx::{
    IsoWeatherFx, WeatherFxClimate, WeatherFxFrame, WeatherFxTextures, WeatherFxView,
};
use pz_iso_render::weather_fx_mask::{
    render_weather_fx, FxMaskFrame, FxMaskPlayer, FxMaskSprites, FxMaskWorld, WeatherFxMask,
};
use pz_iso_render::cutaways::{CUTAWAY_N, CUTAWAY_W};
use pz_iso_render::cutaways::GridWorld;
use pz_iso_render::world::BuildingId;
use pz_iso_render::{Camera, ObjectKind, RenderObject, RenderSquare, SpriteFlags, SpriteInfo, TileType};
use pz_render::{flags, DrawCmd, DrawList, Rgba, Texture, TextureManager};

const W: u32 = 512;
const H: u32 = 256;

struct Room {
    squares: HashMap<(i32, i32, i32), RenderSquare>,
    /// The player's cutaway flags by square.
    cut: HashMap<(i32, i32, i32), u8>,
    /// A character standing on this square, drawn as this screen rect,
    /// after a blend function change the mask must ignore.
    character: Option<((i32, i32), [f32; 4])>,
}

impl FxMaskWorld for Room {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare> {
        self.squares.get(&(x, y, z))
    }
    fn cutaway_flags(&self, x: i32, y: i32, z: i32) -> u8 {
        self.cut.get(&(x, y, z)).copied().unwrap_or(0)
    }
    fn render_characters(&self, sq: &RenderSquare, _view: &WeatherFxView, out: &mut Vec<DrawCmd>) {
        if let Some(((x, y), [rx, ry, rw, rh])) = self.character {
            if (sq.x, sq.y) == (x, y) {
                out.push(DrawCmd::BlendFunc(pz_render::gl::ZERO, pz_render::gl::ONE));
                out.push(DrawCmd::Quad { texture: None, x: rx, y: ry, w: rw, h: rh, color: Rgba(1.0, 1.0, 1.0, 1.0) });
            }
        }
    }
}

/// A path under the system temp dir unique to this call (tag, process id and a
/// per-call counter): tests run in parallel and must not share directories.
fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("{tag}-{}-{n}", std::process::id()))
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
    Room { squares: m, cut: HashMap::new(), character: None }
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
    write_bmp_sized(path, px, W, H);
}

fn write_bmp_sized(path: &std::path::Path, px: &[u8], w: u32, h: u32) {
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
    fs.set_cache_dir(unique_temp_dir("pz-weatherfx-test"));
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
        ..Default::default()
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

/// Rain drawn and let run until it covers the view.
fn rainy(tm: &mut TextureManager, seed: i64) -> IsoWeatherFx {
    let streak = solid(tm, "fogwhite_tex", 8, 8, [255, 255, 255, 255]);
    let textures = WeatherFxTextures {
        fog_white: Some(streak),
        ..Default::default()
    };
    let rand = Rand::new(seed);
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
    fx
}

/// A wall stand-in trimmed to 66x226 at `offset_x` in a 128x256 frame.
fn wall(tm: &mut TextureManager, name: &str, offset_x: f32) -> Rc<SpriteInfo> {
    // Pixels are laid out at the power-of-two size.
    let mut t = tm.texture_from_pixels(
        name,
        66,
        226,
        vec![255; 128 * 256 * 4],
        flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST,
    );
    t.offset_x = offset_x;
    t.offset_y = 30.0;
    t.width_orig = 128;
    t.height_orig = 256;
    Rc::new(SpriteInfo {
        texture: Some(t),
        ..Default::default()
    })
}

/// A `wallcutaways` stand-in: kept (red) everywhere but the cells the cut
/// north (700) and west (512) walls of these tests use, which are cut
/// (transparent blue).
fn cutaways(tm: &mut TextureManager) -> Texture {
    let (w, h) = (1024usize, 256usize);
    let mut px = Vec::with_capacity(w * h * 4);
    for _ in 0..h {
        for x in 0..w {
            let cut = (512..578).contains(&x) || (700..766).contains(&x);
            px.extend(if cut { [0, 0, 255, 0] } else { [255, 0, 0, 255] });
        }
    }
    tm.texture_from_pixels(
        "wallcutaways",
        w as i32,
        h as i32,
        px,
        flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST,
    )
}

/// Wet pixels (`> 40` green) inside the room's floor diamonds, away from
/// their edges.
fn wet_on_room_floor(p: &[u8]) -> usize {
    let cam = Camera::centred(12.5, 12.5, 0.0, W as i32, H as i32, 1.0);
    let mut n = 0;
    for y in 0..H as usize {
        for x in 0..W as usize {
            let (sx, sy) = (x as f32 + 0.5 + cam.off_x as f32, y as f32 + 0.5 + cam.off_y as f32);
            let (ix, iy) = (pz_render::iso::x_to_iso(sx, sy, 0.0), pz_render::iso::y_to_iso(sx, sy, 0.0));
            let inner = |v: f32| (10.0..16.0).contains(&v) && (0.05..0.95).contains(&v.fract());
            if inner(ix) && inner(iy) && p[(y * W as usize + x) * 4 + 1] > 40 {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn cut_walls_near_the_player_keep_the_rain_off() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else {
        return;
    };
    let mut tm = TextureManager::new();
    let mut fx = rainy(&mut tm, 11);
    let floor = Rc::new(SpriteInfo {
        texture: Some(diamond(&mut tm)),
        ..Default::default()
    });
    let sprites = FxMaskSprites {
        floor: Some(floor),
        wall_n: Some(wall(&mut tm, "wall_n", 62.0)),
        wall_w: Some(wall(&mut tm, "wall_w", 0.0)),
        wall_cutaways: Some(cutaways(&mut tm)),
        ..Default::default()
    };
    let mut world = room();
    let inside = Some(BuildingId(1));
    // Uncut: the south and east walls (on the squares outside) are cut
    // back out of the mask, and the rain falls over them where they rise
    // in front of the room's floor.
    let whole = frame(&mut gpu, &mut tm, &mut fx, &world, &sprites, [12, 12, 0], inside);
    // The player's cutaway opens them: their stencil cells drop the wall,
    // so the floor behind stays masked.
    for i in 10..16 {
        world.cut.insert((16, i, 0), CUTAWAY_W);
        world.cut.insert((i, 16, 0), CUTAWAY_N);
    }
    let cut = frame(&mut gpu, &mut tm, &mut fx, &world, &sprites, [12, 12, 0], inside);
    if let Ok(dir) = std::env::var("PZ_WEATHERFX_BMP") {
        write_bmp(&std::path::Path::new(&dir).join("rain_walls_whole.bmp"), &whole);
        write_bmp(&std::path::Path::new(&dir).join("rain_walls_cut.bmp"), &cut);
    }
    let (wet_whole, wet_cut) = (wet_on_room_floor(&whole), wet_on_room_floor(&cut));
    assert!(wet_whole > 100, "rain over the uncut walls: {wet_whole}");
    assert_eq!(wet_cut, 0, "rain through the cut walls");
}

#[test]
fn characters_drawn_into_the_mask_stay_dry() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else {
        return;
    };
    let mut tm = TextureManager::new();
    let mut fx = rainy(&mut tm, 11);
    let floor = Rc::new(SpriteInfo {
        texture: Some(diamond(&mut tm)),
        ..Default::default()
    });
    let sprites = FxMaskSprites {
        floor: Some(floor),
        ..Default::default()
    };
    let mut world = room();
    // A character on the room's open north edge, standing up over the
    // squares beyond it.
    let cam = Camera::centred(12.5, 12.5, 0.0, W as i32, H as i32, 1.0);
    let sx = pz_render::iso::x_to_screen(12.0, 10.0) - cam.off_x as f32;
    let sy = pz_render::iso::y_to_screen(12.0, 10.0, 0.0, 0) - cam.off_y as f32;
    let rect = [sx - 24.0, sy - 120.0, 48.0, 110.0];
    let wet_in_rect = |p: &[u8]| {
        let mut n = 0;
        for y in (rect[1] as usize + 1)..((rect[1] + rect[3]) as usize - 1) {
            for x in (rect[0] as usize + 1)..((rect[0] + rect[2]) as usize - 1) {
                n += usize::from(p[(y * W as usize + x) * 4 + 1] > 40);
            }
        }
        n
    };
    let inside = Some(BuildingId(1));
    let bare = frame(&mut gpu, &mut tm, &mut fx, &world, &sprites, [12, 12, 0], inside);
    world.character = Some(((12, 10), rect));
    let with = frame(&mut gpu, &mut tm, &mut fx, &world, &sprites, [12, 12, 0], inside);
    assert!(wet_in_rect(&bare) > 10, "rain where the character stands: {}", wet_in_rect(&bare));
    assert_eq!(wet_in_rect(&with), 0, "rain over the character");
}

/// The mask's view of a [`GridWorld`] and the player's cutaways.
struct GameWorld<'a> {
    w: &'a GridWorld,
    st: &'a pz_iso_render::CutawayState,
}

impl FxMaskWorld for GameWorld<'_> {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare> {
        self.w.squares.get(&(x, y, z))
    }
    fn cutaway_flags(&self, x: i32, y: i32, z: i32) -> u8 {
        self.st.cutaway_flags(x, y, z)
    }
}

/// With `PZ_GAME_DIR` and `PZ_WEATHERFX_BMP` set: a house of the game's
/// tiles in rain with the player inside, its south and east walls cut
/// away by the player's cutaways, the weather masked through the cut
/// walls; written as `game_cut_walls_rain.bmp` (and, with the walls the
/// mask cuts drawn whole, `game_whole_walls_rain.bmp`), each with a
/// `_dry` twin without the weather to compare against.
#[test]
fn game_building_with_cut_walls_in_rain() {
    let (Ok(game), Ok(dir)) = (std::env::var("PZ_GAME_DIR"), std::env::var("PZ_WEATHERFX_BMP")) else {
        return;
    };
    const FW: u32 = 1024;
    const FH: u32 = 640;
    let mut fs = pz_fs::GameFs::new(std::path::Path::new(&game));
    fs.set_cache_dir(unique_temp_dir("pz-weatherfx-test"));
    let mut tm = TextureManager::new();
    tm.load_ui_packs(&fs);
    for p in ["Tiles2x", "Tiles2x.floor"] {
        tm.load_texture_pack(&fs, p, 0, None);
    }
    tm.set_texture_pack_lookup(fs.mod_ids());
    let tiles = pz_render::tile_depth::TileDepthTextureManager::init(&mut fs, &mut tm);
    let stencil = tm.shared_texture_flags(&fs, "media/wallcutaways.png", 3);
    assert!(stencil.is_some(), "missing media/wallcutaways.png");
    let maps = pz_iso_render::seam::WorldDepthMaps { tiles: &tiles, seams: Vec::new(), cutaways: stencil.into_iter().collect() };
    let mut sprite = |name: &str, f: &dyn Fn(&mut SpriteInfo)| {
        let t = tm.shared_texture(&fs, name);
        assert!(t.is_some(), "missing {name}");
        let mut s = SpriteInfo { name: name.to_string(), texture: t, tint_mod: [1.0; 3], ..Default::default() };
        f(&mut s);
        Rc::new(s)
    };
    let floor = |s: &mut SpriteInfo| s.flags.solidfloor = true;
    let grass = sprite("blends_natural_01_16", &floor);
    let boards = sprite("floors_interior_tilesandwood_01_16", &floor);
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
    let names = FxMaskSprites::SPRITES;
    let mask_sprites = FxMaskSprites {
        floor: Some(sprite(names[0], &|_| {})),
        wall_n: Some(sprite(names[1], &|_| {})),
        wall_w: Some(sprite(names[2], &|_| {})),
        wall_nw: Some(sprite(names[3], &|_| {})),
        wall_se: Some(sprite(names[4], &|s| s.flags.wall_se = true)),
        white: tm.shared_texture(&fs, FxMaskSprites::WHITE),
        ..Default::default()
    }
    .with_depth_maps(&maps);
    // Grass around a 6x6 house at (10..16, 10..16): walls on its west and
    // north squares, and on the north and west sides of the squares
    // beyond its south and east edges.
    let mut w = GridWorld::default();
    let (x0, x1) = (10, 16);
    w.buildings.insert(BuildingId(1), pz_iso_render::cutaways::BuildingDef { x: x0, y: x0, x2: x1, y2: x1, entirely_empty_outside: false });
    let obj = |s: &Rc<SpriteInfo>| RenderObject::new(ObjectKind::Plain, Some(s.clone()));
    for y in 0..32 {
        for x in 0..32 {
            let mut sq = RenderSquare::new(x, y, 0);
            let inside = (x0..x1).contains(&x) && (x0..x1).contains(&y);
            sq.cut.vision_blocked_below = true;
            if inside {
                sq.objects.push(obj(&boards));
                sq.cut.room = Some(1);
                sq.cut.building = Some(BuildingId(1));
                sq.cut.associated_building = Some(BuildingId(1));
            } else {
                let mut g = (*grass).clone();
                g.flags.exterior = true;
                sq.objects.push(obj(&Rc::new(g)));
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
    let player = [13, 13, 0];
    let mut cp = pz_iso_render::CutawayPlayer::at(13.5, 13.5, 0.0);
    cp.forward = [1.0, 1.0];
    let st = pz_iso_render::Cutaways::new().update(&w, &cp);
    let cut = (x0..x1).filter(|&i| st.cutaway_flags(x1, i, 0) & CUTAWAY_W != 0 || st.cutaway_flags(i, x1, 0) & CUTAWAY_N != 0).count();
    assert!(cut > 0, "no wall cut away");
    let textures = WeatherFxTextures::load(|path, flags| {
        if flags == 0 {
            tm.shared_texture(&fs, path)
        } else {
            tm.shared_texture_flags(&fs, path, flags)
        }
    });
    let rand = Rand::new(21);
    let mut fx = IsoWeatherFx::new(&rand, &textures);
    fx.update_fx(
        &WeatherFxClimate {
            precipitation_intensity: 1.0,
            precipitation_is_snow: false,
            wind_intensity: 0.3,
            wind_precip_intensity: 0.2,
            wind_angle_intensity: 0.4,
            ..Default::default()
        },
        &Default::default(),
    );
    let fxf = WeatherFxFrame { player_indoors: true, player_has_mask_to_draw: true, ..Default::default() };
    for _ in 0..400 {
        fx.update(&rand, &fxf);
    }
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(FW, FH) else {
        return;
    };
    let cam = Camera::centred(13.0, 13.0, 0.0, FW as i32, FH as i32, 1.0);
    let size = pz_iso_render::frame::FrameSize { width: FW, height: FH };
    for (name, cutaways, rain) in [
        ("game_whole_walls_rain", false, true),
        ("game_whole_walls_dry", false, false),
        ("game_cut_walls_rain", true, true),
        ("game_cut_walls_dry", true, false),
    ] {
        // Without cutaways nothing is cut, in the world or in the mask.
        let none = pz_iso_render::Cutaways::new().update(&GridWorld::default(), &cp);
        let state = if cutaways { &st } else { &none };
        let cv = cutaways.then_some(pz_iso_render::CutawayView { state, world: &w });
        let r = pz_iso_render::ChunkRenderer::new(cam, &maps, (FW, FH)).with_cutaways(cv);
        let mut chunks = Vec::new();
        for wy in 0..4 {
            for wx in 0..4 {
                let mut level = [None; 64];
                for (i, slot) in level.iter_mut().enumerate() {
                    *slot = w.squares.get(&(wx * 8 + (i % 8) as i32, wy * 8 + (i / 8) as i32, 0));
                }
                chunks.push(pz_iso_render::frame::ChunkLevels { texture: pz_iso_render::ChunkTexture::new(wx, wy, 0, 0, 0), levels: vec![(0, level)], dirty: true });
            }
        }
        let view = WeatherFxView::new(cam.off_x as f32, cam.off_y as f32, FW as i32, FH as i32, 1.0);
        let mw = GameWorld { w: &w, st: state };
        let mut mask = WeatherFxMask::default();
        mask.init_mask(&mw, FxMaskPlayer { square: Some(player), building: Some(BuildingId(1)), z: 0.0, master_region: None }, true);
        let cmds = pz_iso_render::frame::world_frame_into_with(&r, size, &chunks, pz_render::ScreenParams { desaturation: 0.0, timer: 0.0 }, Vec::new(), &mut |out| {
            if rain {
                render_weather_fx(&mut fx, &mut mask, &mw, &mask_sprites, &view, &FxMaskFrame::default(), out);
            }
        });
        let list = DrawList { cmds };
        gpu.render_frame(&[(pz_gpu::FrameParams::ui(FW, FH), &list)], &mut tm);
        write_bmp_sized(&std::path::Path::new(&dir).join(format!("{name}.bmp")), &gpu.read_pixels(), FW, FH);
    }
}
