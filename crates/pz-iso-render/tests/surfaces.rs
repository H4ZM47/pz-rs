//! Water, puddles, snow and blood on synthetic squares, drawn through
//! pz-gpu and read back.

use std::collections::HashMap;
use std::rc::Rc;

use pz_iso_render::blood::{ChunkBlood, FloorBloodSplat};
use pz_iso_render::cutaways::GridWorld;
use pz_iso_render::puddles::{PuddleSquare, PuddlesGeometry};
use pz_iso_render::water::{WaterGeometry, WaterInit};
use pz_iso_render::weather::{ClimateInputs, PuddlesState, SkyReflection, SurfaceSettings, SurfaceTextures, Surfaces, WaterState};
use pz_iso_render::{Camera, ChunkRenderer, ChunkTexture, DepthMaps, DepthPreset, ObjectKind, RenderObject, RenderSquare, SpriteFlags, SpriteInfo};
use pz_render::{flags, DrawCmd, Texture, TextureManager};

const W: u32 = 512;
const H: u32 = 256;

/// Every depth map is the floor's: tiles sit at their square's far depth.
struct Maps(Texture);

impl DepthMaps for Maps {
    fn preset(&self, _: DepthPreset) -> Option<Texture> {
        Some(self.0.clone())
    }
    fn default_texture(&self) -> Option<Texture> {
        Some(self.0.clone())
    }
}

struct Textures(HashMap<String, Texture>);

impl SurfaceTextures for Textures {
    fn texture(&self, name: &str) -> Option<Texture> {
        self.0.get(name).cloned()
    }
}

fn solid(tm: &mut TextureManager, name: &str, w: i32, h: i32, rgba: [u8; 4]) -> Texture {
    tm.texture_from_pixels(name, w, h, rgba.repeat((w * h) as usize), flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST)
}

/// A floor tile: a diamond in the bottom 64 rows of a 128x256 frame.
fn floor_tex(tm: &mut TextureManager, name: &str, rgba: [u8; 4]) -> Texture {
    let mut px = Vec::with_capacity(128 * 64 * 4);
    for y in 0..64 {
        for x in 0..128 {
            let inside = ((x as f32 + 0.5 - 64.0).abs() / 64.0 + (y as f32 + 0.5 - 32.0).abs() / 32.0) <= 1.0;
            px.extend_from_slice(&if inside { rgba } else { [0; 4] });
        }
    }
    let mut t = tm.texture_from_pixels(name, 128, 64, px, flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST);
    t.offset_y = 192.0;
    t.width_orig = 128;
    t.height_orig = 256;
    t
}

/// The floor depth map: 1 inside the floor diamond, 0 (no depth, the
/// texel discarded) elsewhere, as the floor preset is.
fn depth_map(tm: &mut TextureManager) -> Texture {
    let mut px = Vec::with_capacity(128 * 256 * 4);
    for y in 0..256 {
        for x in 0..128 {
            let inside = y >= 192 && ((x as f32 + 0.5 - 64.0).abs() / 64.0 + (y as f32 - 192.0 + 0.5 - 32.0).abs() / 32.0) <= 1.0;
            px.extend_from_slice(&if inside { [255; 4] } else { [0, 0, 0, 255] });
        }
    }
    let mut d = tm.texture_from_pixels("depth", 128, 256, px, flags::FILTER_MIN_NEAREST | flags::FILTER_MAG_NEAREST);
    d.width_orig = 128;
    d.height_orig = 256;
    d
}

fn sprite(name: &str, tex: Texture, flags: SpriteFlags) -> Rc<SpriteInfo> {
    Rc::new(SpriteInfo { name: name.into(), texture: Some(tex), flags, tint_mod: [1.0; 3], ..Default::default() })
}

fn ground(name: &str, tex: Texture) -> Rc<SpriteInfo> {
    sprite(name, tex, SpriteFlags { solidfloor: true, exterior: true, ..Default::default() })
}

fn surfaces<'a>(world: &'a GridWorld, textures: &'a Textures, settings: SurfaceSettings, climate: ClimateInputs, puddles: PuddlesState) -> Surfaces<'a> {
    Surfaces { settings, climate, water: WaterState::default(), puddles, sky: SkyReflection::default(), world_age_hours: 0.0, meta_grid_min_x: 0, world, textures, frame: Default::default(), rain: None, fish: None }
}

/// Chunk (0, 0) level 0 recorded and composited into the main target,
/// then the per-frame surface passes as `world_frame` orders them.
fn draw(gpu: &mut pz_gpu::Gpu, tm: &mut TextureManager, r: &ChunkRenderer<'_>, world: &GridWorld) -> Vec<u8> {
    draw_after(gpu, tm, r, world, Vec::new())
}

/// [`draw`] after the commands in `cmds`.
fn draw_after(gpu: &mut pz_gpu::Gpu, tm: &mut TextureManager, r: &ChunkRenderer<'_>, world: &GridWorld, mut cmds: Vec<DrawCmd>) -> Vec<u8> {
    let mut level = [None; 64];
    for sq in world.squares.values().filter(|s| s.z == 0 && (0..8).contains(&s.x) && (0..8).contains(&s.y)) {
        level[(sq.x + sq.y * 8) as usize] = Some(sq);
    }
    let tex = ChunkTexture::new(0, 0, 0, 0, 0);
    r.record(tex, &[(0, level)], &mut cmds);
    cmds.push(DrawCmd::DepthMask(true));
    cmds.push(DrawCmd::Clear(pz_render::gl::DEPTH_BUFFER_BIT));
    r.composite(&[tex], W, H, &mut cmds);
    let squares: Vec<&RenderSquare> = level.iter().flatten().copied().collect();
    let s = r.surfaces.as_ref().expect("surfaces");
    if s.settings.puddles_quality < 2 {
        pz_iso_render::puddles::render_puddles(r, s, &squares, &mut cmds);
    }
    pz_iso_render::water::render_water(r, s, &squares, &mut cmds);
    r.render_translucent_floors(&squares, 0, &mut cmds);
    pz_iso_render::puddles::render_puddles_translucent_floors(r, s, &squares, 0, &mut cmds);
    pz_iso_render::water::render_water_shore(r, s, &squares, &mut cmds);
    pz_iso_render::splashes::render_rain_splashes(r, s, 0, 0, 0, &mut cmds);
    r.render_translucent_objects(&squares, 0, &mut cmds);
    let list = pz_render::DrawList { cmds };
    let params = pz_gpu::FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..pz_gpu::FrameParams::ui(W, H) };
    gpu.render_frame(&[(params, &list)], tm);
    gpu.read_pixels()
}

fn px(p: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    p[i..i + 4].try_into().unwrap()
}

/// The camera on square (4, 4): its diamond's top corner is at (256, 128),
/// square (5, 4)'s at (320, 160).
fn cam() -> Camera {
    let c = Camera::centred(4.0, 4.0, 0.0, W as i32, H as i32, 1.0);
    assert_eq!((c.off_x, c.off_y), (-256, 128));
    c
}

/// Open water replaces the water tile; the shore's floor fades towards
/// the water and the shore water fills in.
/// Water on square (4, 4) in grass, its neighbours shores.
fn water_world(tm: &mut TextureManager) -> GridWorld {
    let red = floor_tex(tm, "water", [255, 0, 0, 255]);
    let green = floor_tex(tm, "grass", [0, 200, 0, 255]);
    let water_sprite = sprite("blends_natural_02_0", red, SpriteFlags { solidfloor: true, water: true, exterior: true, ..Default::default() });
    let grass = ground("blends_natural_01_16", green);
    let mut world = GridWorld::default();
    let is_water = |x: i32, y: i32| (x, y) == (4, 4);
    for y in 2..7 {
        for x in 2..8 {
            let mut sq = RenderSquare::new(x, y, 0);
            let s = if is_water(x, y) { water_sprite.clone() } else { grass.clone() };
            sq.objects.push(RenderObject::new(ObjectKind::Plain, Some(s.clone())));
            let n = [(-1, 0), (-1, -1), (0, -1), (-1, 1), (0, 1), (1, 1), (1, 0), (1, -1)].map(|(dx, dy)| Some(is_water(x + dx, y + dy)));
            sq.water = WaterGeometry::init(&WaterInit { water: is_water(x, y), exterior: true, shore_zone: 1, floor_sprite: Some(&s.name), neighbours: n, flow: [(0.0, 0.0); 4], shader: true }).filter(|w| w.is_valid());
            world.squares.insert((x, y, 0), sq);
        }
    }
    world
}

#[test]
fn water_covers_the_water_tile_and_the_shore_fades_into_it() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else { return };
    let mut tm = TextureManager::new();
    let world = water_world(&mut tm);
    assert!(world.squares[&(5, 4, 0)].water.is_some_and(|w| w.shore));
    let depth = depth_map(&mut tm);
    let maps = Maps(depth);
    let textures = Textures(HashMap::new());
    let s = surfaces(&world, &textures, SurfaceSettings::default(), ClimateInputs::default(), PuddlesState::default());
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(s));
    let p = draw(&mut gpu, &mut tm, &r, &world);
    // The water square's centre: water (blue-green, brightened by the
    // white stand-in reflection), not the red tile.
    let c = px(&p, 256, 160);
    assert!(c[2] >= c[0] && c[1] > 100, "{c:?}");
    // A dry square away from the water: plain grass.
    assert_eq!(px(&p, 256 - 128, 160 + 64), [0, 200, 0, 255]);
    // The shore (5, 4): grass at its east corner, water at its west corner.
    let east = px(&p, 378, 192);
    let west = px(&p, 262, 192);
    assert!(west[0] > 100 && west[2] > 100, "{west:?}");
    // Procedural bank noise changes the exact blend across GPU backends.
    // Compare the water contribution at both ends instead of fixing its
    // amount: the east remains grass and has less red/blue water than the
    // west. Uniform, reversed or missing shore blending still fails. The
    // dry RGBA endpoint stays exact.
    assert!(
        east[1] > 170 && east[0] < west[0] && east[2] < west[2],
        "east={east:?}, west={west:?}"
    );
}

fn puddle_world(tm: &mut TextureManager) -> GridWorld {
    let floor = floor_tex(tm, "floor", [200, 100, 50, 255]);
    let road = ground("street", floor);
    let mut world = GridWorld::default();
    for y in 2..7 {
        for x in 2..7 {
            let mut sq = RenderSquare::new(x, y, 0);
            sq.objects.push(RenderObject::new(ObjectKind::Plain, Some(road.clone())));
            let p = PuddleSquare { exterior: true, puddles_dir: 8, ..Default::default() };
            sq.puddles = Some(PuddlesGeometry::init(&p, 0, |_, _| Some(p)));
            world.squares.insert((x, y, 0), sq);
        }
    }
    world
}

fn wet() -> PuddlesState {
    let mut p = PuddlesState::default();
    p.wet_ground.set(1.0);
    p.puddles_size.set(0.7);
    p
}

/// Low-quality puddles go into the chunk texture: with a white height
/// map the puddle is `mix(dark, reflection, 0.8)` at alpha 0.5 over the
/// floor.
#[test]
fn low_quality_puddles_are_recorded_into_the_chunk_texture() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else { return };
    let mut tm = TextureManager::new();
    let world = puddle_world(&mut tm);
    let hm = solid(&mut tm, "hm", 8, 8, [255, 255, 255, 255]);
    let maps = Maps(depth_map(&mut tm));
    let textures = Textures(HashMap::from([("media/textures/puddles_hm.png".to_string(), hm)]));
    let settings = SurfaceSettings { puddles_quality: 2, ..Default::default() };
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(surfaces(&world, &textures, settings, ClimateInputs::default(), wet())));
    let p = draw(&mut gpu, &mut tm, &r, &world);
    let c = px(&p, 256, 160);
    let want = [153, 103, 86];
    assert!(c[..3].iter().zip(want).all(|(a, b)| (i32::from(*a) - b).abs() <= 2), "{c:?}");
    // Dry ground: the floor alone.
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(surfaces(&world, &textures, settings, ClimateInputs::default(), PuddlesState::default())));
    let p = draw(&mut gpu, &mut tm, &r, &world);
    assert_eq!(px(&p, 256, 160), [200, 100, 50, 255]);
}

/// Medium-quality puddles are drawn per frame over the composited floor.
#[test]
fn puddles_are_drawn_per_frame_below_quality_two() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else { return };
    let mut tm = TextureManager::new();
    let world = puddle_world(&mut tm);
    let maps = Maps(depth_map(&mut tm));
    let textures = Textures(HashMap::new());
    let settings = SurfaceSettings { puddles_quality: 1, ..Default::default() };
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(surfaces(&world, &textures, settings, ClimateInputs::default(), wet())));
    let mut cmds = Vec::new();
    let sq: Vec<&RenderSquare> = world.squares.values().collect();
    pz_iso_render::puddles::render_puddles(&r, r.surfaces.as_ref().unwrap(), &sq, &mut cmds);
    let Some(DrawCmd::Surface(d)) = cmds.first() else { panic!("no puddle draw") };
    assert_eq!(d.program, pz_render::SurfaceProgram::PuddlesMq);
    assert_eq!(d.quads.len(), 25);
    let p = draw(&mut gpu, &mut tm, &r, &world);
    // Wet ground darkens the whole floor.
    let c = px(&p, 256, 160);
    assert!(c != [200, 100, 50, 255] && c[0] < 200, "{c:?}");
    // Snow anywhere stops puddles.
    let snowy = ClimateInputs { snow_target: 50, ..Default::default() };
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(surfaces(&world, &textures, settings, snowy, wet())));
    let mut cmds = Vec::new();
    pz_iso_render::puddles::render_puddles(&r, r.surfaces.as_ref().unwrap(), &sq, &mut cmds);
    assert!(cmds.is_empty());
}

/// Full winter snow covers every outdoor floor with the square tiles.
#[test]
fn snow_covers_outdoor_floors_in_winter() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else { return };
    let mut tm = TextureManager::new();
    let world = puddle_world(&mut tm);
    let maps = Maps(depth_map(&mut tm));
    let mut map = HashMap::new();
    for i in 0..48 {
        map.insert(format!("e_newsnow_ground_1_{i}"), floor_tex(&mut tm, &format!("snow{i}"), [250, 250, 255, 255]));
    }
    let textures = Textures(map);
    let winter = ClimateInputs { snow_target: 100, winter: true, ..Default::default() };
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(surfaces(&world, &textures, SurfaceSettings::default(), winter, PuddlesState::default())));
    let p = draw(&mut gpu, &mut tm, &r, &world);
    assert_eq!(px(&p, 256, 160), [250, 250, 255, 255]);
    // Autumn: no snow.
    let autumn = ClimateInputs { snow_target: 100, winter: false, ..Default::default() };
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(surfaces(&world, &textures, SurfaceSettings::default(), autumn, PuddlesState::default())));
    let p = draw(&mut gpu, &mut tm, &r, &world);
    assert_eq!(px(&p, 256, 160), [200, 100, 50, 255]);
}

/// A blood splat is recorded into the chunk texture over the floor,
/// including one in the neighbouring chunk within a square of the edge.
#[test]
fn blood_splats_darken_the_floor_where_they_lie() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else { return };
    let mut tm = TextureManager::new();
    let mut world = puddle_world(&mut tm);
    for sq in world.squares.values_mut() {
        sq.puddles = None;
    }
    let maps = Maps(depth_map(&mut tm));
    let textures = Textures(HashMap::from([("blood_floor_small_01".to_string(), solid(&mut tm, "blood", 32, 16, [120, 0, 0, 255]))]));
    let splat = FloorBloodSplat { x: 4.5, y: 4.5, z: 0.0, kind: 0, world_age: 0.0, index: 1, fade: 0 };
    let fading = FloorBloodSplat { x: 2.5, y: 2.5, fade: 2, ..splat };
    world.blood.insert((0, 0), ChunkBlood { fade: vec![fading].into(), splats: vec![splat], next_splat_index: 0 });
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(surfaces(&world, &textures, SurfaceSettings::default(), ClimateInputs::default(), PuddlesState::default())));
    let p = draw(&mut gpu, &mut tm, &r, &world);
    // The splat's centre: (4.5, 4.5) is (256, 160); the texture is placed
    // by its full size times the tile scale, drawn at its own size.
    let at = px(&p, 256 - 32 + 16, 160 - 16 + 8);
    assert!(at[0] < 200 && at[1] < 100, "{at:?}");
    assert_eq!(px(&p, 256, 200), [200, 100, 50, 255]);
}

/// With the sky rendered, the water reflects it instead of the white
/// stand-in: darker and bluer.
#[test]
fn water_reflects_the_rendered_sky() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else { return };
    let mut tm = TextureManager::new();
    let world = water_world(&mut tm);
    let maps = Maps(depth_map(&mut tm));
    let textures = Textures(HashMap::new());
    let plain = {
        let s = surfaces(&world, &textures, SurfaceSettings::default(), ClimateInputs::default(), PuddlesState::default());
        let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(s));
        px(&draw(&mut gpu, &mut tm, &r, &world), 256, 160)
    };
    let mut sky = pz_iso_render::skybox::SkyBox::new(0);
    sky.update(&pz_iso_render::skybox::SkyInputs {
        time_of_day: 12.0,
        dawn: 6.0,
        dusk: 20.0,
        day_high_noon: 13.0,
        exterior_light: [1.0, 0.95, 0.9, 1.0],
        night_strength: 0.0,
        cloud_intensity: 0.0,
        precipitation_intensity: 0.0,
        temperature: 20.0,
        fog_intensity: 0.0,
        wind_angle_intensity: 0.0,
        wind_intensity: 0.0,
    });
    let cmd = sky.render(1001, 1.0, true, 1).expect("sky render");
    let mut s = surfaces(&world, &textures, SurfaceSettings::default(), ClimateInputs::default(), PuddlesState::default());
    // Mid-way through the blend between the two textures; both hold sky
    // once rendered twice.
    let cmd2 = sky.render(2002, 1.0, true, 1).expect("sky render");
    s.sky = sky.reflection(2502, true, 1, 0.0, &textures);
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(s));
    let c = px(&draw_after(&mut gpu, &mut tm, &r, &world, vec![cmd, cmd2]), 256, 160);
    assert!(c[0] < plain[0] && c[2] >= c[0], "{c:?} vs {plain:?}");
}

/// A rain splash in flight on an outdoor floor, at its square position.
#[test]
fn rain_splashes_land_on_outdoor_floors() {
    use pz_iso_render::splashes::{ChunkLevelRain, RainSplashes};
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else { return };
    let mut tm = TextureManager::new();
    let world = puddle_world(&mut tm);
    let maps = Maps(depth_map(&mut tm));
    let textures = Textures(HashMap::from([("RainSplash_00_0".to_string(), solid(&mut tm, "splash", 16, 16, [255, 255, 255, 255]))]));
    let mut level = ChunkLevelRain { raining: true, frame_num: 7, ..Default::default() };
    // Square (4, 4): splash frame 0 at its centre.
    level.frame[36] = 0.0;
    level.splash_xy[36] = (0.5, 0.5);
    let rain = std::cell::RefCell::new(RainSplashes { levels: HashMap::from([((0, 0, 0), level)]) });
    let wet = ClimateInputs { cell_rain_intensity: 5, ..Default::default() };
    let mut s = surfaces(&world, &textures, SurfaceSettings::default(), wet, PuddlesState::default());
    s.frame.frame_count = 7;
    s.rain = Some(&rain);
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(s));
    let p = draw(&mut gpu, &mut tm, &r, &world);
    // 0.6 of (0.8, 0.9, 1.0) over the floor.
    let c = px(&p, 256, 160);
    let want = [202, 177, 173];
    assert!(c[..3].iter().zip(want).all(|(a, b)| (i32::from(*a) - b).abs() <= 2), "{c:?}");
    assert_eq!(px(&p, 256, 200), [200, 100, 50, 255]);
}

/// A fish splash on open water, drawn after the water.
#[test]
fn fish_splashes_show_on_open_water() {
    use pz_iso_render::splashes::{FishSplashes, FrameState};
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else { return };
    let mut tm = TextureManager::new();
    let world = water_world(&mut tm);
    let maps = Maps(depth_map(&mut tm));
    let splash = solid(&mut tm, "fish", 64, 64, [255, 255, 255, 255]);
    let textures = Textures(HashMap::from([("media/textures/waterSplashes/WaterSplashSmall0_0.png".to_string(), splash)]));
    let frame = FrameState { frame_count: 1, paused: false, unpaused_accumulator: 10.0 };
    let fish = std::cell::RefCell::new(FishSplashes::default());
    fish.borrow_mut().start(4, 4, 0, false, Some((0.0, 0.0)), true, &frame);
    let mut s = surfaces(&world, &textures, SurfaceSettings::default(), ClimateInputs::default(), PuddlesState::default());
    s.frame = frame;
    s.fish = Some(&fish);
    let r = ChunkRenderer::new(cam(), &maps, (W, H)).with_surfaces(Some(s));
    let p = draw(&mut gpu, &mut tm, &r, &world);
    assert_eq!(px(&p, 256, 160), [204, 229, 255, 255]);
}
