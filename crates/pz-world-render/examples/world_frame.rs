//! Renders a map area through pz-world, pz-world-render, pz-lighting and
//! pz-iso-render. Needs the installed game and its enum tables:
//!
//! world_frame <game dir> <map> <sprite tables> <object tables> <x> <y> <out.png> [unlit]
//!
//! One player stands at `(x, y, 0)`. It turns through the eight directions
//! (three frames each) and then faces east until the lighting settles, under
//! [`NoonSettings`].

use pz_fs::GameFs;
use pz_iso_render::frame::{world_frame, ChunkLevels, FrameSize};
use pz_iso_render::{Camera, ChunkRenderer, ChunkTexture};
use pz_lighting::jni::{IsoDirection, VisionConeInput};
use pz_render::tile_depth::{
    TileDepthTextureAssignmentManager, TileDepthTextureManager, TileGeometryManager,
};
use pz_render::TextureManager;
use pz_world::chunk::{ChunkLoadEnv, IsoCell};
use pz_world::fix2x::Fix2xTable;
use pz_world::{
    CreateParams, GameRand, IsoMetaGrid, IsoSpriteManager, ObjectLoadSettings, ObjectTables,
    SpriteTables, WorldGenBounds,
};
use pz_world_render::{
    default_player, GameTextures, NoWorldHooks, NoonSettings, PlayerFrameInput, RenderWorld,
    SpriteCache, WorldLighting, WorldView,
};

struct Zero;
impl GameRand for Zero {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 8 {
        eprintln!("usage: world_frame <game dir> <map> <sprite tables> <object tables> <x> <y> <out.png> [unlit]");
        std::process::exit(2);
    }
    let unlit = a.get(8).is_some_and(|s| s == "unlit");
    let game = std::path::Path::new(&a[1]);
    let mut fs = GameFs::new(game);
    let mut tm = TextureManager::new();
    for pack in [
        "Tiles2x",
        "Overlays2x",
        "JumboTrees2x",
        "JumboTreesBigs2x",
        "Tiles2x.floor",
        "B42ChunkCaching2x",
        "B42ChunkCaching2x.floor",
    ] {
        tm.load_texture_pack(&fs, pack, 0x40, None);
    }
    tm.set_texture_pack_lookup(&[]);
    let depth = TileDepthTextureManager::init(&mut fs, &mut tm);
    let assignments = TileDepthTextureAssignmentManager::init(&mut fs);
    let geometry = TileGeometryManager::from_media_dirs([game.join("media").as_path()]);
    let meta = IsoMetaGrid::create(
        &fs,
        CreateParams {
            game_map: &a[2],
            world_gen: WorldGenBounds::default(),
            rand: &mut Zero,
            voronois: Vec::new(),
        },
    )
    .expect("meta grid");
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[3]).unwrap()).unwrap();
    let objects = ObjectTables::from_text(&std::fs::read_to_string(&a[4]).unwrap()).unwrap();
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let (cx, cy): (f32, f32) = (a[5].parse().unwrap(), a[6].parse().unwrap());
    let (w, h) = (1280u32, 720u32);
    let cam = Camera::centred(cx, cy, 0.0, w as i32, h as i32, 1.0);
    let (wx0, wy0, n) = (cam.chunk_x() - 4, cam.chunk_y() - 4, 9);

    // Load and recalculate the chunks (x outer, y inner, like
    // `checkNewlyOnScreenChunks`).
    let fix2x = Fix2xTable::default();
    let mut cell = IsoCell::default();
    let mut order = Vec::new();
    for wx in wx0..wx0 + n {
        for wy in wy0..wy0 + n {
            let mut env = ChunkLoadEnv {
                sprites: &mut sprites,
                fix2x: &fix2x,
                rand: &mut Zero,
                tables: &objects,
                settings: ObjectLoadSettings::default(),
                hooks: &mut pz_world::NoHooks,
            };
            cell.load_chunk(&meta, wx, wy, &mut env);
            cell.recalc_chunk_squares(wx, wy, &meta, &sprites);
            order.push((wx, wy));
        }
    }

    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &sprites,
    };
    let mut tex = GameTextures {
        fs: &fs,
        textures: &mut tm,
        depth: &depth,
        assignments: &assignments,
        geometry: &geometry,
    };
    let mut cache = SpriteCache::new();
    let mut world = RenderWorld::new();
    world.set_buildings(&meta);
    let hooks = NoWorldHooks;
    let mut lighting = WorldLighting::new();
    for &(wx, wy) in &order {
        world.build_chunk(&view, wx, wy, &mut cache, &mut tex);
        lighting.sync_chunk(&view, wx, wy, &hooks);
    }

    if !unlit {
        let mut settings = NoonSettings::default();
        let mut schedule: Vec<IsoDirection> =
            IsoDirection::ALL.iter().flat_map(|&d| [d; 3]).collect();
        schedule.extend([IsoDirection::E; 60]);
        let mut changed = Vec::new();
        for (f, dir) in schedule.into_iter().enumerate() {
            let input = PlayerFrameInput {
                index: 0,
                player: default_player(cx, cy, 0.0, dir),
                cone: VisionConeInput {
                    worn_items_vision_multiplier: 1.0,
                    ..Default::default()
                },
                chunk_x_min: wx0,
                chunk_y_min: wy0,
                chunk_grid_width: n,
            };
            let r = lighting
                .frame(
                    &view,
                    &input,
                    &mut settings,
                    &hooks,
                    f as u64 * 33_000_000,
                    &|_, _, _| true,
                )
                .expect("lighting");
            changed.push(r.dirty_levels.len());
        }
        eprintln!("dirty levels per frame: {changed:?}");
        lighting.apply(0, &mut world);
        let black = world
            .squares
            .values()
            .filter(|s| s.light_level == 0.0)
            .count();
        eprintln!("{} squares, {black} unlit", world.squares.len());
    }

    let mut chunks = Vec::new();
    let inputs = world.chunk_inputs(&order);
    for c in &inputs {
        let mut z = pz_render::iso::calculate_min_level(c.min_level);
        while z <= c.max_level {
            let texture =
                ChunkTexture::new(c.wx, c.wy, z.max(c.min_level), c.min_level, c.max_level);
            let levels = c
                .levels
                .iter()
                .filter(|(l, _)| *l >= texture.min_level && *l <= texture.top_level)
                .cloned()
                .collect();
            chunks.push(ChunkLevels {
                texture,
                levels,
                dirty: true,
            });
            z += 2;
        }
    }
    let sheet = tm
        .shared_texture_flags(&fs, "media/depthmaps/SEAMS_01.png", 0)
        .expect("seams");
    let cutaways: Vec<_> = [
        "media/wallcutaways.png",
        "media/windowframe_cutaways.png",
        "media/windowframe_cutaways_2.png",
        "media/windowframe_cutaways_3.png",
    ]
    .iter()
    .filter_map(|p| tm.shared_texture_flags(&fs, p, 3))
    .collect();
    let maps = pz_iso_render::seam::WorldDepthMaps {
        tiles: &depth,
        seams: pz_iso_render::seam::seam_masks(&sheet),
        cutaways,
    };
    let player = pz_iso_render::cutaways::CutawayPlayer {
        x: cx,
        y: cy,
        z: 0.0,
        forward: [1.0, 0.0],
        aiming: false,
        mouse_poi: None,
        peeking: None,
        occlusion_bounds: Some(pz_iso_render::occlusion::grid_bounds(
            &cam, w as i32, h as i32,
        )),
    };
    let state = pz_iso_render::cutaways::Cutaways::new().update(&world, &player);
    let cv = pz_iso_render::alpha::CutawayView {
        state: &state,
        world: &world,
    };
    let r = ChunkRenderer::new(cam, &maps, (w, h)).with_cutaways(Some(cv));
    let cmds = world_frame(
        &r,
        FrameSize {
            width: w,
            height: h,
        },
        &chunks,
        pz_render::ScreenParams {
            desaturation: 0.0,
            timer: 0.0,
        },
    );
    let list = pz_render::DrawList { cmds };
    let mut gpu = pz_gpu::Gpu::new_headless(w, h).expect("gpu");
    gpu.render_frame(
        &[(
            pz_gpu::FrameParams {
                clear: Some([0.0, 0.0, 0.0, 1.0]),
                ..pz_gpu::FrameParams::ui(w, h)
            },
            &list,
        )],
        &mut tm,
    );
    let px = gpu.read_pixels();
    let file = std::fs::File::create(&a[7]).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&px).unwrap();
}
