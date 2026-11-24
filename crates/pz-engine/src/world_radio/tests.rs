use super::*;
use pz_kahlua::exec::java_value_of;
use pz_world::chunk::IsoCell;
use pz_world::objects::ObjectKind;
use pz_world::{IsoChunk, IsoGridSquare};
use std::sync::atomic::{AtomicUsize, Ordering};

fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let d = std::env::temp_dir().join(format!("pz-engine-radio-{tag}-{}-{n}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn eval_lua(lua: &mut LuaManager, src: &str) -> Value {
    let f = pz_kahlua::stdlib::load_string(src, Some("world_radio_test"), lua.thread.globals().clone())
        .unwrap();
    lua.thread.call1(&f, &[]).unwrap()
}

fn eval_str(lua: &mut LuaManager, src: &str) -> String {
    java_value_of(&eval_lua(lua, src))
}

fn fresh_lua(game_dir: &Path, cache_dir: &Path) -> LuaManager {
    let mut fs = pz_fs::GameFs::new(game_dir);
    fs.set_cache_dir(cache_dir);
    let mut lua = LuaManager::init(Rc::new(RefCell::new(fs))).unwrap();
    pz_lua_api::register_all(&mut lua);
    lua
}

fn minimal_radio_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="utf-8"?>
<RadioData>
  <RootInfo>
    <SourceFile>RadioData.xml</SourceFile>
    <FileGUID>test-guid-001</FileGUID>
    <Version>1</Version>
  </RootInfo>
  <Adverts />
  <Channels>
    <ChannelEntry ID="ch-fm-1" name="Knox FM" cat="Radio" freq="91200" startscript="main">
      <ScriptEntry ID="sc-1" name="main" timestamp="0" loopmin="1" loopmax="1">
        <BroadcastEntry ID="bc-1" timestamp="0" endstamp="1440" isSegment="false" advertCat="none">
          <LineEntry ID="ln-1" r="255" g="255" b="255" codes="BOR-5">Hello Knox Country!</LineEntry>
        </BroadcastEntry>
      </ScriptEntry>
    </ChannelEntry>
    <ChannelEntry ID="ch-tv-1" name="Life and Living TV" cat="Television" freq="203" startscript="main">
      <ScriptEntry ID="sc-2" name="main" timestamp="0" loopmin="1" loopmax="1">
        <BroadcastEntry ID="bc-2" timestamp="0" endstamp="1440" isSegment="false" advertCat="none">
          <LineEntry ID="ln-2" r="255" g="255" b="255" codes="CRP+1">Woodcraft time!</LineEntry>
        </BroadcastEntry>
      </ScriptEntry>
    </ChannelEntry>
  </Channels>
</RadioData>"#
}

#[test]
fn init_loads_xml_fires_lua_events_and_feeds_world_dictionary() {
    reset_louisville_obfuscation_for_test();
    let root = temp_dir("init");
    let game_dir = root.join("game");
    let cache_dir = root.join("cache");
    let save_dir = root.join("save");
    std::fs::create_dir_all(game_dir.join("media/radio")).unwrap();
    std::fs::write(game_dir.join("media/radio/RadioData.xml"), minimal_radio_xml()).unwrap();
    std::fs::create_dir_all(&save_dir).unwrap();

    let mut lua = fresh_lua(&game_dir, &cache_dir);
    // Register Lua handlers mirroring ISRecordedMedia.lua and ISDynamicRadio.lua.
    eval_lua(
        &mut lua,
        r#"
        _G.test_media_called = false
        _G.test_scripts_called = false
        _G.test_scripts_new_game = nil
        Events.OnInitRecordedMedia.Add(function(rm)
            _G.test_media_called = true
            local d = rm:register("Retail-VHS", "vhs-woodcraft", "Woodcraft Ep 1", 0)
            d:setTitle("Woodcraft")
            d:addLine("RM_wood_1", 1.0, 1.0, 1.0, "CRP+1")
        end)
        Events.OnLoadRadioScripts.Add(function(sm, isNew)
            _G.test_scripts_called = true
            _G.test_scripts_new_game = isNew
            local ch = DynamicRadioChannel.new("AEBS", 93200, ChannelCategory.Emergency, "EMRG-TEST")
            ch:setAirCounterMultiplier(1.0)
            sm:AddChannel(ch, false)
        end)
        "#,
    );

    let sb = Rc::new(RefCell::new(pz_sandbox::SandboxOptions::default()));
    let render = Rc::new(RefCell::new(None));
    let wr = WorldRadio::init(
        &game_dir,
        save_dir.clone(),
        -1,
        false,
        &mut lua,
        &sb,
        None,
        render.clone(),
    )
    .unwrap();

    assert_eq!(eval_str(&mut lua, "return _G.test_media_called"), "true");
    assert_eq!(eval_str(&mut lua, "return _G.test_scripts_called"), "true");
    assert_eq!(eval_str(&mut lua, "return _G.test_scripts_new_game"), "true");
    assert_eq!(eval_str(&mut lua, "return getRadioAPI() ~= nil"), "true");
    assert_eq!(
        eval_str(&mut lua, "return getZomboidRadio():getChannelName(91200)"),
        "Knox FM"
    );
    assert_eq!(
        eval_str(&mut lua, "return getZomboidRadio():getChannelName(93200)"),
        "AEBS"
    );

    // Verify dictionary inputs get live recorded media + channel lists.
    let media_names = wr.recorded_media_names();
    assert_eq!(media_names.get(&0).map(String::as_str), Some("Woodcraft Ep 1"));
    let channels = wr.channel_lists();
    assert!(channels
        .channel_list("Radio")
        .unwrap()
        .iter()
        .any(|(f, n)| *f == 91200 && n == "Knox FM"));
    assert!(channels
        .channel_list("Emergency")
        .unwrap()
        .iter()
        .any(|(f, n)| *f == 93200 && n == "AEBS"));

    // Verify save writes RADIO_SAVE.txt + recorded_media.bin and reset keeps louisville_obfuscation.
    wr.radio.borrow_mut().louisville_obfuscation = true;
    wr.save();
    assert!(save_dir.join("radio/data/RADIO_SAVE.txt").is_file());
    assert!(save_dir.join("recorded_media.bin").is_file());

    wr.reset(&render);
    assert_eq!(eval_str(&mut lua, "return ZomboidRadio.hasInstance()"), "false");

    // Reloading into a new WorldRadio carries over static louisville_obfuscation.
    let wr2 = WorldRadio::init(
        &game_dir,
        save_dir.clone(),
        249,
        false,
        &mut lua,
        &sb,
        None,
        render.clone(),
    )
    .unwrap();
    assert!(wr2.radio.borrow().louisville_obfuscation);
    assert_eq!(eval_str(&mut lua, "return _G.test_scripts_new_game"), "false");

    wr2.reset(&render);
    reset_louisville_obfuscation_for_test();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tv_overlay_and_broadcast_delivery_fire_on_device_text_outside_borrows() {
    reset_louisville_obfuscation_for_test();
    let root = temp_dir("tv");
    let game_dir = root.join("game");
    let cache_dir = root.join("cache");
    let save_dir = root.join("save");
    std::fs::create_dir_all(game_dir.join("media/radio")).unwrap();
    std::fs::write(game_dir.join("media/radio/RadioData.xml"), minimal_radio_xml()).unwrap();
    std::fs::create_dir_all(&save_dir).unwrap();

    let mut lua = fresh_lua(&game_dir, &cache_dir);
    eval_lua(
        &mut lua,
        r#"
        _G.device_texts = {}
        Events.OnDeviceText.Add(function(guid, codes, x, y, z, line, dev)
            -- Reenter getZomboidRadio() inside the Lua handler to prove no borrow is held.
            local chName = getZomboidRadio():getChannelName(203)
            table.insert(_G.device_texts, { codes = codes, x = x, y = y, z = z, line = line, ch = chName })
        end)
        "#,
    );

    // Build a sprite manager with a TV sprite and screen sprites at offsets +16 and +32.
    let mut sm_sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
    let base_tv = sm_sprites.add_sprite("appliances_television_01_0", 1);
    sm_sprites.sprites[base_tv].name = Some("appliances_television_01_0".into());
    let test_screen = sm_sprites.add_sprite("appliances_television_01_16", 17);
    sm_sprites.sprites[test_screen].name = Some("appliances_television_01_16".into());
    let alt_screen = sm_sprites.add_sprite("appliances_television_01_32", 33);
    sm_sprites.sprites[alt_screen].name = Some("appliances_television_01_32".into());
    let sprites = Rc::new(RefCell::new(sm_sprites));

    let sb = Rc::new(RefCell::new(pz_sandbox::SandboxOptions::default()));
    let render = Rc::new(RefCell::new(None));
    let wr = WorldRadio::init(
        &game_dir,
        save_dir.clone(),
        -1,
        true,
        &mut lua,
        &sb,
        Some(sprites.clone()),
        render.clone(),
    )
    .unwrap();

    // Put a live IsoCell with a square at (10, 20, 0) and a nearby local player
    // so world sounds, player-in-range and listener distance checks succeed.
    let mut cell = IsoCell::default();
    let mut chunk = IsoChunk::new(10i32.div_euclid(8), 20i32.div_euclid(8));
    chunk.set_square(2, 4, 0, Some(IsoGridSquare::new(10, 20, 0).into()));
    cell.chunks.insert((10i32.div_euclid(8), 20i32.div_euclid(8)), chunk);
    pz_lua_api::bind_world::set_cell(Some(cell));

    let sm = pz_lua_api::bind_registry::shared_script_manager();
    let player = Rc::new(RefCell::new(
        pz_lua_api::bind_character_sim::LuaGameCharacter::new(
            &sm,
            pz_characters::CharacterKind::Player,
        ),
    ));
    {
        let mut p = player.borrow_mut();
        p.vars.x = 10.5;
        p.vars.y = 20.5;
        p.vars.z = 0.0;
    }
    pz_lua_api::bind_players::set_player(0, Some(player.clone()));
    pz_lua_api::bind_players::set_player_instance(Some(player.clone()));

    let mut tv_data = DeviceData {
        is_television: true,
        is_turned_on: true,
        is_battery_powered: true,
        has_battery: true,
        power_delta: 1.0,
        channel: 203,
        device_volume: 1.0,
        ..Default::default()
    };
    tv_data.runtime.wave.has_player_in_range = true;

    let tv_obj: IsoObjectRef = Rc::new(RefCell::new(IsoObject::new(
        1,
        ObjectKind::Television(tv_data),
        Some(base_tv),
    )));

    wr.register_world_object(&tv_obj, (10, 20, 0));

    // Tick the TV device once -> resolves screen sprites (+16, +32) and sets Test screen overlay.
    wr.update_devices(&mut lua, Some(&sprites), &render);
    assert_eq!(
        tv_obj.borrow().overlay_sprite,
        Some(OverlaySprite {
            name: SpriteName::Str("appliances_television_01_16".into()),
            color: None,
        })
    );

    // Activate the TV channel's broadcast window, mark a player listening, and
    // tick ZomboidRadio::update so it airs to the TV.
    {
        let mut r = wr.radio.borrow_mut();
        let mut w = EngineRadioWorld::new(wr.state.clone(), wr.emitters.clone(), Some(sprites.clone()), None);
        let mut rng = |b: i32| pz_core::random::next(b);
        r.update_scripts(9, 0, &mut w, &mut rng);
        r.player_listens_channel(203, true, true, &mut w);
    }
    wr.update_radio(&mut lua, Some(&sprites), &render);

    assert_eq!(eval_str(&mut lua, "return #_G.device_texts"), "1.0");
    assert_eq!(
        eval_str(&mut lua, "return _G.device_texts[1].line"),
        "RD_ln-2"
    );
    assert_eq!(
        eval_str(&mut lua, "return _G.device_texts[1].codes"),
        "CRP+1"
    );
    assert_eq!(
        eval_str(&mut lua, "return _G.device_texts[1].ch"),
        "Life and Living TV"
    );

    // Now the TV is receiving a signal, so the next device tick switches its overlay to +32.
    wr.update_devices(&mut lua, Some(&sprites), &render);
    assert_eq!(
        tv_obj.borrow().overlay_sprite,
        Some(OverlaySprite {
            name: SpriteName::Str("appliances_television_01_32".into()),
            color: None,
        })
    );

    wr.unregister_chunk(10i32.div_euclid(8), 20i32.div_euclid(8), &render);
    assert!(wr.radio.borrow().devices().is_empty());

    wr.reset(&render);
    pz_lua_api::bind_players::set_player(0, None);
    pz_lua_api::bind_players::set_player_instance(None);
    pz_lua_api::bind_world::set_cell(None);
    reset_louisville_obfuscation_for_test();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn object_load_hooks_caches_device_data_and_exposes_channel_lists() {
    reset_louisville_obfuscation_for_test();
    let root = temp_dir("hooks");
    let game_dir = root.join("game");
    let cache_dir = root.join("cache");
    let save_dir = root.join("save");
    std::fs::create_dir_all(game_dir.join("media/radio")).unwrap();
    std::fs::write(game_dir.join("media/radio/RadioData.xml"), minimal_radio_xml()).unwrap();
    std::fs::create_dir_all(&save_dir).unwrap();

    let mut lua = fresh_lua(&game_dir, &cache_dir);
    let sm = pz_scripts::ScriptManager::new();
    sm.registries
        .get("item_type")
        .unwrap()
        .borrow_mut()
        .register_id("base:radio", true)
        .unwrap();
    sm.statics
        .items
        .borrow_mut()
        .constants
        .insert("item_type", "RADIO", "base:radio");
    sm.parse_script(
        pz_scripts::script_type::ScriptLoadMode::Init,
        r#"
        module Base {
            item TestRadio {
                ItemType = base:radio,
                DisplayName = Test Radio,
                TwoWay = true,
                TransmitRange = 2000,
                MicRange = 5,
                BaseVolumeRange = 12,
                IsPortable = true,
                IsTelevision = false,
                MinChannel = 88000,
                MaxChannel = 108000,
                UsesBattery = true,
                UseDelta = 0.005,
                AcceptMediaType = 0,
            }
        }
        "#,
    )
    .unwrap();
    sm.get_module("Base")
        .unwrap()
        .bucket(pz_scripts::script_type::ScriptType::Item)
        .load_scripts(&sm, pz_scripts::script_type::ScriptLoadMode::Init);
    pz_lua_api::bind_registry::install_script_manager(Rc::new(sm));

    let sb = Rc::new(RefCell::new(pz_sandbox::SandboxOptions::default()));
    let render = Rc::new(RefCell::new(None));
    let wr = WorldRadio::init(
        &game_dir,
        save_dir.clone(),
        -1,
        true,
        &mut lua,
        &sb,
        None,
        render.clone(),
    )
    .unwrap();

    let mut hooks = RadioObjectLoadHooks::new(Some(&wr));
    assert!(hooks.channel_list("Radio").is_some());
    let mut rand = crate::iso_world::StandardRand;
    let first = hooks.device_data_from_item("Base.TestRadio", &mut rand).unwrap();
    assert_eq!(first.device_name, "Test Radio");
    assert!(first.two_way);
    assert_eq!(first.transmit_range, 2000);
    // Second call hits the cache and produces an independent presets deep copy.
    let second = hooks.device_data_from_item("Base.TestRadio", &mut rand).unwrap();
    assert_eq!(second.device_name, "Test Radio");
    assert!(!Rc::ptr_eq(first.presets.owner(), second.presets.owner()));

    wr.reset(&render);
    reset_louisville_obfuscation_for_test();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn dropped_world_device_releases_emitter_and_reset_drops_world_hooks() {
    reset_louisville_obfuscation_for_test();
    let root = temp_dir("emitter_cleanup");
    let game_dir = root.join("game");
    let cache_dir = root.join("cache");
    let save_dir = root.join("save");
    std::fs::create_dir_all(game_dir.join("media/radio")).unwrap();
    std::fs::write(game_dir.join("media/radio/RadioData.xml"), minimal_radio_xml()).unwrap();
    std::fs::create_dir_all(&save_dir).unwrap();

    let mut lua = fresh_lua(&game_dir, &cache_dir);
    let sb = Rc::new(RefCell::new(pz_sandbox::SandboxOptions::default()));
    let sprites = Rc::new(RefCell::new(IsoSpriteManager::new(
        pz_world::sprite::test_tables(),
    )));
    let render = Rc::new(RefCell::new(None));
    let weak_render = Rc::downgrade(&render);
    let weak_sprites = Rc::downgrade(&sprites);

    let wr = WorldRadio::init(
        &game_dir,
        save_dir.clone(),
        -1,
        true,
        &mut lua,
        &sb,
        Some(sprites.clone()),
        render.clone(),
    )
    .unwrap();

    // Register a TV and allocate an emitter owned by it, then drop the last
    // `Rc<IsoObject>` BEFORE `unregister_chunk` runs (simulating chunk unload
    // or stale object teardown).
    let tv_obj = Rc::new(RefCell::new(IsoObject::new(
        1,
        ObjectKind::Television(DeviceData {
            is_television: true,
            ..Default::default()
        }),
        None,
    )));
    wr.register_world_object(&tv_obj, (10, 20, 0));
    let dev_id = *wr.state.borrow().world_devices.keys().next().unwrap();
    {
        let mut audio = EngineRadioAudio::new(wr.emitters.clone(), Some(dev_id));
        let eid = audio.get_free_emitter(10.0, 20.0, 0);
        assert!(wr.emitters.borrow().emitters.contains_key(&eid));
    }
    drop(tv_obj);

    wr.unregister_chunk(10i32.div_euclid(8), 20i32.div_euclid(8), &render);
    assert!(
        wr.emitters.borrow().emitters.is_empty(),
        "emitter owned by dropped world device must be returned on chunk unload"
    );

    // Also verify stale-object cleanup in `update_devices` returns emitters.
    let radio_obj = Rc::new(RefCell::new(IsoObject::new(
        2,
        ObjectKind::Radio(DeviceData::default()),
        None,
    )));
    wr.register_world_object(&radio_obj, (12, 22, 0));
    let dev_id2 = *wr.state.borrow().world_devices.keys().next().unwrap();
    {
        let mut audio = EngineRadioAudio::new(wr.emitters.clone(), Some(dev_id2));
        let eid = audio.get_free_emitter(12.0, 22.0, 0);
        assert!(wr.emitters.borrow().emitters.contains_key(&eid));
    }
    drop(radio_obj);
    wr.update_devices(&mut lua, Some(&sprites), &render);
    assert!(
        wr.emitters.borrow().emitters.is_empty(),
        "emitter owned by dropped world device must be returned during stale update cleanup"
    );

    // Queueing world-device chat renders into ChatElement::render_batch(0)
    // when a local player is within hearing range of the TV.
    let sm_chat = pz_lua_api::bind_registry::shared_script_manager();
    let listener = Rc::new(RefCell::new(
        pz_lua_api::bind_character_sim::LuaGameCharacter::new(
            &sm_chat,
            pz_characters::CharacterKind::Player,
        ),
    ));
    {
        let mut p = listener.borrow_mut();
        p.vars.x = 10.5;
        p.vars.y = 20.5;
        p.vars.z = 0.0;
    }
    pz_lua_api::bind_players::set_player(0, Some(listener.clone()));
    pz_lua_api::bind_players::set_player_instance(Some(listener));

    let chat_tv = Rc::new(RefCell::new(IsoObject::new(
        3,
        ObjectKind::Television(DeviceData {
            is_television: true,
            ..Default::default()
        }),
        None,
    )));
    wr.register_world_object(&chat_tv, (10, 20, 0));
    {
        let st = wr.state.borrow();
        let entry = st.world_devices.values().next().unwrap();
        entry.chat.borrow_mut().add_chat_line_full(
            "Hello Knox Country!",
            1.0,
            1.0,
            1.0,
            UIFont::Medium,
            20.0,
            "default",
            true,
            true,
            true,
            true,
            true,
            true,
        );
        assert!(entry.chat.borrow().get_has_chat_to_display());
    }
    wr.queue_device_chat(0);
    pz_lua_api::chat::element::ChatElement::render_batch(0);
    pz_lua_api::bind_players::set_player(0, None);
    pz_lua_api::bind_players::set_player_instance(None);

    // Reset must reinstall NoHooks/KeyText so the old render/sprites are not pinned.
    wr.reset(&render);
    drop(wr);
    drop(render);
    drop(sprites);
    assert!(
        weak_render.upgrade().is_none(),
        "reset must release EngineRadioLuaHooks's WorldRender reference"
    );
    assert!(
        weak_sprites.upgrade().is_none(),
        "reset must release EngineRadioLuaHooks's IsoSpriteManager reference"
    );

    // `query_square_power` must include `TimeSinceApo` apocalypse months.
    let mut cell = IsoCell::default();
    let mut chunk = IsoChunk::new(0, 0);
    let sq = IsoGridSquare::new(2, 2, 0);
    chunk.set_square(2, 2, 0, Some(sq.into()));
    cell.chunks.insert((0, 0), chunk);
    let meta = pz_world::IsoMetaGrid::blank(0, 0, 1, 1);
    pz_lua_api::bind_world::set_cell(Some(cell));
    pz_lua_api::bind_world::set_meta_grid(Some(meta));
    let sp = Rc::new(RefCell::new(IsoSpriteManager::new(
        pz_world::sprite::test_tables(),
    )));
    {
        let gt = pz_lua_api::bind_climate::shared_game_time();
        let mut g = gt.borrow_mut();
        g.nights_survived = 0;
        g.time_of_day = 9.0;
    }
    let cfg_day0 = RadioPowerConfig {
        time_since_apo: 1,
        elec_shut_modifier: 14,
        ..Default::default()
    };
    let (MainsPower::Square(Some(p_day0)), _) = query_square_power((2, 2, 0), Some(&sp), cfg_day0) else {
        panic!("expected square power");
    };
    assert!(p_day0.grid_power, "day 0 with elec_shut_modifier=14 should have grid power");

    let cfg_six_months = RadioPowerConfig {
        time_since_apo: 7,
        elec_shut_modifier: 14,
        ..Default::default()
    };
    let (MainsPower::Square(Some(p_late)), _) =
        query_square_power((2, 2, 0), Some(&sp), cfg_six_months)
    else {
        panic!("expected square power");
    };
    assert!(
        !p_late.grid_power,
        "six months later (TimeSinceApo=7) with elec_shut_modifier=14 must have no grid power"
    );

    pz_lua_api::bind_world::set_cell(None);
    pz_lua_api::bind_world::set_meta_grid(None);
    reset_louisville_obfuscation_for_test();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
#[ignore = "reads installed Build 42 game files via PZ_GAME_DIR"]
fn installed_build42_radio_init_loads_vanilla_channels_and_media() {
    let game_dir = PathBuf::from(
        std::env::var("PZ_GAME_DIR")
            .expect("set PZ_GAME_DIR to the installed Project Zomboid Java/media root"),
    );
    reset_louisville_obfuscation_for_test();
    let root = temp_dir("installed");
    let cache_dir = root.join("cache");
    let save_dir = root.join("save");
    std::fs::create_dir_all(&save_dir).unwrap();

    let mut lua = fresh_lua(&game_dir, &cache_dir);
    // Boot translator so RD_* and RM_* keys resolve against the real install.
    let core = pz_lua_api::bind_core::shared_core();
    pz_lua_api::bind_text::boot(&lua.fs.borrow(), &core.borrow()).unwrap();

    // Load the real shipped RecordedMedia + DynamicRadio + WeatherChannel scripts.
    for rel in [
        "media/lua/shared/RecordedMedia/ISRecordedMedia.lua",
        "media/lua/shared/RecordedMedia/recorded_media.lua",
        "media/lua/server/radio/ISDynamicRadio.lua",
        "media/lua/server/radio/ISWeatherChannel.lua",
    ] {
        let path = game_dir.join(rel);
        if path.is_file() {
            let src = std::fs::read_to_string(&path).unwrap();
            let f = pz_kahlua::stdlib::load_string(&src, Some(rel), lua.thread.globals().clone())
                .unwrap();
            lua.thread.call1(&f, &[]).unwrap();
        }
    }

    let sb = Rc::new(RefCell::new(pz_sandbox::SandboxOptions::default()));
    let render = Rc::new(RefCell::new(None));
    let wr = WorldRadio::init(
        &game_dir,
        save_dir.clone(),
        -1,
        false,
        &mut lua,
        &sb,
        None,
        render.clone(),
    )
    .unwrap();

    let (chan_count, media_count, has_aebs) = {
        let r = wr.radio.borrow();
        let chans = r.script_manager().map_or(0, |m| m.channels().len());
        let media = r.recorded_media().indexes().len();
        let aebs = r
            .script_manager()
            .and_then(|m| m.radio_channel("EMRG-711984"))
            .is_some();
        (chans, media, aebs)
    };
    println!("Installed Build 42 ZomboidRadio.init: {chan_count} channels, {media_count} recorded media, AEBS={has_aebs}");
    assert!(chan_count >= 5, "expected vanilla radio/TV channels, got {chan_count}");
    assert!(media_count > 100, "expected vanilla CDs/VHS tapes, got {media_count}");
    assert!(has_aebs, "expected ISDynamicRadio to register AEBS channel EMRG-711984");
    assert!(save_dir.join("recorded_media.bin").is_file());

    wr.save();
    assert!(save_dir.join("radio/data/RADIO_SAVE.txt").is_file());

    wr.reset(&render);
    reset_louisville_obfuscation_for_test();
    let _ = std::fs::remove_dir_all(&root);
}
