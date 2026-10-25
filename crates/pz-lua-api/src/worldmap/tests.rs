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
