//! `Translator.getText` for script objects. The translator lives with the
//! UI text state, which this crate cannot reach, so the engine installs
//! it here before the scripts load (`Translator.loadFiles` runs before
//! `GameWindow.initShared` loads them).

use std::cell::RefCell;

type GetText = Box<dyn Fn(&str) -> String>;
type GetItemName = Box<dyn Fn(&str, &dyn Fn(&str) -> Option<String>) -> String>;

thread_local! {
    static GET_TEXT: RefCell<Option<GetText>> = const { RefCell::new(None) };
    static GET_ITEM_NAME: RefCell<Option<GetItemName>> = const { RefCell::new(None) };
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

/// Install (or with `None` remove) the translator's
/// `getItemNameFromFullType(fullType)`. Its second argument is the script
/// item's display name, for a type without a translation.
pub fn install_get_item_name(f: Option<GetItemName>) {
    GET_ITEM_NAME.with(|g| *g.borrow_mut() = f);
}

/// `Translator.getItemNameFromFullType(fullType)`. `script_display_name`
/// is `ScriptManager.instance.getItem(fullType)?.getDisplayName()`; while
/// no translator is installed it gives the name, else the full type.
pub fn get_item_name_from_full_type(
    full_type: &str,
    script_display_name: &dyn Fn(&str) -> Option<String>,
) -> String {
    GET_ITEM_NAME.with(|g| match g.borrow().as_ref() {
        Some(f) => f(full_type, script_display_name),
        None => script_display_name(full_type).unwrap_or_else(|| full_type.to_string()),
    })
}
