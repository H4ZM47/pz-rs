//! `Translator.getText` for script objects. The translator lives with the
//! UI text state, which this crate cannot reach, so the engine installs
//! it here before the scripts load (`Translator.loadFiles` runs before
//! `GameWindow.initShared` loads them).

use std::cell::RefCell;

type GetText = Box<dyn Fn(&str) -> String>;

thread_local! {
    static GET_TEXT: RefCell<Option<GetText>> = const { RefCell::new(None) };
}

/// Install (or with `None` remove) the translator's `getText(key)`.
pub fn install_get_text(f: Option<GetText>) {
    GET_TEXT.with(|g| *g.borrow_mut() = f);
}

/// `Translator.getText(key)`; the key itself while no translator is
/// installed.
pub fn get_text(key: &str) -> String {
    GET_TEXT.with(|g| match g.borrow().as_ref() {
        Some(f) => f(key),
        None => key.to_string(),
    })
}
