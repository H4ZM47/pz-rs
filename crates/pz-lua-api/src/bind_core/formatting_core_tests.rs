use super::*;

#[test]
fn missing_format_authority_is_not_a_default_core_or_false() {
    assert!(SHARED.with(|slot| slot.borrow().is_none()));
    for result in [try_format_core_debug(), try_format_core_is_dev()] {
        assert!(matches!(result, Err(LuaError::Runtime {
            class: "pz.host.Unsupported", message
        }) if message == "formatter current Core authority unavailable"));
    }
    assert!(SHARED.with(|slot| slot.borrow().is_none()));
}

#[test]
fn scalar_reads_use_the_same_current_owner_and_release_all_guards() {
    let actual = Rc::new(RefCell::new(Core::new()));
    actual.borrow_mut().debug = false;
    actual.borrow_mut().is_dev = true;
    install_core(actual.clone());
    assert!(!try_format_core_debug().unwrap());
    assert!(try_format_core_is_dev().unwrap());
    {
        let mut body = actual.try_borrow_mut().unwrap();
        body.debug = true;
        body.is_dev = false;
    }
    assert!(try_format_core_debug().unwrap());
    assert!(!try_format_core_is_dev().unwrap());
    let _slot = SHARED.with(|slot| slot.try_borrow_mut().is_ok());
    assert!(_slot);
}

#[test]
fn a_replaced_authority_is_reacquired_instead_of_captured_by_a_plan() {
    let first = Rc::new(RefCell::new(Core::new()));
    first.borrow_mut().debug = true;
    first.borrow_mut().is_dev = false;
    install_core(first.clone());
    assert!(try_format_core_debug().unwrap());
    let second = Rc::new(RefCell::new(Core::new()));
    second.borrow_mut().debug = false;
    second.borrow_mut().is_dev = true;
    install_core(second.clone());
    assert!(!try_format_core_debug().unwrap());
    assert!(try_format_core_is_dev().unwrap());
    assert!(Rc::ptr_eq(&current_format_core().unwrap(), &second));
    assert!(first.borrow().debug);
}

#[test]
fn busy_slot_and_body_are_typed_and_readers_do_not_mutate_the_authority() {
    let actual = Rc::new(RefCell::new(Core::new()));
    actual.borrow_mut().debug = true;
    install_core(actual.clone());
    SHARED.with(|slot| {
        let _held = slot.borrow_mut();
        assert!(matches!(try_format_core_debug(), Err(LuaError::Runtime {
            class: "pz.host.Borrowed", message
        }) if message == "formatter current Core slot"));
    });
    let _deferral = pz_kahlua::gc::defer_collection();
    let held = actual.borrow_mut();
    for result in [try_format_core_debug(), try_format_core_is_dev()] {
        assert!(matches!(result, Err(LuaError::Runtime { class: "pz.host.Borrowed", .. })));
    }
    assert!(held.debug);
}
