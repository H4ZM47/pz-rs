use super::*;
use pz_kahlua::TableRef;
use std::rc::Rc;

fn setup() -> (Thread, super::super::LuaGameCharacterRef) {
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    let builder =
        crate::expose::expose_class(&mut thread, super::super::ISO_GAME_CHARACTER_CLASS, None);
    crate::expose::expose_class(
        &mut thread,
        super::super::ISO_PLAYER_CLASS,
        Some(super::super::ISO_GAME_CHARACTER_CLASS),
    );
    register(&builder);
    let character = Rc::new(RefCell::new(super::super::LuaGameCharacter::new(
        &pz_scripts::ScriptManager::new(),
        pz_characters::CharacterKind::Player,
    )));
    let value = Value::Object(Rc::new(
        super::super::CharacterObject::try_new(&character).unwrap(),
    ));
    thread.globals().set_str("player", value);
    (thread, character)
}
fn call(thread: &mut Thread, source: &str) -> LuaResult<Vec<Value>> {
    let function = pz_kahlua::stdlib::load_string(
        source,
        Some("actual actor reading"),
        thread.globals().clone(),
    )
    .unwrap();
    thread
        .pcall_ex(&function, &[])
        .map_err(|(_, _, error)| error)
}

#[test]
fn real_lua_read_state_uses_current_fields_and_absent_title_skips_busy_clock() {
    let (mut thread, character) = setup();
    character
        .borrow_mut()
        .read_print_media
        .insert("News".into(), ());
    character
        .borrow_mut()
        .read_books
        .push(pz_characters::character::save::ReadBook {
            full_type: "Base.Book".into(),
            already_read_pages: 17,
        });
    let time = crate::bind_climate::shared_game_time();
    let held = time.borrow_mut();
    let values = call(&mut thread, "return player:isLiteratureRead('absent'),player:isPrintMediaRead('News'),player:isPrintMediaRead('news'),player:getAlreadyReadPages('Base.Book'),player:getAlreadyReadPages(nil)").unwrap();
    assert!(values[0].raw_equals(&Value::Bool(false)));
    assert!(values[1].raw_equals(&Value::Bool(true)));
    assert!(values[2].raw_equals(&Value::Bool(false)));
    assert!(values[3].raw_equals(&Value::Num(17.0)));
    assert!(values[4].raw_equals(&Value::Num(0.0)));
    character
        .borrow_mut()
        .read_literature
        .insert("present".into(), 10);
    assert!(
        matches!(call(&mut thread, "return player:isLiteratureRead('present')"), Err(LuaError::Runtime{class:"pz.host.Borrowed",message}) if message=="reading GameTime.nightsSurvived")
    );
    drop(held);
}

#[test]
fn real_lua_method_dispatch_retains_typed_character_fault_and_conversion_does_not_mutate() {
    let (mut thread, character) = setup();
    let held = character.borrow_mut();
    for source in [
        "return player:isLiteratureRead('x')",
        "return player:isPrintMediaRead('x')",
        "return player:getAlreadyReadPages('x')",
    ] {
        assert!(
            matches!(
                call(&mut thread, source),
                Err(LuaError::Runtime {
                    class: "pz.host.Borrowed",
                    ..
                })
            ),
            "{source}"
        );
    }
    drop(held);
    for source in [
        "return player:isLiteratureRead()",
        "return player:isPrintMediaRead({})",
        "return player:getAlreadyReadPages(12)",
        "return player:getAlreadyReadPages('x','extra')",
    ] {
        assert!(
            matches!(
                call(&mut thread, source),
                Err(LuaError::Runtime {
                    class: "pz.host.Unsupported",
                    ..
                })
            ),
            "{source}"
        );
    }
    assert!(character.borrow().read_books.is_empty());
    assert!(character.borrow().read_literature.is_empty());
    assert!(character.borrow().read_print_media.is_empty());
}

#[test]
fn rejected_last_owned_input_is_retired_until_foreign_guards_end() {
    use std::cell::Cell;
    struct Rejected {
        foreign: Rc<RefCell<usize>>,
        dropped: Rc<Cell<bool>>,
    }
    impl pz_kahlua::value::LuaObject for Rejected {
        fn class_name(&self) -> &str {
            "test.RejectedReadingArgument"
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }
    impl Drop for Rejected {
        fn drop(&mut self) {
            *self.foreign.borrow_mut() += 1;
            self.dropped.set(true);
        }
    }
    for source in [
        "return player:isLiteratureRead(rejected())",
        "return player:getAlreadyReadPages('x',rejected())",
    ] {
        let (mut thread, _) = setup();
        let foreign = Rc::new(RefCell::new(0));
        let dropped = Rc::new(Cell::new(false));
        let slot = Rc::new(RefCell::new(Some(Value::Object(Rc::new(Rejected {
            foreign: foreign.clone(),
            dropped: dropped.clone(),
        })))));
        let producer = slot.clone();
        thread.globals().set_str(
            "rejected",
            Value::Native(pz_kahlua::NativeFunction::new(
                "rejected reading input",
                move |call| {
                    let value = producer.borrow_mut().take().unwrap();
                    Ok(call.push(value))
                },
            )),
        );
        let deferral = pz_kahlua::gc::defer_collection();
        let held = foreign.borrow_mut();
        assert!(matches!(
            call(&mut thread, source),
            Err(LuaError::Runtime {
                class: "pz.host.Unsupported",
                ..
            })
        ));
        assert!(slot.borrow().is_none());
        assert!(!dropped.get(), "{source}");
        drop(held);
        drop(deferral);
        pz_inventory::component_host::drain_retired_components();
        assert!(dropped.get(), "{source}");
        assert_eq!(*foreign.borrow(), 1);
    }
}
