//! Water, puddles, snow and blood on synthetic squares, drawn through
//! pz-gpu and read back.

use std::collections::HashMap;
use std::rc::Rc;

use pz_iso_render::cutaways::GridWorld;
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
    Surfaces { settings, climate, water: WaterState::default(), puddles, sky: SkyReflection::default(), world_age_hours: 0.0, meta_grid_min_x: 0, world, textures }
}

/// Chunk (0, 0) level 0 recorded and composited into the main target,
/// then the per-frame surface passes as `world_frame` orders them.
fn draw(gpu: &mut pz_gpu::Gpu, tm: &mut TextureManager, r: &ChunkRenderer<'_>, world: &GridWorld) -> Vec<u8> {
    let mut level = [None; 64];
    for sq in world.squares.values().filter(|s| s.z == 0 && (0..8).contains(&s.x) && (0..8).contains(&s.y)) {
        level[(sq.x + sq.y * 8) as usize] = Some(sq);
    }
    let tex = ChunkTexture::new(0, 0, 0, 0, 0);
    let mut cmds = Vec::new();
    r.record(tex, &[(0, level)], &mut cmds);
    cmds.push(DrawCmd::DepthMask(true));
    cmds.push(DrawCmd::Clear(pz_render::gl::DEPTH_BUFFER_BIT));
    r.composite(&[tex], W, H, &mut cmds);
    let squares: Vec<&RenderSquare> = level.iter().flatten().copied().collect();
    let s = r.surfaces.as_ref().expect("surfaces");
    pz_iso_render::water::render_water(r, s, &squares, &mut cmds);
    r.render_translucent_floors(&squares, 0, &mut cmds);
    pz_iso_render::water::render_water_shore(r, s, &squares, &mut cmds);
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
#[test]
fn water_covers_the_water_tile_and_the_shore_fades_into_it() {
    let Ok(mut gpu) = pz_gpu::Gpu::new_headless(W, H) else { return };
    let mut tm = TextureManager::new();
    let red = floor_tex(&mut tm, "water", [255, 0, 0, 255]);
    let green = floor_tex(&mut tm, "grass", [0, 200, 0, 255]);
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
    assert!(east[1] > 170 && east[0] < 30, "{east:?}");
    assert!(west[0] > 100 && west[2] > 100, "{west:?}");
}