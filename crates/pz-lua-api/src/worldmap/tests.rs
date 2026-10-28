use super::*;
use crate::bind_ui::element_of;
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

/// A one-cell `.bin` (cell (2, 3), one polygon) in a fresh temp dir.
fn temp_map() -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pz-worldmap-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut b = b"IGMB".to_vec();
    for v in [2i32, 256, 1, 1] {
        b.extend(v.to_le_bytes());
    }
    let strings = ["Polygon", "building", "yes"];
    b.extend((strings.len() as i32).to_le_bytes());
    for s in strings {
        b.extend((s.len() as i16).to_le_bytes());
        b.extend(s.as_bytes());
    }
    for v in [2i32, 3, 1] {
        b.extend(v.to_le_bytes());
    }
    b.extend(0i16.to_le_bytes());
    b.push(1);
    b.extend(4i16.to_le_bytes());
    for (x, y) in [(0i16, 0i16), (10, 0), (10, 10), (0, 10)] {
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
    }
    b.push(1);
    b.extend(1i16.to_le_bytes());
    b.extend(2i16.to_le_bytes());
    let xml = dir.join("worldmap.xml");
    std::fs::write(&xml, "<world/>").unwrap();
    std::fs::write(dir.join("worldmap.xml.bin"), b).unwrap();
    dir
}

/// `addData` loads the `.bin` beside the named file, sets the data
/// bounds and resets the view once the element has a size.
#[test]
fn add_data() {
    let dir = temp_map();
    let mut t = thread();
    t.globals().set_str(
        "file",
        dir.join("worldmap.xml").to_string_lossy().to_string(),
    );
    run(
        &mut t,
        r#"
        jo = UIWorldMap.new({})
        api = jo:getAPIv3()
        jo:setWidth(400) jo:setHeight(300)
        api:addData(file)
        api:addData(file)
        api:addData("")
        api:addData(file .. ".missing")
        api:endDirectoryData()
        api:setBoundsFromData()
        "#,
    );
    assert_eq!(eval(&mut t, "api:getDataCount()").as_num(), Some(1.0));
    assert!(eval(&mut t, "api:getDataFileByIndex(0) == file").truthy());
    assert!(eval(&mut t, "api:isDataLoaded()").truthy());
    assert_eq!(
        eval(&mut t, "api:getDataWidthInCells()").as_num(),
        Some(1.0)
    );
    assert_eq!(eval(&mut t, "api:getMinXInSquares()").as_num(), Some(512.0));
    assert_eq!(
        eval(&mut t, "api:getMaxYInSquares()").as_num(),
        Some(1023.0)
    );
    // The view was reset around the bounds at the time of the first add
    // (all 0 squares): centre 0.5.
    assert_eq!(eval(&mut t, "api:getCenterWorldX()").as_num(), Some(0.5));
    run(&mut t, "api:resetView()");
    assert_eq!(eval(&mut t, "api:getCenterWorldX()").as_num(), Some(640.0));
    run(&mut t, "api:clearData()");
    assert_eq!(eval(&mut t, "api:getDataCount()").as_num(), Some(0.0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The style API as `MapUtils.initDefaultStyleV1` and the V3 text and
/// pyramid layers use it.
#[test]
fn style_api() {
    let mut t = thread();
    run(
        &mut t,
        r#"
        jo = UIWorldMap.new({})
        v1 = jo:getAPIv1():getStyleAPI()
        s = jo:getAPIv3():getStyleAPI()
        sameV1 = jo:getAPIv1():getStyleAPI() == v1
        s:clear()
        local l = s:newPolygonLayer("forest")
        l:setMinZoom(13.5)
        l:setFilter("natural", "forest")
        l:addFill(0, 189, 197, 163, 0)
        l:addFill(15, 189, 197, 163, 255)
        l:addTexture(0, nil)
        l:addScale(13.5, 4.0)
        local w = s:newPolygonLayer("water")
        w:setFilter("water", "*")
        local p = s:newPyramidLayer("pyr")
        p:setPyramidFileName("pyramid.zip")
        p:addFill(0, 1, 2, 3, 4)
        local tx = s:newTextLayer("text-town")
        tx:setLineHeight(80)
        tx:setFont(UIFont.Handwritten)
        s:moveLayer(0, 1)
        n = s:getLayerCount()
        first = s:getLayerByIndex(0):getId()
        idx = s:indexOfLayer("forest")
        fk = s:getLayerByName("forest"):getFilterKey()
        fz = s:getLayerByName("forest"):getFillZoom(1)
        alpha = s:getLayerByName("forest"):getFillAlpha(1)
        ts = s:getLayerByName("forest"):getTextureStops()
        lh = s:getLayerByName("text-town"):getLineHeight()
        pyr = s:getLayerByName("pyr"):getPyramidFileName()
        v1class = v1:getLayerByName("pyr"):getTypeString()
        s:getLayerByName("water"):setId("river")
        s:removeLayerById("pyr")
        n2 = v1:getLayerCount()
        hasRiver = s:indexOfLayer("river")
        "#,
    );
    assert!(eval(&mut t, "sameV1").truthy());
    assert_eq!(eval(&mut t, "n").as_num(), Some(4.0));
    assert!(eval(&mut t, "first == 'water'").truthy());
    assert_eq!(eval(&mut t, "idx").as_num(), Some(1.0));
    assert!(eval(&mut t, "fk == 'natural'").truthy());
    assert_eq!(eval(&mut t, "fz").as_num(), Some(15.0));
    assert_eq!(eval(&mut t, "alpha").as_num(), Some(255.0));
    assert_eq!(eval(&mut t, "ts").as_num(), Some(1.0));
    assert_eq!(eval(&mut t, "lh").as_num(), Some(80.0));
    assert!(eval(&mut t, "pyr == 'pyramid.zip'").truthy());
    assert!(eval(&mut t, "v1class == 'Pyramid'").truthy());
    assert_eq!(eval(&mut t, "n2").as_num(), Some(3.0));
    assert_eq!(eval(&mut t, "hasRiver").as_num(), Some(0.0));
    // V1 wraps a pyramid layer in the base class, which has no fill API.
    run(&mut t, "local p = jo:getAPIv3():getStyleAPI():newPyramidLayer('x') ok = pcall(function() v1:getLayerByName('x'):addFill(0,0,0,0,0) end)");
    assert!(!eval(&mut t, "ok").truthy());
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

/// `render()` draws the background, the styled features and the bounds
/// into the UI draw list.
#[test]
fn render_draws_features() {
    let dir = temp_map();
    let mut t = thread();
    t.globals().set_str(
        "file",
        dir.join("worldmap.xml").to_string_lossy().to_string(),
    );
    run(
        &mut t,
        r#"
        jo = UIWorldMap.new({})
        api = jo:getAPIv3()
        jo:setWidth(400) jo:setHeight(300)
        api:setBoolean("Isometric", false)
        api:addData(file)
        api:setBoundsFromData()
        api:resetView()
        api:centerOn(517, 773)
        api:setZoom(18)
        local l = api:getStyleAPI():newPolygonLayer("b")
        l:setFilter("building", "yes")
        l:addFill(0, 255, 0, 0, 255)
        "#,
    );
    let el = element_of(&eval(&mut t, "jo")).unwrap();
    pz_ui::with_ui(|m| m.sprite.clear());
    el.render(&mut t);
    let red = pz_ui::with_ui(|m| {
        m.sprite
            .cmds
            .iter()
            .filter(|c| matches!(c, pz_ui::UiDrawCmd::Render(pz_render::DrawCmd::Sprite(s)) if s.color[0] == pz_render::color_to_abgr(1.0, 0.0, 0.0, 1.0)))
            .count()
    });
    // The 10x10 square, as two triangles.
    assert_eq!(red, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn first_pyramid_creation_reads_actual_core_and_propagates_borrow_failure() {
    let mut t = thread();
    run(&mut t, "jo = UIWorldMap.new({}); api = jo:getAPIv3()");
    assert!(!pz_worldmap::pyramid::class_initialized());
    let core = crate::bind_core::shared_core();
    let mut held = core.borrow_mut();
    held.debug = true;
    run(
        &mut t,
        "ok, message = pcall(function() api:addImagePyramid('missing-pyramid.zip') end)",
    );
    assert!(!eval(&mut t, "ok").truthy());
    assert!(
        matches!(eval(&mut t, "message"), Value::Str(message) if message.contains("Core is borrowed"))
    );
    assert!(!pz_worldmap::pyramid::class_initialized());
    drop(held);
    run(&mut t, "api:addImagePyramid('missing-pyramid.zip')");
    assert_eq!(pz_worldmap::pyramid::initialize_class(false), 10_000);
    let held = core.borrow_mut();
    run(&mut t, "api:addImages('missing-image-directory')");
    assert_eq!(
        pz_worldmap::pyramid::initialize_class(false),
        10_000,
        "later calls do not requery a borrowed Core"
    );
    drop(held);
}
