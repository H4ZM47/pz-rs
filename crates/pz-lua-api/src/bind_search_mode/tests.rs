use super::*;
use crate::bind_character_sim::LuaGameCharacter;
use pz_characters::CharacterKind;
use pz_scripts::ScriptManager;

fn thread() -> Thread {
    reset();
    let mut t = Thread::new(pz_kahlua::TableRef::new());
    register(&mut t);
    t
}

fn run(t: &mut Thread, src: &str) -> Vec<Value> {
    let prelude = "local n = 0\nlocal function check(c, m) n = n + 1 \
                   if not c then error((m or 'check failed') .. ' #' .. n) end end\n";
    let f = pz_kahlua::stdlib::load_string(&format!("{prelude}{src}"), Some("test"), t.globals().clone()).expect("compile");
    t.call(&f, &[]).unwrap_or_else(|e| panic!("runtime error: {e}"))
}

#[test]
fn lua_api_drives_the_shared_instance() {
    let mut t = thread();
    run(
        &mut t,
        r#"
        local sm = getSearchMode()
        check(sm == getSearchMode() and sm == SearchMode.getInstance())
        local p = sm:getSearchModeForPlayer(1)
        check(p == sm:getSearchModeForPlayer(1))
        check(p:getBlur() == sm:getBlur(1))
        check(sm:getFadeTime() == 1)
        check(p:getRadius():getMax() == 50 and p:getGradientWidth():getMax() == 20)
        check(p:getDesat():getStepsize() > 0.0099 and p:getDesat():getStepsize() < 0.0101)
        check(p:getRadius():getTargetExterior() == 4)
        p:getBlur():setTargets(0.5, 0.25)
        p:getRadius():setTargets(8, 6)
        check(not sm:isEnabled(1) and not p:isShaderEnabled())
        sm:setEnabled(1, true)
        check(sm:isEnabled(1) and p:isShaderEnabled())
        sm:setOverrideSearchManager(1, true)
        check(sm:isOverrideSearchManager(1) and not sm:isOverride(1))
        "#,
    );
    update(0.5);
    let s = instance();
    let m = *s.borrow().player(1).unwrap();
    assert_eq!(m.blur.exterior(), 0.25);
    assert_eq!(m.blur.interior(), 0.125);
    assert_eq!(m.radius.exterior(), 4.0);
    // No player in slot 1: interior values.
    assert_eq!(
        shader_inputs(1).unwrap(),
        SearchModeShaderInputs { enabled: true, blur: 0.125, radius: 3.0, gradient_width: 0.04, desat: 0.85 * 0.5, darkness: 0.0 }
    );
    run(
        &mut t,
        r#"
        local p = getSearchMode():getSearchModeForPlayer(1)
        check(p:getShaderRadius() == 3)
        check(p:getRadius():getExterior() == 4)
        p:getRadius():setAll(2)
        check(p:getRadius():getInterior() == 2 and p:getRadius():getTargetExterior() == 2)
        p:getRadius():resetAll()
        check(p:getRadius():getTargetInterior() == 0)
        "#,
    );
}

#[test]
fn bad_slots_throw() {
    let mut t = thread();
    let f = pz_kahlua::stdlib::load_string("return getSearchMode():isEnabled(4)", Some("test"), t.globals().clone()).unwrap();
    let e = t.call(&f, &[]).unwrap_err();
    assert!(e.to_string().contains("Index 4 out of bounds for length 4"), "{e}");
}

#[test]
fn lua_made_player_search_mode_is_detached() {
    let mut t = thread();
    run(
        &mut t,
        r#"
        local p = SearchMode.PlayerSearchMode.new(0, getSearchMode())
        check(p ~= getSearchMode():getSearchModeForPlayer(0))
        check(p:getBlur():getTargetExterior() == 0)
        p:getBlur():setTargets(1, 1)
        check(getSearchMode():getBlur(0):getTargetExterior() == 1)
        check(p:getBlur():getTargetExterior() == 1)
        "#,
    );
}

#[test]
fn screen_inputs_read_the_player_slot() {
    let _t = thread();
    let sm = ScriptManager::new();
    let chr = Rc::new(RefCell::new(LuaGameCharacter::new(&sm, CharacterKind::Player)));
    {
        let mut c = chr.borrow_mut();
        c.environment.has_current_square = true;
        c.environment.square_in_room = false;
        c.set_wearing_night_vision_goggles(true);
        c.player.blur_factor = 0.5;
    }
    bind_players::set_player(0, Some(chr.clone()));
    {
        let s = instance();
        let mut s = s.borrow_mut();
        let m = s.player_mut(0).unwrap();
        m.radius.set(7.0, 7.0, 2.0, 2.0);
    }
    let mut inputs = PlayerRenderInputs::default();
    // Without the jar's CharacterStat table INTOXICATION is unknown: the
    // error surfaces after the goggles flag is set.
    assert!(update_render_inputs(0, &mut inputs).is_err());
    assert!(inputs.night_vision_goggles);
    let base = ScreenParams { zoom: 1.0, ..Default::default() };
    let p = apply_screen_inputs(base, 0, &inputs);
    assert!(p.night_vision_goggles);
    assert_eq!(p.zoom, 1.0);
    // Outdoors: the exterior radius.
    assert_eq!(p.search.radius, 7.0);
    chr.borrow_mut().environment.square_in_room = true;
    assert_eq!(apply_screen_inputs(base, 0, &inputs).search.radius, 2.0);
    bind_players::set_player(0, None);
    assert_eq!(apply_screen_inputs(base, 0, &inputs).search.radius, 2.0);
}
