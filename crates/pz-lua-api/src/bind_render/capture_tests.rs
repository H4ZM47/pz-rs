use super::*;
use pz_scripts::objects::items::Item;
use pz_scripts::{ScriptManager, ScriptObject};

struct Restore {
    fs: Option<Rc<RefCell<GameFs>>>,
    loader: Option<pz_render::TextureRefLoader>,
    observer: Option<pz_render::TextureObserver>,
}
impl Restore {
    fn new() -> Self {
        Self {
            fs: FS.with(|slot| slot.borrow_mut().take()),
            loader: pz_render::install_texture_ref_loader(None),
            observer: pz_render::install_texture_observer(None),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        let old = FS.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), self.fs.take()));
        let loader = pz_render::install_texture_ref_loader(self.loader.take());
        pz_render::install_texture_observer(self.observer.take());
        drop((old, loader));
        pz_inventory::component_host::drain_retired_components();
        drop(pz_scripts::texture_capture::take_retired_textures());
    }
}
fn fixture() -> (std::path::PathBuf, Rc<RefCell<GameFs>>) {
    let path = std::env::temp_dir().join(format!(
        "pz-capture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(path.join("media/textures")).unwrap();
    // Independent 2x1 RGBA PNG: opaque red, transparent blue.
    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 244, 34, 127, 138, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0,
        70, 0, 14, 250, 2, 254, 245, 30, 76, 169, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    std::fs::write(path.join("media/textures/Item_ActualCapture.png"), PNG).unwrap();
    let fs = Rc::new(RefCell::new(GameFs::new(&path)));
    (path, fs)
}
#[test]
fn early_real_script_capture_and_lua_share_actual_mutable_texture_with_real_pixels() {
    let _restore = Restore::new();
    let (path, fs) = fixture();
    install_texture_capture(fs.clone()); // Genuine startup phase, before Lua registration.
    let sm = ScriptManager::new();
    sm.enable_texture_capture();
    let mut item = Item::new();
    {
        let _shared_boot_read = fs.borrow();
        item.load(&sm, "Actual", "item Actual { Icon=ActualCapture, }")
            .unwrap();
    }
    assert!(sm.texture_host_fault().is_none() && item.texture_capture_complete());
    let actual = item.normal_texture_slot().unwrap().clone();
    let descriptor = pz_render::try_texture_descriptor(&actual).unwrap();
    assert_eq!((descriptor.width, descriptor.height), (2, 1));
    let manager = textures();
    let manager = manager.borrow();
    let data = manager.data(descriptor.id.unwrap()).unwrap();
    let image = data.image.as_ref().unwrap();
    assert_eq!(&image.data.pixels[..8], &[255, 0, 0, 255, 0, 0, 255, 0]);
    drop(manager);
    actual.0.borrow_mut().offset_x = 23.0;
    let mut lua = LuaManager::init(fs.clone()).unwrap();
    let get = lua.thread.globals().get_str("tryGetTexture");
    let value = lua
        .thread
        .call(&get, &[Value::str("Item_ActualCapture")])
        .unwrap()
        .remove(0);
    assert!(value.raw_equals(&texture_value_ref(Some(&actual))));
    let again = pz_render::try_get_texture_ref("Item_ActualCapture")
        .unwrap()
        .unwrap();
    assert!(Rc::ptr_eq(&actual, &again));
    assert_eq!(again.0.borrow().offset_x, 23.0);
    let prototype = pz_kahlua::compiler::compile(
        "return function(texture) return texture:getOffsetX() end",
        Some("captured-texture-offset"),
        Some("captured-texture-offset"),
        None,
    )
    .unwrap();
    let factory = Value::Closure(pz_kahlua::vm::LuaClosure::new(
        Rc::new(prototype),
        lua.thread.globals().clone(),
    ));
    let offset = lua.thread.call(&factory, &[]).unwrap().remove(0);
    let held = actual.0.borrow_mut();
    assert!(Rc::ptr_eq(
        &pz_render::try_get_texture_ref("Item_ActualCapture")
            .unwrap()
            .unwrap(),
        &actual
    ));
    assert!(matches!(
        lua.thread.call(&offset, &[value]),
        Err(pz_kahlua::LuaError::Runtime {
            class: "pz.host.Borrowed",
            ..
        })
    ));
    drop(held);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn actual_file_system_and_image_manager_borrow_faults_are_not_native_misses() {
    let _restore = Restore::new();
    let (path, fs) = fixture();
    install_texture_capture(fs.clone());
    {
        let _held = fs.borrow_mut();
        assert_eq!(
            pz_render::try_get_texture_ref("Item_ActualCapture").unwrap_err(),
            TextureHostError::Borrowed("texture file system")
        );
    }
    let manager = textures();
    {
        let _held = manager.borrow_mut();
        assert_eq!(
            pz_render::try_get_texture_ref("Item_ActualCapture").unwrap_err(),
            TextureHostError::Borrowed("texture manager")
        );
    }
    assert!(pz_render::try_get_texture_ref("Item_ActualCapture")
        .unwrap()
        .is_some());
    assert!(pz_render::try_get_texture_ref("NoSuchCapturedAsset")
        .unwrap()
        .is_none());
    std::fs::remove_dir_all(path).unwrap();
}
fn after_host_guards(_: &TextureRef) {
    assert!(OBJECTS.with(|cache| cache.try_borrow_mut().is_ok()));
    assert!(FS.with(|slot| slot.borrow().as_ref().unwrap().try_borrow_mut().is_ok()));
    assert!(textures().try_borrow_mut().is_ok());
}
#[test]
fn actual_texture_observer_runs_guard_free_for_new_and_cached_capture() {
    let _restore = Restore::new();
    let (path, fs) = fixture();
    install_texture_capture(fs);
    pz_render::install_texture_observer(Some(after_host_guards));
    let first = pz_render::try_get_texture_ref("Item_ActualCapture")
        .unwrap()
        .unwrap();
    let second = pz_render::try_get_texture_ref("Item_ActualCapture")
        .unwrap()
        .unwrap();
    assert!(Rc::ptr_eq(&first, &second));
    std::fs::remove_dir_all(path).unwrap();
}
