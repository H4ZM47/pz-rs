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
    renderer.render_player(&view, &mut v, Some(&mut models), &[], &regions, None, &mut out);
    assert_eq!(renderer.corpse_error, None);

    // The isometric frame does reach the corpse.
    renderer.view3d.mode = pz_world3d::ViewMode::Iso;
    renderer.render_player(&view, &mut v, Some(&mut models), &[], &regions, None, &mut out);
    assert!(renderer.corpse_error.is_some());
}

#[test]
fn surface_passes_read_current_lua_quality_settings() {
    use pz_lua_api::{bind_core, bind_performance};
    let previous = bind_core::shared_core();
    let core = Rc::new(RefCell::new(pz_core::core::Core::new()));
    bind_core::install_core(core.clone());
    let mut thread = pz_kahlua::Thread::new(pz_kahlua::TableRef::new());
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
