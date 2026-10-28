use super::*;
use pz_kahlua::TableRef;

fn thread() -> Thread {
    *shared_puddles().borrow_mut() = PuddlesManager::default();
    reset_session_handles();
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    register(&mut t);
    run(&mut t, "function assert(v, m) if not v then error(m or 'assertion failed') end return v end");
    t
}

fn run(t: &mut Thread, source: &str) -> Vec<Value> {
    let f = pz_kahlua::stdlib::load_string(source, Some("puddles-test"), t.globals().clone()).unwrap();
    t.call(&f, &[]).unwrap()
}

#[test]
fn live_float_views_share_identity_admin_values_and_surface_state() {
    let mut t = thread();
    let result = run(
        &mut t,
        r#"
        local m = getPuddlesManager()
        assert(m == getPuddlesManager() and m == IsoPuddles.getInstance())
        assert(m:getFloatMax() == 5 and m:getBoolMax() == 0)
        assert(m:getPuddlesFloat(-1) == nil and m:getPuddlesFloat(5) == nil)
        local f = m:getPuddlesFloat(IsoPuddles.FLOAT_PUDDLESSIZE)
        assert(f == m:getPuddlesFloat(3) and f:getID() == 3)
        assert(f:getMin() == 0 and f:getMax() == 1 and f:getName() ~= nil)
        f:setFinalValue(0.35)
        assert(f:getFinalValue() == m:getPuddlesSize() and m:getPuddlesSize() == m:getPuddlesSizeFinalValue())
        f:setAdminValue(0.8)
        assert(not f:isEnableAdmin() and m:getPuddlesSize() < 0.4)
        f:setEnableAdmin(true)
        assert(m:getPuddlesSize() > 0.7)
        f:setEnableAdmin(false)
        assert(f:getAdminValue() > 0.7 and m:getPuddlesSize() < 0.4)
        f:setEnableAdmin(true)
        assert(m:getPuddlesSize() > 0.7)
        f:setFinalValue(0.2)
        f:setEnableAdmin(false)
        f:addFinalValue(5)
        assert(m:getPuddlesSize() == 1)
        f:addFinalValueForMax(1, 0.7)
        f:interpolateFinalValue(0.6)
        return m:getPuddlesSize(), f
    "#,
    );
    assert!((result[0].as_num().unwrap() - 0.69).abs() < 1e-6);
    assert!((shared_puddles().borrow().state.params(0)[1][2] - 0.69).abs() < 1e-6);
    // A retained Lua float remains a view when the host advances rainfall.
    shared_puddles().borrow_mut().state.update(
        &ClimateInputs {
            rain_intensity: 1.0,
            ..Default::default()
        },
        1.6,
        1.0,
    );
    t.globals().set_str("retained", result[1].clone());
    let value = run(&mut t, "return retained:getFinalValue()")[0].as_num().unwrap();
    assert!((value - 0.6905).abs() < 1e-6);
}

#[test]
fn menu_reset_retires_wrappers_but_preserves_puddle_singleton() {
    let mut old = thread();
    let values = run(&mut old, "return getPuddlesManager(), getPuddlesManager():getPuddlesFloat(3)");
    let owner = shared_puddles();
    owner.borrow_mut().state.puddles_size.set(0.45);
    crate::session_reset::reset_session_bindings();
    let mut fresh = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut fresh, None).unwrap();
    register(&mut fresh);
    let now = run(
        &mut fresh,
        "return getPuddlesManager(), getPuddlesManager():getPuddlesFloat(3), getPuddlesManager():getPuddlesSize()",
    );
    assert!(Rc::ptr_eq(&owner, &shared_puddles()));
    assert!(!values[0].raw_equals(&now[0]));
    assert!(!values[1].raw_equals(&now[1]));
    assert!((now[2].as_num().unwrap() - 0.45).abs() < 1e-6);
    // Explicitly retained old Java-object references still see the singleton.
    old.globals().set_str("retained", values[1].clone());
    owner.borrow_mut().state.puddles_size.set(0.6);
    assert!((run(&mut old, "return retained:getFinalValue()")[0].as_num().unwrap() - 0.6).abs() < 1e-6);
}

#[test]
fn weather_fx_is_real_cell_owned_idempotent_and_uses_shared_standard_rand() {
    crate::bind_world::set_cell(None);
    let fs = pz_fs::GameFs::new(std::env::temp_dir().join("pz-puddles-missing-assets"));
    assert!(!init_weather_fx(&fs));
    crate::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    pz_core::random::with_standard(|r| r.set_seed(771));
    let expected = pz_core::random::Rand::new(771);
    let reference = IsoWeatherFx::new(&expected, &WeatherFxTextures::default());
    assert!(init_weather_fx(&fs));
    let fx = crate::bind_world::with_world(|w| w.weather_fx.clone().unwrap());
    let weak = Rc::downgrade(&fx);
    assert_eq!(fx.borrow().rects[0].particles.len(), reference.rects[0].particles.len());
    assert_eq!(fx.borrow().rects[3].particles[0].position, reference.rects[3].particles[0].position);
    assert_eq!(pz_core::random::next(100_000), expected.next(100_000));
    assert!(init_weather_fx(&fs));
    assert!(Rc::ptr_eq(&fx, &crate::bind_world::with_world(|w| w.weather_fx.clone().unwrap())));
    assert_eq!(
        pz_core::random::next(100_000),
        expected.next(100_000),
        "second init consumed randomness"
    );
    drop(fx);
    crate::bind_world::set_cell(Some(pz_world::IsoCell::default()));
    assert!(weak.upgrade().is_none());
    assert!(crate::bind_world::with_world(|w| w.weather_fx.is_none()));
    crate::bind_world::set_cell(None);
}
