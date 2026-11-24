use super::*;
use pz_iso_render::frame::world_frame_extras;
use pz_render::{DrawCmd, ScreenParams, Texture};
use pz_world::objects::ObjectKind;
use pz_world::sprite::test_tables;
use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoObject, IsoSpriteManager, ObjectSlot};
use pz_world_render::{ClimateInput, ClimateSettings, PlayerRenderInput};
use std::path::Path;

#[test]
fn world_extras_updates_fog_flies_and_drains_vispoly_invalidations() {
    let path = Path::new("unused-extras-fixture");
    let fs = GameFs::new(path);
    let mut tm = TextureManager::new();
    let mut extras = WorldExtras::new(&fs, &mut tm);
    assert!(extras.white.is_some());

    // Advance CorpseFlies by 60 ms -> advances 1 frame.
    let frame_before = extras.flies.frame;
    extras.update_tick(1.0, 0.06);
    assert_eq!(extras.flies.frame, (frame_before + 1) % 56);

    // Draining `PendingRender::vispoly` bumps `dirty_obstacle_counter`.
    let mut pending = PendingRender::new();
    pending.vispoly.insert((2, 3, 0));
    let before = extras.vispoly.dirty_obstacle_counter;
    extras.drain_vispoly(&mut pending);
    assert!(pending.vispoly.is_empty());
    assert_eq!(extras.vispoly.dirty_obstacle_counter, before + 1);
}

#[test]
fn build_chunk_vispoly_extracts_solid_squares_and_vision_blocking_walls() {
    let mut sprites = IsoSpriteManager::new(test_tables());
    let solid_idx = sprites.add_sprite("solid_box", 900);
    let f = sprites.tables.flag;
    sprites.sprites[solid_idx].properties.set_flag(f.solid);
    let wall_n_idx = sprites.add_sprite("wall_north", 901);
    sprites.sprites[wall_n_idx].properties.set_flag(f.collideN);
    sprites.sprites[wall_n_idx].cut_n = true;
    if let Some(wall_n) = sprites.tables.flag_from_string("WallN") {
        sprites.sprites[wall_n_idx].properties.set_flag(wall_n);
    }

    let mut chunk = IsoChunk::new(0, 0);
    let mut sq_solid = IsoGridSquare::new(2, 2, 0);
    sq_solid.objects.push(ObjectSlot::new(IsoObject::new(
        1,
        ObjectKind::Object,
        Some(solid_idx.into()),
    )));
    let mut sq_wall = IsoGridSquare::new(3, 4, 0);
    sq_wall.objects.push(ObjectSlot::new(IsoObject::new(
        2,
        ObjectKind::Object,
        Some(wall_n_idx.into()),
    )));
    chunk.set_square(2, 2, 0, Some(sq_solid.into()));
    chunk.set_square(3, 4, 0, Some(sq_wall.into()));
    chunk.set_square(3, 3, 0, Some(IsoGridSquare::new(3, 3, 0).into()));

    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    let meta = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    cell.recalc_chunk_squares(0, 0, &meta, &sprites);

    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &sprites,
    };
    let data = build_chunk_vispoly(&view, &cell.chunks[&(0, 0)], 0, 0, 0);
    assert!(
        data.solid_squares.contains(&(2, 2)),
        "solid square (2,2) should enter solid_squares: {data:?}"
    );
    assert!(
        data.all_walls.iter().any(|w| w.is_horizontal() && w.y1 == 4 && w.x1 <= 3 && w.x2 > 3),
        "north wall on (3,4) should enter horizontal all_walls: {data:?}"
    );
}

#[test]
fn frame_extras_emits_fog_and_vision_polygon_commands_in_world_frame() {
    let path = Path::new("unused-extras-frame-fixture");
    let mut fs = GameFs::new(path);
    let mut tm = TextureManager::new();
    let mut renderer = crate::world_render::WorldRender::new(path, &mut fs, &mut tm);
    // Supply a ready synthetic fog noise texture and active fog alpha.
    let mut noise = Texture::empty("fog_noise");
    noise.width = 64;
    noise.height = 64;
    noise.ready = true;
    renderer.extras.fog_noise = Some(noise);
    renderer.extras.fog.init(0.8);

    let mut chunk = IsoChunk::new(0, 0);
    for y in 0..4 {
        for x in 0..4 {
            chunk.set_square(x, y, 0, Some(IsoGridSquare::new(x, y, 0).into()));
        }
    }
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    let meta = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    let sprites = IsoSpriteManager::new(test_tables());
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &sprites,
    };
    renderer.extras.on_chunk_added();

    let mut v = RenderView {
        split_view: None,
        player: 0,
        x: 2.5,
        y: 2.5,
        z: 0.0,
        dir: pz_world::square::IsoDirections::S,
        chunk_map: (0, 0, 1),
        width: 320,
        height: 240,
        zoom: 1.0,
        climate: ClimateSettings {
            climate: ClimateInput {
                global_light_exterior: [1.0; 4],
                global_light_interior: [1.0; 4],
                ambient: 1.0,
                day_light_strength: 1.0,
                night_strength: 0.0,
                desaturation: 0.0,
                view_distance: 30.0,
                fog_intensity: 0.8,
                moon_float: 0.0,
                game_night: 0.0,
                night_darkness: 3,
                weather_shader: false,
                last_stand: false,
            },
            players: [PlayerRenderInput::default(); 4],
        },
        screen: ScreenParams::default(),
        tick: 0,
        multiplier: 1.0,
    };
    let cam = Camera::player(v.x, v.y, v.z, v.width as i32, v.height as i32, v.zoom);
    let mut extras = renderer.extras.build_frame(&view, &cam, &v, &renderer.lighting, &[(0, 0)]);
    let size = pz_iso_render::frame::FrameSize { width: v.width, height: v.height };
    let depth_mgr = pz_render::tile_depth::TileDepthTextureManager::init(&mut fs, &mut tm);
    let maps = pz_iso_render::seam::WorldDepthMaps {
        tiles: &depth_mgr,
        seams: Vec::new(),
        cutaways: Vec::new(),
    };
    let r = pz_iso_render::ChunkRenderer::new(cam, &maps, (v.width, v.height));
    let mut late = |_out: &mut Vec<DrawCmd>| {};
    let cmds = world_frame_extras(
        &r,
        size,
        &[],
        &pz_iso_render::objects::FrameObjects::default(),
        None,
        &mut extras,
        v.screen,
        Vec::new(),
        &mut late,
    );
    assert!(
        cmds.iter().any(|c| matches!(c, DrawCmd::VisibilityPolygon(_))),
        "expected VisibilityPolygon draw command from VisionFrame"
    );
    assert!(
        cmds.iter().any(|c| matches!(c, DrawCmd::ImprovedFog(_))),
        "expected ImprovedFog draw command from FogFrame"
    );
    let _ = &mut v;
}
