use super::*;
use pz_kahlua::TableRef;

fn thread() -> Thread {
    pz_ui::UIManager::reset();
    pz_ui::with_ui(|m| m.time_override_ms = Some(1_000_000));
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_ui::register(&mut t);
    register(&mut t);
    t
}

fn run(t: &mut Thread, src: &str) {
    let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
    let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
        Rc::new(p),
        t.globals().clone(),
    ));
    if let Err((e, tr)) = t.pcall(&f, &[]) {
        panic!("lua error: {e:?}\n{tr}");
    }
}

fn eval(t: &mut Thread, expr: &str) -> Value {
    run(t, &format!("__r = {expr}"));
    t.globals().get_str("__r")
}

/// `MapSpawnSelectImage:instantiate()` as the main menu runs it.
#[test]
fn menu_instantiate() {
    let mut t = thread();
    run(
        &mut t,
        r#"
        local o = {}
        local jo = UIWorldMap.new(o)
        api = jo:getAPIv3()
        api:setBoolean("ClampBaseZoomToPoint5", false)
        api:setBoolean("DebugInfo", false)
        api:setBoolean("ImagePyramid", true)
        api:setBoolean("InfiniteZoom", true)
        api:setBoolean("Isometric", false)
        jo:setX(0) jo:setY(0) jo:setWidth(800) jo:setHeight(600)
        jo:setAnchorLeft(true)
        same = jo:getAPIv3() == api and jo:getAPI() == api
        v1 = jo:getAPIv1()
        jobj = jo
        "#,
    );
    assert!(eval(&mut t, "same").truthy());
    assert!(!eval(&mut t, "api:getBoolean('Isometric')").truthy());
    assert!(eval(&mut t, "v1:getBoolean('InfiniteZoom')").truthy());
    assert_eq!(eval(&mut t, "api:getOptionCount()").as_num(), Some(49.0));
    assert!(eval(&mut t, "api:getOptionByIndex(30):getName() == 'Isometric'").truthy());
    run(&mut t, "api:getOptionByIndex(30):setValue(true)");
    assert!(eval(&mut t, "api:getBoolean('Isometric')").truthy());
    assert_eq!(eval(&mut t, "api:getDataCount()").as_num(), Some(0.0));
    assert!(eval(&mut t, "api:isDataLoaded()").truthy());
    assert_eq!(eval(&mut t, "api:uiToWorldX(10, 10)").as_num(), Some(0.0));
    assert_eq!(
        eval(&mut t, "api:getDouble('Isometric', 2.5)").as_num(),
        Some(2.5)
    );
}

/// View arithmetic through the Lua API once the map has bounds.
#[test]
fn view_methods() {
    let mut t = thread();
    run(
        &mut t,
        r#"
        jo = UIWorldMap.new({})
        api = jo:getAPIv3()
        api:setBoolean("Isometric", false)
        jo:setWidth(800) jo:setHeight(600)
        api:setBoundsInCells(0, 0, 9, 9)
        api:setZoom(14)
        "#,
    );
    let base = eval(&mut t, "api:getBaseZoom()").as_num().unwrap();
    assert!(base >= 12.0);
    assert_eq!(eval(&mut t, "api:getMaxZoom()").as_num(), Some(18.0));
    run(&mut t, "api:setMaxZoom(30)");
    assert_eq!(eval(&mut t, "api:getMaxZoom()").as_num(), Some(24.0));
    assert_eq!(
        eval(&mut t, "api:getWidthInSquares()").as_num(),
        Some(2560.0)
    );
}

/// `setBoundsFromWorld` reads `IsoWorld.instance.getMetaGrid()`'s cell
/// bounds and NPEs without a meta grid.
#[test]
fn bounds_from_world_meta_grid() {
    let mut t = thread();
    crate::bind_world::set_meta_grid(None);
    run(
        &mut t,
        r#"
        jo = UIWorldMap.new({})
        api = jo:getAPIv3()
        ok = pcall(function() api:setBoundsFromWorld() end)
        "#,
    );
    assert!(matches!(t.globals().get_str("ok"), Value::Bool(false)));
    crate::bind_world::set_meta_grid(Some(pz_world::meta_grid::IsoMetaGrid::with_bounds(
        2, 3, 40, 50,
    )));
    run(&mut t, "api:setBoundsFromWorld()");
    assert_eq!(
        crate::bind_world::meta_grid_cell_bounds(),
        Some((2, 3, 40, 50))
    );
    let v = eval(&mut t, "api:getMinXInCells() .. ',' .. api:getMinYInCells() .. ',' .. api:getMaxXInCells() .. ',' .. api:getMaxYInCells()");
    assert_eq!(pz_kahlua::exec::java_value_of(&v), "2,3,40,50");
    crate::bind_world::set_meta_grid(None);
}
