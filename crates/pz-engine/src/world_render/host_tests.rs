use super::*;

#[test]
fn failed_chunk_input_keeps_current_and_later_notifications_for_retry() {
    use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoSpriteManager};
    let path = std::path::Path::new("unused-render-host-fixture");
    let mut fs = GameFs::new(path);
    let mut textures = TextureManager::new();
    let mut renderer = WorldRender::new(path, &mut fs, &mut textures);
    let mut first = IsoChunk::new(0, 0);
    first.set_square(1, 1, 0, Some(IsoGridSquare::new(1, 1, 0).into()));
    let square = first.get_grid_square(1, 1, 0).unwrap();
    let mut later = IsoChunk::new(1, 0);
    later.set_square(0, 1, 0, Some(IsoGridSquare::new(8, 1, 0).into()));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), first);
    cell.chunks.insert((1, 0), later);
    let meta = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    let sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &sprites,
    };
    let mut events = RenderChunkEvents {
        added: vec![(0, 0), (1, 0)],
        removed: vec![],
        blood: vec![],
    };
    let guard = square.borrow_mut();
    assert!(matches!(
        renderer.sync_chunks(&view, &mut events, &fs, &mut textures),
        Err(RuntimeHostError::Borrowed(_))
    ));
    assert_eq!(events.added, vec![(0, 0), (1, 0)]);
    assert!(!renderer.world.chunks.contains_key(&(1, 0)));
    drop(guard);
    renderer
        .sync_chunks(&view, &mut events, &fs, &mut textures)
        .unwrap();
    assert!(events.added.is_empty());
    assert!(renderer.world.chunks.contains_key(&(1, 0)));
}

#[test]
fn view3d_frame_makes_no_corpse_atlas_entry() {
    use pz_world::save::visual::IsoDeadBody;
    use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoSpriteManager};
    use pz_world_render::{ClimateInput, PlayerRenderInput};
    use std::cell::RefCell;
    use std::rc::Rc;
    // No game files: preparing the corpse's atlas entry fails to load the
    // character media and reports it in `corpse_error`.
    let path = std::path::Path::new("unused-render-host-fixture");
    let fs = Rc::new(RefCell::new(GameFs::new(path)));
    let textures = Rc::new(RefCell::new(TextureManager::new()));
    let mut renderer = WorldRender::new(path, &mut fs.borrow_mut(), &mut textures.borrow_mut());
    let mut models = WorldModels::new(path, fs, textures);
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(1, 2, 0, Some(IsoGridSquare::new(1, 2, 0).into()));
    chunk
        .get_grid_square(1, 2, 0)
        .unwrap()
        .borrow_mut()
        .static_moving
        .push(Rc::new(RefCell::new(IsoDeadBody::for_tests(1.5, 2.5, 0.0))));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    let meta = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    let sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &sprites,
    };
    let mut v = RenderView {
        player: 0,
        x: 1.5,
        y: 1.5,
        z: 0.0,
        dir: IsoDirections::S,
        chunk_map: (0, 0, 1),
        width: 64,
        height: 64,
        split_view: None,
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
                fog_intensity: 0.0,
                moon_float: 0.0,
                game_night: 0.0,
                night_darkness: 3,
                weather_shader: false,
                last_stand: false,
            },
            players: [PlayerRenderInput::default(); MAX_PLAYERS],
        },
        screen: ScreenParams::default(),
        tick: 0,
        multiplier: 1.0,
    };
    let mut out = Vec::new();
    // No regions and no weather fx (#268's inputs): neither reaches the corpses.
    let regions = pz_world::isoregion::IsoRegions::new(0);

    renderer.view3d.mode = pz_world3d::ViewMode::FirstPerson;
    renderer.render_player(&view, &mut v, Some(&mut models), &[], &regions, None, None, &mut out);
    assert_eq!(renderer.corpse_error, None);

    // The isometric frame does reach the corpse.
    renderer.view3d.mode = pz_world3d::ViewMode::Iso;
    renderer.render_player(&view, &mut v, Some(&mut models), &[], &regions, None, None, &mut out);
    assert!(renderer.corpse_error.is_some());
}

#[test]
fn split_screen_player_view_emits_shared_offscreen_and_quarter_viewport() {
    use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoSpriteManager};
    use pz_world_render::{ClimateInput, PlayerRenderInput};
    let path = std::path::Path::new("unused-splitscreen-fixture");
    let mut fs = GameFs::new(path);
    let mut textures = TextureManager::new();
    let mut renderer = WorldRender::new(path, &mut fs, &mut textures);
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(1, 1, 0, Some(IsoGridSquare::new(1, 1, 0).into()));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    let meta = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    let sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &sprites,
    };
    let pv = PlayerView {
        player: 1,
        num_players: 2,
        screen: (1280, 720),
    };
    let [left, top, w, h] = pv.screen_rect();
    assert_eq!([left, top, w, h], [640, 0, 640, 720]);
    let mut v = RenderView {
        player: 1,
        x: 1.5,
        y: 1.5,
        z: 0.0,
        dir: IsoDirections::S,
        chunk_map: (0, 0, 1),
        width: w,
        height: h,
        split_view: Some(pv),
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
                fog_intensity: 0.0,
                moon_float: 0.0,
                game_night: 0.0,
                night_darkness: 3,
                weather_shader: false,
                last_stand: false,
            },
            players: [PlayerRenderInput::default(); MAX_PLAYERS],
        },
        screen: ScreenParams {
            offscreen_left: left as f32,
            offscreen_top: top as f32,
            offscreen_width: w as f32,
            offscreen_height: h as f32,
            zoom: 1.0,
            tile_width: 64.0,
            ..Default::default()
        },
        tick: 0,
        multiplier: 1.0,
    };
    let regions = pz_world::isoregion::IsoRegions::new(0);
    let mut out = Vec::new();
    renderer.render_player(&view, &mut v, None, &[], &regions, None, None, &mut out);
    let (ow, oh) = FrameSize { width: 1280, height: 720 }.offscreen();
    let [vx, vy, vw, vh] = pv.viewport();
    assert!(
        out.iter().any(|c| matches!(c, DrawCmd::OffscreenBegin { width, height } if (*width, *height) == (ow, oh))),
        "expected full-screen shared offscreen buffer ({ow}x{oh}) for split-screen player"
    );
    assert!(
        out.iter().any(|c| matches!(c, DrawCmd::Viewport { x, y, width, height } if [*x, *y, *width, *height] == [vx, vy, vw, vh])),
        "expected player 1 viewport ({vx},{vy},{vw}x{vh}) inside shared offscreen buffer"
    );
}

#[test]
fn surface_passes_read_current_lua_quality_settings() {
    use pz_lua_api::{bind_core, bind_performance};
    use std::rc::Rc;
    let previous = bind_core::shared_core();
    let core = Rc::new(RefCell::new(pz_core::core::Core::new()));
    bind_core::install_core(core.clone());
    let mut thread = pz_kahlua::Thread::new(pz_kahlua::TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    bind_performance::register(&mut thread, core.clone());
    let script = pz_kahlua::stdlib::load_string(
        "local p = getPerformance(); p:setWaterQuality(2); p:setPuddlesQuality(1)",
        Some("surface options"), thread.globals().clone(),
    ).unwrap();
    thread.pcall(&script, &[]).unwrap();
    core.borrow_mut().set_perf_puddles(3);
    core.borrow_mut().set_option_blood_decals(7);
    let (settings, _) = surface_values();
    bind_core::install_core(previous);
    assert_eq!((settings.water_quality, settings.puddles_quality, settings.perf_puddles), (2, 1, 3));
    assert_eq!(settings.blood_decals, 7);
    assert!(!settings.water_shader() && !settings.puddles_shader());
}

#[test]
fn collect_object_outlines_emits_sprite_overlay_attached_and_clears_after_render_last() {
    use pz_iso_render::frame::ChunkInput;
    use pz_iso_render::world::{
        AttachedSprite, ObjectKind as RenderKind, Overlay, RenderObject, RenderSquare, SpriteInfo,
    };
    use pz_render::Texture;
    use pz_world::chunk::{IsoChunk, IsoGridSquare};
    use pz_world::objects::{IsoObject, ObjectKind};
    use std::rc::Rc;

    let mut cell = pz_world::chunk::IsoCell::default();
    let mut chunk = IsoChunk::new(0, 0);
    let mut sq = IsoGridSquare::new(2, 3, 0);
    let mut obj = IsoObject::new(42, ObjectKind::Object, None);
    obj.outline.set(0, true, (0, 0));
    obj.outline.set_attached(0, true);
    obj.outline.set_col(0, 1.0, 0.5, 0.0, 1.0);
    sq.objects.push(pz_world::ObjectSlot::new(obj));
    chunk.set_square(2, 3, 0, Some(pz_world::SquareSlot::new(sq)));
    cell.chunks.insert((0, 0), chunk);

    let mut t_main = Texture::empty("main");
    t_main.width = 64;
    t_main.height = 128;
    let mut t_ov = Texture::empty("overlay");
    t_ov.width = 64;
    t_ov.height = 128;
    let mut t_att = Texture::empty("attached");
    t_att.width = 64;
    t_att.height = 128;

    let mut ro = RenderObject::new(
        RenderKind::Plain,
        Some(Rc::new(SpriteInfo {
            texture: Some(t_main),
            ..Default::default()
        })),
    );
    ro.id = 42;
    ro.overlay = Some(Overlay {
        sprite: Rc::new(SpriteInfo {
            texture: Some(t_ov),
            ..Default::default()
        }),
        color: None,
    });
    ro.attached.push(AttachedSprite {
        sprite: Rc::new(SpriteInfo {
            texture: Some(t_att),
            ..Default::default()
        }),
        alpha: 1.0,
    });
    let mut rsq = RenderSquare::new(2, 3, 0);
    rsq.objects.push(ro);
    let mut level: [Option<&RenderSquare>; 64] = [None; 64];
    level[0] = Some(&rsq);
    let inputs = [ChunkInput {
        wx: 0,
        wy: 0,
        min_level: 0,
        max_level: 0,
        levels: vec![(0, level)],
    }];
    let cam = pz_iso_render::Camera::player(2.5, 3.5, 0.0, 320, 240, 1.0);
    let mut out = Vec::new();
    super::collect_object_outlines(&inputs, &cell, 0, &cam, &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].textures.len(), 3);
    // Non-UI highlight is cleared by render_last after drawing.
    let cell_sq = cell.get_grid_square(2, 3, 0).unwrap();
    assert!(!cell_sq.borrow().objects[0].borrow().outline.is_on(0, true));
}
