use super::*;
use pz_kahlua::exec::java_value_of;

fn run(t: &mut Thread, src: &str) -> Result<Vec<Value>, String> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone())
        .map_err(|e| e.to_string())?;
    t.pcall(&f, &[]).map_err(|(m, _)| java_value_of(&m))
}

fn setup() -> Thread {
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    crate::bind_vehicles::register(&mut t);
    register(&mut t);
    t
}

fn strs(v: &[Value]) -> Vec<String> {
    v.iter().map(java_value_of).collect()
}

struct Obj(RefCell<DeviceData>);
impl WorldDevice for Obj {
    fn with_device(&self, f: &mut dyn FnMut(&mut DeviceData)) {
        f(&mut self.0.borrow_mut())
    }
}

fn device(t: &mut Thread, d: DeviceData) -> Rc<Obj> {
    let o = Rc::new(Obj(RefCell::new(d)));
    let w: Rc<dyn WorldDevice> = o.clone();
    t.globals().set_str(
        "dd",
        Value::Object(Rc::new(LuaDeviceData(DeviceOwner::World(w)))),
    );
    o
}

#[test]
fn radio_globals_follow_the_instance() {
    let mut t = setup();
    install_radio(None);
    let r = run(
        &mut t,
        "return getZomboidRadio() == nil, getRadioAPI() == nil",
    )
    .unwrap();
    assert_eq!(strs(&r), vec!["true", "true"]);
    let radio = Rc::new(RefCell::new(ZomboidRadio::new(9)));
    radio
        .borrow_mut()
        .add_channel_name("KnoxTalk", 93200, "Emergency", false);
    install_radio(Some(radio.clone()));
    radio_api_created();
    let r = run(
        &mut t,
        r#"
        local r = getZomboidRadio()
        r:setDisableBroadcasting(true)
        local ch = getRadioAPI():getChannels("Emergency")
        return r:getChannelName(93200), ch[93200], r:getDisableBroadcasting(),
            RadioAPI.timeToTimeStamp(1, 2, 3), RadioAPI.timeStampToHours(1500),
            ZomboidRadio.isStaticSound("<bzzt>")
        "#,
    )
    .unwrap();
    assert_eq!(
        strs(&r),
        vec!["KnoxTalk", "KnoxTalk", "true", "147.0", "1.0", "true"]
    );
    assert!(radio.borrow().disable_broadcasting);
    install_radio(None);
}

#[test]
fn recorded_media_registers_during_the_event() {
    let mut t = setup();
    install_radio(None);
    let mut media = RecordedMedia::default();
    t.globals().set_str("RM", MEDIA_OBJ.with(|v| v.clone()));
    let r = with_scoped_media(&mut media, || {
        run(
            &mut t,
            r#"
            local m = RM:register("CDs", "cd-1", "RM_cd1", 3)
            m:setTitle("RM_title")
            m:addLine("RM_l1", 0, 0, 0, nil)
            m:addLine("RM_l2", 0.5, 0.25, 1, "BOR+1")
            return RM:register("CDs", "cd-1", "x", 0) == nil
            "#,
        )
    })
    .unwrap();
    assert_eq!(strs(&r), vec!["true"]);
    let d = media.media_data("cd-1").unwrap();
    assert_eq!(d.line_count(), 2);
    assert_eq!(d.title.as_deref(), Some("RM_title"));
    // Outside the event (and without a radio) the media is gone.
    let r = run(&mut t, "return RM:getMediaData('cd-1') == nil").unwrap();
    assert_eq!(strs(&r), vec!["true"]);
    let radio = Rc::new(RefCell::new(ZomboidRadio::new(9)));
    *radio.borrow_mut().recorded_media_mut() = media;
    install_radio(Some(radio));
    let r = run(
        &mut t,
        r#"
        local rm = getZomboidRadio():getRecordedMedia()
        local m = rm:getMediaDataFromIndex(0)
        local l = m:getLine(1)
        return m:getId(), m:getTranslatedTitle(), m:getLineCount(), l:getCodes(),
            l:getTextGuid(), rm:getCategories():get(0), m == rm:getMediaData("cd-1"),
            rm:getAllMediaForCategory("CDs"):size(), m:getMediaType()
        "#,
    )
    .unwrap();
    assert_eq!(
        strs(&r),
        vec!["cd-1", "RM_title", "2.0", "BOR+1", "RM_l2", "CDs", "true", "1.0", "0.0"]
    );
    install_radio(None);
}

#[test]
fn device_data_presets_list_is_live() {
    let mut t = setup();
    let mut d = DeviceData::default();
    d.presets.add_preset("A", 88000);
    d.presets.add_preset("B", 90000);
    let o = device(&mut t, d);
    run(
        &mut t,
        r#"
        local presets = dd:getDevicePresets():getPresets()
        presets:add(PresetEntry.new("C", 92000))
        local p = presets:get(0)
        presets:remove(p)
        presets:get(0):setName("B2")
        "#,
    )
    .unwrap();
    let names: Vec<_> =
        o.0.borrow()
            .presets
            .list()
            .borrow()
            .iter()
            .map(|p| p.borrow().name.clone())
            .collect();
    assert_eq!(names, vec!["B2", "C"]);
    let r = run(
        &mut t,
        "local p = dd:getDevicePresets() return p:getMaxPresets(), p:getPresetFreq(1), p:getPresetName(5)",
    )
    .unwrap();
    assert_eq!(strs(&r), vec!["10.0", "92000.0", ""]);
}

#[test]
fn device_data_runs_the_simulation() {
    let mut t = setup();
    let o = device(
        &mut t,
        DeviceData {
            is_battery_powered: true,
            power_delta: 0.5,
            min_channel_range: 88000,
            max_channel_range: 108000,
            ..Default::default()
        },
    );
    let r = run(
        &mut t,
        r#"
        dd:setIsTurnedOn(true)
        dd:setChannel(91000)
        dd:setChannel(50)
        dd:setDeviceVolume(3)
        return dd:getIsTurnedOn(), dd:getChannel(), dd:getDeviceVolume(), dd:isIsoDevice(),
            dd:canBePoweredHere(), dd:isReceivingSignal(), dd:getLastRecordedDistance()
        "#,
    )
    .unwrap();
    assert_eq!(
        strs(&r),
        vec!["true", "91000.0", "1.0", "true", "true", "false", "-1.0"]
    );
    // Not battery powered and no square: it turns off.
    o.0.borrow_mut().is_battery_powered = false;
    run(&mut t, "dd:setIsTurnedOn(true)").unwrap();
    assert!(!o.0.borrow().is_turned_on);
}

#[test]
fn removed_and_duplicated_presets_keep_the_actual_entry() {
    let mut t = setup();
    let d = DeviceData::default();
    d.presets.add_preset("A", 88000);
    d.presets.add_preset("B", 90000);
    device(&mut t, d);
    let result = run(&mut t, r#"
        local p = dd:getDevicePresets()
        local list = p:getPresets()
        local a, b = list:get(0), list:get(1)
        p:removePreset(0)
        a:setName("removed A")
        local e = PresetEntry.new("C", 92000)
        list:add(e)
        list:add(e)
        e:setFrequency(93000)
        list:get(1):setName("shared C")
        return a:getName(), b:getName(), list:get(0) == b,
            list:get(1) == e, list:get(2) == e,
            list:get(2):getName(), list:get(1):getFrequency()
    "#).unwrap();
    assert_eq!(strs(&result), vec!["removed A", "B", "true", "true", "true", "shared C", "93000.0"]);
}

#[test]
fn preset_getters_return_the_same_owners_and_lua_tables_are_snapshots() {
    let mut t = setup();
    let d = DeviceData::default();
    d.presets.add_preset("A", 88000);
    device(&mut t, d);
    let result = run(&mut t, r#"
        local p = dd:getDevicePresets()
        local list = p:getPresets()
        local rows = p:getPresetsLua()
        list:get(0):setName("B")
        return p == dd:getDevicePresets(), list == p:getPresets(),
            list:get(0) == p:getPresets():get(0), rows[0].name,
            p:getPresetsLua()[0].name
    "#).unwrap();
    assert_eq!(strs(&result), vec!["true", "true", "true", "A", "B"]);
}

#[test]
fn assigning_device_presets_shares_the_owner_and_replacement_keeps_old_aliases() {
    let mut t = setup();
    let d = DeviceData::default();
    d.presets.add_preset("A", 88000);
    device(&mut t, d);
    t.globals().set_str("a", t.globals().get_str("dd"));
    let d = DeviceData::default();
    d.presets.add_preset("B", 90000);
    device(&mut t, d);
    let result = run(&mut t, r#"
        local ap, bp = a:getDevicePresets(), dd:getDevicePresets()
        local old_list = ap:getPresets()
        a:setDevicePresets(bp)
        bp:addPreset("C", 92000)
        local shared = a:getDevicePresets() == bp
        local count = a:getDevicePresets():getPresets():size()
        a:setDevicePresets(nil)
        return shared, count, ap:getPresetName(0), old_list:get(0):getName(),
            a:getDevicePresets() == bp, a:getDevicePresets():getPresets():size(),
            bp:getPresets():size(), bp:getPresetName(1)
    "#).unwrap();
    assert_eq!(strs(&result), vec!["true", "2.0", "A", "A", "false", "0.0", "2.0", "C"]);
}

#[test]
fn cloning_presets_clears_before_rereading_source_and_copies_entries() {
    let mut t = setup();
    let d = DeviceData::default();
    d.presets.add_preset("A", 88000);
    device(&mut t, d);
    t.globals().set_str("a", t.globals().get_str("dd"));
    let d = DeviceData::default();
    d.presets.add_preset("B", 90000);
    d.presets.add_preset("C", 92000);
    device(&mut t, d);
    let result = run(&mut t, r#"
        local ap, bp = a:getDevicePresets(), dd:getDevicePresets()
        local removed = ap:getPresets():get(0)
        a:cloneDevicePresets(ap)
        local empty = ap:getPresets():isEmpty()
        ap:setMaxPresets(1)
        a:cloneDevicePresets(bp)
        local copy = ap:getPresets():get(0)
        local original = bp:getPresets():get(0)
        original:setName("source changed")
        return empty, removed:getName(), ap:getMaxPresets(),
            ap:getPresets():size(), copy == original, copy:getName(), original:getName()
    "#).unwrap();
    assert_eq!(strs(&result), vec!["true", "A", "1.0", "1.0", "false", "B", "source changed"]);
}

struct ItemCallerEnv {
    base: FallbackEnv,
    mode: NetMode,
}
impl DeviceEnv for ItemCallerEnv {
    fn net(&self) -> NetMode { self.mode }
    fn minutes_stamp(&self) -> i64 { self.base.minutes_stamp() }
    fn multiplier(&self) -> f32 { self.base.multiplier() }
    fn parent(&self) -> ParentKind { self.base.parent() }
    fn next_int(&mut self, n: i32) -> i32 { self.base.next_int(n) }
    fn next_float(&mut self) -> f32 { self.base.next_float() }
    fn mains_power(&self) -> MainsPower { self.base.mains_power() }
    fn channel_list(&self, c: &str) -> Option<Vec<(i32, String)>> { self.base.channel_list(c) }
    fn media_exists(&self, i: i16) -> bool { self.base.media_exists(i) }
    fn media_line(&self, i: i16, n: i32) -> Option<MediaLine> { self.base.media_line(i, n) }
    fn iso_object(&self) -> Option<IsoObjectPos> { self.base.iso_object() }
    fn closest_listener_distance_sq(&self, x: f32, y: f32, z: f32) -> f32 { self.base.closest_listener_distance_sq(x, y, z) }
    fn parent_pos(&self) -> [f32; 3] { self.base.parent_pos() }
    fn event(&mut self, e: DeviceEvent) { self.base.event(e) }
}
struct ItemCallerHooks(NetMode);
impl RadioLuaHooks for ItemCallerHooks {
    fn device_env(&self, owner: &DeviceOwner) -> Option<Box<dyn DeviceEnv>> {
        Some(Box::new(ItemCallerEnv { base: FallbackEnv::new(owner), mode: self.0 }))
    }
}

/// An actual InventoryItem in an actual SharedContainer stays there on a
/// client, while the device accepts its charge. The same SP call removes it.
#[test]
fn adding_battery_keeps_actual_client_item_and_removes_single_player_item() {
    for mode in [NetMode::Client, NetMode::SinglePlayer] {
        let mut t = setup();
        crate::bind_items::register(&mut t);
        crate::bind_item_container::register(&mut t);
        install_hooks(Rc::new(ItemCallerHooks(mode)));
        let d = DeviceData { has_battery: false, ..Default::default() };
        let device = device(&mut t, d);
        let mut template = pz_inventory::ItemTemplate::new(pz_inventory::ItemClass::Drainable, "Base", "Battery");
        template.use_delta = 0.1;
        let item = pz_inventory::shared::new_item(pz_inventory::InventoryItem::create(Rc::new(template), 1));
        let container = pz_inventory::shared::SharedContainer::new(-1, "none").into_ref();
        crate::bind_item_container::add_item(&mut t, &container, Some(item.clone())).unwrap();
        t.globals().set_str("battery", crate::bind_items::item_value(&item));
        run(&mut t, "dd:addBattery(battery)").unwrap();
        assert!(device.0.borrow().has_battery);
        assert_eq!(container.borrow().items.len(), if mode == NetMode::Client { 1 } else { 0 });
        install_hooks(Rc::new(NoHooks));
    }
}

/// Synthetic item-call services pin the Lua boundary's phase ordering;
/// they are not an oracle for InventoryItem virtual/Java conversion rules.
#[test]
fn adding_media_checks_then_removes_then_reads_and_writes_item_fields() {
    for mode in [NetMode::Client, NetMode::SinglePlayer] {
        let mut t = setup();
        install_hooks(Rc::new(ItemCallerHooks(mode)));
        let device = device(&mut t, DeviceData { media_type: 0, ..Default::default() });
        let result = run(&mut t, r#"
            local trace = ""
            local container = { DoRemoveItem = function(self, item) trace = trace .. "remove," end }
            local item = {
                isRecordedMedia = function() trace = trace .. "recorded," return true end,
                getMediaType = function() trace = trace .. "type," return 0 end,
                getContainer = function() trace = trace .. "container," return container end,
                getRecordedMediaIndex = function() trace = trace .. "index," return 7 end,
                getFullType = function() trace = trace .. "full," return "Base.Disc" end,
            }
            dd:addMediaItem(item)
            return trace
        "#).unwrap();
        let expected = if mode == NetMode::Client {
            "recorded,type,container,index,full,"
        } else {
            "recorded,type,container,remove,index,full,"
        };
        assert_eq!(strs(&result), vec![expected]);
        assert_eq!(device.0.borrow().media_index, 7);
        assert_eq!(device.0.borrow().media_item.as_deref(), Some("Base.Disc"));
        install_hooks(Rc::new(NoHooks));
    }
}

#[test]
fn media_removal_failure_keeps_device_empty_and_original_lua_error() {
    let mut t = setup();
    install_hooks(Rc::new(ItemCallerHooks(NetMode::SinglePlayer)));
    let device = device(&mut t, DeviceData { media_type: 0, ..Default::default() });
    let result = run(&mut t, r#"
        local marker = {}
        local later = 0
        local container = { DoRemoveItem = function() error(marker) end }
        local item = {
            isRecordedMedia = function() return true end,
            getMediaType = function() return 0 end,
            getContainer = function() return container end,
            getRecordedMediaIndex = function() later = later + 1 return 7 end,
            getFullType = function() later = later + 1 return "Base.Disc" end,
        }
        local ok, err = pcall(function() dd:addMediaItem(item) end)
        return ok, err == marker, later
    "#).unwrap();
    assert_eq!(strs(&result), vec!["false", "true", "0.0"]);
    assert_eq!(device.0.borrow().media_index, -1);
    assert_eq!(device.0.borrow().media_item, None);
    install_hooks(Rc::new(NoHooks));
}

#[test]
fn media_full_type_failure_retains_only_the_prior_index_write() {
    let mut t = setup();
    install_hooks(Rc::new(ItemCallerHooks(NetMode::SinglePlayer)));
    let device = device(&mut t, DeviceData { media_type: 0, ..Default::default() });
    let result = run(&mut t, r#"
        local marker = {}
        local removed = 0
        local container = { DoRemoveItem = function() removed = removed + 1 end }
        local item = {
            isRecordedMedia = function() return true end,
            getMediaType = function() return 0 end,
            getContainer = function() return container end,
            getRecordedMediaIndex = function() return 7 end,
            getFullType = function() error(marker) end,
        }
        local ok, err = pcall(function() dd:addMediaItem(item) end)
        return ok, err == marker, removed
    "#).unwrap();
    assert_eq!(strs(&result), vec!["false", "true", "1.0"]);
    assert_eq!(device.0.borrow().media_index, 7);
    assert_eq!(device.0.borrow().media_item, None);
    install_hooks(Rc::new(NoHooks));
}

/// The actual WorldDevice accessor can stop publishing DeviceData. This
/// models a missing positional owner without claiming replacement identity.
struct OptionalItemCallDevice(RefCell<Option<DeviceData>>);
impl WorldDevice for OptionalItemCallDevice {
    fn with_device(&self, f: &mut dyn FnMut(&mut DeviceData)) {
        if let Some(data) = self.0.borrow_mut().as_mut() { f(data); }
    }
}

fn optional_item_call_device(t: &mut Thread, data: Option<DeviceData>) -> Rc<OptionalItemCallDevice> {
    let owner = Rc::new(OptionalItemCallDevice(RefCell::new(data)));
    let world: Rc<dyn WorldDevice> = owner.clone();
    t.globals().set_str("dd", Value::Object(Rc::new(LuaDeviceData(DeviceOwner::World(world)))));
    owner
}

fn install_item_call_owner_removal(
    t: &mut Thread,
    owner: &Rc<OptionalItemCallDevice>,
) -> Rc<RefCell<Option<DeviceData>>> {
    let removed = Rc::new(RefCell::new(None));
    let slot = owner.clone();
    let retained = removed.clone();
    t.globals().set_str("removeDeviceOwner", Value::Native(pz_kahlua::NativeFunction::new("removeDeviceOwner", move |_| {
        *retained.borrow_mut() = slot.0.borrow_mut().take();
        Ok(0)
    })));
    removed
}

fn assert_item_call_capability_error(result: &[Value]) {
    assert!(matches!(result.first(), Some(Value::Bool(false))));
    let text = match result.get(1) { Some(Value::Str(s)) => s.as_ref(), other => panic!("expected capability text: {other:?}") };
    assert!(text.contains("port capability unavailable: actual radio DeviceData owner for item call"), "{text}");
}

#[test]
fn missing_initial_device_owner_never_returns_success_or_reads_the_item() {
    for method in ["addBattery", "addMediaItem"] {
        let mut t = setup();
        optional_item_call_device(&mut t, None);
        t.globals().set_str("insertMethod", Value::str(method));
        let result = run(&mut t, r#"
            local touched = 0
            local item = setmetatable({}, { __index = function() touched = touched + 1 end })
            local ok, err = pcall(function() dd[insertMethod](dd, item) end)
            return ok, err, touched
        "#).unwrap();
        assert_item_call_capability_error(&result);
        assert!(matches!(result.get(2), Some(Value::Num(n)) if *n == 0.0));
    }
}

#[test]
fn item_callbacks_removing_actual_world_device_stop_at_the_completed_prefix() {
    for phase in ["battery-container", "media-container", "media-index", "media-full"] {
        let mut t = setup();
        install_hooks(Rc::new(ItemCallerHooks(NetMode::SinglePlayer)));
        let owner = optional_item_call_device(&mut t, Some(DeviceData { has_battery: false, media_type: 0, ..Default::default() }));
        let removed = install_item_call_owner_removal(&mut t, &owner);
        t.globals().set_str("dropPhase", Value::str(phase));
        let result = run(&mut t, r#"
            local trace = ""
            local container = { DoRemoveItem = function() trace = trace .. "remove," end }
            local item = {
                isRecordedMedia = function() trace = trace .. "recorded," return true end,
                getMediaType = function() trace = trace .. "type," return 0 end,
                getContainer = function()
                    trace = trace .. "container,"
                    if dropPhase == "battery-container" or dropPhase == "media-container" then removeDeviceOwner() end
                    return container
                end,
                getCurrentUsesFloat = function() trace = trace .. "uses," return 0.5 end,
                getRecordedMediaIndex = function()
                    trace = trace .. "index,"
                    if dropPhase == "media-index" then removeDeviceOwner() end
                    return 7
                end,
                getFullType = function()
                    trace = trace .. "full,"
                    if dropPhase == "media-full" then removeDeviceOwner() end
                    return dropPhase == "battery-container" and "Base.Battery" or "Base.Disc"
                end,
            }
            local ok, err = pcall(function()
                if dropPhase == "battery-container" then dd:addBattery(item) else dd:addMediaItem(item) end
            end)
            return ok, err, trace
        "#).unwrap();
        assert_item_call_capability_error(&result);
        let expected = match phase {
            "battery-container" => "full,container,",
            "media-container" => "recorded,type,container,",
            "media-index" => "recorded,type,container,remove,index,",
            "media-full" => "recorded,type,container,remove,index,full,",
            _ => unreachable!(),
        };
        assert!(matches!(result.get(2), Some(Value::Str(s)) if s.as_ref() == expected));
        assert!(owner.0.borrow().is_none());
        let retained = removed.borrow();
        let data = retained.as_ref().unwrap();
        assert!(!data.has_battery);
        assert_eq!(data.media_index, if phase == "media-full" { 7 } else { -1 });
        assert_eq!(data.media_item, None);
        install_hooks(Rc::new(NoHooks));
    }
}

#[test]
fn item_method_lookup_removing_owner_skips_the_returned_callable() {
    let mut t = setup();
    let owner = optional_item_call_device(&mut t, Some(DeviceData { has_battery: false, ..Default::default() }));
    install_item_call_owner_removal(&mut t, &owner);
    let result = run(&mut t, r#"
        local invoked = 0
        local item = setmetatable({}, { __index = function()
            removeDeviceOwner()
            return function() invoked = invoked + 1 return "Base.Battery" end
        end })
        local ok, err = pcall(function() dd:addBattery(item) end)
        return ok, err, invoked
    "#).unwrap();
    assert_item_call_capability_error(&result);
    assert!(matches!(result.get(2), Some(Value::Num(n)) if *n == 0.0));
}

struct RemovingItemEnvFactory(Rc<OptionalItemCallDevice>);
impl RadioLuaHooks for RemovingItemEnvFactory {
    fn device_env(&self, owner: &DeviceOwner) -> Option<Box<dyn DeviceEnv>> {
        self.0.0.borrow_mut().take();
        Some(Box::new(ItemCallerEnv { base: FallbackEnv::new(owner), mode: NetMode::SinglePlayer }))
    }
}

#[test]
fn environment_factory_removing_owner_cannot_default_to_single_player_removal() {
    let mut t = setup();
    let owner = optional_item_call_device(&mut t, Some(DeviceData { has_battery: false, ..Default::default() }));
    install_hooks(Rc::new(RemovingItemEnvFactory(owner.clone())));
    let result = run(&mut t, r#"
        local later = 0
        local container = { DoRemoveItem = function() later = later + 1 end }
        local item = {
            getFullType = function() return "Base.Battery" end,
            getContainer = function() return container end,
            getCurrentUsesFloat = function() later = later + 1 return 0.5 end,
        }
        local ok, err = pcall(function() dd:addBattery(item) end)
        return ok, err, later
    "#).unwrap();
    assert_item_call_capability_error(&result);
    assert!(matches!(result.get(2), Some(Value::Num(n)) if *n == 0.0));
    assert!(owner.0.borrow().is_none());
    install_hooks(Rc::new(NoHooks));
}

#[test]
fn original_item_error_survives_even_when_it_also_removes_the_owner() {
    let mut t = setup();
    let owner = optional_item_call_device(&mut t, Some(DeviceData { has_battery: false, ..Default::default() }));
    install_item_call_owner_removal(&mut t, &owner);
    let result = run(&mut t, r#"
        local marker = {}
        local item = { getFullType = function() removeDeviceOwner() error(marker) end }
        local ok, err = pcall(function() dd:addBattery(item) end)
        return ok, err == marker
    "#).unwrap();
    assert_eq!(strs(&result), vec!["false", "true"]);
}

fn assert_item_call_environment_error(result: &[Value]) {
    assert!(matches!(result.first(), Some(Value::Bool(false))));
    let text = match result.get(1) { Some(Value::Str(s)) => s.as_ref(), other => panic!("expected capability text: {other:?}") };
    assert!(text.contains("port capability unavailable: actual radio DeviceEnv for item call"), "{text}");
}

#[test]
fn missing_item_environment_stops_before_container_removal_and_item_reads() {
    let mut t = setup();
    install_hooks(Rc::new(NoHooks));
    let device = device(&mut t, DeviceData { has_battery: false, power_delta: 0.3, ..Default::default() });
    let result = run(&mut t, r#"
        local later = 0
        local container = { DoRemoveItem = function() later = later + 1 end }
        local item = {
            getFullType = function() return "Base.Battery" end,
            getContainer = function() return container end,
            getCurrentUsesFloat = function() later = later + 1 return 0.5 end,
        }
        local ok, err = pcall(function() dd:addBattery(item) end)
        return ok, err, later
    "#).unwrap();
    assert_item_call_environment_error(&result);
    assert!(matches!(result.get(2), Some(Value::Num(n)) if *n == 0.0));
    assert!(!device.0.borrow().has_battery);
    assert_eq!(device.0.borrow().power_delta, 0.3);
}

struct ItemEnvironmentForNetOnly(Rc<std::cell::Cell<usize>>);
impl RadioLuaHooks for ItemEnvironmentForNetOnly {
    fn device_env(&self, owner: &DeviceOwner) -> Option<Box<dyn DeviceEnv>> {
        let call = self.0.get();
        self.0.set(call + 1);
        (call == 0).then(|| Box::new(ItemCallerEnv {
            base: FallbackEnv::new(owner), mode: NetMode::SinglePlayer,
        }) as Box<dyn DeviceEnv>)
    }
}

#[test]
fn environment_missing_at_transmit_preserves_completed_media_fields_and_removal() {
    let mut t = setup();
    let calls = Rc::new(std::cell::Cell::new(0));
    install_hooks(Rc::new(ItemEnvironmentForNetOnly(calls.clone())));
    let device = device(&mut t, DeviceData { media_type: 0, ..Default::default() });
    let result = run(&mut t, r#"
        local removed = 0
        local container = { DoRemoveItem = function() removed = removed + 1 end }
        local item = {
            isRecordedMedia = function() return true end,
            getMediaType = function() return 0 end,
            getContainer = function() return container end,
            getRecordedMediaIndex = function() return 7 end,
            getFullType = function() return "Base.Disc" end,
        }
        local ok, err = pcall(function() dd:addMediaItem(item) end)
        return ok, err, removed
    "#).unwrap();
    assert_item_call_environment_error(&result);
    assert!(matches!(result.get(2), Some(Value::Num(n)) if *n == 1.0));
    assert_eq!(calls.get(), 2);
    assert_eq!(device.0.borrow().media_index, 7);
    assert_eq!(device.0.borrow().media_item.as_deref(), Some("Base.Disc"));
    install_hooks(Rc::new(NoHooks));
}
