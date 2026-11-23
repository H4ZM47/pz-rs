use super::*;
use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoObject, ObjectKind, ObjectSlot};
use std::sync::atomic::{AtomicUsize, Ordering};

fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let d = std::env::temp_dir().join(format!(
        "pz-engine-vehicles-{tag}-{}-{n}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fresh_lua(game_dir: &Path, cache_dir: &Path) -> LuaManager {
    let mut fs = GameFs::new(game_dir);
    fs.set_cache_dir(cache_dir);
    let mut lua = LuaManager::init(Rc::new(RefCell::new(fs))).unwrap();
    pz_lua_api::register_all(&mut lua);
    lua
}

fn minimal_vehicle_script_manager() -> Rc<ScriptManager> {
    let sm = ScriptManager::new();
    sm.parse_script(
        pz_scripts::script_type::ScriptLoadMode::Init,
        r#"
        module Base {
            vehicle TestSedan {
                mechanicType = 1,
                engineRepairLevel = 4,
                playerDamageProtection = 0.8,
                model {
                    file = Vehicles_CarNormal,
                    scale = 1.8200,
                    offset = 0.0000 0.2500 0.0000,
                }
                extents = 0.8600 0.6500 2.0000,
                mass = 800,
                physicsChassisShape = 0.8600 0.6500 2.0000,
                centerOfMassOffset = 0.0000 0.3000 0.0000,
                shadowExtents = 1.7500 4.2000,
                shadowOffset = 0.0000 0.0000,
                engineForce = 3600,
                engineQuality = 80,
                engineLoudness = 60,
                maxSpeed = 90f,
                brakingForce = 60,
                stoppingMovementForce = 2.0f,
                rollInfluence = 1.0f,
                steeringIncrement = 0.04,
                steeringClamp = 0.3,
                suspensionStiffness = 45,
                suspensionCompression = 3.83,
                suspensionDamping = 2.88,
                maxSuspensionTravelCm = 12,
                suspensionRestLength = 0.20f,
                wheelFriction = 1.5f,
                frontEndHealth = 150,
                rearEndHealth = 150,
                seats = 4,
                wheel FrontLeft {
                    front = true,
                    offset = 0.3600 -0.2600 0.6800,
                    radius = 0.15f,
                    width = 0.2f,
                }
                wheel FrontRight {
                    front = true,
                    offset = -0.3600 -0.2600 0.6800,
                    radius = 0.15f,
                    width = 0.2f,
                }
                wheel RearLeft {
                    front = false,
                    offset = 0.3600 -0.2600 -0.6800,
                    radius = 0.15f,
                    width = 0.2f,
                }
                wheel RearRight {
                    front = false,
                    offset = -0.3600 -0.2600 -0.6800,
                    radius = 0.15f,
                    width = 0.2f,
                }
                part HeadlightLeft {
                    area = Engine,
                    lua {
                        create = Vehicles.Create.TestHeadlight,
                    }
                }
            }
        }
        "#,
    )
    .unwrap();
    sm.get_module("Base")
        .unwrap()
        .bucket(ScriptType::Vehicle)
        .load_scripts(&sm, pz_scripts::script_type::ScriptLoadMode::Init);
    Rc::new(sm)
}

#[test]
fn bullet_world_init_vehicle_script_upload_and_physics_step() {
    let root = temp_dir("bullet");
    let game_dir = root.join("game");
    let cache_dir = root.join("cache");
    std::fs::create_dir_all(&game_dir).unwrap();
    let mut lua = fresh_lua(&game_dir, &cache_dir);

    let sm = minimal_vehicle_script_manager();
    pz_lua_api::bind_registry::install_script_manager(sm.clone());

    let mut cell = IsoCell::default();
    let mut chunk = IsoChunk::new(10, 10);
    chunk.set_square(2, 2, 0, Some(IsoGridSquare::new(82, 82, 0).into()));
    cell.chunks.insert((10, 10), chunk);
    let mut meta = IsoMetaGrid::empty();
    meta.min_x = 0;
    meta.min_y = 0;
    meta.max_x = 40;
    meta.max_y = 40;
    pz_lua_api::bind_world::set_cell(Some(cell));
    pz_lua_api::bind_world::set_meta_grid(Some(meta));

    let sprites = Rc::new(RefCell::new(IsoSpriteManager::new(
        pz_world::sprite::test_tables(),
    )));
    let sb = Rc::new(RefCell::new(pz_sandbox::SandboxOptions::default()));
    let render = Rc::new(RefCell::new(None));

    let mut wv = WorldVehicles::init(
        &game_dir,
        &lua,
        &sb,
        None,
        Some(sprites.clone()),
        render,
        (0, 0, 40, 40),
        (4, 4, 13),
    );
    assert!(world_sim::created());
    assert!(pz_vehicles::physics::take_exceptions().is_empty());

    // Register a synthetic Vehicles.Create.TestHeadlight Lua callback so
    // `add_to_world` exercises the Lua part-create hook and creates the spot light.
    let f = pz_kahlua::stdlib::load_string(
        "Vehicles = { Create = { TestHeadlight = function(v, part) part:createSpotLight(0.5, 1.0, 15.0, 0.9, 0.5, 10) end } }",
        Some("test_vehicle_lua"),
        lua.thread.globals().clone(),
    )
    .unwrap();
    lua.thread.pcall(&f, &[]).unwrap();

    // Create a vehicle from our uploaded script and step Bullet simulation.
    let v = BaseVehicle::new();
    v.set_script_name("Base.TestSedan");
    v.set_position(82.5, 82.5, 0.0);
    {
        let mut st = v.st_mut();
        st.square = Some((82, 82, 0));
        st.current = Some((82, 82, 0));
        st.chunk = Some((10, 10));
    }
    pz_vehicles::lua::with_thread(&mut lua.thread, || {
        manager::add_to_world(&v);
    });
    assert_eq!(manager::cell_vehicles().len(), 1);
    assert!(v.script().is_some());
    assert_eq!(v.light_count(), 1);

    // Verify PathfindVehicles sees the live vehicle.
    use pz_pathfind::game_vehicle::VehicleSource;
    let mut pf_count = 0usize;
    PathfindVehicles.for_each_vehicle(&mut |_| pf_count += 1);
    assert_eq!(pf_count, 1);

    // Enable the headlight part and check cell_torches() output.
    let hl = v.part_by_id("HeadlightLeft").expect("HeadlightLeft part");
    hl.set_light_active(true);
    let torches = cell_torches();
    assert_eq!(torches.len(), 1);
    assert!(torches[0].id >= 4096);

    // Step one simulation frame through Bullet.
    let before_frame = world_sim::state().bullet_frame_no;
    let sim = FrameSim::capture();
    wv.update_frame(&mut lua, &sim, Some(&sprites), None, None, None);
    assert!(world_sim::state().bullet_frame_no > before_frame);
    assert!(v.x().is_finite() && v.y().is_finite() && v.z().is_finite());

    wv.reset();
    assert!(manager::cell_vehicles().is_empty());
    assert!(!world_sim::created());
    pz_lua_api::bind_world::set_cell(None);
    pz_lua_api::bind_world::set_meta_grid(None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn chunk_physics_marks_and_builds_to_bullet_on_handoff() {
    let root = temp_dir("chunk_phys");
    let game_dir = root.join("game");
    let cache_dir = root.join("cache");
    std::fs::create_dir_all(&game_dir).unwrap();
    let lua = fresh_lua(&game_dir, &cache_dir);

    let mut sm_sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
    let wall_idx = sm_sprites.add_sprite("walls_exterior_house_01_0", 1);
    sm_sprites.sprites[wall_idx].name = Some("walls_exterior_house_01_0".into());
    let (tables, alias) = (sm_sprites.tables.clone(), sm_sprites.alias.clone());
    sm_sprites.sprites[wall_idx]
        .properties
        .set(&tables, &alias, "WallN", "", true);
    let sprites = Rc::new(RefCell::new(sm_sprites));

    let mut sq = IsoGridSquare::new(16, 16, 0);
    sq.properties.set(&tables, &alias, "WallN", "", true);
    sq.objects
        .push(ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, Some(wall_idx))));
    let mut chunk = IsoChunk::new(2, 2);
    chunk.set_square(0, 0, 0, Some(sq.into()));
    let mut cell = IsoCell::default();
    cell.chunks.insert((2, 2), chunk);
    let mut meta = IsoMetaGrid::empty();
    meta.min_x = 0;
    meta.min_y = 0;
    meta.max_x = 10;
    meta.max_y = 10;
    pz_lua_api::bind_world::set_cell(Some(cell));
    pz_lua_api::bind_world::set_meta_grid(Some(meta));

    let sb = Rc::new(RefCell::new(pz_sandbox::SandboxOptions::default()));
    let render = Rc::new(RefCell::new(None));
    let mut wv = WorldVehicles::init(
        &game_dir,
        &lua,
        &sb,
        None,
        Some(sprites.clone()),
        render,
        (0, 0, 10, 10),
        (0, 0, 13),
    );

    wv.note_chunk_handed_off(2, 2);
    let mut bufs = Vec::new();
    let updated = env::world().update_physics_for_level_if_needed(2, 2, 0, &mut bufs);
    assert!(updated);
    assert!(!bufs.is_empty());
    // Dirty flag is cleared after building once.
    bufs.clear();
    assert!(!env::world().update_physics_for_level_if_needed(2, 2, 0, &mut bufs));

    wv.reset();
    pz_lua_api::bind_world::set_cell(None);
    pz_lua_api::bind_world::set_meta_grid(None);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn virtual_vehicle_meta_and_ids_lifecycle() {
    let root = temp_dir("vvm");
    let game_dir = root.join("game");
    let cache_dir = root.join("cache");
    std::fs::create_dir_all(&game_dir).unwrap();
    let lua = fresh_lua(&game_dir, &cache_dir);

    let sm = minimal_vehicle_script_manager();
    pz_lua_api::bind_registry::install_script_manager(sm);

    let sb = Rc::new(RefCell::new(pz_sandbox::SandboxOptions::default()));
    let render = Rc::new(RefCell::new(None));
    let mut wv = WorldVehicles::init(
        &game_dir,
        &lua,
        &sb,
        None,
        None,
        render,
        (0, 0, 10, 10),
        (0, 0, 13),
    );

    let v = BaseVehicle::new();
    v.set_script_name("Base.TestSedan");
    v.set_position(24.5, 32.5, 0.0);
    v.st_mut().square = Some((24, 32, 0));
    v.st_mut().current = Some((24, 32, 0));
    v.add_to_world();
    let sql_id = v.st().sql_id;
    assert!(sql_id >= 1);
    assert!(wv.ids().borrow().is_vehicle_loaded(sql_id));

    let mut meta_hook = WorldVehicles::meta_hook();
    use pz_world::save::vehicles_db::VehicleMetaHook;
    meta_hook.add_to_meta(cell_link::handle(&v));
    let count = virtual_vehicle::with_manager(|m| m.vehicles.len());
    assert_eq!(count, 1);

    wv.reset();
    assert_eq!(virtual_vehicle::with_manager(|m| m.vehicles.len()), 0);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "reads installed Build 42.22 game files via PZ_GAME_DIR"]
fn installed_build42_vehicle_scripts_define_in_bullet() {
    let game_dir = PathBuf::from(
        std::env::var("PZ_GAME_DIR")
            .expect("set PZ_GAME_DIR to the installed Project Zomboid Java/media root"),
    );
    let root = temp_dir("installed_veh");
    let cache_dir = root.join("cache");
    let lua = fresh_lua(&game_dir, &cache_dir);

    let sm = ScriptManager::new();
    let candidate_dirs = [
        game_dir.join("media/scripts/generated/vehicles"),
        game_dir.join("media/scripts/vehicles"),
    ];
    let scripts_dir = candidate_dirs
        .iter()
        .find(|d| d.is_dir())
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "expected vehicles scripts dir under {}",
                game_dir.join("media/scripts").display()
            )
        });
    let mut files: Vec<PathBuf> = Vec::new();
    fn collect_txt(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    collect_txt(&p, out);
                } else if p.extension().is_some_and(|x| x == "txt") {
                    out.push(p);
                }
            }
        }
    }
    collect_txt(&scripts_dir, &mut files);
    files.sort();
    for file in &files {
        if let Ok(text) = std::fs::read_to_string(file) {
            let _ = sm.parse_script(pz_scripts::script_type::ScriptLoadMode::Init, &text);
        }
    }
    if let Some(m) = sm.get_module("Base") {
        m.bucket(ScriptType::VehicleTemplate)
            .load_scripts(&sm, pz_scripts::script_type::ScriptLoadMode::Init);
        m.bucket(ScriptType::Vehicle)
            .load_scripts(&sm, pz_scripts::script_type::ScriptLoadMode::Init);
    }
    let sm = Rc::new(sm);
    pz_lua_api::bind_registry::install_script_manager(sm.clone());

    let sb = Rc::new(RefCell::new(pz_sandbox::SandboxOptions::default()));
    let render = Rc::new(RefCell::new(None));
    let mut wv = WorldVehicles::init(
        &game_dir,
        &lua,
        &sb,
        None,
        None,
        render,
        (0, 0, 50, 50),
        (10, 10, 13),
    );
    let defined = define_all_vehicle_scripts(&sm);
    let exceptions = pz_vehicles::physics::take_exceptions();
    println!(
        "Installed Build 42.22 vehicle scripts defined in PzBullet: {defined} (exceptions: {})",
        exceptions.len()
    );
    assert!(defined > 10, "expected real Build 42.22 vehicle scripts, got {defined}");
    assert!(
        exceptions.is_empty(),
        "unexpected Bullet exceptions defining installed vehicle scripts: {exceptions:?}"
    );
    wv.reset();
    let _ = std::fs::remove_dir_all(&root);
}

