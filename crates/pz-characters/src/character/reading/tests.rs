use super::*;
use std::cell::Cell;

struct Host {
    character: Rc<RefCell<GameCharacter<()>>>,
    calls: RefCell<Vec<&'static str>>,
    nights: Cell<i32>,
    cooldown: Cell<i32>,
    fail: Cell<Option<&'static str>>,
    mutate: Cell<bool>,
}
impl ReadingHost for Host {
    fn nights_survived(&self) -> HostResult<i32> {
        self.calls.borrow_mut().push("clock");
        assert!(self.character.try_borrow_mut().is_ok());
        if self.fail.get() == Some("clock") {
            return Err(RuntimeHostError::Borrowed("clock"));
        }
        Ok(self.nights.get())
    }
    fn literature_cooldown(&self) -> HostResult<i32> {
        self.calls.borrow_mut().push("sandbox");
        let mut character = self.character.try_borrow_mut().unwrap();
        if self.mutate.get() {
            character.read_literature.insert("title".into(), 1000);
        }
        if self.fail.get() == Some("sandbox") {
            return Err(RuntimeHostError::Borrowed("sandbox"));
        }
        Ok(self.cooldown.get())
    }
}
fn setup() -> (Rc<RefCell<GameCharacter<()>>>, Host) {
    let character = Rc::new(RefCell::new(GameCharacter::<()>::new(
        &pz_scripts::ScriptManager::new(),
        crate::CharacterKind::Player,
    )));
    let host = Host {
        character: character.clone(),
        calls: RefCell::new(vec![]),
        nights: Cell::new(12),
        cooldown: Cell::new(3),
        fail: Cell::new(None),
        mutate: Cell::new(false),
    };
    (character, host)
}

#[test]
fn absent_title_skips_clock_and_sandbox_and_read_state_is_current() {
    let (character, host) = setup();
    host.fail.set(Some("clock"));
    assert!(!is_literature_read(&character, Some("title"), &host).unwrap());
    assert!(host.calls.borrow().is_empty());
    character
        .borrow_mut()
        .read_literature
        .insert("title".into(), 10);
    assert_eq!(
        is_literature_read(&character, Some("title"), &host),
        Err(RuntimeHostError::Borrowed("clock"))
    );
    assert_eq!(*host.calls.borrow(), ["clock"]);
}

#[test]
fn cooldown_uses_strict_native_boundary_and_wrapping_integer_addition() {
    let (character, host) = setup();
    character
        .borrow_mut()
        .read_literature
        .insert("title".into(), 10);
    assert!(is_literature_read(&character, Some("title"), &host).unwrap());
    host.nights.set(13);
    assert!(!is_literature_read(&character, Some("title"), &host).unwrap());
    character
        .borrow_mut()
        .read_literature
        .insert("title".into(), i32::MAX);
    host.cooldown.set(1);
    host.nights.set(i32::MIN);
    assert!(!is_literature_read(&character, Some("title"), &host).unwrap());
    host.nights.set(i32::MAX);
    assert!(!is_literature_read(&character, Some("title"), &host).unwrap());
}

#[test]
fn sandbox_reentry_has_no_guard_and_cannot_retarget_saved_day() {
    let (character, host) = setup();
    character
        .borrow_mut()
        .read_literature
        .insert("title".into(), 1);
    host.mutate.set(true);
    assert!(!is_literature_read(&character, Some("title"), &host).unwrap());
    assert_eq!(character.borrow().read_literature.get("title"), Some(&1000));
    assert_eq!(*host.calls.borrow(), ["clock", "sandbox"]);
}

#[test]
fn character_borrow_precedes_null_key_limit_and_host_queries() {
    let (character, host) = setup();
    let held = character.borrow_mut();
    assert!(matches!(
        is_literature_read(&character, None, &host),
        Err(RuntimeHostError::Borrowed("character readLiterature field"))
    ));
    assert!(matches!(
        is_print_media_read(&character, None),
        Err(RuntimeHostError::Borrowed("character readPrintMedia field"))
    ));
    assert!(matches!(
        already_read_pages(&character, None),
        Err(RuntimeHostError::Borrowed("character readBooks field"))
    ));
    assert!(host.calls.borrow().is_empty());
    drop(held);
    assert!(matches!(
        is_literature_read(&character, None, &host),
        Err(RuntimeHostError::Unsupported { .. })
    ));
    assert!(matches!(
        is_print_media_read(&character, None),
        Err(RuntimeHostError::Unsupported { .. })
    ));
}

#[test]
fn print_media_and_first_read_book_match_exact_case_and_preserve_raw_pages() {
    let (character, _) = setup();
    character
        .borrow_mut()
        .read_print_media
        .insert("News".into(), ());
    assert!(is_print_media_read(&character, Some("News")).unwrap());
    assert!(!is_print_media_read(&character, Some("news")).unwrap());
    character.borrow_mut().read_books.extend([
        super::super::save::ReadBook {
            full_type: "Base.Book".into(),
            already_read_pages: -7,
        },
        super::super::save::ReadBook {
            full_type: "Base.Book".into(),
            already_read_pages: 99,
        },
    ]);
    assert_eq!(
        already_read_pages(&character, Some("Base.Book")).unwrap(),
        -7
    );
    assert_eq!(
        already_read_pages(&character, Some("base.book")).unwrap(),
        0
    );
    assert_eq!(already_read_pages(&character, None).unwrap(), 0);
}
