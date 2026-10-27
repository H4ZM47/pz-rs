use super::*;

struct Restore(Option<TextureRefLoader>, Option<TextureObserver>);
impl Restore {
    fn new() -> Self {
        Self(
            install_texture_ref_loader(None),
            install_texture_observer(None),
        )
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        let retired = install_texture_ref_loader(self.0.take());
        install_texture_observer(self.1.take());
        drop(retired);
        drop(take_retired_texture_loaders());
    }
}

#[test]
fn missing_provider_is_distinct_from_native_missing_texture_and_retained_pointer() {
    let _restore = Restore::new();
    assert_eq!(
        get_shared_texture_ref("missing").unwrap_err(),
        TextureHostError::Unavailable("texture provider")
    );
    install_texture_ref_loader(Some(Rc::new(|_, _| Ok(None))));
    assert!(get_shared_texture_ref("missing").unwrap().is_none());
    let actual = Rc::new(LuaTexture::new(Texture::empty("actual")));
    let owned = actual.clone();
    install_texture_ref_loader(Some(Rc::new(move |_, _| Ok(Some(owned.clone())))));
    let held = actual.0.borrow_mut();
    let again = try_get_texture_ref("actual").unwrap().unwrap();
    assert!(Rc::ptr_eq(&again, &actual));
    assert_eq!(
        try_texture_descriptor(&again).unwrap_err(),
        TextureHostError::Borrowed("actual texture body")
    );
    drop(held);
    actual.0.borrow_mut().offset_x = 17.0;
    assert_eq!(try_texture_descriptor(&again).unwrap().offset_x, 17.0);
}

thread_local! { static OBSERVED: Cell<usize> = const { Cell::new(0) }; }
fn observer(texture: &TextureRef) {
    assert!(REF_LOADER.with(|loader| loader.try_borrow_mut().is_ok()));
    assert!(texture.0.try_borrow_mut().is_ok());
    OBSERVED.with(|count| count.set(count.get() + 1));
}
#[test]
fn host_and_observer_run_after_registry_and_texture_guards() {
    let _restore = Restore::new();
    OBSERVED.with(|count| count.set(0));
    let actual = Rc::new(LuaTexture::new(Texture::empty("observed")));
    let owned = actual.clone();
    install_texture_ref_loader(Some(Rc::new(move |lookup, name| {
        assert_eq!((lookup, name), (TextureLookup::Shared, "observed"));
        assert!(REF_LOADER.with(|loader| loader.try_borrow_mut().is_ok()));
        {
            let mut body = owned.0.borrow_mut();
            body.offset_y += 1.0;
        }
        Ok(Some(owned.clone()))
    })));
    install_texture_observer(Some(observer));
    assert!(Rc::ptr_eq(
        &get_shared_texture_ref("observed").unwrap().unwrap(),
        &actual
    ));
    assert!(Rc::ptr_eq(
        &get_shared_texture_ref("observed").unwrap().unwrap(),
        &actual
    ));
    assert_eq!(OBSERVED.with(Cell::get), 2);
    assert_eq!(actual.0.borrow().offset_y, 2.0);
}

#[test]
fn provider_fault_is_not_converted_to_null_or_descriptor_loader_fallback() {
    let _restore = Restore::new();
    install_texture_ref_loader(Some(Rc::new(|_, _| {
        Err(TextureHostError::Borrowed("actual image manager"))
    })));
    assert_eq!(
        try_get_texture_ref("anything").unwrap_err(),
        TextureHostError::Borrowed("actual image manager")
    );
}

#[test]
fn self_replaced_last_provider_is_retired_beyond_enclosing_host_guard() {
    struct Probe {
        host: Rc<RefCell<()>>,
        drops: Rc<Cell<usize>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            assert!(self.host.try_borrow_mut().is_ok());
            self.drops.set(self.drops.get() + 1);
        }
    }
    let _restore = Restore::new();
    let host = Rc::new(RefCell::new(()));
    let drops = Rc::new(Cell::new(0));
    let probe = Probe {
        host: host.clone(),
        drops: drops.clone(),
    };
    install_texture_ref_loader(Some(Rc::new(move |_, _| {
        let _retained = &probe;
        drop(install_texture_ref_loader(None));
        Err(TextureHostError::Borrowed("completed callback fault"))
    })));
    let guard = host.borrow_mut();
    assert_eq!(
        get_shared_texture_ref("anything").unwrap_err(),
        TextureHostError::Borrowed("completed callback fault")
    );
    assert_eq!(drops.get(), 0);
    drop(guard);
    drop(take_retired_texture_loaders());
    assert_eq!(drops.get(), 1);
}
