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
            .presets
            .iter()
            .map(|p| p.name.clone())
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
