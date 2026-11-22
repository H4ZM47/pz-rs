use super::*;

fn init(field: &str, id: &str) -> StaticInit {
    StaticInit {
        class: "zombie/core/GameMode".into(),
        field: field.into(),
        value: Value::Call {
            class: "zombie/core/GameMode".into(),
            name: "registerBase".into(),
            descriptor: "(Ljava/lang/String;)Lzombie/core/GameMode;".into(),
            args: vec![Value::Str(id.into())],
        },
    }
}
fn modes() -> NativeGameModeStaticsRef {
    Rc::new(
        NativeGameModeStatics::from_class_initializers(&[
            init("SANDBOX", "FixtureSandbox"),
            init("TUTORIAL", "FixtureTutorial"),
            StaticInit {
                class: "zombie/core/GameMode".into(),
                field: "ALIAS".into(),
                value: Value::Static {
                    class: "zombie/core/GameMode".into(),
                    name: "TUTORIAL".into(),
                },
            },
        ])
        .unwrap(),
    )
}
#[test]
fn static_aliases_and_managers_share_the_actual_registry_keys() {
    let modes = modes();
    let one = crate::ScriptManager::with_native_game_mode_statics(&modes);
    let two = crate::ScriptManager::with_native_game_mode_statics(&modes);
    let tutorial = modes.constant("TUTORIAL").unwrap();
    assert!(Rc::ptr_eq(tutorial, modes.constant("ALIAS").unwrap()));
    for sm in [&one, &two] {
        let key = crate::objects::items::GameMode::constant(sm, "TUTORIAL").unwrap();
        assert!(Rc::ptr_eq(&key, tutorial));
        let found = crate::objects::items::GameMode::get_id(sm, "fixturetutorial")
            .unwrap()
            .unwrap();
        assert!(Rc::ptr_eq(&found, tutorial));
    }
    let custom = modes
        .registry()
        .borrow_mut()
        .register_id("fixture:custom", false)
        .unwrap();
    assert!(Rc::ptr_eq(
        &modes.get_id("fixture:custom").unwrap().unwrap(),
        &custom
    ));
    one.registries.reset_all();
    assert!(modes.get_id("fixture:custom").unwrap().is_none());
    assert!(Rc::ptr_eq(
        &crate::objects::items::GameMode::constant(&two, "TUTORIAL").unwrap(),
        tutorial
    ));
}
#[test]
fn duplicate_registration_does_not_replace_the_retained_static_authority() {
    let modes = modes();
    let original = modes.constant("TUTORIAL").unwrap().clone();
    assert!(modes
        .registry()
        .borrow_mut()
        .register_id("FixtureTutorial", true)
        .is_err());
    let replaced = modes.get_id("FixtureTutorial").unwrap().unwrap();
    assert!(!Rc::ptr_eq(&replaced, &original));
    assert!(Rc::ptr_eq(modes.constant("TUTORIAL").unwrap(), &original));
    assert!(matches!(
        modes.get_id(""),
        Err(ScriptAccessError::Native(_))
    ));
    let _guard = modes.registry().borrow_mut();
    assert!(matches!(
        modes.get_id("FixtureTutorial"),
        Err(ScriptAccessError::Borrowed(_))
    ));
}
