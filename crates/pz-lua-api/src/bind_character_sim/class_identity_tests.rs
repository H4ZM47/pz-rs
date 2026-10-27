use super::*;

#[test]
fn actual_character_class_and_superclasses_do_not_retarget_after_body_kind_changes() {
    for (kind, class, supers) in [
        (
            CharacterKind::Player,
            ISO_PLAYER_CLASS,
            &[
                "zombie.characters.IsoLivingCharacter",
                ISO_GAME_CHARACTER_CLASS,
                "zombie.iso.IsoMovingObject",
                "zombie.iso.IsoObject",
            ][..],
        ),
        (
            CharacterKind::Survivor,
            ISO_SURVIVOR_CLASS,
            &[
                "zombie.characters.IsoLivingCharacter",
                ISO_GAME_CHARACTER_CLASS,
                "zombie.iso.IsoMovingObject",
                "zombie.iso.IsoObject",
            ][..],
        ),
        (
            CharacterKind::Zombie,
            ISO_ZOMBIE_CLASS,
            &[
                ISO_GAME_CHARACTER_CLASS,
                "zombie.iso.IsoMovingObject",
                "zombie.iso.IsoObject",
            ][..],
        ),
        (
            CharacterKind::Animal,
            ISO_ANIMAL_CLASS,
            &[
                ISO_PLAYER_CLASS,
                "zombie.characters.IsoLivingCharacter",
                ISO_GAME_CHARACTER_CLASS,
                "zombie.iso.IsoMovingObject",
                "zombie.iso.IsoObject",
            ][..],
        ),
    ] {
        let character = Rc::new(RefCell::new(LuaGameCharacter::new(
            &ScriptManager::new(),
            kind,
        )));
        let object = CharacterObject::try_new(&character).unwrap();
        let mut body = character.borrow_mut();
        body.kind = if kind == CharacterKind::Player {
            CharacterKind::Zombie
        } else {
            CharacterKind::Player
        };
        assert_eq!(object.class_name(), class);
        assert_eq!(object.super_classes(), supers);
        assert!(Rc::ptr_eq(&object.0, &character));
    }
}

#[test]
fn cold_character_publication_reports_needed_borrows_without_publishing_parent_or_chat() {
    let character = Rc::new(RefCell::new(LuaGameCharacter::new(
        &ScriptManager::new(),
        CharacterKind::Player,
    )));
    let inventory = character.borrow().items.inventory.clone();
    let key = (Handle::Character, Rc::as_ptr(&character) as usize, 0);
    {
        let _body = character.borrow_mut();
        assert!(matches!(
            try_character_value(&character),
            Err(RuntimeHostError::Borrowed("character userdata publication"))
        ));
        assert!(matches!(
            CharacterObject::try_new(&character),
            Err(RuntimeHostError::Borrowed("character userdata class"))
        ));
    }
    assert!(!HANDLES.with(|handles| handles.borrow().contains_key(&key)));
    {
        let body = inventory.borrow_mut();
        assert!(matches!(
            try_character_value(&character),
            Err(RuntimeHostError::Borrowed(
                "character userdata inventory parent"
            ))
        ));
        assert!(body.parent.is_none());
    }
    assert!(!HANDLES.with(|handles| handles.borrow().contains_key(&key)));
    assert!(character.borrow().chat.element.is_none());
    assert!(inventory.borrow().parent.is_none());
}

#[test]
fn canonical_character_lookup_and_real_lua_method_dispatch_need_no_body_borrow() {
    let (mut thread, character, _) = tests::setup_with(ScriptManager::new());
    let inventory = character.borrow().items.inventory.clone();
    let original = thread.globals().get_str("player");
    let character_guard = character.borrow_mut();
    let inventory_guard = inventory.borrow_mut();
    assert!(try_character_value(&character)
        .unwrap()
        .raw_equals(&original));
    assert!(character_value(&character).raw_equals(&original));
    let function = pz_kahlua::stdlib::load_string(
        "return player.getInventory, player.getStats",
        Some("character-class-with-held-bodies"),
        thread.globals().clone(),
    )
    .unwrap();
    let values = thread.pcall_ex(&function, &[]).unwrap();
    assert_eq!(values.len(), 2);
    assert!(values.iter().all(Value::is_function));
    assert!(crate::bind_item_container::parent_of(&inventory_guard).raw_equals(&original));
    assert_eq!(character_guard.kind, CharacterKind::Player);
}
